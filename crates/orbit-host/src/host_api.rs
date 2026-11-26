//! The host's call surface: the bridge dispatcher and the raw frame renderers.
//!
//! Every call blocks, so a shell runs them off its UI thread.

use super::*;

impl Host {
    pub fn select_media_files(&self) -> Option<Vec<std::path::PathBuf>> {
        FileDialog::new()
            .set_title("Import media")
            .add_filter(
                "Images and videos",
                &[
                    "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "svg", "mp4", "mov",
                    "mkv", "webm", "avi",
                ],
            )
            .pick_files()
    }

    /// Native bitmap counterpart of renderMediaEffectPreview for the GPUI inspector.
    pub fn render_media_effect_preview(
        &self,
        workspace: &str,
        media_id: &str,
        chain: &Value,
    ) -> Result<image::RgbaImage, String> {
        let state = self.state::<AppState>();
        let path = active_workspace(state, Value::String(workspace.to_string()))?;
        media_thumbnail::effect_preview_image(&path, media_id, chain)
    }

    /// A bounded bin thumbnail from the UI's current media snapshot, without rereading workspace files.
    pub fn render_media_thumbnail(
        &self,
        workspace: &str,
        item: &Value,
    ) -> Result<image::RgbaImage, String> {
        let path = active_workspace(
            self.state::<AppState>(),
            Value::String(workspace.to_string()),
        )?;
        media_thumbnail::thumbnail_image(&path, item)
    }

    pub fn render_background_preview(
        &self,
        css: &str,
    ) -> Result<native_compositor::NativeFrame, String> {
        native_compositor::render_scene_still(&json!({"backgroundCss":css}), 160, 90)
    }

    /// UI-only asset preview; this never creates a program sink or writes workspace data.
    pub fn render_media_asset_preview(
        &self,
        workspace: &str,
        mut item: Value,
        seconds: f64,
        stream: &mut Option<native_compositor::NativeCompositor>,
        clock_revision: Option<u64>,
    ) -> Result<native_compositor::NativeFrame, String> {
        if !seconds.is_finite() || seconds < 0. {
            return Err("Preview position must be finite and non-negative.".into());
        }
        let path = active_workspace(self.state::<AppState>(), json!(workspace))?;
        let source = media_thumbnail::asset_path(&path, &item)?;
        item["sourcePath"] = json!(source);
        let video = item["type"] == "video";
        if item["type"] != "image" && !video {
            return Err("Visual preview requires an image or video.".into());
        }
        item["type"] = json!(if video { "Video" } else { "Image" });
        item["mediaType"] = json!(if video { "video" } else { "image" });
        let (width, height) = INSPECTOR_PREVIEW_SIZE;
        item["bounds"] = json!({"x":0,"y":0,"width":width,"height":height});
        item["mediaPlaybackState"] = json!({
            "status": if clock_revision.is_some() { "playing" } else { "paused" },
            "currentTime": seconds,
            "clockRevision": clock_revision.unwrap_or_else(|| seconds.to_bits())
        });
        let scene = json!({"backgroundCss":"#000000","presentationThemeSlide":{"width":width,"height":height,"layers":[item]}});
        if clock_revision.is_none() {
            *stream = None;
        }
        let compositor =
            stream.get_or_insert_with(native_compositor::NativeCompositor::new_static_preview);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let frame = compositor.try_render_preview_for("media-asset", &scene, width, height)?;
            if !frame.dynamic_media {
                return Ok(frame);
            }
            if std::time::Instant::now() >= deadline {
                return Err("Media preview did not produce a decoded frame.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Persist the inspector's already rendered frame through the ordinary thumbnail transaction.
    pub fn save_inspector_thumbnail(
        &self,
        args: &[Value],
        asset: &Value,
        frame: &native_compositor::NativeFrame,
    ) -> Result<Value, String> {
        update_video_thumbnail(
            self.state::<AppState>(),
            "setVideoThumbnail",
            args,
            Some((asset, frame)),
        )
    }

    /// Serialize native operator edits against cues, automation, and workspace switches.
    pub fn edit_operator_output(
        &self,
        workspace: &str,
        edit: impl FnOnce(&mut Value) -> Result<(), String>,
    ) -> Result<Value, String> {
        let state = self.state::<AppState>();
        let workspace_io = lock_workspace_io(state)?;
        workspace_io.require_active(std::path::Path::new(workspace))?;
        let rss = state
            .rss_output
            .lock()
            .map_err(|_| "Output RSS state is unavailable.")?
            .snapshot();
        let mut output = lock_output_snapshot_state(&state.output_snapshot)?;
        let previous = output
            .cache
            .as_ref()
            .ok_or("Native output is not ready.")?
            .snapshot
            .clone();
        let mut next = previous.clone();
        edit(&mut next)?;
        let patch = next
            .as_object()
            .ok_or("Output snapshot must be an object.")?
            .iter()
            .filter(|(key, value)| previous.get(*key) != Some(*value))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<serde_json::Map<_, _>>();
        let changed = next != previous;
        let (revision, pending) =
            commit_native_output_snapshot(state, &mut output, next, &rss, None, None)?;
        drop(output);
        drop(workspace_io);
        schedule_output_rss_refreshes(self, pending);
        let response = json!({"ok":true,"changed":changed,"revision":revision,"patch":patch});
        if changed {
            self.emit_to("main", "native-output-state-patch", response.clone())?;
        }
        Ok(response)
    }

    /// Operator cue commands share the native coordinator used by Remote, without network dispatch.
    pub fn execute_cue_command(&self, command: &str, payload: &Value) -> Result<Value, String> {
        if !matches!(
            command,
            "cue" | "libraryCue" | "playlistCue" | "nextSlide" | "previousSlide"
        ) {
            return Err("Unsupported operator cue command.".into());
        }
        let state = self.state::<AppState>();
        let _workspace_guard = if matches!(command, "nextSlide" | "previousSlide") {
            Some(lock_workspace_io(state)?)
        } else {
            None
        };
        let path = state
            .active_workspace
            .lock()
            .map_err(|_| "Workspace state is unavailable.")?
            .clone()
            .ok_or("An active workspace is required.")?;
        if payload["workspacePath"].as_str() != path.to_str() {
            return Err("The operator cue command belongs to a different workspace.".into());
        }
        let mut response = match command {
            "playlistCue" => remote_controls::execute(self, state, "operatorPlaylistCue", payload)?,
            "libraryCue" => remote_controls::execute(self, state, "operatorLibraryCue", payload)?,
            _ => execute_native_remote_command(self, command, payload)?,
        };
        if matches!(command, "nextSlide" | "previousSlide") && response["status"] == "not-ready" {
            let output = lock_output_snapshot_state(&state.output_snapshot)?;
            response["presentationNextAnchor"] = output
                .cache
                .as_ref()
                .map(|cache| cache.snapshot["presentationNextAnchor"].clone())
                .unwrap_or(Value::Null);
            return Ok(response);
        }
        if response["ok"] != true {
            return Err(response["error"]
                .as_str()
                .unwrap_or("The cue command was not accepted.")
                .to_string());
        }
        Ok(response)
    }

    /// A direct take can use the last committed cue while destination or enabled flags are being saved.
    pub fn execute_operator_cue_before_save(
        &self,
        command: &str,
        payload: &Value,
        workspace_revision: u64,
    ) -> Result<Value, String> {
        let response = remote_controls::execute_operator_cue_before_save(
            self,
            self.state::<AppState>(),
            command,
            payload,
            workspace_revision,
        )?;
        if response["ok"] != true {
            return Err(response["error"]
                .as_str()
                .unwrap_or("The cue command was not accepted.")
                .to_string());
        }
        Ok(response)
    }

    /// The same occurrence identities and master/arrangement order as native program output.
    pub fn presentation_cues(
        item: &Value,
        arrangement: &str,
        occurrence: Option<&str>,
    ) -> Result<Vec<Value>, String> {
        let mut item = item.clone();
        if !item["slides"].is_array() {
            return Err("Presentation slides must be an array.".into());
        }
        item["selectedArrangementId"] = json!(arrangement);
        output_arrangement::resolve_live_arrangement_slides(&item, None, occurrence)
            .ok_or_else(|| "The cue arrangement is unavailable.".to_string())
    }
    /// Refresh monitor topology; only output hosts reapply physical routing.
    pub fn refresh_output_displays(&self) -> Result<(), String> {
        let state = self.state::<AppState>();
        if state.output_role == OutputRole::Controller {
            return self.emit("output:displays-changed", serde_json::json!({}));
        }
        let snapshot = lock_output_snapshot_state(&state.output_snapshot)?;
        if let Some(cached) = snapshot.cache.as_ref() {
            let config = cached
                .snapshot
                .get("screenConfig")
                .ok_or("The output snapshot has no screen configuration.")?;
            synchronize_display_outputs_locked(self, state, config)?;
        }
        drop(snapshot);
        self.emit("output:displays-changed", serde_json::json!({}))
    }
    /// A frozen cue preview, composed by the same theme/scene builder as program output.
    pub fn render_cue_still(
        &self,
        snapshot: &Value,
        item: &Value,
        slide: &Value,
        playlist_id: &str,
        width: u32,
        height: u32,
    ) -> Result<native_compositor::NativeFrame, String> {
        let mut input = json!({
            "liveItem": item, "liveItemId": item["id"], "liveSlide": slide, "liveSlideId": slide["id"],
            "livePlaylistId": if playlist_id.is_empty() { Value::Null } else { json!(playlist_id) },
            "liveBuildStep": 0, "backgroundId": slide["mediaBackgroundId"],
            "backgroundScale": "fit", "backgroundPlayback": "stop", "backgroundEndBehavior": "hold",
            "screenConfig": { "cue-preview": { "lookLayers": {
                "slide": true, "media": true, "music": false, "props": false,
                "messages": false, "announcements": false, "videoInput": false
            }}},
        });
        for key in [
            "themes",
            "libraries",
            "playlist",
            "media",
            "backgrounds",
            "groupSettings",
            "countdownTimers",
            "countdownTimerRuns",
        ] {
            if let Some(value) = snapshot.get(key) {
                input[key] = value.clone();
            }
        }
        let mut scene = output_scene::build_output_scene_state(&input)?
            .remove("cue-preview")
            .ok_or("The cue preview scene was not built.")?;
        if let Some(composed) = scene
            .get_mut("presentationThemeSlide")
            .filter(|slide| slide.is_object())
        {
            composed["ignoreBuilds"] = json!(true);
        }
        native_compositor::render_scene_still(&scene, width as usize, height as usize)
    }

    /// Runs one bridge method; each method validates its own `args`.
    pub fn invoke(&self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        let state = self.state::<AppState>();
        let result = if matches!(
            method,
            "connectCommunicationInput" | "disconnectCommunicationInput"
        ) {
            if method == "connectCommunicationInput" {
                communication_input_commands::connect(
                    self,
                    state,
                    argument(&args, 0)?,
                    argument(&args, 1)?,
                )
            } else {
                communication_input_commands::disconnect(
                    state,
                    argument(&args, 0)?,
                    argument(&args, 1)?,
                )
            }
        } else if method == "sendCommunication" {
            let device = argument(&args, 0)?.clone();
            let payload = argument(&args, 1)?.clone();
            if device["transport"] == "midi" {
                let pending = communication_midi::submit(Some((device, payload)))?;
                return communication_midi::wait(pending);
            }
            communication::send(device, payload)
        } else if method == "releaseMidiOutputNotes" {
            let pending = communication_midi::submit(None)?;
            communication_midi::wait(pending)
        } else if method == "scanProPresenterWorkspace" {
            scan_pro_presenter_workspace()
        } else if method == "selectProPresenterWorkspace" {
            select_pro_presenter_workspace(self, state, &argument(&args, 0)?, &argument(&args, 1)?)
        } else if method == "checkAppUpdate" {
            self.updater().check(self, false)
        } else if method == "installAppUpdate" {
            self.updater().install(self)
        } else if method == "setVideoThumbnail" || method == "resetVideoThumbnail" {
            update_video_thumbnail(state, method, &args, None)
        } else if method == "renderMediaEffectPreview" {
            let path = active_workspace(state, argument(&args, 0)?.clone())?;
            let media_id = argument(&args, 1)?
                .as_str()
                .ok_or("Media asset id is required.")?
                .to_owned();
            let chain = argument(&args, 2)?.clone();
            media_thumbnail::effect_preview(&path, &media_id, &chain).map(Value::String)
        } else if method == "renderSlideStill" {
            let slide = argument(&args, 0)?.clone();
            let size = |index: usize| {
                argument(&args, index)?
                    .as_u64()
                    .map(|value| value as usize)
                    .ok_or_else(|| "Slide still size must be a whole number.".to_string())
            };
            let (width, height) = (size(1)?, size(2)?);
            native_compositor::render_slide_still(&slide, width, height)
                .and_then(|frame| frame.encode_png())
                .map(|png| {
                    Value::String(format!("data:image/png;base64,{}", workspace::base64(&png)))
                })
        } else if method == "renderTransitionPreview" {
            let effect = argument(&args, 0)?
                .as_str()
                .ok_or("Transition effect id is required.")?
                .to_owned();
            let progress = argument(&args, 1)?
                .as_f64()
                .ok_or("Preview progress must be a number.")?;
            let size = |index: usize| {
                argument(&args, index)?
                    .as_u64()
                    .map(|value| value as usize)
                    .ok_or_else(|| "Preview size must be a whole number.".to_string())
            };
            native_transitions::render_preview(&effect, progress, size(2)?, size(3)?)
                .and_then(|frame| frame.encode_png())
                .map(|png| {
                    Value::String(format!("data:image/png;base64,{}", workspace::base64(&png)))
                })
        } else if method == "probeMediaMetadata" {
            let source = argument(&args, 0)?;
            media_metadata::probe(&source)
        } else if method == "getTimelineWaveform" {
            let path = active_workspace(state, argument(&args, 0)?.clone())?;
            let media_id = argument(&args, 1)?
                .as_str()
                .ok_or("Media asset id is required.")?
                .to_owned();
            let buckets = argument(&args, 2)?
                .as_u64()
                .ok_or("Waveform buckets must be a whole number.")?
                as usize;
            native_soundtrack::waveform_for_media(&path, &media_id, buckets)
        } else if timecode_input::handles(method) {
            timecode_input::invoke_app(self, method, &args)
        } else if method == "getMidiInputStatus" {
            self.state::<midi_input::MidiInputService>()
                .status()
                .and_then(|status| serde_json::to_value(status).map_err(|error| error.to_string()))
        } else if method == "setMidiInputEnabled" {
            let enabled = argument(&args, 0)?
                .as_bool()
                .ok_or("MIDI input enabled state must be a boolean.")?;
            let event_host = self.clone();
            let sink: midi_input::MidiInputSink = Arc::new(move |event| {
                let _ = match event {
                    midi_input::MidiInputEvent::Note(note) => event_host.emit("midi-note", note),
                    midi_input::MidiInputEvent::Status(status) => {
                        event_host.emit("midi-input-status", status)
                    }
                };
            });
            self.state::<midi_input::MidiInputService>()
                .set_enabled(&sink, enabled)
                .and_then(|status| serde_json::to_value(status).map_err(|error| error.to_string()))
        } else if network_link_membership::handles(method) {
            network_link_membership::run(self, method, &args)
        } else if network_link_commands::handles(method) {
            network_link_commands::run(self, state, method, &args)
        } else if bible_commands::handles(method) {
            bible_commands::run(self, state, method, &args)
        } else if external_presentation::handles(method) {
            external_presentation::invoke(
                self.state::<external_presentation::ExternalPresentationService>(),
                method,
                &args,
            )
        } else if method == "listNdiSources" {
            ndi_input::discover_with(&ndi_find_options::from_settings(
                &settings::normalize_general_settings_value(
                    &read_settings(self)?["generalSettings"],
                ),
            ))
            .and_then(|sources| serde_json::to_value(sources).map_err(|error| error.to_string()))
        } else if method == "resolveCaptureStart" {
            capture_commands::resolve_capture_start(self, state, &args)
        } else if method == "startCaptureResolved" {
            capture_commands::start_capture_resolved(self, state, &args)
        } else if method == "listAudioOutputDevices" {
            // GPUI settings list the devices natively; the WebView app enumerates them itself.
            native_audio::list_output_devices()
        } else if method == "listCameraDevices" {
            camera_devices::list_devices().and_then(|devices| {
                serde_json::to_value(devices).map_err(|error| error.to_string())
            })
        } else {
            bridge_invoke_inner(self, state, method, args)
        };
        if let Err(error) = &result {
            record_bridge_error(self, method, error);
        }
        result
    }

    /// Direct compositor access for native UI consumers. Pixels remain straight-alpha BGRA8.
    pub fn render_output_bgra_frame(
        &self,
        source_screen_id: &str,
        width: u32,
        height: u32,
    ) -> Result<Arc<native_compositor::NativeFrame>, String> {
        render_native_output_bgra_frame_impl(
            self.state::<AppState>(),
            source_screen_id,
            width,
            height,
        )
    }

    pub fn render_program_preview_bgra_frame(
        &self,
        preview_id: &str,
        scene: NativeProgramPreviewScene,
        width: u32,
        height: u32,
    ) -> Result<native_compositor::NativeFrame, String> {
        render_native_program_preview_bgra_frame_impl(
            self.state::<AppState>(),
            preview_id,
            scene,
            width,
            height,
        )
    }

    pub fn render_output_frame(
        &self,
        source_screen_id: String,
        width: u32,
        height: u32,
        transport: NativeFrameTransport,
    ) -> Result<Vec<u8>, String> {
        render_native_output_frame_impl(
            self.state::<AppState>(),
            source_screen_id,
            width,
            height,
            transport,
        )
    }

    pub fn render_program_preview_frame(
        &self,
        preview_id: &str,
        scene: NativeProgramPreviewScene,
        width: u32,
        height: u32,
        transport: NativeFrameTransport,
    ) -> Result<Vec<u8>, String> {
        render_native_program_preview_frame_impl(
            self.state::<AppState>(),
            preview_id,
            scene,
            width,
            height,
            transport,
        )
    }

    pub fn render_ndi_input_frame(&self, source: ndi_input::NdiSource) -> Result<Vec<u8>, String> {
        ndi_input::preview_packet(source)
    }

    pub fn render_camera_input_frame(
        &self,
        source: camera_devices::CameraSource,
    ) -> Result<Vec<u8>, String> {
        camera_input::preview_packet(source)
    }
}
