use std::rc::Rc;

use gpui::{
    div, prelude::*, px, AnyElement, App, ElementId, FocusHandle, MouseButton, Pixels,
    SharedString, Window,
};
use gpui_base::{DialogBackdrop, DialogPopup, DialogTitle};

use super::{child_id, ButtonVariant, Handler, IconButton};
use crate::ActiveTheme;

/// The only dialog skin. `standalone` fills an auxiliary window: no backdrop, and the header moves the window.
#[derive(IntoElement)]
pub struct Dialog {
    id: ElementId,
    title: SharedString,
    close_label: SharedString,
    width: Option<Pixels>,
    standalone: bool,
    compact_header: bool,
    flush: bool,
    close_on_backdrop: bool,
    close_on_enter: bool,
    initial_focus: Option<FocusHandle>,
    actions: Vec<AnyElement>,
    footer: Vec<AnyElement>,
    children: Vec<AnyElement>,
    on_close: Handler<()>,
}

impl Dialog {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        close_label: impl Into<SharedString>,
        on_close: impl Fn((), &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            close_label: close_label.into(),
            width: None,
            standalone: false,
            compact_header: false,
            flush: false,
            close_on_backdrop: true,
            close_on_enter: true,
            initial_focus: None,
            actions: Vec::new(),
            footer: Vec::new(),
            children: Vec::new(),
            on_close: Rc::new(on_close),
        }
    }

    pub fn width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
    }

    pub fn standalone(mut self) -> Self {
        self.standalone = true;
        self
    }

    pub fn compact_header(mut self) -> Self {
        self.compact_header = true;
        self
    }

    /// The body fills the dialog without padding, for a layout that draws its own panels.
    pub fn flush(mut self) -> Self {
        self.flush = true;
        self
    }

    pub fn close_on_backdrop(mut self, close: bool) -> Self {
        self.close_on_backdrop = close;
        self
    }

    /// Keep the dialog open when Enter applies an inline editor.
    pub fn close_on_enter(mut self, close: bool) -> Self {
        self.close_on_enter = close;
        self
    }

    pub fn initial_focus(mut self, focus: FocusHandle) -> Self {
        self.initial_focus = Some(focus);
        self
    }

    /// A control in the header, next to the close button.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn footer(mut self, control: impl IntoElement) -> Self {
        self.footer.push(control.into_any_element());
        self
    }
}

impl ParentElement for Dialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Dialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let close = self.on_close.clone();
        let standalone = self.standalone;
        let close_button =
            IconButton::new(child_id(&self.id, "close"), "circle-x", self.close_label);
        let close_button = if self.compact_header {
            close_button
        } else {
            close_button.variant(ButtonVariant::Soft).full_height()
        };
        let header =
            div()
                .id(child_id(&self.id, "header"))
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .min_w_0()
                .gap(theme.metrics.space_sm)
                .py(if self.compact_header {
                    theme.metrics.space_2xs
                } else {
                    theme.metrics.space_md
                })
                .px(theme.metrics.panel_padding)
                .border_b_1()
                .border_color(theme.colors.border)
                .bg(theme.colors.panel_bg_strong)
                .text_color(theme.colors.text_strong)
                .when(standalone && !cfg!(windows), |this| {
                    this.on_mouse_down(MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    })
                })
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .self_stretch()
                        .min_w_0()
                        .items_center()
                        .when(standalone && cfg!(windows), |this| {
                            this.window_control_area(gpui::WindowControlArea::Drag)
                        })
                        .child(
                            DialogTitle::new()
                                .min_w_0()
                                .truncate()
                                .text_size(theme.metrics.font_lg)
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(self.title),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(theme.metrics.space_xs)
                        // A press on a header control must not start a window move.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .children(self.actions)
                        .child(div().debug_selector(|| "dialog-close".to_string()).child(
                            close_button.on_click(move |_, window, cx| close((), window, cx)),
                        )),
                );
        let body = div()
            .id(child_id(&self.id, "body"))
            .flex_1()
            .min_h_0()
            .min_w_0()
            .when(!self.flush, |this| {
                this.p(theme.metrics.panel_padding).overflow_y_scroll()
            })
            .children(self.children);
        let footer = (!self.footer.is_empty()).then(|| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_end()
                .gap(theme.metrics.control_gap)
                .py(theme.metrics.space_md)
                .px(theme.metrics.panel_padding)
                .border_t_1()
                .border_color(theme.colors.border)
                .bg(theme.colors.panel_bg_strong)
                .children(self.footer)
        });
        let content = div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(theme.colors.surface_bg)
            .text_color(theme.colors.text)
            .child(header)
            .child(body)
            .children(footer);
        if standalone {
            return content.into_any_element();
        }
        // Escape and Tab need focus inside the dialog, and nothing else gives it focus on opening.
        let focus = window
            .use_keyed_state(child_id(&self.id, "focus"), cx, |window, cx| {
                let handle = cx.focus_handle();
                let initial = self.initial_focus.unwrap_or_else(|| handle.clone());
                window.defer(cx, move |window, cx| initial.focus(window, cx));
                handle
            })
            .read(cx)
            .clone();
        let on_close = self.on_close;
        let close_on_enter = self.close_on_enter;
        gpui_base::Dialog::new(cx)
            .open(true)
            .on_ok(move |_, _, _| close_on_enter)
            .focus_handle(focus)
            .close_on_backdrop_press(self.close_on_backdrop)
            .on_open_change(move |open, _, window, cx| {
                if !open {
                    on_close((), window, cx);
                }
            })
            .backdrop(
                DialogBackdrop::new()
                    .absolute()
                    .size_full()
                    .bg(theme.colors.overlay_bg),
            )
            .popup(
                DialogPopup::new()
                    .flex()
                    .flex_col()
                    .when_some(self.width, |this, width| this.w(width))
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(theme.metrics.radius_md)
                    .shadow(theme.shadow())
                    .child(content),
            )
            .into_any_element()
    }
}

/// A busy popup for work that reports no progress: the message over an indeterminate bar. It blocks the window
/// behind it without dimming it, and offers no way to close, because the work cannot be interrupted.
#[derive(IntoElement)]
pub struct BusyDialog {
    id: ElementId,
    message: SharedString,
    page: bool,
}

impl BusyDialog {
    pub fn new(id: impl Into<ElementId>, message: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            message: message.into(),
            page: false,
        }
    }

    /// A centered loading state covering the page, without a popup frame.
    pub fn page(mut self) -> Self {
        self.page = true;
        self
    }
}

impl RenderOnce for BusyDialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use gpui::{ease_in_out, relative, Animation, AnimationExt};
        let theme = *cx.theme();
        let bar = div()
            .flex_none()
            .h(px(3.))
            .overflow_hidden()
            .rounded(theme.metrics.radius_xs)
            .bg(theme.colors.menu_separator)
            .child(
                div()
                    .h_full()
                    .w(relative(0.38))
                    .rounded(theme.metrics.radius_xs)
                    .bg(theme.colors.accent)
                    .with_animation(
                        child_id(&self.id, "progress"),
                        Animation::new(std::time::Duration::from_millis(1150))
                            .repeat()
                            .with_easing(gpui::bounce(ease_in_out)),
                        |segment, progress| segment.ml(relative(progress * 0.62)),
                    ),
            );
        if self.page {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p(theme.metrics.space_lg)
                .bg(theme.colors.bg)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(theme.metrics.space_md)
                        .w(theme.metrics.control_height * 10.)
                        .max_w_full()
                        .text_center()
                        .text_color(theme.colors.text)
                        .child(div().child(self.message))
                        .child(bar),
                )
                .into_any_element();
        }
        let focus = window
            .use_keyed_state(child_id(&self.id, "focus"), cx, |window, cx| {
                let handle = cx.focus_handle();
                let initial = handle.clone();
                window.defer(cx, move |window, cx| initial.focus(window, cx));
                handle
            })
            .read(cx)
            .clone();
        gpui_base::Dialog::new(cx)
            .open(true)
            .focus_handle(focus)
            .close_on_backdrop_press(false)
            .on_open_change(|_, _, _, _| {})
            .backdrop(DialogBackdrop::new().absolute().size_full())
            .popup(
                DialogPopup::new()
                    .flex()
                    .flex_col()
                    .gap(theme.metrics.space_md)
                    .w(px(320.))
                    .overflow_hidden()
                    .px(theme.metrics.panel_padding)
                    .py(theme.metrics.space_lg)
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(theme.metrics.radius_md)
                    .bg(theme.colors.surface_bg)
                    .text_color(theme.colors.text_strong)
                    .shadow(theme.shadow())
                    .child(div().child(self.message))
                    .child(bar),
            )
            .into_any_element()
    }
}
