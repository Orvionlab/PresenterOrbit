//! Writes workspace media libraries as ProPresenter media/audio PlaylistDocuments.

use super::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const GRAPHICS: &str = "graphics";
const AUDIO: &str = "audio";

pub(crate) fn encode(
    workspace: &Value,
    relative: &BTreeMap<String, String>,
    media_type: &str,
) -> Result<Vec<u8>, String> {
    if !matches!(media_type, GRAPHICS | AUDIO) {
        return Err("Media bin type must be graphics or audio.".into());
    }
    let libraries = workspace
        .get("mediaLibraries")
        .and_then(Value::as_array)
        .ok_or("Workspace media libraries are required.")?;
    let media = workspace
        .get("media")
        .and_then(Value::as_array)
        .ok_or("Workspace media items are required.")?;
    let indexes = LibraryIndex::new(libraries, media)?;
    let mut root = Vec::new();
    put_uuid(
        1,
        &format!("presenter-orbit-media-root:{media_type}"),
        &mut root,
    );
    put_string(
        2,
        if media_type == AUDIO {
            "Audio"
        } else {
            "Media"
        },
        &mut root,
    );
    put_varint(3, 4, &mut root); // Playlist.TYPE_ROOT

    let mut children = Vec::new();
    for library in libraries {
        if parent_id(library)? != "" {
            continue;
        }
        if let Some(node) = encode_library(library, &indexes, relative, media_type)? {
            put_message(1, node, &mut children);
        }
    }
    let assigned: HashSet<&str> = libraries
        .iter()
        .filter(|library| library["type"] == "playlist")
        .flat_map(|library| library["itemFiles"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .filter_map(|file| indexes.media_by_file(file))
        .filter_map(|item| item["id"].as_str())
        .collect();
    let smart_ids: HashSet<&str> = libraries
        .iter()
        .filter(|library| library["type"] == "smart")
        .filter_map(|library| library["id"].as_str())
        .collect();
    let unassigned = media
        .iter()
        .filter(|item| {
            media_category(item) == Some(media_type)
                && !item["id"].as_str().is_some_and(|id| assigned.contains(id))
                && !item["libraryId"]
                    .as_str()
                    .is_some_and(|id| smart_ids.contains(id))
        })
        .collect::<Vec<_>>();
    if !unassigned.is_empty() {
        let library = json!({"id":Uuid::new_v4().to_string(),"title":"Unassigned Media"});
        let mut node = Vec::new();
        put_uuid(1, library["id"].as_str().unwrap(), &mut node);
        put_string(2, "Unassigned Media", &mut node);
        put_varint(3, 1, &mut node);
        let mut items = Vec::new();
        for (index, item) in unassigned.iter().enumerate() {
            let file = required_string(
                item,
                "assetFile",
                "Unassigned media requires an asset file.",
            )?;
            put_message(
                1,
                encode_media_playlist_item(
                    &library,
                    item,
                    file,
                    index,
                    index + 1 == unassigned.len(),
                    relative,
                )?,
                &mut items,
            );
        }
        put_message(13, items, &mut node);
        put_message(1, node, &mut children);
    }
    put_message(12, children, &mut root); // Playlist.PlaylistArray

    let mut document = Vec::new();
    put_varint(2, if media_type == AUDIO { 3 } else { 2 }, &mut document);
    put_message(3, root, &mut document);
    Ok(document)
}

/// Losses are kept separate so an unsupported smart source does not discard
/// unrelated, portable playlists from the same bin.
pub(crate) fn losses(workspace: &Value, media_type: &str) -> Vec<String> {
    if !matches!(media_type, GRAPHICS | AUDIO) {
        return vec!["Media bin type must be graphics or audio.".into()];
    }
    let Some(libraries) = workspace.get("mediaLibraries").and_then(Value::as_array) else {
        return vec!["Workspace media libraries are required.".into()];
    };
    let Some(media) = workspace.get("media").and_then(Value::as_array) else {
        return vec!["Workspace media items are required.".into()];
    };
    let Ok(indexes) = LibraryIndex::new(libraries, media) else {
        return vec!["Media libraries could not be classified for ProPresenter export.".into()];
    };

    let mut findings = BTreeSet::new();
    for library in libraries {
        if library["type"] == "smart" {
            if indexes.includes(library, media_type) {
                match smart_source_url(library) {
                    Ok(_) => {
                        findings.insert(format!(
                            "{} smart media source folder remains an external live dependency.",
                            label(library)
                        ));
                    }
                    Err(error) => {
                        findings.insert(format!("{}: {error}", label(library)));
                    }
                }
            }
            continue;
        }
        if !indexes.includes(library, media_type) {
            continue;
        }
        if library.get("type").and_then(Value::as_str) != Some("playlist") {
            continue;
        }
        if media_type == AUDIO {
            match library.get("audioPlaylistMode").and_then(Value::as_str) {
                Some("stop" | "next" | "repeat") | None => {}
                Some(mode) => {
                    findings.insert(format!(
                        "{} has unsupported audioPlaylistMode {mode:?}.",
                        label(library)
                    ));
                }
            }
        }
        let Some(item_files) = library.get("itemFiles").and_then(Value::as_array) else {
            findings.insert(format!("{} has no itemFiles list.", label(library)));
            continue;
        };
        for item_file in item_files.iter().filter_map(Value::as_str) {
            let Some(item) = indexes.media_by_file(item_file) else {
                findings.insert(format!(
                    "{} references missing media file {item_file:?}.",
                    label(library)
                ));
                continue;
            };
            let Some(kind) = item.get("type").and_then(Value::as_str) else {
                findings.insert(format!("{} has media without a type.", media_label(item)));
                continue;
            };
            let style = style_value(item, kind);
            for key in layer_media_export::unsupported_fields(&style, "media") {
                findings.insert(format!(
                    "{} {key} is not represented in ProPresenter.",
                    media_label(item)
                ));
            }
            media_losses(item, kind, &mut findings);
        }
    }
    for item in media {
        let Some(kind) = item["type"].as_str() else {
            findings.insert(format!("{} has no media type.", media_label(item)));
            continue;
        };
        if media_category(item).is_none() && media_type == GRAPHICS {
            findings.insert(format!(
                "{} has unsupported media type {kind:?}.",
                media_label(item)
            ));
        }
        if media_category(item) == Some(media_type) {
            for key in layer_media_export::unsupported_fields(&style_value(item, kind), "media") {
                findings.insert(format!(
                    "{} {key} is not represented in ProPresenter.",
                    media_label(item)
                ));
            }
            media_losses(item, kind, &mut findings);
        }
    }
    findings.into_iter().collect()
}

struct LibraryIndex<'a> {
    libraries: &'a [Value],
    media: &'a [Value],
    library_by_id: HashMap<&'a str, &'a Value>,
    media_by_asset_file: HashMap<&'a str, &'a Value>,
}

impl<'a> LibraryIndex<'a> {
    fn new(libraries: &'a [Value], media: &'a [Value]) -> Result<Self, String> {
        let mut library_by_id = HashMap::new();
        for library in libraries {
            if !matches!(
                library["type"].as_str(),
                Some("folder" | "playlist" | "smart")
            ) {
                return Err("Media library type must be folder, playlist or smart.".into());
            }
            if library
                .get("mediaType")
                .is_some_and(|kind| !matches!(kind.as_str(), Some("graphics" | "audio")))
            {
                return Err("Media library mediaType must be graphics or audio.".into());
            }
            let id = required_string(library, "id", "Media library id is required.")?;
            if library_by_id.insert(id, library).is_some() {
                return Err(format!("Media library id {id:?} is duplicated."));
            }
            required_string(library, "title", "Media library title is required.")?;
            if !matches!(
                library.get("type").and_then(Value::as_str),
                Some("folder" | "playlist" | "smart")
            ) {
                return Err(format!("Media library {id:?} has an unsupported type."));
            }
        }
        let smart_library_ids = libraries
            .iter()
            .filter(|library| library.get("type").and_then(Value::as_str) == Some("smart"))
            .filter_map(|library| library.get("id").and_then(Value::as_str))
            .collect::<HashSet<_>>();
        let mut media_by_asset_file = HashMap::new();
        for item in media {
            let id = required_string(item, "id", "Media item id is required.")?;
            if let Some(asset_file) = item.get("assetFile").and_then(Value::as_str) {
                if media_by_asset_file.insert(asset_file, item).is_some() {
                    return Err(format!(
                        "Media asset file {asset_file:?} is assigned to multiple media items."
                    ));
                }
            } else if item.get("missing").and_then(Value::as_bool) != Some(true)
                && !item
                    .get("libraryId")
                    .and_then(Value::as_str)
                    .is_some_and(|library_id| smart_library_ids.contains(library_id))
            {
                // Unreferenced media and smart-source entries need not have a managed asset.
                // A regular playlist reference is checked when its itemFiles are encoded.
                let _ = id;
            }
        }
        let index = Self {
            libraries,
            media,
            library_by_id,
            media_by_asset_file,
        };
        for library in libraries {
            index.validate_parent(library)?;
        }
        for library in libraries {
            let mut visited = HashSet::new();
            let mut current = library;
            while !parent_id(current)?.is_empty() {
                let id = required_string(current, "id", "Media library id is required.")?;
                if !visited.insert(id) {
                    return Err(format!(
                        "Media library hierarchy contains a cycle at {id:?}."
                    ));
                }
                current = index.library_by_id[parent_id(current)?];
            }
        }
        Ok(index)
    }

    fn validate_parent(&self, library: &Value) -> Result<(), String> {
        let id = required_string(library, "id", "Media library id is required.")?;
        let parent = parent_id(library)?;
        if parent.is_empty() {
            return Ok(());
        }
        let parent_library = self
            .library_by_id
            .get(parent)
            .ok_or_else(|| format!("Media library {id:?} has missing parent {parent:?}."))?;
        if parent_library.get("type").and_then(Value::as_str) != Some("folder") {
            return Err(format!("Media library {id:?} must be inside a folder."));
        }
        Ok(())
    }

    fn media_by_file(&self, file: &str) -> Option<&'a Value> {
        self.media_by_asset_file.get(file).copied()
    }

    fn includes(&self, library: &Value, media_type: &str) -> bool {
        library
            .get("mediaType")
            .and_then(Value::as_str)
            .is_none_or(|declared| declared == media_type)
    }
}

fn encode_library(
    library: &Value,
    indexes: &LibraryIndex<'_>,
    relative: &BTreeMap<String, String>,
    media_type: &str,
) -> Result<Option<Vec<u8>>, String> {
    let kind = library
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !indexes.includes(library, media_type) {
        return Ok(None);
    }
    let id = required_string(library, "id", "Media library id is required.")?;
    let title = required_string(library, "title", "Media library title is required.")?;
    let mut node = Vec::new();
    put_uuid(1, id, &mut node);
    put_string(2, title, &mut node);
    put_varint(
        3,
        match kind {
            "folder" => 2,
            "smart" => 3,
            _ => 1,
        },
        &mut node,
    );
    put_varint(
        4,
        u64::from(library["collapsed"].as_bool() != Some(true)),
        &mut node,
    );

    if kind == "smart" {
        let folder = smart_source_url(library)?;
        let mut directory = Vec::new();
        let mut url = Vec::new();
        put_string(1, &folder, &mut url);
        put_message(1, url, &mut directory);
        if let Some(behavior) = library["backgroundBehavior"].as_str() {
            put_varint(2, layer_type(behavior)?, &mut directory);
        }
        put_message(14, directory, &mut node);
        let mut media = indexes
            .media
            .iter()
            .filter(|item| item["libraryId"] == id && media_category(item) == Some(media_type))
            .collect::<Vec<_>>();
        if let Some(order) = library["graphicsOrder"].as_array() {
            media.sort_by_key(|item| {
                order
                    .iter()
                    .position(|entry| entry == &item["id"])
                    .unwrap_or(order.len())
            });
        }
        let mut items = Vec::new();
        for (index, item) in media.iter().enumerate() {
            let reference =
                required_string(item, "sourceUrl", "Smart media source URL is required.")?;
            put_message(
                1,
                encode_media_playlist_item(
                    library,
                    item,
                    reference,
                    index,
                    index + 1 == media.len(),
                    relative,
                )?,
                &mut items,
            );
        }
        put_message(13, items, &mut node);
        return Ok(Some(node));
    }
    if kind == "folder" {
        let mut child_array = Vec::new();
        for child in indexes
            .libraries
            .iter()
            .filter(|candidate| candidate.get("parentId").and_then(Value::as_str) == Some(id))
        {
            if let Some(encoded) = encode_library(child, indexes, relative, media_type)? {
                put_message(1, encoded, &mut child_array);
            }
        }
        put_message(12, child_array, &mut node);
    } else {
        let item_files = library
            .get("itemFiles")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("Media playlist {id:?} requires an itemFiles list."))?;
        let last_index = item_files
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, file)| {
                file.as_str()
                    .and_then(|file| indexes.media_by_file(file))
                    .filter(|item| media_category(item) == Some(media_type))
                    .map(|_| index)
            });
        let mut playlist_items = Vec::new();
        for (index, item_file) in item_files.iter().enumerate() {
            let item_file = item_file
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("Media playlist {id:?} contains an invalid item file."))?;
            let media = indexes.media_by_file(item_file).ok_or_else(|| {
                format!("Media playlist {id:?} references missing media file {item_file:?}.")
            })?;
            let media_category = media_category(media).ok_or_else(|| {
                format!("Media playlist {id:?} references unsupported media type.")
            })?;
            let declared_category = library.get("mediaType").and_then(Value::as_str);
            if declared_category.is_some_and(|declared| declared != media_category) {
                return Err(format!(
                    "Media playlist {id:?} contains an item outside its {declared_category:?} category."
                ));
            }
            if media_category != media_type {
                continue;
            }
            put_message(
                1,
                encode_media_playlist_item(
                    library,
                    media,
                    item_file,
                    index,
                    Some(index) == last_index,
                    relative,
                )?,
                &mut playlist_items,
            );
        }
        put_message(13, playlist_items, &mut node);
    }
    Ok(Some(node))
}

fn encode_media_playlist_item(
    library: &Value,
    media: &Value,
    item_file: &str,
    index: usize,
    last_in_playlist: bool,
    relative: &BTreeMap<String, String>,
) -> Result<Vec<u8>, String> {
    let library_id = required_string(library, "id", "Media library id is required.")?;
    let media_id = required_string(media, "id", "Media item id is required.")?;
    let title = required_string(media, "title", "Media item title is required.")?;
    let kind = media
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Media item {media_id:?} has no type."))?;
    let action_media = encode_action_media(media, item_file, relative)?;

    let mut action = Vec::new();
    put_uuid(
        1,
        &format!("media-action:{library_id}:{media_id}:{index}"),
        &mut action,
    );
    put_string(2, title, &mut action);
    put_varint(6, 1, &mut action); // Action.isEnabled
    put_varint(9, if kind == "audio" { 20 } else { 21 }, &mut action);
    if let Some(delay) = media
        .get("mediaTransport")
        .and_then(|transport| transport.get("delaySeconds"))
        .and_then(Value::as_f64)
    {
        if delay > 0.0 {
            put_f64(4, delay, &mut action);
        }
    }
    put_message(20, action_media, &mut action);

    let mut cue = Vec::new();
    let cue_id = media
        .get("sourceId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(media_id);
    put_uuid(1, cue_id, &mut cue);
    put_string(2, title, &mut cue);
    put_varint(12, 1, &mut cue);
    if kind == "audio" && library["type"] != "smart" {
        let target = match library["audioPlaylistMode"].as_str() {
            None | Some("stop") => 0,
            Some("next") => {
                if last_in_playlist {
                    0
                } else {
                    1
                }
            }
            Some("repeat") => {
                if last_in_playlist {
                    4
                } else {
                    1
                }
            }
            Some(mode) => return Err(format!("Unsupported audio playlist mode: {mode}")),
        };
        if target != 0 {
            put_varint(3, target, &mut cue);
            put_varint(5, 1, &mut cue); // Complete after the last audio action.
        }
    }
    put_message(10, action, &mut cue);

    let mut playlist_item = Vec::new();
    put_uuid(
        1,
        &format!("media-item:{library_id}:{media_id}:{index}"),
        &mut playlist_item,
    );
    put_string(2, title, &mut playlist_item);
    put_message(5, cue, &mut playlist_item);
    Ok(playlist_item)
}

/// The `Action.MediaType` that plays a workspace media item; also used for slide media.
fn encode_action_media(
    media: &Value,
    item_file: &str,
    relative: &BTreeMap<String, String>,
) -> Result<Vec<u8>, String> {
    let media_id = required_string(media, "id", "Media item id is required.")?;
    let kind = media
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Media item {media_id:?} has no type."))?;
    let (absolute_url, relative_url) = media_url(media, item_file, relative)?;
    let (properties_field, properties) = encode_type_properties(media, kind)?;

    let mut element = Vec::new();
    put_uuid(1, media_id, &mut element);
    put_message(
        2,
        encode_local_media_url(&absolute_url, &relative_url),
        &mut element,
    );
    put_message(properties_field, properties, &mut element);

    let mut action_media = Vec::new();
    put_message(5, element, &mut action_media);
    let behavior = media
        .get("backgroundBehavior")
        .and_then(Value::as_str)
        .unwrap_or("foreground");
    put_varint(10, layer_type(behavior)?, &mut action_media);
    if let Some(transition) = media.get("backgroundTransition").and_then(Value::as_str) {
        if let Some(transition) = crate::native_transitions::media_transition(transition)? {
            if let Some(encoded) = encode_transition(&transition) {
                put_message(3, encoded, &mut action_media);
            } else {
                return Err(format!(
                    "Media item {media_id:?} transition could not be encoded."
                ));
            }
        }
    }
    match kind {
        "image" => put_message(6, Vec::new(), &mut action_media),
        "video" => {
            let mut video_action = Vec::new();
            let playback = media
                .get("backgroundPlayback")
                .and_then(Value::as_str)
                .unwrap_or("stop");
            put_varint(1, playback_enum(playback)?, &mut video_action);
            if let Some(end_behavior) = media.get("backgroundEndBehavior").and_then(Value::as_str) {
                put_varint(2, end_behavior_enum(end_behavior)?, &mut video_action);
            }
            put_message(7, video_action, &mut action_media);
        }
        "audio" => {
            let mut audio_action = Vec::new();
            let playback = media
                .get("audioPlayback")
                .and_then(Value::as_str)
                .unwrap_or("stop");
            let behavior = match playback {
                "loop-count" => 2,
                "loop-time" => 3,
                _ => playback_enum(playback)?,
            };
            put_varint(1, behavior, &mut audio_action);
            if behavior != 2 {
                if let Some(count) = media.get("audioLoopCount").and_then(Value::as_u64) {
                    put_varint(3, count, &mut audio_action);
                }
            }
            if behavior != 3 {
                if let Some(time) = media.get("audioLoopTime").and_then(Value::as_f64) {
                    put_f64(2, time, &mut audio_action);
                }
            }
            if behavior == 2 {
                let count = media["audioLoopCount"]
                    .as_u64()
                    .filter(|count| *count > 0 && *count <= u32::MAX as u64)
                    .ok_or("Audio loop count is invalid.")?;
                put_varint(3, count, &mut audio_action);
            }
            if behavior == 3 {
                let time = media["audioLoopTime"]
                    .as_f64()
                    .filter(|time| time.is_finite() && *time > 0.0)
                    .ok_or("Audio loop time is invalid.")?;
                put_f64(2, time, &mut audio_action);
            }
            put_message(8, audio_action, &mut action_media);
        }
        _ => {
            return Err(format!(
                "Media item {media_id:?} has unsupported type {kind:?}."
            ))
        }
    }
    encode_media_markers(media, &mut action_media)?;
    Ok(action_media)
}

/// A slide's media background as the media action of its cue.
pub(super) fn encode_media_cue_action(
    media: &Value,
    slide: &Value,
    relative: &BTreeMap<String, String>,
) -> Result<Vec<u8>, String> {
    let media_id = required_string(media, "id", "Media item id is required.")?;
    let cue = slide["sourceCueId"]
        .as_str()
        .or(slide["id"].as_str())
        .unwrap_or("slide");
    let mut action = Vec::new();
    put_uuid(
        1,
        &super::cue_export::derived_uuid(&format!("{cue}/media-background/{media_id}")),
        &mut action,
    );
    put_string(2, media["title"].as_str().unwrap_or(media_id), &mut action);
    put_varint(6, 1, &mut action);
    put_varint(9, 2, &mut action); // ACTION_TYPE_MEDIA
    put_message(
        20,
        encode_action_media(media, "slide media", relative)?,
        &mut action,
    );
    Ok(action)
}

const IMPORTED_MEDIA_DETAILS: &[&str] = &[
    "sourceUrl",
    "mediaType",
    "backgroundBehavior",
    "layerType",
    "playback",
    "endBehavior",
    "loopTime",
    "timesToLoop",
    "softLoop",
    "softLoopDuration",
    "alwaysRetrigger",
    "selectedEffectPresetId",
    "transitionDuration",
    "transition",
    "mediaEffects",
    "effects",
    "markers",
    "mediaTransport",
    "label",
    "labelColor",
];

/// What `encode_imported_media_action` cannot write for a slide media action.
pub(super) fn imported_media_action_losses(action: &Value) -> Vec<String> {
    let details = &action["details"];
    let mut losses = Vec::new();
    if details["sourceUrl"]
        .as_str()
        .is_none_or(|source| source.trim().is_empty())
    {
        losses.push("Media action without a media file is not exported.".to_string());
    }
    let kind = details["mediaType"].as_str().unwrap_or("media");
    if !matches!(kind, "image" | "video" | "audio") {
        losses.push(format!("Media action type {kind:?} is not exported."));
    }
    if details["transition"]
        .as_object()
        .is_some_and(|transition| !transition.is_empty())
    {
        losses.push("Media action transition is not exported.".to_string());
    }
    if details["effects"]
        .as_array()
        .is_some_and(|effects| !effects.is_empty())
    {
        losses.push("Media action non-library effects are not exported.".to_string());
    }
    if let Some(key) = details
        .as_object()
        .into_iter()
        .flat_map(|details| details.keys())
        .find(|key| {
            !IMPORTED_MEDIA_DETAILS.contains(&key.as_str()) && !key.starts_with("unsupported")
        })
    {
        losses.push(format!("Media action detail {key:?} is not exported."));
    }
    if let Some(key) = action
        .as_object()
        .into_iter()
        .flat_map(|action| action.keys())
        .find(|key| {
            !matches!(
                key.as_str(),
                "id" | "type"
                    | "title"
                    | "enabled"
                    | "delaySeconds"
                    | "durationSeconds"
                    | "targetId"
                    | "details"
            )
        })
    {
        losses.push(format!("Media action field {key:?} is not exported."));
    }
    losses
}

/// An imported slide media action in the `Action.MediaType` fields `decode_media_action` reads.
pub(super) fn encode_imported_media_action(
    action: &Value,
    relative: &BTreeMap<String, String>,
) -> Result<Vec<u8>, String> {
    if let Some(loss) = imported_media_action_losses(action).into_iter().next() {
        return Err(loss);
    }
    let details = &action["details"];
    let kind = details["mediaType"].as_str().unwrap_or_default();
    let id = required_string(action, "id", "Media action id is required.")?;
    let mut item = Map::new();
    item.insert("id".into(), json!(id));
    item.insert("type".into(), json!(kind));
    item.insert("sourceUrl".into(), details["sourceUrl"].clone());
    for key in ["mediaEffects", "mediaTransport", "markers"] {
        if let Some(value) = details.get(key) {
            item.insert(key.into(), value.clone());
        }
    }
    let item = Value::Object(item);
    let (absolute_url, relative_url) = media_url(&item, "slide media action", relative)?;
    let (properties_field, properties) = encode_type_properties(&item, kind)?;
    let mut element = Vec::new();
    put_uuid(1, id, &mut element);
    put_message(
        2,
        encode_local_media_url(&absolute_url, &relative_url),
        &mut element,
    );
    put_message(properties_field, properties, &mut element);

    let mut media = Vec::new();
    if let Some(duration) = details["transitionDuration"]
        .as_f64()
        .filter(|value| *value > 0.0)
    {
        put_f64(1, duration, &mut media);
    }
    if let Some(preset) = details["selectedEffectPresetId"]
        .as_str()
        .filter(|id| !id.is_empty())
    {
        put_uuid(2, preset, &mut media);
    }
    put_message(5, element, &mut media);
    let layer = match (
        details["backgroundBehavior"].as_str(),
        &details["layerType"],
    ) {
        (Some(behavior), _) => layer_type(behavior)?,
        (None, layer) if *layer == 0 || *layer == "LAYER_TYPE_BACKGROUND" => 0,
        (None, layer) if *layer == 1 || *layer == "LAYER_TYPE_FOREGROUND" => 1,
        (None, layer) => return Err(format!("Media action layerType {layer} is not exported.")),
    };
    put_varint(10, layer, &mut media);
    if details["alwaysRetrigger"] == true {
        put_varint(11, 1, &mut media);
    }
    let playback = match details["playback"].as_str().unwrap_or_default() {
        "" | "PLAYBACK_BEHAVIOR_STOP" => 0,
        "PLAYBACK_BEHAVIOR_LOOP" => 1,
        "PLAYBACK_BEHAVIOR_LOOP_FOR_COUNT" => 2,
        "PLAYBACK_BEHAVIOR_LOOP_FOR_TIME" => 3,
        value => return Err(format!("Media action playback {value:?} is not exported.")),
    };
    let loop_time = details["loopTime"].as_f64().filter(|value| *value > 0.0);
    let times_to_loop = details["timesToLoop"].as_u64().filter(|value| *value > 0);
    match kind {
        "image" => put_message(6, Vec::new(), &mut media),
        "video" => {
            let mut video = Vec::new();
            put_varint(1, playback, &mut video);
            let end = match details["endBehavior"].as_str().unwrap_or_default() {
                "" => None,
                "END_BEHAVIOR_STOP" => Some(0),
                "END_BEHAVIOR_STOP_ON_BLACK" => Some(1),
                "END_BEHAVIOR_STOP_ON_CLEAR" => Some(2),
                "END_BEHAVIOR_FADE_TO_BLACK" => Some(3),
                "END_BEHAVIOR_FADE_TO_CLEAR" => Some(4),
                value => {
                    return Err(format!(
                        "Media action endBehavior {value:?} is not exported."
                    ))
                }
            };
            if let Some(end) = end {
                put_varint(2, end, &mut video);
            }
            if let Some(value) = loop_time {
                put_f64(3, value, &mut video);
            }
            if let Some(value) = times_to_loop {
                put_varint(4, value, &mut video);
            }
            if details["softLoop"] == true {
                put_varint(5, 1, &mut video);
            }
            if let Some(value) = details["softLoopDuration"]
                .as_f64()
                .filter(|value| *value > 0.0)
            {
                put_f64(6, value, &mut video);
            }
            put_message(7, video, &mut media);
        }
        _ => {
            let mut audio = Vec::new();
            put_varint(1, playback, &mut audio);
            if let Some(value) = loop_time {
                put_f64(2, value, &mut audio);
            }
            if let Some(value) = times_to_loop {
                put_varint(3, value, &mut audio);
            }
            put_message(8, audio, &mut media);
        }
    }
    encode_media_markers(&item, &mut media)?;

    let mut encoded = Vec::new();
    put_uuid(1, id, &mut encoded);
    if let Some(title) = action["title"].as_str() {
        put_string(2, title, &mut encoded);
    }
    let empty = Map::new();
    super::marker_action_export::encode_label(details.as_object().unwrap_or(&empty), &mut encoded)?;
    if let Some(delay) = action["delaySeconds"].as_f64().filter(|value| *value > 0.0) {
        put_f64(4, delay, &mut encoded);
    }
    put_varint(6, u64::from(action["enabled"] != false), &mut encoded);
    if let Some(duration) = action["durationSeconds"]
        .as_f64()
        .filter(|value| *value > 0.0)
    {
        put_f64(8, duration, &mut encoded);
    }
    put_varint(9, 2, &mut encoded); // ACTION_TYPE_MEDIA
    put_message(20, media, &mut encoded);
    Ok(encoded)
}

fn encode_media_markers(media: &Value, action_media: &mut Vec<u8>) -> Result<(), String> {
    let Some(markers) = media.get("markers") else {
        return Ok(());
    };
    let markers = markers
        .as_array()
        .ok_or("Media markers must be an array.")?;
    for marker in markers {
        let id = required_string(marker, "id", "Media marker id is required.")?;
        let title = required_string(marker, "title", "Media marker title is required.")?;
        let time = marker
            .get("timeSeconds")
            .and_then(Value::as_f64)
            .filter(|time| time.is_finite() && *time >= 0.0)
            .ok_or("Media marker timeSeconds must be finite and non-negative.")?;
        let actions = marker
            .get("actions")
            .and_then(Value::as_array)
            .ok_or("Media marker actions must be an array.")?;
        if marker
            .get("repeatOnLoop")
            .is_some_and(|repeat| !repeat.is_boolean())
        {
            return Err("Media marker repeatOnLoop must be a boolean.".into());
        }
        let mut encoded = Vec::new();
        put_uuid(1, id, &mut encoded);
        put_f64(2, time, &mut encoded);
        if let Some(color) = marker.get("color") {
            let color = color
                .as_str()
                .filter(|color| {
                    color.len() == 7
                        && color.starts_with('#')
                        && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .ok_or("Media marker color must be a six-digit hex color.")?;
            put_color(3, color, &mut encoded);
        }
        put_string(4, title, &mut encoded);
        for (action_index, action) in actions.iter().enumerate() {
            match super::marker_action_export::encode_marker_action(action) {
                Ok(action) => put_message(5, action, &mut encoded),
                Err(super::marker_action_export::MarkerActionExportError::UnsupportedVariant(
                    _,
                )) => {
                    // `media_losses` reports this exact action; keep the other marker actions.
                }
                Err(super::marker_action_export::MarkerActionExportError::Invalid(error)) => {
                    return Err(format!(
                        "Media marker {title:?} action {action_index} cannot be encoded: {error}"
                    ));
                }
            }
        }
        put_message(12, encoded, action_media);
    }
    Ok(())
}

fn encode_type_properties(media: &Value, kind: &str) -> Result<(u32, Vec<u8>), String> {
    let style = style_value(media, kind);
    let (field, base) = layer_media_export::type_properties(&style, "media")
        .ok_or_else(|| format!("Media item has unsupported type {kind:?}."))?;
    let base_fields = decode_message(&base)?;
    let mut properties = Vec::new();
    match kind {
        "image" => {
            if let Some(drawing) = nested_field(&base_fields, 1) {
                put_message(1, encode_fields(&drawing), &mut properties);
            }
            Ok((field, properties))
        }
        "video" => {
            if let Some(drawing) = nested_field(&base_fields, 1) {
                put_message(1, encode_fields(&drawing), &mut properties);
            }
            if let Some(routing) = media.get("audioRouting") {
                let (audio, _) = encode_audio_properties(routing)?;
                if !audio.is_empty() {
                    put_message(2, audio, &mut properties);
                }
            } else if let Some(audio) = nested_field(&base_fields, 2) {
                put_message(2, encode_fields(&audio), &mut properties);
            }
            let transport =
                encode_transport(nested_field(&base_fields, 3), media.get("mediaTransport"))?;
            if !transport.is_empty() {
                put_message(3, transport, &mut properties);
            }
            let mut video = Vec::new();
            // ProPresenter writes the thumbnail time in seconds and -1 when it is unset.
            let thumbnail = match media.get("thumbnailTimeSeconds") {
                None | Some(Value::Null) => -1.0,
                Some(value) => value
                    .as_f64()
                    .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
                    .ok_or("Media thumbnailTimeSeconds must be finite and non-negative.")?,
            };
            put_f64(3, thumbnail, &mut video);
            if let Some(end_behavior) = media.get("backgroundEndBehavior").and_then(Value::as_str) {
                put_varint(4, end_behavior_enum(end_behavior)?, &mut video);
            }
            put_message(4, video, &mut properties);
            Ok((field, properties))
        }
        "audio" => {
            let mut audio_properties = None;
            if let Some(routing) = media.get("audioRouting") {
                audio_properties = Some(encode_audio_properties(routing)?.0);
            } else if let Some(audio) = nested_field(&base_fields, 1) {
                audio_properties = Some(encode_fields(&audio));
            }
            if let Some(audio) = audio_properties.filter(|value| !value.is_empty()) {
                put_message(1, audio, &mut properties);
            }
            if let Some(transport) = nested_field(&base_fields, 2) {
                put_message(2, encode_fields(&transport), &mut properties);
            }
            Ok((field, properties))
        }
        _ => Err(format!("Media item has unsupported type {kind:?}.")),
    }
}

fn encode_transport(base: Option<Vec<Field>>, value: Option<&Value>) -> Result<Vec<u8>, String> {
    let mut transport = base.as_deref().map(encode_fields).unwrap_or_default();
    let Some(value) = value else {
        return Ok(transport);
    };
    crate::output_playback::validate_media_transport(value)?;
    let rate = value
        .get("rate")
        .and_then(Value::as_f64)
        .ok_or("Media transport rate must be finite.")?;
    let start = value
        .get("inSeconds")
        .and_then(Value::as_f64)
        .ok_or("Media transport inSeconds must be finite.")?;
    put_f64(1, rate, &mut transport);
    put_f64(2, start, &mut transport);
    if let Some(end) = value.get("outSeconds").and_then(Value::as_f64) {
        put_f64(3, end, &mut transport);
    }
    let retrigger = match value.get("retrigger").and_then(Value::as_str) {
        Some("restart") => 1,
        Some("continue") => 2,
        Some("automatic") => 3,
        _ => return Err("Media transport retrigger is invalid.".into()),
    };
    put_varint(15, retrigger, &mut transport);
    Ok(transport)
}

fn encode_audio_properties(value: &Value) -> Result<(Vec<u8>, Vec<String>), String> {
    crate::native_audio::validate_audio_routing(value)?;
    let object = value
        .as_object()
        .ok_or("Audio routing must be an object.")?;
    let gain = object
        .get("gainDb")
        .and_then(Value::as_f64)
        .ok_or("Audio routing gainDb is required.")?;
    let mut properties = Vec::new();
    put_f64(1, 10.0_f64.powf(gain / 20.0), &mut properties);
    let channels = object
        .get("channels")
        .and_then(Value::as_array)
        .ok_or("Audio routing channels are required.")?;
    let has_solo = channels
        .iter()
        .any(|channel| channel.get("solo").and_then(Value::as_bool) == Some(true));
    let mut losses = Vec::new();
    if has_solo {
        losses.push("audioRouting solo state is represented by equivalent mute states".into());
    }
    for channel in channels {
        let source = channel
            .get("sourceChannel")
            .and_then(Value::as_u64)
            .ok_or("Audio routing sourceChannel is required.")?;
        let solo = channel.get("solo").and_then(Value::as_bool) == Some(true);
        let muted =
            channel.get("muted").and_then(Value::as_bool) == Some(true) || (has_solo && !solo);
        let mut encoded = Vec::new();
        if source > 1 {
            put_varint(1, source - 1, &mut encoded);
        }
        if muted {
            put_varint(2, 1, &mut encoded);
        }
        if let Some(gain_db) = channel
            .get("gainDb")
            .and_then(Value::as_f64)
            .filter(|gain_db| gain_db.abs() > f64::EPSILON)
        {
            put_f64(3, gain_db, &mut encoded);
        }
        if channel.get("compressLimit").and_then(Value::as_bool) == Some(true) {
            put_varint(4, 1, &mut encoded);
        }
        if let Some(outputs) = channel.get("outputs").and_then(Value::as_array) {
            for output in outputs.iter().filter_map(Value::as_u64) {
                let mut encoded_output = Vec::new();
                if output > 1 {
                    put_varint(2, output - 1, &mut encoded_output);
                }
                put_message(5, encoded_output, &mut encoded);
            }
        }
        put_message(2, encoded, &mut properties);
    }
    let default_mapping = !has_solo
        && channels.len() == 2
        && channels.iter().enumerate().all(|(index, channel)| {
            channel.get("sourceChannel").and_then(Value::as_u64) == Some(index as u64 + 1)
                && channel
                    .get("outputs")
                    .and_then(Value::as_array)
                    .is_some_and(|outputs| {
                        outputs.len() == 1 && outputs[0].as_u64() == Some(index as u64 + 1)
                    })
                && channel.get("muted").and_then(Value::as_bool) == Some(false)
                && channel.get("compressLimit").and_then(Value::as_bool) != Some(true)
                && channel
                    .get("gainDb")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    .abs()
                    <= f64::EPSILON
        });
    if !default_mapping {
        put_varint(3, 1, &mut properties);
    }
    Ok((properties, losses))
}

fn media_url(
    media: &Value,
    item_file: &str,
    relative: &BTreeMap<String, String>,
) -> Result<(String, String), String> {
    let absolute = required_string(media, "sourceUrl", "Media source URL is required.")?;
    let exported_path = relative
        .get(absolute)
        .ok_or_else(|| format!("Media item has no exported relative path for {item_file:?}."))?;
    let exported_path = exported_path.replace('\\', "/");
    if !exported_path.starts_with("Media/")
        || exported_path
            .split('/')
            .any(|part| part == ".." || part.is_empty())
    {
        return Err(format!(
            "Media item export path {exported_path:?} must remain under Media/."
        ));
    }
    let parsed = url::Url::parse(&absolute)
        .map_err(|error| format!("Media item URL is invalid: {error}."))?;
    if parsed.scheme() != "file" || parsed.to_file_path().is_err() {
        return Err(format!(
            "Media item URL must be a local file URL: {absolute:?}."
        ));
    }
    Ok((absolute.to_string(), exported_path))
}

fn encode_local_media_url(absolute: &str, exported_path: &str) -> Vec<u8> {
    let paths = BTreeMap::from([(absolute.to_string(), exported_path.to_string())]);
    encode_playlist_url(absolute, &paths)
}

fn encode_fields(fields: &[Field]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for field in fields {
        match &field.value {
            WireValue::Varint(value) => put_varint(field.number, *value, &mut bytes),
            WireValue::Fixed64(value) => put_f64(field.number, f64::from_bits(*value), &mut bytes),
            WireValue::Bytes(value) => put_bytes(field.number, value, &mut bytes),
            WireValue::Fixed32(value) => put_f32(field.number, f32::from_bits(*value), &mut bytes),
        }
    }
    bytes
}

fn style_value(media: &Value, kind: &str) -> Value {
    let mut style = Map::new();
    style.insert("mediaType".into(), json!(kind));
    for key in [
        "mediaEffects",
        "mediaHue",
        "mediaSaturation",
        "mediaBrightness",
        "mediaContrast",
        "mediaScale",
        "mediaAlignment",
        "mediaFlipX",
        "mediaFlipY",
        "mediaRotation",
        "mediaCrop",
        "mediaCropLeft",
        "mediaCropRight",
        "mediaCropTop",
        "mediaCropBottom",
        "mediaAlphaType",
    ] {
        if let Some(value) = media.get(key) {
            style.insert(key.into(), value.clone());
        }
    }
    if let Some(scale) = media.get("backgroundScale").and_then(Value::as_str) {
        let scale = match scale {
            "fit" => "Scale to Fit",
            "fill" => "Scale to Fill",
            "stretch" => "Stretch to Fill",
            _ => scale,
        };
        style.insert("mediaScale".into(), json!(scale));
    }
    match kind {
        "video" => {
            if let Some(playback) = media.get("backgroundPlayback").and_then(Value::as_str) {
                style.insert(
                    "mediaPlayback".into(),
                    json!(if playback == "loop" { "Loop" } else { "Stop" }),
                );
            }
        }
        "audio" => {
            if let Some(playback) = media.get("audioPlayback").and_then(Value::as_str) {
                style.insert(
                    "mediaPlayback".into(),
                    json!(if playback == "loop" { "Loop" } else { "Stop" }),
                );
            }
        }
        _ => {}
    }
    Value::Object(style)
}

fn media_losses(media: &Value, kind: &str, findings: &mut BTreeSet<String>) {
    let label = media_label(media);
    if kind == "video"
        && media.get("thumbnailTimeSeconds").is_some_and(|value| {
            !value.is_null()
                && value
                    .as_f64()
                    .is_none_or(|seconds| !seconds.is_finite() || seconds < 0.0)
        })
    {
        findings.insert(format!(
            "{label} thumbnailTimeSeconds must be finite and non-negative."
        ));
    }
    for marker in media
        .get("markers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let title = marker
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("unnamed marker");
        for (action_index, action) in marker
            .get("actions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let reference = format!("{label} playback marker {title:?} action {action_index}");
            match super::marker_action_export::encode_marker_action(action) {
                Ok(_) => {}
                Err(super::marker_action_export::MarkerActionExportError::UnsupportedVariant(
                    reason,
                )) => {
                    findings.insert(format!("{reference}: {reason}"));
                }
                Err(super::marker_action_export::MarkerActionExportError::Invalid(reason)) => {
                    findings.insert(format!("{reference} cannot be encoded: {reason}"));
                }
            }
        }
        if marker.get("repeatOnLoop").and_then(Value::as_bool) == Some(true)
            || (kind == "video" && marker.get("repeatOnLoop").is_none())
        {
            findings.insert(format!(
                "{label} playback marker {title:?} repeatOnLoop is not represented in ProPresenter."
            ));
        }
    }
    if media.get("mediaDesaturate").and_then(Value::as_bool) == Some(true) {
        findings.insert(format!(
            "{label} mediaDesaturate is not represented in ProPresenter."
        ));
    }
    if media
        .get("backgroundTransition")
        .and_then(Value::as_str)
        .is_some_and(|value| crate::native_transitions::media_transition(value).is_err())
    {
        findings.insert(format!(
            "{label} backgroundTransition is not a ProPresenter library transition."
        ));
    }
    if media
        .get("backgroundScale")
        .and_then(Value::as_str)
        .is_some_and(|value| !matches!(value, "fit" | "fill" | "stretch"))
    {
        findings.insert(format!(
            "{label} backgroundScale is not represented in ProPresenter."
        ));
    }
    if kind != "video"
        && media
            .get("backgroundEndBehavior")
            .and_then(Value::as_str)
            .is_some_and(|value| value != "hold")
    {
        findings.insert(format!(
            "{label} backgroundEndBehavior has no {kind} schema field."
        ));
    }
    if kind != "video"
        && media
            .get("backgroundPlayback")
            .and_then(Value::as_str)
            .is_some_and(|value| value != "stop")
    {
        findings.insert(format!(
            "{label} backgroundPlayback has no {kind} schema field."
        ));
    }
    if kind != "audio" && media.get("audioPlayback").is_some() {
        findings.insert(format!("{label} audioPlayback has no {kind} schema field."));
    }
    if let Some(routing) = media.get("audioRouting") {
        if let Ok((_, route_losses)) = encode_audio_properties(routing) {
            for loss in route_losses {
                findings.insert(format!("{label} {loss}."));
            }
        }
    }
}

fn media_category(media: &Value) -> Option<&'static str> {
    match media.get("type").and_then(Value::as_str)? {
        "image" | "video" => Some(GRAPHICS),
        "audio" => Some(AUDIO),
        _ => None,
    }
}

fn smart_source_url(library: &Value) -> Result<String, String> {
    let source = required_string(
        library,
        "sourcePath",
        "Smart media source folder is required.",
    )?;
    if source.starts_with("file://") {
        let url = url::Url::parse(source).map_err(|error| error.to_string())?;
        url.to_file_path()
            .map_err(|_| "Smart media source folder must be local.".to_string())?;
        Ok(url.to_string())
    } else {
        url::Url::from_directory_path(Path::new(source))
            .map(|url| url.to_string())
            .map_err(|_| "Smart media source folder must be an absolute local path.".to_string())
    }
}

fn parent_id(library: &Value) -> Result<&str, String> {
    match library.get("parentId") {
        None => Ok(""),
        Some(value) => value
            .as_str()
            .ok_or_else(|| "Media library parentId must be a string.".into()),
    }
}

fn required_string<'a>(value: &'a Value, key: &str, error: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| error.to_string())
}

fn layer_type(value: &str) -> Result<u64, String> {
    match value {
        "background" => Ok(0),
        "foreground" => Ok(1),
        _ => Err(format!("Unsupported media backgroundBehavior {value:?}.")),
    }
}

fn playback_enum(value: &str) -> Result<u64, String> {
    match value {
        "stop" => Ok(0),
        "loop" => Ok(1),
        _ => Err(format!("Unsupported media playback setting {value:?}.")),
    }
}

fn end_behavior_enum(value: &str) -> Result<u64, String> {
    match value {
        "hold" => Ok(0),
        "black" => Ok(1),
        "clear" => Ok(2),
        "fade-black" => Ok(3),
        "fade-clear" => Ok(4),
        _ => Err(format!("Unsupported media end behavior {value:?}.")),
    }
}

fn label(value: &Value) -> String {
    value
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| value.get("id").and_then(Value::as_str))
        .unwrap_or("Media library")
        .to_string()
}

fn media_label(value: &Value) -> String {
    value
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| value.get("id").and_then(Value::as_str))
        .unwrap_or("Media item")
        .to_string()
}
