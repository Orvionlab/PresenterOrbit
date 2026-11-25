use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Read, Write},
    time::Duration,
};

const DEFAULT_PROTOCOL: &str = "2025-11-25";
const STATELESS_PROTOCOL: &str = "2026-07-28";
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_STATUS_BYTES: u64 = 128 * 1024 * 1024;
const READ_TOOLS: [(&str, &str, &str); 4] = [
    (
        "get_live_status",
        "Read the live presentation, current slide, and accepted output state.",
        "status",
    ),
    (
        "list_libraries",
        "List libraries and their presentations with Presenter Orbit IDs.",
        "libraries",
    ),
    (
        "list_playlists",
        "List playlists and their ordered entries with Presenter Orbit IDs.",
        "playlists",
    ),
    (
        "list_themes",
        "List themes and the layout IDs accepted by create_presentation.",
        "themes",
    ),
];

pub fn run() -> Result<(), String> {
    let orbit = Orbit::from_environment()?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut message = Vec::new();
    let mut session = Session::default();
    loop {
        message.clear();
        let size = (&mut input)
            .take(MAX_MESSAGE_BYTES as u64 + 1)
            .read_until(b'\n', &mut message)
            .map_err(|error| error.to_string())?;
        if size == 0 {
            return Ok(());
        }
        if size > MAX_MESSAGE_BYTES {
            return Err("MCP message exceeds the input size limit.".into());
        }
        let response = match serde_json::from_slice(&message) {
            Ok(request) => session.handle(&request, &orbit),
            Err(_) => Some(rpc_error(&Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response).map_err(|error| error.to_string())?;
            output
                .write_all(b"\n")
                .and_then(|()| output.flush())
                .map_err(|error| error.to_string())?;
        }
    }
}

#[derive(Default, PartialEq)]
enum Session {
    #[default]
    New,
    Initializing,
    Ready,
}

impl Session {
    fn handle(&mut self, request: &Value, orbit: &Orbit) -> Option<Value> {
        let id = request.get("id");
        let method = request.get("method").and_then(Value::as_str);
        if request["jsonrpc"] != "2.0"
            || method.is_none()
            || id.is_some_and(|value| !value.is_string() && !value.is_i64() && !value.is_u64())
        {
            return Some(rpc_error(&Value::Null, -32600, "Invalid request"));
        }
        let params = &request["params"];
        let method = method?;
        let Some(id) = id else {
            if method == "notifications/initialized" && *self == Self::Initializing {
                *self = Self::Ready;
            }
            return None;
        };
        let metadata = &params["_meta"];
        let version = metadata.get("io.modelcontextprotocol/protocolVersion");
        let stateless = method == "server/discover" || version.is_some();
        if stateless {
            let Some(version) = version.and_then(Value::as_str) else {
                return Some(rpc_error(id, -32602, "A protocol version is required"));
            };
            if version != STATELESS_PROTOCOL {
                let mut error = rpc_error(id, -32022, "Unsupported protocol version");
                error["error"]["data"] =
                    json!({"requested":version,"supported":[STATELESS_PROTOCOL]});
                return Some(error);
            }
            if !metadata["io.modelcontextprotocol/clientCapabilities"].is_object() {
                return Some(rpc_error(id, -32602, "Client capabilities are required"));
            }
        }
        let server_info =
            || json!({"name": "presenter-orbit", "version": env!("CARGO_PKG_VERSION")});
        let mut result = match method {
            "server/discover" => json!({
                "supportedVersions": [STATELESS_PROTOCOL],
                "capabilities": {"tools": {}},
                "ttlMs": 0,
                "cacheScope": "public"
            }),
            "initialize" if !stateless && *self == Self::New => {
                let Some(version) = params["protocolVersion"].as_str() else {
                    return Some(rpc_error(id, -32602, "A protocolVersion is required"));
                };
                *self = Self::Initializing;
                let version = match version {
                    "2024-11-05" | "2025-03-26" | "2025-06-18" | DEFAULT_PROTOCOL => version,
                    _ => DEFAULT_PROTOCOL,
                };
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": server_info()
                })
            }
            "ping" => json!({}),
            _ if !stateless && *self != Self::Ready => {
                return Some(rpc_error(id, -32002, "Server is not initialized"));
            }
            "tools/list" => json!({"tools": tools()}),
            "tools/call" => match call_tool(params, orbit) {
                Ok(result) => result,
                Err(message) => return Some(rpc_error(id, -32602, message)),
            },
            _ => return Some(rpc_error(id, -32601, "Method not found")),
        };
        if stateless {
            result["resultType"] = json!("complete");
            result["_meta"] = json!({"io.modelcontextprotocol/serverInfo": server_info()});
            if method == "tools/list" {
                result["ttlMs"] = json!(0);
                result["cacheScope"] = json!("public");
            }
        }
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}

fn rpc_error(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn call_tool(params: &Value, orbit: &Orbit) -> Result<Value, &'static str> {
    let name = params["name"].as_str().ok_or("A tool name is required")?;
    let write_path = match name {
        "create_presentation" => Some("presentations"),
        "create_playlist" => Some("playlists"),
        "delete_playlist" => Some("playlists/delete"),
        _ => None,
    };
    let result = if let Some(path) = write_path {
        if !params["arguments"].is_object() {
            return Err("Tool arguments must be an object");
        }
        orbit.request(path, Some(&params["arguments"]))
    } else {
        let (_, _, path) = READ_TOOLS
            .iter()
            .find(|(tool, _, _)| *tool == name)
            .ok_or("Unknown tool")?;
        if params
            .get("arguments")
            .is_some_and(|args| !args.as_object().is_some_and(serde_json::Map::is_empty))
        {
            return Err("This tool takes no arguments");
        }
        orbit.request(path, None)
    };
    Ok(match result {
        Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}]}),
        Err(error) => json!({"content":[{"type":"text","text":error}],"isError":true}),
    })
}

fn tools() -> Vec<Value> {
    let mut tools = READ_TOOLS
        .map(|(name, description, _)| {
            json!({
                "name": name, "description": description,
                "inputSchema": {"type": "object", "additionalProperties": false},
                "annotations": {"readOnlyHint": true}
            })
        })
        .to_vec();
    tools.push(json!({
        "name":"create_presentation",
        "description":"Save a new presentation with a theme layout, resolution, and one text value per slide. Append it to an existing playlist when requested. Requires the Create presentations permission.",
        "annotations":{"readOnlyHint":false,"destructiveHint":false,"idempotentHint":false,"openWorldHint":false},
        "inputSchema":{
            "type":"object", "additionalProperties":false,
            "required":["workspaceId","title","libraryId","themeLayoutId","width","height","slides"],
            "properties":{
                "workspaceId":{"type":"string","description":"Workspace ID from list_libraries or list_themes."},
                "title":{"type":"string","minLength":1},
                "libraryId":{"type":"string","description":"Library ID from list_libraries."},
                "themeLayoutId":{"type":"string","description":"Layout ID from list_themes."},
                "width":{"type":"integer","minimum":1,"maximum":16384},
                "height":{"type":"integer","minimum":1,"maximum":16384},
                "slides":{"type":"array","minItems":1,"items":{
                    "type":"object","additionalProperties":false,"required":["text"],
                    "properties":{"text":{"type":"string"},"notes":{"type":"string"}}
                }},
                "playlistId":{"type":"string","description":"Existing playlist ID from list_playlists. Appends at the end."},
                "operatorNotes":{"type":"string"},
                "layer":{"type":"string","enum":["slides","announcements"],"default":"slides"},
                "arrangement":{"type":"object","additionalProperties":false,"required":["name","slideOrder"],
                    "properties":{
                        "name":{"type":"string","minLength":1},
                        "slideOrder":{"type":"array","minItems":1,"items":{"type":"integer","minimum":1},"description":"One-based slide numbers in playback order. Numbers may repeat."}
                    }
                }
            }
        }
    }));
    let workspace_id = json!({
        "type":"string","description":"Workspace ID from list_playlists, list_libraries or list_themes."
    });
    let arrangement = json!({
        "type":"string","description":"Arrangement ID for this entry's presentation from list_libraries."
    });
    tools.extend([
        json!({
            "name":"create_playlist",
            "description":"Create a playlist with ordered headers, linked or unlinked placeholders, and library presentations. Optional theme and size change the source presentation everywhere it is used. Requires the Create and delete playlists permission.",
            "annotations":{"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":false},
            "inputSchema":{
                "type":"object","additionalProperties":false,"required":["workspaceId","title"],
                "properties":{
                    "workspaceId":workspace_id,
                    "title":{"type":"string","minLength":1},
                    "entries":{"type":"array","items":{"oneOf":[
                        {"type":"object","additionalProperties":false,"required":["type","title"],
                            "properties":{
                                "type":{"const":"header"},
                                "title":{"type":"string","minLength":1},
                                "color":{"type":"string","pattern":"^(transparent|#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?)$","description":"Optional header color as #RRGGBB, #RRGGBBAA, or transparent."}
                            }},
                        {"type":"object","additionalProperties":false,"required":["type","title"],
                            "properties":{
                                "type":{"const":"placeholder"},
                                "title":{"type":"string","minLength":1},
                                "linkedItemId":{"type":"string","description":"Optional presentation ID from list_libraries."},
                                "arrangementId":arrangement
                            }},
                        {"type":"object","additionalProperties":false,"required":["type","itemId"],
                            "properties":{
                                "type":{"const":"presentation"},
                                "itemId":{"type":"string","description":"Presentation ID from list_libraries."},
                                "arrangementId":arrangement,
                                "themeLayoutId":{"type":"string","description":"Apply this layout ID from list_themes to every slide of the source presentation."},
                                "size":{"type":"object","additionalProperties":false,"required":["width","height"],"properties":{
                                    "width":{"type":"integer","minimum":1,"maximum":16384},
                                    "height":{"type":"integer","minimum":1,"maximum":16384}
                                }}
                            }}
                    ]}}
                }
            }
        }),
        json!({
            "name":"delete_playlist",
            "description":"Delete one playlist and its entries by ID. Library presentations are preserved. Requires the Create and delete playlists permission.",
            "annotations":{"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":false},
            "inputSchema":{
                "type":"object","additionalProperties":false,"required":["workspaceId","playlistId"],
                "properties":{
                    "workspaceId":workspace_id,
                    "playlistId":{"type":"string","description":"Playlist ID from list_playlists."}
                }
            }
        }),
    ]);
    tools
}

struct Orbit {
    agent: ureq::Agent,
    endpoint: String,
    authorization: String,
}

impl Orbit {
    fn from_environment() -> Result<Self, String> {
        let password = std::env::var("PRESENTER_ORBIT_MCP_PASSWORD")
            .ok()
            .filter(|value| !value.is_empty())
            .ok_or("Set PRESENTER_ORBIT_MCP_PASSWORD to the Presenter Orbit MCP password.")?;
        let port = match std::env::var("PRESENTER_ORBIT_MCP_PORT") {
            Ok(value) => value
                .parse::<u16>()
                .ok()
                .filter(|port| *port >= 1024)
                .ok_or("PRESENTER_ORBIT_MCP_PORT must be a port from 1024 through 65535.")?,
            Err(std::env::VarError::NotPresent) => 49_324,
            Err(error) => return Err(error.to_string()),
        };
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .new_agent();
        Ok(Self {
            agent,
            endpoint: format!("http://127.0.0.1:{port}/api/v1/mcp"),
            authorization: format!(
                "Basic {}",
                STANDARD.encode(format!("presenter-orbit:{password}"))
            ),
        })
    }

    fn request(&self, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let url = format!("{}/{path}", self.endpoint);
        let response = match body {
            Some(body) => self
                .agent
                .post(&url)
                .header("Authorization", &self.authorization)
                .header("Content-Type", "application/json")
                .send(body.to_string()),
            None => self
                .agent
                .get(&url)
                .header("Authorization", &self.authorization)
                .call(),
        };
        let mut response = response.map_err(|error| format!(
                    "Could not reach Presenter Orbit at {}. Check the Local MCP setting and port: {error}",
                    self.endpoint
                ))?;
        let code = response.status().as_u16();
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_STATUS_BYTES)
            .read_to_vec()
            .map_err(|error| format!("Could not read Presenter Orbit status: {error}"))?;
        let status: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Presenter Orbit returned invalid status JSON: {error}"))?;
        if !matches!(code, 200 | 201) {
            return Err(status["error"].as_str().map_or_else(
                || format!("Presenter Orbit returned HTTP {code}."),
                str::to_string,
            ));
        }
        if !status.is_object() {
            return Err("Presenter Orbit returned an invalid MCP response.".into());
        }
        Ok(status)
    }
}
