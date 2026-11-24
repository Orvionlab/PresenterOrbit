use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
};

const ASSET_KEYS: &[&str] = &[
    "sourcePath",
    "sourceUrl",
    "mediaSourcePath",
    "mediaUrl",
    "mediaPath",
    "mediaFillSourcePath",
    "mediaFillUrl",
    "backgroundMediaPath",
    "backgroundMediaUrl",
];

pub(super) struct Capture<'a> {
    root: PathBuf,
    source_root: &'a Path,
    paths: HashMap<PathBuf, PathBuf>,
    pub warnings: Vec<String>,
}

impl<'a> Capture<'a> {
    pub fn new(root: PathBuf, source_root: &'a Path) -> Self {
        Self {
            root,
            source_root,
            paths: HashMap::new(),
            warnings: Vec::new(),
        }
    }

    fn copy(&mut self, reference: &str) -> Result<Option<PathBuf>, String> {
        let source = if reference.starts_with("file:") {
            let Some(path) = url::Url::parse(reference)
                .ok()
                .and_then(|url| url.to_file_path().ok())
            else {
                return Ok(None);
            };
            path
        } else {
            PathBuf::from(reference)
        };
        let relative = !source.is_absolute();
        let source = if relative {
            let local = reference.replace('\\', "/");
            let path = Path::new(&local);
            if local.contains(':')
                || !path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
            {
                return Ok(None);
            }
            self.source_root.join(path)
        } else {
            source
        };
        let source = match source.canonicalize() {
            Ok(source) if !relative || source.starts_with(self.source_root) => source,
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "Imported media is unavailable: {reference}: {error}"
                ));
            }
        };
        if let Some(target) = self.paths.get(&source) {
            return Ok(Some(target.clone()));
        }
        crate::portable_bundle::validate_asset(&source)?;
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .ok_or("Imported media extension is required.")?;
        if self.paths.is_empty() {
            fs::create_dir_all(&self.root).map_err(|error| error.to_string())?;
        }
        let target = self
            .root
            .join(format!("{}.{}", self.paths.len(), extension));
        fs::copy(&source, &target)
            .map_err(|error| format!("Imported media could not be copied: {error}"))?;
        self.paths.insert(source, target.clone());
        Ok(Some(target))
    }

    pub fn rewrite(&mut self, value: &mut Value, location: &str) -> Result<(), String> {
        match value {
            Value::Object(object) => {
                let provenance = object.contains_key("sourceType");
                for (key, value) in object {
                    if ASSET_KEYS.contains(&key.as_str()) && !(key == "sourcePath" && provenance) {
                        if let Some(reference) = value
                            .as_str()
                            .filter(|value| !value.is_empty() && !value.starts_with("data:"))
                        {
                            let Some(target) = self.copy(reference)? else {
                                self.warnings.push(format!(
                                    "{location}/{key}: media {reference} is unavailable on this system; the original reference was retained"
                                ));
                                continue;
                            };
                            *value = if key.ends_with("Url") {
                                json!(url::Url::from_file_path(target)
                                    .map_err(|_| "Imported media path is invalid.")?
                                    .to_string())
                            } else {
                                json!(target)
                            };
                        }
                    } else {
                        self.rewrite(value, &format!("{location}/{key}"))?;
                    }
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter_mut().enumerate() {
                    self.rewrite(value, &format!("{location}/{index}"))?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

pub(super) fn media_directory(workspace_path: &Path, name: &str) -> Result<PathBuf, String> {
    let folder = workspace_path
        .parent()
        .ok_or("Workspace folder is required.")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let media = folder
        .join("Media")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !media.starts_with(&folder) {
        return Err("Imported media must remain inside the workspace.".into());
    }
    let container = media.join(name);
    fs::create_dir_all(&container).map_err(|error| error.to_string())?;
    let container = container
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !container.starts_with(&media) {
        return Err("Imported media folder escapes the workspace.".into());
    }
    Ok(container)
}

pub(super) fn save(
    workspace_path: &Path,
    source_root: &Path,
    patch: &Value,
) -> Result<Value, String> {
    let container = media_directory(workspace_path, "ProPresenter Imports")?;
    let folder = workspace_path
        .parent()
        .ok_or("Workspace folder is required.")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let mut capture = Capture::new(
        container.join(uuid::Uuid::new_v4().to_string()),
        source_root,
    );
    let mut charts = Vec::new();
    let result = (|| {
        let mut patch = patch.clone();
        for key in ["library", "themes", "props", "masks", "stageLayouts"] {
            if let Some(values) = patch.get_mut(key) {
                capture.rewrite(values, key)?;
            }
        }
        patch["importWarnings"]
            .as_array_mut()
            .ok_or("Import warnings are required.")?
            .extend(capture.warnings.iter().map(|warning| json!(warning)));
        super::propresenter_charts::attach(&folder, &mut patch, &mut charts)?;
        let workspace = super::read_workspace(workspace_path)?;
        for key in ["stageLayouts", "masks"] {
            merge_canvas_documents(&workspace, &mut patch, key)?;
        }
        if crate::authoring::reusable_themes(&patch["themes"])
            .next()
            .is_none()
        {
            let themes = patch["themes"]
                .as_array_mut()
                .ok_or("Imported themes must be an array.")?;
            themes.extend(crate::authoring::reusable_themes(&workspace["themes"]).cloned());
        }
        super::save_read_workspace(workspace_path, workspace, &patch, &[])
    })();
    if result.is_err() {
        for directory in charts {
            fs::remove_dir_all(&directory)
                .map_err(|error| format!("Imported chord chart cleanup failed: {error}"))?;
        }
        if capture.root.exists() {
            fs::remove_dir_all(&capture.root)
                .map_err(|error| format!("Imported media cleanup failed: {error}"))?;
        }
    }
    result
}

fn merge_canvas_documents(workspace: &Value, patch: &mut Value, key: &str) -> Result<(), String> {
    let patch = patch
        .as_object_mut()
        .ok_or("Import patch must be an object.")?;
    let imported = patch
        .remove(key)
        .and_then(|value| match value {
            Value::Array(layouts) => Some(layouts),
            _ => None,
        })
        .ok_or_else(|| format!("Imported {key} must be an array."))?;
    if imported.is_empty() {
        return Ok(());
    }
    let mut layouts = workspace[key]
        .as_array()
        .ok_or_else(|| format!("Workspace {key} must be an array."))?
        .clone();
    for layout in imported {
        let id = layout["id"]
            .as_str()
            .ok_or_else(|| format!("An imported document in {key} requires an ID."))?;
        if let Some(existing) = layouts.iter_mut().find(|existing| existing["id"] == id) {
            *existing = layout;
        } else {
            layouts.push(layout);
        }
    }
    patch.insert(key.into(), Value::Array(layouts));
    Ok(())
}
