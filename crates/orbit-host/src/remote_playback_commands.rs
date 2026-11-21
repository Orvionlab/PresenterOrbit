use crate::Host;
use crate::{output_playback::PlaybackLayer, AppState};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

fn matching_source(content: &Value, payload: &Value, layer: PlaybackLayer, asset: &Value) -> bool {
    let (matches_asset, selection) = match layer {
        PlaybackLayer::Music => (content["audioId"] == payload["id"], "audioSelectionId"),
        PlaybackLayer::Media => (
            asset["sourcePath"]
                .as_str()
                .is_some_and(|path| !path.is_empty() && content["backgroundMediaPath"] == path),
            "backgroundSelectionId",
        ),
    };
    crate::is_main_transport_content(content)
        && matches_asset
        && content[selection].as_f64() == payload["selectionId"].as_f64()
        && payload["selectionId"].as_f64().is_some()
}

fn require_duration(content: &Value, layer: PlaybackLayer) -> Result<(), String> {
    let key = match layer {
        PlaybackLayer::Music => "musicPlaybackState",
        PlaybackLayer::Media => "backgroundPlaybackState",
    };
    if let Some(error) = content[key]["error"].as_str() {
        return Err(error.into());
    }
    if !content[key]["duration"]
        .as_f64()
        .is_some_and(|value| value.is_finite() && value > 0.0)
    {
        return Err("Native media duration is not available for seeking.".into());
    }
    Ok(())
}

pub(crate) fn execute(app: &Host, state: &AppState, payload: &Value) -> Result<Value, String> {
    let mut mutation_started = false;
    let result = apply(app, state, payload, &mut mutation_started);
    match result {
        Ok(value) => Ok(value),
        Err(error) if mutation_started => Ok(json!({"ok":false,"status":"error","error":error})),
        Err(error) => {
            Ok(json!({"ok":false,"status":"invalid-target","changed":false,"error":error}))
        }
    }
}

fn apply(
    app: &Host,
    state: &AppState,
    payload: &Value,
    mutation_started: &mut bool,
) -> Result<Value, String> {
    let path = crate::active_workspace(state, payload["workspacePath"].clone())?;
    let io = crate::lock_workspace_io(state)?;
    io.require_active(&path)?;
    let workspace = crate::workspace::read_workspace(&path)?;
    let rss = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.")?
        .snapshot();
    let mut coordinator = crate::lock_output_snapshot_state(&state.output_snapshot)?;
    let previous_revision = coordinator.revision;
    let snapshot = coordinator
        .cache
        .as_ref()
        .ok_or("Native output has not accepted a snapshot yet.")?
        .snapshot
        .clone();
    let mut request = payload.clone();
    if payload["action"] == "marker" {
        request["action"] = json!("seek");
        request["position"] = json!(0);
    }
    if payload.get("sourceSeconds").is_some() {
        request["position"] = json!(0);
    }
    let mut command = crate::remote_media::clock_command(&snapshot, &workspace, &request)?;
    let asset = snapshot["media"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == payload["id"]))
        .ok_or("Playback media is unavailable in the accepted snapshot.")?;
    let ids = crate::main_transport_source_ids(state)?;
    let sources: HashMap<String, Value> = {
        let content = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.")?;
        ids.into_iter()
            .filter_map(|id| {
                content
                    .get(&id)
                    .filter(|scene| matching_source(scene, payload, command.layer, asset))
                    .map(|scene| (id, scene.clone()))
            })
            .collect()
    };
    if sources.is_empty() {
        return Err("No native playback target matches this occurrence.".into());
    }
    let duration = source_duration(&crate::media_metadata::probe(&asset["sourcePath"])?)?;
    let mut automation = state
        .native_automation
        .lock()
        .map_err(|_| "Native automation state is unavailable.")?;
    let marker = if payload["action"] == "marker" {
        let id = payload["markerId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Playback marker id is required.")?;
        let path = asset["sourcePath"]
            .as_str()
            .ok_or("Playback marker path is required.")?;
        Some(automation.marker_jump_target(
            path,
            &payload["selectionId"],
            command.layer.as_str(),
            id,
        )?)
    } else {
        None
    };
    let mut marker_iteration = 0;
    let mut absolute_position = None;
    {
        let mut playback = state
            .output_playback
            .lock()
            .map_err(|_| "Output playback state is unavailable.")?;
        let now = Instant::now();
        for (id, content) in &sources {
            playback.set_duration(id, command.layer, duration, now);
            let canonical = playback.canonical_content(id, content, now);
            require_duration(&canonical, command.layer)?;
            if let Some(seconds) = payload.get("sourceSeconds") {
                let seconds = seconds
                    .as_f64()
                    .ok_or("Deck source position must be a finite number.")?;
                let (position, _) = source_position(&canonical, command.layer, seconds, true)?;
                if absolute_position
                    .is_some_and(|previous: f64| (previous - position).abs() > 0.000001)
                {
                    return Err("Native targets have different playable trim ranges.".into());
                }
                absolute_position = Some(position);
                command.position = position;
            }
            if let Some((_, seconds)) = marker {
                let (position, iteration) = marker_position(&canonical, command.layer, seconds)?;
                command.position = position;
                marker_iteration = iteration;
            }
        }
    }
    let targets = sources.keys().cloned().collect::<Vec<_>>();
    *mutation_started = true;
    let mut mutation = crate::apply_playback_command_to_target_sources(state, command, &targets)?;
    let changed = mutation.changed;
    let revision =
        crate::anchor_native_playback_revision(&mut coordinator, mutation.targeted, changed);
    crate::propagate_playback_source_mutation(state, command, changed, &mut mutation.payloads)?;
    crate::mark_native_playback_propagated(&mut coordinator, revision);
    let marker_event = marker.map(|(index, _)| automation.jump_marker(index, marker_iteration));
    drop(automation);
    let (revision, patch, pending) = if payload["action"] == "restart" {
        let (next, patch) =
            crate::set_transport_paused_in_output_snapshot(&snapshot, command.layer, false)?;
        let (revision, pending) = crate::commit_native_output_snapshot(
            state,
            &mut coordinator,
            next,
            &rss,
            Some((command.layer, false)),
            None,
        )?;
        (revision, json!({"transportPlaybackPaused":patch}), pending)
    } else {
        (coordinator.revision, json!({}), Vec::new())
    };
    drop(coordinator);
    if let Some(event) = marker_event {
        app.emit_to("main", "automation-due", event)
            .map_err(|error| error.to_string())?;
    }
    crate::schedule_output_rss_refreshes(app, pending);
    let _ = app.emit_to(
        "main",
        "native-output-state-patch",
        json!({"source":"remote","revision":revision,"patch":patch}),
    );
    let mut response = json!({"ok":true,"status":"applied","changed":revision != previous_revision,"revision":revision,"patch":patch});
    if let Some((_, seconds)) = marker {
        response["markerTimeSeconds"] = json!(seconds);
    }
    Ok(response)
}

fn source_duration(metadata: &Value) -> Result<f64, String> {
    metadata["durationSeconds"]
        .as_f64()
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .ok_or_else(|| "Native media metadata has no measured duration for seeking.".into())
}

fn marker_position(
    content: &Value,
    layer: PlaybackLayer,
    seconds: f64,
) -> Result<(f64, u64), String> {
    source_position(content, layer, seconds, false)
}

fn source_position(
    content: &Value,
    layer: PlaybackLayer,
    seconds: f64,
    allow_end: bool,
) -> Result<(f64, u64), String> {
    let key = if layer == PlaybackLayer::Music {
        "musicPlaybackState"
    } else {
        "backgroundPlaybackState"
    };
    let clock = &content[key];
    let start = if layer == PlaybackLayer::Media {
        content["backgroundTransport"]["inSeconds"]
            .as_f64()
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let end = clock["outSeconds"]
        .as_f64()
        .or_else(|| clock["duration"].as_f64())
        .ok_or("Native media duration is unavailable.")?;
    if !seconds.is_finite()
        || seconds < start
        || seconds > end
        || (!allow_end && seconds == end)
        || end <= start
    {
        return Err("Playback marker is outside the playable media range.".into());
    }
    Ok((
        (seconds - start) / (end - start),
        clock["loopIteration"]
            .as_u64()
            .ok_or("Native media loop iteration is unavailable.")?,
    ))
}
