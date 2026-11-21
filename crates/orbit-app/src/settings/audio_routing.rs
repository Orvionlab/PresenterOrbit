//! The routing, source type, and override parts of `core/audio-input.ts`, plus the `audio-inputs-changed` event of
//! `audio-input-controller.ts`.

use std::collections::HashMap;

use gpui::Global;
use serde_json::{json, Map, Value};

use crate::collections::model::Snapshot;
use crate::state::runtime::Runtime;

pub const MAX_SOURCE_CHANNELS: u64 = 64;
pub const MAX_BUS_CHANNELS: u64 = 16;
pub const MAX_ROUTES: usize = 1024;

fn number_in(value: &Value, min: f64, max: f64, integer: bool) -> bool {
    value
        .as_f64()
        .is_some_and(|n| n.is_finite() && n >= min && n <= max && (!integer || n.fract() == 0.))
}

/// `audioInputRoutingError`.
pub fn routing_error(routing: &Value) -> Option<&'static str> {
    const INVALID: &str = "audioInputs.invalid";
    if !routing.is_object()
        || !number_in(
            &routing["sourceChannels"],
            1.,
            MAX_SOURCE_CHANNELS as f64,
            true,
        )
        || !number_in(&routing["busChannels"], 1., MAX_BUS_CHANNELS as f64, true)
    {
        return Some(INVALID);
    }
    let Some(routes) = routing["routes"]
        .as_array()
        .filter(|r| r.len() <= MAX_ROUTES)
    else {
        return Some(INVALID);
    };
    let (sources, buses) = (
        routing["sourceChannels"].as_f64().unwrap_or(0.),
        routing["busChannels"].as_f64().unwrap_or(0.),
    );
    let mut pairs = std::collections::HashSet::new();
    let ok = routes.iter().all(|r| {
        r.is_object()
            && number_in(&r["input"], 1., sources, true)
            && number_in(&r["bus"], 1., buses, true)
            && number_in(&r["levelDb"], -60., 6., false)
            && r["muted"].is_boolean()
            && pairs.insert(format!("{}:{}", r["input"], r["bus"]))
    });
    (!ok).then_some(INVALID)
}

/// `selectAudioInputSourceType`: switching between a device and an NDI sender drops the old source.
pub fn select_source_type(capture: &Value, ndi: bool) -> Value {
    let mut common = Map::new();
    for key in ["routing", "channels", "levelDb", "delayMs", "latencyMs"] {
        if !capture[key].is_null() {
            common.insert(key.into(), capture[key].clone());
        }
    }
    if ndi {
        common.insert("ndiSource".into(), json!({"name": "", "urlAddress": ""}));
    } else {
        common.insert("deviceName".into(), json!(""));
    }
    Value::Object(common)
}

/// `createAudioInputIdentityRouting`.
pub fn identity_routing(sources: u64, buses: u64) -> Value {
    let valid =
        (1..=MAX_SOURCE_CHANNELS).contains(&sources) && (1..=MAX_BUS_CHANNELS).contains(&buses);
    let count = if valid { sources.min(buses) } else { 0 };
    let routes: Vec<Value> = (1..=count)
        .map(|i| json!({"input": i, "bus": i, "levelDb": 0, "muted": false}))
        .collect();
    json!({"sourceChannels": sources, "busChannels": buses, "routes": routes})
}

/// `selectAudioInputRoutingMode`: stereo and matrix convert into each other, keeping the chosen channels.
pub fn select_routing_mode(capture: &Value, matrix: bool) -> Value {
    let mut next = capture.clone();
    let channels = capture["channels"].as_array().cloned();
    if matrix {
        if !capture["routing"].is_null() {
            return next;
        }
        let channels = channels.unwrap_or_default();
        let source = channels
            .iter()
            .filter_map(Value::as_u64)
            .max()
            .unwrap_or(1)
            .max(1);
        let routes: Vec<Value> = channels
            .iter()
            .enumerate()
            .map(|(i, input)| json!({"input": input, "bus": i + 1, "levelDb": 0, "muted": false}))
            .collect();
        if let Some(o) = next.as_object_mut() {
            o.remove("channels");
        }
        next["routing"] = json!({"sourceChannels": source, "busChannels": 2, "routes": routes});
    } else if channels.is_none() {
        if let Some(o) = next.as_object_mut() {
            o.remove("routing");
        }
        next["channels"] = json!([1, 2]);
    }
    next
}

/// `resizeAudioInputRouting`: an out-of-range size is kept as typed (and invalid); a valid one drops routes outside it.
pub fn resize_routing(routing: &Value, field: &str, value: f64) -> Value {
    let mut next = routing.clone();
    next[field] = json!(value);
    let max = if field == "sourceChannels" {
        MAX_SOURCE_CHANNELS
    } else {
        MAX_BUS_CHANNELS
    } as f64;
    if !number_in(&json!(value), 1., max, true) {
        return next;
    }
    let (sources, buses) = (
        next["sourceChannels"].as_f64().unwrap_or(0.),
        next["busChannels"].as_f64().unwrap_or(0.),
    );
    let kept: Vec<Value> = routing["routes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| {
            r["input"].as_f64().unwrap_or(f64::MAX) <= sources
                && r["bus"].as_f64().unwrap_or(f64::MAX) <= buses
        })
        .cloned()
        .collect();
    next["routes"] = Value::Array(kept);
    next
}

/// `toggleAudioInputRoute`.
pub fn toggle_route(routing: &Value, input: u64, bus: u64) -> Value {
    let mut next = routing.clone();
    let mut routes = routing["routes"].as_array().cloned().unwrap_or_default();
    match routes
        .iter()
        .position(|r| r["input"] == input && r["bus"] == bus)
    {
        Some(at) => {
            routes.remove(at);
        }
        None => routes.push(json!({"input": input, "bus": bus, "levelDb": 0, "muted": false})),
    }
    next["routes"] = Value::Array(routes);
    next
}

/// `updateAudioInputRoute`: the level or mute of one route.
pub fn update_route(routing: &Value, input: u64, bus: u64, update: Value) -> Value {
    let mut next = routing.clone();
    if let Some(route) = next["routes"].as_array_mut().and_then(|r| {
        r.iter_mut()
            .find(|r| r["input"] == input && r["bus"] == bus)
    }) {
        for (k, v) in update.as_object().into_iter().flatten() {
            route[k] = v.clone();
        }
    }
    next
}

/// `retainAudioInputOverrides`: an override survives only while its input is unchanged.
pub fn retain_overrides(
    previous: &[Value],
    next: &[Value],
    overrides: &HashMap<String, Value>,
) -> HashMap<String, Value> {
    overrides
        .iter()
        .filter(|(id, _)| {
            let find = |list: &[Value]| list.iter().find(|i| i["id"] == id.as_str()).cloned();
            matches!((find(previous), find(next)), (Some(a), Some(b)) if a == b)
        })
        .map(|(id, value)| (id.clone(), value.clone()))
        .collect()
}

/// The runtime overrides and the last revision of an `audio-inputs-changed` event that was taken, per workspace.
#[derive(Default)]
pub struct AudioInputState {
    pub overrides: HashMap<String, Value>,
    seen: HashMap<String, i64>,
}

impl Global for AudioInputState {}

/// `attachAudioInputChanges`: another window's saved inputs replace the content, unless the event is stale, for
/// another workspace, or already shown. Overrides of changed inputs are dropped.
pub fn accept_change(runtime: &Runtime, event: &Value, cx: &mut gpui::App) -> Result<(), String> {
    let path = event["workspacePath"].as_str().unwrap_or("");
    let Some(revision) = event["revision"].as_i64() else {
        return Ok(());
    };
    let (current_path, workspace) = {
        let content = runtime.content.read(cx);
        (content.workspace_path.clone(), content.workspace.clone())
    };
    let state = cx.default_global::<AudioInputState>();
    if current_path != path || revision < state.seen.get(path).copied().unwrap_or(-1) {
        return Ok(());
    }
    state.seen.insert(path.to_string(), revision);
    let Some(inputs) = event["inputs"].as_array() else {
        return Err("The audio input event has no inputs.".into());
    };
    let previous = workspace["audioInputs"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if *inputs == previous {
        return Ok(());
    }
    let retained = retain_overrides(&previous, inputs, &state.overrides);
    state.overrides = retained;
    let mut merged = (*workspace).clone();
    merged["audioInputs"] = Value::Array(inputs.clone());
    let snapshot = Snapshot::parse(&merged)?;
    runtime.content.update(cx, |content, cx| {
        content.apply_canonical(snapshot);
        cx.notify();
    });
    Ok(())
}
