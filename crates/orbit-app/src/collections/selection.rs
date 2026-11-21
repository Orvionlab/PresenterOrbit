//! Selection in the library tree, the playlist tree, and the document list: `collection-selection-utils.ts`,
//! `collection-selection-controller.ts`, and the `library:selected`, `playlist:item-selected`, and item selection
//! handlers of the library and playlist modules (no cue editor is open in this phase).

use super::model::{EntryKind, Playlist};
use crate::state::slices::{Anchor, Content, Selected, Source, Ui};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub range: bool,
    pub toggle: bool,
}

impl From<gpui::Modifiers> for Modifiers {
    fn from(modifiers: gpui::Modifiers) -> Self {
        Self {
            range: modifiers.shift,
            toggle: modifiers.control || modifiers.platform,
        }
    }
}

/// The ids the document list shows, in order: a playlist's entries (presentation ids, entry ids otherwise), or
/// the selected library's documents sorted by title.
pub fn active_document_ids(content: &Content, ui: &Ui) -> Vec<String> {
    if let Some(playlist) = content.playlist(&ui.selected_playlist_id) {
        return playlist_document_ids(content, playlist);
    }
    content
        .library(&ui.selected_library_id)
        .map(|library| content.sorted_document_ids(library).to_vec())
        .unwrap_or_default()
}

/// `getPlaylistDocumentEntries` reduced to row ids: a presentation whose document is missing has no row.
pub fn playlist_document_ids(content: &Content, playlist: &Playlist) -> Vec<String> {
    playlist
        .entries
        .iter()
        .filter_map(|entry| match entry.kind {
            EntryKind::Presentation => content
                .items
                .contains(&entry.item_id)
                .then(|| entry.item_id.clone()),
            _ => Some(entry.id.clone()),
        })
        .collect()
}

fn current_document_selection<'a>(ui: &'a Ui, source: Source, playlist_id: &str) -> &'a [Selected] {
    if ui.selected_document_source != Some(source) {
        return &[];
    }
    if source == Source::Playlist && ui.selected_document_playlist_id != playlist_id {
        return &[];
    }
    &ui.selected_documents
}

fn selection_entry(item_id: &str, index: usize, source: Source) -> Selected {
    Selected {
        item_id: item_id.to_string(),
        index: (source == Source::Playlist).then_some(index),
    }
}

fn same_entry(first: &Selected, second: &Selected, source: Source) -> bool {
    first.item_id == second.item_id && (source == Source::Library || first.index == second.index)
}

pub struct NextDocumentSelection {
    pub entries: Vec<Selected>,
    pub anchor: Anchor,
}

/// `getNextDocumentSelection`.
pub fn next_document_selection(
    ui: &Ui,
    item_id: &str,
    index: usize,
    source: Source,
    playlist_id: &str,
    modifiers: Modifiers,
    document_ids: &[String],
) -> NextDocumentSelection {
    let current = current_document_selection(ui, source, playlist_id);
    let entry = selection_entry(item_id, index, source);
    let anchor = match &ui.document_anchor {
        Some(anchor)
            if anchor.source == source
                && (source != Source::Playlist || anchor.playlist_id == playlist_id) =>
        {
            anchor.clone()
        }
        _ => Anchor {
            source,
            playlist_id: playlist_id.to_string(),
            index,
        },
    };
    let own_anchor = Anchor {
        source,
        playlist_id: playlist_id.to_string(),
        index,
    };

    if modifiers.range {
        let start = anchor.index.min(index);
        let end = anchor.index.max(index);
        let entries = document_ids
            .iter()
            .enumerate()
            .skip(start)
            .take(end + 1 - start)
            .map(|(position, id)| selection_entry(id, position, source))
            .collect();
        return NextDocumentSelection { entries, anchor };
    }
    if modifiers.toggle {
        let exists = current
            .iter()
            .any(|selected| same_entry(selected, &entry, source));
        let mut entries: Vec<Selected> = if exists {
            current
                .iter()
                .filter(|selected| !same_entry(selected, &entry, source))
                .cloned()
                .collect()
        } else {
            current.iter().cloned().chain([entry.clone()]).collect()
        };
        if entries.is_empty() {
            entries.push(entry);
        }
        return NextDocumentSelection {
            entries,
            anchor: own_anchor,
        };
    }
    NextDocumentSelection {
        entries: vec![entry],
        anchor: own_anchor,
    }
}

/// `isDocumentRowSelected`.
pub fn is_document_row_selected(ui: &Ui, item_id: &str, keep_playlist: bool, index: usize) -> bool {
    let source = if keep_playlist {
        Source::Playlist
    } else {
        Source::Library
    };
    let playlist_id = if keep_playlist {
        ui.selected_playlist_id.as_str()
    } else {
        ""
    };
    let selection = current_document_selection(ui, source, playlist_id);
    if selection.is_empty() {
        return ui.selected_library_item_id == item_id;
    }
    let wanted = selection_entry(item_id, index, source);
    selection
        .iter()
        .any(|selected| same_entry(selected, &wanted, source))
}

/// The selected library ids that still exist.
pub fn selected_library_ids(content: &Content, ui: &Ui) -> Vec<String> {
    ui.selected_library_ids
        .iter()
        .filter(|id| content.library(id).is_some())
        .cloned()
        .collect()
}

pub fn selected_playlist_ids(content: &Content, ui: &Ui) -> Vec<String> {
    ui.selected_playlist_ids
        .iter()
        .filter(|id| content.playlist(id).is_some())
        .cloned()
        .collect()
}

/// `isLibraryTreeItemSelected`. The active selection is hidden while a playlist is selected.
pub fn is_library_selected(content: &Content, ui: &Ui, id: &str, show_active: bool) -> bool {
    if !show_active {
        return false;
    }
    let selected = selected_library_ids(content, ui);
    if selected.is_empty() {
        return ui.selected_library_id == id;
    }
    selected.iter().any(|candidate| candidate == id)
}

/// `isPlaylistTreeItemSelected`.
pub fn is_playlist_selected(content: &Content, ui: &Ui, id: &str) -> bool {
    let selected = selected_playlist_ids(content, ui);
    if !selected.is_empty() {
        return selected.iter().any(|candidate| candidate == id);
    }
    if ui.selected_document_source == Some(Source::Playlist)
        && ui.selected_document_playlist_id == ui.selected_playlist_id
        && !ui.selected_documents.is_empty()
    {
        return false;
    }
    ui.selected_playlist_id == id
}

fn next_tree_selection(
    ids: &[&str],
    current: Vec<String>,
    anchor: Option<usize>,
    id: &str,
    index: usize,
    modifiers: Modifiers,
) -> Vec<String> {
    if modifiers.range {
        let anchor = anchor.unwrap_or(index);
        let start = anchor.min(index);
        let end = anchor.max(index).min(ids.len().saturating_sub(1));
        return ids
            .iter()
            .enumerate()
            .filter(|(position, _)| (start..=end).contains(position))
            .map(|(_, id)| id.to_string())
            .collect();
    }
    if modifiers.toggle {
        let next: Vec<String> = if current.iter().any(|selected| selected == id) {
            current
                .into_iter()
                .filter(|selected| selected != id)
                .collect()
        } else {
            current.into_iter().chain([id.to_string()]).collect()
        };
        return if next.is_empty() {
            vec![id.to_string()]
        } else {
            next
        };
    }
    vec![id.to_string()]
}

/// `selectLibraryTreeItem` and the `library:selected` handler. `index` falls back to the position of the id.
pub fn select_library(
    content: &Content,
    ui: &mut Ui,
    id: &str,
    index: Option<usize>,
    modifiers: Modifiers,
) {
    let clicked = index.or_else(|| {
        content
            .libraries
            .iter()
            .position(|library| library.id == id)
    });
    let ids: Vec<&str> = content
        .libraries
        .iter()
        .map(|library| library.id.as_str())
        .collect();
    let selected = next_tree_selection(
        &ids,
        selected_library_ids(content, ui),
        ui.library_anchor,
        id,
        clicked.unwrap_or(0),
        modifiers,
    );
    ui.selected_library_ids = selected;
    ui.library_anchor = clicked;
    ui.selected_documents.clear();
    ui.document_anchor = None;
    ui.library_menu = None;
    ui.selected_playlist_ids.clear();
    ui.playlist_anchor = None;
    ui.playlist_menu = None;
    // The library module's `library:selected` handler.
    ui.selected_library_id = id.to_string();
    if !ui
        .selected_library_ids
        .iter()
        .any(|selected| selected == id)
    {
        ui.selected_library_ids = vec![id.to_string()];
    }
    ui.selected_playlist_id.clear();
}

/// `selectPlaylistTreeItem` and the `playlist:item-selected` handler.
pub fn select_playlist(
    content: &Content,
    ui: &mut Ui,
    id: &str,
    index: Option<usize>,
    modifiers: Modifiers,
) {
    let clicked = index.or_else(|| {
        content
            .playlists
            .iter()
            .position(|playlist| playlist.id == id)
    });
    let ids: Vec<&str> = content
        .playlists
        .iter()
        .map(|playlist| playlist.id.as_str())
        .collect();
    let selected = next_tree_selection(
        &ids,
        selected_playlist_ids(content, ui),
        ui.playlist_anchor,
        id,
        clicked.unwrap_or(0),
        modifiers,
    );
    ui.selected_documents.clear();
    ui.document_anchor = None;
    ui.selected_library_ids.clear();
    ui.library_anchor = None;
    ui.library_menu = None;
    ui.selected_playlist_ids = selected;
    ui.playlist_anchor = clicked;
    ui.playlist_menu = None;
    // The playlist module's `playlist:item-selected` handler.
    ui.selected_playlist_id = id.to_string();
    if !ui
        .selected_playlist_ids
        .iter()
        .any(|selected| selected == id)
    {
        ui.selected_playlist_ids = vec![id.to_string()];
    }
    if let Some(first) = content.playlist(id).and_then(|playlist| {
        playlist
            .visible_item_ids()
            .into_iter()
            .find(|item| content.items.contains(item))
    }) {
        ui.selected_library_item_id = first;
    }
}

/// `handleDocumentRowClick` for a library row (`keep_playlist` false) or a playlist presentation row.
pub fn click_document_row(
    content: &Content,
    ui: &mut Ui,
    item_id: &str,
    keep_playlist: bool,
    index: usize,
    modifiers: Modifiers,
) {
    let source = if keep_playlist {
        Source::Playlist
    } else {
        Source::Library
    };
    let playlist_id = if keep_playlist {
        ui.selected_playlist_id.clone()
    } else {
        String::new()
    };
    // Only a range needs the ids of every row.
    let document_ids = if modifiers.range {
        active_document_ids(content, ui)
    } else {
        Vec::new()
    };
    let next = next_document_selection(
        ui,
        item_id,
        index,
        source,
        &playlist_id,
        modifiers,
        &document_ids,
    );
    ui.selected_documents = next.entries;
    ui.selected_document_source = Some(source);
    ui.selected_document_playlist_id = playlist_id;
    ui.document_anchor = Some(next.anchor);
    ui.selected_library_ids = if keep_playlist || ui.selected_library_id.is_empty() {
        Vec::new()
    } else {
        vec![ui.selected_library_id.clone()]
    };
    ui.library_anchor = None;
    ui.library_menu = None;
    ui.selected_playlist_ids.clear();
    ui.playlist_anchor = None;
    select_document_item(ui, item_id, keep_playlist);
}

/// `selectDocumentItem` without a cue editor: `library:item-selected` or `playlist:library-item-selected`.
pub fn select_document_item(ui: &mut Ui, item_id: &str, keep_playlist: bool) {
    ui.selected_library_item_id = item_id.to_string();
    if !keep_playlist {
        ui.selected_playlist_id.clear();
    }
}

/// `handlePlaylistEntryRowClick`. A header selects only its row; a placeholder also selects its linked document.
/// Returns whether the entry exists.
pub fn click_entry_row(
    content: &Content,
    ui: &mut Ui,
    entry_id: &str,
    index: usize,
    modifiers: Modifiers,
) -> bool {
    let Some(playlist) = content.playlist(&ui.selected_playlist_id) else {
        return false;
    };
    let Some(entry) = playlist.entries.iter().find(|entry| entry.id == entry_id) else {
        return false;
    };
    let item_id = if entry.kind == EntryKind::Presentation {
        entry.item_id.clone()
    } else {
        entry.id.clone()
    };
    if item_id.is_empty() {
        return false;
    }
    let playlist_id = ui.selected_playlist_id.clone();
    // Only a range needs the ids of every row.
    let document_ids = if modifiers.range {
        active_document_ids(content, ui)
    } else {
        Vec::new()
    };
    let next = next_document_selection(
        ui,
        &item_id,
        index,
        Source::Playlist,
        &playlist_id,
        modifiers,
        &document_ids,
    );
    let linked = entry.linked_document().map(str::to_string);
    let presentation = entry.kind == EntryKind::Presentation;
    ui.selected_documents = next.entries;
    ui.playlist_reveal = None;
    ui.selected_document_source = Some(Source::Playlist);
    ui.selected_document_playlist_id = playlist_id;
    ui.document_anchor = Some(next.anchor);
    ui.selected_library_ids.clear();
    ui.library_anchor = None;
    ui.library_menu = None;
    ui.selected_playlist_ids.clear();
    ui.playlist_anchor = None;
    if let Some(linked) = linked {
        ui.selected_library_item_id = linked;
    }
    if presentation {
        select_document_item(ui, &item_id, true);
    }
    true
}

/// Select the next visible presentation while retaining its original playlist occurrence index.
pub fn navigate_document_item(content: &Content, ui: &mut Ui, direction: i32) -> Option<usize> {
    let in_playlist = content.playlist(&ui.selected_playlist_id);
    let rows: Vec<(String, usize)> = match in_playlist {
        Some(playlist) => playlist
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.kind == EntryKind::Presentation
                    && entry.raw["hidden"] != true
                    && content.items.contains(&entry.item_id)
            })
            .map(|(index, entry)| (entry.item_id.clone(), index))
            .collect(),
        None => content
            .library(&ui.selected_library_id)
            .map(|library| {
                content
                    .sorted_document_ids(library)
                    .iter()
                    .enumerate()
                    .map(|(index, id)| (id.clone(), index))
                    .collect()
            })
            .unwrap_or_default(),
    };
    if rows.is_empty() {
        return None;
    }
    let anchor = ui
        .document_anchor
        .as_ref()
        .filter(|anchor| anchor.playlist_id == ui.selected_playlist_id)
        .map(|anchor| anchor.index);
    let current = rows
        .iter()
        .position(|(id, index)| {
            id == &ui.selected_library_item_id && anchor.is_none_or(|anchor| anchor == *index)
        })
        .or_else(|| {
            rows.iter()
                .position(|(id, _)| id == &ui.selected_library_item_id)
        })
        .unwrap_or(0);
    let next = (current as i64 + i64::from(direction)).clamp(0, rows.len() as i64 - 1) as usize;
    let (id, index) = &rows[next];
    ui.selected_library_item_id = id.clone();
    ui.selected_documents = vec![Selected {
        item_id: id.clone(),
        index: in_playlist.map(|_| *index),
    }];
    let source = if in_playlist.is_some() {
        Source::Playlist
    } else {
        Source::Library
    };
    ui.selected_document_source = Some(source);
    ui.selected_document_playlist_id = ui.selected_playlist_id.clone();
    ui.document_anchor = Some(Anchor {
        source,
        playlist_id: ui.selected_playlist_id.clone(),
        index: *index,
    });
    Some(*index)
}
