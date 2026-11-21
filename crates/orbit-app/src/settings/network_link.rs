//! `NetworkLinkSettingsPanel.tsx`, `network-link-settings.ts`, and `use-network-link-settings.ts`. The saved link
//! settings and the status come from the host; a password never appears in an error or in the status.

use gpui::{div, prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant, CheckboxField};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::ui::{action_row, muted, stack};
use super::view::SettingsView;

fn byte_len(text: &str) -> usize {
    text.len()
}

fn control(text: &str) -> bool {
    text.chars()
        .any(|c| (c as u32) <= 31 || (127..=159).contains(&(c as u32)))
}

fn validate_peer(peer: &Value) -> Option<&'static str> {
    let text = |k: &str| peer[k].as_str().unwrap_or("");
    if text("title").trim().is_empty() || byte_len(text("title")) > 128 {
        return Some("networkLink.validation.titleRequired");
    }
    if text("host").trim().is_empty() {
        return Some("networkLink.validation.hostRequired");
    }
    if !peer["port"]
        .as_u64()
        .is_some_and(|p| (1024..=65535).contains(&p))
    {
        return Some("networkLink.validation.portInvalid");
    }
    if text("password").is_empty() {
        return Some("networkLink.validation.passwordRequired");
    }
    if byte_len(text("password")) > 1024 {
        return Some("networkLink.validation.passwordTooLong");
    }
    None
}

/// `validateNetworkLinkSettings`: a translation key.
pub fn validate(settings: &Value) -> Option<&'static str> {
    let group = settings["group"].as_str().unwrap_or("");
    if group.chars().count() > 128 || group.trim() != group || control(group) {
        return Some("networkLink.validation.groupInvalid");
    }
    if settings["enabled"] == json!(true) && group.trim().is_empty() {
        return Some("networkLink.validation.groupRequired");
    }
    let peers = settings["peers"].as_array().cloned().unwrap_or_default();
    if peers.len() > 32 {
        return Some("networkLink.validation.peerLimit");
    }
    peers.iter().find_map(validate_peer)
}

/// `networkLinkErrorMessage` and `redactNetworkLinkStatus`: no peer password survives in shown text.
pub fn redact(text: &str, settings: &Value) -> String {
    let mut text = text.to_string();
    for peer in settings["peers"].as_array().into_iter().flatten() {
        if let Some(secret) = peer["password"].as_str().filter(|s| !s.is_empty()) {
            text = text.replace(secret, "••••");
        }
    }
    text
}

pub fn redact_status(status: &Value, settings: &Value) -> Value {
    let mut status = status.clone();
    for peer_status in status["peers"].as_array_mut().into_iter().flatten() {
        if let Some(error) = peer_status["error"].as_str() {
            peer_status["error"] = json!(redact(error, settings));
        }
    }
    status
}

#[derive(Default)]
pub struct NetworkLink {
    pub settings: Option<Value>,
    pub status: Option<Value>,
    pub error: Option<String>,
    pub status_error: Option<String>,
    pub loading: bool,
    pub saving: bool,
    pub requested: bool,
}

impl SettingsView {
    pub(super) fn load_network_link(&mut self, cx: &mut Context<Self>) {
        if self.link.requested {
            return;
        }
        self.link.requested = true;
        self.link.loading = true;
        self.host_call(
            "getNetworkLinkSettings",
            Vec::new(),
            cx,
            |view, result, cx| {
                view.link.loading = false;
                match result {
                    Ok(saved) => view.link.settings = Some(saved),
                    Err(error) => view.link.error = Some(error),
                }
                cx.notify();
            },
        );
    }

    pub(super) fn link_edit(&mut self, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        if let Some(settings) = self.link.settings.as_mut() {
            change(settings);
            cx.notify();
        }
    }

    pub(super) fn link_save(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = self.link.settings.clone() else {
            return;
        };
        if let Some(key) = validate(&settings) {
            self.link.error = Some(cx.global::<Translator>().t(key));
            cx.notify();
            return;
        }
        self.link.saving = true;
        self.link.error = None;
        let shown = settings.clone();
        self.host_call(
            "saveNetworkLinkSettings",
            vec![settings],
            cx,
            move |view, result, cx| {
                view.link.saving = false;
                match result {
                    Ok(saved) => view.link.settings = Some(saved),
                    Err(error) => {
                        let t = *cx.global::<Translator>();
                        view.link.error = Some(format!(
                            "{} {}",
                            t.t("networkLink.saveFailed"),
                            redact(&error, &shown)
                        ));
                    }
                }
                cx.notify();
            },
        );
    }

    pub(super) fn link_refresh(&mut self, cx: &mut Context<Self>) {
        let settings = self.link.settings.clone().unwrap_or(Value::Null);
        self.link.status_error = None;
        self.host_call(
            "getNetworkLinkStatus",
            Vec::new(),
            cx,
            move |view, result, cx| {
                match result {
                    Ok(status) => view.link.status = Some(redact_status(&status, &settings)),
                    Err(error) => {
                        let t = *cx.global::<Translator>();
                        view.link.status_error = Some(format!(
                            "{} {}",
                            t.t("networkLink.statusFailed"),
                            redact(&error, &settings)
                        ));
                    }
                }
                cx.notify();
            },
        );
    }

    pub(super) fn network_link_section(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.load_network_link(cx);
        let t = *cx.global::<Translator>();
        let warning = cx.theme().colors.warning;
        let mut body = stack(cx)
            .child(muted(t.t("networkLink.description"), cx))
            .child(muted(t.t("networkLink.groupHelp"), cx))
            .child(muted(t.t("networkLink.cueHelp"), cx));
        if self.link.loading {
            body = body.child(muted(t.t("networkLink.loading"), cx));
        }
        body = body.children(
            self.link
                .error
                .clone()
                .map(|e| div().id("link-error").text_color(warning).child(e)),
        );
        let Some(settings) = self.link.settings.clone() else {
            return body.into_any_element();
        };
        let weak = cx.weak_entity();
        body = body.child(
            CheckboxField::new(
                "link-enabled",
                t.t("networkLink.enabled"),
                settings["enabled"] == json!(true),
            )
            .on_change({
                let weak = weak.clone();
                move |on, _, cx| {
                    weak.update(cx, |v, cx| v.link_edit(|s| s["enabled"] = json!(on), cx))
                        .ok();
                }
            }),
        );
        body = body.child(self.text_field(
            "link-group",
            t.t("networkLink.group"),
            settings["group"].as_str().unwrap_or(""),
            window,
            cx,
            |v, g, cx| {
                if g.chars().count() <= 128 {
                    v.link_edit(|s| s["group"] = json!(g), cx)
                }
            },
        ));
        let peers = settings["peers"].as_array().cloned().unwrap_or_default();
        for (index, peer) in peers.iter().enumerate() {
            let id = peer["id"].as_str().unwrap_or("").to_string();
            let title = peer["title"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    t.format(
                        "networkLink.peerNumber",
                        &[("number", &(index + 1).to_string())],
                    )
                });
            body = body.child(div().child(title));
            for (key, label) in [
                ("title", "networkLink.peerTitle"),
                ("host", "networkLink.peerHost"),
            ] {
                let peer_id = id.clone();
                body = body.child(self.text_field(
                    &format!("link-{id}-{key}"),
                    t.t(label),
                    peer[key].as_str().unwrap_or(""),
                    window,
                    cx,
                    move |v, text, cx| v.link_edit(|s| set_peer(s, &peer_id, key, json!(text)), cx),
                ));
            }
            let peer_id = id.clone();
            body = body.child(self.number_field(
                &format!("link-{id}-port"),
                t.t("networkLink.peerPort"),
                &peer["port"].to_string(),
                (1024., 65535.),
                1.,
                window,
                cx,
                move |v, text, cx| {
                    if let Ok(port) = text.trim().parse::<u64>() {
                        v.link_edit(|s| set_peer(s, &peer_id, "port", json!(port)), cx)
                    }
                },
            ));
            let peer_id = id.clone();
            body = body.child(self.password_field(
                &format!("link-{id}-password"),
                t.t("networkLink.peerPassword"),
                peer["password"].as_str().unwrap_or(""),
                window,
                cx,
                move |v, text, cx| {
                    v.link_edit(|s| set_peer(s, &peer_id, "password", json!(text)), cx)
                },
            ));
            let remove = id.clone();
            body = body.child(
                ActionButton::new(
                    SharedString::from(format!("link-remove-{id}")),
                    t.t("networkLink.removePeer"),
                )
                .variant(ButtonVariant::Danger)
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.link_edit(
                        |s| {
                            if let Some(list) = s["peers"].as_array_mut() {
                                list.retain(|p| p["id"] != remove.as_str());
                            }
                        },
                        cx,
                    )
                })),
            );
        }
        body = body.child(
            action_row(cx)
                .child(ActionButton::new("link-add-peer", t.t("networkLink.addPeer")).variant(ButtonVariant::Secondary).disabled(peers.len() >= 32).on_click(cx.listener(|v, _, _, cx| {
                    let id = uuid::Uuid::new_v4().to_string();
                    v.link_edit(|s| {
                        if let Some(list) = s["peers"].as_array_mut() {
                            list.push(json!({"id": id, "title": "", "host": "", "port": 49321, "password": ""}));
                        }
                    }, cx)
                })))
                .child(ActionButton::new("link-save", t.t(if self.link.saving { "networkLink.saving" } else { "networkLink.save" })).disabled(self.link.saving || self.link.loading).on_click(cx.listener(|v, _, _, cx| v.link_save(cx))))
                .child(ActionButton::new("link-refresh", t.t("networkLink.refresh")).variant(ButtonVariant::Secondary).on_click(cx.listener(|v, _, _, cx| v.link_refresh(cx)))),
        );
        body = body.children(
            self.link
                .status_error
                .clone()
                .map(|e| div().id("link-status-error").text_color(warning).child(e)),
        );
        if let Some(status) = &self.link.status {
            for peer in &peers {
                let entry = status["peers"]
                    .as_array()
                    .and_then(|l| l.iter().find(|e| e["id"] == peer["id"]))
                    .cloned()
                    .unwrap_or(Value::Null);
                let name = peer["title"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or(peer["host"].as_str())
                    .unwrap_or("");
                let state = entry["state"].as_str().unwrap_or("idle");
                let mut line = format!("{name}  {}", t.t(&format!("networkLink.state.{state}")));
                if let Some(sequence) = entry["sequence"].as_u64() {
                    line.push_str(&format!(
                        "  {}",
                        t.format(
                            "networkLink.sequence",
                            &[("sequence", &sequence.to_string())]
                        )
                    ));
                }
                if let Some(error) = entry["error"].as_str() {
                    line.push_str(&format!(
                        "  {}",
                        t.format("networkLink.deliveryError", &[("error", error)])
                    ));
                }
                body = body.child(div().child(line));
            }
        }
        let membership = self.membership_section(window, cx);
        body.child(membership).into_any_element()
    }
}

fn set_peer(settings: &mut Value, id: &str, key: &str, value: Value) {
    if let Some(peer) = settings["peers"]
        .as_array_mut()
        .and_then(|l| l.iter_mut().find(|p| p["id"] == id))
    {
        peer[key] = value;
    }
}
