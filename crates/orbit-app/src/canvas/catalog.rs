use super::editing;
use serde_json::{json, Value};
#[derive(Clone)]
pub enum Kind {
    Text,
    Multiline,
    Readonly,
    Number(f64, f64),
    Bool,
    Color,
    Choice(Vec<(String, String)>),
}
#[derive(Clone, Copy, PartialEq)]
pub enum Target {
    Document,
    Slide,
    Layer,
    Shape,
    Text,
    Bounds,
    Build,
    Tab,
    Timer,
    Trigger,
    Path,
    Effect,
    Object,
    Note,
}
#[derive(Clone)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub kind: Kind,
    pub target: Target,
    pub value: Value,
    pub integer: bool,
}
impl Field {
    pub fn new(key: &str, target: Target, kind: Kind, value: Value) -> Self {
        Self {
            key: key.into(),
            label: format!("theme.gpui.{key}"),
            kind,
            target,
            value,
            integer: target == Target::Bounds
                || matches!(
                    key,
                    "buildStep" | "buildOutStep" | "textListLevel" | "textListStart" | "inputIndex"
                ),
        }
    }
    pub fn named(mut self, label: &str) -> Self {
        self.label = label.into();
        self
    }
}
pub fn choices(values: &[&str]) -> Kind {
    Kind::Choice(
        values
            .iter()
            .map(|s| (s.to_string(), option_label(s)))
            .collect(),
    )
}
/// A choice whose option labels are explicit translation keys, for values that more than one field shares.
pub fn labeled(options: &[(&str, &str)]) -> Kind {
    Kind::Choice(
        options
            .iter()
            .map(|(value, key)| (value.to_string(), format!("i18n:{key}")))
            .collect(),
    )
}
fn option_label(value: &str) -> String {
    let key = match value {
        "All Caps" => "theme.option.allCaps",
        "Automatic" => "theme.option.automatic",
        "Background Blur" => "theme.option.backgroundBlur",
        "Background Invert" => "theme.option.backgroundInvert",
        "Bold" => "theme.gpui.bold",
        "Bottom Center" => "media.alignment.bottom.center",
        "Bottom Left" => "media.alignment.bottom.left",
        "Bottom Right" => "media.alignment.bottom.right",
        "CCLI" => "theme.linkedText.ccli",
        "Container up or down" => "theme.option.containerUpOrDown",
        "Countdown Timer" => "theme.linkedText.countdownTimer",
        "Current" => "theme.linkedText.slideCountSource.current",
        "Current Slide" => "theme.linkedText.currentSlide",
        "Current Slide Element" => "theme.linkedText.currentSlideElement",
        "Next Slide Element" => "theme.linkedText.nextSlideElement",
        "Video Countdown" => "theme.linkedText.videoCountdown",
        "Slide Timer" => "theme.linkedText.slideTimer",
        "Line Width" => "theme.option.lineWidth",
        "Max Line Width" => "theme.option.maxLineWidth",
        "natural" => "theme.text.alignNatural",
        "File" => "theme.linkedText.file",
        "Full Width" => "theme.option.fullWidth",
        "Group" => "theme.linkedText.group",
        "Hold" => "theme.gpui.option.hold",
        "Loop" => "announcements.loop",
        "Media Fill" => "theme.option.mediaFill",
        "Middle Center" => "media.alignment.middle.center",
        "Middle Left" => "media.alignment.middle.left",
        "Middle Right" => "media.alignment.middle.right",
        "Next" => "bible.next",
        "Next Slide" => "theme.linkedText.nextSlide",
        "Next Slide Label" => "theme.linkedText.nextSlideLabel",
        "Next Slide Notes" => "theme.linkedText.nextSlideNotes",
        "Next in Arrangement" => "theme.linkedText.groupSource.nextInArrangement",
        "None" => "theme.option.none",
        "Object" => "theme.objectLink.text",
        "Off Screen" => "theme.option.offScreen",
        "One character per line" => "theme.option.oneCharacterPerLine",
        "One word per line" => "theme.option.oneWordPerLine",
        "Operator Notes" => "theme.linkedText.operatorNotes",
        "Playlist Item" => "theme.linkedText.playlistItem",
        "Premultiplied" => "screenSettings.alphaPremultiplied",
        "Presentation Name" => "theme.linkedText.presentationName",
        "RSS" => "theme.linkedText.rss",
        "Regular" => "theme.option.regular",
        "Remaining" => "theme.linkedText.slideCountSource.remaining",
        "Remove line returns" => "theme.option.removeLineReturns",
        "Replace line returns" => "theme.option.replaceLineReturns",
        "Scale to Fill" => "media.scale.fill",
        "Scale to Fit" => "media.scale.fit",
        "Slide Count" => "theme.linkedText.slideCount",
        "Slide Label" => "theme.linkedText.slideLabel",
        "Slide Notes" => "theme.linkedText.slideNotes",
        "Stage Message" => "theme.linkedText.stageMessage",
        "Start Case" => "theme.option.startCase",
        "Stop" => "media.playback.stop",
        "Straight" => "screenSettings.alphaStraight",
        "Stretch to Fill" => "media.scale.stretch",
        "System Clock" => "theme.linkedText.systemClock",
        "Text Color" => "theme.option.textColor",
        "Text Cut Out" => "theme.option.textCutOut",
        "Text Gradient" => "theme.option.textGradient",
        "Text down" => "theme.option.textDown",
        "Text up" => "theme.option.textUp",
        "Text up or down" => "theme.option.textUpOrDown",
        "Ticker" => "theme.linkedText.ticker",
        "Timecode" => "theme.linkedText.timecode",
        "Title Case" => "theme.option.titleCase",
        "Top Center" => "media.alignment.top.center",
        "Top Left" => "media.alignment.top.left",
        "Top Right" => "media.alignment.top.right",
        "Total" => "theme.linkedText.slideCountSource.total",
        "after-previous" => "build.afterPrevious",
        "background-effect" => "theme.fillType.backgroundEffect",
        "blur" => "theme.blur",
        "bottom" => "media.crop.bottom",
        "box" => "theme.gpui.option.box",
        "capture-status-color" => "theme.fillType.captureStatusColor",
        "center" => "theme.text.tabCenter",
        "check" => "theme.gpui.option.check",
        "chord-chart" => "theme.fillType.chordChart",
        "circle" => "theme.gpui.option.circle",
        "color" => "theme.fillType.color",
        "current-slide-image" => "theme.fillType.currentSlideImage",
        "day-long-month" => "theme.gpui.option.day_long_month",
        "decimal" => "theme.text.tabDecimal",
        "description" => "theme.gpui.option.description",
        "diamond" => "theme.gpui.option.diamond",
        "disc" => "theme.gpui.option.disc",
        "down" => "ticker.down",
        "full" => "theme.gpui.option.full",
        "gradient" => "theme.fillType.gradient",
        "group-color" => "theme.fillType.groupColor",
        "hours-minutes" => "theme.gpui.option.hours_minutes",
        "hours-minutes-seconds" => "theme.gpui.option.hours_minutes_seconds",
        "hyphen" => "theme.text.listHyphen",
        "invert" => "theme.gpui.option.invert",
        "justify" => "theme.text.justify",
        "label-color" => "theme.fillType.labelColor",
        "left" => "theme.text.left",
        "long" => "theme.gpui.option.long",
        "long-month" => "theme.gpui.option.long_month",
        "lower-alpha" => "theme.gpui.option.lower_alpha",
        "lower-roman" => "theme.gpui.option.lower_roman",
        "media" => "theme.fillType.media",
        "middle" => "theme.gpui.option.middle",
        "ndi" => "ndi.tool",
        "next-slide-image" => "theme.fillType.nextSlideImage",
        "none" => "theme.option.none",
        "numeric" => "theme.gpui.option.numeric",
        "on-click" => "build.onClick",
        "paragraph" => "theme.gpui.option.paragraph",
        "remove_long" => "theme.gpui.option.remove_long",
        "remove_short" => "theme.gpui.option.remove_short",
        "right" => "theme.text.right",
        "screen-preview" => "theme.fillType.screenPreview",
        "seconds-zone-long" => "theme.gpui.option.seconds_zone_long",
        "seconds-zone-short" => "theme.gpui.option.seconds_zone_short",
        "short" => "theme.gpui.option.short",
        "short-month" => "theme.gpui.option.short_month",
        "slide-object" => "theme.fillType.slideObject",
        "solid" => "theme.stroke.solid",
        "square" => "theme.stroke.square",
        "title" => "theme.linkedText.rssTitle",
        "title-description" => "theme.gpui.option.title_description",
        "top" => "media.crop.top",
        "underline" => "theme.gpui.underline",
        "up" => "ticker.up",
        "upper-alpha" => "theme.gpui.option.upper_alpha",
        "upper-roman" => "theme.gpui.option.upper_roman",
        "video-input" => "theme.fillType.videoInput",
        "web" => "theme.fillType.web",
        "whole-object" => "build.delivery.wholeObject",
        _ => return value.to_string(),
    };
    format!("i18n:{key}")
}
pub fn fields(
    tab: &str,
    slide: &Value,
    index: Option<usize>,
    paragraph: Option<(usize, usize)>,
    transitions: &Value,
) -> Vec<Field> {
    let layer = index.map(|i| &slide["layers"][i]);
    let mut style = editing::style(slide, layer);
    if let Some((start, _)) = paragraph {
        if let Some(overrides) = style["textParagraphStyles"]
            .get(start)
            .and_then(Value::as_object)
            .cloned()
        {
            let absolute = overrides.get("textAbsoluteRightEdge").is_some();
            let inset = overrides.get("textRightIndent").is_some();
            let fields = style.as_object_mut().unwrap();
            fields.extend(overrides);
            // The override that sets one right-edge form replaces the inherited other form.
            if absolute {
                fields.remove("textRightIndent");
            } else if inset {
                fields.remove("textAbsoluteRightEdge");
            }
        }
    }
    let mut out = Vec::new();
    let Some(layer) = layer else {
        for (key, label, kind, value) in [
            (
                "slideName",
                "theme.slideName",
                Kind::Text,
                slide["name"].clone(),
            ),
            (
                "text",
                "theme.tool.text",
                Kind::Multiline,
                slide["text"].clone(),
            ),
        ] {
            out.push(Field::new(key, Target::Slide, kind, value).named(label));
        }
        out.push(
            Field::new(
                "size",
                Target::Slide,
                choices(&["1920x1080", "1280x720", "3840x2160"]),
                json!(format!(
                    "{}x{}",
                    slide["width"].as_u64().unwrap_or(1920),
                    slide["height"].as_u64().unwrap_or(1080)
                )),
            )
            .named("theme.size"),
        );
        return out;
    };
    match tab {
        "object" => {
            for key in ["name", "text", "webUrl"] {
                out.push(Field::new(
                    key,
                    Target::Layer,
                    if key == "text" {
                        Kind::Multiline
                    } else {
                        Kind::Text
                    },
                    if key == "text" && editing::is_text(layer) {
                        slide["text"].clone()
                    } else {
                        layer.get(key).cloned().unwrap_or(json!(""))
                    },
                ));
            }
            for key in ["hidden", "locked"] {
                out.push(Field::new(
                    key,
                    Target::Layer,
                    Kind::Bool,
                    json!(layer[key] == true),
                ));
            }
            let bounds = super::geometry::layer_bounds(layer, super::geometry::slide_size(slide));
            let bounds = bounds.value();
            for key in ["x", "y", "width", "height"] {
                out.push(Field::new(
                    key,
                    Target::Bounds,
                    Kind::Number(
                        if matches!(key, "width" | "height") {
                            8.
                        } else {
                            -100000.
                        },
                        100000.,
                    ),
                    bounds[key].clone(),
                ));
            }
        }
        "text" => {
            for (key, kind) in text_fields() {
                if !shown_text_field(&key, &style) {
                    continue;
                }
                let value = if key == "fontWeight" {
                    json!(if style["bold"] == true {
                        "Bold"
                    } else {
                        "Regular"
                    })
                } else if key == "superscript" {
                    json!(style["superscript"].as_i64().unwrap_or(0).to_string())
                } else if key == "textRightIndentMode" {
                    json!(if style["textAbsoluteRightEdge"].is_null() {
                        "inset"
                    } else {
                        "absolute"
                    })
                } else if let Some(value) = style.get(&key).filter(|value| !value.is_null()) {
                    value.clone()
                } else {
                    match kind {
                        Kind::Bool => json!(false),
                        Kind::Number(..) => {
                            if key == "textLineHeightMultiple" {
                                json!(1)
                            } else {
                                json!(0)
                            }
                        }
                        _ => json!(match key.as_str() {
                            "linkedTextGroupSource" => "Current Slide",
                            "linkedTextPlaylistItemSource" => "Current",
                            "linkedTextSlideCountSource" => "Current",
                            "linkedTextRssFormat" => "title-description",
                            "linkedTextRssDelimiter" => "•",
                            "linkedTextTimerFormatString" => "${timer}",
                            "systemClockFormatString" => "${clock}",
                            "lineTransformDelimiter" => " ",
                            "videoCountdownSource" => "presentation",
                            "linkedTextLineTransform" => "None",
                            "underlineStyle" | "strikethroughStyle" => "single",
                            "underlinePattern" | "strikethroughPattern" => "solid",
                            "underlineColor" | "strikethroughColor" => {
                                style["textColor"].as_str().unwrap_or("")
                            }
                            "linkedTextStageMessageFlashColor" => "#ffffff",
                            "linesOnlyStyle" => "Full Width",
                            _ => "",
                        }),
                    }
                };
                let field = Field::new(&key, Target::Text, kind, value);
                out.push(match text_label(&key) {
                    Some(label) => field.named(label),
                    None => field,
                });
            }
        }
        "shape" => {
            for (key, kind, default) in shape_fields() {
                let value = match key.as_str() {
                    "fillColor" | "fillStartColor" => layer["fillColors"][0]
                        .as_str()
                        .or_else(|| layer["fill"].as_str())
                        .map(|s| json!(s))
                        .unwrap_or(default),
                    "fillEndColor" => layer["fillColors"][1]
                        .as_str()
                        .or_else(|| layer["fillColors"][0].as_str())
                        .map(|s| json!(s))
                        .unwrap_or(default),
                    "shapeVisibilityEnabled" => json!(layer["hidden"] != true),
                    "shapeShadowEnabled" => json!(layer["shadowEnabled"] == true),
                    "shapeFeatheringEnabled" => json!(layer["featheringEnabled"] == true),
                    "fillEnabled" => json!(layer["fillType"] != "none"),
                    "strokeWidth" => json!(editing::stroke_width(layer)),
                    "strokeColor" => json!(editing::stroke_color(layer)),
                    "strokeEnabled" => json!(
                        layer["stroke"].as_str().is_some_and(|s| !s.is_empty())
                            && layer["strokeEnabled"] != false
                    ),
                    "captureIdleColor"
                    | "captureRecordingColor"
                    | "captureFinalizingColor"
                    | "captureErrorColor" => {
                        let status = match key.as_str() {
                            "captureIdleColor" => "idle",
                            "captureRecordingColor" => "recording",
                            "captureFinalizingColor" => "finalizing",
                            _ => "error",
                        };
                        layer["captureStatusColors"]
                            .get(status)
                            .cloned()
                            .unwrap_or(default)
                    }
                    _ => layer.get(&key).cloned().unwrap_or(default),
                };
                let field = Field::new(&key, Target::Shape, kind, value);
                out.push(if key == "featheringStyle" {
                    field.named("theme.feathering.style")
                } else {
                    field
                });
            }
        }
        "media" | "textMedia" => {
            let text = tab == "textMedia";
            let owner = if text { &style } else { layer };
            for (suffix, kind, default) in media_fields() {
                let key = format!("{}{}", if text { "mediaFill" } else { "media" }, suffix);
                out.push(Field::new(
                    &key,
                    if text { Target::Text } else { Target::Shape },
                    kind,
                    owner.get(&key).cloned().unwrap_or(default),
                ));
            }
            if !text {
                for (key, kind) in [
                    ("webUrl", Kind::Text),
                    ("previewScreenId", Kind::Text),
                    ("fillObjectId", Kind::Text),
                    (
                        "dataFillSource",
                        choices(&["Current Slide", "Next Slide", "Next in Arrangement"]),
                    ),
                ] {
                    out.push(Field::new(key, Target::Shape, kind, layer[key].clone()));
                }
            }
        }
        "build" => {
            if editing::is_text(layer) {
                out.push(Field::new(
                    "buildDelivery",
                    Target::Build,
                    choices(&["whole-object", "paragraph", "underline"]),
                    layer["textReveal"]
                        .get("mode")
                        .cloned()
                        .unwrap_or(json!("whole-object")),
                ));
            }
            for prefix in ["build", "buildOut"] {
                out.push(Field::new(
                    &format!("{prefix}Enabled"),
                    Target::Build,
                    Kind::Bool,
                    json!(layer[prefix].is_object()),
                ));
                if layer[prefix].is_object() {
                    for (suffix, kind, value) in [
                        (
                            "Step",
                            Kind::Number(1., 9_007_199_254_740_991.),
                            layer[prefix]["step"].clone(),
                        ),
                        (
                            "Start",
                            choices(&["on-click", "after-previous"]),
                            layer[prefix]["start"].clone(),
                        ),
                        (
                            "Delay",
                            Kind::Number(0., 100000.),
                            layer[prefix]["delaySeconds"].clone(),
                        ),
                    ] {
                        out.push(Field::new(
                            &format!("{prefix}{suffix}"),
                            Target::Build,
                            kind,
                            value,
                        ));
                    }
                    let mut effects = vec![
                        (
                            super::builds::CUT.into(),
                            "i18n:theme.gpui.option.cut".into(),
                        ),
                        (
                            super::builds::DISSOLVE.into(),
                            "i18n:theme.gpui.option.dissolve".into(),
                        ),
                    ];
                    if let Some(catalog) = transitions.as_array() {
                        for effect in catalog {
                            if let (Some(id), Some(name)) =
                                (effect["id"].as_str(), effect["name"].as_str())
                            {
                                effects.push((id.into(), name.into()));
                            }
                        }
                    }
                    out.push(Field::new(
                        &format!("{prefix}Transition"),
                        Target::Build,
                        Kind::Choice(effects),
                        layer[prefix]["transition"]
                            .get("effectId")
                            .cloned()
                            .unwrap_or(json!(super::builds::CUT)),
                    ));
                    if layer[prefix]["transition"]["effectId"]
                        .as_str()
                        .is_some_and(|id| id != super::builds::CUT)
                    {
                        out.push(Field::new(
                            &format!("{prefix}TransitionDuration"),
                            Target::Build,
                            Kind::Number(0., 10.),
                            layer[prefix]["transition"]["durationSeconds"].clone(),
                        ));
                    }
                    if let Some(effect) = transitions.as_array().and_then(|effects| {
                        effects
                            .iter()
                            .find(|effect| effect["id"] == layer[prefix]["transition"]["effectId"])
                    }) {
                        let directions = effect["directions"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>();
                        if !directions.is_empty() {
                            out.push(Field::new(
                                &format!("{prefix}TransitionDirection"),
                                Target::Build,
                                choices(&directions),
                                layer[prefix]["transition"]
                                    .get("direction")
                                    .cloned()
                                    .unwrap_or(effect["defaultDirection"].clone()),
                            ));
                        }
                        if let Some(variables) = effect["variables"].as_array() {
                            for var in variables {
                                let Some(name) = var["name"].as_str() else {
                                    continue;
                                };
                                let kind = if var["type"] == "color" {
                                    Kind::Color
                                } else {
                                    Kind::Number(
                                        var["min"].as_f64().unwrap_or(0.),
                                        var["max"].as_f64().unwrap_or(100.),
                                    )
                                };
                                let value = layer[prefix]["transition"]["variables"]
                                    .get(name)
                                    .cloned()
                                    .unwrap_or(var["value"].clone());
                                out.push(
                                    Field::new(
                                        &format!("{prefix}Variable:{name}"),
                                        Target::Build,
                                        kind,
                                        value,
                                    )
                                    .named(var["label"].as_str().unwrap_or(name)),
                                );
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

const LINE_TRANSFORMS: [&str; 5] = [
    "None",
    "Remove line returns",
    "Replace line returns",
    "One word per line",
    "One character per line",
];
/// The labels of the fields that have no `theme.gpui.*` key of their own: they use the renderer's.
fn text_label(key: &str) -> Option<&'static str> {
    Some(match key {
        "letterSpacing" => "theme.text.letterSpacing",
        "superscript" => "theme.text.superscript",
        "textMarginLeft" => "theme.text.marginLeft",
        "textMarginRight" => "theme.text.marginRight",
        "textMarginTop" => "theme.text.marginTop",
        "textMarginBottom" => "theme.text.marginBottom",
        "textRightIndentMode" => "theme.text.rightIndentMode",
        "textAbsoluteRightEdge" => "theme.text.absoluteRightEdge",
        "lineTransformDelimiter" => "theme.text.lineTransformDelimiter",
        "linesOnlyStyle" => "theme.text.style",
        "linesOnlyHeightOffset" => "theme.text.mask.heightOffset",
        "linesOnlyVerticalOffset" => "theme.text.mask.verticalOffset",
        "linesOnlyWidthOffset" => "theme.text.mask.widthOffset",
        "linesOnlyHorizontalOffset" => "theme.text.mask.horizontalOffset",
        "underlineStyle" | "strikethroughStyle" => "theme.text.style",
        "underlinePattern" | "strikethroughPattern" => "theme.text.decoration.pattern",
        "underlineByWord" | "strikethroughByWord" => "theme.text.decoration.byWord",
        "underlineColor" | "strikethroughColor" => "theme.color",
        "systemClockFormatString" | "linkedTextTimerFormatString" => {
            "theme.linkedText.textTemplate"
        }
        "videoCountdownSource" => "theme.linkedText.videoSource",
        "videoCountdownIgnoreLooping" => "theme.linkedText.ignoreLooping",
        "linkedTextPlaylistItemSource" => "theme.linkedText.playlistItem",
        "linkedTextPlaylistItemShowArrangement" => "theme.linkedText.showArrangement",
        "linkedTextElementName" => "theme.linkedText.elementName",
        "linkedTextLineTransform" => "theme.linkedText.transform",
        "preserveNotesFormat" => "theme.linkedText.preserveNotesFormat",
        "linkedTextStageMessageFlash" => "theme.linkedText.stageMessageFlash",
        "linkedTextStageMessageFlashColor" => "theme.linkedText.stageMessageFlashColor",
        _ => return None,
    })
}
/// The renderer shows the controls of a linked source, the delimiter and the right-edge form only where they
/// apply; the fields added with the ProPresenter fidelity work follow the same rules.
fn shown_text_field(key: &str, style: &Value) -> bool {
    let source = style["linkedTextSource"].as_str().unwrap_or("");
    match key {
        "linkedTextObjectId" => source == "Object",
        "linkedTextTimerId" | "linkedTextTimerFormatString" => matches!(
            source,
            "Countdown Timer" | "Slide Timer" | "Video Countdown"
        ),
        "linkedTextGroupSource" => source == "Group",
        "linkedTextPlaylistItemSource" => source == "Playlist Item",
        "linkedTextSlideCountSource" => source == "Slide Count",
        "linkedTextFileContent" | "linkedTextFilePath" => source == "File",
        key if key.starts_with("linkedTextRss") => source == "RSS",
        "systemClockDateFormat" | "systemClockTimeFormat" | "systemClockUse24Hour" => {
            source == "System Clock"
        }
        "lineTransformDelimiter" => style["lineTransform"] == "Replace line returns",
        "systemClockFormatString" => source == "System Clock",
        "videoCountdownSource" | "videoCountdownIgnoreLooping" => source == "Video Countdown",
        "linkedTextPlaylistItemShowArrangement" => source == "Playlist Item",
        "linkedTextElementName" => matches!(source, "Current Slide Element" | "Next Slide Element"),
        "preserveNotesFormat" => matches!(source, "Slide Notes" | "Next Slide Notes"),
        "linkedTextLineTransform" => matches!(
            source,
            "Current Slide"
                | "Next Slide"
                | "Object"
                | "Current Slide Element"
                | "Next Slide Element"
                | "Slide Notes"
                | "Next Slide Notes"
        ),
        "linkedTextStageMessageFlash" | "linkedTextStageMessageFlashColor" => {
            source == "Stage Message"
        }
        "textAbsoluteRightEdge" => !style["textAbsoluteRightEdge"].is_null(),
        "textRightIndent" => style["textAbsoluteRightEdge"].is_null(),
        _ => true,
    }
}
/// The paragraph fields of one note paragraph, resolved against the notes style. Their keys carry a `note:` prefix so
/// the inspector's field state does not mix them up with the selected object's fields of the same name.
pub fn note_fields(slide: &Value, paragraph: usize) -> Vec<Field> {
    let pseudo =
        json!({"layers": [{"id": "notes", "type": "Text", "textStyle": slide["notesTextStyle"]}]});
    fields(
        "text",
        &pseudo,
        Some(0),
        Some((paragraph, paragraph)),
        &json!([]),
    )
    .into_iter()
    .filter(|field| {
        editing::PARAGRAPH_FIELDS.contains(&field.key.as_str())
            && !matches!(field.key.as_str(), "textTabStops" | "textBarTabStops")
            || field.key == "textRightIndentMode"
    })
    .map(|mut field| {
        field.target = Target::Note;
        field.key = format!("note:{}", field.key);
        field
    })
    .collect()
}
fn decoration_styles() -> Kind {
    labeled(&[
        ("single", "theme.text.decoration.single"),
        ("thick", "theme.text.decoration.thick"),
        ("double", "theme.text.decoration.double"),
    ])
}
fn decoration_patterns() -> Kind {
    labeled(&[
        ("solid", "theme.text.decoration.solid"),
        ("dot", "theme.text.decoration.dot"),
        ("dash", "theme.text.decoration.dash"),
        ("dash-dot", "theme.text.decoration.dash-dot"),
        ("dash-dot-dot", "theme.text.decoration.dash-dot-dot"),
    ])
}
fn text_fields() -> Vec<(String, Kind)> {
    vec![
        ("fontFamily".into(), Kind::Text),
        ("fontWeight".into(), choices(&["Regular", "Bold"])),
        ("fontSize".into(), Kind::Number(8., 260.)),
        (
            "capitalization".into(),
            choices(&["None", "All Caps", "Start Case", "Title Case"]),
        ),
        (
            "scaling".into(),
            choices(&[
                "None",
                "Container up or down",
                "Text up",
                "Text down",
                "Text up or down",
            ]),
        ),
        ("lineTransform".into(), choices(&LINE_TRANSFORMS)),
        ("lineTransformDelimiter".into(), Kind::Text),
        (
            "textHorizontalAlign".into(),
            choices(&["left", "center", "right", "justify", "natural"]),
        ),
        (
            "textVerticalAlign".into(),
            choices(&["top", "middle", "bottom"]),
        ),
        ("textLineHeightMultiple".into(), Kind::Number(0.5, 3.)),
        ("textLineSpacing".into(), Kind::Number(0., 1000.)),
        ("textMinimumLineHeight".into(), Kind::Number(0., 1000.)),
        ("textMaximumLineHeight".into(), Kind::Number(0., 1000.)),
        ("textParagraphSpacingBefore".into(), Kind::Number(0., 1000.)),
        ("textParagraphSpacingAfter".into(), Kind::Number(0., 1000.)),
        ("textFirstLineIndent".into(), Kind::Number(-1000., 1000.)),
        ("textLeftIndent".into(), Kind::Number(0., 1000.)),
        (
            "textRightIndentMode".into(),
            labeled(&[
                ("inset", "theme.text.rightIndent"),
                ("absolute", "theme.text.absoluteRightEdge"),
            ]),
        ),
        ("textRightIndent".into(), Kind::Number(0., 1000.)),
        ("textAbsoluteRightEdge".into(), Kind::Number(0.01, 100000.)),
        ("textMarginLeft".into(), Kind::Number(0., 100000.)),
        ("textMarginRight".into(), Kind::Number(0., 100000.)),
        ("textMarginTop".into(), Kind::Number(0., 100000.)),
        ("textMarginBottom".into(), Kind::Number(0., 100000.)),
        ("textDefaultTabInterval".into(), Kind::Number(1., 2000.)),
        ("textListLevel".into(), Kind::Number(0., 8.)),
        ("letterSpacing".into(), Kind::Number(-1000., 1000.)),
        (
            "superscript".into(),
            labeled(&[
                ("0", "theme.option.none"),
                ("1", "theme.text.superscript.super"),
                ("-1", "theme.text.superscript.sub"),
            ]),
        ),
        (
            "textFillMode".into(),
            choices(&[
                "Text Color",
                "Text Gradient",
                "Text Cut Out",
                "Media Fill",
                "Background Invert",
                "Background Blur",
            ]),
        ),
        ("textColor".into(), Kind::Color),
        ("textGradientStart".into(), Kind::Color),
        ("textGradientEnd".into(), Kind::Color),
        ("textGradientAngle".into(), Kind::Number(0., 360.)),
        ("textGradientUseObjectBounds".into(), Kind::Bool),
        ("backgroundBlurSaturation".into(), Kind::Number(0., 200.)),
        ("backgroundBlurAmount".into(), Kind::Number(0., 100.)),
        ("strokeEnabled".into(), Kind::Bool),
        ("strokeColor".into(), Kind::Color),
        ("strokeWidth".into(), Kind::Number(0., 30.)),
        ("shadowEnabled".into(), Kind::Bool),
        ("shadowColor".into(), Kind::Color),
        ("shadowOpacity".into(), Kind::Number(0., 100.)),
        ("shadowAngle".into(), Kind::Number(0., 360.)),
        ("shadowOffset".into(), Kind::Number(0., 80.)),
        ("shadowBlur".into(), Kind::Number(0., 80.)),
        ("linesOnlyEnabled".into(), Kind::Bool),
        (
            "linesOnlyStyle".into(),
            choices(&["Full Width", "Line Width", "Max Line Width"]),
        ),
        (
            "linesOnlyHeightOffset".into(),
            Kind::Number(-100000., 100000.),
        ),
        (
            "linesOnlyVerticalOffset".into(),
            Kind::Number(-100000., 100000.),
        ),
        (
            "linesOnlyWidthOffset".into(),
            Kind::Number(-100000., 100000.),
        ),
        (
            "linesOnlyHorizontalOffset".into(),
            Kind::Number(-100000., 100000.),
        ),
        ("scrollingEnabled".into(), Kind::Bool),
        (
            "scrollDirection".into(),
            choices(&["left", "right", "up", "down"]),
        ),
        ("scrollStart".into(), choices(&["Automatic", "Off Screen"])),
        ("scrollSpeed".into(), Kind::Number(0., 100.)),
        ("scrollRepeat".into(), Kind::Bool),
        ("scrollRepeatDistance".into(), Kind::Number(0., 2000.)),
        ("scrollFeatherLeft".into(), Kind::Number(0., 960.)),
        ("scrollFeatherRight".into(), Kind::Number(0., 960.)),
        ("scrollFeatherLinked".into(), Kind::Bool),
        ("linkedTextEnabled".into(), Kind::Bool),
        (
            "linkedTextSource".into(),
            choices(&[
                "Object",
                "None",
                "Current Slide",
                "Next Slide",
                "Slide Notes",
                "Next Slide Notes",
                "Current Slide Element",
                "Next Slide Element",
                "System Clock",
                "Timecode",
                "Countdown Timer",
                "Video Countdown",
                "Slide Timer",
                "Group",
                "Slide Count",
                "Presentation Name",
                "Operator Notes",
                "Playlist Item",
                "CCLI",
                "Stage Message",
                "Slide Label",
                "Next Slide Label",
                "Ticker",
                "File",
                "RSS",
            ]),
        ),
        ("linkedTextTimerId".into(), Kind::Text),
        ("linkedTextTimerName".into(), Kind::Text),
        ("linkedTextTimerFormatString".into(), Kind::Text),
        ("systemClockFormatString".into(), Kind::Text),
        (
            "videoCountdownSource".into(),
            labeled(&[
                ("presentation", "theme.linkedText.videoPresentation"),
                ("announcements", "theme.linkedText.videoAnnouncements"),
            ]),
        ),
        ("videoCountdownIgnoreLooping".into(), Kind::Bool),
        ("linkedTextElementName".into(), Kind::Text),
        (
            "linkedTextLineTransform".into(),
            choices(
                &LINE_TRANSFORMS
                    .into_iter()
                    .filter(|value| *value != "Replace line returns")
                    .collect::<Vec<_>>(),
            ),
        ),
        ("preserveNotesFormat".into(), Kind::Bool),
        ("linkedTextStageMessageFlash".into(), Kind::Bool),
        ("linkedTextStageMessageFlashColor".into(), Kind::Color),
        ("linkedTextObjectId".into(), Kind::Text),
        (
            "linkedTextGroupSource".into(),
            choices(&["Current Slide", "Next Slide", "Next in Arrangement"]),
        ),
        (
            "linkedTextPlaylistItemSource".into(),
            labeled(&[
                ("Current", "theme.linkedText.playlist.Current"),
                ("Next", "theme.linkedText.playlist.Next"),
                ("Current Header", "theme.linkedText.playlist.Current Header"),
                ("Next Header", "theme.linkedText.playlist.Next Header"),
                (
                    "Parent Playlist",
                    "theme.linkedText.playlist.Parent Playlist",
                ),
            ]),
        ),
        ("linkedTextPlaylistItemShowArrangement".into(), Kind::Bool),
        (
            "linkedTextSlideCountSource".into(),
            choices(&["Current", "Remaining", "Total"]),
        ),
        ("linkedTextFileContent".into(), Kind::Readonly),
        ("linkedTextFilePath".into(), Kind::Readonly),
        ("linkedTextRssContent".into(), Kind::Readonly),
        ("linkedTextRssDescriptionSeparateLine".into(), Kind::Bool),
        ("linkedTextRssDelimiter".into(), Kind::Text),
        (
            "linkedTextRssFormat".into(),
            choices(&["title-description", "title"]),
        ),
        ("linkedTextRssTitleSeparateLine".into(), Kind::Bool),
        ("linkedTextRssUrl".into(), Kind::Text),
        (
            "systemClockDateFormat".into(),
            choices(&[
                "none",
                "numeric",
                "short-month",
                "long-month",
                "day-long-month",
                "full",
            ]),
        ),
        (
            "systemClockTimeFormat".into(),
            choices(&[
                "none",
                "hours-minutes",
                "hours-minutes-seconds",
                "seconds-zone-short",
                "seconds-zone-long",
            ]),
        ),
        ("systemClockUse24Hour".into(), Kind::Bool),
        ("bold".into(), Kind::Bool),
        ("italic".into(), Kind::Bool),
        ("underline".into(), Kind::Bool),
        ("strikethrough".into(), Kind::Bool),
        ("underlineStyle".into(), decoration_styles()),
        ("underlinePattern".into(), decoration_patterns()),
        ("underlineByWord".into(), Kind::Bool),
        ("underlineColor".into(), Kind::Color),
        ("strikethroughStyle".into(), decoration_styles()),
        ("strikethroughPattern".into(), decoration_patterns()),
        ("strikethroughByWord".into(), Kind::Bool),
        ("strikethroughColor".into(), Kind::Color),
        (
            "textListStyle".into(),
            choices(&[
                "none",
                "box",
                "check",
                "diamond",
                "disc",
                "circle",
                "square",
                "hyphen",
                "decimal",
                "lower-alpha",
                "upper-alpha",
                "lower-roman",
                "upper-roman",
            ]),
        ),
        ("textListPrefix".into(), Kind::Text),
        ("textListPostfix".into(), Kind::Text),
        ("textListStart".into(), Kind::Number(1., 9999.)),
    ]
}
fn shape_fields() -> Vec<(String, Kind, Value)> {
    vec![
        ("fillEnabled".into(), Kind::Bool, json!(true)),
        (
            "fillType".into(),
            choices(&[
                "color",
                "gradient",
                "media",
                "ndi",
                "web",
                "video-input",
                "background-effect",
                "slide-object",
                "current-slide-image",
                "next-slide-image",
                "screen-preview",
                "chord-chart",
                "group-color",
                "label-color",
                "capture-status-color",
            ]),
            json!("color"),
        ),
        ("fillColor".into(), Kind::Color, json!("#ffffff")),
        ("fillStartColor".into(), Kind::Color, json!("#0b63f6")),
        ("fillEndColor".into(), Kind::Color, json!("#14c8e8")),
        ("fillAngle".into(), Kind::Number(0., 360.), json!(0)),
        ("strokeEnabled".into(), Kind::Bool, json!(true)),
        ("strokeWidth".into(), Kind::Number(0., 1000.), json!(1)),
        ("strokeColor".into(), Kind::Color, json!("#ffffff")),
        (
            "strokeStyle".into(),
            choices(&["solid", "square", "short", "long"]),
            json!("solid"),
        ),
        ("opacity".into(), Kind::Number(0., 100.), json!(100)),
        ("shapeVisibilityEnabled".into(), Kind::Bool, json!(true)),
        ("shapeShadowEnabled".into(), Kind::Bool, json!(false)),
        ("shadowColor".into(), Kind::Color, json!("#000000")),
        ("shadowOpacity".into(), Kind::Number(0., 100.), json!(75)),
        ("shadowAngle".into(), Kind::Number(0., 360.), json!(315)),
        ("shadowOffset".into(), Kind::Number(0., 100.), json!(5)),
        ("shadowBlur".into(), Kind::Number(0., 100.), json!(5)),
        ("shapeFeatheringEnabled".into(), Kind::Bool, json!(false)),
        (
            "featheringStyle".into(),
            labeled(&[
                ("inside", "theme.feathering.inside"),
                ("center", "theme.feathering.center"),
                ("outside", "theme.feathering.outside"),
            ]),
            json!("inside"),
        ),
        ("featheringAmount".into(), Kind::Number(0., 100.), json!(5)),
        (
            "backgroundEffect".into(),
            choices(&["blur", "invert"]),
            json!("blur"),
        ),
        (
            "backgroundBlurAmount".into(),
            Kind::Number(0., 100.),
            json!(14),
        ),
        (
            "backgroundBlurSaturation".into(),
            Kind::Number(0., 200.),
            json!(100),
        ),
        ("captureIdleColor".into(), Kind::Color, json!("#000000")),
        (
            "captureRecordingColor".into(),
            Kind::Color,
            json!("#00aa00"),
        ),
        (
            "captureFinalizingColor".into(),
            Kind::Color,
            json!("#ffaa00"),
        ),
        ("captureErrorColor".into(), Kind::Color, json!("#ff0000")),
    ]
}
fn media_fields() -> Vec<(String, Kind, Value)> {
    vec![
        ("Title".into(), Kind::Text, json!("")),
        (
            "Scale".into(),
            choices(&["Scale to Fit", "Scale to Fill", "Stretch to Fill"]),
            json!("Scale to Fit"),
        ),
        ("FlipX".into(), Kind::Bool, json!(false)),
        ("FlipY".into(), Kind::Bool, json!(false)),
        ("Crop".into(), Kind::Bool, json!(false)),
        ("CropTop".into(), Kind::Number(0., 100000.), json!(0)),
        ("CropRight".into(), Kind::Number(0., 100000.), json!(0)),
        ("CropBottom".into(), Kind::Number(0., 100000.), json!(0)),
        ("CropLeft".into(), Kind::Number(0., 100000.), json!(0)),
        ("Rotation".into(), Kind::Number(0., 360.), json!(0)),
        (
            "Playback".into(),
            choices(&["Loop", "Stop", "Hold"]),
            json!("Loop"),
        ),
        ("Volume".into(), Kind::Number(0., 100.), json!(100)),
        ("RampIn".into(), Kind::Number(0., 100000.), json!(0)),
        ("RampOut".into(), Kind::Number(0., 100000.), json!(0)),
        ("Hue".into(), Kind::Number(-180., 180.), json!(0)),
        ("Saturation".into(), Kind::Number(0., 2.), json!(1)),
        ("Brightness".into(), Kind::Number(-1., 1.), json!(0)),
        ("Contrast".into(), Kind::Number(0., 2.), json!(1)),
    ]
}
