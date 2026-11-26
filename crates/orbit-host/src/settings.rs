use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value};

const SETTINGS_FILE_NAME: &str = "settings.json";

/// Settings persisted by earlier desktop builds and understood by the shared
/// application contract. Keeping this list explicit prevents obsolete or
/// host-only values from being copied into the Tauri store.
const COMPATIBLE_KEYS: &[&str] = &[
    "audioSettings",
    "generalSettings",
    "groupSettings",
    "keyMappings",
    "presenterOrbitWorkspacePath",
    "remoteSettings",
    "screenConfig",
];

pub fn legacy_settings_paths() -> Vec<PathBuf> {
    let app_data = env::var_os("APPDATA").map(PathBuf::from);
    let local_app_data = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from);
    let xdg_config = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);

    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };

    legacy_settings_paths_for(
        platform,
        app_data.as_deref(),
        local_app_data.as_deref(),
        home.as_deref(),
        xdg_config.as_deref(),
    )
}

/// Build the previous desktop settings locations without reading process-global state. The
/// separate arguments make the platform rules testable on any host.
pub fn legacy_settings_paths_for(
    platform: &str,
    app_data: Option<&Path>,
    local_app_data: Option<&Path>,
    home: Option<&Path>,
    xdg_config: Option<&Path>,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let push = |paths: &mut Vec<PathBuf>, directory: Option<&Path>| {
        if let Some(directory) = directory {
            paths.push(directory.join("presenter-orbit").join(SETTINGS_FILE_NAME));
        }
    };

    match platform {
        "windows" => {
            push(&mut paths, app_data);
            push(&mut paths, local_app_data);
            // USERPROFILE is a useful fallback for portable/older desktop
            // installations that did not expose APPDATA to the process.
            if let Some(home) = home {
                push(
                    &mut paths,
                    Some(home.join("AppData").join("Roaming").as_path()),
                );
            }
        }
        "macos" => {
            if let Some(home) = home {
                push(
                    &mut paths,
                    Some(home.join("Library").join("Application Support").as_path()),
                );
                push(
                    &mut paths,
                    Some(home.join("Library").join("Preferences").as_path()),
                );
            }
            push(&mut paths, app_data);
        }
        _ => {
            push(&mut paths, xdg_config);
            if let Some(home) = home {
                push(&mut paths, Some(home.join(".config").as_path()));
            }
            push(&mut paths, app_data);
        }
    }

    let mut unique = Vec::with_capacity(paths.len());
    for path in paths {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

pub fn compatible_settings(value: &Value) -> Option<Value> {
    let source = value.as_object()?;
    let mut compatible = Map::new();

    for key in COMPATIBLE_KEYS {
        let Some(value) = source.get(*key) else {
            continue;
        };
        if *key == "presenterOrbitWorkspacePath"
            && value.as_str().is_none_or(|path| path.trim().is_empty())
        {
            continue;
        }
        compatible.insert((*key).to_string(), value.clone());
    }

    (!compatible.is_empty()).then_some(Value::Object(compatible))
}

pub fn normalize_screen_config_value(value: &Value) -> Value {
    let configured = value.as_object().cloned().unwrap_or_default();
    let defaults = default_screen_config();
    let mut normalized = Map::new();
    let mut has_audience = false;
    let mut has_stage = false;

    for (screen_id, value) in configured {
        let config = value.as_object().cloned().unwrap_or_default();
        let stage = config.get("type").and_then(Value::as_str) == Some("stage")
            || screen_id.starts_with("stage-");
        if stage {
            has_stage = true;
        } else {
            has_audience = true;
        }
        let default_id = if stage { "stage-1" } else { "audience-1" };
        let mut screen = defaults
            .get(default_id)
            .and_then(Value::as_object)
            .expect("default screen configuration must be an object")
            .clone();
        screen.extend(config.clone());
        // Stage layout selection is persisted in Configuration/Stage/Stage.json
        // as a screen assignment, never alongside the per-screen hardware or
        // look settings.
        screen.remove("stageLayoutId");
        screen.remove("settingsExpanded");
        screen.remove("settingsTab");
        screen.insert(
            "lookLayers".to_string(),
            normalize_screen_look_value(
                config.get("lookLayers"),
                [
                    ("music", Value::Bool(true)),
                    ("messages", Value::Bool(true)),
                    ("props", Value::Bool(true)),
                    ("announcements", Value::Bool(true)),
                    ("slide", Value::Bool(true)),
                    ("media", Value::Bool(true)),
                    ("videoInput", Value::Bool(true)),
                ],
            ),
        );
        screen.insert(
            "lookThemes".to_string(),
            normalize_screen_look_value(
                config.get("lookThemes"),
                [
                    ("maskId", Value::String(String::new())),
                    ("presentationThemeId", Value::String(String::new())),
                ],
            ),
        );
        let screen_type = if stage {
            Value::String("stage".to_string())
        } else {
            config
                .get("type")
                .filter(|value| json_value_is_truthy(value))
                .cloned()
                .unwrap_or_else(|| Value::String("audience".to_string()))
        };
        screen.insert("type".to_string(), screen_type);
        normalized.insert(screen_id, Value::Object(screen));
    }

    // Keep one usable screen for a group that has no entries at all, while
    // allowing a configured default (for example audience-1) to be deleted
    // when that group still has another screen. This makes native deletion
    // agree with the renderer's "at least one screen" product flow.
    if !has_audience {
        normalized.insert(
            "audience-1".to_string(),
            defaults
                .get("audience-1")
                .cloned()
                .expect("default audience screen must exist"),
        );
    }
    if !has_stage {
        normalized.insert(
            "stage-1".to_string(),
            defaults
                .get("stage-1")
                .cloned()
                .expect("default stage screen must exist"),
        );
    }

    Value::Object(normalized)
}

#[derive(Debug, PartialEq)]
pub struct ScreenConfigMutationResult {
    pub screen_config: Value,
    pub created_screen_id: Option<String>,
    pub deleted_screen_id: Option<String>,
    pub changed: bool,
}

/// Apply one semantic screen configuration intent to a canonical screen map.
///
/// Workspace scope planning lives in the Tauri host because the workspace may
/// carry an override and stage assignments in separate persisted documents.
/// This helper owns the shared validation and normalization for each map.
pub fn mutate_screen_config_value(
    current: &Value,
    mutation: &Value,
) -> Result<ScreenConfigMutationResult, String> {
    let mutation = mutation
        .as_object()
        .ok_or_else(|| "Screen configuration mutation must be an object.".to_string())?;
    let mutation_type = mutation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Screen configuration mutation type is required.".to_string())?;
    let original = normalize_screen_config_value(current);
    let mut screens = original
        .as_object()
        .cloned()
        .expect("normalized screen configuration must be an object");
    let mut created_screen_id = None;
    let mut deleted_screen_id = None;

    match mutation_type {
        "update" => {
            validate_screen_mutation_keys(
                mutation,
                &["type", "screenId", "patch", "stageLayoutId"],
            )?;
            let screen_id = required_screen_mutation_string(mutation, "screenId")?;
            let patch = mutation
                .get("patch")
                .and_then(Value::as_object)
                .ok_or_else(|| "Screen configuration update patch is required.".to_string())?;
            validate_screen_patch(patch)?;
            if let Some(stage_layout_id) = mutation.get("stageLayoutId") {
                if !stage_layout_id.is_null()
                    && stage_layout_id
                        .as_str()
                        .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(
                        "Screen configuration stage layout id must be a non-empty string or null."
                            .to_string(),
                    );
                }
            }
            let screen = screens
                .get_mut(screen_id)
                .and_then(Value::as_object_mut)
                .ok_or_else(|| format!("Screen configuration '{screen_id}' was not found."))?;
            let had_layout = screen
                .get("outputLayout")
                .is_some_and(|value| !value.is_null());
            screen.extend(patch.clone());
            if patch.contains_key("outputLayout") {
                if screen
                    .get("outputLayout")
                    .is_some_and(|value| !value.is_null())
                {
                    let layout = crate::output_layout::OutputLayout::from_screen(
                        screen_id,
                        &Value::Object(screen.clone()),
                    )?;
                    screen.insert(
                        "canvasSize".into(),
                        serde_json::to_value(layout.canvas()?)
                            .map_err(|error| error.to_string())?,
                    );
                } else if had_layout && !patch.contains_key("canvasSize") {
                    screen.remove("canvasSize");
                }
            }
        }
        "toggleGroup" => {
            validate_screen_mutation_keys(mutation, &["type", "screenType"])?;
            let screen_type = mutation
                .get("screenType")
                .and_then(Value::as_str)
                .filter(|screen_type| matches!(*screen_type, "audience" | "stage"))
                .ok_or_else(|| "Screen configuration toggle group type is invalid.".to_string())?;
            let group_ids = screens
                .iter()
                .filter(|(screen_id, screen)| screen_entry_type(screen_id, screen) == screen_type)
                .map(|(screen_id, _)| screen_id.clone())
                .collect::<Vec<_>>();
            if group_ids.is_empty() {
                return Err(format!(
                    "Screen configuration group '{screen_type}' was not found."
                ));
            }
            let enabled = !group_ids.iter().all(|screen_id| {
                screens
                    .get(screen_id)
                    .and_then(|screen| screen.get("enabled"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            });
            for screen_id in group_ids {
                screens
                    .get_mut(&screen_id)
                    .and_then(Value::as_object_mut)
                    .expect("screen group entries should be objects")
                    .insert("enabled".to_string(), Value::Bool(enabled));
            }
        }
        "setGroupEnabled" => {
            validate_screen_mutation_keys(
                mutation,
                &[
                    "type",
                    "screenType",
                    "enabled",
                    "enabledByScreen",
                    "lookLayersByScreen",
                ],
            )?;
            let screen_type = mutation
                .get("screenType")
                .and_then(Value::as_str)
                .filter(|screen_type| matches!(*screen_type, "audience" | "stage"))
                .ok_or_else(|| "Screen configuration group type is invalid.".to_string())?;
            let enabled = mutation
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| {
                    "Screen configuration group enabled value must be a boolean.".to_string()
                })?;
            let group_ids = screens
                .iter()
                .filter(|(screen_id, screen)| screen_entry_type(screen_id, screen) == screen_type)
                .map(|(screen_id, _)| screen_id.clone())
                .collect::<Vec<_>>();
            if group_ids.is_empty() {
                return Err(format!(
                    "Screen configuration group '{screen_type}' was not found."
                ));
            }
            let enabled_by_screen = mutation
                .get("enabledByScreen")
                .map(|value| {
                    value
                        .as_object()
                        .ok_or_else(|| "Screen activation must be an object.".to_string())
                })
                .transpose()?;
            if let Some(activation) = enabled_by_screen {
                if activation.len() != group_ids.len()
                    || group_ids
                        .iter()
                        .any(|id| !activation.get(id).is_some_and(Value::is_boolean))
                {
                    return Err("Screen activation must contain a boolean for every screen in the requested group.".to_string());
                }
            }
            let live_layers = mutation
                .get("lookLayersByScreen")
                .map(|value| {
                    value
                        .as_object()
                        .ok_or_else(|| "Screen look layers must be an object.".to_string())
                })
                .transpose()?;
            if let Some(layers) = live_layers {
                for (id, value) in layers {
                    if !group_ids.contains(id)
                        || !value
                            .as_object()
                            .is_some_and(|layers| layers.values().all(Value::is_boolean))
                    {
                        return Err("Screen look layers must contain boolean layers for screens in the requested group.".to_string());
                    }
                }
            }
            for screen_id in group_ids {
                let screen = screens
                    .get_mut(&screen_id)
                    .and_then(Value::as_object_mut)
                    .expect("screen group entries should be objects");
                let enabled = enabled_by_screen
                    .and_then(|activation| activation.get(&screen_id))
                    .and_then(Value::as_bool)
                    .unwrap_or(enabled);
                screen.insert("enabled".to_string(), Value::Bool(enabled));
                if let Some(layers) = live_layers.and_then(|layers| layers.get(&screen_id)) {
                    screen.insert("lookLayers".to_string(), layers.clone());
                }
            }
        }
        "setLookLayer" => {
            validate_screen_mutation_keys(mutation, &["type", "screenId", "layerId", "enabled"])?;
            let screen_id = required_screen_mutation_string(mutation, "screenId")?;
            let layer_id = required_screen_mutation_string(mutation, "layerId")?;
            if !matches!(
                layer_id,
                "slide" | "media" | "music" | "props" | "messages" | "announcements" | "videoInput"
            ) {
                return Err("Screen look layer is invalid.".into());
            }
            let enabled = mutation
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or("Screen look layer enabled state must be a boolean.")?;
            let screen = screens
                .get_mut(screen_id)
                .ok_or("Screen configuration was not found.")?;
            screen["lookLayers"][layer_id] = Value::Bool(enabled);
        }
        "clearLookLayer" => {
            validate_screen_mutation_keys(mutation, &["type", "layerId", "lookLayersByScreen"])?;
            let layer_id = required_screen_mutation_string(mutation, "layerId")?;
            let live_layers = mutation
                .get("lookLayersByScreen")
                .map(|value| {
                    value
                        .as_object()
                        .ok_or_else(|| "Screen look layers must be an object.".to_string())
                })
                .transpose()?;
            if let Some(layers) = live_layers {
                for (id, value) in layers {
                    if !screens.contains_key(id)
                        || !value
                            .as_object()
                            .is_some_and(|layers| layers.values().all(Value::is_boolean))
                    {
                        return Err(
                            "Screen look layers must contain boolean layers for existing screens."
                                .to_string(),
                        );
                    }
                }
            }
            for (screen_id, screen) in screens.iter_mut() {
                if let Some(layers) = live_layers.and_then(|layers| layers.get(screen_id)) {
                    screen["lookLayers"] = layers.clone();
                }
                let look_layers = screen
                    .as_object_mut()
                    .expect("normalized screen configuration entries should be objects")
                    .entry("lookLayers".to_string())
                    .or_insert_with(|| Value::Object(Map::new()))
                    .as_object_mut()
                    .expect("screen look layers should be an object");
                look_layers.insert(layer_id.to_string(), Value::Bool(false));
            }
        }
        "add" => {
            validate_screen_mutation_keys(mutation, &["type", "screenType", "baseLabel"])?;
            let screen_type = mutation
                .get("screenType")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| matches!(*value, "audience" | "stage"))
                .ok_or_else(|| "Screen configuration add screen type is invalid.".to_string())?;
            let base_label = required_screen_mutation_string(mutation, "baseLabel")?;
            let label_index = screens
                .iter()
                .filter(|(screen_id, screen)| screen_entry_type(screen_id, screen) == screen_type)
                .count()
                + 1;
            let mut id_index = label_index;
            let screen_id = loop {
                let candidate = format!("{screen_type}-{id_index}");
                if !screens.contains_key(&candidate) {
                    break candidate;
                }
                id_index += 1;
            };
            let label = format!("{} {label_index}", base_label.trim());
            screens.insert(
                screen_id.clone(),
                default_screen(screen_type, &label, &format!("Presenter Orbit {label}")),
            );
            created_screen_id = Some(screen_id);
        }
        "setStageAssignment" | "clearStageAssignment" => {
            return Err(
                "Stage screen assignments must use the workspace stage-assignment mutation."
                    .to_string(),
            );
        }
        "delete" => {
            validate_screen_mutation_keys(mutation, &["type", "screenId"])?;
            let screen_id = required_screen_mutation_string(mutation, "screenId")?;
            let screen = screens
                .get(screen_id)
                .and_then(Value::as_object)
                .ok_or_else(|| format!("Screen configuration '{screen_id}' was not found."))?;
            let screen_type = screen_entry_type_from_object(screen_id, screen);
            if !matches!(screen_type, "audience" | "stage") {
                return Err(format!(
                    "Screen configuration '{screen_id}' has an unsupported screen type."
                ));
            }
            let group_count = screens
                .iter()
                .filter(|(id, value)| screen_entry_type(id, value) == screen_type)
                .count();
            if group_count <= 1 {
                return Err(format!("The last {screen_type} screen cannot be removed."));
            }
            screens.remove(screen_id);
            deleted_screen_id = Some(screen_id.to_string());
        }
        _ => return Err("Unsupported screen configuration mutation.".to_string()),
    }

    crate::key_fill::mirror_activation(&mut screens, mutation)?;
    crate::output_canvas::validate_screen_config_map(&Value::Object(screens.clone()))?;
    let screen_config = normalize_screen_config_value(&Value::Object(screens));
    Ok(ScreenConfigMutationResult {
        changed: screen_config != original,
        screen_config,
        created_screen_id,
        deleted_screen_id,
    })
}

fn validate_screen_mutation_keys(
    mutation: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), String> {
    if let Some(key) = mutation.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!(
            "Unsupported screen configuration mutation field '{key}'."
        ));
    }
    Ok(())
}

fn validate_screen_patch(patch: &Map<String, Value>) -> Result<(), String> {
    crate::key_fill::parse_optional(patch.get("keyFill"))?;
    crate::output_delay::frames(patch.get("videoDelay"))?;
    crate::projector_correction::parse_optional(patch.get("projectorCorrection"))?;
    if let Some(value) = patch.get("ndiAlphaMode") {
        if !matches!(value.as_str(), Some("none" | "straight" | "premultiplied")) {
            return Err("NDI alpha mode must be none, straight or premultiplied.".into());
        }
    }
    if let Some(value) = patch.get("outputFormat") {
        value
            .as_str()
            .ok_or_else(|| "Screen output format must be sdr, hlg or pq.".to_string())
            .and_then(crate::native_hdr::OutputFormat::parse)?;
    }
    if let Some(value) = patch.get("testPattern").filter(|value| !value.is_null()) {
        crate::test_pattern::validate(value)?;
    }
    if patch
        .get("contentDestination")
        .is_some_and(|value| !matches!(value.as_str(), Some("main" | "announcements")))
    {
        return Err("Screen content destination must be main or announcements.".to_string());
    }

    for key in ["type", "stageLayoutId", "settingsExpanded", "settingsTab"] {
        if patch.contains_key(key) {
            return Err(match key {
                "type" => "Screen type cannot be changed by a screen update.".to_string(),
                "stageLayoutId" => {
                    "Stage layout assignment must use the stage assignment mutation.".to_string()
                }
                _ => format!("Screen configuration field '{key}' is transient."),
            });
        }
    }
    Ok(())
}

fn required_screen_mutation_string<'a>(
    mutation: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    mutation
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Screen configuration mutation {key} must not be empty."))
}

fn screen_entry_type<'a>(screen_id: &str, screen: &'a Value) -> &'a str {
    screen
        .as_object()
        .map(|screen| screen_entry_type_from_object(screen_id, screen))
        .unwrap_or_else(|| {
            if screen_id.starts_with("stage-") {
                "stage"
            } else {
                "audience"
            }
        })
}

fn screen_entry_type_from_object<'a>(screen_id: &str, screen: &'a Map<String, Value>) -> &'a str {
    screen
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if screen_id.starts_with("stage-") {
                "stage"
            } else {
                "audience"
            }
        })
}

/// Workspace screen configuration is an optional override. An empty or
/// malformed value must remain empty so loading that workspace keeps the
/// global screen settings instead of replacing them with native defaults.
pub fn normalize_workspace_screen_config_value(value: &Value) -> Value {
    if value.as_object().is_none_or(Map::is_empty) {
        Value::Object(Map::new())
    } else {
        normalize_screen_config_value(value)
    }
}

pub fn normalize_general_settings_value(value: &Value) -> Value {
    let configured = value.as_object().cloned().unwrap_or_default();
    let mut normalized = configured.clone();
    normalized.insert(
        "ccliDisplayEnabled".into(),
        Value::Bool(
            configured
                .get("ccliDisplayEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        ),
    );
    normalized.insert(
        "ccliDisplayLocation".into(),
        Value::String(normalized_string_option(
            configured.get("ccliDisplayLocation"),
            &["first", "last", "first-and-last", "all"],
            "first",
        )),
    );
    normalized.insert(
        "ccliLicenseNumber".into(),
        Value::String(
            configured
                .get("ccliLicenseNumber")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned(),
        ),
    );
    normalized.insert(
        "appTheme".to_string(),
        Value::String(normalized_string_option(
            configured.get("appTheme"),
            &["system", "dark", "light", "red"],
            "system",
        )),
    );
    normalized.insert(
        "appAccent".to_string(),
        Value::String(normalized_string_option(
            configured.get("appAccent"),
            &["blue", "red", "purple", "green"],
            "blue",
        )),
    );
    normalized.insert(
        "appLanguage".to_string(),
        Value::String(match configured.get("appLanguage") {
            None => "system".to_string(),
            value => normalized_string_option(value, &["system", "en", "nl"], "en"),
        }),
    );
    normalized.insert(
        "textScale".to_string(),
        Value::String(normalized_string_option(
            configured.get("textScale"),
            &["small", "default", "large", "xlarge"],
            "default",
        )),
    );
    normalized.insert(
        "uiDensity".to_string(),
        Value::String(normalized_string_option(
            configured.get("uiDensity"),
            &["compact", "comfortable", "spacious"],
            "comfortable",
        )),
    );
    normalized.insert(
        "renderingBitDepth".to_string(),
        Value::String(normalized_string_option(
            configured.get("renderingBitDepth"),
            &["normal", "enhanced"],
            "normal",
        )),
    );
    for key in ["launchAtLogin", "startMinimized", "minimizeToTray"] {
        normalized.insert(
            key.to_string(),
            Value::Bool(configured.get(key).is_some_and(json_value_is_truthy)),
        );
    }
    normalized.insert(
        "checkForUpdatesOnStartup".to_string(),
        Value::Bool(
            configured
                .get("checkForUpdatesOnStartup")
                .map_or(true, json_value_is_truthy),
        ),
    );
    normalized.insert(
        "useContinuousPlaylist".to_string(),
        Value::Bool(
            configured
                .get("useContinuousPlaylist")
                .is_some_and(json_value_is_truthy),
        ),
    );
    normalized.insert(
        "logoImagePath".to_string(),
        Value::String(
            configured
                .get("logoImagePath")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
    );
    normalized.insert(
        "syncRepositoryPath".to_string(),
        serde_json::json!(configured
            .get("syncRepositoryPath")
            .and_then(Value::as_str)
            .unwrap_or_default()),
    );
    normalized.insert(
        "logoImageUrl".to_string(),
        Value::String(
            configured
                .get("logoImageUrl")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
    );
    for (key, default) in [
        ("manageMediaAutomatically", true),
        ("relinkMissingMedia", false),
        ("altClickSuppressesVideoAutoStart", false),
    ] {
        normalized.insert(
            key.to_string(),
            Value::Bool(
                configured
                    .get(key)
                    .and_then(Value::as_bool)
                    .unwrap_or(default),
            ),
        );
    }
    for key in ["mediaSearchPaths", "hiddenMediaSearchPaths"] {
        normalized.insert(
            key.to_string(),
            Value::Array(
                configured
                    .get(key)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|path| !path.is_empty())
                    .map(|path| Value::String(path.to_string()))
                    .collect(),
            ),
        );
    }
    normalized.insert(
        "mediaSearchPathNames".to_string(),
        Value::Object(
            configured
                .get("mediaSearchPathNames")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .filter(|(_, name)| name.as_str().is_some_and(|name| !name.is_empty()))
                .map(|(path, name)| (path.clone(), name.clone()))
                .collect(),
        ),
    );
    for key in [
        "foregroundAudioPresentation",
        "foregroundAudioAnnouncements",
    ] {
        normalized.insert(
            key.to_string(),
            Value::String(normalized_string_option(
                configured.get(key),
                &[
                    "ignore",
                    "clear",
                    "clear-if-audio",
                    "pause",
                    "pause-if-audio",
                ],
                "ignore",
            )),
        );
    }
    crate::general_settings_options::normalize_into(&configured, &mut normalized);
    crate::ndi_find_options::normalize_into(&configured, &mut normalized);
    normalized.insert(
        "cueViewMode".to_string(),
        Value::String(normalized_string_option(
            configured.get("cueViewMode"),
            &["easy", "grid", "table"],
            "easy",
        )),
    );
    normalized.insert(
        "cueViewOptions".to_string(),
        crate::cue_view_options::normalize(
            configured.get("cueViewOptions").unwrap_or(&Value::Null),
        ),
    );
    Value::Object(normalized)
}

pub fn normalize_group_settings_value(value: &Value) -> Value {
    let mut normalized = default_group_settings();
    normalized.extend(normalize_group_setting_entries(value));
    Value::Object(normalized)
}

pub fn normalize_workspace_group_settings_value(value: &Value) -> Value {
    if value.as_object().is_none_or(Map::is_empty) {
        Value::Object(Map::new())
    } else {
        Value::Object(normalize_group_setting_entries(value))
    }
}

#[derive(Debug, PartialEq)]
pub struct GroupSettingsMutationResult {
    pub group_settings: Value,
    pub created_group_id: Option<String>,
    pub deleted_group_id: Option<String>,
    pub changed: bool,
}

pub fn mutate_group_settings_value(
    current: &Value,
    mutation: &Value,
) -> Result<GroupSettingsMutationResult, String> {
    let mutation = mutation
        .as_object()
        .ok_or_else(|| "Group settings mutation must be an object.".to_string())?;
    let mutation_type = mutation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Group settings mutation type is required.".to_string())?;
    let original = Value::Object(normalize_group_setting_entries(current));
    let mut groups = original
        .as_object()
        .cloned()
        .expect("normalized group settings must be an object");
    let mut created_group_id = None;
    let mut deleted_group_id = None;

    match mutation_type {
        "add" => {
            validate_group_mutation_keys(
                mutation,
                &["type", "baseName", "color", "key", "assignment"],
            )?;
            validate_group_assignment(mutation)?;
            let base_name = required_group_mutation_string(mutation, "baseName")?;
            validate_new_group_identity(base_name)?;
            let color = required_group_mutation_string(mutation, "color")?;
            let key = group_mutation_string(mutation, "key")?;
            let name = unique_group_name(base_name, &groups, None);
            let base_id = {
                let candidate = settings_slug(&name);
                if candidate.is_empty() {
                    "group".to_string()
                } else {
                    candidate
                }
            };
            let mut group_id = base_id.clone();
            let mut index = 2;
            while groups.contains_key(&group_id) {
                group_id = format!("{base_id}-{index}");
                index += 1;
            }
            groups.insert(
                group_id.clone(),
                serde_json::json!({ "name": name, "color": color, "key": key }),
            );
            created_group_id = Some(group_id);
        }
        "rename" => {
            validate_group_mutation_keys(mutation, &["type", "groupId", "name"])?;
            let group_id = required_group_mutation_string(mutation, "groupId")?;
            let requested_name = required_group_mutation_string(mutation, "name")?;
            if !groups.contains_key(group_id) {
                return Err(format!("Group setting '{group_id}' was not found."));
            }
            if is_generated_slide_metadata_label(requested_name) {
                return Err(format!(
                    "Group setting name '{requested_name}' is reserved."
                ));
            }
            let name = unique_group_name(requested_name, &groups, Some(group_id));
            groups
                .get_mut(group_id)
                .and_then(Value::as_object_mut)
                .ok_or_else(|| format!("Group setting '{group_id}' is invalid."))?
                .insert("name".to_string(), Value::String(name));
        }
        "update" => {
            validate_group_mutation_keys(mutation, &["type", "groupId", "patch"])?;
            let group_id = required_group_mutation_string(mutation, "groupId")?;
            let patch = mutation
                .get("patch")
                .and_then(Value::as_object)
                .ok_or_else(|| "Group settings update patch is required.".to_string())?;
            if let Some(key) = patch
                .keys()
                .find(|key| !["color", "key"].contains(&key.as_str()))
            {
                return Err(format!("Unsupported group settings update field '{key}'."));
            }
            let group = groups
                .get_mut(group_id)
                .and_then(Value::as_object_mut)
                .ok_or_else(|| format!("Group setting '{group_id}' was not found."))?;
            for field in ["color", "key"] {
                if patch.contains_key(field) {
                    group.insert(
                        field.to_string(),
                        Value::String(group_mutation_string(patch, field)?.to_string()),
                    );
                }
            }
        }
        "delete" => {
            validate_group_mutation_keys(mutation, &["type", "groupId", "slideLabels"])?;
            let group_id = required_group_mutation_string(mutation, "groupId")?;
            validate_group_slide_labels(mutation)?;
            groups.remove(group_id);
            deleted_group_id = Some(group_id.to_string());
        }
        _ => return Err("Unsupported group settings mutation.".to_string()),
    }

    let group_settings = Value::Object(normalize_group_setting_entries(&Value::Object(groups)));
    Ok(GroupSettingsMutationResult {
        changed: group_settings != original,
        group_settings,
        created_group_id,
        deleted_group_id,
    })
}

fn validate_group_mutation_keys(
    mutation: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), String> {
    if let Some(key) = mutation.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!(
            "Unsupported group settings mutation field '{key}'."
        ));
    }
    Ok(())
}

fn validate_group_slide_labels(mutation: &Map<String, Value>) -> Result<(), String> {
    let labels = mutation
        .get("slideLabels")
        .and_then(Value::as_array)
        .ok_or_else(|| "Group deletion slide labels must be an array.".to_string())?;
    let mut identities = HashSet::with_capacity(labels.len());
    for entry in labels {
        let entry = entry
            .as_object()
            .ok_or_else(|| "Group deletion slide labels must be objects.".to_string())?;
        validate_group_mutation_keys(entry, &["itemId", "slideId", "label"])?;
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
        if entry.get("label").and_then(Value::as_str).is_none() {
            return Err("Group deletion slide label must be a string.".to_string());
        }
        if !identities.insert((item_id.to_string(), slide_id.to_string())) {
            return Err(format!(
                "Group deletion supplied duplicate label for slide '{slide_id}' in item '{item_id}'."
            ));
        }
    }
    Ok(())
}

fn validate_group_assignment(mutation: &Map<String, Value>) -> Result<(), String> {
    let Some(assignment) = mutation.get("assignment") else {
        return Ok(());
    };
    let assignment = assignment
        .as_object()
        .ok_or_else(|| "Group assignment must be an object.".to_string())?;
    validate_group_mutation_keys(assignment, &["itemId", "slideId"])?;
    for (field, message) in [
        ("itemId", "Group assignment item id is required."),
        ("slideId", "Group assignment slide id is required."),
    ] {
        assignment
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| message.to_string())?;
    }
    Ok(())
}

fn group_mutation_string<'a>(
    mutation: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    mutation
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Group settings mutation {key} must be a string."))
}

fn required_group_mutation_string<'a>(
    mutation: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    group_mutation_string(mutation, key).and_then(|value| {
        let value = value.trim();
        if value.is_empty() {
            Err(format!("Group settings mutation {key} must not be empty."))
        } else {
            Ok(value)
        }
    })
}

fn unique_group_name(
    base_name: &str,
    groups: &Map<String, Value>,
    ignored_id: Option<&str>,
) -> String {
    let existing_names = groups
        .iter()
        .filter(|(group_id, _)| ignored_id != Some(group_id.as_str()))
        .filter_map(|(_, group)| group.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let mut name = base_name.trim().to_string();
    let mut index = 2;
    while existing_names.contains(&name.as_str()) {
        name = format!("{} {index}", base_name.trim());
        index += 1;
    }
    name
}

fn validate_new_group_identity(base_name: &str) -> Result<(), String> {
    let name = base_name.trim();
    let id = settings_slug(name);
    if id.is_empty()
        || id == "no-group"
        || is_generated_slide_metadata_label(&id)
        || is_generated_slide_metadata_label(name)
    {
        return Err(format!("Group setting name '{name}' is reserved."));
    }
    Ok(())
}

pub fn merge_imported_group_settings_value(
    current: &Value,
    groups: &[Value],
    library: &[Value],
) -> Value {
    let mut normalized = normalize_group_settings_value(current)
        .as_object()
        .cloned()
        .expect("normalized group settings must be an object");

    for group in groups {
        add_imported_group(&mut normalized, group, true, None);
    }
    add_presentation_groups(&mut normalized, library);
    Value::Object(normalized)
}

pub(crate) fn pro_presenter_workspace_group_settings(
    current: &Value,
    existing: &Value,
    groups: &[Value],
    library: &mut [Value],
) -> Value {
    let mut imported = Map::new();
    for group in groups {
        add_imported_group(&mut imported, group, true, group["name"].as_str());
    }
    add_presentation_groups(&mut imported, library);
    let mut settings = default_group_settings();
    let existing = normalize_group_setting_entries(existing);
    settings.extend(existing.clone());
    let preferences = normalize_group_setting_entries(current);
    for (id, definition) in &mut settings {
        if let Some(preference) = preferences.get(id).filter(|_| !existing.contains_key(id)) {
            *definition = preference.clone();
        }
    }
    let mut aliases = Map::new();
    for (id, definition) in imported {
        let name = settings_slug(definition["name"].as_str().unwrap_or_default());
        let by_name = |entries: &Map<String, Value>| {
            entries.iter().find_map(|(key, group)| {
                (settings_slug(group["name"].as_str().unwrap_or_default()) == name)
                    .then(|| key.clone())
            })
        };
        if existing.contains_key(&id) {
            continue;
        }
        if let Some(existing_id) = by_name(&existing) {
            settings.remove(&id);
            aliases.insert(id, Value::String(existing_id));
            continue;
        }
        let preference = preferences
            .get(&id)
            .or_else(|| by_name(&preferences).and_then(|id| preferences.get(&id)));
        settings.insert(id, preference.cloned().unwrap_or(definition));
    }
    if !aliases.is_empty() {
        for item in library {
            for (field, key) in [("groups", "id"), ("slides", "groupId")] {
                for entry in item[field].as_array_mut().into_iter().flatten() {
                    if let Some(id) = entry[key].as_str().and_then(|id| aliases.get(id)) {
                        entry[key] = id.clone();
                    }
                }
            }
        }
    }
    Value::Object(settings)
}

fn add_presentation_groups(normalized: &mut Map<String, Value>, library: &[Value]) {
    for item in library {
        let Some(item) = item.as_object() else {
            continue;
        };
        for group in item
            .get("groups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            add_imported_group(normalized, group, false, None);
        }
        for slide in item
            .get("slides")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            add_imported_slide_group(normalized, slide);
        }
    }
}

pub fn normalize_key_mappings_value(value: &Value) -> Value {
    let mut normalized = default_key_mappings();
    normalized.extend(string_entries(value));
    Value::Object(normalized)
}

pub fn normalize_workspace_key_mappings_value(value: &Value) -> Value {
    if value.as_object().is_none_or(Map::is_empty) {
        Value::Object(Map::new())
    } else {
        normalize_key_mappings_value(value)
    }
}

pub fn mutate_key_mappings_value(
    current: &Value,
    mutation: &Value,
) -> Result<(Value, bool), String> {
    let mutation = mutation
        .as_object()
        .ok_or_else(|| "Key mapping mutation must be an object.".to_string())?;
    if let Some(field) = mutation
        .keys()
        .find(|field| !["type", "commandId", "shortcut"].contains(&field.as_str()))
    {
        return Err(format!("Unsupported key mapping mutation field '{field}'."));
    }
    if mutation.get("type").and_then(Value::as_str) != Some("set") {
        return Err("Unsupported key mapping mutation type.".to_string());
    }
    let command_id = mutation
        .get("commandId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Key mapping command id is required.".to_string())?;
    if !default_key_mappings().contains_key(command_id) {
        return Err(format!("Unknown key mapping command '{command_id}'."));
    }
    let shortcut = mutation
        .get("shortcut")
        .and_then(Value::as_str)
        .ok_or_else(|| "Key mapping shortcut must be a string.".to_string())?;
    let original = normalize_key_mappings_value(current);
    let mut next = original
        .as_object()
        .cloned()
        .expect("normalized key mappings must be an object");
    if !shortcut.is_empty() {
        for (id, value) in &mut next {
            if id != command_id && value.as_str() == Some(shortcut) {
                *value = Value::String(String::new());
            }
        }
    }
    next.insert(command_id.to_string(), Value::String(shortcut.to_string()));
    let next = Value::Object(next);
    let changed = next != original;
    Ok((next, changed))
}

pub struct KeyMappingScopeMutation {
    pub global: Value,
    pub workspace: Option<Value>,
    pub effective: Value,
    pub global_changed: bool,
    pub workspace_changed: bool,
}

pub fn mutate_key_mapping_scopes(
    global: &Value,
    workspace: Option<&Value>,
    mutation: &Value,
) -> Result<KeyMappingScopeMutation, String> {
    let global_current = normalize_key_mappings_value(global);
    let (global, global_changed) = mutate_key_mappings_value(&global_current, mutation)?;
    let Some(workspace) = workspace else {
        return Ok(KeyMappingScopeMutation {
            effective: global.clone(),
            global,
            workspace: None,
            global_changed,
            workspace_changed: false,
        });
    };
    let workspace_current = if workspace.as_object().is_none_or(Map::is_empty) {
        global_current
    } else {
        normalize_key_mappings_value(workspace)
    };
    let (workspace, workspace_changed) = mutate_key_mappings_value(&workspace_current, mutation)?;
    Ok(KeyMappingScopeMutation {
        global,
        effective: workspace.clone(),
        workspace: Some(workspace),
        global_changed,
        workspace_changed,
    })
}

fn normalize_group_setting_entries(value: &Value) -> Map<String, Value> {
    let mut normalized = Map::new();
    let Some(groups) = value.as_object() else {
        return normalized;
    };

    for (key, group) in groups {
        let group = group.as_object().cloned().unwrap_or_default();
        let name = scalar_string(group.get("name"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| key.clone())
            .trim()
            .to_string();
        let id_source = scalar_string(group.get("id"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                if key.is_empty() {
                    name.clone()
                } else {
                    key.clone()
                }
            });
        let id = settings_slug(&id_source);
        if id.is_empty() || id == "no-group" {
            continue;
        }
        let color = group
            .get("color")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("#0f7db7");
        let key = group.get("key").and_then(Value::as_str).unwrap_or_default();
        normalized.insert(
            id,
            serde_json::json!({ "name": name, "color": color, "key": key }),
        );
    }
    normalized
}

fn add_imported_group(
    groups: &mut Map<String, Value>,
    value: &Value,
    include_key: bool,
    id_source: Option<&str>,
) {
    let Some(group) = value.as_object() else {
        return;
    };
    let name = group
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let id_source = id_source
        .or_else(|| group.get("id").and_then(Value::as_str))
        .unwrap_or(&name);
    let id = settings_slug(id_source);
    if id.is_empty() || id == "no-group" || groups.contains_key(&id) {
        return;
    }
    let color = group
        .get("color")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("#0f7db7");
    let key = include_key
        .then(|| group.get("key").and_then(Value::as_str).unwrap_or_default())
        .unwrap_or_default();
    groups.insert(
        id.clone(),
        serde_json::json!({
            "name": if name.is_empty() { id.as_str() } else { name.as_str() },
            "color": color,
            "key": key
        }),
    );
}

fn add_imported_slide_group(groups: &mut Map<String, Value>, value: &Value) {
    let Some(slide) = value.as_object() else {
        return;
    };
    let source = if slide.contains_key("groupId") {
        slide
            .get("groupId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    } else {
        slide
            .get("groupName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let id = settings_slug(&source);
    let name = slide
        .get("groupName")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| id.clone());
    if !is_valid_imported_group(&id, &name) || groups.contains_key(&id) {
        return;
    }
    groups.insert(
        id,
        serde_json::json!({ "name": name, "color": "#0f7db7", "key": "" }),
    );
}

fn is_valid_imported_group(id: &str, name: &str) -> bool {
    !id.is_empty()
        && id != "no-group"
        && !is_generated_slide_metadata_label(id)
        && !is_generated_slide_metadata_label(name)
}

fn string_entries(value: &Value) -> Map<String, Value> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), Value::String(value.to_string())))
        })
        .collect()
}

fn scalar_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn settings_slug(value: &str) -> String {
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

fn is_generated_slide_metadata_label(value: &str) -> bool {
    let value = value.trim();
    if value.len() >= 8 && value.chars().all(|character| character.is_ascii_digit()) {
        return true;
    }
    let value = value.to_ascii_lowercase();
    value == "new-slide"
        || value == "new-slide-copy"
        || value.strip_prefix("new-slide-copy-").is_some_and(|digits| {
            !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
        })
}

fn default_group_settings() -> Map<String, Value> {
    [
        ("verse", "Verse", "#0f7db7", "A"),
        ("verse-1", "Verse 1", "#0f7db7", "A"),
        ("verse-2", "Verse 2", "#0b6697", "S"),
        ("verse-3", "Verse 3", "#0a5782", "D"),
        ("verse-4", "Verse 4", "#084c72", "F"),
        ("verse-5", "Verse 5", "#074565", "G"),
        ("verse-6", "Verse 6", "#063e59", "H"),
        ("chorus", "Chorus", "#d00069", "C"),
        ("chorus-1", "Chorus 1", "#d00069", "C"),
        ("chorus-2", "Chorus 2", "#b9005d", "X"),
        ("chorus-3", "Chorus 3", "#a60054", "Z"),
        ("chorus-4", "Chorus 4", "#94004a", ""),
        ("bridge", "Bridge", "#8d00df", "B"),
        ("bridge-1", "Bridge 1", "#8d00df", "B"),
        ("bridge-2", "Bridge 2", "#6f00b1", "N"),
        ("bridge-3", "Bridge 3", "#510083", "M"),
        ("prechorus", "PreChorus", "#db2c8f", "P"),
        ("tag", "Tag", "#d73434", "T"),
        ("intro", "Intro", "#aaa528", "I"),
        ("ending", "Ending", "#999326", "E"),
        ("outro", "Outro", "#858024", "O"),
        ("interlude", "Interlude", "#20b95a", ""),
        ("vamp", "Vamp", "#22a851", ""),
        ("turnaround", "Turnaround", "#239846", ""),
        ("blank", "Blank", "#000000", ""),
    ]
    .into_iter()
    .map(|(id, name, color, key)| {
        (
            id.to_string(),
            serde_json::json!({ "name": name, "color": color, "key": key }),
        )
    })
    .collect()
}

fn default_key_mappings() -> Map<String, Value> {
    [
        ("newSlide", "Ctrl+Shift+N"),
        ("nextSlide", "ArrowRight"),
        ("previousSlide", "ArrowLeft"),
        ("nextBackground", "Alt+ArrowRight"),
        ("previousBackground", "Alt+ArrowLeft"),
        ("clearAll", "F1"),
        ("clearSlide", "F2"),
        ("clearMedia", "F3"),
        ("clearProps", "F4"),
        ("clearAudio", "F5"),
        ("clearMessages", "F6"),
        ("clearAnnouncements", "F7"),
        ("clearToLogo", "F12"),
        ("themeShapeRoundedRect", "O"),
        ("themeShapeRectangle", "R"),
        ("themeShapeEllipse", "E"),
        ("themeShapeTriangle", "G"),
        ("themeShapeRightTriangle", "L"),
        ("themeShapeRhombus", "H"),
        ("themeShapePolygon", "P"),
        ("themeShapeStar", "S"),
        ("themeShapeDoubleArrow", "D"),
        ("themeShapeArrow", "A"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), Value::String(value.to_string())))
    .collect()
}

fn normalized_string_option(value: Option<&Value>, options: &[&str], fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .filter(|value| options.contains(value))
        .unwrap_or(fallback)
        .to_string()
}

fn default_screen_config() -> Map<String, Value> {
    Map::from_iter([
        (
            "audience-1".to_string(),
            default_screen("audience", "Audience 1", "Presenter Orbit Audience 1"),
        ),
        (
            "stage-1".to_string(),
            default_screen("stage", "Stage 1", "Presenter Orbit Stage 1"),
        ),
    ])
}

fn default_screen(screen_type: &str, label: &str, ndi_name: &str) -> Value {
    serde_json::json!({
        "type": screen_type,
        "label": label,
        "enabled": false,
        "targetType": "display",
        "targetId": "",
        "ndiName": ndi_name,
        "outputExpanded": false,
        "lookLayers": {
            "music": true,
            "messages": true,
            "props": true,
            "announcements": true,
            "slide": true,
            "media": true,
            "videoInput": true
        },
        "lookThemes": {
            "maskId": "",
            "presentationThemeId": ""
        },
        "ndiResolution": "1080p60",
        "ndiWidth": 1920,
        "ndiHeight": 1080,
        "ndiFramerate": 60,
        "ndiAlphaMode": "none",
        "outputFormat": "sdr",
        "videoDelay": 0
    })
}

fn normalize_screen_look_value<const N: usize>(
    value: Option<&Value>,
    defaults: [(&str, Value); N],
) -> Value {
    let mut normalized = Map::from_iter(
        defaults
            .into_iter()
            .map(|(key, value)| (key.to_string(), value)),
    );
    if let Some(value) = value.and_then(Value::as_object) {
        normalized.extend(value.clone());
    }
    Value::Object(normalized)
}

fn json_value_is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

pub fn has_workspace_path(value: &Value) -> bool {
    value
        .get("presenterOrbitWorkspacePath")
        .and_then(Value::as_str)
        .is_some_and(|path| !path.trim().is_empty())
}

fn has_meaningful_value(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::String(value)) => !value.trim().is_empty(),
        Some(Value::Array(value)) => !value.is_empty(),
        Some(Value::Object(value)) => !value.is_empty(),
        Some(Value::Bool(_)) | Some(Value::Number(_)) => true,
    }
}

/// Merge legacy values only into missing/empty Tauri fields.  This function
/// deliberately gives the current Tauri store precedence, including for false
/// booleans and zero-valued numbers, because those are valid user choices.
pub fn merge_missing_settings(existing: &Value, legacy: &Value) -> Option<Value> {
    if has_workspace_path(existing) {
        return None;
    }

    let mut merged = existing.as_object()?.clone();
    let compatible = compatible_settings(legacy)?.as_object()?.clone();
    let mut changed = false;

    for (key, value) in compatible {
        if !has_meaningful_value(merged.get(&key)) {
            merged.insert(key, value);
            changed = true;
        }
    }

    changed.then_some(Value::Object(merged))
}

pub fn read_legacy_settings(paths: &[PathBuf]) -> Option<Value> {
    paths.iter().find_map(|path| {
        let contents = fs::read_to_string(path).ok()?;
        let value = serde_json::from_str::<Value>(&contents).ok()?;
        compatible_settings(&value)
    })
}
