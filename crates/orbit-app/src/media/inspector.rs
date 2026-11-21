use super::*;
use orbit_ui::controls::{CheckboxField, Field, InputEvent, NumberInput};
use std::collections::HashMap;

pub(super) type Inputs = HashMap<String, (Entity<InputState>, Subscription)>;

impl MediaBin {
    pub(super) fn patch_asset(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        if let Some(field) = key.strip_prefix("mediaTransport.") {
            let Some(item) = self.media.iter().find(|item| item["id"] == self.selected) else {
                return;
            };
            self.transport_baseline
                .get_or_insert_with(|| item["mediaTransport"].clone());
            let draft = self.transport_draft.get_or_insert_with(|| {
                item.get("mediaTransport").cloned().unwrap_or_else(|| json!({
                    "inSeconds":0,"outSeconds":null,"rate":1,"delaySeconds":0,"retrigger":"restart"
                }))
            });
            draft[field] = value;
            cx.notify();
            return;
        }
        let mut media = self.media.clone();
        let Some(item) = media.iter_mut().find(|item| item["id"] == self.selected) else {
            return;
        };
        item[key] = value;
        self.mutate("saveMediaSettings", vec![json!(media)], cx);
    }
    pub(super) fn save_transport(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.media.iter().find(|item| item["id"] == self.selected) else {
            return;
        };
        if self
            .transport_baseline
            .as_ref()
            .is_some_and(|baseline| baseline != &item["mediaTransport"])
        {
            this_status(
                self,
                cx,
                cx.global::<Translator>().t("media.transport.invalid"),
            );
            return;
        }
        let mut draft = self.transport_draft.clone().or_else(||item.get("mediaTransport").cloned()).unwrap_or_else(||json!({"inSeconds":0,"outSeconds":null,"rate":1,"delaySeconds":0,"retrigger":"restart"}));
        for field in ["inSeconds", "outSeconds", "rate", "delaySeconds"] {
            if let Some((input, _)) = self.inputs.get(&format!("mediaTransport.{field}")) {
                let value = input.read(cx).value().to_string();
                draft[field] = if field == "outSeconds" && value.is_empty() {
                    Value::Null
                } else if let Ok(value) = value.parse::<f64>() {
                    json!(value)
                } else {
                    json!(value)
                };
            }
        }
        if orbit_host::validate_preview_media_transport(&draft).is_err() {
            this_status(
                self,
                cx,
                cx.global::<Translator>().t("media.transport.invalid"),
            );
            return;
        }
        self.patch_asset("mediaTransport", draft, cx);
    }
    pub(super) fn numeric(
        &mut self,
        key: &str,
        label: &str,
        value: f64,
        (min, max): (f64, f64),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let input = if let Some((input, _)) = self.inputs.get(key) {
            input.clone()
        } else {
            let initial = if key == "mediaTransport.outSeconds"
                && self
                    .media
                    .iter()
                    .find(|item| item["id"] == self.selected)
                    .is_some_and(|item| item["mediaTransport"]["outSeconds"].is_null())
            {
                String::new()
            } else {
                value.to_string()
            };
            let input = input_state(initial.clone(), window, cx);
            let owner = self.selected.clone();
            let field = key.to_string();
            let subscription = cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, _, cx| {
                    if this.selected != owner
                        || !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                    {
                        return;
                    }
                    let value = input.read(cx).value().to_string();
                    if value == initial {
                        return;
                    }
                    if field == "mediaTransport.outSeconds" && value.is_empty() {
                        this.patch_asset(&field, Value::Null, cx);
                        return;
                    }
                    let parsed = value
                        .parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite() && *value >= min && *value <= max);
                    if let Some(value) = parsed {
                        if field == "previewPosition" {
                            this.preview_seek(value, cx);
                            return;
                        }
                        if field == "thumbnailTimeSeconds" {
                            return;
                        }
                        if field.starts_with("route:") {
                            this.route_field(&field, json!(value), cx);
                        } else if field.starts_with("effect:") {
                            this.effect_variable(&field, json!(value), cx);
                        } else {
                            this.patch_asset(&field, json!(value), cx);
                        }
                    } else {
                        this.runtime.status.update(cx, |status, cx| {
                            status.message = cx.global::<Translator>().t("media.transport.invalid");
                            cx.notify();
                        });
                    }
                },
            );
            self.inputs
                .insert(key.to_string(), (input.clone(), subscription));
            input
        };
        Field::new(
            cx.global::<Translator>().t(label),
            NumberInput::new(SharedString::from(key.to_string()), &input)
                .step(0.1)
                .range(min, max)
                .disabled(self.edits_blocked()),
        )
        .inline()
        .into_any_element()
    }
    fn choice(
        &self,
        key: &'static str,
        label: &str,
        value: &str,
        options: &[(&str, &str)],
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = *cx.global::<Translator>();
        let weak = cx.weak_entity();
        Field::new(
            t.t(label),
            Select::new(
                key,
                options
                    .iter()
                    .map(|(value, label)| SelectOption::new(value.to_string(), t.t(label)))
                    .collect(),
                value.to_string(),
            )
            .compact()
            .disabled(self.edits_blocked())
            .on_change(move |value, _, cx| {
                let _ = weak.update(cx, |this, cx| this.patch_asset(key, json!(value), cx));
            }),
        )
        .inline()
        .into_any_element()
    }
    pub(super) fn inspector(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if !self.inspector_open {
            return div().into_any_element();
        }
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let Some(item) = self
            .media
            .iter()
            .find(|item| item["id"] == self.selected)
            .cloned()
        else {
            // React keeps the inspector closed when the selected asset no longer exists.
            self.inspector_open = false;
            self.audition(false, cx);
            return div().into_any_element();
        };
        let tab = if inspector_tab_offered(&item, self.inspector_tab) {
            self.inspector_tab
        } else {
            0
        };
        let mut body = div()
            .id("media-inspector-scroll")
            .flex()
            .flex_col()
            .gap(theme.metrics.space_xs)
            .overflow_y_scroll()
            .min_h_0()
            .flex_1()
            .p(theme.metrics.space_xs);
        self.request_details(&item, cx);
        if item["type"] == "image" || item["type"] == "video" {
            self.request_effect_preview(&item, cx);
            body = body.child(
                div()
                    .h(theme.metrics.row_height * 6.)
                    .child(self.effect_preview.clone()),
            );
        }
        if item["type"] == "video" || item["type"] == "audio" {
            body = body.child(self.preview_controls(window, cx));
        }
        let weak = cx.weak_entity();
        body = body.child(
            orbit_ui::controls::TabList::new(
                "media-inspector-tabs",
                t.t("media.inspector.tabs"),
                [
                    "media.inspector.info",
                    "media.inspector.properties",
                    "media.effects.title",
                    "media.markers.tab",
                ]
                .iter()
                .enumerate()
                .filter(|(index, _)| inspector_tab_offered(&item, *index))
                .map(|(index, key)| orbit_ui::controls::TabItem::new(index, t.t(key)))
                .collect(),
                tab,
            )
            .on_change(move |tab, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.inspector_tab = tab;
                    cx.notify();
                });
            }),
        );
        if tab == 0 {
            let metadata = self.details.get(&self.selected);
            for (key, label) in [
                ("codec", "media.inspector.codec"),
                ("width", "media.inspector.size"),
                ("height", "media.inspector.size"),
                ("fps", "media.inspector.fps"),
                ("durationSeconds", "media.inspector.duration"),
            ] {
                let value = metadata
                    .map(|details| &details.metadata[key])
                    .filter(|value| !value.is_null())
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| value.to_string())
                    })
                    .unwrap_or_else(|| t.t("common.unknown"));
                body = body.child(Field::new(t.t(label), div().child(value)).inline());
            }
            if let Some(details) = metadata.filter(|details| !details.error.is_empty()) {
                body = body.child(
                    div()
                        .id("media-probe-error")
                        .role(gpui::Role::Alert)
                        .child(details.error.clone()),
                );
            }
            body = body.child(Field::new(
                t.t("media.inspector.filePath"),
                div()
                    .truncate()
                    .child(text(&item, "sourcePath").to_string()),
            ));
        }
        if tab == 1 && (item["type"] == "image" || item["type"] == "video") {
            body = body.child(self.choice(
                "mediaScale",
                "media.scaling",
                item["mediaScale"].as_str().unwrap_or("Scale to Fit"),
                &[
                    ("Scale to Fit", "media.scale.fit"),
                    ("Scale to Fill", "media.scale.fill"),
                    ("Stretch to Fill", "media.scale.stretch"),
                    ("Scale + Blur", "media.scale.blur"),
                ],
                cx,
            ));
            body = body.child(self.choice(
                "mediaAlignment",
                "media.alignment",
                item["mediaAlignment"].as_str().unwrap_or("Middle Center"),
                &[
                    ("Top Left", "media.alignment.top.left"),
                    ("Top Center", "media.alignment.top.center"),
                    ("Top Right", "media.alignment.top.right"),
                    ("Middle Left", "media.alignment.middle.left"),
                    ("Middle Center", "media.alignment.middle.center"),
                    ("Middle Right", "media.alignment.middle.right"),
                    ("Bottom Left", "media.alignment.bottom.left"),
                    ("Bottom Center", "media.alignment.bottom.center"),
                    ("Bottom Right", "media.alignment.bottom.right"),
                ],
                cx,
            ));
            for (key, label, neutral, min, max) in [
                ("mediaRotation", "media.rotation", 0., 0., 359.),
                ("mediaCropTop", "media.crop.top", 0., 0., f64::MAX),
                ("mediaCropRight", "media.crop.right", 0., 0., f64::MAX),
                ("mediaCropBottom", "media.crop.bottom", 0., 0., f64::MAX),
                ("mediaCropLeft", "media.crop.left", 0., 0., f64::MAX),
                ("mediaHue", "media.effects.hue", 0., -180., 180.),
                ("mediaSaturation", "media.effects.saturation", 1., 0., 2.),
                ("mediaBrightness", "media.effects.brightness", 0., -1., 1.),
                ("mediaContrast", "media.effects.contrast", 1., 0., 2.),
            ] {
                body = body.child(self.numeric(
                    key,
                    label,
                    item[key].as_f64().unwrap_or(neutral),
                    (min, max),
                    window,
                    cx,
                ));
            }
            for (key, label) in [
                ("mediaFlipX", "media.flipHorizontal"),
                ("mediaFlipY", "media.flipVertical"),
                ("mediaCrop", "media.crop"),
                ("mediaDesaturate", "media.effects.desaturate"),
            ] {
                let weak = cx.weak_entity();
                body = body.child(
                    CheckboxField::new(key, t.t(label), item[key] == true)
                        .disabled(self.edits_blocked())
                        .on_change(move |checked, _, cx| {
                            let _ = weak
                                .update(cx, |this, cx| this.patch_asset(key, json!(checked), cx));
                        }),
                );
            }
        }
        if tab == 2 && (item["type"] == "image" || item["type"] == "video") {
            body = body.child(self.effects(&item, window, cx));
        }
        if tab == 1 && (item["type"] == "audio" || item["type"] == "video") {
            body = body.child(self.routing(&item, window, cx));
        }
        if tab == 1 && item["type"] == "audio" {
            body = body.child(self.choice(
                "audioPlayback",
                "media.audioPlaylist.end",
                model::audio_behavior(&item, &self.libraries, &self.audio_library),
                &[
                    ("stop", "media.audioPlaylist.stop"),
                    ("loop", "media.playback.loop"),
                    ("next", "media.playback.next"),
                ],
                cx,
            ));
        }
        if tab == 1 && item["type"] == "video" {
            body = body.child(PanelHeader::new(t.t("media.transport.title")));
            for (key, neutral, min, max) in [
                ("inSeconds", 0., 0., f64::MAX),
                ("outSeconds", f64::MAX, 0., f64::MAX),
                ("rate", 1., 0.1, 4.),
                ("delaySeconds", 0., 0., f64::MAX),
            ] {
                let field = format!("mediaTransport.{key}");
                let label = format!("media.transport.{key}");
                body = body.child(self.numeric(
                    &field,
                    &label,
                    item["mediaTransport"][key].as_f64().unwrap_or(neutral),
                    (min, max),
                    window,
                    cx,
                ));
            }
            body = body.child(
                self.choice(
                    "mediaTransport.retrigger",
                    "media.transport.retrigger",
                    self.transport_draft
                        .as_ref()
                        .unwrap_or(&item["mediaTransport"])["retrigger"]
                        .as_str()
                        .unwrap_or("restart"),
                    &[
                        ("restart", "media.transport.restart"),
                        ("continue", "media.transport.continue"),
                        ("automatic", "media.transport.automatic"),
                    ],
                    cx,
                ),
            );
            body = body.child(
                ActionButton::new("transport-save", t.t("media.transport.save"))
                    .disabled(self.edits_blocked())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.save_transport(cx);
                    })),
            );
        }
        if tab == 0 && item["type"] == "video" {
            body = body.child(self.numeric(
                "thumbnailTimeSeconds",
                "media.thumbnail.time",
                item["thumbnailTimeSeconds"].as_f64().unwrap_or(0.),
                (0., f64::MAX),
                window,
                cx,
            ));
            for (reset, key) in [
                (false, "media.thumbnail.capture"),
                (true, "media.thumbnail.reset"),
            ] {
                body = body.child(
                    ActionButton::new(key, t.t(key))
                        .disabled(self.busy || reset && item["thumbnailFile"].is_null())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if reset {
                                this.mutate("resetVideoThumbnail", vec![json!(this.selected)], cx);
                            } else {
                                let seconds = this
                                    .inputs
                                    .get("thumbnailTimeSeconds")
                                    .and_then(|(input, _)| {
                                        input.read(cx).value().parse::<f64>().ok()
                                    })
                                    .filter(|seconds| seconds.is_finite() && *seconds >= 0.);
                                if let Some(seconds) = seconds {
                                    this.mutate(
                                        "setVideoThumbnail",
                                        vec![json!(this.selected), json!(seconds)],
                                        cx,
                                    );
                                } else {
                                    this.thumbnail_status =
                                        cx.global::<Translator>().t("media.thumbnail.invalidTime");
                                    cx.notify();
                                }
                            }
                        })),
                );
            }
        }
        if tab == 0 && !self.thumbnail_status.is_empty() {
            body = body.child(
                div()
                    .id("thumbnail-status")
                    .role(gpui::Role::Status)
                    .child(self.thumbnail_status.clone()),
            );
        }
        if tab == 3 && (item["type"] == "audio" || item["type"] == "video") {
            body = body.child(self.markers(&item, window, cx));
        }
        let weak = cx.weak_entity();
        orbit_ui::controls::Dialog::new(
            "media-asset-inspector",
            inspector_title(&item),
            t.t("media.inspector.close"),
            move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.inspector_open = false;
                    this.effect_preview_key = Value::Null;
                    this.audition(false, cx);
                    cx.notify();
                });
            },
        )
        .child(body)
        .into_any_element()
    }
}

/// Tabs 0 info, 1 properties, 2 effects (image and video), 3 markers (audio and video).
pub(super) fn inspector_tab_offered(item: &Value, tab: usize) -> bool {
    match tab {
        2 => matches!(item["type"].as_str(), Some("image" | "video")),
        3 => matches!(item["type"].as_str(), Some("audio" | "video")),
        _ => tab < 2,
    }
}

fn this_status(this: &MediaBin, cx: &mut Context<MediaBin>, error: String) {
    this.runtime.status.update(cx, |status, cx| {
        status.message = error;
        cx.notify();
    });
}

/// React shows the title, else the name, else the id.
pub(super) fn inspector_title(item: &Value) -> String {
    ["title", "name", "id"]
        .iter()
        .map(|key| text(item, key))
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}
