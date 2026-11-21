//! Maps the audio action of an imported ProPresenter timeline to a workspace
//! audio asset. The importer leaves `importedSoundtrack` on the timeline; it
//! never reaches the workspace.

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// Returns `(presentation source path, warning)` for every soundtrack that
/// could not be mapped. Such a timeline is imported without a soundtrack.
pub(crate) fn resolve(items: &mut [Value], media: &mut Vec<Value>) -> Vec<(String, String)> {
    let mut warnings = Vec::new();
    for item in items {
        let Some(timeline) = item.get_mut("timeline").and_then(Value::as_object_mut) else {
            continue;
        };
        let Some(imported) = timeline.remove("importedSoundtrack") else {
            continue;
        };
        let reference = item["importSourcePath"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        match map(&imported, media) {
            Ok(id) => {
                if let Some(timeline) = item.get_mut("timeline").and_then(Value::as_object_mut) {
                    timeline.insert("soundtrackMediaId".into(), json!(id));
                }
            }
            Err(message) => warnings.push((
                reference,
                format!("timeline soundtrack was not imported: {message}"),
            )),
        }
    }
    warnings
}

fn map(imported: &Value, media: &mut Vec<Value>) -> Result<String, String> {
    let url = imported["sourceUrl"]
        .as_str()
        .filter(|url| !url.is_empty())
        .ok_or("the audio action does not name a file")?;
    let path = local_path(url)?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if super::propresenter_media_type(&extension) != Some("audio") {
        return Err(format!("'{}' is not an audio file", path.display()));
    }
    let canonical = path.canonicalize().ok();
    let existing = media.iter().find(|asset| {
        asset["sourceId"].as_str() == Some(url)
            || canonical.as_ref().is_some_and(|canonical| {
                asset["sourcePath"]
                    .as_str()
                    .and_then(|source| Path::new(source).canonicalize().ok())
                    .as_ref()
                    == Some(canonical)
            })
    });
    if let Some(asset) = existing {
        return if asset["type"].as_str() == Some("audio") {
            asset["id"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "the matching media asset has no identity".to_string())
        } else {
            Err(format!("'{}' is not an audio asset", path.display()))
        };
    }
    if !path.is_file() {
        return Err(format!("'{}' was not found", path.display()));
    }
    let mut number = 1;
    let id = loop {
        let id = format!("timeline-soundtrack-{number}");
        if !media.iter().any(|asset| asset["id"].as_str() == Some(&id)) {
            break id;
        }
        number += 1;
    };
    let mut asset = Map::new();
    asset.insert("id".into(), json!(id));
    asset.insert(
        "title".into(),
        json!(path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Soundtrack")),
    );
    asset.insert("type".into(), json!("audio"));
    asset.insert("sourcePath".into(), json!(path));
    asset.insert(
        "sourceUrl".into(),
        json!(url::Url::from_file_path(&path)
            .map_err(|_| format!("'{}' is not an absolute path", path.display()))?
            .to_string()),
    );
    asset.insert("sourceId".into(), json!(url));
    media.push(Value::Object(asset));
    Ok(id)
}

fn local_path(reference: &str) -> Result<PathBuf, String> {
    let path = if reference.starts_with("file:") {
        url::Url::parse(reference)
            .map_err(|error| format!("'{reference}' is not a valid URL: {error}"))?
            .to_file_path()
            .map_err(|_| format!("'{reference}' is not a local file URL"))?
    } else if reference.contains("://") {
        return Err(format!("'{reference}' is not a local file"));
    } else {
        PathBuf::from(reference)
    };
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!("'{reference}' is not an absolute path"))
    }
}
