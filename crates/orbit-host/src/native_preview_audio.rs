//! An operator audition owns its own decoder, player, sink, and meter. It never joins the program registry.
use super::*;
use std::{
    io::Read,
    process::{Child, ChildStdout, Command, Stdio},
    sync::atomic::AtomicU32,
};

const METER_CHANNELS: usize = LOGICAL_CHANNELS as usize;
type ChannelLevels = Arc<[AtomicU32; METER_CHANNELS]>;

#[derive(Default)]
pub struct PreviewAudio {
    player: Option<Player>,
    sink: Option<MixerDeviceSink>,
    level: Arc<AtomicU32>,
    levels: ChannelLevels,
    duration: Option<f64>,
}

impl PreviewAudio {
    pub fn stop(&mut self) {
        self.player = None;
        self.sink = None;
        self.clear_levels();
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
        video: bool,
    ) -> Result<(), String> {
        self.stop();
        let Some((device, channels)) = main_audio_target(main) else {
            return Ok(());
        };
        if !start.is_finite()
            || start < 0.
            || end.is_some_and(|end| !end.is_finite() || end <= start)
        {
            return Err("Preview range requires a finite start and a later end.".into());
        }
        let decoder: Box<dyn Source + Send> = if video {
            let Some(source) = VideoAudio::open_media(path, start, false)? else {
                return Ok(());
            };
            self.duration = source.duration;
            Box::new(source)
        } else {
            let mut decoder =
                Decoder::try_from(File::open(path).map_err(|error| error.to_string())?)
                    .map_err(|error| format!("Cannot decode preview audio: {error}"))?;
            self.duration = decoder
                .total_duration()
                .map(|duration| duration.as_secs_f64());
            if start > 0. {
                decoder
                    .try_seek(Duration::from_secs_f64(start))
                    .map_err(|error| error.to_string())?;
            }
            Box::new(decoder)
        };
        let sink = open_main_audio_sink(&device, channels)?;
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
            levels: self.levels.clone(),
            squares: [0.; METER_CHANNELS],
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
            self.clear_levels();
        }
    }

    fn clear_levels(&self) {
        self.level.store(0, Ordering::Relaxed);
        for level in self.levels.iter() {
            level.store(0, Ordering::Relaxed);
        }
    }

    pub fn channel_levels(&self) -> [f32; METER_CHANNELS] {
        if self
            .player
            .as_ref()
            .is_some_and(|player| !player.empty() && !player.is_paused())
        {
            return std::array::from_fn(|channel| {
                f32::from_bits(self.levels[channel].load(Ordering::Relaxed))
            });
        }
        [0.; METER_CHANNELS]
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

// FFmpeg seeks on the audio stream time base in multi-track containers.
pub(super) struct VideoAudio {
    path: PathBuf,
    duration: Option<f64>,
    looping: bool,
    child: Child,
    reader: BufReader<ChildStdout>,
    first: Option<f32>,
    channels: ChannelCount,
}

impl VideoAudio {
    pub(super) fn open_media(
        path: &Path,
        start: f64,
        looping: bool,
    ) -> Result<Option<Self>, String> {
        let metadata = crate::media_metadata::probe(&json!(path))?;
        if metadata["hasAudio"] == false {
            return Ok(None);
        }
        let channels = metadata["audioChannels"]
            .as_u64()
            .and_then(|count| u16::try_from(count).ok())
            .and_then(ChannelCount::new)
            .ok_or("Cannot determine video audio channels.")?;
        Self::open(
            path,
            start,
            channels,
            metadata["durationSeconds"].as_f64(),
            looping,
        )
        .map(Some)
    }

    fn open(
        path: &Path,
        start: f64,
        channels: ChannelCount,
        duration: Option<f64>,
        looping: bool,
    ) -> Result<Self, String> {
        let mut command = Command::new(ffmpeg_sidecar::paths::ffmpeg_path());
        if looping {
            command.args(["-stream_loop", "-1"]);
        }
        command
            .args([
                "-v",
                "error",
                "-nostdin",
                "-protocol_whitelist",
                "file",
                "-ss",
            ])
            .arg(start.to_string())
            .arg("-i")
            .arg(path)
            .args([
                "-map",
                "0:a:0",
                "-vn",
                "-sn",
                "-ar",
                "48000",
                "-c:a",
                "pcm_f32le",
                "-f",
                "f32le",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("Cannot start video preview audio: {error}"))?;
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Video preview audio has no output pipe.".into());
        };
        let mut source = Self {
            path: path.to_owned(),
            duration,
            looping,
            child,
            reader: BufReader::new(stdout),
            first: None,
            channels,
        };
        source.first = Some(
            source
                .next()
                .ok_or("Cannot decode video preview audio at this position.")?,
        );
        Ok(source)
    }
}

impl Drop for VideoAudio {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Iterator for VideoAudio {
    type Item = f32;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(first) = self.first.take() {
            return Some(first);
        }
        let mut sample = [0; 4];
        self.reader.read_exact(&mut sample).ok()?;
        Some(f32::from_le_bytes(sample))
    }
}

impl Source for VideoAudio {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        rodio::SampleRate::new(48_000).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.duration.map(Duration::from_secs_f64)
    }
    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        let replacement = Self::open(
            &self.path,
            position.as_secs_f64(),
            self.channels,
            self.duration,
            self.looping,
        )
        .map_err(|error| SeekError::Other(Arc::new(std::io::Error::other(error))))?;
        *self = replacement;
        Ok(())
    }
}

struct Meter<I> {
    input: I,
    level: Arc<AtomicU32>,
    levels: ChannelLevels,
    squares: [f64; METER_CHANNELS],
    count: usize,
}

fn meter_percent(mean_square: f64) -> u32 {
    super::program_levels::rms_percent(mean_square.sqrt()).to_bits()
}

impl<I: Source<Item = f32>> Iterator for Meter<I> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sample = self.input.next()?;
        let channels = usize::from(self.input.channels().get());
        if let Some(square) = self.squares.get_mut(self.count % channels) {
            *square += f64::from(sample).powi(2);
        }
        self.count += 1;
        let frames = (self.input.sample_rate().get() as usize / 50).max(1);
        if self.count >= channels * frames {
            let total: f64 = self.squares.iter().sum();
            self.level
                .store(meter_percent(total / self.count as f64), Ordering::Relaxed);
            for (level, square) in self.levels.iter().zip(&mut self.squares) {
                level.store(meter_percent(*square / frames as f64), Ordering::Relaxed);
                *square = 0.;
            }
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
        self.squares.fill(0.);
        self.count = 0;
        self.level.store(0, Ordering::Relaxed);
        for level in self.levels.iter() {
            level.store(0, Ordering::Relaxed);
        }
        self.input.try_seek(pos)
    }
}
