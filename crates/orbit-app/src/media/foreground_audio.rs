//! `foreground-audio-runtime.ts`: the "Foreground Media Behavior - Audio Playback" settings. Foreground media on
//! the presentation or announcements layer clears or pauses the audio layer, and paused audio resumes once that
//! media is cleared or ends. The behaviors are `ignore`, `clear`, `clear-if-audio`, `pause`, and
//! `pause-if-audio`; the `-if-audio` ones act only for a video that has an audio track.

use gpui::{AppContext, Context, Entity, Subscription};
use serde_json::{json, Value};

use crate::state::runtime::Runtime;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Source {
    Presentation,
    Announcements,
}

impl Source {
    const ALL: [Self; 2] = [Self::Presentation, Self::Announcements];

    fn index(self) -> usize {
        self as usize
    }

    fn setting(self) -> &'static str {
        match self {
            Self::Presentation => "foregroundAudioPresentation",
            Self::Announcements => "foregroundAudioAnnouncements",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForegroundMedia {
    pub key: String,
    pub kind: String,
    pub path: String,
    pub volume: f64,
}

/// `presentationMedia`: the background that is on air as foreground media and has not ended.
pub fn presentation_media(live: &Value, media: &Value) -> Option<ForegroundMedia> {
    let id = live["backgroundId"].as_str().filter(|id| !id.is_empty())?;
    if live["backgroundBehavior"] != "foreground" {
        return None;
    }
    let playback = &live["backgroundPlaybackState"];
    if playback["status"] == "ended" && playback["selectionId"] == live["backgroundSelectionId"] {
        return None;
    }
    let item = media.as_array()?.iter().find(|item| item["id"] == id)?;
    let kind = item["type"].as_str().filter(|kind| !kind.is_empty())?;
    if kind == "audio" {
        return None;
    }
    Some(ForegroundMedia {
        key: format!("{id}:{}", live["backgroundSelectionId"]),
        kind: kind.to_string(),
        path: item["sourcePath"].as_str().unwrap_or("").to_string(),
        volume: 1.,
    })
}

/// `announcementsMedia`: the first visible video or image layer of the announcements slide.
pub fn announcements_media(live: &Value) -> Option<ForegroundMedia> {
    let player = &live["announcements"];
    let index = player["player"]["index"].as_u64().unwrap_or(0) as usize;
    let layers = player["item"]["slides"][index]["layers"].as_array()?;
    for layer in layers.iter().filter(|layer| layer["hidden"] != true) {
        let fill = matches!(layer["mediaFillType"].as_str(), Some("video" | "image"));
        let (kind, path, volume) = if fill {
            (
                layer["mediaFillType"].as_str(),
                layer["mediaFillSourcePath"].as_str(),
                &layer["mediaFillVolume"],
            )
        } else {
            (
                layer["mediaType"].as_str(),
                layer["mediaUrl"].as_str(),
                &layer["mediaVolume"],
            )
        };
        if let (Some(kind @ ("video" | "image")), Some(path)) = (kind, path) {
            if path.is_empty() {
                continue;
            }
            return Some(ForegroundMedia {
                key: format!(
                    "{}:{}",
                    player["selectionId"].as_str().unwrap_or(""),
                    layer["id"].as_str().unwrap_or("")
                ),
                kind: kind.to_string(),
                path: path.to_string(),
                volume: volume.as_f64().unwrap_or(1.),
            });
        }
    }
    None
}

/// `audioKey`: the audio selection that is on air, empty when there is none.
pub fn audio_key(live: &Value) -> String {
    match live["audioId"].as_str().filter(|id| !id.is_empty()) {
        Some(id) => format!("{id}:{}", live["audioSelectionId"]),
        None => String::new(),
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Decision {
    Nothing,
    ClearAudio,
    PauseMusic,
}

/// What a behavior does to the audio layer once it is known whether the media has audio.
pub fn decide(behavior: &str, has_audio: bool, music_paused: bool) -> Decision {
    if behavior == "ignore" || (behavior.ends_with("-if-audio") && !has_audio) {
        return Decision::Nothing;
    }
    if behavior.starts_with("clear") {
        return Decision::ClearAudio;
    }
    if music_paused {
        Decision::Nothing
    } else {
        Decision::PauseMusic
    }
}

pub struct ForegroundAudio {
    runtime: Runtime,
    keys: [String; 2],
    /// The audio selection each source paused, so that only its own pause is undone.
    paused: [Option<String>; 2],
    _subscription: Subscription,
}

impl ForegroundAudio {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&runtime.output, |this, _, cx| this.sync(cx));
        Self {
            runtime,
            keys: Default::default(),
            paused: Default::default(),
            _subscription: subscription,
        }
    }

    pub fn sync(&mut self, cx: &mut Context<Self>) {
        let (live, settings) = {
            let output = self.runtime.output.read(cx);
            (output.live.clone(), output.settings.clone())
        };
        let workspace = self.runtime.content.read(cx).workspace.clone();
        for source in Source::ALL {
            let media = match source {
                Source::Presentation => presentation_media(&live, &workspace["media"]),
                Source::Announcements => announcements_media(&live),
            };
            let behavior = settings["generalSettings"][source.setting()]
                .as_str()
                .unwrap_or("ignore")
                .to_string();
            self.update(source, media, behavior, cx);
        }
    }

    fn update(
        &mut self,
        source: Source,
        media: Option<ForegroundMedia>,
        behavior: String,
        cx: &mut Context<Self>,
    ) {
        let key = media.as_ref().map_or("", |media| media.key.as_str());
        if key == self.keys[source.index()] {
            return;
        }
        self.keys[source.index()] = key.to_string();
        self.stop(source, cx);
        if let Some(media) = media {
            self.start(source, media, behavior, cx);
        }
    }

    fn start(
        &mut self,
        source: Source,
        media: ForegroundMedia,
        behavior: String,
        cx: &mut Context<Self>,
    ) {
        let audio = audio_key(&self.runtime.output.read(cx).live);
        if behavior == "ignore" || audio.is_empty() {
            return;
        }
        if !behavior.ends_with("-if-audio") {
            self.apply(source, &media.key, &audio, &behavior, true, cx);
            return;
        }
        if media.kind != "video" || media.volume <= 0. || media.path.is_empty() {
            return;
        }
        let host = self.runtime.host.clone();
        let path = media.path.clone();
        cx.spawn(async move |this, cx| {
            let probed = cx
                .background_spawn(
                    async move { host.invoke("probeMediaMetadata", vec![json!(path)]) },
                )
                .await;
            this.update(cx, |this, cx| match probed {
                Ok(metadata) => this.apply(
                    source,
                    &media.key,
                    &audio,
                    &behavior,
                    metadata["hasAudio"] == json!(true),
                    cx,
                ),
                Err(error) => crate::cues::actions::error(
                    &this.runtime,
                    cx,
                    format!("Foreground media audio behavior failed: {error}"),
                ),
            })
            .ok();
        })
        .detach();
    }

    /// Applies the decision unless the media or the audio changed while the probe ran.
    fn apply(
        &mut self,
        source: Source,
        media_key: &str,
        audio: &str,
        behavior: &str,
        has_audio: bool,
        cx: &mut Context<Self>,
    ) {
        let live = self.runtime.output.read(cx).live.clone();
        if self.keys[source.index()] != media_key || audio_key(&live) != audio {
            return;
        }
        match decide(
            behavior,
            has_audio,
            live["transportPlaybackPaused"]["music"] == true,
        ) {
            Decision::Nothing => {}
            Decision::ClearAudio => self.edit(cx, |live| live["audioId"] = Value::Null),
            Decision::PauseMusic => {
                self.paused[source.index()] = Some(audio.to_string());
                self.edit(cx, |live| {
                    live["transportPlaybackPaused"]["music"] = json!(true)
                });
            }
        }
    }

    /// Resumes the audio this source paused, if it is still the one on air and still paused.
    fn stop(&mut self, source: Source, cx: &mut Context<Self>) {
        let Some(audio) = self.paused[source.index()].take() else {
            return;
        };
        let live = self.runtime.output.read(cx).live.clone();
        if audio == audio_key(&live) && live["transportPlaybackPaused"]["music"] == true {
            self.edit(cx, |live| {
                live["transportPlaybackPaused"]["music"] = json!(false)
            });
        }
    }

    fn edit(&self, cx: &mut Context<Self>, change: impl FnOnce(&mut Value)) {
        self.runtime.output.update(cx, |output, cx| {
            output.edit_live(change);
            cx.notify();
        });
    }
}

/// Starts the behavior for a runtime; the entity lives as long as the application view that holds it.
pub fn attach(runtime: Runtime, cx: &mut gpui::App) -> Entity<ForegroundAudio> {
    cx.new(|cx| ForegroundAudio::new(runtime, cx))
}
