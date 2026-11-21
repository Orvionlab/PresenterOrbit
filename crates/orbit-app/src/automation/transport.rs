//! The transport controls of the right column (`TransportControls.tsx`, `handleTransportAction`): play and pause,
//! ten seconds back and forward, and a jump to a marker, for the media layer and the music layer. The playback
//! clock lives in the output process, so a command is the host's `controlPlayback` sent there (non-marker
//! commands only; see the allow-list). The answer's state patch is applied to the operator's live state so the
//! next scene does not undo a pause. The media position is read back from the output process while media plays.

use std::time::Duration;

use gpui::{div, prelude::*, AppContext, Context, SharedString, Window};
use orbit_ui::controls::{
    range_state, ActionButton, ButtonVariant, Field, IconButton, RangeInput, Select, SelectOption,
    SliderEvent, SliderState,
};
use orbit_ui::{icon, ActiveTheme, Translator};
use serde_json::{json, Value};

use crate::state::runtime::{output_call, Runtime};

/// `PlaybackCommand` for a transport button: `toggle`, `play`, `pause`, `forward`, or `backward`.
pub fn command(layer: &str, action: &str) -> Value {
    json!({"layer": layer, "action": action})
}

/// A `seek` command to `seconds` of `duration`; `None` while the duration is unknown or the time is outside it.
pub fn seek_command(layer: &str, seconds: f64, duration: Option<f64>) -> Option<Value> {
    let duration = duration.filter(|duration| duration.is_finite() && *duration > 0.)?;
    (seconds.is_finite() && (0. ..=duration).contains(&seconds))
        .then(|| json!({"layer": layer, "action": "seek", "position": seconds / duration}))
}

/// `formatTimerValue` for the time row: `MM:SS`, or `H:MM:SS` from an hour.
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.).floor() as u64;
    if total >= 3600 {
        format!("{}:{:02}:{:02}", total / 3600, total / 60 % 60, total % 60)
    } else {
        format!("{:02}:{:02}", total / 60, total % 60)
    }
}

/// The asset on air for a layer, and its title.
pub fn on_air<'a>(live: &Value, media: &'a [Value], layer: &str) -> Option<&'a Value> {
    let id = if layer == "music" {
        &live["audioId"]
    } else {
        &live["backgroundId"]
    };
    media.iter().find(|asset| asset["id"] == *id)
}

/// The slide timer counting down on the live slide, as the host reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Countdown {
    pub live_key: String,
    pub delay: f64,
    pub remaining: f64,
}

impl Countdown {
    /// `None` when the host reports no countdown or one that cannot be drawn.
    pub fn from_host(answer: &Value) -> Option<Self> {
        let countdown = Self {
            live_key: answer["liveKey"].as_str()?.to_string(),
            delay: answer["delaySeconds"].as_f64()?,
            remaining: answer["remainingSeconds"].as_f64()?,
        };
        (countdown.delay > 0. && countdown.remaining.is_finite()).then_some(countdown)
    }

    /// How much of the delay has passed: 0 when the slide went live, 1 when the next slide is taken.
    pub fn progress(&self) -> f32 {
        (1. - self.remaining / self.delay).clamp(0., 1.) as f32
    }

    pub fn seconds_left(&self) -> u64 {
        self.remaining.max(0.).ceil() as u64
    }
}

/// Whether the slide on air has a slide timer; `scene` is the output snapshot, which carries the live slide.
fn counts_down(scene: &Value) -> bool {
    scene["liveSlide"]["completionAction"]
        .as_str()
        .is_some_and(|action| action.ends_with("AFTER_TIME"))
}

pub struct Transport {
    runtime: Runtime,
    pub layer: &'static str,
    /// The last playback state read from the output process for the media layer.
    playback: Value,
    playback_layer: String,
    progress: gpui::Entity<SliderState>,
    polling: bool,
    counting: bool,
    countdown: Option<Countdown>,
    status: String,
}

impl Transport {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        cx.observe(&runtime.output, |_, _, cx| cx.notify()).detach();
        cx.observe(&runtime.content, |_, _, cx| cx.notify())
            .detach();
        let progress = range_state(0., 1., 0.001, 0., cx);
        cx.subscribe(&progress, |this, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(value) = event else {
                return;
            };
            if this.playback["duration"].as_f64().is_some_and(|v| v > 0.) {
                this.control(
                    json!({"layer":this.layer,"action":"seek","position":value.end()}),
                    cx,
                );
            }
        })
        .detach();
        Self {
            runtime,
            progress,
            layer: "media",
            playback: Value::Null,
            playback_layer: String::new(),
            polling: false,
            counting: false,
            countdown: None,
            status: String::new(),
        }
    }

    /// Sends one command to the output process and keeps the live state in step with the answer.
    pub fn control(&mut self, command: Value, cx: &mut Context<Self>) {
        self.status.clear();
        let host = self.runtime.host.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(
                    async move { output_call(&host, "controlPlayback", vec![command]) },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok(answer) => this.accept(&answer, cx),
                    Err(error) => this.status = error,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn accept(&mut self, answer: &Value, cx: &mut Context<Self>) {
        if answer["patch"]
            .as_object()
            .is_some_and(|patch| !patch.is_empty())
        {
            self.runtime.output.update(cx, |output, cx| {
                if let Err(error) = output.accept_native_patch(&json!({
                    "revision": answer["revision"], "patch": answer["patch"]
                })) {
                    output.error = Some(error);
                }
                cx.notify();
            });
        }
    }

    /// The slide timer lives in the scheduler of this process; read it while the slide on air has one.
    fn count(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.counting, true) {
            return;
        }
        cx.spawn(async move |this, cx| loop {
            let Ok(host) = this.update(cx, |this, cx| {
                let output = this.runtime.output.read(cx);
                output
                    .snapshot
                    .as_deref()
                    .is_some_and(counts_down)
                    .then(|| this.runtime.host.clone())
            }) else {
                break;
            };
            let Some(host) = host else {
                let _ = this.update(cx, |this, cx| {
                    this.counting = false;
                    this.countdown = None;
                    cx.notify();
                });
                break;
            };
            let read = cx
                .background_spawn(async move { host.invoke("getSlideCompletion", vec![]) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match read {
                    Ok(answer) => this.countdown = Countdown::from_host(&answer),
                    Err(error) => {
                        this.countdown = None;
                        this.status = error;
                    }
                }
                cx.notify();
            });
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
        })
        .detach();
    }

    /// Stops the pending auto advance of the live slide; the saved slides keep their timers.
    fn cancel_countdown(&mut self, cx: &mut Context<Self>) {
        let Some(countdown) = self.countdown.take() else {
            return;
        };
        let host = self.runtime.host.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move {
                    host.invoke("cancelSlideCompletion", vec![json!(countdown.live_key)])
                })
                .await;
            if let Err(error) = outcome {
                let _ = this.update(cx, |this, cx| {
                    this.status = error;
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// The output process measures the media position; read it while media is on air.
    fn follow(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.polling, true) {
            return;
        }
        cx.spawn(async move |this, cx| loop {
            let Ok(Some((host, screen))) = this.update(cx, |this, cx| {
                let output = this.runtime.output.read(cx);
                let source = output.snapshot.as_ref().and_then(|snapshot| {
                    crate::output::view::screens(snapshot)
                        .into_iter()
                        .find(|(_, config)| config["type"] == "audience")
                        .map(|(id, _)| id)
                });
                let wanted = !output.live[if this.layer == "music" {
                    "audioId"
                } else {
                    "backgroundId"
                }]
                .is_null();
                source
                    .filter(|_| wanted)
                    .map(|screen| (this.runtime.host.clone(), (screen, this.layer)))
            }) else {
                let _ = this.update(cx, |this, _| {
                    this.polling = false;
                    this.playback = Value::Null;
                });
                break;
            };
            let request_layer = screen.1;
            let expected = this
                .update(cx, |this, cx| {
                    super::markers::layer_key(
                        &this.runtime.output.read(cx).live,
                        &this.runtime.content.read(cx).workspace_path,
                        request_layer,
                    )
                })
                .ok();
            let read = cx
                .background_spawn(async move {
                    output_call(
                        &host,
                        if screen.1 == "music" {
                            "getNativeMusicPlayback"
                        } else {
                            "getNativeMediaPlayback"
                        },
                        vec![json!(screen.0)],
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let current = super::markers::layer_key(
                    &this.runtime.output.read(cx).live,
                    &this.runtime.content.read(cx).workspace_path,
                    request_layer,
                );
                if this.layer != request_layer || expected.as_deref() != Some(current.as_str()) {
                    return;
                }
                match read {
                    Ok(answer) => {
                        this.playback = answer["playback"].clone();
                        this.playback_layer = request_layer.to_string();
                    }
                    Err(error) => this.status = error,
                }
                cx.notify();
            });
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
        })
        .detach();
    }
}

impl Render for Transport {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t: Translator = Runtime::translator(cx);
        let layer = self.layer;
        let live = self.runtime.output.read(cx).live.clone();
        let workspace = self.runtime.content.read(cx).workspace.clone();
        let media = workspace["media"].as_array().cloned().unwrap_or_default();
        let asset = on_air(&live, &media, layer).cloned();
        if asset.is_some() {
            self.follow(cx);
        }
        let counting = self
            .runtime
            .output
            .read(cx)
            .snapshot
            .as_deref()
            .is_some_and(counts_down);
        if counting {
            self.count(cx);
        }
        let has_content = asset.is_some();
        let paused = live["transportPlaybackPaused"][layer] == true;
        if !has_content || self.playback_layer != layer {
            self.playback = Value::Null;
        }
        let current = self.playback["currentTime"].as_f64().unwrap_or(0.);
        let duration = self.playback["duration"]
            .as_f64()
            .filter(|value| *value > 0.);
        let fraction = duration.map_or(0., |duration| (current / duration).clamp(0., 1.));
        self.progress.update(cx, |progress, cx| {
            progress.set_value(fraction as f32, window, cx)
        });
        let theme = *cx.theme();
        let mut controls = div()
            .flex()
            .justify_center()
            .items_center()
            .gap(theme.metrics.space_lg);
        let markers = asset
            .as_ref()
            .and_then(|asset| asset["markers"].as_array())
            .cloned()
            .unwrap_or_default();
        if !markers.is_empty() {
            let weak = cx.weak_entity();
            let options: Vec<SelectOption<String>> = markers
                .iter()
                .map(|marker| {
                    SelectOption::new(
                        marker["id"].as_str().unwrap_or("").to_string(),
                        format!(
                            "{} ({})",
                            marker["title"].as_str().unwrap_or(""),
                            clock(marker["timeSeconds"].as_f64().unwrap_or(0.))
                        ),
                    )
                })
                .collect();
            let markers = markers.clone();
            controls = controls.child(
                Field::new(
                    t.t("media.markers.tab"),
                    Select::new(
                        SharedString::from(format!("transport-marker-{layer}")),
                        options,
                        None::<String>,
                    )
                    .compact()
                    .on_change(move |chosen: String, _, cx| {
                        let seconds = markers
                            .iter()
                            .find(|marker| marker["id"] == chosen.as_str())
                            .and_then(|marker| marker["timeSeconds"].as_f64());
                        let _ = weak.update(cx, |this, cx| {
                            let command =
                                seconds.and_then(|seconds| seek_command(layer, seconds, duration));
                            match command {
                                Some(command) => this.control(command, cx),
                                None => {
                                    this.status = "media.markers.jumpFailed".into();
                                    cx.notify();
                                }
                            }
                        });
                    }),
                )
                .inline(),
            );
        }
        for (name, label, action) in [
            ("backward", t.t("transport.back10"), "backward"),
            (
                "toggle",
                t.t(if paused {
                    "transport.play"
                } else {
                    "transport.pause"
                }),
                "toggle",
            ),
            ("forward", t.t("transport.forward10"), "forward"),
        ] {
            let id = SharedString::from(format!("transport-{layer}-{name}"));
            controls = controls.child(if name == "toggle" {
                IconButton::new(
                    id,
                    if paused {
                        "player-play"
                    } else {
                        "player-pause"
                    },
                    label,
                )
                .disabled(!has_content)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.control(command(layer, action), cx)),
                )
                .into_any_element()
            } else {
                ActionButton::new(
                    id,
                    if name == "backward" {
                        "\u{21b6} 10"
                    } else {
                        "10 \u{21b7}"
                    },
                )
                .compact()
                .variant(ButtonVariant::Ghost)
                .disabled(!has_content)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.control(command(layer, action), cx)),
                )
                .into_any_element()
            });
        }
        let mut column = div()
            .flex()
            .flex_col()
            .items_center()
            .px(theme.metrics.field_padding_x)
            .py(theme.metrics.space_snug)
            .child(
                div()
                    .h(theme.metrics.row_height)
                    .text_center()
                    .truncate()
                    .child(
                        asset
                            .as_ref()
                            .and_then(|asset| asset["title"].as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
            )
            .child(
                div().w_full().max_w(theme.metrics.space_lg * 27.5).child(
                    RangeInput::new(&self.progress)
                        .scrub()
                        .label(t.format("transport.progress", &[("layer", layer)]))
                        .disabled(!has_content || duration.is_none()),
                ),
            );
        {
            column = column.child(
                div()
                    .h(theme.metrics.row_height)
                    .w_full()
                    .max_w(theme.metrics.space_lg * 27.5)
                    .flex()
                    .justify_between()
                    .when(!has_content, |row| row.invisible())
                    .child(clock(current))
                    .child(format!(
                        "-{}",
                        clock(duration.map_or(0., |duration| duration - current))
                    )),
            );
        }
        column = column.child(controls);
        if let Some(countdown) = self.countdown.as_ref().filter(|_| counting) {
            column = column.child(
                div()
                    .id("transport-advance")
                    .role(gpui::Role::Button)
                    .aria_label(t.t("transport.advance.cancel"))
                    .tooltip(orbit_ui::controls::tooltip_text(
                        t.t("transport.advance.cancel"),
                    ))
                    .relative()
                    .w_full()
                    .h(theme.metrics.control_height_compact)
                    .mt(theme.metrics.space_sm)
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(theme.metrics.field_padding_x)
                    .overflow_hidden()
                    .cursor_pointer()
                    .rounded(theme.metrics.radius_xs)
                    .bg(theme.colors.panel_bg_soft)
                    .text_size(theme.metrics.font_sm)
                    .text_color(theme.colors.text_strong)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_countdown(cx)))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .bottom_0()
                            .w(gpui::relative(countdown.progress()))
                            .bg(theme.colors.danger),
                    )
                    .child(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .gap(theme.metrics.space_snug)
                            .child(icon("player-play").text_color(theme.colors.text_strong))
                            .child(t.t("transport.advance")),
                    )
                    .child(
                        div()
                            .relative()
                            .child(format!("{}s", countdown.seconds_left())),
                    ),
            );
        }
        if !self.status.is_empty() {
            column = column.child(
                div()
                    .id("transport-status")
                    .role(gpui::Role::Alert)
                    .child(t.t(&self.status)),
            );
        }
        column.into_any_element()
    }
}
