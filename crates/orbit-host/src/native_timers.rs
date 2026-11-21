use std::{collections::HashMap, time::Instant};

use chrono::{Local, TimeZone};
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct TimerRegistry {
    runs: HashMap<String, NativeTimerRun>,
}

#[derive(Debug, Clone)]
struct NativeTimerRun {
    anchor: Instant,
    elapsed: bool,
    running: bool,
    value_at_anchor: i64,
}

impl TimerRegistry {
    pub fn new() -> Self {
        Self {
            runs: HashMap::new(),
        }
    }

    pub fn control(
        &mut self,
        timer: &Value,
        action: &str,
        current_run: Option<&Value>,
        increment_seconds: f64,
    ) -> Result<Value, String> {
        let id = timer
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "Timer id is required.".to_string())?;
        if action == "remove" {
            self.runs.remove(id);
            return Ok(Value::Null);
        }
        let elapsed = timer.get("type").and_then(Value::as_str) == Some("elapsed");
        let now = Instant::now();
        let now_epoch = chrono::Utc::now().timestamp_millis();
        if !self.runs.contains_key(id) {
            if let Some(run) = current_run {
                self.runs.insert(
                    id.to_string(),
                    NativeTimerRun::from_wire(run, elapsed, now, now_epoch),
                );
            }
        }
        let existing = self.runs.get(id).cloned();
        let next = if elapsed {
            control_elapsed_timer(timer, action, existing, increment_seconds, now)
        } else {
            control_countdown_timer(timer, action, existing, increment_seconds, now)
        };
        let Some(next) = next else {
            return Ok(Value::Null);
        };
        let wire = next.to_wire(now, now_epoch);
        self.runs.insert(id.to_string(), next);
        Ok(wire)
    }

    /// Drop the in-memory run for a definition after its persisted mutation
    /// has committed. Definition writes own the ordering; this helper keeps a
    /// failed write from accidentally clearing a still-valid running timer.
    pub fn remove(&mut self, id: &str) -> bool {
        self.runs.remove(id).is_some()
    }

    pub fn clear(&mut self) {
        self.runs.clear();
    }

    pub fn message_value(&self, timer: &Value) -> Result<(i64, bool), String> {
        let id = timer
            .get("id")
            .and_then(Value::as_str)
            .ok_or("Timer id is required.")?;
        let Some(run) = self.runs.get(id) else {
            let initial = if timer.get("type").and_then(Value::as_str) == Some("elapsed") {
                json_seconds(timer.get("elapsedStartSeconds"))
            } else {
                timer_initial_milliseconds(timer)
            };
            return Ok((initial, false));
        };
        let value = run.current_value(Instant::now());
        let complete = if run.elapsed {
            timer
                .get("elapsedEndSeconds")
                .and_then(Value::as_f64)
                .is_some_and(|end| value >= seconds_millis(end))
        } else {
            value <= 0
        };
        Ok((value, run.running && complete))
    }
}

impl NativeTimerRun {
    fn from_wire(run: &Value, elapsed: bool, anchor: Instant, now_epoch: i64) -> Self {
        let running = run.get("paused").and_then(Value::as_bool) != Some(true);
        let remaining = json_i64(run.get("remaining")).unwrap_or(0);
        let end = json_i64(run.get("end")).unwrap_or(now_epoch);
        let value_at_anchor = if running {
            if elapsed || run.get("elapsed").and_then(Value::as_bool) == Some(true) {
                now_epoch.saturating_sub(end).max(0)
            } else {
                end.saturating_sub(now_epoch)
            }
        } else {
            remaining
        };
        Self {
            anchor,
            elapsed,
            running,
            value_at_anchor,
        }
    }

    fn current_value(&self, now: Instant) -> i64 {
        if !self.running {
            return self.value_at_anchor;
        }
        let elapsed_millis =
            i64::try_from(now.duration_since(self.anchor).as_millis()).unwrap_or(i64::MAX);
        if self.elapsed {
            self.value_at_anchor.saturating_add(elapsed_millis)
        } else {
            self.value_at_anchor.saturating_sub(elapsed_millis)
        }
    }

    fn with_value(&self, value: i64, running: bool, anchor: Instant) -> Self {
        Self {
            anchor,
            elapsed: self.elapsed,
            running,
            value_at_anchor: value,
        }
    }

    fn to_wire(&self, now: Instant, now_epoch: i64) -> Value {
        let remaining = self.current_value(now);
        let end = if self.elapsed {
            now_epoch.saturating_sub(remaining)
        } else {
            now_epoch.saturating_add(remaining)
        };
        let mut value = json!({
            "end": end,
            "paused": !self.running,
            "remaining": remaining,
        });
        if self.elapsed {
            value["elapsed"] = json!(true);
        }
        value
    }
}

fn control_elapsed_timer(
    timer: &Value,
    action: &str,
    existing: Option<NativeTimerRun>,
    increment_seconds: f64,
    now: Instant,
) -> Option<NativeTimerRun> {
    if action == "stop" && existing.is_none() {
        return None;
    }
    let initial = json_seconds(timer.get("elapsedStartSeconds"));
    let current = existing
        .as_ref()
        .map(|run| run.current_value(now))
        .unwrap_or(if action == "toggle" || action == "reset-to-start" {
            initial
        } else {
            0
        });
    let base = existing.unwrap_or(NativeTimerRun {
        anchor: now,
        elapsed: true,
        running: false,
        value_at_anchor: current,
    });
    match action {
        "stop" => Some(base.with_value(current, false, now)),
        "reset" | "stop-and-reset" => Some(base.with_value(0, false, now)),
        "reset-to-start" => Some(base.with_value(initial, false, now)),
        "reset-and-start" => Some(base.with_value(initial, true, now)),
        "toggle" => Some(base.with_value(current, !base.running, now)),
        "increment" => Some(base.with_value(
            current.saturating_add(seconds_millis(increment_seconds)),
            true,
            now,
        )),
        _ => Some(base.with_value(current, true, now)),
    }
}

fn control_countdown_timer(
    timer: &Value,
    action: &str,
    existing: Option<NativeTimerRun>,
    increment_seconds: f64,
    now: Instant,
) -> Option<NativeTimerRun> {
    if action == "stop" && existing.is_none() {
        return None;
    }
    let initial = timer_initial_milliseconds(timer);
    let current = existing
        .as_ref()
        .map(|run| run.current_value(now))
        .unwrap_or(0);
    let base = existing.unwrap_or(NativeTimerRun {
        anchor: now,
        elapsed: false,
        running: false,
        value_at_anchor: current,
    });
    let automation_remaining = if base.running {
        current.max(0)
    } else {
        current
    };
    match action {
        "stop" => Some(base.with_value(automation_remaining, false, now)),
        "reset" | "stop-and-reset" | "reset-to-start" => Some(base.with_value(initial, false, now)),
        "reset-and-start" => Some(base.with_value(initial, true, now)),
        "toggle" => Some(base.with_value(
            if current == 0 { initial } else { current },
            !base.running,
            now,
        )),
        "increment" => {
            let incremented =
                automation_remaining.saturating_add(seconds_millis(increment_seconds));
            Some(base.with_value(
                if incremented == 0 {
                    initial
                } else {
                    incremented
                },
                true,
                now,
            ))
        }
        _ => Some(base.with_value(
            if automation_remaining == 0 {
                initial
            } else {
                automation_remaining
            },
            true,
            now,
        )),
    }
}

fn timer_initial_milliseconds(timer: &Value) -> i64 {
    if timer.get("type").and_then(Value::as_str) != Some("countdown-to-time") {
        return json_seconds(timer.get("durationSeconds"));
    }
    let Some(target_seconds) = timer
        .get("targetTimeSeconds")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
    else {
        return json_seconds(timer.get("durationSeconds"));
    };
    let now = Local::now();
    let Some(midnight) = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Local.from_local_datetime(&value).earliest())
    else {
        return json_seconds(timer.get("durationSeconds"));
    };
    let target = midnight + chrono::Duration::milliseconds(seconds_millis(target_seconds));
    let target = if target <= now {
        target + chrono::Duration::days(1)
    } else {
        target
    };
    target
        .timestamp_millis()
        .saturating_sub(now.timestamp_millis())
}

fn json_seconds(value: Option<&Value>) -> i64 {
    value
        .and_then(Value::as_f64)
        .map(seconds_millis)
        .unwrap_or(0)
}

fn seconds_millis(value: f64) -> i64 {
    if value.is_finite() {
        (value * 1000.0) as i64
    } else {
        0
    }
}

fn json_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| {
                value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .map(|value| value as i64)
            })
    })
}
