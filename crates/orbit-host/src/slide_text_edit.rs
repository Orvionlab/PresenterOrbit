use serde_json::{json, Map, Value};

fn paragraphs(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n")
        .replace("\n\r", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(str::to_string)
        .collect()
}

fn remap(style: &mut Value, previous: &str, next: &str, keep_end: bool) {
    let Some(existing) = style.get("textParagraphStyles").and_then(Value::as_array) else {
        return;
    };
    if existing.is_empty() {
        return;
    }
    let old = paragraphs(previous);
    let new = paragraphs(next);
    let limit = old.len().min(new.len());
    let matching_end = |maximum: usize| {
        (0..maximum)
            .take_while(|offset| old[old.len() - offset - 1] == new[new.len() - offset - 1])
            .count()
    };
    let end = if keep_end { matching_end(limit) } else { 0 };
    let prefix = (0..limit - end)
        .take_while(|index| old[*index] == new[*index])
        .count();
    let suffix = if keep_end {
        end
    } else {
        matching_end(limit - prefix)
    };
    let old_middle = old.len() - prefix - suffix;
    let mapped: Vec<Value> = (0..new.len().min(2000))
        .map(|index| {
            let source = if index < prefix {
                index
            } else if index >= new.len() - suffix {
                old.len() - (new.len() - index)
            } else if old_middle > 0 {
                prefix + (index - prefix).min(old_middle - 1)
            } else {
                prefix.saturating_sub(1)
            };
            existing
                .get(source)
                .or_else(|| existing.get(prefix + suffix))
                .cloned()
                .unwrap_or_else(|| json!({}))
        })
        .collect();
    style["textParagraphStyles"] = json!(mapped);
}

fn style_locations(slide: &Map<String, Value>) -> Vec<Vec<String>> {
    let mut paths = vec![vec!["textStyle".into()]];
    if let Some(id) = slide.get("primaryTextLayerId").and_then(Value::as_str) {
        if let Some(index) = slide
            .get("layers")
            .and_then(Value::as_array)
            .and_then(|layers| {
                layers
                    .iter()
                    .position(|layer| layer["id"].as_str() == Some(id))
            })
        {
            paths.push(vec!["layers".into(), index.to_string(), "textStyle".into()]);
        }
        paths.push(vec![
            "themeLayerOverrides".into(),
            id.into(),
            "textStyle".into(),
        ]);
    }
    paths
}

fn pointer(path: &[String]) -> String {
    format!(
        "/{}",
        path.iter()
            .map(|key| key.replace('~', "~0").replace('/', "~1"))
            .collect::<Vec<_>>()
            .join("/")
    )
}

fn store_overrides(slide: &mut Value, path: &[String], overrides: Value) {
    let mut target = slide;
    for key in path {
        if target.is_array() {
            target = &mut target[key.parse::<usize>().unwrap()];
        } else {
            target = target
                .as_object_mut()
                .unwrap()
                .entry(key)
                .or_insert_with(|| json!({}));
        }
    }
    target["textParagraphStyles"] = overrides;
}

/// Materialize inherited paragraph overrides on this slide before changing its content.
pub fn prepare(slide: &mut Value, theme: Option<Value>) {
    let composed = crate::slide_composition::compose_slide(slide, theme);
    if slide
        .get("textStyle")
        .and_then(|style| style.get("textParagraphStyles"))
        .is_none()
    {
        if let Some(overrides) = composed
            .get("textStyle")
            .and_then(|style| style.get("textParagraphStyles"))
        {
            let object = slide.as_object_mut().unwrap();
            object.entry("textStyle").or_insert_with(|| json!({}))["textParagraphStyles"] =
                overrides.clone();
        }
    }
    let Some(id) = composed["primaryTextLayerId"].as_str() else {
        return;
    };
    let Some(style) = composed["layers"]
        .as_array()
        .and_then(|layers| layers.iter().find(|layer| layer["id"] == id))
        .and_then(|layer| layer.get("textStyle"))
    else {
        return;
    };
    let Some(overrides) = style.get("textParagraphStyles") else {
        return;
    };
    slide["primaryTextLayerId"] = json!(id);
    let object = slide.as_object_mut().unwrap();
    if let Some(local) = object
        .get_mut("layers")
        .and_then(Value::as_array_mut)
        .and_then(|layers| layers.iter_mut().find(|layer| layer["id"] == id))
    {
        local
            .as_object_mut()
            .unwrap()
            .entry("textStyle")
            .or_insert_with(|| json!({}))["textParagraphStyles"] = overrides.clone();
    } else {
        let patches = object
            .entry("themeLayerOverrides")
            .or_insert_with(|| json!({}));
        let patch = patches
            .as_object_mut()
            .unwrap()
            .entry(id)
            .or_insert_with(|| json!({}));
        patch
            .as_object_mut()
            .unwrap()
            .entry("textStyle")
            .or_insert_with(|| json!({}))["textParagraphStyles"] = overrides.clone();
    }
}

fn remap_runs(runs: &[Value], content: &str, keep_end: bool) -> Option<Value> {
    let mut old = Vec::new();
    let mut styles = Vec::new();
    for run in runs {
        for character in run["text"].as_str().unwrap_or_default().chars() {
            old.push(character);
            styles.push(run.get("style").cloned());
        }
    }
    let new: Vec<char> = content.chars().collect();
    let maximum = old.len().min(new.len());
    let matching_end = |limit: usize| {
        (0..limit)
            .take_while(|offset| old[old.len() - offset - 1] == new[new.len() - offset - 1])
            .count()
    };
    let end = if keep_end { matching_end(maximum) } else { 0 };
    let prefix = (0..maximum - end)
        .take_while(|index| old[*index] == new[*index])
        .count();
    let suffix = if keep_end {
        end
    } else {
        matching_end(maximum - prefix)
    };
    let inserted = prefix
        .checked_sub(1)
        .and_then(|index| styles[index].clone())
        .or_else(|| styles.get(old.len() - suffix).cloned().flatten());
    let mut result: Vec<Value> = Vec::new();
    for (index, character) in new.iter().enumerate() {
        let style = if index < prefix {
            styles[index].clone()
        } else if index >= new.len() - suffix {
            styles[old.len() - (new.len() - index)].clone()
        } else {
            inserted.clone()
        };
        if let Some(last) = result
            .last_mut()
            .filter(|run| run.get("style") == style.as_ref())
        {
            let text = format!("{}{character}", last["text"].as_str().unwrap());
            last["text"] = json!(text);
        } else {
            let mut run = json!({"text":character.to_string()});
            if let Some(style) = style {
                run["style"] = style;
            }
            result.push(run);
        }
    }
    result
        .iter()
        .any(|run| run.get("style").is_some())
        .then(|| json!(result))
}

pub fn set_content(slide: &mut Map<String, Value>, content: &str, keep_end: bool) {
    let previous = slide
        .get("content")
        .or_else(|| slide.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let paths = style_locations(slide);
    let mut value = Value::Object(std::mem::take(slide));
    for path in paths {
        if let Some(style) = value.pointer_mut(&pointer(&path)) {
            remap(style, &previous, content, keep_end);
        }
    }
    *slide = value.as_object_mut().unwrap().clone();
    if let Some(runs) = slide.get("contentRuns").and_then(Value::as_array) {
        if let Some(mapped) = remap_runs(runs, content, keep_end) {
            slide.insert("contentRuns".into(), mapped);
        } else {
            slide.remove("contentRuns");
        }
    }
    slide.insert("content".into(), json!(content));
}

pub fn merge(first: &Value, second: &Value, content: &str) -> Value {
    let first_text = first["content"].as_str().unwrap_or_default();
    let second_text = second["content"].as_str().unwrap_or_default();
    let mut result = first.clone();
    set_content(result.as_object_mut().unwrap(), content, false);
    let mut left = first.clone();
    let mut right = second.clone();
    set_content(left.as_object_mut().unwrap(), first_text.trim_end(), false);
    set_content(
        right.as_object_mut().unwrap(),
        second_text.trim_start(),
        true,
    );
    let paths = style_locations(first.as_object().unwrap());
    let right_paths = style_locations(second.as_object().unwrap());
    for (index, path) in paths.iter().enumerate() {
        let key = pointer(path);
        let right_key = right_paths
            .get(index)
            .map(|path| pointer(path))
            .unwrap_or_else(|| key.clone());
        let a = left.pointer(&key);
        let b = right.pointer(&right_key);
        if [a, b].iter().all(|style| {
            style
                .and_then(|style| style.get("textParagraphStyles"))
                .is_none()
        }) {
            continue;
        }
        let count_a = if first_text.trim_end().is_empty() {
            0
        } else {
            paragraphs(first_text.trim_end()).len()
        };
        let count = paragraphs(content).len().min(2000);
        let overrides: Vec<Value> = (0..count)
            .map(|paragraph| {
                let (style, index) = if paragraph < count_a {
                    (a, paragraph)
                } else {
                    (b, paragraph - count_a)
                };
                style
                    .and_then(|style| style.get("textParagraphStyles"))
                    .and_then(Value::as_array)
                    .and_then(|styles| styles.get(index))
                    .cloned()
                    .unwrap_or_else(|| json!({}))
            })
            .collect();
        store_overrides(&mut result, path, json!(overrides));
    }
    let joined = match (
        first_text.trim_end().is_empty(),
        second_text.trim_start().is_empty(),
    ) {
        (true, _) => second_text.trim_start().to_string(),
        (_, true) => first_text.trim_end().to_string(),
        _ => format!("{}\n{}", first_text.trim_end(), second_text.trim_start()),
    };
    let mut runs = Vec::new();
    for (index, slide) in [&left, &right].iter().enumerate() {
        if slide["content"].as_str().unwrap_or_default().is_empty() {
            continue;
        }
        if index == 1 && !runs.is_empty() {
            runs.push(json!({"text":"\n"}));
        }
        runs.extend(
            slide
                .get("contentRuns")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_else(|| vec![json!({"text":slide["content"]})]),
        );
    }
    if let Some(mapped) = remap_runs(&runs, content, false).filter(|_| joined == content) {
        result["contentRuns"] = mapped;
    }
    result
}

pub fn remove_paragraph_overrides(slide: &mut Value) {
    let paths = style_locations(slide.as_object().unwrap());
    for path in paths {
        if let Some(style) = slide
            .pointer_mut(&pointer(&path))
            .and_then(Value::as_object_mut)
        {
            style.remove("textParagraphStyles");
        }
        if slide
            .pointer(&pointer(&path))
            .and_then(Value::as_object)
            .is_some_and(Map::is_empty)
        {
            let (last, parent) = path.split_last().unwrap();
            if parent.is_empty() {
                slide.as_object_mut().unwrap().remove(last);
            } else if let Some(object) = slide
                .pointer_mut(&pointer(parent))
                .and_then(Value::as_object_mut)
            {
                object.remove(last);
            }
        }
    }
    if let Some(patches) = slide
        .get_mut("themeLayerOverrides")
        .and_then(Value::as_object_mut)
    {
        patches.retain(|_, value| !value.as_object().is_some_and(Map::is_empty));
        if patches.is_empty() {
            slide.as_object_mut().unwrap().remove("themeLayerOverrides");
        }
    }
}
