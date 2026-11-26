//! The library tree and the playlist tree: selection, inline rename, context menus, collapse, and drag and drop
//! with insertion lines. Ports of `CollectionBrowserTop.tsx`, `CollectionLibraryTreeItem.tsx`, and the tree parts of
//! `CollectionMenuLayer.tsx`. The view only forwards events to the controllers in `tree_actions.rs`.

use gpui::{
    div, prelude::*, px, AnyElement, App, ClickEvent, Context, DragMoveEvent, Entity, MouseButton,
    MouseDownEvent, Render, SharedString, Subscription, Window,
};
use orbit_ui::controls::{
    list_row, ActionButton, ButtonVariant, ContextMenu, Dialog, IconButton, InputEvent, InputState,
    MenuEntry, MenuItem, PanelHeader, TextInput,
};
use orbit_ui::{icon, ActiveTheme, Translator};

use super::drag::{
    after_folder_as_root, is_noop_library_drop, is_noop_playlist_drop, tree_drop_mode, TreeKind,
};
use super::model::{parent_id, DropMode, Library, Playlist, ThemeAssignment};
use super::selection::{
    is_library_selected, is_playlist_selected, select_library, select_playlist,
    selected_library_ids, selected_playlist_ids, Modifiers,
};
use super::tree_actions::{
    self, delete_libraries_message, deleted_item_ids, handle_library_menu_action,
    handle_playlist_menu_action, DetachExt, LibraryMenuOutcome,
};
use crate::state::runtime::Runtime;
use crate::state::slices::{CreateMenu, CreateMenuKind, TreeMenu};
use crate::theme_picker::ThemePicker;

/// The payload of a drag in either tree.
#[derive(Clone)]
pub struct TreeDrag {
    kind: TreeKind,
    id: String,
    title: SharedString,
}

struct DragPreview(SharedString);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px(theme.metrics.space_sm)
            .py(theme.metrics.space_2xs)
            .border_1()
            .border_color(theme.colors.accent)
            .bg(theme.colors.popover_bg)
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_sm)
            .child(self.0.clone())
    }
}

#[derive(Clone, PartialEq)]
struct Indicator {
    kind: TreeKind,
    target: String,
    mode: DropMode,
}

struct Rename {
    key: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

pub struct TreesView {
    runtime: Runtime,
    rename: Option<Rename>,
    confirm_delete: Option<Vec<String>>,
    theme_picker: Option<Entity<ThemePicker>>,
    indicator: Option<Indicator>,
    _subscriptions: Vec<Subscription>,
}

/// The rows of the playlist tree in display order: each playlist with its index and depth, children after their
/// parent unless the parent is collapsed.
pub fn playlist_rows(playlists: &[Playlist]) -> Vec<(usize, usize)> {
    fn visit(
        playlists: &[Playlist],
        parent: &str,
        depth: usize,
        visited: &mut Vec<String>,
        rows: &mut Vec<(usize, usize)>,
    ) {
        for (index, playlist) in playlists.iter().enumerate() {
            if parent_id(playlists, playlist) != parent || visited.contains(&playlist.id) {
                continue;
            }
            rows.push((index, depth));
            if !playlist.collapsed {
                visited.push(playlist.id.clone());
                visit(playlists, &playlist.id, depth + 1, visited, rows);
                visited.pop();
            }
        }
    }
    let mut rows = Vec::new();
    visit(playlists, "", 0, &mut Vec::new(), &mut rows);
    rows
}

impl TreesView {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&runtime.content, |_, _, cx| cx.notify()),
            cx.observe(&runtime.ui, |_, _, cx| cx.notify()),
        ];
        Self {
            runtime,
            rename: None,
            confirm_delete: None,
            theme_picker: None,
            indicator: None,
            _subscriptions: subscriptions,
        }
    }

    /// Keeps the inline rename input in step with `rename_library_id` and `rename_playlist_id`.
    fn sync_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = {
            let (content, ui) = (self.runtime.content.read(cx), self.runtime.ui.read(cx));
            if !ui.rename_library_id.is_empty() {
                content
                    .library(&ui.rename_library_id)
                    .map(|library| (format!("library:{}", library.id), library.title.clone()))
            } else if !ui.rename_playlist_id.is_empty() {
                content
                    .playlist(&ui.rename_playlist_id)
                    .map(|playlist| (format!("playlist:{}", playlist.id), playlist.title.clone()))
            } else {
                None
            }
        };
        match wanted {
            None => self.rename = None,
            Some((key, _)) if self.rename.as_ref().is_some_and(|rename| rename.key == key) => {}
            Some((key, title)) => {
                let input = orbit_ui::controls::input_state(title, window, cx);
                let owner = key.clone();
                let subscription = cx.subscribe_in(
                    &input,
                    window,
                    move |this, input, event: &InputEvent, _, cx| {
                        let commit = match event {
                            InputEvent::PressEnter { .. } => true,
                            // Leaving the field commits too, unless Enter or Escape already ended the rename.
                            InputEvent::Blur => this
                                .rename
                                .as_ref()
                                .is_some_and(|rename| rename.key == owner),
                            _ => false,
                        };
                        if commit {
                            let value = input.read(cx).value().to_string();
                            this.commit_rename(&owner, &value, cx);
                        }
                    },
                );
                input.update(cx, |state, cx| {
                    state.focus(window, cx);
                    state.select_all(window, cx);
                });
                self.rename = Some(Rename {
                    key,
                    input,
                    _subscription: subscription,
                });
            }
        }
    }

    fn commit_rename(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        self.rename = None;
        let Some((kind, id)) = key.split_once(':') else {
            return;
        };
        let runtime = self.runtime.clone();
        if kind == "library" {
            tree_actions::commit_library_title(&runtime, cx, id, value).detach_task();
        } else {
            tree_actions::commit_playlist_title(&runtime, cx, id, value).detach_task();
        }
        cx.notify();
    }

    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.rename = None;
        self.runtime.ui.update(cx, |ui, cx| {
            ui.rename_library_id.clear();
            ui.rename_playlist_id.clear();
            cx.notify();
        });
    }

    fn rename_field(&self, key: &str, id: &'static str) -> Option<AnyElement> {
        let rename = self.rename.as_ref().filter(|rename| rename.key == key)?;
        Some(
            div()
                .flex_1()
                .min_w_0()
                .child(TextInput::new(id, &rename.input).compact())
                .into_any_element(),
        )
    }

    fn open_menu_at(&mut self, kind: TreeKind, id: &str, event: &MouseDownEvent, cx: &mut App) {
        let position = (f32::from(event.position.x), f32::from(event.position.y));
        let id = id.to_string();
        self.runtime.edit_ui(cx, |content, ui| match kind {
            TreeKind::Library => {
                let current = selected_library_ids(content, ui);
                let clicked_selected = current.contains(&id);
                let ids = if clicked_selected {
                    current.clone()
                } else {
                    vec![id.clone()]
                };
                ui.selected_library_ids = if clicked_selected {
                    current
                } else if ui.selected_playlist_id.is_empty() && !ui.selected_library_id.is_empty() {
                    vec![ui.selected_library_id.clone()]
                } else {
                    Vec::new()
                };
                ui.library_anchor = content
                    .libraries
                    .iter()
                    .position(|library| library.id == id);
                ui.close_menus();
                ui.library_menu = Some(TreeMenu { id, ids, position });
            }
            TreeKind::Playlist => {
                let current = selected_playlist_ids(content, ui);
                let clicked_selected = current.contains(&id);
                let ids = if clicked_selected {
                    current.clone()
                } else {
                    vec![id.clone()]
                };
                ui.selected_library_ids =
                    if ui.selected_playlist_id.is_empty() && !ui.selected_library_id.is_empty() {
                        vec![ui.selected_library_id.clone()]
                    } else {
                        Vec::new()
                    };
                ui.selected_playlist_ids = if clicked_selected {
                    current
                } else if !ui.selected_playlist_id.is_empty() {
                    vec![ui.selected_playlist_id.clone()]
                } else {
                    Vec::new()
                };
                ui.close_menus();
                ui.playlist_menu = Some(TreeMenu { id, ids, position });
            }
        });
    }

    fn on_drag_over(
        &mut self,
        kind: TreeKind,
        id: &str,
        event: &DragMoveEvent<TreeDrag>,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx).clone();
        if drag.kind != kind || drag.id == id || !event.bounds.contains(&event.event.position) {
            return;
        }
        let offset = f32::from(event.event.position.y - event.bounds.origin.y);
        let height = f32::from(event.bounds.size.height);
        let content = self.runtime.content.read(cx);
        let (is_folder, nested_last) = match kind {
            TreeKind::Library => (false, false),
            TreeKind::Playlist => (
                content
                    .playlist(id)
                    .is_some_and(|playlist| playlist.is_folder()),
                after_folder_as_root(&content.playlists, id),
            ),
        };
        let mode = tree_drop_mode(offset, height, kind, is_folder, nested_last);
        let noop = match kind {
            TreeKind::Library => is_noop_library_drop(&content.libraries, &drag.id, id, mode),
            TreeKind::Playlist => is_noop_playlist_drop(&content.playlists, &drag.id, id, mode),
        };
        self.indicator = (!noop).then(|| Indicator {
            kind,
            target: id.to_string(),
            mode,
        });
        cx.notify();
    }

    fn drop_on(&mut self, kind: TreeKind, id: &str, drag: &TreeDrag, cx: &mut Context<Self>) {
        let indicator = self.indicator.take();
        cx.notify();
        let Some(indicator) = indicator.filter(|i| i.kind == kind && i.target == id) else {
            return;
        };
        if drag.kind != kind {
            return;
        }
        let runtime = self.runtime.clone();
        match kind {
            TreeKind::Library => {
                tree_actions::reorder_libraries(
                    &runtime,
                    cx,
                    &drag.id,
                    id,
                    indicator.mode == DropMode::After,
                )
                .detach_task();
            }
            TreeKind::Playlist => {
                tree_actions::reorder_playlists(&runtime, cx, &drag.id, id, indicator.mode)
                    .detach_task();
            }
        }
    }

    /// A drop below the last playlist row moves the playlist to the root.
    fn drop_on_root(&mut self, drag: &TreeDrag, cx: &mut Context<Self>) {
        self.indicator = None;
        cx.notify();
        if drag.kind != TreeKind::Playlist {
            return;
        }
        let runtime = self.runtime.clone();
        tree_actions::move_playlist_to_parent(&runtime, cx, &drag.id, "").detach_task();
    }

    /// A row's insertion line, drawn over the row's edge.
    fn insertion_line(
        &self,
        kind: TreeKind,
        id: &str,
        indent: f32,
        cx: &App,
    ) -> Option<AnyElement> {
        let indicator = self
            .indicator
            .as_ref()
            .filter(|i| i.kind == kind && i.target == id)?;
        let theme = cx.theme();
        let line = div()
            .debug_selector(|| "collection-tree-insertion-line".into())
            .absolute()
            .right(px(4.))
            .left(px(indent))
            .h(px(2.))
            .rounded(theme.metrics.radius_xs)
            .bg(theme.colors.accent);
        Some(
            match indicator.mode {
                DropMode::Before => line.top(px(-1.)),
                DropMode::After => line.bottom(px(-1.)),
                DropMode::AfterRoot => line.left(px(18.)).bottom(px(-1.)),
                DropMode::Inside => line.left(px(indent + 18.)).h(px(4.)).bottom(px(-2.)),
            }
            .into_any_element(),
        )
    }

    fn library_row(&self, index: usize, library: &Library, cx: &mut Context<Self>) -> AnyElement {
        let (content, ui) = (self.runtime.content.read(cx), self.runtime.ui.read(cx));
        let selected =
            is_library_selected(content, ui, &library.id, ui.selected_playlist_id.is_empty());
        let context_target = ui
            .library_menu
            .as_ref()
            .is_some_and(|menu| menu.ids.contains(&library.id) || menu.id == library.id);
        let theme = *cx.theme();
        let id = library.id.clone();
        let title: SharedString = library.title.clone().into();
        let renaming = self.rename_field(&format!("library:{id}"), "library-rename");
        let drag = TreeDrag {
            kind: TreeKind::Library,
            id: id.clone(),
            title: title.clone(),
        };
        let line = self.insertion_line(TreeKind::Library, &id, 18., cx);
        let (selector_id, click_id, menu_id, hover_id, drop_id) =
            (id.clone(), id.clone(), id.clone(), id.clone(), id);
        list_row(("library", index), selected, cx)
            .relative()
            .debug_selector(move || format!("library-{selector_id}"))
            .when(context_target && !selected, |row| {
                row.border_1().border_color(theme.colors.accent)
            })
            .child(div().flex_none().w(theme.metrics.space_lg))
            .child(icon("library").text_color(theme.colors.text))
            .child(match renaming {
                Some(field) => field,
                None => div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(title)
                    .into_any_element(),
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if this.rename.is_some() {
                    return;
                }
                let modifiers: Modifiers = event.modifiers().into();
                this.runtime.edit_ui(cx, |content, ui| {
                    select_library(content, ui, &click_id, Some(index), modifiers)
                });
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.open_menu_at(TreeKind::Library, &menu_id, event, cx);
                }),
            )
            .on_drag(drag, |drag, _, _, cx| {
                cx.new(|_| DragPreview(drag.title.clone()))
            })
            .on_drag_move::<TreeDrag>(cx.listener(
                move |this, event: &DragMoveEvent<TreeDrag>, _, cx| {
                    this.on_drag_over(TreeKind::Library, &hover_id, event, cx)
                },
            ))
            .on_drop::<TreeDrag>(cx.listener(move |this, drag: &TreeDrag, _, cx| {
                this.drop_on(TreeKind::Library, &drop_id, drag, cx)
            }))
            .children(line)
            .into_any_element()
    }

    fn playlist_row(
        &self,
        index: usize,
        depth: usize,
        has_children: bool,
        playlist: &Playlist,
        t: Translator,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (content, ui) = (self.runtime.content.read(cx), self.runtime.ui.read(cx));
        let selected = is_playlist_selected(content, ui, &playlist.id);
        let active = ui.selected_playlist_id == playlist.id && !selected;
        let context_target = ui
            .playlist_menu
            .as_ref()
            .is_some_and(|menu| menu.ids.contains(&playlist.id) || menu.id == playlist.id);
        let theme = *cx.theme();
        let id = playlist.id.clone();
        let title: SharedString = playlist.title.clone().into();
        let renaming = self.rename_field(&format!("playlist:{id}"), "playlist-rename");
        let indent = 18. + depth as f32 * 18.;
        let drag = TreeDrag {
            kind: TreeKind::Playlist,
            id: id.clone(),
            title: title.clone(),
        };
        let line = self.insertion_line(TreeKind::Playlist, &id, indent + 14., cx);
        let collapse = (playlist.is_folder() && has_children).then(|| {
            let toggle_id = id.clone();
            div()
                .id(("playlist-collapse", index))
                .debug_selector({
                    let id = id.clone();
                    move || format!("playlist-collapse-{id}")
                })
                .flex_none()
                .w(theme.metrics.space_lg)
                .cursor_pointer()
                .tooltip(orbit_ui::controls::tooltip_text(if playlist.collapsed {
                    t.t("media.folder.open")
                } else {
                    t.t("media.folder.close")
                }))
                .child(
                    icon(if playlist.collapsed {
                        "chevron-right"
                    } else {
                        "chevron-down"
                    })
                    .text_color(theme.colors.text),
                )
                // The chevron toggles; it must not also select the row.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let runtime = this.runtime.clone();
                    tree_actions::toggle_playlist_collapsed(&runtime, cx, &toggle_id).detach_task();
                    cx.stop_propagation();
                }))
        });
        let (selector_id, click_id, rename_id, menu_id, hover_id, drop_id) = (
            id.clone(),
            id.clone(),
            id.clone(),
            id.clone(),
            id.clone(),
            id,
        );
        list_row(("playlist", index), selected, cx)
            .relative()
            .pl(px(8.) + px(depth as f32 * 18.))
            .debug_selector(move || format!("playlist-{selector_id}"))
            .when(active, |row| {
                row.border_2().border_color(theme.colors.accent)
            })
            .when(context_target && !selected && !active, |row| {
                row.border_1().border_color(theme.colors.accent)
            })
            .child(match collapse {
                Some(collapse) => collapse.into_any_element(),
                None => div()
                    .flex_none()
                    .w(theme.metrics.space_lg)
                    .into_any_element(),
            })
            .child(
                icon(if playlist.is_folder() {
                    "folder"
                } else {
                    "list"
                })
                .text_color(theme.colors.text),
            )
            .child(match renaming {
                Some(field) => field,
                None => div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(title)
                    .into_any_element(),
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if this.rename.is_some() {
                    return;
                }
                if event.click_count() >= 2 {
                    this.runtime.ui.update(cx, |ui, cx| {
                        ui.rename_playlist_id = rename_id.clone();
                        cx.notify();
                    });
                    return;
                }
                let modifiers: Modifiers = event.modifiers().into();
                this.runtime.edit_ui(cx, |content, ui| {
                    select_playlist(content, ui, &click_id, Some(index), modifiers)
                });
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.open_menu_at(TreeKind::Playlist, &menu_id, event, cx);
                }),
            )
            .on_drag(drag, |drag, _, _, cx| {
                cx.new(|_| DragPreview(drag.title.clone()))
            })
            .on_drag_move::<TreeDrag>(cx.listener(
                move |this, event: &DragMoveEvent<TreeDrag>, _, cx| {
                    this.on_drag_over(TreeKind::Playlist, &hover_id, event, cx)
                },
            ))
            .on_drop::<TreeDrag>(cx.listener(move |this, drag: &TreeDrag, _, cx| {
                this.drop_on(TreeKind::Playlist, &drop_id, drag, cx)
            }))
            .children(line)
            .into_any_element()
    }

    fn theme_action(
        &self,
        menu: &TreeMenu,
        kind: TreeKind,
        t: Translator,
        cx: &mut Context<Self>,
    ) -> MenuEntry {
        let content = self.runtime.content.read(cx);
        let assignments = menu
            .ids
            .iter()
            .filter_map(|id| match kind {
                TreeKind::Library => content.library(id).map(|library| &library.theme),
                TreeKind::Playlist => content.playlist(id).map(|playlist| &playlist.theme),
            })
            .collect::<Vec<_>>();
        let value = match assignments
            .first()
            .filter(|first| assignments.iter().all(|a| a == *first))
        {
            Some(ThemeAssignment::Theme(id)) => id.clone(),
            Some(ThemeAssignment::NoTheme) => "none".into(),
            Some(ThemeAssignment::Inherit) => "inherit".into(),
            None => String::new(),
        };
        let options = if kind == TreeKind::Playlist {
            vec![
                (t.t("theme.assignment.inherit"), "inherit".into()),
                (t.t("theme.assignment.none"), "none".into()),
            ]
        } else {
            Vec::new()
        };
        let ids = menu.ids.clone();
        let workspace = content.workspace_path.clone();
        let runtime = self.runtime.clone();
        // The selection is captured before the context menu closes.
        let view = cx.weak_entity();
        MenuItem::new(t.t("cue.menu.themes"))
            .on_click(move |_, _, cx| {
                let close = view.clone();
                let (runtime, ids, workspace) = (runtime.clone(), ids.clone(), workspace.clone());
                let picker = ThemePicker::dialog(
                    runtime.clone(),
                    &value,
                    options.clone(),
                    move |_, cx| {
                        let _ = close.update(cx, |view, cx| {
                            view.theme_picker = None;
                            cx.notify();
                        });
                    },
                    move |value, _, cx| {
                        if runtime.content.read(cx).workspace_path != workspace {
                            return;
                        }
                        let assignment = match value.as_str() {
                            "inherit" => ThemeAssignment::Inherit,
                            "none" => ThemeAssignment::NoTheme,
                            _ => ThemeAssignment::Theme(value),
                        };
                        match kind {
                            TreeKind::Library => tree_actions::assign_theme_to_libraries(
                                &runtime,
                                cx,
                                ids.clone(),
                                assignment,
                            ),
                            TreeKind::Playlist => tree_actions::assign_theme_to_playlists(
                                &runtime,
                                cx,
                                ids.clone(),
                                assignment,
                            ),
                        }
                        .detach_task();
                    },
                    cx,
                );
                let _ = view.update(cx, |view, cx| {
                    view.theme_picker = Some(picker);
                    cx.notify();
                });
            })
            .into()
    }

    fn library_menu(&self, menu: &TreeMenu, t: Translator, cx: &mut Context<Self>) -> AnyElement {
        let multi = menu.ids.len() > 1;
        let (runtime, id) = (self.runtime.clone(), menu.id.clone());
        let mut entries = vec![
            self.theme_action(menu, TreeKind::Library, t, cx),
            MenuEntry::Separator,
        ];
        let view = cx.weak_entity();
        let action = |label: String, action: &'static str| -> MenuEntry {
            let (runtime, id, view) = (runtime.clone(), id.clone(), view.clone());
            MenuItem::new(label)
                .on_click(move |_, _, cx| {
                    if let LibraryMenuOutcome::ConfirmDelete(ids) =
                        handle_library_menu_action(&runtime, cx, action, &id)
                    {
                        view.update(cx, |this, cx| {
                            this.confirm_delete = Some(ids);
                            cx.notify();
                        })
                        .ok();
                    }
                })
                .into()
        };
        if !multi {
            entries.push(action(t.t("common.rename"), "rename"));
        }
        entries.push(action(t.t("common.delete"), "delete"));
        let position = gpui::point(px(menu.position.0), px(menu.position.1));
        let close = self.runtime.clone();
        ContextMenu::new("library-menu", position, entries, move |_, _, cx| {
            // The menu item's own action runs after this and still reads the menu, so it closes afterwards.
            let close = close.clone();
            cx.defer(move |cx| {
                close.ui.update(cx, |ui, cx| {
                    ui.library_menu = None;
                    cx.notify();
                });
            });
        })
        .width(px(180.))
        .into_any_element()
    }

    fn playlist_menu(&self, menu: &TreeMenu, t: Translator, cx: &mut Context<Self>) -> AnyElement {
        let multi = menu.ids.len() > 1;
        let (runtime, id) = (self.runtime.clone(), menu.id.clone());
        let mut entries = vec![
            self.theme_action(menu, TreeKind::Playlist, t, cx),
            MenuEntry::Separator,
        ];
        let action = |label: String, action: &'static str| -> MenuEntry {
            let (runtime, id) = (runtime.clone(), id.clone());
            MenuItem::new(label)
                .on_click(move |_, _, cx| handle_playlist_menu_action(&runtime, cx, action, &id))
                .into()
        };
        if !multi {
            entries.push(action(t.t("common.rename"), "rename"));
            entries.push(MenuEntry::Separator);
        }
        entries.push(action(t.t("common.duplicate"), "duplicate"));
        entries.push(action(t.t("common.delete"), "delete"));
        let position = gpui::point(px(menu.position.0), px(menu.position.1));
        let close = self.runtime.clone();
        ContextMenu::new("playlist-menu", position, entries, move |_, _, cx| {
            // The menu item's own action runs after this and still reads the menu, so it closes afterwards.
            let close = close.clone();
            cx.defer(move |cx| {
                close.ui.update(cx, |ui, cx| {
                    ui.playlist_menu = None;
                    cx.notify();
                });
            });
        })
        .width(px(236.))
        .into_any_element()
    }

    fn create_menu(&self, menu: &CreateMenu, t: Translator) -> AnyElement {
        let runtime = self.runtime.clone();
        let item = |label: String, run: fn(&Runtime, &mut App)| -> MenuEntry {
            let runtime = runtime.clone();
            MenuItem::new(label)
                .on_click(move |_, _, cx| run(&runtime, cx))
                .into()
        };
        let entries = match menu.kind {
            CreateMenuKind::LibraryTree => vec![
                item(t.t("appMenu.newLibrary"), |runtime, cx| {
                    tree_actions::create_library(runtime, cx).detach()
                }),
                item(
                    t.t("collections.menu.newPresentation"),
                    super::documents::open_new_presentation,
                ),
            ],
            _ => vec![
                item(t.t("collections.menu.newPlaylist"), |runtime, cx| {
                    let title = tree_actions::translator(cx).t("collections.default.newPlaylist");
                    tree_actions::create_playlist(runtime, cx, title, None).detach()
                }),
                item(t.t("collections.menu.newPlaylistFolder"), |runtime, cx| {
                    let title = tree_actions::translator(cx).t("collections.default.newFolder");
                    tree_actions::create_playlist(runtime, cx, title, Some("folder")).detach()
                }),
            ],
        };
        let position = gpui::point(px(menu.position.0), px(menu.position.1));
        let close = self.runtime.clone();
        ContextMenu::new("tree-create-menu", position, entries, move |_, _, cx| {
            // The menu item's own action runs after this and still reads the menu, so it closes afterwards.
            let close = close.clone();
            cx.defer(move |cx| {
                close.ui.update(cx, |ui, cx| {
                    ui.create_menu = None;
                    cx.notify();
                });
            });
        })
        .width(px(224.))
        .into_any_element()
    }

    fn confirm_dialog(&self, ids: &[String], t: Translator, cx: &mut Context<Self>) -> AnyElement {
        let content = self.runtime.content.read(cx);
        let items = deleted_item_ids(content, ids).len();
        let message = delete_libraries_message(t, ids.len(), items);
        let (view, runtime, deleted) = (cx.weak_entity(), self.runtime.clone(), ids.to_vec());
        let close = {
            let view = view.clone();
            move |_: (), _: &mut Window, cx: &mut App| {
                view.update(cx, |this, cx| {
                    this.confirm_delete = None;
                    cx.notify();
                })
                .ok();
            }
        };
        Dialog::new(
            "confirm-delete-libraries",
            t.t("app.name"),
            t.t("common.cancel"),
            close.clone(),
        )
        .width(px(420.))
        .children(
            message
                .split('\n')
                .filter(|line| !line.trim().is_empty())
                .map(|line| div().child(line.trim().to_string()).into_any_element()),
        )
        .footer(
            ActionButton::new("confirm-delete-cancel", t.t("common.cancel"))
                .variant(ButtonVariant::Secondary)
                .on_click({
                    let close = close.clone();
                    move |_, window, cx| close((), window, cx)
                }),
        )
        .footer(
            ActionButton::new("confirm-delete-ok", t.t("common.delete"))
                .variant(ButtonVariant::Danger)
                .on_click(move |_, window, cx| {
                    close((), window, cx);
                    tree_actions::delete_libraries(&runtime, cx, deleted.clone()).detach();
                }),
        )
        .into_any_element()
    }
}

impl Render for TreesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_rename(window, cx);
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let (libraries, playlists) = {
            let content = self.runtime.content.read(cx);
            (content.libraries.clone(), content.playlists.clone())
        };
        let (library_menu, playlist_menu, create_menu) = {
            let ui = self.runtime.ui.read(cx);
            (
                ui.library_menu.clone(),
                ui.playlist_menu.clone(),
                ui.create_menu.clone().filter(|menu| {
                    matches!(
                        menu.kind,
                        CreateMenuKind::LibraryTree | CreateMenuKind::PlaylistTree
                    )
                }),
            )
        };
        let library_rows: Vec<AnyElement> = libraries
            .iter()
            .enumerate()
            .map(|(index, library)| self.library_row(index, library, cx))
            .collect();
        let rows = playlist_rows(&playlists);
        let playlist_elements: Vec<AnyElement> = rows
            .iter()
            .map(|(index, depth)| {
                let playlist = &playlists[*index];
                let has_children = playlists
                    .iter()
                    .any(|child| parent_id(&playlists, child) == playlist.id);
                self.playlist_row(*index, *depth, has_children, playlist, t, cx)
            })
            .collect();
        let section = |header: PanelHeader, body: AnyElement| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(header)
                .child(body)
        };
        let add_button =
            |id: &'static str, label: String, kind: CreateMenuKind, cx: &mut Context<Self>| {
                IconButton::new(id, "plus", label).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        let position = window.mouse_position();
                        this.runtime.ui.update(cx, |ui, cx| {
                            ui.close_menus();
                            ui.create_menu = Some(CreateMenu {
                                kind,
                                position: (f32::from(position.x), f32::from(position.y)),
                            });
                            cx.notify();
                        });
                    },
                ))
            };
        let library_header = PanelHeader::new(t.t("collections.menu.library")).action(add_button(
            "library-add",
            t.t("newPresentation.addMenu"),
            CreateMenuKind::LibraryTree,
            cx,
        ));
        let playlist_header =
            PanelHeader::new(t.t("collections.menu.playlist")).action(add_button(
                "playlist-add",
                t.t("collections.action.createPlaylist"),
                CreateMenuKind::PlaylistTree,
                cx,
            ));
        let library_menu = library_menu.map(|menu| self.library_menu(&menu, t, cx));
        let playlist_menu = playlist_menu.map(|menu| self.playlist_menu(&menu, t, cx));
        let create_menu = create_menu.map(|menu| self.create_menu(&menu, t));
        let confirm = self
            .confirm_delete
            .clone()
            .map(|ids| self.confirm_dialog(&ids, t, cx));
        let root_drop = cx.listener(|this, drag: &TreeDrag, _, cx| this.drop_on_root(drag, cx));
        div()
            .id("collection-trees")
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.rename.is_some() {
                    this.cancel_rename(cx);
                    cx.stop_propagation();
                }
            }))
            // The container's drag listener runs before the rows': it clears the line, a row under the pointer
            // draws it again.
            .on_drag_move::<TreeDrag>(cx.listener(|this, _, _, cx| {
                if this.indicator.take().is_some() {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.indicator.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.indicator.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(section(
                library_header,
                div()
                    .id("library-tree")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(library_rows)
                    .into_any_element(),
            ))
            .child(section(
                playlist_header,
                div()
                    .id("playlist-tree")
                    .debug_selector(|| "playlist-tree".to_string())
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .on_drop::<TreeDrag>(root_drop)
                    .children(playlist_elements)
                    .into_any_element(),
            ))
            .children(library_menu)
            .children(playlist_menu)
            .children(create_menu)
            .children(confirm)
            .children(self.theme_picker.clone())
            .text_color(theme.colors.text)
    }
}
