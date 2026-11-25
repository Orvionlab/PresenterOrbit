use gpui::{AnyElement, ClipboardItem, Context, IntoElement, ParentElement, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant};
use orbit_ui::Translator;
use serde_json::json;

use super::{remote::valid_port, rows, ui, SettingsView};

impl SettingsView {
    pub(super) fn mcp_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let settings = self.settings(cx);
        let remote = &settings["remoteSettings"];
        let port = self.number_control(
            "mcp-port",
            &remote["mcpPort"].to_string(),
            (1024., 65535.),
            1.,
            window,
            cx,
            |view, text, cx| {
                if let Some(port) = valid_port(&text) {
                    view.patch_remote(json!({ "mcpPort": port }), cx);
                }
            },
        );
        let password = self.password_control(
            "mcp-password",
            t.t("settings.remote.password"),
            remote["mcpPassword"].as_str().unwrap_or_default(),
            window,
            cx,
            |view, password, cx| view.patch_remote(json!({ "mcpPassword": password }), cx),
        );
        let toggle = |id: &'static str, label, field: &'static str| {
            let weak = cx.weak_entity();
            rows::toggle_row(
                id,
                t.t(label),
                None,
                remote[field] == true,
                move |value, _, cx| {
                    let _ = weak.update(cx, |view, cx| {
                        view.patch_remote(json!({field: value}), cx);
                    });
                },
                cx,
            )
        };
        rows::column(cx)
            .child(ui::title(t.t("settings.remote.mcp.title"), cx))
            .child(rows::group(
                [
                    toggle("mcp-enabled", "settings.remote.mcp.enabled", "mcpEnabled"),
                    rows::setting_field_row(
                        "mcp-password-field",
                        t.t("settings.remote.password"),
                        password,
                        cx,
                    ),
                    rows::setting_field_row(
                        "mcp-port-field",
                        t.t("settings.remote.port"),
                        port,
                        cx,
                    ),
                ],
                cx,
            ))
            .child(ui::subheading(t.t("settings.remote.mcp.permissions"), cx))
            .child(rows::group(
                [
                    toggle(
                        "mcp-read-status",
                        "settings.remote.mcp.readStatus",
                        "mcpReadStatus",
                    ),
                    toggle(
                        "mcp-read-catalog",
                        "settings.remote.mcp.readCatalog",
                        "mcpReadCatalog",
                    ),
                    toggle(
                        "mcp-create-presentations",
                        "settings.remote.mcp.createPresentations",
                        "mcpCreatePresentations",
                    ),
                    toggle(
                        "mcp-manage-playlists",
                        "settings.remote.mcp.managePlaylists",
                        "mcpManagePlaylists",
                    ),
                ],
                cx,
            ))
            .child(mcp_setup(&t, &remote["mcpPort"].to_string(), cx))
            .into_any_element()
    }
}

fn mcp_setup(t: &Translator, port: &str, cx: &gpui::App) -> gpui::Div {
    let executable = std::env::current_exe().ok();
    let path = executable
        .as_ref()
        .and_then(|path| path.to_str())
        .unwrap_or("path/to/orbit-app");
    let prompt = t
        .t("settings.remote.mcp.passwordPrompt")
        .replace('\'', "''");
    let password =
        format!("(\"PRESENTER_ORBIT_MCP_PASSWORD=\" + (Read-Host -MaskInput '{prompt}'))");
    let port = format!("'PRESENTER_ORBIT_MCP_PORT={port}'");
    let launch = format!("-- '{}' --mcp", path.replace('\'', "''"));
    let codex = format!("codex mcp add presenter_orbit --env {password} --env {port} {launch}");
    let claude = format!(
        "claude mcp add --transport stdio --scope user presenter_orbit --env {password} {port} {launch}"
    );
    let copy = |id, value: String| {
        ActionButton::new(id, t.t("settings.remote.mcp.copyCommand"))
            .variant(ButtonVariant::Secondary)
            .compact()
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(value.clone()))
            })
    };
    ui::stack(cx)
        .child(ui::subheading(t.t("settings.remote.mcp.setup"), cx))
        .child(ui::muted(t.t("settings.remote.mcp.setupIntro"), cx))
        .child(rows::group(
            [
                rows::setting_row(
                    "Claude Code",
                    Some(t.t("settings.remote.mcp.setupClaude").into()),
                    copy("mcp-copy-claude", claude),
                    cx,
                ),
                rows::setting_row(
                    "Codex",
                    Some(t.t("settings.remote.mcp.setupCodex").into()),
                    copy("mcp-copy-codex", codex),
                    cx,
                ),
            ],
            cx,
        ))
        .child(ui::muted(t.t("settings.remote.mcp.setupVerify"), cx))
}
