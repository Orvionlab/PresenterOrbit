use super::*;

fn uses_chart(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.get("fillType").and_then(Value::as_str) == Some("chord-chart")
                || object
                    .iter()
                    .any(|(key, value)| key != "linkedTextContext" && uses_chart(value))
        }
        Value::Array(values) => values.iter().any(uses_chart),
        _ => false,
    }
}

fn path(content: &Value) -> Result<Option<PathBuf>, String> {
    if let Some(error) = content
        .get("linkedTextContext")
        .and_then(|context| context.get("chordChartError"))
        .and_then(Value::as_str)
    {
        return Err(error.to_string());
    }
    let value = content
        .get("linkedTextContext")
        .and_then(|context| context.get("chordChartPath"))
        .ok_or("Chord chart fill requires an explicit stage chart resource.")?;
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(|value| Some(PathBuf::from(value)))
        .ok_or_else(|| "Chord chart resource must be a local image path or null.".into())
}

pub(super) fn validate(content: &Value) -> Result<(), String> {
    if uses_chart(content) {
        if let Some(path) = path(content)? {
            crate::chord_chart::validate_page(&path)?;
        }
    }
    Ok(())
}

pub(super) fn prepare(content: &Value, renderer: &mut NativeRenderRuntime) -> Result<(), String> {
    if uses_chart(content) {
        if let Some(path) = path(content)? {
            renderer.image_frame(&path)?;
        }
    }
    Ok(())
}

pub(super) fn draw(
    output: &mut RenderTarget,
    slide: &SceneSlide,
    layer: &SceneLayer,
    context: &Value,
    renderer: &mut NativeRenderRuntime,
) -> bool {
    let mut chart_layer = layer.clone();
    for field in [
        "sourcePath",
        "mediaSourcePath",
        "mediaPath",
        "mediaUrl",
        "sourceUrl",
    ] {
        chart_layer.source.remove(field);
    }
    match context.get("chordChartPath").and_then(Value::as_str) {
        Some(path) => {
            chart_layer.source.insert("fillType".into(), json!("media"));
            chart_layer.source.insert("sourcePath".into(), json!(path));
            chart_layer
                .source
                .insert("mediaType".into(), json!("image"));
        }
        None => {
            chart_layer.source.insert("fillType".into(), json!("none"));
        }
    }
    draw_shape_layer(output, slide, &chart_layer, renderer)
}
