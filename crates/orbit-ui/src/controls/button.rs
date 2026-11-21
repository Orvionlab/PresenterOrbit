use std::rc::Rc;

use gpui::{
    div, prelude::*, AnyView, App, ClickEvent, Context, ElementId, Rgba, SharedString, Window,
};
use gpui_base::Button;

use super::focus_ring;
use crate::{icon, ActiveTheme};

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Ghost,
    Danger,
    /// A raised soft fill without a border, as the dialog close button.
    Soft,
}

struct VariantColors {
    background: Option<Rgba>,
    hover: Rgba,
    text: Rgba,
    border: Option<Rgba>,
}

impl ButtonVariant {
    fn colors(self, cx: &App) -> VariantColors {
        let colors = cx.theme().colors;
        match self {
            Self::Primary => VariantColors {
                background: Some(colors.accent),
                hover: colors.accent_hover,
                text: colors.accent_text,
                border: None,
            },
            Self::Secondary => VariantColors {
                background: Some(colors.control_bg),
                hover: colors.control_bg_hover,
                text: colors.text,
                border: Some(colors.control_border),
            },
            Self::Ghost => VariantColors {
                background: None,
                hover: colors.accent_soft,
                text: colors.text,
                border: None,
            },
            Self::Soft => VariantColors {
                background: Some(colors.panel_bg_soft),
                hover: colors.accent_soft,
                text: colors.text,
                border: None,
            },
            Self::Danger => VariantColors {
                background: Some(colors.danger),
                hover: colors.danger,
                text: colors.accent_text,
                border: None,
            },
        }
    }
}

struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px(theme.metrics.space_snug)
            .py(theme.metrics.space_2xs)
            .rounded(theme.metrics.radius_sm)
            .border_1()
            .border_color(theme.colors.border)
            .bg(theme.colors.popover_bg)
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_sm)
            .child(self.0.clone())
    }
}

/// A plain text tooltip for `.tooltip(...)`.
pub fn tooltip_text(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

/// A text action with an optional leading icon. One primary action per view.
#[derive(IntoElement)]
pub struct ActionButton {
    id: ElementId,
    label: SharedString,
    icon: Option<SharedString>,
    variant: ButtonVariant,
    compact: bool,
    disabled: bool,
    selected: bool,
    on_click: Option<ClickHandler>,
}

impl ActionButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            variant: ButtonVariant::Primary,
            compact: false,
            disabled: false,
            selected: false,
            on_click: None,
        }
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn icon(mut self, name: impl Into<SharedString>) -> Self {
        self.icon = Some(name.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The button shows a value that is switched on, as an icon toggle does.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for ActionButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let colors = self.variant.colors(cx);
        let selector = format!("button-{}", self.id);
        Button::new(self.id)
            .debug_selector(move || selector.clone())
            .disabled(self.disabled)
            .selected(self.selected)
            .styles(|styles| styles.selected(|style| style.bg(theme.colors.selected)))
            .gap(theme.metrics.space_snug)
            .h(if self.compact {
                theme.metrics.control_height_compact
            } else {
                theme.metrics.control_height
            })
            .px(theme.metrics.field_padding_x)
            .flex_none()
            .rounded(theme.metrics.radius_md)
            .text_size(theme.metrics.font_md)
            .text_color(colors.text)
            .when_some(colors.background, |this, color| this.bg(color))
            .when_some(colors.border, |this, color| {
                this.border_1().border_color(color)
            })
            .when(self.disabled, |this| this.opacity(0.55))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(colors.hover))
                    .focus_visible(|style| focus_ring(style, cx))
            })
            .when_some(self.on_click, |this, handler| {
                this.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .when_some(self.icon, |this, name| {
                this.child(icon(&name).text_color(colors.text))
            })
            .child(self.label)
    }
}

/// An icon-only action. Its label is both the accessible name and the tooltip.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: SharedString,
    label: SharedString,
    variant: ButtonVariant,
    disabled: bool,
    selected: bool,
    full_height: bool,
    inline: bool,
    large_icon: bool,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    pub fn new(
        id: impl Into<ElementId>,
        icon: impl Into<SharedString>,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            label: label.into(),
            variant: ButtonVariant::Ghost,
            disabled: false,
            selected: false,
            full_height: false,
            inline: false,
            large_icon: false,
            on_click: None,
        }
    }

    /// The 20px icon of the prominent panel headers.
    pub fn large_icon(mut self) -> Self {
        self.large_icon = true;
        self
    }

    /// A control-height square instead of the compact one.
    pub fn full_height(mut self) -> Self {
        self.full_height = true;
        self
    }

    /// A small secondary action embedded in a label or chip.
    pub fn inline(mut self) -> Self {
        self.inline = true;
        self
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let colors = self.variant.colors(cx);
        let selector = format!("button-{}", self.id);
        Button::new(self.id)
            .debug_selector(move || selector.clone())
            .disabled(self.disabled)
            .selected(self.selected)
            .styles(|styles| styles.selected(|style| style.bg(theme.colors.selected)))
            .accessibility_label(self.label.clone())
            .tooltip(tooltip_text(self.label))
            .size(if self.inline {
                theme.metrics.space_lg
            } else if self.full_height {
                theme.metrics.control_height
            } else {
                theme.metrics.control_height_compact
            })
            .flex_none()
            .rounded(theme.metrics.radius_sm)
            .text_color(colors.text)
            .when_some(colors.background, |this, color| this.bg(color))
            .when_some(colors.border, |this, color| {
                this.border_1().border_color(color)
            })
            .when(self.disabled, |this| this.opacity(0.55))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(colors.hover))
                    .focus_visible(|style| focus_ring(style, cx))
            })
            .when_some(self.on_click, |this, handler| {
                this.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .child(
                icon(&self.icon)
                    .when(self.large_icon, |icon| icon.size_5())
                    .when(self.inline, |icon| icon.size(theme.metrics.space_md))
                    .text_color(colors.text),
            )
    }
}
