//! Which parts of a ProPresenter folder an import takes, and the read-only inventory the
//! selection dialog shows.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Categories besides libraries, keyed as the UI sends them.
pub const CATEGORIES: [&str; 12] = [
    "playlists",
    "media",
    "themes",
    "stageLayouts",
    "props",
    "masks",
    "macros",
    "messages",
    "groups",
    "timers",
    "presets",
    "settings",
];

/// The `items` key that narrows the presentations of the selected libraries; the other keys are
/// the categories that list their items. `settings` is a mix of unrelated documents and stays
/// all-or-nothing.
pub const PRESENTATIONS: &str = "presentations";

#[derive(Default)]
struct Filter {
    libraries: HashSet<String>,
    categories: HashSet<String>,
    items: HashMap<String, HashSet<String>>,
}

pub struct ImportSelection {
    /// `None` imports everything.
    filter: Option<Filter>,
}

impl ImportSelection {
    /// `{ "libraries": [folder names], "categories": [keys], "items": { key: [ids] } }`; `items`
    /// is optional and lists the ids that stay selected, a key left out keeps all of its items.
    pub fn parse(value: &Value) -> Result<Self, String> {
        let strings = |list: Option<&Value>, key: &str| -> Result<HashSet<String>, String> {
            list.and_then(Value::as_array)
                .ok_or_else(|| format!("Import selection requires a '{key}' list."))?
                .iter()
                .map(|entry| {
                    entry
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("Import selection '{key}' must hold text."))
                })
                .collect()
        };
        let libraries = strings(value.get("libraries"), "libraries")?;
        let categories = strings(value.get("categories"), "categories")?;
        if let Some(unknown) = categories
            .iter()
            .find(|category| !CATEGORIES.contains(&category.as_str()))
        {
            return Err(format!("Unknown import category '{unknown}'."));
        }
        let mut items = HashMap::new();
        if let Some(listed) = value.get("items") {
            let listed = listed
                .as_object()
                .ok_or("Import selection 'items' must be an object.")?;
            for (key, ids) in listed {
                if key != PRESENTATIONS
                    && (key == "settings" || !CATEGORIES.contains(&key.as_str()))
                {
                    return Err(format!("Unknown import item list '{key}'."));
                }
                items.insert(key.clone(), strings(Some(ids), key)?);
            }
        }
        Ok(Self {
            filter: Some(Filter {
                libraries,
                categories,
                items,
            }),
        })
    }

    pub fn wants(&self, category: &str) -> bool {
        self.filter
            .as_ref()
            .is_none_or(|filter| filter.categories.contains(category))
    }

    pub fn wants_library(&self, folder: &str) -> bool {
        self.filter
            .as_ref()
            .is_none_or(|filter| filter.libraries.contains(folder))
    }

    pub fn excludes_all_libraries(&self) -> bool {
        self.filter
            .as_ref()
            .is_some_and(|filter| filter.libraries.is_empty())
    }

    /// Whether the presentation `id` (`folder/path/inside/it.pro`) is selected.
    pub fn wants_presentation(&self, id: &str) -> bool {
        self.filter.as_ref().is_none_or(|filter| {
            filter
                .items
                .get(PRESENTATIONS)
                .is_none_or(|ids| ids.contains(id))
        })
    }

    /// Fails when a selected presentation id belongs to no scanned library.
    pub fn check_presentations(&self, known: &HashSet<String>) -> Result<(), String> {
        let selected = self
            .filter
            .as_ref()
            .and_then(|filter| filter.items.get(PRESENTATIONS));
        match selected
            .into_iter()
            .flatten()
            .find(|id| !known.contains(*id))
        {
            Some(unknown) => Err(format!("Unknown import presentation '{unknown}'.")),
            None => Ok(()),
        }
    }

    /// Keeps the selected records of `category` in each of `lists`; an id that none of them
    /// holds is rejected.
    pub fn retain_items(
        &self,
        category: &str,
        lists: &mut [&mut Vec<Value>],
    ) -> Result<(), String> {
        let Some(ids) = self
            .filter
            .as_ref()
            .and_then(|filter| filter.items.get(category))
        else {
            return Ok(());
        };
        let id_of = |record: &Value| record["id"].as_str().map(str::to_string);
        let known = lists
            .iter()
            .flat_map(|list| list.iter().filter_map(id_of))
            .collect::<HashSet<_>>();
        if let Some(unknown) = ids.iter().find(|id| !known.contains(*id)) {
            return Err(format!("Unknown import item '{unknown}' in '{category}'."));
        }
        for list in lists {
            list.retain(|record| id_of(record).is_some_and(|id| ids.contains(&id)));
        }
        Ok(())
    }
}

fn names(records: &[&Value]) -> Vec<Value> {
    records
        .iter()
        .filter_map(|record| {
            let name = ["title", "name"]
                .iter()
                .find_map(|key| record[*key].as_str().filter(|name| !name.is_empty()));
            Some(json!({ "id": record["id"].as_str()?, "name": name.unwrap_or_else(|| record["id"].as_str().unwrap_or_default()) }))
        })
        .collect()
}

/// Counts and lists what a full import of `content` would create.
pub fn inventory(content: &Value) -> Value {
    let len = |key: &str| content[key].as_array().map_or(0, Vec::len);
    let libraries = content["libraries"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|library| {
            json!({
                "title": library["title"],
                "presentations": library["itemIds"].as_array().map_or(0, Vec::len),
            })
        })
        .collect::<Vec<_>>();
    let counts = json!({
        "playlists": len("playlist"),
        "media": len("media"),
        "themes": crate::authoring::reusable_themes(&content["themes"]).count(),
        "stageLayouts": len("stageLayouts"),
        "props": len("props"),
        "masks": len("masks"),
        "macros": len("macros"),
        "messages": len("messages"),
        "groups": len("groups"),
        "timers": len("countdownTimers"),
        "presets": len("mediaEffectPresets") + len("transitionPresets"),
        "settings": len("triggerBindings") + len("communicationDevices") + len("clearGroups")
            + len("audienceLooks") + len("videoInputs")
            + len("audioInputs") + len("recordingPresets"),
    });
    let list = |keys: &[&str]| {
        let records = keys
            .iter()
            .flat_map(|key| content[*key].as_array().into_iter().flatten())
            .collect::<Vec<_>>();
        names(&records)
    };
    let items = json!({
        "playlists": list(&["playlist"]),
        "media": list(&["media"]),
        "themes": names(&crate::authoring::reusable_themes(&content["themes"]).collect::<Vec<_>>()),
        "stageLayouts": list(&["stageLayouts"]),
        "props": list(&["props"]),
        "masks": list(&["masks"]),
        "macros": list(&["macros"]),
        "messages": list(&["messages"]),
        "groups": list(&["groups"]),
        "timers": list(&["countdownTimers"]),
        "presets": list(&["mediaEffectPresets", "transitionPresets"]),
    });
    json!({ "libraries": libraries, "counts": counts, "items": items })
}
