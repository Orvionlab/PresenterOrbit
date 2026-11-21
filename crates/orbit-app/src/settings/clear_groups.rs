//! `ClearGroupsPanel.tsx`, `ClearGroupDetailsEditor.tsx`, and `shared/clear-groups.ts`. The draft list is saved
//! as one `clearGroups` workspace section. Icon import (a file picker and image encoding) is not built.

use gpui::{
    prelude::*, AnyElement, Context, MouseButton, MouseDownEvent, Pixels, Point, SharedString,
    Window,
};
use orbit_ui::controls::{
    ActionButton, ButtonVariant, CheckboxField, ColorPicker, ContextMenu, MenuItem,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::ui::{action_row, heading, stack};
use super::view::SettingsView;
use crate::collections::model::Snapshot;

pub const LAYERS: [&str; 9] = [
    "audio",
    "audio-effects",
    "messages",
    "prop",
    "announcements",
    "slide",
    "background",
    "video-input",
    "stage-messages",
];
const FLAGS: [&str; 4] = [
    "hiddenInPreview",
    "stopPresentationTimeline",
    "stopAnnouncementsTimeline",
    "clearPresentationNext",
];

/// `clearGroupError`: a translation key.
pub fn group_error(group: &Value) -> Option<&'static str> {
    if group["id"].as_str().is_none_or(|s| s.trim().is_empty())
        || group["title"].as_str().is_none_or(|s| s.trim().is_empty())
    {
        return Some("clearGroups.nameRequired");
    }
    let Some(layers) = group["layers"].as_array().filter(|l| !l.is_empty()) else {
        return Some("clearGroups.layersRequired");
    };
    if layers.iter().any(|l| !LAYERS.iter().any(|s| l == s)) {
        return Some("clearGroups.unsupported");
    }
    if FLAGS
        .iter()
        .any(|f| !group[*f].is_null() && !group[*f].is_boolean())
    {
        return Some("clearGroups.invalid");
    }
    None
}

/// `uniqueClearGroupTitle`: appends a number until no group has the title.
pub fn unique_title(groups: &[Value], base: &str) -> String {
    let taken = |title: &str| {
        groups.iter().any(|g| {
            g["title"]
                .as_str()
                .is_some_and(|t| t.trim().eq_ignore_ascii_case(title.trim()))
        })
    };
    let mut title = base.trim().to_string();
    let mut n = 2;
    while taken(&title) {
        title = format!("{} {n}", base.trim());
        n += 1;
    }
    title
}

#[derive(Default)]
pub struct ClearGroups {
    pub draft: Option<Vec<Value>>,
    pub active: String,
    pub error: Option<String>,
    pub busy: bool,
    pub menu: Option<(String, Point<Pixels>)>,
}

impl SettingsView {
    fn clear_draft(&mut self, cx: &gpui::App) -> Vec<Value> {
        self.clear_groups
            .draft
            .get_or_insert_with(|| {
                self.runtime.content.read(cx).workspace["clearGroups"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .clone()
    }

    pub(super) fn clear_edit(
        &mut self,
        change: impl FnOnce(&mut Vec<Value>, &mut String),
        cx: &mut Context<Self>,
    ) {
        let mut draft = self.clear_draft(cx);
        let mut active = self.clear_groups.active.clone();
        change(&mut draft, &mut active);
        self.clear_groups.draft = Some(draft);
        self.clear_groups.active = active;
        cx.notify();
    }

    pub(super) fn clear_add(&mut self, cx: &mut Context<Self>) {
        let t = *cx.global::<Translator>();
        self.clear_edit(
            |draft, active| {
                let id = uuid::Uuid::new_v4().to_string();
                let title = unique_title(draft, &t.t("clearGroups.new"));
                draft.push(json!({"id": id, "title": title, "layers": ["slide"]}));
                *active = id;
            },
            cx,
        );
    }

    pub(super) fn clear_duplicate(&mut self, cx: &mut Context<Self>) {
        let t = *cx.global::<Translator>();
        self.clear_edit(
            |draft, active| {
                let Some(at) = draft.iter().position(|g| g["id"] == active.as_str()) else {
                    return;
                };
                let mut copy = draft[at].clone();
                let id = uuid::Uuid::new_v4().to_string();
                copy["id"] = json!(id);
                copy["title"] = json!(unique_title(
                    draft,
                    &format!(
                        "{} {}",
                        draft[at]["title"].as_str().unwrap_or("").trim(),
                        t.t("common.copy")
                    )
                ));
                draft.insert(at + 1, copy);
                *active = id;
            },
            cx,
        );
    }

    pub(super) fn clear_delete(&mut self, cx: &mut Context<Self>) {
        self.clear_edit(
            |draft, active| {
                let Some(at) = draft.iter().position(|g| g["id"] == active.as_str()) else {
                    return;
                };
                draft.remove(at);
                *active = draft
                    .get(at.saturating_sub(1))
                    .and_then(|g| g["id"].as_str())
                    .unwrap_or("")
                    .to_string();
            },
            cx,
        );
    }

    pub(super) fn clear_move(&mut self, offset: isize, cx: &mut Context<Self>) {
        self.clear_edit(
            |draft, active| {
                let Some(at) = draft.iter().position(|g| g["id"] == active.as_str()) else {
                    return;
                };
                let target = at as isize + offset;
                if target >= 0 && (target as usize) < draft.len() {
                    draft.swap(at, target as usize);
                }
            },
            cx,
        );
    }

    pub(super) fn clear_patch(&mut self, patch: Value, cx: &mut Context<Self>) {
        self.clear_edit(
            |draft, active| {
                if let Some(group) = draft.iter_mut().find(|g| g["id"] == active.as_str()) {
                    for (k, v) in patch.as_object().into_iter().flatten() {
                        group[k] = v.clone();
                    }
                }
            },
            cx,
        );
    }

    pub(super) fn clear_toggle_layer(&mut self, layer: &str, cx: &mut Context<Self>) {
        let layer = layer.to_string();
        self.clear_edit(
            |draft, active| {
                if let Some(group) = draft.iter_mut().find(|g| g["id"] == active.as_str()) {
                    let mut layers = group["layers"].as_array().cloned().unwrap_or_default();
                    match layers.iter().position(|l| *l == layer) {
                        Some(at) => {
                            layers.remove(at);
                        }
                        None => layers.push(json!(layer)),
                    }
                    layers.sort_by_key(|l| LAYERS.iter().position(|s| l == s));
                    group["layers"] = Value::Array(layers);
                }
            },
            cx,
        );
    }

    pub(super) fn clear_save(&mut self, cx: &mut Context<Self>) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        let draft = self.clear_draft(cx);
        if path.is_empty() || self.clear_groups.busy {
            return;
        }
        if let Some(error) = draft.iter().find_map(group_error) {
            self.clear_groups.error = Some(cx.global::<Translator>().t(error));
            cx.notify();
            return;
        }
        #[cfg(test)]
        self.runtime
            .sent
            .borrow_mut()
            .push(("saveWorkspaceContent", json!({"clearGroups": draft})));
        self.clear_groups.busy = true;
        self.clear_groups.error = None;
        let (host, scope, groups) = (self.runtime.host.clone(), path.clone(), draft);
        let answer = self.runtime.lanes.workspace(&path, move |_| {
            let snapshot = host.invoke("readWorkspaceSnapshot", vec![json!(scope)])?;
            let saved = host.invoke(
                "saveWorkspaceContent",
                vec![
                    json!(scope),
                    json!({"clearGroups": groups}),
                    snapshot["revision"].clone(),
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
                view.clear_groups.busy = false;
                match parsed {
                    Ok(snapshot) if runtime.content.read(cx).workspace_path == path => {
                        runtime.content.update(cx, |content, cx| {
                            content.apply_canonical(snapshot);
                            cx.notify();
                        });
                        view.clear_groups.draft = None;
                    }
                    Ok(_) => {}
                    Err(error) => view.clear_groups.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn clear_groups_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let warning = cx.theme().colors.warning;
        let draft = self.clear_draft(cx);
        let active = self.clear_groups.active.clone();
        let mut list = action_row(cx);
        for group in &draft {
            let id = group["id"].as_str().unwrap_or("").to_string();
            list = list.child(
                gpui::div()
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener({
                            let id = id.clone();
                            move |view, event: &MouseDownEvent, _, cx| {
                                view.clear_groups.active = id.clone();
                                view.clear_groups.menu = Some((id.clone(), event.position));
                                cx.notify();
                            }
                        }),
                    )
                    .child(
                        ActionButton::new(
                            SharedString::from(format!("clear-group-{id}")),
                            group["title"].as_str().unwrap_or("").to_string(),
                        )
                        .variant(if id == active {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.clear_groups.active = id.clone();
                            cx.notify();
                        })),
                    ),
            );
        }
        let mut panel = stack(cx)
            .id("settings-clear-groups")
            .child(heading(t.t("clearGroups.title"), cx))
            .child(list)
            .child(
                action_row(cx)
                    .child(
                        ActionButton::new("clear-add", t.t("clearGroups.add"))
                            .on_click(cx.listener(|v, _, _, cx| v.clear_add(cx))),
                    )
                    .child(
                        ActionButton::new("clear-copy", t.t("common.copy"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|v, _, _, cx| v.clear_duplicate(cx))),
                    )
                    .child(
                        ActionButton::new("clear-up", "^")
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|v, _, _, cx| v.clear_move(-1, cx))),
                    )
                    .child(
                        ActionButton::new("clear-down", "v")
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|v, _, _, cx| v.clear_move(1, cx))),
                    )
                    .child(
                        ActionButton::new("clear-delete", t.t("common.delete"))
                            .variant(ButtonVariant::Danger)
                            .on_click(cx.listener(|v, _, _, cx| v.clear_delete(cx))),
                    ),
            );
        if let Some(group) = draft.iter().find(|g| g["id"] == active.as_str()).cloned() {
            let id = active.clone();
            panel = panel.child(self.text_field(
                &format!("clear-title-{id}"),
                t.t("clearGroups.name"),
                group["title"].as_str().unwrap_or(""),
                window,
                cx,
                |v, title, cx| v.clear_patch(json!({"title": title.trim()}), cx),
            ));
            for layer in LAYERS {
                let on = group["layers"]
                    .as_array()
                    .is_some_and(|l| l.iter().any(|x| x == layer));
                let weak = cx.weak_entity();
                panel = panel.child(
                    CheckboxField::new(
                        SharedString::from(format!("clear-layer-{layer}")),
                        layer.to_string(),
                        on,
                    )
                    .on_change(move |_, _, cx| {
                        weak.update(cx, |v, cx| v.clear_toggle_layer(layer, cx))
                            .ok();
                    }),
                );
            }
            let tinted = group["iconTinted"] == json!(true);
            let tint_weak = cx.weak_entity();
            panel = panel.child(
                action_row(cx)
                    .child(ActionButton::new("clear-icon-import", t.t("clearGroups.icon")).variant(ButtonVariant::Secondary).on_click(cx.listener(|view, _, _, cx| {
                        let path = view.runtime.content.read(cx).workspace_path.clone();
                        view.host_call("selectMacroImage", vec![serde_json::json!(path)], cx, |view, result, cx| match result {
                            Ok(icon) if icon["imageData"].is_string() => view.clear_patch(json!({"imageType": icon["imageType"], "imageData": icon["imageData"]}), cx),
                            Ok(_) => {}
                            Err(error) => {
                                view.clear_groups.error = Some(error);
                                cx.notify();
                            }
                        });
                    })))
                    .children(group["imageData"].is_string().then(|| ActionButton::new("clear-icon-remove", t.t("common.none")).variant(ButtonVariant::Secondary).on_click(cx.listener(|view, _, _, cx| {
                        view.clear_edit(|draft, active| {
                            if let Some(group) = draft.iter_mut().find(|g| g["id"] == active.as_str()) {
                                if let Some(o) = group.as_object_mut() {
                                    for key in ["imageData", "imageType", "iconTinted", "iconTint"] {
                                        o.remove(key);
                                    }
                                }
                            }
                        }, cx)
                    })))),
            );
            if group["imageData"].is_string() {
                let weak = cx.weak_entity();
                panel = panel.child(CheckboxField::new("clear-icon-tinted", t.t("clearGroups.tint"), tinted).on_change(move |on, _, cx| {
                    weak.update(cx, |v, cx| v.clear_patch(json!({"iconTinted": on, "iconTint": if on { json!("white") } else { Value::Null }}), cx)).ok();
                }));
                if tinted {
                    panel = panel.child(
                        ColorPicker::new(
                            "clear-icon-tint",
                            t.t("clearGroups.tint"),
                            group["iconTint"].as_str().unwrap_or("white").to_string(),
                        )
                        .on_change(move |color, _, cx| {
                            tint_weak
                                .update(cx, |v, cx| v.clear_patch(json!({"iconTint": color}), cx))
                                .ok();
                        }),
                    );
                }
            }
            for flag in FLAGS {
                let weak = cx.weak_entity();
                panel = panel.child(
                    CheckboxField::new(
                        SharedString::from(format!("clear-flag-{flag}")),
                        flag.to_string(),
                        group[flag] == json!(true),
                    )
                    .on_change(move |on, _, cx| {
                        weak.update(cx, |v, cx| v.clear_patch(json!({ flag: on }), cx))
                            .ok();
                    }),
                );
            }
        }
        let menu = self.clear_groups.menu.clone().map(|(_, position)| {
            let (weak, close) = (cx.weak_entity(), cx.weak_entity());
            ContextMenu::new(
                "clear-group-menu",
                position,
                vec![MenuItem::new(t.t("common.delete"))
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.clear_groups.menu = None;
                            view.clear_delete(cx);
                        })
                        .ok();
                    })
                    .into()],
                move |_, _, cx| {
                    let close = close.clone();
                    cx.defer(move |cx| {
                        close
                            .update(cx, |view, cx| {
                                view.clear_groups.menu = None;
                                cx.notify();
                            })
                            .ok();
                    });
                },
            )
        });
        panel
            .children(menu)
            .child(
                ActionButton::new("clear-save", t.t("common.save"))
                    .disabled(self.clear_groups.busy)
                    .on_click(cx.listener(|v, _, _, cx| v.clear_save(cx))),
            )
            .children(
                self.clear_groups
                    .error
                    .clone()
                    .map(|e| gpui::div().id("clear-error").text_color(warning).child(e)),
            )
            .into_any_element()
    }
}
