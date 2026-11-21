//! Layered slide export. Every writer mirrors the field mapping of `decode_graphics_element`, so
//! a layer listed in `LAYER_KEYS` comes back from the importer as the same Orbit layer.

use super::*;

/// Layer fields the exporter writes. `role`, `type`, `fill` strings of gradient/media fills and
/// the media bookkeeping ids are derived or identify a file that `mediaUrl` already carries.
const LAYER_KEYS: &[&str] = &[
    "id",
    "name",
    "type",
    "role",
    "hidden",
    "locked",
    "rotation",
    "flipX",
    "flipY",
    "opacity",
    "bounds",
    "shape",
    "pathPoints",
    "pathCoordinateSpace",
    "closedPath",
    "shapeSettings",
    "fill",
    "fillType",
    "fillColors",
    "fillAngle",
    "fillEnabled",
    "strokeEnabled",
    "strokeWidth",
    "strokeColor",
    "strokeStyle",
    "shadowEnabled",
    "shadowAngle",
    "shadowOffset",
    "shadowBlur",
    "shadowOpacity",
    "shadowColor",
    "text",
    "textRuns",
    "textStyle",
    "usePresentationText",
    "mediaType",
    "mediaUrl",
    "mediaSourcePath",
    "mediaFlipX",
    "mediaFlipY",
    "mediaRotation",
    "mediaScale",
    "mediaAlignment",
    "mediaCrop",
    "mediaCropLeft",
    "mediaCropRight",
    "mediaCropTop",
    "mediaCropBottom",
    "mediaPlayback",
    "mediaId",
    "mediaTitle",
    "mediaPath",
    "build",
    "buildOut",
    "textReveal",
    "backgroundEffect",
    "backgroundBlurAmount",
    "backgroundBlurSaturation",
    "sourcePath",
    "sourceUrl",
    "previewScreenId",
    "fillObjectId",
    "dataFillSource",
    "fillTitle",
    "featheringEnabled",
    "featheringAmount",
    "featheringStyle",
    "webUrl",
    "mediaEffects",
    "mediaRampIn",
    "mediaRampOut",
    "mediaLoopTime",
    "mediaTimesToLoop",
    "mediaHue",
    "mediaSaturation",
    "mediaBrightness",
    "mediaContrast",
    "mediaVolume",
    "mediaAlphaType",
];

const TEXT_STYLE_KEYS: &[&str] = &[
    "fontFamily",
    "fontFace",
    "fontSize",
    "letterSpacing",
    "superscript",
    "superscriptStandardized",
    "italic",
    "bold",
    "textColor",
    "strokeColor",
    "strokeWidth",
    "strokeEnabled",
    "underline",
    "strikethrough",
    "underlineStyle",
    "underlinePattern",
    "underlineByWord",
    "underlineColor",
    "strikethroughStyle",
    "strikethroughPattern",
    "strikethroughByWord",
    "strikethroughColor",
    "textHorizontalAlign",
    "textVerticalAlign",
    "capitalization",
    "scaling",
    "shadowEnabled",
    "shadowColor",
    "shadowOpacity",
    "shadowAngle",
    "shadowOffset",
    "shadowBlur",
    "textFillMode",
    "backgroundBlurAmount",
    "backgroundBlurSaturation",
    "mediaFillSourcePath",
    "mediaFillId",
    "mediaFillTitle",
    "textGradientStart",
    "textGradientEnd",
    "textGradientAngle",
    "mediaFillType",
    "mediaFillUrl",
    "mediaFillScale",
    "mediaFillAlignment",
    "mediaFillFlipX",
    "mediaFillFlipY",
    "mediaFillRotation",
    "mediaFillCrop",
    "mediaFillCropLeft",
    "mediaFillCropRight",
    "mediaFillCropTop",
    "mediaFillCropBottom",
    "mediaFillEffects",
    "mediaFillRampIn",
    "mediaFillRampOut",
    "mediaFillLoopTime",
    "mediaFillTimesToLoop",
    "mediaFillHue",
    "mediaFillSaturation",
    "mediaFillBrightness",
    "mediaFillContrast",
    "mediaFillVolume",
    "mediaFillAlphaType",
    "mediaFillPlayback",
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
    "textMarginLeft",
    "textMarginRight",
    "textMarginTop",
    "textMarginBottom",
    "textDefaultTabInterval",
    "textTabStops",
    "textBarTabStops",
    "textListStyle",
    "textListLevel",
    "textListPrefix",
    "textListPostfix",
    "textListStart",
    "textParagraphStyles",
    "linkedTextEnabled",
    "linkedTextSource",
    "linkedTextObjectId",
    "linkedTextElementName",
    "linkedTextGroupSource",
    "linkedTextSlideCountSource",
    "linkedTextPlaylistItemSource",
    "linkedTextPresentationSource",
    "linkedTextPlanningCenterLiveSource",
    "linkedTextFilePath",
    "linkedTextRssUrl",
    "linkedTextRssFormat",
    "linkedTextRssDelimiter",
    "linkedTextRssTitleSeparateLine",
    "linkedTextRssDescriptionSeparateLine",
    "linkedTextTimerId",
    "linkedTextTimerName",
    "linkedTextTimerFormat",
    "linkedTextTimerFormatString",
    "linkedTextTimerColorTriggers",
    "linkedTextTickerContent",
    "lineTransform",
    "lineTransformDelimiter",
    "systemClockFormatString",
    "videoCountdownSource",
    "videoCountdownIgnoreLooping",
    "systemClockDateFormat",
    "systemClockTimeFormat",
    "systemClockUse24Hour",
    "scrollingEnabled",
    "scrollDirection",
    "scrollStart",
    "scrollSpeed",
    "scrollRepeat",
    "scrollRepeatDistance",
    "scrollFeatherLeft",
    "scrollFeatherRight",
    "scrollFeatherLinked",
    "scrollLoopDelaySeconds",
];

const VERTICAL_ALIGNMENTS: [&str; 3] = ["top", "middle", "bottom"];
const STROKE_STYLES: [&str; 4] = ["solid", "square", "short", "long"];

pub(super) fn is_color(value: &str) -> bool {
    crate::native_compositor::parse_color(value).is_some()
}

fn put_layer_color(number: u32, value: &str, output: &mut Vec<u8>) {
    put_color(number, value, output);
}

fn capitalization_number(value: &str) -> Option<u64> {
    match value {
        "None" => Some(0),
        "All Caps" => Some(1),
        "Small Caps" => Some(2),
        "Title Case" => Some(3),
        "Start Case" => Some(4),
        _ => None,
    }
}

fn scaling_number(value: &str) -> Option<u64> {
    match value {
        "None" => Some(0),
        "Container up or down" => Some(1),
        "Text down" => Some(2),
        "Text up" => Some(3),
        "Text up or down" => Some(4),
        _ => None,
    }
}

fn background_effect_message(
    kind: &str,
    amount: Option<f64>,
    saturation: Option<f64>,
) -> Option<Vec<u8>> {
    let mut effect = Vec::new();
    match kind {
        "invert" => put_message(3, Vec::new(), &mut effect),
        "blur" => {
            let (Some(amount), Some(saturation)) = (amount, saturation) else {
                return None;
            };
            if !(0.0..=40.0).contains(&amount) || !(0.0..=200.0).contains(&saturation) {
                return None;
            }
            let mut blur = Vec::new();
            put_f64(1, saturation / 100.0 - 1.0, &mut blur);
            put_f64(2, amount / 40.0, &mut blur);
            put_message(2, blur, &mut effect);
        }
        _ => return None,
    }
    Some(effect)
}

fn text_background_effect_message(style: &Value) -> Option<Vec<u8>> {
    let kind = match style["textFillMode"].as_str()? {
        "Background Blur" => "blur",
        "Background Invert" => "invert",
        _ => return None,
    };
    background_effect_message(
        kind,
        style["backgroundBlurAmount"].as_f64(),
        style["backgroundBlurSaturation"].as_f64(),
    )
}

/// The `Graphics.Text` message: RTF body plus the base font, color and paragraph attributes.
/// `style` carries the character formatting and `paragraph_style` the paragraph formatting; a plain
/// text slide takes only the paragraphs of its composed primary layer and keeps the portable font.
pub(super) fn text_value(
    text: &str,
    runs: &[Value],
    style: &Value,
    paragraph_style: &Map<String, Value>,
    default_color: &str,
    relative: &BTreeMap<String, String>,
    text_width: Option<f64>,
) -> Vec<u8> {
    let family = style["fontFamily"].as_str().unwrap_or("Arial");
    let size = style["fontSize"].as_f64().unwrap_or(72.0);
    let color = style["textColor"]
        .as_str()
        .filter(|value| is_color(value))
        .unwrap_or(default_color);
    let vertical = VERTICAL_ALIGNMENTS
        .iter()
        .position(|name| style["textVerticalAlign"] == *name)
        .unwrap_or(1);

    let mut font = Vec::new();
    put_string(1, family, &mut font);
    put_f64(2, size, &mut font);
    for (field, key) in [(4, "italic"), (8, "bold")] {
        if let Some(on) = style[key].as_bool() {
            put_varint(field, u64::from(on), &mut font);
        }
    }
    put_string(9, family, &mut font);
    if let Some(face) = style["fontFace"].as_str() {
        put_string(10, face, &mut font);
    }
    let custom_fill = layer_text_paint_export::text_fill_field(style, relative);
    let background_fill = text_background_effect_message(style);
    let mut attributes = Vec::new();
    put_message(1, font, &mut attributes);
    if custom_fill.is_none() && background_fill.is_none() {
        put_layer_color(3, color, &mut attributes);
    }
    let line = |key: &str| {
        style[key].as_bool().map(|on| {
            let mut line = Vec::new();
            let variant = if on {
                match style[format!("{key}Style")].as_str() {
                    Some("thick") => 2,
                    Some("double") => 3,
                    _ => 1,
                }
            } else {
                0
            };
            let pattern = match style[format!("{key}Pattern")].as_str() {
                Some("dot") => 1,
                Some("dash") => 2,
                Some("dash-dot") => 3,
                Some("dash-dot-dot") => 4,
                _ => 0,
            };
            put_varint(1, variant, &mut line);
            put_varint(2, pattern, &mut line);
            if let Some(by_word) = style[format!("{key}ByWord")].as_bool() {
                put_varint(3, u64::from(by_word), &mut line);
            }
            line
        })
    };
    if let Some(line) = line("underline") {
        put_message(4, line, &mut attributes);
    }
    if let Some(color) = style["underlineColor"].as_str() {
        put_layer_color(5, color, &mut attributes);
    }
    if let Some(value) = style["superscript"]
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
    {
        put_varint(8, value as i64 as u64, &mut attributes);
    }
    if let Some(value) = style["letterSpacing"]
        .as_f64()
        .filter(|value| value.is_finite())
    {
        put_f64(7, value, &mut attributes);
    }
    let (paragraph, _) = encode_paragraph_style(paragraph_style);
    put_message(6, paragraph, &mut attributes);
    if let Some(line) = line("strikethrough") {
        put_message(9, line, &mut attributes);
    }
    if let Some(color) = style["strikethroughColor"].as_str() {
        put_layer_color(10, color, &mut attributes);
    }
    if style["strokeEnabled"] == true {
        put_f64(
            11,
            style["strokeWidth"].as_f64().unwrap_or(2.0),
            &mut attributes,
        );
    }
    // Without a stroke width ProPresenter keeps the color but draws no stroke, as imported.
    if let Some(stroke) = style["strokeColor"]
        .as_str()
        .filter(|value| is_color(value))
    {
        put_layer_color(12, stroke, &mut attributes);
    }
    if let Some(capitalization) = style["capitalization"]
        .as_str()
        .and_then(capitalization_number)
    {
        put_varint(2, capitalization, &mut attributes);
    }
    let paragraph_fill = match &custom_fill {
        Some((14, fill)) => Some((5, fill.as_slice())),
        Some((16, fill)) => Some((8, fill.as_slice())),
        Some((17, fill)) => Some((9, fill.as_slice())),
        _ => background_fill.as_deref().map(|effect| (10, effect)),
    };
    if let Some((field, fill)) = paragraph_fill {
        for attribute in layer_text_paint_export::fill_custom_attributes(text, field, fill) {
            put_message(13, attribute, &mut attributes);
        }
    }
    if let Some(effect) = background_fill {
        put_message(18, effect, &mut attributes);
    }
    if let Some((field, fill)) = custom_fill {
        put_message(field, fill, &mut attributes);
    }
    let mut value = Vec::new();
    let linked_transform = style["lineTransform"] != "Replace line returns"
        && style["linkedTextEnabled"] != false
        && matches!(
            style["linkedTextSource"].as_str(),
            Some("Object" | "Current Slide" | "Next Slide" | "Slide Notes" | "Next Slide Notes")
        );
    if let Some(transform) = style["lineTransform"]
        .as_str()
        .filter(|_| !linked_transform)
        .and_then(|value| match value {
            "None" => Some(0),
            "Remove line returns" => Some(1),
            "One word per line" => Some(2),
            "One character per line" => Some(3),
            "Replace line returns" => Some(4),
            _ => None,
        })
    {
        put_varint(10, transform, &mut value);
    }
    if let Some(delimiter) = style["lineTransformDelimiter"]
        .as_str()
        .or_else(|| (style["lineTransform"] == "Replace line returns").then_some(" "))
    {
        put_bytes(11, delimiter.as_bytes(), &mut value);
    }
    put_message(3, attributes, &mut value);
    if let Some(shadow) = shadow_message(style) {
        put_message(4, shadow, &mut value);
    }
    if let Some(scaling) = style["scaling"].as_str().and_then(scaling_number) {
        put_varint(7, scaling, &mut value);
    }
    let margins = [
        (1, "textMarginLeft"),
        (2, "textMarginRight"),
        (3, "textMarginTop"),
        (4, "textMarginBottom"),
    ];
    if margins.iter().any(|(_, key)| style.get(*key).is_some()) {
        let mut insets = Vec::new();
        for (field, key) in margins {
            if let Some(amount) = style[key]
                .as_f64()
                .filter(|amount| amount.is_finite() && *amount >= 0.0)
            {
                put_f64(field, amount, &mut insets);
            }
        }
        put_message(8, insets, &mut value);
    }
    let (run_fonts, run_colors, paragraph_tables, body) =
        rtf_body(text, runs, paragraph_style, text_width);
    let rgb = crate::native_compositor::parse_color(color)
        .map(|[b, g, r, _]| u32::from_be_bytes([0, r, g, b]))
        .unwrap_or(0);
    put_bytes(
        5,
        format!(
            "{{\\rtf1\\ansi\\uc1{{\\fonttbl{{\\f0 {};}}{run_fonts}}}{{\\colortbl;\\red{}\\green{}\\blue{};{run_colors}}}{paragraph_tables}\\f0\\fs{}\\cf1 {body}}}",
            escape_rtf(family),
            rgb >> 16,
            (rgb >> 8) & 255,
            rgb & 255,
            (size * 2.0).round().max(1.0)
        )
        .as_bytes(),
        &mut value,
    );
    if let Some(standardized) = style["superscriptStandardized"].as_bool() {
        put_varint(9, u64::from(standardized), &mut value);
    }
    put_varint(6, vertical as u64, &mut value);
    value
}

fn pair(first: Option<f64>, second: Option<f64>) -> Option<Vec<u8>> {
    if first.is_none() && second.is_none() {
        return None;
    }
    let mut message = Vec::new();
    for (field, value) in [(1, first), (2, second)] {
        if let Some(value) = value {
            put_f64(field, value, &mut message);
        }
    }
    Some(message)
}

fn bounds_message(bounds: &Value) -> Option<Vec<u8>> {
    let mut message = Vec::new();
    if let Some(origin) = pair(bounds["x"].as_f64(), bounds["y"].as_f64()) {
        put_message(1, origin, &mut message);
    }
    if let Some(size) = pair(bounds["width"].as_f64(), bounds["height"].as_f64()) {
        put_message(2, size, &mut message);
    }
    (!message.is_empty()).then_some(message)
}

fn shape_number(layer: &Value) -> Option<u64> {
    let name = layer["shape"].as_str()?;
    if matches!(name, "freeform" | "stroke") && path_points(layer).is_some() {
        return Some(8);
    }
    (1..=11).find(|number| shape_name(Some(*number)) == Some(name))
}

#[derive(Clone, Copy)]
struct PathPoint {
    position: (f64, f64),
    q0: Option<(f64, f64)>,
    q1: Option<(f64, f64)>,
    curved: Option<bool>,
}

fn point_pair(value: &Value) -> Option<(f64, f64)> {
    let x = value["x"].as_f64()?;
    let y = value["y"].as_f64()?;
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

fn path_bounds(layer: &Value) -> Option<((f64, f64), f64, f64)> {
    match layer["pathCoordinateSpace"].as_str()? {
        "slide" | "local" => (),
        _ => return None,
    }
    let origin = (
        layer["bounds"]["x"].as_f64()?,
        layer["bounds"]["y"].as_f64()?,
    );
    let width = layer["bounds"]["width"].as_f64()?;
    let height = layer["bounds"]["height"].as_f64()?;
    (origin.0.is_finite()
        && origin.1.is_finite()
        && width.is_finite()
        && height.is_finite()
        && width > 0.0
        && height > 0.0)
        .then_some((origin, width, height))
}

fn path_points(layer: &Value) -> Option<Vec<PathPoint>> {
    let points = layer["pathPoints"]
        .as_array()
        .filter(|points| !points.is_empty())?;
    let local = match layer["pathCoordinateSpace"].as_str()? {
        "slide" => false,
        "local" => true,
        _ => return None,
    };
    let ((origin_x, origin_y), width, height) = path_bounds(layer)?;
    let points = points
        .iter()
        .map(|point| {
            let (x, y) = point_pair(point)?;
            let q0 = match point.get("q0").filter(|value| !value.is_null()) {
                Some(value) => Some(point_pair(value)?),
                None => None,
            };
            let q1 = match point.get("q1").filter(|value| !value.is_null()) {
                Some(value) => Some(point_pair(value)?),
                None => None,
            };
            let curved = match point.get("curved").filter(|value| !value.is_null()) {
                Some(value) => Some(value.as_bool()?),
                None => None,
            };
            if curved == Some(true) && (q0.is_none() || q1.is_none()) {
                return None;
            }
            let normalized = |coordinates: (f64, f64)| {
                let (x, y) = if local {
                    coordinates
                } else {
                    (coordinates.0 - origin_x, coordinates.1 - origin_y)
                };
                let normalized = (x / width, y / height);
                (normalized.0.is_finite() && normalized.1.is_finite()).then_some(normalized)
            };
            let position = normalized((x, y))?;
            let q0 = match q0 {
                Some(coordinates) => Some(normalized(coordinates)?),
                None => None,
            };
            let q1 = match q1 {
                Some(coordinates) => Some(normalized(coordinates)?),
                None => None,
            };
            Some(PathPoint {
                position,
                q0,
                q1,
                curved,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let has_curve =
        |start: &PathPoint, end: &PathPoint| start.curved == Some(true) || end.curved == Some(true);
    for pair in points.windows(2) {
        if has_curve(&pair[0], &pair[1]) && (pair[0].q1.is_none() || pair[1].q0.is_none()) {
            return None;
        }
    }
    if layer["closedPath"] == true {
        let first = points.first()?;
        let last = points.last()?;
        if has_curve(last, first) && (last.q1.is_none() || first.q0.is_none()) {
            return None;
        }
    }
    Some(points)
}

fn normalized_path_points(points: &[PathPoint]) -> Vec<Value> {
    points
        .iter()
        .map(|point| {
            let mut value = json!({"x":point.position.0,"y":point.position.1});
            if let Some((x, y)) = point.q0 {
                value["q0"] = json!({"x":x,"y":y});
            }
            if let Some((x, y)) = point.q1 {
                value["q1"] = json!({"x":x,"y":y});
            }
            if let Some(curved) = point.curved {
                value["curved"] = json!(curved);
            }
            value
        })
        .collect()
}

fn has_unmapped_path_point_fields(layer: &Value) -> bool {
    let point_has_unknown = |point: &Value| {
        point.as_object().is_some_and(|point| {
            point
                .keys()
                .any(|key| !matches!(key.as_str(), "x" | "y" | "q0" | "q1" | "curved"))
                || ["q0", "q1"].into_iter().any(|key| {
                    point
                        .get(key)
                        .and_then(Value::as_object)
                        .is_some_and(|handle| {
                            handle.keys().any(|key| !matches!(key.as_str(), "x" | "y"))
                        })
                })
        })
    };
    layer["pathPoints"]
        .as_array()
        .is_some_and(|points| points.iter().any(point_has_unknown))
}

fn shape_message(number: u64, settings: &Value) -> Vec<u8> {
    let mut shape = Vec::new();
    put_varint(1, number, &mut shape);
    let mut additional = Vec::new();
    match number {
        11 => {
            if let Some(roundness) = settings["roundness"]
                .as_f64()
                .filter(|value| value.is_finite())
            {
                put_f64(1, roundness, &mut additional);
                put_message(2, additional, &mut shape);
            }
        }
        7 => {
            if let Some(sides) = settings["sides"]
                .as_u64()
                .filter(|value| *value <= u32::MAX as u64)
            {
                put_varint(1, sides, &mut additional);
                put_message(3, additional, &mut shape);
            }
        }
        6 => {
            if let Some(radius) = settings["innerRadius"]
                .as_f64()
                .filter(|value| value.is_finite())
            {
                put_f64(1, radius, &mut additional);
            }
            if let Some(points) = settings["points"]
                .as_u64()
                .filter(|value| *value <= u32::MAX as u64)
            {
                put_varint(2, points, &mut additional);
            }
            if !additional.is_empty() {
                put_message(4, additional, &mut shape);
            }
        }
        9 | 10 => {
            if let Some(corner) = point_pair(&settings["arrowCorner"]) {
                if let Some(point) = pair(Some(corner.0), Some(corner.1)) {
                    put_message(1, point, &mut additional);
                    put_message(5, additional, &mut shape);
                }
            }
        }
        _ => {}
    }
    shape
}

fn shape_settings_losses(layer: &Value) -> Vec<(String, String)> {
    let Some(settings) = layer.get("shapeSettings") else {
        return Vec::new();
    };
    let Some(settings) = settings.as_object() else {
        return vec![(
            "shapeSettings".into(),
            "shape settings must be an object".into(),
        )];
    };
    let Some(shape) = shape_number(layer) else {
        return settings
            .keys()
            .map(|key| {
                (
                    format!("shapeSettings/{key}"),
                    "shape settings require a supported ProPresenter shape".into(),
                )
            })
            .collect();
    };
    let supported = match shape {
        11 => &["roundness"][..],
        7 => &["sides"][..],
        6 => &["innerRadius", "points"][..],
        9 | 10 => &["arrowCorner"][..],
        _ => &[][..],
    };
    settings
        .iter()
        .filter_map(|(key, value)| {
            let valid = match (shape, key.as_str()) {
                (11, "roundness") | (6, "innerRadius") => {
                    value.as_f64().is_some_and(f64::is_finite)
                }
                (7, "sides") => value
                    .as_u64()
                    .is_some_and(|sides| (3..=256).contains(&sides)),
                (6, "points") => {
                    value.as_u64().is_some_and(|value| value <= u32::MAX as u64)
                }
                (9 | 10, "arrowCorner") => point_pair(value).is_some(),
                _ => false,
            };
            if valid
                && shape == 11
                && key == "roundness"
                && value.as_f64().is_some_and(|value| !(0.0..=1.0).contains(&value))
            {
                Some((
                    "shapeSettings/roundness".into(),
                    "Orbit runtime geometry supports rounded-rectangle roundness from 0 through 1; the ProPresenter value is still exported".into(),
                ))
            } else if valid
                && shape == 9
                && key == "arrowCorner"
                && point_pair(value).is_some_and(|(x, y)| {
                    !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y)
                })
            {
                Some((
                    "shapeSettings/arrowCorner".into(),
                    "Orbit runtime geometry expects normalized arrow corner coordinates from 0 through 1; the ProPresenter point is still exported".into(),
                ))
            } else if valid
                && shape == 6
                && (key == "innerRadius" && value.as_f64().is_some_and(|v| !(0.0..=1.0).contains(&v))
                    || key == "points" && value.as_u64().is_some_and(|v| !(3..=256).contains(&v)))
            {
                Some((
                    format!("shapeSettings/{key}"),
                    "Orbit runtime geometry supports a star inner radius from 0 through 1 and 3 through 256 points; the ProPresenter value is still exported".into(),
                ))
            } else if valid && shape == 10 && key == "arrowCorner" {
                Some((
                    "shapeSettings/arrowCorner".into(),
                    "Orbit has no verified double-arrow corner mapping; the ProPresenter value is still exported".into(),
                ))
            } else if valid {
                None
            } else if shape == 7
                && key == "sides"
                && value
                    .as_u64()
                    .is_some_and(|sides| {
                        sides <= u32::MAX as u64 && !(3..=256).contains(&sides)
                    })
            {
                Some((
                    "shapeSettings/sides".into(),
                    "Orbit runtime geometry supports 3 through 256 polygon sides; the ProPresenter value is still exported".into(),
                ))
            } else if supported.contains(&key.as_str()) {
                Some((
                    format!("shapeSettings/{key}"),
                    "the value is not a finite ProPresenter shape parameter".into(),
                ))
            } else {
                Some((
                    format!("shapeSettings/{key}"),
                    "this setting has no equivalent for the selected ProPresenter shape".into(),
                ))
            }
        })
        .collect()
}

pub(super) fn path_message(layer: &Value) -> Option<Vec<u8>> {
    let points = path_points(layer);
    let shape = shape_number(layer);
    if points.is_none() && shape.is_none() {
        return None;
    }
    let mut path = Vec::new();
    if let Some(closed) = layer["closedPath"].as_bool() {
        put_varint(1, u64::from(closed), &mut path);
    }
    for point in points.into_iter().flatten() {
        let mut bezier = Vec::new();
        if let Some(position) = pair(Some(point.position.0), Some(point.position.1)) {
            put_message(1, position, &mut bezier);
        }
        for (field, coordinates) in [(2, point.q0), (3, point.q1)] {
            if let Some((x, y)) = coordinates {
                if let Some(value) = pair(Some(x), Some(y)) {
                    put_message(field, value, &mut bezier);
                }
            }
        }
        if let Some(curved) = point.curved {
            put_varint(4, u64::from(curved), &mut bezier);
        }
        put_message(2, bezier, &mut path);
    }
    if let Some(shape) = shape {
        put_message(3, shape_message(shape, &layer["shapeSettings"]), &mut path);
    }
    Some(path)
}

fn has_field(fields: &[Field], number: u32) -> bool {
    fields.iter().any(|field| field.number == number)
}

fn decode_point(fields: &[Field]) -> Option<Value> {
    if [1, 2].into_iter().any(|number| {
        has_field(fields, number)
            && !fields
                .iter()
                .any(|field| field.number == number && matches!(field.value, WireValue::Fixed64(_)))
    }) {
        return None;
    }
    let x = f64_field(fields, 1).unwrap_or(0.0);
    let y = f64_field(fields, 2).unwrap_or(0.0);
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some(json!({"x":x,"y":y}))
}

fn is_canonical_rectangle_points(points: &[Value], closed: bool) -> bool {
    if !closed || points.len() != 4 {
        return false;
    }
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    points.iter().zip(corners).all(|(point, expected)| {
        point_pair(point) == Some(expected)
            && ["q0", "q1"].into_iter().all(|key| {
                point
                    .get(key)
                    .is_none_or(|handle| point_pair(handle) == Some(expected))
            })
            && point.get("curved").is_none_or(|curved| curved == false)
    })
}

fn is_canonical_ellipse_points(points: &[Value], closed: bool) -> bool {
    if !closed || points.len() != 4 {
        return false;
    }
    let inner = 0.223_857_625_084_603_22;
    let outer = 0.776_142_374_915_396_8;
    let expected = [
        ((0.5, 0.0), (inner, 0.0), (outer, 0.0)),
        ((1.0, 0.5), (1.0, inner), (1.0, outer)),
        ((0.5, 1.0), (outer, 1.0), (inner, 1.0)),
        ((0.0, 0.5), (0.0, outer), (0.0, inner)),
    ];
    points.iter().zip(expected).all(|(point, expected)| {
        point_pair(point) == Some(expected.0)
            && point.get("q0").and_then(point_pair) == Some(expected.1)
            && point.get("q1").and_then(point_pair) == Some(expected.2)
            && point.get("curved").and_then(Value::as_bool) == Some(true)
    })
}

fn decode_shape_settings(
    shape: &[Field],
    shape_number: u64,
    unsupported: &mut Vec<String>,
) -> Option<Value> {
    let (field, key) = match shape_number {
        11 => (2, "roundness"),
        7 => (3, "sides"),
        6 => (4, "star"),
        9 | 10 => (5, "arrowCorner"),
        _ => return None,
    };
    if nested_fields(shape, field).count() > 1 {
        unsupported.push(format!("{key} settings repeat their oneof value"));
    }
    let settings = nested_field(shape, field)?;
    let mut decoded = Map::new();
    match shape_number {
        11 => match f64_field(&settings, 1).filter(|value| value.is_finite()) {
            Some(value) => {
                if !(0.0..=1.0).contains(&value) {
                    unsupported.push(format!(
                        "rounded rectangle roundness {value} is outside Orbit's runtime range of 0 through 1; the value is retained"
                    ));
                }
                decoded.insert("roundness".into(), json!(value));
            }
            None if has_field(&settings, 1) => {
                unsupported.push("rounded rectangle roundness has an invalid value".into())
            }
            None => {}
        },
        7 => match varint_field(&settings, 1).filter(|value| *value <= u32::MAX as u64) {
            Some(value) => {
                if !(3..=256).contains(&value) {
                    unsupported.push(format!(
                        "polygon side count {value} is outside Orbit's runtime range of 3 through 256; the value is retained"
                    ));
                }
                decoded.insert("sides".into(), json!(value));
            }
            None if has_field(&settings, 1) => {
                unsupported.push("polygon sides has an invalid value".into())
            }
            None => {}
        },
        6 => {
            match f64_field(&settings, 1).filter(|value| value.is_finite()) {
                Some(value) => {
                    if !(0.0..=1.0).contains(&value) {
                        unsupported.push(format!(
                            "star inner radius {value} is outside Orbit's runtime range of 0 through 1; the value is retained"
                        ));
                    }
                    decoded.insert("innerRadius".into(), json!(value));
                }
                None if has_field(&settings, 1) => {
                    unsupported.push("star inner radius has an invalid value".into())
                }
                None => {}
            }
            match varint_field(&settings, 2).filter(|value| *value <= u32::MAX as u64) {
                Some(value) => {
                    if !(3..=256).contains(&value) {
                        unsupported.push(format!(
                            "star point count {value} is outside Orbit's runtime range of 3 through 256; the value is retained"
                        ));
                    }
                    decoded.insert("points".into(), json!(value));
                }
                None if has_field(&settings, 2) => {
                    unsupported.push("star point count has an invalid value".into())
                }
                None => {}
            }
        }
        9 | 10 => match nested_field(&settings, 1).and_then(|point| decode_point(&point)) {
            Some(point) => {
                if shape_number == 10 {
                    unsupported.push(
                        "double-arrow corner is retained but has no verified Orbit runtime mapping"
                            .into(),
                    );
                } else if point_pair(&point)
                    .is_some_and(|(x, y)| !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y))
                {
                    unsupported.push(
                        "arrow corner coordinates are outside Orbit's normalized runtime range of 0 through 1; the point is retained".into(),
                    );
                }
                decoded.insert("arrowCorner".into(), point);
            }
            None if has_field(&settings, 1) => {
                unsupported.push("arrow corner has invalid coordinates".into())
            }
            None => {}
        },
        _ => {}
    }
    let known = match shape_number {
        11 | 7 | 9 | 10 => &[1][..],
        6 => &[1, 2][..],
        _ => &[][..],
    };
    for field in &settings {
        if !known.contains(&field.number) {
            unsupported.push(format!(
                "{key} settings field {} is not represented by Orbit",
                field.number
            ));
        }
    }
    (!decoded.is_empty()).then_some(Value::Object(decoded))
}

fn decoded_path_size(layer: &Map<String, Value>) -> Option<(f64, f64)> {
    let width = layer.get("bounds")?.get("width")?.as_f64()?;
    let height = layer.get("bounds")?.get("height")?.as_f64()?;
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        .then_some((width, height))
}

fn scale_decoded_point(point: &Value, width: f64, height: f64) -> Option<Value> {
    let (x, y) = point_pair(point)?;
    let (x, y) = (x * width, y * height);
    (x.is_finite() && y.is_finite()).then(|| json!({ "x": x, "y": y }))
}

fn decoded_curve_handles_complete(points: &[Value], closed: bool) -> bool {
    let complete = |start: &Value, end: &Value| {
        (start["curved"] != true && end["curved"] != true)
            || (start["q1"].is_object() && end["q0"].is_object())
    };
    points.windows(2).all(|pair| complete(&pair[0], &pair[1]))
        && (!closed || complete(points.last().unwrap(), &points[0]))
}

/// Decodes ProPresenter's normalized path coordinates into Orbit-local pixels.
pub(super) fn decode_path(fields: &[Field], layer: &mut Map<String, Value>) -> Vec<String> {
    let mut unsupported = Vec::new();
    let shape_fields = nested_field(fields, 3);
    let shape_id = shape_fields
        .as_deref()
        .and_then(|shape| varint_field(shape, 1));
    if has_field(fields, 3) {
        if let Some(number) = shape_id {
            if let Some(name) = shape_name(Some(number)) {
                layer.insert("shape".into(), json!(name));
                if let Some(shape) = shape_fields.as_deref() {
                    if let Some(settings) = decode_shape_settings(shape, number, &mut unsupported) {
                        layer.insert("shapeSettings".into(), settings);
                    }
                    let expected = match number {
                        11 => Some(2),
                        7 => Some(3),
                        6 => Some(4),
                        9 | 10 => Some(5),
                        _ => None,
                    };
                    for field in shape.iter().filter(|field| (2..=5).contains(&field.number)) {
                        if Some(field.number) != expected {
                            unsupported.push(format!(
                                "shape type {number} has unmodeled additional data field {}",
                                field.number
                            ));
                        }
                    }
                    if expected.is_some_and(|expected| {
                        has_field(shape, expected) && nested_field(shape, expected).is_none()
                    }) {
                        unsupported.push(format!(
                            "shape type {number} has malformed additional shape data"
                        ));
                    }
                    for field in shape.iter().filter(|field| field.number > 5) {
                        unsupported.push(format!(
                            "shape field {} is not represented by Orbit",
                            field.number
                        ));
                    }
                }
            } else {
                unsupported.push(format!("path shape type {number} is not supported"));
            }
        } else {
            unsupported.push("path shape has an invalid type".into());
        }
    }

    let points = nested_fields(fields, 2)
        .map(|point| {
            let mut decoded = Map::new();
            let Some(position) = nested_field(&point, 1).and_then(|value| decode_point(&value))
            else {
                unsupported.push("path point has invalid coordinates".into());
                return None;
            };
            decoded.insert("x".into(), position["x"].clone());
            decoded.insert("y".into(), position["y"].clone());
            for (number, key) in [(2, "q0"), (3, "q1")] {
                if !has_field(&point, number) {
                    continue;
                }
                match nested_field(&point, number).and_then(|value| decode_point(&value)) {
                    Some(value) => {
                        decoded.insert(key.into(), value);
                    }
                    None => unsupported.push(format!("path point {key} has invalid coordinates")),
                }
            }
            if has_field(&point, 4) {
                if let Some(value) = bool_field(&point, 4) {
                    decoded.insert("curved".into(), json!(value));
                } else {
                    unsupported.push("path point curved flag has an invalid wire type".into());
                }
            }
            for field in &point {
                if !(1..=4).contains(&field.number) {
                    unsupported.push(format!(
                        "path point field {} is not represented by Orbit",
                        field.number
                    ));
                }
            }
            Some(Value::Object(decoded))
        })
        .collect::<Option<Vec<_>>>();
    if has_field(fields, 2) {
        match points {
            Some(points)
                if shape_id == Some(1)
                    && is_canonical_rectangle_points(
                        &points,
                        bool_field(fields, 1) == Some(true),
                    ) => {}
            Some(points)
                if shape_id == Some(2)
                    && is_canonical_ellipse_points(
                        &points,
                        bool_field(fields, 1) == Some(true),
                    ) => {}
            // A single point is kept as written; ProPresenter draws nothing for it either.
            Some(points) if !points.is_empty() => {
                if points.iter().any(|point| {
                    point["curved"] == true
                        && (!point["q0"].is_object() || !point["q1"].is_object())
                }) {
                    unsupported.push(
                        "curved path point is missing an incoming or outgoing control point".into(),
                    );
                } else if !decoded_curve_handles_complete(
                    &points,
                    bool_field(fields, 1) == Some(true),
                ) {
                    unsupported
                        .push("curved path segment is missing an endpoint control point".into());
                } else if let Some((width, height)) = decoded_path_size(layer) {
                    let points = points
                        .iter()
                        .map(|point| {
                            let mut point = point.as_object()?.clone();
                            for key in ["x", "y"] {
                                let value = point.get(key)?.as_f64()?;
                                if !value.is_finite() {
                                    return None;
                                }
                            }
                            for key in ["q0", "q1"] {
                                if let Some(control) = point.get(key).cloned() {
                                    let control = scale_decoded_point(&control, width, height)?;
                                    point.insert(key.into(), control);
                                }
                            }
                            let (x, y) = (
                                point.get("x")?.as_f64()? * width,
                                point.get("y")?.as_f64()? * height,
                            );
                            if !(x.is_finite() && y.is_finite()) {
                                return None;
                            }
                            point.insert("x".into(), json!(x));
                            point.insert("y".into(), json!(y));
                            Some(Value::Object(point))
                        })
                        .collect::<Option<Vec<_>>>();
                    if let Some(points) = points {
                        if !matches!(
                            layer.get("shape").and_then(Value::as_str),
                            None | Some("custom")
                        ) {
                            layer.insert(
                                "shape".into(),
                                json!(if bool_field(fields, 1) == Some(true) {
                                    "freeform"
                                } else {
                                    "stroke"
                                }),
                            );
                            layer.remove("shapeSettings");
                        }
                        layer.insert("pathPoints".into(), Value::Array(points));
                        layer.insert("pathCoordinateSpace".into(), json!("local"));
                        layer.insert(
                            "closedPath".into(),
                            json!(bool_field(fields, 1).unwrap_or(false)),
                        );
                    } else {
                        unsupported.push("path point has non-finite coordinates".into());
                    }
                } else {
                    unsupported.push(
                        "custom path coordinates require finite, positive layer bounds".into(),
                    );
                }
            }
            Some(_) => unsupported.push("path has no points".into()),
            None => {}
        }
    }
    for field in fields {
        if !(1..=3).contains(&field.number) {
            unsupported.push(format!(
                "path field {} is not represented by Orbit",
                field.number
            ));
        }
    }
    unsupported
}

fn media_source(layer: &Value) -> std::borrow::Cow<'_, Value> {
    if layer["fillType"] == "web" {
        let mut media = layer.clone();
        media["mediaType"] = json!("web");
        media["mediaUrl"] = layer["webUrl"].clone();
        std::borrow::Cow::Owned(media)
    } else {
        std::borrow::Cow::Borrowed(layer)
    }
}

pub(super) fn media_message(layer: &Value, relative: &BTreeMap<String, String>) -> Option<Vec<u8>> {
    let layer = media_source(layer);
    let url = layer["mediaUrl"]
        .as_str()
        .filter(|url| !url.trim().is_empty())?;
    let (kind, properties) = layer_media_export::type_properties(&layer, "media")?;
    let mut media = Vec::new();
    if let Some(id) = layer["mediaId"].as_str().filter(|id| !id.trim().is_empty()) {
        put_uuid(1, id, &mut media);
    }
    put_message(2, encode_playlist_url(url, relative), &mut media);
    put_message(kind, properties, &mut media);
    Some(media)
}

/// ProPresenter blends the first two stops and writes no stop position or blend point.
pub(super) fn gradient_message(angle: Option<f64>, colors: &[Value]) -> Option<Vec<u8>> {
    if colors.is_empty() {
        return None;
    }
    let mut gradient = Vec::new();
    if let Some(angle) = angle {
        put_f64(2, angle, &mut gradient);
    }
    for color in colors.iter().take(2) {
        let mut stop = Vec::new();
        put_layer_color(
            1,
            color.as_str().filter(|value| is_color(value))?,
            &mut stop,
        );
        put_message(4, stop, &mut gradient);
    }
    Some(gradient)
}

fn fill_message(layer: &Value, relative: &BTreeMap<String, String>) -> Option<Vec<u8>> {
    let kind = layer["fillType"].as_str()?;
    let mut fill = Vec::new();
    match kind {
        "color" | "none" => {
            if let Some(color) = layer["fill"].as_str().filter(|value| is_color(value)) {
                put_layer_color(1, color, &mut fill);
            }
        }
        "gradient" => put_message(
            2,
            gradient_message(layer["fillAngle"].as_f64(), layer["fillColors"].as_array()?)?,
            &mut fill,
        ),
        "background-effect" => put_message(
            5,
            background_effect_message(
                layer["backgroundEffect"].as_str()?,
                layer["backgroundBlurAmount"].as_f64(),
                layer["backgroundBlurSaturation"].as_f64(),
            )?,
            &mut fill,
        ),
        "media" => put_message(3, media_message(layer, relative)?, &mut fill),
        "web" => {
            let url = layer["webUrl"].as_str()?;
            crate::web_capture::page_url(url).ok()?;
            put_message(3, media_message(layer, relative)?, &mut fill);
        }
        _ => return None,
    }
    put_varint(
        4,
        u64::from(layer["fillEnabled"].as_bool().unwrap_or(kind != "none")),
        &mut fill,
    );
    Some(fill)
}

fn fill_message_for_loss_review(
    layer: &Value,
    relative: &BTreeMap<String, String>,
) -> Option<Vec<u8>> {
    if let Some(fill) = fill_message(layer, relative) {
        return Some(fill);
    }
    if layer["fillType"] != "media" {
        return None;
    }
    let source = [
        "sourcePath",
        "mediaSourcePath",
        "mediaPath",
        "mediaUrl",
        "sourceUrl",
    ]
    .into_iter()
    .filter_map(|key| layer.get(key).and_then(Value::as_str))
    .find(|source| !source.trim().is_empty())?;
    let mut review_layer = layer.clone();
    review_layer["mediaUrl"] = Value::from(source);
    fill_message(&review_layer, relative)
}

fn has(layer: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .any(|key| super::super::import_export_report::present(&layer[*key]))
}

fn stroke_message(layer: &Value) -> Option<Vec<u8>> {
    if !has(
        layer,
        &[
            "stroke",
            "strokeEnabled",
            "strokeWidth",
            "strokeColor",
            "strokeStyle",
        ],
    ) {
        return None;
    }
    let mut stroke = Vec::new();
    if let Some(style) = STROKE_STYLES
        .iter()
        .position(|name| layer["strokeStyle"] == *name)
    {
        put_varint(1, style as u64, &mut stroke);
    }
    if let Some(width) = crate::native_compositor::leading_number(layer.get("stroke"))
        .or_else(|| layer["strokeWidth"].as_f64())
        .or_else(|| {
            layer["stroke"]
                .as_str()
                .filter(|stroke| stroke.split_whitespace().any(is_color))
                .map(|_| 1.0)
        })
    {
        put_f64(2, width, &mut stroke);
    }
    let stroke_color = layer["stroke"]
        .as_str()
        .and_then(|stroke| {
            stroke
                .split_whitespace()
                .rev()
                .find(|value| is_color(value))
        })
        .or_else(|| {
            layer["strokeColor"]
                .as_str()
                .filter(|value| is_color(value))
        });
    if let Some(color) = stroke_color {
        put_layer_color(3, color, &mut stroke);
    }
    put_varint(
        5,
        u64::from(
            layer["strokeEnabled"].as_bool().unwrap_or(
                layer["stroke"]
                    .as_str()
                    .is_some_and(|stroke| !stroke.is_empty())
                    && stroke_color.is_some(),
            ),
        ),
        &mut stroke,
    );
    Some(stroke)
}

fn shadow_message(layer: &Value) -> Option<Vec<u8>> {
    let keys = [
        "shadowEnabled",
        "shadowAngle",
        "shadowOffset",
        "shadowBlur",
        "shadowOpacity",
        "shadowColor",
    ];
    if !has(layer, &keys) {
        return None;
    }
    let mut shadow = Vec::new();
    for (field, key) in [(2, "shadowAngle"), (3, "shadowOffset"), (4, "shadowBlur")] {
        if let Some(value) = layer[key].as_f64() {
            put_f64(field, value, &mut shadow);
        }
    }
    if let Some(color) = layer["shadowColor"]
        .as_str()
        .filter(|value| is_color(value))
    {
        put_layer_color(5, color, &mut shadow);
    }
    if let Some(opacity) = layer["shadowOpacity"]
        .as_f64()
        .filter(|value| (0.0..=100.0).contains(value))
    {
        put_f64(6, opacity / 100.0, &mut shadow);
    }
    if let Some(on) = layer["shadowEnabled"].as_bool() {
        put_varint(7, u64::from(on), &mut shadow);
    }
    Some(shadow)
}

fn feathering_message(layer: &Value) -> Option<Vec<u8>> {
    let enabled = layer.get("featheringEnabled").and_then(Value::as_bool);
    let amount = layer
        .get("featheringAmount")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value));
    if enabled.is_none() && amount.is_none() && layer.get("featheringStyle").is_none() {
        return None;
    }
    let style = match layer["featheringStyle"].as_str().unwrap_or("inside") {
        "inside" => 0,
        "center" => 1,
        "outside" => 2,
        _ => return None,
    };
    let mut feather = Vec::new();
    put_varint(1, style, &mut feather);
    if let Some(amount) = amount {
        put_f64(2, amount / 100.0, &mut feather);
    }
    if let Some(enabled) = enabled {
        put_varint(3, u64::from(enabled), &mut feather);
    }
    Some(feather)
}

pub(super) fn decode_feathering(fields: &[Field], layer: &mut Map<String, Value>) -> Vec<String> {
    let mut unsupported = Vec::new();
    let feather_count = nested_fields(fields, 12).count();
    if !has_field(fields, 12) {
        return unsupported;
    }
    if fields
        .iter()
        .any(|field| field.number == 12 && !matches!(field.value, WireValue::Bytes(_)))
    {
        unsupported.push("feathering has an invalid wire type".into());
    }
    if feather_count == 0 {
        return unsupported;
    }
    if feather_count != 1 {
        unsupported.push("feathering occurs more than once".into());
    }
    let Some(feather) = nested_field(fields, 12) else {
        return unsupported;
    };
    // Ignore inert defaults, but retain a disabled authored feather so re-enabling it and round trips preserve its settings.
    if bool_field(&feather, 3) != Some(true)
        && varint_field(&feather, 1).is_none_or(|style| style == 0)
        && f64_field(&feather, 2).is_none_or(|radius| radius == 0.)
        && feather
            .iter()
            .all(|field| match (&field.value, field.number) {
                (WireValue::Varint(value), 1 | 3) => *value == 0,
                (WireValue::Fixed64(value), 2) => f64::from_bits(*value) == 0.,
                _ => false,
            })
    {
        return unsupported;
    }
    if feather.iter().any(|field| !matches!(field.number, 1..=3)) {
        unsupported.push("feathering has unmodeled fields".into());
    }
    for field in &feather {
        let valid_wire_type = match field.number {
            1 | 3 => matches!(field.value, WireValue::Varint(_)),
            2 => matches!(field.value, WireValue::Fixed64(_)),
            _ => true,
        };
        if !valid_wire_type {
            unsupported.push(format!(
                "feathering field {} has an invalid wire type",
                field.number
            ));
        }
    }
    let style = match varint_field(&feather, 1).unwrap_or(0) {
        0 => "inside",
        1 => "center",
        2 => "outside",
        _ => {
            unsupported.push("feathering style is invalid".into());
            return unsupported;
        }
    };
    if has_field(&feather, 1) && varint_field(&feather, 1).is_none() {
        unsupported.push("feathering style has an invalid wire type".into());
        return unsupported;
    }
    layer.insert("featheringStyle".into(), json!(style));
    if has_field(&feather, 1) && feather.iter().filter(|field| field.number == 1).count() > 1 {
        unsupported.push("feathering style occurs more than once".into());
        return unsupported;
    }
    if has_field(&feather, 2) {
        if feather.iter().filter(|field| field.number == 2).count() > 1 {
            unsupported.push("feathering radius occurs more than once".into());
        }
        match f64_field(&feather, 2)
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        {
            Some(radius) => {
                layer.insert("featheringAmount".into(), json!(radius * 100.0));
            }
            None => unsupported.push("feathering radius is invalid or outside 0–1".into()),
        }
    } else {
        layer.insert("featheringAmount".into(), json!(0.0));
    }
    if has_field(&feather, 3) {
        if feather.iter().filter(|field| field.number == 3).count() > 1 {
            unsupported.push("feathering enabled flag occurs more than once".into());
        }
        match bool_field(&feather, 3) {
            Some(enabled) => {
                layer.insert("featheringEnabled".into(), json!(enabled));
            }
            None => unsupported.push("feathering enabled flag has an invalid wire type".into()),
        }
    }
    unsupported
}

fn feathering_losses(layer: &Value) -> Vec<(String, String)> {
    let mut losses = Vec::new();
    if layer.get("featheringStyle").is_some()
        && !matches!(
            layer["featheringStyle"].as_str(),
            Some("inside" | "center" | "outside")
        )
    {
        losses.push((
            "featheringStyle".into(),
            "feather style must be inside, center or outside".into(),
        ));
    }
    if super::super::import_export_report::present(&layer["featheringEnabled"])
        && !layer["featheringEnabled"].is_boolean()
    {
        losses.push((
            "featheringEnabled".into(),
            "feathering enabled must be a boolean".into(),
        ));
    }
    if super::super::import_export_report::present(&layer["featheringAmount"])
        && !layer["featheringAmount"]
            .as_f64()
            .is_some_and(|value| value.is_finite() && (0.0..=100.0).contains(&value))
    {
        losses.push((
            "featheringAmount".into(),
            "inside feather amount must be finite and between 0 and 100 percent".into(),
        ));
    }
    losses
}

fn plain_primary_fields(layer: &Value) -> bool {
    layer["role"] == "primary"
        && layer.as_object().is_some_and(|object| {
            object.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "id" | "type" | "role" | "bounds" | "text" | "usePresentationText"
                )
            })
        })
}

pub(crate) fn is_plain_primary(layer: &Value, width: f64, height: f64) -> bool {
    plain_primary_fields(layer)
        && (layer["bounds"].is_null()
            || layer["bounds"]
                == json!({
                    "x":width * 0.05,"y":height * 0.1,"width":width * 0.9,"height":height * 0.8
                }))
}

fn is_text(layer: &Value) -> bool {
    layer["type"]
        .as_str()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("text"))
}

/// One `Slide.Element`. `text`, `runs` and `paragraph_style` are only written for text layers; the
/// primary text layer takes the paragraphs of the composed slide text style.
fn encode_layer(
    layer: &Value,
    text: &str,
    runs: &[Value],
    paragraph_style: &Map<String, Value>,
    primary: bool,
    relative: &BTreeMap<String, String>,
    default_color: &str,
    slide_width: Option<f64>,
) -> Vec<u8> {
    let layer_id = normalized_uuid(layer["id"].as_str().unwrap_or_default());
    let mut graphics = Vec::new();
    put_uuid(1, &layer_id, &mut graphics);
    if let Some(name) = layer["name"].as_str() {
        put_string(2, name, &mut graphics);
    }
    if let Some(bounds) = bounds_message(&layer["bounds"]) {
        put_message(3, bounds, &mut graphics);
    }
    if let Some(rotation) = layer["rotation"].as_f64() {
        put_f64(4, rotation, &mut graphics);
    }
    let flip_mode = 2 * u64::from(layer["flipX"] == true) + u64::from(layer["flipY"] == true);
    if flip_mode != 0 {
        put_varint(15, flip_mode, &mut graphics);
    }
    if let Some(opacity) = layer["opacity"].as_f64() {
        put_f64(5, opacity / 100.0, &mut graphics);
    }
    if let Some(locked) = layer["locked"].as_bool() {
        put_varint(6, u64::from(locked), &mut graphics);
    }
    for (field, message) in [
        (8, path_message(layer)),
        (9, fill_message(layer, relative)),
        (10, stroke_message(layer)),
        (11, shadow_message(layer)),
        (12, feathering_message(layer)),
    ] {
        if let Some(message) = message {
            put_message(field, message, &mut graphics);
        }
    }
    let text_layer = is_text(layer);
    if text_layer {
        let value = text_value(
            text,
            runs,
            &layer["textStyle"],
            paragraph_style,
            default_color,
            relative,
            layer["bounds"]["width"]
                .as_f64()
                .map(|width| {
                    width
                        - layer["textStyle"]["textMarginLeft"].as_f64().unwrap_or(0.0)
                        - layer["textStyle"]["textMarginRight"]
                            .as_f64()
                            .unwrap_or(0.0)
                })
                .filter(|width| *width > 0.0),
        );
        put_message(13, value, &mut graphics);
    }
    let style = &layer["textStyle"];
    if let Some(enabled) = style["linesOnlyEnabled"]
        .as_bool()
        .filter(|_| layer_text_settings_export::valid_line_mask(style))
    {
        let mut mask = Vec::new();
        put_varint(1, u64::from(enabled), &mut mask);
        let kind = match style["linesOnlyStyle"].as_str() {
            Some("Line Width") => 1,
            Some("Max Line Width") => 2,
            _ => 0,
        };
        put_varint(4, kind, &mut mask);
        for (field, key) in [
            (2, "linesOnlyHeightOffset"),
            (3, "linesOnlyVerticalOffset"),
            (5, "linesOnlyWidthOffset"),
            (6, "linesOnlyHorizontalOffset"),
        ] {
            if let Some(value) = style[key].as_f64() {
                put_f64(field, value, &mut mask);
            }
        }
        put_message(14, mask, &mut graphics);
    }
    if let Some(hidden) = layer["hidden"].as_bool() {
        put_varint(16, u64::from(hidden), &mut graphics);
    }
    let mut element = Vec::new();
    put_message(1, graphics, &mut element);
    if let Some(build) = layer_build_export::build_message(layer, &layer_id) {
        put_message(2, build, &mut element);
    }
    if let Some(build) = layer_build_export::build_message_for(layer, &layer_id, "buildOut") {
        put_message(3, build, &mut element);
    }
    layer_build_export::encode_reveal(layer, &mut element);
    if let Some(scroller) = layer_text_settings_export::encode_text_scroller(
        &Value::Object(paragraph_style.clone()),
        layer.get("bounds"),
        slide_width,
    ) {
        put_message(9, scroller, &mut element);
    }
    for link in layer_link_export::encode_data_links(layer) {
        put_message(6, link, &mut element);
    }
    if text_layer {
        // INFO_IS_TEXT_ELEMENT, plus INFO_IS_TEMPLATE_TEXT so the importer keeps the same slide text
        put_varint(4, if primary { 3 } else { 2 }, &mut element);
    }
    element
}

/// The slide elements of a slide with layers of its own, frontmost first as ProPresenter stores
/// them; `None` when the slide has only the default text layer, which keeps the text-slide export.
pub(super) fn slide_elements(
    slide: &Value,
    width: f64,
    height: f64,
    default_color: &str,
    relative: &BTreeMap<String, String>,
) -> Option<Vec<Vec<u8>>> {
    let layers = slide["layers"].as_array().filter(|layers| {
        layers
            .iter()
            .any(|layer| !is_plain_primary(layer, width, height))
    })?;
    let content = slide
        .get("content")
        .or_else(|| slide.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let authored_runs = slide.get("contentRuns").and_then(Value::as_array);
    let primary_layer_runs = if slide.get("contentRuns").is_none() {
        selected_primary_text_layer(slide)
            .filter(|layer| layer["text"].as_str() == Some(content))
            .and_then(|layer| layer["textRuns"].as_array())
    } else {
        None
    };
    let slide_runs = authored_runs
        .or(primary_layer_runs)
        .map_or(&[][..], Vec::as_slice);
    let primary_style = portable_text_style(slide);
    let primary_layer_id =
        selected_primary_text_layer(slide).and_then(|layer| layer["id"].as_str());
    let scroll_width = slide.get("width").and_then(Value::as_f64);
    let default_text = || {
        encode_text_element_with_style(
            content,
            slide_runs,
            width,
            height,
            default_color,
            &primary_style,
        )
    };
    // Back to front, as composition orders them.
    let mut ordered = layers.iter().collect::<Vec<_>>();
    if let Some(order) = slide["themeLayerOrder"].as_array() {
        ordered.sort_by_key(|layer| {
            order
                .iter()
                .position(|id| *id == layer["id"])
                .unwrap_or(order.len())
        });
    }
    let mut elements = ordered
        .into_iter()
        .rev()
        .map(|layer| {
            let grouped_layer = layer_build_export::grouped_layer(slide, layer);
            let layer = &grouped_layer;
            let primary = primary_layer_id.is_some_and(|id| layer["id"].as_str() == Some(id));
            if primary && is_plain_primary(layer, width, height) {
                return default_text();
            }
            if primary {
                return encode_layer(
                    layer,
                    content,
                    slide_runs,
                    &primary_style,
                    true,
                    relative,
                    default_color,
                    scroll_width,
                );
            }
            let runs = layer["textRuns"].as_array().map_or(&[][..], Vec::as_slice);
            let text = layer["text"].as_str().unwrap_or_default();
            let paragraph_style = layer["textStyle"].as_object().cloned().unwrap_or_default();
            encode_layer(
                layer,
                text,
                runs,
                &paragraph_style,
                false,
                relative,
                default_color,
                scroll_width,
            )
        })
        .collect::<Vec<_>>();
    if !layers.iter().any(|layer| is_text(layer)) {
        elements.push(default_text());
    }
    Some(elements)
}

pub(crate) fn losses_with_slide_width(
    layer: &Value,
    reference: &str,
    slide_width: Option<f64>,
    findings: &mut Vec<String>,
) {
    losses_with_context(layer, reference, slide_width, true, findings);
}

fn losses_with_context(
    layer: &Value,
    reference: &str,
    slide_width: Option<f64>,
    scroller_encoded: bool,
    findings: &mut Vec<String>,
) {
    let Some(object) = layer.as_object().filter(|_| !plain_primary_fields(layer)) else {
        return;
    };
    let mut lost = |key: &str, reason: &str| findings.push(format!("{reference}/{key}: {reason}"));
    for (key, value) in object {
        let derived_stroke = key == "stroke"
            && value
                .as_str()
                .is_some_and(|stroke| stroke.split_whitespace().any(is_color));
        if !LAYER_KEYS.contains(&key.as_str())
            && !derived_stroke
            && super::super::import_export_report::present(value)
        {
            lost(key, "not included in the layered slide export");
        }
    }
    if layer["fillType"] != "media" {
        for key in ["sourcePath", "sourceUrl", "mediaSourcePath", "mediaPath"] {
            if super::super::import_export_report::present(&layer[key]) {
                lost(
                    key,
                    "media source paths are used only to resolve a media fill dependency",
                );
            }
        }
    }
    for (key, value) in layer["textStyle"]
        .as_object()
        .into_iter()
        .flat_map(|style| style.iter())
    {
        if let Some(supported) =
            layer_text_paint_export::style_field_supported(&layer["textStyle"], key)
        {
            if !supported && super::super::import_export_report::present(value) {
                lost(
                    &format!("textStyle/{key}"),
                    "not represented by a supported ProPresenter text fill",
                );
            }
            continue;
        }
        if !TEXT_STYLE_KEYS.contains(&key.as_str()) {
            lost(
                &format!("textStyle/{key}"),
                "not included in the layered slide export",
            );
            continue;
        }
        let style = &layer["textStyle"];
        let encoded = match key.as_str() {
            "fontFamily" | "fontFace" => value.is_string(),
            "letterSpacing" => value.as_f64().is_some_and(f64::is_finite),
            "superscript" => value
                .as_i64()
                .is_some_and(|level| i32::try_from(level).is_ok()),
            "superscriptStandardized" => value.is_boolean(),
            "fontSize" => value
                .as_f64()
                .is_some_and(|size| size.is_finite() && size > 0.0),
            "italic" | "bold" | "underline" | "strikethrough" | "strokeEnabled" => {
                value.is_boolean()
            }
            "underlineStyle" | "strikethroughStyle" => value
                .as_str()
                .is_some_and(|name| matches!(name, "single" | "thick" | "double")),
            "underlinePattern" | "strikethroughPattern" => value.as_str().is_some_and(|name| {
                matches!(name, "solid" | "dot" | "dash" | "dash-dot" | "dash-dot-dot")
            }),
            "underlineByWord" | "strikethroughByWord" => value.is_boolean(),
            "underlineColor" | "strikethroughColor" | "textColor" => {
                value.as_str().is_some_and(is_color)
            }
            "textVerticalAlign" => value
                .as_str()
                .is_some_and(|value| VERTICAL_ALIGNMENTS.contains(&value)),
            "textHorizontalAlign" => value.as_str().is_some_and(|value| {
                matches!(value, "left" | "center" | "right" | "justify" | "natural")
            }),
            "capitalization" => value.as_str().and_then(capitalization_number).is_some(),
            "scaling" => value.as_str().and_then(scaling_number).is_some(),
            "shadowEnabled" => value.is_boolean(),
            "shadowColor" => value.as_str().is_some_and(is_color),
            "shadowOpacity" => value
                .as_f64()
                .is_some_and(|opacity| (0.0..=100.0).contains(&opacity)),
            "shadowAngle" | "shadowOffset" | "shadowBlur" => value.is_number(),
            "strokeWidth" => value.as_f64().is_some_and(|width| {
                if style["strokeEnabled"] == true {
                    width.is_finite() && width > 0.0
                } else {
                    width == 0.0 || width == 2.0
                }
            }),
            "strokeColor" => value.as_str().is_some_and(is_color),
            "textFillMode" => {
                value == "Text Color" || text_background_effect_message(style).is_some()
            }
            "backgroundBlurAmount" | "backgroundBlurSaturation" => {
                style["textFillMode"] == "Background Blur"
                    && text_background_effect_message(style).is_some()
            }
            _ => true,
        };
        if super::super::import_export_report::present(value) && !encoded {
            lost(
                &format!("textStyle/{key}"),
                "not represented by a supported ProPresenter text style",
            );
        }
    }
    if !is_text(layer) && has(layer, &["text", "textRuns", "textStyle"]) {
        lost("text", "only text layers are exported with text");
    }
    let no_media = BTreeMap::new();
    if has(layer, &["fillType", "fillColors", "fillAngle"])
        && fill_message_for_loss_review(layer, &no_media).is_none()
        && !layer_link_export::has_fill_data_link(layer)
    {
        lost("fillType", "only color, gradient, none, image or video media, and valid blur or invert fills are exported");
    }
    if layer["fillType"].is_null() && super::super::import_export_report::present(&layer["fill"]) {
        lost("fill", "a fill without a fill type is not exported");
    }
    if layer["fillType"] == "gradient"
        && layer["fillColors"].as_array().is_some_and(|colors| {
            colors
                .iter()
                .any(|color| !color.as_str().is_some_and(|color| is_color(color)))
        })
    {
        lost(
            "fillColors",
            "every gradient stop must be a color supported by the native renderer",
        );
    }
    if layer["fillType"] == "gradient"
        && super::super::import_export_report::present(&layer["fillAngle"])
        && !layer["fillAngle"].as_f64().is_some_and(f64::is_finite)
    {
        lost("fillAngle", "gradient angle must be a finite number");
    }
    if layer["fill"]
        .as_str()
        .is_some_and(|fill| !fill.is_empty() && !is_color(fill))
        && matches!(layer["fillType"].as_str(), Some("color" | "none"))
    {
        lost(
            "fill",
            "only colors supported by the native renderer are exported",
        );
    }
    if layer["fillType"] != "media" && has(layer, &["mediaUrl"]) {
        lost("mediaUrl", "media is only exported as a media fill");
    }
    if layer["fillType"] == "media"
        && ![
            "sourcePath",
            "mediaSourcePath",
            "mediaPath",
            "mediaUrl",
            "sourceUrl",
        ]
        .into_iter()
        .filter_map(|key| layer.get(key).and_then(Value::as_str))
        .any(|source| !source.trim().is_empty())
    {
        lost(
            "mediaUrl",
            "media fills require a local source path or URL to export",
        );
    }
    if layer["fillType"] == "media" {
        for key in ["sourcePath", "sourceUrl", "mediaSourcePath", "mediaPath"] {
            if super::super::import_export_report::present(&layer[key]) && !layer[key].is_string() {
                lost(key, "media source dependency must be a path or URL string");
            }
        }
    }
    if super::super::import_export_report::present(&layer["mediaId"])
        && (!layer["mediaId"].is_string()
            || !matches!(layer["fillType"].as_str(), Some("media" | "web"))
            || fill_message_for_loss_review(layer, &no_media).is_none())
    {
        lost(
            "mediaId",
            "media identity requires a valid mapped ProPresenter media or web object",
        );
    }
    if super::super::import_export_report::present(&layer["mediaTitle"]) {
        lost(
            "mediaTitle",
            "the selected media title is not mapped to a ProPresenter Media.Metadata field",
        );
    }
    if layer["fillType"] == "media"
        && super::super::import_export_report::present(&layer["mediaType"])
        && layer_media_export::type_properties(layer, "media").is_none()
    {
        lost(
            "mediaType",
            "this media type has no mapped ProPresenter layer property",
        );
    }
    if layer["fillType"] == "web"
        && super::super::import_export_report::present(&layer["mediaType"])
        && layer["mediaType"] != "web"
    {
        lost(
            "mediaType",
            "web fills export as ProPresenter web content regardless of mediaType",
        );
    }
    if !matches!(layer["fillType"].as_str(), Some("media" | "web")) {
        if super::super::import_export_report::present(&layer["mediaType"]) {
            lost(
                "mediaType",
                "media type is exported only with a media or web fill",
            );
        }
        for key in layer_media_export::active_fields(layer, "media") {
            lost(
                &key,
                "active media settings are exported only with a media or web fill",
            );
        }
    }
    if super::super::import_export_report::present(&layer["webUrl"])
        && (layer["fillType"] != "web"
            || layer["webUrl"]
                .as_str()
                .is_none_or(|url| crate::web_capture::page_url(url).is_err()))
    {
        lost("webUrl", "web fills require an http or https URL");
    }
    if layer["shape"].as_str().is_some() && shape_number(layer).is_none() {
        lost("shape", "this shape is not a ProPresenter shape");
    }
    let background_effect = layer["fillType"] == "background-effect"
        && layer["fillEnabled"] != false
        && fill_message(layer, &no_media).is_some();
    if super::super::import_export_report::present(&layer["backgroundEffect"])
        && (!background_effect || layer["backgroundEffect"].as_str().is_none())
    {
        lost(
            "backgroundEffect",
            "only blur and invert background-effect fills are exported",
        );
    }
    for key in ["backgroundBlurAmount", "backgroundBlurSaturation"] {
        if super::super::import_export_report::present(&layer[key])
            && (!background_effect || layer["backgroundEffect"] != "blur")
        {
            lost(
                key,
                "blur settings are exported only with a supported blur fill",
            );
        }
    }
    if layer["shadowOpacity"]
        .as_f64()
        .is_some_and(|opacity| !(0.0..=100.0).contains(&opacity))
    {
        lost(
            "shadowOpacity",
            "shadow opacity must be between 0 and 100 percent",
        );
    }
    let parsed_path_points = path_points(layer);
    let valid_path = parsed_path_points.is_some();
    if super::super::import_export_report::present(&layer["pathPoints"]) && !valid_path {
        lost(
            "pathPoints",
            "paths require finite points, positive bounds, and both handles for each curved anchor",
        );
    }
    if has_unmapped_path_point_fields(layer) {
        lost(
            "pathPoints",
            "path points only export x, y, q0, q1, and curved fields",
        );
    }
    if let Some(points) = parsed_path_points.as_deref() {
        let points = normalized_path_points(points);
        let closed = layer["closedPath"] == true;
        let exact_preset_points = match shape_number(layer) {
            Some(1) => is_canonical_rectangle_points(&points, closed),
            Some(2) => is_canonical_ellipse_points(&points, closed),
            Some(8) | None => true,
            Some(_) => false,
        };
        if !exact_preset_points {
            lost(
                "pathPoints",
                "custom points on a preset shape do not have an Orbit path representation",
            );
        }
    }
    if super::super::import_export_report::present(&layer["pathCoordinateSpace"]) && !valid_path {
        lost(
            "pathCoordinateSpace",
            "a coordinate space is exported only with valid path points",
        );
    }
    if layer["closedPath"] == true && !valid_path {
        lost("closedPath", "closed paths require valid path points");
    }
    if super::super::import_export_report::present(&layer["closedPath"])
        && !layer["closedPath"].is_boolean()
    {
        lost("closedPath", "closedPath must be a boolean");
    }
    for (key, reason) in shape_settings_losses(layer) {
        lost(&key, &reason);
    }
    for (key, reason) in feathering_losses(layer) {
        lost(&key, &reason);
    }
    for key in ["strokeColor", "shadowColor"] {
        if layer[key].as_str().is_some_and(|color| !is_color(color)) {
            lost(
                key,
                "only colors supported by the native renderer are exported",
            );
        }
    }
    if matches!(layer["fillType"].as_str(), Some("media" | "web")) {
        for key in layer_media_export::unsupported_fields(&media_source(layer), "media") {
            findings.push(format!(
                "{reference}/{key}: not represented by a supported ProPresenter media property"
            ));
        }
    }
    layer_text_settings_export::losses_with_context(
        layer,
        reference,
        slide_width,
        scroller_encoded,
        findings,
    );
    layer_link_export::losses(layer, reference, findings);
    layer_build_export::losses(layer, reference, findings);
}

/// Reviews the effective slide text style through the same checks used for a text layer.
pub(crate) fn text_style_losses(
    style: &serde_json::Map<String, Value>,
    reference: &str,
    slide_width: Option<f64>,
    bounds: Option<&Value>,
    scroller_encoded: bool,
    findings: &mut Vec<String>,
) {
    losses_with_context(
        &json!({"type":"Text","bounds":bounds,"textStyle":style}),
        reference,
        slide_width,
        scroller_encoded,
        findings,
    );
}

/// Reviews a layer's object fields while leaving its style to the effective-style check.
pub(crate) fn losses_without_text_style(
    layer: &Value,
    reference: &str,
    slide_width: Option<f64>,
    findings: &mut Vec<String>,
) {
    let mut layer = layer.clone();
    if let Some(object) = layer.as_object_mut() {
        object.remove("textStyle");
    }
    losses_with_context(&layer, reference, slide_width, true, findings);
}
