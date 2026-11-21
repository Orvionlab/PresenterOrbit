use std::rc::Rc;

use gpui::{
    deferred, div, prelude::*, px, Anchor, App, Bounds, ElementId, MouseButton, Pixels, Role,
    SharedString, Window,
};
use gpui_base::actions::{SelectDown, SelectUp};
use gpui_base::{ElementExt, Popup};

use super::{child_id, focus_ring, Handler};
use crate::{icon, ActiveTheme};

const MAX_MENU_HEIGHT: Pixels = px(320.);

#[derive(Clone)]
pub struct SelectOption<T> {
    pub value: T,
    pub label: SharedString,
    pub disabled: bool,
    preview_icon: Option<SharedString>,
}

impl<T> SelectOption<T> {
    pub fn new(value: T, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
            disabled: false,
            preview_icon: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    /// Show a graphical sample instead of text, retaining the label for accessibility.
    pub fn preview_icon(mut self, name: impl Into<SharedString>) -> Self {
        self.preview_icon = Some(name.into());
        self
    }
}

/// The next enabled option in a direction, staying put at the ends, as a native select does.
pub fn next_enabled(disabled: &[bool], current: usize, forward: bool) -> usize {
    let mut index = current;
    loop {
        index = if forward {
            index + 1
        } else if index == 0 {
            return current;
        } else {
            index - 1
        };
        match disabled.get(index) {
            None => return current,
            Some(false) => return index,
            Some(true) => {}
        }
    }
}

#[derive(Default)]
struct SelectState {
    open: bool,
    highlighted: usize,
    /// Where the trigger is, so that pressing it while open toggles instead of closing and reopening.
    trigger: Bounds<Pixels>,
}

/// A dropdown choice. The owner keeps the value.
#[derive(IntoElement)]
pub struct Select<T: Clone + PartialEq + 'static> {
    id: ElementId,
    options: Vec<SelectOption<T>>,
    value: Option<T>,
    label: Option<SharedString>,
    disabled: bool,
    compact: bool,
    on_change: Option<Handler<T>>,
}

impl<T: Clone + PartialEq + 'static> Select<T> {
    pub fn new(
        id: impl Into<ElementId>,
        options: Vec<SelectOption<T>>,
        value: impl Into<Option<T>>,
    ) -> Self {
        Self {
            id: id.into(),
            options,
            value: value.into(),
            label: None,
            disabled: false,
            compact: false,
            on_change: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The inspector and toolbar size.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(T, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl<T: Clone + PartialEq + 'static> RenderOnce for Select<T> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| {
            SelectState::default()
        });
        let trigger_focus = window
            .use_keyed_state(child_id(&self.id, "focus"), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let selected = self
            .options
            .iter()
            .position(|option| Some(&option.value) == self.value.as_ref());
        let current = selected.map(|index| self.options[index].label.clone());
        let current_icon = selected.and_then(|index| self.options[index].preview_icon.clone());
        let open = state.read(cx).open;
        let highlighted = state.read(cx).highlighted;
        let disabled_flags: Rc<Vec<bool>> =
            Rc::new(self.options.iter().map(|option| option.disabled).collect());
        let values: Rc<Vec<T>> = Rc::new(
            self.options
                .iter()
                .map(|option| option.value.clone())
                .collect(),
        );

        let height = if self.compact {
            theme.metrics.control_height_compact
        } else {
            theme.metrics.control_height
        };
        let (press_state, place_state, press_focus) =
            (state.clone(), state.clone(), trigger_focus.clone());
        let disabled = self.disabled;
        let selector = format!("select-{}-trigger", self.id);
        let trigger = div()
            .id(child_id(&self.id, "trigger"))
            .debug_selector(move || selector.clone())
            .on_prepaint(move |bounds, _, cx| {
                if place_state.read(cx).trigger != bounds {
                    place_state.update(cx, |state, _| state.trigger = bounds);
                }
            })
            .when(!disabled, |this| {
                this.on_click(move |_, window, cx| {
                    press_focus.focus(window, cx);
                    press_state.update(cx, |state, cx| {
                        state.open = !state.open;
                        state.highlighted = selected.unwrap_or_default();
                        cx.notify();
                    });
                })
            })
            .flex()
            .items_center()
            .justify_between()
            .gap(theme.metrics.space_xs)
            .w_full()
            .min_w_0()
            .h(height)
            .px(if self.compact {
                theme.metrics.space_snug
            } else {
                theme.metrics.field_padding_x
            })
            .border_1()
            .border_color(theme.colors.control_border)
            .rounded(if self.compact {
                theme.metrics.radius_sm
            } else {
                theme.metrics.radius_md
            })
            .bg(theme.colors.control_bg)
            .text_size(if self.compact {
                theme.metrics.font_sm
            } else {
                theme.metrics.font_md
            })
            .text_color(theme.colors.text)
            .when(trigger_focus.is_focused(window) && !open, |this| {
                focus_ring(this, cx)
            })
            .child(div().flex_1().min_w_0().truncate().map(|label| {
                if let Some(name) = current_icon {
                    label.child(icon(&name).w_full())
                } else {
                    label.children(current.clone())
                }
            }))
            .child(icon("chevron-down").text_color(theme.colors.text));

        let menu = open.then(|| {
            let rows = self.options.iter().enumerate().map(|(index, option)| {
                let chosen = Some(index) == selected;
                let close_state = state.clone();
                let handler = self.on_change.clone();
                let value = option.value.clone();
                let focus = trigger_focus.clone();
                div()
                    .id(child_id(&self.id, format!("option-{index}")))
                    .role(Role::ListBoxOption)
                    .aria_label(option.label.clone())
                    .debug_selector(move || format!("select-option-{index}"))
                    .flex()
                    .items_center()
                    .min_h(theme.metrics.row_height)
                    .px(theme.metrics.space_sm)
                    .text_size(theme.metrics.font_sm)
                    .text_color(if option.disabled {
                        theme.colors.disabled_text
                    } else {
                        theme.colors.text
                    })
                    .when(chosen, |this| this.bg(theme.colors.selected))
                    .when(index == highlighted && !option.disabled, |this| {
                        this.bg(theme.colors.accent_hover)
                            .text_color(theme.colors.accent_text)
                    })
                    .when(!option.disabled, |this| {
                        this.cursor_pointer()
                            .hover(|style| {
                                style
                                    .bg(theme.colors.accent_hover)
                                    .text_color(theme.colors.accent_text)
                            })
                            .on_click(move |_, window, cx| {
                                close_state.update(cx, |state, cx| {
                                    state.open = false;
                                    cx.notify();
                                });
                                focus.focus(window, cx);
                                if let Some(handler) = &handler {
                                    handler(value.clone(), window, cx);
                                }
                            })
                    })
                    .child(if let Some(name) = &option.preview_icon {
                        div().flex_1().child(icon(name).w_full())
                    } else {
                        div().child(option.label.clone())
                    })
            });
            let close_state = state.clone();
            deferred(
                div()
                    .id(child_id(&self.id, "menu"))
                    .role(Role::ListBox)
                    .occlude()
                    .flex()
                    .flex_col()
                    .min_w(px(120.))
                    .max_h(MAX_MENU_HEIGHT)
                    .overflow_y_scroll()
                    .py(theme.metrics.space_2xs)
                    .border_1()
                    .border_color(theme.colors.border)
                    .bg(theme.colors.popover_bg)
                    .shadow(theme.shadow_soft())
                    .on_mouse_down_out(move |event, _, cx| {
                        close_state.update(cx, |state, cx| {
                            if !state.trigger.contains(&event.position) {
                                state.open = false;
                                cx.notify();
                            }
                        });
                    })
                    .children(rows.collect::<Vec<_>>()),
            )
            .priority(gpui_base::POPUP_PRIORITY)
        });

        let toggle_state = state.clone();
        let accessible_value = current.unwrap_or_default();
        let (up, down) = (state.clone(), state.clone());
        let (up_flags, down_flags) = (disabled_flags.clone(), disabled_flags.clone());
        let dismiss_state = state.clone();
        let confirm_state = state;
        let confirm_handler = self.on_change.clone();
        let confirm_focus = trigger_focus.clone();
        let mut root = gpui_base::Select::new(self.id.clone())
            .open(open)
            .disabled(self.disabled)
            .focus_handle(&trigger_focus)
            .accessibility_value(accessible_value)
            .on_dismiss(move |_, cx| {
                dismiss_state.update(cx, |state, cx| {
                    state.open = false;
                    cx.notify();
                })
            })
            .on_open_change(move |open, _, cx| {
                toggle_state.update(cx, |state, cx| {
                    state.open = open;
                    state.highlighted = selected.unwrap_or_default();
                    cx.notify();
                })
            })
            .on_confirm(move |window, cx| {
                let index = confirm_state.read(cx).highlighted;
                confirm_state.update(cx, |state, cx| {
                    state.open = false;
                    cx.notify();
                });
                confirm_focus.focus(window, cx);
                if let (Some(handler), Some(value)) = (&confirm_handler, values.get(index)) {
                    handler(value.clone(), window, cx);
                }
            });
        if let Some(label) = self.label {
            root = root.accessibility_label(label);
        }
        div()
            .w_full()
            .min_w_0()
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            })
            .when(self.disabled, |this| this.opacity(0.55))
            .on_action(move |_: &SelectUp, _, cx| {
                up.update(cx, |state, cx| {
                    state.highlighted = next_enabled(&up_flags, state.highlighted, false);
                    cx.notify();
                })
            })
            .on_action(move |_: &SelectDown, _, cx| {
                down.update(cx, |state, cx| {
                    state.highlighted = next_enabled(&down_flags, state.highlighted, true);
                    cx.notify();
                })
            })
            .child(
                root.child(
                    Popup::new(child_id(&self.id, "popup"), trigger)
                        .anchor(Anchor::TopLeft)
                        .offset(theme.metrics.space_2xs)
                        .when_some(menu, |this, menu| this.content(menu)),
                ),
            )
    }
}
