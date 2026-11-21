//! The theme picker popover: recently picked theme layouts above a grid of every theme. Each thumbnail is a native
//! still of one theme slide; moving the pointer across it scrubs through the theme's slides, and a click assigns
//! the slide on show. Port of `src/renderer/react/ui/ThemePicker.tsx`.

use std::cell::{Cell, RefCell};
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, relative, App, Bounds, Context, Entity, FocusHandle, KeyDownEvent,
    MouseMoveEvent, Pixels, Render, Role, ScrollHandle, SharedString, Window,
};
use orbit_ui::controls::{NativeButton, Popover};
use orbit_ui::{ActiveTheme, FrameSize, Translator};

use crate::collections::model::Theme;
use crate::output::preview::{Frames, Preview, Source};
use crate::state::runtime::Runtime;

pub const COLUMNS: usize = 3;
const WIDTH: Pixels = px(396.);
const MAX_HEIGHT: Pixels = px(460.);

/// The slide under the pointer: `offset` across a thumbnail `width` wide, in `count` equal bands.
pub fn scrub_index(offset: f32, width: f32, count: usize) -> usize {
    if count == 0 || width <= 0. {
        return 0;
    }
    let fraction = (offset / width).clamp(0., 1.);
    ((fraction * count as f32) as usize).min(count - 1)
}

/// The theme layout assignments (`theme::slide`) that still exist, in the order given.
pub fn known_recents(recents: &[String], themes: &[Theme]) -> Vec<String> {
    recents
        .iter()
        .filter(|assignment| position(themes, assignment).is_some())
        .cloned()
        .collect()
}

/// `(theme index, slide index)` of an assignment.
fn position(themes: &[Theme], assignment: &str) -> Option<(usize, usize)> {
    let (theme_id, slide_id) = assignment.split_once("::")?;
    let theme = themes.iter().position(|theme| theme.id == theme_id)?;
    let slide = themes[theme]
        .slides
        .iter()
        .position(|(id, _)| id == slide_id)?;
    Some((theme, slide))
}

/// Where an arrow key moves the highlighted thumbnail in a grid of `len` thumbnails.
pub fn step_highlight(current: usize, key: &str, len: usize) -> usize {
    let last = len.saturating_sub(1);
    match key {
        "left" => current.saturating_sub(1),
        "right" => (current + 1).min(last),
        "up" => current.saturating_sub(COLUMNS),
        "down" if current + COLUMNS <= last => current + COLUMNS,
        // Down from a short last row ends on the last thumbnail, as a grid does.
        "down" if current / COLUMNS < last / COLUMNS => last,
        _ => current,
    }
}

#[derive(Clone, Copy)]
struct Thumb {
    theme: usize,
    slide: usize,
}

type Pick = Rc<dyn Fn(String, &mut Window, &mut App)>;

pub struct ThemePicker {
    runtime: Runtime,
    /// Its own bitmap cache and render lane, so thumbnails never queue behind the cue grid's stills.
    frames: Frames,
    id: SharedString,
    value: String,
    disabled: bool,
    open: bool,
    highlighted: usize,
    /// `(thumbnail, slide)` while the pointer scrubs a thumbnail.
    scrub: Option<(usize, usize)>,
    /// One preview per theme slide, created the first time it is shown and dropped when the popover closes.
    previews: HashMap<String, Entity<Preview>>,
    bounds: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>,
    scroll: ScrollHandle,
    focus: FocusHandle,
    on_pick: Option<Pick>,
}

impl ThemePicker {
    pub fn new(runtime: Runtime, id: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            runtime,
            frames: Frames::default(),
            id: id.into(),
            value: String::new(),
            disabled: false,
            open: false,
            highlighted: 0,
            scrub: None,
            previews: HashMap::new(),
            bounds: Rc::default(),
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            on_pick: None,
        }
    }

    pub fn sync(
        &mut self,
        value: &str,
        disabled: bool,
        on_pick: impl Fn(String, &mut Window, &mut App) + 'static,
    ) {
        self.value = value.to_string();
        self.disabled = disabled;
        self.on_pick = Some(Rc::new(on_pick));
    }

    fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        self.scrub = None;
        // A closed picker keeps no bitmaps, and a reopened one renders current theme content.
        self.previews.clear();
        self.bounds.borrow_mut().clear();
        if open {
            let cells = self.cells(cx);
            self.highlighted = cells
                .iter()
                .position(|cell| self.assignment(cell, cx).as_deref() == Some(&self.value))
                .unwrap_or_default();
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Recently picked layouts first, then every theme with slides.
    fn cells(&self, cx: &App) -> Vec<Thumb> {
        let content = self.runtime.content.read(cx);
        let mut cells: Vec<Thumb> = known_recents(&content.recent_themes, &content.themes)
            .iter()
            .filter_map(|assignment| position(&content.themes, assignment))
            .map(|(theme, slide)| Thumb { theme, slide })
            .collect();
        let current = position(&content.themes, &self.value);
        cells.extend(
            content
                .themes
                .iter()
                .enumerate()
                .filter(|(_, theme)| !theme.slides.is_empty())
                .map(|(index, _)| Thumb {
                    theme: index,
                    slide: current
                        .filter(|(theme, _)| *theme == index)
                        .map_or(0, |(_, slide)| slide),
                }),
        );
        cells
    }

    fn recent_count(&self, cx: &App) -> usize {
        let content = self.runtime.content.read(cx);
        known_recents(&content.recent_themes, &content.themes).len()
    }

    /// The slide a thumbnail shows: the scrubbed one, otherwise its own.
    fn shown(&self, index: usize, cell: &Thumb) -> usize {
        match self.scrub {
            Some((scrubbed, slide)) if scrubbed == index => slide,
            _ => cell.slide,
        }
    }

    fn assignment(&self, cell: &Thumb, cx: &App) -> Option<String> {
        let content = self.runtime.content.read(cx);
        let theme = content.themes.get(cell.theme)?;
        let (slide, _) = theme.slides.get(cell.slide)?;
        Some(format!("{}::{slide}", theme.id))
    }

    fn pick(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let cells = self.cells(cx);
        let Some(cell) = cells.get(index) else {
            return;
        };
        let shown = Thumb {
            slide: self.shown(index, cell),
            ..*cell
        };
        let Some(assignment) = self.assignment(&shown, cx) else {
            return;
        };
        self.set_open(false, window, cx);
        if let Some(on_pick) = self.on_pick.clone() {
            on_pick(assignment, window, cx);
        }
    }

    fn preview(
        &mut self,
        theme: &Theme,
        slide: usize,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Preview>> {
        // The test dispatcher never parks while native stills render on their own threads.
        if cfg!(test) {
            return None;
        }
        let (slide_id, _) = theme.slides.get(slide)?;
        let key = format!("{}::{slide_id}", theme.id);
        if let Some(preview) = self.previews.get(&key) {
            return Some(preview.clone());
        }
        let workspace = self.runtime.content.read(cx).workspace.clone();
        let document = workspace["themes"]
            .as_array()?
            .iter()
            .find(|candidate| candidate["id"] == theme.id)?["slides"]
            .as_array()?
            .iter()
            .find(|candidate| candidate["id"] == *slide_id)?
            .clone();
        let size = FrameSize {
            width: document["width"].as_f64().filter(|w| *w >= 1.)?.ceil() as u32,
            height: document["height"].as_f64().filter(|h| *h >= 1.)?.ceil() as u32,
        };
        // The key carries the slide's content, so an edited theme never shows a stale bitmap.
        let mut hasher = DefaultHasher::new();
        document.to_string().hash(&mut hasher);
        let source = Source::Still {
            key: format!(
                "theme-picker:{}:{key}:{:x}",
                self.runtime.content.read(cx).workspace_path,
                hasher.finish()
            ),
            slide: Arc::new(document),
            transparent: true,
        };
        let (host, frames) = (self.runtime.host.clone(), self.frames.clone());
        let preview = cx.new(|cx| Preview::new(host, frames, cx));
        preview.update(cx, |preview, cx| {
            preview.show_checkerboard(cx);
            preview.set_source(Some(source), Some(size), 0, cx)
        });
        self.previews.insert(key, preview.clone());
        Some(preview)
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let cells = self.cells(cx);
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => self.set_open(false, window, cx),
            "enter" => self.pick(self.highlighted, window, cx),
            "left" | "right" if event.keystroke.modifiers.shift => {
                let Some(cell) = cells.get(self.highlighted) else {
                    return;
                };
                let count = self
                    .runtime
                    .content
                    .read(cx)
                    .themes
                    .get(cell.theme)
                    .map_or(0, |theme| theme.slides.len());
                let shown = self.shown(self.highlighted, cell);
                let next = if key == "left" {
                    shown.saturating_sub(1)
                } else {
                    (shown + 1).min(count.saturating_sub(1))
                };
                self.scrub = Some((self.highlighted, next));
            }
            "left" | "right" | "up" | "down" => {
                self.highlighted = step_highlight(self.highlighted, key, cells.len());
                self.scrub = None;
                self.scroll_to_highlighted();
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Scrolls the thumbnail drawn last frame into the viewport.
    fn scroll_to_highlighted(&self) {
        let Some(cell) = self.bounds.borrow().get(&self.highlighted).copied() else {
            return;
        };
        let viewport = self.scroll.bounds();
        let mut offset = self.scroll.offset();
        if cell.top() < viewport.top() {
            offset.y += viewport.top() - cell.top();
        } else if cell.bottom() > viewport.bottom() {
            offset.y -= cell.bottom() - viewport.bottom();
        }
        self.scroll.set_offset(offset);
    }

    fn thumbnail(
        &mut self,
        section: &'static str,
        index: usize,
        cell: Thumb,
        t: Translator,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = *cx.theme();
        let content = self.runtime.content.read(cx);
        let Some(entry) = content.themes.get(cell.theme).cloned() else {
            return div().into_any_element();
        };
        let current =
            position(&content.themes, &self.value).map(|(theme, _)| theme) == Some(cell.theme);
        let shown = self.shown(index, &cell);
        let count = entry.slides.len();
        let title = if entry.title.is_empty() {
            t.t("cue.untitled")
        } else {
            entry.title.clone()
        };
        let label = t.format(
            "themePicker.slideOf",
            &[
                ("theme", &title),
                ("number", &(shown + 1).to_string()),
                ("count", &count.to_string()),
            ],
        );
        let preview = self.preview(&entry, shown, cx);
        let highlighted = self.highlighted == index;
        let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
        let (placed, scrubbed, registry) = (bounds.clone(), bounds, self.bounds.clone());
        div()
            .id(SharedString::from(format!("{}-{section}-{index}", self.id)))
            .debug_selector({
                let id = format!("theme-picker-{section}-{index}");
                move || id.clone()
            })
            .role(Role::Button)
            .aria_label(label)
            .aria_selected(current)
            .w(relative(1. / COLUMNS as f32))
            .flex_none()
            .p(theme.metrics.space_xs)
            .cursor_pointer()
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                let area = scrubbed.get();
                let slide = scrub_index(
                    f32::from(event.position.x - area.origin.x),
                    f32::from(area.size.width),
                    count,
                );
                if this.scrub != Some((index, slide)) || this.highlighted != index {
                    this.scrub = Some((index, slide));
                    this.highlighted = index;
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if !hovered && this.scrub.is_some_and(|(scrubbed, _)| scrubbed == index) {
                    this.scrub = None;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.pick(index, window, cx)))
            .child(
                div()
                    .relative()
                    .w_full()
                    .aspect_ratio(16. / 9.)
                    .overflow_hidden()
                    .rounded(theme.metrics.radius_xs)
                    .border_1()
                    .border_color(if current {
                        theme.colors.selected
                    } else if highlighted {
                        theme.colors.focus_ring
                    } else {
                        theme.colors.border_soft
                    })
                    .bg(theme.colors.preview_matte_bg)
                    .child(
                        gpui::canvas(
                            move |area, _, _| {
                                placed.set(area);
                                registry.borrow_mut().insert(index, area);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .children(preview.map(|preview| div().absolute().size_full().child(preview))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(theme.metrics.space_xs)
                    .pt(theme.metrics.space_2xs)
                    .text_size(theme.metrics.font_sm)
                    .text_color(theme.colors.text)
                    .children(current.then(|| {
                        div()
                            .debug_selector(|| "theme-picker-current".into())
                            .size(px(6.))
                            .flex_none()
                            .rounded_full()
                            .bg(theme.colors.selected)
                    }))
                    .child(div().min_w_0().truncate().child(title)),
            )
            .into_any_element()
    }
}

impl Render for ThemePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let content = self.runtime.content.read(cx);
        let selected = position(&content.themes, &self.value)
            .map(|(index, slide)| (content.themes[index].clone(), slide));
        let label = selected.as_ref().map_or_else(
            || t.t("theme.assignment.choose"),
            |(theme, slide)| format!("{} / {}", theme.title, theme.slides[*slide].1),
        );
        let preview = selected.and_then(|(theme, slide)| self.preview(&theme, slide, cx));
        let owner = cx.weak_entity();
        let trigger = NativeButton::new(SharedString::from(format!("{}-trigger", self.id)))
            .debug_selector({
                let id = format!("button-{}-trigger", self.id);
                move || id.clone()
            })
            .accessibility_label(label.clone())
            .disabled(self.disabled)
            .flex()
            .items_center()
            .min_w_0()
            .gap(theme.metrics.space_xs)
            .border_1()
            .border_color(theme.colors.control_border)
            .rounded(theme.metrics.radius_sm)
            .when(self.disabled, |button| button.opacity(0.55))
            .when(!self.disabled, |button| {
                button
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.colors.control_bg_hover))
                    .focus_visible(|style| style.border_color(theme.colors.focus_ring))
            })
            .child(
                div()
                    .debug_selector(|| "theme-picker-preview".into())
                    .relative()
                    .w(theme.metrics.control_height * 3.)
                    .aspect_ratio(16. / 9.)
                    .flex_none()
                    .overflow_hidden()
                    .bg(theme.colors.preview_matte_bg)
                    .children(preview.map(|preview| div().absolute().size_full().child(preview))),
            )
            .child(orbit_ui::icon("chevron-down").size(theme.metrics.font_sm))
            .child(
                div()
                    .min_w_0()
                    .pr(theme.metrics.space_sm)
                    .text_size(theme.metrics.font_sm)
                    .truncate()
                    .child(label),
            )
            .on_click({
                let owner = owner.clone();
                move |_, window, cx| {
                    owner
                        .update(cx, |this, cx| {
                            let open = !this.open;
                            this.set_open(open, window, cx)
                        })
                        .ok();
                }
            });
        let mut popover = Popover::new(SharedString::from(format!("{}-popover", self.id)), trigger)
            .on_dismiss(move |_, window, cx| {
                owner
                    .update(cx, |this, cx| this.set_open(false, window, cx))
                    .ok();
            });
        if self.open {
            let cells = self.cells(cx);
            let recents = self.recent_count(cx);
            let thumbnails = cells
                .iter()
                .enumerate()
                .map(|(index, cell)| {
                    let section = if index < recents { "recent" } else { "all" };
                    (
                        index < recents,
                        self.thumbnail(section, index, *cell, t, cx),
                    )
                })
                .collect::<Vec<_>>();
            let (recent, all): (Vec<_>, Vec<_>) =
                thumbnails.into_iter().partition(|(recent, _)| *recent);
            let grid = |items: Vec<(bool, gpui::AnyElement)>| {
                div()
                    .flex()
                    .flex_wrap()
                    .w_full()
                    .children(items.into_iter().map(|(_, element)| element))
            };
            popover = popover.content(
                div()
                    .id(SharedString::from(format!("{}-panel", self.id)))
                    .debug_selector(|| "theme-picker-panel".into())
                    .role(Role::Dialog)
                    .aria_label(t.t("cue.menu.themes"))
                    .track_focus(&self.focus)
                    .key_context("ThemePicker")
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.on_key(event, window, cx)
                    }))
                    .flex()
                    .flex_col()
                    .w(WIDTH)
                    .max_h(MAX_HEIGHT)
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(theme.metrics.radius_md)
                    .bg(theme.colors.popover_bg)
                    .shadow(theme.shadow_soft())
                    .text_color(theme.colors.text)
                    .child(
                        div()
                            .flex_none()
                            .px(theme.metrics.space_sm)
                            .py(theme.metrics.space_xs)
                            .text_size(theme.metrics.font_md)
                            .text_color(theme.colors.text_strong)
                            .child(t.t("cue.menu.themes")),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-scroll", self.id)))
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .px(theme.metrics.space_xs)
                            .pb(theme.metrics.space_xs)
                            .when(!recent.is_empty(), |this| {
                                this.child(
                                    div()
                                        .px(theme.metrics.space_xs)
                                        .text_size(theme.metrics.font_sm)
                                        .text_color(theme.colors.text_muted)
                                        .child(t.t("themePicker.recents")),
                                )
                                .child(grid(recent))
                                .child(
                                    div()
                                        .h(px(1.))
                                        .my(theme.metrics.space_xs)
                                        .bg(theme.colors.menu_separator),
                                )
                            })
                            .child(grid(all)),
                    ),
            );
        }
        div().w_full().child(popover)
    }
}
