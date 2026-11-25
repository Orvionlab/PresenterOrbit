//! The picker popover of every library transition. The left side filters by name and category, lists the recently
//! picked effects, and shows a grid of effect thumbnails; the right side previews the highlighted effect in a loop
//! and edits the default duration. Clicking an effect, or Enter, makes it the default of the footer's kind.
//! Previews are the host's own blend of two sample cards (`native_transitions::render_preview`), rendered off the
//! UI thread and cached per effect. Port of `src/renderer/react/cue-editor/TransitionPicker.tsx`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    div, prelude::*, px, relative, AnyElement, App, Bounds, Context, Entity, FocusHandle,
    Focusable, KeyDownEvent, Pixels, RenderImage, Role, ScrollHandle, SharedString, Subscription,
    Task, Window,
};
use orbit_host::native_transitions::{render_preview, CUT_ID, LIBRARY_DURATION_SECONDS};
use orbit_ui::controls::{
    input_state, ActionButton, ButtonVariant, InputEvent, InputState, NumberInput, RangeInput,
    Select, SelectOption, TextInput,
};
use orbit_ui::{bgra_image, ActiveTheme, FrameSize, Translator};

use super::{mode, recents, seconds_of, Effect, Mode, TransitionDefaults};

pub const COLUMNS: usize = 4;
const THUMB: (u32, u32) = (160, 90);
const BIG: (u32, u32) = (288, 162);
/// Frames of the looping preview, from progress 0 to 1.
const FRAMES: usize = 12;
/// Loop ticks that hold the finished state before the preview restarts.
const HELD: usize = 3;
const WIDTH: Pixels = px(760.);
const DETAIL_WIDTH: Pixels = px(296.);
const HEIGHT: Pixels = px(400.);
/// Below this width the preview side is left out.
const COMPACT_WIDTH: Pixels = px(640.);
const MIN_SECONDS: f64 = 0.;
const MAX_SECONDS: f64 = 5.;

/// The effects whose name contains `query` (case-insensitive) in `category` (empty: every category), as indices
/// into `effects`.
pub fn filter(effects: &[Effect], query: &str, category: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    effects
        .iter()
        .enumerate()
        .filter(|(_, effect)| {
            (category.is_empty() || effect.category == category)
                && effect.name.to_lowercase().contains(&query)
        })
        .map(|(index, _)| index)
        .collect()
}

/// Where an arrow key moves the highlighted effect in a grid of `len` effects.
pub fn step_highlight(current: usize, key: &str, len: usize) -> usize {
    let last = len.saturating_sub(1);
    match key {
        "left" => current.saturating_sub(1),
        "right" => (current + 1).min(last),
        "up" => current.saturating_sub(COLUMNS),
        "down" if current + COLUMNS <= last => current + COLUMNS,
        // Down from a short last row ends on the last effect, as a grid does.
        "down" if current / COLUMNS < last / COLUMNS => last,
        _ => current,
    }
}

/// Cut has no duration, so its picker controls stay hidden.
pub fn has_duration(effect: &str) -> bool {
    !effect.eq_ignore_ascii_case(CUT_ID)
}

/// The effect at `progress` as a bitmap of `size`, rendered by the output's own blend.
pub fn preview_image(
    effect: &str,
    progress: f64,
    size: (u32, u32),
) -> Result<Arc<RenderImage>, String> {
    let frame = render_preview(effect, progress, size.0 as usize, size.1 as usize)?;
    bgra_image(
        FrameSize {
            width: size.0,
            height: size.1,
        },
        frame.pixels,
    )
}

pub(super) struct Picker {
    pub open: bool,
    filter: Option<Entity<InputState>>,
    number: Option<Entity<InputState>>,
    category: String,
    pub(super) highlighted: String,
    /// Thumbnails stay cached for the footer's life; a preview never depends on the workspace.
    thumbs: HashMap<String, Arc<RenderImage>>,
    thumbs_running: bool,
    pub(super) frames: HashMap<String, Vec<Arc<RenderImage>>>,
    requested: HashSet<String>,
    step: usize,
    /// The effect whose frames the user wants now, so a queued render of one passed over is skipped.
    wanted: Arc<Mutex<String>>,
    lane: Arc<futures::lock::Mutex<()>>,
    bounds: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
    scroll: ScrollHandle,
    focus: FocusHandle,
    tasks: Vec<Task<()>>,
    subscriptions: Vec<Subscription>,
}

impl Picker {
    pub fn new(cx: &mut Context<TransitionDefaults>) -> Self {
        cx.on_release(|this, cx| {
            let images = this
                .picker
                .thumbs
                .drain()
                .map(|(_, image)| image)
                .chain(this.picker.frames.drain().flat_map(|(_, frames)| frames));
            for image in images {
                cx.drop_image(image, None);
            }
        })
        .detach();
        Self {
            open: false,
            filter: None,
            number: None,
            category: String::new(),
            highlighted: String::new(),
            thumbs: HashMap::new(),
            thumbs_running: false,
            frames: HashMap::new(),
            requested: HashSet::new(),
            step: 0,
            wanted: Arc::default(),
            lane: Arc::default(),
            bounds: Rc::default(),
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            tasks: Vec::new(),
            subscriptions: Vec::new(),
        }
    }
}

impl TransitionDefaults {
    pub(crate) fn picker_width(window: &Window) -> Pixels {
        WIDTH.min(window.viewport_size().width - px(16.))
    }
    pub(super) fn toggle_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.open {
            self.close_picker(cx);
        } else {
            self.open_picker(window, cx);
        }
    }

    pub(super) fn close_picker(&mut self, cx: &mut Context<Self>) {
        if !self.picker.open {
            return;
        }
        self.picker.open = false;
        self.picker.tasks.clear();
        self.picker.thumbs_running = false;
        self.picker.requested.clear();
        self.picker.bounds.borrow_mut().clear();
        cx.notify();
    }

    pub(super) fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let transition = self.transition(cx);
        self.picker.highlighted = match mode(&transition) {
            Mode::Cut => CUT_ID.to_string(),
            _ => transition["effectId"]
                .as_str()
                .unwrap_or(CUT_ID)
                .to_string(),
        };
        self.picker.open = true;
        self.picker.category.clear();
        self.picker.step = 0;
        if self.picker.filter.is_none() {
            let filter = input_state("", window, cx);
            let number = input_state("", window, cx);
            let commit = cx.subscribe_in(
                &number,
                window,
                |this, state, event: &InputEvent, window, cx| {
                    if !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        return;
                    }
                    match state.read(cx).value().trim().parse::<f64>() {
                        Ok(seconds) if seconds.is_finite() => {
                            this.set_duration(seconds, window, cx)
                        }
                        // Unreadable text goes back to the current duration on the next render.
                        _ => cx.notify(),
                    }
                },
            );
            let refresh = cx.observe(&filter, |_, _, cx| cx.notify());
            self.picker.subscriptions = vec![commit, refresh];
            self.picker.filter = Some(filter);
            self.picker.number = Some(number);
        }
        if let Some(filter) = &self.picker.filter {
            filter.update(cx, |state, cx| state.set_value("", window, cx));
            filter.focus_handle(cx).focus(window, cx);
        }
        self.start_previews(cx);
        cx.notify();
    }

    /// Renders what the open picker shows and the loop that steps through the highlighted effect.
    fn start_previews(&mut self, cx: &mut Context<Self>) {
        self.start_thumbs(cx);
        // The test executor must be able to park without an ongoing animation.
        if cfg!(test) {
            return;
        }
        self.request_frames(cx);
        let looping = cx.spawn(async move |this, cx| loop {
            let wait = this.update(cx, |this, cx| this.tick(cx));
            match wait {
                Ok(wait) => cx.background_executor().timer(wait).await,
                Err(_) => break,
            }
        });
        self.picker.tasks.push(looping);
    }

    pub(super) fn start_thumbs(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self
            .effects
            .iter()
            .filter(|effect| !self.picker.thumbs.contains_key(&effect.id))
            .map(|effect| effect.id.clone())
            .collect();
        if ids.is_empty() || self.picker.thumbs_running {
            return;
        }
        self.picker.thumbs_running = true;
        let task = cx.spawn(async move |this, cx| {
            for id in ids {
                let key = id.clone();
                let image = cx
                    .background_spawn(async move { preview_image(&key, 0.5, THUMB) })
                    .await;
                let delivered = this.update(cx, |this, cx| match image {
                    Ok(image) => {
                        this.picker.thumbs.insert(id, image);
                        cx.notify();
                        true
                    }
                    Err(error) => {
                        crate::cues::actions::error(&this.runtime, cx, error);
                        false
                    }
                });
                if !matches!(delivered, Ok(true)) {
                    break;
                }
            }
            let _ = this.update(cx, |this, _| this.picker.thumbs_running = false);
        });
        self.picker.tasks.push(task);
    }

    /// Renders the looping frames of the highlighted effect, unless they are cached or on their way.
    fn request_frames(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let id = self.picker.highlighted.clone();
        if let Ok(mut wanted) = self.picker.wanted.lock() {
            wanted.clone_from(&id);
        }
        if self.picker.frames.contains_key(&id) || !self.picker.requested.insert(id.clone()) {
            return;
        }
        let count = if has_duration(&id) { FRAMES } else { 2 };
        let (wanted, lane) = (self.picker.wanted.clone(), self.picker.lane.clone());
        let task = cx.spawn(async move |this, cx| {
            let key = id.clone();
            let rendered = cx
                .background_spawn(async move {
                    let _lane = lane.lock().await;
                    if wanted.lock().map_or(true, |wanted| *wanted != key) {
                        return Ok(None);
                    }
                    (0..count)
                        .map(|index| preview_image(&key, index as f64 / (count - 1) as f64, BIG))
                        .collect::<Result<Vec<_>, _>>()
                        .map(Some)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.picker.requested.remove(&id);
                match rendered {
                    Ok(Some(frames)) => {
                        this.picker.frames.insert(id, frames);
                        cx.notify();
                    }
                    Ok(None) => {}
                    Err(error) => crate::cues::actions::error(&this.runtime, cx, error),
                }
            });
        });
        self.picker.tasks.push(task);
    }

    /// Steps the preview loop and returns how long its frame stays: the duration's share, never faster than 40 ms.
    fn tick(&mut self, cx: &mut Context<Self>) -> Duration {
        let frames = self
            .picker
            .frames
            .get(&self.picker.highlighted)
            .map_or(0, Vec::len);
        if frames > 1 && has_duration(&self.picker.highlighted) {
            self.picker.step = (self.picker.step + 1) % (frames + HELD);
            cx.notify();
        }
        let share = seconds_of(&self.duration, cx) * 1000. / frames.max(1) as f64;
        Duration::from_millis(share.max(40.) as u64)
    }

    fn highlight(&mut self, id: String, cx: &mut Context<Self>) {
        self.picker.highlighted = id;
        self.picker.step = 0;
        self.request_frames(cx);
        cx.notify();
    }

    /// Makes `id` the default of this footer's kind, at the current duration, and the newest recent.
    pub(super) fn pick_effect(&mut self, id: &str, cx: &mut Context<Self>) {
        let seconds = seconds_of(&self.duration, cx);
        let workspace = self.runtime.content.read(cx).workspace.clone();
        let Some(effect) = self.effects.iter().find(|effect| effect.id == id) else {
            return;
        };
        if self.has_override() {
            // A presentation keeps a cut as an explicit override; no transition means the default.
            let transition = if effect.id.eq_ignore_ascii_case(CUT_ID) {
                serde_json::json!({"effectId": CUT_ID, "durationSeconds": 0})
            } else {
                super::effect_transition(effect, seconds)
            };
            if self.media_id.is_some() {
                self.save_media_pick(transition, cx);
            } else {
                self.save(transition, cx);
            }
        } else {
            let patch = super::pick_patch(&workspace, self.kind, effect, seconds);
            self.save_patch(patch, cx);
        }
        self.highlight(id.to_string(), cx);
    }

    fn save_media_pick(&mut self, transition: serde_json::Value, cx: &mut Context<Self>) {
        if let Some(id) = transition["effectId"].as_str() {
            let recents = orbit_host::native_transitions::remember_recent(
                &recents(&self.runtime.content.read(cx).workspace),
                id,
            );
            self.save_patch(
                serde_json::json!({"recentTransitionEffectIds": recents}),
                cx,
            );
        }
        self.save(transition, cx);
    }

    pub(super) fn clear_recents(&mut self, cx: &mut Context<Self>) {
        self.save_patch(serde_json::json!({"recentTransitionEffectIds": []}), cx);
    }

    /// Sets the duration shown by the slider and number field, and saves it for the current default.
    pub(super) fn set_duration(
        &mut self,
        seconds: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let seconds = (seconds.clamp(MIN_SECONDS, MAX_SECONDS) * 10.).round() / 10.;
        self.duration.update(cx, |slider, cx| {
            slider.set_value(seconds as f32, window, cx)
        });
        self.save_duration(seconds, cx);
    }

    fn visible(&self, cx: &App) -> Vec<usize> {
        let query = self
            .picker
            .filter
            .as_ref()
            .map(|filter| filter.read(cx).value().to_string())
            .unwrap_or_default();
        filter(&self.effects, &query, &self.picker.category)
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let number_focused = self
            .picker
            .number
            .as_ref()
            .is_some_and(|number| number.focus_handle(cx).is_focused(window));
        let query_empty = self
            .picker
            .filter
            .as_ref()
            .is_none_or(|filter| filter.read(cx).value().is_empty());
        let filter_focused = self
            .picker
            .filter
            .as_ref()
            .is_some_and(|filter| filter.focus_handle(cx).is_focused(window));
        match key {
            "escape" if self.prop_id.is_some() => return,
            "escape" => self.close_picker(cx),
            "enter" if !number_focused => {
                let id = self.picker.highlighted.clone();
                self.pick_effect(&id, cx);
            }
            // Left and Right edit the filter text unless it is empty.
            "left" | "right" if filter_focused && !query_empty => return,
            "left" | "right" | "up" | "down" if !number_focused => {
                let visible = self.visible(cx);
                let at = visible
                    .iter()
                    .position(|index| self.effects[*index].id == self.picker.highlighted)
                    .unwrap_or_default();
                if let Some(index) = visible.get(step_highlight(at, key, visible.len())) {
                    let id = self.effects[*index].id.clone();
                    self.highlight(id.clone(), cx);
                    self.scroll_to(&id);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Scrolls the effect drawn last frame into the viewport.
    fn scroll_to(&self, id: &str) {
        let Some(cell) = self.picker.bounds.borrow().get(id).copied() else {
            return;
        };
        let viewport = self.picker.scroll.bounds();
        let mut offset = self.picker.scroll.offset();
        if cell.top() < viewport.top() {
            offset.y += viewport.top() - cell.top();
        } else if cell.bottom() > viewport.bottom() {
            offset.y -= cell.bottom() - viewport.bottom();
        }
        self.picker.scroll.set_offset(offset);
    }

    fn tile(&mut self, index: usize, active: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let effect = &self.effects[index];
        let (id, name) = (effect.id.clone(), effect.name.clone());
        let chosen = id.eq_ignore_ascii_case(active);
        let highlighted = id == self.picker.highlighted;
        let thumb = self.picker.thumbs.get(&id).cloned();
        let area = Rc::new(Cell::new(Bounds::<Pixels>::default()));
        let (registry, key) = (self.picker.bounds.clone(), id.clone());
        let pick = id.clone();
        div()
            .id(SharedString::from(format!("transition-effect-{index}")))
            .debug_selector({
                let selector = format!("transition-effect-{index}");
                move || selector.clone()
            })
            .role(Role::Button)
            .aria_label(name.clone())
            .aria_selected(chosen)
            .w(relative(1. / COLUMNS as f32))
            .flex_none()
            .p(theme.metrics.space_xs)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.pick_effect(&pick, cx)))
            .child(
                div()
                    .relative()
                    .when(thumb.is_some(), |tile| {
                        tile.debug_selector(move || format!("transition-effect-preview-{index}"))
                    })
                    .w_full()
                    .aspect_ratio(16. / 9.)
                    .overflow_hidden()
                    .rounded(theme.metrics.radius_xs)
                    .border_1()
                    .border_color(if chosen {
                        theme.colors.selected
                    } else if highlighted {
                        theme.colors.focus_ring
                    } else {
                        theme.colors.border_soft
                    })
                    .bg(theme.colors.preview_matte_bg)
                    .child(
                        gpui::canvas(
                            move |bounds, _, _| {
                                area.set(bounds);
                                registry.borrow_mut().insert(key.clone(), bounds);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .children(thumb.map(|image| {
                        gpui::img(image)
                            .absolute()
                            .size_full()
                            .object_fit(gpui::ObjectFit::Contain)
                    })),
            )
            .child(
                div()
                    .pt(theme.metrics.space_2xs)
                    .min_w_0()
                    .truncate()
                    .text_size(theme.metrics.font_sm)
                    .text_color(theme.colors.text)
                    .child(name),
            )
            .into_any_element()
    }

    pub(super) fn picker_panel(
        &mut self,
        active: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let transition = self.transition(cx);
        let inherited = self.has_override() && transition.is_null();
        let active_id = match active {
            _ if inherited => String::new(),
            Mode::Cut => CUT_ID.to_string(),
            _ => transition["effectId"]
                .as_str()
                .unwrap_or(CUT_ID)
                .to_string(),
        };
        let seconds = seconds_of(&self.duration, cx);
        if let Some(number) = self.picker.number.clone() {
            let text = format!("{seconds:.1}");
            if !number.focus_handle(cx).is_focused(window) && number.read(cx).value() != text {
                number.update(cx, |state, cx| state.set_value(text, window, cx));
            }
        }
        let width = Self::picker_width(window);
        let height = HEIGHT.min(window.viewport_size().height - px(48.));
        let owner = cx.weak_entity();
        let visible = self.visible(cx);
        let recent_effects: Vec<(String, SharedString)> =
            recents(&self.runtime.content.read(cx).workspace)
                .iter()
                .filter_map(|id| {
                    self.effects
                        .iter()
                        .find(|effect| effect.id.eq_ignore_ascii_case(id))
                        .map(|effect| (effect.id.clone(), effect.name.clone()))
                })
                .collect();
        let mut categories: Vec<SharedString> = Vec::new();
        for effect in &self.effects {
            if !categories.contains(&effect.category) {
                categories.push(effect.category.clone());
            }
        }
        let mut options = vec![SelectOption::new(
            String::new(),
            t.t("transition.picker.categoryAll"),
        )];
        options.extend(
            categories
                .iter()
                .map(|name| SelectOption::new(name.to_string(), name.clone())),
        );
        let section = |label: String| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(theme.metrics.control_height_compact)
                .px(theme.metrics.space_xs)
                .bg(theme.colors.border_soft)
                .text_size(theme.metrics.font_sm)
                .text_color(theme.colors.text_strong)
                .child(label)
        };
        let tiles: Vec<AnyElement> = visible
            .iter()
            .map(|index| self.tile(*index, &active_id, cx))
            .collect();
        let mut scroll = div()
            .id("transition-picker-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.picker.scroll)
            .px(theme.metrics.space_xs)
            .pb(theme.metrics.space_xs);
        if self.has_override() {
            scroll = scroll.child(
                div()
                    .id("transition-use-default")
                    .debug_selector(|| "transition-use-default".into())
                    .role(Role::Button)
                    .aria_label(t.t("transition.presentation.default"))
                    .px(theme.metrics.space_xs)
                    .py(theme.metrics.space_2xs)
                    .text_size(theme.metrics.font_sm)
                    .text_color(theme.colors.text)
                    .cursor_pointer()
                    .when(inherited, |this| this.bg(theme.colors.accent_soft))
                    .hover(|style| style.bg(theme.colors.accent_soft))
                    .on_click(cx.listener(|this, _, _, cx| this.save(serde_json::Value::Null, cx)))
                    .child(t.t("transition.presentation.default")),
            );
        }
        if self.media_id.is_some() {
            let content = self.runtime.content.read(cx);
            let favorites: Vec<_> = content.workspace["transitionPresets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|preset| {
                    let transition = &preset["transition"];
                    let effect = orbit_host::native_transitions::catalog_entry(
                        transition["effectId"].as_str()?,
                    )?;
                    if transition.get("variables").is_some()
                        || transition["direction"].as_str().is_some_and(|direction| {
                            orbit_host::native_transitions::direction_name(effect.default_direction)
                                != Some(direction)
                        })
                    {
                        return None;
                    }
                    Some((
                        preset["id"].as_str()?.to_owned(),
                        preset["name"].as_str()?.to_owned(),
                        transition.clone(),
                    ))
                })
                .collect();
            if !favorites.is_empty() {
                scroll = scroll
                    .child(section(t.t("transition.picker.favorites")))
                    .children(favorites.into_iter().map(|(id, name, transition)| {
                        ActionButton::new(
                            SharedString::from(format!("transition-favorite-{id}")),
                            name,
                        )
                        .compact()
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.save_media_pick(transition.clone(), cx)
                        }))
                    }));
            }
        }
        if !recent_effects.is_empty() {
            scroll = scroll
                .child(
                    section(t.t("transition.picker.recents")).child(
                        ActionButton::new("transition-clear", t.t("transition.picker.clear"))
                            .compact()
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(|this, _, _, cx| this.clear_recents(cx))),
                    ),
                )
                .children(
                    recent_effects
                        .into_iter()
                        .enumerate()
                        .map(|(index, (id, name))| {
                            let chosen = id.eq_ignore_ascii_case(&active_id);
                            div()
                                .id(SharedString::from(format!("transition-recent-{index}")))
                                .debug_selector({
                                    let selector = format!("transition-recent-{index}");
                                    move || selector.clone()
                                })
                                .role(Role::Button)
                                .aria_label(name.clone())
                                .px(theme.metrics.space_xs)
                                .py(theme.metrics.space_2xs)
                                .text_size(theme.metrics.font_sm)
                                .text_color(theme.colors.text)
                                .cursor_pointer()
                                .when(chosen, |this| this.bg(theme.colors.accent_soft))
                                .hover(|style| style.bg(theme.colors.accent_soft))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.pick_effect(&id, cx)),
                                )
                                .child(name)
                        }),
                );
        }
        scroll = scroll.child(section(t.t("transition.picker.all")));
        scroll = if tiles.is_empty() {
            scroll.child(
                div()
                    .p(theme.metrics.space_sm)
                    .text_size(theme.metrics.font_sm)
                    .text_color(theme.colors.text_muted)
                    .child(t.t("transition.picker.empty")),
            )
        } else {
            scroll.child(div().flex().flex_wrap().w_full().children(tiles))
        };
        let filters = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_end()
            .gap(theme.metrics.space_xs)
            .p(theme.metrics.space_xs)
            .child(
                div().w(theme.metrics.control_height * 3.).child(
                    Select::new("transition-category", options, self.picker.category.clone())
                        .label(t.t("transition.picker.category"))
                        .compact()
                        .on_change({
                            let owner = owner.clone();
                            move |category: String, _, cx| {
                                owner
                                    .update(cx, |this, cx| {
                                        this.picker.category = category;
                                        cx.notify();
                                    })
                                    .ok();
                            }
                        }),
                ),
            )
            .children(self.picker.filter.as_ref().map(|filter| {
                div().w(theme.metrics.control_height * 5.).child(
                    TextInput::new("transition-filter", filter)
                        .label(t.t("transition.picker.filter"))
                        .compact(),
                )
            }));
        let highlighted = self
            .effects
            .iter()
            .find(|effect| effect.id == self.picker.highlighted)
            .map(|effect| (effect.id.clone(), effect.name.clone()));
        let detail = highlighted.map(|(id, name)| {
            let frames = self.picker.frames.get(&id);
            let shown = frames.and_then(|frames| {
                let at = if has_duration(&id) {
                    self.picker.step.min(frames.len() - 1)
                } else {
                    frames.len() / 2
                };
                frames.get(at).cloned()
            });
            let number = self.picker.number.clone();
            div()
                .flex()
                .flex_none()
                .flex_col()
                .gap(theme.metrics.space_sm)
                .w(DETAIL_WIDTH)
                .p(theme.metrics.space_sm)
                .border_l_1()
                .border_color(theme.colors.border)
                .child(
                    div()
                        .truncate()
                        .text_center()
                        .text_size(theme.metrics.font_md)
                        .text_color(theme.colors.text_strong)
                        .child(name.clone()),
                )
                .child(
                    div()
                        .id("transition-preview")
                        .role(Role::Image)
                        .aria_label(t.format("transition.picker.previewOf", &[("name", &name)]))
                        .relative()
                        .w_full()
                        .aspect_ratio(16. / 9.)
                        .overflow_hidden()
                        .rounded(theme.metrics.radius_xs)
                        .border_1()
                        .border_color(theme.colors.border_soft)
                        .bg(theme.colors.preview_matte_bg)
                        .children(shown.map(|image| {
                            gpui::img(image)
                                .absolute()
                                .size_full()
                                .object_fit(gpui::ObjectFit::Contain)
                        })),
                )
                .when(has_duration(&id), |this| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(theme.metrics.space_xs)
                            .text_size(theme.metrics.font_sm)
                            .child(t.t("transition.picker.duration"))
                            .child(
                                div().flex_1().min_w_0().child(
                                    RangeInput::new(&self.duration)
                                        .label(t.t("transition.picker.duration")),
                                ),
                            )
                            .children(number.map(|number| {
                                let owner = owner.clone();
                                div().flex_none().w(px(72.)).child(
                                    NumberInput::new("transition-seconds", &number)
                                        .label(t.t("transition.picker.durationValue"))
                                        .range(MIN_SECONDS, MAX_SECONDS)
                                        .step(0.1)
                                        .stepper()
                                        .on_step(move |value, window, cx| {
                                            owner
                                                .update(cx, |this, cx| {
                                                    this.set_duration(value, window, cx)
                                                })
                                                .ok();
                                        }),
                                )
                            })),
                    )
                    .child(
                        div().flex().justify_center().child(
                            ActionButton::new("transition-reset", t.t("transition.picker.reset"))
                                .variant(ButtonVariant::Secondary)
                                .disabled((seconds - LIBRARY_DURATION_SECONDS).abs() < 0.05)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_duration(LIBRARY_DURATION_SECONDS, window, cx)
                                })),
                        ),
                    )
                })
        });
        div()
            .id("transition-picker-panel")
            .debug_selector(|| "transition-picker-panel".into())
            .role(Role::Dialog)
            .aria_label(t.t("transition.picker.title"))
            .track_focus(&self.picker.focus)
            .key_context("TransitionPicker")
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.on_key(event, window, cx)
                }),
            )
            .flex()
            .w(width)
            .h(height)
            .overflow_hidden()
            .border_1()
            .border_color(theme.colors.border)
            .rounded(theme.metrics.radius_md)
            .bg(theme.colors.popover_bg)
            .shadow(theme.shadow_soft())
            .text_color(theme.colors.text)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .flex_col()
                    .min_w_0()
                    .child(filters)
                    .child(scroll),
            )
            .when(width >= COMPACT_WIDTH, |this| this.children(detail))
    }
}
