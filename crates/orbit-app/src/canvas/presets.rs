use super::{builds, view::SlideCanvas};
use gpui::{div, prelude::*, AnyElement, Context, SharedString};
use orbit_ui::{
    controls::{input_state, ActionButton, ButtonVariant, PanelHeader, Select, SelectOption},
    Translator,
};
use serde_json::json;

impl SlideCanvas {
    pub(super) fn transition_favorites(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let mut body = div().flex().flex_col();
        let Some(index) = self.selected else {
            return body.into_any_element();
        };
        let presets = self.runtime.content.read(cx).workspace["transitionPresets"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for prefix in ["build", "buildOut"] {
            if !self.slide["layers"][index][prefix].is_object() {
                continue;
            }
            let selected = self
                .editor
                .favorites
                .get(prefix)
                .cloned()
                .unwrap_or_default();
            body = body.child(PanelHeader::new(t.t(if prefix == "build" {
                "theme.gpui.buildTransition"
            } else {
                "theme.gpui.buildOutTransition"
            })));
            let mut options = vec![SelectOption::new(String::new(), t.t("transition.none"))];
            options.extend(presets.iter().filter_map(|preset| {
                Some(SelectOption::new(
                    preset["id"].as_str()?.to_string(),
                    preset["name"].as_str()?.to_string(),
                ))
            }));
            let weak = cx.weak_entity();
            body = body.child(
                Select::new(
                    SharedString::from(format!("{prefix}-favorite")),
                    options,
                    selected.clone(),
                )
                .label(t.t("transition.favorite"))
                .disabled(self.saving)
                .on_change(move |id, _, cx| {
                    weak.update(cx, |view, cx| {
                        let preset = view.runtime.content.read(cx).workspace["transitionPresets"]
                            .as_array()
                            .and_then(|presets| presets.iter().find(|preset| preset["id"] == id))
                            .cloned();
                        if let Some(preset) = preset {
                            if let Some(index) = view.selected {
                                match builds::patch(
                                    &view.slide["layers"][index],
                                    &view.slide,
                                    &format!("{prefix}Transition"),
                                    preset["transition"].clone(),
                                ) {
                                    Ok(layer) => {
                                        let mut slide = (*view.slide).clone();
                                        slide["layers"][index] = layer;
                                        view.editor.scope.clear();
                                        view.draft(slide, cx);
                                    }
                                    Err(error) => view.status = error,
                                }
                            }
                        }
                        view.editor.favorites.insert(prefix.into(), id.to_string());
                        cx.notify();
                    })
                    .ok();
                }),
            );
            let weak = cx.weak_entity();
            body = body.child(
                ActionButton::new(
                    SharedString::from(format!("{prefix}-save-favorite")),
                    t.t("transition.saveFavorite"),
                )
                .disabled(self.saving)
                .on_click(move |_, window, cx| {
                    weak.update(cx, |view, cx| {
                        view.editor.dialog =
                            Some((format!("preset:{prefix}"), input_state("", window, cx)));
                        cx.notify();
                    })
                    .ok();
                }),
            );
            if !selected.is_empty() {
                let weak = cx.weak_entity();
                body = body.child(
                    ActionButton::new(
                        SharedString::from(format!("{prefix}-delete-favorite")),
                        t.t("transition.deleteFavorite"),
                    )
                    .variant(ButtonVariant::Danger)
                    .disabled(self.saving)
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.save_transition_favorite(prefix, "", Some(selected.clone()), cx)
                        })
                        .ok();
                    }),
                );
            }
        }
        body.into_any_element()
    }
    pub(super) fn save_transition_favorite(
        &mut self,
        prefix: &str,
        name: &str,
        remove: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.saving {
            return;
        }
        let Some(index) = self.selected else {
            return;
        };
        let transition = self.slide["layers"][index][prefix]["transition"].clone();
        let transition = if transition.is_null() {
            json!({"effectId":builds::CUT,"durationSeconds":0})
        } else {
            transition
        };
        if remove.is_none() {
            if let Err(error) = builds::validate_transition(&transition) {
                self.status = error;
                cx.notify();
                return;
            }
            if name.trim().is_empty() {
                self.status = cx
                    .global::<Translator>()
                    .t("transition.favoriteNameRequired");
                cx.notify();
                return;
            }
        }
        let name = name.to_string();
        let path = self.runtime.content.read(cx).workspace_path.clone();
        let scope = path.clone();
        let host = self.runtime.host.clone();
        let revisions = self.runtime.lanes.revisions().clone();
        let answer = self.runtime.lanes.workspace(&path, move |_| {
            let snapshot = host.invoke("readWorkspaceSnapshot", vec![json!(scope)])?;
            revisions.remember(&scope, &snapshot);
            let revision = snapshot["revision"]
                .as_u64()
                .ok_or("transition.saveFailed")?;
            let mut presets = snapshot["workspace"]["transitionPresets"]
                .as_array()
                .ok_or("transition.saveFailed")?
                .clone();
            if let Some(id) = remove {
                let index = presets
                    .iter()
                    .position(|preset| preset["id"] == id)
                    .ok_or("transition.favoriteRemoved")?;
                presets.remove(index);
            } else {
                presets.push(orbit_host::new_transition_preset(&name, transition)?);
            }
            host.invoke(
                "saveWorkspaceContent",
                vec![
                    json!(scope),
                    json!({"transitionPresets":presets}),
                    json!(revision),
                ],
            )
        });
        self.saving = true;
        self.save_id += 1;
        let nonce = self.save_id;
        self.editor.dialog = None;
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let result = answer
                .await
                .map_err(|_| "The transition favorite worker stopped.".to_string())
                .and_then(|result| result);
            let parsed = cx
                .background_spawn(async move {
                    let result = result?;
                    crate::collections::model::Snapshot::parse(&result["workspace"])
                })
                .await;
            view.update(cx, |view, cx| {
                if nonce != view.save_id || runtime.content.read(cx).workspace_path != path {
                    return;
                }
                view.saving = false;
                match parsed {
                    Ok(snapshot) => {
                        runtime.content.update(cx, |content, cx| {
                            content.apply_document(snapshot);
                            cx.notify();
                        });
                        view.editor.favorites.clear();
                        view.status.clear();
                        if view.editor.base.is_some() {
                            view.draft((*view.slide).clone(), cx);
                        } else {
                            view.emit_close_if_requested(cx);
                        }
                    }
                    Err(error) => {
                        view.status = if error.starts_with("transition.") {
                            cx.global::<Translator>().t(&error)
                        } else {
                            error
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}
