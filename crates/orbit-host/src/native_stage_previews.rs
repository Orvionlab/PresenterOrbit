use super::*;
use serde_json::Map;

const MAX_PREVIEW_OUTPUTS: usize = 16;
const MAX_PREVIEW_EDGE: usize = 1024;

pub(super) fn is_preview(source: &Map<String, Value>) -> bool {
    matches!(
        source.get("fillType").and_then(Value::as_str),
        Some(
            "current-slide-image"
                | "next-slide-image"
                | "screen-preview"
                | "unsupported-slide-image"
        )
    )
}

fn preview_key(source: &Map<String, Value>) -> Result<String, String> {
    match source.get("fillType").and_then(Value::as_str) {
        Some("current-slide-image") => Ok("currentSlide".into()),
        Some("next-slide-image") => Ok("nextSlide".into()),
        Some("screen-preview") => source
            .get("previewScreenId")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(|id| format!("output:{id}"))
            .ok_or_else(|| "Screen preview requires an exact output screen ID.".into()),
        _ => Err("Unsupported stage preview fill.".into()),
    }
}

fn resource<'a>(registry: &'a Value, key: &str) -> Result<Option<&'a Value>, String> {
    let value = if let Some(id) = key.strip_prefix("output:") {
        registry.get("outputs").and_then(|outputs| outputs.get(id))
    } else {
        registry.get(key)
    }
    .ok_or_else(|| format!("Stage preview source '{key}' is missing."))?;
    if value.is_null() && !key.starts_with("output:") {
        return Ok(None);
    }
    let dimensions_valid = ["width", "height"].iter().all(|field| {
        value
            .get(field)
            .and_then(Value::as_u64)
            .is_some_and(|size| size > 0 && size <= 8192)
    });
    if !dimensions_valid || !value.get("scene").is_some_and(Value::is_object) {
        return Err(format!(
            "Stage preview source '{key}' requires a scene and valid canvas dimensions."
        ));
    }
    if value["scene"]
        .get("linkedTextContext")
        .and_then(|context| context.get("stageSceneResources"))
        .is_some()
    {
        return Err("Stage preview source scenes must have a detached resource registry.".into());
    }
    Ok(Some(value))
}

fn references(value: &Value, keys: &mut Vec<String>) -> Result<(), String> {
    if let Some(object) = value.as_object() {
        if is_preview(object) {
            let key = preview_key(object)?;
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        for (key, child) in object {
            if !matches!(key.as_str(), "linkedTextContext" | "stageSceneResources") {
                references(child, keys)?;
            }
        }
    } else if let Some(values) = value.as_array() {
        for child in values {
            references(child, keys)?;
        }
    }
    Ok(())
}

pub(super) fn validate_fields(content: &Value) -> Result<(), String> {
    references(content, &mut Vec::new())
}

pub(super) fn sample_registry(slide: &Value) -> Result<Value, String> {
    let mut keys = Vec::new();
    references(slide, &mut keys)?;
    let empty_screen = json!({"width":1,"height":1,"scene":{"backgroundCss":"transparent"}});
    let outputs = keys
        .iter()
        .filter_map(|key| key.strip_prefix("output:"))
        .map(|id| (id.to_owned(), empty_screen.clone()))
        .collect::<Map<_, _>>();
    Ok(json!({"outputs":outputs,"currentSlide":null,"nextSlide":null}))
}

pub(super) fn validate(content: &Value) -> Result<(), String> {
    let mut keys = Vec::new();
    references(content, &mut keys)?;
    if keys.is_empty() {
        return Ok(());
    }
    let context = content
        .get("linkedTextContext")
        .ok_or("Stage previews require native scene resources.")?;
    let registry = context
        .get("stageSceneResources")
        .ok_or("Stage previews require native scene resources.")?;
    let outputs = registry
        .get("outputs")
        .and_then(Value::as_object)
        .ok_or("Stage preview output registry must be an object.")?;
    if outputs.len() > MAX_PREVIEW_OUTPUTS {
        return Err("Stage preview source count exceeds 16.".into());
    }
    let mut path = Vec::new();
    if let Some(id) = context.get("stagePreviewScreenId").and_then(Value::as_str) {
        path.push(format!("output:{id}"));
    }
    fn visit(
        registry: &Value,
        key: &str,
        path: &mut Vec<String>,
        validated: &mut std::collections::HashSet<String>,
    ) -> Result<(), String> {
        if path.iter().any(|entry| entry == key) {
            return Err(format!(
                "Cyclic stage preview: {} -> {key}.",
                path.join(" -> ")
            ));
        }
        if validated.contains(key) {
            return Ok(());
        }
        let Some(source) = resource(registry, key)? else {
            return Ok(());
        };
        super::validate_object_links(&source["scene"])?;
        native_chart::validate(&source["scene"])?;
        path.push(key.to_owned());
        let mut children = Vec::new();
        references(&source["scene"], &mut children)?;
        for child in children {
            visit(registry, &child, path, validated)?;
        }
        path.pop();
        validated.insert(key.to_owned());
        Ok(())
    }
    let mut validated = std::collections::HashSet::new();
    for key in keys {
        visit(registry, &key, &mut path, &mut validated)?;
    }
    Ok(())
}

pub(super) fn prepare(
    content: &Value,
    pool: &mut HashMap<String, NativeRenderRuntime>,
    parent: &mut NativeRenderRuntime,
) -> Result<(), String> {
    validate(content)?;
    let mut keys = Vec::new();
    references(content, &mut keys)?;
    let Some(registry) = content
        .get("linkedTextContext")
        .and_then(|context| context.get("stageSceneResources"))
    else {
        pool.clear();
        parent.stage_preview_frames.clear();
        return Ok(());
    };
    fn collect(
        key: &str,
        registry: &Value,
        active: &mut HashMap<String, ()>,
    ) -> Result<(), String> {
        if active.contains_key(key) {
            return Ok(());
        }
        let Some(resource) = resource(registry, key)? else {
            return Ok(());
        };
        active.insert(key.to_owned(), ());
        let mut children = Vec::new();
        references(&resource["scene"], &mut children)?;
        for child in children {
            collect(&child, registry, active)?;
        }
        Ok(())
    }
    let mut active = HashMap::new();
    for key in &keys {
        collect(key, registry, &mut active)?;
    }
    if active.len() > MAX_PREVIEW_OUTPUTS + 2 {
        return Err("Stage preview runtime count exceeds 18.".into());
    }
    pool.retain(|key, _| active.contains_key(key));
    fn visit(
        key: &str,
        registry: &Value,
        pool: &mut HashMap<String, NativeRenderRuntime>,
        frames: &mut HashMap<String, Arc<NativeFrame>>,
    ) -> Result<(), String> {
        if frames.contains_key(key) {
            return Ok(());
        }
        let Some(resource) = resource(registry, key)? else {
            return Ok(());
        };
        let mut scene = resource["scene"].clone();
        let mut children = Vec::new();
        references(&scene, &mut children)?;
        for child in children {
            visit(&child, registry, pool, frames)?;
        }
        let source_width = resource["width"].as_u64().unwrap() as usize;
        let source_height = resource["height"].as_u64().unwrap() as usize;
        let factor = (MAX_PREVIEW_EDGE as f64 / source_width.max(source_height) as f64).min(1.0);
        let width = (source_width as f64 * factor).round().max(1.0) as usize;
        let height = (source_height as f64 * factor).round().max(1.0) as usize;
        let renderer = pool.entry(key.to_owned()).or_insert_with(|| {
            NativeRenderRuntime::new_with_video_max_size(Some((width as u32, height as u32)))
        });
        renderer.set_video_max_size(Some((width as u32, height as u32)));
        native_chart::prepare(&scene, renderer)?;
        renderer.static_images = !key.starts_with("output:");
        renderer.stage_preview_frames = frames.clone();
        if !key.starts_with("output:") {
            freeze_video_playback(&mut scene);
        }
        let mut pixels = vec![0; width * height * 4];
        let mut target = RenderTarget {
            pixels: &mut pixels,
            width,
            height,
            dynamic_media: false,
            scroll_elapsed_seconds: native_output_elapsed_seconds(),
        };
        renderer.begin_frame();
        compose_frame(
            &mut target,
            &scene,
            renderer,
            &mut std::collections::VecDeque::new(),
        );
        renderer.end_frame();
        let dynamic_media = target.dynamic_media;
        frames.insert(
            key.to_owned(),
            Arc::new(NativeFrame {
                width,
                height,
                pixels,
                dynamic_media,
            }),
        );
        Ok(())
    }
    let mut frames = HashMap::new();
    for key in keys {
        visit(&key, registry, pool, &mut frames)?;
    }
    pool.retain(|key, _| frames.contains_key(key));
    if pool.len() > MAX_PREVIEW_OUTPUTS + 2 {
        return Err("Stage preview runtime count exceeds 18.".into());
    }
    parent.stage_preview_frames = frames;
    Ok(())
}

/// Pauses every video in a scene at its first frame, for stills.
pub(super) fn freeze_video_playback(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        if object.get("mediaFillType").and_then(Value::as_str) == Some("video")
            || object
                .get("mediaFillSourcePath")
                .or_else(|| object.get("mediaFillUrl"))
                .and_then(Value::as_str)
                .is_some_and(is_video_media_path)
        {
            object.insert(
                "mediaFillPlaybackState".into(),
                json!({"status":"paused","currentTime":0,"clockRevision":0}),
            );
        }
        if object
            .get("mediaType")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("video"))
        {
            object.insert(
                "mediaPlaybackState".into(),
                json!({"status":"paused","currentTime":0,"clockRevision":0}),
            );
        }
        if object.get("backgroundMediaType").and_then(Value::as_str) == Some("video") {
            object.insert(
                "backgroundPlaybackState".into(),
                json!({"status":"paused","currentTime":0,"clockRevision":0}),
            );
        }
        for child in object.values_mut() {
            freeze_video_playback(child);
        }
    } else if let Some(values) = value.as_array_mut() {
        for child in values {
            freeze_video_playback(child);
        }
    }
}

pub(super) fn draw(
    output: &mut RenderTarget,
    slide: &SceneSlide,
    layer: &SceneLayer,
    _context: &Value,
    renderer: &mut NativeRenderRuntime,
) {
    let Ok(key) = preview_key(&layer.source) else {
        return;
    };
    let Some(frame) = renderer.stage_preview_frames.get(&key) else {
        return;
    };
    let width = frame.width;
    let height = frame.height;
    let pixels = &frame.pixels;
    output.dynamic_media |= frame.dynamic_media;
    let bounds = layer.source.get("bounds").and_then(Value::as_object);
    let x = bounds.and_then(|v| json_number(v.get("x"))).unwrap_or(0.0);
    let y = bounds.and_then(|v| json_number(v.get("y"))).unwrap_or(0.0);
    let bw = bounds
        .and_then(|v| json_number(v.get("width")))
        .unwrap_or(slide.width);
    let bh = bounds
        .and_then(|v| json_number(v.get("height")))
        .unwrap_or(slide.height);
    if bw <= 0.0 || bh <= 0.0 {
        return;
    }
    let geometry = ShapeGeometry::from_layer(&layer.source, x, y, bw, bh);
    let opacity =
        (json_number(layer.source.get("opacity")).unwrap_or(100.0) / 100.0).clamp(0.0, 1.0) as f32;
    let feather = shape_feather_amount(&layer.source);
    let scale = layer
        .source
        .get("mediaScale")
        .and_then(Value::as_str)
        .unwrap_or("Scale to Fit");
    let object_width = bw / slide.width * output.width as f64;
    let object_height = bh / slide.height * output.height as f64;
    let fit = if scale == "Scale to Fill" {
        (object_width / width as f64).max(object_height / height as f64)
    } else {
        (object_width / width as f64).min(object_height / height as f64)
    };
    let (draw_width, draw_height) = if scale == "Stretch to Fill" {
        (object_width, object_height)
    } else {
        (width as f64 * fit, height as f64 * fit)
    };
    for py in 0..output.height {
        for px in 0..output.width {
            let u = (px as f64 / output.width as f64 * slide.width - x) / bw;
            let v = (py as f64 / output.height as f64 * slide.height - y) / bh;
            let sx = (u.clamp(0.0, 1.0 - f64::EPSILON) * object_width
                - (object_width - draw_width) / 2.0)
                / draw_width;
            let sy = (v.clamp(0.0, 1.0 - f64::EPSILON) * object_height
                - (object_height - draw_height) / 2.0)
                / draw_height;
            if !(0.0..1.0).contains(&sx) || !(0.0..1.0).contains(&sy) {
                continue;
            }
            let offset = (((sy * height as f64) as usize).min(height - 1) * width
                + ((sx * width as f64) as usize).min(width - 1))
                * 4;
            let alpha = shape_feather_alpha(
                &geometry,
                u as f32,
                v as f32,
                object_width as f32,
                object_height as f32,
                feather,
            );
            blend_pixel(
                output.pixels,
                output.width,
                px,
                py,
                pixels[offset..offset + 4].try_into().unwrap(),
                opacity * alpha,
            );
        }
    }
}
