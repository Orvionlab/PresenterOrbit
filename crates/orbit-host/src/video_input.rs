use std::collections::HashSet;

use serde_json::{json, Value};

pub fn clear_snapshot(snapshot: &Value) -> Result<(Value, Value), String> {
    let mut next = snapshot.clone();
    next.as_object_mut()
        .ok_or("Output snapshot must be an object.")?
        .insert("liveVideoInputId".into(), Value::Null);
    Ok((next, json!({"liveVideoInputId":null})))
}

pub fn validate_collection(value: &Value) -> Result<(), String> {
    let inputs = value.as_array().ok_or("videoInputs must be an array.")?;
    let mut ids = HashSet::new();
    for input in inputs {
        let id = input
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or("Video input ID is required.")?;
        if !ids.insert(id) {
            return Err(format!("Duplicate video input ID: {id}"));
        }
        if input
            .get("title")
            .and_then(Value::as_str)
            .is_none_or(|title| title.trim().is_empty())
        {
            return Err("Video input title is required.".into());
        }
        let (ndi, camera) = (input.get("ndiSource"), input.get("cameraSource"));
        if ndi.is_some() && camera.is_some() {
            return Err(
                "A video input uses either an NDI source or a camera source, not both.".into(),
            );
        }
        if let Some(source) = ndi {
            let source: crate::ndi_input::NdiSource = serde_json::from_value(source.clone())
                .map_err(|error| format!("Invalid video input NDI source: {error}"))?;
            crate::ndi_input::validate_source(&source)?;
        }
        if let Some(source) = camera {
            let source: crate::camera_devices::CameraSource =
                serde_json::from_value(source.clone())
                    .map_err(|error| format!("Invalid video input camera source: {error}"))?;
            crate::camera_devices::validate_source(&source)?;
        }
        if ndi.is_some() || camera.is_some() {
            if !matches!(
                input.get("scale").and_then(Value::as_str),
                Some("fit" | "fill" | "stretch")
            ) {
                return Err("Configured video input scale must be fit, fill or stretch.".into());
            }
        }
    }
    Ok(())
}

pub fn scene(
    inputs: &[Value],
    active_id: Option<&str>,
    width: u32,
    height: u32,
) -> Result<Option<Value>, String> {
    let Some(id) = active_id else { return Ok(None) };
    validate_collection(&json!(inputs))?;
    let input = inputs
        .iter()
        .find(|input| input.get("id").and_then(Value::as_str) == Some(id))
        .ok_or_else(|| format!("Video input is unavailable: {id}"))?;
    let (source_key, source) = ["ndiSource", "cameraSource"]
        .into_iter()
        .find_map(|key| input.get(key).map(|source| (key, source)))
        .ok_or_else(|| format!("Video input has no configured source: {id}"))?;
    let scale = match input.get("scale").and_then(Value::as_str) {
        Some("fit") => "Scale to Fit",
        Some("fill") => "Scale to Fill",
        Some("stretch") => "Stretch to Fill",
        _ => return Err("Configured video input scale is invalid.".into()),
    };
    Ok(Some(json!({
        "id": format!("video-input-{id}"), "width": width, "height": height,
        "backgroundEnabled": false,
        "layers": [{"id": id, "type": "Shape", "shape": "rect", "fill": "transparent",
            "fillType": "ndi", source_key: source, "mediaScale": scale,
            "bounds": {"x":0,"y":0,"width":width,"height":height}}]
    })))
}
