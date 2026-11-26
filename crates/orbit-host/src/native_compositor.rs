use std::{
    collections::{HashMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

use image::{codecs::png::PngEncoder, DynamicImage, ExtendedColorType, ImageEncoder};
use serde_json::{json, Map, Value};

use crate::native_render::{
    scrolling_is_active, CutOutBackdrop, NativeImageFrame, NativeRenderRuntime,
};
use crate::native_transitions;
use crate::output_playback::scroll_content_key_for_value;
use crate::scene::{
    parse_scene_slide, parse_scene_slide_with_build_layers, SceneLayer, SceneLayerKind, SceneSlide,
};

#[path = "native_chart.rs"]
mod native_chart;
#[path = "native_fill_playback.rs"]
mod native_fill_playback;
#[path = "native_linked_text.rs"]
mod native_linked_text;
#[path = "native_object_builds.rs"]
mod native_object_builds;
#[path = "native_object_effects.rs"]
mod native_object_effects;
#[path = "native_object_links.rs"]
mod native_object_links;
#[path = "native_stage_previews.rs"]
mod native_stage_previews;

pub fn validate_chord_chart(content: &Value) -> Result<(), String> {
    native_chart::validate(content)
}

pub fn validate_stage_previews(content: &Value) -> Result<(), String> {
    native_stage_previews::validate(content)
}

pub fn validate_stage_preview_fields(content: &Value) -> Result<(), String> {
    native_stage_previews::validate_fields(content)
}

pub fn validate_object_color_effects(
    object: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    for prefix in ["media", "mediaFill"] {
        for (suffix, min, max) in [
            ("Hue", -180.0, 180.0),
            ("Saturation", 0.0, 2.0),
            ("Brightness", -1.0, 1.0),
            ("Contrast", 0.0, 2.0),
        ] {
            let field = format!("{prefix}{suffix}");
            if let Some(value) = object.get(&field) {
                let number = value
                    .as_f64()
                    .filter(|number| number.is_finite() && *number >= min && *number <= max);
                if number.is_none() {
                    return Err(format!(
                        "{field} must be a finite number between {min} and {max}."
                    ));
                }
            }
        }
        if let Some(chain) = object.get(&format!("{prefix}Effects")) {
            crate::native_media_effects::validate_chain(chain)?;
        }
    }
    Ok(())
}

/// A still of one slide for editors, over black, with videos paused at their first frame.
/// Waits briefly for decoders so video media appears in the still.
pub fn render_slide_still(
    slide: &Value,
    width: usize,
    height: usize,
) -> Result<NativeFrame, String> {
    render_slide_still_over(slide, &json!({}), width, height)
}

/// A still of a stage layout over sample slide text, for layout pickers. Screen previews are left empty.
pub fn render_stage_layout_still(
    slide: &Value,
    width: usize,
    height: usize,
) -> Result<NativeFrame, String> {
    render_scene_still(&stage_preview_content(slide, Value::Null)?, width, height)
}

pub fn render_mask_still(
    slide: &Value,
    width: usize,
    height: usize,
) -> Result<NativeFrame, String> {
    render_scene_still(&mask_preview_content(slide), width, height)
}

fn mask_preview_content(slide: &Value) -> Value {
    json!({"backgroundCss":"transparent","maskSlide":slide})
}

fn stage_preview_content(slide: &Value, mut context: Value) -> Result<Value, String> {
    if !context.is_object() {
        context = json!({"stageSceneResources":native_stage_previews::sample_registry(slide)?});
    }
    if context.get("captureStatus").is_none() {
        context["captureStatus"] = json!({"active":false});
    }
    for (key, text) in [
        ("slide", "Current Slide Text"),
        ("nextSlide", "Next Slide Text"),
        ("nextArrangementSlide", "Next Slide Text"),
    ] {
        if context[key].is_null() {
            context[key] = json!({"content":text,"label":"Verse 1","notes":"","groupColor":"#2563eb","labelColor":"#2563eb"});
        }
    }
    let background = if slide["backgroundEnabled"] == false {
        "transparent".into()
    } else {
        slide
            .get("background")
            .and_then(background_css)
            .unwrap_or_else(|| "#000000".into())
    };
    Ok(json!({"backgroundCss":background,"stageSlide":slide,"linkedTextContext":context}))
}

/// Memory bound for bitmaps rendered for the UI (previews and stills): 128 MiB of RGBA per frame. Callers request
/// device-pixel sizes, so this is far above a 4K frame and only stops a runaway request.
pub const MAX_FRAME_EDGE: u64 = 8_192;
pub const MAX_FRAME_PIXELS: u64 = 33_554_432;

pub fn frame_dimensions_allowed(width: u64, height: u64) -> bool {
    (1..=MAX_FRAME_EDGE).contains(&width)
        && (1..=MAX_FRAME_EDGE).contains(&height)
        && width * height <= MAX_FRAME_PIXELS
}

/// Like [`render_slide_still`], over `background` scene fields such as `backgroundMediaType` and `backgroundMediaPath`.
pub fn render_slide_still_over(
    slide: &Value,
    background: &Value,
    width: usize,
    height: usize,
) -> Result<NativeFrame, String> {
    if !slide.is_object() {
        return Err("Slide stills require a slide object.".into());
    }
    if !frame_dimensions_allowed(width as u64, height as u64) {
        return Err(format!(
            "Slide stills must be between 1 and {MAX_FRAME_EDGE} pixels on each side and contain at most {MAX_FRAME_PIXELS} pixels."
        ));
    }
    let mut content = json!({ "backgroundCss": "#000000", "presentationThemeSlide": slide });
    if let Some(fields) = background.as_object() {
        content
            .as_object_mut()
            .ok_or("Slide still content must be an object.")?
            .extend(fields.clone());
    }
    if let Some(text) = slide.get("content") {
        content["text"] = text.clone();
    }
    render_scene_still(&content, width, height)
}

/// A frozen complete scene, including background-only cue previews.
pub fn render_scene_still(
    content: &Value,
    width: usize,
    height: usize,
) -> Result<NativeFrame, String> {
    if !content.is_object() || !frame_dimensions_allowed(width as u64, height as u64) {
        return Err("A scene still requires an object and valid preview dimensions.".into());
    }
    let mut content = content.clone();
    native_stage_previews::freeze_video_playback(&mut content);
    let mut compositor = NativeCompositor::new_static_preview();
    let deadline = Instant::now() + std::time::Duration::from_millis(1000);
    loop {
        let frame = compositor.try_render_preview_for("slide-still", &content, width, height)?;
        if !frame.dynamic_media || Instant::now() >= deadline {
            return Ok(frame);
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

pub fn validate_object_links(content: &Value) -> Result<(), String> {
    validate_object_links_in_context(content, content.get("linkedTextContext"))
}

fn validate_object_links_in_context(
    content: &Value,
    context: Option<&Value>,
) -> Result<(), String> {
    if let Some(object) = content.as_object() {
        validate_object_color_effects(object)?;
        crate::native_text_layout::validate_paragraph_style(object)?;
        native_fill_playback::validate(object)?;
        if object.get("fillType").and_then(Value::as_str) == Some("web") {
            if let Some(url) = object.get("webUrl") {
                crate::web_capture::page_url(
                    url.as_str().ok_or("Web fill webUrl must be a string.")?,
                )?;
            }
        }
        let context = object.get("linkedTextContext").or(context);
        if let Some(color) = object.get("labelColor") {
            if color.as_str().and_then(parse_color).is_none() {
                return Err("Slide labelColor must be a valid native color string.".into());
            }
        }
        if object.get("layers").is_some_and(Value::is_array) {
            let slide = SceneSlide::from_value(content)?;
            native_object_links::validate(&slide)?;
            if let Some(context) = context {
                for layer in object
                    .get("layers")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_object)
                {
                    // Authored capture colors are checked above; live status is attached at native projection.
                    if layer.get("fillType").and_then(Value::as_str) == Some("capture-status-color")
                        && context.get("captureStatus").is_none()
                    {
                        continue;
                    }
                    native_object_links::resolve(&slide, layer, context, "", false)?;
                }
            }
        }
        for (key, value) in object {
            if key != "textParagraphStyles" {
                validate_object_links_in_context(value, context)?;
            }
        }
    } else if let Some(values) = content.as_array() {
        for value in values {
            validate_object_links_in_context(value, context)?;
        }
    }
    Ok(())
}
use crate::native_media::{prefetch_content, resolve_media_path};
pub use native_linked_text::content_has_dynamic_linked_text;
pub(crate) use native_linked_text::format_ccli_text;
use native_linked_text::resolve_linked_text;
pub(crate) use native_linked_text::{apply_capitalization, apply_line_transform};

/// Binary framing used by the raw WebView transport.
///
/// The packet is intentionally self-describing so the renderer can reject a
/// truncated, incorrectly-sized, or future-format frame before touching a
/// canvas. The payload is tightly packed RGBA8 (not BGRA8, which is the
/// compositor's internal/NDI representation).
pub const RAW_FRAME_HEADER_BYTES: usize = 24;
const RAW_FRAME_MAGIC: &[u8; 4] = b"PORF";
const RAW_FRAME_VERSION: u8 = 1;
const RAW_FRAME_RGBA8: u8 = 1;
const RAW_FRAME_FLAG_DYNAMIC: u8 = 1;

/// The rendered native output frame. Pixels are BGRA8, matching the NDI v2 ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeFrame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    pub dynamic_media: bool,
}

#[derive(Clone, Copy)]
pub enum CanvasRenderKind {
    Presentation,
    Stage,
    Mask,
    Ccli,
}

/// One independently cached visual plane for the native GPU compositor.
/// Pixels use straight-alpha BGRA8; transparent pixels remain zero so layers
/// can be blended in scene order on the GPU.
#[derive(Debug, Clone)]
pub struct NativeLayerFrame {
    pub id: String,
    pub visual_key: u64,
    pub pixels: Arc<Vec<u8>>,
    /// Tight alpha bounds for overlay planes; replacement planes cover the
    /// full target. Cached with the pixels so consumers do not rescan frames.
    pub bounds: Option<NativeLayerBounds>,
    pub dynamic_media: bool,
    pub replace: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeLayerBounds {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone)]
pub struct NativeLayeredFrame {
    pub width: usize,
    pub height: usize,
    pub layers: Vec<NativeLayerFrame>,
    pub dynamic_media: bool,
}

impl NativeLayeredFrame {
    /// Flatten layered output for consumers such as the in-app preview that
    /// still require one transport image. Rasterization remains incremental;
    /// this final blend only touches the preview-sized buffers.
    pub fn flatten(&self) -> NativeFrame {
        let mut pixels = vec![0; self.width.saturating_mul(self.height).saturating_mul(4)];
        for layer in &self.layers {
            if layer.replace {
                pixels.copy_from_slice(layer.pixels.as_slice());
                continue;
            }
            let Some(bounds) = layer.bounds else {
                continue;
            };
            let right = bounds.x.saturating_add(bounds.width).min(self.width);
            let bottom = bounds.y.saturating_add(bounds.height).min(self.height);
            for y in bounds.y.min(bottom)..bottom {
                let row_start = (y * self.width + bounds.x.min(right)) * 4;
                for x in bounds.x.min(right)..right {
                    let offset = row_start + (x - bounds.x.min(right)) * 4;
                    let source = &layer.pixels[offset..offset + 4];
                    if source[3] == 0 {
                        continue;
                    }
                    blend_pixel(
                        &mut pixels,
                        self.width,
                        x,
                        y,
                        [source[0], source[1], source[2], source[3]],
                        1.0,
                    );
                }
            }
        }
        NativeFrame {
            width: self.width,
            height: self.height,
            pixels,
            dynamic_media: self.dynamic_media,
        }
    }
}

impl NativeFrame {
    /// Encode the BGRA framebuffer for transport to a Tauri WebView.
    pub fn encode_png(&self) -> Result<Vec<u8>, String> {
        let mut rgba = self.pixels.clone();
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let width = u32::try_from(self.width)
            .map_err(|_| "Native frame width exceeds the PNG limit.".to_string())?;
        let height = u32::try_from(self.height)
            .map_err(|_| "Native frame height exceeds the PNG limit.".to_string())?;
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
            .map_err(|error| format!("Native frame PNG encoding failed: {error}"))?;
        Ok(png)
    }

    /// Pack the frame as a header plus tightly packed RGBA8 pixels.
    ///
    /// This avoids the per-frame PNG encode/decode work for WebView output.
    /// The conversion is performed here because the compositor owns BGRA8
    /// pixels for the NDI ABI while Canvas ImageData consumes RGBA8.
    pub fn encode_raw_rgba(&self) -> Result<Vec<u8>, String> {
        let width = u32::try_from(self.width)
            .map_err(|_| "Native frame width exceeds the raw transport limit.".to_string())?;
        let height = u32::try_from(self.height)
            .map_err(|_| "Native frame height exceeds the raw transport limit.".to_string())?;
        let stride = width
            .checked_mul(4)
            .ok_or_else(|| "Native frame stride exceeds the raw transport limit.".to_string())?;
        let payload_bytes = usize::try_from(stride)
            .ok()
            .and_then(|row_bytes| row_bytes.checked_mul(self.height))
            .ok_or_else(|| "Native frame payload exceeds the raw transport limit.".to_string())?;
        if self.pixels.len() != payload_bytes {
            return Err(format!(
                "Native frame has {} bytes; expected {payload_bytes} for {width}x{height} RGBA.",
                self.pixels.len()
            ));
        }
        let payload_bytes_u32 = u32::try_from(payload_bytes)
            .map_err(|_| "Native frame payload exceeds the raw transport limit.".to_string())?;

        let mut packet = Vec::with_capacity(RAW_FRAME_HEADER_BYTES + payload_bytes);
        packet.extend_from_slice(RAW_FRAME_MAGIC);
        packet.push(RAW_FRAME_VERSION);
        packet.push(RAW_FRAME_RGBA8);
        packet.push(if self.dynamic_media {
            RAW_FRAME_FLAG_DYNAMIC
        } else {
            0
        });
        packet.push(0);
        packet.extend_from_slice(&width.to_le_bytes());
        packet.extend_from_slice(&height.to_le_bytes());
        packet.extend_from_slice(&stride.to_le_bytes());
        packet.extend_from_slice(&payload_bytes_u32.to_le_bytes());
        for pixel in self.pixels.chunks_exact(4) {
            packet.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
        Ok(packet)
    }
}

/// Shared native scene compositor used by NDI and future native output surfaces.
pub struct NativeCompositor {
    renderer: NativeRenderRuntime,
    background_image_cache: VecDeque<CachedBackgroundImage>,
    layer_cache: NativeLayerCache,
    stage_preview_renderers: HashMap<String, NativeRenderRuntime>,
    layer_transitions: HashMap<(String, TransitionLayer), LayerTransitionState>,
    media_selections: HashMap<String, (Value, bool)>,
    look_transitions: HashMap<String, LookTransitionState>,
    fill_clocks: HashMap<String, native_fill_playback::FillClock>,
    object_build_transitions: native_object_builds::ObjectBuildTransitions,
}

/// Per-output history of one layer, used to transition from what it showed before.
#[derive(Default)]
struct LayerTransitionState {
    key: Value,
    spec: Option<native_transitions::TransitionSpec>,
    /// The prop layer's transition per shown prop.
    transitions: Value,
    layers: Vec<NativeLayerFrame>,
    displayed: Option<Arc<Vec<u8>>>,
    active: Option<native_transitions::ActiveTransition>,
    replace: bool,
    includes_media: bool,
    /// The highest stacking rank the layer last occupied.
    rank: u8,
}

/// Per-output history of the whole frame, so an applied Audience Look can fade from the frame before it.
#[derive(Default)]
struct LookTransitionState {
    token: Value,
    layers: Vec<NativeLayerFrame>,
    displayed: Option<Vec<u8>>,
    active: Option<native_transitions::ActiveTransition>,
}

type NativeLayerCacheKey = (String, String, u64, usize, usize);
type NativeLayerCache = HashMap<NativeLayerCacheKey, CachedLayerPixels>;

#[derive(Clone)]
struct CachedLayerPixels {
    pixels: Arc<Vec<u8>>,
    bounds: Option<NativeLayerBounds>,
}

struct CachedBackgroundImage {
    effects: Vec<crate::native_media_effects::ResolvedEffect>,
    desaturate: bool,
    brightness: f64,
    contrast: f64,
    hue: f64,
    saturation: f64,
    source_image: Arc<DynamicImage>,
    width: usize,
    height: usize,
    scale: String,
    layers: Vec<RenderedBackgroundLayer>,
}

impl CachedBackgroundImage {
    fn bytes(&self) -> usize {
        self.source_image.as_bytes().len().saturating_add(
            self.layers
                .iter()
                .map(|layer| layer.pixels.len())
                .sum::<usize>(),
        )
    }
}

struct RenderedBackgroundLayer {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
    opaque: bool,
    left: isize,
    top: isize,
}

static NATIVE_OUTPUT_CLOCK: OnceLock<Instant> = OnceLock::new();
static NATIVE_SCROLL_EPOCHS: OnceLock<Mutex<HashMap<String, ScrollEpoch>>> = OnceLock::new();
const MAX_NATIVE_SCROLL_EPOCHS: usize = 256;

#[derive(Debug, Clone, Copy)]
struct ScrollEpoch {
    started_at: f64,
    last_used_at: f64,
}

impl NativeCompositor {
    /// Editor pixels use the output renderer, including its backdrop dependency classification.
    /// The layer cache already exposes the three gesture planes without re-rendering filtered copies.
    pub fn render_canvas(
        &mut self,
        slide: &Value,
        snapshot: Option<&Value>,
        width: usize,
        height: usize,
        active: Option<usize>,
        kind: CanvasRenderKind,
    ) -> Result<(Vec<NativeFrame>, bool), String> {
        if !frame_dimensions_allowed(width as u64, height as u64) {
            return Err("Invalid canvas raster size.".into());
        }
        let mut slide = slide.clone();
        slide["ignoreBuilds"] = json!(true);
        let parsed = parse_scene_slide(&slide)?;
        let active = active
            .map(|index| {
                parsed
                    .layers
                    .iter()
                    .position(|layer| layer.source_index == index)
                    .ok_or_else(|| "The active canvas layer is not visible.".to_string())
            })
            .transpose()?;
        let dependent = active.is_some_and(|index| {
            parsed
                .layers
                .iter()
                .skip(index)
                .any(|layer| scene_layer_uses_backdrop(&parsed, layer))
        });
        let mut context = Value::Null;
        if let Some(snapshot) = snapshot {
            let scenes = crate::output_scene::build_output_scene_state(snapshot)?;
            if let Some(scene) = scenes.values().next() {
                context = scene["linkedTextContext"].clone();
            }
        }
        let mut content = match kind {
            CanvasRenderKind::Stage => stage_preview_content(&slide, context)?,
            CanvasRenderKind::Mask => mask_preview_content(&slide),
            CanvasRenderKind::Presentation | CanvasRenderKind::Ccli => {
                let mut content =
                    json!({"backgroundCss":"transparent","presentationThemeSlide":slide});
                if let Some(text) = slide.get("content") {
                    content["text"] = text.clone();
                }
                if matches!(kind, CanvasRenderKind::Ccli) {
                    context["ccliTemplatePreview"] = json!(true);
                }
                if !context.is_null() {
                    content["linkedTextContext"] = context;
                }
                content
            }
        };
        native_stage_previews::freeze_video_playback(&mut content);
        let deadline = Instant::now() + std::time::Duration::from_millis(1000);
        let frame = loop {
            let frame = self.try_render_layered_for("canvas", &content, width, height)?;
            if !frame.dynamic_media || Instant::now() >= deadline {
                break frame;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let Some(active) = active.filter(|_| !dependent) else {
            return Ok((vec![frame.flatten()], dependent));
        };
        let mut planes = [Vec::new(), Vec::new(), Vec::new()];
        for layer in frame.layers {
            let index = layer
                .id
                .strip_prefix(match kind {
                    CanvasRenderKind::Stage => "stage-layer-",
                    CanvasRenderKind::Mask => "mask-layer-",
                    CanvasRenderKind::Presentation | CanvasRenderKind::Ccli => "theme-layer-",
                })
                .and_then(|id| id.parse::<usize>().ok());
            let plane = match index {
                Some(index) if index == active => 1,
                Some(index) if index > active => 2,
                _ => 0,
            };
            planes[plane].push(layer);
        }
        Ok((
            planes
                .into_iter()
                .map(|layers| {
                    NativeLayeredFrame {
                        width,
                        height,
                        layers,
                        dynamic_media: frame.dynamic_media,
                    }
                    .flatten()
                })
                .collect(),
            false,
        ))
    }

    pub fn new() -> Self {
        Self {
            renderer: NativeRenderRuntime::new(),
            background_image_cache: VecDeque::new(),
            layer_cache: HashMap::new(),
            stage_preview_renderers: HashMap::new(),
            layer_transitions: HashMap::new(),
            media_selections: HashMap::new(),
            look_transitions: HashMap::new(),
            fill_clocks: HashMap::new(),
            object_build_transitions: native_object_builds::ObjectBuildTransitions::default(),
        }
    }

    pub fn new_preview() -> Self {
        Self::with_renderer(NativeRenderRuntime::new_with_video_max_size(None))
    }

    fn with_renderer(renderer: NativeRenderRuntime) -> Self {
        Self {
            renderer,
            background_image_cache: VecDeque::new(),
            layer_cache: HashMap::new(),
            stage_preview_renderers: HashMap::new(),
            layer_transitions: HashMap::new(),
            media_selections: HashMap::new(),
            look_transitions: HashMap::new(),
            fill_clocks: HashMap::new(),
            object_build_transitions: native_object_builds::ObjectBuildTransitions::default(),
        }
    }

    /// A preview compositor that draws animated images at their first frame.
    pub fn new_static_preview() -> Self {
        Self::with_renderer(NativeRenderRuntime::new_static_preview())
    }

    pub fn try_render_preview_for(
        &mut self,
        namespace: &str,
        content: &Value,
        width: usize,
        height: usize,
    ) -> Result<NativeFrame, String> {
        self.renderer
            .set_video_max_size(Some((width as u32, height as u32)));
        self.try_render_layered_for(namespace, content, width, height)
            .map(|frame| frame.flatten())
    }

    pub fn set_video_max_size(&mut self, size: Option<(u32, u32)>) {
        self.renderer.set_video_max_size(size);
    }

    pub fn video_available(&self) -> bool {
        self.renderer.video_available()
    }

    pub fn video_error(&self) -> String {
        let error = self.renderer.video_error();
        if !error.is_empty() {
            return error;
        }
        self.stage_preview_renderers
            .values()
            .map(NativeRenderRuntime::video_error)
            .find(|error| !error.is_empty())
            .unwrap_or_default()
    }

    /// Render into a caller-owned BGRA frame buffer without reallocating it.
    pub fn render_into(
        &mut self,
        pixels: &mut [u8],
        width: usize,
        height: usize,
        content: &Value,
    ) -> Result<bool, String> {
        // Slide and layer transitions blend layer planes, which only the layered path separates.
        if content
            .get("keyFillSignal")
            .is_some_and(|value| !value.is_null())
            || content.get("slideTransition").is_some_and(Value::is_object)
            || content.get("slideEntryKey").is_some_and(Value::is_string)
            || self
                .layer_transitions
                .contains_key(&("render-into".into(), TransitionLayer::Slide))
            || content
                .get("layerTransitions")
                .is_some_and(Value::is_object)
            || content
                .get("objectBuildEntryKey")
                .and_then(Value::as_str)
                .is_some_and(|key| !key.trim().is_empty())
            || content
                .get("projectorCorrection")
                .is_some_and(|value| !value.is_null())
            || content
                .get("canvasViewport")
                .is_some_and(|value| !value.is_null())
        {
            let frame = self.try_render_layered_for("render-into", content, width, height)?;
            let flat = frame.flatten();
            if pixels.len() != flat.pixels.len() {
                return Err(format!(
                    "Native frame buffer has {} bytes; expected {} for {width}x{height} BGRA.",
                    pixels.len(),
                    flat.pixels.len()
                ));
            }
            pixels.copy_from_slice(&flat.pixels);
            return Ok(flat.dynamic_media);
        }
        if let Some(pattern) = content.get("testPattern").filter(|value| !value.is_null()) {
            let rendered = crate::test_pattern::render(
                pattern,
                width,
                height,
                crate::test_pattern::clock_seconds(),
                content
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                &mut self.renderer,
            )?;
            if pixels.len() != rendered.len() {
                return Err("Test pattern frame buffer size is invalid.".into());
            }
            pixels.copy_from_slice(&rendered);
            return Ok(crate::test_pattern::is_animated(pattern));
        }
        validate_object_links(content)?;
        validate_stage_previews(content)?;
        let timed = self.with_slide_fill_clocks("render-into", content);
        let content = timed.as_ref().unwrap_or(content);
        native_chart::prepare(content, &mut self.renderer)?;
        native_stage_previews::prepare(
            content,
            &mut self.stage_preview_renderers,
            &mut self.renderer,
        )?;
        let expected = width.saturating_mul(height).saturating_mul(4);
        if pixels.len() != expected {
            return Err(format!(
                "Native frame buffer has {} bytes; expected {expected} for {width}x{height} BGRA.",
                pixels.len()
            ));
        }
        // Remote images are downloaded by the bounded cache worker. This call
        // only schedules missing assets; rendering itself reads ready local
        // files and never performs network I/O.
        let media_download_pending = prefetch_content(content);
        let now = native_output_elapsed_seconds();
        let scroll_elapsed_seconds = if content_has_scrolling_text(content, 0.0) {
            let key = content
                .get("scrollContentKey")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| scroll_content_key_for_value(content));
            native_scroll_elapsed_seconds(&key, now)
        } else {
            now
        };
        let mut target = RenderTarget {
            pixels,
            width,
            height,
            dynamic_media: false,
            scroll_elapsed_seconds,
        };
        self.renderer.begin_frame();
        compose_frame(
            &mut target,
            content,
            &mut self.renderer,
            &mut self.background_image_cache,
        );
        self.renderer.end_frame();
        target.dynamic_media |= media_download_pending;
        Ok(target.dynamic_media)
    }

    pub fn try_render_layered_for(
        &mut self,
        namespace: &str,
        content: &Value,
        width: usize,
        height: usize,
    ) -> Result<NativeLayeredFrame, String> {
        if let Some(value) = content
            .get("keyFillSignal")
            .filter(|value| !value.is_null())
        {
            let signal: crate::key_fill::KeyFillSignal = serde_json::from_value(value.clone())
                .map_err(|error| format!("Invalid key/fill signal: {error}"))?;
            let mut source = content.clone();
            source
                .as_object_mut()
                .ok_or("Key/fill scene must be an object.")?
                .remove("keyFillSignal");
            if content
                .get("projectorCorrection")
                .is_none_or(Value::is_null)
            {
                source["backgroundCss"] = Value::String("transparent".into());
            }
            let mut frame = self.try_render_layered_for(namespace, &source, width, height)?;
            frame.layers.retain(|layer| layer.id != "background-color");
            return Ok(apply_key_fill_signal(frame, signal));
        }
        let correction =
            crate::projector_correction::parse_optional(content.get("projectorCorrection"))?;
        if let Some(correction) = correction {
            let rendered = if content
                .get("testPattern")
                .is_some_and(|value| !value.is_null())
            {
                self.try_render_layered_base_for(namespace, content, width, height)?
            } else if let Some(value) = content
                .get("canvasViewport")
                .filter(|value| !value.is_null())
            {
                let viewport: crate::output_canvas::CanvasViewport =
                    serde_json::from_value(value.clone())
                        .map_err(|error| format!("Canvas viewport is invalid: {error}"))?;
                let viewport = viewport
                    .validate()
                    .map_err(|error| format!("Canvas viewport is invalid: {error}"))?;
                if viewport.can_render_directly(width, height) {
                    self.try_render_layered_base_for(namespace, content, width, height)?
                } else {
                    let mut source_content = content.clone();
                    let source = source_content
                        .as_object_mut()
                        .ok_or_else(|| "Canvas output content must be an object.".to_string())?;
                    source.remove("canvasViewport");
                    source.remove("projectorCorrection");
                    let source = self.try_render_layered_base_for(
                        namespace,
                        &source_content,
                        viewport.canvas_width,
                        viewport.canvas_height,
                    )?;
                    crop_scale_layered_frame(&source, viewport, width, height)?
                }
            } else {
                self.try_render_layered_base_for(namespace, content, width, height)?
            };
            return apply_projector_correction(rendered, &correction);
        }

        if content
            .get("testPattern")
            .is_some_and(|value| !value.is_null())
        {
            return self.try_render_layered_base_for(namespace, content, width, height);
        }
        let Some(value) = content
            .get("canvasViewport")
            .filter(|value| !value.is_null())
        else {
            return self.try_render_layered_base_for(namespace, content, width, height);
        };
        let viewport: crate::output_canvas::CanvasViewport = serde_json::from_value(value.clone())
            .map_err(|error| format!("Canvas viewport is invalid: {error}"))?;
        let viewport = viewport
            .validate()
            .map_err(|error| format!("Canvas viewport is invalid: {error}"))?;
        if viewport.can_render_directly(width, height) {
            return self.try_render_layered_base_for(namespace, content, width, height);
        }
        let mut source_content = content.clone();
        source_content
            .as_object_mut()
            .ok_or_else(|| "Canvas output content must be an object.".to_string())?
            .remove("canvasViewport");
        let source = self.try_render_layered_base_for(
            namespace,
            &source_content,
            viewport.canvas_width,
            viewport.canvas_height,
        )?;
        crop_scale_layered_frame(&source, viewport, width, height)
    }

    pub fn try_render_key_fill_pair_for(
        &mut self,
        namespace: &str,
        content: &Value,
        width: usize,
        height: usize,
    ) -> Result<(NativeLayeredFrame, NativeLayeredFrame), String> {
        let signal: crate::key_fill::KeyFillSignal = serde_json::from_value(
            content
                .get("keyFillSignal")
                .cloned()
                .ok_or("Key/fill source signal is unavailable.")?,
        )
        .map_err(|error| format!("Invalid key/fill source signal: {error}"))?;
        if signal == crate::key_fill::KeyFillSignal::Key {
            return Err("A key signal cannot be rendered as a fill source.".into());
        }
        let mut source = content.clone();
        let source_fields = source
            .as_object_mut()
            .ok_or("Key/fill source must be an object.")?;
        for field in ["keyFillSignal", "keyFillSourceId", "keyFillPartnerId"] {
            source_fields.remove(field);
        }
        if content
            .get("projectorCorrection")
            .is_none_or(Value::is_null)
        {
            source_fields.insert("backgroundCss".into(), Value::String("transparent".into()));
        }
        let mut frame = self.try_render_layered_for(namespace, &source, width, height)?;
        frame.layers.retain(|layer| layer.id != "background-color");
        Ok((
            apply_key_fill_signal(frame.clone(), signal),
            apply_key_fill_signal(frame, crate::key_fill::KeyFillSignal::Key),
        ))
    }

    fn try_render_layered_base_for(
        &mut self,
        namespace: &str,
        content: &Value,
        width: usize,
        height: usize,
    ) -> Result<NativeLayeredFrame, String> {
        self.try_render_layered_base_for_at(namespace, content, width, height, Instant::now())
    }

    fn try_render_layered_base_for_at(
        &mut self,
        namespace: &str,
        content: &Value,
        width: usize,
        height: usize,
        transition_now: Instant,
    ) -> Result<NativeLayeredFrame, String> {
        if let Some(pattern) = content.get("testPattern").filter(|value| !value.is_null()) {
            self.object_build_transitions.reset_namespace(namespace);
            let seconds = crate::test_pattern::clock_seconds();
            let label = content
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let pixels = crate::test_pattern::render(
                pattern,
                width,
                height,
                seconds,
                label,
                &mut self.renderer,
            )?;
            let animated = crate::test_pattern::is_animated(pattern);
            let mut hasher = DefaultHasher::new();
            pattern.to_string().hash(&mut hasher);
            label.hash(&mut hasher);
            width.hash(&mut hasher);
            height.hash(&mut hasher);
            if animated {
                seconds.to_bits().hash(&mut hasher);
            }
            return Ok(NativeLayeredFrame {
                width,
                height,
                dynamic_media: animated,
                layers: vec![NativeLayerFrame {
                    id: format!("{namespace}:test-pattern"),
                    visual_key: hasher.finish(),
                    pixels: Arc::new(pixels),
                    bounds: Some(NativeLayerBounds {
                        x: 0,
                        y: 0,
                        width,
                        height,
                    }),
                    dynamic_media: false,
                    replace: true,
                }],
            });
        }
        validate_object_links(content)?;
        validate_stage_previews(content)?;
        let timed = self.with_slide_fill_clocks(namespace, content);
        let content = timed.as_ref().unwrap_or(content);
        let object_build_entry = content
            .get("objectBuildEntryKey")
            .and_then(Value::as_str)
            .filter(|key| !key.trim().is_empty());
        let mut live_theme_slide = None;
        let build_visuals = match (object_build_entry, content.get("presentationThemeSlide")) {
            (Some(entry_key), Some(slide_value))
                if slide_value.get("ignoreBuilds").and_then(Value::as_bool) != Some(true) =>
            {
                let slide = parse_scene_slide_with_build_layers(slide_value, true)?;
                let build_step = slide
                    .source
                    .get("buildStep")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| {
                        "Live object builds require a numeric presentationThemeSlide.buildStep."
                            .to_string()
                    })?;
                let build_clock = content
                    .get("objectBuildStepStartedAtMsByStep")
                    .map(|value| {
                        native_object_builds::BuildStepClock::from_value(
                            value,
                            chrono::Utc::now().timestamp_millis() as f64,
                            build_step,
                        )
                    })
                    .transpose()?;
                let visuals = self.object_build_transitions.evaluate_slide_with_clock(
                    namespace,
                    entry_key,
                    build_step,
                    false,
                    slide.layers.iter().map(|layer| &layer.source),
                    transition_now,
                    build_clock.as_ref(),
                )?;
                live_theme_slide = Some(slide);
                Some(visuals)
            }
            _ => {
                self.object_build_transitions.reset_namespace(namespace);
                None
            }
        };
        let build_animation_pending = build_visuals
            .as_ref()
            .is_some_and(|visuals| visuals.iter().any(|visual| visual.active || visual.pending));
        native_chart::prepare(content, &mut self.renderer)?;
        native_stage_previews::prepare(
            content,
            &mut self.stage_preview_renderers,
            &mut self.renderer,
        )?;
        let media_download_pending = prefetch_content(content);
        let now = native_output_elapsed_seconds();
        let scroll_elapsed_seconds = if content_has_scrolling_text(content, 0.0) {
            let key = content
                .get("scrollContentKey")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| scroll_content_key_for_value(content));
            native_scroll_elapsed_seconds(&key, now)
        } else {
            now
        };
        let selection = json!([
            content["backgroundMediaPath"],
            content["backgroundSelectionId"]
        ]);
        let playback = &content["backgroundPlaybackState"];
        let selection_state = self
            .media_selections
            .entry(namespace.to_string())
            .or_insert_with(|| {
                (
                    selection.clone(),
                    playback["currentTime"] == playback["inSeconds"],
                )
            });
        if selection_state.0 != selection {
            *selection_state = (selection, true);
        }
        self.renderer.prepare_video_start = selection_state.1;
        self.renderer.begin_frame();
        let mut layers = compose_layered_frame(
            namespace,
            content,
            width,
            height,
            scroll_elapsed_seconds,
            media_download_pending,
            &mut self.renderer,
            &mut self.background_image_cache,
            &mut self.layer_cache,
            build_visuals.as_deref(),
            live_theme_slide.as_ref(),
        );
        self.renderer.end_frame();
        selection_state.1 = self.renderer.prepare_video_start;
        // Loading and composing the first incoming frame must not consume its transition.
        let transition_now = Instant::now();
        let transitioning = self.apply_layer_transitions(
            namespace,
            content,
            &mut layers,
            width,
            height,
            transition_now,
        )?;
        let transitioning = self.apply_look_transition(
            namespace,
            content,
            &mut layers,
            width,
            height,
            transition_now,
        )? || transitioning;
        let dynamic_media = transitioning
            || build_animation_pending
            || media_download_pending
            || layers.iter().any(|layer| layer.dynamic_media);
        Ok(NativeLayeredFrame {
            width,
            height,
            layers,
            dynamic_media,
        })
    }
}

fn apply_key_fill_signal(
    frame: NativeLayeredFrame,
    signal: crate::key_fill::KeyFillSignal,
) -> NativeLayeredFrame {
    let mut flat = frame.flatten();
    crate::key_fill::convert_pixels(&mut flat.pixels, signal);
    let mut hasher = DefaultHasher::new();
    frame.width.hash(&mut hasher);
    frame.height.hash(&mut hasher);
    signal.hash(&mut hasher);
    for layer in &frame.layers {
        layer.visual_key.hash(&mut hasher);
    }
    NativeLayeredFrame {
        width: frame.width,
        height: frame.height,
        dynamic_media: frame.dynamic_media,
        layers: vec![NativeLayerFrame {
            id: "key-fill-signal".into(),
            visual_key: hasher.finish(),
            pixels: Arc::new(flat.pixels),
            bounds: Some(NativeLayerBounds {
                x: 0,
                y: 0,
                width: frame.width,
                height: frame.height,
            }),
            dynamic_media: frame.dynamic_media,
            replace: true,
        }],
    }
}

fn apply_projector_correction(
    frame: NativeLayeredFrame,
    correction: &crate::projector_correction::ProjectorCorrection,
) -> Result<NativeLayeredFrame, String> {
    crate::output_canvas::CanvasSize {
        width: frame.width,
        height: frame.height,
    }
    .validate()?;
    let expected_len = frame
        .width
        .checked_mul(frame.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Projector correction frame is too large.".to_string())?;
    let flattened = frame.flatten();
    if flattened.pixels.len() != expected_len {
        return Err("Projector correction frame buffer has an invalid size.".into());
    }

    let inverse = correction.inverse_homography()?;
    let mut pixels = vec![0_u8; expected_len];
    let corners = correction.corners;
    for y in 0..frame.height {
        let normalized_y = (y as f64 + 0.5) / frame.height as f64;
        for x in 0..frame.width {
            let normalized_x = (x as f64 + 0.5) / frame.width as f64;
            let output_point = crate::projector_correction::Point {
                x: normalized_x,
                y: normalized_y,
            };
            if !crate::projector_correction::contains_output_point(corners, output_point) {
                continue;
            }
            let source_point = inverse.map(normalized_x, normalized_y)?;
            if source_point.x < -1.0e-9
                || source_point.x > 1.0 + 1.0e-9
                || source_point.y < -1.0e-9
                || source_point.y > 1.0 + 1.0e-9
            {
                return Err(
                    "Projector correction perspective transform mapped an output pixel outside the source image.".into(),
                );
            }
            let sampled = sample_straight_alpha_bgra(
                &flattened.pixels,
                frame.width,
                frame.height,
                source_point.x.clamp(0.0, 1.0),
                source_point.y.clamp(0.0, 1.0),
            );
            if sampled[3] <= 0.0 {
                continue;
            }
            let edge_weight = correction.edge_weight(
                source_point.x.clamp(0.0, 1.0),
                source_point.y.clamp(0.0, 1.0),
            );
            let offset = (y * frame.width + x) * 4;
            // RGB values are straight-alpha encoded samples. Correction changes
            // their encoded color while retaining the sampled alpha exactly once.
            for (channel, gain) in [
                (2, correction.gain.red),
                (1, correction.gain.green),
                (0, correction.gain.blue),
            ] {
                let encoded = correction.color.map_or(sampled[channel], |color| {
                    color.apply(sampled[channel], channel)
                });
                let adjusted =
                    ((encoded * gain + correction.black_level) * edge_weight).clamp(0.0, 1.0);
                pixels[offset + channel] = (adjusted * 255.0).round() as u8;
            }
            pixels[offset + 3] = (sampled[3] * 255.0).round() as u8;
        }
    }

    let mut hasher = DefaultHasher::new();
    frame.width.hash(&mut hasher);
    frame.height.hash(&mut hasher);
    frame.dynamic_media.hash(&mut hasher);
    for layer in &frame.layers {
        layer.visual_key.hash(&mut hasher);
        layer.dynamic_media.hash(&mut hasher);
    }
    serde_json::to_string(correction)
        .map_err(|error| format!("Projector correction could not be serialized: {error}"))?
        .hash(&mut hasher);

    let dynamic_media = flattened.dynamic_media;
    Ok(NativeLayeredFrame {
        width: frame.width,
        height: frame.height,
        dynamic_media,
        layers: vec![NativeLayerFrame {
            id: "projector-correction".to_string(),
            visual_key: hasher.finish(),
            pixels: Arc::new(pixels),
            bounds: Some(NativeLayerBounds {
                x: 0,
                y: 0,
                width: frame.width,
                height: frame.height,
            }),
            dynamic_media,
            replace: true,
        }],
    })
}

fn sample_straight_alpha_bgra(
    pixels: &[u8],
    width: usize,
    height: usize,
    normalized_x: f64,
    normalized_y: f64,
) -> [f64; 4] {
    let source_x = (normalized_x * width as f64 - 0.5).clamp(0.0, (width - 1) as f64);
    let source_y = (normalized_y * height as f64 - 0.5).clamp(0.0, (height - 1) as f64);
    let x0 = source_x.floor() as usize;
    let y0 = source_y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let tx = source_x - x0 as f64;
    let ty = source_y - y0 as f64;
    let samples = [
        (x0, y0, (1.0 - tx) * (1.0 - ty)),
        (x1, y0, tx * (1.0 - ty)),
        (x0, y1, (1.0 - tx) * ty),
        (x1, y1, tx * ty),
    ];
    let mut alpha = 0.0;
    let mut premultiplied = [0.0; 3];
    for (x, y, weight) in samples {
        let offset = (y * width + x) * 4;
        let sample_alpha = f64::from(pixels[offset + 3]) / 255.0;
        alpha += sample_alpha * weight;
        for channel in 0..3 {
            premultiplied[channel] +=
                (f64::from(pixels[offset + channel]) / 255.0) * sample_alpha * weight;
        }
    }
    if alpha <= f64::EPSILON {
        return [0.0; 4];
    }
    [
        premultiplied[0] / alpha,
        premultiplied[1] / alpha,
        premultiplied[2] / alpha,
        alpha,
    ]
}

fn crop_scale_layered_frame(
    frame: &NativeLayeredFrame,
    viewport: crate::output_canvas::CanvasViewport,
    width: usize,
    height: usize,
) -> Result<NativeLayeredFrame, String> {
    let viewport = viewport.validate()?;
    crate::output_canvas::CanvasSize { width, height }.validate()?;
    if frame.width != viewport.canvas_width || frame.height != viewport.canvas_height {
        return Err("Canvas source frame dimensions do not match its viewport.".to_string());
    }
    if viewport.x == 0
        && viewport.y == 0
        && viewport.width == frame.width
        && viewport.height == frame.height
        && width == frame.width
        && height == frame.height
    {
        return Ok(frame.clone());
    }

    let source_len = frame
        .width
        .checked_mul(frame.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Canvas source frame is too large.".to_string())?;
    let destination_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Canvas output frame is too large.".to_string())?;
    let crop_len = viewport
        .width
        .checked_mul(viewport.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Canvas crop is too large.".to_string())?;
    let (fitted_width, fitted_height) = if width * viewport.height <= height * viewport.width {
        (
            width,
            ((viewport.height as u128 * width as u128 + viewport.width as u128 / 2)
                / viewport.width as u128)
                .clamp(1, height as u128) as usize,
        )
    } else {
        (
            ((viewport.width as u128 * height as u128 + viewport.height as u128 / 2)
                / viewport.height as u128)
                .clamp(1, width as u128) as usize,
            height,
        )
    };
    let fitted_x = (width - fitted_width) / 2;
    let fitted_y = (height - fitted_height) / 2;
    let mut layers = Vec::with_capacity(frame.layers.len());
    for layer in &frame.layers {
        if layer.pixels.len() != source_len {
            return Err(format!(
                "Canvas source layer '{}' has an invalid pixel buffer.",
                layer.id
            ));
        }
        let mut cropped = Vec::with_capacity(crop_len);
        let row_bytes = viewport.width * 4;
        for row in viewport.y..viewport.y + viewport.height {
            let start = (row * frame.width + viewport.x) * 4;
            cropped.extend_from_slice(&layer.pixels[start..start + row_bytes]);
        }
        let cropped =
            image::RgbaImage::from_raw(viewport.width as u32, viewport.height as u32, cropped)
                .ok_or_else(|| "Canvas crop pixel buffer is invalid.".to_string())?;
        let fitted_pixels = if viewport.width == fitted_width && viewport.height == fitted_height {
            cropped.into_raw()
        } else {
            let mut premultiplied = cropped.into_raw();
            for pixel in premultiplied.chunks_exact_mut(4) {
                let alpha = u16::from(pixel[3]);
                for channel in &mut pixel[..3] {
                    *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                }
            }
            let resized = image::RgbaImage::from_raw(
                viewport.width as u32,
                viewport.height as u32,
                premultiplied,
            )
            .ok_or_else(|| "Canvas crop pixel buffer is invalid.".to_string())?;
            let mut pixels = image::imageops::resize(
                &resized,
                fitted_width as u32,
                fitted_height as u32,
                image::imageops::FilterType::Triangle,
            )
            .into_raw();
            for pixel in pixels.chunks_exact_mut(4) {
                let alpha = u16::from(pixel[3]);
                if alpha == 0 {
                    pixel[..3].fill(0);
                } else {
                    for channel in &mut pixel[..3] {
                        *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                }
            }
            pixels
        };
        let mut pixels = vec![0; destination_len];
        let fitted_row_bytes = fitted_width * 4;
        for row in 0..fitted_height {
            let source_start = row * fitted_row_bytes;
            let destination_start = ((fitted_y + row) * width + fitted_x) * 4;
            pixels[destination_start..destination_start + fitted_row_bytes]
                .copy_from_slice(&fitted_pixels[source_start..source_start + fitted_row_bytes]);
        }
        if pixels.len() != destination_len {
            return Err("Canvas output pixel buffer has an invalid size.".to_string());
        }
        let intersects = layer.bounds.is_some_and(|bounds| {
            let left = bounds.x.max(viewport.x);
            let top = bounds.y.max(viewport.y);
            let right = bounds
                .x
                .saturating_add(bounds.width)
                .min(viewport.x + viewport.width);
            let bottom = bounds
                .y
                .saturating_add(bounds.height)
                .min(viewport.y + viewport.height);
            left < right && top < bottom
        });
        let mut hasher = DefaultHasher::new();
        layer.visual_key.hash(&mut hasher);
        viewport.canvas_width.hash(&mut hasher);
        viewport.canvas_height.hash(&mut hasher);
        viewport.x.hash(&mut hasher);
        viewport.y.hash(&mut hasher);
        viewport.width.hash(&mut hasher);
        viewport.height.hash(&mut hasher);
        width.hash(&mut hasher);
        height.hash(&mut hasher);
        layers.push(NativeLayerFrame {
            id: layer.id.clone(),
            visual_key: hasher.finish(),
            pixels: Arc::new(pixels),
            bounds: if layer.replace {
                Some(NativeLayerBounds {
                    x: 0,
                    y: 0,
                    width,
                    height,
                })
            } else {
                intersects.then_some(NativeLayerBounds {
                    x: fitted_x,
                    y: fitted_y,
                    width: fitted_width,
                    height: fitted_height,
                })
            },
            dynamic_media: layer.dynamic_media,
            replace: layer.replace,
        });
    }
    Ok(NativeLayeredFrame {
        width,
        height,
        layers,
        dynamic_media: frame.dynamic_media,
    })
}

impl NativeCompositor {
    /// The content with its slide fill videos bound to the shown slide occurrence.
    fn with_slide_fill_clocks(&mut self, namespace: &str, content: &Value) -> Option<Value> {
        let entry_key = content.get("slideEntryKey").unwrap_or(&Value::Null);
        let revision = self
            .fill_clocks
            .entry(namespace.to_string())
            .or_insert_with(|| native_fill_playback::FillClock::new(entry_key.clone()))
            .observe(entry_key);
        native_fill_playback::with_slide_fill_clocks(content, revision)
    }

    /// Replaces each changing layer (slide, media, props, messages) with a blend of what it
    /// showed before and what it shows now while its transition runs. Returns whether a
    /// transition frame was produced.
    fn apply_layer_transitions(
        &mut self,
        namespace: &str,
        content: &Value,
        layers: &mut Vec<NativeLayerFrame>,
        width: usize,
        height: usize,
        now: Instant,
    ) -> Result<bool, String> {
        let mut transitioning = false;
        let slide_context = TransitionLayer::Slide.context(content);
        let slide_key = slide_context
            .and_then(|context| context.get("key"))
            .or_else(|| content.get("slideEntryKey"))
            .filter(|key| !key.is_null());
        let previous_slide = self
            .layer_transitions
            .get(&(namespace.into(), TransitionLayer::Slide));
        let media_context = TransitionLayer::Media.context(content);
        let slide_has_media = slide_context
            .and_then(|context| context.get("includesMedia"))
            .and_then(Value::as_bool)
            == Some(true);
        let removes_slide_media = previous_slide.is_some_and(|state| state.includes_media)
            && media_context
                .and_then(|context| context.get("key"))
                .is_none_or(Value::is_null);
        let previous_key = previous_slide
            .map(|state| &state.key)
            .filter(|key| !key.is_null());
        let slide_media_take =
            (slide_has_media || removes_slide_media) && previous_key != slide_key;
        let media_pending =
            (slide_media_take || media_context.is_some()) && self.background_media_pending(content);
        let slide_waiting = slide_media_take
            && media_pending
            && slide_context
                .and_then(|context| context.get("transition"))
                .filter(|value| !value.is_null())
                .map(native_transitions::resolve)
                .transpose()?
                .flatten()
                .is_some();
        // Compose independent media before the slide, which may need it as a backdrop.
        for layer in [
            TransitionLayer::Messages,
            TransitionLayer::Props,
            TransitionLayer::Media,
            TransitionLayer::Slide,
        ] {
            let waiting_for_media = match layer {
                TransitionLayer::Media if slide_waiting => continue,
                TransitionLayer::Media => !slide_media_take && media_pending,
                TransitionLayer::Slide => slide_waiting,
                _ => false,
            };
            transitioning |= self.apply_layer_transition(
                layer,
                namespace,
                content,
                layers,
                (width, height),
                now,
                waiting_for_media,
                slide_media_take,
            )?;
        }
        Ok(transitioning)
    }

    fn background_media_pending(&self, content: &Value) -> bool {
        content["backgroundMediaType"].as_str().is_some_and(|kind| {
            matches!(kind, "image" | "video")
                && content
                    .get("backgroundMediaPath")
                    .or_else(|| content.get("backgroundMediaUrl"))
                    .and_then(local_media_path)
                    .is_none_or(|path| kind == "video" && !self.renderer.video_frame_ready(&path))
        })
    }

    /// When an Audience Look was applied (its token changed), replaces the whole frame with a dissolve
    /// from the previous frame to the new one over the look's duration. That fades layers the look turns
    /// off or on and crossfades a changed theme or mask. A duration of 0 cuts.
    fn apply_look_transition(
        &mut self,
        namespace: &str,
        content: &Value,
        layers: &mut Vec<NativeLayerFrame>,
        width: usize,
        height: usize,
        now: Instant,
    ) -> Result<bool, String> {
        let Some(look) = content.get("lookTransition").and_then(Value::as_object) else {
            self.look_transitions.remove(namespace);
            return Ok(false);
        };
        let token = look.get("token").cloned().unwrap_or(Value::Null);
        let seconds = look
            .get("seconds")
            .and_then(Value::as_f64)
            .ok_or("An Audience Look transition requires seconds.")?;
        let plane = |layers: &[NativeLayerFrame]| {
            NativeLayeredFrame {
                width,
                height,
                layers: layers.to_vec(),
                dynamic_media: false,
            }
            .flatten()
            .pixels
        };
        let first_frame = !self.look_transitions.contains_key(namespace);
        let state = self
            .look_transitions
            .entry(namespace.to_string())
            .or_default();
        if !first_frame && state.token != token {
            let spec = native_transitions::resolve(
                &json!({ "effectId": native_transitions::DISSOLVE_ID, "durationSeconds": seconds }),
            )?;
            state.active = spec.map(|spec| {
                let from = state
                    .active
                    .as_ref()
                    .and(state.displayed.take())
                    .filter(|pixels| pixels.len() == width * height * 4)
                    .unwrap_or_else(|| plane(&state.layers));
                native_transitions::ActiveTransition {
                    spec,
                    started: now,
                    from,
                }
            });
        }
        state.token = token;
        state.layers.clone_from(layers);
        let Some(active) = state.active.as_ref() else {
            return Ok(false);
        };
        let progress = active.progress(now);
        if progress >= 1.0 || active.from.len() != width * height * 4 {
            state.active = None;
            state.displayed = None;
            return Ok(false);
        }
        let to = plane(layers);
        let mut blended = vec![0; to.len()];
        native_transitions::blend_object(
            &active.spec,
            progress,
            &active.from,
            &to,
            &mut blended,
            width,
            height,
        );
        state.displayed = Some(blended.clone());
        let mut hasher = DefaultHasher::new();
        progress.to_bits().hash(&mut hasher);
        *layers = vec![NativeLayerFrame {
            id: "look-transition".into(),
            visual_key: hasher.finish(),
            pixels: Arc::new(blended),
            bounds: Some(NativeLayerBounds {
                x: 0,
                y: 0,
                width,
                height,
            }),
            dynamic_media: true,
            replace: true,
        }];
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_layer_transition(
        &mut self,
        layer: TransitionLayer,
        namespace: &str,
        content: &Value,
        layers: &mut Vec<NativeLayerFrame>,
        (width, height): (usize, usize),
        now: Instant,
        waiting_for_media: bool,
        slide_media_take: bool,
    ) -> Result<bool, String> {
        let state_key = (namespace.to_string(), layer);
        let context = layer.context(content);
        let slide = layer == TransitionLayer::Slide;
        if context.is_none() && !slide {
            self.layer_transitions.remove(&state_key);
            return Ok(false);
        }
        let key = context
            .and_then(|context| context.get("key"))
            .or_else(|| slide.then(|| content.get("slideEntryKey")).flatten())
            .cloned()
            .unwrap_or(Value::Null);
        let spec = match context
            .and_then(|context| context.get("transition"))
            .filter(|value| !value.is_null())
        {
            Some(value) => native_transitions::resolve(value)?,
            None => None,
        };
        let in_plane = |frame: &NativeLayerFrame, backdrop| {
            layer.contains(&frame.id) || (backdrop && layer_rank(&frame.id) <= 5)
        };
        let group = layers
            .iter()
            .filter(|frame| in_plane(frame, slide))
            .cloned()
            .collect::<Vec<_>>();
        // A slide with a replacement plane needs its backdrop on both sides of the blend.
        let has_slide_backdrop = |group: &[NativeLayerFrame]| {
            group
                .iter()
                .any(|frame| frame.replace && is_slide_layer(&frame.id))
        };
        let blendable = slide || !group.iter().any(|frame| frame.replace);
        let plane = |layers: Vec<NativeLayerFrame>| {
            NativeLayeredFrame {
                width,
                height,
                layers,
                dynamic_media: false,
            }
            .flatten()
            .pixels
        };
        let indices = |layers: &[NativeLayerFrame], replace: bool| {
            layers
                .iter()
                .enumerate()
                .filter_map(|(index, frame)| in_plane(frame, replace).then_some(index))
                .collect::<Vec<_>>()
        };
        let transition_frame = |pixels, replace, visual_key| NativeLayerFrame {
            id: layer.plane_id().into(),
            visual_key,
            pixels,
            bounds: Some(NativeLayerBounds {
                x: 0,
                y: 0,
                width,
                height,
            }),
            dynamic_media: true,
            replace,
        };
        let mut first_frame = !self.layer_transitions.contains_key(&state_key);
        let state = self.layer_transitions.entry(state_key).or_default();
        if state
            .layers
            .iter()
            .any(|frame| frame.pixels.len() != width * height * 4)
            || state
                .displayed
                .as_ref()
                .is_some_and(|pixels| pixels.len() != width * height * 4)
        {
            *state = LayerTransitionState::default();
            first_frame = true;
        }
        let outgoing = |state: &LayerTransitionState, replace: bool| {
            if let Some(pixels) = state.displayed.as_ref().filter(|_| state.active.is_some()) {
                if replace && !state.replace {
                    let mut backdrop = state
                        .layers
                        .iter()
                        .filter(|frame| !is_slide_layer(&frame.id))
                        .cloned()
                        .collect::<Vec<_>>();
                    backdrop.push(transition_frame(Arc::clone(pixels), false, 0));
                    return plane(backdrop);
                }
                return pixels.as_ref().clone();
            }
            plane(
                state
                    .layers
                    .iter()
                    .filter(|frame| in_plane(frame, replace))
                    .cloned()
                    .collect(),
            )
        };
        if waiting_for_media {
            // Keep the outgoing picture, including an interrupted fade, until decoding delivers a frame.
            let indices = indices(layers, slide);
            let outgoing = if state.active.is_some() {
                vec![transition_frame(Arc::new(outgoing(state, slide)), slide, 0)]
            } else {
                state.layers.clone()
            };
            let insert_at = indices.first().copied().unwrap_or_else(|| {
                layers
                    .iter()
                    .position(|frame| layer_rank(&frame.id) > state.rank)
                    .unwrap_or(layers.len())
            });
            for index in indices.iter().rev() {
                layers.remove(*index);
            }
            layers.splice(insert_at..insert_at, outgoing);
            return Ok(true);
        }
        if let Some(rank) = group.iter().map(|frame| layer_rank(&frame.id)).max() {
            state.rank = rank;
        }
        if !first_frame && state.key != key {
            let change = match layer {
                TransitionLayer::Media if slide_media_take => None,
                TransitionLayer::Props => {
                    match context.and_then(|context| {
                        props_change_transition(&state.key, &state.transitions, context)
                    }) {
                        Some(value) => native_transitions::resolve(value)?,
                        None => None,
                    }
                }
                // A cleared layer without its own transition leaves with the outgoing one's.
                _ if key.is_null() => spec.clone().or_else(|| state.spec.clone()),
                _ => spec.clone(),
            };
            state.active = match change.filter(|_| blendable) {
                Some(spec) => {
                    let replace = slide
                        && (slide_media_take
                            || has_slide_backdrop(&group)
                            || has_slide_backdrop(&state.layers)
                            || (state.active.is_some() && state.replace));
                    let from = outgoing(state, replace);
                    state.replace = replace;
                    Some(native_transitions::ActiveTransition {
                        spec,
                        started: now,
                        from,
                    })
                }
                None => None,
            };
        }
        state.key = key;
        state.spec = spec;
        state.includes_media = context
            .and_then(|context| context.get("includesMedia"))
            .and_then(Value::as_bool)
            == Some(true);
        state.transitions = context
            .and_then(|context| context.get("transitions"))
            .cloned()
            .unwrap_or(Value::Null);
        state.layers = group;
        let Some(active) = state.active.as_ref() else {
            state.displayed = None;
            return Ok(false);
        };
        let progress = active.progress(now);
        if progress >= 1.0 || active.from.len() != width * height * 4 || !blendable {
            state.active = None;
            state.displayed = None;
            return Ok(false);
        }
        let indices = indices(layers, state.replace);
        let to = plane(indices.iter().map(|index| layers[*index].clone()).collect());
        let mut blended = vec![0; to.len()];
        native_transitions::blend(
            &active.spec,
            progress,
            &active.from,
            &to,
            &mut blended,
            width,
            height,
        );
        let blended = Arc::new(blended);
        state.displayed = Some(Arc::clone(&blended));
        let anchor_rank = match layer {
            TransitionLayer::Slide => 1,
            _ => state.rank,
        };
        let insert_at = indices
            .last()
            .map(|index| index + 1 - indices.len())
            .unwrap_or_else(|| {
                layers
                    .iter()
                    .position(|frame| layer_rank(&frame.id) > anchor_rank)
                    .unwrap_or(layers.len())
            });
        for index in indices.iter().rev() {
            layers.remove(*index);
        }
        let mut hasher = DefaultHasher::new();
        progress.to_bits().hash(&mut hasher);
        layers.insert(
            insert_at.min(layers.len()),
            transition_frame(blended, state.replace, hasher.finish()),
        );
        Ok(true)
    }
}

/// The layers whose changes play library transitions on native output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TransitionLayer {
    Slide,
    Media,
    Props,
    Messages,
}

impl TransitionLayer {
    fn context(self, content: &Value) -> Option<&Map<String, Value>> {
        let layers = content.get("layerTransitions");
        let context = match self {
            Self::Slide => content.get("slideTransition"),
            Self::Media => layers.and_then(|value| value.get("media")),
            Self::Props => layers.and_then(|value| value.get("props")),
            Self::Messages => layers.and_then(|value| value.get("messages")),
        };
        context.and_then(Value::as_object)
    }

    fn contains(self, id: &str) -> bool {
        match self {
            Self::Slide => is_slide_layer(id),
            Self::Media => matches!(id, "background-media" | "foreground-media"),
            Self::Props => id.starts_with("prop-"),
            Self::Messages => id == "message" || id.starts_with("message-"),
        }
    }

    fn plane_id(self) -> &'static str {
        match self {
            Self::Slide => "slide-transition",
            Self::Media => "media-transition",
            Self::Props => "props-transition",
            Self::Messages => "messages-transition",
        }
    }
}

/// The transition of a prop change: the first prop that appears plays its own; otherwise
/// the first prop that leaves plays the one it had while it was shown.
fn props_change_transition<'a>(
    previous_key: &Value,
    previous_transitions: &'a Value,
    context: &'a Map<String, Value>,
) -> Option<&'a Value> {
    let ids = |value: Option<&Value>| {
        value
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let before = ids(Some(previous_key));
    let after = ids(context.get("key"));
    let lookup = |transitions: Option<&'a Value>, id: &str| {
        transitions
            .and_then(|transitions| transitions.get(id))
            .filter(|value| !value.is_null())
    };
    match after.iter().find(|id| !before.contains(id)) {
        Some(entering) => lookup(context.get("transitions"), entering),
        None => before
            .iter()
            .find(|id| !after.contains(id))
            .and_then(|leaving| lookup(Some(previous_transitions), leaving)),
    }
}

/// Stacking order of the native layers, for placing a cleared layer's outgoing plane.
fn layer_rank(id: &str) -> u8 {
    match id {
        "background-color" => 0,
        "background-media" => 2,
        "theme-background" => 3,
        "foreground-media" | "media-transition" => 4,
        "slide-transition" | "fallback-text" => 5,
        "props-transition" => 6,
        "messages-transition" | "message" => 9,
        _ if id.starts_with("video-input-") => 1,
        _ if id.starts_with("theme-layer-") => 5,
        _ if id.starts_with("ccli-layer-") => 5,
        _ if id.starts_with("announcement-") => 6,
        _ if id.starts_with("prop-") => 6,
        _ if id.starts_with("stage-") => 7,
        _ if id.starts_with("mask") => 8,
        _ if id.starts_with("message-") => 9,
        _ => 10,
    }
}

fn is_slide_layer(id: &str) -> bool {
    id == "theme-background"
        || id == "fallback-text"
        || id.starts_with("theme-layer-")
        || id.starts_with("ccli-layer-")
}

impl Default for NativeCompositor {
    fn default() -> Self {
        Self::new()
    }
}

struct RenderTarget<'a> {
    pixels: &'a mut [u8],
    width: usize,
    height: usize,
    dynamic_media: bool,
    scroll_elapsed_seconds: f64,
}

/// The renderer's CSS/SVG shape vocabulary expressed as normalized geometry.
///
/// Keeping this as a small raster mask rather than a second scene schema lets
/// fills, media fills, and borders all use the same clipping rules. The
/// normalized coordinates are local to a layer's bounds, including custom paths.
#[derive(Debug, Clone)]
enum ShapeGeometry {
    Rectangle,
    RoundedRectangle {
        radius_x: f32,
        radius_y: f32,
    },
    Ellipse,
    Polygon(Vec<(f32, f32)>),
    Path {
        points: Vec<(f32, f32)>,
        closed: bool,
    },
}

#[derive(Clone, Copy)]
struct PathAnchor {
    position: (f32, f32),
    incoming: Option<(f32, f32)>,
    outgoing: Option<(f32, f32)>,
    curved: bool,
}

fn layer_point(value: &Value) -> Option<(f64, f64)> {
    Some((json_number(value.get("x"))?, json_number(value.get("y"))?))
}

fn finite_f32(value: f64) -> Option<f32> {
    let value = value as f32;
    value.is_finite().then_some(value)
}

fn append_path_segment(points: &mut Vec<(f32, f32)>, start: PathAnchor, end: PathAnchor) {
    if start.curved || end.curved {
        let (Some(control1), Some(control2)) = (start.outgoing, end.incoming) else {
            return;
        };
        flatten_cubic(start.position, control1, control2, end.position, 0, points);
    } else {
        points.push(end.position);
    }
}

fn path_curve_handles_complete(anchors: &[PathAnchor], closed: bool) -> bool {
    let complete = |start: &PathAnchor, end: &PathAnchor| {
        !(start.curved || end.curved) || (start.outgoing.is_some() && end.incoming.is_some())
    };
    anchors.windows(2).all(|pair| complete(&pair[0], &pair[1]))
        && (!closed || complete(anchors.last().unwrap(), &anchors[0]))
}

fn flatten_cubic(
    start: (f32, f32),
    control1: (f32, f32),
    control2: (f32, f32),
    end: (f32, f32),
    depth: u8,
    points: &mut Vec<(f32, f32)>,
) {
    if depth >= 12 || cubic_flatness(start, control1, control2, end) <= 0.5 {
        points.push(end);
        return;
    }
    let midpoint =
        |left: (f32, f32), right: (f32, f32)| ((left.0 + right.0) * 0.5, (left.1 + right.1) * 0.5);
    let a = midpoint(start, control1);
    let b = midpoint(control1, control2);
    let c = midpoint(control2, end);
    let d = midpoint(a, b);
    let e = midpoint(b, c);
    let split = midpoint(d, e);
    flatten_cubic(start, a, d, split, depth + 1, points);
    flatten_cubic(split, e, c, end, depth + 1, points);
}

fn cubic_flatness(
    start: (f32, f32),
    control1: (f32, f32),
    control2: (f32, f32),
    end: (f32, f32),
) -> f32 {
    point_segment_distance(control1.0, control1.1, start.0, start.1, end.0, end.1).max(
        point_segment_distance(control2.0, control2.1, start.0, start.1, end.0, end.1),
    )
}

impl ShapeGeometry {
    fn from_layer(
        layer: &serde_json::Map<String, Value>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Self {
        if let Some(points) = layer.get("pathPoints").and_then(Value::as_array) {
            let local_coordinates = match layer.get("pathCoordinateSpace").and_then(Value::as_str) {
                Some("local") => true,
                Some("slide") | None => false,
                Some(_) => {
                    return Self::Path {
                        points: Vec::new(),
                        closed: false,
                    }
                }
            };
            let anchors = points
                .iter()
                .filter_map(|point| {
                    let point = point.as_object()?;
                    let point_x = json_number(point.get("x"))?;
                    let point_y = json_number(point.get("y"))?;
                    let point_x = if local_coordinates {
                        point_x
                    } else {
                        point_x - x
                    };
                    let point_y = if local_coordinates {
                        point_y
                    } else {
                        point_y - y
                    };
                    let coordinates = |key: &str| -> Option<(f32, f32)> {
                        let point = layer_point(point.get(key)?)?;
                        let x = if local_coordinates {
                            point.0
                        } else {
                            point.0 - x
                        };
                        let y = if local_coordinates {
                            point.1
                        } else {
                            point.1 - y
                        };
                        Some((finite_f32(x)?, finite_f32(y)?))
                    };
                    let curved = point
                        .get("curved")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let incoming = coordinates("q0");
                    let outgoing = coordinates("q1");
                    if curved && (incoming.is_none() || outgoing.is_none()) {
                        return None;
                    }
                    Some(PathAnchor {
                        position: (finite_f32(point_x)?, finite_f32(point_y)?),
                        incoming,
                        outgoing,
                        curved,
                    })
                })
                .collect::<Vec<_>>();
            if anchors.len() != points.len() || anchors.len() < 2 {
                return Self::Path {
                    points: Vec::new(),
                    closed: false,
                };
            }
            let closed = layer
                .get("closedPath")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !path_curve_handles_complete(&anchors, closed) {
                return Self::Path {
                    points: Vec::new(),
                    closed: false,
                };
            }
            let mut points = vec![anchors[0].position];
            for pair in anchors.windows(2) {
                append_path_segment(&mut points, pair[0], pair[1]);
            }
            if closed {
                append_path_segment(&mut points, *anchors.last().unwrap(), anchors[0]);
            }
            let (Some(width), Some(height)) = (finite_f32(width), finite_f32(height)) else {
                return Self::Path {
                    points: Vec::new(),
                    closed: false,
                };
            };
            if width <= 0.0 || height <= 0.0 {
                return Self::Path {
                    points: Vec::new(),
                    closed: false,
                };
            }
            let points = points
                .into_iter()
                .map(|(x, y)| (x / width, y / height))
                .collect::<Vec<_>>();
            if points.len() > 1 {
                return Self::Path { points, closed };
            }
        }

        let shape = layer
            .get("shape")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match shape.as_str() {
            "rounded-rect" => {
                let settings = layer.get("shapeSettings");
                let roundness = settings
                    .and_then(Value::as_object)
                    .and_then(|settings| settings.get("roundness"))
                    .map(|value| json_number(Some(value)).unwrap_or(0.0))
                    .unwrap_or_else(|| {
                        if settings.is_some_and(|settings| !settings.is_object()) {
                            0.0
                        } else {
                            0.14
                        }
                    });
                let radius = (width.min(height) * roundness * 0.5).max(0.0);
                Self::RoundedRectangle {
                    radius_x: (radius / width.max(1.0)).min(0.5) as f32,
                    radius_y: (radius / height.max(1.0)).min(0.5) as f32,
                }
            }
            "ellipse" => Self::Ellipse,
            "triangle" => Self::Polygon(normalized_polygon(&[
                (50.0, 0.0),
                (100.0, 100.0),
                (0.0, 100.0),
            ])),
            "right-triangle" => Self::Polygon(normalized_polygon(&[
                (0.0, 0.0),
                (100.0, 100.0),
                (0.0, 100.0),
            ])),
            "rhombus" => Self::Polygon(normalized_polygon(&[
                (50.0, 0.0),
                (100.0, 50.0),
                (50.0, 100.0),
                (0.0, 50.0),
            ])),
            "polygon" => {
                let side_setting = layer
                    .get("shapeSettings")
                    .and_then(Value::as_object)
                    .and_then(|settings| settings.get("sides"));
                if let Some(sides) = side_setting {
                    Self::Polygon(sides.as_u64().and_then(regular_polygon).unwrap_or_default())
                } else if layer
                    .get("shapeSettings")
                    .is_some_and(|settings| !settings.is_object())
                {
                    Self::Polygon(Vec::new())
                } else {
                    Self::Polygon(normalized_polygon(&[
                        (25.0, 5.0),
                        (75.0, 5.0),
                        (100.0, 50.0),
                        (75.0, 95.0),
                        (25.0, 95.0),
                        (0.0, 50.0),
                    ]))
                }
            }
            "star" => {
                let settings = layer.get("shapeSettings").and_then(Value::as_object);
                if let Some(settings) = settings.filter(|settings| {
                    settings.contains_key("points") || settings.contains_key("innerRadius")
                }) {
                    // The inner radius is a fraction of the outer one; imported ProPresenter stars carry both.
                    let points = settings.get("points").and_then(Value::as_u64).unwrap_or(5);
                    let inner = settings
                        .get("innerRadius")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.4);
                    Self::Polygon(regular_star(points, inner).unwrap_or_default())
                } else {
                    Self::Polygon(normalized_polygon(&[
                        (50.0, 0.0),
                        (61.0, 35.0),
                        (98.0, 35.0),
                        (68.0, 56.0),
                        (79.0, 91.0),
                        (50.0, 70.0),
                        (21.0, 91.0),
                        (32.0, 56.0),
                        (2.0, 35.0),
                        (39.0, 35.0),
                    ]))
                }
            }
            "arrow" => {
                let settings = layer.get("shapeSettings");
                let corner = settings
                    .and_then(Value::as_object)
                    .and_then(|settings| settings.get("arrowCorner"));
                if let Some(corner) = corner {
                    Self::Polygon(normalized_arrow(corner).unwrap_or_default())
                } else if settings.is_some_and(|settings| !settings.is_object()) {
                    Self::Polygon(Vec::new())
                } else {
                    Self::Polygon(normalized_polygon(&[
                        (0.0, 35.0),
                        (62.0, 35.0),
                        (62.0, 15.0),
                        (100.0, 50.0),
                        (62.0, 85.0),
                        (62.0, 65.0),
                        (0.0, 65.0),
                    ]))
                }
            }
            "double-arrow" => Self::Polygon(normalized_polygon(&[
                (0.0, 50.0),
                (28.0, 15.0),
                (28.0, 35.0),
                (72.0, 35.0),
                (72.0, 15.0),
                (100.0, 50.0),
                (72.0, 85.0),
                (72.0, 65.0),
                (28.0, 65.0),
                (28.0, 85.0),
            ])),
            _ => Self::Rectangle,
        }
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        if !(x.is_finite() && y.is_finite()) {
            return false;
        }
        match self {
            Self::Rectangle => (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
            Self::RoundedRectangle { radius_x, radius_y } => {
                if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                    return false;
                }
                let radius_x = (*radius_x).min(0.5);
                let radius_y = (*radius_y).min(0.5);
                if radius_x <= 0.0 || radius_y <= 0.0 {
                    return true;
                }
                let corner_x = if x < radius_x {
                    radius_x
                } else if x > 1.0 - radius_x {
                    1.0 - radius_x
                } else {
                    return true;
                };
                let corner_y = if y < radius_y {
                    radius_y
                } else if y > 1.0 - radius_y {
                    1.0 - radius_y
                } else {
                    return true;
                };
                let dx = (x - corner_x) / radius_x;
                let dy = (y - corner_y) / radius_y;
                dx * dx + dy * dy <= 1.0
            }
            Self::Ellipse => {
                if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                    return false;
                }
                let dx = x - 0.5;
                let dy = y - 0.5;
                dx * dx + dy * dy <= 0.25
            }
            Self::Polygon(points) => point_in_polygon(points, x, y),
            Self::Path { points, closed } => *closed && point_in_polygon(points, x, y),
        }
    }

    fn is_rectangle(&self) -> bool {
        matches!(self, Self::Rectangle)
    }

    fn is_closed(&self) -> bool {
        !matches!(self, Self::Path { closed: false, .. })
    }
}

fn normalized_polygon(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    points
        .iter()
        .map(|(x, y)| (*x / 100.0, *y / 100.0))
        .collect()
}

fn normalized_arrow(corner: &Value) -> Option<Vec<(f32, f32)>> {
    let (x, y) = layer_point(corner)?;
    let (x, y) = (finite_f32(x)?, finite_f32(y)?);
    Some(vec![
        (0.0, y),
        (x, y),
        (x, 0.0),
        (1.0, 0.5),
        (x, 1.0),
        (x, 1.0 - y),
        (0.0, 1.0 - y),
    ])
}

fn regular_star(points: u64, inner: f64) -> Option<Vec<(f32, f32)>> {
    if !(3..=256).contains(&points) || !(0.0..=1.0).contains(&inner) {
        return None;
    }
    let start = -std::f32::consts::FRAC_PI_2;
    let step = std::f32::consts::PI / points as f32;
    Some(
        (0..points * 2)
            .map(|index| {
                let angle = start + index as f32 * step;
                let radius = if index % 2 == 0 {
                    0.5
                } else {
                    0.5 * inner as f32
                };
                (0.5 + radius * angle.cos(), 0.5 + radius * angle.sin())
            })
            .collect(),
    )
}

fn regular_polygon(sides: u64) -> Option<Vec<(f32, f32)>> {
    if !(3..=256).contains(&sides) {
        return None;
    }
    let start = -std::f32::consts::FRAC_PI_2;
    Some(
        (0..sides)
            .map(|index| {
                let angle = start + index as f32 * std::f32::consts::TAU / sides as f32;
                (0.5 + 0.5 * angle.cos(), 0.5 + 0.5 * angle.sin())
            })
            .collect(),
    )
}

fn point_in_polygon(points: &[(f32, f32)], x: f32, y: f32) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = points[points.len() - 1];
    for current in points {
        let crosses = (current.1 > y) != (previous.1 > y);
        if crosses {
            let intersection =
                (previous.0 - current.0) * (y - current.1) / (previous.1 - current.1) + current.0;
            if x < intersection {
                inside = !inside;
            }
        }
        previous = *current;
    }
    inside
}

fn json_number(value: Option<&Value>) -> Option<f64> {
    let parsed = match value? {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    parsed.is_finite().then_some(parsed)
}

fn normalize_degrees(value: f64) -> f64 {
    let value = value % 360.0;
    if value < 0.0 {
        value + 360.0
    } else {
        value
    }
}

fn native_output_elapsed_seconds() -> f64 {
    NATIVE_OUTPUT_CLOCK
        .get_or_init(Instant::now)
        .elapsed()
        .as_secs_f64()
}

/// Return elapsed time from the first native frame for this cue.
///
/// The key is supplied by the Rust output boundary and changes whenever a
/// source presents new visual content. Keeping the small epoch cache shared
/// lets the display compositor and NDI compositor begin the same cue from the
/// same monotonic timestamp, without resetting on every playback snapshot.
fn native_scroll_elapsed_seconds(key: &str, now: f64) -> f64 {
    let epochs = NATIVE_SCROLL_EPOCHS.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut epochs) = epochs.lock() else {
        return 0.0;
    };
    if let Some(epoch) = epochs.get_mut(key) {
        epoch.last_used_at = now;
        return (now - epoch.started_at).max(0.0);
    }

    if epochs.len() >= MAX_NATIVE_SCROLL_EPOCHS {
        let oldest_key = epochs
            .iter()
            .min_by(|left, right| {
                left.1
                    .last_used_at
                    .partial_cmp(&right.1.last_used_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(key, _)| key.clone());
        if let Some(oldest_key) = oldest_key {
            epochs.remove(&oldest_key);
        }
    }
    epochs.insert(
        key.to_string(),
        ScrollEpoch {
            started_at: now,
            last_used_at: now,
        },
    );
    0.0
}

fn layer_visual_key(value: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    hasher.finish()
}

fn layer_pixel_bounds(
    pixels: &[u8],
    width: usize,
    height: usize,
    replace: bool,
) -> Option<NativeLayerBounds> {
    if replace {
        return (width > 0 && height > 0).then_some(NativeLayerBounds {
            x: 0,
            y: 0,
            width,
            height,
        });
    }
    if width == 0 || height == 0 {
        return None;
    }

    let mut left = width;
    let mut top = height;
    let mut right = 0;
    let mut bottom = 0;
    for (index, pixel) in pixels.chunks_exact(4).enumerate() {
        if pixel[3] == 0 {
            continue;
        }
        let x = index % width;
        let y = index / width;
        left = left.min(x);
        top = top.min(y);
        right = right.max(x + 1);
        bottom = bottom.max(y + 1);
    }
    if right > left && bottom > top {
        Some(NativeLayerBounds {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
    } else {
        None
    }
}

fn selected_content(content: &Value, keys: &[&str]) -> Value {
    let mut selected = serde_json::Map::new();
    for key in keys {
        if let Some(value) = content.get(*key) {
            selected.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(selected)
}

fn render_native_layer<F>(
    namespace: &str,
    id: String,
    key_source: Value,
    dynamic_media: bool,
    width: usize,
    height: usize,
    scroll_elapsed_seconds: f64,
    cache: &mut NativeLayerCache,
    draw: F,
) -> NativeLayerFrame
where
    F: FnOnce(&mut RenderTarget<'_>),
{
    let replace = matches!(id.as_str(), "background-color" | "theme-background");
    let visual_key = layer_visual_key(&key_source);
    let cache_key = (namespace.to_string(), id.clone(), visual_key, width, height);
    if !dynamic_media {
        if let Some(cached) = cache.get(&cache_key) {
            return NativeLayerFrame {
                id,
                visual_key,
                pixels: Arc::clone(&cached.pixels),
                bounds: cached.bounds,
                dynamic_media: false,
                replace,
            };
        }
    }
    let mut pixels = vec![0; width.saturating_mul(height).saturating_mul(4)];
    let mut target = RenderTarget {
        pixels: &mut pixels,
        width,
        height,
        dynamic_media,
        scroll_elapsed_seconds,
    };
    draw(&mut target);
    let dynamic_media = target.dynamic_media;
    let bounds = layer_pixel_bounds(&pixels, width, height, replace);
    let pixels = Arc::new(pixels);
    if !dynamic_media {
        cache.retain(|(cached_namespace, cached_id, _, _, _), _| {
            cached_namespace != namespace || cached_id != &id
        });
        cache.insert(
            cache_key,
            CachedLayerPixels {
                pixels: Arc::clone(&pixels),
                bounds,
            },
        );
    }
    NativeLayerFrame {
        id,
        visual_key,
        pixels,
        bounds,
        dynamic_media,
        replace,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_backdrop_dependent_layer<F>(
    backdrop: &[NativeLayerFrame],
    namespace: &str,
    id: String,
    key_source: Value,
    dynamic_media: bool,
    width: usize,
    height: usize,
    scroll_elapsed_seconds: f64,
    cache: &mut NativeLayerCache,
    draw: F,
) -> NativeLayerFrame
where
    F: FnOnce(&mut RenderTarget<'_>),
{
    let backdrop_key = backdrop
        .iter()
        .map(|layer| (&layer.id, layer.visual_key, layer.dynamic_media))
        .collect::<Vec<_>>();
    let visual_key = layer_visual_key(&json!({
        "backdrop": backdrop_key,
        "layer": key_source,
    }));
    let dynamic_media = dynamic_media || backdrop.iter().any(|layer| layer.dynamic_media);
    let cache_key = (namespace.to_string(), id.clone(), visual_key, width, height);
    if !dynamic_media {
        if let Some(cached) = cache.get(&cache_key) {
            return NativeLayerFrame {
                id,
                visual_key,
                pixels: Arc::clone(&cached.pixels),
                bounds: cached.bounds,
                dynamic_media: false,
                replace: true,
            };
        }
    }

    let mut pixels = NativeLayeredFrame {
        width,
        height,
        layers: backdrop.to_vec(),
        dynamic_media,
    }
    .flatten()
    .pixels;
    let mut target = RenderTarget {
        pixels: &mut pixels,
        width,
        height,
        dynamic_media,
        scroll_elapsed_seconds,
    };
    draw(&mut target);
    let dynamic_media = target.dynamic_media;
    let bounds = layer_pixel_bounds(&pixels, width, height, true);
    let pixels = Arc::new(pixels);
    if !dynamic_media {
        cache.retain(|(cached_namespace, cached_id, _, _, _), _| {
            cached_namespace != namespace || cached_id != &id
        });
        cache.insert(
            cache_key,
            CachedLayerPixels {
                pixels: Arc::clone(&pixels),
                bounds,
            },
        );
    }
    NativeLayerFrame {
        id,
        visual_key,
        pixels,
        bounds,
        dynamic_media,
        replace: true,
    }
}

fn scene_layer_key_source(
    slide: &SceneSlide,
    scene_layer: &SceneLayer,
    fallback: &str,
    linked_text_context: &Value,
    use_presentation_text: bool,
) -> Value {
    let layer = if scene_layer.kind == SceneLayerKind::Text || scene_layer.text_layer {
        json!({
            "source": scene_layer.source,
            "fallback": fallback,
            "linkedTextContext": linked_text_context,
            "presentationText": use_presentation_text,
        })
    } else {
        Value::Object(scene_layer.source.clone())
    };
    json!({
        "slideCanvas": { "width": slide.width, "height": slide.height },
        "slideTextStyle": slide.source.get("textStyle"),
        "objectLinkDependencies": if scene_layer.source.get("fillType").and_then(Value::as_str) == Some("slide-object")
            || merged_text_style(&slide.source, &scene_layer.source).get("linkedTextSource").and_then(Value::as_str) == Some("Object") {
                json!({"layers": slide.source.get("layers"), "fallback": fallback, "presentationText": use_presentation_text})
            } else { Value::Null },
        "dataFillContext": if matches!(scene_layer.source.get("fillType").and_then(Value::as_str), Some("slide-object" | "group-color" | "label-color" | "capture-status-color" | "video-input" | "current-slide-image" | "next-slide-image" | "screen-preview" | "chord-chart")) { Some(linked_text_context) } else { None },
        "layer": layer,
    })
}

fn scene_layer_uses_backdrop(slide: &SceneSlide, layer: &SceneLayer) -> bool {
    if layer.source.get("fillType").and_then(Value::as_str) == Some("background-effect") {
        return true;
    }
    text_layer_uses_backdrop(slide, layer)
}

fn text_layer_uses_backdrop(slide: &SceneSlide, layer: &SceneLayer) -> bool {
    if layer.kind != SceneLayerKind::Text && !layer.text_layer {
        return false;
    }
    merged_text_style(&slide.source, &layer.source)
        .get("textFillMode")
        .and_then(Value::as_str)
        .is_some_and(|mode| {
            mode.eq_ignore_ascii_case("Text Cut Out")
                || mode.eq_ignore_ascii_case("Background Invert")
                || mode.eq_ignore_ascii_case("Background Blur")
        })
}

fn apply_object_build_visual(
    layer: &mut NativeLayerFrame,
    visual: &native_object_builds::BuildVisual,
    beneath: &[NativeLayerFrame],
    width: usize,
    height: usize,
) {
    if visual.active {
        layer.dynamic_media = true;
    }
    native_object_effects::apply(layer, &visual.effects, beneath, width, height);
}

#[allow(clippy::too_many_arguments)]
fn apply_text_reveal_child_effects(
    layer: &mut NativeLayerFrame,
    reveal: &native_object_builds::TextRevealVisual,
    layer_key: &Value,
    namespace: &str,
    id_prefix: &str,
    slide: &SceneSlide,
    scene_layer: &SceneLayer,
    fallback: &str,
    linked_text_context: &Value,
    width: usize,
    height: usize,
    scroll_elapsed_seconds: f64,
    renderer: &mut NativeRenderRuntime,
    cache: &mut NativeLayerCache,
    beneath: &[NativeLayerFrame],
) {
    for (child_index, effects) in &reveal.child_effects {
        let mut isolated = reveal.clone();
        isolated.child_opacities.clear();
        isolated.child_opacities.insert(*child_index, 1.0);
        isolated.child_effects.clear();
        isolated.isolated_child = Some(*child_index);
        let key_source = json!({
            "base": layer_key,
            "textRevealVisual": isolated.cache_value(),
        });
        let id = format!("{id_prefix}-text-child-{child_index}");
        let draw = |target: &mut RenderTarget<'_>| {
            draw_scene_layer(
                target,
                slide,
                scene_layer,
                fallback,
                linked_text_context,
                true,
                false,
                renderer,
                Some(&isolated),
            );
        };

        if text_layer_uses_backdrop(slide, scene_layer) {
            let mut backdrop = beneath.to_vec();
            backdrop.push(layer.clone());
            let mut child = render_backdrop_dependent_layer(
                &backdrop,
                namespace,
                id,
                key_source,
                true,
                width,
                height,
                scroll_elapsed_seconds,
                cache,
                draw,
            );
            native_object_effects::apply(&mut child, effects, &backdrop, width, height);
            *layer = child;
        } else {
            let mut child = render_native_layer(
                namespace,
                id,
                key_source,
                true,
                width,
                height,
                scroll_elapsed_seconds,
                cache,
                draw,
            );
            native_object_effects::apply(&mut child, effects, &[], width, height);
            let mut hasher = DefaultHasher::new();
            layer.visual_key.hash(&mut hasher);
            child.visual_key.hash(&mut hasher);
            let combined = NativeLayeredFrame {
                width,
                height,
                layers: vec![layer.clone(), child.clone()],
                dynamic_media: true,
            }
            .flatten();
            layer.pixels = Arc::new(combined.pixels);
            layer.visual_key = hasher.finish();
            layer.dynamic_media = true;
            layer.bounds =
                layer_pixel_bounds(layer.pixels.as_slice(), width, height, layer.replace);
        }
    }
}

fn compose_layered_frame(
    namespace: &str,
    content: &Value,
    width: usize,
    height: usize,
    scroll_elapsed_seconds: f64,
    media_download_pending: bool,
    renderer: &mut NativeRenderRuntime,
    background_image_cache: &mut VecDeque<CachedBackgroundImage>,
    cache: &mut NativeLayerCache,
    build_visuals: Option<&[native_object_builds::BuildVisual]>,
    live_theme_slide: Option<&SceneSlide>,
) -> Vec<NativeLayerFrame> {
    let mut layers = Vec::new();
    let background = content
        .get("backgroundCss")
        .filter(|value| value.as_str().is_some_and(|value| !value.trim().is_empty()))
        .or_else(|| content.get("background"))
        .and_then(background_css)
        .unwrap_or_else(|| "#000000".to_string());
    layers.push(render_native_layer(
        namespace,
        "background-color".to_string(),
        Value::String(background.clone()),
        false,
        width,
        height,
        scroll_elapsed_seconds,
        cache,
        |target| paint_css_background(&mut target.pixels, width, height, &background),
    ));

    if let Some(slide) = content
        .get("videoInputSlide")
        .and_then(|value| parse_scene_slide(value).ok())
    {
        push_scene_layers(
            &mut layers,
            "video-input",
            namespace,
            &slide,
            "",
            &Value::Null,
            false,
            width,
            height,
            scroll_elapsed_seconds,
            media_download_pending,
            renderer,
            cache,
        );
    }
    let media_is_foreground = content
        .get("backgroundBehavior")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("foreground"));
    if !media_is_foreground {
        push_background_media_layer(
            &mut layers,
            "background-media",
            namespace,
            content,
            width,
            height,
            scroll_elapsed_seconds,
            media_download_pending,
            renderer,
            background_image_cache,
            cache,
        );
    }

    let text = content
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| content.get("title").and_then(Value::as_str))
        .unwrap_or_default();
    let linked_text_context = content.get("linkedTextContext").unwrap_or(&Value::Null);
    let parsed_theme_slide = if live_theme_slide.is_none() {
        content
            .get("presentationThemeSlide")
            .and_then(|value| parse_scene_slide(value).ok())
    } else {
        None
    };
    let theme_slide = live_theme_slide.or(parsed_theme_slide.as_ref());
    if let Some(slide) = theme_slide.as_ref() {
        if slide
            .source
            .get("backgroundEnabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            if let Some(background) = slide.source.get("background").and_then(background_css) {
                layers.push(render_native_layer(
                    namespace,
                    "theme-background".to_string(),
                    Value::String(background.clone()),
                    false,
                    width,
                    height,
                    scroll_elapsed_seconds,
                    cache,
                    |target| paint_css_background(&mut target.pixels, width, height, &background),
                ));
            }
        }
    }
    if media_is_foreground {
        push_background_media_layer(
            &mut layers,
            "foreground-media",
            namespace,
            content,
            width,
            height,
            scroll_elapsed_seconds,
            media_download_pending,
            renderer,
            background_image_cache,
            cache,
        );
    }

    let mut rendered_theme_text = false;
    if let Some(slide) = theme_slide.as_ref() {
        for (index, scene_layer) in slide.layers.iter().enumerate() {
            rendered_theme_text |=
                scene_layer.kind == SceneLayerKind::Text || scene_layer.text_layer;
            let build_visual = build_visuals.and_then(|visuals| visuals.get(index));
            if build_visual.is_some_and(|visual| !visual.visible) {
                continue;
            }
            let text_reveal_visual = build_visual.and_then(|visual| visual.text_reveal.as_ref());
            let mut key_source =
                scene_layer_key_source(slide, scene_layer, text, linked_text_context, true);
            if let Some(reveal) = text_reveal_visual {
                key_source = json!({
                    "layer": key_source,
                    "textRevealVisual": reveal.cache_value(),
                });
            }
            let child_layer_key = key_source.clone();
            let dynamic = (!renderer.static_images
                && content_requires_continuous_render(&key_source))
                || text_reveal_visual.is_some_and(|visual| visual.active || visual.pending)
                || (media_download_pending && contains_media_source(&key_source));
            let id = format!("theme-layer-{index}");
            let layer = if scene_layer_uses_backdrop(slide, scene_layer) {
                render_backdrop_dependent_layer(
                    &layers,
                    namespace,
                    id,
                    key_source,
                    dynamic,
                    width,
                    height,
                    scroll_elapsed_seconds,
                    cache,
                    |target| {
                        draw_scene_layer(
                            target,
                            slide,
                            scene_layer,
                            text,
                            linked_text_context,
                            true,
                            true,
                            renderer,
                            text_reveal_visual,
                        );
                    },
                )
            } else {
                render_native_layer(
                    namespace,
                    id,
                    key_source,
                    dynamic,
                    width,
                    height,
                    scroll_elapsed_seconds,
                    cache,
                    |target| {
                        draw_scene_layer(
                            target,
                            slide,
                            scene_layer,
                            text,
                            linked_text_context,
                            true,
                            true,
                            renderer,
                            text_reveal_visual,
                        );
                    },
                )
            };
            let mut layer = layer;
            if let Some(reveal) =
                text_reveal_visual.filter(|reveal| !reveal.child_effects.is_empty())
            {
                apply_text_reveal_child_effects(
                    &mut layer,
                    reveal,
                    &child_layer_key,
                    namespace,
                    &format!("theme-layer-{index}"),
                    slide,
                    scene_layer,
                    text,
                    linked_text_context,
                    width,
                    height,
                    scroll_elapsed_seconds,
                    renderer,
                    cache,
                    &layers,
                );
            }
            if let Some(visual) = build_visual {
                apply_object_build_visual(&mut layer, visual, &layers, width, height);
            }
            layers.push(layer);
        }
    }
    if !rendered_theme_text && !theme_slide.is_some_and(suppresses_text_fallback) {
        let key_source = json!({ "text": text });
        layers.push(render_native_layer(
            namespace,
            "fallback-text".to_string(),
            key_source,
            false,
            width,
            height,
            scroll_elapsed_seconds,
            cache,
            |target| draw_fallback_text(&mut target.pixels, width, height, text, renderer),
        ));
    }

    if let Some(slide) = content
        .get("ccliSlide")
        .and_then(|value| parse_scene_slide(value).ok())
    {
        push_scene_layers(
            &mut layers,
            "ccli",
            namespace,
            &slide,
            "",
            linked_text_context,
            false,
            width,
            height,
            scroll_elapsed_seconds,
            media_download_pending,
            renderer,
            cache,
        );
    }

    let look_layers = content.get("lookLayers").and_then(Value::as_object);
    if let Some(overlay) = content
        .get("announcementOverlay")
        .filter(|value| value.is_object())
    {
        let mut announcement = compose_layered_frame(
            &format!("{namespace}:announcement"),
            overlay,
            width,
            height,
            scroll_elapsed_seconds,
            media_download_pending,
            renderer,
            background_image_cache,
            cache,
            None,
            None,
        );
        announcement.retain(|layer| layer.id != "background-color");
        for layer in &mut announcement {
            layer.id = format!("announcement-{}", layer.id);
            layer.replace = false;
        }
        layers.extend(announcement);
    }
    if look_layers
        .and_then(|value| value.get("props"))
        .and_then(Value::as_bool)
        != Some(false)
    {
        if let Some(prop_slides) = content.get("propSlides").and_then(Value::as_array) {
            for (slide_index, slide) in prop_slides
                .iter()
                .filter_map(|value| parse_scene_slide(value).ok())
                .enumerate()
            {
                push_scene_layers(
                    &mut layers,
                    &format!("prop-{slide_index}"),
                    namespace,
                    &slide,
                    "",
                    linked_text_context,
                    false,
                    width,
                    height,
                    scroll_elapsed_seconds,
                    media_download_pending,
                    renderer,
                    cache,
                );
            }
        }
    }
    if let Some(stage) = content
        .get("stageSlide")
        .and_then(|value| parse_scene_slide(value).ok())
    {
        push_scene_layers(
            &mut layers,
            "stage",
            namespace,
            &stage,
            "",
            linked_text_context,
            false,
            width,
            height,
            scroll_elapsed_seconds,
            media_download_pending,
            renderer,
            cache,
        );
    }
    if let Some(mask) = content
        .get("maskSlide")
        .and_then(|value| parse_scene_slide(value).ok())
    {
        push_scene_layers(
            &mut layers,
            "mask",
            namespace,
            &mask,
            "",
            &Value::Null,
            false,
            width,
            height,
            scroll_elapsed_seconds,
            media_download_pending,
            renderer,
            cache,
        );
    }

    let message = content
        .get("messageText")
        .and_then(Value::as_str)
        .or_else(|| {
            content
                .get("stageMessage")
                .and_then(Value::as_object)
                .and_then(|value| value.get("text"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    if !message.trim().is_empty() && (message != text || content.get("messageSlide").is_some()) {
        if let Some(slide) = content
            .get("messageSlide")
            .and_then(|value| parse_scene_slide(value).ok())
        {
            push_scene_layers(
                &mut layers,
                "message",
                namespace,
                &slide,
                message,
                linked_text_context,
                true,
                width,
                height,
                scroll_elapsed_seconds,
                media_download_pending,
                renderer,
                cache,
            );
        } else {
            layers.push(render_native_layer(
                namespace,
                "message".to_string(),
                Value::String(message.to_string()),
                false,
                width,
                height,
                scroll_elapsed_seconds,
                cache,
                |target| draw_fallback_text(&mut target.pixels, width, height, message, renderer),
            ));
        }
    }
    layers
}

#[allow(clippy::too_many_arguments)]
fn push_background_media_layer(
    layers: &mut Vec<NativeLayerFrame>,
    id: &str,
    namespace: &str,
    content: &Value,
    width: usize,
    height: usize,
    scroll_elapsed_seconds: f64,
    media_download_pending: bool,
    renderer: &mut NativeRenderRuntime,
    background_image_cache: &mut VecDeque<CachedBackgroundImage>,
    cache: &mut NativeLayerCache,
) {
    if !content
        .get("backgroundMediaType")
        .and_then(Value::as_str)
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("image") || value.eq_ignore_ascii_case("video")
        })
    {
        return;
    }
    let key_source = selected_content(
        content,
        &[
            "backgroundMediaType",
            "backgroundSelectionId",
            "backgroundMediaPath",
            "backgroundMediaUrl",
            "backgroundPlaybackState",
            "backgroundScale",
            "backgroundDesaturate",
        ],
    );
    let dynamic = (!renderer.static_images && content_requires_continuous_render(&key_source))
        || (media_download_pending && contains_media_source(&key_source));
    layers.push(render_native_layer(
        namespace,
        id.to_string(),
        key_source,
        dynamic,
        width,
        height,
        scroll_elapsed_seconds,
        cache,
        |target| paint_background_media(target, content, renderer, background_image_cache),
    ));
}

#[allow(clippy::too_many_arguments)]
fn push_scene_layers(
    layers: &mut Vec<NativeLayerFrame>,
    id_prefix: &str,
    namespace: &str,
    slide: &SceneSlide,
    fallback: &str,
    linked_text_context: &Value,
    use_presentation_text: bool,
    width: usize,
    height: usize,
    scroll_elapsed_seconds: f64,
    media_download_pending: bool,
    renderer: &mut NativeRenderRuntime,
    cache: &mut NativeLayerCache,
) {
    for (index, scene_layer) in slide.layers.iter().enumerate() {
        let key_source = scene_layer_key_source(
            slide,
            scene_layer,
            fallback,
            linked_text_context,
            use_presentation_text,
        );
        let dynamic = (!renderer.static_images && content_requires_continuous_render(&key_source))
            || (media_download_pending && contains_media_source(&key_source));
        let id = format!("{id_prefix}-layer-{index}");
        let draw = |target: &mut RenderTarget<'_>| {
            draw_scene_layer(
                target,
                slide,
                scene_layer,
                fallback,
                linked_text_context,
                use_presentation_text,
                true,
                renderer,
                None,
            );
        };
        let layer = if scene_layer_uses_backdrop(slide, scene_layer) {
            render_backdrop_dependent_layer(
                layers,
                namespace,
                id,
                key_source,
                dynamic,
                width,
                height,
                scroll_elapsed_seconds,
                cache,
                draw,
            )
        } else {
            render_native_layer(
                namespace,
                id,
                key_source,
                dynamic,
                width,
                height,
                scroll_elapsed_seconds,
                cache,
                draw,
            )
        };
        layers.push(layer);
    }
}

fn compose_frame(
    output: &mut RenderTarget,
    content: &Value,
    renderer: &mut NativeRenderRuntime,
    background_image_cache: &mut VecDeque<CachedBackgroundImage>,
) {
    output.dynamic_media = content_has_capture_fill(content)
        || content_has_video(content)
        || content_has_dynamic_linked_text(content)
        || content_has_scrolling_text(content, output.scroll_elapsed_seconds);
    let background = content
        .get("backgroundCss")
        .filter(|value| value.as_str().is_some_and(|value| !value.trim().is_empty()))
        .or_else(|| content.get("background"))
        .and_then(background_css)
        .unwrap_or_else(|| "#000000".to_string());
    paint_css_background(&mut output.pixels, output.width, output.height, &background);

    if let Some(slide) = content
        .get("videoInputSlide")
        .and_then(|value| parse_scene_slide(value).ok())
    {
        draw_scene_slide(output, &slide, "", &Value::Null, false, renderer);
    }

    let media_is_foreground = content
        .get("backgroundBehavior")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("foreground"));
    if !media_is_foreground {
        paint_background_media(output, content, renderer, background_image_cache);
    }

    let text = content
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| content.get("title").and_then(Value::as_str))
        .unwrap_or_default();
    let linked_text_context = content.get("linkedTextContext").unwrap_or(&Value::Null);

    let theme_slide = content
        .get("presentationThemeSlide")
        .and_then(|value| parse_scene_slide(value).ok());

    // A theme slide's background sits above a background media layer in the
    // renderer's layer order. Keep that ordering for native NDI frames too.
    if let Some(slide) = theme_slide.as_ref() {
        if slide
            .source
            .get("backgroundEnabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            if let Some(background) = slide.source.get("background").and_then(background_css) {
                paint_css_background(&mut output.pixels, output.width, output.height, &background);
            }
        }
    }

    // Foreground background media sits above the theme background but below
    // every normalized theme layer, matching ProgramThemeSlide's layer order.
    if media_is_foreground {
        paint_background_media(output, content, renderer, background_image_cache);
    }

    let rendered_theme_text = theme_slide
        .as_ref()
        .map(|slide| draw_scene_slide(output, slide, text, linked_text_context, true, renderer))
        .unwrap_or(false);
    if !rendered_theme_text && !theme_slide.as_ref().is_some_and(suppresses_text_fallback) {
        draw_fallback_text(
            &mut output.pixels,
            output.width,
            output.height,
            text,
            renderer,
        );
    }

    if let Some(slide) = content
        .get("ccliSlide")
        .and_then(|value| parse_scene_slide(value).ok())
    {
        draw_scene_slide(output, &slide, "", linked_text_context, false, renderer);
    }

    let look_layers = content.get("lookLayers").and_then(Value::as_object);
    if let Some(overlay) = content
        .get("announcementOverlay")
        .filter(|value| value.is_object())
    {
        let mut pixels = vec![0; output.pixels.len()];
        let mut target = RenderTarget {
            pixels: &mut pixels,
            width: output.width,
            height: output.height,
            dynamic_media: false,
            scroll_elapsed_seconds: output.scroll_elapsed_seconds,
        };
        compose_frame(&mut target, overlay, renderer, background_image_cache);
        output.dynamic_media |= target.dynamic_media;
        for (index, &source) in pixels.as_chunks::<4>().0.iter().enumerate() {
            if source[3] != 0 {
                blend_pixel(
                    output.pixels,
                    output.width,
                    index % output.width,
                    index / output.width,
                    source,
                    1.0,
                );
            }
        }
    }
    if look_layers
        .and_then(|layers| layers.get("props"))
        .and_then(Value::as_bool)
        != Some(false)
    {
        if let Some(prop_slides) = content.get("propSlides").and_then(Value::as_array) {
            for slide in prop_slides
                .iter()
                .filter_map(|value| parse_scene_slide(value).ok())
            {
                draw_scene_slide(output, &slide, "", linked_text_context, false, renderer);
            }
        }
    }

    if let Some(stage) = content
        .get("stageSlide")
        .and_then(|value| parse_scene_slide(value).ok())
    {
        draw_scene_slide(output, &stage, "", linked_text_context, false, renderer);
    }

    if let Some(mask) = content
        .get("maskSlide")
        .and_then(|value| parse_scene_slide(value).ok())
    {
        draw_scene_slide(output, &mask, "", &Value::Null, false, renderer);
    }

    let message = content
        .get("messageText")
        .and_then(Value::as_str)
        .or_else(|| {
            content
                .get("stageMessage")
                .and_then(Value::as_object)
                .and_then(|value| value.get("text"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    if !message.trim().is_empty() && (message != text || content.get("messageSlide").is_some()) {
        if let Some(slide) = content
            .get("messageSlide")
            .and_then(|value| parse_scene_slide(value).ok())
        {
            draw_scene_slide(output, &slide, message, linked_text_context, true, renderer);
        } else {
            draw_fallback_text(
                &mut output.pixels,
                output.width,
                output.height,
                message,
                renderer,
            );
        }
    }
}

fn content_has_video(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            let direct_video = ["backgroundMediaType", "mediaType", "mediaFillType", "type"]
                .into_iter()
                .filter_map(|key| object.get(key))
                .filter_map(Value::as_str)
                .any(|value| value.eq_ignore_ascii_case("video"));
            let text_media_video = object
                .get("mediaFillSourcePath")
                .or_else(|| object.get("mediaFillUrl"))
                .and_then(Value::as_str)
                .is_some_and(is_video_media_path);
            direct_video
                || matches!(
                    object.get("fillType").and_then(Value::as_str),
                    Some("ndi" | "web")
                )
                || text_media_video
                || object.values().any(content_has_video)
        }
        Value::Array(values) => values.iter().any(content_has_video),
        _ => false,
    }
}

fn contains_media_source(value: &Value) -> bool {
    const SOURCE_KEYS: &[&str] = &[
        "backgroundMediaPath",
        "backgroundMediaUrl",
        "sourcePath",
        "mediaSourcePath",
        "mediaPath",
        "mediaUrl",
        "sourceUrl",
        "mediaFillSourcePath",
        "mediaFillUrl",
    ];
    match value {
        Value::Object(object) => {
            object.iter().any(|(key, value)| {
                SOURCE_KEYS.contains(&key.as_str())
                    && value
                        .as_str()
                        .is_some_and(|source| !source.trim().is_empty())
            }) || object.values().any(contains_media_source)
        }
        Value::Array(values) => values.iter().any(contains_media_source),
        _ => false,
    }
}

pub fn content_requires_continuous_render(value: &Value) -> bool {
    if value
        .get("testPattern")
        .is_some_and(|pattern| !pattern.is_null())
    {
        return false;
    }
    content_has_capture_fill(value)
        || content_has_video(value)
        || content_has_dynamic_linked_text(value)
        || content_has_scrolling_text(value, 0.0)
}

/// True when offline prerendering could bypass a live object effect or delay.
pub(crate) fn object_builds_require_live_composition(value: &Value) -> bool {
    if !value
        .get("objectBuildEntryKey")
        .and_then(Value::as_str)
        .is_some_and(|key| !key.trim().is_empty())
    {
        return false;
    }
    let Some(slide) = value.get("presentationThemeSlide") else {
        return false;
    };
    if slide.get("ignoreBuilds").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    slide
        .get("layers")
        .and_then(Value::as_array)
        .is_some_and(|layers| {
            layers.iter().any(|layer| {
                if layer.get("hidden").and_then(Value::as_bool) == Some(true) {
                    return false;
                }
                ["build", "buildOut"].iter().any(|field| {
                    let Some(build) = layer.get(*field).and_then(Value::as_object) else {
                        return false;
                    };
                    let Some(step_value) = build.get("step") else {
                        return false;
                    };
                    let Some(step) = step_value.as_f64() else {
                        return true;
                    };
                    if !step.is_finite()
                        || step < 0.0
                        || step.fract() != 0.0
                        || step > 9_007_199_254_740_991.0
                    {
                        return true;
                    }
                    let Some(transition) = build.get("transition") else {
                        return has_pending_build_delay(build);
                    };
                    let needs_native_composition = match native_transitions::resolve(transition) {
                        Ok(Some(spec)) => {
                            spec.kind == native_transitions::TransitionKind::Dissolve
                                && spec.duration_seconds > 0.0
                        }
                        Ok(None) => false,
                        Err(_) => true,
                    };
                    needs_native_composition || has_pending_build_delay(build)
                }) || text_reveal_children_require_live_composition(layer)
            })
        })
}

fn text_reveal_children_require_live_composition(layer: &Value) -> bool {
    let Some(reveal_value) = layer.get("textReveal") else {
        return false;
    };
    if reveal_value.is_null() {
        return false;
    }
    let Some(reveal) = reveal_value.as_object() else {
        return true;
    };
    if !matches!(
        reveal.get("mode").and_then(Value::as_str),
        Some("paragraph" | "underline")
    ) {
        return true;
    }
    let Some(children_value) = reveal.get("children") else {
        return false;
    };
    let Some(children) = children_value.as_array() else {
        return true;
    };
    let inherited_transition = layer
        .get("build")
        .and_then(Value::as_object)
        .and_then(|build| build.get("transition"));
    let inherited_dissolve = match inherited_transition {
        Some(transition) => match native_transitions::resolve(transition) {
            Ok(Some(spec)) => {
                spec.kind == native_transitions::TransitionKind::Dissolve
                    && spec.duration_seconds > 0.0
            }
            Ok(None) => false,
            Err(_) => true,
        },
        None => false,
    };
    children.iter().any(|child| {
        let Some(child) = child.as_object() else {
            return true;
        };
        if child.contains_key("transition") {
            return true;
        }
        if !child
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.trim().is_empty())
            || !child
                .get("index")
                .and_then(Value::as_u64)
                .is_some_and(|index| index <= 9_007_199_254_740_991)
            || child.get("start").is_some_and(|start| {
                !matches!(
                    start.as_str(),
                    Some("on-click" | "with-previous" | "after-previous" | "with-slide")
                )
            })
            || child.get("delaySeconds").is_some_and(|delay| {
                !delay
                    .as_f64()
                    .is_some_and(|delay| delay.is_finite() && delay >= 0.0)
            })
        {
            return true;
        }
        let Some(step) = child.get("step") else {
            return false;
        };
        if !step.as_f64().is_some_and(|step| {
            step.is_finite()
                && step >= 0.0
                && step.fract() == 0.0
                && step <= 9_007_199_254_740_991.0
        }) {
            return true;
        }
        inherited_dissolve || has_pending_build_delay(child)
    })
}

fn has_pending_build_delay(build: &Map<String, Value>) -> bool {
    build
        .get("delaySeconds")
        .and_then(Value::as_f64)
        .is_some_and(|delay| {
            delay.is_finite()
                && delay > 0.0
                && build.get("start").and_then(Value::as_str) != Some("after-previous")
        })
}

fn content_has_capture_fill(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.get("fillType").and_then(Value::as_str) == Some("capture-status-color")
                || object.values().any(content_has_capture_fill)
        }
        Value::Array(values) => values.iter().any(content_has_capture_fill),
        _ => false,
    }
}

fn content_has_scrolling_text(value: &Value, elapsed_seconds: f64) -> bool {
    match value {
        Value::Object(object) => {
            let scrolling = object.get("scrollingEnabled").and_then(Value::as_bool) == Some(true);
            let style = scrolling.then(|| object);
            style.is_some_and(|style| scrolling_is_active(style, elapsed_seconds))
                || object
                    .values()
                    .any(|value| content_has_scrolling_text(value, elapsed_seconds))
        }
        Value::Array(values) => values
            .iter()
            .any(|value| content_has_scrolling_text(value, elapsed_seconds)),
        _ => false,
    }
}

pub(crate) fn is_video_media_path(value: &str) -> bool {
    let path = value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase();
    [".mp4", ".mov", ".m4v", ".webm", ".mkv", ".avi"]
        .iter()
        .any(|extension| path.ends_with(extension))
}

fn paint_background_media(
    output: &mut RenderTarget,
    content: &Value,
    renderer: &mut NativeRenderRuntime,
    background_image_cache: &mut VecDeque<CachedBackgroundImage>,
) {
    let media_type = content
        .get("backgroundMediaType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !media_type.eq_ignore_ascii_case("image") && !media_type.eq_ignore_ascii_case("video") {
        return;
    }
    let source = content
        .get("backgroundMediaPath")
        .or_else(|| content.get("backgroundMediaUrl"));
    let Some(path) = source.and_then(local_media_path) else {
        output.dynamic_media |= source
            .and_then(Value::as_str)
            .is_some_and(|source| !source.trim().is_empty());
        return;
    };
    let unadjusted_fit = content["backgroundScale"].as_str().unwrap_or("fit") == "fit"
        && content
            .get("backgroundEffects")
            .is_none_or(|effects| effects.as_array().is_some_and(Vec::is_empty))
        && content["backgroundDesaturate"] != true
        && content["backgroundBrightness"].as_f64().unwrap_or(0.) == 0.
        && content["backgroundContrast"].as_f64().unwrap_or(1.) == 1.
        && content["backgroundHue"].as_f64().unwrap_or(0.) == 0.
        && content["backgroundSaturation"].as_f64().unwrap_or(1.) == 1.;
    let (image, cacheable) = if media_type.eq_ignore_ascii_case("video") {
        (
            renderer
                .video_frame_with_clock(
                    &path,
                    content
                        .get("backgroundPlaybackState")
                        .and_then(Value::as_object),
                    content["backgroundMediaHardwareDecoding"] == "disabled",
                )
                .ok()
                .flatten()
                .cloned()
                .map(Arc::new),
            false,
        )
    } else {
        let frame = if unadjusted_fit {
            renderer.image_frame_for_size(&path, output.width, output.height)
        } else {
            renderer.image_frame(&path)
        };
        match frame.ok() {
            Some(frame) => {
                output.dynamic_media |= frame.dynamic;
                (Some(frame.image), !frame.dynamic)
            }
            None => (None, false),
        }
    };
    let Some(image) = image else {
        // The renderer is intentionally tolerant of media that is still
        // loading or has disappeared. Preserve the CSS background while the
        // next output update gives the path another chance.
        output.dynamic_media = true;
        return;
    };
    // The output scene validated the chain; an invalid one leaves the CSS background.
    let Ok(effects) = content
        .get("backgroundEffects")
        .map_or(Ok(Vec::new()), crate::native_media_effects::resolve_chain)
    else {
        return;
    };
    let animated = effects.iter().any(|effect| effect.animates());
    output.dynamic_media |= animated;
    let cacheable = cacheable && !animated;
    let scale = content
        .get("backgroundScale")
        .and_then(Value::as_str)
        .unwrap_or("fit");
    paint_image_background(
        &mut output.pixels,
        output.width,
        output.height,
        &image,
        scale,
        // This work happens synchronously on the output render path. Triangle
        // keeps a 1080p still responsive when it has to be scaled for a larger
        // physical output; Lanczos can visibly stall the transition.
        image::imageops::FilterType::Triangle,
        cacheable.then_some(background_image_cache),
        &effects,
        content.get("backgroundDesaturate").and_then(Value::as_bool) == Some(true),
        content
            .get("backgroundBrightness")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        content
            .get("backgroundContrast")
            .and_then(Value::as_f64)
            .unwrap_or(1.0),
        content
            .get("backgroundHue")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        content
            .get("backgroundSaturation")
            .and_then(Value::as_f64)
            .unwrap_or(1.0),
    );
}

fn local_media_path(value: &Value) -> Option<PathBuf> {
    resolve_media_path(value)
}

fn paint_image_background(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    image: &Arc<DynamicImage>,
    scale: &str,
    filter: image::imageops::FilterType,
    cache: Option<&mut VecDeque<CachedBackgroundImage>>,
    effects: &[crate::native_media_effects::ResolvedEffect],
    desaturate: bool,
    brightness: f64,
    contrast: f64,
    hue: f64,
    saturation: f64,
) {
    let normalized_scale = scale.to_ascii_lowercase();
    if let Some(cache) = cache {
        let cache_index = cache.iter().position(|cached| {
            Arc::ptr_eq(&cached.source_image, image)
                && cached.width == width
                && cached.height == height
                && cached.scale == normalized_scale
                && cached.effects == effects
                && cached.desaturate == desaturate
                && cached.brightness == brightness
                && cached.contrast == contrast
                && cached.hue == hue
                && cached.saturation == saturation
        });
        if let Some(index) = cache_index {
            let cached = cache.remove(index).expect("cached background exists");
            paint_background_layers(pixels, width, height, &cached.layers);
            cache.push_back(cached);
            return;
        }
        let cached = CachedBackgroundImage {
            effects: effects.to_vec(),
            desaturate,
            brightness,
            contrast,
            hue,
            saturation,
            source_image: Arc::clone(image),
            width,
            height,
            scale: normalized_scale,
            layers: render_background_layers(
                width,
                height,
                image.as_ref(),
                scale,
                filter,
                effects,
                desaturate,
                brightness,
                contrast,
                hue,
                saturation,
            ),
        };
        paint_background_layers(pixels, width, height, &cached.layers);
        let bytes = cached.bytes();
        const CACHE_BYTES: usize = 64 * 1024 * 1024;
        if bytes <= CACHE_BYTES {
            while cache.len() >= 8
                || cache
                    .iter()
                    .map(CachedBackgroundImage::bytes)
                    .sum::<usize>()
                    + bytes
                    > CACHE_BYTES
            {
                cache.pop_front();
            }
            cache.push_back(cached);
        }
        return;
    }

    let layers = render_background_layers(
        width,
        height,
        image.as_ref(),
        scale,
        filter,
        effects,
        desaturate,
        brightness,
        contrast,
        hue,
        saturation,
    );
    paint_background_layers(pixels, width, height, &layers);
}

fn media_hue_matrix(degrees: f64) -> [[f64; 3]; 3] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [
        [
            0.213 + cos * 0.787 - sin * 0.213,
            0.715 - cos * 0.715 - sin * 0.715,
            0.072 - cos * 0.072 + sin * 0.928,
        ],
        [
            0.213 - cos * 0.213 + sin * 0.143,
            0.715 + cos * 0.285 + sin * 0.140,
            0.072 - cos * 0.072 - sin * 0.283,
        ],
        [
            0.213 - cos * 0.213 - sin * 0.787,
            0.715 - cos * 0.715 + sin * 0.715,
            0.072 + cos * 0.928 + sin * 0.072,
        ],
    ]
}

fn adjusted_media_image(
    image: &DynamicImage,
    desaturate: bool,
    brightness: f64,
    contrast: f64,
    hue: f64,
    saturation: f64,
) -> Option<DynamicImage> {
    (desaturate || hue != 0.0 || saturation != 1.0 || brightness != 0.0 || contrast != 1.0).then(
        || {
            let mut pixels = if desaturate {
                image.grayscale().to_rgba8()
            } else {
                image.to_rgba8()
            };
            let hue_matrix = media_hue_matrix(hue);
            for pixel in pixels.pixels_mut() {
                if hue != 0.0 || saturation != 1.0 {
                    let source = [pixel[0] as f64, pixel[1] as f64, pixel[2] as f64];
                    let rotated = hue_matrix.map(|row| {
                        row.iter()
                            .zip(source)
                            .map(|(factor, channel)| factor * channel)
                            .sum::<f64>()
                            .clamp(0.0, 255.0)
                    });
                    let luminance = rotated[0] * 0.213 + rotated[1] * 0.715 + rotated[2] * 0.072;
                    for index in 0..3 {
                        pixel[index] = (luminance + saturation * (rotated[index] - luminance))
                            .round()
                            .clamp(0.0, 255.0) as u8;
                    }
                }
                for channel in &mut pixel.0[..3] {
                    let bright = (*channel as f64 + brightness * 255.0).clamp(0.0, 255.0);
                    *channel = ((bright - 127.5) * contrast + 127.5)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
            DynamicImage::ImageRgba8(pixels)
        },
    )
}

/// Applies an object's color adjustments and effect chain (`{prefix}Effects`).
/// Returns the image and whether the chain animates; `None` when the chain is
/// invalid (content is validated with the workspace, so this is not expected).
fn adjusted_object_media_image(
    image: &Arc<DynamicImage>,
    fields: &serde_json::Map<String, Value>,
    prefix: &str,
) -> Option<(Arc<DynamicImage>, bool)> {
    let number = |suffix: &str, neutral: f64| {
        json_number(fields.get(&format!("{prefix}{suffix}"))).unwrap_or(neutral)
    };
    let effects = match fields.get(&format!("{prefix}Effects")) {
        Some(chain) => crate::native_media_effects::resolve_chain(chain).ok()?,
        None => Vec::new(),
    };
    let adjusted = adjusted_media_image(
        image,
        false,
        number("Brightness", 0.0),
        number("Contrast", 1.0),
        number("Hue", 0.0),
        number("Saturation", 1.0),
    )
    .map(Arc::new)
    .unwrap_or_else(|| image.clone());
    if effects.is_empty() {
        return Some((adjusted, false));
    }
    let animated = effects.iter().any(|effect| effect.animates());
    let affected = crate::native_media_effects::apply_chain(
        &adjusted.to_rgba8(),
        &effects,
        native_output_elapsed_seconds(),
    );
    Some((Arc::new(DynamicImage::ImageRgba8(affected)), animated))
}

fn render_background_layers(
    width: usize,
    height: usize,
    image: &DynamicImage,
    scale: &str,
    filter: image::imageops::FilterType,
    effects: &[crate::native_media_effects::ResolvedEffect],
    desaturate: bool,
    brightness: f64,
    contrast: f64,
    hue: f64,
    saturation: f64,
) -> Vec<RenderedBackgroundLayer> {
    let adjusted = adjusted_media_image(image, desaturate, brightness, contrast, hue, saturation);
    let image = adjusted.as_ref().unwrap_or(image);
    // Effects run on the source resolution, after the color adjustments.
    let affected = (!effects.is_empty()).then(|| {
        DynamicImage::ImageRgba8(crate::native_media_effects::apply_chain(
            &image.to_rgba8(),
            effects,
            native_output_elapsed_seconds(),
        ))
    });
    let image = affected.as_ref().unwrap_or(image);
    let width_u32 = width as u32;
    let height_u32 = height as u32;
    if scale.eq_ignore_ascii_case("blur") {
        let blurred = image
            .resize_to_fill(width_u32, height_u32, filter)
            .blur(22.0)
            .to_rgba8();
        let fitted = image.resize(width_u32, height_u32, filter).to_rgba8();
        let left = width.saturating_sub(fitted.width() as usize) / 2;
        let top = height.saturating_sub(fitted.height() as usize) / 2;
        return vec![
            rendered_background_layer(blurred, 0, 0),
            rendered_background_layer(fitted, left as isize, top as isize),
        ];
    }

    let rendered = if image.width() == width_u32 && image.height() == height_u32 {
        image.to_rgba8()
    } else if scale.eq_ignore_ascii_case("stretch") {
        image.resize_exact(width_u32, height_u32, filter).to_rgba8()
    } else if scale.eq_ignore_ascii_case("fill") {
        image
            .resize_to_fill(width_u32, height_u32, filter)
            .to_rgba8()
    } else if image.width() >= width_u32 && image.height() >= height_u32 {
        image.thumbnail(width_u32, height_u32).to_rgba8()
    } else {
        image.resize(width_u32, height_u32, filter).to_rgba8()
    };
    let left = width.saturating_sub(rendered.width() as usize) / 2;
    let top = height.saturating_sub(rendered.height() as usize) / 2;
    vec![rendered_background_layer(
        rendered,
        left as isize,
        top as isize,
    )]
}

fn rendered_background_layer(
    image: image::RgbaImage,
    left: isize,
    top: isize,
) -> RenderedBackgroundLayer {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let opaque = image.pixels().all(|pixel| pixel[3] == 255);
    let mut pixels = image.into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    RenderedBackgroundLayer {
        pixels,
        width,
        height,
        opaque,
        left,
        top,
    }
}

fn paint_background_layers(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    layers: &[RenderedBackgroundLayer],
) {
    for layer in layers {
        draw_background_layer(pixels, width, height, layer);
    }
}

fn draw_background_layer(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    layer: &RenderedBackgroundLayer,
) {
    let source_left = (-layer.left).max(0) as usize;
    let source_right = layer
        .width
        .min((width as isize - layer.left).max(0) as usize);
    if source_left >= source_right {
        return;
    }
    for source_y in 0..layer.height {
        let target_y = layer.top + source_y as isize;
        if !(0..height as isize).contains(&target_y) {
            continue;
        }
        let source_start = (source_y * layer.width + source_left) * 4;
        let source_end = (source_y * layer.width + source_right) * 4;
        let target_x = layer.left + source_left as isize;
        let target_start = (target_y as usize * width + target_x as usize) * 4;
        let target_end = target_start + source_end - source_start;
        if layer.opaque {
            pixels[target_start..target_end]
                .copy_from_slice(&layer.pixels[source_start..source_end]);
            continue;
        }
        for (offset, source) in layer.pixels[source_start..source_end]
            .chunks_exact(4)
            .enumerate()
        {
            blend_pixel(
                pixels,
                width,
                target_x as usize + offset,
                target_y as usize,
                [source[0], source[1], source[2], source[3]],
                1.0,
            );
        }
    }
}

fn background_css(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.trim().is_empty() => Some(value.clone()),
        Value::Object(value) => value
            .get("css")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                value
                    .get("color")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }),
        _ => None,
    }
}

/// An opaque matte to display while the canvas bitmap is being rendered.
pub fn opaque_canvas_matte(slide: &Value, kind: CanvasRenderKind) -> Option<u32> {
    if slide["backgroundEnabled"] == false || matches!(kind, CanvasRenderKind::Mask) {
        return None;
    }
    let css = match background_css(&slide["background"]) {
        Some(css) => css,
        None if matches!(kind, CanvasRenderKind::Stage) => return Some(0),
        None => return None,
    };
    let [blue, green, red, alpha] = if let Some((stops, _)) = parse_linear_gradient(&css) {
        if stops.iter().any(|(_, color)| color[3] != 255) {
            return None;
        }
        stops.first()?.1
    } else {
        // Keep the same default as paint_css_background for unsupported CSS.
        parse_color(&css).unwrap_or([0, 0, 0, 255])
    };
    (alpha == 255).then_some(u32::from_be_bytes([0, red, green, blue]))
}

fn suppresses_text_fallback(slide: &SceneSlide) -> bool {
    slide.source.contains_key("primaryTextLayerId")
        || slide
            .source
            .get("layers")
            .and_then(Value::as_array)
            .is_some_and(|layers| {
                layers.iter().filter_map(Value::as_object).any(|layer| {
                    crate::scene::is_text_layer(layer)
                        && ["build", "buildOut", "textReveal"]
                            .iter()
                            .any(|key| layer.get(*key).is_some_and(Value::is_object))
                })
            })
}

fn draw_scene_slide(
    output: &mut RenderTarget,
    slide: &SceneSlide,
    fallback: &str,
    linked_text_context: &Value,
    use_presentation_text: bool,
    renderer: &mut NativeRenderRuntime,
) -> bool {
    if native_object_links::validate(slide).is_err() {
        return false;
    }
    let mut rendered_text = false;
    for layer in &slide.layers {
        rendered_text |= draw_scene_layer(
            output,
            slide,
            layer,
            fallback,
            linked_text_context,
            use_presentation_text,
            true,
            renderer,
            None,
        );
    }
    rendered_text
}

#[path = "native_compositor/object_transform.rs"]
mod object_transform;

#[allow(clippy::too_many_arguments)]
fn draw_scene_layer(
    output: &mut RenderTarget,
    slide: &SceneSlide,
    layer: &SceneLayer,
    fallback: &str,
    linked_text_context: &Value,
    use_presentation_text: bool,
    paint_box: bool,
    renderer: &mut NativeRenderRuntime,
    text_reveal_visual: Option<&native_object_builds::TextRevealVisual>,
) -> bool {
    if let Some(link) = layer.source.get("visibilityLink") {
        output.dynamic_media = true;
        let element_text = |id: &str| {
            let target = slide
                .source
                .get("layers")?
                .as_array()?
                .iter()
                .find(|candidate| candidate.get("id").and_then(Value::as_str) == Some(id))?
                .as_object()?;
            let base = target.get("text").and_then(Value::as_str).unwrap_or("");
            Some(resolve_linked_text(
                base,
                &merged_text_style(&slide.source, target),
                linked_text_context,
            ))
        };
        if !native_linked_text::visibility_link_visible(link, linked_text_context, &element_text) {
            return layer.text_layer;
        }
    }
    if (layer.source.get("fillType").and_then(Value::as_str) == Some("slide-object")
        || merged_text_style(&slide.source, &layer.source)
            .get("linkedTextSource")
            .and_then(Value::as_str)
            == Some("Object"))
        && native_object_links::validate(slide).is_err()
    {
        return false;
    }
    let Ok(resolved) = native_object_links::resolve(
        slide,
        &layer.source,
        linked_text_context,
        fallback,
        use_presentation_text,
    ) else {
        return false;
    };
    let mut resolved_layer = layer.clone();
    resolved_layer.source = resolved;
    let stage_style = merged_text_style(&slide.source, &resolved_layer.source);
    if stage_style.get("linkedTextSource").and_then(Value::as_str) == Some("Stage Message")
        && stage_style
            .get("linkedTextEnabled")
            .and_then(Value::as_bool)
            != Some(false)
        && stage_style
            .get("linkedTextStageMessageFlash")
            .and_then(Value::as_bool)
            == Some(true)
        && !resolve_linked_text("", &stage_style, linked_text_context).is_empty()
        && chrono::Utc::now().timestamp_millis().rem_euclid(1000) < 500
    {
        if let Some(color) = stage_style
            .get("linkedTextStageMessageFlashColor")
            .and_then(Value::as_str)
        {
            resolved_layer.source.insert("fill".into(), json!(color));
            resolved_layer
                .source
                .insert("fillType".into(), json!("color"));
            resolved_layer
                .source
                .insert("fillEnabled".into(), json!(true));
        }
    }
    resolved_layer.text_layer |= matches!(
        stage_style.get("linkedTextSource").and_then(Value::as_str),
        Some("Object" | "Countdown Timer" | "Video Countdown" | "System Clock")
    ) && stage_style
        .get("linkedTextEnabled")
        .and_then(Value::as_bool)
        != Some(false);
    let layer = &resolved_layer;
    let mut draw = |target: &mut RenderTarget<'_>, slide: &SceneSlide, layer: &SceneLayer| {
        draw_scene_layer_content(
            target,
            slide,
            layer,
            fallback,
            linked_text_context,
            use_presentation_text,
            paint_box,
            renderer,
            text_reveal_visual,
        )
    };
    if let Some(transform) = object_transform::Transform::new(output, slide, layer) {
        transform.draw(output, slide, layer, draw)
    } else {
        draw(output, slide, layer)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_scene_layer_content(
    output: &mut RenderTarget,
    slide: &SceneSlide,
    layer: &SceneLayer,
    fallback: &str,
    linked_text_context: &Value,
    use_presentation_text: bool,
    paint_box: bool,
    renderer: &mut NativeRenderRuntime,
    text_reveal_visual: Option<&native_object_builds::TextRevealVisual>,
) -> bool {
    if layer.source.get("fillType").and_then(Value::as_str) == Some("chord-chart") {
        if paint_box {
            native_chart::draw(output, slide, layer, linked_text_context, renderer);
        }
        let mut outline = layer.clone();
        outline.source.insert("fillType".into(), json!("none"));
        return layer.text_layer
            && draw_text_layer(
                output,
                slide,
                &outline,
                fallback,
                linked_text_context,
                use_presentation_text,
                paint_box,
                renderer,
                text_reveal_visual,
            );
    }
    if native_stage_previews::is_preview(&layer.source) {
        let mut outline = layer.clone();
        outline.source.insert("fillType".into(), json!("none"));
        if paint_box {
            draw_shape_layer(output, slide, &outline, renderer);
            native_stage_previews::draw(output, slide, layer, linked_text_context, renderer);
        }
        return layer.text_layer
            && draw_text_layer(
                output,
                slide,
                &outline,
                fallback,
                linked_text_context,
                use_presentation_text,
                paint_box,
                renderer,
                text_reveal_visual,
            );
    }
    match layer.kind {
        SceneLayerKind::Shape => {
            let line_mask = layer.text_layer
                && merged_text_style(&slide.source, &layer.source)
                    .get("linesOnlyEnabled")
                    .and_then(Value::as_bool)
                    == Some(true);
            if paint_box && !line_mask {
                draw_shape_layer(output, slide, layer, renderer);
            }
            layer.text_layer
                && draw_text_layer(
                    output,
                    slide,
                    layer,
                    fallback,
                    linked_text_context,
                    use_presentation_text,
                    paint_box && line_mask,
                    renderer,
                    text_reveal_visual,
                )
        }
        SceneLayerKind::Text => draw_text_layer(
            output,
            slide,
            layer,
            fallback,
            linked_text_context,
            use_presentation_text,
            paint_box,
            renderer,
            text_reveal_visual,
        ),
        SceneLayerKind::Media => {
            if paint_box {
                let _ =
                    draw_slide_media(output, &layer.source, slide.width, slide.height, renderer);
            }
            false
        }
        SceneLayerKind::Other => false,
    }
}

fn draw_shape_layer(
    output: &mut RenderTarget,
    slide: &SceneSlide,
    layer: &SceneLayer,
    renderer: &mut NativeRenderRuntime,
) -> bool {
    let source = &layer.source;
    let bounds = source.get("bounds").and_then(Value::as_object);
    let x = bounds
        .and_then(|value| json_number(value.get("x")))
        .unwrap_or(0.0);
    let y = bounds
        .and_then(|value| json_number(value.get("y")))
        .unwrap_or(0.0);
    let width = bounds
        .and_then(|value| json_number(value.get("width")))
        .unwrap_or(slide.width);
    let height = bounds
        .and_then(|value| json_number(value.get("height")))
        .unwrap_or(slide.height);
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    let left = (x / slide.width * output.width as f64).round() as isize;
    let top = (y / slide.height * output.height as f64).round() as isize;
    let right = ((x + width) / slide.width * output.width as f64).round() as isize;
    let bottom = ((y + height) / slide.height * output.height as f64).round() as isize;
    let geometry = ShapeGeometry::from_layer(source, x, y, width, height);
    let feather_amount = shape_feather_amount(source);
    let fill = shape_fill_css(source);
    let opacity =
        (json_number(source.get("opacity")).unwrap_or(100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let mut rendered = false;
    if draw_shape_shadow(
        output,
        source,
        left,
        top,
        right,
        bottom,
        slide.height,
        opacity,
        &geometry,
        feather_amount,
        None,
    ) {
        rendered = true;
    }
    if geometry.is_closed()
        && paint_shape_background_effect(
            output,
            source,
            left,
            top,
            right,
            bottom,
            opacity,
            &geometry,
            feather_amount,
            output.height as f64 / slide.height,
        )
    {
        rendered = true;
    }
    if fill != "transparent" && geometry.is_closed() {
        paint_shape_css(
            &mut output.pixels,
            output.width,
            output.height,
            left,
            top,
            right,
            bottom,
            &fill,
            opacity,
            &geometry,
            feather_amount,
        );
        rendered = true;
    }

    // A shape can use an image/video as its fill. Preserve the shape fill
    // as the fallback while replacing only the media-capable part with a
    // decoded local raster image, matching the browser renderer's
    // prepend/media-fill layer order.
    if source
        .get("fillType")
        .and_then(Value::as_str)
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("media")
                || value.eq_ignore_ascii_case("ndi")
                || value.eq_ignore_ascii_case("web")
        })
        && draw_slide_media_in_bounds(
            output,
            source,
            left,
            top,
            right,
            bottom,
            renderer,
            Some(&geometry),
            output.width as f64 / slide.width,
            output.height as f64 / slide.height,
            feather_amount,
        )
    {
        rendered = true;
    }

    if draw_layer_border(
        output,
        source,
        left,
        top,
        right,
        bottom,
        output.height as f64 / slide.height,
        opacity,
        &geometry,
        feather_amount,
    ) {
        rendered = true;
    }
    rendered
}

#[allow(clippy::too_many_arguments)]
fn draw_shape_shadow(
    output: &mut RenderTarget,
    layer: &serde_json::Map<String, Value>,
    left: isize,
    top: isize,
    right: isize,
    bottom: isize,
    slide_height: f64,
    opacity: f32,
    geometry: &ShapeGeometry,
    feather_amount: ShapeFeather,
    line_mask: Option<&[(isize, isize, isize, isize)]>,
) -> bool {
    if layer.get("shadowEnabled").and_then(Value::as_bool) != Some(true)
        || !geometry.is_closed()
        || output.width == 0
        || output.height == 0
    {
        return false;
    }

    let target_width = (right - left).max(1) as f32;
    let target_height = (bottom - top).max(1) as f32;
    let scale = output.height as f64 / slide_height.max(1.0);
    let offset = json_number(layer.get("shadowOffset")).unwrap_or(5.0);
    let angle = normalize_degrees(json_number(layer.get("shadowAngle")).unwrap_or(315.0));
    let angle = angle.to_radians();
    // Imported files are not constrained by the editor's 0..80 controls. Keep
    // malformed effect values from allocating multi-gigabyte temporary masks.
    let max_effect_extent = output.width.max(output.height).clamp(1, 512) as f64;
    let offset_pixels = offset * scale;
    let offset_pixels = if offset_pixels.is_finite() {
        offset_pixels.clamp(-max_effect_extent, max_effect_extent)
    } else {
        0.0
    };
    let offset_x = (angle.cos() * offset_pixels).round() as isize;
    let offset_y = (-angle.sin() * offset_pixels).round() as isize;
    let blur_pixels = json_number(layer.get("shadowBlur")).unwrap_or(5.0) * scale;
    let blur_radius = if blur_pixels.is_finite() {
        blur_pixels.clamp(0.0, max_effect_extent).round() as usize
    } else {
        0
    };
    let feather_padding = feather_amount
        .outset((right - left) as f32, (bottom - top) as f32)
        .max(0) as usize;
    let padding_x = blur_radius
        .saturating_add(feather_padding)
        .saturating_add(offset_x.saturating_abs() as usize)
        .saturating_add(1);
    let padding_y = blur_radius
        .saturating_add(feather_padding)
        .saturating_add(offset_y.saturating_abs() as usize)
        .saturating_add(1);
    let shape_width = (right - left).max(1) as usize;
    let shape_height = (bottom - top).max(1) as usize;
    let mask_width = shape_width.saturating_add(padding_x.saturating_mul(2));
    let mask_height = shape_height.saturating_add(padding_y.saturating_mul(2));
    let Some(mask_len) = mask_width.checked_mul(mask_height) else {
        return false;
    };
    let mut mask = vec![0_u8; mask_len];
    let mask_left = left.saturating_sub(padding_x as isize);
    let mask_top = top.saturating_sub(padding_y as isize);
    for y in 0..mask_height {
        let global_y = mask_top.saturating_add(y as isize);
        let local_y = (global_y - top) as f32 / target_height;
        for x in 0..mask_width {
            let global_x = mask_left.saturating_add(x as isize);
            let local_x = (global_x - left) as f32 / target_width;
            let local_x = local_x + 0.5 / target_width;
            let local_y = local_y + 0.5 / target_height;
            let line_visible = line_mask.is_none_or(|rects| {
                rects.iter().any(|&(l, t, r, b)| {
                    global_x - left >= l
                        && global_x - left < r
                        && global_y - top >= t
                        && global_y - top < b
                })
            });
            if line_visible {
                mask[y * mask_width + x] = (shape_feather_alpha(
                    geometry,
                    local_x,
                    local_y,
                    target_width,
                    target_height,
                    feather_amount,
                ) * 255.0)
                    .round() as u8;
            }
        }
    }

    let blurred = blur_alpha_mask(&mask, mask_width, mask_height, blur_radius);
    let mut shadow_color = layer
        .get("shadowColor")
        .and_then(Value::as_str)
        .and_then(parse_color)
        .unwrap_or([0, 0, 0, 255]);
    shadow_color[3] = (shadow_color[3] as f32
        * (json_number(layer.get("shadowOpacity"))
            .unwrap_or(75.0)
            .clamp(0.0, 100.0) as f32
            / 100.0))
        .round() as u8;
    let mut rendered = false;
    for y in 0..mask_height {
        for x in 0..mask_width {
            let coverage = blurred[y * mask_width + x];
            if coverage == 0 {
                continue;
            }
            let target_x = mask_left
                .saturating_add(x as isize)
                .saturating_add(offset_x);
            let target_y = mask_top.saturating_add(y as isize).saturating_add(offset_y);
            if target_x < 0
                || target_y < 0
                || target_x >= output.width as isize
                || target_y >= output.height as isize
            {
                continue;
            }
            let mut covered_color = shadow_color;
            covered_color[3] = ((covered_color[3] as u16 * coverage as u16 + 127) / 255) as u8;
            blend_pixel(
                &mut output.pixels,
                output.width,
                target_x as usize,
                target_y as usize,
                covered_color,
                opacity,
            );
            rendered = true;
        }
    }
    rendered
}

fn draw_slide_media(
    output: &mut RenderTarget,
    layer: &serde_json::Map<String, Value>,
    slide_width: f64,
    slide_height: f64,
    renderer: &mut NativeRenderRuntime,
) -> bool {
    let bounds = layer.get("bounds").and_then(Value::as_object);
    let x = bounds
        .and_then(|value| json_number(value.get("x")))
        .unwrap_or(0.0);
    let y = bounds
        .and_then(|value| json_number(value.get("y")))
        .unwrap_or(0.0);
    let width = bounds
        .and_then(|value| json_number(value.get("width")))
        .unwrap_or(slide_width);
    let height = bounds
        .and_then(|value| json_number(value.get("height")))
        .unwrap_or(slide_height);
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    let left = (x / slide_width * output.width as f64).round() as isize;
    let top = (y / slide_height * output.height as f64).round() as isize;
    let right = ((x + width) / slide_width * output.width as f64).round() as isize;
    let bottom = ((y + height) / slide_height * output.height as f64).round() as isize;
    draw_slide_media_in_bounds(
        output,
        layer,
        left,
        top,
        right,
        bottom,
        renderer,
        None,
        output.width as f64 / slide_width,
        output.height as f64 / slide_height,
        ShapeFeather::NONE,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_slide_media_in_bounds(
    output: &mut RenderTarget,
    layer: &serde_json::Map<String, Value>,
    left: isize,
    top: isize,
    right: isize,
    bottom: isize,
    renderer: &mut NativeRenderRuntime,
    shape: Option<&ShapeGeometry>,
    crop_scale_x: f64,
    crop_scale_y: f64,
    feather_amount: ShapeFeather,
) -> bool {
    let ndi = layer.get("fillType").and_then(Value::as_str) == Some("ndi");
    let web = layer.get("fillType").and_then(Value::as_str) == Some("web");
    let is_video = layer
        .get("mediaType")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("video"))
        || layer
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("video"));
    let mut playback_opacity = 1.0;
    let image = if ndi {
        output.dynamic_media = true;
        // Video inputs reuse the live-input fill; the layer carries either an NDI or a camera source.
        if let Some(source) = layer.get("cameraSource") {
            serde_json::from_value(source.clone())
                .ok()
                .and_then(|source| crate::camera_input::frame(&source).ok().flatten())
        } else {
            layer
                .get("ndiSource")
                .and_then(|source| serde_json::from_value(source.clone()).ok())
                .and_then(|source| crate::ndi_input::frame(&source).ok().flatten())
        }
    } else if web {
        output.dynamic_media = true;
        layer.get("webUrl").and_then(Value::as_str).and_then(|url| {
            crate::web_capture::frame(
                url,
                (right - left).max(1) as u32,
                (bottom - top).max(1) as u32,
            )
            .ok()
            .flatten()
        })
    } else if let Some(path) = media_layer_path(layer) {
        if is_video {
            let frame = renderer
                .video_frame_with_clock(
                    &path,
                    layer.get("mediaPlaybackState").and_then(Value::as_object),
                    layer.get("mediaHardwareDecoding").and_then(Value::as_str) == Some("disabled"),
                )
                .ok()
                .flatten()
                .cloned()
                .map(Arc::new);
            playback_opacity = native_fill_playback::opacity(renderer, &path, layer, "media");
            frame
        } else {
            renderer.image_frame(&path).ok().map(|frame| {
                output.dynamic_media |= frame.dynamic;
                frame.image
            })
        }
    } else {
        output.dynamic_media |= media_layer_has_source(layer);
        None
    };
    let Some(image) = image else {
        output.dynamic_media = true;
        return false;
    };
    let Some((image, animated)) = adjusted_object_media_image(&image, layer, "media") else {
        return false;
    };
    output.dynamic_media |= animated;
    let width = (right - left).max(1) as usize;
    let height = (bottom - top).max(1) as usize;
    let scale = layer
        .get("mediaScale")
        .and_then(Value::as_str)
        .unwrap_or("Scale to Fit");
    let alignment = layer
        .get("mediaAlignment")
        .and_then(Value::as_str)
        .unwrap_or("Middle Center");
    let opacity = (json_number(layer.get("opacity"))
        .unwrap_or(100.0)
        .clamp(0.0, 100.0)
        / 100.0
        * playback_opacity) as f32;
    if opacity <= 0.0 {
        return true;
    }
    paint_image_in_rect(
        &mut output.pixels,
        output.width,
        output.height,
        &image,
        left,
        top,
        width,
        height,
        scale,
        alignment,
        opacity,
        layer,
        shape,
        crop_scale_x,
        crop_scale_y,
        feather_amount,
        if is_video || ndi {
            image::imageops::FilterType::Triangle
        } else {
            image::imageops::FilterType::Lanczos3
        },
    );
    true
}

fn media_layer_path(layer: &serde_json::Map<String, Value>) -> Option<PathBuf> {
    [
        "sourcePath",
        "mediaSourcePath",
        "mediaPath",
        "mediaUrl",
        "sourceUrl",
    ]
    .into_iter()
    .filter_map(|key| layer.get(key))
    .find_map(local_media_path)
}

fn media_layer_has_source(layer: &serde_json::Map<String, Value>) -> bool {
    [
        "sourcePath",
        "mediaSourcePath",
        "mediaPath",
        "mediaUrl",
        "sourceUrl",
    ]
    .into_iter()
    .filter_map(|key| layer.get(key))
    .any(|value| {
        value
            .as_str()
            .is_some_and(|source| !source.trim().is_empty())
    })
}

fn text_media_fill_has_source(style: &serde_json::Map<String, Value>) -> bool {
    ["mediaFillSourcePath", "mediaFillUrl"]
        .into_iter()
        .filter_map(|key| style.get(key))
        .any(|value| {
            value
                .as_str()
                .is_some_and(|source| !source.trim().is_empty())
        })
}

fn paint_image_in_rect(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    image: &DynamicImage,
    left: isize,
    top: isize,
    target_width: usize,
    target_height: usize,
    scale: &str,
    alignment: &str,
    opacity: f32,
    layer: &serde_json::Map<String, Value>,
    shape: Option<&ShapeGeometry>,
    crop_scale_x: f64,
    crop_scale_y: f64,
    feather_amount: ShapeFeather,
    filter: image::imageops::FilterType,
) {
    let source = image.to_rgba8();
    let target_width_f = target_width.max(1) as f64;
    let target_height_f = target_height.max(1) as f64;
    let source_width = source.width().max(1) as f64;
    let source_height = source.height().max(1) as f64;
    let normalized_scale = scale.to_ascii_lowercase();
    let fit = if normalized_scale.contains("stretch") {
        "stretch"
    } else if normalized_scale.contains("fill") || normalized_scale == "blur" {
        "fill"
    } else {
        "fit"
    };
    let factor = if fit == "fill" {
        (target_width_f / source_width).max(target_height_f / source_height)
    } else if fit == "fit" {
        (target_width_f / source_width).min(target_height_f / source_height)
    } else {
        1.0
    };
    let rendered_width = if fit == "stretch" {
        target_width
    } else {
        (source_width * factor).round().max(1.0) as usize
    };
    let rendered_height = if fit == "stretch" {
        target_height
    } else {
        (source_height * factor).round().max(1.0) as usize
    };
    let mut rendered = image::imageops::resize(
        &source,
        rendered_width as u32,
        rendered_height as u32,
        filter,
    );
    if layer
        .get("mediaFlipX")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        rendered = image::imageops::flip_horizontal(&rendered);
    }
    if layer
        .get("mediaFlipY")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        rendered = image::imageops::flip_vertical(&rendered);
    }
    let alignment_x = alignment_factor(alignment, false);
    let alignment_y = alignment_factor(alignment, true);
    let rendered_left = left
        + ((target_width as isize - rendered.width() as isize) as f64 * alignment_x).round()
            as isize;
    let rendered_top = top
        + ((target_height as isize - rendered.height() as isize) as f64 * alignment_y).round()
            as isize;
    let rotation = json_number(layer.get("mediaRotation"))
        .map(normalize_degrees)
        .unwrap_or(0.0);
    let crop = if layer
        .get("mediaCrop")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        (
            (json_number(layer.get("mediaCropTop"))
                .unwrap_or(0.0)
                .max(0.0)
                * crop_scale_y)
                .round() as usize,
            (json_number(layer.get("mediaCropRight"))
                .unwrap_or(0.0)
                .max(0.0)
                * crop_scale_x)
                .round() as usize,
            (json_number(layer.get("mediaCropBottom"))
                .unwrap_or(0.0)
                .max(0.0)
                * crop_scale_y)
                .round() as usize,
            (json_number(layer.get("mediaCropLeft"))
                .unwrap_or(0.0)
                .max(0.0)
                * crop_scale_x)
                .round() as usize,
        )
    } else {
        (0, 0, 0, 0)
    };
    let outset = if shape.is_some() {
        feather_amount.outset(target_width as f32, target_height as f32)
    } else {
        0
    };
    let radians = rotation.to_radians();
    let cos = radians.cos() as f32;
    let sin = radians.sin() as f32;
    let center_x = target_width as f32 * 0.5;
    let center_y = target_height as f32 * 0.5;
    for target_y in
        (top - outset).max(0)..(top + target_height as isize + outset).min(height as isize)
    {
        for target_x in
            (left - outset).max(0)..(left + target_width as isize + outset).min(width as isize)
        {
            let local_x = target_x - left;
            let local_y = target_y - top;
            let edge_x = local_x.clamp(0, target_width as isize - 1);
            let edge_y = local_y.clamp(0, target_height as isize - 1);
            if edge_x < crop.3 as isize
                || edge_x + crop.1 as isize >= target_width as isize
                || edge_y < crop.0 as isize
                || edge_y + crop.2 as isize >= target_height as isize
            {
                continue;
            }
            let feather = shape.map_or(1.0, |geometry| {
                shape_feather_alpha(
                    geometry,
                    (local_x as f32 + 0.5) / target_width as f32,
                    (local_y as f32 + 0.5) / target_height as f32,
                    target_width as f32,
                    target_height as f32,
                    feather_amount,
                )
            });
            if feather <= 0.0 {
                continue;
            }
            let dx = edge_x as f32 + 0.5 - center_x;
            let dy = edge_y as f32 + 0.5 - center_y;
            let source_x =
                (dx * cos + dy * sin + center_x).floor() as isize - (rendered_left - left);
            let source_y =
                (-dx * sin + dy * cos + center_y).floor() as isize - (rendered_top - top);
            if source_x < 0
                || source_y < 0
                || source_x >= rendered.width() as isize
                || source_y >= rendered.height() as isize
            {
                continue;
            }
            let [red, green, blue, alpha] = rendered.get_pixel(source_x as u32, source_y as u32).0;
            blend_pixel(
                pixels,
                width,
                target_x as usize,
                target_y as usize,
                [blue, green, red, (alpha as f32 * feather).round() as u8],
                opacity,
            );
        }
    }
}

fn alignment_factor(alignment: &str, vertical: bool) -> f64 {
    let lower = alignment.to_ascii_lowercase();
    if vertical {
        if lower.contains("top") {
            0.0
        } else if lower.contains("bottom") {
            1.0
        } else {
            0.5
        }
    } else if lower.contains("left") {
        0.0
    } else if lower.contains("right") {
        1.0
    } else {
        0.5
    }
}

fn shape_fill_css(layer: &serde_json::Map<String, Value>) -> String {
    let fill_type = layer.get("fillType").and_then(Value::as_str);
    if fill_type.is_some_and(|value| value.eq_ignore_ascii_case("gradient")) {
        if let Some(fill) = layer.get("fill").and_then(Value::as_str) {
            if parse_linear_gradient(fill).is_some() {
                return fill.to_string();
            }
        }
        let colors = layer
            .get("fillColors")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let first = colors.first().copied().unwrap_or("#0b63f6");
        let second = colors.get(1).copied().unwrap_or_else(|| {
            if colors.is_empty() {
                "#14c8e8"
            } else {
                first
            }
        });
        let angle = shape_gradient_css_angle(layer);
        return format!("linear-gradient({angle}deg, {first}, {second})");
    }
    if fill_type.is_some_and(|value| {
        value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("background-effect")
    }) {
        return "transparent".to_string();
    }
    if let Some(fill) = layer.get("fill").and_then(Value::as_str) {
        if !fill.trim().is_empty() {
            return fill.to_string();
        }
    }
    if let Some(colors) = layer.get("fillColors").and_then(Value::as_array) {
        let colors = colors.iter().filter_map(Value::as_str).collect::<Vec<_>>();
        if let Some(first) = colors.first() {
            let second = colors.get(1).copied().unwrap_or(first);
            let angle = shape_gradient_css_angle(layer);
            return format!("linear-gradient({angle}deg, {first}, {second})");
        }
    }
    "rgba(255,255,255,0.2)".to_string()
}

/// `fillAngle` keeps ProPresenter's shape gradient angle, counter-clockwise from the right (0° runs left to right,
/// 90° bottom to top, verified against ProPresenter 21.4.2 renders); CSS gradients measure clockwise from the top.
fn shape_gradient_css_angle(layer: &serde_json::Map<String, Value>) -> f64 {
    (90.0 - json_number(layer.get("fillAngle")).unwrap_or(0.0)).rem_euclid(360.0)
}

#[derive(Clone, Copy, Debug, Default)]
struct ShapeFeather {
    amount: f32,
    style: u8,
}

impl ShapeFeather {
    const NONE: Self = Self {
        amount: 0.0,
        style: 0,
    };

    fn outset(self, width: f32, height: f32) -> isize {
        if self.style == 0 {
            return 0;
        }
        let p = (self.amount / 100.0).clamp(0.0, 1.0);
        let radius = width.min(height) * (0.29 + 0.17 * p) * p;
        (radius * if self.style == 1 { 0.925 } else { 1.85 }).ceil() as isize
    }
}

fn shape_feather_amount(layer: &serde_json::Map<String, Value>) -> ShapeFeather {
    if layer.get("featheringEnabled").and_then(Value::as_bool) != Some(true) {
        return ShapeFeather::NONE;
    }
    ShapeFeather {
        amount: json_number(layer.get("featheringAmount"))
            .unwrap_or(0.0)
            .clamp(0.0, 100.0) as f32,
        style: match layer.get("featheringStyle").and_then(Value::as_str) {
            Some("center") => 1,
            Some("outside") => 2,
            _ => 0,
        },
    }
}

fn shape_feather_alpha(
    geometry: &ShapeGeometry,
    local_x: f32,
    local_y: f32,
    target_width: f32,
    target_height: f32,
    feather: ShapeFeather,
) -> f32 {
    let percent = (feather.amount / 100.0).clamp(0.0, 1.0);
    if percent <= 0.0 {
        return if geometry.contains(local_x, local_y) {
            1.0
        } else {
            0.0
        };
    }

    let feather_width = target_width.min(target_height) * (0.29 + 0.17 * percent) * percent;
    if feather_width <= f32::EPSILON {
        return 1.0;
    }
    let mut edge_distance =
        shape_edge_distance(geometry, local_x, local_y, target_width, target_height);
    let rectangular = matches!(
        geometry,
        ShapeGeometry::Rectangle | ShapeGeometry::RoundedRectangle { .. }
    );
    let edge_inset = if rectangular {
        feather_width * 0.85 * percent
    } else {
        0.0
    };
    let start_distance = if rectangular {
        edge_inset
    } else {
        feather_width * 0.24
    };
    let max_inset = (target_width.min(target_height) * 0.5 - 1.0).max(0.0);
    let full_distance =
        (edge_inset + feather_width * if rectangular { 1.0 } else { 1.28 }).min(max_inset);
    edge_distance += match feather.style {
        1 => (start_distance + full_distance) / 2.0,
        2 => full_distance,
        _ => 0.0,
    };
    if full_distance <= start_distance {
        return if edge_distance >= full_distance {
            1.0
        } else {
            0.0
        };
    }
    let progress =
        ((edge_distance - start_distance) / (full_distance - start_distance)).clamp(0.0, 1.0);
    progress * progress * (3.0 - 2.0 * progress)
}

fn shape_edge_distance(
    geometry: &ShapeGeometry,
    local_x: f32,
    local_y: f32,
    target_width: f32,
    target_height: f32,
) -> f32 {
    match geometry {
        ShapeGeometry::Rectangle => (local_x * target_width)
            .min((1.0 - local_x) * target_width)
            .min(local_y * target_height)
            .min((1.0 - local_y) * target_height),
        ShapeGeometry::RoundedRectangle { radius_x, radius_y } => {
            let radius_x_pixels = radius_x * target_width;
            let radius_y_pixels = radius_y * target_height;
            let corner_x = if local_x < *radius_x {
                *radius_x
            } else if local_x > 1.0 - *radius_x {
                1.0 - *radius_x
            } else {
                return (local_x * target_width)
                    .min((1.0 - local_x) * target_width)
                    .min(local_y * target_height)
                    .min((1.0 - local_y) * target_height);
            };
            let corner_y = if local_y < *radius_y {
                *radius_y
            } else if local_y > 1.0 - *radius_y {
                1.0 - *radius_y
            } else {
                return (local_x * target_width)
                    .min((1.0 - local_x) * target_width)
                    .min(local_y * target_height)
                    .min((1.0 - local_y) * target_height);
            };
            let distance_x = (local_x - corner_x) * target_width;
            let distance_y = (local_y - corner_y) * target_height;
            let normalized = (distance_x / radius_x_pixels.max(f32::EPSILON)).powi(2)
                + (distance_y / radius_y_pixels.max(f32::EPSILON)).powi(2);
            (1.0 - normalized.sqrt()) * radius_x_pixels.min(radius_y_pixels).max(1.0)
        }
        ShapeGeometry::Ellipse => {
            let dx = (local_x - 0.5) / 0.5;
            let dy = (local_y - 0.5) / 0.5;
            (1.0 - (dx * dx + dy * dy).sqrt()) * target_width.min(target_height) * 0.5
        }
        ShapeGeometry::Polygon(points) | ShapeGeometry::Path { points, .. } => {
            points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .map(|(start, end)| {
                    point_segment_distance(
                        local_x * target_width,
                        local_y * target_height,
                        start.0 * target_width,
                        start.1 * target_height,
                        end.0 * target_width,
                        end.1 * target_height,
                    )
                })
                .fold(f32::INFINITY, f32::min)
                * if geometry.contains(local_x, local_y) {
                    1.0
                } else {
                    -1.0
                }
        }
    }
}

fn point_segment_distance(
    point_x: f32,
    point_y: f32,
    start_x: f32,
    start_y: f32,
    end_x: f32,
    end_y: f32,
) -> f32 {
    let delta_x = end_x - start_x;
    let delta_y = end_y - start_y;
    let length_squared = delta_x * delta_x + delta_y * delta_y;
    let projection = if length_squared <= f32::EPSILON {
        0.0
    } else {
        ((point_x - start_x) * delta_x + (point_y - start_y) * delta_y) / length_squared
    }
    .clamp(0.0, 1.0);
    let closest_x = start_x + projection * delta_x;
    let closest_y = start_y + projection * delta_y;
    ((point_x - closest_x).powi(2) + (point_y - closest_y).powi(2)).sqrt()
}

pub(crate) fn leading_number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_str).and_then(|value| {
        let end = value
            .char_indices()
            .find(|(_, character)| {
                !character.is_ascii_digit() && *character != '.' && *character != '-'
            })
            .map(|(index, _)| index)
            .unwrap_or(value.len());
        value[..end].trim().parse::<f64>().ok()
    })
}

fn blur_alpha_mask(mask: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 || width == 0 || height == 0 {
        return mask.to_vec();
    }
    let mut horizontal = vec![0_u8; mask.len()];
    let mut output = vec![0_u8; mask.len()];
    let mut prefix = vec![0_usize; width.saturating_add(1)];
    for y in 0..height {
        prefix[0] = 0;
        for x in 0..width {
            prefix[x + 1] = prefix[x] + mask[y * width + x] as usize;
        }
        for x in 0..width {
            let start = x.saturating_sub(radius);
            let end = (x + radius + 1).min(width);
            let sum = prefix[end] - prefix[start];
            horizontal[y * width + x] = (sum / (end - start).max(1)) as u8;
        }
    }
    prefix.resize(height.saturating_add(1), 0);
    for x in 0..width {
        prefix[0] = 0;
        for y in 0..height {
            prefix[y + 1] = prefix[y] + horizontal[y * width + x] as usize;
        }
        for y in 0..height {
            let start = y.saturating_sub(radius);
            let end = (y + radius + 1).min(height);
            let sum = prefix[end] - prefix[start];
            output[y * width + x] = (sum / (end - start).max(1)) as u8;
        }
    }
    output
}

/// Blurs or inverts the composited backdrop inside a shape, like ProPresenter's background effect fill.
#[allow(clippy::too_many_arguments)]
fn paint_shape_background_effect(
    output: &mut RenderTarget,
    source: &serde_json::Map<String, Value>,
    left: isize,
    top: isize,
    right: isize,
    bottom: isize,
    opacity: f32,
    geometry: &ShapeGeometry,
    feather_amount: ShapeFeather,
    scale: f64,
) -> bool {
    if source.get("fillType").and_then(Value::as_str) != Some("background-effect") {
        return false;
    }
    let blur = match source.get("backgroundEffect").and_then(Value::as_str) {
        Some("invert") => None,
        Some("blur") => {
            let (Some(amount), Some(saturation)) = (
                json_number(source.get("backgroundBlurAmount")),
                json_number(source.get("backgroundBlurSaturation")),
            ) else {
                return false;
            };
            Some((
                (amount.max(0.0) * scale).round().min(256.0) as usize,
                (saturation.max(0.0) / 100.0) as f32,
            ))
        }
        _ => return false,
    };
    let width = (right - left).max(0) as usize;
    let height = (bottom - top).max(0) as usize;
    if width == 0 || height == 0 {
        return false;
    }
    let outset = feather_amount.outset(width as f32, height as f32);
    let mask_width = width + 2 * outset as usize;
    let mask_height = height + 2 * outset as usize;
    let mut coverage = vec![0_u8; mask_width * mask_height * 4];
    paint_shape_css(
        &mut coverage,
        mask_width,
        mask_height,
        outset,
        outset,
        width as isize + outset,
        height as isize + outset,
        "#ffffff",
        opacity,
        geometry,
        feather_amount,
    );
    let coverage = coverage
        .chunks_exact(4)
        .map(|pixel| pixel[3])
        .collect::<Vec<_>>();
    crate::native_render::paint_backdrop_effect(
        output.pixels,
        output.width,
        output.height,
        &coverage,
        mask_width,
        mask_height,
        left - outset,
        top - outset,
        blur,
    )
}

fn paint_shape_css(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    left: isize,
    top: isize,
    right: isize,
    bottom: isize,
    css: &str,
    opacity: f32,
    geometry: &ShapeGeometry,
    feather_amount: ShapeFeather,
) {
    let gradient = parse_linear_gradient(css);
    let color = if gradient.is_none() {
        Some(parse_color(css).unwrap_or([0, 0, 0, 0]))
    } else {
        None
    };
    let rect_width = (right - left).max(1) as f32;
    let rect_height = (bottom - top).max(1) as f32;
    let angle = gradient
        .as_ref()
        .map_or(0.0, |gradient| gradient.1)
        .to_radians();
    let dx = angle.sin();
    let dy = -angle.cos();
    let span = rect_width * dx.abs() + rect_height * dy.abs();
    let outset = feather_amount.outset(rect_width, rect_height);
    for y in (top - outset).max(0)..(bottom + outset).min(height as isize) {
        for x in (left - outset).max(0)..(right + outset).min(width as isize) {
            let local_x = (x - left) as f32 / rect_width;
            let local_y = (y - top) as f32 / rect_height;
            let color = color.unwrap_or_else(|| {
                let gradient = gradient.as_ref().unwrap();
                let (offset_x, offset_y) =
                    ((local_x - 0.5) * rect_width, (local_y - 0.5) * rect_height);
                let amount = (offset_x * dx + offset_y * dy) / span + 0.5;
                positioned_gradient_color(&gradient.0, amount.clamp(0.0, 1.0))
            });
            let feather = shape_feather_alpha(
                geometry,
                local_x + 0.5 / rect_width,
                local_y + 0.5 / rect_height,
                rect_width,
                rect_height,
                feather_amount,
            );
            if feather > 0.0 {
                blend_pixel(
                    pixels,
                    width,
                    x as usize,
                    y as usize,
                    color,
                    opacity * feather,
                );
            }
        }
    }
}

fn paint_rect_outline(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    left: isize,
    top: isize,
    right: isize,
    bottom: isize,
    stroke_width: isize,
    css: &str,
    opacity: f32,
    stroke_style: Option<&str>,
    scale: f32,
) {
    let color = parse_color(css).unwrap_or([255, 255, 255, 255]);
    let max_x = width as isize;
    let max_y = height as isize;
    let outside = stroke_width / 2;
    let inside = stroke_width - outside;
    let outer_left = left - outside;
    let outer_top = top - outside;
    let outer_right = right + inside;
    let outer_bottom = bottom + inside;
    let rect_width = (right - left).max(1) as f32;
    let rect_height = (bottom - top).max(1) as f32;
    for y in outer_top.max(0)..outer_bottom.min(max_y) {
        for x in outer_left.max(0)..outer_right.min(max_x) {
            if x - outer_left >= stroke_width
                && outer_right - 1 - x >= stroke_width
                && y - outer_top >= stroke_width
                && outer_bottom - 1 - y >= stroke_width
            {
                continue;
            }
            let local_x = (x as f32 + 0.5 - left as f32).clamp(0.0, rect_width);
            let local_y = (y as f32 + 0.5 - top as f32).clamp(0.0, rect_height);
            let edges = [
                (local_y.abs(), local_x),
                ((rect_width - local_x).abs(), rect_width + local_y),
                (
                    (rect_height - local_y).abs(),
                    rect_width + rect_height + (rect_width - local_x),
                ),
                (
                    local_x.abs(),
                    2.0 * rect_width + rect_height + (rect_height - local_y),
                ),
            ];
            let path_distance = edges
                .into_iter()
                .min_by(|left, right| {
                    left.0
                        .partial_cmp(&right.0)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|edge| edge.1)
                .unwrap_or(0.0);
            if stroke_dash_visible_at(stroke_style, path_distance, stroke_width as f32, scale) {
                blend_pixel(pixels, width, x as usize, y as usize, color, opacity);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_layer_border(
    output: &mut RenderTarget,
    layer: &serde_json::Map<String, Value>,
    left: isize,
    top: isize,
    right: isize,
    bottom: isize,
    scale: f64,
    opacity: f32,
    geometry: &ShapeGeometry,
    feather_amount: ShapeFeather,
) -> bool {
    let Some(stroke_value) = layer
        .get("stroke")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return false;
    };
    if layer.get("strokeEnabled").and_then(Value::as_bool) == Some(false) {
        return false;
    }
    let source_width = leading_number(layer.get("stroke")).unwrap_or(1.0).max(0.0);
    let stroke_width = (source_width * scale).round().max(1.0) as isize;
    let stroke_color = stroke_value
        .split_whitespace()
        .rev()
        .find(|candidate| parse_color(candidate).is_some())
        .or_else(|| {
            layer
                .get("strokeColor")
                .and_then(Value::as_str)
                .filter(|candidate| parse_color(candidate).is_some())
        })
        .unwrap_or("#ffffff");
    if let ShapeGeometry::Path { points, closed } = geometry {
        paint_path_outline(
            &mut output.pixels,
            output.width,
            output.height,
            left,
            top,
            right,
            bottom,
            points,
            *closed,
            stroke_width,
            stroke_color,
            opacity,
            layer.get("strokeStyle").and_then(Value::as_str),
            scale as f32,
            geometry,
            feather_amount,
        );
    } else if geometry.is_rectangle() && feather_amount.amount <= 0.0 {
        paint_rect_outline(
            &mut output.pixels,
            output.width,
            output.height,
            left,
            top,
            right,
            bottom,
            stroke_width,
            stroke_color,
            opacity,
            layer.get("strokeStyle").and_then(Value::as_str),
            scale as f32,
        );
    } else {
        paint_shape_outline(
            &mut output.pixels,
            output.width,
            output.height,
            left,
            top,
            right,
            bottom,
            stroke_width,
            stroke_color,
            opacity,
            geometry,
            layer.get("strokeStyle").and_then(Value::as_str),
            scale as f32,
            feather_amount,
        );
    }
    true
}

fn paint_shape_outline(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    left: isize,
    top: isize,
    right: isize,
    bottom: isize,
    stroke_width: isize,
    color: &str,
    opacity: f32,
    geometry: &ShapeGeometry,
    stroke_style: Option<&str>,
    scale: f32,
    feather_amount: ShapeFeather,
) {
    let target_width = (right - left).max(1) as f32;
    let target_height = (bottom - top).max(1) as f32;
    let radius_x = stroke_width.max(1) as f32 / target_width;
    let radius_y = stroke_width.max(1) as f32 / target_height;
    let outset = feather_amount.outset(target_width, target_height);
    let start_x = (left - stroke_width / 2 - outset).max(0);
    let start_y = (top - stroke_width / 2 - outset).max(0);
    let end_x = (right + stroke_width / 2 + outset).min(width as isize);
    let end_y = (bottom + stroke_width / 2 + outset).min(height as isize);
    for y in start_y..end_y {
        for x in start_x..end_x {
            let local_x = (x - left) as f32 / target_width;
            let local_y = (y - top) as f32 / target_height;
            if feather_amount.style == 0
                && !geometry.contains(local_x + 0.5 / target_width, local_y + 0.5 / target_height)
            {
                continue;
            }
            let edge = (feather_amount.style != 0
                && shape_edge_distance(geometry, local_x, local_y, target_width, target_height)
                    <= stroke_width as f32)
                || [
                    (-radius_x, 0.0),
                    (radius_x, 0.0),
                    (0.0, -radius_y),
                    (0.0, radius_y),
                    (-radius_x, -radius_y),
                    (radius_x, -radius_y),
                    (-radius_x, radius_y),
                    (radius_x, radius_y),
                ]
                .into_iter()
                .any(|(offset_x, offset_y)| {
                    !geometry.contains(
                        local_x + 0.5 / target_width + offset_x,
                        local_y + 0.5 / target_height + offset_y,
                    )
                });
            if !edge
                || !stroke_dash_visible(
                    stroke_style,
                    local_x,
                    local_y,
                    target_width,
                    target_height,
                    stroke_width as f32,
                    scale,
                )
            {
                continue;
            }
            blend_pixel(
                pixels,
                width,
                x as usize,
                y as usize,
                parse_color(color).unwrap_or([255, 255, 255, 255]),
                opacity
                    * shape_feather_alpha(
                        geometry,
                        local_x + 0.5 / target_width,
                        local_y + 0.5 / target_height,
                        target_width,
                        target_height,
                        feather_amount,
                    ),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_path_outline(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    left: isize,
    top: isize,
    right: isize,
    bottom: isize,
    points: &[(f32, f32)],
    closed: bool,
    stroke_width: isize,
    color: &str,
    opacity: f32,
    stroke_style: Option<&str>,
    scale: f32,
    geometry: &ShapeGeometry,
    feather: ShapeFeather,
) {
    if points.len() < 2 || stroke_width <= 0 {
        return;
    }
    let target_width = (right - left).max(1) as f32;
    let target_height = (bottom - top).max(1) as f32;
    let target_points = points
        .iter()
        .map(|(x, y)| {
            (
                left as f32 + x * target_width,
                top as f32 + y * target_height,
            )
        })
        .collect::<Vec<_>>();
    let mut segments = target_points
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .collect::<Vec<_>>();
    if closed {
        segments.push((target_points[target_points.len() - 1], target_points[0]));
    }
    let mut path_segments = Vec::with_capacity(segments.len());
    let mut path_offset = 0.0;
    for (start, end) in segments {
        let length = (end.0 - start.0).hypot(end.1 - start.1);
        if length <= f32::EPSILON {
            continue;
        }
        path_segments.push((start, end, length, path_offset));
        path_offset += length;
    }
    if path_segments.is_empty() {
        return;
    }

    let half_width = stroke_width as f32 / 2.0;
    let outset = feather.outset(target_width, target_height) as f32;
    let min_x = target_points
        .iter()
        .map(|point| point.0)
        .fold(f32::INFINITY, f32::min);
    let max_x = target_points
        .iter()
        .map(|point| point.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = target_points
        .iter()
        .map(|point| point.1)
        .fold(f32::INFINITY, f32::min);
    let max_y = target_points
        .iter()
        .map(|point| point.1)
        .fold(f32::NEG_INFINITY, f32::max);
    let start_x = (min_x - half_width - outset).floor().max(0.0) as usize;
    let end_x = (max_x + half_width + outset).ceil().min(width as f32) as usize;
    let start_y = (min_y - half_width - outset).floor().max(0.0) as usize;
    let end_y = (max_y + half_width + outset).ceil().min(height as f32) as usize;
    let color = parse_color(color).unwrap_or([255, 255, 255, 255]);

    for y in start_y..end_y {
        for x in start_x..end_x {
            let point = (x as f32 + 0.5, y as f32 + 0.5);
            let closest = path_segments
                .iter()
                .filter_map(|(start, end, length, offset)| {
                    let dx = end.0 - start.0;
                    let dy = end.1 - start.1;
                    let projection = (((point.0 - start.0) * dx + (point.1 - start.1) * dy)
                        / (length * length))
                        .clamp(0.0, 1.0);
                    let nearest = (start.0 + projection * dx, start.1 + projection * dy);
                    let distance = (point.0 - nearest.0).hypot(point.1 - nearest.1);
                    (distance <= half_width + outset)
                        .then_some((distance, offset + projection * length))
                })
                .min_by(|left, right| {
                    left.0
                        .partial_cmp(&right.0)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            let Some((distance, path_distance)) = closest else {
                continue;
            };
            if !stroke_dash_visible_at(stroke_style, path_distance, stroke_width as f32, scale) {
                continue;
            }
            let alpha = if closed {
                shape_feather_alpha(
                    geometry,
                    (point.0 - left as f32) / target_width,
                    (point.1 - top as f32) / target_height,
                    target_width,
                    target_height,
                    feather,
                )
            } else if feather.amount > 0.0 {
                let p = feather.amount / 100.0;
                let radius =
                    (target_width.min(target_height) * (0.29 + 0.17 * p) * p).max(f32::EPSILON);
                let shift = match feather.style {
                    1 => radius / 2.0,
                    2 => radius,
                    _ => 0.0,
                };
                let t = ((half_width - distance + shift) / radius).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            } else {
                1.0
            };
            blend_pixel(pixels, width, x, y, color, opacity * alpha);
        }
    }
}

fn stroke_dash_visible(
    stroke_style: Option<&str>,
    local_x: f32,
    local_y: f32,
    width: f32,
    height: f32,
    stroke_width: f32,
    scale: f32,
) -> bool {
    let perimeter = 2.0 * (width + height);
    let distance = if local_y <= 0.2 {
        local_x * width
    } else if local_x >= 0.8 {
        width + local_y * height
    } else if local_y >= 0.8 {
        width + height + (1.0 - local_x) * width
    } else {
        2.0 * width + height + (1.0 - local_y) * height
    };
    stroke_dash_visible_at(stroke_style, distance, stroke_width, scale) || perimeter <= 0.0
}

fn stroke_dash_visible_at(
    stroke_style: Option<&str>,
    distance: f32,
    stroke_width: f32,
    scale: f32,
) -> bool {
    let Some(style) = stroke_style else {
        return true;
    };
    let scale = scale.max(f32::EPSILON);
    let width = stroke_width.max(1.0);
    let (dash, gap) = match style.to_ascii_lowercase().as_str() {
        "square" => (width, (2.0 * scale).max(width * 2.0)),
        "short" => (
            (4.0 * scale).max(width * 3.0),
            (3.0 * scale).max(width * 2.0),
        ),
        "long" => (
            (10.0 * scale).max(width * 6.0),
            (6.0 * scale).max(width * 3.0),
        ),
        _ => return true,
    };
    (distance % (dash + gap)) < dash
}

fn blend_pixel(pixels: &mut [u8], width: usize, x: usize, y: usize, source: [u8; 4], opacity: f32) {
    let offset = (y * width + x) * 4;
    let source_alpha = source[3] as f32 / 255.0 * opacity.clamp(0.0, 1.0);
    if source_alpha >= 0.999 {
        pixels[offset..offset + 4].copy_from_slice(&source);
        return;
    }
    if source_alpha <= 0.0 {
        return;
    }
    let destination_alpha = pixels[offset + 3] as f32 / 255.0;
    let result_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    for channel in 0..3 {
        let source_value = source[channel] as f32 * source_alpha;
        let destination_value =
            pixels[offset + channel] as f32 * destination_alpha * (1.0 - source_alpha);
        pixels[offset + channel] = if result_alpha > 0.0 {
            ((source_value + destination_value) / result_alpha).round() as u8
        } else {
            0
        };
    }
    pixels[offset + 3] = (result_alpha * 255.0).round() as u8;
}

fn paint_css_background(pixels: &mut [u8], width: usize, height: usize, css: &str) {
    if let Some((colors, angle)) = parse_linear_gradient(css) {
        let angle = angle.to_radians();
        let direction_x = angle.sin();
        let direction_y = -angle.cos();
        let max_x = width.saturating_sub(1).max(1) as f32;
        let max_y = height.saturating_sub(1).max(1) as f32;
        let span = max_x * direction_x.abs() + max_y * direction_y.abs();
        for y in 0..height {
            for x in 0..width {
                let coordinate = ((x as f32 - max_x * 0.5) * direction_x
                    + (y as f32 - max_y * 0.5) * direction_y)
                    / span;
                let color = positioned_gradient_color(&colors, (coordinate + 0.5).clamp(0.0, 1.0));
                let offset = (y * width + x) * 4;
                pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
        return;
    }

    let color = parse_color(css).unwrap_or([0, 0, 0, 255]);
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&color);
    }
}

fn parse_linear_gradient(value: &str) -> Option<(Vec<(f32, [u8; 4])>, f32)> {
    let value = value.trim();
    let prefix = "linear-gradient(";
    if !value.to_ascii_lowercase().starts_with(prefix) || !value.ends_with(')') {
        return None;
    }
    let arguments = split_css_arguments(&value[prefix.len()..value.len() - 1]);
    if arguments.len() < 2 {
        return None;
    }
    let parse_stop = |argument: &str| -> Option<([u8; 4], Option<f32>)> {
        if let Some(color) = parse_color(argument) {
            return Some((color, None));
        }
        let (color, position) = argument.trim().rsplit_once(char::is_whitespace)?;
        let position = position.strip_suffix('%')?.parse::<f32>().ok()? / 100.0;
        if !position.is_finite() || !(0.0..=1.0).contains(&position) {
            return None;
        }
        Some((parse_color(color.trim())?, Some(position)))
    };
    let (start, angle) = if parse_stop(&arguments[0]).is_some() {
        (0, 180.0)
    } else {
        (
            1,
            arguments[0]
                .trim()
                .strip_suffix("deg")
                .and_then(|v| v.trim().parse::<f32>().ok())
                .unwrap_or(180.0),
        )
    };
    if !angle.is_finite() {
        return None;
    }
    let stops = arguments[start..]
        .iter()
        .map(|arg| parse_stop(arg))
        .collect::<Option<Vec<_>>>()?;
    if stops.len() < 2 {
        return None;
    }
    let explicit = stops[0].1.is_some();
    if stops.iter().any(|stop| stop.1.is_some() != explicit) {
        return None;
    }
    let count = stops.len();
    let stops = stops
        .into_iter()
        .enumerate()
        .map(|(i, (color, position))| (position.unwrap_or(i as f32 / (count - 1) as f32), color))
        .collect::<Vec<_>>();
    if stops.windows(2).any(|pair| pair[0].0 > pair[1].0) {
        return None;
    }
    Some((stops, angle))
}

fn positioned_gradient_color(stops: &[(f32, [u8; 4])], amount: f32) -> [u8; 4] {
    if amount < stops[0].0 {
        return stops[0].1;
    }
    let index = stops.partition_point(|stop| stop.0 <= amount);
    if index == stops.len() {
        return stops[stops.len() - 1].1;
    }
    let (left, right) = (stops[index - 1], stops[index]);
    blend_color(left.1, right.1, (amount - left.0) / (right.0 - left.0))
}

fn split_css_arguments(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0_u32;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                result.push(value[start..index].trim().to_string());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    result.push(value[start..].trim().to_string());
    result
}

pub(crate) fn parse_color(value: &str) -> Option<[u8; 4]> {
    if !value.is_ascii() {
        return None;
    }
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    if lower == "black" {
        return Some([0, 0, 0, 255]);
    }
    if lower == "white" {
        return Some([255, 255, 255, 255]);
    }
    if lower == "transparent" {
        return Some([0, 0, 0, 0]);
    }
    if let Some(value) = lower
        .strip_prefix("rgb(")
        .or_else(|| lower.strip_prefix("rgba("))
        .and_then(|value| value.strip_suffix(')'))
    {
        let channels = split_css_arguments(value);
        if channels.len() == 3 || channels.len() == 4 {
            let red = channels[0].parse::<u8>().ok()?;
            let green = channels[1].parse::<u8>().ok()?;
            let blue = channels[2].parse::<u8>().ok()?;
            let alpha = channels
                .get(3)
                .and_then(|value| value.parse::<f32>().ok())
                .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
                .unwrap_or(255);
            return Some([blue, green, red, alpha]);
        }
    }
    let value = value.trim_start_matches('#');
    match value.len() {
        6 | 8 => {
            let red = u8::from_str_radix(&value[0..2], 16).ok()?;
            let green = u8::from_str_radix(&value[2..4], 16).ok()?;
            let blue = u8::from_str_radix(&value[4..6], 16).ok()?;
            let alpha = if value.len() == 8 {
                u8::from_str_radix(&value[6..8], 16).ok()?
            } else {
                255
            };
            Some([blue, green, red, alpha])
        }
        _ => None,
    }
}

fn blend_color(start: [u8; 4], end: [u8; 4], amount: f32) -> [u8; 4] {
    let amount = amount.clamp(0.0, 1.0);
    [
        (start[0] as f32 + (end[0] as f32 - start[0] as f32) * amount).round() as u8,
        (start[1] as f32 + (end[1] as f32 - start[1] as f32) * amount).round() as u8,
        (start[2] as f32 + (end[2] as f32 - start[2] as f32) * amount).round() as u8,
        (start[3] as f32 + (end[3] as f32 - start[3] as f32) * amount).round() as u8,
    ]
}

fn draw_text_layer(
    output: &mut RenderTarget,
    slide: &SceneSlide,
    layer: &SceneLayer,
    fallback: &str,
    linked_text_context: &Value,
    use_presentation_text: bool,
    paint_box: bool,
    renderer: &mut NativeRenderRuntime,
    text_reveal_visual: Option<&native_object_builds::TextRevealVisual>,
) -> bool {
    let slide_width = slide.width;
    let slide_height = slide.height;
    let source = &layer.source;
    let bounds = source.get("bounds").and_then(Value::as_object);
    let x = bounds
        .and_then(|value| json_number(value.get("x")))
        .unwrap_or(slide_width * 0.14);
    let y = bounds
        .and_then(|value| json_number(value.get("y")))
        .unwrap_or(slide_height * 0.22);
    let bounds_width = bounds
        .and_then(|value| json_number(value.get("width")))
        .unwrap_or(slide_width * 0.72);
    let bounds_height = bounds
        .and_then(|value| json_number(value.get("height")))
        .unwrap_or(slide_height * 0.56);
    let mut style = merged_text_style(&slide.source, source);
    if let Some(color) = native_linked_text::resolve_countdown_timer_color(
        &style,
        linked_text_context,
        chrono::Local::now(),
    ) {
        style.insert("textColor".into(), json!(color));
    }
    if slide.source.get("ignoreBuilds").and_then(Value::as_bool) != Some(true) {
        if let Some(reveal) = text_reveal_visual {
            let children = reveal
                .child_opacities
                .iter()
                .map(|(index, opacity)| (index.to_string(), json!(opacity)))
                .collect();
            style.insert("_textRevealMode".into(), json!(reveal.mode.as_str()));
            style.insert(
                "_textRevealChildInitialVisible".into(),
                json!(reveal.initial_visible),
            );
            style.insert("_textRevealChildOpacities".into(), Value::Object(children));
            if let Some(index) = reveal.isolated_child {
                style.insert("_textRevealIsolatedChild".into(), json!(index));
            }
        } else if let Some(reveal) = source
            .get("textReveal")
            .filter(|value| matches!(value["mode"].as_str(), Some("paragraph" | "underline")))
        {
            let step = slide
                .source
                .get("buildStep")
                .and_then(Value::as_f64)
                .filter(|step| step.is_finite())
                .unwrap_or(0.0);
            let underline = reveal["mode"] == "underline";
            let origin = reveal.get("fromIndex").and_then(Value::as_u64).unwrap_or(0);
            let initial = if underline {
                origin.saturating_sub(1)
            } else {
                origin.max(1)
            };
            let visible = reveal["children"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|child| {
                    child["step"]
                        .as_f64()
                        .is_some_and(|child_step| child_step <= step)
                })
                .filter_map(|child| child["index"].as_u64())
                .max()
                .map_or(initial, |index| {
                    initial.max(index.saturating_add(if underline { 1 } else { 2 }))
                });
            style.insert("_textRevealMode".into(), reveal["mode"].clone());
            style.insert(
                if underline {
                    "_textRevealVisibleUnderlines"
                } else {
                    "_textRevealVisibleParagraphs"
                }
                .into(),
                json!(visible),
            );
        }
    }
    let ticker_elapsed = crate::native_ticker::apply_text_style(&mut style, linked_text_context);
    let font_size = json_number(style.get("fontSize")).unwrap_or(90.0).max(1.0);
    let color = style
        .get("textColor")
        .and_then(Value::as_str)
        .and_then(parse_color)
        .unwrap_or([255, 255, 255, 255]);
    let left = (x / slide_width * output.width as f64).round() as isize;
    let top = (y / slide_height * output.height as f64).round() as isize;
    let native_width = (bounds_width / slide_width * output.width as f64)
        .round()
        .max(1.0) as usize;
    let native_height = (bounds_height / slide_height * output.height as f64)
        .round()
        .max(1.0) as usize;
    let right = left + native_width as isize;
    let bottom = top + native_height as isize;
    let geometry = ShapeGeometry::from_layer(source, x, y, bounds_width, bounds_height);
    let opacity = json_number(source.get("opacity"))
        .unwrap_or(100.0)
        .clamp(0.0, 100.0) as f32
        / 100.0;
    let font_scale = output.height as f64 / slide_height;
    style.insert("_layerOpacity".to_string(), json!(opacity));
    style.insert("_fontScale".to_string(), json!(font_scale));
    if let Some(runs) = source.get("textRuns") {
        style.insert("_textRuns".to_string(), runs.clone());
    }
    let mut rendered = false;
    let cut_out = style
        .get("textFillMode")
        .and_then(Value::as_str)
        .is_some_and(|mode| mode.eq_ignore_ascii_case("Text Cut Out"))
        .then(|| {
            CutOutBackdrop::capture(
                output.pixels,
                output.width,
                output.height,
                (left, top, right, bottom),
            )
        });

    let source_text = source
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let use_presentation_text = source
        .get("usePresentationText")
        .and_then(Value::as_bool)
        .unwrap_or(use_presentation_text);
    let base_text = if use_presentation_text
        && (layer.kind == SceneLayerKind::Text
            || (layer.kind == SceneLayerKind::Shape && source_text.is_empty()))
    {
        fallback
    } else if source_text.is_empty() && source.get("usePresentationText").is_none() {
        fallback
    } else {
        source_text
    };
    if style.get("preserveNotesFormat").and_then(Value::as_bool) == Some(true)
        && style.get("linkedTextEnabled").and_then(Value::as_bool) != Some(false)
    {
        let slide_key = match style.get("linkedTextSource").and_then(Value::as_str) {
            Some("Slide Notes") => Some("slide"),
            Some("Next Slide Notes") => Some("nextSlide"),
            _ => None,
        };
        if let Some(note_style) = slide_key
            .and_then(|key| linked_text_context.get(key))
            .and_then(|slide| slide.get("notesTextStyle"))
            .and_then(Value::as_object)
        {
            style.extend(note_style.clone());
        }
        if let Some(runs) = slide_key
            .and_then(|key| linked_text_context.get(key))
            .and_then(|slide| slide.get("notesRuns"))
        {
            style.insert("_textRuns".to_string(), runs.clone());
        }
    }
    let margin = |field: &str, limit: usize| -> usize {
        (json_number(style.get(field)).unwrap_or(0.0).max(0.0) * font_scale)
            .round()
            .min(limit as f64) as usize
    };
    let margin_left = margin("textMarginLeft", native_width);
    let margin_right = margin("textMarginRight", native_width);
    let margin_top = margin("textMarginTop", native_height);
    let margin_bottom = margin("textMarginBottom", native_height);
    let text_left = left + margin_left as isize;
    let text_top = top + margin_top as isize;
    let text_width = native_width
        .saturating_sub(margin_left)
        .saturating_sub(margin_right);
    let text_height = native_height
        .saturating_sub(margin_top)
        .saturating_sub(margin_bottom);
    let linked_text = resolve_linked_text(base_text, &style, linked_text_context);
    if let Some(runs) = style.get("_textRuns").and_then(Value::as_array) {
        let original = runs
            .iter()
            .filter_map(|run| run.get("text").and_then(Value::as_str))
            .collect::<String>();
        if original == linked_text {
            let link = (style.get("linkedTextEnabled").and_then(Value::as_bool) != Some(false))
                .then(|| {
                    style
                        .get("linkedTextLineTransform")
                        .and_then(Value::as_str)
                        .unwrap_or("None")
                });
            if let Some(transformed) = native_linked_text::transform_text_runs(
                runs,
                link,
                style
                    .get("capitalization")
                    .and_then(Value::as_str)
                    .unwrap_or("None"),
                style
                    .get("lineTransform")
                    .and_then(Value::as_str)
                    .unwrap_or("None"),
                style.get("lineTransformDelimiter").and_then(Value::as_str),
            ) {
                style.insert("_textRuns".into(), Value::Array(transformed));
            }
        }
    }
    let linked_text = if style.get("linkedTextEnabled").and_then(Value::as_bool) != Some(false) {
        apply_line_transform(
            &linked_text,
            style
                .get("linkedTextLineTransform")
                .and_then(Value::as_str)
                .unwrap_or("None"),
            None,
        )
    } else {
        linked_text
    };
    let capitalized_text = apply_capitalization(
        &linked_text,
        style
            .get("capitalization")
            .and_then(Value::as_str)
            .unwrap_or("None"),
    );
    let text = apply_line_transform(
        &capitalized_text,
        style
            .get("lineTransform")
            .and_then(Value::as_str)
            .unwrap_or("None"),
        style.get("lineTransformDelimiter").and_then(Value::as_str),
    );
    let line_mask = (paint_box
        && style.get("linesOnlyEnabled").and_then(Value::as_bool) == Some(true))
    .then(|| {
        let mut rects = renderer.text_line_mask_rects(
            &text,
            text_width,
            text_height,
            (font_size * font_scale).max(1.0) as f32,
            &style,
        );
        for (left, top, right, bottom) in &mut rects {
            *left += margin_left as isize;
            *right += margin_left as isize;
            *top += margin_top as isize;
            *bottom += margin_top as isize;
        }
        rects
    });
    if layer.kind == SceneLayerKind::Shape && line_mask.is_some() {
        rendered |= draw_shape_shadow(
            output,
            source,
            left,
            top,
            right,
            bottom,
            slide_height,
            opacity,
            &geometry,
            shape_feather_amount(source),
            line_mask.as_deref(),
        );
    }
    let fill_outset =
        shape_feather_amount(source).outset(native_width as f32, native_height as f32);
    let clip_left = (left - fill_outset).max(0).min(output.width as isize) as usize;
    let clip_top = (top - fill_outset).max(0).min(output.height as isize) as usize;
    let clip_right = (right + fill_outset).max(0).min(output.width as isize) as usize;
    let clip_bottom = (bottom + fill_outset).max(0).min(output.height as isize) as usize;
    let before_fill = line_mask.as_ref().map(|_| {
        let mut before =
            Vec::with_capacity((clip_right - clip_left) * (clip_bottom - clip_top) * 4);
        for row in clip_top..clip_bottom {
            before.extend_from_slice(
                &output.pixels
                    [(row * output.width + clip_left) * 4..(row * output.width + clip_right) * 4],
            );
        }
        before
    });

    if paint_box
        && (source.get("fill").is_some()
            || source
                .get("fillType")
                .and_then(Value::as_str)
                .is_some_and(|value| {
                    value.eq_ignore_ascii_case("gradient")
                        || value.eq_ignore_ascii_case("media")
                        || value == "background-effect"
                }))
    {
        if geometry.is_closed()
            && paint_shape_background_effect(
                output,
                source,
                left,
                top,
                right,
                bottom,
                opacity,
                &geometry,
                shape_feather_amount(source),
                font_scale,
            )
        {
            rendered = true;
        }
        let fill = shape_fill_css(source);
        if fill != "transparent" && geometry.is_closed() {
            paint_shape_css(
                &mut output.pixels,
                output.width,
                output.height,
                left,
                top,
                right,
                bottom,
                &fill,
                opacity,
                &geometry,
                shape_feather_amount(source),
            );
            rendered = true;
        }
        if source
            .get("fillType")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("media"))
            && draw_slide_media_in_bounds(
                output,
                source,
                left,
                top,
                right,
                bottom,
                renderer,
                Some(&geometry),
                output.width as f64 / slide_width,
                output.height as f64 / slide_height,
                shape_feather_amount(source),
            )
        {
            rendered = true;
        }
    }

    if let (Some(rects), Some(before)) = (line_mask, before_fill) {
        for y in clip_top as isize..clip_bottom as isize {
            for x in clip_left as isize..clip_right as isize {
                if !rects.iter().any(|&(l, t, r, b)| {
                    x - left >= l && x - left < r && y - top >= t && y - top < b
                }) {
                    let index = (y as usize * output.width + x as usize) * 4;
                    let saved = ((y as usize - clip_top) * (clip_right - clip_left) + x as usize
                        - clip_left)
                        * 4;
                    output.pixels[index..index + 4].copy_from_slice(&before[saved..saved + 4]);
                }
            }
        }
    }

    if paint_box
        && draw_layer_border(
            output,
            source,
            left,
            top,
            right,
            bottom,
            output.height as f64 / slide_height,
            opacity,
            &geometry,
            shape_feather_amount(source),
        )
    {
        rendered = true;
    }

    if text.is_empty() {
        // A cleared ticker owns empty text and must suppress the cue fallback.
        return rendered || crate::native_ticker::is_enabled_ticker(&style);
    }
    let media_fill_enabled = style
        .get("textFillMode")
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("Media Fill"));
    let media_fill = if media_fill_enabled {
        text_media_fill_image(&style, renderer)
    } else {
        None
    };
    if media_fill_enabled && media_fill.is_none() && text_media_fill_has_source(&style) {
        output.dynamic_media = true;
    }
    if let Some(frame) = media_fill.as_ref() {
        output.dynamic_media |= frame.dynamic;
    }
    let media_fill_image = media_fill.as_ref().map(|frame| frame.image.as_ref());
    let text_color = [
        color[0],
        color[1],
        color[2],
        (color[3] as f32 * opacity).round() as u8,
    ];
    let text_font_size = (font_size * font_scale).max(1.0) as f32;
    renderer.cut_out_backdrop = cut_out;
    let text_rendered = if style.get("scrollingEnabled").and_then(Value::as_bool) == Some(true) {
        renderer.draw_scrolling_text_box_with_media(
            &mut output.pixels,
            output.width,
            output.height,
            &text,
            text_left,
            text_top,
            text_width,
            text_height,
            text_font_size,
            text_color,
            &style,
            slide_width,
            ticker_elapsed.unwrap_or(output.scroll_elapsed_seconds),
            media_fill_image,
        )
    } else {
        renderer.draw_text_box_with_media(
            &mut output.pixels,
            output.width,
            output.height,
            &text,
            text_left,
            text_top,
            text_width,
            text_height,
            text_font_size,
            text_color,
            &style,
            media_fill_image,
            None,
        )
    };
    renderer.cut_out_backdrop = None;
    if text_rendered {
        rendered = true;
    }
    rendered
}

fn text_media_fill_image(
    style: &serde_json::Map<String, Value>,
    renderer: &mut NativeRenderRuntime,
) -> Option<NativeImageFrame> {
    let path = ["mediaFillSourcePath", "mediaFillUrl"]
        .into_iter()
        .filter_map(|key| style.get(key))
        .find_map(local_media_path)?;
    let media_type = style
        .get("mediaFillType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let extension_video = path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "mp4" | "mov" | "m4v" | "webm" | "mkv" | "avi"
            )
        });
    let mut playback_opacity = 1.0;
    let frame = if media_type.eq_ignore_ascii_case("video") || extension_video {
        let frame = renderer
            .video_frame_with_clock(
                &path,
                style
                    .get("mediaFillPlaybackState")
                    .and_then(Value::as_object),
                style.get("mediaHardwareDecoding").and_then(Value::as_str) == Some("disabled"),
            )
            .ok()
            .flatten()
            .cloned()
            .map(|image| NativeImageFrame {
                image: Arc::new(image),
                dynamic: !renderer.static_images,
            });
        playback_opacity = native_fill_playback::opacity(renderer, &path, style, "mediaFill");
        frame
    } else {
        renderer.image_frame(&path).ok()
    };
    let frame = frame?;
    let (mut image, animated) = adjusted_object_media_image(&frame.image, style, "mediaFill")?;
    if playback_opacity < 1.0 {
        let mut faded = image.to_rgba8();
        for pixel in faded.pixels_mut() {
            pixel[3] = (f64::from(pixel[3]) * playback_opacity).round() as u8;
        }
        image = Arc::new(DynamicImage::ImageRgba8(faded));
    }
    Some(NativeImageFrame {
        image,
        dynamic: frame.dynamic || animated,
    })
}

fn merged_text_style(
    slide: &serde_json::Map<String, Value>,
    layer: &serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    let mut style = slide
        .get("textStyle")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(layer_style) = layer.get("textStyle").and_then(Value::as_object) {
        style.extend(layer_style.clone());
    }
    style
}

fn draw_fallback_text(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    text: &str,
    renderer: &mut NativeRenderRuntime,
) {
    let mut style = serde_json::Map::new();
    style.insert("textHorizontalAlign".to_string(), json!("left"));
    style.insert("textVerticalAlign".to_string(), json!("bottom"));
    renderer.draw_text_box(
        pixels,
        width,
        height,
        text,
        (width as f64 * 0.025).round() as isize,
        0,
        (width as f64 * 0.95).round() as usize,
        (height as f64 * 0.96).round() as usize,
        (height as f64 * 0.06).max(12.0) as f32,
        [255, 255, 255, 255],
        &style,
    );
}
