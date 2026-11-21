//! ProPresenter `Slide.Element` text settings that belong outside `Graphics.Text`.

use super::*;

fn direction(value: &str) -> Option<u64> {
    match value {
        "left" => Some(0),
        "right" => Some(1),
        "up" => Some(2),
        "down" => Some(3),
        _ => None,
    }
}

fn positive_dimension(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

fn axis_length(direction_value: &str, width: Option<f64>, height: Option<f64>) -> Option<f64> {
    match direction(direction_value)? {
        0 | 1 => positive_dimension(width),
        2 | 3 => positive_dimension(height),
        _ => None,
    }
}

fn bounds_axis(direction: &str, bounds: Option<&Value>) -> Option<f64> {
    axis_length(
        direction,
        bounds.and_then(|bounds| bounds["width"].as_f64()),
        bounds.and_then(|bounds| bounds["height"].as_f64()),
    )
}

fn style_axis(style: &Value, bounds: Option<&Value>) -> Option<f64> {
    bounds_axis(style["scrollDirection"].as_str().unwrap_or("left"), bounds)
}

fn fields_axis(fields: &[Field], bounds: Option<&Value>) -> Option<f64> {
    let direction = match last_varint(fields, 5) {
        None if !has_field(fields, 5) => "left",
        Some(0) => "left",
        Some(1) => "right",
        Some(2) => "up",
        Some(3) => "down",
        _ => return None,
    };
    bounds_axis(direction, bounds)
}

/// Encodes the text-scroller options with direct `TextScroller` equivalents.
pub(super) fn encode_text_scroller(
    style: &Value,
    bounds: Option<&Value>,
    slide_width: Option<f64>,
) -> Option<Vec<u8>> {
    let mut value = Vec::new();
    if let Some(enabled) = style["scrollingEnabled"].as_bool() {
        put_varint(1, u64::from(enabled), &mut value);
    }
    if let Some(speed) = style["scrollSpeed"]
        .as_f64()
        .filter(|speed| speed.is_finite() && (0.0..=100.0).contains(speed))
        .or_else(|| {
            (active_scrolling(style)
                && !super::super::import_export_report::present(&style["scrollSpeed"]))
            .then_some(50.0)
        })
    {
        if speed > 0.0 {
            put_f64(2, speed / 100.0, &mut value);
        }
    }
    if let Some(distance) = style["scrollRepeatDistance"]
        .as_f64()
        .filter(|distance| distance.is_finite() && *distance >= 0.0)
        .or_else(|| {
            (active_scrolling(style)
                && !super::super::import_export_report::present(&style["scrollRepeatDistance"]))
            .then_some(100.0)
        })
    {
        if let Some(axis) = style_axis(style, bounds) {
            put_f64(4, distance / axis, &mut value);
        }
    }
    if let Some(axis) = positive_dimension(slide_width) {
        for (field, key) in [(7, "scrollFeatherLeft"), (8, "scrollFeatherRight")] {
            if let Some(feather) = style[key]
                .as_f64()
                .filter(|feather| feather.is_finite() && (0.0..=axis * 0.5).contains(feather))
            {
                put_f64(field, feather / axis, &mut value);
            }
        }
    }
    if let Some(repeat) = style["scrollRepeat"].as_bool() {
        put_varint(3, u64::from(repeat), &mut value);
    }
    if let Some(number) = style["scrollDirection"].as_str().and_then(direction) {
        put_varint(5, number, &mut value);
    }
    if let Some(starts_off_screen) = match style["scrollStart"].as_str() {
        Some("Automatic") => Some(false),
        Some("Off Screen") => Some(true),
        _ => None,
    } {
        put_varint(6, u64::from(starts_off_screen), &mut value);
    }
    (!value.is_empty()).then_some(value)
}

fn has_field(fields: &[Field], number: u32) -> bool {
    fields.iter().any(|field| field.number == number)
}

fn last_varint(fields: &[Field], number: u32) -> Option<u64> {
    fields.iter().rev().find_map(|field| {
        if field.number != number {
            return None;
        }
        match &field.value {
            WireValue::Varint(value) => Some(*value),
            _ => None,
        }
    })
}

fn number_name(number: u32) -> &'static str {
    match number {
        1 => "should_scroll",
        2 => "scroll_rate",
        3 => "should_repeat",
        4 => "repeat_distance",
        5 => "scrolling_direction",
        6 => "starts_off_screen",
        7 => "fade_left",
        8 => "fade_right",
        _ => "unknown field",
    }
}

fn text_style_mut(layer: &mut Map<String, Value>) -> &mut Map<String, Value> {
    layer
        .entry("textStyle")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("Decoded text style is an object")
}

/// Decodes one `TextScroller`; recognized values that lack an exact Orbit equivalent are
/// recorded on the owning layer so the import review can disclose them.
pub(super) fn decode_text_scroller(
    fields: &[Field],
    layer: &mut Map<String, Value>,
    slide_width: Option<f64>,
) -> bool {
    if has_field(fields, 1) && last_varint(fields, 1).is_none() {
        note_unsupported(
            layer,
            vec!["text scroller should_scroll has an unexpected wire type".into()],
        );
        return true;
    }
    if !bool_field(fields, 1).unwrap_or(false)
        && layer
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| matches!(kind, "Audio" | "Live Video" | "Media" | "Video"))
    {
        return true;
    }
    let mut unsupported = Vec::new();
    for field in fields {
        if !(1..=8).contains(&field.number) {
            unsupported.push(format!(
                "text scroller {} ({}) is not represented by Orbit",
                number_name(field.number),
                field.number
            ));
        }
    }

    let object_axis = fields_axis(fields, layer.get("bounds"));
    let feather_axis = positive_dimension(slide_width);
    let style = text_style_mut(layer);
    style.insert(
        "scrollingEnabled".into(),
        json!(bool_field(fields, 1).unwrap_or(false)),
    );
    if has_field(fields, 3) {
        if let Some(value) = last_varint(fields, 3) {
            style.insert("scrollRepeat".into(), json!(value != 0));
        } else {
            unsupported.push(format!(
                "text scroller {} has an unexpected wire type",
                number_name(3)
            ));
        }
    }
    if has_field(fields, 5) {
        match last_varint(fields, 5) {
            Some(0) => {
                style.insert("scrollDirection".into(), json!("left"));
            }
            Some(1) => {
                style.insert("scrollDirection".into(), json!("right"));
            }
            Some(2) => {
                style.insert("scrollDirection".into(), json!("up"));
            }
            Some(3) => {
                style.insert("scrollDirection".into(), json!("down"));
            }
            Some(value) => unsupported.push(format!(
                "text scroller direction value {value} is not supported"
            )),
            None => unsupported
                .push("text scroller scrolling_direction has an unexpected wire type".into()),
        }
    }
    if has_field(fields, 6) {
        if let Some(value) = last_varint(fields, 6) {
            style.insert(
                "scrollStart".into(),
                json!(if value == 0 {
                    "Automatic"
                } else {
                    "Off Screen"
                }),
            );
        } else {
            unsupported.push("text scroller starts_off_screen has an unexpected wire type".into());
        }
    }

    if has_field(fields, 4) {
        match f64_field(fields, 4).filter(|distance| distance.is_finite() && *distance >= 0.0) {
            Some(distance) => match object_axis {
                Some(axis) => {
                    style.insert("scrollRepeatDistance".into(), json!(distance * axis));
                }
                None => unsupported.push(
                    "text scroller repeat_distance requires a positive text-bounds dimension on its scrolling axis".into(),
                ),
            }
            None => unsupported.push(
                "text scroller repeat_distance must be a finite non-negative normalized distance".into(),
            ),
        }
    }

    if has_field(fields, 2) {
        // ProPresenter stores 0-1. Probe files written by an earlier Orbit exporter store the
        // percentage itself, so a value above 1 up to 100 is read as that percentage.
        match f64_field(fields, 2)
            .filter(|speed| speed.is_finite() && (0.0..=100.0).contains(speed))
            .map(|speed| if speed > 1.0 { speed } else { speed * 100.0 })
        {
            Some(percent) => {
                style.insert("scrollSpeed".into(), json!(percent));
            }
            None => {
                unsupported
                    .push("text scroller scroll_rate must be a finite value from 0 to 100".into());
            }
        }
    } else {
        // An omitted proto3 scalar is zero; ProPresenter's UI uses the same 0–1 range.
        style.insert("scrollSpeed".into(), json!(0.0));
    }

    for (number, key) in [(7, "scrollFeatherLeft"), (8, "scrollFeatherRight")] {
        if !has_field(fields, number) {
            continue;
        }
        match f64_field(fields, number)
            .filter(|value| value.is_finite() && (0.0..=0.5).contains(value))
        {
            Some(value) if value == 0.0 => {
                style.insert(key.into(), json!(0.0));
            }
            Some(value) => match feather_axis {
                Some(axis) => {
                    style.insert(key.into(), json!(value * axis));
                }
                None => unsupported.push(format!(
                    "text scroller {} requires a positive slide width",
                    number_name(number)
                )),
            },
            None => unsupported.push(format!(
                "text scroller {} must be a finite normalized distance from 0 to 0.5",
                number_name(number)
            )),
        }
    }

    note_unsupported(layer, unsupported);
    true
}

fn active_scrolling(style: &Value) -> bool {
    style["scrollingEnabled"] == true
}

pub(super) fn valid_line_mask(style: &Value) -> bool {
    (style.get("linesOnlyEnabled").is_none() || style["linesOnlyEnabled"].is_boolean())
        && (style.get("linesOnlyStyle").is_none()
            || matches!(
                style["linesOnlyStyle"].as_str(),
                Some("Full Width" | "Line Width" | "Max Line Width")
            ))
        && [
            "linesOnlyHeightOffset",
            "linesOnlyVerticalOffset",
            "linesOnlyWidthOffset",
            "linesOnlyHorizontalOffset",
        ]
        .iter()
        .all(|key| style.get(*key).is_none() || style[*key].as_f64().is_some_and(f64::is_finite))
}

fn numeric_differs(value: &Value, default: f64) -> bool {
    match value.as_f64() {
        Some(number) => !number.is_finite() || number != default,
        None => super::super::import_export_report::present(value),
    }
}

pub(super) fn losses_with_context(
    layer: &Value,
    reference: &str,
    slide_width: Option<f64>,
    scroller_encoded: bool,
    findings: &mut Vec<String>,
) {
    let style = &layer["textStyle"];
    let mut lost = |key: &str, reason: &str| {
        findings.push(format!("{reference}/{key}: {reason}"));
    };

    if !valid_line_mask(style) {
        lost(
            "textStyle/linesOnly",
            "line mask settings must contain a supported style and finite offsets",
        );
    }

    if super::super::import_export_report::present(&style["lineTransform"])
        && !matches!(
            style["lineTransform"].as_str(),
            Some(
                "None"
                    | "Remove line returns"
                    | "Replace line returns"
                    | "One word per line"
                    | "One character per line"
            )
        )
    {
        lost(
            "textStyle/lineTransform",
            "the value is not a supported ProPresenter text transform",
        );
    }

    for (key, valid) in [
        ("scrollingEnabled", style["scrollingEnabled"].is_boolean()),
        ("scrollRepeat", style["scrollRepeat"].is_boolean()),
        (
            "scrollDirection",
            style["scrollDirection"]
                .as_str()
                .is_some_and(|value| direction(value).is_some()),
        ),
        (
            "scrollStart",
            matches!(
                style["scrollStart"].as_str(),
                Some("Automatic" | "Off Screen")
            ),
        ),
    ] {
        if super::super::import_export_report::present(&style[key]) && !valid {
            lost(
                &format!("textStyle/{key}"),
                "the value has no supported ProPresenter text scroller equivalent",
            );
        }
    }

    if super::super::import_export_report::present(&style["scrollSpeed"])
        && style["scrollSpeed"]
            .as_f64()
            .is_none_or(|speed| !speed.is_finite() || !(0.0..=100.0).contains(&speed))
    {
        lost(
            "textStyle/scrollSpeed",
            "scroll speed must be finite and between 0 and 100",
        );
    }
    if super::super::import_export_report::present(&style["scrollRepeatDistance"])
        && style["scrollRepeatDistance"]
            .as_f64()
            .is_none_or(|distance| !distance.is_finite() || distance < 0.0)
    {
        lost(
            "textStyle/scrollRepeatDistance",
            "repeat distance must be a finite non-negative pixel value",
        );
    }
    if scroller_encoded {
        if super::super::import_export_report::present(&style["scrollRepeatDistance"])
            || active_scrolling(style)
        {
            if style_axis(style, layer.get("bounds")).is_none() {
                lost(
                    "textStyle/scrollRepeatDistance",
                    "repeat distance requires a positive text-bounds dimension on its scrolling axis",
                );
            }
        }
        for key in ["scrollFeatherLeft", "scrollFeatherRight"] {
            if numeric_differs(&style[key], 0.0) {
                if style[key].as_f64().is_none_or(|feather| {
                    !feather.is_finite()
                        || feather < 0.0
                        || positive_dimension(slide_width).is_some_and(|axis| feather > axis * 0.5)
                }) {
                    lost(
                        &format!("textStyle/{key}"),
                        "feather distance must be finite and between 0 and half the slide width",
                    );
                } else if positive_dimension(slide_width).is_none() {
                    lost(
                        &format!("textStyle/{key}"),
                        "feather distance requires a positive slide width",
                    );
                }
            }
        }
        if style["scrollFeatherLinked"] == false {
            lost(
                "textStyle/scrollFeatherLinked",
                "the editor-only feather linking option has no ProPresenter text scroller field",
            );
        }
        if style["scrollLoopDelaySeconds"]
            .as_f64()
            .is_some_and(|delay| delay != 0.0)
        {
            lost(
                "textStyle/scrollLoopDelaySeconds",
                "ProPresenter TextScroller has no repeat-delay field",
            );
        }
    } else {
        if active_scrolling(style) {
            lost(
                "textStyle/scrollingEnabled",
                "the plain text export has no Slide.Element TextScroller",
            );
        }
        for (key, default) in [
            ("scrollSpeed", 50.0),
            ("scrollRepeatDistance", 100.0),
            ("scrollFeatherLeft", 0.0),
            ("scrollFeatherRight", 0.0),
        ] {
            if numeric_differs(&style[key], default) {
                lost(
                    &format!("textStyle/{key}"),
                    "the plain text export has no Slide.Element TextScroller",
                );
            }
        }
        for (key, changed) in [
            ("scrollRepeat", style["scrollRepeat"] == true),
            (
                "scrollDirection",
                style["scrollDirection"]
                    .as_str()
                    .is_some_and(|value| direction(value).is_some() && value != "left"),
            ),
            ("scrollStart", style["scrollStart"] == "Off Screen"),
            ("scrollFeatherLinked", style["scrollFeatherLinked"] == false),
            (
                "scrollLoopDelaySeconds",
                style["scrollLoopDelaySeconds"]
                    .as_f64()
                    .is_some_and(|delay| delay != 0.0),
            ),
        ] {
            if changed {
                lost(
                    &format!("textStyle/{key}"),
                    "the plain text export has no Slide.Element TextScroller",
                );
            }
        }
    }
}
