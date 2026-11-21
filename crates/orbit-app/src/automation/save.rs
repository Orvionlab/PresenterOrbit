//! How the authoring editors reach the workspace: `saveWorkspaceContent` for whole arrays (macros, messages)
//! and the semantic `mutateCountdownTimers` for timer definitions. Both queue on the workspace
//! lane with the revision accepted so far, and the canonical answer replaces what the panels show.

use gpui::{App, AsyncApp, Context};
use serde_json::{json, Value};

use crate::state::runtime::Runtime;
use gpui::AppContext;

const STALE_REVISION: &str = "Workspace changed since revision";

/// Applies a prepared edit immediately and queues its write in gesture order. Older answers cannot replace newer edits.
pub fn optimistic_content<T: 'static>(
    runtime: &Runtime,
    patch: Value,
    cx: &mut Context<T>,
    finished: impl FnOnce(&mut T, Result<(), String>, &mut Context<T>) + 'static,
) -> Result<(), String> {
    let content = runtime.content.read(cx);
    let path = content.workspace_path.clone();
    let generation = content.load_generation;
    if path.is_empty() {
        return Err("automation.macros.workspaceRequired".into());
    }
    let keys: Vec<String> = patch
        .as_object()
        .ok_or("Workspace content patch must be an object.")?
        .keys()
        .cloned()
        .collect();
    merge(runtime, &patch, cx);
    let (host, scope, submitted) = (runtime.host.clone(), path.clone(), patch.clone());
    let answer = runtime.lanes.workspace(&path, move |revision| {
        host.invoke(
            "saveWorkspaceContent",
            vec![json!(scope), submitted, json!(revision)],
        )
    });
    let runtime = runtime.clone();
    cx.spawn(async move |owner, cx| {
        let result = answer
            .await
            .map_err(|_| "The workspace queue stopped.".to_string())
            .and_then(|response| response);
        let (canonical, outcome) = match result {
            Ok(response) => (Some(response["workspace"].clone()), Ok(())),
            Err(error) => match runtime.read_snapshot(cx, &path).await {
                Ok(snapshot) => (Some(snapshot.raw.as_ref().clone()), Err(error)),
                Err(refresh) => (None, Err(format!("{error}; {refresh}"))),
            },
        };
        let current = cx.update(|cx| {
            let content = runtime.content.read(cx);
            if content.workspace_path != path || content.load_generation != generation {
                return false;
            }
            if let Some(canonical) = canonical {
                runtime.content.update(cx, |content, cx| {
                    let workspace = std::sync::Arc::make_mut(&mut content.workspace);
                    for key in &keys {
                        if workspace.get(key) == patch.get(key) {
                            workspace[key.as_str()] = canonical[key].clone();
                        }
                    }
                    cx.notify();
                });
            }
            true
        });
        if current {
            owner
                .update(cx, |owner, cx| finished(owner, outcome, cx))
                .ok();
        }
    })
    .detach();
    Ok(())
}

fn path_of(runtime: &Runtime, cx: &AsyncApp) -> String {
    cx.update(|cx| runtime.content.read(cx).workspace_path.clone())
}

/// Builds the patch of a save from the workspace as it is when the settings have arrived.
pub type Build = Box<dyn FnOnce(&Runtime, &Value, &mut App) -> Result<Value, String>>;

/// One authoring save: the settings are read first when `needs_settings` (shortcut conflicts), `build` makes the
/// patch from the workspace at that moment, the patch is written, and `finished` hears the outcome.
pub fn commit<T: 'static>(
    runtime: &Runtime,
    needs_settings: bool,
    build: Build,
    cx: &mut Context<T>,
    finished: impl FnOnce(&mut T, Result<(), String>, &mut Context<T>) + 'static,
) {
    let runtime = runtime.clone();
    cx.spawn(async move |this, cx| {
        let settings = if needs_settings {
            let host = runtime.host.clone();
            cx.background_spawn(async move { host.invoke("getSettings", vec![]) })
                .await
        } else {
            Ok(Value::Null)
        };
        let outcome = match settings {
            Ok(settings) => match cx.update(|cx| build(&runtime, &settings, cx)) {
                Ok(patch) => content(&runtime, cx, patch).await.map(|_| ()),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        let _ = this.update(cx, |this, cx| finished(this, outcome, cx));
    })
    .detach();
}

/// Saves `patch` (`{"macros": [...]}` and the like) and shows the canonical workspace. `Ok(false)` means the
/// workspace changed while the save ran, so nothing was shown.
pub async fn content(runtime: &Runtime, cx: &AsyncApp, patch: Value) -> Result<bool, String> {
    let path = path_of(runtime, cx);
    if path.is_empty() {
        return Err("automation.macros.workspaceRequired".into());
    }
    let keys: Vec<String> = patch
        .as_object()
        .into_iter()
        .flat_map(|patch| patch.keys().cloned())
        .collect();
    let (host, scope) = (runtime.host.clone(), path.clone());
    let answer = runtime.lanes.workspace(&path, move |revision| {
        host.invoke(
            "saveWorkspaceContent",
            vec![json!(scope), patch, json!(revision)],
        )
    });
    let response = answer
        .await
        .map_err(|_| "The workspace queue stopped.".to_string())??;
    Ok(cx.update(|cx| {
        if runtime.content.read(cx).workspace_path != path {
            return false;
        }
        merge(runtime, &response["workspace"], cx);
        // The host drops a key that was saved as null, so the answer lacks it; the shown workspace must too.
        runtime.content.update(cx, |content, cx| {
            let workspace = std::sync::Arc::make_mut(&mut content.workspace);
            for key in &keys {
                if response["workspace"].get(key).is_none() {
                    workspace[key.as_str()] = Value::Null;
                }
            }
            cx.notify();
        });
        true
    }))
}

/// A save answers with the patched top-level keys only. They replace those keys of the shown workspace; the
/// libraries and presentations are not part of these edits.
fn merge(runtime: &Runtime, saved: &Value, cx: &mut App) {
    runtime.content.update(cx, |content, cx| {
        let workspace = std::sync::Arc::make_mut(&mut content.workspace);
        for (key, value) in saved.as_object().into_iter().flatten() {
            if key != "workspacePath" {
                workspace[key] = value.clone();
            }
        }
        cx.notify();
    });
}

/// One semantic mutation that answers with the keys it changed (`mutateCountdownTimers`, `mutateProps`,
/// `mutateStageContent`). A revision conflict refreshes the workspace once and replays the same intent, as the
/// renderer's mutation helpers do. Returns the host's answer, after its keys replaced the shown ones.
pub async fn mutation(
    runtime: &Runtime,
    cx: &AsyncApp,
    method: &'static str,
    mutation: Value,
) -> Result<Value, String> {
    let path = path_of(runtime, cx);
    if path.is_empty() {
        return Err("automation.macros.workspaceRequired".into());
    }
    let mut refreshed = false;
    loop {
        let (host, scope, intent) = (runtime.host.clone(), path.clone(), mutation.clone());
        let answer = runtime.lanes.workspace(&path, move |revision| {
            host.invoke(method, vec![json!(scope), intent, json!(revision)])
        });
        let result = answer
            .await
            .map_err(|_| "The workspace queue stopped.".to_string())?;
        match result {
            Ok(response) => {
                cx.update(|cx| shown(runtime, &path, &response, cx));
                return Ok(response);
            }
            Err(error) if !refreshed && error.contains(STALE_REVISION) => {
                refreshed = true;
                let snapshot = runtime.read_snapshot(cx, &path).await?;
                cx.update(|cx| {
                    if runtime.content.read(cx).workspace_path == path {
                        runtime.content.update(cx, |content, cx| {
                            content.apply_canonical(snapshot);
                            cx.notify();
                        });
                    }
                });
            }
            Err(error) => return Err(error),
        }
    }
}

fn shown(runtime: &Runtime, path: &str, response: &Value, cx: &mut App) {
    if runtime.content.read(cx).workspace_path == path {
        merge(runtime, &response["workspace"], cx);
    }
}
