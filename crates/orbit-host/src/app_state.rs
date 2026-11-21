use super::*;

pub(crate) struct AppState {
    pub(crate) output_role: OutputRole,
    pub(crate) output_freeze: std::sync::RwLock<bool>,
    /// An output process plays the audio of every main source that holds content, also one no window shows.
    pub(crate) idle_program_audio: AtomicBool,
    pub(crate) main_thread_id: std::thread::ThreadId,
    pub(crate) active_workspace: Arc<Mutex<Option<PathBuf>>>,
    pub(crate) workspace_io: Arc<Mutex<WorkspaceIoState>>,
    pub(crate) settings_operations: Mutex<()>,
    pub(crate) settings_revision: Mutex<SettingsRevisionState>,
    pub(crate) native_automation: Mutex<native_automation::NativeAutomationScheduler>,
    pub(crate) library_search: Mutex<library_search::LibrarySearchIndex>,
    pub(crate) output_content: Mutex<HashMap<String, Value>>,
    pub(crate) output_revisions: Mutex<HashMap<String, u64>>,
    pub(crate) output_snapshot: Mutex<OutputSnapshotState>,
    pub(crate) rss_output: Mutex<rss_output::OutputRssCache>,
    pub(crate) rss_service: rss_output::OutputRssService,
    pub(crate) local_text: local_text::LocalTextService,
    pub(crate) linked_services: crate::linked_services::LinkedServices,
    pub(crate) output_playback: Mutex<output_playback::PlaybackRegistry>,
    pub(crate) native_timers: Mutex<native_timers::TimerRegistry>,
    pub(crate) native_messages: Mutex<native_messages::MessageRuntime>,
    pub(crate) native_props: Mutex<native_props::PropRuntime>,
    pub(crate) native_audio: Mutex<native_audio::NativeAudioRegistry>,
    pub(crate) native_soundtrack: Mutex<native_soundtrack::SoundtrackPlayer>,
    pub(crate) capture: Mutex<capture::CaptureService>,
    pub(crate) capture_status: Arc<Mutex<capture::CaptureStatus>>,
    pub(crate) timecode_status: Arc<Mutex<timecode_input::TimecodeStatus>>,
    pub(crate) capture_audio: capture::CaptureAudio,
    pub(crate) native_compositor: Mutex<native_compositor::NativeCompositor>,
    pub(crate) native_key_fill_pairs: Mutex<native_key_fill::DisplayPairs>,
    pub(crate) native_preview_compositor: Mutex<native_compositor::NativeCompositor>,
    pub(crate) native_prerender_compositor: Mutex<native_compositor::NativeCompositor>,
    pub(crate) output_render_requested: AtomicU64,
    pub(crate) output_render_worker_active: AtomicBool,
    pub(crate) output_prerender_worker_active: AtomicBool,
    pub(crate) native_frame_cache: Mutex<HashMap<NativeFrameCacheKey, CachedNativeFrame>>,
    pub(crate) native_prerendered_frames:
        Mutex<HashMap<NativeFrameCacheKey, CachedPrerenderedFrame>>,
    pub(crate) native_preview_request_sizes: Mutex<HashMap<String, (u32, u32)>>,
    pub(crate) native_gpu_contexts: Mutex<Vec<Arc<native_output::NativeGpuContext>>>,
    pub(crate) native_output_surfaces: Mutex<HashMap<String, native_output::NativeOutputSurface>>,
    pub(crate) display_outputs: Mutex<HashMap<String, WindowedOutput>>,
    pub(crate) windowed_outputs: Mutex<HashMap<String, WindowedOutput>>,
    pub(crate) remote_server: Mutex<remote::RemoteServer>,
    pub(crate) network_link: Mutex<network_link::NetworkLinkService>,
    pub(crate) communication_inputs: Mutex<communication_input::Inputs>,
    pub(crate) remote_status: remote::RemoteStatusHandle,
    pub(crate) ndi: Mutex<ndi::NdiService>,
    pub(crate) ndi_ticker_stop: Arc<AtomicBool>,
}

impl AppState {
    pub(crate) fn new(
        main_thread_id: std::thread::ThreadId,
        timecode_status: Arc<Mutex<timecode_input::TimecodeStatus>>,
        output_role: OutputRole,
    ) -> Self {
        let remote_server = remote::RemoteServer::new();
        let remote_status = remote_server.status_handle();
        let capture = capture::CaptureService::default();
        let capture_status = capture.status_handle();
        let capture_audio = capture.audio_handle();
        Self {
            output_role,
            output_freeze: std::sync::RwLock::new(false),
            idle_program_audio: AtomicBool::new(false),
            main_thread_id,
            active_workspace: Arc::new(Mutex::new(None)),
            workspace_io: Arc::new(Mutex::new(WorkspaceIoState::default())),
            settings_operations: Mutex::new(()),
            settings_revision: Mutex::new(SettingsRevisionState::default()),
            native_automation: Mutex::new(native_automation::NativeAutomationScheduler::default()),
            library_search: Mutex::new(library_search::LibrarySearchIndex::default()),
            output_content: Mutex::new(HashMap::new()),
            output_revisions: Mutex::new(HashMap::new()),
            output_snapshot: Mutex::new(OutputSnapshotState::default()),
            rss_output: Mutex::new(rss_output::OutputRssCache::default()),
            rss_service: rss_output::OutputRssService::new(),
            local_text: local_text::LocalTextService::default(),
            linked_services: crate::linked_services::LinkedServices::default(),
            output_playback: Mutex::new(output_playback::PlaybackRegistry::new()),
            native_timers: Mutex::new(native_timers::TimerRegistry::new()),
            native_messages: Mutex::new(native_messages::MessageRuntime::default()),
            native_props: Mutex::new(native_props::PropRuntime::default()),
            native_audio: Mutex::new(native_audio::NativeAudioRegistry::new()),
            native_soundtrack: Mutex::new(native_soundtrack::SoundtrackPlayer::default()),
            capture: Mutex::new(capture),
            capture_status,
            timecode_status,
            capture_audio,
            native_compositor: Mutex::new(native_compositor::NativeCompositor::new()),
            native_key_fill_pairs: Mutex::new(native_key_fill::DisplayPairs::default()),
            native_preview_compositor: Mutex::new(
                native_compositor::NativeCompositor::new_preview(),
            ),
            native_prerender_compositor: Mutex::new(native_compositor::NativeCompositor::new()),
            output_render_requested: AtomicU64::new(0),
            output_render_worker_active: AtomicBool::new(false),
            output_prerender_worker_active: AtomicBool::new(false),
            native_frame_cache: Mutex::new(HashMap::new()),
            native_prerendered_frames: Mutex::new(HashMap::new()),
            native_preview_request_sizes: Mutex::new(HashMap::new()),
            native_gpu_contexts: Mutex::new(Vec::new()),
            native_output_surfaces: Mutex::new(HashMap::new()),
            display_outputs: Mutex::new(HashMap::new()),
            windowed_outputs: Mutex::new(HashMap::new()),
            remote_server: Mutex::new(remote_server),
            network_link: Mutex::new(network_link::NetworkLinkService::new()),
            communication_inputs: Mutex::new(communication_input::Inputs::default()),
            remote_status,
            ndi: Mutex::new(ndi::NdiService::new()),
            ndi_ticker_stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Default)]
pub(crate) struct WorkspaceIoState {
    pub(crate) active_path: Option<PathBuf>,
    pub(crate) revisions: HashMap<PathBuf, u64>,
}

impl WorkspaceIoState {
    pub(crate) fn revision(&mut self, path: &Path) -> u64 {
        *self.revisions.entry(path.to_path_buf()).or_default()
    }

    pub(crate) fn require_revision(&mut self, path: &Path, expected: u64) -> Result<(), String> {
        self.require_active(path)?;
        let current = self.revision(path);
        if current == expected {
            Ok(())
        } else {
            Err(format!(
                "Workspace changed since revision {expected}; current revision is {current}. Reload before saving."
            ))
        }
    }

    pub(crate) fn advance_revision(&mut self, path: &Path) -> u64 {
        let revision = self.revisions.entry(path.to_path_buf()).or_default();
        *revision = revision.saturating_add(1);
        *revision
    }

    pub(crate) fn activate(&mut self, path: &Path) -> u64 {
        self.active_path = Some(path.to_path_buf());
        self.revision(path)
    }

    pub(crate) fn require_active(&self, path: &Path) -> Result<(), String> {
        if self.active_path.as_deref() == Some(path) {
            Ok(())
        } else {
            Err("The requested workspace is no longer active.".to_string())
        }
    }
}

#[derive(Default)]
pub(crate) struct SettingsRevisionState {
    pub(crate) current: u64,
}

impl SettingsRevisionState {
    pub(crate) fn require(&self, expected: u64) -> Result<(), String> {
        if expected == self.current {
            Ok(())
        } else {
            Err(format!(
                "Settings changed since revision {expected}; current revision is {}. Reload before saving.",
                self.current
            ))
        }
    }

    pub(crate) fn advance(&mut self) -> u64 {
        self.current = self.current.saturating_add(1);
        self.current
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NativeFrameCacheKey {
    pub(crate) source_screen_id: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Clone)]
pub(crate) struct CachedNativeFrame {
    pub(crate) revision: u64,
    pub(crate) png: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct CachedPrerenderedFrame {
    pub(crate) visual_key: String,
    pub(crate) frame: Option<Arc<native_compositor::NativeFrame>>,
    pub(crate) layered_frame: Option<Arc<native_compositor::NativeLayeredFrame>>,
}

#[derive(Clone, Debug)]
pub(crate) struct OutputSnapshotCache {
    pub(crate) revision: u64,
    pub(crate) hash: String,
    pub(crate) snapshot: Value,
    pub(crate) dirty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutputSnapshotProtocol {
    pub(crate) session_id: String,
    pub(crate) client_sequence: u64,
    pub(crate) fingerprint: String,
    pub(crate) retired_session_ids: HashSet<String>,
}

#[derive(Default)]
pub(crate) struct OutputSnapshotState {
    pub(crate) cache: Option<OutputSnapshotCache>,
    pub(crate) revision: u64,
    pub(crate) protocol: Option<OutputSnapshotProtocol>,
}

impl OutputSnapshotState {
    pub(crate) fn advance_revision(&mut self) -> u64 {
        self.revision = self.revision.saturating_add(1).max(1);
        self.revision
    }

    pub(crate) fn invalidate_cache(&mut self) {
        self.cache = None;
    }

    pub(crate) fn merge_patch(
        &self,
        base_revision: u64,
        patch: &Value,
    ) -> Result<Option<Value>, String> {
        let Some(cached) = self
            .cache
            .as_ref()
            .filter(|cached| cached.revision == base_revision)
        else {
            return Ok(None);
        };
        output_scene::merge_output_scene_patch(&cached.snapshot, patch).map(Some)
    }

    pub(crate) fn update_base_matches(&self, base_revision: Option<u64>) -> bool {
        self.cache
            .as_ref()
            .is_none_or(|cached| base_revision == Some(cached.revision))
    }
}

pub(crate) fn lock_output_snapshot_state(
    coordinator: &Mutex<OutputSnapshotState>,
) -> Result<MutexGuard<'_, OutputSnapshotState>, String> {
    coordinator
        .lock()
        .map_err(|_| "Output snapshot state is unavailable.".to_string())
}

pub(crate) fn cached_output_hash_matches(cached: &OutputSnapshotCache, hash: &str) -> bool {
    !cached.dirty && cached.hash == hash
}

#[derive(Clone)]
pub(crate) struct WindowedOutput {
    pub(crate) label: String,
    pub(crate) source_screen_id: String,
    pub(crate) window_label: String,
    pub(crate) target_id: Option<String>,
    pub(crate) bounds: Option<DisplayBounds>,
    pub(crate) active: bool,
}

#[derive(Default)]
pub(crate) struct HiddenDisplayOutputs {
    pub(crate) windows: Vec<Window>,
    pub(crate) persisted: bool,
}

impl HiddenDisplayOutputs {
    pub(crate) fn persisted(&mut self) {
        self.persisted = true;
    }
}

impl Drop for HiddenDisplayOutputs {
    fn drop(&mut self) {
        if !self.persisted {
            for window in &self.windows {
                let _ = window.show();
            }
        }
    }
}
