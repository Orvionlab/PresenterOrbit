use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

use crate::native_render::{image_file_stamp, ImageFileStamp};

const CACHE_ENTRIES: usize = 128;
struct Cached {
    path: PathBuf,
    stamp: ImageFileStamp,
    result: Result<Value, String>,
}
static CACHE: OnceLock<Mutex<VecDeque<Cached>>> = OnceLock::new();
static LOADS: AtomicU64 = AtomicU64::new(0);
static HITS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn cache_counts() -> (u64, u64) {
    (LOADS.load(Ordering::Relaxed), HITS.load(Ordering::Relaxed))
}

pub fn probe(source: &Value) -> Result<Value, String> {
    let path = crate::native_media::resolve_media_path(source)
        .filter(|path| path.is_file())
        .ok_or_else(|| "Media source is unavailable for metadata probing.".to_string())?;
    let stamp = image_file_stamp(&path)?;
    let cache = CACHE.get_or_init(Default::default);
    {
        let mut cached = cache
            .lock()
            .map_err(|_| "Media metadata cache is unavailable.")?;
        if let Some(index) = cached
            .iter()
            .position(|entry| entry.path == path && entry.stamp == stamp)
        {
            let entry = cached.remove(index).expect("metadata cache entry exists");
            let result = entry.result.clone();
            cached.push_back(entry);
            HITS.fetch_add(1, Ordering::Relaxed);
            return result;
        }
    }
    LOADS.fetch_add(1, Ordering::Relaxed);
    let result = probe_uncached(&path);
    if image_file_stamp(&path)? != stamp {
        return Err("Media changed while reading its metadata.".into());
    }
    let mut cached = cache
        .lock()
        .map_err(|_| "Media metadata cache is unavailable.")?;
    cached.retain(|entry| entry.path != path);
    if cached.len() >= CACHE_ENTRIES {
        cached.pop_front();
    }
    cached.push_back(Cached {
        path,
        stamp,
        result: result.clone(),
    });
    result
}

fn probe_uncached(path: &Path) -> Result<Value, String> {
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        let image =
            crate::native_render::NativeRenderRuntime::new_static_preview().image_frame(path)?;
        return Ok(
            json!({"width":image.image.width(),"height":image.image.height(),"durationSeconds":null,"fps":null,"hasAudio":false}),
        );
    }
    let ffmpeg = ffmpeg_sidecar::paths::ffmpeg_path();
    let sibling = ffmpeg.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    let executable = if sibling.is_file() {
        sibling
    } else {
        "ffprobe".into()
    };
    let mut command = Command::new(executable);
    command
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .output()
        .map_err(|error| format!("Media metadata probe could not start: {error}"))?;
    if !output.status.success() {
        return Err("Media metadata probe failed to read this asset.".to_string());
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Invalid media metadata probe response: {error}"))?;
    Ok(metadata(&value))
}

fn positive_number(value: &Value) -> Option<f64> {
    let number = value.as_f64().or_else(|| value.as_str()?.parse().ok())?;
    (number.is_finite() && number > 0.0).then_some(number)
}

fn metadata(value: &Value) -> Value {
    let stream = value["streams"].as_array().and_then(|streams| {
        streams.iter().find(|stream| {
            stream["codec_type"] == "video" && stream["disposition"]["attached_pic"] != 1
        })
    });
    let stream = stream
        .or_else(|| {
            value["streams"].as_array().and_then(|streams| {
                streams
                    .iter()
                    .find(|stream| stream["codec_type"] == "audio")
            })
        })
        .unwrap_or(&Value::Null);
    let fps = stream["avg_frame_rate"].as_str().and_then(|rate| {
        let (numerator, denominator) = rate.split_once('/')?;
        let denominator = denominator.parse::<f64>().ok()?;
        let fps = numerator.parse::<f64>().ok()? / denominator;
        (denominator > 0.0 && fps.is_finite() && fps > 0.0).then_some(fps)
    });
    let dynamic_range = match stream["color_transfer"].as_str() {
        Some("smpte2084") => Some("HDR (PQ)"),
        Some("arib-std-b67") => Some("HDR (HLG)"),
        Some(
            "bt709" | "gamma22" | "gamma28" | "smpte170m" | "smpte240m" | "iec61966-2-1"
            | "bt2020-10" | "bt2020-12",
        ) => Some("SDR"),
        _ => None,
    };
    let audio = value["streams"].as_array().and_then(|streams| {
        streams
            .iter()
            .find(|stream| stream["codec_type"] == "audio")
    });
    json!({
        "codec": stream["codec_name"].as_str().filter(|name| !name.trim().is_empty()),
        "width": positive_number(&stream["width"]),
        "height": positive_number(&stream["height"]),
        "durationSeconds": positive_number(&stream["duration"]).or_else(|| positive_number(&value["format"]["duration"])),
        "fps": fps,
        "dynamicRange": dynamic_range,
        "hasAudio": audio.is_some(),
        "audioChannels": audio.and_then(|stream| stream["channels"].as_u64())
    })
}
