//! Process-local, shared decoded media cache. Preparation never holds an output renderer lock.
use super::{decode_native_image_asset, image_file_stamp, ImageFileStamp, NativeImageAsset};
use image::DynamicImage;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock},
};

const CACHE_BYTES: usize = 256 * 1024 * 1024;
const ASSET_BYTES: usize = 128 * 1024 * 1024;
const CACHE_ENTRIES: usize = 128;
const MAX_REQUESTS: usize = 4096;

#[derive(Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    path: PathBuf,
    kind: String,
    start_seconds: f64,
}

enum Asset {
    Image(NativeImageAsset),
    Video(Arc<DynamicImage>),
}

impl Asset {
    fn bytes(&self) -> usize {
        match self {
            Self::Image(image) => image.decoded_bytes,
            Self::Video(image) => image.as_bytes().len(),
        }
    }
}

struct Entry {
    request: Request,
    stamp: ImageFileStamp,
    asset: Asset,
    used: u64,
}

#[derive(Default)]
struct State {
    generation: u64,
    requests: Vec<Request>,
    processed: usize,
    entries: Vec<Entry>,
    errors: Vec<Value>,
    skipped: usize,
    image_hits: u64,
    video_hits: u64,
    uses: u64,
}

impl State {
    fn bytes(&self) -> usize {
        self.entries.iter().map(|entry| entry.asset.bytes()).sum()
    }
    fn status(&self) -> Value {
        let (metadata_loads, metadata_hits) = crate::media_metadata::cache_counts();
        json!({"generation":self.generation,"pending":self.processed < self.requests.len(),
            "requested":self.requests.len(),"bytes":self.bytes(),"budgetBytes":CACHE_BYTES,
            "imageHits":self.image_hits,"videoHits":self.video_hits,"skipped":self.skipped,
            "metadataLoads":metadata_loads,"metadataHits":metadata_hits,
            "ready":self.entries.iter().map(|entry|json!({"path":entry.request.path,"kind":entry.request.kind,"startSeconds":entry.request.start_seconds})).collect::<Vec<_>>(),
            "errors":self.errors})
    }
}

struct Service {
    state: Mutex<State>,
    changed: Condvar,
}
static SERVICE: OnceLock<Result<Arc<Service>, String>> = OnceLock::new();

fn service() -> Result<&'static Arc<Service>, String> {
    SERVICE
        .get_or_init(|| {
            let service = Arc::new(Service {
                state: Mutex::new(State::default()),
                changed: Condvar::new(),
            });
            let worker = service.clone();
            std::thread::Builder::new()
                .name("orbit-media-preparation".into())
                .spawn(move || run(worker))
                .map_err(|error| format!("Media preparation could not start: {error}"))?;
            Ok(service)
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub(crate) fn prepare(value: Value) -> Result<Value, String> {
    let requests: Vec<Request> = serde_json::from_value(value)
        .map_err(|error| format!("Invalid media preparation request: {error}"))?;
    if requests.len() > MAX_REQUESTS {
        return Err(format!(
            "Media preparation accepts at most {MAX_REQUESTS} items."
        ));
    }
    for request in &requests {
        if !request.path.is_absolute()
            || !matches!(request.kind.as_str(), "image" | "video")
            || !request.start_seconds.is_finite()
            || request.start_seconds < 0.
            || request.kind == "image" && request.start_seconds != 0.
        {
            return Err("Media preparation requires an absolute path, image/video kind and a non-negative start point (zero for images).".into());
        }
    }
    let service = service()?;
    let mut state = service
        .state
        .lock()
        .map_err(|_| "Media preparation state is unavailable.")?;
    state.generation = state.generation.wrapping_add(1);
    state.requests = requests;
    state.processed = 0;
    state.errors.clear();
    state.skipped = 0;
    if state.requests.is_empty() {
        state.entries.clear();
    }
    let status = state.status();
    service.changed.notify_one();
    Ok(status)
}

pub(crate) fn status() -> Result<Value, String> {
    let service = service()?;
    let state = service
        .state
        .lock()
        .map_err(|_| "Media preparation state is unavailable.")?;
    Ok(state.status())
}

fn run(service: Arc<Service>) {
    loop {
        let (generation, index, request) = {
            let Ok(mut state) = service.state.lock() else {
                return;
            };
            while state.processed == state.requests.len() {
                state = match service.changed.wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
            (
                state.generation,
                state.processed,
                state.requests[state.processed].clone(),
            )
        };
        let result = (|| {
            let stamp = image_file_stamp(&request.path)?;
            let mut state = service
                .state
                .lock()
                .map_err(|_| "Media preparation state is unavailable.")?;
            let cached = state
                .entries
                .iter()
                .any(|entry| entry.request == request && entry.stamp == stamp);
            if !cached {
                state.entries.retain(|entry| entry.request != request);
            }
            drop(state);
            if cached {
                return Ok((stamp, None));
            }
            let asset = if request.kind == "image" {
                let mut image =
                    decode_native_image_asset(&request.path, stamp, 0, Some(ASSET_BYTES))?;
                image.prepare_reductions(ASSET_BYTES);
                Asset::Image(image)
            } else {
                let metadata = crate::media_metadata::probe(&json!(request.path))?;
                let width = metadata["width"]
                    .as_f64()
                    .ok_or("Video width is unavailable.")?;
                let height = metadata["height"]
                    .as_f64()
                    .ok_or("Video height is unavailable.")?;
                if width * height * 4. > ASSET_BYTES as f64 {
                    return Err("Video exceeds the media preparation budget.".into());
                }
                let bytes = crate::media_thumbnail::extract_frame(
                    &request.path,
                    request.start_seconds,
                    None,
                )?;
                Asset::Video(Arc::new(
                    image::load_from_memory(&bytes).map_err(|error| error.to_string())?,
                ))
            };
            if image_file_stamp(&request.path)? != stamp {
                return Err("Media changed during preparation.".into());
            }
            Ok::<_, String>((stamp, Some(asset)))
        })();
        let Ok(mut state) = service.state.lock() else {
            return;
        };
        if state.generation != generation {
            continue;
        }
        state.processed += 1;
        match result {
            Ok((stamp, Some(asset))) => {
                state.entries.retain(|entry| entry.request != request);
                while state.bytes() + asset.bytes() > CACHE_BYTES
                    || state.entries.len() >= CACHE_ENTRIES
                {
                    let victim = state
                        .entries
                        .iter()
                        .enumerate()
                        .filter_map(|(entry_index, entry)| {
                            let priority = state
                                .requests
                                .iter()
                                .position(|request| *request == entry.request)
                                .unwrap_or(usize::MAX);
                            (priority > index).then_some((entry_index, priority, entry.used))
                        })
                        .max_by_key(|(_, priority, used)| (*priority, std::cmp::Reverse(*used)))
                        .map(|(index, ..)| index);
                    let Some(victim) = victim else {
                        break;
                    };
                    state.entries.remove(victim);
                }
                if state.bytes() + asset.bytes() <= CACHE_BYTES
                    && state.entries.len() < CACHE_ENTRIES
                {
                    state.uses += 1;
                    let used = state.uses;
                    state.entries.push(Entry {
                        request,
                        stamp,
                        asset,
                        used,
                    });
                } else {
                    state.skipped += 1;
                }
            }
            Ok((_, None)) => {}
            Err(error) => state
                .errors
                .push(json!({"path":request.path,"error":error})),
        }
    }
}

pub(super) fn image(
    path: &Path,
    stamp: ImageFileStamp,
) -> Result<Option<NativeImageAsset>, String> {
    let Some(service) = SERVICE.get() else {
        return Ok(None);
    };
    let service = service.as_ref().map_err(Clone::clone)?;
    let mut state = service
        .state
        .lock()
        .map_err(|_| "Media preparation state is unavailable.")?;
    let Some(index) = state.entries.iter().position(|entry| {
        entry.request.path == path && entry.stamp == stamp && matches!(entry.asset, Asset::Image(_))
    }) else {
        return Ok(None);
    };
    state.uses += 1;
    state.entries[index].used = state.uses;
    state.image_hits += 1;
    Ok(match &state.entries[index].asset {
        Asset::Image(asset) => Some(asset.clone()),
        Asset::Video(_) => None,
    })
}

pub(super) fn video(path: &Path, start_seconds: f64) -> Result<Option<Arc<DynamicImage>>, String> {
    let Some(service) = SERVICE.get() else {
        return Ok(None);
    };
    let service = service.as_ref().map_err(Clone::clone)?;
    let stamp = image_file_stamp(path)?;
    let mut state = service
        .state
        .lock()
        .map_err(|_| "Media preparation state is unavailable.")?;
    let Some(index) = state.entries.iter().position(|entry| {
        entry.request.path == path
            && entry.stamp == stamp
            && entry.request.start_seconds == start_seconds
            && matches!(entry.asset, Asset::Video(_))
    }) else {
        return Ok(None);
    };
    state.uses += 1;
    state.entries[index].used = state.uses;
    state.video_hits += 1;
    Ok(match &state.entries[index].asset {
        Asset::Video(image) => Some(image.clone()),
        Asset::Image(_) => None,
    })
}
