//! ProPresenter compatibility listener: current Remote (`/v1/ws`) and optional
//! Stage Display (`/stagedisplay`).
//!
//! Every connection runs on its own thread. Actions are mapped onto the same
//! status and command paths as the HTTP API; this module owns no business logic.

use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tungstenite::{
    accept_hdr_with_config,
    handshake::server::{ErrorResponse, Request, Response},
    protocol::{frame::coding::CloseCode, CloseFrame, WebSocketConfig},
    Message, WebSocket,
};

use super::{current_status, execute_command, CommandOutcome, EventHub, SharedState};
use crate::Host;

#[path = "remote_pro7_data.rs"]
mod data;
#[path = "remote_modern.rs"]
mod modern;
#[path = "remote_modern_discovery.rs"]
mod modern_discovery;
#[path = "remote_pro7_preview.rs"]
mod preview;
#[path = "remote_pro7_stage.rs"]
mod stage;
#[path = "remote_tcp.rs"]
pub(super) mod tcp;

pub(super) const DEFAULT_PORT: u16 = 49_322;
const MAX_CONNECTIONS: usize = 8;
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(3);
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_millis(100);

/// Status and commands behind the protocol, so the handlers do not depend on Tauri.
pub(super) trait Backend: Send + Sync {
    /// Handles ordinary HTTP requests on the shared network listener.
    fn serve_http(&self, mut stream: TcpStream) {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    /// The accepted native output status, without reading the workspace.
    fn live(&self) -> Value;
    /// The live status joined with the workspace catalogs.
    fn full(&self) -> Result<Value, String>;
    /// A status-change signal; backends without one retain periodic refreshes.
    fn status_generation(&self) -> Option<u64> {
        None
    }
    /// Runs an HTTP API command; `Err` carries the rejection reason.
    fn command(&self, command: &Value) -> Result<Value, String>;
    fn media_metadata(&self, _asset: &Value) -> Result<Value, String> {
        Err("Media item duration metadata is unavailable from its source.".into())
    }
    fn asset_image(&self, _kind: &str, _source_id: &str) -> Result<Vec<u8>, String> {
        Err("Native asset previews are unavailable.".into())
    }
    fn slide_images(
        &self,
        workspace: &str,
        path: &str,
        slides: &[Value],
        quality: u8,
    ) -> Result<Vec<String>, String>;
}

pub(super) struct AppBackend {
    pub app: Host,
    pub shared: Arc<Mutex<SharedState>>,
    pub hub: Arc<EventHub>,
    pub mcp_only: bool,
}

impl Backend for AppBackend {
    fn serve_http(&self, stream: TcpStream) {
        super::handle_connection(stream, &self.app, &self.shared, &self.hub, self.mcp_only);
    }
    fn asset_image(&self, kind: &str, source_id: &str) -> Result<Vec<u8>, String> {
        let state = self.app.state::<crate::AppState>();
        let path = state
            .active_workspace
            .lock()
            .map_err(|_| "Workspace state is unavailable.")?
            .clone()
            .ok_or("An active workspace is required.")?;
        let (document, revision) = {
            let mut io = crate::lock_workspace_io(&state)?;
            io.require_active(&path)?;
            (crate::workspace::read_workspace(&path)?, io.revision(&path))
        };
        let kind = match kind {
            "props" => "prop",
            "stageLayouts" => "stageLayout",
            "masks" => "mask",
            "macros" => "macro",
            "clearGroups" => "clearGroup",
            value => value,
        };
        let image = modern::images::render(&document, kind, source_id)?;
        let mut io = crate::lock_workspace_io(&state)?;
        io.require_active(&path)?;
        io.require_revision(&path, revision)?;
        Ok(image)
    }
    fn media_metadata(&self, asset: &Value) -> Result<Value, String> {
        modern::media_metadata::inspect(asset)
    }
    fn live(&self) -> Value {
        self.shared
            .lock()
            .map(|state| state.status.clone())
            .unwrap_or_else(|_| json!({}))
    }

    fn full(&self) -> Result<Value, String> {
        let mut status = current_status(&self.app, &self.shared);
        status["timelinePlayhead"] = self
            .app
            .state::<crate::AppState>()
            .native_automation
            .lock()
            .map_err(|_| "Native timeline state is unavailable.")?
            .current_timeline_playhead()
            .unwrap_or(Value::Null);
        match status.get("controlsError").and_then(Value::as_str) {
            Some(error) => Err(error.to_string()),
            None => Ok(status),
        }
    }

    fn command(&self, command: &Value) -> Result<Value, String> {
        let (outcome, body) = execute_command(&self.app, &self.shared, command.clone());
        if matches!(outcome, CommandOutcome::Applied | CommandOutcome::Queued)
            && body.get("ok") != Some(&json!(false))
        {
            Ok(body)
        } else {
            Err(body.get("error").and_then(Value::as_str).map_or_else(
                || format!("The command failed with status {}.", outcome.http_status()),
                str::to_string,
            ))
        }
    }

    fn status_generation(&self) -> Option<u64> {
        Some(self.hub.generation())
    }

    fn slide_images(
        &self,
        workspace: &str,
        path: &str,
        slides: &[Value],
        quality: u8,
    ) -> Result<Vec<String>, String> {
        let state = self.app.state::<crate::AppState>();
        let path_on_disk = crate::active_workspace(&state, json!(workspace))?;
        let key = format!("{path}\n{quality}\n{}", Value::from(slides.to_vec()));
        {
            let mut io = crate::lock_workspace_io(&state)?;
            io.require_active(&path_on_disk)?;
            let revision = io.revision(&path_on_disk);
            if let Some(images) = cached_slide_images(&path_on_disk, revision, &key)? {
                return Ok(images);
            }
        }
        let (document, catalog, revision) =
            crate::remote_controls::workspace_snapshot(&state, &path_on_disk)?;
        let images = preview::render_with_catalog(&document, &catalog, path, slides, quality)?;
        let mut io = crate::lock_workspace_io(&state)?;
        io.require_active(&path_on_disk)?;
        io.require_revision(&path_on_disk, revision)?;
        drop(io);
        store_slide_images(&path_on_disk, revision, key, &images)?;
        Ok(images)
    }
}

type SlideImageCache = (
    std::path::PathBuf,
    u64,
    std::collections::HashMap<String, Vec<String>>,
);
// Remote clients re-request the same thumbnails while browsing; every workspace write starts over.
static SLIDE_IMAGES: Mutex<Option<SlideImageCache>> = Mutex::new(None);
// shortcut: the cache is simply cleared when full, use an LRU if large workspaces thrash it.
const MAX_CACHED_SLIDE_IMAGES: usize = 1024;

fn cached_slide_images(
    workspace: &std::path::Path,
    revision: u64,
    key: &str,
) -> Result<Option<Vec<String>>, String> {
    let cache = SLIDE_IMAGES
        .lock()
        .map_err(|_| "Slide preview cache is unavailable.")?;
    Ok(cache
        .as_ref()
        .filter(|(path, cached_revision, _)| path == workspace && *cached_revision == revision)
        .and_then(|(_, _, images)| images.get(key).cloned()))
}

fn store_slide_images(
    workspace: &std::path::Path,
    revision: u64,
    key: String,
    images: &[String],
) -> Result<(), String> {
    let mut cache = SLIDE_IMAGES
        .lock()
        .map_err(|_| "Slide preview cache is unavailable.")?;
    let current = cache
        .as_ref()
        .is_some_and(|(path, cached_revision, entries)| {
            path == workspace
                && *cached_revision == revision
                && entries.len() < MAX_CACHED_SLIDE_IMAGES
        });
    if !current {
        *cache = Some((workspace.to_path_buf(), revision, Default::default()));
    }
    if let Some((_, _, entries)) = cache.as_mut() {
        entries.insert(key, images.to_vec());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pro7Settings {
    pub enabled: bool,
    pub network_name: String,
    pub remote_enabled: bool,
    pub stage_enabled: bool,
    pub port: u16,
    pub remote_password: String,
    pub stage_password: String,
}

impl Default for Pro7Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            network_name: "Presenter Orbit".into(),
            remote_enabled: false,
            stage_enabled: false,
            port: DEFAULT_PORT,
            remote_password: String::new(),
            stage_password: String::new(),
        }
    }
}

struct Credentials {
    remote: String,
    stage: String,
}

/// Compares without stopping at the first difference.
pub(super) fn password_matches(supplied: &str, expected: &str) -> bool {
    supplied.len() == expected.len()
        && supplied
            .bytes()
            .zip(expected.bytes())
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0
}

struct Outcome {
    replies: Vec<Value>,
    close: bool,
}

struct Session {
    authenticated: bool,
    last_values: Value,
    last_clock: String,
}

impl Session {
    fn new() -> Self {
        Self {
            authenticated: false,
            last_values: Value::Null,
            last_clock: String::new(),
        }
    }

    fn on_message(
        &mut self,
        backend: &dyn Backend,
        credentials: &Credentials,
        text: &str,
    ) -> Outcome {
        let Some(message) = serde_json::from_str::<Value>(text)
            .ok()
            .filter(Value::is_object)
        else {
            let reply = stage::error("The message must be a JSON object.");
            return Outcome {
                replies: vec![reply],
                close: !self.authenticated,
            };
        };
        if !self.authenticated {
            return self.authenticate(backend, credentials, &message);
        }
        let replies = stage::handle(backend, &message);
        Outcome {
            replies,
            close: false,
        }
    }

    fn authenticate(
        &mut self,
        backend: &dyn Backend,
        credentials: &Credentials,
        message: &Value,
    ) -> Outcome {
        let supplied = |field: &str| {
            message
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let (reply, accepted) = {
            let failure = if message["acn"] != "ath" {
                Some("Authenticate first.")
            } else if !message["ptl"].is_u64() {
                Some("A protocol version is required.")
            } else if !password_matches(&supplied("pwd"), &credentials.stage) {
                Some("Incorrect password")
            } else {
                None
            };
            (
                json!({"acn": "ath", "ath": failure.is_none(), "err": failure.unwrap_or_default()}),
                failure.is_none(),
            )
        };
        if accepted {
            self.authenticated = true;
            let live = backend.live();
            self.last_values = stage::slide_values(&live);
            self.last_clock = stage::clock_text();
        }
        Outcome {
            replies: vec![reply],
            close: !accepted,
        }
    }

    /// Pushes slide values and the clock to stage displays.
    fn on_tick(&mut self, backend: &dyn Backend) -> Vec<Value> {
        if !self.authenticated {
            return Vec::new();
        }
        let live = backend.live();
        let values = stage::slide_values(&live);
        let clock = stage::clock_text();
        let replies = if values != self.last_values {
            vec![stage::frame_values(&live)]
        } else if clock != self.last_clock {
            vec![json!({"acn": "sys", "txt": clock})]
        } else {
            Vec::new()
        };
        self.last_values = values;
        self.last_clock = clock;
        replies
    }
}

pub(super) struct Pro7Server {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    discovery: Option<modern_discovery::Discovery>,
}

impl Pro7Server {
    pub(super) fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
            discovery: None,
        }
    }

    pub(super) fn start(
        &mut self,
        backend: Arc<dyn Backend>,
        settings: &Pro7Settings,
    ) -> Result<(), String> {
        let listener = TcpListener::bind(("0.0.0.0", settings.port))
            .map_err(|error| format!("Orbit network listener could not start: {error}"))?;
        let credentials = Credentials {
            remote: if settings.enabled && settings.remote_enabled {
                settings.remote_password.clone()
            } else {
                String::new()
            },
            stage: if settings.enabled && settings.stage_enabled {
                settings.stage_password.clone()
            } else {
                String::new()
            },
        };
        let discovery = if !credentials.remote.is_empty() {
            Some(modern_discovery::Discovery::start(
                settings.port,
                &settings.network_name,
            )?)
        } else {
            None
        };
        self.serve(listener, backend, credentials)?;
        self.discovery = discovery;
        Ok(())
    }

    pub(super) fn start_local_mcp(
        &mut self,
        backend: Arc<dyn Backend>,
        port: u16,
    ) -> Result<(), String> {
        let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|error| {
            format!("Local MCP listener could not start on port {port}: {error}")
        })?;
        self.serve(
            listener,
            backend,
            Credentials {
                remote: String::new(),
                stage: String::new(),
            },
        )
    }

    fn serve(
        &mut self,
        listener: TcpListener,
        backend: Arc<dyn Backend>,
        credentials: Credentials,
    ) -> Result<(), String> {
        listener.set_nonblocking(true).map_err(|error| {
            format!("ProPresenter compatibility server could not be configured: {error}")
        })?;
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = Arc::clone(&stop);
        let credentials = Arc::new(credentials);
        let thread = thread::Builder::new()
            .name("presenter-orbit-pro7".to_string())
            .spawn(move || accept_loop(&listener, &backend, &credentials, &stop))
            .map_err(|error| {
                format!("ProPresenter compatibility server thread could not start: {error}")
            })?;
        self.thread = Some(thread);
        Ok(())
    }

    pub(super) fn is_running(&self) -> bool {
        self.thread.is_some()
    }

    pub(super) fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.discovery = None;
    }
}

struct ConnectionSlot(Arc<AtomicUsize>);

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn accept_loop(
    listener: &TcpListener,
    backend: &Arc<dyn Backend>,
    credentials: &Arc<Credentials>,
    stop: &Arc<AtomicBool>,
) {
    let connections = Arc::new(AtomicUsize::new(0));
    let image_access = Arc::new(modern::http::Access::default());
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                if connections.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
                    connections.fetch_sub(1, Ordering::AcqRel);
                    let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let slot = ConnectionSlot(Arc::clone(&connections));
                let image_access = Arc::clone(&image_access);
                let (backend, credentials, stop) = (
                    Arc::clone(backend),
                    Arc::clone(credentials),
                    Arc::clone(stop),
                );
                let _ = thread::Builder::new()
                    .name("orbit-pro7-connection".into())
                    .spawn(move || {
                        let _slot = slot;
                        serve_connection(
                            stream,
                            backend.as_ref(),
                            &credentials,
                            &stop,
                            &image_access,
                        );
                    });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50))
            }
            Err(_) => break,
        }
    }
}

fn not_found() -> ErrorResponse {
    let mut response = ErrorResponse::new(Some("Not found.".to_string()));
    *response.status_mut() = tungstenite::http::StatusCode::NOT_FOUND;
    response
}

fn close(socket: &mut WebSocket<TcpStream>, code: CloseCode) {
    let _ = socket.close(Some(CloseFrame {
        code,
        reason: "".into(),
    }));
    let _ = socket.flush();
}

fn serve_connection(
    mut stream: TcpStream,
    backend: &dyn Backend,
    credentials: &Credentials,
    stop: &AtomicBool,
    image_access: &Arc<modern::http::Access>,
) {
    // An accepted socket can inherit the non-blocking mode of the listener.
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
    {
        return;
    }
    let _ = stream.set_nodelay(true);
    if modern::http::serve_thumbnail(&mut stream, backend, image_access) {
        return;
    }
    // Peek without consuming: ordinary HTTP and mobile sockets share the network port.
    let mut head = [0; 8192];
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let target = loop {
        let Ok(count) = stream.peek(&mut head) else {
            return;
        };
        if let Some(end) = head[..count].windows(2).position(|part| part == b"\r\n") {
            let Ok(line) = std::str::from_utf8(&head[..end]) else {
                return;
            };
            break line
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .split('?')
                .next()
                .unwrap_or("")
                .to_string();
        }
        if count == 0 || count == head.len() || Instant::now() >= deadline {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    };
    if !matches!(target.as_str(), "/v1/ws" | "/stagedisplay" | "/remote") {
        return backend.serve_http(stream);
    }
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let mut accepted = false;
    let mut is_modern = false;
    let callback = |request: &Request, response: Response| {
        is_modern = request.uri().path() == "/v1/ws";
        accepted = (is_modern && !credentials.remote.is_empty())
            || (request.uri().path() == "/stagedisplay" && !credentials.stage.is_empty());
        if accepted {
            Ok(response)
        } else {
            Err(not_found())
        }
    };
    let Ok(mut socket) = accept_hdr_with_config(stream, callback, Some(config)) else {
        return;
    };
    if is_modern {
        return modern::serve(socket, backend, &credentials.remote, stop, image_access);
    }
    if socket.get_ref().set_read_timeout(Some(TICK)).is_err() {
        return;
    }
    let mut session = Session::new();
    let connected = Instant::now();
    let mut last_tick = Instant::now();
    while !stop.load(Ordering::Acquire) {
        match socket.read() {
            Ok(Message::Text(text)) => {
                if stop.load(Ordering::Acquire) {
                    return close(&mut socket, CloseCode::Away);
                }
                let outcome = session.on_message(backend, credentials, text.as_str());
                for reply in outcome.replies {
                    if socket.send(Message::text(reply.to_string())).is_err() {
                        return;
                    }
                }
                if outcome.close {
                    close(&mut socket, CloseCode::Policy);
                    return;
                }
            }
            Ok(Message::Binary(_)) => return close(&mut socket, CloseCode::Unsupported),
            Ok(Message::Close(_)) => return,
            Ok(_) => {
                let _ = socket.flush();
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(tungstenite::Error::Capacity(_)) => return close(&mut socket, CloseCode::Size),
            Err(_) => return,
        }
        if last_tick.elapsed() >= TICK {
            last_tick = Instant::now();
            if !session.authenticated && connected.elapsed() >= AUTH_TIMEOUT {
                return close(&mut socket, CloseCode::Policy);
            }
            for push in session.on_tick(backend) {
                if socket.send(Message::text(push.to_string())).is_err() {
                    return;
                }
            }
        }
    }
    close(&mut socket, CloseCode::Away);
}
