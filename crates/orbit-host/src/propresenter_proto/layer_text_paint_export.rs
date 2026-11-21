//! ProPresenter text paint fills supported by Orbit's native text renderer.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::*;

fn mapped_gradient_angle(angle: f64) -> f64 {
    (90.0 - angle).rem_euclid(360.0)
}

fn gradient_fill(style: &Value) -> Option<Vec<u8>> {
    let use_object_bounds = style
        .get("textGradientUseObjectBounds")
        .map(Value::as_bool)
        .unwrap_or(Some(true))?;
    let angle = match style.get("textGradientAngle") {
        Some(angle) => mapped_gradient_angle(angle.as_f64().filter(|angle| angle.is_finite())?),
        None => mapped_gradient_angle(0.0),
    };
    let colors = [
        style.get("textGradientStart")?.clone(),
        style.get("textGradientEnd")?.clone(),
    ];
    let gradient = super::layer_export::gradient_message(Some(angle), &colors)?;
    let mut fill = Vec::new();
    put_message(1, gradient, &mut fill);
    put_varint(2, u64::from(use_object_bounds), &mut fill);
    Some(fill)
}

fn media_layer(style: &Value) -> Option<Value> {
    let object = style.as_object()?;
    if !matches!(style["mediaFillType"].as_str(), Some("image" | "video"))
        || style["mediaFillUrl"].as_str()?.trim().is_empty()
        || !super::layer_media_export::unsupported_fields(style, "mediaFill").is_empty()
    {
        return None;
    }
    Some(Value::Object(
        object
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix("mediaFill")
                    .map(|suffix| (format!("media{suffix}"), value.clone()))
            })
            .collect(),
    ))
}

fn media_layer_for_loss_review(style: &Value) -> Option<Value> {
    if let Some(media) = media_layer(style) {
        return Some(media);
    }
    let source = style["mediaFillSourcePath"]
        .as_str()
        .filter(|source| !source.trim().is_empty())?;
    let mut review_style = style.clone();
    review_style["mediaFillUrl"] = Value::from(source);
    media_layer(&review_style)
}

fn media_fill(style: &Value, relative: &BTreeMap<String, String>) -> Option<Vec<u8>> {
    let media = super::layer_export::media_message(&media_layer(style)?, relative)?;
    let mut fill = Vec::new();
    put_message(1, media, &mut fill);
    Some(fill)
}

/// Returns the text-attribute oneof field and its message bytes for custom text fills.
/// Text color and background effects remain encoded by the existing text writer.
pub(super) fn text_fill_field(
    style: &Value,
    relative: &BTreeMap<String, String>,
) -> Option<(u32, Vec<u8>)> {
    match style.get("textFillMode")?.as_str()? {
        "Text Gradient" => Some((14, gradient_fill(style)?)),
        "Text Cut Out" => Some((16, Vec::new())),
        "Media Fill" => Some((17, media_fill(style, relative)?)),
        _ => None,
    }
}

/// ProPresenter 21 renders text fills from per-paragraph `CustomAttribute` copies of the fill;
/// `attribute_field` is the fill's number in the `CustomAttribute` oneof.
/// Ranges are inclusive UTF-16 offsets; every paragraph, including the last, counts a trailing
/// newline, and a zero start is omitted as in proto3.
pub(super) fn fill_custom_attributes(
    text: &str,
    attribute_field: u32,
    fill: &[u8],
) -> Vec<Vec<u8>> {
    let mut start = 0_u64;
    text.split('\n')
        .map(|paragraph| {
            let end = start + paragraph.encode_utf16().count() as u64;
            let mut range = Vec::new();
            if start > 0 {
                put_varint(1, start, &mut range);
            }
            put_varint(2, end, &mut range);
            let mut attribute = Vec::new();
            put_message(1, range, &mut attribute);
            put_message(attribute_field, fill.to_vec(), &mut attribute);
            start = end + 1;
            attribute
        })
        .collect()
}

/// A loss-report predicate for fields handled by this module. `None` leaves other fields to the
/// caller, which keeps default inactive paint controls from becoming export warnings.
pub(super) fn style_field_supported(style: &Value, key: &str) -> Option<bool> {
    let mode = style
        .get("textFillMode")
        .and_then(Value::as_str)
        .unwrap_or("");
    match key {
        "textFillMode" => match mode {
            "Text Gradient" => Some(gradient_fill(style).is_some()),
            "Text Cut Out" => Some(true),
            "Media Fill" => Some(media_layer_for_loss_review(style).is_some()),
            "Text Color" | "Background Blur" | "Background Invert" => None,
            _ => Some(false),
        },
        "textGradientStart"
        | "textGradientEnd"
        | "textGradientAngle"
        | "textGradientUseObjectBounds" => {
            Some(mode == "Text Gradient" && gradient_fill(style).is_some())
        }
        "mediaFillType"
        | "mediaFillUrl"
        | "mediaFillScale"
        | "mediaFillAlignment"
        | "mediaFillFlipX"
        | "mediaFillFlipY"
        | "mediaFillRotation"
        | "mediaFillCrop"
        | "mediaFillCropLeft"
        | "mediaFillCropRight"
        | "mediaFillCropTop"
        | "mediaFillCropBottom"
        | "mediaFillPlayback"
        | "mediaFillEffects"
        | "mediaFillRampIn"
        | "mediaFillRampOut"
        | "mediaFillLoopTime"
        | "mediaFillTimesToLoop"
        | "mediaFillHue"
        | "mediaFillSaturation"
        | "mediaFillBrightness"
        | "mediaFillContrast"
        | "mediaFillVolume"
        | "mediaFillAlphaType" => {
            Some(mode == "Media Fill" && media_layer_for_loss_review(style).is_some())
        }
        "mediaFillId" => Some(
            mode == "Media Fill"
                && media_layer_for_loss_review(style).is_some()
                && style.get(key).is_some_and(Value::is_string),
        ),
        "mediaFillSourcePath" => {
            Some(mode == "Media Fill" && style.get(key).is_some_and(Value::is_string))
        }
        "mediaFillTitle" => Some(false),
        _ => None,
    }
}

fn has_field(fields: &[Field], number: u32) -> bool {
    fields.iter().any(|field| field.number == number)
}

fn push_unsupported(unsupported: &mut Vec<String>, reason: impl Into<String>) {
    let reason = reason.into();
    if !unsupported.contains(&reason) {
        unsupported.push(reason);
    }
}

fn unsupported_fields(fields: &[Field], known: &[u32], name: &str, unsupported: &mut Vec<String>) {
    for field in fields {
        if !known.contains(&field.number) {
            push_unsupported(unsupported, format!("{name} field {}", field.number));
        }
    }
}

fn gradient_color(fields: &[Field], unsupported: &mut Vec<String>) -> Option<String> {
    for field in fields {
        if !(1..=4).contains(&field.number) {
            push_unsupported(
                unsupported,
                format!("text gradient stop color field {}", field.number),
            );
            continue;
        }
        let WireValue::Fixed32(bits) = &field.value else {
            push_unsupported(
                unsupported,
                format!(
                    "text gradient stop color channel {} has an unexpected wire type",
                    field.number
                ),
            );
            continue;
        };
        let value = f32::from_bits(*bits);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            push_unsupported(
                unsupported,
                format!(
                    "text gradient stop color channel {} is outside 0 to 1",
                    field.number
                ),
            );
        } else if ((value * 255.0).round() / 255.0 - value).abs() > f32::EPSILON {
            push_unsupported(
                unsupported,
                "text gradient stop color has precision beyond Orbit's 8-bit color format",
            );
        }
    }
    color(fields)
}

fn inspect_gradient_fill(
    fill: &[Field],
    style: &mut Map<String, Value>,
    unsupported: &mut Vec<String>,
) {
    unsupported_fields(fill, &[1, 2], "text gradient fill", unsupported);
    let Some(gradient) = nested_field(fill, 1) else {
        push_unsupported(unsupported, "text gradient data");
        return;
    };
    unsupported_fields(&gradient, &[1, 2, 3, 4], "text gradient", unsupported);
    if has_field(&gradient, 1) && varint_field(&gradient, 1) != Some(0) {
        push_unsupported(unsupported, "non-linear text gradient");
    }
    let angle = if has_field(&gradient, 2) {
        if let Some(angle) = f64_field(&gradient, 2).filter(|angle| angle.is_finite()) {
            Some(mapped_gradient_angle(angle))
        } else {
            push_unsupported(unsupported, "text gradient angle");
            None
        }
    } else {
        Some(90.0)
    };
    if has_field(&gradient, 3) {
        push_unsupported(unsupported, "text gradient length");
    }

    let stops = nested_fields(&gradient, 4).collect::<Vec<_>>();
    if stops.len() < 2 {
        push_unsupported(unsupported, "text gradient requires two color stops");
        return;
    }
    // ProPresenter blends only the first two stops and ignores stop positions and blend points.
    let mut colors = Vec::with_capacity(2);
    for stop in stops.iter().take(2) {
        unsupported_fields(stop, &[1, 2, 3], "text gradient stop", unsupported);
        let color = nested_field(stop, 1)
            .and_then(|color_fields| gradient_color(&color_fields, unsupported));
        let Some(color) = color else {
            push_unsupported(unsupported, "text gradient stop color");
            continue;
        };
        colors.push(color);
    }
    if colors.len() < 2 {
        return;
    }
    if let Some(angle) = angle {
        style.insert("textGradientAngle".into(), Value::from(angle));
    }
    style.insert(
        "textGradientUseObjectBounds".into(),
        Value::from(bool_field(fill, 2).unwrap_or(false)),
    );
    if has_field(fill, 2) && bool_field(fill, 2).is_none() {
        push_unsupported(unsupported, "text gradient object bounds flag");
    }
    style.insert("textFillMode".into(), Value::from("Text Gradient"));
    style.insert("textGradientStart".into(), Value::from(colors[0].clone()));
    style.insert("textGradientEnd".into(), Value::from(colors[1].clone()));
}

fn inspect_media(
    media_fill: &[Field],
    style: &mut Map<String, Value>,
    unsupported: &mut Vec<String>,
) {
    unsupported_fields(media_fill, &[1, 2], "text media fill", unsupported);
    if has_field(media_fill, 2) && bool_field(media_fill, 2) != Some(false) {
        push_unsupported(unsupported, "text media fill object bounds");
    }
    let Some(media) = nested_field(media_fill, 1) else {
        push_unsupported(unsupported, "text media fill media");
        return;
    };
    unsupported_fields(&media, &[1, 2, 3, 5, 6], "text fill media", unsupported);
    let media_kinds = [4, 5, 6, 7, 8]
        .into_iter()
        .filter(|number| has_field(&media, *number))
        .count();
    if media_kinds > 1 {
        push_unsupported(unsupported, "multiple text fill media types");
    }
    let kind = media_type(&media);
    if !matches!(kind.as_str(), "image" | "video") {
        push_unsupported(unsupported, format!("text media fill type {kind}"));
        return;
    }
    if decode_url_value(&media, 2).is_none() {
        push_unsupported(unsupported, "text media fill URL");
        return;
    }
    style.insert("textFillMode".into(), Value::from("Media Fill"));
    decode_media_style(&media, style, "mediaFill");

    let properties_field = if kind == "image" { 5 } else { 6 };
    if let Some(properties) = nested_field(&media, properties_field) {
        if kind == "image" {
            unsupported_fields(&properties, &[1], "text image properties", unsupported);
        } else {
            unsupported_fields(
                &properties,
                &[1, 2, 3, 4],
                "text video properties",
                unsupported,
            );
            if let Some(transport) = nested_field(&properties, 3) {
                unsupported_fields(
                    &transport,
                    &[7, 8, 9, 10, 12, 13, 14],
                    "text fill transport",
                    unsupported,
                );
                if has_field(&transport, 12)
                    && varint_field(&transport, 12).is_none_or(|value| value > 3)
                {
                    push_unsupported(unsupported, "text fill playback behavior");
                }
            }
            if has_field(&properties, 4) {
                push_unsupported(unsupported, "text fill video properties");
            }
        }
        if has_field(&properties, 1) && nested_field(&properties, 1).is_none() {
            push_unsupported(unsupported, "text fill drawing properties");
        }
        if kind == "video" && has_field(&properties, 3) && nested_field(&properties, 3).is_none() {
            push_unsupported(unsupported, "text fill transport properties");
        }
        if let Some(drawing) = nested_field(&properties, 1) {
            unsupported_fields(
                &drawing,
                &[1, 2, 3, 4, 6, 12, 13, 14, 15],
                "text fill drawing",
                unsupported,
            );
            if has_field(&drawing, 1) && varint_field(&drawing, 1).is_none_or(|value| value > 2) {
                push_unsupported(unsupported, "text fill scale behavior");
            }
            if has_field(&drawing, 2) && varint_field(&drawing, 2).is_none_or(|value| value > 8) {
                push_unsupported(unsupported, "text fill scale alignment");
            }
        }
    }
}

/// Decodes the supported `Graphics.Text.Attributes` paint oneof and lists schema data the Orbit
/// text style cannot represent. `style` is the same map returned by `decode_text_style`.
pub(super) fn decode_text_fill_style(
    attributes: &[Field],
    style: &mut Map<String, Value>,
    unsupported: &mut Vec<String>,
) {
    let fill_fields = attributes
        .iter()
        .filter(|field| matches!(field.number, 3 | 14 | 16 | 17 | 18))
        .count();
    if fill_fields > 1 {
        push_unsupported(unsupported, "multiple text fill oneof values");
    }
    if has_field(attributes, 14) {
        if let Some(fill) = nested_field(attributes, 14) {
            inspect_gradient_fill(&fill, style, unsupported);
        } else {
            push_unsupported(unsupported, "text gradient fill");
        }
    }
    if has_field(attributes, 16) {
        if let Some(fill) = nested_field(attributes, 16) {
            if !fill.is_empty() {
                unsupported_fields(&fill, &[], "text cut-out fill", unsupported);
            }
            style.insert("textFillMode".into(), Value::from("Text Cut Out"));
        } else {
            push_unsupported(unsupported, "text cut-out fill");
        }
    }
    if has_field(attributes, 17) {
        if let Some(fill) = nested_field(attributes, 17) {
            inspect_media(&fill, style, unsupported);
        } else {
            push_unsupported(unsupported, "text media fill");
        }
    }
}
