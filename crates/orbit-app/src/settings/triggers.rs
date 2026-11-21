//! `TriggerBindingsPanelBridge.tsx` and `trigger-binding-controller.ts`. Bindings are one `triggerBindings`
//! workspace section. A shortcut conflicts with another binding or a key mapping; a MIDI note conflicts with another
//! binding's channel and pitch.

use super::ui::Field;
use gpui::{div, prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant, Select, SelectOption};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::groups::normalize_group_settings;
use super::ui::{action_row, heading, muted, stack};
use super::view::SettingsView;
use crate::collections::model::Snapshot;
use crate::keys::PRESENTATION_MENU_ITEMS;

pub const TYPES: [&str; 7] = [
    "cue",
    "group",
    "prop",
    "timer",
    "macro",
    "clear-group",
    "command",
];

/// `bindingTargetOptions`: the ids and titles a binding of this type can target.
pub fn target_options(
    workspace: &Value,
    group_settings: &Value,
    binding_type: &str,
) -> Vec<(String, String)> {
    let slides = || {
        workspace["library"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|item| {
                item["slides"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(move |slide| (item, slide))
            })
    };
    match binding_type {
        "command" => PRESENTATION_MENU_ITEMS
            .iter()
            .flatten()
            .map(|c| (c.to_string(), c.to_string()))
            .collect(),
        "cue" => slides()
            .map(|(item, slide)| {
                let id = slide["sourceCueId"]
                    .as_str()
                    .or_else(|| slide["id"].as_str())
                    .unwrap_or("")
                    .to_string();
                (
                    id,
                    format!(
                        "{} - {}",
                        item["title"].as_str().unwrap_or(""),
                        slide["id"].as_str().unwrap_or("")
                    ),
                )
            })
            .collect(),
        "group" | "clear-group" => {
            let mut options: Vec<(String, String)> = if binding_type == "group" {
                normalize_group_settings(group_settings)
                    .into_iter()
                    .map(|g| (g.id, g.name))
                    .collect()
            } else {
                workspace["clearGroups"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|g| {
                        (
                            g["id"].as_str().unwrap_or("").to_string(),
                            g["title"].as_str().unwrap_or("").to_string(),
                        )
                    })
                    .collect()
            };
            for (_, slide) in slides() {
                if let Some(id) = slide["sourceGroupId"].as_str() {
                    if !options.iter().any(|(known, _)| known == id) {
                        options.push((
                            id.to_string(),
                            slide["groupName"].as_str().unwrap_or(id).to_string(),
                        ));
                    }
                }
            }
            options
        }
        "prop" => workspace["props"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                (
                    p["id"].as_str().unwrap_or("").to_string(),
                    p["title"]
                        .as_str()
                        .or(p["name"].as_str())
                        .unwrap_or("")
                        .to_string(),
                )
            })
            .collect(),
        "timer" => workspace["countdownTimers"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                (
                    t["id"].as_str().unwrap_or("").to_string(),
                    t["name"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect(),
        "macro" => workspace["macros"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| {
                (
                    m["id"].as_str().unwrap_or("").to_string(),
                    m["title"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// `bindingValidationError`: a translation key.
pub fn validation_error(
    workspace: &Value,
    group_settings: &Value,
    key_mappings: &Value,
    binding: &Value,
) -> Option<&'static str> {
    let target = binding["targetId"].as_str().unwrap_or("");
    if !target_options(
        workspace,
        group_settings,
        binding["type"].as_str().unwrap_or(""),
    )
    .iter()
    .any(|(id, _)| id == target)
    {
        return Some("automation.bindings.invalid");
    }
    if binding["type"] == "cue" {
        let usable = workspace["library"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|item| {
                item["slides"].as_array().into_iter().flatten().any(|s| {
                    s["sourceCueId"].as_str().or(s["id"].as_str()) == Some(target)
                        && s["disabled"] != json!(true)
                })
            });
        if !usable {
            return Some("automation.bindings.invalid");
        }
    }
    if binding["shortcut"].as_str().is_none_or(str::is_empty) && binding["midi"].is_null() {
        return Some("automation.bindings.noInput");
    }
    let others: Vec<&Value> = workspace["triggerBindings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["id"] != binding["id"])
        .collect();
    if let Some(shortcut) = binding["shortcut"].as_str().filter(|s| !s.is_empty()) {
        let taken = others.iter().any(|b| b["shortcut"] == shortcut)
            || key_mappings
                .as_object()
                .is_some_and(|m| m.values().any(|v| v == shortcut));
        if taken {
            return Some("automation.bindings.conflict");
        }
    }
    if !binding["midi"].is_null() {
        let (channel, pitch, velocity) = (
            &binding["midi"]["channel"],
            &binding["midi"]["pitch"],
            &binding["midi"]["velocity"],
        );
        let within =
            |v: &Value, min: u64, max: u64| v.as_u64().is_some_and(|n| (min..=max).contains(&n));
        if !within(channel, 1, 16) || !within(pitch, 0, 127) || !within(velocity, 1, 127) {
            return Some("automation.bindings.invalid");
        }
        if others
            .iter()
            .any(|b| b["midi"]["channel"] == *channel && b["midi"]["pitch"] == *pitch)
        {
            return Some("automation.bindings.conflict");
        }
    }
    None
}

#[derive(Default)]
pub struct Triggers {
    pub draft: Option<Value>,
    pub query: String,
    pub conflicts_only: bool,
    pub error: Option<String>,
    pub busy: bool,
    pub midi_status: Option<Value>,
    #[cfg_attr(test, allow(dead_code))]
    pub midi_requested: bool,
}

impl SettingsView {
    fn trigger_context(&self, cx: &gpui::App) -> (Value, Value, Value) {
        let workspace = (*self.runtime.content.read(cx).workspace).clone();
        let settings = self.settings(cx);
        let groups = if workspace["groupSettings"]
            .as_object()
            .is_some_and(|g| !g.is_empty())
        {
            workspace["groupSettings"].clone()
        } else {
            settings["groupSettings"].clone()
        };
        (workspace, groups, settings["keyMappings"].clone())
    }

    pub(super) fn trigger_edit(&mut self, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        if let Some(draft) = self.triggers.draft.as_mut() {
            change(draft);
            cx.notify();
        }
    }

    pub(super) fn triggers_persist(&mut self, list: Vec<Value>, cx: &mut Context<Self>) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        if path.is_empty() {
            self.triggers.error = Some(
                cx.global::<Translator>()
                    .t("automation.macros.workspaceRequired"),
            );
            cx.notify();
            return;
        }
        self.triggers.busy = true;
        self.triggers.error = None;
        let (host, scope) = (self.runtime.host.clone(), path.clone());
        let answer = self.runtime.lanes.workspace(&path, move |revision| {
            let saved = host.invoke(
                "saveWorkspaceContent",
                vec![
                    json!(scope),
                    json!({"triggerBindings": list}),
                    json!(revision),
                ],
            )?;
            let fresh = host.invoke("readWorkspaceSnapshot", vec![json!(scope)])?;
            Ok(json!({"workspace": fresh["workspace"], "revision": saved["revision"]}))
        });
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let result = answer
                .await
                .map_err(|_| "The workspace queue stopped.".to_string())
                .and_then(|r| r);
            let parsed = match result {
                Ok(done) => {
                    cx.background_spawn(async move { Snapshot::parse(&done["workspace"]) })
                        .await
                }
                Err(error) => Err(error),
            };
            view.update(cx, |view, cx| {
                view.triggers.busy = false;
                match parsed {
                    Ok(snapshot) if runtime.content.read(cx).workspace_path == path => {
                        runtime.content.update(cx, |content, cx| {
                            content.apply_canonical(snapshot);
                            cx.notify();
                        });
                        view.triggers.draft = None;
                    }
                    Ok(_) => {}
                    Err(error) => view.triggers.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    #[cfg_attr(test, allow(dead_code))]
    pub(super) fn load_midi_status(&mut self, cx: &mut Context<Self>) {
        if self.triggers.midi_requested {
            return;
        }
        self.triggers.midi_requested = true;
        self.host_call("getMidiInputStatus", Vec::new(), cx, |view, result, cx| {
            match result {
                Ok(status) => view.triggers.midi_status = Some(status),
                Err(error) => view.triggers.error = Some(error),
            }
            cx.notify();
        });
    }

    pub(super) fn trigger_save(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.triggers.draft.clone() else {
            return;
        };
        let (workspace, groups, keys) = self.trigger_context(cx);
        if let Some(error) = validation_error(&workspace, &groups, &keys, &binding) {
            self.triggers.error = Some(cx.global::<Translator>().t(error));
            cx.notify();
            return;
        }
        let mut list = workspace["triggerBindings"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        match list.iter_mut().find(|b| b["id"] == binding["id"]) {
            Some(slot) => *slot = binding,
            None => list.push(binding),
        }
        self.triggers_persist(list, cx);
    }

    pub(super) fn trigger_delete(&mut self, id: &str, cx: &mut Context<Self>) {
        let (workspace, ..) = self.trigger_context(cx);
        let list = workspace["triggerBindings"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|b| b["id"] != id)
            .collect();
        self.triggers_persist(list, cx);
    }

    /// `testTriggerBinding`: runs the binding's action once; an invalid binding is never sent.
    pub(super) fn trigger_test(&mut self, binding: &Value, cx: &mut Context<Self>) {
        let (workspace, groups, keys) = self.trigger_context(cx);
        if let Some(error) = validation_error(&workspace, &groups, &keys, binding) {
            self.triggers.error = Some(cx.global::<Translator>().t(error));
            cx.notify();
            return;
        }
        let path = self.runtime.content.read(cx).workspace_path.clone();
        self.host_call(
            "testTriggerBinding",
            vec![json!(path), binding["id"].clone()],
            cx,
            |view, result, cx| {
                if let Err(error) = result {
                    view.triggers.error = Some(error);
                }
                cx.notify();
            },
        );
    }

    pub(super) fn triggers_section(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        #[cfg(not(test))]
        self.load_midi_status(cx);
        if let Some(status) = cx
            .try_global::<crate::automation::inputs::Inputs>()
            .map(|s| s.midi_status.clone())
            .filter(Value::is_object)
        {
            self.triggers.midi_status = Some(status);
        }
        let (workspace, groups, keys) = self.trigger_context(cx);
        let weak = cx.weak_entity();
        let query = self.triggers.query.to_lowercase();
        let mut panel = stack(cx)
            .id("settings-triggers")
            .child(heading(t.t("automation.bindings.title"), cx))
            .children(self.triggers.midi_status.clone().map(|status| {
                let devices: Vec<&str> = status["devices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|d| d.as_str())
                    .collect();
                let state = if status["enabled"] != json!(true) {
                    t.t("automation.bindings.midiOff")
                } else if devices.is_empty() {
                    t.t("automation.bindings.midiNoDevices")
                } else {
                    devices.join(", ")
                };
                div().id("trigger-midi-status").child(format!(
                    "{}: {state} {}",
                    t.t("automation.bindings.midiInputs"),
                    status["error"].as_str().unwrap_or("")
                ))
            }))
            .child(self.text_field(
                "trigger-filter",
                t.t("automation.bindings.filter"),
                &self.triggers.query.clone(),
                window,
                cx,
                |v, q, cx| {
                    v.triggers.query = q;
                    cx.notify();
                },
            ))
            .child(
                div()
                    .flex()
                    .gap(cx.theme().metrics.control_gap)
                    .child(
                        ActionButton::new(
                            "trigger-conflicts",
                            t.t("automation.bindings.conflictsOnly"),
                        )
                        .variant(if self.triggers.conflicts_only {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.triggers.conflicts_only = !v.triggers.conflicts_only;
                            cx.notify();
                        })),
                    )
                    .child(
                        ActionButton::new("trigger-new", t.t("automation.bindings.new"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.triggers.draft = Some(
                                    json!({"id": uuid::Uuid::new_v4().to_string(), "type": "cue"}),
                                );
                                cx.notify();
                            })),
                    ),
            );
        for binding in workspace["triggerBindings"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let id = binding["id"].as_str().unwrap_or("").to_string();
            let error = validation_error(&workspace, &groups, &keys, &binding);
            let title = target_options(&workspace, &groups, binding["type"].as_str().unwrap_or(""))
                .into_iter()
                .find(|(i, _)| binding["targetId"] == i.as_str())
                .map(|(_, t)| t)
                .unwrap_or_else(|| binding["targetId"].as_str().unwrap_or("").to_string());
            let line = format!(
                "{}: {title}  {}  {}",
                binding["type"].as_str().unwrap_or(""),
                binding["shortcut"].as_str().unwrap_or(""),
                binding["midi"]
                    .as_object()
                    .map(|m| format!("{} / {} / {}", m["channel"], m["pitch"], m["velocity"]))
                    .unwrap_or_default()
            );
            if (!query.is_empty() && !line.to_lowercase().contains(&query))
                || (self.triggers.conflicts_only && error != Some("automation.bindings.conflict"))
            {
                continue;
            }
            let (edit, test, remove) = (binding.clone(), binding.clone(), id.clone());
            panel = panel.child(
                action_row(cx)
                    .child(line)
                    .children(error.map(|e| muted(t.t(e), cx)))
                    .child(
                        ActionButton::new(
                            SharedString::from(format!("trigger-edit-{id}")),
                            t.t("automation.macros.edit"),
                        )
                        .variant(ButtonVariant::Secondary)
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.triggers.draft = Some(edit.clone());
                            cx.notify();
                        })),
                    )
                    .child(
                        ActionButton::new(
                            SharedString::from(format!("trigger-test-{id}")),
                            t.t("automation.bindings.test"),
                        )
                        .variant(ButtonVariant::Secondary)
                        .on_click(cx.listener(move |v, _, _, cx| v.trigger_test(&test, cx))),
                    )
                    .child(
                        ActionButton::new(
                            SharedString::from(format!("trigger-remove-{id}")),
                            t.t("automation.macros.remove"),
                        )
                        .variant(ButtonVariant::Danger)
                        .on_click(cx.listener(move |v, _, _, cx| v.trigger_delete(&remove, cx))),
                    ),
            );
        }
        if let Some(draft) = self.triggers.draft.clone() {
            let id = draft["id"].as_str().unwrap_or("").to_string();
            let kind = draft["type"].as_str().unwrap_or("cue").to_string();
            let type_weak = weak.clone();
            panel = panel.child(Field::new(
                t.t("automation.macros.type"),
                Select::new(
                    "trigger-type",
                    TYPES
                        .iter()
                        .map(|x| SelectOption::new(x.to_string(), x.to_string()))
                        .collect(),
                    kind.clone(),
                )
                .on_change(move |value, _, cx| {
                    type_weak
                        .update(cx, |v, cx| {
                            v.trigger_edit(
                                |d| {
                                    d["type"] = json!(value);
                                    if let Some(o) = d.as_object_mut() {
                                        o.remove("targetId");
                                        o.remove("menuItem");
                                    }
                                },
                                cx,
                            )
                        })
                        .ok();
                }),
            ));
            let target_weak = weak.clone();
            panel = panel.child(Field::new(
                t.t("automation.macros.target"),
                Select::new(
                    "trigger-target",
                    target_options(&workspace, &groups, &kind)
                        .into_iter()
                        .map(|(i, l)| SelectOption::new(i, l))
                        .collect(),
                    draft["targetId"].as_str().map(str::to_string),
                )
                .on_change(move |value, _, cx| {
                    target_weak
                        .update(cx, |v, cx| {
                            v.trigger_edit(|d| d["targetId"] = json!(value), cx)
                        })
                        .ok();
                }),
            ));
            panel = panel.child(self.text_field(
                &format!("trigger-shortcut-{id}"),
                t.t("settings.keys.shortcut"),
                draft["shortcut"].as_str().unwrap_or(""),
                window,
                cx,
                |v, s, cx| {
                    v.trigger_edit(
                        |d| {
                            if s.trim().is_empty() {
                                if let Some(o) = d.as_object_mut() {
                                    o.remove("shortcut");
                                }
                            } else {
                                d["shortcut"] = json!(s.trim());
                            }
                        },
                        cx,
                    )
                },
            ));
            for (key, label, min, max, default) in [
                ("channel", "automation.bindings.channel", 1., 16., 1u64),
                ("pitch", "automation.bindings.pitch", 0., 127., 60),
                ("velocity", "automation.bindings.velocity", 1., 127., 1),
            ] {
                let current = draft["midi"][key].as_u64().unwrap_or(default).to_string();
                panel = panel.child(self.number_field(
                    &format!("trigger-{key}-{id}"),
                    t.t(label),
                    &current,
                    (min, max),
                    1.,
                    window,
                    cx,
                    move |v, text, cx| {
                        if let Ok(n) = text.trim().parse::<u64>() {
                            v.trigger_edit(
                                |d| {
                                    if d["midi"].is_null() {
                                        d["midi"] =
                                            json!({"channel": 1, "pitch": 60, "velocity": 1});
                                    }
                                    d["midi"][key] = json!(n);
                                },
                                cx,
                            )
                        }
                    },
                ));
            }
            panel = panel.child(
                action_row(cx)
                    .child(
                        ActionButton::new("trigger-save", t.t("automation.macros.save"))
                            .disabled(self.triggers.busy)
                            .on_click(cx.listener(|v, _, _, cx| v.trigger_save(cx))),
                    )
                    .child(
                        ActionButton::new("trigger-midi-clear", t.t("settings.general.clear"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.trigger_edit(
                                    |d| {
                                        if let Some(o) = d.as_object_mut() {
                                            o.remove("midi");
                                        }
                                    },
                                    cx,
                                )
                            })),
                    )
                    .child(
                        ActionButton::new("trigger-cancel", t.t("videoInputs.cancel"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.triggers.draft = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        let warning = cx.theme().colors.warning;
        panel
            .children(
                self.triggers
                    .error
                    .clone()
                    .map(|e| div().id("trigger-error").text_color(warning).child(e)),
            )
            .into_any_element()
    }
}
