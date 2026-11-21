//! `InputsSettingsPanelBridge.tsx`, `AudioInputsPanelBridge.tsx`, `AudioInputFields.tsx`, `InputList.tsx`, and
//! `NdiDiscoveryPage.tsx`: the Inputs tab, with video and audio inputs behind a toggle, and the NDI discovery
//! subpage. Edits save at once when the input is valid. The video side is the media bin's input state in rows
//! (`media::video_inputs_settings`).

use gpui::{div, prelude::*, px, AnyElement, Context, Entity, SharedString, Subscription, Window};
use orbit_ui::controls::{
    range_state, ActionButton, ButtonVariant, CheckboxField, RangeInput, SegmentedControl,
    SegmentedOption, SelectOption, SliderEvent, SliderState,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::audio_inputs::MODES;
use super::audio_routing as routing;
use super::rows::{
    column, group, link_row, select_row, setting_row, subpage_header, toggle_row, InputList,
};
use super::ui::heading;
use super::view::SettingsView;
use crate::media::VideoInputs;

pub const NDI_PAGE: &str = "ndiDiscovery";
const NDI: &str = "__ndi__";

pub struct InputTab {
    side: &'static str,
    video: Option<Entity<VideoInputs>>,
    routing_open: bool,
    /// The level slider of the selected audio input.
    level: Option<(String, Entity<SliderState>, Subscription)>,
    /// What is wrong with the extra hosts that were typed, `settings.ndiDiscovery.extraIpsInvalid`.
    pub ndi_error: Option<String>,
}

impl Default for InputTab {
    fn default() -> Self {
        Self {
            side: "video",
            video: None,
            routing_open: false,
            level: None,
            ndi_error: None,
        }
    }
}

impl InputTab {
    pub fn audio_shown(&self) -> bool {
        self.side == "audio"
    }
}

impl SettingsView {
    fn select_side(&mut self, side: &'static str, cx: &mut Context<Self>) {
        self.input_tab.side = side;
        cx.notify();
    }

    fn video_inputs_entity(&mut self, cx: &mut Context<Self>) -> Entity<VideoInputs> {
        let runtime = self.runtime.clone();
        let entity = self
            .input_tab
            .video
            .get_or_insert_with(|| cx.new(|cx| VideoInputs::for_settings(runtime, cx)))
            .clone();
        entity.update(cx, |video, cx| video.show(cx));
        entity
    }

    fn audio_level_state(&mut self, draft: &Value, cx: &mut Context<Self>) -> Entity<SliderState> {
        let id = draft["id"].as_str().unwrap_or("").to_string();
        if let Some((owner, state, _)) = &self.input_tab.level {
            if *owner == id {
                return state.clone();
            }
        }
        let level = draft["capture"]["levelDb"]
            .as_f64()
            .unwrap_or(0.)
            .clamp(-60., 6.);
        let state = range_state(-60., 6., 1., level as f32, cx);
        let owner = id.clone();
        let subscription = cx.subscribe(&state, move |this, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(value) = event else {
                return;
            };
            let level = f64::from(value.end().round());
            if this
                .audio_inputs
                .draft
                .as_ref()
                .is_some_and(|draft| draft["id"] == owner.as_str())
            {
                this.audio_input_change(|d| d["capture"]["levelDb"] = json!(level), cx);
            }
        });
        self.input_tab.level = Some((id, state.clone(), subscription));
        state
    }

    fn audio_source_rows(&mut self, draft: &Value, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = *cx.global::<Translator>();
        let weak = cx.weak_entity();
        let capture = draft["capture"].clone();
        let device = capture["deviceName"].as_str().unwrap_or("").to_string();
        let ndi = !capture["ndiSource"].is_null();
        let mut options = vec![SelectOption::new(
            String::new(),
            t.t("inputs.audio.noSource"),
        )];
        if !device.is_empty()
            && !self
                .audio_inputs
                .devices
                .iter()
                .any(|(name, _)| *name == device)
        {
            options.push(SelectOption::new(device.clone(), device.clone()));
        }
        options.extend(self.audio_inputs.devices.iter().map(|(name, channels)| {
            SelectOption::new(name.clone(), format!("{name} ({channels})"))
        }));
        options.push(SelectOption::new(NDI.to_string(), t.t("inputs.audio.ndi")));
        let mut rows = vec![select_row(
            "audio-input-source",
            t.t("ndi.source"),
            options,
            if ndi { NDI.to_string() } else { device },
            move |value, _, cx| {
                weak.update(cx, |view, cx| {
                    let to_ndi = value == NDI;
                    view.audio_input_change(
                        |d| {
                            let mut next = routing::select_source_type(&d["capture"], to_ndi);
                            if !to_ndi {
                                next["deviceName"] = json!(value);
                            }
                            d["capture"] = next;
                        },
                        cx,
                    );
                    if to_ndi {
                        view.audio_input_ndi_refresh(cx);
                    }
                })
                .ok();
            },
            cx,
        )];
        if ndi {
            rows.extend(self.audio_ndi_rows(&capture, cx));
        }
        rows
    }

    /// `AudioInputNdiSourceBridge`.
    fn audio_ndi_rows(&mut self, capture: &Value, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = *cx.global::<Translator>();
        let found = self.audio_inputs.ndi_sources.clone();
        let current = Some(&capture["ndiSource"])
            .filter(|source| source["name"].as_str().is_some_and(|name| !name.is_empty()));
        let options = crate::media::video_inputs::ndi_source_options(current, &found);
        let by_key: Vec<(String, Value)> = options
            .iter()
            .map(|source| (source.to_string(), source.clone()))
            .collect();
        let mut choices =
            vec![SelectOption::new(String::new(), t.t("ndi.chooseSource")).disabled(true)];
        choices.extend(options.iter().map(|source| {
            SelectOption::new(
                source.to_string(),
                source["name"].as_str().unwrap_or("").to_string(),
            )
        }));
        let weak = cx.weak_entity();
        let loading = self.audio_inputs.ndi_loading;
        let status = if loading {
            Some(t.t("ndi.discovering"))
        } else if found.is_empty() {
            Some(t.t("ndi.noSources"))
        } else {
            None
        };
        vec![
            select_row(
                "audio-input-ndi",
                t.t("ndi.source"),
                choices,
                current.map(Value::to_string).unwrap_or_default(),
                move |key, _, cx| {
                    let Some((_, source)) = by_key.iter().find(|(k, _)| *k == key) else {
                        return;
                    };
                    let source = source.clone();
                    weak.update(cx, |view, cx| {
                        view.audio_input_change(|d| d["capture"]["ndiSource"] = source, cx)
                    })
                    .ok();
                },
                cx,
            ),
            setting_row(
                t.t("ndi.refresh"),
                status.map(Into::into),
                ActionButton::new("audio-input-ndi-refresh", t.t("ndi.refresh"))
                    .variant(ButtonVariant::Secondary)
                    .disabled(loading)
                    .on_click(cx.listener(|v, _, _, cx| v.audio_input_ndi_refresh(cx))),
                cx,
            ),
        ]
    }

    fn audio_status_rows(&self, draft: &Value, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let suffix = format!("\0{}", draft["id"].as_str().unwrap_or(""));
        self.audio_inputs
            .status
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| key.ends_with(&suffix))
            .map(|(_, status)| {
                let peak = status["peak"].as_f64().unwrap_or(0.).clamp(0., 1.) as f32;
                setting_row(
                    t.t("inputs.audio.signal"),
                    status["error"]
                        .as_str()
                        .map(|error| error.to_string().into()),
                    div()
                        .flex()
                        .items_center()
                        .gap(theme.metrics.space_sm)
                        .child(t.t(if status["running"] == json!(true) {
                            "audioInputs.running"
                        } else {
                            "audioInputs.waiting"
                        }))
                        .child(
                            div()
                                .w(px(80.))
                                .h(theme.metrics.space_sm)
                                .rounded(theme.metrics.radius_xs)
                                .bg(theme.colors.field_bg)
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(peak))
                                        .rounded(theme.metrics.radius_xs)
                                        .bg(theme.colors.accent),
                                ),
                        ),
                    cx,
                )
            })
            .collect()
    }

    fn audio_fields(
        &mut self,
        draft: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let id = draft["id"].as_str().unwrap_or("").to_string();
        let capture = draft["capture"].clone();
        let mode = draft["behaviorMode"].as_str().unwrap_or("off").to_string();
        let name = self.text_control(
            &format!("audio-input-name-{id}"),
            draft["title"].as_str().unwrap_or(""),
            window,
            cx,
            |view, name, cx| view.audio_input_change(|d| d["title"] = json!(name), cx),
        );
        let mut rows = vec![setting_row(
            t.t("settings.devices.name"),
            None,
            div().w(px(420.)).child(name),
            cx,
        )];
        rows.extend(self.audio_source_rows(draft, cx));
        rows.extend(self.audio_status_rows(draft, cx));
        let weak = cx.weak_entity();
        rows.push(setting_row(
            t.t("audioInputs.mode"),
            Some(t.t(&format!("inputs.audio.mode.hint.{mode}")).into()),
            SegmentedControl::new(
                "audio-input-mode",
                t.t("audioInputs.mode"),
                MODES
                    .iter()
                    .map(|value| {
                        SegmentedOption::text(
                            value.to_string(),
                            t.t(&format!("audioInputs.mode.{value}")),
                        )
                    })
                    .collect(),
                mode.clone(),
            )
            .on_change(move |value, _, cx| {
                weak.update(cx, |view, cx| {
                    view.audio_input_change(|d| d["behaviorMode"] = json!(value), cx)
                })
                .ok();
            }),
            cx,
        ));
        if mode.starts_with("auto") {
            let links = draft["linkedVideoInputIds"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let videos: Vec<(String, String)> = self.runtime.content.read(cx).workspace
                ["videoInputs"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| {
                    (
                        v["id"].as_str().unwrap_or("").to_string(),
                        v["title"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect();
            let boxes = videos.into_iter().map(|(video, title)| {
                let linked = links.iter().any(|x| *x == video.as_str());
                let weak = cx.weak_entity();
                CheckboxField::new(
                    SharedString::from(format!("audio-link-{id}-{video}")),
                    title,
                    linked,
                )
                .on_change(move |on, _, cx| {
                    let video = video.clone();
                    weak.update(cx, |v, cx| {
                        v.audio_input_change(
                            |d| {
                                let mut list = d["linkedVideoInputIds"]
                                    .as_array()
                                    .cloned()
                                    .unwrap_or_default();
                                list.retain(|x| *x != video.as_str());
                                if on {
                                    list.push(json!(video));
                                }
                                d["linkedVideoInputIds"] = Value::Array(list);
                            },
                            cx,
                        )
                    })
                    .ok();
                })
            });
            rows.push(setting_row(
                t.t("audioInputs.links"),
                None,
                div()
                    .flex()
                    .flex_col()
                    .gap(theme.metrics.space_xs)
                    .children(boxes),
                cx,
            ));
        }
        let delay = self.number_control(
            &format!("audio-input-delayMs-{id}"),
            &capture["delayMs"].as_f64().unwrap_or(0.).to_string(),
            (0., 2000.),
            1.,
            window,
            cx,
            |view, text, cx| {
                if let Ok(n) = text.trim().parse::<f64>() {
                    if n.is_finite() && n.fract() == 0. {
                        view.audio_input_change(|d| d["capture"]["delayMs"] = json!(n), cx)
                    }
                }
            },
        );
        rows.push(setting_row(
            t.t("settings.audio.delay"),
            None,
            div()
                .flex()
                .items_center()
                .gap(theme.metrics.space_sm)
                .child(div().w(px(110.)).child(delay))
                .child(div().child("ms")),
            cx,
        ));
        let level = self.audio_level_state(draft, cx);
        let shown = level.read(cx).value().end().round();
        rows.push(setting_row(
            t.t("settings.audio.level"),
            None,
            div()
                .flex()
                .items_center()
                .gap(theme.metrics.space_sm)
                .child(
                    div()
                        .w(px(190.))
                        .child(RangeInput::new(&level).label(t.t("audioInputs.level"))),
                )
                .child(
                    div()
                        .w(px(64.))
                        .flex()
                        .justify_end()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(format!("{shown}dB")),
                ),
            cx,
        ));
        let open = self.input_tab.routing_open;
        rows.push(setting_row(
            t.t("inputs.audio.channels"),
            None,
            ActionButton::new("audio-input-routing", t.t("settings.audio.routing"))
                .variant(ButtonVariant::Secondary)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.input_tab.routing_open = !open;
                    cx.notify();
                })),
            cx,
        ));
        let routing_fields = open.then(|| self.audio_input_routing_fields(draft, window, cx));
        column(cx)
            .child(group(rows, cx))
            .children(routing_fields)
            .into_any_element()
    }

    fn audio_side(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.poll_audio_inputs(cx);
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let available = !self.runtime.content.read(cx).workspace_path.is_empty();
        let saved = self.runtime.content.read(cx).workspace["audioInputs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let draft = self.audio_inputs.draft.clone();
        let mut items: Vec<(String, String)> = saved
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
            if !saved.iter().any(|input| input["id"] == draft["id"]) {
                items.push((
                    draft["id"].as_str().unwrap_or("").to_string(),
                    draft["title"].as_str().unwrap_or("").to_string(),
                ));
            }
        }
        let (select_weak, add_weak, remove_weak) =
            (cx.weak_entity(), cx.weak_entity(), cx.weak_entity());
        let list = InputList {
            id: "audio-inputs-list",
            items,
            selected: draft
                .as_ref()
                .and_then(|draft| draft["id"].as_str())
                .unwrap_or("")
                .to_string(),
            empty: t.t("inputs.audio.empty").into(),
            add: t.t("inputs.add").into(),
            remove: t.t("inputs.remove").into(),
            disabled: !available,
        }
        .render(
            move |id, _, cx| {
                select_weak
                    .update(cx, |view, cx| {
                        view.audio_input_select(&id, cx);
                        view.input_tab.routing_open = false;
                        if view
                            .audio_inputs
                            .draft
                            .as_ref()
                            .is_some_and(|d| !d["capture"]["ndiSource"].is_null())
                        {
                            view.audio_input_ndi_refresh(cx);
                        }
                    })
                    .ok();
            },
            move |_, cx| {
                add_weak
                    .update(cx, |view, cx| {
                        view.input_tab.routing_open = false;
                        view.audio_input_add(cx);
                    })
                    .ok();
            },
            move |_, cx| {
                remove_weak
                    .update(cx, |view, cx| view.audio_input_remove_selected(cx))
                    .ok();
            },
            cx,
        );
        let mut body = div()
            .flex()
            .flex_col()
            .gap(theme.metrics.control_gap)
            .child(list);
        if let Some(draft) = draft.filter(|draft| draft["capture"].is_object()) {
            body = body.child(self.audio_fields(&draft, window, cx));
            if let Some(error) = super::audio_inputs::input_error(&draft) {
                body = body.child(
                    div()
                        .id("audio-input-invalid")
                        .text_size(theme.metrics.font_sm)
                        .text_color(theme.colors.text_muted)
                        .child(t.t(error)),
                );
            }
        }
        body.children(self.audio_inputs.error.clone().map(|error| {
            div()
                .id("audio-input-error")
                .text_size(theme.metrics.font_sm)
                .text_color(theme.colors.warning)
                .child(if error.starts_with("audioInputs.") {
                    t.t(&error)
                } else {
                    error
                })
        }))
        .into_any_element()
    }

    pub(super) fn inputs_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.page == Some(NDI_PAGE) {
            return self.ndi_discovery_page(window, cx);
        }
        let t = *cx.global::<Translator>();
        let weak = cx.weak_entity();
        let video = self.input_tab.side == "video";
        let body = if video {
            self.video_inputs_entity(cx).into_any_element()
        } else {
            self.audio_side(window, cx)
        };
        let open = cx.weak_entity();
        column(cx)
            .id("settings-inputs")
            .child(heading(t.t("settings.nav.inputs"), cx))
            .child(
                div().flex().child(
                    SegmentedControl::new(
                        "inputs-type",
                        t.t("inputs.type"),
                        vec![
                            SegmentedOption::text("video", t.t("videoInputs.title")),
                            SegmentedOption::text("audio", t.t("audioInputs.title")),
                        ],
                        if video { "video" } else { "audio" },
                    )
                    .on_change(move |side, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.select_side(if side == "audio" { "audio" } else { "video" }, cx)
                        })
                        .ok();
                    }),
                ),
            )
            .child(body)
            .child(group(
                [link_row(
                    "ndi-discovery-link",
                    None,
                    t.t("settings.ndiDiscovery.title"),
                    move |_, cx| {
                        open.update(cx, |view, cx| view.open_page(NDI_PAGE, cx))
                            .ok();
                    },
                    cx,
                )],
                cx,
            ))
            .into_any_element()
    }

    /// `commitGroups`: the entries trimmed and joined; unchanged groups are not saved.
    pub(super) fn commit_ndi_groups(&mut self, value: &str, cx: &mut Context<Self>) {
        let Some(next) = orbit_host::ndi_find_options::normalize_groups(value) else {
            return;
        };
        if next
            != self.settings(cx)["generalSettings"]["ndiGroups"]
                .as_str()
                .unwrap_or("")
        {
            self.patch_general(json!({ "ndiGroups": next }), cx);
        }
    }

    /// `commitExtraIps`: addresses and host names only; anything else is reported and not saved.
    pub(super) fn commit_ndi_extra_ips(&mut self, value: &str, cx: &mut Context<Self>) {
        let Some(next) = orbit_host::ndi_find_options::normalize_extra_ips(value) else {
            self.input_tab.ndi_error = Some("settings.ndiDiscovery.extraIpsInvalid".into());
            cx.notify();
            return;
        };
        self.input_tab.ndi_error = None;
        if next
            != self.settings(cx)["generalSettings"]["ndiExtraIps"]
                .as_str()
                .unwrap_or("")
        {
            self.patch_general(json!({ "ndiExtraIps": next }), cx);
        }
        cx.notify();
    }

    /// `NdiDiscoveryPage`: the finder options of NDI source discovery.
    fn ndi_discovery_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let general = self.settings(cx)["generalSettings"].clone();
        let text = |key: &str| general[key].as_str().unwrap_or("").to_string();
        let groups = self.text_control(
            "ndi-discovery-groups",
            &text("ndiGroups"),
            window,
            cx,
            |view, value, cx| view.commit_ndi_groups(&value, cx),
        );
        let extra_ips = self.text_control(
            "ndi-discovery-extra-ips",
            &text("ndiExtraIps"),
            window,
            cx,
            |view, value, cx| view.commit_ndi_extra_ips(&value, cx),
        );
        let weak = cx.weak_entity();
        let back = cx.weak_entity();
        let rows = group(
            [
                toggle_row(
                    "ndi-discovery-local",
                    t.t("settings.ndiDiscovery.showLocal"),
                    Some(t.t("settings.ndiDiscovery.showLocalDescription").into()),
                    general["ndiShowLocalSources"].as_bool().unwrap_or(true),
                    move |value, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.patch_general(json!({ "ndiShowLocalSources": value }), cx)
                        })
                        .ok();
                    },
                    cx,
                ),
                setting_row(
                    t.t("settings.ndiDiscovery.groups"),
                    Some(t.t("settings.ndiDiscovery.groupsDescription").into()),
                    div().w(px(420.)).child(groups),
                    cx,
                ),
                setting_row(
                    t.t("settings.ndiDiscovery.extraIps"),
                    Some(t.t("settings.ndiDiscovery.extraIpsDescription").into()),
                    div().w(px(420.)).child(extra_ips),
                    cx,
                ),
            ],
            cx,
        );
        column(cx)
            .id("settings-ndi-discovery")
            .child(subpage_header(
                "ndi-discovery-back",
                t.t("settings.ndiDiscovery.back"),
                t.t("settings.ndiDiscovery.title"),
                move |_, cx| {
                    back.update(cx, |view, cx| {
                        view.input_tab.ndi_error = None;
                        view.close_page(cx)
                    })
                    .ok();
                },
                cx,
            ))
            .child(rows)
            .children(self.input_tab.ndi_error.clone().map(|key| {
                div()
                    .id("ndi-discovery-error")
                    .text_color(theme.colors.warning)
                    .child(t.t(&key))
            }))
            .into_any_element()
    }
}
