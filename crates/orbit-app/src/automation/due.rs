//! `automation-due` events: which ones may act, and on what. The host scheduler decides when something is due;
//! this decides whether it still applies to what is on air (`handleNativeDueEvent`, `matchesEventContext`).

use std::time::{Duration, Instant};

use serde_json::Value;

/// How long after taking over a running output the events of its on-air slide are ignored. A fresh scheduler
/// that adopts a slide already on air finds every cue action and timeline cue of that slide due at once, and
/// firing them would repeat, or run late, what was done before the interface restarted.
pub const ADOPTION_WINDOW: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct Adoption {
    adopted: Option<(String, Instant)>,
}

impl Adoption {
    /// The output process reported a slide on air that this process did not put there.
    pub fn adopt(&mut self, live_key: &str, now: Instant) {
        self.adopted = (!live_key.is_empty()).then(|| (live_key.to_string(), now));
    }

    fn overdue(&self, event: &Value, now: Instant) -> bool {
        let Some((key, at)) = &self.adopted else {
            return false;
        };
        matches!(event["source"].as_str(), Some("cue" | "timeline"))
            && event["kind"] != "build"
            && event["liveKey"] == key.as_str()
            && now.saturating_duration_since(*at) < ADOPTION_WINDOW
    }
}

/// `getLiveCueKey`.
pub fn live_key(live: &Value) -> String {
    match (live["liveItemId"].as_str(), live["liveSlideId"].as_str()) {
        (Some(item), Some(slide)) if !item.is_empty() && !slide.is_empty() => {
            format!(
                "{item}:{slide}:{}",
                live["liveCueInstanceId"].as_str().unwrap_or("")
            )
        }
        _ => String::new(),
    }
}

#[derive(Debug, PartialEq)]
pub enum Due {
    Ignore,
    StatePatch { revision: u64, patch: Value },
    Actions { actions: Vec<Value>, depth: u32 },
    Build { step: u64, started_at_ms: i64 },
    Cue { slide_id: String, instance: Value },
}

/// `matchesEventContext`: a timeline event needs the playing transport of its generation, and any other event
/// must name the cue that is still on air.
fn matches_context(event: &Value, live: &Value) -> bool {
    let transport = &live["timelineTransport"];
    if event["source"] == "timeline" && transport.is_object() {
        return transport["mode"] == "playback"
            && !transport["startedAtMs"].is_null()
            && event["timelineGeneration"]
                .as_str()
                .is_some_and(|generation| {
                    !generation.is_empty() && transport["generation"] == generation
                });
    }
    if event.get("timelineGeneration").is_some() {
        return false;
    }
    match event["liveKey"].as_str() {
        Some(key) if !key.is_empty() => key == live_key(live),
        _ => true,
    }
}

pub fn classify(
    event: &Value,
    session: &str,
    live: &Value,
    adoption: &Adoption,
    now: Instant,
) -> Result<Due, String> {
    if event["sessionId"] != session
        || !matches_context(event, live)
        || adoption.overdue(event, now)
    {
        return Ok(Due::Ignore);
    }
    match event["kind"].as_str().unwrap_or("") {
        "statePatch" => Ok(Due::StatePatch {
            revision: event["revision"]
                .as_u64()
                .ok_or("A state patch has no revision.")?,
            patch: event["patch"].clone(),
        }),
        "actions" => Ok(Due::Actions {
            actions: event["actions"]
                .as_array()
                .cloned()
                .ok_or("The due actions are not a list.")?,
            depth: if event["source"] == "delay" {
                event["depth"].as_u64().unwrap_or(0) as u32
            } else {
                0
            },
        }),
        "build" => {
            let (step, started) = (event["step"].as_u64(), event["startedAtMs"].as_i64());
            let current = live["liveBuildStep"].as_u64().unwrap_or(0);
            match (step, started) {
                (Some(step), Some(started_at_ms)) if step > current && started_at_ms > 0 => {
                    Ok(Due::Build {
                        step,
                        started_at_ms,
                    })
                }
                _ => Ok(Due::Ignore),
            }
        }
        "cue" => Ok(
            match event["targetSlideId"].as_str().filter(|id| !id.is_empty()) {
                Some(slide) => Due::Cue {
                    slide_id: slide.into(),
                    instance: event
                        .get("targetCueInstanceId")
                        .cloned()
                        .unwrap_or(Value::Null),
                },
                None => Due::Ignore,
            },
        ),
        other => Err(format!("Unknown automation event kind {other:?}.")),
    }
}
