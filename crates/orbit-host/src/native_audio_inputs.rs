use super::input_capture::{Capture, InputSource};
use super::*;

struct Track {
    config: crate::audio_input_config::CaptureConfig,
    main_delay_ms: u32,
    capture: Capture,
    player: Option<Player>,
    sink_key: String,
    pending: VecDeque<NativePcmBlock>,
}

#[derive(Default)]
pub(super) struct Inputs {
    tracks: HashMap<String, Track>,
    errors: HashMap<String, String>,
}

impl Inputs {
    pub fn requested(content: &Value) -> bool {
        content["audioInputs"].as_array().is_some_and(|inputs| {
            inputs
                .iter()
                .any(|input| crate::audio_input_config::enabled(input, content))
        })
    }

    pub fn sync(
        &mut self,
        source: &str,
        content: &Value,
        sink: Option<&MixerDeviceSink>,
        sink_key: &str,
    ) -> Result<(), String> {
        let inputs = content
            .get("audioInputs")
            .cloned()
            .unwrap_or_else(|| json!([]));
        if let Err(error) = crate::audio_input_config::validate_collection(&inputs) {
            self.clear(source);
            return Err(error);
        }
        let prefix = format!("{source}\0");
        let mut active = std::collections::HashSet::new();
        let settings =
            normalize_audio_settings_value(content.get("audioSettings").unwrap_or(&Value::Null));
        let main = &settings["main"];
        let main_enabled = main_audio_target(main).is_some();
        let main_delay_ms = u32::try_from(delay_from_settings(main).as_millis())
            .map_err(|_| "Live audio output delay is out of range.")?;
        let ndi_enabled = ndi_audio_enabled(&settings);
        for input in inputs.as_array().unwrap() {
            let id = input["id"].as_str().unwrap();
            let key = format!("{prefix}{id}");
            if !crate::audio_input_config::enabled(input, content)
                || (!main_enabled && !ndi_enabled)
            {
                continue;
            }
            active.insert(key.clone());
            let result = (|| -> Result<(), String> {
                let config: crate::audio_input_config::CaptureConfig =
                    serde_json::from_value(input.get("capture").cloned().ok_or(
                        "Configure the audio input device and channels before enabling it.",
                    )?)
                    .map_err(|e| e.to_string())?;
                if self.tracks.get(&key).is_some_and(|track| {
                    track.config != config || track.main_delay_ms != main_delay_ms
                }) {
                    self.tracks.remove(&key);
                }
                if !self.tracks.contains_key(&key) {
                    let capture = Capture::start(config.clone(), main_delay_ms)?;
                    self.tracks.insert(
                        key.clone(),
                        Track {
                            config,
                            main_delay_ms,
                            capture,
                            player: None,
                            sink_key: String::new(),
                            pending: VecDeque::new(),
                        },
                    );
                }
                let track = self.tracks.get_mut(&key).unwrap();
                let mode = track.capture.shared.ndi_mode.load(Ordering::Acquire);
                if mode & 1 != u64::from(ndi_enabled) {
                    track.capture.shared.ndi_mode.store(
                        (mode.wrapping_add(2) & !1) | u64::from(ndi_enabled),
                        Ordering::Release,
                    );
                    track.capture.ndi.reset();
                    track.pending.clear();
                }
                let route_key = format!("{sink_key}:{}:{}", main["routing"], main["delay"]);
                if !main_enabled || track.sink_key != route_key {
                    track.player = None;
                }
                if main_enabled && track.player.is_none() {
                    let sink = sink.ok_or("Audio input output device is unavailable.")?;
                    let player = Player::connect_new(sink.mixer());
                    player.set_volume(gain_from_settings(main));
                    track
                        .capture
                        .shared
                        .samples
                        .lock()
                        .map_err(|_| "Audio input buffer is unavailable.")?
                        .clear();
                    let source = InputSource {
                        shared: track.capture.shared.clone(),
                        reserve: (track.config.delay_ms + track.config.latency_ms + main_delay_ms)
                            as usize
                            * 48
                            * track.capture.bus_channels,
                        channels: track.capture.bus_channels.max(2),
                        capture_channels: track.capture.bus_channels,
                        frame: vec![0.0; 256 * track.capture.bus_channels.max(2)],
                        cursor: 256 * track.capture.bus_channels.max(2),
                    };
                    player.append(RoutedSource::new(source, audio_routing(main)));
                    track.player = Some(player);
                    track.sink_key = route_key;
                }
                if let Some(player) = &track.player {
                    player.set_volume(gain_from_settings(main));
                }
                Ok(())
            })();
            match result {
                Ok(()) => {
                    self.errors.remove(&key);
                }
                Err(error) => {
                    self.errors.insert(key, error);
                }
            }
        }
        self.tracks
            .retain(|key, _| !key.starts_with(&prefix) || active.contains(key));
        self.errors
            .retain(|key, _| !key.starts_with(&prefix) || active.contains(key));
        match self.errors.iter().find(|(key, _)| key.starts_with(&prefix)) {
            Some((_, error)) => Err(error.clone()),
            None => match self
                .tracks
                .iter()
                .filter(|(key, _)| key.starts_with(&prefix))
                .find_map(|(_, track)| {
                    track
                        .capture
                        .shared
                        .error
                        .lock()
                        .ok()
                        .and_then(|error| error.clone())
                }) {
                Some(error) => Err(error),
                None => Ok(()),
            },
        }
    }

    pub fn clear(&mut self, source: &str) {
        let prefix = format!("{source}\0");
        self.tracks.retain(|key, _| !key.starts_with(&prefix));
        self.errors.retain(|key, _| !key.starts_with(&prefix));
    }

    pub fn drain(&mut self, source: &str, now: Instant) -> VecDeque<NativePcmBlock> {
        let prefix = format!("{source}\0");
        let mut mixed = VecDeque::new();
        for (key, track) in &mut self.tracks {
            if !key.starts_with(&prefix) {
                continue;
            }
            if !track.capture.shared.running.load(Ordering::Acquire)
                || track.capture.shared.ndi_mode.load(Ordering::Acquire) & 1 == 0
            {
                track.pending.clear();
                track.capture.ndi.reset();
                continue;
            }
            while let Some(mut block) = track.capture.ndi.try_pop() {
                block.timestamp += Duration::from_millis(u64::from(
                    track.config.delay_ms + track.config.latency_ms,
                ));
                track.pending.push_back(block);
                if track.pending.len() > 512 {
                    track.pending.pop_front();
                }
            }
            let mut due = VecDeque::new();
            while track
                .pending
                .front()
                .is_some_and(|block| block.start_time() <= now)
            {
                due.push_back(track.pending.pop_front().unwrap());
            }
            mixed = mix_ndi_pending(&mut mixed, &mut due, 512).into();
        }
        mixed
    }

    pub fn status(&self) -> Value {
        let mut status = serde_json::Map::new();
        for (key, track) in &self.tracks {
            status.insert(key.clone(), json!({"running":track.capture.shared.running.load(Ordering::Acquire),"peak":f32::from_bits(track.capture.shared.peak.load(Ordering::Relaxed)),"error":track.capture.shared.error.lock().ok().and_then(|error| error.clone())}));
        }
        for (key, error) in &self.errors {
            status.insert(key.clone(), json!({"running":false,"error":error}));
        }
        Value::Object(status)
    }
}

pub(crate) fn list_devices() -> Result<Value, String> {
    let devices = cpal::default_host()
        .input_devices()
        .map_err(|error| error.to_string())?
        .map(|device| {
            let name = device
                .description()
                .map_err(|error| error.to_string())?
                .name()
                .to_string();
            let channels = device
                .default_input_config()
                .map_err(|error| error.to_string())?
                .channels();
            Ok(json!({"name":name,"channels":channels}))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!(devices))
}
