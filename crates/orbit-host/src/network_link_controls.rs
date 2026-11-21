use std::path::Path;

use serde_json::{json, Map, Value};

use crate::network_link::{NetworkLinkCue, NetworkLinkMediaAction, NetworkLinkShowControl};

const MAX_TOKEN_VALUES: usize = 64;
const MAX_TEXT_CHARS: usize = 2000;

pub(crate) fn map_cue(
    workspace: &Value,
    workspace_path: &Path,
    cue: &NetworkLinkCue,
    snapshot: Option<&Value>,
) -> Result<Value, String> {
    match cue {
        NetworkLinkCue::Media {
            bin_index,
            item_index,
            action,
        } => map_media(
            workspace,
            workspace_path,
            *bin_index,
            *item_index,
            *action,
            snapshot,
        ),
        NetworkLinkCue::ShowControl {
            control,
            index,
            action,
            values,
        } => map_show_control(
            workspace,
            workspace_path,
            *control,
            *index,
            action,
            values.as_ref(),
            snapshot,
        ),
        NetworkLinkCue::Playlist { .. } | NetworkLinkCue::Library { .. } => {
            Err("Network Link control mapper received a presentation cue.".into())
        }
    }
}

fn map_media(
    workspace: &Value,
    workspace_path: &Path,
    bin_index: u32,
    item_index: u32,
    action: NetworkLinkMediaAction,
    snapshot: Option<&Value>,
) -> Result<Value, String> {
    let bins = crate::remote_media::catalog(workspace)?;
    let item = bins
        .as_array()
        .and_then(|bins| bins.get(bin_index as usize))
        .and_then(|bin| bin.get("items"))
        .and_then(Value::as_array)
        .and_then(|items| items.get(item_index as usize))
        .ok_or("Network Link media bin or item index is unavailable.")?;
    let id = item["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Network Link media identity is invalid.")?;
    let action = match action {
        NetworkLinkMediaAction::Trigger => "trigger",
        NetworkLinkMediaAction::Play => "play",
        NetworkLinkMediaAction::Pause => "pause",
        NetworkLinkMediaAction::Stop => "stop",
    };
    let workspace_path = workspace_path
        .to_str()
        .ok_or("Workspace path must be valid Unicode for Network Link.")?;
    let mut payload =
        json!({"command":"media","workspacePath":workspace_path,"id":id,"action":action});
    if action != "trigger" {
        if let Some(snapshot) = snapshot {
            let (target, selection) = if item["type"] == "audio" {
                ("audioId", "audioSelectionId")
            } else if matches!(item["type"].as_str(), Some("video" | "image")) {
                ("backgroundId", "backgroundSelectionId")
            } else {
                return Err("Network Link media item has an unsupported type.".into());
            };
            if snapshot[target].as_str() != Some(id) {
                return Err(
                    "Network Link media transport target is not currently selected.".into(),
                );
            }
            let selection_id = snapshot
                .get(selection)
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0)
                .ok_or("Network Link media selection identity is invalid.")?;
            payload["selectionId"] = json!(selection_id);
        }
    }
    Ok(payload)
}

fn map_show_control(
    workspace: &Value,
    workspace_path: &Path,
    control: NetworkLinkShowControl,
    index: Option<u32>,
    action: &str,
    values: Option<&Value>,
    snapshot: Option<&Value>,
) -> Result<Value, String> {
    let workspace_path = workspace_path
        .to_str()
        .ok_or("Workspace path must be valid Unicode for Network Link.")?;
    match control {
        NetworkLinkShowControl::Timer => {
            let timer = indexed(workspace, "countdownTimers", index, "timer")?;
            let id = target_id(timer, "timer")?;
            if action == "configure" {
                let patch = timer_patch(values.ok_or("Timer configuration values are required.")?)?;
                return Ok(
                    json!({"command":"timerDefinition","workspacePath":workspace_path,"id":id,"patch":patch}),
                );
            }
            if !matches!(
                action,
                "start"
                    | "stop"
                    | "reset"
                    | "reset-and-start"
                    | "reset-to-start"
                    | "stop-and-reset"
                    | "toggle"
                    | "increment"
            ) {
                return Err("Unsupported Network Link timer action.".into());
            }
            let mut payload =
                json!({"command":"timer","workspacePath":workspace_path,"id":id,"action":action});
            if action == "increment" {
                let values = values_object(values, &["incrementSeconds"])?;
                let seconds = finite_number(values.get("incrementSeconds"), "Timer increment")?;
                if !(-MAX_TIMER_SECONDS..=MAX_TIMER_SECONDS).contains(&seconds) {
                    return Err("Timer increment is outside the supported range.".into());
                }
                payload["seconds"] = json!(seconds);
            } else if values.is_some() {
                return Err("Timer action does not accept values.".into());
            }
            Ok(payload)
        }
        NetworkLinkShowControl::Prop => {
            let prop = indexed(workspace, "props", index, "prop")?;
            let id = target_id(prop, "prop")?;
            if !matches!(action, "show" | "clear") {
                return Err("Unsupported Network Link prop action.".into());
            }
            let values = prop_values(values.ok_or("Prop preferences are required.")?)?;
            Ok(
                json!({"command":"prop","workspacePath":workspace_path,"id":id,"action":action,"values":values}),
            )
        }
        NetworkLinkShowControl::Macro => {
            let macro_entry = indexed(workspace, "macros", index, "macro")?;
            let id = target_id(macro_entry, "macro")?;
            if action != "trigger" || values.is_some() {
                return Err("Network Link macro cue must be trigger without values.".into());
            }
            Ok(json!({"command":"macro","workspacePath":workspace_path,"id":id}))
        }
        NetworkLinkShowControl::Message => {
            map_message(workspace, workspace_path, index, action, values)
        }
        NetworkLinkShowControl::StageLayout => {
            let layout = indexed(workspace, "stageLayouts", index, "stage layout")?;
            let id = target_id(layout, "stage layout")?;
            if action != "assign" {
                return Err("Unsupported Network Link stage layout action.".into());
            }
            let values = values_object(values, &["screenIndex"])?;
            let screen_index = values
                .get("screenIndex")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("Stage screen index must be a non-negative integer.")?;
            if let Some(snapshot) = snapshot {
                let screen_id = stage_screen_ids(snapshot)?
                    .get(screen_index)
                    .cloned()
                    .ok_or("Stage screen index is unavailable on this receiver.")?;
                Ok(
                    json!({"command":"stageLayout","workspacePath":workspace_path,"id":id,"screenId":screen_id}),
                )
            } else {
                Ok(
                    json!({"command":"stageLayout","workspacePath":workspace_path,"id":id,"screenIndex":screen_index}),
                )
            }
        }
    }
}

fn map_message(
    workspace: &Value,
    workspace_path: &str,
    index: Option<u32>,
    action: &str,
    values: Option<&Value>,
) -> Result<Value, String> {
    if action == "clear" {
        if index.is_some() || values.is_some() {
            return Err("Message clear does not accept an index or values.".into());
        }
        return Ok(json!({"command":"message","workspacePath":workspace_path,"action":"clear"}));
    }
    let message = indexed(workspace, "messages", index, "message")?;
    let id = target_id(message, "message")?;
    match action {
        "show" => {
            let values = match values {
                Some(values) => {
                    let object = values_object(Some(values), &["tokenValues"])?;
                    let mapped = token_values_to_local(message, object.get("tokenValues"), None)?;
                    if mapped.is_empty() {
                        None
                    } else {
                        Some(mapped)
                    }
                }
                None => None,
            };
            let mut payload =
                json!({"command":"message","workspacePath":workspace_path,"action":"show","id":id});
            if let Some(values) = values {
                payload["tokenValues"] = Value::Object(values);
            }
            Ok(payload)
        }
        "configure" => {
            let values =
                values_object(values, &["text", "tokenValues", "tokenNames", "tokenTypes"])?;
            let token_names = string_array(values.get("tokenNames"), "Message tokenNames")?;
            let token_types = string_array(values.get("tokenTypes"), "Message tokenTypes")?;
            let local_tokens = tokens(message)?;
            if token_names.len() != local_tokens.len() || token_types.len() != local_tokens.len() {
                return Err(
                    "Message token structures do not match between Network Link peers.".into(),
                );
            }
            let mut source_names = std::collections::HashMap::new();
            for (position, ((source_name, source_type), local)) in token_names
                .iter()
                .zip(&token_types)
                .zip(&local_tokens)
                .enumerate()
            {
                if source_name.trim().is_empty()
                    || local.name.trim().is_empty()
                    || source_type != &local.kind
                    || source_names.insert(source_name.clone(), position).is_some()
                {
                    return Err(
                        "Message token structures do not match between Network Link peers.".into(),
                    );
                }
            }
            let text = values
                .get("text")
                .map(|value| -> Result<Value, String> {
                    let text = bounded_string(value, MAX_TEXT_CHARS, "Message text")?;
                    Ok(Value::String(rewrite_placeholders(
                        text,
                        &source_names,
                        &local_tokens,
                    )?))
                })
                .transpose()?;
            let token_values =
                token_values_to_local(message, values.get("tokenValues"), Some(&local_tokens))?;
            if text.is_none() && token_values.is_empty() {
                return Err("Message configuration requires text or token values.".into());
            }
            let mut payload =
                json!({"command":"messageDefinition","workspacePath":workspace_path,"id":id});
            if let Some(text) = text {
                payload["text"] = text;
            }
            if !token_values.is_empty() {
                payload["tokenValues"] = Value::Object(token_values);
            }
            Ok(payload)
        }
        _ => Err("Unsupported Network Link message action.".into()),
    }
}

#[derive(Clone)]
struct MessageToken {
    name: String,
    kind: String,
}

fn tokens(message: &Value) -> Result<Vec<MessageToken>, String> {
    let Some(raw_tokens) = message.get("tokens") else {
        return Ok(Vec::new());
    };
    let mut names = std::collections::HashSet::new();
    raw_tokens
        .as_array()
        .ok_or("Message tokens must be an array.")?
        .iter()
        .map(|token| {
            let name = token
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or("Message token name is invalid.")?;
            let kind = token
                .get("type")
                .and_then(Value::as_str)
                .filter(|kind| matches!(*kind, "clock" | "text" | "timer"))
                .ok_or("Message token type is unsupported.")?;
            if !names.insert(name) {
                return Err("Message token names must be unique for Network Link mapping.".into());
            }
            Ok(MessageToken {
                name: name.to_string(),
                kind: kind.to_string(),
            })
        })
        .collect()
}

fn token_values_to_local(
    message: &Value,
    value: Option<&Value>,
    tokens_override: Option<&[MessageToken]>,
) -> Result<Map<String, Value>, String> {
    let Some(value) = value else {
        return Ok(Map::new());
    };
    let values = value
        .as_object()
        .filter(|values| values.len() <= MAX_TOKEN_VALUES)
        .ok_or("Message tokenValues must be a bounded object.")?;
    let local_tokens = match tokens_override {
        Some(tokens) => tokens.to_vec(),
        None => tokens(message)?,
    };
    let mut mapped = Map::new();
    for (position, text) in values {
        let index = position
            .parse::<usize>()
            .ok()
            .filter(|index| index.to_string() == *position)
            .ok_or("Message tokenValues keys must be decimal token positions.")?;
        let token = local_tokens
            .get(index)
            .filter(|token| token.kind == "text")
            .ok_or("Message token value targets a non-text or unavailable token.")?;
        let text = bounded_string(text, MAX_TEXT_CHARS, "Message token value")?;
        mapped.insert(token.name.clone(), json!(text));
    }
    Ok(mapped)
}

fn rewrite_placeholders(
    text: &str,
    source_names: &std::collections::HashMap<String, usize>,
    local_tokens: &[MessageToken],
) -> Result<String, String> {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find(['{', '}']) else {
            result.push_str(&rest[start..]);
            return Ok(result);
        };
        if after.as_bytes()[end] != b'}' || end == 0 {
            result.push('{');
            rest = after;
            continue;
        }
        let source = &after[..end];
        let position = source_names
            .get(source)
            .copied()
            .ok_or_else(|| format!("Message text references unknown token {source}."))?;
        let local = local_tokens
            .get(position)
            .ok_or("Message token structure changed during mapping.")?;
        result.push('{');
        result.push_str(&local.name);
        result.push('}');
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

fn timer_patch(value: &Value) -> Result<Value, String> {
    let values = values_object(
        Some(value),
        &[
            "type",
            "durationSeconds",
            "targetTimeSeconds",
            "elapsedStartSeconds",
            "elapsedEndSeconds",
            "allowsOverrun",
            "format",
        ],
    )?;
    let kind = values
        .get("type")
        .and_then(Value::as_str)
        .filter(|kind| matches!(*kind, "countdown" | "countdown-to-time" | "elapsed"))
        .ok_or("Timer configuration type is invalid.")?;
    let duration = finite_number(values.get("durationSeconds"), "Timer duration")?;
    if !(0.0..=MAX_TIMER_SECONDS).contains(&duration) {
        return Err("Timer duration is outside the supported range.".into());
    }
    let overrun = values
        .get("allowsOverrun")
        .and_then(Value::as_bool)
        .ok_or("Timer allowsOverrun must be boolean.")?;
    let format = match values.get("format") {
        Some(Value::Null) => Value::Null,
        Some(Value::Object(format)) => {
            let allowed = [
                "hour",
                "minute",
                "second",
                "millisecond",
                "wallClock",
                "use24Hour",
                "millisecondsUnderMinuteOnly",
            ];
            if format.keys().any(|key| !allowed.contains(&key.as_str()))
                || format.iter().any(|(key, value)| {
                    if matches!(
                        key.as_str(),
                        "wallClock" | "use24Hour" | "millisecondsUnderMinuteOnly"
                    ) {
                        !value.is_boolean()
                    } else {
                        !value
                            .as_str()
                            .is_some_and(|text| text.chars().count() <= MAX_TEXT_CHARS)
                    }
                })
            {
                return Err("Timer format is invalid.".into());
            }
            Value::Object(format.clone())
        }
        _ => return Err("Timer format must be an object or null.".into()),
    };
    let mut patch = json!({
        "type":kind,
        "durationSeconds":duration,
        "allowsOverrun":overrun,
        "format":format,
    });
    for field in [
        "targetTimeSeconds",
        "elapsedStartSeconds",
        "elapsedEndSeconds",
    ] {
        let number = match values.get(field) {
            None | Some(Value::Null) => Value::Null,
            Some(value) => {
                let seconds = finite_number(Some(value), "Timer time")?;
                if !(0.0..=MAX_TIMER_SECONDS).contains(&seconds) {
                    return Err("Timer time is outside the supported range.".into());
                }
                json!(seconds)
            }
        };
        patch[field] = number;
    }
    Ok(patch)
}

const MAX_TIMER_SECONDS: f64 = 359_999.0;

fn prop_values(value: &Value) -> Result<Value, String> {
    let values = values_object(
        Some(value),
        &["singleMode", "autoClearEnabled", "autoClearSeconds"],
    )?;
    let single_mode = values
        .get("singleMode")
        .and_then(Value::as_bool)
        .ok_or("Prop singleMode must be boolean.")?;
    let auto_clear = values
        .get("autoClearEnabled")
        .and_then(Value::as_bool)
        .ok_or("Prop autoClearEnabled must be boolean.")?;
    let seconds = match values.get("autoClearSeconds") {
        None | Some(Value::Null) => Value::Null,
        Some(value) => {
            let seconds = finite_number(Some(value), "Prop auto-clear duration")?;
            if !(0.0..=86_400.0).contains(&seconds) || (auto_clear && seconds <= 0.0) {
                return Err("Prop auto-clear duration is outside the supported range.".into());
            }
            json!(seconds)
        }
    };
    Ok(json!({"singleMode":single_mode,"autoClearEnabled":auto_clear,"autoClearSeconds":seconds}))
}

fn values_object<'a>(
    value: Option<&'a Value>,
    allowed: &[&str],
) -> Result<&'a Map<String, Value>, String> {
    let object = value
        .and_then(Value::as_object)
        .ok_or("Network Link control values must be an object.")?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Network Link control values contain unsupported fields.".into());
    }
    Ok(object)
}

fn string_array(value: Option<&Value>, field: &str) -> Result<Vec<String>, String> {
    value
        .and_then(Value::as_array)
        .filter(|items| items.len() <= MAX_TOKEN_VALUES)
        .ok_or_else(|| format!("{field} must be a bounded string array."))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{field} must be a bounded string array."))
        })
        .collect()
}

fn finite_number(value: Option<&Value>, field: &str) -> Result<f64, String> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("{field} must be a finite number."))
}

fn bounded_string<'a>(value: &'a Value, max: usize, field: &str) -> Result<&'a str, String> {
    value
        .as_str()
        .filter(|text| text.chars().count() <= max)
        .ok_or_else(|| format!("{field} must be a string within the supported length."))
}

fn indexed<'a>(
    workspace: &'a Value,
    field: &str,
    index: Option<u32>,
    label: &str,
) -> Result<&'a Value, String> {
    let index = index.ok_or_else(|| format!("Network Link {label} index is required."))?;
    workspace
        .get(field)
        .and_then(Value::as_array)
        .and_then(|entries| entries.get(index as usize))
        .ok_or_else(|| format!("Network Link {label} index is unavailable."))
}

fn target_id<'a>(entry: &'a Value, label: &str) -> Result<&'a str, String> {
    entry
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| format!("Network Link {label} identity is invalid."))
}

fn stage_screen_ids(snapshot: &Value) -> Result<Vec<String>, String> {
    let config = snapshot
        .get("screenConfig")
        .and_then(Value::as_object)
        .ok_or("Accepted output screen configuration is unavailable.")?;
    let mut ids = config
        .iter()
        .filter(|(_, screen)| screen.get("type").and_then(Value::as_str) == Some("stage"))
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}
