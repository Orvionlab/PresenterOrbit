//! `DevicesSettingsPanelBridge.tsx`, `communication-device-controller.ts`, `SerialDeviceFields.tsx`, and
//! `MidiDeviceFields.tsx` for output devices (TCP, UDP, serial, MIDI). A device list is one `communicationDevices`
//! workspace section. Input devices (`CommunicationInputFields`, Art-Net, connect and disconnect) are not built.

use super::ui::Field;
use gpui::{div, prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant, Select, SelectOption};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::ui::{action_row, muted, stack, title};
use super::view::SettingsView;
use crate::collections::model::Snapshot;

const TRANSPORTS: [&str; 4] = ["tcp", "udp", "serial", "midi"];

/// `serialValidationError`.
fn serial_error(serial: &Value) -> Option<&'static str> {
    let path = serial["path"].as_str().unwrap_or("");
    let int_in =
        |v: &Value, min: u64, max: u64| v.as_u64().is_some_and(|n| (min..=max).contains(&n));
    let ok = serial.is_object()
        && !path.trim().is_empty()
        && path.trim() == path
        && int_in(&serial["baudRate"], 1, 4_000_000)
        && [5, 6, 7, 8].iter().any(|d| serial["dataBits"] == *d)
        && [1, 2].iter().any(|d| serial["stopBits"] == *d)
        && ["none", "odd", "even"]
            .iter()
            .any(|p| serial["parity"] == *p)
        && ["none", "software", "hardware"]
            .iter()
            .any(|f| serial["flowControl"] == *f);
    (!ok).then_some("settings.devices.serialInvalid")
}

fn midi_error(device: &Value) -> Option<&'static str> {
    let midi = &device["midi"];
    let id = midi["id"].as_str();
    let name = midi["name"].as_str();
    let ok = device["input"].is_null()
        && midi.as_object().is_some_and(|m| m.len() == 2)
        && id.is_some_and(|i| i.trim() == i)
        && name.is_some_and(|n| !n.trim().is_empty() && n.trim() == n);
    (!ok).then_some("settings.devices.midiDestinationRequired")
}

/// `deviceValidationError` for output devices: a translation key.
pub fn validation_error(device: &Value) -> Option<&'static str> {
    if device["title"].as_str().is_none_or(|t| t.trim().is_empty()) {
        return Some("settings.devices.nameRequired");
    }
    if let Some(error) = super::protocols::input_error(device) {
        return Some(error);
    }
    match device["transport"].as_str() {
        Some("midi") => midi_error(device),
        Some("serial") => serial_error(&device["serial"]),
        Some("tcp" | "udp") => {
            if device["address"]
                .as_str()
                .is_none_or(|a| a.is_empty() || a.chars().any(char::is_whitespace))
            {
                Some("settings.devices.addressRequired")
            } else if !device["port"]
                .as_u64()
                .is_some_and(|p| (1..=65535).contains(&p))
            {
                Some("settings.devices.portInvalid")
            } else {
                None
            }
        }
        _ => Some("settings.devices.unsupported"),
    }
}

/// `withCommunicationTransport`: a serial device gets default line settings.
pub fn with_transport(device: &Value, transport: &str) -> Value {
    let mut next = device.clone();
    next["transport"] = json!(transport);
    if transport == "serial" && next["serial"].is_null() {
        next["serial"] = json!({"path": "", "baudRate": 9600, "dataBits": 8, "parity": "none", "stopBits": 1, "flowControl": "none"});
    }
    next
}

/// `readMidiNote`: exactly the four string fields, and a known command.
pub fn read_midi_note(value: &str) -> Option<Value> {
    let draft: Value = serde_json::from_str(value).ok()?;
    let object = draft.as_object()?;
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    (matches!(draft["command"].as_str(), Some("noteOn" | "noteOff"))
        && keys == ["channel", "command", "note", "velocity"]
        && ["channel", "note", "velocity"]
            .iter()
            .all(|k| draft[*k].is_string()))
    .then_some(draft)
}

/// `newMidiNote`.
pub fn new_midi_note(command: &str) -> String {
    json!({"command": command, "channel": "1", "note": "60", "velocity": if command == "noteOn" { "127" } else { "0" }}).to_string()
}

/// `encodeMidiNote`: decimal text without leading zeros, channel 1-16, note and velocity 0-127.
pub fn encode_midi_note(value: &str) -> Result<Vec<u8>, &'static str> {
    const INVALID: &str = "settings.devices.midiInvalid";
    let draft = read_midi_note(value).ok_or(INVALID)?;
    let number = |key: &str| -> Result<u32, &'static str> {
        let text = draft[key].as_str().unwrap_or("");
        let plain = text == "0"
            || (!text.is_empty()
                && !text.starts_with('0')
                && text.chars().all(|c| c.is_ascii_digit()));
        if plain {
            text.parse().map_err(|_| INVALID)
        } else {
            Err(INVALID)
        }
    };
    let (channel, note, velocity) = (number("channel")?, number("note")?, number("velocity")?);
    if !(1..=16).contains(&channel) || note > 127 || velocity > 127 {
        return Err(INVALID);
    }
    let status: u8 = if draft["command"] == "noteOn" {
        0x90
    } else {
        0x80
    };
    Ok(vec![
        status | (channel as u8 - 1),
        note as u8,
        velocity as u8,
    ])
}

/// `encodeCommunicationPayload`: UTF-8 text is sent as typed, hex as exact bytes with no implicit terminator.
pub fn encode_payload(payload: &str, encoding: &str) -> Result<Value, &'static str> {
    if payload.is_empty() {
        return Err("settings.devices.payloadRequired");
    }
    if encoding == "midi-note" {
        return encode_midi_note(payload).map(|bytes| json!(bytes));
    }
    if encoding == "globalcache-relay" {
        return super::protocols::encode_globalcache(payload);
    }
    if encoding.starts_with("rosstalk-") {
        return super::protocols::encode_rosstalk(payload, encoding).map(|wire| json!(wire));
    }
    if encoding != "hex" {
        return Ok(json!(payload));
    }
    let digits: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.is_empty()
        || !digits.len().is_multiple_of(2)
        || !digits.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err("settings.devices.hexInvalid");
    }
    let bytes: Vec<u8> = (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).expect("hex digits"))
        .collect();
    Ok(json!(bytes))
}

#[derive(Default)]
pub struct Devices {
    pub draft: Option<Value>,
    pub payload: String,
    pub encoding: String,
    pub serial_ports: Vec<String>,
    pub midi_ports: Vec<Value>,
    pub feedback: Option<String>,
    pub busy: bool,
}

impl SettingsView {
    fn device_list(&self, cx: &gpui::App) -> Vec<Value> {
        self.runtime.content.read(cx).workspace["communicationDevices"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn devices_edit(&mut self, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        if let Some(draft) = self.devices.draft.as_mut() {
            change(draft);
            cx.notify();
        }
    }

    /// Saves the whole list (`persist`); the canonical workspace comes back.
    pub(super) fn devices_persist(
        &mut self,
        list: Vec<Value>,
        close: bool,
        cx: &mut Context<Self>,
    ) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        if path.is_empty() {
            self.devices.feedback = Some(
                cx.global::<Translator>()
                    .t("automation.macros.workspaceRequired"),
            );
            cx.notify();
            return;
        }
        #[cfg(test)]
        self.runtime.sent.borrow_mut().push((
            "saveWorkspaceContent",
            json!({"communicationDevices": list}),
        ));
        self.devices.busy = true;
        let (host, scope) = (self.runtime.host.clone(), path.clone());
        let answer = self.runtime.lanes.workspace(&path, move |revision| {
            let saved = host.invoke(
                "saveWorkspaceContent",
                vec![
                    json!(scope),
                    json!({"communicationDevices": list}),
                    json!(revision),
                ],
            )?;
            let fresh = host.invoke("readWorkspaceSnapshot", vec![json!(scope)])?;
            Ok(json!({"workspace": fresh["workspace"], "revision": saved["revision"]}))
        });
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let result = answer
                .await
                .map_err(|_| "The workspace queue stopped.".to_string())
                .and_then(|r| r);
            let parsed = match result {
                Ok(done) => {
                    cx.background_spawn(async move { Snapshot::parse(&done["workspace"]) })
                        .await
                }
                Err(error) => Err(error),
            };
            view.update(cx, |view, cx| {
                view.devices.busy = false;
                match parsed {
                    Ok(snapshot) if runtime.content.read(cx).workspace_path == path => {
                        runtime.content.update(cx, |content, cx| {
                            content.apply_canonical(snapshot);
                            cx.notify();
                        });
                        let t = *cx.global::<Translator>();
                        view.devices.feedback = Some(t.t("settings.devices.saved"));
                        if close {
                            view.devices.draft = None;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => view.devices.feedback = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn devices_save(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.devices.draft.clone() else {
            return;
        };
        if let Some(error) = validation_error(&draft) {
            self.devices.feedback = Some(cx.global::<Translator>().t(error));
            cx.notify();
            return;
        }
        let mut list = self.device_list(cx);
        match list.iter_mut().find(|d| d["id"] == draft["id"]) {
            Some(slot) => *slot = draft,
            None => list.push(draft),
        }
        self.devices_persist(list, true, cx);
    }

    /// Connects or disconnects an input device. Both open a socket or a serial port, so only a button press does it.
    pub(super) fn devices_connection(
        &mut self,
        method: &'static str,
        id: &str,
        cx: &mut Context<Self>,
    ) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        self.host_call(
            method,
            vec![json!(path), json!(id)],
            cx,
            |view, result, cx| {
                view.devices.feedback = result.err();
                cx.notify();
            },
        );
    }

    pub(super) fn devices_delete(&mut self, id: &str, cx: &mut Context<Self>) {
        let list: Vec<Value> = self
            .device_list(cx)
            .into_iter()
            .filter(|d| d["id"] != id)
            .collect();
        self.devices_persist(list, false, cx);
    }

    /// `testCommunicationDevice`: invalid devices and malformed hex never reach the host.
    pub(super) fn devices_test(&mut self, cx: &mut Context<Self>) {
        let Some(device) = self.devices.draft.clone() else {
            return;
        };
        let t = *cx.global::<Translator>();
        if let Some(error) = validation_error(&device) {
            self.devices.feedback = Some(t.t(error));
            cx.notify();
            return;
        }
        let bytes = match encode_payload(&self.devices.payload, &self.devices.encoding) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.devices.feedback = Some(t.t(error));
                cx.notify();
                return;
            }
        };
        self.host_call(
            "sendCommunication",
            vec![device, bytes],
            cx,
            |view, result, cx| {
                let t = *cx.global::<Translator>();
                view.devices.feedback = Some(match result {
                    Ok(r) if r["ok"] == json!(true) => t.t("settings.devices.sent"),
                    Ok(_) => "Communication send failed.".to_string(),
                    Err(error) => error,
                });
                cx.notify();
            },
        );
    }

    /// `releaseCommunicationMidiNotes`: a failed release is reported.
    pub(super) fn devices_release_notes(&mut self, cx: &mut Context<Self>) {
        self.host_call(
            "releaseMidiOutputNotes",
            Vec::new(),
            cx,
            |view, result, cx| {
                let t = *cx.global::<Translator>();
                view.devices.feedback = match result {
                    Ok(r) if r["ok"] == json!(true) => None,
                    Ok(_) => Some(t.t("settings.devices.midiReleaseFailed")),
                    Err(error) => Some(error),
                };
                cx.notify();
            },
        );
    }

    pub(super) fn devices_refresh_ports(&mut self, cx: &mut Context<Self>) {
        self.host_call("listSerialPorts", Vec::new(), cx, |view, result, cx| {
            match result {
                Ok(list) => {
                    view.devices.serial_ports = list
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|p| p.as_str().map(str::to_string))
                        .collect()
                }
                Err(error) => view.devices.feedback = Some(error),
            }
            cx.notify();
        });
        self.host_call("listMidiOutputPorts", Vec::new(), cx, |view, result, cx| {
            match result {
                Ok(list) => view.devices.midi_ports = list.as_array().cloned().unwrap_or_default(),
                Err(error) => view.devices.feedback = Some(error),
            }
            cx.notify();
        });
    }

    pub(super) fn devices_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let feedback_color = cx.theme().colors.text_muted;
        let list = self.device_list(cx);
        let weak = cx.weak_entity();
        let statuses = cx
            .try_global::<crate::automation::inputs::Inputs>()
            .map(|s| s.communication_status.clone())
            .unwrap_or_default();
        let mut panel = stack(cx)
            .children(statuses.iter().map(|s|div().child(format!("{}: {}",s["deviceId"].as_str().unwrap_or(""),s["error"].as_str().map(str::to_string).unwrap_or_else(||t.t(if s["listening"]==true{"settings.devices.listening"}else{"settings.devices.disconnected"}))))))
            .id("settings-devices")
            .child(title(t.t("settings.devices.title"), cx))
            .child(muted(t.t("settings.devices.description"), cx))
            .child(action_row(cx).child(ActionButton::new("devices-add", t.t("settings.devices.add")).on_click(cx.listener(|view, _, _, cx| {
                view.devices.draft = Some(json!({"id": uuid::Uuid::new_v4().to_string(), "title": "", "transport": "tcp", "address": ""}));
                view.devices.feedback = None;
                view.devices_refresh_ports(cx);
                cx.notify();
            }))));
        for device in &list {
            let id = device["id"].as_str().unwrap_or("").to_string();
            let (edit, remove) = (device.clone(), id.clone());
            panel = panel.child(
                action_row(cx)
                    .child(format!(
                        "{} - {}",
                        device["title"].as_str().unwrap_or(""),
                        device["transport"].as_str().unwrap_or("")
                    ))
                    .child(
                        ActionButton::new(
                            SharedString::from(format!("device-edit-{id}")),
                            t.t("automation.macros.edit"),
                        )
                        .variant(ButtonVariant::Secondary)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.devices.draft = Some(edit.clone());
                            view.devices.encoding = if edit["transport"] == "midi" {
                                "midi-note".into()
                            } else {
                                "utf8".into()
                            };
                            view.devices.payload = if edit["transport"] == "midi" {
                                new_midi_note("noteOn")
                            } else {
                                String::new()
                            };
                            cx.notify();
                        })),
                    )
                    .children(device["input"].is_object().then(|| {
                        let (connect, disconnect) = (id.clone(), id.clone());
                        action_row(cx)
                            .child(
                                ActionButton::new(
                                    SharedString::from(format!("device-connect-{id}")),
                                    t.t("settings.devices.connect"),
                                )
                                .variant(ButtonVariant::Secondary)
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.devices_connection(
                                            "connectCommunicationInput",
                                            &connect,
                                            cx,
                                        )
                                    },
                                )),
                            )
                            .child(
                                ActionButton::new(
                                    SharedString::from(format!("device-disconnect-{id}")),
                                    t.t("settings.devices.disconnect"),
                                )
                                .variant(ButtonVariant::Secondary)
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.devices_connection(
                                            "disconnectCommunicationInput",
                                            &disconnect,
                                            cx,
                                        )
                                    },
                                )),
                            )
                    }))
                    .child(
                        ActionButton::new(
                            SharedString::from(format!("device-remove-{id}")),
                            t.t("automation.macros.remove"),
                        )
                        .variant(ButtonVariant::Danger)
                        .on_click(
                            cx.listener(move |view, _, _, cx| view.devices_delete(&remove, cx)),
                        ),
                    ),
            );
        }
        if let Some(draft) = self.devices.draft.clone() {
            let id = draft["id"].as_str().unwrap_or("").to_string();
            panel = panel.child(self.text_field(
                &format!("device-name-{id}"),
                t.t("settings.devices.name"),
                draft["title"].as_str().unwrap_or(""),
                window,
                cx,
                |v, name, cx| v.devices_edit(|d| d["title"] = json!(name.trim()), cx),
            ));
            panel = panel.child(self.input_fields(&draft, window, cx));
            let transport_weak = weak.clone();
            panel = panel.child(Field::new(
                t.t("settings.devices.transport"),
                Select::new(
                    "device-transport",
                    TRANSPORTS
                        .iter()
                        .map(|x| SelectOption::new(x.to_string(), x.to_uppercase()))
                        .collect(),
                    draft["transport"].as_str().unwrap_or("tcp").to_string(),
                )
                .on_change(move |value, _, cx| {
                    transport_weak
                        .update(cx, |v, cx| {
                            v.devices_edit(|d| *d = with_transport(d, &value), cx)
                        })
                        .ok();
                }),
            ));
            match draft["transport"].as_str().unwrap_or("") {
                "tcp" | "udp" => {
                    panel = panel.child(self.text_field(
                        &format!("device-address-{id}"),
                        t.t("settings.devices.address"),
                        draft["address"].as_str().unwrap_or(""),
                        window,
                        cx,
                        |v, a, cx| v.devices_edit(|d| d["address"] = json!(a.trim()), cx),
                    ));
                    panel = panel.child(self.number_field(
                        &format!("device-port-{id}"),
                        t.t("settings.devices.port"),
                        &draft["port"].as_u64().unwrap_or(0).to_string(),
                        (1., 65535.),
                        1.,
                        window,
                        cx,
                        |v, p, cx| {
                            if let Ok(p) = p.trim().parse::<u64>() {
                                v.devices_edit(|d| d["port"] = json!(p), cx)
                            }
                        },
                    ));
                }
                "serial" => {
                    let serial = draft["serial"].clone();
                    panel = panel.child(self.text_field(
                        &format!("device-serial-path-{id}"),
                        t.t("settings.devices.serialPath"),
                        serial["path"].as_str().unwrap_or(""),
                        window,
                        cx,
                        |v, p, cx| v.devices_edit(|d| d["serial"]["path"] = json!(p), cx),
                    ));
                    panel = panel.child(self.number_field(
                        &format!("device-serial-baud-{id}"),
                        t.t("settings.devices.baudRate"),
                        &serial["baudRate"].as_u64().unwrap_or(0).to_string(),
                        (1., 4_000_000.),
                        1.,
                        window,
                        cx,
                        |v, b, cx| {
                            if let Ok(b) = b.trim().parse::<u64>() {
                                v.devices_edit(|d| d["serial"]["baudRate"] = json!(b), cx)
                            }
                        },
                    ));
                    for (key, options) in [
                        ("parity", vec!["none", "odd", "even"]),
                        ("flowControl", vec!["none", "software", "hardware"]),
                    ] {
                        let w = weak.clone();
                        panel = panel.child(Field::new(
                            key.to_string(),
                            Select::new(
                                SharedString::from(format!("device-serial-{key}")),
                                options
                                    .iter()
                                    .map(|o| SelectOption::new(o.to_string(), o.to_string()))
                                    .collect(),
                                serial[key].as_str().unwrap_or("none").to_string(),
                            )
                            .on_change(move |value, _, cx| {
                                w.update(cx, |v, cx| {
                                    v.devices_edit(|d| d["serial"][key] = json!(value), cx)
                                })
                                .ok();
                            }),
                        ));
                    }
                    for (key, options) in
                        [("dataBits", vec![5u64, 6, 7, 8]), ("stopBits", vec![1, 2])]
                    {
                        let w = weak.clone();
                        panel = panel.child(Field::new(
                            key.to_string(),
                            Select::new(
                                SharedString::from(format!("device-serial-{key}")),
                                options
                                    .iter()
                                    .map(|o| SelectOption::new(*o, o.to_string()))
                                    .collect(),
                                serial[key].as_u64().unwrap_or(8),
                            )
                            .on_change(move |value, _, cx| {
                                w.update(cx, |v, cx| {
                                    v.devices_edit(|d| d["serial"][key] = json!(value), cx)
                                })
                                .ok();
                            }),
                        ));
                    }
                    if !self.devices.serial_ports.is_empty() {
                        let w = weak.clone();
                        panel = panel.child(Field::new(
                            t.t("settings.devices.refreshPorts"),
                            Select::new(
                                "device-serial-ports",
                                self.devices
                                    .serial_ports
                                    .iter()
                                    .map(|p| SelectOption::new(p.clone(), p.clone()))
                                    .collect(),
                                serial["path"].as_str().map(str::to_string),
                            )
                            .on_change(move |port, _, cx| {
                                w.update(cx, |v, cx| {
                                    v.devices_edit(|d| d["serial"]["path"] = json!(port), cx)
                                })
                                .ok();
                            }),
                        ));
                    }
                }
                "midi" => {
                    let w = weak.clone();
                    let ports = self.devices.midi_ports.clone();
                    panel = panel.child(Field::new(
                        t.t("settings.devices.midiDestination"),
                        Select::new(
                            "device-midi",
                            ports
                                .iter()
                                .map(|p| {
                                    SelectOption::new(
                                        p["id"].as_str().unwrap_or("").to_string(),
                                        p["name"].as_str().unwrap_or("").to_string(),
                                    )
                                })
                                .collect(),
                            draft["midi"]["id"].as_str().map(str::to_string),
                        )
                        .on_change(move |id, _, cx| {
                            let Some(port) = ports.iter().find(|p| p["id"] == id.as_str()).cloned()
                            else {
                                return;
                            };
                            w.update(cx, |v, cx| {
                                v.devices_edit(
                                    |d| d["midi"] = json!({"id": port["id"], "name": port["name"]}),
                                    cx,
                                )
                            })
                            .ok();
                        }),
                    ));
                }
                _ => {}
            }
            let invalid = validation_error(&draft);
            panel = panel.children(invalid.map(|e| muted(t.t(e), cx)));
            if draft["transport"] == "midi" {
                let note = read_midi_note(&self.devices.payload).unwrap_or_else(|| json!({"command": "noteOn", "channel": "1", "note": "60", "velocity": "127"}));
                let command_weak = weak.clone();
                let command_note = note.clone();
                panel = panel.child(Field::new(
                    t.t("settings.devices.encoding.midi-note"),
                    Select::new(
                        "device-midi-command",
                        vec![
                            SelectOption::new("noteOn".to_string(), "Note On".to_string()),
                            SelectOption::new("noteOff".to_string(), "Note Off".to_string()),
                        ],
                        note["command"].as_str().unwrap_or("noteOn").to_string(),
                    )
                    .on_change(move |command, _, cx| {
                        let mut next = command_note.clone();
                        next["command"] = json!(command);
                        command_weak
                            .update(cx, |v, cx| {
                                v.devices.payload = next.to_string();
                                cx.notify();
                            })
                            .ok();
                    }),
                ));
                for key in ["channel", "note", "velocity"] {
                    let current = note.clone();
                    panel = panel.child(self.text_field(
                        &format!("device-midi-{key}"),
                        key.to_string(),
                        note[key].as_str().unwrap_or(""),
                        window,
                        cx,
                        move |v, text, cx| {
                            let mut next = current.clone();
                            next[key] = json!(text.trim());
                            v.devices.payload = next.to_string();
                            cx.notify();
                        },
                    ));
                }
                panel = panel.child(
                    action_row(cx)
                        .child(
                            ActionButton::new("device-midi-test", t.t("settings.devices.test"))
                                .variant(ButtonVariant::Secondary)
                                .disabled(invalid.is_some())
                                .on_click(cx.listener(|v, _, _, cx| v.devices_test(cx))),
                        )
                        .child(
                            ActionButton::new(
                                "device-midi-release",
                                t.t("settings.devices.midiRelease"),
                            )
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|v, _, _, cx| v.devices_release_notes(cx))),
                        ),
                );
            }
            if draft["transport"] != "midi" {
                panel = panel.child(self.text_field(
                    "device-payload",
                    t.t("settings.devices.payload"),
                    &self.devices.payload.clone(),
                    window,
                    cx,
                    |v, p, cx| {
                        v.devices.payload = p;
                        cx.notify();
                    },
                ));
                let w = weak.clone();
                panel = panel.child(Field::new(
                    t.t("settings.devices.encoding"),
                    Select::new(
                        "device-encoding",
                        vec![
                            SelectOption::new("utf8".to_string(), "UTF-8".to_string()),
                            SelectOption::new("hex".to_string(), "Hex".to_string()),
                            SelectOption::new(
                                "globalcache-relay".to_string(),
                                t.t("settings.devices.encoding.globalcache-relay"),
                            ),
                            SelectOption::new(
                                "rosstalk-carbonite".to_string(),
                                t.t("settings.devices.encoding.rosstalk-carbonite"),
                            ),
                            SelectOption::new(
                                "rosstalk-acuity".to_string(),
                                t.t("settings.devices.encoding.rosstalk-acuity"),
                            ),
                        ],
                        self.devices.encoding.clone(),
                    )
                    .on_change(move |value, _, cx| {
                        w.update(cx, |v, cx| {
                            // A structured protocol starts from its template; the payload is its JSON.
                            v.devices.payload = match value.as_str() {
                                "globalcache-relay" => super::protocols::new_globalcache("on"),
                                "rosstalk-carbonite" | "rosstalk-acuity" => {
                                    super::protocols::new_rosstalk("FTB")
                                }
                                _ => String::new(),
                            };
                            v.devices.encoding = value;
                            cx.notify()
                        })
                        .ok();
                    }),
                ));
            }
            panel = panel.child(
                action_row(cx)
                    .child(
                        ActionButton::new("device-save", t.t("automation.macros.save"))
                            .disabled(invalid.is_some() || self.devices.busy)
                            .on_click(cx.listener(|v, _, _, cx| v.devices_save(cx))),
                    )
                    .children((draft["transport"] != "midi").then(|| {
                        ActionButton::new("device-test", t.t("settings.devices.test"))
                            .variant(ButtonVariant::Secondary)
                            .disabled(invalid.is_some())
                            .on_click(cx.listener(|v, _, _, cx| v.devices_test(cx)))
                    }))
                    .child(
                        ActionButton::new("device-cancel", t.t("videoInputs.cancel"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.devices.draft = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        panel
            .child(self.timecode_panel(window, cx))
            .children(self.devices.feedback.clone().map(|f| {
                div()
                    .id("devices-feedback")
                    .text_color(feedback_color)
                    .child(f)
            }))
            .into_any_element()
    }
}
