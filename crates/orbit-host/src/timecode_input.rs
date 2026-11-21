//! Timecode input service: LTC from an audio input device channel and MTC from
//! a MIDI input port. Decoded readings are published to the renderer, which
//! owns the timeline transport and chases them.

use crate::timecode::{FrameRate, Timecode};
use crate::timecode_ltc::{seconds_at_frame_end, LtcDecoder};
use crate::timecode_mtc::{MtcEvent, MtcParser};
use crate::Host;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const CONFIG_FILE: &str = "timecode-input.json";
/// Without a decoded frame for this long the input is reported as unlocked.
pub const DROPOUT: Duration = Duration::from_millis(500);
const POLL: Duration = Duration::from_millis(20);
/// The speed is measured over readings at most this old, and only once they span `SPEED_MIN_SPAN`.
const SPEED_WINDOW: Duration = Duration::from_secs(1);
const SPEED_MIN_SPAN: Duration = Duration::from_millis(500);
/// A step longer than this is a relocation, not movement, and restarts the speed measurement.
const JUMP_SECONDS: f64 = 2.0;
const MAX_CHANNEL: u16 = 64;
const METHODS: [&str; 4] = [
    "getTimecodeInputConfig",
    "setTimecodeInputConfig",
    "getTimecodeStatus",
    "listTimecodeDevices",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimecodeSource {
    #[default]
    None,
    Ltc,
    Mtc,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimecodeFormat {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "24")]
    Fps24,
    #[serde(rename = "25")]
    Fps25,
    #[serde(rename = "29.97df")]
    Fps2997Drop,
    #[serde(rename = "29.97ndf")]
    Fps2997NonDrop,
    #[serde(rename = "30")]
    Fps30,
}

impl TimecodeFormat {
    pub fn frame_rate(self) -> Option<FrameRate> {
        match self {
            Self::Auto => None,
            Self::Fps24 => Some(FrameRate::Fps24),
            Self::Fps25 => Some(FrameRate::Fps25),
            Self::Fps2997Drop => Some(FrameRate::Fps2997Drop),
            Self::Fps2997NonDrop => Some(FrameRate::Fps2997NonDrop),
            Self::Fps30 => Some(FrameRate::Fps30),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Fps24 => "24",
            Self::Fps25 => "25",
            Self::Fps2997Drop => "29.97df",
            Self::Fps2997NonDrop => "29.97ndf",
            Self::Fps30 => "30",
        }
    }
}

fn frame_rate_label(rate: FrameRate) -> &'static str {
    match rate {
        FrameRate::Fps24 => "24",
        FrameRate::Fps25 => "25",
        FrameRate::Fps2997Drop => "29.97df",
        FrameRate::Fps2997NonDrop => "29.97ndf",
        FrameRate::Fps30 => "30",
    }
}

fn timecode_day_seconds(rate: FrameRate) -> f64 {
    let nominal = u64::from(rate.nominal());
    let frames = nominal * 24 * 60 * 60
        - if rate == FrameRate::Fps2997Drop {
            2 * (24 * 60 - 24 * 6)
        } else {
            0
        };
    frames as f64 / rate.frames_per_second()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimecodeInputConfig {
    pub source: TimecodeSource,
    pub audio_device: String,
    pub audio_channel: u16,
    pub midi_port: String,
    pub target_playlist_id: String,
    pub format: TimecodeFormat,
    pub offset_seconds: f64,
}

impl TimecodeInputConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.offset_seconds.is_finite() || self.offset_seconds.abs() > 86_400.0 {
            return Err(
                "The timecode offset must be finite and between -86400 and 86400 seconds.".into(),
            );
        }
        match self.source {
            TimecodeSource::None => Ok(()),
            TimecodeSource::Ltc if self.audio_device.trim().is_empty() => {
                Err("Choose an audio input device for LTC.".into())
            }
            TimecodeSource::Ltc if !(1..=MAX_CHANNEL).contains(&self.audio_channel) => Err(
                format!("The LTC channel must be between 1 and {MAX_CHANNEL}."),
            ),
            TimecodeSource::Mtc if self.midi_port.trim().is_empty() => {
                Err("Choose a MIDI input port for MTC.".into())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimecodeReading {
    pub source: TimecodeSource,
    pub target_playlist_id: String,
    pub label: String,
    pub frame_rate: FrameRate,
    /// Media time at `at_ms`.
    pub seconds: f64,
    /// Whether the timecode moved, in either direction, since the previous reading.
    pub running: bool,
    /// Whether the timecode moved backwards.
    pub reverse: bool,
    /// Media seconds per wall second over the last second, once enough readings exist.
    pub speed: Option<f64>,
    pub at_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimecodeStatus {
    pub source: TimecodeSource,
    pub locked: bool,
    pub running: bool,
    pub label: Option<String>,
    pub frame_rate: Option<FrameRate>,
    pub seconds: Option<f64>,
    pub error: Option<String>,
}

pub enum TimecodeEvent {
    Reading(TimecodeReading),
    Status(TimecodeStatus),
}

pub type TimecodeSink = Arc<dyn Fn(TimecodeEvent) + Send + Sync>;

pub enum InputEvent {
    Ltc(Timecode, Instant),
    /// An LTC frame read backwards.
    LtcReverse(Timecode, Instant),
    Mtc(MtcEvent, Instant),
    Error(String),
}

struct Worker {
    stop: Sender<()>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
pub struct TimecodeService {
    worker: Mutex<Option<Worker>>,
    config: Mutex<TimecodeInputConfig>,
    status: Arc<Mutex<TimecodeStatus>>,
}

fn publish(sink: &TimecodeSink, shared: &Mutex<TimecodeStatus>, next: TimecodeStatus) {
    let Ok(mut status) = shared.lock() else {
        return;
    };
    let changed = (
        status.source,
        status.locked,
        status.running,
        status.frame_rate,
        &status.error,
    ) != (
        next.source,
        next.locked,
        next.running,
        next.frame_rate,
        &next.error,
    );
    *status = next.clone();
    drop(status);
    if changed {
        sink(TimecodeEvent::Status(next));
    }
}

/// Turns decoded timecodes into readings and tracks lock and dropout.
pub struct Pipeline {
    source: TimecodeSource,
    format: TimecodeFormat,
    offset_seconds: f64,
    target_playlist_id: String,
    previous: Option<f64>,
    /// Unwrapped position and arrival time of the readings that measure the speed.
    history: VecDeque<(f64, Instant)>,
    track: f64,
    moving_reverse: bool,
    last_event: Option<Instant>,
    status: TimecodeStatus,
}

impl Pipeline {
    pub fn with_config(config: &TimecodeInputConfig) -> Self {
        Self {
            source: config.source,
            format: config.format,
            offset_seconds: config.offset_seconds,
            target_playlist_id: config.target_playlist_id.clone(),
            previous: None,
            history: VecDeque::new(),
            track: 0.0,
            moving_reverse: false,
            last_event: None,
            status: TimecodeStatus {
                source: config.source,
                ..TimecodeStatus::default()
            },
        }
    }

    pub fn status(&self) -> &TimecodeStatus {
        &self.status
    }

    pub fn event(&mut self, event: InputEvent, wall_ms: i64) -> Option<TimecodeReading> {
        let (timecode, seconds, locate, at) = match event {
            InputEvent::Ltc(timecode, at) => (timecode, seconds_at_frame_end(&timecode), false, at),
            InputEvent::LtcReverse(timecode, at) => (timecode, timecode.seconds(), false, at),
            InputEvent::Mtc(MtcEvent::Running { timecode, seconds }, at) => {
                if let Some(error) = self.mtc_format_error(timecode.rate) {
                    self.clear_lock(error);
                    return None;
                }
                (timecode, seconds, false, at)
            }
            InputEvent::Mtc(MtcEvent::Locate { timecode, seconds }, at) => {
                if let Some(error) = self.mtc_format_error(timecode.rate) {
                    self.clear_lock(error);
                    return None;
                }
                (timecode, seconds, true, at)
            }
            InputEvent::Error(error) => {
                self.clear_lock(error);
                return None;
            }
        };
        let seconds = seconds + self.offset_seconds;
        let frame = 1.0 / timecode.rate.frames_per_second();
        let (mut running, mut reverse, mut delta) = (false, false, 0.0);
        if !locate {
            if let Some(previous) = self.previous {
                let day = timecode_day_seconds(timecode.rate);
                delta = seconds - previous;
                if delta < -day / 2.0 {
                    delta += day;
                } else if delta > day / 2.0 {
                    delta -= day;
                }
                running = delta.abs() > frame / 2.0;
                reverse = delta < 0.0 && running && delta.abs() <= JUMP_SECONDS;
                if delta.abs() > JUMP_SECONDS {
                    self.history.clear();
                }
            }
        }
        if locate || (running && reverse != self.moving_reverse) {
            self.history.clear();
        }
        if running {
            self.moving_reverse = reverse;
        }
        let speed = self.measure_speed(delta, at);
        self.previous = Some(seconds);
        self.last_event = Some(at);
        let label = timecode.offset_by_seconds(self.offset_seconds).label();
        self.status = TimecodeStatus {
            source: self.source,
            locked: true,
            running,
            label: Some(label.clone()),
            frame_rate: Some(timecode.rate),
            seconds: Some(seconds),
            error: None,
        };
        Some(TimecodeReading {
            source: self.source,
            target_playlist_id: self.target_playlist_id.clone(),
            label,
            frame_rate: timecode.rate,
            seconds,
            running,
            reverse,
            speed,
            at_ms: wall_ms,
        })
    }

    /// Media seconds per wall second between the oldest and newest readings of the window.
    fn measure_speed(&mut self, delta: f64, at: Instant) -> Option<f64> {
        self.track += delta;
        self.history.push_back((self.track, at));
        while self
            .history
            .front()
            .is_some_and(|(_, first)| at.saturating_duration_since(*first) > SPEED_WINDOW)
        {
            self.history.pop_front();
        }
        let ((from, start), (to, end)) = (self.history.front()?, self.history.back()?);
        let span = end.saturating_duration_since(*start);
        (span >= SPEED_MIN_SPAN).then(|| (to - from).abs() / span.as_secs_f64())
    }

    fn mtc_format_error(&self, received: FrameRate) -> Option<String> {
        self.format.frame_rate().and_then(|selected| {
            (selected != received).then(|| {
                format!(
                    "MTC reports the {} format, but the selected format is {}.",
                    frame_rate_label(received),
                    self.format.label()
                )
            })
        })
    }

    fn clear_lock(&mut self, error: String) {
        self.previous = None;
        self.history.clear();
        self.last_event = None;
        self.status.locked = false;
        self.status.running = false;
        self.status.error = Some(error);
    }

    /// Marks the input unlocked when no frame arrived within `DROPOUT`.
    pub fn check_dropout(&mut self, now: Instant, timeout: Duration) -> bool {
        let expired = self
            .last_event
            .is_some_and(|last| now.saturating_duration_since(last) >= timeout);
        if expired {
            self.last_event = None;
            self.previous = None;
            self.history.clear();
            self.status.locked = false;
            self.status.running = false;
        }
        expired
    }
}

fn run(
    config: &TimecodeInputConfig,
    events: &Receiver<InputEvent>,
    stopped: &Receiver<()>,
    sink: &TimecodeSink,
    shared: &Mutex<TimecodeStatus>,
    timeout: Duration,
) {
    let mut pipeline = Pipeline::with_config(config);
    publish(sink, shared, pipeline.status().clone());
    loop {
        match stopped.try_recv() {
            Err(mpsc::TryRecvError::Empty) => {}
            _ => return,
        }
        match events.recv_timeout(POLL) {
            Ok(event) => {
                let wall_ms = chrono::Utc::now().timestamp_millis();
                if let Some(reading) = pipeline.event(event, wall_ms) {
                    sink(TimecodeEvent::Reading(reading));
                }
                publish(sink, shared, pipeline.status().clone());
            }
            Err(RecvTimeoutError::Timeout) => {
                if pipeline.check_dropout(Instant::now(), timeout) {
                    publish(sink, shared, pipeline.status().clone());
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

impl TimecodeService {
    pub fn status_handle(&self) -> Arc<Mutex<TimecodeStatus>> {
        self.status.clone()
    }

    pub fn status(&self) -> Result<TimecodeStatus, String> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| "Timecode status is unavailable.".to_string())
    }

    pub fn config(&self) -> Result<TimecodeInputConfig, String> {
        self.config
            .lock()
            .map(|config| config.clone())
            .map_err(|_| "Timecode configuration is unavailable.".to_string())
    }

    pub fn configure(
        &self,
        sink: &TimecodeSink,
        config: TimecodeInputConfig,
    ) -> Result<TimecodeStatus, String> {
        config.validate()?;
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| "Timecode service is unavailable.".to_string())?;
        if let Some(Worker { stop, handle }) = worker.take() {
            drop(stop);
            handle
                .join()
                .map_err(|_| "The timecode input stopped unexpectedly.".to_string())?;
        }
        *self
            .config
            .lock()
            .map_err(|_| "Timecode configuration is unavailable.")? = config.clone();
        publish(
            sink,
            &self.status,
            TimecodeStatus {
                source: config.source,
                ..TimecodeStatus::default()
            },
        );
        if config.source == TimecodeSource::None {
            return self.status();
        }
        let (stop, stopped) = mpsc::channel();
        let (opened_tx, opened) = mpsc::channel();
        let status = self.status.clone();
        let worker_sink = sink.clone();
        let handle = thread::Builder::new()
            .name("presenter-orbit-timecode-input".into())
            .spawn(move || {
                let (sender, events) = mpsc::channel();
                let input = open_input(&config, sender);
                let failed = input.as_ref().err().cloned();
                let _ = opened_tx.send(failed);
                if let Ok(_input) = input {
                    run(&config, &events, &stopped, &worker_sink, &status, DROPOUT);
                }
            })
            .map_err(|error| format!("The timecode input could not start: {error}"))?;
        match opened.recv() {
            Ok(None) => {
                *worker = Some(Worker { stop, handle });
                self.status()
            }
            Ok(Some(error)) => {
                let _ = handle.join();
                let failed = TimecodeStatus {
                    source: self.config()?.source,
                    error: Some(error.clone()),
                    ..TimecodeStatus::default()
                };
                publish(sink, &self.status, failed);
                Err(error)
            }
            Err(_) => Err("The timecode input stopped while opening.".into()),
        }
    }
}

/// Keeps the opened device or port alive for as long as it is held.
enum OpenInput {
    #[allow(dead_code)]
    Audio(rodio::cpal::Stream),
    #[allow(dead_code)]
    Midi(midir::MidiInputConnection<()>),
}

fn open_input(
    config: &TimecodeInputConfig,
    sender: Sender<InputEvent>,
) -> Result<OpenInput, String> {
    match config.source {
        TimecodeSource::Ltc => open_audio(
            &config.audio_device,
            config.audio_channel,
            config.format.frame_rate(),
            sender,
        ),
        TimecodeSource::Mtc => open_midi(&config.midi_port, sender),
        TimecodeSource::None => Err("No timecode source is selected.".into()),
    }
}

fn open_midi(port_name: &str, sender: Sender<InputEvent>) -> Result<OpenInput, String> {
    let mut input = midir::MidiInput::new("Presenter Orbit timecode")
        .map_err(|error| format!("MIDI input is unavailable: {error}"))?;
    input.ignore(midir::Ignore::ActiveSense);
    let port = input
        .ports()
        .into_iter()
        .find(|port| input.port_name(port).is_ok_and(|name| name == port_name))
        .ok_or_else(|| format!("MIDI input '{port_name}' is unavailable."))?;
    let mut parser = MtcParser::default();
    let connection = input
        .connect(
            &port,
            "presenter-orbit-mtc",
            move |_, message, _| {
                if let Some(event) = parser.message(message) {
                    let _ = sender.send(InputEvent::Mtc(event, Instant::now()));
                }
            },
            (),
        )
        .map_err(|error| format!("MIDI input '{port_name}' could not be opened: {error}"))?;
    Ok(OpenInput::Midi(connection))
}

fn open_audio(
    device_name: &str,
    channel: u16,
    format: Option<FrameRate>,
    sender: Sender<InputEvent>,
) -> Result<OpenInput, String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use rodio::cpal::SampleFormat;
    let host = rodio::cpal::default_host();
    let requested = device_name.trim().to_ascii_lowercase();
    let device = if requested == "default" || requested == "system" {
        host.default_input_device()
            .ok_or("No default audio input device is available.")?
    } else {
        host.input_devices()
            .map_err(|error| format!("Cannot enumerate audio input devices: {error}"))?
            .find(|device| {
                device
                    .description()
                    .is_ok_and(|description| description.name().to_ascii_lowercase() == requested)
            })
            .ok_or_else(|| format!("Audio input device '{device_name}' is unavailable."))?
    };
    let supported = device
        .default_input_config()
        .map_err(|error| format!("Audio input '{device_name}' has no usable format: {error}"))?;
    let channels = usize::from(supported.channels());
    if usize::from(channel) > channels {
        return Err(format!(
            "Audio input '{device_name}' has {channels} channels; channel {channel} was requested."
        ));
    }
    let index = usize::from(channel) - 1;
    let sample_format = supported.sample_format();
    let stream_config = supported.config();
    let mut decoder = LtcDecoder::with_format(supported.sample_rate(), format);
    let failure = sender.clone();
    let on_error = move |error| {
        let _ = failure.send(InputEvent::Error(format!("Audio input failed: {error}")));
    };
    macro_rules! build {
        ($sample:ty) => {
            device.build_input_stream(
                &stream_config,
                move |data: &[$sample], _| {
                    let samples = data
                        .chunks_exact(channels)
                        .map(|frame| rodio::cpal::Sample::to_sample::<f32>(frame[index]));
                    for frame in decoder.push(samples) {
                        let at = Instant::now();
                        let _ = sender.send(if frame.reverse {
                            InputEvent::LtcReverse(frame.timecode, at)
                        } else {
                            InputEvent::Ltc(frame.timecode, at)
                        });
                    }
                },
                on_error,
                None,
            )
        };
    }
    let stream = match sample_format {
        SampleFormat::F32 => build!(f32),
        SampleFormat::I16 => build!(i16),
        SampleFormat::U16 => build!(u16),
        SampleFormat::I32 => build!(i32),
        SampleFormat::F64 => build!(f64),
        other => {
            return Err(format!(
                "Audio input sample format {other:?} is not supported."
            ))
        }
    }
    .map_err(|error| format!("Audio input '{device_name}' could not be opened: {error}"))?;
    stream
        .play()
        .map_err(|error| format!("Audio input '{device_name}' could not start: {error}"))?;
    Ok(OpenInput::Audio(stream))
}

pub fn list_devices() -> Result<Value, String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let host = rodio::cpal::default_host();
    let audio = host
        .input_devices()
        .map_err(|error| format!("Cannot enumerate audio input devices: {error}"))?
        .filter_map(|device| {
            let name = device.description().ok()?.name().to_string();
            let channels = device.default_input_config().ok()?.channels();
            Some(json!({ "name": name, "channels": channels }))
        })
        .collect::<Vec<_>>();
    let midi = midir::MidiInput::new("Presenter Orbit timecode")
        .map_err(|error| format!("MIDI input is unavailable: {error}"))?;
    let midi = midi
        .ports()
        .iter()
        .filter_map(|port| midi.port_name(port).ok())
        .collect::<Vec<_>>();
    Ok(json!({ "audioInputs": audio, "midiInputs": midi }))
}

pub fn config_path(directory: &Path) -> PathBuf {
    directory.join(CONFIG_FILE)
}

/// A missing file means the optional input was never configured.
pub fn load_config(directory: &Path) -> Result<TimecodeInputConfig, String> {
    let path = config_path(directory);
    if !path.exists() {
        return Ok(TimecodeInputConfig::default());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("The timecode input settings could not be read: {error}"))?;
    let config: TimecodeInputConfig = serde_json::from_str(&text)
        .map_err(|error| format!("The timecode input settings are invalid: {error}"))?;
    config.validate()?;
    Ok(config)
}

pub fn save_config(directory: &Path, config: &TimecodeInputConfig) -> Result<(), String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("The timecode input settings could not be saved: {error}"))?;
    let path = config_path(directory);
    let temporary = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(config).map_err(|error| error.to_string())?;
    std::fs::write(&temporary, text)
        .and_then(|()| std::fs::rename(&temporary, &path))
        .map_err(|error| format!("The timecode input settings could not be saved: {error}"))
}

pub fn handles(method: &str) -> bool {
    METHODS.contains(&method)
}

pub fn invoke(
    service: &TimecodeService,
    sink: &TimecodeSink,
    directory: &Path,
    method: &str,
    args: &[Value],
) -> Result<Value, String> {
    match method {
        "getTimecodeInputConfig" => to_json(&service.config()?),
        "getTimecodeStatus" => to_json(&service.status()?),
        "listTimecodeDevices" => list_devices(),
        "setTimecodeInputConfig" => {
            let config: TimecodeInputConfig = serde_json::from_value(
                args.first()
                    .cloned()
                    .ok_or("Timecode input settings are required.")?,
            )
            .map_err(|error| format!("Timecode input settings are invalid: {error}"))?;
            let status = service.configure(sink, config.clone())?;
            save_config(directory, &config)?;
            to_json(&status)
        }
        other => Err(format!("Unknown timecode method '{other}'.")),
    }
}

fn app_sink(app: &Host) -> TimecodeSink {
    let app = app.clone();
    Arc::new(move |event| {
        let _ = match event {
            TimecodeEvent::Reading(reading) => app.emit("timecode-reading", reading),
            TimecodeEvent::Status(status) => app.emit("timecode-status", status),
        };
    })
}

fn app_directory(app: &Host) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|error| error.to_string())
}

/// Blocking: opens audio and MIDI devices.
pub fn invoke_app(app: &Host, method: &str, args: &[Value]) -> Result<Value, String> {
    invoke(
        &app.state::<TimecodeService>(),
        &app_sink(app),
        &app_directory(app)?,
        method,
        args,
    )
}

/// Applies the saved input at startup. An unavailable device is reported
/// through the status instead of discarding the saved choice.
pub fn restore(app: &Host) -> Result<(), String> {
    let config = load_config(&app_directory(app)?)?;
    match app
        .state::<TimecodeService>()
        .configure(&app_sink(app), config)
    {
        Ok(_) => Ok(()),
        Err(error) => {
            eprintln!("Timecode input could not be restored: {error}");
            Ok(())
        }
    }
}

fn to_json<T: Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}
