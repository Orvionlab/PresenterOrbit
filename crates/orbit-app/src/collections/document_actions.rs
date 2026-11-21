//! Actions on documents and playlist entries: rename, duplicate, delete, add to, move to, copy and paste, entry
//! creation, reorder, arrangement choice, and placeholder links. Ports of `document-*-actions-controller.ts`,
//! `library-item-persistence-controller.ts`, `playlist-entry-*.ts`, and `playlist-document-reorder-controller.ts`.
//!
//! Copies and new documents do not get a provisional document: the host allocates their identity and content, so
//! the list shows them when it answers. Everything else changes the slices before the host answers.

use std::collections::HashSet;

use gpui::{App, Task};
use serde_json::{json, Value};

use orbit_ui::controls::DEFAULT_HEADER_COLOR;

use super::model::{unique_entry_id, unique_entry_title, Entry, EntryKind, ThemeAssignment};
use super::tree_actions::{spawn_persist, translator, DetachExt};
use crate::state::runtime::{Kind, Persist, Runtime};
pub use crate::state::slices::NewPresentation;
use crate::state::slices::{Anchor, Content, DocumentMenu, Selected, Source, Ui};

fn string_ids(response: &Option<Value>, field: &str) -> Vec<String> {
    response
        .as_ref()
        .and_then(|response| response[field].as_array())
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `getDocumentMenuEntries`: the documents a context menu acts on, as `(item id, row index)`.
pub fn menu_documents(content: &Content, ui: &Ui) -> Vec<(String, usize)> {
    let Some(menu) = &ui.document_menu else {
        return Vec::new();
    };
    let source = menu.source;
    let selected: Vec<(String, usize)> = ui
        .selected_documents
        .iter()
        .map(|selected| (selected.item_id.clone(), selected.index.unwrap_or(0)))
        .collect();
    let entries = if selected.is_empty() {
        vec![(
            content
                .playlist(&ui.selected_document_playlist_id)
                .filter(|_| source == Source::Playlist)
                .and_then(|playlist| playlist.entries.get(menu.index))
                .map_or_else(|| menu.id.clone(), |entry| entry.item_id.clone()),
            menu.index,
        )]
    } else {
        selected
    };
    entries
        .into_iter()
        .filter(|(item_id, _)| content.items.contains(item_id))
        .collect()
}

/// `getDocumentContextPlaylistEntry`: the entry a playlist row menu was opened on.
pub fn menu_entry(content: &Content, ui: &Ui) -> Option<(usize, Entry)> {
    let menu = ui.document_menu.as_ref()?;
    let playlist = content.playlist(&ui.selected_playlist_id)?;
    let entry = playlist.entries.get(menu.index)?;
    (entry.id == menu.id || entry.item_id == menu.id).then(|| (menu.index, entry.clone()))
}

// ---- Opening menus (`collection-context-menu-controller.ts`) ----

/// `openDocumentContextMenu`.
pub fn open_document_menu(
    content: &Content,
    ui: &mut Ui,
    id: &str,
    source: Source,
    index: usize,
    position: (f32, f32),
) {
    let playlist_id = if source == Source::Playlist {
        ui.selected_playlist_id.clone()
    } else {
        String::new()
    };
    let entry = content
        .playlist(&playlist_id)
        .and_then(|playlist| playlist.entries.get(index))
        .filter(|entry| entry.id == id || entry.item_id == id);
    let item_id = match entry {
        Some(entry) if entry.kind == EntryKind::Presentation => entry.item_id.clone(),
        _ => id.to_string(),
    };
    let clicked = Selected {
        item_id: item_id.clone(),
        index: (source == Source::Playlist).then_some(index),
    };
    let current: Vec<Selected> = if ui.selected_document_source == Some(source)
        && (source == Source::Library || ui.selected_document_playlist_id == playlist_id)
    {
        ui.selected_documents.clone()
    } else {
        Vec::new()
    };
    let contains_clicked = current.iter().any(|selected| {
        selected.item_id == clicked.item_id
            && (source == Source::Library || selected.index == clicked.index)
    });
    if entry.is_some_and(|entry| entry.kind == EntryKind::Presentation) {
        ui.selected_library_item_id = item_id;
    }
    ui.selected_documents = if contains_clicked {
        current
    } else {
        vec![clicked]
    };
    ui.selected_document_source = Some(source);
    ui.selected_document_playlist_id = playlist_id.clone();
    ui.document_anchor = Some(Anchor {
        source,
        playlist_id,
        index,
    });
    ui.selected_library_ids = if source == Source::Playlist || ui.selected_library_id.is_empty() {
        Vec::new()
    } else {
        vec![ui.selected_library_id.clone()]
    };
    ui.library_anchor = None;
    if source == Source::Playlist {
        ui.selected_playlist_ids.clear();
    }
    ui.close_menus();
    ui.document_menu = Some(DocumentMenu {
        id: id.to_string(),
        source,
        index,
        position,
    });
}

// ---- Documents ----

/// `commitLibraryItemTitle` through `mutateItem` / `setTitle`.
pub fn commit_item_title(
    runtime: &Runtime,
    cx: &mut App,
    item_id: &str,
    raw_title: &str,
) -> Option<Task<()>> {
    let title = raw_title.trim().to_string();
    let unchanged = runtime
        .content
        .read(cx)
        .items
        .get(item_id)
        .is_none_or(|item| title.is_empty() || title == item.title);
    if unchanged {
        runtime.ui.update(cx, |ui, cx| {
            ui.rename_item_id.clear();
            cx.notify();
        });
        return None;
    }
    let id = item_id.to_string();
    let request = Persist::new(
        Kind::Library,
        json!({
            "type": "mutateItem",
            "itemId": item_id,
            "operation": { "type": "setTitle", "title": title },
        }),
        {
            let title = title.clone();
            move |content: &mut Content, ui: &mut Ui| {
                let items = content.items.map_item(&id, |item| item.with_title(&title));
                content.replace_items(items);
                ui.rename_item_id.clear();
            }
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// Selects the first copy the host created when the user has not selected anything else meanwhile.
fn adopt_copied_item(
    runtime: &Runtime,
    response: &Option<Value>,
    library_id: &str,
    previous_item: &str,
    cx: &mut App,
) {
    let Some(copied) = string_ids(response, "copiedItemIds").into_iter().next() else {
        return;
    };
    if !runtime.content.read(cx).items.contains(&copied) {
        return;
    }
    runtime.ui.update(cx, |ui, cx| {
        if ui.selected_library_id == library_id && ui.selected_library_item_id == previous_item {
            ui.selected_library_item_id = copied;
            cx.notify();
        }
    });
}

/// `duplicateDocumentItems`. For a library, the host copies the sources into the library; for a playlist, a new
/// entry for each document is added.
pub fn duplicate_documents(
    runtime: &Runtime,
    cx: &mut App,
    documents: Vec<(String, usize)>,
    source: Source,
    playlist_id: &str,
) -> Option<Task<()>> {
    if documents.is_empty() {
        close_document_menu(runtime, cx);
        return None;
    }
    let item_ids: Vec<String> = documents.iter().map(|(id, _)| id.clone()).collect();
    if source == Source::Playlist && !playlist_id.is_empty() {
        let usable = runtime
            .content
            .read(cx)
            .playlist(playlist_id)
            .is_some_and(|playlist| !playlist.is_folder());
        if !usable {
            close_document_menu(runtime, cx);
            return None;
        }
        return Some(add_entries(runtime, cx, playlist_id, item_ids, false));
    }
    let (library_id, first_source) = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        let first = item_ids.first().cloned().unwrap_or_default();
        let library = content.library_id_for_item(&first);
        let library = if library.is_empty() {
            ui.selected_library_id.clone()
        } else {
            library.to_string()
        };
        let valid = content.library(&library).is_some()
            && item_ids.iter().all(|id| content.items.contains(id));
        if !valid {
            close_document_menu(runtime, cx);
            return None;
        }
        (library, ui.selected_library_item_id.clone())
    };
    let mut mutation =
        json!({ "type": "copyItems", "sourceItemIds": item_ids, "targetLibraryId": library_id });
    // A single copy goes right after its source.
    if item_ids.len() == 1
        && runtime
            .content
            .read(cx)
            .library(&library_id)
            .is_some_and(|library| library.item_ids.contains(&item_ids[0]))
    {
        mutation["afterItemId"] = json!(item_ids[0]);
    }
    let library = library_id.clone();
    let request = Persist::new(
        Kind::Library,
        mutation,
        move |_content: &mut Content, ui: &mut Ui| {
            ui.selected_library_id = library;
            ui.selected_playlist_id.clear();
        },
    );
    Some(spawn_persist(
        runtime,
        cx,
        request,
        move |runtime, response, cx| {
            adopt_copied_item(runtime, &response, &library_id, &first_source, cx);
        },
    ))
}

fn close_document_menu(runtime: &Runtime, cx: &mut App) {
    runtime.ui.update(cx, |ui, cx| {
        ui.document_menu = None;
        cx.notify();
    });
}

pub fn set_entries_hidden(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: &str,
    entry_ids: Vec<String>,
    hidden: bool,
) {
    if entry_ids.is_empty() {
        return close_document_menu(runtime, cx);
    }
    let playlist = playlist_id.to_string();
    let entries: HashSet<_> = entry_ids.iter().cloned().collect();
    let request = Persist::new(
        Kind::Playlist,
        json!({"type":"setEntriesHidden", "playlistId":playlist_id, "entryIds":entry_ids, "hidden":hidden}),
        move |content, ui| {
            if let Some(playlist) = content.playlists.iter_mut().find(|p| p.id == playlist) {
                for entry in playlist
                    .entries
                    .iter_mut()
                    .filter(|entry| entries.contains(&entry.id))
                {
                    if hidden {
                        entry.raw["hidden"] = json!(true);
                    } else if let Some(raw) = entry.raw.as_object_mut() {
                        raw.remove("hidden");
                    }
                }
            }
            ui.document_menu = None;
            ui.selected_documents.clear();
            ui.document_anchor = None;
        },
    );
    spawn_persist(runtime, cx, request, |_, _, _| {}).detach();
}

/// `addDocumentItemsToPlaylist` (skip existing) and the duplicate path (no skipping): `addEntry`.
pub fn add_entries(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: &str,
    item_ids: Vec<String>,
    skip_existing: bool,
) -> Task<()> {
    let id = playlist_id.to_string();
    let request = Persist::new(
        Kind::Playlist,
        json!({ "type": "addEntry", "playlistId": playlist_id, "itemIds": item_ids, "skipExisting": skip_existing }),
        move |content: &mut Content, ui: &mut Ui| {
            if let Some(playlist) = content.playlists.iter_mut().find(|p| p.id == id) {
                let existing: HashSet<String> = playlist.item_ids().into_iter().collect();
                for item_id in &item_ids {
                    if skip_existing && existing.contains(item_id) {
                        continue;
                    }
                    let base = format!("presentation-{item_id}");
                    let mut entry_id = base.clone();
                    let mut suffix = 2;
                    while playlist.entries.iter().any(|entry| entry.id == entry_id) {
                        entry_id = format!("{base}-{suffix}");
                        suffix += 1;
                    }
                    playlist.entries.push(Entry {
                        id: entry_id.clone(),
                        kind: EntryKind::Presentation,
                        item_id: item_id.clone(),
                        title: String::new(),
                        linked_item_id: String::new(),
                        arrangement_id: String::new(),
                        color: String::new(),
                        raw: json!({ "id": entry_id, "type": "presentation", "itemId": item_id }),
                    });
                }
            }
            ui.selected_playlist_id = id;
        },
    );
    spawn_persist(runtime, cx, request, |_, _, _| {})
}

/// `addDocumentItemsToPlaylist` for the documents of the open menu.
pub fn add_documents_to_playlist(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: &str,
) -> Option<Task<()>> {
    let (documents, usable) = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        (
            menu_documents(content, ui),
            content
                .playlist(playlist_id)
                .is_some_and(|playlist| !playlist.is_folder()),
        )
    };
    if !usable || documents.is_empty() {
        close_document_menu(runtime, cx);
        return None;
    }
    let ids = documents.into_iter().map(|(id, _)| id).collect();
    Some(add_entries(runtime, cx, playlist_id, ids, true))
}

/// `addDocumentItemsToLibrary`: copies of the menu's documents appended to another library.
pub fn add_documents_to_library(
    runtime: &Runtime,
    cx: &mut App,
    library_id: &str,
) -> Option<Task<()>> {
    let (ids, exists) = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        (
            menu_documents(content, ui),
            content.library(library_id).is_some(),
        )
    };
    if !exists || ids.is_empty() {
        close_document_menu(runtime, cx);
        return None;
    }
    let request = Persist::new(
        Kind::Library,
        json!({
            "type": "copyItems",
            "sourceItemIds": ids.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            "targetLibraryId": library_id,
        }),
        |_: &mut Content, _: &mut Ui| {},
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// `moveDocumentItemsToLibrary`.
pub fn move_documents_to_library(
    runtime: &Runtime,
    cx: &mut App,
    library_id: &str,
) -> Option<Task<()>> {
    let ids: Vec<String> = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        let mut seen = HashSet::new();
        menu_documents(content, ui)
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| seen.insert(id.clone()))
            .collect()
    };
    let valid = !ids.is_empty() && runtime.content.read(cx).library(library_id).is_some();
    if !valid {
        close_document_menu(runtime, cx);
        return None;
    }
    let (target, moved) = (library_id.to_string(), ids.clone());
    let request = Persist::new(
        Kind::Library,
        json!({ "type": "moveItems", "itemIds": ids, "targetLibraryId": library_id }),
        move |content: &mut Content, ui: &mut Ui| {
            let moved_set: HashSet<&String> = moved.iter().collect();
            for library in &mut content.libraries {
                library.item_ids.retain(|id| !moved_set.contains(id));
                if library.id == target {
                    library.item_ids.extend(moved.iter().cloned());
                }
            }
            ui.selected_library_id = target;
            ui.selected_playlist_id.clear();
            ui.selected_library_item_id = moved.first().cloned().unwrap_or_default();
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// `deleteDocumentItems`: a library deletes the documents everywhere; a playlist only removes their entries.
pub fn delete_documents(
    runtime: &Runtime,
    cx: &mut App,
    documents: Vec<(String, usize)>,
    source: Source,
    playlist_id: &str,
) -> Option<Task<()>> {
    if documents.is_empty() {
        close_document_menu(runtime, cx);
        return None;
    }
    if source == Source::Playlist && !playlist_id.is_empty() {
        let indexes: Vec<usize> = documents.iter().map(|(_, index)| *index).collect();
        return delete_entries_at(runtime, cx, playlist_id, &indexes);
    }
    let deleted: HashSet<String> = documents.iter().map(|(id, _)| id.clone()).collect();
    let valid = {
        let content = runtime.content.read(cx);
        !deleted.is_empty() && deleted.iter().all(|id| content.items.contains(id))
    };
    if !valid {
        close_document_menu(runtime, cx);
        return None;
    }
    let ids: Vec<&String> = deleted.iter().collect();
    let request = Persist::new(
        Kind::Library,
        json!({ "type": "deleteItems", "itemIds": ids }),
        move |content: &mut Content, ui: &mut Ui| {
            for library in &mut content.libraries {
                library.item_ids.retain(|id| !deleted.contains(id));
            }
            let items = content.items.without(&deleted);
            content.replace_items(items);
            for playlist in &mut content.playlists {
                playlist
                    .entries
                    .retain(|entry| entry.item_id.is_empty() || !deleted.contains(&entry.item_id));
            }
            ui.selected_library_item_id = content
                .library(&ui.selected_library_id)
                .and_then(|library| library.item_ids.first().cloned())
                .unwrap_or_default();
            ui.selected_playlist_id.clear();
            ui.selected_documents.clear();
            ui.document_anchor = None;
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// `deleteSelectedPlaylistEntries`: occurrences addressed by index with their full expected entries, so a stale
/// retry cannot delete another occurrence.
pub fn delete_entries_at(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: &str,
    indexes: &[usize],
) -> Option<Task<()>> {
    let (entries, anchors) = {
        let playlist = runtime.content.read(cx).playlist(playlist_id)?;
        if indexes.is_empty() || indexes.iter().any(|index| *index >= playlist.entries.len()) {
            close_document_menu(runtime, cx);
            return None;
        }
        let selected: HashSet<usize> = indexes.iter().copied().collect();
        let anchors: Vec<Value> = playlist
            .entries
            .iter()
            .enumerate()
            .filter(|(index, _)| selected.contains(index))
            .map(|(index, entry)| {
                json!({ "entryIndex": index, "entryId": entry.id, "expectedEntry": entry.raw })
            })
            .collect();
        let ids: Vec<String> = playlist
            .entries
            .iter()
            .enumerate()
            .filter(|(index, _)| selected.contains(index))
            .map(|(_, entry)| entry.id.clone())
            .collect();
        (ids, anchors)
    };
    let (id, selected): (String, HashSet<usize>) =
        (playlist_id.to_string(), indexes.iter().copied().collect());
    let request = Persist::new(
        Kind::Playlist,
        json!({ "type": "deleteEntry", "playlistId": playlist_id, "entryIds": entries, "entryAnchors": anchors }),
        move |content: &mut Content, ui: &mut Ui| {
            if let Some(playlist) = content.playlists.iter_mut().find(|p| p.id == id) {
                let mut index = 0;
                playlist.entries.retain(|_| {
                    index += 1;
                    !selected.contains(&(index - 1))
                });
            }
            ui.selected_playlist_id = id;
            ui.selected_documents.clear();
            ui.document_anchor = None;
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// `handleDocumentMenuAction` for a library or playlist presentation row.
pub fn handle_document_menu_action(runtime: &Runtime, cx: &mut App, action: &str, item_id: &str) {
    let (documents, source, playlist_id, exists) = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        let menu = ui.document_menu.clone();
        (
            menu_documents(content, ui),
            menu.as_ref().map_or(Source::Library, |menu| menu.source),
            if menu
                .as_ref()
                .is_some_and(|menu| menu.source == Source::Playlist)
            {
                ui.selected_playlist_id.clone()
            } else {
                String::new()
            },
            content.items.contains(item_id),
        )
    };
    if !exists && action != "paste" {
        return close_document_menu(runtime, cx);
    }
    if let Some(ThemeAssignment::Theme(_)) = ThemeAssignment::parse_action(action) {
        // Slide themes belong to the slide editor in a later phase.
        return close_document_menu(runtime, cx);
    }
    if let Some(arrangement) = action.strip_prefix("arrangement:") {
        if source == Source::Playlist {
            let entry = menu_entry(runtime.content.read(cx), runtime.ui.read(cx));
            if let Some((index, entry)) = entry {
                set_entry_arrangement(
                    runtime,
                    cx,
                    &playlist_id,
                    &entry.id,
                    Some(index),
                    arrangement,
                )
                .detach_task();
            }
            return;
        }
    }
    if let Some(destination) = action.strip_prefix("destination:") {
        for (id, _) in &documents {
            let destination = destination.to_string();
            crate::cues::actions::mutate_optimistic(
                runtime,
                id,
                crate::cues::flow::set_destination(&destination),
                move |item| {
                    if destination == crate::cues::flow::ANNOUNCEMENTS {
                        item["destination"] = json!(destination);
                    } else if let Some(item) = item.as_object_mut() {
                        item.remove("destination");
                    }
                },
                cx,
            );
        }
        return close_document_menu(runtime, cx);
    }
    if let Some(size) = action.strip_prefix("size:") {
        let size = size
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
        let Some((width, height)) = size else {
            return close_document_menu(runtime, cx);
        };
        let size = json!({"width":width,"height":height});
        for (id, _) in &documents {
            let Some(mut item) = runtime
                .content
                .read(cx)
                .items
                .get(id)
                .map(|item| (*item.value).clone())
            else {
                continue;
            };
            if let Err(error) =
                orbit_host::authoring::patch_presentation_setting(&mut item, "size", &size)
            {
                crate::cues::actions::error(runtime, cx, error);
                return close_document_menu(runtime, cx);
            }
            crate::cues::actions::mutate_optimistic(
                runtime,
                id,
                json!({"type":"setPresentationSetting","field":"size","value":size}),
                move |current| *current = item,
                cx,
            );
        }
        return close_document_menu(runtime, cx);
    }
    match action {
        "hide" => {
            let entries: Vec<_> = documents
                .iter()
                .filter_map(|(_, index)| {
                    runtime
                        .content
                        .read(cx)
                        .playlist(&playlist_id)?
                        .entries
                        .get(*index)
                        .map(|entry| entry.id.clone())
                })
                .collect();
            set_entries_hidden(runtime, cx, &playlist_id, entries, true);
        }
        "show-library" => {
            runtime.edit_ui(cx, |content, ui| {
                let library = content.library_id_for_item(item_id).to_string();
                ui.selected_library_id = library;
                ui.selected_library_ids = vec![ui.selected_library_id.clone()];
                ui.selected_playlist_id.clear();
                ui.selected_playlist_ids.clear();
                ui.selected_library_item_id = item_id.to_string();
                ui.selected_documents = vec![Selected {
                    item_id: item_id.to_string(),
                    index: None,
                }];
                ui.selected_document_source = Some(Source::Library);
                ui.selected_document_playlist_id.clear();
                ui.document_menu = None;
            });
        }
        "edit" | "reflow" => {
            close_document_menu(runtime, cx);
            if let Err(error) = crate::automation::inputs::command(
                runtime,
                if action == "edit" {
                    "open-slide-canvas"
                } else {
                    "open-reflow"
                },
                cx,
            ) {
                crate::cues::actions::error(runtime, cx, error);
            }
        }
        "copy" => {
            let items = documents
                .iter()
                .filter_map(|(id, _)| runtime.content.read(cx).items.get(id))
                .map(|item| (*item.value).clone())
                .collect();
            runtime.ui.update(cx, |ui, cx| {
                ui.clipboard.items = items;
                ui.clipboard.entry = None;
                ui.document_menu = None;
                cx.notify();
            });
        }
        "paste" => paste_documents(runtime, cx, item_id).detach_task(),
        "delete" => delete_documents(runtime, cx, documents, source, &playlist_id).detach_task(),
        "duplicate" => {
            duplicate_documents(runtime, cx, documents, source, &playlist_id).detach_task()
        }
        "rename" => runtime.ui.update(cx, |ui, cx| {
            ui.selected_library_item_id = item_id.to_string();
            ui.rename_item_id = item_id.to_string();
            ui.document_menu = None;
            cx.notify();
        }),
        _ => close_document_menu(runtime, cx),
    }
}

/// `pasteDocumentItem`: clipboard documents become copies in the target library, or entries in the playlist.
pub fn paste_documents(runtime: &Runtime, cx: &mut App, fallback_item: &str) -> Option<Task<()>> {
    let (clipboard, source, playlist_id, library_id) = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        let library = content.library_id_for_item(fallback_item);
        let library = if !library.is_empty() {
            library.to_string()
        } else if !ui.selected_library_id.is_empty() {
            ui.selected_library_id.clone()
        } else {
            content
                .libraries
                .first()
                .map(|l| l.id.clone())
                .unwrap_or_default()
        };
        (
            ui.clipboard.items.clone(),
            ui.document_menu.as_ref().map(|menu| menu.source),
            ui.selected_playlist_id.clone(),
            library,
        )
    };
    if clipboard.is_empty() {
        close_document_menu(runtime, cx);
        return None;
    }
    if source == Some(Source::Playlist) && !playlist_id.is_empty() {
        let ids: Vec<String> = clipboard
            .iter()
            .filter_map(|item| item["id"].as_str().map(str::to_string))
            .collect();
        let usable = runtime
            .content
            .read(cx)
            .playlist(&playlist_id)
            .is_some_and(|p| !p.is_folder());
        if !usable || ids.is_empty() {
            close_document_menu(runtime, cx);
            return None;
        }
        return Some(add_entries(runtime, cx, &playlist_id, ids, false));
    }
    if runtime.content.read(cx).library(&library_id).is_none() {
        close_document_menu(runtime, cx);
        return None;
    }
    let (target, previous) = (
        library_id.clone(),
        runtime.ui.read(cx).selected_library_item_id.clone(),
    );
    let request = Persist::new(
        Kind::Library,
        json!({ "type": "copyItems", "items": clipboard, "targetLibraryId": library_id }),
        move |_: &mut Content, ui: &mut Ui| {
            ui.selected_library_id = target;
            ui.selected_playlist_id.clear();
        },
    );
    Some(spawn_persist(
        runtime,
        cx,
        request,
        move |runtime, response, cx| {
            adopt_copied_item(runtime, &response, &library_id, &previous, cx);
        },
    ))
}

// ---- Playlist entries ----

/// `createCollectionPlaylistEntry`: a header or placeholder at the end of the selected playlist.
pub fn create_entry(runtime: &Runtime, cx: &mut App, kind: EntryKind) -> Option<Task<()>> {
    let t = translator(cx);
    let base_title = if kind == EntryKind::Header {
        t.t("collections.default.newHeader")
    } else {
        t.t("collections.default.newPlaceholder")
    };
    let playlist = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        content
            .playlist(&ui.selected_playlist_id)
            .filter(|playlist| !playlist.is_folder())
            .cloned()
    };
    let Some(playlist) = playlist else {
        runtime.ui.update(cx, |ui, cx| {
            ui.create_menu = None;
            cx.notify();
        });
        return None;
    };
    let title = unique_entry_title(&base_title, &playlist.entries);
    let id = unique_entry_id(kind, &title, &playlist.entries);
    let color = (kind == EntryKind::Header).then_some(DEFAULT_HEADER_COLOR);
    let mut mutation = json!({
        "type": "createEntry", "playlistId": playlist.id, "entryType": kind.as_str(), "title": base_title,
    });
    if let Some(color) = color {
        mutation["color"] = json!(color);
    }
    let playlist_id = playlist.id.clone();
    let request = Persist::new(
        Kind::Playlist,
        mutation,
        move |content: &mut Content, ui: &mut Ui| {
            if let Some(playlist) = content.playlists.iter_mut().find(|p| p.id == playlist_id) {
                let mut raw = json!({ "id": id, "type": kind.as_str(), "title": title });
                if let Some(color) = color {
                    raw["color"] = json!(color);
                }
                playlist.entries.push(Entry {
                    id,
                    kind,
                    item_id: String::new(),
                    title,
                    linked_item_id: String::new(),
                    arrangement_id: String::new(),
                    color: color.unwrap_or("").to_string(),
                    raw,
                });
            }
            ui.selected_playlist_id = playlist_id.clone();
            ui.selected_playlist_ids = vec![playlist_id];
            ui.selected_documents.clear();
            ui.document_anchor = None;
            ui.create_menu = None;
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// `commitPlaylistEntryTitle`: the rename carries the entry's index and current title as an occurrence anchor.
pub fn commit_entry_title(
    runtime: &Runtime,
    cx: &mut App,
    entry_id: &str,
    raw_title: &str,
) -> Option<Task<()>> {
    let title = raw_title.trim().to_string();
    let target = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        content
            .playlist(&ui.selected_playlist_id)
            .and_then(|playlist| {
                playlist
                    .entries
                    .iter()
                    .position(|entry| entry.id == entry_id)
                    .map(|index| {
                        (
                            playlist.id.clone(),
                            index,
                            playlist.entries[index].title.clone(),
                        )
                    })
            })
    };
    let Some((playlist_id, index, current)) =
        target.filter(|(_, _, current)| !title.is_empty() && *current != title)
    else {
        runtime.ui.update(cx, |ui, cx| {
            ui.rename_entry_id.clear();
            cx.notify();
        });
        return None;
    };
    let id = entry_id.to_string();
    let request = Persist::new(
        Kind::Playlist,
        json!({
            "type": "renameEntry", "playlistId": playlist_id, "entryId": entry_id,
            "entryIndex": index, "expectedTitle": current, "title": title,
        }),
        {
            let title = title.clone();
            move |content: &mut Content, ui: &mut Ui| {
                if let Some(entry) = content
                    .playlists
                    .iter_mut()
                    .find(|p| p.id == playlist_id)
                    .and_then(|p| p.entries.get_mut(index))
                {
                    entry.title = title.clone();
                    entry.raw["title"] = json!(title);
                }
                ui.rename_entry_id.clear();
                let _ = &id;
            }
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// `setPlaylistEntryArrangement`: an empty `arrangement_id` selects the master.
pub fn set_entry_arrangement(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: &str,
    entry_id: &str,
    requested_index: Option<usize>,
    arrangement_id: &str,
) -> Option<Task<()>> {
    let target = {
        let playlist = runtime.content.read(cx).playlist(playlist_id)?;
        let index = requested_index
            .filter(|index| {
                playlist.entries.get(*index).is_some_and(|entry| {
                    entry.id == entry_id && entry.kind == EntryKind::Presentation
                })
            })
            .or_else(|| {
                playlist
                    .entries
                    .iter()
                    .position(|entry| entry.id == entry_id && entry.kind == EntryKind::Presentation)
            })?;
        (index, playlist.entries[index].raw.clone())
    };
    let (index, expected) = target;
    let (id, arrangement) = (playlist_id.to_string(), arrangement_id.to_string());
    let mutation = json!({
        "type": "setEntryArrangement", "playlistId": playlist_id, "entryId": entry_id,
        "entryIndex": index, "expectedEntry": expected, "arrangementId": arrangement_id,
    });
    let entry_id = entry_id.to_string();
    Some(
        runtime.save_preview(cx, Kind::Playlist, mutation, move |content| {
            if let Some(entry) = content
                .playlists
                .iter_mut()
                .find(|p| p.id == id)
                .and_then(|p| p.entries.iter_mut().find(|entry| entry.id == entry_id))
            {
                entry.arrangement_id = arrangement.clone();
                if arrangement.is_empty() {
                    entry.raw.as_object_mut().map(|o| o.remove("arrangementId"));
                } else {
                    entry.raw["arrangementId"] = json!(arrangement);
                }
            }
        }),
    )
}

/// `commitPlaylistDocumentReorder`: moves the entries at `sources` so they start at `drop_index` in the list
/// without them. Returns `None` for a drop that changes nothing.
pub fn reordered_entries(
    entries: &[Entry],
    sources: &[usize],
    drop_index: usize,
) -> Option<(Vec<Entry>, usize)> {
    let mut valid: Vec<usize> = sources
        .iter()
        .copied()
        .filter(|index| *index < entries.len())
        .collect();
    valid.sort_unstable();
    valid.dedup();
    if valid.is_empty() {
        return None;
    }
    let moved: Vec<Entry> = valid.iter().map(|index| entries[*index].clone()).collect();
    let remaining: Vec<Entry> = entries
        .iter()
        .enumerate()
        .filter(|(index, _)| !valid.contains(index))
        .map(|(_, entry)| entry.clone())
        .collect();
    let next = drop_index.min(remaining.len());
    // Dropping a contiguous block where it already is changes nothing.
    let contiguous = valid.windows(2).all(|pair| pair[1] == pair[0] + 1);
    if contiguous && next == valid[0] {
        return None;
    }
    let mut result = remaining;
    for (offset, entry) in moved.into_iter().enumerate() {
        result.insert(next + offset, entry);
    }
    Some((result, next))
}

/// Reorders playlist entries by dragging them. The mutation quotes the position and complete entry of every
/// moved entry and of the target, so a stale retry is rejected instead of moving other entries.
pub fn reorder_entries(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: &str,
    sources: &[usize],
    drop_index: usize,
) -> Option<Task<()>> {
    let playlist = runtime.content.read(cx).playlist(playlist_id)?.clone();
    let (next, next_index) = reordered_entries(&playlist.entries, sources, drop_index)?;
    let mut valid: Vec<usize> = sources
        .iter()
        .copied()
        .filter(|i| *i < playlist.entries.len())
        .collect();
    valid.sort_unstable();
    valid.dedup();
    let remaining_count = playlist.entries.len() - valid.len();
    if remaining_count == 0 {
        return None;
    }
    let remaining: Vec<&Entry> = playlist
        .entries
        .iter()
        .enumerate()
        .filter(|(index, _)| !valid.contains(index))
        .map(|(_, entry)| entry)
        .collect();
    let target = remaining[next_index.min(remaining_count - 1)];
    let target_index = playlist
        .entries
        .iter()
        .position(|entry| std::ptr::eq(entry, target))?;
    let placement = if next_index >= remaining_count {
        "after"
    } else {
        "before"
    };
    let anchors: Vec<Value> = valid
        .iter()
        .map(|index| {
            let entry = &playlist.entries[*index];
            json!({ "entryIndex": index, "entryId": entry.id, "expectedEntry": entry.raw })
        })
        .collect();
    let moved_ids: Vec<&String> = valid.iter().map(|i| &playlist.entries[*i].id).collect();
    let moved_items: Vec<String> = valid
        .iter()
        .filter_map(|i| {
            let entry = &playlist.entries[*i];
            (entry.kind == EntryKind::Presentation).then(|| entry.item_id.clone())
        })
        .collect();
    let mutation = json!({
        "type": "reorderEntries", "playlistId": playlist_id, "entryIds": moved_ids,
        "targetEntryId": target.id, "placement": placement, "entryAnchors": anchors,
        "targetEntryIndex": target_index, "targetExpectedEntry": target.raw,
    });
    let id = playlist_id.to_string();
    let request = Persist::new(
        Kind::Playlist,
        mutation,
        move |content: &mut Content, ui: &mut Ui| {
            if let Some(playlist) = content.playlists.iter_mut().find(|p| p.id == id) {
                playlist.entries = next;
            }
            ui.selected_playlist_id = id.clone();
            if let Some(first) = moved_items.first() {
                ui.selected_library_item_id = first.clone();
            }
            ui.selected_documents = moved_items
                .iter()
                .enumerate()
                .map(|(offset, item)| Selected {
                    item_id: item.clone(),
                    index: Some(next_index + offset),
                })
                .collect();
            ui.selected_document_source = Some(Source::Playlist);
            ui.selected_document_playlist_id = id.clone();
            ui.document_anchor = Some(Anchor {
                source: Source::Playlist,
                playlist_id: id,
                index: next_index,
            });
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

fn entry_copy_anchor(entries: &[Entry], insertion_index: usize) -> Value {
    if entries.is_empty() {
        return json!({});
    }
    if insertion_index >= entries.len() {
        let index = entries.len() - 1;
        return json!({
            "targetEntryId": entries[index].id, "placement": "after",
            "targetEntryIndex": index, "expectedTargetEntry": entries[index].raw,
        });
    }
    json!({
        "targetEntryId": entries[insertion_index].id, "placement": "before",
        "targetEntryIndex": insertion_index, "expectedTargetEntry": entries[insertion_index].raw,
    })
}

fn merge(mut base: Value, extra: Value) -> Value {
    if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        base.extend(extra.clone());
    }
    base
}

fn copy_of(entry: &Entry, existing: &[Entry]) -> Entry {
    let fallback = if entry.kind == EntryKind::Header {
        "Header"
    } else {
        "Placeholder"
    };
    let title = unique_entry_title(
        if entry.title.is_empty() {
            fallback
        } else {
            &entry.title
        },
        existing,
    );
    let id = unique_entry_id(entry.kind, &title, existing);
    let mut copy = entry.clone();
    copy.raw["id"] = json!(id);
    copy.raw["title"] = json!(title);
    copy.id = id;
    copy.title = title;
    copy
}

/// `duplicatePlaylistEntry`: the copy goes directly after its source.
pub fn duplicate_entry(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: &str,
    entry_id: &str,
    index: usize,
) -> Option<Task<()>> {
    let playlist = runtime.content.read(cx).playlist(playlist_id)?.clone();
    let Some(source) = playlist
        .entries
        .get(index)
        .filter(|entry| entry.id == entry_id)
    else {
        close_document_menu(runtime, cx);
        return None;
    };
    let copy = copy_of(source, &playlist.entries);
    let mutation = json!({
        "type": "copyEntry", "sourcePlaylistId": playlist_id, "targetPlaylistId": playlist_id,
        "entryId": entry_id, "sourceEntryIndex": index, "expectedSourceEntry": source.raw,
    });
    let id = playlist_id.to_string();
    let request = Persist::new(
        Kind::Playlist,
        mutation,
        move |content: &mut Content, ui: &mut Ui| {
            if let Some(playlist) = content.playlists.iter_mut().find(|p| p.id == id) {
                playlist.entries.insert(index + 1, copy);
            }
            ui.selected_playlist_id = id.clone();
            ui.selected_documents.clear();
            ui.selected_document_source = Some(Source::Playlist);
            ui.selected_document_playlist_id = id;
            ui.document_anchor = None;
        },
    );
    let playlist_id = playlist_id.to_string();
    Some(spawn_persist(
        runtime,
        cx,
        request,
        move |runtime, response, cx| select_copied_entry(runtime, &playlist_id, &response, cx),
    ))
}

/// `applyCopiedEntrySelection`: selects the copy unless the user selected something meanwhile.
fn select_copied_entry(
    runtime: &Runtime,
    playlist_id: &str,
    response: &Option<Value>,
    cx: &mut App,
) {
    let Some(copied) = string_ids(response, "copiedEntryIds").into_iter().next() else {
        return;
    };
    let index = runtime
        .content
        .read(cx)
        .playlist(playlist_id)
        .and_then(|p| p.entries.iter().position(|entry| entry.id == copied));
    let Some(index) = index else { return };
    runtime.ui.update(cx, |ui, cx| {
        if ui.selected_playlist_id == playlist_id && ui.selected_documents.is_empty() {
            ui.selected_documents = vec![Selected {
                item_id: copied,
                index: Some(index),
            }];
            ui.selected_document_source = Some(Source::Playlist);
            ui.selected_document_playlist_id = playlist_id.to_string();
            ui.document_anchor = Some(Anchor {
                source: Source::Playlist,
                playlist_id: playlist_id.to_string(),
                index,
            });
            cx.notify();
        }
    });
}

/// `pastePlaylistEntry`: the clipboard entry goes after the entry the menu was opened on.
pub fn paste_entry(runtime: &Runtime, cx: &mut App, playlist_id: &str) -> Option<Task<()>> {
    let (playlist, clipboard, menu_index) = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        (
            content.playlist(playlist_id).cloned(),
            ui.clipboard.entry.clone(),
            ui.document_menu.as_ref().map(|menu| menu.index as i64),
        )
    };
    let (Some(playlist), Some(clipboard)) = (playlist, clipboard) else {
        close_document_menu(runtime, cx);
        return None;
    };
    let insert_at = ((menu_index.unwrap_or(-1) + 1).max(0) as usize).min(playlist.entries.len());
    let copy = copy_of(&clipboard, &playlist.entries);
    let mutation = merge(
        json!({ "type": "copyEntry", "targetPlaylistId": playlist_id, "entry": clipboard.raw }),
        entry_copy_anchor(&playlist.entries, insert_at),
    );
    let id = playlist_id.to_string();
    let request = Persist::new(
        Kind::Playlist,
        mutation,
        move |content: &mut Content, ui: &mut Ui| {
            if let Some(playlist) = content.playlists.iter_mut().find(|p| p.id == id) {
                playlist.entries.insert(insert_at, copy);
            }
            ui.selected_playlist_id = id.clone();
            ui.selected_documents.clear();
            ui.selected_document_source = Some(Source::Playlist);
            ui.selected_document_playlist_id = id;
            ui.document_anchor = None;
        },
    );
    let playlist_id = playlist_id.to_string();
    Some(spawn_persist(
        runtime,
        cx,
        request,
        move |runtime, response, cx| select_copied_entry(runtime, &playlist_id, &response, cx),
    ))
}

/// `addPlaylistEntryToPlaylist`: a copy of the menu's header or placeholder at the end of another playlist.
pub fn add_entry_to_playlist(runtime: &Runtime, cx: &mut App, target_id: &str) -> Option<Task<()>> {
    let (entry, source_playlist, source_index, target) = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        let entry = menu_entry(content, ui);
        (
            entry,
            ui.selected_playlist_id.clone(),
            ui.document_menu.as_ref().map(|menu| menu.index),
            content.playlist(target_id).cloned(),
        )
    };
    let (Some((_, entry)), Some(target)) = (entry, target.filter(|p| !p.is_folder())) else {
        close_document_menu(runtime, cx);
        return None;
    };
    let copy = copy_of(&entry, &target.entries);
    let anchored = source_index
        .and_then(|index| {
            runtime
                .content
                .read(cx)
                .playlist(&source_playlist)
                .and_then(|p| p.entries.get(index).map(|e| (index, e.clone())))
        })
        .filter(|(_, candidate)| candidate.id == entry.id);
    let mutation = match anchored {
        Some((index, source)) => merge(
            json!({
                "type": "copyEntry", "targetPlaylistId": target.id, "sourcePlaylistId": source_playlist,
                "entryId": entry.id, "sourceEntryIndex": index, "expectedSourceEntry": source.raw,
            }),
            entry_copy_anchor(&target.entries, target.entries.len()),
        ),
        None => merge(
            json!({ "type": "copyEntry", "targetPlaylistId": target.id, "entry": entry.raw }),
            entry_copy_anchor(&target.entries, target.entries.len()),
        ),
    };
    let target_id = target.id.clone();
    let request = Persist::new(
        Kind::Playlist,
        mutation,
        move |content: &mut Content, _: &mut Ui| {
            if let Some(playlist) = content.playlists.iter_mut().find(|p| p.id == target_id) {
                playlist.entries.push(copy);
            }
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// `handlePlaylistEntryMenuAction` for a header or placeholder row.
pub fn handle_entry_menu_action(runtime: &Runtime, cx: &mut App, action: &str) {
    let resolved = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        menu_entry(content, ui)
            .map(|(index, entry)| (index, entry, ui.selected_playlist_id.clone()))
    };
    let Some((index, entry, playlist_id)) = resolved else {
        return close_document_menu(runtime, cx);
    };
    match action {
        "hide" => set_entries_hidden(runtime, cx, &playlist_id, vec![entry.id], true),
        "copy" => runtime.ui.update(cx, |ui, cx| {
            ui.clipboard.entry = Some(entry);
            ui.clipboard.items.clear();
            ui.document_menu = None;
            cx.notify();
        }),
        "paste" => paste_entry(runtime, cx, &playlist_id).detach_task(),
        "duplicate" => duplicate_entry(runtime, cx, &playlist_id, &entry.id, index).detach_task(),
        "rename" => runtime.ui.update(cx, |ui, cx| {
            ui.rename_entry_id = entry.id.clone();
            ui.document_menu = None;
            cx.notify();
        }),
        "delete" => delete_entries_at(runtime, cx, &playlist_id, &[index]).detach_task(),
        _ => close_document_menu(runtime, cx),
    }
}

// ---- New presentation ----

/// `getNewPresentationValidationError`: the translation key of the first problem, if any.
pub fn new_presentation_error(content: &Content, draft: &NewPresentation) -> Option<&'static str> {
    if draft.title.trim().is_empty() {
        return Some("newPresentation.nameRequired");
    }
    if content.library(&draft.library_id).is_none() {
        return Some("newPresentation.libraryRequired");
    }
    if !draft.playlist_id.is_empty()
        && !content
            .playlist(&draft.playlist_id)
            .is_some_and(|playlist| !playlist.is_folder())
    {
        return Some("newPresentation.playlistInvalid");
    }
    if !super::tree_actions::theme_is_known(content, &draft.theme_id) {
        return Some("newPresentation.themeInvalid");
    }
    None
}

/// `createNewPresentation`: one native lifecycle mutation that returns the allocated identities.
pub fn create_presentation(
    runtime: &Runtime,
    cx: &mut App,
    draft: NewPresentation,
    after: impl FnOnce(bool, &mut App) + 'static,
) -> Task<()> {
    let mutation = json!({
        "type": "createPresentation",
        "libraryId": draft.library_id,
        "title": draft.title.trim(),
        "content": "",
        "splitMode": "paragraph",
        "splitMarker": "//",
        "splitUnits": 1,
        "presentationThemeId": draft.theme_id,
        "size": {"width":draft.size.0, "height":draft.size.1},
        "playlistId": (!draft.playlist_id.is_empty()).then_some(&draft.playlist_id),
    });
    let mut mutation = mutation;
    if draft.playlist_id.is_empty() {
        mutation.as_object_mut().map(|o| o.remove("playlistId"));
    }
    let library_id = draft.library_id.clone();
    let request = Persist::new(
        Kind::Library,
        mutation,
        move |_: &mut Content, ui: &mut Ui| {
            ui.selected_library_id = library_id;
            ui.create_menu = None;
        },
    );
    spawn_persist(runtime, cx, request, move |runtime, response, cx| {
        let created = response
            .as_ref()
            .and_then(|r| r["createdItemId"].as_str().map(str::to_string));
        if let Some(created) = &created {
            let playlist = draft.playlist_id.clone();
            let index = runtime.content.read(cx).playlist(&playlist).and_then(|p| {
                p.entries
                    .iter()
                    .position(|e| e.kind == EntryKind::Presentation && &e.item_id == created)
            });
            runtime.ui.update(cx, |ui, cx| {
                ui.selected_library_item_id = created.clone();
                ui.selected_playlist_id = playlist.clone();
                ui.selected_documents = vec![Selected {
                    item_id: created.clone(),
                    index,
                }];
                ui.selected_document_source = Some(if index.is_some() {
                    Source::Playlist
                } else {
                    Source::Library
                });
                ui.selected_document_playlist_id = playlist;
                cx.notify();
            });
        }
        after(created.is_some(), cx);
    })
}
