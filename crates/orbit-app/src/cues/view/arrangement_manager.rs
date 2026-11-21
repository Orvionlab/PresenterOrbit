use gpui::{div, prelude::*, AnyElement, Context, Render, SharedString, Window};
use orbit_ui::{
    controls::{list_row, ActionButton, ButtonVariant, Dialog, EmptyState, IconButton},
    ActiveTheme, Translator,
};

use super::{arrangements, CueView, FormKind};

pub(super) struct Manager {
    item: String,
    selected: String,
    drop: Option<(String, bool)>,
}

#[derive(Clone)]
struct ArrangementDrag {
    id: String,
    label: SharedString,
}

impl Render for ArrangementDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .bg(theme.colors.popover_bg)
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_sm)
            .px(theme.metrics.space_sm)
            .py(theme.metrics.space_xs)
            .child(self.label.clone())
    }
}

impl CueView {
    pub(super) fn open_arrangement_manager(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.arrangement_document(cx) else {
            return;
        };
        self.arrangement_manager = Some(Manager {
            item: document.item.id,
            selected: if document.arrangement.is_empty() {
                document
                    .item
                    .arrangements
                    .first()
                    .map(|a| a.id.clone())
                    .unwrap_or_default()
            } else {
                document.arrangement
            },
            drop: None,
        });
        cx.notify();
    }

    pub(super) fn select_created_arrangement(
        &mut self,
        item: &str,
        id: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(manager) = self
            .arrangement_manager
            .as_mut()
            .filter(|manager| manager.item == item)
        {
            manager.selected = id;
            cx.notify();
        } else {
            self.select_arrangement(&id, cx);
        }
    }

    fn open_manager_form(&mut self, kind: FormKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self
            .arrangement_manager
            .as_ref()
            .and_then(|manager| self.runtime.content.read(cx).items.get(&manager.item))
            .cloned()
        else {
            return;
        };
        self.open_item_form(kind, &item, window, cx);
    }

    fn remove_managed_arrangement(&mut self, cx: &mut Context<Self>) {
        let Some(manager) = &mut self.arrangement_manager else {
            return;
        };
        let Some(item) = self.runtime.content.read(cx).items.get(&manager.item) else {
            return;
        };
        let Some(index) = item
            .arrangements
            .iter()
            .position(|a| a.id == manager.selected)
        else {
            return;
        };
        let next = item
            .arrangements
            .get(index + 1)
            .or_else(|| {
                index
                    .checked_sub(1)
                    .and_then(|previous| item.arrangements.get(previous))
            })
            .map(|a| a.id.clone())
            .unwrap_or_default();
        if let Some(task) =
            arrangements::delete_arrangement(&self.runtime, cx, &manager.item, &manager.selected)
        {
            task.detach();
            manager.selected = next;
            cx.notify();
        }
    }

    fn drop_managed_arrangement(&mut self, source: &str, target: &str, cx: &mut Context<Self>) {
        let Some(manager) = &mut self.arrangement_manager else {
            return;
        };
        let drop = manager.drop.take();
        cx.notify();
        let Some((_, after)) = drop.filter(|(id, _)| id == target) else {
            return;
        };
        let Some(item) = self.runtime.content.read(cx).items.get(&manager.item) else {
            return;
        };
        let index = |id: &str| item.arrangements.iter().position(|a| a.id == id);
        let (Some(from), Some(to)) = (index(source), index(target)) else {
            return;
        };
        let gap = to + usize::from(after);
        let to = gap - usize::from(from < gap);
        if let Some(task) =
            arrangements::move_arrangement(&self.runtime, cx, &manager.item, from, to)
        {
            task.detach();
            manager.selected = source.into();
        }
    }

    pub(super) fn arrangement_manager_element(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let manager = self.arrangement_manager.as_ref()?;
        let item = self.runtime.content.read(cx).items.get(&manager.item)?;
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let enabled = item
            .arrangements
            .iter()
            .any(|arrangement| arrangement.id == manager.selected);
        let toolbar = div()
            .flex()
            .items_center()
            .gap(theme.metrics.space_xs)
            .child(
                IconButton::new("arrangement-manager-add", "plus", t.t("arrangements.add"))
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.open_manager_form(FormKind::NewArrangement, window, cx)
                    })),
            )
            .child(
                IconButton::new("arrangement-manager-remove", "minus", t.t("common.delete"))
                    .disabled(!enabled)
                    .on_click(cx.listener(|view, _, _, cx| view.remove_managed_arrangement(cx))),
            )
            .child(
                IconButton::new("arrangement-manager-rename", "pencil", t.t("common.rename"))
                    .disabled(!enabled)
                    .on_click(cx.listener(|view, _, window, cx| {
                        if let Some(manager) = &view.arrangement_manager {
                            view.open_manager_form(
                                FormKind::RenameArrangement(manager.selected.clone()),
                                window,
                                cx,
                            );
                        }
                    })),
            )
            .child(div().flex_1())
            .child(
                ActionButton::new("arrangement-manager-duplicate", t.t("common.duplicate"))
                    .compact()
                    .variant(ButtonVariant::Secondary)
                    .disabled(!enabled)
                    .on_click(cx.listener(|view, _, _, cx| {
                        let Some(manager) = &mut view.arrangement_manager else {
                            return;
                        };
                        if let Some((id, task)) = arrangements::duplicate_arrangement(
                            &view.runtime,
                            cx,
                            &manager.item,
                            &manager.selected,
                        ) {
                            task.detach();
                            manager.selected = id;
                            cx.notify();
                        }
                    })),
            );
        let rows = item.arrangements.iter().map(|arrangement| {
            let id = arrangement.id.clone();
            let label = SharedString::from(arrangement.name.clone());
            let marker = manager
                .drop
                .as_ref()
                .filter(|(target, _)| target == &id && cx.has_active_drag());
            list_row(
                SharedString::from(format!("arrangement-manager-{id}")),
                manager.selected == id,
                cx,
            )
            .relative()
            .cursor_pointer()
            .aria_label(label.clone())
            .debug_selector({
                let id = id.clone();
                move || format!("arrangement-manager-row-{id}")
            })
            .child(div().min_w_0().truncate().child(label.clone()))
            .when_some(marker, |row, (_, after)| {
                row.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .debug_selector(|| "arrangement-manager-drop".into())
                        .h(theme.metrics.space_2xs)
                        .bg(theme.colors.accent)
                        .when(*after, |line| line.bottom_0())
                        .when(!after, |line| line.top_0()),
                )
            })
            .on_click({
                let id = id.clone();
                cx.listener(move |view, event: &gpui::ClickEvent, window, cx| {
                    if let Some(manager) = &mut view.arrangement_manager {
                        manager.selected = id.clone();
                        cx.notify();
                    }
                    if event.click_count() >= 2 {
                        view.open_manager_form(FormKind::RenameArrangement(id.clone()), window, cx);
                    }
                })
            })
            .on_drag(
                ArrangementDrag {
                    id: id.clone(),
                    label,
                },
                |drag, _, _, cx| cx.new(|_| drag.clone()),
            )
            .on_drag_move::<ArrangementDrag>({
                let id = id.clone();
                cx.listener(
                    move |view, event: &gpui::DragMoveEvent<ArrangementDrag>, _, cx| {
                        if !event.bounds.contains(&event.event.position) {
                            return;
                        }
                        let mark = (event.drag(cx).id != id).then(|| {
                            (id.clone(), event.event.position.y > event.bounds.center().y)
                        });
                        if let Some(manager) = &mut view.arrangement_manager {
                            if manager.drop != mark {
                                manager.drop = mark;
                                cx.notify();
                            }
                        }
                    },
                )
            })
            .on_drop::<ArrangementDrag>(cx.listener(
                move |view, drag: &ArrangementDrag, _, cx| {
                    view.drop_managed_arrangement(&drag.id, &id, cx);
                },
            ))
        });
        let weak = cx.weak_entity();
        Some(
            Dialog::new(
                "arrangement-manager",
                format!("{} · {}", t.t("arrangements.edit"), item.title),
                t.t("common.close"),
                move |_, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.arrangement_manager = None;
                        cx.notify();
                    })
                    .ok();
                },
            )
            .width(theme.metrics.control_height * 15.)
            .child(toolbar)
            .child(
                div()
                    .id("arrangement-manager-list")
                    .debug_selector(|| "arrangement-manager-list".into())
                    .flex()
                    .flex_col()
                    .h(theme.metrics.row_height * 7.)
                    .overflow_y_scroll()
                    .bg(theme.colors.field_bg)
                    .when(item.arrangements.is_empty(), |list| {
                        list.child(EmptyState::new(t.t("arrangements.empty")))
                    })
                    .children(rows),
            )
            .into_any_element(),
        )
    }
}
