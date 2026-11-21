use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

fn candidate(workspace: &Value, requested: &Value) -> Result<Value, String> {
    let id = requested["id"].as_str().ok_or("Media id is required.")?;
    if let Some(item) = workspace["media"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"].as_str() == Some(id)))
    {
        if item["sourcePath"] != requested["sourcePath"] {
            return Err("Media source changed while choosing its replacement.".into());
        }
        return Ok(item.clone());
    }
    let library_id = requested["libraryId"]
        .as_str()
        .ok_or("Missing media no longer belongs to this workspace.")?;
    let library = workspace["mediaLibraries"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["id"].as_str() == Some(library_id) && item["type"] == "smart")
        })
        .ok_or("Missing media library no longer belongs to this workspace.")?;
    let source = Path::new(
        requested["sourcePath"]
            .as_str()
            .ok_or("Media source path is required.")?,
    );
    let root = Path::new(
        library["sourcePath"]
            .as_str()
            .ok_or("Smart media folder is required.")?,
    );
    if !source.starts_with(root)
        || source
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("Missing derived source is outside its smart media folder.".into());
    }
    let expected_id = format!(
        "media-{}-{}",
        crate::workspace::safe_name(library_id, "library"),
        crate::workspace::safe_name(&source.to_string_lossy(), "asset")
    );
    if id != expected_id {
        return Err(
            "Missing derived media identity does not match its smart-folder source.".into(),
        );
    }
    Ok(requested.clone())
}

pub fn missing(workspace_path: &Path, known_media: &Value) -> Result<Value, String> {
    let workspace = crate::workspace::read_workspace(workspace_path)?;
    let mut items = workspace["media"]
        .as_array()
        .ok_or("Workspace media must be an array.")?
        .clone();
    for known in known_media
        .as_array()
        .ok_or("Known media must be an array.")?
    {
        if !items.iter().any(|item| item["id"] == known["id"]) {
            items.push(candidate(&workspace, known)?);
        }
    }
    let root = workspace_path
        .parent()
        .ok_or("Workspace folder is required.")?;
    let mut result = Vec::new();
    for item in items {
        let source = item["sourcePath"]
            .as_str()
            .ok_or("Media source path is required.")?;
        if !Path::new(source).is_file() {
            result.push(json!({"media": item, "kind": "source", "path": source}));
        } else if let Some(file) = item["thumbnailFile"].as_str() {
            let path = root.join("Media").join(file);
            if !path.is_file() {
                result.push(json!({"media": item, "kind": "thumbnail", "path": path}));
            }
        }
    }
    Ok(json!(result))
}

pub(crate) fn validate_replacement(path: &Path, media_type: &str) -> Result<(), String> {
    if !path.is_file() {
        return Err("Replacement media must be an existing file.".into());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let kind = match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" => "image",
        "mp4" | "mov" | "m4v" | "avi" | "mkv" | "webm" => "video",
        "mp3" | "wav" | "m4a" | "aac" | "flac" | "aiff" => "audio",
        _ => return Err("Replacement file format is not supported.".into()),
    };
    if kind != media_type {
        return Err(format!(
            "Replacement must be {media_type} media; selected file is {kind}."
        ));
    }
    if kind == "image" {
        crate::native_render::decode_image(path)
            .map_err(|error| format!("Replacement image cannot be decoded: {error}"))?;
    } else {
        let metadata = crate::media_metadata::probe(&json!(path))?;
        if metadata["codec"].as_str().is_none()
            || metadata["durationSeconds"].as_f64().is_none()
            || (kind == "video" && metadata["width"].as_f64().is_none())
            || (kind == "audio" && metadata["width"].as_f64().is_some())
        {
            return Err("Replacement media has no usable compatible stream or duration.".into());
        }
        if kind == "video" {
            crate::media_thumbnail::extract(path, 0.0)
                .map_err(|error| format!("Replacement video cannot be decoded: {error}"))?;
        } else {
            let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
            let mut decoder = rodio::Decoder::try_from(file)
                .map_err(|error| format!("Replacement audio cannot be decoded: {error}"))?;
            if decoder.next().is_none() {
                return Err("Replacement audio contains no usable samples.".into());
            }
        }
    }
    Ok(())
}

pub fn relink(
    workspace_path: &Path,
    request: &Value,
    replacement: &Path,
    manage_media: bool,
) -> Result<Value, String> {
    let mut workspace = crate::workspace::read_workspace(workspace_path)?;
    let original = candidate(&workspace, &request["media"])?;
    let id = original["id"].as_str().ok_or("Media id is required.")?;
    let kind = request["kind"]
        .as_str()
        .ok_or("Missing asset kind is required.")?;
    if kind == "thumbnail" {
        let root = workspace_path
            .parent()
            .ok_or("Workspace folder is required.")?;
        let file = original["thumbnailFile"]
            .as_str()
            .ok_or("Thumbnail reference is required.")?;
        if root.join("Media").join(file).is_file() {
            return Err("Thumbnail is no longer missing; refresh the overview.".into());
        }
        validate_replacement(replacement, "image")?;
        let image = crate::native_render::decode_image(replacement)?;
        let mut png = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        let source = crate::media_thumbnail::source(workspace_path, id)?;
        let seconds = original["thumbnailTimeSeconds"]
            .as_f64()
            .ok_or("Thumbnail timestamp is required.")?;
        return crate::media_thumbnail::store(
            workspace_path,
            id,
            &source,
            Some((&png.into_inner(), seconds)),
        );
    }
    if kind != "source" {
        return Err("Unknown missing asset kind.".into());
    }
    let source = original["sourcePath"]
        .as_str()
        .ok_or("Media source path is required.")?;
    if Path::new(source).is_file() {
        return Err("Media is no longer missing; refresh the overview.".into());
    }
    validate_replacement(
        replacement,
        original["type"].as_str().ok_or("Media type is required.")?,
    )?;
    if original["type"] == "video" || original["type"] == "audio" {
        let metadata = crate::media_metadata::probe(&json!(replacement))?;
        let duration = metadata["durationSeconds"]
            .as_f64()
            .ok_or("Replacement media duration is required.")?;
        if original["markers"].as_array().is_some_and(|markers| {
            markers.iter().any(|marker| {
                marker["timeSeconds"]
                    .as_f64()
                    .is_some_and(|time| time >= duration)
            })
        }) {
            return Err("Replacement media ends before an existing playback marker. Choose a longer file or edit the markers explicitly.".into());
        }
    }
    let mut changed = original.clone();
    let object = changed
        .as_object_mut()
        .ok_or("Media asset must be an object.")?;
    for key in [
        "assetFile",
        "sourceUrl",
        "thumbnailFile",
        "thumbnailTimeSeconds",
        "thumbnailUrl",
        "missing",
        "thumbnailMissing",
        "duration",
    ] {
        object.remove(key);
    }
    object.insert("sourcePath".into(), json!(replacement));
    if manage_media {
        object.remove("externalSource");
    } else {
        object.insert("externalSource".into(), json!(true));
    }
    // A relinked smart-folder file leaves its folder and becomes managed media.
    if smart_library(&workspace, &original).is_some() {
        object.remove("libraryId");
    }
    let media = workspace["media"]
        .as_array_mut()
        .ok_or("Workspace media must be an array.")?;
    if let Some(existing) = media
        .iter_mut()
        .find(|item| item["id"].as_str() == Some(id))
    {
        *existing = changed;
    } else {
        media.push(changed);
    }
    crate::workspace::save_workspace(workspace_path, &json!({"media": media}))
}

const SEARCH_DEPTH: usize = 8;

fn search_name(name: &std::ffi::OsStr) -> String {
    name.to_string_lossy().to_lowercase()
}

/// Walks every search path once, breadth first, collecting files whose (case-insensitive) name is
/// wanted. Folders reachable from several search paths are visited once; symlinks are skipped.
fn find_named_files(roots: &[PathBuf], names: &HashSet<String>) -> HashMap<String, Vec<PathBuf>> {
    let mut found: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut visited = HashSet::new();
    let mut queue: VecDeque<(PathBuf, usize)> =
        roots.iter().map(|root| (root.clone(), 0)).collect();
    while let Some((folder, depth)) = queue.pop_front() {
        if found.len() == names.len() {
            break;
        }
        let Ok(canonical) = folder.canonicalize() else {
            continue;
        };
        if !visited.insert(canonical) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = search_name(&entry.file_name());
            if kind.is_file() && names.contains(&name) {
                found.entry(name).or_default().push(entry.path());
            } else if kind.is_dir() && depth < SEARCH_DEPTH {
                queue.push_back((entry.path(), depth + 1));
            }
        }
    }
    found
}

/// Pairs each missing source from a `missing` overview with the first usable same-named file
/// under the search paths. Touches no workspace files, so it can run without workspace I/O.
pub fn search_missing_sources(
    overview: &Value,
    search_paths: &[PathBuf],
) -> Result<Vec<(Value, PathBuf)>, String> {
    let mut assets = Vec::new();
    for asset in overview
        .as_array()
        .ok_or("Missing media must be an array.")?
    {
        if asset["kind"] != "source" {
            continue;
        }
        let path = asset["path"]
            .as_str()
            .ok_or("Missing media path is required.")?;
        if let Some(name) = Path::new(path).file_name() {
            assets.push((asset.clone(), search_name(name)));
        }
    }
    if assets.is_empty() {
        return Ok(Vec::new());
    }
    let names = assets.iter().map(|(_, name)| name.clone()).collect();
    let found = find_named_files(search_paths, &names);
    let mut matches = Vec::new();
    for (asset, name) in assets {
        let media_type = asset["media"]["type"]
            .as_str()
            .ok_or("Media type is required.")?;
        if let Some(path) = found
            .get(&name)
            .into_iter()
            .flatten()
            .find(|path| validate_replacement(path, media_type).is_ok())
        {
            matches.push((asset, path.clone()));
        }
    }
    Ok(matches)
}

/// Relinks searched matches; returns the saved workspace (if any changed) and the relink count.
pub fn relink_matches(
    workspace_path: &Path,
    matches: &[(Value, PathBuf)],
    manage_media: bool,
) -> Result<(Option<Value>, usize), String> {
    let mut saved = None;
    for (asset, replacement) in matches {
        saved = Some(relink(workspace_path, asset, replacement, manage_media)?);
    }
    Ok((saved, matches.len()))
}

fn smart_library<'a>(workspace: &'a Value, media: &Value) -> Option<&'a Value> {
    let library_id = media["libraryId"].as_str()?;
    workspace["mediaLibraries"]
        .as_array()?
        .iter()
        .find(|library| library["id"].as_str() == Some(library_id) && library["type"] == "smart")
}

/// Removes a missing smart-folder file from its library's known files, so it is no longer
/// listed as missing. Present files and managed media cannot be forgotten.
pub fn forget(workspace_path: &Path, request: &Value) -> Result<Value, String> {
    let mut workspace = crate::workspace::read_workspace(workspace_path)?;
    let original = candidate(&workspace, &request["media"])?;
    if request["kind"] != "source" || smart_library(&workspace, &original).is_none() {
        return Err("Only a missing smart-folder file can be forgotten.".into());
    }
    if Path::new(
        original["sourcePath"]
            .as_str()
            .ok_or("Media source path is required.")?,
    )
    .is_file()
    {
        return Err("Media is no longer missing; refresh the overview.".into());
    }
    let media = workspace["media"]
        .as_array_mut()
        .ok_or("Workspace media must be an array.")?;
    media.retain(|item| item["id"] != original["id"]);
    crate::workspace::save_workspace(workspace_path, &json!({"media": media}))
}
