use serde_json::{json, Value};

use super::super::{data, Backend};
use super::catalog;

pub(super) fn handle(
    backend: &dyn Backend,
    method: &str,
    parts: &[&str],
    _body: Option<&Value>,
) -> Option<Result<Value, String>> {
    Some(match parts {
        ["v1", "presentation", "active", "timeline"] => require_get(method).and_then(|()| {
            let status = backend.full()?;
            active_timeline_status(&status)
        }),
        ["v1", "presentation", "active", "timeline", operation] => {
            require_get(method).and_then(|()| {
                let status = backend.full()?;
                let target = active_target(&status)?;
                perform_operation(backend, &status, &target, operation)
            })
        }
        ["v1", "presentation", "focused", "timeline"] => require_get(method).and_then(|()| {
            let status = backend.full()?;
            if status
                .get("focusedPresentation")
                .is_some_and(Value::is_null)
                || status.get("timelineTransport").is_some_and(Value::is_null)
            {
                return Ok(json!({"current_time":0.0,"is_running":false}));
            }
            timeline_status(&status, &focused_target(&status)?)
        }),
        ["v1", "presentation", "focused", "timeline", operation] => {
            require_get(method).and_then(|()| {
                let status = backend.full()?;
                perform_operation(backend, &status, &focused_target(&status)?, operation)
            })
        }
        ["v1", "playlist", "focused", index, "timeline"] => require_get(method).and_then(|()| {
            let status = backend.full()?;
            let id = status["focusedPlaylistId"]
                .as_str()
                .ok_or("There is no focused playlist.")?;
            let selector = playlist_selector_for_source(&status, id)?;
            timeline_status(&status, &playlist_target(&status, &selector, index)?)
        }),
        ["v1", "playlist", "focused", index, "timeline", operation] => require_get(method)
            .and_then(|()| {
                let status = backend.full()?;
                let id = status["focusedPlaylistId"]
                    .as_str()
                    .ok_or("There is no focused playlist.")?;
                let selector = playlist_selector_for_source(&status, id)?;
                perform_operation(
                    backend,
                    &status,
                    &playlist_target(&status, &selector, index)?,
                    operation,
                )
            }),
        ["v1", "presentation", selector, "timeline", operation] => {
            require_get(method).and_then(|()| {
                let status = backend.full()?;
                let target = presentation_target(&status, selector)?;
                perform_operation(backend, &status, &target, operation)
            })
        }
        ["v1", "playlist", "active", "presentation", index, "timeline"] => require_get(method)
            .and_then(|()| {
                let status = backend.full()?;
                let target = active_playlist_target(&status, index)?;
                timeline_status(&status, &target)
            }),
        ["v1", "playlist", "active", "presentation", index, "timeline", operation] => {
            require_get(method).and_then(|()| {
                let status = backend.full()?;
                let target = active_playlist_target(&status, index)?;
                perform_operation(backend, &status, &target, operation)
            })
        }
        ["v1", "playlist", selector, index, "timeline", operation] => {
            require_get(method).and_then(|()| {
                let status = backend.full()?;
                let target = playlist_target(&status, selector, index)?;
                perform_operation(backend, &status, &target, operation)
            })
        }
        ["v1", "announcement", "active", "timeline"]
        | ["v1", "announcement", "active", "timeline", _]
        | ["v1", "playlist", "active", "announcement", _, "timeline"]
        | ["v1", "playlist", "active", "announcement", _, "timeline", _] => {
            Err("Announcement timelines do not have a native transport clock.".into())
        }
        _ if known_timeline_path(parts) => Err("Timeline routes accept GET requests only.".into()),
        _ => return None,
    })
}

fn known_timeline_path(parts: &[&str]) -> bool {
    matches!(
        parts,
        ["v1", "presentation", "active", "timeline"]
            | ["v1", "presentation", "active", "timeline", _]
            | ["v1", "presentation", "focused", "timeline"]
            | ["v1", "presentation", "focused", "timeline", _]
            | ["v1", "presentation", _, "timeline", _]
            | ["v1", "announcement", "active", "timeline"]
            | ["v1", "announcement", "active", "timeline", _]
            | ["v1", "playlist", "active", "announcement", _, "timeline"]
            | ["v1", "playlist", "active", "announcement", _, "timeline", _]
            | ["v1", "playlist", "active", "presentation", _, "timeline"]
            | ["v1", "playlist", "active", "presentation", _, "timeline", _]
            | ["v1", "playlist", "focused", _, "timeline"]
            | ["v1", "playlist", "focused", _, "timeline", _]
            | ["v1", "playlist", _, _, "timeline", _]
    )
}

fn require_get(method: &str) -> Result<(), String> {
    if method == "GET" {
        Ok(())
    } else {
        Err("Timeline routes accept GET requests only.".into())
    }
}

#[derive(Clone)]
struct Target {
    item_id: String,
    library_id: Option<String>,
    playlist_id: Option<String>,
    playlist_entry_id: Option<String>,
    arrangement_id: Option<String>,
}

fn active_target(status: &Value) -> Result<Target, String> {
    let path = data::live_path(status).ok_or("There is no active presentation.")?;
    target_from_resolved(status, data::resolve(status, &path)?)
}

fn focused_target(status: &Value) -> Result<Target, String> {
    target_from_resolved(status, super::focused_resolved(status)?)
}

fn active_timeline_status(status: &Value) -> Result<Value, String> {
    if status.get("presentation").is_some_and(Value::is_null) {
        return Ok(json!({"current_time":0.0,"is_running":false}));
    }
    let transport = status
        .get("timelineTransport")
        .ok_or("Accepted timeline transport is unavailable.")?;
    if transport.is_null()
        || (transport["itemId"].is_string() && transport["itemId"] != status["presentation"]["id"])
    {
        return Ok(json!({"current_time":0.0,"is_running":false}));
    }
    match status
        .get("presentation")
        .ok_or("Accepted presentation status is unavailable.")?
    {
        Value::Null => Ok(json!({"current_time":0.0,"is_running":false})),
        Value::Object(_) => {
            let target = active_target(status)?;
            timeline_status(status, &target)
        }
        _ => Err("Accepted presentation status is invalid.".into()),
    }
}

fn presentation_target(status: &Value, selector: &str) -> Result<Target, String> {
    target_from_resolved(status, catalog::resolve_presentation(status, selector)?)
}

fn playlist_target(status: &Value, selector: &str, index: &str) -> Result<Target, String> {
    let index = parse_index(index)?;
    target_from_resolved(
        status,
        catalog::resolve_playlist_presentation(status, selector, index)?,
    )
}

fn active_playlist_target(status: &Value, index: &str) -> Result<Target, String> {
    let playlist_id = status
        .get("livePlaylistId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("There is no active playlist presentation.")?;
    let selector = playlist_selector_for_source(status, playlist_id)?;
    playlist_target(status, &selector, index)
}

fn playlist_selector_for_source(status: &Value, source_id: &str) -> Result<String, String> {
    let playlists = data::array(status, "playlists")?;
    let mut sibling_indexes = std::collections::HashMap::<String, usize>::new();
    let mut found = None;
    for playlist in playlists {
        let id = playlist
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("Playlist identity is unavailable.")?;
        let title = playlist
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .ok_or("Playlist title is unavailable.")?;
        let kind = playlist
            .get("modern")
            .and_then(|modern| modern.get("type"))
            .and_then(Value::as_str)
            .or_else(|| playlist.get("type").and_then(Value::as_str))
            .unwrap_or("playlist");
        let parent = match playlist
            .get("modern")
            .and_then(|modern| modern.get("parentId"))
        {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(parent)) => parent.clone(),
            Some(_) => return Err("Playlist parent identity is invalid.".into()),
        };
        let index = sibling_indexes.entry(parent).or_default();
        if id == source_id {
            if kind != "playlist" {
                return Err("The active playlist identity does not refer to a playlist.".into());
            }
            if found.is_some() {
                return Err("The active playlist identity is ambiguous.".into());
            }
            found = Some(
                catalog::identity(id, title, *index)["uuid"]
                    .as_str()
                    .ok_or("Playlist UUID is unavailable.")?
                    .to_string(),
            );
        }
        *index += 1;
    }
    found.ok_or_else(|| "The active playlist is unavailable in the workspace catalog.".into())
}

fn target_from_resolved(status: &Value, resolved: data::Resolved) -> Result<Target, String> {
    let item_id = required_string(
        &resolved.cue,
        "presentationId",
        "Presentation identity is unavailable.",
    )?
    .to_string();
    let (library_id, playlist_id, playlist_entry_id, arrangement_id) =
        match resolved.cue["command"].as_str() {
            Some("libraryCue") => (
                Some(
                    required_string(
                        &resolved.cue,
                        "libraryId",
                        "Library identity is unavailable.",
                    )?
                    .to_string(),
                ),
                None,
                None,
                optional_string(&resolved.cue, "arrangementId")?.map(str::to_string),
            ),
            Some("playlistCue") => (
                None,
                Some(
                    required_string(
                        &resolved.cue,
                        "playlistId",
                        "Playlist identity is unavailable.",
                    )?
                    .to_string(),
                ),
                Some(
                    required_string(
                        &resolved.cue,
                        "playlistEntryId",
                        "Playlist occurrence identity is unavailable.",
                    )?
                    .to_string(),
                ),
                None,
            ),
            _ => return Err("The presentation source is unsupported for timeline control.".into()),
        };
    let mut arrangement_id = arrangement_id;
    if let (Some(playlist_id), Some(playlist_entry_id)) =
        (playlist_id.as_deref(), playlist_entry_id.as_deref())
    {
        arrangement_id = playlist_arrangement_id(status, playlist_id, playlist_entry_id)?;
    }
    let timeline = presentation_timeline(status, &item_id, library_id.as_deref())?;
    crate::native_timeline::validate(&timeline, &Value::Array(resolved.slides))?;
    Ok(Target {
        item_id,
        library_id,
        playlist_id,
        playlist_entry_id,
        arrangement_id,
    })
}

fn playlist_arrangement_id(
    status: &Value,
    playlist_id: &str,
    entry_id: &str,
) -> Result<Option<String>, String> {
    let playlists = data::array(status, "playlists")?;
    let mut playlists = playlists
        .iter()
        .filter(|playlist| playlist.get("id").and_then(Value::as_str) == Some(playlist_id));
    let playlist = playlists.next().ok_or("Playlist is unavailable.")?;
    if playlists.next().is_some() {
        return Err("Playlist identity is ambiguous.".into());
    }
    let entries = data::array(playlist, "entries")?;
    let mut entries = entries
        .iter()
        .filter(|entry| entry.get("id").and_then(Value::as_str) == Some(entry_id));
    let entry = entries
        .next()
        .ok_or("Playlist presentation occurrence is unavailable.")?;
    if entries.next().is_some() {
        return Err("Playlist occurrence identity is ambiguous.".into());
    }
    optional_string(entry, "arrangementId").map(|id| id.map(str::to_string))
}

fn presentation_timeline(
    status: &Value,
    item_id: &str,
    library_id: Option<&str>,
) -> Result<Value, String> {
    let mut matches = Vec::new();
    for library in data::array(status, "libraries")? {
        if library_id.is_some_and(|id| library.get("id").and_then(Value::as_str) != Some(id)) {
            continue;
        }
        for presentation in data::array(library, "presentations")? {
            if presentation.get("id").and_then(Value::as_str) == Some(item_id) {
                let modern = presentation
                    .get("modern")
                    .filter(|modern| modern.is_object())
                    .ok_or("Presentation timeline details are unavailable.")?;
                let timeline = modern
                    .get("timeline")
                    .filter(|timeline| timeline.is_object())
                    .ok_or("The presentation has no timeline.")?;
                matches.push(timeline.clone());
            }
        }
    }
    let Some(timeline) = matches.first() else {
        return Err("Presentation is unavailable in the workspace catalog.".into());
    };
    if matches.iter().all(|candidate| candidate == timeline) {
        Ok(timeline.clone())
    } else {
        Err("Presentation timeline data is inconsistent across library references.".into())
    }
}

fn timeline_status(status: &Value, target: &Target) -> Result<Value, String> {
    if !target_matches_live(status, target)? {
        return Ok(json!({"current_time":0.0,"is_running":false}));
    }
    let transport = match status
        .get("timelineTransport")
        .ok_or("Accepted timeline transport is unavailable.")?
    {
        Value::Null => return Ok(json!({"current_time":0.0,"is_running":false})),
        transport @ Value::Object(_) => transport,
        _ => return Err("Accepted timeline transport is invalid.".into()),
    };
    if transport.get("itemId").and_then(Value::as_str) != Some(target.item_id.as_str()) {
        return Ok(json!({"current_time":0.0,"is_running":false}));
    }
    let mode = required_string(transport, "mode", "Accepted timeline mode is unavailable.")?;
    if mode == "stopped" {
        if !transport.get("startedAtMs").is_some_and(Value::is_null) {
            return Err("Stopped timeline transport must not have a running anchor.".into());
        }
        let position = finite_position(transport.get("positionSeconds"))?;
        return Ok(json!({"current_time":position,"is_running":false}));
    }
    if !matches!(mode, "playback" | "recording") {
        return Err("Accepted timeline mode is unsupported.".into());
    }
    let generation = required_string(
        transport,
        "generation",
        "Accepted timeline generation is unavailable.",
    )?;
    let head = status
        .get("timelinePlayhead")
        .filter(|head| head.is_object())
        .ok_or("Native timeline playhead is unavailable.")?;
    if required_string(
        head,
        "generation",
        "Native timeline generation is unavailable.",
    )? != generation
    {
        return Err("Native timeline playhead belongs to a different generation.".into());
    }
    let position = finite_position(head.get("positionSeconds"))?;
    let running = head
        .get("running")
        .and_then(Value::as_bool)
        .ok_or("Native timeline running state is unavailable.")?;
    Ok(json!({"current_time":position,"is_running":running}))
}

fn target_matches_live(status: &Value, target: &Target) -> Result<bool, String> {
    let presentation = status
        .get("presentation")
        .ok_or("Accepted presentation status is unavailable.")?;
    let live_item_id = match presentation {
        Value::Null => None,
        Value::Object(presentation) => Some(
            presentation
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or("Accepted presentation identity is unavailable.")?,
        ),
        _ => return Err("Accepted presentation status is invalid.".into()),
    };
    let live_playlist_id = nullable_status_string(status, "livePlaylistId")?;
    let live_entry_id = match presentation {
        Value::Null => None,
        presentation @ Value::Object(_) => nullable_object_string(presentation, "playlistEntryId")?,
        _ => None,
    };
    Ok(live_item_id == Some(target.item_id.as_str())
        && live_playlist_id == target.playlist_id.as_deref()
        && live_entry_id == target.playlist_entry_id.as_deref())
}

fn perform_operation(
    backend: &dyn Backend,
    status: &Value,
    target: &Target,
    operation: &str,
) -> Result<Value, String> {
    if !matches!(operation, "play" | "pause" | "rewind") {
        return Err("Timeline operation must be play, pause, or rewind.".into());
    }
    let command = json!({
        "command":"timelineTransport",
        "workspacePath":data::workspace_path(status)?,
        "action":operation,
        "presentationId":target.item_id,
        "libraryId":target.library_id,
        "playlistId":target.playlist_id,
        "playlistEntryId":target.playlist_entry_id,
        "arrangementId":target.arrangement_id,
        "expectedItemId":live_item_id(status)?,
        "expectedPlaylistId":nullable_status_string(status, "livePlaylistId")?,
        "expectedPlaylistEntryId":live_entry_id(status)?,
        "expectedGeneration":transport_generation(status)?,
    });
    if command["libraryId"].is_null() && command["playlistId"].is_null() {
        return Err("Timeline target context is unavailable.".into());
    }
    backend.command(&command)?;
    Ok(Value::Null)
}

fn live_item_id(status: &Value) -> Result<Option<&str>, String> {
    match status
        .get("presentation")
        .ok_or("Accepted presentation status is unavailable.")?
    {
        Value::Null => Ok(None),
        presentation @ Value::Object(_) => nullable_object_string(presentation, "id"),
        _ => Err("Accepted presentation status is invalid.".into()),
    }
}

fn live_entry_id(status: &Value) -> Result<Option<&str>, String> {
    match status
        .get("presentation")
        .ok_or("Accepted presentation status is unavailable.")?
    {
        Value::Null => Ok(None),
        presentation @ Value::Object(_) => nullable_object_string(presentation, "playlistEntryId"),
        _ => Err("Accepted presentation status is invalid.".into()),
    }
}

fn transport_generation(status: &Value) -> Result<Option<&str>, String> {
    match status
        .get("timelineTransport")
        .ok_or("Accepted timeline transport is unavailable.")?
    {
        Value::Null => Ok(None),
        Value::Object(transport) => Ok(Some(
            transport
                .get("generation")
                .and_then(Value::as_str)
                .filter(|generation| !generation.is_empty())
                .ok_or("Accepted timeline generation is unavailable.")?,
        )),
        _ => Err("Accepted timeline transport is invalid.".into()),
    }
}

fn nullable_status_string<'a>(status: &'a Value, field: &str) -> Result<Option<&'a str>, String> {
    match status
        .get(field)
        .ok_or_else(|| format!("Accepted status field {field} is unavailable."))?
    {
        Value::Null => Ok(None),
        Value::String(value) if !value.is_empty() => Ok(Some(value)),
        _ => Err(format!("Accepted status field {field} is invalid.")),
    }
}

fn nullable_object_string<'a>(object: &'a Value, field: &str) -> Result<Option<&'a str>, String> {
    match object
        .get(field)
        .ok_or_else(|| format!("Accepted status field {field} is unavailable."))?
    {
        Value::Null => Ok(None),
        Value::String(value) if !value.is_empty() => Ok(Some(value)),
        _ => Err(format!("Accepted status field {field} is invalid.")),
    }
}

fn required_string<'a>(object: &'a Value, field: &str, message: &str) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| message.to_string())
}

fn optional_string<'a>(object: &'a Value, field: &str) -> Result<Option<&'a str>, String> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        Some(_) => Err(format!("Presentation field {field} is invalid.")),
    }
}

fn finite_position(value: Option<&Value>) -> Result<f64, String> {
    value
        .and_then(Value::as_f64)
        .filter(|position| position.is_finite() && *position >= 0.0 && *position <= 86400.0)
        .ok_or_else(|| "Timeline playhead position is invalid.".into())
}

fn parse_index(value: &str) -> Result<usize, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("A zero-based playlist item index is required.".into());
    }
    value
        .parse()
        .map_err(|_| "Playlist item index is outside the supported range.".into())
}
