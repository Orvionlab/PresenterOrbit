//! `SlideLabelsSettingsPanel.tsx` and `slide-label-utils.ts`: the configured slide labels, which the cue
//! context menu assigns and the cue cards show. The list is a general setting; the host normalizes it.

use gpui::{
    div, prelude::*, AnyElement, Context, Entity, MouseButton, MouseDownEvent, Pixels, Point,
    SharedString, Subscription, Window,
};
use orbit_ui::controls::{
    input_state, ActionButton, ColorPicker, ContextMenu, InputEvent, InputState, MenuItem,
    TextInput,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::view::SettingsView;

use super::groups::DEFAULT_COLOR as NEW_LABEL_COLOR;
const COLOR_COLUMN: Pixels = gpui::px(72.);

#[derive(Clone, Debug, PartialEq)]
pub struct SlideLabel {
    pub id: String,
    pub name: String,
    pub color: String,
}

/// The labels of the saved general settings; an entry without id, name, and color is not a label.
pub fn slide_labels(general: &Value) -> Vec<SlideLabel> {
    general["slideLabels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|label| {
            let id = label["id"].as_str().filter(|id| !id.is_empty())?;
            Some(SlideLabel {
                id: id.to_string(),
                name: label["name"].as_str()?.to_string(),
                color: label["color"].as_str()?.to_string(),
            })
        })
        .collect()
}

pub fn labels_json(labels: &[SlideLabel]) -> Value {
    json!(labels
        .iter()
        .map(|label| json!({"id": label.id, "name": label.name, "color": label.color}))
        .collect::<Vec<_>>())
}

/// `addSlideLabel`: the first `<base> <n>` and `label-<n>` that no label uses.
pub fn add_label(labels: &[SlideLabel], base_name: &str) -> Vec<SlideLabel> {
    let mut number = 1;
    while labels.iter().any(|label| {
        label.name == format!("{base_name} {number}") || label.id == format!("label-{number}")
    }) {
        number += 1;
    }
    labels
        .iter()
        .cloned()
        .chain([SlideLabel {
            id: format!("label-{number}"),
            name: format!("{base_name} {number}"),
            color: NEW_LABEL_COLOR.into(),
        }])
        .collect()
}

/// `updateSlideLabel`: a blank name, or one that another label has, changes nothing (`None`).
pub fn update_label(
    labels: &[SlideLabel],
    id: &str,
    name: Option<&str>,
    color: Option<&str>,
) -> Option<Vec<SlideLabel>> {
    let name = name.map(str::trim);
    if let Some(name) = name {
        if name.is_empty()
            || labels
                .iter()
                .any(|label| label.id != id && label.name == name)
        {
            return None;
        }
    }
    Some(
        labels
            .iter()
            .map(|label| {
                if label.id != id {
                    return label.clone();
                }
                SlideLabel {
                    id: label.id.clone(),
                    name: name.map_or_else(|| label.name.clone(), str::to_string),
                    color: color.map_or_else(|| label.color.clone(), str::to_string),
                }
            })
            .collect(),
    )
}

pub fn delete_label(labels: &[SlideLabel], id: &str) -> Vec<SlideLabel> {
    labels
        .iter()
        .filter(|label| label.id != id)
        .cloned()
        .collect()
}

pub struct Rename {
    id: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

#[derive(Default)]
pub struct SlideLabels {
    pub renaming: Option<Rename>,
    pub menu: Option<(String, Point<Pixels>)>,
}

impl SettingsView {
    pub(super) fn save_labels(&mut self, labels: &[SlideLabel], cx: &mut Context<Self>) {
        self.patch_general(json!({ "slideLabels": labels_json(labels) }), cx);
    }

    fn current_labels(&self, cx: &gpui::App) -> Vec<SlideLabel> {
        slide_labels(&self.settings(cx)["generalSettings"])
    }

    fn begin_label_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(label) = self
            .current_labels(cx)
            .into_iter()
            .find(|label| label.id == id)
        else {
            return;
        };
        let input = input_state(label.name, window, cx);
        let owner = id.to_string();
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    return;
                }
                if !this
                    .slide_labels
                    .renaming
                    .as_ref()
                    .is_some_and(|rename| rename.id == owner)
                {
                    return;
                }
                this.slide_labels.renaming = None;
                let name = input.read(cx).value().to_string();
                let labels = this.current_labels(cx);
                if let Some(next) = update_label(&labels, &owner, Some(&name), None) {
                    if next != labels {
                        this.save_labels(&next, cx);
                    }
                }
                cx.notify();
            },
        );
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        self.slide_labels.menu = None;
        self.slide_labels.renaming = Some(Rename {
            id: id.to_string(),
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn label_row(&mut self, label: &SlideLabel, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let id = label.id.clone();
        let name: AnyElement = match &self.slide_labels.renaming {
            Some(rename) if rename.id == label.id => {
                let owner = id.clone();
                div()
                    .flex_1()
                    .on_key_down(cx.listener(move |view, event: &gpui::KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape"
                            && view
                                .slide_labels
                                .renaming
                                .as_ref()
                                .is_some_and(|rename| rename.id == owner)
                        {
                            view.slide_labels.renaming = None;
                            cx.notify();
                        }
                    }))
                    .child(
                        TextInput::new(
                            SharedString::from(format!("slide-label-rename-{id}")),
                            &rename.input,
                        )
                        .label(label.name.clone()),
                    )
                    .into_any_element()
            }
            _ => {
                let rename_id = id.clone();
                div()
                    .id(SharedString::from(format!("slide-label-name-{id}")))
                    .flex_1()
                    .min_w_0()
                    .cursor_pointer()
                    .child(label.name.clone())
                    .on_click(
                        cx.listener(move |view, event: &gpui::ClickEvent, window, cx| {
                            if event.click_count() >= 2 {
                                view.begin_label_rename(&rename_id, window, cx);
                            }
                        }),
                    )
                    .into_any_element()
            }
        };
        let (color_id, menu_id) = (id.clone(), id.clone());
        let weak = cx.weak_entity();
        div()
            .id(SharedString::from(format!("slide-label-row-{id}")))
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.metrics.space_sm)
            .min_h(theme.metrics.control_height)
            .px(theme.metrics.space_sm)
            .rounded(theme.metrics.radius_sm)
            .bg(theme.colors.panel_bg_soft)
            .text_size(theme.metrics.font_sm)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                    view.slide_labels.menu = Some((menu_id.clone(), event.position));
                    cx.notify();
                }),
            )
            .child(name)
            .child(
                div().w(COLOR_COLUMN).flex().justify_center().child(
                    div().w(gpui::px(54.)).child(
                        ColorPicker::new(
                            SharedString::from(format!("slide-label-color-{color_id}")),
                            label.name.clone(),
                            label.color.clone(),
                        )
                        .on_change(move |color, _, cx| {
                            weak.update(cx, |view, cx| {
                                let labels = view.current_labels(cx);
                                if let Some(next) =
                                    update_label(&labels, &color_id, None, Some(&color))
                                {
                                    view.save_labels(&next, cx);
                                }
                            })
                            .ok();
                        }),
                    ),
                ),
            )
            .into_any_element()
    }

    pub(super) fn slide_labels_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let labels = self.current_labels(cx);
        let rows: Vec<AnyElement> = labels
            .iter()
            .map(|label| self.label_row(label, cx))
            .collect();
        let menu = self.slide_labels.menu.clone().map(|(id, position)| {
            let (weak, close) = (cx.weak_entity(), cx.weak_entity());
            ContextMenu::new(
                "slide-label-menu",
                position,
                vec![MenuItem::new(t.t("settings.slideLabels.delete"))
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.slide_labels.menu = None;
                            let labels = view.current_labels(cx);
                            view.save_labels(&delete_label(&labels, &id), cx);
                        })
                        .ok();
                    })
                    .into()],
                move |_, _, cx| {
                    let close = close.clone();
                    cx.defer(move |cx| {
                        close
                            .update(cx, |view, cx| {
                                view.slide_labels.menu = None;
                                cx.notify();
                            })
                            .ok();
                    });
                },
            )
        });
        let header = |label: String, width: Option<Pixels>| {
            div()
                .when_some(width, |this, width| {
                    this.w(width).flex().justify_center().flex_none()
                })
                .when(width.is_none(), |this| this.flex_1().min_w_0())
                .child(label.to_uppercase())
        };
        div()
            .id("settings-slide-labels")
            .flex()
            .flex_col()
            .size_full()
            .gap(theme.metrics.panel_gap)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(theme.metrics.space_sm)
                    .min_h(theme.metrics.header_height)
                    .px(theme.metrics.space_sm)
                    .text_size(theme.metrics.font_xs)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(theme.colors.text_strong)
                    .child(header(t.t("settings.slideLabels.name"), None))
                    .child(header(
                        t.t("settings.slideLabels.color"),
                        Some(COLOR_COLUMN),
                    )),
            )
            .child(
                div()
                    .id("settings-slide-labels-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(gpui::px(1.))
                    .children(rows),
            )
            .child(
                ActionButton::new("slide-labels-add", t.t("settings.slideLabels.add")).on_click(
                    cx.listener(|view, _, _, cx| {
                        let t = *cx.global::<Translator>();
                        let labels = view.current_labels(cx);
                        let next = add_label(&labels, &t.t("settings.slideLabels.newName"));
                        view.save_labels(&next, cx);
                    }),
                ),
            )
            .children(menu)
            .into_any_element()
    }
}
