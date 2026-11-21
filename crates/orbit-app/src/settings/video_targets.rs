//! Output destinations, monitor topology previews, and virtual output format selection.
use gpui::{
    div, point, prelude::*, relative, AnyElement, Bounds, Context, Entity, Pixels, Point,
    SharedString, Window,
};
use orbit_host::output_layout::{DeviceTarget, OutputLayout};
use orbit_ui::controls::{
    input_state, ActionButton, ButtonVariant, ContextMenu, Dialog, Field, InputEvent, InputState,
    MenuEntry, MenuItem, NumberInput, Select, SelectOption, TextInput,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};
use std::{cell::Cell, rc::Rc};

use super::{video_extra::NDI_PRESETS, view::SettingsView};

#[derive(Clone)]
pub(super) struct TargetSelection {
    pub screen: String,
    pub index: usize,
    pub key: bool,
    pub device: String,
}

#[derive(Clone)]
pub(super) struct TargetMenu {
    pub selection: TargetSelection,
    pub position: Point<Pixels>,
}

#[derive(Clone, Copy)]
pub(super) enum TargetKind {
    Ndi,
    Placeholder,
}

impl TargetKind {
    fn title(self) -> &'static str {
        match self {
            Self::Ndi => "screenTargets.newNdi",
            Self::Placeholder => "screenTargets.newPlaceholder",
        }
    }
}

#[derive(Clone)]
pub(super) struct TargetFormat {
    selection: TargetSelection,
    kind: TargetKind,
    name: Entity<InputState>,
    width: Entity<InputState>,
    height: Entity<InputState>,
    rate: Entity<InputState>,
    error: Option<String>,
}

/// The preview uses physical desktop coordinates, including monitors left or above the primary display.
fn monitor_preview(displays: &[Value], highlighted: &str, cx: &gpui::App) -> AnyElement {
    let theme = *cx.theme();
    let bounds: Vec<_> = displays
        .iter()
        .filter_map(|display| {
            let b = &display["bounds"];
            Some((
                display,
                b["x"].as_f64()?,
                b["y"].as_f64()?,
                b["width"].as_f64()?,
                b["height"].as_f64()?,
            ))
        })
        .collect();
    let left = bounds
        .iter()
        .map(|(_, x, ..)| *x)
        .fold(f64::INFINITY, f64::min);
    let top = bounds
        .iter()
        .map(|(_, _, y, ..)| *y)
        .fold(f64::INFINITY, f64::min);
    let right = bounds
        .iter()
        .map(|(_, x, _, w, _)| x + w)
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = bounds
        .iter()
        .map(|(_, _, y, _, h)| y + h)
        .fold(f64::NEG_INFINITY, f64::max);
    let (width, height) = (right - left, bottom - top);
    let mut map = div()
        .relative()
        .w(theme.metrics.space_lg * 9.)
        .h(theme.metrics.space_lg * 4.)
        .flex_none()
        .bg(theme.colors.canvas_bg);
    if width > 0. && height > 0. {
        let ratio = width / height;
        let fitted_width = (ratio / 2.25).min(1.);
        let fitted_height = (2.25 / ratio).min(1.);
        for (index, (display, x, y, w, h)) in bounds.into_iter().enumerate() {
            let selected = display["id"] == highlighted;
            map = map.child(
                div()
                    .absolute()
                    .left(relative(
                        ((1. - fitted_width) / 2. + (x - left) / width * fitted_width) as f32,
                    ))
                    .top(relative(
                        ((1. - fitted_height) / 2. + (y - top) / height * fitted_height) as f32,
                    ))
                    .w(relative((w / width * fitted_width) as f32))
                    .h(relative((h / height * fitted_height) as f32))
                    .border_1()
                    .border_color(theme.colors.border_strong)
                    .bg(if selected {
                        theme.colors.selected
                    } else {
                        theme.colors.panel_bg_strong
                    })
                    .text_size(theme.metrics.font_xs)
                    .text_color(theme.colors.text)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child((index + 1).to_string()),
            );
        }
    }
    map.into_any_element()
}

impl SettingsView {
    pub(super) fn output_target_button(
        &self,
        selection: TargetSelection,
        target: &DeviceTarget,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let mut title = self.device_title(target, cx);
        if target.kind == "display" {
            if let Some(index) =
                self.runtime
                    .output
                    .read(cx)
                    .displays
                    .as_array()
                    .and_then(|displays| {
                        displays
                            .iter()
                            .position(|display| display["id"] == target.id)
                    })
            {
                title = format!("{}. {title}", index + 1);
            }
        }
        let id = format!(
            "output-target-{}-{}",
            selection.index,
            if selection.key { "key" } else { "fill" }
        );
        let bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
        let painted = bounds.clone();
        div()
            .relative()
            .debug_selector({
                let id = id.clone();
                move || id
            })
            .child(
                gpui::canvas(
                    move |bounds, _, _| painted.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                ActionButton::new(SharedString::from(id), title)
                    .variant(ButtonVariant::Secondary)
                    .compact()
                    .icon("chevron-down")
                    .on_click(cx.listener(move |view, event: &gpui::ClickEvent, _, cx| {
                        let position = bounds.get().map_or_else(
                            || event.position(),
                            |bounds| point(bounds.left(), bounds.bottom()),
                        );
                        view.video.target_menu = Some(TargetMenu {
                            selection: selection.clone(),
                            position,
                        });
                        cx.notify();
                    })),
            )
            .w_full()
            .text_size(theme.metrics.font_sm)
            .into_any_element()
    }

    fn target_layout(
        &self,
        selection: &TargetSelection,
        cx: &gpui::App,
    ) -> Result<OutputLayout, String> {
        let configs = self.screen_configs(cx);
        let config = configs
            .get(&selection.screen)
            .ok_or_else(|| cx.global::<Translator>().t("screenTargets.unavailable"))?;
        let layout = OutputLayout::from_screen(&selection.screen, config)?;
        if layout
            .outputs
            .get(selection.index)
            .is_none_or(|device| device.id != selection.device)
        {
            return Err(cx.global::<Translator>().t("screenTargets.unavailable"));
        }
        Ok(layout)
    }

    fn default_target_name(
        &self,
        selection: &TargetSelection,
        kind: TargetKind,
        cx: &gpui::App,
    ) -> String {
        match kind {
            TargetKind::Placeholder => cx.global::<Translator>().t("screenTargets.placeholder"),
            TargetKind::Ndi => format!(
                "Presenter Orbit {} {}{}",
                selection.screen,
                selection.index + 1,
                if selection.key { " Key" } else { "" }
            ),
        }
    }

    fn apply_target_format(
        &mut self,
        selection: &TargetSelection,
        kind: TargetKind,
        name: String,
        size: (usize, usize, f64),
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let mut layout = self.target_layout(selection, cx)?;
        let device = &mut layout.outputs[selection.index];
        let target = if selection.key {
            &mut device
                .alpha
                .as_mut()
                .ok_or_else(|| cx.global::<Translator>().t("screenTargets.unavailable"))?
                .target
        } else {
            &mut device.target
        };
        let name = name.trim();
        if name.is_empty() {
            return Err(cx.global::<Translator>().t("screenTargets.invalidName"));
        }
        target.kind = match kind {
            TargetKind::Ndi => "ndi",
            TargetKind::Placeholder => "placeholder",
        }
        .into();
        target.id.clear();
        target.name = name.into();
        target.source_type = None;
        target.format = "sdr".into();
        (target.width, target.height, target.frame_rate) = size;
        if !selection.key {
            let target = device.target.clone();
            if let Some(alpha) = &mut device.alpha {
                alpha.target.width = target.width;
                alpha.target.height = target.height;
                alpha.target.frame_rate = target.frame_rate;
                alpha.target.video_delay = target.video_delay;
            }
            if selection.index == 0 {
                layout.tile_width = target.width;
                layout.tile_height = target.height;
            }
        }
        layout.validate()?;
        self.video.layout_error = None;
        self.update_screen(&selection.screen, json!({"outputLayout":layout}), cx);
        Ok(())
    }

    pub(super) fn open_target_format(
        &mut self,
        selection: TargetSelection,
        kind: TargetKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self.target_layout(&selection, cx);
        let layout = match result {
            Ok(layout) => layout,
            Err(error) => {
                self.video.layout_error = Some(error);
                cx.notify();
                return;
            }
        };
        let device = &layout.outputs[selection.index];
        let target = if selection.key {
            device.alpha.as_ref().map(|alpha| &alpha.target)
        } else {
            Some(&device.target)
        };
        let Some(target) = target else {
            return;
        };
        let target_kind = match kind {
            TargetKind::Ndi => "ndi",
            TargetKind::Placeholder => "placeholder",
        };
        let name = if target.name.is_empty() || target.kind != target_kind {
            self.default_target_name(&selection, kind, cx)
        } else {
            target.name.clone()
        };
        let draft = TargetFormat {
            selection,
            kind,
            name: input_state(name, window, cx),
            width: input_state(target.width.to_string(), window, cx),
            height: input_state(target.height.to_string(), window, cx),
            rate: input_state(target.frame_rate.to_string(), window, cx),
            error: None,
        };
        for state in [&draft.name, &draft.width, &draft.height, &draft.rate] {
            cx.subscribe(state, |view, _, event: &InputEvent, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => view.commit_target_format(cx),
                _ => {}
            })
            .detach();
        }
        self.video.target_menu = None;
        self.video.target_format = Some(draft);
        cx.notify();
    }

    fn commit_target_format(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.video.target_format.clone() else {
            return;
        };
        let size = (
            draft.width.read(cx).value().trim().parse::<usize>(),
            draft.height.read(cx).value().trim().parse::<usize>(),
            draft.rate.read(cx).value().trim().parse::<f64>(),
        );
        let result = match size {
            (Ok(width), Ok(height), Ok(rate))
                if (16..=8192).contains(&width)
                    && (16..=8192).contains(&height)
                    && rate.is_finite()
                    && (1. ..=120.).contains(&rate) =>
            {
                self.apply_target_format(
                    &draft.selection,
                    draft.kind,
                    draft.name.read(cx).value().to_string(),
                    (width, height, rate),
                    cx,
                )
            }
            _ => Err(cx.global::<Translator>().t("screenTargets.invalidSize")),
        };
        match result {
            Ok(()) => self.video.target_format = None,
            Err(error) => {
                if let Some(draft) = &mut self.video.target_format {
                    draft.error = Some(error);
                }
            }
        }
        cx.notify();
    }

    fn format_entries(
        &self,
        selection: &TargetSelection,
        kind: TargetKind,
        cx: &Context<Self>,
    ) -> Vec<MenuEntry> {
        let mut entries: Vec<_> = NDI_PRESETS
            .iter()
            .map(|&(label, w, h, rate)| {
                let selection = selection.clone();
                let weak = cx.weak_entity();
                MenuItem::new(label)
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| {
                            let name = view.default_target_name(&selection, kind, cx);
                            if let Err(error) = view.apply_target_format(
                                &selection,
                                kind,
                                name,
                                (w as usize, h as usize, rate),
                                cx,
                            ) {
                                view.video.layout_error = Some(error);
                            }
                            cx.notify();
                        })
                        .ok();
                    })
                    .into()
            })
            .collect();
        entries.push(MenuEntry::Separator);
        let (weak, selection) = (cx.weak_entity(), selection.clone());
        entries.push(
            MenuItem::new(cx.global::<Translator>().t("screenTargets.customSize"))
                .on_click(move |_, window, cx| {
                    weak.update(cx, |view, cx| {
                        view.open_target_format(selection.clone(), kind, window, cx)
                    })
                    .ok();
                })
                .into(),
        );
        entries
    }

    fn target_menu(&self, menu: &TargetMenu, cx: &Context<Self>) -> ContextMenu {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let displays = self.runtime.output.read(cx).displays.clone();
        let current = self
            .target_layout(&menu.selection, cx)
            .ok()
            .and_then(|layout| {
                let device = &layout.outputs[menu.selection.index];
                if menu.selection.key {
                    device.alpha.as_ref().map(|alpha| alpha.target.clone())
                } else {
                    Some(device.target.clone())
                }
            });
        let mut entries = vec![MenuEntry::Header(t.t("screenTargets.system").into())];
        for (index, display) in displays.as_array().into_iter().flatten().enumerate() {
            let Some(id) = display["id"].as_str() else {
                continue;
            };
            let name = display["label"].as_str().unwrap_or(id);
            let (id, label) = (id.to_string(), format!("{}. {name}", index + 1));
            let content = div()
                .flex()
                .items_center()
                .gap(theme.metrics.space_sm)
                .p(theme.metrics.space_xs)
                .debug_selector({
                    let id = id.clone();
                    move || format!("output-monitor-{id}")
                })
                .child(monitor_preview(
                    displays.as_array().map_or(&[], Vec::as_slice),
                    &id,
                    cx,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(label.clone())
                        .child(div().text_size(theme.metrics.font_xs).child(format!(
                            "{} × {}",
                            display["size"]["width"], display["size"]["height"]
                        ))),
                );
            let (weak, selection) = (cx.weak_entity(), menu.selection.clone());
            let checked = current
                .as_ref()
                .is_some_and(|target| target.kind == "display" && target.id == id);
            entries.push(
                MenuItem::new(label)
                    .checked(checked)
                    .content(content)
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.layout_target(
                                &selection.screen,
                                selection.index,
                                selection.key,
                                &format!("display:{id}"),
                                cx,
                            )
                        })
                        .ok();
                    })
                    .into(),
            );
        }
        for (heading, kind) in [
            ("NDI", TargetKind::Ndi),
            ("screenTargets.placeholder", TargetKind::Placeholder),
        ] {
            entries.push(MenuEntry::Header(if heading == "NDI" {
                "NDI".into()
            } else {
                t.t(heading).into()
            }));
            let target_kind = match kind {
                TargetKind::Ndi => "ndi",
                TargetKind::Placeholder => "placeholder",
            };
            if let Some(target) = current
                .as_ref()
                .filter(|target| target.kind == target_kind && !target.name.is_empty())
            {
                entries.push(
                    MenuItem::new(target.name.clone())
                        .checked(true)
                        .meta(format!("{} × {}", target.width, target.height))
                        .into(),
                );
            }
            entries.push(MenuEntry::Submenu {
                label: t.t(kind.title()).into(),
                disabled: false,
                entries: self.format_entries(&menu.selection, kind, cx),
            });
        }
        entries.push(MenuEntry::Separator);
        let (weak, selection) = (cx.weak_entity(), menu.selection.clone());
        entries.push(
            MenuItem::new(t.t("screenLayout.unconfigured"))
                .checked(
                    current.as_ref().is_some_and(|target| {
                        target.kind == "placeholder" && target.name.is_empty()
                    }),
                )
                .on_click(move |_, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.layout_target(
                            &selection.screen,
                            selection.index,
                            selection.key,
                            "placeholder",
                            cx,
                        )
                    })
                    .ok();
                })
                .into(),
        );
        let weak = cx.weak_entity();
        ContextMenu::new(
            "output-destination-menu",
            menu.position,
            entries,
            move |_, _, cx| {
                weak.update(cx, |view, cx| {
                    view.video.target_menu = None;
                    cx.notify();
                })
                .ok();
            },
        )
        .width(theme.metrics.space_lg * 25.)
        .submenu_width(theme.metrics.space_lg * 12.)
    }

    fn target_format_dialog(&self, draft: &TargetFormat, cx: &mut Context<Self>) -> Dialog {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let weak = cx.weak_entity();
        let current = NDI_PRESETS
            .iter()
            .find(|(_, w, h, rate)| {
                draft.width.read(cx).value().as_ref() == w.to_string()
                    && draft.height.read(cx).value().as_ref() == h.to_string()
                    && draft
                        .rate
                        .read(cx)
                        .value()
                        .trim()
                        .parse::<f64>()
                        .is_ok_and(|value| (value - rate).abs() < 0.001)
            })
            .map_or("custom", |(name, ..)| *name)
            .to_string();
        let mut options: Vec<_> = NDI_PRESETS
            .iter()
            .map(|(name, ..)| SelectOption::new(name.to_string(), *name))
            .collect();
        options.push(SelectOption::new(
            "custom".to_string(),
            t.t("screenTargets.customSize"),
        ));
        let preset = Select::new("output-format-preset", options, current)
            .compact()
            .on_change({
                let weak = weak.clone();
                move |preset, window, cx| {
                    weak.update(cx, |view, cx| {
                        if let (Some(draft), Some((_, w, h, rate))) = (
                            &view.video.target_format,
                            NDI_PRESETS.iter().find(|(name, ..)| *name == preset),
                        ) {
                            for (state, value) in [
                                (&draft.width, w.to_string()),
                                (&draft.height, h.to_string()),
                                (&draft.rate, rate.to_string()),
                            ] {
                                state.update(cx, |state, cx| state.set_value(value, window, cx));
                            }
                        }
                        cx.notify();
                    })
                    .ok();
                }
            });
        let mut fields = div().flex().flex_col().gap(theme.metrics.space_sm);
        fields = fields
            .child(Field::new(
                t.t("screenLayout.name"),
                TextInput::new("output-format-name", &draft.name),
            ))
            .child(Field::new(
                t.t("screenTargets.preset"),
                div()
                    .debug_selector(|| "output-format-preset".into())
                    .child(preset),
            ));
        let mut dialog = Dialog::new(
            "output-format-dialog",
            t.t(draft.kind.title()),
            t.t("common.close"),
            {
                let weak = weak.clone();
                move |_, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.video.target_format = None;
                        cx.notify();
                    })
                    .ok();
                }
            },
        )
        .width(theme.metrics.space_lg * 25.);
        for (key, label, state, range, step) in [
            ("width", "capture.width", &draft.width, (16., 8192.), 1.),
            ("height", "capture.height", &draft.height, (16., 8192.), 1.),
            ("rate", "capture.fps", &draft.rate, (1., 120.), 0.01),
        ] {
            fields = fields.child(Field::new(
                t.t(label),
                div()
                    .debug_selector(move || format!("output-format-{key}"))
                    .child(
                        NumberInput::new(SharedString::from(format!("output-format-{key}")), state)
                            .range(range.0, range.1)
                            .step(step)
                            .stepper(),
                    ),
            ));
        }
        dialog = dialog
            .child(fields)
            .children(draft.error.as_ref().map(|error| {
                div()
                    .debug_selector(|| "output-format-error".into())
                    .text_color(theme.colors.warning)
                    .child(error.clone())
            }))
            .footer(
                ActionButton::new("output-format-cancel", t.t("common.cancel"))
                    .variant(ButtonVariant::Ghost)
                    .on_click({
                        let weak = weak.clone();
                        move |_, _, cx| {
                            weak.update(cx, |view, cx| {
                                view.video.target_format = None;
                                cx.notify();
                            })
                            .ok();
                        }
                    }),
            )
            .footer(
                ActionButton::new("output-format-apply", t.t("common.ok")).on_click(
                    move |_, _, cx| {
                        weak.update(cx, |view, cx| view.commit_target_format(cx))
                            .ok();
                    },
                ),
            );
        dialog
    }

    pub(super) fn output_target_overlays(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        if let Some(menu) = &self.video.target_menu {
            overlays.push(self.target_menu(menu, cx).into_any_element());
        }
        if let Some(draft) = &self.video.target_format {
            overlays.push(self.target_format_dialog(draft, cx).into_any_element());
        }
        overlays
    }
}
