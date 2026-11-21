use gpui::{div, prelude::*, App, Context, Entity, Render, Subscription, Window};
use orbit_ui::controls::{
    input_state, CheckboxField, ColorPicker, Field, InputState, Select, SelectOption, TextInput,
    DEFAULT_HEADER_COLOR,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::model::Entry;
use crate::automation::{
    editors::timer_configuration_specs,
    form::{ClockPeriod, Form, Kind as FieldKind},
};
use crate::state::runtime::{Kind, Persist, Runtime};

const COMMANDS: [(&str, &str); 3] = [
    ("start", "timer.start"),
    ("stop", "playlistHeader.stop"),
    ("reset", "timer.reset"),
];

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Request {
    pub playlist: String,
    pub entry: String,
}

pub fn toggle(runtime: &Runtime, cx: &mut App, request: Request) {
    runtime.ui.update(cx, |ui, cx| {
        ui.close_menus();
        ui.header_editor = (ui.header_editor.as_ref() != Some(&request)).then_some(request);
        cx.notify();
    });
}

pub fn close(runtime: &Runtime, cx: &mut App, request: &Request) {
    runtime.ui.update(cx, |ui, cx| {
        if ui.header_editor.as_ref() == Some(request) {
            ui.header_editor = None;
            cx.notify();
        }
    });
}

pub struct Editor {
    pub request: Request,
    runtime: Runtime,
    title: Entity<InputState>,
    color: String,
    timer: String,
    command: String,
    configuration: Form,
    apply_configuration: bool,
    submitted: Option<Value>,
    _subscriptions: [Subscription; 2],
}

impl Editor {
    pub fn new(
        runtime: Runtime,
        request: Request,
        entry: &Entry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let timer = entry.raw["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|action| action["type"] == "timer");
        let timer_text = |key: &str, default: &str| {
            timer
                .and_then(|action| action[key].as_str())
                .unwrap_or(default)
                .to_string()
        };
        let title = input_state(entry.title.clone(), window, cx);
        let stored_configuration = timer
            .map(|action| &action["details"]["configuration"])
            .filter(|value| value.is_object());
        let mut configuration = Form::default();
        if let Some(value) = stored_configuration {
            configuration.reset(value.clone());
        }
        let mut editor = Self {
            runtime,
            request,
            title: title.clone(),
            color: if entry.color.is_empty() {
                DEFAULT_HEADER_COLOR.into()
            } else {
                entry.color.clone()
            },
            timer: timer_text("targetId", ""),
            command: timer_text("value", "start"),
            configuration,
            apply_configuration: stored_configuration.is_some(),
            submitted: None,
            _subscriptions: [
                cx.observe(&title, |_, _, cx| cx.notify()),
                cx.observe_self(|editor, cx| editor.save(cx)),
            ],
        };
        editor.submitted = editor.mutation(cx);
        editor
    }

    fn mutation(&self, cx: &App) -> Option<Value> {
        let title = self.title.read(cx).value();
        let title = title.trim();
        if title.is_empty() || !self.command_supported() {
            return None;
        }
        let configuration = if self.apply_configuration && !self.timer.is_empty() {
            timer_configuration(&self.configuration, cx)?
        } else {
            Value::Null
        };
        Some(json!({
            "type":"setHeader", "playlistId":self.request.playlist, "entryId":self.request.entry,
            "title":title, "color":self.color, "timerId":self.timer, "timerCommand":self.command,
            "timerConfiguration":configuration
        }))
    }

    fn command_supported(&self) -> bool {
        COMMANDS.iter().any(|(value, _)| *value == self.command)
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(mutation) = self
            .mutation(cx)
            .filter(|value| Some(value) != self.submitted.as_ref())
        else {
            return;
        };
        self.submitted = Some(mutation.clone());
        let request = Persist::new(Kind::Playlist, mutation, |_, _| {});
        let owner = cx.weak_entity();
        let submitted = self.submitted.clone();
        super::tree_actions::spawn_persist(&self.runtime, cx, request, move |_, result, cx| {
            if result.is_none() {
                owner
                    .update(cx, |editor, _| {
                        if editor.submitted == submitted {
                            editor.submitted = None;
                        }
                    })
                    .ok();
            }
        })
        .detach();
    }

    fn set_configuration(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.apply_configuration = enabled;
        if enabled {
            let definition = self.runtime.content.read(cx).workspace["countdownTimers"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|timer| timer["id"] == self.timer);
            if let Some(definition) = definition {
                let mut definition = definition.clone();
                if definition["type"].is_null() {
                    definition["type"] = json!("countdown");
                }
                if let Some(target) = definition["targetTimeSeconds"].as_f64() {
                    if ClockPeriod::from_timer(&definition).retarget(target) != target {
                        crate::automation::form::set(
                            &mut definition,
                            "format.targetTimePeriod",
                            Value::Null,
                        );
                    }
                }
                self.configuration.reset(definition);
            }
        }
        cx.notify();
    }

    fn link_timer(&mut self, timer: String, cx: &mut Context<Self>) {
        if self.timer == timer {
            return;
        }
        self.timer = timer;
        if self.timer.is_empty() {
            self.command = "start".into();
        }
        self.set_configuration(self.apply_configuration && !self.timer.is_empty(), cx);
    }
}

fn timer_configuration(form: &Form, cx: &App) -> Option<Value> {
    let draft = &form.draft;
    let seconds = |key: &str| {
        form.time_input_valid(key, cx)
            .then(|| draft[key].as_f64())
            .flatten()
    };
    let kind = draft["type"].as_str()?;
    let mut configuration = json!({
        "type": kind, "allowsOverrun": draft["allowsOverrun"] == true,
        "targetTimeSeconds": null, "elapsedStartSeconds": null, "elapsedEndSeconds": null
    });
    let duration = match kind {
        "countdown" => seconds("durationSeconds")?,
        "countdown-to-time" => {
            let target = seconds("targetTimeSeconds").filter(|target| *target < 86_400.)?;
            let period = ClockPeriod::from_timer(draft);
            if period.retarget(target) != target {
                return None;
            }
            configuration["targetTimeSeconds"] = json!(target);
            configuration["format"] = json!({"targetTimePeriod": period.key()});
            target
        }
        "elapsed" => {
            let start = seconds("elapsedStartSeconds")?;
            if !form.time_input_valid("elapsedEndSeconds", cx) {
                return None;
            }
            let end = seconds("elapsedEndSeconds");
            configuration["elapsedStartSeconds"] = json!(start);
            configuration["elapsedEndSeconds"] = json!(end);
            end.unwrap_or(start)
        }
        _ => return None,
    };
    configuration["durationSeconds"] = json!(duration);
    Some(configuration)
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let owner = cx.weak_entity();
        let mut timers = vec![SelectOption::new(String::new(), t.t("common.none"))];
        timers.extend(
            self.runtime.content.read(cx).workspace["countdownTimers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|timer| {
                    Some(SelectOption::new(
                        timer["id"].as_str()?.to_string(),
                        timer["name"]
                            .as_str()
                            .or_else(|| timer["title"].as_str())?
                            .to_string(),
                    ))
                }),
        );
        if !timers.iter().any(|option| option.value == self.timer) {
            timers.push(SelectOption::new(self.timer.clone(), self.timer.clone()));
        }
        let commands =
            COMMANDS.map(|(value, label)| SelectOption::new(value.to_string(), t.t(label)));
        let error = if !self.timer.is_empty() && !self.command_supported() {
            Some(t.t("playlistHeader.chooseTimerAction"))
        } else if self.apply_configuration && timer_configuration(&self.configuration, cx).is_none()
        {
            Some(t.t("playlistHeader.invalidTimerTime"))
        } else {
            None
        };
        let period_owner = owner.clone();
        let configuration_fields = self.apply_configuration.then(|| {
            let form = &mut self.configuration;
            let clock = form.draft["type"] == "countdown-to-time";
            let elapsed = form.draft["type"] == "elapsed";
            let period = ClockPeriod::from_timer(&form.draft);
            let specs = timer_configuration_specs(&form.draft, t);
            let period_field = clock.then(|| {
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Field::new(
                        t.t("timer.clockPeriod"),
                        div()
                            .debug_selector(|| "playlist-header-config-period".into())
                            .child(
                                Select::new(
                                    "playlist-header-clock-period",
                                    vec![
                                        SelectOption::new(ClockPeriod::Am, t.t("timer.period.am")),
                                        SelectOption::new(ClockPeriod::Pm, t.t("timer.period.pm")),
                                        SelectOption::new(
                                            ClockPeriod::FullDay,
                                            t.t("timer.period.fullDay"),
                                        ),
                                    ],
                                    period,
                                )
                                .compact()
                                .on_change(move |chosen, _, cx| {
                                    period_owner
                                        .update(cx, |editor, cx| {
                                            let form = &mut editor.configuration;
                                            if let Some(seconds) =
                                                form.draft["targetTimeSeconds"].as_f64()
                                            {
                                                form.set(
                                                    "targetTimeSeconds",
                                                    json!(chosen.retarget(seconds)),
                                                );
                                            }
                                            form.set(
                                                "format.targetTimePeriod",
                                                json!(chosen.key()),
                                            );
                                            form.refresh();
                                            cx.notify();
                                        })
                                        .ok();
                                }),
                            ),
                    ))
                    .into_any_element()
            });
            let mut fields = specs.into_iter().map(|mut spec| {
                let path = spec.path.clone();
                if path == "targetTimeSeconds" {
                    spec.kind = FieldKind::ClockTime(period);
                }
                if path == "elapsedEndSeconds" {
                    spec.placeholder = Some(t.t("timer.noLimit"));
                }
                spec.stacked = matches!(
                    path.as_str(),
                    "targetTimeSeconds" | "elapsedStartSeconds" | "elapsedEndSeconds"
                );
                div()
                    .debug_selector(move || format!("playlist-header-config-{path}"))
                    .flex_1()
                    .min_w_0()
                    .child(form.field(
                        "playlist-header-config",
                        &spec,
                        |editor: &mut Self| &mut editor.configuration,
                        window,
                        cx,
                    ))
                    .into_any_element()
            });
            let kind = fields.next();
            let overrun = fields.next_back();
            div()
                .flex()
                .flex_col()
                .gap(theme.metrics.space_xs)
                .children(kind)
                .child(
                    div()
                        .when(clock || elapsed, |row| {
                            row.flex().gap(theme.metrics.space_sm)
                        })
                        .children(fields.chain(period_field)),
                )
                .children(overrun)
        });
        let color_owner = owner.clone();
        let timer_owner = owner.clone();
        let command_owner = owner.clone();
        div()
            .debug_selector(|| "playlist-header-settings-popover".into())
            .flex()
            .flex_col()
            .gap(theme.metrics.space_sm)
            .w(theme.metrics.control_height * 10.)
            .p(theme.metrics.space_md)
            .border_1()
            .border_color(theme.colors.border)
            .rounded(theme.metrics.radius_md)
            .bg(theme.colors.popover_bg)
            .text_color(theme.colors.text)
            .shadow(theme.shadow_soft())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.metrics.space_xs)
                    .child(
                        div()
                            .debug_selector(|| "playlist-header-name".into())
                            .flex_1()
                            .min_w_0()
                            .child(
                                TextInput::new("playlist-header-name", &self.title)
                                    .label(t.t("playlistHeader.name"))
                                    .compact(),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "playlist-header-color".into())
                            .w(theme.metrics.control_height * 2.)
                            .child(
                                ColorPicker::new(
                                    "playlist-header-color",
                                    t.t("playlistHeader.changeColor"),
                                    &self.color,
                                )
                                .on_change(move |color, _, cx| {
                                    color_owner
                                        .update(cx, |editor, cx| {
                                            editor.color = color;
                                            cx.notify();
                                        })
                                        .ok();
                                }),
                            ),
                    ),
            )
            .child(Field::new(
                t.t("playlistHeader.timer"),
                div()
                    .debug_selector(|| "playlist-header-timer".into())
                    .child(
                        Select::new("playlist-header-timer", timers, Some(self.timer.clone()))
                            .compact()
                            .on_change(move |timer, _, cx| {
                                timer_owner
                                    .update(cx, |editor, cx| {
                                        editor.link_timer(timer, cx);
                                    })
                                    .ok();
                            }),
                    ),
            ))
            .when(!self.timer.is_empty(), |panel| {
                panel
                    .child(
                        div()
                            .debug_selector(|| "playlist-header-timer-command".into())
                            .child(
                                Select::new(
                                    "playlist-header-timer-command",
                                    commands.to_vec(),
                                    Some(self.command.clone()),
                                )
                                .label(t.t("playlistHeader.timerCommand"))
                                .compact()
                                .on_change(
                                    move |command, _, cx| {
                                        command_owner
                                            .update(cx, |editor, cx| {
                                                editor.command = command;
                                                cx.notify();
                                            })
                                            .ok();
                                    },
                                ),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "playlist-header-set-configuration".into())
                            .child(
                                CheckboxField::new(
                                    "playlist-header-set-configuration",
                                    t.t("playlistHeader.setConfiguration"),
                                    self.apply_configuration,
                                )
                                .on_change(
                                    move |enabled, _, cx| {
                                        owner
                                            .update(cx, |editor, cx| {
                                                editor.set_configuration(enabled, cx);
                                            })
                                            .ok();
                                    },
                                ),
                            ),
                    )
                    .children(configuration_fields)
            })
            .when_some(error, |panel, error| {
                panel.child(
                    div()
                        .text_size(theme.metrics.font_sm)
                        .text_color(theme.colors.warning)
                        .child(error),
                )
            })
    }
}

pub fn save_color(runtime: &Runtime, cx: &mut App, playlist: &str, entry: &str, color: String) {
    let request = Persist::new(
        Kind::Playlist,
        json!({
            "type": "setHeaderColor", "playlistId": playlist, "entryId": entry, "color": color
        }),
        |_, _| {},
    );
    super::tree_actions::spawn_persist(runtime, cx, request, |_, _, _| {}).detach();
}

pub fn actions(entry: &Entry) -> Vec<Value> {
    entry.raw["actions"].as_array().cloned().unwrap_or_default()
}
