use crate::native_compositor::NativeCompositor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub mod stream;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapturePreset {
    pub id: String,
    pub title: String,
    pub source_screen_id: String,
    pub width: usize,
    pub height: usize,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub output_directory: String,
    /// ProPresenter's custom recording audio map: per recording channel the screen mix
    /// channels (1, 2) summed into it. Absent records the screen mix as stereo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_map: Option<Vec<Vec<u8>>>,
    /// An `rtmp://` or `rtmps://` ingest URL: the preset streams instead of writing a file.
    /// The stream key is a secret kept outside the workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_url: Option<String>,
}

impl CapturePreset {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty()
            || self.title.trim().is_empty()
            || self.source_screen_id.trim().is_empty()
        {
            return Err("Recording preset id, title and source screen are required.".into());
        }
        if self.width < 16
            || self.height < 16
            || self.width > 3840
            || self.height > 2160
            || self.width % 2 != 0
            || self.height % 2 != 0
        {
            return Err("Recording dimensions must be even, between 16 and 3840×2160.".into());
        }
        if !(1..=60).contains(&self.fps) || !(100..=100_000).contains(&self.bitrate_kbps) {
            return Err("Recording frame rate must be 1–60 and bitrate 100–100000 Kbit/s.".into());
        }
        match &self.stream_url {
            Some(url) => stream::validate_url(url)?,
            None if !Path::new(&self.output_directory).is_absolute() => {
                return Err("Choose an absolute recording directory.".into());
            }
            None => {}
        }
        if let Some(map) = &self.audio_map {
            let valid_channel = |sources: &Vec<u8>| {
                sources
                    .iter()
                    .all(|source| (1..=AUDIO_CHANNELS as u8).contains(source))
                    && sources
                        .iter()
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        == sources.len()
            };
            if map.is_empty()
                || map.len() > MAX_RECORDING_CHANNELS
                || !map.iter().all(valid_channel)
            {
                return Err(format!(
                    "The recording audio map needs 1–{MAX_RECORDING_CHANNELS} recording channels, each fed by unique screen channels 1 and 2."
                ));
            }
        }
        Ok(())
    }

    /// Per recording channel the 0-based screen mix channels it sums.
    fn recording_audio_map(&self) -> Vec<Vec<usize>> {
        match &self.audio_map {
            Some(map) => map
                .iter()
                .map(|sources| {
                    sources
                        .iter()
                        .map(|source| usize::from(*source) - 1)
                        .collect()
                })
                .collect(),
            None => (0..AUDIO_CHANNELS).map(|channel| vec![channel]).collect(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub active: bool,
    pub finalizing: bool,
    pub preset_id: String,
    pub file_path: String,
    pub frames_written: u64,
    pub elapsed_seconds: f64,
    pub dropped_frames: u64,
    pub bytes_written: u64,
    pub error: Option<String>,
}

const AUDIO_SAMPLE_RATE: u32 = 48_000;
/// The screen mix delivered to the recording.
const AUDIO_CHANNELS: usize = 2;
/// AAC's discrete channel limit in the muxed recording.
const MAX_RECORDING_CHANNELS: usize = 8;

/// The recorded screen's mixed audio, placed on the recording timeline by block timestamp.
struct AudioTimeline {
    screen_id: String,
    /// Per recording channel the screen mix channels summed into it.
    map: Vec<Vec<usize>>,
    file: Box<dyn Write + Send>,
    started: Instant,
    written_frames: u64,
    error: Option<String>,
}

impl AudioTimeline {
    fn push(&mut self, samples: &[f32], sample_rate: u32, channels: usize, timestamp: Instant) {
        if self.error.is_some() {
            return;
        }
        if sample_rate != AUDIO_SAMPLE_RATE || channels != AUDIO_CHANNELS {
            self.error = Some(format!(
                "Recording audio must be {AUDIO_SAMPLE_RATE} Hz stereo; received {sample_rate} Hz with {channels} channels."
            ));
            return;
        }
        let offset = if timestamp >= self.started {
            (timestamp - self.started).as_secs_f64()
        } else {
            -(self.started - timestamp).as_secs_f64()
        };
        let position = (offset * f64::from(AUDIO_SAMPLE_RATE)).round() as i64;
        let frames = samples.len() / AUDIO_CHANNELS;
        // Blocks that overlap audio already written (or precede the recording) are trimmed.
        let skip = (self.written_frames as i64 - position).max(0) as usize;
        if skip >= frames {
            return;
        }
        let result = self
            .write_silence((position - self.written_frames as i64).max(0) as u64)
            .and_then(|_| {
                self.write_samples(&samples[skip * AUDIO_CHANNELS..frames * AUDIO_CHANNELS])
            });
        if let Err(error) = result {
            self.error = Some(format!("Recording audio write failed: {error}"));
        }
    }

    /// A live stream needs continuous audio, so silence fills the timeline up to `until`.
    fn pad_to(&mut self, until: Instant) -> std::io::Result<()> {
        let target = (until.saturating_duration_since(self.started).as_secs_f64()
            * f64::from(AUDIO_SAMPLE_RATE)) as u64;
        self.write_silence(target.saturating_sub(self.written_frames))
    }

    fn write_silence(&mut self, frames: u64) -> std::io::Result<()> {
        for _ in 0..frames * self.map.len() as u64 {
            self.file.write_all(&0.0_f32.to_le_bytes())?;
        }
        self.written_frames += frames;
        Ok(())
    }

    fn write_samples(&mut self, samples: &[f32]) -> std::io::Result<()> {
        for frame in samples.chunks_exact(AUDIO_CHANNELS) {
            for sources in &self.map {
                let sample: f32 = sources.iter().map(|source| frame[*source]).sum();
                self.file.write_all(&sample.to_le_bytes())?;
            }
        }
        self.written_frames += (samples.len() / AUDIO_CHANNELS) as u64;
        Ok(())
    }
}

/// Shared with the audio ticker, which must never wait for a stopping recording.
#[derive(Clone, Default)]
pub struct CaptureAudio {
    timeline: Arc<Mutex<Option<AudioTimeline>>>,
}

impl CaptureAudio {
    /// The screen whose audio is being recorded.
    pub fn source_screen_id(&self) -> Option<String> {
        self.timeline
            .lock()
            .ok()?
            .as_ref()
            .map(|timeline| timeline.screen_id.clone())
    }

    pub fn push(&self, samples: &[f32], sample_rate: u32, channels: usize, timestamp: Instant) {
        if let Some(timeline) = self.timeline.lock().expect("capture audio mutex").as_mut() {
            timeline.push(samples, sample_rate, channels, timestamp);
        }
    }

    /// Fills silence up to `lag` ago, so late-arriving blocks are not trimmed, and flushes.
    fn pad_live(&self, lag: Duration) {
        if let Some(timeline) = self.timeline.lock().expect("capture audio mutex").as_mut() {
            let until = Instant::now().checked_sub(lag).unwrap_or(timeline.started);
            if let Err(error) = timeline.pad_to(until).and_then(|_| timeline.file.flush()) {
                timeline
                    .error
                    .get_or_insert(format!("Stream audio write failed: {error}"));
            }
        }
    }

    fn take_error(&self) -> Option<String> {
        self.timeline
            .lock()
            .expect("capture audio mutex")
            .as_mut()
            .and_then(|timeline| timeline.error.take())
    }
}

pub struct CaptureService {
    status: Arc<Mutex<CaptureStatus>>,
    audio: CaptureAudio,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Default for CaptureService {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new(CaptureStatus::default())),
            audio: CaptureAudio::default(),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }
}

impl CaptureService {
    pub fn status_handle(&self) -> Arc<Mutex<CaptureStatus>> {
        self.status.clone()
    }

    pub fn audio_handle(&self) -> CaptureAudio {
        self.audio.clone()
    }

    pub fn status(&self) -> CaptureStatus {
        self.status.lock().expect("capture status mutex").clone()
    }

    /// Shared start checks: nothing is running, the source is ready and FFmpeg is installed.
    fn prepare(&mut self, source: &impl Fn() -> Result<Value, String>) -> Result<(), String> {
        if self.status().active || self.status().finalizing {
            return Err("A recording is already running.".into());
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "Recording worker failed.".to_string())?;
        }
        if source()?["outputFrozen"] == true {
            return Err("Resume output before starting a recording or stream.".into());
        }
        if !ffmpeg_sidecar::command::ffmpeg_is_installed() {
            return Err("FFmpeg is unavailable; recording requires the installed encoder.".into());
        }
        Ok(())
    }

    pub fn start(
        &mut self,
        preset: CapturePreset,
        source: impl Fn() -> Result<Value, String> + Send + 'static,
    ) -> Result<CaptureStatus, String> {
        preset.validate()?;
        if preset.stream_url.is_some() {
            return Err("A stream preset starts as a stream, not a file recording.".into());
        }
        if !Path::new(&preset.output_directory).is_dir() {
            return Err("The recording directory no longer exists.".into());
        }
        self.prepare(&source)?;
        let path = Path::new(&preset.output_directory).join(format!(
            "orbit-{}-{}.mp4",
            chrono::Utc::now().format("%Y%m%d-%H%M%S"),
            uuid::Uuid::new_v4()
        ));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("Cannot create recording: {e}"))?;
        let video_path = temporary_path(&path, "video.mp4");
        let audio_path = temporary_path(&path, "f32le");
        let audio_file = File::create(&video_path)
            .and_then(|_| File::create(&audio_path))
            .map_err(|e| {
                let _ = fs::remove_file(&path);
                let _ = fs::remove_file(&video_path);
                format!("Cannot create recording files: {e}")
            })?;
        let mut command = Command::new(ffmpeg_sidecar::paths::ffmpeg_path());
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "rawvideo",
                "-pixel_format",
                "bgra",
                "-video_size",
            ])
            .arg(format!("{}x{}", preset.width, preset.height))
            .arg("-framerate")
            .arg(preset.fps.to_string())
            .args([
                "-i", "pipe:0", "-an", "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt",
                "yuv420p", "-b:v",
            ])
            .arg(format!("{}k", preset.bitrate_kbps))
            .arg(&video_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|e| {
            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(&video_path);
            let _ = fs::remove_file(&audio_path);
            format!("Cannot start recording encoder: {e}")
        })?;
        let mut input = child
            .stdin
            .take()
            .ok_or("Recording encoder stdin is unavailable.")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Recording encoder stderr is unavailable.")?;
        let errors = read_stderr(stderr);
        self.stop.store(false, Ordering::Release);
        let started = Instant::now();
        *self.audio.timeline.lock().expect("capture audio mutex") = Some(AudioTimeline {
            screen_id: preset.source_screen_id.clone(),
            map: preset.recording_audio_map(),
            file: Box::new(BufWriter::new(audio_file)),
            started,
            written_frames: 0,
            error: None,
        });
        *self.status.lock().expect("capture status mutex") = CaptureStatus {
            active: true,
            preset_id: preset.id.clone(),
            file_path: path.to_string_lossy().into_owned(),
            ..CaptureStatus::default()
        };
        let status = Arc::clone(&self.status);
        let stop = Arc::clone(&self.stop);
        let audio = self.audio.clone();
        self.worker = Some(thread::spawn(move || {
            let mut failure = write_frames(
                &preset,
                &source,
                &mut input,
                &status,
                &stop,
                &audio,
                started,
                Some(&video_path),
            );
            {
                let mut current = status.lock().expect("capture status mutex");
                current.active = false;
                current.finalizing = true;
            }
            drop(input);
            let timeline = audio.timeline.lock().expect("capture audio mutex").take();
            let encoder_exit = child.wait();
            let encoder_error = errors
                .join()
                .unwrap_or_else(|_| "Encoder error reader failed.".into());
            let encoded = match encoder_exit {
                Ok(exit) if exit.success() => true,
                Ok(exit) => {
                    failure = Some(format!(
                        "Recording encoder exited with {exit}: {encoder_error}"
                    ));
                    false
                }
                Err(error) => {
                    failure = Some(format!("Recording encoder finalization failed: {error}"));
                    false
                }
            };
            if encoded {
                let frames = status.lock().expect("capture status mutex").frames_written;
                // Finish already encoded frames after a source failure while preserving that error.
                let finalized = finish_audio(timeline, frames, preset.fps).and_then(|_| {
                    mux_audio(
                        &video_path,
                        &audio_path,
                        preset.recording_audio_map().len(),
                        &path,
                    )
                });
                match finalized {
                    Ok(()) => {
                        let _ = fs::remove_file(&video_path);
                        let _ = fs::remove_file(&audio_path);
                    }
                    Err(error) => {
                        failure = Some(match failure {
                            Some(source_error) => format!("{source_error}; {error}"),
                            None => error,
                        });
                    }
                }
            }
            let mut current = status.lock().expect("capture status mutex");
            current.finalizing = false;
            current.elapsed_seconds = started.elapsed().as_secs_f64();
            match fs::metadata(&path) {
                Ok(metadata) => current.bytes_written = metadata.len(),
                Err(error) => {
                    failure = Some(format!("Finalized recording file status failed: {error}"))
                }
            }
            current.error = failure;
        }));
        Ok(self.status())
    }

    pub fn stop_if_current(&mut self, expected_file_path: &str) -> Result<CaptureStatus, String> {
        if self.status().file_path != expected_file_path {
            return Err("Recording changed while awaiting stop confirmation.".into());
        }
        self.stop()
    }

    pub fn stop(&mut self) -> Result<CaptureStatus, String> {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "Recording worker failed.".to_string())?;
        }
        Ok(self.status())
    }
}

/// Renders and writes frames at the preset rate until stopped; returns the failure that ended it.
#[allow(clippy::too_many_arguments)]
fn write_frames(
    preset: &CapturePreset,
    source: &impl Fn() -> Result<Value, String>,
    input: &mut impl Write,
    status: &Mutex<CaptureStatus>,
    stop: &AtomicBool,
    audio: &CaptureAudio,
    started: Instant,
    size_file: Option<&Path>,
) -> Option<String> {
    let period = Duration::from_secs_f64(1.0 / f64::from(preset.fps));
    let mut next_frame = started;
    let mut compositor = NativeCompositor::new();
    let mut pixels = vec![0; preset.width * preset.height * 4];
    let mut failure = None;
    while !stop.load(Ordering::Acquire) {
        let result = source()
            .and_then(|scene| {
                if scene["outputFrozen"] == true {
                    Ok(())
                } else {
                    compositor
                        .render_into(&mut pixels, preset.width, preset.height, &scene)
                        .map(|_| ())
                }
            })
            .and_then(|_| {
                input
                    .write_all(&pixels)
                    .map_err(|e| format!("Recording frame write failed: {e}"))
            })
            .and_then(|_| audio.take_error().map_or(Ok(()), Err));
        if let Err(error) = result {
            return Some(error);
        }
        let now = Instant::now();
        next_frame += period;
        let missed = if now > next_frame {
            ((now - next_frame).as_secs_f64() / period.as_secs_f64()).floor() as u64
        } else {
            0
        };
        // Duplicate the last composited frame for missed slots so mux time follows wall time.
        let mut repeated = 0;
        for _ in 0..missed {
            if let Err(error) = input.write_all(&pixels) {
                failure = Some(format!("Recording frame write failed: {error}"));
                break;
            }
            next_frame += period;
            repeated += 1;
        }
        {
            let mut current = status.lock().expect("capture status mutex");
            current.frames_written += 1 + repeated;
            current.dropped_frames += repeated;
            current.elapsed_seconds = started.elapsed().as_secs_f64();
            if let Some(file) = size_file {
                match fs::metadata(file) {
                    Ok(metadata) => current.bytes_written = metadata.len(),
                    Err(error) => failure = Some(format!("Recording file status failed: {error}")),
                }
            }
        }
        if failure.is_some() {
            return failure;
        }
        thread::sleep(next_frame.saturating_duration_since(Instant::now()));
    }
    None
}

fn read_stderr(stderr: impl Read + Send + 'static) -> JoinHandle<String> {
    thread::spawn(move || {
        let mut message = Vec::new();
        let mut reader = stderr;
        let mut buffer = [0; 4096];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let retain = count.min(65536_usize.saturating_sub(message.len()));
            message.extend_from_slice(&buffer[..retain]);
        }
        String::from_utf8_lossy(&message).trim().to_string()
    })
}

fn temporary_path(path: &Path, extension: &str) -> PathBuf {
    let name = path.file_stem().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{extension}"))
}

/// Pads the audio with silence to the video duration so both streams end together.
fn finish_audio(timeline: Option<AudioTimeline>, frames: u64, fps: u32) -> Result<(), String> {
    let mut timeline = timeline.ok_or("Recording audio timeline is unavailable.")?;
    if let Some(error) = timeline.error.take() {
        return Err(error);
    }
    let total = frames * u64::from(AUDIO_SAMPLE_RATE) / u64::from(fps);
    timeline
        .write_silence(total.saturating_sub(timeline.written_frames))
        .and_then(|_| timeline.file.flush())
        .map_err(|e| format!("Recording audio finalization failed: {e}"))
}

fn mux_audio(video: &Path, audio: &Path, channels: usize, output: &Path) -> Result<(), String> {
    let mut command = Command::new(ffmpeg_sidecar::paths::ffmpeg_path());
    command
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(video)
        .args(["-f", "f32le", "-ar"])
        .arg(AUDIO_SAMPLE_RATE.to_string())
        .arg("-ch_layout")
        .arg(discrete_layout(channels))
        .arg("-i")
        .arg(audio)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-shortest",
            "-movflags",
            "+faststart",
        ])
        .arg(output)
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let result = command
        .output()
        .map_err(|e| format!("Cannot start recording audio muxer: {e}"))?;
    if result.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Recording audio mux failed; the video is kept at {}: {}",
            video.display(),
            String::from_utf8_lossy(&result.stderr).trim()
        ))
    }
}

/// Layouts without an LFE channel, so FFmpeg's AAC encoder keeps every mapped recording
/// channel full-band (its guessed 3-channel layout is 2.1, which low-passes channel 3).
fn discrete_layout(channels: usize) -> &'static str {
    [
        "mono",
        "stereo",
        "3.0",
        "4.0",
        "5.0",
        "6.0",
        "7.0",
        "octagonal",
    ][channels - 1]
}

impl Drop for CaptureService {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
