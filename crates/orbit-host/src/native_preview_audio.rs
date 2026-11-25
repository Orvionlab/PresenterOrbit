//! An operator audition owns its own decoder, player, sink, and meter. It never joins the program registry.
use super::*;
use std::sync::atomic::AtomicU32;

#[derive(Default)]
pub struct PreviewAudio {
    player: Option<Player>,
    sink: Option<MixerDeviceSink>,
    level: Arc<AtomicU32>,
    duration: Option<f64>,
}

impl PreviewAudio {
    pub fn stop(&mut self) {
        self.player = None;
        self.sink = None;
        self.level.store(0, Ordering::Relaxed);
        self.duration = None;
    }

    /// Runs on a worker. The caller resolves the workspace asset before passing it here.
    pub fn play(
        &mut self,
        path: &Path,
        main: &Value,
        asset: Option<&Value>,
        start: f64,
        end: Option<f64>,
    ) -> Result<(), String> {
        self.stop();
        let Some((device, channels)) = main_audio_target(main) else {
            return Ok(());
        };
        let mut decoder = Decoder::try_from(File::open(path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("Cannot decode preview audio: {error}"))?;
        self.duration = decoder
            .total_duration()
            .map(|duration| duration.as_secs_f64());
        let sink = open_main_audio_sink(&device, channels)?;
        if !start.is_finite()
            || start < 0.
            || end.is_some_and(|end| !end.is_finite() || end <= start)
        {
            return Err("Preview range requires a finite start and a later end.".into());
        }
        if start > 0. {
            decoder
                .try_seek(Duration::from_secs_f64(start))
                .map_err(|error| error.to_string())?;
        }
        let player = Player::connect_new(sink.mixer());
        let (route, gain) = match asset {
            Some(asset) => asset_audio_routing(asset)?,
            None => (audio_routing(&Value::Null), 1.),
        };
        let decoder = RoutedSource::new(decoder, route).amplify(gain);
        let source =
            RoutedSource::new(decoder, audio_routing(main)).amplify(gain_from_settings(main));
        let meter = Meter {
            input: source,
            level: self.level.clone(),
            sum: 0.,
            count: 0,
        };
        let delay = delay_from_settings(main);
        if let Some(end) = end {
            player.append(
                meter
                    .take_duration(Duration::from_secs_f64(end - start))
                    .delay(delay),
            );
        } else {
            player.append(meter.delay(delay));
        }
        self.player = Some(player);
        self.sink = Some(sink);
        Ok(())
    }

    pub fn pause(&self, paused: bool) {
        if let Some(player) = &self.player {
            if paused {
                player.pause();
            } else {
                player.play();
            }
        }
        if paused {
            self.level.store(0, Ordering::Relaxed);
        }
    }

    pub fn seek(&self, seconds: f64) -> Result<(), String> {
        if !seconds.is_finite() || seconds < 0. {
            return Err("Preview seek must be finite and non-negative.".into());
        }
        self.player
            .as_ref()
            .ok_or("No preview audio is playing.")?
            .try_seek(Duration::from_secs_f64(seconds))
            .map_err(|error| error.to_string())
    }

    pub fn status(&self) -> Value {
        let playing = self
            .player
            .as_ref()
            .is_some_and(|player| !player.empty() && !player.is_paused());
        json!({"playing":playing,"positionSeconds":self.player.as_ref().map(|player|player.get_pos().as_secs_f64()),"durationSeconds":self.duration,"level":if playing {f32::from_bits(self.level.load(Ordering::Relaxed))}else{0.}})
    }
}

struct Meter<I> {
    input: I,
    level: Arc<AtomicU32>,
    sum: f64,
    count: usize,
}
impl<I: Source<Item = f32>> Iterator for Meter<I> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sample = self.input.next()?;
        self.sum += f64::from(sample) * f64::from(sample);
        self.count += 1;
        if self.count
            >= usize::from(self.input.channels().get()) * self.input.sample_rate().get() as usize
                / 50
        {
            let rms = (self.sum / self.count as f64).sqrt();
            let percent = (((20. * rms.max(0.001).log10() + 60.) / 60.) * 100.)
                .round()
                .clamp(0., 100.) as f32;
            self.level.store(percent.to_bits(), Ordering::Relaxed);
            self.sum = 0.;
            self.count = 0;
        }
        Some(sample)
    }
}
impl<I: Source<Item = f32>> Source for Meter<I> {
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }
    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.input.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.sum = 0.;
        self.count = 0;
        self.level.store(0, Ordering::Relaxed);
        self.input.try_seek(pos)
    }
}
