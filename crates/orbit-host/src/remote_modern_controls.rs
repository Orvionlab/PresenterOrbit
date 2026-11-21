//! ProPresenter Remote endpoints backed by Orbit's accepted status and catalogs.

use chrono::TimeZone;
use serde_json::{json, Map, Value};

use super::super::{data, Backend};
use super::catalog::{identity, matches_id};

pub(super) fn handle(
    backend: &dyn Backend,
    method: &str,
    parts: &[&str],
    body: Option<&Value>,
) -> Option<Result<Value, String>> {
    Some(match (method, parts) {
        ("GET", ["v1", "timers"]) => timers(backend),
        ("GET", ["v1", "timers", "current"]) => current_timers(backend),
        ("GET", ["v1", "timers", operation]) => timer_operation(backend, operation),
        ("GET", ["v1", "timer", "system_time"]) => system_time(),
        ("GET", ["v1", "timer", "video_countdown"]) => video_countdown(backend),
        (_, ["v1", "timer", "system_time" | "video_countdown", ..]) => {
            Err("The requested special timer operation is unavailable.".into())
        }
        ("GET", ["v1", "timer", selector, "increment", seconds]) => {
            timer_increment(backend, selector, seconds)
        }
        ("GET", ["v1", "timer", selector, operation]) => {
            timer_operation_one(backend, selector, operation)
        }
        ("GET", ["v1", "timer", selector]) => timer(backend, selector),
        ("PUT", ["v1", "timer", selector]) => update_timer(backend, selector, body),
        ("GET", ["v1", "macros"]) => macros(backend),
        ("GET", ["v1", "macro_collections"]) => macro_collections(backend),
        ("GET", ["v1", "macro_collection", selector]) => macro_collection(backend, selector),
        ("GET", ["v1", "macro", selector]) => macro_details(backend, selector),
        ("GET", ["v1", "macro", selector, "trigger"]) => trigger_catalog_target(backend, "macros", "macro", selector),
        ("GET", ["v1", "clear", "groups"]) => clear_groups(backend),
        ("GET", ["v1", "clear", "group", selector]) => clear_group(backend, selector),
        ("GET", ["v1", "clear", "group", selector, "trigger"]) => trigger_catalog_target(backend, "clearGroups", "clearGroup", selector),
        ("GET", ["v1", "clear", "layer", layer]) => clear_layer(backend, layer),
        ("GET", ["v1", "status", "layers"]) => status_layers(backend),
        ("GET", ["v1", "stage", "screens"]) => stage_screens(backend),
        ("GET", ["v1", "stage", "layouts"]) => stage_layouts(backend),
        ("GET", ["v1", "stage", "layout_map"]) => stage_layout_map(backend),
        ("PUT", ["v1", "stage", "layout_map"]) => update_stage_layout_map(backend, body),
        ("GET", ["v1", "stage", "screen", screen, "layout"]) => stage_screen_layout(backend, screen),
        ("GET", ["v1", "stage", "screen", screen, "layout", layout]) => set_stage_screen_layout(backend, screen, layout),
        ("GET", ["v1", "stage", "message"]) => stage_message(backend),
        ("PUT", ["v1", "stage", "message"]) => set_stage_message(backend, body),
        ("DELETE", ["v1", "stage", "message"]) => hide_stage_message(backend),
        ("GET", ["v1", "looks"]) => looks(backend),
        ("GET", ["v1", "look", "current"]) => current_look(backend),
        ("GET", ["v1", "look", selector]) => look(backend, selector),
        ("GET", ["v1", "look", selector, "trigger"]) => trigger_catalog_target(backend, "audienceLooks", "audienceLook", selector),
        _ if known_but_unsupported(method, parts) => Err("This ProPresenter Remote endpoint is not available from the current Orbit data and command model.".into()),
        _ => return None,
    })
}

fn known_but_unsupported(method: &str, parts: &[&str]) -> bool {
    match parts {
        ["v1", "timers"] => method != "GET",
        ["v1", "timer", "system_time" | "video_countdown"] => method != "GET",
        ["v1", "timer", _] => !matches!(method, "GET" | "PUT"),
        ["v1", "timer", _, "increment", _] => method != "GET",
        ["v1", "timer", _, _] => method != "GET",
        ["v1", "timers", _] => method != "GET",
        ["v1", "macro_collections"] => method != "GET",
        ["v1", "macro_collection", _] => method != "GET",
        ["v1", "macros"] => method != "GET",
        ["v1", "macro", _] => method != "GET",
        ["v1", "macro", _, "icon"] => true,
        ["v1", "macro", _, _] => method != "GET",
        ["v1", "clear", "groups"] => method != "GET",
        ["v1", "clear", "group", _] => method != "GET",
        ["v1", "clear", "group", _, "icon"] => true,
        ["v1", "clear", "group", _, _] => method != "GET",
        ["v1", "clear", "layer", _] => method != "GET",
        ["v1", "status", "layers"] => method != "GET",
        ["v1", "stage", "screens" | "layouts"] => method != "GET",
        ["v1", "stage", "screen", _, "layout"] => method != "GET",
        ["v1", "stage", "screen", _, "layout", _] => method != "GET",
        ["v1", "stage", "layout_map"] => !matches!(method, "GET" | "PUT"),
        ["v1", "stage", "message"] => !matches!(method, "GET" | "PUT" | "DELETE"),
        ["v1", "look", "current"] => true,
        ["v1", "look", _] => method != "GET",
        ["v1", "look", _, "trigger"] => method != "GET",
        ["v1", "looks"] => method != "GET",
        _ => false,
    }
}

fn modern_entries<'a>(status: &'a Value, field: &str) -> Result<&'a [Value], String> {
    status["modernCatalog"][field]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("The {field} catalog is unavailable."))
}

fn resolve<'a>(
    entries: &'a [Value],
    selector: &str,
    name_field: &str,
) -> Result<(usize, &'a Value), String> {
    let mut found = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let (source_id, name) = catalog_identity(entry, name_field)?;
        if matches_id(source_id, name, index, selector) {
            found.push((index, entry));
        }
    }
    match found.as_slice() {
        [(index, entry)] => Ok((*index, *entry)),
        [] => Err("The requested target is unavailable.".into()),
        _ => Err("The requested target is ambiguous.".into()),
    }
}

fn catalog_identity<'a>(entry: &'a Value, name_field: &str) -> Result<(&'a str, &'a str), String> {
    let source_id = entry["id"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or("Catalog item identity is unavailable.")?;
    let name = entry[name_field]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or("Catalog item name is unavailable.")?;
    Ok((source_id, name))
}

fn timer_source(status: &Value) -> Result<&[Value], String> {
    status["timers"]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| "Accepted timer status is unavailable.".into())
}

fn timers(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = timer_source(&status)?;
    entries
        .iter()
        .enumerate()
        .map(|(index, timer)| timer_value(timer, index))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

pub(super) fn timer_value(timer: &Value, index: usize) -> Result<Value, String> {
    let (id, name) = catalog_identity(timer, "name")?;
    let kind = timer_kind(timer)?;
    let allows_overrun = timer["allowsOverrun"]
        .as_bool()
        .ok_or("Timer overrun setting is unavailable.")?;
    let mut value = json!({"id":identity(id, name, index),"allows_overrun":allows_overrun});
    match kind {
        "countdown" => {
            value["countdown"] =
                json!({"duration": whole_seconds(timer.get("durationSeconds"), "timer duration")?})
        }
        "countdown-to-time" => {
            let target = whole_seconds(timer.get("targetTimeSeconds"), "timer target time")?;
            if !(0..86_400).contains(&target) {
                return Err("Countdown-to-time target is outside one day.".into());
            }
            let use_24_hour = timer_use_24_hour(timer)?;
            let (period, time_of_day) = if use_24_hour {
                ("24_hour", target)
            } else {
                let within_day = target.rem_euclid(86_400);
                let period = if within_day >= 43_200 { "pm" } else { "am" };
                let mut time = within_day % 43_200;
                if time == 0 {
                    time = 43_200;
                }
                (period, time)
            };
            value["count_down_to_time"] = json!({"time_of_day":time_of_day,"period":period});
        }
        "elapsed" => {
            let mut elapsed = json!({"start_time":whole_seconds(timer.get("elapsedStartSeconds"), "elapsed timer start")?});
            if let Some(end) = timer
                .get("elapsedEndSeconds")
                .filter(|value| !value.is_null())
            {
                elapsed["end_time"] = json!(whole_seconds(Some(end), "elapsed timer end")?);
            }
            value["elapsed"] = elapsed;
        }
        _ => {
            return Err(
                "This timer type cannot be represented by the ProPresenter Remote API.".into(),
            )
        }
    }
    Ok(value)
}

fn system_time() -> Result<Value, String> {
    Ok(json!(chrono::Utc::now().timestamp()))
}

fn video_countdown(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let background = status
        .get("background")
        .and_then(Value::as_object)
        .ok_or("Accepted background playback status is unavailable.")?;
    let background_id = background
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("There is no active background video.")?;
    let media = status["modernCatalog"]["media"]
        .as_array()
        .ok_or("Accepted media catalog is unavailable.")?;
    let mut matching_media = media
        .iter()
        .filter(|entry| entry.get("id").and_then(Value::as_str) == Some(background_id));
    let media_entry = matching_media
        .next()
        .ok_or("Active background media is unavailable in the media catalog.")?;
    if matching_media.next().is_some() {
        return Err("Active background media identity is ambiguous.".into());
    }
    if !media_entry
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|media_type| media_type.eq_ignore_ascii_case("video"))
    {
        return Err("There is no active background video.".into());
    }
    let playback = background
        .get("playback")
        .and_then(Value::as_str)
        .ok_or("Background playback mode is unavailable.")?;
    if !matches!(playback, "stop" | "loop") {
        return Err("Background playback mode is unsupported.".into());
    }
    let state = background
        .get("playbackState")
        .and_then(Value::as_object)
        .ok_or("Active background video clock is unavailable.")?;
    let background_selection = background
        .get("selectionId")
        .filter(|value| !value.is_null());
    let playback_selection = state.get("selectionId").filter(|value| !value.is_null());
    if background_selection != playback_selection {
        return Err("Background video playback clock belongs to a stale selection.".into());
    }
    let number = |key: &str, description: &str| -> Result<f64, String> {
        state
            .get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("{description} is unavailable or invalid."))
    };
    let current_time = number("currentTime", "Video playback position")?;
    if current_time < 0.0 {
        return Err("Video playback position cannot be negative.".into());
    }
    let duration = match state.get("duration") {
        Some(Value::Null) => None,
        Some(_) => {
            let duration = number("duration", "Video duration")?;
            if duration <= 0.0 {
                return Err("Video duration must be greater than zero.".into());
            }
            Some(duration)
        }
        None => return Err("Video duration status is unavailable.".into()),
    };
    let in_seconds = number("inSeconds", "Video in point")?;
    if in_seconds < 0.0 {
        return Err("Video in point cannot be negative.".into());
    }
    let end_seconds = match state.get("outSeconds") {
        Some(Value::Null) => {
            duration.ok_or("Video duration is unavailable without an out point.")?
        }
        Some(_) => number("outSeconds", "Video out point")?,
        None => return Err("Video out point status is unavailable.".into()),
    };
    if end_seconds <= in_seconds {
        return Err("Video out point must be after its in point.".into());
    }
    if let Some(duration) = duration {
        crate::output_playback::validate_media_transport_duration(
            in_seconds,
            Some(end_seconds),
            duration,
        )?;
    }
    let rate = number("rate", "Video playback rate")?;
    if rate == 0.0 {
        return Err("Video playback rate cannot be zero.".into());
    }
    let captured_at = state
        .get("capturedAt")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or("Video playback capture time is unavailable or invalid.")?;
    let delayed = state
        .get("delayed")
        .and_then(Value::as_bool)
        .ok_or("Video playback delay status is unavailable.")?;
    let shuttle = state
        .get("shuttle")
        .and_then(Value::as_bool)
        .ok_or("Video shuttle status is unavailable.")?;
    let playback_status = state
        .get("status")
        .and_then(Value::as_str)
        .ok_or("Video playback status is unavailable.")?;

    let remaining = match playback_status {
        "ended" => 0.0,
        "paused" | "playing" => {
            let mut position = current_time;
            if playback_status == "playing" && !delayed {
                let elapsed_seconds = chrono::Utc::now()
                    .timestamp_millis()
                    .saturating_sub(captured_at)
                    .max(0) as f64
                    / 1000.0;
                position += elapsed_seconds * rate;
            }
            if !position.is_finite() {
                return Err("Video playback position is outside the supported range.".into());
            }
            if playback == "loop" && rate > 0.0 && !shuttle && position >= end_seconds {
                let span = end_seconds - in_seconds;
                position = in_seconds + (position - in_seconds).rem_euclid(span);
            } else {
                position = position.clamp(in_seconds, end_seconds);
            }
            end_seconds - position
        }
        _ => return Err("Video playback status is unsupported.".into()),
    };
    let milliseconds = (remaining.max(0.0) * 1000.0).floor();
    if !milliseconds.is_finite() || milliseconds > i64::MAX as f64 {
        return Err("Video countdown is outside the supported range.".into());
    }
    let native_clock = data::format_clock(milliseconds as i64);
    let (hours, rest) = native_clock
        .split_once(':')
        .ok_or("Video countdown format is unavailable.")?;
    let hours = hours
        .parse::<u64>()
        .map_err(|_| "Video countdown format is unavailable.")?;
    Ok(json!(format!("{hours:02}:{rest}")))
}

fn timer_kind(timer: &Value) -> Result<&str, String> {
    match timer.get("type") {
        None => Ok("countdown"),
        Some(Value::String(kind))
            if matches!(kind.as_str(), "countdown" | "countdown-to-time" | "elapsed") =>
        {
            Ok(kind)
        }
        Some(_) => Err("Timer type is invalid or unsupported.".into()),
    }
}

fn timer_use_24_hour(timer: &Value) -> Result<bool, String> {
    match timer.get("format") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Object(format)) => match format.get("use24Hour") {
            None => Ok(false),
            Some(Value::Bool(value)) => Ok(*value),
            Some(_) => Err("Countdown-to-time 24-hour setting is invalid.".into()),
        },
        Some(_) => Err("Countdown-to-time display format is invalid.".into()),
    }
}

fn whole_seconds(value: Option<&Value>, description: &str) -> Result<i64, String> {
    value
        .and_then(Value::as_f64)
        .filter(|seconds| {
            seconds.is_finite()
                && seconds.fract() == 0.0
                && *seconds >= 0.0
                && *seconds <= i64::MAX as f64
        })
        .map(|seconds| seconds as i64)
        .ok_or_else(|| format!("{description} is unavailable or not an integer number of seconds."))
}

fn current_timers(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let timers = timer_source(&status)?;
    let runs = status["timerRuns"]
        .as_object()
        .ok_or("Timer run status is unavailable.")?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    timers
        .iter()
        .enumerate()
        .map(|(index, timer)| {
            let (timer_id, name) = catalog_identity(timer, "name")?;
            let kind = timer_kind(timer)?;
            let run = runs.get(timer_id);
            let paused = run
                .map(|run| {
                    run["paused"]
                        .as_bool()
                        .ok_or("Timer run pause status is unavailable.")
                })
                .transpose()?;
            let value_ms = timer_milliseconds(timer, run, now_ms)?;
            let is_running = paused == Some(false);
            let elapsed = kind == "elapsed";
            let allows_overrun = timer["allowsOverrun"]
                .as_bool()
                .ok_or("Timer overrun setting is unavailable.")?;
            let complete = if elapsed {
                match timer.get("elapsedEndSeconds").filter(|value| !value.is_null()) {
                    Some(end) => value_ms >= whole_seconds(Some(end), "elapsed timer end")?.saturating_mul(1000),
                    None => false,
                }
            } else {
                value_ms <= 0
            };
            let state = if !is_running {
                if complete && run.is_some() {
                    if allows_overrun && (elapsed || value_ms < 0) {
                        "overran"
                    } else {
                        "complete"
                    }
                } else {
                    "stopped"
                }
            } else if complete {
                if allows_overrun { "overrunning" } else { "complete" }
            } else {
                "running"
            };
            Ok(json!({"id":identity(timer_id, name, index),"state":state,"time":api_clock(value_ms)}))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(Value::Array)
}

fn timer_milliseconds(timer: &Value, run: Option<&Value>, now_ms: i64) -> Result<i64, String> {
    let kind = timer_kind(timer)?;
    if let Some(run) = run {
        let elapsed = kind == "elapsed";
        match run.get("elapsed") {
            Some(value) if value.as_bool() != Some(elapsed) => {
                return Err("Timer run type does not match its definition.".into());
            }
            None if elapsed => return Err("Elapsed timer run type is unavailable.".into()),
            _ => {}
        }
        if run["paused"]
            .as_bool()
            .ok_or("Timer run pause status is unavailable.")?
        {
            return run["remaining"]
                .as_i64()
                .ok_or("Paused timer value is unavailable.".into());
        }
        if let Some(end) = run["end"].as_i64() {
            return Ok(if elapsed {
                now_ms.saturating_sub(end)
            } else {
                end.saturating_sub(now_ms)
            });
        }
        return Err("Running timer clock is unavailable.".into());
    }
    match kind {
        "elapsed" => seconds_millis(whole_seconds(
            timer.get("elapsedStartSeconds"),
            "elapsed timer start",
        )?),
        "countdown-to-time" => {
            let target = whole_seconds(timer.get("targetTimeSeconds"), "timer target time")?;
            if !(0..86_400).contains(&target) {
                return Err("Countdown-to-time target is outside one day.".into());
            }
            let target_millis = seconds_millis(target)?;
            let now = chrono::Local::now();
            let midnight = now
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|value| chrono::Local.from_local_datetime(&value).earliest())
                .ok_or("Local time is unavailable.")?;
            let target_time = midnight + chrono::Duration::milliseconds(target_millis);
            let target_time = if target_time <= now {
                target_time + chrono::Duration::days(1)
            } else {
                target_time
            };
            Ok(target_time.timestamp_millis().saturating_sub(now_ms))
        }
        "countdown" => seconds_millis(whole_seconds(
            timer.get("durationSeconds"),
            "timer duration",
        )?),
        _ => Err("Timer type is unavailable or unsupported.".into()),
    }
}

fn seconds_millis(seconds: i64) -> Result<i64, String> {
    seconds
        .checked_mul(1000)
        .ok_or_else(|| "Timer value is outside the supported range.".into())
}

fn api_clock(milliseconds: i64) -> String {
    let sign = if milliseconds < 0 { "-" } else { "" };
    let seconds = milliseconds.unsigned_abs() / 1000;
    format!(
        "{sign}{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

fn timer(backend: &dyn Backend, selector: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = timer_source(&status)?;
    let (index, entry) = resolve(entries, selector, "name")?;
    timer_value(entry, index)
}

fn timer_operation_one(
    backend: &dyn Backend,
    selector: &str,
    operation: &str,
) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = timer_source(&status)?;
    let (_, timer) = resolve(entries, selector, "name")?;
    let action = timer_action(operation)?;
    let timer_id = catalog_identity(timer, "name")?.0;
    backend.command(&json!({"command":"timer","id":timer_id,"action":action}))?;
    Ok(Value::Null)
}

fn timer_operation(backend: &dyn Backend, operation: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let action = timer_action(operation)?;
    let entries = timer_source(&status)?;
    let ids = entries
        .iter()
        .map(|timer| catalog_identity(timer, "name").map(|(id, _)| id))
        .collect::<Result<Vec<_>, _>>()?;
    for id in ids {
        backend.command(&json!({"command":"timer","id":id,"action":action}))?;
    }
    Ok(Value::Null)
}

fn timer_action(operation: &str) -> Result<&'static str, String> {
    match operation {
        "start" => Ok("start"),
        "stop" => Ok("stop"),
        "reset" => Ok("reset"),
        _ => Err("Unsupported timer operation.".into()),
    }
}

fn timer_increment(backend: &dyn Backend, selector: &str, seconds: &str) -> Result<Value, String> {
    let seconds = seconds
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite())
        .ok_or("Timer increment must be finite seconds.")?;
    let status = backend.full()?;
    let entries = timer_source(&status)?;
    let (_, timer) = resolve(entries, selector, "name")?;
    backend.command(
        &json!({"command":"timer","id":timer["id"],"action":"increment","seconds":seconds}),
    )?;
    Ok(Value::Null)
}

fn update_timer(
    backend: &dyn Backend,
    selector: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let body = body
        .filter(|body| body.is_object())
        .ok_or("Timer update body must be an object.")?;
    only_keys(
        body,
        &[
            "id",
            "name",
            "allows_overrun",
            "countdown",
            "count_down_to_time",
            "elapsed",
        ],
    )?;
    let status = backend.full()?;
    let entries = timer_source(&status)?;
    let (timer_index, timer) = resolve(entries, selector, "name")?;
    let body_id = body.get("id").ok_or("Timer update identity is required.")?;
    if !identity_matches(body_id, timer, timer_index)? {
        return Err("Timer update identity does not match the timer path.".into());
    }
    if !body.get("allows_overrun").is_some_and(Value::is_boolean) {
        return Err("allows_overrun must be a boolean.".into());
    }
    let mut patch = Map::new();
    if let Some(name) = body.get("name") {
        if !name.as_str().is_some_and(|value| !value.trim().is_empty()) {
            return Err("Timer name must be a nonempty string.".into());
        }
        patch.insert("name".into(), name.clone());
    }
    if let Some(overrun) = body.get("allows_overrun") {
        if !overrun.is_boolean() {
            return Err("allows_overrun must be a boolean.".into());
        }
        patch.insert("allowsOverrun".into(), overrun.clone());
    }
    let configs = ["countdown", "count_down_to_time", "elapsed"]
        .iter()
        .filter(|field| body.get(**field).is_some())
        .copied()
        .collect::<Vec<_>>();
    if configs.len() != 1 {
        return Err("A timer update must contain exactly one timer configuration.".into());
    }
    match configs[0] {
        "countdown" => {
            only_keys(&body["countdown"], &["duration"])?;
            let value = body["countdown"]["duration"]
                .as_i64()
                .filter(|value| (0..=359_999).contains(value))
                .ok_or("Countdown duration must be integer seconds from 0 through 359999.")?;
            patch.insert("type".into(), json!("countdown"));
            patch.insert("durationSeconds".into(), json!(value));
            patch.insert("targetTimeSeconds".into(), Value::Null);
            patch.insert("elapsedStartSeconds".into(), Value::Null);
            patch.insert("elapsedEndSeconds".into(), Value::Null);
        }
        "count_down_to_time" => {
            let config = &body["count_down_to_time"];
            only_keys(config, &["time_of_day", "period"])?;
            let time = config["time_of_day"]
                .as_i64()
                .ok_or("time_of_day must be integer seconds.")?;
            let period = config["period"]
                .as_str()
                .ok_or("A countdown-to-time period is required.")?;
            let (target, use_24_hour) = match period {
                "24_hour" if (0..86_400).contains(&time) => (time, true),
                "am" if (0..=43_200).contains(&time) => (time.rem_euclid(43_200), false),
                "pm" if (0..=43_200).contains(&time) => (time.rem_euclid(43_200) + 43_200, false),
                _ => return Err("Countdown-to-time period or time is invalid.".into()),
            };
            let mut format = match timer.get("format") {
                None | Some(Value::Null) => json!({}),
                Some(Value::Object(format)) => Value::Object(format.clone()),
                Some(_) => return Err("Countdown-to-time display format is invalid.".into()),
            };
            format["use24Hour"] = json!(use_24_hour);
            patch.insert("type".into(), json!("countdown-to-time"));
            patch.insert("durationSeconds".into(), json!(0));
            patch.insert("targetTimeSeconds".into(), json!(target));
            patch.insert("elapsedStartSeconds".into(), Value::Null);
            patch.insert("elapsedEndSeconds".into(), Value::Null);
            patch.insert("format".into(), format);
        }
        "elapsed" => {
            let config = &body["elapsed"];
            only_keys(config, &["start_time", "end_time"])?;
            let start = config["start_time"]
                .as_i64()
                .filter(|value| (0..=359_999).contains(value))
                .ok_or("Elapsed timer start_time must be integer seconds from 0 through 359999.")?;
            let end = match config.get("end_time") {
                None | Some(Value::Null) => Value::Null,
                Some(value) => {
                    json!(value.as_i64().filter(|value| (0..=359_999).contains(value)).ok_or(
                    "Elapsed timer end_time must be null or integer seconds from 0 through 359999."
                )?)
                }
            };
            patch.insert("type".into(), json!("elapsed"));
            patch.insert("durationSeconds".into(), json!(0));
            patch.insert("targetTimeSeconds".into(), Value::Null);
            patch.insert("elapsedStartSeconds".into(), json!(start));
            patch.insert("elapsedEndSeconds".into(), end);
        }
        _ => unreachable!(),
    }
    if patch.is_empty() {
        return Err("Timer update body contains no editable fields.".into());
    }
    backend.command(&json!({"command":"timerDefinition","workspacePath":data::workspace_path(&status)?,"id":timer["id"],"patch":patch}))?;
    Ok(Value::Null)
}

fn only_keys(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or("A timer configuration must be an object.")?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Timer update contains unsupported fields.".into());
    }
    Ok(())
}

fn identity_matches(body_id: &Value, timer: &Value, index: usize) -> Result<bool, String> {
    let Some(object) = body_id.as_object() else {
        return Err("Target identity must be an object.".into());
    };
    if object
        .keys()
        .any(|key| !["uuid", "name", "index"].contains(&key.as_str()))
    {
        return Err("Target identity contains unsupported fields.".into());
    }
    let (id, name) = catalog_identity(timer, "name")?;
    let mut supplied = false;
    if let Some(value) = body_id.get("uuid") {
        let uuid = value
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or("Target uuid must be a nonempty string.")?;
        supplied = true;
        if !matches_id(id, name, index, uuid) {
            return Ok(false);
        }
    }
    if let Some(value) = body_id.get("name") {
        let target_name = value
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or("Target name must be a nonempty string.")?;
        supplied = true;
        if target_name != name {
            return Ok(false);
        }
    }
    if let Some(value) = body_id.get("index") {
        let target_index = value
            .as_u64()
            .ok_or("Target index must be a non-negative integer.")?;
        supplied = true;
        if target_index != index as u64 {
            return Ok(false);
        }
    }
    if !supplied {
        return Err("Target identity must contain uuid, name or index.".into());
    }
    Ok(true)
}

fn macros(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    modern_entries(&status, "macros")?
        .iter()
        .enumerate()
        .map(|(index, item)| macro_value(item, index))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn macro_value(item: &Value, index: usize) -> Result<Value, String> {
    let (id, name) = catalog_identity(item, "title")?;
    let color = match item.get("color") {
        None => json!({"red":0.0,"green":0.0,"blue":0.0,"alpha":0.0}),
        Some(Value::String(value)) if value.is_empty() => {
            json!({"red":0.0,"green":0.0,"blue":0.0,"alpha":0.0})
        }
        Some(Value::String(value)) => rgba(value)?,
        Some(_) => return Err("Macro color is invalid.".into()),
    };
    let mut result = json!({"id":identity(id,name,index),"color":color});
    if let Some(actions) = item.get("actions") {
        if !actions.is_array() {
            return Err("Macro actions are unavailable.".into());
        }
        result["actions"] = actions.clone();
    }
    if let Some(icon) = item.get("imageType") {
        let icon = icon
            .as_str()
            .ok_or("Macro icon type is invalid.")?
            .strip_prefix("ImageType")
            .ok_or("Macro icon type is unavailable.")?;
        result["image_type"] = json!(icon);
    }
    Ok(result)
}

fn macro_collections(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let collections = modern_entries(&status, "macroCollections")?;
    let macros = modern_entries(&status, "macros")?;
    let mut values = collections
        .iter()
        .enumerate()
        .map(|(index, collection)| macro_collection_value(collection, index, macros))
        .collect::<Result<Vec<_>, _>>()?;
    values.extend(unassigned_macro_collection(collections, macros)?);
    Ok(json!({"collections":values}))
}

fn macro_collection(backend: &dyn Backend, selector: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let collections = modern_entries(&status, "macroCollections")?;
    let macros = modern_entries(&status, "macros")?;
    if let Some(unassigned) = unassigned_macro_collection(collections, macros)? {
        let id = &unassigned["id"];
        if matches_id(
            id["uuid"].as_str().unwrap_or_default(),
            UNASSIGNED_MACROS,
            collections.len(),
            selector,
        ) {
            return Ok(unassigned);
        }
    }
    let (index, collection) = resolve(collections, selector, "title")?;
    macro_collection_value(collection, index, macros)
}

const UNASSIGNED_MACROS: &str = "Unassigned";

/// Orbit allows macros outside any collection, but Remote only lists macros through collections.
fn unassigned_macro_collection(
    collections: &[Value],
    macros: &[Value],
) -> Result<Option<Value>, String> {
    let mut members = Vec::new();
    for (index, item) in macros.iter().enumerate() {
        let assigned = match item.get("collectionIds") {
            None => false,
            Some(ids) => ids
                .as_array()
                .ok_or("Macro collection membership is invalid.")?
                .iter()
                .any(|id| collections.iter().any(|c| c["id"] == *id)),
        };
        if !assigned {
            members.push(macro_value(item, index)?);
        }
    }
    Ok((!members.is_empty()).then(|| {
        json!({"id":identity("orbit-unassigned-macros",UNASSIGNED_MACROS,collections.len()),"macros":members})
    }))
}

fn macro_collection_value(
    collection: &Value,
    index: usize,
    macros: &[Value],
) -> Result<Value, String> {
    let (source_id, name) = catalog_identity(collection, "title")?;
    let mut members = Vec::new();
    for (macro_index, item) in macros.iter().enumerate() {
        let is_member = match item.get("collectionIds") {
            None => false,
            Some(value) => {
                let ids = value
                    .as_array()
                    .ok_or("Macro collection membership is invalid.")?;
                let mut found = false;
                for value in ids {
                    let id = value
                        .as_str()
                        .ok_or("Macro collection membership identity is invalid.")?;
                    found |= id == source_id;
                }
                found
            }
        };
        if is_member {
            members.push((macro_index, item));
        }
    }
    let ordered = match collection.get("macroOrder") {
        None => members,
        Some(value) => {
            let order = value
                .as_array()
                .ok_or("Macro collection order is invalid.")?;
            let mut ordered = Vec::with_capacity(order.len());
            let mut seen = std::collections::HashSet::new();
            for macro_id in order {
                let macro_id = macro_id
                    .as_str()
                    .ok_or("Macro collection order identity is invalid.")?;
                if !seen.insert(macro_id) {
                    return Err("Macro collection order contains a duplicate macro.".into());
                }
                let target = members
                    .iter()
                    .find(|(_, item)| item["id"].as_str() == Some(macro_id))
                    .copied()
                    .ok_or("Macro collection order references a non-member macro.")?;
                ordered.push(target);
            }
            if ordered.len() != members.len() {
                return Err("Macro collection order does not include every member.".into());
            }
            ordered
        }
    };
    let macros = ordered
        .into_iter()
        .map(|(macro_index, item)| macro_value(item, macro_index))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"id":identity(source_id,name,index),"macros":macros}))
}

fn rgba(hex: &str) -> Result<Value, String> {
    let hex = hex
        .strip_prefix('#')
        .ok_or("A source color is not a hexadecimal color.")?;
    if !matches!(hex.len(), 6 | 8) || !hex.chars().all(|digit| digit.is_ascii_hexdigit()) {
        return Err("A source color is not a hexadecimal color.".into());
    }
    let component = |offset| {
        u8::from_str_radix(&hex[offset..offset + 2], 16)
            .map(|value| f64::from(value) / 255.0)
            .map_err(|_| "A source color is invalid.".to_string())
    };
    Ok(
        json!({"red":component(0)?,"green":component(2)?,"blue":component(4)?,"alpha":if hex.len() == 8 { component(6)? } else { 1.0 }}),
    )
}

fn macro_details(backend: &dyn Backend, selector: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = modern_entries(&status, "macros")?;
    let (index, entry) = resolve(entries, selector, "title")?;
    macro_value(entry, index)
}

fn trigger_catalog_target(
    backend: &dyn Backend,
    field: &str,
    command: &str,
    selector: &str,
) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = if field == "macros" || field == "clearGroups" || field == "audienceLooks" {
        modern_entries(&status, field)?
    } else {
        return Err("Unsupported target catalog.".into());
    };
    let (_, entry) = resolve(entries, selector, "title")?;
    backend.command(
        &json!({"command":command,"workspacePath":data::workspace_path(&status)?,"id":entry["id"]}),
    )?;
    Ok(Value::Null)
}

fn clear_groups(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    modern_entries(&status, "clearGroups")?
        .iter()
        .enumerate()
        .map(|(index, item)| clear_group_value(item, index))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn clear_group(backend: &dyn Backend, selector: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = modern_entries(&status, "clearGroups")?;
    let (index, entry) = resolve(entries, selector, "title")?;
    clear_group_value(entry, index)
}

fn clear_group_value(item: &Value, index: usize) -> Result<Value, String> {
    let (id, name) = catalog_identity(item, "title")?;
    let layers = item["layers"]
        .as_array()
        .ok_or("Clear group layers are unavailable.")?
        .iter()
        .map(|layer| match layer.as_str() {
            Some("audio") => Ok("music"),
            Some("audio-effects") => Ok("audio_effects"),
            Some("prop") => Ok("props"),
            Some("messages") => Ok("messages"),
            Some("announcements") => Ok("announcements"),
            Some("slide") => Ok("presentation"),
            Some("background") => Ok("presentation_media"),
            Some("video-input") => Ok("video_input"),
            Some("stage-messages") => Err(
                "Orbit stage-message clearing is not a ProPresenter clear-group layer.".to_string(),
            ),
            _ => Err("Clear group has an unsupported layer.".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let icon = item["imageType"]
        .as_str()
        .ok_or("Clear group icon is unavailable.")?
        .strip_prefix("ImageType")
        .ok_or("Clear group icon is unavailable.")?;
    let tint = match item.get("iconTinted") {
        None | Some(Value::Bool(false)) => Value::Null,
        Some(Value::Bool(true)) => {
            let color = item["iconTint"]
                .as_str()
                .ok_or("Clear group icon tint is unavailable.")?;
            rgba(color)?
        }
        Some(_) => return Err("Clear group icon tint setting is invalid.".into()),
    };
    Ok(json!({
        "id":identity(id,name,index),"layers":layers,"icon":icon,"tint":tint,
        "stop_timeline_announcements":clear_group_bool(item,"stopAnnouncementsTimeline",true)?,
        "stop_timeline_presentation":clear_group_bool(item,"stopPresentationTimeline",true)?,
        "clear_next_presentation":clear_group_bool(item,"clearPresentationNext",false)?,
    }))
}

fn clear_group_bool(value: &Value, field: &str, default: bool) -> Result<bool, String> {
    match value.get(field) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("Clear group setting {field} is invalid.")),
    }
}

fn clear_layer(backend: &dyn Backend, layer: &str) -> Result<Value, String> {
    let layer = match layer {
        "audio" => "music",
        "props" => "props",
        "messages" => "messages",
        "announcements" => "announcements",
        "slide" => "slide",
        "media" => "media",
        "video_input" => "videoInput",
        _ => return Err("Unsupported clear layer.".into()),
    };
    backend.command(&json!({"command":"clearLayer","layer":layer}))?;
    Ok(Value::Null)
}

fn status_layers(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.live();
    let current_slide = status
        .get("currentSlide")
        .ok_or("Presentation layer status is unavailable.")?;
    let announcements = status
        .get("announcements")
        .ok_or("Announcement layer status is unavailable.")?;
    let audio = status
        .get("audio")
        .ok_or("Audio layer status is unavailable.")?;
    let media = status
        .get("background")
        .ok_or("Media layer status is unavailable.")?;
    let messages = status
        .get("activeMessage")
        .ok_or("Message layer status is unavailable.")?;
    let props = status
        .get("livePropIds")
        .and_then(Value::as_array)
        .ok_or("Prop layer status is unavailable.")?;
    let video_input = status
        .get("liveVideoInputId")
        .ok_or("Video input layer status is unavailable.")?;
    let announcement_active = nullable_announcement_active(announcements)?;
    let audio_active = nullable_identity_active(audio, "Audio")?;
    let media_active = nullable_identity_active(media, "Media")?;
    let message_active = nullable_identity_active(messages, "Message")?;
    let slide_active = nullable_slide_active(current_slide)?;
    if !video_input.is_null() && !video_input.as_str().is_some_and(|id| !id.is_empty()) {
        return Err("Video input layer identity is invalid.".into());
    }
    if props
        .iter()
        .any(|prop| !prop.as_str().is_some_and(|id| !id.is_empty()))
    {
        return Err("Prop layer identity is invalid.".into());
    }
    Ok(json!({
        "announcements":announcement_active,
        "audio":audio_active,
        "media":media_active,
        "messages":message_active,
        "props":!props.is_empty(),
        "slide":slide_active,
        "video_input":!video_input.is_null(),
    }))
}

fn nullable_announcement_active(value: &Value) -> Result<bool, String> {
    if value.is_null() {
        return Ok(false);
    }
    let value = value
        .as_object()
        .ok_or("Announcement layer status is invalid.")?;
    if !value.get("item").is_some_and(Value::is_object)
        || !value.get("slide").is_some_and(Value::is_object)
        || !value
            .get("selectionId")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty())
        || !value.get("paused").is_some_and(Value::is_boolean)
        || !value
            .get("currentTime")
            .and_then(Value::as_f64)
            .is_some_and(f64::is_finite)
    {
        return Err("Announcement layer status is incomplete.".into());
    }
    Ok(true)
}

fn nullable_slide_active(value: &Value) -> Result<bool, String> {
    if value.is_null() {
        return Ok(false);
    }
    if !value
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
    {
        return Err("Presentation layer identity is unavailable.".into());
    }
    Ok(true)
}

fn nullable_identity_active(value: &Value, description: &str) -> Result<bool, String> {
    if value.is_null() {
        return Ok(false);
    }
    let id = value
        .get("id")
        .ok_or_else(|| format!("{description} layer identity is unavailable."))?;
    if id.is_null() {
        return Ok(false);
    }
    id.as_str()
        .filter(|id| !id.is_empty())
        .map(|_| true)
        .ok_or_else(|| format!("{description} layer identity is invalid."))
}

fn stage_outputs(status: &Value) -> Result<Vec<Value>, String> {
    Ok(data::array(status, "outputs")?
        .iter()
        .filter(|output| output["type"] == "stage")
        .cloned()
        .collect())
}

fn stage_screens(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    stage_outputs(&status)?
        .iter()
        .enumerate()
        .map(|(index, screen)| {
            let (source_id, name) = catalog_identity(screen, "label")?;
            Ok(json!({"id":identity(source_id,name,index)}))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(Value::Array)
}

fn stage_layouts(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = modern_entries(&status, "stageLayouts")?;
    entries
        .iter()
        .enumerate()
        .map(|(index, layout)| {
            let (source_id, name) = catalog_identity(layout, "title")?;
            Ok(json!({"id":identity(source_id,name,index)}))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(Value::Array)
}

fn stage_layout_map(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let screens = stage_outputs(&status)?;
    let layouts = modern_entries(&status, "stageLayouts")?;
    let mut assignments = Vec::new();
    for (screen_index, screen) in screens.iter().enumerate() {
        let selected = match screen.get("stageLayoutId") {
            None | Some(Value::Null) => continue,
            Some(Value::String(value)) if value.is_empty() => continue,
            Some(Value::String(value)) => value,
            Some(_) => return Err("A stage screen selected layout is invalid.".into()),
        };
        let (screen_id, screen_name) = catalog_identity(screen, "label")?;
        let (layout_index, layout) = layouts
            .iter()
            .enumerate()
            .find(|(_, layout)| layout["id"].as_str() == Some(selected.as_str()))
            .ok_or("A stage screen references an unavailable layout.")?;
        let (layout_id, layout_name) = catalog_identity(layout, "title")?;
        assignments.push(json!({
            "screen":identity(screen_id,screen_name,screen_index),
            "layout":identity(layout_id,layout_name,layout_index),
        }));
    }
    Ok(Value::Array(assignments))
}

fn stage_screen_layout(backend: &dyn Backend, screen: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let screens = stage_outputs(&status)?;
    let (_, screen) = resolve(&screens, screen, "label")?;
    let selected = screen["stageLayoutId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("The stage screen has no layout assigned.")?;
    let layouts = modern_entries(&status, "stageLayouts")?;
    let (index, layout) = layouts
        .iter()
        .enumerate()
        .find(|(_, layout)| layout["id"].as_str() == Some(selected))
        .ok_or("A stage screen references an unavailable layout.")?;
    let (id, name) = catalog_identity(layout, "title")?;
    Ok(json!({"id":identity(id,name,index)}))
}

fn set_stage_screen_layout(
    backend: &dyn Backend,
    screen: &str,
    layout: &str,
) -> Result<Value, String> {
    let status = backend.full()?;
    let screens = stage_outputs(&status)?;
    let (_, screen) = resolve(&screens, screen, "label")?;
    let (_, layout) = resolve(modern_entries(&status, "stageLayouts")?, layout, "title")?;
    backend.command(&json!({"command":"stageLayout","workspacePath":data::workspace_path(&status)?,"screenId":screen["id"],"id":layout["id"]}))?;
    Ok(Value::Null)
}

fn identity_selector(value: &Value) -> Result<String, String> {
    let object = value
        .as_object()
        .ok_or("Target identity must be an object.")?;
    if object
        .keys()
        .any(|key| !["uuid", "name", "index"].contains(&key.as_str()))
    {
        return Err("Target identity contains unsupported fields.".into());
    }
    if let Some(uuid) = value.get("uuid") {
        return uuid
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "Target uuid must be a nonempty string.".into());
    }
    if let Some(name) = value.get("name") {
        return name
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| "Target name must be a nonempty string.".into());
    }
    if let Some(index) = value.get("index") {
        return index
            .as_u64()
            .map(|index| index.to_string())
            .ok_or_else(|| "Target index must be a non-negative integer.".into());
    }
    Err("A target id must contain uuid, name or index.".into())
}

fn resolve_identity<'a>(
    entries: &'a [Value],
    target: &Value,
    name_field: &str,
) -> Result<(usize, &'a Value), String> {
    let selector = identity_selector(target)?;
    let (index, entry) = resolve(entries, &selector, name_field)?;
    if !identity_matches(target, entry, index)? {
        return Err("The target identity fields do not refer to the same item.".into());
    }
    Ok((index, entry))
}

fn update_stage_layout_map(backend: &dyn Backend, body: Option<&Value>) -> Result<Value, String> {
    let body = body
        .and_then(Value::as_array)
        .ok_or("Stage layout map body must be an array.")?;
    let status = backend.full()?;
    let screens = stage_outputs(&status)?;
    let layouts = modern_entries(&status, "stageLayouts")?;
    let workspace = data::workspace_path(&status)?;
    let mut updates = Vec::with_capacity(body.len());
    let mut touched = std::collections::HashSet::new();
    for update in body {
        let update_object = update
            .as_object()
            .ok_or("Each layout assignment must be an object.")?;
        if update_object
            .keys()
            .any(|key| !["screen", "layout"].contains(&key.as_str()))
        {
            return Err("Layout assignment contains unsupported fields.".into());
        }
        let (_, screen) = resolve_identity(
            &screens,
            update
                .get("screen")
                .ok_or("Each layout assignment needs a screen identity.")?,
            "label",
        )?;
        let (_, layout) = resolve_identity(
            layouts,
            update
                .get("layout")
                .ok_or("Each layout assignment needs a layout identity.")?,
            "title",
        )?;
        let screen_id = screen["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Stage screen identity is unavailable.")?;
        if !touched.insert(screen_id.to_string()) {
            return Err("A stage screen occurs more than once in the layout map.".into());
        }
        updates.push(json!({"screenId":screen_id,"id":layout["id"]}));
    }
    for update in updates {
        backend.command(&json!({"command":"stageLayout","workspacePath":workspace,"screenId":update["screenId"],"id":update["id"]}))?;
    }
    Ok(Value::Null)
}

fn stage_message(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.live();
    let message = status
        .get("stageMessage")
        .filter(|message| message.is_object())
        .ok_or("Stage message status is unavailable.")?;
    match message["state"]
        .as_str()
        .ok_or("Stage message state is unavailable.")?
    {
        "visible" => Ok(json!(message["text"]
            .as_str()
            .ok_or("Stage message text is unavailable.")?)),
        "hidden" | "cleared" => Ok(json!("")),
        _ => Err("Stage message state is unsupported.".into()),
    }
}

fn set_stage_message(backend: &dyn Backend, body: Option<&Value>) -> Result<Value, String> {
    let text = body
        .and_then(Value::as_str)
        .filter(|text| text.chars().count() <= 2000)
        .ok_or("Stage message body must be a string within 2000 characters.")?;
    let status = backend.full()?;
    let workspace = data::workspace_path(&status)?;
    backend.command(
        &json!({"command":"stageMessage","workspacePath":workspace,"action":"edit","text":text}),
    )?;
    backend
        .command(&json!({"command":"stageMessage","workspacePath":workspace,"action":"show"}))?;
    Ok(Value::Null)
}

fn hide_stage_message(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    backend.command(&json!({"command":"stageMessage","workspacePath":data::workspace_path(&status)?,"action":"hide"}))?;
    Ok(Value::Null)
}

fn looks(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let looks = modern_entries(&status, "audienceLooks")?;
    looks
        .iter()
        .enumerate()
        .map(|(index, look)| look_value(&status, look, index))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn look(backend: &dyn Backend, selector: &str) -> Result<Value, String> {
    let status = backend.full()?;
    let entries = modern_entries(&status, "audienceLooks")?;
    let (index, look) = resolve(entries, selector, "title")?;
    look_value(&status, look, index)
}

fn look_value(status: &Value, look: &Value, index: usize) -> Result<Value, String> {
    let (id, name) = catalog_identity(look, "title")?;
    let screen_config = look["screenLooks"]
        .as_array()
        .ok_or("Audience look screen data is unavailable.")?;
    let screens = audience_outputs(status)?
        .into_iter()
        .map(|screen| {
            let screen_id = screen["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or("Audience screen identity is unavailable.")?;
            let screen_look = screen_config
                .iter()
                .find(|entry| entry["screenId"].as_str() == Some(screen_id));
            look_screen(status, &screen, screen_look)
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({"id":identity(id,name,index),"screens":screens}))
}

fn look_screen(status: &Value, screen: &Value, saved: Option<&Value>) -> Result<Value, String> {
    let mut layers = screen["layers"]
        .as_object()
        .cloned()
        .ok_or("Accepted audience screen layers are unavailable.")?;
    if let Some(saved) = saved {
        for (name, value) in saved["lookLayers"]
            .as_object()
            .ok_or("Saved audience look layers are unavailable.")?
        {
            if !value.is_boolean() {
                return Err("Saved audience look layer must be a boolean.".into());
            }
            layers.insert(name.clone(), value.clone());
        }
    }
    let layers = Value::Object(layers);
    let theme = saved
        .and_then(|value| value.get("presentationThemeId"))
        .or_else(|| screen.get("presentationThemeId"));
    let mask = saved
        .and_then(|value| value.get("maskId"))
        .or_else(|| screen.get("maskId"));
    Ok(json!({
        "announcements":required_bool(&layers,"announcements")?,"media":required_bool(&layers,"media")?,
        "messages":required_bool(&layers,"messages")?,"props":required_bool(&layers,"props")?,
        "slide":required_bool(&layers,"slide")?,"video_input":required_bool(&layers,"videoInput")?,
        "presentation":look_reference(status,theme,"themes")?,"mask":look_reference(status,mask,"masks")?
    }))
}

fn current_look(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let screens = audience_outputs(&status)?
        .iter()
        .map(|screen| look_screen(&status, screen, None))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(id) = status.get("activeAudienceLookId").and_then(Value::as_str) {
        let entries = modern_entries(&status, "audienceLooks")?;
        if let Some((index, look)) = entries
            .iter()
            .enumerate()
            .find(|(_, look)| look["id"].as_str() == Some(id))
        {
            let saved = look_value(&status, look, index)?;
            if saved["screens"] == json!(screens) {
                return Ok(saved);
            }
        }
    }
    // The live configuration is a distinct read-only look, not a guessed saved selection.
    Ok(
        json!({"id":identity(&format!("live-look:{}",data::workspace_path(&status)?),"Current Look",0),"screens":screens}),
    )
}

fn audience_outputs(status: &Value) -> Result<Vec<Value>, String> {
    Ok(data::array(status, "outputs")?
        .iter()
        .filter(|output| output["type"] == "audience")
        .cloned()
        .collect())
}

fn required_bool(value: &Value, field: &str) -> Result<bool, String> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("Audience look layer {field} is unavailable."))
}

fn look_reference(status: &Value, value: Option<&Value>, catalog: &str) -> Result<Value, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(json!(""));
    };
    let id = value
        .as_str()
        .ok_or("Audience look reference must be a string.")?;
    if id.is_empty() {
        return Ok(json!(""));
    }
    let entries = modern_entries(status, catalog)?;
    let exists = if catalog == "themes" {
        if let Some((theme_id, slide_id)) = id.split_once("::") {
            entries.iter().any(|entry| {
                entry["id"] == theme_id
                    && entry["slides"]
                        .as_array()
                        .is_some_and(|slides| slides.iter().any(|slide| slide["id"] == slide_id))
            })
        } else {
            entries.iter().any(|entry| entry["id"] == id)
        }
    } else {
        entries.iter().any(|entry| entry["id"] == id)
    };
    if !exists {
        return Err("Audience look references an unavailable theme or mask.".into());
    }
    Ok(identity(id, "", 0)["uuid"].clone())
}
