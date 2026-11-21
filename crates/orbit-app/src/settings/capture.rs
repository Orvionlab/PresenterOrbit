//! `CapturePanelBridge.tsx`, `CaptureDestinationFields.tsx`, `CaptureAudioMapEditor.tsx`, `capture-preset-draft.ts`,
//! and `capture-preview-status.ts`. Presets are workspace data saved with `saveCapturePreset`; the recording itself
//! runs in the output process (`output::capture`). The panel sits in the Screens window.

use std::time::Duration;

use super::ui::{Field, OutputField};
use gpui::{div, prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{
    password_state, ActionButton, ButtonVariant, CheckboxField, InputState, Select, SelectOption,
    TextInput,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::ui::{action_row, heading, muted, stack};
use super::view::SettingsView;
use crate::collections::model::Snapshot;

#[derive(Default)]
pub struct Capture {
    pub selected: String,
    pub draft: Option<Value>,
    pub status: Value,
    pub error: Option<String>,
    pub busy: bool,
    /// The stop button asks first, as `capture.confirmStop` does for an action.
    pub confirm_stop: bool,
    #[cfg_attr(test, allow(dead_code))]
    polling: bool,
    key: Option<gpui::Entity<InputState>>,
}

/// A new preset draft: `capture-preset-draft.ts`.
pub fn new_draft(screen: &str, id: &str, title: &str) -> Value {
    json!({
        "id": id, "title": title, "sourceScreenId": screen, "width": 1920, "height": 1080,
        "fps": 30, "bitrateKbps": 8000, "outputDirectory": "",
    })
}

pub fn presets(workspace: &Value) -> Vec<Value> {
    workspace["recordingPresets"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The draft's integer field, or the reason it is invalid.
pub fn parse_number(text: &str, min: f64, max: f64) -> Option<u64> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && v.fract() == 0. && *v >= min && *v <= max)
        .map(|v| v as u64)
}

/// A custom audio map is a list of recording channels, each summing screen channels 1 and 2.
pub fn toggle_map_source(map: &Value, channel: usize, source: u64) -> Value {
    let mut next = map.as_array().cloned().unwrap_or_default();
    if let Some(entry) = next.get_mut(channel).and_then(Value::as_array_mut) {
        if let Some(at) = entry.iter().position(|v| v == &json!(source)) {
            entry.remove(at);
        } else {
            entry.push(json!(source));
            entry.sort_by_key(|v| v.as_u64());
        }
    }
    Value::Array(next)
}

impl SettingsView {
    #[cfg_attr(test, allow(dead_code))]
    fn poll_capture(&mut self, cx: &mut Context<Self>) {
        if self.capture.polling {
            return;
        }
        self.capture.polling = true;
        cx.spawn(async move |view, cx| loop {
            let host = view.update(cx, |view, _| view.runtime.host.clone());
            let Ok(host) = host else { break };
            let status = cx
                .background_spawn(async move { crate::output::capture::status(&host) })
                .await;
            let keep = view.update(cx, |view, cx| {
                match status {
                    Ok(status) => view.capture.status = status,
                    Err(error) => view.capture.error = Some(error),
                }
                cx.notify();
                true
            });
            if keep.is_err() {
                break;
            }
            cx.background_executor().timer(Duration::from_secs(1)).await;
        })
        .detach();
    }

    pub(super) fn capture_refresh(&mut self, cx: &mut Context<Self>) {
        let host = self.runtime.host.clone();
        cx.spawn(async move |view, cx| {
            let status = cx
                .background_spawn(async move { crate::output::capture::status(&host) })
                .await;
            view.update(cx, |view, cx| {
                match status {
                    Ok(status) => view.capture.status = status,
                    Err(error) => view.capture.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn capture_edit(&mut self, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        if let Some(draft) = self.capture.draft.as_mut() {
            change(draft);
            cx.notify();
        }
    }

    pub(super) fn capture_new(&mut self, cx: &mut Context<Self>) {
        let t = *cx.global::<Translator>();
        let configs = self.screen_configs(cx);
        let screen = configs
            .as_object()
            .and_then(|map| {
                map.iter()
                    .find(|(_, c)| c["type"] == "audience" || c["type"] == "stage")
            })
            .map(|(id, _)| id.clone())
            .unwrap_or_default();
        let id = format!("recording-{}", uuid::Uuid::new_v4());
        self.capture.selected = id.clone();
        self.capture.draft = Some(new_draft(&screen, &id, &t.t("capture.new")));
        self.capture.key = None;
        cx.notify();
    }

    pub(super) fn capture_select(&mut self, id: &str, cx: &mut Context<Self>) {
        let workspace = self.runtime.content.read(cx).workspace.clone();
        let preset = presets(&workspace).into_iter().find(|p| p["id"] == id);
        self.capture.selected = id.to_string();
        self.capture.draft = preset.and_then(|p| {
            let mut capture = p["capture"].clone();
            capture.is_object().then(|| {
                capture["title"] = p["title"].clone();
                capture
            })
        });
        self.capture.key = None;
        self.capture.error = None;
        cx.notify();
    }

    /// `saveCapturePreset`: the stream key travels with the request only.
    pub(super) fn capture_save(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(mut preset) = self.capture.draft.clone() else {
            return;
        };
        let path = self.runtime.content.read(cx).workspace_path.clone();
        if path.is_empty() || self.capture.busy {
            return;
        }
        if !preset["streamUrl"].is_null() && !key.is_empty() {
            preset["streamKey"] = json!(key);
        }
        #[cfg(test)]
        self.runtime
            .sent
            .borrow_mut()
            .push(("saveCapturePreset", preset.clone()));
        self.capture.busy = true;
        self.capture.error = None;
        let (host, scope) = (self.runtime.host.clone(), path.clone());
        let answer = self.runtime.lanes.workspace(&path, move |revision| {
            let saved = host.invoke(
                "saveCapturePreset",
                vec![json!(scope), preset, json!(revision)],
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
                view.capture.busy = false;
                if runtime.content.read(cx).workspace_path != path {
                    return;
                }
                match parsed {
                    Ok(snapshot) => runtime.content.update(cx, |content, cx| {
                        content.apply_canonical(snapshot);
                        cx.notify();
                    }),
                    Err(error) => view.capture.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn capture_delete(&mut self, id: String, cx: &mut Context<Self>) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        if path.is_empty() || self.capture.busy {
            return;
        }
        #[cfg(test)]
        self.runtime
            .sent
            .borrow_mut()
            .push(("deleteCapturePreset", json!([path, id])));
        self.capture.busy = true;
        let (host, scope) = (self.runtime.host.clone(), path.clone());
        let answer = self.runtime.lanes.workspace(&path, move |revision| {
            let saved = host.invoke(
                "deleteCapturePreset",
                vec![json!(scope), json!(id), json!(revision)],
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
                view.capture.busy = false;
                match parsed {
                    Ok(snapshot) if runtime.content.read(cx).workspace_path == path => {
                        runtime.content.update(cx, |content, cx| {
                            content.apply_canonical(snapshot);
                            cx.notify();
                        });
                        view.capture.draft = None;
                        view.capture.selected.clear();
                    }
                    Ok(_) => {}
                    Err(error) => view.capture.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn capture_start(&mut self, id: String, cx: &mut Context<Self>) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        self.capture.error = None;
        let host = self.runtime.host.clone();
        cx.spawn(async move |view, cx| {
            let started = cx
                .background_spawn(async move { crate::output::capture::start(&host, &path, &id) })
                .await;
            view.update(cx, |view, cx| {
                match started {
                    Ok(status) => view.capture.status = status,
                    Err(error) => view.capture.error = Some(error),
                }
                view.capture_refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn capture_stop(&mut self, cx: &mut Context<Self>) {
        let host = self.runtime.host.clone();
        cx.spawn(async move |view, cx| {
            let stopped = cx
                .background_spawn(async move { crate::output::capture::stop(&host) })
                .await;
            view.update(cx, |view, cx| {
                match stopped {
                    Ok(status) => view.capture.status = status,
                    Err(error) => view.capture.error = Some(error),
                }
                view.capture_refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn capture_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(not(test))]
        self.poll_capture(cx);
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let workspace = self.runtime.content.read(cx).workspace.clone();
        let list = presets(&workspace);
        let status = self.capture.status.clone();
        let active = status["active"] == json!(true);
        let finalizing = status["finalizing"] == json!(true);
        let screens: Vec<(String, String)> = self
            .screen_configs(cx)
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, c)| c["type"] == "audience" || c["type"] == "stage")
            .map(|(id, c)| (id.clone(), c["label"].as_str().unwrap_or(id).to_string()))
            .collect();
        let weak = cx.weak_entity();
        let mut panel = stack(cx)
            .id("settings-capture")
            .child(heading(t.t("capture.title"), cx))
            .child(muted(t.t("capture.videoAndAudio"), cx))
            .child(
                action_row(cx)
                    .child(Field::new(
                        t.t("capture.preset"),
                        Select::new(
                            "capture-preset",
                            list.iter()
                                .map(|p| {
                                    SelectOption::new(
                                        p["id"].as_str().unwrap_or("").to_string(),
                                        p["title"].as_str().unwrap_or("").to_string(),
                                    )
                                })
                                .collect(),
                            Some(self.capture.selected.clone()).filter(|s| !s.is_empty()),
                        )
                        .on_change({
                            let weak = weak.clone();
                            move |id, _, cx| {
                                weak.update(cx, |view, cx| view.capture_select(&id, cx))
                                    .ok();
                            }
                        }),
                    ))
                    .child(
                        ActionButton::new("capture-new", t.t("capture.new"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|view, _, _, cx| view.capture_new(cx))),
                    ),
            );
        if let Some(draft) = self.capture.draft.clone() {
            let id = draft["id"].as_str().unwrap_or("").to_string();
            let streaming = !draft["streamUrl"].is_null();
            let text = |key: &str| draft[key].as_str().unwrap_or("").to_string();
            let num = |key: &str| draft[key].as_u64().unwrap_or(0).to_string();
            panel = panel.child(self.text_field(
                &format!("capture-title-{id}"),
                t.t("capture.name"),
                &text("title"),
                window,
                cx,
                |view, value, cx| view.capture_edit(|d| d["title"] = json!(value.trim()), cx),
            ));
            let screen_weak = weak.clone();
            panel = panel.child(Field::new(
                t.t("capture.screen"),
                Select::new(
                    "capture-screen",
                    screens
                        .iter()
                        .map(|(i, l)| SelectOption::new(i.clone(), l.clone()))
                        .collect(),
                    Some(text("sourceScreenId")),
                )
                .on_change(move |screen, _, cx| {
                    screen_weak
                        .update(cx, |view, cx| {
                            view.capture_edit(|d| d["sourceScreenId"] = json!(screen), cx)
                        })
                        .ok();
                }),
            ));
            for (key, label, min, max) in [
                ("width", "capture.width", 16., 3840.),
                ("height", "capture.height", 16., 2160.),
                ("fps", "capture.fps", 1., 60.),
                ("bitrateKbps", "capture.bitrateKbps", 100., 100_000.),
            ] {
                panel = panel.child(self.number_field(
                    &format!("capture-{key}-{id}"),
                    t.t(label),
                    &num(key),
                    (min, max),
                    1.,
                    window,
                    cx,
                    move |view, value, cx| {
                        if let Some(n) = parse_number(&value, min, max) {
                            view.capture_edit(|d| d[key] = json!(n), cx);
                        }
                    },
                ));
            }
            let dest_weak = weak.clone();
            panel = panel.child(Field::new(
                t.t("capture.destination"),
                Select::new(
                    "capture-destination",
                    vec![
                        SelectOption::new("file".to_string(), t.t("capture.destinationFile")),
                        SelectOption::new("stream".to_string(), t.t("capture.destinationStream")),
                    ],
                    if streaming { "stream" } else { "file" }.to_string(),
                )
                .on_change(move |kind, _, cx| {
                    dest_weak
                        .update(cx, |view, cx| {
                            view.capture_edit(
                                |d| {
                                    if kind == "stream" {
                                        d["streamUrl"] = json!("rtmps://");
                                    } else if let Some(o) = d.as_object_mut() {
                                        o.remove("streamUrl");
                                    }
                                },
                                cx,
                            )
                        })
                        .ok();
                }),
            ));
            if streaming {
                panel = panel.child(self.text_field(
                    &format!("capture-url-{id}"),
                    t.t("capture.streamUrl"),
                    &text("streamUrl"),
                    window,
                    cx,
                    |view, value, cx| {
                        view.capture_edit(|d| d["streamUrl"] = json!(value.trim()), cx)
                    },
                ));
                let key = self
                    .capture
                    .key
                    .get_or_insert_with(|| password_state("", window, cx))
                    .clone();
                panel = panel.child(Field::new(
                    t.t("capture.streamKey"),
                    TextInput::new("capture-stream-key", &key),
                ));
                panel = panel.child(muted(t.t("capture.streamKeyKept"), cx));
            } else {
                let dir = text("outputDirectory");
                panel = panel.child(OutputField::new(
                    t.t("capture.directory"),
                    if dir.is_empty() {
                        t.t("capture.choose")
                    } else {
                        dir
                    },
                ));
                panel = panel.child(
                    ActionButton::new("capture-choose-directory", t.t("capture.choose"))
                        .variant(ButtonVariant::Secondary)
                        .icon("folder")
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.host_call(
                                "selectCaptureDirectory",
                                Vec::new(),
                                cx,
                                |view, result, cx| match result {
                                    Ok(path) => {
                                        if let Some(path) = path.as_str() {
                                            let path = path.to_string();
                                            view.capture_edit(
                                                |d| d["outputDirectory"] = json!(path),
                                                cx,
                                            );
                                        }
                                    }
                                    Err(error) => view.capture.error = Some(error),
                                },
                            );
                        })),
                );
            }
            // `CaptureAudioMapEditor`: the default records the screen mix as stereo; a custom map is a list of
            // recording channels, each summing screen channels 1 and 2.
            let custom = draft["audioMap"].is_array();
            let map_weak = weak.clone();
            panel = panel.child(
                CheckboxField::new(
                    "capture-audio-custom",
                    t.t("capture.audioMap.custom"),
                    custom,
                )
                .on_change(move |on, _, cx| {
                    map_weak
                        .update(cx, |view, cx| {
                            view.capture_edit(
                                |d| {
                                    if on {
                                        d["audioMap"] = json!([[1], [2]]);
                                    } else if let Some(o) = d.as_object_mut() {
                                        o.remove("audioMap");
                                    }
                                },
                                cx,
                            )
                        })
                        .ok();
                }),
            );
            if let Some(map) = draft["audioMap"].as_array() {
                for (channel, sources) in map.iter().enumerate() {
                    let mut row = action_row(cx).child(format!(
                        "{} {}",
                        t.t("capture.audioMap.recordingChannel"),
                        channel + 1
                    ));
                    for source in [1u64, 2] {
                        let on = sources
                            .as_array()
                            .is_some_and(|s| s.contains(&json!(source)));
                        row = row.child(
                            ActionButton::new(
                                SharedString::from(format!("capture-map-{channel}-{source}")),
                                format!("{} {source}", t.t("capture.audioMap.screenChannel")),
                            )
                            .variant(if on {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            })
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.capture_edit(
                                        |d| {
                                            d["audioMap"] =
                                                toggle_map_source(&d["audioMap"], channel, source)
                                        },
                                        cx,
                                    )
                                },
                            )),
                        );
                    }
                    panel = panel.child(row);
                }
            }
            let (save_key, start_id, delete_id) =
                (self.capture.key.clone(), id.clone(), id.clone());
            panel = panel.child(
                action_row(cx)
                    .child(
                        ActionButton::new("capture-save", t.t("capture.save"))
                            .disabled(self.capture.busy)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                let key = save_key
                                    .as_ref()
                                    .map(|k| k.read(cx).value().to_string())
                                    .unwrap_or_default();
                                view.capture_save(key, cx)
                            })),
                    )
                    .child(
                        ActionButton::new("capture-start", t.t("capture.start"))
                            .disabled(active || finalizing)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.capture_start(start_id.clone(), cx)
                            })),
                    )
                    .child(
                        ActionButton::new("capture-delete", t.t("common.delete"))
                            .variant(ButtonVariant::Danger)
                            .disabled(self.capture.busy)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.capture_delete(delete_id.clone(), cx)
                            })),
                    ),
            );
        }
        let state = if finalizing {
            t.t("capture.finalizing")
        } else if active {
            t.t("capture.recording")
        } else {
            t.t("capture.stopped")
        };
        panel = panel.child(div().id("capture-status").child(format!(
            "{state}  {} {}  {} {}  {} {}",
            t.t("capture.frames"),
            status["framesWritten"],
            t.t("capture.dropped"),
            status["droppedFrames"],
            status["bytesWritten"],
            t.t("capture.bytes"),
        )));
        if active {
            if self.capture.confirm_stop {
                panel = panel.child(div().child(t.t("capture.confirmStop"))).child(
                    action_row(cx)
                        .child(
                            ActionButton::new("capture-stop-confirm", t.t("capture.stop"))
                                .variant(ButtonVariant::Danger)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.capture.confirm_stop = false;
                                    view.capture_stop(cx)
                                })),
                        )
                        .child(
                            ActionButton::new("capture-stop-cancel", t.t("videoInputs.cancel"))
                                .variant(ButtonVariant::Secondary)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.capture.confirm_stop = false;
                                    cx.notify();
                                })),
                        ),
                );
            } else {
                panel = panel.child(
                    ActionButton::new("capture-stop", t.t("capture.stop"))
                        .variant(ButtonVariant::Danger)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.capture.confirm_stop = true;
                            cx.notify();
                        })),
                );
            }
        }
        panel
            .children(
                self.capture
                    .error
                    .clone()
                    .or_else(|| status["error"].as_str().map(str::to_string))
                    .map(|e| {
                        div()
                            .id("capture-error")
                            .text_color(theme.colors.warning)
                            .child(e)
                    }),
            )
            .into_any_element()
    }
}
