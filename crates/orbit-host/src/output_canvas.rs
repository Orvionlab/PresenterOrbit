use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_CANVAS_DIMENSION: usize = 8_192;
pub const MAX_CANVAS_PIXELS: usize = 33_554_432;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasSize {
    pub width: usize,
    pub height: usize,
}

impl CanvasSize {
    pub fn validate(self) -> Result<Self, String> {
        validate_dimensions(self.width, self.height, "Canvas")?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasRegion {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl CanvasRegion {
    pub fn validate(self, canvas: CanvasSize) -> Result<Self, String> {
        validate_dimensions(self.width, self.height, "Canvas region")?;
        let right = self
            .x
            .checked_add(self.width)
            .ok_or_else(|| "Canvas region exceeds the source canvas.".to_string())?;
        let bottom = self
            .y
            .checked_add(self.height)
            .ok_or_else(|| "Canvas region exceeds the source canvas.".to_string())?;
        if right > canvas.width || bottom > canvas.height {
            return Err("Canvas region exceeds the source canvas.".to_string());
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasView {
    pub source_screen_id: String,
    #[serde(default)]
    pub region: Option<CanvasRegion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasViewport {
    pub canvas_width: usize,
    pub canvas_height: usize,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl CanvasViewport {
    pub fn can_render_directly(&self, width: usize, height: usize) -> bool {
        self.x == 0
            && self.y == 0
            && self.width == self.canvas_width
            && self.height == self.canvas_height
            && (height as f64 * self.canvas_width as f64 / self.canvas_height as f64 - width as f64)
                .abs()
                <= 1.
    }

    pub fn new(canvas: CanvasSize, region: CanvasRegion) -> Result<Self, String> {
        let canvas = canvas.validate()?;
        let region = region.validate(canvas)?;
        Ok(Self {
            canvas_width: canvas.width,
            canvas_height: canvas.height,
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        })
    }

    pub fn validate(self) -> Result<Self, String> {
        let canvas = CanvasSize {
            width: self.canvas_width,
            height: self.canvas_height,
        }
        .validate()?;
        CanvasRegion {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
        .validate(canvas)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCanvasConfig {
    pub size: Option<CanvasSize>,
    pub view: Option<CanvasView>,
    pub screen_type: String,
}

pub fn validate_screen_config_map(value: &Value) -> Result<(), String> {
    let expanded = crate::output_layout::expand(value)?;
    parse_screen_config_map(&expanded).map(|_| ())
}

pub fn parse_screen_config_map(
    value: &Value,
) -> Result<std::collections::BTreeMap<String, ScreenCanvasConfig>, String> {
    crate::key_fill::validate_screens(value)?;
    let screens = value
        .as_object()
        .ok_or_else(|| "Screen configuration must be an object.".to_string())?;
    let mut parsed = std::collections::BTreeMap::new();
    for (screen_id, value) in screens {
        let screen = value
            .as_object()
            .ok_or_else(|| format!("Screen configuration '{screen_id}' must be an object."))?;
        crate::projector_correction::parse_optional(screen.get("projectorCorrection"))
            .map_err(|error| format!("Screen '{screen_id}': {error}"))?;
        let size = parse_optional::<CanvasSize>(screen.get("canvasSize"), "canvasSize")?
            .map(CanvasSize::validate)
            .transpose()?;
        let view = parse_optional::<CanvasView>(screen.get("canvasView"), "canvasView")?;
        if let Some(view) = &view {
            if view.source_screen_id.trim().is_empty() {
                return Err(format!(
                    "Screen '{screen_id}' canvasView sourceScreenId must not be empty."
                ));
            }
            if size.is_some() {
                return Err(format!(
                    "Screen '{screen_id}' cannot define canvasSize while following another screen."
                ));
            }
        }
        parsed.insert(
            screen_id.clone(),
            ScreenCanvasConfig {
                size,
                view,
                screen_type: screen_type(screen_id, screen),
            },
        );
    }

    for (screen_id, config) in &parsed {
        let Some(view) = &config.view else {
            continue;
        };
        if view.source_screen_id == *screen_id {
            return Err(format!("Screen '{screen_id}' cannot follow itself."));
        }
        let source = parsed.get(&view.source_screen_id).ok_or_else(|| {
            format!(
                "Screen '{screen_id}' canvas source '{}' was not found.",
                view.source_screen_id
            )
        })?;
        if source.view.is_some() {
            return Err(format!(
                "Screen '{screen_id}' canvas source '{}' cannot itself follow another screen.",
                view.source_screen_id
            ));
        }
        if !matches!(config.screen_type.as_str(), "audience" | "stage")
            || !matches!(source.screen_type.as_str(), "audience" | "stage")
            || config.screen_type != source.screen_type
        {
            return Err(format!(
                "Screen '{screen_id}' and its canvas source must have the same audience or stage type."
            ));
        }
        if let Some(region) = view.region {
            let canvas = source.size.ok_or_else(|| {
                format!(
                    "Canvas source '{}' must define canvasSize before a tile can use a region.",
                    view.source_screen_id
                )
            })?;
            region
                .validate(canvas)
                .map_err(|error| format!("Screen '{screen_id}': {error}"))?;
        }
    }
    Ok(parsed)
}

pub fn parse_output_size(value: Option<&Value>) -> Result<CanvasSize, String> {
    let value = value.ok_or_else(|| "Output canvas dimensions are unavailable.".to_string())?;
    let size: CanvasSize = serde_json::from_value(value.clone())
        .map_err(|error| format!("Output canvas dimensions are invalid: {error}"))?;
    size.validate()
        .map_err(|error| format!("Output canvas dimensions are invalid: {error}"))
}

fn parse_optional<T>(value: Option<&Value>, field: &str) -> Result<Option<T>, String>
where
    T: for<'de> Deserialize<'de>,
{
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|error| format!("Screen {field} is invalid: {error}"))
}

fn screen_type(screen_id: &str, screen: &serde_json::Map<String, Value>) -> String {
    screen
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if screen_id.starts_with("stage-") {
                "stage"
            } else {
                "audience"
            }
        })
        .to_string()
}

fn validate_dimensions(width: usize, height: usize, label: &str) -> Result<(), String> {
    let pixels = width.checked_mul(height);
    if width == 0
        || height == 0
        || width > MAX_CANVAS_DIMENSION
        || height > MAX_CANVAS_DIMENSION
        || pixels.is_none_or(|pixels| pixels > MAX_CANVAS_PIXELS)
    {
        return Err(format!(
            "{label} dimensions must be positive, at most {MAX_CANVAS_DIMENSION} pixels per side, and contain at most {MAX_CANVAS_PIXELS} pixels."
        ));
    }
    Ok(())
}
