//! Theme authoring mutations ported from the renderer's field and object controllers.
use super::geometry::{self, Point};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicU64, Ordering};

pub const PARAGRAPH_FIELDS: &[&str] = &[
    "textHorizontalAlign",
    "textLineHeightMultiple",
    "textLineSpacing",
    "textMinimumLineHeight",
    "textMaximumLineHeight",
    "textParagraphSpacingBefore",
    "textParagraphSpacingAfter",
    "textFirstLineIndent",
    "textLeftIndent",
    "textRightIndent",
    "textAbsoluteRightEdge",
    "textDefaultTabInterval",
    "textTabStops",
    "textBarTabStops",
    "textListStyle",
    "textListLevel",
    "textListPrefix",
    "textListPostfix",
    "textListStart",
];
pub fn id(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{prefix}-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            // A clock before the epoch only weakens the timestamp; the counter keeps ids unique in-process.
            .unwrap_or_default()
            .as_millis(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
pub fn is_text(layer: &Value) -> bool {
    layer["type"]
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("text"))
        || matches!(layer["role"].as_str(), Some("primary" | "slide-text"))
}
pub fn style(slide: &Value, layer: Option<&Value>) -> Value {
    let mut result = defaults();
    for value in [
        slide.get("textStyle"),
        layer.and_then(|layer| layer.get("textStyle")),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(map) = value.as_object() {
            result.as_object_mut().unwrap().extend(map.clone());
        }
    }
    result
}
fn degrees(value: &Value) -> Result<Value, String> {
    let n = number(value)?;
    Ok(numeric(n.rem_euclid(360.)))
}
pub fn number(value: &Value) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| "Enter a finite number.".into())
}
pub fn numeric(value: f64) -> Value {
    if value.fract() == 0. && value.abs() <= 9_007_199_254_740_991. {
        json!(value as i64)
    } else {
        json!(value)
    }
}
fn validate_color(field: &str, value: &Value, prefix: &str) -> Result<(), String> {
    for (suffix, min, max) in [
        ("Hue", -180., 180.),
        ("Saturation", 0., 2.),
        ("Brightness", -1., 1.),
        ("Contrast", 0., 2.),
    ] {
        if field == format!("{prefix}{suffix}") {
            let n = number(value)?;
            if !(min..=max).contains(&n) {
                return Err("Color adjustment is out of range.".into());
            }
        }
    }
    Ok(())
}
fn set_default(target: &mut Value, key: &str, value: Value) {
    if target.get(key).is_none_or(Value::is_null) {
        target[key] = value;
    }
}
pub fn patch_text(
    slide: &Value,
    index: Option<usize>,
    field: &str,
    value: Value,
    range: Option<(usize, usize)>,
) -> Result<Value, String> {
    validate_color(field, &value, "mediaFill")?;
    if index.is_some_and(|index| slide["layers"].get(index).is_none()) {
        return Err("The object no longer exists.".into());
    }
    let mut next = slide.clone();
    let layer = index.and_then(|index| slide["layers"].get(index));
    let mut current = style(slide, layer);
    let field = if field == "fontWeight" { "bold" } else { field };
    let value = if field == "bold" && value.is_string() {
        json!(value == "Bold")
    } else if matches!(field, "textGradientAngle" | "shadowAngle") {
        degrees(&value)?
    } else if field == "superscript" {
        // The Choice control sends the level as text; the host reads an integer.
        let level = match &value {
            Value::String(text) => text
                .parse::<f64>()
                .map_err(|_| "Invalid superscript level.")?,
            other => number(other)?,
        };
        if level.fract() != 0. {
            return Err("Invalid superscript level.".into());
        }
        numeric(level)
    } else {
        value
    };
    let mut patch = Map::new();
    patch.insert(field.into(), value.clone());
    if field == "linkedTextSource" {
        patch.insert("linkedTextEnabled".into(), json!(value != "None"));
    }
    if field == "linkedTextEnabled" && value == true && current["linkedTextSource"] == "None" {
        patch.insert("linkedTextSource".into(), json!("Current Slide"));
    }
    if field == "linkedTextStageMessageFlash"
        && value == true
        && current["linkedTextStageMessageFlashColor"]
            .as_str()
            .is_none_or(str::is_empty)
    {
        patch.insert("linkedTextStageMessageFlashColor".into(), json!("#ffffff"));
    }
    if field == "scrollFeatherLeft" && current["scrollFeatherLinked"] != false {
        patch.insert("scrollFeatherRight".into(), value.clone());
    }
    if field == "scrollFeatherLinked" && value == true && current["scrollFeatherLinked"] == false {
        patch.insert(
            "scrollFeatherRight".into(),
            current["scrollFeatherLeft"].clone(),
        );
    }
    let count = paragraphs(
        layer
            .and_then(|layer| layer["text"].as_str())
            .or_else(|| slide["content"].as_str())
            .or_else(|| slide["text"].as_str())
            .unwrap_or(""),
    )
    .len();
    if PARAGRAPH_FIELDS.contains(&field) {
        if let Some((start, end)) = range {
            if start > end || end >= count || end >= 2000 {
                return Err("Invalid paragraph style range.".into());
            }
            if !current["textParagraphStyles"].is_array() {
                current["textParagraphStyles"] = json!([]);
            }
            let overrides = current["textParagraphStyles"].as_array_mut().unwrap();
            while overrides.len() < count {
                overrides.push(json!({}));
            }
            for item in &mut overrides[start..=end] {
                apply_paragraph(item, &patch);
            }
        } else {
            apply_paragraph(&mut current, &patch);
            if let Some(overrides) = current["textParagraphStyles"].as_array_mut() {
                for item in overrides.iter_mut() {
                    apply_paragraph(item, &patch);
                }
            }
        }
    } else {
        current.as_object_mut().unwrap().extend(patch);
    }
    if let Some(index) = index {
        next["layers"][index]["textStyle"] = current;
    } else {
        next["textStyle"] = current;
    }
    if field.starts_with("linkedTextRss") {
        let owner = if let Some(index) = index {
            &mut next["layers"][index]["textStyle"]
        } else {
            &mut next["textStyle"]
        };
        if owner["linkedTextRssItems"].is_string() {
            owner["linkedTextRssContent"] = json!(format_rss(owner)?);
        }
    }
    share_blur(&mut next, index, field == "backgroundBlurAmount");
    Ok(next)
}
/// `NoteParagraphControls`: a paragraph style override of the slide notes. The override list always spans the notes.
pub fn patch_notes_style(
    slide: &Value,
    field: &str,
    value: Value,
    paragraph: usize,
) -> Result<Value, String> {
    if !PARAGRAPH_FIELDS.contains(&field) {
        return Err("Unknown note paragraph field.".into());
    }
    let count = paragraphs(slide["notes"].as_str().unwrap_or("")).len();
    if paragraph >= count || count > 2000 {
        return Err("Invalid paragraph style range.".into());
    }
    let mut next = slide.clone();
    if !next["notesTextStyle"].is_object() {
        next["notesTextStyle"] = json!({});
    }
    let style = &mut next["notesTextStyle"];
    if !style["textParagraphStyles"].is_array() {
        style["textParagraphStyles"] = json!([]);
    }
    let overrides = style["textParagraphStyles"]
        .as_array_mut()
        .ok_or("Invalid paragraph styles.")?;
    while overrides.len() < count {
        overrides.push(json!({}));
    }
    let mut patch = Map::new();
    patch.insert(field.into(), value);
    apply_paragraph(&mut overrides[paragraph], &patch);
    Ok(next)
}
pub fn format_rss(style: &Value) -> Result<String, String> {
    let items: Vec<Value> = serde_json::from_str(
        style["linkedTextRssItems"]
            .as_str()
            .ok_or("RSS items must be serialized text.")?,
    )
    .map_err(|_| "The RSS item snapshot is invalid.")?;
    let title_only = style["linkedTextRssFormat"] == "title";
    let separate = style["linkedTextRssTitleSeparateLine"] != false;
    let delimiter = if style["linkedTextRssDelimiter"] == false {
        ""
    } else {
        style["linkedTextRssDelimiter"].as_str().unwrap_or("•")
    };
    let delimiter = if (title_only && separate)
        || (!title_only && style["linkedTextRssDescriptionSeparateLine"] != false)
    {
        format!("\n{delimiter}\n")
    } else {
        delimiter.to_string()
    };
    Ok(items
        .iter()
        .map(|item| {
            let title = item["title"].as_str().unwrap_or("");
            if title_only {
                title.to_string()
            } else {
                [title, item["description"].as_str().unwrap_or("")]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(if separate { "\n" } else { " " })
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(&delimiter))
}
fn apply_paragraph(target: &mut Value, patch: &Map<String, Value>) {
    if !target.is_object() {
        *target = json!({});
    }
    let fields = target.as_object_mut().unwrap();
    fields.extend(patch.clone());
    // An absolute right edge replaces the right inset and the other way round; a null removes the field.
    if patch
        .get("textAbsoluteRightEdge")
        .is_some_and(|v| !v.is_null())
    {
        fields.remove("textRightIndent");
    } else if patch.get("textRightIndent").is_some_and(|v| !v.is_null()) {
        fields.remove("textAbsoluteRightEdge");
    }
    fields.retain(|key, value| {
        !(matches!(key.as_str(), "textAbsoluteRightEdge" | "textRightIndent") && value.is_null())
    });
    if patch
        .get("textListStyle")
        .and_then(Value::as_str)
        .is_some_and(|s| {
            matches!(
                s,
                "none" | "box" | "check" | "diamond" | "disc" | "circle" | "square" | "hyphen"
            )
        })
    {
        for field in ["textListPrefix", "textListPostfix", "textListStart"] {
            target.as_object_mut().unwrap().remove(field);
        }
    }
}
pub fn patch_shape(
    slide: &Value,
    index: usize,
    field: &str,
    value: Value,
) -> Result<Value, String> {
    validate_color(field, &value, "media")?;
    if slide["layers"].get(index).is_none() {
        return Err("The object no longer exists.".into());
    }
    let mut slide = slide.clone();
    let layer = &mut slide["layers"][index];
    match field {
        "strokeWidth" | "strokeColor" | "strokeStyle" | "strokeEnabled" => {
            let width = stroke_width(layer);
            let color = stroke_color(layer);
            let stroke = layer["stroke"]
                .as_str()
                .filter(|stroke| !stroke.is_empty())
                .unwrap_or("1px #ffffff")
                .to_string();
            match field {
                "strokeWidth" => {
                    layer["stroke"] = json!(format!("{}px {color}", number(&value)?.max(0.)))
                }
                "strokeColor" => {
                    layer["stroke"] = json!(format!(
                        "{width}px {}",
                        value.as_str().ok_or("Choose a stroke color.")?
                    ))
                }
                "strokeStyle" => {
                    layer["strokeStyle"] = value.clone();
                    layer["stroke"] = json!(stroke);
                }
                _ => {
                    layer["stroke"] = json!(stroke);
                }
            }
            layer["strokeEnabled"] = if field == "strokeEnabled" {
                value.clone()
            } else {
                json!(true)
            };
        }
        "fillType" => fill_type(layer, value.as_str().ok_or("Choose a fill type.")?)?,
        "fillEnabled" => {
            if value == true {
                let fill = layer["fillType"]
                    .as_str()
                    .filter(|fill| *fill != "none")
                    .unwrap_or("color")
                    .to_string();
                if matches!(
                    fill.as_str(),
                    "slide-object" | "group-color" | "label-color" | "capture-status-color"
                ) {
                    fill_type(layer, &fill)?;
                } else {
                    layer["fillType"] = json!(fill);
                    layer["fill"] = json!(layer["fill"]
                        .as_str()
                        .filter(|fill| !fill.is_empty())
                        .or_else(|| layer["fillColors"][0]
                            .as_str()
                            .filter(|fill| !fill.is_empty()))
                        .unwrap_or("#ffffff"));
                    if layer["fillColors"].as_array().is_none_or(Vec::is_empty) {
                        layer["fillColors"] = json!([layer["fill"], "#14c8e8"]);
                    }
                }
            } else {
                layer["fillType"] = json!("none");
                layer["fill"] = json!("");
                layer["fillColors"] = json!([]);
            }
        }
        "fillColor" => {
            layer["fillType"] = json!("color");
            layer["fill"] = value.clone();
            layer["fillColors"] = json!([value]);
        }
        "fillStartColor" | "fillEndColor" => {
            let a = layer["fillColors"][0]
                .as_str()
                .unwrap_or("#0b63f6")
                .to_string();
            let b = layer["fillColors"][1]
                .as_str()
                .unwrap_or("#14c8e8")
                .to_string();
            layer["fillType"] = json!("gradient");
            layer["fill"] = json!("Gradient");
            layer["fillColors"] = if field == "fillStartColor" {
                json!([value, b])
            } else {
                json!([a, value])
            };
        }
        "fillAngle" => {
            layer["fillType"] = json!("gradient");
            layer["fill"] = json!("Gradient");
            layer["fillAngle"] = degrees(&value)?;
        }
        "mediaScale" => {
            layer[field] = if matches!(
                value.as_str(),
                Some("Scale to Fit" | "Scale to Fill" | "Stretch to Fill")
            ) {
                value
            } else {
                json!("Scale to Fit")
            }
        }
        "mediaPlayback" => {
            layer[field] = if matches!(value.as_str(), Some("Loop" | "Stop" | "Hold")) {
                value
            } else {
                json!("Loop")
            }
        }
        "dataFillSource" => {
            if !matches!(
                value.as_str(),
                Some("Current Slide" | "Next Slide" | "Next in Arrangement")
            ) {
                return Err("Invalid data fill source.".into());
            }
            layer[field] = value;
        }
        "mediaRotation" => layer[field] = degrees(&value)?,
        "opacity" | "mediaVolume" => layer[field] = numeric(number(&value)?.clamp(0., 100.)),
        "mediaCropTop" | "mediaCropRight" | "mediaCropBottom" | "mediaCropLeft" | "mediaRampIn"
        | "mediaRampOut" => layer[field] = numeric(number(&value)?.max(0.)),
        "shapeVisibilityEnabled" => layer["hidden"] = json!(value != true),
        "shapeShadowEnabled" => {
            layer["shadowEnabled"] = value;
            for (key, value) in [
                ("shadowBlur", json!(5)),
                ("shadowOpacity", json!(75)),
                ("shadowColor", json!("#000000")),
                ("shadowAngle", json!(315)),
                ("shadowOffset", json!(5)),
            ] {
                set_default(layer, key, value);
            }
        }
        "shapeFeatheringEnabled" => {
            layer["featheringEnabled"] = value;
            set_default(layer, "featheringAmount", json!(5));
        }
        "captureIdleColor"
        | "captureRecordingColor"
        | "captureFinalizingColor"
        | "captureErrorColor" => {
            let key = match field {
                "captureIdleColor" => "idle",
                "captureRecordingColor" => "recording",
                "captureFinalizingColor" => "finalizing",
                _ => "error",
            };
            if !layer["captureStatusColors"].is_object() {
                layer["captureStatusColors"] = json!({});
            }
            layer["captureStatusColors"][key] = value;
        }
        "featheringStyle" => {
            if !matches!(value.as_str(), Some("inside" | "center" | "outside")) {
                return Err("Invalid feathering style.".into());
            }
            layer[field] = value;
        }
        "backgroundEffect" => {
            if !matches!(value.as_str(), Some("blur" | "invert")) {
                return Err("Invalid background effect.".into());
            }
            layer[field] = value;
        }
        _ => layer[field] = value,
    }
    share_blur(&mut slide, Some(index), field == "backgroundBlurAmount");
    Ok(slide)
}
fn fill_type(layer: &mut Value, fill: &str) -> Result<(), String> {
    if let Some(screen) = fill.strip_prefix("screen-preview:") {
        if screen.is_empty() {
            return Err("Choose an output screen.".into());
        }
        layer["previewScreenId"] = json!(screen);
        return fill_type(layer, "screen-preview");
    }
    layer["fillType"] = json!(fill);
    if matches!(
        fill,
        "capture-status-color"
            | "slide-object"
            | "group-color"
            | "label-color"
            | "current-slide-image"
            | "next-slide-image"
            | "screen-preview"
            | "chord-chart"
            | "background-effect"
    ) {
        layer["fill"] = json!("");
        layer["fillColors"] = json!([]);
        if fill == "background-effect" {
            set_default(layer, "backgroundEffect", json!("blur"));
            set_default(layer, "backgroundBlurAmount", json!(14));
            set_default(layer, "backgroundBlurSaturation", json!(100));
        }
        if fill == "capture-status-color" {
            set_default(
                layer,
                "captureStatusColors",
                orbit_host::authoring::default_capture_status_colors(),
            );
        }
    } else {
        layer["fill"] = json!(if fill == "gradient" {
            "Gradient"
        } else if fill == "media" {
            "Media"
        } else {
            layer["fillColors"][0].as_str().unwrap_or("#ffffff")
        });
        if layer["fillColors"].as_array().is_none_or(Vec::is_empty) {
            layer["fillColors"] = json!(["#0b63f6", "#14c8e8"]);
        }
    }
    Ok(())
}
fn blur_amount(layer: &Value) -> Option<Value> {
    if layer["fillType"] == "background-effect" && layer["backgroundEffect"] == "blur" {
        layer.get("backgroundBlurAmount").cloned()
    } else if layer["textStyle"]["textFillMode"] == "Background Blur" {
        layer["textStyle"].get("backgroundBlurAmount").cloned()
    } else {
        None
    }
}
pub fn stroke_width(layer: &Value) -> f64 {
    layer["stroke"]
        .as_str()
        .and_then(|stroke| {
            let number: String = stroke
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            number.parse().ok()
        })
        .unwrap_or(1.)
}
pub fn stroke_color(layer: &Value) -> String {
    let stroke = layer["stroke"].as_str().unwrap_or("");
    stroke
        .char_indices()
        .find_map(|(i, c)| {
            (c == '#')
                .then(|| stroke.get(i..i + 7))
                .flatten()
                .filter(|s| s[1..].bytes().all(|c| c.is_ascii_hexdigit()))
        })
        .unwrap_or("#ffffff")
        .to_string()
}
fn share_blur(slide: &mut Value, index: Option<usize>, edited: bool) {
    let source = index
        .and_then(|index| blur_amount(&slide["layers"][index]))
        .or_else(|| {
            if index.is_none() && slide["textStyle"]["textFillMode"] == "Background Blur" {
                slide["textStyle"].get("backgroundBlurAmount").cloned()
            } else {
                None
            }
        });
    let Some(source) = source else {
        return;
    };
    let other = if index.is_some() && slide["textStyle"]["textFillMode"] == "Background Blur" {
        slide["textStyle"].get("backgroundBlurAmount").cloned()
    } else {
        None
    }
    .or_else(|| {
        slide["layers"].as_array().and_then(|layers| {
            layers.iter().enumerate().find_map(|(i, layer)| {
                if Some(i) != index {
                    blur_amount(layer)
                } else {
                    None
                }
            })
        })
    });
    let amount = if edited {
        source
    } else {
        other.unwrap_or(source)
    };
    if slide["textStyle"]["textFillMode"] == "Background Blur" {
        slide["textStyle"]["backgroundBlurAmount"] = amount.clone();
    }
    if let Some(layers) = slide["layers"].as_array_mut() {
        for layer in layers {
            if layer["fillType"] == "background-effect" && layer["backgroundEffect"] == "blur" {
                layer["backgroundBlurAmount"] = amount.clone();
            }
            if layer["textStyle"]["textFillMode"] == "Background Blur" {
                layer["textStyle"]["backgroundBlurAmount"] = amount.clone();
            }
        }
    }
}
pub fn new_layer(slide: &Value, shape: &str) -> Value {
    let size = geometry::slide_size(slide);
    let text = shape == "text";
    let (w, h) = if text { (0.46, 0.2) } else { (0.24, 0.24) };
    let width = geometry::round(size.x * w);
    let height = geometry::round(size.y * h);
    let mut layer = json!({"id":id(if text{"text"}else{"shape"}),"name":if text{"Text Box"}else{match shape{"rounded-rect"=>"Rounded Rect","rectangle"=>"Rectangle","ellipse"=>"Ellipse","triangle"=>"Triangle","right-triangle"=>"Right Triangle","rhombus"=>"Rhombus","polygon"=>"Polygon","star"=>"Star","double-arrow"=>"Double Arrow","arrow"=>"Arrow",other=>other}},"type":if text{"Text"}else{"Shape"},"role":if text{"custom"}else{"decoration"},"hidden":false,"bounds":{"x":geometry::round((size.x-width)/2.) as i64,"y":geometry::round((size.y-height)/2.) as i64,"width":width as i64,"height":height as i64},"fill":if text{""}else{"#ffffff"},"fillType":if text{"none"}else{"color"},"fillColors":if text{json!([])}else{json!(["#ffffff"])}});
    if text {
        let mut style = defaults();
        style["fontSize"] = json!(geometry::round(size.y * 0.075) as i64);
        layer["textStyle"] = style;
    } else {
        layer["shape"] = json!(shape);
        layer["stroke"] = json!("1px #ffffff");
        layer["strokeStyle"] = json!("solid");
        layer["opacity"] = json!(100);
    }
    layer
}
pub fn pen_layer(slide: &Value, points: &[Point], closed: bool) -> Result<Value, String> {
    if points.len() < 2 {
        return Err("A path needs at least two points.".into());
    }
    let size = geometry::slide_size(slide);
    let padding = geometry::round(size.x.min(size.y) * 0.01).max(12.);
    let min = Point::new(
        points
            .iter()
            .map(|p| p.x.clamp(0., size.x))
            .fold(f32::INFINITY, f32::min),
        points
            .iter()
            .map(|p| p.y.clamp(0., size.y))
            .fold(f32::INFINITY, f32::min),
    );
    let max = Point::new(
        points
            .iter()
            .map(|p| p.x.clamp(0., size.x))
            .fold(f32::NEG_INFINITY, f32::max),
        points
            .iter()
            .map(|p| p.y.clamp(0., size.y))
            .fold(f32::NEG_INFINITY, f32::max),
    );
    let x = geometry::round(min.x - padding).max(0.);
    let y = geometry::round(min.y - padding).max(0.);
    let width = geometry::round((max.x + padding).min(size.x) - (min.x - padding).max(0.)).max(8.);
    let height = geometry::round((max.y + padding).min(size.y) - (min.y - padding).max(0.)).max(8.);
    let mut layer = new_layer(slide, if closed { "freeform" } else { "stroke" });
    layer["id"] = json!(id("pen"));
    layer["name"] = json!(if closed { "Pen Shape" } else { "Pen Stroke" });
    layer["bounds"] =
        json!({"x":x as i64,"y":y as i64,"width":width as i64,"height":height as i64});
    layer["pathCoordinateSpace"] = json!("local");
    layer["closedPath"] = json!(closed);
    layer["pathPoints"]=Value::Array(points.iter().map(|p|json!({"x":geometry::round(p.x-x).max(0.) as i64,"y":geometry::round(p.y-y).max(0.) as i64})).collect());
    layer["stroke"] = json!("3px #ffffff");
    if !closed {
        layer["fill"] = json!("");
        layer["fillType"] = json!("none");
        layer["fillColors"] = json!([]);
    }
    Ok(layer)
}

pub const DEFAULT_LAYER_COLOR: &str = "#ffffff";
pub const DEFAULT_SLIDE_BACKGROUND: &str = "#202020";

pub fn defaults() -> Value {
    json!({
        "fontFamily": "Arial",
        "fontSize": 90,
        "bold": true,
        "italic": false,
        "underline": false,
        "strikethrough": false,
        "capitalization": "None",
        "scaling": "None",
        "lineTransform": "None",
        "textHorizontalAlign": "center",
        "textVerticalAlign": "middle",
        "textFillMode": "Text Color",
        "textColor": "#ffffff",
        "textGradientStart": "#0b63f6",
        "textGradientEnd": "#14c8e8",
        "textGradientAngle": 4,
        "textGradientUseObjectBounds": true,
        "mediaFillId": "",
        "mediaFillUrl": "",
        "mediaFillSourcePath": "",
        "mediaFillTitle": "",
        "mediaFillType": "",
        "mediaFillScale": "Scale to Fill",
        "mediaFillAlignment": "Middle Center",
        "mediaFillAlphaType": "Straight",
        "mediaFillCropTop": 0,
        "mediaFillCropRight": 0,
        "mediaFillCropBottom": 0,
        "mediaFillCropLeft": 0,
        "mediaFillPlayback": "Loop",
        "mediaFillVolume": 100,
        "mediaFillRampIn": 0,
        "mediaFillRampOut": 0,
        "backgroundBlurSaturation": 100,
        "backgroundBlurAmount": 14,
        "strokeEnabled": false,
        "strokeColor": "#000000",
        "strokeWidth": 2,
        "shadowEnabled": false,
        "shadowColor": "#000000",
        "shadowOpacity": 75,
        "shadowAngle": 315,
        "shadowOffset": 5,
        "shadowBlur": 5,
        "linesOnlyEnabled": false,
        "linesOnlyPadding": 0,
        "scrollingEnabled": false,
        "scrollDirection": "left",
        "scrollStart": "Automatic",
        "scrollSpeed": 50,
        "scrollRepeat": true,
        "scrollRepeatDistance": 100,
        "scrollFeatherLeft": 0,
        "scrollFeatherRight": 0,
        "scrollFeatherLinked": true,
        "linkedTextSource": "None",
        "linkedTextRssDescriptionSeparateLine": true,
        "linkedTextRssTitleSeparateLine": true,
        "systemClockDateFormat": "numeric",
        "systemClockTimeFormat": "hours-minutes",
        "systemClockUse24Hour": false,
    })
}

pub fn paragraphs(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n")
        .replace("\n\r", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(str::to_string)
        .collect()
}
pub fn display_text(text: &str, style: &Value) -> String {
    let capitalization = style["capitalization"].as_str().unwrap_or("None");
    let text = match capitalization {
        "All Caps" => text.to_uppercase(),
        "Start Case" | "Title Case" => {
            let text = if capitalization == "Title Case" {
                text.to_lowercase()
            } else {
                text.to_string()
            };
            let mut previous_word = false;
            let mut result = String::new();
            for c in text.chars() {
                let word = c.is_ascii_alphanumeric() || c == '_';
                if !previous_word && c.is_alphabetic() {
                    result.extend(c.to_uppercase());
                } else {
                    result.push(c);
                }
                previous_word = word;
            }
            result
        }
        _ => text.to_string(),
    };
    match style["lineTransform"].as_str().unwrap_or("None") {
        "Remove line returns" => paragraphs(&text).join(""),
        "Replace line returns" => {
            let mut out = String::new();
            let mut newline = false;
            for c in text.chars() {
                if c == '\r' || c == '\n' {
                    if !newline {
                        out.push(' ');
                    }
                    newline = true;
                } else {
                    newline = false;
                    out.push(c);
                }
            }
            out
        }
        "One word per line" => text.split_whitespace().collect::<Vec<_>>().join("\n"),
        "One character per line" => text
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => text,
    }
}
