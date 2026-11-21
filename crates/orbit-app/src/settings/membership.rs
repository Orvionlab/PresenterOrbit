//! `NetworkLinkMembershipPanel.tsx` and `use-network-link-membership.ts`: discover peers on the network and join
//! a group. Discovery and joining touch the network, so nothing here runs until the operator presses a button.

use gpui::{div, prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant};
use orbit_ui::Translator;
use serde_json::{json, Value};

use super::ui::{heading, muted, stack};
use super::view::SettingsView;

#[derive(Default)]
pub struct Membership {
    pub peers: Vec<Value>,
    pub discovering: bool,
    pub scanned: bool,
    pub joining: bool,
    pub host: String,
    pub port: u64,
    pub password: String,
    pub group: String,
    pub error: Option<String>,
    pub joined: bool,
}

impl Membership {
    /// `selectPeer`: fills the form from a discovered peer and clears the password.
    pub fn select_peer(&mut self, peer: &Value) {
        self.host = peer["host"].as_str().unwrap_or("").to_string();
        self.port = peer["port"].as_u64().unwrap_or(49321);
        self.group = peer["group"].as_str().unwrap_or("").to_string();
        self.password.clear();
        self.joined = false;
    }

    pub fn can_join(&self) -> bool {
        !self.host.is_empty() && !self.group.trim().is_empty() && !self.password.is_empty()
    }

    /// The error shown after a failed join: the password is removed from it.
    pub fn shown_error(&self, error: &str) -> String {
        if self.password.is_empty() {
            error.to_string()
        } else {
            error.replace(&self.password, "••••")
        }
    }
}

impl SettingsView {
    pub(super) fn membership_discover(&mut self, cx: &mut Context<Self>) {
        self.membership.discovering = true;
        self.membership.error = None;
        self.host_call(
            "discoverNetworkLinkPeers",
            Vec::new(),
            cx,
            |view, result, cx| {
                view.membership.discovering = false;
                match result {
                    Ok(peers) => {
                        view.membership.peers = peers.as_array().cloned().unwrap_or_default();
                        view.membership.scanned = true;
                    }
                    Err(error) => view.membership.error = Some(error),
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    pub(super) fn membership_join(&mut self, cx: &mut Context<Self>) {
        let m = &self.membership;
        if !m.can_join() || m.joining {
            return;
        }
        let request =
            json!({"host": m.host, "port": m.port, "password": m.password, "group": m.group});
        self.membership.joining = true;
        self.membership.error = None;
        self.membership.joined = false;
        self.host_call(
            "joinNetworkLinkGroup",
            vec![request],
            cx,
            |view, result, cx| {
                view.membership.joining = false;
                match result {
                    Ok(settings) => {
                        view.link.settings = Some(settings);
                        view.membership.password.clear();
                        view.membership.joined = true;
                    }
                    Err(error) => view.membership.error = Some(view.membership.shown_error(&error)),
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    pub(super) fn membership_section(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        if self.membership.port == 0 {
            self.membership.port = 49321;
        }
        let busy = self.membership.joining || self.link.saving;
        let mut body = stack(cx)
            .child(heading(t.t("networkLink.membership.title"), cx))
            .child(muted(t.t("networkLink.membership.help"), cx))
            .child(
                ActionButton::new(
                    "membership-discover",
                    t.t(if self.membership.discovering {
                        "networkLink.membership.scanning"
                    } else {
                        "networkLink.membership.discover"
                    }),
                )
                .variant(ButtonVariant::Secondary)
                .disabled(self.membership.discovering || busy)
                .on_click(cx.listener(|v, _, _, cx| v.membership_discover(cx))),
            );
        for (index, peer) in self.membership.peers.clone().into_iter().enumerate() {
            let label = format!(
                "{} - {}:{} {}",
                peer["title"].as_str().unwrap_or(""),
                peer["host"].as_str().unwrap_or(""),
                peer["port"],
                peer["group"]
                    .as_str()
                    .map(|g| format!("({g})"))
                    .unwrap_or_default()
            );
            body = body.child(
                ActionButton::new(
                    SharedString::from(format!("membership-peer-{index}")),
                    label,
                )
                .variant(ButtonVariant::Secondary)
                .disabled(busy)
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.membership.select_peer(&peer);
                    cx.notify();
                })),
            );
        }
        if self.membership.scanned && self.membership.peers.is_empty() {
            body = body.child(muted(t.t("networkLink.membership.none"), cx));
        }
        body = body.child(self.text_field(
            "membership-host",
            t.t("networkLink.peerHost"),
            &self.membership.host.clone(),
            window,
            cx,
            |v, host, cx| {
                v.membership.host = host;
                cx.notify();
            },
        ));
        body = body.child(self.number_field(
            "membership-port",
            t.t("networkLink.peerPort"),
            &self.membership.port.to_string(),
            (1024., 65535.),
            1.,
            window,
            cx,
            |v, port, cx| {
                if let Ok(port) = port.trim().parse::<u64>() {
                    v.membership.port = port;
                    cx.notify();
                }
            },
        ));
        body = body.child(self.text_field(
            "membership-group",
            t.t("networkLink.group"),
            &self.membership.group.clone(),
            window,
            cx,
            |v, group, cx| {
                v.membership.group = group;
                cx.notify();
            },
        ));
        body = body.child(self.password_field(
            "membership-password",
            t.t("networkLink.peerPassword"),
            &self.membership.password.clone(),
            window,
            cx,
            |v, password, cx| {
                v.membership.password = password;
                cx.notify();
            },
        ));
        body = body.child(
            ActionButton::new(
                "membership-join",
                t.t(if self.membership.joining {
                    "networkLink.membership.joining"
                } else {
                    "networkLink.membership.join"
                }),
            )
            .disabled(!self.membership.can_join() || busy)
            .on_click(cx.listener(|v, _, _, cx| v.membership_join(cx))),
        );
        body = body.children(self.membership.error.clone().map(|e| {
            div()
                .id("membership-error")
                .child(t.format("networkLink.membership.failed", &[("error", &e)]))
        }));
        body.children(
            self.membership
                .joined
                .then(|| div().child(t.t("networkLink.membership.joined"))),
        )
        .into_any_element()
    }
}
