//! The state slices of `runtime-state-slices.ts` that the collection panels need. Each is one `Entity`, so a change
//! only notifies the views that observe that slice.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::collections::model::{
    sorted_library_items, Entry, Items, Library, Playlist, Snapshot, Theme,
};

pub type ContentPreview = Rc<dyn Fn(&mut Content)>;

/// The documents of one library in display order, kept while neither the presentations nor the library's
/// membership change.
struct SortedCache {
    generation: u64,
    library: String,
    membership: Vec<String>,
    ids: Arc<Vec<String>>,
}

/// The workspace content: what the host owns and the panels show.
#[derive(Default)]
pub struct Content {
    pub workspace: Arc<serde_json::Value>,
    pub workspace_path: String,
    pub libraries: Vec<Library>,
    pub items: Items,
    canonical_items: Items,
    pending_slide_enabled: Vec<(u64, String, String, bool)>,
    content_previews: Vec<(u64, ContentPreview)>,
    next_content_preview: u64,
    pub playlists: Vec<Playlist>,
    pub themes: Vec<Theme>,
    pub last_used_theme: Option<String>,
    pub recent_themes: Vec<String>,
    /// Counts replacements of `items`, so the search index and derived rows know when to rebuild.
    pub items_generation: u64,
    pub load_generation: u64,
    /// Stage assignment saves still running; an earlier answer must not overwrite a newer optimistic pick.
    pub pending_stage_assignments: usize,
    sorted: RefCell<Option<SortedCache>>,
}

impl Content {
    /// `applyAcceptedCollectionMutationState`: the canonical libraries, presentations, and playlists replace the
    /// optimistic ones. The theme list is not part of a collection mutation response.
    pub fn apply_canonical(&mut self, snapshot: Snapshot) {
        self.workspace = snapshot.raw;
        self.last_used_theme = snapshot.last_used_theme;
        self.recent_themes = snapshot.recent_themes;
        self.libraries = snapshot.libraries;
        self.items = snapshot.items;
        self.canonical_items = self.items.clone();
        for (_, item, slide, enabled) in self.pending_slide_enabled.clone() {
            self.patch_slide_enabled(&item, &slide, Some(!enabled));
        }
        self.items_generation += 1;
        self.playlists = snapshot.playlists;
        self.workspace_path = snapshot.workspace_path;
        for (_, preview) in self.content_previews.clone() {
            preview(self);
        }
    }

    /// Accept a full document mutation without resetting workspace-scoped output state.
    pub fn apply_document(&mut self, snapshot: Snapshot) {
        self.themes = snapshot.themes.clone();
        self.apply_canonical(snapshot);
    }

    /// A workspace switch or the first load also resets workspace-scoped state.
    pub fn load(&mut self, snapshot: Snapshot) {
        self.load_generation += 1;
        self.pending_slide_enabled.clear();
        self.content_previews.clear();
        self.apply_document(snapshot);
    }

    pub fn preview_content(&mut self, preview: ContentPreview) -> u64 {
        self.next_content_preview += 1;
        let id = self.next_content_preview;
        preview(self);
        self.content_previews.push((id, preview));
        id
    }

    pub fn finish_content_preview(&mut self, id: u64) {
        self.content_previews.retain(|(pending, _)| *pending != id);
    }

    fn patch_slide_enabled(&mut self, item: &str, slide: &str, disabled: Option<bool>) {
        crate::collections::arrangements::edit_item(self, item, |value| {
            if let Some(target) = value["slides"]
                .as_array_mut()
                .and_then(|slides| slides.iter_mut().find(|value| value["id"] == slide))
            {
                if let Some(disabled) = disabled {
                    target["disabled"] = serde_json::json!(disabled);
                } else if let Some(object) = target.as_object_mut() {
                    object.remove("disabled");
                }
            }
        });
    }

    pub fn preview_slide_enabled(&mut self, item: &str, slide: &str, enabled: bool) -> Option<u64> {
        self.items.get(item)?.value["slides"]
            .as_array()?
            .iter()
            .find(|value| value["id"] == slide)?;
        self.patch_slide_enabled(item, slide, Some(!enabled));
        let token = self.items_generation;
        self.pending_slide_enabled
            .push((token, item.into(), slide.into(), enabled));
        Some(token)
    }

    pub fn finish_slide_enabled(&mut self, token: u64) {
        let Some(index) = self
            .pending_slide_enabled
            .iter()
            .position(|entry| entry.0 == token)
        else {
            return;
        };
        let (_, item, slide, _) = self.pending_slide_enabled.remove(index);
        let disabled = self
            .canonical_items
            .get(&item)
            .and_then(|item| item.value["slides"].as_array())
            .and_then(|slides| slides.iter().find(|value| value["id"] == slide))
            .and_then(|slide| slide["disabled"].as_bool());
        self.patch_slide_enabled(&item, &slide, disabled);
        for (_, item, slide, enabled) in self.pending_slide_enabled.clone() {
            self.patch_slide_enabled(&item, &slide, Some(!enabled));
        }
    }

    pub fn library(&self, id: &str) -> Option<&Library> {
        self.libraries.iter().find(|library| library.id == id)
    }

    pub fn destination_pending(&self, id: &str) -> bool {
        let Some((current, canonical)) = self.items.get(id).zip(self.canonical_items.get(id))
        else {
            return false;
        };
        current.value["destination"] != canonical.value["destination"]
            && current
                .value
                .as_object()
                .zip(canonical.value.as_object())
                .is_some_and(|(current, canonical)| {
                    current
                        .iter()
                        .filter(|(key, _)| *key != "destination")
                        .all(|(key, value)| canonical.get(key) == Some(value))
                        && canonical
                            .keys()
                            .filter(|key| *key != "destination")
                            .all(|key| current.contains_key(key))
                })
    }

    pub fn playlist(&self, id: &str) -> Option<&Playlist> {
        self.playlists.iter().find(|playlist| playlist.id == id)
    }

    /// The ids of a library's documents sorted by title. Sorting 10,000 titles is not something to repeat on every
    /// click or arrow key, so the order is kept until the presentations or the membership change.
    pub fn sorted_document_ids(&self, library: &Library) -> Arc<Vec<String>> {
        if let Some(cache) = self.sorted.borrow().as_ref() {
            if cache.generation == self.items_generation
                && cache.library == library.id
                && cache.membership == library.item_ids
            {
                return cache.ids.clone();
            }
        }
        let ids: Arc<Vec<String>> = Arc::new(
            sorted_library_items(&self.items, library)
                .into_iter()
                .map(|item| item.id.clone())
                .collect(),
        );
        *self.sorted.borrow_mut() = Some(SortedCache {
            generation: self.items_generation,
            library: library.id.clone(),
            membership: library.item_ids.clone(),
            ids: ids.clone(),
        });
        ids
    }

    pub fn library_id_for_item(&self, item_id: &str) -> &str {
        self.libraries
            .iter()
            .find(|library| library.item_ids.iter().any(|id| id == item_id))
            .map_or("", |library| library.id.as_str())
    }

    pub fn replace_items(&mut self, items: Items) {
        self.items = items;
        self.items_generation += 1;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Library,
    Playlist,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Selected {
    pub item_id: String,
    pub index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub source: Source,
    pub playlist_id: String,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TreeMenu {
    pub id: String,
    pub ids: Vec<String>,
    pub position: (f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DocumentMenu {
    /// The item id for a library row, the entry id for a playlist row.
    pub id: String,
    pub source: Source,
    pub index: usize,
    pub position: (f32, f32),
}

/// Which "+" menu is open: the library tree's, the playlist tree's, or the document list's in a library or a
/// playlist.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CreateMenuKind {
    LibraryTree,
    PlaylistTree,
    Library,
    Playlist,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateMenu {
    pub kind: CreateMenuKind,
    pub position: (f32, f32),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct NewPresentation {
    pub title: String,
    pub library_id: String,
    pub playlist_id: String,
    pub placeholder_id: Option<String>,
    pub theme_id: String,
    pub size: (u32, u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArrangementEditor {
    pub item_id: String,
}

/// The new presentation dialog (`newPresentationDraft`, `newPresentationSaving`, `newPresentationError`).
#[derive(Clone, Debug, PartialEq)]
pub struct NewPresentationState {
    pub draft: NewPresentation,
    pub saving: bool,
    pub error: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Clipboard {
    /// Full snapshots of the copied presentations.
    pub items: Vec<serde_json::Value>,
    pub entry: Option<Entry>,
}

/// The selection, rename, menu, and clipboard state of the collection panels. Nothing here is persisted.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Ui {
    pub selected_library_id: String,
    pub selected_library_ids: Vec<String>,
    pub selected_library_item_id: String,
    pub library_anchor: Option<usize>,
    pub selected_playlist_id: String,
    pub selected_playlist_ids: Vec<String>,
    pub playlist_anchor: Option<usize>,
    pub selected_documents: Vec<Selected>,
    pub playlist_reveal: Option<(String, String)>,
    pub selected_document_source: Option<Source>,
    pub selected_document_playlist_id: String,
    pub document_anchor: Option<Anchor>,
    pub rename_library_id: String,
    pub rename_playlist_id: String,
    pub rename_item_id: String,
    pub rename_entry_id: String,
    pub library_menu: Option<TreeMenu>,
    pub playlist_menu: Option<TreeMenu>,
    pub document_menu: Option<DocumentMenu>,
    pub header_editor: Option<crate::collections::headers::Request>,
    pub create_menu: Option<CreateMenu>,
    pub clipboard: Clipboard,
    pub new_presentation: Option<NewPresentationState>,
    /// `arrangementEditor`: the presentation and arrangement the arrangement editor shows.
    pub arrangement_editor: Option<ArrangementEditor>,
}

impl Ui {
    /// `restoreUnchangedTransientWorkspaceState`: every field that still holds the value the optimistic update
    /// gave it goes back to its value before the update. A field the user changed since stays.
    pub fn restore_unchanged(&mut self, before: &Ui, requested: &Ui) {
        macro_rules! restore {
            ($($field:ident),* $(,)?) => {
                $(if self.$field == requested.$field { self.$field = before.$field.clone(); })*
            };
        }
        restore!(
            selected_library_id,
            selected_library_ids,
            selected_library_item_id,
            library_anchor,
            selected_playlist_id,
            selected_playlist_ids,
            playlist_anchor,
            selected_documents,
            selected_document_source,
            selected_document_playlist_id,
            document_anchor,
            rename_library_id,
            rename_playlist_id,
            rename_item_id,
            rename_entry_id,
            library_menu,
            playlist_menu,
            document_menu,
            create_menu,
            clipboard,
            new_presentation,
            arrangement_editor,
        );
    }

    /// `getLoadedWorkspaceSelection`: the first library that has documents, and its first document.
    pub fn select_loaded(&mut self, content: &Content) {
        self.header_editor = None;
        self.playlist_reveal = None;
        let first = content
            .libraries
            .iter()
            .find(|library| !library.item_ids.is_empty())
            .or_else(|| content.libraries.first());
        let first_item = first.and_then(|library| {
            content
                .items
                .iter()
                .find(|item| library.item_ids.contains(&item.id))
        });
        self.selected_library_id = first.map(|library| library.id.clone()).unwrap_or_default();
        self.selected_library_ids = first
            .map(|library| vec![library.id.clone()])
            .unwrap_or_default();
        self.library_anchor = None;
        self.selected_playlist_id.clear();
        self.selected_playlist_ids.clear();
        self.playlist_anchor = None;
        self.selected_library_item_id = first_item.map(|item| item.id.clone()).unwrap_or_default();
        self.selected_documents.clear();
        self.document_anchor = None;
    }

    pub fn close_menus(&mut self) {
        self.library_menu = None;
        self.playlist_menu = None;
        self.document_menu = None;
        self.create_menu = None;
    }
}

/// The status line text (`importStatus`).
#[derive(Default)]
pub struct Status {
    pub message: String,
}
