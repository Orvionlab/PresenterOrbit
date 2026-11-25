//! The document list: the documents of the selected library, or the entries of the selected playlist. Virtualized,
//! so 10,000 documents cost what the visible rows cost. Ports of `CollectionDocumentList.tsx`,
//! `PlaylistHeaderRow.tsx`, the document parts of `CollectionMenuLayer.tsx`, and `NewPresentationDialog.tsx`.

use gpui::{
    div, prelude::*, px, uniform_list, AnyElement, App, ClickEvent, Context, DragMoveEvent, Entity,
    FocusHandle, Focusable, KeyDownEvent, MouseButton, MouseDownEvent, Render, ScrollStrategy,
    SharedString, Subscription, UniformListScrollHandle, Window,
};
use orbit_ui::controls::{
    data_color, hex_alpha, list_row, list_row_without_hover, ActionButton, ButtonVariant,
    ColorPicker, ContextMenu, Dialog, EmptyState, Field, IconButton, InputEvent, InputState,
    MenuEntry, MenuItem, PanelHeader, Select, SelectOption, TextInput,
};
use orbit_ui::{icon, ActiveTheme, Translator};
use serde_json::Value;

use super::document_actions as actions;
use super::drag::is_entry_noop_drop;
use super::model::EntryKind;
use super::selection::{
    click_document_row, click_entry_row, is_document_row_selected, navigate_document_item,
    Modifiers,
};
use super::tree_actions::{translator, DetachExt};
use crate::state::runtime::Runtime;
use crate::state::slices::{
    Content, CreateMenu, CreateMenuKind, DocumentMenu, NewPresentation, NewPresentationState,
    Selected, Source, Ui,
};
use crate::theme_picker::ThemePicker;

pub(crate) const PRESENTATION_SIZES: [(u32, u32); 4] =
    [(1920, 1080), (1280, 720), (3840, 2160), (1024, 768)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    Document,
    Header,
    Placeholder,
}

fn visible_rows(rows: &[Row], query: &str) -> Vec<usize> {
    let words: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            (!row.hidden
                && (words.is_empty() || {
                    let title = row.title.to_lowercase();
                    words.iter().all(|word| title.contains(word))
                }))
            .then_some(index)
        })
        .collect()
}

/// One row of the list, resolved from the content so rendering does not search it.
#[derive(Clone, Debug)]
pub struct Row {
    kind: RowKind,
    /// The presentation id for a library row, the entry id for a playlist row.
    id: String,
    /// The document a row selects: the presentation, or the placeholder's linked document.
    document: String,
    title: SharedString,
    color: String,
    hidden: bool,
}

/// The rows of the selected library or playlist, in display order.
pub fn build_rows(content: &Content, ui: &Ui, t: Translator) -> Vec<Row> {
    if let Some(playlist) = content.playlist(&ui.selected_playlist_id) {
        return playlist
            .entries
            .iter()
            .filter_map(|entry| match entry.kind {
                EntryKind::Presentation => {
                    let item = content.items.get(&entry.item_id)?;
                    Some(Row {
                        hidden: entry.raw["hidden"] == true,
                        kind: RowKind::Document,
                        id: entry.id.clone(),
                        document: item.id.clone(),
                        title: item.display_title(&entry.arrangement_id).into(),
                        color: String::new(),
                    })
                }
                EntryKind::Header => Some(Row {
                    hidden: entry.raw["hidden"] == true,
                    kind: RowKind::Header,
                    id: entry.id.clone(),
                    document: String::new(),
                    title: if entry.title.is_empty() {
                        t.t("cue.playlist.header")
                    } else {
                        entry.title.clone()
                    }
                    .into(),
                    color: entry.color.clone(),
                }),
                EntryKind::Placeholder => {
                    let linked = content.items.get(&entry.linked_item_id);
                    Some(Row {
                        hidden: entry.raw["hidden"] == true,
                        kind: RowKind::Placeholder,
                        id: entry.id.clone(),
                        document: linked.map(|item| item.id.clone()).unwrap_or_default(),
                        title: linked
                            .map_or_else(
                                || {
                                    if entry.title.is_empty() {
                                        t.t("cue.playlist.placeholder")
                                    } else {
                                        entry.title.clone()
                                    }
                                },
                                |item| item.display_title(&entry.arrangement_id),
                            )
                            .into(),
                        color: String::new(),
                    })
                }
            })
            .collect();
    }
    content
        .library(&ui.selected_library_id)
        .map(|library| {
            content
                .sorted_document_ids(library)
                .iter()
                .filter_map(|id| content.items.get(id))
                .map(|item| Row {
                    hidden: false,
                    kind: RowKind::Document,
                    id: item.id.clone(),
                    document: item.id.clone(),
                    title: item.title.clone().into(),
                    color: String::new(),
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Row {}

/// The payload of dragging playlist rows.
#[derive(Clone)]
pub struct EntryDrag {
    sources: Vec<usize>,
    label: SharedString,
}

struct EntryDragPreview(SharedString);

impl Render for EntryDragPreview {
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

struct Rename {
    key: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

type MenuRun = Box<dyn Fn(&Runtime, &mut App)>;

struct NewPresentationForm {
    title: Entity<InputState>,
    _subscription: Subscription,
}

struct HeaderColor {
    playlist: String,
    entry: String,
    color: String,
}

struct ThemeUndo {
    workspace: String,
    item: String,
    before: Value,
    after: Value,
}

pub struct DocumentsView {
    runtime: Runtime,
    rows: Vec<Row>,
    rows_key: (String, String),
    rows_dirty: bool,
    visible: Vec<usize>,
    filter: Option<Entity<InputState>>,
    query: String,
    rename: Option<Rename>,
    form: Option<NewPresentationForm>,
    theme_picker: Option<Entity<ThemePicker>>,
    assign_theme_picker: Option<Entity<ThemePicker>>,
    theme_undo: Vec<ThemeUndo>,
    theme_redo: Vec<ThemeUndo>,
    saving_theme: bool,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    /// The gap before this row index shows the insertion line while playlist rows are dragged.
    drop_gap: Option<usize>,
    hidden_menu: Option<gpui::Point<gpui::Pixels>>,
    header_color: Option<HeaderColor>,
    _subscriptions: Vec<Subscription>,
}

/// `openNewPresentation`: the dialog opens with the selected library and the last used theme layout.
pub fn open_new_presentation(runtime: &Runtime, cx: &mut App) {
    open_presentation_form(runtime, cx, String::new(), None);
}

pub fn open_presentation_for_placeholder(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: String,
    entry_id: String,
) {
    open_presentation_form(runtime, cx, playlist_id, Some(entry_id));
}

fn open_presentation_form(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: String,
    placeholder_id: Option<String>,
) {
    let title = translator(cx).t("collections.default.newPresentation");
    runtime.edit_ui(cx, |content, ui| {
        if ui.new_presentation.is_some() {
            return;
        }
        let library = content
            .library(&ui.selected_library_id)
            .or_else(|| content.libraries.first())
            .map(|library| library.id.clone())
            .unwrap_or_default();
        ui.new_presentation = Some(NewPresentationState {
            draft: NewPresentation {
                title,
                library_id: library,
                playlist_id,
                placeholder_id,
                theme_id: content.last_used_theme.clone().unwrap_or_default(),
                size: PRESENTATION_SIZES[0],
            },
            saving: false,
            error: String::new(),
        });
        ui.create_menu = None;
    });
}

impl DocumentsView {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&runtime.content, |this, _, cx| {
                this.rows_dirty = true;
                cx.notify();
            }),
            cx.observe(&runtime.ui, |_, _, cx| cx.notify()),
        ];
        Self {
            runtime,
            rows: Vec::new(),
            rows_key: (String::new(), String::new()),
            rows_dirty: true,
            visible: Vec::new(),
            filter: None,
            query: String::new(),
            rename: None,
            form: None,
            theme_picker: None,
            assign_theme_picker: None,
            theme_undo: Vec::new(),
            theme_redo: Vec::new(),
            saving_theme: false,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            drop_gap: None,
            hidden_menu: None,
            header_color: None,
            _subscriptions: subscriptions,
        }
    }

    /// The rows only change when the content or the selected library or playlist does.
    fn ensure_rows(&mut self, t: Translator, cx: &App) {
        let key = {
            let ui = self.runtime.ui.read(cx);
            (
                ui.selected_library_id.clone(),
                ui.selected_playlist_id.clone(),
            )
        };
        let changed_scope = key != self.rows_key;
        if self.rows_dirty || changed_scope {
            self.rows = build_rows(self.runtime.content.read(cx), self.runtime.ui.read(cx), t);
            self.rows_key = key;
            self.rows_dirty = false;
            self.refilter();
            if changed_scope {
                self.hidden_menu = None;
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
        }
    }

    fn refilter(&mut self) {
        self.visible = visible_rows(&self.rows, &self.query);
        self.drop_gap = None;
    }

    fn filter_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let input = self.filter.get_or_insert_with(|| {
            let input = orbit_ui::controls::filter_state("", window, cx);
            self._subscriptions.push(cx.subscribe(
                &input,
                |view, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        view.query = input.read(cx).value().to_string();
                        view.refilter();
                        view.scroll.scroll_to_item(0, ScrollStrategy::Top);
                        cx.notify();
                    }
                },
            ));
            input
        });
        div()
            .debug_selector(|| "documents-filter-bar".into())
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.metrics.space_xs)
            .p(theme.metrics.space_xs)
            .border_t_1()
            .border_color(theme.colors.border_soft)
            .bg(theme.colors.panel_bg)
            .child(
                orbit_ui::controls::FilterInput::new("documents-filter", input)
                    .label(t.t("collections.filter")),
            )
            .into_any_element()
    }

    fn sync_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = {
            let (content, ui) = (self.runtime.content.read(cx), self.runtime.ui.read(cx));
            if !ui.rename_item_id.is_empty() {
                content
                    .items
                    .get(&ui.rename_item_id)
                    .map(|item| (format!("item:{}", item.id), item.title.clone()))
            } else if !ui.rename_entry_id.is_empty() {
                content
                    .playlist(&ui.selected_playlist_id)
                    .and_then(|playlist| {
                        playlist.entries.iter().find(|e| e.id == ui.rename_entry_id)
                    })
                    .map(|entry| (format!("entry:{}", entry.id), entry.title.clone()))
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
        if kind == "item" {
            actions::commit_item_title(&runtime, cx, id, value).detach_task();
        } else {
            actions::commit_entry_title(&runtime, cx, id, value).detach_task();
        }
        cx.notify();
    }

    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.rename = None;
        self.runtime.ui.update(cx, |ui, cx| {
            ui.rename_item_id.clear();
            ui.rename_entry_id.clear();
            cx.notify();
        });
    }

    /// Opens the new presentation form when the state asks for it and drops it when the dialog closes.
    fn sync_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self
            .runtime
            .ui
            .read(cx)
            .new_presentation
            .as_ref()
            .map(|state| state.draft.title.clone());
        match (title, self.form.is_some()) {
            (None, _) => {
                self.form = None;
                self.theme_picker = None;
            }
            (Some(_), true) => {}
            (Some(title), false) => {
                let input = orbit_ui::controls::input_state(title, window, cx);
                let subscription =
                    cx.subscribe_in(&input, window, |this, input, event, _, cx| match event {
                        InputEvent::Change => {
                            let value = input.read(cx).value().to_string();
                            this.runtime.ui.update(cx, |ui, _| {
                                if let Some(state) = ui.new_presentation.as_mut() {
                                    state.draft.title = value;
                                }
                            });
                        }
                        InputEvent::PressEnter { .. } => this.submit_form(cx),
                        _ => {}
                    });
                input.update(cx, |state, cx| {
                    state.set_placeholder(translator(cx).t("newPresentation.name"), window, cx);
                    state.focus(window, cx);
                    state.select_all(window, cx);
                });
                self.theme_picker = Some(cx.new(|cx| {
                    ThemePicker::new(self.runtime.clone(), "new-presentation-theme", cx)
                }));
                self.form = Some(NewPresentationForm {
                    title: input,
                    _subscription: subscription,
                });
            }
        }
    }

    /// `createNewPresentation`.
    fn submit_form(&mut self, cx: &mut Context<Self>) {
        let t = translator(cx);
        let (draft, saving) = match self.runtime.ui.read(cx).new_presentation.as_ref() {
            Some(state) => (state.draft.clone(), state.saving),
            None => return,
        };
        if saving {
            return;
        }
        if let Some(key) = actions::new_presentation_error(self.runtime.content.read(cx), &draft) {
            let message = t.t(key);
            self.runtime.ui.update(cx, |ui, cx| {
                if let Some(state) = ui.new_presentation.as_mut() {
                    state.error = message;
                }
                cx.notify();
            });
            return;
        }
        self.runtime.ui.update(cx, |ui, cx| {
            if let Some(state) = ui.new_presentation.as_mut() {
                state.saving = true;
                state.error.clear();
            }
            cx.notify();
        });
        let runtime = self.runtime.clone();
        let ui = runtime.ui.clone();
        actions::create_presentation(&runtime, cx, draft, move |created, cx| {
            let failed = t.t("newPresentation.saveFailed");
            ui.update(cx, |ui, cx| {
                if created {
                    ui.new_presentation = None;
                } else if let Some(state) = ui.new_presentation.as_mut() {
                    state.saving = false;
                    state.error = failed;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) {
        self.runtime.ui.update(cx, |ui, cx| {
            if ui
                .new_presentation
                .as_ref()
                .is_some_and(|state| !state.saving)
            {
                ui.new_presentation = None;
            }
            cx.notify();
        });
    }

    fn row_selected(&self, index: usize, row: &Row, ui: &Ui, content: &Content) -> bool {
        let keep_playlist = content.playlist(&ui.selected_playlist_id).is_some();
        let id = if keep_playlist && row.kind != RowKind::Document {
            &row.id
        } else {
            &row.document
        };
        let id = if row.kind == RowKind::Placeholder && !row.document.is_empty() {
            &row.id
        } else {
            id
        };
        is_document_row_selected(ui, id, keep_playlist, index)
    }

    fn on_row_click(
        &mut self,
        index: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        if self.rename.is_some() {
            return;
        }
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        let modifiers: Modifiers = event.modifiers().into();
        if event.click_count() >= 2 {
            self.runtime.ui.update(cx, |ui, cx| {
                match row.kind {
                    RowKind::Document => ui.rename_item_id = row.document.clone(),
                    _ => ui.rename_entry_id = row.id.clone(),
                }
                cx.notify();
            });
            return;
        }
        self.select_row(index, modifiers, cx);
    }

    fn select_row(&self, index: usize, modifiers: Modifiers, cx: &mut App) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        self.runtime.edit_ui(cx, |content, ui| {
            let keep_playlist = content.playlist(&ui.selected_playlist_id).is_some();
            let range = modifiers
                .range
                .then(|| {
                    let anchor = ui
                        .document_anchor
                        .as_ref()
                        .filter(|anchor| {
                            anchor.source
                                == if keep_playlist {
                                    Source::Playlist
                                } else {
                                    Source::Library
                                }
                                && (!keep_playlist || anchor.playlist_id == ui.selected_playlist_id)
                        })
                        .cloned();
                    let start = anchor
                        .as_ref()
                        .and_then(|anchor| self.visible.iter().position(|i| *i == anchor.index));
                    let end = self.visible.iter().position(|i| *i == index)?;
                    let start = start.unwrap_or(end);
                    let mut selected = if modifiers.toggle {
                        ui.selected_documents.clone()
                    } else {
                        Vec::new()
                    };
                    let mut seen: std::collections::HashSet<_> = selected
                        .iter()
                        .map(|entry| (entry.item_id.clone(), entry.index))
                        .collect();
                    for visible in &self.visible[start.min(end)..=start.max(end)] {
                        let row = &self.rows[*visible];
                        let entry = Selected {
                            item_id: if keep_playlist && row.kind != RowKind::Document {
                                row.id.clone()
                            } else {
                                row.document.clone()
                            },
                            index: keep_playlist.then_some(*visible),
                        };
                        if seen.insert((entry.item_id.clone(), entry.index)) {
                            selected.push(entry);
                        }
                    }
                    Some((anchor, selected))
                })
                .flatten();
            let modifiers = Modifiers {
                range: false,
                ..modifiers
            };
            if keep_playlist {
                if click_entry_row(content, ui, &row.id, index, modifiers) {
                    ui.playlist_reveal = Some((ui.selected_playlist_id.clone(), row.id.clone()));
                }
            } else {
                click_document_row(content, ui, &row.document, false, index, modifiers);
            }
            if let Some((anchor, selected)) = range {
                ui.selected_documents = selected;
                if anchor.is_some() {
                    ui.document_anchor = anchor;
                }
            }
        });
    }

    fn on_row_menu(
        &mut self,
        index: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        let position = (f32::from(event.position.x), f32::from(event.position.y));
        self.runtime.edit_ui(cx, |content, ui| {
            let source = if content.playlist(&ui.selected_playlist_id).is_some() {
                Source::Playlist
            } else {
                Source::Library
            };
            actions::open_document_menu(content, ui, &row.id, source, index, position);
        });
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        if self.rename.is_none() && !self.focus.is_focused(window) {
            return;
        }
        let direction = match event.keystroke.key.as_str() {
            "up" => -1,
            "down" => 1,
            "escape" if self.rename.is_some() => {
                self.cancel_rename(cx);
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        if self.rename.is_some() {
            return;
        }
        if !self.runtime.ui.read(cx).selected_playlist_id.is_empty()
            || !self.query.trim().is_empty()
        {
            let rows = &self.visible;
            if rows.is_empty() {
                return;
            }
            let ui = self.runtime.ui.read(cx);
            let current = rows
                .iter()
                .position(|index| {
                    self.row_selected(
                        *index,
                        &self.rows[*index],
                        ui,
                        self.runtime.content.read(cx),
                    ) && ui
                        .document_anchor
                        .as_ref()
                        .is_none_or(|anchor| anchor.index == *index)
                })
                .or_else(|| {
                    rows.iter()
                        .position(|index| self.rows[*index].document == ui.selected_library_item_id)
                });
            let next = match current {
                Some(index) => {
                    (index as i64 + i64::from(direction)).clamp(0, rows.len() as i64 - 1) as usize
                }
                None if direction > 0 => 0,
                None => rows.len() - 1,
            };
            let index = rows[next];
            self.select_row(index, Modifiers::default(), cx);
            if let Some(display) = self.visible.iter().position(|visible| *visible == index) {
                self.scroll.scroll_to_item(display, ScrollStrategy::Nearest);
            }
            cx.stop_propagation();
            return;
        }
        let mut moved = None;
        self.runtime.edit_ui(cx, |content, ui| {
            moved = navigate_document_item(content, ui, direction);
        });
        if let Some(index) = moved {
            if let Some(row) = self.visible.iter().position(|visible| *visible == index) {
                self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
            }
            cx.stop_propagation();
        }
    }

    fn drag_over(
        &mut self,
        index: usize,
        event: &DragMoveEvent<EntryDrag>,
        cx: &mut Context<Self>,
    ) {
        if !event.bounds.contains(&event.event.position) {
            return;
        }
        let drag = event.drag(cx).clone();
        let height = f32::from(event.bounds.size.height);
        let offset = f32::from(event.event.position.y - event.bounds.origin.y);
        let gap = if offset < height / 2. {
            index
        } else {
            index + 1
        };
        let remaining = gap - drag.sources.iter().filter(|source| **source < gap).count();
        self.drop_gap = (!is_entry_noop_drop(&drag.sources, remaining)).then_some(gap);
        cx.notify();
    }

    fn drop_rows(&mut self, drag: &EntryDrag, cx: &mut Context<Self>) {
        let gap = self.drop_gap.take();
        cx.notify();
        let Some(gap) = gap else {
            return;
        };
        let remaining = gap - drag.sources.iter().filter(|source| **source < gap).count();
        let runtime = self.runtime.clone();
        let playlist = runtime.ui.read(cx).selected_playlist_id.clone();
        actions::reorder_entries(&runtime, cx, &playlist, &drag.sources, remaining).detach_task();
    }

    fn row_element(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.rows.get(index).cloned() else {
            return div().into_any_element();
        };
        let (content, ui) = (self.runtime.content.read(cx), self.runtime.ui.read(cx));
        let selected = self.row_selected(index, &row, ui, content);
        let keep_playlist = content.playlist(&ui.selected_playlist_id).is_some();
        let theme = *cx.theme();
        let renaming = self.rename.as_ref().filter(|rename| {
            rename.key
                == match row.kind {
                    RowKind::Document => format!("item:{}", row.document),
                    _ => format!("entry:{}", row.id),
                }
        });
        let sources = {
            let selection: Vec<usize> = ui
                .selected_documents
                .iter()
                .filter(|selected| {
                    ui.selected_document_source == Some(Source::Playlist)
                        && ui.selected_document_playlist_id == ui.selected_playlist_id
                        && selected.index.is_some()
                })
                .filter_map(|selected| selected.index)
                .collect();
            if selection.contains(&index) {
                selection
            } else {
                vec![index]
            }
        };
        let line_before = self.drop_gap == Some(index);
        let line_after =
            self.drop_gap == Some(index + 1) && self.visible.binary_search(&(index + 1)).is_err();
        let line = |top: bool| {
            div()
                .absolute()
                .left(px(4.))
                .right(px(4.))
                .h(px(2.))
                .rounded(theme.metrics.radius_xs)
                .bg(theme.colors.accent)
                .when(top, |line| line.top(px(-1.)))
                .when(!top, |line| line.bottom(px(-1.)))
        };
        let color = self
            .header_color
            .as_ref()
            .filter(|edit| edit.playlist == self.rows_key.1 && edit.entry == row.id)
            .map_or(row.color.as_str(), |edit| edit.color.as_str());
        let background = (row.kind == RowKind::Header
            && !color.is_empty()
            && color != "transparent")
            .then(|| {
                let base = if selected {
                    theme.colors.selected
                } else {
                    theme.colors.panel_bg
                };
                base.blend(data_color(color).opacity(0.4 * f32::from(hex_alpha(color)) / 255.))
            });
        let label = row.title.clone();
        let drag = EntryDrag {
            sources,
            label: label.clone(),
        };
        let strong = matches!(row.kind, RowKind::Header);
        let row_icon = if row.kind == RowKind::Placeholder && row.document.is_empty() {
            "square-dashed"
        } else {
            "library"
        };
        let id = ("document", index);
        let element = match background {
            Some(color) => list_row_without_hover(id, selected, cx).bg(color),
            None => list_row(id, selected, cx),
        };
        let mut element = element
            .relative()
            .debug_selector(move || format!("document-{index}"))
            .child(
                div()
                    .flex_none()
                    .w(theme.metrics.space_lg)
                    .when(!strong, |slot| {
                        slot.debug_selector(move || format!("document-icon-{index}-{row_icon}"))
                            .child(
                                icon(row_icon)
                                    .size(theme.metrics.space_lg)
                                    .text_color(theme.colors.text),
                            )
                    }),
            )
            .child(match renaming {
                Some(rename) => div()
                    .flex_1()
                    .min_w_0()
                    .child(TextInput::new("document-rename", &rename.input).compact())
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(strong, |title| title.font_weight(gpui::FontWeight::BOLD))
                    .child(label)
                    .into_any_element(),
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.on_row_click(index, event, window, cx)
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.on_row_menu(index, event, window, cx)
                }),
            )
            .children(line_before.then(|| line(true)))
            .children(line_after.then(|| line(false)));
        if keep_playlist {
            element = element
                .on_drag(drag, |drag, _, _, cx| {
                    cx.new(|_| EntryDragPreview(drag.label.clone()))
                })
                .on_drag_move::<EntryDrag>(cx.listener(
                    move |this, event: &DragMoveEvent<EntryDrag>, _, cx| {
                        this.drag_over(index, event, cx)
                    },
                ))
                .on_drop::<EntryDrag>(
                    cx.listener(move |this, drag: &EntryDrag, _, cx| this.drop_rows(drag, cx)),
                );
        }
        element.into_any_element()
    }

    fn open_theme_picker(&mut self, item: String, cx: &mut Context<Self>) {
        let content = self.runtime.content.read(cx);
        let Some(slides) = content
            .items
            .get(&item)
            .and_then(|item| item.value["slides"].as_array())
        else {
            return;
        };
        let current = slides
            .first()
            .and_then(|slide| slide["presentationThemeId"].as_str())
            .filter(|id| {
                slides
                    .iter()
                    .all(|slide| slide["presentationThemeId"] == *id)
            })
            .unwrap_or_default()
            .to_owned();
        let workspace = content.workspace_path.clone();
        let close = cx.weak_entity();
        let view = close.clone();
        self.assign_theme_picker = Some(ThemePicker::dialog(
            self.runtime.clone(),
            &current,
            Vec::new(),
            move |window, cx| {
                let _ = close.update(cx, |view, cx| {
                    view.assign_theme_picker = None;
                    view.focus.focus(window, cx);
                    cx.notify();
                });
            },
            move |assignment, _, cx| {
                let _ = view.update(cx, |view, cx| {
                    if view.runtime.content.read(cx).workspace_path == workspace {
                        view.apply_theme(&item, &assignment, cx);
                    }
                });
            },
            cx,
        ));
        cx.notify();
    }

    fn apply_theme(&mut self, item: &str, assignment: &str, cx: &mut Context<Self>) {
        let content = self.runtime.content.read(cx);
        let Some(slides) = content
            .items
            .get(item)
            .and_then(|item| item.value["slides"].as_array())
        else {
            return;
        };
        if self.saving_theme || slides.is_empty() {
            return;
        }
        let workspace = content.workspace_path.clone();
        let item = item.to_owned();
        let before = Value::Array(slides.clone());
        let ids = slides.iter().map(|slide| &slide["id"]).collect::<Vec<_>>();
        let task = crate::cues::actions::mutate_checked(
            &self.runtime,
            &item,
            serde_json::json!({"type":"setSlidesTheme", "slideIds":ids, "expectedSlides":before, "assignment":assignment}),
            cx,
        );
        self.saving_theme = true;
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |view, cx| {
                view.saving_theme = false;
                let content = view.runtime.content.read(cx);
                if content.workspace_path == workspace {
                    if let Some(after) = result
                        .filter(|result| result["changed"] == true)
                        .and_then(|mut result| result.get_mut("themeSlides").map(Value::take))
                        .filter(|after| *after != before)
                    {
                        view.theme_undo.retain(|entry| entry.workspace == workspace);
                        view.theme_undo.push(ThemeUndo {
                            workspace,
                            item,
                            before,
                            after,
                        });
                        view.theme_redo.clear();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn can_edit_theme_history(&self, undo: bool, cx: &App) -> bool {
        let history = if undo {
            &self.theme_undo
        } else {
            &self.theme_redo
        };
        let content = self.runtime.content.read(cx);
        !self.saving_theme
            && history.last().is_some_and(|entry| {
                entry.workspace == content.workspace_path
                    && content.items.get(&entry.item).is_some_and(|item| {
                        let expected = if undo { &entry.after } else { &entry.before };
                        &item.value["slides"] == expected
                    })
            })
    }

    pub fn edit_theme_history(&mut self, undo: bool, cx: &mut Context<Self>) {
        if !self.can_edit_theme_history(undo, cx) {
            return;
        }
        let stack = if undo {
            &mut self.theme_undo
        } else {
            &mut self.theme_redo
        };
        let Some(entry) = stack.pop() else {
            return;
        };
        let (expected, slides) = if undo {
            (&entry.after, &entry.before)
        } else {
            (&entry.before, &entry.after)
        };
        let task = crate::cues::actions::mutate_checked(
            &self.runtime,
            &entry.item,
            serde_json::json!({"type":"setSlideDocument","expectedSlides":expected,"slides":slides}),
            cx,
        );
        self.saving_theme = true;
        cx.spawn(async move |view, cx| {
            let saved = task.await.is_some();
            let _ = view.update(cx, |view, cx| {
                view.saving_theme = false;
                if entry.workspace == view.runtime.content.read(cx).workspace_path {
                    if saved == undo {
                        view.theme_redo.push(entry);
                    } else {
                        view.theme_undo.push(entry);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn document_menu(
        &self,
        menu: &DocumentMenu,
        t: Translator,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (content, ui) = (self.runtime.content.read(cx), self.runtime.ui.read(cx));
        let runtime = self.runtime.clone();
        let item = |label: String, shortcut: Option<&'static str>, disabled: bool, run: MenuRun| {
            let runtime = runtime.clone();
            let entry = MenuItem::new(label)
                .disabled(disabled)
                .on_click(move |_, _, cx| run(&runtime, cx));
            MenuEntry::from(match shortcut {
                Some(shortcut) => entry.meta(shortcut),
                None => entry,
            })
        };
        let header_or_placeholder = actions::menu_entry(content, ui)
            .filter(|(_, entry)| entry.kind != EntryKind::Presentation);
        let mut entries: Vec<MenuEntry> = Vec::new();
        let others: Vec<(String, String)> = content
            .playlists
            .iter()
            .filter(|playlist| !playlist.is_folder())
            .map(|playlist| (playlist.id.clone(), playlist.title.clone()))
            .collect();
        if let Some((_, entry)) = header_or_placeholder {
            let paste_disabled = ui.clipboard.entry.is_none();
            let kind = entry.kind;
            let entry_action = |label: String, action: &'static str, disabled: bool| {
                item(
                    label,
                    match action {
                        "copy" => Some("Ctrl+C"),
                        "paste" => Some("Ctrl+V"),
                        "duplicate" => Some("Ctrl+D"),
                        "delete" => Some("Delete"),
                        _ => None,
                    },
                    disabled,
                    Box::new(move |runtime, cx| {
                        actions::handle_entry_menu_action(runtime, cx, action)
                    }),
                )
            };
            if kind == EntryKind::Header {
                entries.push(entry_action(
                    t.t("collections.menu.hideItem"),
                    "hide",
                    false,
                ));
                entries.push(MenuEntry::Separator);
                entries.push(self.add_to_playlist_submenu(others, true, t));
                let id = entry.id.clone();
                let playlist = ui.selected_playlist_id.clone();
                let color = self
                    .header_color
                    .as_ref()
                    .filter(|edit| edit.playlist == playlist && edit.entry == id)
                    .map_or(entry.color.as_str(), |edit| edit.color.as_str());
                let label = t.t("playlistHeader.changeColor");
                let control = ColorPicker::new(
                    "playlist-menu-header-color",
                    label.clone(),
                    if color.is_empty() {
                        "transparent"
                    } else {
                        color
                    },
                )
                .on_change({
                    let view = cx.weak_entity();
                    move |color, _, cx| {
                        view.update(cx, |view, cx| {
                            view.header_color = Some(HeaderColor {
                                playlist: playlist.clone(),
                                entry: id.clone(),
                                color,
                            });
                            cx.notify();
                        })
                        .ok();
                    }
                });
                entries.push(MenuEntry::Control {
                    label: label.into(),
                    control: div()
                        .debug_selector(|| "playlist-menu-header-color".into())
                        .w(cx.theme().metrics.control_height * 2.)
                        .child(control)
                        .into_any_element(),
                });
                entries.push(MenuEntry::Separator);
            }
            if kind == EntryKind::Placeholder {
                entries.push(entry_action(t.t("common.rename"), "rename", false));
                entries.push(MenuEntry::Separator);
            }
            entries.push(entry_action(t.t("common.copy"), "copy", false));
            entries.push(entry_action(t.t("common.paste"), "paste", paste_disabled));
            entries.push(entry_action(t.t("common.duplicate"), "duplicate", false));
            if kind == EntryKind::Header {
                entries.push(entry_action(t.t("common.rename"), "rename", false));
            }
            entries.push(entry_action(t.t("common.delete"), "delete", false));
        } else {
            let documents = actions::menu_documents(content, ui);
            let multi = documents.len() > 1;
            let item_id = actions::menu_entry(content, ui)
                .map(|(_, entry)| entry.item_id)
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| menu.id.clone());
            let current_library = content.library_id_for_item(&item_id).to_string();
            let libraries: Vec<(String, String)> = content
                .libraries
                .iter()
                .filter(|library| library.id != current_library)
                .map(|library| (library.id.clone(), library.title.clone()))
                .collect();
            let all_libraries: Vec<(String, String)> = content
                .libraries
                .iter()
                .map(|library| (library.id.clone(), library.title.clone()))
                .collect();
            let doc_action = |label: String,
                              shortcut: Option<&'static str>,
                              action: &'static str,
                              disabled: bool| {
                let item_id = item_id.clone();
                item(
                    label,
                    shortcut,
                    disabled,
                    Box::new(move |runtime, cx| {
                        actions::handle_document_menu_action(runtime, cx, action, &item_id)
                    }),
                )
            };
            if menu.source == Source::Playlist {
                entries.push(doc_action(
                    t.t("collections.menu.hideItem"),
                    None,
                    "hide",
                    false,
                ));
                entries.push(MenuEntry::Separator);
            }
            // Add to
            let mut add_entries: Vec<MenuEntry> = all_libraries
                .iter()
                .map(|(id, title)| {
                    let id = id.clone();
                    item(
                        title.clone(),
                        None,
                        false,
                        Box::new(move |runtime, cx| {
                            actions::add_documents_to_library(runtime, cx, &id).detach_task()
                        }),
                    )
                })
                .collect();
            if !all_libraries.is_empty() && !others.is_empty() {
                add_entries.push(MenuEntry::Separator);
            }
            add_entries.extend(others.iter().map(|(id, title)| {
                let id = id.clone();
                item(
                    title.clone(),
                    None,
                    false,
                    Box::new(move |runtime, cx| {
                        actions::add_documents_to_playlist(runtime, cx, &id).detach_task()
                    }),
                )
            }));
            entries.push(MenuEntry::Submenu {
                label: t.t("collections.menu.addTo").into(),
                disabled: add_entries.is_empty(),
                entries: add_entries,
            });
            if menu.source == Source::Library {
                // Move to
                entries.push(MenuEntry::Submenu {
                    label: t.t("collections.menu.moveTo").into(),
                    disabled: libraries.is_empty(),
                    entries: libraries
                        .iter()
                        .map(|(id, title)| {
                            let id = id.clone();
                            item(
                                title.clone(),
                                None,
                                false,
                                Box::new(move |runtime, cx| {
                                    actions::move_documents_to_library(runtime, cx, &id)
                                        .detach_task()
                                }),
                            )
                        })
                        .collect(),
                });
            }
            // Arrangements of a playlist presentation row.
            if menu.source == Source::Playlist {
                if let (Some((_, entry)), Some(doc)) = (
                    actions::menu_entry(content, ui),
                    content.items.get(&item_id),
                ) {
                    let master = MenuItem::new(t.t("arrangements.master"))
                        .checked(entry.arrangement_id.is_empty());
                    let master = {
                        let item_id = item_id.clone();
                        let runtime = runtime.clone();
                        master.on_click(move |_, _, cx| {
                            actions::handle_document_menu_action(
                                &runtime,
                                cx,
                                "arrangement:",
                                &item_id,
                            )
                        })
                    };
                    let mut arrangements = vec![MenuEntry::from(master)];
                    for arrangement in &doc.arrangements {
                        let (item_id, runtime, id) =
                            (item_id.clone(), runtime.clone(), arrangement.id.clone());
                        arrangements.push(
                            MenuItem::new(arrangement.name.clone())
                                .checked(entry.arrangement_id == arrangement.id)
                                .on_click(move |_, _, cx| {
                                    actions::handle_document_menu_action(
                                        &runtime,
                                        cx,
                                        &format!("arrangement:{id}"),
                                        &item_id,
                                    )
                                })
                                .into(),
                        );
                    }
                    entries.push(MenuEntry::Submenu {
                        label: t.t("arrangements.menu").into(),
                        disabled: multi,
                        entries: arrangements,
                    });
                }
            }
            let document = content.items.get(&item_id);
            entries.push(MenuEntry::Submenu {
                label: t.t("cue.destination").into(),
                disabled: false,
                entries: [
                    ("cue.destination.presentation", "presentation"),
                    ("cue.destination.announcements", "announcements"),
                ]
                .into_iter()
                .map(|(label, destination)| {
                    let runtime = runtime.clone();
                    let item_id = item_id.clone();
                    MenuItem::new(t.t(label))
                        .checked(
                            !multi
                                && document.is_some_and(|item| {
                                    crate::cues::flow::announces(&item.value)
                                        == (destination == "announcements")
                                }),
                        )
                        .on_click(move |_, _, cx| {
                            actions::handle_document_menu_action(
                                &runtime,
                                cx,
                                &format!("destination:{destination}"),
                                &item_id,
                            )
                        })
                        .into()
                })
                .collect(),
            });
            entries.push(MenuEntry::Separator);
            entries.push(doc_action(t.t("common.edit"), None, "edit", multi));
            entries.push(MenuEntry::Submenu {
                label: t.t("collections.menu.resize").into(),
                disabled: false,
                entries: PRESENTATION_SIZES
                    .into_iter()
                    .map(|(width, height)| {
                        let runtime = runtime.clone();
                        let item_id = item_id.clone();
                        MenuItem::new(format!("{width} × {height}"))
                            .checked(
                                !multi
                                    && document.is_some_and(|item| {
                                        item.value["size"]["width"] == width
                                            && item.value["size"]["height"] == height
                                    }),
                            )
                            .on_click(move |_, _, cx| {
                                actions::handle_document_menu_action(
                                    &runtime,
                                    cx,
                                    &format!("size:{width}x{height}"),
                                    &item_id,
                                )
                            })
                            .into()
                    })
                    .collect(),
            });
            let theme_view = cx.weak_entity();
            entries.push(
                MenuItem::new(t.t("cue.menu.themes"))
                    .disabled(
                        multi
                            || self.saving_theme
                            || !document.is_some_and(|item| {
                                item.value["slides"]
                                    .as_array()
                                    .is_some_and(|slides| !slides.is_empty())
                            }),
                    )
                    .on_click({
                        let item_id = item_id.clone();
                        move |_, _, cx| {
                            let _ = theme_view
                                .update(cx, |view, cx| view.open_theme_picker(item_id.clone(), cx));
                        }
                    })
                    .into(),
            );
            entries.push(doc_action(t.t("toolbar.reflow"), None, "reflow", multi));
            entries.push(MenuEntry::Separator);
            entries.push(doc_action(
                t.t("common.copy"),
                Some("Ctrl+C"),
                "copy",
                false,
            ));
            entries.push(doc_action(
                t.t("common.paste"),
                Some("Ctrl+V"),
                "paste",
                ui.clipboard.items.is_empty(),
            ));
            entries.push(doc_action(
                t.t("common.delete"),
                Some("Delete"),
                "delete",
                false,
            ));
            entries.push(doc_action(
                t.t("common.duplicate"),
                Some("Ctrl+D"),
                "duplicate",
                false,
            ));
            if menu.source == Source::Playlist {
                entries.push(MenuEntry::Separator);
                entries.push(doc_action(
                    t.t("collections.menu.showInLibrary"),
                    None,
                    "show-library",
                    multi || current_library.is_empty(),
                ));
            } else {
                entries.push(doc_action(t.t("common.rename"), None, "rename", multi));
            }
        }
        let position = gpui::point(px(menu.position.0), px(menu.position.1));
        let close = cx.weak_entity();
        ContextMenu::new("document-menu", position, entries, move |_, _, cx| {
            // The menu item's own action runs after this and still reads the menu, so it closes afterwards.
            let close = close.clone();
            cx.defer(move |cx| {
                close
                    .update(cx, |view, cx| {
                        view.runtime.ui.update(cx, |ui, cx| {
                            ui.document_menu = None;
                            cx.notify();
                        });
                        if let Some(edit) = view.header_color.take() {
                            super::headers::save_color(
                                &view.runtime,
                                cx,
                                &edit.playlist,
                                &edit.entry,
                                edit.color,
                            );
                        }
                        cx.notify();
                    })
                    .ok();
            });
        })
        .width(px(236.))
        .into_any_element()
    }

    fn add_to_playlist_submenu(
        &self,
        playlists: Vec<(String, String)>,
        header_entry: bool,
        t: Translator,
    ) -> MenuEntry {
        let _ = header_entry;
        let runtime = self.runtime.clone();
        MenuEntry::Submenu {
            label: t.t("collections.menu.addTo").into(),
            disabled: playlists.is_empty(),
            entries: playlists
                .into_iter()
                .map(|(id, title)| {
                    let runtime = runtime.clone();
                    MenuItem::new(title)
                        .on_click(move |_, _, cx| {
                            actions::add_entry_to_playlist(&runtime, cx, &id).detach_task()
                        })
                        .into()
                })
                .collect(),
        }
    }

    fn create_menu(&self, menu: &CreateMenu, t: Translator, cx: &App) -> AnyElement {
        let runtime = self.runtime.clone();
        let folder = self
            .runtime
            .content
            .read(cx)
            .playlist(&self.runtime.ui.read(cx).selected_playlist_id)
            .is_some_and(|playlist| playlist.is_folder());
        let entry = |label: String, disabled: bool, run: fn(&Runtime, &mut App)| -> MenuEntry {
            let runtime = runtime.clone();
            MenuItem::new(label)
                .disabled(disabled)
                .on_click(move |_, _, cx| run(&runtime, cx))
                .into()
        };
        let mut entries = vec![entry(
            t.t("collections.menu.newPresentation"),
            false,
            open_new_presentation,
        )];
        if menu.kind == CreateMenuKind::Playlist {
            entries.push(entry(
                t.t("collections.menu.newHeader"),
                folder,
                |runtime, cx| actions::create_entry(runtime, cx, EntryKind::Header).detach_task(),
            ));
            entries.push(entry(
                t.t("collections.menu.newPlaceholder"),
                folder,
                |runtime, cx| {
                    actions::create_entry(runtime, cx, EntryKind::Placeholder).detach_task()
                },
            ));
        }
        let position = gpui::point(px(menu.position.0), px(menu.position.1));
        let close = self.runtime.clone();
        ContextMenu::new(
            "document-create-menu",
            position,
            entries,
            move |_, _, cx| {
                // The menu item's own action runs after this and still reads the menu, so it closes afterwards.
                let close = close.clone();
                cx.defer(move |cx| {
                    close.ui.update(cx, |ui, cx| {
                        ui.create_menu = None;
                        cx.notify();
                    });
                });
            },
        )
        .width(px(224.))
        .into_any_element()
    }

    fn new_presentation_dialog(&self, t: Translator, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.form.as_ref()?;
        let content = self.runtime.content.read(cx);
        let state = self.runtime.ui.read(cx).new_presentation.clone()?;
        let draft = state.draft.clone();
        let view = cx.weak_entity();
        let libraries: Vec<SelectOption<String>> = content
            .libraries
            .iter()
            .map(|library| SelectOption::new(library.id.clone(), library.title.clone()))
            .collect();
        let mut playlists = vec![SelectOption::new(
            String::new(),
            t.t("newPresentation.noPlaylist"),
        )];
        playlists.extend(
            content
                .playlists
                .iter()
                .filter(|playlist| !playlist.is_folder())
                .map(|playlist| SelectOption::new(playlist.id.clone(), playlist.title.clone())),
        );
        let change = |apply: fn(&mut NewPresentation, String)| {
            let runtime = self.runtime.clone();
            move |value: String, _: &mut Window, cx: &mut App| {
                runtime.ui.update(cx, |ui, cx| {
                    if let Some(state) = ui.new_presentation.as_mut() {
                        apply(&mut state.draft, value);
                    }
                    cx.notify();
                });
            }
        };
        let theme_picker = self.theme_picker.clone()?;
        let pick_runtime = self.runtime.clone();
        theme_picker.update(cx, |picker, _| {
            picker.sync(&draft.theme_id, state.saving, move |value, _, cx| {
                pick_runtime.ui.update(cx, |ui, cx| {
                    if let Some(state) = ui.new_presentation.as_mut() {
                        state.draft.theme_id = value;
                    }
                    cx.notify();
                });
            })
        });
        let sizes = PRESENTATION_SIZES
            .into_iter()
            .map(|(width, height)| {
                SelectOption::new((width, height), format!("{width} × {height}"))
            })
            .collect::<Vec<_>>();
        let label_width = cx.theme().metrics.control_height * 3.;
        let runtime = self.runtime.clone();
        let size_picker = Select::new("new-presentation-size", sizes, draft.size)
            .label(t.t("theme.size"))
            .compact()
            .disabled(state.saving)
            .on_change(move |size, _, cx| {
                runtime.ui.update(cx, |ui, cx| {
                    if let Some(state) = ui.new_presentation.as_mut() {
                        state.draft.size = size;
                    }
                    cx.notify();
                });
            });
        let body = div()
            .flex()
            .flex_col()
            .gap(cx.theme().metrics.space_xs)
            .child(
                TextInput::new("new-presentation-title", &form.title)
                    .label(t.t("newPresentation.name"))
                    .compact()
                    .disabled(state.saving),
            )
            .child(
                Field::new(t.t("theme.theme"), theme_picker)
                    .inline()
                    .label_width(label_width),
            )
            .child(
                Field::new(
                    t.t("theme.size"),
                    div()
                        .debug_selector(|| "new-presentation-size".into())
                        .child(size_picker),
                )
                .inline()
                .label_width(label_width),
            )
            .child(
                Field::new(
                    t.t("collections.menu.library"),
                    Select::new(
                        "new-presentation-library",
                        libraries,
                        draft.library_id.clone(),
                    )
                    .label(t.t("collections.menu.library"))
                    .compact()
                    .disabled(state.saving)
                    .on_change(change(|draft, value| draft.library_id = value)),
                )
                .inline()
                .label_width(label_width),
            )
            .child(
                Field::new(
                    t.t("collections.menu.playlist"),
                    Select::new(
                        "new-presentation-playlist",
                        playlists,
                        draft.playlist_id.clone(),
                    )
                    .label(t.t("collections.menu.playlist"))
                    .compact()
                    .disabled(state.saving || draft.placeholder_id.is_some())
                    .on_change(change(|draft, value| draft.playlist_id = value)),
                )
                .inline()
                .label_width(label_width),
            );
        let (close_view, cancel_view, submit_view) = (view.clone(), view.clone(), view);
        Some(
            Dialog::new(
                "new-presentation",
                t.t("collections.menu.newPresentation"),
                t.t("common.close"),
                move |_, _, cx| {
                    close_view.update(cx, |this, cx| this.close_form(cx)).ok();
                },
            )
            .width(px(420.))
            .initial_focus(form.title.read(cx).focus_handle(cx))
            .child(body)
            .children((!state.error.is_empty()).then(|| {
                div()
                    .id("new-presentation-error")
                    .text_color(cx.theme().colors.danger)
                    .child(state.error.clone())
            }))
            .footer(
                ActionButton::new("new-presentation-cancel", t.t("common.cancel"))
                    .variant(ButtonVariant::Secondary)
                    .compact()
                    .disabled(state.saving)
                    .on_click(move |_, _, cx| {
                        cancel_view.update(cx, |this, cx| this.close_form(cx)).ok();
                    }),
            )
            .footer(
                ActionButton::new(
                    "new-presentation-create",
                    if state.saving {
                        t.t("newPresentation.saving")
                    } else {
                        t.t("newPresentation.create")
                    },
                )
                .variant(ButtonVariant::Primary)
                .compact()
                .disabled(state.saving)
                .on_click(move |_, _, cx| {
                    submit_view.update(cx, |this, cx| this.submit_form(cx)).ok();
                }),
            )
            .into_any_element(),
        )
    }
}

impl Render for DocumentsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_rename(window, cx);
        self.sync_form(window, cx);
        let t = *cx.global::<Translator>();
        self.ensure_rows(t, cx);
        let theme = *cx.theme();
        let (title, document_menu, create_menu) = {
            let (content, ui) = (self.runtime.content.read(cx), self.runtime.ui.read(cx));
            (
                match content.playlist(&ui.selected_playlist_id) {
                    Some(playlist) => playlist.title.clone(),
                    None => t.format(
                        "collections.documentCount",
                        &[("count", &self.visible.len().to_string())],
                    ),
                },
                ui.document_menu.clone(),
                ui.create_menu.clone().filter(|menu| {
                    matches!(
                        menu.kind,
                        CreateMenuKind::Library | CreateMenuKind::Playlist
                    )
                }),
            )
        };
        let in_playlist = self
            .runtime
            .content
            .read(cx)
            .playlist(&self.runtime.ui.read(cx).selected_playlist_id)
            .is_some();
        let add = IconButton::new(
            "document-add",
            "plus",
            t.t("collections.action.createDocument"),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            let position = window.mouse_position();
            let kind = if in_playlist {
                CreateMenuKind::Playlist
            } else {
                CreateMenuKind::Library
            };
            this.runtime.ui.update(cx, |ui, cx| {
                ui.close_menus();
                ui.create_menu = Some(CreateMenu {
                    kind,
                    position: (f32::from(position.x), f32::from(position.y)),
                });
                cx.notify();
            });
        }));
        let list = if self.visible.is_empty() {
            div()
                .flex_1()
                .min_h_0()
                .when(
                    !self.query.trim().is_empty() && !self.rows.is_empty(),
                    |list| list.child(EmptyState::new(t.t("collections.filter.empty"))),
                )
                .into_any_element()
        } else {
            uniform_list(
                "documents",
                self.visible.len(),
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|index| this.row_element(this.visible[index], cx))
                        .collect()
                }),
            )
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .into_any_element()
        };
        let document_menu = document_menu.map(|menu| self.document_menu(&menu, t, cx));
        let create_menu = create_menu.map(|menu| self.create_menu(&menu, t, cx));
        let dialog = self.new_presentation_dialog(t, cx);
        let hidden: Vec<_> = self.rows.iter().filter(|row| row.hidden).cloned().collect();
        let hidden_button = (!hidden.is_empty()).then(|| {
            ActionButton::new(
                "playlist-hidden-items",
                t.format(
                    "collections.hiddenItems",
                    &[("count", &hidden.len().to_string())],
                ),
            )
            .variant(ButtonVariant::Ghost)
            .compact()
            .on_click(cx.listener(|this, _, window, cx| {
                this.hidden_menu = Some(window.mouse_position());
                cx.notify();
            }))
        });
        let hidden_menu = self.hidden_menu.map(|position| {
            let owner = cx.weak_entity();
            let playlist = self.runtime.ui.read(cx).selected_playlist_id.clone();
            let hidden_ids: Vec<_> = hidden.iter().map(|row| row.id.clone()).collect();
            let entries = hidden
                .into_iter()
                .map(|row| {
                    let runtime = self.runtime.clone();
                    let playlist = playlist.clone();
                    MenuItem::new(row.title)
                        .on_click(move |_, _, cx| {
                            actions::set_entries_hidden(
                                &runtime,
                                cx,
                                &playlist,
                                vec![row.id.clone()],
                                false,
                            )
                        })
                        .into()
                })
                .collect::<Vec<_>>();
            let runtime = self.runtime.clone();
            let entries = vec![
                MenuEntry::Submenu {
                    label: t.t("collections.menu.unhideItem").into(),
                    disabled: entries.is_empty(),
                    entries,
                },
                MenuItem::new(t.t("collections.menu.unhideAll"))
                    .on_click(move |_, _, cx| {
                        actions::set_entries_hidden(
                            &runtime,
                            cx,
                            &playlist,
                            hidden_ids.clone(),
                            false,
                        );
                    })
                    .into(),
            ];
            ContextMenu::new(
                "playlist-hidden-menu",
                position,
                entries,
                move |_, _, cx| {
                    owner
                        .update(cx, |this, cx| {
                            this.hidden_menu = None;
                            cx.notify();
                        })
                        .ok();
                },
            )
        });
        let filter_bar = self.filter_bar(window, cx);
        div()
            .id("collection-documents")
            .key_context("DocumentList")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .text_color(theme.colors.text)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key_down(event, window, cx)
            }))
            .on_drag_move::<EntryDrag>(cx.listener(|this, _, _, cx| {
                if this.drop_gap.take().is_some() {
                    cx.notify();
                }
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.drop_gap.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(PanelHeader::new(title).action(add))
            .child(list)
            .children(hidden_button)
            .child(filter_bar)
            .children(hidden_menu)
            .children(document_menu)
            .children(create_menu)
            .children(dialog)
            .children(self.assign_theme_picker.clone())
    }
}
