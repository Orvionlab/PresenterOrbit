mod events;
mod host;
mod platform;
mod window;

pub use events::HostEvents;
pub use host::{Host, HostParts, HostStates, PathResolver, WeakHost};
pub use macro_image::decode_data as decode_macro_image_data;
pub use platform::{DesktopIntegration, Updater, WindowSystem};
pub use settings::{
    mutate_group_settings_value, mutate_key_mappings_value, mutate_screen_config_value,
};
pub use window::{
    CloseRequest, DisplayBounds, Monitor, PhysicalPosition, PhysicalSize, Window, WindowBackend,
    WindowEvent, WindowEventHandler,
};
pub use workspace::new_transition_preset;

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
};

use chrono::Utc;
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use serde_json::{json, Value};

mod general_settings_options;
pub mod ndi_find_options;
mod os_trash;
mod sample_content;
pub use sample_content::initialize as initialize_sample_content;
const DEFAULT_WORKSPACE_NAME: &str = "Default";
mod app_state;
pub mod audience_looks;
mod audio_input_config;
pub mod authoring;
mod bible;
mod bible_commands;
mod bible_presentation;
pub mod camera_devices;
mod camera_input;
mod capture;
mod capture_commands;
pub mod ccli_template;
mod chord_chart;
mod chord_chart_commands;
mod chord_chart_pdf;
mod communication;
mod communication_artnet;
mod communication_deck;
mod communication_deck_runtime;
mod communication_globalcache;
mod communication_input;
mod communication_input_commands;
mod communication_input_config;
mod communication_midi;
mod communication_ross_input;
mod communication_serial;
pub mod cue_view_options;
pub mod diagnostics;
mod display_names;
pub mod external_presentation;
mod handout;
mod handout_commands;
#[cfg(windows)]
mod handout_print;
mod host_api;
mod key_fill;
mod keynote_import;
mod keynote_import_commands;
mod library_search;
mod linked_services;
mod local_text;
mod macro_collections;
mod macro_image;
mod media_cleanup;
mod media_metadata;
mod media_relink;
mod media_thumbnail;
pub mod midi_input;
mod native_audio;
mod native_automation;
pub mod native_compositor;
mod native_fonts;
mod native_hdr;
mod native_hdr_gpu;
mod native_hdr_platform;
mod native_key_fill;
mod native_media;
mod native_media_effects;
mod native_messages;
mod native_output;
mod native_props;
mod native_render;
mod native_soundtrack;
mod native_text_layout;
mod native_text_list;
mod native_text_tabs;
mod native_ticker;
mod native_ticker_motion;
mod native_timeline;
mod native_timers;
pub mod native_transitions;
mod native_waveform;
mod ndi;
pub mod ndi_input;
mod network_link;
mod network_link_commands;
mod network_link_controls;
mod network_link_discovery;
mod network_link_membership;
mod output_arrangement;
mod output_canvas;
mod output_delay;
pub mod output_layout;
mod output_playback;
mod output_scene;
pub use output_scene::resolve_stage_layout;

mod portable_bundle;
mod portable_bundle_commands;
mod powerpoint_import;
mod powerpoint_import_commands;
mod presentation_export;
mod presentation_settings;
mod projector_correction;
mod remote;
mod remote_controls;
mod remote_cues;
mod remote_definitions;
mod remote_media;
mod remote_playback_commands;
mod remote_timeline_commands;
mod rss;
mod rss_output;
mod scene;
mod settings;
mod slide_composition;
mod test_pattern;
mod ticker_commands;
mod timecode;
mod timecode_display;
pub mod timecode_input;
mod timecode_ltc;
mod timecode_mtc;
mod trigger_bindings;
mod video_input;
pub mod web_capture;
mod workspace;
pub use workspace::{
    export_pro_presenter_workspace as export_pro_presenter_workspace_to,
    import_pro_presenter_workspace_media, plan_media_library_move, plan_props_mutation,
    ImportSelection, PropsMutationResult,
};
mod workspace_export;
mod workspace_import;
mod workspace_import_selection;
mod workspace_registry;
mod workspace_sync;
mod workspace_sync_commands;

pub(crate) use app_state::*;

fn bridge_invoke_inner(
    app: &Host,
    state: &AppState,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    match method {
        "getVersion" => Ok(json!(app.version())),
        "getAppUpdateStatus" => Ok(app.updater().status()),
        "replaceLibrarySearchIndex" => replace_library_search_index(&state, &args),
        "searchLibraryIndex" => search_library_index(&state, &args),
        "getRemoteNetworkAddress" => remote::network_address().map(Value::String),
        "getSettings" => get_settings(&app, &state),
        "saveSettings" => save_settings(
            &app,
            &state,
            argument(&args, 0)?,
            Some(expected_settings_revision(&args)?),
        ),
        "mutateGroupSettings" => mutate_group_settings(
            &app,
            &state,
            argument(&args, 0)?,
            argument(&args, 1)?,
            expected_settings_revision_at(&args, 2)?,
            args.get(3).cloned().unwrap_or(Value::Null),
        ),
        "mutateKeyMappings" => mutate_key_mappings(
            &app,
            &state,
            argument(&args, 0)?,
            argument(&args, 1)?,
            expected_settings_revision_at(&args, 2)?,
            args.get(3).cloned().unwrap_or(Value::Null),
        ),
        "mutateScreenConfig" => mutate_screen_config(
            &app,
            &state,
            argument(&args, 0)?,
            argument(&args, 1)?,
            expected_settings_revision_at(&args, 2)?,
            args.get(3).cloned().unwrap_or(Value::Null),
        ),
        "windowCommand" => window_command(&app, argument(&args, 0)?),
        "openExternalUrl" => open_external_url(argument(&args, 0)?),
        "sendCommunication" => send_communication(argument(&args, 0)?, argument(&args, 1)?),
        "listSerialPorts" => communication_serial::ports(),
        "listMidiOutputPorts" => communication_midi::ports(),
        "releaseMidiOutputNotes" => communication_midi::release(),
        "getCommunicationInputStatus" => Ok(communication_input_commands::lock(&state)?.status()),
        "acceptCommunicationInput" => {
            communication_input_commands::accept(&state, &argument(&args, 0)?)
        }
        "listAudioInputDevices" => native_audio::inputs::list_devices(),
        "getAudioInputStatus" => Ok(state
            .native_audio
            .lock()
            .map_err(|_| "Native audio state is unavailable.")?
            .input_status()),
        "openSettingsWindow" => open_settings_window(&app, argument(&args, 0)?),
        "openAuxiliaryWindow" => app.windows().open_auxiliary_window(argument(&args, 0)?),
        "prepareAuxiliaryWindows" => app.windows().prepare_auxiliary_windows(),
        "closeAuxiliaryWindow" => app.windows().close_auxiliary_window(&argument(&args, 0)?),
        "listInstalledFonts" => Ok(json!(native_fonts::installed_font_families())),
        "openLogFiles" => open_log_files(&app),
        "showItemInFolder" => show_item_in_folder(argument(&args, 0)?),
        "readClipboardText" => read_clipboard_text(),
        "writeClipboardText" => write_clipboard_text(argument(&args, 0)?),
        "exportDiagnostics" => export_diagnostics(&app, &state),
        "applyAudioSettings" => apply_audio_settings(
            &app,
            &state,
            argument(&args, 0)?,
            Some(expected_settings_revision(&args)?),
        ),
        "applyScreenConfig" => apply_screen_config(
            &app,
            &state,
            argument(&args, 0)?,
            Some(expected_settings_revision(&args)?),
        ),
        "setDisplayOutputVisibility" => set_display_output_visibility(
            &app,
            &state,
            argument(&args, 0)?,
            argument(&args, 1)?,
            args.get(2),
        ),
        "closeDisplayOutput" => close_display_output(&app, &state, argument(&args, 0)?),
        "controlPlayback" => control_playback(&app, &state, argument(&args, 0)?),
        "controlTimer" => control_timer(&state, &args),
        "reportRemoteCommandResult" => state.remote_status.report(&argument(&args, 0)?),
        "testTriggerBinding" => {
            let path = group_settings_workspace_path(&state, argument(&args, 0)?)?
                .ok_or("Trigger binding test requires an active workspace.")?;
            let binding_id = argument(&args, 1)?;
            let binding_id = binding_id
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or("Trigger binding id is required.")?;
            let workspace_io = lock_workspace_io(&state)?;
            workspace_io.require_active(&path)?;
            let workspace = workspace::read_workspace(std::path::Path::new(&path))?;
            if !workspace
                .get("triggerBindings")
                .and_then(Value::as_array)
                .is_some_and(|bindings| {
                    bindings.iter().any(|binding| {
                        binding.get("id").and_then(Value::as_str) == Some(binding_id)
                    })
                })
            {
                return Err("Trigger binding is no longer present.".into());
            }
            app.emit_to(
                "main",
                "trigger-binding-test",
                json!({"workspacePath":argument(&args, 0)?,"bindingId":binding_id}),
            )
            .map_err(|error| error.to_string())?;
            drop(workspace_io);
            Ok(Value::Null)
        }
        "updateAutomationSchedule" => update_automation_schedule(&state, argument(&args, 0)?),
        "scheduleAutomationActions" => schedule_automation_actions(&state, argument(&args, 0)?),
        "getSlideCompletion" => get_slide_completion(state),
        "cancelSlideCompletion" => cancel_slide_completion(state, argument(&args, 0)?),
        "getNdiStatus" => get_ndi_status(&state),
        "startNdiOutput" => start_ndi_output(&state, argument(&args, 0)?, argument(&args, 1)?),
        "updateNdiOutput" => update_ndi_output(&state, argument(&args, 0)?, argument(&args, 1)?),
        "stopNdiOutput" => stop_ndi_output(&state, argument(&args, 0)?),
        "stopAllNdiOutputs" => stop_all_ndi_outputs(&state),
        "updateOutputScene" => update_output_scene(&app, &state, argument(&args, 0)?),
        "getOutputContent" => get_output_content(&state, argument(&args, 0)?),
        "prepareMedia" => native_render::media_preload::prepare(argument(&args, 0)?),
        "getMediaPreparationStatus" => native_render::media_preload::status(),
        "getNativeMediaPlayback" => get_native_media_playback(&state, argument(&args, 0)?, false),
        "getNativeMusicPlayback" => get_native_media_playback(&state, argument(&args, 0)?, true),
        "getProgramAudioLevels" => Ok(json!(native_audio::program_audio_levels())),
        "getMarkerPositions" => Ok(native_render::marker_positions()),
        "acceptMarkerPositions" => native_render::accept_marker_positions(&argument(&args, 0)?),
        "listWindowedOutputs" => list_windowed_outputs(&state),
        "createWindowedOutput" => create_windowed_output(&app, &state, &args),
        "focusWindowedOutput" => focus_windowed_output(&app, &state, argument(&args, 0)?),
        "getOutputOptions" => get_output_options(&app, &state),
        "loadWorkspace" => load_workspace(&app, &state, argument(&args, 0)?),
        "readWorkspaceSnapshot" => read_workspace_snapshot(&state, argument(&args, 0)?),
        "selectSyncRepository" => workspace_sync_commands::select_repository(),
        "previewWorkspaceSync" => workspace_sync_commands::run(&app, &state, &args, false),
        "applyWorkspaceSync" => workspace_sync_commands::run(&app, &state, &args, true),
        "selectOrbitWorkspaceFolder" => select_orbit_workspace_folder(&app, &state),
        "listOrbitWorkspaces" => list_orbit_workspaces(&app, &state),
        "createOrbitWorkspace" => {
            create_orbit_workspace(&app, &state, string_argument(&args, 0, "Workspace name")?)
        }
        "switchOrbitWorkspace" => {
            switch_orbit_workspace(&app, &state, string_argument(&args, 0, "Workspace path")?)
        }
        "renameOrbitWorkspace" => rename_orbit_workspace(
            &app,
            &state,
            string_argument(&args, 0, "Workspace path")?,
            string_argument(&args, 1, "Workspace name")?,
        ),
        "duplicateOrbitWorkspace" => duplicate_orbit_workspace(
            &app,
            &state,
            string_argument(&args, 0, "Workspace path")?,
            string_argument(&args, 1, "Workspace name")?,
        ),
        "deleteOrbitWorkspace" => {
            delete_orbit_workspace(&app, &state, string_argument(&args, 0, "Workspace path")?)
        }
        "exportPresentation" => {
            export_presentation(&state, argument(&args, 0)?, argument(&args, 1)?)
        }
        "exportProPresenterWorkspace" => {
            export_pro_presenter_workspace(&state, argument(&args, 0)?)
        }
        "selectPresentationTextFile" => select_presentation_text_file(),
        "selectPresentationImportFiles" => portable_bundle_commands::select_files(
            argument(&args, 0)?
                .as_bool()
                .ok_or("ProPresenter-only file selection must be a boolean.")?,
        ),
        "importPresentationFiles" => portable_bundle_commands::import_files(state, &args),
        "startLocalTextWatch" => {
            let workspace = args
                .first()
                .and_then(Value::as_str)
                .ok_or("Workspace path is required.")?;
            let active = state
                .active_workspace
                .lock()
                .map_err(|_| "Workspace lock failed.")?;
            if active.as_deref() != Some(Path::new(workspace)) {
                return Err("Local text workspace is no longer active.".into());
            }
            let paths: Vec<String> =
                serde_json::from_value(args.get(1).cloned().ok_or("File paths are required.")?)
                    .map_err(|error| error.to_string())?;
            Ok(json!(state.local_text.start(paths)?))
        }
        "readLocalTextWatch" => state.local_text.snapshot(
            args.first()
                .and_then(Value::as_u64)
                .ok_or("Watch ID is required.")?,
        ),
        "stopLocalTextWatch" => {
            state.local_text.stop(
                args.first()
                    .and_then(Value::as_u64)
                    .ok_or("Watch ID is required.")?,
            )?;
            Ok(json!({ "ok": true }))
        }
        "fetchRssFeed" => fetch_rss_feed(argument(&args, 0)?),
        "fetchRssFeeds" => fetch_rss_feeds(argument(&args, 0)?),
        "selectLogoImage" => select_logo_image(),
        "selectAudioFiles" => select_audio_files(),
        "selectMediaSmartPlaylistFolder" => select_media_smart_playlist_folder(),
        "saveWorkspaceContent" => save_workspace_content(&app, &state, &args),
        "saveLibraryItem" => save_library_item(&state, &args),
        "listThemes" => list_workspace_field(&state, &args, "themes"),
        "saveTheme" => save_named_workspace_value(&state, &args, "themes"),
        "deleteTheme" => delete_workspace_theme(&state, &args),
        "listMasks" => list_workspace_field(&state, &args, "masks"),
        "saveMask" => save_named_workspace_value(&state, &args, "masks"),
        "mutateMasks" => mutate_masks(&state, &args),
        "listProps" => list_props(&state, &args),
        "mutateProps" => mutate_props(&state, &args),
        "mutateCountdownTimers" => mutate_countdown_timers(&state, &args),
        "saveProps" => save_props(&state, &args),
        "listStage" => list_stage(&state, &args),
        "mutateStageContent" => mutate_stage_content(&state, &args),
        "saveStage" => save_stage(&state, &args),
        "saveMediaLibrary" => save_media_library(&state, &args),
        "saveMediaLibraries" => save_media_libraries(&state, &args),
        "saveMediaSettings" => save_media_settings(&state, &args),
        "selectMacroImage" => {
            let path = active_workspace(&state, argument(&args, 0)?)?;
            macro_image::choose(&path).map(|image| json!(image))
        }
        "exportPresentationBundle" => portable_bundle_commands::export(&state, &args),
        "exportHandout" => handout_commands::run(&state, &args, false),
        "previewHandout" => handout_commands::preview(&state, &args),
        "acceptLinkedServiceState" => state.linked_services.accept(argument(&args, 0)?),
        "controlTicker" => ticker_commands::control(&app, &state, &args),
        "getTickerStatus" => ticker_commands::status(&app, &state, &args),
        "saveTickers" => ticker_commands::save(&state, &args),
        "choosePresentationArtwork" => chord_chart_commands::choose_artwork(state, &args),
        "chooseChordChart" => chord_chart_commands::choose(&state, &args),
        "printHandout" => handout_commands::run(&state, &args, true),
        "importPresentationBundle" => portable_bundle_commands::import(&state, &args),
        "importPowerPointImages" => powerpoint_import_commands::run(&app, &state, &args),
        "importKeynotePresentation" => keynote_import_commands::run(&state, &args),
        "selectCaptureDirectory" => capture_commands::select_capture_directory(),
        "saveCapturePreset" => capture_commands::save_capture_preset(&app, &state, &args),
        "deleteCapturePreset" => capture_commands::delete_capture_preset(&app, &state, &args),
        "startCapture" => capture_commands::start_capture(&app, &state, &args),
        "stopCapture" => capture_commands::stop_capture_action(&state, &args),
        "getCaptureStatus" => capture_commands::capture_status(&state),
        "getTransitionCatalog" => Ok(native_transitions::catalog_value()),
        "getMediaEffectCatalog" => Ok(native_media_effects::catalog_value()),
        "listMissingMedia" => list_missing_media(&state, &args),
        "reviewMediaCleanup" => review_media_cleanup(&state, &args),
        "cleanUnusedMedia" => clean_unused_media(&state, &args),
        "relinkMissingMedia" => relink_missing_media(&app, &state, &args),
        "relinkMissingMediaFromSearchPaths" => {
            relink_missing_media_from_search_paths(&app, &state, &args)
        }
        "listMediaSearchLocations" => media_search_locations(&app),
        "selectMediaSearchPath" => Ok(FileDialog::new()
            .set_title("Choose missing media search path")
            .pick_folder()
            .map_or_else(
                || json!({"canceled":true}),
                |path| json!({"canceled":false,"path":path}),
            )),
        "forgetMissingMedia" => forget_missing_media(&state, &args),
        "saveMediaState" => save_media_state(&state, &args),
        "mutateMediaLibrary" => mutate_media_library(&state, &args),
        "mutatePresentationLibrary" => mutate_presentation_library(&state, &args),
        "mutatePresentationPlaylist" => mutate_presentation_playlist(&state, &args),
        "selectThemeAssetFile" => select_theme_asset_file(),
        "getLibraryItemLocation" => get_library_item_location(&state, &args),
        "saveLibraryItems" => save_library_items(&state, &args),
        // Tauri's WebView API has no cross-platform equivalent of Chromium's
        // `replaceMisspelling`. The renderer falls back to its preserved text
        // selection when this returns false.
        "replaceMisspelling" => Ok(json!(false)),
        _ => Err(format!(
            "The Tauri host does not implement bridge method '{method}' yet."
        )),
    }
}

fn record_bridge_error(app: &Host, method: &str, error: &str) {
    let message = format!("{method}: {error}");
    record_diagnostic(
        app,
        "bridge-error",
        diagnostics::LogLevel::Error,
        "ipc",
        Some(&message),
    );
}

pub fn record_diagnostic(
    app: &Host,
    event: &str,
    level: diagnostics::LogLevel,
    subsystem: &str,
    message: Option<&str>,
) {
    let Ok(data_directory) = app.path().app_data_dir() else {
        return;
    };
    diagnostics::append_event(
        &data_directory,
        diagnostics::LogEvent {
            event,
            level,
            subsystem,
            correlation_id: None,
            message,
        },
    );
}

fn argument(args: &[Value], index: usize) -> Result<Value, String> {
    args.get(index)
        .cloned()
        .ok_or_else(|| format!("Missing argument {index}."))
}

fn expected_settings_revision(args: &[Value]) -> Result<u64, String> {
    expected_settings_revision_at(args, 1)
}

fn expected_settings_revision_at(args: &[Value], index: usize) -> Result<u64, String> {
    args.get(index)
        .and_then(Value::as_u64)
        .ok_or_else(|| "Settings base revision is required.".to_string())
}

fn replace_library_search_index(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let session_id = argument(args, 0)?;
    let session_id = session_id
        .as_str()
        .ok_or_else(|| "Library search session ID must be a string.".to_string())?;
    let generation = args
        .get(1)
        .and_then(Value::as_u64)
        .filter(|generation| *generation > 0)
        .ok_or_else(|| "Library search generation must be a positive integer.".to_string())?;
    let library = argument(args, 2)?;
    state
        .library_search
        .lock()
        .map_err(|_| "Library search index is unavailable.".to_string())?
        .replace(session_id, generation, &library)
}

fn search_library_index(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let session_id = argument(args, 0)?;
    let session_id = session_id
        .as_str()
        .ok_or_else(|| "Library search session ID must be a string.".to_string())?;
    let generation = args
        .get(1)
        .and_then(Value::as_u64)
        .filter(|generation| *generation > 0)
        .ok_or_else(|| "Library search generation must be a positive integer.".to_string())?;
    let query = argument(args, 2)?;
    let query = query
        .as_str()
        .ok_or_else(|| "Library search query must be a string.".to_string())?;
    Ok(state
        .library_search
        .lock()
        .map_err(|_| "Library search index is unavailable.".to_string())?
        .search(session_id, generation, query))
}

/// Windows monitor APIs can return verbatim device names. Keep IDs untouched
/// for output routing, but remove the prefix from labels shown to users.
pub fn strip_windows_verbatim_prefix(value: &str) -> String {
    const VERBATIM_UNC_PREFIX: &str = "\\\\?\\UNC\\";
    const VERBATIM_PREFIX: &str = "\\\\?\\";
    const DEVICE_PREFIX: &str = "\\\\.\\";

    if let Some(rest) = strip_prefix_ignore_ascii_case(value, VERBATIM_UNC_PREFIX) {
        format!("\\\\{rest}")
    } else if let Some(rest) = strip_prefix_ignore_ascii_case(value, VERBATIM_PREFIX) {
        rest.to_string()
    } else if let Some(rest) = strip_prefix_ignore_ascii_case(value, DEVICE_PREFIX) {
        rest.to_string()
    } else {
        value.to_string()
    }
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn display_label(name: &str) -> String {
    strip_windows_verbatim_prefix(name)
}

fn settings_path(app: &Host) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    Ok(directory.join("settings.json"))
}

fn read_settings(app: &Host) -> Result<Value, String> {
    let path = settings_path(app)?;
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(error.to_string()),
    }
}

fn snapshot_settings_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn restore_settings_file(path: &Path, previous: Option<&[u8]>) -> Result<(), String> {
    match previous {
        Some(contents) => fs::write(path, contents).map_err(|error| error.to_string()),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        },
    }
}

fn persist_settings_then_workspace<T, F>(
    settings_file: &Path,
    serialized_settings: &str,
    save_workspace: F,
) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String>,
{
    let previous_settings = snapshot_settings_file(settings_file)?;
    if let Err(error) = fs::write(settings_file, serialized_settings) {
        if let Err(restore_error) =
            restore_settings_file(settings_file, previous_settings.as_deref())
        {
            return Err(format!(
                "{error} Settings rollback also failed: {restore_error}"
            ));
        }
        return Err(error.to_string());
    }

    match save_workspace() {
        Ok(result) => Ok(result),
        Err(workspace_error) => {
            if let Err(restore_error) =
                restore_settings_file(settings_file, previous_settings.as_deref())
            {
                return Err(format!(
                    "{workspace_error} Settings rollback also failed: {restore_error}"
                ));
            }
            Err(workspace_error)
        }
    }
}

fn read_normalized_group_settings(app: &Host, state: &AppState) -> Result<Value, String> {
    let _settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    let settings = read_settings(app)?;
    Ok(settings::normalize_group_settings_value(
        settings.get("groupSettings").unwrap_or(&Value::Null),
    ))
}

fn get_settings(app: &Host, state: &AppState) -> Result<Value, String> {
    let settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    let settings_revision = state
        .settings_revision
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    let mut settings = read_settings(app)?;
    settings = migrate_legacy_settings(app, settings)?;
    normalize_native_settings(&mut settings)?;
    let mut revision = settings_revision.current;
    drop(settings_revision);
    drop(settings_operation);

    let stored_path = settings
        .get("presenterOrbitWorkspacePath")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .map(str::to_string);
    let first_run = stored_path.is_none();
    let raw_path = match stored_path {
        Some(path) => path,
        None => {
            let folder = default_workspaces_folder(app)?.join(DEFAULT_WORKSPACE_NAME);
            fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
            workspace::workspace_path(&folder)
                .to_string_lossy()
                .into_owned()
        }
    };
    let mut registry = workspace_registry::parse(
        settings
            .get("presenterOrbitWorkspaces")
            .unwrap_or(&Value::Null),
    )?;
    let registry_before = registry.clone();

    {
        let requested = PathBuf::from(raw_path);
        let workspace_path = if requested.is_dir() {
            workspace::workspace_path(&requested)
        } else {
            requested
        };
        let mut workspace_io = lock_workspace_io(state)?;
        workspace::ensure_workspace(&workspace_path)?;
        let workspace_path = workspace_path
            .canonicalize()
            .map_err(|error| error.to_string())?;
        settings["presenterOrbitWorkspacePath"] = json!(workspace_path);
        // Every window and revision recovery reads settings, so only the first read may activate:
        // a later one would undo an explicit loadWorkspace and stop a running recording.
        let nothing_active = state
            .active_workspace
            .lock()
            .map_err(|_| "Workspace state is unavailable.".to_string())?
            .is_none();
        workspace_registry::register(
            &mut registry,
            &workspace_path.to_string_lossy(),
            &workspace_folder_name(&workspace_path),
        );
        if nothing_active {
            capture_commands::stop_capture(state)?;
            native_ticker::clear()?;
            workspace_io.activate(&workspace_path);
            state
                .network_link
                .lock()
                .map_err(|_| "Network Link state is unavailable.")?
                .invalidate_pending();
            *state
                .active_workspace
                .lock()
                .map_err(|_| "Workspace state is unavailable.".to_string())? =
                Some(workspace_path.clone());
        }
    }

    if first_run || registry != registry_before {
        let _settings_operation = state
            .settings_operations
            .lock()
            .map_err(|_| "Settings state is unavailable.".to_string())?;
        let saved = save_settings_locked(
            app,
            state,
            json!({
                "presenterOrbitWorkspacePath": settings["presenterOrbitWorkspacePath"],
                "presenterOrbitWorkspaces": registry,
            }),
            None,
        )?;
        revision = saved
            .get("settingsRevision")
            .and_then(Value::as_u64)
            .ok_or_else(|| "Settings revision is unavailable.".to_string())?;
        settings["presenterOrbitWorkspaces"] = saved["presenterOrbitWorkspaces"].clone();
    }

    if let Some(remote_settings) = settings.get("remoteSettings") {
        state
            .network_link
            .lock()
            .map_err(|_| "Network Link state is unavailable.")?
            .set_network_enabled(remote_settings["enabled"] == true);
        state
            .remote_server
            .lock()
            .map_err(|_| "Remote server state is unavailable.".to_string())?
            .update_settings(remote_settings, app)?;
    }

    settings["settingsRevision"] = json!(revision);

    Ok(settings)
}

fn migrate_legacy_settings(app: &Host, current: Value) -> Result<Value, String> {
    if settings::has_workspace_path(&current) {
        return Ok(current);
    }

    let tauri_path = settings_path(app)?;
    let mut legacy_paths = settings::legacy_settings_paths();
    legacy_paths.retain(|path| path != &tauri_path);
    let Some(legacy) = settings::read_legacy_settings(&legacy_paths) else {
        return Ok(current);
    };
    let Some(merged) = settings::merge_missing_settings(&current, &legacy) else {
        return Ok(current);
    };

    let serialized = serde_json::to_string_pretty(&merged).map_err(|error| error.to_string())?;
    fs::write(tauri_path, serialized).map_err(|error| error.to_string())?;
    Ok(merged)
}

fn save_settings(
    app: &Host,
    state: &AppState,
    mut settings: Value,
    expected_revision: Option<u64>,
) -> Result<Value, String> {
    // The registry is owned by the workspace commands; renderer saves must not overwrite it.
    if let Some(document) = settings.as_object_mut() {
        document.remove("presenterOrbitWorkspaces");
    }
    let _settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    save_settings_locked(app, state, settings, expected_revision)
}

fn save_settings_locked(
    app: &Host,
    state: &AppState,
    settings: Value,
    expected_revision: Option<u64>,
) -> Result<Value, String> {
    if !settings.is_object() {
        return Err("Settings must be an object.".to_string());
    }

    let mut settings_revision = state
        .settings_revision
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    if let Some(expected) = expected_revision {
        settings_revision.require(expected)?;
    }
    let path = settings_path(app)?;
    let existing = read_settings(app)?;
    let previous_general = settings::normalize_general_settings_value(&existing["generalSettings"]);
    let mut merged = merge_objects(existing, settings)?;
    validate_hdr_settings(&merged)?;
    remote::validate_settings_value(&merged["remoteSettings"])?;
    normalize_native_settings(&mut merged)?;
    if let Some(document) = merged.as_object_mut() {
        document.remove("settingsRevision");
    }
    let serialized = serde_json::to_string_pretty(&merged).map_err(|error| error.to_string())?;
    app.desktop()
        .apply_preferences(&previous_general, &merged["generalSettings"])?;
    if let Err(error) = fs::write(path, serialized) {
        app.desktop()
            .apply_preferences(&merged["generalSettings"], &previous_general)
            .map_err(|rollback| format!("Could not save settings: {error}; could not restore desktop preferences: {rollback}"))?;
        return Err(error.to_string());
    }
    merged["settingsRevision"] = json!(settings_revision.advance());
    general_settings_options::set(&merged["generalSettings"]);
    drop(settings_revision);
    if let Some(remote_settings) = merged.get("remoteSettings") {
        state
            .network_link
            .lock()
            .map_err(|_| "Network Link state is unavailable.")?
            .set_network_enabled(remote_settings["enabled"] == true);
        let mut remote = state
            .remote_server
            .lock()
            .map_err(|_| "Remote server state is unavailable.".to_string())?;
        remote.update_settings(remote_settings, app)?;
    }
    app.emit("settings-changed", merged.clone())
        .map_err(|error| error.to_string())?;
    Ok(merged)
}

fn validate_hdr_settings(settings: &Value) -> Result<(), String> {
    native_hdr::validate_screens(
        &settings["screenConfig"],
        native_hdr::configured_bit_depth(&settings["generalSettings"])?,
        native_hdr::Platform::current(),
    )
}

fn normalize_native_settings(settings: &mut Value) -> Result<(), String> {
    let settings = settings
        .as_object_mut()
        .ok_or_else(|| "Settings must be an object.".to_string())?;
    let audio_settings = output_playback::normalize_audio_settings_value(
        settings.get("audioSettings").unwrap_or(&Value::Null),
    );
    settings.insert("audioSettings".to_string(), audio_settings);
    let remote_settings =
        remote::normalize_settings_value(settings.get("remoteSettings").unwrap_or(&Value::Null));
    settings.insert("remoteSettings".to_string(), remote_settings);
    let general_settings = settings::normalize_general_settings_value(
        settings.get("generalSettings").unwrap_or(&Value::Null),
    );
    settings.insert("generalSettings".to_string(), general_settings);
    let group_settings = settings::normalize_group_settings_value(
        settings.get("groupSettings").unwrap_or(&Value::Null),
    );
    settings.insert("groupSettings".to_string(), group_settings);
    let key_mappings =
        settings::normalize_key_mappings_value(settings.get("keyMappings").unwrap_or(&Value::Null));
    settings.insert("keyMappings".to_string(), key_mappings);
    let screen_config = settings::normalize_screen_config_value(
        settings.get("screenConfig").unwrap_or(&Value::Null),
    );
    output_canvas::validate_screen_config_map(&screen_config)?;
    settings.insert("screenConfig".to_string(), screen_config);
    Ok(())
}

fn merge_objects(existing: Value, incoming: Value) -> Result<Value, String> {
    let mut existing = existing
        .as_object()
        .cloned()
        .ok_or_else(|| "Stored settings must be an object.".to_string())?;
    let incoming = incoming
        .as_object()
        .ok_or_else(|| "Settings must be an object.".to_string())?;
    existing.extend(incoming.clone());
    Ok(Value::Object(existing))
}

fn mutate_group_settings(
    app: &Host,
    state: &AppState,
    requested_workspace: Value,
    mutation: Value,
    expected_settings_revision: u64,
    expected_workspace_revision: Value,
) -> Result<Value, String> {
    let workspace_path = group_settings_workspace_path(state, requested_workspace)?;
    let expected_workspace_revision =
        group_settings_workspace_revision(workspace_path.as_deref(), &expected_workspace_revision)?;
    let requested_mutation = mutation;
    let assignment = group_assignment_details(&requested_mutation)?;
    if assignment.is_some() && workspace_path.is_none() {
        return Err("Group assignment requires an active workspace.".to_string());
    }
    let _settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    let mut workspace_io = if let Some(path) = workspace_path.as_ref() {
        let expected_workspace_revision = expected_workspace_revision
            .ok_or_else(|| "Workspace base revision is required.".to_string())?;
        let mut workspace_io = lock_workspace_io(state)?;
        workspace_io.require_revision(path, expected_workspace_revision)?;
        Some(workspace_io)
    } else {
        None
    };
    let mut settings_revision = state
        .settings_revision
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    settings_revision.require(expected_settings_revision)?;

    let mut settings = read_settings(app)?;
    normalize_native_settings(&mut settings)?;
    let global_group_settings = native_group_settings_for_mutation(&settings);
    let workspace = workspace_path
        .as_ref()
        .map(|path| workspace::read_workspace(path))
        .transpose()?;
    let (mutation, workspace_group_settings) = plan_group_settings_mutation(
        &global_group_settings,
        workspace.as_ref(),
        &requested_mutation,
    )?;
    let settings_changed = mutation.changed;
    let deleted_group_id = mutation.deleted_group_id.clone();
    let created_group_id = mutation.created_group_id.clone();

    let mut saved_workspace = workspace;
    let mut workspace_changed = false;
    let workspace_group_settings_changed = workspace_path.is_some()
        && workspace_group_settings.as_ref().is_some_and(|next| {
            saved_workspace
                .as_ref()
                .and_then(|workspace| workspace.get("groupSettings"))
                .map_or(true, |current| current != next)
        });
    if let Some(group_id) = deleted_group_id.as_deref() {
        if workspace_path.is_some() {
            let workspace = saved_workspace
                .as_mut()
                .ok_or_else(|| "Workspace is unavailable for group deletion.".to_string())?;
            let cleaned = workspace::reset_group_references(
                workspace,
                group_id,
                requested_mutation
                    .get("slideLabels")
                    .ok_or_else(|| "Group deletion slide labels are required.".to_string())?,
            )?;
            workspace_changed = cleaned > 0;
        }
    }
    if let Some((item_id, slide_id)) = assignment.as_ref() {
        let group_id = created_group_id.as_deref().ok_or_else(|| {
            "Group assignment is only supported when creating a group.".to_string()
        })?;
        let workspace = saved_workspace
            .as_mut()
            .ok_or_else(|| "Workspace is unavailable for group assignment.".to_string())?;
        workspace_changed |=
            workspace::assign_group_reference(workspace, item_id, slide_id, group_id)?;
    }
    let changed = settings_changed || workspace_group_settings_changed || workspace_changed;
    let workspace_write = workspace_path.is_some()
        && (settings_changed || workspace_group_settings_changed || workspace_changed);
    let mut event_settings = settings;
    let persisted_group_settings;
    let mut workspace_patch = json!({
        "groupSettings": workspace_group_settings
            .clone()
            .unwrap_or_else(|| mutation.group_settings.clone())
    });
    if deleted_group_id.is_some() && workspace_path.is_some() {
        let workspace = saved_workspace
            .as_ref()
            .ok_or_else(|| "Workspace is unavailable for group deletion.".to_string())?;
        for field in ["groups", "library"] {
            if let Some(value) = workspace.get(field) {
                workspace_patch[field] = value.clone();
            }
        }
    }
    if assignment.is_some() && workspace_path.is_some() {
        let workspace = saved_workspace
            .as_ref()
            .ok_or_else(|| "Workspace is unavailable for group assignment.".to_string())?;
        if let Some(value) = workspace.get("library") {
            workspace_patch["library"] = value.clone();
        }
    }
    if changed {
        if settings_changed {
            event_settings["groupSettings"] = mutation.group_settings.clone();
            normalize_native_settings(&mut event_settings)?;
            if let Some(document) = event_settings.as_object_mut() {
                document.remove("settingsRevision");
            }
        }
        persisted_group_settings = event_settings
            .get("groupSettings")
            .cloned()
            .unwrap_or_else(|| settings::normalize_group_settings_value(&Value::Null));
        if settings_changed {
            let settings_file = settings_path(app)?;
            let serialized =
                serde_json::to_string_pretty(&event_settings).map_err(|error| error.to_string())?;
            saved_workspace = persist_settings_then_workspace(&settings_file, &serialized, || {
                workspace_path.as_ref().map_or_else(
                    || Ok(None),
                    |path| workspace::save_workspace(path, &workspace_patch).map(Some),
                )
            })?;
        } else if workspace_write {
            let path = workspace_path
                .as_ref()
                .ok_or_else(|| "Workspace path is unavailable for group deletion.".to_string())?;
            saved_workspace = workspace::save_workspace(path, &workspace_patch).map(Some)?;
        }
    } else {
        persisted_group_settings = event_settings
            .get("groupSettings")
            .cloned()
            .unwrap_or_else(|| settings::normalize_group_settings_value(&Value::Null));
    }

    let next_settings_revision = if settings_changed {
        settings_revision.advance()
    } else {
        settings_revision.current
    };
    let next_workspace_revision = if let Some(path) = workspace_path.as_ref() {
        let workspace_io = workspace_io
            .as_mut()
            .ok_or_else(|| "Workspace I/O state is unavailable.".to_string())?;
        Some(if workspace_write {
            workspace_io.advance_revision(path)
        } else {
            workspace_io.revision(path)
        })
    } else {
        None
    };

    let mut response = json!({
        "changed": changed,
        "groupSettings": persisted_group_settings,
        "settingsRevision": next_settings_revision,
    });
    if let Some(created_group_id) = mutation.created_group_id {
        response["createdGroupId"] = json!(created_group_id);
    }
    if let Some(deleted_group_id) = deleted_group_id {
        response["deletedGroupId"] = json!(deleted_group_id);
    }
    if let Some(revision) = next_workspace_revision {
        response["revision"] = json!(revision);
        response["workspace"] =
            saved_workspace.ok_or_else(|| "Saved workspace is unavailable.".to_string())?;
    }

    drop(workspace_io);
    drop(settings_revision);
    // Keep the settings-operation guard through the notification so a
    // concurrent settings save cannot publish a newer global event first.
    if settings_changed {
        event_settings["settingsRevision"] = json!(next_settings_revision);
        app.emit("settings-changed", event_settings)
            .map_err(|error| error.to_string())?;
    }
    Ok(response)
}

fn mutate_key_mappings(
    app: &Host,
    state: &AppState,
    requested_workspace: Value,
    mutation: Value,
    expected_settings_revision: u64,
    expected_workspace_revision: Value,
) -> Result<Value, String> {
    let workspace_path = group_settings_workspace_path(state, requested_workspace)?;
    let expected_workspace_revision =
        group_settings_workspace_revision(workspace_path.as_deref(), &expected_workspace_revision)?;
    let _settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    let mut workspace_io = if let Some(path) = workspace_path.as_ref() {
        let expected = expected_workspace_revision
            .ok_or_else(|| "Workspace base revision is required.".to_string())?;
        let mut workspace_io = lock_workspace_io(state)?;
        workspace_io.require_revision(path, expected)?;
        Some(workspace_io)
    } else {
        None
    };
    let mut settings_revision = state
        .settings_revision
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    settings_revision.require(expected_settings_revision)?;

    let mut event_settings = read_settings(app)?;
    normalize_native_settings(&mut event_settings)?;
    let global_current = event_settings
        .get("keyMappings")
        .cloned()
        .unwrap_or_else(|| settings::normalize_key_mappings_value(&Value::Null));
    let workspace = workspace_path
        .as_ref()
        .map(|path| workspace::read_workspace(path))
        .transpose()?;
    let workspace_key_mappings = workspace
        .as_ref()
        .map(|workspace| workspace.get("keyMappings").unwrap_or(&Value::Null));
    let mutation_result =
        settings::mutate_key_mapping_scopes(&global_current, workspace_key_mappings, &mutation)?;
    let changed = mutation_result.global_changed || mutation_result.workspace_changed;
    let settings_changed = mutation_result.global_changed;
    let workspace_write = mutation_result.workspace_changed;
    let key_mappings = mutation_result.effective;
    let global_key_mappings = mutation_result.global;
    let workspace_next_key_mappings = mutation_result.workspace;
    let mut saved_workspace = workspace;

    if changed {
        if settings_changed {
            event_settings["keyMappings"] = global_key_mappings;
            normalize_native_settings(&mut event_settings)?;
            if let Some(document) = event_settings.as_object_mut() {
                document.remove("settingsRevision");
            }
            let settings_file = settings_path(app)?;
            let serialized =
                serde_json::to_string_pretty(&event_settings).map_err(|error| error.to_string())?;
            saved_workspace = persist_settings_then_workspace(&settings_file, &serialized, || {
                if workspace_write {
                    let path = workspace_path
                        .as_ref()
                        .ok_or_else(|| "Workspace path is unavailable.".to_string())?;
                    let mappings = workspace_next_key_mappings
                        .as_ref()
                        .ok_or_else(|| "Workspace key mappings are unavailable.".to_string())?;
                    workspace::save_workspace(path, &json!({ "keyMappings": mappings })).map(Some)
                } else {
                    Ok(saved_workspace.clone())
                }
            })?;
        } else if workspace_write {
            let path = workspace_path
                .as_ref()
                .ok_or_else(|| "Workspace path is unavailable.".to_string())?;
            let mappings = workspace_next_key_mappings
                .as_ref()
                .ok_or_else(|| "Workspace key mappings are unavailable.".to_string())?;
            saved_workspace =
                workspace::save_workspace(path, &json!({ "keyMappings": mappings })).map(Some)?;
        }
    }

    let next_settings_revision = if settings_changed {
        settings_revision.advance()
    } else {
        settings_revision.current
    };
    let next_workspace_revision = if let Some(path) = workspace_path.as_ref() {
        let workspace_io = workspace_io
            .as_mut()
            .ok_or_else(|| "Workspace I/O state is unavailable.".to_string())?;
        Some(if workspace_write {
            workspace_io.advance_revision(path)
        } else {
            workspace_io.revision(path)
        })
    } else {
        None
    };
    let mut response = json!({
        "changed": changed,
        "keyMappings": key_mappings,
        "settingsRevision": next_settings_revision,
    });
    if let Some(revision) = next_workspace_revision {
        response["revision"] = json!(revision);
        response["workspace"] =
            saved_workspace.ok_or_else(|| "Saved workspace is unavailable.".to_string())?;
    }

    drop(workspace_io);
    drop(settings_revision);
    if settings_changed {
        event_settings["settingsRevision"] = json!(next_settings_revision);
        app.emit("settings-changed", event_settings)
            .map_err(|error| error.to_string())?;
    }
    Ok(response)
}

fn mutate_screen_config(
    app: &Host,
    state: &AppState,
    requested_workspace: Value,
    mutation: Value,
    expected_settings_revision: u64,
    expected_workspace_revision: Value,
) -> Result<Value, String> {
    let workspace_path = group_settings_workspace_path(state, requested_workspace)?;
    let expected_workspace_revision =
        group_settings_workspace_revision(workspace_path.as_deref(), &expected_workspace_revision)?;
    let mutation_type = mutation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Screen configuration mutation type is required.".to_string())?;
    if matches!(mutation_type, "setStageAssignment" | "clearStageAssignment")
        && workspace_path.is_none()
    {
        return Err("Stage screen assignment requires an active workspace.".to_string());
    }
    if mutation_type == "update"
        && mutation.get("stageLayoutId").is_some()
        && workspace_path.is_none()
    {
        return Err("Stage screen assignment requires an active workspace.".to_string());
    }

    let _settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    let mut workspace_io = if let Some(path) = workspace_path.as_ref() {
        let expected_workspace_revision = expected_workspace_revision
            .ok_or_else(|| "Workspace base revision is required.".to_string())?;
        let mut workspace_io = lock_workspace_io(state)?;
        workspace_io.require_revision(path, expected_workspace_revision)?;
        Some(workspace_io)
    } else {
        None
    };
    let mut settings_revision = state
        .settings_revision
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    settings_revision.require(expected_settings_revision)?;

    let mut settings = read_settings(app)?;
    normalize_native_settings(&mut settings)?;
    let global_screen_config = native_screen_config_for_mutation(&settings);
    let mut saved_workspace = workspace_path
        .as_ref()
        .map(|path| workspace::read_workspace(path))
        .transpose()?;

    let mut global_next = global_screen_config.clone();
    let mut workspace_screen_next = None;
    let mut global_changed = false;
    let mut workspace_screen_changed = false;
    let mut created_screen_id = None;
    let mut deleted_screen_id = None;
    let mut workspace_changed = false;
    let runtime_screen_config;

    if matches!(mutation_type, "setStageAssignment" | "clearStageAssignment") {
        let workspace = saved_workspace
            .as_mut()
            .ok_or_else(|| "Workspace is unavailable for stage assignment.".to_string())?;
        let effective = effective_screen_config_for_mutation(workspace, &global_screen_config);
        validate_stage_screen_assignment_mutation(&effective, workspace, &mutation)?;
        let screen_id = required_screen_mutation_field(&mutation, "screenId")?;
        let layout_id = if mutation_type == "setStageAssignment" {
            Some(required_screen_mutation_field(&mutation, "layoutId")?)
        } else {
            None
        };
        workspace_changed =
            workspace::mutate_stage_screen_assignment(workspace, &screen_id, layout_id.as_deref())?;
        runtime_screen_config = effective;
    } else {
        let live_screen_config = lock_output_snapshot_state(&state.output_snapshot)?
            .cache
            .as_ref()
            .and_then(|cached| cached.snapshot.get("screenConfig"))
            .cloned();
        let (global_result, workspace_next, screen_changed) = plan_screen_config_mutation(
            &global_screen_config,
            saved_workspace.as_ref(),
            &mutation,
            live_screen_config.as_ref(),
        )?;
        let configured_bit_depth = native_hdr::configured_bit_depth(&settings["generalSettings"])?;
        for screens in std::iter::once(&global_result.screen_config).chain(
            workspace_next.as_ref().filter(|screens| {
                screens
                    .as_object()
                    .is_some_and(|screens| !screens.is_empty())
            }),
        ) {
            native_hdr::validate_screens(
                screens,
                configured_bit_depth,
                native_hdr::Platform::current(),
            )?;
        }
        global_next = global_result.screen_config;
        global_changed = if saved_workspace.is_some() {
            global_next != global_screen_config
        } else {
            global_result.changed
        };
        created_screen_id = global_result.created_screen_id;
        deleted_screen_id = global_result.deleted_screen_id;
        workspace_screen_next = workspace_next;
        workspace_screen_changed = screen_changed;
        if let Some(workspace) = saved_workspace.as_mut() {
            // Keep an empty override empty for a no-op. The effective map is
            // normalized for planning, but materializing it here would make
            // the response claim a workspace write that never reached disk.
            if workspace_screen_changed {
                if let Some(next) = workspace_screen_next.as_ref() {
                    workspace["screenConfig"] = next.clone();
                }
            }
            if mutation_type == "update" && mutation.get("stageLayoutId").is_some() {
                let screen_id = required_screen_mutation_field(&mutation, "screenId")?;
                let stage_mutation = if mutation.get("stageLayoutId").is_some_and(Value::is_null) {
                    json!({ "type": "clearStageAssignment", "screenId": screen_id })
                } else {
                    json!({
                        "type": "setStageAssignment",
                        "screenId": screen_id,
                        "layoutId": required_screen_mutation_field(&mutation, "stageLayoutId")?
                    })
                };
                let effective = effective_screen_config_for_mutation(workspace, &global_next);
                validate_stage_screen_assignment_mutation(&effective, workspace, &stage_mutation)?;
                let layout_id = stage_mutation.get("layoutId").and_then(Value::as_str);
                workspace_changed |=
                    workspace::mutate_stage_screen_assignment(workspace, &screen_id, layout_id)?;
            }
            if let Some(screen_id) = deleted_screen_id.as_deref() {
                // Screen assignments live in the stage document. Delete any
                // matching entry, even for an audience screen, to clean up
                // legacy/orphaned references atomically with the delete.
                workspace_changed |=
                    workspace::mutate_stage_screen_assignment(workspace, screen_id, None)?;
                workspace_changed |= workspace::remove_audience_look_screen(workspace, screen_id)?;
            }
            workspace_changed |= workspace_screen_changed;
            runtime_screen_config = effective_screen_config_for_mutation(workspace, &global_next);
        } else {
            runtime_screen_config = global_next.clone();
        }
    }

    let mut event_settings = settings.clone();
    let mut workspace_patch = Value::Object(serde_json::Map::new());
    if workspace_screen_changed {
        workspace_patch["screenConfig"] = workspace_screen_next
            .clone()
            .ok_or_else(|| "Workspace screen configuration is unavailable.".to_string())?;
    }
    if workspace_changed {
        let workspace = saved_workspace.as_ref().ok_or_else(|| {
            "Workspace is unavailable for screen configuration mutation.".to_string()
        })?;
        workspace_patch["stageScreenAssignments"] = workspace
            .get("stageScreenAssignments")
            .cloned()
            .ok_or_else(|| "Workspace stage screen assignments are unavailable.".to_string())?;
        workspace_patch["audienceLooks"] = workspace
            .get("audienceLooks")
            .cloned()
            .ok_or_else(|| "Workspace audience looks are unavailable.".to_string())?;
    }
    let workspace_write = workspace_path.is_some() && workspace_changed;
    let changed = global_changed || workspace_changed;
    let mut persisted_workspace = saved_workspace;
    let mut hidden_outputs = if changed && mutation_requires_runtime_apply(&mutation) {
        hide_disabled_display_outputs(app, state, &runtime_screen_config)?
    } else {
        HiddenDisplayOutputs::default()
    };

    if changed {
        if global_changed {
            event_settings["screenConfig"] = global_next.clone();
            normalize_native_settings(&mut event_settings)?;
            if let Some(document) = event_settings.as_object_mut() {
                document.remove("settingsRevision");
            }
        }
        if global_changed {
            let settings_file = settings_path(app)?;
            let serialized =
                serde_json::to_string_pretty(&event_settings).map_err(|error| error.to_string())?;
            let saved_workspace_result =
                persist_settings_then_workspace(&settings_file, &serialized, || {
                    if workspace_write {
                        let path = workspace_path.as_ref().ok_or_else(|| {
                            "Workspace path is unavailable for screen configuration mutation."
                                .to_string()
                        })?;
                        workspace::save_workspace(path, &workspace_patch).map(Some)
                    } else {
                        Ok(None)
                    }
                })?;
            if workspace_write {
                persisted_workspace = saved_workspace_result;
            }
        } else if workspace_write {
            let path = workspace_path.as_ref().ok_or_else(|| {
                "Workspace path is unavailable for screen configuration mutation.".to_string()
            })?;
            persisted_workspace = workspace::save_workspace(path, &workspace_patch).map(Some)?;
        }
    }
    hidden_outputs.persisted();

    let next_settings_revision = if global_changed {
        settings_revision.advance()
    } else {
        settings_revision.current
    };
    let next_workspace_revision = if let Some(path) = workspace_path.as_ref() {
        let workspace_io = workspace_io
            .as_mut()
            .ok_or_else(|| "Workspace I/O state is unavailable.".to_string())?;
        Some(if workspace_write {
            workspace_io.advance_revision(path)
        } else {
            workspace_io.revision(path)
        })
    } else {
        None
    };

    let mut runtime_error = if mutation_requires_runtime_apply(&mutation)
        && (global_changed || workspace_screen_changed)
    {
        apply_runtime_screen_config(app, state, &runtime_screen_config)
            .err()
            .map(|error| format!("Native screen output synchronization failed: {error}"))
    } else {
        None
    };

    let effective_screen_config = if workspace_path.is_some() {
        persisted_workspace
            .as_ref()
            .map(|workspace| effective_screen_config_for_mutation(workspace, &global_next))
            .unwrap_or_else(|| runtime_screen_config.clone())
    } else {
        global_next.clone()
    };
    let mut response = json!({
        "changed": changed,
        "screenConfig": effective_screen_config,
        "settingsRevision": next_settings_revision,
    });
    if let Some(created_screen_id) = created_screen_id {
        response["createdScreenId"] = json!(created_screen_id);
    }
    if let Some(deleted_screen_id) = deleted_screen_id {
        response["deletedScreenId"] = json!(deleted_screen_id);
    }
    if let Some(revision) = next_workspace_revision {
        response["revision"] = json!(revision);
        response["workspace"] =
            persisted_workspace.ok_or_else(|| "Saved workspace is unavailable.".to_string())?;
        response["stageScreenAssignments"] = response["workspace"]
            .get("stageScreenAssignments")
            .cloned()
            .unwrap_or_else(|| json!([]));
    }

    drop(workspace_io);
    drop(settings_revision);
    if changed {
        let mut screen_config_event = json!({
            "workspacePath": workspace_path
                .as_ref()
                .map(|path| path.to_string_lossy().to_string()),
            "screenConfig": response
                .get("screenConfig")
                .cloned()
                .unwrap_or_else(|| json!({})),
            "settingsRevision": next_settings_revision,
        });
        if let Some(document) = screen_config_event.as_object_mut() {
            if let Some(workspace) = response.get("workspace") {
                document.insert(
                    "workspaceScreenConfig".to_string(),
                    workspace
                        .get("screenConfig")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                );
                document.insert(
                    "stageScreenAssignments".to_string(),
                    workspace
                        .get("stageScreenAssignments")
                        .cloned()
                        .unwrap_or_else(|| json!([])),
                );
                document.insert(
                    "audienceLooks".to_string(),
                    workspace
                        .get("audienceLooks")
                        .cloned()
                        .ok_or("Workspace audience looks are unavailable.")?,
                );
            }
            if let Some(revision) = next_workspace_revision {
                document.insert("revision".to_string(), json!(revision));
            }
        }
        if let Err(error) = app.emit("screen-config-changed", screen_config_event) {
            if runtime_error.is_none() {
                runtime_error = Some(format!("Screen configuration notification failed: {error}"));
            }
        }
    }
    if global_changed {
        event_settings["settingsRevision"] = json!(next_settings_revision);
        if let Err(error) = app.emit("settings-changed", event_settings) {
            if runtime_error.is_none() {
                runtime_error = Some(format!("Settings change notification failed: {error}"));
            }
        }
    }
    drop(_settings_operation);
    if let Some(runtime_error) = runtime_error {
        response["runtimeError"] = json!(runtime_error);
    }
    Ok(response)
}

fn native_screen_config_for_mutation(settings: &Value) -> Value {
    settings
        .get("screenConfig")
        .cloned()
        .unwrap_or_else(|| settings::normalize_screen_config_value(&Value::Null))
}

fn mutation_requires_runtime_apply(mutation: &Value) -> bool {
    match mutation.get("type").and_then(Value::as_str) {
        Some("add" | "delete" | "toggleGroup") => true,
        Some("update") => mutation
            .get("patch")
            .and_then(Value::as_object)
            .is_some_and(|patch| {
                patch.keys().any(|key| {
                    matches!(
                        key.as_str(),
                        "enabled"
                            | "outputLayout"
                            | "keyFill"
                            | "targetType"
                            | "targetId"
                            | "ndiName"
                            | "ndiResolution"
                            | "ndiWidth"
                            | "ndiHeight"
                            | "ndiFramerate"
                            | "ndiAlphaMode"
                            | "outputFormat"
                            | "videoDelay"
                            | "label"
                    )
                })
            }),
        _ => false,
    }
}

fn effective_screen_config_for_mutation(workspace: &Value, global: &Value) -> Value {
    let explicit = workspace
        .get("screenConfig")
        .cloned()
        .unwrap_or(Value::Null);
    if explicit
        .as_object()
        .is_some_and(|screens| !screens.is_empty())
    {
        settings::normalize_screen_config_value(&explicit)
    } else {
        global.clone()
    }
}

fn plan_screen_config_mutation(
    global: &Value,
    workspace: Option<&Value>,
    mutation: &Value,
    live_screen_config: Option<&Value>,
) -> Result<(settings::ScreenConfigMutationResult, Option<Value>, bool), String> {
    let persisted = workspace
        .map(|workspace| effective_screen_config_for_mutation(workspace, global))
        .unwrap_or_else(|| global.clone());
    let mut current = persisted.clone();
    // Cues restore live layers through the output snapshot, independently of
    // persisted screen settings. Apply the intent to that live layer state.
    if let Some(live_screens) = live_screen_config.and_then(Value::as_object) {
        for (screen_id, screen) in current
            .as_object_mut()
            .ok_or_else(|| "Screen configuration must be an object.".to_string())?
        {
            if mutation["type"] == "setLookLayer"
                && mutation["screenId"].as_str() != Some(screen_id.as_str())
            {
                continue;
            }
            if let Some(layers) = live_screens
                .get(screen_id)
                .and_then(|screen| screen.get("lookLayers"))
            {
                screen["lookLayers"] = layers.clone();
            }
        }
    }
    let mut result = settings::mutate_screen_config_value(&current, mutation)?;
    result.changed = result.screen_config != persisted;
    let workspace_changed = workspace.is_some_and(|workspace| {
        let explicit = workspace
            .get("screenConfig")
            .and_then(Value::as_object)
            .is_some_and(|screens| !screens.is_empty());
        if explicit {
            workspace
                .get("screenConfig")
                .is_some_and(|stored| stored != &result.screen_config)
        } else {
            result.changed
        }
    });
    let workspace_next = workspace.map(|workspace| {
        if workspace_changed {
            result.screen_config.clone()
        } else {
            // Return the persisted override for a no-op instead of the
            // normalized effective map. Callers use this snapshot in their
            // canonical response, so an empty override must remain empty.
            workspace
                .get("screenConfig")
                .cloned()
                .unwrap_or_else(|| json!({}))
        }
    });
    Ok((
        settings::ScreenConfigMutationResult {
            changed: result.changed,
            screen_config: result.screen_config.clone(),
            created_screen_id: result.created_screen_id,
            deleted_screen_id: result.deleted_screen_id,
        },
        workspace_next,
        workspace_changed,
    ))
}

fn required_screen_mutation_field(mutation: &Value, field: &str) -> Result<String, String> {
    mutation
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Screen configuration mutation {field} must not be empty."))
}

fn validate_stage_screen_assignment_mutation(
    screen_config: &Value,
    workspace: &Value,
    mutation: &Value,
) -> Result<(), String> {
    let mutation_type = mutation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "Screen configuration mutation type is required.".to_string())?;
    let allowed = if mutation_type == "setStageAssignment" {
        ["type", "screenId", "layoutId"].as_slice()
    } else {
        ["type", "screenId"].as_slice()
    };
    if let Some(field) = mutation.as_object().and_then(|mutation| {
        mutation
            .keys()
            .find(|field| !allowed.contains(&field.as_str()))
    }) {
        return Err(format!(
            "Unsupported stage screen assignment field '{field}'."
        ));
    }
    let screen_id = required_screen_mutation_field(mutation, "screenId")?;
    let screen = screen_config
        .get(&screen_id)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("Screen configuration '{screen_id}' was not found."))?;
    if screen.get("type").and_then(Value::as_str) != Some("stage") {
        return Err(format!(
            "Stage screen assignment requires a stage screen; '{screen_id}' is not a stage."
        ));
    }
    if mutation_type == "setStageAssignment" {
        let layout_id = required_screen_mutation_field(mutation, "layoutId")?;
        let layouts = workspace
            .get("stageLayouts")
            .and_then(Value::as_array)
            .ok_or_else(|| "Workspace stage layouts must be an array.".to_string())?;
        if !layouts
            .iter()
            .any(|layout| layout.get("id").and_then(Value::as_str) == Some(layout_id.as_str()))
        {
            return Err(format!(
                "Stage screen assignment layout '{layout_id}' was not found."
            ));
        }
    }
    Ok(())
}

fn group_settings_workspace_path(
    state: &AppState,
    requested: Value,
) -> Result<Option<PathBuf>, String> {
    match requested {
        Value::Null => Ok(None),
        Value::String(path) if path.trim().is_empty() => {
            Err("Workspace path must not be empty.".to_string())
        }
        Value::String(path) => Ok(Some(active_workspace(state, Value::String(path))?)),
        _ => Err("Workspace path must be a string or null.".to_string()),
    }
}

fn group_settings_workspace_revision(
    workspace_path: Option<&Path>,
    requested: &Value,
) -> Result<Option<u64>, String> {
    match workspace_path {
        Some(_) => requested
            .as_u64()
            .map(Some)
            .ok_or_else(|| "Workspace base revision is required.".to_string()),
        None if requested.is_null() => Ok(None),
        None => Err(
            "Workspace base revision is only valid when a workspace path is supplied.".to_string(),
        ),
    }
}

fn group_assignment_details(mutation: &Value) -> Result<Option<(String, String)>, String> {
    let Some(assignment) = mutation.get("assignment") else {
        return Ok(None);
    };
    let assignment = assignment
        .as_object()
        .ok_or_else(|| "Group assignment must be an object.".to_string())?;
    let item_id = assignment
        .get("itemId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Group assignment item id is required.".to_string())?;
    let slide_id = assignment
        .get("slideId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Group assignment slide id is required.".to_string())?;
    Ok(Some((item_id.to_string(), slide_id.to_string())))
}

fn native_group_settings_for_mutation(settings: &Value) -> Value {
    settings
        .get("groupSettings")
        .cloned()
        .unwrap_or_else(|| settings::normalize_group_settings_value(&Value::Null))
}

fn effective_group_settings_for_mutation(workspace: &Value, global: &Value) -> Value {
    let explicit = workspace
        .get("groupSettings")
        .cloned()
        .unwrap_or(Value::Null);
    if explicit
        .as_object()
        .is_some_and(|groups| !groups.is_empty())
    {
        return settings::normalize_workspace_group_settings_value(&explicit);
    }

    let mut resolved = workspace.clone();
    workspace::merge_missing_workspace_group_settings(&mut resolved, global);
    let merged = resolved
        .get("groupSettings")
        .cloned()
        .unwrap_or(Value::Null);
    if merged.as_object().is_some_and(|groups| !groups.is_empty()) {
        settings::normalize_workspace_group_settings_value(&merged)
    } else {
        global.clone()
    }
}

fn plan_group_settings_mutation(
    global_group_settings: &Value,
    workspace: Option<&Value>,
    requested_mutation: &Value,
) -> Result<(settings::GroupSettingsMutationResult, Option<Value>), String> {
    let mutation_type = requested_mutation
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if mutation_type == "delete" {
        return plan_group_settings_delete(global_group_settings, workspace, requested_mutation);
    }
    if mutation_type == "add" {
        if let Some(workspace) = workspace {
            return plan_group_settings_add(global_group_settings, workspace, requested_mutation);
        }
    }
    if workspace.is_some() && matches!(mutation_type, "rename" | "update") {
        return plan_group_settings_update(
            global_group_settings,
            workspace.expect("workspace checked above"),
            requested_mutation,
        );
    }

    let mutation =
        settings::mutate_group_settings_value(global_group_settings, requested_mutation)?;
    Ok((mutation, None))
}

fn plan_group_settings_delete(
    global_group_settings: &Value,
    workspace: Option<&Value>,
    requested_mutation: &Value,
) -> Result<(settings::GroupSettingsMutationResult, Option<Value>), String> {
    let current_group_settings = workspace
        .map(|workspace| effective_group_settings_for_mutation(workspace, global_group_settings))
        .unwrap_or_else(|| global_group_settings.clone());
    let global_mutation =
        settings::mutate_group_settings_value(global_group_settings, requested_mutation)?;
    let workspace_group_settings = workspace
        .map(|_| {
            settings::mutate_group_settings_value(&current_group_settings, requested_mutation)
                .map(|mutation| mutation.group_settings)
        })
        .transpose()?;
    Ok((global_mutation, workspace_group_settings))
}

fn plan_group_settings_add(
    global_group_settings: &Value,
    workspace: &Value,
    requested_mutation: &Value,
) -> Result<(settings::GroupSettingsMutationResult, Option<Value>), String> {
    let global = settings::normalize_group_settings_value(global_group_settings);
    let workspace_current = workspace_group_settings_for_mutation(workspace, &global);
    let allocation_input = merged_group_settings_for_allocation(&global, &workspace_current);
    let allocation = settings::mutate_group_settings_value(&allocation_input, requested_mutation)?;
    let created_group_id = allocation
        .created_group_id
        .clone()
        .ok_or_else(|| "Group add did not create a group.".to_string())?;
    let created_group = allocation
        .group_settings
        .get(&created_group_id)
        .cloned()
        .ok_or_else(|| "Group add did not return its created group.".to_string())?;
    let global_next = insert_group_setting(&global, &created_group_id, &created_group)?;
    let workspace_next =
        insert_group_setting(&workspace_current, &created_group_id, &created_group)?;
    Ok((
        settings::GroupSettingsMutationResult {
            changed: global_next != global,
            group_settings: global_next,
            created_group_id: Some(created_group_id),
            deleted_group_id: None,
        },
        Some(workspace_next),
    ))
}

fn plan_group_settings_update(
    global_group_settings: &Value,
    workspace: &Value,
    requested_mutation: &Value,
) -> Result<(settings::GroupSettingsMutationResult, Option<Value>), String> {
    let global = settings::normalize_group_settings_value(global_group_settings);
    let workspace_current = workspace_group_settings_for_mutation(workspace, &global);
    let group_id = requested_group_id(requested_mutation)?;
    let global_has_target = global
        .as_object()
        .is_some_and(|groups| groups.contains_key(&group_id));
    let workspace_has_target = workspace_current
        .as_object()
        .is_some_and(|groups| groups.contains_key(&group_id));
    if !global_has_target && !workspace_has_target {
        return Err(format!("Group setting '{group_id}' was not found."));
    }

    let mutation_type = requested_mutation
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let global_next;
    let workspace_next;
    if mutation_type == "rename" {
        // Resolve the final name once against the union of both scopes. This
        // prevents a colliding workspace name from causing a different
        // accepted name than the global scope receives.
        let allocation_input = merged_group_settings_for_allocation(&global, &workspace_current);
        let canonical =
            settings::mutate_group_settings_value(&allocation_input, requested_mutation)?;
        let canonical_name = canonical
            .group_settings
            .get(&group_id)
            .and_then(|group| group.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Group setting '{group_id}' was not found."))?;
        global_next = update_group_name(&global, &group_id, canonical_name, global_has_target)?;
        workspace_next = update_group_name(
            &workspace_current,
            &group_id,
            canonical_name,
            workspace_has_target,
        )?;
    } else {
        // Color/key patches have no name allocation. Apply the same validated
        // patch independently to each scope that owns the target so unrelated
        // entries and scope-specific fields remain intact.
        let allocation_input = merged_group_settings_for_allocation(&global, &workspace_current);
        settings::mutate_group_settings_value(&allocation_input, requested_mutation)?;
        global_next =
            mutate_group_settings_scope(&global, requested_mutation, global_has_target, true)?;
        workspace_next = mutate_group_settings_scope(
            &workspace_current,
            requested_mutation,
            workspace_has_target,
            false,
        )?;
    }
    Ok((
        settings::GroupSettingsMutationResult {
            changed: global_next != global,
            group_settings: global_next,
            created_group_id: None,
            deleted_group_id: None,
        },
        Some(workspace_next),
    ))
}

fn update_group_name(
    current: &Value,
    group_id: &str,
    name: &str,
    has_target: bool,
) -> Result<Value, String> {
    let mut groups = current
        .as_object()
        .cloned()
        .ok_or_else(|| "Group settings must be an object.".to_string())?;
    if has_target {
        let group = groups
            .get_mut(group_id)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("Group setting '{group_id}' was not found."))?;
        group.insert("name".to_string(), Value::String(name.to_string()));
    }
    Ok(Value::Object(groups))
}

fn workspace_group_settings_for_mutation(workspace: &Value, global: &Value) -> Value {
    let explicit = workspace
        .get("groupSettings")
        .cloned()
        .unwrap_or(Value::Null);
    if explicit
        .as_object()
        .is_some_and(|groups| !groups.is_empty())
    {
        settings::normalize_workspace_group_settings_value(&explicit)
    } else {
        settings::normalize_workspace_group_settings_value(&effective_group_settings_for_mutation(
            workspace, global,
        ))
    }
}

fn merged_group_settings_for_allocation(global: &Value, workspace: &Value) -> Value {
    let mut merged = global.as_object().cloned().unwrap_or_default();
    if let Some(workspace) = workspace.as_object() {
        for (group_id, group) in workspace {
            if let Some(existing) = merged.get(group_id) {
                if existing == group {
                    continue;
                }
                let mut suffix = 2;
                loop {
                    let candidate = format!("{group_id}-workspace-{suffix}");
                    if !merged.contains_key(&candidate) {
                        merged.insert(candidate, group.clone());
                        break;
                    }
                    suffix += 1;
                }
            } else {
                merged.insert(group_id.clone(), group.clone());
            }
        }
    }
    Value::Object(merged)
}

fn insert_group_setting(current: &Value, group_id: &str, group: &Value) -> Result<Value, String> {
    let mut groups = current
        .as_object()
        .cloned()
        .ok_or_else(|| "Group settings must be an object.".to_string())?;
    groups.insert(group_id.to_string(), group.clone());
    Ok(Value::Object(groups))
}

fn requested_group_id(mutation: &Value) -> Result<String, String> {
    mutation
        .get("groupId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Group settings mutation group id must not be empty.".to_string())
}

fn mutate_group_settings_scope(
    current: &Value,
    requested_mutation: &Value,
    has_target: bool,
    global_scope: bool,
) -> Result<Value, String> {
    let normalized = if global_scope {
        settings::normalize_group_settings_value(current)
    } else {
        settings::normalize_workspace_group_settings_value(current)
    };
    if has_target {
        return settings::mutate_group_settings_value(&normalized, requested_mutation)
            .map(|mutation| mutation.group_settings);
    }

    // Validate the same mutation against a temporary target so a request that
    // targets the other scope cannot bypass field/name validation here.
    let group_id = requested_group_id(requested_mutation)?;
    let mut validation = normalized
        .as_object()
        .cloned()
        .ok_or_else(|| "Group settings must be an object.".to_string())?;
    validation.insert(
        group_id,
        json!({ "name": "Temporary Group", "color": "#0f7db7", "key": "" }),
    );
    settings::mutate_group_settings_value(&Value::Object(validation), requested_mutation)?;
    Ok(normalized)
}

fn apply_audio_settings(
    app: &Host,
    state: &AppState,
    audio_settings: Value,
    expected_revision: Option<u64>,
) -> Result<Value, String> {
    let _settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    if !audio_settings.is_object() {
        return Err("Audio settings must be an object.".to_string());
    }
    let audio_settings = output_playback::normalize_audio_settings_value(&audio_settings);
    let saved = save_settings_locked(
        app,
        state,
        json!({ "audioSettings": audio_settings.clone() }),
        expected_revision,
    )?;
    let saved_revision = saved.get("settingsRevision").and_then(Value::as_u64);
    let audio_settings = saved
        .get("audioSettings")
        .cloned()
        .unwrap_or(audio_settings);
    let mut snapshot_state = lock_output_snapshot_state(&state.output_snapshot)?;
    // Re-run the active output payloads so the native mixer applies device,
    // gain, delay, and codec changes immediately. Inactive outputs are left
    // untouched until they become playback targets.
    let target_ids = active_native_audio_source_ids(state)?;
    if !target_ids.is_empty() {
        let stored = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?;
        let mut updates = serde_json::Map::new();
        for source_id in &target_ids {
            if let Some(mut content) = stored.get(source_id).cloned() {
                content["audioSettings"] = audio_settings.clone();
                updates.insert(source_id.clone(), content);
            }
        }
        drop(stored);
        if !updates.is_empty() {
            update_canonical_output_content(state, updates)?;
            snapshot_state.invalidate_cache();
        }
    }
    Ok(json!({
        "audioSettings": audio_settings,
        "settingsRevision": saved_revision
    }))
}

fn control_playback(app: &Host, state: &AppState, value: Value) -> Result<Value, String> {
    if let Some(path) = value.get("workspacePath") {
        active_workspace(state, path.clone())?;
    }
    if value["action"] == "marker" {
        return remote_playback_commands::execute(app, state, &value);
    }
    let command = output_playback::PlaybackCommand::from_value(&value)?;
    if matches!(
        command.action,
        output_playback::PlaybackAction::Toggle
            | output_playback::PlaybackAction::Pause
            | output_playback::PlaybackAction::Play
    ) {
        return control_coordinated_transport(app, state, command);
    }
    control_coordinated_playback_clock(state, command)
}

struct PlaybackSourceMutation {
    payloads: HashMap<String, Value>,
    changed: bool,
    targeted: bool,
}

fn apply_playback_command_to_sources(
    state: &AppState,
    command: output_playback::PlaybackCommand,
) -> Result<PlaybackSourceMutation, String> {
    let mut targets = main_transport_source_ids(state)?;
    // A source nothing displays yet still has a clock: the output process holds the program content of a UI
    // whose preview does not count as an output here.
    targets.extend(
        state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .iter()
            .filter(|(_, content)| is_main_transport_content(content))
            .map(|(id, _)| id.clone()),
    );
    apply_playback_command_to_target_sources(state, command, &targets)
}

fn apply_playback_command_to_target_sources(
    state: &AppState,
    command: output_playback::PlaybackCommand,
    targets: &[String],
) -> Result<PlaybackSourceMutation, String> {
    let mut unique_sources = std::collections::HashSet::new();
    let mut payloads = HashMap::new();
    let mut changed = false;
    {
        let mut playback = state
            .output_playback
            .lock()
            .map_err(|_| "Output playback state is unavailable.".to_string())?;
        let now = std::time::Instant::now();
        for source_screen_id in targets {
            if !unique_sources.insert(source_screen_id.clone()) {
                continue;
            }
            if let Some(outcome) = playback.apply_command(source_screen_id, command, now) {
                changed |= outcome.changed;
                payloads.insert(source_screen_id.clone(), outcome.payload);
            }
        }
    }
    Ok(PlaybackSourceMutation {
        payloads,
        changed,
        targeted: !unique_sources.is_empty(),
    })
}

fn propagate_playback_source_mutation(
    state: &AppState,
    command: output_playback::PlaybackCommand,
    changed: bool,
    payloads: &mut HashMap<String, Value>,
) -> Result<(), String> {
    {
        let frozen = state
            .output_freeze
            .read()
            .map_err(|_| "Output freeze state is unavailable.")?;
        if !*frozen && playback_clock_requires_audio_reset(command, changed) {
            {
                let mut native_audio = state
                    .native_audio
                    .lock()
                    .map_err(|_| "Native audio state is unavailable.".to_string())?;
                for source_id in payloads.keys() {
                    native_audio.reset_ndi_audio(source_id);
                }
            }
            let mut ndi = state
                .ndi
                .lock()
                .map_err(|_| "NDI state is unavailable.".to_string())?;
            for source_id in payloads.keys() {
                ndi.reset_audio_source(source_id);
            }
        }
    }
    let refreshed = refresh_playback_content(state, payloads.keys(), false)?;
    for (source_screen_id, payload) in payloads.iter_mut() {
        if let Some(content) = refreshed.get(source_screen_id) {
            if let Some(layers) = content.get("nativeAudioLayers") {
                payload["nativeAudioLayers"] = layers.clone();
            }
            if let Some(active) = content.get("nativeAudioActive") {
                payload["nativeAudioActive"] = active.clone();
            }
        }
    }
    present_native_output_sources(state, &payloads.keys().cloned().collect())?;
    Ok(())
}

fn playback_clock_requires_audio_reset(
    command: output_playback::PlaybackCommand,
    changed: bool,
) -> bool {
    changed
        && matches!(
            command.action,
            output_playback::PlaybackAction::Seek
                | output_playback::PlaybackAction::Forward
                | output_playback::PlaybackAction::Backward
        )
}

fn control_coordinated_playback_clock(
    state: &AppState,
    command: output_playback::PlaybackCommand,
) -> Result<Value, String> {
    let mut snapshot_state = lock_output_snapshot_state(&state.output_snapshot)?;
    let mut mutation = apply_playback_command_to_sources(state, command)?;
    let revision =
        anchor_native_playback_revision(&mut snapshot_state, mutation.targeted, mutation.changed);
    propagate_playback_source_mutation(state, command, mutation.changed, &mut mutation.payloads)?;
    mark_native_playback_propagated(&mut snapshot_state, revision);
    let Some(revision) = revision else {
        return Ok(json!({ "ok": true }));
    };
    Ok(json!({
        "ok": true,
        "revision": revision,
        "patch": {}
    }))
}

fn anchor_native_playback_revision(
    snapshot_state: &mut OutputSnapshotState,
    targeted: bool,
    changed: bool,
) -> Option<u64> {
    if !targeted || snapshot_state.cache.is_none() {
        return None;
    }
    if changed {
        let revision = snapshot_state.advance_revision();
        let cached = snapshot_state
            .cache
            .as_mut()
            .expect("checked native output snapshot cache");
        cached.revision = revision;
        cached.dirty = true;
        Some(revision)
    } else {
        let cached = snapshot_state
            .cache
            .as_mut()
            .expect("checked native output snapshot cache");
        cached.dirty = true;
        Some(cached.revision)
    }
}

fn mark_native_playback_propagated(
    snapshot_state: &mut OutputSnapshotState,
    revision: Option<u64>,
) {
    if let (Some(revision), Some(cached)) = (revision, snapshot_state.cache.as_mut()) {
        if cached.revision == revision {
            cached.dirty = false;
        }
    }
}

fn control_coordinated_transport(
    app: &Host,
    state: &AppState,
    command: output_playback::PlaybackCommand,
) -> Result<Value, String> {
    let requested_paused = match command.action {
        output_playback::PlaybackAction::Pause => Some(true),
        output_playback::PlaybackAction::Play => Some(false),
        output_playback::PlaybackAction::Toggle => None,
        _ => return Err("Coordinated transport requires toggle, play, or pause.".to_string()),
    };
    let rss_snapshot = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.".to_string())?
        .snapshot();
    let mut snapshot_state = lock_output_snapshot_state(&state.output_snapshot)?;
    let Some(cached) = snapshot_state.cache.as_ref() else {
        let paused = match requested_paused {
            Some(paused) => paused,
            None => !native_playback_paused(state, command.layer)?
                .or(native_output_content_paused(state, command.layer)?)
                .unwrap_or(false),
        };
        let command = playback_transport_command(command.layer, paused);
        let mut mutation = apply_playback_command_to_sources(state, command)?;
        propagate_transport_source_mutation(
            state,
            command,
            paused,
            mutation.changed,
            &mut mutation.payloads,
        )?;
        return Ok(json!({ "ok": true }));
    };
    let paused = match requested_paused {
        Some(paused) => paused,
        None => toggled_transport_target(
            native_playback_paused(state, command.layer)?,
            &cached.snapshot,
            command.layer,
        ),
    };
    let (snapshot, playback_patch) =
        set_transport_paused_in_output_snapshot(&cached.snapshot, command.layer, paused)?;
    let (revision, pending_rss_urls) = commit_native_output_snapshot(
        state,
        &mut snapshot_state,
        snapshot,
        &rss_snapshot,
        Some((command.layer, paused)),
        None,
    )?;
    drop(snapshot_state);
    schedule_output_rss_refreshes(app, pending_rss_urls);
    Ok(json!({
        "ok": true,
        "revision": revision,
        "patch": { "transportPlaybackPaused": playback_patch }
    }))
}

fn is_main_transport_content(content: &Value) -> bool {
    content.get("contentDestination").and_then(Value::as_str) != Some("announcements")
}

fn main_transport_source_ids(state: &AppState) -> Result<Vec<String>, String> {
    let mut source_ids = output_source_ids(state)?;
    source_ids.extend(active_native_audio_source_ids(state)?);
    let content = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?;
    source_ids.retain(|id| content.get(id).is_none_or(is_main_transport_content));
    Ok(source_ids)
}

fn native_playback_paused(
    state: &AppState,
    layer: output_playback::PlaybackLayer,
) -> Result<Option<bool>, String> {
    let source_ids = main_transport_source_ids(state)?;
    let mut unique_sources = HashSet::new();
    let mut playback = state
        .output_playback
        .lock()
        .map_err(|_| "Output playback state is unavailable.".to_string())?;
    let now = std::time::Instant::now();
    Ok(source_ids.into_iter().find_map(|source_id| {
        unique_sources
            .insert(source_id.clone())
            .then(|| playback.is_paused(&source_id, layer, now))
            .flatten()
    }))
}

fn native_output_content_paused(
    state: &AppState,
    layer: output_playback::PlaybackLayer,
) -> Result<Option<bool>, String> {
    let source_ids = main_transport_source_ids(state)?;
    let content = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?;
    Ok(source_ids.into_iter().find_map(|source_id| {
        content
            .get(&source_id)
            .and_then(|value| snapshot_transport_paused(value, layer))
    }))
}

fn playback_transport_command(
    layer: output_playback::PlaybackLayer,
    paused: bool,
) -> output_playback::PlaybackCommand {
    output_playback::PlaybackCommand {
        layer,
        action: if paused {
            output_playback::PlaybackAction::Pause
        } else {
            output_playback::PlaybackAction::Play
        },
        position: 0.0,
    }
}

fn propagate_transport_source_mutation(
    state: &AppState,
    command: output_playback::PlaybackCommand,
    paused: bool,
    changed: bool,
    payloads: &mut HashMap<String, Value>,
) -> Result<(), String> {
    let rebuilt_source_ids = payloads.keys().cloned().collect::<HashSet<_>>();
    propagate_playback_source_mutation(state, command, changed, payloads)?;
    refresh_unmapped_transport_sources(state, &rebuilt_source_ids, command.layer, paused)
}

fn update_automation_schedule(state: &AppState, snapshot: Value) -> Result<Value, String> {
    state
        .native_automation
        .lock()
        .map_err(|_| "Native automation state is unavailable.".to_string())?
        .update(&snapshot)
}

fn schedule_automation_actions(state: &AppState, request: Value) -> Result<Value, String> {
    state
        .native_automation
        .lock()
        .map_err(|_| "Native automation state is unavailable.".to_string())?
        .schedule_actions(&request)
}

fn get_slide_completion(state: &AppState) -> Result<Value, String> {
    Ok(state
        .native_automation
        .lock()
        .map_err(|_| "Native automation state is unavailable.".to_string())?
        .completion_countdown()
        .unwrap_or(Value::Null))
}

fn cancel_slide_completion(state: &AppState, live_key: Value) -> Result<Value, String> {
    state
        .native_automation
        .lock()
        .map_err(|_| "Native automation state is unavailable.".to_string())?
        .cancel_completion(&live_key)
}

fn control_timer(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let timer = argument(args, 0)?;
    let action = argument(args, 1)?;
    let action = action
        .as_str()
        .ok_or_else(|| "Timer action must be a string.".to_string())?;
    let current_run = args.get(2).filter(|value| !value.is_null());
    let increment_seconds = args.get(3).and_then(Value::as_f64).unwrap_or(0.0);
    let active_workspace = state
        .active_workspace
        .lock()
        .map_err(|_| "Workspace state is unavailable.".to_string())?
        .clone();
    let _workspace_io = if let Some(path) = active_workspace.as_ref() {
        let guard = lock_workspace_io(state)?;
        let workspace = workspace::read_workspace(path)?;
        validate_current_timer_definition(&workspace, &timer)?;
        Some(guard)
    } else {
        None
    };
    state
        .native_timers
        .lock()
        .map_err(|_| "Native timer state is unavailable.".to_string())?
        .control(&timer, action, current_run, increment_seconds)
}

fn validate_current_timer_definition(workspace: &Value, timer: &Value) -> Result<(), String> {
    let timer_id = timer
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "Timer id is required.".to_string())?;
    let persisted = workspace
        .get("countdownTimers")
        .and_then(Value::as_array)
        .and_then(|timers| {
            timers
                .iter()
                .find(|candidate| candidate.get("id").and_then(Value::as_str) == Some(timer_id))
        })
        .ok_or_else(|| "Timer definition is no longer available.".to_string())?;
    if !runtime_timer_definition_matches(persisted, timer) {
        return Err("Timer definition changed before the action was applied.".to_string());
    }
    Ok(())
}

fn runtime_timer_definition_matches(persisted: &Value, requested: &Value) -> bool {
    let persisted_type = persisted
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("countdown");
    let requested_type = requested
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("countdown");
    persisted_type == requested_type
        && [
            "durationSeconds",
            "allowsOverrun",
            "targetTimeSeconds",
            "elapsedStartSeconds",
            "elapsedEndSeconds",
            "format",
        ]
        .into_iter()
        .all(|key| persisted.get(key) == requested.get(key))
}

fn refresh_playback_content<'a>(
    state: &AppState,
    source_ids: impl Iterator<Item = &'a String>,
    synchronize_transport: bool,
) -> Result<serde_json::Map<String, Value>, String> {
    let source_ids = source_ids
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    if source_ids.is_empty() {
        return Ok(serde_json::Map::new());
    }
    let stored = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?;
    let inputs = source_ids
        .iter()
        .filter_map(|source_id| {
            stored
                .get(source_id)
                .cloned()
                .map(|content| (source_id.clone(), content))
        })
        .collect::<Vec<_>>();
    drop(stored);
    let now = std::time::Instant::now();
    let mut canonical = {
        let mut playback = state
            .output_playback
            .lock()
            .map_err(|_| "Output playback state is unavailable.".to_string())?;
        inputs
            .into_iter()
            .map(|(source_id, content)| {
                let content = if synchronize_transport {
                    playback.sync_content(&source_id, &content, now)
                } else {
                    playback.canonical_content(&source_id, &content, now)
                };
                (source_id, content)
            })
            .collect::<serde_json::Map<_, _>>()
    };
    synchronize_native_audio_outputs(state, &mut canonical, &source_ids, now)?;
    {
        let mut content = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?;
        for (source_id, output) in &canonical {
            content.insert(source_id.clone(), output.clone());
        }
    }
    {
        let mut revisions = state
            .output_revisions
            .lock()
            .map_err(|_| "Output revision state is unavailable.".to_string())?;
        for source_id in canonical.keys() {
            let revision = revisions.entry(source_id.clone()).or_default();
            *revision = revision.saturating_add(1);
        }
    }
    state
        .native_frame_cache
        .lock()
        .map_err(|_| "Native frame cache is unavailable.".to_string())?
        .retain(|key, _| !canonical.contains_key(&key.source_screen_id));
    state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .update_output_content(&Value::Object(canonical.clone()))?;
    Ok(canonical)
}

fn window_command(app: &Host, command: Value) -> Result<Value, String> {
    let command = command
        .as_str()
        .ok_or_else(|| "Window command must be a string.".to_string())?;
    let window = app
        .get_window("main")
        .ok_or_else(|| "The main window is unavailable.".to_string())?;

    match command {
        "maximize" => window.maximize()?,
        "minimize" => app.windows().minimize_main_window()?,
        "restore" => window.unmaximize()?,
        _ => return Err("Unsupported window command.".to_string()),
    }

    Ok(json!({ "ok": true }))
}

fn load_workspace(app: &Host, state: &AppState, workspace_path: Value) -> Result<Value, String> {
    let mut inputs = communication_input_commands::lock(state)?;
    inputs.stop_all();
    let workspace_path = workspace_path
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "A workspace path is required.".to_string())?;
    let path = PathBuf::from(workspace_path)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let group_settings = read_normalized_group_settings(app, state)?;
    capture_commands::stop_capture(state)?;
    native_ticker::clear()?;
    let _settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    let mut settings = read_settings(app)?;
    normalize_native_settings(&mut settings)?;
    let mut workspace_io = lock_workspace_io(state)?;
    let mut workspace = workspace::read_workspace(&path)?;
    workspace::merge_missing_workspace_group_settings(&mut workspace, &group_settings);
    let effective_screen_config = effective_screen_config_for_mutation(
        &workspace,
        &native_screen_config_for_mutation(&settings),
    );
    let revision = workspace_io.activate(&path);
    remote_controls::prime_workspace_cache(&path, revision, workspace.clone())?;
    state
        .network_link
        .lock()
        .map_err(|_| "Network Link state is unavailable.")?
        .invalidate_pending();
    {
        let mut active = state
            .active_workspace
            .lock()
            .map_err(|_| "Workspace state is unavailable.".to_string())?;
        state.local_text.shutdown();
        *active = Some(path);
    }
    state
        .native_timers
        .lock()
        .map_err(|_| "Native timer state is unavailable.".to_string())?
        .clear();
    *state
        .native_messages
        .lock()
        .map_err(|_| "Native message state is unavailable.".to_string())? =
        native_messages::MessageRuntime::default();
    *state
        .native_props
        .lock()
        .map_err(|_| "Native prop state is unavailable.")? = native_props::PropRuntime::default();
    Ok(json!({
        "workspace": workspace,
        "revision": revision,
        "effectiveScreenConfig": effective_screen_config,
        "settingsRevision": settings.get("settingsRevision").and_then(Value::as_u64),
    }))
}

fn read_workspace_snapshot(state: &AppState, workspace_path: Value) -> Result<Value, String> {
    let workspace_path = workspace_path
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "A workspace path is required.".to_string())?;
    let mut workspace_io = lock_workspace_io(state)?;
    let path = PathBuf::from(workspace_path)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    workspace_io.require_active(&path)?;
    let workspace = workspace::read_workspace(&path)?;
    Ok(json!({
        "workspace": workspace,
        "revision": workspace_io.revision(&path),
    }))
}

fn select_orbit_workspace_folder(app: &Host, state: &AppState) -> Result<Value, String> {
    let Some(folder) = FileDialog::new()
        .set_title("Choose Presenter Orbit workspace folder")
        .pick_folder()
    else {
        return Ok(json!({ "canceled": true }));
    };
    activate_orbit_workspace(
        app,
        state,
        workspace::workspace_path(&folder),
        |registry, path| {
            workspace_registry::register(registry, path, &workspace_folder_name(Path::new(path)));
            Ok(())
        },
    )
}

fn select_presentation_text_file() -> Result<Value, String> {
    let Some(path) = FileDialog::new()
        .set_title("Choose text file")
        .add_filter("Text files", &["txt", "rtf", "md", "csv"])
        .pick_file()
    else {
        return Ok(json!({ "canceled": true }));
    };
    let text = local_text::read_text(&path)?;
    Ok(json!({ "canceled": false, "filePath": path, "text": text }))
}

fn fetch_rss_feed(value: Value) -> Result<Value, String> {
    let raw_url = value
        .as_str()
        .ok_or_else(|| "RSS URL must be a string.".to_string())?;
    rss::fetch_feed(raw_url)
}

fn fetch_rss_feeds(value: Value) -> Result<Value, String> {
    let urls = value
        .as_array()
        .ok_or_else(|| "RSS URLs must be an array.".to_string())?;
    let mut feeds = serde_json::Map::new();

    for value in urls {
        let url = value
            .as_str()
            .ok_or_else(|| "Each RSS URL must be a string.".to_string())?;
        if feeds.contains_key(url) {
            continue;
        }
        if let Ok(feed) = rss::fetch_feed(url) {
            feeds.insert(url.to_string(), feed);
        }
    }

    Ok(json!({ "feeds": feeds }))
}

fn select_logo_image() -> Result<Value, String> {
    let Some(path) = FileDialog::new()
        .set_title("Select logo image")
        .add_filter(
            "Images",
            &["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg"],
        )
        .pick_file()
    else {
        return Ok(json!({ "canceled": true }));
    };
    let file_url = file_url(&path)?;
    Ok(json!({ "canceled": false, "filePath": path, "fileUrl": file_url }))
}

fn select_media_smart_playlist_folder() -> Result<Value, String> {
    let Some(path) = FileDialog::new()
        .set_title("Select folder for smart playlist")
        .pick_folder()
    else {
        return Ok(json!({ "canceled": true }));
    };
    let scanned = workspace::scan_media_folder(&path)?;
    let mut result = scanned
        .as_object()
        .cloned()
        .ok_or_else(|| "Media scan result must be an object.".to_string())?;
    result.insert("canceled".to_string(), json!(false));
    Ok(Value::Object(result))
}

fn select_theme_asset_file() -> Result<Value, String> {
    let Some(path) = FileDialog::new()
        .set_title("Choose theme asset")
        .add_filter(
            "Media",
            &[
                "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "mp4", "mov",
            ],
        )
        .pick_file()
    else {
        return Ok(json!({ "canceled": true }));
    };
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let media_type = match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" => "image",
        "mp4" | "mov" => "video",
        _ => "media",
    };
    let title = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Asset");
    let id = format!(
        "theme-asset-{}-{}",
        Utc::now().timestamp_millis(),
        safe_slug(title)
    );
    Ok(json!({
        "canceled": false,
        "media": {
            "id": id,
            "title": title,
            "type": media_type,
            "sourcePath": path,
            "sourceUrl": file_url(&path)?
        }
    }))
}

fn file_url(path: &Path) -> Result<String, String> {
    url::Url::from_file_path(path)
        .map(|value| value.to_string())
        .map_err(|_| "Could not convert file path to a URL.".to_string())
}

fn safe_slug(value: &str) -> String {
    let slug: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "asset".to_string()
    } else {
        slug.to_string()
    }
}

/// Lets the user pick a ProPresenter folder and lists what it holds, without importing.
fn scan_pro_presenter_workspace() -> Result<Value, String> {
    let Some(source) = FileDialog::new()
        .set_title("Select ProPresenter workspace")
        .pick_folder()
    else {
        return Ok(json!({ "canceled": true }));
    };
    let inventory = workspace::scan_pro_presenter_workspace(&source)?;
    Ok(json!({ "canceled": false, "path": source, "inventory": inventory }))
}

fn select_pro_presenter_workspace(
    app: &Host,
    state: &AppState,
    source: &Value,
    selection: &Value,
) -> Result<Value, String> {
    let source = PathBuf::from(
        source
            .as_str()
            .ok_or("ProPresenter folder path is required.")?,
    );
    let selection = workspace::ImportSelection::parse(selection)?;
    let current_group_settings = read_normalized_group_settings(app, state)?;
    let workspace_io = Arc::clone(&state.workspace_io);
    let active_workspace = Arc::clone(&state.active_workspace);

    let result = (move || -> Result<Value, String> {
        let mut workspace_io = workspace_io
            .lock()
            .map_err(|_| "Workspace I/O state is unavailable.".to_string())?;
        let destination = active_workspace
            .lock()
            .map_err(|_| "Workspace state is unavailable.".to_string())?
            .clone()
            .ok_or_else(|| "Choose a Presenter Orbit workspace folder first.".to_string())?;
        let workspace = workspace::import_pro_presenter_workspace_media(
            &source,
            &destination,
            &current_group_settings,
            workspace::propresenter_app_preferences().as_deref(),
            manage_media_setting(app)?,
            &selection,
        )?;
        let revision = workspace_io.advance_revision(&destination);
        drop(workspace_io);
        Ok(json!({ "canceled": false, "workspace": workspace, "revision": revision }))
    })()?;

    if result.get("canceled") != Some(&json!(true)) {
        let settings_destination = result
            .get("workspace")
            .and_then(|workspace| workspace.get("workspacePath"))
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .ok_or_else(|| "Imported workspace path is unavailable.".to_string())?;
        let _settings_operation = state
            .settings_operations
            .lock()
            .map_err(|_| "Settings state is unavailable.".to_string())?;
        let workspace_io = lock_workspace_io(state)?;
        workspace_io
            .require_active(&settings_destination)
            .map_err(|_| "Active workspace changed while the import was completing.".to_string())?;
        save_settings_locked(
            app,
            state,
            json!({ "presenterOrbitWorkspacePath": settings_destination }),
            None,
        )?;
        state
            .native_timers
            .lock()
            .map_err(|_| "Native timer state is unavailable.".to_string())?
            .clear();
        *state
            .native_messages
            .lock()
            .map_err(|_| "Native message state is unavailable.".to_string())? =
            native_messages::MessageRuntime::default();
        *state
            .native_props
            .lock()
            .map_err(|_| "Native prop state is unavailable.")? =
            native_props::PropRuntime::default();
    }

    Ok(result)
}

fn export_presentation(
    state: &AppState,
    requested_path: Value,
    request: Value,
) -> Result<Value, String> {
    let path = active_workspace(state, requested_path)?;
    let request: presentation_export::ExportRequest = serde_json::from_value(request)
        .map_err(|error| format!("Invalid export options: {error}"))?;
    let (revision, prepared) = {
        let mut io = lock_workspace_io(state)?;
        io.require_active(&path)?;
        (
            io.revision(&path),
            presentation_export::prepare(&workspace::read_workspace(&path)?, &request)?,
        )
    };
    let destination = if request.format == "images" {
        FileDialog::new()
            .set_title("Choose parent folder for numbered slide PNG files")
            .pick_folder()
            .map(|parent| parent.join(format!("Presentation Slides-{}", uuid::Uuid::new_v4())))
    } else {
        let extension = if request.format == "pdf" {
            "pdf"
        } else {
            "txt"
        };
        FileDialog::new()
            .set_title("Export presentation (choose a new file)")
            .add_filter("Presentation export", &[extension])
            .set_file_name(format!("presentation.{extension}"))
            .save_file()
    };
    let Some(destination) = destination else {
        return Ok(json!({"canceled": true}));
    };
    let mut io = lock_workspace_io(state)?;
    io.require_revision(&path, revision)?;
    let current = presentation_export::prepare(&workspace::read_workspace(&path)?, &request)?;
    if current.text != prepared.text
        || current.scenes != prepared.scenes
        || current.dimensions != prepared.dimensions
    {
        return Err("Presentation changed after export preflight. Start export again.".into());
    }
    presentation_export::write_export(&current, &request.format, &destination)?;
    Ok(
        json!({"canceled": false, "destinationPath": destination, "slideCount": if request.format == "text" { current.text.matches("--- Slide ").count() } else { current.scenes.len() }}),
    )
}
fn export_pro_presenter_workspace(
    state: &AppState,
    requested_path: Value,
) -> Result<Value, String> {
    let path = active_workspace(state, requested_path)?;
    let (reviewed_revision, losses) = {
        let mut workspace_io = lock_workspace_io(state)?;
        workspace_io.require_active(&path)?;
        (
            workspace_io.revision(&path),
            workspace::pro_presenter_export_losses(&path)?,
        )
    };
    let summary = if losses.is_empty() {
        "No known data losses were found in the current portable export subset.".to_string()
    } else {
        format!(
            "These data will not survive export:\n\n{}",
            losses.join("\n")
        )
    };
    workspace::reviewed_export(
        || {
            MessageDialog::new()
            .set_title("Review ProPresenter export")
            .set_description(format!("{summary}\n\nThe encoder exports portable text slides, not a lossless round trip. Continue?"))
            .set_level(MessageLevel::Warning)
            .set_buttons(MessageButtons::OkCancel)
            .show() == MessageDialogResult::Ok
        },
        || {
            FileDialog::new()
                .set_title("Choose an empty parent folder for the ProPresenter export")
                .pick_folder()
        },
        |parent| {
            let mut workspace_io = lock_workspace_io(state)?;
            workspace_io.require_revision(&path, reviewed_revision)?;
            if workspace::pro_presenter_export_losses(&path)? != losses {
                return Err(
                    "Workspace export losses changed after review. Review the export again."
                        .to_string(),
                );
            }
            let destination = parent.join("Presenter Orbit Export");
            let result = workspace::export_pro_presenter_workspace(&path, &destination)?;
            let mut result = result
                .as_object()
                .cloned()
                .ok_or_else(|| "ProPresenter export result must be an object.".to_string())?;
            result.insert("canceled".to_string(), json!(false));
            Ok(Value::Object(result))
        },
    )
}

fn get_library_item_location(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let workspace_io = lock_workspace_io(state)?;
    workspace_io.require_active(&path)?;
    let item_id_value = argument(args, 1)?;
    let item_id = item_id_value
        .as_str()
        .ok_or_else(|| "Library item id must be a string.".to_string())?;
    Ok(json!({ "filePath": workspace::library_item_location(&path, item_id)? }))
}

fn strip_rtf(value: &str) -> String {
    let mut result = String::new();
    let mut control = false;
    for character in value.chars() {
        match character {
            '\\' => control = true,
            '{' | '}' => {}
            ' ' if control => control = false,
            _ if control => {}
            _ => result.push(character),
        }
    }
    result.trim().to_string()
}

fn open_external_url(value: Value) -> Result<Value, String> {
    let value = value
        .as_str()
        .ok_or_else(|| "URL must be a string.".to_string())?;
    let url = url::Url::parse(value).map_err(|error| error.to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only http and https URLs can be opened.".to_string());
    }
    open::that(url.as_str()).map_err(|error| error.to_string())?;
    Ok(json!({ "ok": true }))
}

fn send_communication(device: Value, payload: Value) -> Result<Value, String> {
    communication::send(device, payload)
}

fn open_settings_window(app: &Host, panel: Value) -> Result<Value, String> {
    app.windows()
        .open_auxiliary_window(json!({ "kind": "settings", "panel": panel }))
}

fn open_log_files(app: &Host) -> Result<Value, String> {
    let logs = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("logs");
    fs::create_dir_all(&logs).map_err(|error| error.to_string())?;
    open::that(logs).map_err(|error| error.to_string())?;
    Ok(json!({ "ok": true }))
}

fn show_item_in_folder(value: Value) -> Result<Value, String> {
    let path = PathBuf::from(
        value
            .as_str()
            .ok_or_else(|| "File path must be a string.".to_string())?,
    );
    let target = if path.is_file() {
        path.parent().map(PathBuf::from)
    } else {
        Some(path)
    }
    .ok_or_else(|| "File path has no parent folder.".to_string())?;
    open::that(target).map_err(|error| error.to_string())?;
    Ok(json!({ "ok": true }))
}

fn read_clipboard_text() -> Result<Value, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard
        .get_text()
        .map(Value::String)
        .map_err(|error| error.to_string())
}

fn write_clipboard_text(value: Value) -> Result<Value, String> {
    let text = value
        .as_str()
        .ok_or_else(|| "Clipboard text must be a string.".to_string())?;
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard
        .set_text(text)
        .map_err(|error| error.to_string())?;
    Ok(json!(true))
}

fn export_diagnostics(app: &Host, state: &AppState) -> Result<Value, String> {
    let Some(path) = FileDialog::new()
        .set_title("Export Presenter Orbit diagnostics")
        .set_file_name("presenter-orbit-diagnostics.json")
        .save_file()
    else {
        return Ok(json!({ "canceled": true }));
    };
    let data_directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let settings = {
        let _settings_operation = state
            .settings_operations
            .lock()
            .map_err(|_| "Settings state is unavailable.".to_string())?;
        read_settings(app)?
    };
    let ndi_status = state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .status();
    let diagnostics = diagnostics::build_bundle(
        app.version(),
        app.runtime(),
        &settings,
        &ndi_status,
        &data_directory,
    );
    fs::write(
        &path,
        serde_json::to_string_pretty(&diagnostics).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(json!({ "canceled": false, "filePath": path }))
}

fn active_workspace(state: &AppState, requested: Value) -> Result<PathBuf, String> {
    let active = state
        .active_workspace
        .lock()
        .map_err(|_| "Workspace state is unavailable.".to_string())?
        .clone()
        .ok_or_else(|| "Choose a Presenter Orbit workspace folder first.".to_string())?;
    let requested = requested.as_str().unwrap_or_default().trim();
    if !requested.is_empty()
        && PathBuf::from(requested)
            .canonicalize()
            .map_err(|error| error.to_string())?
            != active
    {
        return Err(
            "The requested workspace is not the active Presenter Orbit workspace.".to_string(),
        );
    }
    Ok(active)
}

fn lock_workspace_io<'a>(
    state: &'a AppState,
) -> Result<std::sync::MutexGuard<'a, WorkspaceIoState>, String> {
    state
        .workspace_io
        .lock()
        .map_err(|_| "Workspace I/O state is unavailable.".to_string())
}

fn expected_workspace_revision(args: &[Value], index: usize) -> Result<u64, String> {
    args.get(index)
        .and_then(Value::as_u64)
        .ok_or_else(|| "Workspace base revision is required.".to_string())
}

fn save_workspace_content(app: &Host, state: &AppState, args: &[Value]) -> Result<Value, String> {
    let mut input_guard = if argument(args, 1)?.get("communicationDevices").is_some() {
        Some(communication_input_commands::lock(state)?)
    } else {
        None
    };
    if let Some(inputs) = input_guard.as_mut() {
        inputs.stop_all();
    }
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let workspace = workspace::save_workspace(&path, &argument(args, 1)?)?;
    let revision = workspace_io.advance_revision(&path);
    if argument(args, 1)?.get("communicationDevices").is_some() {
        app.emit("communication-devices-changed",json!({"workspacePath":argument(args,0)?,"revision":revision,"devices":workspace["communicationDevices"]})).map_err(|error|error.to_string())?;
    }
    if argument(args, 1)?.get("triggerBindings").is_some() {
        app.emit_to("main", "trigger-bindings-changed", json!({"workspacePath":argument(args, 0)?,"revision":revision,"bindings":workspace["triggerBindings"]}))
            .map_err(|error| error.to_string())?;
    }
    if argument(args, 1)?.get("audioInputs").is_some() {
        app.emit_to("main", "audio-inputs-changed", json!({"workspacePath":argument(args, 0)?,"revision":revision,"inputs":workspace["audioInputs"]}))
            .map_err(|error| error.to_string())?;
    }
    let response = json!({ "workspace": saved_content_response(&workspace, &argument(args, 1)?), "revision": revision });
    remote_controls::prime_workspace_cache(&path, revision, workspace)?;
    Ok(response)
}

/// Returns the patched top-level keys of a saved workspace, plus the scalars a save derives outside its patch:
/// `workspacePath`, `lastUsedPresentationThemeId`, and `recentPresentationThemeIds` (rewritten by content theme refresh).
/// Returning the whole workspace cost a multi-megabyte IPC payload per save.
pub(crate) fn saved_content_response(workspace: &Value, patch: &Value) -> Value {
    let keys = patch
        .as_object()
        .into_iter()
        .flat_map(|patch| patch.keys().map(String::as_str));
    let keys = keys.chain([
        "workspacePath",
        "lastUsedPresentationThemeId",
        "recentPresentationThemeIds",
    ]);
    Value::Object(
        keys.filter_map(|key| {
            workspace
                .get(key)
                .map(|value| (key.to_string(), value.clone()))
        })
        .collect(),
    )
}

/// Returns only `keys` of a saved workspace; callers list the keys their renderer code reads.
pub(crate) fn workspace_keys_response(workspace: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|key| {
                workspace
                    .get(*key)
                    .map(|value| (key.to_string(), value.clone()))
            })
            .collect(),
    )
}

/// Responds with the patched keys of `workspace` and feeds the full saved document to the Remote catalog cache.
fn saved_patch_response(
    path: &std::path::Path,
    workspace: Value,
    patch: &Value,
    revision: u64,
) -> Result<Value, String> {
    let response =
        json!({ "workspace": saved_content_response(&workspace, patch), "revision": revision });
    remote_controls::prime_workspace_cache(path, revision, workspace)?;
    Ok(response)
}

fn save_library_item(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let item = argument(args, 1)?;
    let workspace = workspace::save_library_item(&path, &item)?;
    let revision = workspace_io.advance_revision(&path);
    // Renderer callers only look up the saved item in `library`.
    let saved = workspace["library"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["id"] == item["id"])
        .cloned()
        .collect::<Vec<_>>();
    let response = json!({ "workspace": { "library": saved }, "revision": revision });
    remote_controls::prime_workspace_cache(&path, revision, workspace)?;
    Ok(response)
}

fn save_library_items(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    workspace::save_library_items(&path, &argument(args, 1)?)?;
    let revision = workspace_io.advance_revision(&path);
    Ok(json!({ "ok": true, "revision": revision }))
}

fn list_workspace_field(state: &AppState, args: &[Value], field: &str) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_active(&path)?;
    let workspace = workspace::read_workspace(&path)?;
    let mut response = serde_json::Map::new();
    response.insert(
        field.to_string(),
        workspace.get(field).cloned().unwrap_or_else(|| json!([])),
    );
    response.insert("revision".to_string(), json!(workspace_io.revision(&path)));
    Ok(Value::Object(response))
}

fn save_named_workspace_value(
    state: &AppState,
    args: &[Value],
    field: &str,
) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let value = argument(args, 1)?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} id is required."))?;
    let workspace = workspace::save_named_collection(&path, field, &value, id)?;
    let values = workspace.get(field).cloned().unwrap_or_else(|| json!([]));
    let saved = values
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
        })
        .cloned()
        .ok_or_else(|| format!("Saved {field} is unavailable."))?;
    let response_key = if field == "themes" { "theme" } else { "mask" };
    let mut response = serde_json::Map::new();
    response.insert(response_key.to_string(), saved);
    response.insert(field.to_string(), values);
    if field == "themes" {
        response.insert("library".to_string(), workspace["library"].clone());
    }
    response.insert(
        "revision".to_string(),
        json!(workspace_io.advance_revision(&path)),
    );
    Ok(Value::Object(response))
}

fn delete_workspace_theme(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 3)?)?;
    let theme_id = argument(args, 1)?;
    let theme_id = theme_id
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Theme id is required.")?;
    let slide_id = argument(args, 2)?;
    let slide_id = match &slide_id {
        Value::Null => None,
        Value::String(id) if !id.is_empty() => Some(id.as_str()),
        _ => return Err("Theme layout id must be a non-empty string or null.".into()),
    };
    let workspace = workspace::delete_theme(&path, theme_id, slide_id)?;
    Ok(json!({ "workspace": workspace, "revision": workspace_io.advance_revision(&path) }))
}

fn mutate_masks(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let result = workspace::mutate_masks(&path, &argument(args, 1)?)?;
    let revision = if result.changed {
        workspace_io.advance_revision(&path)
    } else {
        workspace_io.revision(&path)
    };
    let mut response = json!({
        "changed": result.changed,
        "workspace": result.workspace,
        "revision": revision,
    });
    if let Some(created_mask_id) = result.created_mask_id {
        response["createdMaskId"] = json!(created_mask_id);
    }
    if let Some(created_layer_id) = result.created_layer_id {
        response["createdLayerId"] = json!(created_layer_id);
    }
    Ok(response)
}

fn list_props(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_active(&path)?;
    let workspace = workspace::read_workspace(&path)?;
    Ok(json!({
        "props": workspace.get("props").cloned().unwrap_or_else(|| json!([])),
        "propFolders": workspace.get("propFolders").cloned().unwrap_or_else(|| json!([])),
        "countdownTimers": workspace.get("countdownTimers").cloned().unwrap_or_else(|| json!([])),
        "revision": workspace_io.revision(&path)
    }))
}

fn mutate_props(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let result = workspace::mutate_props(&path, &argument(args, 1)?)?;
    let revision = if result.changed {
        workspace_io.advance_revision(&path)
    } else {
        workspace_io.revision(&path)
    };
    // The renderer only consumes props and propFolders; returning the whole workspace cost a multi-megabyte payload.
    let mut response = json!({
        "changed": result.changed,
        "workspace": workspace_keys_response(&result.workspace, &["props", "propFolders"]),
        "revision": revision,
    });
    if result.changed {
        remote_controls::prime_workspace_cache(&path, revision, result.workspace)?;
    }
    if let Some(created_prop_id) = result.created_prop_id {
        response["createdPropId"] = json!(created_prop_id);
    }
    if let Some(created_folder) = result.created_folder {
        response["createdFolder"] = json!(created_folder);
    }
    if let Some(deleted_prop_id) = result.deleted_prop_id {
        response["deletedPropId"] = json!(deleted_prop_id);
    }
    Ok(response)
}

fn mutate_countdown_timers(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    // Keep the native run registry locked across the persisted transaction.
    // A delete or type change therefore clears an in-memory run only after
    // the definition commit succeeds; failed writes leave the run intact.
    let mut native_timers = state
        .native_timers
        .lock()
        .map_err(|_| "Native timer state is unavailable.".to_string())?;
    let result = workspace::mutate_countdown_timers(&path, &argument(args, 1)?)?;
    if result.changed {
        for timer_id in &result.removed_timer_ids {
            native_timers.remove(timer_id);
        }
    }
    let revision = if result.changed {
        workspace_io.advance_revision(&path)
    } else {
        workspace_io.revision(&path)
    };
    // The renderer only consumes the timers; returning the whole workspace cost a multi-megabyte IPC payload.
    let mut response = json!({
        "changed": result.changed,
        "workspace": {"countdownTimers": result.workspace["countdownTimers"]},
        "revision": revision,
    });
    if result.changed {
        crate::remote_controls::prime_workspace_cache(&path, revision, result.workspace)?;
    }
    if let Some(created_timer_id) = result.created_timer_id {
        response["createdTimerId"] = json!(created_timer_id);
    }
    if let Some(deleted_timer_id) = result.deleted_timer_id {
        response["deletedTimerId"] = json!(deleted_timer_id);
    }
    if !result.removed_timer_ids.is_empty() {
        response["removedTimerIds"] = json!(result.removed_timer_ids);
    }
    Ok(response)
}

fn save_props(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 4)?)?;
    let mut patch = serde_json::Map::new();
    patch.insert("props".to_string(), argument(args, 1)?);
    patch.insert("propFolders".to_string(), argument(args, 2)?);
    if let Some(value) = args.get(3).filter(|value| !value.is_null()) {
        patch.insert("countdownTimers".to_string(), value.clone());
    }
    let patch = Value::Object(patch);
    let workspace = workspace::save_workspace(&path, &patch)?;
    let revision = workspace_io.advance_revision(&path);
    saved_patch_response(&path, workspace, &patch, revision)
}

fn list_stage(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_active(&path)?;
    let workspace = workspace::read_workspace(&path)?;
    Ok(json!({
        "stageLayouts": workspace.get("stageLayouts").cloned().unwrap_or_else(|| json!([])),
        "stageScreenAssignments": workspace.get("stageScreenAssignments").cloned().unwrap_or_else(|| json!([])),
        "stageMessage": workspace.get("stageMessage").cloned().unwrap_or_else(|| json!({ "text": "", "state": "cleared" })),
        "revision": workspace_io.revision(&path)
    }))
}

fn mutate_stage_content(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let result = workspace::mutate_stage_content(&path, &argument(args, 1)?)?;
    let revision = if result.changed {
        workspace_io.advance_revision(&path)
    } else {
        workspace_io.revision(&path)
    };
    let mut response = json!({
        "changed": result.changed,
        "workspace": workspace_keys_response(&result.workspace, &["stageLayouts", "stageMessage"]),
        "revision": revision,
    });
    if result.changed {
        remote_controls::prime_workspace_cache(&path, revision, result.workspace)?;
    }
    if let Some(created_layout_id) = result.created_layout_id {
        response["createdLayoutId"] = json!(created_layout_id);
    }
    Ok(response)
}

fn save_stage(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 4)?)?;
    let patch = json!({ "stageLayouts": argument(args, 1)?, "stageScreenAssignments": argument(args, 2)?, "stageMessage": argument(args, 3)? });
    let workspace = workspace::save_workspace(&path, &patch)?;
    let revision = workspace_io.advance_revision(&path);
    saved_patch_response(&path, workspace, &patch, revision)
}

fn save_media_library(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let library = argument(args, 1)?;
    let id = library
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "Media library id is required.".to_string())?;
    let workspace = workspace::save_named_collection(&path, "mediaLibraries", &library, id)?;
    let revision = workspace_io.advance_revision(&path);
    Ok(json!({ "workspace": workspace, "revision": revision }))
}

fn save_media_libraries(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let patch = json!({ "mediaLibraries": argument(args, 1)? });
    let workspace = workspace::save_workspace(&path, &patch)?;
    let revision = workspace_io.advance_revision(&path);
    // The renderer applies the canonical media list together with the libraries.
    let response = json!({ "workspace": workspace_keys_response(&workspace, &["mediaLibraries", "media"]), "revision": revision });
    remote_controls::prime_workspace_cache(&path, revision, workspace)?;
    Ok(response)
}

fn save_media_settings(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let patch = json!({ "media": argument(args, 1)? });
    let workspace = workspace::save_workspace(&path, &patch)?;
    let revision = workspace_io.advance_revision(&path);
    saved_patch_response(&path, workspace, &patch, revision)
}

fn list_missing_media(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let _workspace_io = lock_workspace_io(state)?;
    media_relink::missing(&path, &argument(args, 1)?)
}

fn review_media_cleanup(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_active(&path)?;
    let live = state
        .output_content
        .lock()
        .map_err(|_| "Output content is unavailable.")?
        .clone();
    let mut review = media_cleanup::review(&path, &json!(live))?;
    review["revision"] = json!(workspace_io.revision(&path));
    Ok(review)
}

fn clean_unused_media(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut reviewed = argument(args, 1)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(
        &path,
        reviewed["revision"]
            .as_u64()
            .ok_or("Reviewed workspace revision is required.")?,
    )?;
    reviewed
        .as_object_mut()
        .ok_or("Cleanup review must be an object.")?
        .remove("revision");
    let live = state
        .output_content
        .lock()
        .map_err(|_| "Output content is unavailable.")?
        .clone();
    let prepared = media_cleanup::prepare(&path, &reviewed, &json!(live))?;
    let _snapshot = lock_output_snapshot_state(&state.output_snapshot)?;
    let live = state
        .output_content
        .lock()
        .map_err(|_| "Output content is unavailable.")?;
    let mut result = media_cleanup::execute(&path, prepared, &json!(*live))?;
    result["revision"] = json!(workspace_io.advance_revision(&path));
    Ok(result)
}

fn relink_missing_media(app: &Host, state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let request = argument(args, 1)?;
    let Some(replacement) = FileDialog::new()
        .set_title("Choose replacement for missing media")
        .pick_file()
    else {
        return Ok(json!({ "canceled": true }));
    };
    let path_after_dialog = active_workspace(state, argument(args, 0)?)?;
    if path != path_after_dialog {
        return Err("Workspace changed while choosing replacement media.".into());
    }
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let mut workspace =
        media_relink::relink(&path, &request, &replacement, manage_media_setting(app)?)?;
    let revision = workspace_io.advance_revision(&path);
    let cleanup_warning = workspace
        .as_object_mut()
        .and_then(|object| object.remove("thumbnailCleanupWarning"));
    Ok(
        json!({ "canceled": false, "workspace": workspace, "revision": revision, "cleanupWarning": cleanup_warning }),
    )
}

fn forget_missing_media(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let workspace = media_relink::forget(&path, &argument(args, 1)?)?;
    let revision = workspace_io.advance_revision(&path);
    Ok(json!({ "workspace": workspace, "revision": revision }))
}

fn save_media_state(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 3)?)?;
    let patch = json!({
        "mediaLibraries": argument(args, 1)?,
        "media": argument(args, 2)?,
    });
    let workspace = workspace::save_workspace(&path, &patch)?;
    let revision = workspace_io.advance_revision(&path);
    saved_patch_response(&path, workspace, &patch, revision)
}

fn mutate_media_library(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let result = workspace::mutate_media_library(&path, &argument(args, 1)?)?;
    let revision = if result.changed {
        workspace_io.advance_revision(&path)
    } else {
        workspace_io.revision(&path)
    };
    let mut response = json!({
        "workspace": result.workspace,
        "revision": revision,
        "changed": result.changed,
    });
    if let Some(selected_library_id) = result.selected_library_id {
        response["selectedMediaLibraryId"] = json!(selected_library_id);
    }
    Ok(response)
}

fn mutate_presentation_library(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let mutation = argument(args, 1)?;
    let result = workspace::mutate_presentation_library(&path, &mutation)?;
    let revision = if result.changed {
        workspace_io.advance_revision(&path)
    } else {
        workspace_io.revision(&path)
    };
    let mut response = json!({
        "workspace": result.workspace,
        "revision": revision,
        "changed": result.changed,
    });
    if result.changed
        && mutation["operation"]["type"] == "setSlidesTheme"
        && mutation["operation"].get("expectedSlides").is_some()
    {
        if let Some(item) = result.workspace["library"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| item["id"] == mutation["itemId"])
        {
            response["themeSlides"] = item["slides"].clone();
        }
    }
    if result.changed {
        remote_controls::prime_workspace_cache(&path, revision, result.workspace)?;
    }
    if let Some(created_library_id) = result.created_library_id {
        response["createdLibraryId"] = json!(created_library_id);
    }
    if let Some(created_item_id) = result.created_item_id {
        response["createdItemId"] = json!(created_item_id);
    }
    if let Some(created_playlist_id) = result.created_playlist_id {
        response["createdPlaylistId"] = json!(created_playlist_id);
    }
    if let Some(created_slide_id) = result.created_slide_id {
        response["createdSlideId"] = json!(created_slide_id);
    }
    if let Some(selected_slide_id) = result.selected_slide_id {
        response["selectedSlideId"] = json!(selected_slide_id);
    }
    if let Some(created_arrangement_id) = result.created_arrangement_id {
        response["createdArrangementId"] = json!(created_arrangement_id);
    }
    if let Some(copied_arrangement_id) = result.copied_arrangement_id {
        response["copiedArrangementId"] = json!(copied_arrangement_id);
    }
    if !result.deleted_library_ids.is_empty() {
        response["deletedLibraryIds"] = json!(result.deleted_library_ids);
    }
    if !result.copied_item_ids.is_empty() {
        response["copiedItemIds"] = json!(result.copied_item_ids);
    }
    if !result.moved_item_ids.is_empty() {
        response["movedItemIds"] = json!(result.moved_item_ids);
    }
    if !result.deleted_item_ids.is_empty() {
        response["deletedItemIds"] = json!(result.deleted_item_ids);
    }
    Ok(response)
}

fn mutate_presentation_playlist(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let result = workspace::mutate_presentation_playlist(&path, &argument(args, 1)?)?;
    let revision = if result.changed {
        workspace_io.advance_revision(&path)
    } else {
        workspace_io.revision(&path)
    };
    let mut response = json!({
        "workspace": result.workspace,
        "revision": revision,
        "changed": result.changed,
    });
    if result.changed {
        remote_controls::prime_workspace_cache(&path, revision, result.workspace)?;
    }
    if !result.created_playlist_ids.is_empty() {
        response["createdPlaylistIds"] = json!(result.created_playlist_ids);
    }
    if !result.deleted_playlist_ids.is_empty() {
        response["deletedPlaylistIds"] = json!(result.deleted_playlist_ids);
    }
    if !result.created_entry_ids.is_empty() {
        response["createdEntryIds"] = json!(result.created_entry_ids);
    }
    if !result.added_entry_ids.is_empty() {
        response["addedEntryIds"] = json!(result.added_entry_ids);
    }
    if !result.renamed_entry_ids.is_empty() {
        response["renamedEntryIds"] = json!(result.renamed_entry_ids);
    }
    if !result.updated_entry_ids.is_empty() {
        response["updatedEntryIds"] = json!(result.updated_entry_ids);
    }
    if !result.deleted_entry_ids.is_empty() {
        response["deletedEntryIds"] = json!(result.deleted_entry_ids);
    }
    if !result.reordered_entry_ids.is_empty() {
        response["reorderedEntryIds"] = json!(result.reordered_entry_ids);
    }
    if !result.copied_entry_ids.is_empty() {
        response["copiedEntryIds"] = json!(result.copied_entry_ids);
    }
    Ok(response)
}

fn update_output_scene(app: &Host, state: &AppState, value: Value) -> Result<Value, String> {
    let update = output_scene::parse_output_scene_update(&value)?;
    let (kind, session_id, client_sequence, base_revision, payload) = match update {
        output_scene::OutputSceneUpdateV2::Full {
            session_id,
            client_sequence,
            base_revision,
            snapshot,
        } => ("full", session_id, client_sequence, base_revision, snapshot),
        output_scene::OutputSceneUpdateV2::Patch {
            session_id,
            client_sequence,
            base_revision,
            patch,
        } => (
            "patch",
            session_id,
            client_sequence,
            Some(base_revision),
            patch,
        ),
    };
    let fingerprint_payload = match (kind, base_revision) {
        ("patch", Some(revision)) => json!({ "baseRevision": revision, "patch": &payload }),
        ("full", Some(revision)) => json!({ "baseRevision": revision, "snapshot": &payload }),
        _ => payload.clone(),
    };
    let fingerprint = output_scene::update_fingerprint(kind, &fingerprint_payload);
    // Snapshot RSS state before taking protocol/cache locks. Fetch workers
    // release the RSS lock before rebuilding output, so this ordering keeps
    // network completion from ever forming a lock cycle with the protocol.
    let rss_snapshot = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.".to_string())?
        .snapshot();

    // Protocol metadata, the accepted raw snapshot, and its native revision
    // share one coordinator lock. Raw scene callers may invalidate only the
    // cache, while sequence history remains available for a full re-anchor.
    let mut snapshot_state = lock_output_snapshot_state(&state.output_snapshot)?;

    let protocol_decision = validate_output_snapshot_protocol(
        snapshot_state.protocol.as_ref(),
        snapshot_state.cache.is_some(),
        kind,
        &session_id,
        client_sequence,
        &fingerprint,
    )?;
    match protocol_decision {
        OutputSnapshotProtocolDecision::Stale => {
            return Ok(output_scene_stale(snapshot_state.cache.as_ref()));
        }
        OutputSnapshotProtocolDecision::CacheMiss => {
            return Ok(output_scene_cache_miss(snapshot_state.cache.as_ref()));
        }
        OutputSnapshotProtocolDecision::Idempotent | OutputSnapshotProtocolDecision::Accept => {}
    }
    if !snapshot_state.update_base_matches(base_revision) {
        return Ok(output_scene_cache_miss(snapshot_state.cache.as_ref()));
    }
    match protocol_decision {
        OutputSnapshotProtocolDecision::Idempotent => {
            let Some(cached) = snapshot_state.cache.as_ref() else {
                return Ok(output_scene_cache_miss(None));
            };
            update_remote_status_from_output_snapshot(state, &cached.snapshot)?;
            let pending_rss_urls = pending_rss_urls_for_snapshot(&cached.snapshot, &rss_snapshot)?;
            let result = output_scene_ack(cached, false);
            drop(snapshot_state);
            schedule_output_rss_refreshes(app, pending_rss_urls);
            return Ok(result);
        }
        OutputSnapshotProtocolDecision::Accept => {}
        OutputSnapshotProtocolDecision::Stale | OutputSnapshotProtocolDecision::CacheMiss => {
            unreachable!("terminal protocol decisions returned before base validation")
        }
    }

    let mut snapshot = if kind == "full" {
        payload
    } else {
        let Some(merged) = snapshot_state
            .merge_patch(base_revision.expect("patch revision was parsed"), &payload)?
        else {
            return Ok(output_scene_cache_miss(snapshot_state.cache.as_ref()));
        };
        merged
    };
    normalize_output_snapshot_audio(&mut snapshot)?;
    let hash = if kind == "patch" {
        let base_hash = snapshot_state
            .cache
            .as_ref()
            .map(|cached| cached.hash.as_str())
            .unwrap_or_default();
        output_scene::patched_snapshot_hash(base_hash, &fingerprint)
    } else {
        output_scene::snapshot_hash(&snapshot)
    };

    // A changed snapshot is the only path that rebuilds native scenes and
    // advances the native revision. Client sequence and native revision are
    // intentionally independent clocks.
    if snapshot_state
        .cache
        .as_ref()
        .is_some_and(|cached| cached_output_hash_matches(cached, &hash))
    {
        // A fresh automation session may accept the same output snapshot after
        // teardown or takeover. The native scheduler intentionally clears its
        // presentation/emission state for that session, so re-arm it even
        // though the output scene itself does not need rebuilding.
        let mut automation = state
            .native_automation
            .lock()
            .map_err(|_| "Native automation state is unavailable.".to_string())?;
        if automation.presentation_requires_refresh() {
            let automation_update =
                native_automation::NativeAutomationScheduler::prepare_presentation(&snapshot)?;
            automation.apply_presentation(automation_update);
        }
        drop(automation);
        let cached = snapshot_state.cache.as_mut().expect("cache was checked");
        cached.snapshot = snapshot;
        advance_output_snapshot_protocol(
            &mut snapshot_state.protocol,
            session_id,
            client_sequence,
            fingerprint,
        );
        let cached = snapshot_state.cache.as_ref().expect("cache was preserved");
        update_remote_status_from_output_snapshot(state, &cached.snapshot)?;
        let pending_rss_urls = pending_rss_urls_for_snapshot(&cached.snapshot, &rss_snapshot)?;
        let result = output_scene_ack(cached, false);
        drop(snapshot_state);
        schedule_output_rss_refreshes(app, pending_rss_urls);
        return Ok(result);
    }

    // Build all per-screen choices before playback, audio, or NDI state is
    // mutated. Every output target then consumes the same canonical scenes.
    let canonical_inputs = output_scene::build_output_scene_state(&snapshot)?;
    let automation_update =
        native_automation::NativeAutomationScheduler::prepare_presentation(&snapshot)?;
    let (canonical_inputs, pending_rss_urls) =
        rss_output::resolve_output_scenes(canonical_inputs, &rss_snapshot);
    commit_canonical_output_content(state, canonical_inputs)?;
    let revision = snapshot_state.advance_revision();
    // Queue the physical output before the preview begins its own render.
    schedule_output_consumer_refresh(app);
    // The preview reads output_content and output_revisions directly, so it
    // does not depend on the snapshot coordinator held by this handler.
    let _ = app.emit("output:scene-committed", revision);
    state
        .native_automation
        .lock()
        .map_err(|_| "Native automation state is unavailable.".to_string())?
        .apply_presentation(automation_update);
    snapshot_state.cache = Some(OutputSnapshotCache {
        revision,
        hash: hash.clone(),
        snapshot,
        dirty: false,
    });
    advance_output_snapshot_protocol(
        &mut snapshot_state.protocol,
        session_id,
        client_sequence,
        fingerprint,
    );
    if let Some(cached) = snapshot_state.cache.as_ref() {
        update_remote_status_from_output_snapshot(state, &cached.snapshot)?;
    }
    let mut result = serde_json::Map::new();
    result.insert("ok".to_string(), json!(true));
    result.insert("revision".to_string(), json!(revision));
    result.insert("hash".to_string(), json!(hash));
    result.insert("changed".to_string(), json!(true));
    let result = Value::Object(result);
    drop(snapshot_state);
    schedule_output_rss_refreshes(app, pending_rss_urls);
    Ok(result)
}

fn standalone_native_due_action(event: &Value) -> Option<&Value> {
    if event.get("kind").and_then(Value::as_str) != Some("actions") {
        return None;
    }
    for key in ["sessionId", "source", "id"] {
        event
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())?;
    }
    let actions = event.get("actions")?.as_array()?;
    if actions.len() != 1 {
        return None;
    }
    let action = actions.first()?;
    if action.get("enabled").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    if action
        .get("delaySeconds")
        .is_some_and(|delay| delay.as_f64() != Some(0.0))
    {
        return None;
    }
    Some(action)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NativeDueOutputAction {
    ClearAudio,
    ClearBackground,
    ClearMessage(Option<String>),
    ClearProps,
    SelectMedia(NativeDueMediaAction),
    SetMediaPaused(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeDueMediaAction {
    target_id: String,
    playback: String,
    end_behavior: String,
    transition: String,
    behavior: Option<String>,
    scale: Option<String>,
    /// The action's own media effect chain, which replaces the asset's chain.
    effects: Option<Value>,
    transport: Option<Value>,
}

fn native_due_output_action(event: &Value) -> Option<NativeDueOutputAction> {
    let action = standalone_native_due_action(event)?;
    if action
        .get("details")
        .and_then(|details| details.get("expectedMarkerSelectionId"))
        .is_some()
    {
        return None;
    }
    match action.get("type").and_then(Value::as_str) {
        Some("clear") => match action.get("targetLayer").and_then(Value::as_str) {
            Some("audio") => Some(NativeDueOutputAction::ClearAudio),
            Some("background") => Some(NativeDueOutputAction::ClearBackground),
            Some("messages") => {
                let expected_instance_id = match action.get("details") {
                    None | Some(Value::Null) => None,
                    Some(Value::Object(details)) => {
                        match details.get("expectedMessageInstanceId") {
                            None | Some(Value::Null) => None,
                            Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
                            Some(_) => return None,
                        }
                    }
                    Some(_) => return None,
                };
                Some(NativeDueOutputAction::ClearMessage(expected_instance_id))
            }
            Some("prop") => Some(NativeDueOutputAction::ClearProps),
            _ => None,
        },
        Some("transport") => {
            let details = match action.get("details") {
                None | Some(Value::Null) => None,
                Some(Value::Object(details)) => Some(details),
                Some(_) => return None,
            };
            if details.is_some_and(|details| details.contains_key("jumpToTime")) {
                return None;
            }
            let pause = match details.and_then(|details| details.get("pause")) {
                None | Some(Value::Null) => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => return None,
            };
            let _play = match details.and_then(|details| details.get("play")) {
                None | Some(Value::Null) => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => return None,
            };
            Some(NativeDueOutputAction::SetMediaPaused(pause))
        }
        Some("media") => native_due_media_action(action).map(NativeDueOutputAction::SelectMedia),
        _ => None,
    }
}

fn native_due_media_action(action: &Value) -> Option<NativeDueMediaAction> {
    if action
        .get("details")
        .and_then(|details| details.get("unsupportedMediaTransport"))
        .is_some()
    {
        return None;
    }
    let target_id = action
        .get("targetId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?
        .to_string();
    let details = match action.get("details") {
        None | Some(Value::Null) => None,
        Some(Value::Object(details)) => Some(details),
        Some(_) => return None,
    };
    for key in ["effects", "markers"] {
        match details.and_then(|details| details.get(key)) {
            None | Some(Value::Null) => {}
            Some(Value::Array(values)) if values.is_empty() => {}
            _ => return None,
        }
    }
    let effects = match details.and_then(|details| details.get("mediaEffects")) {
        None | Some(Value::Null) => None,
        Some(chain) => {
            native_media_effects::validate_chain(chain).ok()?;
            Some(chain.clone())
        }
    };
    let transport = match details.and_then(|details| details.get("mediaTransport")) {
        None => None,
        Some(value) => {
            output_playback::validate_media_transport(value).ok()?;
            Some(value.clone())
        }
    };
    let playback = optional_media_action_string(details, "playback")?;
    let end_behavior = optional_media_action_string(details, "endBehavior")?;
    let behavior = optional_media_action_string(details, "backgroundBehavior")?;
    let scale = optional_media_action_string(details, "backgroundScale")?;
    if behavior.is_some_and(|value| !["background", "foreground"].contains(&value))
        || scale.is_some_and(|value| !["fit", "fill", "stretch"].contains(&value))
    {
        return None;
    }
    let recorded_transition = optional_media_action_string(details, "backgroundTransition")?;
    let transition_details = match details.and_then(|details| details.get("transition")) {
        None | Some(Value::Null) => None,
        Some(Value::Object(transition)) => Some(transition),
        Some(_) => return None,
    };
    let direct_duration = optional_media_action_number(details, "transitionDuration")?;
    let nested_duration = optional_media_action_number(transition_details, "durationSeconds")?;
    let duration = direct_duration
        .filter(|value| value.abs() > f64::EPSILON)
        .or(nested_duration)
        .unwrap_or(0.0);
    let effect = match transition_details.and_then(|transition| transition.get("effect")) {
        None | Some(Value::Null) => "dissolve".to_string(),
        Some(Value::Object(effect)) => optional_media_action_string(Some(effect), "renderId")?
            .filter(|value| !value.is_empty())
            .or(optional_media_action_string(Some(effect), "name")?
                .filter(|value| !value.is_empty()))
            .unwrap_or("dissolve")
            .to_string(),
        Some(_) => return None,
    };
    Some(NativeDueMediaAction {
        target_id,
        playback: if playback.is_some_and(|value| value.contains("LOOP")) {
            "loop"
        } else {
            "stop"
        }
        .to_string(),
        end_behavior: match end_behavior.unwrap_or_default() {
            value if value.contains("FADE_TO_BLACK") => "fade-black",
            value if value.contains("FADE_TO_CLEAR") => "fade-clear",
            value if value.contains("STOP_ON_BLACK") => "black",
            value if value.contains("STOP_ON_CLEAR") => "clear",
            _ => "hold",
        }
        .to_string(),
        transition: if let Some(transition) = recorded_transition {
            transition.to_string()
        } else if duration > 0.0 {
            format!("fade:{duration}:{effect}")
        } else {
            "none".to_string()
        },
        behavior: behavior.map(str::to_string),
        scale: scale.map(str::to_string),
        effects,
        transport,
    })
}

fn optional_media_action_string<'a>(
    details: Option<&'a serde_json::Map<String, Value>>,
    key: &str,
) -> Option<Option<&'a str>> {
    match details.and_then(|details| details.get(key)) {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(value)) => Some(Some(value)),
        Some(_) => None,
    }
}

fn optional_media_action_number(
    details: Option<&serde_json::Map<String, Value>>,
    key: &str,
) -> Option<Option<f64>> {
    match details.and_then(|details| details.get(key)) {
        None | Some(Value::Null) => Some(None),
        Some(value) => media_action_number_value(value).map(Some),
    }
}

fn media_action_number_value(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn clear_audio_in_output_snapshot(snapshot: &Value) -> Result<Value, String> {
    let mut snapshot = snapshot.clone();
    let snapshot_object = snapshot
        .as_object_mut()
        .ok_or_else(|| "Cached output scene snapshot must be an object.".to_string())?;
    snapshot_object.insert("audioId".to_string(), Value::Null);
    snapshot_object.insert("selectedAudio".to_string(), Value::Null);
    Ok(snapshot)
}

fn clear_all_in_output_snapshot(snapshot: &Value) -> Result<(Value, Value), String> {
    let mut snapshot = snapshot.clone();
    let snapshot_object = snapshot
        .as_object_mut()
        .ok_or_else(|| "Cached output scene snapshot must be an object.".to_string())?;

    let already_cleared = snapshot_object.get("liveItem").is_none_or(Value::is_null)
        && snapshot_object.get("liveItemId").is_none_or(Value::is_null)
        && snapshot_object.get("liveSlide").is_none_or(Value::is_null)
        && snapshot_object
            .get("liveSlideId")
            .is_none_or(Value::is_null)
        && snapshot_object
            .get("liveCueInstanceId")
            .is_none_or(Value::is_null)
        && snapshot_object
            .get("livePlaylistId")
            .is_none_or(Value::is_null)
        && snapshot_object
            .get("livePlaylistEntryId")
            .is_none_or(Value::is_null)
        && snapshot_object
            .get("activeActionSlide")
            .is_none_or(Value::is_null)
        && snapshot_object.get("audioId").is_none_or(Value::is_null)
        && snapshot_object
            .get("liveVideoInputId")
            .is_none_or(Value::is_null)
        && snapshot_object
            .get("selectedAudio")
            .is_none_or(Value::is_null)
        && snapshot_object.get("backgroundId") == Some(&json!(""))
        && snapshot_object
            .get("backgroundPlaybackState")
            .is_none_or(Value::is_null)
        && snapshot_object
            .get("transportPlaybackPaused")
            .and_then(Value::as_object)
            .is_some_and(|playback| {
                playback.get("media").and_then(Value::as_bool) == Some(false)
                    && playback.get("music").and_then(Value::as_bool) == Some(false)
            });
    let selection_id = if already_cleared {
        snapshot_object
            .get("backgroundSelectionId")
            .cloned()
            .unwrap_or_else(|| json!(0.0))
    } else {
        json!(
            snapshot_object
                .get("backgroundSelectionId")
                .and_then(media_action_number_value)
                .unwrap_or(0.0)
                + 1.0
        )
    };
    let transport_playback_paused = json!({ "media": false, "music": false });

    // Keep this mutation aligned with the renderer's clearLiveOutput action,
    // while also removing the derived live values that native automation uses
    // to decide whether a presentation is active.
    snapshot_object.insert("activeActionSlide".to_string(), Value::Null);
    snapshot_object.insert("liveItem".to_string(), Value::Null);
    snapshot_object.insert("liveItemId".to_string(), Value::Null);
    snapshot_object.insert("liveVideoInputId".to_string(), Value::Null);
    snapshot_object.insert("liveSlide".to_string(), Value::Null);
    snapshot_object.insert("liveSlideId".to_string(), Value::Null);
    snapshot_object.insert("liveCueInstanceId".to_string(), Value::Null);
    snapshot_object.insert("livePlaylistId".to_string(), Value::Null);
    snapshot_object.insert("livePlaylistEntryId".to_string(), Value::Null);
    snapshot_object.insert("audioId".to_string(), Value::Null);
    snapshot_object.insert("selectedAudio".to_string(), Value::Null);
    snapshot_object.insert("backgroundId".to_string(), json!(""));
    snapshot_object.insert("backgroundSelectionId".to_string(), selection_id.clone());
    snapshot_object.insert("backgroundPlaybackState".to_string(), Value::Null);
    snapshot_object.insert(
        "transportPlaybackPaused".to_string(),
        transport_playback_paused.clone(),
    );

    let patch = json!({
        "activeActionSlide": null,
        "liveVideoInputId": null,
        "liveItemId": null,
        "liveSlideId": null,
        "liveCueInstanceId": null,
        "livePlaylistId": null,
        "audioId": null,
        "backgroundId": "",
        "backgroundSelectionId": selection_id,
        "backgroundPlaybackState": null,
        "transportPlaybackPaused": transport_playback_paused,
    });
    Ok((snapshot, patch))
}

fn replace_output_snapshot_screen_config(
    snapshot: &Value,
    screen_config: Value,
) -> Result<Value, String> {
    if !screen_config.is_object() {
        return Err("Screen configuration must be an object.".to_string());
    }
    let mut snapshot = snapshot.clone();
    snapshot
        .as_object_mut()
        .ok_or_else(|| "Cached output scene snapshot must be an object.".to_string())?
        .insert("screenConfig".to_string(), screen_config);
    Ok(snapshot)
}

fn clear_message_in_output_snapshot(
    snapshot: &Value,
    expected_instance_id: Option<&str>,
) -> Result<Option<Value>, String> {
    let mut snapshot = snapshot.clone();
    let snapshot_object = snapshot
        .as_object_mut()
        .ok_or_else(|| "Cached output scene snapshot must be an object.".to_string())?;
    if expected_instance_id.is_some_and(|expected| {
        snapshot_object
            .get("activeMessageInstanceId")
            .and_then(Value::as_str)
            != Some(expected)
    }) {
        return Ok(None);
    }
    snapshot_object.insert("activeMessageId".to_string(), Value::Null);
    snapshot_object.insert("activeMessageInstanceId".to_string(), Value::Null);
    snapshot_object.insert("activeMessageText".to_string(), json!(""));
    Ok(Some(snapshot))
}

fn clear_output_field(snapshot: &Value, field: &str, value: Value) -> Result<Value, String> {
    let mut snapshot = snapshot.clone();
    snapshot
        .as_object_mut()
        .ok_or_else(|| "Cached output scene snapshot must be an object.".to_string())?
        .insert(field.to_string(), value);
    Ok(snapshot)
}

fn set_transport_paused_in_output_snapshot(
    snapshot: &Value,
    layer: output_playback::PlaybackLayer,
    paused: bool,
) -> Result<(Value, Value), String> {
    let mut snapshot = snapshot.clone();
    let snapshot_object = snapshot
        .as_object_mut()
        .ok_or_else(|| "Cached output scene snapshot must be an object.".to_string())?;
    let playback = snapshot_object
        .entry("transportPlaybackPaused".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "Cached transport playback state must be an object.".to_string())?;
    playback.insert(layer.as_str().to_string(), json!(paused));
    let playback_patch = Value::Object(playback.clone());
    Ok((snapshot, playback_patch))
}

fn snapshot_transport_paused(
    snapshot: &Value,
    layer: output_playback::PlaybackLayer,
) -> Option<bool> {
    snapshot
        .get("transportPlaybackPaused")
        .and_then(Value::as_object)
        .and_then(|playback| playback.get(layer.as_str()))
        .and_then(Value::as_bool)
}

fn toggled_transport_target(
    native_paused: Option<bool>,
    snapshot: &Value,
    layer: output_playback::PlaybackLayer,
) -> bool {
    !native_paused
        .or_else(|| snapshot_transport_paused(snapshot, layer))
        .unwrap_or(false)
}

fn select_media_in_output_snapshot(
    snapshot: &Value,
    action: &NativeDueMediaAction,
) -> Result<Option<(Value, Value)>, String> {
    let mut snapshot = snapshot.clone();
    let snapshot_object = snapshot
        .as_object_mut()
        .ok_or_else(|| "Cached output scene snapshot must be an object.".to_string())?;
    let media = snapshot_object
        .get_mut("media")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "Cached output media must be an array.".to_string())?;
    let Some(index) = media
        .iter()
        .position(|candidate| candidate["id"].as_str() == Some(action.target_id.as_str()))
        .or_else(|| {
            media.iter().position(|candidate| {
                candidate["sourceId"].as_str() == Some(action.target_id.as_str())
            })
        })
    else {
        return Ok(None);
    };
    if action.transport.is_some()
        && media[index].get("type").and_then(Value::as_str) != Some("video")
    {
        return Err("Media transport settings require a video source.".into());
    }
    if media[index].get("type").and_then(Value::as_str) == Some("audio") {
        return Ok(None);
    }
    let media_id = media[index]
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Selected output media must have an id.".to_string())?
        .to_string();
    for candidate in media
        .iter_mut()
        .filter(|candidate| candidate.get("id").and_then(Value::as_str) == Some(media_id.as_str()))
    {
        let media_object = candidate
            .as_object_mut()
            .ok_or_else(|| "Selected output media must be an object.".to_string())?;
        media_object.insert("backgroundPlayback".to_string(), json!(action.playback));
        media_object.insert(
            "backgroundEndBehavior".to_string(),
            json!(action.end_behavior),
        );
    }
    let transport = action
        .transport
        .clone()
        .or_else(|| media[index].get("mediaTransport").cloned());
    let media = media.clone();
    let selection_id = snapshot_object
        .get("backgroundSelectionId")
        .and_then(media_action_number_value)
        .unwrap_or(0.0)
        + 1.0;
    snapshot_object.insert("backgroundId".to_string(), json!(media_id));
    snapshot_object.insert("backgroundSelectionId".to_string(), json!(selection_id));
    snapshot_object.insert("backgroundPlayback".to_string(), json!(action.playback));
    snapshot_object.insert(
        "backgroundEndBehavior".to_string(),
        json!(action.end_behavior),
    );
    snapshot_object.insert("backgroundTransition".to_string(), json!(action.transition));
    let mut patch = json!({
        "backgroundId": media_id,
        "backgroundSelectionId": selection_id,
        "backgroundPlayback": action.playback,
        "backgroundEndBehavior": action.end_behavior,
        "backgroundTransition": action.transition,
        "media": media
    });
    let behavior = action.behavior.as_deref().unwrap_or_else(|| {
        if media[index]["backgroundBehavior"] == "background" {
            "background"
        } else {
            "foreground"
        }
    });
    snapshot_object.insert("backgroundBehavior".into(), json!(behavior));
    patch["backgroundBehavior"] = json!(behavior);
    if let Some(scale) = &action.scale {
        snapshot_object.insert("backgroundScale".into(), json!(scale));
        patch["backgroundScale"] = json!(scale);
    }
    let playback_state = json!({ "selectionId": selection_id, "status": "playing" });
    snapshot_object.insert(
        "backgroundPlaybackState".to_string(),
        playback_state.clone(),
    );
    patch["backgroundPlaybackState"] = playback_state;
    let effects_override = action.effects.as_ref().map_or(
        Value::Null,
        |chain| json!({ "selectionId": selection_id, "chain": chain }),
    );
    snapshot_object.insert(
        "backgroundEffectsOverride".to_string(),
        effects_override.clone(),
    );
    patch["backgroundEffectsOverride"] = effects_override;
    let transport_override = transport.as_ref().map_or(Value::Null, |transport| {
        let mut value = json!({"selectionId":selection_id,"transport":transport});
        if let Some(behavior) = &action.behavior {
            value["behavior"] = json!(behavior);
        }
        value
    });
    snapshot_object.insert(
        "backgroundTransportOverride".into(),
        transport_override.clone(),
    );
    patch["backgroundTransportOverride"] = transport_override;
    resume_selected_output_layer(snapshot_object, "media", &mut patch)?;
    Ok(Some((snapshot, patch)))
}

fn select_audio_in_output_snapshot(
    snapshot: &Value,
    action: &NativeDueMediaAction,
) -> Result<Option<(Value, Value)>, String> {
    let mut snapshot = snapshot.clone();
    let snapshot_object = snapshot
        .as_object_mut()
        .ok_or_else(|| "Cached output scene snapshot must be an object.".to_string())?;
    let media = snapshot_object
        .get("media")
        .and_then(Value::as_array)
        .ok_or_else(|| "Cached output media must be an array.".to_string())?;
    let Some(media) = media
        .iter()
        .find(|candidate| candidate["id"].as_str() == Some(action.target_id.as_str()))
        .or_else(|| {
            media
                .iter()
                .find(|candidate| candidate["sourceId"].as_str() == Some(action.target_id.as_str()))
        })
    else {
        return Ok(None);
    };
    if media.get("type").and_then(Value::as_str) != Some("audio") {
        return Ok(None);
    }
    let media_id = media
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Selected audio media must have an id.".to_string())?
        .to_string();
    let selected_audio = media.clone();
    let previous_selection_id = match snapshot_object.get("audioSelectionId") {
        None | Some(Value::Null) => 0,
        Some(value) => value
            .as_u64()
            .ok_or("Audio selection identity must be an unsigned integer.")?,
    };
    let selection_id = previous_selection_id
        .checked_add(1)
        .ok_or("Audio selection identity is exhausted.")?;
    snapshot_object.insert("audioId".to_string(), json!(&media_id));
    snapshot_object.insert("audioSelectionId".to_string(), json!(selection_id));
    snapshot_object.insert("selectedAudio".to_string(), selected_audio);
    let mut patch = json!({ "audioId": media_id, "audioSelectionId": selection_id });
    resume_selected_output_layer(snapshot_object, "music", &mut patch)?;
    Ok(Some((snapshot, patch)))
}

fn resume_selected_output_layer(
    snapshot: &mut serde_json::Map<String, Value>,
    layer: &str,
    patch: &mut Value,
) -> Result<(), String> {
    let playback = snapshot
        .entry("transportPlaybackPaused".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "Cached transport playback state must be an object.".to_string())?;
    playback.insert(layer.to_string(), json!(false));
    patch["transportPlaybackPaused"] = Value::Object(playback.clone());
    let (screen_config, _) = show_output_snapshot_look_layer(snapshot.get("screenConfig"), layer)?;
    snapshot.insert("screenConfig".to_string(), screen_config.clone());
    patch["screenConfig"] = screen_config;
    Ok(())
}

fn apply_native_output_due_event(
    app: &Host,
    state: &AppState,
    event: &Value,
) -> Result<Option<Value>, String> {
    let Some(action) = native_due_output_action(event) else {
        return Ok(None);
    };
    let rss_snapshot = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.".to_string())?
        .snapshot();
    let mut snapshot_state = state
        .output_snapshot
        .lock()
        .map_err(|_| "Output snapshot state is unavailable.".to_string())?;
    let mut automation = state
        .native_automation
        .lock()
        .map_err(|_| "Native automation state is unavailable.".to_string())?;
    if !automation.accepts_due_event(event) {
        return Ok(None);
    }
    let Some(cached) = snapshot_state.cache.as_ref() else {
        return Ok(None);
    };
    let (snapshot, renderer_patch, transport_change) = match action {
        NativeDueOutputAction::ClearAudio => (
            clear_audio_in_output_snapshot(&cached.snapshot)?,
            json!({ "audioId": null }),
            None,
        ),
        NativeDueOutputAction::ClearBackground => (
            clear_output_field(&cached.snapshot, "backgroundId", Value::Null)?,
            json!({ "backgroundId": null }),
            None,
        ),
        NativeDueOutputAction::ClearMessage(expected_instance_id) => {
            let Some(snapshot) = clear_message_in_output_snapshot(
                &cached.snapshot,
                expected_instance_id.as_deref(),
            )?
            else {
                return Ok(None);
            };
            (
                snapshot,
                json!({
                    "activeMessageId": null,
                    "activeMessageInstanceId": null,
                    "activeMessageText": ""
                }),
                None,
            )
        }
        NativeDueOutputAction::ClearProps => (
            clear_output_field(&cached.snapshot, "livePropIds", json!([]))?,
            json!({ "livePropIds": [] }),
            None,
        ),
        NativeDueOutputAction::SelectMedia(action) => {
            if let Some((snapshot, renderer_patch)) =
                select_media_in_output_snapshot(&cached.snapshot, &action)?
            {
                (
                    snapshot,
                    renderer_patch,
                    Some((output_playback::PlaybackLayer::Media, false)),
                )
            } else if let Some((snapshot, renderer_patch)) =
                select_audio_in_output_snapshot(&cached.snapshot, &action)?
            {
                (
                    snapshot,
                    renderer_patch,
                    Some((output_playback::PlaybackLayer::Music, false)),
                )
            } else {
                return Ok(None);
            }
        }
        NativeDueOutputAction::SetMediaPaused(paused) => {
            let (snapshot, playback) = set_transport_paused_in_output_snapshot(
                &cached.snapshot,
                output_playback::PlaybackLayer::Media,
                paused,
            )?;
            (
                snapshot,
                json!({ "transportPlaybackPaused": playback }),
                Some((output_playback::PlaybackLayer::Media, paused)),
            )
        }
    };
    let (revision, pending_rss_urls) = commit_native_output_snapshot(
        state,
        &mut snapshot_state,
        snapshot,
        &rss_snapshot,
        transport_change,
        Some(&mut automation),
    )?;
    drop(snapshot_state);
    schedule_output_rss_refreshes(app, pending_rss_urls);
    let mut patch_event = serde_json::Map::new();
    for key in ["sessionId", "source", "id", "liveKey", "timelineGeneration"] {
        if let Some(value) = event.get(key) {
            patch_event.insert(key.to_string(), value.clone());
        }
    }
    patch_event.insert("kind".to_string(), json!("statePatch"));
    patch_event.insert("revision".to_string(), json!(revision));
    patch_event.insert("patch".to_string(), renderer_patch);
    Ok(Some(Value::Object(patch_event)))
}

pub(crate) fn execute_native_remote_command(
    app: &Host,
    command: &str,
    payload: &Value,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    if command == "ticker" {
        return ticker_commands::execute_remote(app, &state, payload);
    }
    if command == "timelineTransport" {
        return remote_timeline_commands::execute(app, &state, payload);
    }
    if command == "capture" {
        return capture_commands::execute_remote(app, &state, payload);
    }
    if command == "media"
        && matches!(
            payload["action"].as_str(),
            Some("seek" | "inPoint" | "outPoint" | "restart")
        )
    {
        return remote_playback_commands::execute(app, &state, payload);
    }
    if matches!(
        command,
        "timer" | "prop" | "cue" | "libraryCue" | "playlistCue" | "stageMessage" | "media"
    ) {
        return remote_controls::execute(app, &state, command, payload);
    }
    if command == "clearLayer" {
        return execute_native_remote_clear_layer(app, &state, payload);
    }
    if matches!(command, "nextSlide" | "previousSlide") {
        return execute_native_remote_slide_navigation(
            app,
            &state,
            if command == "nextSlide" { 1 } else { -1 },
        );
    }
    if command != "clearAll" {
        return Err("The native remote command handler does not support this command.".to_string());
    }

    let rss_snapshot = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.".to_string())?
        .snapshot();
    let mut snapshot_state = lock_output_snapshot_state(&state.output_snapshot)?;
    let Some(cached_snapshot) = snapshot_state
        .cache
        .as_ref()
        .map(|cached| cached.snapshot.clone())
    else {
        return Ok(json!({
            "ok": false,
            "status": "not-ready",
            "changed": false,
            "revision": snapshot_state.revision,
            "error": "Native output has not accepted an output snapshot yet."
        }));
    };
    let (snapshot, renderer_patch) = clear_all_in_output_snapshot(&cached_snapshot)?;
    let changed = snapshot != cached_snapshot;
    let (revision, pending_rss_urls) = commit_native_output_snapshot(
        &state,
        &mut snapshot_state,
        snapshot,
        &rss_snapshot,
        None,
        None,
    )?;
    drop(snapshot_state);
    schedule_output_rss_refreshes(app, pending_rss_urls);

    // The renderer remains the authoring surface. Let it adopt the committed
    // native revision without replaying clearAll. This event is intentionally
    // independent from automation-session lifetime and is emitted only after
    // the output coordinator lock has been released.
    if changed {
        let event = json!({
            "source": "remote",
            "revision": revision,
            "patch": renderer_patch,
        });
        let _ = app.emit_to("main", "native-output-state-patch", event);
    }

    Ok(json!({
        "ok": true,
        "status": if changed { "applied" } else { "unchanged" },
        "changed": changed,
        "revision": revision,
    }))
}

fn execute_native_remote_slide_navigation(
    app: &Host,
    state: &AppState,
    direction: i8,
) -> Result<Value, String> {
    let rss_snapshot = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.".to_string())?
        .snapshot();
    let mut snapshot_state = lock_output_snapshot_state(&state.output_snapshot)?;
    let Some(cached_snapshot) = snapshot_state
        .cache
        .as_ref()
        .map(|cached| cached.snapshot.clone())
    else {
        return Ok(json!({
            "ok": false,
            "status": "not-ready",
            "changed": false,
            "revision": snapshot_state.revision,
            "error": "Native output has not accepted an output snapshot yet."
        }));
    };
    let Some((snapshot, renderer_patch)) =
        navigate_remote_slide_in_output_snapshot(&cached_snapshot, direction)?
    else {
        return Ok(json!({
            "ok": false,
            "status": "not-ready",
            "changed": false,
            "revision": snapshot_state.revision,
            "error": "Native output has no live presentation to navigate."
        }));
    };
    let changed = snapshot != cached_snapshot;
    let (revision, pending_rss_urls) = commit_native_output_snapshot(
        state,
        &mut snapshot_state,
        snapshot,
        &rss_snapshot,
        None,
        None,
    )?;
    drop(snapshot_state);
    schedule_output_rss_refreshes(app, pending_rss_urls);

    if changed {
        let event = json!({
            "source": "remote",
            "revision": revision,
            "patch": renderer_patch,
        });
        let _ = app.emit_to("main", "native-output-state-patch", event);
    }

    Ok(json!({
        "ok": true,
        "status": if changed { "applied" } else { "unchanged" },
        "changed": changed,
        "revision": revision,
    }))
}

fn navigate_remote_slide_in_output_snapshot(
    snapshot: &Value,
    direction: i8,
) -> Result<Option<(Value, Value)>, String> {
    if !matches!(direction, -1 | 1) {
        return Err("Remote slide navigation direction must be -1 or 1.".to_string());
    }
    let snapshot_object = snapshot
        .as_object()
        .ok_or_else(|| "Cached output scene snapshot must be an object.".to_string())?;
    let Some(live_item) = snapshot_object
        .get("liveItem")
        .filter(|value| value.is_object())
    else {
        return Ok(None);
    };
    let live_instance_id = snapshot_object
        .get("liveCueInstanceId")
        .and_then(Value::as_str);
    let live_playlist_entry_id = snapshot_object
        .get("livePlaylistEntryId")
        .and_then(Value::as_str);
    let Some(slides) = output_arrangement::resolve_live_arrangement_slides(
        live_item,
        live_instance_id,
        live_playlist_entry_id,
    ) else {
        return Ok(None);
    };
    let enabled = |slide: &Value| !remote_json_truthy(slide.get("disabled"));
    if !slides.iter().any(enabled) {
        return Ok(None);
    }

    let live_slide_id = snapshot_object
        .get("liveSlideId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let current_index =
        output_arrangement::live_slide_index(&slides, live_slide_id, live_instance_id);
    if direction > 0 {
        if let Some(index) = current_index.filter(|&index| enabled(&slides[index])) {
            let live_build_step = snapshot_object
                .get("liveBuildStep")
                .and_then(media_action_number_value)
                .unwrap_or(0.0);
            if let Some(next_step) = remote_slide_next_build_step(&slides[index], live_build_step) {
                let started_at_ms = Utc::now().timestamp_millis();
                let mut next = snapshot.clone();
                let next_object = next.as_object_mut().expect("validated snapshot object");
                let timeline_started_at_ms = snapshot_object
                    .get("timelineStartedAtMs")
                    .and_then(Value::as_i64)
                    .filter(|value| *value > 0)
                    .unwrap_or(started_at_ms);
                next_object.insert("liveBuildStep".to_string(), json!(next_step));
                next_object.insert(
                    "timelineStartedAtMs".to_string(),
                    json!(timeline_started_at_ms),
                );
                next_object.insert(
                    "liveBuildStepStartedAtMsByStep".to_string(),
                    add_remote_build_step_clock(
                        snapshot_object.get("liveBuildStepStartedAtMsByStep"),
                        next_step as u64,
                        timeline_started_at_ms,
                        started_at_ms,
                    ),
                );
                let patch = json!({
                    "liveBuildStep": next_step,
                    "timelineStartedAtMs": timeline_started_at_ms,
                    "liveBuildStepStartedAtMsByStep": next["liveBuildStepStartedAtMsByStep"]
                });
                return Ok(Some((next, patch)));
            }
        }
    }

    let mut ordered = slides.iter().enumerate();
    let target = if direction > 0 {
        ordered.find(|(index, slide)| {
            current_index.is_none_or(|current| *index > current) && enabled(slide)
        })
    } else {
        ordered.rfind(|(index, slide)| {
            current_index.is_none_or(|current| *index < current) && enabled(slide)
        })
    };
    let Some((_, target)) = target else {
        return Ok(Some((snapshot.clone(), json!({}))));
    };

    select_remote_slide_in_output_snapshot(snapshot, target)
}

pub(crate) fn select_remote_slide_in_output_snapshot(
    snapshot: &Value,
    target: &Value,
) -> Result<Option<(Value, Value)>, String> {
    let snapshot_object = snapshot
        .as_object()
        .ok_or("Cached output snapshot must be an object.")?;
    let live_item = snapshot.get("liveItem").ok_or("A live item is required.")?;
    let target_id = target
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Remote navigation target slide id is required.".to_string())?;
    let raw_target = live_item
        .get("slides")
        .and_then(Value::as_array)
        .and_then(|slides| {
            slides
                .iter()
                .find(|slide| slide.get("id").and_then(Value::as_str) == Some(target_id))
        })
        .cloned()
        .ok_or_else(|| {
            "Remote navigation target slide was not found in the live item.".to_string()
        })?;
    let target_instance_id = target
        .get("arrangementInstanceId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let live_item_id = live_item
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Remote navigation live item id is required.".to_string())?;
    let timeline_started_at_ms = Utc::now().timestamp_millis();
    let (screen_config, screen_config_changed) =
        show_output_snapshot_look_layer(snapshot_object.get("screenConfig"), "slide")?;
    let background_patch = slide_media_patch(snapshot, &raw_target, false);

    let mut next = snapshot.clone();
    let next_object = next.as_object_mut().expect("validated snapshot object");
    if let Some(background) = &background_patch {
        next_object.extend(
            background
                .as_object()
                .expect("background patch is an object")
                .clone(),
        );
    }
    next_object.insert("liveItemId".to_string(), json!(live_item_id));
    next_object.insert("liveSlide".to_string(), raw_target);
    next_object.insert("liveSlideId".to_string(), json!(target_id));
    next_object.insert(
        "liveCueInstanceId".to_string(),
        target_instance_id
            .clone()
            .map_or(Value::Null, Value::String),
    );
    next_object.insert("liveBuildStep".to_string(), json!(0));
    next_object.insert(
        "liveBuildStepStartedAtMsByStep".to_string(),
        json!({ "0": timeline_started_at_ms }),
    );
    next_object.insert(
        "timelineStartedAtMs".to_string(),
        json!(timeline_started_at_ms),
    );
    if screen_config_changed {
        next_object.insert("screenConfig".to_string(), screen_config.clone());
    }

    let mut patch = serde_json::Map::new();
    patch.insert("liveItem".into(), next["liveItem"].clone());
    patch.insert("liveSlide".into(), next["liveSlide"].clone());
    let media_changed = background_patch.is_some();
    if let Some(background) = background_patch {
        patch.extend(
            background
                .as_object()
                .expect("background patch is an object")
                .clone(),
        );
    }
    patch.insert("liveItemId".to_string(), json!(live_item_id));
    patch.insert("liveSlideId".to_string(), json!(target_id));
    patch.insert(
        "liveCueInstanceId".to_string(),
        target_instance_id.map_or(Value::Null, Value::String),
    );
    patch.insert("liveBuildStep".to_string(), json!(0));
    patch.insert(
        "liveBuildStepStartedAtMsByStep".to_string(),
        json!({ "0": timeline_started_at_ms }),
    );
    patch.insert(
        "timelineStartedAtMs".to_string(),
        json!(timeline_started_at_ms),
    );
    if screen_config_changed {
        patch.insert("screenConfig".to_string(), screen_config);
    }
    let mut patch = Value::Object(patch);
    if media_changed {
        resume_selected_output_layer(
            next.as_object_mut().expect("validated snapshot object"),
            "media",
            &mut patch,
        )?;
    }
    Ok(Some((next, patch)))
}

fn slide_media_patch(
    snapshot: &Value,
    target: &Value,
    clear_removed_background: bool,
) -> Option<Value> {
    let media_catalog = snapshot["media"].as_array().map_or(&[][..], Vec::as_slice);
    let media = target
        .get("mediaBackgroundId")
        .and_then(Value::as_str)
        .and_then(|id| media_catalog.iter().find(|media| media["id"] == id));
    let clear_media = (clear_removed_background || snapshot["backgroundBehavior"] == "foreground")
        && snapshot["backgroundId"].as_str().is_some_and(|id| {
            !id.is_empty()
                && output_scene::slide_connects_media(&snapshot["liveSlide"], id, media_catalog)
                && !output_scene::slide_connects_media(target, id, media_catalog)
        });
    let next_selection_id = || {
        snapshot
            .get("backgroundSelectionId")
            .and_then(media_action_number_value)
            .unwrap_or(0.0)
            + 1.0
    };
    if let Some(media) = media {
        let selection_id = next_selection_id();
        let behavior = if target["mediaBackgroundOnly"] == true {
            "background"
        } else {
            target["mediaBackgroundBehavior"]
                .as_str()
                .unwrap_or_else(|| {
                    if media["backgroundBehavior"] == "background" {
                        "background"
                    } else {
                        "foreground"
                    }
                })
        };
        Some(json!({
            "backgroundId": media["id"],
            "backgroundSelectionId": selection_id,
            "backgroundBehavior": behavior,
            "backgroundEndBehavior": media.get("backgroundEndBehavior").filter(|value| !value.is_null()).cloned().unwrap_or(json!("hold")),
            "backgroundPlayback": media.get("backgroundPlayback").filter(|value| !value.is_null()).cloned().unwrap_or(json!("stop")),
            "backgroundPlaybackState": { "selectionId": selection_id, "status": "playing" },
            "backgroundScale": media.get("backgroundScale").filter(|value| !value.is_null()).cloned().unwrap_or(json!("fit")),
            "backgroundTransition": media.get("backgroundTransition").filter(|value| !value.is_null()).cloned().unwrap_or(json!("none"))
        }))
    } else if clear_media {
        Some(json!({
            "backgroundId": "",
            "backgroundSelectionId": next_selection_id(),
            "backgroundPlaybackState": null,
        }))
    } else {
        None
    }
}

fn add_remote_build_step_clock(
    value: Option<&Value>,
    step: u64,
    timeline_started_at_ms: i64,
    started_at_ms: i64,
) -> Value {
    let mut clock = value
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    clock.retain(|key, _| key.parse::<u64>().is_ok_and(|existing| existing <= step));
    clock.insert("0".to_string(), json!(timeline_started_at_ms));
    clock.insert(step.to_string(), json!(started_at_ms));
    Value::Object(clock)
}

fn remote_slide_next_build_step(slide: &Value, current: f64) -> Option<f64> {
    let mut next = f64::INFINITY;
    for layer in slide
        .get("layers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if layer.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        for key in ["build", "buildOut"] {
            if let Some(step) = layer
                .get(key)
                .and_then(|build| build.get("step"))
                .and_then(media_action_number_value)
                .filter(|step| {
                    *step > 0.0 && step.fract() == 0.0 && *step <= 9_007_199_254_740_991.0
                })
            {
                if step > current {
                    next = next.min(step);
                }
            }
        }
        for step in layer
            .get("textReveal")
            .and_then(|reveal| reveal.get("children"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|child| child.get("step"))
            .filter_map(media_action_number_value)
            .filter(|step| *step > 0.0 && step.fract() == 0.0 && *step <= 9_007_199_254_740_991.0)
        {
            if step > current {
                next = next.min(step);
            }
        }
    }
    next.is_finite().then_some(next)
}

fn show_output_snapshot_look_layer(
    screen_config: Option<&Value>,
    layer_id: &str,
) -> Result<(Value, bool), String> {
    let screens = screen_config
        .and_then(Value::as_object)
        .ok_or_else(|| "Cached output screen configuration must be an object.".to_string())?;
    let mut next = screens.clone();
    let mut changed = false;
    for screen in next.values_mut() {
        let screen = screen
            .as_object_mut()
            .ok_or_else(|| "Cached output screen entries must be objects.".to_string())?;
        let look_layers = screen
            .entry("lookLayers".to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .ok_or_else(|| "Cached output look layers must be an object.".to_string())?;
        if look_layers.get(layer_id).and_then(Value::as_bool) != Some(true) {
            look_layers.insert(layer_id.to_string(), Value::Bool(true));
            changed = true;
        }
    }
    Ok((Value::Object(next), changed))
}

fn remote_json_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
    }
}

fn execute_native_remote_clear_layer(
    app: &Host,
    state: &AppState,
    payload: &Value,
) -> Result<Value, String> {
    let layer_id = native_remote_layer_id(payload)?;
    if layer_id == "videoInput" {
        return remote_controls::execute(app, state, "videoInputClear", payload);
    }
    let mutation = json!({ "type": "clearLookLayer", "layerId": layer_id });
    let persisted = mutate_remote_screen_config(app, state, mutation)?;
    let screen_config = persisted.get("screenConfig").cloned().ok_or_else(|| {
        "Native screen mutation did not return a screen configuration.".to_string()
    })?;
    let persisted_changed = persisted
        .get("changed")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let rss_snapshot = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.".to_string())?
        .snapshot();
    let mut snapshot_state = lock_output_snapshot_state(&state.output_snapshot)?;
    let mut output_changed = false;
    let mut pending_rss_urls = Vec::new();
    let output_revision = if let Some(cached) = snapshot_state.cache.as_ref() {
        let snapshot = replace_output_snapshot_screen_config(&cached.snapshot, screen_config)?;
        output_changed = snapshot != cached.snapshot;
        let (revision, pending) = commit_native_output_snapshot(
            state,
            &mut snapshot_state,
            snapshot,
            &rss_snapshot,
            None,
            None,
        )?;
        pending_rss_urls = pending;
        revision
    } else {
        snapshot_state.revision
    };
    drop(snapshot_state);
    schedule_output_rss_refreshes(app, pending_rss_urls);

    let changed = persisted_changed || output_changed;
    Ok(json!({
        "ok": true,
        "status": if changed { "applied" } else { "unchanged" },
        "changed": changed,
        "revision": output_revision,
        "settingsRevision": persisted.get("settingsRevision").cloned().unwrap_or(Value::Null),
        "workspaceRevision": persisted.get("revision").cloned().unwrap_or(Value::Null),
    }))
}

fn native_remote_layer_id(payload: &Value) -> Result<&str, String> {
    payload
        .get("layer")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "A layer is required.".to_string())
}

fn mutate_remote_screen_config(
    app: &Host,
    state: &AppState,
    mutation: Value,
) -> Result<Value, String> {
    for attempt in 0..3 {
        let workspace_path = state
            .active_workspace
            .lock()
            .map_err(|_| "Workspace state is unavailable.".to_string())?
            .clone();
        let settings_revision = state
            .settings_revision
            .lock()
            .map_err(|_| "Settings state is unavailable.".to_string())?
            .current;
        let workspace_revision = if let Some(path) = workspace_path.as_ref() {
            json!(lock_workspace_io(state)?.revision(path))
        } else {
            Value::Null
        };
        let requested_workspace = workspace_path
            .as_ref()
            .map(|path| json!(path.to_string_lossy().to_string()))
            .unwrap_or(Value::Null);

        match mutate_screen_config(
            app,
            state,
            requested_workspace,
            mutation.clone(),
            settings_revision,
            workspace_revision,
        ) {
            Ok(result) => return Ok(result),
            Err(error)
                if attempt < 2
                    && (error.contains("Workspace changed since revision")
                        || error.contains("Settings changed since revision")
                        || error.contains("workspace is no longer active")
                        || error.contains("workspace is not the active")) => {}
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded screen mutation retry must return")
}

fn commit_native_output_snapshot(
    state: &AppState,
    snapshot_state: &mut OutputSnapshotState,
    snapshot: Value,
    rss_snapshot: &rss_output::OutputRssSnapshot,
    transport_change: Option<(output_playback::PlaybackLayer, bool)>,
    mut automation: Option<&mut native_automation::NativeAutomationScheduler>,
) -> Result<(u64, Vec<String>), String> {
    // Native takes use the same delta hash as protocol patches, without rehashing the unchanged theme catalog.
    let hash = match snapshot_state.cache.as_ref() {
        Some(cached)
            if cached.snapshot.as_object().is_some_and(|previous| {
                snapshot
                    .as_object()
                    .is_some_and(|next| previous.keys().all(|key| next.contains_key(key)))
            }) =>
        {
            let patch = snapshot
                .as_object()
                .expect("snapshot object was checked")
                .iter()
                .filter(|(key, value)| cached.snapshot.get(*key) != Some(*value))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<serde_json::Map<_, _>>();
            if patch.is_empty() {
                cached.hash.clone()
            } else {
                output_scene::patched_snapshot_hash(
                    &cached.hash,
                    &output_scene::snapshot_hash(&Value::Object(patch)),
                )
            }
        }
        _ => output_scene::snapshot_hash(&snapshot),
    };
    if let Some(cached) = snapshot_state.cache.as_ref() {
        if cached_output_hash_matches(cached, &hash) {
            let current_revision = cached.revision;
            if let Some((layer, paused)) = transport_change {
                let command = playback_transport_command(layer, paused);
                let mut mutation = apply_playback_command_to_sources(state, command)?;
                let revision = anchor_native_playback_revision(
                    snapshot_state,
                    mutation.targeted,
                    mutation.changed,
                )
                .unwrap_or(current_revision);
                propagate_transport_source_mutation(
                    state,
                    command,
                    paused,
                    mutation.changed,
                    &mut mutation.payloads,
                )?;
                mark_native_playback_propagated(snapshot_state, Some(revision));
                return Ok((revision, Vec::new()));
            }
            return Ok((current_revision, Vec::new()));
        }
    }
    let canonical_inputs = output_scene::build_output_scene_state(&snapshot)?;
    let automation_update =
        native_automation::NativeAutomationScheduler::prepare_presentation(&snapshot)?;
    let (canonical_inputs, pending_rss_urls) =
        rss_output::resolve_output_scenes(canonical_inputs, rss_snapshot);
    let rebuilt_source_ids = canonical_inputs.keys().cloned().collect::<HashSet<_>>();
    let revision = snapshot_state.advance_revision();
    snapshot_state.cache = Some(OutputSnapshotCache {
        revision,
        hash,
        snapshot,
        dirty: true,
    });
    let commit_result = (|| -> Result<(), String> {
        if let Some((layer, paused)) = transport_change {
            apply_playback_command_to_sources(state, playback_transport_command(layer, paused))?;
        }
        update_canonical_output_content(state, canonical_inputs)?;
        if let Some((layer, paused)) = transport_change {
            refresh_unmapped_transport_sources(state, &rebuilt_source_ids, layer, paused)?;
        }
        if let Some(automation) = automation.as_deref_mut() {
            automation.apply_presentation(automation_update);
        } else {
            state
                .native_automation
                .lock()
                .map_err(|_| "Native automation state is unavailable.".to_string())?
                .apply_presentation(automation_update);
        }
        let accepted = snapshot_state
            .cache
            .as_ref()
            .expect("native output mutation committed a snapshot");
        state
            .native_props
            .lock()
            .map_err(|_| "Native prop state is unavailable.")?
            .retain_active(&accepted.snapshot);
        update_remote_status_from_output_snapshot(state, &accepted.snapshot)
    })();
    if let Err(error) = commit_result {
        snapshot_state.invalidate_cache();
        return Err(error);
    }
    snapshot_state
        .cache
        .as_mut()
        .expect("native output mutation committed a snapshot")
        .dirty = false;
    Ok((revision, pending_rss_urls))
}

fn refresh_unmapped_transport_sources(
    state: &AppState,
    rebuilt_source_ids: &HashSet<String>,
    layer: output_playback::PlaybackLayer,
    paused: bool,
) -> Result<(), String> {
    let active_source_ids = active_native_audio_source_ids(state)?;
    let source_ids = {
        let mut content = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?;
        apply_transport_to_unmapped_content(
            &mut content,
            &active_source_ids,
            rebuilt_source_ids,
            layer,
            paused,
        )?
    };
    if source_ids.is_empty() {
        return Ok(());
    }
    refresh_playback_content(state, source_ids.iter(), true)?;
    present_native_output_sources(state, &source_ids.iter().cloned().collect())?;
    Ok(())
}

fn present_native_output_sources(
    state: &AppState,
    source_ids: &HashSet<String>,
) -> Result<(), String> {
    if source_ids.is_empty() {
        return Ok(());
    }
    let mut outputs = state
        .display_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .values()
        .filter(|output| output.active)
        .cloned()
        .collect::<Vec<_>>();
    outputs.extend(
        state
            .windowed_outputs
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .values()
            .cloned(),
    );
    let mut paired = HashSet::new();
    for output in outputs {
        if source_ids.contains(&output.source_screen_id)
            && native_key_fill::claim(state, &output, &mut paired)?
        {
            present_native_output(state, &output)?;
        }
    }
    Ok(())
}

fn apply_transport_to_unmapped_content(
    content: &mut HashMap<String, Value>,
    active_source_ids: &HashSet<String>,
    rebuilt_source_ids: &HashSet<String>,
    layer: output_playback::PlaybackLayer,
    paused: bool,
) -> Result<Vec<String>, String> {
    let mut updated_source_ids = Vec::new();
    for source_id in active_source_ids.difference(rebuilt_source_ids) {
        let Some(current) = content.get(source_id).cloned() else {
            continue;
        };
        let (updated, _) = set_transport_paused_in_output_snapshot(&current, layer, paused)?;
        content.insert(source_id.clone(), updated);
        updated_source_ids.push(source_id.clone());
    }
    Ok(updated_source_ids)
}

fn normalize_output_snapshot_audio(snapshot: &mut Value) -> Result<(), String> {
    let snapshot = snapshot
        .as_object_mut()
        .ok_or_else(|| "Output scene snapshot must be an object.".to_string())?;
    let audio_settings = output_playback::normalize_audio_settings_value(
        snapshot.get("audioSettings").unwrap_or(&Value::Null),
    );
    snapshot.insert("audioSettings".to_string(), audio_settings);
    Ok(())
}

/// Queue native RSS fetches without holding any output or protocol lock. The
/// worker completion only updates the cache; effective output is rebuilt on
/// Tauri's main thread from the newest raw renderer snapshot. This prevents a
/// late response from applying an old scene or changing the renderer protocol
/// revision/hash.
fn schedule_output_rss_refreshes(app: &Host, urls: Vec<String>) {
    if urls.is_empty() {
        return;
    }

    let requests = {
        let state = app.state::<AppState>();
        let Ok(mut cache) = state.rss_output.lock() else {
            return;
        };
        let now = std::time::Instant::now();
        urls.into_iter()
            .filter_map(|url| {
                cache
                    .begin_fetch(&url, now)
                    .map(|token| rss_output::RssFetchRequest { url, token })
            })
            .collect::<Vec<_>>()
    };
    if requests.is_empty() {
        return;
    }

    let completion_app = app.clone();
    let completion: rss_output::BatchCompletion = Arc::new(move |results| {
        let mut has_success = false;
        if let Some(state) = completion_app.try_state::<AppState>() {
            if let Ok(mut cache) = state.rss_output.lock() {
                let now = std::time::Instant::now();
                for result in results {
                    match result.result {
                        Ok(items) => {
                            has_success |=
                                cache.complete_success(&result.url, &result.token, items, now);
                        }
                        Err(_) => {
                            let _ = cache.complete_error(&result.url, &result.token, now);
                        }
                    }
                }
            }
        }
        if !has_success {
            return;
        }

        let rebuild_app = completion_app.clone();
        let callback_app = rebuild_app.clone();
        let _ = rebuild_app.run_on_main_thread(move || {
            let _ = rebuild_output_from_latest_rss(&callback_app);
        });
    });

    let submit_error = {
        let state = app.state::<AppState>();
        state.rss_service.submit(requests.clone(), completion).err()
    };
    if submit_error.is_some() {
        // A full or disconnected worker queue must not leave requests marked
        // in-flight forever. Treat the failed submission as a transient
        // fetch failure and let the normal backoff apply.
        if let Ok(mut cache) = app.state::<AppState>().rss_output.lock() {
            let now = std::time::Instant::now();
            for request in requests {
                let _ = cache.complete_error(&request.url, &request.token, now);
            }
        }
    }
}

fn rebuild_output_from_latest_rss(app: &Host) -> Result<(), String> {
    let state = app.state::<AppState>();
    let rss_snapshot = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.".to_string())?
        .snapshot();
    // Hold the coordinator while resolving and applying RSS. Otherwise a
    // renderer update could replace the raw snapshot after this function
    // clones it and then have its newer output overwritten by a late fetch.
    let snapshot_state = state
        .output_snapshot
        .lock()
        .map_err(|_| "Output snapshot state is unavailable.".to_string())?;
    let Some(raw_snapshot) = snapshot_state
        .cache
        .as_ref()
        .map(|cached| cached.snapshot.clone())
    else {
        drop(snapshot_state);
        return Ok(());
    };

    let canonical_inputs = output_scene::build_output_scene_state(&raw_snapshot)?;
    let (canonical_inputs, pending_rss_urls) =
        rss_output::resolve_output_scenes(canonical_inputs, &rss_snapshot);
    if canonical_output_content_changed(&state, &canonical_inputs)? {
        update_canonical_output_content(&state, canonical_inputs)?;
    }
    drop(snapshot_state);
    // Revalidation discovered while rebuilding is queued only after all RSS
    // and output locks have been released.
    schedule_output_rss_refreshes(app, pending_rss_urls);
    Ok(())
}

fn pending_rss_urls_for_snapshot(
    raw_snapshot: &Value,
    rss_snapshot: &rss_output::OutputRssSnapshot,
) -> Result<Vec<String>, String> {
    let canonical_inputs = output_scene::build_output_scene_state(raw_snapshot)?;
    let (_, pending_rss_urls) = rss_output::resolve_output_scenes(canonical_inputs, rss_snapshot);
    Ok(pending_rss_urls)
}

fn canonical_output_content_changed(
    state: &AppState,
    canonical_inputs: &serde_json::Map<String, Value>,
) -> Result<bool, String> {
    let current = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?;
    Ok(canonical_inputs.iter().any(|(screen_id, next)| {
        current
            .get(screen_id)
            .map(|previous| {
                output_playback::scroll_content_key_for_value(previous)
                    != output_playback::scroll_content_key_for_value(next)
            })
            .unwrap_or(true)
    }))
}

fn update_remote_status_from_output_snapshot(
    state: &AppState,
    snapshot: &Value,
) -> Result<(), String> {
    let status = remote::build_output_remote_status(snapshot)?;
    state.remote_status.update(status)
}

fn output_scene_cache_miss(cached: Option<&OutputSnapshotCache>) -> Value {
    let mut result = serde_json::Map::new();
    result.insert("ok".to_string(), json!(false));
    result.insert("cacheMiss".to_string(), json!(true));
    if let Some(cached) = cached {
        result.insert("revision".to_string(), json!(cached.revision));
        result.insert("hash".to_string(), json!(cached.hash));
        result.insert("snapshot".to_string(), cached.snapshot.clone());
    }
    Value::Object(result)
}

fn output_scene_stale(cached: Option<&OutputSnapshotCache>) -> Value {
    let result = cached
        .map(|cached| output_scene_ack(cached, false))
        .unwrap_or_else(|| json!({ "ok": true, "changed": false }));
    let result = result
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);
    let mut result = result;
    result.insert("stale".to_string(), json!(true));
    Value::Object(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputSnapshotProtocolDecision {
    Accept,
    CacheMiss,
    Idempotent,
    Stale,
}

fn validate_output_snapshot_protocol(
    protocol: Option<&OutputSnapshotProtocol>,
    cache_present: bool,
    kind: &str,
    session_id: &str,
    client_sequence: u64,
    fingerprint: &str,
) -> Result<OutputSnapshotProtocolDecision, String> {
    let Some(last) = protocol else {
        return Ok(if kind == "full" {
            OutputSnapshotProtocolDecision::Accept
        } else {
            OutputSnapshotProtocolDecision::CacheMiss
        });
    };

    if session_id != last.session_id && last.retired_session_ids.contains(session_id) {
        return Ok(OutputSnapshotProtocolDecision::Stale);
    }
    if session_id != last.session_id {
        return Ok(if kind == "full" {
            OutputSnapshotProtocolDecision::Accept
        } else {
            OutputSnapshotProtocolDecision::CacheMiss
        });
    }
    if client_sequence < last.client_sequence {
        return Ok(OutputSnapshotProtocolDecision::Stale);
    }
    if client_sequence == last.client_sequence {
        if fingerprint != last.fingerprint {
            return Err(format!(
                "Output scene client sequence {client_sequence} was already accepted with a different payload."
            ));
        }
        if kind == "patch" && !cache_present {
            return Ok(OutputSnapshotProtocolDecision::CacheMiss);
        }
        return Ok(if cache_present {
            OutputSnapshotProtocolDecision::Idempotent
        } else {
            // A duplicated full can restore a cache lost by a raw scene
            // caller without consuming another client sequence.
            OutputSnapshotProtocolDecision::Accept
        });
    }
    if client_sequence > last.client_sequence.saturating_add(1) && kind == "patch" {
        return Ok(OutputSnapshotProtocolDecision::CacheMiss);
    }
    if kind == "patch" && !cache_present {
        return Ok(OutputSnapshotProtocolDecision::CacheMiss);
    }
    Ok(OutputSnapshotProtocolDecision::Accept)
}

fn advance_output_snapshot_protocol(
    protocol: &mut Option<OutputSnapshotProtocol>,
    session_id: String,
    client_sequence: u64,
    fingerprint: String,
) {
    if let Some(state) = protocol.as_mut() {
        if state.session_id != session_id {
            state.retired_session_ids.insert(state.session_id.clone());
            state.session_id = session_id;
        }
        state.client_sequence = client_sequence;
        state.fingerprint = fingerprint;
        return;
    }
    *protocol = Some(OutputSnapshotProtocol {
        session_id,
        client_sequence,
        fingerprint,
        retired_session_ids: HashSet::new(),
    });
}

fn output_scene_ack(cached: &OutputSnapshotCache, changed: bool) -> Value {
    json!({
        "ok": true,
        "changed": changed,
        "revision": cached.revision,
        "hash": cached.hash,
    })
}

fn update_canonical_output_content(
    state: &AppState,
    canonical_inputs: serde_json::Map<String, Value>,
) -> Result<Value, String> {
    commit_canonical_output_content(state, canonical_inputs)?;
    refresh_output_consumers(state)?;
    Ok(json!({ "ok": true }))
}

fn commit_canonical_output_content(
    state: &AppState,
    canonical_inputs: serde_json::Map<String, Value>,
) -> Result<(), String> {
    if let Some(content) = canonical_inputs.values().next() {
        let frozen = content["outputFrozen"]
            .as_bool()
            .ok_or("Output freeze state must be a boolean.")?;
        let changed = *state
            .output_freeze
            .read()
            .map_err(|_| "Output freeze state is unavailable.")?
            != frozen;
        if changed {
            let mut current = state
                .output_freeze
                .write()
                .map_err(|_| "Output freeze state is unavailable.")?;
            if *current != frozen {
                state
                    .ndi
                    .lock()
                    .map_err(|_| "NDI state is unavailable.")?
                    .set_frozen(frozen);
                *current = frozen;
            }
        }
    }
    let now = std::time::Instant::now();
    let mut canonical_outputs = {
        let mut playback = state
            .output_playback
            .lock()
            .map_err(|_| "Output playback state is unavailable.".to_string())?;
        canonical_inputs
            .iter()
            .map(|(screen_id, output)| {
                (
                    screen_id.clone(),
                    playback.sync_content(screen_id, output, now),
                )
            })
            .collect::<serde_json::Map<_, _>>()
    };
    let new_main_ids = canonical_outputs
        .iter()
        .filter(|(_, content)| is_main_transport_content(content))
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    let active_ids = active_native_audio_source_ids_with(state, &new_main_ids)?;
    synchronize_native_audio_outputs(state, &mut canonical_outputs, &active_ids, now)?;
    {
        let mut content = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?;
        for (screen_id, output) in canonical_outputs.iter() {
            content.insert(screen_id.clone(), output.clone());
        }
        state
            .native_key_fill_pairs
            .lock()
            .map_err(|_| "Native key/fill state is unavailable.")?
            .reconcile_scenes(&content);
    }
    {
        let mut revisions = state
            .output_revisions
            .lock()
            .map_err(|_| "Output revision state is unavailable.".to_string())?;
        for screen_id in canonical_inputs.keys() {
            let revision = revisions.entry(screen_id.clone()).or_default();
            *revision = revision.saturating_add(1);
        }
    }
    {
        let mut cache = state
            .native_frame_cache
            .lock()
            .map_err(|_| "Native frame cache is unavailable.".to_string())?;
        cache.retain(|key, _| !canonical_inputs.contains_key(&key.source_screen_id));
    }
    Ok(())
}

fn refresh_output_consumers(state: &AppState) -> Result<(), String> {
    let mut windows = state
        .display_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .values()
        .filter(|output| output.active)
        .cloned()
        .collect::<Vec<_>>();
    windows.extend(
        state
            .windowed_outputs
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .values()
            .cloned(),
    );
    let mut paired = HashSet::new();
    for output in &windows {
        if native_key_fill::claim(state, output, &mut paired)? {
            present_native_output(state, output)?;
        }
    }
    let ndi_source_ids = state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .active_output_ids();
    if ndi_source_ids.is_empty() {
        return Ok(());
    }
    let canonical_outputs = {
        let output_content = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?;
        ndi_source_ids
            .iter()
            .filter_map(|source_id| {
                output_content
                    .get(source_id)
                    .cloned()
                    .map(|content| (source_id.clone(), content))
            })
            .collect::<serde_json::Map<_, _>>()
    };
    state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .update_output_content(&Value::Object(canonical_outputs))?;
    Ok(())
}

fn schedule_output_consumer_refresh(app: &Host) {
    let state = app.state::<AppState>();
    state.output_render_requested.fetch_add(1, Ordering::AcqRel);
    if state
        .output_render_worker_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    let app = app.clone();
    std::thread::spawn(move || loop {
        let target = app
            .state::<AppState>()
            .output_render_requested
            .load(Ordering::Acquire);
        if let Err(error) = refresh_output_consumers_latest(&app.state::<AppState>(), target) {
            eprintln!("Failed to refresh native output consumers: {error}");
        }
        if app
            .state::<AppState>()
            .output_render_requested
            .load(Ordering::Acquire)
            == target
        {
            schedule_output_prerender(&app);
        }

        let state = app.state::<AppState>();
        if state.output_render_requested.load(Ordering::Acquire) != target {
            continue;
        }
        state
            .output_render_worker_active
            .store(false, Ordering::Release);
        if state.output_render_requested.load(Ordering::Acquire) == target
            || state
                .output_render_worker_active
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            break;
        }
    });
}

fn refresh_output_consumers_latest(state: &AppState, target: u64) -> Result<(), String> {
    let is_current = || state.output_render_requested.load(Ordering::Acquire) == target;
    if !is_current() {
        return Ok(());
    }
    let mut windows = state
        .display_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .values()
        .filter(|output| output.active)
        .cloned()
        .collect::<Vec<_>>();
    windows.extend(
        state
            .windowed_outputs
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .values()
            .cloned(),
    );
    let mut paired = HashSet::new();
    for output in &windows {
        if !is_current() {
            break;
        }
        if native_key_fill::claim(state, output, &mut paired)? {
            present_native_output_for_generation(state, output, Some(target))?;
        }
    }
    if !is_current() {
        return Ok(());
    }
    // The native displays are the latency-critical consumers. NDI is optional,
    // so skip scene copying entirely when it has no active outputs, and clone
    // only sources that NDI will consume when it is active.
    let ndi_source_ids = state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .active_output_ids();
    if ndi_source_ids.is_empty() {
        return Ok(());
    }
    let canonical_outputs = {
        let output_content = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?;
        ndi_source_ids
            .iter()
            .filter_map(|source_id| {
                output_content
                    .get(source_id)
                    .cloned()
                    .map(|content| (source_id.clone(), content))
            })
            .collect::<serde_json::Map<_, _>>()
    };
    if !is_current() {
        return Ok(());
    }
    state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .update_output_content(&Value::Object(canonical_outputs))?;
    Ok(())
}

fn schedule_output_prerender(app: &Host) {
    let state = app.state::<AppState>();
    if state
        .output_prerender_worker_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    let app = app.clone();
    std::thread::spawn(move || loop {
        let target = app
            .state::<AppState>()
            .output_render_requested
            .load(Ordering::Acquire);
        if let Err(error) = prerender_next_output_frames(&app.state::<AppState>(), target) {
            eprintln!("Failed to prerender next native output: {error}");
        }

        let state = app.state::<AppState>();
        if state.output_render_requested.load(Ordering::Acquire) != target {
            continue;
        }
        state
            .output_prerender_worker_active
            .store(false, Ordering::Release);
        if state.output_render_requested.load(Ordering::Acquire) == target
            || state
                .output_prerender_worker_active
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            break;
        }
    });
}

fn native_output_requires_live_composition(content: &Value) -> bool {
    native_compositor::content_requires_continuous_render(content)
        || native_transitions::scene_animates(content)
        || native_compositor::object_builds_require_live_composition(content)
}

fn prerender_next_output_frames(state: &AppState, target: u64) -> Result<(), String> {
    let is_current = || state.output_render_requested.load(Ordering::Acquire) == target;
    if !is_current() {
        return Ok(());
    }
    // Keep the established RSS -> output snapshot lock order used by the
    // update path so a completed feed refresh cannot deadlock prerendering.
    let rss_snapshot = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.".to_string())?
        .snapshot();
    let snapshot = lock_output_snapshot_state(&state.output_snapshot)?
        .cache
        .as_ref()
        .map(|cached| cached.snapshot.clone());
    let Some(snapshot) = snapshot else {
        return Ok(());
    };
    let Some(next_inputs) = output_scene::build_next_output_scene_state(&snapshot)? else {
        state
            .native_prerendered_frames
            .lock()
            .map_err(|_| "Native prerender cache is unavailable.".to_string())?
            .clear();
        return Ok(());
    };
    let (next_outputs, _) = rss_output::resolve_output_scenes(next_inputs, &rss_snapshot);
    let mut windows = state
        .display_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .values()
        .filter(|output| output.active)
        .cloned()
        .collect::<Vec<_>>();
    windows.extend(
        state
            .windowed_outputs
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .values()
            .cloned(),
    );
    let mut rendered = HashMap::new();
    for output in &windows {
        if !is_current() {
            return Ok(());
        }
        let Some(content) = next_outputs.get(&output.source_screen_id) else {
            continue;
        };
        // A prerendered frame would cut where the slide change must animate.
        if native_output_requires_live_composition(content) {
            continue;
        }
        let dimensions = state
            .native_output_surfaces
            .lock()
            .map_err(|_| "Native output surface state is unavailable.".to_string())?
            .get(&output.window_label)
            .map(native_output::NativeOutputSurface::dimensions);
        let Some((width, height)) = dimensions else {
            continue;
        };
        let key = NativeFrameCacheKey {
            source_screen_id: output.source_screen_id.clone(),
            width,
            height,
        };
        if rendered.contains_key(&key) {
            continue;
        }
        let layered_frame = state
            .native_prerender_compositor
            .lock()
            .map_err(|_| "Native prerender compositor is unavailable.".to_string())?
            .try_render_layered_for(
                &output.source_screen_id,
                content,
                width as usize,
                height as usize,
            )?;
        if layered_frame.dynamic_media || !is_current() {
            continue;
        }
        rendered.insert(
            key,
            CachedPrerenderedFrame {
                visual_key: output_playback::scroll_content_key_for_value(content),
                frame: None,
                layered_frame: Some(Arc::new(layered_frame)),
            },
        );
    }
    let preview_sizes = state
        .native_preview_request_sizes
        .lock()
        .map_err(|_| "Native preview request state is unavailable.".to_string())?
        .clone();
    for (source_screen_id, (width, height)) in preview_sizes {
        if !is_current() {
            return Ok(());
        }
        let key = NativeFrameCacheKey {
            source_screen_id: source_screen_id.clone(),
            width,
            height,
        };
        if rendered
            .get(&key)
            .is_some_and(|cached: &CachedPrerenderedFrame| cached.frame.is_some())
        {
            continue;
        }
        let Some(content) = next_outputs.get(&source_screen_id) else {
            continue;
        };
        // A prerendered frame would cut where the slide change must animate.
        if native_output_requires_live_composition(content) {
            continue;
        }
        let frame = state
            .native_prerender_compositor
            .lock()
            .map_err(|_| "Native prerender compositor is unavailable.".to_string())?
            .try_render_preview_for(&source_screen_id, content, width as usize, height as usize)?;
        if frame.dynamic_media || !is_current() {
            continue;
        }
        let visual_key = output_playback::scroll_content_key_for_value(content);
        if let Some(cached) = rendered.get_mut(&key) {
            cached.frame = Some(Arc::new(frame));
        } else {
            rendered.insert(
                key,
                CachedPrerenderedFrame {
                    visual_key,
                    frame: Some(Arc::new(frame)),
                    layered_frame: None,
                },
            );
        }
    }
    if is_current() {
        *state
            .native_prerendered_frames
            .lock()
            .map_err(|_| "Native prerender cache is unavailable.".to_string())? = rendered;
    }
    Ok(())
}

fn get_output_content(state: &AppState, source_screen_id: Value) -> Result<Value, String> {
    let source_screen_id = source_screen_id
        .as_str()
        .ok_or_else(|| "Output source screen id must be a string.".to_string())?;
    state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .get(source_screen_id)
        .cloned()
        .ok_or_else(|| format!("Output content for '{source_screen_id}' is unavailable."))
}

fn get_native_media_playback(
    state: &AppState,
    source_screen_id: Value,
    music: bool,
) -> Result<Value, String> {
    let content = get_output_content(state, source_screen_id.clone())?;
    let source = source_screen_id
        .as_str()
        .ok_or("Output source screen id must be a string.")?;
    let content = canonical_output_content(state, source, content)?;
    Ok(
        json!({"playback":content.get(if music { "musicPlaybackState" } else { "backgroundPlaybackState" }),"transport":content.get(if music { "audioTransport" } else { "backgroundTransport" })}),
    )
}

#[derive(Clone, Copy)]
pub enum NativeFrameTransport {
    Png,
    RawRgba8,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeProgramPreviewScene {
    timeline_started_at_ms: i64,
    object_build_step_started_at_ms_by_step: std::collections::BTreeMap<u64, i64>,
    background_css: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_media_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_media_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_scale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_playback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_end_behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_selection_id: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_desaturate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_brightness: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_contrast: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_hue: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_saturation: Option<f64>,
    presentation_theme_slide: Value,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    linked_text_context: Option<Value>,
    object_build_entry_key: String,
}

fn render_native_program_preview_bgra_frame_impl(
    state: &AppState,
    preview_id: &str,
    scene: NativeProgramPreviewScene,
    width: u32,
    height: u32,
) -> Result<native_compositor::NativeFrame, String> {
    validate_native_program_preview_request(preview_id, &scene, width, height)?;
    let encoded_scene = serde_json::to_vec(&scene).map_err(|error| error.to_string())?;
    if encoded_scene.len() > 8 * 1024 * 1024 {
        return Err("Native program preview scenes must not exceed 8 MiB.".into());
    }
    let content: Value =
        serde_json::from_slice(&encoded_scene).map_err(|error| error.to_string())?;
    let namespace = format!("program-preview:{preview_id}");
    let frame = state
        .native_preview_compositor
        .lock()
        .map_err(|_| "Native preview compositor is unavailable.".to_string())?
        .try_render_preview_for(&namespace, &content, width as usize, height as usize)?;
    Ok(frame)
}

fn render_native_program_preview_frame_impl(
    state: &AppState,
    preview_id: &str,
    scene: NativeProgramPreviewScene,
    width: u32,
    height: u32,
    transport: NativeFrameTransport,
) -> Result<Vec<u8>, String> {
    let frame =
        render_native_program_preview_bgra_frame_impl(state, preview_id, scene, width, height)?;
    let encoded = match transport {
        NativeFrameTransport::Png => frame.encode_png()?,
        NativeFrameTransport::RawRgba8 => frame.encode_raw_rgba()?,
    };
    Ok(encoded)
}

fn validate_native_program_preview_request(
    preview_id: &str,
    scene: &NativeProgramPreviewScene,
    width: u32,
    height: u32,
) -> Result<(), String> {
    const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
    if preview_id.trim().is_empty() || preview_id.len() > 256 {
        return Err("Native program preview requires a preview id of 1 to 256 bytes.".into());
    }
    if !native_compositor::frame_dimensions_allowed(width.into(), height.into()) {
        return Err(format!(
            "Native program preview dimensions must be non-zero, at most {} per edge, and at most {} pixels.",
            native_compositor::MAX_FRAME_EDGE,
            native_compositor::MAX_FRAME_PIXELS
        ));
    }
    if scene.object_build_entry_key.trim().is_empty() || scene.object_build_entry_key.len() > 512 {
        return Err(
            "Native program preview requires a build occurrence key of 1 to 512 bytes.".into(),
        );
    }
    if !scene.presentation_theme_slide.is_object() {
        return Err("Native program preview requires a composed presentation theme slide.".into());
    }
    if !(1..=MAX_SAFE_INTEGER).contains(&scene.timeline_started_at_ms) {
        return Err("Native program preview timeline start must be a positive safe epoch-millisecond value.".into());
    }
    let current_step = scene
        .presentation_theme_slide
        .get("buildStep")
        .and_then(Value::as_u64)
        .filter(|step| *step <= MAX_SAFE_INTEGER as u64)
        .ok_or("Native program preview slide buildStep must be a non-negative safe integer.")?;
    if scene.object_build_step_started_at_ms_by_step.get(&0) != Some(&scene.timeline_started_at_ms)
    {
        return Err(
            "Native program preview build step zero must match the timeline start timestamp."
                .into(),
        );
    }
    if !scene
        .object_build_step_started_at_ms_by_step
        .contains_key(&current_step)
    {
        return Err("Native program preview requires an authoritative timestamp for the current build step.".into());
    }
    for (&step, &started_at_ms) in &scene.object_build_step_started_at_ms_by_step {
        if step > current_step || step > MAX_SAFE_INTEGER as u64 {
            return Err("Native program preview build clock must not contain future or unsafe step numbers.".into());
        }
        if !(0..=MAX_SAFE_INTEGER).contains(&started_at_ms) {
            return Err(
                "Native program preview build timestamps must be non-negative safe integers."
                    .into(),
            );
        }
    }
    if scene
        .background_behavior
        .as_deref()
        .is_some_and(|value| !matches!(value, "background" | "foreground"))
    {
        return Err(
            "Native program preview background behavior must be background or foreground.".into(),
        );
    }
    Ok(())
}

fn validate_native_frame_request(
    source_screen_id: &str,
    width: u32,
    height: u32,
) -> Result<(), String> {
    if !native_compositor::frame_dimensions_allowed(width.into(), height.into()) {
        return Err(
            "Native output dimensions must be non-zero and contain at most 33554432 pixels."
                .to_string(),
        );
    }
    if source_screen_id.trim().is_empty() {
        return Err("Native output source screen id must be non-empty.".to_string());
    }
    Ok(())
}

fn render_native_output_bgra_frame_impl(
    state: &AppState,
    source_screen_id: &str,
    width: u32,
    height: u32,
) -> Result<Arc<native_compositor::NativeFrame>, String> {
    validate_native_frame_request(source_screen_id, width, height)?;
    let source_screen_id = source_screen_id.trim();
    let key = NativeFrameCacheKey {
        source_screen_id: source_screen_id.to_string(),
        width,
        height,
    };
    state
        .native_preview_request_sizes
        .lock()
        .map_err(|_| "Native preview request state is unavailable.".to_string())?
        .insert(source_screen_id.to_string(), (width, height));
    let content = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .get(source_screen_id)
        .cloned()
        .ok_or_else(|| format!("Output content for '{source_screen_id}' is unavailable."))?;
    let content = canonical_output_content(state, source_screen_id, content)?;
    let candidate = state
        .native_prerendered_frames
        .lock()
        .map_err(|_| "Native prerender cache is unavailable.".to_string())?
        .get(&key)
        .filter(|cached| cached.frame.is_some())
        .cloned();
    let prerendered = candidate
        .filter(|cached| {
            cached.visual_key == output_playback::scroll_content_key_for_value(&content)
        })
        .and_then(|cached| cached.frame);
    let frame = match prerendered {
        Some(frame) => frame,
        None => Arc::new(
            state
                .native_preview_compositor
                .lock()
                .map_err(|_| "Native preview compositor is unavailable.".to_string())?
                .try_render_preview_for(
                    source_screen_id,
                    &content,
                    width as usize,
                    height as usize,
                )?,
        ),
    };
    Ok(frame)
}

fn render_native_output_frame_impl(
    state: &AppState,
    source_screen_id: String,
    width: u32,
    height: u32,
    transport: NativeFrameTransport,
) -> Result<Vec<u8>, String> {
    validate_native_frame_request(&source_screen_id, width, height)?;
    let source_screen_id = source_screen_id.trim();
    let key = NativeFrameCacheKey {
        source_screen_id: source_screen_id.to_string(),
        width,
        height,
    };
    state
        .native_preview_request_sizes
        .lock()
        .map_err(|_| "Native preview request state is unavailable.".to_string())?
        .insert(source_screen_id.to_string(), (width, height));
    let revision = state
        .output_revisions
        .lock()
        .map_err(|_| "Output revision state is unavailable.".to_string())?
        .get(source_screen_id)
        .copied()
        .unwrap_or(0);
    match transport {
        NativeFrameTransport::Png => {
            if let Some(cached) = state
                .native_frame_cache
                .lock()
                .map_err(|_| "Native frame cache is unavailable.".to_string())?
                .get(&key)
                .filter(|cached| cached.revision == revision)
                .cloned()
            {
                return Ok(cached.png);
            }
        }
        NativeFrameTransport::RawRgba8 => {}
    }
    let frame = render_native_output_bgra_frame_impl(state, source_screen_id, width, height)?;
    let dynamic_media = frame.dynamic_media;
    let encoded = match transport {
        NativeFrameTransport::Png => frame.encode_png()?,
        NativeFrameTransport::RawRgba8 => frame.encode_raw_rgba()?,
    };
    if !dynamic_media {
        let current_revision = state
            .output_revisions
            .lock()
            .map_err(|_| "Output revision state is unavailable.".to_string())?
            .get(source_screen_id)
            .copied()
            .unwrap_or(0);
        if current_revision == revision {
            match transport {
                NativeFrameTransport::Png => {
                    state
                        .native_frame_cache
                        .lock()
                        .map_err(|_| "Native frame cache is unavailable.".to_string())?
                        .insert(
                            key,
                            CachedNativeFrame {
                                revision,
                                png: encoded.clone(),
                            },
                        );
                }
                // Raw 1080p frames already occupy about 8 MiB each. They are
                // requested only on content/size changes (or for dynamic
                // media), so retaining copies here would trade little work for
                // unbounded native memory across screens and resize variants.
                NativeFrameTransport::RawRgba8 => {}
            }
        }
    }
    Ok(encoded)
}

fn apply_screen_config(
    app: &Host,
    state: &AppState,
    value: Value,
    expected_revision: Option<u64>,
) -> Result<Value, String> {
    let _settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    let value = normalize_runtime_screen_config(&value)?;
    let saved = save_settings_locked(
        app,
        state,
        json!({ "screenConfig": value }),
        expected_revision,
    )?;
    let value = saved
        .get("screenConfig")
        .cloned()
        .ok_or_else(|| "Saved screen configuration is unavailable.".to_string())?;
    apply_runtime_screen_config(app, state, &value)?;
    Ok(json!({
        "ok": true,
        "settingsRevision": saved.get("settingsRevision").and_then(Value::as_u64)
    }))
}

fn apply_runtime_screen_config(app: &Host, state: &AppState, value: &Value) -> Result<(), String> {
    let physical_config = output_layout::expand(value)?;
    state
        .native_key_fill_pairs
        .lock()
        .map_err(|_| "Native key/fill state is unavailable.")?
        .reconcile_config(&physical_config)?;
    let mut snapshot_state = lock_output_snapshot_state(&state.output_snapshot)?;
    let current_content = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .clone();
    let physical = state.output_role == OutputRole::Full;
    if physical {
        let mut ndi = state
            .ndi
            .lock()
            .map_err(|_| "NDI state is unavailable.".to_string())?;
        ndi.apply_screen_config(&physical_config)?;
        let ndi_source_ids = ndi.active_output_ids();
        drop(ndi);
        {
            let mut native_audio = state
                .native_audio
                .lock()
                .map_err(|_| "Native audio state is unavailable.".to_string())?;
            for source_id in ndi_source_ids {
                native_audio.reset_ndi_audio(&source_id);
            }
        }
        let active_ids = active_native_audio_source_ids(state)?;
        let mut canonical_content = current_content
            .into_iter()
            .collect::<serde_json::Map<_, _>>();
        synchronize_native_audio_outputs(
            state,
            &mut canonical_content,
            &active_ids,
            std::time::Instant::now(),
        )?;
        state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .extend(canonical_content);
    }
    // Routing changes must retain the live selection for protocol rebasing.
    let pending_rss_urls = if let Some(cached) = snapshot_state.cache.as_ref() {
        let snapshot = replace_output_snapshot_screen_config(&cached.snapshot, value.clone())?;
        let rss_snapshot = state
            .rss_output
            .lock()
            .map_err(|_| "Output RSS state is unavailable.".to_string())?
            .snapshot();
        commit_native_output_snapshot(
            state,
            &mut snapshot_state,
            snapshot,
            &rss_snapshot,
            None,
            None,
        )?
        .1
    } else {
        let explicit_content = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .clone();
        state
            .ndi
            .lock()
            .map_err(|_| "NDI state is unavailable.".to_string())?
            .update_output_content(&Value::Object(explicit_content.into_iter().collect()))?;
        Vec::new()
    };
    // Populate the new screen's scene before its first frame is shown.
    if physical {
        synchronize_display_outputs_locked(app, state, &physical_config)?;
    }
    drop(snapshot_state);
    schedule_output_rss_refreshes(app, pending_rss_urls);
    Ok(())
}

fn normalize_runtime_screen_config(value: &Value) -> Result<Value, String> {
    if !value.is_object() {
        return Err("Screen configuration must be an object.".to_string());
    }
    Ok(settings::normalize_screen_config_value(value))
}

fn get_ndi_status(state: &AppState) -> Result<Value, String> {
    Ok(state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .status())
}

fn start_ndi_output(state: &AppState, screen_id: Value, config: Value) -> Result<Value, String> {
    let screen_id = screen_id
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "NDI screen id must be a non-empty string.".to_string())?;
    let _snapshot_guard = lock_output_snapshot_state(&state.output_snapshot)?;
    let existing_content = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .get(screen_id)
        .cloned();
    let mut ndi = state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?;
    ndi.start_or_update(screen_id, &config)?;
    if let Some(content) = existing_content {
        let mut current = serde_json::Map::new();
        current.insert(screen_id.to_string(), content);
        ndi.update_output_content(&Value::Object(current))?;
    }
    let status = ndi.status();
    drop(ndi);
    state
        .native_audio
        .lock()
        .map_err(|_| "Native audio state is unavailable.".to_string())?
        .reset_ndi_audio(screen_id);
    synchronize_native_audio_source_locked(state, screen_id)?;
    Ok(status)
}

fn update_ndi_output(state: &AppState, screen_id: Value, content: Value) -> Result<Value, String> {
    let screen_id = screen_id
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "NDI screen id must be a non-empty string.".to_string())?;
    let content = output_scene::build_output_scene(screen_id, &content)?;
    let mut snapshot_state = lock_output_snapshot_state(&state.output_snapshot)?;
    let mut outputs = serde_json::Map::new();
    outputs.insert(screen_id.to_string(), content);
    let active = std::iter::once(screen_id.to_string()).collect();
    synchronize_native_audio_outputs(state, &mut outputs, &active, std::time::Instant::now())?;
    state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .update_output_content(&Value::Object(outputs.clone()))?;
    state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .extend(outputs);
    snapshot_state.invalidate_cache();
    Ok(json!({ "ok": true }))
}

fn stop_ndi_output(state: &AppState, screen_id: Value) -> Result<Value, String> {
    let screen_id = screen_id
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "NDI screen id must be a non-empty string.".to_string())?;
    state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .stop(screen_id);
    clear_native_audio_source(state, screen_id)?;
    Ok(json!({ "ok": true }))
}

fn stop_all_ndi_outputs(state: &AppState) -> Result<Value, String> {
    let ids = state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .active_output_ids();
    state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .stop_all();
    for id in ids {
        clear_native_audio_source(state, &id)?;
    }
    Ok(json!({ "ok": true }))
}

fn shutdown_all_outputs(app: &Host, state: &AppState) {
    let outputs = state
        .display_outputs
        .lock()
        .map(|mut outputs| std::mem::take(&mut *outputs))
        .unwrap_or_default()
        .into_values()
        .chain(
            state
                .windowed_outputs
                .lock()
                .map(|mut outputs| std::mem::take(&mut *outputs))
                .unwrap_or_default()
                .into_values(),
        )
        .collect::<Vec<_>>();

    for source_screen_id in outputs
        .iter()
        .map(|output| output.source_screen_id.as_str())
        .collect::<HashSet<_>>()
    {
        let _ = clear_native_audio_source(state, source_screen_id);
    }
    if let Ok(mut surfaces) = state.native_output_surfaces.lock() {
        surfaces.clear();
    }
    let _ = stop_all_ndi_outputs(state);

    for window_label in outputs
        .iter()
        .map(|output| output.window_label.as_str())
        .collect::<HashSet<_>>()
    {
        if let Some(window) = app.get_window(window_label) {
            let _ = window.close();
        }
    }
    for screen_id in outputs
        .iter()
        .filter(|output| output.target_id.is_some())
        .map(|output| output.source_screen_id.as_str())
        .collect::<HashSet<_>>()
    {
        if let Some(control) = app.get_window(&display_control_window_label(screen_id)) {
            let _ = control.close();
        }
    }
}

fn get_output_options(app: &Host, state: &AppState) -> Result<Value, String> {
    let display_names = display_names::connected_display_names()?;
    let primary_name = app
        .primary_monitor()
        .map_err(|error| error.to_string())?
        .and_then(|monitor| monitor.name().map(|name| name.to_string()));
    let displays = app
        .available_monitors()
        .map_err(|error| error.to_string())?
        .into_iter()
        .enumerate()
        .map(|(index, monitor)| {
            let name = monitor.name().map(|name| name.to_string());
            let id = name.clone().unwrap_or_else(|| format!("display-{index}"));
            let position = monitor.position();
            let size = monitor.size();
            json!({
                "id": id,
                "label": name
                    .as_deref()
                    .map(|name| display_names.iter()
                        .find(|(device, _)| monitor_identity_matches(name, device))
                        .map(|(_, label)| label.clone())
                        .unwrap_or_else(|| display_label(name)))
                    .unwrap_or_else(|| format!("Display {}", index + 1)),
                "primary": primary_name.as_deref() == Some(id.as_str()),
                "bounds": { "x": position.x, "y": position.y, "width": size.width, "height": size.height },
                "size": { "width": size.width, "height": size.height }
            })
        })
        .collect::<Vec<_>>();
    let mut displays = displays;
    {
        let outputs = state
            .display_outputs
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?;
        let surfaces = state
            .native_output_surfaces
            .lock()
            .map_err(|_| "Native output surface state is unavailable.".to_string())?;
        for display in &mut displays {
            let status = outputs
                .values()
                .filter(|output| output.active)
                .filter(|output| output.target_id.as_deref() == display["id"].as_str())
                .find_map(|output| surfaces.get(&output.window_label)?.hdr_status());
            if let Some(status) = status {
                display["hdrStatus"] = json!(status.summary());
            }
        }
    }
    let ndi_status = state
        .ndi
        .lock()
        .map_err(|error| format!("NDI state is unavailable: {error}"))?
        .status();
    let mut result = ndi_status
        .as_object()
        .cloned()
        .ok_or_else(|| "NDI status must be an object.".to_string())?;
    result.insert("displays".to_string(), json!(displays));
    Ok(Value::Object(result))
}

fn create_windowed_output(app: &Host, state: &AppState, args: &[Value]) -> Result<Value, String> {
    let source_screen_id = argument(args, 0)?.as_str().unwrap_or_default().to_string();
    if let Some(content) = args.get(1) {
        let content = output_scene::build_output_scene(&source_screen_id, content)?;
        let mut snapshot_state = lock_output_snapshot_state(&state.output_snapshot)?;
        state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .insert(source_screen_id.clone(), content);
        snapshot_state.invalidate_cache();
    }
    let id = format!("windowed-output-{}", uuid::Uuid::new_v4());
    let window_label = format!("output-{}", uuid::Uuid::new_v4());
    let label = if source_screen_id.is_empty() {
        "Window Output".to_string()
    } else {
        format!("{source_screen_id} Window Output")
    };
    let (output_window, surface) = create_windowed_native_output_window(
        app,
        state,
        state.main_thread_id,
        &window_label,
        &label,
    )?;
    state
        .native_output_surfaces
        .lock()
        .map_err(|_| "Native output surface state is unavailable.".to_string())?
        .insert(window_label.clone(), surface);
    let output = WindowedOutput {
        label: label.clone(),
        source_screen_id: source_screen_id.clone(),
        window_label: window_label.clone(),
        target_id: None,
        bounds: None,
        active: true,
    };
    state
        .windowed_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .insert(id.clone(), output);

    let lifecycle_app = app.clone();
    let lifecycle_id = id.clone();
    let lifecycle_source_screen_id = source_screen_id.clone();
    let lifecycle_window = output_window.clone();
    let lifecycle_window_label = window_label.clone();
    output_window.on_window_event(Box::new(move |event| {
        if let WindowEvent::Resized(size)
        | WindowEvent::ScaleFactorChanged {
            new_inner_size: size,
            ..
        } = event
        {
            let state = lifecycle_app.state::<AppState>();
            if let Err(error) = resize_native_output_surface(&state, &lifecycle_window_label, *size)
            {
                record_diagnostic(
                    &lifecycle_app,
                    "native-output-resize-failed",
                    diagnostics::LogLevel::Warn,
                    "gpu",
                    Some(&error),
                );
            }
            let output = state
                .windowed_outputs
                .lock()
                .ok()
                .and_then(|outputs| outputs.get(&lifecycle_id).cloned());
            if let Some(output) = output {
                let _ = present_native_output(&state, &output);
            }
            return;
        }
        if let WindowEvent::CloseRequested { api, .. } = event {
            if remove_native_output_surface(
                &lifecycle_app.state::<AppState>(),
                &lifecycle_window_label,
            ) {
                api.prevent_close();
                let _ = lifecycle_window.close();
            }
            return;
        }
        if !matches!(event, WindowEvent::Destroyed) {
            return;
        }
        let state = lifecycle_app.state::<AppState>();
        let removed = state
            .windowed_outputs
            .lock()
            .ok()
            .and_then(|mut outputs| outputs.remove(&lifecycle_id));
        let Some(_) = removed else {
            return;
        };
        let _ = clear_native_audio_source(&state, &lifecycle_source_screen_id);
        if let Ok(mut surfaces) = state.native_output_surfaces.lock() {
            surfaces.remove(&lifecycle_window_label);
        }
        if let Ok(outputs) = state.windowed_outputs.lock() {
            let payload = outputs
                .iter()
                .map(|(output_id, output)| {
                    json!({
                        "id": output_id,
                        "label": output.label,
                        "sourceScreenId": output.source_screen_id,
                    })
                })
                .collect::<Vec<_>>();
            let _ = lifecycle_app.emit_to("main", "windowed-outputs-changed", payload);
        };
    }));
    let output = state
        .windowed_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .get(&id)
        .cloned();
    if let Some(output) = output {
        synchronize_native_audio_source(state, &output.source_screen_id)?;
        if let Err(error) = present_native_output(state, &output) {
            remove_native_output_surface(state, &window_label);
            let _ = output_window.close();
            return Err(error);
        }
    }
    output_window.show().map_err(|error| error.to_string())?;
    notify_windowed_outputs(app, state)?;
    Ok(json!({ "id": id, "label": label, "sourceScreenId": source_screen_id }))
}

fn list_windowed_outputs(state: &AppState) -> Result<Value, String> {
    let outputs = state
        .windowed_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?;
    Ok(Value::Array(outputs.iter().map(|(id, output)| json!({ "id": id, "label": output.label, "sourceScreenId": output.source_screen_id })).collect()))
}

fn focus_windowed_output(app: &Host, state: &AppState, id: Value) -> Result<Value, String> {
    let id = id
        .as_str()
        .ok_or_else(|| "Output id must be a string.".to_string())?;
    let output = state
        .windowed_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .get(id)
        .cloned();
    let Some(output) = output else {
        return Ok(json!({ "ok": false }));
    };
    let Some(window) = app.get_window(&output.window_label) else {
        return Ok(json!({ "ok": false }));
    };
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    Ok(json!({ "ok": true }))
}

fn notify_windowed_outputs(app: &Host, state: &AppState) -> Result<(), String> {
    let outputs = list_windowed_outputs(state)?;
    app.emit_to("main", "windowed-outputs-changed", outputs)
        .map_err(|error| error.to_string())
}

fn output_source_ids(state: &AppState) -> Result<Vec<String>, String> {
    let mut targets = state
        .windowed_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .values()
        .map(|output| output.source_screen_id.clone())
        .collect::<Vec<_>>();
    targets.extend(
        state
            .display_outputs
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .values()
            .filter(|output| output.active)
            .map(|output| output.source_screen_id.clone()),
    );
    Ok(targets)
}

fn active_native_audio_source_ids(
    state: &AppState,
) -> Result<std::collections::HashSet<String>, String> {
    active_native_audio_source_ids_with(state, &[])
}

/// `idle_candidates` are main sources whose content is not stored yet.
fn active_native_audio_source_ids_with(
    state: &AppState,
    idle_candidates: &[String],
) -> Result<std::collections::HashSet<String>, String> {
    let mut ids = output_source_ids(state)?
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let ndi_ids = state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?
        .active_output_ids();
    ids.extend(ndi_ids);
    if state.output_role == OutputRole::Full {
        ids.extend(
            state
                .native_preview_request_sizes
                .lock()
                .map_err(|_| "Native preview request state is unavailable.".to_string())?
                .keys()
                .cloned(),
        );
    }
    if ids.is_empty() && state.idle_program_audio.load(Ordering::Relaxed) {
        // The controller's preview is no output here, but its program content is on air. Every screen holds the
        // same program, so one source plays it: one per screen is a chorus of out-of-phase copies.
        let stored = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .iter()
            .filter(|(_, content)| is_main_transport_content(content))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        ids.extend(stored.iter().chain(idle_candidates).min().cloned());
    }
    Ok(ids)
}

fn synchronize_native_audio_outputs(
    state: &AppState,
    outputs: &mut serde_json::Map<String, Value>,
    active_source_ids: &std::collections::HashSet<String>,
    now: std::time::Instant,
) -> Result<(), String> {
    let frozen = state
        .output_freeze
        .read()
        .map_err(|_| "Output freeze state is unavailable.")?;
    if *frozen {
        return Ok(());
    }
    let mut native_audio = state
        .native_audio
        .lock()
        .map_err(|_| "Native audio state is unavailable.".to_string())?;
    let mut durations = Vec::new();
    for (source_id, content) in outputs.iter_mut() {
        if !active_source_ids.contains(source_id) {
            native_audio.clear_source(source_id);
            let (media, music) = native_audio::idle_layer_durations(content);
            durations.push((source_id.clone(), media, music));
            continue;
        }
        let status = native_audio.sync_content(source_id, content, now);
        durations.push((
            source_id.clone(),
            status.media_duration,
            status.music_duration,
        ));
        let mut canonical = content.as_object().cloned().unwrap_or_default();
        canonical.insert("nativeAudioLayers".to_string(), status.to_value());
        canonical.insert(
            "nativeAudioActive".to_string(),
            json!(status.media_active && status.music_active),
        );
        *content = Value::Object(canonical);
    }
    let tap_dropped_blocks = native_audio.ndi_tap_dropped_blocks();
    drop(native_audio);
    for content in outputs.values_mut() {
        if let Some(object) = content.as_object_mut() {
            object.insert(
                "nativeAudioTapDroppedBlocks".to_string(),
                json!(tap_dropped_blocks),
            );
        }
    }
    if !durations.is_empty() {
        let mut playback = state
            .output_playback
            .lock()
            .map_err(|_| "Output playback state is unavailable.".to_string())?;
        for (source_id, media_duration, music_duration) in durations {
            if let Some(duration) = media_duration {
                playback.set_duration(
                    &source_id,
                    output_playback::PlaybackLayer::Media,
                    duration,
                    now,
                );
            }
            if let Some(duration) = music_duration {
                playback.set_duration(
                    &source_id,
                    output_playback::PlaybackLayer::Music,
                    duration,
                    now,
                );
            }
            if let Some(content) = outputs.get_mut(&source_id) {
                *content = playback.canonical_content(&source_id, content, now);
            }
        }
    }
    Ok(())
}

fn synchronize_native_audio_source(state: &AppState, source_id: &str) -> Result<(), String> {
    let _snapshot_guard = lock_output_snapshot_state(&state.output_snapshot)?;
    synchronize_native_audio_source_locked(state, source_id)
}

fn synchronize_native_audio_source_locked(state: &AppState, source_id: &str) -> Result<(), String> {
    let content = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .get(source_id)
        .cloned();
    let Some(content) = content else {
        return Ok(());
    };
    let now = std::time::Instant::now();
    let content = state
        .output_playback
        .lock()
        .map_err(|_| "Output playback state is unavailable.".to_string())?
        .canonical_content(source_id, &content, now);
    state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .insert(source_id.to_string(), content.clone());
    let mut outputs = serde_json::Map::new();
    outputs.insert(source_id.to_string(), content);
    let active = std::iter::once(source_id.to_string()).collect();
    synchronize_native_audio_outputs(state, &mut outputs, &active, now)?;
    if let Some(content) = outputs.remove(source_id) {
        state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .insert(source_id.to_string(), content);
    }
    Ok(())
}

fn clear_native_audio_source(state: &AppState, source_id: &str) -> Result<(), String> {
    if active_native_audio_source_ids(state)?.contains(source_id) {
        return Ok(());
    }
    state
        .native_audio
        .lock()
        .map_err(|_| "Native audio state is unavailable.".to_string())?
        .clear_source(source_id);
    Ok(())
}

fn screen_output_format(
    screens: &Value,
    screen_id: &str,
) -> Result<native_hdr::OutputFormat, String> {
    screens
        .get(screen_id)
        .and_then(|screen| screen.get("outputFormat"))
        .map_or(Ok(native_hdr::OutputFormat::Sdr), |format| {
            format
                .as_str()
                .ok_or_else(|| "Screen output format must be sdr, hlg or pq.".to_string())
                .and_then(native_hdr::OutputFormat::parse)
        })
}

fn display_screen_configs(value: &Value) -> Result<Vec<(String, String, String)>, String> {
    let configs = value
        .as_object()
        .ok_or_else(|| "Screen configuration must be an object.".to_string())?;
    Ok(configs
        .iter()
        .filter_map(|(screen_id, config)| {
            let config = config.as_object()?;
            let enabled = config
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let target_type = config
                .get("targetType")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !enabled || target_type != "display" {
                return None;
            }
            let label = display_label(
                config
                    .get("label")
                    .and_then(Value::as_str)
                    .filter(|label| !label.trim().is_empty())
                    .unwrap_or(screen_id),
            );
            let target_id = config
                .get("targetId")
                .and_then(target_id_string)
                .unwrap_or_default();
            Some((screen_id.clone(), label, target_id))
        })
        .collect())
}

fn set_display_output_visibility(
    app: &Host,
    state: &AppState,
    screen_ids: Value,
    visible: Value,
    screen_config: Option<&Value>,
) -> Result<Value, String> {
    let screen_ids = screen_ids
        .as_array()
        .ok_or_else(|| "Display output screen ids must be an array.".to_string())?;
    let visible = visible
        .as_bool()
        .ok_or_else(|| "Display output visibility must be a boolean.".to_string())?;
    if let Some(screen_config) = screen_config.filter(|value| !value.is_null()) {
        apply_runtime_screen_config(app, state, screen_config)?;
        return Ok(json!({ "ok": true }));
    }
    let mut outputs = state
        .display_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?;
    let labels = screen_ids
        .iter()
        .map(|screen_id| {
            let screen_id = screen_id
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "Display output screen id must be non-empty.".to_string())?;
            Ok(outputs.get_mut(screen_id).map(|output| {
                output.active = visible;
                output.clone()
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    drop(outputs);
    for output in labels.into_iter().flatten() {
        if let Some(window) = app.get_window(&output.window_label) {
            let control = app.get_window(&display_control_window_label(&output.source_screen_id));
            if visible {
                present_native_output(state, &output)?;
                window.show().map_err(|error| error.to_string())?;
                keep_display_window_on_top(&window)?;
                if let Some(control) = control {
                    if control.is_visible().map_err(|error| error.to_string())? {
                        keep_display_control_on_top(&control)?;
                    }
                }
                present_native_output(state, &output)?;
            } else {
                window.hide().map_err(|error| error.to_string())?;
                if let Some(control) = control {
                    control.hide().map_err(|error| error.to_string())?;
                }
            }
        }
    }
    Ok(json!({ "ok": true }))
}

fn close_display_output(app: &Host, state: &AppState, screen_id: Value) -> Result<Value, String> {
    let screen_id = screen_id
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| "Display output screen id must be non-empty.".to_string())?;
    let output_exists = state
        .display_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .contains_key(screen_id);
    if !output_exists {
        return Err(format!("Display output '{screen_id}' is unavailable."));
    }
    let result = set_display_output_visibility(app, state, json!([screen_id]), json!(false), None)?;
    clear_native_audio_source(state, screen_id)?;
    app.emit_to("main", "output-closed", screen_id.to_string())
        .map_err(|error| error.to_string())?;
    Ok(result)
}

fn hide_disabled_display_outputs(
    app: &Host,
    state: &AppState,
    next_config: &Value,
) -> Result<HiddenDisplayOutputs, String> {
    let physical_config = output_layout::expand(next_config)?;
    let desired_ids = display_screen_configs(&physical_config)?
        .into_iter()
        .map(|(screen_id, _, _)| screen_id)
        .collect::<std::collections::HashSet<_>>();
    let stale_labels = state
        .display_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .iter()
        .filter(|(screen_id, output)| output.active && !desired_ids.contains(*screen_id))
        .map(|(screen_id, output)| (screen_id.clone(), output.window_label.clone()))
        .collect::<Vec<_>>();
    let mut hidden = HiddenDisplayOutputs::default();
    for (screen_id, label) in stale_labels {
        if let Some(window) = app.get_window(&label) {
            window.hide().map_err(|error| error.to_string())?;
            hidden.windows.push(window);
        }
        if let Some(control) = app.get_window(&display_control_window_label(&screen_id)) {
            control.hide().map_err(|error| error.to_string())?;
        }
    }
    Ok(hidden)
}

fn target_id_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn monitor_identity(value: &str) -> String {
    display_label(value).trim().to_ascii_lowercase()
}

fn monitor_identity_matches(target_id: &str, monitor_name: &str) -> bool {
    let target_id = monitor_identity(target_id);
    !target_id.is_empty() && target_id == monitor_identity(monitor_name)
}

fn monitor_index_from_target_id(target_id: &str) -> Option<usize> {
    target_id
        .strip_prefix("display-")
        .and_then(|index| index.parse::<usize>().ok())
}

fn monitor_is_primary(monitor: &Monitor, primary: Option<&Monitor>) -> bool {
    let Some(primary) = primary else {
        return false;
    };
    if let (Some(monitor_name), Some(primary_name)) = (monitor.name(), primary.name()) {
        return monitor_identity_matches(monitor_name, primary_name);
    }
    DisplayBounds::from_monitor(monitor) == DisplayBounds::from_monitor(primary)
}

fn find_target_monitor<'a>(
    monitors: &'a [Monitor],
    primary: Option<&Monitor>,
    target_id: &str,
) -> Option<&'a Monitor> {
    monitors
        .iter()
        .find(|monitor| {
            monitor
                .name()
                .map(|name| monitor_identity_matches(target_id, name))
                .unwrap_or(false)
        })
        // Older Tauri builds exposed unnamed monitors as display-N. Keep that
        // index form valid for workspaces saved by those builds.
        .or_else(|| monitor_index_from_target_id(target_id).and_then(|index| monitors.get(index)))
        // Previously persisted numeric IDs cannot be translated to a Tauri
        // monitor name. Route those legacy IDs to the primary display instead
        // of opening a 1920x1080 window at (0, 0).
        .or_else(|| {
            monitors
                .iter()
                .find(|monitor| monitor_is_primary(monitor, primary))
        })
        .or_else(|| monitors.first())
}

fn display_window_label(screen_id: &str) -> String {
    format!("display-output-{}", output_window_suffix(screen_id))
}

fn display_control_window_label(screen_id: &str) -> String {
    format!("display-output-control-{}", output_window_suffix(screen_id))
}

fn output_window_suffix(screen_id: &str) -> String {
    screen_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn update_display_window(
    window: &Window,
    label: &str,
    bounds: DisplayBounds,
    update_geometry: bool,
) -> Result<(), String> {
    window.set_title(label).map_err(|error| error.to_string())?;
    if update_geometry {
        window
            .set_position(PhysicalPosition::new(bounds.x, bounds.y))
            .map_err(|error| error.to_string())?;
        window
            .set_size(PhysicalSize::new(bounds.width, bounds.height))
            .map_err(|error| error.to_string())?;
    }
    keep_display_window_on_top(window)
}

#[cfg(windows)]
fn keep_display_window_on_top(window: &Window) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOREDRAW,
        SWP_NOSIZE,
    };

    let hwnd = windows::Win32::Foundation::HWND(window.native_window_pointer()?);
    // Z-order changes must not erase the retained static GPU frame.
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOREDRAW | SWP_ASYNCWINDOWPOS,
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn keep_display_window_on_top(window: &Window) -> Result<(), String> {
    window
        .set_always_on_top(true)
        .map_err(|error| error.to_string())
}

fn display_control_position(
    window: &Window,
    bounds: DisplayBounds,
) -> Result<PhysicalPosition<i32>, String> {
    let size = window.outer_size().map_err(|error| error.to_string())?;
    Ok(PhysicalPosition::new(
        bounds.x + 18,
        bounds.y + bounds.height as i32 - size.height as i32 - 18,
    ))
}

fn update_display_control_window(window: &Window, bounds: DisplayBounds) -> Result<(), String> {
    window
        .set_always_on_top(true)
        .map_err(|error| error.to_string())?;
    window
        .set_position(display_control_position(window, bounds)?)
        .map_err(|error| error.to_string())
}

fn cursor_on_display(bounds: DisplayBounds, cursor: PhysicalPosition<f64>) -> bool {
    cursor.x >= f64::from(bounds.x)
        && cursor.y >= f64::from(bounds.y)
        && cursor.x < f64::from(bounds.x) + f64::from(bounds.width)
        && cursor.y < f64::from(bounds.y) + f64::from(bounds.height)
}

fn tick_display_output_controls(
    app: &Host,
    state: &AppState,
    hovered: &mut HashSet<String>,
) -> Result<(), String> {
    let cursor = app.cursor_position().map_err(|error| error.to_string())?;
    let outputs = state
        .display_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .iter()
        .map(|(screen_id, output)| (screen_id.clone(), output.clone()))
        .collect::<Vec<_>>();
    for (_, output) in outputs.iter().filter(|(_, output)| output.active) {
        if let Some(window) = app.get_window(&output.window_label) {
            keep_display_window_on_top(&window)?;
        }
    }
    let next_hovered = outputs
        .iter()
        .filter(|(_, output)| {
            output.active
                && output
                    .bounds
                    .is_some_and(|bounds| cursor_on_display(bounds, cursor))
        })
        .map(|(screen_id, _)| screen_id.clone())
        .collect::<HashSet<_>>();
    for screen_id in hovered.difference(&next_hovered) {
        if let Some(control) = app.get_window(&display_control_window_label(screen_id)) {
            control.hide().map_err(|error| error.to_string())?;
        }
    }
    for screen_id in &next_hovered {
        let control = match app.get_window(&display_control_window_label(screen_id)) {
            Some(control) => control,
            None => {
                let bounds = outputs
                    .iter()
                    .find(|(id, _)| id == screen_id)
                    .and_then(|(_, output)| output.bounds)
                    .ok_or_else(|| "Physical output bounds are unavailable.".to_string())?;
                // WebView2 startup must not delay the physical output's first frame.
                create_display_output_control_window(app, screen_id, bounds).inspect_err(
                    |error| {
                        record_diagnostic(
                            app,
                            "display-control-create-failed",
                            diagnostics::LogLevel::Warn,
                            "output",
                            Some(error),
                        );
                    },
                )?
            }
        };
        if !hovered.contains(screen_id)
            || !control.is_visible().map_err(|error| error.to_string())?
        {
            control.show().map_err(|error| error.to_string())?;
        }
        keep_display_control_on_top(&control)?;
    }
    *hovered = next_hovered;
    Ok(())
}

#[cfg(windows)]
fn keep_display_control_on_top(window: &Window) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    let hwnd = windows::Win32::Foundation::HWND(window.native_window_pointer()?);
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
        .map_err(|error| error.to_string())
    }
}

#[cfg(not(windows))]
fn keep_display_control_on_top(window: &Window) -> Result<(), String> {
    window
        .set_always_on_top(true)
        .map_err(|error| error.to_string())
}

fn synchronize_display_outputs_locked(
    app: &Host,
    state: &AppState,
    value: &Value,
) -> Result<(), String> {
    let mut setup = native_key_fill::DisplaySetup::begin(app, value)?;
    synchronize_display_outputs_inner(app, state, value, &setup.members)?;
    setup.finish(state)
}

fn synchronize_display_outputs_inner(
    app: &Host,
    state: &AppState,
    value: &Value,
    paired: &HashSet<String>,
) -> Result<(), String> {
    let desired = display_screen_configs(value)?;
    let desired_ids = desired
        .iter()
        .map(|(screen_id, _, _)| screen_id.clone())
        .collect::<std::collections::HashSet<_>>();
    let stale = state
        .display_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .iter()
        .filter(|(screen_id, _)| !desired_ids.contains(*screen_id))
        .map(|(screen_id, output)| (screen_id.clone(), output.window_label.clone()))
        .collect::<Vec<_>>();
    for (screen_id, window_label) in stale {
        let parked = value.get(&screen_id).is_some_and(|config| {
            config.get("targetType").and_then(Value::as_str) == Some("display")
                && config.get("enabled").and_then(Value::as_bool) == Some(false)
        });
        if parked {
            if let Some(window) = app.get_window(&window_label) {
                window.hide().map_err(|error| error.to_string())?;
                if let Some(control) = app.get_window(&display_control_window_label(&screen_id)) {
                    control.hide().map_err(|error| error.to_string())?;
                }
                if let Some(output) = state
                    .display_outputs
                    .lock()
                    .map_err(|_| "Output state is unavailable.".to_string())?
                    .get_mut(&screen_id)
                {
                    output.active = false;
                }
                clear_native_audio_source(state, &screen_id)?;
                continue;
            }
        }
        remove_native_output_surface(state, &window_label);
        if let Some(control) = app.get_window(&display_control_window_label(&screen_id)) {
            let _ = control.close();
        }
        if let Some(window) = app.get_window(&window_label) {
            let _ = window.close();
        }
        state
            .display_outputs
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .remove(&screen_id);
        clear_native_audio_source(state, &screen_id)?;
    }

    let monitors = app
        .available_monitors()
        .map_err(|error| error.to_string())?;
    let primary_monitor = app.primary_monitor().map_err(|error| error.to_string())?;
    for (screen_id, label, target_id) in desired {
        let monitor = if paired.contains(&screen_id) {
            monitors
                .iter()
                .find(|monitor| {
                    monitor
                        .name()
                        .is_some_and(|name| monitor_identity_matches(&target_id, name))
                })
                .or_else(|| {
                    monitor_index_from_target_id(&target_id).and_then(|index| monitors.get(index))
                })
        } else {
            find_target_monitor(&monitors, primary_monitor.as_ref(), &target_id)
        };
        if paired.contains(&screen_id) && monitor.is_none() {
            return Err(format!(
                "Key/fill display target '{target_id}' is unavailable."
            ));
        }
        let bounds = monitor
            .map(DisplayBounds::from_monitor)
            .unwrap_or(DisplayBounds {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            });
        let output_format = screen_output_format(value, &screen_id)?;
        let existing = state
            .display_outputs
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .get(&screen_id)
            .cloned();
        if let Some(window) = existing.as_ref().and_then(|existing| {
            let applied = state
                .native_output_surfaces
                .lock()
                .ok()?
                .get(&existing.window_label)
                .map(native_output::NativeOutputSurface::output_format)?;
            (applied != output_format)
                .then(|| app.get_window(&existing.window_label))
                .flatten()
        }) {
            // The swapchain format and color space are fixed at creation, so
            // the output window gets a new surface.
            remove_native_output_surface(state, &window.label());
            let size = window.inner_size().map_err(|error| error.to_string())?;
            let surface = create_shared_native_output_surface(
                app,
                state,
                window.clone(),
                size.width,
                size.height,
                output_format,
            )
            .inspect_err(|_| {
                let _ = window.close();
            })?;
            state
                .native_output_surfaces
                .lock()
                .map_err(|_| "Native output surface state is unavailable.".to_string())?
                .insert(window.label().to_string(), surface);
        }
        if let Some(existing) = existing {
            if let Some(window) = app.get_window(&existing.window_label) {
                let geometry_changed = existing.bounds != Some(bounds)
                    || existing.target_id.as_deref() != Some(target_id.as_str());
                update_display_window(&window, &label, bounds, geometry_changed)?;
                if let Some(control) = app.get_window(&display_control_window_label(&screen_id)) {
                    if let Err(error) = update_display_control_window(&control, bounds) {
                        record_diagnostic(
                            app,
                            "display-control-update-failed",
                            diagnostics::LogLevel::Warn,
                            "output",
                            Some(&error),
                        );
                    }
                }
                let mut outputs = state
                    .display_outputs
                    .lock()
                    .map_err(|_| "Output state is unavailable.".to_string())?;
                if let Some(output) = outputs.get_mut(&screen_id) {
                    output.label = label.clone();
                    output.target_id = Some(target_id.clone());
                    output.bounds = Some(bounds);
                    output.active = true;
                }
                drop(outputs);
                synchronize_native_audio_source_locked(state, &screen_id)?;
                if paired.contains(&screen_id) {
                    continue;
                }
                present_native_output(state, &existing)?;
                window.show().map_err(|error| error.to_string())?;
                keep_display_window_on_top(&window)?;
                if let Some(control) = app.get_window(&display_control_window_label(&screen_id)) {
                    if control.is_visible().map_err(|error| error.to_string())? {
                        keep_display_control_on_top(&control)?;
                    }
                }
                if !existing.active {
                    present_native_output(state, &existing)?;
                }
                continue;
            }
            state
                .display_outputs
                .lock()
                .map_err(|_| "Output state is unavailable.".to_string())?
                .remove(&screen_id);
        }
        let window_label = display_window_label(&screen_id);
        let (output_window, surface) = create_display_native_output_window(
            app,
            state,
            state.main_thread_id,
            &window_label,
            &label,
            bounds,
            output_format,
        )?;
        state
            .native_output_surfaces
            .lock()
            .map_err(|_| "Native output surface state is unavailable.".to_string())?
            .insert(window_label.clone(), surface);
        let output = WindowedOutput {
            label: label.clone(),
            source_screen_id: screen_id.clone(),
            window_label: window_label.clone(),
            target_id: Some(target_id.clone()),
            bounds: Some(bounds),
            active: true,
        };
        state
            .display_outputs
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .insert(screen_id.clone(), output);
        let lifecycle_app = app.clone();
        let lifecycle_screen_id = screen_id.clone();
        let lifecycle_window = output_window.clone();
        let lifecycle_window_label = window_label.clone();
        output_window.on_window_event(Box::new(move |event| {
            if let WindowEvent::Resized(size)
            | WindowEvent::ScaleFactorChanged {
                new_inner_size: size,
                ..
            } = event
            {
                let state = lifecycle_app.state::<AppState>();
                if let Err(error) =
                    resize_native_output_surface(&state, &lifecycle_window_label, *size)
                {
                    record_diagnostic(
                        &lifecycle_app,
                        "native-output-resize-failed",
                        diagnostics::LogLevel::Warn,
                        "gpu",
                        Some(&error),
                    );
                }
                let output = state
                    .display_outputs
                    .lock()
                    .ok()
                    .and_then(|outputs| outputs.get(&lifecycle_screen_id).cloned());
                if let Some(output) = output.filter(|output| output.active) {
                    let _ = present_native_output(&state, &output);
                }
            } else if let WindowEvent::CloseRequested { api, .. } = event {
                if remove_native_output_surface(
                    &lifecycle_app.state::<AppState>(),
                    &lifecycle_window_label,
                ) {
                    api.prevent_close();
                    let _ = lifecycle_window.close();
                }
            } else if matches!(event, WindowEvent::Destroyed) {
                let state = lifecycle_app.state::<AppState>();
                if let Some(control) =
                    lifecycle_app.get_window(&display_control_window_label(&lifecycle_screen_id))
                {
                    let _ = control.close();
                }
                let was_registered = state
                    .display_outputs
                    .lock()
                    .map(|mut outputs| outputs.remove(&lifecycle_screen_id).is_some())
                    .unwrap_or(false);
                let _ = clear_native_audio_source(&state, &lifecycle_screen_id);
                if let Ok(mut surfaces) = state.native_output_surfaces.lock() {
                    surfaces.remove(&lifecycle_window_label);
                }
                if was_registered {
                    let _ =
                        lifecycle_app.emit_to("main", "output-closed", lifecycle_screen_id.clone());
                }
            }
        }));
        synchronize_native_audio_source_locked(state, &screen_id)?;
        let output = state
            .display_outputs
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .get(&screen_id)
            .cloned();
        if let Some(output) = output {
            if paired.contains(&screen_id) {
                continue;
            }
            if let Err(error) = present_native_output(state, &output) {
                remove_native_output_surface(state, &window_label);
                let _ = output_window.close();
                return Err(error);
            }
            output_window.show().map_err(|error| error.to_string())?;
            keep_display_window_on_top(&output_window)?;
            present_native_output(state, &output)?;
        }
    }
    Ok(())
}

fn create_windowed_native_output_window(
    app: &Host,
    state: &AppState,
    main_thread_id: std::thread::ThreadId,
    window_label: &str,
    label: &str,
) -> Result<(Window, native_output::NativeOutputSurface), String> {
    let windows = app.clone();
    let window_label = window_label.to_string();
    let label = label.to_string();
    let create = move || {
        windows
            .windows()
            .create_windowed_output_window(&window_label, &label)
    };
    let window = if app.windows().output_windows_use_main_thread() {
        run_on_main_thread(app, main_thread_id, create)?
    } else {
        create()?
    };
    window.set_always_on_top(true).inspect_err(|_| {
        let _ = window.close();
    })?;
    let surface = create_shared_native_output_surface(
        app,
        state,
        window.clone(),
        960,
        540,
        native_hdr::OutputFormat::Sdr,
    )
    .inspect_err(|_| {
        let _ = window.close();
    })?;
    Ok((window, surface))
}

fn create_display_native_output_window(
    app: &Host,
    state: &AppState,
    main_thread_id: std::thread::ThreadId,
    window_label: &str,
    label: &str,
    bounds: DisplayBounds,
    output_format: native_hdr::OutputFormat,
) -> Result<(Window, native_output::NativeOutputSurface), String> {
    let windows = app.clone();
    let window_label = window_label.to_string();
    let label = label.to_string();
    let create = move || {
        windows
            .windows()
            .create_display_output_window(&window_label, &label, bounds)
    };
    let window = if app.windows().output_windows_use_main_thread() {
        run_on_main_thread(app, main_thread_id, create)?
    } else {
        create()?
    };
    window.set_position(PhysicalPosition::new(bounds.x, bounds.y))?;
    window.set_size(PhysicalSize::new(bounds.width, bounds.height))?;
    keep_display_window_on_top(&window)?;
    let surface = create_shared_native_output_surface(
        app,
        state,
        window.clone(),
        bounds.width,
        bounds.height,
        output_format,
    )
    .inspect_err(|_| {
        let _ = window.close();
    })?;
    Ok((window, surface))
}

fn create_display_output_control_window(
    app: &Host,
    screen_id: &str,
    bounds: DisplayBounds,
) -> Result<Window, String> {
    if let Some(window) = app.get_window(&display_control_window_label(screen_id)) {
        update_display_control_window(&window, bounds)?;
        return Ok(window);
    }
    let window = app.windows().create_output_control_window(
        screen_id,
        &display_control_window_label(screen_id),
        &display_window_label(screen_id),
        bounds,
    )?;
    window.set_position(display_control_position(&window, bounds)?)?;
    Ok(window)
}

fn create_shared_native_output_surface(
    app: &Host,
    state: &AppState,
    window: Window,
    width: u32,
    height: u32,
    output_format: native_hdr::OutputFormat,
) -> Result<native_output::NativeOutputSurface, String> {
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = app;
    #[cfg(target_os = "linux")]
    {
        let native_window = window.clone();
        let realize = move || {
            // Hidden display windows need a GDK handle before a GPU surface can be created.
            native_window.realize()
        };
        if app.windows().output_windows_use_main_thread() {
            run_on_main_thread(app, state.main_thread_id, realize)?;
        } else {
            realize()?;
        }
    }
    let shared_gpus = state
        .native_gpu_contexts
        .lock()
        .map_err(|_| "Native GPU context state is unavailable.".to_string())?
        .clone();
    #[cfg(target_os = "macos")]
    let surface = {
        // Metal attaches a CAMetalLayer to the NSView on the AppKit thread.
        run_on_main_thread(app, state.main_thread_id, move || {
            native_output::NativeOutputSurface::new_with_contexts(
                window,
                width,
                height,
                &shared_gpus,
                output_format,
            )
        })?
    };
    #[cfg(not(target_os = "macos"))]
    let surface = native_output::NativeOutputSurface::new_with_contexts(
        window,
        width,
        height,
        &shared_gpus,
        output_format,
    )?;
    let gpu = surface.gpu_context();
    let mut stored_gpus = state
        .native_gpu_contexts
        .lock()
        .map_err(|_| "Native GPU context state is unavailable.".to_string())?;
    if !stored_gpus.iter().any(|stored| Arc::ptr_eq(stored, &gpu)) {
        stored_gpus.push(gpu);
    }
    Ok(surface)
}

fn run_on_main_thread<T, F>(
    app: &Host,
    main_thread_id: std::thread::ThreadId,
    task: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    if std::thread::current().id() == main_thread_id {
        return task();
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = sender.send(task());
    })?;
    receiver
        .recv()
        .map_err(|_| "Native output main-thread initialization was interrupted.".to_string())?
}

fn present_native_output(state: &AppState, output: &WindowedOutput) -> Result<(), String> {
    present_native_output_for_generation(state, output, None)
}

fn present_native_output_for_generation(
    state: &AppState,
    output: &WindowedOutput,
    generation: Option<u64>,
) -> Result<(), String> {
    // Keep the read lock through presentation so Freeze waits for any in-flight frame.
    let frozen = state
        .output_freeze
        .read()
        .map_err(|_| "Output freeze state is unavailable.")?;
    if *frozen {
        if let Some(surface) = state
            .native_output_surfaces
            .lock()
            .map_err(|_| "Native output surface state is unavailable.")?
            .get_mut(&output.window_label)
        {
            surface.redraw()?;
        }
        return Ok(());
    }
    let is_current = || match generation {
        Some(generation) => state.output_render_requested.load(Ordering::Acquire) == generation,
        None => true,
    };
    if !is_current() {
        return Ok(());
    }
    if native_key_fill::present(state, output, generation)? {
        return Ok(());
    }
    let content = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .get(&output.source_screen_id)
        .cloned()
        .unwrap_or_else(|| json!({}));
    let content = canonical_output_content(state, &output.source_screen_id, content)?;
    let video_delay = output_delay::frames(content.get("videoDelay"))?;
    let (width, height, video_decode_size) = {
        let surfaces = state
            .native_output_surfaces
            .lock()
            .map_err(|_| "Native output surface state is unavailable.".to_string())?;
        let (width, height) = surfaces
            .get(&output.window_label)
            .map(native_output::NativeOutputSurface::dimensions)
            .ok_or_else(|| {
                format!(
                    "Native output surface '{}' is unavailable.",
                    output.window_label
                )
            })?;
        let largest = surfaces
            .values()
            .map(native_output::NativeOutputSurface::dimensions)
            .fold((width, height), |largest, size| {
                (largest.0.max(size.0), largest.1.max(size.1))
            });
        (width, height, largest)
    };
    if generation.is_some() && state.output_render_worker_active.load(Ordering::Acquire) {
        let frame_key = NativeFrameCacheKey {
            source_screen_id: output.source_screen_id.clone(),
            width,
            height,
        };
        let candidate = state
            .native_prerendered_frames
            .lock()
            .map_err(|_| "Native prerender cache is unavailable.".to_string())?
            .get(&frame_key)
            .filter(|cached| cached.layered_frame.is_some())
            .cloned();
        let prerendered = candidate
            .filter(|cached| {
                cached.visual_key == output_playback::scroll_content_key_for_value(&content)
            })
            .and_then(|cached| cached.layered_frame);
        if let Some(frame) = prerendered {
            if !is_current() {
                return Ok(());
            }
            return state
                .native_output_surfaces
                .lock()
                .map_err(|_| "Native output surface state is unavailable.".to_string())?
                .get_mut(&output.window_label)
                .ok_or_else(|| {
                    format!(
                        "Native output surface '{}' is unavailable.",
                        output.window_label
                    )
                })?
                .present_layered(&frame, video_delay);
        }
    }
    let frame = {
        let mut compositor = state
            .native_compositor
            .lock()
            .map_err(|_| "Native compositor is unavailable.".to_string())?;
        compositor.set_video_max_size(Some(video_decode_size));
        compositor.try_render_layered_for(
            &output.source_screen_id,
            &content,
            width as usize,
            height as usize,
        )?
    };
    if !is_current() {
        return Ok(());
    }
    state
        .native_output_surfaces
        .lock()
        .map_err(|_| "Native output surface state is unavailable.".to_string())?
        .get_mut(&output.window_label)
        .ok_or_else(|| {
            format!(
                "Native output surface '{}' is unavailable.",
                output.window_label
            )
        })?
        .present_layered(&frame, video_delay)
}

fn canonical_output_content(
    state: &AppState,
    source_screen_id: &str,
    mut content: Value,
) -> Result<Value, String> {
    let path = state
        .active_workspace
        .lock()
        .map_err(|_| "Workspace state is unavailable.".to_string())?
        .clone();
    let ticker = match path {
        Some(path) => native_ticker::snapshot(&path)?,
        None => Value::Null,
    };
    let rss_snapshot = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.")?
        .snapshot();
    let (ticker, _) = rss_output::resolve_ticker(ticker, &rss_snapshot);
    let linked = state.linked_services.snapshot()?;
    let ticker = linked.as_ref().map_or(&ticker, |v| &v["ticker"]);
    attach_native_ticker(&mut content, ticker);
    let capture_status = serde_json::to_value(
        state
            .capture_status
            .lock()
            .map_err(|_| "Recording status is unavailable.".to_string())?
            .clone(),
    )
    .map_err(|error| error.to_string())?;
    capture_commands::attach_status(&mut content, &capture_status);
    if timecode_display::uses_timecode(&content) {
        let timecode_status = serde_json::to_value(
            state
                .timecode_status
                .lock()
                .map_err(|_| "Timecode status is unavailable.")?
                .clone(),
        )
        .map_err(|error| error.to_string())?;
        timecode_display::attach_status(&mut content, &timecode_status);
    }
    crate::native_compositor::validate_stage_previews(&content)?;
    state
        .output_playback
        .lock()
        .map_err(|_| "Output playback state is unavailable.".to_string())
        .map(|mut playback| {
            let now = std::time::Instant::now();
            let mut scene = playback.canonical_content(source_screen_id, &content, now);
            if let Some(outputs) = scene
                .get_mut("linkedTextContext")
                .and_then(|context| context.get_mut("stageSceneResources"))
                .and_then(|resources| resources.get_mut("outputs"))
                .and_then(Value::as_object_mut)
            {
                for (source_id, resource) in outputs {
                    if let Some(child) = resource.get_mut("scene") {
                        *child = playback.canonical_content(source_id, child, now);
                    }
                }
            }
            scene
        })
}

fn attach_native_ticker(scene: &mut Value, ticker: &Value) {
    let Some(context) = scene
        .get_mut("linkedTextContext")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    context.insert("ticker".into(), ticker.clone());
    if let Some(resources) = context
        .get_mut("stageSceneResources")
        .and_then(Value::as_object_mut)
    {
        for (key, resource) in resources {
            if key == "outputs" {
                if let Some(outputs) = resource.as_object_mut() {
                    for child in outputs.values_mut() {
                        if let Some(scene) = child.get_mut("scene") {
                            attach_native_ticker(scene, ticker);
                        }
                    }
                }
            } else if let Some(scene) = resource.get_mut("scene") {
                attach_native_ticker(scene, ticker);
            }
        }
    }
}

fn resize_native_output_surface(
    state: &AppState,
    window_label: &str,
    size: PhysicalSize<u32>,
) -> Result<(), String> {
    let mut surfaces = state
        .native_output_surfaces
        .lock()
        .map_err(|_| "Native output surface state is unavailable.".to_string())?;
    let Some(surface) = surfaces.get_mut(window_label) else {
        return Ok(());
    };
    surface.resize(size.width, size.height)
}

fn remove_native_output_surface(state: &AppState, window_label: &str) -> bool {
    state
        .native_output_surfaces
        .lock()
        .ok()
        .and_then(|mut surfaces| surfaces.remove(window_label))
        .is_some()
}

fn tick_native_outputs(state: &AppState) -> Result<usize, String> {
    // A freshly selected slide owns the compositor until its first frame has
    // been presented. Otherwise a continuously rendered video frame can keep
    // the CPU compositor busy and make the slide change visibly lag behind.
    if state.output_render_worker_active.load(Ordering::Acquire) {
        return Ok(0);
    }
    let generation = state.output_render_requested.load(Ordering::Acquire);
    let mut outputs = state
        .display_outputs
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .values()
        .filter(|output| output.active)
        .cloned()
        .collect::<Vec<_>>();
    outputs.extend(
        state
            .windowed_outputs
            .lock()
            .map_err(|_| "Output state is unavailable.".to_string())?
            .values()
            .cloned(),
    );
    let mut presented = 0;
    let mut paired = HashSet::new();
    for output in outputs {
        let dynamic = state
            .native_output_surfaces
            .lock()
            .map_err(|_| "Native output surface state is unavailable.".to_string())?
            .get(&output.window_label)
            .is_some_and(native_output::NativeOutputSurface::has_dynamic_media);
        if dynamic {
            if !native_key_fill::claim(state, &output, &mut paired)? {
                continue;
            }
            synchronize_native_audio_source(state, &output.source_screen_id)?;
            present_native_output_for_generation(state, &output, Some(generation))?;
            if state.output_render_requested.load(Ordering::Acquire) != generation {
                break;
            }
            presented += 1;
        } else if output.target_id.is_some() {
            // Windows can discard a static swapchain frame when an overlay
            // becomes visible. Re-present retained GPU layers without decoding
            // or rasterizing the slide again.
            let mut surfaces = state
                .native_output_surfaces
                .lock()
                .map_err(|_| "Native output surface state is unavailable.".to_string())?;
            if let Some(surface) = surfaces.get_mut(&output.window_label) {
                surface.redraw()?;
                presented += 1;
            }
        }
    }
    Ok(presented)
}

fn tick_native_messages(app: &Host, state: &AppState) -> Result<(), String> {
    let rss_snapshot = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.")?
        .snapshot();
    let path = state
        .active_workspace
        .lock()
        .map_err(|_| "Workspace state is unavailable.")?
        .clone();
    if let Some(path) = path {
        let (_, pending) =
            rss_output::resolve_ticker(native_ticker::snapshot(&path)?, &rss_snapshot);
        schedule_output_rss_refreshes(app, pending);
    }
    let mut coordinator = lock_output_snapshot_state(&state.output_snapshot)?;
    let Some(cached) = coordinator.cache.as_ref() else {
        return Ok(());
    };
    let mut refreshed = {
        let mut timers = state
            .native_timers
            .lock()
            .map_err(|_| "Native timer state is unavailable.")?;
        state
            .native_messages
            .lock()
            .map_err(|_| "Native message state is unavailable.")?
            .refresh(&cached.snapshot, &mut timers)?
    };
    let prop_refresh = state
        .native_props
        .lock()
        .map_err(|_| "Native prop state is unavailable.")?
        .refresh(
            refreshed
                .as_ref()
                .map(|(snapshot, _)| snapshot)
                .unwrap_or(&cached.snapshot),
        )?;
    if let Some((snapshot, prop_patch)) = prop_refresh {
        let mut patch = refreshed
            .map(|(_, patch)| patch)
            .unwrap_or_else(|| json!({}));
        patch.as_object_mut().expect("native patch object").extend(
            prop_patch
                .as_object()
                .expect("native prop patch object")
                .clone(),
        );
        refreshed = Some((snapshot, patch));
    }
    let Some((snapshot, patch)) = refreshed else {
        return Ok(());
    };
    let (revision, pending) = commit_native_output_snapshot(
        state,
        &mut coordinator,
        snapshot,
        &rss_snapshot,
        None,
        None,
    )?;
    drop(coordinator);
    schedule_output_rss_refreshes(app, pending);
    schedule_output_consumer_refresh(app);
    let _ = app.emit("output:scene-committed", revision);
    let _ = app.emit_to(
        "main",
        "native-output-state-patch",
        json!({"source":"remote", "revision":revision,"patch":patch}),
    );
    Ok(())
}

/// Drain native decoded PCM outside the audio callback, then hand it to NDI
/// and the running recording without holding the native-audio mutex. The audio
/// source only writes into bounded lock-free tap rings; all routing, delay, ABI
/// conversion, and NDI calls happen on this ticker thread.
fn tick_ndi_audio(state: &AppState) -> Result<usize, String> {
    let ndi_ids = {
        let ndi = state
            .ndi
            .lock()
            .map_err(|_| "NDI state is unavailable.".to_string())?;
        if ndi.audio_available() {
            ndi.active_output_ids()
        } else {
            Vec::new()
        }
    };
    let recorded = state.capture_audio.source_screen_id();
    let mut source_ids = ndi_ids.clone();
    if let Some(screen_id) = recorded.as_ref().filter(|id| !ndi_ids.contains(id)) {
        source_ids.push(screen_id.clone());
    }
    if source_ids.is_empty() {
        return Ok(0);
    }
    let drained = {
        let mut native_audio = state
            .native_audio
            .lock()
            .map_err(|_| "Native audio state is unavailable.".to_string())?;
        source_ids
            .into_iter()
            .filter_map(|source_id| {
                let blocks = native_audio.drain_ndi_audio(&source_id);
                (!blocks.is_empty()).then_some((source_id, blocks))
            })
            .collect::<Vec<_>>()
    };
    for (_, blocks) in drained
        .iter()
        .filter(|(source_id, _)| recorded.as_ref() == Some(source_id))
    {
        for block in blocks {
            state.capture_audio.push(
                &block.samples,
                block.sample_rate,
                block.channels,
                block.start_time(),
            );
        }
    }
    let drained = drained
        .into_iter()
        .filter(|(source_id, _)| ndi_ids.contains(source_id))
        .collect::<Vec<_>>();
    if drained.is_empty() {
        return Ok(0);
    }
    let mut ndi = state
        .ndi
        .lock()
        .map_err(|_| "NDI state is unavailable.".to_string())?;
    let mut sent = 0;
    for (source_id, blocks) in drained {
        for block in blocks {
            ndi.send_audio_pcm(
                &source_id,
                &block.samples,
                block.sample_rate,
                block.channels,
            )?;
            sent += 1;
        }
    }
    Ok(sent)
}

fn tick_audio_playlist_events(app: &Host, state: &AppState) -> Result<(), String> {
    let ended = {
        let mut audio = state
            .native_audio
            .lock()
            .map_err(|_| "Native audio state is unavailable.".to_string())?;
        audio.music_ended_sources()
    };
    if ended.is_empty() {
        return Ok(());
    }
    let workspace_path = match state.output_snapshot.lock() {
        Ok(output) => output.cache.as_ref().and_then(|cache| {
            cache.snapshot["playbackWorkspacePath"]
                .as_str()
                .map(PathBuf::from)
        }),
        Err(_) => return Err("Output snapshot is unavailable.".into()),
    }
    .or(state
        .active_workspace
        .lock()
        .map_err(|_| "Workspace state is unavailable.")?
        .clone());
    let Some(workspace_path) = workspace_path else {
        return Ok(());
    };
    let content = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?;
    for (source_id, identity) in ended {
        let Some(scene) = content.get(&source_id) else {
            continue;
        };
        let Some(audio_id) = scene.get("audioId").and_then(Value::as_str) else {
            continue;
        };
        let Some(selection_id) = scene.get("audioSelectionId").and_then(Value::as_u64) else {
            continue;
        };
        if format!("{}|{}", json!(audio_id), json!(selection_id)) != identity {
            continue;
        }
        app.emit_to(
            "main",
            "presenter-orbit:music-playback-ended",
            json!({
                "audioId": audio_id, "selectionId": selection_id, "workspacePath": workspace_path
            }),
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn completion_program_scene(state: &AppState) -> Result<Value, String> {
    let source_id = state
        .output_snapshot
        .lock()
        .map_err(|_| "Output snapshot is unavailable.".to_string())?
        .cache
        .as_ref()
        .and_then(|cache| cache.snapshot.get("screenConfig"))
        .and_then(Value::as_object)
        .and_then(|screens| {
            screens.iter().find_map(|(id, config)| {
                (config.get("type").and_then(Value::as_str) == Some("audience")
                    && config.get("contentDestination").and_then(Value::as_str)
                        != Some("announcements"))
                .then(|| id.clone())
            })
        });
    let Some(source_id) = source_id else {
        return Ok(Value::Null);
    };
    let scene = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.".to_string())?
        .get(&source_id)
        .cloned();
    match scene {
        Some(scene) => canonical_output_content(state, &source_id, scene),
        None => Ok(Value::Null),
    }
}

/// Loads the saved state the host needs before it serves anything. Returns the general settings.
pub fn start_host(app: &Host) -> Result<Value, String> {
    network_link_commands::load(app, app.state::<AppState>())?;
    let startup_settings = read_settings(app)?;
    network_link_discovery::start(app);
    let general_settings =
        settings::normalize_general_settings_value(&startup_settings["generalSettings"]);
    general_settings_options::set(&general_settings);
    native_hdr::set_applied_bit_depth(native_hdr::BitDepth::parse(
        general_settings["renderingBitDepth"]
            .as_str()
            .unwrap_or_default(),
    )?);
    if let Err(error) = timecode_input::restore(app) {
        eprintln!("Timecode input settings: {error}");
    }
    record_diagnostic(
        app,
        "host-started",
        diagnostics::LogLevel::Info,
        "host",
        None,
    );
    Ok(general_settings)
}

/// Starts the thread that drives output frames, NDI, automation, and messages.
pub fn start_tickers(app: &Host) -> std::io::Result<()> {
    start_ticker_thread(app, false)
}

/// What an output process needs at startup: the rendering bit depth. It starts no network discovery, timecode
/// input, or automation, and reads no settings file.
pub fn start_output_host(app: &Host, rendering_bit_depth: &str) -> Result<(), String> {
    app.state::<AppState>()
        .idle_program_audio
        .store(true, Ordering::Relaxed);
    let requested = native_hdr::BitDepth::parse(rendering_bit_depth)?;
    native_hdr::bit_depth_compatible(native_hdr::applied_bit_depth_if_set(), requested)?;
    native_hdr::set_applied_bit_depth(requested);
    Ok(())
}

/// The output process tickers: NDI frames and audio, display close controls, and native output frames. A panic
/// in one pass is recorded and the next pass runs, because the process holds the live windows.
pub fn start_output_tickers(app: &Host) -> std::io::Result<()> {
    start_ticker_thread(app, true)
}

/// Whether an output scene patch may carry `field`; a change to any other field needs a full snapshot.
pub fn output_patch_supports(field: &str) -> bool {
    output_scene::patch_field_supported(field)
}

/// Whether any window or NDI sender remains. An output process with none and no controller has nothing to keep.
pub fn has_physical_outputs(app: &Host) -> bool {
    let state = app.state::<AppState>();
    let windowed = state
        .windowed_outputs
        .lock()
        .is_ok_and(|outputs| !outputs.is_empty());
    let displays = state
        .display_outputs
        .lock()
        .is_ok_and(|outputs| outputs.values().any(|output| output.active));
    let ndi = state
        .ndi
        .lock()
        .is_ok_and(|ndi| !ndi.active_output_ids().is_empty());
    windowed || displays || ndi
}

/// Stops the output tickers and closes every window and NDI sender.
pub fn stop_output_host(app: &Host) {
    let state = app.state::<AppState>();
    state.ndi_ticker_stop.store(true, Ordering::Release);
    shutdown_all_outputs(app, state);
}

fn start_ticker_thread(app: &Host, output_process: bool) -> std::io::Result<()> {
    let ticker_app = app.clone();
    let ticker_stop = app.state::<AppState>().ndi_ticker_stop.clone();
    std::thread::Builder::new()
        .name("presenter-orbit-ndi-ticker".to_string())
        .spawn(move || {
            let mut next_native_output_frame = std::time::Instant::now();
            let mut next_automation_tick = std::time::Instant::now();
            let mut hovered_display_outputs = HashSet::new();
            let mut completion_scene_error = None;
            let mut ndi_tick_error = None;
            let mut message_tick_error = None;
            let mut audio_marker_tick_error = None;
            let mut next_cache_keepalive = std::time::Instant::now();
            let mut pass = |ticker_app: &Host| {
                if let Some(state) = ticker_app.try_state::<AppState>() {
                    if let Ok(mut ndi) = state.ndi.lock() {
                        match ndi.tick(std::time::Instant::now(), |id, content| {
                            canonical_output_content(state, id, content.clone())
                        }) {
                            Ok(_) => ndi_tick_error = None,
                            Err(error) => {
                                if ndi_tick_error.as_ref() != Some(&error) {
                                    eprintln!("NDI frame projection failed: {error}");
                                    ndi_tick_error = Some(error);
                                }
                            }
                        }
                    }
                    let _ = tick_ndi_audio(state);
                    let now = std::time::Instant::now();
                    if now >= next_native_output_frame {
                        let _ = tick_display_output_controls(
                            &ticker_app,
                            state,
                            &mut hovered_display_outputs,
                        );
                        let _ = tick_native_outputs(state);
                        if let Ok(mut audio) = state.native_audio.lock() {
                            audio.tick_crossfades(now);
                        }
                        next_native_output_frame = now + std::time::Duration::from_millis(33);
                    }
                    if output_process {
                        let _ = tick_audio_playlist_events(ticker_app, state);
                    }
                    if output_process && now >= next_cache_keepalive {
                        native_media::keep_cache_session_alive();
                        next_cache_keepalive = now + std::time::Duration::from_secs(60 * 60);
                    }
                    if !output_process && now >= next_automation_tick {
                        match state.native_audio.lock() {
                            Ok(audio) => {
                                audio.publish_music_marker_positions();
                                audio_marker_tick_error = None;
                            }
                            Err(error) => {
                                let error = error.to_string();
                                if audio_marker_tick_error.as_ref() != Some(&error) {
                                    eprintln!("Audio marker position publication failed: {error}");
                                    audio_marker_tick_error = Some(error);
                                }
                            }
                        }
                        match tick_native_messages(&ticker_app, state) {
                            Ok(()) => message_tick_error = None,
                            Err(error) => {
                                if message_tick_error.as_ref() != Some(&error) {
                                    eprintln!("Message refresh failed: {error}");
                                    message_tick_error = Some(error);
                                }
                            }
                        }
                        let completion_scene = match completion_program_scene(state) {
                            Ok(scene) => {
                                completion_scene_error = None;
                                scene
                            }
                            Err(error) => {
                                if completion_scene_error.as_ref() != Some(&error) {
                                    eprintln!("Failed to bind media-end completion: {error}");
                                    completion_scene_error = Some(error);
                                }
                                Value::Null
                            }
                        };
                        let due = state
                            .native_automation
                            .lock()
                            .map(|mut scheduler| {
                                scheduler.update_completion_media(&completion_scene);
                                scheduler.tick_now()
                            })
                            .unwrap_or_default();
                        tick_timeline_soundtrack(&ticker_app, state);
                        for event in due {
                            let event = apply_native_output_due_event(&ticker_app, state, &event)
                                .ok()
                                .flatten()
                                .unwrap_or(event);
                            let _ = ticker_app.emit("automation-due", event);
                        }
                        let _ = tick_audio_playlist_events(ticker_app, state);
                        next_automation_tick = now + std::time::Duration::from_millis(25);
                    }
                }
            };
            while !ticker_stop.load(Ordering::Acquire) {
                if output_process {
                    if let Err(panic) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pass(&ticker_app)))
                    {
                        record_diagnostic(
                            &ticker_app,
                            "ticker-panic",
                            diagnostics::LogLevel::Error,
                            "output",
                            panic
                                .downcast_ref::<&str>()
                                .copied()
                                .or_else(|| panic.downcast_ref::<String>().map(String::as_str)),
                        );
                    }
                } else {
                    pass(&ticker_app);
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        })?;
    Ok(())
}

/// Stops background services and native outputs when the application exits.
pub fn stop_host(app: &Host) {
    let state = app.state::<AppState>();
    state.ndi_ticker_stop.store(true, Ordering::Release);
    if let Err(error) = communication_midi::release() {
        eprintln!("MIDI shutdown failed: {error}");
    }
    camera_input::shutdown();
    let _ = capture_commands::stop_capture(state);
    if let Err(error) = native_ticker::clear() {
        eprintln!("Ticker shutdown failed: {error}");
    }
    // RSS requests have a bounded native worker and a join
    // handle. Stop it before tearing down native surfaces;
    // no output/protocol/cache lock is held here.
    state.rss_service.shutdown();
    state.local_text.shutdown();
    shutdown_all_outputs(app, state);
}

fn tick_timeline_soundtrack(app: &Host, state: &AppState) {
    let target = state
        .native_automation
        .lock()
        .ok()
        .and_then(|scheduler| scheduler.soundtrack_target());
    let Ok(mut player) = state.native_soundtrack.lock() else {
        return;
    };
    let before = player.error().map(str::to_owned);
    player.sync(target.as_ref(), std::time::Instant::now());
    let after = player.error().map(str::to_owned);
    if before != after {
        if let Some(error) = &after {
            eprintln!("Timeline soundtrack: {error}");
        }
        let _ = app.emit("timeline-soundtrack-status", json!({ "error": after }));
    }
}

const INSPECTOR_PREVIEW_SIZE: (usize, usize) = (960, 540);

fn update_video_thumbnail(
    state: &AppState,
    method: &str,
    args: &[Value],
    preview: Option<(&Value, &native_compositor::NativeFrame)>,
) -> Result<Value, String> {
    let path_argument = argument(args, 0)?
        .as_str()
        .ok_or("Workspace path is required.")?
        .to_owned();
    let path = active_workspace(state, json!(path_argument))?;
    let media_id = argument(args, 1)?
        .as_str()
        .ok_or("Video asset id is required.")?
        .to_owned();
    let reset = method == "resetVideoThumbnail";
    let revision_index = if reset { 2 } else { 3 };
    let expected_revision = expected_workspace_revision(args, revision_index)?;
    let source = {
        let mut workspace_io = lock_workspace_io(state)?;
        workspace_io.require_revision(&path, expected_revision)?;
        media_thumbnail::source(&path, &media_id)?
    };
    let seconds = if reset {
        0.0
    } else {
        argument(args, 2)?
            .as_f64()
            .ok_or("Thumbnail time must be a number.")?
    };
    let frame = if reset {
        None
    } else if let Some((asset, preview)) = preview {
        if asset["id"] != media_id || media_thumbnail::asset_path(&path, asset)? != source {
            return Err("The preview source changed before the thumbnail could be saved.".into());
        }
        if !seconds.is_finite()
            || seconds < 0.
            || preview.dynamic_media
            || (preview.width, preview.height) != INSPECTOR_PREVIEW_SIZE
            || preview.pixels.len() != preview.width * preview.height * 4
        {
            return Err(
                "Inspector thumbnail requires a complete preview frame and a valid time.".into(),
            );
        }
        Some(preview.encode_png()?)
    } else {
        Some(media_thumbnail::extract(&source, seconds)?)
    };
    let mut workspace_io = lock_workspace_io(state)?;
    if active_workspace(state, json!(path_argument))? != path {
        return Err("Workspace changed during thumbnail extraction.".into());
    }
    workspace_io.require_revision(&path, expected_revision)?;
    let mut workspace = media_thumbnail::store(
        &path,
        &media_id,
        &source,
        frame.as_deref().map(|bytes| (bytes, seconds)),
    )?;
    let revision = workspace_io.advance_revision(&path);
    let cleanup_warning = workspace
        .as_object_mut()
        .and_then(|object| object.remove("thumbnailCleanupWarning"));
    Ok(json!({ "workspace": workspace, "revision": revision, "cleanupWarning": cleanup_warning }))
}

/// Whether this host owns the physical outputs. A controller host holds the model for an output process and
/// never creates a window, NDI sender, or audio sink for program output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputRole {
    Full,
    Controller,
}

/// Creates the services every host shares, keyed by type.
pub fn host_states(main_thread_id: std::thread::ThreadId) -> HostStates {
    states_with_role(main_thread_id, OutputRole::Full)
}

/// The services of a host whose physical outputs live in another process.
pub fn host_states_controller(main_thread_id: std::thread::ThreadId) -> HostStates {
    states_with_role(main_thread_id, OutputRole::Controller)
}

fn states_with_role(main_thread_id: std::thread::ThreadId, role: OutputRole) -> HostStates {
    let timecode = timecode_input::TimecodeService::default();
    let mut states = HostStates::default();
    states.insert(AppState::new(
        main_thread_id,
        timecode.status_handle(),
        role,
    ));
    states.insert(timecode);
    states.insert(midi_input::MidiInputService::default());
    states.insert(external_presentation::ExternalPresentationService::default());
    states
}

/// Stops every native output, e.g. before an installer replaces the application.
pub fn shutdown_outputs(app: &Host) {
    shutdown_all_outputs(app, app.state::<AppState>());
}

/// Independent operator audition, separate from program output sinks.
pub use native_audio::PreviewAudio;

/// Validates an operator media transport draft using the native playback contract.
pub fn validate_preview_media_transport(value: &Value) -> Result<(), String> {
    output_playback::validate_media_transport(value)
}

fn string_argument(args: &[Value], index: usize, label: &str) -> Result<String, String> {
    args.get(index)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{label} must be a string."))
}

fn manage_media_setting(app: &Host) -> Result<bool, String> {
    Ok(
        settings::normalize_general_settings_value(&read_settings(app)?["generalSettings"])
            ["manageMediaAutomatically"]
            == true,
    )
}

/// Where new and duplicated workspaces go instead of the user's data folder. A shell that runs against a
/// private data directory (its tests) registers this so it never writes to the real one.
pub struct WorkspacesFolder(pub PathBuf);

fn default_workspaces_folder(app: &Host) -> Result<PathBuf, String> {
    if let Some(folder) = app.try_state::<WorkspacesFolder>() {
        return Ok(folder.0.clone());
    }
    #[cfg(test)]
    let root = app.path().app_data_dir()?;
    #[cfg(not(test))]
    let root = dirs::data_dir().ok_or("Application data directory is unavailable.")?;
    Ok(root.join("PresenterOrbit").join("Workspaces"))
}

fn workspace_folder_name(workspace_json: &Path) -> String {
    workspace_json
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| DEFAULT_WORKSPACE_NAME.to_string())
}

fn orbit_workspace_items(settings: &Value) -> Result<Vec<Value>, String> {
    let active = settings
        .get("presenterOrbitWorkspacePath")
        .and_then(Value::as_str);
    Ok(workspace_registry::parse(
        settings
            .get("presenterOrbitWorkspaces")
            .unwrap_or(&Value::Null),
    )?
    .into_iter()
    .map(|entry| {
        json!({
            "name": entry.name,
            "active": Some(entry.path.as_str()) == active,
            "missing": !Path::new(&entry.path).is_file(),
            "path": entry.path,
        })
    })
    .collect())
}

fn list_orbit_workspaces(app: &Host, state: &AppState) -> Result<Value, String> {
    let _settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    let settings = read_settings(app)?;
    Ok(json!({
        "workspaces": orbit_workspace_items(&settings)?,
        "defaultFolder": default_workspaces_folder(app)?,
    }))
}

fn create_orbit_workspace(app: &Host, state: &AppState, name: String) -> Result<Value, String> {
    let name = workspace_registry::validate_name(&name)?;
    let folder = default_workspaces_folder(app)?.join(&name);
    if folder.exists() {
        return Err(format!("A folder named \"{name}\" already exists."));
    }
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let result = activate_orbit_workspace(
        app,
        state,
        workspace::workspace_path(&folder),
        |registry, path| workspace_registry::add(registry, &name, path),
    );
    if result.is_err() {
        // The folder was created by this call and is only populated by ensure_workspace.
        let _ = fs::remove_dir_all(&folder);
    }
    result
}

fn switch_orbit_workspace(app: &Host, state: &AppState, path: String) -> Result<Value, String> {
    {
        let _settings_operation = state
            .settings_operations
            .lock()
            .map_err(|_| "Settings state is unavailable.".to_string())?;
        let settings = read_settings(app)?;
        let registry = workspace_registry::parse(
            settings
                .get("presenterOrbitWorkspaces")
                .unwrap_or(&Value::Null),
        )?;
        if !registry.iter().any(|entry| entry.path == path) {
            return Err("Workspace is not registered.".to_string());
        }
    }
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Err("Workspace no longer exists on disk.".to_string());
    }
    activate_orbit_workspace(app, state, path, |_, _| Ok(()))
}

fn rename_orbit_workspace(
    app: &Host,
    state: &AppState,
    path: String,
    name: String,
) -> Result<Value, String> {
    let name = workspace_registry::validate_name(&name)?;
    update_orbit_workspace_registry(app, state, |registry, _| {
        workspace_registry::rename(registry, &path, &name)
    })
}

fn duplicate_orbit_workspace(
    app: &Host,
    state: &AppState,
    path: String,
    name: String,
) -> Result<Value, String> {
    let name = workspace_registry::validate_name(&name)?;
    let target = default_workspaces_folder(app)?.join(&name);
    if target.exists() {
        return Err(format!("A folder named \"{name}\" already exists."));
    }
    let source = Path::new(&path)
        .parent()
        .ok_or_else(|| "Workspace path has no folder.".to_string())?
        .to_path_buf();
    let target_json = workspace::workspace_path(&target);
    let result = update_orbit_workspace_registry(app, state, |registry, _| {
        if !registry.iter().any(|entry| entry.path == path) {
            return Err("Workspace is not registered.".to_string());
        }
        workspace_registry::require_unique_name(registry, &name, None)?;
        // Holding workspace I/O keeps the copy consistent with in-flight saves.
        let _workspace_io = lock_workspace_io(state)?;
        workspace_registry::copy_folder(
            &source,
            &target,
            workspace::WORKSPACE_TRANSACTION_DIRECTORY,
        )?;
        let mut document: Value =
            serde_json::from_slice(&fs::read(&target_json).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        document["id"] = json!(uuid::Uuid::new_v4().to_string());
        fs::write(
            &target_json,
            serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let target_json = target_json
            .canonicalize()
            .map_err(|error| error.to_string())?;
        workspace_registry::add(registry, &name, &target_json.to_string_lossy())
    });
    if result.is_err() && target.exists() {
        // The folder was created by this call, so a failed duplicate leaves nothing behind.
        let _ = fs::remove_dir_all(&target);
    }
    result
}

fn delete_orbit_workspace(app: &Host, state: &AppState, path: String) -> Result<Value, String> {
    update_orbit_workspace_registry(app, state, |registry, active| {
        workspace_registry::forget(registry, &path, active)?;
        let manifest = Path::new(&path);
        if !manifest.is_file() {
            return Ok(());
        }
        let folder = manifest
            .parent()
            .ok_or_else(|| "Workspace folder is unavailable.".to_string())?;
        os_trash::move_to_trash(folder)
    })
}

fn update_orbit_workspace_registry(
    app: &Host,
    state: &AppState,
    update: impl FnOnce(&mut Vec<workspace_registry::Entry>, Option<&str>) -> Result<(), String>,
) -> Result<Value, String> {
    let _settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    let settings = read_settings(app)?;
    let mut registry = workspace_registry::parse(
        settings
            .get("presenterOrbitWorkspaces")
            .unwrap_or(&Value::Null),
    )?;
    update(
        &mut registry,
        settings
            .get("presenterOrbitWorkspacePath")
            .and_then(Value::as_str),
    )?;
    let saved = save_settings_locked(
        app,
        state,
        json!({ "presenterOrbitWorkspaces": registry }),
        None,
    )?;
    app.emit("workspace-list-changed", ())
        .map_err(|error| error.to_string())?;
    Ok(json!({
        "workspaces": orbit_workspace_items(&saved)?,
        "settingsRevision": saved.get("settingsRevision").and_then(Value::as_u64),
    }))
}

fn activate_orbit_workspace(
    app: &Host,
    state: &AppState,
    path: PathBuf,
    update_registry: impl FnOnce(&mut Vec<workspace_registry::Entry>, &str) -> Result<(), String>,
) -> Result<Value, String> {
    let mut inputs = communication_input_commands::lock(state)?;
    inputs.stop_all();
    capture_commands::stop_capture(state)?;
    native_ticker::clear()?;
    let group_settings = read_normalized_group_settings(app, state)?;
    let _settings_operation = state
        .settings_operations
        .lock()
        .map_err(|_| "Settings state is unavailable.".to_string())?;
    let mut settings = read_settings(app)?;
    normalize_native_settings(&mut settings)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace::ensure_workspace(&path)?;
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    let mut registry = workspace_registry::parse(
        settings
            .get("presenterOrbitWorkspaces")
            .unwrap_or(&Value::Null),
    )?;
    update_registry(&mut registry, &path.to_string_lossy())?;
    let mut workspace = workspace::read_workspace(&path)?;
    workspace::merge_missing_workspace_group_settings(&mut workspace, &group_settings);
    let effective_screen_config = effective_screen_config_for_mutation(
        &workspace,
        &native_screen_config_for_mutation(&settings),
    );
    {
        let mut active = state
            .active_workspace
            .lock()
            .map_err(|_| "Workspace state is unavailable.".to_string())?;
        state.local_text.shutdown();
        *active = Some(path.clone());
    }
    let saved_settings = save_settings_locked(
        app,
        state,
        json!({ "presenterOrbitWorkspacePath": path, "presenterOrbitWorkspaces": registry }),
        None,
    )?;
    let revision = workspace_io.activate(&path);
    state
        .network_link
        .lock()
        .map_err(|_| "Network Link state is unavailable.")?
        .invalidate_pending();
    state
        .native_timers
        .lock()
        .map_err(|_| "Native timer state is unavailable.".to_string())?
        .clear();
    *state
        .native_messages
        .lock()
        .map_err(|_| "Native message state is unavailable.".to_string())? =
        native_messages::MessageRuntime::default();
    *state
        .native_props
        .lock()
        .map_err(|_| "Native prop state is unavailable.")? = native_props::PropRuntime::default();
    let activation = json!({
        "canceled": false,
        "workspace": workspace,
        "revision": revision,
        "effectiveScreenConfig": effective_screen_config,
        "settingsRevision": saved_settings.get("settingsRevision").and_then(Value::as_u64),
    });
    // The activating window may be an auxiliary window; only the main window owns the loaded workspace state.
    app.emit_to("main", "workspace-activated", &activation)
        .map_err(|error| error.to_string())?;
    app.emit("workspace-list-changed", ())
        .map_err(|error| error.to_string())?;
    Ok(activation)
}

fn select_audio_files() -> Result<Value, String> {
    let Some(paths) = FileDialog::new()
        .set_title("Import audio")
        .add_filter("Audio", &["mp3", "wav", "m4a", "aac", "aiff", "flac"])
        .pick_files()
    else {
        return Ok(json!({ "canceled": true }));
    };
    let stamp = Utc::now().timestamp_millis();
    let media = paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let title = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("Audio");
            let mut entry = serde_json::Map::new();
            entry.insert(
                "id".into(),
                json!(format!("audio-{stamp}-{index}-{}", safe_slug(title))),
            );
            entry.insert("title".into(), json!(title));
            entry.insert("type".into(), json!("audio"));
            entry.insert("sourcePath".into(), json!(path));
            entry.insert("sourceUrl".into(), json!(file_url(path)?));
            general_settings_options::apply(&mut entry);
            Ok(Value::Object(entry))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({ "canceled": false, "media": media }))
}

fn media_search_locations(_app: &Host) -> Result<Value, String> {
    let mut locations = Vec::new();
    for (kind, path) in [
        ("documents", dirs::document_dir()),
        ("downloads", dirs::download_dir()),
        ("music", dirs::audio_dir()),
        ("pictures", dirs::picture_dir()),
        ("videos", dirs::video_dir()),
        ("home", dirs::home_dir()),
        ("appData", dirs::data_dir()),
    ] {
        if let Some(path) = path {
            locations.push(json!({ "kind": kind, "path": path }));
        }
    }
    #[cfg(windows)]
    {
        if let Some(path) = std::env::var_os("ProgramData") {
            locations.push(json!({ "kind": "programData", "path": PathBuf::from(path) }));
        }
        let system = std::env::var("SystemDrive")
            .unwrap_or_default()
            .to_ascii_uppercase();
        for letter in b'A'..=b'Z' {
            let drive = format!("{}:", letter as char);
            let root = PathBuf::from(format!("{drive}\\"));
            if root.is_dir() {
                let kind = if drive == system {
                    "bootVolume"
                } else {
                    "volume"
                };
                locations.push(json!({ "kind": kind, "path": root }));
            }
        }
    }
    #[cfg(not(windows))]
    {
        locations.push(json!({ "kind": "bootVolume", "path": "/" }));
        for entry in fs::read_dir("/Volumes").into_iter().flatten().flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                locations.push(json!({ "kind": "volume", "path": entry.path() }));
            }
        }
    }
    Ok(json!(locations))
}

fn relink_missing_media_from_search_paths(
    app: &Host,
    state: &AppState,
    args: &[Value],
) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let general =
        settings::normalize_general_settings_value(&read_settings(app)?["generalSettings"]);
    let search_paths = general["mediaSearchPaths"]
        .as_array()
        .ok_or("Media search paths must be an array.")?
        .iter()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let base_revision = expected_workspace_revision(args, 2)?;
    let overview = {
        let mut workspace_io = lock_workspace_io(state)?;
        workspace_io.require_revision(&path, base_revision)?;
        media_relink::missing(&path, &argument(args, 1)?)?
    };
    // Searching whole volumes can take a while; workspace I/O stays available meanwhile.
    let matches = media_relink::search_missing_sources(&overview, &search_paths)?;
    let mut workspace_io = lock_workspace_io(state)?;
    workspace_io.require_revision(&path, base_revision)?;
    let (workspace, relinked) =
        media_relink::relink_matches(&path, &matches, general["manageMediaAutomatically"] == true)?;
    let revision = if workspace.is_some() {
        workspace_io.advance_revision(&path)
    } else {
        workspace_io.revision(&path)
    };
    Ok(json!({ "workspace": workspace, "relinked": relinked, "revision": revision }))
}
