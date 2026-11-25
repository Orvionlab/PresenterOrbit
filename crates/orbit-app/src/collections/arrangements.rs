//! Arrangements of a presentation and their ordered sections: `arrangement-controller.ts`,
//! `arrangement-section-controller.ts`, and the section and title helpers of `arrangement-utils.ts`.
//!
//! The cue editor owns the surface. Runtime canonical commits refresh the pinned live presentation;
//! optimistic changes here touch only the document and the editor.

use gpui::{App, Task};
use serde_json::{json, Value};

use super::document_actions::set_entry_arrangement;
use super::model::Item;
use crate::state::runtime::{Kind, Runtime};
use crate::state::slices::{ArrangementEditor, Content};

/// One section of a presentation: a run of slides with one group, or one slide without a group.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub id: String,
    pub name: String,
    pub slide_count: usize,
}

const NO_GROUP: &str = "no-group";

/// `isGeneratedSlideMetadataLabel`: imported numeric labels and "new slide" copies are not group names.
fn generated_label(value: &str) -> bool {
    let label = value.trim();
    let numeric = label.len() >= 8 && label.chars().all(|c| c.is_ascii_digit());
    let lower = label.to_lowercase();
    let copy = lower.strip_prefix("new-slide").is_some_and(|rest| {
        rest.is_empty()
            || rest.strip_prefix("-copy").is_some_and(|tail| {
                tail.is_empty()
                    || tail
                        .strip_prefix('-')
                        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            })
    });
    numeric || copy
}

/// `getSlideGroupId`.
fn slide_group_id(slide: &Value) -> String {
    let value = if slide.get("groupId").is_some() {
        &slide["groupId"]
    } else {
        &slide["groupName"]
    };
    let text = value.as_str().unwrap_or("");
    if generated_label(text) {
        String::new()
    } else {
        text.trim().to_string()
    }
}

/// `getEffectiveSlideGroupId`: the slide's own group, else the nearest one before it.
pub(crate) fn effective_group_id(slides: &[Value], index: usize) -> String {
    for slide in slides[..slides.len().min(index.saturating_add(1))]
        .iter()
        .rev()
    {
        let id = slide_group_id(slide);
        if !id.is_empty() {
            return id;
        }
    }
    NO_GROUP.to_string()
}

/// `getPresentationSections`.
pub fn presentation_sections(item: &Item) -> Vec<Section> {
    let slides: Vec<Value> = item.value["slides"].as_array().cloned().unwrap_or_default();
    let mut sections: Vec<Section> = Vec::new();
    for (index, slide) in slides.iter().enumerate() {
        let group = effective_group_id(&slides, index);
        let id = if group == NO_GROUP {
            format!("section-{}", index + 1)
        } else {
            format!("section-{group}")
        };
        if let Some(existing) = sections.iter_mut().find(|section| section.id == id) {
            existing.slide_count += 1;
            continue;
        }
        let name = if group == NO_GROUP {
            ["label", "name", "title"]
                .iter()
                .find_map(|key| slide[*key].as_str().filter(|text| !text.is_empty()))
                .unwrap_or("Section")
                .to_string()
        } else {
            group
        };
        sections.push(Section {
            id,
            name,
            slide_count: 1,
        });
    }
    sections
}

fn unique_arrangement_name(item: &Item, base: &str) -> String {
    let names: Vec<String> = item
        .arrangements
        .iter()
        .map(|a| a.name.to_lowercase())
        .collect();
    if !names.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    let mut suffix = 2;
    while names.contains(&format!("{base} {suffix}").to_lowercase()) {
        suffix += 1;
    }
    format!("{base} {suffix}")
}

/// Applies `change` to the item's JSON and keeps the typed view in step.
pub(crate) fn edit_item(content: &mut Content, item_id: &str, change: impl FnOnce(&mut Value)) {
    let Some(item) = content.items.get(item_id) else {
        return;
    };
    let mut value = (*item.value).clone();
    change(&mut value);
    if let Ok(next) = Item::parse(&value) {
        let items = content.items.map_item(item_id, |_| next.clone());
        content.replace_items(items);
    }
}

fn arrangements_mut(value: &mut Value) -> &mut Vec<Value> {
    if !value["arrangements"].is_array() {
        value["arrangements"] = json!([]);
    }
    value["arrangements"]
        .as_array_mut()
        .expect("arrangements is an array")
}

/// `openArrangementEditor`.
pub fn open_arrangement_editor(runtime: &Runtime, cx: &mut App, item_id: &str) {
    runtime.edit_ui(cx, |content, ui| {
        if content.items.get(item_id).is_some() {
            ui.arrangement_editor = Some(ArrangementEditor {
                item_id: item_id.to_string(),
            });
        }
    });
}

pub fn close_arrangement_editor(runtime: &Runtime, cx: &mut App) {
    runtime.edit_ui(cx, |_, ui| ui.arrangement_editor = None);
}

/// `createArrangement`: a new arrangement with every section in order. An empty name creates nothing.
pub fn create_arrangement(
    runtime: &Runtime,
    cx: &mut App,
    item_id: &str,
    raw_name: &str,
) -> Option<(String, Task<()>)> {
    let name = raw_name.trim();
    if name.is_empty() {
        return None;
    }
    let (unique, sections) = {
        let item = runtime.content.read(cx).items.get(item_id)?;
        (
            unique_arrangement_name(item, name),
            presentation_sections(item)
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
        )
    };
    let id = uuid::Uuid::new_v4().to_string();
    let mutation = json!({ "type": "createArrangement", "itemId": item_id, "name": unique, "sectionIds": sections, "newArrangementId": id });
    let target = item_id.to_string();
    let arrangement = json!({ "id": id, "name": unique, "sectionIds": sections });
    let task = runtime.save_preview(cx, Kind::Library, mutation, move |content| {
        preview_created(content, &target, &arrangement);
    });
    Some((id, task))
}

/// `duplicateArrangement`.
pub fn duplicate_arrangement(
    runtime: &Runtime,
    cx: &mut App,
    item_id: &str,
    arrangement_id: &str,
) -> Option<(String, Task<()>)> {
    let item = runtime.content.read(cx).items.get(item_id)?;
    let mut copy = item.value["arrangements"]
        .as_array()?
        .iter()
        .find(|arrangement| arrangement["id"] == arrangement_id)?
        .clone();
    let name = unique_arrangement_name(item, &format!("{} Copy", copy["name"].as_str()?));
    let id = uuid::Uuid::new_v4().to_string();
    let mutation = json!({ "type": "duplicateArrangement", "itemId": item_id, "arrangementId": arrangement_id, "newArrangementId": id });
    copy["id"] = json!(id);
    copy["name"] = json!(name);
    let target = item_id.to_string();
    let task = runtime.save_preview(cx, Kind::Library, mutation, move |content| {
        preview_created(content, &target, &copy);
    });
    Some((id, task))
}

fn preview_created(content: &mut Content, target: &str, arrangement: &Value) {
    edit_item(content, target, |value| {
        let arrangements = arrangements_mut(value);
        if !arrangements
            .iter()
            .any(|existing| existing["id"] == arrangement["id"])
        {
            arrangements.push(arrangement.clone());
        }
    });
}

/// `moveArrangement`: a position-based intent.
pub fn move_arrangement(
    runtime: &Runtime,
    cx: &mut App,
    item_id: &str,
    from: usize,
    to: usize,
) -> Option<Task<()>> {
    let count = runtime
        .content
        .read(cx)
        .items
        .get(item_id)?
        .arrangements
        .len();
    if from == to || from >= count {
        return None;
    }
    let mutation =
        json!({ "type": "moveArrangement", "itemId": item_id, "fromIndex": from, "toIndex": to });
    let target = item_id.to_string();
    Some(
        runtime.save_preview(cx, Kind::Library, mutation, move |content| {
            edit_item(content, &target, |value| {
                let list = arrangements_mut(value);
                if from < list.len() {
                    let moved = list.remove(from);
                    list.insert(to.min(list.len()), moved);
                }
            });
        }),
    )
}

/// `renameArrangement`.
pub fn rename_arrangement(
    runtime: &Runtime,
    cx: &mut App,
    item_id: &str,
    arrangement_id: &str,
    raw_name: &str,
) -> Option<Task<()>> {
    let name = raw_name.trim().to_string();
    if name.is_empty() || runtime.content.read(cx).items.get(item_id).is_none() {
        return None;
    }
    let mutation = json!({ "type": "renameArrangement", "itemId": item_id, "arrangementId": arrangement_id, "name": name });
    let (target, id) = (item_id.to_string(), arrangement_id.to_string());
    Some(
        runtime.save_preview(cx, Kind::Library, mutation, move |content| {
            edit_item(content, &target, |value| {
                for arrangement in arrangements_mut(value)
                    .iter_mut()
                    .filter(|a| a["id"] == id.as_str())
                {
                    arrangement["name"] = json!(name);
                }
            });
        }),
    )
}

/// `selectActiveArrangement` for a presentation outside a playlist entry: the document's default. For a playlist
/// entry, `set_entry_arrangement` carries the anchored mutation.
pub fn select_arrangement(
    runtime: &Runtime,
    cx: &mut App,
    item_id: &str,
    arrangement_id: &str,
    playlist_entry: Option<(&str, &str)>,
) -> Option<Task<()>> {
    if let Some((playlist_id, entry_id)) = playlist_entry {
        let entry_exists = runtime
            .content
            .read(cx)
            .playlist(playlist_id)
            .is_some_and(|p| {
                p.entries
                    .iter()
                    .any(|e| e.id == entry_id && e.linked_document() == Some(item_id))
            });
        return entry_exists
            .then(|| {
                set_entry_arrangement(runtime, cx, playlist_id, entry_id, None, arrangement_id)
            })
            .flatten();
    }
    runtime.content.read(cx).items.get(item_id)?;
    let mutation = json!({ "type": "setSelectedArrangement", "itemId": item_id, "arrangementId": arrangement_id });
    let (target, id) = (item_id.to_string(), arrangement_id.to_string());
    Some(
        runtime.save_preview(cx, Kind::Library, mutation, move |content| {
            edit_item(content, &target, |value| set_selected(value, &id));
        }),
    )
}

fn set_selected(value: &mut Value, arrangement_id: &str) {
    if arrangement_id.is_empty() {
        value
            .as_object_mut()
            .map(|object| object.remove("selectedArrangementId"));
    } else {
        value["selectedArrangementId"] = json!(arrangement_id);
    }
}

/// `deleteArrangement`: also clears the default and every playlist entry that chose it.
pub fn delete_arrangement(
    runtime: &Runtime,
    cx: &mut App,
    item_id: &str,
    arrangement_id: &str,
) -> Option<Task<()>> {
    runtime.content.read(cx).items.get(item_id)?;
    let mutation =
        json!({ "type": "deleteArrangement", "itemId": item_id, "arrangementId": arrangement_id });
    let (target, id) = (item_id.to_string(), arrangement_id.to_string());
    Some(
        runtime.save_preview(cx, Kind::Library, mutation, move |content| {
            edit_item(content, &target, |value| {
                arrangements_mut(value).retain(|a| a["id"] != id.as_str());
                if value["selectedArrangementId"] == id.as_str() {
                    set_selected(value, "");
                }
            });
            for playlist in &mut content.playlists {
                for entry in playlist.entries.iter_mut().filter(|e| {
                    e.linked_document() == Some(target.as_str()) && e.arrangement_id == id
                }) {
                    entry.arrangement_id.clear();
                    entry
                        .raw
                        .as_object_mut()
                        .map(|object| object.remove("arrangementId"));
                }
            }
        }),
    )
}

/// One edit of an arrangement's section list.
#[derive(Clone, Debug, PartialEq)]
pub enum SectionEdit {
    Add(String),
    Insert { section: String, index: usize },
    Remove(usize),
    Move { from: usize, to: usize },
}

impl SectionEdit {
    fn mutation(&self, item_id: &str, arrangement_id: &str) -> Value {
        let base = json!({ "itemId": item_id, "arrangementId": arrangement_id });
        let mut mutation = match self {
            Self::Add(section) => json!({ "type": "addArrangementSection", "sectionId": section }),
            Self::Insert { section, index } => {
                json!({ "type": "insertArrangementSection", "sectionId": section, "index": index })
            }
            Self::Remove(index) => json!({ "type": "removeArrangementSection", "index": index }),
            Self::Move { from, to } => {
                json!({ "type": "moveArrangementSection", "fromIndex": from, "toIndex": to })
            }
        };
        mutation
            .as_object_mut()
            .expect("an object")
            .extend(base.as_object().expect("an object").clone());
        mutation
    }

    /// The section list after the edit; an edit that does not apply leaves it as it is.
    pub fn apply(&self, sections: &[String]) -> Vec<String> {
        let mut next = sections.to_vec();
        match self {
            Self::Add(section) => next.push(section.clone()),
            Self::Insert { section, index } => {
                next.insert((*index).min(next.len()), section.clone())
            }
            Self::Remove(index) => {
                if *index < next.len() {
                    next.remove(*index);
                }
            }
            Self::Move { from, to } => {
                if from != to && *from < next.len() {
                    let moved = next.remove(*from);
                    next.insert((*to).min(next.len()), moved);
                }
            }
        }
        next
    }
}

/// The edit that dropping a chip in `gap` of the order bar asks for: a move for a chip of the bar, an insert for
/// a group. `None` when a chip would land where it already is.
pub fn drop_edit(from: Option<usize>, section: &str, gap: usize) -> Option<SectionEdit> {
    match from {
        None => Some(SectionEdit::Insert {
            section: section.to_string(),
            index: gap,
        }),
        Some(from) => {
            let to = if gap > from { gap - 1 } else { gap };
            (to != from).then_some(SectionEdit::Move { from, to })
        }
    }
}

/// `addArrangementSection`, `insertArrangementSection`, `removeArrangementSection`, and
/// `moveArrangementSection`. Only adding replays after a stale revision: the others address a position.
pub fn edit_sections(
    runtime: &Runtime,
    cx: &mut App,
    item_id: &str,
    arrangement_id: &str,
    edit: SectionEdit,
) -> Option<Task<()>> {
    let item = runtime.content.read(cx).items.get(item_id)?;
    let arrangement = item
        .arrangements
        .iter()
        .find(|arrangement| arrangement.id == arrangement_id)?;
    let next = edit.apply(&arrangement.section_ids);
    let mut mutation = edit.mutation(item_id, arrangement_id);
    if !matches!(edit, SectionEdit::Add(_)) {
        mutation["expectedSectionIds"] = json!(arrangement.section_ids);
    }
    let (target, id) = (item_id.to_string(), arrangement_id.to_string());
    Some(
        runtime.save_preview(cx, Kind::Library, mutation, move |content| {
            edit_item(content, &target, |value| {
                for arrangement in arrangements_mut(value)
                    .iter_mut()
                    .filter(|a| a["id"] == id.as_str())
                {
                    arrangement["sectionIds"] = json!(next);
                }
            });
        }),
    )
}
