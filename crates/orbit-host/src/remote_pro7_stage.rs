//! ProPresenter Stage Display messages served from Orbit's live status.

use chrono::Local;
use serde_json::{json, Value};

use super::{
    data::{array, text},
    Backend,
};

pub(super) fn error(message: &str) -> Value {
    json!({"acn": "err", "err": message})
}

/// Orbit stage layouts are free-form slides rendered natively, not ProPresenter
/// frame lists, so every layout is described with the same standard frames.
fn frames() -> Value {
    let frame = |name: &str, kind: u8, ufr: &str| json!({"ufr": ufr, "mde": 1, "tCl": "1.000000 1.000000 1.000000", "tAl": 2, "tSz": 60, "nme": name, "typ": kind});
    json!([
        frame("Current Slide", 1, "{{0.025, 0.374}, {0.4, 0.501}}"),
        frame("Current Slide Notes", 3, "{{0.024, 0.272}, {0.402, 0.104}}"),
        frame("Next Slide", 2, "{{0.45, 0.474}, {0.3, 0.4}}"),
        frame("Next Slide Notes", 4, "{{0.45, 0.373}, {0.3, 0.102}}"),
        frame("Clock", 6, "{{0.05, 0.899}, {0.2, 0.102}}"),
        frame("Message", 5, "{{0.05, 0.025}, {0.9, 0.101}}"),
    ])
}

fn layouts(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let layouts = array(&status, "stageLayouts")?
        .iter()
        .map(|layout| {
            json!({
                "brd": false,
                "uid": layout["id"],
                "zro": 0,
                "oCl": "1.000000 1.000000 1.000000",
                "fme": frames(),
                "ovr": false,
                "acn": "sl",
                "nme": layout["title"],
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({"acn": "asl", "ary": layouts}))
}

fn current_layout(backend: &dyn Backend) -> Result<Value, String> {
    let status = backend.full()?;
    let layout = array(&status, "outputs")?
        .iter()
        .filter(|output| output["type"] == "stage")
        .find_map(|output| output["stageLayoutId"].as_str())
        .ok_or("No stage layout is assigned to a stage screen.")?;
    Ok(json!({"acn": "psl", "uid": layout}))
}

/// Current and next slide text and notes, the clock and the visible stage message.
pub(super) fn slide_values(live: &Value) -> Value {
    let message = &live["stageMessage"];
    let visible = message["state"] == "visible";
    json!([
        {"acn": "cs", "uid": live["currentSlide"]["id"].as_str().unwrap_or_default(), "txt": text(&live["currentSlide"], "text")},
        {"acn": "ns", "uid": live["nextSlide"]["id"].as_str().unwrap_or_default(), "txt": text(&live["nextSlide"], "text")},
        {"acn": "csn", "txt": text(&live["currentSlide"], "notes")},
        {"acn": "nsn", "txt": text(&live["nextSlide"], "notes")},
        {"acn": "msg", "txt": if visible { text(message, "text") } else { "" }},
    ])
}

pub(super) fn clock_text() -> String {
    Local::now().format("%l:%M %p").to_string()
}

pub(super) fn frame_values(live: &Value) -> Value {
    let mut values = slide_values(live);
    values
        .as_array_mut()
        .expect("slide values array")
        .push(json!({"acn": "sys", "txt": clock_text()}));
    json!({"acn": "fv", "ary": values})
}

pub(super) fn handle(backend: &dyn Backend, message: &Value) -> Vec<Value> {
    let result = match message.get("acn").and_then(Value::as_str) {
        Some("asl") => layouts(backend),
        Some("psl") => current_layout(backend),
        Some("fv") => Ok(frame_values(&backend.live())),
        _ => Err("Unsupported action.".to_string()),
    };
    vec![result.unwrap_or_else(|reason| error(&reason))]
}
