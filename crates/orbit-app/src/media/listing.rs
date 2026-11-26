use super::*;
use gpui::{AnyElement, App, Div};
use orbit_ui::controls::{InputEvent, SegmentedControl, SegmentedOption};

fn columns(cells: [AnyElement; 6], cx: &App) -> Div {
    let m = cx.theme().metrics;
    let widths = [
        Some(m.control_height_compact),
        Some(m.control_height_compact),
        Some(m.control_height * 2.),
        Some(m.control_height * 3.),
        None,
        None,
    ];
    let mut row = div().size_full().flex().items_center().gap(m.space_xs);
    for (cell, width) in cells.into_iter().zip(widths) {
        row = row.child(
            div()
                .min_w_0()
                .overflow_hidden()
                .when_some(width, |el, width| el.w(width).flex_none())
                .when(width.is_none(), |el| el.flex_1())
                .child(cell),
        );
    }
    row
}

impl MediaBin {
    pub(super) fn filter_rows(&mut self) {
        let words: Vec<_> = self
            .filter_query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                if words.is_empty() {
                    return Some(index);
                }
                let searchable =
                    format!("{} {}", text(item, "title"), text(item, "sourcePath")).to_lowercase();
                words
                    .iter()
                    .all(|word| searchable.contains(word))
                    .then_some(index)
            })
            .collect();
        self.card_bounds.borrow_mut().clear();
    }

    pub(super) fn filter_control(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let input = self.filter_input.get_or_insert_with(|| {
            let input = orbit_ui::controls::filter_state(self.filter_query.clone(), window, cx);
            let subscription = cx.subscribe(&input, |view, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    view.filter_query = input.read(cx).value().to_string();
                    view.graphics_cancel(cx);
                    view.filter_rows();
                    view.grid_scroll
                        .scroll_to_item(0, gpui::ScrollStrategy::Top);
                    cx.notify();
                }
            });
            self._subscriptions.push(subscription);
            input
        });
        div()
            .w(gpui::relative(0.8))
            .min_w_0()
            .child(
                orbit_ui::controls::FilterInput::new("media-filter", input)
                    .label(t.t("media.filter")),
            )
            .into_any_element()
    }

    pub(super) fn view_control(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let owner = cx.weak_entity();
        div()
            .debug_selector(|| "media-view-controls".to_string())
            .flex_none()
            .child(
                SegmentedControl::new(
                    "media-view",
                    t.t("media.view.label"),
                    vec![
                        SegmentedOption::icon(true, "layout-grid", t.t("media.view.grid")),
                        SegmentedOption::icon(false, "list", t.t("media.view.list")),
                    ],
                    self.grid,
                )
                .on_change(move |grid, _, cx| {
                    owner
                        .update(cx, |view, cx| {
                            view.grid = grid;
                            view.graphics_cancel(cx);
                            view.card_bounds.borrow_mut().clear();
                            view.grid_scroll
                                .scroll_to_item(0, gpui::ScrollStrategy::Top);
                            cx.notify();
                        })
                        .ok();
                }),
            )
    }

    pub(super) fn table_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let labels = [
            "#".to_string(),
            t.t("media.table.layer"),
            t.t("media.table.thumbnail"),
            t.t("media.table.transition"),
            t.t("media.inspector.name"),
            t.t("media.table.path"),
        ];
        div()
            .debug_selector(|| "media-table-header".to_string())
            .h(theme.metrics.header_height)
            .flex_none()
            .px(theme.metrics.space_sm)
            .bg(theme.colors.panel_bg_strong)
            .text_color(theme.colors.text_muted)
            .text_size(theme.metrics.font_xs)
            .child(columns(
                labels.map(|label| div().truncate().child(label).into_any_element()),
                cx,
            ))
    }

    pub(super) fn table_row(&self, item: &Value, index: usize, cx: &Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let live = &self.runtime.output.read(cx).live;
        let layer = menu::option_value(item, live, "backgroundBehavior");
        let layer_label = t.t(if layer == "background" {
            "media.behavior.background"
        } else {
            "media.behavior.foreground"
        });
        let layer_selector = format!("media-layer-{layer}-{}", text(item, "id"));
        let transition = menu::option_value(item, live, "backgroundTransition");
        let transition = match (layer.as_str(), transition.as_str()) {
            ("foreground", _) => t.t("media.transition.slideDefault"),
            (_, "none") => t.t("media.transition.layerDefault"),
            (_, "cut") => t.t("media.transition.cut"),
            (_, "dissolve") => t.t("media.transition.dissolve"),
            _ => transition,
        };
        let picture = div()
            .relative()
            .overflow_hidden()
            .h(theme.metrics.control_height)
            .w_full()
            .flex()
            .items_center()
            .when(
                self.details
                    .get(text(item, "id"))
                    .is_some_and(|details| details.transparent),
                |picture| picture.child(assets::transparency_background(text(item, "id"), cx)),
            );
        let picture = if let Some(image) = self
            .details
            .get(text(item, "id"))
            .and_then(|details| details.image.clone())
        {
            picture.child(
                gpui::img(image)
                    .absolute()
                    .inset_0()
                    .size_full()
                    .aspect_ratio(16. / 9.)
                    .object_fit(gpui::ObjectFit::Contain),
            )
        } else {
            picture.child(orbit_ui::icon(if item["type"] == "video" {
                "video"
            } else {
                "polaroid"
            }))
        };
        columns(
            [
                div().child(index.to_string()).into_any_element(),
                div()
                    .id(SharedString::from(format!(
                        "media-layer-{}",
                        text(item, "id")
                    )))
                    .debug_selector(move || layer_selector.clone())
                    .aria_label(layer_label.clone())
                    .tooltip(orbit_ui::controls::tooltip_text(layer_label))
                    .child(orbit_ui::icons::media_layer_icon(
                        layer == "foreground",
                        &theme,
                    ))
                    .into_any_element(),
                picture.into_any_element(),
                div().truncate().child(transition).into_any_element(),
                div()
                    .truncate()
                    .child(text(item, "title").to_string())
                    .into_any_element(),
                div()
                    .truncate()
                    .text_color(theme.colors.text_muted)
                    .child(text(item, "sourcePath").to_string())
                    .into_any_element(),
            ],
            cx,
        )
        .into_any_element()
    }
}
