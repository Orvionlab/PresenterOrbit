use crate::native_compositor::{NativeCompositor, NativeFrame};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportRequest {
    pub item_id: String,
    pub format: String,
    pub arrangement_id: Option<String>,
    pub include_disabled: bool,
    pub include_notes: bool,
}

pub struct PreparedExport {
    pub text: String,
    pub scenes: Vec<Value>,
    pub dimensions: Vec<(usize, usize)>,
    /// Decoded video frames the scenes draw; removed with the prepared export.
    _stills: StillFrames,
}

/// Video sources drawn as one full-resolution decoded frame, so a static export shows a
/// deterministic picture: backgrounds at their media thumbnail time, slide objects and
/// text media fills at their first frame (as Orbit's slide stills show them).
#[derive(Default)]
struct StillFrames {
    directory: Option<PathBuf>,
    frames: HashMap<(PathBuf, u64), PathBuf>,
}

impl StillFrames {
    fn frame(&mut self, source: &Value, seconds: f64) -> Result<String, String> {
        let source = source
            .as_str()
            .filter(|source| !source.trim().is_empty())
            .ok_or("Video media has no renderable source.")?;
        let path = local_asset_path(source)?;
        let key = (path.clone(), seconds.to_bits());
        if let Some(still) = self.frames.get(&key) {
            return Ok(still.to_string_lossy().into_owned());
        }
        let png = crate::media_thumbnail::extract_frame(&path, seconds, None)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let directory = match &self.directory {
            Some(directory) => directory.clone(),
            None => {
                let directory = std::env::temp_dir()
                    .join(format!("orbit-export-stills-{}", uuid::Uuid::new_v4()));
                fs::create_dir(&directory)
                    .map_err(|error| format!("Cannot create video still folder: {error}"))?;
                self.directory = Some(directory.clone());
                directory
            }
        };
        let still = directory.join(format!("{}.png", self.frames.len()));
        fs::write(&still, png).map_err(|error| format!("Cannot write video still: {error}"))?;
        self.frames.insert(key, still.clone());
        Ok(still.to_string_lossy().into_owned())
    }

    /// Replaces the video sources in `value` by their still frames.
    fn freeze(&mut self, value: &mut Value, background_seconds: f64) -> Result<(), String> {
        match value {
            Value::Object(object) => {
                if is_kind(object.get("backgroundMediaType"), "video") {
                    let source =
                        first_source(object, &["backgroundMediaPath", "backgroundMediaUrl"]);
                    let still = self.frame(&source, background_seconds)?;
                    object.insert("backgroundMediaType".into(), json!("image"));
                    object.insert("backgroundMediaPath".into(), json!(still));
                    object.remove("backgroundMediaUrl");
                    object.remove("backgroundPlaybackState");
                }
                if is_kind(object.get("mediaType"), "video") || is_kind(object.get("type"), "video")
                {
                    let still = self.frame(&first_source(object, MEDIA_SOURCE_KEYS), 0.0)?;
                    for key in MEDIA_SOURCE_KEYS {
                        object.remove(*key);
                    }
                    object.insert("sourcePath".into(), json!(still));
                    if object.contains_key("mediaType") {
                        object.insert("mediaType".into(), json!("image"));
                    }
                    if is_kind(object.get("type"), "video") {
                        object.insert("type".into(), json!("Media"));
                    }
                    object.remove("mediaPlaybackState");
                }
                let fill = first_source(object, &["mediaFillSourcePath", "mediaFillUrl"]);
                if is_kind(object.get("mediaFillType"), "video")
                    || fill
                        .as_str()
                        .is_some_and(crate::native_compositor::is_video_media_path)
                {
                    let still = self.frame(&fill, 0.0)?;
                    object.insert("mediaFillType".into(), json!("image"));
                    object.insert("mediaFillSourcePath".into(), json!(still));
                    object.remove("mediaFillUrl");
                    object.remove("mediaFillPlaybackState");
                }
                for (key, child) in object.iter_mut() {
                    if key != "actions" {
                        self.freeze(child, background_seconds)?;
                    }
                }
            }
            Value::Array(values) => {
                for child in values {
                    self.freeze(child, background_seconds)?;
                }
            }
            _ => (),
        }
        Ok(())
    }
}

impl Drop for StillFrames {
    fn drop(&mut self) {
        if let Some(directory) = &self.directory {
            let _ = fs::remove_dir_all(directory);
        }
    }
}

const MEDIA_SOURCE_KEYS: &[&str] = &[
    "sourcePath",
    "mediaSourcePath",
    "mediaPath",
    "mediaUrl",
    "sourceUrl",
];

fn first_source(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Value {
    keys.iter()
        .filter_map(|key| object.get(*key))
        .find(|source| {
            source
                .as_str()
                .is_some_and(|source| !source.trim().is_empty())
        })
        .cloned()
        .unwrap_or(Value::Null)
}

fn is_kind(value: Option<&Value>, kind: &str) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case(kind))
}

pub fn prepare(workspace: &Value, request: &ExportRequest) -> Result<PreparedExport, String> {
    if !matches!(request.format.as_str(), "text" | "images" | "pdf") {
        return Err("Unsupported presentation export format.".into());
    }
    if request.format != "text" && request.include_notes {
        return Err(
            "Slide canvas exports do not include notes. Choose text to export notes.".into(),
        );
    }
    let (item, selected, slides) = select_slides(workspace, request)?;
    prepare_slides(workspace, request, item, selected, slides)
}

pub(crate) fn select_slides<'a>(
    workspace: &'a Value,
    request: &ExportRequest,
) -> Result<(&'a Value, Value, Vec<Value>), String> {
    let item = workspace["library"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["id"].as_str() == Some(&request.item_id))
        })
        .ok_or("Presentation is not in the active workspace.")?;
    let mut selected = item.clone();
    if let Some(id) = &request.arrangement_id {
        let arrangement = item["arrangements"]
            .as_array()
            .and_then(|values| values.iter().find(|value| value["id"].as_str() == Some(id)))
            .ok_or("Selected arrangement is not in the presentation.")?;
        if !arrangement["sectionIds"]
            .as_array()
            .is_some_and(|ids| ids.iter().all(Value::is_string))
        {
            return Err("Arrangement section references are invalid.".into());
        }
        selected["selectedArrangementId"] = json!(id);
        for section in arrangement["sectionIds"]
            .as_array()
            .expect("Validated sections")
        {
            let mut single = selected.clone();
            let target = single["arrangements"]
                .as_array_mut()
                .expect("Validated arrangements")
                .iter_mut()
                .find(|value| value["id"].as_str() == Some(id))
                .expect("Validated arrangement");
            target["sectionIds"] = json!([section]);
            if crate::output_arrangement::resolve_live_arrangement_slides(&single, None, None)
                .is_none_or(|slides| slides.is_empty())
            {
                return Err(format!("Arrangement refers to missing section {section}."));
            }
        }
    } else {
        selected
            .as_object_mut()
            .ok_or("Presentation must be an object.")?
            .remove("selectedArrangementId");
    }
    let slides = crate::output_arrangement::resolve_live_arrangement_slides(&selected, None, None)
        .ok_or("Presentation slide order is invalid.")?
        .into_iter()
        .filter(|slide| request.include_disabled || slide["disabled"].as_bool() != Some(true))
        .collect::<Vec<_>>();
    if slides.is_empty() {
        return Err("The selected export order contains no slides.".into());
    }
    if slides.len() > 500 {
        return Err("Export is limited to 500 slide occurrences per operation.".into());
    }
    Ok((item, selected, slides))
}

fn prepare_slides(
    workspace: &Value,
    request: &ExportRequest,
    item: &Value,
    selected: Value,
    slides: Vec<Value>,
) -> Result<PreparedExport, String> {
    let title = item["title"]
        .as_str()
        .ok_or("Presentation title is required.")?
        .to_string();
    let mut text = format!("{title}\n");
    if let Some(ccli) = item.get("ccli").and_then(Value::as_object) {
        for key in [
            "title",
            "author",
            "copyright",
            "copyrightYear",
            "publisher",
            "songNumber",
        ] {
            if let Some(value) = ccli.get(key).filter(|value| !value.is_null()) {
                text.push_str(&format!(
                    "{key}: {}\n",
                    value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string())
                ));
            }
        }
    }
    if request.include_notes {
        if let Some(notes) = item["operatorNotes"].as_str() {
            text.push_str(&format!("\nOperator notes:\n{notes}\n"));
        }
    }
    let fonts = if request.format == "text" {
        Vec::new()
    } else {
        crate::native_fonts::installed_font_families()
    };
    let mut scenes = Vec::new();
    let mut stills = StillFrames::default();
    let mut dimensions = Vec::new();
    for (index, slide) in slides.iter().enumerate() {
        let body = slide["content"]
            .as_str()
            .or_else(|| slide["text"].as_str())
            .unwrap_or("");
        text.push_str(&format!(
            "\n--- Slide {}{} ---\n{body}\n",
            index + 1,
            if slide["disabled"] == true {
                " (disabled)"
            } else {
                ""
            }
        ));
        if request.include_notes {
            let notes = slide["notes"].as_str().map(str::to_string).or_else(|| {
                slide["notesRuns"].as_array().map(|runs| {
                    runs.iter()
                        .filter_map(|run| run["text"].as_str())
                        .collect::<String>()
                })
            });
            if let Some(notes) = notes {
                text.push_str(&format!("Notes:\n{notes}\n"));
            }
        }
        if request.format == "text" {
            continue;
        }
        let theme = resolve_theme(workspace, item, slide)?;
        let mut composed = crate::slide_composition::compose_slide(slide, theme);
        // Static exports include every authored object, regardless of live build progress.
        composed["ignoreBuilds"] = json!(true);
        preflight(&composed, &fonts).map_err(|error| format!("Slide {}: {error}", index + 1))?;
        let parsed = crate::scene::parse_scene_slide(&composed)?;
        if parsed
            .layers
            .iter()
            .any(|layer| layer.kind == crate::scene::SceneLayerKind::Other)
        {
            return Err(format!(
                "Slide {} contains an unsupported visual object.",
                index + 1
            ));
        }
        let width = parsed.width;
        let height = parsed.height;
        if !width.is_finite()
            || !height.is_finite()
            || width < 1.0
            || height < 1.0
            || width > 4096.0
            || height > 4096.0
        {
            return Err(format!(
                "Slide {} dimensions must be between 1 and 4096 pixels.",
                index + 1
            ));
        }
        if let Some(media_id) = slide["mediaBackgroundId"].as_str() {
            let media = workspace["media"]
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .find(|media| media["id"].as_str() == Some(media_id))
                })
                .ok_or_else(|| format!("Slide {} background media is missing.", index + 1))?;
            preflight(media, &fonts)?;
            let source = media["sourceUrl"]
                .as_str()
                .filter(|source| !source.is_empty())
                .ok_or("Background media URL is required.")?;
            validate_asset(source)?;
        }
        let background_seconds = slide["mediaBackgroundId"]
            .as_str()
            .and_then(|id| {
                workspace["media"]
                    .as_array()?
                    .iter()
                    .find(|media| media["id"].as_str() == Some(id))
            })
            .and_then(|media| media["thumbnailTimeSeconds"].as_f64())
            .unwrap_or(0.0);
        let mut native_scenes = crate::output_scene::build_output_scene_state(&json!({
            "liveItem": selected, "liveSlide": slide, "liveSlideId": slide["id"], "liveCueInstanceId": slide["arrangementInstanceId"],
            "libraries": workspace["libraries"], "themes": workspace["themes"], "media": workspace["media"],
            "screenConfig": {"export": {"type":"audience"}}, "liveBuildStep": 0
        }))?;
        let mut scene = native_scenes
            .remove("export")
            .ok_or("Native export scene is missing.")?;
        scene["presentationThemeSlide"] = composed;
        stills
            .freeze(&mut scene, background_seconds)
            .map_err(|error| format!("Slide {}: {error}", index + 1))?;
        scene["linkedTextContext"] = json!({"item": item, "slide": slide, "nextSlide": slides.get(index + 1),
            "ccli": item["ccli"], "presentationName": title, "operatorNotes": item["operatorNotes"],
            "slideCount": slides.len(), "slideIndex": index});
        scenes.push(scene);
        dimensions.push((width.round() as usize, height.round() as usize));
    }
    if dimensions
        .iter()
        .map(|(width, height)| width * height)
        .sum::<usize>()
        > 64_000_000
    {
        return Err(
            "Visual export exceeds the 64 megapixel operation limit. Export a smaller arrangement."
                .into(),
        );
    }
    Ok(PreparedExport {
        text,
        scenes,
        dimensions,
        _stills: stills,
    })
}

fn resolve_theme(workspace: &Value, item: &Value, slide: &Value) -> Result<Option<Value>, String> {
    let input: crate::output_scene::OutputSceneStateInput = serde_json::from_value(
        json!({"liveItem": item, "liveSlide": slide, "libraries": workspace["libraries"]}),
    )
    .map_err(|error| format!("Invalid library theme assignment: {error}"))?;
    let assignment = crate::output_scene::resolve_content_theme_id(&input);
    if let Some(id) = assignment.as_deref().filter(|id| !id.is_empty()) {
        let (theme_id, slide_id) = id
            .split_once("::")
            .ok_or("Presentation theme reference is invalid.")?;
        let theme = workspace["themes"]
            .as_array()
            .and_then(|themes| {
                themes
                    .iter()
                    .find(|theme| theme["id"].as_str() == Some(theme_id))
            })
            .and_then(|theme| theme["slides"].as_array())
            .and_then(|slides| {
                slides
                    .iter()
                    .find(|slide| slide["id"].as_str() == Some(slide_id))
            })
            .cloned();
        return theme
            .map(Some)
            .ok_or_else(|| format!("Presentation theme {id} is missing."));
    }
    Ok(slide.get("presentationThemeSnapshot").cloned())
}

fn preflight(value: &Value, fonts: &[String]) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            if object.get("scrollingEnabled") == Some(&Value::Bool(true)) {
                return Err("Scrolling text is not supported in static export.".into());
            }
            for key in ["type", "mediaType", "mediaFillType", "backgroundMediaType"] {
                if object.get(key).and_then(Value::as_str).is_some_and(|kind| {
                    matches!(
                        kind.to_lowercase().as_str(),
                        "audio" | "web" | "webpage" | "ndi" | "live video" | "video input"
                    )
                }) {
                    return Err("Audio, web and live-input objects cannot be exported as static slide canvases.".into());
                }
            }
            if object
                .get("ndiSource")
                .is_some_and(|value| !value.is_null())
                || object
                    .get("webUrl")
                    .and_then(Value::as_str)
                    .is_some_and(|url| !url.is_empty())
            {
                return Err(
                    "Live-input and web objects are not renderable in static export.".into(),
                );
            }
            if let Some(source) = object.get("linkedTextSource").and_then(Value::as_str) {
                if object.get("linkedTextEnabled") != Some(&Value::Bool(false))
                    && !matches!(
                        source,
                        "None"
                            | "CCLI"
                            | "Current Slide"
                            | "Next Slide"
                            | "Slide Notes"
                            | "Next Slide Notes"
                            | "Slide Label"
                            | "Next Slide Label"
                            | "Presentation Name"
                            | "Operator Notes"
                    )
                {
                    return Err(format!(
                        "Linked text source {source} is not supported in independent export."
                    ));
                }
            }
            if object
                .get("fontFamily")
                .and_then(Value::as_str)
                .is_some_and(|font| {
                    !font.is_empty()
                        && !fonts
                            .iter()
                            .any(|available| available.eq_ignore_ascii_case(font))
                })
            {
                return Err(format!(
                    "Requested font {} is not installed. Choose a replacement before export.",
                    object["fontFamily"]
                ));
            }
            if object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| {
                    matches!(kind.to_lowercase().as_str(), "image" | "media" | "video")
                })
                && ![
                    "sourcePath",
                    "sourceUrl",
                    "mediaSourcePath",
                    "mediaPath",
                    "mediaUrl",
                ]
                .iter()
                .any(|key| {
                    object
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|source| !source.trim().is_empty())
                })
            {
                return Err("Image object has no renderable source.".into());
            }
            if object
                .get("textFillMode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode.eq_ignore_ascii_case("Media"))
                && !["mediaFillSourcePath", "mediaFillUrl"].iter().any(|key| {
                    object
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|source| !source.trim().is_empty())
                })
            {
                return Err("Text media fill has no renderable source.".into());
            }
            if object
                .get("background")
                .and_then(Value::as_str)
                .is_some_and(|background| background.contains("url("))
            {
                return Err(
                    "CSS image backgrounds are not supported by native slide export.".into(),
                );
            }
            for key in [
                "sourcePath",
                "sourceUrl",
                "mediaUrl",
                "mediaPath",
                "mediaSourcePath",
                "mediaFillSourcePath",
                "mediaFillUrl",
                "backgroundMediaPath",
                "backgroundMediaUrl",
            ] {
                if let Some(source) = object
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|source| !source.is_empty())
                {
                    validate_asset(source)?;
                }
            }
            for (key, value) in object {
                // Slide actions run on output; they draw nothing on the static canvas.
                if key != "actions" {
                    preflight(value, fonts)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                preflight(value, fonts)?;
            }
        }
        _ => (),
    }
    Ok(())
}

fn local_asset_path(source: &str) -> Result<PathBuf, String> {
    if source.starts_with("file://") {
        url::Url::parse(source)
            .map_err(|error| error.to_string())?
            .to_file_path()
            .map_err(|_| "Invalid local asset URL.".into())
    } else if source.contains("://") {
        Err(format!(
            "Remote asset cannot be exported independently: {source}"
        ))
    } else {
        Ok(PathBuf::from(source))
    }
}

fn validate_asset(source: &str) -> Result<(), String> {
    let path = local_asset_path(source)?;
    if !path.is_file() {
        return Err(format!("Missing asset: {}", path.display()));
    }
    // Videos are decoded when their still frame is extracted.
    if crate::native_compositor::is_video_media_path(&path.to_string_lossy()) {
        return Ok(());
    }
    let bytes = fs::read(&path).map_err(|error| format!("Asset read failed: {error}"))?;
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        resvg::usvg::Tree::from_data(&bytes, &resvg::usvg::Options::default())
            .map_err(|error| format!("Invalid SVG asset: {error}"))?;
    } else {
        image::guess_format(&bytes)
            .map_err(|error| format!("Asset is not a supported static image: {error}"))?;
        // Animated images export at their first frame.
        image::load_from_memory(&bytes)
            .map_err(|error| format!("Image asset cannot be decoded: {error}"))?;
    }
    Ok(())
}

pub fn render(prepared: &PreparedExport) -> Result<Vec<NativeFrame>, String> {
    let mut compositor = NativeCompositor::new_static_preview();
    prepared
        .scenes
        .iter()
        .zip(&prepared.dimensions)
        .map(|(scene, &(width, height))| {
            let frame =
                compositor.try_render_preview_for("presentation-export", scene, width, height)?;
            if frame.dynamic_media {
                return Err("Dynamic content was detected. Export canceled before writing.".into());
            }
            Ok(frame)
        })
        .collect()
}

pub fn write_export(
    prepared: &PreparedExport,
    format: &str,
    destination: &Path,
) -> Result<(), String> {
    if format == "text" {
        return write_new(destination, prepared.text.as_bytes());
    }
    let frames = render(prepared)?;
    if format == "pdf" {
        return write_new(destination, &encode_pdf(&frames)?);
    }
    if format != "images" {
        return Err("Unsupported export format.".into());
    }
    // Encode before creating the folder, so failed rasterization leaves no partial export.
    let pngs = frames
        .iter()
        .map(NativeFrame::encode_png)
        .collect::<Result<Vec<_>, _>>()?;
    fs::create_dir(destination).map_err(|error| format!("Cannot create export folder: {error}"))?;
    for (index, png) in pngs.iter().enumerate() {
        if let Err(error) = write_new(
            &destination.join(format!("slide-{:04}.png", index + 1)),
            png,
        ) {
            for written in 0..index {
                let _ = fs::remove_file(destination.join(format!("slide-{:04}.png", written + 1)));
            }
            let _ = fs::remove_dir(destination);
            return Err(error);
        }
    }
    Ok(())
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Cannot create export file: {error}"))?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(format!("Export write failed: {error}"));
    }
    Ok(())
}

pub fn encode_pdf(frames: &[NativeFrame]) -> Result<Vec<u8>, String> {
    encode_pdf_with_page_scale(frames, 1.0)
}

pub(crate) fn encode_pdf_with_page_scale(
    frames: &[NativeFrame],
    scale: f64,
) -> Result<Vec<u8>, String> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err("PDF page scale must be positive.".into());
    }
    if frames.is_empty() {
        return Err("PDF requires at least one page.".into());
    }
    let mut objects = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), Vec::new()];
    let kids = (0..frames.len())
        .map(|index| format!("{} 0 R", 3 + index * 3))
        .collect::<Vec<_>>()
        .join(" ");
    objects[1] = format!("<< /Type /Pages /Count {} /Kids [{kids}] >>", frames.len()).into_bytes();
    for (index, frame) in frames.iter().enumerate() {
        if frame.width == 0
            || frame.height == 0
            || frame.pixels.len()
                != frame
                    .width
                    .checked_mul(frame.height)
                    .and_then(|count| count.checked_mul(4))
                    .ok_or("PDF image size overflow.")?
        {
            return Err("PDF frame dimensions are invalid.".into());
        }
        let image_id = 4 + index * 3;
        let content_id = image_id + 1;
        let page_width = frame.width as f64 * scale;
        let page_height = frame.height as f64 * scale;
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_width} {page_height}] /Resources << /XObject << /Slide {image_id} 0 R >> >> /Contents {content_id} 0 R >>").into_bytes());
        let rgb = frame
            .pixels
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0]])
            .collect::<Vec<_>>();
        let mut image = format!("<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n", frame.width, frame.height, rgb.len()).into_bytes();
        image.extend(rgb);
        image.extend(b"\nendstream");
        objects.push(image);
        let instructions = format!("q {} 0 0 {} 0 0 cm /Slide Do Q\n", page_width, page_height);
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{instructions}endstream",
                instructions.len()
            )
            .into_bytes(),
        );
    }
    let mut pdf = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend(object);
        pdf.extend(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        pdf.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    Ok(pdf)
}
