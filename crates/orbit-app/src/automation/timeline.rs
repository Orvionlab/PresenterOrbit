//! Timeline authoring and controller transport. Native scheduling and soundtrack decoding stay in the host.
use super::{
    form::{Form, Kind, Spec},
    runner::Facts,
};
use crate::services::{choices, entries};
use crate::state::runtime::{Kind as Collection, Persist, Runtime};
use gpui::{div, prelude::*, App, Context, Window};
use orbit_ui::{
    controls::{ActionButton, Dialog},
    ActiveTheme,
};
use serde_json::{json, Value};
pub fn position(transport: &Value, now: f64) -> f64 {
    transport["positionSeconds"].as_f64().unwrap_or(0.)
        + transport["startedAtMs"].as_f64().map_or(0., |at| {
            (now - at).max(0.) / 1000. * transport["playbackRate"].as_f64().unwrap_or(1.)
        })
}
pub fn validate(facts: &Facts, item: &Value, timeline: &Value) -> Result<(), String> {
    let invalid = || "automation.timeline.invalid".to_string();
    let duration = timeline["durationSeconds"]
        .as_f64()
        .filter(|d| *d > 0. && *d <= 86400.)
        .ok_or_else(invalid)?;
    if !timeline["loop"].is_boolean() {
        return Err(invalid());
    }
    if timeline
        .get("timecodeEnabled")
        .is_some_and(|v| !v.is_boolean())
    {
        return Err(invalid());
    }
    let offset = timeline.get("timecodeOffsetSeconds");
    if (timeline["timecodeEnabled"] == true && offset.is_none())
        || offset.is_some_and(|v| v.as_f64().is_none_or(|v| !(0. ..86400.).contains(&v)))
    {
        return Err(invalid());
    }
    if let Some(id) = timeline.get("soundtrackMediaId") {
        if !entries(&facts.workspace["media"])
            .iter()
            .any(|m| m["id"] == *id && m["type"] == "audio")
        {
            return Err(invalid());
        }
    }
    let cues = timeline["cues"]
        .as_array()
        .filter(|c| c.len() <= 4096)
        .ok_or_else(invalid)?;
    let resolved = orbit_host::Host::presentation_cues(
        item,
        crate::cues::model::selected_arrangement_id(item),
        None,
    )?;
    let mut ids = std::collections::HashSet::new();
    for cue in cues {
        let id = cue["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(invalid)?;
        if !ids.insert(id)
            || cue["timeSeconds"]
                .as_f64()
                .is_none_or(|t| t < 0. || t >= duration)
        {
            return Err(invalid());
        }
        if cue
            .get("cueInstanceId")
            .is_some_and(|v| v.as_str().is_none_or(str::is_empty))
        {
            return Err(invalid());
        }
        if cue["action"].is_object() {
            if super::validate::action_error(&cue["action"], facts, "").is_some() {
                return Err(invalid());
            }
        } else if !resolved.iter().any(|s| {
            (s["id"] == cue["cueId"] || s["sourceCueId"] == cue["cueId"])
                && (cue.get("cueInstanceId").is_none()
                    || s["arrangementInstanceId"] == cue["cueInstanceId"])
        }) {
            return Err(invalid());
        }
    }
    Ok(())
}
pub fn transport_patch(
    item: &Value,
    timeline: &Value,
    seconds: f64,
    playing: bool,
    at: f64,
    rate: f64,
    generation: &str,
) -> Result<Value, String> {
    let duration = timeline["durationSeconds"]
        .as_f64()
        .ok_or("automation.timeline.invalid")?;
    if !seconds.is_finite()
        || seconds < 0.
        || seconds > duration
        || !rate.is_finite()
        || rate == 0.
        || rate.abs() > 16.
    {
        return Err("automation.timeline.invalid".into());
    }
    let cues = orbit_host::Host::presentation_cues(
        item,
        crate::cues::model::selected_arrangement_id(item),
        None,
    )?;
    let target = entries(&timeline["cues"])
        .into_iter()
        .filter(|c| {
            c["cueId"].is_string() && c["timeSeconds"].as_f64().is_some_and(|t| t <= seconds)
        })
        .max_by(|a, b| {
            a["timeSeconds"]
                .as_f64()
                .partial_cmp(&b["timeSeconds"].as_f64())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let slide = target
        .and_then(|target| {
            cues.iter().find(|s| {
                s["disabled"] != true
                    && (s["id"] == target["cueId"] || s["sourceCueId"] == target["cueId"])
                    && (target.get("cueInstanceId").is_none()
                        || s["arrangementInstanceId"] == target["cueInstanceId"])
            })
        })
        .or_else(|| cues.iter().find(|s| s["disabled"] != true))
        .ok_or("automation.timeline.invalid")?;
    Ok(
        json!({"liveItemId":item["id"],"liveSlideId":slide["id"],"liveCueInstanceId":slide["arrangementInstanceId"],"livePlaylistId":null,"livePlaylistEntryId":null,"editorProgramSnapshot":null,
        "timelineTransport":{"mode":"playback","itemId":item["id"],"generation":generation,"positionSeconds":seconds,"playbackRate":rate,"startedAtMs":if playing{json!(at as i64)}else{Value::Null}}}),
    )
}
pub fn apply(runtime: &Runtime, patch: Value, cx: &mut App) {
    if let Some(fields) = patch.as_object() {
        runtime.output.update(cx, |output, cx| {
            output.edit_live(|live| {
                if let Some(live) = live.as_object_mut() {
                    live.extend(fields.clone());
                }
            });
            cx.notify();
        });
    }
    crate::cues::actions::edit_native(runtime, cx, move |live| {
        live.as_object_mut()
            .ok_or("Live state must be an object.")?
            .extend(
                patch
                    .as_object()
                    .ok_or("Timeline patch must be an object.")?
                    .clone(),
            );
        Ok(())
    });
}
struct Recorder(gpui::WeakEntity<Timeline>);
impl gpui::Global for Recorder {}
pub fn record_operator(runtime: &Runtime, actions: &[Value], cx: &mut App) {
    let Some(recorder) = cx.try_global::<Recorder>().map(|r| r.0.clone()) else {
        return;
    };
    let facts = super::facts(runtime, cx);
    let actions = actions
        .iter()
        .filter(|a| a["enabled"] != false && super::validate::action_error(a, &facts, "").is_none())
        .cloned()
        .collect::<Vec<_>>();
    let _=recorder.update(cx,|this,cx| {
        let live=this.runtime.output.read(cx).live.clone();let transport=&live["timelineTransport"];
        if this.retired || transport["mode"]!="recording" || transport["generation"]!=this.generation || transport["startedAtMs"].is_null(){return;}
        let seconds=position(transport,super::announcements::now_ms());
        for action in actions {if let Some(cues)=this.form.draft["timeline"]["cues"].as_array_mut(){cues.push(json!({"id":uuid::Uuid::new_v4().to_string(),"title":action["title"].as_str().unwrap_or(action["type"].as_str().unwrap_or("")),"timeSeconds":rounded_time(seconds),"action":action}));}}
        let duration=this.form.draft["timeline"]["durationSeconds"].as_f64().unwrap_or(0.);
        this.form.set("timeline.durationSeconds",json!(duration.max(seconds+0.001)));this.form.refresh();cx.notify();
    });
}
fn rounded_time(seconds: f64) -> f64 {
    (seconds * 1000.).round() / 1000.
}
pub fn suppress_operator(active: bool, cx: &mut App) {
    let Some(recorder) = cx.try_global::<Recorder>().map(|r| r.0.clone()) else {
        return;
    };
    let _ = recorder.update(cx, |this, _| {
        if active {
            this.suppress += 1;
        } else {
            this.suppress = this.suppress.saturating_sub(1);
        }
    });
}
pub struct Timeline {
    runtime: Runtime,
    item: Value,
    path: String,
    form: Form,
    close: std::rc::Rc<dyn Fn(&mut App)>,
    status: String,
    generation: String,
    retired: bool,
    last_cue: String,
    suppress: usize,
    dragging: Option<(usize, f64, f32, f64)>,
    dragged: bool,
    waveform: Value,
    waveform_key: String,
}
fn form(this: &mut Timeline) -> &mut Form {
    &mut this.form
}
impl Timeline {
    pub fn new(
        runtime: Runtime,
        close: impl Fn(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.set_global(Recorder(cx.weak_entity()));
        let id = runtime.ui.read(cx).selected_library_item_id.clone();
        let item = runtime
            .content
            .read(cx)
            .items
            .get(&id)
            .map(|i| (*i.value).clone())
            .unwrap_or(Value::Null);
        let path = runtime.content.read(cx).workspace_path.clone();
        let timeline = if item["timeline"].is_object() {
            item["timeline"].clone()
        } else {
            json!({"durationSeconds":60,"loop":false,"cues":[]})
        };
        let mut form = Form::default();
        form.reset(json!({"timeline":timeline,"position":0,"zoom":1}));
        cx.observe(&runtime.output, |this, _, cx| this.record_cue(cx))
            .detach();
        cx.observe(&runtime.ui, |this, _, cx| {
            if this.runtime.ui.read(cx).selected_library_item_id
                != this.item["id"].as_str().unwrap_or("")
            {
                this.retired = true;
            }
        })
        .detach();
        cx.observe(&runtime.content, |this, _, cx| {
            if this.runtime.content.read(cx).workspace_path != this.path
                || !this
                    .runtime
                    .content
                    .read(cx)
                    .items
                    .contains(this.item["id"].as_str().unwrap_or(""))
            {
                this.retired = true;
            }
        })
        .detach();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        })
        .detach();
        Self {
            runtime,
            item,
            path,
            form,
            close: std::rc::Rc::new(close),
            status: String::new(),
            generation: String::new(),
            retired: false,
            last_cue: String::new(),
            suppress: 0,
            dragging: None,
            dragged: false,
            waveform: Value::Null,
            waveform_key: String::new(),
        }
    }
    fn waveform(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.form.draft["timeline"]["soundtrackMediaId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            self.waveform_key.clear();
            self.waveform = Value::Null;
            return;
        };
        if self.waveform_key == id {
            return;
        }
        self.waveform_key = id.clone();
        self.waveform = Value::Null;
        let (host, path) = (self.runtime.host.clone(), self.path.clone());
        cx.spawn(async move |this, cx| {
            let key = id.clone();
            let result = cx
                .background_spawn(async move {
                    host.invoke(
                        "getTimelineWaveform",
                        vec![json!(path), json!(id), json!(256)],
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.retired && this.waveform_key == key {
                    match result {
                        Ok(v) => this.waveform = v,
                        Err(e) => this.status = e,
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn record_cue(&mut self, cx: &mut Context<Self>) {
        let live = &self.runtime.output.read(cx).live;
        let transport = &live["timelineTransport"];
        if self.suppress > 0
            || self.retired
            || transport["mode"] != "recording"
            || transport["generation"] != self.generation
            || transport["startedAtMs"].is_null()
            || live["liveItemId"] != self.item["id"]
        {
            return;
        }
        let key = json!([
            live["liveSlideId"],
            live["liveCueInstanceId"],
            live["timelineStartedAtMs"]
        ])
        .to_string();
        if key == self.last_cue {
            return;
        }
        self.last_cue = key;
        let seconds = position(transport, super::announcements::now_ms());
        let mut cue = json!({"id":uuid::Uuid::new_v4().to_string(),"title":live["liveSlideId"],"cueId":live["liveSlideId"],"timeSeconds":(seconds*1000.).round()/1000.});
        if live["liveCueInstanceId"].is_string() {
            cue["cueInstanceId"] = live["liveCueInstanceId"].clone();
        }
        if let Some(c) = self.form.draft["timeline"]["cues"].as_array_mut() {
            c.push(cue)
        }
        let duration = self.form.draft["timeline"]["durationSeconds"]
            .as_f64()
            .unwrap_or(0.);
        self.form.set(
            "timeline.durationSeconds",
            json!(duration.max(seconds + 0.001)),
        );
        self.form.refresh();
        cx.notify();
    }
    fn control(&mut self, action: &str, cx: &mut Context<Self>) {
        if self.retired {
            self.status = "automation.timeline.retired".into();
            cx.notify();
            return;
        }
        let now = super::announcements::now_ms();
        let live = self.runtime.output.read(cx).live.clone();
        let current = &live["timelineTransport"];
        if (matches!(action, "pause" | "resume")
            || (action == "play" && current["mode"] == "recording"))
            && current["itemId"] == self.item["id"]
        {
            if current["mode"] == "recording" && action != "pause" {
                self.last_cue = json!([
                    live["liveSlideId"],
                    live["liveCueInstanceId"],
                    live["timelineStartedAtMs"]
                ])
                .to_string();
            }
            let mut next = current.clone();
            next["positionSeconds"] = json!(position(current, now));
            next["startedAtMs"] = if action != "pause" {
                json!(now as i64)
            } else {
                Value::Null
            };
            apply(&self.runtime, json!({"timelineTransport":next}), cx);
            return;
        }
        self.generation = uuid::Uuid::new_v4().to_string();
        let timeline = &self.form.draft["timeline"];
        let result =
            validate(&super::facts(&self.runtime, cx), &self.item, timeline).and_then(|_| {
                transport_patch(
                    &self.item,
                    timeline,
                    self.form.draft["position"].as_f64().unwrap_or(0.),
                    action == "play",
                    now,
                    1.,
                    &self.generation,
                )
            });
        match result {
            Ok(mut patch) => {
                if action == "record" {
                    patch["timelineTransport"]["mode"] = json!("recording");
                }
                if action == "stop" {
                    patch = json!({"timelineTransport":{"mode":"stopped","itemId":self.item["id"],"generation":self.generation,"positionSeconds":0,"playbackRate":1,"startedAtMs":null}});
                }
                apply(&self.runtime, patch, cx);
            }
            Err(e) => self.status = e,
        }
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.retired {
            self.status = "automation.timeline.retired".into();
            return;
        }
        let timeline = self.form.draft["timeline"].clone();
        if let Err(e) = validate(&super::facts(&self.runtime, cx), &self.item, &timeline) {
            self.status = e;
            cx.notify();
            return;
        }
        self.control("stop", cx);
        let mutation = json!({"type":"mutateItem","itemId":self.item["id"],"operation":{"type":"setTimeline","timeline":timeline}});
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let saved = runtime
                .persist(cx, Persist::new(Collection::Library, mutation, |_, _| {}))
                .await;
            let _ = this.update(cx, |this, cx| {
                if saved.is_none() {
                    this.status = "automation.timeline.invalid".into();
                }
                cx.notify();
            });
        })
        .detach();
    }
}
impl Render for Timeline {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Runtime::translator(cx);
        let gap = cx.theme().metrics.space_xs;
        let source = (*self.runtime.content.read(cx).workspace).clone();
        let mut body = div()
            .id("timeline-body")
            .flex()
            .flex_col()
            .gap(gap)
            .max_h(window.viewport_size().height - cx.theme().metrics.row_height * 6.)
            .overflow_y_scroll()
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if let Some((index, start, x, scale)) = this.dragging {
                    let delta = f32::from(event.position.x) - x;
                    if delta.abs() < 3. && !this.dragged {
                        return;
                    }
                    let duration = this.form.draft["timeline"]["durationSeconds"]
                        .as_f64()
                        .unwrap_or(0.);
                    let seconds = rounded_time(
                        (start + f64::from(delta) / scale).clamp(0., (duration - 0.001).max(0.)),
                    );
                    this.form.set(
                        &format!("timeline.cues.{index}.timeSeconds"),
                        json!(seconds),
                    );
                    this.form.refresh();
                    this.dragged = true;
                    cx.notify();
                }
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.dragging = None;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.dragging = None;
                    cx.notify();
                }),
            );
        for spec in [
            Spec::new(
                "timeline.durationSeconds",
                t.t("automation.timeline.duration"),
                Kind::Number,
            ),
            Spec::new(
                "timeline.loop",
                t.t("automation.timeline.repeat"),
                Kind::Check,
            ),
            Spec::new(
                "timeline.timecodeEnabled",
                t.t("automation.timecode.follow"),
                Kind::Check,
            ),
            Spec::new(
                "timeline.timecodeOffsetSeconds",
                t.t("automation.timecode.start"),
                Kind::Number,
            ),
            Spec::new(
                "timeline.soundtrackMediaId",
                t.t("automation.timeline.soundtrack"),
                Kind::Choice(choices(
                    &json!(entries(&source["media"])
                        .into_iter()
                        .filter(|m| m["type"] == "audio")
                        .collect::<Vec<_>>()),
                    "title",
                )),
            ),
            Spec::new(
                "position",
                t.t("automation.timeline.position"),
                Kind::Number,
            ),
        ] {
            body = body.child(self.form.field("timeline", &spec, form, window, cx));
        }
        let mut controls = div().flex().flex_wrap().gap(gap);
        for name in ["record", "play", "pause", "resume", "stop", "seek"] {
            controls = controls.child(
                ActionButton::new(
                    format!("timeline-{name}"),
                    t.t(&format!("automation.timeline.{name}")),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.control(name, cx))),
            );
        }
        body = body.child(controls);
        self.waveform(cx);
        let peaks = entries(&self.waveform["peaks"]);
        if !peaks.is_empty() {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .h(cx.theme().metrics.row_height * 2.)
                    .children(peaks.iter().map(|peak| {
                        div()
                            .flex_1()
                            .h(gpui::px((peak.as_f64().unwrap_or(0.).abs() * 40.) as f32))
                            .bg(cx.theme().colors.accent)
                    })),
            );
        }
        body = body.child(self.form.field(
            "timeline-zoom",
            &Spec::new("zoom", t.t("automation.timeline.zoom"), Kind::Number),
            form,
            window,
            cx,
        ));
        let zoom = self.form.draft["zoom"]
            .as_f64()
            .unwrap_or(1.)
            .clamp(1., 20.);
        let duration = self.form.draft["timeline"]["durationSeconds"]
            .as_f64()
            .unwrap_or(60.);
        let cues = entries(&self.form.draft["timeline"]["cues"]);
        for lane in ["slides", "actions"] {
            let mut track = div()
                .flex()
                .flex_col()
                .gap(gap)
                .child(t.t(&format!("automation.timeline.lane.{lane}")));
            let pixels_per_second = 12. * zoom;
            let mut lane_view = div()
                .relative()
                .w(gpui::px((duration * pixels_per_second) as f32))
                .h(cx.theme().metrics.row_height)
                .bg(cx.theme().colors.field_bg);
            for (index, cue) in cues
                .iter()
                .enumerate()
                .filter(|(_, c)| c["action"].is_object() == (lane == "actions"))
            {
                let seconds = cue["timeSeconds"].as_f64().unwrap_or(0.);
                lane_view = lane_view.child(
                    div()
                        .id(format!("timeline-marker-hit-{index}"))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, _| {
                                this.dragging = Some((
                                    index,
                                    seconds,
                                    f32::from(event.position.x),
                                    pixels_per_second,
                                ));
                                this.dragged = false;
                            }),
                        )
                        .absolute()
                        .left(gpui::px((seconds * pixels_per_second) as f32))
                        .child(
                            ActionButton::new(
                                format!("timeline-marker-{index}"),
                                cue["title"].as_str().unwrap_or("").to_string(),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if this.dragged {
                                        this.dragged = false;
                                        return;
                                    }
                                    this.form.set("position", json!(seconds));
                                    this.form.refresh();
                                    this.control("seek", cx);
                                },
                            )),
                        ),
                );
            }
            track = track.child(
                div()
                    .id(format!("timeline-lane-{lane}"))
                    .overflow_x_scroll()
                    .child(lane_view),
            );
            for (index, cue) in cues
                .iter()
                .enumerate()
                .filter(|(_, c)| c["action"].is_object() == (lane == "actions"))
            {
                let path = format!("timeline.cues.{index}");
                for spec in [
                    Spec::new(
                        format!("{path}.timeSeconds"),
                        t.t("automation.timeline.time"),
                        Kind::Number,
                    ),
                    Spec::new(
                        format!("{path}.title"),
                        t.t("automation.macros.name"),
                        Kind::Text,
                    ),
                ] {
                    track = track.child(self.form.field("timeline-cue", &spec, form, window, cx));
                }
                if lane == "slides" {
                    track = track.child(self.form.field(
                        "timeline-target",
                        &Spec::new(
                            format!("{path}.cueId"),
                            t.t("automation.timeline.cue"),
                            Kind::Choice(super::slide_authoring::targets(&self.item)),
                        ),
                        form,
                        window,
                        cx,
                    ));
                } else {
                    for spec in super::action_list::specs(
                        &format!("{path}.action"),
                        &cue["action"],
                        &source,
                        "",
                        t,
                    ) {
                        track = track.child(self.form.field(
                            "timeline-action",
                            &spec,
                            form,
                            window,
                            cx,
                        ));
                    }
                }
                track = track.child(
                    ActionButton::new(format!("timeline-remove-{index}"), t.t("common.delete"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(cues) = this.form.draft["timeline"]["cues"].as_array_mut() {
                                cues.remove(index);
                            }
                            this.form.refresh();
                            cx.notify();
                        })),
                );
            }
            let action = lane == "actions";
            track=track.child(ActionButton::new(format!("timeline-add-{lane}"),t.t("automation.timeline.add")).on_click(cx.listener(move|this,_,_,cx|{
                let mut cue=json!({"id":uuid::Uuid::new_v4().to_string(),"timeSeconds":this.form.draft["position"],"title":""});
                if action{cue["action"]=super::action_list::new_action(uuid::Uuid::new_v4().to_string());}
                else {cue["cueId"]=this.item["slides"][0]["id"].clone();}
                if let Some(cues)=this.form.draft["timeline"]["cues"].as_array_mut(){cues.push(cue);}this.form.refresh();cx.notify();
            })));
            body = body.child(track);
        }
        let transport = &self.runtime.output.read(cx).live["timelineTransport"];
        body = body
            .child(super::transport::clock(position(
                transport,
                super::announcements::now_ms(),
            )))
            .child(
                ActionButton::new("timeline-save", t.t("automation.macros.save"))
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
            .child(t.t(&self.status));
        let close = self.close.clone();
        let weak = cx.weak_entity();
        Dialog::new(
            "timeline",
            t.t("automation.timeline.title"),
            t.t("common.close"),
            move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    let current = &this.runtime.output.read(cx).live["timelineTransport"];
                    if current["generation"] == this.generation && current["mode"] == "recording" {
                        this.control("stop", cx);
                    }
                    this.retired = true;
                });
                close(cx)
            },
        )
        .child(body)
    }
}
