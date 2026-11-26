//! Library and playlist tree actions: create, rename, reorder, theme assignment, collapse, duplicate, and delete.
//! Ports of `collection-actions-controller.ts`, `collection-library-*.ts`, `collection-playlist-*.ts`,
//! `playlist-copy-delete-controller.ts`, and `collection-theme-assignment-actions.ts`. Each action applies its
//! optimistic change, sends one semantic mutation, and adopts the identities the host allocated.

use std::collections::HashSet;

use gpui::{App, Task};
use orbit_ui::Translator;
use serde_json::{json, Value};

use super::model::{
    copy_name, descendant_ids, insertion_anchor, is_descendant, unique_collection_id, unique_title,
    visible_ids, DropMode, Library, Playlist, ThemeAssignment,
};
use crate::state::runtime::{Kind, Persist, Runtime};
use crate::state::slices::{Content, Ui};

pub fn translator(cx: &App) -> Translator {
    *cx.global::<Translator>()
}

pub trait DetachExt {
    fn detach_task(self);
}

impl DetachExt for Option<Task<()>> {
    fn detach_task(self) {
        if let Some(task) = self {
            task.detach();
        }
    }
}

/// Runs one persisted mutation and then `after` with the host's response, on the foreground.
pub fn spawn_persist(
    runtime: &Runtime,
    cx: &mut App,
    request: Persist,
    after: impl FnOnce(&Runtime, Option<Value>, &mut App) + 'static,
) -> Task<()> {
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        let response = runtime.persist(cx, request).await;
        cx.update(|cx| after(&runtime, response, cx));
    })
}

fn ids_of(response: &Option<Value>, field: &str) -> Vec<String> {
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

fn id_of(response: &Option<Value>, field: &str) -> Option<String> {
    response
        .as_ref()
        .and_then(|response| response[field].as_str())
        .map(str::to_string)
}

// ---- Libraries ----

/// `createPresentationLibrary`.
pub fn create_library(runtime: &Runtime, cx: &mut App) -> Task<()> {
    let base_title = translator(cx).t("collections.default.newLibrary");
    let (title, id, position) = {
        let content = runtime.content.read(cx);
        let titles: Vec<&str> = content.libraries.iter().map(|l| l.title.as_str()).collect();
        let title = unique_title(&base_title, &titles);
        let ids: Vec<&str> = content.libraries.iter().map(|l| l.id.as_str()).collect();
        (
            title.clone(),
            unique_collection_id("library", &title, &ids),
            content.libraries.len(),
        )
    };
    let provisional = id.clone();
    let request = Persist::new(
        Kind::Library,
        json!({ "type": "create", "baseTitle": base_title }),
        move |content: &mut Content, ui: &mut Ui| {
            content.libraries.push(Library {
                id: id.clone(),
                title,
                item_ids: Vec::new(),
                theme: ThemeAssignment::Inherit,
            });
            ui.selected_library_id = id.clone();
            ui.selected_library_ids = vec![id.clone()];
            ui.library_anchor = Some(position);
            ui.selected_playlist_id.clear();
            ui.rename_library_id = id;
            ui.create_menu = None;
        },
    );
    spawn_persist(runtime, cx, request, move |runtime, response, cx| {
        let Some(created) = id_of(&response, "createdLibraryId") else {
            return;
        };
        runtime.ui.update(cx, |ui, cx| {
            // A newer selection or rename wins over the allocated identity.
            if ui.selected_library_id == provisional && ui.rename_library_id == provisional {
                ui.selected_library_id = created.clone();
                ui.selected_library_ids = vec![created.clone()];
                ui.rename_library_id = created;
                ui.create_menu = None;
                cx.notify();
            }
        });
    })
}

/// `commitCollectionLibraryTitle`.
pub fn commit_library_title(
    runtime: &Runtime,
    cx: &mut App,
    library_id: &str,
    raw_title: &str,
) -> Option<Task<()>> {
    let title = raw_title.trim().to_string();
    let unchanged = runtime
        .content
        .read(cx)
        .library(library_id)
        .is_none_or(|library| title.is_empty() || title == library.title);
    if unchanged {
        runtime.ui.update(cx, |ui, cx| {
            ui.rename_library_id.clear();
            cx.notify();
        });
        return None;
    }
    let id = library_id.to_string();
    let request = Persist::new(
        Kind::Library,
        json!({ "type": "rename", "libraryId": library_id, "title": title }),
        {
            let title = title.clone();
            move |content: &mut Content, ui: &mut Ui| {
                if let Some(library) = content.libraries.iter_mut().find(|l| l.id == id) {
                    library.title = title;
                }
                ui.rename_library_id.clear();
                ui.library_menu = None;
            }
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// `reorderCollectionLibraries`.
pub fn reordered_libraries(
    libraries: &[Library],
    dragged: &str,
    target: &str,
    after: bool,
) -> Option<Vec<Library>> {
    let mut reordered = libraries.to_vec();
    let dragged_index = reordered.iter().position(|l| l.id == dragged)?;
    reordered.iter().position(|l| l.id == target)?;
    let moved = reordered.remove(dragged_index);
    let target_index = reordered.iter().position(|l| l.id == target)?;
    reordered.insert(
        if after {
            target_index + 1
        } else {
            target_index
        },
        moved,
    );
    Some(reordered)
}

pub fn reorder_libraries(
    runtime: &Runtime,
    cx: &mut App,
    dragged: &str,
    target: &str,
    after: bool,
) -> Option<Task<()>> {
    reordered_libraries(&runtime.content.read(cx).libraries, dragged, target, after)?;
    let (dragged, target) = (dragged.to_string(), target.to_string());
    Some(runtime.save_preview(
        cx,
        Kind::Library,
        json!({
            "type": "reorder",
            "libraryId": dragged,
            "targetLibraryId": target,
            "placement": if after { "after" } else { "before" },
        }),
        move |content| {
            if let Some(reordered) =
                reordered_libraries(&content.libraries, &dragged, &target, after)
            {
                content.libraries = reordered;
            }
        },
    ))
}

/// The message of the delete confirmation: how many libraries and how many documents go with them.
pub fn delete_libraries_message(t: Translator, library_count: usize, item_count: usize) -> String {
    let count = item_count.to_string();
    let detail = match item_count {
        0 => String::new(),
        1 => t.format("collections.deleteLibraries.itemDetail.one", &[]),
        _ => t.format(
            "collections.deleteLibraries.itemDetail.many",
            &[("count", &count)],
        ),
    };
    let libraries = library_count.to_string();
    let key = if library_count == 1 {
        "collections.deleteLibraries.message.one"
    } else {
        "collections.deleteLibraries.message.many"
    };
    t.format(key, &[("count", &libraries), ("itemDetail", &detail)])
}

/// The documents that deleting these libraries removes.
pub fn deleted_item_ids(content: &Content, library_ids: &[String]) -> HashSet<String> {
    content
        .libraries
        .iter()
        .filter(|library| library_ids.contains(&library.id))
        .flat_map(|library| library.item_ids.iter().cloned())
        .collect()
}

/// `deleteLibraries` after the user confirmed.
pub fn delete_libraries(runtime: &Runtime, cx: &mut App, library_ids: Vec<String>) -> Task<()> {
    let deleted_libraries: HashSet<String> = library_ids.iter().cloned().collect();
    let deleted_items = deleted_item_ids(runtime.content.read(cx), &library_ids);
    let optimistic_items = deleted_items.clone();
    let request = Persist::new(
        Kind::Library,
        json!({ "type": "delete", "libraryIds": library_ids }),
        move |content: &mut Content, ui: &mut Ui| {
            content
                .libraries
                .retain(|library| !deleted_libraries.contains(&library.id));
            for library in &mut content.libraries {
                library.item_ids.retain(|id| !optimistic_items.contains(id));
            }
            let items = content.items.without(&optimistic_items);
            content.replace_items(items);
            remove_items_from_playlists(content, &optimistic_items);
            let selected_library =
                if deleted_libraries_contains(&ui.selected_library_id, &content.libraries) {
                    ui.selected_library_id.clone()
                } else {
                    content
                        .libraries
                        .first()
                        .map(|l| l.id.clone())
                        .unwrap_or_default()
                };
            let selected_item = if optimistic_items.contains(&ui.selected_library_item_id) {
                content
                    .library(&selected_library)
                    .and_then(|library| {
                        library
                            .item_ids
                            .iter()
                            .find(|id| !optimistic_items.contains(*id))
                    })
                    .cloned()
                    .unwrap_or_default()
            } else {
                ui.selected_library_item_id.clone()
            };
            ui.selected_library_id = selected_library.clone();
            ui.selected_library_item_id = selected_item;
            ui.selected_playlist_id.clear();
            ui.selected_library_ids = if selected_library.is_empty() {
                vec![]
            } else {
                vec![selected_library]
            };
            ui.library_anchor = None;
            ui.selected_documents.clear();
            ui.document_anchor = None;
            ui.library_menu = None;
            ui.rename_library_id.clear();
        },
    );
    spawn_persist(runtime, cx, request, |runtime, response, cx| {
        if response.is_none() {
            return;
        }
        let deleted_items: HashSet<String> =
            ids_of(&response, "deletedItemIds").into_iter().collect();
        let content = runtime.content.read(cx);
        let (selected_library, selected_item) = {
            let ui = runtime.ui.read(cx);
            let library_invalid = content.library(&ui.selected_library_id).is_none();
            let library = if library_invalid {
                content
                    .libraries
                    .first()
                    .map(|l| l.id.clone())
                    .unwrap_or_default()
            } else {
                ui.selected_library_id.clone()
            };
            let item_invalid = !content.items.contains(&ui.selected_library_item_id)
                || deleted_items.contains(&ui.selected_library_item_id);
            let item = if item_invalid {
                content
                    .library(&library)
                    .and_then(|l| l.item_ids.iter().find(|id| !deleted_items.contains(*id)))
                    .cloned()
                    .unwrap_or_default()
            } else {
                ui.selected_library_item_id.clone()
            };
            (library, item)
        };
        runtime.ui.update(cx, |ui, cx| {
            let invalid = ui.selected_library_id != selected_library;
            ui.selected_library_id = selected_library.clone();
            if invalid {
                ui.selected_library_ids = if selected_library.is_empty() {
                    vec![]
                } else {
                    vec![selected_library]
                };
            }
            ui.selected_library_item_id = selected_item;
            cx.notify();
        });
    })
}

fn deleted_libraries_contains(id: &str, remaining: &[Library]) -> bool {
    remaining.iter().any(|library| library.id == id)
}

fn remove_items_from_playlists(content: &mut Content, removed: &HashSet<String>) {
    for playlist in &mut content.playlists {
        playlist
            .entries
            .retain(|entry| entry.item_id.is_empty() || !removed.contains(&entry.item_id));
    }
}

/// `assignThemeToLibraries`: only a known theme layout can be assigned to a library.
pub fn assign_theme_to_libraries(
    runtime: &Runtime,
    cx: &mut App,
    library_ids: Vec<String>,
    assignment: ThemeAssignment,
) -> Option<Task<()>> {
    let known = matches!(&assignment, ThemeAssignment::Theme(id) if theme_is_known(runtime.content.read(cx), id));
    if !known {
        runtime.ui.update(cx, |ui, cx| {
            ui.library_menu = None;
            cx.notify();
        });
        return None;
    }
    let request = Persist::new(
        Kind::Library,
        json!({ "type": "setTheme", "libraryIds": library_ids, "assignment": assignment.mutation_value() }),
        move |content: &mut Content, ui: &mut Ui| {
            for library in content
                .libraries
                .iter_mut()
                .filter(|l| library_ids.contains(&l.id))
            {
                library.theme = assignment.clone();
            }
            ui.selected_library_ids = library_ids;
            ui.library_menu = None;
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// `isKnownPresentationThemeId`: `<theme id>::<slide id>` names a slide of a theme.
pub fn theme_is_known(content: &Content, selection: &str) -> bool {
    let Some((theme_id, slide_id)) = selection
        .split_once("::")
        .filter(|(theme, _)| !theme.is_empty())
    else {
        return false;
    };
    content
        .themes
        .iter()
        .find(|theme| theme.id == theme_id)
        .is_some_and(|theme| theme.slides.iter().any(|(id, _)| id == slide_id))
}

// ---- Playlists ----

/// `createPresentationPlaylist`.
pub fn create_playlist(
    runtime: &Runtime,
    cx: &mut App,
    base_title: String,
    kind: Option<&str>,
) -> Task<()> {
    let (title, id, position) = {
        let content = runtime.content.read(cx);
        let titles: Vec<&str> = content.playlists.iter().map(|p| p.title.as_str()).collect();
        let title = unique_title(&base_title, &titles);
        let ids: Vec<&str> = content.playlists.iter().map(|p| p.id.as_str()).collect();
        (
            title.clone(),
            unique_collection_id("playlist", &title, &ids),
            content.playlists.len(),
        )
    };
    let provisional = id.clone();
    let mut mutation = json!({ "type": "create", "baseTitle": base_title });
    // The template carries what the playlist is made from, as `playlistData` does in the TypeScript controller.
    let mut template = json!({ "title": base_title });
    if let Some(kind) = kind {
        mutation["playlistType"] = json!(kind);
        template["type"] = json!(kind);
    }
    mutation["template"] = template;
    let kind_name = kind.unwrap_or("playlist").to_string();
    let request = Persist::new(
        Kind::Playlist,
        mutation,
        move |content: &mut Content, ui: &mut Ui| {
            content.playlists.push(Playlist {
                id: id.clone(),
                title,
                kind: kind_name,
                parent_id: String::new(),
                collapsed: false,
                theme: ThemeAssignment::Inherit,
                entries: Vec::new(),
            });
            ui.selected_playlist_id = id.clone();
            ui.selected_playlist_ids = vec![id.clone()];
            ui.playlist_anchor = Some(position);
            ui.rename_playlist_id = id;
            ui.create_menu = None;
        },
    );
    spawn_persist(runtime, cx, request, move |runtime, response, cx| {
        let Some(created) = ids_of(&response, "createdPlaylistIds").into_iter().next() else {
            return;
        };
        let first_item = runtime
            .content
            .read(cx)
            .playlist(&created)
            .and_then(|playlist| playlist.visible_item_ids().into_iter().next());
        runtime.ui.update(cx, |ui, cx| {
            if ui.selected_playlist_id == provisional && ui.rename_playlist_id == provisional {
                ui.selected_playlist_id = created.clone();
                ui.selected_playlist_ids = vec![created.clone()];
                if let Some(item) = first_item {
                    ui.selected_library_item_id = item;
                }
                ui.rename_playlist_id = created;
                ui.create_menu = None;
                cx.notify();
            }
        });
    })
}

/// `commitPresentationPlaylistTitle`.
pub fn commit_playlist_title(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: &str,
    raw_title: &str,
) -> Option<Task<()>> {
    let title = raw_title.trim().to_string();
    let unchanged = runtime
        .content
        .read(cx)
        .playlist(playlist_id)
        .is_none_or(|playlist| title.is_empty() || title == playlist.title);
    if unchanged {
        runtime.ui.update(cx, |ui, cx| {
            ui.rename_playlist_id.clear();
            cx.notify();
        });
        return None;
    }
    let id = playlist_id.to_string();
    let request = Persist::new(
        Kind::Playlist,
        json!({ "type": "rename", "playlistId": playlist_id, "title": title }),
        {
            let title = title.clone();
            move |content: &mut Content, ui: &mut Ui| {
                if let Some(playlist) = content.playlists.iter_mut().find(|p| p.id == id) {
                    playlist.title = title;
                }
                ui.rename_playlist_id.clear();
            }
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// The playlist order after dropping `dragged` on `target`, or `None` for an invalid drop.
pub fn reordered_playlists(
    playlists: &[Playlist],
    dragged: &str,
    target: &str,
    mode: DropMode,
) -> Option<Vec<Playlist>> {
    let dragged_index = playlists.iter().position(|p| p.id == dragged)?;
    let target_playlist = playlists.iter().find(|p| p.id == target)?;
    if dragged == target || is_descendant(playlists, target, dragged) {
        return None;
    }
    if mode == DropMode::Inside {
        if !target_playlist.is_folder() {
            return None;
        }
        let mut next = playlists.to_vec();
        next[dragged_index].parent_id = target.to_string();
        return Some(next);
    }
    let parent = if mode == DropMode::AfterRoot {
        String::new()
    } else {
        target_playlist.parent_id.clone()
    };
    let mut moved = playlists[dragged_index].clone();
    moved.parent_id = parent;
    let mut without: Vec<Playlist> = playlists.to_vec();
    without.remove(dragged_index);
    let visible = visible_ids(&without);
    let anchor = insertion_anchor(&without, target, mode);
    let anchor_index = without.iter().position(|p| p.id == anchor)?;
    if !visible.iter().any(|id| id == target) {
        return None;
    }
    let at = if matches!(mode, DropMode::After | DropMode::AfterRoot) {
        anchor_index + 1
    } else {
        anchor_index
    };
    without.insert(at, moved);
    Some(without)
}

pub fn reorder_playlists(
    runtime: &Runtime,
    cx: &mut App,
    dragged: &str,
    target: &str,
    mode: DropMode,
) -> Option<Task<()>> {
    reordered_playlists(&runtime.content.read(cx).playlists, dragged, target, mode)?;
    let (dragged, target) = (dragged.to_string(), target.to_string());
    Some(runtime.save_preview(
        cx,
        Kind::Playlist,
        json!({ "type": "reorder", "playlistId": dragged, "targetPlaylistId": target, "placement": mode.as_str() }),
        move |content| {
            if let Some(next) = reordered_playlists(&content.playlists, &dragged, &target, mode) {
                content.playlists = next;
            }
        },
    ))
}

/// `movePlaylistToParent`: an empty `parent_id` moves it to the root.
pub fn move_playlist_to_parent(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: &str,
    parent_id: &str,
) -> Option<Task<()>> {
    {
        let playlists = &runtime.content.read(cx).playlists;
        if !playlists.iter().any(|p| p.id == playlist_id)
            || playlist_id == parent_id
            || is_descendant(playlists, parent_id, playlist_id)
        {
            return None;
        }
    }
    let (id, parent) = (playlist_id.to_string(), parent_id.to_string());
    Some(runtime.save_preview(
        cx,
        Kind::Playlist,
        json!({ "type": "moveParent", "playlistId": playlist_id, "parentId": parent_id }),
        move |content| {
            if !is_descendant(&content.playlists, &parent, &id) {
                if let Some(playlist) = content.playlists.iter_mut().find(|p| p.id == id) {
                    playlist.parent_id.clone_from(&parent);
                }
            }
        },
    ))
}

pub fn toggle_playlist_collapsed(
    runtime: &Runtime,
    cx: &mut App,
    playlist_id: &str,
) -> Option<Task<()>> {
    let collapsed = !runtime.content.read(cx).playlist(playlist_id)?.collapsed;
    let id = playlist_id.to_string();
    let request = Persist::new(
        Kind::Playlist,
        json!({ "type": "setCollapsed", "playlistId": playlist_id, "collapsed": collapsed }),
        move |content: &mut Content, _ui: &mut Ui| {
            if let Some(playlist) = content.playlists.iter_mut().find(|p| p.id == id) {
                playlist.collapsed = collapsed;
            }
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// `assignThemeToPlaylists`: a playlist can also inherit or have no theme.
pub fn assign_theme_to_playlists(
    runtime: &Runtime,
    cx: &mut App,
    playlist_ids: Vec<String>,
    assignment: ThemeAssignment,
) -> Option<Task<()>> {
    let valid = match &assignment {
        ThemeAssignment::Theme(id) => theme_is_known(runtime.content.read(cx), id),
        _ => true,
    };
    if !valid {
        runtime.ui.update(cx, |ui, cx| {
            ui.playlist_menu = None;
            cx.notify();
        });
        return None;
    }
    let request = Persist::new(
        Kind::Playlist,
        json!({ "type": "setTheme", "playlistIds": playlist_ids, "assignment": assignment.mutation_value() }),
        move |content: &mut Content, ui: &mut Ui| {
            for playlist in content
                .playlists
                .iter_mut()
                .filter(|p| playlist_ids.contains(&p.id))
            {
                playlist.theme = assignment.clone();
            }
            ui.selected_playlist_ids = playlist_ids;
            ui.playlist_menu = None;
        },
    );
    Some(spawn_persist(runtime, cx, request, |_, _, _| {}))
}

/// `duplicatePlaylist` and `duplicatePlaylists`: each copy directly follows its source.
pub fn duplicate_playlists(
    runtime: &Runtime,
    cx: &mut App,
    playlist_ids: Vec<String>,
) -> Option<Task<()>> {
    let fallback = translator(cx).t("collections.default.playlist");
    let (next, copies) = {
        let playlists = &runtime.content.read(cx).playlists;
        if playlist_ids
            .iter()
            .all(|id| !playlists.iter().any(|p| &p.id == id))
        {
            return None;
        }
        let mut next = playlists.clone();
        let mut copies: Vec<(String, String)> = Vec::new();
        let sources: Vec<Playlist> = playlists
            .iter()
            .filter(|p| playlist_ids.contains(&p.id))
            .cloned()
            .collect();
        for source in sources {
            let titles: Vec<&str> = next.iter().map(|p| p.title.as_str()).collect();
            let title = copy_name(
                if source.title.is_empty() {
                    &fallback
                } else {
                    &source.title
                },
                &titles,
            );
            let ids: Vec<&str> = next.iter().map(|p| p.id.as_str()).collect();
            let id = unique_collection_id("playlist", &title, &ids);
            let mut copy = source.clone();
            copy.id = id.clone();
            copy.title = title;
            let at = next
                .iter()
                .position(|p| p.id == source.id)
                .map_or(next.len(), |i| i + 1);
            next.insert(at, copy);
            copies.push((source.id.clone(), id));
        }
        (next, copies)
    };
    // The copies in the order the user asked for them.
    let copy_ids: Vec<String> = playlist_ids
        .iter()
        .filter_map(|id| {
            copies
                .iter()
                .find(|(source, _)| source == id)
                .map(|(_, copy)| copy.clone())
        })
        .collect();
    let provisional = copy_ids.clone();
    let first_item = next
        .iter()
        .find(|p| copy_ids.first() == Some(&p.id))
        .and_then(|p| p.visible_item_ids().into_iter().next());
    let request = Persist::new(
        Kind::Playlist,
        json!({ "type": "duplicate", "playlistIds": playlist_ids, "fallbackTitle": fallback }),
        move |content: &mut Content, ui: &mut Ui| {
            content.playlists = next;
            if let Some(first) = copy_ids.first() {
                ui.selected_playlist_id = first.clone();
            }
            if let Some(item) = first_item {
                ui.selected_library_item_id = item;
            }
            ui.selected_playlist_ids = copy_ids;
            ui.playlist_anchor = None;
            ui.playlist_menu = None;
        },
    );
    Some(spawn_persist(
        runtime,
        cx,
        request,
        move |runtime, response, cx| {
            let created = ids_of(&response, "createdPlaylistIds");
            if created.is_empty() {
                return;
            }
            let first_item = runtime
                .content
                .read(cx)
                .playlist(&created[0])
                .and_then(|p| p.visible_item_ids().into_iter().next());
            runtime.ui.update(cx, |ui, cx| {
                if !provisional.contains(&ui.selected_playlist_id) {
                    return;
                }
                ui.selected_playlist_id = created[0].clone();
                ui.selected_playlist_ids = created;
                if let Some(item) = first_item {
                    ui.selected_library_item_id = item;
                }
                cx.notify();
            });
        },
    ))
}

/// `deletePlaylists`: the playlists and everything below them.
pub fn delete_playlists(runtime: &Runtime, cx: &mut App, playlist_ids: Vec<String>) -> Task<()> {
    let deleted: HashSet<String> = {
        let playlists = &runtime.content.read(cx).playlists;
        playlist_ids
            .iter()
            .flat_map(|id| descendant_ids(playlists, id))
            .collect()
    };
    let optimistic = deleted.clone();
    let request = Persist::new(
        Kind::Playlist,
        json!({ "type": "delete", "playlistIds": playlist_ids }),
        move |content: &mut Content, ui: &mut Ui| {
            content.playlists.retain(|p| !optimistic.contains(&p.id));
            if optimistic.contains(&ui.selected_playlist_id) {
                ui.selected_playlist_id.clear();
            }
            ui.selected_playlist_ids = if ui.selected_playlist_id.is_empty() {
                vec![]
            } else {
                vec![ui.selected_playlist_id.clone()]
            };
            ui.playlist_anchor = None;
            ui.selected_documents.clear();
            ui.document_anchor = None;
            ui.playlist_menu = None;
        },
    );
    spawn_persist(runtime, cx, request, move |runtime, response, cx| {
        if response.is_none() {
            return;
        }
        let mut deleted_ids = ids_of(&response, "deletedPlaylistIds");
        if deleted_ids.is_empty() {
            deleted_ids = deleted.into_iter().collect();
        }
        runtime.ui.update(cx, |ui, cx| {
            if deleted_ids.contains(&ui.selected_playlist_id) {
                ui.selected_playlist_id.clear();
                ui.selected_playlist_ids.clear();
                cx.notify();
            }
        });
    })
}

/// The ids a library context menu acts on (`getLibraryMenuIds`).
pub fn library_menu_ids(content: &Content, ui: &Ui, fallback: &str) -> Vec<String> {
    let menu_ids = ui
        .library_menu
        .as_ref()
        .map(|menu| menu.ids.clone())
        .unwrap_or_default();
    let ids = if menu_ids.is_empty() {
        vec![fallback.to_string()]
    } else {
        menu_ids
    };
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| content.library(id).is_some() && seen.insert(id.clone()))
        .collect()
}

pub fn playlist_menu_ids(content: &Content, ui: &Ui, fallback: &str) -> Vec<String> {
    let menu_ids = ui
        .playlist_menu
        .as_ref()
        .map(|menu| menu.ids.clone())
        .unwrap_or_default();
    let ids = if menu_ids.is_empty() {
        vec![fallback.to_string()]
    } else {
        menu_ids
    };
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| content.playlist(id).is_some() && seen.insert(id.clone()))
        .collect()
}

/// What a library context menu action does, before any confirmation.
pub enum LibraryMenuOutcome {
    Done,
    /// The user must confirm deleting these libraries first.
    ConfirmDelete(Vec<String>),
}

/// `handleLibraryMenuAction`.
pub fn handle_library_menu_action(
    runtime: &Runtime,
    cx: &mut App,
    action: &str,
    library_id: &str,
) -> LibraryMenuOutcome {
    let (ids, exists) = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        (
            library_menu_ids(content, ui, library_id),
            content.library(library_id).is_some(),
        )
    };
    if !exists || ids.is_empty() {
        runtime.ui.update(cx, |ui, cx| {
            ui.library_menu = None;
            cx.notify();
        });
        return LibraryMenuOutcome::Done;
    }
    if let Some(assignment) = ThemeAssignment::parse_action(action) {
        assign_theme_to_libraries(runtime, cx, ids, assignment).detach_task();
        return LibraryMenuOutcome::Done;
    }
    match action {
        "rename" => {
            runtime.ui.update(cx, |ui, cx| {
                ui.rename_library_id = library_id.to_string();
                ui.library_menu = None;
                ui.selected_library_ids = vec![library_id.to_string()];
                cx.notify();
            });
            LibraryMenuOutcome::Done
        }
        "delete" => {
            runtime.ui.update(cx, |ui, cx| {
                ui.library_menu = None;
                cx.notify();
            });
            LibraryMenuOutcome::ConfirmDelete(ids)
        }
        _ => LibraryMenuOutcome::Done,
    }
}

/// `handlePlaylistMenuAction`.
pub fn handle_playlist_menu_action(
    runtime: &Runtime,
    cx: &mut App,
    action: &str,
    playlist_id: &str,
) {
    let (ids, exists) = {
        let (content, ui) = (runtime.content.read(cx), runtime.ui.read(cx));
        (
            playlist_menu_ids(content, ui, playlist_id),
            content.playlist(playlist_id).is_some(),
        )
    };
    let close = |runtime: &Runtime, cx: &mut App| {
        runtime.ui.update(cx, |ui, cx| {
            ui.playlist_menu = None;
            cx.notify();
        });
    };
    if !exists || ids.is_empty() {
        return close(runtime, cx);
    }
    let theme = ThemeAssignment::parse_action(action);
    let multi = ids.len() > 1;
    if multi && theme.is_none() && !["duplicate", "delete"].contains(&action) {
        return close(runtime, cx);
    }
    if let Some(assignment) = theme {
        assign_theme_to_playlists(runtime, cx, ids, assignment).detach_task();
        return;
    }
    match action {
        "rename" => runtime.ui.update(cx, |ui, cx| {
            ui.rename_playlist_id = playlist_id.to_string();
            ui.playlist_menu = None;
            cx.notify();
        }),
        "duplicate" => {
            duplicate_playlists(runtime, cx, ids).detach_task();
        }
        "delete" => delete_playlists(runtime, cx, ids).detach(),
        _ => close(runtime, cx),
    }
}
