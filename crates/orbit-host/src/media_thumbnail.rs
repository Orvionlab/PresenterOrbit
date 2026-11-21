use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub fn source(workspace_path: &Path, media_id: &str) -> Result<PathBuf, String> {
    let workspace = crate::workspace::read_workspace(workspace_path)?;
    let item = workspace["media"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["id"].as_str() == Some(media_id))
        })
        .ok_or("Video asset was not found.")?;
    if item["type"] != "video" {
        return Err("Thumbnail extraction requires a video asset.".into());
    }
    asset_path(workspace_path, item)
}

/// A still of an image or video asset with an effect chain applied, as a PNG
/// data URL no larger than 480 pixels. Videos use their thumbnail time.
pub fn effect_preview(
    workspace_path: &Path,
    media_id: &str,
    chain: &Value,
) -> Result<String, String> {
    let affected = effect_preview_image(workspace_path, media_id, chain)?;
    let mut png = std::io::Cursor::new(Vec::new());
    affected
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(format!(
        "data:image/png;base64,{}",
        crate::workspace::base64(png.get_ref())
    ))
}

pub(crate) fn effect_preview_image(
    workspace_path: &Path,
    media_id: &str,
    chain: &Value,
) -> Result<image::RgbaImage, String> {
    let effects = crate::native_media_effects::resolve_chain(chain)?;
    let workspace = crate::workspace::read_workspace(workspace_path)?;
    let item = workspace["media"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["id"].as_str() == Some(media_id))
        })
        .ok_or("Media asset was not found.")?;
    let image = source_image(workspace_path, item)?;
    let still = image.thumbnail(480, 480).to_rgba8();
    let affected = crate::native_media_effects::apply_chain(&still, &effects, 0.0);
    Ok(affected)
}

pub(crate) fn thumbnail_image(
    workspace_path: &Path,
    item: &Value,
) -> Result<image::RgbaImage, String> {
    if !matches!(item["type"].as_str(), Some("image" | "video")) {
        return Err("Thumbnails require an image or video asset.".into());
    }
    let image = if let Some(file) = item["thumbnailFile"]
        .as_str()
        .filter(|_| item["thumbnailMissing"] != true)
    {
        let folder = workspace_path
            .parent()
            .ok_or("Workspace folder is required.")?;
        let path = asset_path(
            workspace_path,
            &json!({"sourcePath":folder.join("Media").join(file),"assetFile":file}),
        )?;
        crate::native_render::NativeRenderRuntime::new_static_preview()
            .image_frame(&path)?
            .image
            .as_ref()
            .clone()
    } else {
        source_image(workspace_path, item)?
    };
    // The integer thumbnail upsampler can truncate fully opaque alpha to 254.
    Ok(image
        .resize(480, 480, image::imageops::FilterType::Triangle)
        .to_rgba8())
}

fn source_image(workspace_path: &Path, item: &Value) -> Result<image::DynamicImage, String> {
    let path = asset_path(workspace_path, item)?;
    Ok(match item["type"].as_str() {
        Some("image") => crate::native_render::NativeRenderRuntime::new_static_preview()
            .image_frame(&path)?
            .image
            .as_ref()
            .clone(),
        Some("video") => {
            let seconds = item["thumbnailTimeSeconds"].as_f64().unwrap_or(0.0);
            image::load_from_memory(&extract(&path, seconds)?).map_err(|error| error.to_string())?
        }
        _ => return Err("Effect previews require an image or video asset.".into()),
    })
}

/// The asset's canonical source path, kept inside workspace media storage.
pub(crate) fn asset_path(workspace_path: &Path, item: &Value) -> Result<PathBuf, String> {
    let path = PathBuf::from(
        item["sourcePath"]
            .as_str()
            .ok_or("Media source path is required.")?,
    );
    let root = workspace_path
        .parent()
        .ok_or("Workspace folder is required.")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let path = path
        .canonicalize()
        .map_err(|e| format!("Media source is unavailable: {e}"))?;
    if item["assetFile"].as_str().is_some() && !path.starts_with(root.join("Media")) {
        return Err("Media source is outside workspace media storage.".into());
    }
    Ok(path)
}

pub fn extract(source: &Path, seconds: f64) -> Result<Vec<u8>, String> {
    extract_frame(
        source,
        seconds,
        Some("scale=640:360:force_original_aspect_ratio=decrease"),
    )
}

/// One decoded video frame as PNG, optionally through an FFmpeg video filter.
pub fn extract_frame(source: &Path, seconds: f64, filter: Option<&str>) -> Result<Vec<u8>, String> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err("Thumbnail time must be a finite non-negative number of seconds.".into());
    }
    let metadata = crate::media_metadata::probe(&json!(source))?;
    let duration = metadata["durationSeconds"]
        .as_f64()
        .ok_or("Video duration could not be determined.")?;
    if seconds >= duration {
        return Err("Thumbnail time must be before the end of the video.".into());
    }
    let mut command = Command::new(ffmpeg_sidecar::paths::ffmpeg_path());
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-protocol_whitelist",
            "file",
            "-ss",
            &seconds.to_string(),
            "-i",
        ])
        .arg(source)
        .args(["-map", "0:v:0", "-frames:v", "1"]);
    if let Some(filter) = filter {
        command.args(["-vf", filter]);
    }
    command.args(["-f", "image2pipe", "-vcodec", "png", "pipe:1"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .output()
        .map_err(|e| format!("Video thumbnail extraction could not start: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Video thumbnail extraction failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let image = image::load_from_memory(&output.stdout)
        .map_err(|e| format!("Video thumbnail extraction returned no usable image: {e}"))?;
    if image.width() == 0 || image.height() == 0 {
        return Err("Video thumbnail extraction returned an empty image.".into());
    }
    Ok(output.stdout)
}

pub fn store(
    workspace_path: &Path,
    media_id: &str,
    expected_source: &Path,
    frame: Option<(&[u8], f64)>,
) -> Result<Value, String> {
    if source(workspace_path, media_id)? != expected_source {
        return Err("Video source changed during thumbnail extraction.".into());
    }
    let mut workspace = crate::workspace::read_workspace(workspace_path)?;
    let media = workspace["media"]
        .as_array_mut()
        .ok_or("Workspace media must be an array.")?;
    let item = media
        .iter_mut()
        .find(|item| item["id"].as_str() == Some(media_id))
        .ok_or("Video asset was not found.")?;
    let object = item
        .as_object_mut()
        .ok_or("Media asset must be an object.")?;
    let previous = object
        .get("thumbnailFile")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let root = workspace_path
        .parent()
        .ok_or("Workspace folder is required.")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let directory = root.join("Media/Thumbnails");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    if !directory.starts_with(root.join("Media")) {
        return Err("Thumbnail storage resolves outside workspace media storage.".into());
    }
    let mut created = None;
    if let Some((bytes, seconds)) = frame {
        let name = format!("{}.png", uuid::Uuid::new_v4());
        let path = directory.join(&name);
        fs::write(&path, bytes).map_err(|e| format!("Video thumbnail could not be saved: {e}"))?;
        object.insert("thumbnailFile".into(), json!(format!("Thumbnails/{name}")));
        object.insert("thumbnailTimeSeconds".into(), json!(seconds));
        created = Some(path);
    } else {
        object.remove("thumbnailFile");
        object.remove("thumbnailTimeSeconds");
    }
    object.remove("thumbnailUrl");
    let mut result = crate::workspace::save_workspace(workspace_path, &json!({"media": media}));
    if result.is_err() {
        if let Some(path) = created {
            let _ = fs::remove_file(path);
        }
    }
    if let Ok(saved) = &mut result {
        if let Some(previous) = previous {
            let still_used = saved["media"].as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item["thumbnailFile"].as_str() == Some(previous.as_str()))
            });
            if !still_used {
                if let Some(name) = previous
                    .strip_prefix("Thumbnails/")
                    .filter(|name| !name.contains(['/', '\\']) && name.ends_with(".png"))
                {
                    let old = directory.join(name);
                    if old.is_file() {
                        if let Err(error) = fs::remove_file(old) {
                            saved["thumbnailCleanupWarning"] = json!(format!("Thumbnail was saved, but the previous thumbnail could not be removed: {error}"));
                        }
                    }
                }
            }
        }
    }
    result
}
