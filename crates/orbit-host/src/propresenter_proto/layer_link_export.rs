//! ProPresenter `Slide.Element.DataLink` encoding for Orbit's layered slide fields.

use super::*;

fn has_field(fields: &[Field], number: u32) -> bool {
    fields.iter().any(|field| field.number == number)
}

fn link(number: u32, message: Vec<u8>) -> Vec<u8> {
    let mut value = Vec::new();
    put_message(number, message, &mut value);
    value
}

fn nonempty_string(value: &Value) -> Option<&str> {
    value.as_str().filter(|value| !value.is_empty())
}

pub(super) fn string_field_including_empty(fields: &[Field], number: u32) -> Option<String> {
    fields.iter().find_map(|field| {
        if field.number != number {
            return None;
        }
        match &field.value {
            WireValue::Bytes(value) => String::from_utf8(value.clone()).ok(),
            _ => None,
        }
    })
}

fn string_or_default<'a>(value: &'a Value, key: &str, default: &'a str) -> Option<&'a str> {
    match value.get(key) {
        None => Some(default),
        Some(Value::String(value)) => Some(value),
        _ => None,
    }
}

fn data_color_source(value: Option<&str>, group: bool) -> Option<u64> {
    match value? {
        "Current Slide" => Some(0),
        "Next Slide" => Some(1),
        "Next in Arrangement" if group => Some(2),
        _ => None,
    }
}

fn fill_link(layer: &Value) -> Option<Vec<u8>> {
    let mut value = Vec::new();
    match layer["fillType"].as_str()? {
        "screen-preview" => {
            put_uuid(1, nonempty_string(&layer["previewScreenId"])?, &mut value);
            if let Some(name) = layer["fillTitle"].as_str() {
                put_string(2, name, &mut value);
            }
            Some(link(6, value))
        }
        "current-slide-image" | "next-slide-image" => {
            put_varint(
                1,
                u64::from(layer["fillType"] == "next-slide-image"),
                &mut value,
            );
            Some(link(13, value))
        }
        "slide-object" => {
            put_uuid(1, nonempty_string(&layer["fillObjectId"])?, &mut value);
            if let Some(name) = layer["fillTitle"].as_str() {
                put_string(2, name, &mut value);
            }
            Some(link(8, value))
        }
        "group-color" | "label-color" => {
            let group = layer["fillType"] == "group-color";
            put_varint(
                1,
                data_color_source(layer["dataFillSource"].as_str(), group)?,
                &mut value,
            );
            Some(link(if group { 16 } else { 26 }, value))
        }
        "capture-status-color" => Some(link(21, value)),
        _ => None,
    }
}

/// Whether `fillType` is represented by a native data link rather than a Graphics.Fill.
pub(super) fn has_fill_data_link(layer: &Value) -> bool {
    fill_link(layer).is_some()
}

fn line_transform(value: Option<&str>) -> Option<u64> {
    match value.unwrap_or("None") {
        "None" | "Replace line returns" => Some(0),
        "Remove line returns" => Some(1),
        "One word per line" => Some(2),
        "One character per line" => Some(3),
        _ => None,
    }
}

fn clock_format(style: &Value) -> Option<Vec<u8>> {
    let date = match string_or_default(style, "systemClockDateFormat", "numeric")? {
        "none" => 0,
        "numeric" => 1,
        "short-month" => 2,
        "long-month" | "day-long-month" => 3,
        "full" => 4,
        _ => return None,
    };
    let time = match string_or_default(style, "systemClockTimeFormat", "hours-minutes")? {
        "none" => 0,
        "hours-minutes" => 1,
        "hours-minutes-seconds" => 2,
        "seconds-zone-short" => 3,
        "seconds-zone-long" => 4,
        _ => return None,
    };
    let military_time = match style.get("systemClockUse24Hour") {
        None => false,
        Some(Value::Bool(value)) => *value,
        _ => return None,
    };
    let mut format = Vec::new();
    put_varint(2, date, &mut format);
    put_varint(3, time, &mut format);
    put_varint(4, u64::from(military_time), &mut format);
    Some(format)
}

fn timer_style_number(value: &str) -> Option<u64> {
    match value {
        "none" => Some(0),
        "short" => Some(1),
        "long" => Some(2),
        "remove_short" => Some(3),
        "remove_long" => Some(4),
        _ => None,
    }
}

fn timer_format_message(value: &Value) -> Option<Vec<u8>> {
    let format = value.as_object()?;
    let mut message = Vec::new();
    for (key, field) in [
        ("hour", 1),
        ("minute", 2),
        ("second", 3),
        ("millisecond", 4),
    ] {
        if let Some(value) = format.get(key).filter(|value| !value.is_null()) {
            put_varint(field, timer_style_number(value.as_str()?)?, &mut message);
        }
    }
    for (key, field) in [
        ("wallClock", 5),
        ("use24Hour", 6),
        ("millisecondsUnderMinuteOnly", 7),
    ] {
        if let Some(value) = format.get(key).filter(|value| !value.is_null()) {
            put_varint(field, u64::from(value.as_bool()?), &mut message);
        }
    }
    let allowed = [
        "hour",
        "minute",
        "second",
        "millisecond",
        "wallClock",
        "use24Hour",
        "millisecondsUnderMinuteOnly",
    ];
    format
        .keys()
        .all(|key| allowed.contains(&key.as_str()))
        .then_some(message)
}

pub(super) fn timer_color_trigger(value: &Value) -> Option<Vec<u8>> {
    let fields = value.as_object()?;
    if fields
        .keys()
        .any(|key| !matches!(key.as_str(), "time" | "color" | "sourceColorComponents"))
    {
        return None;
    }
    let time = value
        .get("time")?
        .as_f64()
        .filter(|value| value.is_finite())?;
    let color = value.get("color")?.as_str()?;
    crate::native_compositor::parse_color(color)?;
    let mut trigger = Vec::new();
    put_f64(1, time, &mut trigger);
    let raw = match value.get("sourceColorComponents") {
        Some(value) => {
            let values = value.as_array().filter(|values| values.len() == 4)?;
            Some(
                values
                    .iter()
                    .map(|value| {
                        value
                            .as_f64()
                            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
                            .map(|value| value as f32)
                    })
                    .collect::<Option<Vec<_>>>()?,
            )
        }
        None => None,
    };
    if let Some(raw) = raw.filter(|raw| {
        let mut source = Vec::new();
        for (index, channel) in raw.iter().enumerate() {
            put_f32(index as u32 + 1, *channel, &mut source);
        }
        decode_message(&source)
            .ok()
            .and_then(|fields| super::color(&fields))
            .as_deref()
            == Some(color)
    }) {
        let mut source = Vec::new();
        for (index, channel) in raw.iter().enumerate() {
            put_f32(index as u32 + 1, *channel, &mut source);
        }
        put_message(2, source, &mut trigger);
    } else {
        put_color(2, color, &mut trigger);
    }
    Some(trigger)
}

fn timer_color_triggers(style: &Value) -> Option<Vec<Vec<u8>>> {
    style
        .get("linkedTextTimerColorTriggers")
        .filter(|value| !value.is_null())?
        .as_array()?
        .iter()
        .map(timer_color_trigger)
        .collect()
}

pub(super) fn decode_timer_trigger_color(fields: &[Field]) -> Result<(String, [f32; 4]), String> {
    if fields.iter().any(|field| !(1..=4).contains(&field.number)) {
        return Err("timer color has unmodeled fields".into());
    }
    let mut channels = [0.0_f32, 0.0, 0.0, 1.0];
    for (index, channel) in channels.iter_mut().enumerate() {
        let number = index as u32 + 1;
        if !has_field(fields, number) {
            continue;
        }
        if fields.iter().filter(|field| field.number == number).count() > 1 {
            return Err("timer color component occurs more than once".into());
        }
        let Some(value) = f32_field(fields, number)
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        else {
            return Err("timer color component is invalid".into());
        };
        *channel = value;
    }
    let color = super::color(fields).ok_or("timer color is invalid")?;
    Ok((color, channels))
}

fn timer_style_name(value: u64) -> Option<&'static str> {
    match value {
        0 => Some("none"),
        1 => Some("short"),
        2 => Some("long"),
        3 => Some("remove_short"),
        4 => Some("remove_long"),
        _ => None,
    }
}

fn decode_timer_format(fields: &[Field]) -> (Value, Vec<String>) {
    let mut format = Map::new();
    let mut unsupported = Vec::new();
    for (number, key) in [
        (1, "hour"),
        (2, "minute"),
        (3, "second"),
        (4, "millisecond"),
    ] {
        if !has_field(fields, number) {
            continue;
        }
        if fields.iter().filter(|field| field.number == number).count() > 1 {
            unsupported.push(format!("timer {key} format occurs more than once"));
        }
        match varint_field(fields, number).and_then(timer_style_name) {
            Some(value) => {
                format.insert(key.into(), json!(value));
            }
            None => unsupported.push(format!("timer {key} format is invalid or unsupported")),
        }
    }
    for (number, key) in [
        (5, "wallClock"),
        (6, "use24Hour"),
        (7, "millisecondsUnderMinuteOnly"),
    ] {
        if !has_field(fields, number) {
            continue;
        }
        if fields.iter().filter(|field| field.number == number).count() > 1 {
            unsupported.push(format!("timer {key} format occurs more than once"));
        }
        match bool_field(fields, number) {
            Some(value) => {
                format.insert(key.into(), json!(value));
            }
            None => unsupported.push(format!("timer {key} format has an invalid wire type")),
        }
    }
    for field in fields
        .iter()
        .filter(|field| !(1..=7).contains(&field.number))
    {
        unsupported.push(format!(
            "timer format field {} is not represented by Orbit",
            field.number
        ));
    }
    (Value::Object(format), unsupported)
}

fn encode_url(value: &str) -> Vec<u8> {
    let mut url = Vec::new();
    put_string(1, value, &mut url);
    url
}

fn text_link(style: &Value) -> Option<Vec<u8>> {
    if style["linkedTextEnabled"] == false {
        return None;
    }
    let source = style["linkedTextSource"].as_str()?;
    let mut value = Vec::new();
    match source {
        "Object" => {
            put_uuid(
                1,
                nonempty_string(&style["linkedTextObjectId"])?,
                &mut value,
            );
            if let Some(name) = style["linkedTextElementName"].as_str() {
                put_string(2, name, &mut value);
            }
            put_varint(
                4,
                line_transform(style["linkedTextLineTransform"].as_str())?,
                &mut value,
            );
            Some(link(2, value))
        }
        "Current Slide"
        | "Next Slide"
        | "Slide Notes"
        | "Next Slide Notes"
        | "Current Slide Element"
        | "Next Slide Element" => {
            let (slide, option) = match source {
                "Current Slide" => (0, 0),
                "Next Slide" => (1, 0),
                "Slide Notes" => (0, 1),
                "Next Slide Notes" => (1, 1),
                "Current Slide Element" => (0, 2),
                "Next Slide Element" => (1, 2),
                _ => unreachable!(),
            };
            put_varint(1, slide, &mut value);
            put_varint(2, option, &mut value);
            if option == 2 {
                put_string(
                    4,
                    nonempty_string(&style["linkedTextElementName"])?,
                    &mut value,
                );
            }
            if style["preserveNotesFormat"].as_bool() == Some(true) {
                put_varint(3, 1, &mut value);
            }
            put_varint(
                5,
                line_transform(style["linkedTextLineTransform"].as_str())?,
                &mut value,
            );
            Some(link(10, value))
        }
        "System Clock" => {
            if let Some(template) = style["systemClockFormatString"].as_str() {
                put_string(1, template, &mut value);
            }
            put_message(5, clock_format(style)?, &mut value);
            Some(link(4, value))
        }
        "Countdown Timer" => {
            put_uuid(1, nonempty_string(&style["linkedTextTimerId"])?, &mut value);
            if let Some(name) = style
                .get("linkedTextTimerName")
                .filter(|name| !name.is_null())
            {
                put_string(2, name.as_str()?, &mut value);
            }
            if let Some(format) = style
                .get("linkedTextTimerFormat")
                .filter(|format| !format.is_null())
            {
                put_message(3, timer_format_message(format)?, &mut value);
            }
            if let Some(format) = style
                .get("linkedTextTimerFormatString")
                .filter(|format| !format.is_null())
            {
                put_string(4, format.as_str()?, &mut value);
            }
            if let Some(triggers) = timer_color_triggers(style) {
                for trigger in triggers {
                    put_message(5, trigger, &mut value);
                }
            } else if style
                .get("linkedTextTimerColorTriggers")
                .is_some_and(|triggers| !triggers.is_null())
            {
                return None;
            }
            Some(link(3, value))
        }
        "Video Countdown" => {
            if let Some(format) = style.get("linkedTextTimerFormat") {
                put_message(1, timer_format_message(format)?, &mut value);
            }
            if let Some(template) = style["linkedTextTimerFormatString"].as_str() {
                put_string(2, template, &mut value);
            }
            if style.get("linkedTextTimerColorTriggers").is_some() {
                for trigger in timer_color_triggers(style)? {
                    put_message(3, trigger, &mut value);
                }
            }
            put_varint(
                4,
                u64::from(
                    style["videoCountdownIgnoreLooping"]
                        .as_bool()
                        .unwrap_or(false),
                ),
                &mut value,
            );
            let source = match string_or_default(style, "videoCountdownSource", "presentation")? {
                "presentation" => 0,
                "announcements" => 1,
                _ => return None,
            };
            put_varint(5, source, &mut value);
            Some(link(12, value))
        }
        "Slide Timer" => {
            if let Some(format) = style.get("linkedTextTimerFormat") {
                put_message(2, timer_format_message(format)?, &mut value);
            }
            Some(link(19, value))
        }
        "Timecode" => Some(link(33, value)),
        "Planning Center Live"
            if string_or_default(style, "linkedTextPlanningCenterLiveSource", "Current Item")?
                == "Current Item" =>
        {
            Some(link(7, value))
        }
        "Group" => {
            let source = match string_or_default(style, "linkedTextGroupSource", "Current Slide")? {
                "Current Slide" => 0,
                "Next Slide" => 1,
                "Next in Arrangement" => 2,
                _ => return None,
            };
            put_varint(1, source, &mut value);
            Some(link(15, value))
        }
        "Slide Count" => {
            let source = match string_or_default(style, "linkedTextSlideCountSource", "Current")? {
                "Current" => 0,
                "Remaining" => 1,
                "Total" => 2,
                _ => return None,
            };
            put_varint(1, source, &mut value);
            Some(link(22, value))
        }
        "Presentation Name" => {
            put_varint(1, 0, &mut value);
            Some(link(24, value))
        }
        "Operator Notes" => Some(link(17, value)),
        "Playlist Item" => {
            let source = match string_or_default(style, "linkedTextPlaylistItemSource", "Current")?
            {
                "Current" => 0,
                "Next" => 1,
                "Current Header" => 2,
                "Next Header" => 3,
                "Parent Playlist" => 4,
                _ => return None,
            };
            put_varint(1, source, &mut value);
            if let Some(show) = style.get("linkedTextPlaylistItemShowArrangement") {
                put_varint(2, u64::from(show.as_bool()?), &mut value);
            }
            Some(link(18, value))
        }
        "CCLI" => Some(link(14, value)),
        "Stage Message" => {
            if let Some(enabled) = style.get("linkedTextStageMessageFlash") {
                put_varint(1, u64::from(enabled.as_bool()?), &mut value);
            }
            if let Some(color) = style.get("linkedTextStageMessageFlashColor") {
                let color = color.as_str()?;
                crate::native_compositor::parse_color(color)?;
                let trigger = timer_color_trigger(
                    &json!({"time": 0, "color": color, "sourceColorComponents": style.get("linkedTextStageMessageFlashColorComponents").cloned().unwrap_or(Value::Null)}),
                );
                if style
                    .get("linkedTextStageMessageFlashColorComponents")
                    .is_some()
                {
                    let fields = decode_message(&trigger?).ok()?;
                    let bytes = bytes_field(&fields, 2)?;
                    put_message(2, bytes.to_vec(), &mut value);
                } else {
                    put_color(2, color, &mut value);
                }
            }
            Some(link(11, value))
        }
        "Slide Label" | "Next Slide Label" => {
            put_varint(1, u64::from(source == "Next Slide Label"), &mut value);
            Some(link(25, value))
        }
        "File" => {
            put_message(
                1,
                encode_url(nonempty_string(&style["linkedTextFilePath"])?),
                &mut value,
            );
            Some(link(28, value))
        }
        "RSS" => {
            let url = nonempty_string(&style["linkedTextRssUrl"])?;
            let format = match string_or_default(style, "linkedTextRssFormat", "title-description")?
            {
                "title" => 0,
                "title-description" => 1,
                _ => return None,
            };
            put_message(1, encode_url(url), &mut value);
            put_varint(2, format, &mut value);
            let delimiter = match style.get("linkedTextRssDelimiter") {
                Some(Value::String(value)) => value.as_str(),
                Some(Value::Bool(false)) => "",
                None => "•",
                _ => return None,
            };
            put_string(3, delimiter, &mut value);
            Some(link(27, value))
        }
        "Ticker" => {
            let mut text = Vec::new();
            if let Some(value) = style["linkedTextTickerContent"].as_str() {
                put_string(1, value, &mut text);
            }
            put_message(1, text, &mut value);
            Some(link(1, value))
        }
        _ => None,
    }
}

const TIMER_VISIBILITY: [&str; 4] = [
    "has-time-remaining",
    "has-expired",
    "is-running",
    "not-running",
];
const ELEMENT_VISIBILITY: [&str; 2] = ["has-text", "has-no-text"];
const COUNTDOWN_VISIBILITY: [&str; 6] = [
    "has-time-remaining",
    "has-expired",
    "is-running",
    "not-running",
    "looping",
    "not-looping",
];
const ACTIVE_VISIBILITY: [&str; 2] = ["active", "inactive"];

fn decode_visibility_condition(item: &[Field]) -> Result<Value, String> {
    let [field] = item else {
        return Err("visibility condition has more than one type".into());
    };
    let body =
        nested_field(item, field.number).ok_or("visibility condition has an invalid wire type")?;
    let named = |names: &[&'static str], number: u32| -> Result<&'static str, String> {
        names
            .get(varint_field(&body, number).unwrap_or(0) as usize)
            .copied()
            .ok_or_else(|| "unknown visibility criterion".to_string())
    };
    match field.number {
        1 if only_fields(&body, &[1, 2, 3]) => {
            let id = uuid(&body, 1).ok_or("element visibility has no element identifier")?;
            Ok(
                json!({"kind":"element","objectId":id,"objectName":string_field(&body, 2).unwrap_or_default(),"criterion":named(&ELEMENT_VISIBILITY, 3)?}),
            )
        }
        2 if only_fields(&body, &[1, 2, 3]) => {
            let id = uuid(&body, 1).ok_or("timer visibility has no timer identifier")?;
            Ok(
                json!({"timerId":id,"timerName":string_field(&body, 2).unwrap_or_default(),"criterion":named(&TIMER_VISIBILITY, 3)?}),
            )
        }
        3 | 6 if only_fields(&body, &[1]) => {
            let kind = if field.number == 3 { "video" } else { "audio" };
            Ok(json!({"kind":kind,"criterion":named(&COUNTDOWN_VISIBILITY, 1)?}))
        }
        4 if only_fields(&body, &[1]) => {
            Ok(json!({"kind":"capture","criterion":named(&ACTIVE_VISIBILITY, 1)?}))
        }
        5 if only_fields(&body, &[1, 2]) => {
            let index = varint_field(&body, 1).unwrap_or(0);
            if index > i32::MAX as u64 {
                return Err("video input visibility has an invalid index".into());
            }
            Ok(
                json!({"kind":"video-input","inputIndex":index,"criterion":named(&ACTIVE_VISIBILITY, 2)?}),
            )
        }
        _ => Err("unsupported visibility condition".into()),
    }
}

fn visibility_link(layer: &Value) -> Option<Vec<u8>> {
    let visibility = layer.get("visibilityLink")?;
    let criterion = match visibility["criterion"].as_str()? {
        "all" => 0,
        "any" => 1,
        "none" => 2,
        _ => return None,
    };
    let mut value = Vec::new();
    put_varint(1, criterion, &mut value);
    for condition in visibility["conditions"].as_array()? {
        let criterion = condition["criterion"].as_str()?;
        let position = |names: &[&str]| {
            names
                .iter()
                .position(|name| *name == criterion)
                .map(|index| index as u64)
        };
        let mut body = Vec::new();
        let field = match condition["kind"].as_str().unwrap_or("timer") {
            "timer" => {
                let criterion = position(&TIMER_VISIBILITY)?;
                let id = condition["timerId"]
                    .as_str()
                    .filter(|id| !id.trim().is_empty())?;
                put_uuid(1, id, &mut body);
                put_string(2, condition["timerName"].as_str()?, &mut body);
                put_varint(3, criterion, &mut body);
                2
            }
            "element" => {
                let criterion = position(&ELEMENT_VISIBILITY)?;
                let id = condition["objectId"]
                    .as_str()
                    .filter(|id| !id.trim().is_empty())?;
                put_uuid(1, id, &mut body);
                put_string(2, condition["objectName"].as_str()?, &mut body);
                put_varint(3, criterion, &mut body);
                1
            }
            kind @ ("video" | "audio") => {
                put_varint(1, position(&COUNTDOWN_VISIBILITY)?, &mut body);
                if kind == "video" {
                    3
                } else {
                    6
                }
            }
            "capture" => {
                put_varint(1, position(&ACTIVE_VISIBILITY)?, &mut body);
                4
            }
            "video-input" => {
                let index = condition["inputIndex"]
                    .as_u64()
                    .filter(|index| *index <= i32::MAX as u64)?;
                put_varint(1, index, &mut body);
                put_varint(2, position(&ACTIVE_VISIBILITY)?, &mut body);
                5
            }
            _ => return None,
        };
        let mut item = Vec::new();
        put_message(field, body, &mut item);
        put_message(2, item, &mut value);
    }
    Some(link(9, value))
}

/// Encodes the fill and text links currently represented by Orbit's layered slide model.
pub(super) fn encode_data_links(layer: &Value) -> Vec<Vec<u8>> {
    let mut links = Vec::new();
    if let Some(value) = visibility_link(layer) {
        links.push(value);
    }
    if let Some(value) = fill_link(layer) {
        links.push(value);
    }
    if let Some(value) = text_link(&layer["textStyle"]) {
        links.push(value);
    }
    links
}

fn set_text_source<'a>(
    layer: &'a mut Map<String, Value>,
    source: &str,
) -> &'a mut Map<String, Value> {
    let style = layer
        .entry("textStyle")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("Decoded text style is an object");
    style.insert("linkedTextEnabled".into(), json!(true));
    style.insert("linkedTextSource".into(), json!(source));
    style
}

fn unsupported_link(number: u32, reason: &str) -> String {
    format!("data link {}: {reason}", data_link_name(number))
}

fn decode_transform(value: u64) -> Option<&'static str> {
    match value {
        0 => Some("None"),
        1 => Some("Remove line returns"),
        2 => Some("One word per line"),
        3 => Some("One character per line"),
        _ => None,
    }
}

fn set_data_fill(layer: &mut Map<String, Value>, fill_type: &str) {
    layer.insert("fillType".into(), json!(fill_type));
    layer.remove("fill");
    layer.remove("fillColors");
}

fn selector(value: u64, maximum: u64) -> Option<&'static str> {
    match value {
        0 => Some("Current Slide"),
        1 => Some("Next Slide"),
        2 if maximum == 2 => Some("Next in Arrangement"),
        _ => None,
    }
}

fn clock_name(value: u64) -> Option<&'static str> {
    match value {
        0 => Some("none"),
        1 => Some("numeric"),
        2 => Some("short-month"),
        3 => Some("long-month"),
        4 => Some("full"),
        _ => None,
    }
}

fn clock_time(value: u64) -> Option<&'static str> {
    match value {
        0 => Some("none"),
        1 => Some("hours-minutes"),
        2 => Some("hours-minutes-seconds"),
        3 => Some("seconds-zone-short"),
        4 => Some("seconds-zone-long"),
        _ => None,
    }
}

fn only_fields(fields: &[Field], allowed: &[u32]) -> bool {
    fields.iter().all(|field| allowed.contains(&field.number))
}

fn decode_link_payload(
    number: u32,
    fields: &[Field],
    layer: &mut Map<String, Value>,
) -> Result<(), String> {
    match number {
        9 => {
            if !only_fields(fields, &[1, 2]) {
                return Err("visibility link contains unsupported fields".into());
            }
            let criterion = match varint_field(fields, 1).unwrap_or(0) {
                0 => "all",
                1 => "any",
                2 => "none",
                _ => return Err("unknown visibility criterion".into()),
            };
            let mut conditions = Vec::new();
            for item in nested_fields(fields, 2) {
                // A condition row with no type selected is an unfinished editor row: it is
                // the link without that condition, the same state Orbit's editor starts from.
                if item.is_empty() {
                    continue;
                }
                conditions.push(decode_visibility_condition(&item)?);
            }
            layer.insert(
                "visibilityLink".into(),
                json!({"criterion":criterion,"conditions":conditions}),
            );
            Ok(())
        }
        1 => {
            let text = nested_field(fields, 1).ok_or("ticker has no plain-text source")?;
            if !only_fields(fields, &[1, 4, 5, 6, 7])
                || f64_field(fields, 4).is_some()
                || bool_field(fields, 5) == Some(true)
                || f64_field(fields, 6).is_some_and(|value| value != 0.0)
                || string_field(fields, 7).is_some_and(|value| !value.is_empty())
                || !only_fields(&text, &[1])
            {
                return Err("ticker configuration has fields Orbit does not model".into());
            }
            let style = set_text_source(layer, "Ticker");
            if let Some(value) = string_field(&text, 1) {
                style.insert("linkedTextTickerContent".into(), json!(value));
            }
            Ok(())
        }
        2 => {
            let transform = varint_field(fields, 4).unwrap_or(0);
            let style = set_text_source(layer, "Object");
            style.insert("linkedTextObjectId".into(), json!(uuid(fields, 1)));
            if let Some(name) = string_field(fields, 2) {
                style.insert("linkedTextElementName".into(), json!(name));
            }
            if let Some(value) = decode_transform(transform) {
                style.insert("linkedTextLineTransform".into(), json!(value));
            } else {
                style.insert(
                    "lineTransform".into(),
                    json!("Unsupported object text transform"),
                );
                return Err(format!(
                    "object text transform {transform} is not supported"
                ));
            }
            if varint_field(fields, 3).unwrap_or(0) != 0 || !only_fields(fields, &[1, 2, 3, 4]) {
                return Err("object text link has unsupported text transform options".into());
            }
            if uuid(fields, 1).is_none() {
                return Err("object text link has no target UUID".into());
            }
            Ok(())
        }
        3 | 12 => {
            let video = number == 12;
            let (format_field, template_field, triggers_field) =
                if video { (1, 2, 3) } else { (3, 4, 5) };
            let timer = if video {
                let source = match varint_field(fields, 5).unwrap_or(0) {
                    0 => "presentation",
                    1 => "announcements",
                    _ => return Err("video countdown source is invalid".into()),
                };
                let style = set_text_source(layer, "Video Countdown");
                style.insert("videoCountdownSource".into(), json!(source));
                style.insert(
                    "videoCountdownIgnoreLooping".into(),
                    json!(bool_field(fields, 4).unwrap_or(false)),
                );
                style
            } else {
                let timer_id = uuid(fields, 1)
                    .filter(|id| Uuid::parse_str(id).is_ok())
                    .ok_or("timer text link has no timer UUID")?;
                let style = set_text_source(layer, "Countdown Timer");
                style.insert("linkedTextTimerId".into(), json!(timer_id));
                if has_field(fields, 2) {
                    let name = string_field_including_empty(fields, 2)
                        .ok_or("timer name has an invalid wire type or encoding")?;
                    style.insert("linkedTextTimerName".into(), json!(name));
                }
                style
            };
            if has_field(fields, format_field) {
                let Some(format) = nested_field(fields, format_field) else {
                    return Err("timer format is malformed".into());
                };
                let (format, problems) = decode_timer_format(&format);
                timer.insert("linkedTextTimerFormat".into(), format);
                if let Some(problem) = problems.first() {
                    return Err(problem.clone());
                }
            }
            if has_field(fields, template_field) {
                let Some(format) = string_field_including_empty(fields, template_field) else {
                    return Err(
                        "custom timer format string has an invalid wire type or encoding".into(),
                    );
                };
                timer.insert("linkedTextTimerFormatString".into(), json!(format));
            }
            if has_field(fields, triggers_field) {
                let mut decoded = Vec::new();
                let mut problems = Vec::new();
                let trigger_bytes = bytes_fields(fields, triggers_field).count();
                let trigger_fields = nested_fields(fields, triggers_field).collect::<Vec<_>>();
                if trigger_fields.len() != trigger_bytes {
                    problems.push("timer color trigger payload is malformed".into());
                }
                for trigger in trigger_fields {
                    let time = if has_field(&trigger, 1) {
                        f64_field(&trigger, 1)
                    } else {
                        Some(0.0)
                    }
                    .filter(|value| value.is_finite());
                    let color_fields = nested_field(&trigger, 2);
                    let color = color_fields.as_deref().map(decode_timer_trigger_color);
                    if trigger.iter().any(|field| !matches!(field.number, 1 | 2)) {
                        problems.push("timer color trigger has unmodeled fields".to_string());
                    }
                    if trigger.iter().filter(|field| field.number == 1).count() > 1 {
                        problems.push("timer color trigger time occurs more than once".into());
                    }
                    if time.is_none() {
                        problems.push("timer color trigger has an invalid time".into());
                    }
                    if color.is_none() {
                        problems.push("timer color trigger has an invalid color".into());
                    }
                    if let Some(Err(reason)) = color.as_ref() {
                        problems.push(reason.clone());
                    }
                    if let (Some(time), Some(Ok((color, components)))) = (time, color) {
                        let mut trigger = json!({"time":time,"color":color});
                        if let Some([blue, green, red, alpha]) =
                            crate::native_compositor::parse_color(&color)
                        {
                            if components
                                != [red, green, blue, alpha]
                                    .map(|channel| f32::from(channel) / 255.0)
                            {
                                trigger["sourceColorComponents"] = json!(components);
                            }
                        }
                        decoded.push(trigger);
                    }
                }
                timer.insert("linkedTextTimerColorTriggers".into(), json!(decoded));
                if let Some(problem) = problems.first() {
                    return Err(problem.clone());
                }
            }
            if !only_fields(fields, &[1, 2, 3, 4, 5]) {
                return Err("timer text link contains unsupported settings".into());
            }
            Ok(())
        }
        4 => {
            let clock = set_text_source(layer, "System Clock");
            let Some(format) = nested_field(fields, 5) else {
                return Err("clock text link has no modeled Clock.Format".into());
            };
            if let Some(template) = string_field_including_empty(fields, 1) {
                clock.insert("systemClockFormatString".into(), json!(template));
            }
            if !only_fields(fields, &[1, 5]) || !only_fields(&format, &[2, 3, 4]) {
                return Err("clock format contains fields Orbit does not model".into());
            }
            let date = varint_field(&format, 2).unwrap_or(0);
            let time = varint_field(&format, 3).unwrap_or(0);
            let (Some(date), Some(time)) = (clock_name(date), clock_time(time)) else {
                return Err("clock format has an unsupported date or time style".into());
            };
            clock.insert("systemClockDateFormat".into(), json!(date));
            clock.insert("systemClockTimeFormat".into(), json!(time));
            clock.insert(
                "systemClockUse24Hour".into(),
                json!(bool_field(&format, 4).unwrap_or(false)),
            );
            Ok(())
        }
        6 => {
            let screen = fields;
            set_data_fill(layer, "screen-preview");
            layer.insert("previewScreenId".into(), json!(uuid(screen, 1)));
            layer.insert("role".into(), json!("custom"));
            layer.insert("mediaScale".into(), json!("Scale to Fit"));
            if layer
                .get("text")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                layer.insert("type".into(), json!("Shape"));
            }
            if uuid(screen, 1).is_none() {
                return Err("output screen link has no screen UUID".into());
            }
            if let Some(name) = string_field(screen, 2) {
                layer.insert("fillTitle".into(), json!(name));
            }
            if !only_fields(screen, &[1, 2]) {
                return Err("output screen link contains unsupported settings".into());
            }
            Ok(())
        }
        7 => Err("Planning Center Live is no longer available in Orbit".into()),
        8 => {
            let fill = fields;
            set_data_fill(layer, "slide-object");
            layer.insert("fillObjectId".into(), json!(uuid(fill, 1)));
            if let Some(name) = string_field(fill, 2) {
                layer.insert("fillTitle".into(), json!(name));
            }
            if uuid(fill, 1).is_none() {
                return Err("object fill link has no target UUID".into());
            }
            if !only_fields(fill, &[1, 2]) {
                return Err("object fill link contains unsupported settings".into());
            }
            Ok(())
        }
        10 => {
            let source_slide = varint_field(fields, 1).unwrap_or(0);
            let source_option = varint_field(fields, 2).unwrap_or(0);
            let source = match (source_slide, source_option) {
                (0, 0) => Some("Current Slide"),
                (1, 0) => Some("Next Slide"),
                (0, 1) => Some("Slide Notes"),
                (1, 1) => Some("Next Slide Notes"),
                (0, 2) => Some("Current Slide Element"),
                (1, 2) => Some("Next Slide Element"),
                _ => None,
            };
            let Some(source) = source else {
                return Err(format!(
                    "slide text selector {source_slide}/{source_option} is not modeled"
                ));
            };
            if (fields.iter().any(|field| field.number == 3) && bool_field(fields, 3).is_none())
                || (source_option != 2
                    && string_field(fields, 4).is_some_and(|value| !value.is_empty()))
                || !only_fields(fields, &[1, 2, 3, 4, 5])
            {
                set_text_source(layer, source);
                return Err("slide text link has unsupported formatting settings".into());
            }
            let style = set_text_source(layer, source);
            if source_option == 2 {
                let name = string_field(fields, 4)
                    .filter(|name| !name.is_empty())
                    .ok_or("matched slide element has no name")?;
                style.insert("linkedTextElementName".into(), json!(name));
            }
            if let Some(preserve) = bool_field(fields, 3) {
                style.insert("preserveNotesFormat".into(), json!(preserve));
            }
            let transform = varint_field(fields, 5).unwrap_or(0);
            let Some(transform) = decode_transform(transform) else {
                return Err(format!("slide text transform {transform} is not supported"));
            };
            style.insert("linkedTextLineTransform".into(), json!(transform));
            Ok(())
        }
        11 => {
            let style = set_text_source(layer, "Stage Message");
            if (fields.iter().any(|field| field.number == 1) && bool_field(fields, 1).is_none())
                || (fields.iter().any(|field| field.number == 2)
                    && nested_field(fields, 2).is_none())
                || !only_fields(fields, &[1, 2])
            {
                return Err("Stage Message flash settings have an unexpected wire type".into());
            }
            style.insert(
                "linkedTextStageMessageFlash".into(),
                json!(bool_field(fields, 1).unwrap_or(false)),
            );
            if let Some(color_fields) = nested_field(fields, 2) {
                let (color, components) = decode_timer_trigger_color(&color_fields)?;
                style.insert("linkedTextStageMessageFlashColor".into(), json!(color));
                style.insert(
                    "linkedTextStageMessageFlashColorComponents".into(),
                    json!(components),
                );
            }
            Ok(())
        }
        13 => {
            let image = fields;
            if image.iter().any(|field| field.number == 1) && varint_field(image, 1).is_none() {
                return Err("slide image source has an unexpected wire type".into());
            }
            let source = varint_field(image, 1).unwrap_or(0);
            let fill_type = match source {
                0 => "current-slide-image",
                1 => "next-slide-image",
                _ => "unsupported-slide-image",
            };
            set_data_fill(layer, fill_type);
            layer.insert("role".into(), json!("custom"));
            layer.insert("mediaScale".into(), json!("Scale to Fit"));
            if layer
                .get("text")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                layer.insert("type".into(), json!("Shape"));
            }
            if !only_fields(image, &[1]) {
                return Err("slide image link contains unsupported settings".into());
            }
            if source > 1 {
                return Err(format!("slide image source {source} is not supported"));
            }
            Ok(())
        }
        14 => {
            if !fields.is_empty() {
                return Err("CCLI text link contains fields Orbit does not model".into());
            }
            set_text_source(layer, "CCLI");
            Ok(())
        }
        15 => {
            let source = varint_field(fields, 1).unwrap_or(0);
            let group = set_text_source(layer, "Group");
            if let Some(source) = selector(source, 2) {
                group.insert("linkedTextGroupSource".into(), json!(source));
            } else {
                group.insert(
                    "linkedTextGroupSource".into(),
                    json!(format!("Unsupported group source {source}")),
                );
                return Err(format!("group name source {source} is not supported"));
            }
            if !only_fields(fields, &[1]) {
                return Err("group name link contains unsupported settings".into());
            }
            Ok(())
        }
        16 | 26 => {
            let group = number == 16;
            let fill_type = if group { "group-color" } else { "label-color" };
            let source = varint_field(fields, 1).unwrap_or(0);
            set_data_fill(layer, fill_type);
            if let Some(source) = selector(source, u64::from(group) * 2) {
                layer.insert("dataFillSource".into(), json!(source));
            } else {
                layer.insert(
                    "dataFillSource".into(),
                    json!(format!("Unsupported data color source: Some({source})")),
                );
                return Err(format!("data color source {source} is not supported"));
            }
            if !only_fields(fields, &[1]) {
                return Err("data color link contains unsupported settings".into());
            }
            Ok(())
        }
        21 => {
            // ProPresenter stores no colors for this link; Orbit's editor defaults supply them.
            set_data_fill(layer, "capture-status-color");
            layer.insert(
                "captureStatusColors".into(),
                crate::authoring::default_capture_status_colors(),
            );
            if !fields.is_empty() {
                return Err("capture status color link contains unmodeled settings".into());
            }
            Ok(())
        }
        17 => {
            if !fields.is_empty() {
                return Err("presentation notes link contains fields Orbit does not model".into());
            }
            set_text_source(layer, "Operator Notes");
            Ok(())
        }
        18 => {
            let source = varint_field(fields, 1).unwrap_or(0);
            let name = match source {
                0 => "Current",
                1 => "Next",
                2 => "Current Header",
                3 => "Next Header",
                4 => "Parent Playlist",
                _ => return Err(format!("playlist item source {source} is not supported")),
            };
            let style = set_text_source(layer, "Playlist Item");
            style.insert("linkedTextPlaylistItemSource".into(), json!(name));
            if has_field(fields, 2) && bool_field(fields, 2).is_none()
                || !only_fields(fields, &[1, 2])
            {
                return Err("playlist item arrangement display has an unexpected wire type".into());
            }
            style.insert(
                "linkedTextPlaylistItemShowArrangement".into(),
                json!(bool_field(fields, 2).unwrap_or(false)),
            );
            Ok(())
        }
        22 => {
            let source = varint_field(fields, 1).unwrap_or(0);
            let name = match source {
                0 => "Current",
                1 => "Remaining",
                2 => "Total",
                _ => return Err(format!("slide count source {source} is not supported")),
            };
            let style = set_text_source(layer, "Slide Count");
            style.insert("linkedTextSlideCountSource".into(), json!(name));
            if !only_fields(fields, &[1]) {
                return Err("slide count link contains unsupported settings".into());
            }
            Ok(())
        }
        24 => {
            let source = varint_field(fields, 1).unwrap_or(0);
            if source != 0 || !only_fields(fields, &[1]) {
                return Err(format!("presentation source {source} is not modeled"));
            }
            set_text_source(layer, "Presentation Name");
            Ok(())
        }
        25 => {
            let source = varint_field(fields, 1).unwrap_or(0);
            if source > 1 || !only_fields(fields, &[1]) {
                return Err(format!("slide label source {source} is not supported"));
            }
            set_text_source(
                layer,
                if source == 0 {
                    "Slide Label"
                } else {
                    "Next Slide Label"
                },
            );
            Ok(())
        }
        27 => {
            let feed = set_text_source(layer, "RSS");
            if let Some(url) = decode_url_value(fields, 1) {
                feed.insert("linkedTextRssUrl".into(), json!(url));
            }
            if let Some(delimiter) = string_field(fields, 3) {
                feed.insert("linkedTextRssDelimiter".into(), json!(delimiter));
            }
            let content = varint_field(fields, 2).unwrap_or(0);
            match content {
                0 => {
                    feed.insert("linkedTextRssFormat".into(), json!("title"));
                }
                1 => {
                    feed.insert("linkedTextRssFormat".into(), json!("title-description"));
                }
                _ => return Err(format!("RSS content selector {content} is not supported")),
            }
            if !only_fields(fields, &[1, 2, 3]) {
                return Err("RSS link contains unsupported settings".into());
            }
            if !feed.contains_key("linkedTextRssUrl") {
                return Err("RSS link has no URL".into());
            }
            Ok(())
        }
        28 => {
            let file = set_text_source(layer, "File");
            if let Some(path) = decode_url_value(fields, 1) {
                file.insert("linkedTextFilePath".into(), json!(path));
            }
            if !only_fields(fields, &[1]) {
                return Err("file feed link contains unsupported settings".into());
            }
            if !file.contains_key("linkedTextFilePath") {
                return Err("file feed link has no path".into());
            }
            Ok(())
        }
        19 => {
            if !only_fields(fields, &[2]) {
                return Err("slide timer link contains unsupported settings".into());
            }
            let style = set_text_source(layer, "Slide Timer");
            if has_field(fields, 2) {
                let Some(format) = nested_field(fields, 2) else {
                    return Err("timer format is malformed".into());
                };
                let (format, problems) = decode_timer_format(&format);
                style.insert("linkedTextTimerFormat".into(), format);
                if let Some(problem) = problems.first() {
                    return Err(problem.clone());
                }
            }
            Ok(())
        }
        33 => {
            if !fields.is_empty() {
                return Err("timecode text link contains fields Orbit does not model".into());
            }
            set_text_source(layer, "Timecode");
            Ok(())
        }
        _ => return Err("unknown data link property".into()),
    }
}

/// Decodes one `DataLink`; `false` means its property type is not modeled by Orbit.
pub(super) fn decode_data_link(link_fields: &[Field], layer: &mut Map<String, Value>) -> bool {
    let Some(first) = link_fields.first() else {
        return false;
    };
    let number = first.number;
    if !matches!(
        number,
        1 | 2
            | 3
            | 4
            | 6
            | 7
            | 8
            | 9
            | 10
            | 11
            | 12
            | 13
            | 14
            | 15
            | 16
            | 17
            | 18
            | 19
            | 21
            | 22
            | 24
            | 25
            | 26
            | 27
            | 28
            | 33
    ) {
        return false;
    }
    match number {
        6 | 8 | 13 => {
            let kind = match number {
                6 => "screen-preview",
                8 => "slide-object",
                _ => "unsupported-slide-image",
            };
            set_data_fill(layer, kind);
            if number == 6 {
                layer.insert("previewScreenId".into(), Value::Null);
            }
            if number == 8 {
                layer.insert("fillObjectId".into(), Value::Null);
            }
            if number != 8 {
                layer.insert("role".into(), json!("custom"));
                layer.insert("mediaScale".into(), json!("Scale to Fit"));
                if layer
                    .get("text")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    layer.insert("type".into(), json!("Shape"));
                }
            }
        }
        _ => {}
    }
    let mut unsupported = Vec::new();
    if link_fields.len() != 1 {
        unsupported.push(unsupported_link(
            number,
            "contains multiple property values",
        ));
    }
    let result = match &first.value {
        WireValue::Bytes(bytes) => decode_message(bytes)
            .map_err(|_| "property payload is malformed".to_string())
            .and_then(|fields| decode_link_payload(number, &fields, layer)),
        _ => Err("property payload is not a message".into()),
    };
    if let Err(reason) = result {
        unsupported.push(unsupported_link(number, &reason));
    }
    note_unsupported(layer, unsupported);
    true
}

fn active_source(style: &Value) -> Option<&str> {
    let source = style["linkedTextSource"].as_str()?;
    (source != "None" && style["linkedTextEnabled"] != false).then_some(source)
}

/// Reports authored link values that have no exact `Slide.Element.DataLink` representation.
pub(super) fn losses(layer: &Value, reference: &str, findings: &mut Vec<String>) {
    if layer.get("visibilityLink").is_some() && visibility_link(layer).is_none() {
        findings.push(format!("{reference}/visibilityLink: visibility requires valid timer conditions and a supported criterion"));
    }

    let mut lost = |key: &str, reason: &str| {
        findings.push(format!("{reference}/{key}: {reason}"));
    };
    if matches!(
        layer["fillType"].as_str(),
        Some("screen-preview" | "slide-object" | "group-color" | "label-color")
    ) && fill_link(layer).is_none()
    {
        lost(
            "fillType",
            "the data link requires a nonempty supported source identifier or selector",
        );
    }
    if layer["fillType"] == "capture-status-color"
        && layer["captureStatusColors"] != crate::authoring::default_capture_status_colors()
    {
        lost(
            "captureStatusColors",
            "ProPresenter stores no colors for the capture status fill",
        );
    }
    if layer["fillType"] == "unsupported-slide-image" {
        lost(
            "fillType",
            "the slide image source is not supported by ProPresenter",
        );
    }
    let style = &layer["textStyle"];
    if let Some(source) = active_source(style) {
        if source == "Countdown Timer" {
            if nonempty_string(&style["linkedTextTimerId"]).is_none() {
                lost(
                    "textStyle/linkedTextTimerId",
                    "a countdown timer link requires a nonempty timer identifier",
                );
            }
            if style
                .get("linkedTextTimerName")
                .is_some_and(|value| !value.is_null() && !value.is_string())
            {
                lost(
                    "textStyle/linkedTextTimerName",
                    "the timer name must be text",
                );
            }
            if style
                .get("linkedTextTimerFormat")
                .is_some_and(|value| !value.is_null() && timer_format_message(value).is_none())
            {
                lost(
                    "textStyle/linkedTextTimerFormat",
                    "the timer format must use supported unit styles and boolean options",
                );
            }
            if style
                .get("linkedTextTimerFormatString")
                .is_some_and(|value| !value.is_null() && !value.is_string())
            {
                lost(
                    "textStyle/linkedTextTimerFormatString",
                    "the custom timer format must be text",
                );
            }
            if style
                .get("linkedTextTimerColorTriggers")
                .is_some_and(|triggers| {
                    !triggers.is_null() && timer_color_triggers(style).is_none()
                })
            {
                lost(
                    "textStyle/linkedTextTimerColorTriggers",
                    "each timer color trigger requires a finite time and supported color",
                );
            }
        } else if text_link(style).is_none() {
            lost(
                "textStyle/linkedTextSource",
                match source {
                    "Stage Message" => "the Orbit layer stores no Stage Message payload represented by this data link",
                    "Planning Center Live" => "this Planning Center Live mode has no matching ProPresenter text link setting",
                    "File" => "a file data link requires a nonempty linkedTextFilePath",
                    "RSS" => "an RSS data link requires a nonempty URL and supported feed format",
                    "Object" => "an object text data link requires a nonempty target identifier",
                    "System Clock" => "the authored clock format is not represented by Clock.Format",
                    _ => "this linked text source is not represented by a supported ProPresenter data link",
                },
            );
        }
        if style["linkedTextLineTransform"]
            .as_str()
            .is_some_and(|value| value != "None")
            && (!matches!(
                source,
                "Object"
                    | "Current Slide"
                    | "Next Slide"
                    | "Slide Notes"
                    | "Next Slide Notes"
                    | "Current Slide Element"
                    | "Next Slide Element"
            ) || line_transform(style["linkedTextLineTransform"].as_str()).is_none())
        {
            lost(
                "textStyle/linkedTextLineTransform",
                "this linked text data link has no element text transform field",
            );
        }
        if source == "System Clock" {
            if style["systemClockDateFormat"] == "day-long-month" {
                lost(
                    "textStyle/systemClockDateFormat",
                    "Clock.Format cannot retain Orbit's specific day-first long date format",
                );
            }
            if style.get("systemClockDateFormat").is_some()
                && string_or_default(style, "systemClockDateFormat", "numeric").is_none_or(
                    |value| {
                        !matches!(
                            value,
                            "none"
                                | "numeric"
                                | "short-month"
                                | "long-month"
                                | "day-long-month"
                                | "full"
                        )
                    },
                )
            {
                lost(
                    "textStyle/systemClockDateFormat",
                    "the date style has no Clock.Format equivalent",
                );
            }
            if style.get("systemClockTimeFormat").is_some()
                && string_or_default(style, "systemClockTimeFormat", "hours-minutes").is_none_or(
                    |value| {
                        !matches!(
                            value,
                            "none"
                                | "hours-minutes"
                                | "hours-minutes-seconds"
                                | "seconds-zone-short"
                                | "seconds-zone-long"
                        )
                    },
                )
            {
                lost(
                    "textStyle/systemClockTimeFormat",
                    "the time style has no Clock.Format equivalent",
                );
            }
            if style.get("systemClockUse24Hour").is_some()
                && !style["systemClockUse24Hour"].is_boolean()
            {
                lost(
                    "textStyle/systemClockUse24Hour",
                    "Clock.Format requires a boolean military-time setting",
                );
            }
        }
        if source == "RSS" {
            for key in [
                "linkedTextRssTitleSeparateLine",
                "linkedTextRssDescriptionSeparateLine",
            ] {
                if style[key] == false {
                    lost(
                        &format!("textStyle/{key}"),
                        "ProPresenter's RSS link does not store this line-break option",
                    );
                }
            }
        }
        if source == "Stage Message"
            && super::super::import_export_report::present(&style["linkedTextTickerContent"])
        {
            lost(
                "textStyle/linkedTextTickerContent",
                "the Stage Message data link does not store Orbit's cached message text",
            );
        }
    } else if style["linkedTextSource"]
        .as_str()
        .is_some_and(|source| source != "None" && style["linkedTextEnabled"] == false)
    {
        lost(
            "textStyle/linkedTextSource",
            "ProPresenter data links cannot retain an authored link while it is disabled",
        );
    }
}
