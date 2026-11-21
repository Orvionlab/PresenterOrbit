use gpui::{App, AppContext};
use serde_json::{json, Value};

use crate::state::runtime::Runtime;

pub fn group_live(runtime: &Runtime, stage: bool, cx: &App) -> bool {
    runtime
        .output
        .read(cx)
        .snapshot
        .as_ref()
        .is_some_and(|snapshot| {
            group_entries(snapshot, stage)
                .iter()
                .any(|(_, config)| config["enabled"] == true)
        })
}

fn group_snapshot(runtime: &Runtime, cx: &App) -> Option<Value> {
    let output = runtime.output.read(cx);
    let snapshot = output.snapshot.as_ref()?;
    Some(json!({"screenConfig": output.settings_drafts.section(
        &runtime.content.read(cx).workspace_path, "screenConfig", &snapshot["screenConfig"]
    )}))
}

fn group_entries(snapshot: &Value, stage: bool) -> Vec<(String, Value)> {
    super::view::screens(snapshot)
        .into_iter()
        .filter(|(id, config)| (config["type"] == "stage" || id.starts_with("stage-")) == stage)
        .collect()
}

pub fn group_mutation(snapshot: &Value, stage: bool) -> Value {
    let entries = group_entries(snapshot, stage);
    json!({"type":"setGroupEnabled", "screenType": if stage {"stage"} else {"audience"},
        "enabled": !entries.iter().all(|(_, config)| config["enabled"] == true)})
}

/// `restoreStartupOutputs` with auto-enable off: every screen starts disabled instead of in its saved state.
pub fn launch_starts_disabled(settings: &Value) -> bool {
    settings["generalSettings"]["autoEnableScreensAtLaunch"] == json!(false)
}

/// Starts every screen disabled in the running output only. The saved activation stays as it was.
pub fn start_disabled(runtime: &Runtime, cx: &mut App) {
    if runtime.content.read(cx).workspace_path.is_empty() {
        return;
    }
    let mut config = crate::settings::persist::shown(runtime, cx)["screenConfig"].clone();
    let result = config
        .as_object_mut()
        .ok_or("The output snapshot has no screen configuration.")
        .and_then(|screens| {
            for screen in screens.values_mut() {
                screen
                    .as_object_mut()
                    .ok_or("Screen configuration must be an object.")?
                    .insert("enabled".into(), json!(false));
            }
            Ok(())
        });
    runtime.output.update(cx, |output, cx| {
        match result {
            Ok(()) => output.edit_live(|live| live["screenConfig"] = config),
            Err(error) => output.device_error = Some(error.into()),
        }
        cx.notify();
    });
}

pub fn toggle_group(runtime: &Runtime, stage: bool, cx: &mut App) {
    let Some(snapshot) = group_snapshot(runtime, cx) else {
        return;
    };
    let mutation = group_mutation(&snapshot, stage);
    mutate_config(runtime, mutation, cx);
}

pub fn refresh_displays(runtime: &Runtime, cx: &mut App) {
    let request = runtime.output.update(cx, |output, _| {
        output.display_request += 1;
        output.display_request
    });
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        let host = runtime.host.clone();
        let result = cx
            .background_spawn(async move {
                let options = host.invoke("getOutputOptions", vec![])?;
                options
                    .get("displays")
                    .filter(|displays| displays.is_array())
                    .cloned()
                    .ok_or_else(|| "Output options have no display list.".to_string())
            })
            .await;
        cx.update(|cx| {
            runtime.output.update(cx, |output, cx| {
                if output.display_request != request {
                    return;
                }
                match result {
                    Ok(displays) => {
                        output.set_displays(displays);
                        output.device_error = None;
                    }
                    Err(error) => output.device_error = Some(error),
                }
                cx.notify();
            })
        });
    })
    .detach();
}

pub fn create_windowed(runtime: &Runtime, screen_id: String, cx: &mut App) {
    let runtime = runtime.clone();
    let workspace = runtime.content.read(cx).workspace_path.clone();
    cx.spawn(async move |cx| {
        let host = runtime.host.clone();
        let result = cx
            .background_spawn(async move {
                crate::state::runtime::output_call(
                    &host,
                    "createWindowedOutput",
                    vec![json!(screen_id)],
                )
            })
            .await;
        cx.update(|cx| {
            if runtime.content.read(cx).workspace_path != workspace {
                return;
            }
            runtime.output.update(cx, |output, cx| {
                if let Err(error) = result {
                    output.device_error = Some(error);
                }
                cx.notify();
            });
        });
    })
    .detach();
}

pub fn closed(runtime: &Runtime, payload: &Value, cx: &mut App) {
    let Some(screen_id) = payload.as_str() else {
        return;
    };
    set_enabled(runtime, screen_id.to_string(), false, cx);
}

/// Screen visibility is a persisted semantic mutation, including key/fill partners.
pub fn set_enabled(runtime: &Runtime, screen_id: String, enabled: bool, cx: &mut App) {
    let path = runtime.content.read(cx).workspace_path.clone();
    if path.is_empty() {
        return;
    }
    let output = runtime.output.read(cx);
    if output
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot["screenConfig"].get(&screen_id))
        .is_none()
    {
        return;
    }
    let mutation =
        json!({ "type": "update", "screenId": screen_id, "patch": { "enabled": enabled } });
    mutate_config(runtime, mutation, cx);
}

pub fn mutate_config(runtime: &Runtime, mutation: Value, cx: &mut App) {
    mutate_config_with(runtime, mutation, None, cx);
}

pub type Revert = Box<dyn FnOnce(&mut App)>;

/// Like [`mutate_config`]; `revert` runs on the UI thread when the host rejects the mutation.
pub fn mutate_config_with(
    runtime: &Runtime,
    mutation: Value,
    revert: Option<Revert>,
    cx: &mut App,
) {
    let path = runtime.content.read(cx).workspace_path.clone();
    if path.is_empty() {
        return;
    }
    let deleted_screen = (mutation["type"] == "delete")
        .then(|| mutation["screenId"].as_str().map(str::to_string))
        .flatten();
    let stage_assignment = matches!(
        mutation["type"].as_str(),
        Some("setStageAssignment" | "clearStageAssignment")
    );
    let preview = match crate::settings::persist::begin_mutation(
        runtime,
        cx,
        "mutateScreenConfig",
        &mutation,
    ) {
        Ok(request) => request,
        Err(error) => {
            runtime.output.update(cx, |output, cx| {
                output.device_error = Some(error);
                cx.notify();
            });
            return;
        }
    };
    if stage_assignment {
        runtime
            .content
            .update(cx, |content, _| content.pending_stage_assignments += 1);
    }
    let host = runtime.host.clone();
    let revisions = runtime.lanes.revisions().clone();
    let requested_path = path.clone();
    let answer = runtime.lanes.workspace(&path, move |revision| {
        let mut response = host.invoke(
            "mutateScreenConfig",
            vec![
                json!(requested_path),
                mutation,
                json!(revisions.settings()),
                json!(revision),
            ],
        )?;
        revisions.remember_settings(&response);
        let config = response["screenConfig"].clone();
        response["nativeOutputPatch"] =
            host.edit_operator_output(&requested_path, move |snapshot| {
                snapshot["screenConfig"] = config;
                Ok(())
            })?;
        Ok(response)
    });
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        let response = match answer.await {
            Ok(result) => result,
            Err(_) => Err("The screen configuration worker stopped.".into()),
        };
        let result = cx
            .background_spawn(async move {
                let response = response?;
                let snapshot = crate::collections::model::Snapshot::parse(&response["workspace"])?;
                Ok::<_, String>((response, snapshot))
            })
            .await;
        cx.update(|cx| {
            crate::settings::persist::finish_mutation(&runtime, cx, preview);
            let newer_pick = stage_assignment
                && runtime.content.update(cx, |content, _| {
                    content.pending_stage_assignments -= 1;
                    content.pending_stage_assignments > 0
                });
            if runtime.content.read(cx).workspace_path != path {
                return;
            }
            match result {
                Ok((response, snapshot)) => {
                    // A newer pick is still saving and its answer carries this one too.
                    let stale = response["revision"].as_u64().is_some_and(|revision| {
                        revision < runtime.lanes.revisions().workspace(&path)
                    });
                    if !newer_pick && !stale {
                        runtime.content.update(cx, |content, cx| {
                            // A screen acknowledgement must not replace newer edits to presets or other content.
                            let workspace = std::sync::Arc::make_mut(&mut content.workspace);
                            for key in ["screenConfig", "stageScreenAssignments"] {
                                workspace[key] = snapshot.raw[key].clone();
                            }
                            if let Some(screen) = &deleted_screen {
                                if let Some(looks) = workspace["audienceLooks"].as_array_mut() {
                                    for look in looks {
                                        if let Some(entries) = look["screenLooks"].as_array_mut() {
                                            entries.retain(|entry| {
                                                entry["screenId"] != screen.as_str()
                                            });
                                        }
                                    }
                                }
                            }
                            cx.notify();
                        });
                    }
                    runtime.output.update(cx, |output, cx| {
                        if response["nativeOutputPatch"]["revision"]
                            .as_u64()
                            .is_some_and(|revision| revision >= output.committed_revision)
                        {
                            if let Err(error) =
                                output.accept_native_patch(&response["nativeOutputPatch"])
                            {
                                output.error = Some(error);
                            }
                        }
                        output.device_error = response["runtimeError"].as_str().map(str::to_string);
                        cx.notify();
                    });
                }
                Err(error) => {
                    if let Some(revert) = revert {
                        revert(cx);
                    }
                    runtime.output.update(cx, |output, cx| {
                        output.device_error = Some(error);
                        cx.notify();
                    })
                }
            }
            // Keep newer pending edits active; a rejected save returns to the canonical configuration.
            let config = crate::settings::persist::shown(&runtime, cx)["screenConfig"].clone();
            runtime.output.update(cx, |output, cx| {
                if output.live.get("screenConfig") != Some(&config) {
                    output.edit_live(|live| live["screenConfig"] = config);
                    cx.notify();
                }
            });
        });
    })
    .detach();
}
