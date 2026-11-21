//! Native NDI sender support for the Tauri host.
//!
//! The NDI SDK is an optional runtime dependency.  We deliberately load the
//! runtime at application start instead of linking against an SDK import
//! library.  This keeps the application distributable without bundling NDI
//! binaries while still allowing an installed NDI Runtime to provide the
//! sender implementation at runtime.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::{c_char, c_int, c_void, CStr, CString},
    mem,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};

use crate::native_compositor::NativeCompositor;
use crate::output_playback::normalize_audio_settings_value;

const BGRA_FOURCC: u32 = u32::from_le_bytes(*b"BGRA");
const PROGRESSIVE_FRAME: u32 = 1;

type InitializeFn = unsafe extern "C" fn() -> bool;
type DestroyFn = unsafe extern "C" fn();
type SendCreateFn = unsafe extern "C" fn(*const NDIlibSendCreate) -> *mut c_void;
type SendDestroyFn = unsafe extern "C" fn(*mut c_void);
type SendVideoV2Fn = unsafe extern "C" fn(*mut c_void, *const NDIlibVideoFrameV2);
type SendAudioV3Fn = unsafe extern "C" fn(*mut c_void, *const NDIlibAudioFrameV3);

#[repr(C)]
struct NDIlibSendCreate {
    p_ndi_name: *const c_char,
    p_groups: *const c_char,
    clock_video: u8,
    clock_audio: u8,
}

#[repr(C)]
struct NDIlibVideoFrameV2 {
    xres: c_int,
    yres: c_int,
    four_cc: u32,
    frame_rate_n: c_int,
    frame_rate_d: c_int,
    picture_aspect_ratio: f32,
    frame_format_type: u32,
    timecode: i64,
    p_data: *const u8,
    line_stride_in_bytes: c_int,
    p_metadata: *const c_char,
    timestamp: i64,
}

/// ABI layout of `NDIlib_audio_frame_v3_t` from Processing.NDI.structs.h.
///
/// NDI's native floating-point audio format is planar (`FLTp`), so
/// `channel_stride_in_bytes` describes the distance between channel planes.
#[repr(C)]
pub(crate) struct NDIlibAudioFrameV3 {
    pub(crate) sample_rate: c_int,
    pub(crate) no_channels: c_int,
    pub(crate) no_samples: c_int,
    timecode: i64,
    pub(crate) four_cc: u32,
    pub(crate) p_data: *mut u8,
    pub(crate) channel_stride_in_bytes: c_int,
    p_metadata: *const c_char,
    timestamp: i64,
}

const FLTP_FOURCC: u32 = u32::from_le_bytes(*b"FLTp");

#[derive(Debug, Clone, PartialEq, Eq)]
struct NdiConfig {
    name: String,
    width: usize,
    height: usize,
    frame_rate_n: i32,
    frame_rate_d: i32,
    alpha_mode: AlphaMode,
    video_delay: usize,
}

impl NdiConfig {
    fn from_value(screen_id: &str, value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "NDI screen configuration must be an object.".to_string())?;
        let width = positive_dimension(object.get("ndiWidth"), 1920)?;
        let height = positive_dimension(object.get("ndiHeight"), 1080)?;
        let framerate = positive_framerate(object.get("ndiFramerate"), 60.0)?;
        let (frame_rate_n, frame_rate_d) = frame_rate_fraction(framerate);
        let name = object
            .get("ndiName")
            .or_else(|| object.get("label"))
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(screen_id)
            .trim()
            .to_string();
        if name.is_empty() {
            return Err("NDI sender name cannot be empty.".to_string());
        }
        let video_delay = crate::output_delay::frames(object.get("videoDelay"))?;
        crate::output_delay::validate_size(width, height, video_delay)?;
        Ok(Self {
            name,
            width,
            height,
            frame_rate_n,
            frame_rate_d,
            alpha_mode: AlphaMode::parse(object.get("ndiAlphaMode"))?,
            video_delay,
        })
    }
}

fn positive_dimension(value: Option<&Value>, default: usize) -> Result<usize, String> {
    let parsed = value
        .and_then(Value::as_f64)
        .unwrap_or(default as f64)
        .round();
    if !parsed.is_finite() || parsed < 16.0 || parsed > 16_384.0 {
        return Err("NDI frame dimensions must be between 16 and 16384 pixels.".to_string());
    }
    Ok(parsed as usize)
}

fn positive_framerate(value: Option<&Value>, default: f64) -> Result<f64, String> {
    let parsed = value.and_then(Value::as_f64).unwrap_or(default);
    if !parsed.is_finite() || parsed < 1.0 || parsed > 240.0 {
        return Err("NDI frame rate must be between 1 and 240 frames per second.".to_string());
    }
    Ok(parsed)
}

fn frame_rate_fraction(value: f64) -> (i32, i32) {
    if (value - 59.94).abs() < 0.01 {
        (60_000, 1_001)
    } else if (value - 29.97).abs() < 0.01 {
        (30_000, 1_001)
    } else {
        (value.round() as i32, 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlphaMode {
    None,
    Straight,
    Premultiplied,
}

impl AlphaMode {
    fn parse(value: Option<&Value>) -> Result<Self, String> {
        match value.and_then(Value::as_str) {
            None if value.is_none() => Ok(Self::None),
            Some("none") => Ok(Self::None),
            Some("straight") => Ok(Self::Straight),
            Some("premultiplied") => Ok(Self::Premultiplied),
            _ => Err("NDI alpha mode must be none, straight or premultiplied.".into()),
        }
    }
    fn pixels(self, source: &[u8]) -> Vec<u8> {
        let mut pixels = source.to_vec();
        for pixel in pixels.chunks_exact_mut(4) {
            match self {
                Self::None => pixel[3] = 255,
                Self::Straight => {
                    if pixel[3] == 0 {
                        pixel[..3].fill(0);
                    }
                }
                Self::Premultiplied => {
                    let alpha = pixel[3] as u16;
                    for channel in &mut pixel[..3] {
                        *channel = ((*channel as u16 * alpha + 127) / 255) as u8;
                    }
                }
            }
        }
        pixels
    }
}

fn render_ndi_frame(
    compositor: &mut NativeCompositor,
    screen_id: &str,
    content: &Value,
    config: &NdiConfig,
) -> Result<crate::native_compositor::NativeFrame, String> {
    let mut frame = compositor.try_render_layered_for(
        &format!("ndi:{screen_id}"),
        content,
        config.width,
        config.height,
    )?;
    if config.alpha_mode != AlphaMode::None {
        frame.layers.retain(|layer| layer.id != "background-color");
    }
    Ok(frame.flatten())
}

fn render_ndi_key_fill_pair(
    compositor: &mut NativeCompositor,
    pair: &NdiKeyFillPair,
    source_content: &Value,
    key_content: &Value,
) -> Result<
    Option<(
        crate::native_compositor::NativeFrame,
        crate::native_compositor::NativeFrame,
    )>,
    String,
> {
    if source_content
        .as_object()
        .is_some_and(|content| content.is_empty())
        || key_content
            .as_object()
            .is_some_and(|content| content.is_empty())
    {
        return Ok(None);
    }
    validate_key_fill_pair_content(pair, source_content, key_content)?;
    compositor.set_video_max_size(Some((pair.width as u32, pair.height as u32)));
    let (fill, key) = compositor.try_render_key_fill_pair_for(
        &format!("ndi-key-fill:{}", pair.source_screen_id),
        source_content,
        pair.width,
        pair.height,
    )?;
    Ok(Some((fill.flatten(), key.flatten())))
}

fn validate_key_fill_pair_content(
    pair: &NdiKeyFillPair,
    source_content: &Value,
    key_content: &Value,
) -> Result<(), String> {
    let source_id = source_content
        .get("keyFillSourceId")
        .and_then(Value::as_str);
    let source_partner = source_content
        .get("keyFillPartnerId")
        .and_then(Value::as_str);
    let key_source_id = key_content.get("keyFillSourceId").and_then(Value::as_str);
    let key_partner = key_content.get("keyFillPartnerId").and_then(Value::as_str);
    if source_id != Some(pair.source_screen_id.as_str())
        || source_partner != Some(pair.key_screen_id.as_str())
        || key_source_id != Some(pair.source_screen_id.as_str())
        || key_partner != Some(pair.source_screen_id.as_str())
    {
        return Err("Paired NDI scenes do not identify the same fill and key outputs.".to_string());
    }
    let fill_signal: crate::key_fill::KeyFillSignal = serde_json::from_value(
        source_content
            .get("keyFillSignal")
            .cloned()
            .ok_or_else(|| "Paired NDI fill scene has no signal mode.".to_string())?,
    )
    .map_err(|error| format!("Paired NDI fill signal is invalid: {error}"))?;
    if fill_signal == crate::key_fill::KeyFillSignal::Key {
        return Err("Paired NDI source scene cannot use the key signal mode.".to_string());
    }
    let key_signal: crate::key_fill::KeyFillSignal = serde_json::from_value(
        key_content
            .get("keyFillSignal")
            .cloned()
            .ok_or_else(|| "Paired NDI key scene has no signal mode.".to_string())?,
    )
    .map_err(|error| format!("Paired NDI key signal is invalid: {error}"))?;
    if key_signal != crate::key_fill::KeyFillSignal::Key {
        return Err("Paired NDI partner scene must use the key signal mode.".to_string());
    }
    Ok(())
}

#[derive(Debug)]
struct DynamicLibrary {
    handle: *mut c_void,
    path: PathBuf,
}

// The OS loader handle can be moved with the service mutex.  Calls remain
// serialized by NdiService, and the handle is never exposed to the renderer.
unsafe impl Send for DynamicLibrary {}
unsafe impl Sync for DynamicLibrary {}

impl DynamicLibrary {
    fn open(path: &Path) -> Result<Self, String> {
        let handle = unsafe { platform::open(path)? };
        Ok(Self {
            handle,
            path: path.to_path_buf(),
        })
    }

    fn symbol<T: Copy>(&self, name: &'static [u8]) -> Result<T, String> {
        let symbol = unsafe { platform::symbol(self.handle, name)? };
        // Function pointers have the same representation as the pointers
        // returned by GetProcAddress/dlsym on supported targets.
        Ok(unsafe { mem::transmute_copy(&symbol) })
    }
}

impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        unsafe { platform::close(self.handle) };
    }
}

#[derive(Debug)]
pub(crate) struct NdiRuntime {
    library: DynamicLibrary,
    destroy: DestroyFn,
    send_create: SendCreateFn,
    send_destroy: SendDestroyFn,
    send_video_v2: SendVideoV2Fn,
    send_audio_v3: Option<SendAudioV3Fn>,
}

unsafe impl Send for NdiRuntime {}
unsafe impl Sync for NdiRuntime {}

impl NdiRuntime {
    pub(crate) fn symbol<T: Copy>(&self, name: &'static [u8]) -> Result<T, String> {
        self.library.symbol(name)
    }
    fn load() -> Result<Self, String> {
        let mut errors = Vec::new();
        for path in runtime_candidates() {
            if path.is_absolute() && !path.exists() {
                continue;
            }
            match Self::load_from_path(&path) {
                Ok(runtime) => return Ok(runtime),
                Err(error) => errors.push(format!("{}: {error}", path.display())),
            }
        }
        if errors.is_empty() {
            Err("NDI Runtime library was not found.".to_string())
        } else {
            Err(format!(
                "NDI Runtime could not be loaded: {}",
                errors.join("; ")
            ))
        }
    }

    fn load_from_path(path: &Path) -> Result<Self, String> {
        let library = DynamicLibrary::open(path)?;
        let initialize: InitializeFn = library.symbol(b"NDIlib_initialize\0")?;
        let destroy: DestroyFn = library.symbol(b"NDIlib_destroy\0")?;
        let send_create: SendCreateFn = library.symbol(b"NDIlib_send_create\0")?;
        let send_destroy: SendDestroyFn = library.symbol(b"NDIlib_send_destroy\0")?;
        let send_video_v2: SendVideoV2Fn = library.symbol(b"NDIlib_send_send_video_v2\0")?;
        // NDI 4 introduced the v3 floating-point audio frame. Keep the
        // symbol optional so older runtimes can still provide video output;
        // status reports the missing audio ABI explicitly.
        let send_audio_v3 = library
            .symbol::<SendAudioV3Fn>(b"NDIlib_send_send_audio_v3\0")
            .ok();
        if !unsafe { initialize() } {
            // NDI recommends balancing a failed initialization attempt with
            // destroy before releasing the dynamic library handle.
            unsafe { destroy() };
            return Err("NDIlib_initialize returned false.".to_string());
        }
        Ok(Self {
            library,
            destroy,
            send_create,
            send_destroy,
            send_video_v2,
            send_audio_v3,
        })
    }

    fn create_sender(&self, name: &str) -> Result<*mut c_void, String> {
        let name =
            CString::new(name).map_err(|_| "NDI sender name contains a NUL byte.".to_string())?;
        let create = NDIlibSendCreate {
            p_ndi_name: name.as_ptr(),
            p_groups: std::ptr::null(),
            clock_video: 0,
            clock_audio: 0,
        };
        let sender = unsafe { (self.send_create)(&create) };
        if sender.is_null() {
            Err("NDIlib_send_create returned a null sender.".to_string())
        } else {
            Ok(sender)
        }
    }

    fn destroy_sender(&self, sender: *mut c_void) {
        if !sender.is_null() {
            unsafe { (self.send_destroy)(sender) };
        }
    }
}

impl Drop for NdiRuntime {
    fn drop(&mut self) {
        unsafe { (self.destroy)() };
    }
}

pub(crate) fn shared_runtime() -> Result<Arc<NdiRuntime>, String> {
    static RUNTIME: OnceLock<Result<Arc<NdiRuntime>, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| NdiRuntime::load().map(Arc::new))
        .clone()
}

#[derive(Debug)]
struct NdiOutput {
    config: NdiConfig,
    sender: *mut c_void,
    pixels: Vec<u8>,
    timecode: i64,
    timestamp: i64,
    pair_generation: Option<u64>,
    last_content: Value,
    next_frame_at: Instant,
    frames_sent: u64,
    audio_frames_sent: u64,
    /// The first audio packet is aligned to the current video timecode. After
    /// that, the clock advances by the exact number of PCM samples sent.
    audio_timecode: Option<i64>,
    audio_timecode_remainder: u64,
    audio_processor: NdiAudioPcmProcessor,
    audio_settings_key: String,
    dynamic_media: bool,
    video_delay: crate::output_delay::OutputDelay,
}

unsafe impl Send for NdiOutput {}
unsafe impl Sync for NdiOutput {}

impl NdiOutput {
    fn accept_frame(
        &mut self,
        frame: crate::native_compositor::NativeFrame,
        now: Instant,
    ) -> Result<(), String> {
        self.dynamic_media = frame.dynamic_media || self.config.video_delay > 0;
        if let Some(frame) = self.video_delay.push(
            frame,
            self.config.video_delay,
            self.config.frame_interval(),
            now,
        )? {
            self.pixels = frame.pixels;
        }
        Ok(())
    }

    fn update_audio_settings(&mut self, content: &Value) {
        let settings =
            normalize_audio_settings_value(content.get("audioSettings").unwrap_or(&Value::Null));
        let key = serde_json::to_string(&settings).unwrap_or_default();
        if self.audio_settings_key == key {
            return;
        }
        self.audio_processor = NdiAudioPcmProcessor::from_audio_settings(&settings);
        self.audio_settings_key = key;
        self.audio_timecode = None;
        self.audio_timecode_remainder = 0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NdiKeyFillPair {
    source_screen_id: String,
    key_screen_id: String,
    fill_signal: crate::key_fill::KeyFillSignal,
    width: usize,
    height: usize,
    frame_rate_n: i32,
    frame_rate_d: i32,
    video_delay: usize,
}

impl NdiKeyFillPair {
    fn frame_interval(&self) -> Duration {
        let seconds = self.frame_rate_d as f64 / self.frame_rate_n.max(1) as f64;
        Duration::from_secs_f64(seconds.max(1.0 / 240.0))
    }
}

struct NdiKeyFillState {
    pair: NdiKeyFillPair,
    timecode: i64,
    next_frame_at: Instant,
    generation: u64,
    fill_signal: Option<crate::key_fill::KeyFillSignal>,
    dynamic_media: bool,
    delay: crate::key_fill::PairDelay,
}

impl NdiKeyFillState {
    fn new(pair: NdiKeyFillPair, timecode: i64, now: Instant) -> Self {
        let fill_signal = pair.fill_signal;
        Self {
            pair,
            timecode,
            next_frame_at: now,
            generation: 0,
            fill_signal: Some(fill_signal),
            dynamic_media: false,
            delay: crate::key_fill::PairDelay::default(),
        }
    }
}

fn configured_ndi_key_fill_pairs(
    screens: &serde_json::Map<String, Value>,
) -> (
    HashMap<String, NdiKeyFillPair>,
    HashMap<String, String>,
    HashMap<String, String>,
) {
    let mut pairs = HashMap::new();
    let mut members = HashMap::new();
    let mut errors = HashMap::new();
    let mut assigned_keys = HashSet::new();

    for (source_screen_id, source) in screens {
        let pair = match crate::key_fill::parse_optional(source.get("keyFill")) {
            Ok(Some(pair)) => pair,
            Ok(None) => continue,
            Err(error) => {
                if screen_target_type(source) == "ndi" {
                    errors.insert(source_screen_id.clone(), error);
                }
                continue;
            }
        };
        let key_screen_id = pair.key_screen_id;
        let Some(key) = screens.get(&key_screen_id) else {
            errors.insert(
                source_screen_id.clone(),
                "Paired NDI key screen is unavailable.".to_string(),
            );
            continue;
        };

        let source_is_ndi = screen_target_type(source) == "ndi";
        let key_is_ndi = screen_target_type(key) == "ndi";
        if !source_is_ndi && !key_is_ndi {
            continue;
        }
        members.insert(source_screen_id.clone(), source_screen_id.clone());
        members.insert(key_screen_id.clone(), source_screen_id.clone());

        let error = if source_screen_id == &key_screen_id {
            Some("Paired NDI key/fill outputs require two distinct screens.".to_string())
        } else if !source_is_ndi || !key_is_ndi {
            Some("Paired NDI key/fill outputs must both target NDI.".to_string())
        } else if screen_enabled(source) != screen_enabled(key) {
            Some("Paired NDI key/fill outputs must have matching enabled states.".to_string())
        } else if !screen_enabled(source) {
            None
        } else if !assigned_keys.insert(key_screen_id.clone()) {
            Some("An NDI key output cannot be assigned to more than one fill output.".to_string())
        } else {
            match ndi_key_fill_pair_config(
                source_screen_id,
                &key_screen_id,
                source,
                key,
                pair.fill_mode,
            ) {
                Ok(pair) => {
                    pairs.insert(source_screen_id.clone(), pair);
                    None
                }
                Err(error) => Some(error),
            }
        };
        if let Some(error) = error {
            errors.insert(source_screen_id.clone(), error.clone());
            errors.insert(key_screen_id, error);
        }
    }

    (pairs, members, errors)
}

fn screen_target_type(screen: &Value) -> &str {
    screen
        .get("targetType")
        .and_then(Value::as_str)
        .unwrap_or("display")
}

fn screen_enabled(screen: &Value) -> bool {
    screen
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn ndi_key_fill_pair_config(
    source_screen_id: &str,
    key_screen_id: &str,
    source: &Value,
    key: &Value,
    fill_mode: crate::key_fill::FillMode,
) -> Result<NdiKeyFillPair, String> {
    for screen in [source, key] {
        if screen
            .get("outputFormat")
            .and_then(Value::as_str)
            .is_some_and(|format| format != "sdr")
        {
            return Err("Paired NDI key/fill outputs currently require SDR.".to_string());
        }
    }
    let source_config = NdiConfig::from_value(source_screen_id, source)?;
    let key_config = NdiConfig::from_value(key_screen_id, key)?;
    if source_config.width != key_config.width
        || source_config.height != key_config.height
        || source_config.frame_rate_n != key_config.frame_rate_n
        || source_config.frame_rate_d != key_config.frame_rate_d
        || source_config.video_delay != key_config.video_delay
    {
        return Err(
            "Paired NDI key/fill outputs must have matching dimensions, frame rates and video delay."
                .to_string(),
        );
    }
    if source_config.name == key_config.name {
        return Err("Paired NDI key/fill outputs require distinct sender names.".to_string());
    }
    let paired_height = source_config
        .height
        .checked_mul(2)
        .ok_or_else(|| "Paired NDI key/fill dimensions overflow.".to_string())?;
    crate::output_delay::validate_size(
        source_config.width,
        paired_height,
        source_config.video_delay,
    )
    .map_err(|_| "Paired NDI delay exceeds the 128 MiB shared buffer limit.".to_string())?;
    Ok(NdiKeyFillPair {
        source_screen_id: source_screen_id.to_string(),
        key_screen_id: key_screen_id.to_string(),
        fill_signal: match fill_mode {
            crate::key_fill::FillMode::Straight => crate::key_fill::KeyFillSignal::FillStraight,
            crate::key_fill::FillMode::Premultiplied => {
                crate::key_fill::KeyFillSignal::FillPremultiplied
            }
        },
        width: source_config.width,
        height: source_config.height,
        frame_rate_n: source_config.frame_rate_n,
        frame_rate_d: source_config.frame_rate_d,
        video_delay: source_config.video_delay,
    })
}

/// Native NDI service owned by the Tauri application state.
///
/// It provides the runtime/sender lifecycle and a native scene compositor. The
/// compositor intentionally accepts the renderer's JSON output model, but it
/// does not depend on WebView or desktop capture. It supports CSS colors and
/// simple gradients, native raster/SVG media decoding, and shaped system-font
/// text.
/// This native framebuffer is the only NDI frame source on every supported
/// operating system; NDI output never depends on a capturable WebView window.
pub struct NdiService {
    frozen: bool,
    runtime: Option<Arc<NdiRuntime>>,
    compositor: NativeCompositor,
    outputs: HashMap<String, NdiOutput>,
    key_fill_pairs: HashMap<String, NdiKeyFillPair>,
    key_fill_pair_members: HashMap<String, String>,
    key_fill_pair_states: HashMap<String, NdiKeyFillState>,
    error: Option<String>,
    output_errors: HashMap<String, String>,
}

impl NdiService {
    pub(crate) fn set_frozen(&mut self, frozen: bool) {
        self.frozen = frozen;
    }

    pub fn new() -> Self {
        let (runtime, error) = match shared_runtime() {
            Ok(runtime) => (Some(runtime), None),
            Err(error) => (None, Some(error)),
        };
        Self {
            frozen: false,
            runtime,
            compositor: NativeCompositor::new(),
            outputs: HashMap::new(),
            key_fill_pairs: HashMap::new(),
            key_fill_pair_members: HashMap::new(),
            key_fill_pair_states: HashMap::new(),
            error,
            output_errors: HashMap::new(),
        }
    }

    pub fn status(&self) -> Value {
        let runtime_available = self.runtime.is_some();
        let audio_abi_available = self
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.send_audio_v3.is_some());
        let frames_sent = self
            .outputs
            .values()
            .map(|output| output.frames_sent)
            .sum::<u64>();
        let audio_frames_sent = self
            .outputs
            .values()
            .map(|output| output.audio_frames_sent)
            .sum::<u64>();
        json!({
            "available": runtime_available,
            "frozen": self.frozen,
            "ndiRuntimeAvailable": runtime_available,
            "ndiSenderAvailable": runtime_available,
            "ndiError": self.error.clone().unwrap_or_default(),
            "ndiOutputErrors": self.output_errors,
            "runtimePath": self.runtime.as_ref().map(|runtime| runtime.library.path.to_string_lossy().to_string()),
            "activeSenders": self.outputs.len(),
            "framesSent": frames_sent,
            "ndiAudioAbiAvailable": audio_abi_available,
            "ndiAudioFramesSent": audio_frames_sent,
            "ndiAudioPipeline": if audio_abi_available { "native-tap+decoder" } else { "unavailable" },
            "frameSource": "native-rust",
            "nativeTextRenderer": "cosmic-text",
            "nativeImageDecoder": "image+resvg",
            "nativeVideoDecoderAvailable": self.compositor.video_available(),
            "nativeVideoDecoderError": self.compositor.video_error(),
        })
    }

    pub fn apply_screen_config(&mut self, config: &Value) -> Result<(), String> {
        let object = config
            .as_object()
            .ok_or_else(|| "Screen configuration must be an object.".to_string())?;
        let (pairs, pair_members, pair_errors) = configured_ndi_key_fill_pairs(object);
        let now = Instant::now();
        let mut previous_states = mem::take(&mut self.key_fill_pair_states);
        self.key_fill_pairs = pairs;
        let previous_pair_members = mem::replace(&mut self.key_fill_pair_members, pair_members);
        let mut reset_pair_members = HashSet::new();
        let mut preserved_pairs = HashSet::new();
        for (source_screen_id, pair) in &self.key_fill_pairs {
            let previous = previous_states.remove(source_screen_id);
            let state = match previous {
                Some(state) if state.pair == *pair => {
                    preserved_pairs.insert(source_screen_id.clone());
                    state
                }
                previous => {
                    reset_pair_members.insert(pair.source_screen_id.clone());
                    reset_pair_members.insert(pair.key_screen_id.clone());
                    if let Some(previous) = previous {
                        reset_pair_members.insert(previous.pair.source_screen_id.clone());
                        reset_pair_members.insert(previous.pair.key_screen_id.clone());
                    }
                    let timecode = self
                        .outputs
                        .get(source_screen_id)
                        .map(|output| output.timecode)
                        .unwrap_or(0)
                        .max(
                            self.outputs
                                .get(&pair.key_screen_id)
                                .map(|output| output.timecode)
                                .unwrap_or(0),
                        );
                    NdiKeyFillState::new(pair.clone(), timecode, now)
                }
            };
            self.key_fill_pair_states
                .insert(source_screen_id.clone(), state);
        }
        for state in previous_states.into_values() {
            reset_pair_members.insert(state.pair.source_screen_id);
            reset_pair_members.insert(state.pair.key_screen_id);
        }
        for source_screen_id in &preserved_pairs {
            if let Some(pair) = self.key_fill_pairs.get(source_screen_id) {
                reset_pair_members.remove(&pair.source_screen_id);
                reset_pair_members.remove(&pair.key_screen_id);
            }
        }
        for screen_id in previous_pair_members.keys() {
            if !self.key_fill_pair_members.contains_key(screen_id) {
                reset_pair_members.insert(screen_id.clone());
            }
        }

        let paired_ids: HashSet<String> = self.key_fill_pair_members.keys().cloned().collect();
        for (screen_id, output) in &mut self.outputs {
            let was_pair_member = previous_pair_members.contains_key(screen_id);
            let is_pair_member = paired_ids.contains(screen_id);
            if reset_pair_members.contains(screen_id) || was_pair_member != is_pair_member {
                output.last_content = json!({});
                output.pixels.fill(0);
                output.dynamic_media = false;
                output.video_delay.clear();
                output.pair_generation = None;
            }
            if is_pair_member {
                output.video_delay.clear();
                if let Some(source_screen_id) = self.key_fill_pair_members.get(screen_id) {
                    if let Some(state) = self.key_fill_pair_states.get(source_screen_id) {
                        output.next_frame_at = state.next_frame_at;
                    }
                }
            }
        }
        let configured_ids: Vec<String> = object.keys().cloned().collect();
        let stale: Vec<String> = self
            .outputs
            .keys()
            .filter(|id| !configured_ids.iter().any(|candidate| candidate == *id))
            .cloned()
            .collect();
        for screen_id in stale {
            self.stop_output(&screen_id);
        }
        for (screen_id, screen_config) in object {
            let enabled = screen_config
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let target_type = screen_config
                .get("targetType")
                .and_then(Value::as_str)
                .unwrap_or("display");
            if let Some(error) = pair_errors.get(screen_id) {
                self.stop_output(screen_id);
                self.output_errors.insert(screen_id.clone(), error.clone());
                continue;
            }
            if enabled && target_type == "ndi" {
                match self.start_or_update(screen_id, screen_config) {
                    Ok(()) => {
                        self.output_errors.remove(screen_id);
                    }
                    Err(error) => {
                        self.output_errors.insert(screen_id.clone(), error);
                    }
                }
            } else {
                self.stop_output(screen_id);
            }
        }
        let configured_pairs: Vec<NdiKeyFillPair> = self.key_fill_pairs.values().cloned().collect();
        for pair in configured_pairs {
            let source_active = self.outputs.contains_key(&pair.source_screen_id);
            let key_active = self.outputs.contains_key(&pair.key_screen_id);
            if source_active != key_active {
                self.stop_output(&pair.source_screen_id);
                self.stop_output(&pair.key_screen_id);
                let error = "Paired NDI key/fill senders must both be active.".to_string();
                self.output_errors
                    .insert(pair.source_screen_id.clone(), error.clone());
                self.output_errors.insert(pair.key_screen_id.clone(), error);
            }
        }
        Ok(())
    }

    pub fn start_or_update(&mut self, screen_id: &str, config: &Value) -> Result<(), String> {
        let normalized = NdiConfig::from_value(screen_id, config)?;
        let requested_pair = crate::key_fill::parse_optional(config.get("keyFill"))?;
        if let Some(requested_pair) = requested_pair {
            let configured_pair = self.key_fill_pairs.get(screen_id).ok_or_else(|| {
                "Paired NDI output requires a validated complete screen configuration.".to_string()
            })?;
            if configured_pair.key_screen_id != requested_pair.key_screen_id {
                return Err("Paired NDI screen configuration is stale.".into());
            }
        }
        if self
            .outputs
            .get(screen_id)
            .map(|output| output.config == normalized)
            .unwrap_or(false)
        {
            return Ok(());
        }
        if let Some(output) = self.outputs.get_mut(screen_id) {
            let mut previous = output.config.clone();
            previous.alpha_mode = normalized.alpha_mode;
            previous.video_delay = normalized.video_delay;
            if previous == normalized {
                if self.key_fill_pair_members.contains_key(screen_id) {
                    output.config = normalized;
                    output.video_delay.clear();
                    return Ok(());
                }
                let frame = render_ndi_frame(
                    &mut self.compositor,
                    screen_id,
                    &output.last_content,
                    &normalized,
                )?;
                output.config = normalized;
                output.video_delay = Default::default();
                output.pixels.fill(0);
                output.accept_frame(frame, Instant::now())?;
                output.timecode = next_timecode(output.timecode, 0);
                let runtime = self
                    .runtime
                    .as_ref()
                    .ok_or_else(|| "NDI sender is unavailable.".to_string())?;
                send_frame_with_runtime(runtime, output)?;
                output.frames_sent = output.frames_sent.saturating_add(1);
                output.next_frame_at =
                    next_frame_deadline(output.config.frame_interval(), Instant::now());
                return Ok(());
            }
        }
        self.stop_output(screen_id);
        let runtime = self.runtime.as_ref().ok_or_else(|| {
            self.error
                .clone()
                .unwrap_or_else(|| "NDI sender is unavailable.".to_string())
        })?;
        let sender = runtime.create_sender(&normalized.name)?;
        let mut output = NdiOutput {
            pixels: vec![0; normalized.width * normalized.height * 4],
            config: normalized,
            sender,
            timecode: 0,
            timestamp: 0,
            pair_generation: None,
            last_content: json!({}),
            next_frame_at: Instant::now(),
            frames_sent: 0,
            audio_frames_sent: 0,
            audio_timecode: None,
            audio_timecode_remainder: 0,
            audio_processor: NdiAudioPcmProcessor::from_audio_settings(&Value::Null),
            audio_settings_key: String::new(),
            dynamic_media: false,
            video_delay: Default::default(),
        };
        if self.key_fill_pair_members.contains_key(screen_id) {
            if let Some(source_screen_id) = self.key_fill_pair_members.get(screen_id) {
                if let Some(state) = self.key_fill_pair_states.get(source_screen_id) {
                    output.next_frame_at = state.next_frame_at;
                }
            }
            self.outputs.insert(screen_id.to_string(), output);
            self.output_errors.remove(screen_id);
            return Ok(());
        }
        let frame = render_ndi_frame(&mut self.compositor, screen_id, &json!({}), &output.config)?;
        output.accept_frame(frame, Instant::now())?;
        if let Err(error) = self.send_frame(&mut output) {
            runtime.destroy_sender(sender);
            return Err(error);
        }
        self.outputs.insert(screen_id.to_string(), output);
        self.output_errors.remove(screen_id);
        Ok(())
    }

    pub fn stop(&mut self, screen_id: &str) {
        if let Some(source_screen_id) = self.key_fill_pair_members.get(screen_id).cloned() {
            let key_screen_id = self
                .key_fill_pairs
                .get(&source_screen_id)
                .map(|pair| pair.key_screen_id.clone());
            self.stop_output(&source_screen_id);
            if let Some(ref key_screen_id) = key_screen_id {
                self.stop_output(&key_screen_id);
            }
            self.key_fill_pair_states.remove(&source_screen_id);
            self.output_errors.remove(&source_screen_id);
            if let Some(key_screen_id) = key_screen_id {
                self.output_errors.remove(&key_screen_id);
            }
            return;
        }
        self.stop_output(screen_id);
    }

    fn stop_output(&mut self, screen_id: &str) {
        if let Some(output) = self.outputs.remove(screen_id) {
            if let Some(runtime) = self.runtime.as_ref() {
                runtime.destroy_sender(output.sender);
            }
        }
        self.output_errors.remove(screen_id);
    }

    pub fn stop_all(&mut self) {
        let ids: Vec<String> = self.outputs.keys().cloned().collect();
        for id in ids {
            self.stop_output(&id);
        }
        self.key_fill_pair_states.clear();
    }

    pub(crate) fn active_output_ids(&self) -> Vec<String> {
        self.outputs.keys().cloned().collect()
    }

    pub(crate) fn audio_available(&self) -> bool {
        self.runtime
            .as_ref()
            .is_some_and(|runtime| runtime.send_audio_v3.is_some())
    }

    pub(crate) fn reset_audio_source(&mut self, screen_id: &str) {
        let Some(output) = self.outputs.get_mut(screen_id) else {
            return;
        };
        output.audio_processor = NdiAudioPcmProcessor::from_audio_settings(&output.last_content);
        output.audio_timecode = None;
        output.audio_timecode_remainder = 0;
    }

    pub fn update_output_content(&mut self, outputs: &Value) -> Result<(), String> {
        if self.frozen {
            return Ok(());
        }
        let Some(outputs) = outputs.as_object() else {
            return Err("Output content must be an object.".to_string());
        };
        if self.outputs.is_empty() {
            return Ok(());
        }
        let runtime = self
            .runtime
            .clone()
            .ok_or_else(|| "NDI sender is unavailable.".to_string())?;
        let now = Instant::now();
        let changed_ids: Vec<String> = outputs
            .keys()
            .filter(|screen_id| self.outputs.contains_key(*screen_id))
            .cloned()
            .collect();
        for screen_id in &changed_ids {
            let Some(content) = outputs.get(screen_id) else {
                continue;
            };
            let Some(output) = self.outputs.get_mut(screen_id) else {
                continue;
            };
            output.update_audio_settings(content);
            output.last_content = content.clone();
        }

        let mut pair_sources = HashSet::new();
        for screen_id in changed_ids {
            if let Some(source_screen_id) = self.key_fill_pair_members.get(&screen_id) {
                pair_sources.insert(source_screen_id.clone());
                continue;
            }
            let output = self
                .outputs
                .get_mut(&screen_id)
                .ok_or_else(|| format!("NDI output '{screen_id}' is not active."))?;
            if output.last_content.get("keyFillSourceId").is_some()
                || output.last_content.get("keyFillPartnerId").is_some()
            {
                let error = format!(
                    "NDI screen '{screen_id}' has key/fill scene metadata without an active paired sender."
                );
                self.output_errors.insert(screen_id.clone(), error.clone());
                return Err(error);
            }
            let frame = render_ndi_frame(
                &mut self.compositor,
                &screen_id,
                &output.last_content,
                &output.config,
            )?;
            output.accept_frame(frame, Instant::now())?;
            output.timecode = next_timecode(output.timecode, 0);
            send_frame_with_runtime(&runtime, output)?;
            output.frames_sent = output.frames_sent.saturating_add(1);
            output.next_frame_at = next_frame_deadline(output.config.frame_interval(), now);
        }
        for source_screen_id in pair_sources {
            let pair = self
                .key_fill_pairs
                .get(&source_screen_id)
                .cloned()
                .ok_or_else(|| "Paired NDI screen configuration is unavailable.".to_string())?;
            match self.update_key_fill_pair(&runtime, &pair, now) {
                Ok(_) => {
                    self.output_errors.remove(&pair.source_screen_id);
                    self.output_errors.remove(&pair.key_screen_id);
                }
                Err(error) => {
                    self.output_errors
                        .insert(pair.source_screen_id.clone(), error.clone());
                    self.output_errors
                        .insert(pair.key_screen_id.clone(), error.clone());
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn update_key_fill_pair(
        &mut self,
        runtime: &NdiRuntime,
        pair: &NdiKeyFillPair,
        now: Instant,
    ) -> Result<bool, String> {
        let (source_content, key_content, source_timecode, key_timecode) = {
            let source = self
                .outputs
                .get(&pair.source_screen_id)
                .ok_or_else(|| "Paired NDI fill sender is not active.".to_string())?;
            let key = self
                .outputs
                .get(&pair.key_screen_id)
                .ok_or_else(|| "Paired NDI key sender is not active.".to_string())?;
            if source.config.width != pair.width
                || source.config.height != pair.height
                || key.config.width != pair.width
                || key.config.height != pair.height
                || source.config.frame_rate_n != pair.frame_rate_n
                || source.config.frame_rate_d != pair.frame_rate_d
                || key.config.frame_rate_n != pair.frame_rate_n
                || key.config.frame_rate_d != pair.frame_rate_d
                || source.config.video_delay != pair.video_delay
                || key.config.video_delay != pair.video_delay
            {
                return Err(
                    "Active paired NDI senders have mismatched dimensions, rates or delay."
                        .to_string(),
                );
            }
            (
                source.last_content.clone(),
                key.last_content.clone(),
                source.timecode,
                key.timecode,
            )
        };
        let Some(frames) =
            render_ndi_key_fill_pair(&mut self.compositor, pair, &source_content, &key_content)?
        else {
            return Ok(false);
        };
        let fill_signal: crate::key_fill::KeyFillSignal = serde_json::from_value(
            source_content
                .get("keyFillSignal")
                .cloned()
                .ok_or_else(|| "Paired NDI fill scene has no signal mode.".to_string())?,
        )
        .map_err(|error| format!("Paired NDI fill signal is invalid: {error}"))?;
        if fill_signal != pair.fill_signal {
            return Err(
                "Paired NDI fill scene mode does not match the screen configuration.".into(),
            );
        }
        let state = self
            .key_fill_pair_states
            .get_mut(&pair.source_screen_id)
            .ok_or_else(|| "Paired NDI frame clock is unavailable.".to_string())?;
        if state.pair != *pair {
            return Err("Paired NDI frame clock has stale screen configuration.".to_string());
        }
        if state.fill_signal != Some(fill_signal) {
            return Err("Paired NDI frame clock has stale fill mode.".into());
        }
        state.generation = state.generation.saturating_add(1);
        let frames = state
            .delay
            .push(frames, pair.video_delay, pair.frame_interval(), now)?;
        state.dynamic_media = frames.0.dynamic_media || frames.1.dynamic_media;
        let timecode = next_timecode(source_timecode.max(key_timecode).max(state.timecode), 0);
        let next_frame_at = next_frame_deadline(pair.frame_interval(), now);
        state.timecode = timecode;
        state.next_frame_at = next_frame_at;
        send_key_fill_pair_with_runtime(
            runtime,
            &mut self.outputs,
            pair,
            frames,
            timecode,
            state.generation,
            next_frame_at,
        )?;
        Ok(true)
    }

    /// Submit one raw mixed interleaved PCM block to an active NDI sender. The
    /// block is routed through the output's `sdiNdi` settings, converted to
    /// NDI's planar FLTp ABI, and uses the
    /// sender's current video timecode for the first block. Subsequent blocks
    /// use a sample-accurate 10 MHz audio clock, keeping an eventual native
    /// audio tap aligned with the video stream without rounding each block to
    /// a video frame.
    pub(crate) fn send_audio_pcm(
        &mut self,
        screen_id: &str,
        samples: &[f32],
        sample_rate: u32,
        channels: usize,
    ) -> Result<(), String> {
        if channels == 0 {
            return Err("NDI audio channel count must be greater than zero.".to_string());
        }
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| "NDI sender is unavailable.".to_string())?;
        if runtime.send_audio_v3.is_none() {
            return Err("NDI Runtime does not expose send_send_audio_v3.".to_string());
        }
        let output = self
            .outputs
            .get_mut(screen_id)
            .ok_or_else(|| format!("NDI output '{screen_id}' is not active."))?;
        let Some((output_channels, processed)) =
            output
                .audio_processor
                .process_interleaved(samples, channels, sample_rate)?
        else {
            return Ok(());
        };
        let timecode = output.audio_timecode.unwrap_or(output.timecode);
        let mut frame =
            NdiAudioFrame::from_interleaved(&processed, sample_rate, output_channels, timecode)?;
        send_audio_with_runtime(runtime, output.sender, &mut frame)?;
        output.audio_timecode = Some(advance_audio_timecode(
            timecode,
            processed.len() / output_channels,
            sample_rate,
            &mut output.audio_timecode_remainder,
        ));
        output.audio_frames_sent = output.audio_frames_sent.saturating_add(1);
        Ok(())
    }

    /// Send frames that are due for each active output.
    ///
    /// NDI receivers expect a continuous video clock, even when the scene is
    /// static. `update_output_content` composes and sends immediately; this
    /// method reuses the already-composed pixel buffer at the configured frame
    /// cadence. The Tauri host calls it from a lightweight ticker.
    pub fn tick(
        &mut self,
        now: Instant,
        mut canonical: impl FnMut(&str, &Value) -> Result<Value, String>,
    ) -> Result<usize, String> {
        let Some(runtime) = self.runtime.clone() else {
            return Ok(0);
        };
        let mut sent = 0;
        let pairs: Vec<NdiKeyFillPair> = self.key_fill_pairs.values().cloned().collect();
        for pair in pairs {
            match self.tick_key_fill_pair(&runtime, &pair, now, &mut canonical) {
                Ok(true) => sent += 2,
                Ok(false) => {}
                Err(error) => {
                    self.output_errors
                        .insert(pair.source_screen_id.clone(), error.clone());
                    self.output_errors
                        .insert(pair.key_screen_id.clone(), error.clone());
                    return Err(error);
                }
            }
        }
        for (screen_id, output) in self.outputs.iter_mut() {
            if self.key_fill_pair_members.contains_key(screen_id) {
                continue;
            }
            if now < output.next_frame_at {
                continue;
            }
            if output.dynamic_media && !self.frozen {
                let content = canonical(screen_id, &output.last_content)?;
                let frame =
                    render_ndi_frame(&mut self.compositor, screen_id, &content, &output.config)?;
                output.accept_frame(frame, now)?;
                output.last_content = content;
            }
            output.timecode = next_timecode(output.timecode, 0);
            send_frame_with_runtime(&runtime, output)?;
            output.frames_sent = output.frames_sent.saturating_add(1);
            output.next_frame_at = next_frame_deadline(output.config.frame_interval(), now);
            sent += 1;
        }
        Ok(sent)
    }

    fn tick_key_fill_pair(
        &mut self,
        runtime: &NdiRuntime,
        pair: &NdiKeyFillPair,
        now: Instant,
        canonical: &mut impl FnMut(&str, &Value) -> Result<Value, String>,
    ) -> Result<bool, String> {
        let Some(state) = self.key_fill_pair_states.get(&pair.source_screen_id) else {
            return Ok(false);
        };
        if now < state.next_frame_at {
            return Ok(false);
        }
        let (source_content, key_content, dynamic_media, source_timecode, key_timecode) = {
            let source = self
                .outputs
                .get(&pair.source_screen_id)
                .ok_or_else(|| "Paired NDI fill sender is not active.".to_string())?;
            let key = self
                .outputs
                .get(&pair.key_screen_id)
                .ok_or_else(|| "Paired NDI key sender is not active.".to_string())?;
            if source
                .last_content
                .as_object()
                .is_some_and(|content| content.is_empty())
                || key
                    .last_content
                    .as_object()
                    .is_some_and(|content| content.is_empty())
            {
                return Ok(false);
            }
            (
                source.last_content.clone(),
                key.last_content.clone(),
                state.dynamic_media,
                source.timecode,
                key.timecode,
            )
        };
        validate_key_fill_pair_content(pair, &source_content, &key_content)?;
        let fill_signal: crate::key_fill::KeyFillSignal = serde_json::from_value(
            source_content
                .get("keyFillSignal")
                .cloned()
                .ok_or_else(|| "Paired NDI fill scene has no signal mode.".to_string())?,
        )
        .map_err(|error| format!("Paired NDI fill signal is invalid: {error}"))?;
        if fill_signal != pair.fill_signal {
            return Err(
                "Paired NDI fill scene mode does not match the screen configuration.".into(),
            );
        }
        if dynamic_media && !self.frozen {
            let content = canonical(&pair.source_screen_id, &source_content)?;
            self.outputs
                .get_mut(&pair.source_screen_id)
                .ok_or_else(|| "Paired NDI fill sender is not active.".to_string())?
                .last_content = content;
            return self.update_key_fill_pair(runtime, pair, now);
        }

        let (fill, key) = {
            let source = self
                .outputs
                .get(&pair.source_screen_id)
                .ok_or_else(|| "Paired NDI fill sender is not active.".to_string())?;
            let key = self
                .outputs
                .get(&pair.key_screen_id)
                .ok_or_else(|| "Paired NDI key sender is not active.".to_string())?;
            (
                native_frame_from_output(source),
                native_frame_from_output(key),
            )
        };
        let state = self
            .key_fill_pair_states
            .get_mut(&pair.source_screen_id)
            .ok_or_else(|| "Paired NDI frame clock is unavailable.".to_string())?;
        if state.pair != *pair || state.fill_signal != Some(fill_signal) {
            return Err("Paired NDI frame clock has stale screen configuration.".to_string());
        }
        state.generation = state.generation.saturating_add(1);
        let timecode = next_timecode(source_timecode.max(key_timecode).max(state.timecode), 0);
        let next_frame_at = next_frame_deadline(pair.frame_interval(), now);
        state.timecode = timecode;
        state.next_frame_at = next_frame_at;
        send_key_fill_pair_with_runtime(
            runtime,
            &mut self.outputs,
            pair,
            (fill, key),
            timecode,
            state.generation,
            next_frame_at,
        )?;
        Ok(true)
    }

    fn send_frame(&self, output: &mut NdiOutput) -> Result<(), String> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| "NDI sender is unavailable.".to_string())?;
        send_frame_with_runtime(runtime, output)?;
        output.frames_sent = output.frames_sent.saturating_add(1);
        output.next_frame_at = next_frame_deadline(output.config.frame_interval(), Instant::now());
        Ok(())
    }
}

fn next_frame_deadline(interval: Duration, now: Instant) -> Instant {
    now.checked_add(interval).unwrap_or(now)
}

fn next_timecode(previous: i64, requested: i64) -> i64 {
    let candidate = if requested == 0 {
        current_timecode()
    } else {
        requested
    };
    if candidate <= previous {
        previous.saturating_add(1).max(1)
    } else {
        candidate
    }
}

/// Advance an NDI timestamp by a PCM block using NDI's 100 ns timecode unit.
///
/// Most sample rates do not divide 10 MHz evenly. Keeping the fractional
/// remainder between blocks prevents a long-running stream from drifting by
/// one or more timestamp units per block.
fn advance_audio_timecode(
    current: i64,
    sample_count: usize,
    sample_rate: u32,
    remainder: &mut u64,
) -> i64 {
    if sample_count == 0 || sample_rate == 0 {
        return current;
    }
    let numerator = sample_count as u128 * 10_000_000u128 + *remainder as u128;
    let divisor = sample_rate as u128;
    let delta = numerator / divisor;
    *remainder = (numerator % divisor) as u64;
    current.saturating_add(delta.min(i64::MAX as u128) as i64)
}

fn current_timecode() -> i64 {
    let units = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() / 100)
        .unwrap_or(1);
    units.min(i64::MAX as u128) as i64
}

impl NdiConfig {
    fn frame_interval(&self) -> Duration {
        let seconds = self.frame_rate_d as f64 / self.frame_rate_n.max(1) as f64;
        Duration::from_secs_f64(seconds.max(1.0 / 240.0))
    }
}

/// Owned planar floating-point audio packet suitable for
/// `NDIlib_audio_frame_v3_t`.
///
/// The packet owns all memory referenced by the ABI struct, so callers can
/// safely submit it synchronously and then drop it. NDI's v3 API requires
/// planar `FLTp` samples rather than the interleaved samples emitted by
/// rodio/cpal.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NdiAudioFrame {
    sample_rate: i32,
    channels: i32,
    samples_per_channel: i32,
    timecode: i64,
    data: Vec<f32>,
}

impl NdiAudioFrame {
    pub(crate) fn from_interleaved(
        samples: &[f32],
        sample_rate: u32,
        channels: usize,
        timecode: i64,
    ) -> Result<Self, String> {
        if sample_rate == 0 {
            return Err("NDI audio sample rate must be greater than zero.".to_string());
        }
        if channels == 0 {
            return Err("NDI audio channel count must be greater than zero.".to_string());
        }
        if samples.is_empty() {
            return Err("NDI audio frames cannot be empty.".to_string());
        }
        if !samples.len().is_multiple_of(channels) {
            return Err("NDI audio samples must contain complete frames.".to_string());
        }
        let samples_per_channel = samples.len() / channels;
        let sample_rate = i32::try_from(sample_rate)
            .map_err(|_| "NDI audio sample rate exceeds the ABI range.".to_string())?;
        let channels = i32::try_from(channels)
            .map_err(|_| "NDI audio channel count exceeds the ABI range.".to_string())?;
        let samples_per_channel = i32::try_from(samples_per_channel)
            .map_err(|_| "NDI audio frame is too large for the ABI range.".to_string())?;
        let mut data = vec![0.0; samples.len()];
        for (frame_index, frame) in samples.chunks_exact(channels as usize).enumerate() {
            for (channel_index, sample) in frame.iter().enumerate() {
                data[channel_index * samples_per_channel as usize + frame_index] = *sample;
            }
        }
        Ok(Self {
            sample_rate,
            channels,
            samples_per_channel,
            timecode,
            data,
        })
    }

    fn as_ffi(&mut self) -> NDIlibAudioFrameV3 {
        NDIlibAudioFrameV3 {
            sample_rate: self.sample_rate,
            no_channels: self.channels,
            no_samples: self.samples_per_channel,
            timecode: self.timecode,
            four_cc: FLTP_FOURCC,
            p_data: self.data.as_mut_ptr().cast(),
            channel_stride_in_bytes: (self.samples_per_channel as usize
                * std::mem::size_of::<f32>()) as c_int,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        }
    }
}

/// PCM processor for the NDI audio settings section.
///
/// This is deliberately independent from the OS output sink: it provides a
/// deterministic, testable bridge from either a native playback tap or the
/// bounded ticker-owned fallback decoder to NDI.
/// It applies the same native routing, gain, tone, and delay semantics while
/// keeping the delayed samples across successive blocks.
#[derive(Debug)]
pub(crate) struct NdiAudioPcmProcessor {
    section: Value,
    enabled: bool,
    gain: f32,
    delay_milliseconds: f64,
    frame_position: u64,
    output_channels: Option<usize>,
    delay_queue: VecDeque<f32>,
}

impl NdiAudioPcmProcessor {
    pub(crate) fn from_audio_settings(value: &Value) -> Self {
        let audio_settings = value.get("audioSettings").unwrap_or(value);
        let normalized = normalize_audio_settings_value(audio_settings);
        let section = normalized
            .get("sdiNdi")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let enabled = section
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let level = section
            .get("level")
            .and_then(json_number)
            .unwrap_or(0.0)
            .clamp(-60.0, 6.0);
        let delay_milliseconds = section
            .get("delay")
            .and_then(json_number)
            .unwrap_or(0.0)
            .max(0.0);
        Self {
            section,
            enabled,
            gain: 10.0_f64.powf(level / 20.0).clamp(0.0, 2.0) as f32,
            delay_milliseconds,
            frame_position: 0,
            output_channels: None,
            delay_queue: VecDeque::new(),
        }
    }

    pub(crate) fn process_interleaved(
        &mut self,
        samples: &[f32],
        input_channels: usize,
        sample_rate: u32,
    ) -> Result<Option<(usize, Vec<f32>)>, String> {
        if !self.enabled {
            return Ok(None);
        }
        let (output_channels, mut routed) = crate::native_audio::route_interleaved_audio(
            &self.section,
            samples,
            input_channels,
            sample_rate,
            self.frame_position,
        )?;
        self.frame_position = self
            .frame_position
            .saturating_add((samples.len() / input_channels) as u64);
        let output_channels = output_channels as usize;
        if self.output_channels != Some(output_channels) {
            self.output_channels = Some(output_channels);
            self.delay_queue.clear();
        }
        for sample in &mut routed {
            *sample *= self.gain;
        }
        let delay_samples = (self.delay_milliseconds * sample_rate as f64 / 1_000.0)
            .round()
            .max(0.0) as usize;
        if self.delay_queue.is_empty() && delay_samples > 0 {
            self.delay_queue
                .resize(delay_samples.saturating_mul(output_channels), 0.0);
        }
        self.delay_queue.extend(routed);
        let output_len = samples.len() / input_channels * output_channels;
        let mut delayed = Vec::with_capacity(output_len);
        for _ in 0..output_len {
            delayed.push(self.delay_queue.pop_front().unwrap_or(0.0));
        }
        Ok(Some((output_channels, delayed)))
    }
}

fn json_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn send_frame_with_runtime(runtime: &NdiRuntime, output: &NdiOutput) -> Result<(), String> {
    let pixels = output.config.alpha_mode.pixels(&output.pixels);
    let frame = NDIlibVideoFrameV2 {
        xres: output.config.width as c_int,
        yres: output.config.height as c_int,
        four_cc: BGRA_FOURCC,
        frame_rate_n: output.config.frame_rate_n,
        frame_rate_d: output.config.frame_rate_d,
        picture_aspect_ratio: output.config.width as f32 / output.config.height as f32,
        frame_format_type: PROGRESSIVE_FRAME,
        timecode: output.timecode,
        p_data: pixels.as_ptr(),
        line_stride_in_bytes: (output.config.width * 4) as c_int,
        p_metadata: std::ptr::null(),
        timestamp: output.timestamp,
    };
    unsafe { (runtime.send_video_v2)(output.sender, &frame) };
    Ok(())
}

fn send_key_fill_pair_with_runtime(
    runtime: &NdiRuntime,
    outputs: &mut HashMap<String, NdiOutput>,
    pair: &NdiKeyFillPair,
    frames: (
        crate::native_compositor::NativeFrame,
        crate::native_compositor::NativeFrame,
    ),
    timecode: i64,
    generation: u64,
    next_frame_at: Instant,
) -> Result<(), String> {
    if frames.0.width != pair.width
        || frames.0.height != pair.height
        || frames.1.width != pair.width
        || frames.1.height != pair.height
    {
        return Err("Paired NDI frame dimensions do not match the configured outputs.".into());
    }
    for screen_id in [&pair.source_screen_id, &pair.key_screen_id] {
        let output = outputs
            .get(screen_id)
            .ok_or_else(|| format!("Paired NDI output '{screen_id}' is not active."))?;
        if output.config.width != pair.width || output.config.height != pair.height {
            return Err(format!(
                "Paired NDI output '{screen_id}' has mismatched frame dimensions."
            ));
        }
    }
    for (screen_id, frame) in [
        (pair.source_screen_id.as_str(), frames.0),
        (pair.key_screen_id.as_str(), frames.1),
    ] {
        let output = outputs
            .get_mut(screen_id)
            .ok_or_else(|| format!("Paired NDI output '{screen_id}' is not active."))?;
        output.video_delay.clear();
        output.pixels = frame.pixels;
        output.dynamic_media = frame.dynamic_media;
        output.timecode = timecode;
        output.timestamp = timecode;
        output.pair_generation = Some(generation);
        output.next_frame_at = next_frame_at;
        send_frame_with_runtime(runtime, output)?;
        output.frames_sent = output.frames_sent.saturating_add(1);
    }
    Ok(())
}

fn native_frame_from_output(output: &NdiOutput) -> crate::native_compositor::NativeFrame {
    crate::native_compositor::NativeFrame {
        width: output.config.width,
        height: output.config.height,
        pixels: output.pixels.clone(),
        dynamic_media: output.dynamic_media,
    }
}

fn send_audio_with_runtime(
    runtime: &NdiRuntime,
    sender: *mut c_void,
    frame: &mut NdiAudioFrame,
) -> Result<(), String> {
    let send_audio = runtime
        .send_audio_v3
        .ok_or_else(|| "NDI Runtime does not expose send_send_audio_v3.".to_string())?;
    let frame = frame.as_ffi();
    unsafe { send_audio(sender, &frame) };
    Ok(())
}

impl Default for NdiService {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for NdiService {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// Values used while constructing the runtime search list.
///
/// Keeping this separate from `std::env` makes candidate ordering testable on
/// every host platform. `NDI_RUNTIME_PATH` and `NDI_LIB_PATH` are explicit
/// library overrides (or lists of overrides); `NDI_RUNTIME_DIR` and `NDI_ROOT`
/// are directory overrides. Overrides are tried first, then the conventional
/// installation locations for the current platform.
#[derive(Debug, Clone, Default)]
struct RuntimeEnvironment {
    library_overrides: Vec<PathBuf>,
    runtime_dirs: Vec<PathBuf>,
    roots: Vec<PathBuf>,
    program_w6432: Option<PathBuf>,
    program_files: Option<PathBuf>,
    program_files_x86: Option<PathBuf>,
    home: Option<PathBuf>,
}

impl RuntimeEnvironment {
    fn from_process() -> Self {
        Self {
            library_overrides: read_env_path_list("NDI_RUNTIME_PATH")
                .into_iter()
                .chain(read_env_path_list("NDI_LIB_PATH"))
                .collect(),
            runtime_dirs: read_env_path_list("NDI_RUNTIME_DIR"),
            roots: read_env_path_list("NDI_ROOT"),
            program_w6432: std::env::var_os("ProgramW6432").map(PathBuf::from),
            program_files: std::env::var_os("ProgramFiles").map(PathBuf::from),
            program_files_x86: std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
            home: std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from),
        }
    }
}

fn read_env_path_list(name: &str) -> Vec<PathBuf> {
    std::env::var_os(name)
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
}

fn runtime_candidates() -> Vec<PathBuf> {
    let environment = RuntimeEnvironment::from_process();
    runtime_candidates_for(std::env::consts::OS, std::env::consts::ARCH, &environment)
}

fn runtime_candidates_for(
    target_os: &str,
    architecture: &str,
    environment: &RuntimeEnvironment,
) -> Vec<PathBuf> {
    let names = runtime_library_names(target_os, architecture);
    let mut candidates = Vec::new();

    for path in &environment.library_overrides {
        add_library_override(&mut candidates, path, target_os, &names);
    }
    for path in &environment.runtime_dirs {
        add_library_directory(&mut candidates, path, &names);
    }
    for root in &environment.roots {
        add_runtime_root(&mut candidates, root, target_os, architecture, &names);
    }

    match target_os {
        "windows" => {
            let roots = [
                environment.program_w6432.as_ref(),
                environment.program_files.as_ref(),
                environment.program_files_x86.as_ref(),
            ];
            for root in roots.into_iter().flatten() {
                for relative in [
                    "NDI/NDI 6 Runtime/v6",
                    "NDI/NDI 5 Runtime/v5",
                    "NewTek/NDI 4 Runtime/v4",
                    "NewTek/NDI 3 Runtime/v3",
                    "NDI/NDI 6 Runtime",
                    "NDI/NDI 5 Runtime",
                ] {
                    add_library_directory(&mut candidates, &root.join(relative), &names);
                }
            }
        }
        "macos" => {
            for directory in [
                PathBuf::from("/Library/NDI SDK for Apple/lib/macOS"),
                PathBuf::from("/Library/NDI 6 Runtime/lib/macOS"),
                PathBuf::from("/Library/NDI 5 Runtime/lib/macOS"),
                PathBuf::from("/usr/local/lib"),
                PathBuf::from("/opt/homebrew/lib"),
                PathBuf::from("/usr/lib"),
            ] {
                add_library_directory(&mut candidates, &directory, &names);
            }
            if let Some(home) = environment.home.as_ref() {
                for relative in [
                    "Library/NDI SDK for Apple/lib/macOS",
                    "Library/NDI 6 Runtime/lib/macOS",
                    "Library/NDI 5 Runtime/lib/macOS",
                    "lib",
                ] {
                    add_library_directory(&mut candidates, &home.join(relative), &names);
                }
            }
        }
        "linux" => {
            let architecture_directory = linux_architecture_directory(architecture);
            for root in [
                PathBuf::from("/usr/lib"),
                PathBuf::from("/usr/local/lib"),
                PathBuf::from("/lib"),
                PathBuf::from("/lib64"),
            ] {
                add_library_directory(&mut candidates, &root, &names);
                add_library_directory(&mut candidates, &root.join(architecture_directory), &names);
            }
            for relative in [
                "NDI SDK for Linux/lib",
                "NDI 6 Runtime/v6/lib",
                "NDI 5 Runtime/v5/lib",
            ] {
                let root = PathBuf::from("/opt").join(relative);
                add_library_directory(&mut candidates, &root, &names);
                add_library_directory(&mut candidates, &root.join(architecture_directory), &names);
            }
        }
        _ => {}
    }

    // A bare soname lets the platform loader use its normal search rules,
    // including LD_LIBRARY_PATH/DYLD_LIBRARY_PATH and application rpaths.
    for name in names {
        push_unique(&mut candidates, PathBuf::from(name));
    }
    candidates
}

fn runtime_library_names(target_os: &str, architecture: &str) -> Vec<&'static str> {
    match target_os {
        "windows" if matches!(architecture, "x86" | "i686") => {
            vec!["Processing.NDI.Lib.x86.dll", "Processing.NDI.Lib.dll"]
        }
        "windows" if matches!(architecture, "aarch64" | "arm64") => {
            vec!["Processing.NDI.Lib.arm64.dll", "Processing.NDI.Lib.x64.dll"]
        }
        "windows" => vec!["Processing.NDI.Lib.x64.dll", "Processing.NDI.Lib.dll"],
        "macos" => vec!["libndi.dylib", "libndi.6.dylib", "libndi.5.dylib"],
        "linux" => vec!["libndi.so.6", "libndi.so", "libndi.so.5"],
        _ => vec!["libndi"],
    }
}

fn linux_architecture_directory(architecture: &str) -> &'static str {
    match architecture {
        "x86_64" => "x86_64-linux-gnu",
        "aarch64" => "aarch64-linux-gnu",
        "arm" | "armv7" => "arm-linux-gnueabihf",
        "x86" | "i686" => "i386-linux-gnu",
        _ => "",
    }
}

fn add_library_override(
    candidates: &mut Vec<PathBuf>,
    path: &Path,
    target_os: &str,
    names: &[&'static str],
) {
    if looks_like_library_path(path, target_os) {
        push_unique(candidates, path.to_path_buf());
    } else {
        add_library_directory(candidates, path, names);
    }
}

fn looks_like_library_path(path: &Path, target_os: &str) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match target_os {
        "windows" => name.ends_with(".dll"),
        "macos" => name.contains(".dylib"),
        "linux" => name.contains(".so"),
        _ => path.extension().is_some(),
    }
}

fn add_library_directory(candidates: &mut Vec<PathBuf>, directory: &Path, names: &[&'static str]) {
    for name in names {
        push_unique(candidates, directory.join(name));
    }
}

fn add_runtime_root(
    candidates: &mut Vec<PathBuf>,
    root: &Path,
    target_os: &str,
    architecture: &str,
    names: &[&'static str],
) {
    match target_os {
        "windows" => {
            for relative in [
                "NDI/NDI 6 Runtime/v6",
                "NDI/NDI 5 Runtime/v5",
                "NewTek/NDI 4 Runtime/v4",
                "NewTek/NDI 3 Runtime/v3",
                "v6",
                "v5",
                "bin",
            ] {
                add_library_directory(candidates, &root.join(relative), names);
            }
        }
        "macos" => {
            for relative in ["lib/macOS", "lib", "Frameworks"] {
                add_library_directory(candidates, &root.join(relative), names);
            }
        }
        "linux" => {
            let architecture_directory = linux_architecture_directory(architecture);
            for relative in ["lib", "lib64"] {
                let directory = root.join(relative);
                add_library_directory(candidates, &directory, names);
                add_library_directory(candidates, &directory.join(architecture_directory), names);
            }
            add_library_directory(candidates, root, names);
        }
        _ => add_library_directory(candidates, root, names),
    }
}

fn push_unique(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(handle: *mut c_void, name: *const c_char) -> *mut c_void;
        fn FreeLibrary(handle: *mut c_void) -> i32;
    }

    pub unsafe fn open(path: &Path) -> Result<*mut c_void, String> {
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let handle = LoadLibraryW(wide.as_ptr());
        if handle.is_null() {
            Err(format!(
                "LoadLibraryW failed for {}: {}.",
                path.display(),
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(handle)
        }
    }

    pub unsafe fn symbol(handle: *mut c_void, name: &[u8]) -> Result<*mut c_void, String> {
        let pointer = GetProcAddress(handle, name.as_ptr().cast());
        if pointer.is_null() {
            Err(format!(
                "NDI symbol {} is missing: {}.",
                symbol_name(name),
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(pointer)
        }
    }

    pub unsafe fn close(handle: *mut c_void) {
        if !handle.is_null() {
            let _ = FreeLibrary(handle);
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::*;
    use std::ffi::CString;

    const RTLD_NOW: c_int = 0x2;
    const RTLD_LOCAL: c_int = 0x4;

    #[cfg_attr(target_os = "linux", link(name = "dl"))]
    extern "C" {
        fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
        fn dlerror() -> *const c_char;
    }

    pub unsafe fn open(path: &Path) -> Result<*mut c_void, String> {
        let name = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| "NDI library path contains a NUL byte.".to_string())?;
        // RTLD_NOW reports missing transitive dependencies while loading and
        // RTLD_LOCAL prevents NDI symbols from leaking into the application.
        let handle = dlopen(name.as_ptr(), RTLD_NOW | RTLD_LOCAL);
        if handle.is_null() {
            Err(format!(
                "dlopen failed for {}: {}.",
                path.display(),
                loader_error()
            ))
        } else {
            Ok(handle)
        }
    }

    pub unsafe fn symbol(handle: *mut c_void, name: &[u8]) -> Result<*mut c_void, String> {
        // Clear any stale error left by an earlier dlsym call before looking
        // up the next symbol.
        let _ = dlerror();
        let pointer = dlsym(handle, name.as_ptr().cast());
        if pointer.is_null() {
            Err(format!(
                "NDI symbol {} is missing: {}.",
                symbol_name(name),
                loader_error()
            ))
        } else {
            Ok(pointer)
        }
    }

    pub unsafe fn close(handle: *mut c_void) {
        if !handle.is_null() {
            let _ = dlclose(handle);
        }
    }

    unsafe fn loader_error() -> String {
        let pointer = dlerror();
        if pointer.is_null() {
            "unknown dynamic loader error".to_string()
        } else {
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        }
    }
}

fn symbol_name(name: &[u8]) -> String {
    CStr::from_bytes_until_nul(name)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(name).into_owned())
}
