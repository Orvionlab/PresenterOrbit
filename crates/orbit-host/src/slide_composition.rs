use serde_json::{json, Value};

pub fn merge_conflict(first: &Value, second: &Value) -> bool {
    fn visual(slide: &Value) -> Value {
        let mut value = slide.clone();
        crate::workspace::slide_text_edit::remove_paragraph_overrides(&mut value);
        let normalized_id = |id: &Value| {
            slide
                .get("layers")
                .and_then(Value::as_array)
                .and_then(|layers| layers.iter().position(|layer| layer.get("id") == Some(id)))
                .map_or_else(|| id.clone(), |index| json!({"local": index}))
        };
        let order = slide
            .get("themeLayerOrder")
            .and_then(Value::as_array)
            .map(|ids| ids.iter().map(normalized_id).collect::<Vec<_>>());
        let primary_id = slide.get("primaryTextLayerId").map(normalized_id);
        let primary = value
            .get("primaryTextLayerId")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(object) = value.as_object_mut() {
            for key in [
                "id",
                "content",
                "text",
                "name",
                "label",
                "groupId",
                "groupName",
                "sectionId",
                "notes",
                "notesRuns",
                "contentRuns",
                "primaryTextLayerId",
                "themeLayerOrder",
            ] {
                object.remove(key);
            }
            if let Some(order) = order {
                object.insert("themeLayerOrder".to_string(), json!(order));
            }
            if let Some(primary_id) = primary_id {
                object.insert("primaryTextLayerId".to_string(), primary_id);
            }
            if let Some(layers) = object.get_mut("layers").and_then(Value::as_array_mut) {
                for layer in layers {
                    if let Some(layer) = layer.as_object_mut() {
                        if layer.get("id").and_then(Value::as_str) == primary.as_deref() {
                            layer.insert("text".to_string(), json!(""));
                        }
                        layer.remove("id");
                    }
                }
            }
        }
        value
    }
    second
        .get("notes")
        .and_then(Value::as_str)
        .is_some_and(|notes| !notes.trim().is_empty())
        || second
            .get("notesRuns")
            .and_then(Value::as_array)
            .is_some_and(|runs| {
                runs.iter().any(|run| {
                    run.get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.trim().is_empty())
                })
            })
        || [first, second].iter().any(|slide| {
            slide
                .get("actions")
                .and_then(Value::as_array)
                .is_some_and(|actions| !actions.is_empty())
        })
        || visual(first) != visual(second)
}

pub fn compose_slide(slide: &Value, theme: Option<Value>) -> Value {
    let theme = theme.or_else(|| slide.get("presentationThemeSnapshot").cloned());
    let mut result = json!({});
    let base = result
        .as_object_mut()
        .expect("Slide composition source is an object.");
    if let Some(theme) = theme.as_ref().and_then(Value::as_object) {
        for key in [
            "background",
            "backgroundEnabled",
            "textStyle",
            "width",
            "height",
            "buildStep",
        ] {
            if let Some(value) = theme.get(key) {
                base.insert(key.to_string(), value.clone());
            }
        }
    }
    if let Some(source) = slide.as_object() {
        for (key, value) in source {
            base.insert(key.clone(), value.clone());
        }
    }
    let mut layers = theme
        .as_ref()
        .and_then(|value| value.get("layers"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let overrides = slide.get("themeLayerOverrides").and_then(Value::as_object);
    for layer in &mut layers {
        let id = layer
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Some(patch) = overrides
            .and_then(|patches| patches.get(&id))
            .and_then(Value::as_object)
        {
            if let Some(object) = layer.as_object_mut() {
                for (key, value) in patch {
                    if key == "textStyle" {
                        let mut style = object
                            .get(key)
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default();
                        if let Some(patch) = value.as_object() {
                            style.extend(patch.clone());
                        }
                        object.insert(key.clone(), Value::Object(style));
                    } else {
                        object.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }
    if let Some(local) = slide.get("layers").and_then(Value::as_array) {
        for layer in local {
            if !layers.iter().any(|base| base.get("id") == layer.get("id")) {
                layers.push(layer.clone());
            }
        }
    }
    if !layers.iter().any(|layer| {
        layer["type"]
            .as_str()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("text"))
            || matches!(layer["role"].as_str(), Some("primary" | "slide-text"))
    }) {
        let width = base.get("width").and_then(Value::as_f64).unwrap_or(1920.0);
        let height = base.get("height").and_then(Value::as_f64).unwrap_or(1080.0);
        layers.insert(0, json!({ "id": format!("{}-primary", slide["id"].as_str().unwrap_or_default()), "type": "Text", "role": "primary",
            "bounds": { "x": width * 0.05, "y": height * 0.1, "width": width * 0.9, "height": height * 0.8 } }));
    }
    let primary = slide
        .get("primaryTextLayerId")
        .and_then(Value::as_str)
        .filter(|id| layers.iter().any(|layer| layer["id"].as_str() == Some(id)))
        .or_else(|| {
            layers
                .iter()
                .find(|layer| matches!(layer["role"].as_str(), Some("primary" | "slide-text")))
                .and_then(|layer| layer["id"].as_str())
        })
        .or_else(|| {
            layers
                .iter()
                .find(|layer| {
                    layer["type"]
                        .as_str()
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("text"))
                })
                .and_then(|layer| layer["id"].as_str())
        })
        .map(str::to_string);
    if let Some(primary) = &primary {
        base.insert("primaryTextLayerId".into(), json!(primary));
    }
    for layer in &mut layers {
        let is_primary = layer.get("id").and_then(Value::as_str) == primary.as_deref();
        if let Some(object) = layer.as_object_mut() {
            object.insert("usePresentationText".to_string(), json!(is_primary));
            if is_primary {
                object.insert(
                    "text".to_string(),
                    slide
                        .get("content")
                        .or_else(|| slide.get("text"))
                        .cloned()
                        .unwrap_or_else(|| json!("")),
                );
                if let Some(runs) = slide.get("contentRuns") {
                    object.insert("textRuns".to_string(), runs.clone());
                }
            }
        }
    }
    if let Some(order) = slide.get("themeLayerOrder").and_then(Value::as_array) {
        layers.sort_by_key(|layer| {
            order
                .iter()
                .position(|id| Some(id) == layer.get("id"))
                .unwrap_or(order.len())
        });
    }
    base.insert("layers".to_string(), Value::Array(layers));
    result
}
