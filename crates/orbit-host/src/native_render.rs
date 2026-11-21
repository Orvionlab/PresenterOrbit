use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Cursor, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Condvar, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};

use cosmic_text::{
    Align, Attrs, Color, Family, FeatureTag, FontFeatures, FontSystem, LineEnding, LineIter, Style,
    SwashCache, Weight,
};
use image::{
    metadata::LoopCount, AnimationDecoder, DynamicImage, ImageDecoder, ImageReader, Limits,
    RgbaImage,
};
use serde_json::{json, Map, Value};

use crate::native_text_layout::{
    decoration_source_index, draw_blocks, layout_text, text_height, underline_glyph_visible,
    underline_reveal_index, with_superscript, ParagraphStyle, TextBlock, TextLayoutSpec, TextRun,
    RUN_STRIKETHROUGH, RUN_UNDERLINE,
};

#[path = "native_media_preload.rs"]
pub(crate) mod media_preload;

pub struct NativeRenderRuntime {
    font_system: FontSystem,
    swash_cache: SwashCache,
    video_available: bool,
    video_decoders: HashMap<PathBuf, VideoDecoder>,
    video_failures: HashMap<PathBuf, VideoFailureState>,
    video_frame_revision: u64,
    pub(crate) prepare_video_start: bool,
    video_error: Arc<Mutex<Option<String>>>,
    video_max_size: Option<(u32, u32)>,
    image_assets: HashMap<PathBuf, NativeImageAsset>,
    scroll_buffer: Vec<u8>,
    pub(crate) stage_preview_frames: HashMap<String, Arc<crate::native_compositor::NativeFrame>>,
    pub(crate) static_images: bool,
    /// What lies beneath the text box being drawn, set while a Text Cut Out box draws its text.
    pub(crate) cut_out_backdrop: Option<CutOutBackdrop>,
}

/// The pixels beneath a text box before its fill was painted. Text Cut Out glyphs cut through
/// the box's fill and reveal them, as ProPresenter's cut-out text shows what is behind the object.
pub struct CutOutBackdrop {
    left: isize,
    top: isize,
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl CutOutBackdrop {
    pub fn capture(
        pixels: &[u8],
        canvas_width: usize,
        canvas_height: usize,
        (left, top, right, bottom): (isize, isize, isize, isize),
    ) -> Self {
        let left = left.clamp(0, canvas_width as isize);
        let top = top.clamp(0, canvas_height as isize);
        let width = (right.clamp(left, canvas_width as isize) - left) as usize;
        let height = (bottom.clamp(top, canvas_height as isize) - top) as usize;
        let mut region = Vec::with_capacity(width * height * 4);
        for y in top as usize..top as usize + height {
            let start = (y * canvas_width + left as usize) * 4;
            region.extend_from_slice(&pixels[start..start + width * 4]);
        }
        Self {
            left,
            top,
            width,
            height,
            pixels: region,
        }
    }

    fn pixel(&self, x: isize, y: isize) -> Option<[u8; 4]> {
        let (x, y) = (x - self.left, y - self.top);
        if x < 0 || y < 0 || x >= self.width as isize || y >= self.height as isize {
            return None;
        }
        let offset = (y as usize * self.width + x as usize) * 4;
        self.pixels[offset..offset + 4].try_into().ok()
    }
}

/// Moves a canvas pixel toward the backdrop by `amount`, mixing premultiplied colors so a
/// transparent backdrop leaves a clean hole in layered output.
fn reveal_backdrop_pixel(
    pixels: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    backdrop: [u8; 4],
    amount: f32,
) {
    let offset = (y * width + x) * 4;
    let amount = amount.clamp(0.0, 1.0);
    let kept = pixels[offset + 3] as f32 / 255.0 * (1.0 - amount);
    let revealed = backdrop[3] as f32 / 255.0 * amount;
    let alpha = kept + revealed;
    for channel in 0..3 {
        let value = pixels[offset + channel] as f32 * kept + backdrop[channel] as f32 * revealed;
        pixels[offset + channel] = if alpha > 0.0 {
            (value / alpha).round() as u8
        } else {
            0
        };
    }
    pixels[offset + 3] = (alpha * 255.0).round() as u8;
}

pub struct NativeImageFrame {
    pub image: Arc<DynamicImage>,
    pub dynamic: bool,
}

#[derive(Clone)]
struct NativeImageAsset {
    frames: Vec<Arc<DynamicImage>>,
    reduced_frames: Vec<Arc<DynamicImage>>,
    frame_ends_seconds: Vec<f64>,
    total_duration_seconds: f64,
    loop_count: LoopCount,
    decoded_bytes: usize,
    stamp: ImageFileStamp,
    started_at: Instant,
    last_used_frame: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ImageFileStamp {
    bytes: u64,
    modified: Option<SystemTime>,
}

struct DecodedVideoFrame {
    image: DynamicImage,
    position: f64,
}

#[derive(Clone, Copy)]
struct MarkerPosition {
    seconds: f64,
    loop_iteration: u64,
    observed_at: Instant,
}

static MARKER_POSITIONS: OnceLock<Mutex<HashMap<(PathBuf, String), MarkerPosition>>> =
    OnceLock::new();
static MEDIA_ENDS: OnceLock<Mutex<HashMap<(PathBuf, String), Instant>>> = OnceLock::new();

pub(crate) fn measured_media_ended(path: &Path, selection_id: &Value, after: Instant) -> bool {
    measured_media_end(path, selection_id, after).is_some()
}

pub(crate) fn measured_media_end(
    path: &Path,
    selection_id: &Value,
    after: Instant,
) -> Option<Instant> {
    MEDIA_ENDS
        .get()
        .and_then(|ends| ends.lock().ok())
        .and_then(|ends| {
            ends.get(&(path.to_path_buf(), selection_id.to_string()))
                .copied()
                .filter(|ended| *ended >= after)
        })
}

pub(crate) fn publish_media_ended(path: &Path, selection_id: &Value) {
    let now = Instant::now();
    if let Ok(mut ends) = MEDIA_ENDS.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        ends.retain(|_, ended| now.saturating_duration_since(*ended) < Duration::from_secs(300));
        if ends.len() < 256 {
            ends.entry((path.to_path_buf(), selection_id.to_string()))
                .or_insert(now);
        }
    }
}

pub(crate) fn publish_loop_marker_position(
    path: &Path,
    selection_id: &Value,
    seconds: f64,
    loop_iteration: u64,
) {
    insert_marker_position(
        path,
        selection_id.to_string(),
        seconds,
        loop_iteration,
        Instant::now(),
    );
}

fn insert_marker_position(
    path: &Path,
    selection: String,
    seconds: f64,
    loop_iteration: u64,
    observed_at: Instant,
) {
    let now = Instant::now();
    if let Ok(mut positions) = MARKER_POSITIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        positions.retain(|_, position| {
            now.saturating_duration_since(position.observed_at) < Duration::from_secs(5)
        });
        let key = (path.to_path_buf(), selection);
        if positions.len() < 256 || positions.contains_key(&key) {
            positions.insert(
                key,
                MarkerPosition {
                    seconds,
                    loop_iteration,
                    observed_at,
                },
            );
        }
    }
}

/// The marker positions measured in this process, for a controller that schedules markers but does not decode
/// the media. `selection` is the key form the scheduler measures with.
pub(crate) fn marker_positions() -> Value {
    let now = Instant::now();
    let positions = MARKER_POSITIONS
        .get()
        .and_then(|positions| positions.lock().ok())
        .map(|positions| {
            positions
                .iter()
                .filter(|(_, position)| {
                    now.saturating_duration_since(position.observed_at) < Duration::from_secs(5)
                })
                .map(|((path, selection), position)| {
                    json!({
                        "path": path.to_string_lossy(),
                        "selection": selection,
                        "seconds": position.seconds,
                        "loopIteration": position.loop_iteration,
                        "ageMs": now.saturating_duration_since(position.observed_at).as_millis() as u64,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Value::Array(positions)
}

/// Takes the positions another process measured, aged by the time they spent in transit.
pub(crate) fn accept_marker_positions(positions: &Value) -> Result<Value, String> {
    let entries = positions
        .as_array()
        .ok_or("Marker positions must be an array.")?;
    let now = Instant::now();
    for entry in entries {
        let path = entry["path"]
            .as_str()
            .ok_or("A marker position has no path.")?;
        let selection = entry["selection"]
            .as_str()
            .ok_or("A marker position has no selection.")?;
        let seconds = entry["seconds"]
            .as_f64()
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            .ok_or("A marker position is not a finite time.")?;
        let iteration = entry["loopIteration"]
            .as_u64()
            .ok_or("A marker position has no loop iteration.")?;
        let age = Duration::from_millis(entry["ageMs"].as_u64().unwrap_or(0));
        insert_marker_position(
            Path::new(path),
            selection.to_string(),
            seconds,
            iteration,
            now.checked_sub(age).unwrap_or(now),
        );
    }
    Ok(json!({ "ok": true, "accepted": entries.len() }))
}

pub(crate) fn measured_marker_sample(
    path: &Path,
    selection_id: &Value,
    after: Instant,
) -> Option<(f64, u64)> {
    let positions = MARKER_POSITIONS.get()?.lock().ok()?;
    let position = positions.get(&(path.to_path_buf(), selection_id.to_string()))?;
    (position.observed_at >= after).then_some((position.seconds, position.loop_iteration))
}

fn decoded_frame_position(line: &str) -> Option<f64> {
    if !line.contains("showinfo") {
        return None;
    }
    let value = line.split_once("pts_time:")?.1.split_whitespace().next()?;
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn video_crossed_trim_boundary(
    position: f64,
    playback_state: Option<&Map<String, Value>>,
    rate: f64,
) -> bool {
    if rate < 0.0 {
        playback_state
            .and_then(|state| state.get("inSeconds"))
            .and_then(json_number)
            .is_some_and(|start| position <= start)
    } else {
        playback_state
            .and_then(|state| state.get("outSeconds"))
            .and_then(json_number)
            .is_some_and(|end| position >= end)
    }
}

/// Where a decoded video is, for fills that apply their own playback rules.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoProgress {
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub ended: bool,
}

/// FFmpeg's input summary line, e.g. `  Duration: 00:01:02.50, start: 0.000000`.
fn reported_duration(line: &str) -> Option<f64> {
    let value = line
        .trim_start()
        .strip_prefix("Duration: ")?
        .split(',')
        .next()?;
    let mut seconds = 0.0;
    for part in value.split(':') {
        seconds = seconds * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    (seconds.is_finite() && seconds > 0.0).then_some(seconds)
}

struct VideoDecoder {
    receiver: mpsc::Receiver<DecodedVideoFrame>,
    /// The source duration in seconds as `f64` bits; zero until FFmpeg reports it.
    duration: Arc<AtomicU64>,
    ended: bool,
    latest: Option<Arc<DynamicImage>>,
    prepared_frame: bool,
    latest_position: Option<f64>,
    clock_revision: u64,
    selection_id: Option<Value>,
    last_used_frame: u64,
    cancellation: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    worker: Option<JoinHandle<()>>,
    started_at: Instant,
    successful_eof: Arc<AtomicBool>,
    completion_published: bool,
    reverse_control: Option<Arc<ReverseVideoControl>>,
    #[cfg(test)]
    reverse_sampler_done: Option<Arc<AtomicBool>>,
    decode_error: Arc<Mutex<Option<String>>>,
}

struct ReverseVideoControl {
    target_seconds: Mutex<f64>,
    changed: Condvar,
    samples_started: AtomicU64,
}

const REVERSE_VIDEO_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

const MAX_ACTIVE_VIDEO_DECODERS: usize = 4;
const MAX_ACTIVE_IMAGE_ASSETS: usize = 64;
const MAX_ANIMATED_IMAGE_FRAMES: usize = 512;
const IMAGE_CACHE_TARGET_BYTES: usize = 256 * 1024 * 1024;
const MIN_ANIMATED_FRAME_SECONDS: f64 = 0.01;
const MAX_VIDEO_FRAME_BYTES: usize = 128 * 1024 * 1024;
const FIRST_VIDEO_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const VIDEO_RETRY_BACKOFF: Duration = Duration::from_secs(5);
const MAX_VIDEO_DECODE_ATTEMPTS: u32 = 6;

struct VideoFailureState {
    attempts: u32,
    retry_after: Instant,
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Release);
        if let Some(control) = &self.reverse_control {
            control.changed.notify_all();
        }
        if let Ok(mut child) = self.child.lock() {
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
        }
        // The FFmpeg worker owns the remaining child handle and exits after
        // the kill closes its stdout. Do not join it on the render path: a
        // media transition must not wait for process cleanup.
        self.worker.take();
    }
}

impl NativeRenderRuntime {
    pub fn new() -> Self {
        Self::new_with_video_max_size(None)
    }

    pub fn new_with_video_max_size(video_max_size: Option<(u32, u32)>) -> Self {
        let video_available = ffmpeg_sidecar::command::ffmpeg_is_installed();
        Self::with_resources(FontSystem::new(), video_available, video_max_size)
    }

    pub fn new_static_preview() -> Self {
        static RESOURCES: OnceLock<(String, cosmic_text::fontdb::Database, bool)> = OnceLock::new();
        let (locale, database, video_available) = RESOURCES.get_or_init(|| {
            let (locale, database) = FontSystem::new().into_locale_and_db();
            (
                locale,
                database,
                ffmpeg_sidecar::command::ffmpeg_is_installed(),
            )
        });
        let mut renderer = Self::with_resources(
            FontSystem::new_with_locale_and_db(locale.clone(), database.clone()),
            *video_available,
            None,
        );
        renderer.static_images = true;
        renderer
    }

    fn with_resources(
        font_system: FontSystem,
        video_available: bool,
        video_max_size: Option<(u32, u32)>,
    ) -> Self {
        let video_error = if video_available {
            None
        } else {
            Some("FFmpeg was not found beside Presenter Orbit or on PATH.".to_string())
        };
        Self {
            font_system,
            swash_cache: SwashCache::new(),
            video_available,
            video_decoders: HashMap::new(),
            video_failures: HashMap::new(),
            video_frame_revision: 0,
            prepare_video_start: false,
            video_error: Arc::new(Mutex::new(video_error)),
            video_max_size,
            image_assets: HashMap::new(),
            scroll_buffer: Vec::new(),
            stage_preview_frames: HashMap::new(),
            static_images: false,
            cut_out_backdrop: None,
        }
    }

    pub fn video_available(&self) -> bool {
        self.video_available
    }

    pub fn set_video_max_size(&mut self, video_max_size: Option<(u32, u32)>) {
        if self.video_max_size == video_max_size {
            return;
        }
        self.video_max_size = video_max_size;
        self.video_decoders.clear();
    }

    pub fn video_error(&self) -> String {
        self.video_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
            .unwrap_or_default()
    }

    pub fn begin_frame(&mut self) {
        self.video_frame_revision = self.video_frame_revision.wrapping_add(1);
    }

    pub(crate) fn video_frame_ready(&self, path: &Path) -> bool {
        self.video_decoders
            .get(path)
            .is_some_and(|decoder| decoder.latest_position.is_some() || decoder.prepared_frame)
    }

    pub fn end_frame(&mut self) {
        let current_frame = self.video_frame_revision;
        self.video_decoders
            .retain(|_, decoder| decoder.last_used_frame == current_frame);
        self.video_failures.retain(|path, _| path.is_file());
    }

    pub fn image_frame(&mut self, path: &Path) -> Result<NativeImageFrame, String> {
        let stamp = image_file_stamp(path)?;
        let reload = self
            .image_assets
            .get(path)
            .is_some_and(|asset| asset.stamp != stamp);
        if reload {
            self.image_assets.remove(path);
        }
        if !self.image_assets.contains_key(path) {
            let mut asset = match media_preload::image(path, stamp)? {
                Some(asset) => asset,
                None => decode_native_image_asset(path, stamp, self.video_frame_revision, None)?,
            };
            asset.started_at = Instant::now();
            self.reserve_image_capacity(asset.decoded_bytes)?;
            self.image_assets.insert(path.to_path_buf(), asset);
        }
        let asset = self
            .image_assets
            .get_mut(path)
            .expect("native image asset inserted");
        asset.last_used_frame = self.video_frame_revision;
        let elapsed_seconds = asset.started_at.elapsed().as_secs_f64();
        let (frame_index, dynamic) = if self.static_images {
            (0, false)
        } else {
            asset.frame_at(elapsed_seconds)
        };
        Ok(NativeImageFrame {
            image: Arc::clone(&asset.frames[frame_index]),
            dynamic,
        })
    }

    pub fn image_frame_for_size(
        &mut self,
        path: &Path,
        width: usize,
        height: usize,
    ) -> Result<NativeImageFrame, String> {
        let mut frame = self.image_frame(path)?;
        if !frame.dynamic {
            let factor = (width as f64 / frame.image.width() as f64)
                .min(height as f64 / frame.image.height() as f64)
                .min(1.);
            let needed_width = (frame.image.width() as f64 * factor).ceil() as u32;
            let needed_height = (frame.image.height() as f64 * factor).ceil() as u32;
            if let Some(image) =
                self.image_assets.get(path).and_then(|asset| {
                    asset.reduced_frames.iter().rev().find(|image| {
                        image.width() >= needed_width && image.height() >= needed_height
                    })
                })
            {
                frame.image = image.clone();
            }
        }
        Ok(frame)
    }

    fn reserve_image_capacity(&mut self, decoded_bytes: usize) -> Result<(), String> {
        let current_frame = self.video_frame_revision;
        while self.image_assets.len() >= MAX_ACTIVE_IMAGE_ASSETS
            || self.active_image_bytes().saturating_add(decoded_bytes) > IMAGE_CACHE_TARGET_BYTES
        {
            let stale = self
                .image_assets
                .iter()
                .filter(|(_, asset)| asset.last_used_frame != current_frame)
                .min_by_key(|(_, asset)| asset.last_used_frame)
                .map(|(path, _)| path.clone());
            let Some(stale) = stale else {
                break;
            };
            self.image_assets.remove(&stale);
        }
        if self.image_assets.len() >= MAX_ACTIVE_IMAGE_ASSETS {
            return Err(format!(
                "Native image cache limit of {MAX_ACTIVE_IMAGE_ASSETS} active assets was reached."
            ));
        }
        Ok(())
    }

    fn active_image_bytes(&self) -> usize {
        self.image_assets
            .values()
            .map(|asset| asset.decoded_bytes)
            .sum()
    }

    /// Decode the frame stream from the Rust-owned output clock. A playback
    /// revision restarts the decoder at a seek/transport boundary. Paused clocks
    /// retain the last frame. Estimated ended clocks continue draining until
    /// successful decoded EOF or a consumed trim boundary confirms completion.
    pub fn video_frame_with_clock(
        &mut self,
        path: &Path,
        playback_state: Option<&Map<String, Value>>,
    ) -> Result<Option<&DynamicImage>, String> {
        if !self.video_available {
            return Err(
                "Native video decoding requires FFmpeg beside Presenter Orbit or on PATH."
                    .to_string(),
            );
        }
        if let Some(error) = playback_state
            .and_then(|state| state.get("error"))
            .and_then(Value::as_str)
        {
            return Err(error.to_string());
        }
        let clock_revision = playback_state
            .and_then(|state| state.get("clockRevision"))
            .and_then(json_number)
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value as u64)
            .unwrap_or(0);
        let selection_id = playback_state
            .and_then(|state| state.get("selectionId"))
            .cloned();
        let current_time = playback_state
            .and_then(|state| state.get("currentTime"))
            .and_then(json_number)
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(0.0);
        let status = playback_state
            .and_then(|state| state.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("playing");
        let rate = playback_state
            .and_then(|state| state.get("rate"))
            .and_then(json_number)
            .unwrap_or(1.0);
        if playback_state
            .and_then(|state| state.get("delayed"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            return Ok(None);
        }
        if let Some(failure) = self.video_failures.get(path) {
            if failure.attempts >= MAX_VIDEO_DECODE_ATTEMPTS {
                return Err(
                    "Native video decoder retry limit was reached for this source.".to_string(),
                );
            }
            if failure.retry_after > Instant::now() {
                return Err(
                    "Native video decoder is waiting before retrying this source.".to_string(),
                );
            }
        }
        let now = Instant::now();
        let first_frame_timed_out = self.video_decoders.get(path).is_some_and(|decoder| {
            video_first_frame_timed_out(decoder.latest_position.is_some(), decoder.started_at, now)
        });
        if first_frame_timed_out {
            self.video_decoders.remove(path);
            self.record_video_failure(path);
            return Err("Native video decoder timed out before its first frame.".to_string());
        }
        let restart = self.video_decoders.get(path).is_some_and(|decoder| {
            decoder.clock_revision != clock_revision || decoder.selection_id != selection_id
        });
        let previous_frame = restart
            .then(|| {
                self.video_decoders
                    .remove(path)
                    .and_then(|mut decoder| decoder.latest.take())
            })
            .flatten();
        if !self.video_decoders.contains_key(path) {
            if let Some(id) = playback_state.and_then(|state| state.get("selectionId")) {
                if let Some(ends) = MEDIA_ENDS.get() {
                    if let Ok(mut ends) = ends.lock() {
                        ends.remove(&(path.to_path_buf(), id.to_string()));
                    }
                }
            }
            if self.video_decoders.len() >= MAX_ACTIVE_VIDEO_DECODERS {
                let stale_path = self
                    .video_decoders
                    .iter()
                    .filter(|(_, decoder)| decoder.last_used_frame != self.video_frame_revision)
                    .min_by_key(|(_, decoder)| decoder.last_used_frame)
                    .map(|(path, _)| path.clone());
                if let Some(stale_path) = stale_path {
                    self.video_decoders.remove(&stale_path);
                }
            }
            if self.video_decoders.len() >= MAX_ACTIVE_VIDEO_DECODERS {
                return Err(format!(
                    "Native video decoder limit of {MAX_ACTIVE_VIDEO_DECODERS} active layers was reached."
                ));
            }
            let decoder_result = if rate < 0.0 {
                spawn_reverse_video_decoder(
                    path.to_path_buf(),
                    current_time,
                    clock_revision,
                    self.video_frame_revision,
                    Arc::clone(&self.video_error),
                    self.video_max_size,
                )
            } else {
                spawn_video_decoder(
                    path.to_path_buf(),
                    current_time,
                    rate,
                    playback_state.is_none()
                        || playback_state
                            .and_then(|state| state.get("loop"))
                            .and_then(Value::as_bool)
                            == Some(true),
                    clock_revision,
                    self.video_frame_revision,
                    Arc::clone(&self.video_error),
                    self.video_max_size,
                )
            };
            let mut decoder = match decoder_result {
                Ok(decoder) => decoder,
                Err(error) => {
                    self.record_video_failure(path);
                    return Err(error);
                }
            };
            decoder.selection_id = selection_id;
            let prepare_start =
                if playback_state.is_some_and(|state| state.contains_key("preparedStart")) {
                    std::mem::take(&mut self.prepare_video_start)
                } else {
                    false
                };
            let prepared = if prepare_start
                && rate > 0.0
                && playback_state
                    .is_some_and(|state| state.get("preparedStart") == Some(&Value::Bool(true)))
            {
                playback_state
                    .and_then(|state| state.get("inSeconds"))
                    .and_then(json_number)
                    .map(|start| media_preload::video(path, start))
                    .transpose()?
                    .flatten()
            } else {
                None
            };
            decoder.prepared_frame = prepared.is_some();
            decoder.latest = prepared.or(previous_frame);
            self.video_decoders.insert(path.to_path_buf(), decoder);
        }
        if let Some(error) = self
            .video_decoders
            .get(path)
            .and_then(|decoder| decoder.decode_error.lock().ok()?.clone())
        {
            return Err(error);
        }
        let mut reached_trim_end = false;
        let mut decoder_drained = false;
        let frame_received = {
            let decoder = self.video_decoders.get_mut(path).expect("decoder inserted");
            decoder.last_used_frame = self.video_frame_revision;
            if let Some(control) = &decoder.reverse_control {
                if let Ok(mut target) = control.target_seconds.lock() {
                    if target.to_bits() != current_time.to_bits() {
                        *target = current_time;
                        control.changed.notify_one();
                    }
                }
                let mut received = false;
                loop {
                    let frame = match decoder.receiver.try_recv() {
                        Ok(frame) => frame,
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            decoder_drained = true;
                            break;
                        }
                    };
                    if video_crossed_trim_boundary(frame.position, playback_state, rate) {
                        reached_trim_end = true;
                        continue;
                    }
                    decoder.latest_position = Some(frame.position);
                    decoder.latest = Some(Arc::new(frame.image));
                    received = true;
                }
                received
            } else if status == "paused" {
                if decoder.latest_position.is_none() {
                    if let Ok(frame) = decoder.receiver.try_recv() {
                        decoder.latest_position = Some(frame.position);
                        decoder.latest = Some(Arc::new(frame.image));
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                let mut received = false;
                loop {
                    let frame = match decoder.receiver.try_recv() {
                        Ok(frame) => frame,
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            decoder_drained = true;
                            break;
                        }
                    };
                    if video_crossed_trim_boundary(frame.position, playback_state, rate) {
                        reached_trim_end = true;
                        continue;
                    }
                    decoder.latest_position = Some(frame.position);
                    decoder.latest = Some(Arc::new(frame.image));
                    received = true;
                }
                received
            }
        };
        if decoder_drained {
            let decoder = self.video_decoders.get_mut(path).expect("decoder inserted");
            decoder.ended = decoder.successful_eof.load(Ordering::Acquire);
        }
        if status != "paused" && playback_state.is_some() {
            let decoder = self.video_decoders.get_mut(path).expect("decoder inserted");
            if !decoder.completion_published
                && (reached_trim_end
                    || (decoder_drained
                        && decoder.latest.is_some()
                        && decoder.successful_eof.load(Ordering::Acquire)))
            {
                if let Some(selection_id) =
                    playback_state.and_then(|state| state.get("selectionId"))
                {
                    publish_media_ended(path, selection_id);
                    decoder.completion_published = true;
                }
            }
        }
        if frame_received {
            if let Some(decoder) = self.video_decoders.get_mut(path) {
                decoder.prepared_frame = false;
            }
            self.mark_video_frame_ready(path);
            if status == "playing" {
                if let Some(selection_id) =
                    playback_state.and_then(|state| state.get("selectionId"))
                {
                    if let Some(position) = self
                        .video_decoders
                        .get(path)
                        .and_then(|decoder| decoder.latest_position)
                    {
                        let duration = playback_state
                            .and_then(|state| state.get("duration"))
                            .and_then(json_number)
                            .filter(|value| *value > 0.0);
                        publish_loop_marker_position(
                            path,
                            selection_id,
                            duration.map_or(position, |duration| position % duration),
                            playback_state
                                .and_then(|state| state.get("loopIteration"))
                                .and_then(Value::as_u64)
                                .ok_or("Native marker loop iteration is required.")?,
                        );
                    }
                }
            }
        }
        Ok(self
            .video_decoders
            .get(path)
            .and_then(|decoder| decoder.latest.as_deref()))
    }

    pub fn video_progress(&self, path: &Path) -> Option<VideoProgress> {
        self.video_decoders.get(path).map(|decoder| VideoProgress {
            position: decoder.latest_position,
            duration: Some(f64::from_bits(decoder.duration.load(Ordering::Acquire)))
                .filter(|seconds| *seconds > 0.0),
            ended: decoder.ended,
        })
    }

    fn mark_video_frame_ready(&mut self, path: &Path) {
        self.video_failures.remove(path);
    }

    fn record_video_failure(&mut self, path: &Path) {
        let attempts = self
            .video_failures
            .get(path)
            .map_or(1, |failure| failure.attempts.saturating_add(1));
        let multiplier = 2_u32.pow(attempts.saturating_sub(1).min(5));
        self.video_failures.insert(
            path.to_path_buf(),
            VideoFailureState {
                attempts,
                retry_after: Instant::now() + VIDEO_RETRY_BACKOFF * multiplier,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_box(
        &mut self,
        pixels: &mut [u8],
        canvas_width: usize,
        canvas_height: usize,
        text: &str,
        left: isize,
        top: isize,
        box_width: usize,
        box_height: usize,
        font_size: f32,
        bgra: [u8; 4],
        style: &Map<String, Value>,
    ) -> bool {
        self.draw_text_box_with_media(
            pixels,
            canvas_width,
            canvas_height,
            text,
            left,
            top,
            box_width,
            box_height,
            font_size,
            bgra,
            style,
            None,
            Some((box_width, box_height)),
        )
    }

    /// Draw a scrolling text layer into the caller-owned framebuffer.
    ///
    /// The text is rasterized once into a bounded native surface and then
    /// translated from the Rust monotonic output clock. Keeping the animation
    /// here means scrolling retains the same text effects, media fill, stroke,
    /// and shadow pipeline as static text without relying on a DOM animation.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_scrolling_text_box_with_media(
        &mut self,
        pixels: &mut [u8],
        canvas_width: usize,
        canvas_height: usize,
        text: &str,
        left: isize,
        top: isize,
        box_width: usize,
        box_height: usize,
        font_size: f32,
        bgra: [u8; 4],
        style: &Map<String, Value>,
        slide_width: f64,
        elapsed_seconds: f64,
        media_fill: Option<&DynamicImage>,
    ) -> bool {
        if text.is_empty() || box_width == 0 || box_height == 0 || font_size <= 0.0 {
            return false;
        }

        let (intrinsic_width, intrinsic_height) = self.measure_text_extent(text, font_size, style);
        let direction = scroll_direction(style);
        let ticker = crate::native_ticker::is_enabled_ticker(style);
        let copy_width =
            if ticker && matches!(direction, ScrollDirection::Left | ScrollDirection::Right) {
                intrinsic_width
            } else {
                box_width.max(intrinsic_width)
            }
            .max(1);
        let copy_height =
            if ticker && matches!(direction, ScrollDirection::Up | ScrollDirection::Down) {
                intrinsic_height
            } else {
                box_height.max(intrinsic_height)
            }
            .max(1);
        let Some((copy_width, copy_height)) = bounded_scroll_dimensions(copy_width, copy_height)
        else {
            return false;
        };
        let Some(copy_pixels) = copy_width
            .checked_mul(copy_height)
            .and_then(|pixels| pixels.checked_mul(4))
        else {
            return false;
        };

        let mut scroll_buffer = std::mem::take(&mut self.scroll_buffer);
        scroll_buffer.resize(copy_pixels, 0);
        scroll_buffer.fill(0);

        let mut static_style = style.clone();
        static_style.insert("scrollingEnabled".to_string(), Value::Bool(false));
        // A scrolling cut-out copy cannot reach the backdrop from its buffer: its stroke and
        // shadow are drawn as usual, and its glyph coverage reveals the backdrop when blitted.
        let cut_out = self.cut_out_backdrop.take();
        let mut coverage = None;
        if cut_out.is_some() {
            static_style.insert("textFillMode".into(), Value::from("Text Color"));
            let mut glyphs_style = static_style.clone();
            glyphs_style.insert("strokeEnabled".into(), Value::Bool(false));
            glyphs_style.insert("shadowEnabled".into(), Value::Bool(false));
            let mut glyphs = vec![0; copy_pixels];
            if self.draw_text_box_with_media(
                &mut glyphs,
                copy_width,
                copy_height,
                text,
                0,
                0,
                copy_width,
                copy_height,
                font_size,
                [255, 255, 255, 255],
                &glyphs_style,
                None,
                None,
            ) {
                coverage = Some(glyphs);
            }
        }
        let rendered = self.draw_text_box_with_media(
            &mut scroll_buffer,
            copy_width,
            copy_height,
            text,
            0,
            0,
            copy_width,
            copy_height,
            font_size,
            if cut_out.is_some() {
                [0, 0, 0, 0]
            } else {
                bgra
            },
            &static_style,
            media_fill,
            Some((box_width, box_height)),
        ) || coverage.is_some();
        self.cut_out_backdrop = cut_out;
        if !rendered {
            self.scroll_buffer = scroll_buffer;
            return false;
        }

        let repeat_enabled = style.get("scrollRepeat").and_then(Value::as_bool) == Some(true);
        let loop_delay = style
            .get("scrollLoopDelaySeconds")
            .and_then(Value::as_f64)
            .filter(|delay| repeat_enabled && delay.is_finite() && *delay > 0.0);
        let repeated = repeat_enabled && loop_delay.is_none();
        let offscreen_start =
            style.get("scrollStart").and_then(Value::as_str) == Some("Off Screen");
        let gap = style_number(style, "scrollRepeatDistance", 100.0) as f64
            * style_number(style, "_fontScale", 1.0) as f64;
        let gap = gap.max(0.0);
        let copy_extent = match direction {
            ScrollDirection::Left | ScrollDirection::Right => copy_width as f64,
            ScrollDirection::Up | ScrollDirection::Down => copy_height as f64,
        };
        let (offset, second_offset, repeated, copies) = if ticker {
            let viewport = match direction {
                ScrollDirection::Left | ScrollDirection::Right => box_width as f64,
                ScrollDirection::Up | ScrollDirection::Down => box_height as f64,
            };
            let Some(frame) = crate::native_ticker_motion::frame(
                style,
                viewport,
                copy_extent,
                canvas_width as f64 / slide_width.max(1.0),
                elapsed_seconds,
            ) else {
                self.scroll_buffer = scroll_buffer;
                return true;
            };
            (
                frame.offset,
                frame.second_offset,
                frame.repeated,
                frame.copies,
            )
        } else {
            let total_extent = if repeated {
                copy_extent * 2.0 + gap
            } else {
                copy_extent
            };
            let cycle_extent = copy_extent + gap;
            let (mut start, mut end) = match direction {
                ScrollDirection::Left | ScrollDirection::Up => (
                    if offscreen_start { total_extent } else { 0.0 },
                    if repeated {
                        -cycle_extent
                    } else {
                        -copy_extent
                    },
                ),
                ScrollDirection::Right | ScrollDirection::Down => (
                    if offscreen_start {
                        -total_extent
                    } else if repeated {
                        -cycle_extent
                    } else {
                        0.0
                    },
                    if repeated { 0.0 } else { copy_extent },
                ),
            };
            let duration = scrolling_duration_seconds(style);
            let elapsed = if elapsed_seconds.is_finite() {
                elapsed_seconds.max(0.0)
            } else {
                0.0
            };
            let elapsed = if let Some(delay) = loop_delay {
                let phase = elapsed.rem_euclid(duration + delay);
                if phase >= duration {
                    self.scroll_buffer = scroll_buffer;
                    return true;
                }
                let viewport_extent = match direction {
                    ScrollDirection::Left | ScrollDirection::Right => box_width as f64,
                    ScrollDirection::Up | ScrollDirection::Down => box_height as f64,
                };
                (start, end) = match direction {
                    ScrollDirection::Left | ScrollDirection::Up => (viewport_extent, -copy_extent),
                    ScrollDirection::Right | ScrollDirection::Down => {
                        (-copy_extent, viewport_extent)
                    }
                };
                phase
            } else {
                elapsed
            };
            let progress = if repeated {
                (elapsed.rem_euclid(duration) / duration).clamp(0.0, 1.0)
            } else {
                (elapsed / duration).clamp(0.0, 1.0)
            };
            let offset = start + (end - start) * progress;
            (offset, offset + cycle_extent, repeated, 2)
        };
        let (offset_x, offset_y) = match direction {
            ScrollDirection::Left | ScrollDirection::Right => (offset, 0.0),
            ScrollDirection::Up | ScrollDirection::Down => (0.0, offset),
        };
        let (second_x, second_y) = match direction {
            ScrollDirection::Left | ScrollDirection::Right => (second_offset, 0.0),
            ScrollDirection::Up | ScrollDirection::Down => (0.0, second_offset),
        };
        let slide_width = slide_width.max(1.0);
        let feather_extent = match direction {
            ScrollDirection::Left | ScrollDirection::Right => box_width as f64,
            ScrollDirection::Up | ScrollDirection::Down => box_height as f64,
        };
        let feather_left =
            (style_number(style, "scrollFeatherLeft", 0.0).max(0.0) as f64 / slide_width).min(0.5)
                * feather_extent;
        let feather_right =
            (style_number(style, "scrollFeatherRight", 0.0).max(0.0) as f64 / slide_width).min(0.5)
                * feather_extent;

        let copy_count = if repeated { copies.max(1) } else { 1 };
        for copy in 0..copy_count {
            let offset = (
                (offset_x + (second_x - offset_x) * copy as f64).round() as isize,
                (offset_y + (second_y - offset_y) * copy as f64).round() as isize,
            );
            let mut blit = |source: &[u8], backdrop: Option<&CutOutBackdrop>| {
                blit_scrolling_copy(
                    pixels,
                    canvas_width,
                    canvas_height,
                    source,
                    copy_width,
                    copy_height,
                    left,
                    top,
                    offset.0,
                    offset.1,
                    box_width,
                    box_height,
                    feather_left,
                    feather_right,
                    backdrop,
                );
            };
            blit(&scroll_buffer, None);
            if let Some(coverage) = coverage.as_deref() {
                blit(coverage, self.cut_out_backdrop.as_ref());
            }
        }
        self.scroll_buffer = scroll_buffer;
        // A valid off-screen frame owns blank output and must suppress static cue fallback.
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_box_with_media(
        &mut self,
        pixels: &mut [u8],
        canvas_width: usize,
        canvas_height: usize,
        text: &str,
        left: isize,
        top: isize,
        box_width: usize,
        box_height: usize,
        font_size: f32,
        bgra: [u8; 4],
        style: &Map<String, Value>,
        media_fill: Option<&DynamicImage>,
        gradient_object_bounds: Option<(usize, usize)>,
    ) -> bool {
        if text.is_empty() || box_width == 0 || box_height == 0 || font_size <= 0.0 {
            return false;
        }

        let scale = text_scale(style, font_size);
        let (blocks, runs) = self.wrapped_text_blocks(text, box_width, font_size, style);

        let content_height = text_height(&blocks).ceil() as isize;
        let free_height = box_height as isize - content_height;
        let vertical_offset = match style
            .get("textVerticalAlign")
            .and_then(Value::as_str)
            .unwrap_or("middle")
            .to_ascii_lowercase()
            .as_str()
        {
            "top" => 0,
            "bottom" => free_height.max(0),
            _ => free_height.max(0) / 2,
        };
        let gradient_bounds =
            text_gradient_layout_bounds(&blocks, vertical_offset, box_width, box_height);

        let default_color = Color::rgba(bgra[2], bgra[1], bgra[0], bgra[3]);
        let mut masks = TextMasks::new(
            box_width.saturating_mul(box_height),
            runs.iter().any(|run| run.color.is_some())
                || ["underlineColor", "strikethroughColor"]
                    .iter()
                    .any(|key| style.get(*key).and_then(Value::as_str).is_some())
                || style
                    .get("_textRuns")
                    .and_then(Value::as_array)
                    .is_some_and(|runs| {
                        runs.iter().any(|run| {
                            ["underlineColor", "strikethroughColor"].iter().any(|key| {
                                run.get("style")
                                    .and_then(|style| style.get(*key))
                                    .and_then(Value::as_str)
                                    .is_some()
                            })
                        })
                    }),
        );
        let child_opacities = text_reveal_child_opacities(style);
        let paragraph_opacities =
            text_reveal_paragraph_opacities(text, style, child_opacities.as_ref());
        let visible_underlines = text_reveal_visible_underlines(style);
        let isolated_child = style
            .get("_textRevealIsolatedChild")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok());
        let isolated_underline_child = (style.get("_textRevealMode").and_then(Value::as_str)
            == Some("underline"))
        .then_some(isolated_child)
        .flatten();
        let base_underline = style.get("underline").and_then(Value::as_bool) == Some(true);
        let child_underline_opacities = (style.get("_textRevealMode").and_then(Value::as_str)
            == Some("underline"))
        .then_some(child_opacities.as_ref())
        .flatten();
        let initial_visible_underlines = if child_underline_opacities.is_some() {
            style
                .get("_textRevealChildInitialVisible")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize
        } else {
            0
        };
        let mut fragments = Vec::new();
        for block in blocks.iter().filter(|block| {
            paragraph_opacity(paragraph_opacities.as_deref(), block.paragraph_index) > 0.0
        }) {
            draw_blocks(
                std::slice::from_ref(block),
                &mut self.font_system,
                &mut self.swash_cache,
                default_color,
                visible_underlines,
                base_underline,
                paragraph_opacities.as_deref(),
                child_underline_opacities,
                initial_visible_underlines,
                isolated_underline_child,
                |x, y, width, height, color, opacity| {
                    fragments.push((x, y, width, height, color, opacity))
                },
            );
        }
        for (x, y, width, height, color, opacity) in fragments {
            let (_, _, _, alpha) = color.as_rgba_tuple();
            let alpha = (alpha as f32 * opacity).round() as u8;
            for row in 0..height as isize {
                for column in 0..width as isize {
                    let mask_x = x as isize + column;
                    let mask_y = vertical_offset + y as isize + row;
                    if mask_x < 0
                        || mask_y < 0
                        || mask_x >= box_width as isize
                        || mask_y >= box_height as isize
                    {
                        continue;
                    }
                    masks.put(
                        mask_y as usize * box_width + mask_x as usize,
                        alpha,
                        (color != default_color).then_some(color),
                        bgra[3],
                    );
                }
            }
        }
        append_text_decorations(
            &mut masks,
            box_width,
            box_height,
            &blocks,
            vertical_offset,
            style,
            bgra[3],
            paragraph_opacities.as_deref(),
            visible_underlines,
            base_underline,
            child_underline_opacities,
            initial_visible_underlines,
            isolated_underline_child,
        );
        let TextMasks {
            all: mask,
            fill,
            overlays,
        } = masks;

        let layer_opacity = style_number(style, "_layerOpacity", 1.0).clamp(0.0, 1.0);
        let mut rendered = false;
        if style.get("shadowEnabled").and_then(Value::as_bool) == Some(true) {
            let shadow_offset = style_number(style, "shadowOffset", 5.0) * scale;
            let shadow_angle = style_number(style, "shadowAngle", 315.0).to_radians();
            let shadow_x = (shadow_angle.cos() * shadow_offset).round() as isize;
            let shadow_y = (-shadow_angle.sin() * shadow_offset).round() as isize;
            let blur_radius = (style_number(style, "shadowBlur", 5.0) * scale)
                .round()
                .clamp(0.0, 64.0) as usize;
            let (padded_mask, padded_width, padded_height) =
                pad_alpha_mask(&mask, box_width, box_height, blur_radius);
            let shadow_mask =
                blur_alpha_mask(&padded_mask, padded_width, padded_height, blur_radius);
            let mut shadow_color = style_color(style, "shadowColor", [0, 0, 0, 255]);
            shadow_color[3] = ((shadow_color[3] as f32
                * style_number(style, "shadowOpacity", 75.0).clamp(0.0, 100.0)
                / 100.0
                * layer_opacity)
                .round()) as u8;
            rendered |= paint_alpha_mask(
                pixels,
                canvas_width,
                canvas_height,
                &shadow_mask,
                padded_width,
                padded_height,
                left + shadow_x - blur_radius as isize,
                top + shadow_y - blur_radius as isize,
                shadow_color,
            );
        }

        if style.get("strokeEnabled").and_then(Value::as_bool) == Some(true) {
            // CSS text strokes are centered on the glyph outline. The mask
            // is painted before the fill, so only the outside half remains
            // visible; dilating by the full width made native strokes twice
            // as thick as `-webkit-text-stroke` in the renderer.
            let stroke_radius = (style_number(style, "strokeWidth", 2.0) * scale / 2.0)
                .round()
                .clamp(0.0, 64.0) as usize;
            if stroke_radius > 0 {
                let (padded_mask, padded_width, padded_height) =
                    pad_alpha_mask(&mask, box_width, box_height, stroke_radius);
                let stroke_mask =
                    dilate_alpha_mask(&padded_mask, padded_width, padded_height, stroke_radius);
                let mut stroke_color = style_color(style, "strokeColor", [0, 0, 0, 255]);
                stroke_color[3] = (stroke_color[3] as f32 * layer_opacity).round() as u8;
                rendered |= paint_alpha_mask(
                    pixels,
                    canvas_width,
                    canvas_height,
                    &stroke_mask,
                    padded_width,
                    padded_height,
                    left - stroke_radius as isize,
                    top - stroke_radius as isize,
                    stroke_color,
                );
            }
        }

        rendered |= paint_text_fill_mask_with_media(
            pixels,
            canvas_width,
            canvas_height,
            fill.as_deref().unwrap_or(&mask),
            box_width,
            box_height,
            left,
            top,
            bgra,
            style,
            media_fill,
            self.cut_out_backdrop.as_ref(),
            gradient_bounds,
            gradient_object_bounds.map(|(width, height)| TextGradientBounds {
                left: 0.0,
                top: 0.0,
                width: width as f32,
                height: height as f32,
            }),
        );
        for (color, overlay) in &overlays {
            rendered |= paint_alpha_mask(
                pixels,
                canvas_width,
                canvas_height,
                overlay,
                box_width,
                box_height,
                left,
                top,
                [color.b(), color.g(), color.r(), bgra[3]],
            );
        }
        rendered
    }

    pub fn text_fits_box(
        &mut self,
        text: &str,
        box_width: usize,
        box_height: usize,
        font_size: f32,
        style: &Map<String, Value>,
    ) -> bool {
        let scale = style_number(style, "_fontScale", 1.0).max(0.0);
        let margin =
            |field: &str| (style_number(style, field, 0.0).max(0.0) * scale).round() as usize;
        let box_width = box_width
            .saturating_sub(margin("textMarginLeft"))
            .saturating_sub(margin("textMarginRight"));
        let box_height = box_height
            .saturating_sub(margin("textMarginTop"))
            .saturating_sub(margin("textMarginBottom"));
        if text.is_empty() || box_width == 0 || box_height == 0 || font_size <= 0.0 {
            return false;
        }
        let (blocks, _) = self.wrapped_text_blocks(text, box_width, font_size, style);
        text_height(&blocks).ceil() as usize <= box_height
    }

    pub(crate) fn text_line_mask_rects(
        &mut self,
        text: &str,
        box_width: usize,
        box_height: usize,
        font_size: f32,
        style: &Map<String, Value>,
    ) -> Vec<(isize, isize, isize, isize)> {
        if text.is_empty() {
            return Vec::new();
        }
        let (blocks, _) = self.wrapped_text_blocks(text, box_width, font_size, style);
        let free = (box_height as f32 - text_height(&blocks)).max(0.0);
        let vertical = match style.get("textVerticalAlign").and_then(Value::as_str) {
            Some("top") => 0.0,
            Some("bottom") => free,
            _ => free / 2.0,
        };
        let mut lines = Vec::new();
        for block in &blocks {
            for run in block.runs() {
                let mut left = f32::INFINITY;
                let mut right = f32::NEG_INFINITY;
                for glyph in run.glyphs {
                    left = left.min(block.x + glyph.x);
                    right = right.max(block.x + glyph.x + glyph.w);
                }
                if left.is_finite() && right > left {
                    lines.push((
                        left,
                        block.y + run.line_top + vertical,
                        right,
                        run.line_height,
                    ));
                }
            }
        }
        lines.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut merged: Vec<(f32, f32, f32, f32)> = Vec::new();
        for (left, top, right, height) in lines {
            if let Some(line) = merged.last_mut().filter(|line| (line.1 - top).abs() < 0.5) {
                line.0 = line.0.min(left);
                line.2 = line.2.max(right);
                line.3 = line.3.max(height);
            } else {
                merged.push((left, top, right, height));
            }
        }
        let lines = merged;
        let longest = lines
            .iter()
            .max_by(|a, b| (a.2 - a.0).total_cmp(&(b.2 - b.0)))
            .map(|line| (line.0, line.2));
        let scale = style
            .get("_fontScale")
            .and_then(Value::as_f64)
            .unwrap_or(1.0) as f32;
        let offset = |key| style.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32 * scale;
        let (width, height, x, y) = (
            offset("linesOnlyWidthOffset"),
            offset("linesOnlyHeightOffset"),
            offset("linesOnlyHorizontalOffset"),
            offset("linesOnlyVerticalOffset"),
        );
        lines
            .into_iter()
            .map(|(mut l, top, mut r, line_height)| {
                match style.get("linesOnlyStyle").and_then(Value::as_str) {
                    Some("Full Width") | None => {
                        l = 0.0;
                        r = box_width as f32;
                    }
                    Some("Max Line Width") => {
                        if let Some((left, right)) = longest {
                            l = left;
                            r = right;
                        }
                    }
                    _ => {}
                }
                (
                    (l - width / 2.0 + x).floor() as isize,
                    (top - height / 2.0 + y).floor() as isize,
                    (r + width / 2.0 + x).ceil() as isize,
                    (top + line_height + height / 2.0 + y).ceil() as isize,
                )
            })
            .collect()
    }

    /// Paragraph-aware wrapping shared by drawing and `text_fits_box`, so both agree.
    fn wrapped_text_blocks(
        &mut self,
        text: &str,
        box_width: usize,
        font_size: f32,
        style: &Map<String, Value>,
    ) -> (Vec<TextBlock>, Vec<TextRun>) {
        let scale = text_scale(style, font_size);
        let family_name = style
            .get("fontFamily")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Arial");
        let mut attrs = Attrs::new().family(Family::Name(family_name));
        if style.get("bold").and_then(Value::as_bool) != Some(false) {
            attrs = attrs.weight(Weight::BOLD);
        }
        if style.get("italic").and_then(Value::as_bool) == Some(true) {
            attrs = attrs.style(Style::Italic);
        }
        if style.get("capitalization").and_then(Value::as_str) == Some("Small Caps") {
            let mut features = FontFeatures::new();
            features.enable(FeatureTag::SMALL_CAPS);
            attrs = attrs.font_features(features);
        }
        if let Some(level) = style.get("superscript").and_then(Value::as_i64) {
            attrs = with_superscript(attrs, level as i32);
        }
        if let Some(spacing) = style.get("letterSpacing").and_then(json_number) {
            attrs = attrs.letter_spacing(spacing as f32 * scale / font_size.max(1.0));
        }
        let align = match style
            .get("textHorizontalAlign")
            .and_then(Value::as_str)
            .unwrap_or("center")
            .to_ascii_lowercase()
            .as_str()
        {
            "left" => Align::Left,
            "right" => Align::Right,
            "justify" | "justified" => Align::Justified,
            _ => Align::Center,
        };
        let runs = text_runs(style, text, scale);
        let blocks = layout_text(
            &mut self.font_system,
            text,
            &TextLayoutSpec {
                font_size,
                width: Some(box_width as f32),
                attrs: &attrs,
                align,
                paragraph: ParagraphStyle::from_style(style, scale),
                runs: &runs,
            },
        );
        (blocks, runs)
    }

    fn measure_text_extent(
        &mut self,
        text: &str,
        font_size: f32,
        style: &Map<String, Value>,
    ) -> (usize, usize) {
        let scale = text_scale(style, font_size);
        let paragraph = ParagraphStyle::from_style(style, scale);
        let line_height = paragraph.line_height(font_size);
        let family_name = style
            .get("fontFamily")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Arial");
        let mut attrs = Attrs::new().family(Family::Name(family_name));
        if style.get("bold").and_then(Value::as_bool) != Some(false) {
            attrs = attrs.weight(Weight::BOLD);
        }
        if style.get("italic").and_then(Value::as_bool) == Some(true) {
            attrs = attrs.style(Style::Italic);
        }
        if style.get("capitalization").and_then(Value::as_str) == Some("Small Caps") {
            let mut features = FontFeatures::new();
            features.enable(FeatureTag::SMALL_CAPS);
            attrs = attrs.font_features(features);
        }
        if let Some(level) = style.get("superscript").and_then(Value::as_i64) {
            attrs = with_superscript(attrs, level as i32);
        }
        if let Some(spacing) = style.get("letterSpacing").and_then(json_number) {
            attrs = attrs.letter_spacing(spacing as f32 * scale / font_size.max(1.0));
        }
        let align = match style
            .get("textHorizontalAlign")
            .and_then(Value::as_str)
            .unwrap_or("center")
            .to_ascii_lowercase()
            .as_str()
        {
            "left" => Align::Left,
            "right" => Align::Right,
            "justify" | "justified" => Align::Justified,
            _ => Align::Center,
        };
        let runs = text_runs(style, text, scale);
        let blocks = layout_text(
            &mut self.font_system,
            text,
            &TextLayoutSpec {
                font_size,
                width: None,
                attrs: &attrs,
                align,
                paragraph,
                runs: &runs,
            },
        );
        let width = blocks
            .iter()
            .flat_map(|block| {
                block.runs().flat_map(move |run| {
                    run.glyphs
                        .iter()
                        .map(move |glyph| block.x + glyph.x + glyph.w)
                })
            })
            .fold(0.0_f32, f32::max);
        (
            width.ceil().max(1.0) as usize,
            text_height(&blocks).ceil().max(line_height).max(1.0) as usize,
        )
    }
}

/// Opacity is applied after layout, so child reveals keep every paragraph's position fixed.
fn text_reveal_paragraph_opacities(
    text: &str,
    style: &Map<String, Value>,
    child_opacities: Option<&HashMap<usize, f32>>,
) -> Option<Vec<f32>> {
    if style.get("_textRevealMode").and_then(Value::as_str) != Some("paragraph") {
        return None;
    }
    let mut opacities = Vec::new();
    let mut nonempty_ordinal = 0;
    let mut ended_with_break = text.is_empty();
    if let Some(children) = child_opacities {
        let isolated_child = style
            .get("_textRevealIsolatedChild")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok());
        let initial_visible = style
            .get("_textRevealChildInitialVisible")
            .and_then(Value::as_u64)? as usize;
        for (range, ending) in LineIter::new(text) {
            let paragraph = &text[range];
            let opacity = if paragraph.is_empty() && isolated_child.is_some() {
                0.0
            } else if paragraph.is_empty() {
                1.0
            } else {
                let ordinal = nonempty_ordinal;
                nonempty_ordinal += 1;
                if let Some(isolated) = isolated_child {
                    if ordinal > 0 && ordinal - 1 == isolated {
                        1.0
                    } else {
                        0.0
                    }
                } else if ordinal == 0 {
                    1.0
                } else {
                    children.get(&(ordinal - 1)).copied().unwrap_or_else(|| {
                        if ordinal < initial_visible {
                            1.0
                        } else {
                            0.0
                        }
                    })
                }
            };
            opacities.push(opacity);
            ended_with_break = ending != LineEnding::None;
        }
        if ended_with_break {
            opacities.push(if isolated_child.is_some() { 0.0 } else { 1.0 });
        }
        return Some(opacities);
    }
    let visible_count = style
        .get("_textRevealVisibleParagraphs")
        .and_then(Value::as_u64)? as usize;
    for (range, ending) in LineIter::new(text) {
        let paragraph = &text[range];
        opacities.push(
            if paragraph.is_empty() || nonempty_ordinal < visible_count {
                1.0
            } else {
                0.0
            },
        );
        if !paragraph.is_empty() {
            nonempty_ordinal += 1;
        }
        ended_with_break = ending != LineEnding::None;
    }
    if ended_with_break {
        opacities.push(1.0);
    }
    opacities
        .iter()
        .any(|opacity| *opacity == 0.0)
        .then_some(opacities)
}

fn text_reveal_child_opacities(style: &Map<String, Value>) -> Option<HashMap<usize, f32>> {
    style
        .get("_textRevealChildOpacities")
        .and_then(Value::as_object)?
        .iter()
        .map(|(index, opacity)| {
            let index = index.parse().ok()?;
            let opacity = opacity
                .as_f64()
                .filter(|opacity| opacity.is_finite() && (0.0..=1.0).contains(opacity))?
                as f32;
            Some((index, opacity))
        })
        .collect()
}

fn paragraph_opacity(opacities: Option<&[f32]>, index: usize) -> f32 {
    opacities
        .and_then(|paragraphs| paragraphs.get(index))
        .copied()
        .unwrap_or(1.0)
}

fn text_reveal_visible_underlines(style: &Map<String, Value>) -> Option<usize> {
    (style.get("_textRevealMode").and_then(Value::as_str) == Some("underline")
        && style.get("_textRevealChildOpacities").is_none())
    .then(|| {
        style
            .get("_textRevealVisibleUnderlines")
            .and_then(Value::as_u64)
    })
    .flatten()
    .map(|count| count as usize)
}

fn text_reveal_underline_opacity(
    metadata: usize,
    base_underline: bool,
    child_opacities: &HashMap<usize, f32>,
    initial_visible: usize,
    isolated_child: Option<usize>,
) -> f32 {
    let reveal_index = underline_reveal_index(metadata).or_else(|| base_underline.then_some(0));
    match (isolated_child, reveal_index) {
        (Some(isolated), Some(index)) => {
            if index == isolated {
                1.0
            } else {
                0.0
            }
        }
        (Some(_), None) => 0.0,
        (None, Some(index)) => child_opacities.get(&index).copied().unwrap_or_else(|| {
            if index < initial_visible {
                1.0
            } else {
                0.0
            }
        }),
        (None, None) => 1.0,
    }
}

/// Output pixels per slide pixel: the factor that took `fontSize` to the drawn font size.
fn text_scale(style: &Map<String, Value>, font_size: f32) -> f32 {
    style_number(
        style,
        "_fontScale",
        font_size
            / style
                .get("fontSize")
                .and_then(json_number)
                .unwrap_or(font_size as f64)
                .max(1.0) as f32,
    )
    .max(0.01)
}

/// Reads the `_textRuns` formatting of the text drawn in a box. Runs only apply while they still
/// spell the drawn text: linked text, capitalization and line transforms change it, and the box
/// style then applies throughout. A run `fontSize` is in slide pixels and scales by `scale`, like
/// the layer font size.
fn text_runs(style: &Map<String, Value>, text: &str, scale: f32) -> Vec<TextRun> {
    let Some(source) = style.get("_textRuns").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut runs = Vec::new();
    let mut end = 0;
    let mut next_underline_reveal_index = 0;
    let mut previous_underlined_end = None;
    let mut previous_underlined_index = None;
    for (source_index, run) in source.iter().enumerate() {
        let Some(part) = run.get("text").and_then(Value::as_str) else {
            return Vec::new();
        };
        let range = end..end + part.len();
        if text.get(range.clone()) != Some(part) {
            return Vec::new();
        }
        end = range.end;
        let format = run.get("style");
        let flag = |key: &str| {
            format
                .and_then(|format| format.get(key))
                .and_then(Value::as_bool)
        };
        let underline = flag("underline") == Some(true);
        let underline_reveal_index = if underline && !part.is_empty() {
            let index = if previous_underlined_end == Some(range.start) {
                previous_underlined_index.unwrap()
            } else {
                let index = next_underline_reveal_index;
                next_underline_reveal_index += 1;
                index
            };
            previous_underlined_end = Some(range.end);
            previous_underlined_index = Some(index);
            Some(index)
        } else {
            if !part.is_empty() {
                previous_underlined_end = None;
                previous_underlined_index = None;
            }
            None
        };
        runs.push(TextRun {
            range,
            bold: flag("bold"),
            italic: flag("italic"),
            underline,
            underline_reveal_index,
            source_index,
            strikethrough: flag("strikethrough") == Some(true),
            color: format
                .and_then(|format| format.get("textColor"))
                .and_then(Value::as_str)
                .and_then(parse_bgra_color)
                .map(|[blue, green, red, _]| Color::rgb(red, green, blue)),
            family: format
                .and_then(|format| format.get("fontFamily"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            font_size: format
                .and_then(|format| format.get("fontSize"))
                .and_then(json_number)
                .map(|size| size as f32 * scale),
            letter_spacing: format
                .and_then(|format| format.get("letterSpacing"))
                .and_then(json_number)
                .map(|spacing| spacing as f32 * scale),
            superscript: format
                .and_then(|format| format.get("superscript"))
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok()),
        });
    }
    if end != text.len() || runs.iter().all(TextRun::is_plain) {
        return Vec::new();
    }
    runs
}

/// Glyph coverage of a text box: `all` drives the shadow and stroke, `fill` the text fill when
/// runs have their own color, and each run color paints through its own overlay.
struct TextMasks {
    all: Vec<u8>,
    fill: Option<Vec<u8>>,
    overlays: Vec<(Color, Vec<u8>)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TextGradientBounds {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

fn text_gradient_layout_bounds(
    blocks: &[TextBlock],
    vertical_offset: isize,
    box_width: usize,
    box_height: usize,
) -> Option<TextGradientBounds> {
    let mut bounds = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for block in blocks {
        for run in block.runs() {
            let mut line_left = f32::INFINITY;
            let mut line_right = f32::NEG_INFINITY;
            for glyph in run.glyphs {
                let left = block.x + glyph.x;
                let right = left + glyph.w;
                line_left = line_left.min(left.min(right));
                line_right = line_right.max(left.max(right));
            }
            if line_left.is_finite() && line_right > line_left && run.line_height > 0.0 {
                bounds.0 = bounds.0.min(line_left.max(0.0));
                bounds.1 = bounds
                    .1
                    .min((block.y + run.line_top + vertical_offset as f32).max(0.0));
                bounds.2 = bounds.2.max(line_right.min(box_width as f32));
                bounds.3 = bounds.3.max(
                    (block.y + run.line_top + run.line_height + vertical_offset as f32)
                        .min(box_height as f32),
                );
            }
        }
    }
    (bounds.2 > bounds.0 && bounds.3 > bounds.1).then_some(TextGradientBounds {
        left: bounds.0,
        top: bounds.1,
        width: bounds.2 - bounds.0,
        height: bounds.3 - bounds.1,
    })
}

impl TextMasks {
    fn new(len: usize, colored: bool) -> Self {
        Self {
            all: vec![0; len],
            fill: colored.then(|| vec![0; len]),
            overlays: Vec::new(),
        }
    }

    /// Marks `alpha` coverage at `index`. `color` is the run color of the glyph, if it has one;
    /// `base_alpha` is the text box color alpha that run colors share.
    fn put(&mut self, index: usize, alpha: u8, color: Option<Color>, base_alpha: u8) {
        self.all[index] = self.all[index].max(alpha);
        let Some(fill) = self.fill.as_mut() else {
            return;
        };
        let Some(color) = color else {
            fill[index] = fill[index].max(alpha);
            return;
        };
        let position = self
            .overlays
            .iter()
            .position(|(overlay, _)| *overlay == color)
            .unwrap_or_else(|| {
                self.overlays.push((color, vec![0; fill.len()]));
                self.overlays.len() - 1
            });
        let overlay = &mut self.overlays[position].1;
        overlay[index] = overlay[index].max((alpha as u16 * base_alpha as u16 / 255) as u8);
    }
}

fn append_text_decorations(
    masks: &mut TextMasks,
    width: usize,
    height: usize,
    blocks: &[TextBlock],
    vertical_offset: isize,
    style: &Map<String, Value>,
    base_alpha: u8,
    paragraph_opacities: Option<&[f32]>,
    visible_underlines: Option<usize>,
    base_underline: bool,
    child_opacities: Option<&HashMap<usize, f32>>,
    initial_visible_underlines: usize,
    isolated_child: Option<usize>,
) {
    let mut layer_flags = 0;
    if style.get("underline").and_then(Value::as_bool) == Some(true) {
        layer_flags |= RUN_UNDERLINE;
    }
    if style.get("strikethrough").and_then(Value::as_bool) == Some(true) {
        layer_flags |= RUN_STRIKETHROUGH;
    }
    for (block, run) in blocks
        .iter()
        .filter(|block| paragraph_opacity(paragraph_opacities, block.paragraph_index) > 0.0)
        .flat_map(|block| block.runs().map(move |run| (block, run)))
    {
        let paragraph_opacity = paragraph_opacity(paragraph_opacities, block.paragraph_index);
        let thickness = (run.line_height * 0.055).round().max(1.0) as usize;
        let baseline = vertical_offset as f32 + block.y + run.line_y;
        // Consecutive glyphs with the same decorations and color share one line.
        for group in run.glyphs.chunk_by(|a, b| {
            a.metadata == b.metadata
                && a.color_opt == b.color_opt
                && run
                    .text
                    .get(a.start..a.end)
                    .is_some_and(|text| text.chars().all(char::is_whitespace))
                    == run
                        .text
                        .get(b.start..b.end)
                        .is_some_and(|text| text.chars().all(char::is_whitespace))
        }) {
            let opacity = paragraph_opacity
                * child_opacities.map_or(1.0, |opacities| {
                    text_reveal_underline_opacity(
                        group[0].metadata,
                        base_underline,
                        opacities,
                        initial_visible_underlines,
                        isolated_child,
                    )
                });
            if opacity <= 0.0 {
                continue;
            }
            if visible_underlines.is_some_and(|count| {
                !underline_glyph_visible(group[0].metadata, count, base_underline)
            }) {
                continue;
            }
            let flags = layer_flags | group[0].metadata;
            if flags == 0 {
                continue;
            }
            let first = &group[0];
            let last = &group[group.len() - 1];
            let start = (block.x + first.x).floor().max(0.0) as usize;
            let end = (block.x + last.x + last.w).ceil().min(width as f32) as usize;
            let run_style = decoration_source_index(first.metadata)
                .and_then(|index| {
                    style
                        .get("_textRuns")
                        .and_then(Value::as_array)
                        .and_then(|runs| runs.get(index))
                })
                .and_then(|run| run.get("style"));
            let value = |key: &str| {
                run_style
                    .and_then(|run| run.get(key))
                    .or_else(|| style.get(key))
            };
            let whitespace = group.iter().all(|glyph| {
                run.text
                    .get(glyph.start..glyph.end)
                    .is_some_and(|text| text.chars().all(char::is_whitespace))
            });
            for (flag, key, position) in [
                (
                    RUN_UNDERLINE,
                    "underline",
                    baseline + run.line_height * 0.08,
                ),
                (
                    RUN_STRIKETHROUGH,
                    "strikethrough",
                    baseline - run.line_height * 0.3,
                ),
            ] {
                if flags & flag == 0
                    || (whitespace
                        && value(&format!("{key}ByWord")).and_then(Value::as_bool) == Some(true))
                {
                    continue;
                }
                let variant = value(&format!("{key}Style"))
                    .and_then(Value::as_str)
                    .unwrap_or("single");
                let pattern = value(&format!("{key}Pattern"))
                    .and_then(Value::as_str)
                    .unwrap_or("solid");
                let color = value(&format!("{key}Color"))
                    .and_then(Value::as_str)
                    .and_then(parse_bgra_color)
                    .map(|[b, g, r, _]| Color::rgb(r, g, b))
                    .or(first.color_opt);
                let line_thickness = if variant == "thick" {
                    thickness * 2
                } else {
                    thickness
                };
                let row = position.round() as isize;
                let offsets: &[isize] = if variant == "double" {
                    &[0, (thickness * 2) as isize]
                } else {
                    &[0]
                };
                for offset in offsets {
                    for y in row + offset..row + offset + line_thickness as isize {
                        if y < 0 || y >= height as isize {
                            continue;
                        }
                        for x in start..end {
                            let unit = thickness.max(1);
                            let phase = (x - start) / unit;
                            let painted = match pattern {
                                "dot" => phase % 3 == 0,
                                "dash" => phase % 7 < 4,
                                "dash-dot" => matches!(phase % 7, 0..=3 | 5),
                                "dash-dot-dot" => matches!(phase % 9, 0..=3 | 5 | 7),
                                _ => true,
                            };
                            if painted {
                                masks.put(
                                    y as usize * width + x,
                                    (255.0 * opacity).round() as u8,
                                    color,
                                    base_alpha,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

fn dilate_alpha_mask(mask: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let mut output = vec![0_u8; mask.len()];
    let radius = radius as isize;
    for y in 0..height as isize {
        for x in 0..width as isize {
            let alpha = mask[y as usize * width + x as usize];
            if alpha == 0 {
                continue;
            }
            for offset_y in -radius..=radius {
                for offset_x in -radius..=radius {
                    if offset_x * offset_x + offset_y * offset_y > radius * radius {
                        continue;
                    }
                    let target_x = x + offset_x;
                    let target_y = y + offset_y;
                    if target_x < 0
                        || target_y < 0
                        || target_x >= width as isize
                        || target_y >= height as isize
                    {
                        continue;
                    }
                    let index = target_y as usize * width + target_x as usize;
                    output[index] = output[index].max(alpha);
                }
            }
        }
    }
    output
}

fn pad_alpha_mask(
    mask: &[u8],
    width: usize,
    height: usize,
    padding: usize,
) -> (Vec<u8>, usize, usize) {
    if padding == 0 {
        return (mask.to_vec(), width, height);
    }
    let padded_width = width.saturating_add(padding.saturating_mul(2));
    let padded_height = height.saturating_add(padding.saturating_mul(2));
    let mut output = vec![0_u8; padded_width.saturating_mul(padded_height)];
    for y in 0..height {
        let source = &mask[y * width..(y + 1) * width];
        let start = (y + padding) * padded_width + padding;
        output[start..start + width].copy_from_slice(source);
    }
    (output, padded_width, padded_height)
}

fn blur_alpha_mask(mask: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 {
        return mask.to_vec();
    }
    let mut horizontal = vec![0_u8; mask.len()];
    let mut output = vec![0_u8; mask.len()];
    let mut prefix = vec![0_usize; width.saturating_add(1)];
    for y in 0..height {
        prefix[0] = 0;
        for x in 0..width {
            prefix[x + 1] = prefix[x] + mask[y * width + x] as usize;
        }
        for x in 0..width {
            let start = x.saturating_sub(radius);
            let end = (x + radius + 1).min(width);
            let sum = prefix[end] - prefix[start];
            horizontal[y * width + x] = (sum / (end - start).max(1)) as u8;
        }
    }
    prefix.resize(height.saturating_add(1), 0);
    for x in 0..width {
        prefix[0] = 0;
        for y in 0..height {
            prefix[y + 1] = prefix[y] + horizontal[y * width + x] as usize;
        }
        for y in 0..height {
            let start = y.saturating_sub(radius);
            let end = (y + radius + 1).min(height);
            let sum = prefix[end] - prefix[start];
            output[y * width + x] = (sum / (end - start).max(1)) as u8;
        }
    }
    output
}

#[allow(clippy::too_many_arguments)]
fn paint_alpha_mask(
    pixels: &mut [u8],
    canvas_width: usize,
    canvas_height: usize,
    mask: &[u8],
    mask_width: usize,
    mask_height: usize,
    left: isize,
    top: isize,
    color: [u8; 4],
) -> bool {
    let mut rendered = false;
    for y in 0..mask_height {
        for x in 0..mask_width {
            let coverage = mask[y * mask_width + x];
            if coverage == 0 {
                continue;
            }
            let target_x = left + x as isize;
            let target_y = top + y as isize;
            if target_x < 0
                || target_y < 0
                || target_x >= canvas_width as isize
                || target_y >= canvas_height as isize
            {
                continue;
            }
            let mut covered_color = color;
            covered_color[3] = ((covered_color[3] as u16 * coverage as u16) / 255) as u8;
            blend_bgra_pixel(
                pixels,
                canvas_width,
                target_x as usize,
                target_y as usize,
                covered_color,
            );
            rendered = true;
        }
    }
    rendered
}

#[allow(clippy::too_many_arguments)]
fn paint_text_fill_mask_with_media(
    pixels: &mut [u8],
    canvas_width: usize,
    canvas_height: usize,
    mask: &[u8],
    mask_width: usize,
    mask_height: usize,
    left: isize,
    top: isize,
    color: [u8; 4],
    style: &Map<String, Value>,
    media_fill: Option<&DynamicImage>,
    backdrop: Option<&CutOutBackdrop>,
    gradient_bounds: Option<TextGradientBounds>,
    gradient_object_bounds: Option<TextGradientBounds>,
) -> bool {
    let mode = style
        .get("textFillMode")
        .and_then(Value::as_str)
        .unwrap_or("Text Color");
    if mode.eq_ignore_ascii_case("Media Fill") {
        if let Some(image) = media_fill {
            return paint_media_fill_mask(
                pixels,
                canvas_width,
                canvas_height,
                mask,
                mask_width,
                mask_height,
                left,
                top,
                style,
                image,
            );
        }
        return paint_alpha_mask(
            pixels,
            canvas_width,
            canvas_height,
            mask,
            mask_width,
            mask_height,
            left,
            top,
            color,
        );
    }
    if mode.eq_ignore_ascii_case("Text Cut Out") {
        // Without a box beneath, cut-out text cuts through nothing and stays invisible.
        return backdrop.is_some_and(|backdrop| {
            paint_cut_out_mask(
                pixels,
                canvas_width,
                mask,
                mask_width,
                mask_height,
                left,
                top,
                backdrop,
            )
        });
    }
    if mode.eq_ignore_ascii_case("Background Invert") {
        return paint_difference_mask(
            pixels,
            canvas_width,
            canvas_height,
            mask,
            mask_width,
            mask_height,
            left,
            top,
            style,
        );
    }
    if mode.eq_ignore_ascii_case("Background Blur") {
        return paint_blurred_text_mask(
            pixels,
            canvas_width,
            canvas_height,
            mask,
            mask_width,
            mask_height,
            left,
            top,
            style,
        );
    }
    if !mode.eq_ignore_ascii_case("Text Gradient") {
        return paint_alpha_mask(
            pixels,
            canvas_width,
            canvas_height,
            mask,
            mask_width,
            mask_height,
            left,
            top,
            color,
        );
    }

    let start = style_color(
        style,
        "textGradientStart",
        style_color(style, "textColor", [246, 99, 11, 255]),
    );
    let end = style_color(style, "textGradientEnd", [232, 200, 20, 255]);
    let angle = style_number(style, "textGradientAngle", 0.0).to_radians();
    let bounds = if style
        .get("textGradientUseObjectBounds")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        gradient_object_bounds.unwrap_or(TextGradientBounds {
            left: 0.0,
            top: 0.0,
            width: mask_width as f32,
            height: mask_height as f32,
        })
    } else if let Some(bounds) = gradient_bounds {
        bounds
    } else {
        return false;
    };
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        return false;
    }
    let dx = angle.sin();
    let dy = -angle.cos();
    let layer_opacity = style_number(style, "_layerOpacity", 1.0).clamp(0.0, 1.0);
    let mut rendered = false;
    for y in 0..mask_height {
        for x in 0..mask_width {
            let coverage = mask[y * mask_width + x];
            if coverage == 0 {
                continue;
            }
            let target_x = left + x as isize;
            let target_y = top + y as isize;
            if target_x < 0
                || target_y < 0
                || target_x >= canvas_width as isize
                || target_y >= canvas_height as isize
            {
                continue;
            }
            let local_x = (x as f32 + 0.5 - bounds.left) / bounds.width;
            let local_y = (y as f32 + 0.5 - bounds.top) / bounds.height;
            let coordinate = (local_x - 0.5) * dx + (local_y - 0.5) * dy;
            let amount = (coordinate + 0.5).clamp(0.0, 1.0);
            let mut covered_color = blend_bgra_color(start, end, amount);
            covered_color[3] = (covered_color[3] as f32 * layer_opacity).round() as u8;
            covered_color[3] = ((covered_color[3] as u16 * coverage as u16) / 255) as u8;
            blend_bgra_pixel(
                pixels,
                canvas_width,
                target_x as usize,
                target_y as usize,
                covered_color,
            );
            rendered = true;
        }
    }
    rendered
}

fn paint_media_fill_mask(
    pixels: &mut [u8],
    canvas_width: usize,
    canvas_height: usize,
    mask: &[u8],
    mask_width: usize,
    mask_height: usize,
    left: isize,
    top: isize,
    style: &Map<String, Value>,
    image: &DynamicImage,
) -> bool {
    let source = image.to_rgba8();
    if source.width() == 0 || source.height() == 0 || mask_width == 0 || mask_height == 0 {
        return false;
    }

    let target_width = mask_width as f64;
    let target_height = mask_height as f64;
    let source_width = source.width() as f64;
    let source_height = source.height() as f64;
    let scale = style
        .get("mediaFillScale")
        .and_then(Value::as_str)
        .unwrap_or("Scale to Fill")
        .to_ascii_lowercase();
    let fit = if scale.contains("stretch") {
        "stretch"
    } else if scale.contains("fit") {
        "fit"
    } else {
        "fill"
    };
    let factor = match fit {
        "fit" => (target_width / source_width).min(target_height / source_height),
        "fill" => (target_width / source_width).max(target_height / source_height),
        _ => 1.0,
    };
    let rendered_width = if fit == "stretch" {
        mask_width
    } else {
        (source_width * factor).round().max(1.0) as usize
    };
    let rendered_height = if fit == "stretch" {
        mask_height
    } else {
        (source_height * factor).round().max(1.0) as usize
    };
    let rendered = image::imageops::resize(
        &source,
        rendered_width as u32,
        rendered_height as u32,
        image::imageops::FilterType::Lanczos3,
    );
    let alignment = style
        .get("mediaFillAlignment")
        .and_then(Value::as_str)
        .unwrap_or("Middle Center");
    let alignment_x = text_media_alignment_factor(alignment, false);
    let alignment_y = text_media_alignment_factor(alignment, true);
    let rendered_left =
        ((mask_width as isize - rendered.width() as isize) as f64 * alignment_x).round() as isize;
    let rendered_top =
        ((mask_height as isize - rendered.height() as isize) as f64 * alignment_y).round() as isize;
    let layer_opacity = style_number(style, "_layerOpacity", 1.0).clamp(0.0, 1.0);
    let mut painted = false;

    for y in 0..mask_height {
        for x in 0..mask_width {
            let coverage = mask[y * mask_width + x];
            if coverage == 0 {
                continue;
            }
            let source_x = x as isize - rendered_left;
            let source_y = y as isize - rendered_top;
            if source_x < 0
                || source_y < 0
                || source_x >= rendered.width() as isize
                || source_y >= rendered.height() as isize
            {
                continue;
            }
            let [red, green, blue, alpha] = rendered.get_pixel(source_x as u32, source_y as u32).0;
            let target_x = left + x as isize;
            let target_y = top + y as isize;
            if target_x < 0
                || target_y < 0
                || target_x >= canvas_width as isize
                || target_y >= canvas_height as isize
            {
                continue;
            }
            let mut source_color = [blue, green, red, alpha];
            source_color[3] = (source_color[3] as f32 * layer_opacity).round() as u8;
            source_color[3] = ((source_color[3] as u16 * coverage as u16) / 255) as u8;
            blend_bgra_pixel(
                pixels,
                canvas_width,
                target_x as usize,
                target_y as usize,
                source_color,
            );
            painted = true;
        }
    }
    painted
}

fn text_media_alignment_factor(alignment: &str, vertical: bool) -> f64 {
    let alignment = alignment.to_ascii_lowercase();
    if vertical {
        if alignment.contains("top") {
            0.0
        } else if alignment.contains("bottom") {
            1.0
        } else {
            0.5
        }
    } else if alignment.contains("left") {
        0.0
    } else if alignment.contains("right") {
        1.0
    } else {
        0.5
    }
}

fn paint_cut_out_mask(
    pixels: &mut [u8],
    canvas_width: usize,
    mask: &[u8],
    mask_width: usize,
    mask_height: usize,
    left: isize,
    top: isize,
    backdrop: &CutOutBackdrop,
) -> bool {
    let mut painted = false;
    for y in 0..mask_height {
        for x in 0..mask_width {
            let coverage = mask[y * mask_width + x];
            if coverage == 0 {
                continue;
            }
            let (target_x, target_y) = (left + x as isize, top + y as isize);
            if let Some(beneath) = backdrop.pixel(target_x, target_y) {
                reveal_backdrop_pixel(
                    pixels,
                    canvas_width,
                    target_x as usize,
                    target_y as usize,
                    beneath,
                    coverage as f32 / 255.0,
                );
                painted = true;
            }
        }
    }
    painted
}

fn paint_difference_mask(
    pixels: &mut [u8],
    canvas_width: usize,
    canvas_height: usize,
    mask: &[u8],
    mask_width: usize,
    mask_height: usize,
    left: isize,
    top: isize,
    style: &Map<String, Value>,
) -> bool {
    let layer_opacity = style_number(style, "_layerOpacity", 1.0).clamp(0.0, 1.0);
    let mut painted = false;
    for y in 0..mask_height {
        for x in 0..mask_width {
            let coverage = mask[y * mask_width + x];
            if coverage == 0 {
                continue;
            }
            let target_x = left + x as isize;
            let target_y = top + y as isize;
            if target_x < 0
                || target_y < 0
                || target_x >= canvas_width as isize
                || target_y >= canvas_height as isize
            {
                continue;
            }
            let offset = (target_y as usize * canvas_width + target_x as usize) * 4;
            let amount = coverage as f32 / 255.0 * layer_opacity;
            for channel in 0..3 {
                let backdrop = pixels[offset + channel] as f32;
                let difference = (backdrop - 255.0).abs();
                pixels[offset + channel] =
                    (difference * amount + backdrop * (1.0 - amount)).round() as u8;
            }
            pixels[offset + 3] = 255;
            painted = true;
        }
    }
    painted
}

fn paint_blurred_text_mask(
    pixels: &mut [u8],
    canvas_width: usize,
    canvas_height: usize,
    mask: &[u8],
    mask_width: usize,
    mask_height: usize,
    left: isize,
    top: isize,
    style: &Map<String, Value>,
) -> bool {
    let radius = (style_number(style, "backgroundBlurAmount", 0.0).max(0.0)
        * style_number(style, "_fontScale", 1.0).max(0.01))
    .round()
    .min(256.0) as usize;
    let saturation = style_number(style, "backgroundBlurSaturation", 100.0).max(0.0) / 100.0;
    let opacity = style_number(style, "_layerOpacity", 1.0).clamp(0.0, 1.0);
    let coverage = mask
        .iter()
        .map(|value| (*value as f32 * opacity).round() as u8)
        .collect::<Vec<_>>();
    paint_backdrop_effect(
        pixels,
        canvas_width,
        canvas_height,
        &coverage,
        mask_width,
        mask_height,
        left,
        top,
        Some((radius, saturation)),
    )
}

/// Mixes the blurred, saturation-adjusted backdrop (or its inverse without blur) into the
/// covered pixels, like ProPresenter's background effects.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_backdrop_effect(
    pixels: &mut [u8],
    canvas_width: usize,
    canvas_height: usize,
    coverage: &[u8],
    width: usize,
    height: usize,
    left: isize,
    top: isize,
    blur: Option<(usize, f32)>,
) -> bool {
    let radius = blur.map_or(0, |(radius, _)| radius) as isize;
    let region_left = (left - radius).clamp(0, canvas_width as isize);
    let region_top = (top - radius).clamp(0, canvas_height as isize);
    let region_width =
        ((left + width as isize + radius).clamp(0, canvas_width as isize) - region_left) as usize;
    let region_height =
        ((top + height as isize + radius).clamp(0, canvas_height as isize) - region_top) as usize;
    let pixel_offset = |x: usize, y: usize| {
        ((region_top as usize + y) * canvas_width + region_left as usize + x) * 4
    };
    let blurred = blur.map(|(radius, _)| {
        (0..3)
            .map(|channel| {
                let plane = (0..region_width * region_height)
                    .map(|index| {
                        pixels[pixel_offset(index % region_width, index / region_width) + channel]
                    })
                    .collect::<Vec<_>>();
                let once = blur_alpha_mask(&plane, region_width, region_height, radius.div_ceil(2));
                blur_alpha_mask(&once, region_width, region_height, radius.div_ceil(2))
            })
            .collect::<Vec<_>>()
    });
    let mut painted = false;
    for y in 0..region_height {
        for x in 0..region_width {
            let (mask_x, mask_y) = (
                region_left + x as isize - left,
                region_top + y as isize - top,
            );
            if mask_x < 0 || mask_y < 0 || mask_x >= width as isize || mask_y >= height as isize {
                continue;
            }
            let amount = coverage[mask_y as usize * width + mask_x as usize] as f32 / 255.0;
            if amount <= 0.0 {
                continue;
            }
            let offset = pixel_offset(x, y);
            let effected = match (&blurred, blur) {
                (Some(planes), Some((_, saturation))) => {
                    let index = y * region_width + x;
                    let [b, g, r] = [0, 1, 2].map(|channel| planes[channel][index] as f32);
                    let luma = 0.0722 * b + 0.7152 * g + 0.2126 * r;
                    [b, g, r].map(|value| luma + (value - luma) * saturation)
                }
                _ => [0, 1, 2].map(|channel| 255.0 - pixels[offset + channel] as f32),
            };
            for channel in 0..3 {
                let backdrop = pixels[offset + channel] as f32;
                pixels[offset + channel] = (backdrop
                    + (effected[channel].clamp(0.0, 255.0) - backdrop) * amount)
                    .round() as u8;
            }
            painted = true;
        }
    }
    painted
}

fn blend_bgra_color(start: [u8; 4], end: [u8; 4], amount: f32) -> [u8; 4] {
    let alpha = start[3] as f32 * (1.0 - amount) + end[3] as f32 * amount;
    if alpha == 0.0 {
        return [0; 4];
    }
    let mut color = [0; 4];
    for channel in 0..3 {
        color[channel] = ((start[channel] as f32 * start[3] as f32 * (1.0 - amount)
            + end[channel] as f32 * end[3] as f32 * amount)
            / alpha)
            .round() as u8;
    }
    color[3] = alpha.round() as u8;
    color
}

fn style_number(style: &Map<String, Value>, key: &str, fallback: f32) -> f32 {
    style
        .get(key)
        .and_then(|value| match value {
            Value::Number(value) => value.as_f64(),
            Value::String(value) => value.trim().parse::<f64>().ok(),
            _ => None,
        })
        .map(|value| value as f32)
        .filter(|value| value.is_finite())
        .unwrap_or(fallback)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScrollDirection {
    Left,
    Right,
    Up,
    Down,
}

const MAX_SCROLL_CONTENT_DIMENSION: usize = 16_384;
const MAX_SCROLL_BUFFER_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn scrolling_duration_seconds(style: &Map<String, Value>) -> f64 {
    let raw_speed = style_number(style, "scrollSpeed", 50.0) as f64;
    if raw_speed == 0.0 {
        return f64::INFINITY;
    }
    let speed = raw_speed.clamp(1.0, 100.0);
    (38.0 - speed * 0.32).round().max(3.0)
}

pub(crate) fn scrolling_is_active(style: &Map<String, Value>, elapsed_seconds: f64) -> bool {
    if style_number(style, "scrollSpeed", 50.0) == 0.0 {
        return false;
    }
    if style.get("scrollingEnabled").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    if style.get("scrollRepeat").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    elapsed_seconds.is_finite() && elapsed_seconds.max(0.0) < scrolling_duration_seconds(style)
}

fn scroll_direction(style: &Map<String, Value>) -> ScrollDirection {
    match style
        .get("scrollDirection")
        .and_then(Value::as_str)
        .unwrap_or("left")
        .to_ascii_lowercase()
        .as_str()
    {
        "right" => ScrollDirection::Right,
        "up" => ScrollDirection::Up,
        "down" => ScrollDirection::Down,
        _ => ScrollDirection::Left,
    }
}

fn bounded_scroll_dimensions(width: usize, height: usize) -> Option<(usize, usize)> {
    let mut width = width.min(MAX_SCROLL_CONTENT_DIMENSION).max(1);
    let mut height = height.min(MAX_SCROLL_CONTENT_DIMENSION).max(1);
    let max_pixels = MAX_SCROLL_BUFFER_BYTES / 4;
    if width > max_pixels {
        width = max_pixels;
    }
    if width.checked_mul(height)? > max_pixels {
        height = (max_pixels / width).max(1);
    }
    if width.checked_mul(height)? > max_pixels {
        width = (max_pixels / height).max(1);
    }
    Some((width, height))
}

#[allow(clippy::too_many_arguments)]
fn blit_scrolling_copy(
    pixels: &mut [u8],
    canvas_width: usize,
    canvas_height: usize,
    source: &[u8],
    source_width: usize,
    source_height: usize,
    left: isize,
    top: isize,
    offset_x: isize,
    offset_y: isize,
    viewport_width: usize,
    viewport_height: usize,
    feather_left: f64,
    feather_right: f64,
    cut_out: Option<&CutOutBackdrop>,
) -> bool {
    let mut painted = false;
    for source_y in 0..source_height {
        let local_y = offset_y + source_y as isize;
        if local_y < 0 || local_y >= viewport_height as isize {
            continue;
        }
        let target_y = top + local_y;
        if target_y < 0 || target_y >= canvas_height as isize {
            continue;
        }
        for source_x in 0..source_width {
            let local_x = offset_x + source_x as isize;
            if local_x < 0 || local_x >= viewport_width as isize {
                continue;
            }
            let target_x = left + local_x;
            if target_x < 0 || target_x >= canvas_width as isize {
                continue;
            }
            let source_pixel = &source[(source_y * source_width + source_x) * 4..][..4];
            if source_pixel[3] == 0 {
                continue;
            }
            let local_x = local_x as f64 + 0.5;
            let mut feather: f64 = 1.0;
            if feather_left > 0.0 && local_x < feather_left {
                feather = feather.min(local_x / feather_left);
            }
            if feather_right > 0.0 && local_x > viewport_width as f64 - feather_right {
                feather = feather
                    .min(((viewport_width as f64 - local_x) / feather_right).clamp(0.0, 1.0));
            }
            if let Some(backdrop) = cut_out {
                if let Some(beneath) = backdrop.pixel(target_x, target_y) {
                    reveal_backdrop_pixel(
                        pixels,
                        canvas_width,
                        target_x as usize,
                        target_y as usize,
                        beneath,
                        source_pixel[3] as f32 / 255.0 * feather as f32,
                    );
                }
                continue;
            }
            blend_bgra_pixel_with_factor(
                pixels,
                canvas_width,
                target_x as usize,
                target_y as usize,
                [
                    source_pixel[0],
                    source_pixel[1],
                    source_pixel[2],
                    source_pixel[3],
                ],
                feather as f32,
            );
            painted = true;
        }
    }
    painted
}

fn blend_bgra_pixel_with_factor(
    pixels: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    source: [u8; 4],
    factor: f32,
) {
    let offset = (y * width + x) * 4;
    let source_alpha = (source[3] as f32 / 255.0 * factor).clamp(0.0, 1.0);
    if source_alpha <= 0.0 {
        return;
    }
    let destination_alpha = pixels[offset + 3] as f32 / 255.0;
    let result_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    for channel in 0..3 {
        let value = source[channel] as f32 * source_alpha
            + pixels[offset + channel] as f32 * destination_alpha * (1.0 - source_alpha);
        pixels[offset + channel] = if result_alpha > 0.0 {
            (value / result_alpha).round() as u8
        } else {
            0
        };
    }
    pixels[offset + 3] = (result_alpha * 255.0).round() as u8;
}

fn style_color(style: &Map<String, Value>, key: &str, fallback: [u8; 4]) -> [u8; 4] {
    style
        .get(key)
        .and_then(Value::as_str)
        .and_then(parse_bgra_color)
        .unwrap_or(fallback)
}

fn parse_bgra_color(value: &str) -> Option<[u8; 4]> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 && value.len() != 8 {
        return None;
    }
    let red = u8::from_str_radix(&value[0..2], 16).ok()?;
    let green = u8::from_str_radix(&value[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&value[4..6], 16).ok()?;
    let alpha = if value.len() == 8 {
        u8::from_str_radix(&value[6..8], 16).ok()?
    } else {
        255
    };
    Some([blue, green, red, alpha])
}

fn json_number(value: &Value) -> Option<f64> {
    let parsed = match value {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    parsed.is_finite().then_some(parsed)
}

fn spawn_video_decoder(
    path: PathBuf,
    start_time: f64,
    rate: f64,
    loop_input: bool,
    clock_revision: u64,
    last_used_frame: u64,
    shared_error: Arc<Mutex<Option<String>>>,
    max_size: Option<(u32, u32)>,
) -> Result<VideoDecoder, String> {
    if !path.is_file() {
        return Err(format!("Video media does not exist: {}", path.display()));
    }
    let mut command = Command::new(ffmpeg_sidecar::paths::ffmpeg_path());
    command.args([
        "-hide_banner",
        "-loglevel",
        "info",
        // Native output is live playback, so spending the default multi-second
        // probe window before the first frame is not acceptable. The stream is
        // still decoded normally after this small initial probe.
        "-analyzeduration",
        "0",
        "-probesize",
        "32k",
        // Frame threading buffers one frame per thread, delaying live output
        // even when readrate is correct. Slice threading retains intra-frame
        // parallelism without that extra frame queue.
        "-thread_type",
        "slice",
    ]);
    if start_time > 0.0 {
        command.arg("-ss").arg(format!("{start_time:.3}"));
    }
    command.arg("-readrate").arg(format!("{rate:.3}"));
    if loop_input {
        command.args(["-stream_loop", "-1"]);
    }
    command.arg("-i").arg(&path);
    let filter = max_size.map_or_else(
        || "showinfo".to_string(),
        |(width, height)| {
            format!("scale={width}:{height}:force_original_aspect_ratio=decrease,showinfo")
        },
    );
    command.args(["-vf", &filter, "-fps_mode", "passthrough"]);
    command
        .args([
            "-an",
            "-sn",
            "-f",
            "image2pipe",
            "-vcodec",
            "ppm",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let duration = Arc::new(AtomicU64::new(0));
    let decoder_duration = duration.clone();
    let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let cancellation = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::sync_channel(2);
    let thread_child = Arc::clone(&child);
    let thread_cancellation = Arc::clone(&cancellation);
    let successful_eof = Arc::new(AtomicBool::new(false));
    let thread_eof = Arc::clone(&successful_eof);
    let decode_error = Arc::new(Mutex::new(None));
    let thread_decode_error = Arc::clone(&decode_error);
    // Process creation can be slow on Windows. Cached pixels must not wait for it.
    let worker = thread::Builder::new()
        .name("presenter-orbit-video-decoder".to_string())
        .spawn(move || {
            let result = (|| {
                if thread_cancellation.load(Ordering::Acquire) {
                    return Ok(());
                }
                let duration = decoder_duration;
                let mut process = command.spawn().map_err(|error| {
                    format!("Could not launch FFmpeg for {}: {error}", path.display())
                })?;
                let Some(stdout) = process.stdout.take() else {
                    let _ = process.kill();
                    let _ = process.wait();
                    return Err("FFmpeg did not expose its decoded video stream.".to_string());
                };
                let Some(stderr) = process.stderr.take() else {
                    let _ = process.kill();
                    let _ = process.wait();
                    return Err("FFmpeg did not expose decoded frame timestamps.".to_string());
                };
                let (timestamp_sender, timestamps) = mpsc::sync_channel(8);
                let reported = Arc::clone(&duration);
                if let Err(error) = thread::Builder::new()
                    .name("presenter-orbit-video-timestamps".to_string())
                    .spawn(move || {
                        for line in BufReader::new(stderr).lines() {
                            let Ok(line) = line else {
                                break;
                            };
                            if let Some(seconds) = reported_duration(&line) {
                                reported.store(seconds.to_bits(), Ordering::Release);
                            }
                            if let Some(position) = decoded_frame_position(&line) {
                                if timestamp_sender.send(position + start_time).is_err() {
                                    break;
                                }
                            }
                        }
                    })
                {
                    let _ = process.kill();
                    let _ = process.wait();
                    return Err(format!("Could not start video timestamp reader: {error}"));
                }

                {
                    let mut slot = match thread_child.lock() {
                        Ok(slot) => slot,
                        Err(_) => {
                            let _ = process.kill();
                            let _ = process.wait();
                            return Err("Video decoder process state is unavailable.".into());
                        }
                    };
                    if thread_cancellation.load(Ordering::Acquire) {
                        drop(slot);
                        let _ = process.kill();
                        let _ = process.wait();
                        return Ok(());
                    }
                    *slot = Some(process);
                }
                decode_video_loop(
                    BufReader::new(stdout),
                    sender,
                    timestamps,
                    &thread_cancellation,
                )
            })();
            let process = thread_child.lock().ok().and_then(|mut slot| slot.take());
            if let Some(mut process) = process {
                if thread_cancellation.load(Ordering::Acquire) || result.is_err() {
                    let _ = process.kill();
                }
                if let Ok(status) = process.wait() {
                    if result.is_ok()
                        && status.success()
                        && !thread_cancellation.load(Ordering::Acquire)
                    {
                        thread_eof.store(true, Ordering::Release);
                    }
                }
            }
            if !thread_cancellation.load(Ordering::Acquire) {
                if let Err(error) = result {
                    if let Ok(mut target) = thread_decode_error.lock() {
                        *target = Some(error.clone());
                    }
                    if let Ok(mut target) = shared_error.lock() {
                        *target = Some(error);
                    }
                }
            }
        })
        .map_err(|error| format!("Could not start video decoder: {error}"))?;
    Ok(VideoDecoder {
        receiver,
        duration,
        ended: false,
        latest: None,
        prepared_frame: false,
        latest_position: None,
        clock_revision,
        selection_id: None,
        last_used_frame,
        cancellation,
        child,
        worker: Some(worker),
        started_at: Instant::now(),
        successful_eof,
        completion_published: false,
        reverse_control: None,
        #[cfg(test)]
        reverse_sampler_done: None,
        decode_error,
    })
}

fn spawn_reverse_video_decoder(
    path: PathBuf,
    initial_position: f64,
    clock_revision: u64,
    last_used_frame: u64,
    shared_error: Arc<Mutex<Option<String>>>,
    max_size: Option<(u32, u32)>,
) -> Result<VideoDecoder, String> {
    if !path.is_file() {
        return Err(format!("Video media does not exist: {}", path.display()));
    }
    // A rendezvous channel holds no decoded-frame backlog. The worker can own
    // at most one sample while the renderer keeps its last displayed frame.
    let (sender, receiver) = mpsc::sync_channel(0);
    let target = Arc::new(ReverseVideoControl {
        target_seconds: Mutex::new(initial_position),
        changed: Condvar::new(),
        samples_started: AtomicU64::new(0),
    });
    let cancellation = Arc::new(AtomicBool::new(false));
    let child = Arc::new(Mutex::new(None));
    let duration = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let decode_error = Arc::new(Mutex::new(None));

    let thread_target = Arc::clone(&target);
    let thread_cancellation = Arc::clone(&cancellation);
    let thread_child = Arc::clone(&child);
    let thread_duration = Arc::clone(&duration);
    let thread_done = Arc::clone(&done);
    let thread_decode_error = Arc::clone(&decode_error);
    let thread_shared_error = Arc::clone(&shared_error);
    let worker = thread::Builder::new()
        .name("presenter-orbit-reverse-video".to_string())
        .spawn(move || {
            reverse_video_sampling_loop(
                path,
                thread_target,
                sender,
                thread_cancellation,
                thread_child,
                thread_duration,
                thread_decode_error,
                thread_shared_error,
                thread_done,
                max_size,
            );
        })
        .map_err(|error| format!("Could not start reverse video sampler: {error}"))?;

    Ok(VideoDecoder {
        receiver,
        duration,
        ended: false,
        latest: None,
        prepared_frame: false,
        latest_position: None,
        clock_revision,
        selection_id: None,
        last_used_frame,
        cancellation,
        child,
        worker: Some(worker),
        started_at: Instant::now(),
        successful_eof: Arc::new(AtomicBool::new(false)),
        completion_published: false,
        reverse_control: Some(target),
        #[cfg(test)]
        reverse_sampler_done: Some(done),
        decode_error,
    })
}

#[allow(clippy::too_many_arguments)]
fn reverse_video_sampling_loop(
    path: PathBuf,
    control: Arc<ReverseVideoControl>,
    sender: mpsc::SyncSender<DecodedVideoFrame>,
    cancellation: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    duration: Arc<AtomicU64>,
    decode_error: Arc<Mutex<Option<String>>>,
    shared_error: Arc<Mutex<Option<String>>>,
    done: Arc<AtomicBool>,
    max_size: Option<(u32, u32)>,
) {
    let mut last_target = None;
    let mut last_sample_started = None;
    while !cancellation.load(Ordering::Acquire) {
        let Some((target, started_at)) =
            next_reverse_video_target(&control, &cancellation, last_target, last_sample_started)
        else {
            break;
        };
        last_target = Some(target.to_bits());
        last_sample_started = Some(started_at);
        control.samples_started.fetch_add(1, Ordering::AcqRel);

        match decode_reverse_video_frame(&path, target, &child, &cancellation, &duration, max_size)
        {
            Ok(Some(frame)) => {
                if sender.send(frame).is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                if !cancellation.load(Ordering::Acquire) {
                    if let Ok(mut target) = decode_error.lock() {
                        *target = Some(error.clone());
                    }
                    if let Ok(mut target) = shared_error.lock() {
                        *target = Some(error);
                    }
                }
                break;
            }
        }
    }
    done.store(true, Ordering::Release);
}

fn next_reverse_video_target(
    control: &ReverseVideoControl,
    cancellation: &AtomicBool,
    last_target: Option<u64>,
    last_sample_started: Option<Instant>,
) -> Option<(f64, Instant)> {
    let mut target = control.target_seconds.lock().ok()?;
    loop {
        if cancellation.load(Ordering::Acquire) {
            return None;
        }
        if last_target.is_some_and(|last| last == target.to_bits()) {
            target = control.changed.wait(target).ok()?;
            continue;
        }

        let wait = last_sample_started
            .map(|started| REVERSE_VIDEO_SAMPLE_INTERVAL.saturating_sub(started.elapsed()))
            .unwrap_or(Duration::ZERO);
        if wait.is_zero() {
            return Some((*target, Instant::now()));
        }
        let (next_target, _) = control.changed.wait_timeout(target, wait).ok()?;
        target = next_target;
    }
}

fn decode_reverse_video_frame(
    path: &Path,
    target_seconds: f64,
    active_child: &Arc<Mutex<Option<Child>>>,
    cancellation: &AtomicBool,
    duration: &Arc<AtomicU64>,
    max_size: Option<(u32, u32)>,
) -> Result<Option<DecodedVideoFrame>, String> {
    if cancellation.load(Ordering::Acquire) {
        return Ok(None);
    }
    let mut command = Command::new(ffmpeg_sidecar::paths::ffmpeg_path());
    command.args([
        "-hide_banner",
        "-loglevel",
        "info",
        "-copyts",
        "-analyzeduration",
        "0",
        "-probesize",
        "32k",
        "-thread_type",
        "slice",
        "-ss",
    ]);
    command.arg(format!("{target_seconds:.6}"));
    command.arg("-i").arg(path);
    let filter = max_size.map_or_else(
        || "showinfo".to_string(),
        |(width, height)| {
            format!("scale={width}:{height}:force_original_aspect_ratio=decrease,showinfo")
        },
    );
    command.args([
        "-vf",
        &filter,
        "-frames:v",
        "1",
        "-fps_mode",
        "passthrough",
        "-an",
        "-sn",
        "-f",
        "image2pipe",
        "-vcodec",
        "ppm",
        "-pix_fmt",
        "rgb24",
        "pipe:1",
    ]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut process = command.spawn().map_err(|error| {
        format!(
            "Could not launch reverse video sampler for {}: {error}",
            path.display()
        )
    })?;
    let Some(stdout) = process.stdout.take() else {
        let _ = process.kill();
        let _ = process.wait();
        return Err("FFmpeg did not expose its reverse-sampled video frame.".to_string());
    };
    let Some(stderr) = process.stderr.take() else {
        let _ = process.kill();
        let _ = process.wait();
        return Err("FFmpeg did not expose reverse-sampled frame timestamps.".to_string());
    };
    let mut child_slot = match active_child.lock() {
        Ok(child_slot) => child_slot,
        Err(_) => {
            let _ = process.kill();
            let _ = process.wait();
            return Err("Reverse video decoder state is unavailable.".to_string());
        }
    };
    *child_slot = Some(process);
    drop(child_slot);
    if cancellation.load(Ordering::Acquire) {
        kill_active_video_child(active_child);
    }

    let frame_position = Arc::new(Mutex::new(None));
    let stderr_frame_position = Arc::clone(&frame_position);
    let stderr_duration = Arc::clone(duration);
    let stderr_worker = thread::Builder::new()
        .name("presenter-orbit-reverse-video-timestamps".to_string())
        .spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Some(seconds) = reported_duration(&line) {
                    stderr_duration.store(seconds.to_bits(), Ordering::Release);
                }
                if let Some(position) = decoded_frame_position(&line) {
                    if let Ok(mut position_slot) = stderr_frame_position.lock() {
                        if position_slot.is_none() {
                            *position_slot = Some(position);
                        }
                    }
                }
            }
        })
        .map_err(|error| {
            kill_active_video_child(active_child);
            let _ = wait_for_video_child(active_child, &AtomicBool::new(false));
            format!("Could not start reverse frame timestamp reader: {error}")
        })?;

    let image = read_ppm_frame(&mut BufReader::new(stdout));
    if image.is_err() || cancellation.load(Ordering::Acquire) {
        kill_active_video_child(active_child);
    }
    let process_status = wait_for_video_child(active_child, cancellation);
    let _ = stderr_worker.join();
    if cancellation.load(Ordering::Acquire) {
        return Ok(None);
    }
    let image = image?;
    let process_status = process_status?;
    if !process_status.success() {
        return Err("FFmpeg could not decode the requested reverse video frame.".into());
    }
    let position = frame_position
        .lock()
        .map_err(|_| "Reverse video timestamp is unavailable.")?
        .ok_or("FFmpeg did not report the reverse-sampled source timestamp.")?;
    Ok(Some(DecodedVideoFrame { image, position }))
}

fn kill_active_video_child(child: &Arc<Mutex<Option<Child>>>) {
    if let Ok(mut child) = child.lock() {
        if let Some(child) = child.as_mut() {
            let _ = child.kill();
        }
    }
}

fn wait_for_video_child(
    child: &Arc<Mutex<Option<Child>>>,
    cancellation: &AtomicBool,
) -> Result<std::process::ExitStatus, String> {
    loop {
        if cancellation.load(Ordering::Acquire) {
            kill_active_video_child(child);
        }
        let status: Option<std::process::ExitStatus> = {
            let mut child_slot = child
                .lock()
                .map_err(|_| "Reverse video decoder state is unavailable.")?;
            let child = child_slot
                .as_mut()
                .ok_or("Reverse video decoder process is unavailable.")?;
            match child.try_wait() {
                Ok(Some(status)) => {
                    child_slot.take();
                    return Ok(status);
                }
                Ok(None) => None,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    child_slot.take();
                    return Err(format!("Could not wait for reverse video frame: {error}"));
                }
            }
        };
        if status.is_none() {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

fn decode_video_loop(
    mut reader: BufReader<impl Read>,
    sender: mpsc::SyncSender<DecodedVideoFrame>,
    timestamps: mpsc::Receiver<f64>,
    cancellation: &AtomicBool,
) -> Result<(), String> {
    let mut pending_frame = None;
    while !cancellation.load(Ordering::Acquire) {
        if reader
            .fill_buf()
            .map_err(|error| format!("Could not read decoded video: {error}"))?
            .is_empty()
        {
            if let Some(mut frame) = pending_frame {
                while !cancellation.load(Ordering::Acquire) {
                    match sender.try_send(frame) {
                        Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => break,
                        Err(mpsc::TrySendError::Full(unsent)) => {
                            frame = unsent;
                            thread::sleep(Duration::from_millis(5));
                        }
                    }
                }
            }
            return Ok(());
        }
        let image = read_ppm_frame(&mut reader)?;
        if cancellation.load(Ordering::Acquire) {
            return Ok(());
        }
        let position = timestamps
            .recv_timeout(FIRST_VIDEO_FRAME_TIMEOUT)
            .map_err(|error| format!("Decoded video frame timestamp is unavailable: {error}"))?;
        match sender.try_send(DecodedVideoFrame { image, position }) {
            Ok(()) => pending_frame = None,
            Err(mpsc::TrySendError::Full(frame)) => pending_frame = Some(frame),
            Err(mpsc::TrySendError::Disconnected(_)) => return Ok(()),
        }
    }
    Ok(())
}

fn video_first_frame_timed_out(has_frame: bool, started_at: Instant, now: Instant) -> bool {
    !has_frame && now.saturating_duration_since(started_at) >= FIRST_VIDEO_FRAME_TIMEOUT
}

fn read_ppm_frame(reader: &mut impl Read) -> Result<DynamicImage, String> {
    if read_ppm_token(reader)? != "P6" {
        return Err("FFmpeg video stream did not contain binary PPM frames.".to_string());
    }
    let width = read_ppm_dimension(reader, "width")?;
    let height = read_ppm_dimension(reader, "height")?;
    if read_ppm_token(reader)? != "255" {
        return Err("FFmpeg PPM frame used an unsupported color depth.".to_string());
    }
    let frame_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or_else(|| "FFmpeg PPM frame size overflowed.".to_string())?;
    if frame_bytes > MAX_VIDEO_FRAME_BYTES {
        return Err("FFmpeg PPM frame exceeded the decoder size limit.".to_string());
    }
    let mut pixels = vec![0_u8; frame_bytes];
    reader
        .read_exact(&mut pixels)
        .map_err(|error| format!("Incomplete FFmpeg PPM frame: {error}"))?;
    image::RgbImage::from_raw(width, height, pixels)
        .map(DynamicImage::ImageRgb8)
        .ok_or_else(|| "FFmpeg produced an invalid PPM video frame.".to_string())
}

fn read_ppm_dimension(reader: &mut impl Read, name: &str) -> Result<u32, String> {
    let value = read_ppm_token(reader)?
        .parse::<u32>()
        .map_err(|_| format!("FFmpeg PPM frame had an invalid {name}."))?;
    if value == 0 || value > 8_192 {
        return Err(format!(
            "FFmpeg PPM frame {name} exceeded the decoder limit."
        ));
    }
    Ok(value)
}

fn read_ppm_token(reader: &mut impl Read) -> Result<String, String> {
    let mut token = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        reader
            .read_exact(&mut byte)
            .map_err(|error| format!("FFmpeg video stream ended: {error}"))?;
        if byte[0] == b'#' && token.is_empty() {
            while byte[0] != b'\n' {
                reader
                    .read_exact(&mut byte)
                    .map_err(|error| format!("Incomplete FFmpeg PPM comment: {error}"))?;
            }
            continue;
        }
        if byte[0].is_ascii_whitespace() {
            if token.is_empty() {
                continue;
            }
            break;
        }
        token.push(byte[0]);
        if token.len() > 32 {
            return Err("FFmpeg PPM header token exceeded the decoder limit.".to_string());
        }
    }
    String::from_utf8(token).map_err(|_| "FFmpeg PPM header was not ASCII.".to_string())
}

impl NativeImageAsset {
    fn prepare_reductions(&mut self, budget: usize) {
        if self.frames.len() != 1 {
            return;
        }
        let mut current = self.frames[0].clone();
        while current.width() > 64 || current.height() > 64 {
            let width = current.width().div_ceil(2);
            let height = current.height().div_ceil(2);
            let bytes = width as usize * height as usize * 4;
            if self.decoded_bytes.saturating_add(bytes) > budget {
                break;
            }
            let image = Arc::new(current.thumbnail(width, height));
            self.decoded_bytes += image.as_bytes().len();
            self.reduced_frames.push(image.clone());
            current = image;
        }
    }

    fn frame_at(&self, elapsed_seconds: f64) -> (usize, bool) {
        if self.frames.len() <= 1 || self.total_duration_seconds <= 0.0 {
            return (0, false);
        }
        let elapsed_seconds = elapsed_seconds.max(0.0);
        let (cycle_seconds, dynamic) = match self.loop_count {
            LoopCount::Infinite => (
                elapsed_seconds.rem_euclid(self.total_duration_seconds),
                true,
            ),
            LoopCount::Finite(count) => {
                let available_seconds = self.total_duration_seconds * f64::from(count.get());
                if elapsed_seconds >= available_seconds {
                    return (self.frames.len() - 1, false);
                }
                (
                    elapsed_seconds.rem_euclid(self.total_duration_seconds),
                    true,
                )
            }
        };
        let index = self
            .frame_ends_seconds
            .iter()
            .position(|end| cycle_seconds < *end)
            .unwrap_or(self.frames.len() - 1);
        (index, dynamic)
    }
}

pub(crate) fn image_file_stamp(path: &Path) -> Result<ImageFileStamp, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Could not inspect media {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("Media {} is not a file.", path.display()));
    }
    Ok(ImageFileStamp {
        bytes: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn decode_native_image_asset(
    path: &Path,
    stamp: ImageFileStamp,
    current_frame: u64,
    budget: Option<usize>,
) -> Result<NativeImageAsset, String> {
    if budget.is_some_and(|limit| stamp.bytes > limit as u64) {
        return Err("Image source exceeds the media preparation budget.".into());
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("Could not read media {}: {error}", path.display()))?;
    let is_gif = bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a");
    if !is_gif {
        let image = decode_image_bytes_bounded(path, &bytes, budget)?;
        let decoded_bytes = image.as_bytes().len();
        return Ok(NativeImageAsset {
            frames: vec![Arc::new(image)],
            reduced_frames: Vec::new(),
            frame_ends_seconds: vec![0.0],
            total_duration_seconds: 0.0,
            loop_count: LoopCount::Infinite,
            decoded_bytes,
            stamp,
            started_at: Instant::now(),
            last_used_frame: current_frame,
        });
    }

    let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes.as_slice()))
        .map_err(|error| format!("Could not decode GIF {}: {error}", path.display()))?;
    decoder
        .set_limits(image_limits(budget))
        .map_err(|error| format!("GIF dimensions are invalid for {}: {error}", path.display()))?;
    let loop_count = decoder.loop_count();
    let mut frames = Vec::new();
    let mut frame_ends_seconds = Vec::new();
    let mut total_duration_seconds = 0.0;
    let mut decoded_bytes = 0_usize;
    for (index, frame) in decoder.into_frames().enumerate() {
        if index >= MAX_ANIMATED_IMAGE_FRAMES {
            return Err(format!(
                "GIF {} exceeds the native limit of {MAX_ANIMATED_IMAGE_FRAMES} frames.",
                path.display()
            ));
        }
        let frame = frame.map_err(|error| {
            format!("Could not decode GIF frame for {}: {error}", path.display())
        })?;
        let (delay_numerator, delay_denominator) = frame.delay().numer_denom_ms();
        let delay_seconds = if delay_denominator == 0 {
            MIN_ANIMATED_FRAME_SECONDS
        } else {
            (f64::from(delay_numerator) / f64::from(delay_denominator) / 1_000.0)
                .max(MIN_ANIMATED_FRAME_SECONDS)
        };
        total_duration_seconds += delay_seconds;
        frame_ends_seconds.push(total_duration_seconds);
        let image = DynamicImage::ImageRgba8(frame.into_buffer());
        decoded_bytes = decoded_bytes.saturating_add(image.as_bytes().len());
        if budget.is_some_and(|limit| decoded_bytes > limit) {
            return Err("Image exceeds the media preparation budget.".into());
        }
        frames.push(Arc::new(image));
    }
    if frames.is_empty() {
        return Err(format!("GIF {} contains no frames.", path.display()));
    }
    Ok(NativeImageAsset {
        frames,
        reduced_frames: Vec::new(),
        frame_ends_seconds,
        total_duration_seconds,
        loop_count,
        decoded_bytes,
        stamp,
        started_at: Instant::now(),
        last_used_frame: current_frame,
    })
}

pub(crate) fn decode_image(path: &Path) -> Result<DynamicImage, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Could not read media {}: {error}", path.display()))?;
    decode_image_bytes(path, &bytes)
}

fn image_limits(budget: Option<usize>) -> Limits {
    let mut limits = Limits::no_limits();
    limits.max_alloc = budget.map(|bytes| bytes as u64);
    limits
}

fn decode_image_bytes(path: &Path, bytes: &[u8]) -> Result<DynamicImage, String> {
    decode_image_bytes_bounded(path, bytes, None)
}

fn decode_image_bytes_bounded(
    path: &Path,
    bytes: &[u8],
    budget: Option<usize>,
) -> Result<DynamicImage, String> {
    let is_svg = path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("svg"))
        || bytes
            .iter()
            .copied()
            .skip_while(u8::is_ascii_whitespace)
            .take(5)
            .eq(b"<svg ".iter().copied());
    if !is_svg {
        let mut reader = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|error| format!("Could not detect media {}: {error}", path.display()))?;
        reader.limits(image_limits(budget));
        return reader
            .decode()
            .map_err(|error| format!("Could not decode media {}: {error}", path.display()));
    }

    let mut options = resvg::usvg::Options {
        resources_dir: path.parent().map(Path::to_path_buf),
        ..resvg::usvg::Options::default()
    };
    options.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_data(&bytes, &options)
        .map_err(|error| format!("Could not parse SVG {}: {error}", path.display()))?;
    let size = tree.size().to_int_size();
    if size.width() == 0 || size.height() == 0 {
        return Err(format!(
            "SVG dimensions are invalid for {}.",
            path.display()
        ));
    }
    if budget
        .is_some_and(|limit| u64::from(size.width()) * u64::from(size.height()) * 4 > limit as u64)
    {
        return Err("SVG exceeds the media preparation budget.".into());
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| format!("Could not allocate SVG surface for {}.", path.display()))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );
    let mut rgba = Vec::with_capacity(size.width() as usize * size.height() as usize * 4);
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }
    let image = RgbaImage::from_raw(size.width(), size.height(), rgba)
        .ok_or_else(|| format!("Could not create SVG image for {}.", path.display()))?;
    Ok(DynamicImage::ImageRgba8(image))
}

fn blend_bgra_pixel(pixels: &mut [u8], width: usize, x: usize, y: usize, source: [u8; 4]) {
    blend_bgra_pixel_with_factor(pixels, width, x, y, source, 1.0);
}
