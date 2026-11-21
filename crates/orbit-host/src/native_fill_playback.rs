//! Playback rules for video fills on slide objects and text glyphs.
//!
//! A slide's fill videos start when the slide occurrence is shown and follow the
//! object's authored playback: `Loop` repeats, `Hold` keeps the last frame and `Stop`
//! clears the fill at the end. Ramp In fades the fill in from its start; Ramp Out fades
//! a stopping fill out before its end.

use std::path::Path;

use serde_json::{json, Map, Value};

use super::is_video_media_path;
use crate::native_render::NativeRenderRuntime;

const PLAYBACK_MODES: [&str; 3] = ["Loop", "Stop", "Hold"];

pub(super) fn validate(object: &Map<String, Value>) -> Result<(), String> {
    for prefix in ["media", "mediaFill"] {
        let field = format!("{prefix}Playback");
        if let Some(value) = object.get(&field) {
            if !value
                .as_str()
                .is_some_and(|mode| PLAYBACK_MODES.contains(&mode))
            {
                return Err(format!("{field} must be Loop, Stop or Hold."));
            }
        }
        for suffix in ["RampIn", "RampOut"] {
            let field = format!("{prefix}{suffix}");
            if let Some(value) = object.get(&field) {
                if !value
                    .as_f64()
                    .is_some_and(|seconds| seconds.is_finite() && seconds >= 0.0)
                {
                    return Err(format!("{field} must be a finite number of seconds."));
                }
            }
        }
    }
    Ok(())
}

/// The fill clock of one output: a new revision each time another slide occurrence is shown.
pub(super) struct FillClock {
    key: Value,
    revision: u64,
}

impl FillClock {
    pub(super) fn new(key: Value) -> Self {
        Self { key, revision: 1 }
    }

    pub(super) fn observe(&mut self, key: &Value) -> u64 {
        if &self.key != key {
            self.key = key.clone();
            self.revision += 1;
        }
        self.revision
    }
}

/// The content with the slide's untimed fill videos bound to the slide occurrence, or
/// `None` when the slide has none. Explicit playback states, such as frozen stage
/// previews, are kept.
pub(super) fn with_slide_fill_clocks(content: &Value, revision: u64) -> Option<Value> {
    let slide = content.get("presentationThemeSlide")?.as_object()?;
    let untimed = |object: &Map<String, Value>| {
        untimed_fill(object, "media")
            || object
                .get("textStyle")
                .and_then(Value::as_object)
                .is_some_and(|style| untimed_fill(style, "mediaFill"))
    };
    let layers = || {
        slide
            .get("layers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
    };
    if !untimed(slide) && !layers().any(untimed) {
        return None;
    }
    let mut content = content.clone();
    let slide = content["presentationThemeSlide"].as_object_mut()?;
    time_object(slide, revision);
    for layer in slide
        .get_mut("layers")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object_mut)
    {
        time_object(layer, revision);
    }
    Some(content)
}

fn time_object(object: &mut Map<String, Value>, revision: u64) {
    time_fill(object, "media", revision);
    if let Some(style) = object.get_mut("textStyle").and_then(Value::as_object_mut) {
        time_fill(style, "mediaFill", revision);
    }
}

fn time_fill(object: &mut Map<String, Value>, prefix: &str, revision: u64) {
    if !untimed_fill(object, prefix) {
        return;
    }
    let looping = !matches!(
        object
            .get(&format!("{prefix}Playback"))
            .and_then(Value::as_str),
        Some("Stop" | "Hold")
    );
    object.insert(
        format!("{prefix}PlaybackState"),
        json!({ "status": "playing", "currentTime": 0, "clockRevision": revision, "loop": looping }),
    );
}

fn untimed_fill(object: &Map<String, Value>, prefix: &str) -> bool {
    !object.contains_key(&format!("{prefix}PlaybackState")) && is_video_fill(object, prefix)
}

fn is_video_fill(object: &Map<String, Value>, prefix: &str) -> bool {
    let sources: &[&str] = if prefix == "media" {
        &[
            "sourcePath",
            "mediaSourcePath",
            "mediaPath",
            "mediaUrl",
            "sourceUrl",
        ]
    } else {
        &["mediaFillSourcePath", "mediaFillUrl"]
    };
    let Some(source) = sources
        .iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .find(|source| !source.trim().is_empty())
    else {
        return false;
    };
    let video = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("video"))
    };
    if prefix == "media" {
        video("mediaType") || video("type")
    } else {
        video("mediaFillType") || is_video_media_path(source)
    }
}

/// The opacity of a fill video under its playback rules; zero once a stopping fill ended.
/// Fills without a running playback state, such as frozen previews, stay fully visible.
pub(super) fn opacity(
    renderer: &NativeRenderRuntime,
    path: &Path,
    object: &Map<String, Value>,
    prefix: &str,
) -> f64 {
    let playing = object
        .get(&format!("{prefix}PlaybackState"))
        .and_then(|state| state.get("status"))
        .and_then(Value::as_str)
        == Some("playing");
    let Some(progress) = renderer.video_progress(path).filter(|_| playing) else {
        return 1.0;
    };
    let stops = object
        .get(&format!("{prefix}Playback"))
        .and_then(Value::as_str)
        == Some("Stop");
    if stops && progress.ended {
        return 0.0;
    }
    let position = progress.position.unwrap_or(0.0);
    let ramp = |suffix: &str| {
        object
            .get(&format!("{prefix}{suffix}"))
            .and_then(Value::as_f64)
            .filter(|seconds| *seconds > 0.0)
    };
    let fade_in = ramp("RampIn").map_or(1.0, |seconds| position / seconds);
    let fade_out = match (stops, ramp("RampOut"), progress.duration) {
        (true, Some(seconds), Some(duration)) => (duration - position) / seconds,
        _ => 1.0,
    };
    fade_in.min(fade_out).clamp(0.0, 1.0)
}
