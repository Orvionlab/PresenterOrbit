use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use super::{
    base64_basic, current_status, port_setting, remote_pro7::password_matches, string_setting,
    Host, HttpRequest, SharedState,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Settings {
    pub enabled: bool,
    pub password: String,
    pub port: u16,
    pub read_status: bool,
    pub read_catalog: bool,
    pub create_presentations: bool,
    pub manage_playlists: bool,
}

impl Settings {
    pub(super) fn from_value(value: &Value) -> Self {
        Self {
            enabled: value["mcpEnabled"] == true,
            password: string_setting(value.as_object(), "mcpPassword"),
            port: port_setting(value.as_object(), "mcpPort", 49_324),
            read_status: value.get("mcpReadStatus").is_none_or(|flag| flag == true),
            read_catalog: value.get("mcpReadCatalog").is_none_or(|flag| flag == true),
            create_presentations: value["mcpCreatePresentations"] == true,
            manage_playlists: value["mcpManagePlaylists"] == true,
        }
    }
}

pub(super) fn handle(
    app: &Host,
    shared: &Arc<Mutex<SharedState>>,
    request: &HttpRequest,
) -> (u16, Value) {
    let error = |code, message| (code, json!({"error":message}));
    let Ok(settings) = shared.lock().map(|state| state.settings.mcp.clone()) else {
        return error(503, "Local MCP settings are unavailable.");
    };
    if !settings.enabled || settings.password.is_empty() {
        return error(
            403,
            "Enable Local MCP and set its password in Presenter Orbit.",
        );
    }
    if request.has_origin || request.host != format!("127.0.0.1:{}", settings.port) {
        return error(
            403,
            "Local MCP requires a direct connection on this computer.",
        );
    }
    let password = base64_basic(&format!("presenter-orbit:{}", settings.password));
    if !password_matches(&request.authorization, &format!("Basic {password}")) {
        return error(401, "Presenter Orbit rejected the MCP password.");
    }
    if request.path == "/api/v1/mcp/presentations" && request.method == "POST" {
        if !settings.create_presentations {
            return error(
                403,
                "Creating presentations is disabled in Local MCP settings.",
            );
        }
        return match create_presentation(app, &request.body) {
            Ok(result) => (201, result),
            Err(message) => error(400, &message),
        };
    }
    if request.method == "POST"
        && matches!(
            request.path.as_str(),
            "/api/v1/mcp/playlists" | "/api/v1/mcp/playlists/delete"
        )
    {
        if !settings.manage_playlists {
            return error(403, "Managing playlists is disabled in Local MCP settings.");
        }
        let create = request.path == "/api/v1/mcp/playlists";
        return match mutate_playlist(app, &request.body, create) {
            Ok(result) => (if create { 201 } else { 200 }, result),
            Err(message) => error(400, &message),
        };
    }
    if request.method != "GET" {
        return error(405, "Local MCP accepts read requests only.");
    }
    let field = match request.path.as_str() {
        "/api/v1/mcp/status" => {
            if !settings.read_status {
                return error(
                    403,
                    "Reading live status is disabled in Local MCP settings.",
                );
            }
            let status = current_status(app, shared);
            return (
                200,
                json!({
                    "workspaceOpen":status["workspacePath"].is_string(),
                    "updatedAt":status["updatedAt"], "presentation":status["presentation"],
                    "livePlaylistId":status["livePlaylistId"], "currentSlide":status["currentSlide"],
                    "outputs":status["outputs"], "layers":status["layers"]
                }),
            );
        }
        "/api/v1/mcp/libraries" => "libraries",
        "/api/v1/mcp/playlists" => "playlists",
        "/api/v1/mcp/themes" => "themes",
        _ => return error(404, "Local MCP route not found."),
    };
    if !settings.read_catalog {
        return error(
            403,
            "Reading libraries, playlists, and themes is disabled in Local MCP settings.",
        );
    }
    if field == "themes" {
        return match themes(app) {
            Ok(value) => (200, value),
            Err(message) => error(503, &message),
        };
    }
    match catalog(&current_status(app, shared), field) {
        Ok(value) => (200, value),
        Err(message) => error(503, &message),
    }
}

fn catalog(status: &Value, field: &str) -> Result<Value, String> {
    if let Some(error) = status["controlsError"].as_str() {
        return Err(format!("The workspace catalog is unavailable: {error}"));
    }
    let items = entries(status, field)?
        .iter()
        .map(|item| {
            if field == "libraries" {
                let presentations = entries(item, "presentations")?
                    .iter()
                    .map(|item| {
                        json!({
                            "id":item["id"],"title":item["title"],
                            "arrangementId":item["arrangementId"],
                            "arrangements":item["modern"]["arrangements"]
                        })
                    })
                    .collect::<Vec<_>>();
                return Ok(
                    json!({"id":item["id"],"title":item["title"],"presentations":presentations}),
                );
            }
            let metadata = &item["modern"];
            Ok(json!({
                "id":item["id"], "title":item["title"], "type":metadata["type"],
                "parentId":metadata["parentId"], "entries":entries(metadata, "items")?
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({field: items, "workspaceId": status["workspacePath"].as_str().map(workspace_id)}))
}

fn workspace_id(path: &str) -> String {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, path.as_bytes()).to_string()
}

fn themes(app: &Host) -> Result<Value, String> {
    let state = app.state::<crate::AppState>();
    let path = crate::active_workspace(state, Value::Null)?;
    let path = path.to_str().ok_or("Workspace path is not text.")?;
    let snapshot = app.invoke("readWorkspaceSnapshot", vec![json!(path)])?;
    let themes = entries(&snapshot["workspace"], "themes")?
        .iter()
        .map(|theme| {
            let theme_id = theme["id"].as_str().ok_or("Theme ID is required.")?;
            let layouts = entries(theme, "slides")?
                .iter()
                .map(|layout| {
                    let layout_id = layout["id"]
                        .as_str()
                        .ok_or("Theme layout ID is required.")?;
                    Ok(json!({"id":format!("{theme_id}::{layout_id}"),"name":layout["name"]}))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(json!({"id":theme_id,"title":theme["title"],"layouts":layouts}))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({"themes":themes,"workspaceId":workspace_id(path)}))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Presentation {
    workspace_id: String,
    title: String,
    library_id: String,
    theme_layout_id: String,
    width: u32,
    height: u32,
    slides: Vec<Slide>,
    playlist_id: Option<String>,
    operator_notes: Option<String>,
    arrangement: Option<Value>,
    layer: Option<Layer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Slide {
    text: String,
    notes: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum Layer {
    Slides,
    Announcements,
}

fn create_presentation(app: &Host, body: &[u8]) -> Result<Value, String> {
    let input: Presentation =
        serde_json::from_slice(body).map_err(|error| format!("Invalid presentation: {error}"))?;
    let (path, revision) = workspace_target(app, &input.workspace_id)?;
    let mut mutation = json!({
        "type":"createPresentation", "title":input.title, "libraryId":input.library_id,
        "presentationThemeId":input.theme_layout_id,
        "size":{"width":input.width,"height":input.height},
        "content":"", "splitMode":"line", "splitMarker":"", "splitUnits":1,
        "slideContents":input.slides.iter().map(|slide| &slide.text).collect::<Vec<_>>(),
        "slideNotes":input.slides.iter().map(|slide| &slide.notes).collect::<Vec<_>>(),
    });
    if let Some(playlist_id) = &input.playlist_id {
        mutation["playlistId"] = json!(playlist_id);
    }
    if let Some(notes) = input.operator_notes {
        mutation["operatorNotes"] = json!(notes);
    }
    if let Some(arrangement) = input.arrangement {
        mutation["arrangement"] = arrangement;
    }
    if matches!(input.layer, Some(Layer::Announcements)) {
        mutation["destination"] = json!("announcements");
    }
    let accepted = app.invoke(
        "mutatePresentationLibrary",
        vec![json!(path), mutation, json!(revision)],
    )?;
    let id = &accepted["createdItemId"];
    let item = entries(&accepted["workspace"], "library")?
        .iter()
        .find(|item| item["id"] == *id)
        .ok_or("The saved presentation is missing from the workspace.")?;
    let result = json!({
        "id": id, "title": item["title"], "libraryId":input.library_id,
        "playlistId": input.playlist_id, "arrangementId":accepted["createdArrangementId"],
        "revision":accepted["revision"]
    });
    let _ = app.emit_to("main", "workspace-mcp-mutated", accepted);
    Ok(result)
}

fn entries<'a>(value: &'a Value, field: &str) -> Result<&'a [Value], String> {
    value[field]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("Presenter Orbit status is missing the {field} catalog."))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreatePlaylist {
    workspace_id: String,
    title: String,
    #[serde(default)]
    entries: Vec<Value>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeletePlaylist {
    workspace_id: String,
    playlist_id: String,
}

fn mutate_playlist(app: &Host, body: &[u8], create: bool) -> Result<Value, String> {
    let (expected_id, mutation) = if create {
        let input: CreatePlaylist = serde_json::from_slice(body)
            .map_err(|error| format!("Invalid playlist creation: {error}"))?;
        (
            input.workspace_id,
            json!({"type":"createWithEntries","baseTitle":input.title,"entries":input.entries}),
        )
    } else {
        let input: DeletePlaylist = serde_json::from_slice(body)
            .map_err(|error| format!("Invalid playlist deletion: {error}"))?;
        (
            input.workspace_id,
            json!({"type":"deletePlaylist","playlistId":input.playlist_id}),
        )
    };
    let (path, revision) = workspace_target(app, &expected_id)?;
    let accepted = app.invoke(
        "mutatePresentationPlaylist",
        vec![json!(path), mutation, json!(revision)],
    )?;
    let result = if create {
        let id = &accepted["createdPlaylistIds"][0];
        let playlist = entries(&accepted["workspace"], "playlist")?
            .iter()
            .find(|playlist| playlist["id"] == *id)
            .ok_or("The saved playlist is missing from the workspace.")?;
        json!({"id":id,"title":playlist["title"],"entries":playlist["items"],"revision":accepted["revision"]})
    } else {
        json!({"id":accepted["deletedPlaylistIds"][0],"revision":accepted["revision"]})
    };
    let _ = app.emit_to("main", "workspace-mcp-mutated", accepted);
    Ok(result)
}

fn workspace_target(app: &Host, expected_id: &str) -> Result<(std::path::PathBuf, u64), String> {
    let state = app.state::<crate::AppState>();
    let path = crate::active_workspace(state, Value::Null)?;
    let path_text = path.to_str().ok_or("Workspace path is not text.")?;
    if expected_id != workspace_id(path_text) {
        return Err("The workspace has changed. Read the current workspace catalog.".into());
    }
    let revision = {
        let mut io = crate::lock_workspace_io(state)?;
        io.require_active(&path)?;
        io.revision(&path)
    };
    Ok((path, revision))
}
