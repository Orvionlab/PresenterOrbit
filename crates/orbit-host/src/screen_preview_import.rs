use std::collections::HashMap;

use serde_json::Value;

pub(super) fn resolve_sources(workspace: &mut Value) -> Result<(), String> {
    let mut sources: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(screens) = workspace.get("screenConfig").and_then(Value::as_object) {
        for (id, screen) in screens {
            if let Some(source) = screen.get("sourceScreenId").and_then(Value::as_str) {
                sources
                    .entry(source.to_owned())
                    .or_default()
                    .push(id.clone());
            }
        }
    }
    resolve_value(workspace, &sources)
}

fn resolve_value(value: &mut Value, sources: &HashMap<String, Vec<String>>) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            if object.get("fillType").and_then(Value::as_str) == Some("screen-preview") {
                if let Some(source) = object.get("previewScreenId").and_then(Value::as_str) {
                    if let Some(ids) = sources.get(source) {
                        if ids.len() != 1 {
                            return Err(format!("Screen preview source '{source}' matches multiple imported outputs."));
                        }
                        object.insert("previewScreenId".into(), Value::String(ids[0].clone()));
                    }
                }
            }
            for child in object.values_mut() {
                resolve_value(child, sources)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                resolve_value(child, sources)?;
            }
        }
        _ => {}
    }
    Ok(())
}
