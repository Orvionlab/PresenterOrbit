use serde_json::{json, Map, Value};

pub fn layout(document: &Map<String, Value>, assignment: &str) -> Option<Value> {
    let (theme_id, slide_id) = assignment.split_once("::")?;
    document
        .get("themes")?
        .as_array()?
        .iter()
        .find(|theme| theme["id"].as_str() == Some(theme_id))?
        .get("slides")?
        .as_array()?
        .iter()
        .find(|slide| slide["id"].as_str() == Some(slide_id))
        .cloned()
}

/// How many recently picked theme layouts the theme picker keeps.
pub const RECENT_LIMIT: usize = 8;

/// `picked` moved to the front of the recents, once, capped at [`RECENT_LIMIT`].
pub fn remember_recent(workspace: &mut Value, picked: &str) {
    let mut recents = vec![picked.to_string()];
    recents.extend(
        workspace
            .get("recentPresentationThemeIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|id| *id != picked)
            .map(str::to_string),
    );
    recents.truncate(RECENT_LIMIT);
    workspace["recentPresentationThemeIds"] = json!(recents);
}

pub fn assign(
    document: &Map<String, Value>,
    slide: &mut Value,
    assignment: &str,
) -> Result<(), String> {
    let snapshot = layout(document, assignment).ok_or_else(|| {
        format!(
            "Presentation theme '{assignment}' was not found. Choose an available theme layout."
        )
    })?;
    let theme_id = assignment.split_once("::").unwrap().0;
    let theme = document["themes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|theme| theme["id"].as_str() == Some(theme_id))
        .unwrap();
    let name = format!(
        "{} / {}",
        theme["title"].as_str().unwrap_or(theme_id),
        snapshot["name"]
            .as_str()
            .unwrap_or(snapshot["id"].as_str().unwrap_or_default())
    );
    let object = slide
        .as_object_mut()
        .ok_or("Presentation slide must be an object.")?;
    if object.get("presentationThemeId").and_then(Value::as_str) != Some(assignment) {
        for key in [
            "themeLayerOverrides",
            "themeLayerOrder",
            "primaryTextLayerId",
        ] {
            object.remove(key);
        }
    }
    object.insert("presentationThemeId".into(), json!(assignment));
    object.insert("presentationThemeSnapshot".into(), snapshot);
    object.insert("presentationThemeName".into(), json!(name));
    Ok(())
}

pub fn insertion_assignment(
    document: &Map<String, Value>,
    slides: &[Value],
    index: usize,
) -> Result<String, String> {
    let assignment = slides
        .get(index.saturating_sub(1))
        .or_else(|| slides.first())
        .and_then(|slide| slide.get("presentationThemeId"))
        .or_else(|| {
            if slides.is_empty() {
                document.get("lastUsedPresentationThemeId")
            } else {
                None
            }
        })
        .and_then(Value::as_str)
        .ok_or("Choose a theme layout before adding a slide.")?;
    layout(document, assignment).ok_or("The preceding slide's theme is missing. Choose an available theme layout before adding a slide.")?;
    Ok(assignment.to_string())
}

pub fn refresh(previous: &Value, next: &mut Value) -> Result<(), String> {
    let workspace = next.as_object().ok_or("Workspace must be an object.")?;
    let themes_changed = previous.get("themes") != workspace.get("themes");
    let mut document = Map::new();
    if let Some(themes) = workspace.get("themes") {
        document.insert("themes".into(), themes.clone());
    }
    let mut last = None;
    if let Some(items) = next.get_mut("library").and_then(Value::as_array_mut) {
        for item in items {
            let old_item = previous
                .get("library")
                .and_then(Value::as_array)
                .and_then(|items| items.iter().find(|old| old["id"] == item["id"]));
            if !themes_changed && old_item == Some(item) {
                continue;
            }
            if let Some(slides) = item.get_mut("slides").and_then(Value::as_array_mut) {
                for slide in slides {
                    let old = old_item
                        .and_then(|item| item.get("slides"))
                        .and_then(Value::as_array)
                        .and_then(|slides| slides.iter().find(|old| old["id"] == slide["id"]));
                    let changed = old.is_none_or(|old| {
                        old.get("presentationThemeId") != slide.get("presentationThemeId")
                    });
                    let assignment = slide
                        .get("presentationThemeId")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if changed
                        && assignment
                            .as_deref()
                            .and_then(|id| layout(&document, id))
                            .is_none()
                    {
                        return Err("Every new slide or theme assignment requires an available theme layout.".into());
                    }
                    if let Some(id) = assignment {
                        if layout(&document, &id).is_some() {
                            // Refresh only the base layout; slide overrides remain independent.
                            assign(&document, slide, &id)?;
                            if changed
                                && !crate::authoring::is_imported_layout_assignment(
                                    &document["themes"],
                                    &id,
                                )
                            {
                                last = Some(id);
                            }
                        } else if let Some(old) = old {
                            for key in ["presentationThemeSnapshot", "presentationThemeName"] {
                                if let Some(value) = old.get(key) {
                                    slide[key] = value.clone();
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(id) = last {
        next["lastUsedPresentationThemeId"] = json!(id);
        remember_recent(next, &id);
    } else if themes_changed
        && next["lastUsedPresentationThemeId"]
            .as_str()
            .is_none_or(|id| {
                layout(&document, id).is_none()
                    || crate::authoring::is_imported_layout_assignment(&document["themes"], id)
            })
    {
        let first = crate::authoring::reusable_themes(&document["themes"]).find_map(|theme| {
            Some(format!(
                "{}::{}",
                theme["id"].as_str()?,
                theme["slides"][0]["id"].as_str()?
            ))
        });
        next["lastUsedPresentationThemeId"] = json!(first);
    }
    Ok(())
}

pub fn basic_countdown_timers() -> Vec<Value> {
    vec![json!({
        "id": "7bdebe74-0aa6-4665-8e31-98831ae5adc8",
        "name": "Segment Countdown",
        "durationSeconds": 300,
        "allowsOverrun": true,
    })]
}

fn imported_layout(slide: &Value) -> Value {
    let mut layout = Map::new();
    for key in [
        "id",
        "width",
        "height",
        "background",
        "backgroundEnabled",
        "textStyle",
        "layers",
        "themeLayerOrder",
    ] {
        if let Some(value) = slide.get(key) {
            layout.insert(key.into(), value.clone());
        }
    }
    layout.insert(
        "name".into(),
        json!(slide["name"]
            .as_str()
            .unwrap_or(slide["id"].as_str().unwrap_or("Slide"))),
    );
    Value::Object(layout)
}

fn imported_visual(slide: &Value) -> Value {
    let mut visual = imported_layout(slide);
    let object = visual.as_object_mut().unwrap();
    object.remove("id");
    object.remove("name");
    object.remove("themeLayerOrder");
    if let Some(layers) = object.get_mut("layers").and_then(Value::as_array_mut) {
        for layer in layers {
            if let Some(layer) = layer.as_object_mut() {
                layer.remove("id");
                layer.remove("name");
                if layer.get("type").and_then(Value::as_str) == Some("Text") {
                    layer.remove("text");
                }
            }
        }
    }
    visual
}

/// A native theme layout with its visual precomputed, so each slide compares against it
/// without rebuilding it.
struct NativeLayout {
    reference: Option<String>,
    source_slide_id: Option<Value>,
    visual: Value,
}

fn native_layouts(themes: &[Value]) -> Vec<NativeLayout> {
    themes
        .iter()
        .filter(|theme| theme["sourceType"] == "propresenter")
        .flat_map(|theme| {
            theme["slides"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |layout| NativeLayout {
                    reference: theme["id"]
                        .as_str()
                        .zip(layout["id"].as_str())
                        .map(|(theme, layout)| format!("{theme}::{layout}")),
                    source_slide_id: layout.get("sourceSlideId").cloned(),
                    visual: imported_visual(layout),
                })
        })
        .collect()
}

fn matching_imported_layout(slide: &Value, native: &[NativeLayout]) -> Option<String> {
    // Presentation files contain the applied graphics, not a theme reference.
    // Only restore a link when the decoded layout identifies one candidate.
    if !slide
        .get("layers")
        .and_then(Value::as_array)
        .is_some_and(|layers| !layers.is_empty())
    {
        return None;
    }
    let visual = imported_visual(slide);
    let matches = native
        .iter()
        .filter(|candidate| candidate.visual == visual)
        .collect::<Vec<_>>();
    let candidate = if matches.len() == 1 {
        matches.first().copied()
    } else {
        let identified = matches
            .into_iter()
            .filter(|candidate| {
                slide
                    .get("sourceSlideId")
                    .is_some_and(|id| candidate.source_slide_id.as_ref() == Some(id))
            })
            .collect::<Vec<_>>();
        (identified.len() == 1).then(|| identified[0])
    }?;
    candidate.reference.clone()
}

pub fn import_layouts(items: &mut [Value], themes: &mut Vec<Value>) -> Result<(), String> {
    // Themes added below are never native, so this set stays valid for every item.
    let native = native_layouts(themes);
    let mut imported = std::collections::HashMap::new();
    for item in items {
        let id = format!("imported-{}", uuid::Uuid::new_v4());
        let title = format!(
            "Imported {}",
            item["title"].as_str().unwrap_or("Presentation")
        );
        let Some(slides) = item.get_mut("slides").and_then(Value::as_array_mut) else {
            continue;
        };
        let mut assignments = Vec::with_capacity(slides.len());
        let mut layouts = Vec::new();
        for slide in slides.iter() {
            let slide_id = slide["id"]
                .as_str()
                .ok_or("Imported slide id is required.")?;
            let assignment = if let Some(assignment) = matching_imported_layout(slide, &native) {
                assignment
            } else {
                let visual = serde_json::to_string(&imported_visual(slide))
                    .map_err(|error| error.to_string())?;
                imported
                    .entry(visual)
                    .or_insert_with(|| {
                        layouts.push(imported_layout(slide));
                        format!("{id}::{slide_id}")
                    })
                    .clone()
            };
            assignments.push(assignment);
        }
        if !layouts.is_empty() {
            themes.push(json!({ "id": id, "title": title, "editable": true, "schemaVersion": 2, "sourceType": "presenter-orbit", "presentationOnly": true, "slides": layouts }));
        }
        // Moved, not cloned: cloning every theme per presentation is quadratic.
        let mut document = Map::new();
        document.insert("themes".into(), Value::Array(std::mem::take(themes)));
        for (slide, assignment) in slides.iter_mut().zip(assignments) {
            let source_layers = slide
                .get("layers")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let primary_source = slide["primaryTextLayerId"].as_str().map(str::to_string);
            let source_order = slide
                .get("themeLayerOrder")
                .and_then(Value::as_array)
                .cloned();
            assign(&document, slide, &assignment)?;
            let target_layers = slide["presentationThemeSnapshot"]["layers"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if let Some(order) = source_order {
                let order = order
                    .iter()
                    .map(|id| {
                        source_layers
                            .iter()
                            .position(|layer| layer.get("id") == Some(id))
                            .and_then(|index| target_layers.get(index))
                            .and_then(|layer| layer.get("id"))
                            .cloned()
                            .ok_or("Imported layer order refers to an unavailable object.")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                slide["themeLayerOrder"] = json!(order);
            }
            let mut overrides = Map::new();
            for (source, target) in source_layers.iter().zip(&target_layers) {
                if source["type"] == "Text" {
                    let target_id = target["id"]
                        .as_str()
                        .ok_or("Imported theme layer id is required.")?;
                    let primary = primary_source.as_deref().map_or_else(
                        || slide.get("primaryTextLayerId").is_none(),
                        |id| source["id"].as_str() == Some(id),
                    );
                    if primary {
                        slide["primaryTextLayerId"] = json!(target_id);
                    } else {
                        overrides.insert(target_id.into(), json!({"text": source["text"]}));
                    }
                }
            }
            if !overrides.is_empty() {
                slide["themeLayerOverrides"] = Value::Object(overrides);
            }
            // Imported visual objects now belong to their retained theme layout.
            for key in [
                "layers",
                "background",
                "backgroundEnabled",
                "textStyle",
                "width",
                "height",
            ] {
                slide.as_object_mut().unwrap().remove(key);
            }
        }
        if let Some(Value::Array(restored)) = document.remove("themes") {
            *themes = restored;
        }
    }
    Ok(())
}

pub fn compose_item(document: &Map<String, Value>, item: &Value) -> Value {
    let mut item = item.clone();
    if let Some(slides) = item.get_mut("slides").and_then(Value::as_array_mut) {
        for slide in slides {
            let theme = slide
                .get("presentationThemeId")
                .and_then(Value::as_str)
                .and_then(|id| layout(document, id));
            *slide = crate::slide_composition::compose_slide(slide, theme);
        }
    }
    item
}
