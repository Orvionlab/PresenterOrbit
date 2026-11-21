use std::rc::Rc;
use std::time::Duration;

use gpui::{
    div, prelude::*, App, ElementId, Entity, Focusable, MouseButton, SharedString, Task, Window,
};
use gpui_base::input::{Input, InputState};

use super::fields::input_frame;
use super::{child_id, Handler};
use crate::{icon, ActiveTheme};

const HOLD_DELAY: Duration = Duration::from_millis(320);
const REPEAT_INTERVAL: Duration = Duration::from_millis(70);

/// Mirrors `stepNumberInput`: an unparsable value starts from `min`, or 0 without one. The result is clamped
/// and keeps as many decimals as `step` has.
pub fn step_number(
    current: &str,
    direction: i32,
    step: f64,
    min: Option<f64>,
    max: Option<f64>,
) -> String {
    let step = if step.is_finite() && step > 0. {
        step
    } else {
        1.
    };
    let start = current
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .unwrap_or_else(|| min.unwrap_or(0.));
    let next = start + step * f64::from(direction);
    let next = max.map_or(next, |max| next.min(max));
    let next = min.map_or(next, |min| next.max(min));
    match step.to_string().split_once('.') {
        Some((_, decimals)) => format!("{next:.*}", decimals.len()),
        None => format!("{}", next.round() as i64),
    }
}

/// A numeric text field. With `stepper` it gets chevron buttons that repeat while held.
#[derive(IntoElement)]
pub struct NumberInput {
    id: ElementId,
    state: Entity<InputState>,
    label: Option<SharedString>,
    suffix: Option<SharedString>,
    step: f64,
    min: Option<f64>,
    max: Option<f64>,
    stepper: bool,
    disabled: bool,
    on_step: Option<Handler<f64>>,
}

impl NumberInput {
    pub fn new(id: impl Into<ElementId>, state: &Entity<InputState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            label: None,
            suffix: None,
            step: 1.,
            min: None,
            max: None,
            stepper: false,
            disabled: false,
            on_step: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
    /// A visible unit that leaves the editable value numeric.
    pub fn suffix(mut self, suffix: impl Into<SharedString>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }

    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }

    pub fn stepper(mut self) -> Self {
        self.stepper = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called with the new value after a stepper press changes the field.
    pub fn on_step(mut self, handler: impl Fn(f64, &mut Window, &mut App) + 'static) -> Self {
        self.on_step = Some(Rc::new(handler));
        self
    }
}

#[derive(Clone)]
struct StepAction {
    state: Entity<InputState>,
    step: f64,
    min: Option<f64>,
    max: Option<f64>,
    on_step: Option<Handler<f64>>,
}

impl StepAction {
    fn run(&self, direction: i32, window: &mut Window, cx: &mut App) {
        let current = self.state.read(cx).value();
        let next = step_number(&current, direction, self.step, self.min, self.max);
        let value = next.parse::<f64>().unwrap_or_default();
        self.state
            .update(cx, |state, cx| state.set_value(next, window, cx));
        if let Some(handler) = &self.on_step {
            handler(value, window, cx);
        }
    }
}

fn stepper_button(
    id: ElementId,
    name: &'static str,
    direction: i32,
    action: StepAction,
    disabled: bool,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let theme = *cx.theme();
    let hold = window.use_keyed_state(id.clone(), cx, |_, _| None::<Task<()>>);
    let stop = hold.clone();
    let release = move |_: &gpui::MouseUpEvent, _: &mut Window, cx: &mut App| {
        stop.update(cx, |task, _| *task = None);
    };
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(theme.colors.control_border)
        .bg(theme.colors.control_bg)
        .text_color(if disabled {
            theme.colors.disabled_text
        } else {
            theme.colors.text_strong
        })
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(theme.colors.accent_soft))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    window.prevent_default();
                    action.run(direction, window, cx);
                    let repeat = action.clone();
                    let handle = window.window_handle();
                    let task = cx.spawn(async move |cx| {
                        cx.background_executor().timer(HOLD_DELAY).await;
                        loop {
                            let stepped = handle.update(cx, |_, window, cx| {
                                repeat.run(direction, window, cx);
                            });
                            if stepped.is_err() {
                                return;
                            }
                            cx.background_executor().timer(REPEAT_INTERVAL).await;
                        }
                    });
                    hold.update(cx, |current, _| *current = Some(task));
                })
                .on_mouse_up(MouseButton::Left, release.clone())
                .on_mouse_up_out(MouseButton::Left, release)
        })
        .child(
            icon(name)
                .size(theme.metrics.space_md)
                .text_color(if disabled {
                    theme.colors.disabled_text
                } else {
                    theme.colors.text_strong
                }),
        )
}

impl RenderOnce for NumberInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let field = input_frame(
            self.id.clone(),
            self.label,
            focused,
            self.disabled,
            true,
            cx,
        )
        .h(theme.metrics.control_height_compact)
        .child(div().flex_1().min_w_0().child(Input::new(&self.state)))
        .when_some(self.suffix, |field, suffix| {
            field.child(
                div()
                    .flex_none()
                    .text_size(theme.metrics.font_sm)
                    .text_color(theme.colors.text_muted)
                    .child(suffix),
            )
        });
        if !self.stepper {
            return field.into_any_element();
        }
        let action = StepAction {
            state: self.state.clone(),
            step: self.step,
            min: self.min,
            max: self.max,
            on_step: self.on_step,
        };
        let up = stepper_button(
            child_id(&self.id, "up"),
            "chevron-up",
            1,
            action.clone(),
            self.disabled,
            window,
            cx,
        );
        let down = stepper_button(
            child_id(&self.id, "down"),
            "chevron-down",
            -1,
            action,
            self.disabled,
            window,
            cx,
        );
        div()
            .flex()
            .w_full()
            .min_w_0()
            .h(theme.metrics.control_height_compact)
            .child(div().flex_1().min_w_0().child(field))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .w(theme.metrics.space_lg)
                    .h_full()
                    .child(up)
                    .child(down),
            )
            .into_any_element()
    }
}
