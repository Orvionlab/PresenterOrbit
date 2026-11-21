//! The Manage Workspaces window, a port of `ManageWorkspacesDialog.tsx`: the registered workspaces with the tools
//! of a ProPresenter list footer. Naming and deleting open the prompt window.

use gpui::{
    div, prelude::*, px, Context, Entity, FontWeight, MouseButton, Pixels, Point, SharedString,
    Subscription, WeakEntity, Window,
};
use orbit_host::Host;
use orbit_ui::controls::{list_row, ContextMenu, IconButton, MenuEntry, MenuItem};
use orbit_ui::{icon, ActiveTheme, Translator};

use super::{request_prompt, Item, Prompt, SharedWorkspaces, Workspaces};

#[derive(Clone, Copy, PartialEq)]
enum MenuKind {
    Add,
    Row,
}

struct Menu {
    kind: MenuKind,
    anchor: Point<Pixels>,
}

pub struct ManageView {
    host: Host,
    workspaces: Entity<Workspaces>,
    selected: String,
    menu: Option<Menu>,
    _observe: Subscription,
}

impl ManageView {
    pub fn new(host: Host, cx: &mut Context<Self>) -> Self {
        let workspaces = cx.global::<SharedWorkspaces>().0.clone();
        workspaces.update(cx, |workspaces, cx| workspaces.refresh(cx));
        Self {
            host,
            _observe: cx.observe(&workspaces, |_, _, cx| cx.notify()),
            workspaces,
            selected: String::new(),
            menu: None,
        }
    }

    /// The list is read again whenever the window is shown, as it is when it mounts.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        self.workspaces
            .update(cx, |workspaces, cx| workspaces.refresh(cx));
    }

    /// The selected row, or the active workspace while nothing is selected.
    fn selection(&self, items: &[Item]) -> Option<Item> {
        items
            .iter()
            .find(|item| item.path == self.selected)
            .or_else(|| items.iter().find(|item| item.active))
            .cloned()
    }

    fn open_menu(&mut self, kind: MenuKind, anchor: Point<Pixels>, cx: &mut Context<Self>) {
        self.menu = Some(Menu { kind, anchor });
        cx.notify();
    }

    fn prompt(&mut self, prompt: Prompt, cx: &mut Context<Self>) {
        self.menu = None;
        cx.notify();
        request_prompt(self.host.clone(), prompt, cx);
    }

    fn menu_entries(
        weak: WeakEntity<Self>,
        kind: MenuKind,
        selection: Option<&Item>,
        t: Translator,
    ) -> Vec<MenuEntry> {
        let item = |label: String, action: fn(&mut Self, Option<String>, &mut Context<Self>)| {
            let weak = weak.clone();
            let path = selection.map(|item| item.path.clone());
            MenuItem::new(label).on_click(move |_, _, cx| {
                let path = path.clone();
                weak.update(cx, |view, cx| action(view, path, cx)).ok();
            })
        };
        match kind {
            MenuKind::Add => vec![
                item(t.t("workspace.newMenu"), |view, _, cx| {
                    view.prompt(Prompt::New, cx)
                })
                .into(),
                item(t.t("workspace.openFolderMenu"), |view, _, cx| {
                    view.menu = None;
                    view.workspaces
                        .update(cx, |workspaces, cx| workspaces.choose_folder(cx));
                    cx.notify();
                })
                .into(),
            ],
            MenuKind::Row => {
                let Some(selection) = selection else {
                    return Vec::new();
                };
                vec![
                    item(t.t("workspace.duplicateMenu"), |view, path, cx| {
                        if let Some(path) = path {
                            view.prompt(Prompt::Duplicate(path), cx)
                        }
                    })
                    .disabled(selection.missing)
                    .into(),
                    item(t.t("common.rename"), |view, path, cx| {
                        if let Some(path) = path {
                            view.prompt(Prompt::Rename(path), cx)
                        }
                    })
                    .into(),
                    item(t.t("workspace.revealMenu"), |view, path, cx| {
                        view.menu = None;
                        if let Some(path) = path {
                            view.workspaces
                                .update(cx, |workspaces, cx| workspaces.reveal(&path, cx));
                        }
                        cx.notify();
                    })
                    .disabled(selection.missing)
                    .into(),
                ]
            }
        }
    }
}

impl Render for ManageView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let (items, error) = {
            let workspaces = self.workspaces.read(cx);
            (workspaces.items.clone(), workspaces.error.clone())
        };
        let selection = self.selection(&items);
        let list = items.iter().enumerate().map(|(index, item)| {
            let path = item.path.clone();
            let (select, menu) = (path.clone(), path);
            list_row(
                SharedString::from(format!("workspace-row-{index}")),
                item.path == self.selected,
                cx,
            )
            .h_auto()
            .py(theme.metrics.space_xs)
            .on_click(cx.listener(move |view, _, _, cx| {
                view.selected = select.clone();
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                    view.selected = menu.clone();
                    view.open_menu(MenuKind::Row, event.position, cx);
                }),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(38.))
                    .flex()
                    .justify_center()
                    .children(
                        item.active
                            .then(|| icon("check").text_color(theme.colors.text)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(item.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(theme.metrics.font_sm)
                            .text_color(theme.colors.text_muted)
                            .child(if item.missing {
                                t.t("workspace.missing")
                            } else {
                                item.folder()
                            }),
                    ),
            )
        });
        let removable = selection.as_ref().is_some_and(|item| !item.active);
        let has_selection = selection.is_some();
        let remove_path = selection.as_ref().map(|item| item.path.clone());
        let menu = self.menu.as_ref().map(|menu| {
            ContextMenu::new(
                "workspace-menu",
                menu.anchor,
                Self::menu_entries(cx.weak_entity(), menu.kind, selection.as_ref(), t),
                {
                    let weak = cx.weak_entity();
                    move |_, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.menu = None;
                            cx.notify();
                        })
                        .ok();
                    }
                },
            )
        });
        div()
            .id("manage-workspaces")
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .child(
                div()
                    .id("manage-workspaces-list")
                    .flex()
                    .flex_col()
                    .mt(theme.metrics.space_md)
                    .mx(theme.metrics.panel_padding)
                    .max_h_full()
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(theme.colors.border)
                    .bg(theme.colors.panel_bg_strong)
                    .children(list),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(theme.metrics.control_gap)
                    .mx(theme.metrics.panel_padding)
                    .mb(theme.metrics.space_md)
                    .border_1()
                    .border_t_0()
                    .border_color(theme.colors.border)
                    .bg(theme.colors.panel_bg)
                    .child(
                        IconButton::new("workspace-add", "plus", t.t("workspace.add")).on_click(
                            cx.listener(|view, _, window, cx| {
                                view.open_menu(MenuKind::Add, window.mouse_position(), cx)
                            }),
                        ),
                    )
                    .child(
                        IconButton::new("workspace-remove", "trash", t.t("workspace.remove"))
                            .disabled(!removable)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if let Some(path) = remove_path.clone() {
                                    view.prompt(Prompt::Delete(path), cx);
                                }
                            })),
                    )
                    .child(
                        IconButton::new("workspace-more", "settings", t.t("workspace.more"))
                            .disabled(!has_selection)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.open_menu(MenuKind::Row, window.mouse_position(), cx)
                            })),
                    ),
            )
            .children(error.map(|error| {
                div()
                    .mx(theme.metrics.panel_padding)
                    .mb(theme.metrics.space_md)
                    .child(error)
            }))
            .children(menu)
    }
}
