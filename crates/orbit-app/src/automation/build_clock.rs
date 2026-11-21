//! `synchronizeLiveBuildClock`: retain valid occurrence clocks, clear and rewind explicitly.
use serde_json::{json, Value};
#[derive(Default)]
pub struct Clock {
    key: String,
    step: u64,
}
impl Clock {
    pub fn synchronize(&mut self, live: &Value, now: f64) -> Option<Value> {
        let key = super::due::live_key(live);
        let step = live["liveBuildStep"].as_u64().unwrap_or(0);
        let clock = &live["liveBuildStepStartedAtMsByStep"];
        let started = live["timelineStartedAtMs"].as_f64();
        let epoch = |v: Option<f64>| {
            v.is_some_and(|v| v >= 0. && v.fract() == 0. && v <= 9007199254740991.)
        };
        let mut patch = None;
        if key.is_empty() {
            if clock.as_object().is_some_and(|v| !v.is_empty()) {
                patch = Some(json!({"liveBuildStepStartedAtMsByStep":{}}));
            }
            self.step = 0;
        } else if key != self.key {
            if !(epoch(started)
                && clock["0"].as_f64() == started
                && epoch(clock[step.to_string()].as_f64()))
            {
                let at = now.max(started.unwrap_or(0.) + 1.) as i64;
                patch = Some(
                    json!({"timelineStartedAtMs":at,"liveBuildStep":0,"liveBuildStepStartedAtMsByStep":{"0":at}}),
                );
                self.step = 0;
            } else {
                self.step = step;
            }
        } else if step < self.step {
            let at = if step == 0 {
                now.max(started.unwrap_or(0.) + 1.)
            } else {
                started.unwrap_or(now)
            };
            let mut clock = json!({"0":at as i64});
            clock[step.to_string()] = json!(now as i64);
            patch = Some(json!({"liveBuildStepStartedAtMsByStep":clock}));
            if step == 0 {
                if let Some(patch) = patch.as_mut() {
                    patch["timelineStartedAtMs"] = json!(at as i64);
                    patch["liveBuildStepStartedAtMsByStep"]["0"] = json!(at as i64);
                }
            }
            self.step = step;
        } else {
            self.step = step;
        }
        self.key = key;
        patch
    }
}
