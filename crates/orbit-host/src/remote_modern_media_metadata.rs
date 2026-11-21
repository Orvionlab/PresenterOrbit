//! Metadata for assets in the current Remote media catalogs.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

use serde_json::{json, Value};

#[derive(Clone)]
struct CachedProbe {
    modified: SystemTime,
    length: u64,
    result: Value,
}

static PROBES: OnceLock<Mutex<HashMap<PathBuf, CachedProbe>>> = OnceLock::new();

/// Reads source-backed fields missing from the Orbit media record.
pub(in super::super) fn inspect(asset: &Value) -> Result<Value, String> {
    let media_type = asset
        .get("type")
        .and_then(Value::as_str)
        .ok_or("Media item type is unavailable for metadata probing.")?;
    let duration = match media_type {
        "image" => 0.0,
        "audio" | "video" => {
            let source = asset
                .get("sourcePath")
                .filter(|source| source.as_str().is_some_and(|path| !path.trim().is_empty()))
                .ok_or("Media source path is unavailable for metadata probing.")?;
            let probed = probe(source)?;
            probed
                .get("durationSeconds")
                .and_then(Value::as_f64)
                .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
                .ok_or("Media source duration is unavailable.")?
        }
        _ => return Err("Media item type is unsupported for metadata probing.".into()),
    };
    Ok(json!({
        "artist": artist(asset)?,
        "durationSeconds": duration
    }))
}

fn probe(source: &Value) -> Result<Value, String> {
    let path = crate::native_media::resolve_media_path(source)
        .filter(|path| path.is_file())
        .ok_or_else(|| "Media source is unavailable for metadata probing.".to_string())?;
    let file = std::fs::metadata(&path)
        .map_err(|_| "Media source metadata is unavailable for probing.".to_string())?;
    let modified = file
        .modified()
        .map_err(|_| "Media source modification time is unavailable for probing.".to_string())?;
    let length = file.len();
    let cache = PROBES.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let entries = cache
            .lock()
            .map_err(|_| "Media metadata cache is unavailable.".to_string())?;
        if let Some(cached) = entries.get(&path) {
            if cached.modified == modified && cached.length == length {
                return Ok(cached.result.clone());
            }
        }
    }
    let result = crate::media_metadata::probe(source)?;
    let mut entries = cache
        .lock()
        .map_err(|_| "Media metadata cache is unavailable.".to_string())?;
    entries.insert(
        path,
        CachedProbe {
            modified,
            length,
            result: result.clone(),
        },
    );
    Ok(result)
}

pub(super) fn artist(asset: &Value) -> Result<&str, String> {
    match asset.get("artist") {
        None | Some(Value::Null) => Ok(""),
        Some(Value::String(artist)) => Ok(artist),
        Some(_) => Err("Media item artist metadata is invalid.".into()),
    }
}

pub(super) fn duration(metadata: &Value) -> Result<f64, String> {
    metadata
        .get("durationSeconds")
        .and_then(Value::as_f64)
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .ok_or_else(|| "Media item duration metadata is unavailable from its source.".into())
}

pub(super) fn api_duration(seconds: f64) -> Result<u32, String> {
    if !seconds.is_finite() || seconds < 0.0 || seconds.floor() > u32::MAX as f64 {
        return Err("Media duration cannot be represented by the Remote API.".into());
    }
    Ok(seconds.floor() as u32)
}
