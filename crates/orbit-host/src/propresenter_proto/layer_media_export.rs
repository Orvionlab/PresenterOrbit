//! ProPresenter media type properties shared by slide media and text media fills.

use super::*;
use serde_json::Value;

const SCALES: [&str; 3] = ["Scale to Fit", "Scale to Fill", "Stretch to Fill"];
const ALIGNMENTS: [&str; 9] = [
    "Middle Center",
    "Top Left",
    "Top Center",
    "Top Right",
    "Middle Right",
    "Bottom Right",
    "Bottom Center",
    "Bottom Left",
    "Middle Left",
];
const ADJUST_COLOR: &str = "D4C199A9-64D7-4CCB-8C26-443E7A9C5C03";

fn key(prefix: &str, suffix: &str) -> String {
    format!("{prefix}{suffix}")
}

fn value<'a>(layer: &'a Value, prefix: &str, suffix: &str) -> Option<&'a Value> {
    layer.get(key(prefix, suffix))
}

fn kind<'a>(layer: &'a Value, prefix: &str) -> Option<&'a str> {
    value(layer, prefix, "Type").and_then(Value::as_str)
}

fn valid_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

fn valid_nonnegative(value: &Value) -> Option<f64> {
    valid_number(value).filter(|number| *number >= 0.0)
}

fn valid_u32(value: &Value) -> Option<u32> {
    valid_number(value)
        .filter(|number| *number >= 0.0 && number.fract() == 0.0 && *number <= u32::MAX as f64)
        .map(|number| number as u32)
}

fn is_drawing_type(kind: &str) -> bool {
    matches!(kind, "image" | "video" | "web")
}

fn is_audio_type(kind: &str) -> bool {
    matches!(kind, "audio" | "video")
}

fn is_transport_type(kind: &str) -> bool {
    is_audio_type(kind)
}

fn neutral(layer: &Value, prefix: &str, suffix: &str) -> bool {
    let Some(value) = value(layer, prefix, suffix) else {
        return true;
    };
    match suffix {
        "Scale" => {
            value.as_str()
                == Some(if prefix == "mediaFill" {
                    "Scale to Fill"
                } else {
                    "Scale to Fit"
                })
        }
        "Alignment" => value.as_str() == Some("Middle Center"),
        "FlipX" | "FlipY" | "Crop" => value.as_bool() == Some(false),
        "Rotation" | "CropLeft" | "CropRight" | "CropTop" | "CropBottom" | "RampIn" | "RampOut"
        | "LoopTime" => valid_number(value) == Some(0.0),
        "TimesToLoop" => valid_u32(value) == Some(0),
        "Playback" => value.as_str() == Some("Loop"),
        "Volume" => valid_number(value) == Some(100.0),
        "AlphaType" => value.as_str() == Some("Straight"),
        "Effects" => value.as_array().is_some_and(Vec::is_empty),
        "Hue" | "Brightness" => valid_number(value) == Some(0.0),
        "Saturation" | "Contrast" => valid_number(value) == Some(1.0),
        _ => false,
    }
}

/// Media settings that carry non-default authored values and would be ignored without a media fill.
pub(super) fn active_fields(layer: &Value, prefix: &str) -> Vec<String> {
    [
        "Scale",
        "Alignment",
        "FlipX",
        "FlipY",
        "Rotation",
        "Crop",
        "CropLeft",
        "CropRight",
        "CropTop",
        "CropBottom",
        "Effects",
        "AlphaType",
        "Volume",
        "Playback",
        "RampIn",
        "RampOut",
        "LoopTime",
        "TimesToLoop",
        "Hue",
        "Saturation",
        "Brightness",
        "Contrast",
    ]
    .into_iter()
    .filter(|suffix| value(layer, prefix, suffix).is_some_and(|_| !neutral(layer, prefix, suffix)))
    .map(|suffix| key(prefix, suffix))
    .collect()
}

fn mark_if(
    layer: &Value,
    prefix: &str,
    suffix: &str,
    supported: bool,
    valid: bool,
    unsupported: &mut Vec<String>,
) {
    if value(layer, prefix, suffix).is_some()
        && (!valid || (!supported && !neutral(layer, prefix, suffix)))
    {
        unsupported.push(key(prefix, suffix));
    }
}

/// Returns active media keys that do not have a faithful schema representation for this type.
/// Defaults used by the editor stay quiet when a media type has no corresponding property.
pub(super) fn unsupported_fields(layer: &Value, prefix: &str) -> Vec<String> {
    let Some(kind) = kind(layer, prefix) else {
        return Vec::new();
    };
    let mut unsupported = Vec::new();
    let drawing = is_drawing_type(kind);

    for (suffix, valid) in [
        (
            "Scale",
            value(layer, prefix, "Scale")
                .and_then(Value::as_str)
                .is_some_and(|value| SCALES.contains(&value)),
        ),
        (
            "Alignment",
            value(layer, prefix, "Alignment")
                .and_then(Value::as_str)
                .is_some_and(|value| ALIGNMENTS.contains(&value)),
        ),
        (
            "FlipX",
            value(layer, prefix, "FlipX").is_some_and(Value::is_boolean),
        ),
        (
            "FlipY",
            value(layer, prefix, "FlipY").is_some_and(Value::is_boolean),
        ),
        (
            "Rotation",
            value(layer, prefix, "Rotation").is_some_and(|value| valid_number(value).is_some()),
        ),
        (
            "Crop",
            value(layer, prefix, "Crop").is_some_and(Value::is_boolean),
        ),
        (
            "CropLeft",
            value(layer, prefix, "CropLeft").is_some_and(|value| valid_number(value).is_some()),
        ),
        (
            "CropRight",
            value(layer, prefix, "CropRight").is_some_and(|value| valid_number(value).is_some()),
        ),
        (
            "CropTop",
            value(layer, prefix, "CropTop").is_some_and(|value| valid_number(value).is_some()),
        ),
        (
            "CropBottom",
            value(layer, prefix, "CropBottom").is_some_and(|value| valid_number(value).is_some()),
        ),
        (
            "Effects",
            value(layer, prefix, "Effects")
                .is_some_and(|value| crate::native_media_effects::validate_chain(value).is_ok()),
        ),
        (
            "AlphaType",
            value(layer, prefix, "AlphaType")
                .and_then(Value::as_str)
                .is_some_and(|value| matches!(value, "Straight" | "Premultiplied")),
        ),
    ] {
        mark_if(layer, prefix, suffix, drawing, valid, &mut unsupported);
    }

    let audio = is_audio_type(kind);
    mark_if(
        layer,
        prefix,
        "Volume",
        audio,
        value(layer, prefix, "Volume").is_some_and(|value| {
            valid_number(value).is_some_and(|number| (0.0..=100.0).contains(&number))
        }),
        &mut unsupported,
    );

    let transport = is_transport_type(kind);
    mark_if(
        layer,
        prefix,
        "Playback",
        transport,
        value(layer, prefix, "Playback")
            .and_then(Value::as_str)
            .is_some_and(|value| matches!(value, "Loop" | "Stop")),
        &mut unsupported,
    );
    for suffix in ["RampIn", "RampOut", "LoopTime"] {
        mark_if(
            layer,
            prefix,
            suffix,
            transport,
            value(layer, prefix, suffix).is_some_and(|value| valid_nonnegative(value).is_some()),
            &mut unsupported,
        );
    }
    mark_if(
        layer,
        prefix,
        "TimesToLoop",
        transport,
        value(layer, prefix, "TimesToLoop").is_some_and(|value| valid_u32(value).is_some()),
        &mut unsupported,
    );
    if value(layer, prefix, "LoopTime")
        .and_then(valid_nonnegative)
        .is_some_and(|number| number > 0.0)
        && value(layer, prefix, "TimesToLoop")
            .and_then(valid_u32)
            .is_some_and(|number| number > 0)
    {
        unsupported.push(key(prefix, "LoopTime"));
        unsupported.push(key(prefix, "TimesToLoop"));
    }

    for (suffix, range) in [
        ("Hue", (-180.0, 180.0)),
        ("Saturation", (0.0, 2.0)),
        ("Brightness", (-1.0, 1.0)),
        ("Contrast", (0.0, 2.0)),
    ] {
        let valid = value(layer, prefix, suffix).is_some_and(|value| {
            valid_number(value).is_some_and(|number| (range.0..=range.1).contains(&number))
        });
        mark_if(layer, prefix, suffix, drawing, valid, &mut unsupported);
    }

    unsupported
}

fn drawing_properties(layer: &Value, prefix: &str) -> Vec<u8> {
    let mut drawing = Vec::new();
    if let Some(scale) = value(layer, prefix, "Scale").and_then(Value::as_str) {
        if let Some(number) = SCALES.iter().position(|candidate| *candidate == scale) {
            put_varint(1, number as u64, &mut drawing);
        }
    }
    if let Some(alignment) = value(layer, prefix, "Alignment").and_then(Value::as_str) {
        if let Some(number) = ALIGNMENTS
            .iter()
            .position(|candidate| *candidate == alignment)
        {
            put_varint(2, number as u64, &mut drawing);
        }
    }
    for (field, suffix) in [(3, "FlipX"), (4, "FlipY")] {
        if let Some(value) = value(layer, prefix, suffix).and_then(Value::as_bool) {
            put_varint(field, u64::from(value), &mut drawing);
        }
    }
    if let Some(rotation) = value(layer, prefix, "Rotation").and_then(valid_number) {
        put_f64(6, rotation, &mut drawing);
    }
    if let Some(crop) = value(layer, prefix, "Crop").and_then(Value::as_bool) {
        put_varint(13, u64::from(crop), &mut drawing);
    }
    let mut insets = Vec::new();
    for (field, suffix) in [
        (1, "CropLeft"),
        (2, "CropRight"),
        (3, "CropTop"),
        (4, "CropBottom"),
    ] {
        if let Some(value) = value(layer, prefix, suffix).and_then(valid_number) {
            put_f64(field, value, &mut insets);
        }
    }
    if !insets.is_empty() {
        put_message(14, insets, &mut drawing);
    }
    if let Some(alpha_type) = value(layer, prefix, "AlphaType").and_then(Value::as_str) {
        match alpha_type {
            "Straight" => put_varint(15, 1, &mut drawing),
            "Premultiplied" => put_varint(15, 2, &mut drawing),
            _ => {}
        }
    }
    if let Some(bytes) = color_adjustment(layer, prefix) {
        put_message(12, bytes, &mut drawing);
    }
    if let Some(chain) = value(layer, prefix, "Effects")
        .filter(|chain| crate::native_media_effects::validate_chain(chain).is_ok())
        .and_then(Value::as_array)
    {
        for effect in chain {
            if let Some(bytes) = super::encode_media_effect(effect) {
                put_message(12, bytes, &mut drawing);
            }
        }
    }
    drawing
}

fn color_adjustment(layer: &Value, prefix: &str) -> Option<Vec<u8>> {
    let numbers = [
        value(layer, prefix, "Hue")
            .and_then(valid_number)
            .filter(|value| (-180.0..=180.0).contains(value))
            .unwrap_or(0.0),
        value(layer, prefix, "Saturation")
            .and_then(valid_number)
            .filter(|value| (0.0..=2.0).contains(value))
            .unwrap_or(1.0),
        value(layer, prefix, "Brightness")
            .and_then(valid_number)
            .filter(|value| (-1.0..=1.0).contains(value))
            .unwrap_or(0.0),
        value(layer, prefix, "Contrast")
            .and_then(valid_number)
            .filter(|value| (0.0..=2.0).contains(value))
            .unwrap_or(1.0),
    ];
    if numbers == [0.0, 1.0, 0.0, 1.0] {
        return None;
    }
    let effect = json!({
        "effectId": ADJUST_COLOR,
        "enabled": true,
        "variables": {
            "hue": numbers[0].to_radians(),
            "saturation": numbers[1],
            "brightness": numbers[2],
            "contrast": numbers[3],
        },
    });
    super::encode_media_effect(&effect)
}

fn audio_properties(layer: &Value, prefix: &str) -> Vec<u8> {
    let mut audio = Vec::new();
    if let Some(volume) = value(layer, prefix, "Volume")
        .and_then(valid_number)
        .filter(|volume| (0.0..=100.0).contains(volume))
    {
        put_f64(1, volume / 100.0, &mut audio);
    }
    audio
}

fn transport_properties(layer: &Value, prefix: &str) -> Vec<u8> {
    let mut transport = Vec::new();
    for (field, suffix) in [(7, "RampIn"), (8, "RampOut")] {
        if let Some(ramp) = value(layer, prefix, suffix).and_then(valid_nonnegative) {
            put_f64(field, ramp, &mut transport);
            if ramp > 0.0 {
                put_varint(field + 2, 1, &mut transport);
            }
        }
    }
    let loop_time = value(layer, prefix, "LoopTime")
        .and_then(valid_nonnegative)
        .is_some_and(|number| number > 0.0);
    let loop_count = value(layer, prefix, "TimesToLoop")
        .and_then(valid_u32)
        .is_some_and(|number| number > 0);
    if !loop_time || !loop_count {
        if let Some(loop_time) = value(layer, prefix, "LoopTime").and_then(valid_nonnegative) {
            put_f64(13, loop_time, &mut transport);
        }
        if let Some(times) = value(layer, prefix, "TimesToLoop").and_then(valid_u32) {
            put_varint(14, u64::from(times), &mut transport);
        }
    }
    let playback = value(layer, prefix, "Playback").and_then(Value::as_str);
    let behavior = match (loop_time && loop_count, playback) {
        (true, Some("Loop")) => None,
        (false, Some("Stop")) => Some(0),
        (false, Some("Loop")) if loop_time && !loop_count => Some(3),
        (false, Some("Loop")) if loop_count && !loop_time => Some(2),
        (false, Some("Loop")) => Some(1),
        (false, _) if loop_time && !loop_count => Some(3),
        (false, _) if loop_count && !loop_time => Some(2),
        _ => None,
    };
    if let Some(behavior) = behavior {
        put_varint(12, behavior, &mut transport);
    }
    transport
}

fn web_url<'a>(layer: &'a Value, prefix: &str) -> Option<&'a str> {
    if prefix == "media" {
        layer
            .get("webUrl")
            .and_then(Value::as_str)
            .or_else(|| value(layer, prefix, "Url").and_then(Value::as_str))
    } else {
        value(layer, prefix, "Url").and_then(Value::as_str)
    }
}

/// Returns the Media oneof field and type-properties bytes for image, video, audio or web media.
/// `prefix` is `media` for a layer or `mediaFill` for a text fill.
pub(super) fn type_properties(layer: &Value, prefix: &str) -> Option<(u32, Vec<u8>)> {
    let kind = kind(layer, prefix)?;
    let mut properties = Vec::new();
    match kind {
        "image" => {
            let drawing = drawing_properties(layer, prefix);
            if !drawing.is_empty() {
                put_message(1, drawing, &mut properties);
            }
            Some((5, properties))
        }
        "video" => {
            let drawing = drawing_properties(layer, prefix);
            if !drawing.is_empty() {
                put_message(1, drawing, &mut properties);
            }
            let audio = audio_properties(layer, prefix);
            if !audio.is_empty() {
                put_message(2, audio, &mut properties);
            }
            let transport = transport_properties(layer, prefix);
            if !transport.is_empty() {
                put_message(3, transport, &mut properties);
            }
            Some((6, properties))
        }
        "audio" => {
            let audio = audio_properties(layer, prefix);
            if !audio.is_empty() {
                put_message(1, audio, &mut properties);
            }
            let transport = transport_properties(layer, prefix);
            if !transport.is_empty() {
                put_message(2, transport, &mut properties);
            }
            Some((4, properties))
        }
        "web" => {
            let url = web_url(layer, prefix)?.trim();
            if url.is_empty() {
                return None;
            }
            let drawing = drawing_properties(layer, prefix);
            if !drawing.is_empty() {
                put_message(1, drawing, &mut properties);
            }
            let mut url_message = Vec::new();
            put_string(1, url, &mut url_message);
            put_message(2, url_message, &mut properties);
            Some((8, properties))
        }
        _ => None,
    }
}
