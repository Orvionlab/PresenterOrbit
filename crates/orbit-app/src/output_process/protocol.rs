//! The messages between the UI process and the output process, and the endpoint file that lets a UI find a
//! running output process. Every message is one length-framed JSON document.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use crate::native_outputs::framing::{read_packet, write_packet};

pub const VERSION: u32 = 1;
/// The largest message either side accepts. A scene snapshot with inline assets is the biggest legitimate one.
pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;
pub const ENDPOINT_FILE: &str = "endpoint.json";
pub const LOCK_FILE: &str = "output.lock";

/// The host calls the UI may make on the output process. Everything else is an error.
pub const INVOKE_ALLOWED: &[&str] = &[
    "updateOutputScene",
    "setDisplayOutputVisibility",
    "createWindowedOutput",
    "focusWindowedOutput",
    "listWindowedOutputs",
    "closeDisplayOutput",
    "getOutputOptions",
    "getNdiStatus",
    "getOutputContent",
    // Read-only preparation: decoded assets stay in bounded process memory.
    "prepareMedia",
    "getMediaPreparationStatus",
    "getNativeMediaPlayback",
    "getNativeMusicPlayback",
    "getAudioInputStatus",
    // Read-only: the live per-channel level of the program audio, which plays in this process.
    "getProgramAudioLevels",
    // Read-only: the marker positions the output process measured, for the UI scheduler (phase 15).
    "getMarkerPositions",
    // The playback clock lives here, so the transport controls cross the link. Never a marker jump or a command
    // that names a workspace: both read the workspace, which stays in the UI process (`transport_command_allowed`).
    "controlPlayback",
    "acceptLinkedServiceState",
    // Recording runs where the program picture and the program audio are, so these cross the link. The preset
    // and stream key arrive already resolved; the output process reads no workspace and writes no settings.
    "startCaptureResolved",
    "stopCapture",
    "getCaptureStatus",
];

/// `controlPlayback` is allowed for the clock commands only. A marker jump resolves the marker from the
/// workspace, and a workspace path makes the host read it; the output process reads no workspace.
pub fn transport_command_allowed(command: &serde_json::Value) -> bool {
    command["action"] != "marker" && command.get("workspacePath").is_none()
}

/// The only host events the output process forwards. Others, such as `automation-due`, belong to the UI model.
pub const EVENTS_FORWARDED: &[&str] = &[
    "output:displays-changed",
    "output:display-error",
    "output-closed",
    "presenter-orbit:music-playback-ended",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToOutput {
    Hello {
        version: u32,
        token: String,
        controller_pid: u32,
        rendering_bit_depth: String,
    },
    Invoke {
        id: u64,
        method: String,
        args: Vec<Value>,
    },
    Ping,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FromOutput {
    Welcome {
        version: u32,
        pid: u32,
        has_scene: bool,
    },
    Busy {
        controller_pid: u32,
    },
    Reject {
        reason: String,
    },
    Result {
        id: u64,
        #[serde(default)]
        ok: Option<Value>,
        #[serde(default)]
        error: Option<String>,
    },
    Event {
        event: String,
        payload: Value,
    },
    Pong,
}

pub fn write_message(writer: &mut impl Write, message: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(message).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_MESSAGE {
        return Err("An output process message exceeds its size limit.".into());
    }
    write_packet(writer, &bytes)?;
    writer.flush().map_err(|error| error.to_string())
}

pub fn read_message<T: DeserializeOwned>(reader: &mut impl Read) -> Result<T, String> {
    let bytes = read_packet(reader, MAX_MESSAGE)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("The output process sent an unreadable message: {error}"))
}

/// Where a running output process listens. Written once the socket is bound, deleted when the process exits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub protocol_version: u32,
}

/// The output process data directory inside the UI process data directory.
pub fn data_directory(ui_data: &Path) -> PathBuf {
    ui_data.join("output-process")
}

pub fn write_endpoint(directory: &Path, endpoint: &Endpoint) -> Result<(), String> {
    let temporary = directory.join(format!("{ENDPOINT_FILE}.{}.tmp", endpoint.pid));
    std::fs::write(
        &temporary,
        serde_json::to_vec(endpoint).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("The output endpoint could not be written: {error}"))?;
    std::fs::rename(&temporary, directory.join(ENDPOINT_FILE))
        .map_err(|error| format!("The output endpoint could not be published: {error}"))
}

/// `None` when no endpoint file exists. An unreadable file is an error, not an absent output process.
pub fn read_endpoint(directory: &Path) -> Result<Option<Endpoint>, String> {
    match std::fs::read(directory.join(ENDPOINT_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| format!("The output endpoint file is unreadable: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("The output endpoint file is unreadable: {error}")),
    }
}

pub fn remove_endpoint(directory: &Path) {
    let _ = std::fs::remove_file(directory.join(ENDPOINT_FILE));
}
