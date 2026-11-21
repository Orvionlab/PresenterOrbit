//! The data directory of the application and the one-time import of the Tauri app's settings.
//!
//! A release build uses the Tauri identifier as its data directory, so an installation that was updated from
//! the Tauri build keeps its settings, workspaces registry and downloaded Bible packages without any copying.
//! A debug build keeps its own directory (`com.presenterorbit.gpui`) so development never writes to the settings
//! an installed release reads; on its first start it copies the Tauri `settings.json` once (workspace path,
//! screens, devices, general settings). The Tauri file is only read.

use std::path::{Path, PathBuf};

const TAURI_IDENTIFIER: &str = "com.presenterorbit.desktop";
const DEVELOPMENT_IDENTIFIER: &str = "com.presenterorbit.gpui";
const SETTINGS_FILE: &str = "settings.json";

pub fn data_directory_name() -> &'static str {
    if cfg!(debug_assertions) {
        DEVELOPMENT_IDENTIFIER
    } else {
        TAURI_IDENTIFIER
    }
}

pub fn data_directory() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|directory| directory.join(data_directory_name()))
        .ok_or_else(|| "The application data directory is unavailable.".to_string())
}

/// Copies `settings.json` from the Tauri data directory when this one has none. Returns whether it copied.
pub fn import_tauri_settings(
    data_directory: &Path,
    tauri_directory: &Path,
) -> Result<bool, String> {
    if data_directory == tauri_directory {
        return Ok(false);
    }
    let own = data_directory.join(SETTINGS_FILE);
    if own.exists() {
        return Ok(false);
    }
    let source = tauri_directory.join(SETTINGS_FILE);
    if !source.is_file() {
        return Ok(false);
    }
    std::fs::create_dir_all(data_directory).map_err(|error| error.to_string())?;
    std::fs::copy(&source, &own).map_err(|error| {
        format!(
            "Importing the settings of the previous version from {} failed: {error}",
            source.display()
        )
    })?;
    Ok(true)
}

/// The import for a normal start: from the Tauri directory next to this one in the user's data folder.
pub fn import_for_first_start() -> Result<bool, String> {
    let own = data_directory()?;
    let tauri = own.with_file_name(TAURI_IDENTIFIER);
    import_tauri_settings(&own, &tauri)
}
