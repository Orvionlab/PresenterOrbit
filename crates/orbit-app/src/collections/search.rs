//! Library search through the native index: the runtime of `native-library-search-runtime.ts`, the helpers of
//! `collection-search-utils.ts` and `library-search-controller.ts`, and the search popover.
//!
//! The index lives in the host. The runtime sends it the presentations whenever they are replaced, and asks it
//! for the ordinals that match a query. An answer counts only while its index generation, request, and query are
//! still the current ones.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
    div, prelude::*, px, AnyElement, App, ClickEvent, Context, Entity, FocusHandle, KeyDownEvent,
    Render, SharedString, Subscription, Window,
};
use orbit_ui::controls::{
    input_state, list_row, ActionButton, ButtonVariant, Dialog, EmptyState, InputEvent, InputState,
    TextInput,
};
use orbit_ui::{icon, ActiveTheme, Translator};
use serde_json::{json, Value};

use super::document_actions::add_entries;
use super::model::{natural_cmp, Item};
use super::selection::{select_document_item, Modifiers};
use crate::state::runtime::Runtime;
use crate::state::slices::{Anchor, Content, Selected, Source, Ui};

/// The most results the popover lists.
const RESULT_LIMIT: usize = 30;

/// What the native index answered for one query against one version of the presentations.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeResult {
    items_generation: u64,
    query: String,
    ordinals: Vec<usize>,
}

/// The search slice: the popover's own state and the bookkeeping of the native runtime.
#[derive(Default)]
pub struct Search {
    pub open: bool,
    pub query: String,
    pub selected_item_id: String,
    pub selected_item_ids: Vec<String>,
    pub anchor: Option<usize>,
    result: Option<NativeResult>,
    session: String,
    generation: u64,
    indexed_generation: u64,
    indexed_items: Option<u64>,
    requested_key: String,
    token: u64,
}

impl Search {
    pub fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        Self {
            session: format!("gpui-{}-{nanos}", std::process::id()),
            ..Self::default()
        }
    }
}

/// `getSearchTerms`.
pub fn search_terms(query: &str) -> Vec<String> {
    query
        .trim()
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// `getLibrarySearchResults` with the native index: the matching presentations sorted by title, at most 30.
/// Results for another query or another version of the presentations count as none.
pub fn search_results(content: &Content, search: &Search) -> Vec<Item> {
    if search_terms(&search.query).is_empty() {
        return Vec::new();
    }
    let Some(result) = search.result.as_ref().filter(|result| {
        result.query == search.query && result.items_generation == content.items_generation
    }) else {
        return Vec::new();
    };
    let items: Vec<&Item> = content.items.iter().collect();
    let mut found: Vec<Item> = result
        .ordinals
        .iter()
        .filter_map(|ordinal| items.get(*ordinal).map(|item| (*item).clone()))
        .collect();
    found.sort_by(|first, second| natural_cmp(&first.title, &second.title));
    found.truncate(RESULT_LIMIT);
    found
}

/// `getSelectedSearchItemIds`.
pub fn selected_result_ids(search: &Search, results: &[Item]) -> Vec<String> {
    let kept: Vec<String> = search
        .selected_item_ids
        .iter()
        .filter(|id| results.iter().any(|item| &item.id == *id))
        .cloned()
        .collect();
    if !kept.is_empty() {
        return kept;
    }
    if results
        .iter()
        .any(|item| item.id == search.selected_item_id)
    {
        return vec![search.selected_item_id.clone()];
    }
    results
        .first()
        .map(|item| vec![item.id.clone()])
        .unwrap_or_default()
}

/// `getSelectedSearchItem`: the focused result, else the first.
pub fn focused_result<'a>(search: &Search, results: &'a [Item]) -> Option<&'a Item> {
    results
        .iter()
        .find(|item| item.id == search.selected_item_id)
        .or_else(|| results.first())
}

/// `selectSearchResult`.
pub fn select_result(
    search: &mut Search,
    results: &[Item],
    id: &str,
    index: usize,
    modifiers: Modifiers,
) {
    let ids: Vec<String> = results.iter().map(|item| item.id.clone()).collect();
    let selected = selected_result_ids(search, results);
    let anchor = search.anchor.unwrap_or(index);
    search.selected_item_id = id.to_string();
    if modifiers.range {
        let (start, end) = (
            anchor.min(index),
            anchor.max(index).min(ids.len().saturating_sub(1)),
        );
        search.selected_item_ids = ids[start..=end].to_vec();
        search.anchor = Some(anchor);
    } else if modifiers.toggle {
        let next: Vec<String> = if selected.iter().any(|selected| selected == id) {
            selected
                .into_iter()
                .filter(|selected| selected != id)
                .collect()
        } else {
            selected.into_iter().chain([id.to_string()]).collect()
        };
        search.selected_item_ids = if next.is_empty() {
            vec![id.to_string()]
        } else {
            next
        };
        search.anchor = Some(index);
    } else {
        search.selected_item_ids = vec![id.to_string()];
        search.anchor = Some(index);
    }
}

/// `moveSearchSelection`.
pub fn move_selection(search: &mut Search, results: &[Item], direction: i32, extend: bool) {
    if results.is_empty() {
        return;
    }
    let ids: Vec<&str> = results.iter().map(|item| item.id.as_str()).collect();
    let current = ids
        .iter()
        .position(|id| *id == search.selected_item_id)
        .unwrap_or(0);
    let next = (current as i64 + i64::from(direction)).clamp(0, ids.len() as i64 - 1) as usize;
    search.selected_item_id = ids[next].to_string();
    if !extend {
        search.selected_item_ids = vec![ids[next].to_string()];
        search.anchor = Some(next);
        return;
    }
    let anchor = search.anchor.unwrap_or(current);
    let (start, end) = (anchor.min(next), anchor.max(next));
    search.selected_item_ids = ids[start..=end].iter().map(|id| id.to_string()).collect();
    search.anchor = Some(anchor);
}

/// The query text changed: the selection moves to what is still listed, else to the first result.
pub fn change_query(search: &mut Search, query: &str, results: &[Item]) {
    search.query = query.to_string();
    let fallback = results
        .first()
        .map(|item| item.id.clone())
        .unwrap_or_default();
    if !results
        .iter()
        .any(|item| item.id == search.selected_item_id)
    {
        search.selected_item_id = fallback.clone();
    }
    let visible = selected_result_ids(search, results);
    search.selected_item_ids = if !visible.is_empty() {
        visible
    } else if fallback.is_empty() {
        Vec::new()
    } else {
        vec![fallback.clone()]
    };
    search.anchor = (!fallback.is_empty()).then_some(0);
}

// ---- The native runtime ----

/// Reacts to a replaced library or a changed query: re-indexes, and asks for the current query.
pub fn sync(runtime: &Runtime, cx: &mut App) {
    let generation_of_items = runtime.content.read(cx).items_generation;
    let needs_index = runtime.search.read(cx).indexed_items != Some(generation_of_items);
    if needs_index {
        let items = runtime.content.read(cx).items.clone();
        let (session, replacement) = runtime.search.update(cx, |search, _| {
            search.generation += 1;
            search.indexed_generation = 0;
            search.indexed_items = Some(generation_of_items);
            search.requested_key.clear();
            search.token += 1;
            search.result = None;
            (search.session.clone(), search.generation)
        });
        let runtime = runtime.clone();
        cx.spawn(async move |cx| {
            let host = runtime.host.clone();
            let answer = cx
                .background_spawn(async move {
                    host.invoke(
                        "replaceLibrarySearchIndex",
                        vec![json!(session), json!(replacement), items.to_values()],
                    )
                })
                .await;
            cx.update(|cx| match answer {
                Ok(answer) => {
                    let accepted = answer["ok"] == true
                        && answer["stale"] != true
                        && answer["generation"] == replacement;
                    let current = runtime.search.read(cx).generation == replacement;
                    if accepted && current {
                        runtime
                            .search
                            .update(cx, |search, _| search.indexed_generation = replacement);
                        request_current_query(&runtime, cx);
                    }
                }
                Err(error) => eprintln!("Native library search index failed: {error}"),
            });
        })
        .detach();
        return;
    }
    request_current_query(runtime, cx);
}

fn request_current_query(runtime: &Runtime, cx: &mut App) {
    let search = runtime.search.read(cx);
    if search.indexed_generation != search.generation {
        return;
    }
    if search.open && !search.query.trim().is_empty() {
        let query = search.query.clone();
        request_query(runtime, &query, cx);
    }
}

fn request_query(runtime: &Runtime, query: &str, cx: &mut App) {
    let generation = runtime.search.read(cx).generation;
    let key = format!("{generation}\0{query}");
    if runtime.search.read(cx).requested_key == key {
        return;
    }
    let (session, token) = runtime.search.update(cx, |search, _| {
        search.requested_key = key;
        search.token += 1;
        (search.session.clone(), search.token)
    });
    let (runtime, query) = (runtime.clone(), query.to_string());
    cx.spawn(async move |cx| {
        let host = runtime.host.clone();
        let asked = query.clone();
        let answer = cx
            .background_spawn(async move {
                host.invoke(
                    "searchLibraryIndex",
                    vec![json!(session), json!(generation), json!(asked)],
                )
            })
            .await;
        cx.update(|cx| match answer {
            Ok(answer) => accept_answer(&runtime, generation, token, &query, &answer, cx),
            Err(error) => eprintln!("Native library search query failed: {error}"),
        });
    })
    .detach();
}

/// An answer is the current one only while nothing it was asked about has changed.
fn accept_answer(
    runtime: &Runtime,
    generation: u64,
    token: u64,
    query: &str,
    answer: &Value,
    cx: &mut App,
) {
    let items_generation = runtime.content.read(cx).items_generation;
    runtime.search.update(cx, |search, cx| {
        if apply_answer(search, items_generation, generation, token, query, answer) {
            cx.notify();
        }
    });
}

/// Stores `answer` as the result when it answers the current index, request, and query. Returns whether it did.
pub fn apply_answer(
    search: &mut Search,
    items_generation: u64,
    generation: u64,
    token: u64,
    query: &str,
    answer: &Value,
) -> bool {
    let valid =
        answer["ok"] == true && answer["stale"] != true && answer["generation"] == generation;
    if !(valid && search.generation == generation && search.token == token && search.query == query)
    {
        return false;
    }
    search.result = Some(NativeResult {
        items_generation,
        query: query.to_string(),
        ordinals: answer["ordinals"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|n| n.as_u64().map(|n| n as usize))
                    .collect()
            })
            .unwrap_or_default(),
    });
    true
}

// ---- Actions ----

/// `openSearchResult`: closes the popover and selects the document, in the playlist when one is selected.
pub fn open_result(runtime: &Runtime, cx: &mut App, item_id: &str) {
    if item_id.is_empty() {
        return;
    }
    runtime.search.update(cx, |search, cx| {
        search.open = false;
        cx.notify();
    });
    let id = item_id.to_string();
    runtime.edit_ui(cx, |_, ui: &mut Ui| {
        ui.selected_documents = vec![Selected {
            item_id: id.clone(),
            index: None,
        }];
        ui.selected_document_source = Some(Source::Library);
        ui.selected_document_playlist_id.clear();
        ui.document_anchor = Some(Anchor {
            source: Source::Library,
            playlist_id: String::new(),
            index: 0,
        });
        ui.selected_library_ids.clear();
        ui.playlist_anchor = None;
        ui.document_menu = None;
        let keep_playlist = !ui.selected_playlist_id.is_empty();
        select_document_item(ui, &id, keep_playlist);
    });
}

/// Whether the results can be added to the selected playlist.
pub fn can_add_to_playlist(content: &Content, ui: &Ui, item_ids: &[String]) -> bool {
    content
        .playlist(&ui.selected_playlist_id)
        .is_some_and(|playlist| !playlist.is_folder())
        && !item_ids.is_empty()
}

/// `addSearchSelectionToSelectedPlaylist`.
pub fn add_selection_to_playlist(runtime: &Runtime, cx: &mut App, fallback_item: &str) {
    let (playlist, ids) = {
        let (content, ui, search) = (
            runtime.content.read(cx),
            runtime.ui.read(cx),
            runtime.search.read(cx),
        );
        let results = search_results(content, search);
        let selected = selected_result_ids(search, &results);
        let ids: Vec<String> = if selected.is_empty() {
            vec![fallback_item.to_string()]
        } else {
            selected
        }
        .into_iter()
        .filter(|id| content.items.contains(id))
        .collect();
        (
            content
                .playlist(&ui.selected_playlist_id)
                .filter(|playlist| !playlist.is_folder())
                .map(|playlist| playlist.id.clone()),
            ids,
        )
    };
    if let (Some(playlist), false) = (playlist, ids.is_empty()) {
        add_entries(runtime, cx, &playlist, ids, false).detach();
    }
}

// ---- The popover ----

pub struct SearchView {
    runtime: Runtime,
    input: Option<(Entity<InputState>, Subscription)>,
    focus_attempts: u32,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl SearchView {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&runtime.search, |_, _, cx| cx.notify()),
            cx.observe(&runtime.content, |_, _, cx| cx.notify()),
            cx.observe(&runtime.ui, |_, _, cx| cx.notify()),
        ];
        Self {
            runtime,
            input: None,
            focus_attempts: 6,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    fn sync_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.runtime.search.read(cx).open;
        if !open {
            self.input = None;
            return;
        }
        if self.input.is_some() {
            return;
        }
        let query = self.runtime.search.read(cx).query.clone();
        let state = input_state(query, window, cx);
        let subscription =
            cx.subscribe_in(&state, window, |this, state, event: &InputEvent, _, cx| {
                match event {
                    InputEvent::Change => {
                        let value = state.read(cx).value().to_string();
                        this.change_query(&value, cx);
                    }
                    // The input consumes Enter itself, so the popover hears it here and not as a key press.
                    InputEvent::PressEnter { secondary, .. } => this.press_enter(*secondary, cx),
                    _ => {}
                }
            });
        self.input = Some((state, subscription));
        self.focus_attempts = 0;
    }

    /// The dialog's focus trap holds the focus until its content is painted, so the input asks again on the
    /// next frames until it has it.
    fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui::Focusable;
        let Some((state, _)) = &self.input else {
            return;
        };
        if self.focus_attempts >= 6 || state.read(cx).focus_handle(cx).is_focused(window) {
            self.focus_attempts = 6;
            return;
        }
        self.focus_attempts += 1;
        state.clone().update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    fn change_query(&mut self, value: &str, cx: &mut Context<Self>) {
        let runtime = self.runtime.clone();
        // The results shown for the new query are not known yet: the selection follows when they arrive.
        let results = {
            let (content, search) = (runtime.content.read(cx), runtime.search.read(cx));
            let probe = Search {
                query: value.to_string(),
                result: search.result.clone(),
                ..Search::default()
            };
            search_results(content, &probe)
        };
        runtime.search.update(cx, |search, cx| {
            change_query(search, value, &results);
            cx.notify();
        });
    }

    /// Enter opens the focused result; Ctrl or Cmd with Enter adds the selection to the selected playlist.
    fn press_enter(&mut self, add_to_playlist: bool, cx: &mut Context<Self>) {
        let runtime = self.runtime.clone();
        let results = search_results(runtime.content.read(cx), runtime.search.read(cx));
        let focused = focused_result(runtime.search.read(cx), &results).map(|item| item.id.clone());
        let Some(id) = focused else { return };
        if add_to_playlist {
            add_selection_to_playlist(&runtime, cx, &id);
        } else {
            open_result(&runtime, cx, &id);
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let runtime = self.runtime.clone();
        let results = search_results(runtime.content.read(cx), runtime.search.read(cx));
        match event.keystroke.key.as_str() {
            "escape" => {
                runtime.search.update(cx, |search, cx| {
                    search.open = false;
                    cx.notify();
                });
                cx.stop_propagation();
            }
            key @ ("up" | "down") => {
                let direction = if key == "down" { 1 } else { -1 };
                let extend = event.keystroke.modifiers.shift;
                runtime.search.update(cx, |search, cx| {
                    move_selection(search, &results, direction, extend);
                    cx.notify();
                });
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    fn source_detail(content: &Content, item: &Item, t: Translator) -> String {
        let mut titles: Vec<String> = Vec::new();
        for library in content
            .libraries
            .iter()
            .filter(|l| l.item_ids.contains(&item.id))
        {
            if !library.title.is_empty() && !titles.contains(&library.title) {
                titles.push(library.title.clone());
            }
        }
        match titles.len() {
            0 => t.t("search.source.library"),
            1 | 2 => titles.join(", "),
            more => format!("{} +{}", titles[..2].join(", "), more - 2),
        }
    }

    fn slide_detail(item: &Item, t: Translator) -> String {
        let count = item.value["slides"].as_array().map_or(0, Vec::len);
        if count == 1 {
            t.t("search.slideCount.one")
        } else {
            t.format("search.slideCount.many", &[("count", &count.to_string())])
        }
    }

    /// `getLibraryItemPreviewText`.
    fn preview_text(item: &Item) -> String {
        let slides = item.value["slides"]
            .as_array()
            .map(|slides| {
                slides
                    .iter()
                    .map(|slide| slide["content"].as_str().unwrap_or("").trim().to_string())
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default();
        if slides.is_empty() {
            item.value["operatorNotes"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string()
        } else {
            slides
        }
    }
}

impl Render for SearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_input(window, cx);
        self.focus_input(window, cx);
        let Some((input, _)) = &self.input else {
            return div().into_any_element();
        };
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let (content, ui, search) = (
            self.runtime.content.read(cx),
            self.runtime.ui.read(cx),
            self.runtime.search.read(cx),
        );
        let results = search_results(content, search);
        let selected_ids = selected_result_ids(search, &results);
        let focused = focused_result(search, &results).cloned();
        let can_add = can_add_to_playlist(content, ui, &selected_ids);
        let in_playlist = content
            .playlist(&ui.selected_playlist_id)
            .is_some_and(|playlist| !playlist.is_folder());
        let rows: Vec<AnyElement> = results
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let selected = selected_ids.contains(&item.id);
                let id = item.id.clone();
                let detail = Self::source_detail(content, item, t);
                list_row(("search-result", index), selected, cx)
                    .debug_selector(move || format!("search-result-{index}"))
                    .child(icon("library").text_color(theme.colors.text))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.colors.text_muted)
                            .text_size(theme.metrics.font_xs)
                            .child(detail),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        let runtime = this.runtime.clone();
                        let results =
                            search_results(runtime.content.read(cx), runtime.search.read(cx));
                        let modifiers: Modifiers = event.modifiers().into();
                        runtime.search.update(cx, |search, cx| {
                            select_result(search, &results, &id, index, modifiers);
                            cx.notify();
                        });
                    }))
                    .into_any_element()
            })
            .collect();
        let list: AnyElement = if search.query.trim().is_empty() {
            EmptyState::new(t.t("search.empty.prompt")).into_any_element()
        } else if results.is_empty() {
            EmptyState::new(t.t("search.empty.noResults")).into_any_element()
        } else {
            div().flex().flex_col().children(rows).into_any_element()
        };
        let preview: AnyElement = match &focused {
            None => div()
                .p(theme.metrics.space_sm)
                .text_color(theme.colors.text_muted)
                .child(t.t("search.preview.empty"))
                .into_any_element(),
            Some(item) => {
                let (open_id, add_id) = (item.id.clone(), item.id.clone());
                let text = Self::preview_text(item);
                let detail = format!(
                    "{} · {}",
                    Self::slide_detail(item, t),
                    Self::source_detail(content, item, t)
                );
                let count = selected_ids.len();
                let add_label = if !can_add {
                    if count > 1 {
                        t.t("search.selectedInPlaylist")
                    } else {
                        t.t("search.inPlaylist")
                    }
                } else if count > 1 {
                    t.format("search.addManyToPlaylist", &[("count", &count.to_string())])
                } else {
                    t.t("search.addToPlaylist")
                };
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(theme.metrics.space_sm)
                    .p(theme.metrics.space_sm)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(item.title.clone()),
                            )
                            .child(
                                div()
                                    .text_color(theme.colors.text_muted)
                                    .text_size(theme.metrics.font_xs)
                                    .child(detail),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(theme.metrics.control_gap)
                            .child(
                                ActionButton::new("search-open", t.t("common.open"))
                                    .variant(ButtonVariant::Secondary)
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        open_result(&this.runtime.clone(), cx, &open_id)
                                    })),
                            )
                            .children(in_playlist.then(|| {
                                ActionButton::new("search-add", add_label)
                                    .variant(ButtonVariant::Secondary)
                                    .disabled(!can_add)
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        add_selection_to_playlist(
                                            &this.runtime.clone(),
                                            cx,
                                            &add_id,
                                        )
                                    }))
                            })),
                    )
                    .child(
                        div()
                            .id("search-preview-text")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .whitespace_normal()
                            .child(if text.is_empty() {
                                t.t("search.preview.empty")
                            } else {
                                text
                            }),
                    )
                    .into_any_element()
            }
        };
        let search_view = cx.weak_entity();
        Dialog::new(
            "library-search",
            t.t("search.library"),
            t.t("common.close"),
            move |_, _, cx| {
                search_view
                    .update(cx, |this, cx| {
                        this.runtime.search.update(cx, |search, cx| {
                            search.open = false;
                            cx.notify();
                        })
                    })
                    .ok();
            },
        )
        .width(px(680.))
        .child(
            div()
                .id("search-body")
                .track_focus(&self.focus)
                .capture_key_down(
                    cx.listener(|this, event: &KeyDownEvent, _, cx| this.key_down(event, cx)),
                )
                .flex()
                .flex_col()
                .gap(theme.metrics.space_sm)
                .h(px(380.))
                .child(
                    TextInput::new("search-input", input)
                        .label(SharedString::from(t.t("search.placeholder.library"))),
                )
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .gap(theme.metrics.space_sm)
                        .child(
                            div()
                                .id("search-results")
                                .flex_none()
                                .w(px(260.))
                                .overflow_y_scroll()
                                .child(list),
                        )
                        .child(preview),
                ),
        )
        .into_any_element()
    }
}

/// `search` command: opens the popover.
pub fn open_search(runtime: &Runtime, cx: &mut App) {
    runtime.search.update(cx, |search, cx| {
        search.open = true;
        cx.notify();
    });
}
