use super::{
    active_workspace, argument, canonical_output_content, expected_workspace_revision,
    lock_workspace_io, settings_path, workspace, AppState,
};
use crate::capture::{stream, CapturePreset};
use crate::Host;
use serde_json::{json, Value};
use std::path::PathBuf;

fn stream_keys_path(app: &Host) -> Result<PathBuf, String> {
    Ok(settings_path(app)?.with_file_name("stream-keys.json"))
}

pub fn select_capture_directory() -> Result<Value, String> {
    Ok(json!(rfd::FileDialog::new()
        .set_title("Choose recording directory")
        .pick_folder()
        .map(|path| path.to_string_lossy().into_owned())))
}

pub fn save_capture_preset(app: &Host, state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut value = argument(args, 1)?;
    // The stream key travels with the save request only; it is stored outside the workspace.
    let new_key = value
        .as_object_mut()
        .and_then(|preset| preset.remove("streamKey"))
        .and_then(|key| {
            key.as_str()
                .filter(|key| !key.is_empty())
                .map(str::to_string)
        });
    let preset: CapturePreset =
        serde_json::from_value(value).map_err(|e| format!("Invalid recording preset: {e}"))?;
    preset.validate()?;
    let mut io = lock_workspace_io(state)?;
    io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let keys = stream_keys_path(app)?;
    if preset.stream_url.is_some() {
        let key = match new_key {
            Some(key) => key,
            None => stream::read_key(&keys, &preset.id)?
                .ok_or("Enter the stream key for this preset.")?,
        };
        stream::validate_key(&key)?;
        stream::write_key(&keys, &preset.id, Some(&key))?;
    } else {
        stream::write_key(&keys, &preset.id, None)?;
    }
    let current = workspace::read_workspace(&path)?;
    let presets = upsert_preset(&current, &preset)?;
    let saved = workspace::save_workspace(&path, &json!({"recordingPresets":presets}))?;
    Ok(json!({"workspace":saved,"revision":io.advance_revision(&path)}))
}

fn upsert_preset(current: &Value, preset: &CapturePreset) -> Result<Vec<Value>, String> {
    let mut presets = current
        .get("recordingPresets")
        .and_then(Value::as_array)
        .cloned()
        .ok_or("Workspace recordingPresets must be an array.")?;
    for entry in &mut presets {
        if let Some(entry) = entry.as_object_mut() {
            entry.remove("active");
        }
    }
    let entry = json!({"id":preset.id,"title":preset.title,"capture":preset,"active":true});
    if let Some(existing) = presets
        .iter_mut()
        .find(|value| value.get("id") == entry.get("id"))
    {
        let existing = existing
            .as_object_mut()
            .ok_or("Recording preset must be an object.")?;
        existing.insert("title".into(), entry["title"].clone());
        existing.insert("capture".into(), entry["capture"].clone());
        existing.insert("active".into(), json!(true));
    } else {
        presets.push(entry);
    }
    Ok(presets)
}

pub fn delete_capture_preset(
    app: &Host,
    state: &AppState,
    args: &[Value],
) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let id = argument(args, 1)?
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Recording preset id is required.")?
        .to_string();
    let mut io = lock_workspace_io(state)?;
    io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let capture = state
        .capture
        .lock()
        .map_err(|_| "Recording service is unavailable.")?;
    let status = capture.status();
    if (status.active || status.finalizing) && status.preset_id == id {
        return Err("Stop recording before deleting its preset.".into());
    }
    let current = workspace::read_workspace(&path)?;
    let presets = remove_preset(&current, &id)?;
    let saved = workspace::save_workspace(&path, &json!({"recordingPresets":presets}))?;
    stream::write_key(&stream_keys_path(app)?, &id, None)?;
    Ok(json!({"workspace":saved,"revision":io.advance_revision(&path)}))
}

fn remove_preset(current: &Value, id: &str) -> Result<Vec<Value>, String> {
    let presets = current["recordingPresets"]
        .as_array()
        .ok_or("Workspace recordingPresets must be an array.")?;
    if !presets
        .iter()
        .any(|preset| preset["id"].as_str() == Some(id))
    {
        return Err("Recording preset does not exist.".into());
    }
    fn referenced(value: &Value, id: &str) -> bool {
        match value {
            Value::Object(object) => {
                (object.get("type").and_then(Value::as_str) == Some("capture")
                    && ["presetIdentification", "preset_identification"]
                        .iter()
                        .any(|key| value["details"]["start"][key].as_str() == Some(id)))
                    || object.values().any(|child| referenced(child, id))
            }
            Value::Array(values) => values.iter().any(|child| referenced(child, id)),
            _ => false,
        }
    }
    if referenced(current, id) {
        return Err("Recording preset is referenced by a capture action. Change or remove that action first.".into());
    }
    Ok(presets
        .iter()
        .filter(|preset| preset["id"].as_str() != Some(id))
        .cloned()
        .collect())
}

/// Reads the recording preset from the active workspace and its stream key from the app data: what starting a
/// recording needs from the model.
fn resolve_capture(
    app: &Host,
    state: &AppState,
    args: &[Value],
) -> Result<(PathBuf, CapturePreset, Option<String>), String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let id = argument(args, 1)?
        .as_str()
        .ok_or("Recording preset id is required.")?
        .to_string();
    let io = lock_workspace_io(state)?;
    io.require_active(&path)?;
    let current = workspace::read_workspace(&path)?;
    let entry = current
        .get("recordingPresets")
        .and_then(Value::as_array)
        .and_then(|values| {
            values.iter().find(|value| {
                if id.is_empty() {
                    value.get("active").and_then(Value::as_bool) == Some(true)
                } else {
                    value.get("id").and_then(Value::as_str) == Some(&id)
                }
            })
        })
        .ok_or("Recording preset does not exist.")?;
    let preset: CapturePreset = serde_json::from_value(entry.get("capture").cloned().ok_or("This imported preset has no supported file recording configuration. Edit and save it before recording.")?).map_err(|e| format!("Invalid recording preset: {e}"))?;
    let key = match preset.stream_url {
        Some(_) => Some(
            stream::read_key(&stream_keys_path(app)?, &preset.id)?.ok_or(
                "The stream key for this preset is not set. Edit and save the preset to enter it.",
            )?,
        ),
        None => None,
    };
    Ok((path, preset, key))
}

/// `resolveCapture`: the preset and key a recording starts with, for a process that runs the recording but has
/// no workspace. The UI process answers it and sends the result to the output process.
pub fn resolve_capture_start(
    app: &Host,
    state: &AppState,
    args: &[Value],
) -> Result<Value, String> {
    let (_, preset, key) = resolve_capture(app, state, args)?;
    preset.validate()?;
    Ok(json!({ "preset": preset, "streamKey": key }))
}

/// Starts the recording service for an already resolved preset. `workspace` ends the recording when the active
/// workspace changes; a process without a workspace passes `None`, and its controller stops the recording.
fn begin_capture(
    app: &Host,
    state: &AppState,
    preset: CapturePreset,
    key: Option<String>,
    workspace: Option<PathBuf>,
) -> Result<Value, String> {
    let screen = preset.source_screen_id.clone();
    {
        let snapshot = state
            .output_snapshot
            .lock()
            .map_err(|_| "Output snapshot is unavailable.")?;
        let config = snapshot
            .cache
            .as_ref()
            .and_then(|cached| cached.snapshot.get("screenConfig"))
            .and_then(|value| value.get(&screen))
            .ok_or("Recording screen configuration is unavailable.")?;
        validate_capture_screen(config)?;
    }
    let app = app.clone();
    let source = move || {
        let state = app.state::<AppState>();
        if let Some(path) = &workspace {
            if state
                .active_workspace
                .lock()
                .map_err(|_| "Workspace state is unavailable.")?
                .as_ref()
                != Some(path)
            {
                return Err("Recording stopped because its workspace changed.".into());
            }
        }
        let content = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.")?
            .get(&screen)
            .cloned()
            .ok_or("Recording source screen is unavailable.")?;
        canonical_output_content(&state, &screen, content)
    };
    let mut capture = state
        .capture
        .lock()
        .map_err(|_| "Recording service is unavailable.")?;
    let result = match key {
        Some(key) => capture.start_stream(preset, key, source)?,
        None => capture.start(preset, source)?,
    };
    Ok(json!(result))
}

pub fn start_capture(app: &Host, state: &AppState, args: &[Value]) -> Result<Value, String> {
    let (path, preset, key) = resolve_capture(app, state, args)?;
    let io = lock_workspace_io(state)?;
    io.require_active(&path)?;
    let result = begin_capture(app, state, preset, key, Some(path));
    drop(io);
    result
}

/// `startCaptureResolved`: starts a recording from a preset the controller resolved. It is a full preset, so it
/// is validated again here, and it writes only into the preset's recording directory or its stream ingest; it
/// reads no workspace and writes no settings.
pub fn start_capture_resolved(
    app: &Host,
    state: &AppState,
    args: &[Value],
) -> Result<Value, String> {
    let request = argument(args, 0)?;
    let preset: CapturePreset = serde_json::from_value(request["preset"].clone())
        .map_err(|e| format!("Invalid recording preset: {e}"))?;
    preset.validate()?;
    let key = match (&preset.stream_url, request["streamKey"].as_str()) {
        (Some(_), Some(key)) => {
            stream::validate_key(key)?;
            Some(key.to_string())
        }
        (Some(_), None) => return Err("A streaming preset needs its stream key.".into()),
        (None, _) => None,
    };
    begin_capture(app, state, preset, key, None)
}

fn validate_capture_screen(config: &Value) -> Result<(), String> {
    if !matches!(config["type"].as_str(), Some("audience" | "stage")) {
        return Err("Recording requires a configured audience or stage screen.".into());
    }
    if config
        .get("contentDestination")
        .is_some_and(|destination| !matches!(destination.as_str(), Some("main" | "announcements")))
    {
        return Err("Recording screen destination must be main or announcements.".into());
    }
    Ok(())
}

pub fn stop_capture_action(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let mut capture = state
        .capture
        .lock()
        .map_err(|_| "Recording service is unavailable.")?;
    if let Some(expected) = args.first().filter(|value| !value.is_null()) {
        let expected = expected
            .as_str()
            .filter(|path| !path.is_empty())
            .ok_or("Expected recording path is required.")?;
        return Ok(json!(capture.stop_if_current(expected)?));
    }
    Ok(json!(capture.stop()?))
}

pub fn stop_capture(state: &AppState) -> Result<Value, String> {
    Ok(json!(state
        .capture
        .lock()
        .map_err(|_| "Recording service is unavailable.")?
        .stop()?))
}

pub fn capture_status(state: &AppState) -> Result<Value, String> {
    Ok(json!(state
        .capture_status
        .lock()
        .map_err(|_| "Recording status is unavailable.")?
        .clone()))
}

pub(crate) fn attach_status(scene: &mut Value, status: &Value) {
    let Some(object) = scene.as_object_mut() else {
        return;
    };
    let context = object
        .entry("linkedTextContext")
        .or_insert_with(|| json!({}));
    let Some(context) = context.as_object_mut() else {
        return;
    };
    context.insert("captureStatus".into(), json!({"active":status["active"],"finalizing":status["finalizing"],"error":status["error"]}));
    if let Some(resources) = context
        .get_mut("stageSceneResources")
        .and_then(Value::as_object_mut)
    {
        for (key, resource) in resources {
            if key == "outputs" {
                if let Some(outputs) = resource.as_object_mut() {
                    for child in outputs.values_mut() {
                        if let Some(scene) = child.get_mut("scene") {
                            attach_status(scene, status);
                        }
                    }
                }
            } else if let Some(scene) = resource.get_mut("scene") {
                attach_status(scene, status);
            }
        }
    }
}

pub(crate) fn execute_remote(
    app: &Host,
    state: &AppState,
    payload: &Value,
) -> Result<Value, String> {
    let path = active_workspace(state, payload["workspacePath"].clone())?;
    let status = match payload["action"].as_str() {
        Some("start") => start_capture(app, state, &[json!(path), payload["id"].clone()])?,
        Some("stop") => {
            let io = lock_workspace_io(state)?;
            io.require_active(&path)?;
            stop_capture(state)?
        }
        _ => return Err("Unsupported capture action.".into()),
    };
    Ok(
        json!({"ok":!status["error"].is_string(),"error":status["error"],"status":if status["active"] == true {"recording"} else if status["finalizing"] == true {"finalizing"} else if status["error"].is_string() {"error"} else {"idle"},"captureStatus":status}),
    )
}
