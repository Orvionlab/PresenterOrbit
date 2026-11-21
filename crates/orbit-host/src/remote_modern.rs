//! Current ProPresenter Remote transport. Requests use the HTTP API's relative
//! paths inside password-encrypted JSON WebSocket messages.

use std::{
    collections::BTreeMap,
    net::TcpStream,
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tungstenite::{protocol::frame::coding::CloseCode, Message, WebSocket};

use super::{close, data, Backend, AUTH_TIMEOUT, TICK};

#[path = "remote_modern_catalog.rs"]
mod catalog;
#[path = "remote_modern_codec.rs"]
mod codec;
#[path = "remote_modern_controls.rs"]
mod controls;
#[path = "remote_modern_http.rs"]
pub(super) mod http;
#[path = "remote_modern_images.rs"]
pub(super) mod images;
#[path = "remote_modern_media.rs"]
mod media;
#[path = "remote_modern_media_metadata.rs"]
pub(super) mod media_metadata;
#[path = "remote_modern_overlays.rs"]
mod overlays;

#[path = "remote_modern_thumbnail.rs"]
mod thumbnail;
#[path = "remote_modern_timeline.rs"]
mod timeline;

const MAX_SUBSCRIPTIONS: usize = 64;
const UPDATE_INTERVAL: Duration = Duration::from_millis(500);

struct Subscription {
    request: Value,
    previous: Value,
}

pub(super) struct Session {
    codec: codec::Codec,
    authenticated: bool,
    subscriptions: BTreeMap<String, Subscription>,
}

fn version() -> Value {
    json!({
        "name": "Presenter Orbit",
        "platform": if cfg!(target_os = "windows") { "win" } else if cfg!(target_os = "macos") { "mac" } else { "unknown" },
        "os_version": std::env::consts::OS,
        "host_description": concat!("Presenter Orbit ", env!("CARGO_PKG_VERSION")),
        "api_version": "v1",
    })
}

fn envelope(url: &str, result: Result<Value, String>) -> Value {
    match result {
        Ok(data) => json!({"url":url,"data":data}),
        Err(error) => json!({"url":url,"error":error}),
    }
}

struct Request {
    original: String,
    parts: Vec<String>,
    method: String,
    body: Option<Value>,
    streaming: bool,
}

impl Request {
    fn parse(value: &Value) -> Result<Self, String> {
        let original = value
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty() && url.len() <= 4096)
            .ok_or("A bounded relative API URL is required.")?;
        if original.starts_with('/') || original.contains('#') || original.contains('\\') {
            return Err("A relative API URL without a leading slash is required.".into());
        }
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .filter(|method| matches!(*method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE"))
            .ok_or("A supported API method is required.")?;
        let (path, query) = original.split_once('?').unwrap_or((original, ""));
        let parts = path
            .split('/')
            .map(decode_segment)
            .collect::<Result<Vec<_>, _>>()?;
        if parts
            .iter()
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains('/'))
        {
            return Err("Invalid API path segment.".into());
        }
        let mut streaming = match value.get("chunked") {
            None => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err("The chunked member must be boolean.".into()),
        };
        let mut chunked_seen = value.get("chunked").is_some();
        for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
            if name == "chunked" {
                if chunked_seen {
                    return Err("Duplicate chunked parameter.".into());
                }
                chunked_seen = true;
                streaming = match value.as_ref() {
                    "true" => true,
                    "false" => false,
                    _ => {
                        trace("invalid-streaming-flag");
                        return Err("The chunked parameter must be boolean.".into());
                    }
                };
            }
        }
        Ok(Self {
            original: original.to_string(),
            parts,
            method: method.to_string(),
            body: value.get("body").cloned(),
            streaming,
        })
    }
}

fn decode_segment(value: &str) -> Result<String, String> {
    let mut decoded = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next().and_then(|byte| (byte as char).to_digit(16));
            let low = bytes.next().and_then(|byte| (byte as char).to_digit(16));
            decoded.push(match (high, low) {
                (Some(high), Some(low)) => (high * 16 + low) as u8,
                _ => return Err("Invalid API path encoding.".into()),
            });
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8(decoded).map_err(|_| "The API path must be UTF-8.".into())
}

fn streamable(parts: &[String]) -> bool {
    let parts = parts.iter().map(String::as_str).collect::<Vec<_>>();
    matches!(
        parts.as_slice(),
        [
            "v1",
            "libraries"
                | "playlists"
                | "timers"
                | "macros"
                | "macro_collections"
                | "looks"
                | "messages"
                | "prop_collections"
                | "props"
        ] | [
            "v1",
            "library"
                | "playlist"
                | "macro"
                | "macro_collection"
                | "prop"
                | "prop_collection"
                | "message"
                | "timer"
                | "look",
            _
        ] | ["v1", "presentation", _]
            | ["v1", "announcement", "active" | "slide_index"]
            | ["v1", "presentation", "active", "timeline"]
            | ["v1", "presentation", "focused", "timeline"]
            | ["v1", "playlist", "focused", _, "timeline"]
            | ["v1", "playlist", "active", "presentation", _, "timeline"]
            | ["v1", "timers", "current"]
            | [
                "v1",
                "stage",
                "screens" | "layouts" | "layout_map" | "message"
            ]
            | ["v1", "stage", "screen", _, "layout"]
            | ["v1", "clear", "groups"]
            | ["v1", "clear", "group", _]
            | ["v1", "status", "layers"]
            | ["v1", "transport", _, "current"]
            | ["v1", "audio" | "media", "playlists"]
            | ["v1", "audio" | "media", "playlist", _]
    )
}

impl Session {
    pub(super) fn new(password: &str) -> Self {
        Self {
            codec: codec::Codec::new(password),
            authenticated: false,
            subscriptions: BTreeMap::new(),
        }
    }

    fn receive(&mut self, backend: &dyn Backend, encoded: &str) -> Result<Value, String> {
        let message = self.codec.decode(encoded).map_err(|error| {
            trace("decode-failed");
            error
        })?;
        self.receive_plain(backend, message)
    }

    pub(super) fn receive_plain(
        &mut self,
        backend: &dyn Backend,
        message: Value,
    ) -> Result<Value, String> {
        if !message.is_object() {
            return Err("A Remote message must be an object.".into());
        }
        let request = Request::parse(&message).map_err(|error| {
            trace("request-validation-failed");
            error
        })?;
        #[cfg(debug_assertions)]
        if std::env::var_os("PRESENTER_ORBIT_REMOTE_TRACE").is_some()
            && request.parts.get(1).is_some_and(|part| {
                part == "prop"
                    || part == "props"
                    || part == "prop_collections"
                    || part == "prop_collection"
            })
        {
            eprintln!(
                "[modern-remote] prop websocket {} {}",
                request.method,
                request.parts.join("/")
            );
        }
        self.authenticated = true;
        if request.streaming && (request.method != "GET" || !streamable(&request.parts)) {
            return Ok(envelope(
                &request.original,
                Err("This endpoint does not support streaming.".into()),
            ));
        }
        if request.streaming
            && !self.subscriptions.contains_key(&request.original)
            && self.subscriptions.len() >= MAX_SUBSCRIPTIONS
        {
            return Ok(envelope(
                &request.original,
                Err("The Remote subscription limit has been reached.".into()),
            ));
        }
        let result = dispatch(backend, &request);
        let reply = envelope(&request.original, result);
        if request.streaming && reply.get("error").is_none() {
            self.subscriptions.insert(
                request.original,
                Subscription {
                    request: message,
                    previous: reply.clone(),
                },
            );
        }
        Ok(reply)
    }

    pub(super) fn tick(&mut self, backend: &dyn Backend) -> Vec<Value> {
        if self.subscriptions.is_empty() {
            return Vec::new();
        }
        let snapshot = SnapshotBackend {
            backend,
            live: backend.live(),
            full: backend.full(),
        };
        let mut replies = Vec::new();
        for subscription in self.subscriptions.values_mut() {
            let Ok(request) = Request::parse(&subscription.request) else {
                continue;
            };
            let next = envelope(&request.original, dispatch(&snapshot, &request));
            if next != subscription.previous {
                subscription.previous = next.clone();
                replies.push(next);
            }
        }
        replies
    }
}

struct SnapshotBackend<'a> {
    backend: &'a dyn Backend,
    live: Value,
    full: Result<Value, String>,
}

impl Backend for SnapshotBackend<'_> {
    fn asset_image(&self, kind: &str, source_id: &str) -> Result<Vec<u8>, String> {
        self.backend.asset_image(kind, source_id)
    }
    fn media_metadata(&self, asset: &Value) -> Result<Value, String> {
        self.backend.media_metadata(asset)
    }
    fn live(&self) -> Value {
        self.live.clone()
    }
    fn full(&self) -> Result<Value, String> {
        self.full.clone()
    }
    fn command(&self, command: &Value) -> Result<Value, String> {
        self.backend.command(command)
    }
    fn slide_images(
        &self,
        workspace: &str,
        path: &str,
        slides: &[Value],
        quality: u8,
    ) -> Result<Vec<String>, String> {
        self.backend.slide_images(workspace, path, slides, quality)
    }
}

fn live_identity(status: &Value, id: &str) -> Result<Value, String> {
    for library in data::array(status, "libraries")? {
        for (index, presentation) in data::array(library, "presentations")?.iter().enumerate() {
            if presentation["id"].as_str() == Some(id) {
                return Ok(catalog::identity(
                    id,
                    data::text(presentation, "title"),
                    index,
                ));
            }
        }
    }
    Err("The live presentation is unavailable in the workspace catalog.".into())
}

fn active_presentation(status: &Value) -> Result<Value, String> {
    let Some(id) = status["presentation"]["id"].as_str() else {
        return Ok(json!({"presentation":null}));
    };
    let identity = live_identity(status, id)?;
    let mut response = catalog::presentation(
        status,
        identity["uuid"]
            .as_str()
            .ok_or("Presentation UUID is required.")?,
    )?;
    if let Some(arrangement) = status["presentation"]["arrangementId"].as_str() {
        response["presentation"]["current_arrangement"] =
            catalog::identity(arrangement, "", 0)["uuid"].clone();
    }
    Ok(response)
}

fn focused_resolved(status: &Value) -> Result<data::Resolved, String> {
    let focus = status
        .get("focusedPresentation")
        .filter(|value| value.is_object())
        .ok_or("There is no focused presentation.")?;
    let id = focus["id"]
        .as_str()
        .ok_or("Focused presentation identity is unavailable.")?;
    let resolved = if let (Some(playlist), Some(entry)) = (
        focus["playlistId"].as_str(),
        focus["playlistEntryId"].as_str(),
    ) {
        let source = data::array(status, "playlists")?
            .iter()
            .find(|item| item["id"] == playlist)
            .ok_or("Focused playlist is unavailable.")?;
        let index = data::array(&source["modern"], "items")?
            .iter()
            .position(|item| item["id"] == entry)
            .ok_or("Focused playlist occurrence is unavailable.")?;
        let selector = catalog::identity(playlist, "", 0)["uuid"]
            .as_str()
            .ok_or("Focused playlist UUID is unavailable.")?
            .to_string();
        catalog::resolve_playlist_presentation(status, &selector, index)?
    } else {
        let selector = catalog::identity(id, "", 0)["uuid"]
            .as_str()
            .ok_or("Focused presentation UUID is unavailable.")?
            .to_string();
        catalog::resolve_presentation(status, &selector)?
    };
    if resolved.cue["presentationId"] != id {
        return Err("Focused presentation occurrence changed.".into());
    }
    Ok(resolved)
}

fn focused_presentation(status: &Value) -> Result<Value, String> {
    let resolved = focused_resolved(status)?;
    let id = resolved.cue["presentationId"]
        .as_str()
        .ok_or("Focused presentation identity is unavailable.")?;
    live_identity(status, id)
}

fn focused_playlist(status: &Value) -> Result<Value, String> {
    let Some(id) = status["focusedPlaylistId"].as_str() else {
        return Ok(json!({"playlist":null,"item":null}));
    };
    let selector = catalog::identity(id, "", 0)["uuid"]
        .as_str()
        .ok_or("Focused playlist UUID is unavailable.")?
        .to_string();
    let playlist = catalog::playlist(status, &selector)?;
    let entry = status["focusedPresentation"]["playlistEntryId"].as_str();
    let item = match entry {
        Some(entry) => {
            let uuid = catalog::identity(entry, "", 0)["uuid"].clone();
            playlist["items"]
                .as_array()
                .and_then(|items| items.iter().find(|item| item["id"]["uuid"] == uuid))
                .ok_or("Focused playlist occurrence is unavailable.")?["id"]
                .clone()
        }
        None => Value::Null,
    };
    Ok(json!({"playlist":playlist["id"],"item":item}))
}

fn live_slide_index(status: &Value) -> Result<Value, String> {
    let Some(id) = status["presentation"]["id"].as_str() else {
        return Ok(json!({"presentation_index":null}));
    };
    let resolved = live_resolved(status)?;
    if status["currentSlide"]["id"].as_str().is_none() {
        return Ok(json!({"presentation_index":null}));
    }
    let index = resolved
        .slides
        .iter()
        .position(|slide| {
            slide["id"] == status["currentSlide"]["id"]
                && slide["cueInstanceId"] == status["currentSlide"]["cueInstanceId"]
        })
        .ok_or("The live cue occurrence is unavailable in the active arrangement.")?;
    let identity = live_identity(status, id)?;
    let total = resolved.slides.len();
    if index >= total {
        return Err("The live slide index is outside the presentation.".into());
    }
    Ok(
        json!({"presentation_index":{"index":index,"presentation_id":identity,"total_cues":total,"remaining_cues":total-index-1}}),
    )
}

fn index(value: &str) -> Result<usize, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("A zero-based cue index is required.".into());
    }
    value
        .parse()
        .map_err(|_| "The cue index is outside the supported range.".into())
}

fn trigger(
    backend: &dyn Backend,
    status: &Value,
    resolved: data::Resolved,
    index: usize,
) -> Result<Value, String> {
    let slide = resolved
        .slides
        .get(index)
        .ok_or("The cue index is unavailable.")?;
    if slide["disabled"] == true {
        return Err("The requested cue is disabled.".into());
    }
    let mut command = resolved.cue;
    command["workspacePath"] = json!(data::workspace_path(status)?);
    command["slideId"] = slide["id"].clone();
    command["cueInstanceId"] = slide["cueInstanceId"].clone();
    backend.command(&command)?;
    Ok(Value::Null)
}

fn live_resolved(status: &Value) -> Result<data::Resolved, String> {
    let path = data::live_path(status).ok_or("There is no active presentation.")?;
    data::resolve(status, &path)
}

fn active_playlist(status: &Value) -> Result<Value, String> {
    let empty = json!({"playlist":null,"item":null});
    let Some(entry_id) = status["presentation"]["playlistEntryId"].as_str() else {
        return Ok(json!({"presentation":empty,"announcements":empty}));
    };
    let mut matches = Vec::new();
    for playlist in data::array(status, "playlists")? {
        if data::array(playlist, "entries")?
            .iter()
            .any(|entry| entry["id"].as_str() == Some(entry_id))
        {
            matches.push(playlist);
        }
    }
    let [playlist] = matches.as_slice() else {
        return Err("The active playlist occurrence is unavailable or ambiguous.".into());
    };
    let items = playlist["modern"]["items"]
        .as_array()
        .ok_or("The active playlist items are unavailable.")?;
    let item_index = items
        .iter()
        .position(|item| item["id"].as_str() == Some(entry_id))
        .ok_or("The active playlist item is unavailable.")?;
    let playlist_uuid =
        catalog::identity(data::text(playlist, "id"), data::text(playlist, "title"), 0)["uuid"]
            .as_str()
            .ok_or("Playlist UUID is required.")?
            .to_string();
    let api_playlist = catalog::playlist(status, &playlist_uuid)?;
    Ok(
        json!({"presentation":{"playlist":api_playlist["id"],"item":api_playlist["items"][item_index]["id"]},"announcements":empty}),
    )
}

fn announcement(status: &Value, want_index: bool) -> Result<Value, String> {
    let player = &status["announcements"];
    let Some(item_id) = player["item"]["id"].as_str() else {
        return Ok(if want_index {
            json!({"announcement_index":null})
        } else {
            json!({"announcement":null})
        });
    };
    let identity = live_identity(status, item_id)?;
    let selector = identity["uuid"]
        .as_str()
        .ok_or("Announcement UUID is required.")?;
    if !want_index {
        let presentation = catalog::presentation(status, selector)?;
        return Ok(json!({"announcement":presentation["presentation"]}));
    }
    let resolved = catalog::resolve_presentation(status, selector)?;
    let Some(index) = resolved
        .slides
        .iter()
        .position(|slide| slide["id"] == player["slide"]["id"])
    else {
        return Err("The active announcement cue is unavailable.".into());
    };
    Ok(
        json!({"announcement_index":{"index":index,"presentation_id":identity,"total_cues":resolved.slides.len(),"remaining_cues":resolved.slides.len()-index-1}}),
    )
}

fn dispatch(backend: &dyn Backend, request: &Request) -> Result<Value, String> {
    let parts = request.parts.iter().map(String::as_str).collect::<Vec<_>>();
    if let Some(result) = timeline::handle(backend, &request.method, &parts, request.body.as_ref())
    {
        return result;
    }
    if request.method == "GET" {
        match parts.as_slice() {
            ["version"] => return Ok(version()),
            ["v1", "libraries"] => return catalog::libraries(&backend.full()?),
            ["v1", "library", id] => return catalog::library(&backend.full()?, id),
            ["v1", "playlists"] => return catalog::playlists(&backend.full()?),
            ["v1", "playlist", "active"] => return active_playlist(&backend.full()?),
            ["v1", "playlist", "focused"] => return focused_playlist(&backend.full()?),
            ["v1", "playlist", id] if *id != "active" => {
                return catalog::playlist(&backend.full()?, id)
            }
            ["v1", "presentation", "active"] => return active_presentation(&backend.full()?),
            ["v1", "presentation", "focused"] => return focused_presentation(&backend.full()?),
            ["v1", "presentation", "slide_index"] => return live_slide_index(&backend.full()?),
            ["v1", "announcement", "active"] => return announcement(&backend.full()?, false),
            ["v1", "announcement", "slide_index"] => return announcement(&backend.full()?, true),
            ["v1", "presentation", id] => return catalog::presentation(&backend.full()?, id),
            ["v1", "presentation", "active", direction, "trigger"]
                if matches!(*direction, "next" | "previous") =>
            {
                backend.command(
                    &json!({"command":if *direction=="next" {"nextSlide"} else {"previousSlide"}}),
                )?;
                return Ok(Value::Null);
            }
            ["v1", "presentation", "active", position, "trigger"] => {
                let status = backend.full()?;
                return trigger(backend, &status, live_resolved(&status)?, index(position)?);
            }
            ["v1", "presentation", "focused", position, "trigger"] => {
                let status = backend.full()?;
                return trigger(
                    backend,
                    &status,
                    focused_resolved(&status)?,
                    index(position)?,
                );
            }
            ["v1", "presentation", id, position, "trigger"] => {
                let status = backend.full()?;
                return trigger(
                    backend,
                    &status,
                    catalog::resolve_presentation(&status, id)?,
                    index(position)?,
                );
            }
            ["v1", "library", library, presentation, position, "trigger"] => {
                let status = backend.full()?;
                let listing = catalog::library(&status, library)?;
                let items = listing["items"]
                    .as_array()
                    .ok_or("The library presentation list is unavailable.")?;
                let candidates = items
                    .iter()
                    .enumerate()
                    .filter(|(index, item)| {
                        item["uuid"].as_str().is_some_and(|id| {
                            catalog::matches_id(id, data::text(item, "name"), *index, presentation)
                        })
                    })
                    .map(|(_, item)| item)
                    .collect::<Vec<_>>();
                let [item] = candidates.as_slice() else {
                    return Err(
                        "The library presentation identifier must match exactly one item.".into(),
                    );
                };
                return trigger(
                    backend,
                    &status,
                    catalog::resolve_presentation(&status, data::text(item, "uuid"))?,
                    index(position)?,
                );
            }
            ["v1", "playlist", playlist, entry, position, "trigger"] => {
                let status = backend.full()?;
                return trigger(
                    backend,
                    &status,
                    catalog::resolve_playlist_presentation(&status, playlist, index(entry)?)?,
                    index(position)?,
                );
            }
            _ => {}
        }
    }
    if let Some(result) = controls::handle(backend, &request.method, &parts, request.body.as_ref())
    {
        return result;
    }
    if let Some(result) = overlays::handle(backend, &request.method, &parts, request.body.as_ref())
    {
        return result;
    }
    if let Some(result) = media::handle(backend, &request.method, &parts, request.body.as_ref()) {
        return result;
    }
    Err("404 Not Found".into())
}

fn trace(event: &str) {
    #[cfg(debug_assertions)]
    if std::env::var_os("PRESENTER_ORBIT_REMOTE_TRACE").is_some() {
        eprintln!("[modern-remote] {event}");
    }
    #[cfg(not(debug_assertions))]
    let _ = event;
}

pub(super) fn serve(
    mut socket: WebSocket<TcpStream>,
    backend: &dyn Backend,
    password: &str,
    stop: &AtomicBool,
    access: &Arc<http::Access>,
) {
    trace("upgraded");
    if socket.get_ref().set_read_timeout(Some(TICK)).is_err() {
        return;
    }
    let mut session = Session::new(password);
    let Ok(greeting) = session.codec.encode(&envelope("version", Ok(version()))) else {
        return;
    };
    if socket.send(Message::text(greeting)).is_err() {
        return;
    }
    trace("greeting-sent");
    let mut first_request = true;
    let mut image_access = None;
    let connected = Instant::now();
    let mut updated = Instant::now();
    let mut status_generation = backend.status_generation();
    while !stop.load(Ordering::Acquire) {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let Ok(reply) = session.receive(backend, text.as_str()) else {
                    trace("message-rejected");
                    return close(&mut socket, CloseCode::Policy);
                };
                if first_request {
                    let Ok(peer) = socket.get_ref().peer_addr() else {
                        return;
                    };
                    let Ok(authorized) = access.authorize(peer.ip()) else {
                        return;
                    };
                    image_access = Some(authorized);
                    trace("first-request-accepted");
                    first_request = false;
                }
                let Ok(reply) = session.codec.encode(&reply) else {
                    trace("response-encoding-failed");
                    return close(&mut socket, CloseCode::Policy);
                };
                if socket.send(Message::text(reply)).is_err() {
                    return;
                }
            }
            Ok(Message::Binary(_)) => return close(&mut socket, CloseCode::Unsupported),
            Ok(Message::Close(_)) => {
                trace("client-closed");
                return;
            }
            Ok(_) => {
                let _ = socket.flush();
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(tungstenite::Error::Capacity(_)) => return close(&mut socket, CloseCode::Size),
            Err(_) => {
                trace("connection-read-failed");
                return;
            }
        }
        if !session.authenticated && connected.elapsed() >= AUTH_TIMEOUT {
            trace("authentication-timeout");
            return close(&mut socket, CloseCode::Policy);
        }
        let generation = backend.status_generation();
        if generation != status_generation || updated.elapsed() >= UPDATE_INTERVAL {
            updated = Instant::now();
            status_generation = generation;
            for reply in session.tick(backend) {
                let Ok(encoded) = session.codec.encode(&reply) else {
                    return close(&mut socket, CloseCode::Size);
                };
                if socket.send(Message::text(encoded)).is_err() {
                    return;
                }
            }
        }
    }
    close(&mut socket, CloseCode::Away);
    drop(image_access);
}
