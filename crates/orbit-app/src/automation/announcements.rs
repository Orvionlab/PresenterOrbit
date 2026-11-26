//! The announcements player (`announcements-controller.ts`): an independent presentation with its own slide index,
//! timed rotation, and clock, shown on the screens whose destination is announcements. The player is
//! `live["announcements"]`, which is also what the scene reads (`item`, `slide`, `selectionId`, `clockRevision`,
//! `paused`, `currentTime`, `capturedAt`); the `player` object beside them is the part only this module reads.
//! Every function takes the time and the new ids, so the clocks are testable.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{div, prelude::*, Context, SharedString, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant, CheckboxField, Dialog};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::form::{get, Form, Kind, Spec};
use crate::state::runtime::Runtime;

const MAX_INTERVAL_SECONDS: f64 = 86_400.;

pub fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0., |elapsed| elapsed.as_millis() as f64)
}

/// A clock revision no earlier one equals, seeded from the time like React's.
pub fn next_revision() -> u64 {
    static CLOCK: AtomicU64 = AtomicU64::new(0);
    let mut seed = now_ms() as u64;
    loop {
        let current = CLOCK.load(Ordering::Relaxed);
        seed = seed.max(current + 1);
        if CLOCK
            .compare_exchange(current, seed, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return seed;
        }
    }
}

fn enabled(cues: &[Value]) -> Vec<Value> {
    cues.iter()
        .filter(|slide| slide["disabled"] != true)
        .cloned()
        .collect()
}

/// `announcementsElapsed`.
pub fn elapsed(player: &Value, now: f64) -> f64 {
    let state = &player["player"];
    state["elapsedMs"].as_f64().unwrap_or(0.)
        + state["startedAt"]
            .as_f64()
            .map_or(0., |started| (now - started).max(0.))
}

fn playing(player: &Value) -> bool {
    player["player"]["status"] == "playing"
}

/// The scene-facing fields follow the player state.
fn project(player: &mut Value, now: f64) {
    let index = player["player"]["index"].as_u64().unwrap_or(0) as usize;
    player["slide"] = player["item"]["slides"][index].clone();
    player["paused"] = json!(!playing(player));
    player["currentTime"] = json!(elapsed(player, now) / 1000.);
    player["capturedAt"] = json!(now);
}

/// `selectAnnouncements`: the enabled cues of the presentation, starting at the first, playing.
pub fn select(
    item: &Value,
    cues: &[Value],
    interval_seconds: Option<f64>,
    looped: bool,
    now: f64,
    selection_id: String,
    revision: u64,
) -> Result<Value, String> {
    if interval_seconds.is_some_and(|seconds| {
        !seconds.is_finite() || !(1. ..=MAX_INTERVAL_SECONDS).contains(&seconds)
    }) {
        return Err("Announcements interval must be between 1 and 86400 seconds.".into());
    }
    if interval_seconds.is_none() && looped {
        return Err("Manual announcement cues cannot loop automatically.".into());
    }
    let slides = enabled(cues);
    if slides.is_empty() {
        return Err("The presentation has no enabled slides.".into());
    }
    Ok(start_player(
        item,
        slides,
        interval_seconds,
        looped,
        now,
        selection_id,
        revision,
    ))
}

fn start_player(
    item: &Value,
    slides: Vec<Value>,
    interval_seconds: Option<f64>,
    looped: bool,
    now: f64,
    selection_id: String,
    revision: u64,
) -> Value {
    let mut shown = item.clone();
    shown["slides"] = Value::Array(slides);
    let mut player = json!({
        "item": shown,
        "selectionId": selection_id,
        "clockRevision": revision,
        "player": {
            "index": 0, "status": "playing", "elapsedMs": 0, "startedAt": now,
            "intervalSeconds": interval_seconds, "loop": looped,
        },
    });
    project(&mut player, now);
    player
}

/// The announcements player an action starts: manual, on the slide the action names.
pub fn select_slide(
    item: &Value,
    cues: &[Value],
    slide_id: &str,
    instance: &Value,
    now: f64,
    selection_id: String,
    revision: u64,
) -> Result<Value, String> {
    select_slide_from(
        item,
        enabled(cues),
        slide_id,
        instance,
        now,
        selection_id,
        revision,
    )
}

pub fn select_operator_slide(
    item: &Value,
    cues: &[Value],
    slide_id: &str,
    instance: &Value,
    now: f64,
    selection_id: String,
    revision: u64,
) -> Result<Value, String> {
    select_slide_from(
        item,
        cues.to_vec(),
        slide_id,
        instance,
        now,
        selection_id,
        revision,
    )
}

fn select_slide_from(
    item: &Value,
    slides: Vec<Value>,
    slide_id: &str,
    instance: &Value,
    now: f64,
    selection_id: String,
    revision: u64,
) -> Result<Value, String> {
    let index = slides
        .iter()
        .position(|slide| slide["id"] == slide_id && slide["arrangementInstanceId"] == *instance)
        .ok_or("automation.macros.targetInvalid")?;
    let mut player = start_player(item, slides, None, false, now, selection_id, revision);
    player["player"]["index"] = json!(index);
    project(&mut player, now);
    Ok(player)
}

/// `navigateAnnouncements`: one slide on or back. Past an end without looping it freezes there, paused.
pub fn navigate(
    player: &Value,
    direction: i64,
    now: f64,
    selection_id: String,
    revision: u64,
) -> Value {
    let slides = player["item"]["slides"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let count = slides.len() as i64;
    let current = player["player"]["index"].as_i64().unwrap_or(0);
    let looped = player["player"]["loop"] == true;
    let target = (1..=count)
        .map(|step| current + step * direction)
        .take_while(|&index| looped || (0..count).contains(&index))
        .map(|index| index.rem_euclid(count) as usize)
        .find(|&index| slides[index]["disabled"] != true);
    let mut next = player.clone();
    if let Some(index) = target {
        next["player"]["index"] = json!(index);
        next["player"]["elapsedMs"] = json!(0);
        next["player"]["startedAt"] = if playing(player) {
            json!(now)
        } else {
            Value::Null
        };
        next["selectionId"] = json!(selection_id);
        next["clockRevision"] = json!(revision);
    } else {
        next["player"]["status"] = json!("paused");
        next["player"]["elapsedMs"] = json!(elapsed(player, now));
        next["player"]["startedAt"] = Value::Null;
    }
    project(&mut next, now);
    next
}

/// `toggleAnnouncements`.
pub fn toggle(player: &Value, now: f64, revision: u64) -> Value {
    let was_playing = playing(player);
    let mut next = player.clone();
    next["player"]["status"] = json!(if was_playing { "paused" } else { "playing" });
    next["clockRevision"] = json!(revision);
    next["player"]["elapsedMs"] = json!(elapsed(player, now));
    next["player"]["startedAt"] = if was_playing { Value::Null } else { json!(now) };
    project(&mut next, now);
    next
}

/// Whether the timed rotation is due to advance.
pub fn due(player: &Value, now: f64) -> bool {
    playing(player)
        && player["player"]["intervalSeconds"]
            .as_f64()
            .is_some_and(|seconds| elapsed(player, now) >= seconds * 1000.)
}

/// Whether the player rotates by itself and so needs the clock tick.
pub fn timed(player: &Value) -> bool {
    playing(player) && player["player"]["intervalSeconds"].is_number()
}

// ----- the dialog -----

/// Advances a timed player whenever its interval has passed, for as long as one plays.
pub fn tick(runtime: &Runtime, cx: &mut gpui::App) {
    let runtime = runtime.clone();
    cx.spawn(async move |cx| loop {
        cx.background_executor()
            .timer(Duration::from_millis(100))
            .await;
        let keep = cx.update(|cx| {
            let timed_now = timed(&runtime.output.read(cx).live["announcements"]);
            if timed_now {
                let now = now_ms();
                runtime.output.update(cx, |output, cx| {
                    let player = output.live["announcements"].clone();
                    if due(&player, now) {
                        let next = navigate(
                            &player,
                            1,
                            now,
                            uuid::Uuid::new_v4().to_string(),
                            next_revision(),
                        );
                        output.edit_live(|live| live["announcements"] = next);
                        cx.notify();
                    }
                });
            }
            timed_now
        });
        if !keep {
            break;
        }
    })
    .detach();
}

pub struct Announcements {
    runtime: Runtime,
    form: Form,
    status: String,
    on_close: std::rc::Rc<dyn Fn(&mut gpui::App)>,
}

fn form_of(dialog: &mut Announcements) -> &mut Form {
    &mut dialog.form
}

impl Announcements {
    /// `item_id` preselects the presentation, as the cue editor's entry does.
    pub fn new(
        runtime: Runtime,
        item_id: &str,
        on_close: impl Fn(&mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&runtime.output, |_, _, cx| cx.notify()).detach();
        cx.observe(&runtime.content, |_, _, cx| cx.notify())
            .detach();
        let mut form = Form::default();
        form.reset(json!({"itemId": item_id, "seconds": 10, "loop": true}));
        Self {
            runtime,
            form,
            status: String::new(),
            on_close: std::rc::Rc::new(on_close),
        }
    }

    fn edit(&mut self, change: impl FnOnce(&Value, f64) -> Value, cx: &mut Context<Self>) {
        let now = now_ms();
        self.runtime.output.update(cx, |output, cx| {
            let player = output.live["announcements"].clone();
            if player.is_null() {
                return;
            }
            let next = change(&player, now);
            output.edit_live(|live| live["announcements"] = next);
            cx.notify();
        });
        tick(&self.runtime, cx);
    }

    /// `startAnnouncements`.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        let item_id = get(&self.form.draft, "itemId")
            .as_str()
            .unwrap_or("")
            .to_string();
        let seconds = get(&self.form.draft, "seconds").as_f64();
        let looped = get(&self.form.draft, "loop") == &json!(true);
        let content = self.runtime.content.read(cx);
        let Some(item) = content
            .items
            .get(&item_id)
            .map(|item| (*item.value).clone())
        else {
            self.status = "announcements.missing".into();
            cx.notify();
            return;
        };
        let arrangement = crate::cues::model::selected_arrangement_id(&item).to_string();
        let started =
            orbit_host::Host::presentation_cues(&item, &arrangement, None).and_then(|cues| {
                select(
                    &item,
                    &cues,
                    seconds,
                    looped,
                    now_ms(),
                    uuid::Uuid::new_v4().to_string(),
                    next_revision(),
                )
            });
        match started {
            Ok(player) => {
                self.status.clear();
                self.runtime.output.update(cx, |output, cx| {
                    output.edit_live(|live| live["announcements"] = player);
                    cx.notify();
                });
                tick(&self.runtime, cx);
            }
            Err(error) => self.status = error,
        }
        cx.notify();
    }

    /// `updateScreenOutput` for the routing: a screen shows announcements or the main presentation.
    fn route(&mut self, screen_id: &str, on: bool, cx: &mut Context<Self>) {
        let screens = self.runtime.content.read(cx).workspace["screenConfig"].clone();
        let screens = if screens
            .as_object()
            .is_some_and(|screens| !screens.is_empty())
        {
            screens
        } else {
            self.runtime.output.read(cx).settings["screenConfig"].clone()
        };
        let Some(current) = screens.get(screen_id) else {
            return;
        };
        let mut layers = current["lookLayers"].clone();
        if !layers.is_object() {
            layers = json!({});
        }
        layers["announcements"] = json!(true);
        let patch = json!({"contentDestination": if on { "announcements" } else { "main" }, "lookLayers": layers});
        let mutation = crate::settings::video::update_mutation(screen_id, current, patch);
        crate::output::windows::mutate_config(&self.runtime, mutation, cx);
    }
}

impl Render for Announcements {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t: Translator = Runtime::translator(cx);
        let gap = cx.theme().metrics.space_xs;
        let secondary = ButtonVariant::Secondary;
        let content = self.runtime.content.read(cx);
        let mut library: Vec<(String, String)> = content
            .items
            .iter()
            .map(|item| (item.id.clone(), item.title.clone()))
            .collect();
        library.sort_by_key(|a| a.1.to_lowercase());
        let player = self.runtime.output.read(cx).live["announcements"].clone();
        let screens = {
            let local = content.workspace["screenConfig"].clone();
            if local.as_object().is_some_and(|screens| !screens.is_empty()) {
                local
            } else {
                self.runtime.output.read(cx).settings["screenConfig"].clone()
            }
        };
        let mut choices = vec![(String::new(), t.t("common.none"))];
        choices.extend(library);
        let specs = [
            Spec::new(
                "itemId",
                t.t("announcements.presentation"),
                Kind::Choice(choices),
            ),
            Spec::new("seconds", t.t("announcements.seconds"), Kind::Number),
            Spec::new("loop", t.t("announcements.loop"), Kind::Check),
        ];
        let mut body = div()
            .flex()
            .flex_col()
            .gap(gap)
            .child(div().child(t.t("announcements.help")));
        for spec in &specs {
            body = body.child(self.form.field("announcements", spec, form_of, window, cx));
        }
        let can_start = get(&self.form.draft, "itemId")
            .as_str()
            .is_some_and(|id| !id.is_empty());
        body = body.child(
            ActionButton::new("announcements-start", t.t("announcements.start"))
                .disabled(!can_start)
                .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
        );
        if !self.status.is_empty() {
            body = body.child(
                div()
                    .id("announcements-status")
                    .role(gpui::Role::Alert)
                    .child(t.t(&self.status)),
            );
        }
        if !player.is_null() {
            let count = player["item"]["slides"].as_array().map_or(0, Vec::len);
            let status = if playing(&player) {
                "announcements.playing"
            } else {
                "announcements.paused"
            };
            let nav = |id: &'static str, label: String, direction: i64, cx: &mut Context<Self>| {
                ActionButton::new(id, label)
                    .variant(secondary)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit(
                            |player, now| {
                                navigate(
                                    player,
                                    direction,
                                    now,
                                    uuid::Uuid::new_v4().to_string(),
                                    next_revision(),
                                )
                            },
                            cx,
                        )
                    }))
            };
            body = body
                .child(div().child(format!(
                    "{} · {}/{} · {}",
                    player["item"]["title"].as_str().unwrap_or(""),
                    player["player"]["index"].as_u64().unwrap_or(0) + 1,
                    count,
                    t.t(status)
                )))
                .child(
                    div()
                        .flex()
                        .gap(gap)
                        .child(nav(
                            "announcements-previous",
                            t.t("presentation.command.previousSlide"),
                            -1,
                            cx,
                        ))
                        .child(
                            ActionButton::new(
                                "announcements-toggle",
                                t.t(if playing(&player) {
                                    "transport.pause"
                                } else {
                                    "transport.play"
                                }),
                            )
                            .variant(secondary)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.edit(|player, now| toggle(player, now, next_revision()), cx)
                            })),
                        )
                        .child(nav(
                            "announcements-next",
                            t.t("presentation.command.nextSlide"),
                            1,
                            cx,
                        ))
                        .child(
                            ActionButton::new(
                                "announcements-clear",
                                t.t("presentation.command.clearAnnouncements"),
                            )
                            .variant(secondary)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.runtime.output.update(cx, |output, cx| {
                                    output.edit_live(|live| live["announcements"] = Value::Null);
                                    cx.notify();
                                });
                            })),
                        ),
                );
        }
        body = body.child(div().child(t.t("announcements.routing")));
        for (id, config) in screens
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, config)| config["type"] != "stage")
        {
            let (weak, screen) = (cx.weak_entity(), id.clone());
            body = body.child(
                CheckboxField::new(
                    SharedString::from(format!("announcements-route-{id}")),
                    config["label"].as_str().unwrap_or(id).to_string(),
                    config["contentDestination"] == "announcements",
                )
                .on_change(move |on, _, cx| {
                    let screen = screen.clone();
                    let _ = weak.update(cx, |this, cx| this.route(&screen, on, cx));
                }),
            );
        }
        let close = self.on_close.clone();
        Dialog::new(
            "announcements",
            t.t("utilityPanels.announcements"),
            t.t("common.close"),
            move |_, _, cx| close(cx),
        )
        .width(gpui::px(480.))
        .child(body)
    }
}
