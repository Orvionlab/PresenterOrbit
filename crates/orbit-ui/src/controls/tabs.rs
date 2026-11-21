use std::rc::Rc;

use gpui::{div, prelude::*, App, ElementId, FocusHandle, Role, SharedString, Window};
use gpui_base::Button;

use super::{child_id, focus_ring, tooltip_text, Handler};
use crate::{icon, ActiveTheme};

/// Which tab a key moves to, wrapping at the ends, as `handleTabKeyDown` does. `None` for any other key.
pub fn tab_key_target(key: &str, index: usize, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match key {
        "right" => Some((index + 1) % count),
        "left" => Some((index + count - 1) % count),
        "home" => Some(0),
        "end" => Some(count - 1),
        _ => None,
    }
}

#[derive(Clone)]
pub struct TabItem<T> {
    pub value: T,
    pub label: SharedString,
    pub icon: Option<SharedString>,
    pub disabled: bool,
    pub show_label: bool,
}

impl<T> TabItem<T> {
    pub fn new(value: T, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
            icon: None,
            disabled: false,
            show_label: false,
        }
    }

    /// An icon-only tab; the label stays as its accessible name and tooltip.
    pub fn icon(mut self, name: impl Into<SharedString>) -> Self {
        self.icon = Some(name.into());
        self
    }

    /// A leading icon and visible text, as the inspector tab strips.
    pub fn leading_icon(mut self, name: impl Into<SharedString>) -> Self {
        self.icon = Some(name.into());
        self.show_label = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// Tabs with roving focus: arrow keys, Home, and End move and activate.
#[derive(IntoElement)]
pub struct TabList<T: Clone + PartialEq + 'static> {
    id: ElementId,
    label: SharedString,
    tabs: Vec<TabItem<T>>,
    active: T,
    on_change: Option<Handler<T>>,
    equal_width: bool,
}

impl<T: Clone + PartialEq + 'static> TabList<T> {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        tabs: Vec<TabItem<T>>,
        active: T,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tabs,
            active,
            on_change: None,
            equal_width: false,
        }
    }

    /// Divide the available width equally among the tabs.
    pub fn equal_width(mut self) -> Self {
        self.equal_width = true;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(T, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl<T: Clone + PartialEq + 'static> RenderOnce for TabList<T> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let enabled: Rc<Vec<(usize, T)>> = Rc::new(
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, tab)| !tab.disabled)
                .map(|(index, tab)| (index, tab.value.clone()))
                .collect(),
        );
        let handles: Rc<Vec<FocusHandle>> = Rc::new(
            (0..self.tabs.len())
                .map(|index| {
                    window
                        .use_keyed_state(
                            child_id(&self.id, format!("focus-{index}")),
                            cx,
                            |_, cx| cx.focus_handle(),
                        )
                        .read(cx)
                        .clone()
                })
                .collect(),
        );
        let mut tabs = Vec::with_capacity(self.tabs.len());
        for (index, tab) in self.tabs.into_iter().enumerate() {
            let active = tab.value == self.active;
            let (keys, focus, click, key) = (
                enabled.clone(),
                handles.clone(),
                self.on_change.clone(),
                self.on_change.clone(),
            );
            let value = tab.value.clone();
            let button = Button::new(child_id(&self.id, index.to_string()))
                .role(Role::Tab)
                .selected(active)
                .disabled(tab.disabled)
                .track_focus(&handles[index])
                .tab_stop(active)
                .accessibility_label(tab.label.clone())
                .gap(theme.metrics.space_xs)
                .min_w_0()
                .when(self.equal_width, |this| this.flex_1().justify_center())
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
                .when(!tab.disabled, |this| {
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
                .when_some(click, |this, handler| {
                    this.on_click(move |_, window, cx| handler(value.clone(), window, cx))
                })
                .on_key_down(move |event, window, cx| {
                    let Some(position) = keys.iter().position(|(tab, _)| *tab == index) else {
                        return;
                    };
                    let Some(target) = tab_key_target(&event.keystroke.key, position, keys.len())
                    else {
                        return;
                    };
                    cx.stop_propagation();
                    let (tab, value) = &keys[target];
                    focus[*tab].focus(window, cx);
                    if let Some(handler) = &key {
                        handler(value.clone(), window, cx);
                    }
                });
            tabs.push(match tab.icon {
                Some(name) if tab.show_label => button
                    .min_w(
                        theme.metrics.space_lg
                            * if self.id == ElementId::from("transport-tabs") {
                                6.
                            } else {
                                5.
                            },
                    )
                    .text_size(theme.metrics.font_xs)
                    .font_weight(gpui::FontWeight::BOLD)
                    .border_r_1()
                    .border_color(theme.colors.border)
                    .h(theme.metrics.tabbar_height)
                    .px(theme.metrics.space_sm)
                    .child(icon(&name).text_color(if active {
                        theme.colors.accent_text
                    } else {
                        theme.colors.text_muted
                    }))
                    .child(tab.label)
                    .into_any_element(),
                Some(name) if self.id == ElementId::from("utility-tabs") => button
                    .flex_1()
                    .h(theme.metrics.tabbar_height)
                    .border_r_1()
                    .border_color(theme.colors.border)
                    .tooltip(tooltip_text(tab.label))
                    .child(icon(&name).text_color(if active {
                        theme.colors.accent_text
                    } else {
                        theme.colors.text_muted
                    }))
                    .into_any_element(),
                Some(name) => button
                    .size(theme.metrics.control_height_compact)
                    .tooltip(tooltip_text(tab.label))
                    .child(icon(&name).text_color(if active {
                        theme.colors.accent_text
                    } else {
                        theme.colors.text_muted
                    }))
                    .into_any_element(),
                None => button
                    .h(theme.metrics.tabbar_height)
                    .px(theme.metrics.space_sm)
                    .child(tab.label)
                    .into_any_element(),
            });
        }
        div()
            .id(self.id.clone())
            .role(Role::TabList)
            .aria_label(self.label)
            .flex()
            .items_stretch()
            .min_w_0()
            .bg(theme.colors.panel_bg_soft)
            .when(self.id == ElementId::from("transport-tabs"), |list| {
                list.justify_center()
            })
            .border_b_1()
            .border_color(theme.colors.border_strong)
            .text_color(theme.colors.text)
            .children(tabs)
    }
}
