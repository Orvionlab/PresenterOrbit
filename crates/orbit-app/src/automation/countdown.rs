//! Timer-list values use React's running, paused and elapsed clock rules.
use serde_json::Value;
pub fn milliseconds(timer: &Value, run: &Value, now: f64) -> f64 {
    let elapsed = timer["type"] == "elapsed" || run["elapsed"] == true;
    let mut value = if run.is_object() {
        if run["paused"] == true {
            run["remaining"].as_f64().unwrap_or(0.)
        } else if elapsed {
            (now - run["end"].as_f64().unwrap_or(now)).max(0.)
        } else {
            run["end"].as_f64().unwrap_or(now) - now
        }
    } else if elapsed {
        timer["elapsedStartSeconds"].as_f64().unwrap_or(0.) * 1000.
    } else if timer["type"] == "countdown-to-time" {
        use chrono::TimeZone;
        chrono::Local
            .timestamp_millis_opt(now as i64)
            .single()
            .and_then(|current| current.date_naive().and_hms_opt(0, 0, 0))
            .and_then(|midnight| chrono::Local.from_local_datetime(&midnight).earliest())
            .map_or(0., |midnight| {
                let target = midnight.timestamp_millis() as f64
                    + timer["targetTimeSeconds"].as_f64().unwrap_or(0.) * 1000.;
                target - now + if target <= now { 86_400_000. } else { 0. }
            })
    } else {
        timer["durationSeconds"].as_f64().unwrap_or(0.) * 1000.
    };
    if timer["allowsOverrun"] != true {
        if timer["type"] == "elapsed" {
            if let Some(end) = timer["elapsedEndSeconds"].as_f64().filter(|end| *end > 0.) {
                value = value.min(end * 1000.);
            }
        }
        value = value.max(0.);
    }
    value
}
pub fn display(milliseconds: f64) -> String {
    let total = (milliseconds.abs() / 1000.).floor() as u64;
    format!(
        "{}{}{}:{:02}",
        if milliseconds < 0. { "\u{2212}" } else { "" },
        if total >= 3600 {
            format!("{}:", total / 3600)
        } else {
            String::new()
        },
        total / 60 % 60,
        total % 60
    )
}
