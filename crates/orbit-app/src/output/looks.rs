//! Applying an Audience Look, from the settings editor and the toolbar switcher. The look's entries become the
//! screens' own `lookLayers` and `lookThemes`; the live look id is the output's `activeAudienceLookId`.
//! Each patch also carries `lookTransition` (the look's seconds, 0.1 when unset, and a per-application token),
//! which the output compositor fades the whole frame over.

use gpui::App;
use serde_json::{json, Value};

use crate::state::runtime::Runtime;

pub struct LiveLook {
    pub current: Value,
    pub saved: Value,
}

pub fn live_look<'a>(runtime: &Runtime, cx: &'a App) -> Option<&'a LiveLook> {
    let output = runtime.output.read(cx);
    output
        .audience_look
        .as_ref()
        .filter(|look| look.current["id"] == output.live["activeAudienceLookId"])
}

pub fn live_id(runtime: &Runtime, cx: &App) -> Option<String> {
    runtime.output.read(cx).live["activeAudienceLookId"]
        .as_str()
        .map(str::to_string)
}

/// The presets of the active workspace.
pub fn presets(runtime: &Runtime, cx: &App) -> Vec<Value> {
    runtime.content.read(cx).workspace["audienceLooks"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The applied screens as the output sees them, or an error while no output state exists yet.
pub fn screen_configs(runtime: &Runtime, cx: &App) -> Result<Value, String> {
    runtime
        .output
        .read(cx)
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot["screenConfig"].clone())
        .filter(Value::is_object)
        .ok_or_else(|| "The screens are not loaded yet.".to_string())
}

/// Writes each changed screen's look state and marks the look live.
pub fn activate(
    runtime: &Runtime,
    configs: &Value,
    look: &Value,
    cx: &mut App,
) -> Result<(), String> {
    let id = look["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("An Audience Look id is required.")?
        .to_string();
    // A newly created workspace has a Default preset before any audience screen exists.
    let mut look = look.clone();
    if id == "default-look" && look["screenLooks"].as_array().is_some_and(Vec::is_empty) {
        look["screenLooks"] = json!(configs
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, config)| config["type"] == "audience")
            .map(|(id, _)| json!({"screenId": id, "lookLayers": {}}))
            .collect::<Vec<_>>());
    }
    orbit_host::audience_looks::activation_patches(configs, &look)?;
    crate::settings::complete_audience_look(&mut look, configs);
    apply(runtime, configs, &look, cx)?;
    runtime.output.update(cx, |output, cx| {
        output.audience_look = Some(LiveLook {
            current: look.clone(),
            saved: look,
        });
        output.edit_live(|live| live["activeAudienceLookId"] = json!(id));
        cx.notify();
    });
    Ok(())
}

pub fn edit(runtime: &Runtime, configs: &Value, look: &Value, cx: &mut App) -> Result<(), String> {
    if live_id(runtime, cx).as_deref() != look["id"].as_str() || live_look(runtime, cx).is_none() {
        return Err("The live Audience Look is unavailable.".into());
    }
    apply(runtime, configs, look, cx)?;
    runtime.output.update(cx, |output, cx| {
        if let Some(active) = output.audience_look.as_mut() {
            active.current = look.clone();
        }
        cx.notify();
    });
    Ok(())
}

fn apply(runtime: &Runtime, configs: &Value, look: &Value, cx: &mut App) -> Result<(), String> {
    for (screen, patch) in orbit_host::audience_looks::activation_patches(configs, look)? {
        super::windows::mutate_config(
            runtime,
            json!({"type": "update", "screenId": screen, "patch": patch}),
            cx,
        );
    }
    Ok(())
}
