use std::{cell::Cell, rc::Rc};

use gpui::{
    canvas, div, point, prelude::*, size, App, Bounds, ElementId, MouseButton, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, SharedString, Window,
};

use super::{tooltip_text, Handler};
use crate::ActiveTheme;

/// A pointer dial for a degree value. Pair it with a number field for keyboard entry.
#[derive(IntoElement)]
pub struct AngleDial {
    id: ElementId,
    label: SharedString,
    degrees: f64,
    disabled: bool,
    on_change: Option<Handler<f64>>,
}

impl AngleDial {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, degrees: f64) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            degrees,
            disabled: false,
            on_change: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(&f64, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(move |angle, window, cx| {
            handler(&angle, window, cx)
        }));
        self
    }
}

fn drag_to(
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    handler: &Option<Handler<f64>>,
    window: &mut Window,
    cx: &mut App,
) {
    let offset = position - bounds.center();
    let (x, y) = (f64::from(offset.x), f64::from(offset.y));
    if let Some(handler) = handler.as_ref().filter(|_| x.hypot(y) >= 1.) {
        handler(
            x.atan2(-y).to_degrees().round().rem_euclid(360.),
            window,
            cx,
        );
    }
}

impl RenderOnce for AngleDial {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let selector = self.id.to_string();
        let held = window.use_keyed_state(self.id.clone(), cx, |_, _| false);
        let pressed = held.clone();
        let area = Rc::new(Cell::new(Bounds::default()));
        let measured = area.clone();
        let changed = self.on_change.clone();
        let disabled = self.disabled;
        let (sin, cos) = self.degrees.to_radians().sin_cos();
        div()
            .id(self.id)
            .debug_selector(move || selector.clone())
            .tooltip(tooltip_text(self.label))
            .size(theme.metrics.control_height_compact)
            .flex_none()
            .rounded(theme.metrics.radius_pill)
            .bg(theme.colors.control_bg)
            .border_1()
            .border_color(theme.colors.control_border)
            .when(disabled, |el| el.opacity(0.55))
            .when(!disabled, |el| {
                el.cursor_pointer()
                    .hover(|style| style.bg(theme.colors.control_bg_hover))
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        pressed.update(cx, |held, _| *held = true);
                        drag_to(event.position, area.get(), &changed, window, cx);
                    })
            })
            .child(
                canvas(
                    move |bounds, _, _| measured.set(bounds),
                    move |bounds, _, window, _| {
                        let dot = theme.metrics.space_xs;
                        let radius = bounds.size.width * 0.29;
                        let center =
                            bounds.center() + point(radius * sin as f32, -radius * cos as f32);
                        window.paint_quad(
                            gpui::fill(
                                Bounds {
                                    origin: center - point(dot / 2., dot / 2.),
                                    size: size(dot, dot),
                                },
                                theme.colors.text,
                            )
                            .corner_radii(dot / 2.),
                        );
                        if !disabled {
                            let drag = held.clone();
                            let release = held.clone();
                            let changed = self.on_change.clone();
                            window.on_mouse_event(
                                move |event: &MouseMoveEvent, phase, window, cx| {
                                    if phase.bubble()
                                        && *drag.read(cx)
                                        && event.pressed_button == Some(MouseButton::Left)
                                    {
                                        drag_to(event.position, bounds, &changed, window, cx);
                                    }
                                },
                            );
                            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                                if phase.bubble()
                                    && event.button == MouseButton::Left
                                    && *release.read(cx)
                                {
                                    release.update(cx, |held, _| *held = false);
                                }
                            });
                        }
                    },
                )
                .size_full(),
            )
    }
}
