use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
};

fn fingerprint(bytes: &[u8]) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hash);
    format!("{:016x}", hash.finish())
}

fn collect_reference(
    value: &Value,
    key: &str,
    root: &Path,
    paths: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    let text = value
        .as_str()
        .ok_or_else(|| format!("Media dependency {key} must be a path string."))?;
    if text.is_empty() {
        return Ok(());
    }
    let path = if key.ends_with("Url") || (key == "artwork" && !Path::new(text).is_absolute()) {
        let url = url::Url::parse(text)
            .map_err(|error| format!("Invalid media dependency {key}: {error}"))?;
        if url.scheme() != "file" {
            return Ok(());
        }
        url.to_file_path()
            .map_err(|_| format!("Invalid local media dependency {key}: {text}"))?
    } else {
        let path = PathBuf::from(text.replace('\\', "/"));
        if path.is_absolute() {
            path
        } else if matches!(key, "assetFile" | "thumbnailFile") {
            root.join("Media").join(path)
        } else if key == "path" {
            root.join(path)
        } else {
            return Err(format!(
                "Media dependency {key} must be an absolute path: {text}"
            ));
        }
    };
    let canonical = path.canonicalize().map_err(|error| {
        format!(
            "Media dependency {key} is unavailable: {} ({error}). Relink it before cleanup.",
            path.display()
        )
    })?;
    if !canonical.is_file() {
        return Err(format!(
            "Media dependency {key} is not a file: {}",
            path.display()
        ));
    }
    paths.insert(path_key(&canonical));
    Ok(())
}

fn collect_paths(value: &Value, root: &Path, paths: &mut HashSet<PathBuf>) -> Result<(), String> {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_paths(value, root, paths)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                // Imported document provenance and smart-folder roots are not playback assets.
                if key == "sourcePath"
                    && (values.contains_key("slides")
                        || values.contains_key("slide")
                        || values.get("type").and_then(Value::as_str) == Some("smart"))
                {
                    continue;
                }
                if matches!(
                    key.as_str(),
                    "sourcePath"
                        | "sourceUrl"
                        | "mediaFillSourcePath"
                        | "mediaFillUrl"
                        | "mediaUrl"
                        | "thumbnailUrl"
                        | "chordChartUrl"
                        | "artwork"
                        | "mediaSourcePath"
                        | "mediaPath"
                        | "backgroundMediaPath"
                        | "backgroundMediaUrl"
                        | "audioPath"
                        | "audioUrl"
                        | "logoImagePath"
                        | "logoImageUrl"
                        | "assetFile"
                        | "thumbnailFile"
                        | "path"
                ) {
                    collect_reference(value, key, root, paths)?;
                } else if key == "pagePaths" {
                    for page in value
                        .as_array()
                        .ok_or("Chart page dependencies must be an array.")?
                    {
                        collect_reference(page, "pagePaths", root, paths)?;
                    }
                } else {
                    collect_paths(value, root, paths)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn path_key(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

fn file_fingerprint(path: &Path) -> Result<(u64, String), String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hash.write(&buffer[..count]);
    }
    let after = file.metadata().map_err(|error| error.to_string())?;
    if metadata.len() != after.len()
        || metadata.modified().map_err(|error| error.to_string())?
            != after.modified().map_err(|error| error.to_string())?
    {
        return Err("Managed media changed while scanning. Scan again.".into());
    }
    Ok((metadata.len(), format!("{:016x}", hash.finish())))
}

fn scan(
    folder: &Path,
    assets: &Path,
    referenced: &HashSet<PathBuf>,
    files: &mut Vec<Value>,
) -> Result<(), String> {
    for entry in fs::read_dir(folder).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_symlink() {
            return Err("Managed media cleanup cannot traverse symbolic links.".into());
        }
        let canonical = path.canonicalize().map_err(|error| error.to_string())?;
        if !canonical.starts_with(assets) || canonical != path {
            return Err("Managed media path is outside the asset folder or traverses a redirected directory.".into());
        }
        if kind.is_dir() {
            scan(&path, assets, referenced, files)?;
        } else if kind.is_file() && !referenced.contains(&path_key(&canonical)) {
            let (bytes, fingerprint) = file_fingerprint(&canonical)?;
            files.push(json!({"relativePath":canonical.strip_prefix(assets).map_err(|error| error.to_string())?.to_string_lossy().replace('\\', "/"), "bytes":bytes, "fingerprint":fingerprint}));
        }
    }
    Ok(())
}

pub fn review(workspace_path: &Path, live: &Value) -> Result<Value, String> {
    let workspace = crate::workspace::read_workspace(workspace_path)?;
    let root = workspace_path
        .parent()
        .ok_or("Workspace folder is required.")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let assets = root
        .join("Media/Assets")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if assets != root.join("Media/Assets") {
        return Err("Managed assets cannot use a redirected directory.".into());
    }
    let mut referenced = HashSet::new();
    collect_paths(&workspace, &root, &mut referenced)?;
    collect_paths(live, &root, &mut referenced)?;
    let mut files = Vec::new();
    scan(&assets, &assets, &referenced, &mut files)?;
    files.sort_by(|a, b| a["relativePath"].as_str().cmp(&b["relativePath"].as_str()));
    Ok(
        json!({"workspaceFingerprint":fingerprint(&serde_json::to_vec(&workspace).map_err(|error| error.to_string())?),"files":files}),
    )
}

pub struct PreparedCleanup {
    plan: Value,
}

pub fn prepare(
    workspace_path: &Path,
    reviewed: &Value,
    live: &Value,
) -> Result<PreparedCleanup, String> {
    let current = review(workspace_path, live)?;
    if current != *reviewed {
        return Err(
            "Media or references changed after review. Scan again before cleaning up.".into(),
        );
    }
    Ok(PreparedCleanup { plan: current })
}

pub fn execute(
    workspace_path: &Path,
    prepared: PreparedCleanup,
    live: &Value,
) -> Result<Value, String> {
    let files = prepared.plan["files"]
        .as_array()
        .ok_or("Cleanup files are required.")?;
    if files.is_empty() {
        return Err("No unused managed media was selected.".into());
    }
    let root = workspace_path
        .parent()
        .ok_or("Workspace folder is required.")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let assets = root
        .join("Media/Assets")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let workspace = crate::workspace::read_workspace(workspace_path)?;
    if prepared.plan["workspaceFingerprint"]
        != fingerprint(&serde_json::to_vec(&workspace).map_err(|error| error.to_string())?)
    {
        return Err("Workspace changed after cleanup review. Scan again.".into());
    }
    let mut referenced = HashSet::new();
    collect_paths(&workspace, &root, &mut referenced)?;
    collect_paths(live, &root, &mut referenced)?;
    for file in files {
        let relative = file["relativePath"]
            .as_str()
            .ok_or("Cleanup path is required.")?;
        let source = assets
            .join(relative)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if source != assets.join(relative)
            || !source.starts_with(&assets)
            || referenced.contains(&path_key(&source))
        {
            return Err(
                "Managed media became referenced or changed its path after review. Scan again."
                    .into(),
            );
        }
        let (bytes, fingerprint) = file_fingerprint(&source)?;
        if file["bytes"].as_u64() != Some(bytes)
            || file["fingerprint"].as_str() != Some(&fingerprint)
        {
            return Err("Managed media bytes changed after cleanup review. Scan again.".into());
        }
    }
    let recovery = root.join("Media/Cleanup");
    fs::create_dir_all(&recovery).map_err(|error| error.to_string())?;
    let recovery = recovery.canonicalize().map_err(|error| error.to_string())?;
    if recovery != root.join("Media/Cleanup") {
        return Err("Media recovery folder cannot use a redirected directory.".into());
    }
    let destination = recovery.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&destination).map_err(|error| error.to_string())?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for file in files {
        let relative = file["relativePath"]
            .as_str()
            .ok_or("Cleanup path is required.")?;
        let source = assets.join(relative);
        let target = destination.join(relative);
        let operation = fs::create_dir_all(target.parent().ok_or("Cleanup parent is required.")?)
            .and_then(|_| fs::rename(&source, &target));
        if let Err(error) = operation {
            let mut rollback_errors = Vec::new();
            for (source, target) in moved.iter().rev() {
                if let Err(error) = fs::rename(target, source) {
                    rollback_errors.push(error.to_string());
                }
            }
            return Err(format!(
                "Media cleanup failed: {error}. Recovery folder: {}. Rollback errors: {}",
                destination.display(),
                rollback_errors.join(", ")
            ));
        }
        moved.push((source, target));
    }
    Ok(json!({"movedFiles":moved.len(), "recoveryPath":destination}))
}
