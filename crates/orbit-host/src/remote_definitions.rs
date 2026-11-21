//! Remote editing of persisted timer and message definitions.
//!
//! The remote only accepts an edit after the exact persisted target has been
//! validated here while the workspace operation lock is held. The renderer then
//! persists the edit through the same controllers as local editing and reports
//! the result for the request id.

use serde_json::{json, Map, Value};

const MAX_NAME_CHARS: usize = 200;
const MAX_TEXT_CHARS: usize = 2000;
const MAX_TOKEN_VALUES: usize = 64;
const MAX_DURATION_SECONDS: f64 = 359_999.0;

pub(crate) fn is_definition_command(command: &str) -> bool {
    matches!(command, "timerDefinition" | "messageDefinition")
}

fn only_keys(payload: &Value, allowed: &[&str]) -> Result<(), &'static str> {
    let object = payload
        .as_object()
        .ok_or("A definition edit must be an object.")?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("The definition edit contains unsupported fields.");
    }
    Ok(())
}

fn bounded_text(value: &Value, limit: usize) -> bool {
    value
        .as_str()
        .is_some_and(|text| !text.trim().is_empty() && text.chars().count() <= limit)
}

/// Validates the request shape before the persisted target is consulted.
pub(crate) fn validate_payload(payload: &Value) -> Result<(), &'static str> {
    for field in ["workspacePath", "id"] {
        if !payload
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err("A definition id and workspacePath are required.");
        }
    }
    if payload["command"] == "timerDefinition" {
        only_keys(payload, &["command", "workspacePath", "id", "patch"])?;
        let patch = payload
            .get("patch")
            .and_then(Value::as_object)
            .filter(|patch| !patch.is_empty())
            .ok_or("A timer definition edit requires a nonempty patch.")?;
        for (key, value) in patch {
            let valid = match key.as_str() {
                "name" => bounded_text(value, MAX_NAME_CHARS),
                "durationSeconds" => value.as_f64().is_some_and(|seconds| {
                    seconds.is_finite() && (0.0..=MAX_DURATION_SECONDS).contains(&seconds)
                }),
                "targetTimeSeconds" | "elapsedStartSeconds" | "elapsedEndSeconds" => {
                    value.is_null()
                        || value.as_f64().is_some_and(|seconds| {
                            seconds.is_finite() && (0.0..=MAX_DURATION_SECONDS).contains(&seconds)
                        })
                }
                "allowsOverrun" => value.is_boolean(),
                "type" => value.as_str().is_some_and(|value| {
                    matches!(value, "countdown" | "countdown-to-time" | "elapsed")
                }),
                "format" => validate_timer_format(value),
                _ => return Err("The timer definition edit contains unsupported fields."),
            };
            if !valid {
                return Err("A timer definition field is invalid.");
            }
        }
        return Ok(());
    }
    only_keys(
        payload,
        &["command", "workspacePath", "id", "text", "tokenValues"],
    )?;
    if payload.get("text").is_none() && payload.get("tokenValues").is_none() {
        return Err("A message definition edit requires text or tokenValues.");
    }
    if payload
        .get("text")
        .is_some_and(|text| !bounded_text(text, MAX_TEXT_CHARS))
    {
        return Err("Message text must be a nonempty string within the supported length.");
    }
    if let Some(values) = payload.get("tokenValues") {
        let values = values
            .as_object()
            .filter(|values| !values.is_empty() && values.len() <= MAX_TOKEN_VALUES)
            .ok_or("Message tokenValues must be a nonempty object within the supported size.")?;
        if values.iter().any(|(name, value)| {
            name.is_empty()
                || !value
                    .as_str()
                    .is_some_and(|text| text.chars().count() <= MAX_TEXT_CHARS)
        }) {
            return Err("Message token values must be strings within the supported length.");
        }
    }
    Ok(())
}

fn find<'a>(workspace: &'a Value, field: &str, id: &str) -> Result<&'a Value, String> {
    workspace
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Workspace {field} must be an array."))?
        .iter()
        .find(|entry| entry["id"].as_str() == Some(id))
        .ok_or_else(|| "Target is unavailable in the active workspace.".to_string())
}

fn timer_is_countdown(timer: &Value) -> bool {
    timer
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("countdown")
        == "countdown"
}

fn validate_timer_format(value: &Value) -> bool {
    let Some(format) = value.as_object() else {
        return value.is_null();
    };
    let allowed = [
        "hour",
        "minute",
        "second",
        "millisecond",
        "wallClock",
        "use24Hour",
        "millisecondsUnderMinuteOnly",
    ];
    !format.keys().any(|key| !allowed.contains(&key.as_str()))
        && format.iter().all(|(key, value)| {
            if matches!(
                key.as_str(),
                "wallClock" | "use24Hour" | "millisecondsUnderMinuteOnly"
            ) {
                value.is_boolean()
            } else {
                value
                    .as_str()
                    .is_some_and(|text| text.chars().count() <= MAX_TEXT_CHARS)
            }
        })
}

fn text_token_names(message: &Value) -> Vec<&str> {
    message
        .get("tokens")
        .and_then(Value::as_array)
        .map(|tokens| {
            tokens
                .iter()
                .filter(|token| token["type"] == "text")
                .filter_map(|token| token["name"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

fn placeholders(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find(['{', '}']) else {
            break;
        };
        if after.as_bytes()[end] == b'}' && end > 0 {
            names.push(&after[..end]);
        }
        rest = &after[end..];
    }
    names
}

/// Validates the edit against the exact persisted target.
pub(crate) fn validate_persisted(workspace: &Value, payload: &Value) -> Result<(), String> {
    let id = payload["id"].as_str().ok_or("A target id is required.")?;
    if payload["command"] == "timerDefinition" {
        let _timer = find(workspace, "countdownTimers", id)?;
        return Ok(());
    }
    let message = find(workspace, "messages", id)?;
    let editable = text_token_names(message);
    let mut values: Map<String, Value> = message
        .get("tokenValues")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(edits) = payload.get("tokenValues").and_then(Value::as_object) {
        for (name, value) in edits {
            if !editable.contains(&name.as_str()) {
                return Err(format!(
                    "Message token {name} is not an editable text token."
                ));
            }
            values.insert(name.clone(), value.clone());
        }
    }
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| message["text"].as_str())
        .ok_or("The persisted message text is invalid.")?;
    let known: Vec<&str> = message
        .get("tokens")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|token| token["name"].as_str())
        .chain(values.keys().map(String::as_str))
        .collect();
    if let Some(name) = placeholders(text)
        .into_iter()
        .find(|name| !known.contains(name))
    {
        return Err(format!(
            "Message text references the undefined token {name}."
        ));
    }
    Ok(())
}

/// Editable definition fields exposed through the remote status.
pub(crate) fn catalog(workspace: &Value) -> Result<(Vec<Value>, Vec<Value>), String> {
    let timers = workspace
        .get("countdownTimers")
        .and_then(Value::as_array)
        .ok_or("Workspace countdownTimers must be an array.")?
        .iter()
        .map(|timer| {
            json!({
                "id": timer["id"],
                "name": timer["name"],
                "type": timer.get("type").and_then(Value::as_str).unwrap_or("countdown"),
                "durationSeconds": timer["durationSeconds"],
                "allowsOverrun": timer["allowsOverrun"],
                "durationEditable": timer_is_countdown(timer),
            })
        })
        .collect();
    let messages = workspace
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("Workspace messages must be an array.")?
        .iter()
        .map(|message| {
            json!({
                "id": message["id"],
                "title": message["title"],
                "text": message["text"],
                "tokenValues": message.get("tokenValues").cloned().unwrap_or_else(|| json!({})),
                "textTokens": text_token_names(message),
            })
        })
        .collect();
    Ok((timers, messages))
}
