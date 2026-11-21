use super::*;
use rodio::cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU32},
    Mutex,
};

pub(super) struct Shared {
    pub samples: Mutex<VecDeque<f32>>,
    pub error: Mutex<Option<String>>,
    pub running: AtomicBool,
    pub peak: AtomicU32,
    pub ndi_mode: AtomicU64,
    stop: AtomicBool,
    capacity: usize,
}

pub(super) struct Capture {
    pub shared: Arc<Shared>,
    pub ndi: PcmTapReader,
    pub bus_channels: usize,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}

impl Capture {
    pub fn start(
        config: crate::audio_input_config::CaptureConfig,
        main_delay_ms: u32,
    ) -> Result<Self, String> {
        config.validate()?;
        if main_delay_ms > 10_000 {
            return Err("Live audio output delay must not exceed 10000 ms.".into());
        }
        let routing = config.routing_plan();
        let bus_channels = routing.bus_channels;
        let capacity = (config.delay_ms + config.latency_ms + main_delay_ms + 500) as usize
            * 48
            * bus_channels;
        let shared = Arc::new(Shared {
            samples: Mutex::new(VecDeque::with_capacity(capacity)),
            error: Mutex::new(None),
            running: AtomicBool::new(false),
            peak: AtomicU32::new(0),
            ndi_mode: AtomicU64::new(1),
            stop: AtomicBool::new(false),
            capacity,
        });
        let (writer, ndi) = PcmTap::new_input(bus_channels, 48_000);
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("orbit-audio-input".into())
            .spawn(move || {
                if config.ndi_source.is_some() {
                    run_ndi(config, worker, writer);
                    return;
                }
                let mut writer = Some(writer);
                while !worker.stop.load(Ordering::Acquire) {
                    match open(
                        &config,
                        worker.clone(),
                        writer.take().expect("capture writer"),
                    ) {
                        Ok((stream, returned)) => {
                            // The stream owns the writer until it is dropped; reconnect creates a new producer
                            // for the same bounded ring after the old callback has stopped.
                            while !worker.stop.load(Ordering::Acquire)
                                && worker.error.lock().unwrap().is_none()
                            {
                                std::thread::sleep(Duration::from_millis(100));
                            }
                            worker.running.store(false, Ordering::Release);
                            drop(stream);
                            writer = Some(returned());
                        }
                        Err((error, returned)) => {
                            *worker.error.lock().unwrap() = Some(error);
                            writer = Some(returned);
                        }
                    }
                    worker.peak.store(0, Ordering::Relaxed);
                    if let Ok(mut samples) = worker.samples.lock() {
                        samples.clear();
                    }
                    if !worker.stop.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(500));
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            shared,
            ndi,
            bus_channels,
        })
    }
}

fn run_ndi(
    config: crate::audio_input_config::CaptureConfig,
    shared: Arc<Shared>,
    writer: PcmTapWriter,
) {
    let source = config
        .ndi_source
        .as_ref()
        .expect("validated NDI source")
        .clone();
    let mut processor = Processor {
        writer,
        phase: 0,
        ndi_mode: 1,
        routing: Arc::new(config.routing_plan()),
        config,
        shared: shared.clone(),
    };
    while !shared.stop.load(Ordering::Acquire) {
        let result = (|| -> Result<(), String> {
            let mut receiver = crate::ndi_input::AudioReceiver::new(&source)?;
            let mut received = Instant::now();
            let mut rate = 0;
            let mut clock = CaptureClock {
                start: Instant::now(),
                frames: 0,
            };
            while !shared.stop.load(Ordering::Acquire) {
                match receiver.poll_routed(&processor.routing)? {
                    Some((next_rate, samples)) => {
                        if !shared.running.swap(true, Ordering::AcqRel) || rate != next_rate {
                            processor.writer.reset(Duration::ZERO);
                            processor.phase = 0;
                            clock = CaptureClock {
                                start: Instant::now(),
                                frames: 0,
                            };
                        }
                        rate = next_rate;
                        *shared.error.lock().unwrap() = None;
                        processor.push_interleaved(&samples, rate, Some(&mut clock));
                        received = Instant::now();
                    }
                    None if received.elapsed() > Duration::from_secs(2) => {
                        return Err("NDI audio source is not delivering audio.".into());
                    }
                    None => {}
                }
            }
            Ok(())
        })();
        shared.running.store(false, Ordering::Release);
        shared.peak.store(0, Ordering::Relaxed);
        shared.samples.lock().unwrap().clear();
        processor.writer.reset(Duration::ZERO);
        processor.phase = 0;
        if let Err(error) = result {
            *shared.error.lock().unwrap() = Some(error);
        }
        if !shared.stop.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

// The callback state can only be recovered once CPAL has stopped the stream.
type ReturnWriter = Box<dyn FnOnce() -> PcmTapWriter>;
type OpenResult = Result<(rodio::cpal::Stream, ReturnWriter), (String, PcmTapWriter)>;

fn open(
    config: &crate::audio_input_config::CaptureConfig,
    shared: Arc<Shared>,
    mut writer: PcmTapWriter,
) -> OpenResult {
    writer.reset(Duration::ZERO);
    let routing = Arc::new(config.routing_plan());
    let prepare = || -> Result<_, String> {
        let host = rodio::cpal::default_host();
        let device = host
            .input_devices()
            .map_err(|e| e.to_string())?
            .find(|device| {
                device.description().is_ok_and(|description| {
                    Some(description.name()) == config.device_name.as_deref()
                })
            })
            .ok_or_else(|| {
                format!(
                    "Audio input '{}' is unavailable.",
                    config
                        .device_name
                        .as_deref()
                        .expect("validated local device")
                )
            })?;
        let format = device.default_input_config().map_err(|e| e.to_string())?;
        if routing.source_channels > format.channels() as usize {
            return Err("Selected audio input channel is unavailable.".into());
        }
        Ok((device, format))
    };
    let (device, format) = match prepare() {
        Ok(value) => value,
        Err(error) => return Err((error, writer)),
    };
    let state = Arc::new(Mutex::new(Some(Processor {
        writer,
        phase: 0,
        ndi_mode: 1,
        routing: routing.clone(),
        config: config.clone(),
        shared: shared.clone(),
    })));
    let callback = state.clone();
    let error_shared = shared.clone();
    let on_error = move |error| {
        error_shared.running.store(false, Ordering::Release);
        *error_shared.error.lock().unwrap() = Some(format!("Audio capture failed: {error}"));
    };
    let channels = format.channels() as usize;
    let rate = format.sample_rate();
    macro_rules! build {
        ($sample:ty) => {
            device.build_input_stream(
                &format.config(),
                move |data: &[$sample], _| {
                    if let Ok(mut state) = callback.try_lock() {
                        if let Some(processor) = state.as_mut() {
                            let routing = processor.routing.clone();
                            processor.push_frames(
                                data.chunks_exact(channels).map(move |frame| {
                                    let mut buses =
                                        [0.0; crate::audio_input_config::MAX_AUDIO_INPUT_BUSES];
                                    routing.map_channel_frame(
                                        |input| rodio::cpal::Sample::to_sample::<f32>(frame[input]),
                                        &mut buses,
                                    );
                                    buses
                                }),
                                rate,
                                None,
                            );
                        }
                    }
                },
                on_error,
                None,
            )
        };
    }
    let stream = match format.sample_format() {
        SampleFormat::F32 => build!(f32),
        SampleFormat::F64 => build!(f64),
        SampleFormat::I16 => build!(i16),
        SampleFormat::U16 => build!(u16),
        SampleFormat::I32 => build!(i32),
        _ => Err(rodio::cpal::BuildStreamError::StreamConfigNotSupported),
    };
    let recover = move || {
        state
            .lock()
            .unwrap()
            .take()
            .expect("stopped capture callback")
            .writer
    };
    let stream = match stream {
        Ok(stream) => stream,
        Err(error) => return Err((error.to_string(), recover())),
    };
    *shared.error.lock().unwrap() = None;
    shared.running.store(true, Ordering::Release);
    if let Err(error) = stream.play() {
        shared.running.store(false, Ordering::Release);
        drop(stream);
        return Err((error.to_string(), recover()));
    }
    Ok((stream, Box::new(recover)))
}

struct Processor {
    writer: PcmTapWriter,
    phase: u64,
    ndi_mode: u64,
    config: crate::audio_input_config::CaptureConfig,
    routing: Arc<crate::audio_input_config::InputRoutingPlan>,
    shared: Arc<Shared>,
}

struct CaptureClock {
    start: Instant,
    frames: u64,
}

impl CaptureClock {
    fn timestamp(&self) -> Instant {
        self.start + Duration::from_secs_f64(self.frames as f64 / 48_000.0)
    }
}

impl Processor {
    fn push_interleaved(&mut self, samples: &[f32], rate: u32, clock: Option<&mut CaptureClock>) {
        let channels = self.routing.bus_channels;
        let frames = samples.chunks_exact(channels).map(|frame| {
            let mut buses = [0.0; crate::audio_input_config::MAX_AUDIO_INPUT_BUSES];
            buses[..channels].copy_from_slice(frame);
            buses
        });
        self.push_frames(frames, rate, clock);
    }

    fn push_frames(
        &mut self,
        frames: impl Iterator<Item = [f32; crate::audio_input_config::MAX_AUDIO_INPUT_BUSES]>,
        rate: u32,
        mut clock: Option<&mut CaptureClock>,
    ) {
        let mode = self.shared.ndi_mode.load(Ordering::Acquire);
        if mode != self.ndi_mode {
            self.writer.reset(Duration::ZERO);
            self.ndi_mode = mode;
        }
        let gain = 10.0f32.powf(self.config.level_db / 20.0);
        let channels = self.routing.bus_channels;
        let mut queue = self.shared.samples.try_lock().ok();
        let capacity = self.shared.capacity;
        let mut peak = 0.0f32;
        for frame in frames {
            self.phase += 48_000;
            while self.phase >= u64::from(rate) {
                self.phase -= u64::from(rate);
                for sample in frame.iter().take(channels) {
                    let sample = if sample.is_finite() {
                        *sample * gain
                    } else {
                        0.0
                    };
                    peak = peak.max(sample.abs());
                    if mode & 1 == 1 {
                        if let Some(clock) = clock.as_ref() {
                            self.writer.push_at(sample, clock.timestamp());
                        } else {
                            self.writer.push(sample);
                        }
                    }
                    if let Some(queue) = queue.as_mut() {
                        if queue.len() >= capacity {
                            queue.pop_front();
                        }
                        queue.push_back(sample);
                    }
                }
                if let Some(clock) = clock.as_mut() {
                    clock.frames += 1;
                }
            }
        }
        self.shared.peak.store(peak.to_bits(), Ordering::Relaxed);
    }
}

pub(super) struct InputSource {
    pub shared: Arc<Shared>,
    pub reserve: usize,
    pub channels: usize,
    pub capture_channels: usize,
    pub frame: Vec<f32>,
    pub cursor: usize,
}

impl Iterator for InputSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if !self.shared.running.load(Ordering::Acquire) {
            self.cursor = self.frame.len();
            return Some(0.0);
        }
        if self.cursor == self.frame.len() {
            self.frame.fill(0.0);
            let frames = self.frame.len() / self.channels;
            let required = frames * self.capture_channels;
            if let Ok(mut queue) = self.shared.samples.try_lock() {
                if self.shared.running.load(Ordering::Acquire)
                    && queue.len() >= self.reserve + required
                {
                    for frame in 0..frames {
                        for channel in 0..self.capture_channels {
                            self.frame[frame * self.channels + channel] =
                                queue.pop_front().unwrap();
                        }
                    }
                }
            }
            self.cursor = 0;
        }
        let sample = self.frame[self.cursor];
        self.cursor += 1;
        Some(sample)
    }
}

impl Source for InputSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.channels as u16).unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        rodio::SampleRate::new(48_000).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
