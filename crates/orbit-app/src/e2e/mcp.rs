use super::{seed, Scenario};
use base64::{engine::general_purpose::STANDARD, Engine};
use gpui::{AppContext, TestAppContext};
use serde_json::{json, Value};
use std::{
    io::Write,
    net::TcpListener,
    process::{Command, Stdio},
    time::Duration,
};

#[gpui::test]
fn mcp_reads_the_open_workspace_over_the_remote_api(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, seed);
    let port = TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let settings = app.host.invoke("getSettings", vec![]).unwrap();
    app.host
        .invoke(
            "saveSettings",
            vec![
                json!({"remoteSettings": {
                    "enabled": false,
                    "proPresenterEnabled": false,
                    "mcpEnabled": true,
                    "mcpPort": port,
                    "password": "remote-e2e-password",
                    "mcpPassword": "mcp-e2e-password"
                }}),
                settings["settingsRevision"].clone(),
            ],
        )
        .unwrap();
    let agent = ureq::Agent::config_builder()
        .proxy(None)
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .new_agent();
    let address = format!("http://127.0.0.1:{port}");
    let mcp_status = format!("{address}/api/v1/mcp/status");
    let basic = |password: &str| {
        format!(
            "Basic {}",
            STANDARD.encode(format!("presenter-orbit:{password}"))
        )
    };
    assert!(matches!(
        agent
            .get(&mcp_status)
            .header("Authorization", basic("remote-e2e-password"))
            .call(),
        Err(ureq::Error::StatusCode(401))
    ));
    assert!(matches!(
        agent
            .get(&mcp_status)
            .header("Origin", "https://example.invalid")
            .header("Authorization", basic("mcp-e2e-password"))
            .call(),
        Err(ureq::Error::StatusCode(403))
    ));
    assert!(matches!(
        agent
            .get(format!("{address}/api/v1/status"))
            .header("Authorization", basic("mcp-e2e-password"))
            .call(),
        Err(ureq::Error::StatusCode(404))
    ));
    let metadata = modern_metadata();
    let responses = mcp_requests(
        port,
        [
            json!({"id":"discover","method":"server/discover","params":{"_meta":metadata}}),
            json!({"id":"modern","method":"tools/list","params":{"_meta":metadata}}),
            json!({"id":0,"method":"initialize","params":{
                "protocolVersion":"2025-11-25","capabilities":{},
                "clientInfo":{"name":"orbit-e2e","version":"1"}
            }}),
            json!({"method":"notifications/initialized"}),
            json!({"id":1,"method":"tools/list"}),
            json!({"id":2,"method":"tools/call","params":{"name":"get_live_status"}}),
            json!({"id":3,"method":"tools/call","params":{"name":"list_libraries","arguments":{}}}),
            json!({"id":4,"method":"tools/call","params":{"name":"list_playlists"}}),
            json!({"id":5,"method":"tools/call","params":{"name":"trigger_cue"}}),
            json!({"id":6,"method":"tools/call","params":{"name":"list_libraries","arguments":{"id":"invalid"}}}),
            json!({"id":7,"method":"tools/call","params":{"name":"list_themes"}}),
        ],
    );
    assert_eq!(responses.len(), 10, "{responses:?}");
    assert_eq!(
        responses[0]["result"]["supportedVersions"],
        json!(["2026-07-28"])
    );
    assert_eq!(responses[1]["result"]["resultType"], "complete");
    let responses = &responses[2..];
    assert_eq!(responses[0]["result"]["protocolVersion"], "2025-11-25");
    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 5);
    assert!(
        tools
            .iter()
            .all(|tool| tool["annotations"]["readOnlyHint"]
                == (tool["name"] != "create_presentation"))
    );
    let tool_content = |id: usize| -> Value {
        let response = &responses[id];
        assert_eq!(response["id"], id);
        let result = &response["result"];
        assert_ne!(result["isError"], true, "{result}");
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    assert_eq!(tool_content(2)["workspaceOpen"], true);
    let libraries = tool_content(3);
    let seeded = libraries["libraries"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|library| library["presentations"].as_array().unwrap())
        .find(|item| item["title"] == "Sunday service")
        .unwrap();
    let playlists = tool_content(4);
    let sunday = playlists["playlists"]
        .as_array()
        .unwrap()
        .iter()
        .find(|playlist| playlist["title"] == "Sunday")
        .unwrap();
    assert_eq!(sunday["entries"][0]["presentationId"], seeded["id"]);
    assert_eq!(responses[5]["error"]["code"], -32602);
    assert_eq!(responses[6]["error"]["code"], -32602);

    let themes = tool_content(7);
    let workspace_id = &libraries["workspaceId"];
    assert!(workspace_id.is_string());
    assert_eq!(*workspace_id, themes["workspaceId"]);
    let layout = themes["themes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|theme| theme["layouts"].as_array().unwrap())
        .next()
        .unwrap();
    let library = libraries["libraries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|library| {
            library["presentations"]
                .as_array()
                .unwrap()
                .contains(seeded)
        })
        .unwrap();
    let arguments = json!({
        "workspaceId":workspace_id,"title":"MCP service","libraryId":library["id"],
        "themeLayoutId":layout["id"],"width":1280,"height":720,"playlistId":sunday["id"],
        "slides":[{"text":"Intro\nSecond line","notes":"First slide note"},{"text":"Close"}],
        "operatorNotes":"Whole presentation note","layer":"announcements",
        "arrangement":{"name":"Alternate","slideOrder":[2,1,2]}
    });
    let create = |arguments: Value| {
        mcp_requests(
            port,
            [json!({"id":"create","method":"tools/call","params":{
                "_meta":modern_metadata(),"name":"create_presentation","arguments":arguments
            }})],
        )
        .remove(0)["result"]
            .clone()
    };
    assert_eq!(create(arguments.clone())["isError"], true);

    let set_permissions = |app: &Scenario, read_status, read_catalog, create_presentations| {
        let settings = app.host.invoke("getSettings", vec![]).unwrap();
        let mut remote = settings["remoteSettings"].clone();
        remote["mcpReadStatus"] = json!(read_status);
        remote["mcpReadCatalog"] = json!(read_catalog);
        remote["mcpCreatePresentations"] = json!(create_presentations);
        app.host
            .invoke(
                "saveSettings",
                vec![
                    json!({"remoteSettings": remote}),
                    settings["settingsRevision"].clone(),
                ],
            )
            .unwrap();
    };
    let read = |path| {
        agent
            .get(format!("{address}/api/v1/mcp/{path}"))
            .header("Authorization", basic("mcp-e2e-password"))
            .call()
    };
    set_permissions(&app, false, true, true);
    assert!(matches!(read("status"), Err(ureq::Error::StatusCode(403))));
    assert!(read("libraries").is_ok());
    set_permissions(&app, true, false, true);
    assert!(read("status").is_ok());
    assert!(matches!(
        read("libraries"),
        Err(ureq::Error::StatusCode(403))
    ));
    assert!(matches!(
        read("playlists"),
        Err(ureq::Error::StatusCode(403))
    ));
    assert!(matches!(read("themes"), Err(ureq::Error::StatusCode(403))));

    let mut malformed = arguments.clone();
    malformed["arrangement"]["slideOrder"] = json!([3]);
    assert_eq!(create(malformed)["isError"], true);
    let mut stale = arguments.clone();
    stale["workspaceId"] = json!("different-workspace");
    assert_eq!(create(stale)["isError"], true);
    let created = create(arguments);
    assert_ne!(created["isError"], true, "{created}");
    let created: Value =
        serde_json::from_str(created["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(created["title"], "MCP service");
    let runtime = app.runtime(cx);
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime
                .content
                .read(cx)
                .items
                .get(created["id"].as_str().unwrap())
                .is_some()
        })
    });
    let workspace = app.snapshot();
    let item = workspace["library"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == created["id"])
        .unwrap();
    assert_eq!(item["size"], json!({"width":1280,"height":720}));
    assert_eq!(item["slides"][0]["content"], "Intro\nSecond line");
    assert_eq!(item["slides"][0]["notes"], "First slide note");
    assert_eq!(item["slides"][0]["presentationThemeId"], layout["id"]);
    assert_eq!(item["slides"][1]["width"], 1280);
    assert_eq!(item["operatorNotes"], "Whole presentation note");
    assert_eq!(item["destination"], "announcements");
    assert_eq!(item["selectedArrangementId"], created["arrangementId"]);
    assert_eq!(
        item["arrangements"][0]["sectionIds"],
        json!(["section-2", "section-1", "section-2"])
    );
    let playlist = workspace["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|playlist| playlist["id"] == sunday["id"])
        .unwrap();
    let entry = playlist["items"].as_array().unwrap().last().unwrap();
    assert_eq!(entry["itemId"], created["id"]);
    assert_eq!(entry["arrangementId"], created["arrangementId"]);

    let runtime = app.runtime(cx);
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            crate::settings::persist::shown(&runtime, cx)["remoteSettings"]["mcpPort"] == port
        })
    });
    let window = cx.add_window(move |window, cx| {
        let view = cx.new(|cx| crate::settings::SettingsView::new(runtime, cx));
        view.update(cx, |view, cx| view.select("mcp", cx));
        orbit_ui::controls::Root::new(view, window, cx)
    });
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    for field in ["mcp-password-field", "mcp-port-field"] {
        assert!(visual.debug_bounds(field).unwrap().size.width >= gpui::px(100.));
    }
    let mut copy = |button| {
        super::click(&mut visual, button);
        let contents = visual.read_from_clipboard().unwrap().text().unwrap();
        assert!(!contents.contains("mcp-e2e-password"));
        contents
    };
    let claude = copy("button-mcp-copy-claude");
    assert_eq!(claude.lines().count(), 1);
    assert!(
        claude.starts_with("claude mcp add --transport stdio --scope user presenter_orbit --env ")
    );
    assert!(claude.contains("Read-Host -MaskInput"));
    assert!(claude.contains(&format!("'PRESENTER_ORBIT_MCP_PORT={port}' -- ")));
    assert!(claude.ends_with("' --mcp"));
    let codex = copy("button-mcp-copy-codex");
    assert_eq!(codex.lines().count(), 1);
    assert!(codex.starts_with("codex mcp add presenter_orbit "));
    assert!(codex.contains("Read-Host -MaskInput"));
    assert!(codex.contains(&format!("--env 'PRESENTER_ORBIT_MCP_PORT={port}'")));
    assert!(codex.ends_with("' --mcp"));
}

fn modern_metadata() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

fn mcp_requests(port: u16, requests: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let executable = std::env::var_os("PRESENTER_ORBIT_E2E_EXECUTABLE").unwrap();
    let mut child = Command::new(&executable)
        .arg("--mcp")
        .env("PRESENTER_ORBIT_MCP_PORT", port.to_string())
        .env("PRESENTER_ORBIT_MCP_PASSWORD", "mcp-e2e-password")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut input = child.stdin.take().unwrap();
        for request in requests {
            let mut request = request;
            request["jsonrpc"] = json!("2.0");
            writeln!(input, "{request}").unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
