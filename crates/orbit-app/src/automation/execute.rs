//! Performs what the runner decided, in order, exactly once. Runs on the workspace command lane, so it follows
//! earlier workspace edits and never blocks the UI thread. Only the UI process runs this: the output process
//! never receives `automation-due`, so no action runs in both processes.

use orbit_host::Host;
use serde_json::{json, Value};

use super::markers;
use super::runner::{self, Facts, Failure, Run, Step};
use crate::state::revisions::Revisions;

#[derive(Clone)]
pub struct Context {
    pub host: Host,
    pub revisions: Revisions,
    pub session: String,
    /// Asks the operator a question; `None` where no dialog exists.
    pub confirm: Option<super::confirm::Confirmer>,
}

/// What the UI itself must apply once the lane job is done.
#[derive(Default)]
pub struct Report {
    pub external: Option<Value>,
    pub external_error: Option<String>,
    pub failures: Vec<Failure>,
    pub audio_inputs: Vec<(String, Value)>,
    /// Layers that now have markers armed with the scheduler.
    pub armed: Vec<&'static str>,
}

fn serial<T: Send + 'static>(
    lanes: Option<&crate::state::lane::Lanes>,
    path: &str,
    job: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    match lanes {
        Some(lanes) => futures::executor::block_on(lanes.task(path, job))
            .map_err(|_| "The automation worker stopped.".to_string())?,
        None => job(),
    }
}

/// The orchestrator waits outside the workspace lane. Each batch and side effect rejoins the lane;
/// recording questions alone wait outside it, preserving the remaining action order.
pub fn execute_queued(
    context: &Context,
    facts: Facts,
    actions: &[Value],
    depth: u32,
    lanes: Option<&crate::state::lane::Lanes>,
) -> Result<Report, String> {
    let path = facts.workspace_path.clone();
    let mut report = Report::default();
    let mut pending = runner::queue(actions, depth);
    loop {
        let (host, batch_facts) = (context.host.clone(), facts.clone());
        let scope = path.clone();
        let (mut run, done, rest) = serial(lanes, &path, move || {
            let mut run = Run::default();
            let mut done = true;
            host.edit_operator_output(&scope, |live| {
                done = runner::run_batch(live, &batch_facts, &mut pending, &mut run);
                Ok(())
            })?;
            Ok((run, done, pending))
        })?;
        pending = rest;
        report.failures.append(&mut run.failures);
        for step in run.steps {
            // Capture validates the file identity again after the answer, at the output boundary.
            let question = matches!(&step, Step::Capture { action } if
                action["details"]["stop"]["showsAlertBeforeStopping"] == true);
            let (context, facts, scope) = (context.clone(), facts.clone(), path.clone());
            let (next, result) = serial(if question { None } else { lanes }, &path, move || {
                let result = perform(&context, &facts, &scope, step, &mut report);
                Ok((report, result))
            })?;
            report = next;
            if let Err(message) = result {
                report.failures.push(Failure {
                    action_id: String::new(),
                    kind: "step".into(),
                    message,
                });
            }
        }
        if done {
            return Ok(report);
        }
    }
}

fn perform(
    context: &Context,
    facts: &Facts,
    path: &str,
    step: Step,
    report: &mut Report,
) -> Result<(), String> {
    let host = &context.host;
    match step {
        Step::Cue {
            command,
            mut payload,
        } => {
            payload["workspacePath"] = json!(path);
            host.execute_cue_command(command, &payload).map(|_| ())
        }
        Step::Delay {
            actions,
            seconds,
            depth,
        } => schedule(context, actions, seconds, depth, None),
        Step::Markers { layer, markers } => {
            let mut live = Value::Null;
            host.edit_operator_output(path, |snapshot| {
                live = snapshot.clone();
                Ok(())
            })?;
            let armed = schedule_markers(
                context,
                &live,
                &facts.workspace["media"],
                path,
                layer,
                &markers,
            )?;
            if armed > 0 {
                report.armed.push(layer);
            }
            Ok(())
        }
        Step::Timer {
            timer,
            command,
            increment,
            configuration,
        } => timer_step(context, path, timer, &command, increment, configuration),
        Step::Communication {
            device,
            payload,
            details,
        } => {
            if device.is_null() {
                return Err("Communication device is unavailable.".into());
            }
            let encoding = details["payloadEncoding"].as_str().unwrap_or("");
            let bytes = crate::settings::devices::encode_payload(&payload, encoding)
                .map_err(str::to_string)?;
            let result = host.invoke("sendCommunication", vec![device, bytes])?;
            if result["ok"] == false {
                return Err("Communication send failed.".into());
            }
            Ok(())
        }
        Step::Capture { action } => capture(context, path, &action),
        Step::ExternalPresentation { url } => {
            match host.invoke("startExternalPresentation", vec![json!(url)]) {
                Ok(status) => {
                    report.external = Some(status);
                    report.external_error = None;
                    Ok(())
                }
                Err(error) => {
                    report.external_error = Some(error.clone());
                    Err(error)
                }
            }
        }
        Step::AudioInput { id, overrides } => {
            report.audio_inputs.push((id, overrides));
            Ok(())
        }
    }
}

/// Schedules the markers of the media on `layer` with the host scheduler.
pub fn schedule_markers(
    context: &Context,
    live: &Value,
    media: &Value,
    workspace: &str,
    layer: &str,
    markers: &Value,
) -> Result<usize, String> {
    let mut armed = 0;
    for marker in markers::schedule(live, media, workspace, layer, markers)? {
        schedule(
            context,
            marker.actions,
            marker.seconds,
            0,
            Some(marker.playback),
        )?;
        armed += 1;
    }
    Ok(armed)
}

fn schedule(
    context: &Context,
    actions: Vec<Value>,
    seconds: f64,
    depth: u32,
    marker: Option<Value>,
) -> Result<(), String> {
    let mut request = json!({
        "sessionId": context.session,
        "requestId": uuid::Uuid::new_v4().to_string(),
        "delaySeconds": seconds,
        "depth": depth,
        "actions": actions,
    });
    if let Some(marker) = marker {
        request["markerPlayback"] = marker;
    }
    let answer = context
        .host
        .invoke("scheduleAutomationActions", vec![request])?;
    if answer["ok"] != true || answer["stale"] == true {
        return Err("Native automation rejected the delayed action.".into());
    }
    Ok(())
}

/// The timer a `mutateCountdownTimers` answer carries: the host returns only the timers, under `workspace`.
fn configured_timer(result: &Value, id: &str) -> Option<Value> {
    result["workspace"]["countdownTimers"]
        .as_array()?
        .iter()
        .find(|entry| entry["id"] == id)
        .cloned()
}

fn timer_step(
    context: &Context,
    path: &str,
    mut timer: Value,
    command: &str,
    increment: f64,
    configuration: Option<Value>,
) -> Result<(), String> {
    let id = timer["id"].as_str().unwrap_or_default().to_string();
    if let Some(patch) = configuration {
        // ProPresenter's Set Configuration replaces the timer definition before the control runs.
        let revision = context.revisions.workspace(path);
        let result = context.host.invoke(
            "mutateCountdownTimers",
            vec![
                json!(path),
                json!({"type":"update","timerId":id,"patch":patch}),
                json!(revision),
            ],
        )?;
        context.revisions.remember(path, &result);
        timer = configured_timer(&result, &id)
            .ok_or("The configured timer is missing from the answer.")?;
    }
    let mut current = Value::Null;
    context.host.edit_operator_output(path, |live| {
        current = live["countdownTimerRuns"][&id].clone();
        Ok(())
    })?;
    let run = context.host.invoke(
        "controlTimer",
        vec![timer, json!(command), current, json!(increment)],
    )?;
    if run.is_null() {
        return Ok(());
    }
    context.host.edit_operator_output(path, |live| {
        if !live["countdownTimerRuns"].is_object() {
            live["countdownTimerRuns"] = json!({});
        }
        live["countdownTimerRuns"][&id] = run;
        Ok(())
    })?;
    Ok(())
}

/// `captureActionSettings` and `performCaptureAction`.
/// A stop with an operator alert: only a recording that runs is asked about, and only an accepted question stops
/// it. The recording file identifies the recording that was asked about, so a newer one is never stopped by it.
fn confirmed_stop(
    status: &Value,
    ask: impl FnOnce() -> Result<bool, String>,
) -> Result<Option<String>, String> {
    if status["active"] != true {
        return Ok(None);
    }
    let file = status["filePath"]
        .as_str()
        .filter(|file| !file.is_empty())
        .ok_or("The recording has no file to confirm.")?;
    Ok(ask()?.then(|| file.to_string()))
}

fn capture(context: &Context, path: &str, action: &Value) -> Result<(), String> {
    let (start, stop) = (&action["details"]["start"], &action["details"]["stop"]);
    if !start.is_null() && !stop.is_null() {
        return Err("Choose a valid recording action and preset.".into());
    }
    if start.is_object() {
        let preset = start["presetIdentification"]
            .as_str()
            .or_else(|| start["preset_identification"].as_str())
            .ok_or("Choose a valid recording action and preset.")?;
        let status = crate::output::capture::start(&context.host, path, preset)?;
        return status["error"]
            .as_str()
            .map_or(Ok(()), |error| Err(error.into()));
    }
    let confirm = match stop {
        Value::Bool(true) => false,
        Value::Object(fields) => fields
            .get("showsAlertBeforeStopping")
            .and_then(Value::as_bool)
            .ok_or("Choose a valid recording action and preset.")?,
        _ => return Err("Choose a valid recording action and preset.".into()),
    };
    if confirm {
        let status = crate::output::capture::status(&context.host)?;
        let asker = context
            .confirm
            .as_ref()
            .ok_or("There is no dialog to confirm stopping the recording.")?;
        let Some(file) = confirmed_stop(&status, || {
            asker.ask("capture.confirmStop", "capture.stop", super::confirm::WAIT)
        })?
        else {
            return Ok(());
        };
        let stopped = crate::output::capture::stop_if_current(&context.host, &file)?;
        return stopped["error"]
            .as_str()
            .map_or(Ok(()), |error| Err(error.into()));
    }
    let status = crate::output::capture::stop(&context.host)?;
    status["error"]
        .as_str()
        .map_or(Ok(()), |error| Err(error.into()))
}
