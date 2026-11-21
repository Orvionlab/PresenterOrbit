use crate::{
    communication_input::{DecoderFactory, ExchangeFactory, Inputs},
    AppState,
};
use serde_json::{json, Value};
use std::sync::{Arc, MutexGuard};

pub(crate) fn lock<'a>(state: &'a AppState) -> Result<MutexGuard<'a, Inputs>, String> {
    state
        .communication_inputs
        .lock()
        .map_err(|_| "Communication inputs are unavailable.".into())
}
use crate::Host;

pub(crate) fn connect(
    app: &Host,
    state: &AppState,
    path: Value,
    id: Value,
) -> Result<Value, String> {
    let mut inputs = lock(state)?;
    let path = crate::active_workspace(state, path)?;
    let io = crate::lock_workspace_io(state)?;
    io.require_active(&path)?;
    let workspace = crate::workspace::read_workspace(&path)?;
    let device = workspace["communicationDevices"]
        .as_array()
        .and_then(|devices| devices.iter().find(|device| device["id"] == id))
        .ok_or("Communication input device is unavailable.")?
        .clone();
    crate::communication_input_config::validate(&device)?;
    let protocol = device["input"]["protocol"]
        .as_str()
        .ok_or("Select an input protocol before connecting.")?
        .to_string();
    let path = path
        .to_str()
        .ok_or("Workspace path must be Unicode.")?
        .to_string();
    drop(io);
    let (decoder, exchange) = factories(app, &path, &device, &protocol)?;
    let events = app.clone();
    inputs.start(
        &path,
        device,
        decoder,
        exchange,
        Arc::new(move |status| {
            let _ = events.emit("communication-input-status", status);
        }),
    )
}

pub(crate) fn disconnect(state: &AppState, path: Value, id: Value) -> Result<Value, String> {
    let mut inputs = lock(state)?;
    crate::active_workspace(state, path)?;
    inputs.stop(id.as_str().ok_or("A device identity is required.")?);
    Ok(Value::Null)
}

pub(crate) fn accept(state: &AppState, event: &Value) -> Result<Value, String> {
    let inputs = lock(state)?;
    let path = match crate::active_workspace(state, event["workspacePath"].clone()) {
        Ok(path) => path,
        Err(_) => return Ok(json!(false)),
    };
    let io = crate::lock_workspace_io(state)?;
    io.require_active(&path)?;
    let workspace = crate::workspace::read_workspace(&path)?;
    let accepted = workspace["communicationDevices"]
        .as_array()
        .and_then(|devices| {
            devices
                .iter()
                .find(|device| device["id"] == event["deviceId"])
        })
        .is_some_and(|device| {
            matches!(
                device["input"]["protocol"].as_str(),
                Some("rosstalk" | "artnet")
            ) && inputs.accepts(
                event["workspacePath"].as_str().unwrap_or(""),
                device,
                event["sessionId"].as_str().unwrap_or(""),
            )
        });
    Ok(json!(accepted))
}

fn factories(
    app: &Host,
    path: &str,
    device: &Value,
    protocol: &str,
) -> Result<(DecoderFactory, ExchangeFactory), String> {
    if protocol == "artnet" {
        return crate::communication_artnet::factories(app, path, device);
    }
    if protocol == "rosstalk" {
        let app = app.clone();
        let path = path.to_string();
        let id = device["id"].clone();
        return Ok((
            Arc::new(crate::communication_ross_input::decoder),
            Arc::new(move || {
                let app = app.clone();
                let path = path.clone();
                let id = id.clone();
                Box::new(move |session, frame| {
                    let mut command = crate::communication_ross_input::parse(frame)?;
                    command["workspacePath"] = json!(path);
                    command["deviceId"] = id.clone();
                    command["sessionId"] = json!(session);
                    app.emit_to("main", "communication-input", command)
                        .map_err(|error| error.to_string())?;
                    Ok(Vec::new().into())
                })
            }),
        ));
    }
    crate::communication_deck_runtime::factories(app, path, device, protocol)
}
