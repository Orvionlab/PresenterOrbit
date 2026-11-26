#[path = "native_preview_audio.rs"]
mod preview;
pub use preview::PreviewAudio;
#[path = "native_program_levels.rs"]
mod program_levels;
pub use program_levels::program_audio_levels;
#[path = "native_audio_crossfade.rs"]
mod crossfade;
use std::{
    cell::UnsafeCell,
    collections::{HashMap, VecDeque},
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use rodio::{
    cpal::{
        self,
        traits::{DeviceTrait, HostTrait},
    },
    source::SeekError,
    ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source,
};
use serde_json::{json, Map, Value};

use crate::native_media::{prefetch_media_url, resolve_media_path};
use crate::output_playback::{normalize_audio_settings_value, PlaybackLayer, PlaybackStatus};

const DEFAULT_DEVICE: &str = "system";
#[path = "native_audio_input_capture.rs"]
mod input_capture;
#[path = "native_audio_inputs.rs"]
pub(crate) mod inputs;
const NONE_DEVICE: &str = "none";
const MAX_DRIFT_SECONDS: f64 = 0.25;
const MAX_OUTPUT_CHANNELS: u16 = 16;
const TONE_GAIN: f32 = 0.18;
const TONE_FREQUENCIES_HZ: [f32; 2] = [440.0, 660.0];
const NDI_TAP_BLOCK_FRAMES: usize = 256;
const NDI_TAP_QUEUE_CAPACITY: usize = 64;
const NDI_MAX_PENDING_BLOCKS: usize = 32;
const NDI_MAX_BLOCKS_PER_DRAIN: usize = 8;
const NDI_MIX_SAMPLE_RATE: u32 = 48_000;
const NDI_MIX_CHANNELS: usize = 2;
const NDI_DECODER_MAX_LAG: Duration = Duration::from_millis(20);
const NDI_MAX_DECODE_FRAMES_PER_BLOCK: usize = 4096;
const NDI_MAX_DECODER_CATCH_UP_BLOCKS: usize = 4;

/// Per-layer native playback state exposed in canonical output diagnostics.
///
/// A layer is marked active when Rust owns it, including the intentional
/// silence case (no selected media or `device: none`). A false layer carries
/// an explicit native error; output playback never falls back to a WebView.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeAudioStatus {
    pub media_active: bool,
    pub music_active: bool,
    pub error: Option<String>,
    pub media_duration: Option<f64>,
    pub music_duration: Option<f64>,
}

impl NativeAudioStatus {
    pub fn to_value(&self) -> Value {
        let mut value = json!({
            "media": self.media_active,
            "music": self.music_active,
        });
        if let Some(error) = &self.error {
            value["error"] = json!(error);
        }
        value
    }

    fn set_layer(&mut self, layer: PlaybackLayer, active: bool) {
        match layer {
            PlaybackLayer::Media => self.media_active = active,
            PlaybackLayer::Music => self.music_active = active,
        }
    }

    fn set_layer_error(&mut self, layer: PlaybackLayer, error: String) {
        self.set_layer(layer, false);
        self.error.get_or_insert(error);
    }
}

struct NativeAudioTrack {
    identity: String,
    player: Player,
    looping: bool,
    duration: Option<f64>,
    loop_limit: Option<f64>,
    rate: f32,
    delay: Duration,
    status: PlaybackStatus,
    ndi_tap: PcmTapReader,
    ndi_pending: VecDeque<NativePcmBlock>,
    ndi_generation: u64,
}

/// Ticker-owned decoder used when a source has no usable native output sink.
///
/// This deliberately does not use `Source::repeat_infinite()`: rodio's repeat
/// adapter buffers the whole source so it can replay it, which is an
/// unbounded allocation from the point of view of a live output. The worker
/// reopens the local file at EOF instead and only retains the same bounded
/// pending queue used by the sink tap.
struct NdiDecoderTrack {
    identity: String,
    path: PathBuf,
    decoder: Decoder<BufReader<File>>,
    duration: Option<f64>,
    channels: usize,
    sample_rate: u32,
    rate: f32,
    looping: bool,
    loop_limit: Option<f64>,
    consumed_frames: u64,
    /// How often a looping source has restarted, so markers can repeat per pass.
    loop_iteration: u64,
    status: PlaybackStatus,
    position_frames: u64,
    /// Fractional source frames carried between fixed 256-frame/48 kHz NDI
    /// deadlines. This keeps 44.1 kHz sources alternating 235/236 frames
    /// instead of rounding every block up to 236.
    source_frame_remainder: u64,
    pending: VecDeque<NativePcmBlock>,
    next_block_at: Option<Instant>,
    ready: bool,
    ended: bool,
    error: Option<String>,
}

impl NdiDecoderTrack {
    fn open(path: &Path, looping: bool, state: PlaybackState) -> Result<Self, String> {
        let (decoder, duration, channels, sample_rate) = open_ndi_decoder(path)?;
        let mut track = Self {
            identity: ndi_decoder_identity(path, looping),
            path: path.to_path_buf(),
            decoder,
            duration,
            channels,
            sample_rate,
            rate: 1.0,
            looping,
            loop_limit: None,
            consumed_frames: 0,
            loop_iteration: 0,
            status: PlaybackStatus::Paused,
            position_frames: 0,
            source_frame_remainder: 0,
            pending: VecDeque::new(),
            next_block_at: None,
            ready: false,
            ended: false,
            error: None,
        };
        track.apply_state(state)?;
        Ok(track)
    }

    fn apply_state(&mut self, state: PlaybackState) -> Result<(), String> {
        let current_position = if self.loop_limit.is_some() {
            self.consumed_frames as f64 / self.sample_rate.max(1) as f64
        } else {
            self.position_seconds()
        };
        let target_position = state.position.max(0.0);
        // A pause/resume transition is status-only. Repositioning here would
        // reset decoder state (and fail for sequential-only codecs); explicit
        // position drift, initial non-zero starts, and ended-track restarts
        // remain seek points below.
        let needs_seek = !self.ready
            || self.ended && self.loop_limit.is_none() && state.status == PlaybackStatus::Playing
            || (!is_status_only_transition(self.status, state.status)
                && position_drift(
                    current_position,
                    target_position,
                    self.duration,
                    self.looping && self.loop_limit.is_none(),
                ) > MAX_DRIFT_SECONDS);

        // Starting a decoder at zero does not require a seek. A few native
        // codec readers can decode sequentially but do not expose a seek
        // implementation; they should still work for the common initial
        // position while an explicit non-zero transport seek reports its
        // limitation.
        let initial_sequential_start = !self.ready && target_position <= f64::EPSILON;
        if needs_seek && !initial_sequential_start {
            self.seek(target_position)?;
        } else if initial_sequential_start {
            self.pending.clear();
            self.position_frames = 0;
            self.consumed_frames = 0;
            self.source_frame_remainder = 0;
            self.ended = false;
            self.next_block_at = None;
        }
        self.status = state.status;
        self.ready = true;
        if state.status == PlaybackStatus::Ended {
            self.ended = true;
            self.pending.clear();
            self.next_block_at = None;
        } else if state.status == PlaybackStatus::Paused {
            self.pending.clear();
            self.next_block_at = None;
        }
        Ok(())
    }

    fn seek(&mut self, position: f64) -> Result<(), String> {
        let position = if position.is_finite() {
            position.max(0.0)
        } else {
            0.0
        };
        let local_position = if self.looping {
            self.duration
                .filter(|duration| *duration > 0.0)
                .map_or(position, |duration| position % duration)
        } else {
            position
        };
        self.decoder
            .try_seek(Duration::from_secs_f64(local_position))
            .map_err(|error| {
                format!(
                    "Cannot seek native NDI audio '{}' to {position:.3}s: {error}",
                    self.path.display()
                )
            })?;
        self.consumed_frames = (position * self.sample_rate as f64).round().max(0.0) as u64;
        self.loop_iteration = if self.looping {
            self.duration
                .filter(|duration| *duration > 0.0)
                .map_or(0, |duration| (position / duration).floor() as u64)
        } else {
            0
        };
        let frames = local_position * self.sample_rate.max(1) as f64;
        self.position_frames = if frames.is_finite() && frames >= 0.0 {
            frames.round().min(u64::MAX as f64) as u64
        } else {
            0
        };
        self.source_frame_remainder = 0;
        self.pending.clear();
        self.next_block_at = None;
        self.ready = true;
        self.ended = false;
        self.error = None;
        Ok(())
    }

    fn position_seconds(&self) -> f64 {
        self.position_frames as f64 / self.sample_rate.max(1) as f64
    }

    fn reset(&mut self) {
        self.pending.clear();
        self.next_block_at = None;
        self.ready = false;
        self.source_frame_remainder = 0;
        self.ended = false;
        self.consumed_frames = 0;
        self.error = None;
    }

    fn decode_block(&mut self, timestamp: Instant) -> Result<Option<NativePcmBlock>, String> {
        if !self.ready || self.status != PlaybackStatus::Playing || self.ended {
            return Ok(None);
        }
        let numerator = (NDI_TAP_BLOCK_FRAMES as u64)
            .saturating_mul((self.sample_rate as f32 * self.rate).max(1.0) as u64)
            .saturating_add(self.source_frame_remainder);
        let denominator = NDI_MIX_SAMPLE_RATE as u64;
        let target_frames = (numerator / denominator) as usize;
        self.source_frame_remainder = numerator % denominator;
        let target_frames = target_frames.clamp(1, NDI_MAX_DECODE_FRAMES_PER_BLOCK);
        let mut samples = Vec::with_capacity(target_frames * self.channels.max(1));
        let target_samples = target_frames * self.channels.max(1);
        // Frames of this block that precede a loop restart belong to the previous pass.
        let mut frames_before_restart = 0;
        while samples.len() < target_samples {
            if self.loop_limit.is_some_and(|limit| {
                self.consumed_frames + (samples.len() / self.channels.max(1)) as u64
                    >= (limit * self.sample_rate as f64).round() as u64
            }) {
                self.ended = true;
                break;
            }
            match self.decoder.next() {
                Some(sample) => samples.push(sample),
                None if self.looping => {
                    let (decoder, duration, channels, sample_rate) = open_ndi_decoder(&self.path)?;
                    if channels != self.channels || sample_rate != self.sample_rate {
                        return Err(format!(
                            "Native NDI audio '{}' changed channel layout while looping.",
                            self.path.display()
                        ));
                    }
                    self.decoder = decoder;
                    self.duration = duration;
                    self.position_frames = 0;
                    self.loop_iteration = self.loop_iteration.saturating_add(1);
                    frames_before_restart = samples.len() / self.channels.max(1);
                }
                None => {
                    self.ended = true;
                    break;
                }
            }
        }
        let complete_samples = samples.len() - samples.len() % self.channels.max(1);
        samples.truncate(complete_samples);
        if samples.is_empty() {
            return Ok(None);
        }
        let frames = samples.len() / self.channels.max(1);
        self.consumed_frames = self.consumed_frames.saturating_add(frames as u64);
        self.position_frames = self
            .position_frames
            .saturating_add(frames.saturating_sub(frames_before_restart.min(frames)) as u64);
        Ok(Some(NativePcmBlock {
            asset_routed: false,
            preserve_channel_count: false,
            samples,
            sample_rate: (self.sample_rate as f32 * self.rate).max(1.0) as u32,
            channels: self.channels,
            timestamp,
            offset_frames: 0,
        }))
    }
}

/// Native output audio service.
///
/// One OS mixer is shared by the media and music players for each active
/// output source. This keeps the audio stream alive independently of renderer
/// windows while the Rust playback registry remains the clock authority.
/// Remote HTTP(S) sources enter the shared bounded native media cache before
/// the decoder opens them; Tauri asset URLs remain renderer-only values.
pub struct NativeAudioRegistry {
    inputs: inputs::Inputs,
    input_media_probe: Option<(PathBuf, std::time::SystemTime, bool)>,
    sink: Option<MixerDeviceSink>,
    device_key: String,
    tracks: HashMap<String, NativeAudioTrack>,
    ndi_settings_keys: HashMap<String, String>,
    /// Latest canonical payloads are kept only so the NDI ticker can lazily
    /// start a decoder for a source that has no output sink. The map is
    /// cleared with the source and is bounded by active output sources.
    ndi_content: HashMap<String, Value>,
    ndi_decoders: HashMap<String, NdiDecoderTrack>,
    ndi_decoder_errors: HashMap<String, (String, String)>,
    music_ended_reported: HashMap<String, String>,
    music_crossfades: HashMap<String, crossfade::MusicCrossfade>,
}

impl Default for NativeAudioRegistry {
    fn default() -> Self {
        Self {
            inputs: inputs::Inputs::default(),
            input_media_probe: None,
            sink: None,
            device_key: String::new(),
            tracks: HashMap::new(),
            ndi_settings_keys: HashMap::new(),
            ndi_content: HashMap::new(),
            ndi_decoders: HashMap::new(),
            ndi_decoder_errors: HashMap::new(),
            music_ended_reported: HashMap::new(),
            music_crossfades: HashMap::new(),
        }
    }
}

impl NativeAudioRegistry {
    /// Publish the position consumed by the native sink, never canonical transport estimates.
    pub(crate) fn publish_music_marker_positions(&self) {
        let mut positions: HashMap<(PathBuf, String), (Value, f64, u64)> = HashMap::new();
        for (source_id, content) in &self.ndi_content {
            let Some(path) = local_music_path(content) else {
                continue;
            };
            let Some(selection) = content
                .get("audioSelectionId")
                .filter(|value| !value.is_null())
            else {
                continue;
            };
            let Some(track) = self.tracks.get(&track_key(source_id, PlaybackLayer::Music)) else {
                continue;
            };
            if track.status != PlaybackStatus::Playing {
                continue;
            }
            let consumed = track.player.get_pos();
            let terminal = track
                .player
                .empty()
                .then_some(track.loop_limit.or(track.duration))
                .flatten();
            if track.player.empty() && terminal.is_none() {
                continue;
            }
            if consumed <= track.delay && terminal.is_none() {
                continue;
            }
            let position =
                consumed.saturating_sub(track.delay).as_secs_f64() * f64::from(track.rate);
            let position = terminal.map_or(position, |duration| duration.max(position));
            let sample = if track.player.empty() && track.looping && track.loop_limit.is_some() {
                loop_terminal(position, track.duration)
            } else {
                loop_pass(position, track.looping, track.duration)
            };
            positions
                .entry((path, selection.to_string()))
                .and_modify(|entry| {
                    if (sample.1, sample.0) < (entry.2, entry.1) {
                        (entry.1, entry.2) = sample;
                    }
                })
                .or_insert((selection.clone(), sample.0, sample.1));
        }
        for ((path, _), (selection, position, iteration)) in positions {
            crate::native_render::publish_loop_marker_position(
                &path,
                &audio_marker_selection(&selection),
                position,
                iteration,
            );
        }
    }

    fn main_music_end(&self, path: &Path, selection: Option<&Value>) -> Option<bool> {
        let mut tracks = self
            .ndi_content
            .iter()
            .filter(|(_, content)| {
                local_music_path(content).as_deref() == Some(path)
                    && content.get("audioSelectionId") == selection
            })
            .filter_map(|(source_id, _)| {
                self.tracks.get(&track_key(source_id, PlaybackLayer::Music))
            });
        let first = tracks.next()?;
        Some(first.player.empty() && tracks.all(|track| track.player.empty()))
    }

    fn has_main_music_clock(&self, path: &Path, selection: &Value) -> bool {
        self.main_music_end(path, Some(selection)).is_some()
    }

    pub fn music_ended_sources(&mut self) -> Vec<(String, String)> {
        let mut ended = Vec::new();
        for (source_id, content) in &self.ndi_content {
            if playback_state(content, PlaybackLayer::Music).status == PlaybackStatus::Paused
                || content["transportPlaybackPaused"]["music"] == true
            {
                continue;
            }
            let key = track_key(source_id, PlaybackLayer::Music);
            let identity = format!(
                "{}|{}",
                content.get("audioId").unwrap_or(&Value::Null),
                content.get("audioSelectionId").unwrap_or(&Value::Null)
            );
            let at_end = local_music_path(content)
                .and_then(|path| self.main_music_end(&path, content.get("audioSelectionId")))
                .unwrap_or_else(|| {
                    self.ndi_decoders.get(&key).is_some_and(|track| {
                        track.ended && track.pending.is_empty() && track.error.is_none()
                    })
                });
            if at_end && self.music_ended_reported.get(source_id) != Some(&identity) {
                self.music_ended_reported
                    .insert(source_id.clone(), identity.clone());
                ended.push((source_id.clone(), identity));
            }
        }
        ended
    }
    pub fn new() -> Self {
        Self::default()
    }

    /// Synchronize a source with a canonical Rust playback payload.
    pub fn sync_content(
        &mut self,
        source_id: &str,
        content: &Value,
        now: Instant,
    ) -> NativeAudioStatus {
        let mut status = self.sync_playback(source_id, content, now);
        self.update_music_crossfade(source_id, content, now);
        let mut input_content = content.clone();
        input_content["foregroundAudioActive"] = json!(self.foreground_input_audio(content));
        if let Err(error) = self.inputs.sync(
            source_id,
            &input_content,
            self.sink.as_ref(),
            &self.device_key,
        ) {
            status.error.get_or_insert(error);
        }
        status
    }

    fn foreground_input_audio(&mut self, content: &Value) -> bool {
        let identity = resolve_media_path(&content["audioInputContext"]["foregroundPath"])
            .and_then(|path| {
                let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
                Some((path, modified))
            });
        let Some((path, modified)) = identity else {
            self.input_media_probe = None;
            return false;
        };
        if let Some((cached_path, cached_modified, present)) = &self.input_media_probe {
            if cached_path == &path && cached_modified == &modified {
                return *present;
            }
        }
        let present = File::open(&path)
            .ok()
            .and_then(|file| Decoder::try_from(file).ok())
            .is_some();
        self.input_media_probe = Some((path, modified, present));
        present
    }

    pub fn input_status(&self) -> Value {
        self.inputs.status()
    }

    fn sync_playback(
        &mut self,
        source_id: &str,
        content: &Value,
        now: std::time::Instant,
    ) -> NativeAudioStatus {
        self.prepare_music_crossfade(source_id, content, now);
        self.ndi_content
            .insert(source_id.to_string(), content.clone());
        self.sync_ndi_settings(source_id, content);
        let settings =
            normalize_audio_settings_value(content.get("audioSettings").unwrap_or(&Value::Null));
        let main = settings.get("main").unwrap_or(&Value::Null);
        let device = selected_device(main);
        let enabled = main.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        let ndi_enabled = ndi_audio_enabled(&settings);

        let mut status = NativeAudioStatus::default();
        // The audio/video sync test pattern owns the media layer with its beep.
        let sync_beep = crate::test_pattern::plays_sync_beep(content);
        let media_path = local_media_path(content).filter(|_| !sync_beep);
        let music_path = local_music_path(content);
        let media_requested = !sync_beep
            && (media_path.is_some() || media_url_present(content, PlaybackLayer::Media));
        let music_requested =
            music_path.is_some() || media_url_present(content, PlaybackLayer::Music);

        if !enabled || device.eq_ignore_ascii_case(NONE_DEVICE) {
            self.clear_playback_layers(source_id);
            if !ndi_enabled {
                self.remove_ndi_decoder_source(source_id);
            }
            status.media_active = true;
            status.music_active = true;
            (status.media_duration, status.music_duration) = idle_layer_durations(content);
            if let Some(error) = self.sync_existing_ndi_decoders(source_id, content) {
                status.error = Some(error);
            }
            return status;
        }

        let routing = audio_routing(main);

        if media_path.is_none() && media_requested {
            status.error = Some("Native media audio is not ready for native decoding.".to_string());
        }
        if music_path.is_none() && music_requested {
            status.error.get_or_insert_with(|| {
                "Native music audio is not ready for native decoding.".to_string()
            });
        }

        let has_local_track = sync_beep
            || media_path.is_some()
            || music_path.is_some()
            || inputs::Inputs::requested(content);
        if has_local_track {
            if let Err(error) = self.ensure_sink(&device, routing.output_channels) {
                self.clear_playback_layers(source_id);
                status.error = Some(error);
                status.media_active = !media_requested;
                status.music_active = !music_requested;
                if ndi_enabled {
                    if let Some(error) = self.sync_existing_ndi_decoders(source_id, content) {
                        status.error.get_or_insert(error);
                    }
                }
                return status;
            }
        } else {
            self.remove_layer(source_id, PlaybackLayer::Media);
            self.remove_layer(source_id, PlaybackLayer::Music);
        }

        let gain = gain_from_settings(main);
        let delay = delay_from_settings(main);
        let media_state = playback_state(content, PlaybackLayer::Media);
        let music_state = playback_state(content, PlaybackLayer::Music);

        if sync_beep {
            self.sync_beep(source_id, gain, delay, routing.clone());
            status.media_active = true;
        } else if let Some(ref path) = media_path {
            match self.sync_layer(
                source_id,
                PlaybackLayer::Media,
                &path,
                media_state,
                gain,
                delay,
                background_loops(content),
                None,
                routing.clone(),
                content
                    .get("backgroundAudioRouting")
                    .filter(|value| !value.is_null()),
                playback_rate(content, PlaybackLayer::Media),
                None,
            ) {
                Ok(duration) => {
                    status.media_active = true;
                    status.media_duration = duration;
                }
                Err(error) => {
                    status.set_layer_error(PlaybackLayer::Media, error);
                }
            }
        } else {
            self.remove_layer(source_id, PlaybackLayer::Media);
            status.media_active = !media_requested;
        }

        if let Some(ref path) = music_path {
            match self.sync_layer(
                source_id,
                PlaybackLayer::Music,
                &path,
                music_state,
                gain,
                delay,
                music_loops(content),
                Some(content),
                routing.clone(),
                content
                    .get("musicAudioRouting")
                    .filter(|value| !value.is_null()),
                1.0,
                content.get("audioSelectionId"),
            ) {
                Ok(duration) => {
                    status.music_active = true;
                    status.music_duration = duration;
                }
                Err(error) => {
                    status.set_layer_error(PlaybackLayer::Music, error);
                }
            }
        } else {
            self.remove_layer(source_id, PlaybackLayer::Music);
            status.music_active = !music_requested;
        }

        // Decode failures remain explicit per-layer native errors. Other
        // successfully decoded layers continue without duplicate playback.
        if media_path.is_some() && !status.media_active {
            status.media_active = false;
        }
        if music_path.is_some() && !status.music_active {
            status.music_active = false;
        }
        if !ndi_enabled {
            self.remove_ndi_decoder_source(source_id);
        } else if let Some(error) = self.sync_existing_ndi_decoders(source_id, content) {
            status.error.get_or_insert(error);
        }
        status
    }

    /// Drop players for a source that no longer has a native output window.
    pub fn clear_source(&mut self, source_id: &str) {
        self.music_crossfades.remove(source_id);
        self.inputs.clear(source_id);
        self.clear_playback_layers(source_id);
        self.remove_ndi_decoder_source(source_id);
        self.ndi_content.remove(source_id);
        self.ndi_settings_keys.remove(source_id);
        self.music_ended_reported.remove(source_id);
    }

    fn clear_playback_layers(&mut self, source_id: &str) {
        if let Some(fade) = self.music_crossfades.get_mut(source_id) {
            fade.native = None;
        }
        self.remove_layer(source_id, PlaybackLayer::Media);
        self.remove_layer(source_id, PlaybackLayer::Music);
        // Keep the device stream open between media selections. Dropping a
        // DeviceSink synchronously can stall the output update that replaces
        // a video with a static image.
    }

    fn remove_ndi_decoder_source(&mut self, source_id: &str) {
        if let Some(fade) = self.music_crossfades.get_mut(source_id) {
            fade.decoder = None;
        }
        for layer in [PlaybackLayer::Media, PlaybackLayer::Music] {
            let key = track_key(source_id, layer);
            self.ndi_decoders.remove(&key);
            self.ndi_decoder_errors.remove(&key);
        }
    }

    /// Reset queued native PCM for an output lifecycle transition. The tap is
    /// lock-free on the audio side, so resetting only advances a generation;
    /// stale blocks are discarded by the ticker without touching audio state.
    pub(crate) fn reset_ndi_audio(&mut self, source_id: &str) {
        for layer in [PlaybackLayer::Media, PlaybackLayer::Music] {
            if let Some(track) = self.tracks.get_mut(&track_key(source_id, layer)) {
                track.ndi_tap.reset();
                track.ndi_pending.clear();
                track.ndi_generation = track.ndi_tap.generation();
            }
            if let Some(track) = self.ndi_decoders.get_mut(&track_key(source_id, layer)) {
                track.reset();
            }
        }
    }

    pub(crate) fn ndi_tap_dropped_blocks(&self) -> u64 {
        self.tracks
            .values()
            .map(|track| track.ndi_tap.dropped_blocks())
            .sum()
    }

    fn sync_ndi_settings(&mut self, source_id: &str, content: &Value) {
        let settings =
            normalize_audio_settings_value(content.get("audioSettings").unwrap_or(&Value::Null));
        let key = serde_json::to_string(&settings).unwrap_or_default();
        if self
            .ndi_settings_keys
            .insert(source_id.to_string(), key.clone())
            .is_some_and(|previous| previous == key)
        {
            return;
        }
        self.reset_ndi_audio(source_id);
    }

    /// Update a decoder that was already started by the NDI ticker. New
    /// fallback decoders are intentionally lazy: sources without an active
    /// NDI sender never pay the cost of opening or decoding their media.
    fn sync_existing_ndi_decoders(&mut self, source_id: &str, content: &Value) -> Option<String> {
        let settings =
            normalize_audio_settings_value(content.get("audioSettings").unwrap_or(&Value::Null));
        if !ndi_audio_enabled(&settings) {
            self.remove_ndi_decoder_source(source_id);
            return None;
        }
        let layer_state = [
            (
                PlaybackLayer::Media,
                local_media_path(content),
                playback_state(content, PlaybackLayer::Media),
                background_loops(content),
            ),
            (
                PlaybackLayer::Music,
                local_music_path(content),
                playback_state(content, PlaybackLayer::Music),
                music_loops(content),
            ),
        ];
        let mut errors = Vec::new();
        for (layer, path, state, looping) in layer_state {
            let key = track_key(source_id, layer);
            let has_live_tap = self.tracks.contains_key(&key);
            let Some(path) = path else {
                self.ndi_decoders.remove(&key);
                self.ndi_decoder_errors.remove(&key);
                continue;
            };
            let identity = ndi_content_decoder_identity(&path, looping, layer, content);
            if has_live_tap {
                // A sink tap is the single decode owner. Drop a stale
                // fallback before it can ever be mixed a second time.
                self.ndi_decoders.remove(&key);
                self.ndi_decoder_errors.remove(&key);
                continue;
            }
            if self
                .ndi_decoders
                .get(&key)
                .is_some_and(|track| track.identity != identity)
            {
                self.ndi_decoders.remove(&key);
            }
            if self
                .ndi_decoder_errors
                .get(&key)
                .is_some_and(|(error_identity, _)| error_identity != &identity)
            {
                self.ndi_decoder_errors.remove(&key);
            }
            if let Some(track) = self.ndi_decoders.get_mut(&key) {
                if layer == PlaybackLayer::Music && state.status == PlaybackStatus::Ended {
                    if track.ended {
                        track.status = PlaybackStatus::Ended;
                    }
                    continue;
                }
                let rate = playback_rate(content, layer);
                if track.rate != rate {
                    track.pending.clear();
                    track.next_block_at = None;
                    track.rate = rate;
                }
                if let Err(error) = track.apply_state(state) {
                    track.error = Some(error.clone());
                    errors.push(error);
                } else if let Some(error) = track.error.clone() {
                    errors.push(error);
                }
            }
            if let Some((error_identity, error)) = self.ndi_decoder_errors.get(&key) {
                if error_identity == &identity {
                    errors.push(error.clone());
                }
            }
        }
        errors.into_iter().next()
    }

    /// Open one lazy decoder when the NDI ticker proves that this source is
    /// actually being sent. All state comes from the latest canonical
    /// payload, so the decoder follows the same Rust playback clock as the
    /// sink-backed tap.
    fn ensure_ndi_decoder(&mut self, source_id: &str, layer: PlaybackLayer) {
        let key = track_key(source_id, layer);
        if self.tracks.contains_key(&key) || self.ndi_decoders.contains_key(&key) {
            return;
        }
        let Some(content) = self.ndi_content.get(source_id).cloned() else {
            return;
        };
        let settings =
            normalize_audio_settings_value(content.get("audioSettings").unwrap_or(&Value::Null));
        if !ndi_audio_enabled(&settings) {
            return;
        }
        let path = match layer {
            PlaybackLayer::Media => local_media_path(&content),
            PlaybackLayer::Music => local_music_path(&content),
        };
        let Some(path) = path else {
            return;
        };
        let looping = match layer {
            PlaybackLayer::Media => background_loops(&content),
            PlaybackLayer::Music => music_loops(&content),
        };
        let identity = ndi_content_decoder_identity(&path, looping, layer, &content);
        if self
            .ndi_decoder_errors
            .get(&key)
            .is_some_and(|(error_identity, _)| error_identity == &identity)
        {
            return;
        }
        let mut state = playback_state(&content, layer);
        if layer == PlaybackLayer::Music && state.status == PlaybackStatus::Ended {
            state.status = PlaybackStatus::Playing;
        }
        if state.status != PlaybackStatus::Playing {
            return;
        }
        match NdiDecoderTrack::open(&path, looping, state) {
            Ok(mut track) => {
                if layer == PlaybackLayer::Music {
                    match music_loop_limit(&content, track.duration) {
                        Ok(limit) => track.loop_limit = limit,
                        Err(error) => {
                            self.ndi_decoder_errors.insert(key, (identity, error));
                            return;
                        }
                    }
                }
                track.identity = identity;
                track.rate = playback_rate(&content, layer);
                self.ndi_decoder_errors.remove(&key);
                self.ndi_decoders.insert(key, track);
            }
            Err(error) => {
                self.ndi_decoder_errors.insert(key, (identity, error));
            }
        }
    }

    /// Drain and mix the media and music taps for one source. This method only
    /// touches lock-free tap queues and allocates on the ticker thread; it
    /// never calls NDI or enters an audio callback.
    pub(crate) fn drain_ndi_audio(&mut self, source_id: &str) -> Vec<NativePcmBlock> {
        self.drain_ndi_audio_at(source_id, Instant::now())
    }

    fn drain_ndi_audio_at(&mut self, source_id: &str, now: Instant) -> Vec<NativePcmBlock> {
        let media_key = track_key(source_id, PlaybackLayer::Media);
        let music_key = track_key(source_id, PlaybackLayer::Music);
        self.fill_ndi_pending_for_layer(source_id, PlaybackLayer::Media, now);
        self.fill_ndi_pending_for_layer(source_id, PlaybackLayer::Music, now);
        let mut media = self.take_ndi_pending(&media_key);
        let mut music = self.take_ndi_pending(&music_key);
        let content = self.ndi_content.get(source_id);
        for (blocks, field) in [
            (&mut media, "backgroundAudioRouting"),
            (&mut music, "musicAudioRouting"),
        ] {
            let routing = content
                .and_then(|content| content.get(field))
                .filter(|value| !value.is_null())
                .and_then(|value| asset_audio_routing(value).ok());
            if let Some((routing, gain)) = routing {
                apply_asset_routing(blocks, &routing, gain);
            }
        }
        let music_before = pending_audio_seconds(&music);
        let mixed = self
            .mix_crossfade_audio(source_id, &mut media, &mut music, now)
            .unwrap_or_else(|| mix_ndi_pending(&mut media, &mut music, NDI_MAX_BLOCKS_PER_DRAIN));
        let mut live = self.inputs.drain(source_id, now);
        let mixed = if live.is_empty() {
            mixed
        } else {
            let mut mixed: VecDeque<_> = mixed.into();
            mix_ndi_pending(&mut mixed, &mut live, NDI_MAX_BLOCKS_PER_DRAIN)
        };
        let music_after = pending_audio_seconds(&music);
        if music_after < music_before && !self.tracks.contains_key(&music_key) {
            if let (Some(track), Some(content)) = (
                self.ndi_decoders.get(&music_key),
                self.ndi_content.get(source_id),
            ) {
                if track.status == PlaybackStatus::Playing {
                    if let Some(selection) = content
                        .get("audioSelectionId")
                        .filter(|value| !value.is_null())
                    {
                        if !self.has_main_music_clock(&track.path, selection) {
                            let position = (track.position_seconds()
                                - music_after * f64::from(track.rate))
                            .max(0.0);
                            crate::native_render::publish_loop_marker_position(
                                &track.path,
                                &audio_marker_selection(selection),
                                position,
                                track.loop_iteration,
                            );
                        }
                    }
                }
            }
        }
        self.put_ndi_pending(&media_key, media);
        self.put_ndi_pending(&music_key, music);
        mixed
    }

    fn fill_ndi_pending_for_layer(&mut self, source_id: &str, layer: PlaybackLayer, now: Instant) {
        let key = track_key(source_id, layer);
        if let Some(track) = self.tracks.get_mut(&key) {
            fill_ndi_pending(track);
            return;
        }
        self.ensure_ndi_decoder(source_id, layer);
        if let Some(track) = self.ndi_decoders.get_mut(&key) {
            fill_ndi_decoder_pending(track, now);
        }
    }

    fn take_ndi_pending(&mut self, key: &str) -> VecDeque<NativePcmBlock> {
        if let Some(track) = self.tracks.get_mut(key) {
            return std::mem::take(&mut track.ndi_pending);
        }
        self.ndi_decoders
            .get_mut(key)
            .map(|track| std::mem::take(&mut track.pending))
            .unwrap_or_default()
    }

    fn put_ndi_pending(&mut self, key: &str, pending: VecDeque<NativePcmBlock>) {
        if let Some(track) = self.tracks.get_mut(key) {
            track.ndi_pending = pending;
        } else if let Some(track) = self.ndi_decoders.get_mut(key) {
            track.pending = pending;
        }
    }

    fn sync_layer(
        &mut self,
        source_id: &str,
        layer: PlaybackLayer,
        path: &Path,
        state: PlaybackState,
        gain: f32,
        delay: Duration,
        looping: bool,
        loop_settings: Option<&Value>,
        routing: AudioRouting,
        asset_routing: Option<&Value>,
        rate: f32,
        selection_id: Option<&Value>,
    ) -> Result<Option<f64>, String> {
        let gain = if layer == PlaybackLayer::Music {
            gain * self
                .music_crossfades
                .get(source_id)
                .map(crossfade::MusicCrossfade::fraction)
                .unwrap_or(1.)
        } else {
            gain
        };
        let estimated_music_end =
            layer == PlaybackLayer::Music && state.status == PlaybackStatus::Ended;
        let state = PlaybackState {
            status: if estimated_music_end {
                PlaybackStatus::Playing
            } else {
                state.status
            },
            position: state.position / f64::from(rate),
        };
        let (asset_routing, asset_gain) = match asset_routing {
            Some(value) => asset_audio_routing(value)?,
            None => (audio_routing(&Value::Null), 1.0),
        };
        let key = track_key(source_id, layer);
        let identity = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            path.to_string_lossy(),
            looping,
            delay.as_millis(),
            layer.as_str(),
            routing.signature(),
            asset_routing.signature(),
            asset_gain,
            rate,
            selection_id.unwrap_or(&Value::Null),
            loop_settings
                .and_then(|value| value.get("musicLoopCount"))
                .unwrap_or(&Value::Null),
            loop_settings
                .and_then(|value| value.get("musicLoopTime"))
                .unwrap_or(&Value::Null),
        );
        let needs_reload = self.tracks.get(&key).is_none_or(|track| {
            track.identity != identity
                || (layer == PlaybackLayer::Media
                    && track.player.empty()
                    && state.status == PlaybackStatus::Playing)
        });

        if !needs_reload {
            let track = self.tracks.get_mut(&key).expect("track exists");
            track.player.set_volume(gain);
            if estimated_music_end {
                return Ok(track.duration);
            }
            reconcile_player(track, state);
            return Ok(track.duration);
        }

        self.remove_layer(source_id, layer);
        let decoder: Box<dyn Source + Send> = if layer == PlaybackLayer::Media {
            let Some(source) = preview::VideoAudio::open_media(path, 0., looping)? else {
                return native_source_duration(layer, path, None);
            };
            Box::new(source)
        } else {
            let file = File::open(path).map_err(|error| {
                format!(
                    "Cannot open native music audio '{}': {error}",
                    path.display()
                )
            })?;
            Box::new(Decoder::try_from(file).map_err(|error| {
                format!(
                    "Cannot decode native music audio '{}': {error}",
                    path.display()
                )
            })?)
        };
        let duration = native_source_duration(
            layer,
            path,
            decoder.total_duration().map(|value| value.as_secs_f64()),
        )?;
        let loop_limit = match loop_settings {
            Some(settings) => music_loop_limit(settings, duration)?,
            None => None,
        };
        let (tap_writer, tap_reader) = PcmTap::new(
            decoder.channels().get() as usize,
            (decoder.sample_rate().get() as f32 * rate).max(1.0) as u32,
        );
        let player = {
            let sink = self
                .sink
                .as_ref()
                .ok_or_else(|| "Native audio output device is unavailable.".to_string())?;
            Player::connect_new(sink.mixer())
        };
        player.set_volume(gain);
        // Keep a replacement source silent until its canonical start position
        // has been applied. Otherwise a failing/non-zero seek can leak samples
        // from the beginning before the temporary player is dropped.
        player.pause();
        if looping && loop_limit.is_some() {
            let source = BoundedLoopSource::new(
                decoder,
                duration.ok_or("Audio loop count requires a known duration.")?,
                loop_limit.unwrap(),
            )?;
            let tapped = TappedSource::new(source.speed(rate), tap_writer);
            let asset = RoutedSource::new(tapped, asset_routing).amplify(asset_gain);
            let routed = RoutedSource::new(asset, routing).metered();
            if delay.is_zero() {
                player.append(routed);
            } else {
                player.append(routed.delay(delay));
            }
        } else if looping && layer == PlaybackLayer::Music {
            let tapped = TappedSource::new(decoder.repeat_infinite().speed(rate), tap_writer);
            let asset = RoutedSource::new(tapped, asset_routing).amplify(asset_gain);
            let routed = RoutedSource::new(asset, routing).metered();
            if delay.is_zero() {
                player.append(routed);
            } else {
                player.append(routed.delay(delay));
            }
        } else {
            let tapped = TappedSource::new(decoder.speed(rate), tap_writer);
            let asset = RoutedSource::new(tapped, asset_routing).amplify(asset_gain);
            let routed = RoutedSource::new(asset, routing).metered();
            if delay.is_zero() {
                player.append(routed);
            } else {
                player.append(routed.delay(delay));
            }
        }
        let mut track = NativeAudioTrack {
            identity,
            player,
            looping,
            duration,
            loop_limit,
            rate,
            delay,
            status: PlaybackStatus::Paused,
            ndi_tap: tap_reader,
            ndi_pending: VecDeque::new(),
            ndi_generation: 0,
        };

        // Do not register a layer whose canonical non-zero start position could
        // not be reached. Registering it first would make status report active
        // audio while the player is actually still at the beginning of the file.
        if state.position > 0.0 {
            validate_native_main_seek(
                layer,
                path,
                state.position,
                track
                    .player
                    .try_seek(Duration::from_secs_f64(state.position)),
            )?;
        }

        reconcile_player(&mut track, state);
        self.tracks.insert(key, track);
        Ok(duration)
    }

    /// Keep the clock-aligned sync beep in the media slot so main and NDI
    /// outputs receive it through the same routing, gain and delay.
    fn sync_beep(&mut self, source_id: &str, gain: f32, delay: Duration, routing: AudioRouting) {
        let key = track_key(source_id, PlaybackLayer::Media);
        let identity = format!("sync-beep|{}|{}", delay.as_millis(), routing.signature());
        if let Some(track) = self
            .tracks
            .get_mut(&key)
            .filter(|track| track.identity == identity)
        {
            track.player.set_volume(gain);
            return;
        }
        self.remove_layer(source_id, PlaybackLayer::Media);
        let Some(sink) = self.sink.as_ref() else {
            return;
        };
        let beep = SyncBeepSource::new(crate::test_pattern::clock_seconds());
        let (tap_writer, tap_reader) = PcmTap::new(2, SyncBeepSource::SAMPLE_RATE);
        let player = Player::connect_new(sink.mixer());
        player.set_volume(gain);
        let routed = RoutedSource::new(TappedSource::new(beep, tap_writer), routing);
        if delay.is_zero() {
            player.append(routed);
        } else {
            player.append(routed.delay(delay));
        }
        player.play();
        self.tracks.insert(
            key,
            NativeAudioTrack {
                identity,
                player,
                looping: false,
                duration: None,
                loop_limit: None,
                rate: 1.0,
                delay,
                status: PlaybackStatus::Playing,
                ndi_tap: tap_reader,
                ndi_pending: VecDeque::new(),
                ndi_generation: 0,
            },
        );
    }

    fn ensure_sink(&mut self, requested_device: &str, output_channels: u16) -> Result<(), String> {
        let device_key = canonical_device_key(requested_device);
        let device_key = format!("{device_key}:{output_channels}");
        if self.sink.is_some() && self.device_key == device_key {
            return Ok(());
        }
        self.sink = None;
        self.tracks.clear();
        self.music_crossfades.clear();
        let sink = open_sink(requested_device, output_channels)?;
        self.sink = Some(sink);
        self.device_key = device_key;
        Ok(())
    }

    fn remove_layer(&mut self, source_id: &str, layer: PlaybackLayer) {
        self.tracks.remove(&track_key(source_id, layer));
    }
}

#[derive(Clone, Copy)]
struct PlaybackState {
    status: PlaybackStatus,
    position: f64,
}

fn playback_state(content: &Value, layer: PlaybackLayer) -> PlaybackState {
    let key = match layer {
        PlaybackLayer::Media => "backgroundPlaybackState",
        PlaybackLayer::Music => "musicPlaybackState",
    };
    let state = content.get(key).and_then(Value::as_object);
    let state_status = state
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str);
    let transport_paused = content
        .get("transportPlaybackPaused")
        .and_then(Value::as_object)
        .and_then(|value| value.get(layer.as_str()))
        .and_then(Value::as_bool);
    let status = if state
        .and_then(|value| value.get("shuttle"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        PlaybackStatus::Paused
    } else if state
        .and_then(|value| value.get("delayed"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        PlaybackStatus::Paused
    } else if state_status == Some("ended") {
        PlaybackStatus::Ended
    } else if let Some(paused) = transport_paused {
        if paused {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        }
    } else if state_status == Some("paused") {
        PlaybackStatus::Paused
    } else {
        PlaybackStatus::Playing
    };
    let position = state
        .and_then(|value| value.get("currentTime"))
        .and_then(|value| number(Some(value)))
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        .max(0.0);
    PlaybackState { status, position }
}

fn playback_rate(content: &Value, layer: PlaybackLayer) -> f32 {
    let key = match layer {
        PlaybackLayer::Media => "backgroundPlaybackState",
        PlaybackLayer::Music => "musicPlaybackState",
    };
    content
        .get(key)
        .and_then(|state| state.get("rate"))
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .abs() as f32
}

fn is_status_only_transition(previous: PlaybackStatus, next: PlaybackStatus) -> bool {
    matches!(
        (previous, next),
        (PlaybackStatus::Playing, PlaybackStatus::Paused)
            | (PlaybackStatus::Paused, PlaybackStatus::Playing)
    )
}

fn should_seek_native_player(
    previous_status: PlaybackStatus,
    current_position: f64,
    state: PlaybackState,
) -> bool {
    !is_status_only_transition(previous_status, state.status)
        && (current_position - state.position).abs() > MAX_DRIFT_SECONDS
}

fn reconcile_player(track: &mut NativeAudioTrack, state: PlaybackState) {
    if state.status == PlaybackStatus::Ended {
        track.player.pause();
        if let Some(duration) = track.duration {
            let _ = track
                .player
                .try_seek(Duration::from_secs_f64(duration / f64::from(track.rate)));
        }
        track.status = state.status;
        return;
    }
    let current = track.player.get_pos().as_secs_f64();
    if should_seek_native_player(track.status, current, state) {
        let _ = track
            .player
            .try_seek(Duration::from_secs_f64(state.position));
    }
    if state.status == PlaybackStatus::Paused {
        track.player.pause();
    } else {
        track.player.play();
    }
    track.status = state.status;
}

fn native_main_seek_error(
    layer: PlaybackLayer,
    path: &Path,
    position: f64,
    error: SeekError,
) -> String {
    format!(
        "Cannot seek native {} audio '{}' to {position:.3}s: {error}",
        layer.as_str(),
        path.display()
    )
}

fn validate_native_main_seek(
    layer: PlaybackLayer,
    path: &Path,
    position: f64,
    result: Result<(), SeekError>,
) -> Result<(), String> {
    result.map_err(|error| native_main_seek_error(layer, path, position, error))
}

/// The names of the audio output devices, as `[{id, label}]`. Opening a sink resolves a device by this name, so
/// the name is both the id and the label. Nothing is opened here.
pub(crate) fn list_output_devices() -> Result<Value, String> {
    let devices = cpal::default_host()
        .output_devices()
        .map_err(|error| format!("Cannot enumerate audio output devices: {error}"))?
        .map(|device| {
            let name = device
                .description()
                .map_err(|error| error.to_string())?
                .name()
                .to_string();
            Ok(json!({"id": name, "label": name}))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Value::Array(devices))
}

fn open_sink(requested_device: &str, output_channels: u16) -> Result<MixerDeviceSink, String> {
    let channel_count = ChannelCount::new(output_channels)
        .ok_or_else(|| "Native audio routing requested zero output channels.".to_string())?;
    let requested = requested_device.trim();
    let mut sink = if requested.is_empty()
        || requested.eq_ignore_ascii_case(DEFAULT_DEVICE)
        || requested.eq_ignore_ascii_case("default")
    {
        DeviceSinkBuilder::from_default_device()
            .map_err(|error| format!("Cannot select the default audio output: {error}"))?
            .with_channels(channel_count)
            .open_stream()
            .map_err(|error| {
                format!(
                    "Cannot open the default audio output with {output_channels} channels: {error}"
                )
            })?
    } else {
        let requested_lower = requested.to_ascii_lowercase();
        let device = cpal::default_host()
            .output_devices()
            .map_err(|error| format!("Cannot enumerate audio output devices: {error}"))?
            .find(|device| {
                device
                    .description()
                    .map(|description| {
                        let name = description.name().to_ascii_lowercase();
                        name == requested_lower || name.contains(&requested_lower)
                    })
                    .unwrap_or(false)
            })
            .ok_or_else(|| format!("Audio output device '{requested}' is unavailable."))?;
        DeviceSinkBuilder::from_device(device)
            .map_err(|error| format!("Cannot select audio output device '{requested}': {error}"))?
            .with_channels(channel_count)
            .open_stream()
            .map_err(|error| {
                format!(
                    "Cannot open audio output device '{requested}' with {output_channels} channels: {error}"
                )
            })?
    };
    sink.log_on_drop(false);
    Ok(sink)
}

/// The device and channel count the main audio settings route to, or `None`
/// when main audio is disabled or set to no device.
pub(crate) fn main_audio_target(main: &Value) -> Option<(String, u16)> {
    let device = selected_device(main);
    let enabled = main.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    (enabled && !device.eq_ignore_ascii_case(NONE_DEVICE))
        .then(|| (device, audio_routing(main).output_channels))
}

pub(crate) fn open_main_audio_sink(device: &str, channels: u16) -> Result<MixerDeviceSink, String> {
    open_sink(device, channels)
}

pub(crate) fn main_audio_sink_key(device: &str, channels: u16) -> String {
    format!("{}:{channels}", canonical_device_key(device))
}

/// A paused player for a timeline soundtrack on the main output, routed, leveled
/// and delayed like other main-output audio.
pub(crate) fn connect_soundtrack_player(
    sink: &MixerDeviceSink,
    path: &Path,
    main: &Value,
    reverse: bool,
) -> Result<(Player, Option<f64>, Option<Arc<Mutex<Option<String>>>>), String> {
    let file = File::open(path)
        .map_err(|error| format!("Cannot open soundtrack '{}': {error}", path.display()))?;
    let decoder = Decoder::try_from(file)
        .map_err(|error| format!("Cannot decode soundtrack '{}': {error}", path.display()))?;
    let duration = decoder.total_duration().map(|value| value.as_secs_f64());
    let player = Player::connect_new(sink.mixer());
    player.set_volume(gain_from_settings(main));
    player.pause();
    let routing = audio_routing(main);
    let delay = delay_from_settings(main);
    let reverse_error = if reverse {
        let duration =
            duration.ok_or("Reverse soundtrack playback requires a known audio duration.")?;
        let source = ReverseAudioSource::new(decoder, Duration::from_secs_f64(duration))?;
        let error = source.error.clone();
        let routed = RoutedSource::new(source, routing).metered();
        if delay.is_zero() {
            player.append(routed);
        } else {
            player.append(routed.delay(delay));
        }
        Some(error)
    } else {
        let routed = RoutedSource::new(decoder, routing).metered();
        if delay.is_zero() {
            player.append(routed);
        } else {
            player.append(routed.delay(delay));
        }
        None
    };
    Ok((player, duration, reverse_error))
}

const REVERSE_AUDIO_WINDOW_SECONDS: u64 = 2;
const REVERSE_AUDIO_QUEUED_WINDOWS: usize = 4;
const REVERSE_AUDIO_MAX_CHUNK_SAMPLES: usize = 1_048_576;

enum ReverseAudioCommand {
    Seek {
        frame: u64,
        generation: u64,
        complete: std::sync::mpsc::Sender<Result<(), String>>,
    },
    Stop,
}

struct ReverseAudioChunk {
    generation: u64,
    samples: Vec<f32>,
    finished: bool,
}

struct ReverseAudioSource {
    chunks: std::sync::mpsc::Receiver<ReverseAudioChunk>,
    commands: std::sync::mpsc::Sender<ReverseAudioCommand>,
    worker: Option<std::thread::JoinHandle<()>>,
    current: std::vec::IntoIter<f32>,
    finished: bool,
    duration: Duration,
    generation: u64,
    channels: ChannelCount,
    sample_rate: rodio::SampleRate,
    error: Arc<Mutex<Option<String>>>,
}

impl ReverseAudioSource {
    fn new(decoder: Decoder<BufReader<File>>, duration: Duration) -> Result<Self, String> {
        let channels = decoder.channels();
        let sample_rate = decoder.sample_rate();
        let (chunk_sender, chunks) = std::sync::mpsc::sync_channel(REVERSE_AUDIO_QUEUED_WINDOWS);
        let (commands, command_receiver) = std::sync::mpsc::channel();
        let error = Arc::new(Mutex::new(None));
        let worker_error = Arc::clone(&error);
        let worker = std::thread::Builder::new()
            .name("presenter-orbit-reverse-soundtrack".into())
            .spawn(move || {
                decode_reverse_audio(
                    decoder,
                    duration,
                    channels.get() as usize,
                    sample_rate.get(),
                    chunk_sender,
                    command_receiver,
                    worker_error,
                );
            })
            .map_err(|error| format!("Reverse soundtrack decoder could not start: {error}"))?;
        Ok(Self {
            chunks,
            commands,
            worker: Some(worker),
            current: Vec::new().into_iter(),
            finished: false,
            duration,
            generation: 0,
            channels,
            sample_rate,
            error,
        })
    }
}

fn decode_reverse_audio(
    mut decoder: Decoder<BufReader<File>>,
    duration: Duration,
    channels: usize,
    sample_rate: u32,
    chunks: std::sync::mpsc::SyncSender<ReverseAudioChunk>,
    commands: std::sync::mpsc::Receiver<ReverseAudioCommand>,
    error: Arc<Mutex<Option<String>>>,
) {
    use std::sync::mpsc::{RecvTimeoutError, TryRecvError, TrySendError};

    let duration_frames = (duration.as_secs_f64() * f64::from(sample_rate)).round() as u64;
    let max_window_frames = (REVERSE_AUDIO_MAX_CHUNK_SAMPLES / channels).max(1) as u64;
    let window_frames =
        (u64::from(sample_rate) * REVERSE_AUDIO_WINDOW_SECONDS).min(max_window_frames);
    let mut cursor = duration_frames;
    let mut generation = 0;
    let mut acknowledgement = None;
    let mut pending_command = None;
    'decode: loop {
        let command = pending_command
            .take()
            .or_else(|| match commands.try_recv() {
                Ok(command) => Some(command),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(ReverseAudioCommand::Stop),
            });
        match command {
            Some(ReverseAudioCommand::Seek {
                frame,
                generation: seek_generation,
                complete,
            }) => {
                cursor = frame.min(duration_frames);
                generation = seek_generation;
                acknowledgement = Some(complete);
            }
            Some(ReverseAudioCommand::Stop) => break,
            None => {}
        }
        if cursor == 0 {
            if let Some(complete) = acknowledgement.take() {
                let _ = complete.send(Ok(()));
            }
            let mut chunk = ReverseAudioChunk {
                generation,
                samples: Vec::new(),
                finished: true,
            };
            loop {
                match chunks.try_send(chunk) {
                    Ok(()) => break,
                    Err(TrySendError::Full(full)) => {
                        chunk = full;
                        match commands.recv_timeout(Duration::from_millis(10)) {
                            Ok(command) => {
                                pending_command = Some(command);
                                continue 'decode;
                            }
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => break 'decode,
                        }
                    }
                    Err(TrySendError::Disconnected(_)) => break 'decode,
                }
            }
            pending_command = match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            };
            continue;
        }
        let start = cursor.saturating_sub(window_frames);
        let start_seconds = start as f64 / f64::from(sample_rate);
        if let Err(seek_error) = decoder.try_seek(Duration::from_secs_f64(start_seconds)) {
            let message = format!("Reverse soundtrack seek failed: {seek_error}");
            *error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(message.clone());
            if let Some(complete) = acknowledgement.take() {
                let _ = complete.send(Err(message));
            }
            break;
        }
        if let Some(complete) = acknowledgement.take() {
            let _ = complete.send(Ok(()));
        }
        let sample_count = (cursor - start) as usize * channels;
        let mut samples = Vec::with_capacity(sample_count);
        samples.extend(decoder.by_ref().take(sample_count));
        if samples.len() % channels != 0 {
            samples.truncate(samples.len() / channels * channels);
            *error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some("Reverse soundtrack decoder returned an incomplete audio frame.".into());
        }
        if samples.len() != sample_count {
            *error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some("Reverse soundtrack decoder ended before the requested audio window.".into());
        }
        for frame in 0..(samples.len() / channels) / 2 {
            let other = samples.len() / channels - frame - 1;
            for channel in 0..channels {
                samples.swap(frame * channels + channel, other * channels + channel);
            }
        }
        cursor = start;
        let mut chunk = ReverseAudioChunk {
            generation,
            samples,
            finished: false,
        };
        loop {
            match chunks.try_send(chunk) {
                Ok(()) => continue 'decode,
                Err(TrySendError::Full(full)) => {
                    chunk = full;
                    match commands.recv_timeout(Duration::from_millis(10)) {
                        Ok(command) => {
                            pending_command = Some(command);
                            continue 'decode;
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break 'decode,
                    }
                }
                Err(TrySendError::Disconnected(_)) => break 'decode,
            }
        }
    }
}

impl Iterator for ReverseAudioSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.finished {
                return None;
            }
            if let Some(sample) = self.current.next() {
                return Some(sample);
            }
            let chunk = match self.chunks.recv() {
                Ok(chunk) => chunk,
                Err(_) => {
                    *self
                        .error
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some("Reverse soundtrack decoder stopped unexpectedly.".into());
                    return None;
                }
            };
            if chunk.generation != self.generation {
                continue;
            }
            if chunk.finished {
                self.finished = true;
                return None;
            }
            self.current = chunk.samples.into_iter();
        }
    }
}

impl Source for ReverseAudioSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(self.duration)
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        let media_position = self.duration.saturating_sub(position);
        let frame =
            (media_position.as_secs_f64() * f64::from(self.sample_rate.get())).round() as u64;
        while self.chunks.try_recv().is_ok() {}
        self.current = Vec::new().into_iter();
        self.finished = false;
        self.generation = self.generation.saturating_add(1);
        let (complete, result) = std::sync::mpsc::channel();
        self.commands
            .send(ReverseAudioCommand::Seek {
                frame,
                generation: self.generation,
                complete,
            })
            .map_err(|_| SeekError::NotSupported {
                underlying_source: "ReverseAudioSource",
            })?;
        result
            .recv()
            .map_err(|_| SeekError::NotSupported {
                underlying_source: "ReverseAudioSource",
            })?
            .map_err(|_| SeekError::NotSupported {
                underlying_source: "ReverseAudioSource",
            })?;
        Ok(())
    }
}

impl Drop for ReverseAudioSource {
    fn drop(&mut self) {
        let _ = self.commands.send(ReverseAudioCommand::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn selected_device(main: &Value) -> String {
    let configured = string_value(main.get("device")).unwrap_or_else(|| DEFAULT_DEVICE.to_string());
    if configured.eq_ignore_ascii_case(NONE_DEVICE)
        || configured.eq_ignore_ascii_case(DEFAULT_DEVICE)
        || configured.eq_ignore_ascii_case("default")
    {
        return configured;
    }
    string_value(main.get("deviceLabel")).unwrap_or(configured)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AudioRouting {
    output_channels: u16,
    routes: Vec<AudioRoute>,
    /// Whether a route reads the last available channel when its source channel is absent,
    /// so mono sources reach both logical channels. Authored asset routes read silence instead.
    upmix_missing_sources: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct AudioRoute {
    source_index: usize,
    gain: f32,
    destinations: Vec<bool>,
    mute: bool,
    solo: bool,
    tone: bool,
    compress_limit: bool,
}

/// ProPresenter's logical audio buses, which main-device and SDI/NDI routing can address.
const LOGICAL_CHANNELS: u16 = crate::audio_input_config::MAX_AUDIO_INPUT_BUSES as u16;
/// The most source channels a per-asset route can address.
const MAX_SOURCE_CHANNELS: u64 = 8;
const LIMIT_THRESHOLD: f32 = 0.8;

/// ProPresenter's per-channel Compress/Limit: samples below the threshold pass unchanged and
/// louder ones are compressed smoothly towards full scale, which they never exceed.
fn compress_limit(sample: f32) -> f32 {
    let level = sample.abs();
    if level <= LIMIT_THRESHOLD {
        return sample;
    }
    let headroom = 1.0 - LIMIT_THRESHOLD;
    sample.signum() * (LIMIT_THRESHOLD + headroom * ((level - LIMIT_THRESHOLD) / headroom).tanh())
}

impl AudioRouting {
    fn signature(&self) -> String {
        format!("{self:?}")
    }

    fn has_solo(&self) -> bool {
        self.routes.iter().any(|route| route.solo)
    }

    fn render_frame(&self, input_frame: &[f32], sample_rate: f32, frame_position: u64) -> Vec<f32> {
        let has_solo = self.has_solo();
        let mut output = vec![0.0; self.output_channels as usize];
        for route in &self.routes {
            if route.mute || (has_solo && !route.solo) {
                continue;
            }
            let source_sample = match input_frame.get(route.source_index) {
                Some(sample) => *sample,
                None if self.upmix_missing_sources && route.source_index < 2 => {
                    input_frame[input_frame.len() - 1]
                }
                None => continue,
            };
            let tone_sample = if route.tone {
                let frequency = TONE_FREQUENCIES_HZ[route.source_index.min(1)];
                let phase =
                    2.0 * std::f32::consts::PI * frequency * frame_position as f32 / sample_rate;
                TONE_GAIN * phase.sin()
            } else {
                0.0
            };
            let mut sample = (source_sample + tone_sample) * route.gain;
            if route.compress_limit {
                sample = compress_limit(sample);
            }
            for (output_index, enabled) in route.destinations.iter().take(output.len()).enumerate()
            {
                if *enabled {
                    output[output_index] += sample;
                }
            }
        }
        output
    }
}

pub(crate) fn audio_routing(main: &Value) -> AudioRouting {
    let routing = main.get("routing").and_then(Value::as_object);
    let output_channels = number(routing.and_then(|value| value.get("deviceChannels")))
        .filter(|value| value.is_finite())
        .map(|value| value.round() as i64)
        .unwrap_or(2)
        .clamp(1, i64::from(MAX_OUTPUT_CHANNELS)) as u16;
    let pro_channels = routing
        .and_then(|value| value.get("proChannels"))
        .and_then(Value::as_array);
    let routes = (0..LOGICAL_CHANNELS)
        .map(|source_index| {
            let id = format!("pro-{}", source_index + 1);
            let channel = pro_channels.and_then(|channels| {
                channels
                    .iter()
                    .find(|value| value.get("id").and_then(Value::as_str) == Some(id.as_str()))
            });
            let channel_map = channel
                .and_then(|value| value.get("channels"))
                .and_then(Value::as_object);
            let modes = channel
                .and_then(|value| value.get("modes"))
                .and_then(Value::as_object);
            let destinations = (0..output_channels)
                .map(|channel_index| {
                    let channel_key = (channel_index + 1).to_string();
                    channel_map
                        .and_then(|value| value.get(&channel_key))
                        .and_then(Value::as_bool)
                        .unwrap_or(source_index < 2 && channel_index == source_index)
                })
                .collect();
            AudioRoute {
                source_index: source_index as usize,
                gain: 1.0,
                destinations,
                mute: modes
                    .and_then(|value| value.get("mute"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                solo: modes
                    .and_then(|value| value.get("solo"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                tone: modes
                    .and_then(|value| value.get("tone"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                compress_limit: false,
            }
        })
        .collect();
    AudioRouting {
        output_channels,
        routes,
        upmix_missing_sources: true,
    }
}

pub fn validate_audio_routing(value: &Value) -> Result<(), String> {
    asset_audio_routing(value).map(|_| ())
}

/// A per-asset route as ProPresenter's `Media.AudioProperties`: a master gain and, per source
/// channel, the logical channels it feeds plus its gain, mute, solo and Compress/Limit.
/// Unlisted source channels are silent; several sources feeding one channel are summed.
fn asset_audio_routing(value: &Value) -> Result<(AudioRouting, f32), String> {
    let object = value
        .as_object()
        .ok_or("Per-asset audio routing must be an object.")?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "gainDb" | "channels"))
    {
        return Err("Per-asset audio routing has an unknown field.".into());
    }
    let decibels = |value: Option<&Value>, label: &str| -> Result<f32, String> {
        let gain = value
            .and_then(Value::as_f64)
            .filter(|gain| gain.is_finite() && (-60.0..=12.0).contains(gain))
            .ok_or_else(|| format!("{label} must be between -60 and 12 dB."))?;
        Ok(10.0_f32.powf(gain as f32 / 20.0))
    };
    let gain = decibels(object.get("gainDb"), "Per-asset audio gain")?;
    let channels = object
        .get("channels")
        .and_then(Value::as_array)
        .filter(|channels| (1..=MAX_SOURCE_CHANNELS as usize).contains(&channels.len()))
        .ok_or("Per-asset audio routing requires one to eight source channels.")?;
    let flag = |channel: &Map<String, Value>, key: &str| -> Result<bool, String> {
        match channel.get(key) {
            None => Ok(false),
            Some(value) => value
                .as_bool()
                .ok_or_else(|| format!("Per-asset audio {key} must be a boolean.")),
        }
    };
    let mut sources = std::collections::HashSet::new();
    let mut output_channels = NDI_MIX_CHANNELS as u16;
    let mut routes = Vec::with_capacity(channels.len());
    for channel in channels {
        let channel = channel
            .as_object()
            .ok_or("Per-asset audio source channel must be an object.")?;
        if channel.keys().any(|key| {
            !matches!(
                key.as_str(),
                "sourceChannel" | "outputs" | "muted" | "solo" | "gainDb" | "compressLimit"
            )
        }) {
            return Err("Per-asset audio source channel has an unknown field.".into());
        }
        let source = channel
            .get("sourceChannel")
            .and_then(Value::as_u64)
            .filter(|source| (1..=MAX_SOURCE_CHANNELS).contains(source))
            .ok_or("Per-asset audio source channel must be 1 to 8.")?;
        if !sources.insert(source) {
            return Err("Per-asset audio source channels must be unique.".into());
        }
        let mut destinations = vec![false; LOGICAL_CHANNELS as usize];
        for output in channel
            .get("outputs")
            .and_then(Value::as_array)
            .ok_or("Per-asset audio source channel requires its outputs.")?
        {
            let output = output
                .as_u64()
                .filter(|output| (1..=NDI_MIX_CHANNELS as u64).contains(output))
                .ok_or("Per-asset audio output must be ProPresenter channel 1 or 2.")?;
            if std::mem::replace(&mut destinations[output as usize - 1], true) {
                return Err("Per-asset audio outputs must be unique.".into());
            }
            output_channels = output_channels.max(output as u16);
        }
        let mute = channel
            .get("muted")
            .and_then(Value::as_bool)
            .ok_or("Per-asset audio source channel must specify its mute state.")?;
        routes.push(AudioRoute {
            source_index: source as usize - 1,
            gain: match channel.get("gainDb") {
                None => 1.0,
                value => decibels(value, "Per-channel audio gain")?,
            },
            destinations,
            mute,
            solo: flag(channel, "solo")?,
            tone: false,
            compress_limit: flag(channel, "compressLimit")?,
        });
    }
    Ok((
        AudioRouting {
            output_channels,
            routes,
            upmix_missing_sources: false,
        },
        gain,
    ))
}

/// Route an interleaved PCM block with the same mapping used by native output
/// playback. This is shared by the NDI PCM processor so an eventual NDI tap
/// cannot drift from the main output's mute/solo/tone semantics.
pub(crate) fn route_interleaved_audio(
    main: &Value,
    samples: &[f32],
    input_channels: usize,
    sample_rate: u32,
    starting_frame: u64,
) -> Result<(u16, Vec<f32>), String> {
    if input_channels == 0 {
        return Err("Native audio routing requires at least one input channel.".to_string());
    }
    if sample_rate == 0 {
        return Err("Native audio routing requires a non-zero sample rate.".to_string());
    }
    if !samples.len().is_multiple_of(input_channels) {
        return Err("Native audio PCM block is not frame-aligned.".to_string());
    }
    let routing = audio_routing(main);
    let mut output =
        Vec::with_capacity(samples.len() / input_channels * routing.output_channels as usize);
    for (offset, frame) in samples.chunks_exact(input_channels).enumerate() {
        output.extend(routing.render_frame(
            frame,
            sample_rate as f32,
            starting_frame.saturating_add(offset as u64),
        ));
    }
    Ok((routing.output_channels, output))
}

/// One bounded, timestamped PCM packet crossing from a rodio source to the
/// native ticker. The packet is only created by the ticker-side queue reader;
/// the audio source writes into preallocated ring slots.
#[derive(Clone, Debug)]
pub(crate) struct NativePcmBlock {
    asset_routed: bool,
    preserve_channel_count: bool,
    pub(crate) samples: Vec<f32>,
    pub(crate) sample_rate: u32,
    pub(crate) channels: usize,
    timestamp: Instant,
    offset_frames: usize,
}

impl NativePcmBlock {
    fn frame_count(&self) -> usize {
        self.samples.len() / self.channels.max(1)
    }

    fn remaining_frames(&self) -> usize {
        self.frame_count().saturating_sub(self.offset_frames)
    }

    pub(crate) fn start_time(&self) -> Instant {
        self.timestamp
            .checked_add(Duration::from_secs_f64(
                self.offset_frames as f64 / self.sample_rate.max(1) as f64,
            ))
            .unwrap_or(self.timestamp)
    }

    fn end_time(&self) -> Instant {
        self.timestamp
            .checked_add(Duration::from_secs_f64(
                self.frame_count() as f64 / self.sample_rate.max(1) as f64,
            ))
            .unwrap_or(self.timestamp)
    }
}

struct PcmRingSlot {
    samples: UnsafeCell<Vec<f32>>,
    len: UnsafeCell<usize>,
    sample_rate: UnsafeCell<u32>,
    channels: UnsafeCell<usize>,
    timestamp: UnsafeCell<Instant>,
    generation: UnsafeCell<u64>,
}

unsafe impl Send for PcmRingSlot {}
unsafe impl Sync for PcmRingSlot {}

impl PcmRingSlot {
    fn new(max_samples: usize) -> Self {
        Self {
            samples: UnsafeCell::new(vec![0.0; max_samples]),
            len: UnsafeCell::new(0),
            sample_rate: UnsafeCell::new(0),
            channels: UnsafeCell::new(0),
            timestamp: UnsafeCell::new(Instant::now()),
            generation: UnsafeCell::new(0),
        }
    }
}

/// A single-producer/single-consumer ring. Each instance belongs to one rodio
/// source iterator (the producer) and one NDI ticker consumer. The release
/// store on `head` publishes a completely written slot; the acquire load on
/// `head` makes those fields visible before the consumer copies them. The
/// reverse ordering on `tail` prevents the producer from reusing a slot while
/// it is being read. It never allocates, waits, or takes a mutex on the audio
/// side; a full queue drops the tap block.
struct PcmRing {
    slots: Box<[PcmRingSlot]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    generation: AtomicU64,
    dropped_blocks: AtomicU64,
    max_samples: usize,
    preserve_channel_count: bool,
}

unsafe impl Send for PcmRing {}
unsafe impl Sync for PcmRing {}

impl PcmRing {
    fn new(channels: usize, preserve_channel_count: bool) -> Self {
        let max_samples = NDI_TAP_BLOCK_FRAMES.saturating_mul(channels.max(1));
        let slots = (0..NDI_TAP_QUEUE_CAPACITY)
            .map(|_| PcmRingSlot::new(max_samples))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            slots,
            mask: NDI_TAP_QUEUE_CAPACITY - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            generation: AtomicU64::new(0),
            dropped_blocks: AtomicU64::new(0),
            max_samples,
            preserve_channel_count,
        }
    }

    fn reset(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn dropped_blocks(&self) -> u64 {
        self.dropped_blocks.load(Ordering::Relaxed)
    }

    fn try_push(&self, samples: &[f32], sample_rate: u32, channels: usize, timestamp: Instant) {
        if samples.is_empty() || samples.len() > self.max_samples || channels == 0 {
            self.dropped_blocks.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= self.slots.len() {
            self.dropped_blocks.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let slot = &self.slots[head & self.mask];
        let generation = self.generation.load(Ordering::Acquire);
        unsafe {
            let slot_samples = &mut *slot.samples.get();
            slot_samples[..samples.len()].copy_from_slice(samples);
            *slot.len.get() = samples.len();
            *slot.sample_rate.get() = sample_rate;
            *slot.channels.get() = channels;
            *slot.timestamp.get() = timestamp;
            *slot.generation.get() = generation;
        }
        self.head.store(head.wrapping_add(1), Ordering::Release);
    }

    fn try_pop(&self) -> Option<NativePcmBlock> {
        loop {
            let tail = self.tail.load(Ordering::Relaxed);
            let head = self.head.load(Ordering::Acquire);
            if tail == head {
                return None;
            }
            let slot = &self.slots[tail & self.mask];
            let expected_generation = self.generation.load(Ordering::Acquire);
            let (len, sample_rate, channels, timestamp, generation, samples) = unsafe {
                let len = *slot.len.get();
                let slot_samples = &*slot.samples.get();
                let samples = slot_samples[..len].to_vec();
                (
                    len,
                    *slot.sample_rate.get(),
                    *slot.channels.get(),
                    *slot.timestamp.get(),
                    *slot.generation.get(),
                    samples,
                )
            };
            let current_generation = self.generation.load(Ordering::Acquire);
            self.tail.store(tail.wrapping_add(1), Ordering::Release);
            if generation != expected_generation || generation != current_generation {
                continue;
            }
            if len == 0 || channels == 0 || sample_rate == 0 {
                continue;
            }
            return Some(NativePcmBlock {
                asset_routed: false,
                preserve_channel_count: self.preserve_channel_count,
                samples,
                sample_rate,
                channels,
                timestamp,
                offset_frames: 0,
            });
        }
    }
}

#[derive(Clone)]
struct PcmTapReader {
    ring: Arc<PcmRing>,
}

impl PcmTapReader {
    fn try_pop(&self) -> Option<NativePcmBlock> {
        self.ring.try_pop()
    }

    fn reset(&self) {
        self.ring.reset();
    }

    fn generation(&self) -> u64 {
        self.ring.generation()
    }

    fn dropped_blocks(&self) -> u64 {
        self.ring.dropped_blocks()
    }
}

struct PcmTapWriter {
    ring: Arc<PcmRing>,
    channels: usize,
    sample_rate: u32,
    scratch: Vec<f32>,
    block_timestamp: Option<Instant>,
    finished: bool,
}

struct PcmTap;

impl PcmTap {
    fn new(channels: usize, sample_rate: u32) -> (PcmTapWriter, PcmTapReader) {
        Self::new_with_layout(channels, sample_rate, false)
    }

    fn new_input(channels: usize, sample_rate: u32) -> (PcmTapWriter, PcmTapReader) {
        Self::new_with_layout(channels, sample_rate, true)
    }

    fn new_with_layout(
        channels: usize,
        sample_rate: u32,
        preserve_channel_count: bool,
    ) -> (PcmTapWriter, PcmTapReader) {
        let channels = channels.max(1);
        let ring = Arc::new(PcmRing::new(channels, preserve_channel_count));
        (
            PcmTapWriter {
                ring: ring.clone(),
                channels,
                sample_rate: sample_rate.max(1),
                scratch: Vec::with_capacity(NDI_TAP_BLOCK_FRAMES * channels),
                block_timestamp: None,
                finished: false,
            },
            PcmTapReader { ring },
        )
    }
}

impl PcmTapWriter {
    fn push(&mut self, sample: f32) {
        self.push_at(sample, self.block_timestamp.unwrap_or_else(Instant::now));
    }

    fn push_at(&mut self, sample: f32, timestamp: Instant) {
        if self.finished {
            return;
        }
        if self.scratch.is_empty() {
            self.block_timestamp = Some(timestamp);
        }
        self.scratch.push(sample);
        if self.scratch.len() == NDI_TAP_BLOCK_FRAMES * self.channels {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.scratch.is_empty() {
            return;
        }
        self.ring.try_push(
            &self.scratch,
            self.sample_rate,
            self.channels,
            self.block_timestamp.unwrap_or_else(Instant::now),
        );
        self.scratch.clear();
        self.block_timestamp = None;
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.flush();
        self.finished = true;
    }

    fn reset(&mut self, _position: Duration) {
        self.ring.reset();
        self.scratch.clear();
        self.block_timestamp = None;
        self.finished = false;
    }
}

/// 1 kHz stereo beep during the first part of every second of the shared
/// test pattern clock, matching the audio/video sync pattern's white flash.
struct SyncBeepSource {
    start_seconds: f64,
    frame: u64,
    channel: u16,
}

impl SyncBeepSource {
    const SAMPLE_RATE: u32 = 48_000;

    fn new(start_seconds: f64) -> Self {
        Self {
            start_seconds,
            frame: 0,
            channel: 0,
        }
    }
}

impl Iterator for SyncBeepSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let seconds = self.start_seconds + self.frame as f64 / f64::from(Self::SAMPLE_RATE);
        let sample = if seconds.fract() < crate::test_pattern::SYNC_FLASH_SECONDS {
            0.5 * (std::f64::consts::TAU * 1_000.0 * seconds).sin() as f32
        } else {
            0.0
        };
        self.channel += 1;
        if self.channel == 2 {
            self.channel = 0;
            self.frame += 1;
        }
        Some(sample)
    }
}

impl Source for SyncBeepSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).expect("stereo")
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        rodio::SampleRate::new(Self::SAMPLE_RATE).expect("non-zero sample rate")
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

struct BoundedLoopSource<I> {
    input: I,
    duration: f64,
    limit: f64,
    consumed: u64,
    max_samples: u64,
}

impl<I: Source> BoundedLoopSource<I> {
    fn new(input: I, duration: f64, limit: f64) -> Result<Self, String> {
        if !duration.is_finite() || duration <= 0.0 || !limit.is_finite() || limit <= 0.0 {
            return Err("Audio loop duration is invalid.".into());
        }
        let samples = (limit * f64::from(input.sample_rate().get())).round()
            * f64::from(input.channels().get());
        if samples > u64::MAX as f64 {
            return Err("Audio loop duration is too large.".into());
        }
        Ok(Self {
            input,
            duration,
            limit,
            consumed: 0,
            max_samples: samples as u64,
        })
    }
}

impl<I: Source> Iterator for BoundedLoopSource<I> {
    type Item = I::Item;
    fn next(&mut self) -> Option<Self::Item> {
        if self.consumed >= self.max_samples {
            return None;
        }
        let sample = match self.input.next() {
            Some(sample) => sample,
            None => {
                self.input.try_seek(Duration::ZERO).ok()?;
                self.input.next()?
            }
        };
        self.consumed += 1;
        Some(sample)
    }
}

impl<I: Source> Source for BoundedLoopSource<I> {
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }
    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.input.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(self.limit))
    }
    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        let seconds = position.as_secs_f64().min(self.limit);
        self.input
            .try_seek(Duration::from_secs_f64(seconds % self.duration))?;
        self.consumed = (seconds * f64::from(self.input.sample_rate().get())).round() as u64
            * u64::from(self.input.channels().get());
        Ok(())
    }
}

struct TappedSource<I>
where
    I: Source,
{
    input: I,
    tap: PcmTapWriter,
}

impl<I> TappedSource<I>
where
    I: Source,
{
    fn new(input: I, tap: PcmTapWriter) -> Self {
        Self { input, tap }
    }
}

impl<I> Iterator for TappedSource<I>
where
    I: Source,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        match self.input.next() {
            Some(sample) => {
                self.tap.push(sample);
                Some(sample)
            }
            None => {
                self.tap.finish();
                None
            }
        }
    }
}

impl<I> Source for TappedSource<I>
where
    I: Source,
{
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.input.try_seek(position)?;
        self.tap.reset(position);
        Ok(())
    }
}

/// Applies a per-asset route to decoded NDI blocks that have not been routed yet.
fn apply_asset_routing(blocks: &mut VecDeque<NativePcmBlock>, routing: &AudioRouting, gain: f32) {
    for block in blocks.iter_mut().filter(|block| !block.asset_routed) {
        block.samples = block
            .samples
            .chunks_exact(block.channels)
            .flat_map(|frame| {
                routing
                    .render_frame(frame, block.sample_rate as f32, 0)
                    .into_iter()
                    .map(|sample| sample * gain)
            })
            .collect();
        block.channels = routing.output_channels as usize;
        block.asset_routed = true;
    }
}

/// A consumed position as (position within the current pass, completed passes).
fn loop_pass(position: f64, looping: bool, duration: Option<f64>) -> (f64, u64) {
    match duration.filter(|duration| looping && *duration > 0.0) {
        Some(duration) => (position % duration, (position / duration).floor() as u64),
        None => (position, 0),
    }
}

fn loop_terminal(position: f64, duration: Option<f64>) -> (f64, u64) {
    let (local, iteration) = loop_pass(position, true, duration);
    match duration {
        Some(duration) if local.abs() < 1e-9 && iteration > 0 => (duration, iteration - 1),
        _ => (local, iteration),
    }
}

fn music_loops(content: &Value) -> bool {
    content.get("musicLoop").and_then(Value::as_bool) == Some(true)
}

fn music_loop_limit(content: &Value, duration: Option<f64>) -> Result<Option<f64>, String> {
    if let Some(count) = content
        .get("musicLoopCount")
        .filter(|value| !value.is_null())
    {
        let count = count
            .as_u64()
            .filter(|count| *count > 0 && *count <= u32::MAX as u64)
            .ok_or("Audio loop count is invalid.")?;
        let duration = duration
            .filter(|duration| duration.is_finite() && *duration > 0.0)
            .ok_or("Audio loop count requires a known duration.")?;
        return Ok(Some(duration * count as f64));
    }
    if let Some(time) = content
        .get("musicLoopTime")
        .filter(|value| !value.is_null())
    {
        return time
            .as_f64()
            .filter(|time| time.is_finite() && *time > 0.0)
            .map(Some)
            .ok_or_else(|| "Audio loop time is invalid.".into());
    }
    Ok(None)
}

fn audio_marker_selection(selection: &Value) -> Value {
    json!(format!("audio:{selection}"))
}

fn pending_audio_seconds(pending: &VecDeque<NativePcmBlock>) -> f64 {
    pending
        .iter()
        .map(|block| {
            let remaining = block.samples.len() / block.channels - block.offset_frames;
            remaining as f64 / f64::from(block.sample_rate)
        })
        .sum()
}

fn fill_ndi_pending(track: &mut NativeAudioTrack) {
    let generation = track.ndi_tap.generation();
    if generation != track.ndi_generation {
        track.ndi_pending.clear();
        track.ndi_generation = generation;
    }
    while track.ndi_pending.len() < NDI_MAX_PENDING_BLOCKS {
        let Some(block) = track.ndi_tap.try_pop() else {
            break;
        };
        track.ndi_pending.push_back(block);
    }
}

fn fill_ndi_decoder_pending(track: &mut NdiDecoderTrack, now: Instant) {
    if !track.ready || track.status != PlaybackStatus::Playing {
        return;
    }
    let mut next_block_at = track.next_block_at.unwrap_or(now);
    if next_block_at > now {
        return;
    }
    if now.saturating_duration_since(next_block_at) > NDI_DECODER_MAX_LAG {
        // A delayed ticker must not catch up by sending several blocks in one
        // call. Drop the stale wall-clock schedule and restart at the current
        // deadline; the Rust playback sync will correct the source position
        // on its next transport snapshot.
        track.pending.clear();
        next_block_at = now;
    }
    if track.pending.len() >= NDI_MAX_PENDING_BLOCKS {
        track.next_block_at = Some(next_block_at);
        return;
    }
    let block_duration =
        Duration::from_secs_f64(NDI_TAP_BLOCK_FRAMES as f64 / NDI_MIX_SAMPLE_RATE as f64);
    let mut decoded = 0;
    while next_block_at <= now
        && decoded < NDI_MAX_DECODER_CATCH_UP_BLOCKS
        && track.pending.len() < NDI_MAX_PENDING_BLOCKS
    {
        let timestamp = next_block_at;
        match track.decode_block(timestamp) {
            Ok(Some(block)) => {
                next_block_at = timestamp.checked_add(block_duration).unwrap_or(now);
                track.pending.push_back(block);
                decoded += 1;
            }
            Ok(None) => {
                next_block_at = now.checked_add(block_duration).unwrap_or(now);
                break;
            }
            Err(error) => {
                track.error = Some(error);
                track.ended = true;
                track.next_block_at = None;
                return;
            }
        }
    }
    track.next_block_at = Some(next_block_at);
}

/// The duration of the media and music layers of a source with no output open, so the playback clock still
/// knows its length (transport progress, seek) while nothing is decoded for audio. Probed once per file.
pub fn idle_layer_durations(content: &Value) -> (Option<f64>, Option<f64>) {
    let probe = |path: Option<PathBuf>| {
        let path = path?;
        crate::media_metadata::probe(&json!(path.to_string_lossy()))
            .ok()?
            .get("durationSeconds")?
            .as_f64()
    };
    (
        probe(local_media_path(content)),
        probe(local_music_path(content)),
    )
}

fn native_source_duration(
    layer: PlaybackLayer,
    path: &Path,
    audio_duration: Option<f64>,
) -> Result<Option<f64>, String> {
    if layer == PlaybackLayer::Music {
        return Ok(audio_duration);
    }
    // An audio decoder's MP4 duration can use the audio time base with the
    // video track's sample count. Video transport must use the video timeline.
    let metadata = crate::media_metadata::probe(&json!(path.to_string_lossy()))?;
    metadata["durationSeconds"]
        .as_f64()
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map(Some)
        .ok_or_else(|| "Native video source duration is unavailable.".to_string())
}

fn open_ndi_decoder(
    path: &Path,
) -> Result<(Decoder<BufReader<File>>, Option<f64>, usize, u32), String> {
    let file = File::open(path)
        .map_err(|error| format!("Cannot open native NDI audio '{}': {error}", path.display()))?;
    let decoder = Decoder::try_from(file).map_err(|error| {
        format!(
            "Cannot decode native NDI audio '{}': {error}",
            path.display()
        )
    })?;
    let duration = decoder.total_duration().map(|value| value.as_secs_f64());
    let channels = decoder.channels().get() as usize;
    let sample_rate = decoder.sample_rate().get();
    if channels == 0 || sample_rate == 0 {
        return Err(format!(
            "Native NDI audio '{}' has an invalid channel layout.",
            path.display()
        ));
    }
    Ok((decoder, duration, channels, sample_rate))
}

fn ndi_content_decoder_identity(
    path: &Path,
    looping: bool,
    layer: PlaybackLayer,
    content: &Value,
) -> String {
    let selection = match layer {
        PlaybackLayer::Media => "backgroundSelectionId",
        PlaybackLayer::Music => "audioSelectionId",
    };
    format!(
        "{}|{}|{}|{}",
        ndi_decoder_identity(path, looping),
        content.get(selection).unwrap_or(&Value::Null),
        content.get("musicLoopCount").unwrap_or(&Value::Null),
        content.get("musicLoopTime").unwrap_or(&Value::Null)
    )
}

fn ndi_decoder_identity(path: &Path, looping: bool) -> String {
    format!("{}|{looping}", path.to_string_lossy())
}

fn position_drift(current: f64, target: f64, duration: Option<f64>, looping: bool) -> f64 {
    let direct = (current - target).abs();
    if !looping {
        return direct;
    }
    let Some(duration) = duration.filter(|value| value.is_finite() && *value > 0.0) else {
        return direct;
    };
    direct.min((duration - direct.rem_euclid(duration)).abs())
}

fn ndi_audio_enabled(settings: &Value) -> bool {
    settings
        .get("sdiNdi")
        .and_then(Value::as_object)
        .and_then(|value| value.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn mix_ndi_pending(
    media: &mut VecDeque<NativePcmBlock>,
    music: &mut VecDeque<NativePcmBlock>,
    max_blocks: usize,
) -> Vec<NativePcmBlock> {
    let mut mixed = Vec::with_capacity(max_blocks);
    for _ in 0..max_blocks {
        let Some(block) = mix_ndi_window(media, music) else {
            break;
        };
        mixed.push(block);
    }
    mixed
}

fn mix_ndi_window(
    media: &mut VecDeque<NativePcmBlock>,
    music: &mut VecDeque<NativePcmBlock>,
) -> Option<NativePcmBlock> {
    mix_ndi_window_layers(&mut [media, music])
}

fn mix_ndi_window_layers(layers: &mut [&mut VecDeque<NativePcmBlock>]) -> Option<NativePcmBlock> {
    let start = layers
        .iter()
        .filter_map(|layer| layer.front())
        .map(NativePcmBlock::start_time)
        .min()?;
    let end = start.checked_add(Duration::from_secs_f64(
        NDI_TAP_BLOCK_FRAMES as f64 / NDI_MIX_SAMPLE_RATE as f64,
    ))?;
    let mix_channels = layers
        .iter()
        .flat_map(|layer| layer.iter())
        .filter(|block| block.start_time() < end && block.end_time() > start)
        .map(|block| block.channels)
        .max()
        .unwrap_or(NDI_MIX_CHANNELS)
        .clamp(NDI_MIX_CHANNELS, LOGICAL_CHANNELS as usize);
    let mut samples = vec![0.0; NDI_TAP_BLOCK_FRAMES * mix_channels];
    for frame in 0..NDI_TAP_BLOCK_FRAMES {
        let timestamp = start.checked_add(Duration::from_secs_f64(
            frame as f64 / NDI_MIX_SAMPLE_RATE as f64,
        ))?;
        for layer in layers.iter() {
            add_ndi_layer_frame(layer, timestamp, &mut samples, frame, mix_channels);
        }
    }
    for layer in layers.iter_mut() {
        advance_ndi_queue(layer, end);
    }
    Some(NativePcmBlock {
        asset_routed: true,
        preserve_channel_count: true,
        samples,
        sample_rate: NDI_MIX_SAMPLE_RATE,
        channels: mix_channels,
        timestamp: start,
        offset_frames: 0,
    })
}

fn add_ndi_layer_frame(
    queue: &VecDeque<NativePcmBlock>,
    timestamp: Instant,
    output: &mut [f32],
    frame: usize,
    output_channels: usize,
) {
    let Some(block) = queue
        .iter()
        .find(|block| block.start_time() <= timestamp && timestamp < block.end_time())
    else {
        return;
    };
    let start = block.start_time();
    if timestamp < start || timestamp >= block.end_time() {
        return;
    }
    let position =
        timestamp.saturating_duration_since(start).as_secs_f64() * block.sample_rate.max(1) as f64;
    let index = position.floor() as usize;
    let fraction = (position - index as f64) as f32;
    let first = block.offset_frames.saturating_add(index);
    let second = first.saturating_add(1);
    for channel in 0..output_channels {
        let source_channel = if channel < block.channels {
            channel
        } else if block.channels == 1 && !block.preserve_channel_count && channel == 1 {
            0
        } else {
            continue;
        };
        let first_index = first
            .saturating_mul(block.channels)
            .saturating_add(source_channel);
        if first_index >= block.samples.len() {
            continue;
        }
        let second_index = second
            .saturating_mul(block.channels)
            .saturating_add(source_channel);
        let first_sample = block.samples[first_index];
        let second_sample = block
            .samples
            .get(second_index)
            .copied()
            .unwrap_or(first_sample);
        output[frame * output_channels + channel] +=
            first_sample + (second_sample - first_sample) * fraction;
    }
}

fn advance_ndi_queue(queue: &mut VecDeque<NativePcmBlock>, end: Instant) {
    loop {
        let Some(block) = queue.front_mut() else {
            return;
        };
        if block.end_time() <= end {
            queue.pop_front();
            continue;
        }
        let start = block.start_time();
        if start >= end {
            return;
        }
        let frames = end
            .saturating_duration_since(start)
            .as_secs_f64()
            .mul_add(block.sample_rate.max(1) as f64, 0.0)
            .ceil() as usize;
        if frames == 0 {
            return;
        }
        if frames >= block.remaining_frames() {
            queue.pop_front();
        } else {
            block.offset_frames = block.offset_frames.saturating_add(frames);
        }
    }
}

/// Converts a decoded interleaved source into the exact channel layout used by
/// the browser audio graph: every configured Pro channel can feed any device
/// channel, with mute/solo/tone applied before the final device mixer.
struct RoutedSource<I>
where
    I: Source,
{
    input: I,
    routing: AudioRouting,
    input_channels: usize,
    frame: Vec<f32>,
    next_output_channel: usize,
    frame_position: u64,
    tap: Option<program_levels::Tap>,
}

impl<I> RoutedSource<I>
where
    I: Source,
{
    fn new(input: I, routing: AudioRouting) -> Self {
        let input_channels = input.channels().get() as usize;
        Self {
            input,
            routing,
            input_channels,
            frame: Vec::new(),
            next_output_channel: 0,
            frame_position: 0,
            tap: None,
        }
    }

    /// Publishes the routed output levels for the program meters.
    fn metered(mut self) -> Self {
        self.tap = Some(program_levels::Tap::new(
            self.routing.output_channels as usize,
            self.input.sample_rate().get(),
        ));
        self
    }

    fn fill_frame(&mut self) -> bool {
        if self.input_channels == 0 {
            return false;
        }
        let mut input_frame = Vec::with_capacity(self.input_channels);
        for _ in 0..self.input_channels {
            match self.input.next() {
                Some(sample) => input_frame.push(sample),
                None if input_frame.is_empty() => return false,
                None => input_frame.push(0.0),
            }
        }

        let frame_position = self.frame_position;
        self.frame_position = self.frame_position.saturating_add(1);
        self.frame = self.routing.render_frame(
            &input_frame,
            self.input.sample_rate().get() as f32,
            frame_position,
        );
        if let Some(tap) = &mut self.tap {
            tap.push(&self.frame);
        }
        self.next_output_channel = 0;
        true
    }
}

impl<I> Iterator for RoutedSource<I>
where
    I: Source,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_output_channel >= self.frame.len() && !self.fill_frame() {
            return None;
        }
        let sample = self.frame[self.next_output_channel];
        self.next_output_channel += 1;
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

impl<I> Source for RoutedSource<I>
where
    I: Source,
{
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.routing.output_channels)
            .expect("audio routing always clamps output channels to one or more")
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.input.try_seek(position)?;
        if let Some(tap) = &mut self.tap {
            tap.reset();
        }
        self.frame.clear();
        self.next_output_channel = 0;
        let frame_position = position.as_secs_f64() * self.sample_rate().get() as f64;
        self.frame_position = if frame_position.is_finite() && frame_position >= 0.0 {
            frame_position.round().min(u64::MAX as f64) as u64
        } else {
            u64::MAX
        };
        Ok(())
    }
}

fn local_media_path(content: &Value) -> Option<PathBuf> {
    let media_type = string_value(content.get("backgroundMediaType"))?;
    if !media_type.eq_ignore_ascii_case("video") {
        return None;
    }
    media_source_path(
        content.get("backgroundMediaPath"),
        content.get("backgroundMediaUrl"),
    )
}

fn local_music_path(content: &Value) -> Option<PathBuf> {
    media_source_path(content.get("audioPath"), content.get("audioUrl"))
}

fn media_source_path(path: Option<&Value>, url: Option<&Value>) -> Option<PathBuf> {
    if let Some(path) = path.and_then(resolve_media_path) {
        return Some(path);
    }
    let url = url.and_then(Value::as_str)?.trim();
    if url.is_empty() {
        return None;
    }
    if let Some(path) = resolve_media_path(&Value::String(url.to_string())) {
        return Some(path);
    }
    prefetch_media_url(url);
    None
}

fn media_url_present(content: &Value, layer: PlaybackLayer) -> bool {
    let key = match layer {
        PlaybackLayer::Media => "backgroundMediaUrl",
        PlaybackLayer::Music => "audioUrl",
    };
    content
        .get(key)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn background_loops(content: &Value) -> bool {
    string_value(content.get("backgroundPlayback"))
        .is_some_and(|value| value.eq_ignore_ascii_case("loop"))
}

fn gain_from_settings(main: &Value) -> f32 {
    let decibels = number(main.get("level"))
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        .clamp(-60.0, 6.0);
    10.0_f64.powf(decibels / 20.0).clamp(0.0, 2.0) as f32
}

fn delay_from_settings(main: &Value) -> Duration {
    let milliseconds = number(main.get("delay"))
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
        .max(0.0);
    Duration::from_secs_f64(milliseconds / 1_000.0)
}

fn canonical_device_key(value: &str) -> String {
    let value = value.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case(DEFAULT_DEVICE)
        || value.eq_ignore_ascii_case("default")
    {
        DEFAULT_DEVICE.to_string()
    } else {
        value.to_ascii_lowercase()
    }
}

fn track_key(source_id: &str, layer: PlaybackLayer) -> String {
    format!("{source_id}:{}", layer.as_str())
}

fn string_value(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(value)) => value.as_f64(),
        Some(Value::String(value)) => value.trim().parse().ok(),
        _ => None,
    }
}
