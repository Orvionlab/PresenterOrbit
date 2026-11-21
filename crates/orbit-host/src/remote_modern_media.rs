//! ProPresenter Remote media catalogs and transport backed by Orbit status.

use std::collections::HashSet;

use serde_json::{json, Value};

use super::super::{data, Backend};
use super::catalog::{identity, matches_id};
use super::media_metadata;

#[derive(Clone, Copy, PartialEq, Eq)]
enum MediaKind {
    Audio,
    Graphics,
}

impl MediaKind {
    fn media_type(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Graphics => "graphics",
        }
    }

    fn accepts_item(self, item_type: &str) -> bool {
        match self {
            Self::Audio => item_type == "audio",
            Self::Graphics => matches!(item_type, "image" | "video"),
        }
    }
}

struct Playlist<'a> {
    library: &'a Value,
    api_index: usize,
}

pub(super) fn handle(
    backend: &dyn Backend,
    method: &str,
    parts: &[&str],
    _body: Option<&Value>,
) -> Option<Result<Value, String>> {
    Some(match (method, parts) {
        ("GET", ["v1", "audio", "playlists"]) => audio_playlists(backend),
        ("GET", ["v1", "media", "playlists"]) => media_playlists(backend),
        ("GET", ["v1", "audio", "playlist", "active" | "focused"]) => {
            active_playlist(backend, MediaKind::Audio)
        }
        ("GET", ["v1", "media", "playlist", "active" | "focused"]) => {
            active_playlist(backend, MediaKind::Graphics)
        }
        ("GET", ["v1", "audio", "playlist", selector])
            if *selector != "active" && *selector != "focused" =>
        {
            playlist_detail(backend, MediaKind::Audio, selector)
        }
        ("GET", ["v1", "media", "playlist", selector])
            if *selector != "active" && *selector != "focused" =>
        {
            playlist_detail(backend, MediaKind::Graphics, selector)
        }
        ("GET", ["v1", "transport", "audio", "current"]) => transport_current(backend),
        ("GET", ["v1", "transport", "audio", "play"]) => set_audio_transport(backend, "play"),
        ("GET", ["v1", "transport", "audio", "pause"]) => set_audio_transport(backend, "pause"),
        ("GET", ["v1", "audio", "playlist", selector, "trigger"]) => {
            trigger_playlist(backend, MediaKind::Audio, selector)
        }
        ("GET", ["v1", "media", "playlist", selector, "trigger"]) => {
            trigger_playlist(backend, MediaKind::Graphics, selector)
        }
        ("GET", ["v1", "audio", "playlist", selector, direction, "trigger"])
            if matches!(*direction, "next" | "previous") =>
        {
            navigate_playlist(backend, MediaKind::Audio, selector, direction)
        }
        ("GET", ["v1", "audio", "playlist", playlist, item, "trigger"]) => {
            trigger_item(backend, MediaKind::Audio, playlist, item)
        }
        ("GET", ["v1", "media", "playlist", playlist, item, "trigger"]) => {
            trigger_item(backend, MediaKind::Graphics, playlist, item)
        }
        _ => return None,
    })
}

fn audio_playlists(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    playlist_tree(&status, MediaKind::Audio)
}

fn media_playlists(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    playlist_tree(&status, MediaKind::Graphics)
}

fn playlist_detail(
    backend: &dyn Backend,
    kind: MediaKind,
    selector: &str,
) -> Result<Value, String> {
    let status = backend.full()?;
    let (playlist, items) = resolve_playlist_items(backend, &status, kind, selector)?;
    Ok(json!({
        "id": identity(
            required_id(playlist.library, "id", "Media playlist identity is required.")?,
            required_string(playlist.library, "title", "Media playlist title is required.")?,
            playlist.api_index
        ),
        "items": items
    }))
}

fn playlist_tree(status: &Value, kind: MediaKind) -> Result<Value, String> {
    Ok(playlist_catalog(status, kind)?.0)
}

fn playlist_catalog<'a>(
    status: &'a Value,
    kind: MediaKind,
) -> Result<(Value, Vec<Playlist<'a>>), String> {
    let libraries = required_array(
        &status["modernCatalog"],
        "mediaLibraries",
        "Media library catalog is unavailable.",
    )?;
    let mut included = HashSet::new();
    for (index, library) in libraries.iter().enumerate() {
        let library_type = required_string(library, "type", "Media library type is required.")?;
        match library_type {
            "playlist" | "smart" => {
                let media_type = required_string(
                    library,
                    "mediaType",
                    "Media playlist category is unavailable.",
                )?;
                if media_type == kind.media_type() {
                    included.insert(index);
                } else if !matches!(media_type, "audio" | "graphics") {
                    return Err("Media playlist category is unsupported.".into());
                }
            }
            "folder" => {}
            _ => return Err("Media library type is unsupported.".into()),
        }
    }

    let selected_leaves = included.iter().copied().collect::<Vec<_>>();
    for leaf in selected_leaves {
        let mut cursor = leaf;
        let mut ancestors = HashSet::new();
        while let Some(parent_id) = parent_id(&libraries[cursor])? {
            if !ancestors.insert(cursor) {
                return Err("Media library folder structure is cyclic.".into());
            }
            let parent_index = find_library_index(libraries, parent_id)?;
            if required_string(
                &libraries[parent_index],
                "type",
                "Media library type is required.",
            )? != "folder"
            {
                return Err("Media playlists must be nested in folders.".into());
            }
            included.insert(parent_index);
            cursor = parent_index;
        }
    }

    for (index, library) in libraries.iter().enumerate() {
        if required_string(library, "type", "Media library type is required.")? == "folder"
            && library.get("mediaType").and_then(Value::as_str) == Some(kind.media_type())
        {
            included.insert(index);
        }
    }

    let mut nodes = Vec::new();
    let mut playlists = Vec::new();
    let mut seen = HashSet::new();
    build_nodes(
        libraries,
        None,
        &included,
        &mut seen,
        &mut playlists,
        &mut Vec::new(),
        &mut nodes,
    )?;
    if seen != included {
        return Err("A media library parent is unavailable or cyclic.".into());
    }
    Ok((Value::Array(nodes), playlists))
}

fn build_nodes<'a>(
    libraries: &'a [Value],
    parent: Option<&str>,
    included: &HashSet<usize>,
    seen: &mut HashSet<usize>,
    playlists: &mut Vec<Playlist<'a>>,
    ancestors: &mut Vec<String>,
    result: &mut Vec<Value>,
) -> Result<(), String> {
    for (source_index, library) in libraries.iter().enumerate() {
        if !included.contains(&source_index) || parent_id(library)? != parent {
            continue;
        }
        if !seen.insert(source_index) {
            return Err("Media library folder structure is cyclic.".into());
        }
        let id = required_id(library, "id", "Media library identity is required.")?;
        let title = required_string(library, "title", "Media library title is required.")?;
        let kind = required_string(library, "type", "Media library type is required.")?;
        let index = result.len();
        match kind {
            "folder" => {
                if ancestors.iter().any(|ancestor| ancestor == id) {
                    return Err("Media library folder structure is cyclic.".into());
                }
                ancestors.push(id.to_string());
                let mut children = Vec::new();
                build_nodes(
                    libraries,
                    Some(id),
                    included,
                    seen,
                    playlists,
                    ancestors,
                    &mut children,
                )?;
                ancestors.pop();
                result.push(json!({
                    "id": identity(id, title, index),
                    "type": "group",
                    "children": children
                }));
            }
            "playlist" | "smart" => {
                playlists.push(Playlist {
                    library,
                    api_index: index,
                });
                result.push(json!({
                    "id": identity(id, title, index),
                    "type": "playlist",
                    "children": []
                }));
            }
            _ => return Err("Media library type is unsupported.".into()),
        }
    }
    Ok(())
}

fn resolve_playlist<'a>(
    status: &'a Value,
    kind: MediaKind,
    selector: &str,
) -> Result<Playlist<'a>, String> {
    let (_, playlists) = playlist_catalog(status, kind)?;
    let mut matching = Vec::new();
    for playlist in playlists {
        let id = required_string(
            playlist.library,
            "id",
            "Media playlist identity is required.",
        )?;
        let title = required_string(
            playlist.library,
            "title",
            "Media playlist title is required.",
        )?;
        if matches_id(id, title, playlist.api_index, selector) {
            matching.push(playlist);
        }
    }
    match matching.as_slice() {
        [playlist] => Ok(Playlist {
            library: playlist.library,
            api_index: playlist.api_index,
        }),
        [] => Err("Media playlist is unavailable.".into()),
        _ => Err("The media playlist identifier is ambiguous.".into()),
    }
}

fn resolve_playlist_items<'a>(
    backend: &dyn Backend,
    status: &'a Value,
    kind: MediaKind,
    selector: &str,
) -> Result<(Playlist<'a>, Vec<Value>), String> {
    let (playlist, assets) = resolve_playlist_assets(status, kind, selector)?;
    let mut result = Vec::new();
    for asset in assets {
        let source_id = required_id(asset, "id", "Media item identity is required.")?;
        let item_type = required_string(asset, "type", "Media item type is required.")?;
        let title = required_string(asset, "title", "Media item title is required.")?;
        let artist = media_metadata::artist(asset)?;
        let duration = media_metadata::api_duration(asset_duration(backend, asset)?)?;
        result.push(json!({
            "id": identity(source_id, title, result.len()),
            "type": item_type,
            "artist": artist,
            "duration": duration
        }));
    }
    Ok((playlist, result))
}

fn asset_duration(backend: &dyn Backend, asset: &Value) -> Result<f64, String> {
    match asset.get("durationSeconds") {
        Some(Value::Null) | None => media_metadata::duration(&backend.media_metadata(asset)?),
        Some(Value::Number(_)) => media_metadata::duration(asset),
        Some(_) => Err("Media item duration metadata is invalid.".into()),
    }
}

fn resolve_playlist_assets<'a>(
    status: &'a Value,
    kind: MediaKind,
    selector: &str,
) -> Result<(Playlist<'a>, Vec<&'a Value>), String> {
    let playlist = resolve_playlist(status, kind, selector)?;
    let library_id = required_string(
        playlist.library,
        "id",
        "Media playlist identity is required.",
    )?;
    let bin = media_bin(status, library_id)?;
    let items = required_array(bin, "items", "Media playlist items are unavailable.")?;
    let mut result = Vec::new();
    for item in items {
        let source_id = required_id(item, "id", "Media item identity is required.")?;
        let asset = media_asset(status, source_id)?;
        let item_type = required_string(asset, "type", "Media item type is required.")?;
        if !kind.accepts_item(item_type) {
            continue;
        }
        required_string(asset, "title", "Media item title is required.")?;
        result.push(asset);
    }
    Ok((playlist, result))
}

fn active_playlist(backend: &dyn Backend, kind: MediaKind) -> Result<Value, String> {
    let status = backend.full()?;
    let active = active_media(&status, kind)?;
    Ok(json!({ "playlist": active.0, "item": active.1 }))
}

fn active_media(status: &Value, kind: MediaKind) -> Result<(Value, Value), String> {
    let state_key = match kind {
        MediaKind::Audio => "audio",
        MediaKind::Graphics => "background",
    };
    let Some(source_id) = status[state_key]["id"]
        .as_str()
        .filter(|source_id| !source_id.is_empty())
    else {
        return Ok((Value::Null, Value::Null));
    };
    let asset = media_asset(status, source_id)?;
    let item_type = required_string(asset, "type", "Media item type is required.")?;
    if !kind.accepts_item(item_type) {
        return Err("The active media type does not match this playlist.".into());
    }
    let title = required_string(asset, "title", "Media item title is required.")?;
    let source_index = media_assets(status)?
        .iter()
        .position(|candidate| candidate["id"].as_str() == Some(source_id))
        .ok_or("The active media item is unavailable.")?;
    let (_, candidates) = playlist_catalog(status, kind)?;
    let mut memberships = Vec::new();
    for candidate in candidates {
        let library_id = required_string(
            candidate.library,
            "id",
            "Media playlist identity is required.",
        )?;
        let bin = media_bin(status, library_id)?;
        let items = required_array(bin, "items", "Media playlist items are unavailable.")?;
        if let Some(item_index) = items
            .iter()
            .filter(|item| {
                item.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|item_type| kind.accepts_item(item_type))
            })
            .position(|item| item["id"].as_str() == Some(source_id))
        {
            memberships.push((candidate, item_index));
        }
    }
    if memberships.len() > 1 {
        return Err("The active media item belongs to multiple playlists.".into());
    }
    let (playlist, index) = match memberships.pop() {
        Some((playlist, index)) => (
            identity(
                required_string(
                    playlist.library,
                    "id",
                    "Media playlist identity is required.",
                )?,
                required_string(
                    playlist.library,
                    "title",
                    "Media playlist title is required.",
                )?,
                playlist.api_index,
            ),
            index,
        ),
        None => (Value::Null, source_index),
    };
    Ok((playlist, identity(source_id, title, index)))
}

fn transport_current(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let audio = &status["audio"];
    let source_id = required_id(audio, "id", "There is no active audio item.")?;
    let asset = media_asset(&status, source_id)?;
    if required_string(asset, "type", "Media item type is required.")? != "audio" {
        return Err("The active transport item is not audio.".into());
    }
    let title = required_string(asset, "title", "Media item title is required.")?;
    let artist = media_metadata::artist(asset)?;
    let duration = match audio
        .get("playback")
        .and_then(|playback| playback.get("duration"))
    {
        Some(value) => value
            .as_f64()
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            .ok_or("Audio playback duration is invalid.")?,
        None => asset_duration(backend, asset)?,
    };
    let paused = audio
        .get("paused")
        .and_then(Value::as_bool)
        .ok_or("Audio playback state is unavailable.")?;
    let item_uuid = identity(source_id, title, 0)["uuid"]
        .as_str()
        .ok_or("Media item UUID is unavailable.")?
        .to_string();
    Ok(json!({
        "uuid": item_uuid,
        "name": title,
        "artist": artist,
        "audio_only": true,
        "duration": duration,
        "is_playing": !paused
    }))
}

fn set_audio_transport(backend: &dyn Backend, action: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let audio = &status["audio"];
    let id = required_id(audio, "id", "There is no active audio item.")?;
    let selection_id = required_number(
        audio.get("selectionId"),
        "Audio playback occurrence is unavailable.",
    )?;
    backend.command(&json!({
        "command": "media",
        "workspacePath": data::workspace_path(&status)?,
        "id": id,
        "action": action,
        "selectionId": selection_id
    }))?;
    Ok(Value::Null)
}

fn trigger_playlist(
    backend: &dyn Backend,
    kind: MediaKind,
    selector: &str,
) -> Result<Value, String> {
    let status = backend.full()?;
    let playlist_selector = if selector == "active" || selector == "focused" {
        let active_id = active_source_id(&status, kind)?;
        let playlist = active_playlist_for_item(&status, kind, active_id)?;
        api_selector(playlist.library, playlist.api_index)?
    } else {
        selector.to_string()
    };
    let (_, items) = resolve_playlist_assets(&status, kind, &playlist_selector)?;
    let first = items.first().ok_or("The media playlist is empty.")?;
    let source_id = required_id(first, "id", "Media item identity is required.")?;
    trigger_source_item(backend, &status, source_id)
}

fn trigger_item(
    backend: &dyn Backend,
    kind: MediaKind,
    playlist_selector: &str,
    item_selector: &str,
) -> Result<Value, String> {
    let status = backend.full()?;
    let playlist_selector = if playlist_selector == "active" || playlist_selector == "focused" {
        let active_id = active_source_id(&status, kind)?;
        let playlist = active_playlist_for_item(&status, kind, active_id)?;
        api_selector(playlist.library, playlist.api_index)?
    } else {
        playlist_selector.to_string()
    };
    let (_playlist, items) = resolve_playlist_assets(&status, kind, &playlist_selector)?;
    let mut matches = Vec::new();
    for (index, asset) in items.iter().enumerate() {
        let id = required_id(asset, "id", "Media item identity is required.")?;
        let name = required_string(asset, "title", "Media item title is required.")?;
        if matches_id(id, name, index, item_selector) {
            matches.push(*asset);
        }
    }
    let [asset] = matches.as_slice() else {
        return Err(if matches.is_empty() {
            "Media item is unavailable in the playlist.".into()
        } else {
            "The media item identifier is ambiguous.".into()
        });
    };
    let source_id = required_id(asset, "id", "Media item identity is required.")?;
    trigger_source_item(backend, &status, source_id)
}

fn navigate_playlist(
    backend: &dyn Backend,
    kind: MediaKind,
    selector: &str,
    direction: &str,
) -> Result<Value, String> {
    if kind != MediaKind::Audio {
        return Err("Playlist navigation requires audio.".into());
    }
    let status = backend.full()?;
    let source_playlist = if selector == "active" || selector == "focused" {
        let active_id = active_source_id(&status, MediaKind::Audio)?;
        active_playlist_for_item(&status, MediaKind::Audio, active_id)?
    } else {
        resolve_playlist(&status, MediaKind::Audio, selector)?
    };
    let library_id = required_string(
        source_playlist.library,
        "id",
        "Media playlist identity is required.",
    )?;
    let bin = media_bin(&status, library_id)?;
    let items = required_array(bin, "items", "Media playlist items are unavailable.")?;
    let order = items
        .iter()
        .filter(|item| {
            item["type"]
                .as_str()
                .is_some_and(|kind| MediaKind::Audio.accepts_item(kind))
        })
        .map(|item| item["id"].clone())
        .collect::<Vec<_>>();
    if order.is_empty() {
        return Err("The audio playlist is empty.".into());
    }
    let audio = &status["audio"];
    let id = required_id(audio, "id", "There is no active audio item.")?;
    if !order.iter().any(|candidate| candidate.as_str() == Some(id)) {
        return Err("The active audio item is not in this playlist.".into());
    }
    let selection_id = required_number(
        audio.get("selectionId"),
        "Audio playback occurrence is unavailable.",
    )?;
    backend.command(&json!({
        "command": "media",
        "workspacePath": data::workspace_path(&status)?,
        "id": id,
        "action": direction,
        "selectionId": selection_id,
        "playlistId": library_id,
        "playlistOrder": order
    }))?;
    Ok(Value::Null)
}

fn active_source_id<'a>(status: &'a Value, kind: MediaKind) -> Result<&'a str, String> {
    let state_key = match kind {
        MediaKind::Audio => "audio",
        MediaKind::Graphics => "background",
    };
    required_id(&status[state_key], "id", "There is no active media item.")
}

fn active_playlist_for_item<'a>(
    status: &'a Value,
    kind: MediaKind,
    source_id: &str,
) -> Result<Playlist<'a>, String> {
    let (_, playlists) = playlist_catalog(status, kind)?;
    let mut matching = Vec::new();
    for playlist in playlists {
        let library_id = required_string(
            playlist.library,
            "id",
            "Media playlist identity is required.",
        )?;
        let bin = media_bin(status, library_id)?;
        if required_array(bin, "items", "Media playlist items are unavailable.")?
            .iter()
            .any(|item| item["id"].as_str() == Some(source_id))
        {
            matching.push(playlist);
        }
    }
    match matching.as_slice() {
        [playlist] => Ok(Playlist {
            library: playlist.library,
            api_index: playlist.api_index,
        }),
        [] => Err("The active media item is not in a playlist.".into()),
        _ => Err("The active media item belongs to multiple playlists.".into()),
    }
}

fn trigger_source_item(
    backend: &dyn Backend,
    status: &Value,
    source_id: &str,
) -> Result<Value, String> {
    backend.command(&json!({
        "command": "media",
        "workspacePath": data::workspace_path(status)?,
        "id": source_id,
        "action": "trigger"
    }))?;
    Ok(Value::Null)
}

fn api_selector(library: &Value, index: usize) -> Result<String, String> {
    let id = required_id(library, "id", "Media playlist identity is required.")?;
    let title = required_string(library, "title", "Media playlist title is required.")?;
    identity(id, title, index)["uuid"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Media playlist UUID is unavailable.".into())
}

fn media_bin<'a>(status: &'a Value, library_id: &str) -> Result<&'a Value, String> {
    let bins = required_array(status, "mediaBins", "Media bins are unavailable.")?;
    let matching = bins
        .iter()
        .filter(|bin| bin["id"].as_str() == Some(library_id))
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [bin] => Ok(*bin),
        [] => Err("Media playlist is unavailable in the accepted catalog.".into()),
        _ => Err("Media playlist identity is ambiguous in the accepted catalog.".into()),
    }
}

fn media_assets(status: &Value) -> Result<&[Value], String> {
    required_array(
        &status["modernCatalog"],
        "media",
        "Media item catalog is unavailable.",
    )
}

fn media_asset<'a>(status: &'a Value, source_id: &str) -> Result<&'a Value, String> {
    let matching = media_assets(status)?
        .iter()
        .filter(|asset| asset["id"].as_str() == Some(source_id))
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [asset] => Ok(*asset),
        [] => Err("Media item is unavailable in the workspace catalog.".into()),
        _ => Err("Media item identity is ambiguous in the workspace catalog.".into()),
    }
}

fn find_library_index(libraries: &[Value], source_id: &str) -> Result<usize, String> {
    let matches = libraries
        .iter()
        .enumerate()
        .filter(|(_, library)| library["id"].as_str() == Some(source_id))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [index] => Ok(*index),
        [] => Err("Media playlist parent is unavailable.".into()),
        _ => Err("Media library identity is ambiguous.".into()),
    }
}

fn parent_id(library: &Value) -> Result<Option<&str>, String> {
    match library.get("parentId") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(parent)) if parent.is_empty() => Ok(None),
        Some(Value::String(parent)) => Ok(Some(parent)),
        Some(_) => Err("Media library parent identity must be a string.".into()),
    }
}

fn required_array<'a>(value: &'a Value, field: &str, message: &str) -> Result<&'a [Value], String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| message.to_string())
}

fn required_string<'a>(value: &'a Value, field: &str, message: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| message.to_string())
}

fn required_id<'a>(value: &'a Value, field: &str, message: &str) -> Result<&'a str, String> {
    required_string(value, field, message).and_then(|id| {
        (!id.is_empty())
            .then_some(id)
            .ok_or_else(|| message.to_string())
    })
}

fn required_number(value: Option<&Value>, message: &str) -> Result<f64, String> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number >= 0.0)
        .ok_or_else(|| message.to_string())
}
