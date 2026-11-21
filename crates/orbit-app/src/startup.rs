pub(crate) mod first_page;

use std::path::Path;
use std::sync::Arc;

use orbit_host::{diagnostics::LogLevel, record_diagnostic, Host};
use orbit_ui::AppearanceSettings;
use serde_json::{json, Value};

use crate::collections::model::Snapshot;
use crate::desktop::StartupPreferences;
use crate::keys::KeyMappings;
use crate::state::revisions::Revisions;

#[derive(Clone, Debug)]
pub struct Startup {
    pub output_settings: Option<Arc<Value>>,
    pub language: Option<String>,
    pub appearance: Option<AppearanceSettings>,
    pub mappings: KeyMappings,
    pub desktop: StartupPreferences,
    /// The loaded workspace, parsed for the collection panels. A workspace that lacks a required field is an
    /// error, not a default.
    pub workspace: Result<Option<Snapshot>, String>,
}

fn language_setting(settings: &Value) -> Option<String> {
    settings["generalSettings"]["appLanguage"]
        .as_str()
        .map(str::to_string)
}

fn workspace_path(override_path: Option<&Path>, settings: &Value) -> Option<String> {
    override_path
        .map(|path| path.to_string_lossy().into_owned())
        .or_else(|| {
            settings["presenterOrbitWorkspacePath"]
                .as_str()
                .filter(|path| !path.trim().is_empty())
                .map(str::to_string)
        })
}

fn load_workspace(host: &Host, path: &str, revisions: &Revisions) -> Result<Snapshot, String> {
    let loaded = host.invoke("loadWorkspace", vec![json!(path)])?;
    revisions.remember(path, &loaded);
    Snapshot::parse(&loaded["workspace"])
}

/// Turns the tray on when the saved preferences ask for it. The tray lives on the main thread, so this runs on the
/// host's thread, which waits for it.
fn start_tray(host: &Host, settings: &Value) -> Result<(), String> {
    let general = &settings["generalSettings"];
    if general["minimizeToTray"] != true {
        return Ok(());
    }
    let mut off = general.clone();
    off["minimizeToTray"] = json!(false);
    host.desktop().apply_preferences(&off, general)
}

/// Blocks on host calls, so it runs on a background thread.
pub fn load(host: &Host, override_path: Option<&Path>, revisions: &Revisions) -> Startup {
    let startup = match host.invoke("getSettings", Vec::new()) {
        Ok(settings) => match AppearanceSettings::from_settings(&settings) {
            Ok(appearance) => {
                revisions.remember_settings(&settings);
                let workspace = start_tray(host, &settings).and_then(|_| {
                    workspace_path(override_path, &settings)
                        .map(|path| load_workspace(host, &path, revisions))
                        .transpose()
                });
                Startup {
                    output_settings: Some(Arc::new(settings.clone())),
                    language: language_setting(&settings),
                    appearance: Some(appearance),
                    mappings: KeyMappings::from_settings(&settings),
                    desktop: StartupPreferences::from_settings(&settings),
                    workspace,
                }
            }
            Err(error) => Startup {
                output_settings: Some(Arc::new(settings.clone())),
                language: language_setting(&settings),
                appearance: None,
                mappings: KeyMappings::from_settings(&settings),
                desktop: StartupPreferences::default(),
                workspace: Err(error),
            },
        },
        Err(error) => Startup {
            output_settings: None,
            language: None,
            appearance: None,
            mappings: KeyMappings::default(),
            desktop: StartupPreferences::default(),
            workspace: Err(error),
        },
    };
    if let Err(error) = &startup.workspace {
        eprintln!("Presenter Orbit startup failed: {error}");
        record_diagnostic(
            host,
            "startup-failed",
            LogLevel::Error,
            "startup",
            Some(error),
        );
    }
    startup
}
