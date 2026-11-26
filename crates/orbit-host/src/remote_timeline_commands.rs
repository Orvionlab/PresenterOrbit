use crate::Host;
use serde_json::{json, Value};

use crate::AppState;

const COMMAND_FIELDS: [&str; 12] = [
    "command",
    "workspacePath",
    "action",
    "presentationId",
    "libraryId",
    "playlistId",
    "playlistEntryId",
    "arrangementId",
    "expectedItemId",
    "expectedPlaylistId",
    "expectedPlaylistEntryId",
    "expectedGeneration",
];

pub(crate) fn validate_payload(payload: &Value) -> Result<(), String> {
    let fields = payload
        .as_object()
        .ok_or("Timeline transport command must be an object.")?;
    if fields
        .keys()
        .any(|key| !COMMAND_FIELDS.contains(&key.as_str()))
    {
        return Err("Timeline transport command contains unsupported fields.".into());
    }
    if payload.get("command").and_then(Value::as_str) != Some("timelineTransport") {
        return Err("Timeline transport command identity is invalid.".into());
    }
    if !matches!(
        payload.get("action").and_then(Value::as_str),
        Some("play" | "pause" | "rewind")
    ) {
        return Err("Timeline transport action must be play, pause, or rewind.".into());
    }
    required_string(payload, "workspacePath", "Workspace path is required.")?;
    required_string(
        payload,
        "presentationId",
        "Presentation identity is required.",
    )?;
    nullable_string(payload, "expectedItemId")?;
    nullable_string(payload, "expectedPlaylistId")?;
    nullable_string(payload, "expectedPlaylistEntryId")?;
    nullable_string(payload, "expectedGeneration")?;

    let library_id = nullable_string(payload, "libraryId")?;
    let playlist_id = nullable_string(payload, "playlistId")?;
    let playlist_entry_id = nullable_string(payload, "playlistEntryId")?;
    nullable_string(payload, "arrangementId")?;
    match (library_id, playlist_id, playlist_entry_id) {
        (Some(_), None, None) | (None, Some(_), Some(_)) => Ok(()),
        _ => Err("Timeline target must identify either a library presentation or one playlist occurrence.".into()),
    }
}

fn required_string<'a>(payload: &'a Value, field: &str, message: &str) -> Result<&'a str, String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| message.to_string())
}

fn nullable_string<'a>(payload: &'a Value, field: &str) -> Result<Option<&'a str>, String> {
    match payload
        .get(field)
        .ok_or_else(|| format!("Timeline field {field} is required."))?
    {
        Value::Null => Ok(None),
        Value::String(value) if !value.trim().is_empty() => Ok(Some(value)),
        _ => Err(format!(
            "Timeline field {field} must be null or a nonempty string."
        )),
    }
}

fn snapshot_nullable_id<'a>(snapshot: &'a Value, field: &str) -> Result<Option<&'a str>, String> {
    match snapshot
        .get(field)
        .ok_or_else(|| format!("Accepted output field {field} is unavailable."))?
    {
        Value::Null => Ok(None),
        Value::String(value) if !value.is_empty() => Ok(Some(value)),
        _ => Err(format!("Accepted output field {field} is invalid.")),
    }
}

fn accepted_transport(snapshot: &Value) -> Result<Option<&Value>, String> {
    match snapshot
        .get("timelineTransport")
        .ok_or("Accepted timeline transport is unavailable.")?
    {
        Value::Null => Ok(None),
        Value::Object(transport) => {
            for field in ["mode", "itemId", "generation"] {
                if !transport
                    .get(field)
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty())
                {
                    return Err("Accepted timeline transport identity is invalid.".into());
                }
            }
            Ok(snapshot.get("timelineTransport"))
        }
        _ => Err("Accepted timeline transport is invalid.".into()),
    }
}

fn validate_expected_state<'a>(
    snapshot: &'a Value,
    payload: &Value,
    playhead: Option<&Value>,
) -> Result<Option<&'a Value>, String> {
    let current_item_id = match snapshot.get("liveItem") {
        Some(Value::Null) => None,
        Some(Value::Object(item)) => Some(
            item.get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or("Accepted live presentation identity is unavailable.")?,
        ),
        _ => return Err("Accepted live presentation is invalid.".into()),
    };
    if current_item_id != nullable_string(payload, "expectedItemId")? {
        return Err(
            "The live presentation changed before the timeline command was applied.".into(),
        );
    }
    if snapshot_nullable_id(snapshot, "livePlaylistId")?
        != nullable_string(payload, "expectedPlaylistId")?
        || snapshot_nullable_id(snapshot, "livePlaylistEntryId")?
            != nullable_string(payload, "expectedPlaylistEntryId")?
    {
        return Err(
            "The live playlist occurrence changed before the timeline command was applied.".into(),
        );
    }

    let transport = accepted_transport(snapshot)?;
    let current_generation = transport
        .map(|transport| {
            transport["generation"]
                .as_str()
                .filter(|generation| !generation.is_empty())
                .ok_or("Accepted timeline generation is unavailable.")
        })
        .transpose()?;
    if current_generation != nullable_string(payload, "expectedGeneration")? {
        return Err("The timeline generation changed before the command was applied.".into());
    }
    match (playhead, current_generation) {
        (Some(head), Some(generation)) => {
            let head_generation = head
                .get("generation")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or("Native timeline playhead identity is unavailable.")?;
            if head_generation != generation {
                return Err(
                    "The native timeline generation changed before the command was applied.".into(),
                );
            }
        }
        (Some(_), None) => {
            return Err("A native timeline playhead exists without an accepted transport.".into())
        }
        _ => {}
    }
    Ok(transport)
}

pub(crate) fn mutate(
    snapshot: &Value,
    workspace: &Value,
    payload: &Value,
    playhead: Option<&Value>,
    now_ms: i64,
) -> Result<(Value, Value), String> {
    validate_payload(payload)?;
    let transport = validate_expected_state(snapshot, payload, playhead)?;
    let action = required_string(payload, "action", "Timeline action is required.")?;
    let target_id = required_string(
        payload,
        "presentationId",
        "Presentation identity is required.",
    )?;
    let item = unique_item(workspace, target_id)?;
    let timeline = item
        .get("timeline")
        .filter(|timeline| timeline.is_object())
        .ok_or("The target presentation has no valid timeline.")?;
    let item_id = required_string(item, "id", "Presentation identity is required.")?;
    let duration = timeline
        .get("durationSeconds")
        .and_then(Value::as_f64)
        .filter(|duration| duration.is_finite() && *duration > 0.0 && *duration <= 86400.0)
        .ok_or("Timeline duration is invalid.")?;

    let library_id = nullable_string(payload, "libraryId")?;
    let playlist_id = nullable_string(payload, "playlistId")?;
    let playlist_entry_id = nullable_string(payload, "playlistEntryId")?;
    let arrangement_id = nullable_string(payload, "arrangementId")?;
    validate_target_context(
        workspace,
        item_id,
        library_id,
        playlist_id,
        playlist_entry_id,
        arrangement_id,
    )?;

    let current_transport = transport;
    let same_target = current_transport.is_some_and(|transport| {
        transport.get("itemId").and_then(Value::as_str) == Some(item_id)
            && snapshot_nullable_id(snapshot, "livePlaylistId")
                .ok()
                .flatten()
                == playlist_id
            && snapshot_nullable_id(snapshot, "livePlaylistEntryId")
                .ok()
                .flatten()
                == playlist_entry_id
            && snapshot
                .get("liveItem")
                .and_then(|item| item.get("id"))
                .and_then(Value::as_str)
                == Some(item_id)
            && snapshot
                .get("liveItem")
                .and_then(|item| item.get("selectedArrangementId"))
                .and_then(Value::as_str)
                == arrangement_id
    });
    if same_target
        && snapshot
            .get("liveItem")
            .and_then(|live_item| live_item.get("timeline"))
            != Some(timeline)
    {
        return Err(
            "The accepted presentation timeline changed. Refresh the output before controlling it."
                .into(),
        );
    }
    if current_transport.is_some_and(|value| value["mode"] == "recording") {
        return Err("Timeline playback cannot override an active recording.".into());
    }

    let current_head = if same_target {
        current_transport
            .filter(|transport| transport["mode"] == "playback")
            .map(|transport| {
                let head = playhead.ok_or("The native timeline playhead is unavailable.")?;
                let generation = transport["generation"]
                    .as_str()
                    .ok_or("Accepted timeline generation is unavailable.")?;
                if head["generation"].as_str() != Some(generation) {
                    return Err(
                        "The native timeline generation changed before the command was applied."
                            .into(),
                    );
                }
                let position = head["positionSeconds"]
                    .as_f64()
                    .filter(|position| position.is_finite() && *position >= 0.0)
                    .ok_or("Native timeline position is invalid.")?;
                let running = head["running"]
                    .as_bool()
                    .ok_or("Native timeline running state is unavailable.")?;
                let playback_rate = head["playbackRate"]
                    .as_f64()
                    .filter(|rate| {
                        rate.is_finite()
                            && *rate != 0.0
                            && rate.abs() <= crate::native_timeline::MAX_PLAYBACK_RATE
                    })
                    .ok_or("Native timeline playback rate is invalid.")?;
                Ok::<_, String>((
                    generation.to_string(),
                    position.min(duration),
                    running,
                    playback_rate,
                ))
            })
            .transpose()?
    } else {
        None
    };

    let (generation, position, playback_rate, started_at_ms, select_slide) = match action {
        "pause" => {
            let Some(_) = current_transport
                .filter(|transport| same_target && transport["mode"] == "playback")
            else {
                return Err("The target timeline is not active.".into());
            };
            let (generation, position, _, playback_rate) = current_head
                .as_ref()
                .ok_or("The native timeline playhead is unavailable.")?;
            (generation.clone(), *position, *playback_rate, None, false)
        }
        "play" => {
            if let Some((generation, position, running, playback_rate)) = &current_head {
                if (*running || transport.is_some_and(|value| value["startedAtMs"].is_i64()))
                    && *position < duration
                {
                    return Ok((snapshot.clone(), json!({})));
                }
                if *position < duration {
                    (
                        generation.clone(),
                        *position,
                        *playback_rate,
                        Some(now_ms),
                        false,
                    )
                } else {
                    (new_generation(), 0.0, 1.0, Some(now_ms), true)
                }
            } else {
                (new_generation(), 0.0, 1.0, Some(now_ms), true)
            }
        }
        "rewind" => {
            let playing = same_target
                && current_transport.is_some_and(|value| {
                    value["mode"] == "playback" && value["startedAtMs"].is_i64()
                });
            (new_generation(), 0.0, 1.0, playing.then_some(now_ms), true)
        }
        _ => unreachable!("validate_payload accepts only supported actions"),
    };

    let (mut next, mut patch) = if select_slide {
        select_timeline_start(
            snapshot,
            workspace,
            item,
            timeline,
            item_id,
            library_id,
            playlist_id,
            playlist_entry_id,
            arrangement_id,
            position,
        )?
    } else {
        (snapshot.clone(), json!({}))
    };
    let timeline_transport = json!({
        "mode": "playback",
        "itemId": item_id,
        "generation": generation,
        "positionSeconds": position,
        "playbackRate": playback_rate,
        "startedAtMs": started_at_ms,
    });
    next["timelineTransport"] = timeline_transport.clone();
    patch["timelineTransport"] = timeline_transport;
    Ok((next, patch))
}

fn unique_item<'a>(workspace: &'a Value, item_id: &str) -> Result<&'a Value, String> {
    let items = workspace
        .get("library")
        .and_then(Value::as_array)
        .ok_or("Workspace library is unavailable.")?;
    let mut matches = items
        .iter()
        .filter(|item| item.get("id").and_then(Value::as_str) == Some(item_id));
    let item = matches
        .next()
        .ok_or("Presentation is unavailable in the active workspace.")?;
    if matches.next().is_some() {
        return Err("Presentation identity is ambiguous in the active workspace.".into());
    }
    Ok(item)
}

fn validate_target_context(
    workspace: &Value,
    item_id: &str,
    library_id: Option<&str>,
    playlist_id: Option<&str>,
    playlist_entry_id: Option<&str>,
    arrangement_id: Option<&str>,
) -> Result<(), String> {
    if let Some(library_id) = library_id {
        let libraries = workspace
            .get("libraries")
            .and_then(Value::as_array)
            .ok_or("Workspace libraries are unavailable.")?;
        let mut matches = libraries.iter().filter(|library| {
            library.get("id").and_then(Value::as_str) == Some(library_id)
                && library
                    .get("itemIds")
                    .and_then(Value::as_array)
                    .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(item_id)))
        });
        if matches.next().is_none() {
            return Err("Presentation is unavailable in the selected library.".into());
        }
        if matches.next().is_some() {
            return Err("Library identity is ambiguous in the active workspace.".into());
        }
    } else {
        let playlist_id = playlist_id.ok_or("Playlist identity is required.")?;
        let entry_id = playlist_entry_id.ok_or("Playlist occurrence identity is required.")?;
        let playlists = workspace
            .get("playlist")
            .and_then(Value::as_array)
            .ok_or("Workspace playlists are unavailable.")?;
        let mut playlists = playlists
            .iter()
            .filter(|playlist| playlist.get("id").and_then(Value::as_str) == Some(playlist_id));
        let playlist = playlists
            .next()
            .ok_or("Playlist is unavailable in the active workspace.")?;
        if playlists.next().is_some() {
            return Err("Playlist identity is ambiguous in the active workspace.".into());
        }
        let entries = playlist
            .get("items")
            .and_then(Value::as_array)
            .ok_or("Playlist items are unavailable.")?;
        let mut entries = entries.iter().filter(|entry| {
            entry.get("id").and_then(Value::as_str) == Some(entry_id)
                && entry.get("type").and_then(Value::as_str) == Some("presentation")
                && entry.get("itemId").and_then(Value::as_str) == Some(item_id)
        });
        let entry = entries
            .next()
            .ok_or("Playlist presentation occurrence is unavailable.")?;
        if entries.next().is_some() {
            return Err("Playlist occurrence identity is ambiguous.".into());
        }
        let item = unique_item(workspace, item_id)?;
        let entry_arrangement = entry
            .get("arrangementId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .or_else(|| item.get("selectedArrangementId").and_then(Value::as_str));
        if entry_arrangement != arrangement_id {
            return Err(
                "The playlist arrangement changed before the timeline command was applied.".into(),
            );
        }
    }
    if let Some(arrangement_id) = arrangement_id {
        let item = unique_item(workspace, item_id)?;
        let arrangements = item
            .get("arrangements")
            .and_then(Value::as_array)
            .ok_or("Presentation arrangements are unavailable.")?;
        if !arrangements.iter().any(|arrangement| {
            arrangement.get("id").and_then(Value::as_str) == Some(arrangement_id)
        }) {
            return Err("Presentation arrangement is unavailable.".into());
        }
    }
    Ok(())
}

fn select_timeline_start(
    snapshot: &Value,
    workspace: &Value,
    item: &Value,
    timeline: &Value,
    item_id: &str,
    library_id: Option<&str>,
    playlist_id: Option<&str>,
    playlist_entry_id: Option<&str>,
    arrangement_id: Option<&str>,
    position: f64,
) -> Result<(Value, Value), String> {
    let mut selected_item = item.clone();
    if let Some(arrangement_id) = arrangement_id {
        selected_item["selectedArrangementId"] = json!(arrangement_id);
    }
    let slides = crate::output_arrangement::resolve_live_arrangement_slides(
        &selected_item,
        None,
        playlist_entry_id,
    )
    .ok_or("The target presentation arrangement is unavailable.")?;
    crate::native_timeline::validate(timeline, &Value::Array(slides.clone()))?;
    crate::native_timeline::validate_occurrences(timeline, &selected_item)?;
    let first = slides
        .iter()
        .find(|slide| slide.get("disabled").and_then(Value::as_bool) != Some(true))
        .ok_or("Timeline playback requires an enabled presentation cue.")?;
    let selected_cue = timeline["cues"]
        .as_array()
        .ok_or("Timeline cues are unavailable.")?
        .iter()
        .enumerate()
        .filter_map(|(index, cue)| {
            let time = cue.get("timeSeconds")?.as_f64()?;
            (time <= position && cue.get("cueId").and_then(Value::as_str).is_some())
                .then_some((index, time, cue))
        })
        .fold(None, |selected, candidate| match selected {
            Some((_, previous_time, _)) if previous_time >= candidate.1 => selected,
            _ => Some(candidate),
        });
    let slide = selected_cue
        .and_then(|(_, _, cue)| {
            let cue_id = cue.get("cueId")?.as_str()?;
            let occurrence = cue.get("cueInstanceId").and_then(Value::as_str);
            slides.iter().find(|slide| {
                slide.get("disabled").and_then(Value::as_bool) != Some(true)
                    && (slide.get("id").and_then(Value::as_str) == Some(cue_id)
                        || slide.get("sourceCueId").and_then(Value::as_str) == Some(cue_id))
                    && occurrence.is_none_or(|occurrence| {
                        slide.get("arrangementInstanceId").and_then(Value::as_str)
                            == Some(occurrence)
                    })
            })
        })
        .unwrap_or(first);
    let selection = json!({
        "presentationId": item_id,
        "slideId": slide["id"],
        "cueInstanceId": slide.get("arrangementInstanceId").cloned().unwrap_or(Value::Null),
        "playlistEntryId": playlist_entry_id,
    });
    if let Some(library_id) = library_id {
        let mut cue = selection;
        cue["libraryId"] = json!(library_id);
        if let Some(arrangement_id) = arrangement_id {
            cue["arrangementId"] = json!(arrangement_id);
        }
        crate::remote_cues::select_library(snapshot, workspace, &cue, false, false, None)
    } else {
        let mut cue = selection;
        cue["playlistId"] = json!(playlist_id.ok_or("Playlist identity is required.")?);
        crate::remote_cues::select_playlist(snapshot, workspace, &cue)
    }
}

fn new_generation() -> String {
    uuid::Uuid::new_v4().hyphenated().to_string()
}

pub(crate) fn execute(app: &Host, state: &AppState, payload: &Value) -> Result<Value, String> {
    validate_payload(payload)?;
    let path = state
        .active_workspace
        .lock()
        .map_err(|_| "Workspace state is unavailable.")?
        .clone()
        .ok_or("An active workspace is required.")?;
    let _workspace_io = crate::lock_workspace_io(state)?;
    _workspace_io.require_active(&path)?;
    if payload["workspacePath"].as_str() != path.to_str() {
        return Ok(
            json!({"ok":false,"status":"invalid-target","changed":false,"error":"The remote command belongs to a different workspace."}),
        );
    }
    let workspace = crate::workspace::read_workspace(&path)?;
    let rss = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.")?
        .snapshot();
    let mut output = crate::lock_output_snapshot_state(&state.output_snapshot)?;
    let Some(cached) = output.cache.as_ref().map(|cache| cache.snapshot.clone()) else {
        return Ok(
            json!({"ok":false,"status":"not-ready","changed":false,"error":"Native output has not accepted a snapshot yet."}),
        );
    };
    let mut automation = state
        .native_automation
        .lock()
        .map_err(|_| "Native automation state is unavailable.")?;
    let playhead = automation.current_timeline_playhead();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let (next, patch) = match mutate(&cached, &workspace, payload, playhead.as_ref(), now_ms) {
        Ok(result) => result,
        Err(error) => {
            return Ok(json!({"ok":false,"status":"invalid-target","changed":false,"error":error}));
        }
    };
    let changed = next != cached;
    let (revision, pending) = crate::commit_native_output_snapshot(
        state,
        &mut output,
        next,
        &rss,
        None,
        Some(&mut automation),
    )?;
    drop(automation);
    drop(output);
    crate::schedule_output_rss_refreshes(app, pending);
    if changed {
        let _ = app.emit_to(
            "main",
            "native-output-state-patch",
            json!({"source":"remote","revision":revision,"patch":patch}),
        );
    }
    Ok(json!({
        "ok":true,
        "status":if changed {"applied"} else {"unchanged"},
        "changed":changed,
        "revision":revision,
        "patch":patch,
    }))
}
