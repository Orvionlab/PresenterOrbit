#[path = "action_fields.rs"]
mod action_fields;
#[path = "assets.rs"]
mod assets;
pub(crate) use assets::transparency_background;
#[path = "audio_bin.rs"]
mod audio_bin;
pub use audio_bin::AudioBin;
#[path = "effects.rs"]
mod effects;
#[path = "gesture.rs"]
mod gesture;
#[path = "inspector.rs"]
mod inspector;
#[path = "listing.rs"]
mod listing;
#[path = "maintenance.rs"]
mod maintenance;
#[path = "markers.rs"]
mod markers;
#[path = "menu.rs"]
mod menu;
#[path = "preview.rs"]
mod preview;
#[path = "preview_timeline.rs"]
mod preview_timeline;
#[path = "routing.rs"]
mod routing;
#[path = "tree.rs"]
mod tree;
use super::model::{self, text, Playlist};
use crate::state::runtime::Runtime;
use gpui::{
    div, prelude::*, uniform_list, Context, Entity, ExternalPaths, Focusable, Render, SharedString,
    Subscription, Window,
};
use orbit_ui::controls::Dialog;
use orbit_ui::controls::{
    input_state, ActionButton, ButtonVariant, IconButton, InputState, PanelHeader, Select,
    SelectOption, TextInput,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
pub(crate) use tree::AssetDrag;

#[path = "preload.rs"]
mod preload;

pub struct MediaBin {
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    asset_preview_busy: bool,
    preview_playing: bool,
    preview_offset: f64,
    preview_epoch: std::time::Instant,
    backgrounds: Vec<Value>,
    graphics_press: Option<gesture::GraphicsPress>,
    /// Set from the first optimistic reorder until the host has answered the last one.
    reorder: Option<gesture::Reorder>,
    suppress_click: bool,
    card_bounds: std::rc::Rc<
        std::cell::RefCell<std::collections::HashMap<String, gpui::Bounds<gpui::Pixels>>>,
    >,
    grid_bounds: std::rc::Rc<std::cell::RefCell<Option<gpui::Bounds<gpui::Pixels>>>>,
    grid_scroll: gpui::UniformListScrollHandle,
    audio_card_bounds: std::rc::Rc<
        std::cell::RefCell<std::collections::HashMap<String, gpui::Bounds<gpui::Pixels>>>,
    >,
    audio_grid_bounds: std::rc::Rc<std::cell::RefCell<Option<gpui::Bounds<gpui::Pixels>>>>,
    audio_scroll: gpui::UniformListScrollHandle,
    thumbnail_status: String,
    maintenance_busy: bool,
    maintenance_error: String,
    marker_draft: Option<Value>,
    marker_baseline: Option<Value>,
    marker_status: String,
    marker_menu: Option<(Option<String>, gpui::Point<gpui::Pixels>)>,
    marker_editor: Option<(String, markers::MarkerEditor)>,
    selected_marker: Option<String>,
    marker_clipboard: Option<Value>,
    inspector_open: bool,
    inspector_tab: usize,
    routing_open: bool,
    routing_sliders: std::collections::HashMap<String, routing::GainSlider>,
    inspector_transition: Option<Entity<crate::transitions::TransitionDefaults>>,
    preview_position: f64,
    preview_trim_drag: Option<preview_timeline::TrimDrag>,
    details: std::collections::HashMap<String, assets::AssetDetails>,
    preparation: preload::Preparation,
    grid_columns: usize,
    grid_size: Entity<orbit_ui::controls::SliderState>,
    filter_input: Option<Entity<InputState>>,
    filter_query: String,
    visible: Vec<usize>,
    transitions: Entity<crate::transitions::TransitionDefaults>,
    library_rename: Option<tree::LibraryRename>,
    library_menu: Option<(gpui::Point<gpui::Pixels>, bool)>,
    library_menu_id: Option<String>,
    library_drop: Option<(String, String)>,
    transport_draft: Option<Value>,
    transport_rate_slider: Option<(Entity<orbit_ui::controls::SliderState>, Subscription)>,
    transport_baseline: Option<Value>,
    menu_position: Option<gpui::Point<gpui::Pixels>>,
    clipboard: Option<Value>,
    graphics_drop: Option<(String, bool, String)>,
    effect_preview: Entity<orbit_ui::FrameView>,
    effect_preview_key: Value,
    rendered_media_preview: Option<effects::MediaPreviewFrame>,
    pending_thumbnail: Option<(String, Option<std::sync::Arc<gpui::RenderImage>>)>,
    maintenance: Option<(bool, Value)>,
    maintenance_request: u64,
    effect_menu: Option<(effects::EffectMenu, gpui::Point<gpui::Pixels>)>,
    effect_preset_dialog: Option<effects::PresetDialog>,
    effect_catalog: Vec<Value>,
    inputs: inspector::Inputs,
    preview: Arc<Mutex<orbit_host::PreviewAudio>>,
    preview_generation: Arc<AtomicU64>,
    runtime: Runtime,
    grid: bool,
    /// Which bin the open library menu, rename, or create action belongs to.
    audio: bool,
    /// The audio utility tab's playlists section shows only its header.
    pub audio_collapsed: bool,
    library: String,
    audio_library: String,
    /// The library whose delete the user has to confirm.
    confirm_delete: Option<String>,
    audio_rows: Vec<Value>,
    audio_visible: Vec<usize>,
    audio_filter_input: Option<Entity<InputState>>,
    audio_filter_query: String,
    audio_fade: Option<audio_bin::AudioFade>,
    workspace: String,
    /// The workspace and search paths the last automatic relink ran for.
    relink_key: Option<String>,
    source: Arc<Value>,
    media: Vec<Value>,
    libraries: Vec<Value>,
    rows: Vec<Value>,
    playlist: Playlist,
    selected: String,
    graphics_selected: HashSet<String>,
    graphics_anchor: Option<String>,
    graphics_selection_library: String,
    rename: Option<Entity<InputState>>,
    mutation_request: u64,
    busy: bool,
    video_inputs: Entity<super::VideoInputs>,
    _subscriptions: Vec<Subscription>,
}
pub struct SidebarChanged;
impl gpui::EventEmitter<SidebarChanged> for MediaBin {}

impl MediaBin {
    pub fn focus_filter(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = &self.filter_input {
            window.focus(&input.read(cx).focus_handle(cx), cx);
        }
    }
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let output_subscription = cx.observe(&runtime.output, |this, _, cx| {
            let backgrounds = this.runtime.output.read(cx).live["backgrounds"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if backgrounds != this.backgrounds {
                this.backgrounds = backgrounds;
                this.file_unassigned(cx);
                this.rebuild();
            }
            let countdown = this
                .runtime
                .output
                .read(cx)
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot["linkedTextContext"]["audioCountdown"].clone())
                .unwrap_or(Value::Null);
            let mut live = (*this.runtime.output.read(cx).live).clone();
            this.playlist.reconcile(
                &this.workspace,
                &this.media,
                &this.libraries,
                &mut live,
                &countdown,
            );
            if live != *this.runtime.output.read(cx).live {
                this.runtime.output.update(cx, |output, cx| {
                    output.edit_live(|current| *current = live);
                    cx.notify();
                });
            }
            cx.notify();
        });
        let subscription = cx.observe(&runtime.content, |this, _, cx| {
            this.refresh(cx);
        });
        cx.on_release(|this, cx| {
            for (_, details) in this.details.drain() {
                if let Some(image) = details.image {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        let preview = Arc::new(Mutex::new(orbit_host::PreviewAudio::default()));
        let meter = preview.clone();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            if this.upgrade().is_none() {
                cx.background_spawn(async move {
                    drop(meter);
                })
                .await;
                break;
            }
            if this
                .update(cx, |this, cx| {
                    this.graphics_tick(cx);
                    this.preview_tick(cx);
                })
                .is_err()
            {
                cx.background_spawn(async move {
                    drop(meter);
                })
                .await;
                break;
            }
        })
        .detach();
        let host = runtime.host.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { host.invoke("getMediaEffectCatalog", vec![]) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(catalog) => {
                        if let Some(catalog) = catalog.as_array() {
                            this.effect_catalog = catalog.clone();
                        }
                    }
                    Err(error) => this.runtime.status.update(cx, |status, cx| {
                        status.message = error;
                        cx.notify();
                    }),
                }
                cx.notify();
            });
        })
        .detach();
        let grid_size = orbit_ui::controls::range_state(0., 1., 0.01, 0.5, cx);
        Self::follow_audio_playlist(cx);
        let grid_subscription = cx.subscribe(
            &grid_size,
            |this, _, event: &orbit_ui::controls::SliderEvent, cx| {
                let orbit_ui::controls::SliderEvent::Change(value) = event else {
                    return;
                };
                this.grid_columns = (12. - 11. * value.end()).round() as usize;
                this.card_bounds.borrow_mut().clear();
                this.grid_scroll
                    .scroll_to_item(0, gpui::ScrollStrategy::Top);
                cx.notify();
            },
        );
        Self {
            asset_preview_busy: false,
            preview_playing: false,
            preview_offset: 0.,
            preview_epoch: std::time::Instant::now(),
            backgrounds: vec![],
            graphics_press: None,
            reorder: None,
            suppress_click: false,
            audio_card_bounds: Default::default(),
            audio_grid_bounds: Default::default(),
            audio_scroll: gpui::UniformListScrollHandle::new(),
            card_bounds: Default::default(),
            grid_bounds: Default::default(),
            grid_scroll: gpui::UniformListScrollHandle::new(),
            thumbnail_status: String::new(),
            maintenance_busy: false,
            maintenance_error: String::new(),
            marker_draft: None,
            marker_baseline: None,
            marker_status: String::new(),
            marker_menu: None,
            marker_editor: None,
            selected_marker: None,
            marker_clipboard: None,
            inspector_open: false,
            inspector_tab: 0,
            routing_open: false,
            routing_sliders: Default::default(),
            inspector_transition: None,
            preview_position: 0.,
            preview_trim_drag: None,
            details: Default::default(),
            preparation: Default::default(),
            grid_columns: 7,
            grid_size,
            filter_input: None,
            filter_query: String::new(),
            visible: vec![],
            transitions: cx.new(|cx| {
                crate::transitions::TransitionDefaults::new(
                    crate::transitions::Kind::Media,
                    runtime.clone(),
                    cx,
                )
            }),
            sidebar_width: 176.,
            sidebar_collapsed: false,
            library_rename: None,
            library_menu: None,
            library_drop: None,
            library_menu_id: None,
            transport_draft: None,
            transport_rate_slider: None,
            transport_baseline: None,
            menu_position: None,
            clipboard: None,
            graphics_drop: None,
            video_inputs: cx.new(|cx| super::VideoInputs::new(runtime.clone(), cx)),
            effect_preview: cx.new(|cx| orbit_ui::FrameView::new(|_, _| {}, cx)),
            effect_preview_key: Value::Null,
            rendered_media_preview: None,
            pending_thumbnail: None,
            maintenance: None,
            maintenance_request: 0,
            effect_menu: None,
            effect_preset_dialog: None,
            effect_catalog: vec![],
            inputs: Default::default(),
            preview,
            preview_generation: Arc::new(AtomicU64::new(0)),
            runtime,
            grid: true,
            audio: false,
            audio_collapsed: false,
            library: String::new(),
            audio_library: String::new(),
            confirm_delete: None,
            audio_rows: vec![],
            audio_visible: vec![],
            audio_filter_input: None,
            audio_filter_query: String::new(),
            workspace: String::new(),
            relink_key: None,
            source: Arc::new(Value::Null),
            media: vec![],
            libraries: vec![],
            rows: vec![],
            playlist: Playlist::default(),
            audio_fade: None,
            selected: String::new(),
            graphics_selected: HashSet::new(),
            graphics_anchor: None,
            graphics_selection_library: String::new(),
            rename: None,
            mutation_request: 0,
            busy: false,
            _subscriptions: vec![subscription, output_subscription, grid_subscription],
        }
    }
    fn library_for(&self, audio: bool) -> &String {
        if audio {
            &self.audio_library
        } else {
            &self.library
        }
    }
    fn library_for_mut(&mut self, audio: bool) -> &mut String {
        if audio {
            &mut self.audio_library
        } else {
            &mut self.library
        }
    }
    /// The library of the bin the open menu or pending action belongs to.
    fn context_library(&self) -> &String {
        self.library_for(self.audio)
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let (path, source) = {
            let content = self.runtime.content.read(cx);
            (content.workspace_path.clone(), content.workspace.clone())
        };
        if self.workspace != path {
            self.mutation_request += 1;
            self.busy = false;
            self.reorder = None;
            self.routing_open = false;
            self.routing_sliders.clear();
            self.inputs.clear();
            self.maintenance = None;
            self.maintenance_busy = false;
            self.maintenance_error.clear();
            self.inspector_transition = None;
            self.reset_marker_editor();
            self.marker_clipboard = None;
            self.library_rename = None;
            self.library_menu = None;
            self.confirm_delete = None;
            self.preview_position = 0.;
            self.preview_trim_drag = None;
            self.effect_preview_key = Value::Null;
            self.rendered_media_preview = None;
            self.pending_thumbnail = None;
            self.maintenance_request += 1;
            self.effect_menu = None;
            self.effect_preset_dialog = None;
            self.transport_draft = None;
            self.transport_rate_slider = None;
            self.transport_baseline = None;
            for (_, details) in self.details.drain() {
                if let Some(image) = details.image {
                    cx.drop_image(image, None);
                }
            }
            self.preparation.hovered.clear();
            self.workspace = path;
            self.library.clear();
            self.audio_library.clear();
            self.selected.clear();
            self.graphics_selected.clear();
            self.graphics_anchor = None;
            self.playlist.stop();
            self.audition(false, cx);
        }
        if Arc::ptr_eq(&self.source, &source) {
            return;
        }
        self.source = source;
        let (Some(media), Some(libraries)) = (
            self.source["media"].as_array(),
            self.source["mediaLibraries"].as_array(),
        ) else {
            if !self.workspace.is_empty() {
                self.runtime.status.update(cx, |status, cx| {
                    status.message = cx.global::<Translator>().t("media.invalidWorkspace");
                    cx.notify();
                });
            }
            self.media.clear();
            self.libraries.clear();
            self.rows.clear();
            self.visible.clear();
            self.audio_rows.clear();
            self.audio_visible.clear();
            cx.notify();
            return;
        };
        // An optimistic reorder is in flight: the host's lists become its baseline and the shown order stays.
        if let Some(reorder) = self.reorder.as_mut() {
            reorder.media = media.clone();
            reorder.libraries = libraries.clone();
            cx.notify();
            return;
        }
        self.media = media.clone();
        if let Some(draft) = self.transport_draft.as_ref() {
            if self
                .media
                .iter()
                .find(|item| item["id"] == self.selected)
                .is_some_and(|item| &item["mediaTransport"] == draft)
            {
                self.transport_baseline = Some(draft.clone());
            }
        }
        self.libraries = libraries.clone();
        self.file_unassigned(cx);
        self.rebuild();
        if self.library_rename.as_ref().is_some_and(|rename| {
            !self
                .libraries
                .iter()
                .any(|library| library["id"] == rename.id)
        }) {
            self.library_rename = None;
        }
        cx.notify();
    }
    fn select_asset(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.selected == id {
            return;
        }
        self.rename = None;
        self.routing_open = false;
        self.routing_sliders.clear();
        self.preview_trim_drag = None;
        self.audition(false, cx);
        self.preview_position = 0.;
        self.effect_preview_key = Value::Null;
        self.inputs.clear();
        self.transport_draft = None;
        self.transport_rate_slider = None;
        self.transport_baseline = None;
        self.reset_marker_editor();
        self.effect_menu = None;
        self.effect_preset_dialog = None;
        self.thumbnail_status.clear();
        self.selected = id.to_string();
        self.preparation.hovered = id.to_string();
        cx.notify();
    }
    fn rebuild(&mut self) {
        self.preparation.revision += 1;
        self.graphics_press = None;
        self.graphics_drop = None;
        self.card_bounds.borrow_mut().clear();
        self.audio_card_bounds.borrow_mut().clear();
        self.library = model::resolve_graphics_library(&self.libraries, &self.library);
        if self.graphics_selection_library != self.library {
            self.graphics_selected.clear();
            self.graphics_anchor = None;
            self.graphics_selection_library.clone_from(&self.library);
        }
        self.rows = model::selection(&self.media, &self.libraries, false, &self.library)
            .into_iter()
            .cloned()
            .collect();
        self.audio_library = model::resolve_audio_library(&self.libraries, &self.audio_library);
        self.audio_rows = model::selection(&self.media, &self.libraries, true, &self.audio_library)
            .into_iter()
            .cloned()
            .collect();
        self.filter_audio_rows();
        if !self.library.is_empty() {
            let backgrounds: Vec<_> = self
                .runtime_backgrounds()
                .into_iter()
                .filter(|background| background["libraryId"] == self.library)
                .collect();
            self.rows.extend(backgrounds.into_iter().map(|mut item| {
                item["type"] = json!("background");
                item["title"] = item["name"].clone();
                item
            }));
        }
        self.filter_rows();
    }

    fn select_graphics(&mut self, id: &str, modifiers: gpui::Modifiers) {
        let toggle = modifiers.control || modifiers.platform;
        if modifiers.shift {
            let start = self.graphics_anchor.as_ref().and_then(|anchor| {
                self.visible
                    .iter()
                    .position(|&index| self.rows[index]["id"] == *anchor)
            });
            let end = self
                .visible
                .iter()
                .position(|&index| self.rows[index]["id"] == id);
            if let Some((start, end)) = start.zip(end) {
                if !toggle {
                    self.graphics_selected.clear();
                }
                self.graphics_selected.extend(
                    self.visible[start.min(end)..=start.max(end)]
                        .iter()
                        .filter_map(|&index| self.rows[index]["id"].as_str().map(str::to_string)),
                );
                return;
            }
        }
        if toggle {
            if !self.graphics_selected.remove(id) {
                self.graphics_selected.insert(id.to_string());
            }
        } else {
            self.graphics_selected.clear();
            self.graphics_selected.insert(id.to_string());
        }
        self.graphics_anchor = Some(id.to_string());
    }

    fn clear_graphics_selection(&mut self, cx: &mut Context<Self>) {
        if !self.graphics_selected.is_empty() || self.graphics_anchor.is_some() {
            self.graphics_selected.clear();
            self.graphics_anchor = None;
            cx.notify();
        }
    }
    /// `attachMissingMediaAutoRelink`: with relinking on, missing media are searched for in the search paths
    /// whenever a workspace opens or the relink settings change.
    pub fn auto_relink(&mut self, cx: &mut Context<Self>) {
        self.refresh(cx);
        let general = self.runtime.output.read(cx).settings["generalSettings"].clone();
        let paths = general["mediaSearchPaths"].clone();
        let key = (general["relinkMissingMedia"] == json!(true)
            && !self.workspace.is_empty()
            && paths.as_array().is_some_and(|paths| !paths.is_empty()))
        .then(|| json!([self.workspace, paths]).to_string());
        if key == self.relink_key {
            return;
        }
        if key.is_some() && self.busy {
            return;
        }
        self.relink_key = key.clone();
        if key.is_some() {
            let media = json!(self.media);
            self.mutate("relinkMissingMediaFromSearchPaths", vec![media], cx);
        }
    }
    fn runtime_backgrounds(&self) -> Vec<Value> {
        self.backgrounds.clone()
    }
    fn mutate(&mut self, method: &'static str, args: Vec<Value>, cx: &mut Context<Self>) {
        if matches!(method, "saveMediaSettings" | "saveMediaLibraries") {
            if self.edits_blocked() || self.workspace.is_empty() {
                return;
            }
            if let Some(values) = args.first().and_then(Value::as_array) {
                self.reorder.get_or_insert_with(|| gesture::Reorder {
                    media: self.media.clone(),
                    libraries: self.libraries.clone(),
                });
                if method == "saveMediaSettings" {
                    self.media = values.clone();
                } else {
                    self.libraries = values.clone();
                }
                self.rebuild();
                self.flush_reorder(cx);
                cx.notify();
                return;
            }
        }
        self.mutate_saved(method, args, cx);
    }

    pub(super) fn edits_blocked(&self) -> bool {
        self.busy && self.reorder.is_none() && self.pending_thumbnail.is_none()
    }

    // The workspace lane owns revisions. Parse and merge the partial media response off the UI thread.
    pub(super) fn mutate_saved(
        &mut self,
        method: &'static str,
        mut args: Vec<Value>,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.workspace.is_empty() {
            return;
        }
        let copied_backgrounds = args
            .first()
            .filter(|mutation| method == "mutateMediaLibrary" && mutation["type"] == "duplicate")
            .map(|mutation| {
                self.backgrounds
                    .iter()
                    .filter(|background| background["libraryId"] == mutation["libraryId"])
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let previous_libraries = self.libraries.clone();
        self.busy = true;
        let thumbnail = matches!(method, "setVideoThumbnail" | "resetVideoThumbnail");
        let captured = self
            .rendered_media_preview
            .as_ref()
            .filter(|frame| {
                method == "setVideoThumbnail"
                    && frame.key[0] == self.workspace
                    && args
                        .get(1)
                        .is_some_and(|seconds| self.preview_playing || frame.key[2] == *seconds)
                    && self
                        .media
                        .iter()
                        .find(|asset| asset["id"] == self.selected)
                        .is_some_and(|asset| frame.key[1] == effects::visual_asset(asset))
            })
            .cloned();
        if let Some(frame) = &captured {
            args[1] = frame.key[2].clone();
        }
        if thumbnail {
            self.pending_thumbnail = Some((
                self.selected.clone(),
                captured.as_ref().map(|frame| frame.image.clone()),
            ));
            if captured.is_some() {
                self.thumbnail_status.clear();
            } else {
                self.thumbnail_status = cx.global::<Translator>().t("media.thumbnail.extracting");
            }
        }
        self.mutation_request += 1;
        let request = self.mutation_request;
        let owner = self.selected.clone();
        let request_audio = self.audio;
        let request_library = self.context_library().clone();
        let markers_save = self.marker_draft.as_ref().is_some_and(|draft| {
            args.first().and_then(Value::as_array).is_some_and(|media| {
                media
                    .iter()
                    .any(|item| item["id"] == owner && &item["markers"] == draft)
            })
        });
        let (runtime, path) = (self.runtime.clone(), self.workspace.clone());
        let (host, expected) = (runtime.host.clone(), path.clone());
        let light = matches!(method, "saveMediaSettings" | "saveMediaLibraries");
        let submitted = args.first().and_then(Value::as_array).cloned();
        let original = if let Some(reorder) = self.reorder.as_ref().filter(|_| light) {
            if method == "saveMediaLibraries" {
                reorder.libraries.clone()
            } else {
                reorder.media.clone()
            }
        } else if method == "saveWorkspaceContent" {
            self.source["mediaEffectPresets"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        } else if method == "saveMediaLibraries" {
            self.libraries.clone()
        } else {
            self.media.clone()
        };
        let original_media = (method == "saveWorkspaceContent").then(|| self.media.clone());
        let answer = runtime.lanes.workspace(&path, move |_revision| {
            let mut args = args;
            let revision = if method == "saveMediaSettings"
                || method == "saveMediaLibraries"
                || method == "saveWorkspaceContent"
            {
                let field = if method == "saveWorkspaceContent" {
                    "mediaEffectPresets"
                } else if method == "saveMediaSettings" {
                    "media"
                } else {
                    "mediaLibraries"
                };
                let snapshot = host.invoke("readWorkspaceSnapshot", vec![json!(expected)])?;
                let current = snapshot["workspace"][field]
                    .as_array()
                    .ok_or("Workspace media must be an array.")?;
                let requested = if method == "saveWorkspaceContent" {
                    args.first().and_then(|value| value[field].as_array())
                } else {
                    args.first().and_then(Value::as_array)
                }
                .ok_or("Media settings must be an array.")?;
                let merged = json!(model::merge_media(&original, requested, current)?);
                if let Some(original_media) = original_media {
                    args[0][field] = merged;
                    if let Some(media) = args[0].get("media") {
                        let requested =
                            media.as_array().ok_or("Media settings must be an array.")?;
                        let current = snapshot["workspace"]["media"]
                            .as_array()
                            .ok_or("Workspace media must be an array.")?;
                        args[0]["media"] =
                            json!(model::merge_media(&original_media, requested, current)?);
                    }
                } else {
                    args[0] = merged;
                }
                snapshot["revision"]
                    .as_u64()
                    .ok_or("Workspace revision is required.")?
            } else {
                host.invoke("readWorkspaceSnapshot", vec![json!(expected)])?["revision"]
                    .as_u64()
                    .ok_or("Workspace revision is required.")?
            };
            args.insert(0, json!(expected));
            args.push(json!(revision));
            let mut response = if let Some(frame) = captured {
                let pixels = frame
                    .image
                    .as_bytes(0)
                    .ok_or("Inspector preview has no pixels.")?
                    .to_vec();
                host.save_inspector_thumbnail(
                    &args,
                    &frame.key[1],
                    &orbit_host::native_compositor::NativeFrame {
                        width: frame.size.width as usize,
                        height: frame.size.height as usize,
                        pixels,
                        dynamic_media: false,
                    },
                )?
            } else {
                host.invoke(method, args)?
            };
            // Thumbnail writes already return the new media list.
            if thumbnail {
                response["workspace"] = json!({"media":response["workspace"]["media"].take()});
            } else if !light {
                let snapshot = host.invoke("readWorkspaceSnapshot", vec![json!(expected)])?;
                response["workspace"] = snapshot["workspace"].clone();
            }
            Ok(response)
        });
        cx.spawn(async move |this, cx| {
            let result = answer
                .await
                .map_err(|_| "The workspace queue stopped.".to_string())
                .and_then(|value| value);
            let parsed = cx
                .background_spawn(async move {
                    result.and_then(|response| {
                        let snapshot = if light || thumbnail {
                            None
                        } else {
                            Some(crate::collections::model::Snapshot::parse(
                                &response["workspace"],
                            )?)
                        };
                        Ok((response, snapshot))
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if request != this.mutation_request {
                    return;
                }
                this.busy = false;
                if this.workspace != path {
                    return;
                }
                let pending_thumbnail = thumbnail.then(|| this.pending_thumbnail.take()).flatten();
                match parsed {
                    Ok((response, snapshot)) => {
                        if let Some((id, Some(image))) = pending_thumbnail {
                            if let Some(details) = this.details.get_mut(&id) {
                                if let Some(asset) = response["workspace"]["media"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .find(|asset| asset["id"] == id)
                                {
                                    if !details.pending {
                                        details.key = assets::details_key(&this.workspace, asset);
                                    }
                                }
                                if let Some(previous) = details.image.replace(image.clone()) {
                                    if !Arc::ptr_eq(&previous, &image) {
                                        cx.drop_image(previous, None);
                                    }
                                }
                                details.transparent = false;
                            }
                        }
                        if method == "mutateMediaLibrary" {
                            let removed: Vec<_> = previous_libraries
                                .iter()
                                .filter(|library| {
                                    !response["workspace"]["mediaLibraries"]
                                        .as_array()
                                        .is_some_and(|libraries| {
                                            libraries
                                                .iter()
                                                .any(|current| current["id"] == library["id"])
                                        })
                                })
                                .map(|library| library["id"].clone())
                                .collect();
                            let mut backgrounds = this.backgrounds.clone();
                            backgrounds
                                .retain(|background| !removed.contains(&background["libraryId"]));
                            if let Some(id) = response["selectedMediaLibraryId"].as_str() {
                                for mut background in copied_backgrounds {
                                    background["id"] =
                                        json!(format!("background-{}", uuid::Uuid::new_v4()));
                                    background["libraryId"] = json!(id);
                                    backgrounds.push(background);
                                }
                            }
                            if backgrounds != this.backgrounds {
                                this.backgrounds = backgrounds.clone();
                                runtime.output.update(cx, |output, cx| {
                                    output
                                        .edit_live(|live| live["backgrounds"] = json!(backgrounds));
                                    cx.notify();
                                });
                            }
                        }
                        if let Some(reorder) = this.reorder.as_mut().filter(|_| thumbnail) {
                            let merged = response["workspace"]["media"]
                                .as_array()
                                .ok_or_else(|| "The thumbnail save returned no media.".to_string())
                                .and_then(|accepted| {
                                    let merged =
                                        model::merge_media(&reorder.media, &this.media, accepted)?;
                                    reorder.media.clone_from(accepted);
                                    Ok(merged)
                                });
                            match merged {
                                Ok(merged) => this.media = merged,
                                Err(error) => {
                                    this.reorder = None;
                                    runtime.status.update(cx, |status, cx| {
                                        status.message = error;
                                        cx.notify();
                                    });
                                }
                            }
                        }
                        if light && this.reorder.is_some() {
                            let (field, local) = if method == "saveMediaSettings" {
                                ("media", &mut this.media)
                            } else {
                                ("mediaLibraries", &mut this.libraries)
                            };
                            let merged = submitted
                                .as_deref()
                                .zip(response["workspace"][field].as_array())
                                .ok_or_else(|| "The media save returned no collection.".to_string())
                                .and_then(|(submitted, accepted)| {
                                    model::merge_media(submitted, local, accepted)
                                });
                            match merged {
                                Ok(merged) => *local = merged,
                                Err(error) => {
                                    this.reorder = None;
                                    runtime.status.update(cx, |status, cx| {
                                        status.message = error;
                                        cx.notify();
                                    });
                                }
                            }
                        }
                        if owner == this.selected
                            && matches!(method, "setVideoThumbnail" | "resetVideoThumbnail")
                        {
                            this.thumbnail_status.clear();
                        }

                        if let Some(draft) = this.marker_draft.as_ref() {
                            if response["workspace"]["media"]
                                .as_array()
                                .is_some_and(|media| {
                                    media.iter().any(|item| {
                                        item["id"] == this.selected && &item["markers"] == draft
                                    })
                                })
                            {
                                this.marker_baseline = Some(draft.clone());
                                this.marker_status = "media.markers.saved".into();
                            }
                        }
                        if let Some(id) = response["selectedMediaLibraryId"]
                            .as_str()
                            .filter(|_| this.library_for(request_audio) == &request_library)
                        {
                            *this.library_for_mut(request_audio) = id.to_string();
                        }
                        // Drops the bin's hold on the workspace so the merge below edits it in place.
                        this.source = Arc::new(Value::Null);
                        runtime.content.update(cx, |content, cx| {
                            match snapshot {
                                Some(snapshot) => content.apply_canonical(snapshot),
                                None => {
                                    let workspace = Arc::make_mut(&mut content.workspace);
                                    for (key, value) in
                                        response["workspace"].as_object().into_iter().flatten()
                                    {
                                        if key != "workspacePath" {
                                            workspace[key] = value.clone();
                                        }
                                    }
                                }
                            }
                            cx.notify();
                        });
                        this.refresh(cx);
                        this.flush_reorder(cx);
                        if method == "relinkMissingMediaFromSearchPaths" {
                            let count = response["relinked"].as_u64().unwrap_or(0);
                            if count > 0 {
                                let t = *cx.global::<Translator>();
                                runtime.status.update(cx, |status, cx| {
                                    status.message = t.format(
                                        "status.media.searchPathsRelinked",
                                        &[("count", &count.to_string())],
                                    );
                                    cx.notify();
                                });
                            }
                        }
                        if method == "cleanUnusedMedia" {
                            this.maintenance = Some((true, response.clone()));
                        } else if method == "relinkMissingMedia" || method == "forgetMissingMedia" {
                            this.scan_maintenance(false, cx);
                        }
                    }
                    Err(error) => {
                        if thumbnail {
                            this.flush_reorder(cx);
                        } else if this.reorder.take().is_some() {
                            this.source = Arc::new(Value::Null);
                            this.refresh(cx);
                        }
                        if owner == this.selected {
                            if markers_save {
                                this.marker_baseline = this
                                    .media
                                    .iter()
                                    .find(|item| item["id"] == owner)
                                    .map(|item| {
                                        item.get("markers").cloned().unwrap_or_else(|| json!([]))
                                    });
                                this.marker_status = "media.markers.failed".into();
                            }
                            if matches!(method, "setVideoThumbnail" | "resetVideoThumbnail") {
                                this.thumbnail_status = format!(
                                    "{} {}",
                                    cx.global::<Translator>().t("media.thumbnail.failed"),
                                    error
                                );
                            }
                        }
                        if method == "relinkMissingMedia" || method == "forgetMissingMedia" {
                            this.maintenance_error = error.clone();
                        }
                        let message = if method == "relinkMissingMediaFromSearchPaths" {
                            cx.global::<Translator>()
                                .format("status.media.searchPathsFailed", &[("error", &error)])
                        } else {
                            error
                        };
                        runtime.status.update(cx, |status, cx| {
                            status.message = message;
                            cx.notify();
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn ended(&mut self, event: Value, cx: &mut Context<Self>) {
        self.runtime.output.update(cx, |output, cx| {
            output.edit_live(|live| {
                self.playlist
                    .ended(&self.workspace, event, &self.media, &self.libraries, live)
            });
            cx.notify();
        });
    }
    fn audition(&mut self, play: bool, cx: &mut Context<Self>) {
        if !play {
            self.preview_playing = false;
        }
        let seconds = self.preview_position;
        let end = play
            .then(|| self.preview_range().map(|range| range.end))
            .flatten();
        let source = self
            .media
            .iter()
            .find(|item| item["id"] == self.selected)
            .cloned();
        let preview = self.preview.clone();
        let settings = model::inspector_audio_settings(
            &self.runtime.output.read(cx).settings["audioSettings"],
        );
        let guard = self.preview_generation.clone();
        let generation = guard.fetch_add(1, Ordering::AcqRel) + 1;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut preview = preview
                        .lock()
                        .map_err(|_| "Preview audio is unavailable.".to_string())?;
                    if guard.load(Ordering::Acquire) != generation {
                        return Ok(());
                    }
                    preview.stop();
                    if !play {
                        return Ok(());
                    }
                    let source = source.ok_or("Select media to preview.")?;
                    let path = source["sourcePath"]
                        .as_str()
                        .ok_or("Preview media has no source path.")?;
                    preview.play(
                        std::path::Path::new(path),
                        &settings,
                        source.get("audioRouting"),
                        seconds,
                        end,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.preview_generation.load(Ordering::Acquire) {
                    return;
                }
                match result {
                    Ok(()) => {}
                    Err(error) => this.runtime.status.update(cx, |status, cx| {
                        status.message = error;
                        cx.notify();
                    }),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn drop_files(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let mut media = self.media.clone();
        for path in &paths.0 {
            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                continue;
            };
            let kind = match extension.to_ascii_lowercase().as_str() {
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "svg" => "image",
                "mp4" | "mov" | "mkv" | "webm" | "avi" => "video",
                "wav" | "mp3" | "flac" | "m4a" | "ogg" | "aac" => "audio",
                _ => continue,
            };
            let Some(title) = path.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };
            let mut item = json!({"id":uuid::Uuid::new_v4().to_string(),"title":title,"type":kind,"sourcePath":path});
            // Media only land in the selected playlist: with none (or a folder or the root selected) the file is ignored.
            let target = if kind == "audio" {
                &self.audio_library
            } else {
                &self.library
            };
            if !self
                .libraries
                .iter()
                .any(|library| library["id"] == *target && library["type"] == "playlist")
            {
                continue;
            }
            item["libraryId"] = json!(target);
            media.push(item);
        }
        if media != self.media {
            self.mutate("saveMediaSettings", vec![json!(media)], cx);
        }
    }
    fn can_add_media(&self) -> bool {
        !self.workspace.is_empty()
            && self
                .libraries
                .iter()
                .any(|library| library["id"] == self.library && library["type"] == "playlist")
    }
    fn choose_media(&mut self, cx: &mut Context<Self>) {
        if self.busy || !self.can_add_media() {
            return;
        }
        let host = self.runtime.host.clone();
        let workspace = self.workspace.clone();
        let library = self.library.clone();
        self.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let paths = cx
                .background_spawn(async move { host.select_media_files() })
                .await;
            this.update(cx, |this, cx| {
                if this.workspace != workspace {
                    return;
                }
                this.busy = false;
                if this.library == library {
                    if let Some(paths) = paths {
                        this.drop_files(&ExternalPaths(paths.into()), cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    fn smart_library(&mut self, cx: &mut Context<Self>) {
        let host = self.runtime.host.clone();
        let workspace = self.workspace.clone();
        let audio = self.audio;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { host.invoke("selectMediaSmartPlaylistFolder", vec![]) },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.workspace != workspace {
                    return;
                }
                match result {
                    Ok(result) if result["canceled"] != true && result["library"].is_object() => {
                        let mut library = result["library"].clone();
                        library["mediaType"] = json!(if audio { "audio" } else { "graphics" });
                        library["parentId"] = json!("");
                        if let Some(id) = library["id"].as_str() {
                            *this.library_for_mut(audio) = id.to_string();
                        }
                        this.mutate("saveMediaLibrary", vec![library], cx);
                    }
                    Err(error) => this.runtime.status.update(cx, |status, cx| {
                        status.message = error;
                        cx.notify();
                    }),
                    _ => {}
                }
            });
        })
        .detach();
    }
    pub fn step_program(&mut self, direction: i32, cx: &mut Context<Self>) {
        let id = self.runtime.output.read(cx).live["backgroundId"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let rows = self
            .rows
            .iter()
            .filter(|v| v["type"] != "audio")
            .collect::<Vec<_>>();
        let current = rows.iter().position(|v| v["id"] == id);
        let next = match current {
            Some(i) => i as i32 + direction,
            None => {
                if direction > 0 {
                    0
                } else {
                    rows.len() as i32 - 1
                }
            }
        };
        if let Some(id) = usize::try_from(next)
            .ok()
            .and_then(|i| rows.get(i))
            .and_then(|v| v["id"].as_str())
            .map(str::to_string)
        {
            self.take(&id, cx);
        }
    }
    fn take(&mut self, id: &str, cx: &mut Context<Self>) {
        self.take_with(id, false, cx);
    }
    /// `attachAltClickVideoSuppression`: an Alt-click takes a video paused on its first frame, when the setting is on.
    fn take_with(&mut self, id: &str, alt_click: bool, cx: &mut Context<Self>) {
        self.playlist.stop();
        let Some(item) = self
            .media
            .iter()
            .chain(self.rows.iter())
            .find(|item| item["id"] == id)
            .cloned()
        else {
            return;
        };
        crate::automation::timeline::record_operator(
            &self.runtime,
            &[
                json!({"id":uuid::Uuid::new_v4().to_string(),"type":"media","targetId":id,"title":item["title"],"details":{"mediaBehavior":if item["type"]=="audio"{"audio"}else{"foreground"}}}),
            ],
            cx,
        );
        let suppress = alt_click
            && self.runtime.output.read(cx).settings["generalSettings"]
                ["altClickSuppressesVideoAutoStart"]
                == json!(true);
        self.runtime.output.update(cx, |output, cx| {
            let config = output
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot["screenConfig"].clone());
            output.edit_live(|live| {
                if live.get("screenConfig").is_none() {
                    if let Some(config) = config {
                        live["screenConfig"] = config;
                    }
                }
                if item["type"] == "audio" {
                    model::select_audio(live, id, &self.audio_library);
                    self.playlist
                        .follow(&self.workspace, &self.audio_library, id);
                } else {
                    let selection = live["backgroundSelectionId"].as_u64().unwrap_or(0) + 1;
                    live["backgroundId"] = json!(id);
                    live["backgroundSelectionId"] = json!(selection);
                    live["backgroundPlaybackState"] =
                        json!({"selectionId": selection, "status": "playing"});
                    crate::automation::runner::show_look_layer(live, "media");
                    live["transportPlaybackPaused"]["media"] = json!(false);
                    for (key, default) in [
                        ("backgroundBehavior", "foreground"),
                        ("backgroundScale", "fit"),
                        ("backgroundPlayback", "stop"),
                        ("backgroundTransition", "none"),
                        ("backgroundEndBehavior", "hold"),
                    ] {
                        live[key] = item.get(key).cloned().unwrap_or_else(|| json!(default));
                    }
                    if suppress && item["type"] == "video" {
                        model::pause_on_first_frame(live);
                    }
                }
            });
            cx.notify();
        });
    }
    /// The 16:9 preview height for the painted grid width; a uniform list needs one explicit row height.
    fn tile_preview_height(&self, cx: &Context<Self>) -> gpui::Pixels {
        let metrics = cx.theme().metrics;
        let width = self
            .grid_bounds
            .borrow()
            .map_or(gpui::px(160.), |bounds| bounds.size.width);
        let columns = self.grid_columns as f32;
        let tile = (width - metrics.space_sm * 2. - metrics.space_md * (columns - 1.)) / columns;
        tile * (9. / 16.)
    }
    fn rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        self.card_bounds.borrow_mut().clear();
        let columns = if self.grid { self.grid_columns } else { 1 };
        let selection: Arc<[String]> = self
            .visible
            .iter()
            .map(|&index| &self.rows[index])
            .filter(|item| matches!(item["type"].as_str(), Some("image" | "video")))
            .filter_map(|item| item["id"].as_str())
            .filter(|id| self.graphics_selected.contains(*id))
            .map(str::to_string)
            .collect();
        range
            .map(|row| {
                let mut line = div().flex().w_full().gap(cx.theme().metrics.space_xs);
                if self.grid {
                    line = line
                        .h(self.tile_preview_height(cx)
                            + gpui::px(22.)
                            + cx.theme().metrics.space_sm)
                        .gap(cx.theme().metrics.space_md)
                        .px(cx.theme().metrics.space_sm)
                        .pt(cx.theme().metrics.space_sm);
                }
                for index in row * columns..((row + 1) * columns).min(self.visible.len()) {
                    let item = self.rows[self.visible[index]].clone();
                    line = line.child(self.cell(item, Some(&selection), cx));
                }
                if self.grid {
                    // Keep the last row's tiles at column width, as the CSS grid does.
                    for _ in self.visible.len().min((row + 1) * columns)..(row + 1) * columns {
                        line = line.child(div().flex_1().min_w_0());
                    }
                }
                line.into_any_element()
            })
            .collect()
    }
    fn audio_item_rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        self.audio_card_bounds.borrow_mut().clear();
        let items: Vec<Value> = range
            .filter_map(|index| {
                self.audio_visible
                    .get(index)
                    .and_then(|index| self.audio_rows.get(*index))
                    .cloned()
            })
            .collect();
        items
            .into_iter()
            .map(|item| {
                div()
                    .flex()
                    .w_full()
                    .child(self.cell(item, None, cx))
                    .into_any_element()
            })
            .collect()
    }
    fn cell(
        &mut self,
        item: Value,
        selection: Option<&Arc<[String]>>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = *cx.theme();
        self.request_details(&item, cx);
        let audio = selection.is_none();
        let grid = self.grid && !audio;
        let id = text(&item, "id").to_string();
        let selected = if audio {
            self.selected == id
        } else {
            self.graphics_selected.contains(&id)
        };
        let live = self.runtime.output.read(cx).live.clone();
        let on_air = live["audioId"] == id || live["backgroundId"] == id;
        let title = text(&item, "title").to_string();
        let image = audio
            .then(|| item["thumbnailFile"].as_str())
            .flatten()
            .and_then(|file| {
                std::path::Path::new(&self.workspace)
                    .parent()
                    .map(|folder| folder.join("Media").join(file))
            });
        let has_picture = image.is_some()
            || self
                .details
                .get(&id)
                .is_some_and(|details| details.image.is_some());
        let pressing = self
            .graphics_press
            .as_ref()
            .is_some_and(|press| press.id == id);
        let dragging = pressing
            && self
                .graphics_press
                .as_ref()
                .is_some_and(|press| press.active);
        let marker = self
            .graphics_drop
            .as_ref()
            .filter(|(target, _, _)| target == &id)
            .map(|(_, after, _)| *after);
        let drag = tree::AssetDrag {
            id: id.clone(),
            title: title.clone(),
            workspace: self.workspace.clone(),
            slide_media: if let Some(selection) =
                selection.filter(|_| matches!(item["type"].as_str(), Some("image" | "video")))
            {
                if selected {
                    selection.clone()
                } else {
                    Arc::from([id.clone()])
                }
            } else {
                Arc::from([])
            },
        };
        let press_id = id.clone();
        let painted_id = id.clone();
        let startup_asset = crate::startup::first_page::Asset::Media(cx.entity_id(), id.clone());
        let startup_ready = !self.details.get(&id).is_some_and(|details| details.pending);
        let painted = if audio {
            self.audio_card_bounds.clone()
        } else {
            self.card_bounds.clone()
        };
        let drag_owner = cx.weak_entity();
        let hover_id = id.clone();
        let key_id = id.clone();
        let menu_id = id.clone();
        let selector = format!("media-item-{id}");
        let duration = card_duration(
            &item,
            self.details
                .get(&id)
                .and_then(|details| details.metadata["durationSeconds"].as_f64()),
        );
        let visible = if audio {
            &self.audio_visible
        } else {
            &self.visible
        };
        let rows = if audio { &self.audio_rows } else { &self.rows };
        let index = visible
            .iter()
            .position(|index| rows[*index]["id"] == id)
            .map_or(1, |index| index + 1);
        // `.thumb-preview` + `<footer>` of `MediaCard`: a 16:9 box with the picture, the clock chip, then the number and title.
        let table = (!grid && !audio).then(|| self.table_row(&item, index, cx));
        let preview = grid.then(|| {
            div()
                .debug_selector({
                    let id = id.clone();
                    move || format!("media-thumbnail-viewport-{id}")
                })
                .relative()
                .w_full()
                .h(self.tile_preview_height(cx))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .border_1()
                .border_color(if on_air {
                    theme.colors.live
                } else if selected {
                    theme.colors.selected
                } else {
                    theme.colors.border
                })
                .bg(theme.colors.border_strong)
                .when(
                    self.details
                        .get(&id)
                        .is_some_and(|details| details.transparent),
                    |frame| frame.child(assets::transparency_background(&id, cx)),
                )
                .when_some(
                    self.details
                        .get(&id)
                        .and_then(|details| details.image.clone()),
                    |frame, image| {
                        frame.child(
                            gpui::img(image)
                                .debug_selector({
                                    let id = id.clone();
                                    move || format!("media-thumbnail-{id}")
                                })
                                .absolute()
                                .inset_0()
                                .size_full()
                                .aspect_ratio(16. / 9.)
                                .object_fit(gpui::ObjectFit::Contain),
                        )
                    },
                )
                .when(!has_picture, |frame| {
                    frame.child(
                        orbit_ui::icons::icon(if item["type"] == "video" {
                            "video"
                        } else {
                            "polaroid"
                        })
                        .text_color(theme.colors.text_subtle),
                    )
                })
                .when_some(duration.clone(), |frame, duration| {
                    frame.child(
                        div()
                            .absolute()
                            .left(theme.metrics.space_snug)
                            .bottom(theme.metrics.space_snug)
                            .px(theme.metrics.space_xs)
                            .text_size(theme.metrics.font_xs)
                            .text_color(theme.colors.text_strong)
                            .bg(theme.colors.preview_matte_bg.opacity(0.75))
                            .child(duration),
                    )
                })
                .when(on_air, |frame| {
                    frame.child(
                        div()
                            .absolute()
                            .inset_0()
                            .border_2()
                            .border_color(theme.colors.live),
                    )
                })
                .when(selected && !on_air, |frame| {
                    frame.child(
                        div()
                            .absolute()
                            .inset_0()
                            .border_2()
                            .border_color(theme.colors.selected),
                    )
                })
        });
        div()
            .relative()
            .debug_selector(move || selector.clone())
            .id(SharedString::from(format!("media-item-{id}")))
            .flex()
            .flex_1()
            .min_w_0()
            .when(grid, |tile| tile.flex_col())
            .when(dragging, |el| el.opacity(0.72).cursor_grabbing())
            .when(!grid, |row| {
                row.items_center()
                    .h(if audio {
                        theme.metrics.row_height
                    } else {
                        theme.metrics.control_height + theme.metrics.space_sm
                    })
                    .px(theme.metrics.space_sm)
                    .bg(if selected {
                        theme.colors.selected
                    } else {
                        theme.colors.panel_bg
                    })
                    .hover(|row| row.bg(theme.colors.accent_soft))
                    .when(on_air, |row| {
                        row.border_l_2()
                            .border_color(theme.colors.live)
                            .when(audio, |row| row.text_color(theme.colors.live))
                    })
            })
            .when(audio, |row| {
                row.when_some(image, |row, path| {
                    row.child(
                        gpui::img(path)
                            .object_fit(gpui::ObjectFit::Contain)
                            .h(theme.metrics.row_height)
                            .max_w_full(),
                    )
                })
                .when_some(
                    self.details
                        .get(&id)
                        .and_then(|details| details.image.clone()),
                    |row, image| {
                        row.child(
                            gpui::img(image)
                                .object_fit(gpui::ObjectFit::Contain)
                                .h(theme.metrics.row_height)
                                .max_w_full(),
                        )
                    },
                )
                .when(
                    !has_picture && matches!(item["type"].as_str(), Some("audio" | "video")),
                    |row| {
                        row.child(
                            orbit_ui::icons::icon(if item["type"] == "audio" {
                                "music"
                            } else {
                                "video"
                            })
                            .text_color(theme.colors.text_muted),
                        )
                    },
                )
                .when_some(duration.clone().filter(|_| !audio), |row, duration| {
                    row.child(duration)
                })
            })
            .when(audio, |row| row.gap(theme.metrics.space_xs))
            .children(preview)
            .children(table)
            .when(grid || audio, |row| {
                row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(theme.metrics.space_xs)
                        .when(grid, |footer| footer.h(gpui::px(22.)).flex_none().min_w_0())
                        .when(!grid, |footer| footer.min_w_0())
                        .when(grid, |footer| {
                            footer.child(
                                div()
                                    .flex_none()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(index.to_string()),
                            )
                        })
                        .child(div().truncate().when(audio, |title| title.flex_1()).child(
                            if audio || grid {
                                title.clone()
                            } else {
                                format!("{index}  {title}")
                            },
                        )),
                )
            })
            .when_some(duration.filter(|_| audio), |row, duration| {
                row.child(div().text_color(theme.colors.text_muted).child(duration))
            })
            .role(gpui::Role::Button)
            .aria_label(title.clone())
            .aria_selected(selected)
            .tab_index(0)
            .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    if !audio {
                        this.select_graphics(&key_id, Default::default());
                    }
                    this.select_asset(&key_id, cx);
                    this.take(&key_id, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .when_some(marker, |tile, after| {
                // Centred in the gap to the neighbour, like the slide drop marker.
                let thickness = gpui::px(2.);
                let columns = self.grid_columns.max(1);
                let space = if (index - 1) % columns == 0 && !after || index % columns == 0 && after
                {
                    theme.metrics.space_sm
                } else {
                    theme.metrics.space_md
                };
                tile.child(
                    div()
                        .absolute()
                        .debug_selector(|| "media-item-drop-line".to_string())
                        .bg(theme.colors.accent)
                        .when(grid, |bar| {
                            bar.top_0()
                                .bottom_0()
                                .w(thickness)
                                .when(!after, |bar| bar.left(-(space + thickness) / 2.))
                                .when(after, |bar| bar.right(-(space + thickness) / 2.))
                        })
                        .when(!grid, |bar| {
                            let space = theme.metrics.space_xs;
                            bar.left_0()
                                .right_0()
                                .h(thickness)
                                .when(!after, |bar| bar.top(-(space + thickness) / 2.))
                                .when(after, |bar| bar.bottom(-(space + thickness) / 2.))
                        }),
                )
            })
            .child(
                gpui::canvas(
                    move |bounds, window, cx| {
                        if !bounds.intersect(&window.content_mask().bounds).is_empty() {
                            crate::startup::first_page::visible(
                                startup_asset.clone(),
                                startup_ready,
                                window,
                                cx,
                            );
                        }
                        painted.borrow_mut().insert(painted_id.clone(), bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    this.graphics_begin(press_id.clone(), event.position, false, audio, cx);
                }),
            )
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    this.audition(false, cx);
                    this.preview_position = 0.;
                    this.effect_preview_key = Value::Null;
                    this.select_asset(&menu_id, cx);
                    this.inputs.clear();
                    this.transport_draft = None;
                    this.transport_rate_slider = None;
                    this.transport_baseline = None;
                    this.thumbnail_status.clear();
                    this.reset_marker_editor();
                    this.menu_position = Some(event.position);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_drag(drag, move |drag, position, _, cx| {
                let _ = drag_owner.update(cx, |this, cx| {
                    this.graphics_begin(drag.id.clone(), position, true, audio, cx)
                });
                let label = if drag.slide_media.len() > 1 {
                    format!("{} ({})", drag.title, drag.slide_media.len())
                } else {
                    drag.title.clone()
                };
                cx.new(|_| tree::DragLabel(label))
            })
            .on_hover(cx.listener(move |this, hovered, _, cx| {
                if *hovered && !audio && this.preparation.hovered != hover_id {
                    this.preparation.hovered = hover_id.clone();
                    this.prepare_media(cx);
                }
            }))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                if this.suppress_click {
                    this.suppress_click = false;
                    return;
                }
                let modifiers = event.modifiers();
                if !audio {
                    this.select_graphics(&id, modifiers);
                    if modifiers.shift || modifiers.control || modifiers.platform {
                        cx.notify();
                        return;
                    }
                }
                this.inputs.clear();
                this.transport_draft = None;
                this.transport_rate_slider = None;
                this.transport_baseline = None;
                this.reset_marker_editor();
                this.audition(false, cx);
                this.preview_position = 0.;
                this.effect_preview_key = Value::Null;
                this.select_asset(&id, cx);
                this.take_with(&id, modifiers.alt, cx);
                cx.notify();
            }))
            .into_any_element()
    }
}

impl Render for MediaBin {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh(cx);
        self.prepare_media(cx);
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();

        let painted_grid = self.grid_bounds.clone();
        let body = uniform_list(
            "media-items",
            self.visible
                .len()
                .div_ceil(if self.grid { self.grid_columns } else { 1 }),
            cx.processor(|this, range, _, cx| this.rows(range, cx)),
        )
        .track_scroll(&self.grid_scroll)
        .flex_1()
        .min_h_0();
        let body = div()
            .id("media-grid-gesture")
            .relative()
            .when(
                self.graphics_press
                    .as_ref()
                    .is_some_and(|press| press.active),
                |el| el.cursor_grabbing(),
            )
            .flex_1()
            .min_h_0()
            .flex()
            .capture_any_mouse_down(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                let on_tile = this
                    .card_bounds
                    .borrow()
                    .values()
                    .any(|bounds| bounds.contains(&event.position));
                if !on_tile {
                    this.clear_graphics_selection(cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.clear_graphics_selection(cx)))
            .child(body)
            .child(
                gpui::canvas(
                    move |bounds, _, _| {
                        *painted_grid.borrow_mut() = Some(bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                this.graphics_move(event.position, cx)
            }))
            .on_drag_move::<tree::AssetDrag>(cx.listener(
                |this, event: &gpui::DragMoveEvent<tree::AssetDrag>, _, cx| {
                    this.graphics_move(event.event.position, cx)
                },
            ))
            .on_drop::<tree::AssetDrag>(cx.listener(|this, _, _, cx| this.graphics_finish(cx)))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.graphics_finish(cx)),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this
                        .graphics_press
                        .as_ref()
                        .is_some_and(|press| !press.audio)
                    {
                        this.graphics_cancel(cx);
                    }
                }),
            )
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.graphics_cancel(cx);
                    this.menu_position = None;
                    this.library_menu = None;
                    cx.stop_propagation();
                }
            }));
        let rename_dialog = self.rename.clone().map(|input| {
            let weak = cx.weak_entity();
            Dialog::new(
                "media-rename-dialog",
                t.t("common.newName"),
                t.t("common.cancel"),
                move |_, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.rename = None;
                        cx.notify();
                    });
                },
            )
            .child(TextInput::new("media-title", &input))
            .child(
                ActionButton::new("media-rename", t.t("common.rename"))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let title = input.read(cx).value().to_string();
                        if title.trim().is_empty() {
                            return;
                        }
                        if this
                            .backgrounds
                            .iter()
                            .any(|item| item["id"] == this.selected)
                        {
                            let mut backgrounds = this.backgrounds.clone();
                            for item in &mut backgrounds {
                                if item["id"] == this.selected {
                                    item["name"] = json!(title.trim());
                                }
                            }
                            this.rename = None;
                            this.runtime.output.update(cx, |output, cx| {
                                output.edit_live(|live| live["backgrounds"] = json!(backgrounds));
                                cx.notify();
                            });
                            return;
                        }
                        let mut media = this.media.clone();
                        if let Some(item) =
                            media.iter_mut().find(|item| item["id"] == this.selected)
                        {
                            item["title"] = json!(title.trim());
                        }
                        this.rename = None;
                        this.mutate("saveMediaSettings", vec![json!(media)], cx);
                    })),
            )
            .into_any_element()
        });
        let mut footer = div()
            .flex()
            .items_center()
            .flex_none()
            .h(theme.metrics.control_height)
            .gap(theme.metrics.space_sm)
            .px(theme.metrics.space_sm)
            .border_t_1()
            .border_color(theme.colors.border_soft)
            .bg(theme.colors.border_strong);
        let smart_playlist = self
            .libraries
            .iter()
            .any(|library| library["id"] == self.library && library["type"] == "smart");
        footer = footer
            .when(!smart_playlist, |footer| {
                footer.child(
                    IconButton::new(
                        "media-add-files",
                        "plus",
                        t.t(if self.can_add_media() {
                            "media.action.addFiles"
                        } else {
                            "media.import.selectPlaylist"
                        }),
                    )
                    .disabled(self.busy || !self.can_add_media())
                    .on_click(cx.listener(|this, _, _, cx| this.choose_media(cx))),
                )
            })
            .child(self.transitions.clone())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .justify_center()
                    .child(self.filter_control(window, cx)),
            );
        footer = footer.child(self.view_control(cx));
        footer = footer.child(
            div()
                .debug_selector(|| "media-size-slider".to_string())
                .flex_none()
                .w(theme.metrics.space_lg * 8.)
                .child(
                    orbit_ui::controls::RangeInput::new(&self.grid_size)
                        .disabled(!self.grid)
                        .label(t.t("media.size")),
                ),
        );
        let maintenance = self.maintenance_dialog(cx);
        let menu = self.library_menu(cx).or_else(|| self.menu(cx));
        let sidebar = (!self.sidebar_collapsed).then(|| self.sidebar(cx));
        let video_inputs = self.library == super::video_inputs::VIDEO_INPUT_LIBRARY;
        let crumb = if video_inputs {
            t.t("videoInputs.title")
        } else {
            self.libraries
                .iter()
                .find(|item| item["id"] == self.library)
                .map(|item| text(item, "title").to_string())
                .unwrap_or_else(|| t.t("media.graphicsFiles"))
        };
        let body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(theme.metrics.space_xs)
                    .h(theme.metrics.header_height)
                    .px(theme.metrics.space_sm)
                    .bg(theme.colors.border_strong)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(
                        IconButton::new(
                            "media-sidebar-toggle",
                            "columns-2",
                            t.t(if self.sidebar_collapsed {
                                "media.sidebar.show"
                            } else {
                                "media.sidebar.hide"
                            }),
                        )
                        .selected(!self.sidebar_collapsed)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sidebar_collapsed = !this.sidebar_collapsed;
                            cx.emit(SidebarChanged);
                            cx.notify();
                        })),
                    )
                    .child(orbit_ui::icon("polaroid").text_color(theme.colors.text_muted))
                    .child(t.t("media.graphics"))
                    .child("\u{203a}")
                    .child(div().truncate().child(crumb)),
            )
            .child(if video_inputs {
                self.video_inputs.update(cx, |panel, cx| panel.show(cx));
                self.video_inputs.clone().into_any_element()
            } else if self.rows.is_empty()
                && !model::has_playlist(&self.libraries, false)
                && !self.can_add_media()
            {
                div()
                    .id("media-no-playlist")
                    .debug_selector(|| "media-no-playlist".to_string())
                    .flex_1()
                    .child(orbit_ui::controls::EmptyState::new(t.t("media.noPlaylist")))
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .when(!self.grid, |el| el.child(self.table_header(cx)))
                    .when(
                        self.visible.is_empty() && !self.filter_query.trim().is_empty(),
                        |el| {
                            el.child(orbit_ui::controls::EmptyState::new(
                                t.t("media.filter.empty"),
                            ))
                        },
                    )
                    .child(body)
                    .into_any_element()
            })
            .child(footer);
        let inspector = self.inspector(window, cx);
        let content = div()
            .flex()
            .flex_1()
            .min_h_0()
            .children(sidebar)
            .child(body)
            .child(inspector);
        div()
            .id("media-bin")
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_sm)
            .when_some(maintenance, |element, dialog| element.child(dialog))
            .when_some(rename_dialog, |element, dialog| element.child(dialog))
            .when_some(self.delete_dialog(cx), |element, dialog| {
                element.child(dialog)
            })
            .when_some(menu, |element, menu| element.child(menu))
            .child(content)
            .on_drop::<ExternalPaths>(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.drop_files(paths, cx);
            }))
    }
}

/// Clock text for a video or audio card: the probed duration, else the saved duration when it is clock-shaped.
pub(super) fn card_duration(item: &Value, probed: Option<f64>) -> Option<String> {
    if !matches!(item["type"].as_str(), Some("audio" | "video")) {
        return None;
    }
    if let Some(seconds) = probed.filter(|seconds| seconds.is_finite()) {
        let total = seconds.max(0.) as u64;
        let (hours, minutes, seconds) = (total / 3600, total % 3600 / 60, total % 60);
        return Some(if hours > 0 {
            format!("{hours}:{minutes:02}:{seconds:02}")
        } else {
            format!("{minutes:02}:{seconds:02}")
        });
    }
    let saved = item["duration"].as_str()?;
    let parts: Vec<&str> = saved.split(':').collect();
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let clock = matches!(parts.len(), 2 | 3)
        && parts[0].len() <= 2
        && parts.iter().all(|part| digits(part))
        && parts[1..].iter().all(|part| part.len() == 2);
    clock.then(|| saved.to_string())
}
