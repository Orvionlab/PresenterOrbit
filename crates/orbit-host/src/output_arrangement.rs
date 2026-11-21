use std::collections::HashMap;

use serde_json::{json, Value};

pub(crate) fn resolve_live_arrangement_slides(
    item: &Value,
    live_instance_id: Option<&str>,
    live_playlist_entry_id: Option<&str>,
) -> Option<Vec<Value>> {
    let object = item.as_object()?;
    let master_slides = object.get("slides")?.as_array()?.clone();
    if live_instance_id
        .zip(live_playlist_entry_id)
        .is_some_and(|(instance_id, entry_id)| {
            instance_id
                .strip_prefix(entry_id)
                .is_some_and(|suffix| suffix.starts_with(":master:"))
        })
    {
        return Some(scope_master_slides(&master_slides, live_playlist_entry_id));
    }
    let Some(arrangements) = object.get("arrangements").and_then(Value::as_array) else {
        return Some(scope_master_slides(&master_slides, live_playlist_entry_id));
    };
    if let Some(instance_id) = live_instance_id {
        for arrangement in arrangements {
            let Some(slides) =
                resolve_arrangement_slides(item, arrangement, live_playlist_entry_id)
            else {
                continue;
            };
            if slides.iter().any(|slide| {
                slide
                    .get("arrangementInstanceId")
                    .and_then(Value::as_str)
                    .is_some_and(|candidate| arrangement_instance_matches(instance_id, candidate))
            }) {
                return Some(slides);
            }
        }
    }
    let Some(selected_id) = object.get("selectedArrangementId").and_then(Value::as_str) else {
        return Some(scope_master_slides(&master_slides, live_playlist_entry_id));
    };
    Some(
        arrangements
            .iter()
            .find(|arrangement| arrangement.get("id").and_then(Value::as_str) == Some(selected_id))
            .and_then(|arrangement| {
                resolve_arrangement_slides(item, arrangement, live_playlist_entry_id)
            })
            .unwrap_or_else(|| scope_master_slides(&master_slides, live_playlist_entry_id)),
    )
}

pub(crate) fn live_slide_index(
    slides: &[Value],
    current_id: &str,
    live_instance_id: Option<&str>,
) -> Option<usize> {
    live_instance_id
        .and_then(|instance_id| {
            slides.iter().position(|slide| {
                slide
                    .get("arrangementInstanceId")
                    .and_then(Value::as_str)
                    .is_some_and(|candidate| arrangement_instance_matches(instance_id, candidate))
            })
        })
        .or_else(|| {
            slides
                .iter()
                .position(|slide| slide.get("id").and_then(Value::as_str) == Some(current_id))
        })
}

fn resolve_arrangement_slides(
    item: &Value,
    arrangement: &Value,
    instance_scope: Option<&str>,
) -> Option<Vec<Value>> {
    let item_slides = item.get("slides")?.as_array()?;
    let arrangement_id = arrangement.get("id")?.as_str()?;
    let section_ids = arrangement.get("sectionIds")?.as_array()?;
    let mut slides_by_section: HashMap<String, Vec<Value>> = HashMap::new();
    for (index, slide) in item_slides.iter().enumerate() {
        let section_id = slide_section_id(item_slides, index);
        slides_by_section
            .entry(section_id.clone())
            .or_default()
            .push(slide.clone());
        if let Some(explicit) = slide.get("sectionId").and_then(Value::as_str) {
            if explicit != section_id {
                slides_by_section
                    .entry(explicit.to_string())
                    .or_default()
                    .push(slide.clone());
            }
        }
    }
    let mut resolved = Vec::new();
    for (section_index, section_id) in section_ids.iter().filter_map(Value::as_str).enumerate() {
        for (slide_index, slide) in slides_by_section
            .get(section_id)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let mut slide = slide.as_object()?.clone();
            let slide_id = slide.get("id").and_then(Value::as_str)?;
            slide.insert(
                "arrangementInstanceId".to_string(),
                json!(format!(
                    "{}{arrangement_id}:{section_index}:{slide_index}:{slide_id}",
                    instance_scope.map_or(String::new(), |scope| format!("{scope}:"))
                )),
            );
            resolved.push(Value::Object(slide));
        }
    }
    Some(resolved)
}

fn scope_master_slides(slides: &[Value], instance_scope: Option<&str>) -> Vec<Value> {
    let Some(scope) = instance_scope else {
        return slides.to_vec();
    };
    slides
        .iter()
        .enumerate()
        .map(|(slide_index, slide)| {
            let mut slide = slide.clone();
            let slide_id = slide.get("id").and_then(Value::as_str).map(str::to_string);
            if let (Some(object), Some(slide_id)) = (slide.as_object_mut(), slide_id) {
                object.insert(
                    "arrangementInstanceId".to_string(),
                    json!(format!("{scope}:master:0:{slide_index}:{slide_id}")),
                );
            }
            slide
        })
        .collect()
}

pub(crate) fn arrangement_instance_matches(live: &str, unscoped: &str) -> bool {
    live == unscoped
        || live
            .strip_suffix(unscoped)
            .is_some_and(|prefix| prefix.ends_with(':'))
}

pub(crate) fn slide_section_id(slides: &[Value], index: usize) -> String {
    let group_id = (0..=index)
        .rev()
        .find_map(|candidate| slide_group_id(&slides[candidate]))
        .unwrap_or_else(|| "no-group".to_string());
    if group_id == "no-group" {
        format!("section-{}", index + 1)
    } else {
        format!("section-{group_id}")
    }
}

pub(crate) fn slide_group_id(slide: &Value) -> Option<String> {
    let object = slide.as_object()?;
    let value = if object.contains_key("groupId") {
        object.get("groupId")
    } else {
        object.get("groupName")
    }?;
    let group = match value {
        Value::String(value) => value.trim().to_string(),
        Value::Null => String::new(),
        value => value.to_string(),
    };
    (!group.is_empty() && !is_generated_slide_metadata_label(&group)).then_some(group)
}

fn is_generated_slide_metadata_label(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    (lower.len() >= 8 && lower.bytes().all(|byte| byte.is_ascii_digit()))
        || lower == "new-slide"
        || lower == "new-slide-copy"
        || lower.strip_prefix("new-slide-copy-").is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        })
}
