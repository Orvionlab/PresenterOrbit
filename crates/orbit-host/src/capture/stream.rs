//! RTMP/RTMPS streaming through the capture pipeline: the same composited frames and screen
//! audio, encoded live to an FLV muxer instead of an MP4 file.
use super::*;
use std::{
    collections::BTreeMap,
    net::{TcpListener, TcpStream},
};

/// Silence is filled this far behind real time so a late audio block is never trimmed.
const AUDIO_LAG: Duration = Duration::from_millis(250);
const KEYFRAME_SECONDS: u32 = 2;
const NETWORK_TIMEOUT_MICROSECONDS: &str = "10000000";
const URL_MESSAGE: &str =
    "The stream URL must be rtmp:// or rtmps:// with a host and no credentials, query or fragment.";

pub fn validate_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| URL_MESSAGE.to_string())?;
    if !matches!(parsed.scheme(), "rtmp" | "rtmps")
        || parsed.host_str().is_none_or(str::is_empty)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(URL_MESSAGE.into());
    }
    Ok(())
}

pub fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("The stream key is required and must not contain whitespace.".into());
    }
    Ok(())
}

/// Keys are secrets: they live in their own app-data file, never in the workspace.
fn read_keys(path: &Path) -> Result<BTreeMap<String, String>, String> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|error| format!("The stream key file is invalid: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn read_key(path: &Path, preset_id: &str) -> Result<Option<String>, String> {
    Ok(read_keys(path)?.remove(preset_id))
}

pub fn write_key(path: &Path, preset_id: &str, key: Option<&str>) -> Result<(), String> {
    let mut keys = read_keys(path)?;
    if key.is_none() && !keys.contains_key(preset_id) {
        return Ok(());
    }
    match key {
        Some(key) => keys.insert(preset_id.to_string(), key.to_string()),
        None => keys.remove(preset_id),
    };
    fs::write(
        path,
        serde_json::to_string(&keys).map_err(|e| e.to_string())?,
    )
    .map_err(|error| error.to_string())
}

pub fn mask(text: &str, key: &str) -> String {
    text.replace(key, "***")
}

fn destination(url: &str, key: &str) -> String {
    format!("{}/{key}", url.trim_end_matches('/'))
}

/// FFmpeg lists the protocols it can write after an `Output:` heading; RTMPS needs a TLS build.
fn supports_output_protocol(listing: &str, scheme: &str) -> bool {
    listing
        .split("Output:")
        .nth(1)
        .is_some_and(|outputs| outputs.split_whitespace().any(|name| name == scheme))
}

fn require_protocol(scheme: &str) -> Result<(), String> {
    let mut command = Command::new(ffmpeg_sidecar::paths::ffmpeg_path());
    command
        .args(["-hide_banner", "-protocols"])
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .output()
        .map_err(|e| format!("Cannot query FFmpeg protocols: {e}"))?;
    if supports_output_protocol(&String::from_utf8_lossy(&output.stdout), scheme) {
        Ok(())
    } else {
        Err(format!(
            "The installed FFmpeg cannot stream over {scheme}://; use an FFmpeg build with that protocol (RTMPS needs TLS)."
        ))
    }
}

fn stream_args(preset: &CapturePreset, audio_port: u16, destination: &str) -> Vec<String> {
    let bitrate = format!("{}k", preset.bitrate_kbps);
    [
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pixel_format",
        "bgra",
        "-video_size",
        &format!("{}x{}", preset.width, preset.height),
        "-framerate",
        &preset.fps.to_string(),
        "-i",
        "pipe:0",
        "-f",
        "f32le",
        "-ar",
        &AUDIO_SAMPLE_RATE.to_string(),
        "-ch_layout",
        discrete_layout(preset.recording_audio_map().len()),
        "-i",
        &format!("tcp://127.0.0.1:{audio_port}"),
        "-map",
        "0:v:0",
        "-map",
        "1:a:0",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-pix_fmt",
        "yuv420p",
        "-b:v",
        &bitrate,
        "-maxrate",
        &bitrate,
        "-bufsize",
        &bitrate,
        "-g",
        &(preset.fps * KEYFRAME_SECONDS).to_string(),
        "-sc_threshold",
        "0",
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        "-rw_timeout",
        NETWORK_TIMEOUT_MICROSECONDS,
        "-f",
        "flv",
        destination,
    ]
    .map(String::from)
    .to_vec()
}

/// Receives the audio buffer written by the screen-audio timeline.
#[derive(Clone)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("stream audio buffer")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// FFmpeg connects to the listener for its audio input; this runs apart from the video writer
/// because FFmpeg waits for audio data while probing, before it reads more video.
fn send_audio(
    listener: TcpListener,
    buffer: SharedBuffer,
    audio: CaptureAudio,
    done: Arc<AtomicBool>,
) {
    let mut socket: TcpStream = loop {
        match listener.accept() {
            Ok((socket, _)) => break socket,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    && !done.load(Ordering::Acquire) =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return,
        }
    };
    if socket.set_nonblocking(false).is_err()
        || socket
            .set_write_timeout(Some(Duration::from_secs(10)))
            .is_err()
    {
        return;
    }
    loop {
        let finished = done.load(Ordering::Acquire);
        audio.pad_live(AUDIO_LAG);
        let bytes = std::mem::take(&mut *buffer.0.lock().expect("stream audio buffer"));
        if socket.write_all(&bytes).is_err() || finished {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

impl CaptureService {
    pub fn start_stream(
        &mut self,
        preset: CapturePreset,
        key: String,
        source: impl Fn() -> Result<Value, String> + Send + 'static,
    ) -> Result<CaptureStatus, String> {
        preset.validate()?;
        let url = preset
            .stream_url
            .clone()
            .ok_or("This preset has no stream URL.")?;
        validate_key(&key)?;
        self.prepare(&source)?;
        require_protocol(url.split_once(':').map_or("", |(scheme, _)| scheme))?;
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .and_then(|listener| listener.set_nonblocking(true).map(|_| listener))
            .map_err(|e| format!("Cannot open the stream audio channel: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("Cannot open the stream audio channel: {e}"))?
            .port();
        let mut command = Command::new(ffmpeg_sidecar::paths::ffmpeg_path());
        command
            .args(stream_args(&preset, port, &destination(&url, &key)))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|e| mask(&format!("Cannot start stream encoder: {e}"), &key))?;
        let mut input = child
            .stdin
            .take()
            .ok_or("Stream encoder stdin is unavailable.")?;
        let errors = read_stderr(
            child
                .stderr
                .take()
                .ok_or("Stream encoder stderr is unavailable.")?,
        );
        self.stop.store(false, Ordering::Release);
        let started = Instant::now();
        let buffer = SharedBuffer(Arc::default());
        *self.audio.timeline.lock().expect("capture audio mutex") = Some(AudioTimeline {
            screen_id: preset.source_screen_id.clone(),
            map: preset.recording_audio_map(),
            file: Box::new(BufWriter::new(buffer.clone())),
            started,
            written_frames: 0,
            error: None,
        });
        *self.status.lock().expect("capture status mutex") = CaptureStatus {
            active: true,
            preset_id: preset.id.clone(),
            file_path: url,
            ..CaptureStatus::default()
        };
        let done = Arc::new(AtomicBool::new(false));
        let sender = {
            let (audio, done) = (self.audio.clone(), Arc::clone(&done));
            thread::spawn(move || send_audio(listener, buffer, audio, done))
        };
        let status = Arc::clone(&self.status);
        let stop = Arc::clone(&self.stop);
        let audio = self.audio.clone();
        self.worker = Some(thread::spawn(move || {
            let mut failure = write_frames(
                &preset, &source, &mut input, &status, &stop, &audio, started, None,
            );
            {
                let mut current = status.lock().expect("capture status mutex");
                current.active = false;
                current.finalizing = true;
            }
            drop(input);
            done.store(true, Ordering::Release);
            audio.timeline.lock().expect("capture audio mutex").take();
            let _ = sender.join();
            let encoder_error = errors
                .join()
                .unwrap_or_else(|_| "Encoder error reader failed.".into());
            match child.wait() {
                Ok(exit) if exit.success() => {}
                Ok(exit) => {
                    failure = Some(format!(
                        "The stream ended: encoder exited with {exit}: {encoder_error}"
                    ));
                }
                Err(error) => {
                    failure = Some(format!("Stream encoder finalization failed: {error}"))
                }
            }
            let mut current = status.lock().expect("capture status mutex");
            current.finalizing = false;
            current.elapsed_seconds = started.elapsed().as_secs_f64();
            current.error = failure.map(|error| mask(&error, &key));
        }));
        Ok(self.status())
    }
}
