//! ProPresenter Remote message and prop endpoints backed by Orbit's accepted
//! output state and persisted catalogs.

use serde_json::{json, Value};

use super::super::{data, Backend};
use super::catalog::{identity, matches_id};

const MAX_TOKEN_COUNT: usize = 64;
const MAX_TOKEN_TEXT_CHARS: usize = 2000;

pub(super) fn handle(
    backend: &dyn Backend,
    method: &str,
    parts: &[&str],
    body: Option<&Value>,
) -> Option<Result<Value, String>> {
    Some(match (method, parts) {
        ("GET", ["v1", "props"]) => props(backend),
        ("GET", ["v1", "prop_collections"]) => prop_collections(backend),
        ("GET", ["v1", "prop_collection", selector]) => prop_collection(backend, selector),
        ("GET", ["v1", "prop", selector, "trigger"]) => prop_command(backend, selector, "show"),
        ("GET", ["v1", "prop", selector, "clear"]) => prop_command(backend, selector, "clear"),
        ("GET", ["v1", "prop", selector]) => prop(backend, selector),
        ("GET", ["v1", "messages"]) => messages(backend),
        ("GET", ["v1", "message", selector, "clear"]) => clear_message(backend, selector),
        ("POST", ["v1", "message", selector, "trigger"]) => {
            trigger_message(backend, selector, body)
        }
        ("GET", ["v1", "message", selector]) => message(backend, selector),
        _ if known_but_unsupported(method, parts) => Err(
            "This ProPresenter Remote endpoint is not available from Orbit's current data and command model.".into(),
        ),
        _ => return None,
    })
}

fn known_but_unsupported(method: &str, parts: &[&str]) -> bool {
    match parts {
        ["v1", "props"] | ["v1", "prop_collections"] | ["v1", "prop_collection", _] => {
            method != "GET"
        }
        ["v1", "prop", _, "trigger" | "clear"] => method != "GET",
        ["v1", "prop", _, "auto_clear", "pause" | "resume"] | ["v1", "prop", _, "thumbnail"] => {
            true
        }
        ["v1", "prop", _] => method != "GET",
        ["v1", "messages"] => method != "GET",
        ["v1", "message", _, "trigger"] => method != "POST",
        ["v1", "message", _, "clear"] => method != "GET",
        ["v1", "message", _] => method != "GET",
        _ => false,
    }
}

fn modern_entries<'a>(status: &'a Value, field: &str) -> Result<&'a [Value], String> {
    status["modernCatalog"][field]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("The {field} catalog is unavailable."))
}

fn item_identity<'a>(entry: &'a Value, name_field: &str) -> Result<(&'a str, &'a str), String> {
    let id = entry["id"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .ok_or("Catalog item identity is unavailable.")?;
    let name = entry[name_field]
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .ok_or("Catalog item name is unavailable.")?;
    Ok((id, name))
}

fn resolve<'a>(
    entries: &'a [Value],
    selector: &str,
    name_field: &str,
) -> Result<(usize, &'a Value), String> {
    let mut found = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let (id, name) = item_identity(entry, name_field)?;
        if matches_id(id, name, index, selector) {
            found.push((index, entry));
        }
    }
    match found.as_slice() {
        [(index, entry)] => Ok((*index, *entry)),
        [] => Err("The requested target is unavailable.".into()),
        _ => Err("The requested target is ambiguous.".into()),
    }
}

fn props(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = modern_entries(&status, "props")?;
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| prop_value(&status, entry, index))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn prop(backend: &dyn Backend, selector: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = modern_entries(&status, "props")?;
    let (index, entry) = resolve(entries, selector, "title")?;
    prop_value(&status, entry, index)
}

fn prop_value(status: &Value, entry: &Value, index: usize) -> Result<Value, String> {
    let (source_id, name) = item_identity(entry, "title")?;
    let live_ids = status["livePropIds"]
        .as_array()
        .ok_or("Accepted live prop identities are unavailable.")?;
    let active = live_ids
        .iter()
        .map(|id| id.as_str().ok_or("Accepted live prop identity is invalid."))
        .collect::<Result<Vec<_>, _>>()?
        .contains(&source_id);
    let mut value = json!({
        "id": identity(source_id, name, index),
        "is_active": active,
        "auto_clear_enabled": false,
        "auto_clear_duration": 0,
        "transition": Value::Null
    });
    if let Some(seconds) = entry.get("autoClearSeconds") {
        if !seconds.is_null() {
            let seconds = seconds
                .as_f64()
                .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
                .ok_or("Prop auto-clear duration is invalid.")?;
            value["auto_clear_duration"] = json!(seconds);
            value["auto_clear_enabled"] = json!(true);
        }
    }
    if let Some(transition) = entry.get("transition").filter(|value| !value.is_null()) {
        value["transition"] = prop_transition(transition)?;
    }
    Ok(value)
}

fn prop_transition(transition: &Value) -> Result<Value, String> {
    let effect_id = transition["effectId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Prop transition identity is unavailable.")?;
    let effect = crate::native_transitions::CATALOG
        .iter()
        .find(|effect| effect.id.eq_ignore_ascii_case(effect_id))
        .ok_or("Prop transition is not in the supported transition catalog.")?;
    let duration = transition["durationSeconds"]
        .as_f64()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .ok_or("Prop transition duration is invalid.")?;
    Ok(json!({ "uuid": effect.id, "name": effect.name, "duration": duration }))
}

fn collection_names(status: &Value) -> Result<Vec<String>, String> {
    let declared = modern_entries(status, "propFolders")?;
    let props = modern_entries(status, "props")?;
    let mut names = Vec::new();
    for folder in declared {
        let folder = folder
            .as_str()
            .ok_or("Prop folder name is invalid.")?
            .to_string();
        if names.contains(&folder) {
            return Err("Prop folder names must be unique.".into());
        }
        names.push(folder);
    }
    for prop in props {
        let folder = prop_folder(prop)?.to_string();
        if !names.contains(&folder) {
            names.push(folder);
        }
    }
    Ok(names)
}

fn prop_folder(prop: &Value) -> Result<&str, String> {
    match prop.get("folder") {
        None | Some(Value::Null) => Ok(""),
        Some(Value::String(folder)) => Ok(folder),
        Some(_) => Err("Prop folder identity is invalid.".into()),
    }
}

fn prop_collections(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let names = collection_names(&status)?;
    names
        .iter()
        .enumerate()
        .map(|(index, name)| prop_collection_value(&status, name, index))
        .collect::<Result<Vec<_>, _>>()
        .map(|collections| json!({ "prop_collections": { "collections": collections } }))
}

fn prop_collection(backend: &dyn Backend, selector: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let names = collection_names(&status)?;
    let matches = names
        .iter()
        .enumerate()
        .filter(|(index, name)| matches_id(name, name, *index, selector))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [(index, name)] => prop_collection_value(&status, name, *index),
        [] => Err("The requested prop collection is unavailable.".into()),
        _ => Err("The requested prop collection is ambiguous.".into()),
    }
}

fn prop_collection_value(status: &Value, name: &str, index: usize) -> Result<Value, String> {
    let props = modern_entries(status, "props")?;
    let mut members = Vec::new();
    for (prop_index, entry) in props.iter().enumerate() {
        if prop_folder(entry)? == name {
            members.push(prop_value(status, entry, prop_index)?);
        }
    }
    // Native Remote prop triggers preserve other active props in the collection.
    Ok(json!({
        "id": identity(name, name, index),
        "props": members,
        "single_prop_enabled": false
    }))
}

fn messages(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    modern_entries(&status, "messages")?
        .iter()
        .enumerate()
        .map(|(index, entry)| message_value(&status, entry, index))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn message(backend: &dyn Backend, selector: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = modern_entries(&status, "messages")?;
    let (index, entry) = resolve(entries, selector, "title")?;
    message_value(&status, entry, index)
}

fn message_value(status: &Value, entry: &Value, index: usize) -> Result<Value, String> {
    let (source_id, title) = item_identity(entry, "title")?;
    let text = entry["text"]
        .as_str()
        .ok_or("Message text is unavailable.")?;
    // Orbit messages without a theme use the built-in default; visibleOnNetwork is optional and defaults to visible.
    let theme = match entry.get("themeId").filter(|id| !id.is_null()) {
        None => identity("default-message-theme", "Default", 0),
        Some(theme_id) => {
            let theme_id = theme_id
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .ok_or("Message theme identity is unavailable.")?;
            let themes = modern_entries(status, "themes")?;
            let theme_matches = themes
                .iter()
                .enumerate()
                .filter(|(_, theme)| theme["id"].as_str() == Some(theme_id))
                .collect::<Vec<_>>();
            let (theme_index, theme) = match theme_matches.as_slice() {
                [(index, theme)] => (*index, *theme),
                [] => return Err("Message theme is unavailable in the workspace catalog.".into()),
                _ => return Err("Message theme identity is ambiguous.".into()),
            };
            let (theme_source_id, theme_name) = item_identity(theme, "title")?;
            identity(theme_source_id, theme_name, theme_index)
        }
    };
    let visible_on_network = match entry.get("visibleOnNetwork") {
        None | Some(Value::Null) => true,
        Some(value) => value
            .as_bool()
            .ok_or("Message network visibility is invalid.")?,
    };
    let active_id = active_message_id(status)?;
    let tokens = message_tokens(status, entry)?;
    Ok(json!({
        "id": identity(source_id, title, index),
        "is_active": active_id == Some(source_id),
        "message": text,
        "theme": theme,
        "tokens": tokens,
        "visible_on_network": visible_on_network
    }))
}

fn active_message_id(status: &Value) -> Result<Option<&str>, String> {
    match status.get("activeMessage") {
        Some(Value::Null) => Ok(None),
        Some(Value::Object(active)) => active
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(Some)
            .ok_or_else(|| "Accepted active message identity is invalid.".into()),
        Some(_) => Err("Accepted active message status is invalid.".into()),
        None => Err("Accepted active message status is unavailable.".into()),
    }
}

fn message_tokens(status: &Value, entry: &Value) -> Result<Vec<Value>, String> {
    let Some(value) = entry.get("tokens") else {
        return Ok(Vec::new());
    };
    let tokens = value
        .as_array()
        .filter(|tokens| tokens.len() <= MAX_TOKEN_COUNT)
        .ok_or("Message tokens are invalid.")?;
    let values = entry.get("tokenValues").and_then(Value::as_object);
    let mut names = std::collections::HashSet::new();
    tokens
        .iter()
        .enumerate()
        .map(|(index, token)| {
            let name = token["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .ok_or("Message token name is unavailable.")?;
            if !names.insert(name) {
                return Err("Message token names must be unique.".into());
            }
            let mut api_token = json!({"name":name});
            if let Some(uuid) = token_uuid(token, name, index)? {
                api_token["uuid"] = json!(uuid);
            }
            match token["type"].as_str() {
                Some("text") => {
                    let text = values
                        .and_then(|values| values.get(name))
                        .and_then(Value::as_str)
                        .filter(|text| text.chars().count() <= MAX_TOKEN_TEXT_CHARS)
                        .ok_or_else(|| {
                            format!("Message text token value is unavailable: {name}.")
                        })?;
                    api_token["text"] = json!({"text":text});
                    Ok(api_token)
                }
                Some("clock") => {
                    api_token["clock"] = json!({
                        "date":"none",
                        "time":"long",
                        "is_24_hours":true
                    });
                    Ok(api_token)
                }
                Some("timer") => {
                    api_token["timer"] = timer_token_value(status, token)?;
                    Ok(api_token)
                }
                _ => Err("Message token type is unavailable or unsupported.".into()),
            }
        })
        .collect()
}

fn token_uuid(token: &Value, name: &str, index: usize) -> Result<Option<String>, String> {
    match token.get("id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(source_id)) if !source_id.trim().is_empty() => {
            identity(source_id, name, index)["uuid"]
                .as_str()
                .map(str::to_string)
                .map(Some)
                .ok_or_else(|| "Message token UUID is unavailable.".into())
        }
        Some(_) => Err("Message token identity is invalid.".into()),
    }
}

fn timer_token_value(status: &Value, token: &Value) -> Result<Value, String> {
    let timer_id = token["timerId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Message timer target is unavailable.")?;
    let timers = status["timers"]
        .as_array()
        .ok_or("Accepted timer status is unavailable.")?;
    let matches = timers
        .iter()
        .enumerate()
        .filter(|(_, timer)| timer["id"].as_str() == Some(timer_id))
        .collect::<Vec<_>>();
    let [(index, timer)] = matches.as_slice() else {
        return Err(if matches.is_empty() {
            "Message timer target is unavailable.".into()
        } else {
            "Message timer target is ambiguous.".into()
        });
    };
    Ok(json!({
        "configuration": super::controls::timer_value(timer, *index)?,
        "format": {
            "hour":"remove_long",
            "minute":"remove_long",
            "second":"short",
            "millisecond":"none"
        }
    }))
}

fn trigger_message(
    backend: &dyn Backend,
    selector: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = modern_entries(&status, "messages")?;
    let (_, entry) = resolve(entries, selector, "title")?;
    let source_id = entry["id"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .ok_or("Message identity is unavailable.")?;
    let workspace = data::workspace_path(&status)?;
    if let Some(values) = trigger_text_values(&status, entry, body)? {
        backend.command(&json!({
            "command": "messageDefinition",
            "workspacePath": workspace,
            "id": source_id,
            "tokenValues": values
        }))?;
    }
    backend.command(&json!({
        "command": "message",
        "action": "show",
        "workspacePath": workspace,
        "id": source_id
    }))?;
    Ok(Value::Null)
}

fn trigger_text_values(
    status: &Value,
    entry: &Value,
    body: Option<&Value>,
) -> Result<Option<Value>, String> {
    let Some(body) = body else {
        return Ok(None);
    };
    let tokens = body
        .as_array()
        .filter(|tokens| tokens.len() <= MAX_TOKEN_COUNT)
        .ok_or("Message trigger body must be a bounded token array.")?;
    let definitions = message_tokens(status, entry)?;
    let mut values = serde_json::Map::new();
    let mut seen = std::collections::HashSet::new();
    for token in tokens {
        let object = token
            .as_object()
            .ok_or("Message trigger tokens must be objects.")?;
        let name = token["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or("Message trigger token name is required.")?;
        if !seen.insert(name) {
            return Err("Message trigger contains duplicate token names.".into());
        }
        let expected = definitions
            .iter()
            .find(|definition| definition["name"].as_str() == Some(name))
            .ok_or_else(|| format!("Message token {name} is unavailable."))?;
        if let Some(uuid) = token.get("uuid") {
            if uuid.as_str() != expected["uuid"].as_str() {
                return Err(format!("Message token {name} identity does not match."));
            }
        }
        if expected.get("text").is_some() {
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "name" | "text" | "uuid"))
            {
                return Err("Only text message token overrides are supported.".into());
            }
            let text_object = token["text"]
                .as_object()
                .filter(|object| object.len() == 1 && object.contains_key("text"))
                .ok_or("Message trigger text must contain only a text value.")?;
            let text = text_object["text"]
                .as_str()
                .filter(|text| text.chars().count() <= MAX_TOKEN_TEXT_CHARS)
                .ok_or("Message trigger text must be a bounded string.")?;
            values.insert(name.to_string(), json!(text));
        } else if expected.get("clock").is_some() {
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "name" | "clock" | "uuid"))
            {
                return Err("Only the unchanged clock token can be sent with a trigger.".into());
            }
            if let Some(clock) = token.get("clock") {
                let expected_clock = &expected["clock"];
                let wrapped = json!({"format":expected_clock});
                if clock != expected_clock && clock != &wrapped {
                    return Err("Clock token configuration cannot be changed by a trigger.".into());
                }
            }
        } else if expected.get("timer").is_some() {
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "name" | "timer" | "uuid"))
            {
                return Err("Only the unchanged timer token can be sent with a trigger.".into());
            }
            if let Some(timer) = token.get("timer") {
                if timer != &expected["timer"] {
                    return Err("Timer token configuration cannot be changed by a trigger.".into());
                }
            }
        } else {
            return Err(format!(
                "Message token {name} is unavailable or unsupported."
            ));
        }
    }
    Ok((!values.is_empty()).then_some(Value::Object(values)))
}

fn clear_message(backend: &dyn Backend, selector: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = modern_entries(&status, "messages")?;
    let (_, entry) = resolve(entries, selector, "title")?;
    let source_id = entry["id"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .ok_or("Message identity is unavailable.")?;
    if active_message_id(&status)? != Some(source_id) {
        return Err("The requested message is not currently active.".into());
    }
    backend.command(&json!({
        "command": "message",
        "action": "clear",
        "workspacePath": data::workspace_path(&status)?
    }))?;
    Ok(Value::Null)
}

fn prop_command(backend: &dyn Backend, selector: &str, action: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = modern_entries(&status, "props")?;
    let (_, prop) = resolve(entries, selector, "title")?;
    let source_id = prop["id"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .ok_or("Prop identity is unavailable.")?;
    backend.command(&json!({
        "command": "prop",
        "action": action,
        "id": source_id
    }))?;
    Ok(Value::Null)
}
