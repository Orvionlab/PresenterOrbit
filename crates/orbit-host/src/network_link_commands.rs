use crate::Host;
use std::{fs, io::Write, path::PathBuf};

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::{network_link, AppState};

const SETTINGS_FILE: &str = "network-link.json";
const MAX_SETTINGS_BYTES: u64 = 64 * 1024;
const METHODS: [&str; 4] = [
    "getNetworkLinkSettings",
    "saveNetworkLinkSettings",
    "getNetworkLinkStatus",
    "sendNetworkLinkCue",
];

pub fn handles(method: &str) -> bool {
    METHODS.contains(&method)
}

pub(crate) fn save(
    app: &Host,
    state: &crate::AppState,
    settings: network_link::NetworkLinkSettings,
) -> Result<network_link::NetworkLinkSettings, String> {
    update_settings(app, state, |_| Ok(settings))
}

pub(crate) fn update_settings(
    app: &Host,
    state: &crate::AppState,
    update: impl FnOnce(
        network_link::NetworkLinkSettings,
    ) -> Result<network_link::NetworkLinkSettings, String>,
) -> Result<network_link::NetworkLinkSettings, String> {
    let mut link = state
        .network_link
        .lock()
        .map_err(|_| "Network Link state is unavailable.")?;
    let settings = network_link::validate_settings(update(link.settings())?)?;
    write_settings_file(&settings_path(app)?, &settings)?;
    link.configure(settings.clone())?;
    Ok(settings)
}

pub fn load(app: &Host, state: &AppState) -> Result<(), String> {
    let settings = read_settings_file(&settings_path(app)?)?;
    state
        .network_link
        .lock()
        .map_err(|_| "Network Link state is unavailable.")?
        .configure(settings)
}

pub fn run(app: &Host, state: &AppState, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "getNetworkLinkSettings" => encode(
            state
                .network_link
                .lock()
                .map_err(|_| "Network Link state is unavailable.")?
                .settings(),
        ),
        "saveNetworkLinkSettings" => {
            let settings = decode::<network_link::NetworkLinkSettings>(argument(args, 0)?)?;
            let settings = save(app, state, settings)?;
            encode(settings)
        }
        "getNetworkLinkStatus" => encode(
            state
                .network_link
                .lock()
                .map_err(|_| "Network Link state is unavailable.")?
                .status()?,
        ),
        "sendNetworkLinkCue" => {
            let link_enabled = state
                .network_link
                .lock()
                .map_err(|_| "Network Link state is unavailable.")?
                .can_send();
            if !link_enabled {
                return Ok(json!({"queued": false}));
            }
            let requested_path = argument(args, 0)?.clone();
            let cue = decode::<network_link::NetworkLinkCue>(argument(args, 1)?)?;
            let path = crate::active_workspace(state, requested_path)?;
            let workspace_io = crate::lock_workspace_io(state)?;
            workspace_io.require_active(&path)?;
            let workspace = crate::workspace::read_workspace(&path)?;
            network_link::map_cue(&workspace, &path, cue.clone())?;
            let mut link = state
                .network_link
                .lock()
                .map_err(|_| "Network Link state is unavailable.")?;
            if !link.can_send() {
                return Ok(json!({"queued": false}));
            }
            link.send(cue)
        }
        _ => Err("Unknown Network Link operation.".into()),
    }
}

fn settings_path(app: &Host) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join(SETTINGS_FILE))
        .map_err(|error| error.to_string())
}

fn read_settings_file(path: &std::path::Path) -> Result<network_link::NetworkLinkSettings, String> {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(network_link::NetworkLinkSettings::default())
        }
        Err(error) => return Err(format!("Network Link settings could not be read: {error}")),
    };
    if contents.len() as u64 > MAX_SETTINGS_BYTES {
        return Err("Network Link settings exceed the 64 KiB size limit.".into());
    }
    let settings: network_link::NetworkLinkSettings = serde_json::from_slice(&contents)
        .map_err(|error| format!("Network Link settings are invalid: {error}"))?;
    network_link::validate_settings(settings)
}

fn write_settings_file(
    path: &std::path::Path,
    settings: &network_link::NetworkLinkSettings,
) -> Result<(), String> {
    let folder = path
        .parent()
        .ok_or("Network Link settings path has no parent folder.")?;
    fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("Network Link settings path must have a file name.")?;
    let temporary = folder.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        crate::workspace_sync::replace_file_atomically(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| format!("Network Link settings could not be saved: {error}"))
}

fn argument(args: &[Value], index: usize) -> Result<&Value, String> {
    args.get(index)
        .ok_or_else(|| format!("Network Link argument {} is required.", index + 1))
}

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone())
        .map_err(|error| format!("Invalid Network Link request: {error}"))
}

fn encode(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}
