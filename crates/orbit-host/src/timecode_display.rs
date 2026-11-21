use serde_json::{json, Value};

pub(crate) const NO_SIGNAL: &str = "--:--:--:--";

pub(crate) fn uses_timecode(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            (object.get("linkedTextSource").and_then(Value::as_str) == Some("Timecode")
                && object.get("linkedTextEnabled").and_then(Value::as_bool) != Some(false))
                || object.values().any(uses_timecode)
        }
        Value::Array(values) => values.iter().any(uses_timecode),
        _ => false,
    }
}

pub(crate) fn text(status: &Value) -> String {
    if status["locked"] == true
        && matches!(status["source"].as_str(), Some("ltc" | "mtc"))
        && status["error"].is_null()
    {
        if let Some(label) = status["label"].as_str().filter(|label| !label.is_empty()) {
            return label.to_owned();
        }
    }
    NO_SIGNAL.into()
}

pub(crate) fn attach_status(scene: &mut Value, status: &Value) {
    let Some(object) = scene.as_object_mut() else {
        return;
    };
    let context = object
        .entry("linkedTextContext")
        .or_insert_with(|| json!({}));
    let Some(context) = context.as_object_mut() else {
        return;
    };
    context.insert("timecodeStatus".into(), status.clone());
    if let Some(resources) = context
        .get_mut("stageSceneResources")
        .and_then(Value::as_object_mut)
    {
        for (key, resource) in resources {
            if key == "outputs" {
                if let Some(outputs) = resource.as_object_mut() {
                    for child in outputs.values_mut() {
                        if let Some(scene) = child.get_mut("scene") {
                            attach_status(scene, status);
                        }
                    }
                }
            } else if let Some(scene) = resource.get_mut("scene") {
                attach_status(scene, status);
            }
        }
    }
}
