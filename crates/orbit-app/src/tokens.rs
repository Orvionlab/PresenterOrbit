use gpui::{div, prelude::*, px, Context, Rgba, SharedString, Subscription, Window};
use orbit_ui::{follow_theme, ActiveTheme};

/// Developer screen (`--tokens`) that shows every color and metric token of the active theme.
pub struct TokenView {
    _theme: [Subscription; 2],
}

impl TokenView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            _theme: follow_theme(window, cx),
        }
    }
}

fn hex(color: Rgba) -> SharedString {
    let channel = |value: f32| (value * 255.0).round() as u8;
    let (r, g, b) = (channel(color.r), channel(color.g), channel(color.b));
    if color.a < 1.0 {
        format!("#{r:02x}{g:02x}{b:02x} {:.0}%", color.a * 100.0).into()
    } else {
        format!("#{r:02x}{g:02x}{b:02x}").into()
    }
}

impl Render for TokenView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let (colors, metrics) = (theme.colors, theme.metrics);
        let swatches = colors.entries().into_iter().map(|(name, color)| {
            div()
                .flex()
                .items_center()
                .gap(metrics.space_sm)
                .h(metrics.row_height)
                .child(
                    div()
                        .w(metrics.control_height)
                        .h(metrics.row_height - px(4.0))
                        .rounded(metrics.radius_sm)
                        .border_1()
                        .border_color(colors.border)
                        .bg(color),
                )
                .child(div().w(px(220.0)).child(name))
                .child(div().text_color(colors.text_muted).child(hex(color)))
        });
        let sizes = metrics.entries().into_iter().map(|(name, value)| {
            div()
                .flex()
                .gap(metrics.space_sm)
                .h(metrics.row_height)
                .child(div().w(px(240.0)).child(name))
                .child(
                    div()
                        .text_color(colors.text_muted)
                        .child(format!("{value:.2} px")),
                )
        });
        div()
            .id("tokens")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .gap(metrics.space_lg)
            .p(metrics.space_lg)
            .bg(colors.bg)
            .text_color(colors.text)
            .text_size(metrics.font_sm)
            .child(div().flex().flex_col().children(swatches))
            .child(div().flex().flex_col().children(sizes))
    }
}
