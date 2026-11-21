//! The preview loop and the rendering of the video input panel.

use gpui::{div, prelude::*, px, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{
    input_state, ActionButton, ButtonVariant, Field, InputEvent, PanelHeader, Select, SelectOption,
    TextInput,
};
use orbit_ui::frame::FrameView;
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::video_inputs::*;

impl VideoInputs {
    /// Polls frames of the draft's source every 33 ms on a worker, as `startInputPreview` does. A missing frame
    /// is "waiting", not an error.
    pub(super) fn start_preview(&mut self, draft: Value, cx: &mut Context<Self>) {
        let id = draft["id"].as_str().unwrap_or("").to_string();
        let source = source_of(&draft);
        let camera = !draft["cameraSource"].is_null();
        let frames = cx.new(|cx| FrameView::new(|_, _| {}, cx));
        let (host, target) = (self.runtime.host.clone(), frames.clone());
        let task = cx.spawn(async move |view, cx| loop {
            let (host, source) = (host.clone(), source.clone());
            let packet = cx
                .background_spawn(async move {
                    let packet = if camera {
                        let source = serde_json::from_value(source).map_err(|e| e.to_string())?;
                        host.render_camera_input_frame(source)?
                    } else {
                        let source = serde_json::from_value(source).map_err(|e| e.to_string())?;
                        host.render_ndi_input_frame(source)?
                    };
                    frame_from_packet(&packet).map(|(_, image)| image)
                })
                .await;
            let alive = view.update(cx, |view, cx| {
                let Some(preview) = view.preview.as_mut() else {
                    return false;
                };
                match &packet {
                    Ok(image) => {
                        preview.waiting = false;
                        preview.error = None;
                        target.update(cx, |frames, cx| frames.set_image(image.clone(), cx));
                    }
                    Err(error) if error.contains("NDI_WAITING_FOR_VIDEO") => preview.waiting = true,
                    Err(error) => preview.error = Some(error.clone()),
                }
                cx.notify();
                true
            });
            if !matches!(alive, Ok(true)) {
                break;
            }
            cx.background_executor()
                .timer(std::time::Duration::from_millis(33))
                .await;
        });
        self.preview_source = Some(source_of(&draft));
        self.preview = Some(Preview::new(id, frames, task));
    }

    fn source_fields(&mut self, t: Translator, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let Some(draft) = self.draft.clone() else {
            return div().into_any_element();
        };
        let weak = cx.weak_entity();
        let (kind_weak, source_weak) = (weak.clone(), weak.clone());
        let mut fields = div()
            .flex()
            .flex_col()
            .gap(theme.metrics.space_sm)
            .child(Field::new(
                t.t("videoInputs.sourceType"),
                Select::new(
                    "video-input-kind",
                    vec![
                        SelectOption::new("ndi".to_string(), t.t("videoInputs.sourceNdi")),
                        SelectOption::new("camera".to_string(), t.t("videoInputs.sourceCamera")),
                    ],
                    self.kind.to_string(),
                )
                .on_change(move |kind, _, cx| {
                    kind_weak
                        .update(cx, |view, cx| {
                            view.set_kind(if kind == "camera" { "camera" } else { "ndi" }, cx)
                        })
                        .ok();
                }),
            ));
        if self.kind == "camera" {
            let current = draft["cameraSource"].clone();
            let devices = camera_device_options(current["deviceName"].as_str(), &self.cameras);
            let mode_weak = source_weak.clone();
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
            let no_cameras = self.error.is_empty() && devices.is_empty();
            fields = fields
                .child(Field::new(
                    t.t("videoInputs.camera"),
                    Select::new(
                        "video-input-camera",
                        devices
                            .iter()
                            .map(|d| SelectOption::new(d.clone(), d.clone()))
                            .collect(),
                        current["deviceName"].as_str().map(str::to_string),
                    )
                    .on_change(move |device, _, cx| {
                        source_weak
                            .update(cx, |view, cx| {
                                view.edit_draft(
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
                    }),
                ))
                .children(no_cameras.then(|| div().child(t.t("videoInputs.noCameras"))))
                .child(Field::new(
                    t.t("videoInputs.cameraFormat"),
                    Select::new(
                        "video-input-camera-format",
                        modes
                            .iter()
                            .map(|(w, h, f)| {
                                let key = camera_mode_key(*w, *h, *f);
                                SelectOption::new(key.clone(), key)
                            })
                            .collect(),
                        camera_mode_key(mode.0, mode.1, mode.2),
                    )
                    .disabled(self.busy || current.is_null())
                    .on_change(move |key, _, cx| {
                        let Some((w, h, f)) = modes
                            .iter()
                            .find(|(w, h, f)| camera_mode_key(*w, *h, *f) == key)
                            .copied()
                        else {
                            return;
                        };
                        mode_weak
                            .update(cx, |view, cx| {
                                view.edit_draft(
                                    |draft| {
                                        draft["cameraSource"]["width"] = json!(w);
                                        draft["cameraSource"]["height"] = json!(h);
                                        draft["cameraSource"]["fps"] = json!(f);
                                    },
                                    cx,
                                )
                            })
                            .ok();
                    }),
                ));
        } else {
            let options = ndi_source_options(
                Some(&draft["ndiSource"]).filter(|source| !source.is_null()),
                &self.sources,
            );
            let by_key: Vec<(String, Value)> = options
                .iter()
                .map(|source| (source.to_string(), source.clone()))
                .collect();
            let selected = Some(&draft["ndiSource"])
                .filter(|source| !source.is_null())
                .map(|source| source.to_string());
            fields = fields.child(Field::new(
                t.t("ndi.source"),
                Select::new(
                    "video-input-ndi",
                    options
                        .iter()
                        .map(|source| {
                            SelectOption::new(
                                source.to_string(),
                                source["name"].as_str().unwrap_or("").to_string(),
                            )
                        })
                        .collect(),
                    selected,
                )
                .on_change(move |key, _, cx| {
                    let Some((_, source)) = by_key.iter().find(|(k, _)| *k == key) else {
                        return;
                    };
                    let source =
                        json!({"name": source["name"], "urlAddress": source["urlAddress"]});
                    source_weak
                        .update(cx, |view, cx| {
                            view.edit_draft(|draft| draft["ndiSource"] = source, cx)
                        })
                        .ok();
                }),
            ));
        }
        fields.into_any_element()
    }

    pub(super) fn preview_box(&self, t: Translator, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = *cx.theme();
        let preview = self.preview.as_ref()?;
        let status = if let Some(error) = &preview.error {
            Some(error.clone())
        } else if preview.waiting {
            Some(t.t("ndi.waiting"))
        } else {
            None
        };
        Some(
            div()
                .id("video-input-preview")
                .w(px(320.))
                .h(px(180.))
                .bg(theme.colors.preview_matte_bg)
                .child(preview.frames.clone())
                .children(
                    status.map(|status| div().text_color(theme.colors.text_muted).child(status)),
                )
                .into_any_element(),
        )
    }
}

impl Render for VideoInputs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.settings_mode {
            return self.settings_view(window, cx);
        }
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let inputs = self.inputs(cx);
        let live = self.live_id(cx);
        let busy = self.busy;
        let mut cards = div().flex().flex_wrap().gap(theme.metrics.panel_gap);
        for input in &inputs {
            let id = input["id"].as_str().unwrap_or("").to_string();
            let configured = !input["ndiSource"].is_null() || !input["cameraSource"].is_null();
            let on_air = live.as_deref() == Some(id.as_str());
            let subtitle = if on_air {
                t.t("videoInputs.live")
            } else {
                input["ndiSource"]["name"]
                    .as_str()
                    .or_else(|| input["cameraSource"]["deviceName"].as_str())
                    .map_or_else(|| t.t("videoInputs.unconfigured"), str::to_string)
            };
            let (take_id, edit_id) = (id.clone(), id.clone());
            cards = cards.child(
                div()
                    .id(SharedString::from(format!("video-input-{id}")))
                    .flex()
                    .flex_col()
                    .gap(theme.metrics.space_xs)
                    .w(px(220.))
                    .p(theme.metrics.space_xs)
                    .border_2()
                    .when(on_air, |card| card.border_color(theme.colors.live))
                    .child({
                        // `.video-input-card > button:first-child`: a 16:9 placeholder, the name, then the source or "live".
                        let enabled = !busy && configured;
                        div()
                            .id(SharedString::from(format!("video-input-take-{id}")))
                            .flex()
                            .flex_col()
                            .gap(theme.metrics.space_xs)
                            .w_full()
                            .when(enabled, |card| card.cursor_pointer())
                            .when(!enabled, |card| card.text_color(theme.colors.disabled_text))
                            .role(gpui::Role::Button)
                            .aria_label(input["title"].as_str().unwrap_or("").to_string())
                            .aria_selected(on_air)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w_full()
                                    .aspect_ratio(16. / 9.)
                                    .bg(theme.colors.panel_bg)
                                    .child("NDI"),
                            )
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(input["title"].as_str().unwrap_or("").to_string()),
                            )
                            .child(subtitle.clone())
                            .when(enabled, |card| {
                                card.on_click(
                                    cx.listener(move |view, _, _, cx| view.trigger(&take_id, cx)),
                                )
                            })
                    })
                    .child(
                        ActionButton::new(
                            SharedString::from(format!("video-input-edit-{id}")),
                            t.t("videoInputs.configure"),
                        )
                        .variant(ButtonVariant::Ghost)
                        .disabled(busy)
                        .on_click(cx.listener(move |view, _, _, cx| view.begin_edit(&edit_id, cx))),
                    ),
            );
        }
        let header = div()
            .flex()
            .items_center()
            .gap(theme.metrics.control_gap)
            .child(PanelHeader::new(t.t("videoInputs.title")))
            .child(
                ActionButton::new("video-input-add", t.t("videoInputs.add"))
                    .disabled(busy)
                    .on_click(cx.listener(|view, _, _, cx| view.begin_new(cx))),
            )
            .child(
                ActionButton::new("video-input-clear", t.t("videoInputs.clear"))
                    .variant(ButtonVariant::Secondary)
                    .disabled(busy || live.is_none())
                    .on_click(cx.listener(|view, _, _, cx| view.clear(cx))),
            );
        let editor = self.draft.clone().map(|draft| {
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
                        cx.subscribe(&state, |view, state, event: &InputEvent, cx| {
                            if matches!(event, InputEvent::Change) {
                                let title = state.read(cx).value().to_string();
                                view.edit_draft(|draft| draft["title"] = json!(title), cx);
                            }
                        });
                    self.name = Some((id.clone(), state.clone()));
                    self.name_subscription = Some(subscription);
                    state
                }
            };
            let scale_weak = cx.weak_entity();
            let saveable = (!draft["ndiSource"].is_null() || !draft["cameraSource"].is_null())
                && !draft["title"].as_str().unwrap_or("").trim().is_empty();
            let source_fields = self.source_fields(t, cx);
            let preview = self.preview_box(t, cx);
            let scale_options = vec![
                SelectOption::new("fit".to_string(), t.t("media.scale.fit")),
                SelectOption::new("fill".to_string(), t.t("media.scale.fill")),
                SelectOption::new("stretch".to_string(), t.t("media.scale.stretch")),
            ];
            div()
                .id("video-input-editor")
                .flex()
                .flex_col()
                .gap(theme.metrics.space_sm)
                .child(Field::new(
                    t.t("videoInputs.name"),
                    TextInput::new("video-input-name", &state),
                ))
                .child(source_fields)
                .children(preview)
                .child(Field::new(
                    t.t("ndi.scale"),
                    Select::new(
                        "video-input-scale",
                        scale_options,
                        draft["scale"].as_str().map(str::to_string),
                    )
                    .disabled(busy)
                    .on_change(move |scale, _, cx| {
                        scale_weak
                            .update(cx, |view, cx| {
                                view.edit_draft(|draft| draft["scale"] = json!(scale), cx)
                            })
                            .ok();
                    }),
                ))
                .child(
                    div()
                        .flex()
                        .gap(theme.metrics.control_gap)
                        .child(
                            ActionButton::new(
                                "video-input-refresh",
                                t.t(if self.loading {
                                    "ndi.discovering"
                                } else {
                                    "ndi.refresh"
                                }),
                            )
                            .variant(ButtonVariant::Secondary)
                            .disabled(busy || self.loading)
                            .on_click(cx.listener(|view, _, _, cx| view.discover(cx))),
                        )
                        .child(
                            ActionButton::new("video-input-save", t.t("videoInputs.save"))
                                .disabled(busy || !saveable)
                                .on_click(cx.listener(|view, _, _, cx| view.save(cx))),
                        )
                        .child(
                            ActionButton::new("video-input-cancel", t.t("videoInputs.cancel"))
                                .variant(ButtonVariant::Secondary)
                                .disabled(busy)
                                .on_click(cx.listener(|view, _, _, cx| view.cancel(cx))),
                        )
                        .children((!self.creating).then(|| {
                            ActionButton::new("video-input-delete", t.t("videoInputs.delete"))
                                .variant(ButtonVariant::Danger)
                                .disabled(busy)
                                .on_click(
                                    cx.listener(move |view, _, _, cx| view.delete(id.clone(), cx)),
                                )
                        })),
                )
        });
        let error = (!self.error.is_empty()).then(|| {
            let message = if self.error.starts_with("videoInputs.") {
                t.t(&self.error)
            } else {
                self.error.clone()
            };
            div()
                .id("video-input-error")
                .text_color(theme.colors.warning)
                .child(message)
        });
        div()
            .id("video-inputs")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(theme.metrics.panel_gap)
            .p(theme.metrics.panel_padding)
            .child(header)
            .child(div().child(t.t("videoInputs.help")))
            .children(error)
            .child(cards)
            .children(
                inputs
                    .is_empty()
                    .then(|| div().child(t.t("videoInputs.empty"))),
            )
            .children(editor)
            .into_any_element()
    }
}
