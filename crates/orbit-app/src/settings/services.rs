//! Updates, ProPresenter import and export, and media cleanup.

use std::time::Duration;

use gpui::{div, prelude::*, AnyElement, Context, Entity, SharedString, Window};
use orbit_ui::controls::{
    ActionButton, SegmentedControl, SegmentedOption, SelectOption, ToggleSwitch,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::rows::{column, group, select_row};
use super::ui::OutputField;
use super::ui::{action_row, heading, muted, stack, title};
use super::view::SettingsView;
use crate::collections::model::Snapshot;

pub const MEDIA_IMPORT_SCALES: [&str; 4] = ["fit", "fill", "stretch", "blur"];
pub const MEDIA_IMPORT_BEHAVIORS: [&str; 2] = ["foreground", "background"];
pub const MEDIA_IMPORT_PLAYBACKS: [&str; 2] = ["stop", "loop"];
pub const MEDIA_IMPORT_VIDEO_ENDS: [&str; 5] =
    ["hold", "black", "clear", "fade-black", "fade-clear"];

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImportSection {
    #[default]
    Import,
    Settings,
}

#[derive(Default)]
pub struct Services {
    pub import_section: ImportSection,
    pub update: Option<Value>,
    pub update_error: Option<String>,
    pub update_busy: bool,
    #[cfg_attr(test, allow(dead_code))]
    polling: bool,
    pub import_status: Option<String>,
    pub pro_import: Option<super::pro_presenter_import::ProImport>,
    file_import: Option<Entity<crate::imports::Imports>>,
    pub review: Option<Value>,
    pub cleanup_busy: bool,
    pub cleanup_error: Option<String>,
    pub recovery: Option<String>,
    cleanup_request: u64,
}

/// `busy` of `useAppUpdates`: a running operation, or an update in a working state.
pub fn update_is_busy(update: Option<&Value>, pending: bool) -> bool {
    pending
        || matches!(
            update.and_then(|update| update["status"].as_str()),
            Some("checking" | "downloading" | "installing")
        )
}

impl SettingsView {
    fn import_pro_presenter_file(&mut self, cx: &mut Context<Self>) {
        if self.services.file_import.is_some() {
            return;
        }
        let runtime = self.runtime.clone();
        let owner = cx.weak_entity();
        self.services.file_import = Some(cx.new(|cx| {
            let mut import = crate::imports::Imports::new(
                runtime,
                move |cx| {
                    let _ = owner.update(cx, |view, cx| {
                        view.services.file_import = None;
                        cx.notify();
                    });
                },
                cx,
            );
            import.pick_file(true, cx);
            import
        }));
        cx.notify();
    }

    /// Reads the update status once a second while the panel is shown, as the React hook does, and stops when it
    /// is not.
    #[cfg_attr(test, allow(dead_code))]
    fn poll_updates(&mut self, cx: &mut Context<Self>) {
        if self.services.polling {
            return;
        }
        self.services.polling = true;
        self.refresh_update_status(cx);
        cx.spawn(async move |view, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let keep = view.update(cx, |view, cx| {
                if view.panel != "updates" {
                    view.services.polling = false;
                    return false;
                }
                view.refresh_update_status(cx);
                true
            });
            if !matches!(keep, Ok(true)) {
                break;
            }
        })
        .detach();
    }

    pub(super) fn refresh_update_status(&mut self, cx: &mut Context<Self>) {
        if self.services.update_busy {
            return;
        }
        let host = self.runtime.host.clone();
        cx.spawn(async move |view, cx| {
            let status = cx
                .background_spawn(async move { host.invoke("getAppUpdateStatus", Vec::new()) })
                .await;
            view.update(cx, |view, cx| {
                // An operation that started meanwhile owns the status.
                if !view.services.update_busy {
                    match status {
                        Ok(status) => view.services.update = Some(status),
                        Err(error) => view.services.update_error = Some(error),
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn run_update(&mut self, install: bool, cx: &mut Context<Self>) {
        if self.services.update_busy {
            return;
        }
        self.services.update_busy = true;
        self.services.update_error = None;
        cx.notify();
        let method = if install {
            "installAppUpdate"
        } else {
            "checkAppUpdate"
        };
        self.host_call(method, Vec::new(), cx, |view, result, cx| {
            view.services.update_busy = false;
            match result {
                Ok(status) => view.services.update = Some(status),
                Err(error) => view.services.update_error = Some(error),
            }
            cx.notify();
        });
    }

    fn updates_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        #[cfg(not(test))]
        self.poll_updates(cx);
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let settings = self.settings(cx);
        let update = self.services.update.clone();
        let status = update.as_ref().and_then(|update| update["status"].as_str());
        let busy = update_is_busy(update.as_ref(), self.services.update_busy);
        let weak = cx.weak_entity();
        let version = update
            .as_ref()
            .and_then(|update| update["version"].as_str());
        let headline = match status {
            Some(status) => format!(
                "{}{}",
                t.t(&format!("settings.updates.{status}")),
                version
                    .map(|version| format!(" ({version})"))
                    .unwrap_or_default()
            ),
            None => t.t("settings.updates.loading"),
        };
        let error = self.services.update_error.clone().or_else(|| {
            update
                .as_ref()
                .and_then(|update| update["error"].as_str().map(str::to_string))
        });
        let mut panel = stack(cx)
            .id("settings-updates")
            .child(title(t.t("settings.updates.title"), cx));
        if let Some(update) = &update {
            panel = panel.child(
                div().w(gpui::px(220.)).child(OutputField::new(
                    t.t("settings.updates.version"),
                    update["currentVersion"]
                        .as_str()
                        .map_or_else(|| self.runtime.host.version().to_string(), str::to_string),
                )),
            );
        }
        panel = panel.child(muted(headline, cx).id("updates-status"));
        if status == Some("downloading") {
            let update = update.as_ref().expect("downloading implies a status");
            let (downloaded, total) = (update["downloaded"].as_f64(), update["total"].as_f64());
            panel = panel.child(muted(
                match total.filter(|total| *total > 0.) {
                    Some(total) => format!(
                        "{} {:.0}%",
                        t.t("settings.updates.downloading"),
                        downloaded.unwrap_or(0.) / total * 100.
                    ),
                    None => t.t("settings.updates.downloading"),
                },
                cx,
            ));
        }
        panel = panel
            .children(error.map(|error| {
                div()
                    .id("updates-error")
                    .text_color(theme.colors.warning)
                    .child(error)
            }))
            .children(
                update
                    .as_ref()
                    .and_then(|update| update["notes"].as_str())
                    .map(|notes| muted(notes.to_string(), cx)),
            )
            .child(
                action_row(cx)
                    .child(
                        ActionButton::new("updates-check", t.t("settings.updates.check"))
                            .disabled(update.is_none() || busy || status == Some("unconfigured"))
                            .on_click(cx.listener(|view, _, _, cx| view.run_update(false, cx))),
                    )
                    .children((status == Some("available")).then(|| {
                        ActionButton::new("updates-install", t.t("settings.updates.install"))
                            .disabled(busy)
                            .on_click(cx.listener(|view, _, _, cx| view.run_update(true, cx)))
                    })),
            )
            .child(
                action_row(cx)
                    .child(
                        ToggleSwitch::new(
                            "updates-check-on-startup",
                            t.t("settings.updates.checkOnStartup"),
                            settings["generalSettings"]["checkForUpdatesOnStartup"] == json!(true),
                        )
                        .bare()
                        .on_change(move |checked, _, cx| {
                            weak.update(cx, |view, cx| {
                                view.patch_general(json!({"checkForUpdatesOnStartup": checked}), cx)
                            })
                            .ok();
                        }),
                    )
                    .child(
                        div()
                            .text_size(theme.metrics.font_lg)
                            .child(t.t("settings.updates.checkOnStartup")),
                    ),
            )
            .child(muted(t.t("settings.updates.restartHint"), cx));
        panel.into_any_element()
    }

    pub(super) fn import_pro_presenter(&mut self, cx: &mut Context<Self>) {
        let t = *cx.global::<Translator>();
        if self.runtime.content.read(cx).workspace_path.is_empty() {
            self.services.import_status = Some(t.t("status.import.chooseWorkspace"));
            cx.notify();
            return;
        }
        self.services.import_status = Some(t.t("status.import.scanning"));
        cx.notify();
        self.host_call(
            "scanProPresenterWorkspace",
            Vec::new(),
            cx,
            |view, result, cx| {
                let t = *cx.global::<Translator>();
                match result {
                    Ok(result) if result["canceled"] == json!(true) => {
                        view.services.import_status = Some(t.t("status.import.canceled"));
                    }
                    Ok(result) => {
                        let Some(path) = result["path"].as_str() else {
                            return;
                        };
                        let (libraries, categories) =
                            super::pro_presenter_import::choices_from_inventory(
                                &result["inventory"],
                            );
                        view.services.import_status = None;
                        view.services.pro_import =
                            Some(super::pro_presenter_import::ProImport::Select {
                                path: path.to_string(),
                                libraries,
                                categories,
                            });
                    }
                    Err(error) => {
                        view.services.import_status =
                            Some(t.format("status.import.failed", &[("error", &error)]));
                    }
                }
                cx.notify();
            },
        );
    }

    pub(super) fn run_pro_import(&mut self, cx: &mut Context<Self>) {
        use super::pro_presenter_import::{selection, ProImport};
        let Some(ProImport::Select {
            path,
            libraries,
            categories,
        }) = self.services.pro_import.as_ref()
        else {
            return;
        };
        let args = vec![json!(path), selection(libraries, categories)];
        self.services.pro_import = Some(ProImport::Importing);
        cx.notify();
        self.host_call(
            "selectProPresenterWorkspace",
            args,
            cx,
            |view, result, cx| {
                let t = *cx.global::<Translator>();
                match result {
                    Ok(result) => {
                        view.services.pro_import = Some(ProImport::Done(
                            super::pro_presenter_import::report_from(&result["workspace"]),
                        ));
                        cx.notify();
                        view.runtime.lanes.revisions().remember_selected(&result);
                        let Some(path) = result["workspace"]["workspacePath"]
                            .as_str()
                            .map(str::to_string)
                        else {
                            return;
                        };
                        let host = view.runtime.host.clone();
                        let revisions = view.runtime.lanes.revisions().clone();
                        let runtime = view.runtime.clone();
                        cx.spawn(async move |view, cx| {
                            let loaded = cx
                                .background_spawn(async move {
                                    let loaded = host.invoke("loadWorkspace", vec![json!(path)])?;
                                    revisions.remember(&path, &loaded);
                                    Snapshot::parse(&loaded["workspace"])
                                })
                                .await;
                            view.update(cx, |view, cx| {
                                let t = *cx.global::<Translator>();
                                match loaded {
                                    Ok(snapshot) => {
                                        runtime.content.update(cx, |content, cx| {
                                            content.load(snapshot);
                                            cx.notify();
                                        });
                                        runtime
                                            .edit_ui(cx, |content, ui| ui.select_loaded(content));
                                        view.services.import_status = None;
                                    }
                                    Err(error) => {
                                        view.services.import_status = Some(
                                            t.format("status.import.failed", &[("error", &error)]),
                                        )
                                    }
                                }
                                cx.notify();
                            })
                            .ok();
                        })
                        .detach();
                    }
                    Err(error) => {
                        view.services.pro_import = None;
                        view.services.import_status =
                            Some(t.format("status.import.failed", &[("error", &error)]));
                        cx.notify();
                    }
                }
            },
        );
    }

    pub(super) fn export_pro_presenter(&mut self, cx: &mut Context<Self>) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        if path.is_empty() {
            return;
        }
        self.host_call(
            "exportProPresenterWorkspace",
            vec![json!(path)],
            cx,
            |view, result, cx| {
                if let Err(error) = result {
                    view.fail(error, cx);
                }
            },
        );
    }

    fn import_panel(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let has_workspace = !self.runtime.content.read(cx).workspace_path.is_empty();
        let status = self
            .services
            .import_status
            .clone()
            .unwrap_or_else(|| t.t("settings.import.instructions"));
        let general = self.settings(cx)["generalSettings"].clone();
        let row = |key: &'static str,
                   label: &str,
                   values: &[&str],
                   option_key: &dyn Fn(&str) -> String,
                   cx: &mut Context<Self>| {
            let weak = cx.weak_entity();
            select_row(
                SharedString::from(format!("import-{key}")),
                label.to_string(),
                values
                    .iter()
                    .map(|value| SelectOption::new(value.to_string(), t.t(&option_key(value))))
                    .collect(),
                general[key].as_str().map(str::to_string),
                move |value, _, cx| {
                    weak.update(cx, |view, cx| view.patch_general(json!({ key: value }), cx))
                        .ok();
                },
                cx,
            )
        };
        let scale = |value: &str| format!("media.scale.{value}");
        let behavior = |value: &str| format!("media.behavior.{value}");
        let playback = |value: &str| format!("media.playback.{value}");
        let end = |value: &str| {
            match value {
                "fade-black" => "media.end.fadeBlack",
                "fade-clear" => "media.end.fadeClear",
                "hold" => "media.end.hold",
                "black" => "media.end.black",
                _ => "media.end.clear",
            }
            .to_string()
        };
        let scaling = group(
            [
                row(
                    "importScaleForeground",
                    &t.t("settings.import.foreground"),
                    &MEDIA_IMPORT_SCALES,
                    &scale,
                    cx,
                ),
                row(
                    "importScaleBackground",
                    &t.t("settings.import.background"),
                    &MEDIA_IMPORT_SCALES,
                    &scale,
                    cx,
                ),
            ],
            cx,
        );
        let image = group(
            [row(
                "importImageBehavior",
                &t.t("media.behavior"),
                &MEDIA_IMPORT_BEHAVIORS,
                &behavior,
                cx,
            )],
            cx,
        );
        let video = group(
            [
                row(
                    "importVideoBehavior",
                    &t.t("media.behavior"),
                    &MEDIA_IMPORT_BEHAVIORS,
                    &behavior,
                    cx,
                ),
                row(
                    "importVideoPlayback",
                    &t.t("media.playbackBehavior"),
                    &MEDIA_IMPORT_PLAYBACKS,
                    &playback,
                    cx,
                ),
                row(
                    "importVideoEndBehavior",
                    &t.t("media.endBehavior"),
                    &MEDIA_IMPORT_VIDEO_ENDS,
                    &end,
                    cx,
                ),
            ],
            cx,
        );
        let audio = group(
            [row(
                "importAudioPlayback",
                &t.t("media.playbackBehavior"),
                &MEDIA_IMPORT_PLAYBACKS,
                &playback,
                cx,
            )],
            cx,
        );
        let weak = cx.weak_entity();
        let sections = SegmentedControl::new(
            "import-sections",
            t.t("settings.import.sections"),
            vec![
                SegmentedOption::text("import", t.t("settings.nav.import")),
                SegmentedOption::text("settings", t.t("settings.title")),
            ],
            match self.services.import_section {
                ImportSection::Import => "import",
                ImportSection::Settings => "settings",
            },
        )
        .on_change(move |value, _, cx| {
            weak.update(cx, |view, cx| {
                view.services.import_section = match value {
                    "settings" => ImportSection::Settings,
                    _ => ImportSection::Import,
                };
                cx.notify();
            })
            .ok();
        });
        let panel = column(cx).id("settings-import").child(
            div()
                .flex()
                .p(cx.theme().metrics.panel_padding)
                .child(sections),
        );
        match self.services.import_section {
            ImportSection::Settings => panel
                .child(heading(t.t("media.scaling"), cx))
                .child(scaling)
                .child(heading(t.t("settings.import.image"), cx))
                .child(image)
                .child(heading(t.t("settings.import.video"), cx))
                .child(video)
                .child(heading(t.t("settings.import.audio"), cx))
                .child(audio),
            ImportSection::Import => panel
                .child(heading(t.t("settings.import.proPresenter"), cx))
                .child(
                    action_row(cx)
                        .child(
                            ActionButton::new(
                                "import-pro-presenter",
                                t.t("settings.import.importProPresenter"),
                            )
                            .icon("library")
                            .on_click(cx.listener(|view, _, _, cx| view.import_pro_presenter(cx))),
                        )
                        .child(
                            ActionButton::new(
                                "import-pro-presenter-file",
                                t.t("settings.import.importProPresenterFiles"),
                            )
                            .variant(orbit_ui::controls::ButtonVariant::Secondary)
                            .disabled(!has_workspace)
                            .on_click(
                                cx.listener(|view, _, _, cx| view.import_pro_presenter_file(cx)),
                            ),
                        )
                        .child(
                            ActionButton::new(
                                "export-pro-presenter",
                                t.t("settings.import.exportProPresenter"),
                            )
                            .icon("send")
                            .disabled(!has_workspace)
                            .on_click(cx.listener(|view, _, _, cx| view.export_pro_presenter(cx))),
                        ),
                )
                .child(muted(status, cx)),
        }
        .children(self.pro_import_dialog(window, cx))
        .children(self.services.file_import.clone())
        .into_any_element()
    }

    /// Reviews unused managed media, then removes the reviewed files. A newer request retires the older one.
    pub(super) fn run_cleanup(&mut self, clean: bool, cx: &mut Context<Self>) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        if path.is_empty() {
            return;
        }
        self.services.cleanup_request += 1;
        let request = self.services.cleanup_request;
        self.services.cleanup_busy = true;
        self.services.cleanup_error = None;
        self.services.recovery = None;
        cx.notify();
        if clean {
            let Some(review) = self.services.review.clone() else {
                self.services.cleanup_busy = false;
                return;
            };
            #[cfg(test)]
            self.runtime
                .sent
                .borrow_mut()
                .push(("cleanUnusedMedia", json!([path, review])));
            let (host, scope) = (self.runtime.host.clone(), path.clone());
            let answer = self.runtime.lanes.workspace(&path, move |_| {
                host.invoke("cleanUnusedMedia", vec![json!(scope), review])
            });
            let runtime = self.runtime.clone();
            cx.spawn(async move |view, cx| {
                let result = answer
                    .await
                    .map_err(|_| "The workspace queue stopped.".to_string())
                    .and_then(|result| result);
                view.update(cx, |view, cx| {
                    if view.services.cleanup_request != request
                        || runtime.content.read(cx).workspace_path != path
                    {
                        return;
                    }
                    view.services.cleanup_busy = false;
                    match result {
                        Ok(done) => {
                            view.services.review = None;
                            view.services.recovery =
                                done["recoveryPath"].as_str().map(str::to_string);
                            if let Ok(snapshot) = Snapshot::parse(&done["workspace"]) {
                                runtime.content.update(cx, |content, cx| {
                                    content.apply_canonical(snapshot);
                                    cx.notify();
                                });
                            }
                        }
                        Err(error) => {
                            view.services.cleanup_error = Some(error);
                            view.services.review = None;
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        } else {
            self.host_call(
                "reviewMediaCleanup",
                vec![json!(path)],
                cx,
                move |view, result, cx| {
                    if view.services.cleanup_request != request {
                        return;
                    }
                    view.services.cleanup_busy = false;
                    match result {
                        Ok(review) => view.services.review = Some(review),
                        Err(error) => {
                            view.services.cleanup_error = Some(error);
                            view.services.review = None;
                        }
                    }
                    cx.notify();
                },
            );
        }
    }

    pub(super) fn service_panel(
        &mut self,
        panel: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        Some(match panel {
            "updates" => self.updates_panel(cx),
            "import" => self.import_panel(window, cx),
            _ => return None,
        })
    }
}
