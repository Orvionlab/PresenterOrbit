//! Audience Looks: presets of per-screen layer visibility, mask and presentation theme. Presets live in the
//! workspace as `audienceLooks`; applying one writes its entries onto the screens' own `lookLayers` and
//! `lookThemes`, which stay the applied state the output reads.

use serde_json::{json, Map, Value};

/// The longest look transition ProPresenter's Looks window offers.
pub const MAX_TRANSITION_SECONDS: f64 = 10.0;

/// The fade of a look without a `transitionSeconds`; 0 cuts.
pub const DEFAULT_TRANSITION_SECONDS: f64 = 0.1;

/// The seconds a look fades over, after `validate_transitions` accepted it.
pub fn transition_seconds(look: &Value) -> f64 {
    look["transitionSeconds"]
        .as_f64()
        .unwrap_or(DEFAULT_TRANSITION_SECONDS)
}

/// What the output reads to fade an applied look: the duration, and a token that differs per application so
/// only look-driven changes fade. Manual layer toggles leave it untouched and cut.
pub fn transition_patch(look: &Value) -> Value {
    let token = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    json!({"seconds": transition_seconds(look), "token": token})
}

/// `transitionSeconds` is optional on a look; when present it must be a finite number of seconds in range.
pub fn validate_transitions(looks: Option<&Value>) -> Result<(), String> {
    for look in looks.and_then(Value::as_array).into_iter().flatten() {
        let Some(seconds) = look
            .get("transitionSeconds")
            .filter(|value| !value.is_null())
        else {
            continue;
        };
        if !seconds
            .as_f64()
            .is_some_and(|value| (0.0..=MAX_TRANSITION_SECONDS).contains(&value))
        {
            return Err(format!(
                "Audience Look transition must be between 0 and {MAX_TRANSITION_SECONDS} seconds."
            ));
        }
    }
    Ok(())
}

/// The layers and themes a look entry sets over the screen's own; a theme the entry does not name is kept.
pub fn screen_patch(config: &Value, entry: &Value) -> Value {
    let mut layers = config["lookLayers"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    layers.extend(entry["lookLayers"].as_object().cloned().unwrap_or_default());
    let mut themes = config["lookThemes"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for key in ["maskId", "presentationThemeId"] {
        if !entry[key].is_null() {
            themes.insert(key.into(), entry[key].clone());
        }
    }
    json!({"lookLayers": layers, "lookThemes": themes})
}

/// The screen patches applying `look` changes. Every entry must name an existing audience screen; screens the
/// look would not change are left out so applying never rewrites settings for nothing.
pub fn activation_patches(
    screen_configs: &Value,
    look: &Value,
) -> Result<Vec<(String, Value)>, String> {
    let entries = look["screenLooks"]
        .as_array()
        .filter(|entries| !entries.is_empty())
        .ok_or("An Audience Look requires at least one audience screen.")?;
    let mut seen = Map::new();
    let mut patches = Vec::new();
    for entry in entries {
        let id = entry["screenId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Audience Look screen id is required.")?;
        if seen.insert(id.to_string(), Value::Null).is_some() {
            return Err("Audience Look screen identities must be unique.".into());
        }
        let config = screen_configs
            .get(id)
            .filter(|config| config["type"] == "audience")
            .ok_or_else(|| format!("Audience Look screen is unavailable: {id}"))?;
        let mut patch = screen_patch(config, entry);
        if patch["lookLayers"] != config["lookLayers"]
            || patch["lookThemes"] != config["lookThemes"]
        {
            patch["lookTransition"] = transition_patch(look);
            patches.push((id.to_string(), patch));
        }
    }
    Ok(patches)
}
