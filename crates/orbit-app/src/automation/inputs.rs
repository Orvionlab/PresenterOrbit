//! Device events are consumed only by the controller. Device I/O stays in the host.
use crate::state::runtime::Runtime;
use gpui::{App, AppContext, Global};
use serde_json::{json, Value};
#[derive(Default)]
pub struct Inputs {
    midi: bool,
    generation: u64,
    queued: usize,
    path: String,
    devices: Value,
    pub midi_status: Value,
    pub communication_status: Vec<Value>,
}
impl Global for Inputs {}
pub struct Commands {
    pub window: gpui::AnyWindowHandle,
    pub commands: crate::keys::Commands,
}
impl Global for Commands {}
pub fn start(runtime: &Runtime, cx: &mut App) {
    super::artnet::reset(cx);
    cx.set_global(Inputs::default());
    let (host, status_runtime) = (runtime.host.clone(), runtime.clone());
    cx.spawn(async move |cx| {
        let result = cx
            .background_spawn(async move { host.invoke("getCommunicationInputStatus", vec![]) })
            .await;
        cx.update(|cx| match result {
            Ok(v) => cx.global_mut::<Inputs>().communication_status = crate::services::entries(&v),
            Err(e) => crate::cues::actions::error(&status_runtime, cx, e),
        });
    })
    .detach();
    let runtime = runtime.clone();
    cx.observe(&runtime.content.clone(), move |_, cx| {
        synchronize(&runtime, cx)
    })
    .detach();
}
fn synchronize(runtime: &Runtime, cx: &mut App) {
    let required = runtime.content.read(cx).workspace["triggerBindings"]
        .as_array()
        .is_some_and(|list| {
            list.iter()
                .any(|b| b["midi"].is_object() && b["targetId"].is_string())
        });
    let content = runtime.content.read(cx);
    let path = content.workspace_path.clone();
    let devices = content.workspace["communicationDevices"].clone();
    let state = cx.global_mut::<Inputs>();
    if state.path != path || state.devices != devices {
        state.generation += 1;
        state.path = path;
        state.devices = devices;
        state.communication_status.clear();
    }
    if state.midi == required {
        return;
    }
    state.midi = required;
    let (host, runtime) = (runtime.host.clone(), runtime.clone());
    cx.spawn(async move |cx| {
        let result = cx
            .background_spawn(
                async move { host.invoke("setMidiInputEnabled", vec![json!(required)]) },
            )
            .await;
        if let Err(error) = result {
            cx.update(|cx| crate::cues::actions::error(&runtime, cx, error));
        }
    })
    .detach();
}
pub fn command(runtime: &Runtime, name: &str, cx: &mut App) -> Result<(), String> {
    match name {
        "artnetFixture" => return Err("Art-Net fixture requires its channel packet.".into()),
        "transportBeginning" | "transportPlay" | "transportPause" | "transportToggle" => {
            let action = match name {
                "transportBeginning" => "seek",
                "transportPlay" => "play",
                "transportPause" => "pause",
                _ => "toggle",
            };
            let (host, runtime) = (runtime.host.clone(), runtime.clone());
            let path = runtime.content.read(cx).workspace_path.clone();
            cx.spawn(async move |cx| {
                let result = cx
                    .background_spawn(async move {
                        crate::state::runtime::output_call(
                            &host,
                            "controlPlayback",
                            vec![json!({"layer":"media","action":action,"position":0})],
                        )
                    })
                    .await;
                cx.update(|cx| {
                    if runtime.content.read(cx).workspace_path != path {
                        return;
                    }
                    match result {
                        Ok(v) => runtime.output.update(cx, |output, cx| {
                            if let Err(e) = output.accept_native_patch(&v) {
                                output.error = Some(e);
                            }
                            cx.notify();
                        }),
                        Err(e) => crate::cues::actions::error(&runtime, cx, e),
                    }
                });
            })
            .detach();
        }
        "timelineStart" | "timelineStop" | "timelineBeginning" => {
            let content = runtime.content.read(cx);
            let selected = &runtime.ui.read(cx).selected_library_item_id;
            let item = content
                .items
                .get(selected)
                .ok_or("Art-Net requires a selected presentation with a timeline.")?;
            let current = &runtime.output.read(cx).live["timelineTransport"];
            if current["mode"] == "recording" {
                return Err("Art-Net cannot override timeline recording.".into());
            }
            let same = current["itemId"] == item.value["id"] && current["mode"] == "playback";
            let at = super::announcements::now_ms();
            let mut patch = super::timeline::transport_patch(
                &item.value,
                &item.value["timeline"],
                0.,
                name == "timelineStart"
                    || (name == "timelineBeginning" && same && !current["startedAtMs"].is_null()),
                at,
                1.,
                &uuid::Uuid::new_v4().to_string(),
            )?;
            if name == "timelineStop" {
                patch["timelineTransport"]["mode"] = json!("stopped");
            } else if name == "timelineStart"
                && same
                && super::timeline::position(current, at)
                    < item.value["timeline"]["durationSeconds"]
                        .as_f64()
                        .ok_or("Timeline duration, cue time or target is invalid.")?
            {
                patch = json!({"timelineTransport":current});
                if current["startedAtMs"].is_null() {
                    patch["timelineTransport"]["startedAtMs"] = json!(at as i64);
                }
            }
            super::timeline::apply(runtime, patch, cx);
        }
        "nextItem" | "previousItem" => {
            let mut moved = None;
            runtime.edit_ui(cx, |content, ui| {
                moved = crate::collections::selection::navigate_document_item(
                    content,
                    ui,
                    if name == "nextItem" { 1 } else { -1 },
                )
            });
            if moved.is_none() {
                return Err("The selected playlist has no navigable presentation.".into());
            }
        }
        "nextSlide" => crate::cues::actions::navigate(runtime, 1, cx),
        "previousSlide" => crate::cues::actions::navigate(runtime, -1, cx),
        "clearAll" => crate::cues::actions::clear(runtime, "all", cx),
        "clearSlide" => crate::cues::actions::clear(runtime, "slide", cx),
        "clearMedia" => crate::cues::actions::clear(runtime, "media", cx),
        "clearProps" => crate::cues::actions::clear(runtime, "props", cx),
        "clearAudio" => crate::cues::actions::clear(runtime, "music", cx),
        "clearMessages" => crate::cues::actions::clear(runtime, "messages", cx),
        "clearVideoInput" => crate::cues::actions::clear(runtime, "videoInput", cx),
        "clearLogo" => {
            if runtime.output.read(cx).live["backgroundId"] == "__presenter-orbit-logo" {
                crate::cues::actions::clear(runtime, "media", cx);
            }
        }
        "clearAnnouncements" => crate::cues::actions::clear(runtime, "announcements", cx),
        _ => {
            let route = cx
                .try_global::<Commands>()
                .map(|r| (r.window, r.commands.clone()));
            let Some((window, commands)) =
                route.filter(|(_, commands)| commands.is_registered(name))
            else {
                return Err(format!("Unsupported input command: {name}"));
            };
            let name = name.to_string();
            let runtime = runtime.clone();
            cx.defer(move |cx| {
                if let Err(error) = cx.update_window(window, |_, window, cx| {
                    commands.run(&name, false, window, cx)
                }) {
                    crate::cues::actions::error(&runtime, cx, error.to_string());
                }
            });
        }
    }
    Ok(())
}
pub fn binding_action(workspace: &Value, selected: &str, binding: &Value) -> Result<Value, String> {
    let kind = binding["type"]
        .as_str()
        .ok_or("The binding target or input is invalid.")?;
    let target = binding["targetId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("The binding target or input is invalid.")?;
    if matches!(kind, "cue" | "group") {
        let items = workspace["library"]
            .as_array()
            .ok_or("The binding target or input is invalid.")?;
        let item = items
            .iter()
            .find(|item| {
                if kind == "group" {
                    item["id"] == selected
                } else {
                    item["slides"].as_array().is_some_and(|slides| {
                        slides.iter().any(|s| {
                            s["sourceCueId"].as_str().or_else(|| s["id"].as_str()) == Some(target)
                        })
                    })
                }
            })
            .ok_or("The binding target or input is invalid.")?;
        let cues = orbit_host::Host::presentation_cues(
            item,
            crate::cues::model::selected_arrangement_id(item),
            None,
        )?;
        let slide = cues
            .iter()
            .find(|s| {
                s["disabled"] != true
                    && if kind == "cue" {
                        s["sourceCueId"].as_str().or_else(|| s["id"].as_str()) == Some(target)
                    } else {
                        s["sourceGroupId"] == target || s["groupId"] == target
                    }
            })
            .ok_or("The binding target or input is invalid.")?;
        return Ok(
            json!({"id":binding["id"],"type":"presentation","targetId":item["id"],"details":{"slideId":slide["id"],"cueInstanceId":slide["arrangementInstanceId"]}}),
        );
    }
    let field = match kind {
        "macro" => "macros",
        "prop" => "props",
        "timer" => "countdownTimers",
        "clear-group" => "clearGroups",
        _ => return Err("The binding target or input is invalid.".into()),
    };
    if !workspace[field]
        .as_array()
        .is_some_and(|list| list.iter().any(|v| v["id"] == target))
    {
        return Err("The binding target or input is invalid.".into());
    }
    Ok(json!({"id":binding["id"],"type":kind,"targetId":target}))
}
pub(super) fn run_binding(runtime: &Runtime, binding: &Value, cx: &mut App) -> Result<(), String> {
    if binding["type"] == "command" {
        return command(
            runtime,
            binding["targetId"]
                .as_str()
                .ok_or("The binding target or input is invalid.")?,
            cx,
        );
    }
    let action = binding_action(
        &runtime.content.read(cx).workspace,
        &runtime.ui.read(cx).selected_library_item_id,
        binding,
    )?;
    super::run(runtime, vec![action], 0, cx);
    Ok(())
}
pub fn current(runtime: &Runtime, event: &Value, cx: &App) -> bool {
    runtime.content.read(cx).workspace_path == event["workspacePath"].as_str().unwrap_or("")
        && cx.try_global::<Inputs>().is_none_or(|state| {
            state
                .communication_status
                .iter()
                .find(|s| s["deviceId"] == event["deviceId"])
                .is_none_or(|s| s["listening"] == true && s["sessionId"] == event["sessionId"])
        })
}
pub fn dispatch_note(
    workspace: &Value,
    note: &Value,
    mut run: impl FnMut(&Value) -> Result<(), String>,
) -> Result<(), String> {
    if let Some(binding) = workspace["triggerBindings"].as_array().and_then(|list| {
        list.iter().find(|b| {
            b["midi"].is_object()
                && b["midi"]["channel"] == note["channel"]
                && b["midi"]["pitch"] == note["pitch"]
        })
    }) {
        run(binding)?;
    }
    Ok(())
}
pub fn event(runtime: &Runtime, name: &str, payload: &Value, cx: &mut App) {
    let path = runtime.content.read(cx).workspace_path.clone();
    if name == "midi-input-status" {
        cx.global_mut::<Inputs>().midi_status = payload.clone();
        runtime.output.update(cx, |_, cx| cx.notify());
        return;
    }
    if name == "communication-input-status" && payload["workspacePath"] == path {
        let state = cx.global_mut::<Inputs>();
        if let Some(previous) = state
            .communication_status
            .iter_mut()
            .find(|v| v["deviceId"] == payload["deviceId"])
        {
            *previous = payload.clone();
        } else {
            state.communication_status.push(payload.clone());
        }
        runtime.output.update(cx, |_, cx| cx.notify());
        return;
    }
    let result = match name {
        "midi-note" => {
            let workspace = runtime.content.read(cx).workspace.clone();
            dispatch_note(&workspace, payload, |binding| {
                run_binding(runtime, binding, cx)
            })
        }
        "trigger-binding-test" if payload["workspacePath"] == path => {
            let binding = runtime.content.read(cx).workspace["triggerBindings"]
                .as_array()
                .and_then(|list| list.iter().find(|b| b["id"] == payload["bindingId"]))
                .cloned();
            binding
                .ok_or_else(|| "The binding target or input is invalid.".to_string())
                .and_then(|b| run_binding(runtime, &b, cx))
        }
        "trigger-bindings-changed" | "communication-devices-changed"
            if payload["workspacePath"] == path =>
        {
            let (key, field) = if name == "trigger-bindings-changed" {
                ("triggerBindings", "bindings")
            } else {
                ("communicationDevices", "devices")
            };
            if !payload[field].is_array() {
                Err("Invalid device configuration event.".into())
            } else {
                runtime.content.update(cx, |content, cx| {
                    std::sync::Arc::make_mut(&mut content.workspace)[key] = payload[field].clone();
                    cx.notify();
                });
                Ok(())
            }
        }
        "communication-input" if payload["workspacePath"] == path => {
            if cx.global::<Inputs>().queued >= 128 {
                crate::cues::actions::error(
                    runtime,
                    cx,
                    "Communication input queue is full. Commands were not executed.".into(),
                );
                return;
            }
            cx.global_mut::<Inputs>().queued += 1;
            let (host, runtime, payload) = (runtime.host.clone(), runtime.clone(), payload.clone());
            let generation = cx.global::<Inputs>().generation;
            cx.spawn(async move |cx| {
                let event = payload.clone();
                let accepted = cx
                    .background_spawn(async move {
                        host.invoke("acceptCommunicationInput", vec![event])
                    })
                    .await;
                cx.update(|cx| {
                    cx.global_mut::<Inputs>().queued -= 1;
                    if runtime.content.read(cx).workspace_path != path
                        || cx.global::<Inputs>().generation != generation
                    {
                        return;
                    }
                    let result = accepted.and_then(|accepted| {
                        if accepted == true {
                            let state = cx.global::<Inputs>();
                            if state
                                .communication_status
                                .iter()
                                .find(|s| s["deviceId"] == payload["deviceId"])
                                .is_some_and(|s| {
                                    s["listening"] != true || s["sessionId"] != payload["sessionId"]
                                })
                            {
                                return Ok(());
                            }
                            if payload["command"] == "artnetFixture" {
                                return super::artnet::dispatch(&runtime, &payload, cx);
                            }
                            command(
                                &runtime,
                                payload["command"]
                                    .as_str()
                                    .ok_or("Communication command is missing.")?,
                                cx,
                            )
                        } else {
                            Ok(())
                        }
                    });
                    if let Err(error) = result {
                        crate::cues::actions::error(&runtime, cx, error);
                    }
                });
            })
            .detach();
            Ok(())
        }
        _ => Ok(()),
    };
    if let Err(error) = result {
        crate::cues::actions::error(runtime, cx, error);
    }
}
