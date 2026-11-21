use super::*;
use std::collections::{BTreeSet, VecDeque};

fn requests(
    value: &Value,
    outputs: &mut BTreeSet<String>,
    images: &mut [bool; 2],
) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            match object.get("fillType").and_then(Value::as_str) {
                Some("current-slide-image") => images[0] = true,
                Some("next-slide-image") => images[1] = true,
                Some("screen-preview") => {
                    let id = object
                        .get("previewScreenId")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .ok_or("Output preview requires a source screen.")?;
                    outputs.insert(id.to_string());
                }
                _ => {}
            }
            for value in object.values() {
                requests(value, outputs, images)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                requests(value, outputs, images)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn slide_scene(input: &OutputSceneStateInput, next: bool) -> Result<Value, String> {
    let mut selected = input.clone();
    if next {
        let Some(slide) = next_enabled_slide_for_live_item(input) else {
            return Ok(Value::Null);
        };
        selected.live_slide_id = string_field(&slide, "id");
        selected.live_cue_instance_id = string_field(&slide, "arrangementInstanceId");
        selected.live_slide = Some(slide);
        selected.live_build_step = 0.0;
    }
    if selected.live_slide.is_none() {
        return Ok(Value::Null);
    }
    let config: OutputScreenConfigInput = serde_json::from_value(json!({
        "type":"audience", "label":"Slide image", "lookLayers":{
            "slide":true, "media":false, "props":false, "music":false,
            "messages":false, "announcements":false, "videoInput":false
        }
    }))
    .map_err(|error| error.to_string())?;
    let scene = build_screen_scene(&selected, "slide-image", &config, None)?;
    let slide = scene
        .presentation_theme_slide
        .as_ref()
        .ok_or("Slide image requires a composed slide.")?;
    let parsed = crate::scene::parse_scene_slide(slide)?;
    Ok(
        json!({"scene":scene, "width":parsed.width.round() as u64, "height":parsed.height.round() as u64}),
    )
}

pub(super) fn attach(
    input: &OutputSceneStateInput,
    scenes: &mut Map<String, Value>,
) -> Result<(), String> {
    let detached = scenes.clone();
    for (screen_id, scene) in scenes.iter_mut() {
        let mut required = BTreeSet::new();
        let mut images = [false; 2];
        requests(scene, &mut required, &mut images)?;
        if required.is_empty() && !images.iter().any(|requested| *requested) {
            continue;
        }
        let current = slide_scene(input, false)?;
        let next = slide_scene(input, true)?;
        let mut pending: VecDeque<_> = required.iter().cloned().collect();
        let mut prepared = BTreeSet::new();
        let mut output_resources = Map::new();
        let mut image_prepared = [false; 2];
        loop {
            for index in 0..2 {
                if images[index] && !image_prepared[index] {
                    image_prepared[index] = true;
                    let source = if index == 0 { &current } else { &next };
                    let before = required.clone();
                    requests(source, &mut required, &mut images)?;
                    pending.extend(required.difference(&before).cloned());
                }
            }
            let Some(source_id) = pending.pop_front() else {
                if (0..2).any(|index| images[index] && !image_prepared[index]) {
                    continue;
                }
                break;
            };
            if !prepared.insert(source_id.clone()) {
                continue;
            }
            if prepared.len() > 16 {
                return Err("Output preview resources exceed 16 source screens.".into());
            }
            let source = detached
                .get(&source_id)
                .ok_or_else(|| format!("Output preview source '{source_id}' is missing."))?;
            let size = input.output_canvas_sizes.get(&source_id).ok_or_else(|| {
                format!("Output preview source '{source_id}' has no canvas size.")
            })?;
            let before = required.clone();
            requests(source, &mut required, &mut images)?;
            pending.extend(required.difference(&before).cloned());
            output_resources.insert(
                source_id,
                json!({"scene":source,"width":size["width"],"height":size["height"]}),
            );
        }
        let context = scene
            .get_mut("linkedTextContext")
            .and_then(Value::as_object_mut)
            .ok_or("Stage previews require linked text context.")?;
        context.insert("stagePreviewScreenId".into(), json!(screen_id));
        context.insert(
            "stageSceneResources".into(),
            json!({
                "currentSlide": if images[0] { current } else { Value::Null },
                "nextSlide": if images[1] { next } else { Value::Null },
                "outputs":output_resources
            }),
        );
        crate::native_compositor::validate_stage_previews(scene)?;
    }
    Ok(())
}
