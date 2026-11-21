//! Saving settings through the settings lane, with the revision bookkeeping of `invokeSettingsMutation`.
//! Settings are written only here, in the UI process; the output process never reads or writes them.

use gpui::{App, AppContext, Task};
use serde_json::{json, Map, Value};

use crate::state::runtime::Runtime;

const STALE: &str = "changed since revision";

pub fn shown(runtime: &Runtime, cx: &App) -> Value {
    let content = runtime.content.read(cx);
    let output = runtime.output.read(cx);
    let mut shown = output.settings_drafts.shown(&output.settings);
    for section in ["screenConfig", "groupSettings", "keyMappings"] {
        let base = content
            .workspace
            .get(section)
            .filter(|value| value.as_object().is_some_and(|fields| !fields.is_empty()))
            .unwrap_or(&shown[section]);
        shown[section] = output
            .settings_drafts
            .section(&content.workspace_path, section, base);
    }
    shown
}

pub(crate) fn begin_mutation(
    runtime: &Runtime,
    cx: &mut App,
    method: &str,
    mutation: &Value,
) -> Result<Option<u64>, String> {
    let (section, desired) = match (method, mutation["type"].as_str()) {
        ("mutateScreenConfig", Some("update" | "toggleGroup" | "setGroupEnabled")) => (
            "screenConfig",
            orbit_host::mutate_screen_config_value(&shown(runtime, cx)["screenConfig"], mutation)?
                .screen_config,
        ),
        ("mutateGroupSettings", Some("update")) => (
            "groupSettings",
            orbit_host::mutate_group_settings_value(
                &shown(runtime, cx)["groupSettings"],
                mutation,
            )?
            .group_settings,
        ),
        ("mutateKeyMappings", Some("set")) => (
            "keyMappings",
            orbit_host::mutate_key_mappings_value(&shown(runtime, cx)["keyMappings"], mutation)?.0,
        ),
        _ => return Ok(None),
    };
    let before = shown(runtime, cx);
    let path = runtime.content.read(cx).workspace_path.clone();
    let activation = section == "screenConfig"
        && (mutation["type"] == "setGroupEnabled" || mutation["patch"].get("enabled").is_some());
    let activation_patch = if activation {
        let output = runtime.output.read(cx);
        let active = output.snapshot.as_ref().map_or_else(
            || before[section].clone(),
            |snapshot| {
                output
                    .settings_drafts
                    .section(&path, section, &snapshot[section])
            },
        );
        let next = orbit_host::mutate_screen_config_value(&active, mutation)?.screen_config;
        // A switch can change the saved state, the runtime state, or both (for example after launch).
        Some(
            next.as_object()
                .into_iter()
                .flatten()
                .filter(|(id, value)| {
                    active.get(*id) != Some(*value) || before[section].get(*id) != desired.get(*id)
                })
                .map(|(id, value)| (id.clone(), value.clone()))
                .collect(),
        )
    } else {
        None
    };
    Ok(Some(runtime.output.update(cx, |output, cx| {
        let request = match activation_patch {
            Some(patch) => output
                .settings_drafts
                .begin_section_patch(&path, section, patch),
            None => {
                output
                    .settings_drafts
                    .begin_section(&path, section, &before[section], &desired)
            }
        };
        if section == "screenConfig" {
            output.invalidate_scene();
        }
        cx.notify();
        request
    })))
}

pub(crate) fn finish_mutation(runtime: &Runtime, cx: &mut App, request: Option<u64>) {
    if let Some(request) = request {
        runtime.output.update(cx, |output, cx| {
            output.settings_drafts.finish(request);
            output.invalidate_scene();
            cx.notify();
        });
    }
}

/// The settings `createSettingsSaver` sends: the global sections the settings panels edit. Screen, group, key
/// mapping, and timer edits have native mutations, so they are never part of this payload.
pub fn save_payload(settings: &Value, overrides: &[(&str, Value)]) -> Value {
    let mut payload = Map::new();
    for key in [
        "presenterOrbitWorkspacePath",
        "generalSettings",
        "audioSettings",
        "remoteSettings",
    ] {
        if let Some(value) = settings.get(key) {
            payload.insert(key.to_string(), value.clone());
        }
    }
    for (key, value) in overrides {
        payload.insert((*key).to_string(), value.clone());
    }
    Value::Object(payload)
}

/// `updateGeneralSettings`: the saved general settings with `patch` applied, as one save.
pub fn general_payload(settings: &Value, patch: &Value) -> Value {
    let mut general = settings["generalSettings"].clone();
    if let (Some(target), Some(patch)) = (general.as_object_mut(), patch.as_object()) {
        target.extend(patch.clone());
    }
    save_payload(settings, &[("generalSettings", general)])
}

/// Runs one settings-lane host call. When `adopt` is set the answer is the complete settings and replaces what
/// every window sees; otherwise the host's `settings-changed` event does. A stale revision refreshes the
/// settings first and still reports the error, so the next attempt starts from the saved state.
pub fn run(
    runtime: &Runtime,
    cx: &mut App,
    adopt: bool,
    call: impl FnOnce(&orbit_host::Host, u64) -> Result<Value, String> + Send + 'static,
) -> Task<Result<Value, String>> {
    let host = runtime.host.clone();
    let answer = {
        let host = host.clone();
        runtime
            .lanes
            .settings(move |revision| call(&host, revision))
    };
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        let result = answer
            .await
            .map_err(|_| "The settings queue stopped.".to_string())
            .and_then(|result| result);
        match &result {
            Ok(saved) if adopt => {
                let saved = saved.clone();
                cx.update(|cx| apply(&runtime, saved, cx));
            }
            Ok(_) => {}
            Err(error) if error.contains(STALE) => {
                let revisions = runtime.lanes.revisions().clone();
                let fresh = cx
                    .background_spawn(async move {
                        let fresh = host.invoke("getSettings", Vec::new())?;
                        revisions.remember_settings(&fresh);
                        Ok::<_, String>(fresh)
                    })
                    .await;
                if let Ok(fresh) = fresh {
                    cx.update(|cx| apply(&runtime, fresh, cx));
                }
            }
            Err(_) => {}
        }
        result
    })
}

/// Makes a host answer the settings of this process, unless a newer one is already known.
pub fn apply(runtime: &Runtime, settings: Value, cx: &mut App) {
    if !runtime.lanes.revisions().accept_settings_event(&settings) {
        return;
    }
    let shown = runtime.output.read(cx).settings_drafts.shown(&settings);
    if let Err(error) = crate::appearance::apply_host_event(cx, "settings-changed", &shown) {
        eprintln!("Presenter Orbit ignored the saved settings: {error}");
    }
    runtime.output.update(cx, |output, cx| {
        output.set_settings(settings);
        cx.notify();
    });
}

/// `saveSettings` with the given global sections.
pub fn save(runtime: &Runtime, cx: &mut App, payload: Value) -> Task<Result<Value, String>> {
    #[cfg(test)]
    runtime
        .sent
        .borrow_mut()
        .push(("saveSettings", payload.clone()));
    save_preview(runtime, cx, payload, "saveSettings")
}

fn save_preview(
    runtime: &Runtime,
    cx: &mut App,
    payload: Value,
    method: &'static str,
) -> Task<Result<Value, String>> {
    let (request, patch, shown) = runtime.output.update(cx, |output, cx| {
        let canonical = output.settings.clone();
        let (request, patch) = output.settings_drafts.begin(&canonical, &payload);
        let shown = output.settings_drafts.shown(&canonical);
        cx.notify();
        (request, patch, shown)
    });
    if let Err(error) = crate::appearance::apply_host_event(cx, "settings-changed", &shown) {
        runtime.output.update(cx, |output, cx| {
            output.settings_drafts.finish(request);
            cx.notify();
        });
        return Task::ready(Err(error));
    }
    let task = run(runtime, cx, true, move |host, revision| {
        let current = host.invoke("getSettings", vec![])?;
        let mut payload = json!({});
        for key in patch.as_object().into_iter().flatten().map(|(key, _)| key) {
            if let Some(value) = current.get(key) {
                payload[key] = value.clone();
            }
        }
        super::drafts::merge(&mut payload, &patch);
        if method == "applyAudioSettings" {
            let audio = payload
                .get("audioSettings")
                .cloned()
                .unwrap_or_else(|| current["audioSettings"].clone());
            host.invoke(method, vec![audio, json!(revision)])?;
            host.invoke("getSettings", vec![])
        } else {
            host.invoke(method, vec![payload, json!(revision)])
        }
    });
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let shown = runtime.output.update(cx, |output, cx| {
                output.settings_drafts.finish(request);
                cx.notify();
                output.settings_drafts.shown(&output.settings)
            });
            crate::appearance::apply_host_event(cx, "settings-changed", &shown)
        })?;
        result
    })
}

/// `applyAudioSettings`: saves the audio settings and re-applies the active outputs. The program audio sinks
/// live in the output process, which reads the settings from the next scene (`audioSettings`), so no output
/// process call is needed here.
pub fn apply_audio(runtime: &Runtime, cx: &mut App, audio: Value) -> Task<Result<Value, String>> {
    #[cfg(test)]
    runtime
        .sent
        .borrow_mut()
        .push(("applyAudioSettings", audio.clone()));
    save_preview(
        runtime,
        cx,
        json!({"audioSettings":audio}),
        "applyAudioSettings",
    )
}

/// A native settings mutation that also writes the active workspace (`mutateGroupSettings`,
/// `mutateKeyMappings`, `mutateScreenConfig`): `invokeSettingsWorkspaceMutation`. It waits in the workspace queue
/// and reads both revisions when it starts. The canonical workspace of the answer replaces the content, and the
/// host's `settings-changed` event replaces the settings. A stale revision refreshes both and reports the error.
pub fn mutate(
    runtime: &Runtime,
    cx: &mut App,
    method: &'static str,
    mutation: Value,
) -> Task<Result<Value, String>> {
    let preview = match begin_mutation(runtime, cx, method, &mutation) {
        Ok(request) => request,
        Err(error) => return Task::ready(Err(error)),
    };
    #[cfg(test)]
    runtime.sent.borrow_mut().push((method, mutation.clone()));
    let path = runtime.content.read(cx).workspace_path.clone();
    let (host, revisions) = (runtime.host.clone(), runtime.lanes.revisions().clone());
    let answer = if path.is_empty() {
        let host = host.clone();
        runtime.lanes.settings(move |revision| {
            host.invoke(method, vec![Value::Null, mutation, json!(revision)])
        })
    } else {
        let (host, revisions, scope) = (host.clone(), revisions.clone(), path.clone());
        runtime.lanes.workspace(&path, move |workspace_revision| {
            let result = host.invoke(
                method,
                vec![
                    json!(scope),
                    mutation,
                    json!(revisions.settings()),
                    json!(workspace_revision),
                ],
            )?;
            revisions.remember_settings(&result);
            Ok(result)
        })
    };
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        let result = answer
            .await
            .map_err(|_| "The settings queue stopped.".to_string())
            .and_then(|result| result);
        match &result {
            Ok(accepted) => {
                let workspace = accepted["workspace"].clone();
                let parsed = cx
                    .background_spawn(async move {
                        if workspace.is_null() {
                            Ok(None)
                        } else {
                            crate::collections::model::Snapshot::parse(&workspace).map(Some)
                        }
                    })
                    .await;
                cx.update(|cx| {
                    if runtime.content.read(cx).workspace_path != path
                        || accepted["revision"].as_u64().is_some_and(|revision| {
                            revision < runtime.lanes.revisions().workspace(&path)
                        })
                    {
                        return;
                    }
                    if let Ok(Some(snapshot)) = parsed {
                        runtime.content.update(cx, |content, cx| {
                            content.apply_canonical(snapshot);
                            cx.notify();
                        });
                    }
                });
            }
            Err(error) if error.contains(STALE) || error.contains("Workspace changed since") => {
                let scope = path.clone();
                let refreshed = cx
                    .background_spawn(async move {
                        let settings = host.invoke("getSettings", Vec::new())?;
                        revisions.remember_settings(&settings);
                        let snapshot = if scope.is_empty() {
                            None
                        } else {
                            let loaded =
                                host.invoke("readWorkspaceSnapshot", vec![json!(scope)])?;
                            revisions.remember_workspace(&scope, &loaded);
                            Some(crate::collections::model::Snapshot::parse(
                                &loaded["workspace"],
                            )?)
                        };
                        Ok::<_, String>((settings, snapshot))
                    })
                    .await;
                if let Ok((settings, snapshot)) = refreshed {
                    cx.update(|cx| {
                        apply(&runtime, settings, cx);
                        if runtime.content.read(cx).workspace_path == path {
                            if let Some(snapshot) = snapshot {
                                runtime.content.update(cx, |content, cx| {
                                    content.apply_canonical(snapshot);
                                    cx.notify();
                                });
                            }
                        }
                    });
                }
            }
            Err(_) => {}
        }
        cx.update(|cx| finish_mutation(&runtime, cx, preview));
        result
    })
}
