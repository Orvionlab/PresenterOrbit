use crate::workspace::{content_themes, propresenter_proto};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct NewDirectory(std::path::PathBuf, bool);

impl NewDirectory {
    pub fn create(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("Export destination must be an absolute path.".into());
        }
        std::fs::create_dir(path)
            .map_err(|error| format!("Cannot reserve new export destination: {error}"))?;
        Ok(Self(path.into(), false))
    }
    pub fn commit(mut self) {
        self.1 = true;
    }
}

impl Drop for NewDirectory {
    fn drop(&mut self) {
        if !self.1 {
            if let Err(error) = std::fs::remove_dir_all(&self.0) {
                eprintln!("Failed to remove incomplete ProPresenter export: {error}");
            }
        }
    }
}

#[derive(Default)]
pub struct Names(HashSet<String>);

impl Names {
    pub fn reserve(&mut self, title: &str, kind: &str) -> String {
        let base = crate::workspace::safe_name(title, kind);
        self.reserve_base(base)
    }
    pub fn folder(&mut self, title: &str, kind: &str) -> String {
        let base = title
            .chars()
            .map(|c| {
                if c.is_control() || "<>:\"/\\|?*".contains(c) {
                    '-'
                } else {
                    c
                }
            })
            .collect::<String>();
        let base = base.trim_end_matches(['.', ' ']);
        let base = if base.is_empty() { kind } else { base };
        let stem = base.split('.').next().unwrap().to_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        self.reserve_base(if reserved {
            format!("_{base}")
        } else {
            base.to_string()
        })
    }
    /// ProPresenter lists effect presets by the file name up to its first dot and
    /// replaces dots with underscores in the preset files it names itself.
    pub fn preset(&mut self, title: &str, kind: &str) -> String {
        self.folder(&title.replace('.', "_"), kind)
    }
    fn reserve_base(&mut self, base: String) -> String {
        for index in 1_u64.. {
            let name = if index == 1 {
                base.clone()
            } else {
                format!("{base}-{index}")
            };
            if self.0.insert(name.to_uppercase()) {
                return name;
            }
        }
        unreachable!()
    }
}

pub fn file_url(path: &Path) -> Result<String, String> {
    url::Url::from_file_path(path)
        .map(String::from)
        .map_err(|_| "ProPresenter export requires an absolute destination path.".into())
}

/// The image and video files that exported layers fill with, copied once each into the show's
/// `Media` folder. `relative` maps the file URL of each copy to its path inside the show.
#[derive(Default)]
pub struct MediaFiles {
    names: Names,
    copied: HashMap<PathBuf, String>,
    relative: BTreeMap<String, String>,
}

impl MediaFiles {
    fn copy(&mut self, reference: &str, destination: &Path) -> Result<String, String> {
        let source = if reference.starts_with("file://") {
            url::Url::parse(reference)
                .map_err(|error| error.to_string())?
                .to_file_path()
                .map_err(|_| format!("Invalid local media URL: {reference}"))?
        } else if reference.contains("://") {
            return Err(format!("Remote media cannot be exported: {reference}"));
        } else {
            PathBuf::from(reference)
        };
        if let Some(url) = self.copied.get(&source) {
            return Ok(url.clone());
        }
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .ok_or_else(|| format!("Media file has no extension: {}", source.display()))?;
        let stem = source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        let name = format!("{}.{extension}", self.names.folder(stem, "Media"));
        let folder = destination.join("Media");
        std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        let target = folder.join(&name);
        std::fs::copy(&source, &target).map_err(|error| {
            format!(
                "Media file {} cannot be exported: {error}",
                source.display()
            )
        })?;
        let url = file_url(&target)?;
        self.relative.insert(url.clone(), format!("Media/{name}"));
        self.copied.insert(source, url.clone());
        Ok(url)
    }

    pub fn prepare_media_bins(
        &mut self,
        workspace: &Value,
        destination: &Path,
    ) -> Result<(Value, BTreeMap<String, String>), String> {
        let mut exported = workspace.clone();
        let mut relative = self.relative.clone();
        for item in exported["media"]
            .as_array_mut()
            .ok_or("Workspace media must be an array.")?
        {
            let source = item["sourceUrl"]
                .as_str()
                .ok_or("Media source URL is required for export.")?
                .to_string();
            let copied = self.copy(&source, destination)?;
            let path = self
                .relative
                .get(&copied)
                .ok_or("Copied media has no show-relative path.")?
                .clone();
            for key in ["sourceUrl", "sourcePath", "assetFile"] {
                if let Some(reference) = item[key].as_str() {
                    if relative
                        .get(reference)
                        .is_some_and(|existing| existing != &path)
                    {
                        return Err(format!("Ambiguous media export reference: {reference}"));
                    }
                    relative.insert(reference.to_string(), path.clone());
                }
            }
            relative.insert(copied.clone(), path);
            item["sourceUrl"] = json!(copied);
        }
        Ok((exported, relative))
    }

    /// The ProPresenter document of a workspace item with its themes composed into its slides and
    /// the media of its layers copied into the show.
    pub fn encode(
        &mut self,
        workspace: &Map<String, Value>,
        item: &Value,
        destination: &Path,
    ) -> Result<Vec<u8>, String> {
        let mut item = presentation(content_themes::compose_item(workspace, item));
        self.copy_slide_media(workspace, &mut item, destination)?;
        Ok(propresenter_proto::encode_presentation_with_media(
            &item,
            &self.relative,
        ))
    }

    /// The ProPresenter template of a theme with the media of its layers copied into the show.
    pub fn encode_theme(
        &mut self,
        workspace: &Map<String, Value>,
        theme: &Value,
        destination: &Path,
    ) -> Result<Vec<u8>, String> {
        let mut theme = theme.clone();
        self.copy_slide_media(workspace, &mut theme, destination)?;
        Ok(propresenter_proto::encode_template_with_media(
            &theme,
            &self.relative,
        ))
    }

    /// Props and masks with the media of their slide's layers copied into the show, and the
    /// show-relative paths of every copy so far.
    pub fn copy_collection_media(
        &mut self,
        workspace: &Map<String, Value>,
        items: &[Value],
        destination: &Path,
    ) -> Result<(Vec<Value>, BTreeMap<String, String>), String> {
        let mut copied = Vec::with_capacity(items.len());
        for item in items {
            let mut slides = json!({"slides": [item.get("slide").unwrap_or(item)]});
            self.copy_slide_media(workspace, &mut slides, destination)?;
            let slide = slides["slides"][0].take();
            let mut item = item.clone();
            if item.get("slide").is_some() {
                item["slide"] = slide;
            } else {
                item = slide;
            }
            copied.push(item);
        }
        Ok((copied, self.relative.clone()))
    }

    fn copy_slide_media(
        &mut self,
        workspace: &Map<String, Value>,
        item: &mut Value,
        destination: &Path,
    ) -> Result<(), String> {
        for slide in item["slides"].as_array_mut().into_iter().flatten() {
            for action in slide["actions"].as_array_mut().into_iter().flatten() {
                if action["type"] == "media" {
                    if let Some(reference) = action["details"]["sourceUrl"]
                        .as_str()
                        .filter(|reference| !reference.trim().is_empty())
                    {
                        action["details"]["sourceUrl"] = json!(self.copy(reference, destination)?);
                    }
                }
            }
            if let Some(media) = slide["mediaBackgroundId"].as_str().and_then(|id| {
                workspace["media"]
                    .as_array()?
                    .iter()
                    .find(|media| media["id"] == id)
            }) {
                let mut media = media.clone();
                let source = media["sourceUrl"]
                    .as_str()
                    .ok_or("Slide media background has no source URL.")?
                    .to_string();
                media["sourceUrl"] = json!(self.copy(&source, destination)?);
                slide[propresenter_proto::cue_export::MEDIA_BACKGROUND] = media;
            }
            for layer in slide["layers"].as_array_mut().into_iter().flatten() {
                if layer["fillType"] == "media" {
                    if let Some(reference) = [
                        "sourcePath",
                        "mediaSourcePath",
                        "mediaPath",
                        "mediaUrl",
                        "sourceUrl",
                    ]
                    .into_iter()
                    .filter_map(|key| layer.get(key).and_then(Value::as_str))
                    .find(|reference| !reference.trim().is_empty())
                    {
                        layer["mediaUrl"] = json!(self.copy(reference, destination)?);
                    }
                }
                if layer["textStyle"]["textFillMode"] == "Media Fill" {
                    if let Some(reference) = layer["textStyle"]["mediaFillSourcePath"]
                        .as_str()
                        .filter(|reference| !reference.trim().is_empty())
                        .or_else(|| {
                            layer["textStyle"]["mediaFillUrl"]
                                .as_str()
                                .filter(|reference| !reference.trim().is_empty())
                        })
                    {
                        layer["textStyle"]["mediaFillUrl"] =
                            json!(self.copy(reference, destination)?);
                    }
                }
            }
        }
        Ok(())
    }
}

pub fn playlist_roots(playlists: &[Value]) -> Result<Vec<Value>, String> {
    fn parent(playlist: &Value) -> Result<&str, String> {
        match playlist.get("parentId") {
            None => Ok(""),
            Some(Value::String(id)) => Ok(id),
            _ => Err("Export playlist parent id must be a string.".into()),
        }
    }
    let mut ids = HashMap::new();
    for playlist in playlists {
        let id = playlist["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Export playlist id is required.")?;
        if ids.insert(id, playlist).is_some() {
            return Err(format!("Duplicate export playlist id: {id}"));
        }
    }
    for playlist in playlists {
        let mut visited = HashSet::new();
        let mut current = playlist;
        loop {
            if !visited.insert(current["id"].as_str().unwrap()) {
                return Err("Export playlist hierarchy contains a cycle.".into());
            }
            let parent_id = parent(current)?;
            if parent_id.is_empty() {
                break;
            }
            if visited.len() > 128 {
                return Err("Export playlist hierarchy exceeds 128 levels.".into());
            }
            current = ids
                .get(parent_id)
                .ok_or_else(|| format!("Export playlist parent is missing: {parent_id}"))?;
            if current["type"] != "folder" {
                return Err("Export playlist parent must be a folder.".into());
            }
        }
    }
    fn node(playlist: &Value, playlists: &[Value]) -> Value {
        let mut result = playlist.clone();
        if let Some(entries) = result["items"].as_array_mut() {
            for entry in entries {
                entry.as_object_mut().unwrap().remove("arrangementId");
            }
        }
        let children = playlists
            .iter()
            .filter(|child| child["parentId"] == playlist["id"])
            .map(|child| node(child, playlists))
            .collect::<Vec<_>>();
        result["children"] = json!(children);
        result
    }
    Ok(playlists
        .iter()
        .filter(|playlist| playlist["parentId"].as_str().unwrap_or("").is_empty())
        .map(|playlist| node(playlist, playlists))
        .collect())
}

pub fn validate_references(workspace: &Value) -> Result<(), String> {
    let items = workspace["library"]
        .as_array()
        .ok_or("Export presentations are required.")?;
    let mut ids = HashSet::new();
    for item in items {
        let id = item["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Export presentation id is required.")?;
        if !ids.insert(id) {
            return Err(format!("Duplicate export presentation id: {id}"));
        }
    }
    for library in workspace["libraries"]
        .as_array()
        .ok_or("Export libraries are required.")?
    {
        for id in library["itemIds"]
            .as_array()
            .ok_or("Export library item ids are required.")?
        {
            if !id.as_str().is_some_and(|id| ids.contains(id)) {
                return Err(format!(
                    "Export library references a missing presentation: {id}"
                ));
            }
        }
    }
    for playlist in workspace["playlist"]
        .as_array()
        .ok_or("Export playlists are required.")?
    {
        for entry in playlist["items"]
            .as_array()
            .ok_or("Export playlist entries are required.")?
        {
            let key = match entry["type"].as_str() {
                Some("presentation") => "itemId",
                Some("placeholder") if entry.get("linkedItemId").is_some() => "linkedItemId",
                Some("placeholder" | "header") => continue,
                _ => return Err("Export playlist entry type is unsupported.".into()),
            };
            if !entry[key].as_str().is_some_and(|id| ids.contains(id)) {
                return Err(format!(
                    "Export playlist references a missing presentation: {}",
                    entry[key]
                ));
            }
        }
    }
    Ok(())
}

pub fn presentation(mut item: Value) -> Value {
    let id = item["sourceId"]
        .as_str()
        .or_else(|| item["id"].as_str())
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .unwrap_or_else(uuid::Uuid::new_v4);
    item["sourceId"] = json!(id.to_string());
    if let Some(slides) = item["slides"].as_array_mut() {
        for slide in slides {
            let id = slide["sourceCueId"]
                .as_str()
                .or_else(|| slide["id"].as_str())
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                .unwrap_or_else(uuid::Uuid::new_v4);
            slide["sourceCueId"] = json!(id.to_string());
        }
    }
    item
}

pub fn theme(mut theme: Value) -> Value {
    if let Some(slides) = theme["slides"].as_array_mut() {
        for slide in slides {
            let id = slide["sourceSlideId"]
                .as_str()
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                .unwrap_or_else(uuid::Uuid::new_v4);
            slide["sourceSlideId"] = json!(id.to_string());
        }
    }
    theme
}
