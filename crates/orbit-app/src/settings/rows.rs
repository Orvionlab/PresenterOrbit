//! `SettingsRows.tsx` and `settings-media.css`: the grouped rows of the Media, Import, Audio, Inputs, Video
//! and Integrations panels, and the header of a subpage.

use gpui::{div, prelude::*, px, AnyElement, App, Div, SharedString, Window};
use orbit_ui::controls::{IconButton, Select, SelectOption, ToggleSwitch};
use orbit_ui::{icon, ActiveTheme};

/// The widest a settings page grows; every panel and subpage is centred at most this wide.
pub const CONTENT_MAX_WIDTH: f32 = 760.;

/// `.media-settings`: a panel column at most 760 px wide, centred in the content area.
pub fn column(cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(cx.theme().metrics.control_gap)
        .w_full()
        .max_w(px(CONTENT_MAX_WIDTH))
        .mx_auto()
}

/// `.media-settings-group`: rows on a strong panel fill with a rule between them.
pub fn group(rows: impl IntoIterator<Item = AnyElement>, cx: &App) -> Div {
    let theme = cx.theme();
    let mut group = div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(theme.metrics.radius_sm)
        .bg(theme.colors.panel_bg_strong);
    for (index, row) in rows.into_iter().enumerate() {
        group = group.child(
            div()
                .when(index > 0, |this| {
                    this.border_t_1().border_color(theme.colors.border)
                })
                .child(row),
        );
    }
    group
}

fn row_frame(cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(theme.metrics.space_md)
        .min_h(theme.metrics.control_height + theme.metrics.space_md + theme.metrics.space_2xs)
        .px(theme.metrics.space_md)
        .py(theme.metrics.space_sm)
        .text_size(theme.metrics.font_sm)
        .text_color(theme.colors.text)
}

/// `.media-settings-label`: the label with an optional description under it.
fn label_block(label: SharedString, description: Option<SharedString>, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(theme.metrics.space_2xs)
        .child(div().child(label))
        .children(
            description
                .map(|description| div().text_color(theme.colors.text_muted).child(description)),
        )
}

/// `SettingRow`: a label, an optional description, and a control on the right.
pub fn setting_row(
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> AnyElement {
    row_frame(cx)
        .child(label_block(label.into(), description, cx))
        .child(div().flex_none().child(control))
        .into_any_element()
}

pub fn setting_field_row(
    id: &'static str,
    label: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> AnyElement {
    row_frame(cx)
        .child(label_block(label.into(), None, cx))
        .child(
            div()
                .id(id)
                .debug_selector(move || id.into())
                .w(gpui::relative(0.5))
                .flex_none()
                .child(control),
        )
        .into_any_element()
}

/// `SettingToggle`.
pub fn toggle_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    checked: bool,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let label = label.into();
    setting_row(
        label.clone(),
        description,
        ToggleSwitch::new(id.into(), label, checked)
            .bare()
            .on_change(on_change),
        cx,
    )
}

/// `SettingSelect`: the label on the left and a select that takes the rest of the row.
pub fn select_row<T: Clone + PartialEq + 'static>(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    options: Vec<SelectOption<T>>,
    value: impl Into<Option<T>>,
    on_change: impl Fn(T, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let label = label.into();
    row_frame(cx)
        .child(label_block(label.clone(), None, cx))
        .child(
            div().w(px(290.)).flex_none().child(
                Select::new(id.into(), options, value)
                    .label(label)
                    .on_change(on_change),
            ),
        )
        .into_any_element()
}

/// `SettingLink`: a row that opens a subpage; the whole row is the button.
pub fn link_row(
    id: impl Into<SharedString>,
    glyph: Option<&'static str>,
    label: impl Into<SharedString>,
    on_open: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let hover = theme.colors.accent_soft;
    row_frame(cx)
        .id(id.into())
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .on_click(move |_, window, cx| on_open(window, cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(theme.metrics.space_sm)
                .children(glyph.map(|glyph| icon(glyph).text_color(theme.colors.text)))
                .child(label.into()),
        )
        .child(
            icon("arrow-right")
                .size(theme.metrics.font_md)
                .text_color(theme.colors.text),
        )
        .into_any_element()
}

/// `SubpageHeader`: a back button and the page title.
pub fn subpage_header(
    id: impl Into<SharedString>,
    back_label: impl Into<SharedString>,
    title: impl Into<SharedString>,
    on_back: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    div()
        .flex()
        .items_center()
        .gap(theme.metrics.control_gap)
        .child(
            IconButton::new(id.into(), "arrow-left", back_label)
                .on_click(move |_, window, cx| on_back(window, cx)),
        )
        .child(super::ui::heading(title, cx))
        .into_any_element()
}

/// `InputList`: the selectable inputs with add and remove under them.
pub struct InputList {
    pub id: &'static str,
    pub items: Vec<(String, String)>,
    pub selected: String,
    pub empty: SharedString,
    pub add: SharedString,
    pub remove: SharedString,
    pub disabled: bool,
}

impl InputList {
    pub fn render(
        self,
        on_select: impl Fn(String, &mut Window, &mut App) + 'static,
        on_add: impl Fn(&mut Window, &mut App) + 'static,
        on_remove: impl Fn(&mut Window, &mut App) + 'static,
        cx: &App,
    ) -> AnyElement {
        let theme = cx.theme();
        let on_select = std::rc::Rc::new(on_select);
        let (hover, border, strong) = (
            theme.colors.accent_soft,
            theme.colors.border,
            theme.colors.text_strong,
        );
        let items = self.items.iter().map(|(id, label)| {
            let (selected, pick, key) = (
                *id == self.selected,
                on_select.clone(),
                SharedString::from(format!("{}-item-{id}", self.id)),
            );
            let id = id.clone();
            div()
                .id(key)
                .px(theme.metrics.space_sm)
                .py(theme.metrics.space_xs)
                .rounded(theme.metrics.radius_sm)
                .border_1()
                .border_color(if selected {
                    border
                } else {
                    theme.colors.panel_bg_strong
                })
                .text_color(if selected { strong } else { theme.colors.text })
                .cursor_pointer()
                .when(selected, |this| this.bg(hover))
                .hover(move |style| style.bg(hover).border_color(border))
                .on_click(move |_, window, cx| pick(id.clone(), window, cx))
                .child(label.clone())
        });
        div()
            .id(self.id)
            .flex()
            .flex_col()
            .w_full()
            .rounded(theme.metrics.radius_sm)
            .bg(theme.colors.panel_bg_strong)
            .text_size(theme.metrics.font_sm)
            .child(
                div()
                    .id(SharedString::from(format!("{}-items", self.id)))
                    .flex()
                    .flex_col()
                    .gap(theme.metrics.space_2xs)
                    .min_h(px(96.))
                    .max_h(px(180.))
                    .overflow_y_scroll()
                    .p(theme.metrics.space_xs)
                    .children(items)
                    .children(self.items.is_empty().then(|| {
                        div()
                            .p(theme.metrics.space_sm)
                            .text_color(theme.colors.text_muted)
                            .child(self.empty.clone())
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap(theme.metrics.space_2xs)
                    .p(theme.metrics.space_xs)
                    .border_t_1()
                    .border_color(theme.colors.border)
                    .child(
                        IconButton::new(format!("{}-add", self.id), "plus", self.add)
                            .disabled(self.disabled)
                            .on_click(move |_, window, cx| on_add(window, cx)),
                    )
                    .child(
                        IconButton::new(format!("{}-remove", self.id), "minus", self.remove)
                            .disabled(self.disabled || self.selected.is_empty())
                            .on_click(move |_, window, cx| on_remove(window, cx)),
                    ),
            )
            .into_any_element()
    }
}
