//! Shared slide-editor composition and text commits. The renderer equivalents live in slide-composition.ts.
use serde_json::{json, Map, Value};

pub use crate::workspace::{reordered_slides, split_presentation_text};

pub fn preview_slide_deletion(item: &mut Value, operation: &Value) -> Result<(), String> {
    crate::workspace::apply_presentation_slide_deletion(
        item,
        operation
            .as_object()
            .ok_or("Slide deletion must be an operation object.")?,
    )
}

/// Applies the media mutation to a draft using the same validation and slide assembly as saving.
pub fn preview_media_edit(
    workspace: &Value,
    item: &mut Value,
    operation: &Value,
) -> Result<(), String> {
    crate::workspace::apply_presentation_media_edit(
        workspace
            .as_object()
            .ok_or("Workspace must be an object.")?,
        item,
        operation
            .as_object()
            .ok_or("Presentation media operation must be an object.")?,
    )?;
    Ok(())
}

pub fn is_imported_layout_theme(theme: &Value) -> bool {
    theme["presentationOnly"] == true
        || (theme["sourceType"] == "presenter-orbit"
            && theme["id"]
                .as_str()
                .and_then(|id| id.strip_prefix("imported-"))
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()))
}

pub fn reusable_themes(themes: &Value) -> impl Iterator<Item = &Value> {
    themes
        .as_array()
        .into_iter()
        .flatten()
        .filter(|theme| !is_imported_layout_theme(theme))
}

pub fn is_imported_layout_assignment(themes: &Value, assignment: &str) -> bool {
    let Some((theme_id, layout_id)) = assignment.split_once("::") else {
        return false;
    };
    themes.as_array().into_iter().flatten().any(|theme| {
        theme["id"] == theme_id
            && is_imported_layout_theme(theme)
            && theme["slides"]
                .as_array()
                .is_some_and(|slides| slides.iter().any(|slide| slide["id"] == layout_id))
    })
}

/// Validate and apply the same presentation setting used by workspace persistence.
pub fn patch_presentation_setting(
    item: &mut Value,
    field: &str,
    value: &Value,
) -> Result<(), String> {
    crate::presentation_settings::patch(item, field, value)
}

/// The colors the editor gives a new capture status fill; ProPresenter's import uses them too.
pub fn default_capture_status_colors() -> Value {
    json!({"idle":"#000000","recording":"#00aa00","finalizing":"#ffaa00","error":"#ff0000"})
}

pub fn slide_theme(workspace: &Value, slide: &Value) -> Option<Value> {
    slide_theme_ref(workspace, slide).cloned()
}

pub fn slide_theme_ref<'a>(workspace: &'a Value, slide: &'a Value) -> Option<&'a Value> {
    slide["presentationThemeId"]
        .as_str()
        .and_then(|assignment| {
            let (theme, layout) = assignment.split_once("::")?;
            workspace["themes"]
                .as_array()?
                .iter()
                .find(|entry| entry["id"] == theme)?["slides"]
                .as_array()?
                .iter()
                .find(|entry| entry["id"] == layout)
        })
        .or_else(|| slide.get("presentationThemeSnapshot"))
}
pub fn compose(slide: &Value, theme: Option<Value>) -> Result<Value, String> {
    let mut result = crate::slide_composition::compose_slide(slide, theme);
    for (key, default) in [("width", 1920), ("height", 1080)] {
        if result[key].is_null() {
            result[key] = json!(default);
        }
    }
    let primary = result["primaryTextLayerId"].clone();
    for layer in result["layers"]
        .as_array_mut()
        .ok_or("The slide has no layer list.")?
    {
        layer
            .as_object_mut()
            .ok_or("Slide layers must be objects.")?
            .remove("usePresentationText");
        if layer["id"] == primary {
            layer["role"] = json!("primary");
        }
    }
    Ok(result)
}
pub fn with_content(slide: &Value, content: &str) -> Result<Value, String> {
    let mut next = with_raw_content(slide, content)?;
    let primary = next["primaryTextLayerId"].clone();
    if let Some(layers) = next["layers"].as_array_mut() {
        for layer in layers {
            if layer["id"] == primary {
                layer["text"] = json!(content);
            }
        }
    }
    Ok(next)
}

pub fn with_raw_content(slide: &Value, content: &str) -> Result<Value, String> {
    let mut next = slide.clone();
    crate::workspace::slide_text_edit::set_content(
        next.as_object_mut().ok_or("The slide must be an object.")?,
        content,
        false,
    );
    Ok(next)
}
pub fn store(source: &Value, edited: &Value, theme: Option<&Value>) -> Result<Value, String> {
    let inherited = theme
        .and_then(|theme| theme["layers"].as_array())
        .cloned()
        .unwrap_or_default();
    let layers = edited["layers"].as_array().cloned().unwrap_or_default();
    let mut overrides = Map::new();
    for base in &inherited {
        let id = base["id"]
            .as_str()
            .ok_or("Theme layers must have an identity.")?;
        let Some(layer) = layers.iter().find(|layer| layer["id"] == id) else {
            overrides.insert(id.into(), json!({"hidden":true}));
            continue;
        };
        let mut patch = Map::new();
        for (key, value) in layer.as_object().ok_or("Slide layers must be objects.")? {
            if key == "id"
                || (matches!(key.as_str(), "text" | "textRuns")
                    && layer["id"] == edited["primaryTextLayerId"])
            {
                continue;
            }
            if key == "textStyle" {
                if let Some(style) = value.as_object() {
                    let style: Map<String, Value> = style
                        .iter()
                        .filter(|(name, value)| base["textStyle"].get(*name) != Some(*value))
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect();
                    if !style.is_empty() {
                        patch.insert(key.clone(), Value::Object(style));
                    }
                }
            } else if base.get(key) != Some(value) {
                patch.insert(key.clone(), value.clone());
            }
        }
        if !patch.is_empty() {
            overrides.insert(id.into(), Value::Object(patch));
        }
    }
    let mut result = source.clone();
    result
        .as_object_mut()
        .ok_or("The stored slide must be an object.")?
        .extend(
            edited
                .as_object()
                .ok_or("The edited slide must be an object.")?
                .clone(),
        );
    result["themeLayerOverrides"] = Value::Object(overrides);
    result["themeLayerOrder"] = json!(layers
        .iter()
        .map(|layer| layer["id"].clone())
        .collect::<Vec<_>>());
    let mut own = Vec::new();
    for layer in &layers {
        if inherited.iter().any(|base| base["id"] == layer["id"]) {
            continue;
        }
        let mut layer = layer.clone();
        layer
            .as_object_mut()
            .ok_or("Slide layers must be objects.")?
            .remove("textRuns");
        own.push(layer);
    }
    result["layers"] = json!(own);
    let content = layers
        .iter()
        .find(|layer| layer["id"] == edited["primaryTextLayerId"])
        .and_then(|layer| layer["text"].as_str())
        .or_else(|| edited["content"].as_str())
        .unwrap_or("");
    result = with_content(&result, content)?;
    if let Some(theme) = theme {
        for key in [
            "background",
            "backgroundEnabled",
            "width",
            "height",
            "textStyle",
        ] {
            if source.get(key).is_none() && edited.get(key) == theme.get(key) {
                if let Some(result) = result.as_object_mut() {
                    result.remove(key);
                }
            }
        }
    }
    Ok(result)
}
const MAX_PARAGRAPHS: usize = 2000;

fn paragraphs(text: &str) -> Vec<&str> {
    // `\r\n`, `\n\r`, `\r` and `\n` all end a paragraph, as in the renderer's splitParagraphs.
    let mut result = Vec::new();
    let (mut start, bytes) = (0, text.as_bytes());
    let mut index = 0;
    while index < bytes.len() {
        if matches!(bytes[index], b'\r' | b'\n') {
            result.push(&text[start..index]);
            let pair = bytes
                .get(index + 1)
                .is_some_and(|next| matches!(next, b'\r' | b'\n') && *next != bytes[index]);
            index += if pair { 2 } else { 1 };
            start = index;
        } else {
            index += 1;
        }
    }
    result.push(&text[start..]);
    result
}

/// Carries the per-paragraph overrides of a text style over to edited text: unchanged paragraphs at both ends keep
/// their override and a changed middle takes the overrides of the paragraph it replaces. Port of the renderer's
/// `preservePresentationTextParagraphOverrides`.
pub fn preserve_paragraph_overrides(
    style: &Value,
    previous: &str,
    next: &str,
) -> Result<Value, String> {
    let Some(existing) = style["textParagraphStyles"]
        .as_array()
        .filter(|existing| !existing.is_empty())
    else {
        return Ok(style.clone());
    };
    if existing.len() > MAX_PARAGRAPHS {
        return Err("Too many paragraph style overrides.".into());
    }
    let previous = paragraphs(previous);
    let next: Vec<&str> = paragraphs(next).into_iter().take(MAX_PARAGRAPHS).collect();
    let at = |index: usize| existing.get(index).cloned().unwrap_or_else(|| json!({}));
    let limit = previous.len().min(next.len());
    let mut prefix = 0;
    while prefix < limit && previous[prefix] == next[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < previous.len() - prefix
        && suffix < next.len() - prefix
        && previous[previous.len() - suffix - 1] == next[next.len() - suffix - 1]
    {
        suffix += 1;
    }
    let mut mapped = vec![json!({}); next.len()];
    for (index, slot) in mapped.iter_mut().enumerate().take(prefix) {
        *slot = at(index);
    }
    let old_middle = previous.len() - prefix - suffix;
    let new_middle = next.len() - prefix - suffix;
    if new_middle > 0 && old_middle > 0 {
        for offset in 0..new_middle {
            mapped[prefix + offset] = at(prefix + offset.min(old_middle - 1));
        }
    } else if new_middle > 0 {
        let inherited = existing
            .get(prefix.saturating_sub(1))
            .or_else(|| existing.get(prefix + suffix))
            .cloned()
            .unwrap_or_else(|| json!({}));
        for offset in 0..new_middle {
            mapped[prefix + offset] = inherited.clone();
        }
    }
    for offset in 0..suffix {
        mapped[next.len() - suffix + offset] = at(previous.len() - suffix + offset);
    }
    let mut result = style.clone();
    result["textParagraphStyles"] = Value::Array(mapped);
    Ok(result)
}

pub fn text_commit(slide: &Value, runs: Value, notes: bool) -> Result<Value, String> {
    let entries = runs.as_array().ok_or("Text runs must be an array.")?;
    let mut content = String::new();
    for run in entries {
        content.push_str(run["text"].as_str().ok_or("Text runs require text.")?);
    }
    let mut next = if notes {
        slide.clone()
    } else {
        with_content(slide, &content)?
    };
    if notes {
        if next["notesTextStyle"].is_object() {
            next["notesTextStyle"] = preserve_paragraph_overrides(
                &next["notesTextStyle"],
                slide["notes"].as_str().unwrap_or(""),
                &content,
            )?;
        }
        next["notes"] = json!(content);
        next["notesRuns"] = runs;
    } else {
        let styled = entries.iter().any(|run| run.get("style").is_some());
        if styled {
            next["contentRuns"] = runs.clone();
        } else {
            next.as_object_mut()
                .ok_or("The slide must be an object.")?
                .remove("contentRuns");
        }
        let primary = next["primaryTextLayerId"].clone();
        if let Some(layer) = next["layers"]
            .as_array_mut()
            .and_then(|layers| layers.iter_mut().find(|layer| layer["id"] == primary))
        {
            if styled {
                layer["textRuns"] = runs;
            } else {
                layer
                    .as_object_mut()
                    .ok_or("Slide layers must be objects.")?
                    .remove("textRuns");
            }
        }
    }
    Ok(next)
}
