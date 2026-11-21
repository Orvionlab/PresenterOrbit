//! A logical screen, its selectable physical outputs, and immediate device calibration.
use std::rc::Rc;

use gpui::{div, prelude::*, relative, AnyElement, Context, SharedString, Window};
use orbit_host::output_layout::{
    neutral_corners, AlphaOutput, DeviceTarget, LayoutMode, OutputLayout,
};
use orbit_ui::controls::{
    range_state, ActionButton, ButtonVariant, CheckboxField, ColorPicker, Field, IconButton,
    PanelHeader, RangeInput, Select, SelectOption, SliderEvent, TabItem, TabList, TextInput,
    ToggleSwitch,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::{
    video::{group_entries, OUTPUT_FORMATS},
    view::SettingsView,
};

struct Numeric {
    key: String,
    label: String,
    value: f64,
    range: (f64, f64),
    step: f64,
}

fn numeric(
    key: impl Into<String>,
    label: String,
    value: f64,
    range: (f64, f64),
    step: f64,
) -> Numeric {
    Numeric {
        key: key.into(),
        label,
        value,
        range,
        step,
    }
}

impl SettingsView {
    pub(super) fn edit_layout(
        &mut self,
        id: &str,
        change: impl FnOnce(&mut OutputLayout),
        cx: &mut Context<Self>,
    ) {
        let result =
            OutputLayout::from_screen(id, &self.screen_configs(cx)[id]).and_then(|mut layout| {
                change(&mut layout);
                layout.validate()?;
                serde_json::to_value(layout).map_err(|error| error.to_string())
            });
        match result {
            Ok(value) => {
                self.video.layout_error = None;
                self.update_screen(id, json!({"outputLayout": value}), cx);
            }
            Err(error) => {
                self.video.layout_error = Some(error);
                cx.notify();
            }
        }
    }

    fn resize_layout(
        &mut self,
        id: &str,
        mode: LayoutMode,
        rows: usize,
        columns: usize,
        cx: &mut Context<Self>,
    ) {
        let result =
            OutputLayout::from_screen(id, &self.screen_configs(cx)[id]).and_then(|mut layout| {
                layout.resize(mode, rows, columns, columns)?;
                serde_json::to_value(layout).map_err(|error| error.to_string())
            });
        match result {
            Ok(value) => {
                self.video.selected_output = 0;
                self.video.selected_blend = None;
                self.video.output_tab = "hardware".into();
                self.video.calibration_sliders.clear();
                self.video.layout_error = None;
                self.update_screen(id, json!({"outputLayout": value}), cx);
            }
            Err(error) => {
                self.video.layout_error = Some(error);
                cx.notify();
            }
        }
    }

    fn layout_numeric(
        &mut self,
        screen: &str,
        spec: Numeric,
        slider: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl Fn(&mut OutputLayout, f64) + 'static,
    ) -> AnyElement {
        let theme = *cx.theme();
        let change = Rc::new(change);
        let key = format!("output-{screen}-{}", spec.key);
        let (owner, typed) = (screen.to_string(), change.clone());
        let control = self.number_control(
            &key,
            &spec.value.to_string(),
            spec.range,
            spec.step,
            window,
            cx,
            move |view, value, cx| {
                if let Ok(value) = value.trim().parse::<f64>() {
                    view.edit_layout(&owner, |layout| typed(layout, value), cx);
                }
            },
        );
        let mut row = div()
            .flex()
            .items_center()
            .gap(theme.metrics.space_sm)
            .child(
                div()
                    .flex_none()
                    .w(theme.metrics.space_lg * 5.)
                    .text_size(theme.metrics.font_sm)
                    .child(spec.label.clone()),
            );
        let number_selector = format!("{key}-number");
        if slider {
            let state = if let Some(state) = self.video.calibration_sliders.get(&key) {
                state.clone()
            } else {
                let state = range_state(
                    spec.range.0 as f32,
                    spec.range.1 as f32,
                    spec.step as f32,
                    spec.value as f32,
                    cx,
                );
                let owner = screen.to_string();
                cx.subscribe(&state, move |view, _, event: &SliderEvent, cx| {
                    if let SliderEvent::Change(value) = event {
                        view.edit_layout(
                            &owner,
                            |layout| change(layout, f64::from(value.end())),
                            cx,
                        );
                    }
                })
                .detach();
                self.video
                    .calibration_sliders
                    .insert(key.clone(), state.clone());
                state
            };
            if (f64::from(state.read(cx).value().end()) - spec.value).abs() > spec.step * 0.5 {
                state.update(cx, |state, cx| {
                    state.set_value(spec.value as f32, window, cx)
                });
            }
            row = row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .debug_selector(move || format!("{key}-slider"))
                    .child(RangeInput::new(&state).label(spec.label)),
            );
        }
        row.child(
            div()
                .w(theme.metrics.space_lg * 4.)
                .debug_selector(move || number_selector)
                .child(control),
        )
        .into_any_element()
    }

    pub(super) fn layout_target(
        &mut self,
        screen: &str,
        index: usize,
        key: bool,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        let display = value.strip_prefix("display:").and_then(|id| {
            self.runtime
                .output
                .read(cx)
                .displays
                .as_array()?
                .iter()
                .find(|display| display["id"] == id)
                .cloned()
        });
        let selected = value.to_string();
        let default_name = format!(
            "Presenter Orbit {screen} {}{}",
            index + 1,
            if key { " Key" } else { "" }
        );
        self.edit_layout(
            screen,
            move |layout| {
                let target = if key {
                    layout.outputs[index]
                        .alpha
                        .as_mut()
                        .map(|alpha| &mut alpha.target)
                } else {
                    Some(&mut layout.outputs[index].target)
                };
                let Some(target) = target else {
                    return;
                };
                target.source_type = None;
                if selected == "placeholder" {
                    target.kind = "placeholder".into();
                    target.id.clear();
                    target.name.clear();
                } else if selected == "ndi" {
                    target.kind = "ndi".into();
                    target.id.clear();
                    target.name = default_name;
                } else if let Some(display) = display {
                    target.kind = "display".into();
                    target.id = display["id"].as_str().unwrap_or("").into();
                    target.name = display["label"].as_str().unwrap_or("").into();
                    if let (Some(width), Some(height)) = (
                        display["bounds"]["width"].as_u64(),
                        display["bounds"]["height"].as_u64(),
                    ) {
                        target.width = width as usize;
                        target.height = height as usize;
                    }
                }
                if index == 0 && !key {
                    layout.tile_width = layout.outputs[0].target.width;
                    layout.tile_height = layout.outputs[0].target.height;
                }
            },
            cx,
        );
    }

    pub(super) fn device_title(&self, target: &DeviceTarget, cx: &gpui::App) -> String {
        if target.kind == "placeholder" && target.name.is_empty() {
            return cx.global::<Translator>().t("screenLayout.unconfigured");
        }
        if target.kind == "display" {
            if let Some(label) = self
                .runtime
                .output
                .read(cx)
                .displays
                .as_array()
                .into_iter()
                .flatten()
                .find(|display| display["id"] == target.id)
                .and_then(|display| display["label"].as_str())
            {
                return label.to_string();
            }
        }
        if target.name.is_empty() {
            target.id.clone()
        } else {
            target.name.clone()
        }
    }

    fn layout_diagram(&self, layout: &OutputLayout, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let canvas = layout.canvas().ok();
        let width = canvas.map_or(1., |canvas| canvas.width as f32);
        let height = canvas.map_or(1., |canvas| canvas.height as f32);
        let width = if layout.mode == LayoutMode::Mirror {
            (layout.tile_width * layout.outputs.len()) as f32
        } else {
            width
        };
        let mut diagram = div()
            .relative()
            .w_full()
            .aspect_ratio(width / height)
            .max_h(theme.metrics.space_lg * 12.)
            .bg(theme.colors.canvas_bg);
        for (index, device) in layout.outputs.iter().enumerate() {
            let region = layout.region(index);
            let x = if layout.mode == LayoutMode::Mirror {
                index * layout.tile_width
            } else {
                region.x
            };
            let selected =
                self.video.selected_output == index && self.video.selected_blend.is_none();
            let title = self.device_title(&device.target, cx);
            diagram = diagram.child(
                div()
                    .id(SharedString::from(format!("output-device-{index}")))
                    .debug_selector(move || format!("output-device-{index}"))
                    .absolute()
                    .left(relative(x as f32 / width))
                    .top(relative(region.y as f32 / height))
                    .w(relative(region.width as f32 / width))
                    .h(relative(region.height as f32 / height))
                    .border_2()
                    .border_color(if selected {
                        theme.colors.selected
                    } else {
                        theme.colors.border_strong
                    })
                    .bg(theme.colors.surface_raised_bg)
                    .p(theme.metrics.space_sm)
                    .cursor_pointer()
                    .text_size(theme.metrics.font_sm)
                    .text_color(theme.colors.text)
                    .text_center()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.video.selected_output = index;
                        view.video.selected_blend = None;
                        if view.video.output_tab == "blend" {
                            view.video.output_tab = "hardware".into();
                        }
                        view.video.calibration_sliders.clear();
                        cx.notify();
                    }))
                    .when(device.alpha.is_none(), |tile| tile.child(title.clone()))
                    .when_some(device.alpha.as_ref(), |tile, alpha| {
                        tile.child(
                            div()
                                .flex()
                                .size_full()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .pr(theme.metrics.space_xs)
                                        .child(cx.global::<Translator>().t("screenLayout.fill"))
                                        .child(div().truncate().child(title)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .pl(theme.metrics.space_xs)
                                        .border_l_1()
                                        .border_color(theme.colors.border)
                                        .bg(gpui::checkerboard(
                                            theme.colors.border_soft,
                                            f32::from(theme.metrics.space_xs),
                                        ))
                                        .child(cx.global::<Translator>().t("screenLayout.alpha"))
                                        .child(
                                            div()
                                                .truncate()
                                                .child(self.device_title(&alpha.target, cx)),
                                        ),
                                ),
                        )
                    }),
            );
        }
        for (index, seam) in layout.blends.iter().enumerate() {
            let region = layout.region(index + 1);
            diagram = diagram.child(
                div()
                    .id(SharedString::from(format!("output-blend-{index}")))
                    .debug_selector(move || format!("output-blend-{index}"))
                    .absolute()
                    .left(relative(region.x as f32 / width))
                    .top_0()
                    .bottom_0()
                    .w(relative(seam.width as f32 / width))
                    .bg(theme.colors.surface_bg)
                    .border_2()
                    .border_color(if self.video.selected_blend == Some(index) {
                        theme.colors.selected
                    } else {
                        theme.colors.border_strong
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.video.selected_blend = Some(index);
                        view.video.output_tab = "blend".into();
                        view.video.calibration_sliders.clear();
                        cx.notify();
                    })),
            );
        }
        div()
            .flex()
            .justify_center()
            .bg(theme.colors.canvas_bg)
            .py(theme.metrics.space_lg)
            .px(theme.metrics.panel_padding)
            .min_h(theme.metrics.space_lg * 14.)
            .items_center()
            .child(
                div()
                    .w_full()
                    .max_w(theme.metrics.space_lg * (12. * width / height).min(40.))
                    .child(diagram),
            )
            .into_any_element()
    }

    fn output_hardware(
        &mut self,
        id: &str,
        layout: &OutputLayout,
        index: usize,
        key: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let device = &layout.outputs[index];
        let target = if key {
            device.alpha.as_ref().map(|alpha| &alpha.target)
        } else {
            Some(&device.target)
        };
        let Some(target) = target else {
            return div().into_any_element();
        };
        let selection = super::video_targets::TargetSelection {
            screen: id.to_string(),
            index,
            key,
            device: device.id.clone(),
        };
        let prefix = format!("{index}-{}", if key { "key" } else { "fill" });
        let mut body = div().flex().flex_col().gap(theme.metrics.space_sm).child(
            div()
                .flex()
                .gap(theme.metrics.space_sm)
                .child(div().flex_1().min_w_0().child(Field::new(
                    t.t("screenSettings.output"),
                    self.output_target_button(selection.clone(), target, cx),
                )))
                .child(
                    div().flex_1().min_w_0().child(Field::new(
                        t.t("screenLayout.outputTarget"),
                        Select::new(
                            SharedString::from(format!("output-target-region-{prefix}")),
                            vec![SelectOption::new("full", t.t("screenLayout.full"))],
                            "full",
                        )
                        .compact()
                        .disabled(true),
                    )),
                ),
        );
        if target.kind == "placeholder" && target.name.is_empty() {
            return body.into_any_element();
        }
        let mut details = div()
            .flex()
            .flex_col()
            .max_w(theme.metrics.space_lg * 21.)
            .gap(theme.metrics.space_sm);
        if target.kind == "display" {
            details = details
                .child(
                    Field::new(
                        t.t("screenLayout.name"),
                        div().truncate().child(self.device_title(target, cx)),
                    )
                    .inline()
                    .label_width(theme.metrics.space_lg * 5.),
                )
                .child(
                    div()
                        .debug_selector({
                            let selector = format!("output-{prefix}-fixed-size");
                            move || selector
                        })
                        .child(
                            Field::new(
                                t.t("screenLayout.size"),
                                format!("{} × {}", target.width, target.height),
                            )
                            .inline()
                            .label_width(theme.metrics.space_lg * 5.),
                        ),
                );
        }
        if target.kind != "display" {
            let owner = id.to_string();
            details = details.child(self.text_field(
                &format!("output-name-{prefix}"),
                t.t("screenLayout.name"),
                &target.name,
                window,
                cx,
                move |view, value, cx| {
                    view.edit_layout(
                        &owner,
                        |layout| {
                            if key {
                                if let Some(alpha) = layout.outputs[index].alpha.as_mut() {
                                    alpha.target.name = value;
                                }
                            } else {
                                layout.outputs[index].target.name = value;
                            }
                        },
                        cx,
                    )
                },
            ));
        }
        if target.kind != "display" {
            let kind = if target.kind == "ndi" {
                super::video_targets::TargetKind::Ndi
            } else {
                super::video_targets::TargetKind::Placeholder
            };
            details = details.child(
                Field::new(
                    t.t("screenLayout.size"),
                    ActionButton::new(
                        SharedString::from(format!("output-size-{prefix}")),
                        format!(
                            "{} × {} · {} fps",
                            target.width, target.height, target.frame_rate
                        ),
                    )
                    .variant(ButtonVariant::Secondary)
                    .compact()
                    .icon("edit")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.open_target_format(selection.clone(), kind, window, cx)
                    })),
                )
                .inline()
                .label_width(theme.metrics.space_lg * 5.),
            );
        }
        if target.kind == "placeholder" {
            return body.child(details).into_any_element();
        }
        let delay = Some(self.layout_numeric(
            id,
            numeric(
                format!("{prefix}-delay"),
                t.t("screenSettings.videoDelayFrames"),
                target.video_delay as f64,
                (0., 120.),
                1.,
            ),
            false,
            window,
            cx,
            move |layout, value| {
                if key {
                    if let Some(alpha) = &mut layout.outputs[index].alpha {
                        alpha.target.video_delay = value as usize;
                    }
                } else {
                    layout.outputs[index].target.video_delay = value as usize;
                    if let Some(alpha) = &mut layout.outputs[index].alpha {
                        alpha.target.video_delay = value as usize;
                    }
                }
            },
        ));
        let owner = id.to_string();
        let weak = cx.weak_entity();
        details = details.child(
            Field::new(
                t.t("screenLayout.format"),
                Select::new(
                    SharedString::from(format!("output-format-{prefix}")),
                    OUTPUT_FORMATS
                        .iter()
                        .map(|(value, label)| SelectOption::new(value.to_string(), *label))
                        .collect(),
                    target.format.clone(),
                )
                .compact()
                .on_change(move |format, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.edit_layout(
                            &owner,
                            |layout| {
                                if key {
                                    if let Some(alpha) = layout.outputs[index].alpha.as_mut() {
                                        alpha.target.format = format;
                                    }
                                } else {
                                    layout.outputs[index].target.format = format;
                                }
                            },
                            cx,
                        )
                    })
                    .ok();
                }),
            )
            .inline()
            .label_width(theme.metrics.space_lg * 5.),
        );
        body = body.child(details).children(delay.map(|control| {
            div()
                .mt(theme.metrics.space_sm)
                .max_w(theme.metrics.space_lg * 21.)
                .child(control)
        }));
        body.into_any_element()
    }

    fn output_calibration(
        &mut self,
        id: &str,
        layout: &OutputLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let index = self.video.selected_output.min(layout.outputs.len() - 1);
        let device = &layout.outputs[index];
        let mut body = div().flex().flex_col().gap(theme.metrics.space_sm);
        match self.video.output_tab.as_str() {
            "color" => {
                body = body.max_w(theme.metrics.space_lg * 30.).mx_auto();
                for name in ["red", "green", "blue", "gamma", "black"] {
                    let value = match name {
                        "red" => device.color.red,
                        "green" => device.color.green,
                        "blue" => device.color.blue,
                        "gamma" => device.color.gamma,
                        _ => device.color.black,
                    };
                    body = body.child(self.layout_numeric(
                        id,
                        numeric(
                            format!("{index}-color-{name}"),
                            t.t(&format!("screenLayout.{name}")),
                            value,
                            if name == "gamma" {
                                (-2., 2.)
                            } else {
                                (-1., 1.)
                            },
                            0.01,
                        ),
                        true,
                        window,
                        cx,
                        move |layout, value| {
                            let color = &mut layout.outputs[index].color;
                            match name {
                                "red" => color.red = value,
                                "green" => color.green = value,
                                "blue" => color.blue = value,
                                "gamma" => color.gamma = value,
                                _ => color.black = value,
                            }
                        },
                    ));
                }
            }
            "corners" => {
                let owner = id.to_string();
                let weak = cx.weak_entity();
                body = body.child(
                    div()
                        .debug_selector(|| "output-corner-enabled".into())
                        .child(
                            CheckboxField::new(
                                "output-corner-enabled",
                                t.t("screenLayout.enabled"),
                                device.corner_pin.is_some(),
                            )
                            .on_change(move |enabled, _, cx| {
                                weak.update(cx, |view, cx| {
                                    view.edit_layout(
                                        &owner,
                                        |layout| {
                                            layout.outputs[index].corner_pin =
                                                enabled.then(neutral_corners)
                                        },
                                        cx,
                                    )
                                })
                                .ok();
                            }),
                        ),
                );
                if let Some(corners) = device.corner_pin {
                    let corners = [
                        ("topLeft", corners.top_left, 0., 0., -1., -1.),
                        ("topRight", corners.top_right, 1., 0., 1., -1.),
                        ("bottomLeft", corners.bottom_left, 0., 1., -1., 1.),
                        ("bottomRight", corners.bottom_right, 1., 1., 1., 1.),
                    ];
                    for pair in corners.chunks(2) {
                        let mut row = div().flex().gap(theme.metrics.space_lg);
                        for &(name, point, ox, oy, sx, sy) in pair {
                            let mut fields = div().flex().gap(theme.metrics.space_sm);
                            for axis in ["x", "y"] {
                                let (value, size) = if axis == "x" {
                                    (
                                        (point.x - ox) * sx * layout.tile_width as f64,
                                        layout.tile_width as f64,
                                    )
                                } else {
                                    (
                                        (point.y - oy) * sy * layout.tile_height as f64,
                                        layout.tile_height as f64,
                                    )
                                };
                                let key = format!("output-{id}-{index}-{name}-{axis}");
                                let owner = id.to_string();
                                let control = self.number_control(
                                    &key,
                                    &value.to_string(),
                                    (-size, size),
                                    1.,
                                    window,
                                    cx,
                                    move |view, value, cx| {
                                        if let Ok(value) = value.parse::<f64>() {
                                            view.edit_layout(
                                                &owner,
                                                |layout| {
                                                    let size = if axis == "x" {
                                                        layout.tile_width
                                                    } else {
                                                        layout.tile_height
                                                    }
                                                        as f64;
                                                    if let Some(corners) =
                                                        layout.outputs[index].corner_pin.as_mut()
                                                    {
                                                        let point = match name {
                                                            "topLeft" => &mut corners.top_left,
                                                            "topRight" => &mut corners.top_right,
                                                            "bottomLeft" => {
                                                                &mut corners.bottom_left
                                                            }
                                                            _ => &mut corners.bottom_right,
                                                        };
                                                        if axis == "x" {
                                                            point.x = ox + sx * value / size;
                                                        } else {
                                                            point.y = oy + sy * value / size;
                                                        }
                                                    }
                                                },
                                                cx,
                                            );
                                        }
                                    },
                                );
                                fields = fields.child(
                                    div()
                                        .w(theme.metrics.space_lg * 5.)
                                        .flex()
                                        .flex_col()
                                        .gap(theme.metrics.space_2xs)
                                        .child(
                                            div()
                                                .debug_selector(move || format!("{key}-number"))
                                                .child(control),
                                        )
                                        .child(
                                            div()
                                                .text_center()
                                                .text_size(theme.metrics.font_xs)
                                                .text_color(theme.colors.text_muted)
                                                .child(format!("Δ{}", axis.to_uppercase())),
                                        ),
                                );
                            }
                            row = row.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(theme.metrics.space_xs)
                                    .child(
                                        div()
                                            .text_size(theme.metrics.font_sm)
                                            .child(t.t(&format!("screenLayout.{name}"))),
                                    )
                                    .child(fields),
                            );
                        }
                        body = body.child(row);
                    }
                }
            }
            "alpha" => {
                let current = device
                    .alpha
                    .as_ref()
                    .map_or("none", |alpha| match alpha.mode {
                        orbit_host::output_layout::FillMode::Straight => "straight",
                        orbit_host::output_layout::FillMode::Premultiplied => "premultiplied",
                    });
                let owner = id.to_string();
                let weak = cx.weak_entity();
                body = body.child(
                    div().max_w(theme.metrics.space_lg * 26.).mx_auto().child(
                        Field::new(
                            t.t("screenLayout.alphaType"),
                            div().debug_selector(|| "output-alpha-mode".into()).child(
                                Select::new(
                                    "output-alpha-mode",
                                    [
                                        ("none", t.t("common.none")),
                                        ("premultiplied", t.t("screenLayout.premultiplied")),
                                        ("straight", t.t("screenLayout.straight")),
                                    ]
                                    .into_iter()
                                    .map(|(value, label)| {
                                        SelectOption::new(value.to_string(), label)
                                    })
                                    .collect(),
                                    current.to_string(),
                                )
                                .compact()
                                .on_change(move |mode, _, cx| {
                                    weak.update(cx, |view, cx| {
                                        view.edit_layout(
                                &owner,
                                |layout| {
                                    if mode == "none" {
                                        layout.outputs[index].alpha = None;
                                    } else {
                                        let mut target =
                                            layout.outputs[index].alpha.as_ref().map_or_else(
                                                || layout.outputs[index].target.clone(),
                                                |alpha| alpha.target.clone(),
                                            );
                                        if layout.outputs[index].alpha.is_none() {
                                            target.kind = "placeholder".into();
                                            target.id.clear();
                                            target.name.clear();
                                        }
                                        layout.outputs[index].alpha = Some(AlphaOutput {
                                            target,
                                            mode: if mode == "straight" {
                                                orbit_host::output_layout::FillMode::Straight
                                            } else {
                                                orbit_host::output_layout::FillMode::Premultiplied
                                            },
                                        });
                                    }
                                },
                                cx,
                            )
                                    })
                                    .ok();
                                }),
                            ),
                        )
                        .inline()
                        .label_width(theme.metrics.space_lg * 5.),
                    ),
                );
                if device.alpha.is_some() {
                    body = body.child(self.output_hardware(id, layout, index, true, window, cx));
                }
            }
            "blend" => {
                if let Some(seam_index) = self
                    .video
                    .selected_blend
                    .filter(|index| *index < layout.blends.len())
                {
                    let seam = &layout.blends[seam_index];
                    body = body.child(self.layout_numeric(
                        id,
                        numeric(
                            format!("blend-{seam_index}-width"),
                            t.t("capture.width"),
                            seam.width as f64,
                            (1., (layout.tile_width / 2 - 1) as f64),
                            1.,
                        ),
                        true,
                        window,
                        cx,
                        move |layout, value| layout.blends[seam_index].width = value as usize,
                    ));
                    let mut columns = div().flex().gap(theme.metrics.space_lg);
                    for (left, curve) in [(true, seam.left), (false, seam.right)] {
                        let mut column = div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(theme.metrics.space_sm)
                            .child(PanelHeader::new(t.t(if left {
                                "screenLayout.leftScreen"
                            } else {
                                "screenLayout.rightScreen"
                            })));
                        let owner = id.to_string();
                        let weak = cx.weak_entity();
                        column = column.child(
                            Select::new(
                                SharedString::from(format!("blend-curve-{left}")),
                                ["linear", "quadratic", "cubic"]
                                    .iter()
                                    .map(|mode| {
                                        SelectOption::new(
                                            mode.to_string(),
                                            t.t(&format!("screenLayout.{mode}")),
                                        )
                                    })
                                    .collect(),
                                serde_json::to_value(curve.mode)
                                    .ok()
                                    .and_then(|v| v.as_str().map(str::to_string))
                                    .unwrap_or_default(),
                            )
                            .compact()
                            .on_change(move |mode, _, cx| {
                                weak.update(cx, |view, cx| {
                                    view.edit_layout(
                                        &owner,
                                        |layout| {
                                            let curve = if left {
                                                &mut layout.blends[seam_index].left
                                            } else {
                                                &mut layout.blends[seam_index].right
                                            };
                                            curve.mode = match mode.as_str() {
                                                "quadratic" => {
                                                    orbit_host::output_layout::BlendMode::Quadratic
                                                }
                                                "cubic" => {
                                                    orbit_host::output_layout::BlendMode::Cubic
                                                }
                                                _ => orbit_host::output_layout::BlendMode::Linear,
                                            };
                                        },
                                        cx,
                                    )
                                })
                                .ok();
                            }),
                        );
                        for (name, value, range) in [
                            ("radius", curve.radius, (0.01, 1.)),
                            ("intensity", curve.intensity, (0., 2.)),
                            ("gamma", curve.gamma, (-2., 2.)),
                            ("black", curve.black_level, (0., 1.)),
                        ] {
                            column = column.child(self.layout_numeric(
                                id,
                                numeric(
                                    format!("blend-{seam_index}-{left}-{name}"),
                                    t.t(&format!("screenLayout.{name}")),
                                    value,
                                    range,
                                    0.01,
                                ),
                                true,
                                window,
                                cx,
                                move |layout, value| {
                                    let curve = if left {
                                        &mut layout.blends[seam_index].left
                                    } else {
                                        &mut layout.blends[seam_index].right
                                    };
                                    match name {
                                        "radius" => curve.radius = value,
                                        "intensity" => curve.intensity = value,
                                        "gamma" => curve.gamma = value,
                                        _ => curve.black_level = value,
                                    }
                                },
                            ));
                        }
                        columns = columns.child(column);
                    }
                    body = body.child(columns);
                }
            }
            _ => {
                body = body.child(self.output_hardware(id, layout, index, false, window, cx));
            }
        }
        body.into_any_element()
    }

    fn output_configuration(
        &mut self,
        id: &str,
        config: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if config
            .get("canvasView")
            .is_some_and(|value| !value.is_null())
            || config.get("keyFill").is_some_and(|value| !value.is_null())
            || config
                .get("projectorCorrection")
                .is_some_and(|value| !value.is_null())
        {
            return self.screen_card(id, config, window, cx);
        }
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let layout = match OutputLayout::from_screen(id, config) {
            Ok(layout) => layout,
            Err(error) => {
                return div()
                    .text_color(theme.colors.warning)
                    .child(error)
                    .into_any_element()
            }
        };
        let size = layout.canvas().ok();
        let owner = id.to_string();
        let weak = cx.weak_entity();
        let mode = serde_json::to_value(layout.mode)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let mut controls = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(theme.metrics.space_sm)
            .child(
                div()
                    .w(theme.metrics.space_lg * 9.)
                    .debug_selector(|| "output-layout-mode".into())
                    .child(
                        Select::new(
                            "output-layout-mode",
                            ["single", "mirror", "grouped", "edge-blend"]
                                .iter()
                                .map(|mode| {
                                    SelectOption::new(
                                        mode.to_string(),
                                        t.t(&format!("screenLayout.{mode}")),
                                    )
                                })
                                .collect(),
                            mode,
                        )
                        .compact()
                        .on_change(move |mode, _, cx| {
                            weak.update(cx, |view, cx| {
                                let mode = match mode.as_str() {
                                    "mirror" => LayoutMode::Mirror,
                                    "grouped" => LayoutMode::Grouped,
                                    "edge-blend" => LayoutMode::EdgeBlend,
                                    _ => LayoutMode::Single,
                                };
                                view.resize_layout(
                                    &owner,
                                    mode,
                                    if mode == LayoutMode::Grouped { 2 } else { 1 },
                                    if mode == LayoutMode::Single { 1 } else { 2 },
                                    cx,
                                );
                            })
                            .ok();
                        }),
                    ),
            );
        let mut counts = div().flex().items_center().gap(theme.metrics.space_sm);
        for (key, label, count) in if layout.mode == LayoutMode::Grouped {
            vec![
                ("rows", "screenLayout.rows", layout.rows),
                ("columns", "screenLayout.columns", layout.columns),
            ]
        } else if layout.mode != LayoutMode::Single {
            vec![("devices", "screenLayout.devices", layout.outputs.len())]
        } else {
            Vec::new()
        } {
            let owner = id.to_string();
            let (mode, rows, columns) = (layout.mode, layout.rows, layout.columns);
            let control = self.number_control(
                &format!("output-layout-{key}"),
                &count.to_string(),
                (1., 16.),
                1.,
                window,
                cx,
                move |view, value, cx| {
                    if let Ok(value) = value.parse::<usize>() {
                        view.resize_layout(
                            &owner,
                            mode,
                            if key == "rows" { value } else { rows },
                            if key != "rows" { value } else { columns },
                            cx,
                        );
                    }
                },
            );
            counts = counts
                .child(div().text_size(theme.metrics.font_sm).child(t.t(label)))
                .child(div().w(theme.metrics.space_lg * 4.).child(control));
        }
        controls = controls.child(counts);
        let title: AnyElement = if let Some(rename) = self
            .video
            .renaming
            .as_ref()
            .filter(|rename| rename.id == id)
        {
            TextInput::new("output-screen-title", &rename.input).into_any_element()
        } else {
            let owner = id.to_string();
            div()
                .id("output-screen-title")
                .font_weight(gpui::FontWeight::BOLD)
                .text_size(theme.metrics.font_lg)
                .cursor_pointer()
                .on_click(
                    cx.listener(move |view, event: &gpui::ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            view.begin_screen_rename(&owner, window, cx);
                        }
                    }),
                )
                .child(config["label"].as_str().unwrap_or(id).to_string())
                .into_any_element()
        };
        let owner = id.to_string();
        let weak = cx.weak_entity();
        let color = CheckboxField::new(
            "output-screen-color-enabled",
            t.t("screenLayout.screenColor"),
            layout.screen_color.is_some(),
        )
        .on_change(move |enabled, _, cx| {
            weak.update(cx, |view, cx| {
                view.edit_layout(
                    &owner,
                    |layout| layout.screen_color = enabled.then(|| "transparent".to_string()),
                    cx,
                )
            })
            .ok();
        });
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(theme.metrics.space_sm)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(title)
                    .child(div().text_size(theme.metrics.font_xs).child(
                        size.map_or_else(String::new, |size| {
                            format!("{} × {}", size.width, size.height)
                        }),
                    )),
            )
            .child(
                div()
                    .debug_selector(|| "output-screen-color-enabled".into())
                    .child(color),
            );
        if let Some(color) = &layout.screen_color {
            let owner = id.to_string();
            let weak = cx.weak_entity();
            header = header.child(
                div()
                    .w(theme.metrics.space_lg * 4.)
                    .flex_none()
                    .debug_selector(|| "output-screen-color".into())
                    .child(
                        ColorPicker::new(
                            "output-screen-color",
                            t.t("screenLayout.screenColor"),
                            color.clone(),
                        )
                        .on_change(move |color, _, cx| {
                            weak.update(cx, |view, cx| {
                                view.edit_layout(
                                    &owner,
                                    |layout| layout.screen_color = Some(color),
                                    cx,
                                )
                            })
                            .ok();
                        }),
                    ),
            );
        }
        let owner = id.to_string();
        header = if self.video.confirm_delete.as_deref() == Some(id) {
            header.child(
                ActionButton::new(
                    "output-delete-confirm",
                    t.format(
                        "screenSettings.confirmDeleteOutput",
                        &[("name", config["label"].as_str().unwrap_or(id))],
                    ),
                )
                .variant(ButtonVariant::Danger)
                .on_click(cx.listener(move |view, _, _, cx| view.delete_screen(&owner, true, cx))),
            )
        } else {
            header.child(
                IconButton::new("output-delete", "trash", t.t("common.delete")).on_click(
                    cx.listener(move |view, _, _, cx| view.delete_screen(&owner, false, cx)),
                ),
            )
        };
        let tabs = if self.video.selected_blend.is_some() {
            vec![TabItem::new(
                "blend".to_string(),
                t.t("screenLayout.edge-blend"),
            )]
        } else {
            vec![
                TabItem::new("hardware".to_string(), t.t("screenLayout.hardware")),
                TabItem::new("color".to_string(), t.t("screenLayout.color")),
                TabItem::new("corners".to_string(), t.t("screenLayout.corners")),
                TabItem::new("alpha".to_string(), t.t("screenLayout.alpha"))
                    .disabled(config["type"] == "stage"),
            ]
        };
        if self.video.output_tab.is_empty() {
            self.video.output_tab = "hardware".into();
        }
        let weak = cx.weak_entity();
        let tabs = TabList::new(
            "output-device-tabs",
            t.t("screenLayout.deviceSettings"),
            tabs,
            self.video.output_tab.clone(),
        )
        .equal_width()
        .on_change(move |tab, _, cx| {
            weak.update(cx, |view, cx| {
                view.video.output_tab = tab.clone();
                view.video.calibration_sliders.clear();
                cx.notify();
            })
            .ok();
        });
        let calibration = self.output_calibration(id, &layout, window, cx);
        let device = &layout.outputs[self.video.selected_output.min(layout.outputs.len() - 1)];
        let device_title = self
            .video
            .selected_blend
            .filter(|index| *index < layout.blends.len())
            .map_or_else(
                || self.device_title(&device.target, cx),
                |index| {
                    format!(
                        "{} + {}",
                        self.device_title(&layout.outputs[index].target, cx),
                        self.device_title(&layout.outputs[index + 1].target, cx)
                    )
                },
            );
        let stage = self.video_extra_fields(id, config, window, cx);
        div()
            .id("output-configuration")
            .debug_selector(|| "output-configuration".into())
            .flex()
            .flex_col()
            .child(div().p(theme.metrics.panel_padding).child(header))
            .child(
                div()
                    .px(theme.metrics.panel_padding)
                    .pb(theme.metrics.space_sm)
                    .child(controls),
            )
            .child(self.layout_diagram(&layout, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .bg(theme.colors.panel_bg)
                    .p(theme.metrics.panel_padding)
                    .gap(theme.metrics.space_sm)
                    .child(div().text_size(theme.metrics.font_sm).child(device_title))
                    .child(
                        div().flex().justify_center().child(
                            div()
                                .w(theme.metrics.space_lg * 22.)
                                .debug_selector(|| "output-device-tabs".into())
                                .child(tabs),
                        ),
                    )
                    .child(
                        div()
                            .border_1()
                            .border_color(theme.colors.border)
                            .p(theme.metrics.panel_padding)
                            .min_h(theme.metrics.space_lg * 16.)
                            .child(calibration),
                    ),
            )
            .when(config["type"] == "stage", |view| view.child(stage))
            .children(
                self.video
                    .layout_error
                    .clone()
                    .map(|error| div().text_color(theme.colors.warning).child(error)),
            )
            .into_any_element()
    }

    pub(super) fn output_configuration_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let configs = self.screen_configs(cx);
        if configs.get(&self.video.selected_screen).is_none() {
            self.video.selected_screen = configs
                .as_object()
                .and_then(|screens| screens.keys().next())
                .cloned()
                .unwrap_or_default();
            self.video.selected_output = 0;
            self.video.selected_blend = None;
        }
        let mut sidebar = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(theme.metrics.space_lg * 12.)
            .border_r_1()
            .border_color(theme.colors.border)
            .bg(theme.colors.panel_bg_strong);
        for (group, label) in [("audience", "common.audience"), ("stage", "common.stage")] {
            let all_on = group_entries(&configs, group)
                .iter()
                .all(|(_, screen)| screen["enabled"] == true);
            let weak = cx.weak_entity();
            sidebar = sidebar.child(
                PanelHeader::new(t.t(label))
                    .action(
                        ToggleSwitch::new(
                            SharedString::from(format!("output-group-{group}")),
                            t.t(label),
                            all_on,
                        )
                        .bare()
                        .on_change(move |_, _, cx| {
                            weak.update(cx, |view, cx| view.toggle_group(group, cx))
                                .ok();
                        }),
                    )
                    .action(
                        IconButton::new(
                            SharedString::from(format!("output-add-{group}")),
                            "plus",
                            t.t("screenLayout.addScreen"),
                        )
                        .on_click(cx.listener(move |view, _, _, cx| view.add_screen(group, cx))),
                    ),
            );
            for (id, config) in group_entries(&configs, group) {
                let owner = id.clone();
                let select = id.clone();
                let weak = cx.weak_entity();
                let enabled = config["enabled"] == true;
                let toggle = ToggleSwitch::new(
                    SharedString::from(format!("output-enabled-{id}")),
                    t.t("screenSettings.output"),
                    enabled,
                )
                .bare()
                .on_change(move |enabled, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.update_screen(&owner, json!({"enabled": enabled}), cx)
                    })
                    .ok();
                });
                let row = Self::sidebar_item(
                    id.clone(),
                    config["label"].as_str().unwrap_or(&id).to_string(),
                    self.describe_target(&config, cx),
                    enabled,
                    false,
                    toggle,
                    cx,
                );
                sidebar = sidebar.child(
                    div()
                        .id(SharedString::from(format!("output-screen-{id}")))
                        .debug_selector(move || format!("output-screen-{id}"))
                        .cursor_pointer()
                        .border_l_2()
                        .border_color(if self.video.selected_screen == select {
                            theme.colors.selected
                        } else {
                            theme.colors.panel_bg_strong
                        })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.video.selected_screen = select.clone();
                            view.video.selected_output = 0;
                            view.video.selected_blend = None;
                            view.video.output_tab = "hardware".into();
                            view.video.calibration_sliders.clear();
                            cx.notify();
                        }))
                        .child(row),
                );
            }
        }
        let selected = self.video.selected_screen.clone();
        let content = configs
            .get(&selected)
            .map(|config| self.output_configuration(&selected, config, window, cx));
        div()
            .id("settings-video")
            .flex()
            .min_w_0()
            .child(sidebar)
            .child(div().flex_1().min_w_0().children(content))
            .into_any_element()
    }
}
