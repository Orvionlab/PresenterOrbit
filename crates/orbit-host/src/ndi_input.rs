use crate::ndi::{shared_runtime, NdiRuntime};
use crate::ndi_find_options::FindOptions;
#[path = "ndi_input_audio.rs"]
mod audio;
pub(crate) use audio::AudioReceiver;
use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ffi::{c_char, c_void, CStr, CString},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

/// `NDIlib_frame_type_error`; the SDK reports a failed connection with 4, not -1.
const FRAME_ERROR: i32 = 4;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NdiSource {
    pub name: String,
    pub url_address: String,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Source {
    name: *const c_char,
    url: *const c_char,
}
#[repr(C)]
struct FindCreate {
    local: bool,
    groups: *const c_char,
    extra_ips: *const c_char,
}
#[repr(C)]
struct RecvCreate {
    source: Source,
    color: i32,
    bandwidth: i32,
    fields: bool,
    name: *const c_char,
}
#[repr(C)]
struct Video {
    width: i32,
    height: i32,
    fourcc: u32,
    rate_n: i32,
    rate_d: i32,
    aspect: f32,
    format: i32,
    timecode: i64,
    data: *const u8,
    stride: i32,
    metadata: *const c_char,
    timestamp: i64,
}
type FindCreateFn = unsafe extern "C" fn(*const FindCreate) -> *mut c_void;
type FindDestroyFn = unsafe extern "C" fn(*mut c_void);
type FindWaitFn = unsafe extern "C" fn(*mut c_void, u32) -> bool;
type FindSourcesFn = unsafe extern "C" fn(*mut c_void, *mut u32) -> *const Source;
type RecvCreateFn = unsafe extern "C" fn(*const RecvCreate) -> *mut c_void;
type RecvDestroyFn = unsafe extern "C" fn(*mut c_void);
type CaptureFn =
    unsafe extern "C" fn(*mut c_void, *mut Video, *mut c_void, *mut c_void, u32) -> i32;
type FreeVideoFn = unsafe extern "C" fn(*mut c_void, *const Video);

pub fn discover_with(options: &FindOptions) -> Result<Vec<NdiSource>, String> {
    let runtime = shared_runtime()?;
    let create: FindCreateFn = runtime.symbol(b"NDIlib_find_create_v2\0")?;
    let destroy: FindDestroyFn = runtime.symbol(b"NDIlib_find_destroy\0")?;
    let wait: FindWaitFn = runtime.symbol(b"NDIlib_find_wait_for_sources\0")?;
    let sources: FindSourcesFn = runtime.symbol(b"NDIlib_find_get_current_sources\0")?;
    let text = |value: &str| {
        (!value.is_empty())
            .then(|| CString::new(value).map_err(|error| error.to_string()))
            .transpose()
    };
    let groups = text(&options.groups)?;
    let extra_ips = text(&options.extra_ips)?;
    let pointer = |value: &Option<CString>| value.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
    let finder = unsafe {
        create(&FindCreate {
            local: options.show_local_sources,
            groups: pointer(&groups),
            extra_ips: pointer(&extra_ips),
        })
    };
    if finder.is_null() {
        return Err("NDI discovery could not be started.".into());
    }
    let mut result = Vec::new();
    unsafe {
        wait(finder, 1000);
        let mut count = 0;
        let found = sources(finder, &mut count);
        if !found.is_null() {
            for index in 0..count as usize {
                let source = *found.add(index);
                if !source.name.is_null() && !source.url.is_null() {
                    result.push(NdiSource {
                        name: CStr::from_ptr(source.name).to_string_lossy().into_owned(),
                        url_address: CStr::from_ptr(source.url).to_string_lossy().into_owned(),
                    });
                }
            }
        }
        destroy(finder);
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

struct Receiver {
    _runtime: Arc<NdiRuntime>,
    handle: *mut c_void,
    capture: CaptureFn,
    free: FreeVideoFn,
    destroy: RecvDestroyFn,
    latest: Option<Arc<DynamicImage>>,
    received: Instant,
    used: Instant,
}
// Handles and their frame buffers are accessed only while holding RECEIVERS.
unsafe impl Send for Receiver {}
impl Receiver {
    fn new(source: &NdiSource, bandwidth: i32) -> Result<Self, String> {
        validate_source(source)?;
        let runtime = shared_runtime()?;
        let create: RecvCreateFn = runtime.symbol(b"NDIlib_recv_create_v3\0")?;
        let capture = runtime.symbol(b"NDIlib_recv_capture_v3\0")?;
        let free = runtime.symbol(b"NDIlib_recv_free_video_v2\0")?;
        let destroy = runtime.symbol(b"NDIlib_recv_destroy\0")?;
        let name = CString::new(source.name.as_str()).map_err(|error| error.to_string())?;
        let url = CString::new(source.url_address.as_str()).map_err(|error| error.to_string())?;
        let config = RecvCreate {
            source: Source {
                name: name.as_ptr(),
                url: url.as_ptr(),
            },
            color: 0,
            bandwidth,
            fields: false,
            name: std::ptr::null(),
        };
        let handle = unsafe { create(&config) };
        if handle.is_null() {
            return Err("NDI receiver could not be created.".into());
        }
        Ok(Self {
            _runtime: runtime,
            handle,
            capture,
            free,
            destroy,
            latest: None,
            received: Instant::now(),
            used: Instant::now(),
        })
    }
    fn frame(&mut self) -> Result<Option<Arc<DynamicImage>>, String> {
        self.used = Instant::now();
        for _ in 0..4 {
            let mut video: Video = unsafe { std::mem::zeroed() };
            let kind = unsafe {
                (self.capture)(
                    self.handle,
                    &mut video,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                )
            };
            if kind == 1 {
                let image = unsafe { copy_video(&video) };
                unsafe { (self.free)(self.handle, &video) };
                self.latest = Some(Arc::new(image?));
                self.received = Instant::now();
            } else if kind == FRAME_ERROR {
                return Err("NDI receiver connection failed.".into());
            } else {
                break;
            }
        }
        if self.received.elapsed() > Duration::from_secs(2) {
            self.latest = None;
        }
        Ok(self.latest.clone())
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.handle) };
    }
}

pub fn validate_source(source: &NdiSource) -> Result<(), String> {
    if source.name.trim().is_empty()
        || source.url_address.trim().is_empty()
        || source.name.contains('\0')
        || source.url_address.contains('\0')
    {
        return Err(
            "NDI source name and address are required and cannot contain NUL bytes.".into(),
        );
    }
    Ok(())
}

pub fn frame(source: &NdiSource) -> Result<Option<Arc<DynamicImage>>, String> {
    static RECEIVERS: OnceLock<Mutex<HashMap<String, Receiver>>> = OnceLock::new();
    static REAPER: OnceLock<()> = OnceLock::new();
    REAPER.get_or_init(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(1));
            if let Some(receivers) = RECEIVERS.get() {
                if let Ok(mut receivers) = receivers.lock() {
                    receivers
                        .retain(|_, receiver| receiver.used.elapsed() < Duration::from_secs(5));
                }
            }
        });
    });
    let mut receivers = RECEIVERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "NDI receivers are unavailable.")?;
    receivers.retain(|_, receiver| receiver.used.elapsed() < Duration::from_secs(5));
    let key = serde_json::to_string(source).map_err(|error| error.to_string())?;
    if !receivers.contains_key(&key) {
        if receivers.len() >= 16 {
            return Err("At most 16 NDI sources can be received at once.".into());
        }
        receivers.insert(key.clone(), Receiver::new(source, 100)?);
    }
    receivers.get_mut(&key).unwrap().frame()
}

unsafe fn copy_video(video: &Video) -> Result<DynamicImage, String> {
    if video.width <= 0
        || video.height <= 0
        || video.width > 8192
        || video.height > 8192
        || i64::from(video.width) * i64::from(video.height) > 33_554_432
        || video.data.is_null()
        || i64::from(video.stride) < i64::from(video.width) * 4
    {
        return Err("NDI returned an invalid video frame.".into());
    }
    let bgra = video.fourcc == u32::from_le_bytes(*b"BGRA");
    if !bgra && video.fourcc != u32::from_le_bytes(*b"BGRX") {
        return Err("NDI returned an unsupported pixel format.".into());
    }
    let mut pixels = vec![0; video.width as usize * video.height as usize * 4];
    for y in 0..video.height as usize {
        let row = std::slice::from_raw_parts(
            video.data.add(y * video.stride as usize),
            video.width as usize * 4,
        );
        for (input, output) in row.chunks_exact(4).zip(
            pixels[y * video.width as usize * 4..][..video.width as usize * 4].chunks_exact_mut(4),
        ) {
            output.copy_from_slice(&[
                input[2],
                input[1],
                input[0],
                if bgra { input[3] } else { 255 },
            ]);
        }
    }
    Ok(DynamicImage::ImageRgba8(
        RgbaImage::from_raw(video.width as u32, video.height as u32, pixels).unwrap(),
    ))
}

pub fn preview_packet(source: NdiSource) -> Result<Vec<u8>, String> {
    encode_preview(frame(&source)?)
}

/// `None` means no frame has arrived yet; the renderer shows it as waiting rather than as an error.
pub fn encode_preview(image: Option<Arc<DynamicImage>>) -> Result<Vec<u8>, String> {
    let image = image.ok_or_else(|| "NDI_WAITING_FOR_VIDEO".to_string())?;
    let image = image
        .resize(640, 360, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    crate::native_compositor::NativeFrame {
        width: width as usize,
        height: height as usize,
        pixels,
        dynamic_media: true,
    }
    .encode_raw_rgba()
}
