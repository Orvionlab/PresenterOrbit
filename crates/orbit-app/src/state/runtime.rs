//! The composition root of the state runtime: the slice entities, the persistence lanes, and the optimistic then
//! canonical persistence flow of `workspace-content-persistence.ts`.
//!
//! Every semantic mutation goes through [`Runtime::persist`]. It applies the optimistic change to the slices, sends
//! the mutation through the workspace lane with the revision accepted so far, and replaces the optimistic data
//! with the canonical workspace the host returns. A stale revision refreshes the slices from
//! `readWorkspaceSnapshot`, never `loadWorkspace`, and replays the mutation once only if it addresses stable ids.

use gpui::{App, AppContext, AsyncApp, Entity};
use orbit_host::Host;
use orbit_ui::Translator;
use serde_json::{json, Value};
use std::{cell::RefCell, rc::Rc};

use super::lane::Lanes;
use super::slices::{Content, Status, Ui};
use crate::collections::model::Snapshot;
use crate::collections::search::Search;

/// The host's message for a revision conflict. A conflict is the only error that is refreshed and replayed.
const STALE_REVISION: &str = "Workspace changed since revision";

type PreviewWrites = Rc<RefCell<Option<futures::channel::oneshot::Receiver<()>>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Library,
    Playlist,
}

impl Kind {
    fn method(self) -> &'static str {
        match self {
            Self::Library => "mutatePresentationLibrary",
            Self::Playlist => "mutatePresentationPlaylist",
        }
    }
}

/// The optimistic change of one mutation, applied to the content and the transient state before the host answers.
pub type Optimistic = Box<dyn FnOnce(&mut Content, &mut Ui)>;

pub struct Persist {
    pub kind: Kind,
    pub mutation: Value,
    pub optimistic: Optimistic,
    pub content_preview: Option<u64>,
}

impl Persist {
    pub fn new(
        kind: Kind,
        mutation: Value,
        optimistic: impl FnOnce(&mut Content, &mut Ui) + 'static,
    ) -> Self {
        Self {
            kind,
            mutation,
            optimistic: Box::new(optimistic),
            content_preview: None,
        }
    }
}

/// A host call that belongs to the output: on the output process when the host routes there, otherwise on the
/// host itself. Blocks.
pub fn output_call(host: &Host, method: &str, args: Vec<Value>) -> Result<Value, String> {
    match host.try_state::<crate::output_process::OutputRoute>() {
        None => host.invoke(method, args),
        Some(route) => route
            .0
            .get()
            .ok_or("The output process has not started yet.")?
            .invoke(method, args),
    }
}

#[derive(Clone)]
pub struct Runtime {
    pub host: Host,
    pub lanes: Lanes,
    preview_writes: PreviewWrites,
    pub content: Entity<Content>,
    pub ui: Entity<Ui>,
    pub status: Entity<Status>,
    pub search: Entity<Search>,
    pub output: Entity<crate::output::scene::Output>,
    pub frames: crate::output::preview::Frames,
    pub text_files: Entity<super::text_files::TextFiles>,
    /// Every mutation sent to the host, in order, for the tests that replay them.
    #[cfg(test)]
    pub sent: std::rc::Rc<std::cell::RefCell<Vec<(&'static str, Value)>>>,
}

/// `isReplaySafePresentationLibraryMutation`: intents addressed by stable ids can run again on a refreshed
/// workspace. Content, structural, and position-based intents cannot.
pub fn replay_safe_library_mutation(mutation: &Value) -> bool {
    const SAFE: [&str; 15] = [
        "create",
        "createPresentation",
        "rename",
        "setTheme",
        "reorder",
        "delete",
        "copyItems",
        "moveItems",
        "deleteItems",
        "createArrangement",
        "duplicateArrangement",
        "renameArrangement",
        "setSelectedArrangement",
        "deleteArrangement",
        "addArrangementSection",
    ];
    const SAFE_ITEM_OPERATIONS: [&str; 6] = [
        "setTitle",
        "setOperatorNotes",
        "setSlideEnabled",
        "setSlideGroup",
        "setSlideTheme",
        "setSlideMedia",
    ];
    match mutation["type"].as_str() {
        Some("createPresentation") if is_set(mutation, "placeholderId") => false,
        Some("mutateItem") if mutation["operation"]["type"] == "insertMediaSlides" => {
            non_empty_array(&mutation["operation"], "newSlideIds")
        }
        Some("mutateItem") => mutation["operation"]["type"]
            .as_str()
            .is_some_and(|operation| SAFE_ITEM_OPERATIONS.contains(&operation)),
        Some(kind) => SAFE.contains(&kind),
        None => false,
    }
}

fn is_set(mutation: &Value, field: &str) -> bool {
    !mutation[field].is_null()
}

fn non_empty_array(mutation: &Value, field: &str) -> bool {
    mutation[field]
        .as_array()
        .is_some_and(|list| !list.is_empty())
}

/// `isReplaySafePresentationPlaylistMutation`: entry mutations that address an occurrence by position replay only
/// when they carry complete anchors, because a refreshed list could put another entry at that index.
pub fn replay_safe_playlist_mutation(mutation: &Value) -> bool {
    match mutation["type"].as_str() {
        Some("deleteEntry") => non_empty_array(mutation, "entryAnchors"),
        Some("setPlaceholderLink") => {
            is_set(mutation, "entryIndex") && is_set(mutation, "expectedEntry")
        }
        Some("reorderEntries") => {
            mutation["entryAnchors"].as_array().map(Vec::len)
                == mutation["entryIds"].as_array().map(Vec::len)
                && is_set(mutation, "targetEntryIndex")
                && is_set(mutation, "targetExpectedEntry")
        }
        Some("copyEntry") => {
            let has_target_anchor =
                is_set(mutation, "targetEntryIndex") || is_set(mutation, "expectedTargetEntry");
            let complete_target_anchor =
                is_set(mutation, "targetEntryIndex") && is_set(mutation, "expectedTargetEntry");
            let has_target_id = mutation["targetEntryId"]
                .as_str()
                .is_some_and(|id| !id.is_empty());
            if (has_target_anchor || has_target_id) && !complete_target_anchor {
                return false;
            }
            if mutation.get("sourcePlaylistId").is_none() {
                return true;
            }
            is_set(mutation, "sourceEntryIndex") && is_set(mutation, "expectedSourceEntry")
        }
        Some(_) => true,
        None => false,
    }
}

impl Kind {
    fn replay_safe(self, mutation: &Value) -> bool {
        match self {
            Self::Library => replay_safe_library_mutation(mutation),
            Self::Playlist => replay_safe_playlist_mutation(mutation),
        }
    }
}

/// What the lane's answer carries: the response without its workspace, and the workspace parsed off the UI thread.
struct Accepted {
    response: Value,
    snapshot: Snapshot,
}

impl Runtime {
    /// Keep an edit visible over earlier workspace snapshots while its write is queued.
    pub fn save_preview(
        &self,
        cx: &mut App,
        kind: Kind,
        mutation: Value,
        change: impl Fn(&mut Content) + 'static,
    ) -> gpui::Task<()> {
        let (id, load) = self.content.update(cx, |content, cx| {
            let id = content.preview_content(Rc::new(change));
            cx.notify();
            (id, content.load_generation)
        });
        let mut request = Persist::new(kind, mutation, |_, _| {});
        request.content_preview = Some(id);
        self.queue_preview_write(cx, request, load)
    }

    fn queue_preview_write(&self, cx: &mut App, request: Persist, load: u64) -> gpui::Task<()> {
        let (finished, next) = futures::channel::oneshot::channel();
        let previous = self.preview_writes.replace(Some(next));
        let runtime = self.clone();
        cx.spawn(async move |cx| {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            if cx.update(|cx| runtime.content.read(cx).load_generation) == load {
                runtime.persist(cx, request).await;
            }
            let _ = finished.send(());
        })
    }

    pub fn new(host: Host, lanes: Lanes, cx: &mut App) -> Self {
        let runtime = Self {
            host,
            lanes,
            preview_writes: Default::default(),
            content: cx.new(|_| Content::default()),
            ui: cx.new(|_| Ui::default()),
            status: cx.new(|_| Status::default()),
            search: cx.new(|_| Search::new()),
            output: cx.new(|_| crate::output::scene::Output::default()),
            frames: Default::default(),
            text_files: cx.new(|_| super::text_files::TextFiles::default()),
            #[cfg(test)]
            sent: Default::default(),
        };
        super::text_files::start(&runtime, cx);
        runtime
    }

    /// Changes the transient state with the content at hand, and notifies the observers.
    pub fn edit_ui(&self, cx: &mut App, change: impl FnOnce(&Content, &mut Ui)) {
        self.ui.update(cx, |ui, cx| {
            change(self.content.read(cx), ui);
            cx.notify();
        });
    }

    pub fn translator(cx: &App) -> Translator {
        *cx.global::<Translator>()
    }

    /// Runs one host call on a lane thread and parses its workspace on a background thread.
    async fn run(
        &self,
        cx: &AsyncApp,
        kind: Kind,
        path: &str,
        mutation: &Value,
    ) -> Result<Accepted, String> {
        #[cfg(test)]
        self.sent
            .borrow_mut()
            .push((kind.method(), mutation.clone()));
        let (host, path_for_call, mutation_for_call) =
            (self.host.clone(), path.to_string(), mutation.clone());
        let answer = self.lanes.workspace(path, move |revision| {
            host.invoke(
                kind.method(),
                vec![json!(path_for_call), mutation_for_call, json!(revision)],
            )
        });
        let response = answer
            .await
            .map_err(|_| "The workspace queue stopped.".to_string())??;
        parse_response(cx, response).await
    }

    /// `readWorkspaceSnapshot` outside the queue: it reads, it never activates a workspace.
    pub async fn read_snapshot(&self, cx: &AsyncApp, path: &str) -> Result<Snapshot, String> {
        let (host, lanes_revisions, path) = (
            self.host.clone(),
            self.lanes.revisions().clone(),
            path.to_string(),
        );
        let loaded = cx
            .background_spawn(async move {
                let response = host.invoke("readWorkspaceSnapshot", vec![json!(path)])?;
                lanes_revisions.remember_workspace(&path, &response);
                Ok::<_, String>(response)
            })
            .await?;
        parse_response(cx, loaded)
            .await
            .map(|accepted| accepted.snapshot)
    }

    fn current_path(&self, cx: &AsyncApp) -> String {
        cx.update(|cx| self.content.read(cx).workspace_path.clone())
    }

    fn apply_canonical(&self, cx: &AsyncApp, snapshot: Snapshot) {
        cx.update(|cx| {
            self.content.update(cx, |content, cx| {
                content.apply_canonical(snapshot);
                cx.notify();
            });
            crate::cues::refresh_live(self, cx);
        });
    }

    pub fn accept_created_presentation(&self, payload: &Value, cx: &mut App) {
        let runtime = self.clone();
        let response = payload.clone();
        cx.spawn(async move |cx| {
            let Ok(accepted) = parse_response(cx, response).await else {
                return;
            };
            let path = &accepted.snapshot.workspace_path;
            let revisions = runtime.lanes.revisions();
            if runtime.current_path(cx) == *path
                && accepted.response["revision"]
                    .as_u64()
                    .is_some_and(|revision| revision >= revisions.workspace(path))
            {
                revisions.remember_workspace(path, &accepted.response);
                runtime.apply_canonical(cx, accepted.snapshot);
            }
        })
        .detach();
    }

    fn set_status(&self, cx: &AsyncApp, key: &str, values: &[(&str, &str)]) {
        cx.update(|cx| {
            let message = Self::translator(cx).format(key, values);
            self.status.update(cx, |status, cx| {
                status.message = message;
                cx.notify();
            });
        });
    }

    /// The optimistic then canonical flow. Returns the host's response (without its workspace) when it accepted
    /// the mutation, and `None` when there is no workspace, the workspace changed meanwhile, or the host rejected
    /// the mutation; a rejection is reported in the status line.
    pub async fn persist(&self, cx: &AsyncApp, request: Persist) -> Option<Value> {
        let Persist {
            kind,
            mutation,
            optimistic,
            content_preview,
        } = request;
        let (path, before, requested) = cx.update(|cx| {
            let before = self.ui.read(cx).clone();
            self.content.update(cx, |content, cx| {
                self.ui.update(cx, |ui, cx| {
                    optimistic(content, ui);
                    // `applyRequestedWorkspaceState` closes the context menu and the item rename on every request.
                    ui.document_menu = None;
                    ui.rename_item_id.clear();
                    cx.notify();
                });
                cx.notify();
            });
            let requested = self.ui.read(cx).clone();
            (
                self.content.read(cx).workspace_path.clone(),
                before,
                requested,
            )
        });
        if path.is_empty() {
            return None;
        }

        let result = self.run_with_replay(cx, kind, &path, &mutation).await;
        match result {
            Ok(None) => None,
            Ok(Some(accepted)) => {
                if self.current_path(cx) != path {
                    return None;
                }
                if let Some(id) = content_preview {
                    cx.update(|cx| {
                        self.content
                            .update(cx, |content, _| content.finish_content_preview(id))
                    });
                }
                if accepted.response["revision"]
                    .as_u64()
                    .is_none_or(|revision| revision >= self.lanes.revisions().workspace(&path))
                {
                    self.apply_canonical(cx, accepted.snapshot);
                }
                Some(accepted.response)
            }
            Err(error) => {
                if self.current_path(cx) == path {
                    if let Some(id) = content_preview {
                        cx.update(|cx| {
                            self.content
                                .update(cx, |content, _| content.finish_content_preview(id))
                        });
                        if let Ok(snapshot) = self.read_snapshot(cx, &path).await {
                            if self.current_path(cx) == path {
                                self.apply_canonical(cx, snapshot);
                            }
                        }
                    }
                    if kind == Kind::Playlist {
                        cx.update(|cx| {
                            self.ui.update(cx, |ui, cx| {
                                ui.restore_unchanged(&before, &requested);
                                cx.notify();
                            });
                        });
                    }
                    self.set_status(cx, "status.workspace.saveFailed", &[("error", &error)]);
                }
                None
            }
        }
    }

    /// `Ok(None)` means the active workspace changed while the mutation ran.
    async fn run_with_replay(
        &self,
        cx: &AsyncApp,
        kind: Kind,
        path: &str,
        mutation: &Value,
    ) -> Result<Option<Accepted>, String> {
        match self.run(cx, kind, path, mutation).await {
            Ok(accepted) => Ok(Some(accepted)),
            Err(error) if error.contains(STALE_REVISION) => {
                let snapshot = self.read_snapshot(cx, path).await?;
                if self.current_path(cx) != path {
                    return Ok(None);
                }
                self.apply_canonical(cx, snapshot);
                if !kind.replay_safe(mutation) {
                    return Err(error);
                }
                self.run(cx, kind, path, mutation).await.map(Some)
            }
            Err(error) => Err(error),
        }
    }
}

async fn parse_response(cx: &AsyncApp, mut response: Value) -> Result<Accepted, String> {
    cx.background_spawn(async move {
        let workspace = response["workspace"].take();
        let snapshot = Snapshot::parse(&workspace)?;
        Ok(Accepted { response, snapshot })
    })
    .await
}
