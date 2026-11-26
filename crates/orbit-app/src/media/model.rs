//! Media selection and playlist progression, ported from media-library-utils and audio-playlist-runtime.
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

pub fn selection<'a>(
    media: &'a [Value],
    libraries: &[Value],
    audio: bool,
    id: &str,
) -> Vec<&'a Value> {
    let library = libraries
        .iter()
        .find(|library| text(library, "id") == id && id != "workspace");
    let files: HashSet<&str> = library
        .and_then(|library| library["itemFiles"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut items: Vec<_> = media
        .iter()
        .filter(|item| {
            let kind = text(item, "type");
            let matches_type = if audio {
                kind == "audio"
            } else {
                kind == "image" || kind == "video"
            };
            matches_type
                && if id.is_empty() {
                    text(item, "libraryId").is_empty()
                } else {
                    text(item, "libraryId") == id || files.contains(text(item, "assetFile"))
                }
        })
        .collect();
    if let Some(library) = library {
        let key = if audio && library["type"] == "playlist" {
            "itemFiles"
        } else if !audio {
            "graphicsOrder"
        } else {
            return items;
        };
        if let Some(order) = library[key].as_array() {
            let order: HashMap<_, _> = order
                .iter()
                .filter_map(Value::as_str)
                .enumerate()
                .map(|(index, id)| (id, index))
                .collect();
            items.sort_by_key(|item| {
                order
                    .get(text(
                        item,
                        if key == "itemFiles" {
                            "assetFile"
                        } else {
                            "id"
                        },
                    ))
                    .copied()
                    .unwrap_or(order.len())
            });
        }
    }
    items
}

/// `resolveAudioLibraryId`: the selected audio playlist, else the seeded Default playlist, else the first one.
pub fn resolve_audio_library(libraries: &[Value], selected: &str) -> String {
    let audio = |library: &&Value| {
        text(library, "id") != "workspace"
            && library["type"] != "folder"
            && (library["mediaType"].is_null() || library["mediaType"] == "audio")
    };
    let find = |id: &str| {
        libraries
            .iter()
            .filter(audio)
            .find(|library| text(library, "id") == id)
    };
    find(selected)
        .or_else(|| find("default-audio"))
        .or_else(|| {
            libraries
                .iter()
                .filter(audio)
                .find(|library| library["type"] == "playlist")
        })
        .map_or_else(String::new, |library| text(library, "id").to_string())
}

/// Media can only be added to a playlist: with none, import, drop and paste are unavailable.
pub fn has_playlist(libraries: &[Value], audio: bool) -> bool {
    libraries.iter().any(|library| {
        text(library, "id") != "workspace"
            && library["type"] == "playlist"
            && (library["mediaType"].is_null()
                || library["mediaType"] == if audio { "audio" } else { "graphics" })
    })
}

/// The selected graphics playlist, otherwise the first available playlist.
pub fn resolve_graphics_library(libraries: &[Value], selected: &str) -> String {
    if selected == super::video_inputs::VIDEO_INPUT_LIBRARY {
        return selected.to_string();
    }
    let graphics = |library: &&Value| {
        library["type"] != "folder"
            && (library["mediaType"].is_null() || library["mediaType"] == "graphics")
    };
    libraries
        .iter()
        .filter(graphics)
        .find(|library| text(library, "id") == selected)
        .or_else(|| {
            libraries
                .iter()
                .filter(graphics)
                .find(|library| library["type"] == "playlist")
        })
        .map_or_else(String::new, |library| text(library, "id").to_string())
}

/// The video of the current selection goes live paused on its first frame, as an Alt-click does with the setting.
pub fn pause_on_first_frame(live: &mut Value) {
    let captured_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    live["backgroundPlaybackState"] = json!({
        "selectionId": live["backgroundSelectionId"],
        "status": "paused",
        "currentTime": 0,
        "capturedAt": captured_at,
    });
    live["transportPlaybackPaused"]["media"] = json!(true);
}

/// `applyPreviewMediaAudioSettings`: the monitor of the inspector plays on its own device, level, and routing;
/// the delay stays the one of the main output.
pub fn inspector_audio_settings(audio: &Value) -> Value {
    let mut settings = audio["inspector"].clone();
    settings["delay"] = audio["main"]["delay"].clone();
    settings
}

/// The library and its sub-libraries: the media `delete_media_library` removes with it.
pub fn library_family(libraries: &[Value], id: &str) -> HashSet<String> {
    let mut family = HashSet::from([id.to_string()]);
    while libraries.iter().any(|library| {
        family.contains(text(library, "parentId")) && family.insert(text(library, "id").to_string())
    }) {}
    family
}

/// Whether the media with this id is filed in the family of libraries.
pub fn in_family(media: &[Value], family: &HashSet<String>, id: &Value) -> bool {
    id.as_str().is_some_and(|id| {
        media
            .iter()
            .any(|item| text(item, "id") == id && family.contains(text(item, "libraryId")))
    })
}

pub fn audio_behavior<'a>(item: &'a Value, libraries: &[Value], library: &str) -> &'a str {
    item["audioPlayback"].as_str().unwrap_or_else(|| {
        if libraries.iter().any(|entry| {
            entry["id"] == library && matches!(text(entry, "audioPlaylistMode"), "next" | "repeat")
        }) {
            "next"
        } else {
            "stop"
        }
    })
}

pub fn select_audio(live: &mut Value, id: &str, library: &str) {
    live["audioId"] = json!(id);
    live["audioSelectionId"] = json!(live["audioSelectionId"].as_u64().unwrap_or(0) + 1);
    live["selectedAudio"] =
        json!({"id":id,"selectionId":live["audioSelectionId"],"playlistId":library});
    live["transportPlaybackPaused"]["music"] = json!(false);
    if let Some(screens) = live["screenConfig"].as_object_mut() {
        for screen in screens.values_mut() {
            if let Some(screen) = screen.as_object_mut() {
                let layers = screen.entry("lookLayers").or_insert_with(|| json!({}));
                if let Some(layers) = layers.as_object_mut() {
                    layers.insert("music".into(), json!(true));
                }
            }
        }
    }
}

/// Merge only authored fields into the latest canonical list; reject a field edited since the form opened.
pub fn merge_media(
    before: &[Value],
    requested: &[Value],
    current: &[Value],
) -> Result<Vec<Value>, String> {
    let mut result = current.to_vec();
    for next in requested {
        let id = text(next, "id");
        if id.is_empty() {
            return Err("Media id is required.".into());
        }
        let original = before.iter().find(|item| item["id"] == id);
        let accepted = result.iter_mut().find(|item| item["id"] == id);
        match (original, accepted) {
            (None, None) => result.push(next.clone()),
            (None, Some(_)) => return Err("Media id was allocated before saving.".into()),
            (Some(_), None) => return Err("Media was removed before saving.".into()),
            (Some(original), Some(accepted)) => {
                let keys: HashSet<_> = original
                    .as_object()
                    .ok_or("Media must be an object.")?
                    .keys()
                    .chain(next.as_object().ok_or("Media must be an object.")?.keys())
                    .collect();
                for key in keys {
                    if next.get(key) == original.get(key) {
                        continue;
                    }
                    if accepted.get(key) != original.get(key) {
                        return Err("Media changed before saving.".into());
                    }
                    let object = accepted.as_object_mut().ok_or("Media must be an object.")?;
                    match next.get(key) {
                        Some(value) => {
                            object.insert(key.clone(), value.clone());
                        }
                        None => {
                            object.remove(key);
                        }
                    }
                }
            }
        }
    }
    for old in before
        .iter()
        .filter(|old| !requested.iter().any(|next| next["id"] == old["id"]))
    {
        if current.iter().find(|item| item["id"] == old["id"]) != Some(old) {
            return Err("Media changed before deletion.".into());
        }
        result.retain(|item| item["id"] != old["id"]);
    }
    let old_order: Vec<_> = before
        .iter()
        .filter(|old| requested.iter().any(|next| next["id"] == old["id"]))
        .map(|item| text(item, "id"))
        .collect();
    let requested_order: Vec<_> = requested
        .iter()
        .filter(|next| before.iter().any(|old| next["id"] == old["id"]))
        .map(|item| text(item, "id"))
        .collect();
    if old_order != requested_order {
        let ordered: Vec<_> = requested_order
            .iter()
            .filter_map(|id| result.iter().find(|item| item["id"] == *id).cloned())
            .collect();
        let mut ordered = ordered.into_iter();
        for item in &mut result {
            if requested_order.contains(&text(item, "id")) {
                if let Some(next) = ordered.next() {
                    *item = next;
                }
            }
        }
    }
    Ok(result)
}

pub fn reorder_media(
    media: &[Value],
    libraries: &[Value],
    library: &str,
    audio: bool,
    dragged: &str,
    target: &str,
    after: bool,
) -> Option<(Vec<Value>, Vec<Value>)> {
    let mut items = selection(media, libraries, audio, library);
    let index = items.iter().position(|item| item["id"] == dragged)?;
    if dragged == target || !items.iter().any(|item| item["id"] == target) {
        return None;
    }
    let dragged = items.remove(index);
    let index = items.iter().position(|item| item["id"] == target)?;
    items.insert(index + usize::from(after), dragged);
    let mut libraries = libraries.to_vec();
    let mut media = media.to_vec();
    if library.is_empty() {
        let ids: HashSet<_> = items.iter().map(|item| text(item, "id")).collect();
        let mut order = items.into_iter();
        for item in &mut media {
            if ids.contains(text(item, "id")) {
                if let Some(next) = order.next() {
                    *item = next.clone();
                }
            }
        }
    } else if let Some(library) = libraries.iter_mut().find(|item| item["id"] == library) {
        let key = if audio { "itemFiles" } else { "graphicsOrder" };
        let field = if audio { "assetFile" } else { "id" };
        library[key] = json!(items
            .iter()
            .map(|item| item[field].as_str())
            .collect::<Option<Vec<_>>>()?);
    }
    Some((media, libraries))
}

#[derive(Default)]
pub struct Playlist {
    active: Option<(String, String)>,
    pending: Option<Value>,
    played: HashSet<String>,
}

impl Playlist {
    pub fn active(&self) -> bool {
        self.active.is_some()
    }
    pub fn follow(&mut self, workspace: &str, library: &str, track: &str) {
        self.active = Some((workspace.into(), library.into()));
        self.pending = None;
        self.played.clear();
        self.played.insert(track.into());
    }
    pub fn stop(&mut self) {
        self.active = None;
        self.pending = None;
        self.played.clear();
    }
    pub fn ended(
        &mut self,
        workspace: &str,
        event: Value,
        media: &[Value],
        libraries: &[Value],
        live: &mut Value,
    ) {
        let Some((path, id)) = self.active.clone() else {
            return;
        };
        if path != workspace
            || event["workspacePath"] != path
            || event["audioId"] != live["audioId"]
            || event["selectionId"] != live["audioSelectionId"]
        {
            return;
        }
        if live["transportPlaybackPaused"]["music"] == true {
            self.pending = Some(event);
            return;
        }
        self.pending = None;
        let Some(library) = libraries.iter().find(|library| {
            library["id"] == id && library["type"] == "playlist" && library["mediaType"] == "audio"
        }) else {
            self.stop();
            return;
        };
        let tracks = selection(media, libraries, true, &id);
        let Some(index) = tracks
            .iter()
            .position(|item| item["id"] == event["audioId"])
        else {
            self.stop();
            return;
        };
        let mode = match text(tracks[index], "audioPlayback") {
            "stop" | "loop" => "stop",
            "next" => "next",
            _ => text(library, "audioPlaylistMode"),
        };
        if mode != "next" && mode != "repeat" {
            self.stop();
            return;
        }
        self.played.insert(text(tracks[index], "id").into());
        let next = if library["audioShuffle"] == true {
            let mut remaining: Vec<_> = tracks
                .iter()
                .copied()
                .filter(|track| !self.played.contains(text(track, "id")))
                .collect();
            if remaining.is_empty() && mode == "repeat" {
                self.played.clear();
                remaining = tracks
                    .iter()
                    .copied()
                    .filter(|track| tracks.len() == 1 || track["id"] != event["audioId"])
                    .collect();
            }
            remaining.into_iter().max_by_key(|_| uuid::Uuid::new_v4())
        } else {
            tracks.get(index + 1).copied().or_else(|| {
                if mode == "repeat" {
                    tracks.first().copied()
                } else {
                    None
                }
            })
        };
        match next {
            Some(next) => {
                self.played.insert(text(next, "id").into());
                select_audio(live, text(next, "id"), &id);
            }
            None => self.stop(),
        }
    }
    pub fn reconcile(
        &mut self,
        workspace: &str,
        media: &[Value],
        libraries: &[Value],
        live: &mut Value,
        countdown: &Value,
    ) {
        if self.active.as_ref().is_some_and(|(path, id)| {
            path != workspace
                || live["audioId"].is_null()
                || !libraries.iter().any(|library| {
                    library["id"] == *id
                        && library["type"] == "playlist"
                        && library["mediaType"] == "audio"
                })
        }) {
            self.stop();
        } else if live["transportPlaybackPaused"]["music"] != true {
            if let Some(event) = self.pending.take() {
                self.ended(workspace, event, media, libraries, live);
            } else if let Some((_, id)) = &self.active {
                let looping = media.iter().any(|item| {
                    item["id"] == live["audioId"] && text(item, "audioPlayback").starts_with("loop")
                });
                if looping {
                    return;
                }
                let fade = libraries
                    .iter()
                    .find(|library| library["id"] == *id)
                    .and_then(|library| library["audioCrossfadeSeconds"].as_f64())
                    .unwrap_or(0.);
                if let (Some(duration), Some(position)) = (
                    countdown["duration"].as_f64(),
                    countdown["currentTime"].as_f64(),
                ) {
                    if fade > 0.
                        && countdown["selectionId"] == live["audioSelectionId"]
                        && countdown["status"] == "playing"
                        && countdown["looping"] != true
                        && duration > 0.
                        && position >= duration - fade.min(duration / 2.)
                    {
                        self.ended(workspace, json!({"workspacePath":workspace,"audioId":live["audioId"],"selectionId":live["audioSelectionId"]}), media, libraries, live);
                    }
                }
            }
        }
    }
}
