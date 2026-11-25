use super::*;
use orbit_ui::controls::{
    range_state, ButtonVariant, CheckboxField, Field, RangeInput, SliderEvent, SliderState,
};

pub(super) fn asset_route(item: &Value) -> Value {
    item.get("audioRouting").cloned().unwrap_or_else(||json!({"gainDb":0,"channels":[{"sourceChannel":1,"outputs":[1],"muted":false},{"sourceChannel":2,"outputs":[2],"muted":false}]}))
}

pub(super) struct GainSlider {
    slider: Entity<SliderState>,
    stored_gain: f64,
    _subscription: Subscription,
}

impl MediaBin {
    pub(super) fn route_field(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        let Some(item) = self.media.iter().find(|item| item["id"] == self.selected) else {
            return;
        };
        let mut route = asset_route(item);
        let (target, field) = if key == "route:gainDb" {
            (&mut route, "gainDb")
        } else {
            let Some((index, field)) = key
                .strip_prefix("route:")
                .and_then(|key| key.split_once(':'))
            else {
                return;
            };
            let Some(channel) = index
                .parse::<usize>()
                .ok()
                .and_then(|index| route["channels"].get_mut(index))
            else {
                return;
            };
            (channel, field)
        };
        if target.get(field) == Some(&value)
            || (field == "gainDb" && value.as_f64() == Some(target[field].as_f64().unwrap_or(0.)))
        {
            return;
        }
        target[field] = value;
        self.patch_asset("audioRouting", route, cx);
    }

    fn routing_gain(
        &mut self,
        key: &str,
        label: String,
        gain: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let m = cx.theme().metrics;
        let disabled = self.edits_blocked();
        let numeric = self
            .numeric_input(key, gain, (-60., 12.), window, cx)
            .label(label.clone())
            .suffix("dB");
        let entry = self
            .routing_sliders
            .entry(key.to_owned())
            .or_insert_with(|| {
                let slider = range_state(-60., 12., 0.5, gain as f32, cx);
                let key = key.to_owned();
                let subscription = cx.subscribe_in(
                    &slider,
                    window,
                    move |this, _, event: &SliderEvent, window, cx| match event {
                        SliderEvent::Change(value) => {
                            if let Some((input, _)) = this.inputs.get(&key) {
                                input.update(cx, |state, cx| {
                                    state.set_value(value.end().to_string(), window, cx)
                                });
                            }
                        }
                        SliderEvent::Release(value) => {
                            this.route_field(&key, json!(value.end()), cx)
                        }
                    },
                );
                GainSlider {
                    slider,
                    stored_gain: gain,
                    _subscription: subscription,
                }
            });
        if entry.stored_gain != gain {
            entry.stored_gain = gain;
            entry
                .slider
                .update(cx, |slider, cx| slider.set_value(gain as f32, window, cx));
            if let Some((input, _)) = self
                .inputs
                .get(key)
                .filter(|(input, _)| !input.read(cx).focus_handle(cx).is_focused(window))
            {
                input.update(cx, |state, cx| {
                    state.set_value(gain.to_string(), window, cx)
                });
            }
        }
        let selector = format!("routing-slider-{key}");
        Field::new(
            label.clone(),
            div()
                .flex()
                .items_center()
                .gap(m.space_xs)
                .child(
                    div()
                        .debug_selector(move || selector.clone())
                        .flex_1()
                        .min_w_0()
                        .child(
                            RangeInput::new(&entry.slider)
                                .label(label)
                                .disabled(disabled),
                        ),
                )
                .child(div().w(m.control_height * 2.4).flex_none().child(numeric)),
        )
        .inline()
        .label_width(m.space_lg * 5.)
    }

    pub(super) fn routing(
        &mut self,
        item: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let m = theme.metrics;
        let route = asset_route(item);
        let mut body = div()
            .flex()
            .flex_col()
            .gap(m.space_xs)
            .child(self.routing_gain(
                "route:gainDb",
                t.t("media.audioRouting.masterVolume"),
                route["gainDb"].as_f64().unwrap_or(0.),
                window,
                cx,
            ))
            .child(
                div()
                    .border_t_1()
                    .border_color(theme.colors.border_soft)
                    .pt(m.space_xs)
                    .child(
                        Field::new(
                            t.t("media.audioRouting.channels"),
                            ActionButton::new(
                                "route-configure",
                                t.t(if self.routing_open {
                                    "common.done"
                                } else {
                                    "media.audioRouting.configure"
                                }),
                            )
                            .compact()
                            .variant(ButtonVariant::Soft)
                            .selected(self.routing_open)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.routing_open = !this.routing_open;
                                cx.notify();
                            })),
                        )
                        .inline()
                        .label_width(m.space_lg * 5.),
                    ),
            );
        let Some(channels) = route["channels"].as_array() else {
            return body.into_any_element();
        };
        for (index, channel) in channels.iter().enumerate() {
            let mut actions = div().flex().gap(m.space_xs);
            for (flag, label) in [
                ("muted", "media.audioRouting.mute"),
                ("solo", "theme.media.solo"),
            ] {
                let selected = channel[flag] == true;
                actions = actions.child(
                    ActionButton::new(
                        SharedString::from(format!("route-{index}-{flag}")),
                        t.t(label),
                    )
                    .compact()
                    .variant(ButtonVariant::Soft)
                    .selected(selected)
                    .disabled(self.edits_blocked())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.route_field(&format!("route:{index}:{flag}"), json!(!selected), cx);
                    })),
                );
            }
            let mut section = div()
                .flex()
                .flex_col()
                .gap(m.space_xs)
                .mt(m.space_xs)
                .pt(m.space_xs)
                .border_t_1()
                .border_color(theme.colors.border_soft)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(m.space_xs)
                        .child(div().text_color(theme.colors.text_strong).child(format!(
                            "{} {}",
                            t.t("media.audioRouting.input"),
                            channel["sourceChannel"]
                        )))
                        .child(actions),
                )
                .child(self.routing_gain(
                    &format!("route:{index}:gainDb"),
                    t.t("theme.media.volume"),
                    channel["gainDb"].as_f64().unwrap_or(0.),
                    window,
                    cx,
                ));
            if self.routing_open {
                section =
                    section.child(self.routing_options(index, channel, channels.len() > 1, cx));
            } else {
                let outputs = channel["outputs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_u64)
                    .map(|output| format!("{} {output}", t.t("media.audioRouting.output")))
                    .collect::<Vec<_>>()
                    .join(", ");
                section = section.child(
                    Field::new(
                        t.t("media.audioRouting.outputs"),
                        div()
                            .text_size(m.font_sm)
                            .text_color(theme.colors.text_muted)
                            .child(if outputs.is_empty() {
                                t.t("common.none")
                            } else {
                                outputs
                            }),
                    )
                    .inline()
                    .label_width(m.space_lg * 5.),
                );
            }
            body = body.child(section);
        }
        if self.routing_open {
            if let Some(source) = (1..=8).find(|source| {
                !channels
                    .iter()
                    .any(|channel| channel["sourceChannel"] == *source)
            }) {
                body = body.child(
                    ActionButton::new("route-add", t.t("media.audioRouting.addChannel"))
                        .compact()
                        .disabled(self.edits_blocked())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.edit_route_channels(cx, |channels| {
                                if channels.len() < 8
                                    && !channels
                                        .iter()
                                        .any(|channel| channel["sourceChannel"] == source)
                                {
                                    channels.push(
                                        json!({"sourceChannel":source,"outputs":[],"muted":false}),
                                    );
                                    return true;
                                }
                                false
                            });
                        })),
                );
            }
        }
        body.into_any_element()
    }

    fn routing_options(
        &self,
        index: usize,
        channel: &Value,
        removable: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let m = cx.theme().metrics;
        let mut outputs = div().flex().flex_wrap().gap(m.control_gap);
        for output in [1, 2] {
            let checked = channel["outputs"]
                .as_array()
                .is_some_and(|outputs| outputs.contains(&json!(output)));
            let owner = cx.weak_entity();
            outputs = outputs.child(
                CheckboxField::new(
                    SharedString::from(format!("route-{index}-output-{output}")),
                    format!("{} {output}", t.t("media.audioRouting.output")),
                    checked,
                )
                .disabled(self.edits_blocked())
                .on_change(move |checked, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.edit_route_channels(cx, |channels| {
                            let Some(outputs) = channels
                                .get_mut(index)
                                .and_then(|channel| channel["outputs"].as_array_mut())
                            else {
                                return false;
                            };
                            outputs.retain(|value| *value != json!(output));
                            if checked {
                                outputs.push(json!(output));
                            }
                            outputs.sort_by_key(Value::as_u64);
                            true
                        });
                    });
                }),
            );
        }
        let owner = cx.weak_entity();
        div()
            .flex()
            .flex_col()
            .gap(m.space_xs)
            .child(outputs)
            .child(
                CheckboxField::new(
                    SharedString::from(format!("route-{index}-compressLimit")),
                    t.t("media.audioRouting.compressLimit"),
                    channel["compressLimit"] == true,
                )
                .disabled(self.edits_blocked())
                .on_change(move |checked, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.route_field(
                            &format!("route:{index}:compressLimit"),
                            json!(checked),
                            cx,
                        )
                    });
                }),
            )
            .when(removable, |section| {
                section.child(
                    ActionButton::new(
                        SharedString::from(format!("route-remove-{index}")),
                        t.t("media.audioRouting.removeChannel"),
                    )
                    .variant(ButtonVariant::Ghost)
                    .compact()
                    .disabled(self.edits_blocked())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_route_channels(cx, |channels| {
                            if channels.len() <= 1 || index >= channels.len() {
                                return false;
                            }
                            channels.remove(index);
                            true
                        });
                        this.inputs.retain(|key, _| !key.starts_with("route:"));
                        this.routing_sliders.clear();
                    })),
                )
            })
    }

    fn edit_route_channels(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Vec<Value>) -> bool,
    ) {
        let Some(item) = self.media.iter().find(|item| item["id"] == self.selected) else {
            return;
        };
        let mut route = asset_route(item);
        if route["channels"].as_array_mut().is_some_and(edit) {
            self.patch_asset("audioRouting", route, cx);
        }
    }
}
