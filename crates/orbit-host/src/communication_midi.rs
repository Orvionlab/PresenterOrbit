use midir::{MidiOutput, MidiOutputConnection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Mutex, OnceLock};

pub(crate) type Pending = Receiver<Result<Value, String>>;
type Request = (Option<(Value, Value)>, mpsc::Sender<Result<Value, String>>);
static REQUESTS: OnceLock<Result<SyncSender<Request>, String>> = OnceLock::new();

fn queue() -> Result<&'static SyncSender<Request>, String> {
    REQUESTS
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<Request>(256);
            std::thread::Builder::new()
                .name("presenter-orbit-midi-output".into())
                .spawn(move || {
                    while let Ok((command, reply)) = receiver.recv() {
                        let result = match command {
                            Some((device, payload)) => send_now(&device, &payload),
                            None => release_now(),
                        };
                        let _ = reply.send(result);
                    }
                })
                .map_err(|error| format!("MIDI output worker could not start: {error}"))?;
            Ok(sender)
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub(crate) fn submit(command: Option<(Value, Value)>) -> Result<Pending, String> {
    if let Some((device, payload)) = &command {
        validate(device)?;
        note_bytes(payload)?;
    }
    let (sender, receiver) = mpsc::channel();
    queue()?
        .try_send((command, sender))
        .map_err(|_| "MIDI output queue is full or unavailable; the command was not submitted.")?;
    Ok(receiver)
}

pub(crate) fn wait(pending: Pending) -> Result<Value, String> {
    pending.recv().map_err(|_| {
        "MIDI output stopped before confirming submission; the command was not retried."
    })?
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub(crate) struct Port {
    pub id: String,
    pub name: String,
}

pub(crate) fn validate(device: &Value) -> Result<Port, String> {
    if device.get("input").is_some() {
        return Err("MIDI output cannot be a receiving communication device.".into());
    }
    let port: Port = serde_json::from_value(device["midi"].clone())
        .map_err(|_| "Select an explicit MIDI output destination.")?;
    if port.name.trim().is_empty() || port.name.trim() != port.name || port.id.trim() != port.id {
        return Err("The MIDI output destination identity is invalid.".into());
    }
    Ok(port)
}

pub(crate) fn note_bytes(payload: &Value) -> Result<[u8; 3], String> {
    let values = payload
        .as_array()
        .filter(|values| values.len() == 3)
        .ok_or("MIDI output requires one complete Note On or Note Off message.")?;
    let mut bytes = [0; 3];
    for (index, value) in values.iter().enumerate() {
        bytes[index] = value
            .as_u64()
            .and_then(|value| u8::try_from(value).ok())
            .ok_or("MIDI bytes must be integers.")?;
    }
    if !matches!(bytes[0] & 0xf0, 0x80 | 0x90) || bytes[1] > 127 || bytes[2] > 127 {
        return Err(
            "MIDI output supports Note On/Off, channels 1–16 and note/velocity 0–127.".into(),
        );
    }
    Ok(bytes)
}

fn output() -> Result<MidiOutput, String> {
    MidiOutput::new("Presenter Orbit")
        .map_err(|error| format!("MIDI output is unavailable: {error}"))
}

pub(crate) fn ports() -> Result<Value, String> {
    let output = output()?;
    let ports = output
        .ports()
        .iter()
        .map(|port| {
            output
                .port_name(port)
                .map(|name| Port {
                    id: port.id(),
                    name,
                })
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!(ports))
}

struct Connection<C> {
    connection: C,
    notes: HashSet<(u8, u8)>,
}
struct Registry<C> {
    connections: HashMap<Port, Connection<C>>,
}
impl<C> Default for Registry<C> {
    fn default() -> Self {
        Self {
            connections: HashMap::new(),
        }
    }
}
impl<C> Registry<C> {
    fn send(
        &mut self,
        port: &Port,
        bytes: [u8; 3],
        open: impl FnOnce() -> Result<C, String>,
        mut send: impl FnMut(&mut C, &[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        if !self.connections.contains_key(port) {
            self.connections.insert(
                port.clone(),
                Connection {
                    connection: open()?,
                    notes: HashSet::new(),
                },
            );
        }
        let entry = self.connections.get_mut(port).unwrap();
        if let Err(error) = send(&mut entry.connection, &bytes) {
            self.connections.remove(port);
            return Err(format!(
                "MIDI send failed; the message was not retried: {error}"
            ));
        }
        let note = (bytes[0] & 15, bytes[1]);
        if bytes[0] & 0xf0 == 0x90 && bytes[2] > 0 {
            entry.notes.insert(note);
        } else {
            entry.notes.remove(&note);
        }
        Ok(())
    }
    fn release(
        &mut self,
        mut send: impl FnMut(&mut C, &[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut errors = Vec::new();
        for (port, mut entry) in self.connections.drain() {
            for (channel, note) in entry.notes {
                if let Err(error) = send(&mut entry.connection, &[0x80 | channel, note, 0]) {
                    errors.push(format!("{}: {error}", port.name));
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Some Orbit MIDI notes could not be released: {}",
                errors.join("; ")
            ))
        }
    }
}
static OUTPUTS: OnceLock<Mutex<Registry<MidiOutputConnection>>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry<MidiOutputConnection>> {
    OUTPUTS.get_or_init(|| Mutex::new(Registry::default()))
}

pub(crate) fn send(device: &Value, payload: &Value) -> Result<Value, String> {
    wait(submit(Some((device.clone(), payload.clone())))?)
}

fn send_now(device: &Value, payload: &Value) -> Result<Value, String> {
    let target = validate(device)?;
    let bytes = note_bytes(payload)?;
    let mut registry = registry()
        .lock()
        .map_err(|_| "MIDI output state is unavailable.")?;
    let scanner = output()?;
    let ports = scanner.ports();
    let matches = ports
        .iter()
        .filter(|port| {
            port.id() == target.id && scanner.port_name(port).as_deref() == Ok(target.name.as_str())
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(
            "The selected MIDI destination is missing or ambiguous. Refresh and select it again."
                .into(),
        );
    }
    registry.send(
        &target,
        bytes,
        || {
            scanner
                .connect(matches[0], "presenter-orbit-output")
                .map_err(|error| error.to_string())
        },
        |connection, bytes| connection.send(bytes).map_err(|error| error.to_string()),
    )?;
    Ok(json!({"ok":true,"bytesSent":3}))
}

pub(crate) fn release() -> Result<Value, String> {
    wait(submit(None)?)
}

fn release_now() -> Result<Value, String> {
    registry()
        .lock()
        .map_err(|_| "MIDI output state is unavailable.")?
        .release(|connection, bytes| connection.send(bytes).map_err(|error| error.to_string()))?;
    Ok(json!({"ok":true}))
}
