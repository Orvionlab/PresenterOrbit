use super::{
    content_document::{open_document, DocumentKind},
    document::CanvasDocument,
    geometry::{self, Bounds, CanvasView, Gesture, Point},
    raster::Worker,
    surface::Surface,
    toolbar::CanvasTools,
};
use crate::state::runtime::Runtime;
use gpui::{
    div, prelude::*, App, Context, Entity, FocusHandle, Focusable, KeyDownEvent, MouseButton,
    Render, Subscription, Window,
};
use orbit_host::native_compositor::CanvasRenderKind;
use orbit_ui::controls::{ActionButton, ButtonVariant, EmptyState};
use orbit_ui::{ActiveTheme, FrameSize, FrameView, Translator};
use serde_json::Value;
use std::{collections::HashMap, rc::Rc, sync::Arc, time::Duration};

pub struct SlideCanvas {
    pub sidebar_width: f32,
    pub inspector_width: f32,
    pub(super) runtime: Runtime,
    pub(super) kind: DocumentKind,
    pub(super) prop_collection: Option<String>,
    pub(super) rich: Option<super::rich_panel::RichPanel>,
    pub(super) document: Option<Rc<dyn CanvasDocument>>,
    pub(super) slide: Arc<Value>,
    pub(super) view: CanvasView,
    pub(super) gesture: Gesture,
    pub(super) selected: Option<usize>,
    pub(super) frames: Vec<Entity<FrameView>>,
    pub(super) planes: bool,
    pub(super) plane_bounds: Option<Bounds>,
    pub(super) held: Option<Bounds>,
    pub(super) dependent: bool,
    pub(super) worker: Worker,
    pub(super) request: u64,
    pub(super) memory: HashMap<String, CanvasView>,
    pub(super) focus: FocusHandle,
    pub(super) origin: Point,
    pub(super) canvas_pointer: Option<gpui::Point<gpui::Pixels>>,
    pub(super) space: bool,
    pub(super) saving: bool,
    pub(super) pending_saves: usize,
    documents: super::saves::Documents,
    pub(super) save_id: u64,
    pub(super) active: bool,
    pub(super) focus_hook: Option<Subscription>,
    pub(super) status: String,
    pub(super) theme_id: String,
    pub(super) slide_id: String,
    pub(super) workspace: String,
    pub(super) tools: CanvasTools,
    pub(super) undo: Vec<Arc<Value>>,
    pub(super) redo: Vec<Arc<Value>>,
    pub(super) editor: super::inspector::Editor,
    pub(super) _subscriptions: Vec<Subscription>,
}
pub enum CanvasEvent {
    Close,
    Reflow { item: String, slide: String },
    SlideActions { item: String, slide: String },
}
impl gpui::EventEmitter<CanvasEvent> for SlideCanvas {}
impl SlideCanvas {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let frames = (0..4)
            .map(|_| {
                cx.new(|cx| {
                    let mut frame = FrameView::new(|_, _| {}, cx);
                    frame.set_stretch(true);
                    frame
                })
            })
            .collect();
        let subscriptions = vec![
            cx.observe(&runtime.content, |view, _, cx| view.canonical_changed(cx)),
            cx.observe(&runtime.ui, |_, _, cx| cx.notify()),
            cx.observe(&runtime.text_files, |view, _, cx| {
                if view.active {
                    view.request_raster(false, cx);
                    cx.notify();
                }
            }),
        ];
        cx.spawn(async move |view, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            if view
                .update(cx, |view, cx| {
                    if view.has_clock() && matches!(view.gesture, Gesture::Idle) && !view.saving {
                        view.request_raster(false, cx);
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
        Self {
            sidebar_width: 164.,
            inspector_width: 360.,
            runtime,
            kind: DocumentKind::Theme,
            prop_collection: None,
            rich: None,
            document: None,
            slide: Arc::new(Value::Null),
            view: CanvasView::new(Point::new(1920., 1080.)),
            gesture: Gesture::Idle,
            selected: None,
            frames,
            planes: false,
            plane_bounds: None,
            held: None,
            dependent: false,
            worker: Worker::new(),
            request: 0,
            memory: HashMap::new(),
            focus: cx.focus_handle(),
            origin: Point::default(),
            canvas_pointer: None,
            space: false,
            saving: false,
            pending_saves: 0,
            documents: Default::default(),
            save_id: 0,
            active: false,
            focus_hook: None,
            status: String::new(),
            theme_id: String::new(),
            slide_id: String::new(),
            workspace: String::new(),
            tools: CanvasTools::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            editor: super::inspector::Editor::default(),
            _subscriptions: subscriptions,
        }
    }
    pub fn open_selected(&mut self, cx: &mut Context<Self>) {
        self.active = true;
        let content = self.runtime.content.read(cx);
        let target = content
            .themes
            .iter()
            .find(|theme| theme.id == self.theme_id)
            .or_else(|| content.themes.first())
            .and_then(|theme| {
                theme
                    .slides
                    .iter()
                    .find(|(id, _)| *id == self.slide_id)
                    .or_else(|| theme.slides.first())
                    .map(|(id, _)| (theme.id.clone(), id.clone()))
            });
        if let Some((theme, slide)) = target {
            self.open(&theme, &slide, cx);
        } else {
            self.open_kind(DocumentKind::Theme, "", "", cx);
        }
    }
    pub(super) fn open(&mut self, theme: &str, slide: &str, cx: &mut Context<Self>) {
        self.open_kind(DocumentKind::Theme, theme, slide, cx);
    }
    pub fn open_kind(
        &mut self,
        kind: DocumentKind,
        theme: &str,
        slide: &str,
        cx: &mut Context<Self>,
    ) {
        self.active = true;
        self.flush_rich(cx);
        if self.saving
            || self.editor.document_preview.is_some()
            || self.editor.pending_operation.is_some()
        {
            self.editor.navigate = Some((kind, theme.to_string(), slide.to_string()));
            return;
        }
        if self.editor.base.is_some() {
            self.save((*self.slide).clone(), History::Edit, cx);
        }
        if kind != DocumentKind::Prop {
            self.prop_collection = None;
        } else if !theme.is_empty() {
            self.prop_collection = self.runtime.content.read(cx).workspace["props"]
                .as_array()
                .and_then(|props| props.iter().find(|prop| prop["id"] == theme))
                .map(|prop| prop["folder"].as_str().unwrap_or("").to_string());
        }
        if theme.is_empty() {
            self.kind = kind;
            self.theme_id.clear();
            self.slide_id.clear();
            self.workspace = self.runtime.content.read(cx).workspace_path.clone();
            self.document = None;
            self.status.clear();
            self.set_slide(Arc::new(Value::Null));
            self.rich = None;
            self.editor.reset();
            self.undo.clear();
            self.redo.clear();
            self.invalidate(cx);
            cx.notify();
            return;
        }
        if kind == DocumentKind::Slide && slide.is_empty() {
            self.kind = kind;
            self.theme_id = theme.into();
            self.slide_id.clear();
            self.workspace = self.runtime.content.read(cx).workspace_path.clone();
            self.document = None;
            self.set_slide(Arc::new(Value::Null));
            self.rich = None;
            self.editor.reset();
            self.undo.clear();
            self.redo.clear();
            self.invalidate(cx);
            cx.notify();
            return;
        }
        match open_document(&self.runtime, kind, theme, slide, cx) {
            Ok(document) => {
                let document = self.documents.open(document);
                if let Some(old) = &self.document {
                    self.memory.insert(old.key(), self.view);
                }
                self.kind = kind;
                self.rich = None;
                self.theme_id = theme.into();
                self.slide_id = slide.into();
                self.workspace = self.runtime.content.read(cx).workspace_path.clone();
                self.set_slide(document.slide());
                self.view = self
                    .memory
                    .get(&document.key())
                    .copied()
                    .unwrap_or_else(|| CanvasView::new(geometry::slide_size(&self.slide)));
                self.document = Some(document);
                self.editor.reset();
                self.selected = None;
                self.gesture = Gesture::Idle;
                self.undo.clear();
                self.redo.clear();
                self.status.clear();
                self.held = None;
                self.planes = false;
                self.invalidate(cx);
                self.request_raster(false, cx);
            }
            Err(error) => {
                self.kind = kind;
                self.theme_id = theme.into();
                self.slide_id = slide.into();
                self.workspace = self.runtime.content.read(cx).workspace_path.clone();
                self.document = None;
                self.set_slide(Arc::new(Value::Null));
                self.rich = None;
                self.editor.reset();
                self.selected = None;
                self.gesture = Gesture::Idle;
                self.held = None;
                self.undo.clear();
                self.redo.clear();
                self.invalidate(cx);
                self.status = error;
            }
        }
        cx.notify();
    }
    pub fn open_editor(&mut self, kind: DocumentKind, cx: &mut Context<Self>) {
        if kind == DocumentKind::Ccli {
            self.open_kind(
                kind,
                orbit_host::ccli_template::ID,
                orbit_host::ccli_template::ID,
                cx,
            );
            return;
        }
        if kind == DocumentKind::Theme {
            self.open_selected(cx);
            return;
        }
        let content = self.runtime.content.read(cx);
        let owner = if kind == DocumentKind::Slide {
            content
                .items
                .get(&self.runtime.ui.read(cx).selected_library_item_id)
                .map(|item| (*item.value).clone())
        } else {
            content.workspace[kind.collection()]
                .as_array()
                .and_then(|documents| {
                    documents
                        .iter()
                        .find(|document| self.kind == kind && document["id"] == self.theme_id)
                        .or_else(|| documents.first())
                })
                .cloned()
        };
        let (owner, slide) = owner
            .map(|owner| {
                (
                    owner["id"].as_str().unwrap_or("").to_string(),
                    if kind == DocumentKind::Slide {
                        owner["slides"]
                            .as_array()
                            .and_then(|slides| slides.first())
                            .and_then(|slide| slide["id"].as_str())
                            .unwrap_or("")
                            .to_string()
                    } else {
                        owner["slide"]["id"].as_str().unwrap_or("").to_string()
                    },
                )
            })
            .unwrap_or_default();
        if kind == DocumentKind::Prop && owner.is_empty() {
            let folder = self.selected_prop_collection(cx);
            self.open_prop_collection(&folder, cx);
        } else {
            self.open_kind(kind, &owner, &slide, cx);
        }
    }
    pub(super) fn prop_collections(&self, cx: &App) -> Vec<String> {
        let workspace = &self.runtime.content.read(cx).workspace;
        crate::automation::props::collections(
            workspace["props"].as_array().map_or(&[], Vec::as_slice),
            workspace["propFolders"]
                .as_array()
                .map_or(&[], Vec::as_slice),
        )
    }

    pub(super) fn selected_prop_collection(&self, cx: &App) -> String {
        let workspace = &self.runtime.content.read(cx).workspace;
        let folder = workspace["props"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|prop| self.kind == DocumentKind::Prop && prop["id"] == self.theme_id)
            .map(|prop| prop["folder"].as_str().unwrap_or(""))
            .or(self.prop_collection.as_deref())
            .unwrap_or("");
        crate::automation::props::active(&self.prop_collections(cx), folder)
    }

    pub fn open_prop_collection(&mut self, folder: &str, cx: &mut Context<Self>) {
        self.prop_collection = Some(folder.to_string());
        let target = self.runtime.content.read(cx).workspace["props"]
            .as_array()
            .and_then(|props| {
                props
                    .iter()
                    .find(|prop| prop["folder"].as_str().unwrap_or("") == folder)
            })
            .map(|prop| {
                prop["id"]
                    .as_str()
                    .zip(prop["slide"]["id"].as_str())
                    .map(|(owner, slide)| (owner.to_string(), slide.to_string()))
                    .ok_or("The prop has no document identity.")
            });
        let (owner, slide) = match target.transpose() {
            Ok(target) => target.unwrap_or_default(),
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        self.open_kind(DocumentKind::Prop, &owner, &slide, cx);
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn kind(&self) -> DocumentKind {
        self.kind
    }
    pub(super) fn report_background_save_error(&self, cx: &mut Context<Self>) {
        if !self.active && !self.status.is_empty() {
            self.runtime.status.update(cx, |status, cx| {
                status.message = self.status.clone();
                cx.notify();
            });
        }
    }
    pub fn suspend(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.flush_rich(cx);
        if !self.saving && self.editor.base.is_some() {
            self.save((*self.slide).clone(), History::Edit, cx);
        }
        self.editor.close_after_save = false;
        self.active = false;
        self.canvas_pointer = None;
        self.tools.close_controls();
        self.runtime
            .text_files
            .update(cx, |files, _| files.preview = Arc::new(Value::Null));
        self.cancel(window, cx);
        self.space = false;
        self.request += 1;
        for frame in &self.frames {
            frame.update(cx, |frame, cx| frame.release_after_present(window, cx));
        }
        true
    }
    fn canonical_changed(&mut self, cx: &mut Context<Self>) {
        if self.runtime.content.read(cx).workspace_path != self.workspace {
            self.prop_collection = None;
            self.document = None;
            self.rich = None;
            self.set_slide(Arc::new(Value::Null));
            self.editor.reset();
            self.editor.close_after_save = false;
            self.editor.navigate = None;
            self.editor.pending_operation = None;
            self.editor.after_slide_action = None;
            self.request += 1;
            self.save_id += 1;
            self.saving = false;
            self.gesture = Gesture::Idle;
            self.undo.clear();
            self.redo.clear();
            self.invalidate(cx);
            cx.notify();
            return;
        }
        if self.document.is_none()
            || self.saving
            || self.pending_saves > 0
            || self.editor.base.is_some()
        {
            if self.active
                && matches!(
                    self.kind,
                    DocumentKind::Prop | DocumentKind::Stage | DocumentKind::Mask
                )
            {
                cx.notify();
            }
            return;
        }
        match open_document(&self.runtime, self.kind, &self.theme_id, &self.slide_id, cx) {
            Ok(document) if document.slide().as_ref() != self.slide.as_ref() => {
                self.document = Some(document.clone());
                self.set_slide(document.slide());
                self.rich = None;
                self.selected = None;
                self.gesture = Gesture::Idle;
                self.held = None;
                self.undo.clear();
                self.redo.clear();
                self.request_raster(false, cx);
                cx.notify();
            }
            Err(error) => {
                self.document = None;
                self.request += 1;
                self.status = error;
                self.invalidate(cx);
                cx.notify();
            }
            Ok(document) => {
                self.document = Some(document);
                if self.active
                    && matches!(
                        self.kind,
                        DocumentKind::Prop | DocumentKind::Stage | DocumentKind::Mask
                    )
                {
                    cx.notify();
                }
            }
        }
    }
    /// Every slide assignment goes through here so `selected` never outlives the layer it names.
    pub(super) fn set_slide(&mut self, slide: Arc<Value>) {
        let layers = slide["layers"].as_array().map_or(0, Vec::len);
        if self.selected.is_some_and(|index| index >= layers) {
            self.selected = None;
        }
        self.slide = slide;
    }
    fn invalidate(&mut self, cx: &mut Context<Self>) {
        for frame in &self.frames {
            frame.update(cx, |frame, cx| frame.clear(cx));
        }
    }
    fn has_clock(&self) -> bool {
        orbit_host::native_compositor::content_has_dynamic_linked_text(&self.slide)
    }
    pub(super) fn layout(
        &mut self,
        size: Point,
        origin: Point,
        scale: f32,
        cx: &mut Context<Self>,
    ) {
        let changed = self.view.viewport != size || self.view.scale_factor != scale;
        self.origin = origin;
        if changed {
            self.view.viewport = size;
            self.view.scale_factor = scale;
            self.view.slide = geometry::slide_size(&self.slide);
            if self.view.fit {
                self.view.fit_zoom();
            } else {
                self.view.clamp_pan();
            }
            if !matches!(self.gesture, Gesture::Idle) {
                self.gesture = Gesture::Idle;
                self.held = None;
            }
            self.request_raster(false, cx);
            cx.notify();
        }
    }
    pub(super) fn render_kind(&self) -> CanvasRenderKind {
        match self.kind {
            DocumentKind::Stage => CanvasRenderKind::Stage,
            DocumentKind::Mask => CanvasRenderKind::Mask,
            DocumentKind::Ccli => CanvasRenderKind::Ccli,
            _ => CanvasRenderKind::Presentation,
        }
    }

    pub(super) fn request_raster(&mut self, split: bool, cx: &mut Context<Self>) {
        if !self.active
            || self.document.is_none()
            || self.view.viewport.x <= 0.
            || self.view.viewport.y <= 0.
        {
            return;
        }
        let size = geometry::slide_size(&self.slide);
        if self.view.slide != size {
            self.view.slide = size;
            if self.view.fit {
                self.view.fit_zoom();
            } else {
                self.view.clamp_pan();
            }
        }
        let (width, height) = self.view.raster();
        self.request += 1;
        let request = self.request;
        let mut slide = (*self.slide).clone();
        self.runtime
            .text_files
            .update(cx, |files, _| files.preview = self.slide.clone());
        crate::state::text_files::overlay(&mut slide, &self.runtime.text_files.read(cx).entries);
        let current = self.gesture.current().or(self.held);
        if let (Some(index), Some(bounds)) = (self.selected, current) {
            let layer = &slide["layers"][index];
            if bounds != geometry::layer_bounds(layer, self.view.slide) {
                let next = geometry::patch_bounds(layer, self.view.slide, bounds);
                slide["layers"][index] = next;
            }
        }
        let active = split.then_some(self.selected).flatten();
        let base = current.or_else(|| {
            self.selected
                .map(|index| geometry::layer_bounds(&self.slide["layers"][index], self.view.slide))
        });
        let answer = self.worker.render(
            Arc::new(slide),
            self.runtime.output.read(cx).snapshot.clone(),
            FrameSize { width, height },
            active,
            self.render_kind(),
        );
        cx.spawn(async move |view, cx| {
            let Ok(result) = answer.await else {
                return;
            };
            view.update(cx, |view, cx| {
                if request != view.request {
                    return;
                }
                match result {
                    Ok(raster) => {
                        view.dependent = raster.dependent;
                        view.planes = raster.images.len() == 3;
                        view.plane_bounds = base;
                        let offset = if view.planes { 1 } else { 0 };
                        for (index, image) in raster.images.into_iter().enumerate() {
                            view.frames[index + offset].update(cx, |frame, cx| {
                                frame.set_source(Some(FrameSize { width, height }), cx);
                                frame.set_image(image, cx);
                            });
                        }
                        if !view.planes {
                            for frame in &view.frames[1..] {
                                frame.update(cx, |frame, cx| frame.clear(cx));
                            }
                        }
                        if !view.planes && matches!(view.gesture, Gesture::Idle) {
                            view.held = None;
                        }
                        if view.dependent
                            && view
                                .gesture
                                .current()
                                .is_some_and(|bounds| Some(bounds) != base)
                        {
                            view.request_raster(true, cx);
                        }
                        cx.notify();
                    }
                    Err(error) => {
                        view.status = error;
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn down(
        &mut self,
        p: Point,
        button: MouseButton,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.focus.focus(window, cx);
        if self.editor.pen_tool && self.editor_pen_down(p, button, cx) {
            return false;
        }
        if button == MouseButton::Left && self.node_down(p, cx) {
            return true;
        }
        if self.document.is_none() || self.saving {
            return false;
        }
        if button == MouseButton::Middle || (button == MouseButton::Left && self.space) {
            self.view.fit = false;
            self.gesture = Gesture::Panning {
                start: p,
                pan: self.view.pan,
            };
            cx.notify();
            return true;
        }
        if button == MouseButton::Right {
            if let Some(index) = geometry::hit_layer(&self.slide, self.view, p) {
                self.selected = Some(index);
                self.editor.rows.open_menu(
                    gpui::point(gpui::px(p.x + self.origin.x), gpui::px(p.y + self.origin.y)),
                    index,
                );
                cx.notify();
            }
            return false;
        }
        if button != MouseButton::Left {
            return false;
        }
        let handle = self
            .selected
            .filter(|index| self.slide["layers"][*index]["hidden"] != true)
            .and_then(|index| {
                geometry::hit_handle(
                    geometry::layer_bounds(&self.slide["layers"][index], self.view.slide),
                    self.view,
                    p,
                )
            });
        let index = if handle.is_some() {
            self.selected
        } else {
            geometry::hit_layer(&self.slide, self.view, p)
        };
        self.selected = index;
        self.held = None;
        self.planes = false;
        if let Some(index) = index {
            let layer = &self.slide["layers"][index];
            let bounds = geometry::layer_bounds(layer, self.view.slide);
            if layer["locked"] == true {
                self.status = cx.global::<Translator>().t("status.theme.objectLocked");
                cx.notify();
                return false;
            }
            self.status.clear();
            self.gesture = if let Some(handle) = handle {
                Gesture::Resizing {
                    start: p,
                    bounds,
                    current: bounds,
                    handle,
                }
            } else {
                Gesture::PendingMove { start: p, bounds }
            };
            self.request_raster(true, cx);
        } else {
            self.gesture = Gesture::Idle;
            self.planes = false;
            self.request_raster(false, cx);
        }
        cx.notify();
        index.is_some()
    }
    pub(super) fn motion(&mut self, p: Point, cx: &mut Context<Self>) {
        if self.node_motion(p, cx) {
            return;
        }
        self.editor.pen_preview = Some(self.view.to_slide(p));
        let before = self.gesture.current();
        self.gesture.update(p, &mut self.view);
        if self.gesture.current() != before
            && (self.dependent || matches!(self.gesture, Gesture::Resizing { .. }))
        {
            self.request_raster(true, cx);
        }
        cx.notify();
    }
    pub(super) fn up(&mut self, p: Point, cx: &mut Context<Self>) {
        if self.editor.node_drag.is_some() {
            self.node_motion(p, cx);
            self.node_up(cx);
            return;
        }
        if matches!(self.gesture, Gesture::Idle) {
            return;
        }
        self.gesture.update(p, &mut self.view);
        let change = self.gesture.finish();
        if let (Some((bounds, _)), Some(index)) = (change, self.selected) {
            self.held = Some(bounds);
            let mut slide = (*self.slide).clone();
            slide["layers"][index] =
                geometry::patch_bounds(&slide["layers"][index], self.view.slide, bounds);
            self.save(slide, History::Edit, cx);
        } else {
            self.held = None;
            self.request_raster(false, cx);
            if self.editor.base.is_some() {
                self.draft((*self.slide).clone(), cx);
            }
        }
        cx.notify();
    }
    pub(super) fn save(&mut self, slide: Value, history: History, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else {
            return;
        };
        if self.saving {
            return;
        }
        let document = self.documents.open(document);
        self.document = Some(document.clone());
        self.pending_saves += 1;
        self.request += 1;
        self.save_id += 1;
        let request = self.save_id;
        let key = document.key();
        let workspace = self.workspace.clone();
        let previous = self
            .editor
            .base
            .take()
            .unwrap_or_else(|| self.slide.clone());
        self.editor.edit_nonce += 1;
        match history {
            History::Edit => {
                self.undo.push(previous.clone());
                self.redo.clear();
            }
            History::Undo => {
                self.undo.pop();
                self.redo.push(previous.clone());
            }
            History::Redo => {
                self.redo.pop();
                self.undo.push(previous.clone());
            }
        }
        if matches!(history, History::Undo | History::Redo) {
            self.editor.reset();
            self.rich = None;
            self.selected = None;
        }
        self.set_slide(Arc::new(slide.clone()));
        self.request_raster(false, cx);
        let task = document.commit(slide, cx);
        cx.spawn(async move |view, cx| {
            let result = task.await;
            view.update(cx, |view, cx| {
                view.pending_saves -= 1;
                let current = request == view.save_id
                    && view.workspace == workspace
                    && view
                        .document
                        .as_ref()
                        .is_some_and(|document| document.key() == key);
                match result {
                    Ok(document) if current => {
                        view.document = Some(view.documents.open(document.clone()));
                        if view.editor.base.is_none() && view.editor.document_preview.is_none() {
                            view.set_slide(document.slide());
                        }
                        view.status.clear();
                        view.request_raster(false, cx);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        if current {
                            if view.editor.base.is_none() {
                                view.editor.base = Some(previous);
                            }
                            if let Some(slide) = view.editor.document_before_slide.take() {
                                view.editor.document_preview = None;
                                view.editor.document_history = false;
                                view.editor.document_operations.clear();
                                view.set_slide(slide);
                                view.editor.scope.clear();
                            }
                            view.status = error.clone();
                            view.documents.keep_draft(&key, view.slide.clone());
                        }
                        if view.runtime.content.read(cx).workspace_path == workspace {
                            view.runtime.status.update(cx, |status, cx| {
                                status.message = error;
                                cx.notify();
                            });
                        }
                    }
                }
                if view.pending_saves == 0 && !view.saving {
                    view.flush_rich(cx);
                    if view.editor.base.is_none() {
                        view.finish_document_save(cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    pub(super) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.release_pointer();
        self.editor.rows.objects_resize = None;
        self.node_cancel(cx);
        self.gesture = Gesture::Idle;
        self.editor.pen.clear();
        self.editor.pen_tool = false;
        self.held = None;
        if !self.saving {
            if self.editor.base.is_some() {
                self.draft((*self.slide).clone(), cx);
            } else {
                self.request_raster(false, cx);
            }
        }
        cx.notify();
    }
    pub(super) fn zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        self.view.set_zoom(zoom);
        self.request_raster(false, cx);
        cx.notify();
    }
    pub(super) fn wheel(&mut self, d: Point, zoom: bool, cx: &mut Context<Self>) {
        if !matches!(self.gesture, Gesture::Idle) || self.saving {
            return;
        }
        if zoom {
            self.zoom(self.view.zoom * (d.y * 0.002).exp(), cx);
        } else {
            self.view.pan.x += d.x;
            self.view.pan.y += d.y;
            self.view.fit = false;
            self.view.clamp_pan();
            cx.notify();
        }
    }
    pub(super) fn key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if orbit_ui::controls::text_entry_focused(window) {
            return;
        }
        if self.mask_list_focused(window) && self.editor.dialog.is_none() {
            if let Some(shortcut) = crate::keys::shortcut_from_keystroke(&event.keystroke) {
                let mappings =
                    crate::keys::KeyMappings::from_settings(&self.runtime.output.read(cx).settings);
                if let Some(action) = ["copy", "paste", "duplicate", "delete"]
                    .into_iter()
                    .find(|action| mappings.shortcut(action) == Some(shortcut.as_str()))
                {
                    self.mask_action(&self.theme_id.clone(), action, window, cx);
                    cx.stop_propagation();
                    return;
                }
            }
        }
        if self.editor_key(event, window, cx) {
            return;
        }
        let modifier = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        match event.keystroke.key.as_str() {
            "space" => self.space = true,
            "escape" => {
                let idle = matches!(self.gesture, Gesture::Idle);
                self.cancel(window, cx);
                if idle {
                    self.selected = None;
                }
            }
            "z" if modifier && !self.saving && matches!(self.gesture, Gesture::Idle) => {
                if event.keystroke.modifiers.shift {
                    if let Some(slide) = self.redo.last() {
                        self.save((**slide).clone(), History::Redo, cx);
                    }
                } else if let Some(base) = self.editor.base.take() {
                    self.editor.edit_nonce += 1;
                    self.redo.push(self.slide.clone());
                    self.set_slide(base);
                    self.editor.reset();
                    self.rich = None;
                    self.selected = None;
                    self.request_raster(false, cx);
                } else if let Some(slide) = self.undo.last() {
                    self.save((**slide).clone(), History::Undo, cx);
                }
            }
            "y" if modifier && !self.saving && matches!(self.gesture, Gesture::Idle) => {
                if let Some(slide) = self.redo.last() {
                    self.save((**slide).clone(), History::Redo, cx);
                }
            }
            "delete" | "backspace" if !self.saving && matches!(self.gesture, Gesture::Idle) => {
                if let Some(index) = self
                    .selected
                    .filter(|&index| !self.required_ccli_layer(index))
                {
                    let mut slide = (*self.slide).clone();
                    if let Some(layers) = slide["layers"].as_array_mut() {
                        if layers
                            .get(index)
                            .is_some_and(|layer| layer["locked"] != true)
                        {
                            layers.remove(index);
                            self.selected = None;
                            self.save(slide, History::Edit, cx);
                        }
                    }
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
    pub fn can_edit(&self, action: &str) -> bool {
        if self.saving || !matches!(self.gesture, Gesture::Idle) {
            return false;
        }
        match action {
            "undo" => !self.undo.is_empty() || self.editor.base.is_some(),
            "redo" => !self.redo.is_empty(),
            "cut" | "delete" => self
                .selected
                .is_some_and(|index| !self.required_ccli_layer(index)),
            "copy" | "duplicate" => self.selected.is_some(),
            "paste" => self.document.is_some(),
            "deselectAll" => self.selected.is_some(),
            _ => false,
        }
    }
    pub(super) fn required_ccli_layer(&self, index: usize) -> bool {
        self.kind == DocumentKind::Ccli
            && self.slide["layers"][index]["id"] == orbit_host::ccli_template::TEXT_ID
    }
    pub fn edit_command(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.mask_list_focused(window)
            && matches!(action, "copy" | "paste" | "duplicate" | "delete")
        {
            self.mask_action(&self.theme_id.clone(), action, window, cx);
            return;
        }
        if !self.can_edit(action) {
            return;
        }
        match action {
            "undo" | "redo" | "delete" => {
                let key = match action {
                    "undo" => "ctrl-z",
                    "redo" => "ctrl-y",
                    _ => "delete",
                };
                self.key(
                    &KeyDownEvent {
                        keystroke: gpui::Keystroke::parse(key).expect("fixed edit shortcut"),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    window,
                    cx,
                );
            }
            "deselectAll" => {
                self.selected = None;
                cx.notify();
            }
            "copy" | "cut" | "duplicate" => {
                let Some(layer) = self
                    .selected
                    .and_then(|index| self.slide["layers"].as_array()?.get(index))
                    .cloned()
                else {
                    return;
                };
                if action == "duplicate" {
                    self.paste_layer(layer, cx);
                } else {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        serde_json::to_string(&layer).expect("layer JSON serializes"),
                    ));
                    if action == "cut" {
                        self.edit_command("delete", window, cx);
                    }
                }
            }
            "paste" => {
                if let Some(layer) = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .filter(|layer| {
                        layer.is_object() && layer["type"].is_string() && layer["id"].is_string()
                    })
                {
                    self.paste_layer(layer, cx);
                }
            }
            _ => {}
        }
    }
    fn mask_list_focused(&self, window: &Window) -> bool {
        self.kind == DocumentKind::Mask
            && self
                .editor
                .rows
                .mask_focus
                .as_ref()
                .is_some_and(|focus| focus.is_focused(window))
    }
    fn paste_layer(&mut self, mut layer: Value, cx: &mut Context<Self>) {
        let mut slide = (*self.slide).clone();
        layer["id"] = serde_json::json!(super::editing::id("layer"));
        if self.append_layer(&mut slide, layer).is_some() {
            self.draft(slide, cx);
        }
    }
    pub(super) fn plane_transform(&self) -> Option<(Bounds, Bounds)> {
        Some((self.plane_bounds?, self.gesture.current().or(self.held)?))
    }
}
pub(super) enum History {
    Edit,
    Undo,
    Redo,
}
impl Focusable for SlideCanvas {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for SlideCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_hook.is_none() {
            self._subscriptions
                .push(cx.observe_window_activation(window, |view, window, cx| {
                    if !window.is_window_active() {
                        view.space = false;
                        view.editor.rows.objects_resize = None;
                        if !matches!(view.gesture, Gesture::Idle) || view.editor.node_drag.is_some()
                        {
                            view.cancel(window, cx);
                        }
                    }
                }));
            self.focus_hook = Some(cx.on_focus_out(
                &self.focus.clone(),
                window,
                |view, _, window, cx| {
                    view.space = false;
                    view.editor.rows.objects_resize = None;
                    if !matches!(view.gesture, Gesture::Idle) || view.editor.node_drag.is_some() {
                        view.cancel(window, cx);
                    }
                },
            ));
        }
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let toolbar = self.canvas_toolbar(window, cx);
        let bounds = self
            .selected
            .and_then(|index| self.slide["layers"].as_array()?.get(index))
            .map(|layer| {
                self.gesture
                    .current()
                    .or(self.held)
                    .unwrap_or_else(|| geometry::layer_bounds(layer, self.view.slide))
            });
        let pointer = self
            .canvas_pointer
            .filter(|_| self.document.is_some())
            .map(|position| Point::new(position.x.into(), position.y.into()).minus(self.origin))
            .filter(|p| {
                p.x >= 0. && p.y >= 0. && p.x < self.view.viewport.x && p.y < self.view.viewport.y
            })
            .map(|p| self.view.to_slide(p));
        let readout = div()
            .flex()
            .flex_none()
            .gap(theme.metrics.space_xs)
            .children(
                [
                    ("X", pointer.map(|p| p.x)),
                    ("Y", pointer.map(|p| p.y)),
                    ("W", bounds.map(|b| b.width)),
                    ("H", bounds.map(|b| b.height)),
                ]
                .map(|(label, value)| {
                    let value = value.map(geometry::round);
                    div()
                        .debug_selector(move || format!("canvas-readout-{label}"))
                        .flex()
                        .flex_none()
                        .w(theme.metrics.font_sm * 5.)
                        .items_center()
                        .gap(theme.metrics.space_2xs)
                        .text_size(theme.metrics.font_sm)
                        .child(format!("{label}:"))
                        .child(
                            div()
                                .debug_selector(move || {
                                    let state = match value {
                                        Some(n) if n < 0. => "negative",
                                        Some(_) => "number",
                                        None => "empty",
                                    };
                                    format!("canvas-readout-{label}-{state}")
                                })
                                .flex_1()
                                .min_w_0()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(value.map_or_else(|| "----".into(), |n| n.to_string())),
                        )
                }),
            );
        let tools = self.editor_tools(cx);
        let surface = div()
            .relative()
            .debug_selector(|| "canvas-viewport".into())
            .size_full()
            .child(Surface::new(cx.entity()))
            .child(
                div()
                    .absolute()
                    .top(theme.metrics.row_height * 1.5)
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(tools),
            )
            .when(self.document.is_none(), |el| {
                el.child(
                    div()
                        .absolute()
                        .size_full()
                        .bg(theme.colors.canvas_bg)
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(theme.metrics.space_sm)
                        .p(theme.metrics.panel_padding)
                        .child(EmptyState::new(t.t("gpui.editor.empty")))
                        .when(self.kind != DocumentKind::Slide, |empty| {
                            empty.child(
                                ActionButton::new("canvas-create-document", t.t("common.new"))
                                    .variant(ButtonVariant::Primary)
                                    .disabled(
                                        self.saving
                                            || self
                                                .runtime
                                                .content
                                                .read(cx)
                                                .workspace_path
                                                .is_empty(),
                                    )
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.new_document(window, cx)
                                    })),
                            )
                        }),
                )
            });
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .track_focus(&self.focus)
            .tab_index(0)
            .id("slide-canvas")
            .aria_label(t.t(self.kind.label()))
            .on_key_down(cx.listener(Self::key))
            .on_key_up(cx.listener(|view, event: &gpui::KeyUpEvent, _, cx| {
                if event.keystroke.key == "space" {
                    view.space = false;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.document_sidebar(window, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(div().flex_1().min_h_0().child(surface))
                            .child(
                                div()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .gap(theme.metrics.space_sm)
                                    .px(theme.metrics.space_xs)
                                    .h(theme.metrics.control_height)
                                    .border_t_1()
                                    .border_color(theme.colors.border)
                                    .text_size(theme.metrics.font_sm)
                                    .child(readout)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .child(self.status.clone()),
                                    )
                                    .child(toolbar),
                            )
                            .children(self.rich_panel(window, cx)),
                    )
                    .child(self.editor_inspector(window, cx)),
            )
            .children(self.canvas_zoom_menu(cx))
            .children(self.prop_transition_dialog(window, cx))
            .children(self.editor_dialog(window, cx))
            .children(self.object_menu(cx))
            .children(self.slide_menu(cx))
    }
}
