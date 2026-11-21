use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufReader, Read},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

#[cfg(windows)]
use std::fs::OpenOptions;

use chrono::Utc;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::settings;
pub use crate::workspace_import_selection::ImportSelection;

#[path = "propresenter_proto.rs"]
pub(crate) mod propresenter_proto;

#[path = "import_export_report.rs"]
mod import_export_report;
pub use import_export_report::reviewed_export;

#[path = "content_themes.rs"]
pub(crate) mod content_themes;

#[path = "slide_text_edit.rs"]
pub(crate) mod slide_text_edit;

#[path = "propresenter_archive.rs"]
mod propresenter_archive;
pub use propresenter_archive::archive_titles as pro_presenter_archive_titles;
pub use propresenter_archive::import_archive as import_pro_presenter_archive;

#[path = "pptx_import.rs"]
mod pptx_import;
pub use pptx_import::import as import_pptx;
pub use pptx_import::title as pptx_title;

#[path = "pptx_package.rs"]
mod pptx_package;

#[path = "pptx_slide.rs"]
mod pptx_slide;

#[path = "pptx_text.rs"]
mod pptx_text;

#[path = "propresenter_assets.rs"]
mod propresenter_assets;

#[path = "propresenter_charts.rs"]
mod propresenter_charts;

#[path = "screen_preview_import.rs"]
mod screen_preview_import;

#[path = "timeline_soundtrack_import.rs"]
mod timeline_soundtrack_import;

const WORKSPACE_FILE_NAME: &str = "workspace.json";
pub(crate) const WORKSPACE_TRANSACTION_DIRECTORY: &str = ".presenter-orbit-write";
const WORKSPACE_TRANSACTION_JOURNAL: &str = "journal.json";
const WORKSPACE_TRANSACTION_COMMITTED: &str = "committed";
const MAX_IMPORT_WARNINGS: usize = 64;

#[derive(serde::Deserialize, serde::Serialize)]
struct WorkspaceTransactionJournal {
    entries: Vec<WorkspaceTransactionEntry>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct WorkspaceTransactionEntry {
    path: String,
    existed: bool,
}

#[derive(Default)]
struct ImportStats {
    scanned_file_count: u64,
    skipped_file_count: u64,
    warnings: Vec<String>,
    newer_application_versions: Vec<String>,
}

impl ImportStats {
    fn findings(&mut self, path: &Path, value: &Value) {
        import_export_report::import_findings(
            value,
            &path.display().to_string(),
            &mut self.warnings,
        );
    }

    fn scanned(&mut self) {
        self.scanned_file_count = self.scanned_file_count.saturating_add(1);
    }

    /// Reports once per ProPresenter release newer than the verified one, whose added fields
    /// the decoder skips.
    fn source_version(&mut self, path: &Path, bytes: &[u8]) {
        let Some(version) = propresenter_proto::newer_application_version(bytes) else {
            return;
        };
        if self.newer_application_versions.contains(&version) {
            return;
        }
        self.warnings.push(format!(
            "{}: written by {version}, newer than the newest verified ProPresenter version; fields added after it are skipped",
            path.display()
        ));
        self.newer_application_versions.push(version);
    }

    fn warn(&mut self, path: &Path, finding: &str) {
        if self.warnings.len() < MAX_IMPORT_WARNINGS {
            self.warnings.push(format!("{}: {finding}", path.display()));
        }
    }

    fn skipped(&mut self, path: &Path, reason: &str) {
        self.skipped_file_count = self.skipped_file_count.saturating_add(1);
        if self.warnings.len() < MAX_IMPORT_WARNINGS {
            self.warnings.push(format!("{}: {reason}", path.display()));
        }
    }
}

pub fn workspace_path(folder: &Path) -> PathBuf {
    folder.join(WORKSPACE_FILE_NAME)
}

pub fn ensure_workspace(path: &Path) -> Result<(), String> {
    recover_pending_workspace_write(path)?;
    if path.exists() {
        ensure_workspace_settings(path)?;
        return Ok(());
    }

    let folder = path
        .parent()
        .ok_or_else(|| "Workspace path has no parent folder.".to_string())?;
    for name in [
        "Libraries",
        "Playlists",
        "Media",
        "Themes",
        "Masks",
        "Configuration",
    ] {
        fs::create_dir_all(folder.join(name)).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(folder.join("Media").join("Assets")).map_err(|error| error.to_string())?;
    fs::create_dir_all(folder.join("Libraries").join("Default"))
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(folder.join("Configuration").join("Props"))
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(folder.join("Configuration").join("Stage"))
        .map_err(|error| error.to_string())?;

    write_json(
        &folder.join("Libraries/index.json"),
        json!({
            "libraries": [{ "id": "default", "title": "Default", "folder": "Default" }]
        }),
    )?;
    write_json(
        &folder.join("Libraries/Default/library.json"),
        json!({
            "id": "default", "title": "Default", "itemIds": [], "items": []
        }),
    )?;
    write_indexed(
        folder,
        "Playlists",
        "playlists",
        vec![json!({ "id": "default", "title": "Default", "itemIds": [], "items": [] })],
    )?;
    write_json(
        &folder.join("Media/index.json"),
        json!({ "libraries": [default_audio_playlist(), default_graphics_playlist()], "media": [] }),
    )?;
    write_themes(folder, crate::sample_content::install(folder)?)?;
    write_masks(folder, vec![new_mask_document("mask", "Mask")])?;
    write_json(
        &folder.join("Configuration/Props/Props"),
        json!({
            "schemaVersion": 1, "props": [], "propFolders": [], "countdownTimers": crate::sample_content::countdown_timers()?,
            "macros": [], "macroCollections": [], "messages": [], "plans": [], "scheduledEvents": []
        }),
    )?;
    write_json(
        &folder.join("Configuration/Stage/Stage.json"),
        json!({
            "schemaVersion": 1, "stageLayouts": crate::sample_content::stage_layouts()?, "stageScreenAssignments": [],
            "stageMessage": { "text": "", "state": "cleared" },
            "audienceLooks": [{ "id": "default-look", "title": "Default", "screenLooks": [] }],
            "videoInputs": [], "audioInputs": [], "recordingPresets": [], "tickers": []
        }),
    )?;
    ensure_workspace_settings(path)?;
    write_json(
        path,
        json!({
            "schema": "presenter-orbit.workspace", "version": 1, "id": Uuid::new_v4().to_string(),
            "title": "Presenter Orbit Workspace", "workspacePath": path, "createdAt": "1970-01-01T00:00:00.000Z",
            "updatedAt": "1970-01-01T00:00:00.000Z", "importedFrom": null,
            "storage": { "layout": "folders", "libraryFolder": "Libraries", "playlistsFolder": "Playlists", "mediaFolder": "Media", "themesFolder": "Themes", "masksFolder": "Masks", "propsFolder": "Configuration/Props" },
            "stats": { "scannedFileCount": 0, "skippedFileCount": 0 },
            "importWarnings": [], "mediaEffectPresets": [], "transitionPresets": [], "triggerBindings": [], "communicationDevices": [], "clearGroups": []
            , "lastUsedPresentationThemeId": "basic-black::general-text-slide",
            "ccliTemplate": crate::ccli_template::default_slide()
        }),
    )
}

fn default_audio_playlist() -> Value {
    json!({ "id": "default-audio", "title": "Default", "type": "playlist", "mediaType": "audio", "itemFiles": [] })
}

fn default_graphics_playlist() -> Value {
    json!({ "id": "default-graphics", "title": "Default", "type": "playlist", "mediaType": "graphics", "itemFiles": [] })
}

fn ensure_workspace_settings(path: &Path) -> Result<(), String> {
    let folder = path
        .parent()
        .ok_or_else(|| "Workspace path has no parent folder.".to_string())?;
    let settings_path = folder.join("Configuration/Settings.json");

    if settings_path.exists() {
        return Ok(());
    }

    write_json(
        &settings_path,
        json!({
            "schemaVersion": 1, "screenConfig": {}, "countdownTimers": crate::sample_content::countdown_timers()?, "groupSettings": {}, "groups": [], "keyMappings": {}
        }),
    )
}

pub fn read_workspace(path: &Path) -> Result<Value, String> {
    recover_pending_workspace_write(path)?;
    let mut workspace = read_json(path)?;
    let folder = path
        .parent()
        .ok_or_else(|| "Workspace path has no parent folder.".to_string())?;
    let root = workspace
        .as_object_mut()
        .ok_or_else(|| "Workspace manifest must be an object.".to_string())?;
    validate_manifest(root)?;
    validate_clear_groups(root.get("clearGroups"))?;
    if let Some(template) = root.get("ccliTemplate") {
        crate::ccli_template::validate(template)?;
    }
    root.insert("workspacePath".to_string(), json!(path));

    let (libraries, library) = read_libraries(folder)?;
    root.insert("libraries".to_string(), Value::Array(libraries));
    root.insert("library".to_string(), Value::Array(library));
    let playlists = normalize_playlists_value(&read_indexed(folder, "Playlists", "playlists")?);
    root.insert("playlist".to_string(), playlists);
    let (media_libraries, media) = read_media(folder)?;
    root.insert("mediaLibraries".to_string(), Value::Array(media_libraries));
    root.insert("media".to_string(), Value::Array(media));
    root.insert("themes".to_string(), read_themes(folder)?);
    root.insert("masks".to_string(), read_masks(folder)?);
    merge_document(
        root,
        &read_json(&folder.join("Configuration/Props/Props"))?,
        &[
            "props",
            "propFolders",
            "countdownTimers",
            "macros",
            "macroCollections",
            "messages",
            "plans",
            "scheduledEvents",
        ],
    )?;
    if let Some(collections) = root.get("macroCollections") {
        if !collections.is_array() {
            return Err("macroCollections must be an array.".to_string());
        }
    }
    merge_document(
        root,
        &read_json(&folder.join("Configuration/Stage/Stage.json"))?,
        &[
            "stageLayouts",
            "stageScreenAssignments",
            "stageMessage",
            "audienceLooks",
            "videoInputs",
            "audioInputs",
            "recordingPresets",
            "tickers",
        ],
    )?;
    let stage_layouts = root
        .get_mut("stageLayouts")
        .and_then(Value::as_array_mut)
        .ok_or("Workspace Stage layouts must be an array.")?;
    normalize_imported_canvas_documents(stage_layouts)?;
    merge_document(
        root,
        &read_json(&folder.join("Configuration/Settings.json"))?,
        &[
            "screenConfig",
            "countdownTimers",
            "groupSettings",
            "groups",
            "keyMappings",
        ],
    )?;
    let screen_config = settings::normalize_workspace_screen_config_value(
        root.get("screenConfig").unwrap_or(&Value::Null),
    );
    crate::output_canvas::validate_screen_config_map(&screen_config)?;
    root.insert("screenConfig".to_string(), screen_config);
    let group_settings = settings::normalize_workspace_group_settings_value(
        root.get("groupSettings").unwrap_or(&Value::Null),
    );
    root.insert("groupSettings".to_string(), group_settings);
    let key_mappings = settings::normalize_workspace_key_mappings_value(
        root.get("keyMappings").unwrap_or(&Value::Null),
    );
    root.insert("keyMappings".to_string(), key_mappings);
    crate::native_ticker::validate_collection(
        root.get("tickers")
            .ok_or("Workspace tickers are required.")?,
    )?;
    validate_capture_presets(root.get("recordingPresets"))?;
    crate::video_input::validate_collection(
        root.get("videoInputs")
            .ok_or("Workspace videoInputs are required.")?,
    )?;
    crate::audio_input_config::validate_collection(
        root.get("audioInputs")
            .ok_or("Workspace audioInputs are required.")?,
    )?;
    validate_media_effect_presets(root.get("mediaEffectPresets"))?;
    validate_transition_presets(root.get("transitionPresets"))?;
    validate_layer_transitions(root)?;
    crate::native_compositor::validate_object_links(&workspace)?;
    crate::native_compositor::validate_stage_preview_fields(&workspace)?;
    validate_workspace_charts(path, &workspace)?;
    crate::macro_collections::validate_orders(&workspace)?;
    validate_audio_marker_dependencies(
        workspace
            .as_object()
            .ok_or("Workspace must be an object.")?,
    )?;
    Ok(workspace)
}

pub fn merge_missing_workspace_group_settings(workspace: &mut Value, current: &Value) {
    let Some(document) = workspace.as_object_mut() else {
        return;
    };
    if document
        .get("groupSettings")
        .and_then(Value::as_object)
        .is_some_and(|groups| !groups.is_empty())
    {
        return;
    }
    let current = settings::normalize_group_settings_value(current);
    let groups = document
        .get("groups")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let library = document
        .get("library")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let merged = settings::merge_imported_group_settings_value(&current, groups, library);
    if merged != current {
        document.insert("groupSettings".to_string(), merged);
    }
}

/// Assign a presentation slide to a newly-created group in the supplied
/// workspace snapshot.
///
/// This mirrors the existing `mutateItem/setSlideGroup` semantics: the
/// canonical `groupId` is written, any legacy group metadata is left intact
/// while the canonical field takes precedence, and the denormalized item
/// `groups` list is removed. The caller can then persist the changed snapshot
/// together with the group-settings mutation as one transaction.
pub fn assign_group_reference(
    workspace: &mut Value,
    item_id: &str,
    slide_id: &str,
    group_id: &str,
) -> Result<bool, String> {
    let mut next_workspace = workspace.clone();
    let changed =
        assign_group_reference_in_place(&mut next_workspace, item_id, slide_id, group_id)?;
    *workspace = next_workspace;
    Ok(changed)
}

fn assign_group_reference_in_place(
    workspace: &mut Value,
    item_id: &str,
    slide_id: &str,
    group_id: &str,
) -> Result<bool, String> {
    let item_id = item_id.trim();
    let slide_id = slide_id.trim();
    let group_id = group_id.trim();
    if item_id.is_empty() {
        return Err("Group assignment item id is required.".to_string());
    }
    if slide_id.is_empty() {
        return Err("Group assignment slide id is required.".to_string());
    }
    if group_id.is_empty() {
        return Err("Group assignment group id is required.".to_string());
    }

    let document = workspace
        .as_object_mut()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let library = document
        .get_mut("library")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "Workspace library must be an array.".to_string())?;
    let item = library
        .iter_mut()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(item_id))
        .ok_or_else(|| format!("Presentation item '{item_id}' was not found."))?;
    let original_item = item.clone();
    let mut slides = presentation_item_slides(item)?;
    let slide_index = find_presentation_slide_index(&slides, slide_id)?;
    let slide = slides
        .get_mut(slide_index)
        .ok_or_else(|| format!("Presentation slide '{slide_id}' was not found."))?;
    let slide_object = slide
        .as_object_mut()
        .ok_or_else(|| "Presentation slides must be objects.".to_string())?;
    if slide_object.get("groupId").and_then(Value::as_str) != Some(group_id) {
        slide_object.insert("groupId".to_string(), json!(group_id));
    }
    set_presentation_item_slides(item, slides)?;
    normalize_presentation_item_groups(item);
    Ok(*item != original_item)
}

/// Reset every slide assignment that resolves to `group_id`.
///
/// ProPresenter imports can carry either a canonical `groupId` or a legacy
/// `groupName`; the renderer resolves both through the same slug rule. The
/// `groupId` property wins when it is present, including when it is empty, so
/// this helper deliberately mirrors that precedence. The renderer's delete
/// path supplies the localized labels; requiring one label for every matched
/// slide keeps the native transaction identical to the optimistic state.
pub fn reset_group_references(
    workspace: &mut Value,
    group_id: &str,
    slide_labels: &Value,
) -> Result<usize, String> {
    let mut next_workspace = workspace.clone();
    let cleaned = reset_group_references_in_place(&mut next_workspace, group_id, slide_labels)?;
    *workspace = next_workspace;
    Ok(cleaned)
}

fn reset_group_references_in_place(
    workspace: &mut Value,
    group_id: &str,
    slide_labels: &Value,
) -> Result<usize, String> {
    let group_id = group_id.trim();
    if group_id.is_empty() {
        return Err("Group id must not be empty.".to_string());
    }
    let labels = parse_group_slide_labels(slide_labels)?;
    let document = workspace
        .as_object_mut()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut cleaned = 0;
    if let Some(groups) = document.get_mut("groups") {
        cleaned += remove_matching_group_entries(groups, group_id)?;
    }
    let library = document
        .get_mut("library")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "Workspace library must be an array.".to_string())?;
    let mut matched_labels = HashSet::new();

    for item in library {
        let item = item
            .as_object_mut()
            .ok_or_else(|| "Workspace library items must be objects.".to_string())?;
        if let Some(groups) = item.get_mut("groups") {
            cleaned += remove_matching_group_entries(groups, group_id)?;
        }
        let item_id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| "Workspace library item id is required.".to_string())?;
        let Some(slides) = item.get_mut("slides") else {
            continue;
        };
        let slides = slides
            .as_array_mut()
            .ok_or_else(|| "Workspace presentation slides must be an array.".to_string())?;
        for slide in slides {
            let slide = slide
                .as_object_mut()
                .ok_or_else(|| "Workspace presentation slides must be objects.".to_string())?;
            if !slide_group_reference_matches(slide, group_id) {
                continue;
            }
            let slide_id = slide
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| "Workspace presentation slide id is required.".to_string())?;
            let identity = (item_id.clone(), slide_id.clone());
            let label = labels.get(&identity).ok_or_else(|| {
                format!(
                    "Group deletion is missing a label for slide '{}' in item '{}'.",
                    slide_id, item_id
                )
            })?;
            slide.insert("label".to_string(), Value::String(label.clone()));
            slide.remove("groupId");
            slide.remove("groupName");
            slide.remove("sectionId");
            matched_labels.insert(identity);
            cleaned += 1;
        }
    }

    if matched_labels.len() != labels.len() {
        let extra = labels
            .keys()
            .find(|identity| !matched_labels.contains(*identity))
            .expect("label count mismatch must have an unmatched identity");
        return Err(format!(
            "Group deletion supplied a label for unknown slide '{}' in item '{}'.",
            extra.1, extra.0
        ));
    }

    Ok(cleaned)
}

fn remove_matching_group_entries(value: &mut Value, group_id: &str) -> Result<usize, String> {
    let entries = value
        .as_array_mut()
        .ok_or_else(|| "Workspace group entries must be an array.".to_string())?;
    let before = entries.len();
    entries.retain(|entry| !group_entry_matches(entry, group_id));
    Ok(before.saturating_sub(entries.len()))
}

fn group_entry_matches(entry: &Value, group_id: &str) -> bool {
    let Some(entry) = entry.as_object() else {
        return false;
    };
    ["id", "groupId", "name", "groupName"].iter().any(|key| {
        entry
            .get(*key)
            .and_then(group_reference_slug)
            .is_some_and(|value| value == group_id)
    })
}

fn parse_group_slide_labels(
    slide_labels: &Value,
) -> Result<HashMap<(String, String), String>, String> {
    let labels = slide_labels
        .as_array()
        .ok_or_else(|| "Group deletion slide labels must be an array.".to_string())?;
    let mut parsed = HashMap::with_capacity(labels.len());
    for entry in labels {
        let entry = entry
            .as_object()
            .ok_or_else(|| "Group deletion slide labels must be objects.".to_string())?;
        if let Some(key) = entry
            .keys()
            .find(|key| !["itemId", "slideId", "label"].contains(&key.as_str()))
        {
            return Err(format!(
                "Unsupported group deletion slide label field '{key}'."
            ));
        }
        let item_id = entry
            .get("itemId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Group deletion slide label item id is required.".to_string())?;
        let slide_id = entry
            .get("slideId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Group deletion slide label slide id is required.".to_string())?;
        let label = entry
            .get("label")
            .and_then(Value::as_str)
            .ok_or_else(|| "Group deletion slide label must be a string.".to_string())?;
        if parsed
            .insert(
                (item_id.to_string(), slide_id.to_string()),
                label.to_string(),
            )
            .is_some()
        {
            return Err(format!(
                "Group deletion supplied duplicate label for slide '{}' in item '{}'.",
                slide_id, item_id
            ));
        }
    }
    Ok(parsed)
}

fn slide_group_reference_matches(slide: &Map<String, Value>, group_id: &str) -> bool {
    let value = if slide.contains_key("groupId") {
        slide.get("groupId")
    } else {
        slide.get("groupName")
    };
    let Some(value) = value else {
        return false;
    };
    group_reference_slug(value).is_some_and(|value| value == group_id)
}

fn group_reference_slug(value: &Value) -> Option<String> {
    let source = match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => return None,
    };
    let source = source.trim();
    if source.is_empty() || is_generated_slide_metadata_label(source) {
        return None;
    }
    let mut slug = String::new();
    let mut separator = false;
    for character in source.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            separator = false;
        } else if !separator && !slug.is_empty() {
            slug.push('-');
            separator = true;
        }
    }
    let slug = slug.trim_matches('-');
    (!slug.is_empty()).then(|| slug.to_string())
}

fn validate_clear_groups(value: Option<&Value>) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    let groups = value.as_array().ok_or("Clear groups must be an array.")?;
    let mut ids = HashSet::new();
    for group in groups {
        let id = group
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or("Clear group id is required.")?;
        if !ids.insert(id) {
            return Err("Clear group ids must be unique.".to_string());
        }
        validate_clear_group_icon(group)?;
        group
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .ok_or("Clear group title is required.")?;
        let layers = group
            .get("layers")
            .and_then(Value::as_array)
            .filter(|layers| !layers.is_empty())
            .ok_or("Clear group layers are required.")?;
        for layer in layers {
            if !matches!(
                layer.as_str(),
                Some(
                    "slide"
                        | "background"
                        | "prop"
                        | "audio"
                        | "audio-effects"
                        | "video-input"
                        | "messages"
                        | "stage-messages"
                        | "announcements"
                )
            ) {
                return Err("Unsupported clear group layer.".to_string());
            }
        }
        for key in [
            "hiddenInPreview",
            "stopPresentationTimeline",
            "stopAnnouncementsTimeline",
            "clearPresentationNext",
            "iconTinted",
        ] {
            if group.get(key).is_some_and(|value| !value.is_boolean()) {
                return Err(format!("Clear group {key} must be a boolean."));
            }
        }
    }
    Ok(())
}

fn validate_clear_group_icon(group: &Value) -> Result<(), String> {
    if let Some(tint) = group.get("iconTint") {
        let tint = tint
            .as_str()
            .and_then(|tint| tint.strip_prefix('#'))
            .ok_or("Clear group icon tint must be a hex color.")?;
        if !matches!(tint.len(), 6 | 8) || !tint.chars().all(|value| value.is_ascii_hexdigit()) {
            return Err("Clear group icon tint must be a hex color.".to_string());
        }
    } else if group.get("iconTinted").and_then(Value::as_bool) == Some(true) {
        return Err("Tinted clear group icons require a tint color.".to_string());
    }
    let image_type = match group.get("imageType") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or("Clear group icon type must be a string.")?,
        ),
    };
    match (image_type, group.get("imageData")) {
        (Some("ImageTypeCustom"), Some(data)) => data
            .as_str()
            .filter(|data| !data.is_empty())
            .map(|_| ())
            .ok_or_else(|| "Custom clear group icon data must be a string.".to_string()),
        (Some("ImageTypeCustom"), None) => {
            Err("Custom clear group icon data is required.".to_string())
        }
        (_, Some(_)) => Err("Clear group icon data requires a custom icon type.".to_string()),
        (Some(name), None) if !propresenter_proto::is_clear_group_image_type(name) => {
            Err("Unsupported clear group icon type.".to_string())
        }
        _ => Ok(()),
    }
}

pub fn save_workspace(path: &Path, patch: &Value) -> Result<Value, String> {
    save_workspace_with_assets(path, patch, &[])
}

pub(crate) struct SyncWorkspaceAsset {
    pub relative_path: PathBuf,
    pub source_path: PathBuf,
    pub expected_fingerprint: String,
}

pub(crate) fn save_workspace_with_assets(
    path: &Path,
    patch: &Value,
    additional_assets: &[SyncWorkspaceAsset],
) -> Result<Value, String> {
    save_read_workspace(path, read_workspace(path)?, patch, additional_assets)
}

/// Saves `patch` over `workspace`, which the caller has just read from `path` under the workspace I/O lock.
fn save_read_workspace(
    path: &Path,
    mut workspace: Value,
    patch: &Value,
    additional_assets: &[SyncWorkspaceAsset],
) -> Result<Value, String> {
    let previous = workspace.clone();
    let patch = patch
        .as_object()
        .ok_or_else(|| "Workspace content must be an object.".to_string())?;
    let document = workspace
        .as_object_mut()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    for (key, value) in patch {
        document.insert(key.clone(), value.clone());
    }
    if let Some(template) = patch.get("ccliTemplate") {
        crate::ccli_template::validate(template)?;
    }
    validate_clear_groups(document.get("clearGroups"))?;
    crate::native_ticker::validate_collection(
        document
            .get("tickers")
            .ok_or("Workspace tickers are required.")?,
    )?;
    validate_capture_presets(document.get("recordingPresets"))?;
    crate::audience_looks::validate_transitions(document.get("audienceLooks"))?;
    crate::video_input::validate_collection(
        document
            .get("videoInputs")
            .ok_or("Workspace videoInputs are required.")?,
    )?;
    crate::audio_input_config::validate_collection(
        document
            .get("audioInputs")
            .ok_or("Workspace audioInputs are required.")?,
    )?;
    validate_media_effect_presets(document.get("mediaEffectPresets"))?;
    validate_transition_presets(document.get("transitionPresets"))?;
    validate_slide_transitions(document)?;
    validate_layer_transitions(document)?;
    crate::native_transitions::validate_recents(document.get("recentTransitionEffectIds"))?;
    validate_changed_macro_images(&previous, document)?;
    validate_changed_clear_group_icons(&previous, document)?;
    crate::communication::validate_changed(&previous, &Value::Object(document.clone()))?;
    crate::trigger_bindings::validate_changed(&previous, document)?;
    validate_audio_marker_dependencies(document)?;
    let screen_config = settings::normalize_workspace_screen_config_value(
        document.get("screenConfig").unwrap_or(&Value::Null),
    );
    crate::output_canvas::validate_screen_config_map(&screen_config)?;
    document.insert("screenConfig".to_string(), screen_config);
    let group_settings = settings::normalize_workspace_group_settings_value(
        document.get("groupSettings").unwrap_or(&Value::Null),
    );
    document.insert("groupSettings".to_string(), group_settings);
    let key_mappings = settings::normalize_workspace_key_mappings_value(
        document.get("keyMappings").unwrap_or(&Value::Null),
    );
    document.insert("keyMappings".to_string(), key_mappings);
    let playlists = normalize_playlists_value(document.get("playlist").unwrap_or(&Value::Null));
    document.insert("playlist".to_string(), playlists);
    document.insert("workspacePath".to_string(), json!(path));
    document.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
    if patch.contains_key("library") || patch.contains_key("themes") {
        content_themes::refresh(&previous, &mut workspace)?;
    }
    if let Some(choice) = patch.get("lastUsedPresentationThemeId") {
        workspace["lastUsedPresentationThemeId"] = choice.clone();
        if let Some(id) = choice.as_str() {
            if content_themes::layout(
                workspace
                    .as_object()
                    .ok_or("Workspace must be an object.")?,
                id,
            )
            .is_some()
            {
                content_themes::remember_recent(&mut workspace, id);
            }
        }
    }
    write_workspace_files_with_assets(path, &workspace, additional_assets, Some(&previous))?;
    read_workspace(path)
}

/// Set or clear one stage-screen assignment in a workspace snapshot.
///
/// Stage assignments are stored in the stage document, independently from
/// the screen hardware/look configuration in the workspace settings document.
/// The helper operates on a cloned snapshot so malformed requests never leave
/// a partially-mutated workspace value behind.
pub fn mutate_stage_screen_assignment(
    workspace: &mut Value,
    screen_id: &str,
    layout_id: Option<&str>,
) -> Result<bool, String> {
    let mut next_workspace = workspace.clone();
    let changed =
        mutate_stage_screen_assignment_in_place(&mut next_workspace, screen_id, layout_id)?;
    *workspace = next_workspace;
    Ok(changed)
}

fn mutate_stage_screen_assignment_in_place(
    workspace: &mut Value,
    screen_id: &str,
    layout_id: Option<&str>,
) -> Result<bool, String> {
    let screen_id = screen_id.trim();
    if screen_id.is_empty() {
        return Err("Stage screen assignment screen id is required.".to_string());
    }
    let layout_id = layout_id.map(str::trim);
    if layout_id.is_some_and(str::is_empty) {
        return Err("Stage screen assignment layout id must not be empty.".to_string());
    }

    let document = workspace
        .as_object_mut()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let assignments = document
        .get("stageScreenAssignments")
        .and_then(Value::as_array)
        .ok_or_else(|| "Workspace stage screen assignments must be an array.".to_string())?;
    let matching = assignments
        .iter()
        .filter(|assignment| {
            assignment
                .get("screenId")
                .and_then(Value::as_str)
                .map(str::trim)
                == Some(screen_id)
        })
        .collect::<Vec<_>>();
    if layout_id.is_none() && matching.is_empty() {
        return Ok(false);
    }
    if layout_id.is_some_and(|layout_id| {
        matching.len() == 1
            && matching[0].get("layoutId").and_then(Value::as_str) == Some(layout_id)
    }) {
        return Ok(false);
    }
    let mut next_assignments =
        Vec::with_capacity(assignments.len() + usize::from(layout_id.is_some()));
    let mut changed = false;
    for assignment in assignments {
        let assignment_screen_id = assignment
            .get("screenId")
            .and_then(Value::as_str)
            .map(str::trim);
        if assignment_screen_id == Some(screen_id) {
            changed = true;
            continue;
        }
        next_assignments.push(assignment.clone());
    }
    if let Some(layout_id) = layout_id {
        next_assignments.push(json!({ "screenId": screen_id, "layoutId": layout_id }));
        changed = true;
    }
    if changed {
        document.insert(
            "stageScreenAssignments".to_string(),
            Value::Array(next_assignments),
        );
    }
    Ok(changed)
}

pub fn remove_audience_look_screen(workspace: &mut Value, screen_id: &str) -> Result<bool, String> {
    if screen_id.trim().is_empty() {
        return Err("Audience Look screen id is required.".to_string());
    }
    let mut looks = workspace
        .get("audienceLooks")
        .and_then(Value::as_array)
        .cloned()
        .ok_or("Workspace audience looks must be an array.")?;
    let mut changed = false;
    for look in &mut looks {
        let screens = look
            .get_mut("screenLooks")
            .and_then(Value::as_array_mut)
            .ok_or("Audience Look screens must be an array.")?;
        for screen in screens.iter() {
            screen
                .get("screenId")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or("Audience Look screen id is required.")?;
        }
        let previous = screens.len();
        screens.retain(|screen| screen["screenId"] != screen_id);
        changed |= screens.len() != previous;
    }
    if changed {
        workspace["audienceLooks"] = Value::Array(looks);
    }
    Ok(changed)
}

pub struct MediaLibraryMutationResult {
    pub workspace: Value,
    pub changed: bool,
    pub selected_library_id: Option<String>,
}

pub struct PropsMutationResult {
    pub workspace: Value,
    pub changed: bool,
    pub created_prop_id: Option<String>,
    pub created_folder: Option<String>,
    pub deleted_prop_id: Option<String>,
}

pub struct MaskMutationResult {
    pub workspace: Value,
    pub changed: bool,
    pub created_mask_id: Option<String>,
    pub created_layer_id: Option<String>,
}

pub struct CountdownTimerMutationResult {
    pub workspace: Value,
    pub changed: bool,
    pub created_timer_id: Option<String>,
    pub deleted_timer_id: Option<String>,
    pub removed_timer_ids: Vec<String>,
}

pub struct PresentationLibraryMutationResult {
    pub workspace: Value,
    pub changed: bool,
    pub created_library_id: Option<String>,
    pub created_item_id: Option<String>,
    pub created_playlist_id: Option<String>,
    pub created_slide_id: Option<String>,
    pub selected_slide_id: Option<String>,
    pub created_arrangement_id: Option<String>,
    pub copied_arrangement_id: Option<String>,
    pub deleted_library_ids: Vec<String>,
    pub deleted_item_ids: Vec<String>,
    pub copied_item_ids: Vec<String>,
    pub moved_item_ids: Vec<String>,
}

pub struct StageContentMutationResult {
    pub workspace: Value,
    pub changed: bool,
    pub created_layout_id: Option<String>,
}

/// Apply one semantic stage-layout or stage-message mutation.
///
/// The caller owns the workspace revision lock. This function validates the
/// complete affected document before handing one patch to the staged workspace
/// transaction, so unrelated stage assignments cannot be replaced by a stale
/// renderer snapshot.
pub fn mutate_stage_content(
    path: &Path,
    mutation: &Value,
) -> Result<StageContentMutationResult, String> {
    let mutation = mutation
        .as_object()
        .ok_or_else(|| "Stage content mutation must be an object.".to_string())?;
    let operation = mutation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Stage content mutation type is required.".to_string())?;
    validate_stage_content_mutation_keys(mutation, operation)?;

    let workspace = read_workspace(path)?;
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut layouts = value_array(document, "stageLayouts")?;
    let mut message = document
        .get("stageMessage")
        .cloned()
        .ok_or_else(|| "Workspace stage message is required.".to_string())?;
    validate_stage_message(&message)?;
    let original_layouts = layouts.clone();
    let original_message = message.clone();
    let mut created_layout_id = None;

    match operation {
        "createLayout" => {
            let title =
                required_mutation_string(mutation, "title", "Stage layout title is required.")?
                    .trim();
            if title.is_empty() {
                return Err("Stage layout title must not be empty.".to_string());
            }
            let id = unique_collection_id("stage-layout", title, &layouts);
            let slide_id = format!("{id}-slide");
            let mut slide = mutation.get("slide").cloned().unwrap_or_else(|| {
                json!({
                    "id": slide_id,
                    "name": title,
                    "width": 1920,
                    "height": 1080,
                    "background": "#000000",
                    "layers": []
                })
            });
            let slide_object = slide
                .as_object_mut()
                .ok_or_else(|| "Stage layout slide must be an object.".to_string())?;
            slide_object.insert("id".to_string(), json!(slide_id));
            layouts.push(json!({
                "id": id,
                "title": title,
                "slide": slide
            }));
            validate_stage_layout(
                layouts
                    .last()
                    .expect("created stage layout must have been appended"),
            )?;
            created_layout_id = Some(id);
        }
        "replaceLayout" => {
            let layout_id =
                required_mutation_string(mutation, "layoutId", "Stage layout id is required.")?;
            let mut layout = mutation
                .get("layout")
                .cloned()
                .ok_or_else(|| "Stage layout snapshot is required.".to_string())?;
            let layout_object = layout
                .as_object_mut()
                .ok_or_else(|| "Stage layout must be an object.".to_string())?;
            if layout_object.get("id").and_then(Value::as_str) != Some(layout_id) {
                return Err("Stage layout snapshot id does not match the target id.".to_string());
            }
            let title = layout_object
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Stage layout title is required.".to_string())?;
            layout_object.insert("title".to_string(), json!(title));
            validate_stage_layout(&layout)?;
            let index = layouts
                .iter()
                .position(|item| item.get("id").and_then(Value::as_str) == Some(layout_id))
                .ok_or_else(|| "Stage layout was not found.".to_string())?;
            let current_slide_id = layouts[index]
                .get("slide")
                .and_then(|slide| slide.get("id"))
                .and_then(Value::as_str);
            let next_slide_id = layout
                .get("slide")
                .and_then(|slide| slide.get("id"))
                .and_then(Value::as_str)
                .expect("validated stage layout must have a slide id");
            if current_slide_id.is_some_and(|current| current != next_slide_id) {
                return Err("Stage layout replacement cannot change the slide id.".to_string());
            }
            layouts[index] = layout;
        }
        "setMessageText" => {
            let text = mutation
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| "Stage message text must be a string.".to_string())?;
            message
                .as_object_mut()
                .expect("validated stage message must be an object")
                .insert("text".to_string(), json!(text));
        }
        "setMessageState" => {
            let state = mutation
                .get("state")
                .and_then(Value::as_str)
                .filter(|value| matches!(*value, "visible" | "hidden" | "cleared"))
                .ok_or_else(|| "Stage message state is unsupported.".to_string())?;
            message
                .as_object_mut()
                .expect("validated stage message must be an object")
                .insert("state".to_string(), json!(state));
        }
        "clearMessage" => {
            message = json!({ "text": "", "state": "cleared" });
        }
        _ => return Err("Unsupported stage content mutation type.".to_string()),
    }

    validate_stage_message(&message)?;
    let changed = layouts != original_layouts || message != original_message;
    let workspace = if changed {
        save_workspace(
            path,
            &json!({ "stageLayouts": layouts, "stageMessage": message }),
        )?
    } else {
        workspace
    };
    Ok(StageContentMutationResult {
        workspace,
        changed,
        created_layout_id,
    })
}

fn validate_stage_content_mutation_keys(
    mutation: &Map<String, Value>,
    operation: &str,
) -> Result<(), String> {
    let allowed = match operation {
        "createLayout" => &["type", "title", "slide"][..],
        "replaceLayout" => &["type", "layoutId", "layout"][..],
        "setMessageText" => &["type", "text"][..],
        "setMessageState" => &["type", "state"][..],
        "clearMessage" => &["type"][..],
        _ => return Err("Unsupported stage content mutation type.".to_string()),
    };
    validate_mutation_keys(mutation, allowed, "stage content mutation")
}

fn validate_stage_layout(layout: &Value) -> Result<(), String> {
    let layout = layout
        .as_object()
        .ok_or_else(|| "Stage layout must be an object.".to_string())?;
    layout
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Stage layout id is required.".to_string())?;
    layout
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Stage layout title is required.".to_string())?;
    let slide = layout
        .get("slide")
        .and_then(Value::as_object)
        .ok_or_else(|| "Stage layout slide is required.".to_string())?;
    slide
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Stage layout slide id is required.".to_string())?;
    for key in ["width", "height"] {
        slide
            .get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| format!("Stage layout slide {key} must be a positive number."))?;
    }
    if slide.get("layers").and_then(Value::as_array).is_none() {
        return Err("Stage layout slide layers must be an array.".to_string());
    }
    Ok(())
}

fn validate_stage_message(message: &Value) -> Result<(), String> {
    let message = message
        .as_object()
        .ok_or_else(|| "Stage message must be an object.".to_string())?;
    if message.get("text").and_then(Value::as_str).is_none() {
        return Err("Stage message text must be a string.".to_string());
    }
    if !message
        .get("state")
        .and_then(Value::as_str)
        .is_some_and(|value| matches!(value, "visible" | "hidden" | "cleared"))
    {
        return Err("Stage message state is unsupported.".to_string());
    }
    Ok(())
}

/// Apply one semantic mask-document mutation against the accepted workspace.
///
/// The renderer owns interactive editing, while Rust owns stable document
/// identity and the persisted collection. Updating a mask replaces only its
/// authored title, size, and slide so imported metadata remains intact.
pub fn mutate_masks(path: &Path, mutation: &Value) -> Result<MaskMutationResult, String> {
    let mutation = mutation
        .as_object()
        .ok_or_else(|| "Mask mutation must be an object.".to_string())?;
    let operation = mutation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Mask mutation type is required.".to_string())?;
    validate_mask_mutation_keys(mutation, operation)?;

    let workspace = read_workspace(path)?;
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut masks = value_array(document, "masks")?;
    let original_masks = masks.clone();
    let mut created_mask_id = None;
    let mut created_layer_id = None;
    let find_mask = |id: &str| {
        original_masks
            .iter()
            .position(|mask| mask["id"] == id)
            .ok_or("Mask was not found.")
    };

    match operation {
        "create" => {
            let title =
                required_mutation_string(mutation, "baseTitle", "Mask base title is required.")?
                    .trim();
            if title.is_empty() {
                return Err("Mask base title must not be empty.".to_string());
            }
            let id = unique_collection_id("mask", title, &masks);
            masks.push(new_mask_document(&id, title));
            created_mask_id = Some(id);
        }
        "reorder" => {
            let id = required_mutation_string(mutation, "maskId", "Mask id is required.")?;
            let target =
                required_mutation_string(mutation, "targetMaskId", "Target mask id is required.")?;
            let before = mutation
                .get("before")
                .and_then(Value::as_bool)
                .ok_or("Mask drop position is required.")?;
            let from = find_mask(id)?;
            let target = find_mask(target)?;
            if from != target {
                let gap = target + usize::from(!before);
                let moved = masks.remove(from);
                masks.insert(gap - usize::from(from < gap), moved);
            }
        }
        "duplicate" | "paste" => {
            let id = required_mutation_string(mutation, "maskId", "Mask id is required.")?;
            let index = find_mask(id)?;
            let source = if operation == "duplicate" {
                &masks[index]
            } else {
                mutation.get("mask").ok_or("Copied mask is required.")?
            };
            let title = source["title"]
                .as_str()
                .filter(|title| !title.trim().is_empty())
                .ok_or("Mask title is required.")?;
            let title = unique_copy_title(title, &masks);
            let id = format!("mask-{}", Uuid::new_v4());
            let mut copy = new_mask_document(&id, &title);
            validate_mask_size(&source["size"])?;
            copy["size"] = source["size"].clone();
            let mut slide = source["slide"]
                .as_object()
                .cloned()
                .ok_or("Copied mask slide is required.")?;
            // Older imported empty masks omit the layer list.
            slide.entry("layers").or_insert_with(|| json!([]));
            slide.insert("id".into(), copy["slide"]["id"].clone());
            slide.insert("name".into(), json!(title));
            copy["slide"] = Value::Object(slide);
            validate_mask_slide(&copy["slide"])?;
            masks.insert(index + 1, copy);
            created_mask_id = Some(id);
        }
        "delete" => {
            let id = required_mutation_string(mutation, "maskId", "Mask id is required.")?;
            masks.remove(find_mask(id)?);
        }
        "updateDocument" => {
            let mask_id = required_mutation_string(mutation, "maskId", "Mask id is required.")?;
            if !mutation.contains_key("title")
                && !mutation.contains_key("size")
                && !mutation.contains_key("slide")
            {
                return Err("Mask document update requires a field patch.".to_string());
            }
            let index = masks
                .iter()
                .position(|mask| mask.get("id").and_then(Value::as_str) == Some(mask_id))
                .ok_or_else(|| "Mask was not found.".to_string())?;
            let mask = masks[index]
                .as_object_mut()
                .ok_or_else(|| "Mask must be an object.".to_string())?;
            if let Some(title) = mutation.get("title") {
                let title = title
                    .as_str()
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .ok_or("Mask title must not be empty.")?;
                mask.insert("title".to_string(), json!(title));
            }
            if let Some(size) = mutation.get("size") {
                validate_mask_size(size)?;
                mask.insert("size".to_string(), size.clone());
            }
            if let Some(slide) = mutation.get("slide") {
                validate_mask_slide(slide)?;
                let replacement_slide_id = slide
                    .get("id")
                    .and_then(Value::as_str)
                    .expect("validated mask slide must have an id");
                let current_slide_id = mask
                    .get("slide")
                    .and_then(|value| value.get("id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Persisted mask slide id is required.".to_string())?;
                if current_slide_id != replacement_slide_id {
                    return Err("Mask document update cannot change the slide id.".to_string());
                }
                mask.insert("slide".to_string(), slide.clone());
            }
        }
        "addLayer" => {
            let mask_id = required_mutation_string(mutation, "maskId", "Mask id is required.")?;
            let mut layer = mutation
                .get("layer")
                .and_then(Value::as_object)
                .cloned()
                .ok_or_else(|| "Mask layer snapshot must be an object.".to_string())?;
            if layer.contains_key("id") {
                return Err("Mask layer identity is allocated natively.".to_string());
            }
            let layer_type = layer
                .get("type")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Mask layer type is required.".to_string())?;
            let index = masks
                .iter()
                .position(|mask| mask.get("id").and_then(Value::as_str) == Some(mask_id))
                .ok_or_else(|| "Mask was not found.".to_string())?;
            let slide = masks[index]
                .get_mut("slide")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| "Persisted mask slide is required.".to_string())?;
            slide
                .entry("layers".to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
            let layers = slide
                .get_mut("layers")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| "Persisted mask layers must be an array.".to_string())?;
            let layer_kind = if layer_type.eq_ignore_ascii_case("text") {
                "text"
            } else if layer.get("mediaFillEnabled").and_then(Value::as_bool) == Some(true) {
                "media"
            } else {
                "shape"
            };
            let base_id = format!("mask-{layer_kind}");
            let id = unique_id(
                &base_id,
                layers
                    .iter()
                    .filter_map(|value| value.get("id").and_then(Value::as_str)),
            );
            layer.insert("id".to_string(), json!(id));
            layers.push(Value::Object(layer));
            validate_mask_slide(&Value::Object(slide.clone()))?;
            created_layer_id = Some(id);
        }
        _ => return Err("Unsupported mask mutation type.".to_string()),
    }

    let changed = masks != original_masks;
    let workspace = if changed {
        let mut patch = json!({ "masks": masks });
        if operation == "delete" {
            let removed = &mutation["maskId"];
            let mut looks = value_array(document, "audienceLooks")?;
            for look in &mut looks {
                if let Some(screens) = look["screenLooks"].as_array_mut() {
                    for screen in screens
                        .iter_mut()
                        .filter(|screen| &screen["maskId"] == removed)
                    {
                        screen["maskId"] = json!("");
                    }
                }
            }
            patch["audienceLooks"] = json!(looks);
            let mut screens = workspace["screenConfig"].clone();
            if let Some(screens) = screens.as_object_mut() {
                for screen in screens
                    .values_mut()
                    .filter(|screen| &screen["lookThemes"]["maskId"] == removed)
                {
                    screen["lookThemes"]["maskId"] = json!("");
                }
            }
            patch["screenConfig"] = screens;
        }
        save_workspace(path, &patch)?
    } else {
        workspace
    };
    Ok(MaskMutationResult {
        workspace,
        changed,
        created_mask_id,
        created_layer_id,
    })
}

fn new_mask_document(id: &str, title: &str) -> Value {
    json!({
        "id": id,
        "title": title,
        "schemaVersion": 1,
        "sourceType": "presenter-orbit",
        "sourcePath": "",
        "editable": true,
        "size": { "width": 1920, "height": 1080 },
        "slide": {
            "id": format!("{id}-slide"),
            "name": title,
            "width": 1920,
            "height": 1080,
            "background": "transparent",
            "backgroundEnabled": false,
            "text": "",
            "layers": []
        }
    })
}

fn validate_mask_mutation_keys(
    mutation: &Map<String, Value>,
    operation: &str,
) -> Result<(), String> {
    let allowed = match operation {
        "create" => &["type", "baseTitle"][..],
        "reorder" => &["type", "maskId", "targetMaskId", "before"][..],
        "duplicate" | "delete" => &["type", "maskId"][..],
        "paste" => &["type", "maskId", "mask"][..],
        "updateDocument" => &["type", "maskId", "title", "size", "slide"][..],
        "addLayer" => &["type", "maskId", "layer"][..],
        _ => return Err("Unsupported mask mutation type.".to_string()),
    };
    validate_mutation_keys(mutation, allowed, "mask mutation")
}

fn validate_mask_slide(slide: &Value) -> Result<(), String> {
    let slide = slide
        .as_object()
        .ok_or_else(|| "Mask slide snapshot must be an object.".to_string())?;
    slide
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Mask slide snapshot id is required.".to_string())?;
    let layers = slide
        .get("layers")
        .and_then(Value::as_array)
        .ok_or_else(|| "Mask slide layers must be an array.".to_string())?;
    let mut ids = HashSet::new();
    for layer in layers {
        let id = layer
            .as_object()
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Mask layer id is required.".to_string())?;
        if !ids.insert(id) {
            return Err(format!("Duplicate mask layer id '{id}'."));
        }
    }
    Ok(())
}

fn validate_mask_size(size: &Value) -> Result<(), String> {
    let size = size
        .as_object()
        .ok_or_else(|| "Mask size must be an object.".to_string())?;
    for key in ["width", "height"] {
        size.get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| format!("Mask {key} must be a positive number."))?;
    }
    Ok(())
}

pub fn mutate_props(path: &Path, mutation: &Value) -> Result<PropsMutationResult, String> {
    let workspace = read_workspace(path)?;
    let mut result = plan_props_mutation(&workspace, mutation)?;
    result.workspace = if result.changed {
        save_workspace(path, &result.workspace)?
    } else {
        workspace
    };
    Ok(result)
}

/// Validates and prepares prop edits without storage, so the UI can display the exact pending result.
pub fn plan_props_mutation(
    workspace: &Value,
    mutation: &Value,
) -> Result<PropsMutationResult, String> {
    let mutation = mutation
        .as_object()
        .ok_or_else(|| "Props mutation must be an object.".to_string())?;
    let operation = mutation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Props mutation type is required.".to_string())?;
    validate_props_mutation_keys(mutation, operation)?;
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut props = value_array(document, "props")?;
    let original_props = props.clone();
    let mut prop_folders = value_array(document, "propFolders")?
        .into_iter()
        .map(|folder| {
            folder
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "Prop folder must be a string.".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let original_prop_folders = prop_folders.clone();
    let mut created_prop_id = None;
    let mut created_folder = None;
    let mut deleted_prop_id = None;

    match operation {
        "create" => {
            let base_title =
                required_mutation_string(mutation, "baseTitle", "Prop base title is required.")?;
            let folder =
                optional_mutation_string(mutation, "folder", "Prop folder must be a string")?
                    .unwrap_or_default();
            let title = unique_prop_title(base_title, &props, None);
            let id = unique_prop_id(&title, &props);
            let slide_id = unique_prop_slide_id(&id, &props);
            props.push(json!({
                "id": id,
                "title": title,
                "folder": folder,
                "slide": default_prop_slide(&slide_id),
            }));
            created_prop_id = Some(id);
        }
        "createFolder" => {
            let base_title = required_mutation_string(
                mutation,
                "baseTitle",
                "Prop folder base title is required.",
            )?;
            let title = unique_prop_folder_title(base_title, &prop_folders);
            prop_folders.push(title.clone());
            created_folder = Some(title);
        }
        "renameFolder" | "duplicateFolder" | "deleteFolder" => {
            let folder =
                optional_mutation_string(mutation, "folder", "Prop folder must be a string")?
                    .ok_or("Prop folder is required.")?;
            let in_folder = |prop: &Value| {
                prop.get("folder")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    == folder
            };
            if !folder.is_empty() && !prop_folders.contains(&folder) && !props.iter().any(in_folder)
            {
                return Err("Prop folder was not found.".to_string());
            }
            if operation != "duplicateFolder" && folder.is_empty() {
                return Err("The default prop collection cannot be renamed or deleted.".to_string());
            }
            match operation {
                "renameFolder" => {
                    let title = required_mutation_string(
                        mutation,
                        "title",
                        "Prop folder title is required.",
                    )?;
                    let others = prop_folders
                        .iter()
                        .filter(|entry| **entry != folder)
                        .cloned()
                        .collect::<Vec<_>>();
                    if unique_prop_folder_title(title, &others) != title {
                        return Err("A prop folder with this title already exists.".to_string());
                    }
                    match prop_folders.iter().position(|entry| *entry == folder) {
                        Some(index) => prop_folders[index] = title.to_string(),
                        None => prop_folders.push(title.to_string()),
                    }
                    for prop in props.iter_mut().filter(|prop| in_folder(prop)) {
                        prop.as_object_mut()
                            .ok_or("Prop must be an object.")?
                            .insert("folder".to_string(), json!(title));
                    }
                }
                "duplicateFolder" => {
                    let source = if folder.is_empty() {
                        "Default Collection"
                    } else {
                        folder.as_str()
                    };
                    let title = unique_prop_folder_title(&format!("{source} Copy"), &prop_folders);
                    let index = prop_folders
                        .iter()
                        .position(|entry| *entry == folder)
                        .map_or(0, |index| index + 1);
                    prop_folders.insert(index, title.clone());
                    for source in original_props.iter().filter(|prop| in_folder(prop)) {
                        let (prop, _) = clone_prop_snapshot(source, &title, &props)?;
                        props.push(prop);
                    }
                    created_folder = Some(title);
                }
                _ => {
                    // Like ProPresenter, deleting a collection deletes the props in it.
                    prop_folders.retain(|entry| *entry != folder);
                    props.retain(|prop| !in_folder(prop));
                }
            }
        }
        "reorderFolders" => {
            let folders = mutation
                .get("folders")
                .and_then(Value::as_array)
                .ok_or("Prop folder order is required.")?
                .iter()
                .map(|folder| {
                    folder
                        .as_str()
                        .map(str::to_string)
                        .ok_or("Prop folder must be a string.")
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut expected = prop_folders.clone();
            for name in props
                .iter()
                .filter_map(|prop| prop["folder"].as_str())
                .filter(|name| !name.is_empty())
            {
                if !expected.iter().any(|folder| folder == name) {
                    expected.push(name.to_string());
                }
            }
            let mut requested = folders.clone();
            expected.sort();
            requested.sort();
            if expected != requested {
                return Err("Prop folder order must contain every prop folder once.".to_string());
            }
            prop_folders = folders;
        }
        "paste" => {
            let snapshot = mutation
                .get("prop")
                .ok_or_else(|| "Prop snapshot is required.".to_string())?;
            let folder =
                optional_mutation_string(mutation, "folder", "Prop folder must be a string")?
                    .unwrap_or_default();
            let (prop, id) = clone_prop_snapshot(snapshot, &folder, &props)?;
            props.push(prop);
            created_prop_id = Some(id);
        }
        "duplicate" => {
            let prop_id = required_mutation_string(mutation, "propId", "Prop id is required.")?;
            let source = props
                .iter()
                .find(|prop| prop.get("id").and_then(Value::as_str) == Some(prop_id))
                .cloned()
                .ok_or_else(|| "Prop was not found.".to_string())?;
            let folder =
                optional_mutation_string(mutation, "folder", "Prop folder must be a string")?
                    .or_else(|| {
                        source
                            .get("folder")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_default();
            let (prop, id) = clone_prop_snapshot(&source, &folder, &props)?;
            props.push(prop);
            created_prop_id = Some(id);
        }
        "move" => {
            let prop_id = required_mutation_string(mutation, "propId", "Prop id is required.")?;
            let folder =
                optional_mutation_string(mutation, "folder", "Prop folder must be a string")?
                    .unwrap_or_default();
            let prop = props
                .iter_mut()
                .find(|prop| prop.get("id").and_then(Value::as_str) == Some(prop_id))
                .ok_or_else(|| "Prop was not found.".to_string())?;
            let object = prop
                .as_object_mut()
                .ok_or_else(|| "Prop must be an object.".to_string())?;
            if object
                .get("folder")
                .and_then(Value::as_str)
                .unwrap_or_default()
                != folder
            {
                object.insert("folder".to_string(), json!(folder));
            }
        }
        "reorder" => {
            let source = required_mutation_string(mutation, "propId", "Prop id is required.")?;
            let target =
                required_mutation_string(mutation, "targetPropId", "Target prop id is required.")?;
            let index = |id: &str| props.iter().position(|prop| prop["id"] == id);
            let from = index(source).ok_or("Prop was not found.")?;
            let to = index(target).ok_or("Target prop was not found.")?;
            if props[from]["folder"].as_str().unwrap_or_default()
                != props[to]["folder"].as_str().unwrap_or_default()
            {
                return Err("Props can only be reordered within the same collection.".into());
            }
            let after = match mutation["placement"].as_str() {
                Some("before") => false,
                Some("after") => true,
                _ => return Err("Prop placement must be before or after.".into()),
            };
            if from != to {
                let gap = to + usize::from(after);
                let prop = props.remove(from);
                props.insert(gap - usize::from(from < gap), prop);
            }
        }
        "updateDocument" => {
            let prop_id = required_mutation_string(mutation, "propId", "Prop id is required.")?;
            let slide = mutation
                .get("slide")
                .cloned()
                .ok_or_else(|| "Prop slide snapshot is required.".to_string())?;
            let slide_object = slide
                .as_object()
                .ok_or_else(|| "Prop slide snapshot must be an object.".to_string())?;
            let replacement_slide_id = slide_object
                .get("id")
                .and_then(Value::as_str)
                .filter(|slide_id| !slide_id.trim().is_empty())
                .ok_or_else(|| "Prop slide snapshot id is required.".to_string())?;
            let index = props
                .iter()
                .position(|prop| prop.get("id").and_then(Value::as_str) == Some(prop_id))
                .ok_or_else(|| "Prop was not found.".to_string())?;
            let current_slide_id = props[index]
                .get("slide")
                .and_then(|slide| slide.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(|| "Persisted prop slide id is required.".to_string())?;
            if current_slide_id != replacement_slide_id {
                return Err("Prop document update cannot change the slide id.".to_string());
            }
            let prop = props[index]
                .as_object_mut()
                .ok_or_else(|| "Prop must be an object.".to_string())?;
            if let Some(title) = slide_object
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
            {
                prop.insert("title".to_string(), json!(title));
            }
            prop.insert("slide".to_string(), slide);
        }
        "setTransition" => {
            let prop_id = required_mutation_string(mutation, "propId", "Prop id is required.")?;
            let transition = mutation
                .get("transition")
                .ok_or("Transition is required; use null to remove it.")?;
            if !transition.is_null() {
                crate::native_transitions::validate(transition)?;
            }
            let object = props
                .iter_mut()
                .find(|prop| prop.get("id").and_then(Value::as_str) == Some(prop_id))
                .and_then(Value::as_object_mut)
                .ok_or_else(|| "Prop was not found.".to_string())?;
            if transition.is_null() {
                object.remove("transition");
            } else {
                object.insert("transition".to_string(), transition.clone());
            }
        }
        "setAutoClear" => {
            let prop_id = required_mutation_string(mutation, "propId", "Prop id is required.")?;
            let seconds = match mutation.get("seconds") {
                Some(Value::Null) => None,
                Some(value) => {
                    let seconds = value
                        .as_f64()
                        .filter(|value| value.is_finite() && *value > 0.0)
                        .ok_or_else(|| {
                            "Prop auto-clear seconds must be positive or null.".to_string()
                        })?;
                    Some(seconds)
                }
                None => return Err("Prop auto-clear seconds are required.".to_string()),
            };
            let prop = props
                .iter_mut()
                .find(|prop| prop.get("id").and_then(Value::as_str) == Some(prop_id))
                .ok_or_else(|| "Prop was not found.".to_string())?;
            let object = prop
                .as_object_mut()
                .ok_or_else(|| "Prop must be an object.".to_string())?;
            let next = seconds.map_or(Value::Null, |value| json!(value));
            let current = object
                .get("autoClearSeconds")
                .cloned()
                .unwrap_or(Value::Null);
            if current != next {
                object.insert("autoClearSeconds".to_string(), next);
            }
        }
        "rename" => {
            let prop_id = required_mutation_string(mutation, "propId", "Prop id is required.")?;
            let requested_title =
                required_mutation_string(mutation, "title", "Prop title is required.")?;
            let title = requested_title.to_string();
            let prop = props
                .iter_mut()
                .find(|prop| prop.get("id").and_then(Value::as_str) == Some(prop_id))
                .ok_or_else(|| "Prop was not found.".to_string())?;
            let object = prop
                .as_object_mut()
                .ok_or_else(|| "Prop must be an object.".to_string())?;
            object.insert("title".to_string(), json!(title));
            if let Some(slide) = object.get_mut("slide").and_then(Value::as_object_mut) {
                slide.insert("name".to_string(), json!(title));
            }
        }
        "delete" => {
            let prop_id = required_mutation_string(mutation, "propId", "Prop id is required.")?;
            let index = props
                .iter()
                .position(|prop| prop.get("id").and_then(Value::as_str) == Some(prop_id))
                .ok_or_else(|| "Prop was not found.".to_string())?;
            props.remove(index);
            deleted_prop_id = Some(prop_id.to_string());
        }
        _ => return Err("Unsupported props mutation type.".to_string()),
    }

    let changed = props != original_props || prop_folders != original_prop_folders;
    let workspace = json!({ "props": props, "propFolders": prop_folders });
    Ok(PropsMutationResult {
        workspace,
        changed,
        created_prop_id,
        created_folder,
        deleted_prop_id,
    })
}

/// Apply one semantic mutation to the persisted countdown definitions.
///
/// The entire definition collection is read, changed in memory, and handed
/// to `save_workspace` only after validation succeeds. `save_workspace`
/// writes all workspace documents through its staged transaction, so an
/// invalid definition or failed commit leaves both the old definitions and
/// the caller's revision unchanged.
pub fn mutate_countdown_timers(
    path: &Path,
    mutation: &Value,
) -> Result<CountdownTimerMutationResult, String> {
    let mutation = mutation
        .as_object()
        .ok_or_else(|| "Countdown timer mutation must be an object.".to_string())?;
    let operation = mutation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Countdown timer mutation type is required.".to_string())?;
    validate_countdown_timer_mutation_keys(mutation, operation)?;

    let workspace = read_workspace(path)?;
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut timers = value_array(document, "countdownTimers")?;
    validate_countdown_timer_definitions(&timers)?;
    let original_timers = timers.clone();
    let mut created_timer_id = None;
    let mut deleted_timer_id = None;
    let mut removed_timer_ids = Vec::new();

    match operation {
        "create" => {
            let base_name = required_mutation_string(
                mutation,
                "baseName",
                "Countdown timer base name is required.",
            )?;
            let patch = mutation.get("patch");
            let requested_name = patch
                .and_then(Value::as_object)
                .and_then(|patch| patch.get("name"))
                .map(|value| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .ok_or_else(|| {
                            "Countdown timer name must be a non-empty string.".to_string()
                        })
                })
                .transpose()?
                .unwrap_or(base_name)
                .to_string();
            let id = unique_collection_id("timer", &requested_name, &timers);
            // Keep renderer-visible names stable. Only the native id is
            // disambiguated when another timer uses the same name.
            let name = requested_name.clone();
            let mut timer = json!({
                "id": id,
                "name": name,
                "durationSeconds": 300,
                "allowsOverrun": false
            });
            if let Some(patch) = patch {
                apply_countdown_timer_patch(&mut timer, patch)?;
            }
            timer
                .as_object_mut()
                .ok_or_else(|| "Countdown timer must be an object.".to_string())?
                .insert("name".to_string(), json!(name));
            validate_countdown_timer_definition(&timer)?;
            created_timer_id = Some(id);
            timers.push(timer);
        }
        "update" => {
            let timer_id =
                required_mutation_string(mutation, "timerId", "Countdown timer id is required.")?;
            let patch = mutation
                .get("patch")
                .ok_or_else(|| "Countdown timer patch is required.".to_string())?;
            let index = timers
                .iter()
                .position(|timer| timer.get("id").and_then(Value::as_str) == Some(timer_id))
                .ok_or_else(|| "Countdown timer was not found.".to_string())?;
            // A missing type means "countdown"; an explicit equal type is not a type change.
            let effective_type = |timer: &Value| {
                timer
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("countdown")
                    .to_string()
            };
            let previous_type = effective_type(&timers[index]);
            let mut next = timers[index].clone();
            apply_countdown_timer_patch(&mut next, patch)?;
            validate_countdown_timer_definition(&next)?;
            if next != timers[index] && effective_type(&next) != previous_type {
                removed_timer_ids.push(timer_id.to_string());
            }
            timers[index] = next;
        }
        "reorder" => {
            let timer_id =
                required_mutation_string(mutation, "timerId", "Countdown timer id is required.")?;
            let target_timer_id = required_mutation_string(
                mutation,
                "targetTimerId",
                "Countdown timer target id is required.",
            )?;
            let placement = mutation
                .get("placement")
                .and_then(Value::as_str)
                .ok_or_else(|| "Countdown timer placement is required.".to_string())?;
            if !matches!(placement, "before" | "after") {
                return Err("Countdown timer placement must be before or after.".to_string());
            }
            if timer_id == target_timer_id {
                if !timers
                    .iter()
                    .any(|timer| timer.get("id").and_then(Value::as_str) == Some(timer_id))
                {
                    return Err("Countdown timer was not found.".to_string());
                }
                return Ok(unchanged_countdown_timer_result(workspace));
            }
            let source_index = timers
                .iter()
                .position(|timer| timer.get("id").and_then(Value::as_str) == Some(timer_id))
                .ok_or_else(|| "Countdown timer was not found.".to_string())?;
            let target_index = timers
                .iter()
                .position(|timer| timer.get("id").and_then(Value::as_str) == Some(target_timer_id))
                .ok_or_else(|| "Countdown timer target was not found.".to_string())?;
            let moved = timers.remove(source_index);
            let target_index = if source_index < target_index {
                target_index.saturating_sub(1)
            } else {
                target_index
            };
            let insertion_index = if placement == "after" {
                target_index.saturating_add(1)
            } else {
                target_index
            };
            timers.insert(insertion_index.min(timers.len()), moved);
        }
        "delete" => {
            let timer_id =
                required_mutation_string(mutation, "timerId", "Countdown timer id is required.")?;
            let index = timers
                .iter()
                .position(|timer| timer.get("id").and_then(Value::as_str) == Some(timer_id))
                .ok_or_else(|| "Countdown timer was not found.".to_string())?;
            timers.remove(index);
            deleted_timer_id = Some(timer_id.to_string());
            removed_timer_ids.push(timer_id.to_string());
        }
        _ => return Err("Unsupported countdown timer mutation type.".to_string()),
    }

    let changed = timers != original_timers;
    let workspace = if changed {
        save_read_workspace(path, workspace, &json!({ "countdownTimers": timers }), &[])?
    } else {
        workspace
    };
    Ok(CountdownTimerMutationResult {
        workspace,
        changed,
        created_timer_id,
        deleted_timer_id,
        removed_timer_ids: if changed {
            removed_timer_ids
        } else {
            Vec::new()
        },
    })
}

fn unchanged_countdown_timer_result(workspace: Value) -> CountdownTimerMutationResult {
    CountdownTimerMutationResult {
        workspace,
        changed: false,
        created_timer_id: None,
        deleted_timer_id: None,
        removed_timer_ids: Vec::new(),
    }
}

fn validate_countdown_timer_mutation_keys(
    mutation: &Map<String, Value>,
    operation: &str,
) -> Result<(), String> {
    let allowed = match operation {
        "create" => &["type", "baseName", "patch"][..],
        "update" => &["type", "timerId", "patch"][..],
        "reorder" => &["type", "timerId", "targetTimerId", "placement"][..],
        "delete" => &["type", "timerId"][..],
        _ => return Err("Unsupported countdown timer mutation type.".to_string()),
    };
    validate_mutation_keys(mutation, allowed, "countdown timer mutation")
}

fn validate_countdown_timer_definitions(timers: &[Value]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for timer in timers {
        validate_countdown_timer_definition(timer)?;
        let id = timer
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Countdown timer id is required.".to_string())?;
        if !ids.insert(id.to_string()) {
            return Err(format!("Duplicate countdown timer id '{id}'."));
        }
    }
    Ok(())
}

fn validate_countdown_timer_definition(timer: &Value) -> Result<(), String> {
    let object = timer
        .as_object()
        .ok_or_else(|| "Countdown timer must be an object.".to_string())?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Countdown timer id is required.".to_string())?;
    let _ = id;
    object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Countdown timer name is required.".to_string())?;
    let duration = object
        .get("durationSeconds")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| "Countdown timer duration must be a non-negative number.".to_string())?;
    let _ = duration;
    if object
        .get("allowsOverrun")
        .and_then(Value::as_bool)
        .is_none()
    {
        return Err("Countdown timer allowsOverrun must be a boolean.".to_string());
    }
    if let Some(kind) = object.get("type") {
        let kind = kind
            .as_str()
            .ok_or_else(|| "Countdown timer type must be a string.".to_string())?;
        if !matches!(kind, "countdown" | "countdown-to-time" | "elapsed") {
            return Err("Countdown timer type is unsupported.".to_string());
        }
    }
    for key in [
        "targetTimeSeconds",
        "elapsedStartSeconds",
        "elapsedEndSeconds",
    ] {
        if let Some(value) = object.get(key) {
            if value
                .as_f64()
                .filter(|number| number.is_finite() && *number >= 0.0)
                .is_none()
            {
                return Err(format!(
                    "Countdown timer {key} must be a non-negative number."
                ));
            }
        }
    }
    if let Some(format) = object.get("format") {
        if !format.is_null() && !format.is_object() {
            return Err("Countdown timer format must be an object or null.".to_string());
        }
    }
    Ok(())
}

fn apply_countdown_timer_patch(timer: &mut Value, patch: &Value) -> Result<(), String> {
    let patch = patch
        .as_object()
        .ok_or_else(|| "Countdown timer patch must be an object.".to_string())?;
    let allowed = [
        "name",
        "allowsOverrun",
        "durationSeconds",
        "type",
        "targetTimeSeconds",
        "elapsedEndSeconds",
        "elapsedStartSeconds",
        "format",
    ];
    validate_mutation_keys(patch, &allowed, "countdown timer patch")?;
    let object = timer
        .as_object_mut()
        .ok_or_else(|| "Countdown timer must be an object.".to_string())?;
    for (key, value) in patch {
        if key == "name" {
            let name = value
                .as_str()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| "Countdown timer name must be a non-empty string.".to_string())?;
            object.insert(key.clone(), json!(name));
        } else if value.is_null()
            && matches!(
                key.as_str(),
                "targetTimeSeconds" | "elapsedEndSeconds" | "elapsedStartSeconds" | "format"
            )
        {
            object.remove(key);
        } else {
            object.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}

fn validate_header_timer_configuration(configuration: &Value) -> Result<(), String> {
    let fields = configuration
        .as_object()
        .ok_or("Header timer configuration must be an object.")?;
    validate_mutation_keys(
        fields,
        &[
            "type",
            "allowsOverrun",
            "durationSeconds",
            "targetTimeSeconds",
            "elapsedStartSeconds",
            "elapsedEndSeconds",
            "format",
        ],
        "header timer configuration",
    )?;
    let seconds = |key: &str| {
        configuration[key]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.)
    };
    let duration =
        seconds("durationSeconds").ok_or("Header timer duration must be non-negative.")?;
    let valid = match configuration["type"].as_str() {
        Some("countdown") => {
            configuration["targetTimeSeconds"].is_null()
                && configuration["elapsedStartSeconds"].is_null()
                && configuration["elapsedEndSeconds"].is_null()
                && configuration["format"].is_null()
        }
        Some("countdown-to-time") => {
            let format = &configuration["format"];
            let valid_period = format.is_null()
                || (format.as_object().is_some_and(|fields| fields.len() == 1)
                    && match format["targetTimePeriod"].as_str() {
                        Some("am") => duration < 43_200.,
                        Some("pm") => duration >= 43_200.,
                        Some("24h") => true,
                        _ => false,
                    });
            duration < 86_400.
                && valid_period
                && seconds("targetTimeSeconds") == Some(duration)
                && configuration["elapsedStartSeconds"].is_null()
                && configuration["elapsedEndSeconds"].is_null()
        }
        Some("elapsed") => {
            configuration["targetTimeSeconds"].is_null()
                && configuration["format"].is_null()
                && seconds("elapsedStartSeconds").is_some()
                && if configuration["elapsedEndSeconds"].is_null() {
                    seconds("elapsedStartSeconds") == Some(duration)
                } else {
                    seconds("elapsedEndSeconds") == Some(duration)
                }
        }
        _ => false,
    };
    if !valid || !configuration["allowsOverrun"].is_boolean() {
        return Err("Header timer configuration is invalid.".into());
    }
    Ok(())
}

fn validate_props_mutation_keys(
    mutation: &Map<String, Value>,
    operation: &str,
) -> Result<(), String> {
    let allowed = match operation {
        "create" => &["type", "baseTitle", "folder"][..],
        "createFolder" => &["type", "baseTitle"][..],
        "renameFolder" => &["type", "folder", "title"][..],
        "duplicateFolder" | "deleteFolder" => &["type", "folder"][..],
        "reorderFolders" => &["type", "folders"][..],
        "duplicate" => &["type", "propId", "folder"][..],
        "paste" => &["type", "prop", "folder"][..],
        "move" => &["type", "propId", "folder"][..],
        "reorder" => &["type", "propId", "targetPropId", "placement"][..],
        "updateDocument" => &["type", "propId", "slide"][..],
        "setAutoClear" => &["type", "propId", "seconds"][..],
        "setTransition" => &["type", "propId", "transition"][..],
        "rename" => &["type", "propId", "title"][..],
        "delete" => &["type", "propId"][..],
        _ => return Err("Unsupported props mutation type.".to_string()),
    };
    validate_mutation_keys(mutation, allowed, "props mutation")
}

fn optional_mutation_string(
    mutation: &Map<String, Value>,
    key: &str,
    message: &str,
) -> Result<Option<String>, String> {
    match mutation.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.trim().to_string())),
        Some(_) => Err(message.to_string()),
    }
}

fn clone_prop_snapshot(
    snapshot: &Value,
    folder: &str,
    props: &[Value],
) -> Result<(Value, String), String> {
    let source = snapshot
        .as_object()
        .ok_or_else(|| "Prop snapshot must be an object.".to_string())?;
    if source
        .get("id")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err("Prop snapshot id is required.".to_string());
    }
    let source_title = source
        .get("title")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Prop snapshot title is required.".to_string())?;
    let source_slide = source
        .get("slide")
        .and_then(Value::as_object)
        .ok_or_else(|| "Prop snapshot slide is required.".to_string())?;
    if source_slide
        .get("id")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err("Prop snapshot slide id is required.".to_string());
    }
    let title = source_title.to_string();
    let id = unique_prop_id(&title, props);
    let slide_id = unique_prop_slide_id(&id, props);
    let mut prop = source.clone();
    prop.insert("id".to_string(), json!(id));
    prop.insert("title".to_string(), json!(title));
    prop.insert("folder".to_string(), json!(folder));
    let mut slide = source_slide.clone();
    slide.insert("id".to_string(), json!(slide_id));
    prop.insert("slide".to_string(), Value::Object(slide));
    Ok((Value::Object(prop), id))
}

fn unique_prop_title(base_title: &str, props: &[Value], exclude_id: Option<&str>) -> String {
    let base = if base_title.trim().is_empty() {
        "New prop"
    } else {
        base_title.trim()
    };
    let used = props
        .iter()
        .filter(|prop| prop.get("id").and_then(Value::as_str) != exclude_id)
        .filter_map(|prop| prop.get("title").and_then(Value::as_str))
        .map(|title| title.trim().to_lowercase())
        .collect::<HashSet<_>>();
    if !used.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base} {index}");
        if !used.contains(&candidate.to_lowercase()) {
            return candidate;
        }
        index += 1;
    }
}

fn unique_prop_folder_title(base_title: &str, folders: &[String]) -> String {
    let base = if base_title.trim().is_empty() {
        "New prop folder"
    } else {
        base_title.trim()
    };
    let used = folders
        .iter()
        .map(|folder| folder.trim().to_lowercase())
        .collect::<HashSet<_>>();
    if !used.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base} {index}");
        if !used.contains(&candidate.to_lowercase()) {
            return candidate;
        }
        index += 1;
    }
}

fn unique_prop_id(title: &str, props: &[Value]) -> String {
    unique_collection_id("prop", title, props)
}

fn unique_prop_slide_id(prop_id: &str, props: &[Value]) -> String {
    let used = props
        .iter()
        .filter_map(|prop| prop.get("slide"))
        .filter_map(|slide| slide.get("id").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    let base = format!("{prop_id}-slide");
    let mut candidate = base.clone();
    let mut index = 2;
    while used.contains(candidate.as_str()) {
        candidate = format!("{base}-{index}");
        index += 1;
    }
    candidate
}

fn default_prop_slide(slide_id: &str) -> Value {
    static DEFAULT_TEXT_STYLE: OnceLock<Value> = OnceLock::new();
    let text_style = DEFAULT_TEXT_STYLE
        .get_or_init(|| {
            serde_json::from_str(
                r###"{
        "fontFamily": "Arial",
        "fontSize": 90,
        "bold": true,
        "italic": false,
        "underline": false,
        "strikethrough": false,
        "capitalization": "None",
        "scaling": "None",
        "lineTransform": "None",
        "textHorizontalAlign": "center",
        "textVerticalAlign": "middle",
        "textFillMode": "Text Color",
        "textColor": "#ffffff",
        "textGradientStart": "#0b63f6",
        "textGradientEnd": "#14c8e8",
        "textGradientAngle": 4,
        "mediaFillId": "",
        "mediaFillUrl": "",
        "mediaFillSourcePath": "",
        "mediaFillTitle": "",
        "mediaFillType": "",
        "mediaFillScale": "Scale to Fill",
        "mediaFillAlignment": "Middle Center",
        "mediaFillAlphaType": "Straight",
        "mediaFillCropTop": 0,
        "mediaFillCropRight": 0,
        "mediaFillCropBottom": 0,
        "mediaFillCropLeft": 0,
        "mediaFillPlayback": "Loop",
        "mediaFillVolume": 100,
        "mediaFillRampIn": 0,
        "mediaFillRampOut": 0,
        "backgroundBlurSaturation": 100,
        "backgroundBlurAmount": 14,
        "strokeEnabled": false,
        "strokeColor": "#000000",
        "strokeWidth": 2,
        "shadowEnabled": false,
        "shadowColor": "#000000",
        "shadowOpacity": 75,
        "shadowAngle": 315,
        "shadowOffset": 5,
        "shadowBlur": 5,
        "linesOnlyEnabled": false,
        "linesOnlyPadding": 0,
        "scrollingEnabled": false,
        "scrollDirection": "left",
        "scrollStart": "Automatic",
        "scrollSpeed": 50,
        "scrollRepeat": true,
        "scrollRepeatDistance": 100,
        "scrollFeatherLeft": 0,
        "scrollFeatherRight": 0,
        "scrollFeatherLinked": true,
        "linkedTextSource": "None",
        "linkedTextRssDescriptionSeparateLine": true,
        "linkedTextRssTitleSeparateLine": true,
        "systemClockDateFormat": "numeric",
        "systemClockTimeFormat": "hours-minutes",
        "systemClockUse24Hour": false
    }"###,
            )
            .expect("default prop text style should be valid JSON")
        })
        .clone();
    json!({
        "id": slide_id,
        "name": "Theme Slide",
        "text": "Text",
        "background": "#202020",
        "backgroundEnabled": false,
        "width": 1920,
        "height": 1080,
        "layers": [{
            "id": "text",
            "name": "Text",
            "type": "Text",
            "role": "slide-text",
            "hidden": false,
            "textStyle": text_style
        }]
    })
}

pub struct PresentationPlaylistMutationResult {
    pub workspace: Value,
    pub changed: bool,
    pub created_playlist_ids: Vec<String>,
    pub deleted_playlist_ids: Vec<String>,
    pub created_entry_ids: Vec<String>,
    pub added_entry_ids: Vec<String>,
    pub renamed_entry_ids: Vec<String>,
    pub updated_entry_ids: Vec<String>,
    pub deleted_entry_ids: Vec<String>,
    pub reordered_entry_ids: Vec<String>,
    pub copied_entry_ids: Vec<String>,
}

pub fn mutate_presentation_library(
    path: &Path,
    mutation: &Value,
) -> Result<PresentationLibraryMutationResult, String> {
    let mutation = mutation
        .as_object()
        .ok_or_else(|| "Presentation library mutation must be an object.".to_string())?;
    let operation = mutation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Presentation library mutation type is required.".to_string())?;
    let workspace = read_workspace(path)?;
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut libraries = value_array(document, "libraries")?;

    match operation {
        "create" => {
            let base_title = required_mutation_string(
                mutation,
                "baseTitle",
                "Presentation library base title is required.",
            )?;
            let title = unique_presentation_library_title(base_title, &libraries);
            let library_id = unique_presentation_library_id(&title, &libraries);
            libraries.push(json!({ "id": library_id, "title": title, "itemIds": [] }));
            let workspace = save_workspace(path, &json!({ "libraries": libraries }))?;
            return Ok(PresentationLibraryMutationResult {
                workspace,
                changed: true,
                created_library_id: Some(library_id),
                created_item_id: None,
                created_playlist_id: None,
                created_slide_id: None,
                selected_slide_id: None,
                created_arrangement_id: None,
                copied_arrangement_id: None,
                deleted_library_ids: Vec::new(),
                deleted_item_ids: Vec::new(),
                copied_item_ids: Vec::new(),
                moved_item_ids: Vec::new(),
            });
        }
        "rename" => {
            let library_id = required_mutation_string(
                mutation,
                "libraryId",
                "Presentation library id is required.",
            )?;
            let title = required_mutation_string(
                mutation,
                "title",
                "Presentation library title is required.",
            )?;
            let library = libraries
                .iter_mut()
                .find(|library| library.get("id").and_then(Value::as_str) == Some(library_id))
                .ok_or_else(|| "Presentation library was not found.".to_string())?;
            let object = library
                .as_object_mut()
                .ok_or_else(|| "Presentation library must be an object.".to_string())?;
            let changed = object.get("title").and_then(Value::as_str) != Some(title);
            if !changed {
                return Ok(PresentationLibraryMutationResult {
                    workspace,
                    changed: false,
                    created_library_id: None,
                    created_item_id: None,
                    created_playlist_id: None,
                    created_slide_id: None,
                    selected_slide_id: None,
                    created_arrangement_id: None,
                    copied_arrangement_id: None,
                    deleted_library_ids: Vec::new(),
                    deleted_item_ids: Vec::new(),
                    copied_item_ids: Vec::new(),
                    moved_item_ids: Vec::new(),
                });
            }
            object.insert("title".to_string(), json!(title));
        }
        "setTheme" => {
            validate_mutation_keys(
                mutation,
                &["type", "libraryIds", "assignment"],
                "presentation library theme mutation",
            )?;
            let library_ids = dedupe_mutation_strings(required_mutation_string_array(
                mutation,
                "libraryIds",
                "Presentation library theme ids are required.",
            )?);
            let known_library_ids = libraries
                .iter()
                .filter_map(|library| library.get("id").and_then(Value::as_str))
                .collect::<HashSet<_>>();
            if let Some(missing) = library_ids
                .iter()
                .find(|library_id| !known_library_ids.contains(library_id.as_str()))
            {
                return Err(format!("Presentation library '{missing}' was not found."));
            }
            let assignment = presentation_theme_assignment_mutation(document, mutation)?;
            let assignment_id = assignment
                .as_ref()
                .and_then(Value::as_str)
                .ok_or("Library theme requires an available layout.")?;
            let mut items = value_array(document, "library")?;
            let mut selected_items = HashSet::new();
            let requested = library_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            for library in &mut libraries {
                if !library
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|library_id| requested.contains(library_id))
                {
                    continue;
                }
                selected_items.extend(
                    library["itemIds"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string),
                );
                set_presentation_theme_assignment(library, assignment.as_ref())?;
            }
            for item in &mut items {
                if selected_items.contains(item["id"].as_str().unwrap_or_default()) {
                    let mut slides = presentation_item_slides(item)?;
                    for slide in &mut slides {
                        content_themes::assign(document, slide, assignment_id)?;
                    }
                    set_presentation_item_slides(item, slides)?;
                }
            }
            if libraries == value_array(document, "libraries")?
                && items == value_array(document, "library")?
                && document
                    .get("lastUsedPresentationThemeId")
                    .and_then(Value::as_str)
                    == Some(assignment_id)
            {
                return Ok(unchanged_presentation_library_result(workspace));
            }
            let saved = save_workspace(
                path,
                &json!({ "libraries": libraries, "library": items, "lastUsedPresentationThemeId": assignment_id }),
            )?;
            let mut result = unchanged_presentation_library_result(saved);
            result.changed = true;
            return Ok(result);
        }
        "reorder" => {
            let library_id = required_mutation_string(
                mutation,
                "libraryId",
                "Presentation library id is required.",
            )?;
            let target_id = required_mutation_string(
                mutation,
                "targetLibraryId",
                "Presentation library reorder target is required.",
            )?;
            let placement = required_mutation_string(
                mutation,
                "placement",
                "Presentation library reorder placement is required.",
            )?;
            if !matches!(placement, "before" | "after") {
                return Err(
                    "Presentation library reorder placement must be before or after.".to_string(),
                );
            }
            let dragged_index = libraries
                .iter()
                .position(|library| library.get("id").and_then(Value::as_str) == Some(library_id))
                .ok_or_else(|| "Presentation library was not found.".to_string())?;
            if !libraries
                .iter()
                .any(|library| library.get("id").and_then(Value::as_str) == Some(target_id))
            {
                return Err("Presentation library reorder target was not found.".to_string());
            }
            if library_id == target_id {
                return Ok(PresentationLibraryMutationResult {
                    workspace,
                    changed: false,
                    created_library_id: None,
                    created_item_id: None,
                    created_playlist_id: None,
                    created_slide_id: None,
                    selected_slide_id: None,
                    created_arrangement_id: None,
                    copied_arrangement_id: None,
                    deleted_library_ids: Vec::new(),
                    deleted_item_ids: Vec::new(),
                    copied_item_ids: Vec::new(),
                    moved_item_ids: Vec::new(),
                });
            }
            let original = libraries.clone();
            let dragged = libraries.remove(dragged_index);
            let target_index = libraries
                .iter()
                .position(|library| library.get("id").and_then(Value::as_str) == Some(target_id))
                .unwrap_or(libraries.len());
            let insert_index = if placement == "after" {
                target_index.saturating_add(1)
            } else {
                target_index
            };
            libraries.insert(insert_index.min(libraries.len()), dragged);
            if libraries == original {
                return Ok(PresentationLibraryMutationResult {
                    workspace,
                    changed: false,
                    created_library_id: None,
                    created_item_id: None,
                    created_playlist_id: None,
                    created_slide_id: None,
                    selected_slide_id: None,
                    created_arrangement_id: None,
                    copied_arrangement_id: None,
                    deleted_library_ids: Vec::new(),
                    deleted_item_ids: Vec::new(),
                    copied_item_ids: Vec::new(),
                    moved_item_ids: Vec::new(),
                });
            }
        }
        "copyItems" => {
            let target_library_id = required_mutation_string(
                mutation,
                "targetLibraryId",
                "Presentation library copy target is required.",
            )?;
            let has_source_item_ids = mutation.contains_key("sourceItemIds");
            let has_snapshot_items = mutation.contains_key("items");
            if has_source_item_ids == has_snapshot_items {
                return Err(
                    "Presentation library copy must use either sourceItemIds or items (but not both)."
                        .to_string(),
                );
            }
            let target_index = libraries
                .iter()
                .position(|library| {
                    library.get("id").and_then(Value::as_str) == Some(target_library_id)
                })
                .ok_or_else(|| "Presentation library copy target was not found.".to_string())?;
            let after_item_id = mutation
                .get("afterItemId")
                .map(|value| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| {
                            "Presentation library copy anchor must be a non-empty string."
                                .to_string()
                        })
                })
                .transpose()?;

            let library_items = value_array(document, "library")?;
            let source_items = if has_source_item_ids {
                let source_item_ids = required_mutation_string_array(
                    mutation,
                    "sourceItemIds",
                    "Presentation library copy source ids are required.",
                )?;
                let source_items_by_id = library_items
                    .iter()
                    .filter_map(|item| item.get("id").and_then(Value::as_str).map(|id| (id, item)))
                    .collect::<HashMap<_, _>>();
                source_item_ids
                    .into_iter()
                    .map(|source_item_id| {
                        source_items_by_id
                            .get(source_item_id.as_str())
                            .map(|item| (source_item_id.clone(), (*item).clone()))
                            .ok_or_else(|| {
                                format!(
                                    "Presentation library item '{source_item_id}' was not found."
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                mutation
                    .get("items")
                    .and_then(Value::as_array)
                    .filter(|items| !items.is_empty())
                    .ok_or_else(|| {
                        "Presentation library copy snapshot items are required.".to_string()
                    })?
                    .iter()
                    .map(|item| {
                        if !item.is_object() {
                            return Err(
                                "Presentation library copy snapshot items must be objects."
                                    .to_string(),
                            );
                        }
                        let source_item_id = item
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|id| !id.is_empty())
                            .ok_or_else(|| {
                                "Presentation library copy snapshot item ids must be non-empty strings."
                                    .to_string()
                            })?;
                        Ok((source_item_id.to_string(), item.clone()))
                    })
                    .collect::<Result<Vec<_>, String>>()?
            };

            let target_item_ids = libraries[target_index]
                .get("itemIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let insertion_index = after_item_id
                .as_deref()
                .map(|after_item_id| {
                    target_item_ids
                        .iter()
                        .position(|item_id| item_id.as_str() == Some(after_item_id))
                        .map(|index| index.saturating_add(1))
                        .ok_or_else(|| {
                            "Presentation library copy anchor was not found in the target."
                                .to_string()
                        })
                })
                .transpose()?;

            let mut used_item_ids = library_items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<HashSet<_>>();
            for library in &libraries {
                used_item_ids.extend(
                    library
                        .get("itemIds")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string),
                );
            }
            let mut existing_titles = library_items
                .iter()
                .filter_map(|item| item.get("title").and_then(Value::as_str))
                .map(str::trim)
                .map(str::to_string)
                .collect::<HashSet<_>>();
            let mut copies = Vec::with_capacity(source_items.len());
            let mut copied_item_ids = Vec::with_capacity(source_items.len());
            for (source_item_id, source_item) in source_items {
                let copied_item_id = unique_copy_id(&source_item_id, &used_item_ids);
                used_item_ids.insert(copied_item_id.clone());
                let source_title = match source_item.get("title").and_then(Value::as_str) {
                    None | Some("") => Some("Untitled"),
                    Some(title) => Some(title),
                };
                let copied_title = copy_name(
                    source_title,
                    existing_titles.iter().map(|title| Some(title.as_str())),
                );
                existing_titles.insert(copied_title.clone());
                let mut copied_item = source_item.as_object().cloned().ok_or_else(|| {
                    "Presentation library copy snapshot items must be objects.".to_string()
                })?;
                copied_item.insert("id".to_string(), json!(&copied_item_id));
                copied_item.insert("title".to_string(), json!(&copied_title));
                copies.push(Value::Object(copied_item));
                copied_item_ids.push(copied_item_id);
            }

            let mut target_item_ids = target_item_ids;
            if let Some(insertion_index) = insertion_index {
                target_item_ids.splice(
                    insertion_index..insertion_index,
                    copied_item_ids.iter().cloned().map(Value::String),
                );
            } else {
                target_item_ids.extend(copied_item_ids.iter().cloned().map(Value::String));
            }
            libraries[target_index]
                .as_object_mut()
                .ok_or_else(|| "Presentation library must be an object.".to_string())?
                .insert("itemIds".to_string(), Value::Array(target_item_ids));
            let mut library_items = library_items;
            library_items.extend(copies);
            let workspace = save_workspace(
                path,
                &json!({ "libraries": libraries, "library": library_items }),
            )?;
            return Ok(PresentationLibraryMutationResult {
                workspace,
                changed: true,
                created_library_id: None,
                created_item_id: None,
                created_playlist_id: None,
                created_slide_id: None,
                selected_slide_id: None,
                created_arrangement_id: None,
                copied_arrangement_id: None,
                deleted_library_ids: Vec::new(),
                deleted_item_ids: Vec::new(),
                copied_item_ids,
                moved_item_ids: Vec::new(),
            });
        }
        "moveItems" => {
            let target_library_id = required_mutation_string(
                mutation,
                "targetLibraryId",
                "Presentation library move target is required.",
            )?;
            let requested_item_ids = required_mutation_string_array(
                mutation,
                "itemIds",
                "Presentation library move item ids are required.",
            )?;
            let item_ids = dedupe_mutation_strings(requested_item_ids);
            let target_index = libraries
                .iter()
                .position(|library| {
                    library.get("id").and_then(Value::as_str) == Some(target_library_id)
                })
                .ok_or_else(|| "Presentation library move target was not found.".to_string())?;
            let library_items = value_array(document, "library")?;
            let known_item_ids = library_items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .collect::<HashSet<_>>();
            if let Some(missing) = item_ids
                .iter()
                .find(|item_id| !known_item_ids.contains(item_id.as_str()))
            {
                return Err(format!(
                    "Presentation library item '{missing}' was not found."
                ));
            }
            let original = libraries.clone();
            let item_id_set = item_ids.iter().map(String::as_str).collect::<HashSet<_>>();
            for library in &mut libraries {
                let object = library
                    .as_object_mut()
                    .ok_or_else(|| "Presentation library must be an object.".to_string())?;
                let next_item_ids = object
                    .get("itemIds")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|item_id| {
                        item_id
                            .as_str()
                            .is_none_or(|item_id| !item_id_set.contains(item_id))
                    })
                    .collect::<Vec<_>>();
                object.insert("itemIds".to_string(), Value::Array(next_item_ids));
            }
            let target = libraries[target_index]
                .as_object_mut()
                .ok_or_else(|| "Presentation library must be an object.".to_string())?;
            let mut target_item_ids = target
                .get("itemIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            target_item_ids.extend(item_ids.iter().cloned().map(Value::String));
            target.insert("itemIds".to_string(), Value::Array(target_item_ids));
            if libraries == original {
                return Ok(PresentationLibraryMutationResult {
                    workspace,
                    changed: false,
                    created_library_id: None,
                    created_item_id: None,
                    created_playlist_id: None,
                    created_slide_id: None,
                    selected_slide_id: None,
                    created_arrangement_id: None,
                    copied_arrangement_id: None,
                    deleted_library_ids: Vec::new(),
                    deleted_item_ids: Vec::new(),
                    copied_item_ids: Vec::new(),
                    moved_item_ids: Vec::new(),
                });
            }
            let workspace = save_workspace(path, &json!({ "libraries": libraries }))?;
            return Ok(PresentationLibraryMutationResult {
                workspace,
                changed: true,
                created_library_id: None,
                created_item_id: None,
                created_playlist_id: None,
                created_slide_id: None,
                selected_slide_id: None,
                created_arrangement_id: None,
                copied_arrangement_id: None,
                deleted_library_ids: Vec::new(),
                deleted_item_ids: Vec::new(),
                copied_item_ids: Vec::new(),
                moved_item_ids: item_ids,
            });
        }
        "deleteItems" => {
            let requested_item_ids = required_mutation_string_array(
                mutation,
                "itemIds",
                "Presentation library delete item ids are required.",
            )?;
            let item_ids = dedupe_mutation_strings(requested_item_ids);
            let library_items = value_array(document, "library")?;
            let known_item_ids = library_items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .collect::<HashSet<_>>();
            if let Some(missing) = item_ids
                .iter()
                .find(|item_id| !known_item_ids.contains(item_id.as_str()))
            {
                return Err(format!(
                    "Presentation library item '{missing}' was not found."
                ));
            }
            let item_id_set = item_ids.iter().map(String::as_str).collect::<HashSet<_>>();
            for library in &mut libraries {
                let object = library
                    .as_object_mut()
                    .ok_or_else(|| "Presentation library must be an object.".to_string())?;
                let next_item_ids = object
                    .get("itemIds")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|item_id| {
                        item_id
                            .as_str()
                            .is_none_or(|item_id| !item_id_set.contains(item_id))
                    })
                    .collect::<Vec<_>>();
                object.insert("itemIds".to_string(), Value::Array(next_item_ids));
            }
            let library = library_items
                .into_iter()
                .filter(|item| {
                    item.get("id")
                        .and_then(Value::as_str)
                        .is_none_or(|item_id| !item_id_set.contains(item_id))
                })
                .collect::<Vec<_>>();
            let mut playlists = value_array(document, "playlist")?;
            for playlist in &mut playlists {
                let object = playlist
                    .as_object_mut()
                    .ok_or_else(|| "Playlist must be an object.".to_string())?;
                let items = object
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|entry| {
                        entry.get("type").and_then(Value::as_str) != Some("presentation")
                            || entry
                                .get("itemId")
                                .and_then(Value::as_str)
                                .is_none_or(|item_id| !item_id_set.contains(item_id))
                    })
                    .collect::<Vec<_>>();
                let item_ids = items
                    .iter()
                    .filter(|entry| {
                        entry.get("type").and_then(Value::as_str) == Some("presentation")
                    })
                    .filter_map(|entry| entry.get("itemId").and_then(Value::as_str))
                    .map(|item_id| json!(item_id))
                    .collect::<Vec<_>>();
                object.insert("items".to_string(), Value::Array(items));
                object.insert("itemIds".to_string(), Value::Array(item_ids));
            }
            let workspace = save_workspace(
                path,
                &json!({ "libraries": libraries, "library": library, "playlist": playlists }),
            )?;
            return Ok(PresentationLibraryMutationResult {
                workspace,
                changed: true,
                created_library_id: None,
                created_item_id: None,
                created_playlist_id: None,
                created_slide_id: None,
                selected_slide_id: None,
                created_arrangement_id: None,
                copied_arrangement_id: None,
                deleted_library_ids: Vec::new(),
                deleted_item_ids: item_ids,
                copied_item_ids: Vec::new(),
                moved_item_ids: Vec::new(),
            });
        }
        "createPresentation" => {
            return create_presentation_document(path, workspace, mutation);
        }
        "mutateItem" => {
            return mutate_presentation_item(path, workspace, mutation);
        }
        "createArrangement"
        | "duplicateArrangement"
        | "renameArrangement"
        | "moveArrangement"
        | "setSelectedArrangement"
        | "deleteArrangement"
        | "addArrangementSection"
        | "insertArrangementSection"
        | "removeArrangementSection"
        | "moveArrangementSection" => {
            return mutate_presentation_arrangement(path, workspace, mutation, operation);
        }
        "delete" => {
            let library_ids = mutation
                .get("libraryIds")
                .and_then(Value::as_array)
                .filter(|ids| !ids.is_empty())
                .ok_or_else(|| "Presentation library delete ids are required.".to_string())?;
            let mut deleted_library_ids = HashSet::new();
            for library_id in library_ids {
                let library_id = library_id
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        "Presentation library delete ids must be strings.".to_string()
                    })?;
                deleted_library_ids.insert(library_id.to_string());
            }
            let known_library_ids = libraries
                .iter()
                .filter_map(|library| library.get("id").and_then(Value::as_str))
                .collect::<HashSet<_>>();
            if let Some(missing) = deleted_library_ids
                .iter()
                .find(|library_id| !known_library_ids.contains(library_id.as_str()))
            {
                return Err(format!("Presentation library '{missing}' was not found."));
            }
            let deleted_library_id_order = libraries
                .iter()
                .filter_map(|library| library.get("id").and_then(Value::as_str))
                .filter(|id| deleted_library_ids.contains(*id))
                .map(str::to_string)
                .collect::<Vec<_>>();

            let mut deleted_item_ids = Vec::new();
            let mut deleted_item_id_set = HashSet::new();
            for library in &libraries {
                if !library
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| deleted_library_ids.contains(id))
                {
                    continue;
                }
                for item_id in library
                    .get("itemIds")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    if deleted_item_id_set.insert(item_id.to_string()) {
                        deleted_item_ids.push(item_id.to_string());
                    }
                }
            }
            libraries.retain(|library| {
                library
                    .get("id")
                    .and_then(Value::as_str)
                    .is_none_or(|id| !deleted_library_ids.contains(id))
            });
            for library in &mut libraries {
                let Some(object) = library.as_object_mut() else {
                    return Err("Presentation library must be an object.".to_string());
                };
                let item_ids = object
                    .get("itemIds")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|item_id| {
                        item_id
                            .as_str()
                            .is_none_or(|item_id| !deleted_item_id_set.contains(item_id))
                    })
                    .collect();
                object.insert("itemIds".to_string(), Value::Array(item_ids));
            }
            let library = value_array(document, "library")?
                .into_iter()
                .filter(|item| {
                    item.get("id")
                        .and_then(Value::as_str)
                        .is_none_or(|id| !deleted_item_id_set.contains(id))
                })
                .collect::<Vec<_>>();
            let mut playlists = value_array(document, "playlist")?;
            for playlist in &mut playlists {
                let object = playlist
                    .as_object_mut()
                    .ok_or_else(|| "Playlist must be an object.".to_string())?;
                let items = object
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|entry| {
                        entry.get("type").and_then(Value::as_str) != Some("presentation")
                            || entry
                                .get("itemId")
                                .and_then(Value::as_str)
                                .is_none_or(|id| !deleted_item_id_set.contains(id))
                    })
                    .collect::<Vec<_>>();
                let item_ids = items
                    .iter()
                    .filter(|entry| {
                        entry.get("type").and_then(Value::as_str) == Some("presentation")
                    })
                    .filter_map(|entry| entry.get("itemId").and_then(Value::as_str))
                    .map(|id| json!(id))
                    .collect::<Vec<_>>();
                object.insert("items".to_string(), Value::Array(items));
                object.insert("itemIds".to_string(), Value::Array(item_ids));
            }
            let workspace = save_workspace(
                path,
                &json!({ "libraries": libraries, "library": library, "playlist": playlists }),
            )?;
            return Ok(PresentationLibraryMutationResult {
                workspace,
                changed: true,
                created_library_id: None,
                created_item_id: None,
                created_playlist_id: None,
                created_slide_id: None,
                selected_slide_id: None,
                created_arrangement_id: None,
                copied_arrangement_id: None,
                deleted_library_ids: deleted_library_id_order,
                deleted_item_ids,
                copied_item_ids: Vec::new(),
                moved_item_ids: Vec::new(),
            });
        }
        _ => return Err("Unsupported presentation library mutation type.".to_string()),
    }

    let workspace = save_workspace(path, &json!({ "libraries": libraries }))?;
    Ok(PresentationLibraryMutationResult {
        workspace,
        changed: true,
        created_library_id: None,
        created_item_id: None,
        created_playlist_id: None,
        created_slide_id: None,
        selected_slide_id: None,
        created_arrangement_id: None,
        copied_arrangement_id: None,
        deleted_library_ids: Vec::new(),
        deleted_item_ids: Vec::new(),
        copied_item_ids: Vec::new(),
        moved_item_ids: Vec::new(),
    })
}

fn create_presentation_document(
    path: &Path,
    workspace: Value,
    mutation: &Map<String, Value>,
) -> Result<PresentationLibraryMutationResult, String> {
    let allowed = [
        "type",
        "libraryId",
        "title",
        "content",
        "splitMode",
        "splitMarker",
        "splitUnits",
        "author",
        "operatorNotes",
        "playlistTitle",
        "playlistId",
        "presentationThemeId",
        "size",
        "slideContents",
        "ccli",
    ];
    if let Some(key) = mutation.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unsupported presentation creation field '{key}'."));
    }

    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut libraries = value_array(document, "libraries")?;
    let mut library_items = value_array(document, "library")?;
    let mut playlists = value_array(document, "playlist")?;
    let library_id = required_mutation_string(
        mutation,
        "libraryId",
        "Presentation creation library id is required.",
    )?;
    let library_index = libraries
        .iter()
        .position(|library| library.get("id").and_then(Value::as_str) == Some(library_id))
        .ok_or_else(|| "Presentation creation library was not found.".to_string())?;
    let base_title = required_mutation_string(
        mutation,
        "title",
        "Presentation creation title is required.",
    )?;
    let content = mutation_string(
        mutation,
        "content",
        "Presentation creation content must be a string.",
    )?;
    let split_mode = required_mutation_string(
        mutation,
        "splitMode",
        "Presentation creation split mode is required.",
    )?;
    if !matches!(split_mode, "paragraph" | "line" | "marker") {
        return Err("Presentation creation split mode is invalid.".to_string());
    }
    let split_marker = mutation_string(
        mutation,
        "splitMarker",
        "Presentation creation split marker must be a string.",
    )?;
    let split_units = mutation
        .get("splitUnits")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| "Presentation creation split units must be a finite number.".to_string())?
        .max(1.0);
    let author = optional_trimmed_mutation_string(mutation, "author")?;
    let operator_notes = optional_trimmed_mutation_string(mutation, "operatorNotes")?;
    let playlist_title = optional_trimmed_mutation_string(mutation, "playlistTitle")?;
    if mutation.contains_key("playlistId") && mutation.contains_key("playlistTitle") {
        return Err(
            "Presentation creation cannot specify both playlistId and playlistTitle.".to_string(),
        );
    }
    let playlist_index = if mutation.contains_key("playlistId") {
        let playlist_id = required_mutation_string(
            mutation,
            "playlistId",
            "Presentation creation playlist id is required.",
        )?;
        Some(
            playlists
                .iter()
                .position(|playlist| {
                    playlist.get("id").and_then(Value::as_str) == Some(playlist_id)
                        && playlist.get("type").and_then(Value::as_str) != Some("folder")
                })
                .ok_or_else(|| {
                    "Presentation creation playlist was not found or is a folder.".to_string()
                })?,
        )
    } else {
        None
    };
    let theme_assignment = mutation
        .get("presentationThemeId")
        .or_else(|| document.get("lastUsedPresentationThemeId"))
        .and_then(Value::as_str)
        .filter(|id| known_presentation_theme_assignment(document, id))
        .ok_or("Presentation creation requires an available theme layout.")?
        .to_string();

    let (title, item_id) = unique_created_presentation_identity(
        base_title,
        &library_items,
        libraries[library_index]
            .get("itemIds")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default(),
    );
    let sections = match mutation.get("slideContents") {
        Some(value) => value
            .as_array()
            .filter(|slides| !slides.is_empty())
            .ok_or("Edited presentation slides must be a nonempty array.")?
            .iter()
            .map(|slide| {
                slide
                    .as_str()
                    .map(str::to_string)
                    .ok_or("Edited presentation slide content must be text.")
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => split_presentation_text(content, split_mode, split_marker, split_units),
    };
    let mut slides: Vec<_> = sections
        .into_iter()
        .enumerate()
        .map(|(index, content)| json!({ "id": format!("slide-{}", index + 1), "content": content }))
        .collect();
    for slide in &mut slides {
        content_themes::assign(document, slide, &theme_assignment)?;
    }
    let mut item = Map::new();
    item.insert("id".to_string(), json!(&item_id));
    item.insert("title".to_string(), json!(&title));
    item.insert("slides".to_string(), Value::Array(slides));
    if let Some(author) = author {
        item.insert("author".to_string(), json!(author));
    }
    if let Some(operator_notes) = operator_notes {
        item.insert("operatorNotes".to_string(), json!(operator_notes));
    }
    let mut item = Value::Object(item);
    if let Some(size) = mutation.get("size") {
        crate::presentation_settings::patch(&mut item, "size", size)?;
    }
    if let Some(ccli) = mutation.get("ccli") {
        for (field, value) in ccli.as_object().ok_or("CCLI metadata must be an object.")? {
            if !matches!(
                field.as_str(),
                "title"
                    | "artistCredits"
                    | "author"
                    | "publisher"
                    | "copyrightYear"
                    | "songNumber"
                    | "display"
            ) {
                return Err(format!("Unknown CCLI field: {field}."));
            }
            crate::presentation_settings::patch(&mut item, field, value)?;
        }
    }
    library_items.push(item);

    let target_library = libraries[library_index]
        .as_object_mut()
        .ok_or_else(|| "Presentation library must be an object.".to_string())?;
    let mut item_ids = target_library
        .get("itemIds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    item_ids.push(json!(&item_id));
    target_library.insert("itemIds".to_string(), Value::Array(item_ids));

    if let Some(index) = playlist_index {
        let playlist = &mut playlists[index];
        let mut entries = value_array(
            playlist
                .as_object()
                .ok_or_else(|| "Presentation playlist must be an object.".to_string())?,
            "items",
        )?;
        let entry_id = unique_presentation_entry_id(&item_id, &entries);
        entries.push(json!({ "id": entry_id, "type": "presentation", "itemId": &item_id }));
        let item_ids = entries
            .iter()
            .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("presentation"))
            .filter_map(|entry| entry.get("itemId").cloned())
            .collect::<Vec<_>>();
        playlist["items"] = Value::Array(entries);
        playlist["itemIds"] = Value::Array(item_ids);
    }

    let created_playlist_id = playlist_title.map(|base_playlist_title| {
        let playlist_title = unique_collection_title(&base_playlist_title, &playlists);
        let playlist_id = unique_collection_id("playlist", &playlist_title, &playlists);
        playlists.push(json!({
            "id": &playlist_id,
            "title": playlist_title,
            "itemIds": [&item_id],
            "items": [{
                "id": format!("presentation-{item_id}"),
                "type": "presentation",
                "itemId": &item_id,
            }],
        }));
        playlist_id
    });

    let workspace = save_workspace(
        path,
        &json!({
            "libraries": libraries,
            "library": library_items,
            "playlist": playlists,
        }),
    )?;
    Ok(PresentationLibraryMutationResult {
        workspace,
        changed: true,
        created_library_id: None,
        created_item_id: Some(item_id),
        created_playlist_id,
        created_slide_id: None,
        selected_slide_id: None,
        created_arrangement_id: None,
        copied_arrangement_id: None,
        deleted_library_ids: Vec::new(),
        deleted_item_ids: Vec::new(),
        copied_item_ids: Vec::new(),
        moved_item_ids: Vec::new(),
    })
}

fn optional_trimmed_mutation_string(
    mutation: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, String> {
    match mutation.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => {
            Ok((!value.trim().is_empty()).then(|| value.trim().to_string()))
        }
        Some(_) => Err(format!("Presentation creation {key} must be a string.")),
    }
}

pub fn split_presentation_text(
    content: &str,
    split_mode: &str,
    split_marker: &str,
    split_units: f64,
) -> Vec<String> {
    let split_units = split_units.max(1.0);
    let source = content.trim();
    let entries = match split_mode {
        "marker" => source
            .split(if split_marker.is_empty() {
                "//"
            } else {
                split_marker
            })
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>(),
        "line" => source
            .split('\n')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>(),
        _ => split_native_presentation_paragraphs(source),
    };
    let mut sections = Vec::<String>::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let target = ((index as f64) / split_units).floor() as usize;
        if target == sections.len() {
            sections.push(entry);
        } else if let Some(section) = sections.get_mut(target) {
            section.push('\n');
            section.push_str(&entry);
        }
    }
    if sections.is_empty() {
        sections.push(String::new());
    }
    sections
}

fn split_native_presentation_paragraphs(source: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut paragraph = String::new();
    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);
        if content.trim().is_empty() {
            if !paragraph.is_empty() {
                paragraphs.push(paragraph.trim().to_string());
                paragraph.clear();
            }
        } else {
            paragraph.push_str(line);
        }
    }
    if !paragraph.is_empty() {
        paragraphs.push(paragraph.trim().to_string());
    }
    paragraphs
}

fn unique_created_presentation_identity(
    base_title: &str,
    items: &[Value],
    target_item_ids: &[Value],
) -> (String, String) {
    let titles = items
        .iter()
        .filter_map(|item| item.get("title").and_then(Value::as_str))
        .map(str::trim)
        .collect::<HashSet<_>>();
    let used_ids = items
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let target_ids = target_item_ids
        .iter()
        .filter_map(Value::as_str)
        .collect::<HashSet<_>>();
    let storage_names = items
        .iter()
        .filter(|item| {
            item.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| target_ids.contains(id))
        })
        .filter_map(|item| {
            let id = item.get("id")?.as_str()?;
            let title = item.get("title").and_then(Value::as_str).unwrap_or(id);
            Some(safe_name(title, id).to_ascii_lowercase())
        })
        .collect::<HashSet<_>>();

    let mut suffix = 1;
    loop {
        let title = if suffix == 1 {
            base_title.to_string()
        } else {
            format!("{base_title} {suffix}")
        };
        suffix += 1;
        if titles.contains(title.as_str()) {
            continue;
        }
        let slug = playlist_slug(&title);
        let item_id = unique_copy_id(
            if slug.is_empty() {
                "presentation"
            } else {
                slug.as_str()
            },
            &used_ids,
        );
        if storage_names.contains(&safe_name(&title, &item_id).to_ascii_lowercase()) {
            continue;
        }
        return (title, item_id);
    }
}

fn validate_presentation_item_mutation_keys(
    mutation: &Map<String, Value>,
    operation: &Map<String, Value>,
    operation_type: &str,
) -> Result<(), String> {
    let allowed_outer = ["type", "itemId", "operation"];
    if let Some(key) = mutation
        .keys()
        .find(|key| !allowed_outer.contains(&key.as_str()))
    {
        return Err(format!(
            "Unsupported presentation item mutation field '{key}'."
        ));
    }
    let allowed_operation = match operation_type {
        "setSlideDocument" => &["type", "slides", "title"][..],
        "setTitle" => &["type", "title"][..],
        "setPresentationSetting" => &["type", "field", "value"][..],
        "setSlideShortcut" => &["type", "slideId", "shortcut"][..],
        "setTimeline" => &["type", "timeline"][..],
        "setOperatorNotes" => &["type", "operatorNotes"][..],
        "setDestination" => &["type", "destination"][..],
        "setTransition" => &["type", "slideId", "transition"][..],
        "setSlideContents" => &["type", "changes"][..],
        "setSlideEnabled" => &["type", "slideId", "enabled"][..],
        "setSlidesEnabled" => &["type", "slideIds", "enabled"][..],
        "setSlideMedia" => &["type", "slideId", "mediaId", "backgroundOnly"][..],
        "setSlideTheme" => &["type", "slideId", "assignment"][..],
        "setSlidesTheme" => &["type", "slideIds", "assignment"][..],
        "setSlideGroup" => &["type", "slideId", "groupId"][..],
        "setSlidesGroup" => &["type", "slideIds", "groupId"][..],
        "setSlidesLabel" => &["type", "slideIds", "label"][..],
        "splitSlide" => &["type", "slideId", "beforeContent", "afterContent"][..],
        "mergeSlides" => &["type", "keepSlideId", "removeSlideId", "content"][..],
        "pasteSlide" => &["type", "afterSlideId", "slide"][..],
        "pasteSlides" => &["type", "afterSlideId", "slides"][..],
        "insertBlankSlide" => &["type", "afterSlideId"][..],
        "insertMediaSlide" => &["type", "beforeSlideId", "mediaId"][..],
        "insertMediaSlides" => &["type", "slideId", "placement", "mediaIds", "newSlideIds"][..],
        "reorderSlide" => &["type", "slideId", "targetIndex"][..],
        "reorderSlides" => &["type", "slideIds", "targetIndex"][..],
        "deleteSlide" => &["type", "slideId"][..],
        "deleteSlides" => &["type", "slideIds"][..],
        _ => return Err("Unsupported presentation item mutation operation.".to_string()),
    };
    if let Some(key) = operation
        .keys()
        .find(|key| !allowed_operation.contains(&key.as_str()))
    {
        return Err(format!(
            "Unsupported presentation item mutation operation field '{key}'."
        ));
    }
    Ok(())
}

fn mutate_presentation_item(
    path: &Path,
    workspace: Value,
    mutation: &Map<String, Value>,
) -> Result<PresentationLibraryMutationResult, String> {
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut library = value_array(document, "library")?;
    let item_id = required_mutation_string(
        mutation,
        "itemId",
        "Presentation item mutation item id is required.",
    )?;
    let item_index = library
        .iter()
        .position(|item| item.get("id").and_then(Value::as_str) == Some(item_id))
        .ok_or_else(|| "Presentation item mutation item was not found.".to_string())?;
    let operation = mutation
        .get("operation")
        .and_then(Value::as_object)
        .ok_or_else(|| "Presentation item mutation operation is required.".to_string())?;
    let operation_type = operation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Presentation item mutation operation type is required.".to_string())?;
    validate_presentation_item_mutation_keys(mutation, operation, operation_type)?;
    let original_item = library
        .get(item_index)
        .cloned()
        .ok_or_else(|| "Presentation item mutation item was not found.".to_string())?;
    let mut next_item = original_item.clone();
    if matches!(
        operation_type,
        "setSlideContents" | "splitSlide" | "mergeSlides"
    ) {
        if let Some(slides) = next_item.get_mut("slides").and_then(Value::as_array_mut) {
            for slide in slides {
                let id = slide.get("id").and_then(Value::as_str);
                let targeted = if operation_type == "setSlideContents" {
                    operation
                        .get("changes")
                        .and_then(Value::as_array)
                        .is_some_and(|changes| {
                            changes.iter().any(|change| {
                                change.get("slideId").and_then(Value::as_str) == id
                                    && change.get("content") != slide.get("content")
                            })
                        })
                } else {
                    ["slideId", "keepSlideId", "removeSlideId"]
                        .iter()
                        .any(|key| {
                            operation
                                .get(*key)
                                .and_then(Value::as_str)
                                .is_some_and(|target| Some(target) == id)
                        })
                };
                if !targeted {
                    continue;
                }
                let theme = slide
                    .get("presentationThemeId")
                    .and_then(Value::as_str)
                    .and_then(|id| content_themes::layout(document, id));
                slide_text_edit::prepare(slide, theme);
            }
        }
    }
    let mut created_slide_id = None;
    let mut selected_slide_id = None;

    match operation_type {
        "setTimeline" => {
            let timeline = operation.get("timeline").ok_or("Timeline is required.")?;
            crate::native_timeline::validate(timeline, &next_item["slides"])?;
            crate::native_timeline::validate_occurrences(timeline, &next_item)?;
            crate::native_timeline::validate_soundtrack(
                timeline,
                document
                    .get("media")
                    .and_then(Value::as_array)
                    .map_or(&[][..], Vec::as_slice),
            )?;
            next_item
                .as_object_mut()
                .ok_or("Presentation must be an object.")?
                .insert("timeline".to_string(), timeline.clone());
        }
        "setSlideDocument" => {
            let title =
                required_mutation_string(operation, "title", "Presentation title is required.")?;
            let slides = operation
                .get("slides")
                .and_then(Value::as_array)
                .ok_or_else(|| "Presentation slides must be an array.".to_string())?;
            let mut ids = std::collections::HashSet::new();
            for slide in slides {
                crate::native_automation::validate_slide_completion(slide)?;
                let id = slide
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| "Presentation slide id is required.".to_string())?;
                if !ids.insert(id) {
                    return Err("Presentation slide ids must be unique.".to_string());
                }
                if slide.get("notes").is_some_and(|notes| !notes.is_string()) {
                    return Err("Slide notes must be a string.".to_string());
                }
                if let Some(runs) = slide.get("notesRuns") {
                    let runs = runs
                        .as_array()
                        .ok_or_else(|| "Slide notes runs must be an array.".to_string())?;
                    if runs
                        .iter()
                        .any(|run| !run.get("text").is_some_and(Value::is_string))
                    {
                        return Err("Slide notes run text must be a string.".to_string());
                    }
                }
                validate_content_runs(slide)?;
                if let Some(layers) = slide.get("layers") {
                    let layers = layers
                        .as_array()
                        .ok_or_else(|| "Slide layers must be an array.".to_string())?;
                    let mut layer_ids = std::collections::HashSet::new();
                    for layer in layers {
                        if let Some(source) = layer.get("ndiSource") {
                            let source = serde_json::from_value(source.clone())
                                .map_err(|error| format!("Invalid NDI source: {error}"))?;
                            crate::ndi_input::validate_source(&source)?;
                        }
                        let id = layer
                            .get("id")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                            .ok_or_else(|| "Slide layer id is required.".to_string())?;
                        if !layer_ids.insert(id) {
                            return Err("Slide layer ids must be unique.".to_string());
                        }
                    }
                }
            }
            set_presentation_item_slides(&mut next_item, slides.clone())?;
            next_item
                .as_object_mut()
                .ok_or_else(|| "Presentation must be an object.".to_string())?
                .insert("title".to_string(), json!(title));
            normalize_presentation_item_groups(&mut next_item);
        }
        "setTransition" => {
            let transition = operation
                .get("transition")
                .ok_or("Transition is required; use null to remove it.")?;
            if !transition.is_null() {
                crate::native_transitions::validate(transition)?;
            }
            // Without a slide id the transition applies to the whole presentation.
            let target = match operation.get("slideId") {
                Some(slide_id) => next_item["slides"]
                    .as_array_mut()
                    .and_then(|slides| {
                        slides
                            .iter_mut()
                            .find(|slide| slide.get("id") == Some(slide_id))
                    })
                    .ok_or("Transition slide was not found.")?,
                None => &mut next_item,
            }
            .as_object_mut()
            .ok_or("Transition target must be an object.")?;
            if transition.is_null() {
                target.remove("transition");
            } else {
                target.insert("transition".to_string(), transition.clone());
            }
        }
        "setPresentationSetting" => {
            let field = required_mutation_string(
                operation,
                "field",
                "Presentation setting field is required.",
            )?;
            let value = operation
                .get("value")
                .ok_or("Presentation setting value is required.")?;
            crate::presentation_settings::patch(&mut next_item, field, value)?;
        }
        "setSlideShortcut" => {
            let slide_id = required_mutation_string(
                operation,
                "slideId",
                "Slide shortcut slide id is required.",
            )?;
            let key = crate::presentation_settings::shortcut(
                operation
                    .get("shortcut")
                    .ok_or("Slide shortcut is required.")?,
            )?;
            let mut slides = presentation_item_slides(&next_item)?;
            let index = find_presentation_slide_index(&slides, slide_id)?;
            if !key.is_empty()
                && slides.iter().enumerate().any(|(i, slide)| {
                    i != index
                        && slide["shortcut"]
                            .as_str()
                            .is_some_and(|existing| existing.eq_ignore_ascii_case(&key))
                })
            {
                return Err("Another slide already uses this shortcut.".into());
            }
            let slide = slides[index]
                .as_object_mut()
                .ok_or("Presentation slide must be an object.")?;
            if key.is_empty() {
                slide.remove("shortcut");
            } else {
                slide.insert("shortcut".into(), json!(key));
            }
            set_presentation_item_slides(&mut next_item, slides)?;
        }
        "setTitle" => {
            let title = required_mutation_string(
                operation,
                "title",
                "Presentation item title is required.",
            )?;
            if next_item.get("title").and_then(Value::as_str) != Some(title) {
                next_item
                    .as_object_mut()
                    .ok_or_else(|| "Presentation item must be an object.".to_string())?
                    .insert("title".to_string(), json!(title));
            }
        }
        "setDestination" => {
            let item = next_item
                .as_object_mut()
                .ok_or_else(|| "Presentation item must be an object.".to_string())?;
            match operation.get("destination").and_then(Value::as_str) {
                Some("announcements") => {
                    item.insert("destination".to_string(), json!("announcements"));
                }
                Some("presentation") => {
                    item.remove("destination");
                }
                _ => {
                    return Err(
                        "Presentation destination must be presentation or announcements.".into(),
                    )
                }
            }
        }
        "setOperatorNotes" => {
            let operator_notes = mutation_string(
                operation,
                "operatorNotes",
                "Presentation item operator notes must be a string.",
            )?
            .trim()
            .to_string();
            let current_notes = next_item
                .get("operatorNotes")
                .filter(|value| !value.is_null())
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            if operator_notes != current_notes {
                next_item
                    .as_object_mut()
                    .ok_or_else(|| "Presentation item must be an object.".to_string())?
                    .insert("operatorNotes".to_string(), json!(operator_notes));
            }
        }
        "setSlideContents" => {
            let changes = operation
                .get("changes")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    "Presentation slide content changes must be an array.".to_string()
                })?;
            if changes.is_empty() {
                return Ok(unchanged_presentation_library_result(workspace));
            }
            let slides = presentation_item_slides(&next_item)?;
            let slide_ids = presentation_slide_ids(&slides)?;
            let mut parsed_changes = Vec::with_capacity(changes.len());
            let mut change_ids = HashSet::with_capacity(changes.len());
            for change in changes {
                let change = change.as_object().ok_or_else(|| {
                    "Presentation slide content changes must be objects.".to_string()
                })?;
                if let Some(key) = change
                    .keys()
                    .find(|key| !["slideId", "content"].contains(&key.as_str()))
                {
                    return Err(format!(
                        "Unsupported presentation slide content change field '{key}'."
                    ));
                }
                let slide_id = required_mutation_string(
                    change,
                    "slideId",
                    "Presentation slide content slide id is required.",
                )?;
                if !slide_ids.contains(slide_id) {
                    return Err(format!("Presentation slide '{slide_id}' was not found."));
                }
                if !change_ids.insert(slide_id.to_string()) {
                    return Err(format!(
                        "Presentation slide content change '{slide_id}' was provided more than once."
                    ));
                }
                let content = mutation_string(
                    change,
                    "content",
                    "Presentation slide content must be a string.",
                )?;
                parsed_changes.push((slide_id.to_string(), content.to_string()));
            }

            let mut next_slides = slides;
            for (slide_id, content) in parsed_changes {
                let slide = next_slides
                    .iter_mut()
                    .find(|slide| {
                        slide.get("id").and_then(Value::as_str) == Some(slide_id.as_str())
                    })
                    .ok_or_else(|| format!("Presentation slide '{slide_id}' was not found."))?;
                let slide_object = slide
                    .as_object_mut()
                    .ok_or_else(|| "Presentation slides must be objects.".to_string())?;
                let current_content = slide_object
                    .get("content")
                    .filter(|value| !value.is_null())
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if current_content != content {
                    set_slide_content(slide_object, &content);
                }
            }
            set_presentation_item_slides(&mut next_item, next_slides)?;
            normalize_presentation_item_groups(&mut next_item);
        }
        "setSlideEnabled" | "setSlidesEnabled" => {
            let enabled = operation
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| "Presentation slide enabled state must be a boolean.".to_string())?;
            let mut slides = presentation_item_slides(&next_item)?;
            for index in slide_mutation_indices(&slides, operation)? {
                let slide = slides[index]
                    .as_object_mut()
                    .ok_or("Presentation slides must be objects.")?;
                if enabled {
                    slide.remove("disabled");
                } else {
                    slide.insert("disabled".into(), json!(true));
                }
            }
            set_presentation_item_slides(&mut next_item, slides)?;
            normalize_presentation_item_groups(&mut next_item);
        }
        "setSlideMedia" | "insertMediaSlide" | "insertMediaSlides" => {
            created_slide_id = apply_presentation_media_edit(document, &mut next_item, operation)?;
        }
        "setSlideTheme" | "setSlidesTheme" => {
            let assignment = required_mutation_string(
                operation,
                "assignment",
                "Choose an available theme layout.",
            )?;
            if content_themes::layout(document, assignment).is_none() {
                return Err("Choose an available theme layout.".into());
            }
            let mut slides = presentation_item_slides(&next_item)?;
            for index in slide_mutation_indices(&slides, operation)? {
                content_themes::assign(document, &mut slides[index], assignment)?;
            }
            set_presentation_item_slides(&mut next_item, slides)?;
        }
        "setSlideGroup" | "setSlidesGroup" => {
            if !operation.contains_key("groupId") {
                return Err("Presentation slide group id is required.".to_string());
            }
            let group_id = match operation.get("groupId") {
                Some(Value::Null) => None,
                Some(Value::String(value)) if !value.trim().is_empty() => {
                    Some(value.trim().to_string())
                }
                Some(_) => {
                    return Err(
                        "Presentation slide group id must be a non-empty string or null."
                            .to_string(),
                    )
                }
                None => return Err("Presentation slide group id is required.".to_string()),
            };
            let mut slides = presentation_item_slides(&next_item)?;
            for index in slide_mutation_indices(&slides, operation)? {
                let slide = slides[index]
                    .as_object_mut()
                    .ok_or("Presentation slides must be objects.")?;
                if let Some(group_id) = &group_id {
                    slide.insert("groupId".into(), json!(group_id));
                } else {
                    slide.remove("groupId");
                    slide.remove("groupName");
                    slide.remove("sectionId");
                }
            }
            set_presentation_item_slides(&mut next_item, slides)?;
            normalize_presentation_item_groups(&mut next_item);
        }
        "setSlidesLabel" => {
            let label = match operation.get("label") {
                Some(Value::Null) => None,
                Some(Value::Object(label)) => {
                    validate_mutation_keys(label, &["name", "color"], "slide label")?;
                    let name =
                        required_mutation_string(label, "name", "Slide label name is required.")?;
                    let color =
                        required_mutation_string(label, "color", "Slide label color is required.")?;
                    if !matches!(color.len(), 7 | 9)
                        || !color.starts_with('#')
                        || !color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        return Err("Slide label color must be a hex color.".into());
                    }
                    Some((name, color))
                }
                _ => return Err("Slide label must be an object or null.".into()),
            };
            let mut slides = presentation_item_slides(&next_item)?;
            for index in slide_mutation_indices(&slides, operation)? {
                let slide = slides[index]
                    .as_object_mut()
                    .ok_or("Presentation slides must be objects.")?;
                slide.remove("label");
                slide.remove("labelColor");
                // Imported ProPresenter actions can also carry the old label.
                if let Some(actions) = slide.get_mut("actions").and_then(Value::as_array_mut) {
                    for action in actions
                        .iter_mut()
                        .filter(|action| action["type"] == "presentation-slide")
                    {
                        if let Some(details) = action["details"].as_object_mut() {
                            details.remove("label");
                            details.remove("labelColor");
                        }
                    }
                }
                if let Some((name, color)) = label {
                    slide.insert("label".into(), json!(name));
                    slide.insert("labelColor".into(), json!(color));
                }
            }
            set_presentation_item_slides(&mut next_item, slides)?;
        }
        "splitSlide" => {
            let slide_id = required_mutation_string(
                operation,
                "slideId",
                "Presentation slide split slide id is required.",
            )?;
            let before_content = mutation_string(
                operation,
                "beforeContent",
                "Presentation slide split before content must be a string.",
            )?;
            let after_content = mutation_string(
                operation,
                "afterContent",
                "Presentation slide split after content must be a string.",
            )?;
            let mut slides = presentation_item_slides(&next_item)?;
            let slide_index = find_presentation_slide_index(&slides, slide_id)?;
            let new_id = next_presentation_slide_id(&slides, &format!("{slide_id}-line"))?;
            let current_slide = slides
                .get(slide_index)
                .cloned()
                .ok_or_else(|| "Presentation slide was not found.".to_string())?;
            let mut next_slide = current_slide.clone();
            current_slide_with_content(&mut slides[slide_index], before_content)?;
            let next_object = next_slide
                .as_object_mut()
                .ok_or_else(|| "Presentation slides must be objects.".to_string())?;
            next_object.insert("id".to_string(), json!(&new_id));
            slide_text_edit::set_content(next_object, after_content, true);
            next_object.remove("notes");
            next_object.remove("notesRuns");
            next_object.remove("notesTextStyle");
            next_object.remove("actions");
            next_object.remove("shortcut");
            let mut layer_ids = std::collections::HashMap::new();
            if let Some(layers) = next_object.get_mut("layers").and_then(Value::as_array_mut) {
                for layer in layers {
                    if let Some(object) = layer.as_object_mut() {
                        if let Some(id) =
                            object.get("id").and_then(Value::as_str).map(str::to_string)
                        {
                            let renamed = format!("{new_id}-{id}");
                            object.insert("id".to_string(), json!(&renamed));
                            layer_ids.insert(id, renamed);
                        }
                    }
                }
            }
            if let Some(primary) = next_object
                .get("primaryTextLayerId")
                .and_then(Value::as_str)
            {
                if let Some(renamed) = layer_ids.get(primary) {
                    next_object.insert("primaryTextLayerId".to_string(), json!(renamed));
                }
            }
            if let Some(order) = next_object
                .get_mut("themeLayerOrder")
                .and_then(Value::as_array_mut)
            {
                for id in order {
                    if let Some(renamed) = id.as_str().and_then(|id| layer_ids.get(id)) {
                        *id = json!(renamed);
                    }
                }
            }
            slides.insert(slide_index + 1, next_slide);
            set_presentation_item_slides(&mut next_item, slides)?;
            normalize_presentation_item_groups(&mut next_item);
            created_slide_id = Some(new_id.clone());
            selected_slide_id = Some(new_id);
        }
        "mergeSlides" => {
            let keep_slide_id = required_mutation_string(
                operation,
                "keepSlideId",
                "Presentation slide merge keep slide id is required.",
            )?;
            let remove_slide_id = required_mutation_string(
                operation,
                "removeSlideId",
                "Presentation slide merge remove slide id is required.",
            )?;
            if keep_slide_id == remove_slide_id {
                return Err("Presentation slide merge requires two different slides.".to_string());
            }
            let content = mutation_string(
                operation,
                "content",
                "Presentation slide merge content must be a string.",
            )?;
            let mut slides = presentation_item_slides(&next_item)?;
            let keep_index = find_presentation_slide_index(&slides, keep_slide_id)?;
            let remove_index = find_presentation_slide_index(&slides, remove_slide_id)?;
            if keep_index.abs_diff(remove_index) != 1 {
                return Err("Presentation slide merge targets must be adjacent.".to_string());
            }
            let keep_group = effective_presentation_slide_group_id(&slides, keep_index);
            let remove_group = effective_presentation_slide_group_id(&slides, remove_index);
            if keep_group == "no-group" || keep_group != remove_group {
                return Err("Presentation slides do not share a mergeable group.".to_string());
            }
            if crate::slide_composition::merge_conflict(&slides[keep_index], &slides[remove_index])
            {
                return Err("Slides contain different objects, notes or actions. Resolve the differences in Edit before merging.".to_string());
            }
            if remove_index < keep_index && slides[keep_index].get("groupId").is_none() {
                let group = slides[remove_index].get("groupId").cloned();
                let section = slides[remove_index].get("sectionId").cloned();
                if let Some(object) = slides[keep_index].as_object_mut() {
                    if let Some(group) = group {
                        object.insert("groupId".to_string(), group);
                    }
                    if let Some(section) = section {
                        object.insert("sectionId".to_string(), section);
                    }
                }
            }
            slides[keep_index] =
                slide_text_edit::merge(&slides[keep_index], &slides[remove_index], content);
            slides.remove(remove_index);
            set_presentation_item_slides(&mut next_item, slides)?;
            normalize_presentation_item_groups(&mut next_item);
            selected_slide_id = Some(keep_slide_id.to_string());
        }
        "pasteSlide" | "pasteSlides" => {
            if !operation.contains_key("afterSlideId") {
                return Err("Presentation slide paste anchor is required.".to_string());
            }
            let mut slides = presentation_item_slides(&next_item)?;
            let insert_index = match operation.get("afterSlideId") {
                Some(Value::Null) => 0,
                Some(Value::String(value)) if !value.trim().is_empty() => {
                    find_presentation_slide_index(&slides, value.trim())? + 1
                }
                _ => {
                    return Err(
                        "Presentation slide paste anchor must be a non-empty string or null."
                            .to_string(),
                    )
                }
            };
            let pasted = if operation_type == "pasteSlide" {
                std::slice::from_ref(
                    operation
                        .get("slide")
                        .ok_or("Presentation pasted slide is required.")?,
                )
            } else {
                operation
                    .get("slides")
                    .and_then(Value::as_array)
                    .filter(|slides| !slides.is_empty())
                    .ok_or("Presentation pasted slides must be a non-empty array.")?
                    .as_slice()
            };
            for (offset, source) in pasted.iter().enumerate() {
                let mut slide = source
                    .as_object()
                    .cloned()
                    .ok_or("Presentation pasted slide must be an object.")?;
                let source_id = required_mutation_string(
                    &slide,
                    "id",
                    "Presentation pasted slide id is required.",
                )?;
                let new_id = next_presentation_slide_id(&slides, source_id)?;
                slide.insert("id".into(), json!(&new_id));
                slide.remove("shortcut");
                slides.insert(insert_index + offset, Value::Object(slide));
                if offset == 0 {
                    created_slide_id = Some(new_id);
                }
            }
            set_presentation_item_slides(&mut next_item, slides)?;
            normalize_presentation_item_groups(&mut next_item);
        }
        "insertBlankSlide" => {
            let mut slides = presentation_item_slides(&next_item)?;
            let insert_index =
                optional_slide_insert_after_index(&slides, operation, "afterSlideId")?;
            let new_id = next_presentation_slide_id(&slides, "new-slide")?;
            let assignment = content_themes::insertion_assignment(document, &slides, insert_index)?;
            let mut slide = json!({ "id": &new_id, "label": "", "groupName": "", "content": "" });
            content_themes::assign(document, &mut slide, &assignment)?;
            crate::presentation_settings::apply_defaults(&next_item, &mut slide)?;
            slides.insert(insert_index, slide);
            set_presentation_item_slides(&mut next_item, slides)?;
            normalize_presentation_item_groups(&mut next_item);
            created_slide_id = Some(new_id);
        }
        "reorderSlide" | "reorderSlides" => {
            let target_index = mutation_index(
                operation,
                "targetIndex",
                "Presentation slide reorder target index is required.",
            )?;
            let slides = presentation_item_slides(&next_item)?;
            let sources = slide_mutation_indices(&slides, operation)?;
            let next_index = target_index.clamp(0, (slides.len() - sources.len()) as i64) as usize;
            set_presentation_item_slides(
                &mut next_item,
                reordered_slides(slides, &sources, next_index),
            )?;
            normalize_presentation_item_groups(&mut next_item);
        }
        "deleteSlide" | "deleteSlides" => {
            apply_presentation_slide_deletion(&mut next_item, operation)?;
        }
        _ => return Err("Unsupported presentation item mutation operation.".to_string()),
    }

    let remembers_assignment = !matches!(operation_type, "setSlideTheme" | "setSlidesTheme")
        || document.get("lastUsedPresentationThemeId") == operation.get("assignment");
    if next_item == original_item && remembers_assignment {
        return Ok(unchanged_presentation_library_result(workspace));
    }
    library[item_index] = next_item;
    let mut patch = json!({ "library": library });
    if matches!(operation_type, "setSlideTheme" | "setSlidesTheme") {
        patch["lastUsedPresentationThemeId"] = operation["assignment"].clone();
    }
    let workspace = save_read_workspace(path, workspace, &patch, &[])?;
    Ok(PresentationLibraryMutationResult {
        workspace,
        changed: true,
        created_library_id: None,
        created_item_id: None,
        created_playlist_id: None,
        created_slide_id,
        selected_slide_id,
        created_arrangement_id: None,
        copied_arrangement_id: None,
        deleted_library_ids: Vec::new(),
        deleted_item_ids: Vec::new(),
        copied_item_ids: Vec::new(),
        moved_item_ids: Vec::new(),
    })
}

// The editor previews these edits with the same rules the workspace uses when saving.
pub(crate) fn apply_presentation_media_edit(
    document: &Map<String, Value>,
    item: &mut Value,
    operation: &Map<String, Value>,
) -> Result<Option<String>, String> {
    let operation_type = operation
        .get("type")
        .and_then(Value::as_str)
        .ok_or("Presentation media operation is required.")?;
    let mut created_slide_id = None;
    match operation_type {
        "setSlideMedia" => {
            if !operation.contains_key("mediaId") {
                return Err("Presentation slide media id is required.".to_string());
            }
            let slide_id = required_mutation_string(
                operation,
                "slideId",
                "Presentation slide media slide id is required.",
            )?;
            let background_only = match operation.get("backgroundOnly") {
                None => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => {
                    return Err("Presentation slide backgroundOnly must be a boolean.".into())
                }
            };
            let media_id = match operation.get("mediaId") {
                Some(Value::Null) => None,
                None => {
                    return Err(
                        "Presentation slide media id must be a non-empty string or null."
                            .to_string(),
                    )
                }
                Some(Value::String(value)) if !value.trim().is_empty() => {
                    let media_id = value.trim();
                    let media = document
                        .get("media")
                        .and_then(Value::as_array)
                        .ok_or("Workspace media must be an array.")?;
                    let valid_media = media.iter().any(|entry| {
                        entry.get("id").and_then(Value::as_str) == Some(media_id)
                            && matches!(
                                entry.get("type").and_then(Value::as_str),
                                Some("image" | "video")
                            )
                    });
                    if !valid_media {
                        return Err(format!("Presentation media '{media_id}' was not found."));
                    }
                    Some(media_id.to_string())
                }
                Some(_) => {
                    return Err(
                        "Presentation slide media id must be a non-empty string or null."
                            .to_string(),
                    )
                }
            };
            let mut slides = presentation_item_slides(item)?;
            let slide_index = find_presentation_slide_index(&slides, slide_id)?;
            let slide = slides
                .get_mut(slide_index)
                .ok_or_else(|| "Presentation slide was not found.".to_string())?;
            let slide_object = slide
                .as_object_mut()
                .ok_or_else(|| "Presentation slides must be objects.".to_string())?;
            if let Some(media_id) = media_id {
                if slide_object
                    .get("mediaBackgroundId")
                    .and_then(Value::as_str)
                    != Some(media_id.as_str())
                {
                    slide_object.insert("mediaBackgroundId".to_string(), json!(media_id));
                }
            } else {
                slide_object.remove("mediaBackgroundId");
            }
            if background_only && slide_object.contains_key("mediaBackgroundId") {
                slide_object.insert("mediaBackgroundOnly".to_string(), json!(true));
            } else {
                slide_object.remove("mediaBackgroundOnly");
            }
            set_presentation_item_slides(item, slides)?;
            normalize_presentation_item_groups(item);
        }
        "insertMediaSlide" | "insertMediaSlides" => {
            let batch = operation_type == "insertMediaSlides";
            let media_ids = if !batch {
                vec![required_mutation_string(
                    operation,
                    "mediaId",
                    "Presentation media slide media id is required.",
                )?]
            } else {
                operation
                    .get("mediaIds")
                    .and_then(Value::as_array)
                    .filter(|ids| !ids.is_empty())
                    .ok_or("Presentation media slide ids must be a non-empty array.")?
                    .iter()
                    .map(|id| {
                        id.as_str()
                            .map(str::trim)
                            .filter(|id| !id.is_empty())
                            .ok_or("Presentation media slide ids must be non-empty strings.")
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };
            let media = document
                .get("media")
                .and_then(Value::as_array)
                .ok_or("Workspace media must be an array.")?;
            for &media_id in &media_ids {
                let valid_media = media.iter().any(|entry| {
                    entry.get("id").and_then(Value::as_str) == Some(media_id)
                        && matches!(
                            entry.get("type").and_then(Value::as_str),
                            Some("image" | "video")
                        )
                });
                if !valid_media {
                    return Err(format!("Presentation media '{media_id}' was not found."));
                }
            }
            let mut slides = presentation_item_slides(item)?;
            let (insert_index, group) = if batch {
                let slide_id = required_mutation_string(
                    operation,
                    "slideId",
                    "Presentation media target slide id is required.",
                )?;
                let anchor = find_presentation_slide_index(&slides, slide_id)?;
                let after = match operation.get("placement").and_then(Value::as_str) {
                    Some("before") => false,
                    Some("after") => true,
                    _ => return Err("Presentation media placement must be before or after.".into()),
                };
                let group = slides[..=anchor]
                    .iter()
                    .rev()
                    .find_map(crate::output_arrangement::slide_group_id);
                (anchor + usize::from(after), group)
            } else {
                (
                    optional_slide_insert_before_index(&slides, operation, "beforeSlideId")?,
                    None,
                )
            };
            let assignment = content_themes::insertion_assignment(document, &slides, insert_index)?;
            let new_ids = if operation.contains_key("newSlideIds") {
                let requested = required_mutation_string_array(
                    operation,
                    "newSlideIds",
                    "New presentation slide ids are required.",
                )?;
                let mut used = presentation_slide_ids(&slides)?;
                if requested.len() != media_ids.len()
                    || requested.iter().any(|id| !used.insert(id.clone()))
                {
                    return Err("Each media slide requires a unique new slide id.".into());
                }
                Some(requested)
            } else {
                None
            };
            for (offset, media_id) in media_ids.into_iter().enumerate() {
                let new_id = match &new_ids {
                    Some(ids) => ids[offset].clone(),
                    None => next_presentation_slide_id(&slides, "media")?,
                };
                let mut slide =
                    json!({ "id": &new_id, "content": "", "mediaBackgroundId": media_id });
                if batch {
                    slide["mediaBackgroundOnly"] = json!(true);
                }
                if let Some(group) = &group {
                    slide["groupId"] = json!(group);
                }
                content_themes::assign(document, &mut slide, &assignment)?;
                crate::presentation_settings::apply_defaults(item, &mut slide)?;
                slides.insert(insert_index + offset, slide);
                created_slide_id.get_or_insert(new_id);
            }
            set_presentation_item_slides(item, slides)?;
            normalize_presentation_item_groups(item);
        }
        _ => return Err("Unsupported presentation media operation.".into()),
    }
    Ok(created_slide_id)
}

pub(crate) fn apply_presentation_slide_deletion(
    item: &mut Value,
    operation: &Map<String, Value>,
) -> Result<(), String> {
    let mut slides = presentation_item_slides(item)?;
    for removed_index in slide_mutation_indices(&slides, operation)?
        .into_iter()
        .rev()
    {
        let removed_slide = slides
            .get(removed_index)
            .cloned()
            .ok_or_else(|| "Presentation slide was not found.".to_string())?;
        let removed_group_id = presentation_slide_group_id(Some(&removed_slide));
        let should_promote_next = removed_group_id
            .as_deref()
            .is_some_and(|group_id| group_id != "no-group")
            && slides
                .get(removed_index + 1)
                .is_some_and(|slide| presentation_slide_group_id(Some(slide)).is_none());
        slides.remove(removed_index);
        if should_promote_next {
            let next_slide = slides
                .get_mut(removed_index)
                .ok_or_else(|| "Presentation slide was not found.".to_string())?;
            let next_object = next_slide
                .as_object_mut()
                .ok_or_else(|| "Presentation slides must be objects.".to_string())?;
            if let Some(group_id) = removed_group_id {
                next_object.insert("groupId".to_string(), json!(group_id));
            }
            if let Some(section_id) = removed_slide
                .get("sectionId")
                .filter(|value| value.as_str().is_some_and(|value| !value.is_empty()))
            {
                next_object.insert("sectionId".to_string(), section_id.clone());
            }
        }
    }
    set_presentation_item_slides(item, slides)?;
    normalize_presentation_item_groups(item);
    Ok(())
}

fn presentation_item_slides(item: &Value) -> Result<Vec<Value>, String> {
    match item.get("slides") {
        None => Ok(Vec::new()),
        Some(Value::Array(slides)) => Ok(slides.clone()),
        Some(_) => Err("Presentation item slides must be an array.".to_string()),
    }
}

// The target is an index in the remaining list. The selected slides keep their master order.
pub fn reordered_slides(slides: Vec<Value>, sources: &[usize], target: usize) -> Vec<Value> {
    let (moved, mut remaining): (Vec<_>, Vec<_>) = slides
        .into_iter()
        .enumerate()
        .partition(|(index, _)| sources.binary_search(index).is_ok());
    remaining.splice(target..target, moved);
    remaining.into_iter().map(|(_, slide)| slide).collect()
}

fn slide_mutation_indices(
    slides: &[Value],
    operation: &Map<String, Value>,
) -> Result<Vec<usize>, String> {
    let ids = if operation.contains_key("slideIds") {
        required_mutation_string_array(
            operation,
            "slideIds",
            "Presentation slide ids are required.",
        )?
    } else {
        vec![
            required_mutation_string(operation, "slideId", "Presentation slide id is required.")?
                .to_owned(),
        ]
    };
    let mut wanted = HashSet::with_capacity(ids.len());
    for id in &ids {
        if !wanted.insert(id.as_str()) {
            return Err(format!(
                "Presentation slide '{id}' was provided more than once."
            ));
        }
    }
    presentation_slide_ids(slides)?;
    let indices = slides
        .iter()
        .enumerate()
        .filter_map(|(index, slide)| {
            let id = slide["id"].as_str()?;
            wanted.remove(id).then_some(index)
        })
        .collect();
    if let Some(id) = wanted.into_iter().next() {
        return Err(format!("Presentation slide '{id}' was not found."));
    }
    Ok(indices)
}

fn presentation_slide_ids(slides: &[Value]) -> Result<HashSet<String>, String> {
    let mut ids = HashSet::with_capacity(slides.len());
    for slide in slides {
        let id = slide
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "Presentation slide id is required.".to_string())?;
        if !ids.insert(id.to_string()) {
            return Err(format!("Duplicate presentation slide id '{id}'."));
        }
    }
    Ok(ids)
}

fn find_presentation_slide_index(slides: &[Value], slide_id: &str) -> Result<usize, String> {
    presentation_slide_ids(slides)?;
    slides
        .iter()
        .position(|slide| slide.get("id").and_then(Value::as_str) == Some(slide_id))
        .ok_or_else(|| format!("Presentation slide '{slide_id}' was not found."))
}

fn next_presentation_slide_id(slides: &[Value], base_id: &str) -> Result<String, String> {
    let ids = presentation_slide_ids(slides)?;
    Ok(unique_copy_id(base_id, &ids))
}

fn current_slide_with_content(slide: &mut Value, content: &str) -> Result<(), String> {
    set_slide_content(
        slide
            .as_object_mut()
            .ok_or_else(|| "Presentation slides must be objects.".to_string())?,
        content,
    );
    Ok(())
}

fn set_slide_content(slide: &mut Map<String, Value>, content: &str) {
    slide_text_edit::set_content(slide, content, false);
}

fn validate_content_runs(slide: &Value) -> Result<(), String> {
    let Some(runs) = slide.get("contentRuns") else {
        return Ok(());
    };
    let runs = runs
        .as_array()
        .ok_or_else(|| "Slide content runs must be an array.".to_string())?;
    let mut text = String::new();
    for run in runs {
        text.push_str(
            run.get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| "Slide content run text must be a string.".to_string())?,
        );
        let Some(style) = run.get("style") else {
            continue;
        };
        let style = style
            .as_object()
            .ok_or_else(|| "Slide content run style must be an object.".to_string())?;
        for (key, value) in style {
            let valid = match key.as_str() {
                "bold" | "italic" | "underline" | "strikethrough" => value.is_boolean(),
                "textColor" => value.as_str().is_some_and(|color| {
                    color.len() == 7
                        && color.starts_with('#')
                        && color[1..].chars().all(|digit| digit.is_ascii_hexdigit())
                }),
                "fontFamily" => value.as_str().is_some_and(|family| !family.is_empty()),
                "fontSize" => value
                    .as_f64()
                    .is_some_and(|size| size.is_finite() && size > 0.0),
                _ => false,
            };
            if !valid {
                return Err(format!(
                    "Slide content run style '{key}' is not a supported value."
                ));
            }
        }
    }
    if text
        != slide
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
    {
        return Err("Slide content runs must add up to the slide content.".to_string());
    }
    Ok(())
}

fn optional_slide_insert_after_index(
    slides: &[Value],
    operation: &Map<String, Value>,
    key: &str,
) -> Result<usize, String> {
    match operation.get(key) {
        None => {
            presentation_slide_ids(slides)?;
            Ok(slides.len())
        }
        Some(Value::String(value)) if !value.trim().is_empty() => {
            Ok(find_presentation_slide_index(slides, value.trim())? + 1)
        }
        Some(_) => {
            Err("Presentation slide insertion anchor must be a non-empty string.".to_string())
        }
    }
}

fn optional_slide_insert_before_index(
    slides: &[Value],
    operation: &Map<String, Value>,
    key: &str,
) -> Result<usize, String> {
    match operation.get(key) {
        None => {
            presentation_slide_ids(slides)?;
            Ok(slides.len())
        }
        Some(Value::String(value)) if !value.trim().is_empty() => {
            find_presentation_slide_index(slides, value.trim())
        }
        Some(_) => {
            Err("Presentation slide insertion anchor must be a non-empty string.".to_string())
        }
    }
}

fn effective_presentation_slide_group_id(slides: &[Value], index: usize) -> String {
    (0..=index.min(slides.len().saturating_sub(1)))
        .rev()
        .find_map(|candidate| presentation_slide_group_id(slides.get(candidate)))
        .unwrap_or_else(|| "no-group".to_string())
}

fn set_presentation_item_slides(item: &mut Value, slides: Vec<Value>) -> Result<(), String> {
    item.as_object_mut()
        .ok_or_else(|| "Presentation item must be an object.".to_string())?
        .insert("slides".to_string(), Value::Array(slides));
    Ok(())
}

fn normalize_presentation_item_groups(item: &mut Value) {
    if let Some(object) = item.as_object_mut() {
        object.remove("groups");
    }
}

fn mutation_string<'a>(
    mutation: &'a Map<String, Value>,
    key: &str,
    message: &str,
) -> Result<&'a str, String> {
    mutation
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| message.to_string())
}

fn validate_mutation_keys(
    mutation: &Map<String, Value>,
    allowed: &[&str],
    context: &str,
) -> Result<(), String> {
    if let Some(key) = mutation.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unsupported {context} field '{key}'."));
    }
    Ok(())
}

fn presentation_theme_assignment_mutation(
    document: &Map<String, Value>,
    mutation: &Map<String, Value>,
) -> Result<Option<Value>, String> {
    match mutation.get("assignment") {
        Some(Value::String(value)) if value == "inherit" => Ok(None),
        Some(Value::Null) => Ok(Some(Value::Null)),
        Some(Value::String(value)) if known_presentation_theme_assignment(document, value) => {
            Ok(Some(Value::String(value.clone())))
        }
        Some(_) => Err(
            "Presentation theme assignment must be a known theme id, inherit, or null.".to_string(),
        ),
        None => Err("Presentation theme assignment is required.".to_string()),
    }
}

fn set_presentation_theme_assignment(
    target: &mut Value,
    assignment: Option<&Value>,
) -> Result<(), String> {
    let object = target
        .as_object_mut()
        .ok_or_else(|| "Presentation theme target must be an object.".to_string())?;
    if let Some(assignment) = assignment {
        object.insert("presentationThemeId".to_string(), assignment.clone());
    } else {
        object.remove("presentationThemeId");
    }
    Ok(())
}

fn mutation_index(mutation: &Map<String, Value>, key: &str, message: &str) -> Result<i64, String> {
    mutation
        .get(key)
        .and_then(Value::as_i64)
        .or_else(|| {
            mutation
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|value| i64::try_from(value).ok())
        })
        .ok_or_else(|| message.to_string())
}

fn known_presentation_theme_assignment(document: &Map<String, Value>, assignment: &str) -> bool {
    let Some((theme_id, slide_id)) = assignment.split_once("::") else {
        return false;
    };
    if theme_id.is_empty() || slide_id.is_empty() {
        return false;
    }
    value_array(document, "themes")
        .unwrap_or_default()
        .iter()
        .any(|theme| {
            theme.get("id").and_then(Value::as_str) == Some(theme_id)
                && theme
                    .get("slides")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|slide| slide.get("id").and_then(Value::as_str) == Some(slide_id))
        })
}

fn presentation_slide_group_id(slide: Option<&Value>) -> Option<String> {
    let slide = slide?.as_object()?;
    let value = if slide.contains_key("groupId") {
        slide.get("groupId")
    } else {
        slide.get("groupName")
    }?;
    let value = match value {
        Value::String(value) => value.trim().to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => return None,
        _ => return None,
    };
    let value = value.trim();
    if value.is_empty() || is_generated_slide_metadata_label(value) {
        return None;
    }
    Some(value.to_string())
}

fn is_generated_slide_metadata_label(value: &str) -> bool {
    if value.len() >= 8 && value.chars().all(|character| character.is_ascii_digit()) {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    if lower == "new-slide" || lower == "new-slide-copy" {
        return true;
    }
    lower.strip_prefix("new-slide-copy-").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
    })
}

fn mutate_presentation_arrangement(
    path: &Path,
    workspace: Value,
    mutation: &Map<String, Value>,
    operation: &str,
) -> Result<PresentationLibraryMutationResult, String> {
    validate_presentation_arrangement_mutation_keys(mutation, operation)?;
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut library = value_array(document, "library")?;
    let item_id = required_mutation_string(
        mutation,
        "itemId",
        "Presentation arrangement item id is required.",
    )?;
    let item_index = library
        .iter()
        .position(|item| item.get("id").and_then(Value::as_str) == Some(item_id))
        .ok_or_else(|| "Presentation arrangement item was not found.".to_string())?;
    let item = library
        .get(item_index)
        .ok_or_else(|| "Presentation arrangement item was not found.".to_string())?;
    let original_item = item.clone();
    let mut next_item = item.clone();
    let mut arrangements = presentation_arrangements(item)?;
    let known_section_ids = presentation_section_ids(item);
    let mut created_arrangement_id = None;
    let mut copied_arrangement_id = None;
    let mut next_playlists = None;

    match operation {
        "createArrangement" => {
            let name = required_mutation_string(
                mutation,
                "name",
                "Presentation arrangement name is required.",
            )?;
            let section_ids = match mutation.get("sectionIds") {
                None => known_section_ids.clone(),
                Some(value) => {
                    mutation_string_array(value, "Presentation arrangement section ids")?
                }
            };
            let arrangement_id = new_arrangement_id(mutation, &arrangements)?;
            let arrangement_name = unique_arrangement_name(name, &arrangements);
            arrangements.push(json!({
                "id": arrangement_id,
                "name": arrangement_name,
                "sectionIds": section_ids,
            }));
            created_arrangement_id = Some(arrangement_id);
        }
        "duplicateArrangement" => {
            let source_id = required_mutation_string(
                mutation,
                "arrangementId",
                "Presentation arrangement id is required.",
            )?;
            let source = arrangements
                .iter()
                .find(|arrangement| {
                    arrangement.get("id").and_then(Value::as_str) == Some(source_id)
                })
                .cloned()
                .ok_or_else(|| "Presentation arrangement was not found.".to_string())?;
            let source_object = source
                .as_object()
                .ok_or_else(|| "Presentation arrangement must be an object.".to_string())?;
            let source_name = source_object
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "Presentation arrangement name must be a string.".to_string())?;
            let source_section_ids = arrangement_section_ids(&source)?;
            let arrangement_id = new_arrangement_id(mutation, &arrangements)?;
            let arrangement_name =
                unique_arrangement_name(&format!("{source_name} Copy"), &arrangements);
            let mut copy = source_object.clone();
            copy.insert("id".to_string(), json!(&arrangement_id));
            copy.insert("name".to_string(), json!(arrangement_name));
            copy.insert("sectionIds".to_string(), json!(source_section_ids));
            arrangements.push(Value::Object(copy));
            copied_arrangement_id = Some(arrangement_id);
        }
        "renameArrangement" => {
            let arrangement_id = required_mutation_string(
                mutation,
                "arrangementId",
                "Presentation arrangement id is required.",
            )?;
            let name = required_mutation_string(
                mutation,
                "name",
                "Presentation arrangement name is required.",
            )?;
            let arrangement = arrangements
                .iter_mut()
                .find(|arrangement| {
                    arrangement.get("id").and_then(Value::as_str) == Some(arrangement_id)
                })
                .ok_or_else(|| "Presentation arrangement was not found.".to_string())?;
            arrangement
                .as_object_mut()
                .ok_or_else(|| "Presentation arrangement must be an object.".to_string())?
                .insert("name".to_string(), json!(name));
        }
        "moveArrangement" => {
            let from_index = required_mutation_index(
                mutation,
                "fromIndex",
                "Presentation arrangement source index is required.",
            )?;
            let to_index = required_mutation_index(
                mutation,
                "toIndex",
                "Presentation arrangement target index is required.",
            )?;
            if from_index == to_index || from_index < 0 || from_index as usize >= arrangements.len()
            {
                return Ok(unchanged_presentation_library_result(workspace));
            }
            let moved = arrangements.remove(from_index as usize);
            let insertion_index = to_index.clamp(0, arrangements.len() as i64) as usize;
            arrangements.insert(insertion_index, moved);
        }
        "setSelectedArrangement" => {
            let arrangement_id = mutation
                .get("arrangementId")
                .and_then(Value::as_str)
                .ok_or_else(|| "Presentation selected arrangement id is required.".to_string())?;
            if !arrangement_id.is_empty()
                && !arrangements.iter().any(|arrangement| {
                    arrangement.get("id").and_then(Value::as_str) == Some(arrangement_id)
                })
            {
                return Err("Presentation arrangement was not found.".to_string());
            }
            let object = next_item
                .as_object_mut()
                .ok_or_else(|| "Presentation library item must be an object.".to_string())?;
            if arrangement_id.is_empty() {
                object.remove("selectedArrangementId");
            } else {
                object.insert("selectedArrangementId".to_string(), json!(arrangement_id));
            }
        }
        "deleteArrangement" => {
            let arrangement_id = required_mutation_string(
                mutation,
                "arrangementId",
                "Presentation arrangement id is required.",
            )?;
            if !arrangements.iter().any(|arrangement| {
                arrangement.get("id").and_then(Value::as_str) == Some(arrangement_id)
            }) {
                return Err("Presentation arrangement was not found.".to_string());
            }
            arrangements.retain(|arrangement| {
                arrangement.get("id").and_then(Value::as_str) != Some(arrangement_id)
            });
            let object = next_item
                .as_object_mut()
                .ok_or_else(|| "Presentation library item must be an object.".to_string())?;
            if object.get("selectedArrangementId").and_then(Value::as_str) == Some(arrangement_id) {
                object.remove("selectedArrangementId");
            }

            let mut playlists = value_array(document, "playlist")?;
            for playlist in &mut playlists {
                let object = playlist
                    .as_object_mut()
                    .ok_or_else(|| "Playlist must be an object.".to_string())?;
                let entries = object
                    .get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|entry| {
                        if entry.get("type").and_then(Value::as_str) != Some("presentation")
                            || entry.get("itemId").and_then(Value::as_str) != Some(item_id)
                            || entry.get("arrangementId").and_then(Value::as_str)
                                != Some(arrangement_id)
                        {
                            return Ok(entry);
                        }
                        let mut entry = entry
                            .as_object()
                            .cloned()
                            .ok_or_else(|| "Playlist entry must be an object.".to_string())?;
                        entry.remove("arrangementId");
                        Ok(Value::Object(entry))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                object.insert("items".to_string(), Value::Array(entries));
            }
            next_playlists = Some(playlists);
        }
        "addArrangementSection"
        | "insertArrangementSection"
        | "removeArrangementSection"
        | "moveArrangementSection" => {
            let arrangement_id = required_mutation_string(
                mutation,
                "arrangementId",
                "Presentation arrangement id is required.",
            )?;
            let arrangement = arrangements
                .iter_mut()
                .find(|arrangement| {
                    arrangement.get("id").and_then(Value::as_str) == Some(arrangement_id)
                })
                .ok_or_else(|| "Presentation arrangement was not found.".to_string())?;
            let object = arrangement
                .as_object_mut()
                .ok_or_else(|| "Presentation arrangement must be an object.".to_string())?;
            let mut section_ids = arrangement_section_ids(&Value::Object(object.clone()))?;
            if let Some(expected) = mutation.get("expectedSectionIds") {
                if mutation_string_array(expected, "Expected arrangement sections")? != section_ids
                {
                    return Err(
                        "Arrangement sections changed before the edit could be saved.".into(),
                    );
                }
            }
            match operation {
                "addArrangementSection" | "insertArrangementSection" => {
                    let section_id = mutation
                        .get("sectionId")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            "Presentation arrangement section id must be a string.".to_string()
                        })?;
                    let index = if operation == "addArrangementSection" {
                        section_ids.len() as i64
                    } else {
                        required_mutation_index(
                            mutation,
                            "index",
                            "Presentation arrangement insertion index is required.",
                        )?
                    };
                    let insertion_index = index.clamp(0, section_ids.len() as i64) as usize;
                    section_ids.insert(insertion_index, section_id.to_string());
                }
                "removeArrangementSection" => {
                    let index = required_mutation_index(
                        mutation,
                        "index",
                        "Presentation arrangement section index is required.",
                    )?;
                    if index < 0 || index as usize >= section_ids.len() {
                        return Ok(unchanged_presentation_library_result(workspace));
                    }
                    section_ids.remove(index as usize);
                }
                "moveArrangementSection" => {
                    let from_index = required_mutation_index(
                        mutation,
                        "fromIndex",
                        "Presentation arrangement section source index is required.",
                    )?;
                    let to_index = required_mutation_index(
                        mutation,
                        "toIndex",
                        "Presentation arrangement section target index is required.",
                    )?;
                    if from_index == to_index
                        || from_index < 0
                        || from_index as usize >= section_ids.len()
                        || section_ids[from_index as usize].is_empty()
                    {
                        return Ok(unchanged_presentation_library_result(workspace));
                    }
                    let moved = section_ids.remove(from_index as usize);
                    let insertion_index = to_index.clamp(0, section_ids.len() as i64) as usize;
                    section_ids.insert(insertion_index, moved);
                }
                _ => unreachable!(),
            }
            object.insert("sectionIds".to_string(), json!(section_ids));
        }
        _ => return Err("Unsupported presentation arrangement mutation type.".to_string()),
    }

    if matches!(
        operation,
        "createArrangement"
            | "duplicateArrangement"
            | "renameArrangement"
            | "moveArrangement"
            | "deleteArrangement"
            | "addArrangementSection"
            | "insertArrangementSection"
            | "removeArrangementSection"
            | "moveArrangementSection"
    ) {
        next_item
            .as_object_mut()
            .ok_or_else(|| "Presentation library item must be an object.".to_string())?
            .insert("arrangements".to_string(), Value::Array(arrangements));
    }
    if next_item == original_item && next_playlists.is_none() {
        return Ok(unchanged_presentation_library_result(workspace));
    }
    library[item_index] = next_item;
    let patch = match next_playlists {
        Some(playlists) => json!({ "library": library, "playlist": playlists }),
        None => json!({ "library": library }),
    };
    let workspace = save_read_workspace(path, workspace, &patch, &[])?;
    Ok(PresentationLibraryMutationResult {
        workspace,
        changed: true,
        created_library_id: None,
        created_item_id: None,
        created_playlist_id: None,
        created_slide_id: None,
        selected_slide_id: None,
        created_arrangement_id,
        copied_arrangement_id,
        deleted_library_ids: Vec::new(),
        deleted_item_ids: Vec::new(),
        copied_item_ids: Vec::new(),
        moved_item_ids: Vec::new(),
    })
}

fn unchanged_presentation_library_result(workspace: Value) -> PresentationLibraryMutationResult {
    PresentationLibraryMutationResult {
        workspace,
        changed: false,
        created_library_id: None,
        created_item_id: None,
        created_playlist_id: None,
        created_slide_id: None,
        selected_slide_id: None,
        created_arrangement_id: None,
        copied_arrangement_id: None,
        deleted_library_ids: Vec::new(),
        deleted_item_ids: Vec::new(),
        copied_item_ids: Vec::new(),
        moved_item_ids: Vec::new(),
    }
}

fn presentation_arrangements(item: &Value) -> Result<Vec<Value>, String> {
    let Some(value) = item.get("arrangements") else {
        return Ok(Vec::new());
    };
    let arrangements = value
        .as_array()
        .ok_or_else(|| "Presentation arrangements must be an array.".to_string())?;
    let mut ids = HashSet::new();
    for arrangement in arrangements {
        let id = arrangement
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "Presentation arrangement id is required.".to_string())?;
        if !ids.insert(id.to_string()) {
            return Err(format!("Duplicate presentation arrangement id '{id}'."));
        }
        arrangement
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "Presentation arrangement name must be a string.".to_string())?;
        arrangement_section_ids(arrangement)?;
    }
    Ok(arrangements.clone())
}

fn arrangement_section_ids(arrangement: &Value) -> Result<Vec<String>, String> {
    let section_ids = arrangement
        .get("sectionIds")
        .and_then(Value::as_array)
        .ok_or_else(|| "Presentation arrangement section ids must be an array.".to_string())?;
    section_ids
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                "Presentation arrangement section ids must contain strings.".to_string()
            })
        })
        .collect()
}

fn mutation_string_array(value: &Value, label: &str) -> Result<Vec<String>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{label} must be an array."))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{label} must contain strings."))
        })
        .collect()
}

fn required_mutation_index(
    mutation: &Map<String, Value>,
    key: &str,
    message: &str,
) -> Result<i64, String> {
    mutation
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| message.to_string())
}

fn optional_mutation_index(
    mutation: &Map<String, Value>,
    key: &str,
    message: &str,
) -> Result<Option<usize>, String> {
    let Some(value) = mutation.get(key) else {
        return Ok(None);
    };
    let index = value.as_u64().ok_or_else(|| message.to_string())?;
    usize::try_from(index)
        .map(Some)
        .map_err(|_| message.to_string())
}

fn validate_presentation_arrangement_mutation_keys(
    mutation: &Map<String, Value>,
    operation: &str,
) -> Result<(), String> {
    let allowed = match operation {
        "createArrangement" => &["type", "itemId", "name", "sectionIds", "newArrangementId"][..],
        "duplicateArrangement" => &["type", "itemId", "arrangementId", "newArrangementId"][..],
        "setSelectedArrangement" | "deleteArrangement" => &["type", "itemId", "arrangementId"][..],
        "renameArrangement" => &["type", "itemId", "arrangementId", "name"][..],
        "moveArrangement" => &["type", "itemId", "fromIndex", "toIndex"][..],
        "moveArrangementSection" => &[
            "type",
            "itemId",
            "arrangementId",
            "fromIndex",
            "toIndex",
            "expectedSectionIds",
        ][..],
        "addArrangementSection" => &["type", "itemId", "arrangementId", "sectionId"][..],
        "insertArrangementSection" => &[
            "type",
            "itemId",
            "arrangementId",
            "sectionId",
            "index",
            "expectedSectionIds",
        ][..],
        "removeArrangementSection" => &[
            "type",
            "itemId",
            "arrangementId",
            "index",
            "expectedSectionIds",
        ][..],
        _ => return Err("Unsupported presentation arrangement mutation type.".to_string()),
    };
    if let Some(key) = mutation.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!(
            "Unsupported presentation arrangement mutation field '{key}'."
        ));
    }
    Ok(())
}

fn new_arrangement_id(
    mutation: &Map<String, Value>,
    arrangements: &[Value],
) -> Result<String, String> {
    let ids = arrangements
        .iter()
        .filter_map(|arrangement| arrangement.get("id").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    if let Some(value) = mutation.get("newArrangementId") {
        let id = value
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or("Arrangement id must be a UUID.")?
            .to_string();
        if ids.contains(id.as_str()) {
            return Err("Arrangement id is already in use.".into());
        }
        return Ok(id);
    }
    loop {
        let candidate = Uuid::new_v4().to_string();
        if !ids.contains(candidate.as_str()) {
            return Ok(candidate);
        }
    }
}

fn unique_arrangement_name(base_name: &str, arrangements: &[Value]) -> String {
    let names = arrangements
        .iter()
        .filter_map(|arrangement| arrangement.get("name").and_then(Value::as_str))
        .map(str::to_lowercase)
        .collect::<HashSet<_>>();
    if !names.contains(&base_name.to_lowercase()) {
        return base_name.to_string();
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base_name} {suffix}");
        if !names.contains(&candidate.to_lowercase()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn presentation_section_ids(item: &Value) -> Vec<String> {
    let slides = item
        .get("slides")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut known = Vec::new();
    let mut seen = HashSet::new();
    let mut effective_group = None;
    for (index, slide) in slides.iter().enumerate() {
        if let Some(group_id) = native_slide_group_change(slide) {
            effective_group = group_id;
        }
        let section_id = match effective_group.as_deref() {
            Some(group_id) => format!("section-{group_id}"),
            None => format!("section-{}", index + 1),
        };
        if seen.insert(section_id.clone()) {
            known.push(section_id);
        }
    }
    known
}

fn native_slide_group_change(slide: &Value) -> Option<Option<String>> {
    let object = slide.as_object()?;
    let value = if object.contains_key("groupId") {
        object.get("groupId")
    } else {
        object.get("groupName")
    }?;
    let value = value.as_str()?.trim();
    if value.is_empty() || generated_slide_metadata_label(value) {
        return None;
    }
    if value == "no-group" {
        return Some(None);
    }
    Some(Some(value.to_string()))
}

fn generated_slide_metadata_label(value: &str) -> bool {
    let is_digits = !value.is_empty() && value.chars().all(|character| character.is_ascii_digit());
    (is_digits && value.chars().count() >= 8)
        || value.eq_ignore_ascii_case("new-slide")
        || value.strip_prefix("new-slide-copy-").is_some_and(|suffix| {
            !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
        })
        || value.eq_ignore_ascii_case("new-slide-copy")
}

fn unique_presentation_library_title(base_title: &str, libraries: &[Value]) -> String {
    let titles = libraries
        .iter()
        .filter_map(|library| library.get("title").and_then(Value::as_str))
        .map(str::trim)
        .collect::<HashSet<_>>();
    if !titles.contains(base_title) {
        return base_title.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base_title} {index}");
        if !titles.contains(candidate.as_str()) {
            return candidate;
        }
        index += 1;
    }
}

fn unique_presentation_library_id(title: &str, libraries: &[Value]) -> String {
    let slug = title
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = format!("library-{}", if slug.is_empty() { "item" } else { &slug });
    let ids = libraries
        .iter()
        .filter_map(|library| library.get("id").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    let mut candidate = base.clone();
    let mut index = 2;
    while ids.contains(candidate.as_str()) {
        candidate = format!("{base}-{index}");
        index += 1;
    }
    candidate
}

pub fn mutate_presentation_playlist(
    path: &Path,
    mutation: &Value,
) -> Result<PresentationPlaylistMutationResult, String> {
    let mutation = mutation
        .as_object()
        .ok_or_else(|| "Presentation playlist mutation must be an object.".to_string())?;
    let operation = mutation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Presentation playlist mutation type is required.".to_string())?;
    let workspace = read_workspace(path)?;
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut playlists = value_array(document, "playlist")?;
    let original = playlists.clone();
    let mut created_playlist_ids = Vec::new();
    let mut deleted_playlist_ids = Vec::new();
    let mut created_entry_ids = Vec::new();
    let mut added_entry_ids = Vec::new();
    let mut renamed_entry_ids = Vec::new();
    let mut updated_entry_ids = Vec::new();
    let mut deleted_entry_ids = Vec::new();
    let mut reordered_entry_ids = Vec::new();
    let mut copied_entry_ids = Vec::new();

    match operation {
        "create" => {
            let base_title = required_mutation_string(
                mutation,
                "baseTitle",
                "Presentation playlist base title is required.",
            )?;
            let title = unique_collection_title(base_title, &playlists);
            let playlist_id = unique_collection_id("playlist", &title, &playlists);
            let mut playlist = match mutation.get("template") {
                Some(Value::Object(template)) => template.clone(),
                Some(_) => {
                    return Err("Presentation playlist template must be an object.".to_string())
                }
                None => Map::new(),
            };
            playlist.insert("id".to_string(), json!(playlist_id));
            playlist.insert("title".to_string(), json!(title));
            playlist
                .entry("items".to_string())
                .or_insert_with(|| json!([]));
            if let Some(playlist_type) = mutation
                .get("playlistType")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                playlist.insert("type".to_string(), json!(playlist_type));
            }
            let playlist = normalize_playlist_value(&Value::Object(playlist))
                .ok_or_else(|| "Presentation playlist could not be normalized.".to_string())?;
            playlists.push(playlist);
            created_playlist_ids.push(playlist_id);
        }
        "rename" => {
            let playlist_id = required_mutation_string(
                mutation,
                "playlistId",
                "Presentation playlist id is required.",
            )?;
            let title = required_mutation_string(
                mutation,
                "title",
                "Presentation playlist title is required.",
            )?;
            let playlist = playlist_object_mut(&mut playlists, playlist_id)?;
            if playlist.get("title").and_then(Value::as_str) == Some(title) {
                return Ok(unchanged_presentation_playlist_result(workspace));
            }
            playlist.insert("title".to_string(), json!(title));
        }
        "setTheme" => {
            validate_mutation_keys(
                mutation,
                &["type", "playlistIds", "assignment"],
                "presentation playlist theme mutation",
            )?;
            let playlist_ids = dedupe_mutation_strings(required_mutation_string_array(
                mutation,
                "playlistIds",
                "Presentation playlist theme ids are required.",
            )?);
            let known_playlist_ids = playlists
                .iter()
                .filter_map(|playlist| playlist.get("id").and_then(Value::as_str))
                .collect::<HashSet<_>>();
            if let Some(missing) = playlist_ids
                .iter()
                .find(|playlist_id| !known_playlist_ids.contains(playlist_id.as_str()))
            {
                return Err(format!("Presentation playlist '{missing}' was not found."));
            }
            let assignment = presentation_theme_assignment_mutation(document, mutation)?;
            let requested = playlist_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            for playlist in &mut playlists {
                if playlist
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|playlist_id| requested.contains(playlist_id))
                {
                    set_presentation_theme_assignment(playlist, assignment.as_ref())?;
                }
            }
        }
        "reorder" => {
            let playlist_id = required_mutation_string(
                mutation,
                "playlistId",
                "Presentation playlist id is required.",
            )?;
            let target_id = required_mutation_string(
                mutation,
                "targetPlaylistId",
                "Presentation playlist reorder target is required.",
            )?;
            let placement = required_mutation_string(
                mutation,
                "placement",
                "Presentation playlist reorder placement is required.",
            )?;
            if !matches!(placement, "before" | "after" | "after-root" | "inside") {
                return Err("Presentation playlist reorder placement is invalid.".to_string());
            }
            reorder_presentation_playlist(&mut playlists, playlist_id, target_id, placement)?;
        }
        "moveParent" => {
            let playlist_id = required_mutation_string(
                mutation,
                "playlistId",
                "Presentation playlist id is required.",
            )?;
            let parent_id = mutation
                .get("parentId")
                .and_then(Value::as_str)
                .ok_or_else(|| "Presentation playlist parent id is required.".to_string())?;
            if !playlist_exists(&playlists, playlist_id) {
                return Err("Presentation playlist was not found.".to_string());
            }
            if playlist_id == parent_id
                || playlist_is_descendant(&playlists, parent_id, playlist_id)
            {
                return Ok(unchanged_presentation_playlist_result(workspace));
            }
            playlist_object_mut(&mut playlists, playlist_id)?
                .insert("parentId".to_string(), json!(parent_id));
        }
        "setCollapsed" => {
            let playlist_id = required_mutation_string(
                mutation,
                "playlistId",
                "Presentation playlist id is required.",
            )?;
            let collapsed = mutation
                .get("collapsed")
                .and_then(Value::as_bool)
                .ok_or_else(|| "Presentation playlist collapsed state is required.".to_string())?;
            let playlist = playlist_object_mut(&mut playlists, playlist_id)?;
            if playlist.get("collapsed").and_then(Value::as_bool) == Some(collapsed) {
                return Ok(unchanged_presentation_playlist_result(workspace));
            }
            playlist.insert("collapsed".to_string(), json!(collapsed));
        }
        "duplicate" => {
            let requested_ids = required_mutation_string_array(
                mutation,
                "playlistIds",
                "Presentation playlist duplicate ids are required.",
            )?;
            let fallback_title = required_mutation_string(
                mutation,
                "fallbackTitle",
                "Presentation playlist fallback title is required.",
            )?;
            let requested = requested_ids.iter().cloned().collect::<HashSet<_>>();
            let sources = playlists
                .iter()
                .filter(|playlist| {
                    playlist
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| requested.contains(id))
                })
                .cloned()
                .collect::<Vec<_>>();
            let mut copies_by_source = HashMap::new();
            for source in sources {
                let source_id = source
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Presentation playlist id is required.".to_string())?
                    .to_string();
                let source_title = source
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|title| !title.is_empty())
                    .unwrap_or(fallback_title);
                let copy_title = unique_copy_title(source_title, &playlists);
                let copy_id = unique_collection_id("playlist", &copy_title, &playlists);
                let mut copy = source
                    .as_object()
                    .cloned()
                    .ok_or_else(|| "Presentation playlist must be an object.".to_string())?;
                copy.insert("id".to_string(), json!(copy_id));
                copy.insert("title".to_string(), json!(copy_title));
                let copy = normalize_playlist_value(&Value::Object(copy))
                    .ok_or_else(|| "Presentation playlist could not be normalized.".to_string())?;
                let source_index = playlists
                    .iter()
                    .position(|playlist| {
                        playlist.get("id").and_then(Value::as_str) == Some(source_id.as_str())
                    })
                    .ok_or_else(|| "Presentation playlist was not found.".to_string())?;
                playlists.insert(source_index + 1, copy);
                copies_by_source.insert(source_id, copy_id);
            }
            created_playlist_ids = requested_ids
                .iter()
                .filter_map(|id| copies_by_source.get(id).cloned())
                .collect();
        }
        "delete" => {
            let requested_ids = required_mutation_string_array(
                mutation,
                "playlistIds",
                "Presentation playlist delete ids are required.",
            )?;
            let mut deleted = HashSet::new();
            for playlist_id in requested_ids {
                deleted.extend(playlist_descendant_ids(&playlists, &playlist_id));
            }
            deleted_playlist_ids = playlists
                .iter()
                .filter_map(|playlist| playlist.get("id").and_then(Value::as_str))
                .filter(|id| deleted.contains(*id))
                .map(str::to_string)
                .collect();
            playlists.retain(|playlist| {
                playlist
                    .get("id")
                    .and_then(Value::as_str)
                    .is_none_or(|id| !deleted.contains(id))
            });
        }
        "createEntry" => {
            let playlist_id = required_mutation_string(
                mutation,
                "playlistId",
                "Presentation playlist id is required for an entry.",
            )?;
            let entry_type = required_mutation_string(
                mutation,
                "entryType",
                "Presentation playlist entry type is required.",
            )?;
            if !matches!(entry_type, "header" | "placeholder") {
                return Err(
                    "Presentation playlist entry type must be header or placeholder.".to_string(),
                );
            }
            let title = required_mutation_string(
                mutation,
                "title",
                "Presentation playlist entry title is required.",
            )?;
            let playlist = entry_playlist_object_mut(&mut playlists, playlist_id)?;
            let mut entries = playlist_entries(playlist);
            let title = unique_playlist_entry_title(title, &entries);
            let entry_id = unique_playlist_entry_id(entry_type, &title, &entries);
            let mut entry = Map::new();
            entry.insert("id".to_string(), json!(&entry_id));
            entry.insert("type".to_string(), json!(entry_type));
            entry.insert("title".to_string(), json!(&title));
            if entry_type == "header" {
                let color = match mutation.get("color") {
                    None | Some(Value::Null) => "#64748b".to_string(),
                    Some(Value::String(color)) if !color.trim().is_empty() => color.clone(),
                    Some(_) => {
                        return Err(
                            "Presentation playlist header color must be a non-empty string."
                                .to_string(),
                        )
                    }
                };
                entry.insert("color".to_string(), json!(color));
            }
            entries.push(Value::Object(entry));
            set_playlist_entries(playlist, entries)?;
            created_entry_ids.push(entry_id);
        }
        "renameEntry" => {
            let playlist_id = required_mutation_string(
                mutation,
                "playlistId",
                "Presentation playlist id is required for an entry.",
            )?;
            let entry_id = required_mutation_string(
                mutation,
                "entryId",
                "Presentation playlist entry id is required.",
            )?;
            let title = required_mutation_string(
                mutation,
                "title",
                "Presentation playlist entry title is required.",
            )?;
            let entry_index = optional_mutation_index(
                mutation,
                "entryIndex",
                "Presentation playlist entry index must be a non-negative integer.",
            )?;
            let expected_title = mutation
                .get("expectedTitle")
                .map(|value| {
                    value.as_str().ok_or_else(|| {
                        "Presentation playlist expected entry title must be a string.".to_string()
                    })
                })
                .transpose()?;
            let playlist = entry_playlist_object_mut(&mut playlists, playlist_id)?;
            let mut entries = playlist_entries(playlist);
            let entry = if let Some(index) = entry_index {
                let entry = entries.get_mut(index).ok_or_else(|| {
                    "Presentation playlist entry index is out of range.".to_string()
                })?;
                if entry.get("id").and_then(Value::as_str) != Some(entry_id) {
                    return Err("Presentation playlist entry anchor no longer matches.".to_string());
                }
                entry
            } else {
                entries
                    .iter_mut()
                    .find(|entry| entry.get("id").and_then(Value::as_str) == Some(entry_id))
                    .ok_or_else(|| "Presentation playlist entry was not found.".to_string())?
            };
            let entry = entry
                .as_object_mut()
                .ok_or_else(|| "Presentation playlist entry must be an object.".to_string())?;
            if expected_title.is_some_and(|expected| {
                entry.get("title").and_then(Value::as_str).unwrap_or("") != expected
            }) {
                return Err("Presentation playlist entry title no longer matches.".to_string());
            }
            if entry.get("title").and_then(Value::as_str) == Some(title) {
                return Ok(unchanged_presentation_playlist_result(workspace));
            }
            entry.insert("title".to_string(), json!(title));
            set_playlist_entries(playlist, entries)?;
            renamed_entry_ids.push(entry_id.to_string());
        }
        "setHeader" | "setHeaderColor" => {
            let color_only = operation == "setHeaderColor";
            if mutation.keys().any(|key| {
                ![
                    "type",
                    "playlistId",
                    "entryId",
                    "title",
                    "color",
                    "timerId",
                    "timerCommand",
                    "timerConfiguration",
                ]
                .contains(&key.as_str())
            }) {
                return Err("Unsupported playlist header mutation field.".into());
            }
            let playlist_id =
                required_mutation_string(mutation, "playlistId", "Playlist id is required.")?;
            let entry_id =
                required_mutation_string(mutation, "entryId", "Playlist entry id is required.")?;
            let color = mutation["color"]
                .as_str()
                .ok_or("Header color must be text.")?;
            if color != "transparent"
                && !(matches!(color.len(), 7 | 9)
                    && color.starts_with('#')
                    && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit()))
            {
                return Err("Header color must be a hex color.".into());
            }
            let playlist = entry_playlist_object_mut(&mut playlists, playlist_id)?;
            let mut entries = playlist_entries(playlist);
            let entry = entries
                .iter_mut()
                .find(|entry| entry["id"] == entry_id && entry["type"] == "header")
                .and_then(Value::as_object_mut)
                .ok_or("Playlist header was not found.")?;
            entry.insert("color".into(), json!(color));
            if !color_only {
                let title =
                    required_mutation_string(mutation, "title", "Header title is required.")?;
                entry.insert("title".into(), json!(title));
                let timer_id = mutation["timerId"]
                    .as_str()
                    .ok_or("Header timer id must be text.")?;
                let command = mutation["timerCommand"]
                    .as_str()
                    .filter(|command| matches!(*command, "start" | "stop" | "reset"))
                    .ok_or("Unsupported header timer command.")?;
                let configuration = mutation.get("timerConfiguration");
                if let Some(configuration) = configuration.filter(|value| !value.is_null()) {
                    if timer_id.is_empty() {
                        return Err("Header timer configuration requires a linked timer.".into());
                    }
                    validate_header_timer_configuration(configuration)?;
                }
                let mut actions = entry
                    .get("actions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let timer = actions.iter().position(|action| action["type"] == "timer");
                if timer_id.is_empty() {
                    if let Some(index) = timer {
                        actions.remove(index);
                    }
                } else {
                    let index = timer.unwrap_or_else(|| {
                        actions
                            .push(json!({"id": uuid::Uuid::new_v4().to_string(), "type":"timer"}));
                        actions.len() - 1
                    });
                    let action = &mut actions[index];
                    action["targetId"] = json!(timer_id);
                    action["value"] = json!(command);
                    if let Some(configuration) = configuration {
                        if configuration.is_null() {
                            if let Some(details) = action["details"].as_object_mut() {
                                details.remove("configuration");
                            }
                        } else {
                            action["details"]["configuration"] = configuration.clone();
                        }
                    }
                }
                if actions.is_empty() {
                    entry.remove("actions");
                } else {
                    entry.insert("actions".into(), json!(actions));
                }
            } else if ["title", "timerId", "timerCommand", "timerConfiguration"]
                .iter()
                .any(|key| mutation.contains_key(*key))
            {
                return Err("A header color change only accepts the color.".into());
            }
            set_playlist_entries(playlist, entries)?;
            updated_entry_ids.push(entry_id.to_string());
        }
        "setEntriesHidden" => {
            if mutation
                .keys()
                .any(|key| !["type", "playlistId", "entryIds", "hidden"].contains(&key.as_str()))
            {
                return Err("Unsupported playlist visibility mutation field.".into());
            }
            let playlist_id =
                required_mutation_string(mutation, "playlistId", "Playlist id is required.")?;
            let entry_ids = dedupe_mutation_strings(required_mutation_string_array(
                mutation,
                "entryIds",
                "Playlist entry ids are required.",
            )?);
            let hidden = mutation
                .get("hidden")
                .and_then(Value::as_bool)
                .ok_or("Playlist entry visibility must be a boolean.")?;
            let playlist = entry_playlist_object_mut(&mut playlists, playlist_id)?;
            let mut entries = playlist_entries(playlist);
            for entry_id in &entry_ids {
                let entry = entries
                    .iter_mut()
                    .find(|entry| entry["id"] == *entry_id)
                    .and_then(Value::as_object_mut)
                    .ok_or("Playlist entry was not found.")?;
                if hidden {
                    entry.insert("hidden".into(), json!(true));
                } else {
                    entry.remove("hidden");
                }
            }
            set_playlist_entries(playlist, entries)?;
            updated_entry_ids.extend(entry_ids);
        }
        "setEntryArrangement" => {
            let playlist_id = required_mutation_string(
                mutation,
                "playlistId",
                "Presentation playlist id is required for an entry.",
            )?;
            let entry_id = required_mutation_string(
                mutation,
                "entryId",
                "Presentation playlist entry id is required.",
            )?;
            let arrangement_id = mutation
                .get("arrangementId")
                .and_then(Value::as_str)
                .ok_or_else(|| "Presentation playlist arrangement id is required.".to_string())?;
            let entry_index = optional_mutation_index(
                mutation,
                "entryIndex",
                "Presentation playlist entry index must be a non-negative integer.",
            )?;
            let expected_entry = mutation.get("expectedEntry");
            if entry_index.is_some() != expected_entry.is_some() {
                return Err("Presentation playlist arrangement anchor is incomplete.".to_string());
            }
            if expected_entry.is_some_and(|entry| !entry.is_object()) {
                return Err("Presentation playlist expected entry must be an object.".to_string());
            }
            let playlist = entry_playlist_object_mut(&mut playlists, playlist_id)?;
            let mut entries = playlist_entries(playlist);
            let entry = if let Some(index) = entry_index {
                let entry = entries.get_mut(index).ok_or_else(|| {
                    "Presentation playlist entry index is out of range.".to_string()
                })?;
                if entry.get("id").and_then(Value::as_str) != Some(entry_id)
                    || Some(&*entry) != expected_entry
                {
                    return Err("Presentation playlist entry anchor no longer matches.".to_string());
                }
                entry
            } else {
                entries
                    .iter_mut()
                    .find(|entry| entry.get("id").and_then(Value::as_str) == Some(entry_id))
                    .ok_or_else(|| "Presentation playlist entry was not found.".to_string())?
            };
            if entry.get("type").and_then(Value::as_str) != Some("presentation") {
                return Err(
                    "Only presentation playlist entries can select an arrangement.".to_string(),
                );
            }
            let entry = entry
                .as_object_mut()
                .ok_or_else(|| "Presentation playlist entry must be an object.".to_string())?;
            let current = entry
                .get("arrangementId")
                .and_then(Value::as_str)
                .unwrap_or("");
            if current == arrangement_id {
                return Ok(unchanged_presentation_playlist_result(workspace));
            }
            if arrangement_id.is_empty() {
                entry.remove("arrangementId");
            } else {
                entry.insert("arrangementId".to_string(), json!(arrangement_id));
            }
            set_playlist_entries(playlist, entries)?;
            updated_entry_ids.push(entry_id.to_string());
        }
        "setPlaceholderLink" => {
            let playlist_id = required_mutation_string(
                mutation,
                "playlistId",
                "Presentation playlist id is required for an entry.",
            )?;
            let entry_id = required_mutation_string(
                mutation,
                "entryId",
                "Presentation playlist entry id is required.",
            )?;
            let linked_item_id = mutation
                .get("linkedItemId")
                .and_then(Value::as_str)
                .ok_or_else(|| "Presentation playlist linked item id is required.".to_string())?;
            let entry_index = optional_mutation_index(
                mutation,
                "entryIndex",
                "Presentation playlist entry index must be a non-negative integer.",
            )?;
            let expected_entry = mutation.get("expectedEntry");
            if entry_index.is_some() != expected_entry.is_some() {
                return Err("Presentation playlist placeholder anchor is incomplete.".to_string());
            }
            if expected_entry.is_some_and(|entry| !entry.is_object()) {
                return Err("Presentation playlist expected entry must be an object.".to_string());
            }
            let playlist = entry_playlist_object_mut(&mut playlists, playlist_id)?;
            let mut entries = playlist_entries(playlist);
            let entry = if let Some(index) = entry_index {
                let entry = entries.get_mut(index).ok_or_else(|| {
                    "Presentation playlist entry index is out of range.".to_string()
                })?;
                if entry.get("id").and_then(Value::as_str) != Some(entry_id)
                    || Some(&*entry) != expected_entry
                {
                    return Err("Presentation playlist entry anchor no longer matches.".to_string());
                }
                entry
            } else {
                entries
                    .iter_mut()
                    .find(|entry| entry.get("id").and_then(Value::as_str) == Some(entry_id))
                    .ok_or_else(|| "Presentation playlist entry was not found.".to_string())?
            };
            if entry.get("type").and_then(Value::as_str) != Some("placeholder") {
                return Err(
                    "Only placeholder playlist entries can link a presentation.".to_string()
                );
            }
            let entry = entry
                .as_object_mut()
                .ok_or_else(|| "Presentation playlist entry must be an object.".to_string())?;
            let current = entry
                .get("linkedItemId")
                .and_then(Value::as_str)
                .unwrap_or("");
            if current == linked_item_id {
                return Ok(unchanged_presentation_playlist_result(workspace));
            }
            if linked_item_id.is_empty() {
                entry.remove("linkedItemId");
            } else {
                entry.insert("linkedItemId".to_string(), json!(linked_item_id));
            }
            set_playlist_entries(playlist, entries)?;
            updated_entry_ids.push(entry_id.to_string());
        }
        "addEntry" => {
            let playlist_id = required_mutation_string(
                mutation,
                "playlistId",
                "Presentation playlist id is required for an entry.",
            )?;
            let item_ids = required_mutation_string_array(
                mutation,
                "itemIds",
                "Presentation playlist item ids are required.",
            )?;
            let skip_existing = mutation
                .get("skipExisting")
                .and_then(Value::as_bool)
                .ok_or_else(|| "Presentation playlist add skipExisting is required.".to_string())?;
            let library_items = value_array(document, "library")?;
            let library_item_ids = library_items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
                .collect::<HashSet<_>>();
            if let Some(missing) = item_ids
                .iter()
                .find(|item_id| !library_item_ids.contains(item_id.as_str()))
            {
                return Err(format!(
                    "Presentation library item '{missing}' was not found."
                ));
            }
            let playlist = entry_playlist_object_mut(&mut playlists, playlist_id)?;
            let mut entries = playlist_entries(playlist);
            let existing_item_ids = entries
                .iter()
                .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("presentation"))
                .filter_map(|entry| {
                    entry
                        .get("itemId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<HashSet<_>>();
            for item_id in item_ids {
                if skip_existing && existing_item_ids.contains(&item_id) {
                    continue;
                }
                let entry_id = unique_presentation_entry_id(item_id.as_str(), &entries);
                entries.push(json!({
                    "id": &entry_id,
                    "type": "presentation",
                    "itemId": &item_id,
                }));
                added_entry_ids.push(entry_id);
            }
            set_playlist_entries(playlist, entries)?;
        }
        "deleteEntry" => {
            let playlist_id = required_mutation_string(
                mutation,
                "playlistId",
                "Presentation playlist id is required for an entry.",
            )?;
            let entry_ids = required_mutation_string_array(
                mutation,
                "entryIds",
                "Presentation playlist entry ids are required.",
            )?;
            let entry_anchors = mutation
                .get("entryAnchors")
                .map(|value| {
                    value.as_array().ok_or_else(|| {
                        "Presentation playlist entry anchors must be an array.".to_string()
                    })
                })
                .transpose()?;
            let playlist = entry_playlist_object_mut(&mut playlists, playlist_id)?;
            let entries = playlist_entries(playlist);
            if let Some(anchors) = entry_anchors {
                if anchors.len() != entry_ids.len() {
                    return Err(
                        "Presentation playlist entry anchors must match the requested entries."
                            .to_string(),
                    );
                }
                let mut anchored_indexes = HashSet::new();
                for (position, anchor) in anchors.iter().enumerate() {
                    let anchor = anchor.as_object().ok_or_else(|| {
                        "Presentation playlist entry anchor must be an object.".to_string()
                    })?;
                    let entry_index = optional_mutation_index(
                        anchor,
                        "entryIndex",
                        "Presentation playlist entry anchor index must be a non-negative integer.",
                    )?
                    .ok_or_else(|| {
                        "Presentation playlist entry anchor index must be a non-negative integer."
                            .to_string()
                    })?;
                    let entry_id = required_mutation_string(
                        anchor,
                        "entryId",
                        "Presentation playlist entry anchor id is required.",
                    )?;
                    let expected_entry = anchor
                        .get("expectedEntry")
                        .filter(|value| value.is_object())
                        .ok_or_else(|| {
                            "Presentation playlist expected entry must be an object.".to_string()
                        })?;
                    if entry_ids.get(position).map(String::as_str) != Some(entry_id) {
                        return Err(
                            "Presentation playlist entry anchor order no longer matches."
                                .to_string(),
                        );
                    }
                    if !anchored_indexes.insert(entry_index) {
                        return Err(
                            "Presentation playlist entry anchors must be unique.".to_string()
                        );
                    }
                    let current_entry = entries.get(entry_index).ok_or_else(|| {
                        "Presentation playlist entry anchor index is out of range.".to_string()
                    })?;
                    if current_entry.get("id").and_then(Value::as_str) != Some(entry_id)
                        || current_entry != expected_entry
                    {
                        return Err(
                            "Presentation playlist entry anchor no longer matches.".to_string()
                        );
                    }
                }
                let mut retained = Vec::with_capacity(entries.len() - anchored_indexes.len());
                for (index, entry) in entries.into_iter().enumerate() {
                    if anchored_indexes.contains(&index) {
                        if let Some(entry_id) = entry.get("id").and_then(Value::as_str) {
                            deleted_entry_ids.push(entry_id.to_string());
                        }
                    } else {
                        retained.push(entry);
                    }
                }
                set_playlist_entries(playlist, retained)?;
            } else {
                let mut requested_counts =
                    entry_ids
                        .into_iter()
                        .fold(HashMap::new(), |mut counts, entry_id| {
                            *counts.entry(entry_id).or_insert(0_usize) += 1;
                            counts
                        });
                let mut retained = Vec::with_capacity(entries.len());
                for entry in entries {
                    let Some(entry_id) = entry.get("id").and_then(Value::as_str) else {
                        retained.push(entry);
                        continue;
                    };
                    if let Some(remaining) = requested_counts.get_mut(entry_id) {
                        if *remaining > 0 {
                            *remaining -= 1;
                            deleted_entry_ids.push(entry_id.to_string());
                            continue;
                        }
                    }
                    retained.push(entry);
                }
                if let Some((missing, _)) = requested_counts
                    .iter()
                    .find(|(_, remaining)| **remaining > 0)
                {
                    return Err(format!(
                        "Presentation playlist entry '{missing}' was not found."
                    ));
                }
                set_playlist_entries(playlist, retained)?;
            }
        }
        "reorderEntries" => {
            let playlist_id = required_mutation_string(
                mutation,
                "playlistId",
                "Presentation playlist id is required for an entry.",
            )?;
            let entry_ids = required_mutation_string_array(
                mutation,
                "entryIds",
                "Presentation playlist entry ids are required for reorder.",
            )?;
            let target_entry_id = required_mutation_string(
                mutation,
                "targetEntryId",
                "Presentation playlist reorder target is required.",
            )?;
            let placement = required_mutation_string(
                mutation,
                "placement",
                "Presentation playlist reorder placement is required.",
            )?;
            if !matches!(placement, "before" | "after") {
                return Err(
                    "Presentation playlist reorder placement must be before or after.".to_string(),
                );
            }
            let playlist = entry_playlist_object_mut(&mut playlists, playlist_id)?;
            let mut entries = playlist_entries(playlist);
            let entry_anchors = mutation.get("entryAnchors");
            let target_entry_index = optional_mutation_index(
                mutation,
                "targetEntryIndex",
                "Presentation playlist target entry index must be a non-negative integer.",
            )?;
            let target_expected_entry = mutation.get("targetExpectedEntry");
            if target_entry_index.is_some() != target_expected_entry.is_some() {
                return Err(
                    "Presentation playlist reorder target anchor is incomplete.".to_string()
                );
            }
            if target_expected_entry.is_some_and(|entry| !entry.is_object()) {
                return Err(
                    "Presentation playlist expected target entry must be an object.".to_string(),
                );
            }
            reordered_entry_ids = if let Some(anchors) = entry_anchors {
                let anchors = anchors.as_array().ok_or_else(|| {
                    "Presentation playlist reorder entry anchors must be an array.".to_string()
                })?;
                let target_entry_index = target_entry_index.ok_or_else(|| {
                    "Presentation playlist reorder target anchor is incomplete.".to_string()
                })?;
                let target_expected_entry = target_expected_entry.ok_or_else(|| {
                    "Presentation playlist reorder target anchor is incomplete.".to_string()
                })?;
                reorder_playlist_entries_by_anchors(
                    &mut entries,
                    &entry_ids,
                    anchors,
                    target_entry_id,
                    target_entry_index,
                    target_expected_entry,
                    placement,
                )?
            } else {
                if target_entry_index.is_some() || target_expected_entry.is_some() {
                    return Err(
                        "Presentation playlist reorder target anchor is incomplete.".to_string()
                    );
                }
                reorder_playlist_entries_by_id(
                    &mut entries,
                    &entry_ids,
                    target_entry_id,
                    placement,
                )?
            };
            if !reordered_entry_ids.is_empty() {
                set_playlist_entries(playlist, entries)?;
            }
        }
        "copyEntry" => {
            let target_playlist_id = required_mutation_string(
                mutation,
                "targetPlaylistId",
                "Presentation playlist target id is required for an entry copy.",
            )?;
            let target_index_in_workspace = playlist_index(&playlists, target_playlist_id)?;
            if source_playlist_is_folder(&playlists[target_index_in_workspace]) {
                return Err("Presentation playlist folders cannot contain entries.".to_string());
            }
            let source_entry_index = optional_mutation_index(
                mutation,
                "sourceEntryIndex",
                "Presentation playlist copy source entry index must be a non-negative integer.",
            )?;
            let expected_source_entry = mutation.get("expectedSourceEntry");
            if source_entry_index.is_some() != expected_source_entry.is_some() {
                return Err("Presentation playlist copy source anchor is incomplete.".to_string());
            }
            if expected_source_entry.is_some_and(|entry| !entry.is_object()) {
                return Err(
                    "Presentation playlist expected source entry must be an object.".to_string(),
                );
            }
            let target_entry_index = optional_mutation_index(
                mutation,
                "targetEntryIndex",
                "Presentation playlist copy target entry index must be a non-negative integer.",
            )?;
            let expected_target_entry = mutation.get("expectedTargetEntry");
            if target_entry_index.is_some() != expected_target_entry.is_some() {
                return Err("Presentation playlist copy target anchor is incomplete.".to_string());
            }
            if expected_target_entry.is_some_and(|entry| !entry.is_object()) {
                return Err(
                    "Presentation playlist expected target entry must be an object.".to_string(),
                );
            }
            let target_entry_id = match mutation.get("targetEntryId") {
                None => None,
                Some(value) => Some(
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            "Presentation playlist copy target entry id must be non-empty."
                                .to_string()
                        })?,
                ),
            };
            if target_entry_index.is_some() && target_entry_id.is_none() {
                return Err(
                    "Presentation playlist copy target anchor requires a target entry id."
                        .to_string(),
                );
            }
            let (source_index, source_entry_index, source_entry) = if let Some(snapshot) =
                mutation.get("entry")
            {
                if mutation.get("sourcePlaylistId").is_some()
                    || mutation.get("entryId").is_some()
                    || source_entry_index.is_some()
                    || expected_source_entry.is_some()
                {
                    return Err(
                        "Presentation playlist copy must use either a source entry or a snapshot."
                            .to_string(),
                    );
                }
                let source_entry = normalize_playlist_entry_value(snapshot)
                    .ok_or_else(|| "Presentation playlist copy entry is invalid.".to_string())?;
                (None, None, source_entry)
            } else {
                let source_playlist_id = required_mutation_string(
                    mutation,
                    "sourcePlaylistId",
                    "Presentation playlist source id is required for an entry copy.",
                )?;
                let entry_id = required_mutation_string(
                    mutation,
                    "entryId",
                    "Presentation playlist entry id is required for an entry copy.",
                )?;
                let source_index = playlist_index(&playlists, source_playlist_id)?;
                if source_playlist_is_folder(&playlists[source_index]) {
                    return Err("Presentation playlist folders cannot contain entries.".to_string());
                }
                let source_entries = playlist_entries(
                    playlists[source_index]
                        .as_object()
                        .ok_or_else(|| "Presentation playlist must be an object.".to_string())?,
                );
                let source_entry_index = if let Some(index) = source_entry_index {
                    let expected_entry = expected_source_entry.ok_or_else(|| {
                        "Presentation playlist copy source anchor is incomplete.".to_string()
                    })?;
                    let current_entry = source_entries.get(index).ok_or_else(|| {
                        "Presentation playlist copy source entry anchor index is out of range."
                            .to_string()
                    })?;
                    if current_entry.get("id").and_then(Value::as_str) != Some(entry_id)
                        || current_entry != expected_entry
                    {
                        return Err(
                            "Presentation playlist copy source entry anchor no longer matches."
                                .to_string(),
                        );
                    }
                    index
                } else {
                    source_entries
                        .iter()
                        .position(|entry| entry.get("id").and_then(Value::as_str) == Some(entry_id))
                        .ok_or_else(|| "Presentation playlist entry was not found.".to_string())?
                };
                (
                    Some(source_index),
                    Some(source_entry_index),
                    source_entries[source_entry_index].clone(),
                )
            };
            let mut target_entries = playlist_entries(
                playlists[target_index_in_workspace]
                    .as_object()
                    .ok_or_else(|| "Presentation playlist must be an object.".to_string())?,
            );
            if target_entry_id.is_some() && mutation.get("placement").is_none() {
                return Err(
                    "Presentation playlist copy placement is required with a target entry."
                        .to_string(),
                );
            }
            let title = source_entry
                .get("title")
                .and_then(Value::as_str)
                .filter(|title| !title.trim().is_empty())
                .unwrap_or(
                    if source_entry.get("type").and_then(Value::as_str) == Some("header") {
                        "Header"
                    } else {
                        "Placeholder"
                    },
                );
            let title = unique_playlist_entry_title(title, &target_entries);
            let copied_entry_id = unique_playlist_entry_id(
                source_entry
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("presentation"),
                &title,
                &target_entries,
            );
            let mut copied_entry = source_entry
                .as_object()
                .cloned()
                .ok_or_else(|| "Presentation playlist entry must be an object.".to_string())?;
            copied_entry.insert("id".to_string(), json!(&copied_entry_id));
            copied_entry.insert("title".to_string(), json!(&title));
            let insertion_index = if let Some(target_entry_id) = target_entry_id {
                let placement = mutation
                    .get("placement")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        "Presentation playlist copy placement is required with a target entry."
                            .to_string()
                    })?;
                if !matches!(placement, "before" | "after") {
                    return Err(
                        "Presentation playlist copy placement must be before or after.".to_string(),
                    );
                }
                let target_entry_index = if let Some(index) = target_entry_index {
                    let expected_entry = expected_target_entry.ok_or_else(|| {
                        "Presentation playlist copy target anchor is incomplete.".to_string()
                    })?;
                    let current_entry = target_entries.get(index).ok_or_else(|| {
                        "Presentation playlist copy target entry anchor index is out of range."
                            .to_string()
                    })?;
                    if current_entry.get("id").and_then(Value::as_str) != Some(target_entry_id)
                        || current_entry != expected_entry
                    {
                        return Err(
                            "Presentation playlist copy target entry anchor no longer matches."
                                .to_string(),
                        );
                    }
                    index
                } else {
                    target_entries
                        .iter()
                        .position(|entry| {
                            entry.get("id").and_then(Value::as_str) == Some(target_entry_id)
                        })
                        .ok_or_else(|| {
                            "Presentation playlist copy target entry was not found.".to_string()
                        })?
                };
                if placement == "after" {
                    target_entry_index.saturating_add(1)
                } else {
                    target_entry_index
                }
            } else if mutation.get("placement").is_some() {
                return Err(
                    "Presentation playlist copy placement requires a target entry.".to_string(),
                );
            } else if source_index == Some(target_index_in_workspace) {
                source_entry_index
                    .map(|index| index.saturating_add(1))
                    .unwrap_or(target_entries.len())
            } else {
                target_entries.len()
            }
            .min(target_entries.len());
            target_entries.insert(insertion_index, Value::Object(copied_entry));
            set_playlist_entries(
                playlists[target_index_in_workspace]
                    .as_object_mut()
                    .ok_or_else(|| "Presentation playlist must be an object.".to_string())?,
                target_entries,
            )?;
            copied_entry_ids.push(copied_entry_id);
        }
        _ => return Err("Unsupported presentation playlist mutation type.".to_string()),
    }

    if playlists == original {
        return Ok(unchanged_presentation_playlist_result(workspace));
    }
    let workspace = save_workspace(path, &json!({ "playlist": playlists }))?;
    Ok(PresentationPlaylistMutationResult {
        workspace,
        changed: true,
        created_playlist_ids,
        deleted_playlist_ids,
        created_entry_ids,
        added_entry_ids,
        renamed_entry_ids,
        updated_entry_ids,
        deleted_entry_ids,
        reordered_entry_ids,
        copied_entry_ids,
    })
}

fn unchanged_presentation_playlist_result(workspace: Value) -> PresentationPlaylistMutationResult {
    PresentationPlaylistMutationResult {
        workspace,
        changed: false,
        created_playlist_ids: Vec::new(),
        deleted_playlist_ids: Vec::new(),
        created_entry_ids: Vec::new(),
        added_entry_ids: Vec::new(),
        renamed_entry_ids: Vec::new(),
        updated_entry_ids: Vec::new(),
        deleted_entry_ids: Vec::new(),
        reordered_entry_ids: Vec::new(),
        copied_entry_ids: Vec::new(),
    }
}

fn entry_playlist_object_mut<'a>(
    playlists: &'a mut [Value],
    playlist_id: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    let playlist = playlist_object_mut(playlists, playlist_id)?;
    if playlist.get("type").and_then(Value::as_str) == Some("folder") {
        return Err("Presentation playlist folders cannot contain entries.".to_string());
    }
    Ok(playlist)
}

fn source_playlist_is_folder(playlist: &Value) -> bool {
    playlist.get("type").and_then(Value::as_str) == Some("folder")
}

fn playlist_index(playlists: &[Value], playlist_id: &str) -> Result<usize, String> {
    playlists
        .iter()
        .position(|playlist| playlist.get("id").and_then(Value::as_str) == Some(playlist_id))
        .ok_or_else(|| "Presentation playlist was not found.".to_string())
}

fn playlist_entries(playlist: &Map<String, Value>) -> Vec<Value> {
    playlist
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn set_playlist_entries(
    playlist: &mut Map<String, Value>,
    entries: Vec<Value>,
) -> Result<(), String> {
    let entries = entries
        .into_iter()
        .map(|entry| {
            normalize_playlist_entry_value(&entry)
                .ok_or_else(|| "Presentation playlist entry could not be normalized.".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let item_ids = entries
        .iter()
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("presentation"))
        .filter_map(|entry| entry.get("itemId").and_then(Value::as_str))
        .map(|item_id| json!(item_id))
        .collect::<Vec<_>>();
    playlist.insert("items".to_string(), Value::Array(entries));
    playlist.insert("itemIds".to_string(), Value::Array(item_ids));
    Ok(())
}

fn unique_playlist_entry_title(base_title: &str, entries: &[Value]) -> String {
    let titles = entries
        .iter()
        .filter_map(|entry| entry.get("title").and_then(Value::as_str))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .collect::<HashSet<_>>();
    if !titles.contains(base_title) {
        return base_title.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base_title} {index}");
        if !titles.contains(candidate.as_str()) {
            return candidate;
        }
        index += 1;
    }
}

fn unique_playlist_entry_id(entry_type: &str, title: &str, entries: &[Value]) -> String {
    let slug = playlist_slug(title);
    let base = format!(
        "{entry_type}-{}",
        if slug.is_empty() {
            "item"
        } else {
            slug.as_str()
        }
    );
    let ids = entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    let mut candidate = base.clone();
    let mut index = 2;
    while ids.contains(candidate.as_str()) {
        candidate = format!("{base}-{index}");
        index += 1;
    }
    candidate
}

fn unique_presentation_entry_id(item_id: &str, entries: &[Value]) -> String {
    let base = format!("presentation-{item_id}");
    let ids = entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    let mut candidate = base.clone();
    let mut index = 2;
    while ids.contains(candidate.as_str()) {
        candidate = format!("{base}-{index}");
        index += 1;
    }
    candidate
}

fn reorder_playlist_entries_by_anchors(
    entries: &mut Vec<Value>,
    entry_ids: &[String],
    anchors: &[Value],
    target_entry_id: &str,
    target_entry_index: usize,
    target_expected_entry: &Value,
    placement: &str,
) -> Result<Vec<String>, String> {
    if anchors.is_empty() || anchors.len() != entry_ids.len() {
        return Err(
            "Presentation playlist reorder entry anchors must match the requested entries."
                .to_string(),
        );
    }

    let mut source_indexes = Vec::with_capacity(anchors.len());
    let mut anchored_indexes = HashSet::new();
    for (position, anchor) in anchors.iter().enumerate() {
        let anchor = anchor.as_object().ok_or_else(|| {
            "Presentation playlist reorder entry anchor must be an object.".to_string()
        })?;
        let entry_index = optional_mutation_index(
            anchor,
            "entryIndex",
            "Presentation playlist reorder entry anchor index must be a non-negative integer.",
        )?
        .ok_or_else(|| {
            "Presentation playlist reorder entry anchor index must be a non-negative integer."
                .to_string()
        })?;
        let entry_id = required_mutation_string(
            anchor,
            "entryId",
            "Presentation playlist reorder entry anchor id is required.",
        )?;
        let expected_entry = anchor
            .get("expectedEntry")
            .filter(|value| value.is_object())
            .ok_or_else(|| "Presentation playlist expected entry must be an object.".to_string())?;
        if entry_ids.get(position).map(String::as_str) != Some(entry_id) {
            return Err(
                "Presentation playlist reorder entry anchor order no longer matches.".to_string(),
            );
        }
        if !anchored_indexes.insert(entry_index) {
            return Err("Presentation playlist reorder entry anchors must be unique.".to_string());
        }
        let current_entry = entries.get(entry_index).ok_or_else(|| {
            "Presentation playlist reorder entry anchor index is out of range.".to_string()
        })?;
        if current_entry.get("id").and_then(Value::as_str) != Some(entry_id)
            || current_entry != expected_entry
        {
            return Err(
                "Presentation playlist reorder entry anchor no longer matches.".to_string(),
            );
        }
        source_indexes.push(entry_index);
    }

    let target_entry = entries.get(target_entry_index).ok_or_else(|| {
        "Presentation playlist reorder target entry anchor index is out of range.".to_string()
    })?;
    if target_entry.get("id").and_then(Value::as_str) != Some(target_entry_id)
        || target_entry != target_expected_entry
    {
        return Err("Presentation playlist reorder target anchor no longer matches.".to_string());
    }
    if anchored_indexes.contains(&target_entry_index) {
        return Ok(Vec::new());
    }

    source_indexes.sort_unstable();
    let source_index_set = source_indexes.iter().copied().collect::<HashSet<_>>();
    let moved_entries = source_indexes
        .iter()
        .filter_map(|index| entries.get(*index).cloned())
        .collect::<Vec<_>>();
    let mut remaining_entries = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| (!source_index_set.contains(&index)).then_some(entry.clone()))
        .collect::<Vec<_>>();
    let remaining_target_index = target_entry_index.saturating_sub(
        source_indexes
            .iter()
            .filter(|index| **index < target_entry_index)
            .count(),
    );
    if remaining_target_index >= remaining_entries.len() {
        return Err(
            "Presentation playlist reorder target entry anchor could not be resolved.".to_string(),
        );
    }
    let insertion_index = if placement == "after" {
        remaining_target_index.saturating_add(1)
    } else {
        remaining_target_index
    };
    let moved_ids = moved_entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    remaining_entries.splice(insertion_index..insertion_index, moved_entries);
    *entries = remaining_entries;
    Ok(moved_ids)
}

fn reorder_playlist_entries_by_id(
    entries: &mut Vec<Value>,
    entry_ids: &[String],
    target_entry_id: &str,
    placement: &str,
) -> Result<Vec<String>, String> {
    let mut requested_counts = entry_ids
        .iter()
        .fold(HashMap::new(), |mut counts, entry_id| {
            *counts.entry(entry_id.as_str()).or_insert(0_usize) += 1;
            counts
        });
    let mut source_indexes = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let Some(entry_id) = entry.get("id").and_then(Value::as_str) else {
            continue;
        };
        if let Some(remaining) = requested_counts.get_mut(entry_id) {
            if *remaining > 0 {
                *remaining -= 1;
                source_indexes.push(index);
            }
        }
    }
    if let Some((missing, _)) = requested_counts
        .iter()
        .find(|(_, remaining)| **remaining > 0)
    {
        return Err(format!(
            "Presentation playlist entry '{missing}' was not found."
        ));
    }
    if source_indexes.is_empty() {
        return Ok(Vec::new());
    }
    let target_index = entries
        .iter()
        .position(|entry| entry.get("id").and_then(Value::as_str) == Some(target_entry_id))
        .ok_or_else(|| "Presentation playlist reorder target was not found.".to_string())?;
    if source_indexes.iter().any(|index| *index == target_index) {
        return Ok(Vec::new());
    }
    let source_index_set = source_indexes.iter().copied().collect::<HashSet<_>>();
    let moved_entries = source_indexes
        .iter()
        .filter_map(|index| entries.get(*index).cloned())
        .collect::<Vec<_>>();
    let mut remaining_entries = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| (!source_index_set.contains(&index)).then_some(entry.clone()))
        .collect::<Vec<_>>();
    let remaining_target_index = remaining_entries
        .iter()
        .position(|entry| entry.get("id").and_then(Value::as_str) == Some(target_entry_id))
        .ok_or_else(|| "Presentation playlist reorder target was not found.".to_string())?;
    let insertion_index = if placement == "after" {
        remaining_target_index.saturating_add(1)
    } else {
        remaining_target_index
    };
    let moved_ids = moved_entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    remaining_entries.splice(insertion_index..insertion_index, moved_entries);
    *entries = remaining_entries;
    Ok(moved_ids)
}

fn required_mutation_string_array(
    mutation: &Map<String, Value>,
    key: &str,
    message: &str,
) -> Result<Vec<String>, String> {
    mutation
        .get(key)
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
        .ok_or_else(|| message.to_string())?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("{message} Values must be non-empty strings."))
        })
        .collect()
}

fn dedupe_mutation_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn playlist_exists(playlists: &[Value], playlist_id: &str) -> bool {
    playlists
        .iter()
        .any(|playlist| playlist.get("id").and_then(Value::as_str) == Some(playlist_id))
}

fn playlist_object_mut<'a>(
    playlists: &'a mut [Value],
    playlist_id: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    playlists
        .iter_mut()
        .find(|playlist| playlist.get("id").and_then(Value::as_str) == Some(playlist_id))
        .ok_or_else(|| "Presentation playlist was not found.".to_string())?
        .as_object_mut()
        .ok_or_else(|| "Presentation playlist must be an object.".to_string())
}

fn unique_collection_title(base_title: &str, values: &[Value]) -> String {
    let titles = values
        .iter()
        .filter_map(|value| value.get("title").and_then(Value::as_str))
        .map(str::trim)
        .collect::<HashSet<_>>();
    if !titles.contains(base_title) {
        return base_title.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base_title} {index}");
        if !titles.contains(candidate.as_str()) {
            return candidate;
        }
        index += 1;
    }
}

fn unique_copy_title(base_title: &str, values: &[Value]) -> String {
    let trimmed = base_title.trim();
    let base_title = if trimmed.is_empty() { "Media" } else { trimmed };
    let titles = values
        .iter()
        .filter_map(|value| value.get("title").and_then(Value::as_str))
        .map(str::trim)
        .collect::<HashSet<_>>();
    let mut index = 1;
    loop {
        let candidate = format!("{base_title} {index}");
        if !titles.contains(candidate.as_str()) {
            return candidate;
        }
        index += 1;
    }
}

fn unique_collection_id(prefix: &str, title: &str, values: &[Value]) -> String {
    let slug = title
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = format!("{prefix}-{}", if slug.is_empty() { "item" } else { &slug });
    let ids = values
        .iter()
        .filter_map(|value| value.get("id").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    let mut candidate = base.clone();
    let mut index = 2;
    while ids.contains(candidate.as_str()) {
        candidate = format!("{base}-{index}");
        index += 1;
    }
    candidate
}

fn playlist_parent_id(playlists: &[Value], playlist: &Value) -> String {
    let parent_id = playlist
        .get("parentId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !parent_id.is_empty() && playlist_exists(playlists, parent_id) {
        parent_id.to_string()
    } else {
        String::new()
    }
}

fn playlist_visible_ids(playlists: &[Value]) -> Vec<String> {
    fn visit(playlists: &[Value], parent_id: &str, path: &HashSet<String>, ids: &mut Vec<String>) {
        for playlist in playlists {
            let Some(playlist_id) = playlist.get("id").and_then(Value::as_str) else {
                continue;
            };
            if playlist_parent_id(playlists, playlist) != parent_id || path.contains(playlist_id) {
                continue;
            }
            ids.push(playlist_id.to_string());
            let mut next_path = path.clone();
            next_path.insert(playlist_id.to_string());
            visit(playlists, playlist_id, &next_path, ids);
        }
    }

    let mut ids = Vec::new();
    visit(playlists, "", &HashSet::new(), &mut ids);
    ids
}

fn playlist_is_descendant(playlists: &[Value], candidate_id: &str, parent_id: &str) -> bool {
    let mut current_id = candidate_id.to_string();
    let mut visited = HashSet::new();
    while visited.insert(current_id.clone()) {
        let Some(current) = playlists.iter().find(|playlist| {
            playlist.get("id").and_then(Value::as_str) == Some(current_id.as_str())
        }) else {
            return false;
        };
        let Some(ancestor_id) = current
            .get("parentId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            return false;
        };
        if ancestor_id == parent_id {
            return true;
        }
        current_id = ancestor_id.to_string();
    }
    false
}

fn playlist_root_ancestor_id(playlists: &[Value], playlist_id: &str) -> String {
    let mut root_id = playlist_id.to_string();
    let mut current_id = playlist_id.to_string();
    let mut visited = HashSet::new();
    while visited.insert(current_id.clone()) {
        let Some(current) = playlists.iter().find(|playlist| {
            playlist.get("id").and_then(Value::as_str) == Some(current_id.as_str())
        }) else {
            break;
        };
        let parent_id = playlist_parent_id(playlists, current);
        if parent_id.is_empty() {
            break;
        }
        root_id.clone_from(&parent_id);
        current_id = parent_id;
    }
    root_id
}

fn last_visible_playlist_descendant_id(playlists: &[Value], playlist_id: &str) -> String {
    let visible_ids = playlist_visible_ids(playlists);
    let Some(start_index) = visible_ids.iter().position(|id| id == playlist_id) else {
        return playlist_id.to_string();
    };
    let mut last_id = playlist_id.to_string();
    for id in visible_ids.iter().skip(start_index + 1) {
        if !playlist_is_descendant(playlists, id, playlist_id) {
            break;
        }
        last_id.clone_from(id);
    }
    last_id
}

fn playlist_descendant_ids(playlists: &[Value], playlist_id: &str) -> HashSet<String> {
    let mut ids = HashSet::from([playlist_id.to_string()]);
    loop {
        let before = ids.len();
        for playlist in playlists {
            let Some(id) = playlist.get("id").and_then(Value::as_str) else {
                continue;
            };
            let Some(parent_id) = playlist.get("parentId").and_then(Value::as_str) else {
                continue;
            };
            if ids.contains(parent_id) {
                ids.insert(id.to_string());
            }
        }
        if ids.len() == before {
            return ids;
        }
    }
}

fn reorder_presentation_playlist(
    playlists: &mut Vec<Value>,
    playlist_id: &str,
    target_id: &str,
    placement: &str,
) -> Result<(), String> {
    let dragged_index = playlists
        .iter()
        .position(|playlist| playlist.get("id").and_then(Value::as_str) == Some(playlist_id))
        .ok_or_else(|| "Presentation playlist was not found.".to_string())?;
    let target = playlists
        .iter()
        .find(|playlist| playlist.get("id").and_then(Value::as_str) == Some(target_id))
        .cloned()
        .ok_or_else(|| "Presentation playlist reorder target was not found.".to_string())?;
    if playlist_id == target_id {
        return Ok(());
    }
    if placement == "inside" {
        if target.get("type").and_then(Value::as_str) != Some("folder")
            || playlist_is_descendant(playlists, target_id, playlist_id)
        {
            return Ok(());
        }
        playlist_object_mut(playlists, playlist_id)?
            .insert("parentId".to_string(), json!(target_id));
        return Ok(());
    }
    if playlist_is_descendant(playlists, target_id, playlist_id) {
        return Ok(());
    }

    let mut dragged = playlists.remove(dragged_index);
    let next_parent_id = if placement == "after-root" {
        String::new()
    } else {
        target
            .get("parentId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let visible_ids = playlist_visible_ids(playlists);
    if !visible_ids.iter().any(|id| id == target_id) {
        playlists.insert(dragged_index.min(playlists.len()), dragged);
        return Ok(());
    }
    let anchor_id = if placement == "after-root" {
        let root_id = playlist_root_ancestor_id(playlists, target_id);
        last_visible_playlist_descendant_id(playlists, &root_id)
    } else if placement == "after" {
        last_visible_playlist_descendant_id(playlists, target_id)
    } else {
        target_id.to_string()
    };
    let Some(anchor_index) = playlists.iter().position(|playlist| {
        playlist.get("id").and_then(Value::as_str) == Some(anchor_id.as_str())
    }) else {
        playlists.insert(dragged_index.min(playlists.len()), dragged);
        return Ok(());
    };
    dragged
        .as_object_mut()
        .ok_or_else(|| "Presentation playlist must be an object.".to_string())?
        .insert("parentId".to_string(), json!(next_parent_id));
    let insert_index = if matches!(placement, "after" | "after-root") {
        anchor_index + 1
    } else {
        anchor_index
    };
    playlists.insert(insert_index.min(playlists.len()), dragged);
    Ok(())
}

fn required_mutation_string<'a>(
    mutation: &'a Map<String, Value>,
    key: &str,
    message: &str,
) -> Result<&'a str, String> {
    mutation
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| message.to_string())
}

pub fn mutate_media_library(
    path: &Path,
    mutation: &Value,
) -> Result<MediaLibraryMutationResult, String> {
    let mutation = mutation
        .as_object()
        .ok_or_else(|| "Media library mutation must be an object.".to_string())?;
    let operation = mutation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Media library mutation type is required.".to_string())?;
    let allowed = match operation {
        "create" => &["type", "baseTitle", "libraryType", "mediaType"][..],
        "duplicate" | "delete" => &["type", "libraryId"][..],
        "rename" => &["type", "libraryId", "title"][..],
        "reorder" => &[
            "type",
            "libraryId",
            "targetLibraryId",
            "placement",
            "mediaType",
        ][..],
        "moveParent" => &["type", "libraryId", "parentId", "mediaType"][..],
        "setCollapsed" => &["type", "libraryId", "collapsed"][..],
        "addAsset" => &["type", "libraryId", "mediaId"][..],
        _ => return Err("Unsupported media library mutation type.".to_string()),
    };
    validate_mutation_keys(mutation, allowed, "media library mutation")?;
    let workspace = read_workspace(path)?;
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut media_libraries = value_array(document, "mediaLibraries")?;
    let mut media = value_array(document, "media")?;
    let original_media_libraries = media_libraries.clone();
    let original_media = media.clone();
    let mut selected_library_id = None;

    match operation {
        "create" => {
            let base_title = required_mutation_string(
                mutation,
                "baseTitle",
                "Media library base title is required.",
            )?;
            let library_type = required_mutation_string(
                mutation,
                "libraryType",
                "Media library type is required.",
            )?;
            if !matches!(library_type, "folder" | "playlist") {
                return Err("Media library type must be folder or playlist.".to_string());
            }
            let media_type = required_mutation_string(
                mutation,
                "mediaType",
                "Media library media type is required.",
            )?;
            if !matches!(media_type, "audio" | "graphics") {
                return Err("Media library media type must be audio or graphics.".to_string());
            }
            let title = unique_media_library_title(base_title, &media_libraries);
            let library_id = unique_media_library_id(&title, &media_libraries);
            media_libraries.push(json!({
                "id": library_id,
                "title": title,
                "type": library_type,
                "mediaType": media_type,
                "parentId": "",
                "collapsed": false,
                "itemFiles": [],
            }));
            selected_library_id = Some(library_id);
        }
        "duplicate" => {
            let library_id = required_mutation_string(
                mutation,
                "libraryId",
                "Media library mutation id is required.",
            )?;
            let (next_libraries, next_media, selected) =
                duplicate_media_library(&workspace, library_id)?;
            media_libraries = next_libraries;
            media = next_media;
            selected_library_id = selected;
        }
        "rename" => {
            let library_id = required_mutation_string(
                mutation,
                "libraryId",
                "Media library mutation id is required.",
            )?;
            let title =
                required_mutation_string(mutation, "title", "Media library title is required.")?;
            let library = media_library_object_mut(&mut media_libraries, library_id)?;
            if library.get("title").and_then(Value::as_str) != Some(title) {
                library.insert("title".to_string(), json!(title));
            }
        }
        "reorder" => {
            let library_id = required_mutation_string(
                mutation,
                "libraryId",
                "Media library mutation id is required.",
            )?;
            let target_id = required_mutation_string(
                mutation,
                "targetLibraryId",
                "Media library reorder target is required.",
            )?;
            let placement = required_mutation_string(
                mutation,
                "placement",
                "Media library reorder placement is required.",
            )?;
            if !matches!(placement, "before" | "after" | "inside") {
                return Err("Media library reorder placement is invalid.".to_string());
            }
            let media_type = required_mutation_string(
                mutation,
                "mediaType",
                "Media library media type is required.",
            )?;
            if !matches!(media_type, "audio" | "graphics") {
                return Err("Media library media type must be audio or graphics.".to_string());
            }
            reorder_media_library(
                &mut media_libraries,
                library_id,
                target_id,
                placement,
                media_type,
            )?;
        }
        "moveParent" => {
            let library_id = required_mutation_string(
                mutation,
                "libraryId",
                "Media library mutation id is required.",
            )?;
            let parent_id = mutation
                .get("parentId")
                .and_then(Value::as_str)
                .ok_or_else(|| "Media library parent id is required.".to_string())?;
            let media_type = required_mutation_string(
                mutation,
                "mediaType",
                "Media library media type is required.",
            )?;
            if !matches!(media_type, "audio" | "graphics") {
                return Err("Media library media type must be audio or graphics.".to_string());
            }
            move_media_library_parent(&mut media_libraries, library_id, parent_id, media_type)?;
        }
        "setCollapsed" => {
            let library_id = required_mutation_string(
                mutation,
                "libraryId",
                "Media library mutation id is required.",
            )?;
            let collapsed = mutation
                .get("collapsed")
                .and_then(Value::as_bool)
                .ok_or_else(|| "Media library collapsed state is required.".to_string())?;
            let library = media_library_object_mut(&mut media_libraries, library_id)?;
            if library.get("collapsed").and_then(Value::as_bool) != Some(collapsed) {
                library.insert("collapsed".to_string(), json!(collapsed));
            }
        }
        "addAsset" => {
            let library_id = required_mutation_string(
                mutation,
                "libraryId",
                "Media library mutation id is required.",
            )?;
            let media_id =
                required_mutation_string(mutation, "mediaId", "Media asset id is required.")?;
            add_media_asset_to_library(&mut media_libraries, &media, library_id, media_id)?;
            selected_library_id = Some(library_id.to_string());
        }
        "delete" => {
            let library_id = required_mutation_string(
                mutation,
                "libraryId",
                "Media library mutation id is required.",
            )?;
            let (next_libraries, next_media, selected) =
                delete_media_library(&workspace, library_id)?;
            media_libraries = next_libraries;
            media = next_media;
            selected_library_id = selected;
        }
        _ => unreachable!("media library mutation operation was validated above"),
    }
    let changed = media_libraries != original_media_libraries || media != original_media;
    let workspace = if changed {
        save_workspace(
            path,
            &json!({ "mediaLibraries": media_libraries, "media": media }),
        )?
    } else {
        workspace
    };
    Ok(MediaLibraryMutationResult {
        workspace,
        changed,
        selected_library_id,
    })
}

fn duplicate_media_library(
    workspace: &Value,
    library_id: &str,
) -> Result<(Vec<Value>, Vec<Value>, Option<String>), String> {
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut libraries = value_array(document, "mediaLibraries")?;
    let mut media = value_array(document, "media")?;
    let source = libraries
        .iter()
        .find(|library| library.get("id").and_then(Value::as_str) == Some(library_id))
        .cloned()
        .ok_or_else(|| "Media library was not found.".to_string())?;
    let source_object = source
        .as_object()
        .ok_or_else(|| "Media library must be an object.".to_string())?;
    let is_smart = source_object.get("type").and_then(Value::as_str) == Some("smart");
    let title = copy_name(
        source_object.get("title").and_then(Value::as_str),
        libraries
            .iter()
            .map(|library| library.get("title").and_then(Value::as_str)),
    );
    let copy_id = unique_media_library_id(&title, &libraries);
    let mut library_copy = source_object.clone();
    library_copy.insert("id".to_string(), json!(copy_id));
    library_copy.insert("title".to_string(), json!(title));
    library_copy.insert(
        "parentId".to_string(),
        source_object
            .get("parentId")
            .cloned()
            .unwrap_or_else(|| json!("")),
    );
    library_copy.insert(
        "itemFiles".to_string(),
        source_object
            .get("itemFiles")
            .cloned()
            .unwrap_or_else(|| json!([])),
    );
    let copied_media = if is_smart {
        // Smart-library rows are a scanner result, not authored workspace data.
        // Keeping them out of this mutation means the persisted media index only
        // contains source-path settings; read_media derives the rows again for
        // both library identities after the metadata copy is committed.
        Vec::new()
    } else {
        let existing_media_ids = media
            .iter()
            .filter_map(|item| item.get("id").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<HashSet<_>>();
        media
            .iter()
            .filter(|item| item.get("libraryId").and_then(Value::as_str) == Some(library_id))
            .map(|item| {
                let mut item = item
                    .as_object()
                    .cloned()
                    .ok_or_else(|| "Media entry must be an object.".to_string())?;
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| "Media entry id is required.".to_string())?;
                item.insert(
                    "id".to_string(),
                    json!(unique_copy_id(id, &existing_media_ids)),
                );
                item.insert("libraryId".to_string(), json!(copy_id));
                Ok(Value::Object(item))
            })
            .collect::<Result<Vec<_>, String>>()?
    };
    libraries.push(Value::Object(library_copy));
    media.extend(copied_media);
    Ok((libraries, media, Some(copy_id)))
}

fn delete_media_library(
    workspace: &Value,
    library_id: &str,
) -> Result<(Vec<Value>, Vec<Value>, Option<String>), String> {
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let libraries = value_array(document, "mediaLibraries")?;
    if !libraries
        .iter()
        .any(|library| library.get("id").and_then(Value::as_str) == Some(library_id))
    {
        return Err("Media library was not found.".to_string());
    }
    let mut deleted_ids = HashSet::from([library_id.to_string()]);
    loop {
        let previous_len = deleted_ids.len();
        for library in &libraries {
            let Some(id) = library.get("id").and_then(Value::as_str) else {
                return Err("Media library id is required.".to_string());
            };
            if library
                .get("parentId")
                .and_then(Value::as_str)
                .is_some_and(|parent_id| deleted_ids.contains(parent_id))
            {
                deleted_ids.insert(id.to_string());
            }
        }
        if deleted_ids.len() == previous_len {
            break;
        }
    }
    let libraries = libraries
        .into_iter()
        .filter(|library| {
            library
                .get("id")
                .and_then(Value::as_str)
                .is_none_or(|id| !deleted_ids.contains(id))
        })
        .collect();
    let media = value_array(document, "media")?
        .into_iter()
        .filter(|item| {
            item.get("libraryId")
                .and_then(Value::as_str)
                .is_none_or(|id| !deleted_ids.contains(id))
        })
        .collect();
    Ok((libraries, media, None))
}

fn media_library_object_mut<'a>(
    libraries: &'a mut [Value],
    library_id: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    libraries
        .iter_mut()
        .find(|library| library.get("id").and_then(Value::as_str) == Some(library_id))
        .ok_or_else(|| "Media library was not found.".to_string())?
        .as_object_mut()
        .ok_or_else(|| "Media library must be an object.".to_string())
}

fn media_library_is_descendant(libraries: &[Value], candidate_id: &str, ancestor_id: &str) -> bool {
    let mut current_id = candidate_id.to_string();
    let mut visited = HashSet::new();
    while visited.insert(current_id.clone()) {
        let Some(current) = libraries
            .iter()
            .find(|library| library.get("id").and_then(Value::as_str) == Some(current_id.as_str()))
        else {
            return false;
        };
        let Some(parent_id) = current
            .get("parentId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            return false;
        };
        if parent_id == ancestor_id {
            return true;
        }
        current_id = parent_id.to_string();
    }
    false
}

pub fn plan_media_library_move(
    libraries: &[Value],
    source: &str,
    target: &str,
    placement: &str,
    media_type: &str,
) -> Result<Vec<Value>, String> {
    let mut libraries = libraries.to_vec();
    if placement == "root" {
        move_media_library_parent(&mut libraries, source, "", media_type)?;
    } else if matches!(placement, "before" | "after" | "inside") {
        reorder_media_library(&mut libraries, source, target, placement, media_type)?;
    } else {
        return Err("Media library reorder placement is invalid.".into());
    }
    Ok(libraries)
}

fn reorder_media_library(
    libraries: &mut Vec<Value>,
    library_id: &str,
    target_id: &str,
    placement: &str,
    fallback_media_type: &str,
) -> Result<(), String> {
    let dragged_index = libraries
        .iter()
        .position(|library| library.get("id").and_then(Value::as_str) == Some(library_id))
        .ok_or_else(|| "Media library was not found.".to_string())?;
    let target = libraries
        .iter()
        .find(|library| library.get("id").and_then(Value::as_str) == Some(target_id))
        .cloned()
        .ok_or_else(|| "Media library reorder target was not found.".to_string())?;
    if library_id == target_id || media_library_is_descendant(libraries, target_id, library_id) {
        return Ok(());
    }
    let dragged_media_type = libraries[dragged_index]
        .get("mediaType")
        .and_then(Value::as_str)
        .unwrap_or(fallback_media_type);
    let target_media_type = target
        .get("mediaType")
        .and_then(Value::as_str)
        .unwrap_or(fallback_media_type);
    if dragged_media_type != target_media_type {
        return Ok(());
    }
    if placement == "inside" {
        if target.get("type").and_then(Value::as_str) != Some("folder")
            || media_library_is_descendant(libraries, target_id, library_id)
        {
            return Ok(());
        }
        let current_parent_id = libraries[dragged_index]
            .get("parentId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if current_parent_id == target_id {
            return Ok(());
        }
        media_library_object_mut(libraries, library_id)?
            .insert("parentId".to_string(), json!(target_id));
        return Ok(());
    }

    let mut dragged = libraries.remove(dragged_index);
    let target_index = libraries
        .iter()
        .position(|library| library.get("id").and_then(Value::as_str) == Some(target_id))
        .ok_or_else(|| "Media library reorder target was not found.".to_string())?;
    let parent_id = target
        .get("parentId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    dragged
        .as_object_mut()
        .ok_or_else(|| "Media library must be an object.".to_string())?
        .insert("parentId".to_string(), json!(parent_id));
    let insert_index = if placement == "after" {
        target_index + 1
    } else {
        target_index
    };
    libraries.insert(insert_index.min(libraries.len()), dragged);
    Ok(())
}

fn move_media_library_parent(
    libraries: &mut [Value],
    library_id: &str,
    parent_id: &str,
    fallback_media_type: &str,
) -> Result<(), String> {
    let library_media_type = libraries
        .iter()
        .find(|library| library.get("id").and_then(Value::as_str) == Some(library_id))
        .ok_or_else(|| "Media library was not found.".to_string())?
        .get("mediaType")
        .and_then(Value::as_str)
        .unwrap_or(fallback_media_type);
    if library_media_type != fallback_media_type {
        return Ok(());
    }
    if parent_id == library_id || media_library_is_descendant(libraries, parent_id, library_id) {
        return Ok(());
    }
    if !parent_id.is_empty() {
        let parent = libraries
            .iter()
            .find(|library| library.get("id").and_then(Value::as_str) == Some(parent_id))
            .ok_or_else(|| "Media library parent was not found.".to_string())?;
        if parent.get("type").and_then(Value::as_str) != Some("folder") {
            return Err("Media library parent must be a folder.".to_string());
        }
    }
    let library = media_library_object_mut(libraries, library_id)?;
    if library
        .get("parentId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        == parent_id
    {
        return Ok(());
    }
    library.insert("parentId".to_string(), json!(parent_id));
    Ok(())
}

fn add_media_asset_to_library(
    libraries: &mut [Value],
    media: &[Value],
    library_id: &str,
    media_id: &str,
) -> Result<(), String> {
    let asset_file = media
        .iter()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(media_id))
        .ok_or_else(|| "Media asset was not found.".to_string())?
        .get("assetFile")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Media asset file is required.".to_string())?;
    let library = media_library_object_mut(libraries, library_id)?;
    if library.get("type").and_then(Value::as_str) != Some("playlist") {
        return Err("Media assets can only be added to media playlists.".to_string());
    }
    let item_files = library
        .entry("itemFiles".to_string())
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| "Media playlist item files must be an array.".to_string())?;
    if !item_files
        .iter()
        .any(|item| item.as_str() == Some(asset_file.as_str()))
    {
        item_files.push(json!(asset_file));
    }
    Ok(())
}

fn unique_media_library_title(base_title: &str, libraries: &[Value]) -> String {
    unique_collection_title(base_title.trim(), libraries)
}

fn copy_name<'a>(
    name: Option<&str>,
    existing_names: impl Iterator<Item = Option<&'a str>>,
) -> String {
    let base = name.unwrap_or("Media").trim();
    let base = if base.is_empty() { "Media" } else { base };
    let names = existing_names
        .flatten()
        .map(str::trim)
        .collect::<HashSet<_>>();
    let mut index = 1;
    loop {
        let candidate = format!("{base} {index}");
        if !names.contains(candidate.as_str()) {
            return candidate;
        }
        index += 1;
    }
}

fn unique_media_library_id(title: &str, libraries: &[Value]) -> String {
    let slug = title
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = format!(
        "media-library-{}",
        if slug.is_empty() { "library" } else { &slug }
    );
    let ids = libraries
        .iter()
        .filter_map(|library| library.get("id").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    let mut candidate = base.clone();
    let mut index = 2;
    while ids.contains(candidate.as_str()) {
        candidate = format!("{base}-{index}");
        index += 1;
    }
    candidate
}

fn unique_copy_id(base_id: &str, ids: &HashSet<String>) -> String {
    let base = format!("{base_id}-copy");
    let mut candidate = base.clone();
    let mut index = 1;
    while ids.contains(&candidate) {
        index += 1;
        candidate = format!("{base}-{index}");
    }
    candidate
}

pub fn save_library_item(path: &Path, item: &Value) -> Result<Value, String> {
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "Library item id is required.".to_string())?;
    let mut workspace = read_workspace(path)?;
    let document = workspace
        .as_object_mut()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let items = document
        .get_mut("library")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "Library items are required.".to_string())?;
    if let Some(existing) = items
        .iter_mut()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
    {
        *existing = item.clone();
    } else {
        items.push(item.clone());
        let libraries = document
            .get_mut("libraries")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "Libraries are required.".to_string())?;
        let target = libraries
            .first_mut()
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "A default library is required.".to_string())?;
        let item_ids = target
            .entry("itemIds")
            .or_insert_with(|| Value::Array(Vec::new()));
        let item_ids = item_ids
            .as_array_mut()
            .ok_or_else(|| "Library item ids must be an array.".to_string())?;
        item_ids.push(json!(id));
    }
    let snapshot = Value::Object(document.clone());
    save_workspace(path, &snapshot)
}

pub fn save_library_items(path: &Path, items: &Value) -> Result<(), String> {
    let items = items
        .as_array()
        .ok_or_else(|| "Library items must be an array.".to_string())?;
    if items.is_empty() {
        return Err("Library items must not be empty.".to_string());
    }

    let mut item_ids = HashSet::with_capacity(items.len());
    for item in items {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| "Library item id is required.".to_string())?;
        if !item_ids.insert(id.to_string()) {
            return Err(format!("Duplicate library item id '{id}'."));
        }
    }

    let mut workspace = read_workspace(path)?;
    let document = workspace
        .as_object_mut()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let mut new_item_ids = Vec::new();
    {
        let library_items = document
            .get_mut("library")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "Library items are required.".to_string())?;
        for item in items {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| "Library item id is required.".to_string())?;
            if let Some(existing) = library_items
                .iter_mut()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
            {
                *existing = item.clone();
            } else {
                library_items.push(item.clone());
                new_item_ids.push(id.to_string());
            }
        }
    }

    if !new_item_ids.is_empty() {
        let libraries = document
            .get_mut("libraries")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "Libraries are required.".to_string())?;
        let target = libraries
            .first_mut()
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "A default library is required.".to_string())?;
        let item_ids = target
            .entry("itemIds")
            .or_insert_with(|| Value::Array(Vec::new()));
        let item_ids = item_ids
            .as_array_mut()
            .ok_or_else(|| "Library item ids must be an array.".to_string())?;
        item_ids.extend(new_item_ids.into_iter().map(Value::String));
    }

    document.insert("workspacePath".to_string(), json!(path));
    document.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
    content_themes::refresh(&read_workspace(path)?, &mut workspace)?;
    write_workspace_files(path, &workspace)
}

pub fn delete_theme(path: &Path, theme_id: &str, slide_id: Option<&str>) -> Result<Value, String> {
    let workspace = read_workspace(path)?;
    let mut themes = value_array(
        workspace
            .as_object()
            .ok_or("Workspace must be an object.")?,
        "themes",
    )?;
    let index = themes
        .iter()
        .position(|theme| theme["id"].as_str() == Some(theme_id))
        .ok_or("Theme was not found.")?;
    if let Some(slide_id) = slide_id {
        let slides = themes[index]
            .get_mut("slides")
            .and_then(Value::as_array_mut)
            .ok_or("Theme layouts must be an array.")?;
        let index = slides
            .iter()
            .position(|slide| slide["id"].as_str() == Some(slide_id))
            .ok_or("Theme layout was not found.")?;
        slides.remove(index);
    } else {
        themes.remove(index);
    }
    save_workspace(path, &json!({ "themes": themes }))
}

pub fn save_named_collection(
    path: &Path,
    key: &str,
    value: &Value,
    id: &str,
) -> Result<Value, String> {
    let mut workspace = read_workspace(path)?;
    let document = workspace
        .as_object_mut()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let entries = document
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("{key} must be an array."))?;
    if let Some(existing) = entries
        .iter_mut()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
    {
        *existing = value.clone();
    } else {
        entries.push(value.clone());
    }
    let snapshot = Value::Object(document.clone());
    save_workspace(path, &snapshot)
}

fn write_workspace_files(path: &Path, workspace: &Value) -> Result<(), String> {
    write_workspace_files_with_assets(path, workspace, &[], None)
}

fn write_workspace_files_with_assets(
    path: &Path,
    workspace: &Value,
    additional_assets: &[SyncWorkspaceAsset],
    previous: Option<&Value>,
) -> Result<(), String> {
    crate::native_compositor::validate_object_links(workspace)?;
    crate::native_compositor::validate_stage_preview_fields(workspace)?;
    crate::macro_collections::validate_orders(workspace)?;
    recover_pending_workspace_write(path)?;
    let folder = path
        .parent()
        .ok_or_else(|| "Workspace path has no parent folder.".to_string())?;
    let transaction = folder.join(WORKSPACE_TRANSACTION_DIRECTORY);
    let staged_folder = transaction.join("staged");
    if transaction.exists() {
        return Err("A workspace write transaction is already pending.".to_string());
    }
    fs::create_dir_all(&staged_folder).map_err(|error| error.to_string())?;
    validate_existing_workspace_path(folder, &transaction)?;
    validate_existing_workspace_path(folder, &staged_folder)?;
    let staged_result = (|| {
        stage_additional_workspace_assets(&staged_folder, additional_assets)?;
        validate_staged_workspace_charts(folder, &staged_folder, workspace, additional_assets)?;
        write_workspace_files_to(path, folder, &staged_folder, workspace, previous)?;
        for asset in additional_assets {
            crate::workspace_sync::verify_asset(
                &staged_folder.join(&asset.relative_path),
                &asset.expected_fingerprint,
            )?;
        }
        Ok::<(), String>(())
    })();
    if let Err(error) = staged_result {
        let _ = fs::remove_dir_all(&transaction);
        return Err(error);
    }
    let journal = prepare_workspace_transaction(folder, &transaction)?;
    let cleanup_is_durable = match commit_workspace_transaction(folder, &transaction, &journal) {
        Ok(cleanup_is_durable) => cleanup_is_durable,
        Err(error) => {
            if transaction_committed_path(&transaction).exists() {
                return Ok(());
            }
            let rollback = rollback_workspace_transaction(folder, &transaction, &journal);
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(format!(
                    "{error} Workspace rollback also failed: {rollback_error}"
                )),
            };
        }
    };
    if cleanup_is_durable {
        let _ = finalize_committed_workspace_transaction(&transaction);
    }
    Ok(())
}

fn stage_additional_workspace_assets(
    staged_folder: &Path,
    additional_assets: &[SyncWorkspaceAsset],
) -> Result<(), String> {
    let mut paths = HashSet::new();
    for asset in additional_assets {
        let relative = &asset.relative_path;
        let source = &asset.source_path;
        let relative_text = relative.to_string_lossy();
        let relative = validated_workspace_relative_path(&relative_text)?;
        let mut components = relative.components();
        if components.next() != Some(Component::Normal("Media".as_ref()))
            || !matches!(components.next(), Some(Component::Normal(name)) if name == "Assets" || name == "Charts")
        {
            return Err(
                "Synchronization assets must be stored under Media/Assets or Media/Charts.".into(),
            );
        }
        if !paths.insert(relative.clone()) {
            return Err("Synchronization asset paths must be unique.".into());
        }
        let metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Additional workspace asset source must be a regular file.".into());
        }

        let destination = workspace_target_path(staged_folder, &relative)?;
        let parent = destination
            .parent()
            .ok_or("Additional workspace asset folder is required.")?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let destination = workspace_target_path(staged_folder, &relative)?;
        fs::copy(source, &destination).map_err(|error| error.to_string())?;
        crate::workspace_sync::verify_asset(&destination, &asset.expected_fingerprint)?;
    }
    Ok(())
}

fn validate_staged_workspace_charts(
    folder: &Path,
    staged: &Path,
    value: &Value,
    assets: &[SyncWorkspaceAsset],
) -> Result<(), String> {
    if assets.is_empty() {
        return validate_workspace_charts(&workspace_path(folder), value);
    }
    match value {
        Value::Object(object) => {
            if let Some(chart) = object.get("chordChart") {
                let mut chart: crate::chord_chart::ChordChart =
                    serde_json::from_value(chart.clone())
                        .map_err(|error| format!("Invalid chord chart: {error}"))?;
                let paths = std::iter::once(&chart.source_path).chain(chart.page_paths.iter());
                let mapped = paths
                    .map(|path| {
                        assets
                            .iter()
                            .find(|asset| folder.join(&asset.relative_path) == Path::new(path))
                    })
                    .collect::<Vec<_>>();
                if mapped.iter().any(Option::is_some) {
                    if mapped.iter().any(Option::is_none) {
                        return Err(
                            "Synchronized charts require all source and page assets.".into()
                        );
                    }
                    let mut mapped = mapped.into_iter().map(|asset| {
                        staged
                            .join(&asset.expect("Validated chart asset").relative_path)
                            .to_string_lossy()
                            .into_owned()
                    });
                    chart.source_path = mapped.next().expect("Chart source path");
                    chart.page_paths = mapped.collect();
                    crate::chord_chart::validate(staged, &chart)?;
                } else {
                    crate::chord_chart::validate(folder, &chart)?;
                }
            }
            for child in object.values() {
                validate_staged_workspace_charts(folder, staged, child, assets)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                validate_staged_workspace_charts(folder, staged, child, assets)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_workspace_charts(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(object) = value.as_object() {
        if let Some(chart) = object.get("chordChart") {
            let chart: crate::chord_chart::ChordChart = serde_json::from_value(chart.clone())
                .map_err(|error| format!("Invalid chord chart: {error}"))?;
            let folder = path
                .parent()
                .ok_or("Workspace path has no parent folder.")?;
            crate::chord_chart::validate(folder, &chart)?;
        }
        for child in object.values() {
            validate_workspace_charts(path, child)?;
        }
    } else if let Some(values) = value.as_array() {
        for child in values {
            validate_workspace_charts(path, child)?;
        }
    }
    Ok(())
}

fn write_workspace_files_to(
    manifest_path: &Path,
    source_folder: &Path,
    output_folder: &Path,
    workspace: &Value,
    previous: Option<&Value>,
) -> Result<(), String> {
    let document = workspace
        .as_object()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    if let Some(collections) = document.get("macroCollections") {
        if !collections.is_array() {
            return Err("macroCollections must be an array.".to_string());
        }
    }
    // Unstaged files are left untouched by the commit, so collections equal to the document just read
    // from disk need not be regenerated; this keeps small edits from rewriting every presentation.
    let unchanged = |keys: &[&str]| {
        previous.is_some_and(|previous| {
            keys.iter()
                .all(|key| previous.get(*key) == document.get(*key))
        })
    };
    if !unchanged(&["library", "libraries"]) {
        write_libraries(output_folder, document)?;
    }
    if !unchanged(&["playlist"]) {
        write_indexed(
            output_folder,
            "Playlists",
            "playlists",
            value_array(document, "playlist")?,
        )?;
    }
    if !unchanged(&["media", "mediaLibraries"]) {
        write_media(source_folder, output_folder, document)?;
    }
    if !unchanged(&["themes"]) {
        write_themes(output_folder, value_array(document, "themes")?)?;
    }
    if !unchanged(&["masks"]) {
        write_masks(output_folder, value_array(document, "masks")?)?;
    }
    if !unchanged(&[
        "props",
        "propFolders",
        "countdownTimers",
        "macros",
        "macroCollections",
        "messages",
        "plans",
        "scheduledEvents",
    ]) {
        write_json(
            &output_folder.join("Configuration/Props/Props"),
            json!({
                "schemaVersion": 1,
                "props": value_array(document, "props")?,
                "propFolders": document.get("propFolders").cloned().unwrap_or_else(|| json!([])),
                "countdownTimers": value_array(document, "countdownTimers")?,
                "macros": value_array(document, "macros")?,
                "macroCollections": value_array(document, "macroCollections")?,
                "messages": value_array(document, "messages")?,
                "plans": value_array(document, "plans")?,
                "scheduledEvents": value_array(document, "scheduledEvents")?
            }),
        )?;
    }
    if !unchanged(&[
        "stageLayouts",
        "stageScreenAssignments",
        "stageMessage",
        "audienceLooks",
        "videoInputs",
        "audioInputs",
        "tickers",
        "recordingPresets",
    ]) {
        write_json(
            &output_folder.join("Configuration/Stage/Stage.json"),
            json!({
                "schemaVersion": 1,
                "stageLayouts": value_array(document, "stageLayouts")?,
                "stageScreenAssignments": value_array(document, "stageScreenAssignments")?,
                "stageMessage": document.get("stageMessage").cloned().unwrap_or_else(|| json!({ "text": "", "state": "cleared" })),
                "audienceLooks": value_array(document, "audienceLooks")?,
                "videoInputs": value_array(document, "videoInputs")?,
                "audioInputs": value_array(document, "audioInputs")?,
                "tickers": document.get("tickers").ok_or("Workspace tickers are required.")?,
                "recordingPresets": value_array(document, "recordingPresets")?
            }),
        )?;
    }
    write_json(
        &output_folder.join("Configuration/Settings.json"),
        json!({
            "schemaVersion": 1,
            "screenConfig": document.get("screenConfig").cloned().unwrap_or_else(|| json!({})),
            "countdownTimers": value_array(document, "countdownTimers")?,
            "groupSettings": document.get("groupSettings").cloned().unwrap_or_else(|| json!({})),
            "groups": value_array(document, "groups")?,
            "keyMappings": document.get("keyMappings").cloned().unwrap_or_else(|| json!({}))
        }),
    )?;
    write_manifest(
        &output_folder.join(WORKSPACE_FILE_NAME),
        manifest_path,
        document,
    )
}

fn write_manifest(
    output_path: &Path,
    manifest_path: &Path,
    workspace: &Map<String, Value>,
) -> Result<(), String> {
    let mut manifest = json!({
        "schema": workspace.get("schema").cloned().unwrap_or_else(|| json!("presenter-orbit.workspace")),
        "version": workspace.get("version").cloned().unwrap_or_else(|| json!(1)),
        "id": workspace.get("id").cloned().unwrap_or_else(|| json!(Uuid::new_v4().to_string())),
        "title": workspace.get("title").cloned().unwrap_or_else(|| json!("Presenter Orbit Workspace")),
        "workspacePath": manifest_path,
        "createdAt": workspace.get("createdAt").cloned().unwrap_or_else(|| json!(Utc::now().to_rfc3339())),
        "updatedAt": workspace.get("updatedAt").cloned().unwrap_or_else(|| json!(Utc::now().to_rfc3339())),
        "storage": workspace.get("storage").cloned().unwrap_or_else(|| json!({ "layout": "folders" })),
        "importedFrom": workspace.get("importedFrom").cloned().unwrap_or(Value::Null),
        "stats": workspace.get("stats").cloned().unwrap_or_else(|| json!({ "scannedFileCount": 0, "skippedFileCount": 0 })),
        "importWarnings": workspace.get("importWarnings").cloned().unwrap_or_else(|| json!([])),
        "clearGroups": value_array(workspace, "clearGroups")?,
        "triggerBindings": workspace.get("triggerBindings").cloned().unwrap_or_else(|| json!([])),
        "communicationDevices": workspace.get("communicationDevices").cloned().unwrap_or_else(|| json!([]))
        , "lastUsedPresentationThemeId": workspace.get("lastUsedPresentationThemeId").cloned().unwrap_or(Value::Null)
    });
    if let Some(recents) = workspace.get("recentPresentationThemeIds") {
        manifest["recentPresentationThemeIds"] = recents.clone();
    }
    if let Some(template) = workspace.get("ccliTemplate") {
        manifest["ccliTemplate"] = template.clone();
    }
    if let Some(recents) = workspace.get("recentTransitionEffectIds") {
        manifest["recentTransitionEffectIds"] = recents.clone();
    }
    if let Some(presets) = workspace.get("mediaEffectPresets") {
        manifest["mediaEffectPresets"] = presets.clone();
    }
    if let Some(transition) = workspace
        .get("slideTransition")
        .filter(|value| !value.is_null())
    {
        manifest["slideTransition"] = transition.clone();
    }
    if let Some(presets) = workspace.get("transitionPresets") {
        manifest["transitionPresets"] = presets.clone();
    }
    if let Some(layers) = workspace
        .get("layerTransitions")
        .filter(|value| value.as_object().is_some_and(|layers| !layers.is_empty()))
    {
        manifest["layerTransitions"] = layers.clone();
    }
    write_json(output_path, manifest)
}

fn write_libraries(folder: &Path, workspace: &Map<String, Value>) -> Result<(), String> {
    let root = folder.join("Libraries");
    let items = value_array(workspace, "library")?;
    let mut entries = Vec::new();
    let mut storage_folders = HashSet::new();
    for library in value_array(workspace, "libraries")? {
        let library = library
            .as_object()
            .ok_or_else(|| "Library must be an object.".to_string())?;
        let id = library
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Library id is required.".to_string())?;
        let title = library.get("title").and_then(Value::as_str).unwrap_or(id);
        let folder_name = safe_name(title, id);
        if !storage_folders.insert(folder_name.to_ascii_lowercase()) {
            return Err(format!(
                "Presentation library title '{title}' resolves to an existing storage folder."
            ));
        }
        let library_folder = root.join(&folder_name);
        fs::create_dir_all(&library_folder).map_err(|error| error.to_string())?;
        let item_ids = library
            .get("itemIds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut files = Vec::new();
        let mut item_names = crate::workspace_export::Names::default();
        for item_id in &item_ids {
            let item_id = item_id
                .as_str()
                .ok_or_else(|| "Library item id must be a string.".to_string())?;
            let item = items
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(item_id))
                .ok_or_else(|| format!("Library item {item_id} is missing."))?;
            let file = format!(
                "{}.json",
                item_names.reserve(
                    item.get("title").and_then(Value::as_str).unwrap_or(item_id),
                    "Presentation"
                )
            );
            write_json(&library_folder.join(&file), item.clone())?;
            files.push(Value::String(file));
        }
        let mut library_document = json!({
            "id": id,
            "title": title,
            "itemIds": item_ids,
            "items": files,
        });
        if let Some(presentation_theme_id) = library.get("presentationThemeId") {
            library_document["presentationThemeId"] = presentation_theme_id.clone();
        }
        write_json(&library_folder.join("library.json"), library_document)?;
        entries.push(json!({ "id": id, "title": title, "folder": folder_name }));
    }
    write_json(&root.join("index.json"), json!({ "libraries": entries }))
}

fn write_indexed(
    folder: &Path,
    directory: &str,
    key: &str,
    values: Vec<Value>,
) -> Result<(), String> {
    let root = folder.join(directory);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    for value in values {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{key} id is required."))?
            .to_string();
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(&id)
            .to_string();
        let file = unique_indexed_file_name(&title, &id, &entries);
        write_json(&root.join(&file), value)?;
        entries.push(json!({ "id": id, "title": title, "file": file }));
    }
    let mut index = Map::new();
    index.insert(key.to_string(), Value::Array(entries));
    write_json(&root.join("index.json"), Value::Object(index))
}

fn unique_indexed_file_name(title: &str, id: &str, entries: &[Value]) -> String {
    let base = safe_name(title, id);
    let mut suffix = 1;
    loop {
        let name = if suffix == 1 {
            format!("{base}.json")
        } else {
            format!("{base}-{suffix}.json")
        };
        if !entries
            .iter()
            .any(|entry| entry.get("file").and_then(Value::as_str) == Some(name.as_str()))
        {
            return name;
        }
        suffix += 1;
    }
}

fn write_media(
    source_folder: &Path,
    output_folder: &Path,
    workspace: &Map<String, Value>,
) -> Result<(), String> {
    let source_root = source_folder.join("Media");
    let output_root = output_folder.join("Media");
    let assets = output_root.join("Assets");
    fs::create_dir_all(&assets).map_err(|error| error.to_string())?;
    let existing = if source_root.join("index.json").exists() {
        read_json(&source_root.join("index.json"))?
    } else {
        json!({ "libraries": [], "media": [], "settings": {} })
    };
    let mut settings = existing
        .get("settings")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut media_libraries = value_array(workspace, "mediaLibraries")?;
    for library in &media_libraries {
        validate_audio_playlist_options(library)?;
        if let Some(mode) = library.get("audioPlaylistMode") {
            if !matches!(mode.as_str(), Some("stop" | "next" | "repeat"))
                || library.get("type").and_then(Value::as_str) != Some("playlist")
                || library.get("mediaType").and_then(Value::as_str) != Some("audio")
            {
                return Err(
                    "Audio playlist mode requires an audio playlist and stop, next or repeat."
                        .into(),
                );
            }
        }
    }
    let smart_library_ids = media_libraries
        .iter()
        .filter_map(Value::as_object)
        .filter(|library| library.get("type").and_then(Value::as_str) == Some("smart"))
        .filter_map(|library| library.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let mut media_entries = Vec::new();
    let mut existing_names = Vec::new();
    let mut persisted_asset_files = HashMap::new();
    for media in value_array(workspace, "media")? {
        validate_media_runtime_settings(&media)?;
        let source_path = media
            .get("sourcePath")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        let asset_file = media
            .get("assetFile")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(validated_workspace_relative_path)
            .transpose()?;
        if media
            .get("libraryId")
            .and_then(Value::as_str)
            .is_some_and(|library_id| smart_library_ids.contains(library_id))
        {
            store_media_background_settings(&mut settings, &media, source_path.as_deref(), None)?;
            continue;
        }
        let source = source_path
            .as_ref()
            .filter(|path| path.is_file())
            .cloned()
            .map(|path| (path, false))
            .or_else(|| {
                asset_file
                    .as_ref()
                    .map(|path| (source_root.join(path), true))
            });
        let external = media.get("externalSource").and_then(Value::as_bool) == Some(true);
        let Some((source, workspace_asset)) = source.filter(|(path, _)| path.is_file()) else {
            if external {
                let source = source_path
                    .as_deref()
                    .ok_or("External media requires a source path.")?;
                store_media_background_settings(&mut settings, &media, Some(source), None)?;
                let mut entry = media
                    .as_object()
                    .cloned()
                    .ok_or("Media entry must be an object.")?;
                for key in [
                    "assetFile",
                    "sourcePath",
                    "sourceUrl",
                    "thumbnailUrl",
                    "missing",
                    "thumbnailMissing",
                    "externalSource",
                ] {
                    entry.remove(key);
                }
                entry.insert("externalPath".into(), json!(source));
                media_entries.push(Value::Object(entry));
            } else if let Some(asset_file) = asset_file {
                if let Some(name) = asset_file.file_name().and_then(|name| name.to_str()) {
                    existing_names.push(name.to_string());
                }
                let mut entry = media
                    .as_object()
                    .cloned()
                    .ok_or("Media entry must be an object.")?;
                for key in [
                    "assetFile",
                    "sourcePath",
                    "sourceUrl",
                    "thumbnailUrl",
                    "missing",
                    "thumbnailMissing",
                ] {
                    entry.remove(key);
                }
                entry.insert(
                    "file".into(),
                    json!(asset_file.to_string_lossy().replace('\\', "/")),
                );
                media_entries.push(Value::Object(entry));
            } else {
                return Err("Media source is missing and has no managed asset reference.".into());
            }
            continue;
        };
        if workspace_asset {
            validate_existing_workspace_path(&source_root, &source)?;
        }
        // Unmanaged media stay at their original location; managed media are copied into Assets.
        let external = external && !workspace_asset;
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let base = media
            .get("title")
            .and_then(Value::as_str)
            .or_else(|| media.get("id").and_then(Value::as_str))
            .unwrap_or("Media");
        let asset_base = safe_name(base, "Media");
        let mut file_name = unique_asset_name(&asset_base, extension, &existing_names);
        if !external {
            loop {
                let live_destination = source_root.join("Assets").join(&file_name);
                if !live_destination.is_file()
                    || files_have_equal_contents(&source, &live_destination)?
                {
                    break;
                }
                existing_names.push(file_name);
                file_name = unique_asset_name(&asset_base, extension, &existing_names);
            }
        }
        let persisted_destination = source_root.join("Assets").join(&file_name);
        if external {
            store_media_background_settings(&mut settings, &media, Some(&source), None)?;
        } else {
            fs::copy(&source, assets.join(&file_name)).map_err(|error| error.to_string())?;
            store_media_background_settings(
                &mut settings,
                &media,
                source_path.as_deref(),
                Some(&persisted_destination),
            )?;
            existing_names.push(file_name.clone());
        }
        let mut entry = media
            .as_object()
            .cloned()
            .ok_or_else(|| "Media entry must be an object.".to_string())?;
        entry.remove("assetFile");
        entry.remove("sourcePath");
        entry.remove("sourceUrl");
        entry.remove("thumbnailUrl");
        entry.remove("missing");
        entry.remove("thumbnailMissing");
        entry.remove("externalSource");
        if let Some(file) = entry.get("thumbnailFile").and_then(Value::as_str) {
            let relative = validated_workspace_relative_path(file)?;
            let thumbnail = source_root.join(&relative);
            if thumbnail.exists() {
                validate_existing_workspace_path(&source_root, &thumbnail)?;
                let destination = output_root.join(&relative);
                fs::create_dir_all(
                    destination
                        .parent()
                        .ok_or("Thumbnail folder is required.")?,
                )
                .map_err(|e| e.to_string())?;
                fs::copy(thumbnail, destination)
                    .map_err(|e| format!("Video thumbnail could not be persisted: {e}"))?;
            }
        }
        if external {
            entry.insert("externalPath".to_string(), json!(source));
            media_entries.push(Value::Object(entry));
            continue;
        }
        let persisted_file = format!("Assets/{file_name}");
        if let Some(original_file) = media.get("assetFile").and_then(Value::as_str) {
            persisted_asset_files.insert(original_file.to_string(), persisted_file.clone());
        }
        entry.insert("file".to_string(), json!(persisted_file));
        media_entries.push(Value::Object(entry));
    }
    let mut smart_sources: HashMap<&str, std::collections::BTreeSet<&str>> = HashMap::new();
    let workspace_media = workspace.get("media").and_then(Value::as_array);
    for media in workspace_media.into_iter().flatten() {
        if let (Some(library_id), Some(path)) = (
            media
                .get("libraryId")
                .and_then(Value::as_str)
                .filter(|id| smart_library_ids.contains(*id)),
            media.get("sourcePath").and_then(Value::as_str),
        ) {
            smart_sources.entry(library_id).or_default().insert(path);
        }
    }
    for library in media_libraries.iter_mut().filter_map(Value::as_object_mut) {
        if library.get("type").and_then(Value::as_str) != Some("smart") {
            continue;
        }
        let id = library
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        match smart_sources.get(id.as_str()) {
            Some(paths) => library.insert("knownSources".into(), json!(paths)),
            None => library.remove("knownSources"),
        };
    }
    for library in &mut media_libraries {
        if let Some(files) = library.get_mut("itemFiles").and_then(Value::as_array_mut) {
            for file in files {
                if let Some(persisted_file) = file
                    .as_str()
                    .and_then(|file| persisted_asset_files.get(file))
                {
                    *file = json!(persisted_file);
                }
            }
        }
    }
    write_json(
        &output_root.join("index.json"),
        json!({
            "libraries": media_libraries,
            "media": media_entries,
            "settings": Value::Object(settings)
        }),
    )
}

fn store_media_background_settings(
    settings: &mut Map<String, Value>,
    media: &Value,
    source_path: Option<&Path>,
    asset_path: Option<&Path>,
) -> Result<(), String> {
    for path in [source_path, asset_path].into_iter().flatten() {
        let key = path.to_string_lossy().to_string();
        let has_thumbnail = media.get("thumbnailFile").is_some()
            || settings
                .get(&key)
                .is_some_and(|entry| entry.get("thumbnailFile").is_some());
        if !has_thumbnail {
            continue;
        }
        let entry = settings.entry(key).or_insert_with(|| json!({}));
        let object = entry
            .as_object_mut()
            .ok_or("Media settings must be objects.")?;
        for key in ["thumbnailFile", "thumbnailTimeSeconds"] {
            object.remove(key);
            if let Some(value) = media.get(key) {
                object.insert(key.into(), value.clone());
            }
        }
    }
    if ![
        "mediaDesaturate",
        "mediaEffects",
        "mediaBrightness",
        "mediaContrast",
        "mediaHue",
        "mediaSaturation",
        "mediaTransport",
        "audioRouting",
        "backgroundBehavior",
        "backgroundEndBehavior",
        "backgroundPlayback",
        "backgroundScale",
        "backgroundTransition",
    ]
    .iter()
    .any(|key| media.get(key).is_some())
    {
        return Ok(());
    }
    let mut value = json!({
        "backgroundBehavior": media.get("backgroundBehavior").and_then(Value::as_str).unwrap_or("foreground"),
        "backgroundEndBehavior": media.get("backgroundEndBehavior").and_then(Value::as_str).unwrap_or("hold"),
        "backgroundPlayback": media.get("backgroundPlayback").and_then(Value::as_str).unwrap_or("stop"),
        "backgroundScale": media.get("backgroundScale").and_then(Value::as_str).unwrap_or("fit"),
        "backgroundTransition": media.get("backgroundTransition").and_then(Value::as_str).unwrap_or("none")
    });
    for key in [
        "thumbnailFile",
        "thumbnailTimeSeconds",
        "mediaDesaturate",
        "mediaEffects",
        "mediaBrightness",
        "mediaContrast",
        "mediaHue",
        "mediaSaturation",
        "mediaTransport",
        "audioRouting",
    ] {
        if let Some(field) = media.get(key) {
            value[key] = field.clone();
        }
    }
    if let Some(path) = source_path {
        settings.insert(path.to_string_lossy().to_string(), value.clone());
    }
    if let Some(path) = asset_path {
        settings.insert(path.to_string_lossy().to_string(), value);
    }
    Ok(())
}

fn unique_asset_name(base: &str, extension: &str, existing: &[String]) -> String {
    let extension = extension.trim_start_matches('.');
    let suffix = if extension.is_empty() {
        String::new()
    } else {
        format!(".{extension}")
    };
    let mut name = format!("{base}{suffix}");
    let mut index = 2;
    while existing.iter().any(|value| value == &name) {
        name = format!("{base}-{index}{suffix}");
        index += 1;
    }
    name
}

fn write_themes(folder: &Path, themes: Vec<Value>) -> Result<(), String> {
    let root = folder.join("Themes");
    let mut entries = Vec::new();
    for theme in themes {
        let id = theme
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Theme id is required.".to_string())?
            .to_string();
        let title = theme
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(&id)
            .to_string();
        let name = safe_name(&title, &id);
        fs::create_dir_all(root.join(&name)).map_err(|error| error.to_string())?;
        write_json(&root.join(&name).join("theme.json"), theme)?;
        entries.push(json!({ "id": id, "title": title, "folder": name }));
    }
    write_json(&root.join("index.json"), json!({ "themes": entries }))
}

fn write_masks(folder: &Path, masks: Vec<Value>) -> Result<(), String> {
    let root = folder.join("Masks");
    let mut entries = Vec::new();
    let mut used_files = HashSet::new();
    for mask in masks {
        let id = mask
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Mask id is required.".to_string())?
            .to_string();
        let title = mask
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(&id)
            .to_string();
        let base = safe_name(&title, &id);
        let mut file = format!("{base}.json");
        let mut suffix = 2;
        while used_files.contains(&file.to_ascii_lowercase()) {
            file = format!("{base}-{suffix}.json");
            suffix += 1;
        }
        used_files.insert(file.to_ascii_lowercase());
        write_json(&root.join(&file), mask)?;
        entries.push(json!({ "id": id, "title": title, "file": file }));
    }
    write_json(&root.join("index.json"), json!({ "masks": entries }))
}

fn value_array(document: &Map<String, Value>, key: &str) -> Result<Vec<Value>, String> {
    Ok(document
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// Standard base64 with padding, as data URLs need it.
pub(crate) fn base64(bytes: &[u8]) -> String {
    propresenter_proto::base64_encode(bytes)
}

pub(crate) fn safe_name(value: &str, fallback: &str) -> String {
    let name: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('-');
    if name.is_empty() {
        fallback.to_string()
    } else {
        name.to_string()
    }
}

const WORKSPACE_VERSION: u64 = 1;

/// The workspace default and every presentation and slide transition must be playable.
fn validate_slide_transitions(document: &Map<String, Value>) -> Result<(), String> {
    let defined = |value: Option<&Value>| value.filter(|value| !value.is_null()).cloned();
    let mut transitions = vec![defined(document.get("slideTransition"))];
    for item in document
        .get("library")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        transitions.push(defined(item.get("transition")));
        for slide in item
            .get("slides")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            transitions.push(defined(slide.get("transition")));
        }
    }
    transitions
        .iter()
        .flatten()
        .try_for_each(crate::native_transitions::validate)
}

/// The media, prop and message layer defaults, every prop's own transition and every
/// media selection transition must be playable.
fn validate_layer_transitions(document: &Map<String, Value>) -> Result<(), String> {
    crate::native_transitions::validate_layer_defaults(document.get("layerTransitions"))?;
    let items = |key: &str| {
        document
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    };
    for prop in items("props") {
        if let Some(transition) = prop.get("transition").filter(|value| !value.is_null()) {
            crate::native_transitions::validate(transition)?;
        }
    }
    for media in items("media") {
        if let Some(transition) = media.get("backgroundTransition") {
            crate::native_transitions::media_transition(
                transition
                    .as_str()
                    .ok_or("Media transition must be a string.")?,
            )?;
        }
    }
    Ok(())
}

/// Transition favorites: named, uniquely identified library transitions.
pub fn new_transition_preset(name: &str, transition: Value) -> Result<Value, String> {
    if name.trim().is_empty() {
        return Err("transition.favoriteNameRequired".into());
    }
    crate::native_transitions::validate(&transition)?;
    Ok(json!({"id":Uuid::new_v4().to_string(),"name":name.trim(),"transition":transition}))
}

fn validate_transition_presets(value: Option<&Value>) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    let presets = value
        .as_array()
        .ok_or("transitionPresets must be an array.")?;
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for preset in presets {
        let object = preset
            .as_object()
            .ok_or("Transition favorite must be an object.")?;
        if object
            .keys()
            .any(|key| !["id", "name", "transition"].contains(&key.as_str()))
        {
            return Err("Unknown transition favorite field.".into());
        }
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or("Transition favorite id is required.")?;
        // ProPresenter identifies presets by UUID.
        if Uuid::parse_str(id).is_err() {
            return Err("Transition favorite id must be a UUID.".into());
        }
        if !ids.insert(id) {
            return Err("Transition favorite ids must be unique.".into());
        }
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or("Transition favorite name is required.")?;
        if !names.insert(name.trim().to_lowercase()) {
            return Err("Transition favorite names must be unique.".into());
        }
        crate::native_transitions::validate(
            object
                .get("transition")
                .ok_or("Transition favorite transition is required.")?,
        )?;
    }
    Ok(())
}

fn validate_manifest(manifest: &Map<String, Value>) -> Result<(), String> {
    if manifest.get("schema").and_then(Value::as_str) != Some("presenter-orbit.workspace")
        || manifest.get("version").and_then(Value::as_u64) != Some(WORKSPACE_VERSION)
    {
        return Err(
            "Workspace manifest must use schema presenter-orbit.workspace version 1.".to_string(),
        );
    }
    Ok(())
}

fn read_libraries(folder: &Path) -> Result<(Vec<Value>, Vec<Value>), String> {
    let index = read_json(&folder.join("Libraries/index.json"))?;
    let mut libraries = Vec::new();
    let mut items = Vec::new();
    for entry in array_field(&index, "libraries")? {
        let entry = entry
            .as_object()
            .ok_or_else(|| "Library index entry must be an object.".to_string())?;
        let library_folder = entry
            .get("folder")
            .and_then(Value::as_str)
            .ok_or_else(|| "Library folder is required.".to_string())?;
        let document = read_json(
            &folder
                .join("Libraries")
                .join(library_folder)
                .join("library.json"),
        )?;
        let item_files = array_field(&document, "items")?;
        let mut item_ids = Vec::new();
        for file in item_files {
            let file = file
                .as_str()
                .ok_or_else(|| "Library item file must be a string.".to_string())?;
            let item = read_json(&folder.join("Libraries").join(library_folder).join(file))?;
            item_ids.push(
                item.get("id")
                    .cloned()
                    .ok_or_else(|| "Library item id is required.".to_string())?,
            );
            items.push(item);
        }
        let mut library = document
            .as_object()
            .cloned()
            .ok_or_else(|| "Library document must be an object.".to_string())?;
        library.insert("itemIds".to_string(), Value::Array(item_ids));
        library.remove("items");
        libraries.push(Value::Object(library));
    }
    Ok((libraries, items))
}

fn read_indexed(folder: &Path, directory: &str, key: &str) -> Result<Value, String> {
    let index = read_json(&folder.join(directory).join("index.json"))?;
    let mut values = Vec::new();
    for entry in array_field(&index, key)? {
        let entry = entry
            .as_object()
            .ok_or_else(|| format!("{key} index entry must be an object."))?;
        let file = entry
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{key} file is required."))?;
        values.push(read_json(&folder.join(directory).join(file))?);
    }
    Ok(Value::Array(values))
}

fn normalize_playlists_value(value: &Value) -> Value {
    Value::Array(
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(normalize_playlist_value)
            .collect(),
    )
}

fn normalize_playlist_value(value: &Value) -> Option<Value> {
    let mut playlist = value.as_object()?.clone();
    let entries = playlist
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(normalize_playlist_entry_value)
        .collect::<Vec<_>>();
    let item_ids = entries
        .iter()
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("presentation"))
        .filter_map(|entry| entry.get("itemId").and_then(Value::as_str))
        .map(|item_id| Value::String(item_id.to_string()))
        .collect();
    playlist.insert("items".to_string(), Value::Array(entries));
    playlist.insert("itemIds".to_string(), Value::Array(item_ids));
    Some(Value::Object(playlist))
}

fn normalize_playlist_entry_value(value: &Value) -> Option<Value> {
    let mut entry = value.as_object()?.clone();
    if entry.remove("isHidden") == Some(json!(true)) {
        entry.insert("hidden".into(), json!(true));
    }
    let entry_type = match entry.get("type").and_then(Value::as_str) {
        Some("header") => "header",
        Some("placeholder") => "placeholder",
        _ => "presentation",
    };

    if entry_type == "presentation" {
        let item_id = playlist_scalar_string(entry.get("itemId"))
            .filter(|value| !value.is_empty())
            .or_else(|| {
                playlist_scalar_string(entry.get("id")).filter(|value| !value.is_empty())
            })?;
        let id = playlist_scalar_string(entry.get("id"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("presentation-{item_id}"));
        entry.insert("id".to_string(), Value::String(id));
        entry.insert("type".to_string(), Value::String(entry_type.to_string()));
        entry.insert("itemId".to_string(), Value::String(item_id));
        return Some(Value::Object(entry));
    }

    let fallback_title = if entry_type == "header" {
        "Header"
    } else {
        "Placeholder"
    };
    let title = playlist_scalar_string(entry.get("title"))
        .filter(|value| !value.is_empty())
        .or_else(|| playlist_scalar_string(entry.get("name")).filter(|value| !value.is_empty()))
        .unwrap_or_else(|| fallback_title.to_string());
    let id = playlist_scalar_string(entry.get("id"))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("{}-{}", entry_type, playlist_slug(&title)));
    entry.insert("id".to_string(), Value::String(id));
    entry.insert("type".to_string(), Value::String(entry_type.to_string()));
    entry.insert("title".to_string(), Value::String(title));
    Some(Value::Object(entry))
}

fn playlist_scalar_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn playlist_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            separator = false;
        } else if !separator && !slug.is_empty() {
            slug.push('-');
            separator = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn read_media(folder: &Path) -> Result<(Vec<Value>, Vec<Value>), String> {
    let media_index = read_json(&folder.join("Media/index.json"))?;
    let libraries = array_field(&media_index, "libraries")?;
    let mut media = Vec::new();
    for entry in array_field(&media_index, "media")? {
        let entry = entry
            .as_object()
            .ok_or_else(|| "Media index entry must be an object.".to_string())?;
        let external_path = entry.get("externalPath").and_then(Value::as_str);
        let file = match external_path {
            Some(path) => path,
            None => entry
                .get("file")
                .and_then(Value::as_str)
                .ok_or_else(|| "Media file is required.".to_string())?,
        };
        let path = if let Some(external_path) = external_path {
            let path = PathBuf::from(external_path);
            if !path.is_absolute() {
                return Err("External media path must be absolute.".into());
            }
            path
        } else {
            let path = workspace_target_path(
                folder,
                &PathBuf::from("Media").join(validated_workspace_relative_path(file)?),
            )?;
            if path.exists() {
                validate_existing_workspace_path(folder, &path)?;
            }
            path
        };
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let media_type = media_type_for_extension(&extension);
        let title = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Media")
            .to_string();
        let mut item = entry.clone();
        item.remove("file");
        item.remove("externalPath");
        item.insert(
            "id".to_string(),
            entry
                .get("id")
                .filter(|value| value.as_str().is_some_and(|id| !id.is_empty()))
                .cloned()
                .unwrap_or_else(|| json!(format!("media-{}", safe_name(file, "asset")))),
        );
        item.insert(
            "title".to_string(),
            entry
                .get("title")
                .filter(|value| value.as_str().is_some_and(|title| !title.is_empty()))
                .cloned()
                .unwrap_or_else(|| json!(title)),
        );
        item.insert(
            "type".to_string(),
            entry
                .get("type")
                .filter(|value| value.as_str().is_some_and(|kind| !kind.is_empty()))
                .cloned()
                .unwrap_or_else(|| json!(media_type)),
        );
        if external_path.is_some() {
            item.insert("externalSource".to_string(), json!(true));
        } else {
            item.insert("assetFile".to_string(), json!(file));
        }
        item.insert("sourcePath".to_string(), json!(path));
        item.insert("sourceUrl".to_string(), json!(file_url(&path)?));
        item.insert("missing".into(), json!(!path.is_file()));
        apply_media_background_settings(&mut item, &path, &media_index);
        apply_media_thumbnail_url(&mut item, folder)?;
        validate_media_runtime_settings(&Value::Object(item.clone()))?;
        media.push(Value::Object(item));
    }
    let mut media_ids = media
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    for library in libraries.iter().filter_map(Value::as_object) {
        let is_smart = library.get("type").and_then(Value::as_str) == Some("smart");
        let source_path = library.get("sourcePath").and_then(Value::as_str);
        if !is_smart {
            continue;
        }
        if let Some(source_path) = source_path {
            let library_id = library
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // The folder's import behavior applies unless a file has its own setting.
            let behavior = match library.get("backgroundBehavior") {
                None => None,
                Some(value) => Some(
                    value
                        .as_str()
                        .filter(|value| matches!(*value, "background" | "foreground"))
                        .ok_or("Smart library behavior must be background or foreground.")?,
                ),
            };
            let mut scanned = scan_media_files(Path::new(source_path), library_id)?;
            for media in &mut scanned {
                let Some(item) = media.as_object_mut() else {
                    continue;
                };
                let Some(base_id) = item.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let mut media_id = base_id.to_string();
                if media_ids.contains(&media_id) {
                    let library_suffix = safe_name(library_id, "library");
                    let base = format!("{base_id}-{library_suffix}");
                    media_id = base.clone();
                    let mut suffix = 2;
                    while media_ids.contains(&media_id) {
                        media_id = format!("{base}-{suffix}");
                        suffix += 1;
                    }
                }
                item.insert("id".to_string(), json!(&media_id));
                media_ids.insert(media_id);
                if let Some(behavior) = behavior {
                    item.insert("backgroundBehavior".into(), json!(behavior));
                }
                let Some(path) = item
                    .get("sourcePath")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                else {
                    continue;
                };
                apply_media_background_settings(item, &path, &media_index);
                apply_media_thumbnail_url(item, folder)?;
                validate_media_runtime_settings(&Value::Object(item.clone()))?;
            }
            let found = scanned
                .iter()
                .filter_map(|item| item.get("sourcePath").and_then(Value::as_str))
                .map(PathBuf::from)
                .collect::<HashSet<_>>();
            for path in known_smart_sources(library, Path::new(source_path))? {
                if found.contains(&path) {
                    continue;
                }
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let media_type = media_type_for_extension(&extension);
                if media_type == "media" {
                    return Err(format!(
                        "Known smart-folder source '{}' is not a media file.",
                        path.display()
                    ));
                }
                let mut entry = smart_media_entry(&path, library_id, media_type)?;
                let item = entry
                    .as_object_mut()
                    .ok_or("Smart media entry must be an object.")?;
                if !media_ids.insert(item["id"].as_str().unwrap_or_default().to_string()) {
                    continue;
                }
                item.insert("missing".into(), json!(true));
                if let Some(behavior) = behavior {
                    item.insert("backgroundBehavior".into(), json!(behavior));
                }
                apply_media_background_settings(item, &path, &media_index);
                apply_media_thumbnail_url(item, folder)?;
                validate_media_runtime_settings(&entry)?;
                scanned.push(entry);
            }
            media.extend(scanned);
        }
    }
    Ok((libraries, media))
}

fn apply_media_thumbnail_url(item: &mut Map<String, Value>, folder: &Path) -> Result<(), String> {
    item.remove("thumbnailUrl");
    item.remove("thumbnailMissing");
    if let Some(file) = item.get("thumbnailFile").and_then(Value::as_str) {
        let relative = validated_workspace_relative_path(file)?;
        let thumbnail = folder.join("Media").join(relative);
        if thumbnail.exists() {
            validate_existing_workspace_path(folder, &thumbnail)?;
        }
        if thumbnail.is_file() {
            item.insert("thumbnailUrl".into(), json!(file_url(&thumbnail)?));
        } else {
            item.insert("thumbnailMissing".into(), json!(true));
        }
    }
    Ok(())
}

fn validate_audio_marker_dependencies(document: &Map<String, Value>) -> Result<(), String> {
    for media in document
        .get("media")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|media| media["type"] == "audio")
    {
        validate_media_runtime_settings(media)?;
        for marker in media
            .get("markers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mut ids = HashSet::new();
            for action in marker["actions"]
                .as_array()
                .ok_or("Audio marker actions must be an array.")?
            {
                let id = action["id"]
                    .as_str()
                    .filter(|id| !id.trim().is_empty())
                    .ok_or("Audio marker action id is required.")?;
                if !ids.insert(id) {
                    return Err("Audio marker action ids must be unique.".into());
                }
                if action.get("delaySeconds").is_some_and(|delay| {
                    delay
                        .as_f64()
                        .is_none_or(|delay| !delay.is_finite() || delay < 0.0)
                }) {
                    return Err("Audio marker action delay is invalid.".into());
                }
                let kind = action["type"]
                    .as_str()
                    .ok_or("Audio marker action type is required.")?;
                let collection = match kind {
                    "media" => Some("media"),
                    "macro" => Some("macros"),
                    "message" => Some("messages"),
                    "presentation" => Some("library"),
                    "playlist" => Some("playlist"),
                    "prop" => Some("props"),
                    "stage" => Some("stageLayouts"),
                    "audience-look" => Some("audienceLooks"),
                    "video-input" => Some("videoInputs"),
                    "audio-input" if action.get("targetId").is_some() => Some("audioInputs"),
                    "mask" => Some("masks"),
                    "timer" => Some("countdownTimers"),
                    "clear-group" => Some("clearGroups"),
                    "communication" => Some("communicationDevices"),
                    _ => None,
                };
                if let Some(collection) = collection {
                    let target = action["targetId"]
                        .as_str()
                        .filter(|target| !target.is_empty())
                        .ok_or("Audio marker action target is required.")?;
                    if !document
                        .get(collection)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .any(|entry| {
                            entry["id"].as_str() == Some(target)
                                || entry["sourceId"].as_str() == Some(target)
                        })
                    {
                        return Err(format!("Audio marker action target is missing: {target}"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_media_runtime_settings(media: &Value) -> Result<(), String> {
    if let Some(markers) = media.get("markers") {
        let mut ids = HashSet::new();
        for marker in markers
            .as_array()
            .ok_or("Media markers must be an array.")?
        {
            if let Some(color) = marker.get("color") {
                if !color.as_str().is_some_and(|color| {
                    color.len() == 7
                        && color.starts_with('#')
                        && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
                }) {
                    return Err("Playback marker color must be a six-digit hex color.".into());
                }
            }
            if media["type"] == "audio" {
                let id = marker["id"]
                    .as_str()
                    .filter(|id| !id.trim().is_empty())
                    .ok_or("Audio marker id is required.")?;
                if !ids.insert(id) {
                    return Err("Audio marker ids must be unique.".into());
                }
                if marker["title"]
                    .as_str()
                    .is_none_or(|title| title.trim().is_empty())
                    || marker["timeSeconds"]
                        .as_f64()
                        .is_none_or(|time| !time.is_finite() || time < 0.0)
                    || !marker["actions"].is_array()
                {
                    return Err(
                        "Audio marker title, finite non-negative time and actions are required."
                            .into(),
                    );
                }
            }
            if let Some(repeat) = marker.get("repeatOnLoop") {
                if !repeat.is_boolean() {
                    return Err("Media marker repeatOnLoop must be a boolean.".into());
                }
            }
            if marker
                .get("repeatOnLoop")
                .is_none_or(|repeat| repeat == &json!(true))
                && media["type"] == "video"
            {
                if let Some(transport) = media.get("mediaTransport") {
                    crate::output_playback::validate_media_transport(transport)?;
                    let time = marker
                        .get("timeSeconds")
                        .and_then(Value::as_f64)
                        .filter(|time| time.is_finite())
                        .ok_or("Repeating media marker time must be finite.")?;
                    if time < transport["inSeconds"].as_f64().expect("validated in point")
                        || transport["outSeconds"]
                            .as_f64()
                            .is_some_and(|end| time >= end)
                    {
                        return Err(
                            "Repeating media marker must be within its media trim range.".into(),
                        );
                    }
                }
            }
        }
    }
    crate::output_scene::validate_media_color_effects(media)?;
    if let Some(effect) = media.get("mediaDesaturate") {
        if !matches!(
            media.get("type").and_then(Value::as_str),
            Some("image") | Some("video")
        ) || !effect.is_boolean()
        {
            return Err("Desaturation requires an image/video asset and a boolean setting.".into());
        }
    }
    if let Some(transport) = media.get("mediaTransport") {
        if media.get("type").and_then(Value::as_str) != Some("video") {
            return Err("Media transport settings require a video asset.".into());
        }
        crate::output_playback::validate_media_transport(transport)?;
    }
    if let Some(playback) = media.get("audioPlayback") {
        if media.get("type").and_then(Value::as_str) != Some("audio")
            || !matches!(
                playback.as_str(),
                Some("stop" | "next" | "loop" | "loop-count" | "loop-time")
            )
        {
            return Err("Audio playback behavior is invalid.".into());
        }
        if playback == "loop-count"
            && !media["audioLoopCount"]
                .as_u64()
                .is_some_and(|count| count > 0 && count <= u32::MAX as u64)
        {
            return Err("Audio loop count is invalid.".into());
        }
        if playback == "loop-time"
            && !media["audioLoopTime"]
                .as_f64()
                .is_some_and(|time| time.is_finite() && time > 0.0)
        {
            return Err("Audio loop time is invalid.".into());
        }
    }
    if let Some(routing) = media.get("audioRouting") {
        if !matches!(
            media.get("type").and_then(Value::as_str),
            Some("audio" | "video")
        ) {
            return Err("Per-asset audio routing requires an audio or video asset.".into());
        }
        crate::native_audio::validate_audio_routing(routing)?;
    }
    Ok(())
}

fn validate_media_effect_presets(value: Option<&Value>) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    let presets = value
        .as_array()
        .ok_or("mediaEffectPresets must be an array.")?;
    let mut ids = HashSet::new();
    let mut titles = HashSet::new();
    for preset in presets {
        let object = preset
            .as_object()
            .ok_or("Media effect preset must be an object.")?;
        if object
            .keys()
            .any(|key| !["id", "title", "effects", "mediaEffects"].contains(&key.as_str()))
        {
            return Err("Unknown media effect preset field.".into());
        }
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or("Media effect preset id is required.")?;
        if !ids.insert(id) {
            return Err("Media effect preset ids must be unique.".into());
        }
        let title = object
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .ok_or("Media effect preset title is required.")?;
        if !titles.insert(title.trim().to_lowercase()) {
            return Err("Media effect preset titles must be unique.".into());
        }
        let effects = object
            .get("effects")
            .and_then(Value::as_object)
            .ok_or("Media effect preset effects must be an object.")?;
        let fields = [
            "mediaHue",
            "mediaSaturation",
            "mediaBrightness",
            "mediaContrast",
        ];
        if effects.len() != fields.len() || fields.iter().any(|key| !effects.contains_key(*key)) {
            return Err("Media effect presets require four canonical color adjustments.".into());
        }
        crate::native_compositor::validate_object_color_effects(effects)?;
        if let Some(chain) = object.get("mediaEffects") {
            crate::native_media_effects::validate_chain(chain)?;
        }
    }
    Ok(())
}

fn validate_capture_presets(value: Option<&Value>) -> Result<(), String> {
    let presets = value
        .and_then(Value::as_array)
        .ok_or("recordingPresets must be an array.")?;
    let mut ids = HashSet::new();
    let mut active = false;
    for entry in presets {
        if let Some(value) = entry.get("active") {
            let selected = value
                .as_bool()
                .ok_or("Recording preset active must be a boolean.")?;
            if selected && std::mem::replace(&mut active, true) {
                return Err("Only one recording preset may be active.".into());
            }
        }
        if let Some(config) = entry.get("capture") {
            let preset: crate::capture::CapturePreset = serde_json::from_value(config.clone())
                .map_err(|error| format!("Invalid recording preset: {error}"))?;
            preset.validate()?;
            if entry.get("id").and_then(Value::as_str) != Some(preset.id.as_str())
                || entry.get("title").and_then(Value::as_str) != Some(preset.title.as_str())
            {
                return Err(
                    "Recording preset identity must match its capture configuration.".into(),
                );
            }
            if !ids.insert(preset.id) {
                return Err("Duplicate recording preset id.".into());
            }
        }
    }
    Ok(())
}

fn validate_changed_clear_group_icons(
    previous: &Value,
    document: &Map<String, Value>,
) -> Result<(), String> {
    for group in document
        .get("clearGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(data) = group.get("imageData").and_then(Value::as_str) else {
            continue;
        };
        let unchanged = previous
            .get("clearGroups")
            .and_then(Value::as_array)
            .and_then(|groups| groups.iter().find(|old| old.get("id") == group.get("id")))
            .is_some_and(|old| old.get("imageData") == group.get("imageData"));
        if !unchanged {
            crate::macro_image::validate_data(data)?;
        }
    }
    Ok(())
}

fn validate_changed_macro_images(
    previous: &Value,
    document: &Map<String, Value>,
) -> Result<(), String> {
    let Some(macros) = document.get("macros").and_then(Value::as_array) else {
        return Ok(());
    };
    for item in macros {
        if item.get("imageType").and_then(Value::as_str) != Some("ImageTypeCustom") {
            continue;
        }
        let old = previous
            .get("macros")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|old| old.get("id") == item.get("id")));
        if old.is_some_and(|old| {
            old.get("imageData") == item.get("imageData")
                && old.get("imageType") == item.get("imageType")
        }) {
            continue;
        }
        let data = item
            .get("imageData")
            .and_then(Value::as_str)
            .ok_or("Custom macro image data is required.")?;
        crate::macro_image::validate_data(data)?;
    }
    Ok(())
}

fn apply_media_background_settings(item: &mut Map<String, Value>, path: &Path, index: &Value) {
    let path_key = path.to_string_lossy().replace('\\', "/");
    let Some(settings) = index
        .get("settings")
        .and_then(Value::as_object)
        .and_then(|settings| {
            settings
                .iter()
                .find_map(|(key, value)| (key.replace('\\', "/") == path_key).then_some(value))
        })
        .and_then(Value::as_object)
    else {
        return;
    };
    for (key, value) in settings {
        item.insert(key.clone(), value.clone());
    }
}

pub fn scan_media_folder(folder: &Path) -> Result<Value, String> {
    if !folder.is_dir() {
        return Ok(json!({ "library": Value::Null, "media": [] }));
    }
    let resolved = folder.canonicalize().map_err(|error| error.to_string())?;
    let id = format!(
        "smart-{}",
        safe_name(&resolved.to_string_lossy(), "playlist")
    );
    let title = resolved
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Smart Playlist");
    Ok(json!({
        "library": { "id": id, "title": title, "type": "smart", "sourcePath": resolved },
        "media": scan_media_files(&resolved, &id)?
    }))
}

fn scan_media_files(folder: &Path, library_id: &str) -> Result<Vec<Value>, String> {
    let mut files = Vec::new();
    if !folder.is_dir() {
        return Ok(files);
    }
    let entries = fs::read_dir(folder).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            files.extend(scan_media_files(&path, library_id)?);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let media_type = media_type_for_extension(&extension);
        if media_type == "media" {
            continue;
        }
        files.push(smart_media_entry(&path, library_id, media_type)?);
    }
    files.sort_by(|left, right| {
        left.get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(
                right
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
    });
    Ok(files)
}

/// A smart-folder media entry; its id derives from the library and the file path.
fn smart_media_entry(path: &Path, library_id: &str, media_type: &str) -> Result<Value, String> {
    let mut entry = json!({
        "id": format!(
            "media-{}-{}",
            safe_name(library_id, "library"),
            safe_name(&path.to_string_lossy(), "asset")
        ),
        "title": path.file_stem().and_then(|value| value.to_str()).unwrap_or("Media"),
        "type": media_type,
        "libraryId": library_id,
        "sourcePath": path,
        "sourceUrl": file_url(path)?
    });
    if let Some(item) = entry.as_object_mut() {
        crate::general_settings_options::apply(item);
    }
    Ok(entry)
}

/// The files a smart library is known to hold (`knownSources`): every file it showed when
/// the workspace was last saved, or that the ProPresenter smart playlist it was imported
/// from had synced. Each must lie inside the library folder.
fn known_smart_sources(library: &Map<String, Value>, root: &Path) -> Result<Vec<PathBuf>, String> {
    let Some(known) = library.get("knownSources") else {
        return Ok(Vec::new());
    };
    known
        .as_array()
        .ok_or("Smart library known sources must be an array.")?
        .iter()
        .map(|value| {
            let path = PathBuf::from(
                value
                    .as_str()
                    .ok_or("Smart library known sources must be paths.")?,
            );
            if !path.starts_with(root)
                || path
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                return Err(format!(
                    "Known smart-folder source '{}' is outside its folder.",
                    path.display()
                ));
            }
            Ok(path)
        })
        .collect()
}

pub(crate) fn media_type_for_extension(extension: &str) -> &'static str {
    match extension {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" => "image",
        "mp4" | "mov" | "m4v" | "webm" | "avi" | "mkv" => "video",
        "mp3" | "wav" | "m4a" | "aac" | "aiff" | "flac" => "audio",
        _ => "media",
    }
}

fn file_url(path: &Path) -> Result<String, String> {
    url::Url::from_file_path(path)
        .map(|value| value.to_string())
        .map_err(|_| "Could not convert media path to a URL.".to_string())
}

pub fn library_item_location(path: &Path, item_id: &str) -> Result<String, String> {
    let folder = path
        .parent()
        .ok_or_else(|| "Workspace path has no parent folder.".to_string())?;
    let index = read_json(&folder.join("Libraries/index.json"))?;
    for entry in array_field(&index, "libraries")? {
        let entry = entry
            .as_object()
            .ok_or_else(|| "Library index entry must be an object.".to_string())?;
        let library_folder = entry
            .get("folder")
            .and_then(Value::as_str)
            .ok_or_else(|| "Library folder is required.".to_string())?;
        let library_path = folder.join("Libraries").join(library_folder);
        let document = read_json(&library_path.join("library.json"))?;
        for file in array_field(&document, "items")? {
            let file = file
                .as_str()
                .ok_or_else(|| "Library item file must be a string.".to_string())?;
            let file_path = library_path.join(file);
            if read_json(&file_path)
                .ok()
                .and_then(|value| value.get("id").cloned())
                .and_then(|value| value.as_str().map(str::to_string))
                .as_deref()
                == Some(item_id)
            {
                return Ok(file_path.to_string_lossy().to_string());
            }
        }
    }
    Ok(String::new())
}

fn read_themes(folder: &Path) -> Result<Value, String> {
    let index = read_json(&folder.join("Themes/index.json"))?;
    let mut values = Vec::new();
    for entry in array_field(&index, "themes")? {
        let entry = entry
            .as_object()
            .ok_or_else(|| "Theme index entry must be an object.".to_string())?;
        let theme_folder = entry
            .get("folder")
            .and_then(Value::as_str)
            .ok_or_else(|| "Theme folder is required.".to_string())?;
        let mut theme = read_json(&folder.join("Themes").join(theme_folder).join("theme.json"))?;
        normalize_propresentation_theme_opacity(&mut theme);
        values.push(theme);
    }
    Ok(Value::Array(values))
}

fn normalize_propresentation_theme_opacity(theme: &mut Value) {
    if theme.get("sourceType").and_then(Value::as_str) != Some("propresenter") {
        return;
    }

    let Some(slides) = theme.get_mut("slides").and_then(Value::as_array_mut) else {
        return;
    };

    for slide in slides {
        let Some(layers) = slide.get_mut("layers").and_then(Value::as_array_mut) else {
            continue;
        };
        for layer in layers {
            let Some(layer) = layer.as_object_mut() else {
                continue;
            };
            normalize_fractional_opacity(layer, "opacity");
            normalize_fractional_opacity(layer, "shadowOpacity");
        }
    }
}

fn normalize_fractional_opacity(layer: &mut Map<String, Value>, key: &str) {
    let Some(value) = layer.get(key).and_then(Value::as_f64) else {
        return;
    };
    if (0.0..=1.0).contains(&value) {
        layer.insert(key.to_string(), json!(value * 100.0));
    }
}

fn read_masks(folder: &Path) -> Result<Value, String> {
    let index = read_json(&folder.join("Masks/index.json"))?;
    let mut values = Vec::new();
    for entry in array_field(&index, "masks")? {
        let entry = entry
            .as_object()
            .ok_or_else(|| "Mask index entry must be an object.".to_string())?;
        let file = entry
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| "Mask file is required.".to_string())?;
        values.push(read_json(&folder.join("Masks").join(file))?);
    }
    normalize_imported_canvas_documents(&mut values)?;
    Ok(Value::Array(values))
}

fn merge_document(
    target: &mut Map<String, Value>,
    source: &Value,
    keys: &[&str],
) -> Result<(), String> {
    let source = source
        .as_object()
        .ok_or_else(|| "Workspace configuration must be an object.".to_string())?;
    for key in keys {
        if let Some(value) = source.get(*key) {
            target.insert((*key).to_string(), value.clone());
        }
    }
    Ok(())
}

fn array_field(document: &Value, key: &str) -> Result<Vec<Value>, String> {
    document
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| format!("{key} must be an array."))
}

fn validated_workspace_relative_path(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Workspace file path must stay inside the workspace.".to_string());
    }
    Ok(path)
}

fn workspace_target_path(folder: &Path, relative: &Path) -> Result<PathBuf, String> {
    let value = relative.to_string_lossy();
    let relative = validated_workspace_relative_path(&value)?;
    let canonical_folder = folder.canonicalize().map_err(|error| error.to_string())?;
    let target = folder.join(&relative);
    let mut current = folder.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err("Workspace file path must stay inside the workspace.".to_string());
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err("Workspace target cannot pass through a symlink.".to_string());
                }
                let canonical = current.canonicalize().map_err(|error| error.to_string())?;
                if !canonical.starts_with(&canonical_folder) {
                    return Err("Workspace target resolves outside the workspace.".to_string());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(target)
}

fn validate_existing_workspace_path(root: &Path, path: &Path) -> Result<(), String> {
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let canonical_path = path.canonicalize().map_err(|error| error.to_string())?;
    if !canonical_path.starts_with(canonical_root) {
        return Err("Workspace asset resolves outside the workspace.".to_string());
    }
    Ok(())
}

fn transaction_journal_path(transaction: &Path) -> PathBuf {
    transaction.join(WORKSPACE_TRANSACTION_JOURNAL)
}

fn transaction_committed_path(transaction: &Path) -> PathBuf {
    transaction.with_extension(WORKSPACE_TRANSACTION_COMMITTED)
}

fn collect_transaction_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_symlink() {
            return Err("Workspace transaction payload cannot contain symlinks.".to_string());
        }
        if file_type.is_dir() {
            collect_transaction_files(root, &entry.path(), files)?;
        } else if file_type.is_file() {
            files.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|error| error.to_string())?
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

fn transaction_commit_priority(path: &Path) -> u8 {
    if path == Path::new(WORKSPACE_FILE_NAME) {
        3
    } else if path.file_name().and_then(|value| value.to_str()) == Some("index.json")
        || path.file_name().and_then(|value| value.to_str()) == Some("library.json")
    {
        2
    } else if path.starts_with("Configuration") {
        1
    } else {
        0
    }
}

fn sync_file(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
        if metadata.permissions().readonly() {
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions).map_err(|error| error.to_string())?;
        }
        return OpenOptions::new()
            .write(true)
            .open(path)
            .and_then(|file| file.sync_all())
            .map_err(|error| error.to_string());
    }

    #[cfg(not(windows))]
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())
}

fn files_have_equal_contents(left: &Path, right: &Path) -> Result<bool, String> {
    if fs::metadata(left).map_err(|error| error.to_string())?.len()
        != fs::metadata(right)
            .map_err(|error| error.to_string())?
            .len()
    {
        return Ok(false);
    }
    let mut left = BufReader::new(File::open(left).map_err(|error| error.to_string())?);
    let mut right = BufReader::new(File::open(right).map_err(|error| error.to_string())?);
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];
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

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Workspace transaction path has no parent.".to_string())?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory_chain(path: &Path, root: &Path) -> Result<(), String> {
    let mut current = path;
    loop {
        File::open(current)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| error.to_string())?;
        if current == root {
            return Ok(());
        }
        current = current
            .parent()
            .ok_or_else(|| "Workspace transaction directory escaped its root.".to_string())?;
    }
}

#[cfg(not(unix))]
fn sync_directory_chain(_path: &Path, _root: &Path) -> Result<(), String> {
    Ok(())
}

fn prepare_workspace_transaction(
    folder: &Path,
    transaction: &Path,
) -> Result<WorkspaceTransactionJournal, String> {
    let staged = transaction.join("staged");
    let mut files = Vec::new();
    collect_transaction_files(&staged, &staged, &mut files)?;
    files.sort_by(|left, right| {
        transaction_commit_priority(left)
            .cmp(&transaction_commit_priority(right))
            .then_with(|| left.cmp(right))
    });
    let mut entries = Vec::new();
    for path in files {
        let value = path.to_string_lossy().to_string();
        validated_workspace_relative_path(&value)?;
        let staged_path = staged.join(&path);
        let target = workspace_target_path(folder, &path)?;
        if target.is_file() && files_have_equal_contents(&staged_path, &target)? {
            fs::remove_file(staged_path).map_err(|error| error.to_string())?;
            continue;
        }
        entries.push(WorkspaceTransactionEntry {
            existed: target.is_file(),
            path: value,
        });
    }
    for entry in &entries {
        sync_file(&staged.join(validated_workspace_relative_path(&entry.path)?))?;
    }
    for entry in entries.iter().filter(|entry| entry.existed) {
        let relative = validated_workspace_relative_path(&entry.path)?;
        let backup_parent = transaction
            .join("backup")
            .join(relative)
            .parent()
            .ok_or_else(|| "Workspace backup path has no parent.".to_string())?
            .to_path_buf();
        fs::create_dir_all(&backup_parent).map_err(|error| error.to_string())?;
        sync_directory_chain(&backup_parent, transaction)?;
    }
    let journal = WorkspaceTransactionJournal { entries };
    let journal_path = transaction_journal_path(transaction);
    write_json(
        &journal_path,
        serde_json::to_value(&journal).map_err(|error| error.to_string())?,
    )?;
    sync_file(&journal_path)?;
    sync_parent_directory(&journal_path)?;
    Ok(journal)
}

fn apply_workspace_transaction_entries(
    folder: &Path,
    transaction: &Path,
    journal: &WorkspaceTransactionJournal,
    entry_limit: usize,
) -> Result<bool, String> {
    let staged = transaction.join("staged");
    let backup = transaction.join("backup");
    for (index, entry) in journal.entries.iter().enumerate() {
        if index == entry_limit {
            return Ok(false);
        }
        let relative = validated_workspace_relative_path(&entry.path)?;
        let source = staged.join(&relative);
        let target = workspace_target_path(folder, &relative)?;
        let previous = backup.join(&relative);
        if !source.is_file() {
            return Err(format!(
                "Workspace transaction payload is missing '{}'.",
                entry.path
            ));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if entry.existed {
            if !target.is_file() {
                return Err(format!(
                    "Workspace transaction target changed before commit: '{}'.",
                    entry.path
                ));
            }
            if let Some(parent) = previous.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::rename(&target, &previous).map_err(|error| {
                format!("Could not back up workspace file '{}': {error}", entry.path)
            })?;
            sync_parent_directory(&previous)?;
        } else if target.exists() {
            return Err(format!(
                "Workspace transaction target was created concurrently: '{}'.",
                entry.path
            ));
        }
        fs::rename(&source, &target).map_err(|error| {
            format!("Could not commit workspace file '{}': {error}", entry.path)
        })?;
        sync_parent_directory(&target)?;
    }
    Ok(true)
}

fn commit_workspace_transaction(
    folder: &Path,
    transaction: &Path,
    journal: &WorkspaceTransactionJournal,
) -> Result<bool, String> {
    if !apply_workspace_transaction_entries(folder, transaction, journal, usize::MAX)? {
        return Err("Workspace transaction stopped before commit.".to_string());
    }
    let committed = transaction_committed_path(transaction);
    let temporary = transaction.join(format!("committed-{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, b"committed").map_err(|error| error.to_string())?;
    sync_file(&temporary)?;
    if let Err(error) = fs::rename(&temporary, &committed) {
        let _ = fs::remove_file(&temporary);
        if committed.exists() {
            return Ok(false);
        }
        return Err(error.to_string());
    }
    Ok(sync_parent_directory(&committed).is_ok())
}

fn finalize_committed_workspace_transaction(transaction: &Path) -> Result<(), String> {
    let committed = transaction_committed_path(transaction);
    if transaction.exists() {
        fs::remove_dir_all(transaction).map_err(|error| error.to_string())?;
    }
    if committed.exists() {
        fs::remove_file(&committed).map_err(|error| error.to_string())?;
        sync_parent_directory(&committed)?;
    }
    Ok(())
}

fn rollback_workspace_transaction(
    folder: &Path,
    transaction: &Path,
    journal: &WorkspaceTransactionJournal,
) -> Result<(), String> {
    let staged = transaction.join("staged");
    let backup = transaction.join("backup");
    for entry in journal.entries.iter().rev() {
        let relative = validated_workspace_relative_path(&entry.path)?;
        let source = staged.join(&relative);
        let target = workspace_target_path(folder, &relative)?;
        let previous = backup.join(&relative);
        if previous.is_file() {
            if target.exists() {
                if !target.is_file() {
                    return Err(format!(
                        "Workspace rollback target is not a file: '{}'.",
                        entry.path
                    ));
                }
                fs::remove_file(&target).map_err(|error| error.to_string())?;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::rename(&previous, &target).map_err(|error| error.to_string())?;
            sync_parent_directory(&target)?;
        } else if !entry.existed && !source.exists() && target.is_file() {
            fs::remove_file(&target).map_err(|error| error.to_string())?;
            sync_parent_directory(&target)?;
        }
    }
    fs::remove_dir_all(transaction).map_err(|error| error.to_string())
}

fn transaction_backup_contains_files(path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let mut files = Vec::new();
    collect_transaction_files(path, path, &mut files)?;
    Ok(!files.is_empty())
}

fn recover_pending_workspace_write(path: &Path) -> Result<(), String> {
    let folder = path
        .parent()
        .ok_or_else(|| "Workspace path has no parent folder.".to_string())?;
    let transaction = folder.join(WORKSPACE_TRANSACTION_DIRECTORY);
    let committed = transaction_committed_path(&transaction);
    if committed.exists() {
        return finalize_committed_workspace_transaction(&transaction);
    }
    if !transaction.exists() {
        return Ok(());
    }
    let journal_path = transaction_journal_path(&transaction);
    let journal = match read_json(&journal_path)
        .and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string()))
    {
        Ok(journal) => journal,
        Err(error) => {
            if transaction_backup_contains_files(&transaction.join("backup"))? {
                return Err(format!(
                    "Workspace transaction journal is unreadable after commit started: {error}"
                ));
            }
            fs::remove_dir_all(&transaction).map_err(|remove_error| remove_error.to_string())?;
            return Ok(());
        }
    };
    rollback_workspace_transaction(folder, &transaction, &journal)
}

pub(crate) fn read_json(path: &Path) -> Result<Value, String> {
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&contents).map_err(|error| error.to_string())
}

fn write_json(path: &Path, value: Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let contents = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "JSON path must have a file name.".to_string())?;
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, contents).map_err(|error| error.to_string())?;
    // Staged transaction files are synced by prepare_workspace_transaction only when they differ from the
    // committed file; syncing every staged file here made each save fsync the whole workspace.
    let staged = path
        .components()
        .any(|part| part.as_os_str() == WORKSPACE_TRANSACTION_DIRECTORY);
    if !staged {
        if let Err(error) = sync_file(&temporary) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    }
    if !path.exists() {
        fs::rename(&temporary, path).map_err(|error| error.to_string())?;
        return if staged {
            Ok(())
        } else {
            sync_parent_directory(path)
        };
    }

    let backup = path.with_file_name(format!(".{file_name}.{}.previous", Uuid::new_v4()));
    fs::rename(path, &backup).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::rename(&backup, path);
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    let _ = fs::remove_file(backup);
    sync_parent_directory(path)
}

/// ProPresenter's application preferences on this computer, which hold its global layer
/// transitions. ProPresenter keeps them outside the workspace folder; only the Windows
/// location is known.
pub fn propresenter_app_preferences() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let path = PathBuf::from(std::env::var_os("APPDATA")?)
        .join("RenewedVision")
        .join("ProPresenter")
        .join("Preferences")
        .join("AppPreferences.proPref");
    path.is_file().then_some(path)
}

fn canonical_source(source: &Path) -> Result<PathBuf, String> {
    let source = source
        .canonicalize()
        .map_err(|error| format!("ProPresenter workspace could not be read: {error}"))?;
    if !source.is_dir() {
        return Err("ProPresenter workspace must be a folder.".to_string());
    }
    Ok(source)
}

/// Read-only: what importing `source` would create, for the selection dialog. Presentations
/// are counted as `.pro` files instead of decoded, which dominates the cost of a full read.
pub fn scan_pro_presenter_workspace(source: &Path) -> Result<Value, String> {
    let source = canonical_source(source)?;
    let libraries_root = source.join("Libraries");
    if !libraries_root.is_dir() {
        return Err("The ProPresenter workspace does not contain a Libraries folder.".to_string());
    }
    let mut folders = fs::read_dir(&libraries_root)
        .map_err(|error| {
            format!(
                "Could not scan ProPresenter libraries folder '{}': {error}",
                libraries_root.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    folders.sort_by_key(|entry| entry.file_name());
    let mut libraries = Vec::new();
    for entry in folders {
        if !entry.path().is_dir() || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let mut files = Vec::new();
        collect_files_with_extension(&entry.path(), &mut files, &["pro"])?;
        files.sort();
        let presentations = files
            .iter()
            .map(|file| {
                json!({
                    "id": presentation_id(&libraries_root, file),
                    "name": file.file_stem().map(|name| name.to_string_lossy()),
                })
            })
            .collect::<Vec<_>>();
        libraries.push(json!({
            "title": entry.file_name().to_string_lossy(),
            "presentations": presentations,
        }));
    }
    if libraries.is_empty() {
        return Err("The ProPresenter workspace contains no library folders.".to_string());
    }
    let categories = crate::workspace_import_selection::CATEGORIES;
    let selection = ImportSelection::parse(&json!({ "libraries": [], "categories": categories }))?;
    let content =
        read_pro_presenter_content(&source, None, json!({}), &json!({}), None, true, &selection)?;
    let mut inventory = crate::workspace_import_selection::inventory(&content);
    inventory["libraries"] = Value::Array(libraries);
    Ok(inventory)
}

/// Imports a ProPresenter workspace; unmanaged media keep referencing their original files.
pub fn import_pro_presenter_workspace_media(
    source: &Path,
    destination: &Path,
    current_group_settings: &Value,
    preferences: Option<&Path>,
    manage_media: bool,
    selection: &ImportSelection,
) -> Result<Value, String> {
    let source = canonical_source(source)?;
    crate::workspace_import::validate_destination(&source, destination)?;
    let existing_group_settings = if destination.exists() {
        read_workspace(destination)?
            .get("groupSettings")
            .cloned()
            .unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };
    let mut imported_content = read_pro_presenter_content(
        &source,
        Some(destination),
        existing_group_settings,
        current_group_settings,
        preferences,
        manage_media,
        selection,
    )?;
    screen_preview_import::resolve_sources(&mut imported_content)?;
    let import_destination = crate::workspace_import::Destination::reserve(destination)?;
    ensure_workspace(destination)?;
    let imported = propresenter_assets::save(destination, &imported_content)?;
    import_destination.commit();
    Ok(imported)
}

/// Decodes the selected parts of `source` into one workspace patch. Without a destination
/// (the scan), sample themes are not loaded.
fn read_pro_presenter_content(
    source: &Path,
    destination: Option<&Path>,
    existing_group_settings: Value,
    current_group_settings: &Value,
    preferences: Option<&Path>,
    manage_media: bool,
    selection: &ImportSelection,
) -> Result<Value, String> {
    let libraries_root = source.join("Libraries");
    if !libraries_root.is_dir() {
        return Err("The ProPresenter workspace does not contain a Libraries folder.".to_string());
    }
    let mut libraries = Vec::new();
    let mut items = Vec::new();
    let mut import_stats = ImportStats::default();
    let mut known_presentations = HashSet::new();
    let mut library_entries = fs::read_dir(&libraries_root)
        .map_err(|error| {
            format!(
                "Could not scan ProPresenter libraries folder '{}': {error}",
                libraries_root.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    library_entries.sort_by_key(|entry| entry.file_name());
    for entry in library_entries {
        if !entry.path().is_dir() || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let title = entry.file_name().to_string_lossy().to_string();
        if !selection.wants_library(&title) {
            continue;
        }
        let mut item_ids = Vec::new();
        collect_presentations(
            &entry.path(),
            &libraries_root,
            selection,
            &mut known_presentations,
            &mut items,
            &mut item_ids,
            &mut import_stats,
        )?;
        let id = unique_id(
            &safe_name(&title, "library"),
            libraries
                .iter()
                .filter_map(|value: &Value| value.get("id").and_then(Value::as_str)),
        );
        libraries.push(json!({ "id": id, "title": title, "itemIds": item_ids }));
    }
    if libraries.is_empty() && !selection.excludes_all_libraries() {
        return Err("The ProPresenter workspace contains no library folders.".to_string());
    }
    selection.check_presentations(&known_presentations)?;
    let mut playlist = if selection.wants("playlists") {
        let mut playlists = import_native_playlists(source, &items, &mut import_stats)?;
        let parents = playlist_parents(&playlists);
        selection.retain_items("playlists", &mut [&mut playlists])?;
        reparent_playlists(&mut playlists, &parents);
        playlists
    } else {
        Vec::new()
    };
    let (mut media_libraries, mut media) = if selection.wants("media") {
        import_native_media(source, &mut import_stats)?
    } else {
        (Vec::new(), Vec::new())
    };
    for item in media.iter_mut().filter_map(Value::as_object_mut) {
        crate::general_settings_options::apply(item);
    }
    if !manage_media {
        for item in media.iter_mut().filter_map(Value::as_object_mut) {
            item.insert("externalSource".into(), json!(true));
        }
    }
    if selection.wants("media") {
        let (smart_libraries, smart_media) =
            import_native_smart_folders(source, &mut import_stats)?;
        media_libraries.extend(smart_libraries);
        import_native_media_bin_settings(source, &mut media, &mut import_stats)?;
        media.extend(smart_media);
        import_native_media_bin_audio(source, &mut media, &mut import_stats);
        selection.retain_items("media", &mut [&mut media])?;
    }
    let (media_effect_presets, transition_presets) = if selection.wants("presets") {
        let mut effects = import_native_effect_presets(source, &mut import_stats)?;
        let mut transitions = import_native_transition_presets(source, &mut import_stats)?;
        selection.retain_items("presets", &mut [&mut effects, &mut transitions])?;
        (effects, transitions)
    } else {
        (Vec::new(), Vec::new())
    };
    let wants_settings = selection.wants("settings");
    let mut trigger_bindings = if wants_settings {
        import_native_key_mappings(source, &mut import_stats)?
    } else {
        Vec::new()
    };
    let communication_devices = if wants_settings {
        import_native_communications(source, &mut import_stats)?
    } else {
        Vec::new()
    };
    let mut themes = if selection.wants("themes") {
        let mut themes = import_native_themes(source, &mut import_stats)?;
        selection.retain_items("themes", &mut [&mut themes])?;
        themes
    } else {
        Vec::new()
    };
    let imported_theme_count = themes.len();
    let use_sample_themes = themes.is_empty() && selection.wants("themes") && destination.is_some();
    if let (true, Some(destination)) = (use_sample_themes, destination) {
        themes = crate::sample_content::themes(
            destination
                .parent()
                .ok_or("Workspace folder is required.")?,
        )?;
    }
    content_themes::import_layouts(&mut items, &mut themes)?;
    let mut countdown_timers = if selection.wants("timers") {
        let mut timers = import_native_timers(source, &mut import_stats)?;
        selection.retain_items("timers", &mut [&mut timers])?;
        timers
    } else {
        Vec::new()
    };
    if use_sample_themes {
        for timer in crate::sample_content::countdown_timers()? {
            if !countdown_timers
                .iter()
                .any(|existing| existing["id"] == timer["id"])
            {
                countdown_timers.push(timer);
            }
        }
    }
    let masks = if selection.wants("masks") {
        let mut masks = import_native_masks(source, &mut import_stats)?;
        selection.retain_items("masks", &mut [&mut masks])?;
        masks
    } else {
        Vec::new()
    };
    let stage_layouts = if selection.wants("stageLayouts") {
        let mut layouts = import_native_stage_layouts(source, &mut import_stats)?;
        selection.retain_items("stageLayouts", &mut [&mut layouts])?;
        layouts
    } else {
        Vec::new()
    };
    let workspace_config = if wants_settings {
        import_native_workspace_config(source, &mut import_stats)?
    } else {
        json!({})
    };
    let (props, props_transition) = if selection.wants("props") {
        let (mut props, transition) = import_native_props(source, &mut import_stats)?;
        selection.retain_items("props", &mut [&mut props])?;
        (props, transition)
    } else {
        (Vec::new(), None)
    };
    let mut layer_transitions = if wants_settings {
        import_native_layer_transitions(preferences, &mut import_stats)
    } else {
        Map::new()
    };
    if let Some(transition) = props_transition {
        layer_transitions.insert("props".to_string(), transition);
    }
    let groups = if selection.wants("groups") {
        let mut groups = import_native_groups(source, &mut import_stats)?;
        selection.retain_items("groups", &mut [&mut groups])?;
        groups
    } else {
        Vec::new()
    };
    let group_settings = if existing_group_settings
        .as_object()
        .is_some_and(|groups| !groups.is_empty())
    {
        existing_group_settings
    } else {
        settings::merge_imported_group_settings_value(current_group_settings, &groups, &items)
    };
    let messages = if selection.wants("messages") {
        let mut messages = import_native_messages(source, &themes, &mut import_stats)?;
        selection.retain_items("messages", &mut [&mut messages])?;
        messages
    } else {
        Vec::new()
    };
    let (mut macros, macro_collections) = if selection.wants("macros") {
        let (mut macros, collections) = import_native_macros(source, &mut import_stats)?;
        selection.retain_items("macros", &mut [&mut macros])?;
        (macros, collections)
    } else {
        (Vec::new(), Vec::new())
    };
    let clear_groups = if wants_settings {
        import_native_clear_groups(source, &mut import_stats)?
    } else {
        Vec::new()
    };
    for (reference, warning) in timeline_soundtrack_import::resolve(&mut items, &mut media) {
        import_stats.warn(Path::new(&reference), &warning);
    }
    resolve_imported_action_targets(
        &mut items,
        &mut macros,
        &mut playlist,
        &mut trigger_bindings,
        &props,
        workspace_config.get("screenConfig"),
    );
    let imported_content = json!({
        "title": source.file_name().and_then(|value| value.to_str()).unwrap_or("Imported Workspace"),
        "libraries": libraries,
        "library": items,
        "playlist": playlist,
        "mediaLibraries": media_libraries,
        "media": media,
        "clearGroups": clear_groups,
        "triggerBindings": trigger_bindings,
        "communicationDevices": communication_devices,
        "themes": themes,
        "countdownTimers": countdown_timers,
        "mediaEffectPresets": media_effect_presets,
        "transitionPresets": transition_presets,
        "masks": masks,
        "stageLayouts": stage_layouts,
        "screenConfig": workspace_config
            .get("screenConfig")
            .cloned()
            .unwrap_or_else(|| json!({})),
        "stageScreenAssignments": workspace_config
            .get("stageScreenAssignments")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "audienceLooks": workspace_config
            .get("audienceLooks")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "videoInputs": workspace_config
            .get("videoInputs")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "audioInputs": workspace_config
            .get("audioInputs")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "recordingPresets": workspace_config
            .get("recordingPresets")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "tickers": [],
        "layerTransitions": layer_transitions,
        "props": props,
        "groups": groups,
        "groupSettings": group_settings,
        "messages": messages,
        "macros": macros,
        "macroCollections": macro_collections,
        "stats": {
            "scannedFileCount": import_stats.scanned_file_count,
            "skippedFileCount": import_stats.skipped_file_count,
            "importedStageLayoutCount": stage_layouts.len(),
            "importedMaskCount": masks.len(),
            "importedThemeCount": imported_theme_count
        },
        "importWarnings": import_stats.warnings,
        "importedFrom": source
    });
    Ok(imported_content)
}

fn playlist_parents(playlists: &[Value]) -> HashMap<String, String> {
    playlists
        .iter()
        .filter_map(|playlist| {
            Some((
                playlist["id"].as_str()?.to_string(),
                playlist["parentId"].as_str()?.to_string(),
            ))
        })
        .collect()
}

/// A playlist whose folder was deselected moves up to the nearest folder that stayed.
fn reparent_playlists(playlists: &mut [Value], parents: &HashMap<String, String>) {
    let kept = playlists
        .iter()
        .filter_map(|playlist| playlist["id"].as_str().map(str::to_string))
        .collect::<HashSet<_>>();
    for playlist in playlists {
        let Some(mut parent) = playlist["parentId"].as_str().map(str::to_string) else {
            continue;
        };
        let mut steps = parents.len();
        while !parent.is_empty() && !kept.contains(&parent) && steps > 0 {
            parent = parents.get(&parent).cloned().unwrap_or_default();
            steps -= 1;
        }
        if playlist["parentId"] != json!(parent) {
            playlist["parentId"] = json!(parent);
        }
    }
}

/// Rewrite ProPresenter collection identities in imported actions to the
/// Orbit records created by this import. Unresolved identities stay as imported.
fn resolve_imported_action_targets(
    items: &mut [Value],
    macros: &mut [Value],
    playlists: &mut [Value],
    bindings: &mut [Value],
    props: &[Value],
    screen_config: Option<&Value>,
) {
    let prop_ids = props
        .iter()
        .filter_map(|prop| Some((prop["sourceId"].as_str()?, prop["id"].as_str()?)))
        .collect::<HashMap<_, _>>();
    let screen_ids = screen_config
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(id, config)| Some((config["sourceScreenId"].as_str()?, id.as_str())))
        .collect::<HashMap<_, _>>();
    let slide_actions = items
        .iter_mut()
        .filter_map(|item| item.get_mut("slides").and_then(Value::as_array_mut))
        .flatten()
        .filter_map(|slide| slide.get_mut("actions").and_then(Value::as_array_mut));
    let macro_actions = macros
        .iter_mut()
        .filter_map(|value| value.get_mut("actions").and_then(Value::as_array_mut));
    let header_actions = playlists
        .iter_mut()
        .filter_map(|playlist| playlist["items"].as_array_mut())
        .flatten()
        .filter_map(|entry| entry.get_mut("actions").and_then(Value::as_array_mut));
    for action in slide_actions
        .chain(macro_actions)
        .chain(header_actions)
        .flatten()
        .chain(bindings.iter_mut())
    {
        match action["type"].as_str() {
            Some("prop") => {
                if let Some(id) = action["targetId"]
                    .as_str()
                    .and_then(|target| prop_ids.get(target))
                {
                    action["targetId"] = json!(id);
                }
            }
            Some("stage") => {
                let Some(details) = action.get_mut("details").and_then(Value::as_object_mut) else {
                    continue;
                };
                let Some(Value::Array(imported)) = details.remove("stageScreenAssignments") else {
                    continue;
                };
                // "No Change" screens are encoded without a layout.
                let assignments = imported
                    .iter()
                    .filter_map(|entry| {
                        let screen = entry["screenId"].as_str()?;
                        let layout = entry["layoutId"].as_str().filter(|id| !id.is_empty())?;
                        Some(json!({
                            "screen": screen_ids.get(screen).copied().unwrap_or(screen),
                            "layout": layout
                        }))
                    })
                    .collect::<Vec<_>>();
                if let Some(layout) = assignments.first().map(|entry| entry["layout"].clone()) {
                    action["targetId"] = layout;
                }
                if let Some(details) = action.get_mut("details").and_then(Value::as_object_mut) {
                    details.insert("assignments".to_string(), Value::Array(assignments));
                }
            }
            _ => {}
        }
    }
}

/// The selection id of a presentation: its path below the libraries folder, with `/`.
fn presentation_id(libraries_root: &Path, file: &Path) -> String {
    file.strip_prefix(libraries_root)
        .unwrap_or(file)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Presentation files under `folder` in import order.
fn presentation_paths(folder: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = fs::read_dir(folder)
        .map_err(|error| {
            format!(
                "Could not scan ProPresenter folder '{}': {error}",
                folder.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read an entry in '{}': {error}", folder.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            presentation_paths(&path, paths)?;
            continue;
        }
        if !path.is_file()
            || path.file_name().and_then(|value| value.to_str()) == Some("library.json")
        {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(
            extension.as_str(),
            "png" | "jpg" | "jpeg" | "mp4" | "mov" | "mp3" | "wav"
        ) {
            paths.push(path);
        }
    }
    Ok(())
}

/// Maps `paths` on all cores; the results keep the order of `paths`.
fn parallel_map<T: Send>(paths: &[PathBuf], work: impl Fn(&Path) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let chunk = paths.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let work = &work;
        let handles = paths
            .chunks(chunk)
            .map(|paths| {
                scope.spawn(move || paths.iter().map(|path| work(path)).collect::<Vec<_>>())
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("presentation decoding does not panic"))
            .collect()
    })
}

fn collect_presentations(
    folder: &Path,
    libraries_root: &Path,
    selection: &ImportSelection,
    known: &mut HashSet<String>,
    items: &mut Vec<Value>,
    item_ids: &mut Vec<String>,
    stats: &mut ImportStats,
) -> Result<(), String> {
    let mut paths = Vec::new();
    presentation_paths(folder, &mut paths)?;
    paths.retain(|path| {
        let id = presentation_id(libraries_root, path);
        let wanted = selection.wants_presentation(&id);
        known.insert(id);
        wanted
    });
    let decoded = parallel_map(&paths, |path| {
        fs::read(path).ok().map(|bytes| {
            let item = propresenter_proto::decode_presentation(&bytes, path);
            (bytes, item)
        })
    });
    for (path, decoded) in paths.into_iter().zip(decoded) {
        stats.scanned();
        let Some((bytes, item)) = decoded else {
            stats.skipped(&path, "file could not be read");
            continue;
        };
        stats.source_version(&path, &bytes);
        let Some(mut item) = item else {
            // A library can contain non-presentation metadata files.  They are
            // intentionally skipped instead of being treated as slide text.
            stats.skipped(&path, "file is not a supported ProPresenter presentation");
            continue;
        };
        stats.findings(&path, &item);
        let fallback_title = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Presentation");
        let title = item
            .get("title")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback_title)
            .to_string();
        let id = unique_id(
            &safe_name(&title, "presentation"),
            items
                .iter()
                .filter_map(|value| value.get("id").and_then(Value::as_str)),
        );
        let Some(object) = item.as_object_mut() else {
            stats.skipped(&path, "decoded presentation has no object payload");
            continue;
        };
        object.insert("id".to_string(), json!(id));
        object.insert("title".to_string(), json!(title));
        let Some(slides) = object.get_mut("slides").and_then(Value::as_array_mut) else {
            stats.skipped(&path, "decoded presentation has no slides array");
            continue;
        };
        for (index, slide) in slides.iter_mut().enumerate() {
            if let Some(slide) = slide.as_object_mut() {
                slide.insert("id".to_string(), json!(format!("slide-{}", index + 1)));
            }
        }
        item_ids.push(id);
        items.push(item);
    }
    Ok(())
}

fn import_native_playlists(
    source: &Path,
    items: &[Value],
    stats: &mut ImportStats,
) -> Result<Vec<Value>, String> {
    let playlists_root = source.join("Playlists");
    if !playlists_root.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_files_with_extension(&playlists_root, &mut files, &["proplaylist", ""])?;
    files.sort();
    let mut imported: Vec<Value> = Vec::new();
    for file in files {
        // These native documents belong to the media bins, not presentation playlists.
        if file == playlists_root.join("Media") || file == playlists_root.join("Audio") {
            continue;
        }
        // Playlist templates are a ProPresenter-only feature; Orbit has no playlist templates.
        if file == playlists_root.join("Templates").join("PlaylistTemplates") {
            continue;
        }
        stats.scanned();
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(_) => {
                stats.skipped(&file, "file could not be read");
                continue;
            }
        };
        stats.source_version(&file, &bytes);
        let mut decoded_playlists = propresenter_proto::decode_playlists(&bytes);
        if decoded_playlists.is_empty() {
            stats.skipped(&file, "file is not a supported ProPresenter playlist");
        }
        let mut identities = HashMap::new();
        let mut used = imported
            .iter()
            .filter_map(|playlist| playlist["id"].as_str().map(str::to_string))
            .collect::<Vec<_>>();
        for playlist in &decoded_playlists {
            let source_id = playlist["id"]
                .as_str()
                .ok_or("Imported playlist identity is required.")?;
            if identities.contains_key(source_id) {
                return Err("ProPresenter playlist document contains duplicate identities.".into());
            }
            let id = unique_id(source_id, used.iter().map(String::as_str));
            used.push(id.clone());
            identities.insert(source_id.to_string(), id);
        }
        for playlist in &mut decoded_playlists {
            playlist["id"] = json!(identities[playlist["id"].as_str().unwrap()]);
            if let Some(parent) = playlist["parentId"]
                .as_str()
                .filter(|parent| !parent.is_empty())
            {
                playlist["parentId"] = json!(identities
                    .get(parent)
                    .ok_or("Imported playlist parent is missing.")?);
            }
        }
        for mut playlist in decoded_playlists {
            stats.findings(&file, &playlist);
            let Some(object) = playlist.as_object_mut() else {
                stats.skipped(&file, "decoded playlist has no object payload");
                continue;
            };
            let Some(entries) = object.get_mut("items").and_then(Value::as_array_mut) else {
                stats.skipped(&file, "decoded playlist has no items array");
                continue;
            };
            let mut matched = Vec::new();
            for mut entry in std::mem::take(entries) {
                let Some(entry_object) = entry.as_object_mut() else {
                    continue;
                };
                if entry_object.get("type").and_then(Value::as_str) == Some("cue") {
                    stats.warn(
                        &file,
                        "inline playlist cue is unsupported; retained as an unlinked placeholder",
                    );
                    entry_object.insert("type".into(), json!("placeholder"));
                }
                let entry_type = entry_object.get("type").and_then(Value::as_str);
                if matches!(entry_type, Some("presentation") | Some("placeholder")) {
                    let document_path = entry_object
                        .get("documentPath")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let Some(item_id) = crate::workspace_import::match_presentation(
                        &Value::Object(entry_object.clone()),
                        source,
                        items,
                    ) else {
                        if !document_path.is_empty()
                            || entry_object.contains_key("documentRelativePath")
                        {
                            stats.warn(&file, &format!("presentation reference could not be resolved; retained as an unlinked placeholder: {document_path}"));
                        }
                        entry_object.insert("type".into(), json!("placeholder"));
                        entry_object.remove("arrangementId");
                        entry_object.remove("documentRelativePath");
                        entry_object.remove("documentPath");
                        matched.push(entry);
                        continue;
                    };
                    if entry_type == Some("presentation") {
                        entry_object.insert("itemId".to_string(), json!(item_id));
                    } else {
                        entry_object.insert("linkedItemId".to_string(), json!(item_id));
                    }
                    entry_object.remove("documentPath");
                    entry_object.remove("documentRelativePath");
                }
                matched.push(entry);
            }
            *entries = matched;
            let item_ids = entries
                .iter()
                .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("presentation"))
                .filter_map(|entry| {
                    entry
                        .get("itemId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>();
            let _ = entries;
            if !item_ids.is_empty() {
                object.insert("itemIds".to_string(), json!(item_ids));
            }
            let base_id = object
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| safe_name(&file.to_string_lossy(), "playlist"));
            let id = unique_id(
                &base_id,
                imported
                    .iter()
                    .filter_map(|value| value.get("id").and_then(Value::as_str)),
            );
            object.insert("id".to_string(), json!(id));
            imported.push(playlist);
        }
    }
    Ok(imported)
}

/// ProPresenter stores a configuration document either as a single file or as a folder of files.
fn configuration_files(root: &Path, extensions: &[&str]) -> Result<Vec<PathBuf>, String> {
    if root.is_file() {
        return Ok(vec![root.to_path_buf()]);
    }
    let mut files = Vec::new();
    if root.is_dir() {
        collect_files_with_extension(root, &mut files, extensions)?;
        files.sort();
    }
    Ok(files)
}

fn collect_files_with_extension(
    folder: &Path,
    files: &mut Vec<PathBuf>,
    extensions: &[&str],
) -> Result<(), String> {
    for entry in fs::read_dir(folder).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_extension(&path, files, extensions)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if extensions.iter().any(|candidate| *candidate == extension) {
            files.push(path);
        }
    }
    Ok(())
}

fn import_native_themes(source: &Path, stats: &mut ImportStats) -> Result<Vec<Value>, String> {
    let root = source.join("Themes");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_files_with_extension(&root, &mut files, &["", "protheme"])?;
    files.sort();
    let mut themes: Vec<Value> = Vec::new();
    for file in files {
        stats.scanned();
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(_) => {
                stats.skipped(&file, "file could not be read");
                continue;
            }
        };
        let fallback_title = file
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.eq_ignore_ascii_case("theme"))
            .map(str::to_string)
            .or_else(|| {
                file.parent()
                    .and_then(Path::file_name)
                    .and_then(|value| value.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Theme".to_string());
        stats.source_version(&file, &bytes);
        let Some(mut theme) = propresenter_proto::decode_template(&bytes, &file, &fallback_title)
        else {
            stats.skipped(&file, "file is not a supported ProPresenter theme");
            continue;
        };
        stats.findings(&file, &theme);
        let id = unique_id(
            &safe_name(&fallback_title, "theme"),
            themes
                .iter()
                .filter_map(|value| value.get("id").and_then(Value::as_str)),
        );
        if let Some(object) = theme.as_object_mut() {
            object.insert("id".to_string(), json!(id));
        }
        themes.push(theme);
    }
    Ok(themes)
}

fn import_native_timers(source: &Path, stats: &mut ImportStats) -> Result<Vec<Value>, String> {
    let root = source.join("Configuration").join("Timers");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = if root.is_file() {
        vec![root.clone()]
    } else {
        Vec::new()
    };
    if files.is_empty() {
        collect_files_with_extension(&root, &mut files, &["", "timers"])?;
    }
    let mut timers = Vec::new();
    for file in files {
        stats.scanned();
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(_) => {
                stats.skipped(&file, "file could not be read");
                continue;
            }
        };
        if let Some(values) = propresenter_proto::decode_timers(&bytes) {
            timers.extend(values);
        } else {
            stats.skipped(&file, "file is not a supported ProPresenter timer document");
        }
    }
    let mut ids = HashSet::new();
    Ok(timers
        .into_iter()
        .filter(|timer| {
            timer
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| ids.insert(id.to_string()))
        })
        .collect())
}

/// Applies the effects and transport of ProPresenter's media bin cues (`Playlists/Media`)
/// to the imported assets with the same file.
fn import_native_media_bin_settings(
    source: &Path,
    media: &mut Vec<Value>,
    stats: &mut ImportStats,
) -> Result<(), String> {
    let file = source.join("Playlists").join("Media");
    if !file.is_file() {
        return Ok(());
    }
    stats.scanned();
    let Ok(bytes) = fs::read(&file) else {
        stats.skipped(&file, "file could not be read");
        return Ok(());
    };
    let mut applied = HashSet::new();
    for (path, chain, unsupported) in propresenter_proto::decode_media_bin_effects(&bytes) {
        let Some(asset) = media_bin_asset(source, media, &path, stats, &file) else {
            stats.findings(&file, &json!({ "unsupportedMediaEffectSource": path }));
            continue;
        };
        if !unsupported.is_empty() {
            stats.findings(&file, &json!({ "unsupportedMediaEffects": unsupported }));
        }
        if chain.is_empty() {
            continue;
        }
        let id = asset["id"].as_str().unwrap_or_default().to_string();
        if !applied.insert(id) {
            stats.findings(&file, &json!({ "duplicateMediaEffectSource": path }));
            continue;
        }
        asset["mediaEffects"] = Value::Array(chain);
    }
    for (path, seconds) in propresenter_proto::decode_media_bin_thumbnails(&bytes) {
        match media_bin_asset(source, media, &path, stats, &file) {
            Some(asset) if asset["type"] == "video" => {
                asset["thumbnailTimeSeconds"] = json!(seconds)
            }
            _ => stats.findings(&file, &json!({ "unsupportedMediaThumbnailSource": path })),
        }
    }
    let mut applied: HashMap<String, Vec<Value>> = HashMap::new();
    for (path, settings) in propresenter_proto::decode_media_bin_transports(&bytes) {
        let settings = settings
            .map_err(|error| format!("Invalid ProPresenter media transport for {path}: {error}"))?;
        let Some(asset) = media_bin_asset(source, media, &path, stats, &file) else {
            stats.findings(&file, &json!({"unsupportedMediaTransportSource":path}));
            continue;
        };
        if asset.get("type").and_then(Value::as_str) != Some("video") {
            return Err(format!(
                "ProPresenter video transport requires a video asset: {path}"
            ));
        }
        let id = asset["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Imported media asset id is required.")?
            .to_string();
        let previous = applied.entry(id).or_default();
        if previous.contains(&settings) {
            continue;
        }
        let duplicate = !previous.is_empty();
        previous.push(settings.clone());
        if duplicate {
            let mut cue = asset.clone();
            cue["id"] = json!(Uuid::new_v4().to_string());
            cue["mediaTransport"] = settings["mediaTransport"].clone();
            cue["backgroundBehavior"] = settings["backgroundBehavior"].clone();
            cue["backgroundPlayback"] = settings["backgroundPlayback"].clone();
            media.push(cue);
        } else {
            asset["mediaTransport"] = settings["mediaTransport"].clone();
            asset["backgroundBehavior"] = settings["backgroundBehavior"].clone();
            asset["backgroundPlayback"] = settings["backgroundPlayback"].clone();
        }
    }
    Ok(())
}

/// The asset of a media bin cue. A cue whose file is not in the folder keeps its settings on an
/// asset listed as missing media, so they apply once the file is relinked.
fn media_bin_asset<'a>(
    source: &Path,
    media: &'a mut Vec<Value>,
    path: &str,
    stats: &mut ImportStats,
    file: &Path,
) -> Option<&'a mut Value> {
    if let Some(index) = imported_media_bin_index(source, media, path) {
        return media.get_mut(index);
    }
    let extension = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    let media_type = propresenter_media_type(&extension)?;
    let mut absolute = source.to_path_buf();
    absolute.extend(path.split(['/', '\\']).filter(|part| !part.is_empty()));
    let url = file_url(&absolute).ok()?;
    let mut item = Map::new();
    item.insert(
        "id".into(),
        json!(format!("media-{}-{}", media_slug(path), media.len() + 1)),
    );
    item.insert(
        "title".into(),
        json!(absolute.file_stem()?.to_str().unwrap_or("Media")),
    );
    item.insert("sourcePath".into(), json!(absolute));
    item.insert("sourceUrl".into(), json!(&url));
    item.insert("sourceId".into(), json!(url));
    item.insert("type".into(), json!(media_type));
    item.insert("externalSource".into(), json!(true));
    crate::general_settings_options::apply(&mut item);
    media.push(Value::Object(item));
    stats.warn(
        file,
        &format!("media {path} is unavailable on this system; the original reference was retained"),
    );
    media.last_mut()
}

fn imported_media_bin_index(source: &Path, media: &[Value], path: &str) -> Option<usize> {
    let normalize = |path: &str| path.replace('\\', "/").to_lowercase();
    let key = normalize(path);
    media.iter().position(|asset| {
        let Some(path) = asset.get("sourcePath").and_then(Value::as_str) else {
            return false;
        };
        let relative = Path::new(path)
            .strip_prefix(source)
            .map(|relative| normalize(&relative.to_string_lossy()));
        relative.is_ok_and(|relative| relative == key) || normalize(path) == key
    })
}

/// Imports `Presets/Effect` presets; each file holds one `Effect.Preset`.
fn import_native_effect_presets(
    source: &Path,
    stats: &mut ImportStats,
) -> Result<Vec<Value>, String> {
    let root = source.join("Presets").join("Effect");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_files_with_extension(&root, &mut files, &["", "effectpreset"])?;
    files.sort();
    let mut presets: Vec<Value> = Vec::new();
    for file in files {
        stats.scanned();
        let Ok(bytes) = fs::read(&file) else {
            stats.skipped(&file, "file could not be read");
            continue;
        };
        let mut unsupported = Vec::new();
        let Some(preset) = propresenter_proto::decode_effect_preset(&bytes, &mut unsupported)
        else {
            stats.skipped(&file, "file is not a ProPresenter effect preset");
            continue;
        };
        if !unsupported.is_empty() {
            stats.findings(&file, &json!({ "unsupportedMediaEffects": unsupported }));
        }
        let duplicate = presets.iter().any(|existing| {
            existing["id"] == preset["id"]
                || existing["title"].as_str().map(str::to_lowercase)
                    == preset["title"].as_str().map(str::to_lowercase)
        });
        if duplicate {
            stats.skipped(&file, "effect preset id or title is already imported");
            continue;
        }
        presets.push(preset);
    }
    Ok(presets)
}

/// Imports `Presets/Transition` favorites; each file holds one `Transition.Preset`.
fn import_native_transition_presets(
    source: &Path,
    stats: &mut ImportStats,
) -> Result<Vec<Value>, String> {
    let root = source.join("Presets").join("Transition");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_files_with_extension(&root, &mut files, &["", "transitionpreset"])?;
    files.sort();
    let mut presets: Vec<Value> = Vec::new();
    for file in files {
        stats.scanned();
        let Ok(bytes) = fs::read(&file) else {
            stats.skipped(&file, "file could not be read");
            continue;
        };
        let Some(mut preset) = propresenter_proto::decode_transition_preset(&bytes) else {
            stats.skipped(
                &file,
                "file is not a ProPresenter transition preset of a built-in library effect",
            );
            continue;
        };
        if let Some(names) = preset
            .as_object_mut()
            .and_then(|object| object.remove("unsupportedTransitionVariables"))
        {
            stats.findings(&file, &json!({ "unsupportedTransitionVariables": names }));
        }
        let duplicate = presets.iter().any(|existing| {
            existing["id"] == preset["id"]
                || existing["name"].as_str().map(str::to_lowercase)
                    == preset["name"].as_str().map(str::to_lowercase)
        });
        if duplicate {
            stats.skipped(&file, "transition preset id or name is already imported");
            continue;
        }
        presets.push(preset);
    }
    Ok(presets)
}

fn import_native_masks(source: &Path, stats: &mut ImportStats) -> Result<Vec<Value>, String> {
    let candidates = [
        source.join("Workspace"),
        source.join("Configuration").join("Workspace"),
        source.join("Masks"),
        source.join("Configuration").join("Masks"),
    ];
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        stats.scanned();
        let bytes = match fs::read(&candidate) {
            Ok(bytes) => bytes,
            Err(_) => {
                stats.skipped(&candidate, "file could not be read");
                continue;
            }
        };
        if let Some(mut masks) = propresenter_proto::decode_masks(&bytes, &candidate) {
            normalize_imported_canvas_documents(&mut masks)?;
            for value in &masks {
                stats.findings(&candidate, value);
            }
            return Ok(masks);
        }
        stats.skipped(
            &candidate,
            "file is not a supported ProPresenter mask document",
        );
    }
    Ok(Vec::new())
}

fn import_native_stage_layouts(
    source: &Path,
    stats: &mut ImportStats,
) -> Result<Vec<Value>, String> {
    let candidates = [
        source.join("Stage"),
        source.join("Configuration").join("Stage"),
    ];
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        stats.scanned();
        let bytes = match fs::read(&candidate) {
            Ok(bytes) => bytes,
            Err(_) => {
                stats.skipped(&candidate, "file could not be read");
                continue;
            }
        };
        stats.source_version(&candidate, &bytes);
        if let Some(mut layouts) = propresenter_proto::decode_stage(&bytes, &candidate) {
            normalize_imported_canvas_documents(&mut layouts)?;
            for value in &layouts {
                stats.findings(&candidate, value);
            }
            return Ok(layouts);
        }
        stats.skipped(
            &candidate,
            "file is not a supported ProPresenter stage document",
        );
    }
    Ok(Vec::new())
}

// ProPresenter's embedded Stage and mask slides need an Orbit editing ID and a layer list.
fn normalize_imported_canvas_documents(layouts: &mut [Value]) -> Result<(), String> {
    for layout in layouts
        .iter_mut()
        .filter(|layout| layout["sourceType"] == "propresenter")
    {
        let id = layout["id"]
            .as_str()
            .ok_or("Imported canvas document ID is required.")?;
        let slide_id = format!("{id}-slide");
        let slide = layout["slide"]
            .as_object_mut()
            .ok_or("Imported canvas document slide must be an object.")?;
        slide.entry("id").or_insert_with(|| json!(slide_id));
        slide.entry("layers").or_insert_with(|| json!([]));
        slide.entry("width").or_insert_with(|| json!(1920));
        slide.entry("height").or_insert_with(|| json!(1080));
    }
    Ok(())
}

fn import_native_workspace_config(source: &Path, stats: &mut ImportStats) -> Result<Value, String> {
    let candidates = [
        source.join("Workspace"),
        source.join("Configuration").join("Workspace"),
    ];
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        stats.scanned();
        let bytes = match fs::read(&candidate) {
            Ok(bytes) => bytes,
            Err(_) => {
                stats.skipped(&candidate, "file could not be read");
                continue;
            }
        };
        if let Some(config) = propresenter_proto::decode_workspace_config(&bytes)? {
            return Ok(config);
        }
        stats.skipped(
            &candidate,
            "file is not a supported ProPresenter workspace document",
        );
    }
    Ok(json!({
        "screenConfig": {},
        "stageScreenAssignments": [],
        "audienceLooks": [],
        "videoInputs": [],
        "audioInputs": [],
        "recordingPresets": []
    }))
}

/// The background, foreground and message layer defaults from ProPresenter's preferences.
fn import_native_layer_transitions(
    preferences: Option<&Path>,
    stats: &mut ImportStats,
) -> Map<String, Value> {
    let Some(path) = preferences else {
        return Map::new();
    };
    stats.scanned();
    let Ok(text) = fs::read_to_string(path) else {
        stats.skipped(
            path,
            "ProPresenter preferences could not be read; layer transitions were not imported",
        );
        return Map::new();
    };
    let (layers, unsupported) = propresenter_proto::decode_layer_transition_preferences(&text);
    for finding in unsupported {
        stats.warn(
            path,
            &format!(
                "{finding} is not in the built-in library Orbit supports; it was not imported"
            ),
        );
    }
    layers
}

/// The props and the props layer transition of `Configuration/Props`.
fn import_native_props(
    source: &Path,
    stats: &mut ImportStats,
) -> Result<(Vec<Value>, Option<Value>), String> {
    let root = source.join("Configuration").join("Props");
    if !root.exists() {
        return Ok((Vec::new(), None));
    }
    let relative_root = if root.is_file() {
        root.parent().unwrap_or(&root)
    } else {
        &root
    };
    let mut files = if root.is_file() {
        vec![root.clone()]
    } else {
        Vec::new()
    };
    if files.is_empty() {
        collect_files_with_extension(&root, &mut files, &["", "props"])?;
    }
    let mut props: Vec<Value> = Vec::new();
    let mut layer_transition = None;
    for file in files {
        stats.scanned();
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(_) => {
                stats.skipped(&file, "file could not be read");
                continue;
            }
        };
        stats.source_version(&file, &bytes);
        let has_transition = propresenter_proto::decode_props_transition(&bytes);
        let transition_only = has_transition.is_some();
        if let Some(mut found) = has_transition {
            stats.findings(&file, &Value::Object(found.clone()));
            if let Some(transition) = found.remove("transition") {
                if layer_transition
                    .as_ref()
                    .is_some_and(|current| current != &transition)
                {
                    stats.warn(&file, "props documents define different layer transitions; the first was imported");
                } else {
                    layer_transition = Some(transition);
                }
            }
        }
        if let Some(values) = propresenter_proto::decode_props(&bytes, &file) {
            for value in &values {
                stats.findings(&file, value);
            }
            let relative = file.strip_prefix(relative_root).unwrap_or(&file);
            let folder = relative
                .parent()
                .filter(|value| !value.as_os_str().is_empty())
                .map(|value| value.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            for (index, mut prop) in values.into_iter().enumerate() {
                let Some(object) = prop.as_object_mut() else {
                    continue;
                };
                object.insert(
                    "id".to_string(),
                    json!(unique_id(
                        &format!(
                            "prop-{}-{}",
                            safe_name(&relative.to_string_lossy(), "prop"),
                            index + 1
                        ),
                        props
                            .iter()
                            .filter_map(|value| value.get("id").and_then(Value::as_str)),
                    )),
                );
                if object
                    .get("folder")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                    && !folder.is_empty()
                {
                    object.insert("folder".to_string(), json!(&folder));
                }
                props.push(prop);
            }
        } else if !transition_only {
            stats.skipped(&file, "file is not a supported ProPresenter props document");
        }
    }
    Ok((props, layer_transition))
}

fn import_native_groups(source: &Path, stats: &mut ImportStats) -> Result<Vec<Value>, String> {
    let candidates = [
        source.join("Configuration").join("Groups"),
        source.join("Groups"),
    ];
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        stats.scanned();
        let bytes = match fs::read(&candidate) {
            Ok(bytes) => bytes,
            Err(_) => {
                stats.skipped(&candidate, "file could not be read");
                continue;
            }
        };
        if let Some(groups) = propresenter_proto::decode_groups(&bytes) {
            return Ok(groups);
        }
        stats.skipped(
            &candidate,
            "file is not a supported ProPresenter groups document",
        );
    }
    Ok(Vec::new())
}

fn import_native_messages(
    source: &Path,
    themes: &[Value],
    stats: &mut ImportStats,
) -> Result<Vec<Value>, String> {
    let files = configuration_files(
        &source.join("Configuration").join("Messages"),
        &["", "messages"],
    )?;
    let mut messages = Vec::new();
    for file in files {
        stats.scanned();
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(_) => {
                stats.skipped(&file, "file could not be read");
                continue;
            }
        };
        if let Some(values) = propresenter_proto::decode_messages(&bytes) {
            for mut message in values {
                resolve_message_template(&mut message, themes, &file, stats);
                messages.push(message);
            }
        } else {
            stats.skipped(
                &file,
                "file is not a supported ProPresenter messages document",
            );
        }
    }
    Ok(messages)
}

/// ProPresenter messages name their template by theme slide UUID; imported theme slides keep
/// that UUID as `sourceSlideId`.
fn resolve_message_template(
    message: &mut Value,
    themes: &[Value],
    file: &Path,
    stats: &mut ImportStats,
) {
    let Some(slide_id) = message.get("templateSlideId").and_then(Value::as_str) else {
        return;
    };
    let resolved = themes.iter().find_map(|theme| {
        let slide =
            theme.get("slides")?.as_array()?.iter().find(|slide| {
                slide.get("sourceSlideId").and_then(Value::as_str) == Some(slide_id)
            })?;
        Some((theme.get("id")?.clone(), slide.get("id")?.clone()))
    });
    match resolved {
        Some((theme_id, theme_slide_id)) => {
            message["themeId"] = theme_id;
            message["themeSlideId"] = theme_slide_id;
        }
        None => {
            stats.warn(
                file,
                &format!(
                "message {} uses template slide {slide_id}, which is not an imported theme slide",
                message.get("title").and_then(Value::as_str).unwrap_or_default()
            ),
            )
        }
    }
}

fn import_native_macros(
    source: &Path,
    stats: &mut ImportStats,
) -> Result<(Vec<Value>, Vec<Value>), String> {
    let files = configuration_files(
        &source.join("Configuration").join("Macros"),
        &["", "macros"],
    )?;
    let mut macros = Vec::new();
    let mut collections = Vec::new();
    for file in files {
        stats.scanned();
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(_) => {
                stats.skipped(&file, "file could not be read");
                continue;
            }
        };
        if let Some(values) = propresenter_proto::decode_macros(&bytes) {
            for value in &values {
                stats.findings(&file, value);
            }
            macros.extend(values);
            let decoded_collections = propresenter_proto::decode_macro_collections(&bytes)
                .ok_or_else(|| format!("{}: invalid macro collections document", file.display()))?;
            collections.extend(decoded_collections);
        } else if let Some(values) =
            propresenter_proto::decode_macro_collections(&bytes).filter(|values| !values.is_empty())
        {
            collections.extend(values);
        } else {
            stats.skipped(
                &file,
                "file is not a supported ProPresenter macros document",
            );
        }
    }
    Ok((macros, collections))
}

fn import_native_clear_groups(
    source: &Path,
    stats: &mut ImportStats,
) -> Result<Vec<Value>, String> {
    let file = source.join("Configuration").join("ClearGroups");
    if !file.is_file() {
        return Ok(Vec::new());
    }
    stats.scanned();
    let Ok(bytes) = fs::read(&file) else {
        stats.skipped(&file, "file could not be read");
        return Ok(Vec::new());
    };
    let Some((groups, findings)) = propresenter_proto::decode_clear_groups(&bytes) else {
        stats.skipped(
            &file,
            "file is not a supported ProPresenter clear groups document",
        );
        return Ok(Vec::new());
    };
    for finding in findings {
        stats.warn(&file, &finding);
    }
    Ok(groups)
}

fn import_native_media(
    source: &Path,
    stats: &mut ImportStats,
) -> Result<(Vec<Value>, Vec<Value>), String> {
    let root = source.join("Media");
    if !root.is_dir() {
        return Ok((Vec::new(), Vec::new()));
    }

    let mut files = Vec::new();
    collect_native_media_paths(&root, &mut files)?;

    let media = files
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let media_type = propresenter_media_type(&extension)?;
            stats.scanned();
            let relative = path.strip_prefix(&root).unwrap_or(path).to_string_lossy();
            let title = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("Media");
            let source_url = match file_url(path) {
                Ok(value) => value,
                Err(_) => {
                    stats.skipped(path, "media path could not be converted to a URL");
                    return None;
                }
            };
            let mut item = Map::new();
            item.insert(
                "id".to_string(),
                json!(format!("media-{}-{}", media_slug(&relative), index + 1)),
            );
            item.insert("title".to_string(), json!(title));
            item.insert("sourcePath".to_string(), json!(path));
            item.insert("sourceUrl".to_string(), json!(&source_url));
            item.insert("sourceId".to_string(), json!(source_url));
            item.insert("type".to_string(), json!(media_type));
            Some(Value::Object(item))
        })
        .collect::<Vec<_>>();

    Ok((Vec::new(), media))
}

/// ProPresenter's smart media playlists become smart libraries linked to the same folder.
/// The files of the playlist's last folder sync are the library's known files, so files
/// that have since disappeared stay identified as missing media and can be relinked.
fn import_native_smart_folders(
    source: &Path,
    stats: &mut ImportStats,
) -> Result<(Vec<Value>, Vec<Value>), String> {
    let file = source.join("Playlists").join("Media");
    let Ok(bytes) = fs::read(&file) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut libraries: Vec<Value> = Vec::new();
    let mut media = Vec::new();
    for folder in propresenter_proto::decode_media_smart_folders(&bytes) {
        let title = folder["title"].as_str().unwrap_or_default();
        let stored = PathBuf::from(folder["sourcePath"].as_str().unwrap_or_default());
        if !stored.is_absolute() {
            stats.warn(
                &file,
                &format!("smart playlist {title} has no absolute folder; it was not imported"),
            );
            continue;
        }
        let root = if stored.is_dir() {
            stored.canonicalize().map_err(|error| error.to_string())?
        } else {
            stats.warn(&file, &format!("smart playlist {title}: folder {} is missing; its files are listed as missing media", stored.display()));
            stored.clone()
        };
        let id = format!("smart-{}", safe_name(&root.to_string_lossy(), "playlist"));
        if libraries.iter().any(|library| library["id"] == json!(id)) {
            stats.warn(
                &file,
                &format!("smart playlist {title} links a folder that is already imported"),
            );
            continue;
        }
        let mut known = Vec::new();
        let mut missing = 0;
        for item in folder["items"].as_array().into_iter().flatten() {
            let synced = Path::new(item["sourcePath"].as_str().unwrap_or_default());
            let Ok(relative) = synced.strip_prefix(&stored) else {
                stats.warn(
                    &file,
                    &format!(
                        "smart playlist {title}: {} lies outside its folder; it was not imported",
                        synced.display()
                    ),
                );
                continue;
            };
            let path = root.join(relative);
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let media_type = media_type_for_extension(&extension);
            if media_type == "media" {
                stats.warn(
                    &file,
                    &format!(
                        "smart playlist {title}: {} is not a media file Orbit plays",
                        path.display()
                    ),
                );
                continue;
            }
            let mut entry = smart_media_entry(&path, &id, media_type)?;
            let object = entry
                .as_object_mut()
                .ok_or("Smart media entry must be an object.")?;
            object.insert("sourceId".into(), item["sourceId"].clone());
            object.insert(
                "backgroundBehavior".into(),
                folder["backgroundBehavior"].clone(),
            );
            if let Some(chain) = item.get("mediaEffects") {
                object.insert("mediaEffects".into(), chain.clone());
            }
            if let Some(effects) = item.get("unsupportedMediaEffects") {
                stats.findings(&file, &json!({ "unsupportedMediaEffects": effects }));
            }
            if !path.is_file() {
                object.insert("missing".into(), json!(true));
                missing += 1;
            }
            known.push(json!(path));
            media.push(entry);
        }
        if missing > 0 {
            stats.warn(&file, &format!("smart playlist {title}: {missing} synced files are missing; they are listed in Missing media"));
        }
        libraries.push(json!({
            "id": id,
            "title": title,
            "type": "smart",
            "sourcePath": root,
            "sourceId": folder["sourceId"],
            "backgroundBehavior": folder["backgroundBehavior"],
            "knownSources": known,
        }));
    }
    Ok((libraries, media))
}

/// Applies the per-asset audio routes and audio looping of ProPresenter's media and audio
/// bin cues to the imported assets with the same file.
fn import_native_media_bin_audio(source: &Path, media: &mut [Value], stats: &mut ImportStats) {
    // Imported assets carry canonical (verbatim) paths; ProPresenter writes plain ones.
    let normalize = |path: &str| {
        let path = path.replace('\\', "/").to_lowercase();
        path.strip_prefix("//?/")
            .map(str::to_string)
            .unwrap_or(path)
    };
    let mut applied: HashMap<(String, String), Value> = HashMap::new();
    for file in [
        source.join("Playlists").join("Media"),
        source.join("Playlists").join("Audio"),
    ] {
        let Ok(bytes) = fs::read(&file) else {
            continue;
        };
        for (path, settings, unsupported) in propresenter_proto::decode_media_bin_audio(&bytes) {
            if !unsupported.is_empty() {
                stats.warn(
                    &file,
                    &format!("{path}: {} are not imported", unsupported.join(", ")),
                );
            }
            if settings.is_empty() {
                continue;
            }
            let key = normalize(&path);
            let Some(asset) = media.iter_mut().find(|asset| {
                let asset_path = asset
                    .get("sourcePath")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let relative = Path::new(asset_path)
                    .strip_prefix(source)
                    .map(|relative| normalize(&relative.to_string_lossy()))
                    .unwrap_or_default();
                relative == key || normalize(asset_path) == key
            }) else {
                stats.warn(
                    &file,
                    &format!("audio settings of {path} have no imported asset"),
                );
                continue;
            };
            let kind = asset
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let id = asset["id"].as_str().unwrap_or_default().to_string();
            for (field, value) in settings {
                // Looping applies to audio assets; routes to audio and video.
                if !(kind == "audio" || (kind == "video" && field == "audioRouting")) {
                    continue;
                }
                match applied.get(&(id.clone(), field.clone())) {
                    Some(previous) if previous != &value => {
                        stats.warn(&file, &format!("{path} has different {field} settings in ProPresenter; the first was imported"));
                    }
                    Some(_) => {}
                    None => {
                        asset[field.as_str()] = value.clone();
                        applied.insert((id.clone(), field), value);
                    }
                }
            }
        }
    }
}

fn collect_native_media_paths(folder: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(folder)
        .map_err(|error| {
            format!(
                "Could not scan ProPresenter media folder '{}': {error}",
                folder.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read an entry in '{}': {error}", folder.display()))?;
    for entry in entries {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_native_media_paths(&path, files)?;
        } else if path.is_file()
            && path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| propresenter_media_type(&value.to_ascii_lowercase()).is_some())
                .unwrap_or(false)
        {
            files.push(path);
        }
    }
    Ok(())
}

fn propresenter_media_type(extension: &str) -> Option<&'static str> {
    match extension {
        "aac" | "aiff" | "m4a" | "mp3" | "wav" => Some("audio"),
        "bmp" | "gif" | "jpeg" | "jpg" | "png" | "tif" | "tiff" | "webp" => Some("image"),
        "mov" | "mp4" => Some("video"),
        _ => None,
    }
}

fn media_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            separator = false;
        } else if !separator && !slug.is_empty() {
            slug.push('-');
            separator = true;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "item".to_string()
    } else {
        slug.to_string()
    }
}

fn import_native_key_mappings(
    source: &Path,
    stats: &mut ImportStats,
) -> Result<Vec<Value>, String> {
    let root = [
        source.join("Configuration").join("KeyMappings"),
        source.join("Configuration").join("Key Mappings"),
        source.join("KeyMappings"),
        source.join("Key Mappings"),
    ]
    .into_iter()
    .find(|candidate| candidate.exists());
    let Some(root) = root else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    if root.is_file() {
        files.push(root);
    } else {
        collect_all_files(&root, &mut files)?;
    }
    files.sort();

    let mut bindings = HashMap::new();
    for file in files {
        stats.scanned();
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(_) => {
                stats.skipped(&file, "key mappings file could not be read");
                continue;
            }
        };
        let Some(values) = propresenter_proto::decode_key_mappings(&bytes) else {
            stats.skipped(
                &file,
                "file is not a supported ProPresenter key mappings document",
            );
            continue;
        };
        for value in values {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                bindings.insert(id.to_string(), value);
            }
        }
    }
    let mut values = bindings.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        left.get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(right.get("id").and_then(Value::as_str).unwrap_or_default())
    });
    Ok(values)
}

fn import_native_communications(
    source: &Path,
    stats: &mut ImportStats,
) -> Result<Vec<Value>, String> {
    let source_file = [
        source.join("Configuration").join("Communications"),
        source.join("Communications"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file());
    let Some(source_file) = source_file else {
        return Ok(Vec::new());
    };
    stats.scanned();
    let bytes = match fs::read(&source_file) {
        Ok(bytes) => bytes,
        Err(_) => {
            stats.skipped(&source_file, "communications file could not be read");
            return Ok(Vec::new());
        }
    };
    let Some(values) = propresenter_proto::decode_communications(&bytes) else {
        stats.skipped(
            &source_file,
            "file is not a supported ProPresenter communications document",
        );
        return Ok(Vec::new());
    };
    Ok(values)
}

fn collect_all_files(folder: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = fs::read_dir(folder)
        .map_err(|error| {
            format!(
                "Could not scan ProPresenter folder '{}': {error}",
                folder.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not read an entry in '{}': {error}", folder.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_all_files(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn unique_id<'a, I>(base: &str, existing: I) -> String
where
    I: Iterator<Item = &'a str>,
{
    let existing = existing.collect::<Vec<_>>();
    if !existing.iter().any(|value| *value == base) {
        return base.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base}-{index}");
        if !existing.iter().any(|value| *value == candidate) {
            return candidate;
        }
        index += 1;
    }
}

pub fn pro_presenter_export_losses(path: &Path) -> Result<Vec<String>, String> {
    let mut workspace = read_workspace(path)?;
    let object = workspace
        .as_object_mut()
        .ok_or_else(|| "Workspace must be an object.".to_string())?;
    let items = value_array(object, "library")?;
    let composed = items
        .iter()
        .map(|item| content_themes::compose_item(object, item))
        .collect::<Vec<_>>();
    object.insert("library".to_string(), Value::Array(composed));
    Ok(import_export_report::export_losses(&workspace))
}

pub fn export_pro_presenter_workspace(path: &Path, destination: &Path) -> Result<Value, String> {
    if destination.exists() {
        return Err("The ProPresenter export destination already exists.".to_string());
    }
    let workspace = read_workspace(path)?;
    crate::workspace_export::file_url(destination)?;
    crate::workspace_export::validate_references(&workspace)?;
    let playlist_roots = crate::workspace_export::playlist_roots(
        workspace["playlist"]
            .as_array()
            .ok_or("Workspace playlists are required.")?,
    )?;
    let mut library_names = crate::workspace_export::Names::default();
    let mut theme_names = crate::workspace_export::Names::default();
    let export_directory = crate::workspace_export::NewDirectory::create(destination)?;
    fs::create_dir_all(destination.join("Libraries")).map_err(|error| error.to_string())?;
    fs::create_dir_all(destination.join("Playlists")).map_err(|error| error.to_string())?;
    fs::create_dir_all(destination.join("Configuration")).map_err(|error| error.to_string())?;
    let mut exported_presentations = 0_u64;
    let mut media_files = crate::workspace_export::MediaFiles::default();
    let mut presentation_paths = Map::new();
    let items = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "library",
    )?;
    let libraries = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "libraries",
    )?;
    for library in &libraries {
        let library_object = library
            .as_object()
            .ok_or_else(|| "Library must be an object.".to_string())?;
        let title = library_object
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Library");
        let folder = destination
            .join("Libraries")
            .join(library_names.folder(title, "Library"));
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        let mut item_names = crate::workspace_export::Names::default();
        for item_id in library_object
            .get("itemIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let Some(item) = items
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(item_id))
            else {
                continue;
            };
            let item_title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Presentation");
            let file = folder.join(format!(
                "{}.pro",
                item_names.folder(item_title, "Presentation")
            ));
            let bytes = media_files.encode(workspace.as_object().unwrap(), item, destination)?;
            fs::write(&file, bytes).map_err(|error| error.to_string())?;
            presentation_paths.insert(
                item_id.to_string(),
                json!(crate::workspace_export::file_url(&file)?),
            );
            exported_presentations += 1;
        }
    }

    let assigned_ids = libraries
        .iter()
        .filter_map(|library| library.get("itemIds").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .collect::<std::collections::HashSet<_>>();
    let unassigned = items.iter().filter(|item| {
        item.get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !assigned_ids.contains(id))
    });
    if unassigned.clone().next().is_some() {
        let folder = destination
            .join("Libraries")
            .join(library_names.reserve("Unassigned", "Library"));
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        let mut item_names = crate::workspace_export::Names::default();
        for item in unassigned {
            let item_id = item.get("id").and_then(Value::as_str).unwrap_or("item");
            let item_title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Presentation");
            let file = folder.join(format!(
                "{}.pro",
                item_names.folder(item_title, "Presentation")
            ));
            fs::write(
                &file,
                media_files.encode(workspace.as_object().unwrap(), item, destination)?,
            )
            .map_err(|error| error.to_string())?;
            presentation_paths.insert(
                item_id.to_string(),
                json!(crate::workspace_export::file_url(&file)?),
            );
            exported_presentations += 1;
        }
    }

    let playlists = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "playlist",
    )?;
    fs::write(
        destination.join("Playlists/Library"),
        propresenter_proto::encode_workspace_playlist(
            &playlist_roots,
            &presentation_paths,
            destination,
        )?,
    )
    .map_err(|error| error.to_string())?;
    let (bin_workspace, bin_paths) = media_files.prepare_media_bins(&workspace, destination)?;
    for (file, kind) in [("Media", "graphics"), ("Audio", "audio")] {
        let bytes = propresenter_proto::media_bin_export::encode(&bin_workspace, &bin_paths, kind)?;
        fs::write(destination.join("Playlists").join(file), bytes)
            .map_err(|error| error.to_string())?;
    }
    let themes = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "themes",
    )?
    .into_iter()
    .map(crate::workspace_export::theme)
    .collect::<Vec<_>>();
    for theme in &themes {
        let title = theme
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Theme");
        let folder = destination
            .join("Themes")
            .join(theme_names.folder(title, "Theme"));
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        fs::write(
            folder.join("Theme"),
            media_files.encode_theme(workspace.as_object().unwrap(), theme, destination)?,
        )
        .map_err(|error| error.to_string())?;
    }
    let timers = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "countdownTimers",
    )?;
    let effect_presets = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "mediaEffectPresets",
    )?;
    if !effect_presets.is_empty() {
        let folder = destination.join("Presets").join("Effect");
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        let mut file_names = crate::workspace_export::Names::default();
        for preset in &effect_presets {
            let bytes = propresenter_proto::encode_effect_preset(preset)
                .ok_or("Media effect preset cannot be exported.")?;
            let file_name =
                file_names.preset(preset["title"].as_str().unwrap_or_default(), "Effect");
            fs::write(folder.join(format!("{file_name}.effectPreset")), bytes)
                .map_err(|error| error.to_string())?;
        }
    }
    let transition_presets = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "transitionPresets",
    )?;
    if !transition_presets.is_empty() {
        let folder = destination.join("Presets").join("Transition");
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        let mut file_names = crate::workspace_export::Names::default();
        for preset in &transition_presets {
            let bytes = propresenter_proto::encode_transition_preset(preset)
                .ok_or("Transition favorite cannot be exported.")?;
            let file_name =
                file_names.preset(preset["name"].as_str().unwrap_or_default(), "Transition");
            fs::write(folder.join(format!("{file_name}.transitionPreset")), bytes)
                .map_err(|error| error.to_string())?;
        }
    }
    if !timers.is_empty() {
        let folder = destination.join("Configuration");
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        fs::write(
            folder.join("Timers"),
            propresenter_proto::encode_timers(&timers),
        )
        .map_err(|error| error.to_string())?;
    }
    let masks = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "masks",
    )?;
    {
        fs::create_dir_all(destination.join("Configuration")).map_err(|error| error.to_string())?;
        let (masks, relative) = media_files.copy_collection_media(
            workspace.as_object().unwrap(),
            &masks,
            destination,
        )?;
        fs::write(
            destination.join("Configuration/Workspace"),
            propresenter_proto::screen_config::encode(
                &workspace,
                propresenter_proto::encode_workspace_masks(&masks, &relative),
            )?,
        )
        .map_err(|error| error.to_string())?;
    }
    let stage_layouts = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "stageLayouts",
    )?;
    if !stage_layouts.is_empty() {
        fs::write(
            destination.join("Configuration/Stage"),
            propresenter_proto::encode_stage(&stage_layouts),
        )
        .map_err(|error| error.to_string())?;
    }
    let props = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "props",
    )?;
    let props_transition = workspace
        .get("layerTransitions")
        .and_then(|layers| layers.get("props"));
    if !props.is_empty() || props_transition.is_some() {
        let folder = destination.join("Configuration");
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        let (props, relative) = media_files.copy_collection_media(
            workspace.as_object().unwrap(),
            &props,
            destination,
        )?;
        fs::write(
            folder.join("Props"),
            propresenter_proto::encode_props_with_media(&props, props_transition, &relative),
        )
        .map_err(|error| error.to_string())?;
    }
    let groups = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "groups",
    )?;
    if !groups.is_empty() {
        let folder = destination.join("Configuration");
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        fs::write(
            folder.join("Groups"),
            propresenter_proto::encode_groups(&groups),
        )
        .map_err(|error| error.to_string())?;
    }
    let messages = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "messages",
    )?;
    if !messages.is_empty() {
        let folder = destination.join("Configuration");
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        fs::write(
            folder.join("Messages"),
            propresenter_proto::encode_messages(&messages, &themes),
        )
        .map_err(|error| error.to_string())?;
    }
    let macros = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "macros",
    )?;
    if !macros.is_empty() {
        let folder = destination.join("Configuration");
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        fs::write(
            folder.join("Macros"),
            propresenter_proto::encode_macros(&macros),
        )
        .map_err(|error| error.to_string())?;
    }
    let clear_groups = value_array(
        workspace
            .as_object()
            .ok_or_else(|| "Workspace must be an object.".to_string())?,
        "clearGroups",
    )?;
    if !clear_groups.is_empty() {
        let folder = destination.join("Configuration");
        fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        fs::write(
            folder.join("ClearGroups"),
            propresenter_proto::encode_clear_groups(&clear_groups)?,
        )
        .map_err(|error| error.to_string())?;
    }
    let summary = json!({
        "destinationPath": destination,
        "exportedPresentations": exported_presentations,
        "exportedPlaylists": playlists.len(),
        "exportedThemes": themes.len(),
        "exportedTimers": timers.len(),
        "exportedMasks": masks.len(),
        "exportedStageLayouts": stage_layouts.len(),
        "exportedProps": props.len(),
        "exportedGroups": groups.len(),
        "exportedMessages": messages.len(),
        "exportedMacros": macros.len(),
        "exportedClearGroups": clear_groups.len(),
        "exportedEffectPresets": effect_presets.len(),
        "exportedTransitionPresets": transition_presets.len()
    });
    fs::write(destination.join("export-report.json"), serde_json::to_vec_pretty(&json!({
        "format": "Presenter Orbit ProPresenter structure export",
        "summary": summary,
        "presentations": presentation_paths,
        "presentationNames": items.iter().map(|item| (item["id"].as_str().unwrap().to_string(), item["title"].clone())).collect::<Map<String, Value>>(),
        "losses": import_export_report::export_losses(&workspace)
    })).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    export_directory.commit();
    Ok(summary)
}

pub(crate) fn validate_audio_playlist_options(library: &Value) -> Result<(), String> {
    if library
        .get("audioShuffle")
        .is_some_and(|value| !value.is_boolean())
        || library.get("audioCrossfadeSeconds").is_some_and(|value| {
            value
                .as_f64()
                .is_none_or(|seconds| !seconds.is_finite() || !(0.0..=10.0).contains(&seconds))
        })
        || (library.get("audioShuffle").is_some() || library.get("audioCrossfadeSeconds").is_some())
            && (library["type"] != "playlist" || library["mediaType"] != "audio")
    {
        return Err("Shuffle and crossfade require an audio playlist, a boolean shuffle flag and a crossfade duration between 0 and 10 seconds.".into());
    }
    Ok(())
}
