mod arrangement_manager;
mod preview_text;
mod slide_menu;
mod view_options;
pub use slide_menu::SlideMenuRequest;

use super::{
    actions,
    model::{self, Document},
};
use crate::{
    collections::{
        arrangements::{self, SectionEdit},
        headers,
        model::{Entry, EntryKind, Item},
        song_metadata,
    },
    media::AssetDrag,
    output::{
        preview::{Preview, Source},
        view::screen_size,
    },
    state::{
        runtime::{Kind, Persist, Runtime},
        slices::Selected,
    },
};
use gpui::{
    div, prelude::*, px, AnyElement, App, Context, Entity, FocusHandle, Focusable, KeyDownEvent,
    ListAlignment, ListOffset, ListState, MouseButton, Pixels, Point, Render, SharedString,
    Subscription, Window,
};
use orbit_ui::controls::{
    growing_textarea_state, input_state, textarea_state, ActionButton, ButtonVariant, ContextMenu,
    Dialog, EmptyState, Field, IconButton, InputEvent, InputState, MenuEntry, MenuItem,
    PanelHeader, Popover, SegmentedControl, SegmentedOption, Select, SelectOption, TextInput,
    Textarea, TextareaState,
};
use orbit_ui::controls::{Backspace, Delete};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc, time::Duration};

const REFLOW_AUTOSAVE: Duration = Duration::from_millis(700);
const REFLOW_HISTORY: usize = 100;
const REFLOW_MAX_ROWS: usize = 12;
type ChipBounds = Rc<RefCell<Vec<(usize, gpui::Bounds<Pixels>)>>>;
/// Painted tile bounds by (document, cue index, master slide index).
type TileBounds = Rc<RefCell<HashMap<(usize, usize, usize), gpui::Bounds<Pixels>>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum MediaPlacement {
    Before,
    Background,
    After,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct MediaDrop {
    document: usize,
    cue: usize,
    placement: MediaPlacement,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Easy,
    Grid,
    Table,
}
impl Mode {
    fn key(self) -> &'static str {
        match self {
            Self::Easy => "easy",
            Self::Grid => "grid",
            Self::Table => "table",
        }
    }
}
#[derive(Clone)]
enum FormKind {
    Text(String),
    Shortcut(String),
    NewArrangement,
    RenameArrangement(String),
    Song,
}
/// The Reflow editor's Edit button: open this slide in the slide editor.
pub struct EditSlide {
    pub item: String,
    pub slide: String,
}
impl gpui::EventEmitter<EditSlide> for CueView {}
/// The operator notes bar under the header: edited in place, saved when it loses focus.
struct Notes {
    item: String,
    entry: String,
    state: Entity<TextareaState>,
    _events: Subscription,
}
struct PresentationTools {
    destination: Entity<super::flow::Destination>,
    advance: Entity<super::flow::AutoAdvance>,
    transition: Entity<crate::transitions::TransitionDefaults>,
}

struct Form {
    kind: FormKind,
    item: String,
    fields: Vec<(&'static str, Entity<InputState>)>,
    /// One editable text per slide of the presentation, by slide id; only the Reflow form has them.
    rows: Vec<(String, Entity<TextareaState>)>,
    autosave: Option<gpui::Task<()>>,
    saving: bool,
    error: String,
    nonce: u64,
    _observers: Vec<Subscription>,
}

/// Keep editor dialogs visible even when the cue grid is hidden by a canvas.
pub struct Forms {
    owner: Entity<CueView>,
    _subscription: Subscription,
}

impl Forms {
    pub fn new(owner: Entity<CueView>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&owner, |_, _, cx| cx.notify());
        Self {
            owner,
            _subscription: subscription,
        }
    }
}

impl Render for Forms {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner
            .update(cx, |view, cx| view.form_element(cx))
            .unwrap_or_else(|| div().into_any_element())
    }
}
/// A group chip being dragged: from the Groups row (`from` is `None`) or out of the arrangement bar.
#[derive(Clone)]
struct SectionDrag {
    id: String,
    name: SharedString,
    color: gpui::Rgba,
    from: Option<usize>,
}
struct SectionDragPreview(SharedString, gpui::Rgba);
impl Render for SectionDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px(theme.metrics.space_sm)
            .py(theme.metrics.space_2xs)
            .rounded(theme.metrics.radius_sm)
            .bg(self.1)
            .text_color(theme.colors.accent_text)
            .text_size(theme.metrics.font_sm)
            .child(self.0.clone())
    }
}
enum ArrangementMenu {
    Select(Point<Pixels>),
    Chip(Point<Pixels>, usize),
}
struct Menu {
    position: Point<Pixels>,
    document: usize,
    index: usize,
}
struct Press {
    start: Point<Pixels>,
    document: usize,
    sources: Vec<usize>,
    dragging: bool,
    gap: Option<usize>,
    generation: u64,
}
enum PlaylistRow {
    Marker(Box<Entry>, usize),
    Cues(usize),
    EndSpace,
}

pub struct CueView {
    runtime: Runtime,
    visible: HashMap<(usize, usize), Entity<Preview>>,
    documents: Vec<Arc<Document>>,
    rows: Vec<(usize, usize, usize)>,
    key: (u64, String, String, Option<usize>, bool, usize),
    mode: Mode,
    view_options: view_options::ViewOptions,
    columns: usize,
    size: Entity<orbit_ui::controls::SliderState>,
    transitions: Entity<crate::transitions::TransitionDefaults>,
    slide_transition: Option<Entity<crate::transitions::TransitionDefaults>>,
    text_style_clipboard: Option<Value>,
    presentation_tools: HashMap<(String, String, String), PresentationTools>,
    continuous: bool,
    selected: Vec<String>,
    show_selection: bool,
    selection_item: String,
    selection_entry: String,
    anchor: Option<usize>,
    menu: Option<Menu>,
    form: Option<Form>,
    notes: Option<Notes>,
    header_editor: Option<Entity<headers::Editor>>,
    pending_reflow: Option<(String, String)>,
    reflow_cursor: Option<usize>,
    reflow_scroll: gpui::ScrollHandle,
    reflow_preview_scroll: gpui::ScrollHandle,
    reflow_font: Entity<orbit_ui::controls::SliderState>,
    reflow_zoom: Entity<orbit_ui::controls::SliderState>,
    reflow_browser_item: String,
    /// Whole-presentation slide snapshots for the Reflow Undo and Redo buttons.
    reflow_undo: Vec<Value>,
    reflow_redo: Vec<Value>,
    nonce: u64,
    press: Option<Press>,
    tiles: TileBounds,
    media_drop: Option<MediaDrop>,
    arr_menu: Option<ArrangementMenu>,
    arrangement_manager: Option<arrangement_manager::Manager>,
    arr_chips: ChipBounds,
    arr_bar: Rc<RefCell<Option<gpui::Bounds<Pixels>>>>,
    arr_picker: Rc<std::cell::Cell<gpui::Bounds<Pixels>>>,
    arr_gap: Option<usize>,
    /// Slide reorders sent to the host and not answered yet; their snapshots must not overwrite the optimistic order.
    pending_reorders: HashMap<String, usize>,
    suppress_click: bool,
    scroll: ListState,
    focus: FocusHandle,
    loaded_generation: u64,
    loading: bool,
    clipboard: Vec<Value>,
    viewport: Rc<RefCell<Option<gpui::Bounds<Pixels>>>>,
    live_key: (Value, Value, Value, Value),
    workspace: String,
    playlist_rows: Vec<PlaylistRow>,
    playlist_scroll: ListState,
    list_metrics: (Pixels, Pixels),
    _subscriptions: Vec<Subscription>,
}
impl CueView {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let scroll = ListState::new(0, ListAlignment::Top, px(300.));
        scroll.set_scroll_handler(cx.listener(|view, event: &gpui::ListScrollEvent, _, _| {
            view.visible.retain(|(document, index), _| {
                event.visible_range.clone().any(|row| {
                    view.rows.get(row).is_some_and(|(doc, start, end)| {
                        doc == document && index >= start && index < end
                    })
                })
            });
        }));
        let playlist_scroll = ListState::new(0, ListAlignment::Top, px(300.));
        playlist_scroll.set_scroll_handler(cx.listener(
            |view, event: &gpui::ListScrollEvent, _, _| {
                view.visible.retain(|(document, index), _| {
                    event
                        .visible_range
                        .clone()
                        .any(|row| match view.playlist_rows.get(row) {
                            Some(PlaylistRow::Cues(row)) => {
                                view.rows.get(*row).is_some_and(|(doc, start, end)| {
                                    doc == document && index >= start && index < end
                                })
                            }
                            _ => false,
                        })
                });
            },
        ));
        let size = orbit_ui::controls::range_state(2., 8., 1., 5., cx);
        let reflow_font = orbit_ui::controls::range_state(0.75, 1.75, 0.05, 1., cx);
        let reflow_zoom = orbit_ui::controls::range_state(1., 4., 1., 2., cx);
        let size_subscription = cx.subscribe(&size, |view, _, event, cx| {
            let orbit_ui::controls::SliderEvent::Change(value) = event else {
                return;
            };
            view.columns = (10. - value.end()).round() as usize;
            view.playlist_scroll.reset(view.playlist_rows.len());
            cx.notify();
        });
        let subscriptions = vec![
            size_subscription,
            cx.observe(&reflow_font, |_, _, cx| cx.notify()),
            cx.observe(&reflow_zoom, |_, _, cx| cx.notify()),
            cx.observe(&runtime.content, |_, _, cx| cx.notify()),
            cx.observe(&runtime.ui, |_, _, cx| cx.notify()),
            cx.observe(&runtime.output, |_, _, cx| cx.notify()),
        ];
        let transitions = cx.new(|cx| {
            crate::transitions::TransitionDefaults::new(
                crate::transitions::Kind::Slide,
                runtime.clone(),
                cx,
            )
        });
        Self {
            runtime,
            visible: HashMap::new(),
            documents: Vec::new(),
            rows: Vec::new(),
            key: (u64::MAX, String::new(), String::new(), None, false, 0),
            mode: Mode::Easy,
            view_options: Default::default(),
            columns: 5,
            size,
            transitions,
            slide_transition: None,
            text_style_clipboard: None,
            presentation_tools: HashMap::new(),
            continuous: false,
            selected: Vec::new(),
            show_selection: false,
            selection_item: String::new(),
            selection_entry: String::new(),
            anchor: None,
            menu: None,
            form: None,
            notes: None,
            header_editor: None,
            pending_reflow: None,
            reflow_cursor: None,
            reflow_scroll: gpui::ScrollHandle::new(),
            reflow_preview_scroll: gpui::ScrollHandle::new(),
            reflow_font,
            reflow_zoom,
            reflow_browser_item: String::new(),
            reflow_undo: Vec::new(),
            reflow_redo: Vec::new(),
            nonce: 0,
            press: None,
            tiles: Rc::new(RefCell::new(HashMap::new())),
            media_drop: None,
            arr_menu: None,
            arrangement_manager: None,
            arr_chips: Rc::new(RefCell::new(Vec::new())),
            arr_bar: Rc::new(RefCell::new(None)),
            arr_picker: Rc::default(),
            arr_gap: None,
            pending_reorders: HashMap::new(),
            suppress_click: false,
            scroll,
            focus: cx.focus_handle(),
            loaded_generation: 0,
            loading: false,
            clipboard: Vec::new(),
            viewport: Rc::new(RefCell::new(None)),
            live_key: Default::default(),
            workspace: String::new(),
            playlist_rows: Vec::new(),
            playlist_scroll,
            list_metrics: (px(0.), px(0.)),
            _subscriptions: subscriptions,
        }
    }
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let content = self.runtime.content.read(cx);
        let ui = self.runtime.ui.read(cx);
        self.continuous =
            self.continuous(cx) && content.playlist(&ui.selected_playlist_id).is_some();
        let workspace = content.workspace_path.clone();
        let columns = if self.mode == Mode::Table {
            1
        } else {
            self.columns
        };
        let key = (
            content.items_generation,
            ui.selected_library_item_id.clone(),
            ui.selected_playlist_id.clone(),
            ui.selected_documents
                .first()
                .and_then(|selected| selected.index),
            self.continuous,
            columns,
        );
        if key == self.key {
            return;
        }
        if !self.loading
            && self.key.0 == key.0
            && self.key.2 == key.2
            && self.key.4 == key.4
            && (self.continuous || (self.key.1 == key.1 && self.key.3 == key.3))
        {
            let changed_columns = self.key.5 != key.5;
            if self.selection_item != key.1 {
                self.selected.clear();
                self.anchor = None;
            }
            self.key = key;
            if changed_columns {
                self.rebuild_rows(cx);
                self.visible.clear();
                self.live_key = Default::default();
            }
            return;
        }
        let mut seeds = Vec::new();
        if self.continuous {
            if let Some(playlist) = content.playlist(&ui.selected_playlist_id) {
                for (index, entry) in playlist.entries.iter().enumerate() {
                    if entry.raw["hidden"] == true {
                        continue;
                    }
                    let id = if entry.kind == EntryKind::Presentation {
                        &entry.item_id
                    } else {
                        &entry.linked_item_id
                    };
                    if id.is_empty() {
                        continue;
                    }
                    let mut selection = ui.clone();
                    selection.selected_library_item_id = id.clone();
                    selection.selected_documents = vec![Selected {
                        item_id: id.clone(),
                        index: Some(index),
                    }];
                    if let Some(seed) = model::seed(content, &selection) {
                        seeds.push(seed);
                    }
                }
            }
        } else {
            if let Some(seed) = model::seed(content, ui) {
                seeds.push(seed);
            }
        }
        let same_documents = !self.loading
            && self.workspace == workspace
            && self.key.2 == key.2
            && (self.continuous || (self.key.1 == key.1 && self.key.3 == key.3))
            && self.key.4 == key.4
            && self.key.5 == key.5
            && self.documents.len() == seeds.len()
            && self
                .documents
                .iter()
                .zip(&seeds)
                .all(|(document, seed)| document.item.id == seed.item.id);
        // A reorder is still in flight: its optimistic order stays until the last answer reconciles.
        if same_documents && !self.pending_reorders.is_empty() {
            self.key = key;
            return;
        }
        // The host confirmed what an optimistic edit already shows: adopt the generation without reloading.
        if same_documents
            && self.documents.iter().zip(&seeds).all(|(document, seed)| {
                document.item.value == seed.item.value
                    && document.entry == seed.entry
                    && document.arrangement == seed.arrangement
                    && document.playlist == seed.playlist
            })
        {
            if self.continuous
                && content
                    .playlist(&ui.selected_playlist_id)
                    .is_some_and(|playlist| {
                        let current = playlist
                            .entries
                            .iter()
                            .enumerate()
                            .filter(|(_, entry)| entry.raw["hidden"] != true);
                        let shown = self.playlist_rows.iter().filter_map(|row| match row {
                            PlaylistRow::Marker(entry, index) => Some((*index, entry.as_ref())),
                            PlaylistRow::Cues(_) | PlaylistRow::EndSpace => None,
                        });
                        !current.eq(shown)
                    })
            {
                self.rebuild_rows(cx);
            }
            self.key = key;
            return;
        }
        // The same documents with other content are refreshed in place: the old cues and thumbnails stay on screen.
        let in_place = same_documents;
        let order_only = in_place
            && self.documents.iter().zip(&seeds).all(|(document, seed)| {
                model::same_but_order(&document.item.value, &seed.item.value)
            });
        if self.workspace != workspace
            || self.key.2 != key.2
            || (!self.continuous
                && (self.key.1 != key.1 || self.key.3 != key.3 || self.key.4 != key.4))
        {
            self.selected.clear();
            self.anchor = None;
            self.menu = None;
            self.press = None;
            self.documents.clear();
            self.rows.clear();
            self.scroll.reset(0);
            self.visible.clear();
            if self.workspace != workspace || !self.reflow_open() {
                self.form = None;
            }
            self.notes = None;
            self.playlist_rows.clear();
            self.playlist_scroll.reset(0);
        }
        if self.workspace != workspace || self.key.2 != key.2 {
            self.presentation_tools.clear();
            self.arrangement_manager = None;
        }
        self.workspace = workspace;
        self.key = key.clone();
        self.loading = !in_place;
        cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    seeds
                        .into_iter()
                        .map(model::DocumentSeed::resolve)
                        .collect::<Result<Vec<_>, _>>()
                })
                .await;
            view.update(cx, |view, cx| {
                if view.key != key {
                    return;
                }
                view.loading = false;
                view.menu = None;
                if !in_place {
                    view.press = None;
                    view.visible.clear();
                    view.live_key = Default::default();
                }
                if !order_only {
                    view.loaded_generation = key.0;
                }
                match result {
                    Ok(documents) => view.documents = documents.into_iter().map(Arc::new).collect(),
                    Err(error) => {
                        view.documents.clear();
                        actions::error(&view.runtime, cx, error);
                    }
                }
                view.rebuild_rows(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    fn rebuild_rows(&mut self, cx: &App) {
        let columns = if self.mode == Mode::Table {
            1
        } else {
            self.columns
        };
        self.rows.clear();
        for (document, item) in self.documents.iter().enumerate() {
            let mut start = 0;
            let mut group = String::new();
            for (index, cue) in item.cues.iter().enumerate() {
                let next_group = if self.view_options.shown.group_slides {
                    arrangements::slide_group_id(cue)
                } else {
                    String::new()
                };
                let new_group = !next_group.is_empty() && next_group != group;
                if index - start == columns
                    || self.view_options.shown.group_slides && new_group && index > start
                {
                    self.rows.push((document, start, index));
                    start = index;
                }
                if new_group {
                    group = next_group;
                }
            }
            if start < item.cues.len() {
                self.rows.push((document, start, item.cues.len()));
            }
        }
        let top = self.scroll.logical_scroll_top();
        self.scroll.reset(self.rows.len());
        if top.item_ix < self.rows.len() {
            self.scroll.scroll_to(top);
        }
        let scroll = self.playlist_scroll.logical_scroll_top();
        self.playlist_rows.clear();
        if self.continuous {
            if let Some(playlist) = self
                .runtime
                .content
                .read(cx)
                .playlist(&self.runtime.ui.read(cx).selected_playlist_id)
            {
                for (index, entry) in playlist.entries.iter().enumerate() {
                    if entry.raw["hidden"] == true {
                        continue;
                    }
                    self.playlist_rows
                        .push(PlaylistRow::Marker(Box::new(entry.clone()), index));
                    if let Some(document) = self
                        .documents
                        .iter()
                        .position(|document| document.entry == entry.id)
                    {
                        self.playlist_rows.extend(
                            self.rows
                                .iter()
                                .enumerate()
                                .filter(|(_, (doc, _, _))| *doc == document)
                                .map(|(row, _)| PlaylistRow::Cues(row)),
                        );
                    }
                }
            }
        }
        if !self.playlist_rows.is_empty() {
            self.playlist_rows.push(PlaylistRow::EndSpace);
        }
        self.playlist_scroll.reset(self.playlist_rows.len());
        if scroll.item_ix < self.playlist_rows.len() {
            self.playlist_scroll.scroll_to(scroll);
        }
    }
    pub fn command(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading && matches!(command, "takeCue" | "editCue" | "newSlide") {
            return;
        }
        match command {
            "takeCue" => {
                if let Some((document, index)) = self.current(cx) {
                    self.take(document, index, cx);
                }
            }
            "nextSlide" => actions::navigate(&self.runtime, 1, cx),
            "previousSlide" => actions::navigate(&self.runtime, -1, cx),
            "clearAll" => actions::clear(&self.runtime, "all", cx),
            "clearSlide" => actions::clear(&self.runtime, "slide", cx),
            "clearMedia" => actions::clear(&self.runtime, "media", cx),
            "clearAudio" => actions::clear(&self.runtime, "music", cx),
            "clearProps" => actions::clear(&self.runtime, "props", cx),
            "clearMessages" => actions::clear(&self.runtime, "messages", cx),
            "clearAnnouncements" => actions::clear(&self.runtime, "announcements", cx),
            "clearToLogo" => actions::clear(&self.runtime, "logo", cx),
            "newSlide" => {
                if let Some(document) =
                    model::seed(self.runtime.content.read(cx), self.runtime.ui.read(cx))
                {
                    let after = self.selected.last().cloned();
                    let mut operation = json!({"type":"insertBlankSlide"});
                    if let Some(id) = after {
                        operation["afterSlideId"] = json!(id);
                    }
                    actions::mutate(&self.runtime, &document.item.id, operation, cx);
                }
            }
            "open-ccli" => {
                let item = self.runtime.ui.read(cx).selected_library_item_id.clone();
                self.open_song_editor(&item, window, cx);
            }
            "editCue" => {
                if let Some((document, index)) = self.current(cx) {
                    let id = self.documents[document].cues[index]["id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    self.open_form(FormKind::Text(id), document, window, cx);
                }
            }
            _ => {}
        }
    }
    pub fn edit_command(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.reflow_open() {
            match action {
                "undo" => self.reflow_history(true, window, cx),
                "redo" => self.reflow_history(false, window, cx),
                _ => {}
            }
            return;
        }
        if self.loading {
            return;
        }
        if action == "deselectAll" {
            self.selected.clear();
            self.anchor = None;
            cx.notify();
            return;
        }
        let Some((document, index)) = self.current(cx) else {
            return;
        };
        if action == "selectAll" {
            self.selection_item = self.documents[document].item.id.clone();
            self.selection_entry = self.documents[document].entry.clone();
            let mut seen = std::collections::HashSet::new();
            self.selected = self.documents[document]
                .cues
                .iter()
                .filter_map(|cue| cue["id"].as_str().map(str::to_owned))
                .filter(|id| seen.insert(id.clone()))
                .collect();
            self.show_selection = true;
            cx.notify();
        } else {
            self.context_action(document, index, action, window, cx);
        }
    }
    pub fn continuous(&self, cx: &App) -> bool {
        actions::continuous(&self.runtime, cx)
    }
    fn toggle_continuous(&mut self, cx: &mut Context<Self>) {
        let settings = crate::settings::persist::shown(&self.runtime, cx);
        let payload = crate::settings::persist::general_payload(
            &settings,
            &json!({"useContinuousPlaylist": !self.continuous(cx)}),
        );
        let task = crate::settings::persist::save(&self.runtime, cx, payload);
        cx.spawn(async move |view, cx| {
            if let Err(error) = task.await {
                view.update(cx, |view, cx| actions::error(&view.runtime, cx, error))
                    .ok();
            }
        })
        .detach();
        cx.notify();
    }
    pub fn view_mode(&self) -> &'static str {
        self.mode.key()
    }
    pub fn set_view(&mut self, mode: &str, cx: &mut Context<Self>) {
        let previous = self.mode;
        match mode {
            "continuous" => self.toggle_continuous(cx),
            "easy" => self.mode = Mode::Easy,
            "grid" => self.mode = Mode::Grid,
            "table" => self.mode = Mode::Table,
            _ => return,
        }
        if self.mode != previous {
            self.visible.clear();
            self.playlist_scroll.reset(self.playlist_rows.len());
        }
        cx.notify();
    }

    pub fn reflow_open(&self) -> bool {
        self.form
            .as_ref()
            .is_some_and(|form| matches!(form.kind, FormKind::Text(_)))
    }
    pub fn song_editor_open(&self) -> bool {
        self.form
            .as_ref()
            .is_some_and(|form| matches!(form.kind, FormKind::Song))
    }
    pub fn close_form(&mut self, cx: &mut Context<Self>) {
        self.save_form(cx);
    }
    pub fn selected_location(&self, cx: &App) -> Option<(String, String)> {
        if self.loading {
            return None;
        }
        let (document, index) = self.current(cx)?;
        Some((
            self.documents[document].item.id.clone(),
            self.documents[document].cues[index]["id"]
                .as_str()?
                .to_string(),
        ))
    }
    pub fn request_reflow(&mut self, item: String, slide: String, cx: &mut Context<Self>) {
        self.reflow_undo.clear();
        self.reflow_redo.clear();
        self.pending_reflow = Some((item, slide));
        self.reflow_browser_item = self.runtime.ui.read(cx).selected_library_item_id.clone();
        cx.notify();
    }

    fn sync_reflow_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !(self.reflow_open() || self.pending_reflow.is_some())
            || self.form.as_ref().is_some_and(|form| form.saving)
        {
            return;
        }
        let selected = self.runtime.ui.read(cx).selected_library_item_id.clone();
        if self.reflow_browser_item == selected {
            return;
        }
        self.reflow_browser_item = selected.clone();
        let item = self.runtime.content.read(cx).items.get(&selected).cloned();
        let Some(item) = item else {
            return;
        };
        if self.form.as_ref().is_some_and(|form| form.item == item.id) {
            return;
        }
        let slide = item.value["slides"]
            .as_array()
            .and_then(|slides| slides.first())
            .and_then(|slide| slide["id"].as_str())
            .unwrap_or_default()
            .to_string();
        if let Some(task) = self.flush_reflow(cx) {
            task.detach();
        }
        self.pending_reflow = None;
        self.reflow_cursor = None;
        self.open_item_form(FormKind::Text(slide), &item, window, cx);
    }
    fn current(&self, cx: &App) -> Option<(usize, usize)> {
        self.documents
            .iter()
            .enumerate()
            .find_map(|(document, item)| {
                let selected = |cue: &Value| {
                    self.selection_item == item.item.id
                        && self.selection_entry == item.entry
                        && self.selected.iter().any(|id| cue["id"] == *id)
                };
                self.anchor
                    .filter(|index| item.cues.get(*index).is_some_and(selected))
                    .or_else(|| item.cues.iter().position(selected))
                    .map(|index| (document, index))
            })
            .or_else(|| {
                let ui = self.runtime.ui.read(cx);
                let content = self.runtime.content.read(cx);
                let entry = ui
                    .selected_documents
                    .first()
                    .and_then(|selected| selected.index)
                    .and_then(|index| {
                        content
                            .playlist(&ui.selected_playlist_id)?
                            .entries
                            .get(index)
                    });
                self.documents
                    .iter()
                    .enumerate()
                    .find(|(_, document)| {
                        !document.cues.is_empty()
                            && document.item.id == ui.selected_library_item_id
                            && entry.is_none_or(|entry| entry.id == document.entry)
                    })
                    .map(|(index, _)| (index, 0))
            })
    }
    fn take(&self, document: usize, index: usize, cx: &mut App) {
        let document = &self.documents[document];
        actions::take(
            &self.runtime,
            document,
            index,
            self.pending_reorders.contains_key(&document.item.id),
            cx,
        );
    }

    fn click(
        &mut self,
        document: usize,
        index: usize,
        shift: bool,
        toggle: bool,
        cx: &mut Context<Self>,
    ) {
        if self.loading {
            return;
        }
        if self.suppress_click {
            self.suppress_click = false;
            return;
        }
        let item = &self.documents[document];
        if self.selection_item != item.item.id || self.selection_entry != item.entry {
            self.selected.clear();
            self.anchor = None;
            self.selection_item = item.item.id.clone();
            self.selection_entry = item.entry.clone();
        }
        self.selected = model::selected_indices(
            &item.cues,
            &self.selected,
            self.anchor,
            index,
            shift,
            toggle,
        );
        self.show_selection = shift || toggle;
        if !shift {
            self.anchor = Some(index);
        }
        if !shift && !toggle {
            if self.continuous {
                let target = item.clone();
                self.runtime.edit_ui(cx, |content, ui| {
                    ui.selected_playlist_id = target.playlist.clone();
                    if let Some(index) = content.playlist(&target.playlist).and_then(|playlist| {
                        playlist
                            .entries
                            .iter()
                            .position(|entry| entry.id == target.entry)
                    }) {
                        crate::collections::selection::click_entry_row(
                            content,
                            ui,
                            &target.entry,
                            index,
                            Default::default(),
                        );
                    }
                });
            }
            self.take(document, index, cx);
        }
        cx.notify();
    }
    /// A group letter or digit takes the first cue of that group, from anywhere in the window.
    pub fn group_hotkey(&mut self, keystroke: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let modifiers = &keystroke.modifiers;
        if modifiers.control || modifiers.platform || modifiers.alt {
            return false;
        }
        let Some((document, _)) = self.current(cx) else {
            return false;
        };
        let groups = self
            .runtime
            .output
            .read(cx)
            .snapshot
            .as_ref()
            .map(|s| s["groupSettings"].clone());
        let Some(index) = groups.and_then(|groups| {
            model::hotkey_target(&self.documents[document].cues, &groups, &keystroke.key)
        }) else {
            return false;
        };
        self.take(document, index, cx);
        true
    }
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if orbit_ui::controls::text_entry_focused(window) {
            return;
        }
        let toggle = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            self.suppress_click |= self.press.is_some();
            self.menu = None;
            self.press = None;
            self.selected.clear();
            self.anchor = None;
            cx.notify();
            cx.stop_propagation();
            return;
        }
        let Some((document, current)) = self.current(cx) else {
            return;
        };
        match key {
            "enter" => self.take(document, current, cx),
            "up" | "down" => {
                let step = if self.mode == Mode::Table {
                    1
                } else {
                    self.columns
                };
                let adjacent = self
                    .view_options
                    .shown
                    .group_slides
                    .then(|| {
                        let row = self.rows.iter().position(|(doc, start, end)| {
                            *doc == document && (*start..*end).contains(&current)
                        })?;
                        let target = if key == "up" {
                            row.checked_sub(1)?
                        } else {
                            row + 1
                        };
                        let &(doc, start, end) = self.rows.get(target)?;
                        (doc == document).then(|| (start + current - self.rows[row].1).min(end - 1))
                    })
                    .flatten();
                let next = adjacent.unwrap_or_else(|| {
                    if key == "up" {
                        current.saturating_sub(step)
                    } else {
                        (current + step).min(self.documents[document].cues.len() - 1)
                    }
                });
                self.selection_item = self.documents[document].item.id.clone();
                self.selection_entry = self.documents[document].entry.clone();
                self.selected = model::selected_indices(
                    &self.documents[document].cues,
                    &self.selected,
                    self.anchor,
                    next,
                    event.keystroke.modifiers.shift,
                    toggle,
                );
                self.show_selection = event.keystroke.modifiers.shift || toggle;
                if !event.keystroke.modifiers.shift {
                    self.anchor = Some(next);
                }
                if let Some(row) = self
                    .rows
                    .iter()
                    .position(|(doc, start, end)| *doc == document && next >= *start && next < *end)
                {
                    if self.continuous {
                        if let Some(index) = self.playlist_rows.iter().position(
                            |entry| matches!(entry, PlaylistRow::Cues(index) if *index == row),
                        ) {
                            self.playlist_scroll.scroll_to_reveal_item(index);
                        }
                    } else {
                        self.reveal_cue_row(row);
                    }
                }
                cx.notify();
            }
            "a" if toggle => self.edit_command("selectAll", window, cx),
            "delete" => self.context_action(document, current, "delete", window, cx),
            "c" if toggle => self.context_action(document, current, "copy", window, cx),
            "x" if toggle => self.context_action(document, current, "cut", window, cx),
            "v" if toggle => self.context_action(document, current, "paste", window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }
    fn clear_slide_selection(&mut self, cx: &mut Context<Self>) {
        if !self.selected.is_empty() || self.anchor.is_some() {
            self.selected.clear();
            self.anchor = None;
            self.show_selection = false;
            cx.notify();
        }
    }

    fn selected_for(&self, item: &Document, id: &str) -> Vec<String> {
        if self.selection_item == item.item.id
            && self.selection_entry == item.entry
            && self.selected.iter().any(|selected| selected == id)
        {
            self.selected.clone()
        } else {
            vec![id.to_string()]
        }
    }

    fn context_action(
        &mut self,
        document: usize,
        index: usize,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.documents.get(document).cloned() else {
            return;
        };
        let Some(cue) = item.cues.get(index) else {
            return;
        };
        let id = cue["id"].as_str().unwrap_or("").to_string();
        let ids = self.selected_for(&item, &id);
        self.menu = None;
        match action {
            "edit" => self.open_form(FormKind::Text(id), document, window, cx),
            "edit-slide" => cx.emit(EditSlide {
                item: item.item.id.clone(),
                slide: id,
            }),
            "shortcut" => self.open_form(FormKind::Shortcut(id), document, window, cx),
            "transitions" => self.open_slide_transition(item.item.id.clone(), id, window, cx),
            "completion" => cx.emit(SlideMenuRequest::Actions {
                item: item.item.id.clone(),
                slide: id,
                section: Some("completion"),
            }),
            "copy-style" => self.copy_slide_style(&item.item, &id),
            "paste-style" => self.paste_slide_style(&item.item.id, &ids, cx),
            "copy" | "cut" | "duplicate" => {
                let slides: Vec<_> = item.item.value["slides"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|slide| ids.iter().any(|id| slide["id"] == *id))
                    .cloned()
                    .collect();
                let Some(last) = slides.last() else {
                    return;
                };
                if action == "duplicate" {
                    actions::mutate(
                        &self.runtime,
                        &item.item.id,
                        json!({"type":"pasteSlides","afterSlideId":last["id"],"slides":slides}),
                        cx,
                    );
                    cx.notify();
                    return;
                }
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(if slides.len() == 1 {
                    slides[0].to_string()
                } else {
                    json!(slides).to_string()
                }));
                self.clipboard = slides;
                if action == "cut" {
                    actions::delete_selected(&self.runtime, &item.item.id, ids, cx);
                    self.selected.clear();
                    self.anchor = None;
                }
            }
            "paste" => {
                if !self.clipboard.is_empty() {
                    actions::mutate(
                        &self.runtime,
                        &item.item.id,
                        json!({"type":"pasteSlides","afterSlideId":id,"slides":self.clipboard}),
                        cx,
                    );
                }
            }
            "delete" => {
                actions::delete_selected(&self.runtime, &item.item.id, ids, cx);
                self.selected.clear();
                self.anchor = None;
            }
            "enable" | "disable" => actions::set_selected_enabled(
                &self.runtime,
                &item.item.id,
                ids,
                action == "enable",
                cx,
            ),
            _ => {}
        }
        cx.notify();
    }
    fn gap_from(&self, document: usize, position: Point<Pixels>) -> Option<usize> {
        let tiles = self.tiles.borrow();
        let slides = self.documents[document].item.value["slides"].as_array()?;
        let candidates = tiles
            .iter()
            .filter(|((doc, _, master), _)| *doc == document && *master < slides.len())
            .map(|((_, _, master), bounds)| (*master, *bounds));
        if self.continuous
            && (!candidates
                .clone()
                .any(|(_, bounds)| bounds.top() <= position.y)
                || !candidates
                    .clone()
                    .any(|(_, bounds)| bounds.bottom() >= position.y))
        {
            return None;
        }
        crate::gap::gap_at(position, candidates, self.mode == Mode::Table)
    }

    fn media_drop_at(
        &self,
        row: (usize, usize, usize),
        position: Point<Pixels>,
        drag: &AssetDrag,
        cx: &App,
    ) -> Option<MediaDrop> {
        if self.loading
            || self.workspace != drag.workspace
            || drag.slide_media.is_empty()
            || !self
                .viewport
                .borrow()
                .is_some_and(|bounds| bounds.contains(&position))
        {
            return None;
        }
        let (document, start, end) = row;
        let tiles = self.tiles.borrow();
        let (cue, bounds) = crate::gap::closest_tile(
            position,
            tiles
                .iter()
                .filter(|((doc, cue, _), _)| *doc == document && (start..end).contains(cue))
                .map(|((_, cue, _), bounds)| (*cue, *bounds)),
        )?;
        let (value, start, end) = if self.mode == Mode::Table {
            (position.y, bounds.top(), bounds.bottom())
        } else {
            (position.x, bounds.left(), bounds.right())
        };
        let edge = cx.theme().metrics.space_md.min((end - start) / 4.);
        let placement = if bounds.contains(&position) && value > start + edge && value < end - edge
        {
            MediaPlacement::Background
        } else if value <= (start + end) / 2. {
            MediaPlacement::Before
        } else {
            MediaPlacement::After
        };
        Some(MediaDrop {
            document,
            cue,
            placement,
        })
    }

    fn drop_media(&mut self, target: MediaDrop, drag: &AssetDrag, cx: &mut Context<Self>) {
        let Some(document) = self.documents.get(target.document) else {
            return;
        };
        let Some(slide) = document
            .cues
            .get(target.cue)
            .and_then(|cue| cue["id"].as_str())
        else {
            return;
        };
        let operation = match target.placement {
            MediaPlacement::Background => json!({
                "type":"setSlideMedia","slideId":slide,"mediaId":drag.id,
            }),
            placement => json!({
                "type":"insertMediaSlides","slideId":slide,"mediaIds":drag.slide_media.as_ref(),
                "newSlideIds":drag.slide_media.iter().map(|_| format!("media-{}", uuid::Uuid::new_v4()))
                    .collect::<Vec<_>>(),
                "placement":if placement == MediaPlacement::Before {"before"} else {"after"},
            }),
        };
        actions::drop_media(&self.runtime, &document.item.id, operation, cx);
    }
    fn finish_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(press) = self.press.take() {
            if !self.loading && press.dragging && press.generation == self.loaded_generation {
                let over_viewport = self
                    .viewport
                    .borrow()
                    .is_some_and(|bounds| bounds.contains(&position));
                let target = over_viewport
                    .then(|| self.gap_from(press.document, position))
                    .flatten()
                    .and_then(|gap| model::drop_index(&press.sources, gap));
                if let Some(index) = target {
                    let item = &self.documents[press.document].item;
                    let ids: Vec<_> = press
                        .sources
                        .iter()
                        .map(|source| item.value["slides"][*source]["id"].clone())
                        .collect();
                    if let Some(next) =
                        model::reordered(&self.documents[press.document], &press.sources, index)
                    {
                        if self.selection_item == next.item.id && self.selection_entry == next.entry
                        {
                            self.anchor = self.anchor.and_then(|old| {
                                let id = &self.documents[press.document].cues.get(old)?["id"];
                                next.cues.iter().position(|cue| cue["id"] == *id)
                            });
                        }
                        self.documents[press.document] = Arc::new(next);
                        self.rebuild_rows(cx);
                    }
                    let item = self.documents[press.document].item.id.clone();
                    *self.pending_reorders.entry(item.clone()).or_default() += 1;
                    let saved = actions::mutate_checked(
                        &self.runtime,
                        &item,
                        json!({"type":"reorderSlides","slideIds":ids,"targetIndex":index}),
                        cx,
                    );
                    cx.spawn(async move |view, cx| {
                        let accepted = saved.await.is_some();
                        view.update(cx, |view, cx| {
                            if let std::collections::hash_map::Entry::Occupied(mut count) =
                                view.pending_reorders.entry(item)
                            {
                                *count.get_mut() -= 1;
                                if *count.get() == 0 {
                                    count.remove();
                                }
                            }
                            // The last answer reconciles with the host's content; a rejection restores its order.
                            if view.pending_reorders.is_empty() || !accepted {
                                view.key.0 = u64::MAX;
                                cx.notify();
                            }
                        })
                        .ok();
                    })
                    .detach();
                }
            }
        }
        cx.notify();
    }
    fn card(
        &mut self,
        document: usize,
        index: usize,
        row: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let item = self.documents[document].clone();
        let cue = item.cues[index].clone();
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let output = &self.runtime.output.read(cx).live;
        let live = model::is_presentation_live(output, &item, &cue);
        let announcement_live = model::is_announcement_live(output, &item, &cue);
        let selected = self.selection_item == item.item.id
            && self.selection_entry == item.entry
            && self.selected.iter().any(|id| cue["id"] == *id);
        let disabled = cue["disabled"] == true || self.loading;
        let master = item.item.value["slides"]
            .as_array()
            .and_then(|slides| slides.iter().position(|slide| slide["id"] == cue["id"]))
            .unwrap_or(index);
        let group = model::effective_group(&item.cues, index);
        let setting = self
            .runtime
            .output
            .read(cx)
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot["groupSettings"][&group].clone())
            .unwrap_or(Value::Null);
        let label = setting["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .unwrap_or(&group)
            .to_string();
        let color = setting["color"]
            .as_str()
            .and_then(|color| color.strip_prefix('#'))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .map(gpui::rgb)
            .unwrap_or(theme.colors.cue_default);
        let text_view = self.mode == Mode::Easy;
        let options = &self.view_options.shown;
        let thumbnail_color = options.use_thumbnail_color.then(|| {
            options
                .thumbnail_color
                .clone()
                .unwrap_or_else(|| view_options::color_hex(theme.colors.preview_matte_bg))
        });
        let show_preview = !text_view;
        let snapshot = self.runtime.output.read(cx).snapshot.clone();
        let preview = snapshot.filter(|_| show_preview).map(|snapshot| {
            let host = self.runtime.host.clone();
            let frames = self.runtime.frames.clone();
            let preview = self
                .visible
                .entry((document, index))
                .or_insert_with(|| cx.new(|cx| Preview::new(host, frames, cx)))
                .clone();
            let key = format!(
                "{}:{}:{}:{}:{master}:{}:{thumbnail_color:?}",
                self.runtime.content.read(cx).workspace_path,
                self.loaded_generation,
                item.item.id,
                item.entry,
                cue["id"],
            );
            preview.update(cx, |preview, cx| {
                preview.set_source(
                    Some(Source::Cue {
                        key,
                        snapshot,
                        item: item.item.clone(),
                        index: master,
                        playlist_id: item.playlist.clone(),
                        background: thumbnail_color,
                    }),
                    Some(screen_size(&cue)),
                    self.loaded_generation,
                    cx,
                )
            });
            preview
        });
        let table = self.mode == Mode::Table;
        let row_height = table_height(self.columns);
        let first_in_group = !group.is_empty()
            && (index == 0 || model::effective_group(&item.cues, index - 1) != group);
        let key = cue["shortcut"]
            .as_str()
            .filter(|key| !key.is_empty())
            .unwrap_or_else(|| setting["key"].as_str().unwrap_or(""));
        let shortcut = if self
            .runtime
            .output
            .read(cx)
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| {
                model::hotkey_target(&item.cues, &snapshot["groupSettings"], key) == Some(index)
            }) {
            key.to_string()
        } else {
            String::new()
        };
        let media_layers = if !text_view || options.show_media_and_colors {
            slide_menu::connected_visual_layers(&self.runtime.content.read(cx).workspace, &cue)
        } else {
            [false; 2]
        };
        let label = if first_in_group { label } else { String::new() };
        // `.cue-card-label`: the slide's own label, in its own color.
        let label_chip = cue["label"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(|name| {
                let color = cue["labelColor"].as_str().and_then(parse_hex_color);
                (name.to_string(), color)
            });
        let mut footer = Some(
            div()
                .debug_selector(move || format!("cue-footer-{index}"))
                .flex_none()
                .h(if table {
                    theme.metrics.font_xl
                } else {
                    theme.metrics.row_height
                })
                .flex()
                .items_center()
                .when(table, |el| el.ml(theme.metrics.space_md).self_start())
                .gap(theme.metrics.space_snug)
                .px(theme.metrics.space_snug)
                .bg(color)
                .text_size(theme.metrics.font_sm)
                .when(!table, |el| el.child((index + 1).to_string()))
                .child(div().min_w_0().truncate().child(label))
                .when_some(label_chip, |el, (name, color)| {
                    el.child(
                        div()
                            .debug_selector(move || format!("cue-label-{index}"))
                            .flex_none()
                            .max_w(gpui::relative(0.5))
                            .ml_auto()
                            .px(theme.metrics.space_xs)
                            .truncate()
                            .rounded(theme.metrics.radius_sm)
                            .when_some(color, |el, color| el.bg(color))
                            .text_color(theme.colors.text_strong)
                            .text_size(theme.metrics.font_xs)
                            .child(name),
                    )
                })
                .into_any_element(),
        );
        let base_font = if table {
            theme.metrics.font_md
        } else {
            theme.metrics.font_base
        };
        let font_size = options.font_size.map_or(base_font, |size| {
            theme.metrics.font_base * (f32::from(size) / 14.)
        });
        let text_color = options
            .text_color
            .as_deref()
            .and_then(parse_hex_color)
            .unwrap_or(theme.colors.text);
        let authored_color =
            (text_view && options.show_media_and_colors && cue["backgroundEnabled"] != false)
                .then(|| {
                    let slide = if cue.get("background").is_some() {
                        &cue
                    } else {
                        &cue["presentationThemeSnapshot"]
                    };
                    orbit_host::native_compositor::opaque_canvas_matte(
                        slide,
                        orbit_host::native_compositor::CanvasRenderKind::Presentation,
                    )
                    .map(gpui::rgb)
                })
                .flatten();
        let background = if table {
            options
                .table_color
                .as_deref()
                .and_then(parse_hex_color)
                .unwrap_or(theme.colors.stage_bg)
        } else if text_view {
            authored_color
                .or_else(|| options.thumbnail_color.as_deref().and_then(parse_hex_color))
                .unwrap_or(theme.colors.preview_matte_bg)
        } else {
            theme.colors.preview_matte_bg
        };
        let mut copy = (text_view || table).then(|| {
            div()
                .debug_selector(move || format!("cue-copy-{index}"))
                .flex_1()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .p(theme.metrics.space_sm)
                .when(
                    text_view
                        && (!shortcut.is_empty() || media_layers.into_iter().any(|shown| shown)),
                    |el| el.pt(theme.metrics.row_height),
                )
                .child(preview_text::text(
                    cue["content"].as_str().unwrap_or("").to_string(),
                    options,
                    font_size,
                    text_color,
                ))
        });
        let mut visual = Some(
            div()
                .debug_selector(move || format!("cue-visual-{index}"))
                .relative()
                .w_full()
                .aspect_ratio(16. / 9.)
                .flex_none()
                .overflow_hidden()
                .when_some(preview, |el, preview| {
                    el.child(
                        div()
                            .debug_selector(move || format!("cue-picture-{index}"))
                            .absolute()
                            .size_full()
                            .child(preview),
                    )
                })
                .when(text_view, |el| {
                    el.child(div().absolute().size_full().flex().children(copy.take()))
                })
                .into_any_element(),
        );
        let tiles = self.tiles.clone();
        let dragging = self.press.as_ref().is_some_and(|press| {
            press.dragging && press.document == document && press.sources.contains(&master)
        });
        let media_target = self.media_drop.filter(|target| {
            cx.has_active_drag() && target.document == document && target.cue == index
        });
        let media_before =
            media_target.is_some_and(|target| target.placement == MediaPlacement::Before);
        let media_after =
            media_target.is_some_and(|target| target.placement == MediaPlacement::After);
        let reorder = self
            .press
            .as_ref()
            .filter(|press| press.dragging && press.document == document)
            .and_then(|press| {
                press
                    .gap
                    .filter(|gap| model::drop_index(&press.sources, *gap).is_some())
            });
        let before = media_before || reorder == Some(master);
        let after = media_after || reorder == Some(master + 1);
        let token = Rc::new((self.key.clone(), self.loaded_generation));
        let tile = div()
            .relative()
            .id(SharedString::from(format!("cue-{document}-{index}")))
            .debug_selector(move || format!("cue-thumbnail-{index}"))
            .role(gpui::Role::Button)
            .aria_label(t.format("cue.selectSlide", &[("number", &(index + 1).to_string())]))
            .aria_selected(selected)
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .when(table, |el| el.h(row_height).flex_row())
            .when(!table, |el| el.rounded(theme.metrics.radius_xs))
            .border_1()
            .border_color(if live {
                theme.colors.live
            } else {
                theme.colors.border_soft
            })
            .when(dragging, |el| el.opacity(0.72).cursor_grabbing())
            .bg(background)
            .text_color(if disabled {
                theme.colors.disabled_text
            } else {
                theme.colors.text
            })
            .hover(|style| style.bg(theme.colors.control_bg_hover))
            .when(!dragging, |el| el.cursor_pointer())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let token = token.clone();
                    move |this, event: &gpui::MouseDownEvent, window, cx| {
                        if this.loading || token.0 != this.key || token.1 != this.loaded_generation
                        {
                            return;
                        }
                        this.focus.focus(window, cx);
                        this.suppress_click = false;
                        this.press = None;
                        if event.click_count > 1 {
                            return;
                        }
                        let item = &this.documents[document];
                        let Some(id) = item.cues[index]["id"].as_str() else {
                            return;
                        };
                        let selected = this.selected_for(item, id);
                        let sources: Vec<_> = item.item.value["slides"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .enumerate()
                            .filter(|(_, slide)| selected.iter().any(|id| slide["id"] == *id))
                            .map(|(index, _)| index)
                            .collect();
                        if sources.is_empty() {
                            return;
                        }
                        let generation = this.loaded_generation;
                        this.press = Some(Press {
                            start: event.position,
                            document,
                            sources,
                            dragging: false,
                            gap: None,
                            generation,
                        });
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    this.finish_drag(event.position, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener({
                    let token = token.clone();
                    move |this, event: &gpui::MouseDownEvent, window, cx| {
                        if this.loading || token.0 != this.key || token.1 != this.loaded_generation
                        {
                            return;
                        }
                        let item = &this.documents[document];
                        let id = item.cues[index]["id"].as_str().unwrap_or("");
                        let selected = this.selected_for(item, id);
                        if selected.len() == 1 {
                            this.anchor = Some(index);
                        }
                        this.selected = selected;
                        this.selection_item = item.item.id.clone();
                        this.selection_entry = item.entry.clone();
                        this.focus.focus(window, cx);
                        this.menu = Some(Menu {
                            position: event.position,
                            document,
                            index,
                        });
                        cx.notify();
                        cx.stop_propagation();
                    }
                }),
            )
            .on_click(cx.listener({
                let token = token.clone();
                move |this, event: &gpui::ClickEvent, _, cx| {
                    if this.loading || token.0 != this.key || token.1 != this.loaded_generation {
                        return;
                    }
                    cx.stop_propagation();
                    this.click(
                        document,
                        index,
                        event.modifiers().shift,
                        event.modifiers().control || event.modifiers().platform,
                        cx,
                    )
                }
            }))
            .when(!table, |el| {
                el.child(visual.take().unwrap())
                    .child(footer.take().unwrap())
            })
            .when(table, |el| {
                el.child(
                    div()
                        .debug_selector(move || format!("cue-number-{index}"))
                        .w(theme.metrics.tabbar_height)
                        .h_full()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(color)
                        .child((index + 1).to_string()),
                )
                .child(
                    div()
                        .w(gpui::relative(0.38))
                        .max_w(row_height * (16. / 9.))
                        .min_w(px(96.))
                        .flex_none()
                        .child(visual.take().unwrap()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .flex()
                        .flex_col()
                        .child(footer.take().unwrap())
                        .children(copy.take()),
                )
            })
            .when(
                !shortcut.is_empty() || media_layers.into_iter().any(|shown| shown),
                |el| {
                    el.child(
                        div()
                            .absolute()
                            .top(if table {
                                theme.metrics.font_xl + theme.metrics.space_xs
                            } else {
                                theme.metrics.space_xs
                            })
                            .left(if table {
                                theme.metrics.tabbar_height
                                    + row_height * (16. / 9.)
                                    + theme.metrics.space_xs
                            } else {
                                theme.metrics.space_xs
                            })
                            .flex()
                            .items_center()
                            .gap(theme.metrics.space_2xs)
                            .when(!shortcut.is_empty(), |row| {
                                row.child(
                                    div()
                                        .min_w(px(17.))
                                        .h(px(17.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(theme.metrics.radius_md)
                                        .bg(theme.colors.warning)
                                        .text_color(theme.colors.preview_matte_bg)
                                        .text_size(theme.metrics.font_sm)
                                        .font_weight(gpui::FontWeight::EXTRA_BOLD)
                                        .child(shortcut),
                                )
                            })
                            .children(
                                [
                                    ("cue-media-background", false, "media.behavior.background"),
                                    ("cue-media-foreground", true, "media.behavior.foreground"),
                                ]
                                .into_iter()
                                .zip(media_layers)
                                .filter(|(_, shown)| *shown)
                                .map(
                                    |((id, foreground, label), _)| {
                                        let label = t.t(label);
                                        let id = format!("{id}-{index}");
                                        div()
                                            .id(SharedString::from(id.clone()))
                                            .debug_selector(move || id.clone())
                                            .flex_none()
                                            .aria_label(label.clone())
                                            .tooltip(orbit_ui::controls::tooltip_text(label))
                                            .child(orbit_ui::icons::media_layer_icon(
                                                foreground, &theme,
                                            ))
                                    },
                                ),
                            ),
                    )
                },
            )
            .when(disabled, |el| el.opacity(0.55))
            .when(selected && self.show_selection, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .debug_selector(move || format!("cue-selection-{index}"))
                        .border_2()
                        .border_color(theme.colors.selected),
                )
            })
            .when(live, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .debug_selector(move || format!("cue-live-{index}"))
                        .border_2()
                        .border_color(theme.colors.live),
                )
            })
            .when(announcement_live, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .when(live, |frame| {
                            let inset = theme.metrics.space_2xs;
                            frame.top(inset).right(inset).bottom(inset).left(inset)
                        })
                        .debug_selector(move || format!("cue-announcement-live-{index}"))
                        .border_2()
                        .border_color(theme.colors.live_announcement),
                )
            })
            .when(
                media_target.is_some_and(|target| target.placement == MediaPlacement::Background),
                |el| {
                    el.child(
                        div()
                            .absolute()
                            .inset_0()
                            .debug_selector(|| "cue-media-drop-background".into())
                            .border_2()
                            .border_color(theme.colors.accent),
                    )
                },
            )
            .child(
                gpui::canvas(
                    move |bounds, _, _| {
                        tiles.borrow_mut().insert((document, index, master), bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        // The tile clips its children, so the drop marker lives on this wrapper, centred in the gap to the neighbour.
        let metrics = theme.metrics;
        let row_start = index == row.start;
        let row_end = index + 1 == row.end;
        let gap_before = if row_start {
            metrics.field_padding_x
        } else {
            metrics.panel_padding
        };
        let gap_after = if row_end {
            metrics.field_padding_x
        } else {
            metrics.panel_padding
        };
        let thickness = theme.metrics.space_2xs;
        div()
            .relative()
            .flex()
            .flex_1()
            .min_w_0()
            .child(tile)
            .when(before || after, |el| {
                el.child(
                    div()
                        .absolute()
                        .when(media_before || media_after, |bar| {
                            bar.debug_selector(|| "cue-media-drop-insert".into())
                        })
                        .when(!media_before && !media_after, |bar| {
                            bar.debug_selector(|| "cue-reorder-marker".into())
                        })
                        .when(table, |el| {
                            el.left_0()
                                .right_0()
                                .h(thickness)
                                .when(before, |el| el.top(-thickness / 2.))
                                .when(!before, |el| el.bottom(-thickness / 2.))
                        })
                        .when(!table, |el| {
                            el.top_0()
                                .bottom_0()
                                .w(thickness)
                                .when(before, |el| el.left(-(gap_before + thickness) / 2.))
                                .when(!before, |el| el.right(-(gap_after + thickness) / 2.))
                        })
                        .bg(theme.colors.accent),
                )
            })
            .into_any_element()
    }
    fn reveal_cue_row(&self, row: usize) {
        if self.scroll.bounds_for_item(row).is_some() {
            self.scroll.scroll_to_reveal_item(row);
        } else {
            self.scroll.scroll_to(ListOffset {
                item_ix: row,
                offset_in_item: px(0.),
            });
        }
    }
    fn cue_row(&mut self, row: usize, before_heading: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some((document, start, end)) = self.rows.get(row).copied() else {
            return div().into_any_element();
        };
        let theme = *cx.theme();
        let columns = if self.mode == Mode::Table {
            1
        } else {
            self.columns
        };
        let occurrence = self.continuous.then(|| {
            let document = &self.documents[document];
            (document.playlist.clone(), document.entry.clone())
        });
        let selected = occurrence.as_ref().is_some_and(|(playlist, entry)| {
            let ui = self.runtime.ui.read(cx);
            ui.selected_playlist_id == *playlist
                && ui
                    .selected_documents
                    .first()
                    .and_then(|row| row.index)
                    .and_then(|index| {
                        self.runtime
                            .content
                            .read(cx)
                            .playlist(playlist)?
                            .entries
                            .get(index)
                    })
                    .is_some_and(|selected| selected.id == *entry)
        });
        div()
            .id(("cue-row", row))
            .w_full()
            .flex()
            .on_drag_move::<AssetDrag>(cx.listener(
                move |view, event: &gpui::DragMoveEvent<AssetDrag>, _, cx| {
                    let position = event.event.position;
                    let painted_row = (document, start, end);
                    let target = if event.bounds.contains(&position) {
                        view.media_drop_at(painted_row, position, event.drag(cx), cx)
                    } else if view.media_drop.is_some_and(|known| {
                        known.document != document || !(start..end).contains(&known.cue)
                    }) {
                        return;
                    } else {
                        None
                    };
                    if view.media_drop != target {
                        view.media_drop = target;
                        cx.notify();
                    }
                },
            ))
            .on_drop::<AssetDrag>(cx.listener(move |view, drag: &AssetDrag, window, cx| {
                view.media_drop = None;
                if let Some(target) =
                    view.media_drop_at((document, start, end), window.mouse_position(), drag, cx)
                {
                    view.drop_media(target, drag, cx);
                }
                cx.notify();
            }))
            .when(before_heading, |row| row.pb(theme.metrics.space_sm))
            .when(
                self.mode == Mode::Table
                    && self.view_options.shown.group_slides
                    && start > 0
                    && model::effective_group(&self.documents[document].cues, start)
                        != model::effective_group(&self.documents[document].cues, start - 1),
                |row| row.pt(theme.metrics.space_sm),
            )
            .when(selected, |row| row.bg(theme.colors.selected.opacity(0.20)))
            .when_some(occurrence, |row, (playlist, entry)| {
                row.debug_selector({
                    let entry = entry.clone();
                    move || format!("cue-playlist-slide-row-{entry}-{start}")
                })
                .on_click(cx.listener(move |view, event, window, cx| {
                    let index =
                        view.runtime
                            .content
                            .read(cx)
                            .playlist(&playlist)
                            .and_then(|list| {
                                list.entries
                                    .iter()
                                    .position(|candidate| candidate.id == entry)
                            });
                    if let Some(index) = index {
                        view.select_playlist_area(&playlist, &entry, index, event, window, cx);
                    }
                }))
            })
            .when(self.mode != Mode::Table, |el| {
                el.gap(theme.metrics.panel_padding)
                    .px(theme.metrics.field_padding_x)
                    .pt(theme.metrics.panel_padding)
            })
            .children(
                (start..end)
                    .map(|index| self.card(document, index, start..end, cx))
                    .collect::<Vec<_>>(),
            )
            .children((end - start..columns).map(|_| div().flex_1().min_w_0()))
            .into_any_element()
    }

    fn select_playlist_area(
        &mut self,
        playlist: &str,
        entry: &str,
        index: usize,
        event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        if self.selection_entry != entry {
            self.selected.clear();
            self.anchor = None;
        }
        self.runtime.edit_ui(cx, |content, ui| {
            if ui.selected_playlist_id == playlist {
                crate::collections::selection::click_entry_row(
                    content,
                    ui,
                    entry,
                    index,
                    crate::collections::selection::Modifiers {
                        range: event.modifiers().shift,
                        toggle: event.modifiers().control || event.modifiers().platform,
                    },
                );
            }
        });
        cx.notify();
    }
    fn reveal_playlist_selection(&self, cx: &mut Context<Self>) {
        let ui = self.runtime.ui.read(cx);
        let Some((playlist, entry_id)) = &ui.playlist_reveal else {
            return;
        };
        let row = (self.continuous && playlist == &ui.selected_playlist_id)
            .then(|| {
                self.playlist_rows.iter().position(
                    |row| matches!(row, PlaylistRow::Marker(entry, _) if &entry.id == entry_id),
                )
            })
            .flatten();
        if !self.continuous || playlist != &ui.selected_playlist_id || row.is_some() {
            self.runtime
                .ui
                .update(cx, |ui, _| ui.playlist_reveal = None);
        }
        if let Some(row) = row {
            self.playlist_scroll.scroll_to(gpui::ListOffset {
                item_ix: row,
                offset_in_item: px(0.),
            });
        }
    }

    fn placeholder_actions(
        &self,
        header: PanelHeader,
        playlist: &str,
        entry: &Entry,
        cx: &App,
    ) -> PanelHeader {
        let t = *cx.global::<Translator>();
        let runtime = self.runtime.clone();
        let entry_id = entry.id.clone();
        let playlist = playlist.to_string();
        if entry.linked_document().is_some() {
            return header.action(
                IconButton::new(
                    format!("cue-unlink-{entry_id}"),
                    "x",
                    t.t("cue.placeholder.unlink"),
                )
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    crate::collections::document_actions::link_placeholder(
                        &runtime, cx, &playlist, &entry_id, "",
                    )
                    .detach();
                }),
            );
        }
        let create = (runtime.clone(), playlist.clone(), entry_id.clone());
        header
            .action(
                IconButton::new(
                    format!("cue-create-{entry_id}"),
                    "plus",
                    t.t("collections.menu.newPresentation"),
                )
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    crate::collections::documents::open_presentation_for_placeholder(
                        &create.0,
                        cx,
                        create.1.clone(),
                        create.2.clone(),
                    );
                }),
            )
            .action(
                IconButton::new(
                    format!("cue-link-{entry_id}"),
                    "search",
                    t.t("cue.placeholder.linkPresentation"),
                )
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    crate::collections::search::open_placeholder_search(
                        &runtime,
                        cx,
                        playlist.clone(),
                        entry_id.clone(),
                    );
                }),
            )
    }

    fn presentation_header_actions(
        &mut self,
        header: PanelHeader,
        document: &Document,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PanelHeader {
        let item = document.item.id.clone();
        let entry = document.entry.clone();
        let playlist = document.playlist.clone();
        let runtime = self.runtime.clone();
        let tools = self
            .presentation_tools
            .entry((playlist.clone(), entry.clone(), item.clone()))
            .or_insert_with(|| {
                let destination = cx.new(|cx| {
                    let mut control = super::flow::Destination::new(runtime.clone(), cx);
                    control.item_id = item.clone();
                    control
                });
                let advance = cx.new(|cx| {
                    let mut control = super::flow::AutoAdvance::new(runtime.clone(), window, cx);
                    control.item_id = item.clone();
                    control
                });
                let transition = cx.new(|cx| {
                    let mut control = crate::transitions::TransitionDefaults::new(
                        crate::transitions::Kind::Slide,
                        runtime.clone(),
                        cx,
                    );
                    control.item_id = Some(item.clone());
                    control
                });
                PresentationTools {
                    destination,
                    advance,
                    transition,
                }
            });
        let t = *cx.global::<Translator>();
        let notes_open = self
            .notes
            .as_ref()
            .is_some_and(|notes| notes.item == item && notes.entry == entry);
        let notes_id =
            index.map_or_else(|| "cue-toolbar-1".into(), |_| format!("cue-notes-{entry}"));
        let arrangement_id = index.map_or_else(
            || "cue-toolbar-2".into(),
            |_| format!("cue-arrangement-{entry}"),
        );
        let notes = {
            let (item, entry) = (item.clone(), entry.clone());
            IconButton::new(notes_id, "notes", t.t("cue.operatorNotes"))
                .selected(notes_open)
                .on_click(cx.listener(move |view, _, window, cx| {
                    cx.stop_propagation();
                    view.toggle_item_notes(&item, &entry, window, cx);
                }))
        };
        let arrangement = IconButton::new(arrangement_id, "list", t.t("arrangements.edit"))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                let open = runtime
                    .ui
                    .read(cx)
                    .arrangement_editor
                    .as_ref()
                    .is_some_and(|editor| editor.item_id == item);
                if open {
                    arrangements::close_arrangement_editor(&runtime, cx);
                } else {
                    if let Some(index) = index {
                        runtime.edit_ui(cx, |content, ui| {
                            if ui.selected_playlist_id == playlist {
                                crate::collections::selection::click_entry_row(
                                    content,
                                    ui,
                                    &entry,
                                    index,
                                    Default::default(),
                                );
                            }
                        });
                    }
                    arrangements::open_arrangement_editor(&runtime, cx, &item);
                }
            });
        let theme = cx.theme();
        let separator = |selector: &'static str| {
            div()
                .debug_selector(move || selector.into())
                .flex_none()
                .w(theme.metrics.space_2xs)
                .h(theme.metrics.control_height_compact - theme.metrics.space_sm)
                .bg(theme.colors.text_subtle)
        };
        header
            .action(tools.advance.clone())
            .action(separator("cue-advance-separator"))
            .action(notes)
            .action(arrangement)
            .action(separator("cue-destination-separator"))
            .action(tools.destination.clone())
            .action(tools.transition.clone())
    }

    fn playlist_heading(
        &mut self,
        entry: Entry,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let runtime = self.runtime.clone();
        let document = self
            .documents
            .iter()
            .find(|doc| doc.entry == entry.id)
            .cloned();
        let title = document.as_ref().map_or_else(
            || {
                if entry.title.is_empty() {
                    t.t(if entry.kind == EntryKind::Header {
                        "cue.playlist.header"
                    } else {
                        "cue.playlist.placeholder"
                    })
                } else {
                    entry.title.clone()
                }
            },
            |doc| doc.item.display_title(&doc.arrangement),
        );
        let mut header = PanelHeader::new(title);
        let ui = runtime.ui.read(cx);
        let playlist = ui.selected_playlist_id.clone();
        let selected = ui.selected_documents.first().and_then(|row| row.index) == Some(index);
        let mut notes = None;
        let mut arrangement = None;
        if entry.kind == EntryKind::Placeholder {
            header = self.placeholder_actions(header, &playlist, &entry, cx);
        }
        if let Some(doc) = &document {
            header = self.presentation_header_actions(header, doc, Some(index), window, cx);
            if selected {
                arrangement = self.arrangement_panel(doc, cx);
            }
            if self
                .notes
                .as_ref()
                .is_some_and(|notes| notes.entry == entry.id)
            {
                notes = self.notes_element(cx);
            }
        }
        if entry.kind == EntryKind::Header {
            let actions = headers::actions(&entry);
            let trigger = runtime.clone();
            let settings = runtime.clone();
            let id = entry.id.clone();
            let request = headers::Request {
                playlist: playlist.clone(),
                entry: id.clone(),
            };
            let editor = self
                .header_editor
                .as_ref()
                .filter(|editor| editor.read(cx).request == request)
                .cloned();
            let toggle_request = request.clone();
            let settings_button = IconButton::new(
                format!("cue-header-settings-{id}"),
                "settings",
                t.t("playlistHeader.settings"),
            )
            .selected(editor.is_some())
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                headers::toggle(&settings, cx, toggle_request.clone());
            });
            let dismiss_runtime = runtime.clone();
            let popover = Popover::new(format!("cue-header-popover-{id}"), settings_button)
                .anchor(gpui::Anchor::TopRight)
                .on_dismiss(move |_, _, cx| headers::close(&dismiss_runtime, cx, &request))
                .when_some(editor, Popover::content);
            header = header
                .prominent()
                .action(
                    ActionButton::new(
                        format!("cue-header-trigger-{id}"),
                        t.t("playlistHeader.trigger"),
                    )
                    .compact()
                    .disabled(actions.is_empty())
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        crate::automation::run_operator(&trigger, actions.clone(), cx);
                    }),
                )
                .action(popover);
        }
        let theme = *cx.theme();
        let header_color = if entry.kind == EntryKind::Header {
            orbit_ui::controls::data_color(if entry.color.is_empty() {
                orbit_ui::controls::DEFAULT_HEADER_COLOR
            } else {
                &entry.color
            })
        } else {
            theme.colors.border
        };
        let stripe = if selected {
            theme.colors.selected
        } else {
            header_color
        };
        if entry.kind != EntryKind::Presentation && document.is_none() {
            header = header.contrast();
        }
        if entry.kind == EntryKind::Header
            && !entry.color.is_empty()
            && entry.color != "transparent"
        {
            header = header.section_tint(header_color);
        }
        div()
            .id(SharedString::from(format!(
                "cue-playlist-heading-{}",
                entry.id
            )))
            .debug_selector({
                let id = entry.id.clone();
                move || format!("cue-playlist-heading-{id}")
            })
            .child(
                div()
                    .id("playlist-heading-select")
                    .on_click(cx.listener(move |view, event, window, cx| {
                        view.select_playlist_area(&playlist, &entry.id, index, event, window, cx);
                    }))
                    .border_l(theme.metrics.space_xs)
                    .border_color(stripe)
                    .child(header),
            )
            .children(arrangement)
            .children(notes)
            .into_any_element()
    }

    fn playlist_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.playlist_rows.get(index) {
            Some(PlaylistRow::Cues(row)) => {
                let row = *row;
                let before_heading = matches!(
                    self.playlist_rows.get(index + 1),
                    Some(PlaylistRow::Marker(..))
                );
                self.cue_row(row, before_heading, cx)
            }
            Some(PlaylistRow::Marker(entry, index)) => {
                self.playlist_heading(entry.as_ref().clone(), *index, window, cx)
            }
            Some(PlaylistRow::EndSpace) => {
                let viewport = self
                    .viewport
                    .borrow()
                    .map_or(px(0.), |bounds| bounds.size.height);
                // A final header can align to the top even when it has few slides or none.
                div()
                    .h((viewport - cx.theme().metrics.header_height).max(px(0.)))
                    .into_any_element()
            }
            None => div().into_any_element(),
        }
    }
    fn open_form(
        &mut self,
        kind: FormKind,
        document: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let item = self.documents[document].item.clone();
        self.open_item_form(kind, &item, window, cx);
    }

    fn open_song_editor(&mut self, item_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .form
            .as_ref()
            .is_some_and(|form| matches!(form.kind, FormKind::Text(_)))
        {
            if let Some(task) = self.flush_reflow(cx) {
                task.detach();
            }
        }
        let item = self.runtime.content.read(cx).items.get(item_id).cloned();
        if let Some(item) = item {
            self.open_item_form(FormKind::Song, &item, window, cx);
        } else {
            self.nonce += 1;
            self.form = Some(Form {
                kind: FormKind::Song,
                item: String::new(),
                fields: Vec::new(),
                rows: Vec::new(),
                autosave: None,
                saving: false,
                error: String::new(),
                nonce: self.nonce,
                _observers: Vec::new(),
            });
            cx.notify();
        }
    }

    fn open_item_form(
        &mut self,
        kind: FormKind,
        item: &Item,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut fields = Vec::new();
        let mut rows = Vec::new();
        let mut observers = Vec::new();
        let reflow_slide = match &kind {
            FormKind::Text(id) => Some(id.clone()),
            _ => None,
        };
        match &kind {
            FormKind::Text(id) => {
                let slides = item.value["slides"].as_array().cloned().unwrap_or_default();
                (rows, observers) = Self::reflow_rows(&slides, window, cx);
                if let Some((_, state)) = rows.iter().find(|(row, _)| row == id) {
                    state.update(cx, |state, cx| state.focus(window, cx));
                }
                self.reflow_undo.clear();
                self.reflow_redo.clear();
                self.reflow_browser_item =
                    self.runtime.ui.read(cx).selected_library_item_id.clone();
            }
            FormKind::Shortcut(id) => {
                let value = item.value["slides"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|slide| slide["id"] == *id)
                    .and_then(|slide| slide["shortcut"].as_str())
                    .unwrap_or("");
                fields.push(("gpui.editor.slideShortcut", input_state(value, window, cx)));
            }
            FormKind::NewArrangement => {
                fields.push(("arrangements.name", input_state("", window, cx)))
            }
            FormKind::RenameArrangement(id) => fields.push((
                "arrangements.name",
                input_state(
                    item.arrangements
                        .iter()
                        .find(|arrangement| arrangement.id == *id)
                        .map(|arrangement| arrangement.name.clone())
                        .unwrap_or_default(),
                    window,
                    cx,
                ),
            )),
            FormKind::Song => {
                let draft = song_metadata::draft(item);
                for (key, value) in [
                    ("songMetadata.title", draft.title),
                    ("songMetadata.author", draft.author),
                    ("songMetadata.publisher", draft.publisher),
                    ("songMetadata.copyrightYear", draft.copyright_year),
                    ("songMetadata.songNumber", draft.song_number),
                ] {
                    let state = input_state(value, window, cx);
                    observers.push(cx.observe(&state, |_, _, cx| cx.notify()));
                    fields.push((key, state));
                }
            }
        }
        self.nonce += 1;
        self.form = Some(Form {
            kind,
            item: item.id.clone(),
            fields,
            rows,
            autosave: None,
            saving: false,
            error: String::new(),
            nonce: self.nonce,
            _observers: observers,
        });
        if let Some(slide) = reflow_slide {
            self.reflow_scroll_after_layout(&slide, cx);
        }
        cx.notify();
    }
    fn toggle_item_notes(
        &mut self,
        item: &str,
        entry: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_open = self
            .notes
            .as_ref()
            .is_some_and(|notes| notes.item == item && notes.entry == entry);
        self.close_notes(cx);
        if was_open {
            return;
        }
        let Some(document) = self.runtime.content.read(cx).items.get(item) else {
            return;
        };
        let value = document.value["operatorNotes"].as_str().unwrap_or("");
        let state = textarea_state(value.to_string(), window, cx);
        let events =
            cx.subscribe_in(
                &state,
                window,
                |view, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Blur => view.commit_notes(cx),
                    InputEvent::Change => cx.notify(),
                    _ => {}
                },
            );
        let item = item.to_string();
        state.update(cx, |state, cx| state.focus(window, cx));
        self.notes = Some(Notes {
            item,
            entry: entry.to_string(),
            state,
            _events: events,
        });
        cx.notify();
    }
    fn commit_notes(&mut self, cx: &mut Context<Self>) {
        let Some(notes) = &self.notes else { return };
        let text = notes.state.read(cx).value().trim().to_string();
        let current = self.runtime.content.read(cx).items.get(&notes.item);
        let Some(current) = current else { return };
        if current.value["operatorNotes"].as_str().unwrap_or("").trim() == text {
            return;
        }
        let item = notes.item.clone();
        let local = text.clone();
        actions::mutate_optimistic(
            &self.runtime,
            &item,
            json!({"type":"setOperatorNotes","operatorNotes":text}),
            move |value| value["operatorNotes"] = json!(local),
            cx,
        );
    }
    fn close_notes(&mut self, cx: &mut Context<Self>) {
        self.commit_notes(cx);
        self.notes = None;
        cx.notify();
    }
    fn notes_element(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notes = self.notes.as_ref()?;
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let lines = notes.state.read(cx).value().lines().count();
        Some(
            div()
                .debug_selector(|| "cue-operator-notes".to_string())
                .flex_none()
                .p(theme.metrics.space_xs)
                .border_b_1()
                .border_color(theme.colors.border)
                .child(
                    Textarea::new("cue-operator-notes", &notes.state)
                        .label(t.t("cue.operatorNotes"))
                        .rows(lines.clamp(2, 4) as f32),
                )
                .into_any_element(),
        )
    }
    fn save_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut().filter(|form| {
            !form.saving && (!matches!(form.kind, FormKind::Song) || !form.fields.is_empty())
        }) else {
            return;
        };
        let values = form
            .fields
            .iter()
            .map(|(_, state)| state.read(cx).value().to_string())
            .collect::<Vec<_>>();
        let (item, kind, nonce) = (form.item.clone(), form.kind.clone(), form.nonce);
        let runtime = self.runtime.clone();
        form.saving = true;
        form.error.clear();
        match kind {
            FormKind::Shortcut(id) => {
                let shortcut = values.first().map(String::as_str).unwrap_or("").trim();
                let task = actions::mutate_checked(
                    &runtime,
                    &item,
                    json!({"type":"setSlideShortcut","slideId":id,"shortcut":shortcut}),
                    cx,
                );
                cx.spawn(async move |owner, cx| {
                    let saved = task.await.is_some();
                    let _ = owner.update(cx, |view, cx| {
                        if let Some(form) = view.form.as_mut().filter(|form| form.nonce == nonce) {
                            if saved {
                                view.form = None;
                            } else {
                                form.saving = false;
                                form.error = runtime.status.read(cx).message.clone();
                            }
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            FormKind::NewArrangement | FormKind::RenameArrangement(_) => {
                let name = values.first().map(String::as_str).unwrap_or("").trim();
                if name.is_empty() {
                    if let Some(form) = self.form.as_mut() {
                        form.saving = false;
                        form.error = cx.global::<Translator>().t("arrangements.nameRequired");
                    }
                    cx.notify();
                    return;
                }
                let submitted = match kind {
                    FormKind::NewArrangement => arrangements::create_arrangement(
                        &runtime, cx, &item, name,
                    )
                    .is_some_and(|(id, task)| {
                        task.detach();
                        self.select_created_arrangement(&item, id, cx);
                        true
                    }),
                    FormKind::RenameArrangement(ref id) => arrangements::rename_arrangement(
                        &runtime, cx, &item, id, name,
                    )
                    .is_some_and(|task| {
                        task.detach();
                        true
                    }),
                    _ => false,
                };
                if submitted {
                    self.form = None;
                } else if let Some(form) = self.form.as_mut() {
                    form.saving = false;
                }
            }
            FormKind::Song => {
                let draft = song_metadata::Draft {
                    title: values[0].clone(),
                    author: values[1].clone(),
                    publisher: values[2].clone(),
                    copyright_year: values[3].clone(),
                    song_number: values[4].clone(),
                };
                let saved = song_metadata::save(&runtime, cx, &item, &draft);
                let submitted = values.clone();
                cx.spawn(async move |view, cx| {
                    let result = saved.await;
                    view.update(cx, |view, cx| {
                        if view.form.as_ref().is_none_or(|form| form.nonce != nonce) {
                            return;
                        }
                        match result {
                            Ok(()) => {
                                if view.form.as_ref().is_some_and(|form| {
                                    form.fields
                                        .iter()
                                        .map(|(_, input)| input.read(cx).value().to_string())
                                        .collect::<Vec<_>>()
                                        == submitted
                                }) {
                                    view.form = None;
                                } else if let Some(form) = view.form.as_mut() {
                                    form.saving = false;
                                }
                                actions::refresh_live(&runtime, cx);
                            }
                            Err(error) => {
                                let t = *cx.global::<Translator>();
                                if let Some(form) = view.form.as_mut() {
                                    form.saving = false;
                                    form.error = if error.starts_with("songMetadata.") {
                                        t.t(&error)
                                    } else {
                                        error
                                    };
                                }
                            }
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            FormKind::Text(_) => {
                let saved = self.flush_reflow(cx);
                cx.spawn(async move |view, cx| {
                    let saved = match saved {
                        Some(task) => task.await,
                        None => true,
                    };
                    view.update(cx, |view, cx| {
                        if view.form.as_ref().is_none_or(|form| form.nonce != nonce) {
                            return;
                        }
                        if saved {
                            view.form = None;
                        } else if let Some(form) = view.form.as_mut() {
                            form.saving = false;
                            form.error = runtime.status.read(cx).message.clone();
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
        }
        cx.notify();
    }
    fn reflow_rows(
        slides: &[Value],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Vec<(String, Entity<TextareaState>)>, Vec<Subscription>) {
        let (mut rows, mut events) = (Vec::new(), Vec::new());
        for slide in slides {
            let id = slide["id"].as_str().unwrap_or("").to_string();
            let state = growing_textarea_state(
                slide["content"].as_str().unwrap_or("").to_string(),
                1,
                REFLOW_MAX_ROWS,
                window,
                cx,
            );
            let row = id.clone();
            events.push(cx.subscribe_in(
                &state,
                window,
                move |view, state, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => view.reflow_changed(cx),
                    InputEvent::Focus => view.select_reflow_row(row.clone(), false, window, cx),
                    InputEvent::Blur => cx.notify(),
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => view.reflow_enter_split(&row, state, window, cx),
                    _ => {}
                },
            ));
            rows.push((id, state));
        }
        (rows, events)
    }
    /// The stored slides of the Reflow item with the text of every row applied.
    fn reflow_overlay(&self, cx: &App) -> Vec<Value> {
        let Some(form) = self.form.as_ref() else {
            return Vec::new();
        };
        let mut slides = self
            .runtime
            .content
            .read(cx)
            .items
            .get(&form.item)
            .and_then(|item| item.value["slides"].as_array().cloned())
            .unwrap_or_default();
        for slide in &mut slides {
            if let Some((_, state)) = form.rows.iter().find(|(id, _)| slide["id"] == *id) {
                slide["content"] = json!(state.read(cx).value().to_string());
            }
        }
        slides
    }
    fn push_reflow_history(&mut self, slides: Vec<Value>) {
        self.reflow_undo.push(Value::Array(slides));
        if self.reflow_undo.len() > REFLOW_HISTORY {
            self.reflow_undo.remove(0);
        }
        self.reflow_redo.clear();
    }
    /// The Tauri editor saved 700 ms after the last keystroke, and so does this one.
    fn reflow_changed(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self
            .form
            .as_mut()
            .filter(|form| matches!(form.kind, FormKind::Text(_)))
        else {
            return;
        };
        form.autosave = Some(cx.spawn(async move |view, cx| {
            cx.background_executor().timer(REFLOW_AUTOSAVE).await;
            let _ = view.update(cx, |view, cx| {
                if let Some(task) = view.flush_reflow(cx) {
                    task.detach();
                }
            });
        }));
        cx.notify();
    }
    /// Saves every row whose text differs from the stored slide as one undoable step; `None` when nothing changed.
    fn flush_reflow(&mut self, cx: &mut Context<Self>) -> Option<gpui::Task<bool>> {
        let form = self.form.as_mut()?;
        form.autosave = None;
        if !matches!(form.kind, FormKind::Text(_)) {
            return None;
        }
        let item = form.item.clone();
        let stored = self
            .runtime
            .content
            .read(cx)
            .items
            .get(&item)
            .and_then(|item| item.value["slides"].as_array().cloned())
            .unwrap_or_default();
        let changes = form
            .rows
            .iter()
            .filter_map(|(id, state)| {
                let value = state.read(cx).value();
                let slide = stored.iter().find(|slide| slide["id"] == *id)?;
                (slide["content"].as_str().unwrap_or("") != value.as_ref())
                    .then(|| (id.clone(), value.to_string()))
            })
            .collect::<Vec<_>>();
        if changes.is_empty() {
            return None;
        }
        let local = stored
            .iter()
            .map(
                |slide| match changes.iter().find(|(id, _)| slide["id"] == *id) {
                    Some((_, content)) => orbit_host::authoring::with_raw_content(slide, content),
                    None => Ok(slide.clone()),
                },
            )
            .collect::<Result<Vec<_>, _>>();
        let local = match local {
            Ok(local) => local,
            Err(error) => {
                actions::error(&self.runtime, cx, error);
                return None;
            }
        };
        let operation = json!({"type":"setSlideContents","changes":changes.iter().map(|(id, content)| json!({"slideId":id,"content":content})).collect::<Vec<_>>()});
        self.push_reflow_history(stored);
        Some(actions::mutate_optimistic_checked(
            &self.runtime,
            &item,
            operation,
            move |value| value["slides"] = Value::Array(local),
            cx,
        ))
    }
    /// A structural change (split or merge) of the Reflow item, after the pending text edits were saved; the
    /// form is rebuilt around the slide the host selected.
    fn reflow_run(&mut self, operation: Value, cursor: Option<usize>, cx: &mut Context<Self>) {
        let merge = operation["type"] == "mergeSlides";
        if self.form.as_ref().is_none_or(|form| form.saving) {
            return;
        }
        if let Some(task) = self.flush_reflow(cx) {
            task.detach();
        }
        let snapshot = self.reflow_overlay(cx);
        self.push_reflow_history(snapshot);
        let Some(form) = self.form.as_mut() else {
            return;
        };
        form.saving = true;
        form.error.clear();
        let (item, nonce) = (form.item.clone(), form.nonce);
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let answer = runtime
                .persist(
                    cx,
                    Persist::new(
                        Kind::Library,
                        json!({"type":"mutateItem","itemId":item,"operation":operation}),
                        |_, _| {},
                    ),
                )
                .await;
            let _ = view.update(cx, |view, cx| {
                if view.form.as_ref().is_none_or(|f| f.nonce != nonce) {
                    return;
                }
                let selected = answer
                    .as_ref()
                    .and_then(|answer| answer["selectedSlideId"].as_str())
                    .map(str::to_string);
                if let Some(selected) = selected {
                    view.form = None;
                    view.reflow_cursor = cursor;
                    view.pending_reflow = Some((item, selected));
                } else {
                    view.reflow_undo.pop();
                    let message = if answer.is_some() {
                        Runtime::translator(cx).t("reflow.invalidResponse")
                    } else if merge {
                        Runtime::translator(cx).t("slideEditor.mergeConflict")
                    } else {
                        runtime.status.read(cx).message.clone()
                    };
                    if let Some(form) = view.form.as_mut() {
                        form.saving = false;
                        form.error = message;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn selected_reflow_row(&self) -> Option<(String, Entity<TextareaState>)> {
        let form = self.form.as_ref()?;
        let FormKind::Text(id) = &form.kind else {
            return None;
        };
        form.rows.iter().find(|(row, _)| row == id).cloned()
    }
    /// Splits the slide at the cursor of the selected row.
    fn reflow_split(&mut self, cx: &mut Context<Self>) {
        let Some((id, state)) = self.selected_reflow_row() else {
            return;
        };
        let text = state.read(cx);
        let (value, cursor) = (text.value().to_string(), text.cursor());
        self.reflow_split_at(&id, &value, cursor, cx);
    }
    fn reflow_split_at(&mut self, id: &str, value: &str, cursor: usize, cx: &mut Context<Self>) {
        let mut cursor = cursor.min(value.len());
        while !value.is_char_boundary(cursor) {
            cursor -= 1;
        }
        let operation = json!({"type":"splitSlide","slideId":id,"beforeContent":&value[..cursor],"afterContent":trim_blank_lines(&value[cursor..])});
        self.reflow_run(operation, None, cx);
    }
    /// Ctrl+Enter: the textarea already inserted a line break at the cursor; the slide splits there instead.
    fn reflow_enter_split(
        &mut self,
        id: &str,
        state: &Entity<TextareaState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = state.read(cx);
        let value = text.value().to_string();
        let cursor = text.cursor().min(value.len());
        if !value.is_char_boundary(cursor) || !value[..cursor].ends_with('\n') {
            return;
        }
        let cursor = cursor - 1;
        let clean = format!("{}{}", &value[..cursor], &value[cursor + 1..]);
        state.update(cx, |state, cx| state.set_value(clean.clone(), window, cx));
        self.reflow_split_at(id, &clean, cursor, cx);
    }
    /// Merges the selected slide with its neighbour; `false` when there is none.
    fn reflow_merge(&mut self, direction: isize, cx: &mut Context<Self>) -> bool {
        let Some(form) = self.form.as_ref().filter(|form| !form.saving) else {
            return false;
        };
        let FormKind::Text(id) = &form.kind else {
            return false;
        };
        let slides = self.reflow_overlay(cx);
        let Some(index) = slides.iter().position(|slide| slide["id"] == *id) else {
            return false;
        };
        let Some(other) = index
            .checked_add_signed(direction)
            .filter(|other| *other < slides.len())
        else {
            return false;
        };
        let (first, second) = (index.min(other), index.max(other));
        let group = model::effective_group(&slides, first);
        if group.is_empty() || group != model::effective_group(&slides, second) {
            let message = cx.global::<Translator>().t("slideEditor.mergeConflict");
            if let Some(form) = self.form.as_mut() {
                form.error = message;
            }
            cx.notify();
            return true;
        }
        let text = |slide: &Value| slide["content"].as_str().unwrap_or("").to_string();
        let (before, after) = (text(&slides[first]), text(&slides[second]));
        let operation = json!({"type":"mergeSlides","keepSlideId":slides[first]["id"],"removeSlideId":slides[second]["id"],"content":join_content(&before, &after)});
        self.reflow_run(operation, Some(before.trim_end().len()), cx);
        true
    }
    /// Backspace at the start or Delete at the end of a row's text (or in an empty one) merges with the neighbour.
    fn reflow_edge_delete(&mut self, backward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.form.as_ref().and_then(|form| {
            form.rows
                .iter()
                .find(|(_, state)| state.read(cx).focus_handle(cx).is_focused(window))
                .map(|(_, state)| state.clone())
        }) else {
            return;
        };
        let text = state.read(cx);
        let (value, cursor) = (text.value(), text.cursor());
        let at_edge =
            value.is_empty() || (backward && cursor == 0) || (!backward && cursor == value.len());
        if text.selected_range().is_empty()
            && at_edge
            && self.reflow_merge(if backward { -1 } else { 1 }, cx)
        {
            cx.stop_propagation();
        }
    }
    fn reflow_history(&mut self, undo: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.as_ref().is_none_or(|form| form.saving) {
            return;
        }
        if let Some(task) = self.flush_reflow(cx) {
            task.detach();
        }
        let Some(Value::Array(target)) = (if undo {
            self.reflow_undo.pop()
        } else {
            self.reflow_redo.pop()
        }) else {
            return;
        };
        let Some(form) = self.form.as_ref() else {
            return;
        };
        let (item, selected) = (form.item.clone(), form.kind.clone());
        let current = Value::Array(self.reflow_overlay(cx));
        if undo {
            self.reflow_redo.push(current);
        } else {
            self.reflow_undo.push(current);
        }
        let title = self
            .runtime
            .content
            .read(cx)
            .items
            .get(&item)
            .map(|item| item.value["title"].clone())
            .unwrap_or(Value::Null);
        let selected = match selected {
            FormKind::Text(id) if target.iter().any(|slide| slide["id"] == id) => id,
            _ => target
                .first()
                .and_then(|slide| slide["id"].as_str())
                .unwrap_or("")
                .to_string(),
        };
        let (rows, events) = Self::reflow_rows(&target, window, cx);
        if let Some((_, state)) = rows.iter().find(|(id, _)| *id == selected) {
            state.update(cx, |state, cx| state.focus(window, cx));
        }
        if let Some(form) = self.form.as_mut() {
            form.kind = FormKind::Text(selected.clone());
            form.rows = rows;
            form._observers = events;
        }
        let local = target.clone();
        actions::mutate_optimistic(
            &self.runtime,
            &item,
            json!({"type":"setSlideDocument","title":title,"slides":target}),
            move |value| value["slides"] = Value::Array(local),
            cx,
        );
        self.reflow_scroll_to(&selected, cx);
        cx.notify();
    }
    /// Opens the selected slide in the slide editor.
    pub fn reflow_edit(&mut self, cx: &mut Context<Self>) {
        let Some((id, _)) = self.selected_reflow_row() else {
            return;
        };
        if self.form.as_ref().is_none_or(|form| form.saving) {
            return;
        }
        if let Some(task) = self.flush_reflow(cx) {
            task.detach();
        }
        let Some(form) = self.form.take() else {
            return;
        };
        let item = form.item;
        cx.emit(EditSlide { item, slide: id });
        cx.notify();
    }
    /// Scrolls the left column to the slide's row once the rows have been laid out.
    fn reflow_scroll_after_layout(&self, slide: &str, cx: &mut Context<Self>) {
        let slide = slide.to_string();
        cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            let _ = view.update(cx, |view, cx| {
                view.reflow_scroll_to(&slide, cx);
                cx.notify();
            });
        })
        .detach();
    }
    /// Scrolls the left column to the slide's row.
    fn reflow_scroll_to(&self, slide: &str, cx: &App) {
        let Some(form) = self.form.as_ref() else {
            return;
        };
        let slides = self.runtime.content.read(cx).items.get(&form.item);
        if let Some(index) = slides
            .and_then(|item| item.value["slides"].as_array())
            .and_then(|slides| slides.iter().position(|s| s["id"] == slide))
        {
            self.reflow_scroll.scroll_to_item(index);
            self.reflow_preview_scroll.scroll_to_item(index);
        }
    }
    fn select_reflow_row(
        &mut self,
        id: String,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.form.as_mut().filter(|form| !form.saving) else {
            return;
        };
        let FormKind::Text(current) = &form.kind else {
            return;
        };
        let Some(state) = form
            .rows
            .iter()
            .find(|(row, _)| *row == id)
            .map(|(_, state)| state.clone())
        else {
            return;
        };
        let changed = *current != id;
        if focus {
            state.update(cx, |state, cx| state.focus(window, cx));
        }
        if changed {
            form.kind = FormKind::Text(id.clone());
            self.reflow_scroll_to(&id, cx);
        }
        cx.notify();
    }
    /// The Reflow editor's left column: every slide of the presentation with its group color and name on the left
    /// and its text, editable in place, on the right.
    fn reflow_list(&self, form: &Form, slides: &[Value], cx: &Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let text_size = theme.metrics.font_base * self.reflow_font.read(cx).value().end();
        let mut list = div()
            .id("reflow-list")
            .flex()
            .flex_col()
            .gap(theme.metrics.space_2xs)
            .p(theme.metrics.space_xs)
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .track_scroll(&self.reflow_scroll)
            .capture_action(cx.listener(|view, _: &Backspace, window, cx| {
                view.reflow_edge_delete(true, window, cx)
            }))
            .capture_action(cx.listener(|view, _: &Delete, window, cx| {
                view.reflow_edge_delete(false, window, cx)
            }));
        for (index, slide) in slides.iter().enumerate() {
            let id = slide["id"].as_str().unwrap_or("").to_string();
            let (name, color) = self.reflow_group(slides, index, cx);
            let weak = cx.weak_entity();
            let target = id.clone();
            let number = (index + 1).to_string();
            let body = match form.rows.iter().find(|(row, _)| *row == id) {
                Some((_, state)) => {
                    Textarea::new(SharedString::from(format!("reflow-text-{index}")), state)
                        .disabled(form.saving)
                        .label(t.format("cue.slideText", &[("number", &number)]))
                        .text_size(text_size)
                        .grow()
                        .into_any_element()
                }
                None => div()
                    .px(theme.metrics.field_padding_x)
                    .py(theme.metrics.space_xs)
                    .text_size(text_size)
                    .whitespace_normal()
                    .child(slide["content"].as_str().unwrap_or("").to_string())
                    .into_any_element(),
            };
            list = list.child(
                div()
                    .relative()
                    .flex()
                    .items_stretch()
                    .border_l(theme.metrics.space_xs)
                    .border_color(color)
                    .bg(theme.colors.panel_bg_soft)
                    .child(
                        div()
                            .id(SharedString::from(format!("reflow-select-{index}")))
                            .aria_label(t.format("cue.selectSlide", &[("number", &number)]))
                            .flex_none()
                            .w(theme.metrics.control_height * 2.)
                            .p(theme.metrics.space_xs)
                            .bg(theme.colors.panel_bg_strong)
                            .cursor_pointer()
                            .text_size(theme.metrics.font_sm)
                            .text_color(theme.colors.text_strong)
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(format!("{number}.")),
                            )
                            .child(div().truncate().child(name))
                            .on_click(move |_, window, cx| {
                                let target = target.clone();
                                weak.update(cx, |view, cx| {
                                    view.select_reflow_row(target, true, window, cx)
                                })
                                .ok();
                            }),
                    )
                    .child(
                        div()
                            .relative()
                            .debug_selector(move || format!("reflow-text-row-{index}"))
                            .flex_1()
                            .min_w_0()
                            .py(theme.metrics.space_xs)
                            .bg(theme.colors.surface_bg)
                            .child(body),
                    ),
            );
        }
        list.into_any_element()
    }

    fn reflow_group(&self, slides: &[Value], index: usize, cx: &App) -> (String, gpui::Rgba) {
        let group = model::effective_group(slides, index);
        let setting = self
            .runtime
            .output
            .read(cx)
            .snapshot
            .as_ref()
            .map_or(&Value::Null, |snapshot| &snapshot["groupSettings"][&group]);
        let name = setting["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .unwrap_or(&group);
        let color = setting["color"]
            .as_str()
            .and_then(parse_hex_color)
            .unwrap_or(cx.theme().colors.cue_default);
        (name.to_string(), color)
    }

    fn reflow_previews(
        &self,
        form: &Form,
        slides: &[Value],
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let columns = 5 - self.reflow_zoom.read(cx).value().end() as u16;
        let focused = form
            .rows
            .iter()
            .find(|(_, state)| state.read(cx).focus_handle(cx).is_focused(window))
            .map(|(id, _)| id);
        let mut grid = div()
            .id("reflow-preview")
            .debug_selector(|| "cue-list-viewport".into())
            .grid()
            .grid_cols(columns)
            .content_start()
            .items_start()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .border_l_1()
            .border_color(theme.colors.border)
            .gap(theme.metrics.panel_padding)
            .p(theme.metrics.field_padding_x)
            .overflow_y_scroll()
            .track_scroll(&self.reflow_preview_scroll);
        for (index, slide) in slides.iter().enumerate() {
            let id = slide["id"].as_str().unwrap_or_default().to_string();
            let number = (index + 1).to_string();
            let (group, color) = self.reflow_group(slides, index, cx);
            let highlighted = focused == Some(&id);
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("reflow-preview-{id}")))
                    .debug_selector(move || format!("cue-thumbnail-{index}"))
                    .role(gpui::Role::Button)
                    .aria_label(t.format("cue.selectSlide", &[("number", &number)]))
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .rounded(theme.metrics.radius_xs)
                    .border_1()
                    .border_color(theme.colors.border_soft)
                    .bg(theme.colors.preview_matte_bg)
                    .cursor_pointer()
                    .relative()
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .aspect_ratio(16. / 9.)
                            .overflow_hidden()
                            .child(
                                div()
                                    .debug_selector(move || format!("cue-copy-{index}"))
                                    .absolute()
                                    .inset_0()
                                    .overflow_hidden()
                                    .p(theme.metrics.space_sm)
                                    .text_size(theme.metrics.font_base)
                                    .line_height(gpui::relative(1.18))
                                    .child(slide["content"].as_str().unwrap_or("").to_string()),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(move || format!("cue-footer-{index}"))
                            .h(theme.metrics.row_height)
                            .flex()
                            .items_center()
                            .gap(theme.metrics.space_snug)
                            .px(theme.metrics.space_snug)
                            .bg(color)
                            .text_size(theme.metrics.font_sm)
                            .child(number)
                            .child(div().min_w_0().truncate().child(group)),
                    )
                    .when(highlighted, |tile| {
                        tile.child(
                            div()
                                .debug_selector(move || format!("reflow-focus-{index}"))
                                .absolute()
                                .inset_0()
                                .border_2()
                                .border_color(theme.colors.focus_ring),
                        )
                    })
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.select_reflow_row(id.clone(), true, window, cx);
                    })),
            );
        }
        grid.into_any_element()
    }

    fn reflow_footer(&self, form: &Form, cx: &Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let slider =
            |id: &'static str, state: &Entity<orbit_ui::controls::SliderState>, label: &str| {
                div()
                    .debug_selector(move || id.into())
                    .w(theme.metrics.control_height * 4.)
                    .child(orbit_ui::controls::RangeInput::new(state).label(t.t(label)))
            };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.metrics.space_xs)
            .p(theme.metrics.space_xs)
            .border_t_1()
            .border_color(theme.colors.border)
            .child(slider(
                "reflow-font-slider",
                &self.reflow_font,
                "cue.reflowTextSize",
            ))
            .child(
                ActionButton::new("reflow-split", t.t("cue.insertLineBreak"))
                    .compact()
                    .disabled(form.saving || form.rows.is_empty())
                    .on_click(cx.listener(|view, _, _, cx| view.reflow_split(cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.colors.danger)
                    .child(form.error.clone()),
            )
            .child(slider(
                "reflow-zoom-slider",
                &self.reflow_zoom,
                "cue.reflowPreviewSize",
            ))
            .into_any_element()
    }
    fn form_element(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Some(form) = self.form.as_ref() else {
            return self
                .slide_transition_element(cx)
                .or_else(|| self.arrangement_manager_element(cx));
        };
        if matches!(form.kind, FormKind::Text(_)) {
            return None;
        }
        let t = *cx.global::<Translator>();
        let weak = cx.weak_entity();
        let title = t.t(match form.kind {
            FormKind::Text(_) => unreachable!("the text form is the inline reflow editor"),
            FormKind::Shortcut(_) => "gpui.editor.slideShortcut",
            FormKind::NewArrangement => "arrangements.add",
            FormKind::RenameArrangement(_) => "arrangements.name",
            FormKind::Song => "toolbar.editor.ccli",
        });
        let mut dialog = Dialog::new("cue-form", title, t.t("common.close"), move |_, _, cx| {
            weak.update(cx, |view, cx| {
                view.form = None;
                cx.notify();
            })
            .ok();
        });
        if matches!(
            form.kind,
            FormKind::NewArrangement | FormKind::RenameArrangement(_) | FormKind::Shortcut(_)
        ) {
            if let Some((_, input)) = form.fields.first() {
                dialog = dialog.initial_focus(input.read(cx).focus_handle(cx));
            }
        }
        if matches!(form.kind, FormKind::Song) {
            let mut options = vec![SelectOption::new(String::new(), t.t("songMetadata.choose"))];
            options.extend(
                self.runtime
                    .content
                    .read(cx)
                    .items
                    .iter()
                    .map(|item| SelectOption::new(item.id.clone(), item.title.clone())),
            );
            let weak = cx.weak_entity();
            dialog = dialog.child(Field::new(
                t.t("songMetadata.presentation"),
                Select::new("ccli-presentation", options, form.item.clone())
                    .disabled(form.saving)
                    .on_change(move |item, window, cx| {
                        weak.update(cx, |view, cx| view.open_song_editor(&item, window, cx))
                            .ok();
                    }),
            ));
        }
        for (index, (label, state)) in form.fields.iter().enumerate() {
            dialog = dialog.child(Field::new(
                t.t(label),
                TextInput::new(SharedString::from(format!("cue-field-{index}")), state)
                    .disabled(form.saving && !matches!(form.kind, FormKind::Song)),
            ));
        }
        if matches!(form.kind, FormKind::Song) && !form.fields.is_empty() {
            let values = form
                .fields
                .iter()
                .map(|(_, state)| state.read(cx).value().to_string())
                .collect::<Vec<_>>();
            let draft = song_metadata::Draft {
                title: values[0].clone(),
                author: values[1].clone(),
                publisher: values[2].clone(),
                copyright_year: values[3].clone(),
                song_number: values[4].clone(),
            };
            // An invalid number shows no preview; saving reports it.
            let preview = self
                .runtime
                .content
                .read(cx)
                .items
                .get(&form.item)
                .and_then(|item| song_metadata::apply(&item.value, &draft).ok())
                .map(|next| song_metadata::format(&next["ccli"]))
                .unwrap_or_default();
            dialog = dialog.child(Field::new(
                t.t("songMetadata.preview"),
                div().whitespace_normal().child(preview),
            ));
        }
        if !form.error.is_empty() {
            dialog = dialog.child(
                div()
                    .text_color(cx.theme().colors.danger)
                    .child(form.error.clone()),
            );
        }
        let weak = cx.weak_entity();
        Some(
            dialog
                .footer(
                    ActionButton::new("cue-form-save", t.t("common.done"))
                        .disabled(form.saving || form.fields.is_empty())
                        .on_click(move |_, _, cx| {
                            weak.update(cx, |view, cx| view.save_form(cx)).ok();
                        }),
                )
                .into_any_element(),
        )
    }
    fn menu_element(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let document = self.documents.get(menu.document)?.clone();
        let cue = document.cues.get(menu.index)?.clone();
        let selected = self.selected_for(&document, cue["id"].as_str()?);
        let mut selected_cues = document.cues.iter().filter(|cue| {
            cue["id"]
                .as_str()
                .is_some_and(|id| selected.iter().any(|slide| slide == id))
        });
        let (document_index, index, position) = (menu.document, menu.index, menu.position);
        let t = *cx.global::<Translator>();
        let action = |key: &str, name: &'static str| {
            let view = cx.weak_entity();
            MenuItem::new(t.t(key))
                .disabled(
                    (name == "paste" && self.clipboard.is_empty())
                        || (name == "paste-style" && self.text_style_clipboard.is_none())
                        || (matches!(
                            name,
                            "edit-slide" | "shortcut" | "transitions" | "completion" | "copy-style"
                        ) && selected.len() > 1),
                )
                .on_click(move |_, window, cx| {
                    view.update(cx, |view, cx| {
                        view.context_action(document_index, index, name, window, cx)
                    })
                    .ok();
                })
                .into()
        };
        let mut entries: Vec<MenuEntry> = vec![
            action("cue.menu.editSlide", "edit-slide"),
            MenuEntry::Separator,
        ];
        if selected_cues.clone().any(|cue| cue["disabled"] == true) {
            entries.push(action("cue.menu.enable", "enable"));
        }
        if selected_cues.any(|cue| cue["disabled"] != true) {
            entries.push(action("cue.menu.disable", "disable"));
        }
        entries.push(MenuEntry::Separator);
        let groups = self
            .runtime
            .output
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot["groupSettings"].as_object())
            .cloned()
            .unwrap_or_default();
        let mut group_entries = Vec::new();
        let mut groups = groups
            .into_iter()
            .map(|(id, setting)| {
                (
                    id.clone(),
                    setting["name"].as_str().unwrap_or(&id).to_string(),
                )
            })
            .collect::<Vec<_>>();
        groups.insert(0, (String::new(), t.t("cue.menu.noGroup")));
        for (group, label) in groups {
            let runtime = self.runtime.clone();
            let item = document.item.id.clone();
            let ids = selected.clone();
            group_entries.push(MenuItem::new(label).checked(model::group_id(&cue)==group).on_click(move|_,_,cx|actions::mutate(&runtime,&item,json!({"type":"setSlidesGroup","slideIds":ids,"groupId":if group.is_empty(){Value::Null}else{json!(group)}}),cx)).into());
        }

        let themes = self.runtime.content.read(cx).themes.clone();
        let mut theme_entries = Vec::new();
        for theme in themes {
            let mut layouts = Vec::new();
            for (slide_id, name) in theme.slides {
                let runtime = self.runtime.clone();
                let item = document.item.id.clone();
                let selected = selected.clone();
                let assignment = format!("{}::{slide_id}", theme.id);
                layouts.push(MenuItem::new(name).checked(cue["presentationThemeId"] == assignment)
                    .on_click(move |_, _, cx| actions::mutate(&runtime, &item, json!({"type":"setSlidesTheme","slideIds":selected,"assignment":assignment}), cx)).into());
            }
            theme_entries.push(MenuEntry::Submenu {
                label: theme.title.into(),
                disabled: layouts.is_empty(),
                entries: layouts,
            });
        }
        entries.push(MenuEntry::Submenu {
            label: t.t("cue.menu.themes").into(),
            disabled: theme_entries.is_empty(),
            entries: theme_entries,
        });
        entries.extend([
            action("cue.menu.transitions", "transitions"),
            action("cue.menu.shortcut", "shortcut"),
            action("cue.menu.completion", "completion"),
            MenuEntry::Separator,
        ]);
        entries.extend(self.slide_action_entries(&document.item, &cue, cx));
        entries.push(MenuEntry::Separator);
        entries.push(MenuEntry::Submenu {
            label: t.t("cue.menu.group").into(),
            disabled: false,
            entries: group_entries,
        });
        let labels = crate::settings::slide_labels(
            &self.runtime.output.read(cx).settings["generalSettings"],
        );
        let assign = |name_color: Option<(String, String)>| {
            let (runtime, item) = (self.runtime.clone(), document.item.id.clone());
            let slides = selected.clone();
            move |_: (), _: &mut gpui::Window, cx: &mut gpui::App| {
                actions::set_slide_labels(&runtime, cx, &item, slides.clone(), name_color.clone());
            }
        };
        let mut label_entries: Vec<MenuEntry> = vec![MenuItem::new(t.t("cue.menu.noLabel"))
            .on_click(assign(None))
            .into()];
        if !labels.is_empty() {
            label_entries.push(MenuEntry::Separator);
        }
        for label in labels {
            label_entries.push(
                MenuItem::new(label.name.clone())
                    .checked(cue["label"].as_str() == Some(label.name.as_str()))
                    .on_click(assign(Some((label.name, label.color))))
                    .into(),
            );
        }
        entries.push(MenuEntry::Submenu {
            label: t.t("cue.menu.label").into(),
            disabled: false,
            entries: label_entries,
        });
        entries.extend([
            MenuEntry::Separator,
            action("cue.menu.copyTextStyle", "copy-style"),
            action("cue.menu.pasteTextStyle", "paste-style"),
            MenuEntry::Separator,
            action("common.cut", "cut"),
            action("common.copy", "copy"),
            action("common.paste", "paste"),
            action("common.duplicate", "duplicate"),
            action("common.delete", "delete"),
        ]);
        let weak = cx.weak_entity();
        Some(
            ContextMenu::new("cue-context", position, entries, move |_, _, cx| {
                weak.update(cx, |view, cx| {
                    view.menu = None;
                    cx.notify();
                })
                .ok();
            })
            .into_any_element(),
        )
    }
    fn arrangement_document(&self, cx: &App) -> Option<model::DocumentSeed> {
        let content = self.runtime.content.read(cx);
        let document = if self.continuous {
            let ui = self.runtime.ui.read(cx);
            let entry = content
                .playlist(&ui.selected_playlist_id)?
                .entries
                .get(ui.selected_documents.first()?.index?)?;
            self.documents
                .iter()
                .find(|document| document.entry == entry.id)?
        } else {
            self.documents.first()?
        };
        model::DocumentSeed::current_arrangement(document, content)
    }

    fn select_arrangement(&mut self, id: &str, cx: &mut Context<Self>) {
        self.arr_menu = None;
        cx.notify();
        let Some(document) = self.arrangement_document(cx) else {
            return;
        };
        let entry = (!document.entry.is_empty())
            .then(|| (document.playlist.clone(), document.entry.clone()));
        if let Some(task) = arrangements::select_arrangement(
            &self.runtime,
            cx,
            &document.item.id,
            id,
            entry
                .as_ref()
                .map(|(playlist, entry)| (playlist.as_str(), entry.as_str())),
        ) {
            task.detach();
        }
    }

    fn edit_arrangement(&mut self, id: &str, edit: SectionEdit, cx: &mut Context<Self>) {
        let Some(document) = self.arrangement_document(cx) else {
            return;
        };
        if id.is_empty() {
            return;
        }
        if let Some(task) =
            arrangements::edit_sections(&self.runtime, cx, &document.item.id, id, edit)
        {
            task.detach();
        }
    }

    /// The arrangement picker and the context menu of an order chip.
    fn arrangement_menu_element(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.arr_menu.as_ref()?;
        let document = self.arrangement_document(cx)?;
        let t = *cx.global::<Translator>();
        let id = document.arrangement.clone();
        let item = document.item.clone();
        let (position, entries): (Point<Pixels>, Vec<MenuEntry>) = match *menu {
            ArrangementMenu::Chip(position, index) => {
                let weak = cx.weak_entity();
                let entry =
                    MenuItem::new(t.t("arrangements.removeSection")).on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.arr_menu = None;
                            view.edit_arrangement(&id, SectionEdit::Remove(index), cx);
                            cx.notify();
                        })
                        .ok();
                    });
                (position, vec![entry.into()])
            }
            ArrangementMenu::Select(position) => {
                let choice = |label: String, target: String| -> MenuEntry {
                    let weak = cx.weak_entity();
                    MenuItem::new(label)
                        .checked(target == id)
                        .on_click(move |_, _, cx| {
                            weak.update(cx, |view, cx| view.select_arrangement(&target, cx))
                                .ok();
                        })
                        .into()
                };
                let mut entries = vec![choice(t.t("arrangements.master"), String::new())];
                entries.extend(
                    item.arrangements.iter().map(|arrangement| {
                        choice(arrangement.name.clone(), arrangement.id.clone())
                    }),
                );
                entries.push(MenuEntry::Separator);
                let weak = cx.weak_entity();
                entries.push(
                    MenuItem::new(t.t("arrangements.add"))
                        .on_click(move |_, window, cx| {
                            weak.update(cx, |view, cx| {
                                if let Some(document) = view.arrangement_document(cx) {
                                    view.open_item_form(
                                        FormKind::NewArrangement,
                                        &document.item,
                                        window,
                                        cx,
                                    );
                                }
                            })
                            .ok();
                        })
                        .into(),
                );
                let weak = cx.weak_entity();
                entries.push(
                    MenuItem::new(t.t("arrangements.edit"))
                        .on_click(move |_, _, cx| {
                            weak.update(cx, |view, cx| view.open_arrangement_manager(cx))
                                .ok();
                        })
                        .into(),
                );
                (position, entries)
            }
        };
        let weak = cx.weak_entity();
        Some(
            ContextMenu::new(
                "cue-arrangement-menu",
                position,
                entries,
                move |_, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.arr_menu = None;
                        cx.notify();
                    })
                    .ok();
                },
            )
            .into_any_element(),
        )
    }

    /// ProPresenter's arrangement area: the groups to drag from, the arrangement picker, and the order bar.
    fn arrangement_panel(&self, document: &Document, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.runtime
            .ui
            .read(cx)
            .arrangement_editor
            .as_ref()
            .filter(|editor| editor.item_id == document.item.id)?;
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let current =
            model::DocumentSeed::current_arrangement(document, self.runtime.content.read(cx))?;
        let item = current.item;
        let id = current.arrangement;
        let arrangement = item
            .arrangements
            .iter()
            .find(|arrangement| arrangement.id == id);
        let sections = arrangements::presentation_sections(&item);
        let settings = self
            .runtime
            .output
            .read(cx)
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot["groupSettings"].clone())
            .unwrap_or(Value::Null);
        let group = |section: &arrangements::Section| -> (SharedString, gpui::Rgba) {
            let setting = &settings[&section.name];
            let name = setting["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or(&section.name);
            let color = setting["color"]
                .as_str()
                .and_then(parse_hex_color)
                .unwrap_or(theme.colors.cue_default);
            (SharedString::from(name.to_string()), color)
        };
        let chip = |id: SharedString, name: SharedString, color: gpui::Rgba| {
            div()
                .id(id)
                .relative()
                .flex_none()
                .min_h(theme.metrics.row_height)
                .px(theme.metrics.space_sm)
                .py(theme.metrics.space_2xs)
                .flex()
                .items_center()
                .rounded(theme.metrics.radius_sm)
                .bg(color)
                .text_color(theme.colors.accent_text)
                .text_size(theme.metrics.font_sm)
                .line_height(theme.metrics.font_sm + theme.metrics.space_sm)
                .cursor_pointer()
                .child(name)
        };
        let label_width = theme.metrics.control_height * 4.;
        let editable = arrangement.is_some();

        let mut groups = div()
            .id("cue-arrangement-groups")
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme.metrics.space_xs)
            .flex_1()
            .min_w_0()
            .when(editable, |el| {
                el.on_drop::<SectionDrag>(cx.listener(|view, drag: &SectionDrag, _, cx| {
                    // Dragging a chip out of the bar onto the groups removes it.
                    if let (Some(from), Some(document)) = (drag.from, view.arrangement_document(cx))
                    {
                        let id = document.arrangement.clone();
                        view.edit_arrangement(&id, SectionEdit::Remove(from), cx);
                    }
                }))
            });
        for section in &sections {
            let (name, color) = group(section);
            let drag = SectionDrag {
                id: section.id.clone(),
                name: name.clone(),
                color,
                from: None,
            };
            let (section_id, arrangement_id) = (section.id.clone(), id.clone());
            groups = groups.child(
                chip(
                    SharedString::from(format!("cue-drag-section-{}", section.id)),
                    name,
                    color,
                )
                .on_drag(drag, |drag, _, _, cx| {
                    cx.new(|_| SectionDragPreview(drag.name.clone(), drag.color))
                })
                .when(editable, |el| {
                    el.on_click(cx.listener(move |view, _, _, cx| {
                        view.edit_arrangement(
                            &arrangement_id,
                            SectionEdit::Add(section_id.clone()),
                            cx,
                        )
                    }))
                }),
            );
        }

        let picker = {
            let weak = cx.weak_entity();
            let name = arrangement
                .map(|arrangement| arrangement.name.clone())
                .unwrap_or_else(|| t.t("arrangements.master"));
            let bounds = self.arr_picker.clone();
            let placed = bounds.clone();
            div().flex_none().w(label_width).child(
                div()
                    .relative()
                    .child(
                        ActionButton::new("cue-arrangement", name)
                            .variant(ButtonVariant::Ghost)
                            .compact()
                            .icon("chevron-down")
                            .on_click(move |_, _, cx| {
                                let position = bounds.get().bottom_left();
                                weak.update(cx, |view, cx| {
                                    view.arr_menu = Some(ArrangementMenu::Select(position));
                                    cx.notify();
                                })
                                .ok();
                            }),
                    )
                    .child(
                        gpui::canvas(move |area, _, _| placed.set(area), |_, _, _, _| {})
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                    ),
            )
        };

        let marker = self
            .arr_gap
            .filter(|_| cx.has_active_drag())
            .and_then(|gap| {
                let bar = (*self.arr_bar.borrow())?;
                let chips = self.arr_chips.borrow();
                let half = theme.metrics.space_xs / 2.;
                let (x, y, height) = match (chips.get(gap), chips.last()) {
                    (Some((_, bounds)), _) => {
                        (bounds.left() - half, bounds.top(), bounds.size.height)
                    }
                    (None, Some((_, bounds))) => {
                        (bounds.right() + half, bounds.top(), bounds.size.height)
                    }
                    (None, None) => (
                        bar.left() + theme.metrics.space_xs,
                        bar.top() + theme.metrics.space_xs,
                        theme.metrics.row_height,
                    ),
                };
                Some((x - bar.left(), y - bar.top(), height))
            });
        let chips = self.arr_chips.clone();
        let bar_bounds = self.arr_bar.clone();
        chips.borrow_mut().clear();
        let mut bar = div()
            .id("cue-arrangement-bar")
            .relative()
            .flex_1()
            .min_w_0()
            .min_h(theme.metrics.control_height)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme.metrics.space_xs)
            .p(theme.metrics.space_xs)
            .bg(theme.colors.field_bg)
            .border_1()
            .border_color(theme.colors.border)
            .child(
                gpui::canvas(
                    move |bounds, _, _| *bar_bounds.borrow_mut() = Some(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        match arrangement {
            None => {
                bar = bar.child(
                    div()
                        .text_color(theme.colors.text_muted)
                        .text_size(theme.metrics.font_sm)
                        .child(t.t("arrangements.masterHint")),
                );
            }
            Some(arrangement) => {
                for (index, section) in arrangement.section_ids.iter().enumerate() {
                    let (name, color) = sections
                        .iter()
                        .find(|candidate| candidate.id == *section)
                        .map(group)
                        .unwrap_or_else(|| {
                            (
                                SharedString::from(t.t("arrangements.missingSection")),
                                theme.colors.panel_bg_soft,
                            )
                        });
                    let drag = SectionDrag {
                        id: section.clone(),
                        name: name.clone(),
                        color,
                        from: Some(index),
                    };
                    let chips = chips.clone();
                    bar = bar.child(
                        chip(
                            SharedString::from(format!("cue-order-{index}")),
                            name,
                            color,
                        )
                        .gap(theme.metrics.space_xs)
                        .child(
                            IconButton::new(
                                format!("cue-arrangement-remove-{index}"),
                                "x",
                                t.t("arrangements.removeSection"),
                            )
                            .inline()
                            .on_click({
                                let id = id.clone();
                                cx.listener(move |view, _, _, cx| {
                                    cx.stop_propagation();
                                    view.edit_arrangement(&id, SectionEdit::Remove(index), cx);
                                })
                            }),
                        )
                        .on_drag(drag, |drag, _, _, cx| {
                            cx.new(|_| SectionDragPreview(drag.name.clone(), drag.color))
                        })
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                                view.arr_menu = Some(ArrangementMenu::Chip(event.position, index));
                                cx.notify();
                            }),
                        )
                        .child(
                            gpui::canvas(
                                move |bounds, _, _| chips.borrow_mut().push((index, bounds)),
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .size_full(),
                        ),
                    );
                }
                if let Some((x, y, height)) = marker {
                    bar = bar.child(
                        div()
                            .absolute()
                            .left(x - px(1.))
                            .top(y)
                            .w(px(2.))
                            .h(height)
                            .bg(theme.colors.selected),
                    );
                }
                bar = bar.when(editable, |el| {
                    el.on_drag_move::<SectionDrag>(cx.listener(
                        |view, event: &gpui::DragMoveEvent<SectionDrag>, _, cx| {
                            let from = event.drag(cx).from;
                            let gap = event
                                .bounds
                                .contains(&event.event.position)
                                .then(|| {
                                    let chips = view.arr_chips.borrow();
                                    if chips.is_empty() {
                                        Some(0)
                                    } else {
                                        crate::gap::gap_at(
                                            event.event.position,
                                            chips.iter().copied(),
                                            false,
                                        )
                                    }
                                })
                                .flatten()
                                .filter(|gap| arrangements::drop_edit(from, "", *gap).is_some());
                            if gap != view.arr_gap {
                                view.arr_gap = gap;
                                cx.notify();
                            }
                        },
                    ))
                    .on_drop::<SectionDrag>(cx.listener(
                        |view, drag: &SectionDrag, _, cx| {
                            let gap = view.arr_gap.take();
                            cx.notify();
                            let (Some(gap), Some(document)) = (gap, view.arrangement_document(cx))
                            else {
                                return;
                            };
                            if let Some(edit) = arrangements::drop_edit(drag.from, &drag.id, gap) {
                                let id = document.arrangement.clone();
                                view.edit_arrangement(&id, edit, cx);
                            }
                        },
                    ))
                });
            }
        }

        let label = |text: String| {
            div()
                .flex_none()
                .w(label_width)
                .px(theme.metrics.space_sm)
                .text_color(theme.colors.text_muted)
                .text_size(theme.metrics.font_sm)
                .child(text)
        };
        Some(
            div()
                .id("cue-arrangement-panel")
                .flex_none()
                .flex()
                .flex_col()
                .gap(theme.metrics.space_xs)
                .p(theme.metrics.space_xs)
                .border_b_1()
                .border_color(theme.colors.border)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(label(t.t("arrangements.groups")))
                        .child(groups),
                )
                .child(div().flex().items_start().child(picker).child(bar))
                .into_any_element(),
        )
    }
}
impl Focusable for CueView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for CueView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_view_options(cx);
        self.rebuild(cx);
        self.sync_reflow_item(window, cx);
        let request = self.runtime.ui.read(cx).header_editor.clone();
        if self
            .header_editor
            .as_ref()
            .map(|editor| &editor.read(cx).request)
            != request.as_ref()
        {
            self.header_editor = request.and_then(|request| {
                let entry = self
                    .runtime
                    .content
                    .read(cx)
                    .playlist(&request.playlist)?
                    .entries
                    .iter()
                    .find(|entry| entry.id == request.entry)?
                    .clone();
                let runtime = self.runtime.clone();
                Some(cx.new(|cx| headers::Editor::new(runtime, request, &entry, window, cx)))
            });
        }
        self.tiles.borrow_mut().clear();
        if let Some((item, slide)) = self.pending_reflow.take() {
            let slides = self
                .runtime
                .content
                .read(cx)
                .items
                .get(&item)
                .and_then(|item| item.value["slides"].as_array().cloned());
            if let Some(slides) = slides.filter(|slides| slides.iter().any(|s| s["id"] == slide)) {
                let (rows, observers) = Self::reflow_rows(&slides, window, cx);
                if let Some((_, state)) = rows.iter().find(|(id, _)| *id == slide) {
                    let cursor = self.reflow_cursor.take();
                    state.update(cx, |state, cx| {
                        state.focus(window, cx);
                        if let Some(cursor) = cursor {
                            state.set_selected_range(cursor..cursor, cx);
                        }
                    });
                }
                self.nonce += 1;
                self.form = Some(Form {
                    kind: FormKind::Text(slide.clone()),
                    item: item.clone(),
                    fields: Vec::new(),
                    rows,
                    autosave: None,
                    saving: false,
                    error: String::new(),
                    nonce: self.nonce,
                    _observers: observers,
                });
                self.reflow_scroll_after_layout(&slide, cx);
            }
        }
        if let Some(form) = self
            .form
            .as_ref()
            .filter(|form| matches!(form.kind, FormKind::Text(_)))
        {
            let slides = self.reflow_overlay(cx);
            return div()
                .size_full()
                .min_h_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .child(self.reflow_list(form, &slides, cx))
                        .child(self.reflow_previews(form, &slides, window, cx)),
                )
                .child(self.reflow_footer(form, cx))
                .into_any_element();
        }
        let theme = *cx.theme();
        let metrics = (theme.metrics.control_height, theme.metrics.font_sm);
        if self.list_metrics != metrics {
            self.list_metrics = metrics;
            self.scroll.remeasure_items(0..self.rows.len());
            self.playlist_scroll.reset(self.playlist_rows.len());
        }
        let live = self.runtime.output.read(cx).live.clone();
        let live_key = (
            live["liveItemId"].clone(),
            live["liveSlideId"].clone(),
            live["liveCueInstanceId"].clone(),
            live["announcements"]["selectionId"].clone(),
        );
        if live_key != self.live_key {
            let announced = live_key.3 != self.live_key.3 && !live_key.3.is_null();
            self.live_key = live_key;
            if let Some(row) = self.rows.iter().position(|(document, start, end)| {
                self.documents[*document].cues[*start..*end]
                    .iter()
                    .any(|cue| {
                        let document = &self.documents[*document];
                        if announced {
                            model::is_announcement_live(&live, document, cue)
                        } else {
                            model::is_presentation_live(&live, document, cue)
                        }
                    })
            }) {
                if self.continuous {
                    if let Some(index) = self.playlist_rows.iter().position(
                        |entry| matches!(entry, PlaylistRow::Cues(index) if *index == row),
                    ) {
                        self.playlist_scroll.scroll_to_reveal_item(index);
                    }
                } else {
                    self.reveal_cue_row(row);
                }
            }
        }
        self.reveal_playlist_selection(cx);
        let t = *cx.global::<Translator>();
        let active = (!self.continuous)
            .then(|| self.documents.first().cloned())
            .flatten();
        if self.notes.as_ref().is_some_and(|notes| {
            !self
                .documents
                .iter()
                .any(|document| document.item.id == notes.item)
        }) {
            self.close_notes(cx);
        }
        let marker = if self.continuous {
            None
        } else {
            let content = self.runtime.content.read(cx);
            let ui = self.runtime.ui.read(cx);
            model::selected_playlist_entry(content, ui)
                .filter(|(_, entry)| entry.kind != EntryKind::Presentation)
                .map(|(index, entry)| (index, entry.clone()))
        };
        let show_marker = marker.is_some();
        let mut header = PanelHeader::new(
            active
                .as_ref()
                .map(|document| {
                    document
                        .item
                        .arrangements
                        .iter()
                        .find(|arrangement| arrangement.id == document.arrangement)
                        .map_or_else(
                            || document.item.title.clone(),
                            |arrangement| {
                                format!("{} - [ {} ]", document.item.title, arrangement.name)
                            },
                        )
                })
                .unwrap_or_else(|| {
                    self.runtime
                        .content
                        .read(cx)
                        .playlist(&self.runtime.ui.read(cx).selected_playlist_id)
                        .filter(|_| self.continuous)
                        .map(|playlist| playlist.title.clone())
                        .unwrap_or_else(|| t.t("search.preview.empty"))
                }),
        );
        let header = if let Some((index, entry)) = marker {
            self.playlist_heading(entry, index, window, cx)
        } else {
            if let Some(document) = &active {
                header = self.presentation_header_actions(header, document, None, window, cx);
            }
            header.into_any_element()
        };
        let body = if self.rows.is_empty() && (!self.continuous || self.playlist_rows.is_empty()) {
            self.visible.clear();
            EmptyState::new(t.t(if self.loading {
                "status.workspace.loading"
            } else {
                "search.preview.empty"
            }))
            .into_any_element()
        } else if self.continuous {
            gpui::list(
                self.playlist_scroll.clone(),
                cx.processor(|view, index, window, cx| view.playlist_row(index, window, cx)),
            )
            .flex_1()
            .min_h_0()
            .into_any_element()
        } else {
            gpui::list(
                self.scroll.clone(),
                cx.processor(|view, row, _, cx| view.cue_row(row, false, cx)),
            )
            .flex_1()
            .min_h_0()
            .into_any_element()
        };
        let viewport = self.viewport.clone();
        let owner = cx.weak_entity();
        let body = div()
            .relative()
            .debug_selector(|| "cue-list-viewport".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(body)
            .child(
                gpui::canvas(
                    move |bounds, _, cx| {
                        let changed = viewport.borrow().is_none_or(|old| old.size != bounds.size);
                        *viewport.borrow_mut() = Some(bounds);
                        if changed {
                            owner
                                .update(cx, |view, cx| {
                                    view.playlist_scroll
                                        .remeasure_items(0..view.playlist_rows.len());
                                    cx.notify();
                                })
                                .ok();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        let weak = cx.weak_entity();
        let footer = div()
            .flex()
            .flex_wrap()
            .flex_none()
            .items_center()
            .gap(theme.metrics.space_xs)
            .p(theme.metrics.space_xs)
            .border_t_1()
            .border_color(theme.colors.border)
            .child(
                IconButton::new(
                    "cue-new-slide",
                    "plus",
                    t.t("presentation.command.newSlide"),
                )
                .disabled(active.is_none())
                .on_click({
                    let weak = cx.weak_entity();
                    move |_, window, cx| {
                        weak.update(cx, |view, cx| view.command("newSlide", window, cx))
                            .ok();
                    }
                }),
            )
            .child(self.transitions.clone())
            .child(div().flex_1())
            .child(
                SegmentedControl::new(
                    "cue-view-mode",
                    t.t("cue.view.easy"),
                    vec![
                        SegmentedOption::icon(Mode::Easy, "letter-t", t.t("cue.view.easy")),
                        SegmentedOption::icon(Mode::Grid, "layout-grid", t.t("cue.view.grid")),
                        SegmentedOption::icon(Mode::Table, "table", t.t("cue.view.table")),
                    ],
                    self.mode,
                )
                .on_change(move |mode, _, cx| {
                    weak.update(cx, |view, cx| view.set_view(mode.key(), cx))
                        .ok();
                }),
            )
            .child(self.view_options_button(window, cx))
            .child(
                IconButton::new("cue-continuous", "list", t.t("collections.menu.playlist"))
                    .selected(self.continuous)
                    .disabled(
                        self.runtime
                            .content
                            .read(cx)
                            .playlist(&self.runtime.ui.read(cx).selected_playlist_id)
                            .is_none(),
                    )
                    .on_click({
                        let weak = cx.weak_entity();
                        move |_, _, cx| {
                            weak.update(cx, |view, cx| {
                                view.toggle_continuous(cx);
                            })
                            .ok();
                        }
                    }),
            )
            .child(
                div()
                    .w(theme.metrics.control_height * 4.)
                    .child(orbit_ui::controls::RangeInput::new(&self.size).label(t.t("cue.size"))),
            );
        let menu = self.menu_element(cx);
        let notes = (!self.continuous && !show_marker)
            .then(|| self.notes_element(cx))
            .flatten();
        let arrangement_menu = self.arrangement_menu_element(cx);
        let arrangement = active
            .as_ref()
            .filter(|_| !show_marker)
            .and_then(|document| self.arrangement_panel(document, cx));
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .track_focus(&self.focus)
            .tab_index(0)
            .id("cue-workspace")
            .aria_label(t.t("shell.slideCues"))
            .on_key_down(cx.listener(Self::key_down))
            .capture_any_mouse_down(cx.listener(|view, event: &gpui::MouseDownEvent, _, cx| {
                let on_slide = view
                    .tiles
                    .borrow()
                    .values()
                    .any(|bounds| bounds.contains(&event.position));
                if !on_slide {
                    view.clear_slide_selection(cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                if view.menu.is_none() {
                    view.clear_slide_selection(cx);
                }
            }))
            .when(
                self.press.as_ref().is_some_and(|press| press.dragging),
                |el| el.cursor_grabbing(),
            )
            .on_mouse_move(cx.listener(|view, event: &gpui::MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    if view.press.take().is_some() {
                        cx.notify();
                    }
                    return;
                }
                let Some(press) = view.press.as_ref().filter(|press| {
                    press.generation == view.loaded_generation
                        && (press.dragging
                            || crate::gap::moved_far_enough(
                                press.start,
                                event.position,
                                cx.theme().metrics.space_xs,
                            ))
                }) else {
                    return;
                };
                let gap = view.gap_from(press.document, event.position);
                if !press.dragging || gap != press.gap {
                    if let Some(press) = view.press.as_mut() {
                        press.dragging = true;
                        press.gap = gap;
                    }
                    view.suppress_click = true;
                    cx.notify();
                }
                let Some(press) = view.press.as_ref().filter(|press| press.dragging) else {
                    return;
                };
                let Some(bounds) = *view.viewport.borrow() else {
                    return;
                };
                let zone = cx.theme().metrics.control_height;
                if view.continuous {
                    let top = view.playlist_scroll.logical_scroll_top().item_ix;
                    let index = if event.position.y < bounds.top() + zone {
                        top.saturating_sub(1)
                    } else if event.position.y > bounds.bottom() - zone {
                        (top + 1).min(view.playlist_rows.len().saturating_sub(1))
                    } else {
                        top
                    };
                    if index != top {
                        view.playlist_scroll.scroll_to(ListOffset {
                            item_ix: index,
                            offset_in_item: px(0.),
                        });
                        cx.notify();
                    }
                    return;
                }
                if event.position.y < bounds.top() + zone {
                    let top = view.scroll.logical_scroll_top();
                    view.scroll.scroll_to(ListOffset {
                        item_ix: top.item_ix.saturating_sub(1),
                        offset_in_item: px(0.),
                    });
                    cx.notify();
                } else if event.position.y > bounds.bottom() - zone {
                    let document = &view.documents[press.document];
                    let target = document.item.value["slides"]
                        .as_array()
                        .and_then(|slides| slides.get(press.gap?.saturating_sub(1)));
                    let row = view.rows.iter().position(|(doc, start, end)| {
                        *doc == press.document
                            && view.documents[*doc].cues[*start..*end]
                                .iter()
                                .any(|cue| target.is_some_and(|target| cue["id"] == target["id"]))
                    });
                    if let Some(row) = row {
                        view.reveal_cue_row((row + 1).min(view.rows.len() - 1));
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, event: &gpui::MouseUpEvent, _, cx| {
                    view.finish_drag(event.position, cx)
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, event: &gpui::MouseUpEvent, _, cx| {
                    view.finish_drag(event.position, cx)
                }),
            )
            .child(header)
            .children(arrangement)
            .children(notes)
            .child(body)
            .child(footer)
            .children(menu)
            .children(arrangement_menu)
            .into_any_element()
    }
}

fn trim_blank_lines(mut text: &str) -> &str {
    while let Some((line, rest)) = text.split_once('\n') {
        if !line.trim_matches([' ', '\t', '\r']).is_empty() {
            break;
        }
        text = rest;
    }
    text
}
fn join_content(first: &str, second: &str) -> String {
    let (first, second) = (first.trim_end(), second.trim_start());
    if first.is_empty() {
        second.into()
    } else if second.is_empty() {
        first.into()
    } else {
        format!("{first}\n{second}")
    }
}
// React's size slider runs from 2 to 8, opposite to the column count.
fn table_height(columns: usize) -> Pixels {
    px(72. + ((8 - columns) as f32 * (128. / 6.)).round())
}

/// A `#rrggbb` or `#rrggbbaa` color of user data; anything else is not drawn.
pub(crate) fn parse_hex_color(text: &str) -> Option<gpui::Rgba> {
    let hex = text.strip_prefix('#')?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok().map(gpui::rgb),
        8 => u32::from_str_radix(hex, 16).ok().map(gpui::rgba),
        _ => None,
    }
}
