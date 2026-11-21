use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    canvas, div, prelude::*, px, App, Bounds, ClickEvent, ElementId, FontWeight, MouseButton,
    Pixels, Point, Role, SharedString, Window,
};
use gpui_base::Button;

use super::{child_id, focus_ring, Handler};
use crate::{icon, ActiveTheme};

type BoundsHandler = Rc<dyn Fn(Bounds<Pixels>, &mut Window, &mut App)>;
type ContextHandler = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;

const TOOL_BUTTON_MIN_WIDTH: Pixels = px(62.);
const TOOL_ICON_SIZE: Pixels = px(26.);
const BRAND_MIN_WIDTH: Pixels = px(164.);
const BRAND_HEIGHT: Pixels = px(44.);

/// A toolbar action: an icon above its label. `primary` and `active` share one look.
#[derive(IntoElement)]
pub struct ToolButton {
    id: ElementId,
    icon: SharedString,
    label: SharedString,
    primary: bool,
    active: bool,
    live: bool,
    on_context_menu: Option<ContextHandler>,
    disabled: bool,
    on_click: Option<Handler<()>>,
    on_click_bounds: Option<BoundsHandler>,
}

impl ToolButton {
    pub fn new(
        id: impl Into<ElementId>,
        icon: impl Into<SharedString>,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            label: label.into(),
            primary: false,
            active: false,
            live: false,
            on_context_menu: None,
            disabled: false,
            on_click: None,
            on_click_bounds: None,
        }
    }

    /// Like a click, but reports the button's bounds so the owner can open a menu below it.
    pub fn on_click_bounds(
        mut self,
        handler: impl Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click_bounds = Some(Rc::new(handler));
        self
    }

    pub fn primary(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn live(mut self, live: bool) -> Self {
        self.live = live;
        self
    }

    pub fn on_context_menu(
        mut self,
        handler: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_context_menu = Some(Rc::new(handler));
        self
    }

    pub fn on_click(mut self, handler: impl Fn((), &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for ToolButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let bounds = window
            .use_keyed_state(child_id(&self.id, "bounds"), cx, |_, _| {
                Rc::new(Cell::new(Bounds::<Pixels>::default()))
            })
            .read(cx)
            .clone();
        let recorded = bounds.clone();
        let lit = self.primary || self.active;
        let (rest, lit_color) = (
            if self.live {
                theme.colors.live
            } else {
                theme.colors.text_muted
            },
            if self.live {
                theme.colors.live
            } else {
                theme.colors.text
            },
        );
        Button::new(self.id)
            .disabled(self.disabled)
            .accessibility_label(self.label.clone())
            .flex_col()
            .items_center()
            .justify_center()
            .gap(theme.metrics.space_xs)
            .min_w(TOOL_BUTTON_MIN_WIDTH)
            .py(theme.metrics.space_xs)
            .px(theme.metrics.space_snug)
            .rounded(theme.metrics.radius_md)
            .text_size(theme.metrics.font_sm)
            .text_color(if lit { lit_color } else { rest })
            .when(lit, |this| this.bg(theme.colors.accent_soft))
            .when(self.disabled, |this| this.opacity(0.55))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(theme.colors.accent_soft).text_color(lit_color))
                    .focus_visible(|style| focus_ring(style, cx))
            })
            .when_some(self.on_click, |this, handler| {
                this.on_click(move |_, window, cx| handler((), window, cx))
            })
            .when_some(self.on_click_bounds, move |this, handler| {
                this.on_click(move |_: &ClickEvent, window, cx| handler(bounds.get(), window, cx))
            })
            .when_some(self.on_context_menu, |this, handler| {
                this.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                    handler(event.position, window, cx);
                    cx.stop_propagation();
                })
            })
            .relative()
            .child(
                canvas(move |area, _, _| recorded.set(area), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(icon(&self.icon).size(TOOL_ICON_SIZE).text_color(if lit {
                lit_color
            } else {
                rest
            }))
            .child(self.label)
    }
}

/// The application button at the start of the toolbar: the product name and a trailing icon.
#[derive(IntoElement)]
pub struct BrandButton {
    id: ElementId,
    title: SharedString,
    icon: SharedString,
    on_click: Option<Handler<()>>,
}

impl BrandButton {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        icon: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            icon: icon.into(),
            on_click: None,
        }
    }

    pub fn on_click(mut self, handler: impl Fn((), &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for BrandButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        Button::new(self.id)
            .accessibility_label(self.title.clone())
            .gap(theme.metrics.control_gap)
            .min_w(BRAND_MIN_WIDTH)
            .h(BRAND_HEIGHT)
            .px(theme.metrics.field_padding_x)
            .rounded(theme.metrics.radius_md)
            .bg(theme.colors.panel_bg)
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_md)
            .cursor_pointer()
            .hover(|style| style.bg(theme.colors.accent_soft))
            .focus_visible(|style| focus_ring(style, cx))
            .when_some(self.on_click, |this, handler| {
                this.on_click(move |_, window, cx| handler((), window, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::BOLD)
                    .child(self.title),
            )
            .child(icon(&self.icon).text_color(theme.colors.text_muted))
    }
}

/// A flat button in the application menu bar. It reports its bounds so the owner can open a menu below it.
#[derive(IntoElement)]
pub struct BarButton {
    id: ElementId,
    label: SharedString,
    active: bool,
    on_click: Option<BoundsHandler>,
}

impl BarButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            active: false,
            on_click: None,
        }
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for BarButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let bounds = window
            .use_keyed_state(child_id(&self.id, "bounds"), cx, |_, _| {
                Rc::new(Cell::new(Bounds::<Pixels>::default()))
            })
            .read(cx)
            .clone();
        let recorded = bounds.clone();
        Button::new(self.id)
            .role(Role::MenuItem)
            .relative()
            .h_full()
            .min_w_0()
            .px(theme.metrics.field_padding_x)
            .text_size(theme.metrics.font_sm)
            .text_color(if self.active {
                theme.colors.text_strong
            } else {
                theme.colors.text
            })
            .when(self.active, |this| this.bg(theme.colors.surface_raised_bg))
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(theme.colors.surface_raised_bg)
                    .text_color(theme.colors.text_strong)
            })
            .focus_visible(|style| focus_ring(style, cx))
            .when_some(self.on_click, move |this, handler| {
                this.on_click(move |_: &ClickEvent, window, cx| handler(bounds.get(), window, cx))
            })
            .child(
                canvas(move |area, _, _| recorded.set(area), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(self.label)
    }
}
