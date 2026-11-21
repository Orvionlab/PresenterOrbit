use crate::camera_devices::{self, CameraSource};
use image::{DynamicImage, RgbaImage};
use std::{
    collections::HashMap,
    io::Read,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const MAX_CAMERAS: usize = 4;
const IDLE_LIMIT: Duration = Duration::from_secs(5);
const STALE_FRAME: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Shared {
    frame: Option<(Arc<DynamicImage>, Instant)>,
    error: Option<String>,
}

/// One FFmpeg process per device; the reader thread keeps only the newest decoded frame.
struct Camera {
    source: CameraSource,
    child: Child,
    shared: Arc<Mutex<Shared>>,
    used: Instant,
}

impl Camera {
    fn start(source: &CameraSource, input_args: Vec<String>) -> Result<Self, String> {
        camera_devices::validate_source(source)?;
        if !ffmpeg_sidecar::command::ffmpeg_is_installed() {
            return Err(
                "FFmpeg is unavailable; camera capture requires the installed FFmpeg.".into(),
            );
        }
        let mut command = Command::new(ffmpeg_sidecar::paths::ffmpeg_path());
        command
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(&input_args)
            .args(["-an", "-s"])
            .arg(format!("{}x{}", source.width, source.height))
            .args(["-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("Camera capture could not start FFmpeg: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Camera capture output is unavailable.")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Camera capture errors are unavailable.")?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (reader_shared, reader_source) = (shared.clone(), source.clone());
        std::thread::spawn(move || read_frames(stdout, stderr, &reader_source, &reader_shared));
        Ok(Self {
            source: source.clone(),
            child,
            shared,
            used: Instant::now(),
        })
    }

    fn frame(&mut self) -> Result<Option<Arc<DynamicImage>>, String> {
        let shared = self
            .shared
            .lock()
            .map_err(|_| "Camera state is unavailable.")?;
        if let Some(error) = &shared.error {
            // A failed capture is not "used", so it is released and retried after the idle limit.
            return Err(error.clone());
        }
        self.used = Instant::now();
        Ok(shared
            .frame
            .as_ref()
            .filter(|(_, received)| received.elapsed() < STALE_FRAME)
            .map(|(image, _)| image.clone()))
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_frames(
    mut stdout: impl Read,
    stderr: impl Read,
    source: &CameraSource,
    shared: &Mutex<Shared>,
) {
    let (width, height) = (source.width, source.height);
    loop {
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        if stdout.read_exact(&mut pixels).is_err() {
            break;
        }
        let image = RgbaImage::from_raw(width, height, pixels).expect("frame buffer matches size");
        if let Ok(mut shared) = shared.lock() {
            shared.frame = Some((Arc::new(DynamicImage::ImageRgba8(image)), Instant::now()));
        }
    }
    let mut text = Vec::new();
    let _ = stderr.take(8192).read_to_end(&mut text);
    let text = String::from_utf8_lossy(&text);
    let detail: Vec<&str> = text.lines().rev().take(3).collect();
    let detail = detail.into_iter().rev().collect::<Vec<_>>().join(" ");
    let message = format!(
        "Camera \"{}\" stopped delivering video{}",
        source.device_name,
        if detail.trim().is_empty() {
            ".".into()
        } else {
            format!(": {}", detail.trim())
        }
    );
    if let Ok(mut shared) = shared.lock() {
        shared.error = Some(message);
    }
}

static CAMERAS: OnceLock<Mutex<HashMap<String, Camera>>> = OnceLock::new();

pub fn frame(source: &CameraSource) -> Result<Option<Arc<DynamicImage>>, String> {
    frame_with(source, || {
        camera_devices::input_args(camera_devices::host(), source)
    })
}

/// `input_args` is evaluated only when no capture for this device is running yet.
pub(crate) fn frame_with(
    source: &CameraSource,
    input_args: impl FnOnce() -> Vec<String>,
) -> Result<Option<Arc<DynamicImage>>, String> {
    static REAPER: OnceLock<()> = OnceLock::new();
    REAPER.get_or_init(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(1));
            if let Some(Ok(mut cameras)) = CAMERAS.get().map(Mutex::lock) {
                cameras.retain(|_, camera| camera.used.elapsed() < IDLE_LIMIT);
            }
        });
    });
    camera_devices::validate_source(source)?;
    let mut cameras = CAMERAS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "Cameras are unavailable.")?;
    cameras.retain(|_, camera| camera.used.elapsed() < IDLE_LIMIT);
    match cameras.get(&source.device_name) {
        Some(running) if running.source != *source => {
            return Err(format!(
                "Camera \"{}\" is already in use with a different format.",
                source.device_name
            ))
        }
        Some(_) => {}
        None => {
            if cameras.len() >= MAX_CAMERAS {
                return Err(format!(
                    "At most {MAX_CAMERAS} cameras can be captured at once."
                ));
            }
            cameras.insert(
                source.device_name.clone(),
                Camera::start(source, input_args())?,
            );
        }
    }
    cameras.get_mut(&source.device_name).unwrap().frame()
}

/// Stops every capture process; called at app exit.
pub fn shutdown() {
    if let Some(Ok(mut cameras)) = CAMERAS.get().map(Mutex::lock) {
        cameras.clear();
    }
}

pub fn preview_packet(source: CameraSource) -> Result<Vec<u8>, String> {
    crate::ndi_input::encode_preview(frame(&source)?)
}
