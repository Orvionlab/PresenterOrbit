//! `AudioSettingsPanel.tsx` and `AudioRoutingPanel.tsx`. Edits go through `applyAudioSettings`; the output
//! process picks them up from the scene's `audioSettings`, so the routing reaches its sinks without a second call.

use gpui::{div, prelude::*, AnyElement, Context, Entity, SharedString, Window};
use orbit_ui::controls::{
    range_state, ActionButton, ButtonVariant, RangeInput, SelectOption, SliderEvent, SliderState,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::audio_logic::{self as logic, Device};
use super::persist;
use super::rows::{column, group as group_rows, select_row, setting_row, toggle_row};
use super::ui::{heading, stack};
use super::view::SettingsView;

#[derive(Default)]
pub struct Audio {
    /// The level sliders of `main` and `sdiNdi`.
    pub levels: std::collections::HashMap<&'static str, Entity<SliderState>>,
    /// `main`, `sdiNdi`, or empty: which routing grid is open.
    pub routing_open: String,
    pub devices: Vec<Device>,
    pub loaded: bool,
}

impl SettingsView {
    /// Lists the audio output devices once, off the UI thread. Opening a sink is the output process' work; the
    /// list only reads device names.
    #[cfg_attr(test, allow(dead_code))]
    pub(super) fn load_audio_devices(&mut self, cx: &mut Context<Self>) {
        if self.audio.loaded {
            return;
        }
        self.audio.loaded = true;
        self.host_call(
            "listAudioOutputDevices",
            Vec::new(),
            cx,
            |view, result, cx| match result {
                Ok(list) => {
                    view.audio.devices = list
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|device| {
                            Some((
                                device["id"].as_str()?.to_string(),
                                device["label"].as_str()?.to_string(),
                            ))
                        })
                        .collect();
                    cx.notify();
                }
                Err(error) => view.fail(error, cx),
            },
        );
    }

    fn audio_settings(&self, cx: &gpui::App) -> Value {
        logic::get_audio_settings(&self.settings(cx))
    }

    pub(super) fn apply_audio(&mut self, audio: Value, cx: &mut Context<Self>) {
        let task = persist::apply_audio(&self.runtime, cx, audio);
        self.report(task, cx);
    }

    pub(super) fn audio_update(
        &mut self,
        group: &str,
        field: &str,
        value: Value,
        cx: &mut Context<Self>,
    ) {
        let audio = logic::update_setting(
            &self.audio_settings(cx),
            group,
            field,
            value,
            &self.audio.devices,
        );
        self.apply_audio(audio, cx);
    }

    pub(super) fn audio_toggle_channel(
        &mut self,
        group: &str,
        channel: &str,
        device_channel: usize,
        cx: &mut Context<Self>,
    ) {
        let audio = logic::toggle_channel(&self.audio_settings(cx), group, channel, device_channel);
        self.apply_audio(audio, cx);
    }

    pub(super) fn audio_toggle_mode(
        &mut self,
        group: &str,
        channel: &str,
        mode: &str,
        cx: &mut Context<Self>,
    ) {
        let audio = logic::toggle_mode(&self.audio_settings(cx), group, channel, mode);
        self.apply_audio(audio, cx);
    }

    /// The routing row of a group; the grid opens under it, as `AudioRoutingPanel` does in the group.
    fn audio_routing_row(&mut self, group: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let open = self.audio.routing_open == group;
        setting_row(
            t.t("settings.audio.routing"),
            None,
            ActionButton::new(
                SharedString::from(format!("audio-{group}-routing")),
                t.t("settings.audio.routing"),
            )
            .variant(ButtonVariant::Secondary)
            .on_click(cx.listener(move |view, _, _, cx| {
                view.audio.routing_open = if open {
                    String::new()
                } else {
                    group.to_string()
                };
                cx.notify();
            })),
            cx,
        )
    }

    /// `AudioLevelRow`: the level slider and its `dB` output.
    fn audio_level_row(
        &mut self,
        group: &'static str,
        section: &Value,
        label: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let level = section["level"].as_f64().unwrap_or(0.).clamp(-60., 6.);
        let level_state = self
            .audio
            .levels
            .entry(group)
            .or_insert_with(|| {
                let state = range_state(-60., 6., 1., level as f32, cx);
                cx.subscribe(&state, move |this, _, event: &SliderEvent, cx| {
                    let SliderEvent::Change(value) = event else {
                        return;
                    };
                    this.audio_update(group, "level", json!(value.end().round() as f64), cx);
                })
                .detach();
                state
            })
            .clone();
        let shown = level_state.read(cx).value().end().round();
        setting_row(
            t.t("settings.audio.level"),
            None,
            div()
                .flex()
                .items_center()
                .gap(theme.metrics.space_sm)
                .child(
                    div()
                        .w(gpui::px(190.))
                        .child(RangeInput::new(&level_state).label(label)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .w(gpui::px(64.))
                        .min_h(theme.metrics.control_height)
                        .px(theme.metrics.space_snug)
                        .rounded(theme.metrics.radius_sm)
                        .border_1()
                        .border_color(theme.colors.control_border)
                        .bg(theme.colors.field_bg)
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(format!("{shown}dB")),
                ),
            cx,
        )
    }

    /// `AudioDelayRow`: a number field with `ms` beside it.
    fn audio_delay_row(
        &mut self,
        group: &'static str,
        section: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let delay = section["delay"].as_f64().unwrap_or(0.).max(0.);
        let control = self.number_control(
            &format!("audio-{group}-delay"),
            &delay.to_string(),
            (0., f64::MAX),
            1.,
            window,
            cx,
            move |view, text, cx| {
                if let Some(value) = text.trim().parse::<f64>().ok().filter(|v| v.is_finite()) {
                    view.audio_update(group, "delay", json!(value.max(0.)), cx);
                }
            },
        );
        setting_row(
            t.t("settings.audio.delay"),
            None,
            div()
                .flex()
                .items_center()
                .gap(theme.metrics.space_sm)
                .child(div().w(gpui::px(110.)).child(control))
                .child(div().child("ms")),
            cx,
        )
    }

    fn audio_device_row(
        &mut self,
        group: &'static str,
        section: &Value,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let translate = |key: &str| t.t(key);
        let mut options = logic::device_options(
            &self.audio.devices,
            section["device"].as_str(),
            section["deviceLabel"].as_str().unwrap_or(""),
            &translate,
        );
        if group == "inspector" {
            // The inspector always plays somewhere: no "none", and the default device is the system setting.
            options.retain(|(value, _)| value != "none");
            for option in options.iter_mut().filter(|(value, _)| value == "system") {
                option.1 = t.t("settings.audio.inspector.systemSetting");
            }
        }
        let weak = cx.weak_entity();
        select_row(
            SharedString::from(format!("audio-{group}-device")),
            t.t("settings.audio.device"),
            options
                .into_iter()
                .map(|(value, label)| SelectOption::new(value, label))
                .collect(),
            section["device"]
                .as_str()
                .unwrap_or(if group == "main" { "none" } else { "system" })
                .to_string(),
            move |value, _, cx| {
                weak.update(cx, |view, cx| {
                    view.audio_update(group, "device", json!(value), cx)
                })
                .ok();
            },
            cx,
        )
    }

    fn audio_group(
        &mut self,
        group: &'static str,
        audio: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let section = audio[group].clone();
        let mut rows = Vec::new();
        match group {
            "sdiNdi" => {
                let weak = cx.weak_entity();
                rows.push(toggle_row(
                    "audio-sdi-ndi-enabled",
                    t.t("settings.audio.enable"),
                    None,
                    section["enabled"] != json!(false),
                    move |enabled, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.audio_update("sdiNdi", "enabled", json!(enabled), cx)
                        })
                        .ok();
                    },
                    cx,
                ));
                rows.push(self.audio_routing_row(group, cx));
            }
            "inspector" => {
                rows.push(self.audio_device_row(group, &section, cx));
                rows.push(self.audio_level_row(
                    group,
                    &section,
                    t.t("settings.audio.inspector.level"),
                    cx,
                ));
                rows.push(self.audio_routing_row(group, cx));
            }
            _ => {
                rows.push(self.audio_device_row(group, &section, cx));
                rows.push(self.audio_routing_row(group, cx));
            }
        }
        if group != "inspector" {
            rows.push(self.audio_level_row(
                group,
                &section,
                format!("{} (dB)", t.t("settings.audio.level")),
                cx,
            ));
            rows.push(self.audio_delay_row(group, &section, window, cx));
        }
        let open = self.audio.routing_open == group;
        let grid = open.then(|| self.routing_grid(group, &section["routing"], window, cx));
        stack(cx)
            .child(heading(
                t.t(match group {
                    "inspector" => "settings.audio.inspector",
                    "main" => "settings.audio.main",
                    _ => "settings.audio.sdiNdi",
                }),
                cx,
            ))
            .child(group_rows(rows, cx))
            .children(grid)
            .into_any_element()
    }

    fn routing_grid(
        &mut self,
        group: &'static str,
        routing: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let device_channels = routing["deviceChannels"].as_u64().unwrap_or(2) as usize;
        let buses = routing["proChannels"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let (bus_routing, destination_routing) = (routing.clone(), routing.clone());
        let mut grid = stack(cx)
            .id(SharedString::from(format!("audio-{group}-routing-grid")))
            .child(div().child(t.t("settings.audio.deviceChannel")))
            .child(self.number_field(
                &format!("audio-{group}-buses"),
                t.t("settings.audio.internalBuses"),
                &buses.len().to_string(),
                (2., 16.),
                1.,
                window,
                cx,
                move |view, text, cx| {
                    if let Ok(count) = text.trim().parse::<f64>() {
                        let next = logic::resize_buses(&bus_routing, count);
                        view.audio_update(group, "routing", next, cx);
                    }
                },
            ))
            .child(self.number_field(
                &format!("audio-{group}-outputs"),
                t.t("settings.audio.outputChannels"),
                &device_channels.to_string(),
                (1., 16.),
                1.,
                window,
                cx,
                move |view, text, cx| {
                    if let Ok(count) = text.trim().parse::<f64>() {
                        let next = logic::resize_destinations(&destination_routing, count);
                        view.audio_update(group, "routing", next, cx);
                    }
                },
            ));
        for bus in &buses {
            let id = bus["id"].as_str().unwrap_or("").to_string();
            let number = id.rsplit('-').next().unwrap_or("");
            let mut row = div()
                .flex()
                .items_center()
                .gap(theme.metrics.control_gap)
                .child(
                    div()
                        .w(gpui::px(130.))
                        .child(format!("Orbit Channel {number}")),
                );
            for mode in ["mute", "solo", "tone"] {
                let (bus_id, on) = (id.clone(), bus["modes"][mode] == json!(true));
                row = row.child(
                    ActionButton::new(
                        SharedString::from(format!("audio-{group}-{id}-{mode}")),
                        mode[..1].to_uppercase(),
                    )
                    .variant(if on {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.audio_toggle_mode(group, &bus_id, mode, cx)
                    })),
                );
            }
            for channel in 1..=device_channels {
                let (bus_id, on) = (
                    id.clone(),
                    bus["channels"][channel.to_string()] == json!(true),
                );
                row = row.child(
                    ActionButton::new(
                        SharedString::from(format!("audio-{group}-{id}-cell-{channel}")),
                        channel.to_string(),
                    )
                    .variant(if on {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.audio_toggle_channel(group, &bus_id, channel, cx)
                    })),
                );
            }
            grid = grid.child(row);
        }
        grid.into_any_element()
    }

    pub(super) fn audio_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(not(test))]
        self.load_audio_devices(cx);
        let t = *cx.global::<Translator>();
        let audio = self.audio_settings(cx);
        let routing = audio["main"]["routing"].clone();
        let channels = self.number_control(
            "audio-channel-count",
            &routing["deviceChannels"].as_u64().unwrap_or(2).to_string(),
            (1., logic::MAX_OUTPUT_CHANNELS as f64),
            1.,
            window,
            cx,
            move |view, text, cx| {
                if let Ok(count) = text.trim().parse::<f64>() {
                    let next = logic::resize_destinations(&routing, count.round());
                    view.audio_update("main", "routing", next, cx);
                }
            },
        );
        let count = group_rows(
            [setting_row(
                t.t("settings.audio.channelCount"),
                None,
                div().w(gpui::px(110.)).child(channels),
                cx,
            )],
            cx,
        );
        let inspector = self.audio_group("inspector", &audio, window, cx);
        let main = self.audio_group("main", &audio, window, cx);
        let sdi = self.audio_group("sdiNdi", &audio, window, cx);
        column(cx)
            .id("settings-audio")
            .child(heading(t.t("settings.nav.audio"), cx))
            .child(count)
            .child(inspector)
            .child(main)
            .child(sdi)
            .into_any_element()
    }
}
