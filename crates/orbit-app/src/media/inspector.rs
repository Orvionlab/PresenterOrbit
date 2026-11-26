use super::*;
use orbit_ui::controls::{
    range_state, CheckboxField, Field, InputEvent, NumberInput, RangeInput, SliderEvent,
};
use std::collections::HashMap;

pub(super) type Inputs = HashMap<String, (Entity<InputState>, Subscription)>;

impl MediaBin {
    pub(super) fn patch_asset(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        let mut media = self.media.clone();
        let Some(item) = media.iter_mut().find(|item| item["id"] == self.selected) else {
            return;
        };
        let (key, value) =
            if let Some(field) = key.strip_prefix("mediaTransport.") {
                let mut transport = item.get("mediaTransport").cloned().unwrap_or_else(|| json!({
                "inSeconds":0,"outSeconds":null,"rate":1,"delaySeconds":0,"retrigger":"restart"
            }));
                transport[field] = value;
                if orbit_host::validate_preview_media_transport(&transport).is_err() {
                    this_status(
                        self,
                        cx,
                        cx.global::<Translator>().t("media.transport.invalid"),
                    );
                    return;
                }
                ("mediaTransport", transport)
            } else {
                (key, value)
            };
        if item[key] != value {
            item[key] = value;
            self.mutate("saveMediaSettings", vec![json!(media)], cx);
        }
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
        let label = cx.global::<Translator>().t(label);
        Field::new(
            label.clone(),
            self.numeric_input(key, value, (min, max), window, cx)
                .label(label),
        )
        .inline()
        .label_width(cx.theme().metrics.space_lg * 7.)
        .into_any_element()
    }

    pub(super) fn numeric_input(
        &mut self,
        key: &str,
        value: f64,
        (min, max): (f64, f64),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> NumberInput {
        let input = if let Some((input, _)) = self.inputs.get(key) {
            input.clone()
        } else {
            let input = input_state(value.to_string(), window, cx);
            let last_value = std::cell::Cell::new(value);
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
                    let parsed = value
                        .parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite() && *value >= min && *value <= max);
                    if let Some(value) = parsed {
                        if !field.starts_with("route:") && last_value.replace(value) == value {
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
        NumberInput::new(SharedString::from(key.to_string()), &input)
            .step(0.1)
            .range(min, max)
            .disabled(self.edits_blocked())
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
                let _ = weak.update(cx, |this, cx| {
                    let value = if key == "mediaRotation" {
                        let Ok(angle) = value.parse::<f64>() else {
                            return;
                        };
                        json!(angle)
                    } else {
                        json!(value)
                    };
                    this.patch_asset(key, value, cx);
                });
            }),
        )
        .inline()
        .label_width(cx.theme().metrics.space_lg * 7.)
        .into_any_element()
    }
    fn property_time(
        &mut self,
        (key, label): (&'static str, &'static str),
        seconds: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let formatted = property_clock(seconds);
        let input = if let Some((input, _)) = self.inputs.get(key) {
            input.clone()
        } else {
            let input = input_state(formatted, window, cx);
            let owner = self.selected.clone();
            let subscription = cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, window, cx| {
                    if this.selected != owner
                        || !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                    {
                        return;
                    }
                    let Some(item) = this.media.iter().find(|item| item["id"] == owner) else {
                        return;
                    };
                    let duration = this.preview_duration();
                    let range = duration.map(|duration| {
                        super::preview_timeline::TrimRange::from_asset(item, duration)
                    });
                    let current = match key {
                        "preview-in-point" => range.map(|range| range.start),
                        "preview-out-point" => range.map(|range| range.end),
                        _ => Some(
                            item["mediaTransport"]["delaySeconds"]
                                .as_f64()
                                .unwrap_or(0.),
                        ),
                    };
                    let Some(current) = current else { return };
                    let text = input.read(cx).value().trim().to_owned();
                    if text == property_clock(current) {
                        return;
                    }
                    let requested = if text.is_empty() && key == "preview-out-point" {
                        duration
                    } else if text.is_empty() && key == "mediaTransport.delaySeconds" {
                        Some(0.)
                    } else {
                        parse_property_time(&text)
                    };
                    let valid = requested.is_some_and(|seconds| match (key, range) {
                        ("preview-in-point", Some(range)) => seconds < range.end,
                        ("preview-out-point", Some(range)) => {
                            seconds > range.start
                                && duration.is_some_and(|duration| seconds <= duration)
                        }
                        ("mediaTransport.delaySeconds", _) => true,
                        _ => false,
                    });
                    if !valid {
                        input.update(cx, |state, cx| {
                            state.set_value(property_clock(current), window, cx)
                        });
                        this_status(
                            this,
                            cx,
                            cx.global::<Translator>().t("media.transport.invalid"),
                        );
                        return;
                    }
                    let Some(seconds) = requested else { return };
                    if key == "mediaTransport.delaySeconds" {
                        this.patch_asset(key, json!(seconds), cx);
                        input.update(cx, |state, cx| {
                            state.set_value(property_clock(seconds), window, cx)
                        });
                    } else if let Some((duration, mut range)) = duration.zip(range) {
                        if key == "preview-in-point" {
                            range.start = seconds;
                        } else {
                            range.end = seconds;
                        }
                        this.commit_preview_trim(range, duration, cx);
                    }
                },
            );
            self.inputs
                .insert(key.into(), (input.clone(), subscription));
            input
        };
        let label = cx.global::<Translator>().t(label);
        Field::new(
            label.clone(),
            div().debug_selector(move || key.into()).child(
                TextInput::new(key, &input)
                    .compact()
                    .label(label)
                    .disabled(self.edits_blocked()),
            ),
        )
        .inline()
        .label_width(cx.theme().metrics.space_lg * 7.)
    }

    fn playback_rate(&mut self, rate: f64, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.edits_blocked();
        let (slider, _) = self.transport_rate_slider.get_or_insert_with(|| {
            let slider = range_state(10., 400., 1., (rate * 100.) as f32, cx);
            let subscription =
                cx.subscribe(&slider, |this, _, event: &SliderEvent, cx| match event {
                    SliderEvent::Change(_) => cx.notify(),
                    SliderEvent::Release(value) => this.patch_asset(
                        "mediaTransport.rate",
                        json!(f64::from(value.end()) / 100.),
                        cx,
                    ),
                });
            (slider, subscription)
        });
        let percent = slider.read(cx).value().end();
        let label = cx.global::<Translator>().t("media.transport.playRate");
        let m = cx.theme().metrics;
        Field::new(
            label.clone(),
            div()
                .flex()
                .items_center()
                .gap(m.space_sm)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(RangeInput::new(slider).label(label).disabled(disabled)),
                )
                .child(
                    div()
                        .min_w(m.space_lg * 3.)
                        .text_right()
                        .child(format!("{percent:.0}%")),
                ),
        )
        .inline()
        .label_width(m.space_lg * 7.)
    }

    fn inspector_info(
        &mut self,
        item: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let m = theme.metrics;
        let label_width = m.space_lg * 6.;
        let action_name = t.t("media.inspector.actionName");
        let input = if let Some((input, _)) = self.inputs.get("inspector-action-name") {
            input.clone()
        } else {
            let input = input_state(text(item, "title").to_owned(), window, cx);
            let owner = self.selected.clone();
            let subscription = cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, window, cx| {
                    if this.selected != owner
                        || !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                    {
                        return;
                    }
                    let Some(item) = this.media.iter().find(|item| item["id"] == owner) else {
                        return;
                    };
                    let current = text(item, "title");
                    let name = input.read(cx).value().trim().to_owned();
                    if name.is_empty() {
                        let current = current.to_owned();
                        input.update(cx, |state, cx| state.set_value(current, window, cx));
                    } else if name != current {
                        this.patch_asset("title", json!(name), cx);
                    }
                },
            );
            self.inputs.insert(
                "inspector-action-name".into(),
                (input.clone(), subscription),
            );
            input
        };
        let details = self.details.get(&self.selected);
        let metadata = |key: &str| {
            details
                .map(|details| &details.metadata[key])
                .unwrap_or(&Value::Null)
        };
        let source = text(item, "sourcePath");
        let source = if cfg!(windows) {
            orbit_host::strip_windows_verbatim_prefix(source)
        } else {
            source.to_owned()
        };
        let row = |label: String, content: gpui::AnyElement| {
            div()
                .flex()
                .items_start()
                .gap(m.space_sm)
                .child(
                    div()
                        .w(label_width)
                        .flex_none()
                        .text_right()
                        .text_color(theme.colors.text_muted)
                        .child(label),
                )
                .child(div().flex_1().min_w_0().child(content))
        };
        let mut body = div()
            .flex()
            .flex_col()
            .gap(m.space_sm)
            .child(
                Field::new(
                    action_name.clone(),
                    div()
                        .debug_selector(|| "inspector-action-name".into())
                        .child(
                            TextInput::new("inspector-action-name", &input)
                                .label(action_name)
                                .compact(),
                        ),
                )
                .inline()
                .label_width(label_width),
            )
            .child(row(
                t.t("media.inspector.filePath"),
                div()
                    .id("media-inspector-source-path")
                    .text_size(m.font_sm)
                    .tooltip(orbit_ui::controls::tooltip_text(source.clone()))
                    .child(source.replace('\\', "\\\u{200b}"))
                    .into_any_element(),
            ));
        if matches!(item["type"].as_str(), Some("image" | "video")) {
            let image = self
                .queued_thumbnail
                .as_ref()
                .filter(|capture| capture.id == self.selected)
                .and_then(|capture| capture.frame.as_ref().map(|frame| frame.image.clone()))
                .or_else(|| {
                    self.pending_thumbnail
                        .as_ref()
                        .filter(|(id, _)| id == &self.selected)
                        .and_then(|(_, image)| image.clone())
                })
                .or_else(|| details.and_then(|details| details.image.clone()));
            let mut thumbnail = div().flex().flex_col().gap(m.space_xs).child(
                div()
                    .relative()
                    .w_full()
                    .aspect_ratio(16. / 9.)
                    .overflow_hidden()
                    .bg(theme.colors.preview_matte_bg)
                    .children(image.map(|image| {
                        gpui::img(image)
                            .debug_selector(|| "media-inspector-thumbnail".into())
                            .absolute()
                            .inset_0()
                            .size_full()
                            .object_fit(gpui::ObjectFit::Contain)
                    })),
            );
            if item["type"] == "video" {
                let mut actions = div().flex().flex_wrap().gap(m.space_xs).child(
                    ActionButton::new("media.thumbnail.capture", t.t("media.thumbnail.capture"))
                        .compact()
                        .variant(orbit_ui::controls::ButtonVariant::Soft)
                        .on_click(cx.listener(|this, _, _, cx| this.capture_thumbnail(cx))),
                );
                if item["thumbnailFile"].is_string() {
                    actions = actions.child(
                        ActionButton::new("media.thumbnail.reset", t.t("media.thumbnail.reset"))
                            .compact()
                            .variant(orbit_ui::controls::ButtonVariant::Ghost)
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.mutate("resetVideoThumbnail", vec![json!(this.selected)], cx);
                            })),
                    );
                }
                thumbnail = thumbnail.child(actions);
            }
            if !self.thumbnail_status.is_empty() {
                thumbnail = thumbnail.child(
                    div()
                        .id("thumbnail-status")
                        .role(gpui::Role::Status)
                        .child(self.thumbnail_status.clone()),
                );
            }
            body = body.child(row(
                t.t("media.inspector.thumbnail"),
                thumbnail.into_any_element(),
            ));
        }

        let detail = |value: Option<String>, applicable| {
            value.unwrap_or_else(|| {
                t.t(if applicable {
                    "common.unknown"
                } else {
                    "media.inspector.notApplicable"
                })
            })
        };
        let file_type = std::path::Path::new(&source)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_uppercase);
        let duration = metadata("durationSeconds")
            .as_f64()
            .map(crate::automation::form::format_time);
        let size = metadata("width")
            .as_f64()
            .zip(metadata("height").as_f64())
            .map(|(width, height)| format!("{width:.0} \u{00d7} {height:.0}"));
        let fps = metadata("fps").as_f64().map(|fps| format!("{fps:.2}"));
        let video = item["type"] == "video";
        let still = item["type"] == "image";
        let values = [
            ("media.inspector.type", detail(file_type, true)),
            ("media.inspector.duration", detail(duration, !still)),
            (
                "media.inspector.codec",
                detail(metadata("codec").as_str().map(str::to_owned), !still),
            ),
            ("media.inspector.size", detail(size, video || still)),
            ("media.inspector.fps", detail(fps, video)),
            (
                "media.inspector.format",
                detail(metadata("dynamicRange").as_str().map(str::to_owned), video),
            ),
        ];
        let metadata_rows =
            div()
                .flex()
                .flex_col()
                .children(values.into_iter().map(|(key, value)| {
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(m.space_xs)
                        .py(m.space_2xs)
                        .border_b_1()
                        .border_color(theme.colors.border_soft)
                        .child(t.t(key))
                        .child(div().text_color(theme.colors.text_strong).child(value))
                }));
        body = body.child(row(
            t.t("media.inspector.details"),
            metadata_rows.into_any_element(),
        ));
        if let Some(details) = details.filter(|details| !details.error.is_empty()) {
            body = body.child(
                div()
                    .id("media-probe-error")
                    .role(gpui::Role::Alert)
                    .child(details.error.clone()),
            );
        }
        body.into_any_element()
    }

    fn transition_field(&mut self, item: &Value, cx: &mut Context<Self>) -> impl IntoElement {
        let id = text(item, "id");
        if self
            .inspector_transition
            .as_ref()
            .is_none_or(|picker| !picker.read(cx).is_for_media(id))
        {
            let weak = cx.weak_entity();
            let id = id.to_owned();
            let runtime = self.runtime.clone();
            let workspace = self.workspace.clone();
            self.inspector_transition = Some(cx.new(|cx| {
                crate::transitions::TransitionDefaults::for_media(
                    runtime,
                    id.clone(),
                    std::rc::Rc::new(move |transition, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            if this.selected != id || this.workspace != workspace {
                                return;
                            }
                            let encoded = match transition["effectId"].as_str() {
                                None => "none".to_owned(),
                                Some(orbit_host::native_transitions::CUT_ID) => "cut".to_owned(),
                                Some(id) => {
                                    let Some(seconds) = transition["durationSeconds"].as_f64()
                                    else {
                                        return;
                                    };
                                    format!("fade:{seconds}:{id}")
                                }
                            };
                            this.patch_asset("backgroundTransition", json!(encoded), cx);
                        });
                    }),
                    cx,
                )
            }));
        }
        if let Some(picker) = &self.inspector_transition {
            picker.update(cx, |picker, cx| {
                picker.sync_media(item["backgroundTransition"].as_str().unwrap_or("none"), cx)
            });
        }
        div().w_full().children(self.inspector_transition.clone())
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
        let metrics = theme.metrics;
        let viewport = window.viewport_size();
        let visual = matches!(item["type"].as_str(), Some("image" | "video"));
        let playable = matches!(item["type"].as_str(), Some("audio" | "video"));
        let width =
            (viewport.width * 0.94).min(metrics.control_height * if visual { 40. } else { 18. });
        let height = (viewport.height * 0.80).min(metrics.row_height * 23.);
        let settings_width = metrics.control_height * 11.;
        let split = visual && width >= settings_width * 2.;
        self.request_details(&item, cx);

        let weak = cx.weak_entity();
        let tabs = orbit_ui::controls::TabList::new(
            "media-inspector-tabs",
            t.t("media.inspector.tabs"),
            [
                (0, "media.inspector.info", "info-circle"),
                (1, "media.inspector.properties", "settings"),
                (5, "transition.picker.title", "transition-right"),
                (2, "media.effects.title", "icons"),
                (4, "media.audio", "music"),
                (3, "media.markers.tab", "media-marker-tag"),
            ]
            .into_iter()
            .filter(|(index, _, _)| inspector_tab_offered(&item, *index))
            .map(|(index, key, icon)| orbit_ui::controls::TabItem::new(index, t.t(key)).icon(icon))
            .collect(),
            tab,
        )
        .on_change(move |tab, _, cx| {
            let _ = weak.update(cx, |this, cx| {
                if this.inspector_tab != tab {
                    this.inspector_tab = tab;
                    this.routing_open = false;
                    this.marker_menu = None;
                    this.effect_menu = None;
                    this.effect_preset_dialog = None;
                    this.inspector_transition = None;
                    cx.notify();
                }
            });
        });
        let mut body = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(metrics.space_xs)
            .p(metrics.space_sm);
        if tab == 0 {
            body = body.child(self.inspector_info(&item, window, cx));
        }
        if tab == 1 && visual {
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
            for (key, label) in [
                ("mediaFlipX", "media.flipHorizontal"),
                ("mediaFlipY", "media.flipVertical"),
                ("mediaCrop", "media.crop"),
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
        if tab == 1 && visual {
            if item["mediaCrop"] == true {
                for (key, label) in [
                    ("mediaCropTop", "media.crop.top"),
                    ("mediaCropRight", "media.crop.right"),
                    ("mediaCropBottom", "media.crop.bottom"),
                    ("mediaCropLeft", "media.crop.left"),
                ] {
                    body = body.child(self.numeric(
                        key,
                        label,
                        item[key].as_f64().unwrap_or(0.),
                        (0., f64::MAX),
                        window,
                        cx,
                    ));
                }
            }
            let rotation = item["mediaRotation"].as_f64().unwrap_or(0.).to_string();
            let mut angles = vec![
                ("0", "media.rotation.0"),
                ("90", "media.rotation.90"),
                ("180", "media.rotation.180"),
                ("270", "media.rotation.270"),
            ];
            if !angles.iter().any(|(value, _)| *value == rotation) {
                angles.push((&rotation, &rotation));
            }
            body =
                body.child(self.choice("mediaRotation", "media.rotation", &rotation, &angles, cx));
        }
        if tab == 5 {
            body = body.child(self.transition_field(&item, cx));
        }
        if tab == 2 && (item["type"] == "image" || item["type"] == "video") {
            body = body.child(self.effects(&item, window, cx));
        }
        if tab == 4 {
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
            body = body
                .child(
                    div()
                        .flex_none()
                        .w_full()
                        .h(metrics.space_2xs)
                        .bg(theme.colors.border_strong)
                        .my(metrics.space_xs),
                )
                .child(self.choice(
                    "backgroundPlayback",
                    "media.playbackBehavior",
                    item["backgroundPlayback"].as_str().unwrap_or("stop"),
                    &[
                        ("stop", "media.playback.stop"),
                        ("loop", "media.playback.loop"),
                    ],
                    cx,
                ))
                .child(
                    CheckboxField::new(
                        "media-soft-loop",
                        t.t("media.transport.softLoopUnavailable"),
                        false,
                    )
                    .disabled(true),
                );
        }
        if tab == 1 && playable {
            if let Some(duration) = self.preview_duration() {
                let range = self.displayed_preview_trim(&item, duration);
                body = body
                    .child(self.property_time(
                        ("preview-in-point", "media.transport.inPoint"),
                        range.start,
                        window,
                        cx,
                    ))
                    .child(self.property_time(
                        ("preview-out-point", "media.transport.outPoint"),
                        range.end,
                        window,
                        cx,
                    ))
                    .child(
                        Field::new(
                            t.t("media.inspector.duration"),
                            property_clock(range.end - range.start),
                        )
                        .inline()
                        .label_width(metrics.space_lg * 7.),
                    );
            }
        }
        if tab == 1 && item["type"] == "video" {
            let transport = &item["mediaTransport"];
            let delay = transport["delaySeconds"].as_f64().unwrap_or(0.);
            let rate = transport["rate"].as_f64().unwrap_or(1.);
            body = body
                .child(self.property_time(
                    ("mediaTransport.delaySeconds", "media.transport.delayTime"),
                    delay,
                    window,
                    cx,
                ))
                .child(self.playback_rate(rate, cx))
                .child(
                    self.choice(
                        "mediaHardwareDecoding",
                        "media.transport.hardwareDecoding",
                        item["mediaHardwareDecoding"]
                            .as_str()
                            .unwrap_or("automatic"),
                        &[
                            ("automatic", "common.automatic"),
                            ("disabled", "media.transport.hardwareDisabled"),
                        ],
                        cx,
                    ),
                );
        }
        if tab == 3 && (item["type"] == "audio" || item["type"] == "video") {
            body = body.child(self.markers(&item, window, cx));
        }
        let preview = (visual || playable).then(|| {
            let mut pane = div()
                .flex()
                .flex_col()
                .min_w_0()
                .min_h_0()
                .gap(metrics.space_xs)
                .p(metrics.space_sm)
                .bg(theme.colors.panel_bg)
                .when(split, |pane| {
                    pane.flex_1()
                        .border_r_1()
                        .border_color(theme.colors.border_soft)
                })
                .when(!split, |pane| {
                    pane.flex_none()
                        .border_b_1()
                        .border_color(theme.colors.border_soft)
                });
            if visual {
                self.request_effect_preview(&item, cx);
                #[cfg(test)]
                let current_frame = self.rendered_media_preview.as_ref().is_some_and(|frame| {
                    frame.key == self.effect_preview_key
                        && frame.key[2].as_f64() == Some(self.preview_frame_time())
                });
                pane = pane.child(
                    div()
                        .id("media-inspector-preview")
                        .debug_selector(|| "media-inspector-preview".into())
                        .map(|surface| {
                            #[cfg(test)]
                            return surface.children(current_frame.then(|| {
                                div()
                                    .absolute()
                                    .inset_0()
                                    .debug_selector(|| "media-inspector-current-frame".into())
                            }));
                            #[cfg(not(test))]
                            surface
                        })
                        .relative()
                        .min_h_0()
                        .overflow_hidden()
                        .rounded(metrics.radius_sm)
                        .bg(theme.colors.preview_matte_bg)
                        .when(split, |surface| surface.flex_1())
                        .when(!split, |surface| surface.h(height * 0.26))
                        .child(self.effect_preview.clone())
                        .children(
                            (item["type"] == "video"
                                && self
                                    .details
                                    .get(&self.selected)
                                    .is_some_and(|details| details.metadata["hasAudio"] == true))
                            .then(|| {
                                let audio = crate::settings::audio_logic::get_audio_settings(
                                    &self.runtime.output.read(cx).settings,
                                );
                                let bars = audio["inspector"]["routing"]["deviceChannels"]
                                    .as_u64()
                                    .unwrap_or(2)
                                    as usize;
                                let levels = self
                                    .preview
                                    .try_lock()
                                    .ok()
                                    .filter(|_| self.preview_playing)
                                    .map(|preview| preview.channel_levels())
                                    .unwrap_or_default();
                                crate::output::meters::overlay(
                                    &levels,
                                    bars,
                                    0.,
                                    "inspector-audio-meters",
                                    cx,
                                )
                            }),
                        ),
                );
            }
            if playable {
                pane = pane.child(self.preview_controls(&item, cx));
            }
            pane
        });
        let settings = div()
            .id(("media-inspector-scroll", tab))
            .debug_selector(|| "media-inspector-scroll".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .when(split, |pane| pane.flex_none().w(settings_width))
            .bg(theme.colors.panel_bg_strong)
            .overflow_y_scroll()
            .child(body);
        let weak = cx.weak_entity();
        let inspector = orbit_ui::controls::Dialog::new(
            "media-asset-inspector",
            inspector_title(&item),
            t.t("media.inspector.close"),
            move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.inspector_open = false;
                    this.marker_menu = None;
                    this.marker_editor = None;
                    this.effect_menu = None;
                    this.effect_preset_dialog = None;
                    this.routing_open = false;
                    this.inspector_transition = None;
                    this.preview_trim_drag = None;
                    this.preview_marker_drag = None;
                    this.effect_preview_key = Value::Null;
                    this.audition(false, cx);
                    cx.notify();
                });
            },
        )
        .width(width)
        .compact_header()
        .close_on_enter(false)
        .flush()
        .action(
            div()
                .id("media-inspector-tab-bar")
                .debug_selector(|| "media-inspector-tab-bar".into())
                .flex()
                .when(split, |strip| {
                    strip.w(settings_width
                        - metrics.panel_padding
                        - metrics.space_sm
                        - metrics.space_xs
                        - metrics.control_height_compact)
                })
                .child(tabs),
        )
        .child(
            div()
                .flex()
                .h(height)
                .when(!split, |pane| pane.flex_col())
                .children(preview)
                .child(settings),
        );
        div()
            .child(inspector)
            .children(self.effect_menu(&item, cx))
            .children(self.effect_preset_dialog(cx))
            .into_any_element()
    }
}
pub(super) fn inspector_tab_offered(item: &Value, tab: usize) -> bool {
    match tab {
        2 | 5 => matches!(item["type"].as_str(), Some("image" | "video")),
        3 | 4 => matches!(item["type"].as_str(), Some("audio" | "video")),
        _ => tab < 2,
    }
}

fn property_clock(seconds: f64) -> String {
    let millis = (seconds * 1000.).round() as u64;
    format!(
        "{}.{:03}",
        crate::automation::form::format_time((millis / 1000) as f64),
        millis % 1000
    )
}

fn parse_property_time(text: &str) -> Option<f64> {
    let seconds = if text.contains(':') {
        let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
        if !fraction.bytes().all(|digit| digit.is_ascii_digit()) {
            return None;
        }
        let seconds = crate::automation::form::parse_time(whole)? as f64;
        seconds
            + if fraction.is_empty() {
                0.
            } else {
                format!("0.{fraction}").parse::<f64>().ok()?
            }
    } else {
        text.parse::<f64>().ok()?
    };
    (seconds.is_finite() && seconds >= 0.).then_some(seconds)
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
