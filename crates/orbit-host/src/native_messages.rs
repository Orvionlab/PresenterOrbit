use crate::native_timers::TimerRegistry;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct MessageRuntime {
    instance: Option<String>,
    deadline: Option<Instant>,
}

impl MessageRuntime {
    pub fn refresh(
        &mut self,
        snapshot: &Value,
        timers: &mut TimerRegistry,
    ) -> Result<Option<(Value, Value)>, String> {
        self.refresh_at(snapshot, timers, Instant::now())
    }

    fn refresh_at(
        &mut self,
        snapshot: &Value,
        timers: &mut TimerRegistry,
        now: Instant,
    ) -> Result<Option<(Value, Value)>, String> {
        if snapshot
            .get("activeMessageId")
            .and_then(Value::as_str)
            .is_none()
        {
            self.instance = None;
            self.deadline = None;
            return Ok(None);
        }
        let instance = snapshot["activeMessageInstanceId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Active message instance is required.")?;
        if self.instance.as_deref() != Some(instance) {
            let message = snapshot
                .get("activeMessageDefinition")
                .filter(|value| value.is_object())
                .ok_or("Active message definition is required.")?;
            // Validate every token before starting any timer for this display instance.
            refresh(snapshot, timers)?;
            let policy = message.get("clearType").and_then(Value::as_str);
            if !matches!(
                policy,
                None | Some("manual" | "after-time" | "after-timers")
            ) {
                return Err("Unsupported message clear policy.".into());
            }
            let deadline = if policy == Some("after-time")
                || (policy.is_none() && message.get("clearAfterSeconds").is_some())
            {
                let seconds = message["clearAfterSeconds"]
                    .as_f64()
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .ok_or("Message clear duration must be finite and greater than zero.")?;
                let duration = Duration::try_from_secs_f64(seconds)
                    .map_err(|_| "Message clear duration is out of range.")?;
                Some(
                    now.checked_add(duration)
                        .ok_or("Message clear duration is out of range.")?,
                )
            } else {
                None
            };
            let mut started = std::collections::HashSet::new();
            let mut next_timers = timers.clone();
            let mut runs = snapshot["countdownTimerRuns"]
                .as_object()
                .cloned()
                .ok_or("Message timer runs must be an object.")?;
            if let Some(tokens) = message["tokens"].as_array() {
                for token in tokens.iter().filter(|token| token["type"] == "timer") {
                    let id = token["timerId"]
                        .as_str()
                        .ok_or("Message timer target is required.")?;
                    if started.insert(id) {
                        let timer = snapshot["countdownTimers"]
                            .as_array()
                            .and_then(|values| values.iter().find(|timer| timer["id"] == id))
                            .ok_or_else(|| format!("Message timer has been retired: {id}"))?;
                        if timer["type"] == "elapsed" {
                            next_timers.control(timer, "reset-to-start", None, 0.0)?;
                        }
                        runs.insert(
                            id.into(),
                            next_timers.control(timer, "reset-and-start", None, 0.0)?,
                        );
                    }
                }
            }
            *timers = next_timers;
            self.instance = Some(instance.into());
            self.deadline = deadline;
            if !started.is_empty() {
                let (mut next, mut patch) =
                    refresh(snapshot, timers)?.unwrap_or_else(|| (snapshot.clone(), json!({})));
                next["countdownTimerRuns"] = json!(runs);
                patch["countdownTimerRuns"] = next["countdownTimerRuns"].clone();
                return Ok(Some((next, patch)));
            }
        }
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            let patch = json!({"activeMessageId":null,"activeMessageInstanceId":null,"activeMessageText":""});
            let mut next = snapshot.clone();
            for (key, value) in patch.as_object().expect("object patch") {
                next[key] = value.clone();
            }
            self.instance = None;
            self.deadline = None;
            return Ok(Some((next, patch)));
        }
        refresh(snapshot, timers)
    }
}

pub fn refresh(snapshot: &Value, timers: &TimerRegistry) -> Result<Option<(Value, Value)>, String> {
    let Some(id) = snapshot.get("activeMessageId").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(message) = snapshot
        .get("activeMessageDefinition")
        .filter(|v| !v.is_null())
    else {
        return Ok(None);
    };
    if message.get("id").and_then(Value::as_str) != Some(id) {
        return Err("Active message definition does not match its id.".into());
    }
    let mut text = message
        .get("text")
        .and_then(Value::as_str)
        .ok_or("Message text is required.")?
        .to_string();
    let mut all_complete = true;
    let mut timer_count = 0;
    if let Some(tokens) = message.get("tokens").and_then(Value::as_array) {
        for token in tokens {
            let name = token
                .get("name")
                .and_then(Value::as_str)
                .ok_or("Message token name is required.")?;
            let value = match token.get("type").and_then(Value::as_str) {
                Some("clock") => chrono::Local::now().format("%H:%M:%S").to_string(),
                Some("text") => message
                    .get("tokenValues")
                    .and_then(|v| v.get(name))
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("Missing message token value: {name}"))?
                    .to_string(),
                Some("timer") => {
                    let timer_id = token
                        .get("timerId")
                        .and_then(Value::as_str)
                        .ok_or("Message timer target is required.")?;
                    let timer = snapshot
                        .get("countdownTimers")
                        .and_then(Value::as_array)
                        .and_then(|values| {
                            values
                                .iter()
                                .find(|v| v.get("id").and_then(Value::as_str) == Some(timer_id))
                        })
                        .ok_or_else(|| format!("Message timer has been retired: {timer_id}"))?;
                    let (millis, complete) = timers.message_value(timer)?;
                    timer_count += 1;
                    all_complete &= complete;
                    format_timer(
                        if timer.get("allowsOverrun").and_then(Value::as_bool) == Some(true) {
                            millis
                        } else {
                            millis.max(0)
                        },
                    )
                }
                _ => return Err("Unsupported message token type.".into()),
            };
            text = text.replace(&format!("{{{name}}}"), &value);
        }
    }
    if let Some(values) = message.get("tokenValues").and_then(Value::as_object) {
        for (name, value) in values {
            if let Some(value) = value.as_str() {
                text = text.replace(&format!("{{{name}}}"), value);
            }
        }
    }
    if text.contains('{') && text.contains('}') {
        return Err("Message contains an unresolved token.".into());
    }
    let after_timers = message.get("clearType").and_then(Value::as_str) == Some("after-timers");
    if after_timers && timer_count == 0 {
        return Err("Clear after timers requires a timer token.".into());
    }
    let patch = if after_timers && all_complete {
        json!({"activeMessageId": null, "activeMessageInstanceId": null, "activeMessageText": ""})
    } else {
        if snapshot.get("activeMessageText").and_then(Value::as_str) == Some(&text) {
            return Ok(None);
        }
        json!({"activeMessageText": text})
    };
    let mut next = snapshot.clone();
    for (key, value) in patch.as_object().expect("object patch") {
        next[key] = value.clone();
    }
    Ok(Some((next, patch)))
}

fn format_timer(milliseconds: i64) -> String {
    let seconds = milliseconds.unsigned_abs() / 1000;
    let hours = seconds / 3600;
    let minutes = seconds / 60 % 60;
    format!(
        "{}{}{minutes}:{:02}",
        if milliseconds < 0 { "−" } else { "" },
        if hours > 0 {
            format!("{hours}:")
        } else {
            String::new()
        },
        seconds % 60
    )
}
