//! Import of ProPresenter `.proBundle` and `.proPlaylist` archives into an
//! existing Orbit workspace. Both are ZIP files: presentation documents
//! (`*.pro`), their media and, for playlists, a `data` PlaylistDocument.

use super::{
    content_themes, propresenter_proto, read_workspace, safe_name, save_workspace, unique_id,
};
use crate::workspace_import::{place, same_title, Duplicate};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

const ASSET_KEYS: [&str; 10] = [
    "sourcePath",
    "sourceUrl",
    "mediaFillSourcePath",
    "mediaFillUrl",
    "mediaUrl",
    "mediaSourcePath",
    "mediaPath",
    "backgroundMediaPath",
    "backgroundMediaUrl",
    "thumbnailUrl",
];

struct Archive {
    zip: zip::ZipArchive<fs::File>,
    documents: Vec<(String, Vec<u8>)>,
    playlist: Option<Vec<u8>>,
    media: Vec<(usize, String)>,
}

fn read_entry(entry: &mut zip::read::ZipFile<'_, fs::File>, name: &str) -> Result<Vec<u8>, String> {
    if entry.size() > MAX_DOCUMENT_BYTES {
        return Err(format!(
            "ProPresenter archive document is too large: {name}"
        ));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes).map_err(|error| {
        format!("ProPresenter archive entry could not be read: {name}: {error}")
    })?;
    Ok(bytes)
}

fn open(path: &Path) -> Result<Archive, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("ProPresenter archive could not be opened: {error}"))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| format!("ProPresenter archive is not a valid bundle: {error}"))?;
    let mut documents = Vec::new();
    let mut playlist = None;
    let mut media = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .enclosed_name()
            .ok_or_else(|| {
                format!(
                    "ProPresenter archive entry escapes the archive: {}",
                    entry.name()
                )
            })?
            .to_string_lossy()
            .replace('\\', "/");
        if name == "data" {
            playlist = Some(read_entry(&mut entry, &name)?);
        } else if name.to_ascii_lowercase().ends_with(".pro") {
            let bytes = read_entry(&mut entry, &name)?;
            documents.push((name, bytes));
        } else {
            media.push((index, name));
        }
    }
    if documents.is_empty() {
        return Err("ProPresenter archive contains no presentation documents.".into());
    }
    Ok(Archive {
        zip,
        documents,
        playlist,
        media,
    })
}

fn path_components(value: &str) -> Vec<String> {
    let path = url::Url::parse(value)
        .ok()
        .filter(|url| url.scheme() == "file")
        .and_then(|url| url.to_file_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| value.to_string());
    path.replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty() && !part.ends_with(':'))
        .map(str::to_lowercase)
        .collect()
}

/// Bundles store media under their own folder layout, so a reference is located
/// by the longest unique trailing path match inside the archive.
fn locate(reference: &str, media: &[(usize, String)]) -> Result<usize, String> {
    let wanted = path_components(reference);
    let scored = media
        .iter()
        .map(|(index, name)| {
            let parts = path_components(name);
            let shared = parts
                .iter()
                .rev()
                .zip(wanted.iter().rev())
                .take_while(|(left, right)| left == right)
                .count();
            (*index, shared)
        })
        .filter(|(_, shared)| *shared > 0)
        .collect::<Vec<_>>();
    let best = scored.iter().map(|(_, shared)| *shared).max().unwrap_or(0);
    let matches = scored
        .iter()
        .filter(|(_, shared)| *shared == best)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [(index, _)] => Ok(*index),
        [] => Err(format!(
            "ProPresenter archive is missing media: {reference}"
        )),
        _ => Err(format!(
            "ProPresenter archive media is ambiguous: {reference}"
        )),
    }
}

struct Extractor<'a> {
    archive: &'a mut Archive,
    root: PathBuf,
    extracted: HashMap<usize, PathBuf>,
}

impl Extractor<'_> {
    fn extract(&mut self, reference: &str) -> Result<PathBuf, String> {
        let index = locate(reference, &self.archive.media)?;
        if let Some(path) = self.extracted.get(&index) {
            return Ok(path.clone());
        }
        let name = &self
            .archive
            .media
            .iter()
            .find(|(entry, _)| *entry == index)
            .expect("located entry")
            .1;
        let extension = Path::new(name)
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| format!("ProPresenter archive media has no extension: {name}"))?;
        let target = self
            .root
            .join(format!("{}.{extension}", self.extracted.len()));
        let mut entry = self
            .archive
            .zip
            .by_index(index)
            .map_err(|error| error.to_string())?;
        let mut output = fs::File::create(&target).map_err(|error| error.to_string())?;
        std::io::copy(&mut entry, &mut output).map_err(|error| {
            format!("ProPresenter archive media could not be extracted: {name}: {error}")
        })?;
        crate::portable_bundle::validate_asset(&target)?;
        self.extracted.insert(index, target.clone());
        Ok(target)
    }

    fn rewrite(&mut self, value: &mut Value) -> Result<(), String> {
        match value {
            Value::Object(object) => {
                object.remove("importSourcePath");
                for (key, child) in object.iter_mut() {
                    if ASSET_KEYS.contains(&key.as_str()) {
                        if let Some(reference) = child
                            .as_str()
                            .filter(|value| !value.is_empty() && !value.starts_with("data:"))
                        {
                            let path = self.extract(reference)?;
                            *child = if key.ends_with("Url") {
                                json!(url::Url::from_file_path(&path)
                                    .map_err(|_| "Extracted media path is not absolute.")?
                                    .to_string())
                            } else {
                                json!(path)
                            };
                        }
                    } else {
                        self.rewrite(child)?;
                    }
                }
            }
            Value::Array(values) => {
                for child in values {
                    self.rewrite(child)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn media_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "mov" | "m4v" | "avi" | "mkv" | "webm" => "video",
        "mp3" | "wav" | "m4a" | "aac" | "flac" | "aiff" => "audio",
        _ => "image",
    }
}

/// Register one workspace media item per media action target so imported
/// actions keep resolving through their original ProPresenter identity.
fn media_items(value: &Value, workspace: &Value, items: &mut Vec<Value>) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("media") {
                let target = object
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(source) = object
                    .get("details")
                    .and_then(|details| details.get("sourceUrl"))
                    .and_then(Value::as_str)
                    .filter(|_| !target.is_empty())
                {
                    if workspace["media"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|item| {
                            item["sourceId"].as_str() == Some(target)
                                || item["id"].as_str() == Some(target)
                        })
                    {
                        return Err(format!(
                            "Workspace already contains media imported as {target}. Import into a separate workspace or remove the conflicting media explicitly."
                        ));
                    }
                    if !items
                        .iter()
                        .any(|item| item["sourceId"].as_str() == Some(target))
                    {
                        let path = url::Url::parse(source)
                            .ok()
                            .and_then(|url| url.to_file_path().ok())
                            .ok_or("Extracted media URL is invalid.")?;
                        items.push(json!({
                            "id": format!("media-{}", uuid::Uuid::new_v4()),
                            "title": path.file_stem().and_then(|value| value.to_str()).unwrap_or("Media"),
                            "sourcePath": path,
                            "sourceUrl": source,
                            "sourceId": target,
                            "type": media_type(&path),
                        }));
                    }
                }
            }
            for child in object.values() {
                media_items(child, workspace, items)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                media_items(child, workspace, items)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn file_name(path: &str) -> String {
    path.replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

fn playlists(
    bytes: &[u8],
    documents: &[(String, String)],
    workspace: &Value,
) -> Result<Vec<Value>, String> {
    let decoded = propresenter_proto::decode_playlists(bytes);
    if decoded.is_empty() {
        return Err("ProPresenter playlist archive has no supported playlist document.".into());
    }
    let mut ids = workspace["playlist"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|playlist| playlist["id"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    let mut imported = Vec::new();
    for mut playlist in decoded {
        let mut item_ids = Vec::new();
        for entry in playlist["items"].as_array_mut().into_iter().flatten() {
            let Some(path) = entry
                .get("documentPath")
                .and_then(Value::as_str)
                .map(file_name)
            else {
                continue;
            };
            let matches = documents
                .iter()
                .filter(|(name, _)| file_name(name) == path)
                .collect::<Vec<_>>();
            let item_id = match (matches.as_slice(), entry["type"].as_str()) {
                ([(_, id)], _) => Some(id.clone()),
                ([], Some("placeholder")) => None,
                _ => {
                    return Err(format!(
                        "ProPresenter playlist document is missing or ambiguous: {path}"
                    ))
                }
            };
            let object = entry
                .as_object_mut()
                .ok_or("Playlist entry must be an object.")?;
            object.remove("documentPath");
            if let Some(item_id) = item_id {
                if entry["type"] == "presentation" {
                    item_ids.push(item_id.clone());
                    entry["itemId"] = json!(item_id);
                } else {
                    entry["linkedItemId"] = json!(item_id);
                }
            }
        }
        let base = playlist["id"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| "playlist".into());
        let id = unique_id(&base, ids.iter().map(String::as_str));
        ids.push(id.clone());
        playlist["id"] = json!(id);
        playlist["itemIds"] = json!(item_ids);
        imported.push(playlist);
    }
    Ok(imported)
}

fn title(item: &Value, name: &str) -> String {
    item["title"]
        .as_str()
        .filter(|title| !title.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            Path::new(name)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("Presentation")
                .to_string()
        })
}

/// Titles the archive's presentations will be imported under.
pub fn archive_titles(archive_path: &Path) -> Result<Vec<String>, String> {
    open(archive_path)?
        .documents
        .iter()
        .map(|(name, bytes)| {
            propresenter_proto::decode_presentation(bytes, Path::new(name))
                .map(|item| title(&item, name))
                .ok_or_else(|| format!("Not a supported ProPresenter presentation: {name}"))
        })
        .collect()
}

pub fn import_archive(
    workspace_path: &Path,
    archive_path: &Path,
    library_id: &str,
    duplicate: Duplicate,
) -> Result<Value, String> {
    let mut archive = open(archive_path)?;
    let workspace = read_workspace(workspace_path)?;
    let library_index = workspace["libraries"]
        .as_array()
        .and_then(|libraries| {
            libraries
                .iter()
                .position(|library| library["id"].as_str() == Some(library_id))
        })
        .ok_or("Selected destination library is missing.")?;
    let mut taken = workspace["library"]
        .as_array()
        .ok_or("Library is invalid.")?
        .iter()
        .filter_map(|item| item["id"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    let mut items = Vec::new();
    let mut documents = Vec::new();
    for (name, bytes) in &archive.documents {
        let mut item = propresenter_proto::decode_presentation(bytes, Path::new(name))
            .ok_or_else(|| format!("Not a supported ProPresenter presentation: {name}"))?;
        let title = title(&item, name);
        let id = match (same_title(&workspace, library_id, &title), duplicate) {
            (Some(existing), Duplicate::UseExisting) => {
                documents.push((name.clone(), existing));
                continue;
            }
            (Some(existing), Duplicate::Replace) => existing,
            _ => unique_id(
                &safe_name(&title, "presentation"),
                taken.iter().map(String::as_str),
            ),
        };
        taken.push(id.clone());
        let slides = item["slides"]
            .as_array_mut()
            .filter(|slides| !slides.is_empty())
            .ok_or_else(|| format!("ProPresenter presentation has no slides: {name}"))?;
        for (index, slide) in slides.iter_mut().enumerate() {
            slide["id"] = json!(format!("slide-{}", index + 1));
        }
        item["id"] = json!(id);
        item["title"] = json!(title);
        item["libraryId"] = json!(library_id);
        documents.push((name.clone(), id));
        items.push(item);
    }
    if items.is_empty() && archive.playlist.is_none() {
        return Ok(workspace);
    }
    let mut themes = workspace["themes"]
        .as_array()
        .cloned()
        .ok_or("Themes are invalid.")?;
    let existing_themes = themes.len();
    content_themes::import_layouts(&mut items, &mut themes)?;
    let mut new_themes = themes.split_off(existing_themes);
    let playlist = archive
        .playlist
        .take()
        .map(|bytes| playlists(&bytes, &documents, &workspace))
        .transpose()?
        .unwrap_or_default();

    let folder = workspace_path
        .parent()
        .ok_or("Workspace folder is required.")?;
    let container = folder.join("Media").join("ProPresenter Bundles");
    fs::create_dir_all(&container).map_err(|error| error.to_string())?;
    let root = container.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    let result = (|| {
        let mut extractor = Extractor {
            archive: &mut archive,
            root: root.clone(),
            extracted: HashMap::new(),
        };
        for value in items.iter_mut().chain(new_themes.iter_mut()) {
            extractor.rewrite(value)?;
        }
        let mut media = Vec::new();
        for item in &items {
            media_items(item, &workspace, &mut media)?;
        }
        let mut patch = workspace.clone();
        for item in items {
            place(&mut patch, library_index, item)?;
        }
        patch["media"]
            .as_array_mut()
            .ok_or("Media is invalid.")?
            .extend(media);
        patch["themes"]
            .as_array_mut()
            .ok_or("Themes are invalid.")?
            .extend(new_themes);
        patch["playlist"]
            .as_array_mut()
            .ok_or("Playlists are invalid.")?
            .extend(playlist);
        save_workspace(workspace_path, &patch)
    })();
    if result.is_err() {
        fs::remove_dir_all(&root).map_err(|error| {
            format!("ProPresenter archive import failed and media cleanup failed: {error}")
        })?;
    }
    result
}
