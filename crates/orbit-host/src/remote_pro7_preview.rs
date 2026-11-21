use serde_json::{json, Value};
use std::time::{Duration, Instant};

const MAX_SLIDES: usize = 500;
const MAX_IMAGE_BYTES: usize = 24 * 1024 * 1024;

/// `catalog` holds the `libraries` and `playlists` Remote catalogs built from `document`.
pub(super) fn render_with_catalog(
    document: &Value,
    catalog: &Value,
    path: &str,
    expected: &[Value],
    quality: u8,
) -> Result<Vec<String>, String> {
    if !(1..=100).contains(&quality) || expected.len() > MAX_SLIDES {
        return Err("Slide previews require quality 1–100 and at most 500 slides.".into());
    }
    let resolved = super::data::resolve(catalog, path)?;
    let source = document["library"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["id"] == resolved.cue["presentationId"])
        })
        .ok_or("The preview presentation is unavailable.")?;
    let mut item = source.clone();
    if resolved.cue["command"] == "playlistCue" {
        let entry = document["playlist"]
            .as_array()
            .and_then(|lists| {
                lists
                    .iter()
                    .find(|list| list["id"] == resolved.cue["playlistId"])
            })
            .and_then(|list| list["items"].as_array())
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry["id"] == resolved.cue["playlistEntryId"])
            })
            .ok_or("The preview playlist occurrence is unavailable.")?;
        if let Some(id) = entry["arrangementId"].as_str() {
            item["selectedArrangementId"] = json!(id);
        }
    }
    let item = crate::workspace::content_themes::compose_item(
        document
            .as_object()
            .ok_or("Preview workspace must be an object.")?,
        &item,
    );
    let slides = crate::output_arrangement::resolve_live_arrangement_slides(
        &item,
        expected
            .first()
            .and_then(|slide| slide["cueInstanceId"].as_str()),
        resolved.cue["playlistEntryId"].as_str(),
    )
    .ok_or("The preview arrangement is unavailable.")?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut bytes = 0usize;
    let mut images = Vec::with_capacity(expected.len());
    for summary in expected {
        let matches: Vec<_> = slides
            .iter()
            .filter(|slide| {
                slide["id"] == summary["id"]
                    && slide["arrangementInstanceId"] == summary["cueInstanceId"]
            })
            .collect();
        let [slide] = matches.as_slice() else {
            return Err("The preview cue occurrence changed; refresh the presentation.".into());
        };
        if super::data::text(slide, "content") != super::data::text(summary, "text")
            || super::data::text(slide, "notes") != super::data::text(summary, "notes")
        {
            return Err("The preview cue content changed; refresh the presentation.".into());
        }
        if Instant::now() >= deadline {
            return Err("Slide preview rendering exceeded its time budget. Request text-only slides with quality 0.".into());
        }
        let encoded =
            crate::workspace::base64(&jpeg(slide, &slide_background(document, slide), quality)?);
        if Instant::now() >= deadline {
            return Err("Slide preview rendering exceeded its time budget. Request text-only slides with quality 0.".into());
        }
        bytes = bytes
            .checked_add(encoded.len())
            .ok_or("Slide preview size overflowed.")?;
        if bytes > MAX_IMAGE_BYTES {
            return Err("Slide previews exceed the 24 MiB response budget. Reduce quality or request text-only slides with quality 0.".into());
        }
        images.push(encoded);
    }
    Ok(images)
}

/// The slide's own media background, matching the cue previews in Orbit.
fn slide_background(document: &Value, slide: &Value) -> Value {
    let media = slide["mediaBackgroundId"].as_str().and_then(|id| {
        document["media"].as_array()?.iter().find(|media| {
            media["id"] == id
                && matches!(media["type"].as_str(), Some("image" | "video"))
                && media["sourceUrl"]
                    .as_str()
                    .is_some_and(|url| !url.is_empty())
        })
    });
    let Some(media) = media else {
        return json!({});
    };
    json!({
        "backgroundMediaType": media["type"],
        // Like live output, slide media sits in front of the slide's own background unless marked as background.
        "backgroundBehavior": if media["backgroundBehavior"] == "background" { "background" } else { "foreground" },
        "backgroundMediaPath": media.get("sourcePath").cloned().unwrap_or(json!("")),
        "backgroundMediaUrl": media["sourceUrl"],
        "backgroundScale": media["backgroundScale"].as_str().filter(|scale| !scale.is_empty()).unwrap_or("fit"),
        "backgroundDesaturate": media.get("mediaDesaturate").and_then(Value::as_bool).unwrap_or(false),
        "backgroundBrightness": media.get("mediaBrightness").cloned().unwrap_or(Value::Null),
        "backgroundContrast": media.get("mediaContrast").cloned().unwrap_or(Value::Null)
    })
}

fn jpeg(slide: &Value, background: &Value, quality: u8) -> Result<Vec<u8>, String> {
    let (width, height) = size(slide, quality)?;
    let frame =
        crate::native_compositor::render_slide_still_over(slide, background, width, height)?;
    let mut rgb = Vec::with_capacity(width * height * 3);
    for pixel in frame.pixels.chunks_exact(4) {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode(
            &rgb,
            width as u32,
            height as u32,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|error| format!("Slide preview JPEG encoding failed: {error}"))?;
    Ok(jpeg)
}

fn size(slide: &Value, quality: u8) -> Result<(usize, usize), String> {
    for key in ["width", "height"] {
        if slide.get(key).is_some_and(|value| {
            value
                .as_f64()
                .is_none_or(|value| !value.is_finite() || value <= 0.0)
        }) {
            return Err(format!("Slide preview {key} must be finite and positive."));
        }
    }
    let scene = crate::scene::SceneSlide::from_value(slide)?;
    let (width, height) = (scene.width, scene.height);
    let longest = 960.0 * f64::from(quality) / 100.0;
    let scale = longest / width.max(height);
    Ok((
        (width * scale).round().max(1.0) as usize,
        (height * scale).round().max(1.0) as usize,
    ))
}
