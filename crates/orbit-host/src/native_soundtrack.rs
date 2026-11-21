//! Native timeline soundtrack: one audio file that follows the authored
//! timeline transport on the main audio output.
//!
//! The transport (generation, running anchor, position) is the only clock. A
//! new generation (start, seek, stop-and-play) repositions the audio, a
//! running transport is checked for drift on every tick and corrected by
//! seeking when the audio is more than `DRIFT_THRESHOLD` off, and a loop wrap
//! or a retired generation is handled the same way: the audio follows or stops.

use rodio::{MixerDeviceSink, Player};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Largest tolerated difference between audio and transport position.
pub(crate) const DRIFT_THRESHOLD: f64 = 0.1;
/// After a correction the next one waits, so device latency cannot cause seek storms.
const SEEK_COOLDOWN: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SoundtrackPlan {
    pub path: PathBuf,
    /// Normalized main audio settings that route, level and delay the output.
    pub main: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SoundtrackTarget {
    pub plan: SoundtrackPlan,
    pub generation: String,
    pub playing: bool,
    pub position: f64,
    pub playback_rate: f64,
}

/// Resolves the live presentation's soundtrack for a snapshot whose transport
/// is playing or recording.
pub(crate) fn plan_from_snapshot(snapshot: &Value) -> Result<Option<SoundtrackPlan>, String> {
    let timeline = &snapshot["liveItem"]["timeline"];
    let Some(id) = timeline.get("soundtrackMediaId").and_then(Value::as_str) else {
        return Ok(None);
    };
    let media = snapshot
        .get("media")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    crate::native_timeline::validate_soundtrack(timeline, media)?;
    let path = media
        .iter()
        .find(|asset| asset["id"].as_str() == Some(id))
        .and_then(|asset| asset["sourcePath"].as_str())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or("Timeline soundtrack has no local file.")?;
    Ok(Some(SoundtrackPlan {
        path,
        main: snapshot["audioSettings"]["main"].clone(),
    }))
}

/// Peak overview of a workspace audio asset.
pub(crate) fn waveform_for_media(
    workspace_path: &std::path::Path,
    media_id: &str,
    buckets: usize,
) -> Result<Value, String> {
    let workspace = crate::workspace::read_workspace(workspace_path)?;
    let asset = workspace["media"]
        .as_array()
        .and_then(|media| {
            media
                .iter()
                .find(|asset| asset["id"].as_str() == Some(media_id))
        })
        .ok_or("Media asset was not found.")?;
    if asset["type"].as_str() != Some("audio") {
        return Err("Waveforms require an audio asset.".into());
    }
    crate::native_waveform::waveform(
        &crate::media_thumbnail::asset_path(workspace_path, asset)?,
        buckets,
    )
}

pub(crate) trait SoundtrackOutput {
    fn load(&mut self, plan: &SoundtrackPlan, reverse: bool) -> Result<Option<f64>, String>;
    fn unload(&mut self);
    fn seek(&mut self, seconds: f64) -> Result<(), String>;
    fn set_speed(&mut self, speed: f64) -> Result<(), String>;
    fn play(&mut self);
    fn pause(&mut self);
    fn position(&self) -> f64;
    fn error(&self) -> Option<String>;
}

struct Loaded {
    identity: String,
    duration: Option<f64>,
    reverse: bool,
}

pub(crate) struct SoundtrackSync<O> {
    output: O,
    loaded: Option<Loaded>,
    failed: Option<(String, bool)>,
    error: Option<String>,
    generation: Option<String>,
    playing: bool,
    playback_rate: f64,
    ended: bool,
    last_seek: Option<Instant>,
}

impl<O: SoundtrackOutput> SoundtrackSync<O> {
    pub(crate) fn new(output: O) -> Self {
        Self {
            output,
            loaded: None,
            failed: None,
            error: None,
            generation: None,
            playing: false,
            playback_rate: 1.0,
            ended: false,
            last_seek: None,
        }
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn unload(&mut self) {
        if self.loaded.take().is_some() {
            self.output.unload();
        }
        self.generation = None;
        self.playing = false;
        self.playback_rate = 1.0;
        self.ended = false;
        self.last_seek = None;
    }

    fn reset(&mut self) {
        self.unload();
        self.failed = None;
        self.error = None;
    }

    fn seek(&mut self, seconds: f64, now: Instant) {
        match self.output.seek(seconds) {
            Ok(()) => self.error = None,
            Err(error) => self.error = Some(error),
        }
        self.last_seek = Some(now);
    }

    pub(crate) fn sync(&mut self, target: Option<&SoundtrackTarget>, now: Instant) {
        let Some(target) = target else {
            self.reset();
            return;
        };
        if let Some(error) = self.output.error() {
            self.error = Some(error);
        }
        let identity = format!("{}|{}", target.plan.path.display(), target.plan.main);
        let reverse = target.playback_rate < 0.0;
        if self
            .loaded
            .as_ref()
            .is_none_or(|loaded| loaded.identity != identity || loaded.reverse != reverse)
        {
            self.unload();
            if self
                .failed
                .as_ref()
                .is_some_and(|(failed, failed_reverse)| {
                    failed == &identity && *failed_reverse == reverse
                })
            {
                return;
            }
            self.error = None;
            match self.output.load(&target.plan, reverse) {
                Ok(duration) => {
                    self.failed = None;
                    self.loaded = Some(Loaded {
                        identity: identity.clone(),
                        duration,
                        reverse,
                    });
                }
                Err(error) => {
                    self.failed = Some((identity.clone(), reverse));
                    self.error = Some(error);
                    return;
                }
            }
        }
        let duration = self.loaded.as_ref().and_then(|loaded| loaded.duration);
        let beyond = if reverse {
            target.position <= 0.0
        } else {
            duration.is_some_and(|duration| target.position >= duration)
        };
        if self.ended && !beyond {
            self.unload();
            self.error = None;
            match self.output.load(&target.plan, reverse) {
                Ok(duration) => {
                    self.failed = None;
                    self.loaded = Some(Loaded {
                        identity: identity.clone(),
                        duration,
                        reverse,
                    });
                }
                Err(error) => {
                    self.failed = Some((identity, reverse));
                    self.error = Some(error);
                    return;
                }
            }
        }
        let speed = target.playback_rate.abs();
        let speed_changed = (speed - self.playback_rate.abs()).abs() > f64::EPSILON;
        if speed_changed {
            match self.output.set_speed(speed) {
                Ok(()) => self.error = None,
                Err(error) => self.error = Some(error),
            }
            self.playback_rate = target.playback_rate;
        }
        if self.generation.as_ref() != Some(&target.generation) || (self.ended && !beyond) {
            self.generation = Some(target.generation.clone());
            self.ended = beyond;
            self.playing = target.playing && !beyond;
            if beyond {
                self.output.pause();
                return;
            }
            self.seek(target.position, now);
            if target.playing {
                self.output.play();
            } else {
                self.output.pause();
            }
            return;
        }
        if beyond {
            if !self.ended {
                self.ended = true;
                self.playing = false;
                self.output.pause();
            }
            return;
        }
        if speed_changed {
            self.seek(target.position, now);
        }
        let drifting = (self.output.position() - target.position).abs() > DRIFT_THRESHOLD;
        let cooled = self
            .last_seek
            .is_none_or(|seek| now.saturating_duration_since(seek) >= SEEK_COOLDOWN);
        if target.playing != self.playing {
            self.playing = target.playing;
            if target.playing {
                if drifting && !speed_changed {
                    self.seek(target.position, now);
                }
                self.output.play();
            } else {
                self.output.pause();
            }
        } else if self.playing && drifting && cooled && !speed_changed {
            self.seek(target.position, now);
        }
    }
}

#[derive(Default)]
pub(crate) struct SoundtrackDevice {
    sink: Option<(String, MixerDeviceSink)>,
    player: Option<Player>,
    duration: Option<f64>,
    reverse: bool,
    speed: f64,
    reverse_error: Option<Arc<Mutex<Option<String>>>>,
}

impl SoundtrackOutput for SoundtrackDevice {
    fn load(&mut self, plan: &SoundtrackPlan, reverse: bool) -> Result<Option<f64>, String> {
        let (device, channels) = crate::native_audio::main_audio_target(&plan.main)
            .ok_or("Main audio output is disabled, so the timeline soundtrack is silent.")?;
        let key = crate::native_audio::main_audio_sink_key(&device, channels);
        if self.sink.as_ref().is_none_or(|(open, _)| *open != key) {
            self.player = None;
            self.sink = Some((
                key,
                crate::native_audio::open_main_audio_sink(&device, channels)?,
            ));
        }
        let sink = &self.sink.as_ref().expect("sink was opened").1;
        let (player, duration, reverse_error) =
            crate::native_audio::connect_soundtrack_player(sink, &plan.path, &plan.main, reverse)?;
        self.player = Some(player);
        self.duration = duration;
        self.reverse = reverse;
        self.speed = 1.0;
        self.reverse_error = reverse_error;
        Ok(duration)
    }

    fn unload(&mut self) {
        self.player = None;
        self.duration = None;
        self.reverse = false;
        self.speed = 1.0;
        self.reverse_error = None;
    }

    fn seek(&mut self, seconds: f64) -> Result<(), String> {
        let player = self
            .player
            .as_ref()
            .ok_or("The soundtrack is not loaded.")?;
        let position = if self.reverse {
            self.duration
                .ok_or("Reverse soundtrack duration is unavailable.")?
                - seconds
        } else {
            seconds
        };
        player
            .try_seek(Duration::from_secs_f64((position / self.speed).max(0.0)))
            .map_err(|error| format!("Cannot seek the soundtrack to {seconds:.3}s: {error}"))
    }

    fn set_speed(&mut self, speed: f64) -> Result<(), String> {
        let player = self
            .player
            .as_ref()
            .ok_or("The soundtrack is not loaded.")?;
        player.set_speed(speed as f32);
        self.speed = speed;
        Ok(())
    }

    fn play(&mut self) {
        if let Some(player) = &self.player {
            player.play();
        }
    }

    fn pause(&mut self) {
        if let Some(player) = &self.player {
            player.pause();
        }
    }

    fn position(&self) -> f64 {
        let position = self
            .player
            .as_ref()
            .map_or(0.0, |player| player.get_pos().as_secs_f64() * self.speed);
        if self.reverse {
            self.duration.map_or(0.0, |duration| duration - position)
        } else {
            position
        }
    }

    fn error(&self) -> Option<String> {
        self.reverse_error.as_ref().and_then(|error| {
            error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        })
    }
}

pub(crate) type SoundtrackPlayer = SoundtrackSync<SoundtrackDevice>;

impl Default for SoundtrackPlayer {
    fn default() -> Self {
        Self::new(SoundtrackDevice::default())
    }
}
