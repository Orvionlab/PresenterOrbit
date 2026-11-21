//! Status-derived values shared by the ProPresenter Remote and Stage Display handlers.

use serde_json::{json, Value};

pub(super) fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value.get(field).and_then(Value::as_str).unwrap_or_default()
}

pub(super) fn array<'a>(status: &'a Value, field: &str) -> Result<&'a Vec<Value>, String> {
    status
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Status {field} is unavailable."))
}

pub(super) fn workspace_path(status: &Value) -> Result<&str, String> {
    status
        .get("workspacePath")
        .and_then(Value::as_str)
        .ok_or_else(|| "An active workspace is required.".to_string())
}

pub(super) fn library_path(library: &Value, presentation: &Value) -> String {
    format!(
        "{}/{}.pro",
        text(library, "title"),
        text(presentation, "title")
    )
}

/// A presentation addressed by a ProPresenter path, with the command fields that
/// trigger one of its slides.
pub(super) struct Resolved {
    pub path: String,
    pub slides: Vec<Value>,
    pub cue: Value,
}

/// Paths are `<library title>/<presentation title>.pro` or the playlist location
/// `<playlist index>:<entry index>` that ProPresenter documents.
pub(super) fn resolve(status: &Value, path: &str) -> Result<Resolved, String> {
    if let Some((playlist, entry)) = path.split_once(':') {
        let (playlist, entry) = (
            playlist
                .parse::<usize>()
                .map_err(|_| "Invalid playlist location.")?,
            entry
                .parse::<usize>()
                .map_err(|_| "Invalid playlist location.")?,
        );
        let playlist = array(status, "playlists")?
            .get(playlist)
            .ok_or("The playlist is unavailable.")?;
        let entry = playlist["entries"]
            .as_array()
            .and_then(|entries| entries.get(entry))
            .ok_or("The playlist item is unavailable.")?;
        return Ok(Resolved {
            path: path.to_string(),
            slides: entry["slides"].as_array().cloned().unwrap_or_default(),
            cue: json!({
                "command": "playlistCue",
                "playlistId": playlist["id"],
                "playlistEntryId": entry["id"],
                "presentationId": entry["presentationId"],
            }),
        });
    }
    let mut matches = Vec::new();
    for library in array(status, "libraries")? {
        for presentation in library["presentations"].as_array().into_iter().flatten() {
            if library_path(library, presentation) == path {
                matches.push((library, presentation));
            }
        }
    }
    let [(library, presentation)] = matches.as_slice() else {
        return Err(if matches.is_empty() {
            "The presentation path is unavailable.".to_string()
        } else {
            "The presentation path is ambiguous.".to_string()
        });
    };
    let mut cue = json!({
        "command": "libraryCue",
        "libraryId": library["id"],
        "presentationId": presentation["id"],
        "playlistEntryId": null,
    });
    if let Some(arrangement) = presentation["arrangementId"]
        .as_str()
        .filter(|id| !id.is_empty())
    {
        cue["arrangementId"] = json!(arrangement);
    }
    Ok(Resolved {
        path: path.to_string(),
        slides: presentation["slides"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        cue,
    })
}

/// Path of the live presentation, or `None` when nothing is live or the
/// presentation is in neither a playlist nor a library.
pub(super) fn live_path(status: &Value) -> Option<String> {
    let presentation = status
        .get("presentation")
        .filter(|value| value.is_object())?;
    if let Some(entry_id) = presentation["playlistEntryId"].as_str() {
        for (playlist_index, playlist) in status["playlists"].as_array()?.iter().enumerate() {
            let position = playlist["entries"]
                .as_array()?
                .iter()
                .position(|entry| entry["id"].as_str() == Some(entry_id));
            if let Some(entry_index) = position {
                return Some(format!("{playlist_index}:{entry_index}"));
            }
        }
        return None;
    }
    status["libraries"].as_array()?.iter().find_map(|library| {
        library["presentations"]
            .as_array()?
            .iter()
            .find(|candidate| candidate["id"] == presentation["id"])
            .map(|candidate| library_path(library, candidate))
    })
}

pub(super) fn format_clock(milliseconds: i64) -> String {
    let seconds = milliseconds.unsigned_abs() / 1000;
    format!(
        "{}{}:{:02}:{:02}",
        if milliseconds < 0 { "-" } else { "" },
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
