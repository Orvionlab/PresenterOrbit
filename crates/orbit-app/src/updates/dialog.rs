//! The startup update dialog (`StartupUpdateDialog.tsx`): shown once when the startup check found an update.

use std::time::Duration;

use gpui::{div, prelude::*, Context, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant, Dialog};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::Value;

use crate::state::runtime::Runtime;

pub struct StartupUpdate {
    runtime: Runtime,
    status: Option<Value>,
    error: Option<String>,
    dismissed: bool,
    installing: bool,
}

impl StartupUpdate {
    pub fn new(runtime: Runtime) -> Self {
        Self {
            runtime,
            status: None,
            error: None,
            dismissed: false,
            installing: false,
        }
    }

    /// Takes the answer of the startup check; only an available update from that check opens the dialog.
    pub fn offer(&mut self, status: Value, cx: &mut Context<Self>) {
        if status["startupCheck"] == true && status["status"] == "available" {
            self.status = Some(status);
            cx.notify();
        }
    }

    fn visible(&self) -> bool {
        !self.dismissed && self.status.is_some()
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if !self.installing {
            self.dismissed = true;
            cx.notify();
        }
    }

    fn install(&mut self, cx: &mut Context<Self>) {
        if self.installing {
            return;
        }
        self.installing = true;
        self.error = None;
        cx.notify();
        let host = self.runtime.host.clone();
        cx.spawn(async move |this, cx| {
            let install = cx.background_spawn({
                let host = host.clone();
                async move { host.invoke("installAppUpdate", Vec::new()) }
            });
            let mut install = std::pin::pin!(futures::FutureExt::fuse(install));
            loop {
                let poll = cx.background_executor().timer(Duration::from_secs(1));
                futures::select_biased! {
                    result = install => {
                        let _ = this.update(cx, |this, cx| {
                            this.installing = false;
                            match result {
                                Ok(status) => {
                                    this.error = status["error"].as_str().map(str::to_string);
                                    this.status = Some(status);
                                }
                                Err(error) => this.error = Some(error),
                            }
                            cx.notify();
                        });
                        break;
                    }
                    _ = futures::FutureExt::fuse(poll) => {
                        let host = host.clone();
                        let status = cx
                            .background_spawn(async move { host.invoke("getAppUpdateStatus", Vec::new()) })
                            .await;
                        if let Ok(status) = status {
                            let _ = this.update(cx, |this, cx| {
                                this.status = Some(status);
                                cx.notify();
                            });
                        }
                    }
                }
            }
        })
        .detach();
    }
}

impl Render for StartupUpdate {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible() {
            return div().into_any_element();
        }
        let t: Translator = Runtime::translator(cx);
        let status = self.status.clone().expect("visible implies a status");
        let state = status["status"].as_str().unwrap_or("available").to_string();
        let gap = cx.theme().metrics.space_xs;
        let weak = cx.weak_entity();
        let mut headline = t.t(&format!("settings.updates.{state}"));
        if let Some(version) = status["version"].as_str() {
            headline.push_str(&format!(" ({version})"));
        }
        let progress = (state == "downloading").then(|| {
            match (
                status["downloaded"].as_f64(),
                status["total"].as_f64().filter(|total| *total > 0.),
            ) {
                (Some(done), Some(total)) => format!(
                    "{} {:.0}%",
                    t.t("settings.updates.downloading"),
                    done / total * 100.
                ),
                _ => t.t("settings.updates.downloading"),
            }
        });
        let error = self
            .error
            .clone()
            .or_else(|| status["error"].as_str().map(str::to_string));
        let busy = self.installing;
        Dialog::new(
            "startup-update",
            t.t("settings.updates.available"),
            t.t("settings.updates.installLater"),
            move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| this.dismiss(cx));
            },
        )
        .width(gpui::px(440.))
        .close_on_backdrop(false)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(gap)
                .child(div().id("startup-update-status").child(headline))
                .children(progress)
                .children(error.map(|error| {
                    div()
                        .id("startup-update-error")
                        .text_color(cx.theme().colors.warning)
                        .child(error)
                }))
                .children(
                    status["notes"]
                        .as_str()
                        .map(|notes| div().child(notes.to_string())),
                )
                .child(div().child(t.t("settings.updates.restartHint")))
                .child(
                    div()
                        .flex()
                        .gap(gap)
                        .child(
                            ActionButton::new(
                                "startup-update-install",
                                t.t("settings.updates.installNow"),
                            )
                            .disabled(busy || state != "available")
                            .on_click(cx.listener(|this, _, _, cx| this.install(cx))),
                        )
                        .child(
                            ActionButton::new(
                                "startup-update-later",
                                t.t("settings.updates.installLater"),
                            )
                            .variant(ButtonVariant::Secondary)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
                        ),
                ),
        )
        .into_any_element()
    }
}
