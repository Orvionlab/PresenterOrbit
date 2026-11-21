//! The slide header's auto advance and destination: both live on the presentation, auto advance as the slides' own
//! completion fields (the runtime that advances live slides already reads them) and the destination as an item field.

use super::actions;
use crate::keys::{format_shortcut_label, KeyMappings};
use crate::state::runtime::Runtime;
use gpui::{div, prelude::*, App, Bounds, ClickEvent, Context, Entity, Pixels, Point, Window};
use orbit_ui::controls::{
    input_state, ActionButton, ButtonVariant, ContextMenu, Dialog, Field, IconButton, InputEvent,
    InputState, MenuEntry, MenuItem, NumberInput,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};
use std::{cell::Cell, rc::Rc};

const COMPLETION_FIELDS: [&str; 5] = [
    "completionAction",
    "completionTimeSeconds",
    "completionTargetType",
    "completionTargetCueId",
    "completionTargetInstanceId",
];

type TriggerBounds = Rc<Cell<Bounds<Pixels>>>;

/// Wraps a header trigger and records its bounds, so its menu drops down from the bottom-left corner.
fn track(trigger: impl IntoElement, bounds: &TriggerBounds) -> gpui::Div {
    let bounds = bounds.clone();
    // The inner wrapper hugs the button; the outer one may be stretched by the header row.
    div().flex_none().child(
        div().relative().child(trigger).child(
            gpui::canvas(move |area, _, _| bounds.set(area), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        ),
    )
}

pub const ANNOUNCEMENTS: &str = "announcements";

pub fn announces(item: &Value) -> bool {
    item["destination"] == ANNOUNCEMENTS
}

pub const PRESENTATION: &str = "presentation";

pub fn set_destination(destination: &str) -> Value {
    json!({"type":"setDestination","destination":destination})
}

/// The local value after `setDestination`: presentation is the absence of the field, as the host stores it.
fn with_destination(item: &mut Value, destination: &str) {
    if let Some(fields) = item.as_object_mut() {
        if destination == ANNOUNCEMENTS {
            fields.insert("destination".into(), json!(ANNOUNCEMENTS));
        } else {
            fields.remove("destination");
        }
    }
}

pub fn open_looks(runtime: &Runtime, cx: &mut App) {
    let (host, runtime) = (runtime.host.clone(), runtime.clone());
    cx.spawn(async move |cx| {
        let request = json!({"kind":"settings","panel":crate::settings::LOOKS_PANEL});
        let result = cx
            .background_spawn(async move { host.invoke("openAuxiliaryWindow", vec![request]) })
            .await;
        if let Err(message) = result {
            cx.update(|cx| actions::error(&runtime, cx, message));
        }
    })
    .detach();
}

fn timed(slide: &Value) -> bool {
    slide["completionAction"]
        .as_str()
        .is_some_and(|action| action.ends_with("AFTER_TIME"))
}

fn eligible(slide: &Value) -> bool {
    slide["disabled"] != true && slide["completionAction"] != "ON_MEDIA_END"
}

/// The seconds and loop of a presentation whose slides advance by themselves: the first enabled slide's delay, and
/// whether the last one returns to the first.
pub fn read(slides: &[Value]) -> Option<(f64, bool)> {
    let mut eligible = slides.iter().filter(|slide| eligible(slide));
    let first = eligible.next()?;
    if !timed(first) {
        return None;
    }
    let last = eligible.next_back().unwrap_or(first);
    Some((
        first["completionTimeSeconds"].as_f64()?,
        timed(last) && last["completionTargetType"] == "FIRST",
    ))
}

/// The slides with auto advance set (`Some((seconds, loop))`) or removed; media-end and disabled slides are kept.
pub fn apply(slides: &[Value], advance: Option<(f64, bool)>) -> Vec<Value> {
    let last = slides.iter().rposition(eligible);
    slides
        .iter()
        .enumerate()
        .map(|(index, slide)| {
            if !eligible(slide) {
                return slide.clone();
            }
            let mut slide = slide.clone();
            if let Some(fields) = slide.as_object_mut() {
                for key in COMPLETION_FIELDS {
                    fields.remove(key);
                }
                if let Some((seconds, looped)) = advance {
                    fields.insert("completionAction".into(), json!("AFTER_TIME"));
                    fields.insert("completionTimeSeconds".into(), json!(seconds));
                    let target = if looped && Some(index) == last {
                        "FIRST"
                    } else {
                        "NEXT"
                    };
                    fields.insert("completionTargetType".into(), json!(target));
                }
            }
            slide
        })
        .collect()
}

/// The player settings an announcement-destination presentation starts with: its auto advance, else manual.
pub fn player_timing(item: &Value) -> (Option<f64>, bool) {
    match item["slides"].as_array().and_then(|slides| read(slides)) {
        Some((seconds, looped)) => (Some(seconds), looped),
        None => (None, false),
    }
}

/// The delays the header menu offers besides none and a custom value.
pub const PRESETS: [f64; 4] = [5., 10., 15., 20.];

pub fn seconds_label(seconds: f64) -> String {
    format!("{seconds}s")
}

/// The seconds a custom entry asks for; `None` unless they are a finite number above zero.
pub fn parse_seconds(text: &str) -> Option<f64> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds > 0.)
}

/// The slides after a menu choice: every enabled slide goes to the next one after `seconds` (`None` removes the
/// timers). A presentation that already returns to its first slide after the last one keeps doing so.
pub fn choose(slides: &[Value], seconds: Option<f64>) -> Vec<Value> {
    let looped = read(slides).is_some_and(|(_, looped)| looped);
    apply(slides, seconds.map(|seconds| (seconds, looped)))
}

/// The slide header's target button: the layer the presentation plays on, and the way to the Looks.
pub struct Destination {
    runtime: Runtime,
    pub item_id: String,
    menu: Option<Point<Pixels>>,
    bounds: TriggerBounds,
}

impl Destination {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        cx.observe(&runtime.content, |_, _, cx| cx.notify())
            .detach();
        Self {
            runtime,
            item_id: String::new(),
            menu: None,
            bounds: TriggerBounds::default(),
        }
    }

    fn announcing(&self, cx: &Context<Self>) -> bool {
        let content = self.runtime.content.read(cx);
        content
            .items
            .get(&self.item_id)
            .is_some_and(|item| announces(&item.value))
    }

    fn choose(&mut self, destination: &str, cx: &mut Context<Self>) {
        self.menu = None;
        let destination = destination.to_string();
        actions::mutate_optimistic(
            &self.runtime,
            &self.item_id.clone(),
            set_destination(&destination),
            move |item| with_destination(item, &destination),
            cx,
        );
        cx.notify();
    }

    fn menu_entries(&self, announcing: bool, cx: &mut Context<Self>) -> Vec<MenuEntry> {
        let t = *cx.global::<Translator>();
        let owner = cx.weak_entity();
        let choice = |label: String, destination: &'static str| -> MenuEntry {
            let owner = owner.clone();
            MenuItem::new(label)
                .checked((destination == ANNOUNCEMENTS) == announcing)
                .on_click(move |_, _, cx| {
                    owner
                        .update(cx, |this, cx| this.choose(destination, cx))
                        .ok();
                })
                .into()
        };
        let runtime = self.runtime.clone();
        let shortcut = KeyMappings::from_settings(&self.runtime.output.read(cx).settings)
            .shortcut("openLooks")
            .map(format_shortcut_label);
        vec![
            choice(t.t("cue.destination.presentation"), PRESENTATION),
            choice(t.t("cue.destination.announcements"), ANNOUNCEMENTS),
            MenuEntry::Separator,
            {
                let mut item = MenuItem::new(t.t("cue.destination.editLooks"))
                    .on_click(move |_, _, cx| open_looks(&runtime, cx));
                if let Some(shortcut) = shortcut {
                    item = item.meta(shortcut);
                }
                item.into()
            },
        ]
    }
}

impl Render for Destination {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let owner = cx.weak_entity();
        let announcing = self.announcing(cx);
        let option = t.t(if announcing {
            "cue.destination.announcements"
        } else {
            "cue.destination.presentation"
        });
        let trigger = IconButton::new(
            "cue-destination",
            "target",
            format!("{}: {option}", t.t("cue.destination")),
        )
        .selected(announcing)
        .on_click({
            let owner = owner.clone();
            let bounds = self.bounds.clone();
            move |_, _, cx| {
                let position = bounds.get().bottom_left();
                owner
                    .update(cx, |this, cx| {
                        this.menu = Some(position);
                        cx.notify();
                    })
                    .ok();
            }
        });
        let menu = self.menu.map(|position| {
            ContextMenu::new(
                "cue-destination-menu",
                position,
                self.menu_entries(announcing, cx),
                move |_, _, cx| {
                    owner
                        .update(cx, |this, cx| {
                            this.menu = None;
                            cx.notify();
                        })
                        .ok();
                },
            )
        });
        track(trigger, &self.bounds).children(menu)
    }
}

pub struct AutoAdvance {
    runtime: Runtime,
    pub item_id: String,
    menu: Option<Point<Pixels>>,
    bounds: TriggerBounds,
    custom: bool,
    focus_seconds: bool,
    seconds: Entity<InputState>,
}

impl AutoAdvance {
    pub fn new(runtime: Runtime, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&runtime.content, |_, _, cx| cx.notify())
            .detach();
        let seconds = input_state("5", window, cx);
        cx.subscribe_in(&seconds, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_custom(cx);
            }
        })
        .detach();
        Self {
            runtime,
            item_id: String::new(),
            menu: None,
            bounds: TriggerBounds::default(),
            custom: false,
            focus_seconds: false,
            seconds,
        }
    }

    fn current(&self, cx: &Context<Self>) -> Option<f64> {
        let content = self.runtime.content.read(cx);
        let item = content.items.get(&self.item_id)?;
        read(item.value["slides"].as_array()?).map(|(seconds, _)| seconds)
    }

    fn choose(&mut self, seconds: Option<f64>, cx: &mut Context<Self>) {
        self.menu = None;
        self.custom = false;
        let content = self.runtime.content.read(cx);
        if let Some(item) = content.items.get(&self.item_id) {
            if let Some(slides) = item.value["slides"].as_array() {
                let slides = choose(slides, seconds);
                let operation =
                    json!({"type":"setSlideDocument","title":item.value["title"],"slides":slides});
                actions::mutate_optimistic(
                    &self.runtime,
                    &self.item_id.clone(),
                    operation,
                    move |item| item["slides"] = Value::Array(slides),
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn open_custom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let seconds = self.current(cx).unwrap_or(5.);
        self.menu = None;
        self.custom = true;
        self.focus_seconds = true;
        self.seconds.update(cx, |state, cx| {
            state.set_value(seconds.to_string(), window, cx);
        });
        cx.notify();
    }

    fn submit_custom(&mut self, cx: &mut Context<Self>) {
        match parse_seconds(&self.seconds.read(cx).value()) {
            Some(seconds) => self.choose(Some(seconds), cx),
            None => {
                let message = cx.global::<Translator>().t("cue.autoAdvance.invalid");
                actions::error(&self.runtime, cx, message);
            }
        }
    }

    fn menu_entries(&self, current: Option<f64>, cx: &mut Context<Self>) -> Vec<MenuEntry> {
        let t = *cx.global::<Translator>();
        let owner = cx.weak_entity();
        let choice = |label: String, checked: bool, seconds: Option<f64>| {
            let owner = owner.clone();
            MenuItem::new(label)
                .checked(checked)
                .on_click(move |_, _, cx| {
                    owner.update(cx, |this, cx| this.choose(seconds, cx)).ok();
                })
                .into()
        };
        let mut entries: Vec<MenuEntry> =
            vec![choice(t.t("cue.autoAdvance.none"), current.is_none(), None)];
        for seconds in PRESETS {
            entries.push(choice(
                seconds_label(seconds),
                current == Some(seconds),
                Some(seconds),
            ));
        }
        let custom = owner.clone();
        entries.push(
            MenuItem::new(t.t("cue.autoAdvance.custom"))
                .checked(current.is_some_and(|seconds| !PRESETS.contains(&seconds)))
                .on_click(move |_, window, cx| {
                    custom
                        .update(cx, |this, cx| this.open_custom(window, cx))
                        .ok();
                })
                .into(),
        );
        entries
    }
}

impl Render for AutoAdvance {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let owner = cx.weak_entity();
        let current = self.current(cx);
        let open_menu = {
            let owner = owner.clone();
            let bounds = self.bounds.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let position = bounds.get().bottom_left();
                owner
                    .update(cx, |this, cx| {
                        this.custom = false;
                        this.menu = Some(position);
                        cx.notify();
                    })
                    .ok();
            }
        };
        let trigger = match current {
            Some(seconds) => ActionButton::new("cue-auto-advance", seconds_label(seconds))
                .icon("clock-play")
                .variant(ButtonVariant::Ghost)
                .compact()
                .selected(true)
                .on_click(open_menu)
                .into_any_element(),
            None => IconButton::new("cue-auto-advance", "clock-play", t.t("cue.autoAdvance"))
                .on_click(open_menu)
                .into_any_element(),
        };
        let dialog = self.custom.then(|| {
            if std::mem::take(&mut self.focus_seconds) {
                let seconds = self.seconds.clone();
                // The dialog takes focus in its own deferred step on opening; focus the field after it.
                window.defer(cx, move |window, cx| {
                    window.defer(cx, move |window, cx| {
                        seconds.update(cx, |state, cx| {
                            state.focus(window, cx);
                            state.select_all(window, cx);
                        });
                    });
                });
            }
            let (cancel, close, submit) = (owner.clone(), owner.clone(), owner.clone());
            Dialog::new(
                "cue-auto-advance-dialog",
                t.t("cue.autoAdvance.custom"),
                t.t("common.close"),
                move |_, _, cx| {
                    close
                        .update(cx, |this, cx| {
                            this.custom = false;
                            cx.notify();
                        })
                        .ok();
                },
            )
            .width(theme.metrics.control_height * 10.)
            .child(Field::new(
                t.t("cue.autoAdvance.seconds"),
                NumberInput::new("cue-auto-advance-seconds", &self.seconds)
                    .label(t.t("cue.autoAdvance.seconds"))
                    .step(0.5)
                    .range(0.5, 86_400.)
                    .stepper(),
            ))
            .footer(
                ActionButton::new("cue-auto-advance-cancel", t.t("common.cancel"))
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        cancel
                            .update(cx, |this, cx| {
                                this.custom = false;
                                cx.notify();
                            })
                            .ok();
                    }),
            )
            .footer(
                ActionButton::new("cue-auto-advance-set", t.t("common.ok")).on_click(
                    move |_, _, cx| {
                        submit.update(cx, |this, cx| this.submit_custom(cx)).ok();
                    },
                ),
            )
        });
        let menu = self.menu.map(|position| {
            let close = owner.clone();
            ContextMenu::new(
                "cue-auto-advance-menu",
                position,
                self.menu_entries(current, cx),
                move |_, _, cx| {
                    close
                        .update(cx, |this, cx| {
                            this.menu = None;
                            cx.notify();
                        })
                        .ok();
                },
            )
        });
        track(trigger, &self.bounds).children(menu).children(dialog)
    }
}
