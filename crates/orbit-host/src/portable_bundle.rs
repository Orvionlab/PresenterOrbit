use crate::workspace_import::{place, same_title, Duplicate};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bundle {
    schema: String,
    version: u32,
    presentation: Value,
    media: Vec<Value>,
    themes: Vec<Value>,
    assets: BTreeMap<String, Vec<u8>>,
}

fn id(value: &Value) -> Result<&str, String> {
    value["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Bundle object id is required.".into())
}

fn asset_key(key: &str) -> bool {
    matches!(
        key,
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
    )
}

fn references(
    value: &Value,
    media: &mut HashSet<String>,
    themes: &mut HashSet<String>,
) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            if let Some(target) = object
                .get("targetId")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
            {
                match object.get("type").and_then(Value::as_str) {
                    Some("media") => {
                        media.insert(target.into());
                    }
                    Some(kind) => {
                        return Err(format!(
                            "Bundle cannot transfer external {kind} action target: {target}."
                        ))
                    }
                    None => return Err("Bundle target reference has no action type.".into()),
                }
            }
            for (key, child) in object {
                if let Some(reference) = child.as_str().filter(|v| !v.is_empty()) {
                    match key.as_str() {
                    "mediaId" | "mediaFillId" | "audioId" | "backgroundId" | "mediaBackgroundId" | "soundtrackMediaId" => {
                        if !crate::workspace_sync::has_own_media_source(object, key) {
                            media.insert(reference.into());
                        }
                    },
                    "presentationThemeId" => {
                        let (theme, layout) = reference.split_once("::").ok_or("Bundle theme assignment is invalid.")?;
                        if theme.is_empty() || layout.is_empty() { return Err("Bundle theme assignment is invalid.".into()); }
                        themes.insert(theme.into());
                    },
                    "macroId" | "timerId" | "clearGroupId" | "linkedPresentationId" => return Err(format!("Presentation depends on unsupported external reference {key}: {reference}.")),
                    _ => {}
                }
                }
                references(child, media, themes)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                references(child, media, themes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn local_path(value: &str) -> Result<PathBuf, String> {
    if value.starts_with("file:") {
        return url::Url::parse(value)
            .map_err(|e| e.to_string())?
            .to_file_path()
            .map_err(|_| "Invalid local asset URL.".into());
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!(
            "Bundle asset must be a local absolute path: {value}"
        ));
    }
    Ok(path)
}

pub(crate) fn validate_asset(path: &Path) -> Result<(), String> {
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kind = match extension.as_str() {
        "pdf" => {
            let mut header = [0; 5];
            fs::File::open(path)
                .map_err(|e| e.to_string())?
                .read_exact(&mut header)
                .map_err(|e| e.to_string())?;
            return if &header == b"%PDF-" {
                Ok(())
            } else {
                Err("Invalid chart PDF source.".into())
            };
        }
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" => "image",
        "mp4" | "mov" | "m4v" | "avi" | "mkv" | "webm" => "video",
        "mp3" | "wav" | "m4a" | "aac" | "flac" | "aiff" => "audio",
        _ => return Err(format!("Unsupported bundle asset: {}", path.display())),
    };
    crate::media_relink::validate_replacement(path, kind)
}

fn capture(
    value: &mut Value,
    assets: &mut BTreeMap<String, Vec<u8>>,
    paths: &mut BTreeMap<PathBuf, String>,
) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            if let Some(chart) = object.get_mut("chordChart") {
                let parsed: crate::chord_chart::ChordChart = serde_json::from_value(chart.clone())
                    .map_err(|e| format!("Invalid bundle chord chart: {e}"))?;
                let source = Path::new(&parsed.source_path);
                let directory = source
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .ok_or("Managed chart directory is required.")?;
                crate::chord_chart::validate(directory, &parsed)?;
                let mut source = json!({"sourcePath":parsed.source_path});
                capture(&mut source, assets, paths)?;
                chart["sourcePath"] = source["sourcePath"].clone();
                for page in chart["pagePaths"]
                    .as_array_mut()
                    .ok_or("Chart pages are required.")?
                {
                    let mut asset = json!({"sourcePath":page});
                    capture(&mut asset, assets, paths)?;
                    *page = asset["sourcePath"].clone();
                }
            }
            if ["missing", "thumbnailMissing"]
                .iter()
                .any(|key| object.get(*key).and_then(Value::as_bool) == Some(true))
            {
                return Err(
                    "Bundle dependency includes a missing media asset or thumbnail.".into(),
                );
            }
            for key in [
                "importSourcePath",
                "externalSource",
                "assetFile",
                "thumbnailFile",
                "libraryId",
            ] {
                object.remove(key);
            }
            let pairs = [
                ("sourcePath", "sourceUrl"),
                ("mediaFillSourcePath", "mediaFillUrl"),
            ];
            for (path, url) in pairs {
                if object
                    .get(path)
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.is_empty())
                {
                    object.remove(url);
                }
            }
            for (key, child) in object {
                if key == "chordChart" {
                    continue;
                }
                if asset_key(key) {
                    if let Some(path) = child.as_str().filter(|v| !v.is_empty()) {
                        let path = fs::canonicalize(local_path(path)?)
                            .map_err(|e| format!("Bundle asset is unavailable: {e}"))?;
                        let token = if let Some(token) = paths.get(&path) {
                            token.clone()
                        } else {
                            if fs::metadata(&path).map_err(|e| e.to_string())?.len()
                                > MAX_BYTES as u64
                            {
                                return Err("Bundle asset exceeds the 64 MiB limit.".into());
                            }
                            validate_asset(&path)?;
                            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
                            let extension = path
                                .extension()
                                .and_then(|v| v.to_str())
                                .ok_or("Asset extension is required.")?
                                .to_ascii_lowercase();
                            let same = assets
                                .iter()
                                .find(|(name, stored)| {
                                    name.ends_with(&format!(".{extension}")) && **stored == bytes
                                })
                                .map(|(key, _)| key.clone());
                            let token = if let Some(token) = same {
                                token
                            } else {
                                if bytes.is_empty()
                                    || assets.values().map(Vec::len).sum::<usize>() + bytes.len()
                                        > MAX_BYTES
                                {
                                    return Err(
                                        "Bundle assets exceed the 64 MiB limit or are empty."
                                            .into(),
                                    );
                                }
                                let token = format!("{}.{}", assets.len(), extension);
                                assets.insert(token.clone(), bytes);
                                token
                            };
                            paths.insert(path, token.clone());
                            token
                        };
                        *child = json!(format!("bundle://{token}"));
                    }
                } else {
                    capture(child, assets, paths)?;
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                capture(child, assets, paths)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn prepare(workspace: &Value, item_id: &str) -> Result<Bundle, String> {
    let mut presentation = workspace["library"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["id"].as_str() == Some(item_id))
        })
        .cloned()
        .ok_or("Presentation is missing.")?;
    let source_library = workspace["libraries"].as_array().and_then(|items| {
        items.iter().find(|library| {
            library["itemIds"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(item_id)))
        })
    });
    if let Some(assignment) = source_library
        .and_then(|library| library.get("presentationThemeId"))
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        for slide in presentation["slides"]
            .as_array_mut()
            .ok_or("Presentation slides are required.")?
        {
            if slide.get("presentationThemeId").is_none() {
                slide["presentationThemeId"] = json!(assignment);
            }
        }
    }
    let mut media_ids = HashSet::new();
    let mut theme_ids = HashSet::new();
    references(&presentation, &mut media_ids, &mut theme_ids)?;
    let mut themes = Vec::new();
    let mut media = Vec::new();
    loop {
        let mut pending_themes = theme_ids
            .iter()
            .filter(|id| !themes.iter().any(|v: &Value| v["id"].as_str() == Some(id)))
            .cloned()
            .collect::<Vec<_>>();
        let mut pending_media = media_ids
            .iter()
            .filter(|id| {
                !media.iter().any(|v: &Value| {
                    v["id"].as_str() == Some(id) || v["sourceId"].as_str() == Some(id)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        if pending_themes.is_empty() && pending_media.is_empty() {
            break;
        }
        pending_themes.sort();
        pending_media.sort();
        for theme_id in pending_themes {
            let mut theme = workspace["themes"]
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item["id"].as_str() == Some(&theme_id))
                })
                .cloned()
                .ok_or_else(|| format!("Referenced theme is missing: {theme_id}"))?;
            theme
                .as_object_mut()
                .ok_or("Theme must be an object.")?
                .remove("sourcePath");
            references(&theme, &mut media_ids, &mut theme_ids)?;
            themes.push(theme);
        }
        for media_id in pending_media {
            let matches = workspace["media"]
                .as_array()
                .ok_or("Workspace media is invalid.")?
                .iter()
                .filter(|item| {
                    item["id"].as_str() == Some(&media_id)
                        || item["sourceId"].as_str() == Some(&media_id)
                })
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(format!(
                    "Referenced media is missing or ambiguous: {media_id}"
                ));
            }
            let selected = (*matches[0]).clone();
            references(&selected, &mut media_ids, &mut theme_ids)?;
            if !media
                .iter()
                .any(|item: &Value| item["id"] == selected["id"])
            {
                media.push(selected);
            }
        }
    }
    media.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    let mut assets = BTreeMap::new();
    let mut paths = BTreeMap::new();
    themes.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    capture(&mut presentation, &mut assets, &mut paths)?;
    for item in media.iter_mut().chain(themes.iter_mut()) {
        capture(item, &mut assets, &mut paths)?;
    }
    let bundle = Bundle {
        schema: "presenter-orbit.presentation-bundle".into(),
        version: 1,
        presentation,
        media,
        themes,
        assets,
    };
    validate(&bundle)?;
    Ok(bundle)
}

fn visit_assets(
    value: &Value,
    assets: &BTreeMap<String, Vec<u8>>,
    used: &mut HashSet<String>,
) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            if let Some(chart) = object.get("chordChart") {
                let parsed: crate::chord_chart::ChordChart = serde_json::from_value(chart.clone())
                    .map_err(|e| format!("Invalid bundle chord chart: {e}"))?;
                if parsed.page_paths.is_empty()
                    || parsed.page_paths.len() > 64
                    || parsed.page >= parsed.page_paths.len()
                {
                    return Err("Bundle chord chart page is outside the document.".into());
                }
                for reference in
                    std::iter::once(&parsed.source_path).chain(parsed.page_paths.iter())
                {
                    let token = reference
                        .strip_prefix("bundle://")
                        .ok_or("Bundle chart has a nonportable asset reference.")?;
                    if !assets.contains_key(token) {
                        return Err("Bundle chart asset is missing.".into());
                    }
                    used.insert(token.into());
                }
                let source = &assets[parsed
                    .source_path
                    .strip_prefix("bundle://")
                    .expect("Validated chart token")];
                crate::chord_chart::validate_source_fingerprint(
                    source,
                    &parsed.source_fingerprint,
                )?;
            }
            if ["missing", "thumbnailMissing"]
                .iter()
                .any(|key| object.get(*key).and_then(Value::as_bool) == Some(true))
            {
                return Err(
                    "Bundle dependency includes a missing media asset or thumbnail.".into(),
                );
            }
            for (key, child) in object {
                if key == "chordChart" {
                    continue;
                }
                if asset_key(key) {
                    if !child.is_string() {
                        return Err(format!("Bundle asset reference {key} must be a string."));
                    }
                    if let Some(reference) = child.as_str().filter(|v| !v.is_empty()) {
                        let token = reference
                            .strip_prefix("bundle://")
                            .ok_or("Bundle contains a nonportable asset reference.")?;
                        if token.ends_with(".pdf") {
                            return Err("PDF bundle assets must be managed chart sources.".into());
                        }
                        if !assets.contains_key(token) {
                            return Err(format!("Bundle asset is missing: {token}"));
                        }
                        used.insert(token.into());
                    }
                } else {
                    visit_assets(child, assets, used)?;
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                visit_assets(child, assets, used)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate(bundle: &Bundle) -> Result<(), String> {
    if bundle.schema != "presenter-orbit.presentation-bundle" || bundle.version != 1 {
        return Err("Unsupported bundle schema or version.".into());
    }
    id(&bundle.presentation)?;
    if !bundle.presentation["slides"].is_array() {
        return Err("Bundle presentation slides are required.".into());
    }
    let mut slide_ids = HashSet::new();
    for slide in bundle.presentation["slides"]
        .as_array()
        .expect("Validated slides")
    {
        if !slide_ids.insert(id(slide)?) {
            return Err("Duplicate bundle slide id.".into());
        }
    }
    for media in &bundle.media {
        if !matches!(media["type"].as_str(), Some("image" | "audio" | "video")) {
            return Err("Bundle media type is unsupported.".into());
        }
        if !["sourcePath", "sourceUrl"].iter().any(|key| {
            media[*key]
                .as_str()
                .is_some_and(|v| v.starts_with("bundle://"))
        }) {
            return Err("Referenced bundle media has no portable source asset.".into());
        }
    }
    for collection in [&bundle.media, &bundle.themes] {
        let mut ids = HashSet::new();
        for item in collection {
            if !ids.insert(id(item)?) {
                return Err("Duplicate bundle object id.".into());
            }
        }
    }
    if bundle.assets.values().map(Vec::len).sum::<usize>() > MAX_BYTES {
        return Err("Bundle assets exceed 64 MiB.".into());
    }
    for (key, bytes) in &bundle.assets {
        let (number, extension) = key.split_once('.').ok_or("Invalid bundle asset name.")?;
        if number.is_empty()
            || !number.bytes().all(|v| v.is_ascii_digit())
            || extension.is_empty()
            || !extension.bytes().all(|v| v.is_ascii_alphanumeric())
            || bytes.is_empty()
        {
            return Err("Invalid bundle asset name or empty asset.".into());
        }
    }
    let mut used = HashSet::new();
    let mut media_ids = HashSet::new();
    let mut theme_ids = HashSet::new();
    for value in std::iter::once(&bundle.presentation)
        .chain(bundle.media.iter())
        .chain(bundle.themes.iter())
    {
        visit_assets(value, &bundle.assets, &mut used)?;
        references(value, &mut media_ids, &mut theme_ids)?;
    }
    if used.len() != bundle.assets.len() {
        return Err("Bundle contains unreferenced assets.".into());
    }
    if media_ids.iter().any(|id| {
        !bundle
            .media
            .iter()
            .any(|v| v["id"].as_str() == Some(id) || v["sourceId"].as_str() == Some(id))
    }) || theme_ids
        .iter()
        .any(|id| !bundle.themes.iter().any(|v| v["id"].as_str() == Some(id)))
    {
        return Err("Bundle contains unresolved dependencies.".into());
    }
    fn layouts(value: &Value, themes: &[Value]) -> Result<(), String> {
        match value {
            Value::Object(object) => {
                if let Some(assignment) = object
                    .get("presentationThemeId")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                {
                    let (theme, layout) = assignment
                        .split_once("::")
                        .ok_or("Invalid theme assignment.")?;
                    if !themes.iter().any(|v| {
                        v["id"].as_str() == Some(theme)
                            && v["slides"].as_array().is_some_and(|slides| {
                                slides.iter().any(|v| v["id"].as_str() == Some(layout))
                            })
                    }) {
                        return Err(format!("Bundle theme layout is missing: {assignment}"));
                    }
                }
                for child in object.values() {
                    layouts(child, themes)?;
                }
            }
            Value::Array(values) => {
                for child in values {
                    layouts(child, themes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    layouts(&bundle.presentation, &bundle.themes)?;
    for theme in &bundle.themes {
        layouts(theme, &bundle.themes)?;
    }
    fn theme_cycle(
        theme: &Value,
        themes: &[Value],
        active: &mut HashSet<String>,
        finished: &mut HashSet<String>,
    ) -> Result<(), String> {
        let theme_id = id(theme)?.to_string();
        if finished.contains(&theme_id) {
            return Ok(());
        }
        if !active.insert(theme_id.clone()) {
            return Err(format!("Bundle theme dependency cycle: {theme_id}"));
        }
        let mut media = HashSet::new();
        let mut dependencies = HashSet::new();
        references(theme, &mut media, &mut dependencies)?;
        for dependency in dependencies {
            let target = themes
                .iter()
                .find(|v| v["id"].as_str() == Some(&dependency))
                .ok_or("Missing theme dependency.")?;
            theme_cycle(target, themes, active, finished)?;
        }
        active.remove(&theme_id);
        finished.insert(theme_id);
        Ok(())
    }
    let mut active = HashSet::new();
    let mut finished = HashSet::new();
    for theme in &bundle.themes {
        theme_cycle(theme, &bundle.themes, &mut active, &mut finished)?;
    }
    Ok(())
}

pub fn write(bundle: &Bundle, destination: &Path) -> Result<(), String> {
    validate(bundle)?;
    let bytes = serde_json::to_vec(bundle).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("Encoded bundle exceeds 256 MiB.".into());
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|e| e.to_string())?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(destination);
        return Err(error.to_string());
    }
    Ok(())
}

pub fn read(source: &Path) -> Result<Bundle, String> {
    let file = fs::File::open(source).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE_BYTES {
        return Err("Bundle exceeds 256 MiB.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("Bundle exceeds 256 MiB.".into());
    }
    let bundle = serde_json::from_slice(&bytes).map_err(|e| format!("Invalid bundle: {e}"))?;
    validate(&bundle)?;
    Ok(bundle)
}

fn restore(value: &mut Value, root: &Path) {
    match value {
        Value::Object(object) => {
            if let Some(chart) = object.get_mut("chordChart") {
                for page in chart["pagePaths"]
                    .as_array_mut()
                    .expect("Validated chart pages")
                {
                    let token = page
                        .as_str()
                        .and_then(|v| v.strip_prefix("bundle://"))
                        .expect("Validated chart token");
                    *page = json!(root.join(token));
                }
            }
            for (key, child) in object {
                if asset_key(key) {
                    if let Some(token) = child.as_str().and_then(|v| v.strip_prefix("bundle://")) {
                        let path = root.join(token);
                        *child = if key.ends_with("Url") {
                            json!(url::Url::from_file_path(&path)
                                .expect("Absolute validated asset path")
                                .to_string())
                        } else {
                            json!(path)
                        };
                    }
                } else {
                    restore(child, root);
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                restore(child, root);
            }
        }
        _ => {}
    }
}

/// Title the bundled presentation is imported under.
pub fn title(bundle: &Bundle) -> Option<&str> {
    bundle.presentation["title"].as_str()
}

pub fn import(
    path: &Path,
    bundle: &Bundle,
    library_id: &str,
    duplicate: Duplicate,
) -> Result<Value, String> {
    validate(bundle)?;
    let workspace = crate::workspace::read_workspace(path)?;
    let mut presentation = bundle.presentation.clone();
    match (
        title(bundle).and_then(|title| same_title(&workspace, library_id, title)),
        duplicate,
    ) {
        (Some(_), Duplicate::UseExisting) => return Ok(workspace),
        (Some(existing), Duplicate::Replace) => presentation["id"] = json!(existing),
        _ => {}
    }
    let library_index = workspace["libraries"]
        .as_array()
        .and_then(|libraries| {
            libraries
                .iter()
                .position(|v| v["id"].as_str() == Some(library_id))
        })
        .ok_or("Selected destination library is missing.")?;
    if !workspace["libraries"][library_index]["itemIds"].is_array() {
        return Err("Destination library item index is required.".into());
    }
    let mut reused_themes = HashSet::new();
    for (field, imported) in [
        ("library", vec![presentation.clone()]),
        ("media", bundle.media.clone()),
        ("themes", bundle.themes.clone()),
    ] {
        let existing = workspace[field]
            .as_array()
            .ok_or("Workspace collection is invalid.")?;
        for item in imported {
            if field == "library" && presentation["id"] != bundle.presentation["id"] {
                continue;
            }
            if let Some(conflict) = existing.iter().find(|v| v["id"] == item["id"]) {
                let mut conflict = conflict.clone();
                if field == "themes" {
                    conflict
                        .as_object_mut()
                        .ok_or("Theme must be an object.")?
                        .remove("sourcePath");
                    // Paths differ per workspace; capturing against the bundle's assets compares file bytes instead.
                    let mut assets = bundle.assets.clone();
                    if conflict == item
                        || (capture(&mut conflict, &mut assets, &mut BTreeMap::new()).is_ok()
                            && conflict == item)
                    {
                        reused_themes.insert(id(&item)?.to_string());
                        continue;
                    }
                }
                return Err(format!("Bundle {field} id conflict: {}. Import into a separate workspace or remove the conflicting object explicitly.", id(&item)?));
            }
        }
    }
    let folder = fs::canonicalize(path.parent().ok_or("Workspace folder is required.")?)
        .map_err(|e| e.to_string())?;
    let media_root = fs::canonicalize(folder.join("Media")).map_err(|e| e.to_string())?;
    if !media_root.starts_with(&folder) {
        return Err("Bundle assets must remain inside the workspace.".into());
    }
    let container = media_root.join("Charts");
    if container.exists() {
        if !fs::canonicalize(&container)
            .map_err(|e| e.to_string())?
            .starts_with(&folder)
        {
            return Err("Bundle asset folder escapes the workspace.".into());
        }
    } else {
        fs::create_dir(&container).map_err(|e| e.to_string())?;
    }
    let root = container.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&root).map_err(|e| e.to_string())?;
    let result = (|| {
        for (key, bytes) in &bundle.assets {
            let target = root.join(key);
            fs::write(&target, bytes).map_err(|e| e.to_string())?;
            validate_asset(&target)?;
        }
        let mut patch = workspace.clone();
        let mut media = bundle.media.clone();
        let mut themes = bundle
            .themes
            .iter()
            .filter(|v| !reused_themes.contains(v["id"].as_str().unwrap_or_default()))
            .cloned()
            .collect::<Vec<_>>();
        restore(&mut presentation, &root);
        for item in media.iter_mut().chain(themes.iter_mut()) {
            restore(item, &root);
        }
        for item in &mut media {
            if let Some(url) = item["thumbnailUrl"]
                .as_str()
                .filter(|value| !value.is_empty())
            {
                let thumbnail = fs::canonicalize(local_path(url)?).map_err(|e| e.to_string())?;
                let relative = thumbnail
                    .strip_prefix(&media_root)
                    .map_err(|_| "Bundle thumbnail escapes media storage.")?;
                item["thumbnailFile"] = json!(relative.to_string_lossy().replace('\\', "/"));
            }
        }
        presentation["libraryId"] = json!(library_id);
        place(&mut patch, library_index, presentation)?;
        patch["media"]
            .as_array_mut()
            .ok_or("Media is invalid.")?
            .extend(media);
        patch["themes"]
            .as_array_mut()
            .ok_or("Themes are invalid.")?
            .extend(themes);
        crate::workspace::save_workspace(path, &patch)
    })();
    if result.is_err() {
        fs::remove_dir_all(&root)
            .map_err(|e| format!("Bundle import failed and asset cleanup failed: {e}"))?;
    }
    result
}
