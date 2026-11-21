use std::rc::Rc;

use gpui::{
    div, prelude::*, App, Context, ElementId, Entity, FocusHandle, KeyDownEvent, MouseButton,
    Pixels, Point, SharedString, Window,
};
use gpui_base::Button;

use super::{
    child_id, input_state, ActionButton, ButtonVariant, ContextMenu, Dialog, Handler, IconButton,
    InputState, MenuItem, Popover, TextInput,
};
use crate::{icon, ActiveTheme, Translator};

#[derive(Clone)]
pub struct CollectionOption {
    pub id: String,
    pub label: String,
    pub rename: bool,
    pub delete: bool,
    pub movable: bool,
}

impl CollectionOption {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            rename: true,
            delete: true,
            movable: true,
        }
    }
}

#[derive(Clone)]
pub enum CollectionAction {
    Select(String),
    Create,
    Rename {
        id: String,
        title: String,
    },
    Duplicate(String),
    Delete(String),
    Reorder {
        source: String,
        target: String,
        after: bool,
    },
}

struct PickerState {
    open: bool,
    focus: FocusHandle,
    highlighted: usize,
    menu: Option<(String, Point<Pixels>)>,
    rename: Option<(String, Entity<InputState>)>,
    delete: Option<(String, String)>,
    drop: Option<(String, bool)>,
    creating: Option<Vec<String>>,
}

#[derive(Clone)]
struct CollectionDrag {
    scope: String,
    id: String,
    label: String,
}

impl Render for CollectionDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px(theme.metrics.space_sm)
            .py(theme.metrics.space_2xs)
            .bg(theme.colors.popover_bg)
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_sm)
            .child(self.label.clone())
    }
}

/// A collection selector with creation, inline rename, context actions and drag insertion.
#[derive(IntoElement)]
pub struct CollectionPicker {
    id: ElementId,
    label: SharedString,
    options: Vec<CollectionOption>,
    selected: String,
    confirmation: SharedString,
    disabled: bool,
    on_action: Option<Handler<CollectionAction>>,
}

impl CollectionPicker {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        options: Vec<CollectionOption>,
        selected: impl Into<String>,
        confirmation: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            options,
            selected: selected.into(),
            confirmation: confirmation.into(),
            disabled: false,
            on_action: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_action(
        mut self,
        handler: impl Fn(CollectionAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for CollectionPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let scope = self.id.to_string();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, cx| PickerState {
            open: false,
            focus: cx.focus_handle(),
            highlighted: 0,
            menu: None,
            rename: None,
            delete: None,
            drop: None,
            creating: None,
        });
        let created = state
            .read(cx)
            .creating
            .as_ref()
            .and_then(|before| {
                self.options
                    .iter()
                    .find(|option| option.id == self.selected && !before.contains(&option.id))
            })
            .cloned();
        if let Some(created) = created {
            let input = input_state(created.label, window, cx);
            let focus = input.clone();
            state.update(cx, |state, _| {
                state.rename = Some((created.id, input));
                state.creating = None;
            });
            window.defer(cx, move |window, cx| {
                focus.update(cx, |input, cx| {
                    input.focus(window, cx);
                    input.select_all(window, cx);
                })
            });
        }
        let selected = self
            .options
            .iter()
            .position(|option| option.id == self.selected);
        let label = selected.map_or_else(
            || self.label.to_string(),
            |index| self.options[index].label.clone(),
        );
        let trigger_state = state.clone();
        let trigger = Button::new(child_id(&self.id, "trigger"))
            .debug_selector({
                let scope = scope.clone();
                move || format!("collection-trigger-{scope}")
            })
            .disabled(self.disabled)
            .accessibility_label(self.label.clone())
            .h(theme.metrics.control_height_compact)
            .w_auto()
            .justify_start()
            .px(theme.metrics.space_sm)
            .min_w_0()
            .gap(theme.metrics.space_xs)
            .text_size(theme.metrics.font_sm)
            .text_color(theme.colors.text)
            .hover(|style| style.bg(theme.colors.control_bg_hover))
            .focus_visible(|style| super::focus_ring(style, cx))
            .child(div().min_w_0().truncate().child(label))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .text_color(theme.colors.text_muted)
                    .child(icon("chevron-up").size(theme.metrics.space_sm))
                    .child(icon("chevron-down").size(theme.metrics.space_sm)),
            )
            .on_click(move |_, window, cx| {
                trigger_state.update(cx, |state, cx| {
                    state.open = !state.open;
                    state.highlighted = selected.unwrap_or(0);
                    state.menu = None;
                    state.rename = None;
                    state.drop = None;
                    state.creating = None;
                    if state.open {
                        window.focus(&state.focus, cx);
                    }
                    cx.notify();
                });
            });
        let dismiss = state.clone();
        let mut popover =
            Popover::new(child_id(&self.id, "popover"), trigger).on_dismiss(move |_, _, cx| {
                dismiss.update(cx, |state, cx| {
                    // The context menu can extend beyond the collection popup.
                    if state.menu.is_some() {
                        return;
                    }
                    state.open = false;
                    state.menu = None;
                    state.rename = None;
                    state.drop = None;
                    state.creating = None;
                    cx.notify();
                })
            });
        if state.read(cx).open {
            let add = self.on_action.clone();
            let add_state = state.clone();
            let before: Vec<_> = self
                .options
                .iter()
                .map(|option| option.id.clone())
                .collect();
            let mut rows = div()
                .id(child_id(&self.id, "rows"))
                .role(gpui::Role::ListBox)
                .aria_label(self.label.clone())
                .max_h(theme.metrics.row_height * 12.)
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .on_drag_move::<CollectionDrag>({
                    let state = state.clone();
                    move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            if state.drop.take().is_some() {
                                cx.notify();
                            }
                        })
                    }
                });
            for option in &self.options {
                let rename = state
                    .read(cx)
                    .rename
                    .as_ref()
                    .filter(|(id, _)| id == &option.id)
                    .map(|(_, input)| input.clone());
                if let Some(input) = rename {
                    let edit_input = input.clone();
                    let edit_state = state.clone();
                    let handler = self.on_action.clone();
                    let id = option.id.clone();
                    rows = rows.child(
                        div()
                            .debug_selector({
                                let scope = scope.clone();
                                move || format!("collection-rename-{scope}")
                            })
                            .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "escape") {
                                    cx.stop_propagation();
                                    let title = edit_input.read(cx).value().to_string();
                                    edit_state.update(cx, |state, cx| {
                                        state.rename = None;
                                        window.focus(&state.focus, cx);
                                        cx.notify();
                                    });
                                    if event.keystroke.key == "enter" {
                                        if let Some(handler) = &handler {
                                            handler(
                                                CollectionAction::Rename {
                                                    id: id.clone(),
                                                    title,
                                                },
                                                window,
                                                cx,
                                            );
                                        }
                                    }
                                }
                            })
                            .child(
                                TextInput::new(child_id(&self.id, "rename"), &input)
                                    .compact()
                                    .focus_ring(false),
                            ),
                    );
                    continue;
                }
                let id = option.id.clone();
                let select_state = state.clone();
                let handler = self.on_action.clone();
                let menu_state = state.clone();
                let menu_id = id.clone();
                let over_state = state.clone();
                let over = id.clone();
                let over_scope = scope.clone();
                let drop_state = state.clone();
                let target = id.clone();
                let drop_scope = scope.clone();
                let drop_handler = self.on_action.clone();
                let after = state
                    .read(cx)
                    .drop
                    .as_ref()
                    .filter(|(target, _)| target == &id && cx.has_active_drag())
                    .map(|(_, after)| *after);
                let movable = option.movable;
                rows = rows.child(
                    Button::new(child_id(&self.id, id.clone()))
                        .debug_selector({
                            let scope = scope.clone();
                            let id = id.clone();
                            move || format!("collection-row-{scope}-{id}")
                        })
                        .role(gpui::Role::ListBoxOption)
                        .selected(id == self.selected)
                        .disabled(self.disabled)
                        .relative()
                        .w_full()
                        .min_w_0()
                        .min_h(theme.metrics.control_height_compact)
                        .px(theme.metrics.space_sm)
                        .text_size(theme.metrics.font_sm)
                        .justify_start()
                        .when(id == self.selected, |row| row.bg(theme.colors.selected))
                        .when(
                            id != self.selected
                                && self
                                    .options
                                    .get(state.read(cx).highlighted)
                                    .is_some_and(|entry| entry.id == id),
                            |row| row.bg(theme.colors.accent_soft),
                        )
                        .hover(|style| style.bg(theme.colors.accent_soft))
                        .focus_visible(|style| super::focus_ring(style, cx))
                        .child(div().truncate().child(option.label.clone()))
                        .when_some(after, |row, after| {
                            row.child(
                                div()
                                    .debug_selector({
                                        let scope = scope.clone();
                                        move || format!("collection-drop-{scope}")
                                    })
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .h(theme.metrics.space_2xs)
                                    .bg(theme.colors.accent)
                                    .when(after, |line| line.bottom_0())
                                    .when(!after, |line| line.top_0()),
                            )
                        })
                        .on_click(move |_, window, cx| {
                            select_state.update(cx, |state, cx| {
                                state.open = false;
                                state.menu = None;
                                cx.notify();
                            });
                            if let Some(handler) = &handler {
                                handler(CollectionAction::Select(id.clone()), window, cx);
                            }
                        })
                        .on_mouse_down(MouseButton::Right, move |event, _, cx| {
                            cx.stop_propagation();
                            menu_state.update(cx, |state, cx| {
                                state.menu = Some((menu_id.clone(), event.position));
                                cx.notify();
                            });
                        })
                        .when(movable && !self.disabled, |row| {
                            row.on_drag(
                                CollectionDrag {
                                    scope: scope.clone(),
                                    id: option.id.clone(),
                                    label: option.label.clone(),
                                },
                                |drag, _, _, cx| cx.new(|_| drag.clone()),
                            )
                        })
                        .on_drag_move::<CollectionDrag>(move |event, _, cx| {
                            if !movable
                                || event.drag(cx).scope != over_scope
                                || !event.bounds.contains(&event.event.position)
                            {
                                return;
                            }
                            cx.stop_propagation();
                            let next = (event.drag(cx).id != over).then(|| {
                                (
                                    over.clone(),
                                    event.event.position.y > event.bounds.center().y,
                                )
                            });
                            over_state.update(cx, |state, cx| {
                                if state.drop != next {
                                    state.drop = next;
                                    cx.notify();
                                }
                            });
                        })
                        .on_drop::<CollectionDrag>(move |drag, window, cx| {
                            cx.stop_propagation();
                            let destination = drop_state.update(cx, |state, cx| {
                                let drop = state.drop.take();
                                cx.notify();
                                drop
                            });
                            if drag.scope == drop_scope {
                                if let Some((id, after)) =
                                    destination.filter(|(id, _)| id == &target)
                                {
                                    if let Some(handler) = &drop_handler {
                                        handler(
                                            CollectionAction::Reorder {
                                                source: drag.id.clone(),
                                                target: id,
                                                after,
                                            },
                                            window,
                                            cx,
                                        );
                                    }
                                }
                            }
                        }),
                );
            }
            let keyboard_state = state.clone();
            let options = self.options.clone();
            let keyboard_handler = self.on_action.clone();
            let mut content = div()
                .id(child_id(&self.id, "content"))
                .debug_selector({
                    let scope = scope.clone();
                    move || format!("collection-popup-{scope}")
                })
                .track_focus(&state.read(cx).focus)
                .w(theme.metrics.space_lg * 15.)
                .border_1()
                .border_color(theme.colors.border)
                .bg(theme.colors.popover_bg)
                .text_color(theme.colors.text)
                .flex()
                .flex_col()
                .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                    if keyboard_state.read(cx).rename.is_some() {
                        return;
                    }
                    let key = event.keystroke.key.as_str();
                    if matches!(key, "up" | "down" | "enter") && !options.is_empty() {
                        cx.stop_propagation();
                        let chosen = keyboard_state.update(cx, |state, cx| {
                            state.highlighted = state.highlighted.min(options.len() - 1);
                            match key {
                                "up" => state.highlighted = state.highlighted.saturating_sub(1),
                                "down" => {
                                    state.highlighted =
                                        (state.highlighted + 1).min(options.len() - 1)
                                }
                                _ => state.open = false,
                            }
                            cx.notify();
                            options[state.highlighted].id.clone()
                        });
                        if key == "enter" {
                            if let Some(handler) = &keyboard_handler {
                                handler(CollectionAction::Select(chosen), window, cx);
                            }
                        }
                    }
                })
                .child(
                    div()
                        .debug_selector({
                            let scope = scope.clone();
                            move || format!("collection-header-{scope}")
                        })
                        .flex()
                        .items_center()
                        .px(theme.metrics.space_sm)
                        .border_b_1()
                        .border_color(theme.colors.border)
                        .mb(theme.metrics.space_xs)
                        .child(
                            div()
                                .flex_1()
                                .text_size(theme.metrics.font_sm)
                                .child(self.label.clone()),
                        )
                        .child(
                            IconButton::new(
                                format!("collection-add-{scope}"),
                                "plus",
                                t.t("common.add"),
                            )
                            .disabled(self.disabled)
                            .on_click(move |_, window, cx| {
                                add_state.update(cx, |state, cx| {
                                    state.creating = Some(before.clone());
                                    state.menu = None;
                                    cx.notify();
                                });
                                if let Some(handler) = &add {
                                    handler(CollectionAction::Create, window, cx);
                                }
                            }),
                        ),
                )
                .child(rows);
            if let Some((id, position)) = state.read(cx).menu.clone() {
                if let Some(option) = self.options.iter().find(|entry| entry.id == id) {
                    let rename_state = state.clone();
                    let rename_id = id.clone();
                    let label = option.label.clone();
                    let duplicate_state = state.clone();
                    let duplicate_id = id.clone();
                    let duplicate = self.on_action.clone();
                    let delete_state = state.clone();
                    let delete_id = id.clone();
                    let delete_label = option.label.clone();
                    let close = state.clone();
                    content = content.child(ContextMenu::new(
                        child_id(&self.id, "menu"),
                        position,
                        vec![
                            MenuItem::new(t.t("common.rename"))
                                .disabled(!option.rename || self.disabled)
                                .on_click(move |_, window, cx| {
                                    let input = input_state(label.clone(), window, cx);
                                    input.update(cx, |input, cx| {
                                        input.focus(window, cx);
                                        input.select_all(window, cx);
                                    });
                                    rename_state.update(cx, |state, cx| {
                                        state.rename = Some((rename_id.clone(), input));
                                        cx.notify();
                                    });
                                })
                                .into(),
                            MenuItem::new(t.t("common.duplicate"))
                                .disabled(self.disabled)
                                .on_click(move |_, window, cx| {
                                    duplicate_state.update(cx, |state, cx| {
                                        state.menu = None;
                                        cx.notify();
                                    });
                                    if let Some(handler) = &duplicate {
                                        handler(
                                            CollectionAction::Duplicate(duplicate_id.clone()),
                                            window,
                                            cx,
                                        );
                                    }
                                })
                                .into(),
                            MenuItem::new(t.t("common.delete"))
                                .disabled(!option.delete || self.disabled)
                                .on_click(move |_, _, cx| {
                                    delete_state.update(cx, |state, cx| {
                                        state.delete =
                                            Some((delete_id.clone(), delete_label.clone()));
                                        state.open = false;
                                        cx.notify();
                                    });
                                })
                                .into(),
                        ],
                        move |_, _, cx| {
                            close.update(cx, |state, cx| {
                                state.menu = None;
                                cx.notify();
                            })
                        },
                    ));
                }
            }
            popover = popover.content(content);
        }
        let dialog = state.read(cx).delete.clone().map(|(id, label)| {
            let cancel = state.clone();
            let back = state.clone();
            let confirm = state.clone();
            let handler = self.on_action.clone();
            Dialog::new(
                child_id(&self.id, "delete"),
                t.t("common.delete"),
                t.t("common.cancel"),
                move |_, _, cx| {
                    cancel.update(cx, |state, cx| {
                        state.delete = None;
                        cx.notify();
                    })
                },
            )
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(label))
            .child(self.confirmation.clone())
            .footer(
                ActionButton::new(
                    format!("collection-delete-cancel-{scope}"),
                    t.t("common.cancel"),
                )
                .variant(ButtonVariant::Secondary)
                .on_click(move |_, _, cx| {
                    back.update(cx, |state, cx| {
                        state.delete = None;
                        cx.notify();
                    })
                }),
            )
            .footer(
                ActionButton::new(
                    format!("collection-delete-confirm-{scope}"),
                    t.t("common.delete"),
                )
                .variant(ButtonVariant::Danger)
                .on_click(move |_, window, cx| {
                    confirm.update(cx, |state, cx| {
                        state.delete = None;
                        cx.notify();
                    });
                    if let Some(handler) = &handler {
                        handler(CollectionAction::Delete(id.clone()), window, cx);
                    }
                }),
            )
        });
        div()
            .flex()
            .items_start()
            .justify_start()
            .child(div().flex_none().max_w_full().child(popover))
            .children(dialog)
    }
}
