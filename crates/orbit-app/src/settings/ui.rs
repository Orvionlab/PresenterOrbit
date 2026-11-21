//! Small layout helpers shared by the panels.

use gpui::{div, prelude::*, Div, FontWeight, SharedString, Window};
use orbit_ui::ActiveTheme;

/// `normalizeWindowsPathForDisplay`: the verbatim `\\?\` prefix is noise in labels. The original value stays
/// the one that is sent to the host and persisted.
pub fn path_for_display(path: &str) -> String {
    const UNC: &str = r"\\?\UNC\";
    const VERBATIM: &str = r"\\?\";
    let lower = path.to_lowercase();
    if lower.starts_with(&UNC.to_lowercase()) {
        format!(r"\\{}", &path[UNC.len()..])
    } else if lower.starts_with(VERBATIM) {
        path[VERBATIM.len()..].to_string()
    } else {
        path.to_string()
    }
}

/// A vertical stack of controls, `react-settings-stack`.
pub fn stack(cx: &gpui::App) -> Div {
    div().flex().flex_col().gap(cx.theme().metrics.control_gap)
}

/// `.general-settings h3`: the heading of a panel section.
pub fn heading(label: impl Into<SharedString>, cx: &gpui::App) -> Div {
    let theme = cx.theme();
    div()
        .text_size(theme.metrics.font_md)
        .line_height(gpui::relative(1.25))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.colors.text_strong)
        .child(label.into())
}

/// The browser-default `h3` of the panels that do not restyle it, with its em margins.
pub fn title(label: impl Into<SharedString>, cx: &gpui::App) -> Div {
    let theme = cx.theme();
    div()
        .my(theme.metrics.font_xl)
        .text_size(theme.metrics.font_xl)
        .font_weight(FontWeight::BOLD)
        .text_color(theme.colors.text_strong)
        .child(label.into())
}

/// An `h4` of a panel section.
pub fn subheading(label: impl Into<SharedString>, cx: &gpui::App) -> Div {
    let theme = cx.theme();
    div()
        .text_size(theme.metrics.font_lg)
        .font_weight(FontWeight::BOLD)
        .text_color(theme.colors.text)
        .child(label.into())
}

/// `.settings-section label`: a small semibold label above its control.
#[derive(IntoElement)]
pub struct Field {
    label: SharedString,
    control: gpui::AnyElement,
}

impl Field {
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Self {
            label: label.into(),
            control: control.into_any_element(),
        }
    }
}

impl RenderOnce for Field {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .min_w_0()
            .text_size(theme.metrics.font_sm)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.colors.text)
            .flex_col()
            .gap(theme.metrics.space_xs)
            .child(div().flex_none().child(self.label))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_weight(FontWeight::NORMAL)
                    .child(self.control),
            )
    }
}

/// `.settings-section output`: a read-only value in a field box.
#[derive(IntoElement)]
pub struct OutputField {
    label: SharedString,
    value: SharedString,
}

impl OutputField {
    pub fn new(label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

impl RenderOnce for OutputField {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let theme = cx.theme();
        Field::new(
            self.label,
            div()
                .flex()
                .items_center()
                .w_full()
                .min_h(theme.metrics.control_height)
                .px(theme.metrics.space_sm)
                .py(theme.metrics.space_snug)
                .rounded(theme.metrics.radius_md)
                .bg(theme.colors.field_bg)
                .text_size(theme.metrics.font_sm)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.colors.text)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(self.value),
        )
    }
}

pub fn action_row(cx: &gpui::App) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(cx.theme().metrics.control_gap)
}

pub fn muted(label: impl Into<SharedString>, cx: &gpui::App) -> Div {
    let theme = cx.theme();
    div()
        .text_size(theme.metrics.font_sm)
        .text_color(theme.colors.text_muted)
        .child(label.into())
}
