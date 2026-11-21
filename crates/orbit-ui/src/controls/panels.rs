use std::rc::Rc;

use gpui::{div, prelude::*, AnyElement, App, Div, ElementId, SharedString, Stateful, Window};
use gpui_base::Button;

use super::{focus_ring, Handler};
use crate::{icon, ActiveTheme};

/// The one header of a panel.
#[derive(IntoElement)]
pub struct PanelHeader {
    title: AnyElement,
    control: bool,
    actions: Vec<AnyElement>,
    prominent: bool,
    contrast: bool,
    tint: Option<gpui::Rgba>,
}

impl PanelHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        let title: SharedString = title.into();
        Self {
            title: title.into_any_element(),
            control: false,
            actions: Vec::new(),
            prominent: false,
            contrast: false,
            tint: None,
        }
    }

    /// The larger header of the utility panels (`orbit-panel-header` in the Audio, Stage, Timers, Messages,
    /// Props and Macros panels).
    pub fn prominent(mut self) -> Self {
        self.prominent = true;
        self
    }

    /// Stronger separation for headers between adjacent list sections.
    pub fn contrast(mut self) -> Self {
        self.contrast = true;
        self
    }

    /// The section color is blended into the panel surface to preserve text contrast.
    pub fn section_tint(mut self, color: gpui::Rgba) -> Self {
        self.tint = Some(color);
        self
    }

    /// A control (the collection picker of Props and Macros) as the title instead of text.
    pub fn title_control(mut self, control: impl IntoElement) -> Self {
        self.title = control.into_any_element();
        self.control = true;
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl RenderOnce for PanelHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let background = if self.contrast {
            theme.colors.surface_raised_bg
        } else {
            theme.colors.panel_bg
        };
        let background = self
            .tint
            .map_or(background, |color| background.blend(color.opacity(0.32)));
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(theme.metrics.space_xs)
            .min_w_0()
            .h(theme.metrics.header_height)
            .px(theme.metrics.space_sm)
            .border_b_1()
            .border_color(if self.contrast {
                theme.colors.border_strong
            } else {
                theme.colors.border
            })
            .bg(background)
            .text_color(if self.contrast {
                theme.colors.text_strong
            } else {
                theme.colors.text
            })
            .text_size(if self.prominent {
                theme.metrics.font_lg
            } else {
                theme.metrics.font_sm
            })
            .font_weight(if self.prominent {
                gpui::FontWeight::MEDIUM
            } else {
                gpui::FontWeight::SEMIBOLD
            })
            .child(if self.control {
                div().flex_1().min_w_0().child(self.title)
            } else {
                div().truncate().min_w_0().child(self.title)
            })
            .when(!self.actions.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(theme.metrics.space_2xs)
                        .children(self.actions),
                )
            })
    }
}

/// A single muted line inside an empty region.
#[derive(IntoElement)]
pub struct EmptyState(SharedString);

impl EmptyState {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self(text.into())
    }
}

impl RenderOnce for EmptyState {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .p(theme.metrics.space_sm)
            .text_color(theme.colors.text_muted)
            .text_size(theme.metrics.font_sm)
            .child(self.0)
    }
}

/// A collapsible section. The owner keeps `open`.
#[derive(IntoElement)]
pub struct Disclosure {
    id: ElementId,
    summary: SharedString,
    open: bool,
    children: Vec<AnyElement>,
    on_toggle: Option<Handler<bool>>,
}

impl Disclosure {
    pub fn new(id: impl Into<ElementId>, summary: impl Into<SharedString>, open: bool) -> Self {
        Self {
            id: id.into(),
            summary: summary.into(),
            open,
            children: Vec::new(),
            on_toggle: None,
        }
    }

    pub fn on_toggle(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }
}

impl ParentElement for Disclosure {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Disclosure {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let open = self.open;
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(
                Button::new(self.id)
                    .justify_start()
                    .gap(theme.metrics.space_xs)
                    .w_full()
                    .h(theme.metrics.row_height)
                    .cursor_pointer()
                    .text_color(theme.colors.text)
                    .text_size(theme.metrics.font_sm)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .focus_visible(|style| focus_ring(style, cx))
                    .when_some(self.on_toggle, |this, handler| {
                        this.on_click(move |_, window, cx| handler(!open, window, cx))
                    })
                    .child(
                        icon(if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        })
                        .text_color(theme.colors.text),
                    )
                    .child(self.summary),
            )
            .when(open, |this| this.children(self.children))
    }
}

/// A list row. The caller adds the content and interaction.
pub fn list_row(id: impl Into<ElementId>, selected: bool, cx: &App) -> Stateful<Div> {
    let hover = cx.theme().colors.accent_soft;
    list_row_without_hover(id, selected, cx)
        .when(!selected, |this| this.hover(|style| style.bg(hover)))
}

/// A list row whose background changes only when selected.
pub fn list_row_without_hover(id: impl Into<ElementId>, selected: bool, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    let (selected_bg, strong) = (theme.colors.selected, theme.colors.accent_text);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(theme.metrics.space_xs)
        .w_full()
        .min_w_0()
        .h(theme.metrics.row_height)
        .px(theme.metrics.space_sm)
        .text_size(theme.metrics.font_sm)
        .text_color(theme.colors.text)
        .when(selected, |this| this.bg(selected_bg).text_color(strong))
}

/// A list row with tree indentation and, for folders, a chevron.
pub fn tree_row(
    id: impl Into<ElementId>,
    depth: usize,
    expanded: Option<bool>,
    selected: bool,
    cx: &App,
) -> Stateful<Div> {
    let (metrics, color) = (cx.theme().metrics, cx.theme().colors.text);
    list_row(id, selected, cx)
        .pl(metrics.space_sm + metrics.space_md * depth as f32)
        .child(
            div()
                .flex_none()
                .size(metrics.space_lg)
                .when_some(expanded, |this, open| {
                    this.child(
                        icon(if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        })
                        .size(metrics.space_lg)
                        .text_color(color),
                    )
                }),
        )
}
