//! `AudioInputsPanelBridge.tsx`, `audio-input-controller.ts`, and `core/audio-input.ts`: the draft of the
//! selected audio input and how it is saved. The rows are in `input_tab`. Listing devices only reads names; no
//! input is opened here.

use gpui::{AppContext, Context};
use orbit_ui::Translator;
use serde_json::{json, Value};

use super::view::SettingsView;
use crate::collections::model::Snapshot;

pub const MODES: [&str; 4] = ["off", "on", "auto-off", "auto-on"];

fn number_in(value: &Value, min: f64, max: f64, integer: bool) -> bool {
    value
        .as_f64()
        .is_some_and(|n| n.is_finite() && n >= min && n <= max && (!integer || n.fract() == 0.))
}

/// `audioInputError` for a device source with stereo routing: a translation key.
pub fn input_error(input: &Value) -> Option<&'static str> {
    if input["id"].as_str().is_none_or(|s| s.trim().is_empty())
        || input["title"].as_str().is_none_or(|s| s.trim().is_empty())
    {
        return Some("audioInputs.nameRequired");
    }
    if !MODES.iter().any(|m| input["behaviorMode"] == *m) {
        return Some("audioInputs.invalid");
    }
    let capture = &input["capture"];
    if !capture.is_object() {
        return Some("audioInputs.sourceRequired");
    }
    if capture["ndiSource"].is_null() {
        if capture["deviceName"]
            .as_str()
            .is_none_or(|n| n.trim().is_empty())
        {
            return Some("audioInputs.sourceRequired");
        }
    } else {
        if !capture["deviceName"].is_null() {
            return Some("audioInputs.invalid");
        }
        let ndi = &capture["ndiSource"];
        let text = |k: &str| ndi[k].as_str().map(str::to_string);
        match (text("name"), text("urlAddress")) {
            (Some(name), Some(address))
                if !name.trim().is_empty() && !address.trim().is_empty() =>
            {
                if name.contains('\0') || address.contains('\0') {
                    return Some("audioInputs.invalid");
                }
            }
            _ => return Some("audioInputs.sourceRequired"),
        }
    }
    let routing_ok = if capture["routing"].is_null() {
        capture["channels"]
            .as_array()
            .is_some_and(|c| c.len() == 2 && c.iter().all(|v| number_in(v, 1., 64., true)))
    } else {
        capture["channels"].is_null()
            && super::audio_routing::routing_error(&capture["routing"]).is_none()
    };
    if !routing_ok
        || !number_in(&capture["levelDb"], -60., 6., false)
        || !number_in(&capture["delayMs"], 0., 2000., true)
        || !number_in(&capture["latencyMs"], 10., 200., true)
        || !input["linkedVideoInputIds"].as_array().is_some_and(|l| {
            l.iter()
                .all(|id| id.as_str().is_some_and(|s| !s.is_empty()))
        })
    {
        return Some("audioInputs.invalid");
    }
    None
}

pub fn new_input() -> Value {
    json!({
        "id": uuid::Uuid::new_v4().to_string(), "title": "", "behaviorMode": "off", "linkedVideoInputIds": [],
        "capture": {"deviceName": "", "channels": [1, 1], "levelDb": 0, "delayMs": 0, "latencyMs": 50},
    })
}

#[derive(Default)]
pub struct AudioInputs {
    pub draft: Option<Value>,
    pub devices: Vec<(String, u64)>,
    pub error: Option<String>,
    pub busy: bool,
    pub ndi_sources: Vec<Value>,
    pub new_route: (u64, u64),
    pub status: Value,
    pub polling: bool,
    /// An edit arrived while a save was running.
    pub dirty: bool,
    pub ndi_loading: bool,
}

impl SettingsView {
    pub(super) fn audio_input_edit(
        &mut self,
        change: impl FnOnce(&mut Value),
        cx: &mut Context<Self>,
    ) {
        if let Some(draft) = self.audio_inputs.draft.as_mut() {
            change(draft);
            cx.notify();
        }
    }

    pub(super) fn audio_inputs_persist(
        &mut self,
        change: impl FnOnce(&[Value]) -> Vec<Value> + Send + 'static,
        close: bool,
        cx: &mut Context<Self>,
    ) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        if path.is_empty() {
            self.audio_inputs.error = Some(
                cx.global::<Translator>()
                    .t("automation.macros.workspaceRequired"),
            );
            cx.notify();
            return;
        }
        self.audio_inputs.busy = true;
        self.audio_inputs.error = None;
        let (host, scope) = (self.runtime.host.clone(), path.clone());
        let answer = self.runtime.lanes.workspace(&path, move |_| {
            let snapshot = host.invoke("readWorkspaceSnapshot", vec![json!(scope)])?;
            let current = snapshot["workspace"]["audioInputs"]
                .as_array()
                .ok_or("audioInputs.invalid")?
                .clone();
            let next = change(&current);
            let saved = host.invoke(
                "saveWorkspaceContent",
                vec![
                    json!(scope),
                    json!({"audioInputs": next}),
                    snapshot["revision"].clone(),
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
                view.audio_inputs.busy = false;
                if std::mem::take(&mut view.audio_inputs.dirty) {
                    view.audio_input_autosave(cx);
                }
                match parsed {
                    Ok(snapshot) if runtime.content.read(cx).workspace_path == path => {
                        runtime.content.update(cx, |content, cx| {
                            content.apply_canonical(snapshot);
                            cx.notify();
                        });
                        if close {
                            view.audio_inputs.draft = None;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => view.audio_inputs.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Saves the draft and keeps it selected, as the Inputs tab does after every edit.
    fn audio_input_save(&mut self, cx: &mut Context<Self>) {
        let Some(mut input) = self.audio_inputs.draft.clone() else {
            return;
        };
        if let Some(error) = input_error(&input) {
            self.audio_inputs.error = Some(cx.global::<Translator>().t(error));
            cx.notify();
            return;
        }
        input["title"] = json!(input["title"].as_str().unwrap_or("").trim());
        self.audio_inputs_persist(
            move |current| {
                if current.iter().any(|e| e["id"] == input["id"]) {
                    current
                        .iter()
                        .map(|e| {
                            if e["id"] == input["id"] {
                                input.clone()
                            } else {
                                e.clone()
                            }
                        })
                        .collect()
                } else {
                    current.iter().cloned().chain([input.clone()]).collect()
                }
            },
            false,
            cx,
        );
    }

    pub(super) fn audio_input_delete(&mut self, id: String, cx: &mut Context<Self>) {
        self.audio_inputs_persist(
            move |current| {
                current
                    .iter()
                    .filter(|e| e["id"] != id.as_str())
                    .cloned()
                    .collect()
            },
            true,
            cx,
        );
    }

    /// Lists the devices again every two seconds and reads the input status four times a second while the audio
    /// side of the Inputs tab is shown; stops when it is not. The browser's `devicechange` has no counterpart.
    pub(super) fn poll_audio_inputs(&mut self, cx: &mut Context<Self>) {
        if self.audio_inputs.polling {
            return;
        }
        self.audio_inputs.polling = true;
        cx.spawn(async move |this, cx| {
            let mut tick = 0u32;
            loop {
                let Ok(Some(host)) = this.update(cx, |view, _| {
                    if view.panel == "inputs" && view.input_tab.audio_shown() {
                        Some(view.runtime.host.clone())
                    } else {
                        view.audio_inputs.polling = false;
                        None
                    }
                }) else {
                    break;
                };
                let refresh_devices = tick.is_multiple_of(8);
                tick += 1;
                let result = cx
                    .background_spawn(async move {
                        let devices = refresh_devices
                            .then(|| host.invoke("listAudioInputDevices", Vec::new()));
                        let status = crate::state::runtime::output_call(
                            &host,
                            "getAudioInputStatus",
                            vec![],
                        );
                        (devices, status)
                    })
                    .await;
                if this
                    .update(cx, |view, cx| {
                        let (devices, status) = result;
                        match status {
                            Ok(v) => view.audio_inputs.status = v,
                            Err(e) => view.audio_inputs.error = Some(e),
                        }
                        match devices {
                            Some(Ok(list)) => {
                                view.audio_inputs.devices = list
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|d| {
                                        Some((
                                            d["name"].as_str()?.to_string(),
                                            d["channels"].as_u64()?,
                                        ))
                                    })
                                    .collect()
                            }
                            Some(Err(error)) => view.audio_inputs.error = Some(error),
                            None => {}
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
            }
        })
        .detach();
    }

    /// `change`: saves the draft when it is valid; an invalid one only shows why. A save that is still running
    /// makes the next one wait, so a dragged slider does not queue a write per step.
    pub(super) fn audio_input_autosave(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.audio_inputs.draft.as_ref() else {
            return;
        };
        if input_error(draft).is_some() {
            cx.notify();
            return;
        }
        if self.audio_inputs.busy {
            self.audio_inputs.dirty = true;
            return;
        }
        self.audio_input_save(cx);
    }

    /// `change` of the panel: edits the draft, then saves it.
    pub(super) fn audio_input_change(
        &mut self,
        change: impl FnOnce(&mut Value),
        cx: &mut Context<Self>,
    ) {
        self.audio_input_edit(change, cx);
        self.audio_input_autosave(cx);
    }

    /// `select`: the saved input becomes the draft.
    pub(super) fn audio_input_select(&mut self, id: &str, cx: &mut Context<Self>) {
        self.audio_inputs.error = None;
        self.audio_inputs.draft = self.runtime.content.read(cx).workspace["audioInputs"]
            .as_array()
            .and_then(|inputs| inputs.iter().find(|input| input["id"] == id))
            .cloned();
        cx.notify();
    }

    /// `removeSelected`: an input that was never saved is just dropped.
    pub(super) fn audio_input_remove_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self
            .audio_inputs
            .draft
            .take()
            .and_then(|draft| draft["id"].as_str().map(str::to_string))
        else {
            return;
        };
        let saved = self.runtime.content.read(cx).workspace["audioInputs"]
            .as_array()
            .is_some_and(|inputs| inputs.iter().any(|input| input["id"] == id.as_str()));
        if saved {
            self.audio_input_delete(id, cx);
        }
        cx.notify();
    }

    pub(super) fn audio_input_add(&mut self, cx: &mut Context<Self>) {
        self.audio_inputs.error = None;
        let t = *cx.global::<Translator>();
        let mut input = new_input();
        input["title"] = json!(t.t("inputs.audio.new"));
        input["capture"]["latencyMs"] = json!(30);
        input["capture"]["channels"] = json!([1, 2]);
        self.audio_inputs.draft = Some(input);
        cx.notify();
    }
}
