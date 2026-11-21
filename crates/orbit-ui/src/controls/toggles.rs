use std::rc::Rc;

use gpui::{div, prelude::*, px, App, ElementId, Pixels, SharedString, Window};
use gpui_base::{Checkbox, CheckboxState, Switch};

use super::{focus_ring, Handler};
use crate::{icon, ActiveTheme};

/// An immediate on/off, unlike [`CheckboxField`], which belongs in forms.
#[derive(IntoElement)]
pub struct ToggleSwitch {
    id: ElementId,
    label: SharedString,
    checked: bool,
    disabled: bool,
    bare: bool,
    label_size: Option<Pixels>,
    on_change: Option<Handler<bool>>,
}

impl ToggleSwitch {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            checked,
            disabled: false,
            bare: false,
            label_size: None,
            on_change: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Only the switch is drawn; the label stays its accessible name, as when a neighbouring text names it.
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }

    pub fn label_size(mut self, size: Pixels) -> Self {
        self.label_size = Some(size);
        self
    }

    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for ToggleSwitch {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let radius = theme.metrics.radius_pill;
        let label = self.label.clone();
        let row_id = format!("toggle-row-{}", self.id);
        let on_change = self.on_change.clone();
        let checked = self.checked;
        let disabled = self.disabled;
        let switch = Switch::new(self.id)
            .checked(self.checked)
            .disabled(self.disabled)
            .accessibility_label(self.label)
            .flex()
            .flex_none()
            .items_center()
            .w(px(34.))
            .h(px(20.))
            .p(px(2.))
            .rounded(radius)
            .border_1()
            .border_color(if self.checked {
                theme.colors.success
            } else {
                theme.colors.border
            })
            .bg(if self.checked {
                theme.colors.success
            } else {
                theme.colors.panel_bg_soft
            })
            .when(self.checked, |this| this.justify_end())
            .when(self.disabled, |this| this.opacity(0.55))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .focus_visible(|style| focus_ring(style, cx))
            })
            .when_some(self.on_change, |this, handler| {
                this.on_change(move |checked, _, window, cx| {
                    cx.stop_propagation();
                    handler(checked, window, cx);
                })
            })
            .child(
                div()
                    .size(px(16.))
                    .rounded(radius)
                    .bg(theme.colors.text_muted),
            );
        let (bare, label_size) = (self.bare, self.label_size);
        div()
            .id(row_id)
            .flex()
            .items_center()
            .gap(theme.metrics.space_sm)
            .text_size(label_size.unwrap_or(theme.metrics.font_md))
            .text_color(theme.colors.text)
            .when(!disabled, |row| {
                row.cursor_pointer().when_some(on_change, |row, handler| {
                    row.on_click(move |_, window, cx| handler(!checked, window, cx))
                })
            })
            .child(switch)
            .when(!bare, |row| row.child(label))
    }
}

#[derive(IntoElement)]
pub struct CheckboxField {
    id: ElementId,
    label: SharedString,
    checked: bool,
    bare: bool,
    mixed: bool,
    disabled: bool,
    label_style: Option<(Pixels, gpui::FontWeight)>,
    on_change: Option<Handler<bool>>,
}

impl CheckboxField {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            checked,
            bare: false,
            mixed: false,
            disabled: false,
            label_style: None,
            on_change: None,
        }
    }

    /// A compact checkbox whose label is retained as its accessible name.
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Some of the covered items are checked; a click checks all of them.
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    pub fn label_style(mut self, size: Pixels, weight: gpui::FontWeight) -> Self {
        self.label_style = Some((size, weight));
        self
    }

    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for CheckboxField {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        Checkbox::new(self.id)
            .checked(self.checked)
            .disabled(self.disabled)
            .accessibility_label(self.label.clone())
            .flex()
            .items_center()
            .gap(theme.metrics.space_snug)
            .min_h(if self.bare {
                theme.metrics.control_height_compact
            } else {
                theme.metrics.control_height
            })
            .min_w_0()
            .text_size(
                self.label_style
                    .map_or(theme.metrics.font_md, |(size, _)| size),
            )
            .when_some(self.label_style, |this, (_, weight)| {
                this.font_weight(weight)
            })
            .text_color(theme.colors.text)
            .when(self.disabled, |this| this.opacity(0.55))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .focus_visible(|style| focus_ring(style, cx))
            })
            .when_some(self.on_change, |this, handler| {
                this.on_change(move |state, _, window, cx| {
                    handler(state == CheckboxState::Checked, window, cx)
                })
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(16.))
                    .rounded(theme.metrics.radius_xs)
                    .border_1()
                    .border_color(theme.colors.control_border)
                    .bg(if self.checked || self.mixed {
                        theme.colors.focus_ring
                    } else {
                        theme.colors.control_bg
                    })
                    .when(self.checked || self.mixed, |this| {
                        this.child(
                            icon(if self.checked { "check" } else { "minus" })
                                .size(px(12.))
                                .text_color(theme.colors.accent_text),
                        )
                    }),
            )
            .when(!self.bare, |this| this.child(self.label))
    }
}
