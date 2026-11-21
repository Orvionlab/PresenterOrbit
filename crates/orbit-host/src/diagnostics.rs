//! Privacy-safe diagnostics for the native Tauri host.
//!
//! Diagnostics intentionally contain only runtime metadata, a small allowlist
//! of settings, and recent structured log entries.  Workspace content, file
//! paths, credentials, and raw NDI loader errors are never exported.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use serde_json::{json, Map, Value};

const LOG_DIRECTORY_NAME: &str = "logs";
const LOG_FILE_NAME: &str = "presenter-orbit.log";
const PREVIOUS_LOG_FILE_NAME: &str = "presenter-orbit.previous.log";
const MAX_LOG_BYTES: u64 = 512 * 1024;
const MAX_RECENT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEvent<'a> {
    pub event: &'a str,
    pub level: LogLevel,
    pub subsystem: &'a str,
    pub correlation_id: Option<&'a str>,
    pub message: Option<&'a str>,
}

pub fn log_directory(data_directory: &Path) -> PathBuf {
    data_directory.join(LOG_DIRECTORY_NAME)
}

pub fn append_event(data_directory: &Path, event: LogEvent<'_>) {
    let directory = log_directory(data_directory);
    let path = directory.join(LOG_FILE_NAME);
    let previous_path = directory.join(PREVIOUS_LOG_FILE_NAME);

    let Ok(()) = fs::create_dir_all(&directory) else {
        return;
    };

    if fs::metadata(&path)
        .map(|metadata| metadata.len() >= MAX_LOG_BYTES)
        .unwrap_or(false)
    {
        if fs::rename(&path, &previous_path).is_err() {
            let _ = fs::write(&path, "");
        }
    }

    let value = json!({
        "at": Utc::now().to_rfc3339(),
        "correlationId": event.correlation_id,
        "event": event.event,
        "level": event.level.as_str(),
        "message": event.message.map(sanitize_message),
        "subsystem": event.subsystem,
    });
    let Ok(serialized) = serde_json::to_string(&value) else {
        return;
    };

    // A failed diagnostic write must not interrupt presentation playback.
    use std::io::Write;
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{serialized}");
    }
}

pub fn recent_entries(data_directory: &Path) -> Vec<Value> {
    let directory = log_directory(data_directory);
    let combined = [
        directory.join(PREVIOUS_LOG_FILE_NAME),
        directory.join(LOG_FILE_NAME),
    ]
    .into_iter()
    .filter_map(|path| fs::read_to_string(path).ok())
    .collect::<Vec<_>>()
    .join("\n");

    let start = combined.len().saturating_sub(MAX_RECENT_BYTES);
    combined[start..]
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| sanitize_log_entry(&value))
        .collect()
}

pub fn error_summary(logs: &[Value]) -> Value {
    let mut summary = BTreeMap::<String, u64>::new();
    for entry in logs {
        if entry.get("level").and_then(Value::as_str) != Some("error") {
            continue;
        }
        let subsystem = entry
            .get("subsystem")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let event = entry
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        *summary.entry(format!("{subsystem}:{event}")).or_default() += 1;
    }
    json!(summary)
}

/// Keep this allowlist aligned with the renderer diagnostics contract. In
/// particular, do not include workspace paths, screen target IDs, passwords,
/// or arbitrary renderer settings in a support bundle.
pub fn sanitize_settings(settings: &Value) -> Value {
    let general = settings
        .get("generalSettings")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    json!({
        "appLanguage": string_setting(general.get("appLanguage")),
        "appTheme": string_setting(general.get("appTheme")),
        "textScale": string_setting(general.get("textScale")),
        "uiDensity": string_setting(general.get("uiDensity")),
        "useContinuousPlaylist": general
            .get("useContinuousPlaylist")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "remoteEnabled": settings
            .get("remoteSettings")
            .and_then(Value::as_object)
            .and_then(|remote| remote.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "audioConfigured": settings
            .get("audioSettings")
            .is_some_and(|value| value.is_object()),
    })
}

pub fn build_bundle(
    version: &str,
    runtime: &str,
    settings: &Value,
    ndi_status: &Value,
    data_directory: &Path,
) -> Value {
    let logs = recent_entries(data_directory);
    // Match the established support bundle contract: messages are useful while
    // inspecting the local rotating log, but are intentionally omitted from
    // an exported bundle. This prevents an otherwise-unrecognized renderer
    // error from carrying presentation text or another private value out of
    // the application.
    let exported_logs = logs.iter().map(export_log_entry).collect::<Vec<_>>();
    let ndi_available = ndi_status
        .get("ndiRuntimeAvailable")
        .or_else(|| ndi_status.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let ndi_error = ndi_status
        .get("ndiError")
        .and_then(Value::as_str)
        .filter(|error| !error.trim().is_empty())
        .map(|_| "available-but-unusable");

    json!({
        "format": 1,
        "createdAt": Utc::now().to_rfc3339(),
        "app": {
            "version": version,
            "runtime": runtime,
        },
        "operatingSystem": {
            "platform": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
        },
        "runtime": {
            "ndiAvailable": ndi_available,
            "ndiError": ndi_error,
        },
        "settings": sanitize_settings(settings),
        "errorSummary": error_summary(&logs),
        "logs": exported_logs,
        "privacy": {
            "excluded": [
                "workspace content",
                "presentation text",
                "media paths",
                "credentials",
                "remote password",
            ],
            "workspaceContentIncluded": false,
        },
    })
}

fn export_log_entry(value: &Value) -> Value {
    let object = value.as_object().cloned().unwrap_or_default();
    ["at", "correlationId", "event", "level", "subsystem"]
        .into_iter()
        .filter_map(|key| {
            object
                .get(key)
                .cloned()
                .map(|value| (key.to_string(), value))
        })
        .collect::<Map<_, _>>()
        .into()
}

fn string_setting(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or("")
}

fn sanitize_log_entry(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let mut sanitized = Map::new();
    for key in ["at", "correlationId", "event", "level", "subsystem"] {
        if let Some(value) = object.get(key) {
            sanitized.insert(key.to_string(), value.clone());
        }
    }
    if let Some(message) = object.get("message").and_then(Value::as_str) {
        sanitized.insert("message".to_string(), json!(sanitize_message(message)));
    }
    Some(Value::Object(sanitized))
}

fn sanitize_message(message: &str) -> String {
    let mut sanitized = String::with_capacity(message.len());
    let mut token = String::new();
    for character in message.chars() {
        if character.is_whitespace() || matches!(character, '"' | '\'') {
            if !token.is_empty() {
                sanitized.push_str(&sanitize_token(&token));
                token.clear();
            }
            sanitized.push(character);
        } else {
            token.push(character);
        }
    }
    if !token.is_empty() {
        sanitized.push_str(&sanitize_token(&token));
    }
    sanitized
}

fn sanitize_token(token: &str) -> String {
    let looks_like_windows_path = token.len() >= 3
        && token.as_bytes()[1] == b':'
        && matches!(token.as_bytes()[2], b'\\' | b'/');
    let looks_like_unix_path = token.starts_with('/') && token.len() > 1;
    if looks_like_windows_path || looks_like_unix_path {
        "[redacted-path]".to_string()
    } else {
        token.to_string()
    }
}
