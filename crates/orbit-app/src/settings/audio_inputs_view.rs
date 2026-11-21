//! `AudioInputCaptureFields.tsx`: the stereo or matrix routing of an audio input, under its Routing row.

use gpui::{div, prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant, CheckboxField, SelectOption};
use orbit_ui::Translator;
use serde_json::{json, Value};

use super::audio_routing as routing;
use super::rows::{select_row, setting_row};
use super::ui::{action_row, stack};
use super::view::SettingsView;

impl SettingsView {
    pub(super) fn audio_input_ndi_refresh(&mut self, cx: &mut Context<Self>) {
        self.audio_inputs.ndi_loading = true;
        cx.notify();
        self.host_call("listNdiSources", Vec::new(), cx, |view, result, cx| {
            view.audio_inputs.ndi_loading = false;
            match result {
                Ok(list) => {
                    view.audio_inputs.ndi_sources = list.as_array().cloned().unwrap_or_default()
                }
                Err(error) => view.audio_inputs.error = Some(error),
            }
            cx.notify();
        });
    }

    pub(super) fn audio_input_routing_fields(
        &mut self,
        draft: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let weak = cx.weak_entity();
        let id = draft["id"].as_str().unwrap_or("").to_string();
        let capture = draft["capture"].clone();
        let matrix = !capture["routing"].is_null();
        let mode_weak = weak.clone();
        let mut body = stack(cx);
        body = body.child(select_row(
            "audio-input-routing-mode",
            t.t("audioInputs.routingMode"),
            vec![
                SelectOption::new("stereo".to_string(), t.t("audioInputs.routingMode.stereo")),
                SelectOption::new("matrix".to_string(), t.t("audioInputs.routingMode.matrix")),
            ],
            if matrix { "matrix" } else { "stereo" }.to_string(),
            move |mode, _, cx| {
                mode_weak
                    .update(cx, |v, cx| {
                        v.audio_input_change(
                            |d| {
                                d["capture"] =
                                    routing::select_routing_mode(&d["capture"], mode == "matrix")
                            },
                            cx,
                        )
                    })
                    .ok();
            },
            cx,
        ));
        if matrix {
            let table = capture["routing"].clone();
            for (key, label) in [
                ("sourceChannels", "audioInputs.sourceChannels"),
                ("busChannels", "audioInputs.busChannels"),
            ] {
                let (max, current) = (
                    if key == "sourceChannels" { 64. } else { 16. },
                    table[key].as_f64().unwrap_or(0.).to_string(),
                );
                body = body.child(self.number_field(
                    &format!("audio-input-{key}-{id}"),
                    t.t(label),
                    &current,
                    (1., max),
                    1.,
                    window,
                    cx,
                    move |v, text, cx| {
                        if let Ok(n) = text.trim().parse::<f64>() {
                            v.audio_input_change(
                                |d| {
                                    d["capture"]["routing"] =
                                        routing::resize_routing(&d["capture"]["routing"], key, n)
                                },
                                cx,
                            )
                        }
                    },
                ));
            }
            body = body.child(
                action_row(cx)
                    .child(
                        ActionButton::new("audio-input-auto", t.t("audioInputs.auto"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.audio_input_change(
                                    |d| {
                                        let r = &d["capture"]["routing"];
                                        d["capture"]["routing"] = routing::identity_routing(
                                            r["sourceChannels"].as_u64().unwrap_or(0),
                                            r["busChannels"].as_u64().unwrap_or(0),
                                        );
                                    },
                                    cx,
                                )
                            })),
                    )
                    .child(
                        ActionButton::new("audio-input-clear-routes", t.t("audioInputs.clear"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.audio_input_change(
                                    |d| d["capture"]["routing"]["routes"] = json!([]),
                                    cx,
                                )
                            })),
                    ),
            );
            for route in table["routes"].as_array().cloned().unwrap_or_default() {
                let (input, bus) = (
                    route["input"].as_u64().unwrap_or(0),
                    route["bus"].as_u64().unwrap_or(0),
                );
                let (mute_weak, remove_weak, level_weak) =
                    (weak.clone(), weak.clone(), weak.clone());
                let _ = level_weak;
                body = body.child(
                    action_row(cx)
                        .child(format!(
                            "{} {input} {} {bus}",
                            t.t("audioInputs.sourceChannel"),
                            t.t("audioInputs.to")
                        ))
                        .child(
                            CheckboxField::new(
                                SharedString::from(format!("audio-route-mute-{id}-{input}-{bus}")),
                                t.t("audioInputs.routeMute"),
                                route["muted"] == json!(true),
                            )
                            .on_change(move |on, _, cx| {
                                mute_weak
                                    .update(cx, |v, cx| {
                                        v.audio_input_change(
                                            |d| {
                                                d["capture"]["routing"] = routing::update_route(
                                                    &d["capture"]["routing"],
                                                    input,
                                                    bus,
                                                    json!({"muted": on}),
                                                )
                                            },
                                            cx,
                                        )
                                    })
                                    .ok();
                            }),
                        )
                        .child(
                            ActionButton::new(
                                SharedString::from(format!(
                                    "audio-route-remove-{id}-{input}-{bus}"
                                )),
                                t.t("capture.audioMap.remove"),
                            )
                            .variant(ButtonVariant::Danger)
                            .on_click(cx.listener(
                                move |v, _, _, cx| {
                                    let _ = &remove_weak;
                                    v.audio_input_change(
                                        |d| {
                                            d["capture"]["routing"] = routing::toggle_route(
                                                &d["capture"]["routing"],
                                                input,
                                                bus,
                                            )
                                        },
                                        cx,
                                    )
                                },
                            )),
                        ),
                );
                body = body.child(self.number_field(
                    &format!("audio-route-level-{id}-{input}-{bus}"),
                    t.t("audioInputs.routeLevel"),
                    &route["levelDb"].as_f64().unwrap_or(0.).to_string(),
                    (-60., 6.),
                    1.,
                    window,
                    cx,
                    move |v, text, cx| {
                        if let Ok(n) = text.trim().parse::<f64>() {
                            if n.is_finite() {
                                v.audio_input_change(
                                    |d| {
                                        d["capture"]["routing"] = routing::update_route(
                                            &d["capture"]["routing"],
                                            input,
                                            bus,
                                            json!({"levelDb": n}),
                                        )
                                    },
                                    cx,
                                )
                            }
                        }
                    },
                ));
            }
            let (source, bus) = self.audio_inputs.new_route;
            for (index, label, current) in [
                (0usize, "audioInputs.sourceChannel", source),
                (1, "audioInputs.bus", bus),
            ] {
                body = body.child(self.number_field(
                    &format!("audio-new-route-{index}-{id}"),
                    t.t(label),
                    &current.max(1).to_string(),
                    (1., 64.),
                    1.,
                    window,
                    cx,
                    move |v, text, cx| {
                        if let Ok(n) = text.trim().parse::<u64>() {
                            if index == 0 {
                                v.audio_inputs.new_route.0 = n;
                            } else {
                                v.audio_inputs.new_route.1 = n;
                            }
                            cx.notify();
                        }
                    },
                ));
            }
            body = body.child(
                ActionButton::new("audio-input-add-route", t.t("audioInputs.route"))
                    .variant(ButtonVariant::Secondary)
                    .on_click(cx.listener(|v, _, _, cx| {
                        let (input, bus) = (
                            v.audio_inputs.new_route.0.max(1),
                            v.audio_inputs.new_route.1.max(1),
                        );
                        v.audio_input_change(
                            |d| {
                                let exists = d["capture"]["routing"]["routes"]
                                    .as_array()
                                    .is_some_and(|r| {
                                        r.iter().any(|x| x["input"] == input && x["bus"] == bus)
                                    });
                                if !exists {
                                    d["capture"]["routing"] =
                                        routing::toggle_route(&d["capture"]["routing"], input, bus);
                                }
                            },
                            cx,
                        )
                    })),
            );
        }
        if !matrix {
            for (index, label) in [(0usize, "audioInputs.left"), (1, "audioInputs.right")] {
                let current = capture["channels"][index].as_u64().unwrap_or(1).to_string();
                let control = self.number_control(
                    &format!("audio-input-channel-{index}-{id}"),
                    &current,
                    (1., 64.),
                    1.,
                    window,
                    cx,
                    move |v, text, cx| {
                        if let Ok(n) = text.trim().parse::<u64>() {
                            v.audio_input_change(|d| d["capture"]["channels"][index] = json!(n), cx)
                        }
                    },
                );
                body = body.child(setting_row(
                    t.t(label),
                    None,
                    div().w(gpui::px(110.)).child(control),
                    cx,
                ));
            }
        }
        let latency = self.number_control(
            &format!("audio-input-latencyMs-{id}"),
            &capture["latencyMs"].as_f64().unwrap_or(0.).to_string(),
            (10., 200.),
            1.,
            window,
            cx,
            move |v, text, cx| {
                if let Ok(n) = text.trim().parse::<f64>() {
                    if n.is_finite() && n.fract() == 0. {
                        v.audio_input_change(|d| d["capture"]["latencyMs"] = json!(n), cx)
                    }
                }
            },
        );
        body = body.child(setting_row(
            t.t("audioInputs.latency"),
            None,
            div().w(gpui::px(110.)).child(latency),
            cx,
        ));
        body.into_any_element()
    }
}
