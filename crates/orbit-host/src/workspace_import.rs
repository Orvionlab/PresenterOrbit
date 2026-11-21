use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Destination {
    folder: PathBuf,
    owned: bool,
    committed: bool,
}

impl Destination {
    pub fn reserve(manifest: &Path) -> Result<Self, String> {
        let folder = resolved_destination(
            manifest
                .parent()
                .ok_or("Import destination folder is required.")?,
        )?;
        let owned = !folder.exists();
        if owned {
            let parent = folder
                .parent()
                .ok_or("Import destination parent is required.")?;
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            fs::create_dir(&folder).map_err(|error| error.to_string())?;
        }
        Ok(Self {
            folder,
            owned,
            committed: false,
        })
    }
    pub fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for Destination {
    fn drop(&mut self) {
        if self.owned && !self.committed {
            if let Err(error) = fs::remove_dir_all(&self.folder) {
                eprintln!("Failed to remove incomplete ProPresenter import: {error}");
            }
        }
    }
}
fn resolved_destination(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Import destination must be an absolute path.".into());
    }
    let mut ancestor = path;
    let mut missing = Vec::new();
    while !ancestor.exists() {
        missing.push(
            ancestor
                .file_name()
                .ok_or("Invalid import destination.")?
                .to_owned(),
        );
        ancestor = ancestor
            .parent()
            .ok_or("Invalid import destination parent.")?;
    }
    let mut resolved = ancestor.canonicalize().map_err(|error| error.to_string())?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

pub fn validate_destination(source: &Path, manifest: &Path) -> Result<(), String> {
    let destination = resolved_destination(
        manifest
            .parent()
            .ok_or("Import destination folder is required.")?,
    )?;
    #[cfg(windows)]
    let (source, destination) = (
        PathBuf::from(source.to_string_lossy().to_lowercase()),
        PathBuf::from(destination.to_string_lossy().to_lowercase()),
    );
    if source.starts_with(&destination) || destination.starts_with(source) {
        return Err("ProPresenter source and Orbit destination folders must not overlap.".into());
    }
    Ok(())
}

pub fn match_presentation(entry: &Value, source: &Path, items: &[Value]) -> Option<String> {
    let candidate = if let Some(relative) = entry["documentRelativePath"].as_str() {
        let relative = Path::new(relative);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return None;
        }
        source.join(relative).canonicalize().ok()?
    } else {
        let reference = entry["documentPath"].as_str()?;
        let path = if reference.starts_with("file:") {
            url::Url::parse(reference).ok()?.to_file_path().ok()?
        } else {
            let path = PathBuf::from(reference);
            if path.is_absolute() {
                path
            } else {
                source.join(path)
            }
        };
        path.canonicalize().ok()?
    };
    items.iter().find_map(|item| {
        let path = Path::new(item["importSourcePath"].as_str()?)
            .canonicalize()
            .ok()?;
        (path == candidate)
            .then(|| item["id"].as_str().map(str::to_string))
            .flatten()
    })
}

/// How an imported presentation is handled when the destination library already holds one with the same title.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Duplicate {
    /// Overwrites the existing presentation in place, keeping its id so playlists follow the new content.
    Replace,
    KeepBoth,
    /// Skips the imported copy; imported playlists link to the existing presentation.
    UseExisting,
}

impl Duplicate {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "replace" => Ok(Self::Replace),
            "keepBoth" => Ok(Self::KeepBoth),
            "useExisting" => Ok(Self::UseExisting),
            _ => Err(format!("Unknown duplicate import choice: {value}")),
        }
    }
}

/// Id of the destination-library presentation whose title matches, ignoring case.
pub fn same_title(workspace: &Value, library_id: &str, title: &str) -> Option<String> {
    let library = workspace["libraries"]
        .as_array()?
        .iter()
        .find(|library| library["id"].as_str() == Some(library_id))?;
    let title = title.trim().to_lowercase();
    library["itemIds"].as_array()?.iter().find_map(|id| {
        workspace["library"]
            .as_array()?
            .iter()
            .find(|item| {
                item["id"] == *id
                    && item["title"]
                        .as_str()
                        .is_some_and(|value| value.trim().to_lowercase() == title)
            })
            .and_then(|item| item["id"].as_str().map(str::to_string))
    })
}

/// Adds an imported presentation to the destination library, or overwrites the item that already has its id.
pub fn place(patch: &mut Value, library_index: usize, item: Value) -> Result<(), String> {
    let library = patch["library"]
        .as_array_mut()
        .ok_or("Library is invalid.")?;
    if let Some(existing) = library.iter_mut().find(|value| value["id"] == item["id"]) {
        *existing = item;
        return Ok(());
    }
    let id = item["id"].clone();
    library.push(item);
    patch["libraries"][library_index]["itemIds"]
        .as_array_mut()
        .ok_or("Destination library item index is required.")?
        .push(id);
    Ok(())
}
