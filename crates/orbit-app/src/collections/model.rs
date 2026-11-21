//! The library, playlist, and presentation data the collection panels show, and the pure helpers of
//! `collection-data-utils.ts`, `collection-identity-utils.ts`, and `playlist-tree-utils.ts`.
//!
//! The host stays the owner of the full JSON. These types keep only what the panels and the optimistic updates
//! need, plus the raw JSON of what a later semantic mutation has to quote back (playlist entry anchors and
//! clipboard snapshots).

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{json, Map, Value};

/// An explicit theme assignment of a library or playlist. A missing `presentationThemeId` inherits, `null` is
/// "no theme".
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ThemeAssignment {
    #[default]
    Inherit,
    NoTheme,
    Theme(String),
}

impl ThemeAssignment {
    fn from_object(object: &Map<String, Value>) -> Self {
        match object.get("presentationThemeId") {
            None => Self::Inherit,
            Some(Value::Null) => Self::NoTheme,
            Some(Value::String(id)) => Self::Theme(id.clone()),
            Some(_) => Self::Inherit,
        }
    }

    /// The `assignment` of `mutatePresentationLibrary` / `mutatePresentationPlaylist` `setTheme`.
    pub fn mutation_value(&self) -> Value {
        match self {
            Self::Inherit => json!("inherit"),
            Self::NoTheme => Value::Null,
            Self::Theme(id) => json!(id),
        }
    }

    /// `parsePresentationThemeAction`: `presentation-theme:<value>`.
    pub fn parse_action(action: &str) -> Option<Self> {
        let value = action.strip_prefix("presentation-theme:")?;
        Some(match value {
            "inherit" => Self::Inherit,
            "none" => Self::NoTheme,
            id => Self::Theme(id.to_string()),
        })
    }

    pub fn action(&self) -> String {
        match self {
            Self::Inherit => "presentation-theme:inherit".into(),
            Self::NoTheme => "presentation-theme:none".into(),
            Self::Theme(id) => format!("presentation-theme:{id}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    pub id: String,
    pub title: String,
    pub item_ids: Vec<String>,
    pub theme: ThemeAssignment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Presentation,
    Header,
    Placeholder,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Presentation => "presentation",
            Self::Header => "header",
            Self::Placeholder => "placeholder",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub kind: EntryKind,
    pub item_id: String,
    pub title: String,
    pub linked_item_id: String,
    pub arrangement_id: String,
    pub color: String,
    /// The normalized entry as the host stores it, quoted back as `expectedEntry` in anchored mutations.
    pub raw: Value,
}

impl Entry {
    /// `normalizePlaylistEntry`. A presentation without a document id is dropped.
    pub fn normalize(entry: &Value) -> Option<Self> {
        let object = entry.as_object()?;
        let text = |key: &str| object.get(key).and_then(Value::as_str).unwrap_or("");
        let kind = match text("type") {
            "header" => EntryKind::Header,
            "placeholder" => EntryKind::Placeholder,
            _ => EntryKind::Presentation,
        };
        let mut raw = object.clone();
        let (id, item_id, title) = if kind == EntryKind::Presentation {
            let item_id = [text("itemId"), text("id")]
                .into_iter()
                .find(|value| !value.is_empty())
                .unwrap_or("");
            if item_id.is_empty() {
                return None;
            }
            let id = if text("id").is_empty() {
                format!("presentation-{item_id}")
            } else {
                text("id").to_string()
            };
            raw.insert("itemId".into(), json!(item_id));
            (id, item_id.to_string(), text("title").to_string())
        } else {
            let title = [text("title"), text("name")]
                .into_iter()
                .find(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if kind == EntryKind::Header {
                        "Header"
                    } else {
                        "Placeholder"
                    }
                    .to_string()
                });
            let id = if text("id").is_empty() {
                format!("{}-{}", kind.as_str(), slug_id(&title))
            } else {
                text("id").to_string()
            };
            raw.insert("title".into(), json!(title));
            (id, String::new(), title)
        };
        raw.insert("id".into(), json!(id));
        raw.insert("type".into(), json!(kind.as_str()));
        Some(Self {
            id,
            kind,
            item_id,
            title,
            linked_item_id: text("linkedItemId").to_string(),
            arrangement_id: text("arrangementId").to_string(),
            color: text("color").to_string(),
            raw: Value::Object(raw),
        })
    }

    /// The document a row of this entry shows, for a presentation or a linked placeholder.
    pub fn linked_document(&self) -> Option<&str> {
        match self.kind {
            EntryKind::Presentation => Some(&self.item_id),
            EntryKind::Placeholder if !self.linked_item_id.is_empty() => Some(&self.linked_item_id),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    /// `playlist` unless the host stored another type, such as `folder`.
    pub kind: String,
    pub parent_id: String,
    pub collapsed: bool,
    pub theme: ThemeAssignment,
    pub entries: Vec<Entry>,
}

impl Playlist {
    pub fn visible_item_ids(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Presentation && entry.raw["hidden"] != true)
            .map(|entry| entry.item_id.clone())
            .collect()
    }
    pub fn is_folder(&self) -> bool {
        self.kind == "folder"
    }

    /// `getPlaylistItemIds`.
    pub fn item_ids(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Presentation)
            .map(|entry| entry.item_id.clone())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arrangement {
    pub id: String,
    pub name: String,
    pub section_ids: Vec<String>,
}

/// A presentation. The panels use the title and the arrangement names; `value` is the full document for the
/// search index and clipboard copies.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub arrangements: Vec<Arrangement>,
    pub value: Arc<Value>,
}

impl PartialEq for Item {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && Arc::ptr_eq(&self.value, &other.value)
    }
}

impl Item {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let id = value["id"]
            .as_str()
            .ok_or("A library item has no id.")?
            .to_string();
        let arrangements = value["arrangements"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|entry| {
                        Some(Arrangement {
                            id: entry["id"].as_str()?.to_string(),
                            name: entry["name"].as_str().unwrap_or("").to_string(),
                            section_ids: entry["sectionIds"]
                                .as_array()
                                .map(|ids| {
                                    ids.iter()
                                        .filter_map(|id| id.as_str().map(str::to_string))
                                        .collect()
                                })
                                .unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            id,
            title: value["title"].as_str().unwrap_or("").to_string(),
            arrangements,
            value: Arc::new(value.clone()),
        })
    }

    /// `getArrangementDisplayTitle`.
    pub fn display_title(&self, arrangement_id: &str) -> String {
        match self.arrangements.iter().find(|a| a.id == arrangement_id) {
            Some(arrangement) if !arrangement_id.is_empty() => {
                format!("{} [{}]", self.title, arrangement.name)
            }
            _ => self.title.clone(),
        }
    }

    pub fn with_title(&self, title: &str) -> Self {
        let mut value = (*self.value).clone();
        value["title"] = json!(title);
        Self {
            title: title.to_string(),
            value: Arc::new(value),
            ..self.clone()
        }
    }
}

/// The presentations with an index by id. Cloning is cheap, so a background thread can take the whole list.
#[derive(Clone, Debug, Default)]
pub struct Items {
    list: Arc<Vec<Item>>,
    index: Arc<HashMap<String, usize>>,
}

impl Items {
    pub fn new(list: Vec<Item>) -> Self {
        let index = list
            .iter()
            .enumerate()
            .map(|(position, item)| (item.id.clone(), position))
            .collect();
        Self {
            list: Arc::new(list),
            index: Arc::new(index),
        }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Item> {
        self.list.iter()
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.index.get(id).map(|position| &self.list[*position])
    }

    pub fn contains(&self, id: &str) -> bool {
        self.index.contains_key(id)
    }

    /// The full documents in order, as `replaceLibrarySearchIndex` takes them.
    pub fn to_values(&self) -> Value {
        Value::Array(self.list.iter().map(|item| (*item.value).clone()).collect())
    }

    pub fn map_item(&self, id: &str, change: impl Fn(&Item) -> Item) -> Self {
        Self::new(
            self.list
                .iter()
                .map(|item| {
                    if item.id == id {
                        change(item)
                    } else {
                        item.clone()
                    }
                })
                .collect(),
        )
    }

    pub fn without(&self, removed: &HashSet<String>) -> Self {
        Self::new(
            self.list
                .iter()
                .filter(|item| !removed.contains(&item.id))
                .cloned()
                .collect(),
        )
    }

    /// `getItemsByIds`: the items in the order of `ids`, skipping unknown ids.
    pub fn by_ids<'a>(&'a self, ids: &'a [String]) -> impl Iterator<Item = &'a Item> {
        ids.iter().filter_map(|id| self.get(id))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub id: String,
    pub title: String,
    /// `(id, name)` of each theme slide.
    pub slides: Vec<(String, String)>,
}

/// Everything the collection panels take from a workspace response.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Full canonical authoring data for the output scene builder and native stills.
    pub raw: Arc<Value>,
    pub workspace_path: String,
    pub libraries: Vec<Library>,
    pub items: Items,
    pub playlists: Vec<Playlist>,
    pub themes: Vec<Theme>,
    pub last_used_theme: Option<String>,
    /// Theme layout assignments, most recently picked first.
    pub recent_themes: Vec<String>,
}

fn required<'a>(workspace: &'a Value, field: &str) -> Result<&'a Vec<Value>, String> {
    workspace[field]
        .as_array()
        .ok_or_else(|| format!("The workspace has no valid '{field}' field."))
}

impl Snapshot {
    /// Parses the `workspace` of a host response. Missing required fields are an error, not a default.
    pub fn parse(workspace: &Value) -> Result<Self, String> {
        let workspace_path = workspace["workspacePath"]
            .as_str()
            .ok_or("The workspace has no valid 'workspacePath' field.")?
            .to_string();
        workspace["title"]
            .as_str()
            .ok_or("The workspace has no valid 'title' field.")?;
        let libraries = required(workspace, "libraries")?
            .iter()
            .map(|library| {
                let object = library.as_object().ok_or("A library is not an object.")?;
                Ok(Library {
                    id: library["id"].as_str().ok_or("A library has no id.")?.into(),
                    title: library["title"].as_str().unwrap_or("").into(),
                    item_ids: library["itemIds"]
                        .as_array()
                        .map(|ids| {
                            ids.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    theme: ThemeAssignment::from_object(object),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let items = Items::new(
            required(workspace, "library")?
                .iter()
                .map(Item::parse)
                .collect::<Result<Vec<_>, _>>()?,
        );
        let playlists = required(workspace, "playlist")?
            .iter()
            .map(|playlist| {
                let object = playlist.as_object().ok_or("A playlist is not an object.")?;
                Ok(Playlist {
                    id: playlist["id"]
                        .as_str()
                        .ok_or("A playlist has no id.")?
                        .into(),
                    title: playlist["title"].as_str().unwrap_or("").into(),
                    kind: playlist["type"].as_str().unwrap_or("playlist").into(),
                    parent_id: playlist["parentId"].as_str().unwrap_or("").into(),
                    collapsed: playlist["collapsed"].as_bool().unwrap_or(false),
                    theme: ThemeAssignment::from_object(object),
                    entries: playlist["items"]
                        .as_array()
                        .map(|entries| entries.iter().filter_map(Entry::normalize).collect())
                        .unwrap_or_default(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let themes: Vec<Theme> = orbit_host::authoring::reusable_themes(&workspace["themes"])
            .filter_map(|theme| {
                Some(Theme {
                    id: theme["id"].as_str()?.into(),
                    title: theme["title"].as_str().unwrap_or("").into(),
                    slides: theme["slides"]
                        .as_array()
                        .map(|slides| {
                            slides
                                .iter()
                                .filter_map(|slide| {
                                    Some((
                                        slide["id"].as_str()?.to_string(),
                                        slide["name"]
                                            .as_str()
                                            .or_else(|| slide["title"].as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                    ))
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
            })
            .collect();
        let last_used_theme = workspace["lastUsedPresentationThemeId"]
            .as_str()
            .and_then(|assignment| {
                let (theme, slide) = assignment.split_once("::")?;
                themes
                    .iter()
                    .find(|candidate| candidate.id == theme)?
                    .slides
                    .iter()
                    .any(|(id, _)| id == slide)
                    .then(|| assignment.to_string())
            })
            .or_else(|| {
                themes.iter().find_map(|theme| {
                    let (slide, _) = theme.slides.first()?;
                    Some(format!("{}::{slide}", theme.id))
                })
            });
        Ok(Self {
            raw: Arc::new(workspace.clone()),
            workspace_path,
            libraries,
            items,
            playlists,
            themes,
            last_used_theme,
            recent_themes: workspace["recentPresentationThemeIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str().map(str::to_string))
                .collect(),
        })
    }
}

// ---- Identity helpers (`collection-identity-utils.ts`) ----

/// `slugId`: lowercase, runs of other characters become one dash, no leading or trailing dash.
pub fn slug_id(value: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for character in value.to_lowercase().chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(character);
        } else {
            pending_dash = true;
        }
    }
    slug
}

pub fn unique_collection_id(prefix: &str, title: &str, ids: &[&str]) -> String {
    let slug = slug_id(title);
    let base = format!("{prefix}-{}", if slug.is_empty() { "item" } else { &slug });
    let mut id = base.clone();
    let mut index = 2;
    while ids.contains(&id.as_str()) {
        id = format!("{base}-{index}");
        index += 1;
    }
    id
}

pub fn unique_title(base: &str, titles: &[&str]) -> String {
    let taken = |title: &str| titles.iter().any(|existing| existing.trim() == title);
    if !taken(base) {
        return base.to_string();
    }
    let mut index = 2;
    while taken(&format!("{base} {index}")) {
        index += 1;
    }
    format!("{base} {index}")
}

/// `createCopyName`: the first `<name> <n>` from 1 that is free.
pub fn copy_name(name: &str, existing: &[&str]) -> String {
    let base = if name.trim().is_empty() {
        "Media"
    } else {
        name.trim()
    };
    let mut index = 1;
    while existing
        .iter()
        .any(|title| title.trim() == format!("{base} {index}"))
    {
        index += 1;
    }
    format!("{base} {index}")
}

pub fn unique_entry_title(base: &str, entries: &[Entry]) -> String {
    let titles: Vec<&str> = entries
        .iter()
        .map(|entry| entry.title.trim())
        .filter(|title| !title.is_empty())
        .collect();
    unique_title(base, &titles)
}

pub fn unique_entry_id(kind: EntryKind, title: &str, entries: &[Entry]) -> String {
    let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
    unique_collection_id(kind.as_str(), title, &ids)
}

// ---- Playlist tree (`playlist-tree-utils.ts`) ----

/// A parent that does not exist counts as the root.
pub fn parent_id<'a>(playlists: &'a [Playlist], playlist: &'a Playlist) -> &'a str {
    if !playlist.parent_id.is_empty() && playlists.iter().any(|item| item.id == playlist.parent_id)
    {
        &playlist.parent_id
    } else {
        ""
    }
}

pub fn visible_ids(playlists: &[Playlist]) -> Vec<String> {
    fn visit(
        playlists: &[Playlist],
        parent: &str,
        visited: &mut Vec<String>,
        ids: &mut Vec<String>,
    ) {
        for playlist in playlists {
            if parent_id(playlists, playlist) != parent || visited.contains(&playlist.id) {
                continue;
            }
            ids.push(playlist.id.clone());
            visited.push(playlist.id.clone());
            visit(playlists, &playlist.id, visited, ids);
            visited.pop();
        }
    }
    let mut ids = Vec::new();
    visit(playlists, "", &mut Vec::new(), &mut ids);
    ids
}

pub fn is_descendant(playlists: &[Playlist], candidate: &str, parent: &str) -> bool {
    let mut current = playlists.iter().find(|playlist| playlist.id == candidate);
    let mut steps = 0;
    while let Some(playlist) = current {
        if playlist.parent_id.is_empty() || steps > playlists.len() {
            return false;
        }
        if playlist.parent_id == parent {
            return true;
        }
        current = playlists.iter().find(|item| item.id == playlist.parent_id);
        steps += 1;
    }
    false
}

pub fn last_visible_descendant_id(playlists: &[Playlist], id: &str) -> String {
    let visible = visible_ids(playlists);
    let Some(start) = visible.iter().position(|candidate| candidate == id) else {
        return id.to_string();
    };
    let mut last = id.to_string();
    for candidate in &visible[start + 1..] {
        if !is_descendant(playlists, candidate, id) {
            break;
        }
        last = candidate.clone();
    }
    last
}

pub fn root_ancestor_id(playlists: &[Playlist], id: &str) -> String {
    let mut root = id.to_string();
    let mut current = playlists.iter().find(|playlist| playlist.id == id);
    let mut steps = 0;
    while let Some(playlist) = current {
        let parent = parent_id(playlists, playlist);
        if parent.is_empty() || steps > playlists.len() {
            break;
        }
        root = parent.to_string();
        current = playlists.iter().find(|item| item.id == parent);
        steps += 1;
    }
    root
}

pub fn is_after_folder_as_root_drop(playlists: &[Playlist], target: &str) -> bool {
    let Some(playlist) = playlists.iter().find(|item| item.id == target) else {
        return false;
    };
    if parent_id(playlists, playlist).is_empty() {
        return false;
    }
    last_visible_descendant_id(playlists, &root_ancestor_id(playlists, target)) == target
}

/// `getPlaylistDescendantIds`: the playlist and everything below it.
pub fn descendant_ids(playlists: &[Playlist], id: &str) -> HashSet<String> {
    let mut ids: HashSet<String> = [id.to_string()].into();
    loop {
        let before = ids.len();
        for playlist in playlists {
            if !playlist.parent_id.is_empty() && ids.contains(&playlist.parent_id) {
                ids.insert(playlist.id.clone());
            }
        }
        if ids.len() == before {
            return ids;
        }
    }
}

/// `getPlaylistInsertionAnchor`.
pub fn insertion_anchor(playlists: &[Playlist], target: &str, mode: DropMode) -> String {
    match mode {
        DropMode::AfterRoot => {
            last_visible_descendant_id(playlists, &root_ancestor_id(playlists, target))
        }
        DropMode::After => last_visible_descendant_id(playlists, target),
        _ => target.to_string(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropMode {
    Before,
    After,
    AfterRoot,
    Inside,
}

impl DropMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
            Self::AfterRoot => "after-root",
            Self::Inside => "inside",
        }
    }
}

// ---- Ordering of the document list (`sortLibraryItemsByTitle`) ----

/// `localeCompare` with `numeric: true` and `sensitivity: "base"`, without accent folding: case-insensitive, and
/// runs of digits compare as numbers.
pub fn natural_cmp(first: &str, second: &str) -> Ordering {
    let mut a = first.chars().flat_map(char::to_lowercase).peekable();
    let mut b = second.chars().flat_map(char::to_lowercase).peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut left = String::new();
                while let Some(digit) = a.next_if(char::is_ascii_digit) {
                    left.push(digit);
                }
                let mut right = String::new();
                while let Some(digit) = b.next_if(char::is_ascii_digit) {
                    right.push(digit);
                }
                let (left, right) = (left.trim_start_matches('0'), right.trim_start_matches('0'));
                let order = left.len().cmp(&right.len()).then_with(|| left.cmp(right));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                a.next();
                b.next();
                let order = x.cmp(&y);
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// The documents of a library in display order.
pub fn sorted_library_items<'a>(items: &'a Items, library: &'a Library) -> Vec<&'a Item> {
    let mut sorted: Vec<&Item> = items.by_ids(&library.item_ids).collect();
    sorted.sort_by(|first, second| natural_cmp(&first.title, &second.title));
    sorted
}
