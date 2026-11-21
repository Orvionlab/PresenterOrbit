use serde_json::{json, Value};

pub(crate) fn catalog(workspace: &Value) -> Result<Value, String> {
    let assets = workspace["media"]
        .as_array()
        .ok_or("Workspace media must be an array.")?;
    let libraries = workspace["mediaLibraries"]
        .as_array()
        .ok_or("Workspace mediaLibraries must be an array.")?;
    let mut bins = vec![
        json!({"id":"","title":"Unfiled media","items":assets.iter().filter(|asset|asset.get("libraryId").is_none_or(|id| id.as_str()==Some(""))).map(media_summary).collect::<Vec<_>>()}),
    ];
    for library in libraries {
        let id = library["id"]
            .as_str()
            .ok_or("Media library identity is required.")?;
        let files = library.get("itemFiles").and_then(Value::as_array);
        let mut items: Vec<&Value> = assets
            .iter()
            .filter(|asset| {
                asset["libraryId"].as_str() == Some(id)
                    || files.is_some_and(|files| {
                        asset
                            .get("assetFile")
                            .is_some_and(|file| files.contains(file))
                    })
            })
            .collect();
        if library["type"] == "playlist" && library["mediaType"] == "audio" {
            order_audio_playlist_items(&mut items, library);
        } else if let Some(order) = library
            .get("graphicsOrder")
            .and_then(Value::as_array)
            .filter(|_| library["mediaType"] != "audio")
        {
            items.sort_by_key(|asset| {
                order
                    .iter()
                    .position(|id| id == &asset["id"])
                    .unwrap_or(order.len())
            });
        }
        bins.push(json!({"id":id,"title":library["title"],"audioPlaylist":library["type"]=="playlist" && library["mediaType"]=="audio","items":items.into_iter().map(media_summary).collect::<Vec<_>>()}));
    }
    Ok(json!(bins))
}

fn media_summary(asset: &Value) -> Value {
    json!({"id":asset["id"],"title":asset["title"],"type":asset["type"]})
}

fn order_audio_playlist_items(items: &mut [&Value], library: &Value) {
    if let Some(files) = library["itemFiles"].as_array() {
        items.sort_by_key(|asset| {
            files
                .iter()
                .position(|file| Some(file) == asset.get("assetFile"))
                .unwrap_or(files.len())
        });
    }
}

pub(crate) fn transport(
    snapshot: &Value,
    command: &str,
    payload: &Value,
) -> Option<(crate::output_playback::PlaybackLayer, bool)> {
    if command != "media" {
        return None;
    }
    let paused = match payload["action"].as_str()? {
        "play" => false,
        "pause" => true,
        _ => return None,
    };
    let audio = snapshot["audioId"] == payload["id"];
    Some((
        if audio {
            crate::output_playback::PlaybackLayer::Music
        } else {
            crate::output_playback::PlaybackLayer::Media
        },
        paused,
    ))
}

pub(crate) fn mutate(
    snapshot: &Value,
    workspace: &Value,
    payload: &Value,
) -> Result<(Value, Value), String> {
    let id = payload["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Media identity is required.")?;
    let asset = workspace["media"]
        .as_array()
        .and_then(|assets| assets.iter().find(|asset| asset["id"].as_str() == Some(id)))
        .ok_or("Media is unavailable in the active workspace.")?;
    let audio = match asset["type"].as_str() {
        Some("audio") => true,
        Some("image" | "video") => false,
        _ => return Err("Unsupported media type.".into()),
    };
    if !snapshot["media"].as_array().is_some_and(|assets| {
        assets
            .iter()
            .any(|accepted| accepted["id"] == asset["id"] && accepted["type"] == asset["type"])
    }) {
        return Err("Media is unavailable in the accepted output snapshot.".into());
    }
    match payload["action"].as_str() {
        Some("trigger") => {
            let mut accepted = snapshot.clone();
            let assets = accepted["media"]
                .as_array_mut()
                .ok_or("Output media must be an array.")?;
            for candidate in assets
                .iter_mut()
                .filter(|candidate| candidate["id"] == asset["id"])
            {
                *candidate = asset.clone();
            }
            // Match selectMediaBackground: asset override, current option, then its declared initial option.
            let option = |key: &str, fallback: &str| {
                asset
                    .get(key)
                    .and_then(Value::as_str)
                    .or_else(|| snapshot.get(key).and_then(Value::as_str))
                    .unwrap_or(fallback)
                    .to_string()
            };
            let action = crate::NativeDueMediaAction {
                target_id: id.into(),
                playback: option("backgroundPlayback", "stop"),
                end_behavior: option("backgroundEndBehavior", "hold"),
                transition: option("backgroundTransition", "none"),
                behavior: asset
                    .get("backgroundBehavior")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                scale: asset
                    .get("backgroundScale")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                // Bin selection uses the asset's own effect chain.
                effects: None,
                transport: None,
            };
            let (next, mut patch) = if audio {
                crate::select_audio_in_output_snapshot(&accepted, &action)
            } else {
                crate::select_media_in_output_snapshot(&accepted, &action)
            }?
            .ok_or("Media selection is unavailable.")?;
            patch["media"] = next["media"].clone();
            Ok((next, patch))
        }
        Some("previous" | "next") => {
            validate_occurrence(snapshot, payload, audio)?;
            if !audio {
                return Err("Playlist navigation requires audio.".into());
            }
            let target = playlist_neighbor(workspace, payload)?;
            let mut trigger = payload.clone();
            trigger["id"] = target;
            trigger["action"] = json!("trigger");
            mutate(snapshot, workspace, &trigger)
        }
        Some("play" | "pause") => {
            if !audio && asset["type"] != "video" {
                return Err("Still images have no playback transport.".into());
            }
            validate_occurrence(snapshot, payload, audio)?;
            let layer = if audio {
                crate::output_playback::PlaybackLayer::Music
            } else {
                crate::output_playback::PlaybackLayer::Media
            };
            let (next, playback) = crate::set_transport_paused_in_output_snapshot(
                snapshot,
                layer,
                payload["action"] == "pause",
            )?;
            Ok((next, json!({"transportPlaybackPaused":playback})))
        }
        Some("stop") => {
            validate_occurrence(snapshot, payload, audio)?;
            let mut next = snapshot.clone();
            let mut patch = json!({});
            if audio {
                next["audioId"] = Value::Null;
                next["selectedAudio"] = Value::Null;
                patch["audioId"] = Value::Null;
                patch["selectedAudio"] = Value::Null;
            } else {
                let selection_id = snapshot
                    .get("backgroundSelectionId")
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .unwrap_or(0.0)
                    + 1.0;
                next["backgroundId"] = json!("");
                next["backgroundSelectionId"] = json!(selection_id);
                next["backgroundPlaybackState"] = Value::Null;
                let playback = next
                    .get_mut("transportPlaybackPaused")
                    .and_then(Value::as_object_mut)
                    .ok_or("Transport playback state must be an object.")?;
                playback.insert("media".into(), Value::Bool(false));
                patch["backgroundId"] = json!("");
                patch["backgroundSelectionId"] = json!(selection_id);
                patch["backgroundPlaybackState"] = Value::Null;
                patch["transportPlaybackPaused"] = Value::Object(playback.clone());
            }
            Ok((next, patch))
        }
        _ => Err("Unsupported media action.".into()),
    }
}

fn validate_occurrence(snapshot: &Value, payload: &Value, audio: bool) -> Result<(), String> {
    let (target, selection) = if audio {
        ("audioId", "audioSelectionId")
    } else {
        ("backgroundId", "backgroundSelectionId")
    };
    if snapshot[target] != payload["id"]
        || !payload["selectionId"]
            .as_f64()
            .is_some_and(|id| id.is_finite() && id >= 0.0)
        || snapshot[selection].as_f64() != payload["selectionId"].as_f64()
    {
        return Err("The media playback occurrence is no longer active.".into());
    }
    Ok(())
}

pub(crate) fn clock_command(
    snapshot: &Value,
    workspace: &Value,
    payload: &Value,
) -> Result<crate::output_playback::PlaybackCommand, String> {
    let asset = workspace["media"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == payload["id"]))
        .ok_or("Media is unavailable in the active workspace.")?;
    let audio = match asset["type"].as_str() {
        Some("audio") => true,
        Some("video") => false,
        _ => return Err("Still images have no playback transport.".into()),
    };
    if !snapshot["media"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["id"] == asset["id"] && item["type"] == asset["type"])
    }) {
        return Err("Media is unavailable in the accepted output snapshot.".into());
    }
    validate_occurrence(snapshot, payload, audio)?;
    let position = match payload["action"].as_str() {
        Some("seek") => payload["position"]
            .as_f64()
            .filter(|position| position.is_finite() && (0.0..=1.0).contains(position))
            .ok_or("Seek position must be between 0 and 1.")?,
        Some("inPoint" | "restart") => 0.0,
        Some("outPoint") if !audio => 1.0,
        _ => return Err("Unsupported playback clock action.".into()),
    };
    Ok(crate::output_playback::PlaybackCommand {
        layer: if audio {
            crate::output_playback::PlaybackLayer::Music
        } else {
            crate::output_playback::PlaybackLayer::Media
        },
        action: crate::output_playback::PlaybackAction::Seek,
        position,
    })
}

fn playlist_neighbor(workspace: &Value, payload: &Value) -> Result<Value, String> {
    let library = workspace["mediaLibraries"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|item| {
                item["id"] == payload["playlistId"]
                    && item["type"] == "playlist"
                    && item["mediaType"] == "audio"
            })
        })
        .ok_or("An authored audio playlist is required.")?;
    let mut items: Vec<&Value> = workspace["media"]
        .as_array()
        .ok_or("Workspace media must be an array.")?
        .iter()
        .filter(|item| {
            item["type"] == "audio"
                && (item["libraryId"] == library["id"]
                    || library["itemFiles"].as_array().is_some_and(|files| {
                        item.get("assetFile")
                            .is_some_and(|file| files.contains(file))
                    }))
        })
        .collect();
    order_audio_playlist_items(&mut items, library);
    let tracks: Vec<Value> = items.iter().map(|item| item["id"].clone()).collect();
    if payload["playlistOrder"] != json!(tracks) {
        return Err("The audio playlist order has changed. Refresh the remote.".into());
    }
    let index = tracks
        .iter()
        .position(|id| id == &payload["id"])
        .ok_or("The active audio is not in this playlist.")?;
    let neighbor = if payload["action"] == "previous" {
        index.checked_sub(1)
    } else {
        index.checked_add(1)
    };
    neighbor
        .and_then(|index| tracks.get(index))
        .cloned()
        .ok_or("The audio playlist boundary has been reached.".into())
}
