use chrono::{Datelike, Local, Timelike};
use serde_json::Value;

/// Returns whether a native scene contains linked text that needs refresh.
///
/// The compositor uses this to request periodic refreshes for otherwise static
/// output. Keeping this traversal next to the linked-text resolver prevents
/// the refresh rule from drifting away from the supported linked sources.
pub fn content_has_dynamic_linked_text(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            let dynamic_source = object
                .get("linkedTextSource")
                .and_then(Value::as_str)
                .is_some_and(|value| {
                    value.eq_ignore_ascii_case("System Clock")
                        || value.eq_ignore_ascii_case("Countdown Timer")
                        || value == "Video Countdown"
                        || value == "Slide Timer"
                        || value == "Ticker"
                        || value == "Timecode"
                        || (value == "Stage Message"
                            && object
                                .get("linkedTextStageMessageFlash")
                                .and_then(Value::as_bool)
                                == Some(true))
                })
                && object.get("linkedTextEnabled").and_then(Value::as_bool) != Some(false);
            dynamic_source || object.values().any(content_has_dynamic_linked_text)
        }
        Value::Array(values) => values.iter().any(content_has_dynamic_linked_text),
        _ => false,
    }
}

/// Resolves a linked-text layer against the current output context.
pub(super) fn resolve_linked_text(
    text: &str,
    style: &serde_json::Map<String, Value>,
    context: &Value,
) -> String {
    if style.get("linkedTextEnabled").and_then(Value::as_bool) == Some(false) {
        return text.to_string();
    }
    let Some(source) = style.get("linkedTextSource").and_then(Value::as_str) else {
        return text.to_string();
    };
    if source == "None" {
        return text.to_string();
    }
    if source.is_empty() {
        return String::new();
    }

    match source {
        "Slide Notes" => context_slide_field(context, "slide", "currentSlide", "notes")
            .or_else(|| linked_string(context.get("slideNotes")))
            .unwrap_or_default(),
        "Next Slide Notes" => context_slide_field(context, "nextSlide", "nextSlide", "notes")
            .or_else(|| linked_string(context.get("nextSlideNotes")))
            .unwrap_or_default(),
        "Current Slide" => context_slide_text(context, "slide", "currentSlide", "currentSlideText")
            .unwrap_or_default(),
        "Next Slide" => context_slide_text(context, "nextSlide", "nextSlide", "nextSlideText")
            .unwrap_or_default(),
        "Current Slide Element" | "Next Slide Element" => {
            let key = if source == "Current Slide Element" {
                "slide"
            } else {
                "nextSlide"
            };
            let name = style
                .get("linkedTextElementName")
                .and_then(Value::as_str)
                .unwrap_or("");
            context
                .get(key)
                .and_then(|slide| slide.get("layers"))
                .and_then(Value::as_array)
                .and_then(|layers| {
                    layers
                        .iter()
                        .find(|layer| layer.get("name").and_then(Value::as_str) == Some(name))
                })
                .and_then(|layer| layer.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        }
        "System Clock" => format_system_clock(style),
        "Timecode" => crate::timecode_display::text(&context["timecodeStatus"]),
        "Countdown Timer" => resolve_countdown_timer(style, context, Local::now()),
        "Video Countdown" => video_countdown_milliseconds(style, context, Local::now())
            .map(|milliseconds| format_linked_timer(style, milliseconds, false, Local::now()))
            .unwrap_or_default(),
        "Slide Timer" => context
            .get("slideTimer")
            .and_then(|timer| timer.get("dueAtMs")?.as_i64())
            .map(|due| {
                let now = Local::now();
                format_linked_timer(style, (due - now.timestamp_millis()).max(0), false, now)
            })
            .unwrap_or_default(),
        "Group" => resolve_group_text(style, context),
        "Slide Count" => resolve_slide_count(style, context),
        "Presentation Name" => linked_string(context.get("presentationName"))
            .or_else(|| {
                context
                    .get("item")
                    .and_then(|item| item.get("title"))
                    .and_then(|value| linked_string(Some(value)))
            })
            .unwrap_or_default(),
        "Operator Notes" => linked_string(context.get("operatorNotes"))
            .or_else(|| {
                context
                    .get("item")
                    .and_then(|item| item.get("operatorNotes"))
                    .and_then(|value| linked_string(Some(value)))
            })
            .unwrap_or_default(),
        "Stage Message" => style
            .get("linkedTextTickerContent")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| linked_string(context.get("stageMessageText")))
            .unwrap_or_default(),
        "Slide Label" => context_slide_field(context, "slide", "currentSlide", "label")
            .or_else(|| linked_string(context.get("slideLabel")))
            .unwrap_or_default(),
        "Next Slide Label" => context_slide_field(context, "nextSlide", "nextSlide", "label")
            .or_else(|| linked_string(context.get("nextSlideLabel")))
            .unwrap_or_default(),
        "File" => style
            .get("linkedTextFileContent")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        "Ticker" => context
            .get("ticker")
            .and_then(|ticker| ticker.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        "RSS" => style
            .get("linkedTextRssContent")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        "Playlist Item" => {
            let item = style
                .get("linkedTextPlaylistItemSource")
                .and_then(Value::as_str)
                .unwrap_or("Current");
            let item = if style
                .get("linkedTextPlaylistItemShowArrangement")
                .and_then(Value::as_bool)
                == Some(true)
                && matches!(item, "Current" | "Next")
            {
                format!("{item} With Arrangement")
            } else {
                item.to_string()
            };
            context
                .get("playlistText")
                .and_then(|value| value.get(&item))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        }
        "CCLI" => {
            if context["ccliTemplatePreview"] == true {
                text.to_string()
            } else if context.get("ccli").is_some_and(Value::is_object) {
                let mut text = format_ccli_text(context.get("ccli"));
                if let Some(number) = context["ccliLicenseNumber"]
                    .as_str()
                    .filter(|number| !number.is_empty() && !text.is_empty())
                {
                    text.push_str("\nCCLI License #");
                    text.push_str(number);
                }
                text
            } else {
                style
                    .get("linkedTextTickerContent")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            }
        }
        _ => String::new(),
    }
}

/// Unavailable or stopped feeds resolve to empty text so stale values never show as live.
fn resolve_countdown_timer(
    style: &serde_json::Map<String, Value>,
    context: &Value,
    now: chrono::DateTime<Local>,
) -> String {
    let Some((timer, run)) = linked_countdown_timer(style, context) else {
        return String::new();
    };
    let milliseconds = timer_milliseconds(timer, run, now);
    let elapsed = timer.get("type").and_then(Value::as_str) == Some("elapsed")
        || run
            .and_then(|run| run.get("elapsed"))
            .and_then(Value::as_bool)
            == Some(true);
    format_linked_timer(style, milliseconds, elapsed, now)
}

fn format_linked_timer(
    style: &serde_json::Map<String, Value>,
    milliseconds: i64,
    elapsed: bool,
    now: chrono::DateTime<Local>,
) -> String {
    let formatted = style
        .get("linkedTextTimerFormat")
        .and_then(|format| {
            format_wall_clock_timer(milliseconds, elapsed, now, format)
                .or_else(|| format_timer_value_with_style(milliseconds, format))
        })
        .unwrap_or_else(|| format_timer_value(milliseconds));
    let custom_text = style
        .get("linkedTextTimerFormatString")
        .and_then(Value::as_str)
        .unwrap_or_default();
    apply_linked_template(&formatted, custom_text, "${timer}")
}

fn apply_linked_template(value: &str, template: &str, placeholder: &str) -> String {
    if template.contains(placeholder) {
        template.replace(placeholder, value)
    } else {
        format!("{value}{template}")
    }
}

fn video_countdown_milliseconds(
    style: &serde_json::Map<String, Value>,
    context: &Value,
    now: chrono::DateTime<Local>,
) -> Option<i64> {
    let source = style
        .get("videoCountdownSource")
        .and_then(Value::as_str)
        .unwrap_or("presentation");
    let video = context.get("videoCountdowns")?.get(source)?;
    let looping = video["looping"] == true;
    if looping
        && style
            .get("videoCountdownIgnoreLooping")
            .and_then(Value::as_bool)
            == Some(true)
    {
        return None;
    }
    let end = video["outSeconds"]
        .as_f64()
        .or_else(|| video["duration"].as_f64())?;
    let start = video["inSeconds"].as_f64().unwrap_or(0.0);
    let mut position = video["currentTime"].as_f64()?;
    if video["status"] == "playing" && video["delayed"] != true {
        let captured = video["capturedAt"].as_i64()?;
        position += (now.timestamp_millis() - captured).max(0) as f64 / 1000.0
            * video["rate"].as_f64().unwrap_or(1.0);
    }
    if !end.is_finite() || !position.is_finite() || end <= start {
        return None;
    }
    if looping {
        position = start + (position - start).rem_euclid(end - start);
    }
    Some(((end - position).max(0.0) * 1000.0).ceil() as i64)
}

/// Returns the active color trigger for linked timer text, if a threshold has been reached.
/// Trigger times are stored in seconds; countdown triggers apply as time remaining decreases,
/// while elapsed timers apply as their displayed time increases.
pub(crate) fn resolve_countdown_timer_color(
    style: &serde_json::Map<String, Value>,
    context: &Value,
    now: chrono::DateTime<Local>,
) -> Option<String> {
    if style.get("linkedTextEnabled").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let triggers = style.get("linkedTextTimerColorTriggers")?.as_array()?;
    let (milliseconds, elapsed) =
        if style.get("linkedTextSource").and_then(Value::as_str) == Some("Video Countdown") {
            (
                video_countdown_milliseconds(style, context, now)? as f64,
                false,
            )
        } else if style.get("linkedTextSource").and_then(Value::as_str) == Some("Countdown Timer") {
            let (timer, run) = linked_countdown_timer(style, context)?;
            (
                timer_milliseconds(timer, run, now) as f64,
                timer.get("type").and_then(Value::as_str) == Some("elapsed")
                    || run
                        .and_then(|run| run.get("elapsed"))
                        .and_then(Value::as_bool)
                        == Some(true),
            )
        } else {
            return None;
        };
    let mut selected: Option<(f64, String)> = None;

    for trigger in triggers {
        let Some(threshold) = trigger
            .get("time")
            .and_then(Value::as_f64)
            .map(|time| time * 1000.0)
        else {
            continue;
        };
        let Some(color) = trigger.get("color").and_then(Value::as_str) else {
            continue;
        };
        if !threshold.is_finite() || crate::native_compositor::parse_color(color).is_none() {
            continue;
        }
        let reached = if elapsed {
            threshold <= milliseconds
        } else {
            threshold >= milliseconds
        };
        let closer = selected.as_ref().is_none_or(|(current, _)| {
            if elapsed {
                threshold > *current
            } else {
                threshold < *current
            }
        });
        if reached && closer {
            selected = Some((threshold, color.to_string()));
        }
    }
    selected.map(|(_, color)| color)
}

fn linked_countdown_timer<'a>(
    style: &serde_json::Map<String, Value>,
    context: &'a Value,
) -> Option<(&'a Value, Option<&'a Value>)> {
    let timers = context.get("countdownTimers").and_then(Value::as_array)?;
    let selected_id = [
        "linkedTextTimerId",
        "linkedTextCountdownTimerId",
        "linkedTextSourceId",
    ]
    .iter()
    .find_map(|key| style.get(*key).and_then(Value::as_str))
    .filter(|id| !id.is_empty());
    let timer = match selected_id {
        Some(id) => timers
            .iter()
            .find(|timer| timer.get("id").and_then(Value::as_str) == Some(id)),
        None => timers.first(),
    }?;
    let timer_id = timer.get("id").and_then(Value::as_str).unwrap_or_default();
    let run = context
        .get("countdownTimerRuns")
        .and_then(|runs| runs.get(timer_id));
    Some((timer, run))
}

/// `element_text` resolves another object's displayed text; a condition whose data is unavailable satisfies no criterion.
pub(crate) fn visibility_link_visible(
    link: &Value,
    context: &Value,
    element_text: &dyn Fn(&str) -> Option<String>,
) -> bool {
    let Some(conditions) = link.get("conditions").and_then(Value::as_array) else {
        return false;
    };
    let now = Local::now();
    let test = |condition: &Value| -> Option<bool> {
        let criterion = condition.get("criterion").and_then(Value::as_str)?;
        match condition
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("timer")
        {
            "timer" => {}
            "element" => {
                let text = element_text(condition.get("objectId")?.as_str()?)?;
                return Some(if criterion == "has-text" {
                    !text.is_empty()
                } else {
                    text.is_empty()
                });
            }
            "capture" => {
                let active = context.get("captureStatus")?.get("active")?.as_bool()?;
                return Some(if criterion == "active" {
                    active
                } else {
                    !active
                });
            }
            "video-input" => {
                let id = condition.get("inputId")?.as_str()?;
                let active = context.get("videoInputActivity")?.get(id)?.as_bool()?;
                return Some(if criterion == "active" {
                    active
                } else {
                    !active
                });
            }
            kind @ ("video" | "audio") => {
                let source = if kind == "video" {
                    context.get("videoCountdowns")?.get("presentation")?
                } else {
                    context.get("audioCountdown")?
                };
                if source.is_null() {
                    return None;
                }
                return match criterion {
                    "looping" => source.get("looping")?.as_bool(),
                    "not-looping" => source.get("looping")?.as_bool().map(|looping| !looping),
                    "is-running" => Some(source.get("status")?.as_str()? == "playing"),
                    "not-running" => Some(source.get("status")?.as_str()? != "playing"),
                    _ => {
                        let remaining = video_countdown_milliseconds(
                            &serde_json::Map::new(),
                            &serde_json::json!({"videoCountdowns":{"presentation":source}}),
                            now,
                        )?;
                        Some(if criterion == "has-time-remaining" {
                            remaining > 0
                        } else {
                            remaining <= 0
                        })
                    }
                };
            }
            _ => return None,
        }
        let id = condition.get("timerId").and_then(Value::as_str)?;
        let timer = context
            .get("countdownTimers")
            .and_then(Value::as_array)
            .and_then(|timers| {
                timers
                    .iter()
                    .find(|timer| timer.get("id").and_then(Value::as_str) == Some(id))
            })?;
        let run = context
            .get("countdownTimerRuns")
            .and_then(|runs| runs.get(id));
        let running =
            run.is_some_and(|run| run.get("paused").and_then(Value::as_bool) != Some(true));
        let remaining = timer_milliseconds(timer, run, now);
        let expired = if timer.get("type").and_then(Value::as_str) == Some("elapsed") {
            timer
                .get("elapsedEndSeconds")
                .and_then(Value::as_f64)
                .is_some_and(|end| end > 0.0 && remaining as f64 >= end * 1000.0)
        } else {
            remaining <= 0
        };
        match criterion {
            "has-time-remaining" => Some(!expired),
            "has-expired" => Some(expired),
            "is-running" => Some(running),
            "not-running" => Some(!running),
            _ => None,
        }
    };
    match link.get("criterion").and_then(Value::as_str) {
        Some("all") => conditions
            .iter()
            .all(|condition| test(condition) == Some(true)),
        Some("any") => conditions
            .iter()
            .any(|condition| test(condition) == Some(true)),
        Some("none") => conditions
            .iter()
            .all(|condition| test(condition) == Some(false)),
        _ => false,
    }
}

fn timer_milliseconds(timer: &Value, run: Option<&Value>, now: chrono::DateTime<Local>) -> i64 {
    let timer_type = timer
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("countdown");
    let allows_overrun = timer
        .get("allowsOverrun")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let now_ms = now.timestamp_millis();
    let value = if let Some(run) = run {
        let remaining = json_i64(run.get("remaining")).unwrap_or(0);
        if run.get("paused").and_then(Value::as_bool) == Some(true) {
            remaining
        } else {
            let end = json_i64(run.get("end")).unwrap_or(now_ms);
            if timer_type == "elapsed" || run.get("elapsed").and_then(Value::as_bool) == Some(true)
            {
                now_ms.saturating_sub(end).max(0)
            } else {
                end.saturating_sub(now_ms)
            }
        }
    } else if timer_type == "elapsed" {
        json_seconds(timer.get("elapsedStartSeconds"))
    } else if timer_type == "countdown-to-time" {
        countdown_to_time_milliseconds(timer, now)
    } else {
        json_seconds(timer.get("durationSeconds"))
    };

    if timer_type == "elapsed" {
        let end = timer
            .get("elapsedEndSeconds")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| (value * 1000.0) as i64);
        if allows_overrun {
            value
        } else {
            end.map_or(value, |end| value.min(end))
        }
    } else if allows_overrun {
        value
    } else {
        value.max(0)
    }
}

fn countdown_to_time_milliseconds(timer: &Value, now: chrono::DateTime<Local>) -> i64 {
    let Some(target_seconds) = timer
        .get("targetTimeSeconds")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
    else {
        return json_seconds(timer.get("durationSeconds"));
    };
    let midnight = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|value| value.and_local_timezone(Local).earliest());
    let Some(midnight) = midnight else {
        return json_seconds(timer.get("durationSeconds"));
    };
    let target = midnight + chrono::Duration::milliseconds((target_seconds * 1000.0) as i64);
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
        .filter(|value| value.is_finite())
        .map(|value| (value * 1000.0) as i64)
        .unwrap_or(0)
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

fn format_timer_value(milliseconds: i64) -> String {
    let negative = milliseconds < 0;
    let total = milliseconds.unsigned_abs() / 1000;
    let seconds = total % 60;
    let minutes = (total / 60) % 60;
    let hours = total / 3600;
    let value = if hours > 0 {
        format!("{hours}:{minutes}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    };
    if negative {
        format!("−{value}")
    } else {
        value
    }
}

/// Wall-clock timers show the local time of day at which the timer reads zero: now plus the
/// remaining time for countdowns, now minus the elapsed time for elapsed timers.
fn format_wall_clock_timer(
    milliseconds: i64,
    elapsed: bool,
    now: chrono::DateTime<Local>,
    format: &Value,
) -> Option<String> {
    let format = format.as_object()?;
    if format.get("wallClock").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let delta = if elapsed { -milliseconds } else { milliseconds };
    let time = now + chrono::Duration::milliseconds(delta);
    let padded = |key: &str| {
        matches!(
            format.get(key).and_then(Value::as_str),
            Some("long" | "remove_long")
        )
    };
    let shows_seconds = !matches!(
        format.get("second").and_then(Value::as_str),
        None | Some("none")
    );
    let use_24_hour = format.get("use24Hour").and_then(Value::as_bool) == Some(true);
    let hour = if use_24_hour {
        time.hour()
    } else {
        (time.hour() + 11) % 12 + 1
    };
    let mut output = if padded("hour") {
        format!("{hour:02}")
    } else {
        hour.to_string()
    };
    output.push_str(&format!(":{:02}", time.minute()));
    if shows_seconds {
        output.push_str(&format!(":{:02}", time.second()));
    }
    if !use_24_hour {
        output.push_str(if time.hour() < 12 { " AM" } else { " PM" });
    }
    Some(output)
}

fn format_timer_value_with_style(milliseconds: i64, format: &Value) -> Option<String> {
    let format = format.as_object()?;

    #[derive(Clone, Copy, PartialEq)]
    enum UnitStyle {
        None,
        Short,
        Long,
        RemoveShort,
        RemoveLong,
    }

    impl UnitStyle {
        fn parse(value: Option<&Value>) -> Option<Self> {
            match value {
                None | Some(Value::Null) => Some(Self::None),
                Some(Value::String(value)) => match value.as_str() {
                    "none" => Some(Self::None),
                    "short" => Some(Self::Short),
                    "long" => Some(Self::Long),
                    "remove_short" => Some(Self::RemoveShort),
                    "remove_long" => Some(Self::RemoveLong),
                    _ => None,
                },
                _ => None,
            }
        }

        fn is_long(self) -> bool {
            matches!(self, Self::Long | Self::RemoveLong)
        }

        fn removes_zero(self) -> bool {
            matches!(self, Self::RemoveShort | Self::RemoveLong)
        }
    }

    let mut styles = ["hour", "minute", "second", "millisecond"]
        .map(|key| UnitStyle::parse(format.get(key)))
        .into_iter()
        .collect::<Option<Vec<_>>>()?;
    if ["wallClock", "use24Hour", "millisecondsUnderMinuteOnly"]
        .iter()
        .any(|key| format.get(*key).is_some_and(|value| !value.is_boolean()))
    {
        return None;
    }
    if format
        .get("millisecondsUnderMinuteOnly")
        .and_then(Value::as_bool)
        == Some(true)
        && milliseconds.unsigned_abs() >= 60_000
    {
        styles[3] = UnitStyle::None;
    }
    // ProPresenter keeps the seconds when milliseconds are displayed without a seconds style.
    if styles[2] == UnitStyle::None && styles[3] != UnitStyle::None {
        styles[2] = UnitStyle::Short;
    }
    let mut values = [
        milliseconds.unsigned_abs() / 3_600_000,
        (milliseconds.unsigned_abs() / 60_000) % 60,
        (milliseconds.unsigned_abs() / 1_000) % 60,
        (milliseconds.unsigned_abs() % 1_000) / 10,
    ];

    for index in 0..2 {
        if styles[index] == UnitStyle::None {
            values[index + 1] = values[index + 1].saturating_add(values[index].saturating_mul(60));
            values[index] = 0;
        }
    }

    let mut parts = Vec::new();
    for (index, (style, value)) in styles.iter().copied().zip(values).enumerate() {
        if style == UnitStyle::None {
            continue;
        }
        let width = match index {
            0..=2 if style.is_long() => 2,
            3 if style.is_long() => 2,
            _ => 0,
        };
        let displayed = if index == 3 && !style.is_long() {
            value / 10
        } else {
            value
        };
        if style.removes_zero() && displayed == 0 {
            continue;
        }
        let rendered = if width == 0 {
            displayed.to_string()
        } else {
            format!("{displayed:0width$}")
        };
        parts.push((index, rendered));
    }

    // ProPresenter displays total seconds when every unit style is hidden.
    if parts.is_empty() {
        if styles.iter().any(|style| *style != UnitStyle::None) {
            return Some(String::new());
        }
        let seconds = milliseconds.unsigned_abs() / 1_000;
        return Some(if milliseconds < 0 {
            format!("−{seconds}")
        } else {
            seconds.to_string()
        });
    }

    let negative = milliseconds < 0 && !parts.is_empty();
    let mut output = String::new();
    if negative {
        output.push('−');
    }
    for (position, (index, value)) in parts.iter().enumerate() {
        if position > 0 {
            output.push(if *index == 3 { '.' } else { ':' });
        }
        output.push_str(value);
    }
    Some(output)
}

fn linked_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn linked_text_value(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Object(value)) => value
            .get("content")
            .or_else(|| value.get("text"))
            .or_else(|| value.get("value"))
            .and_then(|value| linked_string(Some(value))),
        _ => None,
    }
}

fn context_slide_text(
    context: &Value,
    object_key: &str,
    alternate_object_key: &str,
    flat_key: &str,
) -> Option<String> {
    context
        .get(object_key)
        .or_else(|| context.get(alternate_object_key))
        .and_then(|value| linked_text_value(Some(value)))
        .or_else(|| linked_string(context.get(flat_key)))
}

fn context_slide_field(
    context: &Value,
    object_key: &str,
    alternate_object_key: &str,
    field: &str,
) -> Option<String> {
    context
        .get(object_key)
        .or_else(|| context.get(alternate_object_key))
        .and_then(|value| value.get(field))
        .and_then(|value| linked_string(Some(value)))
}

fn resolve_group_text(style: &serde_json::Map<String, Value>, context: &Value) -> String {
    let source = style
        .get("linkedTextGroupSource")
        .and_then(Value::as_str)
        .unwrap_or("Current Slide");
    let (object_key, flat_key) = match source {
        "Next Slide" => ("nextSlide", "nextGroupName"),
        "Next in Arrangement" => ("nextArrangementSlide", "nextArrangementGroupName"),
        _ => ("slide", "groupName"),
    };
    let group = context
        .get(object_key)
        .and_then(|value| value.get("groupName").or_else(|| value.get("groupId")))
        .and_then(|value| linked_string(Some(value)))
        .or_else(|| linked_string(context.get(flat_key)))
        .or_else(|| {
            let fallback_key = match source {
                "Next Slide" => "nextGroupId",
                "Next in Arrangement" => "nextArrangementGroupId",
                _ => "groupId",
            };
            linked_string(context.get(fallback_key))
        })
        .unwrap_or_default();
    format_group_name(&group)
}

fn resolve_slide_count(style: &serde_json::Map<String, Value>, context: &Value) -> String {
    let source = style
        .get("linkedTextSlideCountSource")
        .and_then(Value::as_str)
        .unwrap_or("Current");
    if let Some(counts) = context.get("slideCount").and_then(Value::as_object) {
        if let Some(value) = counts.get(match source {
            "Remaining" => "remaining",
            "Total" => "total",
            _ => "current",
        }) {
            if let Some(value) = linked_string(Some(value)) {
                return value;
            }
        }
    }
    let keys = match source {
        "Remaining" => ["slideCountRemaining", "remainingSlideCount", "remaining"],
        "Total" => ["slideCountTotal", "totalSlideCount", "total"],
        _ => ["slideCountCurrent", "currentSlideNumber", "current"],
    };
    for key in keys {
        if let Some(value) = linked_string(context.get(key)) {
            return value;
        }
    }
    if let Some(slides) = context
        .get("item")
        .and_then(|item| item.get("slides"))
        .and_then(Value::as_array)
    {
        let current_id = context
            .get("slide")
            .and_then(|slide| slide.get("id"))
            .and_then(|value| linked_string(Some(value)));
        if let Some(index) = current_id.and_then(|id| {
            slides.iter().position(|slide| {
                slide
                    .get("id")
                    .and_then(|value| linked_string(Some(value)))
                    .is_some_and(|candidate| candidate == id)
            })
        }) {
            return match source {
                "Remaining" => format_count(slides.len().saturating_sub(index + 1) as f64),
                "Total" => format_count(slides.len() as f64),
                _ => format_count((index + 1) as f64),
            };
        }
    }
    if source == "Current" {
        for key in ["currentSlideIndex", "slideIndex"] {
            if let Some(value) = json_number(context.get(key)) {
                return format_count(value + 1.0);
            }
        }
    }
    String::new()
}

fn format_group_name(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut separator = false;
    for character in value.chars() {
        if matches!(character, '-' | '_') {
            if !separator {
                normalized.push(' ');
            }
            separator = true;
        } else {
            normalized.push(character);
            separator = false;
        }
    }
    capitalize_word_boundaries(&normalized)
}

fn format_count(value: f64) -> String {
    if value.is_finite() && value.fract().abs() < f64::EPSILON {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

fn format_system_clock(style: &serde_json::Map<String, Value>) -> String {
    let now = Local::now();
    let date_format = style
        .get("systemClockDateFormat")
        .and_then(Value::as_str)
        .unwrap_or("numeric");
    let time_format = style
        .get("systemClockTimeFormat")
        .and_then(Value::as_str)
        .unwrap_or("hours-minutes");
    let use_24_hour = style
        .get("systemClockUse24Hour")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let date = match date_format {
        "none" => String::new(),
        "short-month" => format!(
            "{} {}, {}",
            short_month_name(now.month()),
            now.day(),
            now.year()
        ),
        "long-month" => format!(
            "{} {}, {}",
            long_month_name(now.month()),
            now.day(),
            now.year()
        ),
        "day-long-month" => format!(
            "{} {} {}",
            now.day(),
            long_month_name(now.month()),
            now.year()
        ),
        "full" => format!(
            "{}, {} {} {}",
            long_weekday_name(now.weekday()),
            now.day(),
            long_month_name(now.month()),
            now.year()
        ),
        _ => format!("{:02}/{:02}/{}", now.day(), now.month(), now.year()),
    };
    let time = format_system_clock_time(&now, time_format, use_24_hour);
    let formatted = [date, time]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    apply_linked_template(
        &formatted,
        style
            .get("systemClockFormatString")
            .and_then(Value::as_str)
            .unwrap_or(""),
        "${clock}",
    )
}

fn format_system_clock_time(
    now: &chrono::DateTime<Local>,
    format: &str,
    use_24_hour: bool,
) -> String {
    if format == "none" {
        return String::new();
    }
    let show_seconds = matches!(
        format,
        "seconds" | "hours-minutes-seconds" | "seconds-zone-short" | "seconds-zone-long"
    );
    let zone = matches!(format, "seconds-zone-short" | "seconds-zone-long");
    let time = if use_24_hour {
        if show_seconds {
            format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second())
        } else {
            format!("{:02}:{:02}", now.hour(), now.minute())
        }
    } else {
        let hour = match now.hour() % 12 {
            0 => 12,
            value => value,
        };
        let meridiem = if now.hour() < 12 { "AM" } else { "PM" };
        if show_seconds {
            format!(
                "{}:{:02}:{:02} {}",
                hour,
                now.minute(),
                now.second(),
                meridiem
            )
        } else {
            format!("{}:{:02} {}", hour, now.minute(), meridiem)
        }
    };
    if zone {
        let zone_name = now.format("%Z").to_string();
        if zone_name.is_empty() {
            time
        } else {
            format!("{time} {zone_name}")
        }
    } else {
        time
    }
}

fn short_month_name(month: u32) -> &'static str {
    [
        "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .get(month as usize)
    .copied()
    .unwrap_or("")
}

fn long_month_name(month: u32) -> &'static str {
    [
        "",
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ]
    .get(month as usize)
    .copied()
    .unwrap_or("")
}

fn long_weekday_name(weekday: chrono::Weekday) -> &'static str {
    match weekday {
        chrono::Weekday::Mon => "Monday",
        chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday",
        chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday",
        chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    }
}

pub(crate) fn format_ccli_text(value: Option<&Value>) -> String {
    if value.is_some_and(|value| value["display"] == false) {
        return String::new();
    }
    let Some(value) = value.and_then(Value::as_object) else {
        return String::new();
    };
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let author = value
        .get("author")
        .or_else(|| value.get("artistCredits"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let copyright = [
        value
            .get("copyrightYear")
            .and_then(Value::as_u64)
            .map(|year| year.to_string()),
        value
            .get("publisher")
            .and_then(Value::as_str)
            .map(str::to_string),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    let song_number = value
        .get("songNumber")
        .and_then(Value::as_u64)
        .map(|number| format!("CCLI Song #{number}"))
        .unwrap_or_default();
    [title, author, &copyright, &song_number]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn apply_capitalization(text: &str, capitalization: &str) -> String {
    match capitalization {
        "All Caps" => text.to_uppercase(),
        "Start Case" => capitalize_word_boundaries(text),
        "Title Case" => capitalize_word_boundaries(&text.to_lowercase()),
        _ => text.to_string(),
    }
}

fn capitalize_word_boundaries(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut previous_ascii_word = false;
    for character in text.chars() {
        let current_ascii_word = character.is_ascii_alphanumeric() || character == '_';
        if character.is_alphabetic() && current_ascii_word != previous_ascii_word {
            output.extend(character.to_uppercase());
        } else {
            output.push(character);
        }
        previous_ascii_word = current_ascii_word;
    }
    output
}

pub(crate) fn apply_line_transform(
    text: &str,
    line_transform: &str,
    delimiter: Option<&str>,
) -> String {
    match line_transform {
        "Remove line returns" => text
            .chars()
            .filter(|character| !matches!(character, '\r' | '\n' | '\u{2028}'))
            .collect(),
        "Replace line returns" => {
            let mut output = String::with_capacity(text.len());
            let mut previous_return = false;
            for character in text.chars() {
                if matches!(character, '\r' | '\n' | '\u{2028}') {
                    if !previous_return {
                        output.push_str(delimiter.unwrap_or(" "));
                    }
                    previous_return = true;
                } else {
                    output.push(character);
                    previous_return = false;
                }
            }
            output
        }
        "One word per line" => text
            .split_whitespace()
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        "One character per line" => text
            .chars()
            .filter(|character| !character.is_whitespace())
            .map(|character| character.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => text.to_string(),
    }
}

/// Keeps each character's run formatting while applying the same transforms as the text renderer.
pub(crate) fn transform_text_runs(
    runs: &[Value],
    linked: Option<&str>,
    capitalization: &str,
    graphic: &str,
    delimiter: Option<&str>,
) -> Option<Vec<Value>> {
    let mut chars = Vec::<(char, Value)>::new();
    for run in runs {
        let text = run.get("text")?.as_str()?;
        for character in text.chars() {
            chars.push((character, run.clone()));
        }
    }
    fn line(chars: Vec<(char, Value)>, mode: &str, delimiter: &str) -> Vec<(char, Value)> {
        let mut result = Vec::new();
        let mut pending: Option<Value> = None;
        for (character, run) in chars {
            match mode {
                "Remove line returns" if matches!(character, '\r' | '\n' | '\u{2028}') => continue,
                "Replace line returns" if matches!(character, '\r' | '\n' | '\u{2028}') => {
                    if pending.is_none() {
                        pending = Some(run);
                    }
                    continue;
                }
                "One word per line" if character.is_whitespace() => {
                    if !result.is_empty() && pending.is_none() {
                        pending = Some(run);
                    }
                    continue;
                }
                "One character per line" if character.is_whitespace() => continue,
                _ => {}
            }
            if let Some(format) = pending.take() {
                let separator = if mode == "One word per line" {
                    "\n"
                } else {
                    delimiter
                };
                result.extend(separator.chars().map(|c| (c, format.clone())));
            } else if mode == "One character per line" && !result.is_empty() {
                result.push(('\n', run.clone()));
            }
            result.push((character, run));
        }
        if mode == "Replace line returns" {
            if let Some(format) = pending {
                result.extend(delimiter.chars().map(|c| (c, format.clone())));
            }
        }
        result
    }
    if let Some(mode) = linked {
        chars = line(chars, mode, " ");
    }
    if capitalization == "Title Case" {
        let lower = chars
            .iter()
            .map(|(c, _)| *c)
            .collect::<String>()
            .to_lowercase();
        let formats = chars
            .into_iter()
            .flat_map(|(c, format)| c.to_lowercase().map(move |_| format.clone()))
            .collect::<Vec<_>>();
        chars = lower.chars().zip(formats).collect();
    }
    if capitalization == "All Caps" {
        chars = chars
            .into_iter()
            .flat_map(|(c, format)| {
                let converted: Vec<char> = c.to_uppercase().collect();
                converted.into_iter().map(move |c| (c, format.clone()))
            })
            .collect();
    }
    if capitalization == "Start Case" || capitalization == "Title Case" {
        let mut previous_ascii_word = false;
        chars = chars
            .into_iter()
            .flat_map(|(c, format)| {
                let word = c.is_ascii_alphanumeric() || c == '_';
                let uppercase = c.is_alphabetic() && word != previous_ascii_word;
                previous_ascii_word = word;
                let converted: Vec<char> = if uppercase {
                    c.to_uppercase().collect()
                } else {
                    vec![c]
                };
                converted.into_iter().map(move |c| (c, format.clone()))
            })
            .collect();
    }
    chars = line(chars, graphic, delimiter.unwrap_or(" "));
    let mut result: Vec<Value> = Vec::new();
    for (character, mut run) in chars {
        let object = run.as_object_mut()?;
        object.insert("text".into(), Value::String(character.to_string()));
        if let Some(last) = result
            .last_mut()
            .filter(|last| last.get("style") == run.get("style"))
        {
            last.get_mut("text")?.as_str()?;
            let text = last.get_mut("text")?.as_str()?.to_string() + &character.to_string();
            last["text"] = Value::String(text);
        } else {
            result.push(run);
        }
    }
    Some(result)
}

fn json_number(value: Option<&Value>) -> Option<f64> {
    let parsed = match value? {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    parsed.is_finite().then_some(parsed)
}
