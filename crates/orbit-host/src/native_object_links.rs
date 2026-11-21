use super::{merged_text_style, native_linked_text};
use crate::scene::SceneSlide;
use serde_json::{Map, Value};
use std::collections::HashMap;

fn target<'a>(slide: &'a SceneSlide, id: &str) -> Result<&'a Map<String, Value>, String> {
    let layers = slide.source.get("layers").and_then(Value::as_array);
    let matches: Vec<_> = layers
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|layer| layer.get("id").and_then(Value::as_str) == Some(id))
        .collect();
    match matches.as_slice() {
        [layer] => Ok(layer),
        [] => Err(format!("Object link target '{id}' is missing.")),
        _ => Err(format!("Object link target '{id}' is ambiguous.")),
    }
}

pub(super) fn validate(slide: &SceneSlide) -> Result<(), String> {
    fn visit(
        slide: &SceneSlide,
        layer: &Map<String, Value>,
        path: &mut Vec<String>,
        checked: &mut HashMap<usize, usize>,
    ) -> Result<usize, String> {
        let id = layer.get("id").and_then(Value::as_str).unwrap_or("");
        if path.len() >= 256 {
            return Err("Object link dependency depth exceeds 256.".into());
        }
        if path.iter().any(|entry| entry == id) {
            path.push(id.to_string());
            return Err(format!("Cyclic object links: {}.", path.join(" -> ")));
        }
        let identity = layer as *const Map<String, Value> as usize;
        if let Some(depth) = checked.get(&identity) {
            if path.len() + depth > 256 {
                return Err("Object link dependency depth exceeds 256.".into());
            }
            return Ok(*depth);
        }
        path.push(id.to_string());
        if layer.get("fillType").and_then(Value::as_str) == Some("capture-status-color") {
            capture_colors(layer)?;
        }
        let style = merged_text_style(&slide.source, layer);
        let mut links = Vec::new();
        if matches!(
            layer.get("fillType").and_then(Value::as_str),
            Some("group-color" | "label-color")
        ) && !(matches!(
            layer.get("dataFillSource").and_then(Value::as_str),
            Some("Current Slide" | "Next Slide")
        ) || (layer.get("fillType").and_then(Value::as_str) == Some("group-color")
            && layer.get("dataFillSource").and_then(Value::as_str) == Some("Next in Arrangement")))
        {
            return Err(format!(
                "Object '{id}' requires a valid data fill slide source."
            ));
        }
        if layer.get("fillType").and_then(Value::as_str) == Some("slide-object") {
            links.push(layer.get("fillObjectId").and_then(Value::as_str));
        }
        if style.get("linkedTextEnabled").and_then(Value::as_bool) != Some(false)
            && style.get("linkedTextSource").and_then(Value::as_str) == Some("Object")
        {
            if style.get("lineTransform").and_then(Value::as_str)
                == Some("Unsupported object text transform")
            {
                return Err("Unsupported object text transform.".into());
            }
            links.push(style.get("linkedTextObjectId").and_then(Value::as_str));
        }
        let mut depth = 1;
        for link in links {
            let link = link
                .filter(|id| !id.is_empty())
                .ok_or_else(|| format!("Object '{id}' requires a link target."))?;
            depth = depth.max(1 + visit(slide, target(slide, link)?, path, checked)?);
        }
        path.pop();
        checked.insert(identity, depth);
        Ok(depth)
    }
    if let Some(layers) = slide.source.get("layers").and_then(Value::as_array) {
        let mut checked = HashMap::new();
        for layer in layers.iter().filter_map(Value::as_object) {
            visit(slide, layer, &mut Vec::new(), &mut checked)?;
        }
    }
    Ok(())
}

pub(super) fn resolve(
    slide: &SceneSlide,
    source: &Map<String, Value>,
    context: &Value,
    fallback: &str,
    presentation_text: bool,
) -> Result<Map<String, Value>, String> {
    let mut result = source.clone();
    if source.get("fillType").and_then(Value::as_str) == Some("slide-object") {
        let paint = resolve(
            slide,
            fill_target(slide, source)?,
            context,
            fallback,
            presentation_text,
        )?;
        result.retain(|key, _| !is_fill_field(key));
        result.extend(paint.into_iter().filter(|(key, _)| is_fill_field(key)));
    }
    let style = merged_text_style(&slide.source, source);
    if style.get("linkedTextEnabled").and_then(Value::as_bool) != Some(false)
        && style.get("linkedTextSource").and_then(Value::as_str) == Some("Object")
    {
        let id = style
            .get("linkedTextObjectId")
            .and_then(Value::as_str)
            .ok_or("Object text link requires a target.")?;
        let linked = resolve(
            slide,
            target(slide, id)?,
            context,
            fallback,
            presentation_text,
        )?;
        let linked_style = merged_text_style(&slide.source, &linked);
        let source_text = linked.get("text").and_then(Value::as_str).unwrap_or("");
        let base = if linked
            .get("usePresentationText")
            .and_then(Value::as_bool)
            .unwrap_or(presentation_text)
        {
            fallback
        } else {
            source_text
        };
        let text = native_linked_text::resolve_linked_text(base, &linked_style, context);
        let text = if linked_style
            .get("linkedTextEnabled")
            .and_then(Value::as_bool)
            != Some(false)
        {
            native_linked_text::apply_line_transform(
                &text,
                linked_style
                    .get("linkedTextLineTransform")
                    .and_then(Value::as_str)
                    .unwrap_or("None"),
                None,
            )
        } else {
            text
        };
        let text = native_linked_text::apply_capitalization(
            &text,
            linked_style
                .get("capitalization")
                .and_then(Value::as_str)
                .unwrap_or("None"),
        );
        let text = native_linked_text::apply_line_transform(
            &text,
            linked_style
                .get("lineTransform")
                .and_then(Value::as_str)
                .unwrap_or("None"),
            linked_style
                .get("lineTransformDelimiter")
                .and_then(Value::as_str),
        );
        let text = native_linked_text::apply_line_transform(
            &text,
            style
                .get("linkedTextLineTransform")
                .and_then(Value::as_str)
                .unwrap_or("None"),
            None,
        );
        result.insert("text".into(), Value::String(text));
        result.insert("usePresentationText".into(), Value::Bool(false));
        let mut resolved_style = style;
        resolved_style.insert("linkedTextEnabled".into(), Value::Bool(false));
        result.insert("textStyle".into(), Value::Object(resolved_style));
    }
    if matches!(
        source.get("fillType").and_then(Value::as_str),
        Some("group-color" | "label-color")
    ) {
        let key = if source.get("fillType").and_then(Value::as_str) == Some("group-color") {
            "groupColor"
        } else {
            "labelColor"
        };
        let selector = source
            .get("dataFillSource")
            .and_then(Value::as_str)
            .ok_or("Data fill requires a slide source.")?;
        let slide_key = match selector {
            "Current Slide" => "slide",
            "Next Slide" => "nextSlide",
            "Next in Arrangement" if key == "groupColor" => "nextArrangementSlide",
            _ => return Err("Invalid data fill slide source.".into()),
        };
        let color = context
            .get(slide_key)
            .and_then(|slide| slide.get(key))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{selector} has no {key}."))?;
        if super::parse_color(color).is_none() {
            return Err(format!("{selector} has an invalid {key}."));
        }
        result.insert("fillType".into(), Value::String("color".into()));
        result.insert("fill".into(), Value::String(color.into()));
        result.insert("fillColors".into(), serde_json::json!([color]));
    }
    if source.get("fillType").and_then(Value::as_str) == Some("video-input") {
        // The layer shows its chosen video input; with none chosen, or one without a source, it stays empty.
        let input = source
            .get("videoInputId")
            .and_then(Value::as_str)
            .and_then(|id| context.get("videoInputSources")?.get(id)?.as_object());
        match input {
            Some(input) => {
                result.insert("fillType".into(), Value::String("ndi".into()));
                for (key, value) in input {
                    result.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
            None => {
                result.insert("fillType".into(), Value::String("none".into()));
            }
        }
    }
    if source.get("fillType").and_then(Value::as_str) == Some("capture-status-color") {
        let status = context
            .get("captureStatus")
            .ok_or("Native capture status is unavailable.")?;
        let state = if status.get("error").is_some_and(Value::is_string) {
            "error"
        } else if status.get("finalizing").and_then(Value::as_bool) == Some(true) {
            "finalizing"
        } else if status.get("active").and_then(Value::as_bool) == Some(true) {
            "recording"
        } else if status.get("active").and_then(Value::as_bool) == Some(false) {
            "idle"
        } else {
            return Err("Native capture status is invalid.".into());
        };
        let colors = capture_colors(source)?;
        let color = colors[state].as_str().expect("validated capture color");
        result.insert("fillType".into(), Value::String("color".into()));
        result.insert("fill".into(), Value::String(color.into()));
        result.insert("fillColors".into(), serde_json::json!([color]));
    }
    Ok(result)
}

fn is_fill_field(key: &str) -> bool {
    key.starts_with("fill")
        || key.starts_with("media")
        || matches!(
            key,
            "sourceUrl"
                | "sourcePath"
                | "webUrl"
                | "ndiSource"
                | "cameraSource"
                | "dataFillSource"
                | "captureStatusColors"
                | "previewScreenId"
        )
}

pub(super) fn fill_target<'a>(
    slide: &'a SceneSlide,
    source: &Map<String, Value>,
) -> Result<&'a Map<String, Value>, String> {
    target(
        slide,
        source
            .get("fillObjectId")
            .and_then(Value::as_str)
            .ok_or("Object fill requires a target.")?,
    )
}

fn capture_colors(source: &Map<String, Value>) -> Result<&Map<String, Value>, String> {
    let colors = source
        .get("captureStatusColors")
        .and_then(Value::as_object)
        .ok_or("Capture fill requires authored state colors.")?;
    for key in ["idle", "recording", "finalizing", "error"] {
        let color = colors
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Capture fill requires an authored {key} color."))?;
        if super::parse_color(color).is_none() {
            return Err(format!("Capture fill has an invalid {key} color."));
        }
    }
    Ok(colors)
}
