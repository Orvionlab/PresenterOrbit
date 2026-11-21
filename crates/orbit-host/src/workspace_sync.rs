use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

const MANIFEST_NAME: &str = ".presenter-orbit-sync.json";
const MANIFEST_SCHEMA: &str = "presenter-orbit.sync";
const SNAPSHOT_SCHEMA: &str = "presenter-orbit.sync-snapshot";
const SYNC_ASSET_PREFIX: &str = "orbit-sync://";
const MAX_SYNC_DOCUMENT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Scope {
    Libraries,
    Playlists,
    Media,
    Themes,
}

impl Scope {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "libraries" => Ok(Self::Libraries),
            "playlists" => Ok(Self::Playlists),
            "media" => Ok(Self::Media),
            "themes" => Ok(Self::Themes),
            _ => Err(format!("Unsupported sync scope '{value}'.")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Libraries => "libraries",
            Self::Playlists => "playlists",
            Self::Media => "media",
            Self::Themes => "themes",
        }
    }

    fn fields(self) -> &'static [&'static str] {
        match self {
            Self::Libraries => &["libraries", "library"],
            Self::Playlists => &["playlist"],
            Self::Media => &["mediaLibraries", "media"],
            Self::Themes => &["themes"],
        }
    }

    fn collection_fields(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Libraries => &[("libraries", "libraries"), ("presentations", "library")],
            Self::Playlists => &[("playlists", "playlist")],
            Self::Media => &[("mediaLibraries", "mediaLibraries"), ("media", "media")],
            Self::Themes => &[("themes", "themes")],
        }
    }
}

#[derive(Clone)]
struct Snapshot {
    content: Value,
    assets: HashMap<String, PathBuf>,
}

struct Repository {
    generation: Option<String>,
    content: Value,
    assets: HashMap<String, PathBuf>,
    scopes: BTreeSet<String>,
}

struct Plan {
    merged: Value,
    entries: Vec<Value>,
    added: usize,
    updated: usize,
    conflicts: usize,
    unchanged: usize,
}

pub fn preview(
    workspace_path: &Path,
    repository: &Path,
    direction: &str,
    scopes: &[String],
    replace: bool,
) -> Result<Value, String> {
    let request = request(workspace_path, repository, direction, scopes, replace)?;
    let current = crate::workspace::read_workspace(&request.workspace_path)?;
    let workspace_snapshot = capture_workspace(&request.workspace_root, &current, &request.scopes)?;
    ensure_repository_unlocked(&request.repository_root)?;
    let repository_state = load_repository(&request.repository_root)?;
    if request.direction == "download" {
        require_repository_scopes(&repository_state, &request.scopes)?;
    }
    let (source, destination) = match request.direction.as_str() {
        "upload" => (&workspace_snapshot.content, &repository_state.content),
        "download" => (&repository_state.content, &workspace_snapshot.content),
        _ => unreachable!(),
    };
    let plan = merge(source, destination, &request.scopes, request.replace)?;
    validate_dependencies(
        &plan.merged,
        &current,
        &repository_state.content,
        &request.scopes,
        &request.direction,
    )?;
    let token = plan_token(&request, source, &repository_state, destination, &current)?;
    Ok(plan_value(&request, &plan, &token, source, destination))
}

pub fn apply(
    workspace_path: &Path,
    repository: &Path,
    direction: &str,
    scopes: &[String],
    replace: bool,
    expected_token: &str,
) -> Result<Value, String> {
    let request = request(workspace_path, repository, direction, scopes, replace)?;
    let _repository_lock = acquire_repository_lock(&request.repository_root)?;
    let current = crate::workspace::read_workspace(&request.workspace_path)?;
    let workspace_snapshot = capture_workspace(&request.workspace_root, &current, &request.scopes)?;
    let repository_state = load_repository(&request.repository_root)?;
    if request.direction == "download" {
        require_repository_scopes(&repository_state, &request.scopes)?;
    }
    let (source, destination) = match request.direction.as_str() {
        "upload" => (&workspace_snapshot.content, &repository_state.content),
        "download" => (&repository_state.content, &workspace_snapshot.content),
        _ => unreachable!(),
    };
    let plan = merge(source, destination, &request.scopes, request.replace)?;
    validate_dependencies(
        &plan.merged,
        &current,
        &repository_state.content,
        &request.scopes,
        &request.direction,
    )?;
    let token = plan_token(&request, source, &repository_state, destination, &current)?;
    if token != expected_token {
        return Err("Sync preview is stale. Preview the operation again.".into());
    }

    let needs_upload_scope_marker = request.direction == "upload"
        && request
            .scopes
            .iter()
            .any(|scope| !repository_state.scopes.contains(scope.name()));
    if plan.added == 0 && plan.updated == 0 && !needs_upload_scope_marker {
        return Ok(current);
    }

    match request.direction.as_str() {
        "upload" => {
            let mut content = repository_state.content.clone();
            copy_scopes(&mut content, &plan.merged, &request.scopes)?;
            let assets = combine_assets(&repository_state.assets, &workspace_snapshot.assets)?;
            let scopes = repository_state
                .scopes
                .iter()
                .cloned()
                .chain(request.scopes.iter().map(|scope| scope.name().to_string()))
                .collect::<BTreeSet<_>>();
            write_repository(
                &request.repository_root,
                &content,
                &assets,
                &scopes,
                &repository_state,
                &request,
                &workspace_snapshot,
            )?;
            Ok(current)
        }
        "download" => {
            let mut content = current.clone();
            let mut selected = plan.merged.clone();
            let asset_sources =
                combine_assets(&repository_state.assets, &workspace_snapshot.assets)?;
            let additional_assets = prepare_download(
                &mut selected,
                &request.workspace_root,
                &asset_sources,
                &workspace_snapshot.assets,
            )?;
            copy_scopes(&mut content, &selected, &request.scopes)?;
            let mut patch = Map::new();
            for scope in &request.scopes {
                for field in scope.fields() {
                    patch.insert((*field).to_string(), content[*field].clone());
                }
            }
            crate::workspace::save_workspace_with_assets(
                &request.workspace_path,
                &Value::Object(patch),
                &additional_assets,
            )
        }
        _ => unreachable!(),
    }
}

struct Request {
    workspace_path: PathBuf,
    workspace_root: PathBuf,
    repository_root: PathBuf,
    direction: String,
    scopes: Vec<Scope>,
    scope_names: Vec<String>,
    replace: bool,
}

fn request(
    workspace_path: &Path,
    repository: &Path,
    direction: &str,
    scopes: &[String],
    replace: bool,
) -> Result<Request, String> {
    if !matches!(direction, "upload" | "download") {
        return Err("Sync direction must be upload or download.".into());
    }
    if scopes.is_empty() {
        return Err("Select at least one sync scope.".into());
    }
    let mut parsed = Vec::with_capacity(scopes.len());
    let mut seen = HashSet::new();
    for name in scopes {
        let scope = Scope::parse(name)?;
        if !seen.insert(scope) {
            return Err(format!(
                "Sync scope '{}' was selected more than once.",
                scope.name()
            ));
        }
        parsed.push(scope);
    }
    reject_symlink_components(workspace_path)?;
    reject_symlink_components(repository)?;
    let workspace_path = workspace_path
        .canonicalize()
        .map_err(|error| format!("Workspace path is unavailable: {error}"))?;
    if workspace_path.file_name().and_then(|name| name.to_str()) != Some("workspace.json") {
        return Err("Workspace path must point to workspace.json.".into());
    }
    let workspace_root = workspace_path
        .parent()
        .ok_or("Workspace path has no parent folder.")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let metadata = fs::symlink_metadata(repository).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Sync repository must be an existing folder.".into());
    }
    let repository_root = repository
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if paths_overlap(&workspace_root, &repository_root) {
        return Err("Sync repository must be separate from the workspace.".into());
    }
    let scope_names = scopes.to_vec();
    Ok(Request {
        workspace_path,
        workspace_root,
        repository_root,
        direction: direction.to_string(),
        scopes: parsed,
        scope_names,
        replace,
    })
}

fn content_object() -> Value {
    json!({
        "libraries": [], "library": [], "playlist": [], "mediaLibraries": [],
        "media": [], "themes": []
    })
}

fn capture_workspace(root: &Path, workspace: &Value, scopes: &[Scope]) -> Result<Snapshot, String> {
    let mut content = content_object();
    for scope in scopes {
        for field in scope.fields() {
            let value = workspace
                .get(*field)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("Workspace {field} must be an array."))?;
            content[*field] = Value::Array(value.clone());
        }
    }
    let mut assets = HashMap::new();
    capture_references(&mut content, None, root, &mut assets)?;
    validate_content_ids(&content, scopes)?;
    Ok(Snapshot { content, assets })
}

fn capture_references(
    value: &mut Value,
    key: Option<&str>,
    root: &Path,
    assets: &mut HashMap<String, PathBuf>,
) -> Result<(), String> {
    if matches!(key, Some("pagePaths" | "itemFiles")) {
        let pages = value
            .as_array_mut()
            .ok_or_else(|| format!("{key:?} must be an array."))?;
        for page in pages {
            capture_reference(page, key.unwrap(), root, assets)?;
        }
        return Ok(());
    }
    if let Some(key) = key.filter(|key| is_asset_key(key)) {
        capture_reference(value, key, root, assets)?;
        return Ok(());
    }
    match value {
        Value::Object(object) => {
            let is_import_source = object
                .get("sourceType")
                .and_then(Value::as_str)
                .is_some_and(|kind| matches!(kind, "propresenter" | "presenter-orbit"));
            let is_library_import = is_import_source && object.contains_key("slides");
            if object.get("type").and_then(Value::as_str) == Some("smart")
                && object.get("sourcePath").and_then(Value::as_str).is_some()
            {
                return Err("Smart media library source folders are host-bound and cannot be synchronized. Convert the library to managed media first.".into());
            }
            // Synchronized media always restore as managed copies on the receiving host.
            object.remove("externalSource");
            for (child_key, child) in object.iter_mut() {
                if child_key == "sourcePath" && is_library_import {
                    continue;
                }
                capture_references(child, Some(child_key), root, assets)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                capture_references(child, None, root, assets)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn capture_reference(
    value: &mut Value,
    key: &str,
    root: &Path,
    assets: &mut HashMap<String, PathBuf>,
) -> Result<(), String> {
    let Some(text) = value.as_str() else {
        if value.is_null() {
            return Ok(());
        }
        return Err(format!("Asset reference {key} must be a path string."));
    };
    if text.is_empty() || text.starts_with("data:") {
        return Ok(());
    }
    let path = resolve_asset_path(text, key, root)?;
    let (marker, _) = asset_fingerprint(&path)?;
    if let Some(previous) = assets.get(&marker) {
        if !files_equal(previous, &path)? {
            return Err("Asset fingerprint collision detected.".into());
        }
    } else {
        assets.insert(marker.clone(), path);
    }
    *value = json!(marker);
    Ok(())
}

fn is_asset_key(key: &str) -> bool {
    matches!(
        key,
        "sourcePath"
            | "sourceUrl"
            | "assetFile"
            | "thumbnailFile"
            | "thumbnailPath"
            | "thumbnailUrl"
            | "mediaFillSourcePath"
            | "mediaFillUrl"
            | "mediaUrl"
            | "mediaSourcePath"
            | "mediaPath"
            | "backgroundMediaPath"
            | "backgroundMediaUrl"
            | "audioPath"
            | "audioUrl"
            | "logoImagePath"
            | "logoImageUrl"
            | "foregroundPath"
            | "chordChartPath"
            | "chordChartUrl"
            | "artwork"
            | "pagePaths"
            | "itemFiles"
    )
}

fn resolve_asset_path(text: &str, key: &str, root: &Path) -> Result<PathBuf, String> {
    let raw = if text.starts_with("file:") {
        url::Url::parse(text)
            .map_err(|error| format!("Invalid local asset URL in {key}: {error}"))?
            .to_file_path()
            .map_err(|_| format!("Invalid local asset URL in {key}: {text}"))?
    } else if text.contains("://") {
        return Err(format!(
            "Unsupported external asset reference in {key}: {text}"
        ));
    } else {
        let path = PathBuf::from(text);
        if path.is_absolute() {
            path
        } else {
            validate_relative_asset_path(&path, key)?;
            if matches!(key, "assetFile" | "thumbnailFile" | "itemFiles") {
                root.join("Media").join(path)
            } else {
                root.join(path)
            }
        }
    };
    reject_symlink_components(&raw)
        .map_err(|error| format!("Asset reference {key} is not safe: {error}"))?;
    let canonical = raw
        .canonicalize()
        .map_err(|error| format!("Asset reference {key} is unavailable: {error}"))?;
    let metadata = fs::metadata(&canonical).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(format!(
            "Asset reference {key} is not a file: {}",
            raw.display()
        ));
    }
    Ok(canonical)
}

fn validate_relative_asset_path(path: &Path, key: &str) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "Asset reference {key} must stay inside its workspace."
        ));
    }
    Ok(())
}

fn asset_fingerprint(path: &Path) -> Result<(String, u64), String> {
    reject_symlink_components(path)?;
    let before = fs::metadata(path).map_err(|error| error.to_string())?;
    if !before.is_file() {
        return Err(format!(
            "Sync asset is not a regular file: {}",
            path.display()
        ));
    }
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hash = 0xcbf29ce484222325u64;
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        }
    }
    let after = file.metadata().map_err(|error| error.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("Asset changed while sync was scanning. Preview again.".into());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin")
        .to_ascii_lowercase();
    Ok((
        format!(
            "{SYNC_ASSET_PREFIX}{}-{hash:016x}.{extension}",
            before.len()
        ),
        before.len(),
    ))
}

fn files_equal(left: &Path, right: &Path) -> Result<bool, String> {
    let left_metadata = fs::metadata(left).map_err(|error| error.to_string())?;
    let right_metadata = fs::metadata(right).map_err(|error| error.to_string())?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }
    let mut left = File::open(left).map_err(|error| error.to_string())?;
    let mut right = File::open(right).map_err(|error| error.to_string())?;
    let mut left_buffer = [0u8; 65536];
    let mut right_buffer = [0u8; 65536];
    loop {
        let left_count = left
            .read(&mut left_buffer)
            .map_err(|error| error.to_string())?;
        let right_count = right
            .read(&mut right_buffer)
            .map_err(|error| error.to_string())?;
        if left_count != right_count || left_buffer[..left_count] != right_buffer[..right_count] {
            return Ok(false);
        }
        if left_count == 0 {
            return Ok(true);
        }
    }
}

fn merge(
    source: &Value,
    destination: &Value,
    scopes: &[Scope],
    replace: bool,
) -> Result<Plan, String> {
    let mut merged = destination.clone();
    let mut entries = Vec::new();
    let (mut added, mut updated, mut conflicts, mut unchanged) = (0, 0, 0, 0);
    for scope in scopes {
        for (collection, field) in scope.collection_fields() {
            let source_items = source
                .get(*field)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("Sync {field} collection must be an array."))?;
            let target_items = merged
                .get_mut(*field)
                .and_then(Value::as_array_mut)
                .ok_or_else(|| format!("Sync destination {field} collection must be an array."))?;
            merge_collection(
                target_items,
                source_items,
                scope.name(),
                collection,
                replace,
                &mut entries,
                &mut added,
                &mut updated,
                &mut conflicts,
                &mut unchanged,
            )?;
        }
    }
    entries.sort_by(|left, right| {
        left["scope"]
            .as_str()
            .cmp(&right["scope"].as_str())
            .then_with(|| {
                left["collection"]
                    .as_str()
                    .cmp(&right["collection"].as_str())
            })
            .then_with(|| left["id"].as_str().cmp(&right["id"].as_str()))
    });
    Ok(Plan {
        merged,
        entries,
        added,
        updated,
        conflicts,
        unchanged,
    })
}

#[allow(clippy::too_many_arguments)]
fn merge_collection(
    destination: &mut Vec<Value>,
    source: &[Value],
    scope: &str,
    collection: &str,
    replace: bool,
    entries: &mut Vec<Value>,
    added: &mut usize,
    updated: &mut usize,
    conflicts: &mut usize,
    unchanged: &mut usize,
) -> Result<(), String> {
    let mut positions = HashMap::new();
    for (index, item) in destination.iter().enumerate() {
        let id = item_id(item, collection)?;
        if positions.insert(id.to_string(), index).is_some() {
            return Err(format!(
                "Sync destination {collection} contains duplicate id '{id}'."
            ));
        }
    }
    let mut source_ids = HashSet::new();
    for source_item in source {
        let id = item_id(source_item, collection)?;
        if !source_ids.insert(id.to_string()) {
            return Err(format!(
                "Sync source {collection} contains duplicate id '{id}'."
            ));
        }
        let status = match positions.get(id).copied() {
            None => {
                positions.insert(id.to_string(), destination.len());
                destination.push(source_item.clone());
                *added += 1;
                "added"
            }
            Some(index) if destination[index] == *source_item => {
                *unchanged += 1;
                "unchanged"
            }
            Some(index) if collection == "libraries" => {
                let previous = destination[index].clone();
                let mut unioned = previous.clone();
                merge_item_ids(&mut unioned, source_item)?;
                if record_without_item_ids(&unioned) == record_without_item_ids(source_item) {
                    if unioned == previous {
                        *unchanged += 1;
                        "unchanged"
                    } else {
                        destination[index] = unioned;
                        *updated += 1;
                        "updated"
                    }
                } else if replace {
                    let mut replacement = source_item.clone();
                    merge_item_ids(&mut replacement, &previous)?;
                    destination[index] = replacement;
                    *updated += 1;
                    "updated"
                } else {
                    if unioned != previous {
                        // Keep library membership additive even when its descriptive
                        // metadata conflicts and is deliberately retained locally.
                        destination[index] = unioned;
                        *updated += 1;
                    }
                    *conflicts += 1;
                    "conflict"
                }
            }
            Some(index) if replace => {
                destination[index] = source_item.clone();
                *updated += 1;
                "updated"
            }
            Some(_) => {
                *conflicts += 1;
                "conflict"
            }
        };
        entries.push(json!({
            "scope": scope,
            "collection": collection,
            "id": id,
            "title": source_item.get("title").and_then(Value::as_str)
                .filter(|title| !title.trim().is_empty()).unwrap_or(id),
            "status": status
        }));
    }
    Ok(())
}

fn item_id<'a>(value: &'a Value, collection: &str) -> Result<&'a str, String> {
    value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| format!("Sync {collection} record requires a stable id."))
}

fn merge_item_ids(target: &mut Value, source: &Value) -> Result<(), String> {
    let additions = source
        .get("itemIds")
        .and_then(Value::as_array)
        .ok_or("Presentation library itemIds must be an array.")?;
    let object = target
        .as_object_mut()
        .ok_or("Presentation library record must be an object.")?;
    let current = object
        .entry("itemIds")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("Presentation library itemIds must be an array.")?;
    let mut seen = current
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<HashSet<_>>();
    for item_id in additions {
        let id = item_id
            .as_str()
            .ok_or("Presentation library item id must be a string.")?;
        if seen.insert(id.to_string()) {
            current.push(json!(id));
        }
    }
    Ok(())
}

fn record_without_item_ids(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("itemIds");
    }
    value
}

fn validate_content_ids(content: &Value, scopes: &[Scope]) -> Result<(), String> {
    for scope in scopes {
        for (_, field) in scope.collection_fields() {
            let values = content
                .get(*field)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("Sync {field} collection must be an array."))?;
            let mut ids = HashSet::new();
            for value in values {
                let id = item_id(value, field)?;
                if !ids.insert(id) {
                    return Err(format!("Sync {field} collection has duplicate id '{id}'."));
                }
            }
        }
    }
    Ok(())
}

fn copy_scopes(target: &mut Value, source: &Value, scopes: &[Scope]) -> Result<(), String> {
    let target = target
        .as_object_mut()
        .ok_or("Sync content must be an object.")?;
    for scope in scopes {
        for field in scope.fields() {
            let value = source
                .get(*field)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("Sync {field} collection must be an array."))?;
            target.insert((*field).to_string(), Value::Array(value.clone()));
        }
    }
    Ok(())
}

fn validate_dependencies(
    merged: &Value,
    workspace: &Value,
    repository: &Value,
    scopes: &[Scope],
    direction: &str,
) -> Result<(), String> {
    let available = if direction == "upload" {
        repository
    } else {
        workspace
    };
    let mut full = available.clone();
    copy_scopes(&mut full, merged, scopes)?;
    let library_ids = ids(&full, "library")?;
    let media_ids = ids(&full, "media")?;
    let media_library_ids = ids(&full, "mediaLibraries")?;
    let mut dependencies = Dependencies::default();
    for scope in scopes {
        for field in scope.fields() {
            if let Some(values) = merged.get(*field).and_then(Value::as_array) {
                for value in values {
                    collect_dependencies(value, field, &mut dependencies)?;
                }
            }
        }
    }
    for id in dependencies.presentations {
        if !library_ids.contains(id.as_str()) {
            return Err(format!("Sync item references missing presentation '{id}'. Include libraries or add it at the destination."));
        }
    }
    for id in dependencies.media {
        if !media_ids.contains(id.as_str()) {
            return Err(format!("Sync item references missing media '{id}'. Include media or add it at the destination."));
        }
    }
    for id in dependencies.media_libraries {
        if !media_library_ids.contains(&id) {
            return Err(format!("Sync media references missing media library '{id}'. Include media or add it at the destination."));
        }
    }
    for (collection, id) in dependencies.workspace_targets {
        let exists = workspace
            .get(&collection)
            .and_then(Value::as_array)
            .is_some_and(|values| {
                values.iter().any(|value| {
                    value.get("id").and_then(Value::as_str) == Some(id.as_str())
                        || value.get("sourceId").and_then(Value::as_str) == Some(id.as_str())
                })
            });
        if !exists {
            return Err(format!(
                "Sync item references missing {collection} target '{id}'. Add that item to this workspace before syncing."
            ));
        }
    }
    for (theme, layout) in dependencies.themes {
        let Some(theme_value) = full["themes"]
            .as_array()
            .and_then(|themes| themes.iter().find(|value| value["id"] == theme))
        else {
            return Err(format!("Sync item references missing theme '{theme}'. Include themes or add it at the destination."));
        };
        if !theme_value["slides"]
            .as_array()
            .is_some_and(|slides| slides.iter().any(|slide| slide["id"] == layout))
        {
            return Err(format!("Sync item references missing theme layout '{theme}::{layout}'. Include themes or add it at the destination."));
        }
    }
    Ok(())
}

#[derive(Default)]
struct Dependencies {
    presentations: HashSet<String>,
    media: HashSet<String>,
    media_libraries: HashSet<String>,
    workspace_targets: HashSet<(String, String)>,
    themes: HashSet<(String, String)>,
}

/// A layer with its own source travels as an asset, so its media id need not be in the media bin.
pub(crate) fn has_own_media_source(object: &serde_json::Map<String, Value>, key: &str) -> bool {
    let source = match key {
        "mediaId" => "mediaSourcePath",
        "mediaFillId" => "mediaFillSourcePath",
        _ => return false,
    };
    object
        .get(source)
        .and_then(Value::as_str)
        .is_some_and(|path| !path.is_empty())
}

fn collect_dependencies(
    value: &Value,
    collection: &str,
    dependencies: &mut Dependencies,
) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            if collection == "media" {
                if let Some(id) = object
                    .get("libraryId")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                {
                    dependencies.media_libraries.insert(id.to_string());
                }
            }
            for key in [
                "mediaId",
                "mediaFillId",
                "audioId",
                "backgroundId",
                "mediaBackgroundId",
                "soundtrackMediaId",
            ] {
                if has_own_media_source(object, key) {
                    continue;
                }
                if let Some(id) = object
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                {
                    dependencies.media.insert(id.to_string());
                }
            }
            if object.get("type").and_then(Value::as_str) == Some("media") {
                if let Some(id) = object
                    .get("targetId")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                {
                    dependencies.media.insert(id.to_string());
                }
            }
            if object.get("type").and_then(Value::as_str) == Some("presentation") {
                if let Some(id) = object
                    .get("itemId")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                {
                    dependencies.presentations.insert(id.to_string());
                }
            }
            for (reference_key, collection) in [
                ("macroId", "macros"),
                ("timerId", "countdownTimers"),
                ("clearGroupId", "clearGroups"),
                ("communicationDeviceId", "communicationDevices"),
                ("messageId", "messages"),
                ("planId", "plans"),
                ("propId", "props"),
                ("maskId", "masks"),
                ("linkedPresentationId", "library"),
            ] {
                if let Some(id) = object
                    .get(reference_key)
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                {
                    if collection == "library" {
                        dependencies.presentations.insert(id.to_string());
                    } else {
                        dependencies
                            .workspace_targets
                            .insert((collection.to_string(), id.to_string()));
                    }
                }
            }
            if let Some(target) = object
                .get("targetId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            {
                let collection = match object.get("type").and_then(Value::as_str) {
                    Some("media") => Some("media"),
                    Some("macro") => Some("macros"),
                    Some("message") => Some("messages"),
                    Some("presentation") => Some("library"),
                    Some("playlist") => Some("playlist"),
                    Some("prop") => Some("props"),
                    Some("stage") => Some("stageLayouts"),
                    Some("audience-look") => Some("audienceLooks"),
                    Some("video-input") => Some("videoInputs"),
                    Some("audio-input") => Some("audioInputs"),
                    Some("mask") => Some("masks"),
                    Some("timer") => Some("countdownTimers"),
                    Some("clear-group") => Some("clearGroups"),
                    Some("communication") => Some("communicationDevices"),
                    _ => None,
                };
                match collection {
                    Some("media") => {
                        dependencies.media.insert(target.to_string());
                    }
                    Some("library") => {
                        dependencies.presentations.insert(target.to_string());
                    }
                    Some(collection) => {
                        dependencies
                            .workspace_targets
                            .insert((collection.to_string(), target.to_string()));
                    }
                    None => {
                        return Err(format!(
                            "Sync action has an unsupported target type for '{target}'."
                        ))
                    }
                }
            }
            if let Some(ids) = object.get("itemIds").and_then(Value::as_array) {
                let target = match collection {
                    "libraries" => Some(&mut dependencies.presentations),
                    "mediaLibraries" => Some(&mut dependencies.media),
                    _ => None,
                };
                if let Some(target) = target {
                    for id in ids {
                        target.insert(
                            id.as_str()
                                .filter(|id| !id.is_empty())
                                .ok_or("Library item id must be a nonempty string.")?
                                .to_string(),
                        );
                    }
                }
            }
            if let Some(assignment) = object
                .get("presentationThemeId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            {
                let (theme, layout) = assignment
                    .split_once("::")
                    .ok_or("Presentation theme assignment must include a theme and layout id.")?;
                if theme.is_empty() || layout.is_empty() {
                    return Err(
                        "Presentation theme assignment must include a theme and layout id.".into(),
                    );
                }
                dependencies
                    .themes
                    .insert((theme.to_string(), layout.to_string()));
            }
            for child in object.values() {
                collect_dependencies(child, collection, dependencies)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_dependencies(child, collection, dependencies)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn ids(content: &Value, field: &str) -> Result<HashSet<String>, String> {
    content
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Workspace {field} must be an array."))?
        .iter()
        .map(|item| item_id(item, field).map(str::to_string))
        .collect()
}

fn plan_value(
    request: &Request,
    plan: &Plan,
    token: &str,
    source: &Value,
    destination: &Value,
) -> Value {
    let mut asset_markers = HashSet::new();
    collect_markers(source, &mut asset_markers);
    collect_markers(destination, &mut asset_markers);
    json!({
        "direction": request.direction,
        "scopes": request.scope_names,
        "replace": request.replace,
        "token": token,
        "added": plan.added,
        "updated": plan.updated,
        "conflicts": plan.conflicts,
        "unchanged": plan.unchanged,
        "assetCount": asset_markers.len(),
        "entries": plan.entries
    })
}

fn plan_token(
    request: &Request,
    source: &Value,
    repository: &Repository,
    destination: &Value,
    workspace: &Value,
) -> Result<String, String> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    request.workspace_path.to_string_lossy().hash(&mut hasher);
    request.repository_root.to_string_lossy().hash(&mut hasher);
    request.direction.hash(&mut hasher);
    request.scope_names.hash(&mut hasher);
    request.replace.hash(&mut hasher);
    serde_json::to_vec(source)
        .map_err(|error| error.to_string())?
        .hash(&mut hasher);
    serde_json::to_vec(&repository.content)
        .map_err(|error| error.to_string())?
        .hash(&mut hasher);
    repository.generation.hash(&mut hasher);
    repository.scopes.hash(&mut hasher);
    serde_json::to_vec(destination)
        .map_err(|error| error.to_string())?
        .hash(&mut hasher);
    let mut dependency_ids = BTreeMap::new();
    for field in [
        "libraries",
        "library",
        "playlist",
        "mediaLibraries",
        "media",
        "themes",
    ] {
        let values = workspace
            .get(field)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("Workspace {field} collection must be an array."))?;
        let ids = values
            .iter()
            .map(|value| item_id(value, field).map(str::to_string))
            .collect::<Result<BTreeSet<_>, _>>()?;
        dependency_ids.insert(field, ids.into_iter().collect::<Vec<_>>());
    }
    serde_json::to_vec(&dependency_ids)
        .map_err(|error| error.to_string())?
        .hash(&mut hasher);
    Ok(format!("sync-{:016x}", hasher.finish()))
}

fn collect_markers(value: &Value, output: &mut HashSet<String>) {
    match value {
        Value::String(text) if text.starts_with(SYNC_ASSET_PREFIX) => {
            output.insert(text.clone());
        }
        Value::Object(object) => {
            for value in object.values() {
                collect_markers(value, output);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_markers(value, output);
            }
        }
        _ => {}
    }
}

pub(crate) fn verify_asset(path: &Path, expected: &str) -> Result<(), String> {
    let (actual, _) = asset_fingerprint(path)?;
    if actual != expected {
        return Err(format!(
            "Synchronization asset changed while copying: {}. Preview again.",
            path.display()
        ));
    }
    Ok(())
}

fn load_repository(root: &Path) -> Result<Repository, String> {
    let manifest_path = root.join(MANIFEST_NAME);
    let manifest_metadata = match fs::symlink_metadata(&manifest_path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    let Some(metadata) = manifest_metadata else {
        if fs::read_dir(root)
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .any(|entry| entry.file_name() != ".presenter-orbit-sync.lock")
        {
            return Err(
                "Sync repository is not empty and has no Presenter Orbit sync manifest.".into(),
            );
        }
        return Ok(Repository {
            generation: None,
            content: content_object(),
            assets: HashMap::new(),
            scopes: BTreeSet::new(),
        });
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Sync repository manifest must be a regular file.".into());
    }
    let manifest: Value = serde_json::from_slice(&read_repository_document(&manifest_path)?)
        .map_err(|error| format!("Sync repository manifest is invalid: {error}"))?;
    if manifest.get("schema").and_then(Value::as_str) != Some(MANIFEST_SCHEMA) {
        return Err("Sync repository manifest schema is unsupported.".into());
    }
    if manifest.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("Sync repository manifest version is unsupported.".into());
    }
    let generation = manifest
        .get("generation")
        .and_then(Value::as_str)
        .filter(|value| Uuid::parse_str(value).is_ok())
        .ok_or("Sync repository generation id is invalid.")?
        .to_string();
    let scope_names = manifest
        .get("scopes")
        .and_then(Value::as_array)
        .ok_or("Sync repository scopes are invalid.")?
        .iter()
        .map(|value| {
            let name = value
                .as_str()
                .ok_or("Sync repository scope must be a string.")?;
            Scope::parse(name)?;
            Ok::<String, String>(name.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let scopes = scope_names.iter().cloned().collect::<BTreeSet<_>>();
    if scopes.is_empty() || scopes.len() != scope_names.len() {
        return Err("Sync repository scopes must be nonempty and unique.".into());
    }
    let generation_root = root.join("generations").join(&generation);
    reject_symlink_components(&generation_root)?;
    let canonical_generation = generation_root
        .canonicalize()
        .map_err(|error| format!("Sync repository generation is unavailable: {error}"))?;
    if !canonical_generation.starts_with(root) {
        return Err("Sync repository generation escapes the repository folder.".into());
    }
    let snapshot_path = generation_root.join("snapshot.json");
    let snapshot_metadata =
        fs::symlink_metadata(&snapshot_path).map_err(|error| error.to_string())?;
    if snapshot_metadata.file_type().is_symlink() || !snapshot_metadata.is_file() {
        return Err("Sync repository snapshot must be a regular file.".into());
    }
    let snapshot: Value = serde_json::from_slice(&read_repository_document(&snapshot_path)?)
        .map_err(|error| format!("Sync repository snapshot is invalid: {error}"))?;
    if snapshot.get("schema").and_then(Value::as_str) != Some(SNAPSHOT_SCHEMA) {
        return Err("Sync repository snapshot schema is unsupported.".into());
    }
    if snapshot.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("Sync repository snapshot version is unsupported.".into());
    }
    let content = snapshot
        .get("content")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or("Sync repository content is invalid.")?;
    for field in [
        "libraries",
        "library",
        "playlist",
        "mediaLibraries",
        "media",
        "themes",
    ] {
        if !content.get(field).is_some_and(Value::is_array) {
            return Err(format!("Sync repository {field} collection is invalid."));
        }
    }
    for scope in [
        Scope::Libraries,
        Scope::Playlists,
        Scope::Media,
        Scope::Themes,
    ] {
        if !scopes.contains(scope.name())
            && scope
                .fields()
                .iter()
                .any(|field| !content[*field].as_array().is_some_and(Vec::is_empty))
        {
            return Err(format!(
                "Sync repository contains '{}' data without marking that scope available.",
                scope.name()
            ));
        }
    }
    validate_portable_references(&content, None)?;
    validate_content_ids(
        &content,
        &[
            Scope::Libraries,
            Scope::Playlists,
            Scope::Media,
            Scope::Themes,
        ],
    )?;
    let mut markers = HashSet::new();
    collect_markers(&content, &mut markers);
    let mut assets = HashMap::new();
    let assets_root = generation_root.join("assets");
    for marker in markers {
        let file_name = marker_file_name(&marker)?;
        let path = assets_root.join(file_name);
        reject_symlink_components(&path)?;
        verify_asset(&path, &marker)?;
        assets.insert(marker, path);
    }
    Ok(Repository {
        generation: Some(generation),
        content,
        assets,
        scopes,
    })
}

fn read_repository_document(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Sync repository documents must be regular files.".into());
    }
    if metadata.len() > MAX_SYNC_DOCUMENT_BYTES {
        return Err("Sync repository document exceeds the 32 MiB limit.".into());
    }
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_SYNC_DOCUMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_SYNC_DOCUMENT_BYTES {
        return Err("Sync repository document exceeds the 32 MiB limit.".into());
    }
    Ok(bytes)
}

fn validate_portable_references(value: &Value, key: Option<&str>) -> Result<(), String> {
    if matches!(key, Some("pagePaths" | "itemFiles")) {
        let values = value
            .as_array()
            .ok_or_else(|| format!("Repository {key:?} must be an array."))?;
        for item in values {
            validate_portable_references(item, Some("sourcePath"))?;
        }
        return Ok(());
    }
    if let Some(key) = key.filter(|key| is_asset_key(key)) {
        let Some(text) = value.as_str() else {
            return if value.is_null() {
                Ok(())
            } else {
                Err(format!(
                    "Repository asset reference {key} must be a string."
                ))
            };
        };
        if text.is_empty() || text.starts_with("data:") || text.starts_with(SYNC_ASSET_PREFIX) {
            return Ok(());
        }
        return Err(format!(
            "Sync repository contains a nonportable asset reference in {key}. Preview and upload it again from the source workspace."
        ));
    }
    match value {
        Value::Object(object) => {
            let is_import_source = object.contains_key("slides")
                && object
                    .get("sourceType")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| matches!(kind, "propresenter" | "presenter-orbit"));
            for (child_key, child) in object {
                if child_key == "sourcePath" && is_import_source {
                    continue;
                }
                validate_portable_references(child, Some(child_key))?;
            }
        }
        Value::Array(values) => {
            for child in values {
                validate_portable_references(child, None)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn require_repository_scopes(repository: &Repository, scopes: &[Scope]) -> Result<(), String> {
    for scope in scopes {
        if !repository.scopes.contains(scope.name()) {
            return Err(format!(
                "Sync repository has no '{}' data. Upload that scope first.",
                scope.name()
            ));
        }
    }
    Ok(())
}

fn ensure_repository_unlocked(root: &Path) -> Result<(), String> {
    match fs::symlink_metadata(root.join(".presenter-orbit-sync.lock")) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("Sync repository lock cannot be a symbolic link.".into())
        }
        Ok(_) => Err("Sync repository is locked by another or interrupted operation. Remove the lock only after confirming no sync operation is active.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

struct RepositoryLock {
    path: PathBuf,
    nonce: String,
}

impl Drop for RepositoryLock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).ok().as_deref() == Some(self.nonce.as_str()) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn acquire_repository_lock(root: &Path) -> Result<RepositoryLock, String> {
    let path = root.join(".presenter-orbit-sync.lock");
    let nonce = Uuid::new_v4().to_string();
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err("Sync repository is locked by another or interrupted operation. Remove the lock only after confirming no sync operation is active.".into());
        }
        Err(error) => return Err(format!("Could not lock sync repository: {error}")),
    };
    if let Err(error) = file
        .write_all(nonce.as_bytes())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        if fs::read_to_string(&path).ok().as_deref() == Some(nonce.as_str()) {
            let _ = fs::remove_file(&path);
        }
        return Err(format!(
            "Could not initialize sync repository lock: {error}"
        ));
    }
    Ok(RepositoryLock { path, nonce })
}

fn combine_assets(
    left: &HashMap<String, PathBuf>,
    right: &HashMap<String, PathBuf>,
) -> Result<HashMap<String, PathBuf>, String> {
    let mut assets = left.clone();
    for (marker, path) in right {
        if let Some(previous) = assets.get(marker) {
            if !files_equal(previous, path)? {
                return Err("Synchronization asset fingerprint collision detected.".into());
            }
        } else {
            assets.insert(marker.clone(), path.clone());
        }
    }
    Ok(assets)
}

fn write_repository(
    root: &Path,
    content: &Value,
    assets: &HashMap<String, PathBuf>,
    scopes: &BTreeSet<String>,
    previous: &Repository,
    request: &Request,
    source: &Snapshot,
) -> Result<(), String> {
    let generation = Uuid::new_v4().to_string();
    let staging = root.join(format!(".presenter-orbit-stage-{generation}"));
    let generation_path = root.join("generations").join(&generation);
    let generations_root = root.join("generations");
    let generations_preexisted = generations_root.exists();
    let result = (|| {
        if let Ok(metadata) = fs::symlink_metadata(&generations_root) {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("Sync repository generations path must be a regular folder.".into());
            }
        }
        fs::create_dir_all(&generations_root).map_err(|error| error.to_string())?;
        fs::create_dir(&staging).map_err(|error| error.to_string())?;
        let assets_root = staging.join("assets");
        fs::create_dir(&assets_root).map_err(|error| error.to_string())?;
        let mut markers = HashSet::new();
        collect_markers(content, &mut markers);
        for marker in markers {
            let source_path = assets
                .get(&marker)
                .ok_or_else(|| format!("Sync asset is missing for {marker}."))?;
            reject_symlink_components(source_path)?;
            verify_asset(source_path, &marker)?;
            let target = assets_root.join(marker_file_name(&marker)?);
            fs::copy(source_path, &target).map_err(|error| error.to_string())?;
            verify_asset(source_path, &marker)?;
            verify_asset(&target, &marker)?;
            fs::OpenOptions::new()
                .write(true)
                .open(&target)
                .and_then(|file| file.sync_all())
                .map_err(|error| error.to_string())?;
        }
        let snapshot = json!({"schema": SNAPSHOT_SCHEMA, "version": 1, "content": content});
        let snapshot_bytes =
            serde_json::to_vec_pretty(&snapshot).map_err(|error| error.to_string())?;
        if snapshot_bytes.len() as u64 > MAX_SYNC_DOCUMENT_BYTES {
            return Err("Sync repository snapshot exceeds the 32 MiB limit.".into());
        }
        let mut snapshot_file =
            File::create(staging.join("snapshot.json")).map_err(|error| error.to_string())?;
        snapshot_file
            .write_all(&snapshot_bytes)
            .map_err(|error| error.to_string())?;
        snapshot_file
            .sync_all()
            .map_err(|error| error.to_string())?;
        drop(snapshot_file);

        let now = crate::workspace::read_workspace(&request.workspace_path)?;
        let rescanned = capture_workspace(&request.workspace_root, &now, &request.scopes)?;
        if rescanned.content != source.content {
            return Err("Workspace changed while sync assets were copied. Preview again.".into());
        }
        if previous.generation.is_some() {
            let repository_now = load_repository(root)?;
            if repository_now.generation != previous.generation
                || repository_now.scopes != previous.scopes
                || repository_now.content != previous.content
            {
                return Err(
                    "Sync repository changed while assets were copied. Preview again.".into(),
                );
            }
        } else if root.join(MANIFEST_NAME).exists() {
            return Err("Sync repository changed while assets were copied. Preview again.".into());
        }

        fs::rename(&staging, &generation_path).map_err(|error| error.to_string())?;
        let manifest = json!({
            "schema": MANIFEST_SCHEMA,
            "version": 1,
            "generation": generation,
            "scopes": scopes.iter().collect::<Vec<_>>(),
        });
        replace_repository_manifest(root, &manifest)?;
        Ok(())
    })();
    if result.is_err() {
        if staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        let published_generation = fs::read(root.join(MANIFEST_NAME))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|manifest| {
                manifest
                    .get("generation")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        if published_generation.as_deref() != Some(generation.as_str()) && generation_path.exists()
        {
            let _ = fs::remove_dir_all(&generation_path);
        }
        if !generations_preexisted {
            let _ = fs::remove_dir(&generations_root);
        }
    }
    result
}

fn replace_repository_manifest(root: &Path, manifest: &Value) -> Result<(), String> {
    let id = Uuid::new_v4().to_string();
    let temporary = root.join(format!("{MANIFEST_NAME}.{id}.tmp"));
    let target = root.join(MANIFEST_NAME);
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|error| error.to_string())?;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        replace_file_atomically(&temporary, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
pub(crate) fn replace_file_atomically(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination)
        .map_err(|error| format!("Could not atomically replace file: {error}"))
}

#[cfg(windows)]
pub(crate) fn replace_file_atomically(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(format!(
            "Could not atomically replace file: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

fn marker_file_name(marker: &str) -> Result<&str, String> {
    let name = marker
        .strip_prefix(SYNC_ASSET_PREFIX)
        .ok_or("Sync asset marker is invalid.")?;
    if name.is_empty()
        || name
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':'))
        || Path::new(name).components().count() != 1
        || !matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        )
    {
        return Err("Sync asset marker contains an invalid file name.".into());
    }
    Ok(name)
}

fn prepare_download(
    content: &mut Value,
    workspace_root: &Path,
    assets: &HashMap<String, PathBuf>,
    local_assets: &HashMap<String, PathBuf>,
) -> Result<Vec<crate::workspace::SyncWorkspaceAsset>, String> {
    let mut staged = BTreeMap::<PathBuf, crate::workspace::SyncWorkspaceAsset>::new();
    remap_download_assets(
        content,
        None,
        false,
        workspace_root,
        assets,
        local_assets,
        &mut staged,
    )?;
    Ok(staged.into_values().collect())
}

fn remap_download_assets(
    value: &mut Value,
    key: Option<&str>,
    in_chart: bool,
    workspace_root: &Path,
    assets: &HashMap<String, PathBuf>,
    local_assets: &HashMap<String, PathBuf>,
    staged: &mut BTreeMap<PathBuf, crate::workspace::SyncWorkspaceAsset>,
) -> Result<(), String> {
    if matches!(key, Some("pagePaths" | "itemFiles")) {
        let values = value
            .as_array_mut()
            .ok_or_else(|| format!("{key:?} must be an array."))?;
        for item in values {
            remap_download_asset(
                item,
                key.unwrap(),
                in_chart,
                workspace_root,
                assets,
                local_assets,
                staged,
            )?;
        }
        return Ok(());
    }
    if let Some(key) = key.filter(|key| is_asset_key(key)) {
        remap_download_asset(
            value,
            key,
            in_chart,
            workspace_root,
            assets,
            local_assets,
            staged,
        )?;
        return Ok(());
    }
    match value {
        Value::Object(object) => {
            let chart = in_chart || key == Some("chordChart");
            for (child_key, child) in object.iter_mut() {
                remap_download_assets(
                    child,
                    Some(child_key),
                    chart,
                    workspace_root,
                    assets,
                    local_assets,
                    staged,
                )?;
            }
        }
        Value::Array(values) => {
            for child in values {
                remap_download_assets(
                    child,
                    None,
                    in_chart,
                    workspace_root,
                    assets,
                    local_assets,
                    staged,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn remap_download_asset(
    value: &mut Value,
    key: &str,
    in_chart: bool,
    workspace_root: &Path,
    assets: &HashMap<String, PathBuf>,
    local_assets: &HashMap<String, PathBuf>,
    staged: &mut BTreeMap<PathBuf, crate::workspace::SyncWorkspaceAsset>,
) -> Result<(), String> {
    let Some(marker) = value.as_str() else {
        return if value.is_null() {
            Ok(())
        } else {
            Err(format!("Asset reference {key} must be a path string."))
        };
    };
    if !marker.starts_with(SYNC_ASSET_PREFIX) {
        return Ok(());
    }
    if let Some(path) = local_assets.get(marker).filter(|path| {
        (!in_chart || path.starts_with(workspace_root.join("Media/Charts")))
            && (!matches!(key, "assetFile" | "thumbnailFile" | "itemFiles")
                || path.starts_with(workspace_root.join("Media")))
    }) {
        let media_root = workspace_root.join("Media");
        let output = if matches!(key, "assetFile" | "thumbnailFile" | "itemFiles") {
            path.strip_prefix(&media_root)
                .map_err(|_| {
                    format!(
                        "Workspace asset {marker} is outside Media and cannot be stored in {key}."
                    )
                })?
                .to_string_lossy()
                .replace('\\', "/")
        } else if key.ends_with("Url") || key == "sourceUrl" {
            url::Url::from_file_path(path)
                .map_err(|_| format!("Could not create local URL for workspace asset {marker}."))?
                .to_string()
        } else {
            path.to_string_lossy().into_owned()
        };
        *value = json!(output);
        return Ok(());
    }
    let source_path = assets
        .get(marker)
        .ok_or_else(|| format!("Sync repository asset is missing: {marker}."))?;
    verify_asset(source_path, marker)?;
    let file_name = marker_file_name(marker)?;
    let relative_path = if in_chart {
        PathBuf::from("Media/Charts/Sync").join(file_name)
    } else {
        PathBuf::from("Media/Assets/Sync").join(file_name)
    };
    if let Some(previous) = staged.get(&relative_path) {
        if previous.expected_fingerprint != marker
            || !files_equal(&previous.source_path, source_path)?
        {
            return Err("Sync asset fingerprint collision detected.".into());
        }
    } else {
        staged.insert(
            relative_path.clone(),
            crate::workspace::SyncWorkspaceAsset {
                relative_path: relative_path.clone(),
                source_path: source_path.clone(),
                expected_fingerprint: marker.to_string(),
            },
        );
    }
    let absolute = workspace_root.join(&relative_path);
    let asset_file = if in_chart {
        absolute.to_string_lossy().into_owned()
    } else {
        PathBuf::from("Assets/Sync")
            .join(file_name)
            .to_string_lossy()
            .into_owned()
    };
    let output = if matches!(key, "assetFile" | "thumbnailFile" | "itemFiles") {
        asset_file
    } else if key.ends_with("Url") || key == "sourceUrl" {
        url::Url::from_file_path(&absolute)
            .map_err(|_| format!("Could not create local URL for synchronized asset {marker}."))?
            .to_string()
    } else {
        absolute.to_string_lossy().into_owned()
    };
    *value = json!(output);
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if !matches!(component, Component::Normal(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Path cannot pass through a symbolic link: {}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}
