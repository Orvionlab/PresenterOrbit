use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use serde_json::{json, Value};

#[derive(Default)]
pub struct PropRuntime {
    deadlines: HashMap<String, Instant>,
}

pub fn auto_clear_duration(prop: &Value) -> Result<Option<Duration>, String> {
    match prop.get("autoClearSeconds") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let seconds = value
                .as_f64()
                .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
                .ok_or("Prop auto-clear duration must be finite and greater than zero.")?;
            Duration::try_from_secs_f64(seconds)
                .map(Some)
                .map_err(|_| "Prop auto-clear duration is out of range.".into())
        }
    }
}

impl PropRuntime {
    pub fn trigger(&mut self, id: &str, duration: Option<Duration>) -> Result<(), String> {
        self.trigger_at(id, duration, Instant::now())
    }

    fn trigger_at(
        &mut self,
        id: &str,
        duration: Option<Duration>,
        now: Instant,
    ) -> Result<(), String> {
        if let Some(duration) = duration {
            let deadline = now
                .checked_add(duration)
                .ok_or("Prop auto-clear duration is out of range.")?;
            self.deadlines.insert(id.to_string(), deadline);
        } else {
            self.deadlines.remove(id);
        }
        Ok(())
    }

    pub fn retain_active(&mut self, snapshot: &Value) {
        self.deadlines.retain(|id, _| {
            !snapshot["remotePropDeadlines"][id]
                .as_f64()
                .is_some_and(|deadline| deadline < 0.)
                && snapshot["livePropIds"]
                    .as_array()
                    .is_some_and(|ids| ids.iter().any(|value| value.as_str() == Some(id)))
        });
    }

    pub fn refresh(&mut self, snapshot: &Value) -> Result<Option<(Value, Value)>, String> {
        self.refresh_at(snapshot, Instant::now())
    }

    fn refresh_at(
        &mut self,
        snapshot: &Value,
        now: Instant,
    ) -> Result<Option<(Value, Value)>, String> {
        if !self.deadlines.values().any(|deadline| *deadline <= now) {
            return Ok(None);
        }
        let ids = snapshot["livePropIds"]
            .as_array()
            .ok_or("Live prop ids must be an array.")?;
        let remaining = ids
            .iter()
            .filter(|id| {
                !id.as_str()
                    .and_then(|id| self.deadlines.get(id))
                    .is_some_and(|deadline| *deadline <= now)
            })
            .cloned()
            .collect::<Vec<_>>();
        self.deadlines.retain(|_, deadline| *deadline > now);
        if remaining == *ids {
            return Ok(None);
        }
        let mut next = snapshot.clone();
        next["livePropIds"] = json!(remaining);
        let mut patch = json!({"livePropIds":remaining});
        if let Some(deadlines) = next["remotePropDeadlines"].as_object_mut() {
            deadlines.retain(|id, _| remaining.iter().any(|prop| prop == id.as_str()));
            patch["remotePropDeadlines"] = json!(deadlines);
        }
        Ok(Some((next, patch)))
    }
}
