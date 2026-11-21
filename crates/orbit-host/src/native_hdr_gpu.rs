//! GPU side of the HDR display pipeline. Layers are composed into an
//! `Rgba16Float` intermediate holding sRGB-encoded values; a final pass applies
//! sRGB EOTF, Rec.709 to Rec.2020 primaries, BT.2408 reference white, and the
//! PQ or HLG encoding into the 10-bit swapchain.

use crate::native_hdr::{HdrStatus, OutputFormat, HLG_NOMINAL_PEAK_NITS, SDR_REFERENCE_WHITE_NITS};

pub const INTERMEDIATE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const HDR_SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgb10a2Unorm;

const HDR_SHADER: &str = r#"
struct Params {
    mode: u32,
    white_nits: f32,
    hlg_peak_nits: f32,
    pad: f32,
};

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;

@vertex
fn vertex_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0)
    );
    return vec4<f32>(positions[vertex_index], 0.0, 1.0);
}

fn srgb_eotf(v: vec3<f32>) -> vec3<f32> {
    return select(pow((v + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), v / 12.92, v <= vec3<f32>(0.04045));
}

fn pq_encode(nits: vec3<f32>) -> vec3<f32> {
    let m1 = 2610.0 / 16384.0;
    let m2 = 2523.0 / 4096.0 * 128.0;
    let c1 = 3424.0 / 4096.0;
    let c2 = 2413.0 / 4096.0 * 32.0;
    let c3 = 2392.0 / 4096.0 * 32.0;
    let y = pow(clamp(nits / 10000.0, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(m1));
    return pow((c1 + c2 * y) / (1.0 + c3 * y), vec3<f32>(m2));
}

fn hlg_oetf(scene: vec3<f32>) -> vec3<f32> {
    let a = 0.17883277;
    let b = 0.28466892;
    let c = 0.55991073;
    let e = clamp(scene, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(a * log(max(12.0 * e - vec3<f32>(b), vec3<f32>(1e-6))) + c, sqrt(3.0 * e), e <= vec3<f32>(1.0 / 12.0));
}

@fragment
fn fragment_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = textureLoad(source, vec2<i32>(position.xy), 0);
    let linear709 = srgb_eotf(texel.rgb);
    let linear2020 = vec3<f32>(
        dot(linear709, vec3<f32>(0.627404, 0.329283, 0.043313)),
        dot(linear709, vec3<f32>(0.069097, 0.919540, 0.011362)),
        dot(linear709, vec3<f32>(0.016391, 0.088013, 0.895595))
    );
    var encoded: vec3<f32>;
    if (params.mode == 1u) {
        encoded = pq_encode(linear2020 * params.white_nits);
    } else {
        let display = linear2020 * params.white_nits / params.hlg_peak_nits;
        let luminance = dot(display, vec3<f32>(0.2627, 0.6780, 0.0593));
        var gain = 0.0;
        if (luminance > 0.0) {
            gain = pow(luminance, (1.0 - 1.2) / 1.2);
        }
        encoded = hlg_oetf(display * gain);
    }
    return vec4<f32>(encoded, 1.0);
}
"#;

pub struct HdrPresenter {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    intermediate: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    format: OutputFormat,
    status: HdrStatus,
}

impl HdrPresenter {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        format: OutputFormat,
        width: u32,
        height: u32,
        status: HdrStatus,
    ) -> Self {
        debug_assert!(format.is_hdr());
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Presenter Orbit HDR present layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Presenter Orbit HDR present shader"),
            source: wgpu::ShaderSource::Wgsl(HDR_SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Presenter Orbit HDR present pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Presenter Orbit HDR present pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Presenter Orbit HDR present parameters"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&format.shader_mode().to_le_bytes());
        bytes[4..8].copy_from_slice(&SDR_REFERENCE_WHITE_NITS.to_le_bytes());
        bytes[8..12].copy_from_slice(&HLG_NOMINAL_PEAK_NITS.to_le_bytes());
        queue.write_buffer(&params, 0, &bytes);
        let intermediate = create_intermediate(device, width, height);
        let bind_group = create_bind_group(device, &bind_group_layout, &intermediate, &params);
        Self {
            pipeline,
            bind_group_layout,
            params,
            intermediate,
            bind_group,
            format,
            status,
        }
    }

    pub fn format(&self) -> OutputFormat {
        self.format
    }

    pub fn status(&self) -> &HdrStatus {
        &self.status
    }

    pub fn set_status(&mut self, status: HdrStatus) {
        self.status = status;
    }

    pub fn intermediate_view(&self) -> &wgpu::TextureView {
        &self.intermediate
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.intermediate = create_intermediate(device, width, height);
        self.bind_group = create_bind_group(
            device,
            &self.bind_group_layout,
            &self.intermediate,
            &self.params,
        );
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Presenter Orbit HDR present pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn create_intermediate(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Presenter Orbit HDR composition intermediate"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: INTERMEDIATE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    intermediate: &wgpu::TextureView,
    params: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Presenter Orbit HDR present bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(intermediate),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params.as_entire_binding(),
            },
        ],
    })
}
