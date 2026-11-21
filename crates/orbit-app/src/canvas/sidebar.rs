//! The editor sidebar's slide thumbnails and object rows, with drag reordering (`ThemeSidebar.tsx`,
//! `theme-object-reorder-utils.ts`) and the object context menu (`ThemeObjectContextMenu.tsx`).
use super::{content_document::DocumentKind, geometry, view::SlideCanvas};
use crate::output::preview::{Frames, Preview, Source};
use gpui::{
    canvas, div, prelude::*, px, AnyElement, Bounds, Context, Div, Entity, FocusHandle,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, SharedString, Stateful, Window,
};
use orbit_ui::{
    controls::{list_row, ContextMenu, IconButton, MenuEntry, MenuItem, PanelHeader},
    icon, ActiveTheme, FrameSize, Translator,
};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    hash::{Hash, Hasher},
    rc::Rc,
    sync::Arc,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Lane {
    Slides,
    Objects,
}

struct Drag {
    lane: Lane,
    source: usize,
    gap: usize,
    dragging: bool,
    start: Point<Pixels>,
}

type Tiles = Rc<RefCell<HashMap<(Lane, usize), Bounds<Pixels>>>>;
/// A thumbnail's content generation, its preview, and the slide's aspect ratio.
type Thumb = ((u64, u64), Entity<Preview>, f32);

#[derive(Default)]
pub struct Rows {
    frames: Frames,
    thumbs: HashMap<String, Thumb>,
    tiles: Tiles,
    drag: Option<Drag>,
    suppress_click: bool,
    menu: Option<(Point<Pixels>, usize)>,
    slide_menu: Option<(Point<Pixels>, String)>,
    pub(super) mask_order: Option<Vec<String>>,
    pub(super) mask_clipboard: Option<(String, Value)>,
    pub(super) mask_focus: Option<FocusHandle>,
    collapsed: HashSet<String>,
    objects_fraction: Option<f32>,
    pub(super) objects_resize: Option<(f32, f32, f32)>,
}
impl Rows {
    pub fn open_menu(&mut self, position: Point<Pixels>, index: usize) {
        self.menu = Some((position, index));
    }
    pub fn reset(&mut self) {
        self.drag = None;
        self.menu = None;
        self.slide_menu = None;
        self.mask_order = None;
        self.objects_resize = None;
    }
}

/// The drop gap (a display index, `count` for the end) to the final display index of the moved row.
fn drop_index(source: usize, gap: usize) -> usize {
    gap - usize::from(source < gap)
}

/// Moves the row at `from` to `to`.
pub(super) fn moved<T>(rows: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from >= rows.len() || to >= rows.len() || from == to {
        return false;
    }
    let row = rows.remove(from);
    rows.insert(to, row);
    true
}

/// Contiguous runs of slides that share a group, as `(group, first index, end index)`.
fn group_runs(groups: &[String]) -> Vec<(&str, usize, usize)> {
    let mut runs: Vec<(&str, usize, usize)> = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if run.0 == group => run.2 = index + 1,
            _ => runs.push((group, index, index + 1)),
        }
    }
    runs
}

fn type_icon(layer: &Value) -> &'static str {
    match (layer["type"].as_str(), layer["fillType"].as_str()) {
        (Some("Text"), _) => "letter-t",
        (_, Some("media")) => "polaroid",
        (_, Some("video-input" | "ndi")) => "video",
        (_, Some("web")) => "world",
        _ => "stack-front",
    }
}

impl SlideCanvas {
    pub(super) fn sidebar_panels(
        &mut self,
        upper: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *cx.theme();
        let fraction = self.editor.rows.objects_fraction.unwrap_or(0.28);
        let area = Rc::new(RefCell::new(Bounds::<Pixels>::default()));
        let measured = area.clone();
        let owner = cx.entity();
        let tracking = canvas(
            move |bounds, _, _| *measured.borrow_mut() = bounds,
            move |_, _, window, cx| {
                if owner.read(cx).editor.rows.objects_resize.is_none() {
                    return;
                }
                let moving = owner.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase.capture() {
                        moving.update(cx, |view, cx| {
                            if let Some((start, fraction, height)) = view.editor.rows.objects_resize
                            {
                                let minimum = (f32::from(
                                    cx.theme().metrics.header_height
                                        + cx.theme().metrics.row_height,
                                ) / height)
                                    .min(0.5);
                                view.editor.rows.objects_fraction = Some(
                                    (fraction + (start - f32::from(event.position.y)) / height)
                                        .clamp(minimum, 1. - minimum),
                                );
                                cx.notify();
                                cx.stop_propagation();
                            }
                        });
                    }
                });
                let releasing = owner.clone();
                window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase.capture() && event.button == MouseButton::Left {
                        releasing.update(cx, |view, cx| {
                            if view.editor.rows.objects_resize.take().is_some() {
                                cx.notify();
                                cx.stop_propagation();
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .size_full();
        div()
            .id("editor-sidebar-panels")
            .debug_selector(|| "editor-sidebar-panels".to_string())
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(div().flex_1().min_h_0().flex().flex_col().child(upper))
            .child(
                div()
                    .id("editor-objects-panel")
                    .debug_selector(|| "editor-objects-panel".to_string())
                    .h(gpui::relative(fraction))
                    .flex_none()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("editor-objects-divider")
                            .debug_selector(|| "editor-objects-divider".to_string())
                            .h(theme.metrics.space_snug)
                            .flex_none()
                            .border_t_1()
                            .border_color(theme.colors.border)
                            .cursor_row_resize()
                            .hover(|el| el.bg(theme.colors.control_bg_hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                                    let height = f32::from(area.borrow().size.height);
                                    if height > 0. {
                                        view.editor.rows.objects_resize =
                                            Some((event.position.y.into(), fraction, height));
                                        cx.notify();
                                        cx.stop_propagation();
                                    }
                                }),
                            ),
                    )
                    .child(
                        div()
                            .id("editor-objects-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(self.object_list(cx)),
                    ),
            )
            .child(tracking)
    }

    fn thumb_document(&self, owner: &str, slide: &str, cx: &Context<Self>) -> Option<Value> {
        let content = self.runtime.content.read(cx);
        match self.kind {
            DocumentKind::Theme => content.workspace["themes"]
                .as_array()?
                .iter()
                .find(|theme| theme["id"] == owner)?["slides"]
                .as_array()?
                .iter()
                .find(|candidate| candidate["id"] == slide)
                .cloned(),
            DocumentKind::Slide => {
                let raw = content.items.get(owner)?.value["slides"]
                    .as_array()?
                    .iter()
                    .find(|candidate| candidate["id"] == slide)?
                    .clone();
                let theme = orbit_host::authoring::slide_theme(&content.workspace, &raw);
                orbit_host::authoring::compose(&raw, theme).ok()
            }
            DocumentKind::Prop => content.workspace["props"]
                .as_array()?
                .iter()
                .find(|prop| prop["id"] == owner)?["slide"]
                .as_object()
                .map(|slide| Value::Object(slide.clone())),
            DocumentKind::Stage => content.workspace["stageLayouts"]
                .as_array()?
                .iter()
                .find(|layout| layout["id"] == owner)
                .map(|layout| layout["slide"].clone()),
            DocumentKind::Ccli => None,
            DocumentKind::Mask => {
                let mask = content.workspace["masks"]
                    .as_array()?
                    .iter()
                    .find(|mask| mask["id"] == owner)?;
                let mut slide = mask["slide"].as_object()?.clone();
                if let Some(size) = mask.get("size") {
                    slide.insert("width".into(), size["width"].clone());
                    slide.insert("height".into(), size["height"].clone());
                }
                Some(Value::Object(slide))
            }
        }
    }

    /// A still of one slide as last saved. It is rebuilt when the workspace content changes; an unchanged slide
    /// keeps its bitmap through the content hash in the key.
    fn thumb(
        &mut self,
        owner: &str,
        slide: &str,
        cx: &mut Context<Self>,
    ) -> Option<(Entity<Preview>, f32)> {
        // The test dispatcher never parks while native stills render on their own threads.
        if cfg!(test) {
            return None;
        }
        let content = self.runtime.content.read(cx);
        let generation = (content.load_generation, content.items_generation);
        let workspace = content.workspace_path.clone();
        let key = format!("{owner}:{slide}");
        let existing = self.editor.rows.thumbs.get(&key).cloned();
        if let Some((seen, preview, aspect)) = &existing {
            if *seen == generation {
                return Some((preview.clone(), *aspect));
            }
        }
        let document = self.thumb_document(owner, slide, cx)?;
        let slide_size = geometry::slide_size(&document);
        let size = FrameSize {
            width: slide_size.x.ceil() as u32,
            height: slide_size.y.ceil() as u32,
        };
        let mut hasher = DefaultHasher::new();
        document.to_string().hash(&mut hasher);
        let key_source = format!(
            "editor-thumb:{workspace}:{:?}:{key}:{:x}",
            self.kind,
            hasher.finish()
        );
        let slide = Arc::new(document);
        let source = match self.kind {
            DocumentKind::Stage => Source::StageLayout {
                key: key_source,
                slide,
            },
            DocumentKind::Mask => Source::Mask {
                key: key_source,
                slide,
            },
            _ => Source::Still {
                key: key_source,
                slide,
                transparent: self.kind == DocumentKind::Prop,
            },
        };
        let preview = match existing {
            Some((_, preview, _)) => preview,
            None => {
                let (host, frames) = (self.runtime.host.clone(), self.editor.rows.frames.clone());
                cx.new(|cx| {
                    let preview = Preview::new(host, frames, cx);
                    // The tile already sets the document aspect ratio. A second fit leaves
                    // gaps along the edges when the border or pixel rounding changes the bounds.
                    preview.frame.update(cx, |frame, _| frame.set_stretch(true));
                    preview
                })
            }
        };
        preview.update(cx, |preview, cx| {
            preview.set_source(Some(source), Some(size), 0, cx)
        });
        let aspect = slide_size.x / slide_size.y;
        self.editor
            .rows
            .thumbs
            .insert(key, (generation, preview.clone(), aspect));
        Some((preview, aspect))
    }

    fn press_row(&mut self, lane: Lane, source: usize, event: &MouseDownEvent) {
        let rows = &mut self.editor.rows;
        rows.suppress_click = false;
        rows.drag = Some(Drag {
            lane,
            source,
            gap: source,
            dragging: false,
            start: event.position,
        });
    }

    fn gap_from(&self, lane: Lane, pointer: Point<Pixels>) -> Option<usize> {
        crate::gap::gap_at(
            pointer,
            self.editor
                .rows
                .tiles
                .borrow()
                .iter()
                .filter(|((tile, _), _)| *tile == lane)
                .map(|((_, index), bounds)| (*index, *bounds)),
            true,
        )
    }

    fn rows_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let pointer = event.position;
        if event.pressed_button != Some(MouseButton::Left) {
            if self.editor.rows.drag.take().is_some() {
                cx.notify();
            }
            return;
        }
        let Some(lane) = self
            .editor
            .rows
            .drag
            .as_ref()
            .filter(|drag| {
                drag.dragging
                    || crate::gap::moved_far_enough(
                        drag.start,
                        pointer,
                        cx.theme().metrics.space_xs,
                    )
            })
            .map(|drag| drag.lane)
        else {
            return;
        };
        if let (Some(gap), Some(drag)) =
            (self.gap_from(lane, pointer), self.editor.rows.drag.as_mut())
        {
            if !drag.dragging || drag.gap != gap {
                drag.dragging = true;
                drag.gap = gap;
                cx.notify();
            }
        }
    }

    fn rows_released(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.editor.rows.drag.take() else {
            return;
        };
        if !drag.dragging {
            return;
        }
        self.editor.rows.suppress_click = true;
        let target = drop_index(drag.source, drag.gap);
        if target != drag.source && !self.saving {
            match drag.lane {
                Lane::Slides => self.move_slide(drag.source, target, cx),
                Lane::Objects => self.move_object(drag.source, target, cx),
            }
        }
        cx.notify();
    }

    /// Display positions run top to bottom, which is the reverse of the layer order.
    fn move_object(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let mut slide = (*self.slide).clone();
        let Some(layers) = slide["layers"].as_array_mut() else {
            return;
        };
        let last = layers.len().saturating_sub(1);
        let (from, to) = (last.saturating_sub(from), last.saturating_sub(to));
        let selected = self
            .selected
            .and_then(|index| layers.get(index))
            .map(|layer| layer["id"].clone());
        if !moved(layers, from, to) {
            return;
        }
        self.selected = selected.and_then(|id| layers.iter().position(|layer| layer["id"] == id));
        self.draft(slide, cx);
    }

    /// Wires the pointer tracking that the rows' drag needs onto a sidebar.
    pub(super) fn track_rows(
        &self,
        sidebar: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        sidebar
            .on_mouse_move(
                cx.listener(|view, event: &MouseMoveEvent, _, cx| view.rows_moved(event, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.rows_released(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.rows_released(cx)),
            )
    }

    fn row_marker(&self, lane: Lane, index: usize, count: usize) -> (bool, bool) {
        match self.editor.rows.drag.as_ref() {
            Some(drag)
                if drag.dragging
                    && drag.lane == lane
                    && drop_index(drag.source, drag.gap) != drag.source =>
            {
                (drag.gap == index, index + 1 == count && drag.gap == count)
            }
            _ => (false, false),
        }
    }

    fn row_drag_state(&self, lane: Lane, index: usize) -> bool {
        self.editor
            .rows
            .drag
            .as_ref()
            .is_some_and(|drag| drag.dragging && drag.lane == lane && drag.source == index)
    }

    fn marker(&self, cx: &Context<Self>, before: bool) -> impl IntoElement {
        let theme = cx.theme();
        let thickness = px(2.);
        div()
            .debug_selector(|| "canvas-reorder-marker".into())
            .absolute()
            .left_0()
            .right_0()
            .h(thickness)
            .when(before, |el| el.top_0())
            .when(!before, |el| el.bottom_0())
            .bg(theme.colors.accent)
    }

    /// Registers the row's bounds for the drop gap.
    fn tile_canvas(&self, lane: Lane, index: usize) -> impl IntoElement {
        let tiles = self.editor.rows.tiles.clone();
        canvas(
            move |area, _, _| {
                tiles.borrow_mut().insert((lane, index), area);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    /// The group of every slide, empty for a document without groups.
    fn slide_groups(&self, count: usize, cx: &Context<Self>) -> Vec<String> {
        if self.kind != DocumentKind::Slide {
            return vec![String::new(); count];
        }
        let slides = self
            .root_document(cx)
            .and_then(|item| item["slides"].as_array().cloned())
            .unwrap_or_default();
        (0..count)
            .map(|index| crate::cues::model::effective_group(&slides, index))
            .collect()
    }

    /// The group header's name and color from the live group settings.
    fn group_style(&self, group: &str, cx: &Context<Self>) -> (String, gpui::Rgba) {
        let theme = cx.theme();
        let setting = self
            .runtime
            .output
            .read(cx)
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot["groupSettings"][group].clone())
            .unwrap_or(Value::Null);
        let name = if group.is_empty() {
            cx.global::<Translator>().t("cue.menu.noGroup")
        } else {
            setting["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or(group)
                .to_string()
        };
        let color = setting["color"]
            .as_str()
            .and_then(crate::cues::parse_hex_color)
            .unwrap_or(theme.colors.cue_default);
        (name, color)
    }

    /// One thumbnail per slide of the open theme or presentation, under a collapsible header per group.
    pub(super) fn slide_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let slides = self.layout_choices(cx);
        let count = slides.len();
        // Collapsed rows draw no tile, so each frame registers the drop gaps again.
        self.editor
            .rows
            .tiles
            .borrow_mut()
            .retain(|(lane, _), _| *lane != Lane::Slides);
        let owner = self.theme_id.clone();
        let used: Vec<String> = slides
            .iter()
            .map(|slide| format!("{owner}:{}", slide.value))
            .collect();
        self.editor.rows.thumbs.retain(|key, _| used.contains(key));
        let groups = self.slide_groups(count, cx);
        let grouped = self.kind == DocumentKind::Slide;
        let mut list = div().flex().flex_col().w_full();
        let mut slides = slides.into_iter().enumerate().peekable();
        for (group, first, end) in group_runs(&groups) {
            let key = format!("{owner}:{group}");
            let collapsed = grouped && self.editor.rows.collapsed.contains(&key);
            if grouped {
                let (name, color) = self.group_style(group, cx);
                let on_color = readable_on(color, theme.colors.text, theme.colors.bg);
                let weak = cx.weak_entity();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("authoring-group-{first}")))
                        .flex()
                        .items_center()
                        .gap(theme.metrics.space_xs)
                        .w_full()
                        .h(theme.metrics.row_height)
                        .px(theme.metrics.space_xs)
                        .text_size(theme.metrics.font_sm)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .bg(color)
                        .text_color(on_color)
                        .cursor_pointer()
                        .hover(|style| style.opacity(0.85))
                        .child(icon(if collapsed {
                            "chevron-right"
                        } else {
                            "chevron-down"
                        }))
                        .child(div().min_w_0().flex_1().truncate().child(name))
                        .child(div().flex_none().child((end - first).to_string()))
                        .on_click(move |_, _, cx| {
                            weak.update(cx, |view, cx| {
                                let collapsed = &mut view.editor.rows.collapsed;
                                if !collapsed.remove(&key) {
                                    collapsed.insert(key.clone());
                                }
                                cx.notify();
                            })
                            .ok();
                        }),
                );
            }
            if collapsed {
                continue;
            }
            for (index, slide) in slides.by_ref().take(end - first) {
                list =
                    list.child(self.slide_tile(&owner, index, count, slide.value, slide.label, cx));
            }
        }
        list.id("authoring-slide-thumbs").into_any_element()
    }

    pub(super) fn slide_disabled(&self, id: &str, cx: &Context<Self>) -> bool {
        if self.kind != DocumentKind::Slide {
            return false;
        }
        let content = self.runtime.content.read(cx);
        self.editor
            .document_preview
            .as_ref()
            .or_else(|| {
                content
                    .items
                    .get(&self.theme_id)
                    .map(|item| item.value.as_ref())
            })
            .and_then(|item| item["slides"].as_array())
            .is_some_and(|slides| {
                slides
                    .iter()
                    .any(|slide| slide["id"] == id && slide["disabled"] == true)
            })
    }

    pub(super) fn document_slide_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let choices = self.document_choices(cx);
        let keys: HashSet<_> = choices
            .iter()
            .map(|(owner, slide, _)| format!("{owner}:{slide}"))
            .collect();
        self.editor.rows.thumbs.retain(|key, _| keys.contains(key));
        let mut list = div().flex().flex_col().w_full();
        let count = choices.len();
        if self.kind == DocumentKind::Mask {
            self.editor
                .rows
                .tiles
                .borrow_mut()
                .retain(|(lane, index), _| *lane != Lane::Slides || *index < count);
        }
        for (index, (owner, slide, title)) in choices.into_iter().enumerate() {
            list = list.child(self.slide_tile(&owner, index, count, slide, title.into(), cx));
        }
        list.into_any_element()
    }

    fn slide_tile(
        &mut self,
        owner: &str,
        index: usize,
        count: usize,
        id: String,
        label: SharedString,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = *cx.theme();
        let transparent = matches!(self.kind, DocumentKind::Prop | DocumentKind::Mask);
        let standalone = !matches!(self.kind, DocumentKind::Slide | DocumentKind::Theme);
        let mask = self.kind == DocumentKind::Mask;
        let draggable = !standalone || mask;
        let id_prefix = match self.kind {
            DocumentKind::Prop => "authoring-prop",
            DocumentKind::Stage => "authoring-stage",
            DocumentKind::Mask => "authoring-mask",
            _ => "authoring-slide",
        };
        let selected = owner == self.theme_id && id == self.slide_id;
        let disabled = self.slide_disabled(&id, cx);
        let (preview, aspect) = match self.thumb(owner, &id, cx) {
            Some((preview, aspect)) => (Some(preview), aspect),
            None => (None, 16. / 9.),
        };
        let (before, after) = self.row_marker(Lane::Slides, index, count);
        let dragging = self.row_drag_state(Lane::Slides, index);
        let weak = cx.weak_entity();
        let click_id = id.clone();
        let click_owner = owner.to_string();
        let menu_id = if mask { owner.to_string() } else { id.clone() };
        div()
            .id(SharedString::from(if standalone {
                format!("{id_prefix}-{owner}")
            } else {
                format!("authoring-slide-{id}")
            }))
            .relative()
            .flex()
            .flex_col()
            .gap(theme.metrics.space_2xs)
            .w_full()
            .p(theme.metrics.space_xs)
            .text_size(theme.metrics.font_sm)
            .when(standalone, |el| {
                let selector = format!("{id_prefix}-{owner}");
                el.debug_selector(move || selector.clone())
                    .role(gpui::Role::Button)
                    .aria_selected(selected)
                    .cursor_pointer()
            })
            .when(selected && !disabled, |el| {
                el.bg(theme.colors.selected)
                    .text_color(theme.colors.accent_text)
            })
            .when(!selected && !disabled, |el| {
                el.text_color(theme.colors.text)
                    .hover(|style| style.bg(theme.colors.accent_soft))
            })
            .when(disabled, |el| {
                el.bg(theme.colors.preview_matte_bg)
                    .text_color(theme.colors.disabled_text)
                    .border_l_2()
                    .border_color(if selected {
                        theme.colors.selected
                    } else {
                        theme.colors.border_soft
                    })
            })
            .when(dragging, |el| el.opacity(0.6).cursor_grabbing())
            .when(draggable, |el| {
                el.child(self.tile_canvas(Lane::Slides, index))
            })
            .child(
                div()
                    .relative()
                    .w_full()
                    .aspect_ratio(aspect)
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme.colors.border_soft)
                    .bg(theme.colors.preview_matte_bg)
                    .when(transparent, |el| {
                        el.child(crate::media::transparency_background(
                            &format!("{id_prefix}-{owner}"),
                            cx,
                        ))
                    })
                    .when(disabled, |el| el.opacity(0.35))
                    .children(preview.map(|preview| div().absolute().size_full().child(preview))),
            )
            .child(
                div()
                    .flex()
                    .gap(theme.metrics.space_2xs)
                    .child(div().flex_none().child(format!("{}.", index + 1)))
                    .child(div().min_w_0().truncate().child(label)),
            )
            .when(before, |el| el.child(self.marker(cx, true)))
            .when(after, |el| el.child(self.marker(cx, false)))
            .when(draggable, |el| {
                el.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, event: &MouseDownEvent, _, _| {
                        if !view.saving && view.editor.rows.mask_order.is_none() {
                            view.press_row(Lane::Slides, index, event)
                        }
                    }),
                )
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                    if view.saving || !matches!(view.kind, DocumentKind::Slide | DocumentKind::Mask)
                    {
                        return;
                    }
                    view.editor.rows.slide_menu = Some((event.position, menu_id.clone()));
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .when(draggable, |el| {
                el.on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|view, _, _, cx| view.rows_released(cx)),
                )
            })
            .on_click(move |_, window, cx| {
                weak.update(cx, |view, cx| {
                    if mask {
                        if let Some(focus) = &view.editor.rows.mask_focus {
                            window.focus(focus, cx);
                        }
                    }
                    if (draggable && std::mem::take(&mut view.editor.rows.suppress_click))
                        || (standalone && selected)
                    {
                        return;
                    }
                    view.open_kind(view.kind, &click_owner, &click_id, cx)
                })
                .ok();
            })
    }

    /// The right-click menu of a slide thumbnail: the structure actions of the slide it was opened on.
    pub(super) fn slide_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (position, id) = self.editor.rows.slide_menu.clone()?;
        let t = *cx.global::<Translator>();
        let mask = self.kind == DocumentKind::Mask;
        let item = |label: String, action: &'static str| {
            let (weak, id) = (cx.weak_entity(), id.clone());
            MenuItem::new(label).on_click(move |_, window, cx| {
                weak.update(cx, |view, cx| {
                    view.editor.rows.slide_menu = None;
                    if mask {
                        view.mask_action(&id, action, window, cx);
                    } else {
                        view.slide_structure(action, &id, cx);
                    }
                    cx.notify();
                })
                .ok();
            })
        };
        let entries: Vec<MenuEntry> = if mask {
            let mappings =
                crate::keys::KeyMappings::from_settings(&self.runtime.output.read(cx).settings);
            [
                ("common.copy", "copy"),
                ("common.paste", "paste"),
                ("common.duplicate", "duplicate"),
                ("common.delete", "delete"),
                ("common.rename", "rename"),
            ]
            .into_iter()
            .map(|(label, action)| {
                let mut entry = item(t.t(label), action)
                    .disabled(action == "paste" && self.mask_clipboard(cx).is_none());
                if let Some(shortcut) = mappings.shortcut(action) {
                    entry = entry.meta(shortcut.to_string());
                }
                entry.into()
            })
            .collect()
        } else {
            let slides = self.layout_choices(cx);
            let index = slides.iter().position(|slide| slide.value == id)?;
            vec![
                item(t.t("common.add"), "insert").into(),
                item(t.t("arrangements.moveEarlier"), "up")
                    .disabled(index == 0)
                    .into(),
                item(t.t("arrangements.moveLater"), "down")
                    .disabled(index + 1 >= slides.len())
                    .into(),
                MenuEntry::Separator,
                item(t.t("common.delete"), "delete").into(),
            ]
        };
        let weak = cx.weak_entity();
        Some(
            ContextMenu::new(
                "authoring-slide-menu",
                position,
                entries,
                move |_, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.editor.rows.slide_menu = None;
                        cx.notify();
                    })
                    .ok();
                },
            )
            .into_any_element(),
        )
    }

    pub(super) fn object_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let layers = self.slide["layers"].as_array().cloned().unwrap_or_default();
        let count = layers.len();
        let name_line_height = theme.metrics.font_sm + theme.metrics.space_xs;
        self.editor
            .rows
            .tiles
            .borrow_mut()
            .retain(|(lane, index), _| *lane != Lane::Objects || *index < count);
        let mut list = div().flex().flex_col();
        list = list.child(PanelHeader::new(t.t("theme.objects")));
        for (position, (index, layer)) in layers.into_iter().enumerate().rev().enumerate() {
            let selected = self.selected == Some(index);
            let required = self.required_ccli_layer(index);
            let (before, after) = self.row_marker(Lane::Objects, position, count);
            let dragging = self.row_drag_state(Lane::Objects, position);
            let hidden = layer["hidden"] == true;
            let name = layer["name"]
                .as_str()
                .or_else(|| layer["type"].as_str())
                .unwrap_or("")
                .to_string();
            let weak = cx.weak_entity();
            let mut row = list_row(
                SharedString::from(format!("theme-object-row-{index}")),
                selected,
                cx,
            )
            .relative()
            .h(theme
                .metrics
                .row_height
                .max(name_line_height + theme.metrics.space_2xs))
            .debug_selector(move || format!("canvas-object-{index}"))
            .when(hidden, |el| el.opacity(0.6))
            .when(dragging, |el| el.opacity(0.6).cursor_grabbing())
            .child(self.tile_canvas(Lane::Objects, position))
            .child(icon(type_icon(&layer)).text_color(theme.colors.text_muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .line_height(name_line_height)
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(name),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, _, _| {
                    if !view.saving {
                        view.press_row(Lane::Objects, position, event)
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.rows_released(cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                    if view.saving {
                        return;
                    }
                    view.selected = Some(index);
                    view.editor.rows.menu = Some((event.position, index));
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .on_click(move |_, _, cx| {
                weak.update(cx, |view, cx| {
                    if std::mem::take(&mut view.editor.rows.suppress_click) {
                        return;
                    }
                    if !view.saving && view.slide["layers"].get(index).is_some() {
                        view.selected = Some(index);
                        cx.notify();
                    }
                })
                .ok();
            })
            .when(before, |el| el.child(self.marker(cx, true)))
            .when(after, |el| el.child(self.marker(cx, false)));
            for (action, icon_name, label) in [
                (
                    "hide",
                    if hidden { "eye-off" } else { "eye" },
                    "theme.gpui.hidden",
                ),
                (
                    "lock",
                    if layer["locked"] == true {
                        "lock"
                    } else {
                        "lock-open"
                    },
                    "theme.gpui.locked",
                ),
            ] {
                let weak = cx.weak_entity();
                row = row.child(
                    IconButton::new(
                        SharedString::from(format!("theme-object-{index}-{action}")),
                        icon_name,
                        t.t(label),
                    )
                    .disabled(self.saving || (required && action == "hide"))
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| view.object_action(index, action, cx))
                            .ok();
                    }),
                );
            }
            list = list.child(row);
        }
        list.id("theme-object-rows").into_any_element()
    }

    /// The right-click menu of an object row.
    pub(super) fn object_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (position, index) = self.editor.rows.menu?;
        let layer = self.slide["layers"].get(index)?.clone();
        let t = *cx.global::<Translator>();
        let item = |label: String, action: &'static str| {
            let weak = cx.weak_entity();
            MenuItem::new(label).on_click(move |_, window: &mut Window, cx| {
                weak.update(cx, |view, cx| {
                    view.editor.rows.menu = None;
                    view.object_menu_action(index, action, window, cx)
                })
                .ok();
            })
        };
        let required = self.required_ccli_layer(index);
        let mappings =
            crate::keys::KeyMappings::from_settings(&self.runtime.output.read(cx).settings);
        let mut delete =
            item(t.t("common.delete"), "delete").disabled(required || layer["locked"] == true);
        if let Some(shortcut) = mappings.shortcut("delete") {
            delete = delete.meta(shortcut.to_string());
        }
        let entries: Vec<MenuEntry> = vec![
            item(t.t("common.rename"), "rename").into(),
            item(
                t.t(if layer["hidden"] == true {
                    "theme.object.unhide"
                } else {
                    "theme.object.hide"
                }),
                "hide",
            )
            .disabled(required)
            .into(),
            item(
                t.t(if layer["locked"] == true {
                    "theme.object.unlock"
                } else {
                    "theme.object.lock"
                }),
                "lock",
            )
            .into(),
            delete.into(),
        ];
        let weak = cx.weak_entity();
        Some(
            ContextMenu::new("theme-object-menu", position, entries, move |_, _, cx| {
                weak.update(cx, |view, cx| {
                    view.editor.rows.menu = None;
                    cx.notify();
                })
                .ok();
            })
            .into_any_element(),
        )
    }

    fn object_menu_action(
        &mut self,
        index: usize,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = Some(index);
        match action {
            "rename" => self.editor_action("rename-object", window, cx),
            "delete" => self.editor_action("delete-object", window, cx),
            _ => self.object_action(index, action, cx),
        }
        cx.notify();
    }
}

type Color = gpui::Rgba;

fn luminance(color: Color) -> f32 {
    let linear = |channel: f32| {
        if channel <= 0.03928 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

/// The candidate with the higher WCAG contrast against `background`.
fn readable_on(background: Color, first: Color, second: Color) -> Color {
    let contrast = |text: Color| {
        let (a, b) = (luminance(background), luminance(text));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    };
    if contrast(first) >= contrast(second) {
        first
    } else {
        second
    }
}
