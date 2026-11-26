//! `GeneralSettingsPanel.tsx` and the general half of `settings-actions.ts`.

use super::ui::{Field, OutputField};
use gpui::{div, prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{
    ActionButton, BusyDialog, ButtonVariant, CheckboxField, Select, SelectOption, TextInput,
    ToggleSwitch,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::persist::{self, general_payload};
use super::ui::{action_row, heading, muted, stack};
use super::view::SettingsView;

#[derive(Default)]
pub struct General {
    pub diagnostics: Option<SharedString>,
    pub desktop_error: Option<SharedString>,
    pub saving_desktop: bool,
}

const DESKTOP_SETTINGS: [&str; 3] = ["launchAtLogin", "startMinimized", "minimizeToTray"];

impl SettingsView {
    /// `updateGeneralSettings`: one save with the patch applied to the saved general settings.
    pub(super) fn patch_general(&mut self, patch: Value, cx: &mut Context<Self>) {
        let settings = self.settings(cx);
        self.save(general_payload(&settings, &patch), cx);
    }

    /// A host call that may open an operating system dialog or touch devices; it never runs on the UI thread.
    pub(super) fn host_call(
        &mut self,
        method: &'static str,
        args: Vec<Value>,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Result<Value, String>, &mut Context<Self>) + 'static,
    ) {
        #[cfg(test)]
        self.runtime
            .sent
            .borrow_mut()
            .push((method, Value::Array(args.clone())));
        let host = self.runtime.host.clone();
        cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move { host.invoke(method, args) })
                .await;
            view.update(cx, |view, cx| then(view, result, cx)).ok();
        })
        .detach();
    }

    pub(super) fn set_desktop(
        &mut self,
        name: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.general.saving_desktop {
            return;
        }
        let settings = self.settings(cx);
        let mut general = settings["generalSettings"].clone();
        general[name] = json!(enabled);
        self.general.saving_desktop = true;
        self.general.desktop_error = None;
        // Only the general section is sent: the host applies the login item and the tray from it.
        let task = persist::save(&self.runtime, cx, json!({ "generalSettings": general }));
        cx.spawn(async move |view, cx| {
            let result = task.await;
            view.update(cx, |view, cx| {
                view.general.saving_desktop = false;
                if let Err(error) = result {
                    let t = *cx.global::<Translator>();
                    view.general.desktop_error = Some(
                        format!("{} {error}", t.t("settings.general.desktopSettingFailed")).into(),
                    );
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn choose_logo(&mut self, cx: &mut Context<Self>) {
        self.host_call(
            "selectLogoImage",
            Vec::new(),
            cx,
            |view, result, cx| match result {
                Ok(result) if result["canceled"] == json!(true) => {}
                Ok(result) => view.patch_general(
                    json!({
                        "logoImagePath": result["filePath"].as_str().unwrap_or(""),
                        "logoImageUrl": result["fileUrl"].as_str().unwrap_or(""),
                    }),
                    cx,
                ),
                Err(error) => view.fail(error, cx),
            },
        );
    }

    fn export_diagnostics(&mut self, cx: &mut Context<Self>) {
        let t = *cx.global::<Translator>();
        self.general.diagnostics = Some(t.t("settings.general.diagnosticsExporting").into());
        cx.notify();
        self.host_call("exportDiagnostics", Vec::new(), cx, |view, result, cx| {
            let t = *cx.global::<Translator>();
            let exported = matches!(&result, Ok(result) if result["canceled"] != json!(true));
            view.general.diagnostics = Some(
                t.t(if exported {
                    "settings.general.diagnosticsExported"
                } else {
                    "settings.general.diagnosticsFailed"
                })
                .into(),
            );
            cx.notify();
        });
    }

    /// `WorkspaceSettingsList.tsx`: the active workspace, a choice among the registered ones, and the way to the
    /// Manage Workspaces window. Switching shows the popup until the new workspace is loaded.
    fn workspace_select(&self, t: Translator, cx: &mut Context<Self>) -> AnyElement {
        // Workspace paths are never empty, so the empty value cannot collide with a workspace.
        const MANAGE: &str = "";
        let Some(workspaces) = self.workspaces.clone() else {
            return div().into_any_element();
        };
        let state = workspaces.read(cx);
        let mut options: Vec<SelectOption<String>> = state
            .items
            .iter()
            .map(|item| {
                SelectOption::new(
                    item.path.clone(),
                    if item.missing {
                        format!("{} ({})", item.name, t.t("workspace.missing"))
                    } else {
                        item.name.clone()
                    },
                )
                .disabled(item.missing)
            })
            .collect();
        options.push(SelectOption::new(
            MANAGE.to_string(),
            t.t("workspace.manageMenu"),
        ));
        let active = state
            .active()
            .map_or(MANAGE.to_string(), |item| item.path.clone());
        let switching = state.switching.clone();
        stack(cx)
            .child(Field::new(
                t.t("workspace.activeLabel"),
                Select::new("general-workspace", options, active).on_change(move |path, _, cx| {
                    if path.is_empty() {
                        workspaces.update(cx, |workspaces, cx| workspaces.manage(cx));
                    } else {
                        workspaces.update(cx, |workspaces, cx| workspaces.switch(&path, cx));
                    }
                }),
            ))
            .children(switching.map(|name| {
                BusyDialog::new(
                    "general-workspace-switching",
                    t.format("workspace.switching", &[("name", &name)]),
                )
            }))
            .into_any_element()
    }

    fn copyright_settings(
        &mut self,
        general: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use super::rows;

        let t = *cx.global::<Translator>();
        let license_label = t.t("settings.copyright.license");
        let license = self.field_state(
            "general-ccli-license",
            general["ccliLicenseNumber"].as_str().unwrap_or_default(),
            window,
            cx,
            std::rc::Rc::new(|view, value, cx| {
                view.patch_general(json!({ "ccliLicenseNumber": value.trim() }), cx)
            }),
            false,
        );
        let options = [
            ("first", "settings.copyright.first"),
            ("last", "settings.copyright.last"),
            ("first-and-last", "settings.copyright.firstAndLast"),
            ("all", "settings.copyright.all"),
        ]
        .into_iter()
        .map(|(value, label)| SelectOption::new(value.to_owned(), t.t(label)))
        .collect();
        let placement = Select::new(
            "general-ccli-placement",
            options,
            general["ccliDisplayLocation"]
                .as_str()
                .unwrap_or("first")
                .to_owned(),
        )
        .label(t.t("settings.copyright.placement"))
        .on_change({
            let view = cx.weak_entity();
            move |value, _, cx| {
                let _ = view.update(cx, |view, cx| {
                    view.patch_general(json!({ "ccliDisplayLocation": value }), cx)
                });
            }
        });
        let toggle = ToggleSwitch::new(
            "general-ccli-enabled",
            t.t("settings.copyright.enabled"),
            general["ccliDisplayEnabled"] != false,
        )
        .bare()
        .on_change({
            let view = cx.weak_entity();
            move |enabled, _, cx| {
                let _ = view.update(cx, |view, cx| {
                    view.patch_general(json!({ "ccliDisplayEnabled": enabled }), cx)
                });
            }
        });
        let edit = ActionButton::new("general-ccli-layout", t.t("settings.copyright.editLayout"))
            .variant(ButtonVariant::Secondary)
            .compact()
            .on_click(|_, _, cx| {
                let Some(main) = cx.try_global::<crate::automation::inputs::Commands>() else {
                    return;
                };
                let commands = main.commands.clone();
                let _ = main.window.update(cx, |_, window, cx| {
                    if commands.run("open-ccli", false, window, cx) {
                        window.activate_window();
                    }
                });
            });
        stack(cx)
            .child(heading(t.t("settings.copyright.title"), cx))
            .child(rows::group(
                [
                    rows::setting_row(
                        t.t("settings.copyright.enabled"),
                        None,
                        div()
                            .debug_selector(|| "general-ccli-enabled".into())
                            .child(toggle),
                        cx,
                    ),
                    rows::setting_field_row(
                        "general-ccli-placement-field",
                        t.t("settings.copyright.placement"),
                        placement,
                        cx,
                    ),
                    rows::setting_field_row(
                        "general-ccli-license",
                        license_label.clone(),
                        TextInput::new("ccli-license-input", &license).label(license_label),
                        cx,
                    ),
                    rows::setting_row(t.t("settings.copyright.layout"), None, edit, cx),
                ],
                cx,
            ))
    }

    pub(super) fn general_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let settings = self.settings(cx);
        let general = &settings["generalSettings"];
        let text = |key: &str| general[key].as_str().unwrap_or("").to_string();
        let flag = |key: &str| general[key] == json!(true);
        let weak = cx.weak_entity();
        let (language_weak, playlist_weak) = (weak.clone(), weak.clone());
        let logo = text("logoImagePath");
        let mut panel = stack(cx)
            .id("settings-general")
            .child(heading(t.t("settings.general.title"), cx))
            .child(heading(t.t("settings.general.workspace"), cx))
            .child(self.workspace_select(t, cx))
            .children(
                self.workspaces
                    .as_ref()
                    .and_then(|workspaces| workspaces.read(cx).error.clone())
                    .map(|error| muted(error, cx)),
            )
            .child(
                div()
                    .debug_selector(|| "general-continuous-playlist".to_string())
                    .child(
                        CheckboxField::new(
                            "general-continuous-playlist",
                            t.t("settings.general.continuousPlaylist"),
                            flag("useContinuousPlaylist"),
                        )
                        .label_style(cx.theme().metrics.font_sm, gpui::FontWeight::SEMIBOLD)
                        .on_change(move |checked, _, cx| {
                            playlist_weak
                                .update(cx, |view, cx| {
                                    view.patch_general(
                                        json!({ "useContinuousPlaylist": checked }),
                                        cx,
                                    )
                                })
                                .ok();
                        }),
                    ),
            )
            .child(Field::new(
                t.t("settings.general.language"),
                Select::new(
                    "general-language",
                    vec![
                        SelectOption::new("system".to_string(), t.t("settings.language.system")),
                        SelectOption::new("en".to_string(), t.t("settings.language.english")),
                        SelectOption::new("nl".to_string(), t.t("settings.language.dutch")),
                    ],
                    text("appLanguage"),
                )
                .on_change(move |value, _, cx| {
                    language_weak
                        .update(cx, |view, cx| {
                            view.patch_general(json!({ "appLanguage": value }), cx)
                        })
                        .ok();
                }),
            ))
            .child(OutputField::new(
                t.t("settings.general.logoImage"),
                if logo.is_empty() {
                    t.t("settings.general.noLogoSelected")
                } else {
                    logo
                },
            ))
            .child(
                action_row(cx)
                    .child(
                        ActionButton::new(
                            "general-select-logo",
                            t.t("settings.general.selectLogoImage"),
                        )
                        .icon("polaroid")
                        .on_click(cx.listener(|view, _, _, cx| view.choose_logo(cx))),
                    )
                    .child(
                        ActionButton::new("general-clear-logo", t.t("settings.general.clear"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.patch_general(
                                    json!({"logoImagePath": "", "logoImageUrl": ""}),
                                    cx,
                                )
                            })),
                    ),
            )
            .child(
                action_row(cx)
                    .child(
                        ActionButton::new(
                            "general-export-diagnostics",
                            t.t("settings.general.exportDiagnostics"),
                        )
                        .icon("settings")
                        .on_click(cx.listener(|view, _, _, cx| view.export_diagnostics(cx))),
                    )
                    .children(
                        self.general
                            .diagnostics
                            .clone()
                            .map(|status| muted(status, cx)),
                    ),
            )
            .child(self.copyright_settings(general, window, cx))
            .child(heading(t.t("settings.general.system"), cx));
        for name in DESKTOP_SETTINGS {
            let weak = weak.clone();
            panel = panel.child(
                ToggleSwitch::new(
                    SharedString::from(format!("general-{name}")),
                    t.t(&format!("settings.general.{name}")),
                    flag(name),
                )
                .label_size(cx.theme().metrics.font_lg)
                .disabled(self.general.saving_desktop)
                .on_change(move |enabled, _, cx| {
                    weak.update(cx, |view, cx| view.set_desktop(name, enabled, cx))
                        .ok();
                }),
            );
        }
        let warning = cx.theme().colors.warning;
        panel
            .children(self.general.desktop_error.clone().map(|error| {
                gpui::div()
                    .id("general-desktop-error")
                    .text_color(warning)
                    .child(error)
            }))
            .into_any_element()
    }
}
