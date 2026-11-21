//! Per-channel RMS of the routed main-output audio, for the operator's program meters. Sources opt in with
//! `RoutedSource::metered`; the audio thread only does atomic stores.
use super::*;
use std::sync::atomic::AtomicU32;
use std::sync::{LazyLock, Weak};

/// A paused or stopped source stops pulling samples, so its last reading is older than this when it is silent.
const STALE: Duration = Duration::from_millis(250);
static CLOCK: LazyLock<Instant> = LazyLock::new(Instant::now);
static TRACKS: Mutex<Vec<Weak<TrackLevels>>> = Mutex::new(Vec::new());

struct TrackLevels {
    channels: usize,
    levels: [AtomicU32; MAX_OUTPUT_CHANNELS as usize],
    updated_ms: AtomicU64,
}

pub(super) struct Tap {
    shared: Arc<TrackLevels>,
    sums: Vec<f64>,
    frames: usize,
    window: usize,
}

/// The web-audio analyser scale of `getAnalyserLevel`: -60 dB is 0 and 0 dB is 100.
pub fn rms_percent(rms: f64) -> f32 {
    (((20. * rms.max(0.001).log10() + 60.) / 60.) * 100.)
        .round()
        .clamp(0., 100.) as f32
}

impl Tap {
    pub(super) fn new(channels: usize, sample_rate: u32) -> Self {
        let shared = Arc::new(TrackLevels {
            channels: channels.min(MAX_OUTPUT_CHANNELS as usize),
            levels: Default::default(),
            updated_ms: AtomicU64::new(0),
        });
        if let Ok(mut tracks) = TRACKS.lock() {
            tracks.retain(|track| track.strong_count() > 0);
            tracks.push(Arc::downgrade(&shared));
        }
        Self {
            sums: vec![0.; shared.channels],
            shared,
            frames: 0,
            window: (sample_rate as usize / 50).max(1),
        }
    }

    pub(super) fn push(&mut self, frame: &[f32]) {
        for (sum, sample) in self.sums.iter_mut().zip(frame) {
            *sum += f64::from(*sample) * f64::from(*sample);
        }
        self.frames += 1;
        if self.frames < self.window {
            return;
        }
        for (sum, level) in self.sums.iter_mut().zip(&self.shared.levels) {
            level.store(
                rms_percent((*sum / self.frames as f64).sqrt()).to_bits(),
                Ordering::Relaxed,
            );
            *sum = 0.;
        }
        self.frames = 0;
        self.shared
            .updated_ms
            .store(CLOCK.elapsed().as_millis() as u64 + 1, Ordering::Relaxed);
    }

    pub(super) fn reset(&mut self) {
        self.sums.fill(0.);
        self.frames = 0;
        self.shared.updated_ms.store(0, Ordering::Relaxed);
    }
}

/// The loudest fresh reading per channel across the playing main-output sources; empty when none plays.
pub fn program_audio_levels() -> Vec<f32> {
    let now = CLOCK.elapsed().as_millis() as u64;
    let Ok(tracks) = TRACKS.lock() else {
        return Vec::new();
    };
    let mut levels: Vec<f32> = Vec::new();
    for track in tracks.iter().filter_map(Weak::upgrade) {
        let updated = track.updated_ms.load(Ordering::Relaxed);
        if updated == 0 || now.saturating_sub(updated - 1) > STALE.as_millis() as u64 {
            continue;
        }
        levels.resize(levels.len().max(track.channels), 0.);
        for (level, stored) in levels.iter_mut().zip(&track.levels[..track.channels]) {
            *level = level.max(f32::from_bits(stored.load(Ordering::Relaxed)));
        }
    }
    levels
}
