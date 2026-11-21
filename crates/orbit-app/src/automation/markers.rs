//! Media marker scheduling (`scheduleMediaMarkers`). Each marker becomes a delayed action the host scheduler
//! fires when the media position it measures reaches the marker time. The actions carry the identity of the
//! selection they belong to, so the runner drops them when other media is on air by then.

use serde_json::{json, Value};

pub struct Scheduled {
    pub seconds: f64,
    pub playback: Value,
    pub actions: Vec<Value>,
}

/// The media layer and the music layer read different live fields.
pub fn layer_fields(layer: &str) -> (&'static str, &'static str, &'static str) {
    if layer == "music" {
        ("audioId", "audioSelectionId", "audio")
    } else {
        ("backgroundId", "backgroundSelectionId", "video")
    }
}

/// The key that tells a new selection from the same one: workspace, asset, and selection id.
pub fn layer_key(live: &Value, workspace: &str, layer: &str) -> String {
    let (id, selection, _) = layer_fields(layer);
    match live[id].as_str().filter(|id| !id.is_empty()) {
        Some(media) => json!([workspace, media, live[selection]]).to_string(),
        None => String::new(),
    }
}

pub fn schedule(
    live: &Value,
    media: &Value,
    workspace: &str,
    layer: &str,
    markers: &Value,
) -> Result<Vec<Scheduled>, String> {
    let Some(markers) = markers.as_array().filter(|markers| !markers.is_empty()) else {
        return Ok(Vec::new());
    };
    let (id_key, selection_key, kind) = layer_fields(layer);
    let media_id = live[id_key].as_str().unwrap_or("");
    let selection = &live[selection_key];
    let path = media
        .as_array()
        .and_then(|assets| assets.iter().find(|asset| asset["id"] == media_id))
        .filter(|asset| asset["type"] == kind)
        .and_then(|asset| asset["sourcePath"].as_str())
        .filter(|path| !path.is_empty());
    let (Some(path), false) = (path, selection.is_null()) else {
        return Err("Native marker path and selection are required.".into());
    };
    let mut scheduled = Vec::new();
    for marker in markers {
        let (Some(seconds), Some(actions)) = (
            marker["timeSeconds"]
                .as_f64()
                .filter(|seconds| seconds.is_finite()),
            marker["actions"].as_array(),
        ) else {
            continue;
        };
        let id = marker["id"].as_str().filter(|id| !id.is_empty());
        if id.is_none() && actions.is_empty() {
            continue;
        }
        let repeats = marker["repeatOnLoop"] == true
            || (layer == "media" && marker["repeatOnLoop"].is_null());
        let mut playback = json!({"path": path, "selectionId": selection});
        if let Some(id) = id {
            playback["markerId"] = json!(id);
        }
        if layer == "music" {
            playback["layer"] = json!("music");
        }
        if repeats {
            playback["repeatOnLoop"] = json!(true);
        }
        let guarded = actions
            .iter()
            .map(|action| {
                let mut action = action.clone();
                action["details"]["expectedMarkerMediaId"] = json!(media_id);
                action["details"]["expectedMarkerWorkspacePath"] = json!(workspace);
                action["details"]["expectedMarkerSelectionId"] = selection.clone();
                action["details"]["expectedMarkerLayer"] = json!(layer);
                action
            })
            .collect();
        scheduled.push(Scheduled {
            seconds: seconds.max(0.0),
            playback,
            actions: guarded,
        });
    }
    Ok(scheduled)
}
