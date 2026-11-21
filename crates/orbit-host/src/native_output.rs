use std::sync::Arc;

use crate::Window;

use crate::native_compositor::NativeLayeredFrame;

use crate::native_hdr::{HdrStatus, OutputFormat};
use crate::native_hdr_gpu::{HdrPresenter, HDR_SURFACE_FORMAT, INTERMEDIATE_FORMAT};

/// GPU resources shared by every native output surface in the app.
///
/// Surfaces remain per-window because swapchain configuration, size and
/// frame textures are window-specific. The adapter/device/queue are not.
pub struct NativeGpuContext {
    instance: Arc<wgpu::Instance>,
    adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) max_texture_dimension_2d: u32,
}

struct LayerTexture {
    id: String,
    visual_key: u64,
    width: u32,
    height: u32,
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    replace: bool,
    draw_bounds: Option<LayerBounds>,
    was_dynamic_media: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LayerBounds {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl LayerBounds {
    fn union(self, other: Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }
}

/// Presents compositor-owned BGRA layers directly to a native Tauri window.
/// Static layer textures persist across presentations, and changes upload only
/// the affected pixel rectangle before the GPU composites the layers.
pub struct NativeOutputSurface {
    surface: wgpu::Surface<'static>,
    window: Window,
    hdr: Option<HdrPresenter>,
    gpu: Arc<NativeGpuContext>,
    config: wgpu::SurfaceConfiguration,
    layer_bind_group_layout: wgpu::BindGroupLayout,
    layer_pipeline: wgpu::RenderPipeline,
    replace_layer_pipeline: wgpu::RenderPipeline,
    layer_sampler: wgpu::Sampler,
    layer_textures: Vec<LayerTexture>,
    dynamic_media: bool,
    suspended: bool,
    video_delay: crate::output_delay::OutputDelay,
    delay_frames: usize,
    delayed_frame: Option<crate::native_compositor::NativeFrame>,
}

impl NativeOutputSurface {
    pub fn coordinated_window(&self) -> Window {
        self.window.clone()
    }

    pub fn new_with_contexts(
        window: Window,
        width: u32,
        height: u32,
        shared_gpus: &[Arc<NativeGpuContext>],
        output_format: OutputFormat,
    ) -> Result<Self, String> {
        pollster::block_on(Self::new_async(
            window,
            width,
            height,
            shared_gpus,
            output_format,
        ))
    }

    pub fn output_format(&self) -> OutputFormat {
        self.hdr
            .as_ref()
            .map_or(OutputFormat::Sdr, HdrPresenter::format)
    }

    pub fn hdr_status(&self) -> Option<&HdrStatus> {
        self.hdr.as_ref().map(HdrPresenter::status)
    }

    pub fn gpu_context(&self) -> Arc<NativeGpuContext> {
        Arc::clone(&self.gpu)
    }

    async fn new_async(
        window: Window,
        width: u32,
        height: u32,
        shared_gpus: &[Arc<NativeGpuContext>],
        output_format: OutputFormat,
    ) -> Result<Self, String> {
        let width = valid_dimension(width)?;
        let height = valid_dimension(height)?;
        crate::native_hdr::validate_output_format(
            output_format,
            "display",
            crate::native_hdr::applied_bit_depth(),
            crate::native_hdr::Platform::current(),
        )
        .map_err(|error| {
            format!("{error} The rendering bit depth applies after restarting Presenter Orbit.")
        })?;
        let instance = shared_gpus
            .first()
            .map(|gpu| Arc::clone(&gpu.instance))
            .unwrap_or_else(|| {
                // Windows HDR color spaces are applied to the DXGI swapchain.
                let descriptor = wgpu::InstanceDescriptor {
                    #[cfg(windows)]
                    backends: wgpu::Backends::DX12,
                    ..Default::default()
                };
                Arc::new(wgpu::Instance::new(&descriptor))
            });
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| format!("Native output surface creation failed: {error}"))?;
        let gpu = match shared_gpus.iter().find(|gpu| {
            surface
                .get_default_config(&gpu.adapter, width, height)
                .is_some()
        }) {
            Some(gpu) => Arc::clone(gpu),
            None => Arc::new(NativeGpuContext::create(Arc::clone(&instance), &surface).await?),
        };
        validate_texture_dimensions(width, height, gpu.max_texture_dimension_2d)?;
        let mut config = surface
            .get_default_config(&gpu.adapter, width, height)
            .ok_or_else(|| {
                "The graphics adapter cannot present to this output window.".to_string()
            })?;
        let capabilities = surface.get_capabilities(&gpu.adapter);
        config.format = linear_surface_format(config.format, &capabilities.formats);
        if output_format.is_hdr() {
            if !capabilities.formats.contains(&HDR_SURFACE_FORMAT) {
                return Err(format!(
                    "The graphics adapter cannot present a 10-bit surface required for {} output.",
                    output_format.as_str().to_uppercase()
                ));
            }
            config.format = HDR_SURFACE_FORMAT;
        }
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if capabilities
            .present_modes
            .contains(&wgpu::PresentMode::Mailbox)
        {
            config.present_mode = wgpu::PresentMode::Mailbox;
        }
        config.desired_maximum_frame_latency = 1;
        surface.configure(&gpu.device, &config);
        let hdr = if output_format.is_hdr() {
            let report =
                crate::native_hdr_platform::apply_color_space(&surface, &window, output_format)?;
            Some(HdrPresenter::new(
                &gpu.device,
                &gpu.queue,
                config.format,
                output_format,
                config.width,
                config.height,
                HdrStatus {
                    format: output_format,
                    surface_format: format!("{:?}", config.format),
                    color_space: report.color_space,
                    monitor_hdr: report.monitor_hdr,
                    monitor_max_nits: report.monitor_max_nits,
                },
            ))
        } else {
            None
        };
        let layer_target_format = if hdr.is_some() {
            INTERMEDIATE_FORMAT
        } else {
            config.format
        };
        let layer_bind_group_layout = create_layer_bind_group_layout(&gpu.device);
        let layer_pipeline = create_layer_pipeline(
            &gpu.device,
            layer_target_format,
            &layer_bind_group_layout,
            true,
        );
        let replace_layer_pipeline = create_layer_pipeline(
            &gpu.device,
            layer_target_format,
            &layer_bind_group_layout,
            false,
        );
        let layer_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Presenter Orbit native layer sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Ok(Self {
            surface,
            window,
            hdr,
            gpu,
            config,
            layer_bind_group_layout,
            layer_pipeline,
            replace_layer_pipeline,
            layer_sampler,
            layer_textures: Vec::new(),
            dynamic_media: false,
            suspended: false,
            video_delay: Default::default(),
            delay_frames: 0,
            delayed_frame: None,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        if width == 0 || height == 0 {
            self.suspended = true;
            return Ok(());
        }
        validate_texture_dimensions(width, height, self.gpu.max_texture_dimension_2d)?;
        self.suspended = false;
        self.video_delay.clear();
        self.delayed_frame = None;
        self.config.width = width;
        self.config.height = height;
        self.configure_surface()?;
        if let Some(hdr) = self.hdr.as_mut() {
            hdr.resize(&self.gpu.device, width, height);
        }
        Ok(())
    }

    /// Configures the swapchain and re-applies the HDR color space, which a
    /// reconfigure can reset.
    fn configure_surface(&mut self) -> Result<(), String> {
        self.surface.configure(&self.gpu.device, &self.config);
        if let Some(hdr) = self.hdr.as_mut() {
            let report = crate::native_hdr_platform::apply_color_space(
                &self.surface,
                &self.window,
                hdr.format(),
            )?;
            let mut status = hdr.status().clone();
            status.color_space = report.color_space;
            status.monitor_hdr = report.monitor_hdr;
            status.monitor_max_nits = report.monitor_max_nits;
            hdr.set_status(status);
        }
        Ok(())
    }

    pub fn present_layered(
        &mut self,
        frame: &NativeLayeredFrame,
        delay_frames: usize,
    ) -> Result<(), String> {
        if self.delay_frames != delay_frames {
            self.video_delay.clear();
            self.delayed_frame = None;
            self.delay_frames = delay_frames;
        }
        let delayed;
        let frame = if delay_frames > 0 {
            if let Some(ready) = self.video_delay.push(
                frame.flatten(),
                delay_frames,
                std::time::Duration::from_secs_f64(1.0 / 60.0),
                std::time::Instant::now(),
            )? {
                self.delayed_frame = Some(ready);
            }
            let pixels = self
                .delayed_frame
                .as_ref()
                .map(|ready| ready.pixels.clone())
                .unwrap_or_else(|| {
                    let mut pixels = vec![0; frame.width * frame.height * 4];
                    for pixel in pixels.chunks_exact_mut(4) {
                        pixel[3] = 255;
                    }
                    pixels
                });
            delayed = NativeLayeredFrame {
                width: frame.width,
                height: frame.height,
                dynamic_media: true,
                layers: vec![crate::native_compositor::NativeLayerFrame {
                    id: "delayed-output".into(),
                    visual_key: 0,
                    pixels: Arc::new(pixels),
                    bounds: Some(crate::native_compositor::NativeLayerBounds {
                        x: 0,
                        y: 0,
                        width: frame.width,
                        height: frame.height,
                    }),
                    dynamic_media: true,
                    replace: true,
                }],
            };
            &delayed
        } else {
            frame
        };
        let width = u32::try_from(frame.width)
            .map_err(|_| "Native layered frame width exceeds the GPU limit.".to_string())?;
        let height = u32::try_from(frame.height)
            .map_err(|_| "Native layered frame height exceeds the GPU limit.".to_string())?;
        validate_texture_dimensions(width, height, self.gpu.max_texture_dimension_2d)?;
        self.dynamic_media = frame.dynamic_media;
        if self.suspended {
            return Ok(());
        }

        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(height as usize))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "Native layer dimensions exceed the GPU limit.".to_string())?;
        let row_bytes = width
            .checked_mul(4)
            .ok_or_else(|| "Native layer row size exceeds the GPU limit.".to_string())?;
        for layer in &frame.layers {
            if layer.pixels.len() != expected {
                return Err(format!(
                    "Native layer '{}' has {} bytes; expected {expected} for {width}x{height} BGRA.",
                    layer.id,
                    layer.pixels.len()
                ));
            }
        }

        let mut previous = std::mem::take(&mut self.layer_textures);
        let mut textures = Vec::with_capacity(frame.layers.len());
        for layer in &frame.layers {
            let draw_bounds = layer_draw_bounds(layer, width, height)?;
            if let Some(index) = previous.iter().position(|texture| {
                can_reuse_static_layer_texture(layer.dynamic_media, texture.was_dynamic_media)
                    && texture.id == layer.id
                    && texture.visual_key == layer.visual_key
                    && texture.width == width
                    && texture.height == height
                    && texture.replace == layer.replace
            }) {
                textures.push(previous.swap_remove(index));
                continue;
            }
            if let Some(index) = previous.iter().position(|texture| {
                texture.id == layer.id && texture.width == width && texture.height == height
            }) {
                let mut texture = previous.swap_remove(index);
                if let Some(dirty_bounds) = texture
                    .draw_bounds
                    .into_iter()
                    .chain(draw_bounds)
                    .reduce(LayerBounds::union)
                {
                    write_layer_region(
                        &self.gpu.queue,
                        &texture._texture,
                        layer.pixels.as_slice(),
                        row_bytes,
                        height,
                        dirty_bounds,
                    );
                }
                texture.visual_key = layer.visual_key;
                texture.replace = layer.replace;
                texture.draw_bounds = draw_bounds;
                texture.was_dynamic_media = layer.dynamic_media;
                textures.push(texture);
                continue;
            }
            let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Presenter Orbit native visual layer"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: source_texture_format(self.config.format),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                layer.pixels.as_slice(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = self
                .gpu
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Presenter Orbit native visual layer binding"),
                    layout: &self.layer_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.layer_sampler),
                        },
                    ],
                });
            textures.push(LayerTexture {
                id: layer.id.clone(),
                visual_key: layer.visual_key,
                width,
                height,
                _texture: texture,
                bind_group,
                replace: layer.replace,
                draw_bounds,
                was_dynamic_media: layer.dynamic_media,
            });
        }
        self.layer_textures = textures;
        self.present_uploaded_layers()
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn has_dynamic_media(&self) -> bool {
        self.dynamic_media && !self.suspended
    }

    pub fn redraw(&mut self) -> Result<(), String> {
        if self.suspended {
            return Ok(());
        }
        self.present_uploaded_layers()
    }

    fn present_uploaded_layers(&mut self) -> Result<(), String> {
        let surface_frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.configure_surface()?;
                self.surface.get_current_texture().map_err(surface_error)?
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(error) => return Err(surface_error(error)),
        };
        let target = surface_frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Presenter Orbit native layered output"),
            });
        {
            let compose_target = self
                .hdr
                .as_ref()
                .map_or(&target, HdrPresenter::intermediate_view);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Presenter Orbit native layered composition pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: compose_target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            for layer in &self.layer_textures {
                let Some(bounds) = layer.draw_bounds else {
                    continue;
                };
                pass.set_scissor_rect(bounds.x, bounds.y, bounds.width, bounds.height);
                pass.set_pipeline(if layer.replace {
                    &self.replace_layer_pipeline
                } else {
                    &self.layer_pipeline
                });
                pass.set_bind_group(0, &layer.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        if let Some(hdr) = &self.hdr {
            hdr.encode(&mut encoder, &target);
        }
        self.gpu.queue.submit([encoder.finish()]);
        surface_frame.present();
        Ok(())
    }
}

const LAYER_SHADER: &str = r#"
@group(0) @binding(0) var layer_texture: texture_2d<f32>;
@group(0) @binding(1) var layer_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0)
    );
    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.uv = vec2<f32>((output.position.x + 1.0) * 0.5, (1.0 - output.position.y) * 0.5);
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(layer_texture, layer_sampler, input.uv);
}
"#;

fn create_layer_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Presenter Orbit native layer bind group layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn create_layer_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    blend: bool,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Presenter Orbit native layer shader"),
        source: wgpu::ShaderSource::Wgsl(LAYER_SHADER.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Presenter Orbit native layer pipeline layout"),
        bind_group_layouts: &[bind_group_layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Presenter Orbit native layer pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fragment_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview: None,
        cache: None,
    })
}

impl NativeGpuContext {
    async fn create(
        instance: Arc<wgpu::Instance>,
        surface: &wgpu::Surface<'_>,
    ) -> Result<Self, String> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|error| format!("No compatible graphics adapter is available: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Presenter Orbit shared native output device"),
                ..Default::default()
            })
            .await
            .map_err(|error| format!("Native output GPU device creation failed: {error}"))?;
        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            max_texture_dimension_2d,
        })
    }
}

fn valid_dimension(value: u32) -> Result<u32, String> {
    if value == 0 {
        Err("Native output surface dimensions must be non-zero.".to_string())
    } else {
        Ok(value)
    }
}

fn validate_texture_dimensions(width: u32, height: u32, limit: u32) -> Result<(), String> {
    valid_dimension(width)?;
    valid_dimension(height)?;
    if width > limit || height > limit {
        return Err(format!(
            "Native output dimensions {width}x{height} exceed the GPU texture limit of {limit}."
        ));
    }
    Ok(())
}

fn source_texture_format(surface_format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    if surface_format.is_srgb() {
        wgpu::TextureFormat::Bgra8UnormSrgb
    } else {
        wgpu::TextureFormat::Bgra8Unorm
    }
}

fn linear_surface_format(
    default: wgpu::TextureFormat,
    supported: &[wgpu::TextureFormat],
) -> wgpu::TextureFormat {
    let linear = match default {
        wgpu::TextureFormat::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8Unorm,
        format => format,
    };
    if supported.contains(&linear) {
        linear
    } else {
        default
    }
}

fn surface_error(error: wgpu::SurfaceError) -> String {
    format!("Native output GPU presentation failed: {error}")
}

fn layer_draw_bounds(
    layer: &crate::native_compositor::NativeLayerFrame,
    width: u32,
    height: u32,
) -> Result<Option<LayerBounds>, String> {
    let Some(bounds) = layer.bounds else {
        return Ok(None);
    };
    let x = u32::try_from(bounds.x)
        .map_err(|_| format!("Native layer '{}' bounds exceed the GPU limit.", layer.id))?;
    let y = u32::try_from(bounds.y)
        .map_err(|_| format!("Native layer '{}' bounds exceed the GPU limit.", layer.id))?;
    let bounds_width = u32::try_from(bounds.width)
        .map_err(|_| format!("Native layer '{}' bounds exceed the GPU limit.", layer.id))?;
    let bounds_height = u32::try_from(bounds.height)
        .map_err(|_| format!("Native layer '{}' bounds exceed the GPU limit.", layer.id))?;
    if bounds_width == 0
        || bounds_height == 0
        || x.checked_add(bounds_width)
            .is_none_or(|right| right > width)
        || y.checked_add(bounds_height)
            .is_none_or(|bottom| bottom > height)
    {
        return Err(format!(
            "Native layer '{}' has bounds outside the {width}x{height} target.",
            layer.id
        ));
    }
    Ok(Some(LayerBounds {
        x,
        y,
        width: bounds_width,
        height: bounds_height,
    }))
}

fn can_reuse_static_layer_texture(current_dynamic: bool, previous_dynamic: bool) -> bool {
    !current_dynamic && !previous_dynamic
}

fn write_layer_region(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    pixels: &[u8],
    row_bytes: u32,
    full_height: u32,
    bounds: LayerBounds,
) {
    let source_offset = (bounds.y as usize * (row_bytes as usize / 4) + bounds.x as usize) * 4;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: bounds.x,
                y: bounds.y,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &pixels[source_offset..],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(row_bytes),
            rows_per_image: Some(full_height),
        },
        wgpu::Extent3d {
            width: bounds.width,
            height: bounds.height,
            depth_or_array_layers: 1,
        },
    );
}
