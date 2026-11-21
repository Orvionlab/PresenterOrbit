use serde_json::{Map, Value};
use std::collections::HashSet;

pub fn validate_changed(previous: &Value, document: &Map<String, Value>) -> Result<(), String> {
    if previous.get("triggerBindings") == document.get("triggerBindings") {
        return Ok(());
    }
    let bindings = document
        .get("triggerBindings")
        .and_then(Value::as_array)
        .ok_or("Trigger bindings must be an array.")?;
    let old = previous.get("triggerBindings").and_then(Value::as_array);
    let mut ids = HashSet::new();
    for binding in bindings {
        let id = binding
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or("Trigger binding id is required.")?;
        if !ids.insert(id) {
            return Err("Trigger binding ids must be unique.".into());
        }
        if old.is_some_and(|entries| entries.contains(binding)) {
            continue;
        }
        let kind = binding
            .get("type")
            .and_then(Value::as_str)
            .ok_or("Trigger binding type is required.")?;
        let target = binding
            .get("targetId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("Trigger binding target is required.")?;
        if !known_target(document, kind, target) {
            return Err("Trigger binding target is missing or unsupported.".into());
        }
        let shortcut = binding
            .get("shortcut")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let midi = binding.get("midi").filter(|value| !value.is_null());
        if shortcut.is_none() && midi.is_none() {
            return Err("Trigger binding input is required.".into());
        }
        if let Some(shortcut) = shortcut {
            let canonical = canonical_shortcut(shortcut)?;
            if bindings.iter().any(|entry| {
                entry.get("id").and_then(Value::as_str) != Some(id)
                    && entry
                        .get("shortcut")
                        .and_then(Value::as_str)
                        .is_some_and(|value| canonical_shortcut(value).as_ref() == Ok(&canonical))
            }) {
                return Err("Trigger binding shortcut is already assigned.".into());
            }
            if shortcut_conflicts(document, &canonical) {
                return Err(
                    "Trigger binding shortcut conflicts with a command, group or macro.".into(),
                );
            }
        }
        if let Some(midi) = midi {
            for (field, min, max) in [("channel", 1, 16), ("pitch", 0, 127), ("velocity", 1, 127)] {
                if !midi
                    .get(field)
                    .and_then(Value::as_u64)
                    .is_some_and(|value| value >= min && value <= max)
                {
                    return Err("Trigger binding MIDI input is invalid.".into());
                }
            }
            if bindings.iter().any(|entry| {
                entry.get("id").and_then(Value::as_str) != Some(id)
                    && entry.get("midi").is_some_and(|other| {
                        other.get("channel") == midi.get("channel")
                            && other.get("pitch") == midi.get("pitch")
                    })
            }) {
                return Err("Trigger binding MIDI input is already assigned.".into());
            }
        }
    }
    Ok(())
}

fn canonical_shortcut(value: &str) -> Result<String, String> {
    let mut parts = value.split('+').collect::<Vec<_>>();
    let key = parts.pop().unwrap_or("");
    if key.is_empty()
        || parts
            .iter()
            .any(|part| !["Ctrl", "Meta", "Alt", "Shift"].contains(part))
        || parts.iter().collect::<HashSet<_>>().len() != parts.len()
    {
        return Err("Trigger binding shortcut is invalid.".into());
    }
    let key = match key {
        "Right" => "ArrowRight".into(),
        "Left" => "ArrowLeft".into(),
        "Up" => "ArrowUp".into(),
        "Down" => "ArrowDown".into(),
        key if key.chars().count() == 1 => key.to_uppercase(),
        key if [
            "ArrowRight",
            "ArrowLeft",
            "ArrowUp",
            "ArrowDown",
            "Enter",
            "Escape",
            "Space",
            "Tab",
            "Backspace",
            "Delete",
        ]
        .contains(&key)
            || key
                .strip_prefix('F')
                .and_then(|value| value.parse::<u8>().ok())
                .is_some_and(|value| (1..=12).contains(&value)) =>
        {
            key.into()
        }
        _ => return Err("Trigger binding shortcut is invalid.".into()),
    };
    Ok(["Ctrl", "Meta", "Alt", "Shift"]
        .into_iter()
        .filter(|modifier| parts.contains(modifier))
        .chain(std::iter::once(key.as_str()))
        .collect::<Vec<_>>()
        .join("+"))
}

fn shortcut_conflicts(document: &Map<String, Value>, shortcut: &str) -> bool {
    let matches = |value: &str| canonical_shortcut(value).as_deref() == Ok(shortcut);
    let reserved = [
        "Ctrl+N",
        "Meta+N",
        "ArrowUp",
        "ArrowDown",
        "Shift+ArrowUp",
        "Shift+ArrowDown",
    ];
    if reserved.into_iter().any(matches)
        || crate::settings::normalize_key_mappings_value(
            &document.get("keyMappings").cloned().unwrap_or(Value::Null),
        )
        .as_object()
        .is_some_and(|mappings| mappings.values().filter_map(Value::as_str).any(matches))
    {
        return true;
    }
    if document
        .get("macros")
        .and_then(Value::as_array)
        .is_some_and(|macros| {
            macros
                .iter()
                .filter_map(|entry| entry.get("hotkey").and_then(Value::as_str))
                .any(matches)
        })
    {
        return true;
    }
    document
        .get("groupSettings")
        .and_then(Value::as_object)
        .is_some_and(|groups| {
            groups
                .values()
                .filter_map(|group| group.get("key").and_then(Value::as_str))
                .filter_map(|key| key.trim().chars().next())
                .any(|key| {
                    matches(&key.to_uppercase().to_string())
                        || matches(&format!("Shift+{}", key.to_uppercase()))
                })
        })
}

fn known_target(document: &Map<String, Value>, kind: &str, target: &str) -> bool {
    let entries = |key: &str| {
        document
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    };
    let slides = || {
        entries("library").flat_map(|item| {
            item.get("slides")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
    };
    match kind {
        "command" => matches!(
            target,
            "newSlide"
                | "nextSlide"
                | "previousSlide"
                | "nextBackground"
                | "previousBackground"
                | "clearAll"
                | "clearSlide"
                | "clearMedia"
                | "clearProps"
                | "clearAudio"
                | "clearMessages"
                | "clearAnnouncements"
                | "clearToLogo"
        ),
        "cue" => slides().any(|slide| {
            slide
                .get("sourceCueId")
                .or_else(|| slide.get("id"))
                .and_then(Value::as_str)
                == Some(target)
        }),
        "group" => {
            document
                .get("groupSettings")
                .and_then(Value::as_object)
                .is_some_and(|groups| groups.contains_key(target))
                || slides()
                    .any(|slide| slide.get("sourceGroupId").and_then(Value::as_str) == Some(target))
        }
        "clear-group" => {
            entries("clearGroups")
                .any(|entry| entry.get("id").and_then(Value::as_str) == Some(target))
                || slides()
                    .any(|slide| slide.get("sourceGroupId").and_then(Value::as_str) == Some(target))
        }
        "macro" | "prop" | "timer" => entries(match kind {
            "macro" => "macros",
            "prop" => "props",
            _ => "countdownTimers",
        })
        .any(|entry| entry.get("id").and_then(Value::as_str) == Some(target)),
        _ => false,
    }
}
