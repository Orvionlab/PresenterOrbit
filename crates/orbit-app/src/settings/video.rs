//! `VideoSettingsPanel.tsx`, `ScreenSettingsSidebar.tsx`, `VideoScreenOutputCards.tsx`,
//! `VideoOutputDetails.tsx`, `VideoOutputSettingsBodies.tsx`, `screen-output-actions.ts`, and
//! `screen-output-utils.ts`. Every edit is one `mutateScreenConfig` intent; the output process applies the
//! routing from the scene's `screenConfig`, so no output call is made here.

use super::ui::Field;
use gpui::{div, prelude::*, AnyElement, Context, Entity, SharedString, Subscription, Window};
use orbit_ui::controls::{
    input_state, ActionButton, ButtonVariant, InputEvent, InputState, Select, SelectOption,
    TextInput, ToggleSwitch,
};
use orbit_ui::{icon, ActiveTheme, Translator};
use serde_json::{json, Map, Value};

use super::ui::{action_row, stack};
use super::view::SettingsView;
use crate::output::windows::mutate_config;

/// `DEFAULT_SCREEN_LOOK_LAYERS` in the order of `SCREEN_LOOK_ROWS`.
pub const LOOK_LAYERS: [&str; 7] = [
    "music",
    "messages",
    "props",
    "announcements",
    "slide",
    "media",
    "videoInput",
];

/// `OUTPUT_FORMAT_OPTIONS`.
pub const OUTPUT_FORMATS: [(&str, &str); 3] = [
    ("sdr", "SDR"),
    ("hlg", "HDR - Rec. 2100 HLG"),
    ("pq", "HDR - Rec. 2100 PQ"),
];

pub const ALPHA_MODES: [&str; 3] = ["none", "straight", "premultiplied"];

/// `getScreenLookLayers`: the saved layers over all-visible defaults.
pub fn look_layers(config: &Value) -> Map<String, Value> {
    let mut layers: Map<String, Value> = LOOK_LAYERS
        .iter()
        .map(|id| (id.to_string(), json!(true)))
        .collect();
    if let Some(saved) = config["lookLayers"].as_object() {
        layers.extend(saved.clone());
    }
    layers
}

/// `getScreenGroupEntries`: the screens of one group in saved order.
pub fn group_entries(configs: &Value, group: &str) -> Vec<(String, Value)> {
    configs
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, config)| config["type"] == group)
        .map(|(id, config)| (id.clone(), config.clone()))
        .collect()
}

/// `updateScreenOutput`: the patch React sends. Changing `enabled` quotes the saved look layers with it, so a
/// screen that is switched on or off keeps what it showed.
pub fn update_mutation(screen_id: &str, current: &Value, patch: Value) -> Value {
    let mut patch = patch.as_object().cloned().unwrap_or_default();
    if patch.contains_key("enabled")
        && !current["lookLayers"].is_null()
        && !patch.contains_key("lookLayers")
    {
        patch.insert("lookLayers".into(), current["lookLayers"].clone());
    }
    json!({"type": "update", "screenId": screen_id, "patch": patch})
}

/// `updateSelectedOutputTarget`: `ndi` or `display:<id>`. An NDI target keeps its name or takes the default.
pub fn target_patch(value: &str, screen_id: &str, current: &Value) -> Option<Value> {
    if value == "ndi" {
        let name = current["ndiName"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map_or_else(|| format!("Presenter Orbit {screen_id}"), str::to_string);
        Some(json!({"targetType": "ndi", "ndiName": name}))
    } else {
        value
            .strip_prefix("display:")
            .map(|id| json!({"targetType": "display", "targetId": id}))
    }
}

pub struct Rename {
    pub(super) id: String,
    pub(super) input: Entity<InputState>,
    _subscription: Subscription,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum VideoSection {
    #[default]
    Outputs,
    ScreenOptions,
    Looks,
}

#[derive(Default)]
pub struct Video {
    pub(super) target_menu: Option<super::video_targets::TargetMenu>,
    pub(super) target_format: Option<super::video_targets::TargetFormat>,
    /// The sub-section shown; the default is the screen outputs.
    pub section: VideoSection,
    pub selected_screen: String,
    pub selected_output: usize,
    pub selected_blend: Option<usize>,
    pub output_tab: String,
    pub layout_error: Option<String>,
    pub calibration_sliders:
        std::collections::HashMap<String, Entity<orbit_ui::controls::SliderState>>,
    pub renaming: Option<Rename>,
    /// The screen whose delete awaits confirmation.
    pub confirm_delete: Option<String>,
    /// Screens whose settings row is open; transient UI state the host never sees.
    pub expanded: std::collections::HashSet<String>,
    /// The output picture of each enabled screen.
    pub previews: std::collections::HashMap<String, Entity<crate::output::preview::Preview>>,
}

impl SettingsView {
    /// The screens as the output sees them: the workspace map when it has screens, else the application map.
    pub(super) fn screen_configs(&self, cx: &gpui::App) -> Value {
        self.settings(cx)["screenConfig"].clone()
    }

    pub(super) fn update_screen(&mut self, screen_id: &str, patch: Value, cx: &mut Context<Self>) {
        let configs = self.screen_configs(cx);
        let Some(current) = configs.get(screen_id) else {
            return;
        };
        let mutation = update_mutation(screen_id, current, patch);
        mutate_config(&self.runtime, mutation, cx);
    }

    pub(super) fn toggle_expanded(&mut self, screen_id: &str, cx: &mut Context<Self>) {
        if !self.video.expanded.remove(screen_id) {
            self.video.expanded.insert(screen_id.to_string());
        }
        cx.notify();
    }

    pub(super) fn add_screen(&mut self, group: &'static str, cx: &mut Context<Self>) {
        let t = *cx.global::<Translator>();
        let base = t.t(if group == "stage" {
            "screenSettings.stage"
        } else {
            "screenSettings.audience"
        });
        mutate_config(
            &self.runtime,
            json!({"type": "add", "screenType": group, "baseLabel": base}),
            cx,
        );
    }

    pub(super) fn toggle_group(&mut self, group: &'static str, cx: &mut Context<Self>) {
        let enabled = !group_entries(&self.screen_configs(cx), group)
            .iter()
            .all(|(_, screen)| screen["enabled"] == true);
        mutate_config(
            &self.runtime,
            json!({"type": "setGroupEnabled", "screenType": group, "enabled": enabled}),
            cx,
        );
    }

    /// Deleting the last screen of a group needs no confirmation, as in React.
    pub(super) fn delete_screen(
        &mut self,
        screen_id: &str,
        confirmed: bool,
        cx: &mut Context<Self>,
    ) {
        let configs = self.screen_configs(cx);
        let group = configs[screen_id]["type"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if !confirmed && group_entries(&configs, &group).len() > 1 {
            self.video.confirm_delete = Some(screen_id.to_string());
            cx.notify();
            return;
        }
        self.video.confirm_delete = None;
        mutate_config(
            &self.runtime,
            json!({"type": "delete", "screenId": screen_id}),
            cx,
        );
    }

    pub(super) fn begin_screen_rename(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let configs = self.screen_configs(cx);
        let Some(label) = configs[id]["label"].as_str() else {
            return;
        };
        let input = input_state(label.to_string(), window, cx);
        let owner = id.to_string();
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                    && this
                        .video
                        .renaming
                        .as_ref()
                        .is_some_and(|rename| rename.id == owner)
                {
                    let label = input.read(cx).value().trim().to_string();
                    this.video.renaming = None;
                    if !label.is_empty() {
                        this.update_screen(&owner, json!({"label": label}), cx);
                    }
                    cx.notify();
                }
            },
        );
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        self.video.renaming = Some(Rename {
            id: id.to_string(),
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn screen_card(
        &mut self,
        id: &str,
        config: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let weak = cx.weak_entity();
        let label = config["label"].as_str().unwrap_or(id).to_string();
        let displays = self.runtime.output.read(cx).displays.clone();
        let ndi = config["targetType"] == "ndi";
        let selected = if ndi {
            "ndi".to_string()
        } else {
            format!("display:{}", config["targetId"].as_str().unwrap_or(""))
        };
        let mut targets: Vec<SelectOption<String>> = displays
            .as_array()
            .into_iter()
            .flatten()
            .map(|display| {
                let size = display["size"]
                    .as_object()
                    .map(|size| format!(" {} x {}", size["width"], size["height"]))
                    .unwrap_or_default();
                SelectOption::new(
                    format!("display:{}", display["id"].as_str().unwrap_or("")),
                    format!("{}{size}", display["label"].as_str().unwrap_or("")),
                )
            })
            .collect();
        targets.push(SelectOption::new(
            "ndi".to_string(),
            if ndi {
                config["ndiName"].as_str().unwrap_or("").to_string()
            } else {
                t.t("screenSettings.createNdi")
            },
        ));
        let title: AnyElement = match &self.video.renaming {
            Some(rename) if rename.id == id => TextInput::new(
                SharedString::from(format!("screen-rename-{id}")),
                &rename.input,
            )
            .into_any_element(),
            _ => {
                let rename_id = id.to_string();
                div()
                    .id(SharedString::from(format!("screen-name-{id}")))
                    .cursor_pointer()
                    .text_color(theme.colors.text_strong)
                    .child(label.clone())
                    .on_click(
                        cx.listener(move |view, event: &gpui::ClickEvent, window, cx| {
                            if event.click_count() >= 2 {
                                view.begin_screen_rename(&rename_id, window, cx);
                            }
                        }),
                    )
                    .into_any_element()
            }
        };
        let (enabled_id, target_id, delete_id) = (id.to_string(), id.to_string(), id.to_string());
        let (format_weak, alpha_weak, target_weak) = (weak.clone(), weak.clone(), weak.clone());
        let confirming = self.video.confirm_delete.as_deref() == Some(id);
        let enabled = config["enabled"] == json!(true);
        let type_label = t.t(if config["type"] == "stage" {
            "common.stage"
        } else {
            "common.audience"
        });
        let tag = |text: String| {
            div()
                .flex()
                .items_center()
                .min_h(gpui::px(20.))
                .px(gpui::px(7.))
                .border_1()
                .border_color(theme.colors.border)
                .rounded(theme.metrics.radius_md)
                .bg(theme.colors.panel_bg)
                .text_size(theme.metrics.font_xs)
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(theme.colors.text)
                .child(text)
        };
        let target_text = self.describe_target(config, cx);
        let header = div()
            .flex()
            .items_start()
            .gap(theme.metrics.space_md)
            .min_h(theme.metrics.header_height)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(theme.metrics.space_2xs)
                    .child(
                        div()
                            .text_size(theme.metrics.font_lg)
                            .font_weight(gpui::FontWeight::BOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(theme.metrics.space_snug)
                            .child(tag(
                                t.format("screenSettings.outputType", &[("type", &type_label)])
                            ))
                            .child(tag(target_text)),
                    ),
            )
            .child(
                ToggleSwitch::new(
                    SharedString::from(format!("screen-enabled-{id}")),
                    t.t("screenSettings.output"),
                    enabled,
                )
                .bare()
                .on_change(move |enabled, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.update_screen(&enabled_id, json!({"enabled": enabled}), cx)
                    })
                    .ok();
                }),
            );
        let delete_button = ActionButton::new(
            SharedString::from(format!("screen-delete-{id}")),
            if confirming {
                t.format("screenSettings.confirmDeleteOutput", &[("name", &label)])
            } else {
                t.t("common.delete")
            },
        )
        .variant(ButtonVariant::Danger)
        .on_click(cx.listener(move |view, _, _, cx| {
            let confirmed = view.video.confirm_delete.as_deref() == Some(delete_id.as_str());
            view.delete_screen(&delete_id, confirmed, cx)
        }));
        let mut card = stack(cx).child(Field::new(
            t.t("screenSettings.outputSource"),
            Select::new(
                SharedString::from(format!("screen-target-{id}")),
                targets,
                selected,
            )
            .on_change({
                let current = config.clone();
                move |value, _, cx| {
                    let Some(patch) = target_patch(&value, &target_id, &current) else {
                        return;
                    };
                    let screen = target_id.clone();
                    target_weak
                        .update(cx, |view, cx| view.update_screen(&screen, patch, cx))
                        .ok();
                }
            }),
        ));
        if ndi {
            let ndi_name = config["ndiName"].as_str().unwrap_or("").to_string();
            let name_id = id.to_string();
            card = card.child(self.text_field(
                &format!("screen-ndi-name-{id}"),
                t.t("media.inspector.name"),
                &ndi_name,
                window,
                cx,
                move |view, name, cx| {
                    if !name.trim().is_empty() {
                        view.update_screen(&name_id, json!({"ndiName": name.trim()}), cx);
                    }
                },
            ));
            let current_alpha = config["ndiAlphaMode"]
                .as_str()
                .unwrap_or("none")
                .to_string();
            let alpha_id = id.to_string();
            card = card.child(Field::new(
                t.t("screenSettings.alphaKey"),
                Select::new(
                    SharedString::from(format!("screen-alpha-{id}")),
                    ALPHA_MODES
                        .iter()
                        .map(|mode| SelectOption::new(mode.to_string(), mode.to_string()))
                        .collect(),
                    current_alpha,
                )
                .on_change(move |mode, _, cx| {
                    alpha_weak
                        .update(cx, |view, cx| {
                            view.update_screen(&alpha_id, json!({"ndiAlphaMode": mode}), cx)
                        })
                        .ok();
                }),
            ));
        } else {
            let format_id = id.to_string();
            card = card.child(Field::new(
                t.t("media.inspector.format"),
                Select::new(
                    SharedString::from(format!("screen-format-{id}")),
                    OUTPUT_FORMATS
                        .iter()
                        .map(|(value, label)| {
                            SelectOption::new(value.to_string(), label.to_string())
                        })
                        .collect(),
                    config["outputFormat"].as_str().unwrap_or("sdr").to_string(),
                )
                .on_change(move |format, _, cx| {
                    format_weak
                        .update(cx, |view, cx| {
                            view.update_screen(&format_id, json!({"outputFormat": format}), cx)
                        })
                        .ok();
                }),
            ));
        }
        let delay_id = id.to_string();
        card = card.child(self.number_field(
            &format!("screen-delay-{id}"),
            t.t("screenSettings.videoDelay"),
            &config["videoDelay"].as_f64().unwrap_or(0.).to_string(),
            (0., 5000.),
            1.,
            window,
            cx,
            move |view, text, cx| {
                if let Some(delay) = text.trim().parse::<f64>().ok().filter(|v| v.is_finite()) {
                    view.update_screen(&delay_id, json!({"videoDelay": delay.max(0.)}), cx);
                }
            },
        ));
        if !ndi {
            card = card.child(self.display_map(id, config, cx));
        }
        let extra = self.video_extra_fields(id, config, window, cx);
        let composition = self.composition_fields(id, config, window, cx);
        let details = card
            .child(extra)
            .child(composition)
            .child(action_row(cx).child(delete_button));
        let expanded = self.video.expanded.contains(id);
        let summary = self.describe_summary(id, config, cx);
        let preview = self.screen_preview(id, config, enabled, window, cx);
        let expand_id = id.to_string();
        let expand_weak = cx.weak_entity();
        div()
            .id(SharedString::from(format!("screen-{id}")))
            .flex()
            .flex_col()
            .gap(theme.metrics.space_md)
            .p(theme.metrics.panel_padding)
            .border_b_1()
            .border_color(theme.colors.border)
            .when(!enabled, |this| this.opacity(0.9))
            .child(header)
            .child(preview)
            .child(
                div()
                    .id(SharedString::from(format!("screen-settings-{id}")))
                    .flex()
                    .items_center()
                    .gap(theme.metrics.space_sm)
                    .min_h(theme.metrics.control_height)
                    .px(theme.metrics.field_padding_x)
                    .py(theme.metrics.space_xs)
                    .bg(theme.colors.border)
                    .text_size(theme.metrics.font_sm)
                    .text_color(theme.colors.text_strong)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        expand_weak
                            .update(cx, |view, cx| view.toggle_expanded(&expand_id, cx))
                            .ok();
                    })
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(theme.metrics.space_snug)
                            .text_color(theme.colors.text)
                            .child(
                                icon("settings")
                                    .size(theme.metrics.font_md)
                                    .text_color(theme.colors.text),
                            )
                            .child(t.t("settings.title")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(summary),
                    )
                    .child(
                        icon(if expanded {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        })
                        .size(theme.metrics.font_md)
                        .text_color(theme.colors.text_strong),
                    ),
            )
            .when(expanded, |this| this.child(details))
            .into_any_element()
    }

    /// `describeScreenTarget`.
    pub(super) fn describe_target(&self, config: &Value, cx: &gpui::App) -> String {
        let t = *cx.global::<Translator>();
        if config["enabled"] != json!(true) {
            return t.t("common.off");
        }
        if config["targetType"] == "ndi" {
            return config["ndiName"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map_or_else(|| t.t("screenSettings.newNdi"), str::to_string);
        }
        let displays = self.runtime.output.read(cx).displays.clone();
        let list = displays.as_array().cloned().unwrap_or_default();
        let wanted = config["targetId"].as_str().filter(|id| !id.is_empty());
        let display = list
            .iter()
            .find(|display| Some(display["id"].as_str().unwrap_or("")) == wanted)
            .or_else(|| {
                wanted
                    .is_none()
                    .then(|| {
                        list.iter()
                            .find(|display| display["primary"] == json!(true))
                            .or(list.first())
                    })
                    .flatten()
            });
        match display {
            Some(display) => format!(
                "{} {} x {}",
                display["label"].as_str().unwrap_or(""),
                display["size"]["width"],
                display["size"]["height"]
            ),
            None => t.t("screenSettings.noDisplaySelected"),
        }
    }

    /// `describeOutputSettingsSummary`.
    fn describe_summary(&self, id: &str, config: &Value, cx: &gpui::App) -> String {
        let t = *cx.global::<Translator>();
        let displays = self.runtime.output.read(cx).displays.clone();
        let short = if config["targetType"] == "ndi" {
            config["ndiName"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map_or_else(|| t.t("screenSettings.newNdi"), str::to_string)
        } else {
            let wanted = config["targetId"].as_str().unwrap_or("");
            displays
                .as_array()
                .into_iter()
                .flatten()
                .find(|display| display["id"] == wanted)
                .or_else(|| {
                    displays
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|display| display["primary"] == json!(true))
                })
                .and_then(|display| display["label"].as_str().map(str::to_string))
                .unwrap_or_else(|| t.t("screenSettings.display"))
        };
        let tail = if config["type"] == "stage" {
            let workspace = &self.runtime.content.read(cx).workspace;
            super::video_extra::effective_stage_layout(config, workspace, id)
                .and_then(|layout| layout["title"].as_str())
                .filter(|title| !title.is_empty())
                .map_or_else(|| t.t("stage.noLayout"), str::to_string)
        } else {
            return short;
        };
        format!("{short} \u{b7} {tail}")
    }

    /// One row of the sidebar: the switch, the name, and what it is doing.
    pub(super) fn sidebar_item(
        id: String,
        name: String,
        small: String,
        enabled: bool,
        group: bool,
        toggle: ToggleSwitch,
        cx: &gpui::App,
    ) -> AnyElement {
        let theme = *cx.theme();
        let fill = if enabled {
            theme.colors.control_bg_hover
        } else if group {
            theme.colors.panel_bg_soft
        } else {
            theme.colors.panel_bg
        };
        div()
            .id(SharedString::from(format!("screen-sidebar-{id}")))
            .flex()
            .items_center()
            .gap(theme.metrics.space_sm)
            .min_h(theme.metrics.control_height + theme.metrics.space_sm)
            .px(theme.metrics.space_sm)
            .py(theme.metrics.space_xs)
            .border_1()
            .border_color(if enabled { theme.colors.border } else { fill })
            .bg(fill)
            .child(
                div()
                    .flex_none()
                    .w(gpui::px(40.))
                    .debug_selector(move || format!("screen-switch-{id}"))
                    .child(toggle.bare()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(theme.metrics.font_md)
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(theme.colors.text_strong)
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(theme.metrics.font_sm)
                            .text_color(theme.colors.text)
                            .child(small),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn video_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.output_configuration_panel(window, cx)
    }
}
