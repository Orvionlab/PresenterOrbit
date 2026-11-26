use crate::{
    collections::model::{Entry, EntryKind, Item},
    state::{
        runtime::Runtime,
        slices::{Content, Ui},
    },
};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
pub struct Document {
    pub item: Item,
    pub playlist: String,
    pub entry: String,
    pub arrangement: String,
    pub cues: Arc<Vec<Value>>,
}

pub struct DocumentSeed {
    pub item: Item,
    pub playlist: String,
    pub entry: String,
    pub arrangement: String,
}

impl DocumentSeed {
    pub fn current_arrangement(document: &Document, content: &Content) -> Option<Self> {
        let item = content.items.get(&document.item.id)?.clone();
        let arrangement = if document.entry.is_empty() {
            selected_arrangement_id(&item.value)
        } else {
            let entry = content
                .playlist(&document.playlist)?
                .entries
                .iter()
                .find(|entry| entry.id == document.entry)?;
            existing_arrangement_id(&item.value, &entry.arrangement_id)
        }
        .to_string();
        Some(Self {
            item,
            playlist: document.playlist.clone(),
            entry: document.entry.clone(),
            arrangement,
        })
    }

    pub fn resolve(self) -> Result<Document, String> {
        let cues = orbit_host::Host::presentation_cues(
            &self.item.value,
            &self.arrangement,
            (!self.entry.is_empty()).then_some(self.entry.as_str()),
        )?;
        Ok(Document {
            item: self.item,
            playlist: self.playlist,
            entry: self.entry,
            arrangement: self.arrangement,
            cues: Arc::new(cues),
        })
    }
}

/// The arrangement `id` names in `item`, or "" (master order) when it no longer exists, as React's
/// `resolvePresentationArrangement` does. Imported data often keeps stale arrangement ids.
pub fn existing_arrangement_id<'a>(item: &Value, id: &'a str) -> &'a str {
    let exists = item["arrangements"].as_array().is_some_and(|arrangements| {
        arrangements
            .iter()
            .any(|arrangement| arrangement["id"].as_str() == Some(id))
    });
    if exists {
        id
    } else {
        ""
    }
}

pub fn selected_arrangement_id(item: &Value) -> &str {
    existing_arrangement_id(item, item["selectedArrangementId"].as_str().unwrap_or(""))
}

pub fn selected_playlist_entry<'a>(content: &'a Content, ui: &Ui) -> Option<(usize, &'a Entry)> {
    let index = ui.selected_documents.first()?.index?;
    let entry = content
        .playlist(&ui.selected_playlist_id)?
        .entries
        .get(index)?;
    (entry.raw["hidden"] != true).then_some((index, entry))
}

pub fn seed(content: &Content, ui: &Ui) -> Option<DocumentSeed> {
    let entry = selected_playlist_entry(content, ui).map(|(_, entry)| entry);
    let id = match entry {
        Some(entry) if entry.kind != EntryKind::Presentation => entry.linked_document()?,
        _ => &ui.selected_library_item_id,
    };
    let item = content.items.get(id).cloned()?;
    let entry = entry.filter(|entry| entry.item_id == item.id || entry.linked_item_id == item.id);
    let (playlist, occurrence, arrangement) = match entry {
        Some(entry) => (
            ui.selected_playlist_id.clone(),
            entry.id.clone(),
            existing_arrangement_id(&item.value, &entry.arrangement_id).to_string(),
        ),
        None => (
            String::new(),
            String::new(),
            selected_arrangement_id(&item.value).to_string(),
        ),
    };
    Some(DocumentSeed {
        item,
        playlist,
        entry: occurrence,
        arrangement,
    })
}

pub fn selected_indices(
    cues: &[Value],
    selected: &[String],
    anchor: Option<usize>,
    index: usize,
    shift: bool,
    toggle: bool,
) -> Vec<String> {
    let Some(id) = cues.get(index).and_then(|cue| cue["id"].as_str()) else {
        return selected.to_vec();
    };
    if shift {
        let anchor = anchor.unwrap_or(index).min(cues.len() - 1);
        let mut result = if toggle {
            selected.to_vec()
        } else {
            Vec::new()
        };
        let mut seen = result
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        for cue in &cues[anchor.min(index)..=anchor.max(index)] {
            if let Some(id) = cue["id"].as_str() {
                if seen.insert(id.to_string()) {
                    result.push(id.to_string());
                }
            }
        }
        result
    } else if toggle {
        let mut result = selected.to_vec();
        if result.iter().any(|value| value == id) {
            result.retain(|value| value != id);
        } else {
            result.push(id.to_string());
        }
        result
    } else {
        vec![id.to_string()]
    }
}

pub fn is_announcement_live(live: &Value, document: &Document, cue: &Value) -> bool {
    let player = &live["announcements"];
    player["item"]["id"] == document.item.id
        && player["playlistId"].as_str().unwrap_or("") == document.playlist
        && player["slide"]["id"] == cue["id"]
        && player["slide"]["arrangementInstanceId"] == cue["arrangementInstanceId"]
}

pub fn is_presentation_live(live: &Value, document: &Document, cue: &Value) -> bool {
    live["liveItemId"] == document.item.id
        && live["liveSlideId"] == cue["id"]
        && live["liveCueInstanceId"].as_str().unwrap_or("")
            == cue["arrangementInstanceId"].as_str().unwrap_or("")
        && live["livePlaylistId"].as_str().unwrap_or("") == document.playlist
}

pub fn group_id(cue: &Value) -> &str {
    cue["groupId"]
        .as_str()
        .or_else(|| cue["groupName"].as_str())
        .unwrap_or("")
}
pub fn effective_group(cues: &[Value], index: usize) -> String {
    let group = crate::collections::arrangements::effective_group_id(cues, index);
    if group == "no-group" {
        String::new()
    } else {
        group
    }
}

/// The cue a group hotkey takes: the first enabled cue whose group has that key (`activateGroupShortcut`).
pub fn hotkey_target(cues: &[Value], groups: &Value, key: &str) -> Option<usize> {
    let mut characters = key.chars();
    let (Some(_), None) = (characters.next(), characters.next()) else {
        return None;
    };
    let key = crate::keys::normalize_shortcut_key(key);
    if let Some(index) = cues.iter().position(|cue| {
        cue["disabled"] != true
            && cue["shortcut"]
                .as_str()
                .is_some_and(|shortcut| crate::keys::normalize_shortcut_key(shortcut) == key)
    }) {
        return Some(index);
    }
    let ids: Vec<&str> = groups
        .as_object()?
        .iter()
        .filter(|(_, group)| {
            crate::keys::normalize_shortcut_key(group["key"].as_str().unwrap_or("")) == key
        })
        .map(|(id, _)| id.as_str())
        .collect();
    (0..cues.len()).find(|&index| {
        cues[index]["disabled"] != true && ids.contains(&effective_group(cues, index).as_str())
    })
}

pub fn adjacent_document(
    runtime: &Runtime,
    cx: &gpui::App,
    direction: i8,
) -> Option<(String, Option<usize>, String, String)> {
    let content = runtime.content.read(cx);
    let ui = runtime.ui.read(cx);
    let live = &runtime.output.read(cx).live;
    let current = live["liveItemId"]
        .as_str()
        .or_else(|| live["presentationNextAnchor"]["itemId"].as_str())
        .unwrap_or(&ui.selected_library_item_id);
    let playlist = live["livePlaylistId"]
        .as_str()
        .or_else(|| live["presentationNextAnchor"]["playlistId"].as_str())
        .unwrap_or(&ui.selected_playlist_id)
        .to_string();
    let library = if current == ui.selected_library_item_id {
        ui.selected_library_id.clone()
    } else {
        content.library_id_for_item(current).to_string()
    };
    let rows = if let Some(playlist) = content.playlist(&playlist) {
        playlist
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.raw["hidden"] != true
                    && (entry.kind == EntryKind::Presentation || !entry.linked_item_id.is_empty())
            })
            .map(|(index, entry)| {
                (
                    if entry.kind == EntryKind::Presentation {
                        entry.item_id.clone()
                    } else {
                        entry.linked_item_id.clone()
                    },
                    Some(index),
                )
            })
            .collect::<Vec<_>>()
    } else {
        content
            .library(&library)
            .map(|library| {
                content
                    .sorted_document_ids(library)
                    .iter()
                    .map(|id| (id.clone(), None))
                    .collect()
            })
            .unwrap_or_default()
    };
    let occurrence = live["livePlaylistEntryId"]
        .as_str()
        .or_else(|| live["presentationNextAnchor"]["playlistEntryId"].as_str())
        .and_then(|entry| {
            content
                .playlist(&playlist)?
                .entries
                .iter()
                .position(|candidate| candidate.id == entry)
        })
        .or_else(|| {
            ui.selected_documents
                .first()
                .and_then(|selected| selected.index)
        });
    let index = rows
        .iter()
        .position(|(id, index)| id == current && (index.is_none() || index == &occurrence))?;
    let next = index as isize + direction as isize;
    (next >= 0)
        .then(|| rows.get(next as usize).cloned())
        .flatten()
        .map(|(id, index)| (id, index, playlist, library))
}

/// The document with the selected slides moved to `target`, as `reorderSlides` does. `None` when the cues
/// are not in master order (an arrangement), where the host decides the visible order.
pub fn reordered(document: &Document, sources: &[usize], target: usize) -> Option<Document> {
    let mut value = (*document.item.value).clone();
    let slides = value["slides"].as_array_mut()?;
    let in_master_order = slides.len() == document.cues.len()
        && slides
            .iter()
            .zip(document.cues.iter())
            .all(|(a, b)| a["id"] == b["id"]);
    if !in_master_order {
        return None;
    }
    *slides = orbit_host::authoring::reordered_slides(std::mem::take(slides), sources, target);
    let mut next = document.clone();
    next.cues = Arc::new(orbit_host::authoring::reordered_slides(
        (*document.cues).clone(),
        sources,
        target,
    ));
    next.item.value = Arc::new(value);
    Some(next)
}

/// Final index after removing the selected slides. A contiguous block dropped in place has no target.
pub fn drop_index(sources: &[usize], gap: usize) -> Option<usize> {
    let target = gap - sources.partition_point(|source| *source < gap);
    sources
        .iter()
        .copied()
        .ne(target..target + sources.len())
        .then_some(target)
}

/// Whether two item values differ at most in the order of their slides.
pub fn same_but_order(a: &Value, b: &Value) -> bool {
    let sorted = |value: &Value| {
        let mut value = value.clone();
        if let Some(slides) = value["slides"].as_array_mut() {
            slides.sort_by_key(|slide| slide["id"].as_str().unwrap_or("").to_string());
        }
        value
    };
    sorted(a) == sorted(b)
}
