//! `VideoInputsSettingsBridge.tsx` and `VideoInputFields.tsx`: the video side of the Inputs settings tab. A list
//! of the inputs, and for the selected one rows of fields that save at once when the draft is valid. The name
//! saves when its field is left, as every text field of the settings does.

use gpui::{div, prelude::*, px, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{
    input_state, ActionButton, ButtonVariant, InputEvent, SelectOption, TextInput,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::video_inputs::*;
use crate::settings::rows::{select_row, setting_row, InputList};

fn choose(label: String) -> SelectOption<String> {
    SelectOption::new(String::new(), label).disabled(true)
}

impl VideoInputs {
    /// `known`: an id that was already sent to save, or that the workspace has.
    fn known(&self, id: &str, cx: &gpui::App) -> bool {
        self.saved.contains(id) || self.inputs(cx).iter().any(|input| input["id"] == id)
    }

    /// `change`: saves the draft when it is valid, creating it the first time; an invalid draft only shows why.
    pub(super) fn autosave(&mut self, cx: &mut Context<Self>) {
        let Some(mut input) = self.draft.clone() else {
            return;
        };
        if let Some(title) = input["title"].as_str() {
            input["title"] = json!(title.trim());
        }
        if let Err(error) = validate_video_input(&input) {
            self.error = error;
            cx.notify();
            return;
        }
        self.error.clear();
        let id = input["id"].as_str().unwrap_or("").to_string();
        let create = !self.known(&id, cx);
        self.saved.insert(id);
        self.mutate(
            move |current| saved_inputs(current, &input, create),
            false,
            cx,
        );
    }

    fn change_draft(&mut self, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        self.edit_draft(change, cx);
        self.autosave(cx);
    }

    fn select_input(&mut self, id: &str, cx: &mut Context<Self>) {
        self.error.clear();
        self.begin_edit(id, cx);
    }

    /// `removeSelected`: a draft that was never saved is just dropped.
    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self
            .draft
            .as_ref()
            .and_then(|draft| draft["id"].as_str())
            .map(str::to_string)
        else {
            return;
        };
        self.draft = None;
        self.name = None;
        if self.known(&id, cx) {
            self.saved.remove(&id);
            self.delete(id, cx);
        } else {
            self.sync_preview(cx);
            cx.notify();
        }
    }

    fn name_row(
        &mut self,
        draft: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let id = draft["id"].as_str().unwrap_or("").to_string();
        let state = match &self.name {
            Some((owner, state)) if *owner == id => state.clone(),
            _ => {
                let state = input_state(
                    draft["title"].as_str().unwrap_or("").to_string(),
                    window,
                    cx,
                );
                let subscription =
                    cx.subscribe(&state, |view, state, event: &InputEvent, cx| match event {
                        InputEvent::Change => {
                            let title = state.read(cx).value().to_string();
                            view.edit_draft(|draft| draft["title"] = json!(title), cx);
                        }
                        InputEvent::Blur | InputEvent::PressEnter { .. } => view.autosave(cx),
                        _ => {}
                    });
                self.name = Some((id.clone(), state.clone()));
                self.name_subscription = Some(subscription);
                state
            }
        };
        setting_row(
            t.t("videoInputs.name"),
            None,
            div()
                .w(px(420.))
                .child(TextInput::new("video-input-name", &state).label(t.t("videoInputs.name"))),
            cx,
        )
    }

    fn source_rows(&mut self, draft: &Value, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = *cx.global::<Translator>();
        let weak = cx.weak_entity();
        let busy = self.busy;
        let _ = busy;
        let kind_weak = weak.clone();
        let mut rows = vec![select_row(
            "video-input-kind",
            t.t("videoInputs.sourceType"),
            vec![
                SelectOption::new("ndi".to_string(), t.t("videoInputs.sourceNdi")),
                SelectOption::new("camera".to_string(), t.t("videoInputs.sourceCamera")),
            ],
            self.kind.to_string(),
            move |kind, _, cx| {
                kind_weak
                    .update(cx, |view, cx| {
                        view.set_kind(if kind == "camera" { "camera" } else { "ndi" }, cx);
                        view.autosave(cx);
                    })
                    .ok();
            },
            cx,
        )];
        if self.kind == "camera" {
            let current = draft["cameraSource"].clone();
            let devices = camera_device_options(current["deviceName"].as_str(), &self.cameras);
            let mode = if current.is_null() {
                (1280u64, 720u64, 30u64)
            } else {
                (
                    current["width"].as_u64().unwrap_or(0),
                    current["height"].as_u64().unwrap_or(0),
                    current["fps"].as_u64().unwrap_or(0),
                )
            };
            let mut modes: Vec<(u64, u64, u64)> = CAMERA_MODES
                .iter()
                .map(|(w, h, f)| (u64::from(*w), u64::from(*h), u64::from(*f)))
                .collect();
            if !modes.contains(&mode) {
                modes.insert(0, mode);
            }
            let camera_weak = weak.clone();
            let mut options = vec![choose(t.t("videoInputs.chooseCamera"))];
            options.extend(
                devices
                    .iter()
                    .map(|d| SelectOption::new(d.clone(), d.clone())),
            );
            rows.push(select_row(
                "video-input-camera",
                t.t("videoInputs.camera"),
                options,
                current["deviceName"].as_str().unwrap_or("").to_string(),
                move |device, _, cx| {
                    camera_weak
                        .update(cx, |view, cx| {
                            view.change_draft(
                                |draft| {
                                    let mut next = if draft["cameraSource"].is_null() {
                                        json!({"width": 1280, "height": 720, "fps": 30})
                                    } else {
                                        draft["cameraSource"].clone()
                                    };
                                    next["deviceName"] = json!(device);
                                    draft["cameraSource"] = next;
                                },
                                cx,
                            )
                        })
                        .ok();
                },
                cx,
            ));
            let format_weak = weak.clone();
            let lookup = modes.clone();
            rows.push(select_row(
                "video-input-camera-format",
                t.t("videoInputs.cameraFormat"),
                modes
                    .iter()
                    .map(|(w, h, f)| {
                        let key = camera_mode_key(*w, *h, *f);
                        SelectOption::new(key.clone(), key)
                    })
                    .collect(),
                camera_mode_key(mode.0, mode.1, mode.2),
                move |key, _, cx| {
                    let Some((w, h, f)) = lookup
                        .iter()
                        .find(|(w, h, f)| camera_mode_key(*w, *h, *f) == key)
                        .copied()
                    else {
                        return;
                    };
                    format_weak
                        .update(cx, |view, cx| {
                            // The format of a camera that is not chosen yet has nothing to change.
                            if view
                                .draft
                                .as_ref()
                                .is_some_and(|d| !d["cameraSource"].is_null())
                            {
                                view.change_draft(
                                    |draft| {
                                        draft["cameraSource"]["width"] = json!(w);
                                        draft["cameraSource"]["height"] = json!(h);
                                        draft["cameraSource"]["fps"] = json!(f);
                                    },
                                    cx,
                                )
                            }
                        })
                        .ok();
                },
                cx,
            ));
        } else {
            let selected = Some(&draft["ndiSource"]).filter(|source| !source.is_null());
            let options = ndi_source_options(selected, &self.sources);
            let by_key: Vec<(String, Value)> = options
                .iter()
                .map(|source| (source.to_string(), source.clone()))
                .collect();
            let mut choices = vec![choose(t.t("ndi.chooseSource"))];
            choices.extend(options.iter().map(|source| {
                SelectOption::new(
                    source.to_string(),
                    source["name"].as_str().unwrap_or("").to_string(),
                )
            }));
            let source_weak = weak.clone();
            rows.push(select_row(
                "video-input-ndi",
                t.t("ndi.source"),
                choices,
                selected.map(Value::to_string).unwrap_or_default(),
                move |key, _, cx| {
                    let Some((_, source)) = by_key.iter().find(|(k, _)| *k == key) else {
                        return;
                    };
                    let source =
                        json!({"name": source["name"], "urlAddress": source["urlAddress"]});
                    source_weak
                        .update(cx, |view, cx| {
                            view.change_draft(|draft| draft["ndiSource"] = source, cx)
                        })
                        .ok();
                },
                cx,
            ));
        }
        let scale_weak = weak.clone();
        rows.push(select_row(
            "video-input-scale",
            t.t("ndi.scale"),
            vec![
                choose(t.t("videoInputs.scaleRequired")),
                SelectOption::new("fit".to_string(), t.t("media.scale.fit")),
                SelectOption::new("fill".to_string(), t.t("media.scale.fill")),
                SelectOption::new("stretch".to_string(), t.t("media.scale.stretch")),
            ],
            draft["scale"].as_str().unwrap_or("").to_string(),
            move |scale, _, cx| {
                scale_weak
                    .update(cx, |view, cx| {
                        view.change_draft(|draft| draft["scale"] = json!(scale), cx)
                    })
                    .ok();
            },
            cx,
        ));
        let loading = self.loading;
        rows.push(setting_row(
            t.t("ndi.refresh"),
            None,
            ActionButton::new(
                "video-input-refresh",
                t.t(if loading {
                    "ndi.discovering"
                } else {
                    "ndi.refresh"
                }),
            )
            .variant(ButtonVariant::Secondary)
            .disabled(loading)
            .on_click(cx.listener(|view, _, _, cx| view.discover(cx))),
            cx,
        ));
        rows
    }

    pub(crate) fn settings_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let available = !self.runtime.content.read(cx).workspace_path.is_empty();
        let inputs = self.inputs(cx);
        let draft = self.draft.clone();
        let mut items: Vec<(String, String)> = inputs
            .iter()
            .map(|input| {
                let id = input["id"].as_str().unwrap_or("").to_string();
                let title = match &draft {
                    Some(draft) if draft["id"] == id.as_str() => draft["title"].clone(),
                    _ => input["title"].clone(),
                };
                (id, title.as_str().unwrap_or("").to_string())
            })
            .collect();
        if let Some(draft) = &draft {
            if !inputs.iter().any(|input| input["id"] == draft["id"]) {
                items.push((
                    draft["id"].as_str().unwrap_or("").to_string(),
                    draft["title"].as_str().unwrap_or("").to_string(),
                ));
            }
        }
        let (select_weak, add_weak, remove_weak) =
            (cx.weak_entity(), cx.weak_entity(), cx.weak_entity());
        let list = InputList {
            id: "video-inputs-list",
            items,
            selected: draft
                .as_ref()
                .and_then(|draft| draft["id"].as_str())
                .unwrap_or("")
                .to_string(),
            empty: t.t("videoInputs.empty").into(),
            add: t.t("inputs.add").into(),
            remove: t.t("inputs.remove").into(),
            disabled: !available,
        }
        .render(
            move |id, _, cx| {
                select_weak
                    .update(cx, |view, cx| view.select_input(&id, cx))
                    .ok();
            },
            move |_, cx| {
                add_weak
                    .update(cx, |view, cx| {
                        view.error.clear();
                        view.begin_new(cx);
                    })
                    .ok();
            },
            move |_, cx| {
                remove_weak
                    .update(cx, |view, cx| view.remove_selected(cx))
                    .ok();
            },
            cx,
        );
        let mut body = div()
            .flex()
            .flex_col()
            .gap(theme.metrics.control_gap)
            .child(list);
        if let Some(draft) = draft {
            let mut rows = vec![self.name_row(&draft, window, cx)];
            rows.extend(self.source_rows(&draft, cx));
            body = body.child(crate::settings::rows::group(rows, cx));
            if let Some(preview) = self.preview_box(t, cx) {
                body = body.child(
                    crate::settings::rows::group([preview], cx)
                        .flex()
                        .items_center()
                        .p(theme.metrics.space_sm),
                );
            }
        }
        body.children((!self.error.is_empty()).then(|| {
            let message = if self.error.starts_with("videoInputs.") {
                t.t(&self.error)
            } else {
                self.error.clone()
            };
            div()
                .id(SharedString::from("video-input-error"))
                .text_color(theme.colors.warning)
                .child(message)
        }))
        .into_any_element()
    }
}
