//! HTTP stays in the UI host. Deferred remote commands join the same action runner as local controls.
use crate::services::entries;
use crate::state::runtime::Runtime;
use gpui::{App, AppContext};
use serde_json::{json, Value};
fn report(runtime: &Runtime, request: &Value, result: Result<(), String>, cx: &mut App) {
    if !request["requestId"].is_string() {
        if let Err(error) = result {
            crate::cues::actions::error(runtime, cx, error);
        }
        return;
    }
    let mut answer = json!({"requestId":request["requestId"],"workspacePath":request["workspacePath"],"status":if result.is_ok(){if request["completion"]==true{"completed"}else{"dispatched"}}else{"error"}});
    if let Err(error) = result {
        answer["error"] = json!(error);
    }
    let (host, runtime) = (runtime.host.clone(), runtime.clone());
    cx.spawn(async move |cx| {
        let result = cx
            .background_spawn(async move { host.invoke("reportRemoteCommandResult", vec![answer]) })
            .await;
        if let Err(error) = result {
            cx.update(|cx| crate::cues::actions::error(&runtime, cx, error));
        }
    })
    .detach();
}
pub fn action(workspace: &Value, request: &Value) -> Result<Value, String> {
    let command = request["command"]
        .as_str()
        .ok_or("Unsupported remote command.")?;
    let id = request["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("A target id is required.")?;
    let (kind, field) = match command {
        "macro" => ("macro", "macros"),
        "prop" => ("prop", "props"),
        "message" => ("message", "messages"),
        "stageLayout" => ("stage", "stageLayouts"),
        "clearGroup" => ("clear-group", "clearGroups"),
        "audienceLook" => ("audience-look", "audienceLooks"),
        _ => return Err("Unsupported remote command.".into()),
    };
    if !entries(&workspace[field]).iter().any(|v| v["id"] == id) {
        return Err("Target is unavailable in the active workspace.".into());
    }
    let mut action = json!({"id":uuid::Uuid::new_v4().to_string(),"type":kind,"targetId":id});
    if command == "stageLayout" {
        let screen = request["screenId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("Stage screen is unavailable in the active workspace.")?;
        action["details"] = json!({"assignments":[{"screenId":screen,"layoutId":id}]});
    }
    if matches!(command, "prop" | "message") {
        let operation = request["action"]
            .as_str()
            .ok_or("Show or clear is required.")?;
        if !matches!(operation, "show" | "clear") {
            return Err("Show or clear is required.".into());
        }
        if command == "message" && operation == "clear" {
            action = json!({"id":action["id"],"type":"clear","targetLayer":"messages"});
        } else {
            action["value"] = json!(operation);
            if command == "prop" {
                action["details"] = request["values"].clone();
            } else {
                action["details"] = json!({"tokenValues":request["tokenValues"]});
            }
        }
    }
    Ok(action)
}
#[derive(Default)]
pub struct Suppressed(pub bool);
impl gpui::Global for Suppressed {}
pub fn event(runtime: &Runtime, request: &Value, cx: &mut App) {
    cx.default_global::<Suppressed>().0 = true;
    dispatch(runtime, request, cx);
    cx.default_global::<Suppressed>().0 = false;
}
fn dispatch(runtime: &Runtime, request: &Value, cx: &mut App) {
    let command = request["command"].as_str().unwrap_or("");
    if matches!(
        command,
        "nextSlide" | "previousSlide" | "nextItem" | "previousItem" | "clearAll"
    ) {
        let result = super::inputs::command(runtime, command, cx);
        report(runtime, request, result, cx);
        return;
    }
    if command == "clearLayer" {
        let result = request["layer"]
            .as_str()
            .filter(|s| {
                [
                    "slide",
                    "media",
                    "music",
                    "props",
                    "messages",
                    "announcements",
                    "videoInput",
                ]
                .contains(s)
            })
            .ok_or_else(|| "A valid layer is required.".to_string())
            .map(|layer| crate::cues::actions::clear(runtime, layer, cx));
        report(runtime, request, result, cx);
        return;
    }
    let path = runtime.content.read(cx).workspace_path.clone();
    if request["workspacePath"] != path || path.is_empty() || !request["requestId"].is_string() {
        report(
            runtime,
            request,
            Err("A remote request and current workspace identity are required.".into()),
            cx,
        );
        return;
    }
    if command == "macro" {
        macro_run(runtime, request, cx);
        return;
    }
    if matches!(command, "timerDefinition" | "messageDefinition") {
        definitions(runtime, request, cx);
        return;
    }
    if command == "stageLayout" {
        let screen = request["screenId"].as_str().unwrap_or("");
        let output = runtime.output.read(cx);
        if output
            .snapshot
            .as_ref()
            .is_none_or(|s| s["screenConfig"][screen]["type"] != "stage")
        {
            report(
                runtime,
                request,
                Err("Stage screen is unavailable in the active workspace.".into()),
                cx,
            );
            return;
        }
    }
    match action(&runtime.content.read(cx).workspace, request) {
        Ok(action) => {
            let (runtime, request) = (runtime.clone(), request.clone());
            super::run_reporting(&runtime.clone(), vec![action], 0, cx, move |result, cx| {
                report(&runtime, &request, result, cx)
            });
        }
        Err(error) => report(runtime, request, Err(error), cx),
    }
}
pub fn macro_plan(workspace: &Value, id: &str) -> Result<Vec<(f64, Value)>, String> {
    fn expand(
        workspace: &Value,
        actions: &[Value],
        base: f64,
        depth: u32,
        plan: &mut Vec<(f64, Value)>,
    ) -> Result<(), String> {
        if depth > super::runner::MAX_DEPTH {
            return Err("Automation nesting depth limit of 8 was exceeded.".into());
        }
        for action in actions.iter().filter(|a| a["enabled"] != false) {
            let delay = action["delaySeconds"].as_f64().unwrap_or(0.);
            if !delay.is_finite() || delay < 0. {
                return Err("automation.macros.delayInvalid".into());
            }
            if action["type"] == "macro" {
                let macros = entries(&workspace["macros"]);
                let target = macros
                    .iter()
                    .find(|m| m["id"] == action["targetId"])
                    .ok_or("Macro is unavailable in the active workspace.")?;
                expand(
                    workspace,
                    &entries(&target["actions"]),
                    base + delay,
                    depth + 1,
                    plan,
                )?;
            } else {
                let mut action = action.clone();
                action["delaySeconds"] = json!(0);
                plan.push((base + delay, action));
            }
        }
        Ok(())
    }
    let macros = entries(&workspace["macros"]);
    let target = macros
        .iter()
        .find(|m| m["id"] == id)
        .ok_or("Macro is unavailable in the active workspace.")?;
    let mut plan = Vec::new();
    expand(workspace, &entries(&target["actions"]), 0., 0, &mut plan)?;
    if plan.is_empty() {
        return Err("Macro has no enabled actions.".into());
    }
    Ok(plan)
}
fn macro_run(runtime: &Runtime, request: &Value, cx: &mut App) {
    let plan = match macro_plan(
        &runtime.content.read(cx).workspace,
        request["id"].as_str().unwrap_or(""),
    ) {
        Ok(p) => p,
        Err(e) => {
            report(runtime, request, Err(e), cx);
            return;
        }
    };
    report(runtime, request, Ok(()), cx);
    let (runtime, mut request) = (runtime.clone(), request.clone());
    let path = runtime.content.read(cx).workspace_path.clone();
    let load = runtime.content.read(cx).load_generation;
    cx.spawn(async move |cx| {
        let jobs = plan.into_iter().map(|(delay, action)| {
            let (runtime, cx, path) = (runtime.clone(), cx.clone(), path.clone());
            async move {
                if delay > 0. {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs_f64(delay))
                        .await;
                }
                let (sender, answer) = futures::channel::oneshot::channel();
                cx.update(|cx| {
                    if runtime.content.read(cx).workspace_path != path
                        || runtime.content.read(cx).load_generation != load
                    {
                        let _ = sender.send(Err(
                            "Workspace changed before the delayed action ran.".to_string(),
                        ));
                        return;
                    }
                    super::run_reporting(&runtime, vec![action], 0, cx, move |result, _| {
                        let _ = sender.send(result);
                    });
                });
                answer
                    .await
                    .map_err(|_| "The automation completion worker stopped.".to_string())?
            }
        });
        let results = futures::future::join_all(jobs).await;
        let errors = results
            .into_iter()
            .filter_map(Result::err)
            .collect::<Vec<_>>();
        request["completion"] = json!(true);
        let result = if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        };
        cx.update(|cx| report(&runtime, &request, result, cx));
    })
    .detach();
}
fn definitions(runtime: &Runtime, request: &Value, cx: &mut App) {
    let (runtime, request) = (runtime.clone(), request.clone());
    let path = runtime.content.read(cx).workspace_path.clone();
    let original = runtime.content.read(cx).workspace.clone();
    cx.spawn(async move |cx| {
        let result = if request["command"] == "timerDefinition" {
            super::save::mutation(
                &runtime,
                cx,
                "mutateCountdownTimers",
                json!({"type":"update","timerId":request["id"],"patch":request["patch"]}),
            )
            .await
            .map(|_| ())
        } else {
            let mut messages = entries(&original["messages"]);
            match messages.iter_mut().find(|m| m["id"] == request["id"]) {
                Some(message) => {
                    if let Some(text) = request["text"].as_str() {
                        message["text"] = json!(text);
                    }
                    if let Some(values) = request.get("tokenValues") {
                        if !values
                            .as_object()
                            .is_some_and(|values| values.values().all(Value::is_string))
                        {
                            return cx.update(|cx| {
                                report(
                                    &runtime,
                                    &request,
                                    Err("Message token values must be a string map.".into()),
                                    cx,
                                )
                            });
                        }
                        let mut tokens = message["tokenValues"]
                            .as_object()
                            .cloned()
                            .unwrap_or_default();
                        tokens.extend(values.as_object().cloned().unwrap_or_default());
                        message["tokenValues"] = Value::Object(tokens);
                    }
                    let patch = json!({"messages":messages});
                    super::save::content(&runtime, cx, patch).await.map(|_| ())
                }
                None => Err("Message is unavailable in the active workspace.".into()),
            }
        };
        cx.update(|cx| {
            let result = if runtime.content.read(cx).workspace_path != path {
                Err("The workspace changed during the remote command.".into())
            } else {
                result
            };
            report(&runtime, &request, result, cx);
        });
    })
    .detach();
}
