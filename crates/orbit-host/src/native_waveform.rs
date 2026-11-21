//! Bounded, cached peak overview of an audio asset for the timeline soundtrack lane.

use rodio::{Decoder, Source};
use serde_json::{json, Value};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};

/// Peak resolution kept per cached file: one value per 20 ms.
const BINS_PER_SECOND: u64 = 50;
/// Longest audio the overview decodes; a timeline is limited to 24 hours but
/// a soundtrack of more than four hours is rejected rather than truncated.
const MAX_SECONDS: u64 = 4 * 3600;
const MAX_BUCKETS: usize = 4096;
const CACHE_ENTRIES: usize = 8;

#[derive(Clone, PartialEq)]
struct CacheKey {
    path: PathBuf,
    modified: Option<SystemTime>,
    length: u64,
}

struct Overview {
    duration: f64,
    peaks: Vec<f32>,
}

static CACHE: OnceLock<Mutex<Vec<(CacheKey, Arc<Overview>)>>> = OnceLock::new();

fn decode(path: &Path) -> Result<Overview, String> {
    let file = File::open(path)
        .map_err(|error| format!("Cannot open soundtrack '{}': {error}", path.display()))?;
    let decoder = Decoder::try_from(file)
        .map_err(|error| format!("Cannot decode soundtrack '{}': {error}", path.display()))?;
    let channels = usize::from(decoder.channels().get());
    let rate = u64::from(decoder.sample_rate().get());
    let mut peaks: Vec<f32> = Vec::new();
    let mut frames = 0_u64;
    let mut frame_peak = 0.0_f32;
    for (index, sample) in decoder.enumerate() {
        frame_peak = frame_peak.max(sample.abs());
        if (index + 1) % channels != 0 {
            continue;
        }
        let bin = (frames * BINS_PER_SECOND / rate) as usize;
        if bin as u64 >= MAX_SECONDS * BINS_PER_SECOND {
            return Err("The soundtrack is longer than four hours.".into());
        }
        if peaks.len() <= bin {
            peaks.resize(bin + 1, 0.0);
        }
        peaks[bin] = peaks[bin].max(frame_peak.min(1.0));
        frame_peak = 0.0;
        frames += 1;
    }
    if frames == 0 {
        return Err("The soundtrack contains no audio.".into());
    }
    Ok(Overview {
        duration: frames as f64 / rate as f64,
        peaks,
    })
}

fn overview(path: &Path) -> Result<Arc<Overview>, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("Soundtrack '{}' is unavailable: {error}", path.display()))?;
    let key = CacheKey {
        path: path.to_path_buf(),
        modified: metadata.modified().ok(),
        length: metadata.len(),
    };
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Some((_, found)) = cache
        .lock()
        .map_err(|_| "The waveform cache is unavailable.")?
        .iter()
        .find(|(cached, _)| *cached == key)
    {
        return Ok(found.clone());
    }
    let computed = Arc::new(decode(path)?);
    let mut cache = cache
        .lock()
        .map_err(|_| "The waveform cache is unavailable.")?;
    cache.retain(|(cached, _)| *cached != key);
    cache.push((key, computed.clone()));
    if cache.len() > CACHE_ENTRIES {
        cache.remove(0);
    }
    Ok(computed)
}

/// Peaks in 0..1 reduced to at most `buckets` values, plus the audio duration.
pub fn waveform(path: &Path, buckets: usize) -> Result<Value, String> {
    if buckets == 0 || buckets > MAX_BUCKETS {
        return Err(format!(
            "Waveform buckets must be between 1 and {MAX_BUCKETS}."
        ));
    }
    let overview = overview(path)?;
    let peaks: Vec<f32> = if overview.peaks.len() <= buckets {
        overview.peaks.clone()
    } else {
        (0..buckets)
            .map(|bucket| {
                let start = bucket * overview.peaks.len() / buckets;
                let end = ((bucket + 1) * overview.peaks.len() / buckets).max(start + 1);
                overview.peaks[start..end]
                    .iter()
                    .fold(0.0_f32, |peak, value| peak.max(*value))
            })
            .collect()
    };
    Ok(json!({ "durationSeconds": overview.duration, "peaks": peaks }))
}
