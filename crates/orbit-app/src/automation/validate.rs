//! Authoring validation: what React's `action-authoring.ts`, `macro-authoring.ts`, `macro-trigger-validation.ts`,
//! `message-controller.ts`, and `schedule-controller.ts` refuse before a save. Each returns the translation key of
//! the first problem, or `None`.

use serde_json::Value;

use super::runner::Facts;
use crate::keys::{normalize_shortcut_key, KeyMappings};

const TIMER_COMMANDS: [&str; 6] = [
    "start",
    "stop",
    "reset",
    "reset-and-start",
    "stop-and-reset",
    "increment",
];
pub const CLEAR_LAYERS: [&str; 7] = [
    "all",
    "slide",
    "background",
    "prop",
    "audio",
    "messages",
    "video-input",
];
pub const ACTION_TYPES: [&str; 17] = [
    "clear",
    "clear-group",
    "macro",
    "media",
    "video-input",
    "audio-input",
    "message",
    "presentation",
    "playlist",
    "prop",
    "stage",
    "audience-look",
    "mask",
    "timer",
    "transport",
    "communication",
    "capture",
];

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

/// `actionTargetOptions`: the entries an action of `kind` may target.
pub fn target_options(workspace: &Value, kind: &str, excluded_macro: &str) -> Vec<Value> {
    let list = |key: &str| -> Vec<Value> { workspace[key].as_array().cloned().unwrap_or_default() };
    let keep = |key: &str, test: fn(&Value) -> bool| -> Vec<Value> {
        list(key).into_iter().filter(test).collect()
    };
    match kind {
        "clear-group" => list("clearGroups"),
        "media" => list("media"),
        "message" => list("messages"),
        "presentation" => list("library"),
        "prop" => list("props"),
        "stage" => list("stageLayouts"),
        "audience-look" => list("audienceLooks"),
        "mask" => list("masks"),
        "audio-input" => keep("audioInputs", |input| !input["capture"].is_null()),
        "video-input" => keep("videoInputs", |input| {
            !input["ndiSource"].is_null() || !input["cameraSource"].is_null()
        }),
        "capture" => keep("recordingPresets", |preset| !preset["capture"].is_null()),
        "communication" => keep("communicationDevices", |device| {
            device["input"].is_null()
                && match device["transport"].as_str() {
                    Some("midi") => !device["midi"].is_null(),
                    Some("tcp" | "udp") => true,
                    Some("serial") => !device["serial"].is_null(),
                    _ => false,
                }
        }),
        "macro" => list("macros")
            .into_iter()
            .filter(|entry| entry["id"] != excluded_macro)
            .collect(),
        "playlist" => keep("playlist", |entry| entry["type"] != "folder"),
        "timer" => list("countdownTimers")
            .into_iter()
            .map(|timer| serde_json::json!({"id": timer["id"], "title": timer["name"]}))
            .collect(),
        _ => Vec::new(),
    }
}

/// `isAuthoredAction`: imported actions the editor has no fields for are kept as they are, never validated.
fn authored(action: &Value) -> bool {
    let kind = text(action, "type");
    if kind == "audio-input"
        && text(action, "targetId").is_empty()
        && !action["details"]["index"].is_null()
    {
        return false;
    }
    if kind == "capture"
        && action["details"].is_object()
        && action["details"].get("start").is_none()
        && action["details"].get("stop").is_none()
    {
        return false;
    }
    if kind == "communication"
        && action["details"].is_object()
        && action.get("targetId").is_none()
        && action.get("value").is_none()
    {
        return false;
    }
    ACTION_TYPES.contains(&kind)
}

fn capture_error(action: &Value, facts: &Facts) -> Option<&'static str> {
    let details = &action["details"];
    let (start, stop) = (details.get("start"), details.get("stop"));
    let invalid = Some("automation.macros.targetInvalid");
    if start.is_some() && stop.is_some() {
        return invalid;
    }
    if let Some(start) = start {
        let Some(id) = start["presetIdentification"]
            .as_str()
            .or_else(|| start["preset_identification"].as_str())
        else {
            return invalid;
        };
        let found = facts.workspace["recordingPresets"]
            .as_array()
            .is_some_and(|presets| {
                presets.iter().any(|preset| {
                    (if id.is_empty() {
                        preset["active"] == true
                    } else {
                        preset["id"] == id
                    }) && !preset["capture"].is_null()
                })
            });
        return if found { None } else { invalid };
    }
    match stop {
        Some(Value::Bool(true)) => None,
        Some(stop) if stop["showsAlertBeforeStopping"].is_boolean() => None,
        _ => invalid,
    }
}

/// `actionValidationError`. `excluded_macro` is the macro being edited: it cannot call itself.
pub fn action_error(action: &Value, facts: &Facts, excluded_macro: &str) -> Option<String> {
    if !authored(action) {
        return None;
    }
    let invalid = || Some("automation.macros.targetInvalid".to_string());
    let delay = action.get("delaySeconds").filter(|delay| !delay.is_null());
    if delay.is_some_and(|delay| {
        delay
            .as_f64()
            .is_none_or(|delay| !delay.is_finite() || delay < 0.)
    }) {
        return Some("automation.macros.delayInvalid".into());
    }
    let kind = text(action, "type");
    if kind == "capture" {
        return capture_error(action, facts).map(str::to_string);
    }
    if !matches!(kind, "clear" | "transport") {
        let target = text(action, "targetId");
        let found = target_options(&facts.workspace, kind, excluded_macro)
            .iter()
            .any(|entry| {
                entry["id"] == target
                    || (matches!(kind, "presentation" | "playlist")
                        && !entry["sourceId"].is_null()
                        && entry["sourceId"] == target)
            });
        if !found {
            return invalid();
        }
    }
    let details = &action["details"];
    if kind == "presentation" {
        let resolved = super::runner::jump_target(
            facts,
            text(action, "targetId"),
            details["arrangementId"].as_str(),
            None,
            details,
        );
        if !matches!(resolved, Ok(Some(_))) {
            return invalid();
        }
    }
    if kind == "media" {
        if let Some(transport) = details.get("mediaTransport") {
            let video = facts.workspace["media"].as_array().is_some_and(|media| {
                media
                    .iter()
                    .any(|item| item["id"] == action["targetId"] && item["type"] == "video")
            });
            if !video || orbit_host::validate_preview_media_transport(transport).is_err() {
                return invalid();
            }
        }
    }
    match kind {
        "audio-input" => {
            let mut scratch = super::runner::Run::default();
            super::runner::audio_input(facts, action, &mut scratch)
                .err()
                .map(|_| "audioInputs.invalid".to_string())
        }
        "presentation" => {
            let destination = &details["contentDestination"];
            if !destination.is_null() && !matches!(destination.as_u64(), Some(0 | 1)) {
                return invalid();
            }
            None
        }
        "playlist" => ["selectPlaylist", "alwaysRetrigger"]
            .iter()
            .any(|key| !details[*key].is_null() && !details[*key].is_boolean())
            .then(|| "automation.macros.targetInvalid".to_string()),
        "clear" => (!CLEAR_LAYERS.contains(&action["targetLayer"].as_str().unwrap_or("all")))
            .then(|| "automation.macros.targetInvalid".to_string()),
        "communication" => {
            let encoding = details["payloadEncoding"].as_str().unwrap_or("utf8");
            let payload = action["value"].as_str().unwrap_or("");
            crate::settings::devices::encode_payload(payload, encoding)
                .err()
                .map(str::to_string)
        }
        "timer" => {
            let command = action["value"].as_str().unwrap_or("start");
            if !TIMER_COMMANDS.contains(&command) {
                return invalid();
            }
            (command == "increment"
                && details["incrementSeconds"]
                    .as_f64()
                    .is_none_or(|seconds| !seconds.is_finite()))
            .then(|| "automation.macros.delayInvalid".to_string())
        }
        "transport" => {
            let jump = details.get("jumpToTime")?;
            let time = jump["time"].as_f64();
            (!jump.is_object() || time.is_none_or(|time| !time.is_finite() || time < 0.))
                .then(|| "automation.macros.targetInvalid".to_string())
        }
        "prop" => (!matches!(action["value"].as_str().unwrap_or("show"), "show" | "clear"))
            .then(|| "automation.macros.targetInvalid".to_string()),
        _ => None,
    }
}

fn canonical_key(key: &str) -> String {
    match key {
        " " | "Space" => "Space".into(),
        "Right" | "ArrowRight" => "ArrowRight".into(),
        "Left" | "ArrowLeft" => "ArrowLeft".into(),
        "Up" | "ArrowUp" => "ArrowUp".into(),
        "Down" | "ArrowDown" => "ArrowDown".into(),
        "Enter" | "Escape" | "Tab" | "Backspace" | "Delete" => key.into(),
        _ if key.chars().count() == 1 => key.to_uppercase(),
        _ => {
            let digits = key.strip_prefix(['f', 'F']).unwrap_or("");
            match digits.parse::<u8>() {
                Ok(1..=12) if !digits.starts_with('0') && !digits.starts_with('+') => {
                    key.to_uppercase()
                }
                _ => String::new(),
            }
        }
    }
}

/// `canonicalMacroShortcut`: modifiers in the fixed order Ctrl, Meta, Alt, Shift; empty when invalid.
pub fn canonical_shortcut(value: &str) -> String {
    let mut parts: Vec<&str> = value.split('+').collect();
    let key = canonical_key(parts.pop().unwrap_or(""));
    let known = ["Ctrl", "Meta", "Alt", "Shift"];
    if key.is_empty()
        || parts.iter().any(|part| !known.contains(part))
        || (1..parts.len()).any(|index| parts[..index].contains(&parts[index]))
    {
        return String::new();
    }
    known
        .iter()
        .filter(|modifier| parts.contains(modifier))
        .map(|modifier| modifier.to_string())
        .chain([key])
        .collect::<Vec<_>>()
        .join("+")
}

/// `macroShortcutValidationError`. `settings` supplies the key mappings; the group keys and trigger bindings
/// come from the workspace, with the group settings of the settings as the fallback the settings panel uses.
pub fn shortcut_error(workspace: &Value, settings: &Value, entry: &Value) -> Option<String> {
    let hotkey = text(entry, "hotkey");
    if hotkey.is_empty() {
        return None;
    }
    let shortcut = canonical_shortcut(hotkey);
    if shortcut.is_empty() {
        return Some("automation.macros.hotkeyInvalid".into());
    }
    let id = text(entry, "id");
    let mut assigned: Vec<String> = [
        "Ctrl+N",
        "Meta+N",
        "ArrowUp",
        "ArrowDown",
        "Shift+ArrowUp",
        "Shift+ArrowDown",
    ]
    .iter()
    .map(|key| key.to_string())
    .collect();
    assigned.extend(KeyMappings::from_settings(settings).shortcuts());
    let groups = if workspace["groupSettings"].is_object() {
        &workspace["groupSettings"]
    } else {
        &settings["groupSettings"]
    };
    for group in groups
        .as_object()
        .into_iter()
        .flat_map(|groups| groups.values())
    {
        let key = normalize_shortcut_key(text(group, "key"));
        if !key.is_empty() {
            assigned.push(format!("Shift+{key}"));
            assigned.push(key);
        }
    }
    for binding in workspace["triggerBindings"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if binding["type"] != "macro" || binding["targetId"] != id {
            assigned.push(text(binding, "shortcut").to_string());
        }
    }
    for other in workspace["macros"].as_array().into_iter().flatten() {
        if other["id"] != id {
            assigned.push(text(other, "hotkey").to_string());
        }
    }
    assigned
        .iter()
        .any(|key| canonical_shortcut(key) == shortcut)
        .then(|| "automation.macros.hotkeyConflict".into())
}

/// `macroAuthoringValidationError`.
pub fn macro_error(facts: &Facts, settings: &Value, entry: &Value) -> Option<String> {
    if text(entry, "title").trim().is_empty() {
        return Some("automation.macros.nameRequired".into());
    }
    let workspace = &facts.workspace;
    let mut known: Vec<&Value> = workspace["macroCollections"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|collection| &collection["id"])
        .collect();
    known.extend(
        workspace["macros"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|other| other["collectionIds"].as_array().into_iter().flatten()),
    );
    let memberships = entry["collectionIds"].as_array();
    if let Some(ids) = memberships {
        let unique: std::collections::HashSet<_> = ids.iter().map(Value::to_string).collect();
        if unique.len() != ids.len() || ids.iter().any(|id| !known.contains(&id)) {
            return Some("automation.macros.collections.targetInvalid".into());
        }
    }
    let actions = entry["actions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let ids: std::collections::HashSet<_> = actions
        .iter()
        .map(|action| action["id"].to_string())
        .collect();
    if ids.len() != actions.len() {
        return Some("automation.macros.actionsInvalid".into());
    }
    if let Some(error) = shortcut_error(workspace, settings, entry) {
        return Some(error);
    }
    actions
        .iter()
        .find_map(|action| action_error(action, facts, text(entry, "id")))
}

/// `messageValidationError`.
pub fn message_error(workspace: &Value, message: &Value) -> Option<String> {
    if text(message, "title").trim().is_empty() || text(message, "text").trim().is_empty() {
        return Some("automation.messages.required".into());
    }
    let theme = text(message, "themeId");
    if !theme.is_empty() {
        let slide = text(message, "themeSlideId");
        let found = workspace["themes"].as_array().is_some_and(|themes| {
            themes.iter().any(|candidate| {
                candidate["id"] == theme
                    && candidate["slides"].as_array().is_some_and(|slides| {
                        slides
                            .iter()
                            .any(|entry| slide.is_empty() || entry["id"] == slide)
                    })
            })
        });
        if !found {
            return Some("automation.messages.themeInvalid".into());
        }
    }
    let tokens = message["tokens"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    match text(message, "clearType") {
        "after-time"
            if message["clearAfterSeconds"]
                .as_f64()
                .is_none_or(|seconds| !seconds.is_finite() || seconds <= 0.) =>
        {
            return Some("automation.messages.durationInvalid".into())
        }
        "after-timers" if !tokens.iter().any(|token| token["type"] == "timer") => {
            return Some("automation.messages.afterTimersRequired".into())
        }
        _ => {}
    }
    token_error(workspace, message, tokens)
}

fn token_error(workspace: &Value, message: &Value, tokens: &[Value]) -> Option<String> {
    let invalid = || Some("automation.messages.tokensInvalid".to_string());
    let ids: std::collections::HashSet<_> =
        tokens.iter().map(|token| token["id"].to_string()).collect();
    let names: std::collections::HashSet<_> =
        tokens.iter().map(|token| text(token, "name")).collect();
    if ids.len() != tokens.len() || names.len() != tokens.len() {
        return invalid();
    }
    for token in tokens {
        let name = text(token, "name");
        if name.trim().is_empty() || name.contains(['{', '}']) {
            return invalid();
        }
        if token["type"] == "text" && !message["tokenValues"][name].is_string() {
            return Some("automation.messages.tokenMissing".into());
        }
        if token["type"] == "timer"
            && !workspace["countdownTimers"]
                .as_array()
                .is_some_and(|timers| timers.iter().any(|timer| timer["id"] == token["timerId"]))
        {
            return Some("automation.messages.timerInvalid".into());
        }
    }
    let mut known: std::collections::HashSet<&str> = names;
    known.extend(
        message["tokenValues"]
            .as_object()
            .into_iter()
            .flat_map(|values| values.keys().map(String::as_str)),
    );
    for placeholder in placeholders(text(message, "text")) {
        if !known.contains(placeholder) {
            return Some("automation.messages.tokenMissing".into());
        }
    }
    None
}

/// The `{name}` placeholders of a message text: a name is one or more characters that are not braces.
fn placeholders(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(['{', '}']) else {
            break;
        };
        if after[close..].starts_with('}') && close > 0 {
            found.push(&after[..close]);
        }
        rest = &after[close..];
        if rest.starts_with('}') {
            rest = &rest[1..];
        }
    }
    found
}

pub fn timer_type_patch(kind: &str, seconds_of_day: u32) -> Value {
    match kind {
        "countdown" => serde_json::json!({"type": kind, "durationSeconds": 300}),
        "countdown-to-time" => {
            serde_json::json!({"type": kind, "targetTimeSeconds": seconds_of_day})
        }
        _ => serde_json::json!({"type": kind, "elapsedStartSeconds": 0, "elapsedEndSeconds": 0}),
    }
}
