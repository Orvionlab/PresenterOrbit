//! Automation in the UI process: the Controller host runs the scheduler, this module owns what React's
//! `automation-runtime.ts` does with it. It starts the scheduler session, schedules the markers of media that goes
//! on air, brings the marker positions of the output process to the scheduler, runs startup actions, and runs
//! what falls due. Timed automation does not run while the UI is down; see the decision log.

pub mod action_list;
pub mod announcements;
pub mod artnet;
mod build_clock;
pub mod confirm;
pub mod countdown;
pub mod due;
pub mod editors;
pub mod execute;
pub mod form;
pub mod inputs;
pub mod link;
pub mod macros;
pub mod markers;
pub mod panels;
pub mod props;
pub mod remote;
pub mod runner;
pub mod save;
pub mod slide_authoring;
pub mod stage;
pub mod timecode;
pub mod timeline;
pub mod transport;
pub mod validate;

use std::time::{Duration, Instant};

use gpui::{App, AppContext, Global};
use serde_json::{json, Value};

use crate::state::runtime::Runtime;

#[derive(Default)]
struct Automation {
    session: String,
    sequence: u64,
    /// The marker key of the media layer and of the music layer.
    keys: [String; 2],
    /// Whether the layer has markers armed, so the position poll has something to serve.
    armed: [bool; 2],
    startup_workspace: String,
    startup_load: u64,
    build_clock: build_clock::Clock,
    adoption: due::Adoption,
    positions_running: bool,
    /// How a job asks the operator a question; set by `start`.
    confirmer: Option<confirm::Confirmer>,
}

impl Global for Automation {}

fn context(runtime: &Runtime, state: &Automation) -> execute::Context {
    execute::Context {
        host: runtime.host.clone(),
        revisions: runtime.lanes.revisions().clone(),
        session: state.session.clone(),
        confirm: state.confirmer.clone(),
    }
}

pub(crate) fn facts(runtime: &Runtime, cx: &App) -> runner::Facts {
    let content = runtime.content.read(cx);
    runner::Facts {
        workspace: content.workspace.clone(),
        items: content.items.clone(),
        libraries: content.libraries.clone(),
        workspace_path: content.workspace_path.clone(),
        new_id: || uuid::Uuid::new_v4().to_string(),
    }
}

/// Starts the scheduler session and watches the live state for media and workspace changes. Questions a job
/// asks the operator are shown by `confirm`.
pub fn start(runtime: &Runtime, confirm: gpui::Entity<confirm::Confirm>, cx: &mut App) {
    use futures::StreamExt;
    inputs::start(runtime, cx);
    let session = uuid::Uuid::new_v4().to_string();
    let (confirmer, mut questions) = confirm::channel_pair();
    cx.set_global(Automation {
        session: session.clone(),
        sequence: 1,
        confirmer: Some(confirmer),
        ..Default::default()
    });
    cx.spawn(async move |cx| {
        while let Some(request) = questions.next().await {
            confirm.update(cx, |confirm, cx| confirm.push(request, cx));
        }
    })
    .detach();
    let host = runtime.host.clone();
    let first = json!({"sessionId": session, "sequence": 1});
    cx.background_spawn(async move {
        if let Err(error) = host.invoke("updateAutomationSchedule", vec![first]) {
            eprintln!("[automation] The scheduler session did not start: {error}");
        }
    })
    .detach();
    let watched = runtime.clone();
    cx.observe(&runtime.output, move |_, cx| synchronize(&watched, cx))
        .detach();
    let watched = runtime.clone();
    cx.observe(&runtime.content, move |_, cx| synchronize(&watched, cx))
        .detach();
}

/// `synchronizeAutomationRuntime`: marker scheduling for new media and the startup actions of a workspace.
fn synchronize(runtime: &Runtime, cx: &mut App) {
    if !cx.has_global::<Automation>() {
        return;
    }
    let live = runtime.output.read(cx).live.clone();
    if let Some(patch) = cx
        .global_mut::<Automation>()
        .build_clock
        .synchronize(&live, announcements::now_ms())
    {
        crate::cues::actions::edit_native(runtime, cx, move |live| {
            live.as_object_mut()
                .ok_or("Live state must be an object.")?
                .extend(
                    patch
                        .as_object()
                        .ok_or("Build clock patch must be an object.")?
                        .clone(),
                );
            Ok(())
        });
    }
    let path = runtime.content.read(cx).workspace_path.clone();
    for (position, layer) in ["media", "music"].into_iter().enumerate() {
        let key = markers::layer_key(&live, &path, layer);
        let state = cx.global_mut::<Automation>();
        if state.keys[position] == key {
            continue;
        }
        state.keys[position] = key.clone();
        state.armed[position] = false;
        state.sequence += 1;
        let snapshot = json!({
            "sessionId": state.session,
            "sequence": state.sequence,
            "cancelPendingMarkers": true,
            "cancelPendingMarkerLayer": layer,
        });
        let host = runtime.host.clone();
        let marker_context = context(runtime, state);
        let media = runtime.content.read(cx).workspace["media"].clone();
        let asset_markers = markers::layer_fields(layer);
        let asset = media.as_array().and_then(|assets| {
            assets
                .iter()
                .find(|asset| {
                    asset["id"] == live[asset_markers.0] && asset["type"] == asset_markers.2
                })
                .cloned()
        });
        let (live, path, errors) = (live.clone(), path.clone(), runtime.clone());
        let key_now = key.clone();
        cx.spawn(async move |cx| {
            let outcome = cx
                .background_spawn(async move {
                    host.invoke("updateAutomationSchedule", vec![snapshot])?;
                    match asset.as_ref().map(|asset| &asset["markers"]) {
                        Some(authored) if !key.is_empty() => execute::schedule_markers(
                            &marker_context,
                            &live,
                            &media,
                            &path,
                            layer,
                            authored,
                        ),
                        _ => Ok(0),
                    }
                })
                .await;
            cx.update(|cx| match outcome {
                Ok(armed) => arm(&errors, position, key_now, armed, cx),
                Err(error) => crate::cues::actions::error(&errors, cx, error),
            });
        })
        .detach();
    }
    startup_actions(runtime, &path, cx);
}

/// Records that `armed` markers wait on the layer, if the layer still shows the selection they were scheduled
/// for, and starts the position poll. Without armed markers nothing polls.
fn arm(runtime: &Runtime, position: usize, key: String, armed: usize, cx: &mut App) {
    let state = cx.global_mut::<Automation>();
    if armed == 0 || state.keys[position] != key {
        return;
    }
    state.armed[position] = true;
    follow_positions(runtime, cx);
}

/// True while a layer that is on air has markers armed.
fn polling_needed(state: &Automation) -> bool {
    (0..2).any(|layer| state.armed[layer] && !state.keys[layer].is_empty())
}

/// The output process measures where the media is; the scheduler of this process needs that position. Without
/// an output process the position is already local.
fn follow_positions(runtime: &Runtime, cx: &mut App) {
    if runtime
        .host
        .try_state::<crate::output_process::OutputRoute>()
        .is_none()
        || std::mem::replace(&mut cx.global_mut::<Automation>().positions_running, true)
    {
        return;
    }
    let runtime = runtime.clone();
    cx.spawn(async move |cx| loop {
        let host = runtime.host.clone();
        let outcome = cx
            .background_spawn(async move {
                let positions =
                    crate::state::runtime::output_call(&host, "getMarkerPositions", vec![])?;
                host.invoke("acceptMarkerPositions", vec![positions])
            })
            .await;
        let keep = cx.update(|cx| {
            if let Err(error) = outcome {
                crate::cues::actions::error(&runtime, cx, error);
            }
            let state = cx.global_mut::<Automation>();
            let keep = polling_needed(state);
            state.positions_running = keep;
            keep
        });
        if !keep {
            break;
        }
        cx.background_executor()
            .timer(Duration::from_millis(40))
            .await;
    })
    .detach();
}

/// `runStartupActions`: once per workspace load, the actions of every macro marked to run at startup.
fn startup_actions(runtime: &Runtime, path: &str, cx: &mut App) {
    let load = runtime.content.read(cx).load_generation;
    let state = cx.global_mut::<Automation>();
    if path.is_empty() || (state.startup_workspace == path && state.startup_load == load) {
        return;
    }
    state.startup_workspace = path.to_string();
    state.startup_load = load;
    let actions: Vec<Value> = runtime.content.read(cx).workspace["macros"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["triggerOnStartup"] == true)
        .flat_map(|entry| entry["actions"].as_array().cloned().unwrap_or_default())
        .collect();
    if !actions.is_empty() {
        run(runtime, actions, 0, cx);
    }
}

/// Runs actions once, in order, on the workspace lane. A failure reaches the operator through the status line.
pub fn run_operator(runtime: &Runtime, actions: Vec<Value>, cx: &mut App) {
    timeline::record_operator(runtime, &actions, cx);
    timeline::suppress_operator(true, cx);
    run_reporting(runtime, actions, 0, cx, |_, cx| {
        timeline::suppress_operator(false, cx)
    });
}

pub fn run(runtime: &Runtime, actions: Vec<Value>, depth: u32, cx: &mut App) {
    run_reporting(runtime, actions, depth, cx, |_, _| {});
}

pub fn run_reporting(
    runtime: &Runtime,
    actions: Vec<Value>,
    depth: u32,
    cx: &mut App,
    finished: impl FnOnce(Result<(), String>, &mut App) + 'static,
) {
    if !cx.has_global::<Automation>() {
        finished(Err("Automation has not started.".into()), cx);
        return;
    }
    let facts = facts(runtime, cx);
    let path = facts.workspace_path.clone();
    let context = context(runtime, cx.global::<Automation>());
    let lanes = runtime.lanes.clone();
    let answer = cx.background_spawn(async move {
            let report = execute::execute_queued(&context, facts, &actions, depth, Some(&lanes))?;
            Ok(json!({
                "failures": report.failures.iter().map(|failure| failure.message.clone()).collect::<Vec<_>>(),
                "armed": report.armed,
                "external": report.external,
                "externalError": report.external_error,
                "audioInputs": report.audio_inputs.iter().map(|(id, over)| json!({"id": id, "overrides": over})).collect::<Vec<_>>(),
            }))
        });
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        let result: Result<Value, String> = answer.await;
        cx.update(|cx| {
            if runtime.content.read(cx).workspace_path != path {
                finished(Err("The workspace changed during the action.".into()), cx);
                return;
            }
            let completion = match &result {
                Err(error) => Err(error.clone()),
                Ok(report) => report["failures"]
                    .as_array()
                    .and_then(|list| list.first())
                    .and_then(Value::as_str)
                    .map_or(Ok(()), |error| Err(error.to_string())),
            };
            match result {
                Ok(report) => {
                    crate::external::publish(
                        &report["external"],
                        report["externalError"].as_str(),
                        cx,
                    );
                    for entry in report["audioInputs"].as_array().into_iter().flatten() {
                        apply_audio_override(entry, cx);
                    }
                    if let Some(message) =
                        report["failures"].as_array().and_then(|list| list.first())
                    {
                        crate::cues::actions::error(
                            &runtime,
                            cx,
                            message.as_str().unwrap_or_default().into(),
                        );
                    }
                }
                Err(error) => crate::cues::actions::error(&runtime, cx, error),
            }
            finished(completion, cx);
        });
    })
    .detach();
}

fn apply_audio_override(entry: &Value, cx: &mut App) {
    let Some(id) = entry["id"].as_str() else {
        return;
    };
    let state = cx.default_global::<crate::settings::audio_routing::AudioInputState>();
    let current = state
        .overrides
        .entry(id.to_string())
        .or_insert_with(|| json!({}));
    if let (Some(current), Some(next)) = (current.as_object_mut(), entry["overrides"].as_object()) {
        current.extend(next.clone());
    }
}

/// An `automation-due` event of the scheduler. The output process never sees it, so an action runs only here.
pub fn on_due(runtime: &Runtime, event: &Value, cx: &mut App) {
    if !cx.has_global::<Automation>() {
        return;
    }
    let live = runtime.output.read(cx).live.clone();
    let state = cx.global::<Automation>();
    let due = match due::classify(
        event,
        &state.session,
        &live,
        &state.adoption,
        Instant::now(),
    ) {
        Ok(due) => due,
        Err(error) => return crate::cues::actions::error(runtime, cx, error),
    };
    match due {
        due::Due::Ignore => {}
        due::Due::StatePatch { revision, patch } => runtime.output.update(cx, |output, cx| {
            if let Err(error) =
                output.accept_native_patch(&json!({"revision": revision, "patch": patch}))
            {
                output.error = Some(error);
            }
            cx.notify();
        }),
        due::Due::Actions { actions, depth } => run(runtime, actions, depth, cx),
        due::Due::Build {
            step,
            started_at_ms,
        } => crate::cues::actions::edit_native(runtime, cx, move |live| {
            let current = live["liveBuildStep"].as_u64().unwrap_or(0);
            if step <= current {
                return Ok(());
            }
            live["liveBuildStep"] = json!(step);
            if !live["liveBuildStepStartedAtMsByStep"].is_object() {
                live["liveBuildStepStartedAtMsByStep"] = json!({});
            }
            live["liveBuildStepStartedAtMsByStep"][step.to_string()] = json!(started_at_ms);
            Ok(())
        }),
        due::Due::Cue { slide_id, instance } => {
            let library = runtime
                .content
                .read(cx)
                .library_id_for_item(live["liveItemId"].as_str().unwrap_or(""))
                .to_string();
            let mut payload = json!({"presentationId": live["liveItemId"], "slideId": slide_id, "cueInstanceId": instance});
            let command = if live["livePlaylistId"].is_string() {
                payload["playlistId"] = live["livePlaylistId"].clone();
                payload["playlistEntryId"] = live["livePlaylistEntryId"].clone();
                "playlistCue"
            } else {
                payload["libraryId"] = json!(library);
                "libraryCue"
            };
            crate::cues::actions::run_native(runtime, cx, command, payload, false, false);
        }
    }
}

/// A key press that is the hotkey of a macro runs that macro once. Shortcuts of commands are taken before this.
pub fn run_hotkey(runtime: &Runtime, keystroke: &gpui::Keystroke, cx: &mut App) -> bool {
    let Some(pressed) = crate::keys::shortcut_from_keystroke(keystroke) else {
        return false;
    };
    let pressed = validate::canonical_shortcut(&pressed);
    let workspace = runtime.content.read(cx).workspace.clone();
    if let Some(binding) = workspace["triggerBindings"]
        .as_array()
        .and_then(|bindings| {
            bindings.iter().find(|b| {
                b["shortcut"]
                    .as_str()
                    .is_some_and(|s| validate::canonical_shortcut(s) == pressed)
            })
        })
    {
        if let Err(error) = inputs::run_binding(runtime, binding, cx) {
            crate::cues::actions::error(runtime, cx, error);
        }
        return true;
    }
    let Some(id) = workspace["macros"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| {
            entry["hotkey"]
                .as_str()
                .is_some_and(|hotkey| validate::canonical_shortcut(hotkey) == pressed)
        })
        .and_then(|entry| entry["id"].as_str())
    else {
        return false;
    };
    let action = json!({"id": format!("hotkey-{id}"), "type": "macro", "targetId": id});
    run(runtime, vec![action], 0, cx);
    true
}

/// The output process answered with a slide that is on air although this process put nothing on air: the
/// interface restarted. Until the window passes, the cue and timeline events of that slide are ignored.
pub fn adopted(runtime: &Runtime, cx: &mut App) {
    if !cx.has_global::<Automation>() {
        return;
    }
    let key = due::live_key(&runtime.output.read(cx).live);
    cx.global_mut::<Automation>()
        .adoption
        .adopt(&key, Instant::now());
}
