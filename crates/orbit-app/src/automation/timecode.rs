//! Timecode chase runs in the UI controller; it never opens a device in tests.
use crate::services::entries;
use crate::state::runtime::Runtime;
use gpui::{App, Global};
use serde_json::{json, Value};
#[derive(Default)]
pub struct Chase {
    path: String,
    target: Option<Value>,
    generation: String,
    last: Option<f64>,
    pub status: Value,
}
impl Global for Chase {}
fn rounded(n: f64) -> f64 {
    (n * 1000.).round() / 1000.
}
pub fn chase_position(timeline: &Value, seconds: f64) -> Option<f64> {
    let raw = rounded(seconds - timeline["timecodeOffsetSeconds"].as_f64()?);
    let duration = timeline["durationSeconds"].as_f64()?;
    if raw < 0. || (timeline["loop"] != true && raw >= duration) {
        return None;
    }
    Some(if timeline["loop"] == true {
        rounded(raw.rem_euclid(duration))
    } else {
        raw
    })
}
pub fn step(
    timeline: &Value,
    transport: &Value,
    reading: Option<&Value>,
    last: Option<f64>,
) -> Option<Value> {
    let transport = if transport["mode"] == "playback" {
        transport
    } else {
        &Value::Null
    };
    let running = transport.is_object() && transport["startedAtMs"].is_number();
    let Some(reading) = reading else {
        return if running {
            last.map(|p| json!({"type":"pause","position":p}))
        } else {
            None
        };
    };
    let Some(position) = chase_position(timeline, reading["seconds"].as_f64()?) else {
        return running.then(|| json!({"type":"stop"}));
    };
    if reading["running"] != true {
        return (running
            || (transport.is_object()
                && (transport["positionSeconds"].as_f64().unwrap_or(0.) - position).abs()
                    > 0.001))
            .then(|| json!({"type":"pause","position":position}));
    }
    let rate =
        reading["speed"].as_f64().unwrap_or(1.) * if reading["reverse"] == true { -1. } else { 1. };
    let difference = |from: f64, to: f64| {
        let delta = to - from;
        if timeline["loop"] != true {
            return delta;
        }
        let duration = timeline["durationSeconds"].as_f64().unwrap_or(0.);
        if delta > duration / 2. {
            delta - duration
        } else if delta < -duration / 2. {
            delta + duration
        } else {
            delta
        }
    };
    let kind = if !transport.is_object() {
        "play"
    } else if !running {
        if difference(
            transport["positionSeconds"].as_f64().unwrap_or(0.),
            position,
        )
        .abs()
            <= 0.5
        {
            "resume"
        } else {
            "play"
        }
    } else {
        let mut expected = super::timeline::position(transport, reading["atMs"].as_f64()?);
        if timeline["loop"] == true {
            expected = expected.rem_euclid(timeline["durationSeconds"].as_f64()?);
        }
        let delta = difference(expected, position).abs();
        if delta > 0.5 {
            "play"
        } else if delta > 0.04 + 1e-9
            || (transport["playbackRate"].as_f64().unwrap_or(1.) - rate).abs() > 0.05
        {
            "sync"
        } else {
            return None;
        }
    };
    Some(json!({"type":kind,"position":position,"rate":rate,"at":reading["atMs"]}))
}
pub fn target(
    runtime: &Runtime,
    playlist: &str,
    seconds: f64,
    cx: &App,
) -> Result<Option<Value>, String> {
    let facts = super::facts(runtime, cx);
    let workspace = &facts.workspace;
    let mut targets = Vec::new();
    let mut add = |item: &Value, entry: Option<&Value>| -> Result<(), String> {
        let timeline = &item["timeline"];
        if timeline["timecodeEnabled"] != true {
            return Ok(());
        }
        super::timeline::validate(&facts, item, timeline)?;
        targets.push(json!({"item":item,"timeline":timeline,"playlistId":if playlist.is_empty(){Value::Null}else{json!(playlist)},"entry":entry}));
        Ok(())
    };
    if playlist.is_empty() {
        let selected = &runtime.ui.read(cx).selected_library_item_id;
        if let Some(item) = entries(&workspace["library"])
            .iter()
            .find(|i| i["id"] == *selected)
        {
            add(item, None)?;
        }
    } else {
        let lists = entries(&workspace["playlist"]);
        let list = lists
            .iter()
            .find(|p| p["id"] == playlist && p["type"] != "folder")
            .ok_or("The timecode playlist is unavailable.")?;
        for entry in entries(&list["items"])
            .iter()
            .filter(|e| e["type"] == "presentation")
        {
            if let Some(item) = entries(&workspace["library"])
                .iter()
                .find(|i| i["id"] == entry["itemId"])
            {
                add(item, Some(entry))?;
            }
        }
        targets.sort_by(|a, b| {
            a["timeline"]["timecodeOffsetSeconds"]
                .as_f64()
                .partial_cmp(&b["timeline"]["timecodeOffsetSeconds"].as_f64())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for pair in targets.windows(2) {
            let previous = &pair[0]["timeline"];
            if previous["loop"] == true
                || previous["timecodeOffsetSeconds"].as_f64().unwrap_or(0.)
                    + previous["durationSeconds"].as_f64().unwrap_or(0.)
                    > pair[1]["timeline"]["timecodeOffsetSeconds"]
                        .as_f64()
                        .unwrap_or(0.)
            {
                return Err("Timecode playlist timelines must have distinct, non-overlapping ranges; only the last may loop.".into());
            }
        }
    }
    Ok(targets
        .into_iter()
        .find(|t| playlist.is_empty() || chase_position(&t["timeline"], seconds).is_some()))
}
pub fn event(runtime: &Runtime, name: &str, payload: &Value, cx: &mut App) {
    if name == "timecode-reading"
        && (payload["seconds"].as_f64().is_none_or(|n| !n.is_finite())
            || payload["atMs"].as_i64().is_none()
            || !payload["targetPlaylistId"].is_string()
            || !payload["running"].is_boolean()
            || !payload["reverse"].is_boolean())
    {
        crate::cues::actions::error(runtime, cx, "Invalid timecode reading.".into());
        return;
    }
    if !cx.has_global::<Chase>() {
        cx.set_global(Chase::default());
    }
    if name == "timecode-status" {
        cx.global_mut::<Chase>().status = payload.clone();
        runtime.output.update(cx, |_, cx| cx.notify());
    }
    if name != "timecode-reading" && !(name == "timecode-status" && payload["locked"] == false) {
        return;
    }
    let path = runtime.content.read(cx).workspace_path.clone();
    let state = cx.global_mut::<Chase>();
    if state.path != path {
        *state = Chase {
            path: path.clone(),
            ..Default::default()
        };
    }
    let previous = state.target.clone();
    let owned = state.generation.clone();
    let last = state.last;
    let live = runtime.output.read(cx).live.clone();
    let transport = &live["timelineTransport"];
    if path.is_empty() || transport["mode"] == "recording" {
        return;
    }
    let reading = (name == "timecode-reading").then_some(payload);
    let result = reading.map_or(Ok(previous.clone()), |r| {
        target(
            runtime,
            r["targetPlaylistId"].as_str().unwrap_or(""),
            r["seconds"].as_f64().unwrap_or(0.),
            cx,
        )
    });
    let target = match result {
        Ok(t) => t,
        Err(e) => {
            crate::cues::actions::error(runtime, cx, e);
            previous.clone()
        }
    };
    let Some(target) = target else {
        if transport["generation"] == owned && !owned.is_empty() {
            let mut next = transport.clone();
            next["mode"] = json!("stopped");
            next["startedAtMs"] = Value::Null;
            super::timeline::apply(runtime, json!({"timelineTransport":next}), cx);
        }
        cx.global_mut::<Chase>().target = None;
        return;
    };
    let same = previous.as_ref().is_some_and(|p| {
        p["item"]["id"] == target["item"]["id"]
            && p["entry"] == target["entry"]
            && p["playlistId"] == target["playlistId"]
    }) && transport["generation"] == owned;
    let action = step(
        &target["timeline"],
        if same { transport } else { &Value::Null },
        reading,
        if same { last } else { None },
    );
    {
        let state = cx.global_mut::<Chase>();
        state.target = Some(target.clone());
        if let Some(r) = reading {
            state.last = chase_position(&target["timeline"], r["seconds"].as_f64().unwrap_or(0.));
        }
    }
    let Some(action) = action else {
        return;
    };
    let kind = action["type"].as_str().unwrap_or("");
    if kind == "play" {
        let generation = uuid::Uuid::new_v4().to_string();
        let mut item = target["item"].clone();
        if let Some(id) = target["entry"]["arrangementId"].as_str() {
            let id = crate::cues::model::existing_arrangement_id(&item, id).to_string();
            item["selectedArrangementId"] = json!(id);
        }
        match super::timeline::transport_patch(
            &item,
            &target["timeline"],
            action["position"].as_f64().unwrap_or(0.),
            true,
            action["at"].as_f64().unwrap_or(0.),
            action["rate"].as_f64().unwrap_or(1.),
            &generation,
        ) {
            Ok(mut patch) => {
                patch["livePlaylistId"] = target["playlistId"].clone();
                patch["livePlaylistEntryId"] = target["entry"]["id"].clone();
                cx.global_mut::<Chase>().generation = generation;
                super::timeline::apply(runtime, patch, cx);
            }
            Err(e) => crate::cues::actions::error(runtime, cx, e),
        }
    } else if same {
        let mut next = transport.clone();
        next["positionSeconds"] = action["position"].clone();
        next["startedAtMs"] = if matches!(kind, "pause" | "stop") {
            Value::Null
        } else {
            action["at"].clone()
        };
        if action["rate"].is_number() {
            next["playbackRate"] = action["rate"].clone();
        }
        if kind == "stop" {
            next["mode"] = json!("stopped");
            next["positionSeconds"] = json!(0);
        }
        super::timeline::apply(runtime, json!({"timelineTransport":next}), cx);
    }
}
