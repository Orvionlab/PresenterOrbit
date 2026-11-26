use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    canvas, checkerboard, deferred, div, linear_color_stop, linear_gradient, prelude::*, px, rgb,
    App, Bounds, Context, ElementId, Entity, Hsla, MouseButton, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Rgba, SharedString, Subscription, Window,
};
use gpui_base::input::InputState;
use gpui_base::slider::{SliderEvent, SliderState};
use gpui_base::{Button, ElementExt, Popup};

use super::color_value::{
    format_propresenter_hex, hex_alpha, hex_to_rgb, hsv_to_rgb, is_transparent, normalize_hex,
    parse_hex_input, rgb_to_hex, rgb_to_hsv, rgb_to_hue, Rgb, AVAILABLE_COLORS, STANDARD_COLORS,
};
use super::{child_id, focus_ring, input_state, range_state, tooltip_text, Handler};
use super::{InputEvent, NumberInput, TextInput};
use crate::{icon, ActiveTheme, Translator};

const SWATCH: Pixels = px(16.);
const GRID_WIDTH: Pixels = px(216.);
const POPOVER_WIDTH: Pixels = px(244.);
const SPECTRUM_HEIGHT: Pixels = px(154.);
const HUE_WIDTH: Pixels = px(18.);
const TRANSPARENT_CHECK: Rgba = Rgba {
    r: 0.81,
    g: 0.81,
    b: 0.81,
    a: 1.,
};

fn hex_color(value: &str) -> Rgba {
    let Rgb { red, green, blue } = hex_to_rgb(value);
    rgb(u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue))
}

fn with_alpha(color: Rgba, alpha: u8) -> Rgba {
    Rgba {
        a: f32::from(alpha) / 255.,
        ..color
    }
}

/// The position inside `bounds` as fractions from the top left, clamped to the edges.
pub fn fraction_within(bounds: Bounds<Pixels>, position: Point<Pixels>) -> (f64, f64) {
    let fraction = |offset: Pixels, size: Pixels| {
        if size <= px(0.) {
            0.
        } else {
            f64::from(f32::from(offset) / f32::from(size)).clamp(0., 1.)
        }
    };
    (
        fraction(position.x - bounds.origin.x, bounds.size.width),
        fraction(position.y - bounds.origin.y, bounds.size.height),
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Standard,
    Advanced,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Spectrum,
    Hue,
}

struct Channel {
    label: &'static str,
    slider: Entity<SliderState>,
    input: Entity<InputState>,
}

/// The picker's content. It keeps editing state between renders, so it is a view.
pub struct ColorPopover {
    value: String,
    transparent: bool,
    mode: Mode,
    hue: f64,
    channels: [Channel; 4],
    hex: Entity<InputState>,
    spectrum: Rc<Cell<Bounds<Pixels>>>,
    hue_bar: Rc<Cell<Bounds<Pixels>>>,
    drag: Option<Drag>,
    on_change: Option<Handler<String>>,
    on_select: Option<Handler<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ColorPopover {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let channels = ["R", "G", "B", "A"].map(|label| Channel {
            label,
            slider: range_state(0., 255., 1., 255., cx),
            input: input_state("255", window, cx),
        });
        let hex = input_state("", window, cx);
        let mut subscriptions = Vec::new();
        for (index, channel) in channels.iter().enumerate() {
            subscriptions.push(cx.subscribe_in(
                &channel.slider,
                window,
                move |this, _, event, window, cx| {
                    if let SliderEvent::Change(value) = event {
                        this.set_channel(index, value.end().round() as u8, window, cx);
                    }
                },
            ));
            subscriptions.push(cx.subscribe_in(
                &channel.input,
                window,
                move |this, input, event, window, cx| {
                    if let InputEvent::Change = event {
                        if let Ok(value) = input.read(cx).value().trim().parse::<u8>() {
                            this.set_channel(index, value, window, cx);
                        }
                    }
                },
            ));
        }
        subscriptions.push(
            cx.subscribe_in(&hex, window, |this, input, event, window, cx| {
                if let InputEvent::Change = event {
                    let text = input.read(cx).value();
                    if let Some((color, _)) = parse_hex_input(&text) {
                        this.commit(color, false, window, cx);
                    }
                }
            }),
        );
        Self {
            value: "#ffffff".to_string(),
            transparent: false,
            mode: Mode::Standard,
            hue: 0.,
            channels,
            hex,
            spectrum: Rc::default(),
            hue_bar: Rc::default(),
            drag: None,
            on_change: None,
            on_select: None,
            _subscriptions: subscriptions,
        }
    }

    /// Follows the owner's value. Editing an input is left alone, so typing is not overwritten mid-word.
    fn sync(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let transparent = is_transparent(value);
        let color = if transparent {
            "#ffffff".to_string()
        } else {
            normalize_hex(value, "#ffffff")
        };
        if color != self.value || transparent != self.transparent {
            self.value = color;
            self.transparent = transparent;
            self.update_inputs(window, cx);
        }
    }

    fn update_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rgb = hex_to_rgb(&self.value);
        let alpha = hex_alpha(&self.value);
        // A grey has no hue of its own, so the spectrum keeps the last one.
        if rgb.red != rgb.green || rgb.green != rgb.blue {
            self.hue = f64::from(rgb_to_hue(rgb));
        }
        let values = [rgb.red, rgb.green, rgb.blue, alpha];
        for (channel, value) in self.channels.iter().zip(values) {
            channel.slider.update(cx, |state, cx| {
                state.set_value(f32::from(value), window, cx)
            });
            let text = value.to_string();
            let unchanged = channel.input.read(cx).value() == text.as_str();
            if !unchanged {
                channel
                    .input
                    .update(cx, |state, cx| state.set_value(text, window, cx));
            }
        }
        let hex = format_propresenter_hex(&self.value, alpha);
        self.hex
            .update(cx, |state, cx| state.set_value(hex, window, cx));
    }

    fn set_channel(
        &mut self,
        index: usize,
        value: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut rgb = hex_to_rgb(&self.value);
        let mut alpha = hex_alpha(&self.value);
        match index {
            0 => rgb.red = value,
            1 => rgb.green = value,
            2 => rgb.blue = value,
            _ => alpha = value,
        }
        self.commit(rgb_to_hex(rgb, alpha), false, window, cx);
    }

    fn commit(
        &mut self,
        color: String,
        transparent: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if color == self.value && transparent == self.transparent {
            return;
        }
        self.value = color;
        self.transparent = transparent;
        self.update_inputs(window, cx);
        cx.notify();
        if let Some(handler) = self.on_change.clone() {
            let value = if transparent {
                "transparent".to_string()
            } else {
                self.value.clone()
            };
            handler(value, window, cx);
        }
    }

    fn drag_to(
        &mut self,
        drag: Drag,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let alpha = hex_alpha(&self.value);
        let rgb = match drag {
            Drag::Spectrum => {
                let (x, y) = fraction_within(self.spectrum.get(), position);
                hsv_to_rgb(self.hue, x * 100., (1. - y) * 100.)
            }
            Drag::Hue => {
                let (_, y) = fraction_within(self.hue_bar.get(), position);
                self.hue = y * 360.;
                let (saturation, value) = rgb_to_hsv(hex_to_rgb(&self.value));
                hsv_to_rgb(self.hue, f64::from(saturation), f64::from(value))
            }
        };
        self.commit(rgb_to_hex(rgb, alpha), false, window, cx);
        cx.notify();
    }

    fn finish_selection(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handler) = &self.on_select {
            handler((), window, cx);
        }
    }

    fn swatch(
        &self,
        id: ElementId,
        color: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = color == self.value && !self.transparent;
        let ring = cx.theme().colors.focus_ring;
        Button::new(id)
            .size(SWATCH)
            .bg(hex_color(color))
            .cursor_pointer()
            .accessibility_label(color)
            .debug_selector(move || format!("color-swatch-{color}"))
            .tooltip(tooltip_text(color))
            .when(selected, |this| this.border_2().border_color(ring))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.commit(color.to_string(), false, window, cx);
                this.finish_selection(window, cx);
            }))
    }

    fn standard(&self, t: Translator, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let heading = |key: &str| {
            div()
                .text_size(theme.metrics.font_xs)
                .text_color(theme.colors.text_muted)
                .child(t.t(key))
        };
        let grid = || div().flex().flex_wrap().gap(px(4.)).w(GRID_WIDTH);
        let available: Vec<_> = AVAILABLE_COLORS
            .iter()
            .enumerate()
            .map(|(index, color)| {
                self.swatch(
                    ElementId::Name(format!("available-{index}").into()),
                    color,
                    cx,
                )
            })
            .collect();
        let standard: Vec<_> = STANDARD_COLORS
            .iter()
            .enumerate()
            .map(|(index, color)| {
                self.swatch(
                    ElementId::Name(format!("standard-{index}").into()),
                    color,
                    cx,
                )
            })
            .collect();
        let no_background = t.t("colorPicker.noBackground");
        div()
            .flex()
            .flex_col()
            .gap(theme.metrics.space_xs)
            .p(px(4.))
            .child(heading("colorPicker.availableColors"))
            .child(grid().children(available))
            .child(heading("colorPicker.standardColors"))
            .child(
                grid()
                    .child(
                        Button::new("transparent-swatch")
                            .size(SWATCH)
                            .bg(rgb(0xffffff))
                            .bg(checkerboard(TRANSPARENT_CHECK, 4.))
                            .cursor_pointer()
                            .accessibility_label(no_background.clone())
                            .debug_selector(|| "color-swatch-transparent".to_string())
                            .tooltip(tooltip_text(no_background))
                            .when(self.transparent, |this| {
                                this.border_2().border_color(theme.colors.focus_ring)
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                let color = this.value.clone();
                                this.commit(color, true, window, cx);
                                this.finish_selection(window, cx);
                            })),
                    )
                    .children(standard),
            )
    }

    fn drag_tracker(
        &self,
        drag: Drag,
        bounds: Rc<Cell<Bounds<Pixels>>>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();
        canvas(
            move |area, _, _| bounds.set(area),
            move |_, _, window, _| {
                let moved = view.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase.bubble() && event.pressed_button == Some(MouseButton::Left) {
                        moved.update(cx, |this, cx| {
                            if this.drag == Some(drag) {
                                this.drag_to(drag, event.position, window, cx);
                            }
                        });
                    }
                });
                let released = view.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase.bubble() {
                        released.update(cx, |this, _| this.drag = None);
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }

    fn advanced(&self, t: Translator, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let rgb_value = hex_to_rgb(&self.value);
        let (saturation, value) = rgb_to_hsv(rgb_value);
        let alpha = hex_alpha(&self.value);
        let pure: Hsla = hex_color(&rgb_to_hex(hsv_to_rgb(self.hue, 100., 100.), 255)).into();
        let white: Hsla = rgb(0xffffff).into();
        let black: Hsla = rgb(0x000000).into();
        let transparent_black = Hsla { a: 0., ..black };
        let spectrum = div()
            .id("color-spectrum")
            .relative()
            .flex_1()
            .h(SPECTRUM_HEIGHT)
            .cursor_crosshair()
            .bg(linear_gradient(
                90.,
                linear_color_stop(white, 0.),
                linear_color_stop(pure, 1.),
            ))
            .tooltip(tooltip_text(t.t("colorPicker.spectrum")))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    this.drag = Some(Drag::Spectrum);
                    this.drag_to(Drag::Spectrum, event.position, window, cx);
                }),
            )
            .child(div().absolute().size_full().bg(linear_gradient(
                180.,
                linear_color_stop(transparent_black, 0.),
                linear_color_stop(black, 1.),
            )))
            .child(self.drag_tracker(Drag::Spectrum, self.spectrum.clone(), cx))
            .child(
                div()
                    .absolute()
                    .left(gpui::relative(saturation as f32 / 100.))
                    .top(gpui::relative(1. - value as f32 / 100.))
                    .size(px(8.))
                    .ml(px(-4.))
                    .mt(px(-4.))
                    .rounded(theme.metrics.radius_pill)
                    .border_1()
                    .border_color(white),
            );
        let segments: Vec<_> = (0..6)
            .map(|index| {
                let start = hex_color(&rgb_to_hex(
                    hsv_to_rgb(f64::from(index) * 60., 100., 100.),
                    255,
                ));
                let end = hex_color(&rgb_to_hex(
                    hsv_to_rgb(f64::from(index + 1) * 60., 100., 100.),
                    255,
                ));
                div().flex_1().w_full().bg(linear_gradient(
                    180.,
                    linear_color_stop(start, 0.),
                    linear_color_stop(end, 1.),
                ))
            })
            .collect();
        let hue_bar = div()
            .id("color-hue")
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(HUE_WIDTH)
            .h(SPECTRUM_HEIGHT)
            .cursor_pointer()
            .tooltip(tooltip_text(t.t("colorPicker.hue")))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    this.drag = Some(Drag::Hue);
                    this.drag_to(Drag::Hue, event.position, window, cx);
                }),
            )
            .children(segments)
            .child(self.drag_tracker(Drag::Hue, self.hue_bar.clone(), cx))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .w_full()
                    .h(px(2.))
                    .top(gpui::relative((self.hue / 360.) as f32))
                    .bg(white),
            );
        let channels: Vec<_> = self
            .channels
            .iter()
            .enumerate()
            .map(|(index, channel)| {
                let view = cx.entity();
                div()
                    .flex()
                    .items_center()
                    .gap(theme.metrics.space_xs)
                    .child(
                        div()
                            .flex_none()
                            .w(px(12.))
                            .text_size(theme.metrics.font_xs)
                            .child(channel.label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(super::RangeInput::new(&channel.slider).label(channel.label)),
                    )
                    .child(
                        div().flex_none().w(px(64.)).child(
                            NumberInput::new(
                                ElementId::Name(format!("color-channel-{index}").into()),
                                &channel.input,
                            )
                            .label(channel.label)
                            .range(0., 255.)
                            .stepper()
                            .on_step(move |value, window, cx| {
                                view.update(cx, |this, cx| {
                                    this.set_channel(index, value as u8, window, cx)
                                });
                            }),
                        ),
                    )
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(theme.metrics.space_xs)
            .p(px(4.))
            .child(
                div()
                    .text_size(theme.metrics.font_xs)
                    .text_color(theme.colors.text_muted)
                    .child(t.t("colorPicker.advancedColor")),
            )
            .child(
                div()
                    .flex()
                    .gap(theme.metrics.space_xs)
                    .child(spectrum)
                    .child(hue_bar),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.metrics.space_xs)
                    .child(
                        div()
                            .flex_none()
                            .size(theme.metrics.control_height_compact)
                            .bg(with_alpha(hex_color(&self.value), alpha)),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            TextInput::new("color-hex", &self.hex)
                                .compact()
                                .label(t.t("colorPicker.hexColor")),
                        ),
                    ),
            )
            .children(channels)
            .when(self.on_select.is_some(), |panel| {
                panel.child(
                    super::ActionButton::new("color-picker-done", t.t("common.done"))
                        .compact()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.finish_selection(window, cx);
                        })),
                )
            })
    }
}

impl Render for ColorPopover {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let tab = |id: &'static str, label: String, mode: Mode, cx: &mut Context<Self>| {
            let active = self.mode == mode;
            Button::new(id)
                .selected(active)
                .h(px(22.))
                .px(px(6.))
                .rounded(theme.metrics.radius_sm)
                .text_size(theme.metrics.font_sm)
                .text_color(theme.colors.text)
                .styles(|styles| {
                    styles.selected(|style| {
                        style
                            .bg(theme.colors.selected)
                            .text_color(theme.colors.accent_text)
                    })
                })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mode = mode;
                    cx.notify();
                }))
                .child(label)
        };
        let standard_tab = tab(
            "color-mode-standard",
            t.t("colorPicker.standard"),
            Mode::Standard,
            cx,
        );
        let advanced_tab = tab(
            "color-mode-advanced",
            t.t("colorPicker.advanced"),
            Mode::Advanced,
            cx,
        );
        div()
            .id("color-popover")
            .debug_selector(|| "color-popover".into())
            .occlude()
            .flex()
            .flex_col()
            .w(POPOVER_WIDTH)
            .p(px(4.))
            .border_1()
            .border_color(theme.colors.border)
            .bg(theme.colors.popover_bg)
            .text_color(theme.colors.text)
            .shadow(theme.shadow_soft())
            .child(
                div()
                    .flex()
                    .gap(px(1.))
                    .child(standard_tab)
                    .child(advanced_tab),
            )
            .map(|this| match self.mode {
                Mode::Standard => this.child(self.standard(t, cx)).into_any_element(),
                Mode::Advanced => this.child(self.advanced(t, cx)).into_any_element(),
            })
    }
}

/// A swatch that opens the color picker. Values are `#rrggbb`, `#rrggbbaa`, or `transparent`.
#[derive(IntoElement)]
pub struct ColorPicker {
    id: ElementId,
    label: SharedString,
    value: String,
    disabled: bool,
    on_change: Option<Handler<String>>,
    anchor: gpui::Anchor,
    close_on_selection: bool,
    compact: bool,
}

impl ColorPicker {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value: value.into(),
            disabled: false,
            on_change: None,
            anchor: gpui::Anchor::TopLeft,
            close_on_selection: false,
            compact: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn anchor(mut self, anchor: gpui::Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Standard swatch width for compact inspector rows and toolbars.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Closes after a swatch choice, and adds a Done action to the advanced editor.
    pub fn close_on_selection(mut self) -> Self {
        self.close_on_selection = true;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(String, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for ColorPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let open = window.use_keyed_state(child_id(&self.id, "open"), cx, |_, _| false);
        // Where the trigger is, so that pressing it while open toggles instead of closing and reopening.
        let trigger_bounds = window
            .use_keyed_state(child_id(&self.id, "trigger-bounds"), cx, |_, _| {
                Rc::new(Cell::new(Bounds::<Pixels>::default()))
            })
            .read(cx)
            .clone();
        let (placed, pressed) = (trigger_bounds.clone(), trigger_bounds);
        let popover = window.use_keyed_state(child_id(&self.id, "popover"), cx, ColorPopover::new);
        let value = self.value.clone();
        let on_change = self.on_change.clone();
        let finish = open.clone();
        let on_select: Option<Handler<()>> = self.close_on_selection.then(|| {
            Rc::new(move |_, _: &mut Window, cx: &mut App| {
                finish.update(cx, |open, cx| {
                    *open = false;
                    cx.notify();
                });
            }) as Handler<()>
        });
        popover.update(cx, |popover, cx| {
            popover.on_change = on_change;
            popover.on_select = on_select;
            popover.sync(&value, window, cx);
        });
        let is_open = *open.read(cx);
        let transparent = is_transparent(&self.value);
        let color = normalize_hex(&self.value, "#ffffff");
        let toggle = open.clone();
        let close = open;
        let trigger = Button::new(child_id(&self.id, "trigger"))
            .disabled(self.disabled)
            .selected(is_open)
            .accessibility_label(self.label)
            .justify_between()
            .w_full()
            .h(theme.metrics.control_height_compact)
            .pl(px(1.))
            .border_1()
            .border_color(theme.colors.control_border)
            .rounded(theme.metrics.radius_sm)
            .bg(theme.colors.control_bg)
            .text_color(theme.colors.text)
            .when(self.disabled, |this| this.opacity(0.55))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .focus_visible(|style| focus_ring(style, cx))
                    .on_click(move |_, _, cx| {
                        toggle.update(cx, |open, cx| {
                            *open = !*open;
                            cx.notify();
                        })
                    })
            })
            .child(div().flex_1().h(px(18.)).map(|this| {
                if transparent {
                    this.bg(rgb(0xffffff))
                        .bg(checkerboard(TRANSPARENT_CHECK, 4.))
                } else {
                    this.bg(hex_color(&color))
                }
            }))
            .child(
                icon("chevron-down")
                    .size(theme.metrics.space_lg)
                    .mx(theme.metrics.space_2xs)
                    .text_color(theme.colors.text),
            );
        Popup::new(
            child_id(&self.id, "popup"),
            div()
                .w_full()
                .when(self.compact, |this| {
                    this.w(theme.metrics.control_height_compact * 2.5)
                        .flex_none()
                })
                .on_prepaint(move |bounds, _, _| placed.set(bounds))
                .child(trigger),
        )
        .anchor(self.anchor)
        .offset(theme.metrics.space_2xs)
        .when(is_open, |this| {
            this.content(
                deferred(
                    div()
                        .on_mouse_down_out(move |event, _, cx| {
                            if !pressed.get().contains(&event.position) {
                                close.update(cx, |open, cx| {
                                    *open = false;
                                    cx.notify();
                                })
                            }
                        })
                        .child(popover),
                )
                .priority(gpui_base::POPUP_PRIORITY),
            )
        })
    }
}
