use crate::external_presentation::ExportedSlideImages;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub struct ImportDirectory {
    pub path: PathBuf,
    committed: bool,
}

impl ImportDirectory {
    pub fn create(workspace_path: &Path) -> Result<Self, String> {
        let workspace = workspace_path
            .parent()
            .ok_or("Workspace folder is required.")?
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let assets = workspace.join("Media/Assets");
        for folder in [workspace.join("Media"), assets.clone()] {
            let metadata = fs::symlink_metadata(&folder).map_err(|error| error.to_string())?;
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || !folder
                    .canonicalize()
                    .map_err(|error| error.to_string())?
                    .starts_with(&workspace)
            {
                return Err("PowerPoint images require a managed workspace asset folder.".into());
            }
        }
        let path = assets.join(format!("PowerPoint-{}", Uuid::new_v4()));
        fs::create_dir(&path).map_err(|error| error.to_string())?;
        Ok(Self {
            path,
            committed: false,
        })
    }

    pub fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for ImportDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub fn require_library(workspace: &Value, library_id: &str) -> Result<(), String> {
    let libraries = workspace["libraries"]
        .as_array()
        .ok_or("Workspace libraries are required.")?;
    if library_id.is_empty()
        || !libraries
            .iter()
            .any(|entry| entry["id"].as_str() == Some(library_id))
    {
        return Err("Choose an existing destination library for the PowerPoint import.".into());
    }
    Ok(())
}

pub fn insert_images(
    workspace_path: &Path,
    library_id: &str,
    source: &Path,
    directory: &ImportDirectory,
    images: &ExportedSlideImages,
) -> Result<(Value, String), String> {
    let current = crate::workspace::read_workspace(workspace_path)?;
    require_library(&current, library_id)?;
    validate_images(workspace_path, directory, images)?;
    let title = source
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .ok_or("PowerPoint file name is required.")?;
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("PowerPoint file name must be valid Unicode.")?;
    let item_id = Uuid::new_v4().to_string();
    let theme_id = Uuid::new_v4().to_string();
    let width = images.slides[0].width;
    let height = images.slides[0].height;
    let slides = images.slides.iter().map(|image| {
        let url = url::Url::from_file_path(&image.path)
            .map_err(|_| "Could not create a PowerPoint image URL.")?;
        Ok(json!({
            "id": Uuid::new_v4().to_string(), "name": format!("Slide {}", image.slide_number),
            "label": image.slide_number, "content": "", "notes": image.notes,
            "width": width, "height": height, "background": "#000000", "backgroundEnabled": true,
            "presentationThemeId": format!("{theme_id}::image"),
            "layers": [{"id": Uuid::new_v4().to_string(), "name": "PowerPoint slide", "type": "Shape",
                "shape": "rectangle", "bounds": {"x":0,"y":0,"width":width,"height":height},
                "fillType":"media", "fill":"Media", "sourcePath":image.path,
                "sourceUrl":url.as_str(), "mediaUrl":url.as_str(), "mediaType":"image",
                "mediaScale":"Scale to Fit", "strokeEnabled":false}]
        }))
    }).collect::<Result<Vec<_>, String>>()?;
    let mut libraries = current["libraries"]
        .as_array()
        .ok_or("Workspace libraries are required.")?
        .clone();
    let library = libraries
        .iter_mut()
        .find(|entry| entry["id"].as_str() == Some(library_id))
        .ok_or("Destination library is unavailable.")?;
    library["itemIds"]
        .as_array_mut()
        .ok_or("Destination library itemIds are required.")?
        .push(json!(item_id));
    let mut items = current["library"]
        .as_array()
        .ok_or("Workspace presentations are required.")?
        .clone();
    items.push(
        json!({"id":item_id,"title":title,"sourceType":"presenter-orbit","sourcePath":"",
        "slides":slides,"importedFrom":{"format":"powerpoint-images","fileName":file_name}}),
    );
    let mut themes = current["themes"]
        .as_array()
        .ok_or("Workspace themes are required.")?
        .clone();
    themes.push(json!({"id":theme_id,"title":format!("PowerPoint / {title}"),"editable":true,
        "schemaVersion":2,"sourceType":"presenter-orbit","sourcePath":"","size":{"width":width,"height":height},
        "slides":[{"id":"image","name":"Image","width":width,"height":height,"background":"#000000",
            "backgroundEnabled":true,"layers":[]}]}));
    let workspace = crate::workspace::save_workspace(
        workspace_path,
        &json!({"libraries":libraries,"library":items,"themes":themes,
            "lastUsedPresentationThemeId":current["lastUsedPresentationThemeId"]}),
    )?;
    Ok((workspace, item_id))
}

fn validate_images(
    workspace_path: &Path,
    directory: &ImportDirectory,
    images: &ExportedSlideImages,
) -> Result<(), String> {
    if images.slides.is_empty()
        || images.slides.len() > 500
        || images.hidden_slide_count as usize + images.slides.len() > 500
    {
        return Err(
            "PowerPoint import requires 1–500 visible slides and at most 500 total slides.".into(),
        );
    }
    let assets = workspace_path
        .parent()
        .ok_or("Workspace folder is required.")?
        .join("Media/Assets")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let root = directory
        .path
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if root.parent() != Some(assets.as_path())
        || fs::symlink_metadata(&directory.path)
            .map_err(|error| error.to_string())?
            .file_type()
            .is_symlink()
    {
        return Err("PowerPoint export directory must stay in the managed asset folder.".into());
    }
    let dimensions = (images.slides[0].width, images.slides[0].height);
    let mut previous = 0;
    for slide in &images.slides {
        if slide.slide_number <= previous
            || slide.slide_number as usize
                > images.slides.len() + images.hidden_slide_count as usize
            || slide.notes.len() > 1024 * 1024
            || (slide.width, slide.height) != dimensions
            || slide.width < 16
            || slide.height < 16
            || slide.width > 3840
            || slide.height > 2160
        {
            return Err("PowerPoint exported slide order, dimensions or notes are invalid.".into());
        }
        previous = slide.slide_number;
        let metadata = fs::symlink_metadata(&slide.path).map_err(|error| error.to_string())?;
        let path = slide
            .path
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let expected_name = format!("Slide-{:04}.png", slide.slide_number);
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > 32 * 1024 * 1024
            || path.parent() != Some(root.as_path())
            || path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str())
        {
            return Err(
                "PowerPoint images must be regular PNG files in the reserved import directory."
                    .into(),
            );
        }
        let mut reader = image::ImageReader::open(&path)
            .map_err(|error| error.to_string())?
            .with_guessed_format()
            .map_err(|error| error.to_string())?;
        if reader.format() != Some(image::ImageFormat::Png) {
            return Err("PowerPoint exported an invalid PNG.".into());
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(3840);
        limits.max_image_height = Some(2160);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .map_err(|error| format!("PowerPoint slide image is invalid: {error}"))?;
        if (decoded.width(), decoded.height()) != dimensions {
            return Err("PowerPoint image dimensions do not match the exported slide.".into());
        }
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|file| file.sync_all())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
