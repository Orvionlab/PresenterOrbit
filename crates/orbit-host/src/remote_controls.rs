use crate::Host;
use serde_json::{json, Value};

use crate::{native_timers::TimerRegistry, AppState};

struct CachedWorkspace {
    path: std::path::PathBuf,
    revision: u64,
    document: std::sync::Arc<Value>,
    catalog: Option<Value>,
}

impl CachedWorkspace {
    fn catalog(&mut self) -> Result<Value, String> {
        if self.catalog.is_none() {
            self.catalog = Some(workspace_catalog(&self.path, &self.document)?);
        }
        Ok(self.catalog.as_ref().expect("catalog was built").clone())
    }
}

// Keyed by the workspace revision that every workspace write advances; edits made outside Orbit are not observed.
static WORKSPACE_CATALOG: std::sync::Mutex<Option<CachedWorkspace>> = std::sync::Mutex::new(None);

fn workspace_entry<'a>(
    cache: &'a mut Option<CachedWorkspace>,
    path: &std::path::Path,
    revision: u64,
) -> Result<&'a mut CachedWorkspace, String> {
    if !cache
        .as_ref()
        .is_some_and(|cached| cached.path == path && cached.revision == revision)
    {
        *cache = Some(CachedWorkspace {
            path: path.to_path_buf(),
            revision,
            document: std::sync::Arc::new(crate::workspace::read_workspace(path)?),
            catalog: None,
        });
    }
    Ok(cache.as_mut().expect("workspace cache was populated"))
}

/// Cue commands need the document, without building or copying the Remote catalog.
fn cached_workspace(
    path: &std::path::Path,
    revision: u64,
) -> Result<std::sync::Arc<Value>, String> {
    let mut cache = WORKSPACE_CATALOG
        .lock()
        .map_err(|_| "Remote catalog cache is unavailable.")?;
    Ok(workspace_entry(&mut cache, path, revision)?
        .document
        .clone())
}

fn cached_catalog(
    path: &std::path::Path,
    revision: u64,
) -> Result<(std::sync::Arc<Value>, Value), String> {
    let mut cache = WORKSPACE_CATALOG
        .lock()
        .map_err(|_| "Remote catalog cache is unavailable.")?;
    let cached = workspace_entry(&mut cache, path, revision)?;
    Ok((cached.document.clone(), cached.catalog()?))
}

/// Stores a workspace document that was just written at `revision`, so cue commands need not re-read it.
pub(crate) fn prime_workspace_cache(
    path: &std::path::Path,
    revision: u64,
    document: Value,
) -> Result<(), String> {
    *WORKSPACE_CATALOG
        .lock()
        .map_err(|_| "Remote catalog cache is unavailable.")? = Some(CachedWorkspace {
        path: path.to_path_buf(),
        revision,
        document: std::sync::Arc::new(document),
        catalog: None,
    });
    Ok(())
}

pub(crate) fn workspace_snapshot(
    state: &AppState,
    path: &std::path::Path,
) -> Result<(std::sync::Arc<Value>, Value, u64), String> {
    let mut workspace_io = crate::lock_workspace_io(state)?;
    workspace_io.require_active(path)?;
    let revision = workspace_io.revision(path);
    let (document, catalog) = cached_catalog(path, revision)?;
    Ok((document, catalog, revision))
}

pub(crate) fn catalog(app: &Host) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let mut workspace_io = crate::lock_workspace_io(&state)?;
    let path = state
        .active_workspace
        .lock()
        .map_err(|_| "Workspace state is unavailable.")?
        .clone();
    let Some(path) = path else {
        return Ok(
            json!({"workspacePath":null,"libraries":[],"playlists":[],"audienceLooks":[],"macros":[],"messages":[],"timerDefinitions":[],"messageDefinitions":[],"stageLayouts":[],"clearGroups":[],"recordingPresets":[],"mediaBins":[],"tickers":[],"ticker":null,"captureStatus":crate::capture_commands::capture_status(&state)?}),
        );
    };
    workspace_io.require_active(&path)?;
    let revision = workspace_io.revision(&path);
    let (_, mut catalog) = cached_catalog(&path, revision)?;
    catalog["ticker"] =
        crate::ticker_commands::resolve(app, &state, crate::native_ticker::snapshot(&path)?)?;
    catalog["captureStatus"] = crate::capture_commands::capture_status(&state)?;
    Ok(catalog)
}

const UNFILED_AUDIO_PLAYLIST_ID: &str = "orbit-unfiled-audio";

fn workspace_catalog(path: &std::path::Path, workspace: &Value) -> Result<Value, String> {
    let summarize = |field: &str| -> Result<Vec<Value>, String> {
        let entries = workspace
            .get(field)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("Workspace {field} must be an array."))?;
        Ok(entries.iter().map(|entry| json!({"id":entry["id"],"title":entry["title"],"enabled":field != "macros" || macro_has_enabled_actions(entry)})).collect())
    };
    let (timer_definitions, message_definitions) = crate::remote_definitions::catalog(&workspace)?;
    let mut catalog = json!({"workspacePath":path,"timerDefinitions":timer_definitions,"messageDefinitions":message_definitions,"libraries":crate::remote_cues::library_catalog(&workspace)?,"playlists":crate::remote_cues::playlist_catalog(&workspace)?,"mediaBins":crate::remote_media::catalog(&workspace)?,"tickers":summarize("tickers")?,"audienceLooks":summarize("audienceLooks")?,"macros":summarize("macros")?,"messages":summarize("messages")?,"stageLayouts":summarize("stageLayouts")?,"clearGroups":summarize("clearGroups")?,"recordingPresets":workspace["recordingPresets"].as_array().ok_or("Workspace recordingPresets must be an array.")?.iter().map(|entry|json!({"id":entry["id"],"title":entry["title"],"enabled":entry.get("capture").is_some()})).collect::<Vec<_>>()});
    let mut modern = serde_json::Map::new();
    for field in [
        "audienceLooks",
        "macros",
        "macroCollections",
        "messages",
        "props",
        "propFolders",
        "stageLayouts",
        "clearGroups",
        "mediaLibraries",
        "media",
        "themes",
        "masks",
    ] {
        let entries = workspace[field]
            .as_array()
            .ok_or_else(|| format!("Workspace {field} must be an array."))?;
        modern.insert(field.to_string(), json!(entries));
    }
    // ProPresenter Remote only lists audio inside playlists, so audio without a library gets a virtual one.
    let unfiled_audio = catalog["mediaBins"][0]["items"]
        .as_array()
        .ok_or("Unfiled media bin is unavailable.")?
        .iter()
        .filter(|item| item["type"] == "audio")
        .cloned()
        .collect::<Vec<_>>();
    if !unfiled_audio.is_empty() {
        modern["mediaLibraries"].as_array_mut().ok_or("Workspace mediaLibraries must be an array.")?
            .push(json!({"id":UNFILED_AUDIO_PLAYLIST_ID,"title":"Audio","type":"playlist","mediaType":"audio"}));
        catalog["mediaBins"].as_array_mut().ok_or("Media bins must be an array.")?
            .push(json!({"id":UNFILED_AUDIO_PLAYLIST_ID,"title":"Audio","audioPlaylist":true,"items":unfiled_audio}));
    }
    catalog["modernCatalog"] = Value::Object(modern);
    Ok(catalog)
}

pub(crate) fn validate_workspace_command(
    state: &AppState,
    workspace_io: &mut crate::WorkspaceIoState,
    payload: &Value,
) -> Result<(), String> {
    let path = state
        .active_workspace
        .lock()
        .map_err(|_| "Workspace state is unavailable.")?
        .clone()
        .ok_or("An active workspace is required.")?;
    if payload["workspacePath"].as_str() != path.to_str() {
        return Err("The remote command belongs to a different workspace.".into());
    }
    workspace_io.require_active(&path)?;
    // The revision-keyed cache avoids re-parsing the whole workspace for every remote trigger.
    let workspace = cached_workspace(&path, workspace_io.revision(&path))?;
    validate_target(&workspace, payload)
}

fn macro_has_enabled_actions(entry: &Value) -> bool {
    entry
        .get("actions")
        .and_then(Value::as_array)
        .is_some_and(|actions| {
            actions
                .iter()
                .any(|action| action.get("enabled").and_then(Value::as_bool) != Some(false))
        })
}

pub(crate) fn validate_target(workspace: &Value, payload: &Value) -> Result<(), String> {
    if payload["command"]
        .as_str()
        .is_some_and(crate::remote_definitions::is_definition_command)
    {
        return crate::remote_definitions::validate_persisted(workspace, payload);
    }
    let field = match payload["command"].as_str() {
        Some("audienceLook") => "audienceLooks",
        Some("prop") if matches!(payload["action"].as_str(), Some("show" | "clear")) => "props",
        Some("prop") => return Err("Prop requires show or clear.".into()),
        Some("macro") => "macros",
        Some("message") if payload["action"] == "clear" => return Ok(()),
        Some("message") => "messages",
        Some("stageLayout") => "stageLayouts",
        Some("clearGroup") => "clearGroups",
        _ => return Err("Unsupported workspace remote command.".into()),
    };
    let id = payload
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("A target id is required.")?;
    let entry = workspace
        .get(field)
        .and_then(Value::as_array)
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["id"].as_str() == Some(id))
        })
        .ok_or("Target is unavailable in the active workspace.")?;
    if field == "macros" && !macro_has_enabled_actions(entry) {
        return Err("Macro has no enabled actions.".into());
    }
    Ok(())
}

pub(crate) fn execute(
    app: &Host,
    state: &AppState,
    command: &str,
    payload: &Value,
) -> Result<Value, String> {
    let path = state
        .active_workspace
        .lock()
        .map_err(|_| "Workspace state is unavailable.")?
        .clone()
        .ok_or("An active workspace is required.")?;
    let mut workspace_io = crate::lock_workspace_io(state)?;
    workspace_io.require_active(&path)?;
    let revision = workspace_io.revision(&path);
    let workspace = cached_workspace(&path, revision)?;
    if matches!(
        command,
        "cue" | "libraryCue" | "playlistCue" | "operatorPlaylistCue" | "stageMessage" | "media"
    ) && payload["workspacePath"].as_str() != path.to_str()
    {
        return Ok(
            json!({"ok":false,"status":"invalid-target","changed":false,"error":"The remote command belongs to a different workspace."}),
        );
    }
    execute_resolved(app, state, command, payload, &workspace, None)
}

pub(crate) fn execute_operator_cue_before_save(
    app: &Host,
    state: &AppState,
    command: &str,
    payload: &Value,
    revision: u64,
) -> Result<Value, String> {
    let command = match command {
        "playlistCue" => "operatorPlaylistCue",
        "libraryCue" => "libraryCue",
        _ => return Err("Unsupported operator cue command.".into()),
    };
    let path = crate::active_workspace(state, payload["workspacePath"].clone())?;
    let workspace = WORKSPACE_CATALOG
        .lock()
        .map_err(|_| "Remote catalog cache is unavailable.")?
        .as_ref()
        .filter(|cached| cached.path == path && cached.revision >= revision)
        .map(|cached| cached.document.clone())
        .ok_or("The committed workspace for this cue is unavailable.")?;
    execute_resolved(app, state, command, payload, &workspace, path.to_str())
}

fn execute_resolved(
    app: &Host,
    state: &AppState,
    command: &str,
    payload: &Value,
    workspace: &Value,
    expected_workspace: Option<&str>,
) -> Result<Value, String> {
    let rss = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.")?
        .snapshot();
    let mut output = crate::lock_output_snapshot_state(&state.output_snapshot)?;
    let Some(cached) = output.cache.as_ref().map(|cache| cache.snapshot.clone()) else {
        return Ok(
            json!({"ok": false, "status": "not-ready", "changed": false, "error": "Native output has not accepted a snapshot yet."}),
        );
    };
    if expected_workspace.is_some_and(|path| cached["playbackWorkspacePath"].as_str() != Some(path))
    {
        return Err("The output is switching workspaces.".into());
    }
    let mut registry = state
        .native_timers
        .lock()
        .map_err(|_| "Native timer state is unavailable.")?;
    let mut staged = registry.clone();
    let (mut next, mut patch) = match mutate(&cached, workspace, &mut staged, command, payload) {
        Ok(value) => value,
        Err(error) => {
            return Ok(
                json!({"ok": false, "status": "invalid-target", "changed": false, "error": error}),
            )
        }
    };
    let prop_auto_clear = if command == "prop" && payload["action"] == "show" {
        let prop = workspace["props"]
            .as_array()
            .and_then(|props| props.iter().find(|prop| prop["id"] == payload["id"]))
            .ok_or("Remote prop definition is unavailable.")?;
        Some(crate::native_props::auto_clear_duration(prop)?)
    } else {
        None
    };
    let prop_auto_clear_deadline = prop_auto_clear
        .flatten()
        .map(|duration| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|now| (now + duration).as_millis() as u64)
                .map_err(|_| "System clock is before the Unix epoch.")
        })
        .transpose()?;
    if command == "prop" {
        if let Some(id) = payload["id"].as_str() {
            if !next["remotePropDeadlines"].is_object() {
                next["remotePropDeadlines"] = json!({});
            }
            if let Some(deadlines) = next["remotePropDeadlines"].as_object_mut() {
                deadlines.remove(id);
                if let Some(deadline) = prop_auto_clear_deadline {
                    deadlines.insert(id.into(), json!(deadline));
                }
            }
            patch["remotePropDeadlines"] = next["remotePropDeadlines"].clone();
        }
    }
    let changed = next != cached;
    let (revision, pending) = crate::commit_native_output_snapshot(
        state,
        &mut output,
        next,
        &rss,
        crate::remote_media::transport(&cached, command, payload),
        None,
    )?;
    *registry = staged;
    if let Some(duration) = prop_auto_clear {
        state
            .native_props
            .lock()
            .map_err(|_| "Native prop state is unavailable.")?
            .trigger(
                payload["id"].as_str().ok_or("Prop id is required.")?,
                duration,
            )?;
    }
    drop(registry);
    drop(output);
    crate::schedule_output_rss_refreshes(app, pending);
    if changed || prop_auto_clear.is_some() {
        let _ = app.emit_to(
            "main",
            "native-output-state-patch",
            json!({"source": "remote", "revision": revision, "patch": patch,
                "propAutoClear": prop_auto_clear.map(|_| json!({"id": payload["id"], "deadline": prop_auto_clear_deadline}))}),
        );
    }
    Ok(
        json!({"ok": true, "status": if changed {"applied"} else {"unchanged"}, "changed": changed, "revision": revision, "patch": patch}),
    )
}

pub(crate) fn mutate_stage_message(
    snapshot: &Value,
    payload: &Value,
) -> Result<(Value, Value), String> {
    let mut message = snapshot
        .get("stageMessage")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or("Stage message is unavailable in the accepted output snapshot.")?;
    if !message["text"].is_string()
        || !message["state"]
            .as_str()
            .is_some_and(|state| matches!(state, "visible" | "hidden" | "cleared"))
    {
        return Err("The accepted stage message is invalid.".into());
    }
    match payload["action"].as_str() {
        Some("edit") => {
            message["text"] = json!(payload["text"]
                .as_str()
                .ok_or("Stage message text must be a string.")?)
        }
        Some("show") => message["state"] = json!("visible"),
        Some("hide") => message["state"] = json!("hidden"),
        Some("clear") => message = json!({"text":"","state":"cleared"}),
        _ => return Err("Unsupported stage message action.".into()),
    }
    let mut next = snapshot.clone();
    next["stageMessage"] = message.clone();
    Ok((next, json!({"stageMessage":message})))
}

fn mutate(
    snapshot: &Value,
    workspace: &Value,
    timers: &mut TimerRegistry,
    command: &str,
    payload: &Value,
) -> Result<(Value, Value), String> {
    if command == "videoInputClear" {
        return crate::video_input::clear_snapshot(snapshot);
    }
    if command == "media" {
        return crate::remote_media::mutate(snapshot, workspace, payload);
    }
    if command == "playlistCue" {
        return crate::remote_cues::select_playlist(snapshot, workspace, payload);
    }
    if command == "operatorPlaylistCue" {
        return crate::remote_cues::select_operator_playlist(snapshot, workspace, payload);
    }
    if command == "libraryCue" {
        return crate::remote_cues::select_library(snapshot, workspace, payload);
    }
    if command == "stageMessage" {
        return mutate_stage_message(snapshot, payload);
    }
    if command == "cue" {
        return crate::remote_cues::select(snapshot, payload);
    }
    let id = payload
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("A target id is required.")?;
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .ok_or("An action is required.")?;
    let mut next = snapshot.clone();
    let mut patch = json!({});
    if command == "timer" {
        let timer = workspace
            .get("countdownTimers")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
            })
            .ok_or("Timer id is not available in the active workspace.")?;
        let accepted_timer = snapshot
            .get("countdownTimers")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
            })
            .ok_or("Timer id is not available in the accepted output snapshot.")?;
        crate::validate_current_timer_definition(workspace, accepted_timer)?;
        let increment = if action == "increment" {
            payload
                .get("seconds")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .ok_or("Finite increment seconds are required.")?
        } else {
            0.0
        };
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
            return Err("Unsupported timer action.".into());
        }
        let mut runs = snapshot
            .get("countdownTimerRuns")
            .and_then(Value::as_object)
            .cloned()
            .ok_or("Timer runs must be an object.")?;
        let run = timers.control(timer, action, runs.get(id), increment)?;
        if !run.is_null() {
            runs.insert(id.to_string(), run);
        }
        patch["countdownTimerRuns"] = Value::Object(runs);
    } else if command == "prop" {
        if !matches!(action, "show" | "clear") {
            return Err("Unsupported prop action.".into());
        }
        for source in [workspace, snapshot] {
            if !source
                .get("props")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item.get("id").and_then(Value::as_str) == Some(id))
                })
            {
                return Err(
                    "Prop id is not available in the active workspace and output snapshot.".into(),
                );
            }
        }
        let mut ids = snapshot
            .get("livePropIds")
            .and_then(Value::as_array)
            .cloned()
            .ok_or("Live prop ids must be an array.")?;
        if action == "show" {
            if !ids.iter().any(|value| value.as_str() == Some(id)) {
                ids.push(json!(id));
            }
            let (screens, _) =
                crate::show_output_snapshot_look_layer(snapshot.get("screenConfig"), "props")?;
            patch["screenConfig"] = screens;
        } else {
            ids.retain(|value| value.as_str() != Some(id));
        }
        patch["livePropIds"] = json!(ids);
    } else {
        return Err("Unsupported remote command.".into());
    }
    let object = next
        .as_object_mut()
        .ok_or("Output snapshot must be an object.")?;
    for (key, value) in patch.as_object().expect("patch object") {
        object.insert(key.clone(), value.clone());
    }
    Ok((next, patch))
}
