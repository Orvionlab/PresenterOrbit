//! The footer controls of the slide grid, media bin and props for the global default transition: icon buttons for cut,
//! fade and the picker of every library effect, and the duration of a transition that has one. Slides use the
//! workspace's `slideTransition`; media and props use their entries of `layerTransitions`. A
//! transition set on a slide, presentation, media selection or prop still wins; the output resolves that.

use gpui::{
    div, prelude::*, Anchor, App, Context, Entity, IntoElement, Render, SharedString, Subscription,
    Window,
};
use orbit_host::native_transitions::{CUT_ID, DISSOLVE_ID, LIBRARY_DURATION_SECONDS};
use orbit_ui::{
    controls::{
        range_state, ActionButton, ButtonVariant, Popover, RangeInput, SegmentedControl,
        SegmentedOption, SliderEvent, SliderState,
    },
    i18n::Translator,
    ActiveTheme,
};
use serde_json::{json, Map, Value};

use crate::state::runtime::Runtime;

mod picker;

const LIBRARY_DURATION: f32 = LIBRARY_DURATION_SECONDS as f32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Slide,
    Media,
    Props,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Cut,
    Fade,
    Other,
}

/// The default transition of a kind; `Null` is a cut.
pub fn current(workspace: &Value, kind: Kind) -> Value {
    match kind {
        Kind::Slide => workspace["slideTransition"].clone(),
        Kind::Media => workspace["layerTransitions"]["background"].clone(),
        Kind::Props => workspace["layerTransitions"]["props"].clone(),
    }
}

pub fn mode(transition: &Value) -> Mode {
    let seconds = transition["durationSeconds"].as_f64();
    match transition["effectId"].as_str().map(str::to_uppercase) {
        Some(id) if id != CUT_ID && seconds.is_some_and(|seconds| seconds >= 0.) => {
            if id == DISSOLVE_ID {
                Mode::Fade
            } else {
                Mode::Other
            }
        }
        _ => Mode::Cut,
    }
}

/// A cut has no duration, so its slider and readout stay hidden.
pub fn shows_duration(mode: Mode) -> bool {
    mode != Mode::Cut
}

pub fn fade(seconds: f64) -> Value {
    json!({"effectId": DISSOLVE_ID, "durationSeconds": seconds})
}

/// The `saveWorkspaceContent` patch that makes `transition` (`Null` for a cut) the default of `kind`. Media
/// defaults cover both media layers and leave the prop and message defaults as they are.
pub fn patch(workspace: &Value, kind: Kind, transition: Value) -> Value {
    match kind {
        Kind::Slide => json!({"slideTransition": transition}),
        Kind::Media => layer_patch(workspace, &["background", "foreground"], transition),
        Kind::Props => layer_patch(workspace, &["props"], transition),
    }
}

fn layer_patch(workspace: &Value, keys: &[&str], transition: Value) -> Value {
    let mut layers: Map<String, Value> = workspace["layerTransitions"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for key in keys {
        if transition.is_null() {
            layers.remove(*key);
        } else {
            layers.insert((*key).into(), transition.clone());
        }
    }
    json!({"layerTransitions": layers})
}

/// One effect of the host's transition library.
struct Effect {
    id: String,
    name: SharedString,
    category: SharedString,
    direction: Option<String>,
}

/// The recently picked effect ids of a workspace, newest first.
pub fn recents(workspace: &Value) -> Vec<String> {
    workspace["recentTransitionEffectIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().map(str::to_string))
        .collect()
}

/// The transition that makes `effect` the default at `seconds`; a cut is `Null`.
fn effect_transition(effect: &Effect, seconds: f64) -> Value {
    if effect.id.eq_ignore_ascii_case(CUT_ID) {
        return Value::Null;
    }
    let mut next = json!({"effectId": effect.id, "durationSeconds": seconds});
    if let Some(direction) = &effect.direction {
        next["direction"] = json!(direction);
    }
    next
}

/// The `saveWorkspaceContent` patch of picking `effect` in the picker: it becomes the default of `kind` and the
/// newest recent in one write.
fn pick_patch(workspace: &Value, kind: Kind, effect: &Effect, seconds: f64) -> Value {
    let mut patch = patch(workspace, kind, effect_transition(effect, seconds));
    patch["recentTransitionEffectIds"] = json!(orbit_host::native_transitions::remember_recent(
        &recents(workspace),
        &effect.id
    ));
    patch
}

type InspectorChange = std::rc::Rc<dyn Fn(Value, &mut App)>;

pub struct TransitionDefaults {
    kind: Kind,
    /// Set on the presentation header's instance: the presentation whose own transition this edits, where no
    /// transition means the global default. The footer's instances leave it `None`.
    pub item_id: Option<String>,
    pub slide_id: Option<String>,
    prop_id: Option<String>,
    pub inspector: bool,
    pub preview: Option<Value>,
    preview_workspace: Option<String>,
    save_request: u64,
    pub on_change: Option<InspectorChange>,
    runtime: Runtime,
    duration: Entity<SliderState>,
    /// Every library transition, in library order.
    effects: Vec<Effect>,
    picker: picker::Picker,
    _subscriptions: Vec<Subscription>,
}

fn seconds_of(slider: &Entity<SliderState>, cx: &App) -> f64 {
    (f64::from(slider.read(cx).value().end()) * 10.).round() / 10.
}

impl TransitionDefaults {
    pub fn new(kind: Kind, runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let duration = range_state(0., 5., 0.1, LIBRARY_DURATION, cx);
        let duration_events = cx.subscribe(&duration, |this, slider, event: &SliderEvent, cx| {
            let seconds = seconds_of(&slider, cx);
            match event {
                SliderEvent::Change(_) => {
                    let mut next = this.transition(cx);
                    if mode(&next) != Mode::Cut {
                        next["durationSeconds"] = json!(seconds);
                        this.begin_preview(next, cx);
                    }
                }
                SliderEvent::Release(_) => this.save_duration(seconds, cx),
            }
        });
        let observe_duration = cx.observe(&duration, |_, _, cx| cx.notify());
        let observe_content = cx.observe(&runtime.content, |_, _, cx| cx.notify());
        let host = runtime.host.clone();
        cx.spawn(async move |this, cx| {
            let catalog = cx
                .background_spawn(async move { host.invoke("getTransitionCatalog", vec![]) })
                .await;
            let _ = this.update(cx, |this, cx| match catalog {
                Ok(catalog) => {
                    this.effects = catalog
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|entry| {
                            Some(Effect {
                                id: entry["id"].as_str()?.to_string(),
                                name: entry["name"].as_str()?.to_string().into(),
                                category: entry["category"].as_str()?.to_string().into(),
                                direction: entry["defaultDirection"].as_str().map(str::to_string),
                            })
                        })
                        .collect();
                    if this.picker.open {
                        this.start_thumbs(cx);
                    }
                    cx.notify();
                }
                Err(error) => crate::cues::actions::error(&this.runtime, cx, error),
            });
        })
        .detach();
        Self {
            kind,
            item_id: None,
            slide_id: None,
            prop_id: None,
            inspector: false,
            preview: None,
            preview_workspace: None,
            save_request: 0,
            on_change: None,
            picker: picker::Picker::new(cx),
            runtime,
            duration,
            effects: Vec::new(),
            _subscriptions: vec![duration_events, observe_duration, observe_content],
        }
    }

    /// The transition being edited: the presentation's own (`Null` is the global default) or the default of this kind.
    fn transition(&self, cx: &App) -> Value {
        if let Some(preview) = self.preview.as_ref().filter(|_| {
            self.preview_workspace
                .as_ref()
                .is_none_or(|path| *path == self.runtime.content.read(cx).workspace_path)
        }) {
            return preview.clone();
        }
        if let Some(id) = &self.prop_id {
            return self.runtime.content.read(cx).workspace["props"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|prop| prop["id"] == *id)
                .map_or(Value::Null, |prop| prop["transition"].clone());
        }
        match &self.item_id {
            Some(id) => self
                .runtime
                .content
                .read(cx)
                .items
                .get(id)
                .map_or(Value::Null, |item| match &self.slide_id {
                    Some(id) => item.value["slides"]
                        .as_array()
                        .and_then(|slides| slides.iter().find(|slide| slide["id"] == *id))
                        .map_or(Value::Null, |slide| slide["transition"].clone()),
                    None => item.value["transition"].clone(),
                }),
            None => current(&self.runtime.content.read(cx).workspace, self.kind),
        }
    }

    /// Saves `transition` (`Null` for a cut) as the default of this kind, or as the presentation's own transition
    /// (`Null` removes it).
    pub fn save(&mut self, transition: Value, cx: &mut Context<Self>) {
        if let Some(on_change) = self.on_change.clone() {
            self.preview = self.prop_id.is_none().then(|| transition.clone());
            on_change(transition, cx);
            cx.notify();
            return;
        }
        if let Some(id) = self.item_id.clone() {
            let (request, workspace) = self.begin_preview(transition.clone(), cx);
            let local = transition.clone();
            let slide_id = self.slide_id.clone();
            let mut operation = json!({"type": "setTransition", "transition": transition});
            if let Some(slide_id) = &slide_id {
                operation["slideId"] = json!(slide_id);
            }
            let saved = crate::cues::actions::mutate_optimistic_checked(
                &self.runtime,
                &id,
                operation,
                move |item| {
                    let target = match &slide_id {
                        Some(id) => item["slides"]
                            .as_array_mut()
                            .and_then(|slides| slides.iter_mut().find(|slide| slide["id"] == *id)),
                        None => Some(item),
                    };
                    if let Some(target) = target.and_then(Value::as_object_mut) {
                        if local.is_null() {
                            target.remove("transition");
                        } else {
                            target.insert("transition".into(), local);
                        }
                    }
                },
                cx,
            );
            cx.spawn(async move |this, cx| {
                saved.await;
                this.update(cx, |this, cx| this.finish_preview(request, &workspace, cx))
                    .ok();
            })
            .detach();
            return;
        }
        let patch = patch(
            &self.runtime.content.read(cx).workspace,
            self.kind,
            transition,
        );
        self.save_patch(patch, cx);
    }

    pub fn edit_prop(
        &mut self,
        id: String,
        on_change: InspectorChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prop_id = Some(id);
        self.on_change = Some(on_change);
        self.open_picker(window, cx);
    }

    fn has_override(&self) -> bool {
        self.item_id.is_some() || self.prop_id.is_some()
    }

    fn save_patch(&mut self, patch: Value, cx: &mut Context<Self>) {
        let pending = match self.kind {
            Kind::Slide => patch.get("slideTransition"),
            Kind::Media | Kind::Props => patch.get("layerTransitions"),
        }
        .map(|_| self.begin_preview(current(&patch, self.kind), cx));
        let runtime = self.runtime.clone();
        let completion = pending.clone();
        if let Err(error) = crate::automation::save::optimistic_content(
            &runtime,
            patch,
            cx,
            move |this, result, cx| {
                if let Err(error) = result {
                    crate::cues::actions::error(&this.runtime, cx, error);
                }
                if let Some((request, workspace)) = completion {
                    this.finish_preview(request, &workspace, cx);
                }
            },
        ) {
            crate::cues::actions::error(&self.runtime, cx, error);
            if let Some((request, workspace)) = pending {
                self.finish_preview(request, &workspace, cx);
            }
        }
    }

    fn begin_preview(&mut self, transition: Value, cx: &mut Context<Self>) -> (u64, String) {
        self.save_request += 1;
        let workspace = self.runtime.content.read(cx).workspace_path.clone();
        self.preview_workspace = Some(workspace.clone());
        self.preview = Some(transition);
        cx.notify();
        (self.save_request, workspace)
    }

    fn finish_preview(&mut self, request: u64, workspace: &str, cx: &mut Context<Self>) {
        if request == self.save_request && self.preview_workspace.as_deref() == Some(workspace) {
            self.preview = None;
            self.preview_workspace = None;
            cx.notify();
        }
    }

    /// Saves a new duration for the current default; a cut has none, so the value only waits for the next pick.
    fn save_duration(&mut self, seconds: f64, cx: &mut Context<Self>) {
        let mut next = self.transition(cx);
        if mode(&next) != Mode::Cut {
            next["durationSeconds"] = json!(seconds);
            self.save(next, cx);
        }
    }
}

impl Render for TransitionDefaults {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let transition = self.transition(cx);
        let active = mode(&transition);
        if self.prop_id.is_some() {
            return self.picker_panel(active, window, cx).into_any_element();
        }
        if let Some(stored) = transition["durationSeconds"]
            .as_f64()
            .filter(|_| active != Mode::Cut)
        {
            if (stored - seconds_of(&self.duration, cx)).abs() > 0.05 {
                self.duration
                    .update(cx, |slider, cx| slider.set_value(stored as f32, window, cx));
            }
        }
        let id = match (&self.item_id, self.kind) {
            (Some(_), _) => "item",
            (None, Kind::Slide) => "slide",
            (None, Kind::Media) => "media",
            (None, Kind::Props) => "props",
        };
        let owner = cx.weak_entity();
        let seconds = seconds_of(&self.duration, cx);
        let segmented = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.metrics.space_xs)
            .child(
                div()
                    .flex_none()
                    .debug_selector(move || format!("transition-mode-{id}"))
                    .child(
                        SegmentedControl::new(
                            format!("transition-default-{id}"),
                            t.t("transition.default.label"),
                            vec![
                                SegmentedOption::icon(
                                    Mode::Cut,
                                    "columns-2",
                                    t.t("transition.default.cut"),
                                ),
                                SegmentedOption::icon(
                                    Mode::Fade,
                                    "square-half",
                                    t.t("transition.default.fade"),
                                ),
                                SegmentedOption::icon(
                                    Mode::Other,
                                    "transition-right",
                                    t.t("transition.default.other"),
                                ),
                            ],
                            active,
                        )
                        .on_change({
                            let owner = owner.clone();
                            move |mode, window, cx| {
                                owner
                                    .update(cx, |this, cx| match mode {
                                        Mode::Other => this.toggle_picker(window, cx),
                                        Mode::Cut => {
                                            this.close_picker(cx);
                                            this.save(Value::Null, cx);
                                        }
                                        Mode::Fade => {
                                            this.close_picker(cx);
                                            let seconds = seconds_of(&this.duration, cx);
                                            this.save(fade(seconds), cx);
                                        }
                                    })
                                    .ok();
                            }
                        }),
                    ),
            )
            .when(shows_duration(active), |this| {
                this.child(
                    div()
                        .debug_selector(|| "transition-duration-slider".to_string())
                        .flex_none()
                        .w(theme.metrics.control_height * 2.)
                        .child(
                            RangeInput::new(&self.duration)
                                .label(t.t("transition.default.duration")),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .min_w(theme.metrics.space_lg * 2.)
                        .text_size(theme.metrics.font_sm)
                        .text_color(theme.colors.text_muted)
                        .child(format!("{seconds:.1}s")),
                )
            });
        let controls = if self.item_id.is_some() {
            let inherited = transition.is_null();
            let effect = transition["effectId"].as_str().unwrap_or_default();
            let label = if inherited {
                t.t("transition.presentation.defaultShort")
            } else if active == Mode::Cut {
                t.t("transition.default.cut")
            } else {
                self.effects
                    .iter()
                    .find(|known| known.id.eq_ignore_ascii_case(effect))
                    .map_or_else(|| effect.to_string(), |known| known.name.to_string())
            };
            let button = ActionButton::new(
                "cue-transition",
                if self.inspector {
                    t.t("gpui.editor.chooseTransition")
                } else {
                    label.clone()
                },
            )
            .icon("transition-right")
            .variant(ButtonVariant::Ghost)
            .compact()
            .selected(!inherited)
            .on_click({
                let owner = owner.clone();
                move |_, window, cx| {
                    owner
                        .update(cx, |this, cx| this.toggle_picker(window, cx))
                        .ok();
                }
            });
            if self.inspector {
                let controls = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(theme.metrics.space_xs)
                    .when(self.slide_id.is_some(), |this| this.child(label))
                    .child(button);
                if self.slide_id.is_some() {
                    let image = self
                        .picker
                        .frames
                        .get(effect)
                        .and_then(|frames| frames.get(frames.len() / 2))
                        .cloned();
                    div()
                        .flex()
                        .w_full()
                        .gap(theme.metrics.space_sm)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .aspect_ratio(16. / 9.)
                                .bg(theme.colors.preview_matte_bg)
                                .children(image.map(|image| {
                                    gpui::img(image)
                                        .w_full()
                                        .object_fit(gpui::ObjectFit::Contain)
                                })),
                        )
                        .child(controls)
                        .into_any_element()
                } else {
                    controls.into_any_element()
                }
            } else {
                button.into_any_element()
            }
        } else {
            segmented.into_any_element()
        };
        let mut popover = Popover::new(format!("transition-popover-{id}"), controls)
            .anchor(if self.item_id.is_some() {
                Anchor::TopLeft
            } else {
                Anchor::BottomLeft
            })
            .on_dismiss(move |_, _, cx| {
                owner.update(cx, |this, cx| this.close_picker(cx)).ok();
            });
        if self.picker.open {
            popover = popover.content(self.picker_panel(active, window, cx));
        }
        div()
            .debug_selector(move || format!("transition-controls-{id}"))
            .flex_none()
            .when(self.inspector, |this| this.w_full())
            .child(popover)
            .into_any_element()
    }
}
