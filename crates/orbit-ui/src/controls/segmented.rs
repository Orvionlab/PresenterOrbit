use std::rc::Rc;

use gpui::{div, prelude::*, App, ElementId, Role, SharedString, Window};
use gpui_base::Button;

use super::{child_id, focus_ring, tooltip_text, Handler};
use crate::{icon, ActiveTheme};

#[derive(Clone)]
pub struct SegmentedOption<T> {
    pub value: T,
    pub label: Option<SharedString>,
    pub icon: Option<SharedString>,
    pub title: Option<SharedString>,
    pub disabled: bool,
}

impl<T> SegmentedOption<T> {
    pub fn text(value: T, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: Some(label.into()),
            icon: None,
            title: None,
            disabled: false,
        }
    }

    pub fn icon(value: T, name: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: None,
            icon: Some(name.into()),
            title: Some(title.into()),
            disabled: false,
        }
    }
}

/// An exclusive choice shown as a row of buttons.
#[derive(IntoElement)]
pub struct SegmentedControl<T: Clone + PartialEq + 'static> {
    id: ElementId,
    label: SharedString,
    options: Vec<SegmentedOption<T>>,
    value: T,
    on_change: Option<Handler<T>>,
}

impl<T: Clone + PartialEq + 'static> SegmentedControl<T> {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        options: Vec<SegmentedOption<T>>,
        value: T,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            options,
            value,
            on_change: None,
        }
    }

    pub fn on_change(mut self, handler: impl Fn(T, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl<T: Clone + PartialEq + 'static> RenderOnce for SegmentedControl<T> {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let buttons = self.options.into_iter().enumerate().map(|(index, option)| {
            let active = option.value == self.value;
            let accessible = option
                .label
                .clone()
                .or_else(|| option.title.clone())
                .unwrap_or_default();
            Button::new(child_id(&self.id, index.to_string()))
                .selected(active)
                .disabled(option.disabled)
                .accessibility_label(accessible)
                .gap(theme.metrics.space_xs)
                .min_w(theme.metrics.control_height_compact)
                .h(theme.metrics.control_height_compact)
                .px(theme.metrics.space_sm)
                .rounded(theme.metrics.radius_sm)
                .text_size(theme.metrics.font_sm)
                .text_color(theme.colors.text_muted)
                .styles(|styles| {
                    styles
                        .selected(|style| {
                            style
                                .bg(theme.colors.selected)
                                .text_color(theme.colors.accent_text)
                        })
                        .disabled(|style| style.text_color(theme.colors.disabled_text))
                })
                .when(!option.disabled, |this| {
                    this.cursor_pointer()
                        .when(!active, |this| {
                            this.hover(|style| {
                                style
                                    .bg(theme.colors.accent_soft)
                                    .text_color(theme.colors.text_strong)
                            })
                        })
                        .focus_visible(|style| focus_ring(style, cx))
                })
                .when_some(option.title, |this, title| {
                    this.tooltip(tooltip_text(title))
                })
                .when_some(self.on_change.clone(), |this, handler| {
                    let value = option.value.clone();
                    this.on_click(move |_, window, cx| handler(value.clone(), window, cx))
                })
                .when_some(option.icon, |this, name| {
                    this.child(icon(&name).text_color(if active {
                        theme.colors.accent_text
                    } else {
                        theme.colors.text_muted
                    }))
                })
                .when_some(option.label, |this, label| this.child(label))
        });
        div()
            .id(self.id.clone())
            .role(Role::Group)
            .aria_label(self.label)
            .flex()
            .items_center()
            .gap(theme.metrics.space_2xs)
            .children(buttons.collect::<Vec<_>>())
    }
}
