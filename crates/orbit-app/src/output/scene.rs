//! Collect authoring state for the native scene builder; never build per-screen scenes here.

use std::sync::{Arc, Mutex};

use gpui::{App, AppContext};
use serde_json::{json, Map, Value};

use super::protocol::{self, Protocol, Snapshot};
use crate::collections::model::{Items, Library, Playlist, ThemeAssignment};
use crate::state::runtime::Runtime;

type SyncKey = (usize, u64, u64, String, String, Option<usize>);

pub struct Output {
    pub settings: Arc<Value>,
    pub(crate) settings_drafts: crate::settings::drafts::Drafts,
    pub live: Arc<Value>,
    pub audience_look: Option<super::looks::LiveLook>,
    pub snapshot: Option<Arc<Value>>,
    pub selected_screen: String,
    pub committed_revision: u64,
    pub media_preparation_epoch: u64,
    pub error: Option<String>,
    pub link_error: Option<String>,
    pub displays: Arc<Value>,
    pub device_error: Option<String>,
    pub display_request: u64,
    displays_loaded: bool,
    routing_started: bool,
    /// The launch policy for screen activation has run in this process.
    startup_applied: bool,
    generation: u64,
    busy: bool,
    key: Option<SyncKey>,
    protocol: Arc<Mutex<Protocol>>,
    /// The protocol toward the output process. A new link connection gets a new one, so its first send is a full.
    remote: Arc<Mutex<Protocol>>,
    native_patches: NativePatches,
    workspace_path: String,
    workspace_load: u64,
}

type NativePatches = Arc<Mutex<Vec<(u64, Map<String, Value>)>>>;

impl Default for Output {
    fn default() -> Self {
        // The cleared runtime state of `initial-state.ts`, independent of the selected authoring document.
        Self {
            settings: Arc::new(Value::Null),
            settings_drafts: Default::default(),
            audience_look: None,
            live: Arc::new(json!({
                "outputFrozen": false,
                "liveItemId": null, "liveSlideId": null, "liveItem": null, "liveSlide": null,
                "livePlaylistId": null, "livePlaylistEntryId": null, "liveCueInstanceId": null,
                "presentationNextAnchor": null,
                "liveBuildStep": 0, "liveBuildStepStartedAtMsByStep": {}, "timelineStartedAtMs": null,
                "timelineTransport": null, "livePropIds": [], "liveVideoInputId": null,
                "announcements": null, "activeActionSlide": null, "activeAudienceLookId": null,
                "backgroundId": null, "backgroundScale": "fit", "backgroundBehavior": "foreground",
                "backgroundPlayback": "stop", "backgroundEndBehavior": "hold", "backgroundTransition": "none",
                "backgroundSelectionId": null, "backgroundPlaybackState": null,
                "backgroundEffectsOverride": null, "backgroundTransportOverride": null,
                "audioId": null, "audioSelectionId": 0, "selectedAudio": null,
                "activeMessageId": null, "activeMessageInstanceId": null, "activeMessageText": "",
                "activeMessageDefinition": null, "stageMessage": { "text": "", "state": "cleared" },
                "countdownTimerRuns": {}, "transportPlaybackPaused": { "media": false, "music": false },
            })),
            snapshot: None,
            selected_screen: String::new(),
            committed_revision: 0,
            media_preparation_epoch: 0,
            error: None,
            link_error: None,
            displays: Arc::new(json!([])),
            device_error: None,
            display_request: 0,
            displays_loaded: false,
            routing_started: false,
            startup_applied: false,
            generation: 0,
            busy: false,
            key: None,
            protocol: Arc::new(Mutex::new(Protocol::default())),
            remote: Arc::new(Mutex::new(Protocol::default())),
            native_patches: Arc::default(),
            workspace_path: String::new(),
            workspace_load: 0,
        }
    }
}

/// An answer of the output process that puts a slide on air while this process has none: the interface
/// restarted and the output kept its slide.
pub fn is_takeover(live: &Value, patch: &Map<String, Value>) -> bool {
    live["liveSlideId"].is_null() && patch.get("liveSlideId").is_some_and(|id| !id.is_null())
}

impl Output {
    pub fn invalidate_scene(&mut self) {
        self.generation += 1;
    }

    pub fn set_displays(&mut self, displays: Value) {
        self.displays = Arc::new(displays);
        self.displays_loaded = true;
        self.generation += 1;
    }
    /// The output process connected, disconnected, or restarted: the next sync resends everything to it.
    pub fn link_changed(&mut self) {
        self.media_preparation_epoch += 1;
        self.remote = Arc::new(Mutex::new(Protocol::default()));
        self.generation += 1;
    }

    pub fn set_settings(&mut self, settings: Value) {
        self.settings = Arc::new(settings);
        self.generation += 1;
    }

    /// The cue editor and other feature controllers edit the live slice through this boundary.
    pub fn edit_live(&mut self, change: impl FnOnce(&mut Value)) {
        change(Arc::make_mut(&mut self.live));
        self.generation += 1;
    }

    pub fn accept_native_patch(&mut self, payload: &Value) -> Result<(), String> {
        let revision = payload["revision"]
            .as_u64()
            .filter(|revision| *revision > 0)
            .ok_or("A native output patch has no positive revision.")?;
        let patch = payload["patch"]
            .as_object()
            .ok_or("A native output patch has no patch object.")?;
        if revision < self.committed_revision {
            return Ok(());
        }
        {
            let mut pending = self
                .native_patches
                .lock()
                .map_err(|_| "The native output patch queue is unavailable.")?;
            if let Some((queued_revision, queued)) = pending.first_mut() {
                let mut value = Value::Object(std::mem::take(queued));
                protocol::merge_patch(&mut value, patch);
                *queued = value
                    .as_object()
                    .expect("merged output patch is an object")
                    .clone();
                *queued_revision = revision;
            } else {
                pending.push((revision, patch.clone()));
            }
        }
        protocol::merge_patch(Arc::make_mut(&mut self.live), patch);
        if patch.get("liveSlideId").is_some_and(|id| !id.is_null()) {
            Arc::make_mut(&mut self.live)["presentationNextAnchor"] = Value::Null;
        }
        if let Some(id) = payload["propAutoClear"]["id"].as_str() {
            let live = Arc::make_mut(&mut self.live);
            if !live["remotePropDeadlines"].is_object() {
                live["remotePropDeadlines"] = json!({});
            }
            live["remotePropDeadlines"][id] = payload["propAutoClear"]["deadline"].clone();
        }
        self.committed_revision = revision;
        self.generation += 1;
        Ok(())
    }
}

struct SceneInput {
    workspace: Arc<Value>,
    settings: Arc<Value>,
    live: Arc<Value>,
    displays: Arc<Value>,
    items: Items,
    libraries: Vec<Library>,
    playlists: Vec<Playlist>,
    focused: String,
    focused_playlist: String,
    focused_index: Option<usize>,
    screen_config_draft: Map<String, Value>,
}

fn assignment(value: &mut Value, theme: &ThemeAssignment) {
    match theme {
        ThemeAssignment::Inherit => {
            value
                .as_object_mut()
                .expect("collection is an object")
                .remove("presentationThemeId");
        }
        ThemeAssignment::NoTheme => value["presentationThemeId"] = Value::Null,
        ThemeAssignment::Theme(id) => value["presentationThemeId"] = json!(id),
    }
}

impl SceneInput {
    fn snapshot(self) -> Result<Snapshot, String> {
        let themes = if self.live.get("themes").is_some() {
            self.live.clone()
        } else {
            self.workspace.clone()
        };
        let mut scene: Map<String, Value> = self
            .live
            .as_object()
            .ok_or("The live output state is not an object.")?
            .iter()
            .filter(|(key, _)| key.as_str() != "themes")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        for key in [
            "props",
            "masks",
            "ccliTemplate",
            "stageLayouts",
            "stageScreenAssignments",
            "media",
            "mediaLibraries",
            "backgrounds",
            "countdownTimers",
            "videoInputs",
            "audioInputs",
            "slideTransition",
            "layerTransitions",
        ] {
            if !scene.contains_key(key) {
                if let Some(value) = self.workspace.get(key) {
                    scene.insert(key.into(), value.clone());
                }
            }
        }
        scene.insert(
            "playbackWorkspacePath".into(),
            self.workspace["workspacePath"].clone(),
        );
        let mut selected_audio = scene
            .get("audioId")
            .and_then(Value::as_str)
            .and_then(|id| {
                scene
                    .get("media")
                    .and_then(Value::as_array)
                    .and_then(|media| {
                        media
                            .iter()
                            .find(|asset| asset["id"] == id && asset["type"] == "audio")
                    })
            })
            .cloned()
            .unwrap_or(Value::Null);
        if self.live["selectedAudio"]["id"] == scene["audioId"]
            && self.live["selectedAudio"]["selectionId"] == scene["audioSelectionId"]
            && selected_audio.is_object()
        {
            if let Some(playlist) = self.live["selectedAudio"].get("playlistId") {
                selected_audio["playlistId"] = playlist.clone();
            }
        }
        scene.insert("selectedAudio".into(), selected_audio);
        // The host reads entry titles and arrangement names from `library` (the "Playlist Item" linked text
        // with its arrangement). React sends only id and title, so there the name never reaches the host.
        scene.insert(
            "library".into(),
            Value::Array(
                self.items
                    .iter()
                    .map(|item| {
                        let arrangements: Vec<Value> = item.value["arrangements"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|arrangement| {
                                json!({ "id": arrangement["id"], "name": arrangement["name"] })
                            })
                            .collect();
                        json!({
                            "id": item.id,
                            "title": item.title,
                            "selectedArrangementId": item.value["selectedArrangementId"],
                            "arrangements": arrangements,
                        })
                    })
                    .collect(),
            ),
        );
        let originals = |key: &str, id: &str| {
            self.workspace[key]
                .as_array()
                .and_then(|values| values.iter().find(|value| value["id"] == id))
                .cloned()
                .unwrap_or_else(|| json!({ "id": id }))
        };
        scene.insert(
            "libraries".into(),
            Value::Array(
                self.libraries
                    .iter()
                    .map(|library| {
                        let mut value = originals("libraries", &library.id);
                        value["title"] = json!(library.title);
                        value["itemIds"] = json!(library.item_ids);
                        assignment(&mut value, &library.theme);
                        value
                    })
                    .collect(),
            ),
        );
        scene.insert(
            "playlist".into(),
            Value::Array(
                self.playlists
                    .iter()
                    .map(|playlist| {
                        let mut value = originals("playlist", &playlist.id);
                        value["title"] = json!(playlist.title);
                        value["parentId"] = json!(playlist.parent_id);
                        value["type"] = json!(playlist.kind);
                        value["collapsed"] = json!(playlist.collapsed);
                        value["items"] = Value::Array(
                            playlist
                                .entries
                                .iter()
                                .map(|entry| entry.raw.clone())
                                .collect(),
                        );
                        assignment(&mut value, &playlist.theme);
                        value
                    })
                    .collect(),
            ),
        );
        if !scene.contains_key("screenConfig") {
            let configured = self.settings["screenConfig"]
                .as_object()
                .ok_or("Output settings have no screen configuration.")?
                .clone();
            let local = self.workspace["screenConfig"]
                .as_object()
                .ok_or("The workspace has no screen configuration.")?;
            // `mergeImportedScreenConfig`: a non-empty workspace map replaces the application map.
            let mut config = if local.is_empty() {
                configured
            } else {
                local.clone()
            };
            for screen in config.values_mut() {
                if let Some(screen) = screen.as_object_mut() {
                    screen.remove("settingsExpanded");
                    screen.remove("settingsTab");
                }
            }
            scene.insert("screenConfig".into(), Value::Object(config));
        }
        let screens = scene
            .get_mut("screenConfig")
            .and_then(Value::as_object_mut)
            .ok_or("Output screen configuration is not an object.")?;
        for (id, config) in self.screen_config_draft {
            if config.is_null() {
                screens.remove(&id);
            } else {
                screens.insert(id, config);
            }
        }
        let sizes = scene["screenConfig"]
            .as_object()
            .ok_or("Output screen configuration is not an object.")?
            .iter()
            .map(|(id, config)| {
                let size = super::view::screen_size_for_displays(config, &self.displays);
                (
                    id.clone(),
                    json!({ "width": size.width, "height": size.height }),
                )
            })
            .collect::<Map<_, _>>();
        scene.insert("outputCanvasSizes".into(), Value::Object(sizes));
        scene.insert(
            "audioSettings".into(),
            self.settings
                .get("audioSettings")
                .ok_or("Output settings have no audio settings.")?
                .clone(),
        );
        let groups = self.workspace["groupSettings"]
            .as_object()
            .ok_or("The workspace has no group settings.")?;
        // `mergeImportedSettings` uses the application settings only for an empty workspace map.
        scene.insert(
            "groupSettings".into(),
            if groups.is_empty() {
                self.settings
                    .get("groupSettings")
                    .ok_or("Output settings have no group settings.")?
                    .clone()
            } else {
                Value::Object(groups.clone())
            },
        );
        for key in ["logoImagePath", "logoImageUrl", "ignoreBackgroundColors"] {
            scene.insert(
                key.into(),
                self.settings["generalSettings"]
                    .get(key)
                    .ok_or_else(|| format!("Output settings have no {key}."))?
                    .clone(),
            );
        }
        let playlist = self
            .playlists
            .iter()
            .find(|playlist| playlist.id == self.focused_playlist);
        let entry = playlist
            .and_then(|playlist| {
                self.focused_index
                    .and_then(|index| playlist.entries.get(index))
            })
            .filter(|entry| entry.item_id == self.focused);
        scene.insert(
            "focusedPlaylistId".into(),
            if self.focused_playlist.is_empty() {
                Value::Null
            } else {
                json!(self.focused_playlist)
            },
        );
        scene.insert("focusedPresentation".into(), if self.items.contains(&self.focused) {
            json!({ "id": self.focused, "playlistId": playlist.map(|playlist| &playlist.id),
                "playlistEntryId": entry.map(|entry| &entry.id), "arrangementId": entry.map(|entry| &entry.arrangement_id) })
        } else { Value::Null });
        // A program snapshot is pinned only while both of its IDs still match the live pointers.
        let pinned = scene
            .get("liveItemId")
            .is_some_and(|id| scene["liveItem"].get("id") == Some(id))
            && scene
                .get("liveSlideId")
                .is_some_and(|id| scene["liveSlide"].get("id") == Some(id));
        if !pinned {
            let item = scene
                .get("liveItemId")
                .and_then(Value::as_str)
                .and_then(|id| self.items.get(id));
            scene.insert(
                "liveItem".into(),
                item.map_or(Value::Null, |item| (*item.value).clone()),
            );
            let slide = item
                .and_then(|item| item.value["slides"].as_array())
                .and_then(|slides| {
                    slides
                        .iter()
                        .find(|slide| scene.get("liveSlideId") == slide.get("id"))
                })
                .cloned();
            scene.insert("liveSlide".into(), slide.unwrap_or(Value::Null));
        }
        Ok(Snapshot {
            state: Arc::new(Value::Object(scene)),
            theme_source: themes,
        })
    }
}

/// Applies the launch policy of screen activation once the first workspace has loaded.
fn apply_launch_policy(runtime: &Runtime, cx: &mut App) {
    let content = runtime.content.read(cx);
    let output = runtime.output.read(cx);
    if output.startup_applied
        || content.workspace_path.is_empty()
        || !output.settings.is_object()
        || output.busy
    {
        return;
    }
    let disabled = super::windows::launch_starts_disabled(&output.settings);
    runtime
        .output
        .update(cx, |output, _| output.startup_applied = true);
    if disabled {
        super::windows::start_disabled(runtime, cx);
    }
}

pub fn toggle_freeze(runtime: &Runtime, cx: &mut App) {
    runtime.output.update(cx, |output, cx| {
        output.edit_live(|live| live["outputFrozen"] = json!(live["outputFrozen"] != true));
        cx.notify();
    });
}

pub fn sync(runtime: &Runtime, cx: &mut App) {
    let content = runtime.content.read(cx);
    let ui = runtime.ui.read(cx);
    let output = runtime.output.read(cx);
    if content.workspace_path.is_empty() || !output.settings.is_object() || output.busy {
        return;
    }
    if output.workspace_path != content.workspace_path
        || output.workspace_load != content.load_generation
    {
        let path = content.workspace_path.clone();
        let load = content.load_generation;
        runtime.text_files.update(cx, |files, cx| {
            *files = Default::default();
            cx.notify();
        });
        runtime.output.update(cx, |output, _| {
            let settings = output.settings.clone();
            let displays = output.displays.clone();
            let displays_loaded = output.displays_loaded;
            let display_request = output.display_request;
            let device_error = output.device_error.clone();
            let link_error = output.link_error.clone();
            let startup_applied = output.startup_applied;
            *output = Output::default();
            output.settings = settings;
            output.displays = displays;
            output.displays_loaded = displays_loaded;
            output.display_request = display_request;
            output.device_error = device_error;
            output.link_error = link_error;
            output.startup_applied = startup_applied;
            output.workspace_path = path;
            output.workspace_load = load;
        });
        return sync(runtime, cx);
    }
    if !output.startup_applied {
        apply_launch_policy(runtime, cx);
        return sync(runtime, cx);
    }
    let index = ui.selected_documents.first().and_then(|entry| entry.index);
    let key = (
        Arc::as_ptr(&content.workspace) as usize,
        content.items_generation,
        output.generation,
        ui.selected_library_item_id.clone(),
        ui.selected_playlist_id.clone(),
        index,
    );
    if output.key.as_ref() == Some(&key) {
        return;
    }
    let input = SceneInput {
        workspace: content.workspace.clone(),
        settings: output.settings.clone(),
        live: output.live.clone(),
        displays: output.displays.clone(),
        items: content.items.clone(),
        libraries: content.libraries.clone(),
        playlists: content.playlists.clone(),
        focused: ui.selected_library_item_id.clone(),
        focused_playlist: ui.selected_playlist_id.clone(),
        focused_index: index,
        screen_config_draft: output
            .settings_drafts
            .section_patch(&content.workspace_path, "screenConfig"),
    };
    let link = runtime
        .host
        .try_state::<crate::output_process::OutputRoute>()
        .map(|route| route.0.get().cloned());
    let (protocol, remote, patches, captured_live, workspace) = (
        output.protocol.clone(),
        output.remote.clone(),
        output.native_patches.clone(),
        output.live.clone(),
        content.workspace_path.clone(),
    );
    // With an output process, routing follows the scene there; only an in-process host is told here.
    let restore_routing = link.is_none() && !output.routing_started && output.displays_loaded;
    runtime.output.update(cx, |output, _| {
        output.busy = true;
        output.key = Some(key);
    });
    let files = runtime.text_files.read(cx).entries.clone();
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        let host = runtime.host.clone();
        let answer = cx
            .background_spawn(async move {
                let mut snapshot = input.snapshot()?;
                crate::state::text_files::overlay(Arc::make_mut(&mut snapshot.state), &files);
                if files.as_array().is_some_and(|entries| !entries.is_empty()) {
                    if let Some(themes) = snapshot.theme_source.get("themes") {
                        let mut themes = themes.clone();
                        crate::state::text_files::overlay(&mut themes, &files);
                        snapshot.theme_source = Arc::new(json!({"themes":themes}));
                    }
                }
                let mut protocol = protocol
                    .lock()
                    .map_err(|_| "The output scene protocol is unavailable.")?;
                let pending = std::mem::take(
                    &mut *patches
                        .lock()
                        .map_err(|_| "The output patch queue is unavailable.")?,
                );
                for (revision, patch) in pending {
                    protocol.accept_native_patch(revision, &patch);
                }
                // The output process takes the scene first. Its answer carries what is on air there, and the
                // local host, which feeds the previews and the model, then follows it.
                let (snapshot, mut adopted, link_error) = match &link {
                    Some(Some(link)) => {
                        let mut remote = remote
                            .lock()
                            .map_err(|_| "The output link protocol is unavailable.")?;
                        match remote.send(&**link, snapshot.clone()) {
                            Ok(protocol::Delivery::Accepted(accepted)) => {
                                (accepted.snapshot, accepted.native_patch, None)
                            }
                            Ok(protocol::Delivery::Contended(patch)) => return Ok((None, patch)),
                            Err(error) => {
                                orbit_host::record_diagnostic(
                                    &host,
                                    "output-scene-sync-failed",
                                    orbit_host::diagnostics::LogLevel::Error,
                                    "output",
                                    Some(&error),
                                );
                                (snapshot, Map::new(), Some(error))
                            }
                        }
                    }
                    _ => (snapshot, Map::new(), None),
                };
                let delivery = protocol.send(&host, snapshot).inspect_err(|error| {
                    orbit_host::record_diagnostic(
                        &host,
                        "output-scene-sync-failed",
                        orbit_host::diagnostics::LogLevel::Error,
                        "output",
                        Some(error),
                    );
                })?;
                let mut accepted = match delivery {
                    protocol::Delivery::Accepted(accepted) => accepted,
                    protocol::Delivery::Contended(patch) => {
                        adopted.extend(patch);
                        return Ok((None, adopted));
                    }
                };
                adopted.extend(std::mem::take(&mut accepted.native_patch));
                if restore_routing {
                    host.invoke(
                        "setDisplayOutputVisibility",
                        vec![
                            json!([]),
                            json!(true),
                            accepted.snapshot.state["screenConfig"].clone(),
                        ],
                    )?;
                }
                let snapshot = Arc::new(accepted.snapshot.materialize());
                Ok::<_, String>((Some((accepted, snapshot, link_error)), adopted))
            })
            .await;
        cx.update(|cx| {
            let current = runtime.content.read(cx).workspace_path == workspace;
            let mut taken_over = false;
            runtime.output.update(cx, |output, cx| {
                output.busy = false;
                if current {
                    match answer {
                        Ok((accepted, native_patch)) => {
                            if let Some((accepted, snapshot, link_error)) = accepted {
                                output.routing_started |= restore_routing;
                                if accepted.revision >= output.committed_revision {
                                    output.committed_revision = accepted.revision;
                                    output.snapshot = Some(snapshot);
                                }
                                output.error = link_error;
                            } else {
                                output.key = None;
                            }
                            let patch = native_patch
                                .into_iter()
                                .filter(|(key, _)| output.live.get(key) == captured_live.get(key))
                                .collect::<Map<_, _>>();
                            if !patch.is_empty() {
                                // A slide on air that this process did not put there: the interface restarted.
                                taken_over = is_takeover(&output.live, &patch);
                                protocol::merge_patch(Arc::make_mut(&mut output.live), &patch);
                            }
                        }
                        Err(error) => output.error = Some(error),
                    }
                }
                cx.notify();
            });
            if taken_over {
                crate::automation::adopted(&runtime, cx);
            }
            sync(&runtime, cx);
        });
    })
    .detach();
}
