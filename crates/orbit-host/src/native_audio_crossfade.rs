use super::*;

pub(super) struct MusicCrossfade {
    pub(super) native: Option<NativeAudioTrack>,
    pub(super) decoder: Option<NdiDecoderTrack>,
    routing: Option<Value>,
    pcm_gain: f32,
    elapsed: Duration,
    duration: Duration,
    last_tick: Instant,
    paused: bool,
}

impl MusicCrossfade {
    pub(super) fn fraction(&self) -> f32 {
        (self.elapsed.as_secs_f64() / self.duration.as_secs_f64()).clamp(0., 1.) as f32
    }
    fn scale(&self, blocks: &mut VecDeque<NativePcmBlock>, outgoing: bool) {
        for block in blocks {
            let delta = if self.paused {
                0.
            } else if block.timestamp >= self.last_tick {
                block.timestamp.duration_since(self.last_tick).as_secs_f64()
            } else {
                -self.last_tick.duration_since(block.timestamp).as_secs_f64()
            };
            let start = self.elapsed.as_secs_f64() + delta;
            for (index, sample) in block.samples.iter_mut().enumerate() {
                let time =
                    start + (index / block.channels.max(1)) as f64 / f64::from(block.sample_rate);
                let ratio = (time / self.duration.as_secs_f64()).clamp(0., 1.) as f32;
                *sample *= if outgoing {
                    (1. - ratio) * self.pcm_gain
                } else {
                    ratio
                };
            }
        }
    }
}

impl NativeAudioRegistry {
    pub(crate) fn tick_crossfades(&mut self, now: Instant) {
        let sources: Vec<_> = self.music_crossfades.keys().cloned().collect();
        for source in sources {
            if let Some(content) = self.ndi_content.get(&source).cloned() {
                self.update_music_crossfade(&source, &content, now);
            }
        }
    }

    pub(super) fn prepare_music_crossfade(&mut self, source: &str, content: &Value, now: Instant) {
        let Some(previous) = self.ndi_content.get(source) else {
            return;
        };
        let seconds = content["musicCrossfadeSeconds"].as_f64().unwrap_or(0.);
        if local_music_path(content).is_none() || seconds <= 0. {
            self.music_crossfades.remove(source);
            return;
        }
        if previous["audioSelectionId"] == content["audioSelectionId"]
            && previous["audioId"] == content["audioId"]
        {
            return;
        }
        let key = track_key(source, PlaybackLayer::Music);
        let native = self.tracks.remove(&key);
        let decoder = self.ndi_decoders.remove(&key);
        if native.is_none() && decoder.is_none() {
            return;
        }
        let remaining = native
            .as_ref()
            .and_then(|track| {
                track
                    .loop_limit
                    .or(if track.looping { None } else { track.duration })
                    .map(|duration| {
                        (
                            duration,
                            track
                                .player
                                .get_pos()
                                .saturating_sub(track.delay)
                                .as_secs_f64()
                                * f64::from(track.rate),
                        )
                    })
            })
            .or_else(|| {
                decoder.as_ref().and_then(|track| {
                    track
                        .loop_limit
                        .or(if track.looping { None } else { track.duration })
                        .map(|duration| (duration, track.position_seconds()))
                })
            })
            .map(|(duration, position)| (duration - position).max(0.))
            .unwrap_or(seconds);
        let duration = seconds.min(remaining);
        if duration <= 0.
            || playback_state(previous, PlaybackLayer::Music).status == PlaybackStatus::Paused
        {
            return;
        }
        let pcm_gain = self
            .music_crossfades
            .get(source)
            .map(MusicCrossfade::fraction)
            .unwrap_or(1.);
        self.music_crossfades.insert(
            source.into(),
            MusicCrossfade {
                native,
                decoder,
                routing: previous.get("musicAudioRouting").cloned(),
                pcm_gain,
                elapsed: Duration::ZERO,
                duration: Duration::from_secs_f64(duration),
                last_tick: now,
                paused: false,
            },
        );
    }

    pub(super) fn update_music_crossfade(&mut self, source: &str, content: &Value, now: Instant) {
        let Some(fade) = self.music_crossfades.get_mut(source) else {
            return;
        };
        if !fade.paused {
            fade.elapsed += now.saturating_duration_since(fade.last_tick);
        }
        fade.last_tick = now;
        let paused = playback_state(content, PlaybackLayer::Music).status == PlaybackStatus::Paused;
        let fraction = fade.fraction();
        let settings = normalize_audio_settings_value(&content["audioSettings"]);
        if let Some(track) = &mut fade.native {
            track.player.set_volume(
                gain_from_settings(&settings["main"]) * fade.pcm_gain * (1. - fraction),
            );
            if paused {
                track.player.pause();
            } else {
                track.player.play();
            }
        }
        if let Some(track) = &mut fade.decoder {
            track.status = if paused {
                PlaybackStatus::Paused
            } else {
                PlaybackStatus::Playing
            };
            if fade.paused != paused {
                track.next_block_at = Some(now);
            }
        }
        fade.paused = paused;
        if let Some(track) = self
            .tracks
            .get_mut(&track_key(source, PlaybackLayer::Music))
        {
            track
                .player
                .set_volume(gain_from_settings(&settings["main"]) * fraction);
        }
        if fade.elapsed >= fade.duration {
            self.music_crossfades.remove(source);
        }
    }

    pub(super) fn mix_crossfade_audio(
        &mut self,
        source: &str,
        media: &mut VecDeque<NativePcmBlock>,
        music: &mut VecDeque<NativePcmBlock>,
        now: Instant,
    ) -> Option<Vec<NativePcmBlock>> {
        let fade = self.music_crossfades.get_mut(source)?;
        let mut raw_outgoing = if let Some(track) = &mut fade.native {
            fill_ndi_pending(track);
            std::mem::take(&mut track.ndi_pending)
        } else if let Some(track) = &mut fade.decoder {
            fill_ndi_decoder_pending(track, now);
            std::mem::take(&mut track.pending)
        } else {
            return None;
        };
        let mut incoming = music.clone();
        let mut outgoing = raw_outgoing.clone();
        fade.scale(&mut incoming, false);
        if let Some((routing, gain)) = fade
            .routing
            .as_ref()
            .filter(|value| !value.is_null())
            .and_then(|value| asset_audio_routing(value).ok())
        {
            apply_asset_routing(&mut outgoing, &routing, gain);
        }
        fade.scale(&mut outgoing, true);
        let mut mixed = Vec::new();
        for _ in 0..NDI_MAX_BLOCKS_PER_DRAIN {
            let Some(block) = mix_ndi_window_layers(&mut [media, &mut incoming, &mut outgoing])
            else {
                break;
            };
            mixed.push(block);
        }
        restore_unscaled_queue(music, &incoming);
        restore_unscaled_queue(&mut raw_outgoing, &outgoing);
        if let Some(track) = &mut fade.native {
            track.ndi_pending = raw_outgoing;
        } else if let Some(track) = &mut fade.decoder {
            track.pending = raw_outgoing;
        }
        Some(mixed)
    }
}

// Keep queued samples unscaled so repeated drains and routing changes never apply a fade twice.
fn restore_unscaled_queue(raw: &mut VecDeque<NativePcmBlock>, consumed: &VecDeque<NativePcmBlock>) {
    raw.drain(..raw.len().saturating_sub(consumed.len()));
    for (raw, consumed) in raw.iter_mut().zip(consumed) {
        raw.offset_frames = consumed.offset_frames;
    }
}
