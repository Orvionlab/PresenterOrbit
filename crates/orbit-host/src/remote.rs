use crate::Host;
use chrono::{SecondsFormat, Utc};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{TcpStream, UdpSocket},
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{
    output_arrangement::{live_slide_index, resolve_live_arrangement_slides},
    output_scene::{OutputSceneStateInput, OutputScreenConfigInput},
};

const DEFAULT_PORT: u16 = 49_321;
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const REQUEST_READ_CHUNK_BYTES: usize = 4096;
const CORS_HEADERS: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Authorization, Content-Type\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\n";

#[path = "remote_events.rs"]
mod remote_events;
use remote_events::EventHub;
#[path = "remote_mcp.rs"]
mod remote_mcp;
#[path = "remote_pro7.rs"]
mod remote_pro7;
use remote_pro7::Pro7Settings;

#[derive(Debug, PartialEq, Eq)]
enum RequestReadError {
    Io,
    Malformed,
    TooLarge,
    Incomplete,
}

#[derive(Debug, PartialEq, Eq)]
struct HttpRequest {
    method: String,
    path: String,
    authorization: String,
    host: String,
    has_origin: bool,
    body: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
struct HttpRequestHead {
    method: String,
    path: String,
    authorization: String,
    host: String,
    has_origin: bool,
    content_length: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RemoteSettings {
    enabled: bool,
    port: u16,
    password: String,
    mcp: remote_mcp::Settings,
    pro7: Pro7Settings,
    tcp_enabled: bool,
    tcp_port: u16,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            password: String::new(),
            mcp: remote_mcp::Settings::from_value(&Value::Null),
            pro7: Pro7Settings::default(),
            tcp_enabled: false,
            tcp_port: 49323,
        }
    }
}

struct SharedState {
    settings: RemoteSettings,
    status: Value,
    commands: VecDeque<Value>,
}

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_COMMAND_RESULTS: usize = 100;

const MAX_REPORTED_FAILURES: usize = 100;

fn validate_failures(failures: &Value) -> Result<Value, String> {
    let entries = failures
        .as_array()
        .filter(|entries| !entries.is_empty() && entries.len() <= MAX_REPORTED_FAILURES)
        .ok_or("Macro failures must be a nonempty bounded array.")?;
    let mut accepted = Vec::with_capacity(entries.len());
    for entry in entries {
        let field = |name: &str| {
            entry
                .get(name)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| format!("Macro failure {name} is required."))
        };
        accepted.push(json!({"actionId":field("actionId")?,"type":field("type")?,"message":field("message")?}));
    }
    Ok(Value::Array(accepted))
}

fn resolve_command_result(mut result: Value, workspace_path: Option<&str>, now_ms: i64) -> Value {
    if result["status"] == "dispatched"
        && result["command"] == "macro"
        && workspace_path != result["workspacePath"].as_str()
    {
        result["status"] = json!("error");
        result["ok"] = json!(false);
        result["error"] = json!("Workspace changed before macro completion was confirmed.");
        return result;
    }
    if result["status"] == "queued" && workspace_path != result["workspacePath"].as_str() {
        result["status"] = json!("error");
        result["ok"] = json!(false);
        result["error"] = json!("Workspace changed before execution was confirmed.");
    } else if result["status"] == "queued"
        && result["acceptedAtMs"]
            .as_i64()
            .is_some_and(|time| now_ms - time > 30_000)
    {
        result["status"] = json!("unconfirmed");
        result["ok"] = json!(false);
        result["error"] = json!(
            "Execution has not been confirmed within 30 seconds; the command is not retried."
        );
    }
    result
}

fn uses_native_command_path(command: &str) -> bool {
    matches!(
        command,
        "clearAll"
            | "clearLayer"
            | "nextSlide"
            | "previousSlide"
            | "timer"
            | "prop"
            | "capture"
            | "cue"
            | "libraryCue"
            | "playlistCue"
            | "stageMessage"
            | "media"
            | "ticker"
            | "timelineTransport"
    )
}

fn native_command_outcome(result: &Value) -> CommandOutcome {
    if result.get("ok").and_then(Value::as_bool) == Some(true) {
        CommandOutcome::Applied
    } else if result.get("status").and_then(Value::as_str) == Some("invalid-target") {
        CommandOutcome::Invalid
    } else {
        CommandOutcome::Unavailable
    }
}

pub struct RemoteServer {
    shared: Arc<Mutex<SharedState>>,
    hub: Arc<EventHub>,
    pro7: remote_pro7::Pro7Server,
    mcp: remote_pro7::Pro7Server,
    tcp: remote_pro7::tcp::TcpServer,
}

#[derive(Clone)]
pub struct RemoteStatusHandle {
    shared: Arc<Mutex<SharedState>>,
    hub: Arc<EventHub>,
}

impl RemoteStatusHandle {
    pub fn update(&self, status: Value) -> Result<(), String> {
        update_shared_status(&self.shared, status)?;
        self.hub.notify_status_changed();
        Ok(())
    }

    pub fn report(&self, result: &Value) -> Result<Value, String> {
        let mut shared = self
            .shared
            .lock()
            .map_err(|_| "Remote state is unavailable.")?;
        let request_id = result
            .get("requestId")
            .and_then(Value::as_str)
            .ok_or("A request id is required.")?;
        let entry = shared
            .commands
            .iter_mut()
            .find(|entry| entry["requestId"].as_str() == Some(request_id))
            .ok_or("Remote request is unavailable or expired.")?;
        if entry["workspacePath"] != result["workspacePath"] {
            return Err("Remote result workspace does not match.".into());
        }
        let status = result
            .get("status")
            .and_then(Value::as_str)
            .ok_or("A result status is required.")?;
        if !matches!(status, "applied" | "dispatched" | "completed" | "error") {
            return Err("Invalid remote result status.".into());
        }
        let command = entry["command"].as_str();
        let macro_dispatched = command == Some("macro") && entry["status"] == "dispatched";
        if entry["status"] != "queued"
            && !(macro_dispatched && matches!(status, "completed" | "error"))
        {
            return Err("Remote request already has a result.".into());
        }
        if (status == "applied"
            && !matches!(
                command,
                Some("audienceLook" | "stageLayout" | "timerDefinition" | "messageDefinition")
            ))
            || (status == "dispatched"
                && !matches!(command, Some("macro" | "message" | "prop" | "clearGroup")))
            || (status == "completed" && command != Some("macro"))
        {
            return Err("Remote result does not match the command execution contract.".into());
        }
        let failures = match result.get("failures") {
            None => None,
            Some(failures) if status == "error" && command == Some("macro") => {
                Some(validate_failures(failures)?)
            }
            Some(_) => return Err("Failures are only accepted with a macro error result.".into()),
        };
        let error = if status == "error" {
            Some(
                result
                    .get("error")
                    .and_then(Value::as_str)
                    .filter(|error| !error.is_empty())
                    .ok_or("A runtime error is required.")?,
            )
        } else {
            None
        };
        entry["status"] = json!(status);
        entry["ok"] = json!(status != "error");
        if let Some(error) = error {
            entry["error"] = json!(error);
        }
        if let Some(failures) = failures {
            entry["failures"] = failures;
        }
        self.hub.notify_status_changed();
        Ok(Value::Null)
    }

    pub(crate) fn dispatch_network_link_command(
        &self,
        app: &Host,
        payload: &Value,
    ) -> Result<Value, String> {
        let command = payload
            .get("command")
            .and_then(Value::as_str)
            .filter(|command| {
                matches!(
                    *command,
                    "macro"
                        | "message"
                        | "prop"
                        | "stageLayout"
                        | "timerDefinition"
                        | "messageDefinition"
                )
            })
            .ok_or("Unsupported Network Link renderer command.")?;
        let state = app.state::<crate::AppState>();
        let path = state
            .active_workspace
            .lock()
            .map_err(|_| "Workspace state is unavailable.")?
            .clone()
            .ok_or("An active workspace is required.")?;
        if payload.get("workspacePath").and_then(Value::as_str) != path.to_str() {
            return Err("The Network Link command belongs to a different workspace.".into());
        }
        {
            let workspace_io = crate::lock_workspace_io(&state)?;
            workspace_io.require_active(&path)?;
            let workspace = crate::workspace::read_workspace(&path)?;
            crate::remote_controls::validate_target(&workspace, payload)?;
        }

        let request_id = format!(
            "{}-{}",
            Utc::now().timestamp_millis(),
            REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let mut event = payload.clone();
        event["requestId"] = json!(request_id);
        let queued = json!({
            "ok":true,
            "status":"queued",
            "requestId":request_id,
            "workspacePath":payload["workspacePath"],
            "command":command,
            "id":payload.get("id").cloned().unwrap_or(Value::Null),
            "acceptedAtMs":Utc::now().timestamp_millis()
        });
        {
            let mut shared = self
                .shared
                .lock()
                .map_err(|_| "Remote state is unavailable.")?;
            if shared.commands.len() >= MAX_COMMAND_RESULTS {
                shared.commands.pop_front();
            }
            shared.commands.push_back(queued);
        }
        self.hub.notify_status_changed();
        if let Err(error) = app.emit_to("main", "remote-command", event) {
            let report = json!({
                "requestId":request_id,
                "workspacePath":payload["workspacePath"],
                "status":"error",
                "error":error.to_string()
            });
            let _ = self.report(&report);
            return Err("Network Link command could not be dispatched to the renderer.".into());
        }

        let deadline = std::time::Instant::now() + Duration::from_secs(35);
        loop {
            let active = state
                .active_workspace
                .lock()
                .map_err(|_| "Workspace state is unavailable.")?
                .clone();
            if active.as_deref() != Some(path.as_path()) {
                return Err(
                    "Workspace changed before the Network Link command was confirmed.".into(),
                );
            }
            let result = self
                .shared
                .lock()
                .map_err(|_| "Remote state is unavailable.")?
                .commands
                .iter()
                .find(|entry| entry["requestId"].as_str() == Some(request_id.as_str()))
                .cloned()
                .ok_or("Network Link command result expired before confirmation.")?;
            match result.get("status").and_then(Value::as_str) {
                Some("error") => {
                    return Err(result["error"]
                        .as_str()
                        .unwrap_or("Network Link command failed.")
                        .to_string())
                }
                Some("applied" | "dispatched" | "completed") => {
                    if command == "timerDefinition" {
                        let timer_id = payload["id"].as_str().ok_or("A timer id is required.")?;
                        let workspace = {
                            let workspace_io = crate::lock_workspace_io(&state)?;
                            workspace_io.require_active(&path)?;
                            crate::workspace::read_workspace(&path)?
                        };
                        let output = crate::lock_output_snapshot_state(&state.output_snapshot)?;
                        let accepted_timer = output
                            .cache
                            .as_ref()
                            .and_then(|cache| cache.snapshot.get("countdownTimers"))
                            .and_then(Value::as_array)
                            .and_then(|timers| {
                                timers.iter().find(|timer| {
                                    timer.get("id").and_then(Value::as_str) == Some(timer_id)
                                })
                            });
                        if accepted_timer.is_some_and(|timer| {
                            crate::validate_current_timer_definition(&workspace, timer).is_ok()
                        }) {
                            return Ok(json!({
                                "ok":true,
                                "status":result["status"],
                                "requestId":request_id,
                                "changed":result.get("changed").and_then(Value::as_bool).unwrap_or(true)
                            }));
                        }
                    } else {
                        return Ok(json!({
                            "ok":true,
                            "status":result["status"],
                            "requestId":request_id,
                            "changed":result.get("changed").and_then(Value::as_bool).unwrap_or(true)
                        }));
                    }
                }
                Some("queued") => {}
                _ => return Err("Network Link command returned an invalid confirmation.".into()),
            }
            if std::time::Instant::now() >= deadline {
                return Err(
                    "Network Link command confirmation timed out; it was not retried.".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

/// Build the public remote status from the exact renderer snapshot accepted by
/// the native output-scene cache. Remote status must not be assembled from a
/// second renderer-side state projection: doing so can expose a different
/// slide, screen look, background, or audio selection than the native output.
pub fn build_output_remote_status(snapshot: &Value) -> Result<Value, String> {
    let input: OutputSceneStateInput =
        crate::output_scene::parse_scene_state(snapshot).map_err(|error| {
            format!("Output scene state does not match the remote status contract: {error}")
        })?;

    let live_item = input.live_item.as_ref().filter(|item| item.is_object());
    let slides = live_item
        .and_then(|item| {
            resolve_live_arrangement_slides(
                item,
                input.live_cue_instance_id.as_deref(),
                input.live_playlist_entry_id.as_deref(),
            )
        })
        .map(|slides| {
            slides
                .into_iter()
                .filter(|slide| !json_truthy(slide.get("disabled")))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let slide_id = input.live_slide_id.as_deref();
    let slide_index = slide_id
        .and_then(|id| live_slide_index(&slides, id, input.live_cue_instance_id.as_deref()));
    let current_slide =
        slide_index.and_then(|index| slides.get(index).map(|slide| slide_summary(slide)));
    let previous_slide = slide_index
        .filter(|index| *index > 0)
        .and_then(|index| slides.get(index - 1).map(|slide| slide_summary(slide)));
    let next_slide = slide_index
        .filter(|index| *index + 1 < slides.len())
        .and_then(|index| slides.get(index + 1).map(|slide| slide_summary(slide)));

    let presentation = live_item.map(|item| {
        json!({
            "id": string_field(item, "id", ""),
            "title": string_field(item, "title", ""),
            "slideIndex": slide_index.map(|index| index + 1),
            "slideCount": slides.len(),
            "playlistEntryId": input.live_playlist_entry_id,
            "arrangementId": item.get("selectedArrangementId"),
            "slides": slides
                .iter()
                .enumerate()
                .map(|(index, slide)| {
                    json!({
                        "id": string_field(slide, "id", ""),
                        "index": index + 1,
                        "cueInstanceId": slide.get("arrangementInstanceId"),
                        "title": string_field(slide, "label", ""),
                        "text": string_field(slide, "content", ""),
                        "notes": string_field(slide, "notes", ""),
                        "disabled": json_truthy(slide.get("disabled")),
                    })
                })
                .collect::<Vec<_>>(),
        })
    });

    let outputs = input
        .screen_config
        .iter()
        .map(|(id, config)| {
            let layers = screen_look_layers(config);
            json!({
                "id": id,
                "label": config
                    .label
                    .as_deref()
                    .unwrap_or(id),
                "type": config.screen_type.as_deref().unwrap_or("audience"),
                "stageLayoutId": (config.screen_type.as_deref() == Some("stage")).then(|| {
                    config
                        .stage_layout_id
                        .clone()
                        .filter(|layout| !layout.is_empty())
                        .or_else(|| {
                            input
                                .stage_screen_assignments
                                .iter()
                                .find(|assignment| assignment["screenId"].as_str() == Some(id.as_str()))
                                .and_then(|assignment| assignment["layoutId"].as_str())
                                .map(str::to_string)
                        })
                }),
                "active": json_truthy(config._other.get("enabled")),
                "targetType": string_value(config._other.get("targetType"), "display"),
                "layers": layers,
                "presentationThemeId": config.look_themes.as_ref().and_then(|value| value.get("presentationThemeId")),
                "maskId": config.look_themes.as_ref().and_then(|value| value.get("maskId")),
            })
        })
        .collect::<Vec<_>>();
    let layers = input
        .screen_config
        .iter()
        .map(|(id, config)| (id.clone(), screen_look_layers(config)))
        .collect::<serde_json::Map<_, _>>();
    let audio_id = input.audio_id.as_deref();
    let paused = input
        .transport_playback_paused
        .as_ref()
        .and_then(|values| values.get("music"))
        .copied()
        .unwrap_or(false);

    Ok(json!({
        "version": 1,
        "updatedAt": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "presentation": presentation,
        "activeAudienceLookId": input.active_audience_look_id,
        "focusedPresentation": input.focused_presentation,
        "focusedPlaylistId": input.focused_playlist_id,
        "announcements": input.announcements,
        "timelineTransport": input.timeline_transport,
        "livePlaylistId": input.live_playlist_id,
        "stageScreenAssignments": input.stage_screen_assignments,
        "liveVideoInputId": input.live_video_input_id,
        "currentSlide": current_slide,
        "previousSlide": previous_slide,
        "nextSlide": next_slide,
        "outputs": outputs,
        "timers": input.countdown_timers,
        "timerRuns": input.countdown_timer_runs,
        "stageMessage": snapshot.get("stageMessage"),
        "activeMessage": snapshot
            .get("activeMessageId")
            .and_then(Value::as_str)
            .map(|id| json!({ "id": id, "instanceId": snapshot.get("activeMessageInstanceId") })),
        "props": input.props,
        "livePropIds": snapshot.get("livePropIds"),
        "layers": layers,
        "background": {
            "id": input.background_id.clone(),
            "selectionId":snapshot.get("backgroundSelectionId"),
            "paused":snapshot.get("transportPlaybackPaused").and_then(|value|value.get("media")).and_then(Value::as_bool).unwrap_or(false),
            "playback": input.background_playback.as_deref().unwrap_or("stop"),
            "playbackState": snapshot.get("backgroundPlaybackState"),
        },
        "audio": {
            "id": audio_id,
            "selectionId":snapshot.get("audioSelectionId"),
            "paused": paused,
        },
    }))
}

fn slide_summary(slide: &Value) -> Value {
    json!({
        "id": string_field(slide, "id", ""),
        "cueInstanceId": slide.get("arrangementInstanceId"),
        "title": string_field(slide, "label", ""),
        "text": string_field(slide, "content", ""),
        "notes": string_field(slide, "notes", ""),
    })
}

fn string_field(value: &Value, field: &str, fallback: &str) -> String {
    string_value(value.get(field), fallback)
}

fn string_value(value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn json_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
    }
}

fn screen_look_layers(config: &OutputScreenConfigInput) -> Value {
    let defaults = [
        ("music", true),
        ("messages", true),
        ("props", true),
        ("announcements", true),
        ("slide", true),
        ("media", true),
        ("videoInput", true),
    ];
    let mut layers = serde_json::Map::new();
    for (name, default) in defaults {
        layers.insert(
            name.to_string(),
            config
                .look_layers
                .as_ref()
                .and_then(|values| values.get(name))
                .cloned()
                .unwrap_or_else(|| Value::Bool(default)),
        );
    }
    if let Some(values) = config.look_layers.as_ref() {
        for (name, value) in values {
            layers.entry(name.clone()).or_insert_with(|| value.clone());
        }
    }
    Value::Object(layers)
}

impl RemoteServer {
    pub(crate) fn link_connection(&self) -> Result<Option<(u16, String)>, String> {
        let shared = self
            .shared
            .lock()
            .map_err(|_| "Remote server state is unavailable.")?;
        Ok((self.pro7.is_running()
            && shared.settings.enabled
            && shared.settings.pro7.remote_enabled
            && !shared.settings.password.is_empty())
        .then(|| (shared.settings.port, shared.settings.password.clone())))
    }

    pub fn new() -> Self {
        Self {
            shared: Arc::new(Mutex::new(SharedState {
                settings: RemoteSettings::default(),
                status: json!({}),
                commands: VecDeque::new(),
            })),
            hub: Arc::new(EventHub::new()),
            pro7: remote_pro7::Pro7Server::new(),
            mcp: remote_pro7::Pro7Server::new(),
            tcp: remote_pro7::tcp::TcpServer::new(),
        }
    }

    pub fn status_handle(&self) -> RemoteStatusHandle {
        RemoteStatusHandle {
            shared: Arc::clone(&self.shared),
            hub: Arc::clone(&self.hub),
        }
    }

    pub fn update_settings(&mut self, value: &Value, app: &Host) -> Result<Value, String> {
        validate_settings_value(value)?;
        let next = normalize_settings(value);
        let previous = self
            .shared
            .lock()
            .map_err(|_| "Remote server state is unavailable.".to_string())?
            .settings
            .clone();
        self.shared
            .lock()
            .map_err(|_| "Remote server state is unavailable.")?
            .settings = next.clone();
        let backend = |mcp_only| {
            Arc::new(remote_pro7::AppBackend {
                app: app.clone(),
                shared: Arc::clone(&self.shared),
                hub: Arc::clone(&self.hub),
                mcp_only,
            }) as Arc<dyn remote_pro7::Backend>
        };
        let restart_network = previous.pro7 != next.pro7
            || previous.tcp_enabled != next.tcp_enabled
            || previous.tcp_port != next.tcp_port
            || (next.enabled && !self.pro7.is_running());
        let restart_mcp = previous.mcp.enabled != next.mcp.enabled
            || previous.mcp.port != next.mcp.port
            || (next.mcp.enabled && !self.mcp.is_running());
        if restart_mcp {
            self.mcp.stop();
        }
        if restart_network {
            self.pro7.stop();
            self.tcp.stop();
            self.hub.stop_streams();
        }
        let network_result = if restart_network && next.enabled {
            if next.pro7.remote_enabled {
                // Prepare thumbnail resources before the client starts requesting slides.
                drop(crate::native_render::NativeRenderRuntime::new_static_preview());
            }
            let network = backend(false);
            self.pro7
                .start(Arc::clone(&network), &next.pro7)
                .and_then(|()| {
                    if next.tcp_enabled {
                        if let Err(error) = self.tcp.start(network, next.tcp_port) {
                            self.pro7.stop();
                            return Err(error);
                        }
                    }
                    Ok(())
                })
        } else {
            Ok(())
        };
        let mcp_result = if restart_mcp && next.mcp.enabled {
            self.mcp.start_local_mcp(backend(true), next.mcp.port)
        } else {
            Ok(())
        };
        network_result?;
        mcp_result?;
        Ok(remote_status(&next, None))
    }

    pub fn stop(&mut self) {
        self.pro7.stop();
        self.mcp.stop();
        self.tcp.stop();
        self.hub.stop_streams();
    }
}

fn update_shared_status(shared: &Arc<Mutex<SharedState>>, status: Value) -> Result<(), String> {
    let mut shared = shared
        .lock()
        .map_err(|_| "Remote server state is unavailable.".to_string())?;
    shared.status = if status.is_object() {
        status
    } else {
        json!({})
    };
    Ok(())
}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        self.pro7.stop();
        self.stop();
    }
}

pub fn network_address() -> Result<String, String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|error| error.to_string())?;
    // Select the ordinary unicast route; multicast routes may prefer a virtual adapter.
    socket
        .connect("192.0.2.1:9")
        .map_err(|error| format!("The local network address is unavailable: {error}"))?;
    let address = socket.local_addr().map_err(|error| error.to_string())?.ip();
    if address.is_unspecified() || address.is_loopback() {
        return Err("No local network address is available.".into());
    }
    Ok(address.to_string())
}

fn normalize_settings(value: &Value) -> RemoteSettings {
    let object = value.as_object();
    RemoteSettings {
        enabled: object
            .and_then(|value| value.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        port: port_setting(object, "port", DEFAULT_PORT),
        password: object
            .and_then(|value| value.get("password"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        mcp: remote_mcp::Settings::from_value(value),
        pro7: Pro7Settings {
            enabled: object
                .and_then(|value| value.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            network_name: object
                .and_then(|value| value.get("networkName"))
                .and_then(Value::as_str)
                .unwrap_or("Presenter Orbit")
                .to_string(),
            remote_enabled: object
                .and_then(|value| value.get("proPresenterEnabled"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            stage_enabled: object
                .and_then(|value| value.get("stageEnabled"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            port: port_setting(object, "port", DEFAULT_PORT),
            remote_password: string_setting(object, "password"),
            stage_password: string_setting(object, "proPresenterStagePassword"),
        },
        tcp_enabled: object
            .and_then(|value| value.get("tcpEnabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        tcp_port: port_setting(object, "tcpPort", 49323),
    }
}

fn port_setting(object: Option<&serde_json::Map<String, Value>>, field: &str, default: u16) -> u16 {
    object
        .and_then(|value| value.get(field))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .filter(|port| (1024..=65_535).contains(port))
        .map_or(default, |port| port as u16)
}

fn string_setting(object: Option<&serde_json::Map<String, Value>>, field: &str) -> String {
    object
        .and_then(|value| value.get(field))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn validate_settings_value(value: &Value) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    if let Some(name) = object.get("networkName") {
        let name = name.as_str().ok_or("Network name must be text.")?;
        if name.trim().is_empty() || name.len() > 63 || name.chars().any(char::is_control) {
            return Err(
                "A network name of 1–63 bytes without control characters is required.".into(),
            );
        }
    }
    for field in ["port", "tcpPort", "mcpPort"] {
        if object.contains_key(field) && port_setting(Some(object), field, 0) == 0 {
            return Err(format!("{field} must be a port between 1024 and 65535."));
        }
    }
    for field in [
        "mcpEnabled",
        "mcpReadStatus",
        "mcpReadCatalog",
        "mcpCreatePresentations",
        "mcpManagePlaylists",
    ] {
        if object.get(field).is_some_and(|value| !value.is_boolean()) {
            return Err(format!("{field} must be a boolean."));
        }
    }
    if object
        .get("mcpPassword")
        .is_some_and(|value| !value.is_string())
    {
        return Err("MCP password must be text.".into());
    }
    let settings = normalize_settings(value);
    if settings.enabled && settings.tcp_enabled && settings.port == settings.tcp_port {
        return Err("The network and TCP/IP ports must be different.".into());
    }
    if settings.enabled
        && settings.mcp.enabled
        && (settings.mcp.port == settings.port
            || (settings.tcp_enabled && settings.mcp.port == settings.tcp_port))
    {
        return Err("The MCP port must differ from the enabled network ports.".into());
    }
    Ok(())
}

pub fn normalize_settings_value(value: &Value) -> Value {
    let settings = normalize_settings(value);
    json!({
        "enabled": settings.enabled,
        "port": settings.port,
        "password": settings.password,
        "mcpEnabled": settings.mcp.enabled,
        "mcpPort": settings.mcp.port,
        "mcpPassword": settings.mcp.password,
        "mcpReadStatus": settings.mcp.read_status,
        "mcpReadCatalog": settings.mcp.read_catalog,
        "mcpCreatePresentations": settings.mcp.create_presentations,
        "mcpManagePlaylists": settings.mcp.manage_playlists,
        "networkName": settings.pro7.network_name,
        "proPresenterEnabled": settings.pro7.remote_enabled,
        "stageEnabled": settings.pro7.stage_enabled,
        "tcpEnabled": settings.tcp_enabled,
        "tcpPort": settings.tcp_port,
        "proPresenterStagePassword": settings.pro7.stage_password,
    })
}

fn remote_status(settings: &RemoteSettings, error: Option<&str>) -> Value {
    json!({
        "enabled": settings.enabled,
        "port": settings.port,
        "passwordConfigured": !settings.password.is_empty(),
        "running": settings.enabled && error.is_none(),
        "error": error,
    })
}

fn read_http_request<R: Read>(reader: &mut R) -> Result<HttpRequest, RequestReadError> {
    let mut buffer = Vec::with_capacity(REQUEST_READ_CHUNK_BYTES);
    let header_end = loop {
        if let Some(index) = find_header_end(&buffer) {
            break index;
        }
        if buffer.len() >= MAX_REQUEST_BYTES {
            return Err(RequestReadError::TooLarge);
        }
        let read_size = (MAX_REQUEST_BYTES - buffer.len()).min(REQUEST_READ_CHUNK_BYTES);
        let mut chunk = vec![0_u8; read_size];
        let length = reader.read(&mut chunk).map_err(|_| RequestReadError::Io)?;
        if length == 0 {
            return Err(RequestReadError::Incomplete);
        }
        buffer.extend_from_slice(&chunk[..length]);
    };

    let head = parse_request_head(&buffer[..header_end - 4])?;
    let total_length = header_end
        .checked_add(head.content_length)
        .ok_or(RequestReadError::TooLarge)?;
    if total_length > MAX_REQUEST_BYTES {
        return Err(RequestReadError::TooLarge);
    }

    while buffer.len() < total_length {
        let read_size = (total_length - buffer.len()).min(REQUEST_READ_CHUNK_BYTES);
        let mut chunk = vec![0_u8; read_size];
        let length = reader.read(&mut chunk).map_err(|_| RequestReadError::Io)?;
        if length == 0 {
            return Err(RequestReadError::Incomplete);
        }
        buffer.extend_from_slice(&chunk[..length]);
    }

    Ok(HttpRequest {
        method: head.method,
        path: head.path,
        authorization: head.authorization,
        host: head.host,
        has_origin: head.has_origin,
        body: buffer[header_end..total_length].to_vec(),
    })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn parse_request_head(head: &[u8]) -> Result<HttpRequestHead, RequestReadError> {
    let head = std::str::from_utf8(head).map_err(|_| RequestReadError::Malformed)?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next().ok_or(RequestReadError::Malformed)?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .filter(|method| !method.is_empty())
        .ok_or(RequestReadError::Malformed)?
        .to_string();
    let path = request_parts
        .next()
        .filter(|path| !path.is_empty())
        .ok_or(RequestReadError::Malformed)?
        .to_string();
    let version = request_parts.next().ok_or(RequestReadError::Malformed)?;
    if request_parts.next().is_some() || !version.starts_with("HTTP/") {
        return Err(RequestReadError::Malformed);
    }

    let mut authorization = String::new();
    let mut host = String::new();
    let mut has_origin = false;
    let mut content_length = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(RequestReadError::Malformed)?;
        if name.trim().is_empty() {
            return Err(RequestReadError::Malformed);
        }
        let value = value.trim();
        if name.eq_ignore_ascii_case("authorization") {
            authorization = value.to_string();
        } else if name.eq_ignore_ascii_case("host") {
            host = value.to_string();
        } else if name.eq_ignore_ascii_case("origin") {
            has_origin = true;
        } else if name.eq_ignore_ascii_case("content-length") {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(RequestReadError::Malformed);
            }
            let length = value
                .parse::<usize>()
                .map_err(|_| RequestReadError::TooLarge)?;
            if let Some(previous) = content_length {
                if previous != length {
                    return Err(RequestReadError::Malformed);
                }
            } else {
                content_length = Some(length);
            }
        }
    }

    Ok(HttpRequestHead {
        method,
        path,
        authorization,
        host,
        has_origin,
        content_length: content_length.unwrap_or(0),
    })
}

fn handle_access_and_browser(
    stream: &mut TcpStream,
    request: &HttpRequest,
    shared: &Arc<Mutex<SharedState>>,
) -> bool {
    let expected = {
        let password = shared
            .lock()
            .ok()
            .map(|state| state.settings.password.clone())
            .unwrap_or_default();
        format!(
            "Basic {}",
            base64_basic(&format!("presenter-orbit:{password}"))
        )
    };
    if request.method == "OPTIONS" {
        write_response(stream, 204, "", "");
        return true;
    }
    if !authorized(&request.authorization, &expected) {
        write_response(
            stream,
            401,
            "{\"error\":\"Unauthorized\"}",
            "WWW-Authenticate: Basic realm=\"Presenter Orbit Remote\"\r\n",
        );
        return true;
    }
    if request.method == "GET" && request.path == "/" {
        write_response(
            stream,
            200,
            include_str!("remote-browser.html"),
            "Content-Type: text/html; charset=utf-8\r\n",
        );
        return true;
    }
    false
}

fn compose_status(
    shared: &Arc<Mutex<SharedState>>,
    catalog: Result<Value, String>,
    now_ms: i64,
) -> Value {
    let mut status = shared
        .lock()
        .ok()
        .map(|state| state.status.clone())
        .unwrap_or_else(|| json!({}));
    status["serverTimeMs"] = json!(now_ms);
    match catalog {
        Ok(catalog) => {
            for (key, value) in catalog.as_object().expect("catalog object") {
                status[key] = value.clone();
            }
        }
        Err(error) => {
            status["controlsError"] = json!(error);
        }
    }
    status
}

fn current_status(app: &Host, shared: &Arc<Mutex<SharedState>>) -> Value {
    compose_status(
        shared,
        crate::remote_controls::catalog(app),
        Utc::now().timestamp_millis(),
    )
}

fn command_result(app: &Host, shared: &Mutex<SharedState>, id: &str) -> Option<Value> {
    let result = shared
        .lock()
        .ok()?
        .commands
        .iter()
        .find(|entry| entry["requestId"].as_str() == Some(id))
        .cloned()?;
    let active = app
        .state::<crate::AppState>()
        .active_workspace
        .lock()
        .ok()
        .and_then(|path| path.clone());
    Some(resolve_command_result(
        result,
        active.as_ref().and_then(|path| path.to_str()),
        Utc::now().timestamp_millis(),
    ))
}

fn handle_connection(
    mut stream: TcpStream,
    app: &Host,
    shared: &Arc<Mutex<SharedState>>,
    hub: &Arc<EventHub>,
    mcp_only: bool,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let request = match read_http_request(&mut stream) {
        Ok(request) => request,
        Err(RequestReadError::TooLarge) => {
            write_json_response(
                &mut stream,
                413,
                &json!({ "error": "The request is too large." }),
            );
            return;
        }
        Err(_) => {
            write_json_response(
                &mut stream,
                400,
                &json!({ "error": "The request is malformed or incomplete." }),
            );
            return;
        }
    };
    if mcp_only {
        let (code, result) = remote_mcp::handle(app, shared, &request);
        write_json_response(&mut stream, code, &result);
        return;
    }
    let enabled = shared.lock().is_ok_and(|state| {
        state.settings.enabled
            && state.settings.pro7.remote_enabled
            && !state.settings.password.is_empty()
    });
    if !enabled {
        write_json_response(
            &mut stream,
            403,
            &json!({"error":"Remote control is disabled."}),
        );
        return;
    }
    let method = request.method.as_str();
    let path = request.path.as_str();
    let body = String::from_utf8_lossy(&request.body);
    if handle_access_and_browser(&mut stream, &request, shared) {
        return;
    }
    if method == "POST" && path == "/api/v1/link/join" {
        let result = stream
            .peer_addr()
            .map_err(|error| error.to_string())
            .and_then(|source| {
                crate::network_link_membership::accept(app, source.ip(), &request.body)
            });
        match result {
            Ok(result) => write_json_response(&mut stream, 200, &result),
            Err(error) => write_json_response(&mut stream, 400, &json!({"ok":false,"error":error})),
        }
        return;
    }
    if method == "POST" && path == "/api/v1/link/cue" {
        match crate::network_link::receive_and_apply(app, &request.body) {
            Ok(result) => {
                let status = native_command_outcome(&result).http_status();
                write_json_response(&mut stream, status, &result);
            }
            Err(error) => write_json_response(
                &mut stream,
                400,
                &json!({"ok":false,"status":"invalid-target","changed":false,"error":error}),
            ),
        }
        return;
    }
    if method == "GET" && path == "/api/v1/status" {
        write_json_response(&mut stream, 200, &current_status(app, shared));
        return;
    }
    if method == "GET" && path == "/api/v1/events" {
        let app = app.clone();
        let shared = Arc::clone(shared);
        remote_events::serve_event_stream(stream, hub, move || current_status(&app, &shared));
        return;
    }
    if method == "GET" && path.starts_with("/api/v1/commands/") {
        let id = &path["/api/v1/commands/".len()..];
        if let Some(result) = command_result(app, shared, id) {
            write_json_response(&mut stream, 200, &result);
        } else {
            write_json_response(
                &mut stream,
                404,
                &json!({"error":"Remote request is unavailable or expired."}),
            );
        }
        return;
    }
    if method == "POST" && path == "/api/v1/command" {
        match serde_json::from_str(&body) {
            Ok(payload) => {
                let (outcome, result) = execute_command(app, shared, payload);
                write_json_response(&mut stream, outcome.http_status(), &result);
            }
            Err(_) => write_json_response(
                &mut stream,
                400,
                &json!({"ok":false,"error":"Invalid JSON."}),
            ),
        }
        return;
    }
    write_json_response(&mut stream, 404, &json!({ "error": "Not found." }));
}

enum CommandOutcome {
    Applied,
    Queued,
    Invalid,
    Failed,
    Unavailable,
}

impl CommandOutcome {
    fn http_status(&self) -> u16 {
        match self {
            Self::Applied => 200,
            Self::Queued => 202,
            Self::Invalid => 400,
            Self::Failed => 500,
            Self::Unavailable => 503,
        }
    }
}

fn execute_command(
    app: &Host,
    shared: &Mutex<SharedState>,
    mut payload: Value,
) -> (CommandOutcome, Value) {
    use CommandOutcome::*;
    if let Err(error) = validate_command(&payload) {
        return (Invalid, json!({ "ok": false, "error": error }));
    }
    let command = payload["command"].as_str().expect("validated command");
    if matches!(
        command,
        "audienceLook" | "macro" | "message" | "stageLayout" | "clearGroup"
    ) || crate::remote_definitions::is_definition_command(command)
    {
        let state = app.state::<crate::AppState>();
        let mut workspace_io = match crate::lock_workspace_io(&state) {
            Ok(guard) => guard,
            Err(error) => return (Failed, json!({"ok":false,"error":error})),
        };
        if let Err(error) =
            crate::remote_controls::validate_workspace_command(&state, &mut workspace_io, &payload)
        {
            return (
                Invalid,
                json!({"ok":false,"status":"invalid-target","error":error}),
            );
        }
        let request_id = format!(
            "{}-{}",
            Utc::now().timestamp_millis(),
            REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        payload["requestId"] = json!(request_id);
        let result = json!({"ok":true,"status":"queued","requestId":request_id,"workspacePath":payload["workspacePath"],"command":payload["command"],"id":payload["id"],"acceptedAtMs":Utc::now().timestamp_millis()});
        if let Ok(mut shared) = shared.lock() {
            if shared.commands.len() >= MAX_COMMAND_RESULTS {
                shared.commands.pop_front();
            }
            shared.commands.push_back(result.clone());
        } else {
            return (
                Unavailable,
                json!({"ok":false,"error":"Remote state is unavailable."}),
            );
        }
        return if let Err(error) = app.emit_to("main", "remote-command", payload) {
            let report = json!({"requestId":request_id,"workspacePath":result["workspacePath"],"status":"error","error":error.to_string()});
            let _ = state.remote_status.report(&report);
            (Unavailable, report)
        } else {
            (Queued, result)
        };
    }
    if uses_native_command_path(command) {
        return match crate::execute_native_remote_command(app, command, &payload) {
            Ok(result) => (native_command_outcome(&result), result),
            Err(error) => (
                Failed,
                json!({
                    "ok": false,
                    "status": "error",
                    "changed": false,
                    "error": error,
                }),
            ),
        };
    }
    match app.emit_to("main", "remote-command", payload) {
        Ok(()) => (Queued, json!({ "ok": true, "status": "queued" })),
        Err(error) => (
            Unavailable,
            json!({ "ok": false, "error": error.to_string() }),
        ),
    }
}

fn authorized(authorization: &str, expected: &str) -> bool {
    authorization == expected
}

fn validate_command(payload: &Value) -> Result<(), &'static str> {
    let command = payload
        .get("command")
        .and_then(Value::as_str)
        .ok_or("A command is required.")?;
    if !uses_native_command_path(command)
        && !matches!(
            command,
            "nextItem"
                | "previousItem"
                | "audienceLook"
                | "macro"
                | "message"
                | "stageLayout"
                | "clearGroup"
                | "timerDefinition"
                | "messageDefinition"
        )
    {
        return Err("Unsupported remote command.");
    }
    if crate::remote_definitions::is_definition_command(command) {
        crate::remote_definitions::validate_payload(payload)?;
    }
    if command == "timelineTransport" {
        crate::remote_timeline_commands::validate_payload(payload)
            .map_err(|_| "Invalid timeline transport command.")?;
    }
    if matches!(command, "cue" | "libraryCue" | "playlistCue") {
        for field in ["workspacePath", "presentationId", "slideId"] {
            if !payload[field]
                .as_str()
                .is_some_and(|id| !id.trim().is_empty())
            {
                return Err("A workspace, presentation and slide identity are required.");
            }
        }
        for field in ["playlistEntryId", "cueInstanceId"] {
            if !payload.get(field).is_some_and(|value| {
                value.is_null() || value.as_str().is_some_and(|id| !id.is_empty())
            }) {
                return Err("Explicit playlist and cue occurrence identities are required.");
            }
        }
    }
    if command == "libraryCue"
        && !payload["libraryId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    {
        return Err("A library identity is required.");
    }
    if command == "libraryCue"
        && payload
            .get("arrangementId")
            .is_some_and(|value| !value.as_str().is_some_and(|id| !id.is_empty()))
    {
        return Err("Arrangement identity must be a nonempty string when supplied.");
    }
    if command == "playlistCue"
        && ["playlistId", "playlistEntryId"]
            .iter()
            .any(|field| !payload[*field].as_str().is_some_and(|id| !id.is_empty()))
    {
        return Err("Exact playlist and entry identities are required.");
    }
    if command == "ticker" {
        if !payload["workspacePath"]
            .as_str()
            .is_some_and(|path| !path.trim().is_empty())
        {
            return Err("Ticker workspacePath is required.");
        }
        let mut action = payload
            .as_object()
            .ok_or("Ticker action must be an object.")?
            .clone();
        action.remove("command");
        action.remove("workspacePath");
        serde_json::from_value::<crate::native_ticker::TickerAction>(Value::Object(action))
            .map_err(|_| "Ticker requires a valid play, pause, resume, clear or seek action.")?;
        if payload["type"] == "play"
            && !payload["id"]
                .as_str()
                .is_some_and(|id| !id.trim().is_empty())
        {
            return Err("Ticker identity is required.");
        }
        if payload["type"] == "seek"
            && !payload["seconds"]
                .as_f64()
                .is_some_and(|seconds| seconds.is_finite() && (0.0..=86400.0).contains(&seconds))
        {
            return Err("Ticker seek must be between 0 and 86400 seconds.");
        }
    }
    if command == "media" {
        for field in ["workspacePath", "id"] {
            if !payload[field]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Err("Media workspace and identity are required.");
            }
        }
        match payload["action"].as_str() {
            Some("trigger") => {}
            Some(
                "play" | "pause" | "restart" | "inPoint" | "outPoint" | "previous" | "next"
                | "seek",
            ) if payload["selectionId"]
                .as_f64()
                .is_some_and(|id| id.is_finite() && id >= 0.0) => {}
            _ => return Err("Media transport requires an exact selection identity."),
        }
    }
    if command == "media"
        && payload["action"] == "seek"
        && !payload["position"]
            .as_f64()
            .is_some_and(|position| position.is_finite() && (0.0..=1.0).contains(&position))
    {
        return Err("Seek position must be between 0 and 1.");
    }
    if command == "media"
        && matches!(payload["action"].as_str(), Some("previous" | "next"))
        && (!payload["playlistId"]
            .as_str()
            .is_some_and(|id| !id.trim().is_empty())
            || !payload["playlistOrder"].as_array().is_some_and(|ids| {
                !ids.is_empty()
                    && ids
                        .iter()
                        .all(|id| id.as_str().is_some_and(|id| !id.trim().is_empty()))
            }))
    {
        return Err("Audio navigation requires an exact playlist identity and order.");
    }
    if command == "stageMessage" {
        if !payload["workspacePath"]
            .as_str()
            .is_some_and(|path| !path.trim().is_empty())
        {
            return Err("Stage message workspacePath is required.");
        }
        match payload["action"].as_str() {
            Some("edit") if payload["text"].is_string() => {}
            Some("show" | "hide" | "clear") => {}
            _ => return Err("Stage message requires edit with text, show, hide or clear."),
        }
    }
    if command == "capture" {
        if !payload
            .get("workspacePath")
            .and_then(Value::as_str)
            .is_some_and(|path| !path.trim().is_empty())
        {
            return Err("Capture workspacePath is required.");
        }
        match payload["action"].as_str() {
            Some("start")
                if payload
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.trim().is_empty()) => {}
            Some("stop") => {}
            _ => return Err("Capture requires start with a preset id, or stop."),
        }
    }
    if matches!(command, "audienceLook" | "macro" | "clearGroup") {
        for field in ["id", "workspacePath"] {
            if !payload
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Err("A target id and workspacePath are required.");
            }
        }
    }
    if command == "stageLayout" {
        for field in ["workspacePath", "screenId", "id"] {
            if !payload
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Err("A stage screen, layout id and workspacePath are required.");
            }
        }
    }
    if command == "message" {
        if !payload
            .get("workspacePath")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err("A workspacePath is required.");
        }
        match payload.get("action").and_then(Value::as_str) {
            Some("show")
                if payload
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.trim().is_empty()) => {}
            Some("clear") if payload.get("id").is_none() => {}
            _ => return Err("Message requires show with a message id, or clear."),
        }
    }
    if matches!(command, "timer" | "prop") {
        if !payload
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.trim().is_empty())
        {
            return Err("A target id is required.");
        }
        let action = payload
            .get("action")
            .and_then(Value::as_str)
            .ok_or("An action is required.")?;
        if command == "timer" {
            if !matches!(action, "start" | "stop" | "reset" | "increment") {
                return Err("Unsupported timer action.");
            }
            if action == "increment"
                && !payload
                    .get("seconds")
                    .and_then(Value::as_f64)
                    .is_some_and(f64::is_finite)
            {
                return Err("Finite increment seconds are required.");
            }
        } else if !matches!(action, "show" | "clear") {
            return Err("Unsupported prop action.");
        }
    }
    Ok(())
}

fn write_json_response(stream: &mut TcpStream, status: u16, value: &Value) {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    write_response(
        stream,
        status,
        &body,
        "Content-Type: application/json; charset=utf-8\r\n",
    );
}

fn write_response(stream: &mut TcpStream, status: u16, body: &str, extra_headers: &str) {
    let status_text = match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        413 => "Payload Too Large",
        _ => "Not Found",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Length: {}\r\nConnection: close\r\n{CORS_HEADERS}{extra_headers}\r\n{body}",
        body.as_bytes().len()
    );
    let _ = stream.write_all(response.as_bytes());
}

pub(crate) fn base64_basic(value: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = value.as_bytes();
    let mut result = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        let second = bytes.get(index + 1).copied();
        let third = bytes.get(index + 2).copied();
        result.push(ALPHABET[(first >> 2) as usize] as char);
        result.push(ALPHABET[((first & 0x03) << 4 | second.unwrap_or(0) >> 4) as usize] as char);
        result.push(second.map_or('=', |value| {
            ALPHABET[((value & 0x0f) << 2 | third.unwrap_or(0) >> 6) as usize] as char
        }));
        result.push(third.map_or('=', |value| ALPHABET[(value & 0x3f) as usize] as char));
        index += 3;
    }
    result
}
