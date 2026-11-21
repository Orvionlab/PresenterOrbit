use serde_json::{Map, Value};

const DEFAULT_SLIDE_WIDTH: f64 = 1920.0;
const DEFAULT_SLIDE_HEIGHT: f64 = 1080.0;

/// The native compositor's semantic layer categories.
///
/// The layer's complete JSON map remains available on [`SceneLayer::source`].
/// These categories only provide the stable ordering and dispatch information
/// needed by a compositor; they are deliberately not a second, lossy layer
/// schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneLayerKind {
    Shape,
    Text,
    Media,
    Other,
}

impl SceneLayerKind {
    fn sort_weight(self) -> u8 {
        match self {
            Self::Shape => 0,
            Self::Text => 1,
            Self::Media | Self::Other => 2,
        }
    }
}

/// A normalized layer ready for native composition.
#[derive(Debug, Clone)]
pub struct SceneLayer {
    /// Semantic dispatch category. A shape remains a shape even when it also
    /// carries a text payload; the compositor can therefore paint both parts.
    pub kind: SceneLayerKind,
    /// Whether the layer matches the renderer's text-layer convention. This is
    /// separate from `kind` so a shape carrying text is not turned into a plain
    /// text layer.
    pub text_layer: bool,
    /// The complete normalized layer map, including fields not interpreted by
    /// the scene parser. Compositor stages can read their own fields without
    /// losing imported or future presentation data.
    pub source: Map<String, Value>,
    /// Original position before semantic sorting. This is useful when two
    /// layers have the same sort weight and makes the stable-order guarantee
    /// explicit to later compositor stages.
    pub source_index: usize,
}

impl SceneLayer {
    fn from_source(source: Map<String, Value>, source_index: usize) -> Self {
        let text_layer = is_text_layer(&source)
            || (is_shape_layer(&source)
                && source
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty()));
        let kind = if is_shape_layer(&source) {
            SceneLayerKind::Shape
        } else if text_layer {
            SceneLayerKind::Text
        } else if is_media_layer(&source) {
            SceneLayerKind::Media
        } else {
            SceneLayerKind::Other
        };

        Self {
            kind,
            text_layer,
            source,
            source_index,
        }
    }

    fn fallback() -> Self {
        let mut source = Map::new();
        source.insert("id".to_string(), Value::String("text".to_string()));
        source.insert("type".to_string(), Value::String("Text".to_string()));
        Self {
            kind: SceneLayerKind::Text,
            text_layer: true,
            source,
            source_index: 0,
        }
    }
}

/// A parsed presentation slide for native composition.
#[derive(Debug, Clone)]
pub struct SceneSlide {
    pub width: f64,
    pub height: f64,
    pub layers: Vec<SceneLayer>,
    /// The complete slide map is retained for background and future scene
    /// fields that are not yet part of the typed compositor IR.
    pub source: Map<String, Value>,
}

impl SceneSlide {
    pub fn from_value(value: &Value) -> Result<Self, String> {
        parse_scene_slide(value)
    }
}

/// Parse and normalize a renderer-compatible slide value.
pub fn parse_scene_slide(value: &Value) -> Result<SceneSlide, String> {
    parse_scene_slide_with_build_layers(value, false)
}

/// Parse a slide while retaining Build In and reached Build Out layers for
/// native output animation. Hidden layers are still excluded.
pub fn parse_scene_slide_with_build_layers(
    value: &Value,
    retain_build_layers: bool,
) -> Result<SceneSlide, String> {
    let source = value
        .as_object()
        .ok_or_else(|| "Scene slide must be an object.".to_string())?
        .clone();
    let width = finite_number(source.get("width"), DEFAULT_SLIDE_WIDTH).max(1.0);
    let height = finite_number(source.get("height"), DEFAULT_SLIDE_HEIGHT).max(1.0);
    let build_step = finite_number(source.get("buildStep"), 0.0).max(0.0);
    let ignore_builds = source.get("ignoreBuilds").and_then(Value::as_bool) == Some(true);
    let text = string_value(source.get("text"));
    let layers = normalize_layers(
        &source,
        &text,
        build_step,
        ignore_builds,
        retain_build_layers,
    );

    Ok(SceneSlide {
        width,
        height,
        layers,
        source,
    })
}

fn normalize_layers(
    slide: &Map<String, Value>,
    slide_text: &str,
    build_step: f64,
    ignore_builds: bool,
    retain_build_layers: bool,
) -> Vec<SceneLayer> {
    let input = slide
        .get("layers")
        .and_then(Value::as_array)
        .filter(|layers| !layers.is_empty());

    let mut layers = input
        .map(|values| {
            values
                .iter()
                .enumerate()
                .map(|(source_index, value)| {
                    SceneLayer::from_source(as_object(value), source_index)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![SceneLayer::fallback()]);

    // Match the renderer's single-layer coercion: a non-text, non-video/media
    // layer becomes the slide's text layer when the slide carries text. The
    // source map is cloned so all original fields remain available.
    if layers.len() == 1
        && !layers[0].text_layer
        && !slide_text.trim().is_empty()
        && !is_video_or_media_type(&layers[0].source)
    {
        let mut source = layers[0].source.clone();
        source.insert("type".to_string(), Value::String("Text".to_string()));
        layers[0] = SceneLayer::from_source(source, layers[0].source_index);
    }

    layers.retain(|layer| {
        if retain_build_layers {
            layer.source.get("hidden").and_then(Value::as_bool) != Some(true)
        } else {
            is_visible(layer, build_step, ignore_builds)
        }
    });
    if layers.is_empty()
        && !slide.contains_key("primaryTextLayerId")
        && !input.is_some_and(|values| {
            values.iter().any(|layer| {
                layer.get("build").is_some_and(Value::is_object)
                    || layer.get("buildOut").is_some_and(Value::is_object)
                    || layer.get("textReveal").is_some_and(Value::is_object)
            })
        })
    {
        return vec![SceneLayer::fallback()];
    }

    // `sort_by_key` is stable. Keeping source_index as a secondary key makes
    // the intended ordering explicit even if this implementation changes to
    // an unstable sort in the future.
    if let Some(Value::Array(order)) = slide.get("themeLayerOrder") {
        layers.sort_by_key(|layer| {
            order
                .iter()
                .position(|id| layer.source.get("id") == Some(id))
                .unwrap_or(order.len())
        });
    } else if !slide.contains_key("themeLayerOrder") {
        layers.sort_by_key(|layer| (layer.kind.sort_weight(), layer.source_index));
    }
    layers
}

fn is_visible(layer: &SceneLayer, slide_build_step: f64, ignore_builds: bool) -> bool {
    if layer.source.get("hidden").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    if ignore_builds {
        return true;
    }
    if layer
        .source
        .get("build")
        .and_then(Value::as_object)
        .and_then(|build| finite_optional_number(build.get("step")))
        .is_some_and(|step| step > slide_build_step)
    {
        return false;
    }
    !layer
        .source
        .get("buildOut")
        .and_then(Value::as_object)
        .and_then(|build| finite_optional_number(build.get("step")))
        .is_some_and(|step| step > 0.0 && step <= slide_build_step)
}

/// Match the renderer's text-layer convention.
pub fn is_text_layer(layer: &Map<String, Value>) -> bool {
    let layer_type = normalized_string(layer.get("type"));
    let role = normalized_string(layer.get("role"));
    let name = string_value(layer.get("name")).trim().to_ascii_lowercase();
    let title = string_value(layer.get("title")).trim().to_ascii_lowercase();
    let id = normalized_string(layer.get("id"));

    layer_type == "text"
        || role == "slide-text"
        || name == "text"
        || title == "text"
        || id == "text"
}

/// Match shape layers by their explicit semantic type.
pub fn is_shape_layer(layer: &Map<String, Value>) -> bool {
    normalized_string(layer.get("type")) == "shape"
}

fn is_media_layer(layer: &Map<String, Value>) -> bool {
    matches!(
        normalized_string(layer.get("type")).as_str(),
        "image" | "video" | "media"
    )
}

fn is_video_or_media_type(layer: &Map<String, Value>) -> bool {
    matches!(
        normalized_string(layer.get("type")).as_str(),
        "video" | "media"
    )
}

fn as_object(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn string_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(value) if !value.is_null() => value.to_string(),
        _ => String::new(),
    }
}

fn normalized_string(value: Option<&Value>) -> String {
    string_value(value).trim().to_ascii_lowercase()
}

fn finite_number(value: Option<&Value>, fallback: f64) -> f64 {
    finite_optional_number(value).unwrap_or(fallback)
}

fn finite_optional_number(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    let parsed = match value {
        Value::Number(value) => value.as_f64()?,
        Value::String(value) => value.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    parsed.is_finite().then_some(parsed)
}
