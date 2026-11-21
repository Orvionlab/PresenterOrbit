use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FillMode {
    Straight,
    Premultiplied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyFillConfig {
    pub key_screen_id: String,
    pub fill_mode: FillMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyFillSignal {
    Key,
    FillStraight,
    FillPremultiplied,
}

pub fn parse_optional(value: Option<&Value>) -> Result<Option<KeyFillConfig>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let config: KeyFillConfig = serde_json::from_value(value.clone())
        .map_err(|error| format!("Invalid key/fill configuration: {error}"))?;
    if config.key_screen_id.trim().is_empty() {
        return Err("Key/fill requires a key screen ID.".into());
    }
    Ok(Some(config))
}

pub fn validate_screens(value: &Value) -> Result<(), String> {
    let screens = value
        .as_object()
        .ok_or("Screen configuration must be an object.")?;
    let mut keys = BTreeSet::new();
    for (source_id, source) in screens {
        let Some(pair) = parse_optional(source.get("keyFill"))? else {
            continue;
        };
        let key = screens
            .get(&pair.key_screen_id)
            .ok_or("Key/fill key screen was not found.")?;
        if pair.key_screen_id == *source_id || !keys.insert(pair.key_screen_id.clone()) {
            return Err("Key/fill requires a distinct, exclusively assigned key screen.".into());
        }
        if source.get("type").and_then(Value::as_str) != Some("audience")
            || key.get("type").and_then(Value::as_str) != Some("audience")
        {
            return Err("Key/fill requires two audience screens.".into());
        }
        if !matches!(
            source.get("targetType").and_then(Value::as_str),
            Some("display" | "ndi")
        ) || source.get("targetType") != key.get("targetType")
        {
            return Err(
                "Key/fill requires the same display or NDI transport on both outputs.".into(),
            );
        }
        if source
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            != key.get("enabled").and_then(Value::as_bool).unwrap_or(false)
        {
            return Err("Key/fill outputs must be enabled or disabled together.".into());
        }
        if let Some(transport @ ("display" | "ndi")) =
            source.get("targetType").and_then(Value::as_str)
        {
            if key.get("targetType").and_then(Value::as_str) == Some(transport) {
                let field = if transport == "display" {
                    "targetId"
                } else {
                    "ndiName"
                };
                let destination = |screen: &Value| {
                    screen
                        .get(field)
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                };
                let first = destination(source);
                let second = destination(key);
                if first.is_none() || second.is_none() || first == second {
                    return Err(
                        "Key/fill requires two explicit, distinct output destinations.".into(),
                    );
                }
            }
        }
        for (screen, fields) in [
            (source, &[][..]),
            (
                key,
                &["keyFill", "canvasView", "canvasSize", "projectorCorrection"][..],
            ),
        ] {
            if fields
                .iter()
                .any(|field| screen.get(*field).is_some_and(|value| !value.is_null()))
            {
                return Err("Key/fill cannot combine a key target or fill source with independent canvas following or projector correction.".into());
            }
            if screen
                .get("outputFormat")
                .and_then(Value::as_str)
                .is_some_and(|format| format != "sdr")
            {
                return Err("Key/fill currently requires SDR outputs.".into());
            }
        }
        if crate::output_delay::frames(source.get("videoDelay"))?
            != crate::output_delay::frames(key.get("videoDelay"))?
        {
            return Err("Key/fill outputs must use the same video delay.".into());
        }
        if source.get("targetType").and_then(Value::as_str) == Some("ndi")
            && key.get("targetType").and_then(Value::as_str) == Some("ndi")
            && ["ndiWidth", "ndiHeight", "ndiFramerate"]
                .iter()
                .any(|field| source.get(*field) != key.get(*field))
        {
            return Err(
                "Key/fill NDI outputs must have matching dimensions and frame rates.".into(),
            );
        }
        if screens.values().any(|screen| {
            screen
                .pointer("/canvasView/sourceScreenId")
                .and_then(Value::as_str)
                == Some(pair.key_screen_id.as_str())
        }) {
            return Err("A key screen cannot be used as a canvas source.".into());
        }
    }
    Ok(())
}

pub fn convert_pixels(pixels: &mut [u8], signal: KeyFillSignal) {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3];
        match signal {
            KeyFillSignal::Key => pixel[..3].fill(alpha),
            KeyFillSignal::FillStraight if alpha == 0 => pixel[..3].fill(0),
            KeyFillSignal::FillStraight => {}
            KeyFillSignal::FillPremultiplied => {
                for channel in &mut pixel[..3] {
                    *channel = ((*channel as u16 * alpha as u16 + 127) / 255) as u8;
                }
            }
        }
        pixel[3] = 255;
    }
}

pub fn apply_scenes(
    screens: &Value,
    sizes: &std::collections::BTreeMap<String, Value>,
    outputs: &mut serde_json::Map<String, Value>,
) -> Result<(), String> {
    let screens = screens
        .as_object()
        .ok_or("Screen configuration must be an object.")?;
    for (source_id, source) in screens {
        let Some(pair) = parse_optional(source.get("keyFill"))? else {
            continue;
        };
        let source_size = crate::output_canvas::parse_output_size(sizes.get(source_id))?;
        let key_size = crate::output_canvas::parse_output_size(sizes.get(&pair.key_screen_id))?;
        if source_size != key_size {
            return Err("Key/fill outputs must have matching output dimensions.".into());
        }
        let mut key_scene = outputs
            .get(source_id)
            .ok_or("Key/fill source scene is unavailable.")?
            .clone();
        key_scene["keyFillSourceId"] = Value::String(source_id.clone());
        key_scene["keyFillPartnerId"] = Value::String(source_id.clone());
        key_scene["keyFillSignal"] =
            serde_json::to_value(KeyFillSignal::Key).map_err(|error| error.to_string())?;
        outputs.insert(pair.key_screen_id.clone(), key_scene);
        let source_scene = outputs
            .get_mut(source_id)
            .ok_or("Key/fill source scene is unavailable.")?;
        source_scene["keyFillSourceId"] = Value::String(source_id.clone());
        source_scene["keyFillPartnerId"] = Value::String(pair.key_screen_id.clone());
        outputs
            .get_mut(source_id)
            .ok_or("Key/fill source scene is unavailable.")?["keyFillSignal"] =
            serde_json::to_value(match pair.fill_mode {
                FillMode::Straight => KeyFillSignal::FillStraight,
                FillMode::Premultiplied => KeyFillSignal::FillPremultiplied,
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn mirror_activation(
    screens: &mut serde_json::Map<String, Value>,
    mutation: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    if mutation.get("type").and_then(Value::as_str) != Some("update") {
        return Ok(());
    }
    let Some(enabled) = mutation.get("patch").and_then(|patch| patch.get("enabled")) else {
        return Ok(());
    };
    let enabled = enabled
        .as_bool()
        .ok_or("Screen activation must be a boolean.")?;
    let id = mutation
        .get("screenId")
        .and_then(Value::as_str)
        .ok_or("Screen ID is unavailable.")?;
    let mut partners = Vec::new();
    for (source_id, screen) in screens.iter() {
        if let Some(pair) = parse_optional(screen.get("keyFill"))? {
            if source_id == id {
                partners.push(pair.key_screen_id);
            } else if pair.key_screen_id == id {
                partners.push(source_id.clone());
            }
        }
    }
    for partner in partners {
        screens
            .get_mut(&partner)
            .ok_or("Key/fill activation partner is unavailable.")?["enabled"] =
            Value::Bool(enabled);
    }
    Ok(())
}

#[derive(Default)]
pub struct PairDelay {
    fill: crate::output_delay::OutputDelay,
    key: crate::output_delay::OutputDelay,
    retained: Option<(
        crate::native_compositor::NativeFrame,
        crate::native_compositor::NativeFrame,
    )>,
    format: Option<(usize, usize, usize, std::time::Duration)>,
}

impl PairDelay {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn push(
        &mut self,
        frames: (
            crate::native_compositor::NativeFrame,
            crate::native_compositor::NativeFrame,
        ),
        delay: usize,
        interval: std::time::Duration,
        now: std::time::Instant,
    ) -> Result<
        (
            crate::native_compositor::NativeFrame,
            crate::native_compositor::NativeFrame,
        ),
        String,
    > {
        let (fill, key) = frames;
        if fill.width != key.width || fill.height != key.height {
            self.clear();
            return Err("Key/fill frame dimensions differ.".into());
        }
        crate::output_delay::validate_size(
            fill.width,
            fill.height
                .checked_mul(2)
                .ok_or("Key/fill dimensions overflow.")?,
            delay,
        )?;
        let format = (fill.width, fill.height, delay, interval);
        if self.format != Some(format) {
            self.clear();
            self.format = Some(format);
        }
        let (width, height) = (fill.width, fill.height);
        let ready = self.fill.push(fill, delay, interval, now).and_then(|fill| {
            self.key
                .push(key, delay, interval, now)
                .map(|key| (fill, key))
        });
        match ready {
            Ok((Some(fill), Some(key))) => self.retained = Some((fill, key)),
            Ok((None, None)) => {}
            Ok(_) => {
                self.clear();
                return Err("Key/fill delay queues lost their common capture sequence.".into());
            }
            Err(error) => {
                self.clear();
                return Err(error);
            }
        }
        let mut frames = self.retained.clone().unwrap_or_else(|| {
            let mut pixels = vec![0; width * height * 4];
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
            let frame = crate::native_compositor::NativeFrame {
                width,
                height,
                pixels,
                dynamic_media: true,
            };
            (frame.clone(), frame)
        });
        if delay > 0 {
            frames.0.dynamic_media = true;
            frames.1.dynamic_media = true;
        }
        Ok(frames)
    }
}
