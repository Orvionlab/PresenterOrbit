use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

const MEDIA_LAYER: &str = "media";
const MUSIC_LAYER: &str = "music";

pub fn validate_media_transport(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or("Media transport must be an object.")?;
    let number = |key: &str| -> Result<f64, String> {
        object
            .get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("Media transport {key} must be a finite number."))
    };
    let start = number("inSeconds")?;
    let delay = number("delaySeconds")?;
    let rate = number("rate")?;
    if start < 0.0 || delay < 0.0 || !(0.1..=4.0).contains(&rate) {
        return Err(
            "Media transport requires non-negative in/delay and rate between 0.1 and 4."
                .to_string(),
        );
    }
    if object.get("outSeconds") != Some(&Value::Null) && number("outSeconds")? <= start {
        return Err("Media transport out point must be after its in point.".to_string());
    }
    if !matches!(
        object.get("retrigger").and_then(Value::as_str),
        Some("restart" | "continue" | "automatic")
    ) {
        return Err(
            "Media transport retrigger must be restart, continue or automatic.".to_string(),
        );
    }
    Ok(())
}

pub fn validate_media_transport_duration(
    start: f64,
    end: Option<f64>,
    duration: f64,
) -> Result<(), String> {
    if start >= duration || end.is_some_and(|end| end > duration) {
        return Err("Media transport trim points exceed the decoded source duration.".to_string());
    }
    Ok(())
}

const DEFAULT_AUDIO_DEVICE_CHANNELS: i64 = 2;
const MAX_AUDIO_DEVICE_CHANNELS: i64 = 16;
const LOGICAL_AUDIO_CHANNELS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackLayer {
    Media,
    Music,
}

impl PlaybackLayer {
    pub fn from_value(value: Option<&Value>) -> Self {
        if value.and_then(Value::as_str) == Some(MUSIC_LAYER) {
            Self::Music
        } else {
            Self::Media
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Media => MEDIA_LAYER,
            Self::Music => MUSIC_LAYER,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Ended,
}

impl PlaybackStatus {
    fn from_value(value: Option<&Value>, default: Self) -> Self {
        match value.and_then(Value::as_str) {
            Some("paused") => Self::Paused,
            Some("ended") => Self::Ended,
            Some("playing") => Self::Playing,
            _ => default,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Playing => "playing",
            Self::Paused => "paused",
            Self::Ended => "ended",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackAction {
    Toggle,
    Play,
    Pause,
    Forward,
    Backward,
    Seek,
}

impl PlaybackAction {
    fn from_value(value: Option<&Value>) -> Self {
        match value.and_then(Value::as_str) {
            Some("play") => Self::Play,
            Some("pause") => Self::Pause,
            Some("forward") => Self::Forward,
            Some("backward") => Self::Backward,
            Some("seek") => Self::Seek,
            _ => Self::Toggle,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Forward => "forward",
            Self::Backward => "backward",
            Self::Seek => "seek",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaybackCommand {
    pub layer: PlaybackLayer,
    pub action: PlaybackAction,
    pub position: f64,
}

impl PlaybackCommand {
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let command = value
            .as_object()
            .ok_or_else(|| "Playback command must be an object.".to_string())?;
        let position = command
            .get("position")
            .and_then(json_number)
            .filter(|value| value.is_finite())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        Ok(Self {
            layer: PlaybackLayer::from_value(command.get("layer")),
            action: PlaybackAction::from_value(command.get("action")),
            position,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackSnapshot {
    pub selection_id: Option<Value>,
    pub status: PlaybackStatus,
    pub current_time: f64,
    pub duration: Option<f64>,
    pub clock_revision: u64,
    pub loop_iteration: u64,
    pub rate: f64,
    pub shuttle: bool,
    pub deck_stopped: bool,
    pub in_seconds: f64,
    pub delayed: bool,
    pub prepared_start: bool,
    pub out_seconds: Option<f64>,
    pub error: Option<String>,
}

impl PlaybackSnapshot {
    pub fn to_value(&self, captured_at_ms: u64) -> Value {
        let mut value = Map::new();
        if let Some(selection_id) = &self.selection_id {
            value.insert("selectionId".to_string(), selection_id.clone());
        }
        value.insert("status".to_string(), json!(self.status.as_str()));
        value.insert("currentTime".to_string(), json!(self.current_time));
        value.insert(
            "duration".to_string(),
            self.duration
                .map_or(Value::Null, |duration| json!(duration)),
        );
        value.insert("capturedAt".to_string(), json!(captured_at_ms));
        value.insert("clockRevision".to_string(), json!(self.clock_revision));
        value.insert("loopIteration".to_string(), json!(self.loop_iteration));
        value.insert("rate".to_string(), json!(self.rate));
        value.insert("shuttle".to_string(), json!(self.shuttle));
        value.insert("deckStopped".to_string(), json!(self.deck_stopped));
        value.insert("inSeconds".to_string(), json!(self.in_seconds));
        value.insert("delayed".to_string(), json!(self.delayed));
        value.insert("preparedStart".to_string(), json!(self.prepared_start));
        value.insert("outSeconds".to_string(), json!(self.out_seconds));
        value.insert("error".to_string(), json!(self.error));
        Value::Object(value)
    }

    fn to_control_payload(self, command: PlaybackCommand, captured_at_ms: u64) -> Value {
        let mut value = self
            .to_value(captured_at_ms)
            .as_object()
            .cloned()
            .unwrap_or_default();
        value.insert("layer".to_string(), json!(command.layer.as_str()));
        value.insert("action".to_string(), json!(command.action.as_str()));
        value.insert("position".to_string(), json!(command.position));
        Value::Object(value)
    }
}

#[derive(Clone, Debug)]
struct PlaybackTrack {
    identity: String,
    selection_id: Option<Value>,
    duration: Option<f64>,
    position: f64,
    status: PlaybackStatus,
    looping: bool,
    anchor: Instant,
    clock_revision: u64,
    loop_iteration: u64,
    in_seconds: f64,
    out_seconds: Option<f64>,
    rate: f64,
    shuttle: bool,
    deck_stopped: bool,
    delay_remaining: f64,
    prepared_start: bool,
    error: Option<String>,
}

impl PlaybackTrack {
    fn new(
        identity: String,
        selection_id: Option<Value>,
        duration: Option<f64>,
        position: f64,
        status: PlaybackStatus,
        looping: bool,
        now: Instant,
    ) -> Self {
        let duration = normalize_duration(duration);
        let position = clamp_position(position, duration);
        let status = if status == PlaybackStatus::Ended {
            PlaybackStatus::Ended
        } else if duration.is_some_and(|duration| position >= duration) {
            if looping {
                PlaybackStatus::Playing
            } else {
                PlaybackStatus::Ended
            }
        } else {
            status
        };
        Self {
            identity,
            selection_id,
            duration,
            position: if status == PlaybackStatus::Ended {
                duration.unwrap_or(position)
            } else if looping && duration.is_some_and(|duration| position >= duration) {
                0.0
            } else {
                position
            },
            status,
            looping,
            anchor: now,
            clock_revision: 1,
            loop_iteration: 0,
            in_seconds: 0.0,
            out_seconds: None,
            rate: 1.0,
            shuttle: false,
            deck_stopped: false,
            delay_remaining: 0.0,
            prepared_start: position == 0.0,
            error: None,
        }
    }

    fn snapshot_at(&mut self, now: Instant) -> PlaybackSnapshot {
        self.advance_to(now);
        PlaybackSnapshot {
            selection_id: self.selection_id.clone(),
            status: self.status,
            current_time: self.position,
            duration: self.duration,
            clock_revision: self.clock_revision,
            loop_iteration: self.loop_iteration,
            rate: self.rate,
            shuttle: self.shuttle,
            deck_stopped: self.deck_stopped,
            in_seconds: self.in_seconds,
            delayed: self.delay_remaining > 0.0,
            prepared_start: self.prepared_start,
            out_seconds: self.out_seconds,
            error: self.error.clone(),
        }
    }

    fn advance_to(&mut self, now: Instant) {
        if self.status != PlaybackStatus::Playing {
            self.anchor = now;
            return;
        }
        let elapsed = now.saturating_duration_since(self.anchor).as_secs_f64();
        if elapsed <= 0.0 {
            return;
        }
        let playback_elapsed = (elapsed - self.delay_remaining).max(0.0);
        self.delay_remaining = (self.delay_remaining - elapsed).max(0.0);
        let next = self.position + playback_elapsed * self.rate;
        if self.rate < 0.0 && next <= self.in_seconds {
            self.position = self.in_seconds;
            self.status = PlaybackStatus::Paused;
            self.anchor = now;
            return;
        }
        match self.out_seconds.or(self.duration) {
            Some(duration) if self.looping && !self.shuttle && duration > 0.0 => {
                let length = duration - self.in_seconds;
                self.position = self.in_seconds + (next - self.in_seconds) % length;
                if next >= duration {
                    self.prepared_start = false;
                    self.loop_iteration = self
                        .loop_iteration
                        .saturating_add(((next - self.in_seconds) / length).floor() as u64);
                    self.clock_revision = self.clock_revision.saturating_add(1);
                }
                self.anchor = now;
            }
            Some(duration) if next >= duration => {
                self.position = duration;
                self.status = PlaybackStatus::Ended;
                self.anchor = now;
            }
            Some(_) | None => {
                self.position = next;
                self.anchor = now;
            }
        }
    }

    fn apply(&mut self, command: PlaybackCommand, now: Instant) -> bool {
        if self.error.is_some() {
            return false;
        }
        self.advance_to(now);
        let previous = (
            self.position,
            self.status,
            self.rate,
            self.shuttle,
            self.deck_stopped,
        );
        match command.action {
            PlaybackAction::Toggle => {
                if self.status == PlaybackStatus::Playing {
                    self.status = PlaybackStatus::Paused;
                } else {
                    if self.status == PlaybackStatus::Ended {
                        self.position = self.in_seconds;
                        self.prepared_start = true;
                    }
                    self.status = PlaybackStatus::Playing;
                    self.deck_stopped = false;
                    if self.shuttle {
                        self.rate = 1.0;
                        self.shuttle = false;
                    }
                }
            }
            PlaybackAction::Play => {
                self.deck_stopped = false;
                if self.shuttle {
                    self.rate = 1.0;
                    self.shuttle = false;
                }
                if self.status == PlaybackStatus::Ended {
                    self.position = self.in_seconds;
                    self.prepared_start = true;
                }
                self.status = PlaybackStatus::Playing;
            }
            PlaybackAction::Pause => {
                self.deck_stopped = false;
                self.status = PlaybackStatus::Paused;
            }
            PlaybackAction::Seek => {
                self.prepared_start = false;
                if let Some(duration) = self.out_seconds.or(self.duration) {
                    self.delay_remaining = 0.0;
                    self.position =
                        self.in_seconds + (duration - self.in_seconds) * command.position;
                    self.status = if command.position >= 1.0 && !self.looping {
                        PlaybackStatus::Ended
                    } else {
                        self.status
                    };
                }
            }
            PlaybackAction::Forward | PlaybackAction::Backward => {
                self.prepared_start = false;
                if let Some(duration) = self.out_seconds.or(self.duration) {
                    let delta = if command.action == PlaybackAction::Backward {
                        -10.0
                    } else {
                        10.0
                    };
                    self.position =
                        clamp_position(self.position + delta, Some(duration)).max(self.in_seconds);
                    if self.looping && self.position >= duration {
                        self.position = self.in_seconds;
                    } else if !self.looping && self.position >= duration {
                        self.status = PlaybackStatus::Ended;
                    }
                }
            }
        }
        self.anchor = now;
        let changed = previous
            != (
                self.position,
                self.status,
                self.rate,
                self.shuttle,
                self.deck_stopped,
            );
        if changed {
            self.clock_revision = self.clock_revision.saturating_add(1);
        }
        changed
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackCommandOutcome {
    pub payload: Value,
    pub changed: bool,
}

#[derive(Clone, Debug, Default)]
struct SourcePlayback {
    media: Option<PlaybackTrack>,
    music: Option<PlaybackTrack>,
    scroll_visual_key: Option<String>,
    scroll_sequence: u64,
    scroll_content_key: String,
}

impl SourcePlayback {
    fn track_mut(&mut self, layer: PlaybackLayer) -> Option<&mut PlaybackTrack> {
        match layer {
            PlaybackLayer::Media => self.media.as_mut(),
            PlaybackLayer::Music => self.music.as_mut(),
        }
    }
}

/// Rust-owned playback clocks for native output sources.
///
/// Native output audio, video, and NDI consume these canonical snapshots; no
/// renderer playback clock or media-element graph participates in output.
#[derive(Debug, Default)]
pub struct PlaybackRegistry {
    sources: HashMap<String, SourcePlayback>,
}

impl PlaybackRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Synchronize a newly received output payload. Existing tracks retain
    /// their Rust clock; a changed media identity starts a fresh track from
    /// the payload's initial state.
    pub fn sync_content(&mut self, source_id: &str, content: &Value, now: Instant) -> Value {
        {
            let source = self.sources.entry(source_id.to_string()).or_default();
            sync_media_track(source, content, now);
            sync_music_track(source, content, now);
        }
        self.canonical_content(source_id, content, now)
    }

    /// Return a payload with the current Rust-owned snapshots applied.
    pub fn canonical_content(&mut self, source_id: &str, content: &Value, now: Instant) -> Value {
        let mut value = content.as_object().cloned().unwrap_or_default();
        if let Some(audio_settings) = value.get("audioSettings") {
            value.insert(
                "audioSettings".to_string(),
                normalize_audio_settings_value(audio_settings),
            );
        }
        let visual_key = scroll_content_key_for_value(&Value::Object(value.clone()));
        let source = self.sources.entry(source_id.to_string()).or_default();
        if source.scroll_visual_key.as_deref() != Some(visual_key.as_str()) {
            source.scroll_sequence = source.scroll_sequence.saturating_add(1);
            source.scroll_visual_key = Some(visual_key.clone());
            source.scroll_content_key =
                format!("{source_id}:{}:{visual_key}", source.scroll_sequence);
        }
        value.insert(
            "scrollContentKey".to_string(),
            json!(source.scroll_content_key),
        );
        let captured_at = unix_time_millis();
        let media_snapshot = source.media.as_mut().map(|track| track.snapshot_at(now));
        let music_snapshot = source.music.as_mut().map(|track| track.snapshot_at(now));
        value.insert(
            "backgroundPlaybackState".to_string(),
            media_snapshot
                .as_ref()
                .map_or(Value::Null, |snapshot| snapshot.to_value(captured_at)),
        );
        value.insert(
            "backgroundPlaybackError".to_string(),
            json!(media_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.error.clone())),
        );
        value.insert(
            "musicPlaybackState".to_string(),
            music_snapshot
                .as_ref()
                .map_or(Value::Null, |snapshot| snapshot.to_value(captured_at)),
        );

        let mut transport = value
            .remove("transportPlaybackPaused")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        if let Some(snapshot) = media_snapshot {
            transport.insert(
                MEDIA_LAYER.to_string(),
                json!(snapshot.status != PlaybackStatus::Playing),
            );
        }
        if let Some(snapshot) = music_snapshot {
            transport.insert(
                MUSIC_LAYER.to_string(),
                json!(snapshot.status != PlaybackStatus::Playing),
            );
        }
        value.insert(
            "transportPlaybackPaused".to_string(),
            Value::Object(transport),
        );
        if let Some(videos) = value
            .get_mut("linkedTextContext")
            .and_then(|context| context.get_mut("videoCountdowns"))
            .and_then(Value::as_object_mut)
        {
            for video in videos.values_mut().filter_map(Value::as_object_mut) {
                if let Some(track) =
                    self.sources
                        .values_mut()
                        .filter_map(|source| source.media.as_mut())
                        .find(|track| {
                            track.selection_id.as_ref().is_some_and(|id| {
                                !id.is_null() && Some(id) == video.get("selectionId")
                            })
                        })
                {
                    video.extend(
                        track
                            .snapshot_at(now)
                            .to_value(captured_at)
                            .as_object()
                            .unwrap()
                            .clone(),
                    );
                    video.insert("looping".into(), json!(track.looping));
                }
            }
        }
        let audio_selection = value
            .get("audioSelectionId")
            .filter(|id| !id.is_null())
            .cloned();
        if let (Some(selection), Some(context)) = (
            audio_selection,
            value
                .get_mut("linkedTextContext")
                .and_then(Value::as_object_mut),
        ) {
            if let Some(track) = self
                .sources
                .values_mut()
                .filter_map(|source| source.music.as_mut())
                .find(|track| track.selection_id.as_ref() == Some(&selection))
            {
                let mut audio = track.snapshot_at(now).to_value(captured_at);
                audio["looping"] = serde_json::json!(track.looping);
                context.insert("audioCountdown".into(), audio);
            }
        }
        Value::Object(value)
    }

    pub fn apply_command(
        &mut self,
        source_id: &str,
        command: PlaybackCommand,
        now: Instant,
    ) -> Option<PlaybackCommandOutcome> {
        let source = self.sources.get_mut(source_id)?;
        let track = source.track_mut(command.layer)?;
        let changed = track.apply(command, now);
        let snapshot = track.snapshot_at(now);
        Some(PlaybackCommandOutcome {
            payload: snapshot.to_control_payload(command, unix_time_millis()),
            changed,
        })
    }

    pub fn is_paused(
        &mut self,
        source_id: &str,
        layer: PlaybackLayer,
        now: Instant,
    ) -> Option<bool> {
        self.sources
            .get_mut(source_id)
            .and_then(|source| source.track_mut(layer))
            .map(|track| track.snapshot_at(now).status != PlaybackStatus::Playing)
    }

    pub(crate) fn set_deck_transport(
        &mut self,
        source_id: &str,
        layer: PlaybackLayer,
        rate: f64,
        stopped: bool,
        now: Instant,
    ) -> Result<PlaybackCommandOutcome, String> {
        if !rate.is_finite() || !(-4.0..=4.0).contains(&rate) || (rate != 0.0 && rate.abs() < 0.1) {
            return Err(
                "Deck shuttle rates must be zero or between 0.1x and 4x in either direction."
                    .into(),
            );
        }
        let track = self
            .sources
            .get_mut(source_id)
            .and_then(|source| source.track_mut(layer))
            .ok_or("The live media clock is unavailable.")?;
        if let Some(error) = &track.error {
            return Err(error.clone());
        }
        if track.duration.is_none() {
            return Err("A measured duration is required for deck playback.".into());
        }
        track.advance_to(now);
        let previous = (track.rate, track.status, track.shuttle, track.deck_stopped);
        track.prepared_start = false;
        track.rate = if rate == 0.0 { 1.0 } else { rate };
        track.shuttle = rate != 0.0 && rate != 1.0;
        track.deck_stopped = stopped && rate == 0.0;
        track.status = if rate == 0.0 {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        };
        track.delay_remaining = 0.0;
        track.anchor = now;
        let changed = previous != (track.rate, track.status, track.shuttle, track.deck_stopped);
        if changed {
            track.clock_revision = track.clock_revision.saturating_add(1);
        }
        let command = PlaybackCommand {
            layer,
            action: if rate == 0.0 {
                PlaybackAction::Pause
            } else {
                PlaybackAction::Play
            },
            position: 0.0,
        };
        Ok(PlaybackCommandOutcome {
            payload: track
                .snapshot_at(now)
                .to_control_payload(command, unix_time_millis()),
            changed,
        })
    }

    /// Attach a duration discovered by the native decoder to an existing
    /// clock track. Music payloads intentionally do not carry browser
    /// metadata, so this is filled in after Rust opens the local file.
    pub fn set_duration(
        &mut self,
        source_id: &str,
        layer: PlaybackLayer,
        duration: f64,
        now: Instant,
    ) {
        let Some(track) = self
            .sources
            .get_mut(source_id)
            .and_then(|source| source.track_mut(layer))
        else {
            return;
        };
        let Some(duration) = normalize_duration(Some(duration)) else {
            return;
        };
        if track.duration == Some(duration) {
            return;
        }
        track.duration = Some(duration);
        if let Err(error) =
            validate_media_transport_duration(track.in_seconds, track.out_seconds, duration)
        {
            track.error = Some(error);
            track.status = PlaybackStatus::Ended;
            track.clock_revision = track.clock_revision.saturating_add(1);
            return;
        }
        track.position = clamp_position(track.position, Some(duration));
        if track.position >= duration && !track.looping {
            track.status = PlaybackStatus::Ended;
        }
        track.anchor = now;
        track.clock_revision = track.clock_revision.saturating_add(1);
    }
}

const SCROLL_CONTENT_EPHEMERAL_FIELDS: &[&str] = &[
    "scrollContentKey",
    "backgroundPlaybackState",
    "musicPlaybackState",
    "transportPlaybackPaused",
    "audioSettings",
    "nativeAudioLayers",
    "nativeAudioActive",
    "nativeAudioTapDroppedBlocks",
    "audioPath",
    "audioUrl",
    "label",
    "currentTime",
    "capturedAt",
    "clockRevision",
    "playbackTime",
];

/// Produce a deterministic visual identity for a native output payload.
///
/// Output playback snapshots and audio routing are deliberately excluded:
/// they can change every frame without changing the cue that is on screen.
/// Object keys are sorted recursively so display and NDI get the same identity
/// even when their input maps were assembled in a different order.
pub(crate) fn scroll_content_key_for_value(value: &Value) -> String {
    let canonical = canonical_scroll_value(value, true);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn canonical_scroll_value(value: &Value, top_level: bool) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object
                .iter()
                .filter(|(key, _)| {
                    !top_level || !SCROLL_CONTENT_EPHEMERAL_FIELDS.contains(&key.as_str())
                })
                .map(|(key, value)| (key.clone(), canonical_scroll_value(value, false)))
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut canonical = Map::new();
            for (key, value) in entries {
                canonical.insert(key, value);
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| canonical_scroll_value(value, false))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// Normalize the audio intent consumed by native output and NDI.
///
/// Rust owns this boundary so every native output consumer receives the same
/// safe device, gain, delay, and routing shape.
pub(crate) fn normalize_audio_settings_value(value: &Value) -> Value {
    let source = value.as_object();
    let mut normalized = Map::new();
    normalized.insert(
        "main".to_string(),
        normalize_audio_section(source.and_then(|value| value.get("main")), false),
    );
    normalized.insert(
        "sdiNdi".to_string(),
        normalize_audio_section(source.and_then(|value| value.get("sdiNdi")), true),
    );
    normalized.insert(
        "inspector".to_string(),
        normalize_inspector_audio(source.and_then(|value| value.get("inspector"))),
    );
    Value::Object(normalized)
}

/// Operator monitor audio for the renderer's inspector preview; native outputs ignore it.
fn normalize_inspector_audio(value: Option<&Value>) -> Value {
    let source = value.and_then(Value::as_object);
    json!({
        "device": source.and_then(|value| value.get("device")).and_then(Value::as_str).unwrap_or("system"),
        "deviceLabel": source.and_then(|value| value.get("deviceLabel")).and_then(Value::as_str).unwrap_or(""),
        "level": clamp_audio_level(source.and_then(|value| value.get("level"))),
        "routing": normalize_audio_routing(source.and_then(|value| value.get("routing"))),
    })
}

fn normalize_audio_section(value: Option<&Value>, default_enabled: bool) -> Value {
    let source = value.and_then(Value::as_object);
    let mut normalized = Map::new();
    if let Some(device) = source
        .and_then(|value| value.get("device"))
        .and_then(Value::as_str)
    {
        normalized.insert("device".to_string(), json!(device));
    } else if !default_enabled {
        normalized.insert("device".to_string(), json!("system"));
    }
    if let Some(device_label) = source
        .and_then(|value| value.get("deviceLabel"))
        .and_then(Value::as_str)
    {
        normalized.insert("deviceLabel".to_string(), json!(device_label));
    } else if !default_enabled {
        normalized.insert("deviceLabel".to_string(), json!(""));
    }
    if let Some(enabled) = source
        .and_then(|value| value.get("enabled"))
        .and_then(Value::as_bool)
    {
        normalized.insert("enabled".to_string(), json!(enabled));
    } else if default_enabled {
        normalized.insert("enabled".to_string(), json!(true));
    }
    normalized.insert(
        "level".to_string(),
        json!(clamp_audio_level(
            source.and_then(|value| value.get("level"))
        )),
    );
    normalized.insert(
        "delay".to_string(),
        json!(normalize_audio_delay(
            source.and_then(|value| value.get("delay"))
        )),
    );
    normalized.insert(
        "routing".to_string(),
        normalize_audio_routing(source.and_then(|value| value.get("routing"))),
    );
    Value::Object(normalized)
}

fn normalize_audio_routing(value: Option<&Value>) -> Value {
    let source = value.and_then(Value::as_object);
    let device_channels = source
        .and_then(|value| value.get("deviceChannels"))
        .and_then(json_number)
        .filter(|value| *value != 0.0)
        .map(|value| value.round() as i64)
        .unwrap_or(DEFAULT_AUDIO_DEVICE_CHANNELS)
        .clamp(1, MAX_AUDIO_DEVICE_CHANNELS);
    let incoming_channels = source
        .and_then(|value| value.get("proChannels"))
        .and_then(Value::as_array);
    let highest_authored_channel = incoming_channels
        .into_iter()
        .flatten()
        .filter_map(|channel| channel.get("id").and_then(Value::as_str))
        .filter_map(|id| {
            id.strip_prefix("pro-")
                .and_then(|number| number.parse::<usize>().ok())
        })
        .filter(|number| (1..=LOGICAL_AUDIO_CHANNELS).contains(number))
        .max()
        .unwrap_or(0);
    let channel_count = incoming_channels
        .map(|channels| channels.len())
        .unwrap_or(0)
        .max(highest_authored_channel)
        .clamp(2, LOGICAL_AUDIO_CHANNELS);
    let channels = (0..channel_count)
        .map(|index| {
            let id = format!("pro-{}", index + 1);
            let default_label = format!("Pro Channel {}", index + 1);
            let incoming = incoming_channels
                .and_then(|values| {
                    values.iter().find(|value| {
                        value
                            .get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|incoming_id| incoming_id == id.as_str())
                    })
                })
                .and_then(Value::as_object);
            let incoming_channel_map = incoming.and_then(|value| value.get("channels"));
            let incoming_modes = incoming.and_then(|value| value.get("modes"));
            let mut channel = Map::new();
            channel.insert("id".to_string(), json!(id));
            channel.insert(
                "label".to_string(),
                json!(incoming
                    .and_then(|value| value.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or(default_label.as_str())),
            );
            let mut channel_map = Map::new();
            for device_channel in 1..=device_channels {
                let key = device_channel.to_string();
                let default_enabled = match index {
                    0 => device_channel == 1,
                    1 => device_channel == 2,
                    _ => false,
                };
                let enabled = incoming_channel_map
                    .and_then(|value| value.get(&key))
                    .and_then(Value::as_bool)
                    .unwrap_or(default_enabled);
                channel_map.insert(key, json!(enabled));
            }
            channel.insert("channels".to_string(), Value::Object(channel_map));
            channel.insert(
                "modes".to_string(),
                json!({
                    "mute": incoming_modes
                        .and_then(|value| value.get("mute"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    "solo": incoming_modes
                        .and_then(|value| value.get("solo"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    "tone": incoming_modes
                        .and_then(|value| value.get("tone"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                }),
            );
            Value::Object(channel)
        })
        .collect::<Vec<_>>();
    json!({ "deviceChannels": device_channels, "proChannels": channels })
}

fn clamp_audio_level(value: Option<&Value>) -> f64 {
    value
        .and_then(json_number)
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        .clamp(-60.0, 6.0)
}

fn normalize_audio_delay(value: Option<&Value>) -> i64 {
    value
        .and_then(json_number)
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        .max(0.0)
        .round() as i64
}

fn sync_media_track(source: &mut SourcePlayback, content: &Value, now: Instant) {
    let media_type = content
        .get("backgroundMediaType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = content
        .get("backgroundMediaPath")
        .or_else(|| content.get("backgroundMediaUrl"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !media_type.eq_ignore_ascii_case("video") || url.trim().is_empty() {
        source.media = None;
        return;
    }
    let selection_id = content.get("backgroundSelectionId").cloned();
    let identity = track_identity(url, selection_id.as_ref());
    if source
        .media
        .as_ref()
        .is_some_and(|track| track.identity == identity)
    {
        let paused = transport_paused(content, PlaybackLayer::Media);
        let track = source.media.as_mut().expect("track identity was checked");
        if let Some(duration) = duration_from_content(content) {
            track.duration = Some(duration);
            if let Err(error) =
                validate_media_transport_duration(track.in_seconds, track.out_seconds, duration)
            {
                track.error = Some(error);
                track.status = PlaybackStatus::Ended;
            }
        }
        track.advance_to(now);
        let next_status = if paused {
            PlaybackStatus::Paused
        } else if track.status == PlaybackStatus::Paused {
            PlaybackStatus::Playing
        } else {
            track.status
        };
        if next_status != track.status {
            track.status = next_status;
            track.anchor = now;
            track.clock_revision = track.clock_revision.saturating_add(1);
        }
        return;
    }
    let state = content
        .get("backgroundPlaybackState")
        .and_then(Value::as_object);
    let state_status = PlaybackStatus::from_value(
        state.and_then(|value| value.get("status")),
        PlaybackStatus::Playing,
    );
    let status = if state_status == PlaybackStatus::Ended {
        PlaybackStatus::Ended
    } else if let Some(paused) = transport_pause_state(content, PlaybackLayer::Media) {
        if paused {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        }
    } else {
        state_status
    };
    let duration = duration_from_state_or_content(state, content);
    let position = state
        .and_then(|value| value.get("currentTime"))
        .and_then(json_number)
        .unwrap_or(0.0);
    let previous_position = source
        .media
        .as_mut()
        .filter(|track| track.identity == track_identity(url, track.selection_id.as_ref()))
        .map(|track| {
            track.advance_to(now);
            (
                track.position,
                track.status,
                track.delay_remaining,
                track.loop_iteration,
            )
        });
    let mut track = PlaybackTrack::new(
        identity,
        selection_id,
        duration,
        position,
        status,
        content
            .get("backgroundPlayback")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("loop")),
        now,
    );
    if let Some(transport) = content
        .get("backgroundTransport")
        .and_then(Value::as_object)
    {
        track.in_seconds = transport
            .get("inSeconds")
            .and_then(json_number)
            .expect("validated media transport in point");
        track.out_seconds = transport.get("outSeconds").and_then(json_number);
        track.rate = transport
            .get("rate")
            .and_then(json_number)
            .expect("validated media transport rate");
        track.delay_remaining = transport
            .get("delaySeconds")
            .and_then(json_number)
            .expect("validated media transport delay");
        let continue_playback = match transport.get("retrigger").and_then(Value::as_str) {
            Some("continue") => true,
            Some("automatic") => {
                content.get("backgroundBehavior").and_then(Value::as_str) == Some("background")
                    && content.get("backgroundPlayback").and_then(Value::as_str) == Some("loop")
            }
            _ => false,
        };
        track.position = track.in_seconds;
        track.prepared_start = position <= track.in_seconds;
        if let Some((position, status, delay, iteration)) =
            previous_position.filter(|_| continue_playback)
        {
            track.prepared_start = false;
            track.position = position;
            track.status = status;
            track.delay_remaining = delay;
            track.loop_iteration = iteration;
        }
        track.position = track.position.max(track.in_seconds);
        if let Some(end) = track.out_seconds.or(track.duration) {
            track.position = track.position.min(end);
        }
        if let Some(duration) = track.duration {
            if let Err(error) =
                validate_media_transport_duration(track.in_seconds, track.out_seconds, duration)
            {
                track.error = Some(error);
                track.status = PlaybackStatus::Ended;
            }
        }
    }
    source.media = Some(track);
}

fn sync_music_track(source: &mut SourcePlayback, content: &Value, now: Instant) {
    let url = content
        .get("audioPath")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| content.get("audioUrl").and_then(Value::as_str))
        .unwrap_or_default();
    if url.trim().is_empty() {
        source.music = None;
        return;
    }
    let selection = content.get("audioSelectionId").cloned();
    let identity = track_identity(url, selection.as_ref());
    if source
        .music
        .as_ref()
        .is_some_and(|track| track.identity == identity)
    {
        let paused = transport_paused(content, PlaybackLayer::Music);
        let track = source.music.as_mut().expect("track identity was checked");
        track.advance_to(now);
        let next_status = if paused {
            PlaybackStatus::Paused
        } else if track.status == PlaybackStatus::Paused {
            PlaybackStatus::Playing
        } else {
            track.status
        };
        if next_status != track.status {
            track.status = next_status;
            track.anchor = now;
            track.clock_revision = track.clock_revision.saturating_add(1);
        }
        return;
    }
    let status = if transport_paused(content, PlaybackLayer::Music) {
        PlaybackStatus::Paused
    } else {
        PlaybackStatus::Playing
    };
    source.music = Some(PlaybackTrack::new(
        identity, selection, None, 0.0, status, false, now,
    ));
}

fn duration_from_state_or_content(
    state: Option<&Map<String, Value>>,
    content: &Value,
) -> Option<f64> {
    state
        .and_then(|value| value.get("duration"))
        .and_then(json_number)
        .or_else(|| content.get("duration").and_then(json_number))
        .and_then(|value| normalize_duration(Some(value)))
}

fn duration_from_content(content: &Value) -> Option<f64> {
    content
        .get("duration")
        .and_then(json_number)
        .and_then(|value| normalize_duration(Some(value)))
}

fn transport_paused(content: &Value, layer: PlaybackLayer) -> bool {
    transport_pause_state(content, layer).unwrap_or(false)
}

fn transport_pause_state(content: &Value, layer: PlaybackLayer) -> Option<bool> {
    content
        .get("transportPlaybackPaused")
        .and_then(Value::as_object)
        .and_then(|value| value.get(layer.as_str()))
        .and_then(Value::as_bool)
}

fn track_identity(url: &str, selection_id: Option<&Value>) -> String {
    format!(
        "{}|{}",
        url.trim(),
        selection_id.map(value_key).unwrap_or_default()
    )
}

fn value_key(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn json_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn normalize_duration(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

fn clamp_position(value: f64, duration: Option<f64>) -> f64 {
    let value = value.max(0.0);
    duration.map_or(value, |duration| value.min(duration))
}

fn unix_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
