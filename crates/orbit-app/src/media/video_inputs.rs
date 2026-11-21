//! `VideoInputsPanel.tsx`, `VideoInputSourceFields.tsx`, `VideoInputCameraFields.tsx`, `video-input-controller.ts`,
//! `core/video-input.ts`, `shared/camera-source.ts`, and `shared/ndi-source.ts`.
//!
//! Device ownership: program output runs in the output process, which opens the camera or NDI receiver for an
//! input that is on air. The UI process therefore never opens a source that is live: a preview is shown only for
//! the input being edited, and only while it is not on air. A camera the UI opened is released by the host's idle
//! reaper within five seconds, so a take right after an edit can still find the device held for that long.

use std::sync::Arc;

use gpui::{AppContext, Context, Entity, RenderImage, Task};
use orbit_ui::controls::InputState;
use orbit_ui::frame::{bgra_image, FrameSize, FrameView};
use orbit_ui::Translator;
use serde_json::{json, Value};

use crate::collections::model::Snapshot;
use crate::state::runtime::Runtime;

/// `VIDEO_INPUT_PLAYLIST_ID`.
pub const VIDEO_INPUT_LIBRARY: &str = "orbit:video-inputs";

/// `cameraModes`.
pub const CAMERA_MODES: [(u32, u32, u32); 3] = [(1280, 720, 30), (1920, 1080, 30), (640, 480, 30)];

pub fn camera_mode_key(width: u64, height: u64, fps: u64) -> String {
    format!("{width}x{height}@{fps}")
}

fn camera_valid(source: &Value) -> bool {
    let integer = |value: &Value, min: u64, max: u64| {
        value
            .as_u64()
            .is_some_and(|number| (min..=max).contains(&number))
    };
    let Some(name) = source["deviceName"].as_str() else {
        return false;
    };
    !name.trim().is_empty()
        && name.chars().count() <= 256
        && !name.chars().any(|c| c.is_control())
        && integer(&source["width"], 16, 4096)
        && integer(&source["height"], 16, 4096)
        && source["width"].as_u64().unwrap_or(0) * source["height"].as_u64().unwrap_or(0)
            <= 8_294_400
        && integer(&source["fps"], 1, 120)
}

fn ndi_valid(source: &Value) -> bool {
    ["name", "urlAddress"].iter().all(|key| {
        source[key]
            .as_str()
            .is_some_and(|text| !text.trim().is_empty() && !text.contains('\0'))
    })
}

/// `validateVideoInput`: the error is a translation key, as in React.
pub fn validate_video_input(input: &Value) -> Result<(), String> {
    let text = |key: &str| input[key].as_str().unwrap_or("").trim().to_string();
    if text("id").is_empty() || text("title").is_empty() {
        return Err("videoInputs.nameRequired".into());
    }
    let (ndi, camera) = (
        !input["ndiSource"].is_null(),
        !input["cameraSource"].is_null(),
    );
    if ndi == camera {
        return Err("videoInputs.sourceRequired".into());
    }
    if ndi && !ndi_valid(&input["ndiSource"]) {
        return Err(
            "An NDI source name and address are required and cannot contain NUL bytes.".into(),
        );
    }
    if camera && !camera_valid(&input["cameraSource"]) {
        return Err(
            "A camera device name and a supported width, height and frame rate are required."
                .into(),
        );
    }
    if !matches!(input["scale"].as_str(), Some("fit" | "fill" | "stretch")) {
        return Err("videoInputs.scaleRequired".into());
    }
    Ok(())
}

/// `videoInputSelection`: the id that may go on air, or the reason it may not.
pub fn selection(inputs: &[Value], id: &str) -> Result<String, String> {
    let input = inputs
        .iter()
        .find(|input| input["id"] == id)
        .ok_or("videoInputs.missing")?;
    validate_video_input(input)?;
    Ok(id.to_string())
}

/// `ndiSourceOptions`: a saved source that discovery did not find stays selectable.
pub fn ndi_source_options(current: Option<&Value>, found: &[Value]) -> Vec<Value> {
    match current.filter(|current| {
        !found.iter().any(|source| {
            source["name"] == current["name"] && source["urlAddress"] == current["urlAddress"]
        })
    }) {
        Some(current) => std::iter::once(current.clone())
            .chain(found.iter().cloned())
            .collect(),
        None => found.to_vec(),
    }
}

/// `cameraDeviceOptions`.
pub fn camera_device_options(current: Option<&str>, devices: &[String]) -> Vec<String> {
    match current.filter(|current| !devices.iter().any(|device| device == current)) {
        Some(current) => std::iter::once(current.to_string())
            .chain(devices.iter().cloned())
            .collect(),
        None => devices.to_vec(),
    }
}

/// Whether the UI process may open this input's device for a preview.
pub fn preview_allowed(input_id: &str, draft_id: Option<&str>, live_id: Option<&str>) -> bool {
    draft_id == Some(input_id) && live_id != Some(input_id)
}

/// The inputs after saving one: creating an existing id or editing a missing one is `videoInputs.missing`.
pub fn saved_inputs(inputs: &[Value], input: &Value, create: bool) -> Result<Vec<Value>, String> {
    let exists = inputs
        .iter()
        .any(|candidate| candidate["id"] == input["id"]);
    if create == exists {
        return Err("videoInputs.missing".into());
    }
    Ok(if create {
        inputs.iter().cloned().chain([input.clone()]).collect()
    } else {
        inputs
            .iter()
            .map(|candidate| {
                if candidate["id"] == input["id"] {
                    input.clone()
                } else {
                    candidate.clone()
                }
            })
            .collect()
    })
}

pub fn deleted_inputs(inputs: &[Value], id: &str) -> Result<Vec<Value>, String> {
    if !inputs.iter().any(|input| input["id"] == id) {
        return Err("videoInputs.missing".into());
    }
    Ok(inputs
        .iter()
        .filter(|input| input["id"] != id)
        .cloned()
        .collect())
}

/// A frame packet of the host (`PORF` header, RGBA8) as a GPUI bitmap.
pub fn frame_from_packet(packet: &[u8]) -> Result<(FrameSize, Arc<RenderImage>), String> {
    if packet.len() < 24 || &packet[..4] != b"PORF" {
        return Err("The preview frame is not a raw frame packet.".into());
    }
    let word = |at: usize| u32::from_le_bytes(packet[at..at + 4].try_into().expect("four bytes"));
    let (width, height) = (word(8), word(12));
    let mut pixels = packet[24..].to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let size = FrameSize { width, height };
    bgra_image(size, pixels).map(|image| (size, image))
}

pub(super) struct Preview {
    pub(super) input: String,
    pub(super) frames: Entity<FrameView>,
    pub(super) waiting: bool,
    pub(super) error: Option<String>,
    _task: Task<()>,
}

impl Preview {
    pub(super) fn new(input: String, frames: Entity<FrameView>, task: Task<()>) -> Self {
        Self {
            input,
            frames,
            waiting: true,
            error: None,
            _task: task,
        }
    }
}

pub struct VideoInputs {
    pub(super) runtime: Runtime,
    pub(super) draft: Option<Value>,
    pub(super) creating: bool,
    pub(super) kind: &'static str,
    pub(super) sources: Vec<Value>,
    pub(super) cameras: Vec<String>,
    pub(super) loading: bool,
    pub(super) busy: bool,
    /// Edits in flight; the panel is busy until the last one answered.
    pending: usize,
    pending_inputs: Vec<(u64, String, Vec<Value>)>,
    save_request: u64,
    pub(super) error: String,
    /// Ids already sent to save: the workspace lags behind the write.
    pub(super) saved: std::collections::HashSet<String>,
    /// Rendered as the Inputs settings tab instead of the media bin panel.
    pub(super) settings_mode: bool,
    refresh: u64,
    pub(super) name: Option<(String, Entity<InputState>)>,
    pub(super) name_subscription: Option<gpui::Subscription>,
    pub(super) preview: Option<Preview>,
    pub(super) preview_source: Option<Value>,
    _observers: Vec<gpui::Subscription>,
    discovered: bool,
}

impl VideoInputs {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let observers = vec![
            cx.observe(&runtime.content, |_, _, cx| cx.notify()),
            cx.observe(&runtime.output, |this, _, cx| {
                this.sync_preview(cx);
                cx.notify();
            }),
        ];

        Self {
            runtime,
            draft: None,
            creating: false,
            kind: "ndi",
            sources: Vec::new(),
            cameras: Vec::new(),
            loading: false,
            busy: false,
            pending: 0,
            pending_inputs: Vec::new(),
            save_request: 0,
            error: String::new(),
            saved: Default::default(),
            settings_mode: false,
            refresh: 0,
            name: None,
            name_subscription: None,
            preview: None,
            preview_source: None,
            _observers: observers,
            discovered: false,
        }
    }

    /// The Inputs settings tab: the same state and device discovery, rendered as rows that save at once.
    pub fn for_settings(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let mut view = Self::new(runtime, cx);
        view.settings_mode = true;
        view
    }

    /// Called when the panel is shown: the first time it lists the NDI senders and cameras. Listing touches the
    /// network and the capture stack, so it never happens while the panel is hidden.
    pub fn show(&mut self, cx: &mut Context<Self>) {
        if !self.discovered {
            self.discovered = true;
            self.discover(cx);
        }
    }

    pub(super) fn inputs(&self, cx: &gpui::App) -> Vec<Value> {
        let content = self.runtime.content.read(cx);
        if let Some((_, _, inputs)) = self
            .pending_inputs
            .iter()
            .rev()
            .find(|(_, path, _)| path == &content.workspace_path)
        {
            return inputs.clone();
        }
        content.workspace["videoInputs"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn live_id(&self, cx: &gpui::App) -> Option<String> {
        self.runtime.output.read(cx).live["liveVideoInputId"]
            .as_str()
            .map(str::to_string)
    }

    /// `discoverNdiSources` and `discoverCameraDevices` on a worker; a newer request retires an older answer.
    pub fn discover(&mut self, cx: &mut Context<Self>) {
        self.refresh += 1;
        let request = self.refresh;
        self.loading = true;
        let host = self.runtime.host.clone();
        cx.spawn(async move |view, cx| {
            let (sources, cameras) = cx
                .background_spawn(async move {
                    (
                        host.invoke("listNdiSources", Vec::new()),
                        host.invoke("listCameraDevices", Vec::new()),
                    )
                })
                .await;
            view.update(cx, |view, cx| {
                if view.refresh != request {
                    return;
                }
                view.loading = false;
                view.error.clear();
                match sources {
                    Ok(list) => view.sources = list.as_array().cloned().unwrap_or_default(),
                    Err(error) => view.error = error,
                }
                match cameras {
                    Ok(list) => {
                        view.cameras = list
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|name| name.as_str().map(str::to_string))
                            .collect()
                    }
                    Err(error) => view.error = error,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn trigger(&mut self, id: &str, cx: &mut Context<Self>) {
        match selection(&self.inputs(cx), id) {
            Ok(id) => {
                self.error.clear();
                self.runtime.output.update(cx, |output, cx| {
                    output.edit_live(|live| live["liveVideoInputId"] = json!(id));
                    cx.notify();
                });
            }
            Err(error) => self.error = error,
        }
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.runtime.output.update(cx, |output, cx| {
            output.edit_live(|live| live["liveVideoInputId"] = Value::Null);
            cx.notify();
        });
    }

    /// `mutateInputs`: reads the latest snapshot, writes the changed list with its revision, and adopts the
    /// canonical answer unless the workspace changed meanwhile.
    pub(super) fn mutate(
        &mut self,
        change: impl FnOnce(&[Value]) -> Result<Vec<Value>, String> + Send + 'static,
        close: bool,
        cx: &mut Context<Self>,
    ) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        if path.is_empty() {
            self.error = "videoInputs.workspaceRequired".into();
            cx.notify();
            return;
        }
        let before = self.inputs(cx);
        let next = match change(&before) {
            Ok(next) => next,
            Err(error) => {
                self.error = error;
                cx.notify();
                return;
            }
        };
        self.save_request += 1;
        let request = self.save_request;
        self.pending_inputs
            .push((request, path.clone(), next.clone()));
        let draft = self.draft.clone();
        self.pending += 1;
        self.busy = true;
        self.error.clear();
        let (host, scope) = (self.runtime.host.clone(), path.clone());
        let answer = self.runtime.lanes.workspace(&path, move |_| {
            let snapshot = host.invoke("readWorkspaceSnapshot", vec![json!(scope)])?;
            let current = snapshot["workspace"]["videoInputs"]
                .as_array()
                .ok_or("videoInputs.missing")?
                .clone();
            let next = super::model::merge_media(&before, &next, &current)?;
            let result = host.invoke(
                "saveWorkspaceContent",
                vec![
                    json!(scope),
                    json!({"videoInputs": next}),
                    snapshot["revision"].clone(),
                ],
            )?;
            if !result["workspace"]["videoInputs"].is_array() {
                return Err("videoInputs.missing".into());
            }
            // The save answers with the saved sections only; the canonical workspace comes from a fresh read.
            let fresh = host.invoke("readWorkspaceSnapshot", vec![json!(scope)])?;
            Ok(json!({"workspace": fresh["workspace"], "revision": result["revision"]}))
        });
        let runtime = self.runtime.clone();
        cx.spawn(async move |view, cx| {
            let result = answer
                .await
                .map_err(|_| "The workspace queue stopped.".to_string())
                .and_then(|result| result);
            let parsed = match result {
                Ok(done) => {
                    cx.background_spawn(async move { Snapshot::parse(&done["workspace"]) })
                        .await
                }
                Err(error) => Err(error),
            };
            view.update(cx, |view, cx| {
                view.pending -= 1;
                view.pending_inputs.retain(|(id, _, _)| *id != request);
                view.busy = view.pending > 0;
                if runtime.content.read(cx).workspace_path != path {
                    cx.notify();
                    return;
                }
                match parsed {
                    Ok(snapshot) => {
                        let ids: Vec<String> = snapshot.raw["videoInputs"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|input| input["id"].as_str().map(str::to_string))
                            .collect();
                        runtime.content.update(cx, |content, cx| {
                            content.apply_canonical(snapshot);
                            cx.notify();
                        });
                        // A deleted input leaves the air; everything else on air stays.
                        runtime.output.update(cx, |output, cx| {
                            let live = output.live["liveVideoInputId"].as_str().map(str::to_string);
                            if live.is_some_and(|live| !ids.contains(&live)) {
                                output.edit_live(|live| live["liveVideoInputId"] = Value::Null);
                            }
                            cx.notify();
                        });
                        if view.draft.as_ref().is_some_and(|draft| {
                            draft["id"]
                                .as_str()
                                .is_some_and(|id| ids.iter().any(|saved| saved == id))
                        }) {
                            view.creating = false;
                        }
                        if close && view.draft == draft {
                            view.draft = None;
                            view.name = None;
                        }
                    }
                    Err(error) => view.error = error,
                }
                view.sync_preview(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        let Some(mut input) = self.draft.clone() else {
            return;
        };
        if let Some(title) = input["title"].as_str() {
            input["title"] = json!(title.trim());
        }
        if let Err(error) = validate_video_input(&input) {
            self.error = error;
            cx.notify();
            return;
        }
        let create = self.creating;
        self.mutate(
            move |current| saved_inputs(current, &input, create),
            true,
            cx,
        );
    }

    pub fn delete(&mut self, id: String, cx: &mut Context<Self>) {
        self.mutate(move |current| deleted_inputs(current, &id), true, cx);
    }

    pub fn begin_new(&mut self, cx: &mut Context<Self>) {
        let t = *cx.global::<Translator>();
        self.creating = true;
        self.kind = "ndi";
        self.draft = Some(json!({
            "id": format!("video-input-{}", uuid::Uuid::new_v4()),
            "title": t.t("videoInputs.new"),
            "scale": "fit",
        }));
        self.name = None;
        self.sync_preview(cx);
        cx.notify();
    }

    pub fn begin_edit(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(input) = self.inputs(cx).into_iter().find(|input| input["id"] == id) else {
            return;
        };
        self.creating = false;
        self.kind = if input["cameraSource"].is_null() {
            "ndi"
        } else {
            "camera"
        };
        self.draft = Some(input);
        self.name = None;
        self.sync_preview(cx);
        cx.notify();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.draft = None;
        self.name = None;
        self.sync_preview(cx);
        cx.notify();
    }

    pub(super) fn edit_draft(&mut self, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        if let Some(draft) = self.draft.as_mut() {
            change(draft);
            self.sync_preview(cx);
            cx.notify();
        }
    }

    pub fn set_kind(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        self.kind = kind;
        self.edit_draft(
            |draft| {
                if let Some(object) = draft.as_object_mut() {
                    object.remove("ndiSource");
                    object.remove("cameraSource");
                }
            },
            cx,
        );
    }

    /// Starts or stops the preview so that it follows the draft and never competes with program output.
    pub(super) fn sync_preview(&mut self, cx: &mut Context<Self>) {
        let live = self.live_id(cx);
        let wanted = self.draft.as_ref().and_then(|draft| {
            let id = draft["id"].as_str()?;
            let has_source = !draft["ndiSource"].is_null() || !draft["cameraSource"].is_null();
            (has_source && preview_allowed(id, Some(id), live.as_deref()) && validate_source(draft))
                .then(|| draft.clone())
        });
        match wanted {
            None => {
                self.preview = None;
                self.preview_source = None;
            }
            Some(draft) => {
                let same = self.preview.as_ref().is_some_and(|preview| {
                    preview.input == draft["id"].as_str().unwrap_or("")
                        && self.preview_source.as_ref() == Some(&source_of(&draft))
                });
                if !same {
                    self.start_preview(draft, cx);
                }
            }
        }
    }
}

pub(super) fn validate_source(draft: &Value) -> bool {
    if draft["cameraSource"].is_null() {
        ndi_valid(&draft["ndiSource"])
    } else {
        camera_valid(&draft["cameraSource"])
    }
}

pub(super) fn source_of(draft: &Value) -> Value {
    if draft["cameraSource"].is_null() {
        draft["ndiSource"].clone()
    } else {
        draft["cameraSource"].clone()
    }
}
