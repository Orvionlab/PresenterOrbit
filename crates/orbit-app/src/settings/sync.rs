//! `WorkspaceSyncSettingsPanel.tsx`, `use-workspace-sync.ts`, and `workspace-sync-runtime.ts`. A preview is bound
//! to the exact request and workspace it was made for; any change to the controls drops it, and applying needs a
//! current one. A download refreshes the content scopes it carried and nothing else.

use std::collections::HashMap;

use super::ui::{Field, OutputField};
use gpui::{div, prelude::*, AnyElement, Context, Global, SharedString, Window};
use orbit_ui::controls::{ActionButton, CheckboxField, Select, SelectOption};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::ui::{action_row, muted, path_for_display, stack, subheading, title};
use super::view::SettingsView;
use crate::collections::model::Snapshot;
use crate::state::runtime::Runtime;

pub const SCOPES: [&str; 4] = ["libraries", "playlists", "media", "themes"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Select,
    Preview,
    Apply,
}

impl Operation {
    fn key(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Preview => "preview",
            Self::Apply => "apply",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Binding {
    generation: u64,
    request: Value,
    workspace: String,
}

#[derive(Debug)]
pub struct SyncPanel {
    pub direction: String,
    pub scopes: Vec<&'static str>,
    pub replace: bool,
    pub preview: Option<Value>,
    pub busy: Option<Operation>,
    pub error: String,
    pub status: Option<&'static str>,
    generation: u64,
    binding: Option<Binding>,
    in_flight: bool,
}

impl Default for SyncPanel {
    fn default() -> Self {
        Self {
            direction: "upload".into(),
            scopes: SCOPES.to_vec(),
            replace: false,
            preview: None,
            busy: None,
            error: String::new(),
            status: None,
            generation: 0,
            binding: None,
            in_flight: false,
        }
    }
}

impl SyncPanel {
    pub fn invalidate(&mut self) {
        self.generation += 1;
        self.binding = None;
        self.preview = None;
        self.error.clear();
        self.status = None;
    }

    pub fn set_direction(&mut self, direction: &str) {
        if self.direction != direction {
            self.direction = direction.to_string();
            self.invalidate();
        }
    }

    /// `updateScopes`: the list keeps the canonical scope order.
    pub fn set_scope(&mut self, scope: &'static str, enabled: bool) {
        self.scopes = SCOPES
            .into_iter()
            .filter(|candidate| {
                if *candidate == scope {
                    enabled
                } else {
                    self.scopes.contains(candidate)
                }
            })
            .collect();
        self.invalidate();
    }

    pub fn set_replace(&mut self, replace: bool) {
        if self.replace != replace {
            self.replace = replace;
            self.invalidate();
        }
    }

    pub fn repository_changed(&mut self) {
        self.invalidate();
    }

    pub fn request(&self, repository: &str) -> Option<Value> {
        (!repository.is_empty() && !self.scopes.is_empty()).then(|| {
            json!({
                "repository": repository,
                "direction": self.direction,
                "scopes": self.scopes,
                "replace": self.replace,
            })
        })
    }

    /// The preview shown: only one made for the current request, workspace, and generation.
    pub fn current_preview(&self, repository: &str, workspace: &str) -> Option<&Value> {
        let binding = self.binding.as_ref()?;
        let current = binding.generation == self.generation
            && binding.workspace == workspace
            && Some(&binding.request) == self.request(repository).as_ref();
        current.then_some(self.preview.as_ref()).flatten()
    }

    pub fn can_preview(&self, repository: &str, workspace: &str) -> bool {
        !workspace.is_empty() && self.request(repository).is_some() && self.busy.is_none()
    }

    pub fn can_apply(&self, repository: &str, workspace: &str) -> bool {
        self.current_preview(repository, workspace)
            .is_some_and(|preview| preview["token"].as_str().is_some_and(|t| !t.is_empty()))
            && self.busy.is_none()
            && !self.in_flight
    }

    pub fn begin_select(&mut self) -> bool {
        if self.in_flight {
            return false;
        }
        self.in_flight = true;
        self.busy = Some(Operation::Select);
        self.error.clear();
        self.status = None;
        true
    }

    pub fn finish_select(&mut self) {
        self.in_flight = false;
        self.busy = None;
    }

    /// Starts a preview: the request, workspace, and generation the answer must still match.
    pub fn begin_preview(&mut self, repository: &str, workspace: &str) -> Option<(Value, u64)> {
        let request = self.request(repository)?;
        if workspace.is_empty() || self.in_flight {
            return None;
        }
        self.in_flight = true;
        self.busy = Some(Operation::Preview);
        self.error.clear();
        self.status = None;
        self.preview = None;
        Some((request, self.generation))
    }

    pub fn finish_preview(
        &mut self,
        generation: u64,
        request: Value,
        workspace: &str,
        result: Result<Value, String>,
    ) {
        self.in_flight = false;
        self.busy = None;
        if generation != self.generation {
            return;
        }
        match result {
            Ok(preview) => {
                self.binding = Some(Binding {
                    generation,
                    request,
                    workspace: workspace.to_string(),
                });
                self.preview = Some(preview);
            }
            Err(error) => self.error = error,
        }
    }

    pub fn begin_apply(
        &mut self,
        repository: &str,
        workspace: &str,
    ) -> Option<(Value, u64, String)> {
        if !self.can_apply(repository, workspace) {
            return None;
        }
        let request = self.request(repository)?;
        let token = self.preview.as_ref()?["token"].as_str()?.to_string();
        self.in_flight = true;
        self.busy = Some(Operation::Apply);
        self.error.clear();
        self.status = None;
        Some((request, self.generation, token))
    }

    pub fn finish_apply(&mut self, generation: u64, applied: bool, error: Option<String>) {
        self.in_flight = false;
        self.busy = None;
        if generation != self.generation {
            return;
        }
        match error {
            Some(error) => self.error = error,
            None if applied => {
                self.preview = None;
                self.binding = None;
                self.status = Some("applied");
            }
            None => {}
        }
    }
}

/// The last revision of each workspace whose sync event was taken, so an older or repeated one is ignored.
#[derive(Default)]
struct Seen(HashMap<String, i64>);

impl Global for Seen {}

fn scope_keys(scope: &str) -> &'static [&'static str] {
    match scope {
        "libraries" => &["libraries", "library"],
        "playlists" => &["playlist"],
        "media" => &["media", "mediaLibraries"],
        "themes" => &["themes"],
        _ => &[],
    }
}

/// `acceptWorkspaceSync`: the scopes a download carried replace those parts of the content. Live output, timers,
/// and machine settings stay as they are.
pub fn accept(runtime: &Runtime, event: &Value, cx: &mut gpui::App) -> Result<(), String> {
    let path = event["workspacePath"].as_str().unwrap_or("");
    let (current_path, workspace) = {
        let content = runtime.content.read(cx);
        (content.workspace_path.clone(), content.workspace.clone())
    };
    if current_path != path {
        return Ok(());
    }
    let revision = event["revision"]
        .as_i64()
        .ok_or("The synchronized workspace has no valid revision.")?;
    if revision
        <= cx
            .default_global::<Seen>()
            .0
            .get(path)
            .copied()
            .unwrap_or(-1)
    {
        return Ok(());
    }
    let scopes: Vec<&str> = event["scopes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut merged = (*workspace).clone();
    for key in scopes.iter().flat_map(|scope| scope_keys(scope)) {
        if !event["workspace"][key].is_array() {
            return Err("Synchronized workspace content is invalid.".into());
        }
        merged[key] = event["workspace"][key].clone();
    }
    let snapshot = Snapshot::parse(&merged)?;
    cx.default_global::<Seen>()
        .0
        .insert(path.to_string(), revision);
    runtime.content.update(cx, |content, cx| {
        if scopes.contains(&"themes") {
            content.load(snapshot);
        } else {
            content.apply_canonical(snapshot);
        }
        cx.notify();
    });
    Ok(())
}

impl SettingsView {
    fn sync_repository(&self, cx: &gpui::App) -> String {
        self.settings(cx)["generalSettings"]["syncRepositoryPath"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    fn sync_workspace(&self, cx: &gpui::App) -> String {
        self.runtime.content.read(cx).workspace_path.clone()
    }

    pub(super) fn choose_sync_repository(&mut self, cx: &mut Context<Self>) {
        if !self.sync.begin_select() {
            return;
        }
        cx.notify();
        self.host_call(
            "selectSyncRepository",
            Vec::new(),
            cx,
            |view, result, cx| {
                view.sync.finish_select();
                match result {
                    Ok(chosen) => {
                        if let Some(path) = chosen["path"].as_str() {
                            view.set_sync_repository(path, cx);
                        }
                    }
                    Err(error) => view.sync.error = error,
                }
                cx.notify();
            },
        );
    }

    pub(super) fn set_sync_repository(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.sync_repository(cx) == path {
            return;
        }
        self.patch_general(json!({ "syncRepositoryPath": path }), cx);
        self.sync.repository_changed();
    }

    pub(super) fn create_sync_preview(&mut self, cx: &mut Context<Self>) {
        let (repository, workspace) = (self.sync_repository(cx), self.sync_workspace(cx));
        let Some((request, generation)) = self.sync.begin_preview(&repository, &workspace) else {
            return;
        };
        cx.notify();
        let sent = request.clone();
        self.host_call(
            "previewWorkspaceSync",
            vec![json!(workspace.clone()), sent],
            cx,
            move |view, result, cx| {
                view.sync
                    .finish_preview(generation, request, &workspace, result);
                cx.notify();
            },
        );
    }

    pub(super) fn apply_sync_preview(&mut self, cx: &mut Context<Self>) {
        let (repository, workspace) = (self.sync_repository(cx), self.sync_workspace(cx));
        let Some((request, generation, token)) = self.sync.begin_apply(&repository, &workspace)
        else {
            return;
        };
        cx.notify();
        #[cfg(test)]
        self.runtime
            .sent
            .borrow_mut()
            .push(("applyWorkspaceSync", json!([workspace, request, token])));
        let (host, runtime) = (self.runtime.host.clone(), self.runtime.clone());
        let (path, sent, download) = (
            workspace.clone(),
            request.clone(),
            request["direction"] == "download",
        );
        let answer = self.runtime.lanes.workspace(&workspace, move |_| {
            host.invoke("applyWorkspaceSync", vec![json!(path), sent, json!(token)])
        });
        cx.spawn(async move |view, cx| {
            let result = answer
                .await
                .map_err(|_| "The workspace queue stopped.".to_string())
                .and_then(|result| result);
            view.update(cx, |view, cx| {
                let t = *cx.global::<Translator>();
                let error = match &result {
                    Ok(applied) if download => {
                        let revision = applied["revision"].as_i64();
                        match revision {
                            None => Some(t.t("settings.sync.missingRevision")),
                            Some(revision) => accept(
                                &runtime,
                                &json!({
                                    "workspacePath": workspace,
                                    "revision": revision,
                                    "scopes": request["scopes"],
                                    "workspace": applied["workspace"],
                                }),
                                cx,
                            )
                            .err(),
                        }
                    }
                    Ok(_) => None,
                    Err(error) => Some(error.clone()),
                };
                view.sync.finish_apply(generation, result.is_ok(), error);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn sync_panel(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let (repository, workspace) = (self.sync_repository(cx), self.sync_workspace(cx));
        let busy = self.sync.busy;
        let applying = busy == Some(Operation::Apply);
        let weak = cx.weak_entity();
        let (direction_weak, replace_weak) = (weak.clone(), weak.clone());
        let mut scopes = stack(cx).child(
            div()
                .text_size(theme.metrics.font_lg)
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(theme.colors.text_strong)
                .child(t.t("settings.sync.scopes")),
        );
        for scope in SCOPES {
            let weak = weak.clone();
            scopes = scopes.child(
                CheckboxField::new(
                    SharedString::from(format!("sync-scope-{scope}")),
                    t.t(&format!("settings.sync.scope.{scope}")),
                    self.sync.scopes.contains(&scope),
                )
                .disabled(applying)
                .on_change(move |checked, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.sync.set_scope(scope, checked);
                        cx.notify();
                    })
                    .ok();
                }),
            );
        }
        let preview = self
            .sync
            .current_preview(&repository, &workspace)
            .cloned()
            .map(|preview| {
                let count = |key: &str| preview[key].to_string();
                let mut block = stack(cx)
                    .id("sync-preview")
                    .child(subheading(t.t("settings.sync.previewTitle"), cx))
                    .child(
                        div()
                            .flex()
                            .gap(theme.metrics.panel_gap)
                            .child(format!("{} {}", t.t("settings.sync.added"), count("added")))
                            .child(format!(
                                "{} {}",
                                t.t("settings.sync.updated"),
                                count("updated")
                            ))
                            .child(format!(
                                "{} {}",
                                t.t("settings.sync.conflicts"),
                                count("conflicts")
                            ))
                            .child(format!(
                                "{} {}",
                                t.t("settings.sync.unchanged"),
                                count("unchanged")
                            )),
                    );
                if preview["conflicts"].as_u64().unwrap_or(0) > 0 {
                    block = block.child(subheading(t.t("settings.sync.conflictDetails"), cx));
                    for entry in preview["entries"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|entry| entry["status"] == "conflict")
                    {
                        block = block.child(div().child(format!(
                            "{}  {}",
                            entry["title"].as_str().unwrap_or(""),
                            t.t(&format!(
                                "settings.sync.scope.{}",
                                entry["scope"].as_str().unwrap_or("")
                            ))
                        )));
                    }
                }
                block
            });
        let (can_preview, can_apply) = (
            self.sync.can_preview(&repository, &workspace),
            self.sync.can_apply(&repository, &workspace),
        );
        stack(cx)
            .id("settings-sync")
            .child(title(t.t("settings.sync.title"), cx))
            .child(muted(t.t("settings.sync.description"), cx))
            .child(OutputField::new(
                t.t("settings.sync.repository"),
                if repository.is_empty() {
                    t.t("settings.sync.noRepository")
                } else {
                    path_for_display(&repository)
                },
            ))
            .child(
                action_row(cx)
                    .child(
                        ActionButton::new("sync-choose", t.t("settings.sync.chooseRepository"))
                            .icon("folder")
                            .disabled(busy.is_some())
                            .on_click(
                                cx.listener(|view, _, _, cx| view.choose_sync_repository(cx)),
                            ),
                    )
                    .children(
                        busy.map(|busy| {
                            muted(t.t(&format!("settings.sync.busy.{}", busy.key())), cx)
                        }),
                    ),
            )
            .child(Field::new(
                t.t("settings.sync.direction"),
                Select::new(
                    "sync-direction",
                    vec![
                        SelectOption::new("upload".to_string(), t.t("settings.sync.upload")),
                        SelectOption::new("download".to_string(), t.t("settings.sync.download")),
                    ],
                    self.sync.direction.clone(),
                )
                .disabled(applying)
                .on_change(move |value, _, cx| {
                    direction_weak
                        .update(cx, |view, cx| {
                            view.sync.set_direction(&value);
                            cx.notify();
                        })
                        .ok();
                }),
            ))
            .child(scopes)
            .child(
                CheckboxField::new(
                    "sync-replace",
                    t.t("settings.sync.replaceConflicts"),
                    self.sync.replace,
                )
                .disabled(applying)
                .on_change(move |checked, _, cx| {
                    replace_weak
                        .update(cx, |view, cx| {
                            view.sync.set_replace(checked);
                            cx.notify();
                        })
                        .ok();
                }),
            )
            .child(muted(t.t("settings.sync.scopeHelp"), cx))
            .child(
                action_row(cx)
                    .child(
                        ActionButton::new("sync-preview-button", t.t("settings.sync.preview"))
                            .disabled(!can_preview)
                            .on_click(cx.listener(|view, _, _, cx| view.create_sync_preview(cx))),
                    )
                    .child(
                        ActionButton::new("sync-apply", t.t("settings.sync.apply"))
                            .disabled(!can_apply)
                            .on_click(cx.listener(|view, _, _, cx| view.apply_sync_preview(cx))),
                    ),
            )
            .children(
                workspace
                    .is_empty()
                    .then(|| muted(t.t("settings.sync.noWorkspace"), cx)),
            )
            .children((!self.sync.error.is_empty()).then(|| {
                div()
                    .id("sync-error")
                    .text_color(theme.colors.warning)
                    .child(format!(
                        "{} {}",
                        t.t("settings.sync.error"),
                        self.sync.error
                    ))
            }))
            .children(
                self.sync
                    .status
                    .map(|status| muted(t.t(&format!("settings.sync.status.{status}")), cx)),
            )
            .children(preview)
            .into_any_element()
    }
}
