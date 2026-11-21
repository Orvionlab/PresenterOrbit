use serde_json::{Map, Value};

use super::{
    encode_countdown_clock, put_color, put_f64, put_message, put_string, put_uuid, put_varint,
};

#[derive(Debug)]
pub(super) enum MarkerActionExportError {
    UnsupportedVariant(String),
    Invalid(String),
}

impl From<String> for MarkerActionExportError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}

impl From<&str> for MarkerActionExportError {
    fn from(message: &str) -> Self {
        Self::Invalid(message.to_string())
    }
}

/// Encodes the action variants whose payload fields are retained by `decode_action`.
pub(super) fn encode_marker_action(action: &Value) -> Result<Vec<u8>, MarkerActionExportError> {
    let kind = required_string(action, "type", "Playback marker action type is required.")?;
    let action_fields = action
        .as_object()
        .ok_or("Playback marker action must be an object.")?;
    let empty_details = Map::new();
    let details = match action.get("details") {
        None => &empty_details,
        Some(Value::Object(details)) => details,
        Some(_) => return Err("Playback marker action details must be an object.".into()),
    };
    let id = required_string(action, "id", "Playback marker action id is required.")?;
    if id.is_empty() {
        return Err("Playback marker action id must not be empty.".into());
    }

    let (action_type, payload_field, payload) = match kind {
        "clear" => (5, 26, encode_clear(action, details)?),
        "prop" => (6, 28, encode_prop(action, details)?),
        "timer" => (3, 25, encode_timer(action_fields, details)?),
        "mask" => (7, 29, encode_collection_target(action_fields, 3)?),
        "audience-look" => (18, 37, encode_collection_target(action_fields, 1)?),
        "stage" => (1, 27, encode_stage(details)?),
        "macro" => (23, 40, encode_collection_target(action_fields, 1)?),
        "clear-group" => (24, 41, encode_collection_target(action_fields, 1)?),
        _ => {
            return Err(MarkerActionExportError::UnsupportedVariant(format!(
                "Playback marker action type {kind:?} is not exported."
            )))
        }
    };

    let mut encoded = Vec::new();
    put_uuid(1, id, &mut encoded);
    if let Some(title) = optional_string(action_fields, "title")? {
        put_string(2, title, &mut encoded);
    }
    encode_label(details, &mut encoded)?;
    let enabled = optional_bool(action_fields, "enabled")?.unwrap_or(true);
    put_varint(6, u64::from(enabled), &mut encoded);
    put_optional_nonnegative_f64(action_fields, "delaySeconds", 4, &mut encoded)?;
    put_optional_nonnegative_f64(action_fields, "durationSeconds", 8, &mut encoded)?;
    put_message(payload_field, payload, &mut encoded);
    put_varint(9, action_type, &mut encoded);

    if matches!(kind, "mask" | "audience-look" | "macro" | "clear-group") {
        validate_details(details, &[])?;
    }
    validate_action_fields(action_fields, kind)?;
    Ok(encoded)
}

fn encode_clear(
    action: &Value,
    details: &Map<String, Value>,
) -> Result<Vec<u8>, MarkerActionExportError> {
    let layer = required_string(
        action,
        "targetLayer",
        "Clear action targetLayer is required.",
    )?;
    let (layer, announcement_layer) = match layer {
        "announcements" => (0, true),
        "all" => (0, false),
        "audio" => (1, false),
        "background" => (2, false),
        "video-input" => (3, false),
        "prop" => (4, false),
        "slide" => (5, false),
        "logo" => (6, false),
        "messages" => (7, false),
        "audio-effects" => (8, false),
        _ => {
            return Err(MarkerActionExportError::UnsupportedVariant(format!(
                "Clear action targetLayer {layer:?} is not exported."
            )))
        }
    };
    let destination = match optional_string(details, "contentDestination")? {
        Some("CONTENT_DESTINATION_GLOBAL") => 0,
        Some("CONTENT_DESTINATION_ANNOUNCEMENTS") => 1,
        Some(value) => {
            return Err(MarkerActionExportError::UnsupportedVariant(format!(
                "Clear action contentDestination {value:?} is not exported."
            )));
        }
        None => u64::from(announcement_layer),
    };
    if (announcement_layer && destination != 1)
        || (!announcement_layer && layer == 0 && destination == 1)
    {
        return Err(
            "Clear action targetLayer and contentDestination do not round-trip together.".into(),
        );
    }

    let mut payload = Vec::new();
    put_varint(2, layer, &mut payload);
    put_varint(4, destination, &mut payload);
    validate_details(details, &["contentDestination"])?;
    Ok(payload)
}

fn encode_collection_target(
    action: &Map<String, Value>,
    identification_field: u32,
) -> Result<Vec<u8>, String> {
    let target = required_string_map(
        action,
        "targetId",
        "Playback marker action targetId is required.",
    )?;
    if target.is_empty() {
        return Err("Playback marker action targetId must not be empty.".into());
    }
    let mut identification = Vec::new();
    put_uuid(1, target, &mut identification);
    let mut payload = Vec::new();
    put_message(identification_field, identification, &mut payload);
    Ok(payload)
}

fn encode_prop(
    action: &Value,
    details: &Map<String, Value>,
) -> Result<Vec<u8>, MarkerActionExportError> {
    let target = required_string(action, "targetId", "Prop action targetId is required.")?;
    if target.is_empty() {
        return Err("Prop action targetId must not be empty.".into());
    }
    let value = required_string(action, "value", "Prop action value is required.")?;
    let mut payload = Vec::new();
    let mut identification = Vec::new();
    put_uuid(1, target, &mut identification);
    put_message(3, identification, &mut payload);
    match value {
        "show" => {
            let keys = [
                "autoClearEnabled",
                "autoClearFollowsPropDuration",
                "autoClearSeconds",
            ];
            if keys.iter().any(|key| details.contains_key(*key)) {
                let mut trigger = Vec::new();
                if let Some(enabled) = optional_bool(details, "autoClearEnabled")? {
                    put_varint(1, u64::from(enabled), &mut trigger);
                }
                if let Some(follows) = optional_bool(details, "autoClearFollowsPropDuration")? {
                    put_varint(2, u64::from(follows), &mut trigger);
                }
                if let Some(seconds) = optional_nonnegative_f64(details, "autoClearSeconds")? {
                    put_f64(3, seconds, &mut trigger);
                }
                put_message(4, trigger, &mut payload);
            }
        }
        "clear" => {
            if [
                "autoClearEnabled",
                "autoClearFollowsPropDuration",
                "autoClearSeconds",
            ]
            .iter()
            .any(|key| details.contains_key(*key))
            {
                return Err(MarkerActionExportError::UnsupportedVariant(
                    "Prop clear action cannot carry show trigger settings.".into(),
                ));
            }
            put_message(5, Vec::new(), &mut payload);
        }
        _ => {
            return Err(MarkerActionExportError::UnsupportedVariant(format!(
                "Prop action value {value:?} is not exported."
            )))
        }
    }
    validate_details(
        details,
        &[
            "autoClearEnabled",
            "autoClearFollowsPropDuration",
            "autoClearSeconds",
        ],
    )?;
    Ok(payload)
}

fn encode_timer(
    action: &Map<String, Value>,
    details: &Map<String, Value>,
) -> Result<Vec<u8>, MarkerActionExportError> {
    let action_value = optional_string(action, "value")?.unwrap_or("start");
    let action_value = match action_value {
        "start" => 0,
        "stop" => 1,
        "reset" => 2,
        "reset-and-start" => 3,
        "stop-and-reset" => 4,
        "increment" => 5,
        value => {
            return Err(MarkerActionExportError::UnsupportedVariant(format!(
                "Timer action value {value:?} is not exported."
            )))
        }
    };
    let mut payload = Vec::new();
    put_varint(2, action_value, &mut payload);
    if let Some(target) = optional_string(action, "targetId")? {
        if target.is_empty() {
            return Err("Timer action targetId must not be empty.".into());
        }
        let mut identification = Vec::new();
        put_uuid(1, target, &mut identification);
        put_message(4, identification, &mut payload);
    }
    if let Some(increment) = optional_f64(details, "incrementSeconds")? {
        put_f64(11, increment, &mut payload);
    }
    if let Some(configuration) = details.get("configuration") {
        let configuration = configuration
            .as_object()
            .ok_or("Timer action configuration must be an object.")?;
        put_message(10, encode_timer_configuration(configuration)?, &mut payload);
    }
    validate_details(details, &["incrementSeconds", "configuration"])?;
    Ok(payload)
}

fn encode_timer_configuration(
    value: &Map<String, Value>,
) -> Result<Vec<u8>, MarkerActionExportError> {
    let kind = required_string_map(value, "type", "Timer configuration type is required.")?;
    let duration = required_nonnegative_f64(value, "durationSeconds")?;
    let mut configuration = Vec::new();
    if let Some(allows_overrun) = optional_bool(value, "allowsOverrun")? {
        put_varint(4, u64::from(allows_overrun), &mut configuration);
    }
    match kind {
        "countdown" => {
            require_null(value, "format")?;
            require_null(value, "targetTimeSeconds")?;
            require_null(value, "elapsedStartSeconds")?;
            require_null(value, "elapsedEndSeconds")?;
            let mut countdown = Vec::new();
            put_f64(1, duration, &mut countdown);
            put_message(1, countdown, &mut configuration);
        }
        "countdown-to-time" => {
            let target = required_nonnegative_f64(value, "targetTimeSeconds")?;
            if target >= 86_400.0 {
                return Err("Timer action targetTimeSeconds must be less than one day.".into());
            }
            if duration != target {
                return Err(
                    "Timer configuration durationSeconds must match targetTimeSeconds.".into(),
                );
            }
            require_null(value, "elapsedStartSeconds")?;
            require_null(value, "elapsedEndSeconds")?;
            let countdown =
                encode_countdown_clock(target, value.get("format").unwrap_or(&Value::Null));
            put_message(2, countdown, &mut configuration);
        }
        "elapsed" => {
            require_null(value, "format")?;
            let start = required_nonnegative_f64(value, "elapsedStartSeconds")?;
            let end = optional_nonnegative_f64(value, "elapsedEndSeconds")?;
            if duration != end.unwrap_or(start) {
                return Err(
                    "Timer configuration durationSeconds must match its elapsed range.".into(),
                );
            }
            require_null(value, "targetTimeSeconds")?;
            let mut elapsed = Vec::new();
            put_f64(1, start, &mut elapsed);
            if let Some(end) = end {
                put_f64(2, end, &mut elapsed);
                put_varint(3, 1, &mut elapsed);
            }
            put_message(3, elapsed, &mut configuration);
        }
        kind => {
            return Err(MarkerActionExportError::UnsupportedVariant(format!(
                "Timer configuration type {kind:?} is not exported."
            )))
        }
    }
    reject_unknown_keys(
        value,
        &[
            "type",
            "durationSeconds",
            "allowsOverrun",
            "targetTimeSeconds",
            "elapsedStartSeconds",
            "elapsedEndSeconds",
            "format",
        ],
        "Timer action configuration",
    )?;
    Ok(configuration)
}

fn encode_stage(details: &Map<String, Value>) -> Result<Vec<u8>, MarkerActionExportError> {
    let mut payload = Vec::new();
    if let Some(target) = optional_u64(details, "slideTarget")? {
        if target > 2 {
            return Err("Stage action slideTarget must be 0, 1, or 2.".into());
        }
        put_varint(5, target, &mut payload);
    }
    if let Some(assignments) = details.get("stageScreenAssignments") {
        let assignments = assignments
            .as_array()
            .ok_or("Stage action stageScreenAssignments must be an array.")?;
        for assignment in assignments {
            let assignment = assignment
                .as_object()
                .ok_or("Stage action assignment must be an object.")?;
            let mut encoded = Vec::new();
            for (key, field) in [("screenId", 1), ("layoutId", 2)] {
                let id = required_string_map(
                    assignment,
                    key,
                    &format!("Stage action assignment {key} is required."),
                )?;
                if !id.is_empty() {
                    let mut identification = Vec::new();
                    put_uuid(1, id, &mut identification);
                    put_message(field, identification, &mut encoded);
                }
            }
            reject_unknown_keys(
                assignment,
                &["screenId", "layoutId"],
                "Stage action assignment",
            )?;
            put_message(4, encoded, &mut payload);
        }
    }
    validate_details(details, &["slideTarget", "stageScreenAssignments"])?;
    Ok(payload)
}

pub(super) fn encode_label(
    details: &Map<String, Value>,
    action: &mut Vec<u8>,
) -> Result<(), String> {
    let label = optional_string(details, "label")?;
    let color = optional_string(details, "labelColor")?;
    if label.is_none() && color.is_none() {
        return Ok(());
    }
    let mut value = Vec::new();
    if let Some(label) = label {
        put_string(2, label, &mut value);
    }
    if let Some(color) = color {
        if crate::native_compositor::parse_color(color).is_none() {
            return Err("Action labelColor must be a supported color.".into());
        }
        put_color(3, color, &mut value);
    }
    put_message(3, value, action);
    Ok(())
}

fn validate_action_fields(
    action: &Map<String, Value>,
    kind: &str,
) -> Result<(), MarkerActionExportError> {
    let mut fields = vec![
        "id",
        "type",
        "title",
        "enabled",
        "delaySeconds",
        "durationSeconds",
        "details",
    ];
    match kind {
        "clear" => fields.push("targetLayer"),
        "prop" => fields.extend(["targetId", "value"]),
        "timer" => fields.extend(["targetId", "value"]),
        "mask" | "audience-look" | "macro" | "clear-group" => fields.push("targetId"),
        "stage" => (),
        _ => unreachable!("action type has already been matched"),
    }
    reject_unknown_keys(action, &fields, "Playback marker action")
}

fn validate_details(
    details: &Map<String, Value>,
    payload_fields: &[&str],
) -> Result<(), MarkerActionExportError> {
    let mut fields = payload_fields.to_vec();
    fields.extend(["label", "labelColor"]);
    reject_unknown_keys(details, &fields, "Playback marker action detail")
}

fn reject_unknown_keys(
    value: &Map<String, Value>,
    allowed: &[&str],
    label: &str,
) -> Result<(), MarkerActionExportError> {
    if let Some(key) = value.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(MarkerActionExportError::UnsupportedVariant(format!(
            "{label} field {key:?} is not exported."
        )));
    }
    Ok(())
}

fn required_string<'a>(value: &'a Value, key: &str, message: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| message.to_string())
}

fn required_string_map<'a>(
    value: &'a Map<String, Value>,
    key: &str,
    message: &str,
) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| message.to_string())
}

fn optional_string<'a>(
    value: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, String> {
    value
        .get(key)
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("Playback marker action field {key} must be a string."))
        })
        .transpose()
}

fn optional_bool(value: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    value
        .get(key)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| format!("Playback marker action field {key} must be a boolean."))
        })
        .transpose()
}

fn optional_f64(value: &Map<String, Value>, key: &str) -> Result<Option<f64>, String> {
    value
        .get(key)
        .map(|value| {
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| format!("Playback marker action detail {key} must be finite."))
        })
        .transpose()
}

fn required_nonnegative_f64(value: &Map<String, Value>, key: &str) -> Result<f64, String> {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| format!("Timer action field {key} must be finite and non-negative."))
}

fn optional_nonnegative_f64(value: &Map<String, Value>, key: &str) -> Result<Option<f64>, String> {
    value
        .get(key)
        .map(|_| required_nonnegative_f64(value, key))
        .transpose()
}

fn optional_u64(value: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    value
        .get(key)
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                format!("Playback marker action detail {key} must be a non-negative integer.")
            })
        })
        .transpose()
}

fn put_optional_nonnegative_f64(
    action: &Map<String, Value>,
    key: &str,
    field: u32,
    output: &mut Vec<u8>,
) -> Result<(), String> {
    let Some(value) = optional_nonnegative_f64(action, key)? else {
        return Ok(());
    };
    if value > 0.0 {
        put_f64(field, value, output);
    }
    Ok(())
}

fn require_null(value: &Map<String, Value>, key: &str) -> Result<(), String> {
    if value.get(key).is_some_and(|value| !value.is_null()) {
        return Err(format!(
            "Timer action configuration {key} is not represented."
        ));
    }
    Ok(())
}
