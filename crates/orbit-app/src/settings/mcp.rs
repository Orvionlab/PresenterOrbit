use gpui::{AnyElement, Context, IntoElement, ParentElement, Window};
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
                    rows::setting_row(t.t("settings.remote.password"), None, password, cx),
                    rows::setting_row(t.t("settings.remote.port"), None, port, cx),
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
                ],
                cx,
            ))
            .into_any_element()
    }
}
