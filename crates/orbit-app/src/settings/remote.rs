//! `RemoteSettingsPanel.tsx` and `use-remote-network-address.ts`. `updateRemoteSettings` merges the patch into the
//! saved remote settings and saves; the host validates ports and passwords.

use gpui::{div, prelude::*, AnyElement, Context, Window};
use orbit_ui::controls::{Disclosure, ToggleSwitch};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::persist::save_payload;
use super::ui::muted;
use super::view::SettingsView;

#[derive(Default)]
pub struct Remote {
    pub address: Option<Result<String, String>>,
    pub requested: bool,
    pub link_open: bool,
}

/// The ports `PortInput` accepts.
pub fn valid_port(text: &str) -> Option<u16> {
    text.trim()
        .parse::<u32>()
        .ok()
        .filter(|port| (1024..=65535).contains(port))
        .map(|port| port as u16)
}

impl SettingsView {
    pub(super) fn patch_remote(&mut self, patch: Value, cx: &mut Context<Self>) {
        let settings = self.settings(cx);
        let mut remote = settings["remoteSettings"].clone();
        if let (Some(target), Some(patch)) = (remote.as_object_mut(), patch.as_object()) {
            target.extend(patch.clone());
        }
        self.save(save_payload(&settings, &[("remoteSettings", remote)]), cx);
    }

    #[cfg_attr(test, allow(dead_code))]
    pub(super) fn load_remote_address(&mut self, cx: &mut Context<Self>) {
        if self.remote.requested {
            return;
        }
        self.remote.requested = true;
        self.host_call(
            "getRemoteNetworkAddress",
            Vec::new(),
            cx,
            |view, result, cx| {
                view.remote.address = Some(result.and_then(|address| {
                    address
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| "The network address is not text.".to_string())
                }));
                cx.notify();
            },
        );
    }

    fn remote_toggle(
        &self,
        id: &'static str,
        label: String,
        checked: bool,
        field: &'static str,
        cx: &mut Context<Self>,
    ) -> ToggleSwitch {
        let weak = cx.weak_entity();
        ToggleSwitch::new(id, label, checked).on_change(move |value, _, cx| {
            weak.update(cx, |view, cx| {
                view.patch_remote(json!({ field: value }), cx)
            })
            .ok();
        })
    }

    pub(super) fn remote_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(not(test))]
        self.load_remote_address(cx);
        let t = *cx.global::<Translator>();
        let settings = self.settings(cx);
        let remote = settings["remoteSettings"].clone();
        let text = |key: &str| remote[key].as_str().unwrap_or("").to_string();
        let enabled = remote["enabled"] == json!(true);
        let address = match &self.remote.address {
            Some(Ok(address)) => address.clone(),
            Some(Err(_)) => t.t("settings.remote.addressUnavailable"),
            None => t.t("settings.remote.addressLoading"),
        };
        let toggle = self.remote_toggle(
            "remote-enabled",
            t.t("settings.remote.networkEnabled"),
            enabled,
            "enabled",
            cx,
        );
        self.inputs.2 = !enabled;
        let name = self.text_control(
            "remote-network-name",
            &text("networkName"),
            window,
            cx,
            |view, name, cx| {
                if name.chars().count() <= 63 {
                    view.patch_remote(json!({ "networkName": name }), cx);
                }
            },
        );
        let port = self.number_control(
            "remote-port",
            &remote["port"].to_string(),
            (1024., 65535.),
            1.,
            window,
            cx,
            |view, text, cx| {
                if let Some(port) = valid_port(&text) {
                    view.patch_remote(json!({ "port": port }), cx);
                }
            },
        );
        let pro_password = self.password_control(
            "remote-pro-password",
            t.t("settings.remote.proPresenter.remotePassword"),
            &text("password"),
            window,
            cx,
            |view, password, cx| view.patch_remote(json!({ "password": password }), cx),
        );
        let stage_password = self.password_control(
            "remote-stage-password",
            t.t("settings.remote.proPresenter.stagePassword"),
            &text("proPresenterStagePassword"),
            window,
            cx,
            |view, password, cx| {
                view.patch_remote(json!({ "proPresenterStagePassword": password }), cx)
            },
        );
        let tcp_port = self.number_control(
            "remote-tcp-port",
            &remote["tcpPort"].to_string(),
            (1024., 65535.),
            1.,
            window,
            cx,
            |view, text, cx| {
                if let Some(port) = valid_port(&text) {
                    view.patch_remote(json!({ "tcpPort": port }), cx);
                }
            },
        );
        self.inputs.2 = false;
        let link = if self.remote.link_open {
            self.network_link_section(window, cx)
        } else {
            div().into_any_element()
        };
        let link_weak = cx.weak_entity();
        let pro_toggle = self
            .remote_toggle(
                "remote-pro-enabled",
                t.t("settings.remote.proPresenter.enabled"),
                remote["proPresenterEnabled"] == json!(true),
                "proPresenterEnabled",
                cx,
            )
            .disabled(!enabled);
        let stage_toggle = self
            .remote_toggle(
                "remote-stage-enabled",
                t.t("settings.remote.stage.enabled"),
                remote["stageEnabled"] == json!(true),
                "stageEnabled",
                cx,
            )
            .disabled(!enabled);
        let tcp_toggle = self
            .remote_toggle(
                "remote-tcp-enabled",
                t.t("settings.remote.tcp.enabled"),
                remote["tcpEnabled"] == json!(true),
                "tcpEnabled",
                cx,
            )
            .disabled(!enabled);
        let theme = *cx.theme();
        let panel = |children: Vec<AnyElement>| {
            div()
                .flex()
                .flex_col()
                .rounded(theme.metrics.radius_sm)
                .bg(theme.colors.panel_bg_strong)
                .children(children)
        };
        let settings_row = |id: &'static str, label: String, control: AnyElement, first: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_between()
                .gap(theme.metrics.space_md)
                .min_h(gpui::px(44.))
                .px(theme.metrics.space_md)
                .py(theme.metrics.space_sm)
                .text_size(theme.metrics.font_sm)
                .when(!first, |this| {
                    this.border_t_1().border_color(theme.colors.border)
                })
                .child(div().flex_1().min_w_0().child(label))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .w(gpui::px(220.))
                        .min_w(gpui::px(120.))
                        .child(control),
                )
                .into_any_element()
        };
        let app_row = |label: String,
                       field_label: String,
                       control: AnyElement,
                       control_width: gpui::Pixels,
                       toggle: ToggleSwitch| {
            div()
                .flex()
                .items_center()
                .gap(theme.metrics.space_sm)
                .rounded(theme.metrics.radius_sm)
                .bg(theme.colors.panel_bg_strong)
                .px(theme.metrics.space_md)
                .py(theme.metrics.space_sm)
                .text_size(theme.metrics.font_sm)
                .child(div().flex_1().min_w_0().child(label))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(theme.metrics.space_sm)
                        .w(control_width)
                        .child(div().flex_none().child(field_label))
                        .child(div().flex_1().min_w_0().child(control)),
                )
                .child(toggle.bare())
        };
        let section = |heading: AnyElement, row: gpui::Div| {
            div()
                .flex()
                .flex_col()
                .gap(theme.metrics.space_sm)
                .child(heading)
                .child(row)
        };
        let h4 = |label: String| {
            div()
                .text_size(theme.metrics.font_lg)
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(theme.colors.text_strong)
                .child(label)
                .into_any_element()
        };
        let network_rows = vec![
            settings_row(
                "remote-network-enabled-row",
                t.t("settings.remote.networkEnabled"),
                toggle.bare().into_any_element(),
                true,
            ),
            div()
                .border_t_1()
                .border_color(theme.colors.border)
                .when(!enabled, |this| this.opacity(0.5))
                .child(settings_row(
                    "remote-name-row",
                    t.t("settings.remote.networkName"),
                    name,
                    true,
                ))
                .child(settings_row(
                    "remote-address-row",
                    t.t("settings.remote.address"),
                    div().id("remote-address").child(address).into_any_element(),
                    false,
                ))
                .child(settings_row(
                    "remote-port-row",
                    t.t("settings.remote.port"),
                    div().w(gpui::px(96.)).child(port).into_any_element(),
                    false,
                ))
                .into_any_element(),
        ];
        div()
            .id("settings-remote")
            .flex()
            .flex_col()
            .gap(theme.metrics.panel_gap)
            .child(
                div()
                    .text_size(theme.metrics.font_xl)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(theme.colors.text_strong)
                    .child(t.t("settings.remote.title")),
            )
            .child(panel(network_rows))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme.metrics.space_lg)
                    .when(!enabled, |this| this.opacity(0.5))
                    .child(
                        div()
                            .rounded(theme.metrics.radius_sm)
                            .bg(theme.colors.panel_bg_strong)
                            .p(theme.metrics.space_md)
                            .child(
                                Disclosure::new(
                                    "remote-network-link",
                                    t.t("networkLink.title"),
                                    self.remote.link_open,
                                )
                                .on_toggle(move |open, _, cx| {
                                    link_weak
                                        .update(cx, |view, cx| {
                                            view.remote.link_open = open;
                                            cx.notify();
                                        })
                                        .ok();
                                })
                                .child(link),
                            ),
                    )
                    .child(section(
                        h4(t.t("settings.remote.proPresenter.title")),
                        app_row(
                            t.t("settings.remote.enable"),
                            t.t("settings.remote.password"),
                            pro_password,
                            gpui::px(310.),
                            pro_toggle,
                        ),
                    ))
                    .child(section(
                        h4(t.t("settings.remote.stage.title")),
                        app_row(
                            t.t("settings.remote.enable"),
                            t.t("settings.remote.password"),
                            stage_password,
                            gpui::px(310.),
                            stage_toggle,
                        ),
                    ))
                    .child(section(
                        h4(t.t("settings.remote.tcp.title")),
                        app_row(
                            t.t("settings.remote.enable"),
                            t.t("settings.remote.port"),
                            tcp_port,
                            gpui::px(160.),
                            tcp_toggle,
                        ),
                    ))
                    .child(muted(t.t("settings.remote.browser.sharedHelp"), cx)),
            )
            .into_any_element()
    }
}
