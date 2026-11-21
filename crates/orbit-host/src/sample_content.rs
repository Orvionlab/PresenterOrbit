use serde_json::Value;
use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

static ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn initialize(root: PathBuf) -> Result<(), String> {
    ROOT.set(root)
        .map_err(|_| "Sample content has already been initialized.".to_string())
}

fn root() -> Result<PathBuf, String> {
    if cfg!(test) {
        return Ok(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/Sample Content"));
    }
    ROOT.get()
        .cloned()
        .ok_or_else(|| "Sample content has not been initialized.".to_string())
}

fn index() -> Result<Value, String> {
    serde_json::from_slice(
        &fs::read(root()?.join("Themes/index.json"))
            .map_err(|error| format!("Sample theme index is unavailable: {error}"))?,
    )
    .map_err(|error| error.to_string())
}

pub fn countdown_timers() -> Result<Vec<Value>, String> {
    let mut timers = crate::workspace::content_themes::basic_countdown_timers();
    timers.extend(
        index()?["countdownTimers"]
            .as_array()
            .ok_or("Sample countdown timers are required.")?
            .iter()
            .cloned(),
    );
    Ok(timers)
}

pub fn themes(workspace: &Path) -> Result<Vec<Value>, String> {
    let root = root()?.join("Themes");
    let index = index()?;
    let entries = index["themes"]
        .as_array()
        .ok_or("Sample theme index requires themes.")?;
    entries
        .iter()
        .map(|entry| {
            let folder = entry["folder"]
                .as_str()
                .ok_or("Sample theme folder is required.")?;
            let mut theme = serde_json::from_slice(
                &fs::read(root.join(folder).join("theme.json"))
                    .map_err(|error| format!("Sample theme '{folder}' is unavailable: {error}"))?,
            )
            .map_err(|error| error.to_string())?;
            resolve_assets(&mut theme, &root, &workspace.join("Themes"), "")?;
            Ok(theme)
        })
        .collect()
}

pub fn install(workspace: &Path) -> Result<Vec<Value>, String> {
    let themes = themes(workspace)?;
    crate::workspace_registry::copy_folder(&root()?.join("Themes"), &workspace.join("Themes"), "")?;
    Ok(themes)
}

pub fn stage_layouts() -> Result<Vec<Value>, String> {
    let index: Value = serde_json::from_slice(
        &fs::read(root()?.join("Stage Layouts/index.json"))
            .map_err(|error| format!("Sample stage layouts are unavailable: {error}"))?,
    )
    .map_err(|error| error.to_string())?;
    index["stageLayouts"]
        .as_array()
        .cloned()
        .ok_or_else(|| "Sample stage index requires stageLayouts.".to_string())
}

fn resolve_assets(
    value: &mut Value,
    source: &Path,
    target: &Path,
    key: &str,
) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                resolve_assets(value, source, target, key)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                resolve_assets(value, source, target, key)?;
            }
        }
        Value::String(reference) => {
            if let Some(relative) = reference.strip_prefix("./") {
                let path = Path::new(relative);
                if path
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
                    || !source.join(path).is_file()
                {
                    return Err(format!("Sample media is unavailable: {reference}"));
                }
                let path = target.join(path);
                *reference = if key.ends_with("Url") {
                    url::Url::from_file_path(path)
                        .map_err(|_| "Sample media requires an absolute workspace path.")?
                        .to_string()
                } else {
                    path.to_string_lossy().into_owned()
                };
            }
        }
        _ => {}
    }
    Ok(())
}
