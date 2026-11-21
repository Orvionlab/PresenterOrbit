use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashSet, time::Duration};

pub(crate) const MAX_PLAYBACK_RATE: f64 = 16.0;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TimelineTransport {
    pub mode: String,
    pub item_id: String,
    pub generation: String,
    pub position_seconds: f64,
    pub playback_rate: f64,
    #[serde(deserialize_with = "deserialize_started_at")]
    pub started_at_ms: Option<i64>,
}

fn deserialize_started_at<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<i64>, D::Error> {
    Option::<i64>::deserialize(deserializer)
}

pub(crate) fn validate(timeline: &Value, slides: &Value) -> Result<(), String> {
    let cues = timeline
        .get("cues")
        .and_then(Value::as_array)
        .ok_or("Timeline cues must be an array.")?;
    let duration = timeline
        .get("durationSeconds")
        .and_then(Value::as_f64)
        .ok_or("Timeline duration is required.")?;
    if !duration.is_finite() || duration <= 0.0 || duration > 86400.0 || cues.len() > 4096 {
        return Err("Timeline duration or cue count is invalid.".into());
    }
    if timeline.get("loop").and_then(Value::as_bool).is_none() {
        return Err("Timeline loop must be a boolean.".into());
    }
    validate_timecode_and_soundtrack_fields(timeline)?;
    let slides = slides
        .as_array()
        .ok_or("Timeline presentation slides must be an array.")?;
    let mut ids = HashSet::new();
    for cue in cues {
        let id = cue
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("Timeline cue ID is required.")?;
        let time = cue
            .get("timeSeconds")
            .and_then(Value::as_f64)
            .ok_or("Timeline cue time is required.")?;
        if !ids.insert(id) || !time.is_finite() || time < 0.0 || time >= duration {
            return Err("Timeline cue identity or time is invalid.".into());
        }
        if cue
            .get("cueInstanceId")
            .is_some_and(|value| value.as_str().is_none_or(str::is_empty))
        {
            return Err("Timeline cue occurrence must be a nonempty string.".into());
        }
        let target = cue.get("cueId").and_then(Value::as_str);
        if let Some(target) = target {
            if !slides.iter().any(|slide| {
                slide.get("id").and_then(Value::as_str) == Some(target)
                    || slide.get("sourceCueId").and_then(Value::as_str) == Some(target)
            }) {
                return Err("Timeline cue target is missing.".into());
            }
        }
        if target.is_none() && !cue.get("action").is_some_and(Value::is_object) {
            return Err("Timeline cue needs a slide or action.".into());
        }
    }
    Ok(())
}

const TIMELINE_FIELDS: [&str; 6] = [
    "cues",
    "durationSeconds",
    "loop",
    "soundtrackMediaId",
    "timecodeEnabled",
    "timecodeOffsetSeconds",
];

fn validate_timecode_and_soundtrack_fields(timeline: &Value) -> Result<(), String> {
    if let Some(field) = timeline.as_object().and_then(|fields| {
        fields
            .keys()
            .find(|key| !TIMELINE_FIELDS.contains(&key.as_str()))
    }) {
        return Err(format!("Timeline field '{field}' is not supported."));
    }
    let enabled = match timeline.get("timecodeEnabled") {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or("Timeline timecode enabled must be a boolean.")?,
    };
    match timeline.get("timecodeOffsetSeconds") {
        None if enabled => return Err("Timeline timecode needs a start offset.".into()),
        None => {}
        Some(value) => {
            if !value
                .as_f64()
                .is_some_and(|offset| offset.is_finite() && (0.0..86400.0).contains(&offset))
            {
                return Err("Timeline timecode offset must be within 24 hours.".into());
            }
        }
    }
    if timeline
        .get("soundtrackMediaId")
        .is_some_and(|value| value.as_str().is_none_or(str::is_empty))
    {
        return Err("Timeline soundtrack must identify a media asset.".into());
    }
    Ok(())
}

/// The soundtrack must be an audio asset of the workspace media library.
pub(crate) fn validate_soundtrack(timeline: &Value, media: &[Value]) -> Result<(), String> {
    let Some(id) = timeline.get("soundtrackMediaId").and_then(Value::as_str) else {
        return Ok(());
    };
    if media.iter().any(|asset| {
        asset.get("id").and_then(Value::as_str) == Some(id)
            && asset.get("type").and_then(Value::as_str) == Some("audio")
    }) {
        Ok(())
    } else {
        Err("Timeline soundtrack audio asset is unavailable.".into())
    }
}

pub(crate) fn validate_occurrences(timeline: &Value, item: &Value) -> Result<(), String> {
    for cue in timeline["cues"]
        .as_array()
        .ok_or("Timeline cues must be an array.")?
    {
        if let Some(instance) = cue.get("cueInstanceId").and_then(Value::as_str) {
            let slides = crate::output_arrangement::resolve_live_arrangement_slides(
                item,
                Some(instance),
                None,
            )
            .ok_or("Timeline presentation slides must be an array.")?;
            if !slides.iter().any(|slide| {
                slide["arrangementInstanceId"].as_str() == Some(instance)
                    && slide["id"] == cue["cueId"]
            }) {
                return Err("Timeline cue occurrence is unavailable.".into());
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Playhead {
    pub generation: String,
    pub running: bool,
    pub position: f64,
    pub playback_rate: f64,
}

#[derive(Default)]
pub(crate) struct TimelineClock {
    transport: Option<TimelineTransport>,
    anchor: Duration,
    timeline: Value,
    emitted: HashSet<String>,
    retired: HashSet<String>,
    last_tick_position: Option<f64>,
    last_direction: i8,
}

impl TimelineClock {
    pub(crate) fn update(
        &mut self,
        transport: Option<TimelineTransport>,
        timeline: Value,
        wall_ms: i64,
        now: Duration,
    ) {
        let same = self
            .transport
            .as_ref()
            .zip(transport.as_ref())
            .is_some_and(|(a, b)| a.item_id == b.item_id && a.generation == b.generation);
        if !same {
            if let Some(previous) = &self.transport {
                self.retired.insert(previous.generation.clone());
            }
            self.emitted.clear();
            self.last_tick_position = None;
            self.last_direction = transport
                .as_ref()
                .map_or(1, |transport| transport.playback_rate.signum() as i8);
        }
        if transport
            .as_ref()
            .is_some_and(|transport| self.retired.contains(&transport.generation))
        {
            self.transport = transport.map(|mut transport| {
                transport.started_at_ms = None;
                transport
            });
            return;
        }
        let same_anchor = self
            .transport
            .as_ref()
            .zip(transport.as_ref())
            .is_some_and(|(a, b)| a.started_at_ms == b.started_at_ms);
        if !same_anchor || !same {
            self.anchor = transport
                .as_ref()
                .and_then(|transport| transport.started_at_ms)
                .map_or(now, |start| {
                    now.saturating_sub(Duration::from_millis(
                        wall_ms.saturating_sub(start).max(0) as u64
                    ))
                });
        }
        self.transport = transport;
        self.timeline = timeline;
    }

    /// The position the soundtrack must follow. Retired generations have no
    /// playhead, so their audio is stopped rather than paused.
    pub(crate) fn playhead(&self, now: Duration) -> Option<Playhead> {
        let transport = self.transport.as_ref()?;
        if !matches!(transport.mode.as_str(), "playback" | "recording")
            || self.retired.contains(&transport.generation)
        {
            return None;
        }
        let running = transport.started_at_ms.is_some();
        let elapsed = if running {
            now.saturating_sub(self.anchor).as_secs_f64()
        } else {
            0.0
        };
        let mut position = transport.position_seconds + elapsed * transport.playback_rate;
        if let Some(duration) = self.timeline["durationSeconds"]
            .as_f64()
            .filter(|duration| *duration > 0.0)
        {
            if self.timeline["loop"].as_bool() == Some(true) {
                position = position.rem_euclid(duration);
            } else if position >= duration || position <= 0.0 && transport.playback_rate < 0.0 {
                return Some(Playhead {
                    generation: transport.generation.clone(),
                    running: false,
                    position: position.clamp(0.0, duration),
                    playback_rate: transport.playback_rate,
                });
            }
        }
        Some(Playhead {
            generation: transport.generation.clone(),
            running,
            position,
            playback_rate: transport.playback_rate,
        })
    }

    pub(crate) fn active(&self) -> bool {
        self.transport.is_some()
    }

    pub(crate) fn running_generation(&self) -> Option<&str> {
        self.transport
            .as_ref()
            .filter(|transport| transport.mode == "playback" && transport.started_at_ms.is_some())
            .map(|transport| transport.generation.as_str())
    }

    pub(crate) fn tick(&mut self, now: Duration) -> Vec<Value> {
        let Some(transport) = &self.transport else {
            return Vec::new();
        };
        if transport.mode != "playback" || transport.started_at_ms.is_none() {
            return Vec::new();
        }
        let mut position = transport.position_seconds
            + now.saturating_sub(self.anchor).as_secs_f64() * transport.playback_rate;
        let duration = self.timeline["durationSeconds"].as_f64().unwrap();
        let repeat = self.timeline["loop"].as_bool().unwrap();
        if !repeat {
            position = position.clamp(0.0, duration);
        }
        let direction = if transport.playback_rate < 0.0 { -1 } else { 1 };
        if direction != self.last_direction {
            self.emitted.clear();
            self.last_direction = direction;
        }
        let from = self
            .last_tick_position
            .unwrap_or(transport.position_seconds);
        let mut cues: Vec<_> = self.timeline["cues"].as_array().unwrap().iter().collect();
        if cues.is_empty() {
            self.last_tick_position = Some(position);
            return Vec::new();
        }
        cues.sort_by(|a, b| {
            a["timeSeconds"]
                .as_f64()
                .unwrap()
                .total_cmp(&b["timeSeconds"].as_f64().unwrap())
        });
        let mut due = Vec::new();
        let first_cycle = (from / duration).floor() as i64;
        let last_cycle = (position / duration).floor() as i64;
        let cycles: Box<dyn Iterator<Item = i64>> = if !repeat {
            Box::new(std::iter::once(0))
        } else if direction > 0 {
            Box::new(first_cycle..=last_cycle)
        } else {
            Box::new((last_cycle..=first_cycle).rev())
        };
        let mut cursor = position;
        let mut capped = false;
        'cycles: for current_cycle in cycles {
            let cycle_cues: Box<dyn Iterator<Item = &&Value>> = if direction > 0 {
                Box::new(cues.iter())
            } else {
                Box::new(cues.iter().rev())
            };
            for cue in cycle_cues {
                let time = cue["timeSeconds"].as_f64().unwrap() + current_cycle as f64 * duration;
                let crossed = if direction > 0 {
                    time >= from && time <= position
                } else {
                    time <= from && time >= position
                };
                if !crossed {
                    continue;
                }
                let key = format!("{current_cycle}:{}", cue["id"].as_str().unwrap());
                if self.emitted.contains(&key) {
                    continue;
                }
                if due.len() == 4096 {
                    capped = true;
                    break 'cycles;
                }
                self.emitted.insert(key);
                due.push((*cue).clone());
                cursor = time;
            }
        }
        let retained_cycle = if capped { cursor } else { position };
        let retained_cycle = (retained_cycle / duration).floor() as i64;
        self.emitted
            .retain(|key| key.starts_with(&format!("{retained_cycle}:")));
        self.last_tick_position = Some(if capped { cursor } else { position });
        due
    }
}
