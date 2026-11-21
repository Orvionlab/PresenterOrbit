//! Macro and macro collection edits as pure functions over the workspace's `macros` and `macroCollections`
//! arrays, ported from `macro-controller.ts`, `macro-collection-controller.ts`, and `macro-palette-utils.ts`. Each
//! returns the [`Change`] to save, or the translation key of the refusal.

use serde_json::{json, Value};

pub const DEFAULT_COLLECTION: &str = "Default Collection";
const NEW_COLLECTION: &str = "New Collection";

/// What `saveWorkspaceContent` receives: the macros, and the collections only when they changed.
#[derive(Debug, PartialEq)]
pub struct Change {
    pub macros: Vec<Value>,
    pub collections: Option<Vec<Value>>,
}

impl Change {
    pub fn patch(&self) -> Value {
        match &self.collections {
            Some(collections) => json!({"macros": self.macros, "macroCollections": collections}),
            None => json!({"macros": self.macros}),
        }
    }
}

pub struct Library<'a> {
    pub macros: &'a [Value],
    pub collections: &'a [Value],
}

impl<'a> Library<'a> {
    pub fn of(workspace: &'a Value) -> Self {
        let list = |key: &str| workspace[key].as_array().map(Vec::as_slice).unwrap_or(&[]);
        Self {
            macros: list("macros"),
            collections: list("macroCollections"),
        }
    }

    fn collection(&self, id: &str) -> Option<&Value> {
        self.collections.iter().find(|entry| entry["id"] == id)
    }

    fn macro_entry(&self, id: &str) -> Option<&Value> {
        self.macros.iter().find(|entry| entry["id"] == id)
    }
}

fn ids(entry: &Value, key: &str) -> Vec<String> {
    entry[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().map(str::to_string))
        .collect()
}

fn member(entry: &Value, collection: &str) -> bool {
    ids(entry, "collectionIds")
        .iter()
        .any(|id| id == collection)
}

/// `collectionMacros`: the macros of a collection in its order; an empty id is every macro.
pub fn collection_macros(library: &Library, id: &str) -> Result<Vec<Value>, String> {
    if id.is_empty() {
        return Ok(library.macros.to_vec());
    }
    let collection = library.collection(id).ok_or("automation.macros.missing")?;
    let members: Vec<&Value> = library
        .macros
        .iter()
        .filter(|entry| member(entry, id))
        .collect();
    if collection["macroOrder"].is_null() {
        return Ok(members.into_iter().cloned().collect());
    }
    let order = ids(collection, "macroOrder");
    let unique: std::collections::HashSet<_> = order.iter().collect();
    if order.len() != members.len()
        || unique.len() != members.len()
        || order
            .iter()
            .any(|id| !members.iter().any(|entry| entry["id"] == id.as_str()))
    {
        return Err("automation.macros.responseInvalid".into());
    }
    Ok(order
        .iter()
        .filter_map(|id| members.iter().find(|entry| entry["id"] == id.as_str()))
        .map(|entry| (*entry).clone())
        .collect())
}

/// The selected collection, or the first one when the selection is empty or stale.
pub fn active_collection(collections: &[Value], selected: &str) -> String {
    collections
        .iter()
        .find(|entry| entry["id"] == selected)
        .or_else(|| collections.first())
        .and_then(|entry| entry["id"].as_str())
        .unwrap_or("")
        .to_string()
}

/// `persistMacros`: the collections' explicit orders follow the membership that the new macros imply.
pub fn persist(library: &Library, macros: Vec<Value>, collections: Option<Vec<Value>>) -> Change {
    let explicit = collections.is_some();
    let mut next = collections.unwrap_or_else(|| library.collections.to_vec());
    for collection in next.iter_mut() {
        if collection["macroOrder"].is_null() {
            continue;
        }
        let id = collection["id"].as_str().unwrap_or("").to_string();
        let members: Vec<String> = macros
            .iter()
            .filter(|entry| member(entry, &id))
            .filter_map(|entry| entry["id"].as_str().map(str::to_string))
            .collect();
        let kept = ids(collection, "macroOrder")
            .into_iter()
            .filter(|known| members.contains(known));
        let kept: Vec<String> = kept.collect();
        let added = members
            .iter()
            .filter(|known| !kept.contains(known))
            .cloned();
        collection["macroOrder"] = json!(kept.iter().cloned().chain(added).collect::<Vec<_>>());
    }
    let changed = explicit
        || next.iter().enumerate().any(|(index, collection)| {
            collection["macroOrder"]
                != library
                    .collections
                    .get(index)
                    .map_or(Value::Null, |old| old["macroOrder"].clone())
        });
    Change {
        macros,
        collections: changed.then_some(next),
    }
}

fn unique_title(collections: &[Value], base: &str) -> String {
    let taken = |title: &str| collections.iter().any(|entry| entry["title"] == title);
    let mut title = base.to_string();
    let mut counter = 2;
    while taken(&title) {
        title = format!("{base} {counter}");
        counter += 1;
    }
    title
}

/// `saveMacro`: validated by the caller; the title is trimmed and a hotkey is stored canonically.
pub fn save(library: &Library, entry: &Value, canonical_hotkey: impl Fn(&str) -> String) -> Change {
    let mut saved = entry.clone();
    saved["title"] = json!(entry["title"].as_str().unwrap_or("").trim());
    if let Some(hotkey) = entry["hotkey"].as_str().filter(|hotkey| !hotkey.is_empty()) {
        saved["hotkey"] = json!(canonical_hotkey(hotkey));
    }
    let mut macros = library.macros.to_vec();
    match macros.iter_mut().find(|known| known["id"] == entry["id"]) {
        Some(slot) => *slot = saved,
        None => macros.push(saved),
    }
    persist(library, macros, None)
}

pub fn delete(library: &Library, id: &str) -> Result<Change, String> {
    library.macro_entry(id).ok_or("automation.macros.missing")?;
    Ok(persist(
        library,
        library
            .macros
            .iter()
            .filter(|entry| entry["id"] != id)
            .cloned()
            .collect(),
        None,
    ))
}

/// `ensureDefaultMacroCollection`: a default collection once macros exist (or `force`), and every macro in a
/// collection that resolves; orphans join the first collection. `None` when nothing changes.
pub fn ensure_default(library: &Library, force: bool, new_id: &str) -> Option<Change> {
    let mut collections = library.collections.to_vec();
    let created = collections.is_empty();
    if created {
        if !force && library.macros.is_empty() {
            return None;
        }
        collections.push(json!({"id": new_id, "title": DEFAULT_COLLECTION}));
    }
    let target = collections[0]["id"].clone();
    let mut changed = created;
    let macros = library
        .macros
        .iter()
        .map(|entry| {
            let resolves = entry["collectionIds"].as_array().is_some_and(|ids| {
                ids.iter()
                    .any(|id| collections.iter().any(|collection| collection["id"] == *id))
            });
            if resolves {
                return entry.clone();
            }
            changed = true;
            let mut adopted = entry.clone();
            let mut memberships = entry["collectionIds"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            memberships.push(target.clone());
            adopted["collectionIds"] = json!(memberships);
            adopted
        })
        .collect();
    changed.then(|| persist(library, macros, Some(collections)))
}

/// `createMacroCollection`: the new collection and the change that adds it.
pub fn create_collection(library: &Library, new_id: &str, title: &str) -> (Value, Change) {
    let collection = json!({"id": new_id, "title": unique_title(library.collections, title)});
    let mut collections = library.collections.to_vec();
    collections.push(collection.clone());
    (
        collection,
        persist(library, library.macros.to_vec(), Some(collections)),
    )
}

pub fn new_collection_title() -> &'static str {
    NEW_COLLECTION
}

pub fn rename_collection(library: &Library, id: &str, title: &str) -> Result<Change, String> {
    if title.trim().is_empty() {
        return Err("automation.macros.collections.nameRequired".into());
    }
    let mut collections = library.collections.to_vec();
    let slot = collections
        .iter_mut()
        .find(|entry| entry["id"] == id)
        .ok_or("automation.macros.missing")?;
    slot["title"] = json!(title.trim());
    Ok(persist(library, library.macros.to_vec(), Some(collections)))
}

/// `duplicateMacroCollection`: the copy follows its source and every member of the source joins it.
pub fn duplicate_collection(library: &Library, id: &str, new_id: &str) -> Result<Change, String> {
    let index = library
        .collections
        .iter()
        .position(|entry| entry["id"] == id)
        .ok_or("automation.macros.missing")?;
    let source = &library.collections[index];
    let mut copy = source.clone();
    copy["id"] = json!(new_id);
    copy["title"] = json!(unique_title(
        library.collections,
        &format!("{} Copy", source["title"].as_str().unwrap_or(""))
    ));
    let macros = library
        .macros
        .iter()
        .map(|entry| {
            if !member(entry, id) {
                return entry.clone();
            }
            let mut joined = entry.clone();
            let mut memberships = ids(entry, "collectionIds");
            memberships.push(new_id.to_string());
            joined["collectionIds"] = json!(memberships);
            joined
        })
        .collect();
    let mut collections = library.collections.to_vec();
    collections.insert(index + 1, copy);
    Ok(persist(library, macros, Some(collections)))
}

/// `deleteMacroCollection`: macros that only belonged to it are deleted with it, as in ProPresenter.
pub fn delete_collection(library: &Library, id: &str) -> Result<Change, String> {
    library.collection(id).ok_or("automation.macros.missing")?;
    let remaining: Vec<Value> = library
        .collections
        .iter()
        .filter(|entry| entry["id"] != id)
        .cloned()
        .collect();
    if remaining.is_empty() {
        return Err("automation.macros.collections.lastRequired".into());
    }
    let macros = library
        .macros
        .iter()
        .filter_map(|entry| {
            if !member(entry, id) {
                return Some(entry.clone());
            }
            let memberships: Vec<String> = ids(entry, "collectionIds")
                .into_iter()
                .filter(|known| known != id)
                .collect();
            let alive = memberships
                .iter()
                .any(|known| remaining.iter().any(|entry| entry["id"] == known.as_str()));
            alive.then(|| {
                let mut kept = entry.clone();
                kept["collectionIds"] = json!(memberships);
                kept
            })
        })
        .collect();
    Ok(persist(library, macros, Some(remaining)))
}

pub fn reorder_collections(library: &Library, order: &[String]) -> Result<Change, String> {
    let unique: std::collections::HashSet<_> = order.iter().collect();
    if order.len() != library.collections.len()
        || unique.len() != order.len()
        || order.iter().any(|id| library.collection(id).is_none())
    {
        return Err("automation.macros.collections.targetInvalid".into());
    }
    let collections = order
        .iter()
        .filter_map(|id| library.collection(id).cloned())
        .collect();
    Ok(persist(library, library.macros.to_vec(), Some(collections)))
}

/// `reorderMacro`: one step in a collection's order, or in the macro list when no collection is given.
/// `Ok(None)` at an end of the list.
pub fn reorder(
    library: &Library,
    id: &str,
    direction: i32,
    collection: &str,
) -> Result<Option<Change>, String> {
    if !collection.is_empty() {
        let mut order: Vec<Value> = collection_macros(library, collection)?
            .iter()
            .map(|entry| entry["id"].clone())
            .collect();
        let index = order
            .iter()
            .position(|known| known == id)
            .ok_or("automation.macros.missing")?;
        let Some(destination) = index
            .checked_add_signed(direction as isize)
            .filter(|destination| *destination < order.len())
        else {
            return Ok(None);
        };
        order.swap(index, destination);
        let collections = library
            .collections
            .iter()
            .map(|entry| {
                let mut entry = entry.clone();
                if entry["id"] == collection {
                    entry["macroOrder"] = json!(order);
                }
                entry
            })
            .collect();
        return Ok(Some(persist(
            library,
            library.macros.to_vec(),
            Some(collections),
        )));
    }
    let mut macros = library.macros.to_vec();
    let index = macros
        .iter()
        .position(|entry| entry["id"] == id)
        .ok_or("automation.macros.missing")?;
    let Some(destination) = index
        .checked_add_signed(direction as isize)
        .filter(|destination| *destination < macros.len())
    else {
        return Ok(None);
    };
    macros.swap(index, destination);
    Ok(Some(persist(library, macros, None)))
}

/// `duplicateMacro`: new ids, and no hotkey, because a shortcut belongs to one macro.
pub fn duplicate(entry: &Value, mut new_id: impl FnMut() -> String) -> Value {
    let mut copy = entry.clone();
    copy["id"] = json!(new_id());
    if let Some(object) = copy.as_object_mut() {
        object.remove("hotkey");
    }
    if let Some(actions) = copy["actions"].as_array_mut() {
        for action in actions {
            action["id"] = json!(new_id());
        }
    }
    copy
}

/// What a copy or cut put on the clipboard.
#[derive(Clone, Debug, PartialEq)]
pub struct Clipboard {
    pub workspace: String,
    pub entry: Value,
    pub collection: String,
    pub cut: bool,
}

/// `copyMacro`.
pub fn copy(
    library: &Library,
    workspace: &str,
    id: &str,
    collection: &str,
    cut: bool,
) -> Result<Clipboard, String> {
    let entry = library
        .macro_entry(id)
        .filter(|entry| collection.is_empty() || member(entry, collection))
        .ok_or("automation.macros.missing")?;
    if workspace.is_empty() {
        return Err("automation.macros.workspaceRequired".into());
    }
    Ok(Clipboard {
        workspace: workspace.to_string(),
        entry: entry.clone(),
        collection: collection.to_string(),
        cut,
    })
}

/// `planMacroPaste` and the save of `pasteMacro`: a cut moves the one macro, a copy pastes a duplicate.
pub fn paste(
    library: &Library,
    workspace: &str,
    clipboard: Option<&Clipboard>,
    collection: &str,
    new_id: impl FnMut() -> String,
    canonical_hotkey: impl Fn(&str) -> String,
) -> Result<(Value, Change), String> {
    let clipboard = clipboard
        .filter(|clipboard| clipboard.workspace == workspace)
        .ok_or("automation.macros.missing")?;
    if !collection.is_empty() && library.collection(collection).is_none() {
        return Err("automation.macros.collections.targetInvalid".into());
    }
    if clipboard.cut
        && library.macro_entry(clipboard.entry["id"].as_str().unwrap_or(""))
            != Some(&clipboard.entry)
    {
        return Err("automation.macros.clipboardStale".into());
    }
    let mut entry = if clipboard.cut {
        clipboard.entry.clone()
    } else {
        duplicate(&clipboard.entry, new_id)
    };
    let mut memberships: Vec<String> = if clipboard.cut && !clipboard.collection.is_empty() {
        ids(&entry, "collectionIds")
            .into_iter()
            .filter(|known| *known != clipboard.collection)
            .collect()
    } else {
        Vec::new()
    };
    if !collection.is_empty() {
        memberships.push(collection.to_string());
    }
    let mut seen = Vec::new();
    memberships.retain(|known| {
        let fresh = !seen.contains(known);
        seen.push(known.clone());
        fresh
    });
    entry["collectionIds"] = json!(memberships);
    let change = save(library, &entry, canonical_hotkey);
    Ok((entry, change))
}

/// `actionSummary` / `macroActionSummary`: the ordered actions with disabled state, delay, and named target.
pub fn action_summary(workspace: &Value, entry: &Value) -> String {
    entry["actions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|action| {
            let kind = action["type"].as_str().unwrap_or("");
            let target = match action["targetId"].as_str().filter(|id| !id.is_empty()) {
                Some(id) => super::validate::target_options(workspace, kind, "")
                    .iter()
                    .find(|option| option["id"] == id)
                    .and_then(|option| option["title"].as_str().or_else(|| option["name"].as_str()))
                    .unwrap_or(id)
                    .to_string(),
                None => action["targetLayer"].as_str().unwrap_or("").to_string(),
            };
            let title = action["title"]
                .as_str()
                .filter(|title| !title.is_empty())
                .unwrap_or(kind);
            let delay = action["delaySeconds"].as_f64().filter(|delay| *delay != 0.);
            format!(
                "{}{}{}{}",
                if action["enabled"] == false {
                    "− "
                } else {
                    ""
                },
                title,
                if target.is_empty() {
                    String::new()
                } else {
                    format!(": {target}")
                },
                delay.map_or(String::new(), |delay| format!(" (+{delay}s)"))
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}
