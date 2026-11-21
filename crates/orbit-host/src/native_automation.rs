use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use chrono::Utc;
use serde_json::{json, Value};

use crate::output_arrangement::{live_slide_index, resolve_live_arrangement_slides};

const MAX_PENDING_DELAYS: usize = 4_096;
const MAX_DELAY_ID_HISTORY: usize = 8_192;

pub struct NativeAutomationScheduler {
    monotonic_origin: Instant,
    active_session: Option<String>,
    retired_sessions: HashSet<String>,
    sequence: u64,
    live_key: String,
    presentation_restart_key: String,
    timeline_cues: Vec<TimelineCue>,
    authored_timeline: crate::native_timeline::TimelineClock,
    soundtrack_plan: Option<crate::native_soundtrack::SoundtrackPlan>,
    executed_timeline_cues: HashSet<String>,
    builds: Vec<ScheduledBuild>,
    emitted_build_steps: HashSet<u64>,
    build_step_started: HashMap<u64, Duration>,
    completion: Option<ScheduledCompletion>,
    completion_emitted: bool,
    completion_media: Option<MarkerPlaybackDeadline>,
    completion_armed_at: Instant,
    pending_cue_actions: Option<Vec<Value>>,
    delayed_actions: Vec<DelayedActions>,
    accepted_delay_ids: HashSet<String>,
    presentation: Option<AutomationPresentationContext>,
    presentation_ready_for_session: bool,
}

impl Default for NativeAutomationScheduler {
    fn default() -> Self {
        Self {
            monotonic_origin: Instant::now(),
            active_session: None,
            retired_sessions: HashSet::new(),
            sequence: 0,
            live_key: String::new(),
            presentation_restart_key: String::new(),
            timeline_cues: Vec::new(),
            authored_timeline: crate::native_timeline::TimelineClock::default(),
            soundtrack_plan: None,
            executed_timeline_cues: HashSet::new(),
            builds: Vec::new(),
            emitted_build_steps: HashSet::new(),
            build_step_started: HashMap::new(),
            completion: None,
            completion_emitted: false,
            completion_media: None,
            completion_armed_at: Instant::now(),
            pending_cue_actions: None,
            delayed_actions: Vec::new(),
            accepted_delay_ids: HashSet::new(),
            presentation: None,
            presentation_ready_for_session: false,
        }
    }
}

#[derive(Clone)]
struct TimelineCue {
    id: String,
    due_at: Duration,
    action: Option<Value>,
    cue_id: Option<String>,
}

#[derive(Clone)]
struct ScheduledBuild {
    step: u64,
    due_at: Duration,
}

#[derive(Clone)]
struct ScheduledCompletion {
    due_at: Duration,
    cue_id: Option<String>,
    media_end: bool,
    delay: Duration,
    foreground_video_paths: Vec<std::path::PathBuf>,
}

#[derive(Clone)]
struct DelayedActions {
    id: String,
    due_at: Duration,
    depth: u64,
    actions: Vec<Value>,
    marker_playback: Option<MarkerPlaybackDeadline>,
}

#[derive(Clone)]
struct MarkerPlaybackDeadline {
    marker_id: Option<String>,
    path: std::path::PathBuf,
    selection_id: Value,
    seconds: f64,
    requested_at: Instant,
    repeat_on_loop: bool,
    emitted_loop: Option<u64>,
    layer: String,
}

#[derive(Clone)]
struct AutomationPresentationContext {
    build_step: u64,
    live_key: String,
    restart_key: String,
    current_index: Option<usize>,
    slides: Vec<AutomationCueTarget>,
    timeline_started_at_ms: Option<i64>,
    build_step_started_at_ms_by_step: std::collections::BTreeMap<u64, i64>,
    timeline_offset_seconds: f64,
    timeline_cues: Value,
    builds: Value,
    completion: Value,
    cue_actions: Vec<Value>,
}

#[derive(Clone)]
struct AutomationCueTarget {
    slide_id: String,
    instance_id: Option<String>,
    source_cue_id: Option<String>,
}

pub(crate) struct AutomationPresentationUpdate {
    timeline_transport: Option<crate::native_timeline::TimelineTransport>,
    authored_timeline: Value,
    soundtrack_plan: Option<crate::native_soundtrack::SoundtrackPlan>,
    presentation: Option<AutomationPresentationContext>,
    timeline_cues: Vec<PreparedTimelineCue>,
    builds: Vec<PreparedBuild>,
    completion: Option<PreparedCompletion>,
}

struct PreparedTimelineCue {
    id: String,
    due_at_ms: i64,
    action: Option<Value>,
    cue_id: Option<String>,
}

struct PreparedBuild {
    step: u64,
    due_at_ms: i64,
    anchor_step: Option<u64>,
    delay: Duration,
}

struct PreparedCompletion {
    due_at_ms: i64,
    cue_id: Option<String>,
    media_end: bool,
    delay_seconds: f64,
    foreground_video_paths: Vec<std::path::PathBuf>,
}

impl NativeAutomationScheduler {
    pub(crate) fn update_completion_media(&mut self, scene: &Value) {
        let Some(completion) = self.completion.as_ref() else {
            self.completion_media = None;
            return;
        };
        let after_foreground_video =
            !completion.media_end && !completion.foreground_video_paths.is_empty();
        if !(completion.media_end || after_foreground_video)
            || scene.get("backgroundMediaType").and_then(Value::as_str) != Some("video")
            || scene.get("backgroundPlayback").and_then(Value::as_str) == Some("loop")
        {
            self.completion_media = None;
            return;
        }
        let Some(path) = scene
            .get("backgroundMediaPath")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
        else {
            self.completion_media = None;
            return;
        };
        let Some(selection_id) = scene
            .get("backgroundSelectionId")
            .filter(|id| !id.is_null())
        else {
            self.completion_media = None;
            return;
        };
        let path = std::path::PathBuf::from(path);
        if after_foreground_video
            && !self
                .completion
                .as_ref()
                .is_some_and(|completion| completion.foreground_video_paths.contains(&path))
        {
            self.completion_media = None;
            return;
        }
        if self
            .completion_media
            .as_ref()
            .is_some_and(|media| media.path == path && media.selection_id == *selection_id)
        {
            return;
        }
        self.completion_media = Some(MarkerPlaybackDeadline {
            marker_id: None,
            path,
            selection_id: selection_id.clone(),
            seconds: 0.0,
            requested_at: self.completion_armed_at,
            repeat_on_loop: false,
            emitted_loop: None,
            layer: "media".into(),
        });
    }

    pub(crate) fn prepare_presentation(
        snapshot: &Value,
    ) -> Result<AutomationPresentationUpdate, String> {
        let presentation = build_presentation_context(snapshot)?;
        let timeline_transport = snapshot
            .get("timelineTransport")
            .filter(|value| !value.is_null())
            .map(|value| {
                serde_json::from_value::<crate::native_timeline::TimelineTransport>(value.clone())
                    .map_err(|error| format!("Invalid timeline transport: {error}"))
            })
            .transpose()?;
        let authored_timeline = snapshot["liveItem"]["timeline"].clone();
        if let Some(transport) = &timeline_transport {
            if transport.item_id.is_empty()
                || transport.generation.is_empty()
                || !transport.position_seconds.is_finite()
                || transport.position_seconds < 0.0
                || transport.position_seconds > 86400.0
                || !transport.playback_rate.is_finite()
                || transport.playback_rate == 0.0
                || transport.playback_rate.abs() > crate::native_timeline::MAX_PLAYBACK_RATE
                || snapshot["liveItem"]["id"].as_str() != Some(&transport.item_id)
            {
                return Err("Timeline transport identity or position is invalid.".into());
            }
            match transport.mode.as_str() {
                "playback" => {
                    crate::native_timeline::validate(
                        &authored_timeline,
                        &snapshot["liveItem"]["slides"],
                    )?;
                    let context = presentation
                        .as_ref()
                        .ok_or("Timeline playback requires a live presentation cue.")?;
                    for cue in authored_timeline["cues"].as_array().unwrap() {
                        if let Some(target) = cue.get("cueId").and_then(Value::as_str) {
                            if !context.slides.iter().any(|slide| {
                                (slide.slide_id == target
                                    || slide.source_cue_id.as_deref() == Some(target))
                                    && cue.get("cueInstanceId").and_then(Value::as_str).is_none_or(
                                        |instance| slide.instance_id.as_deref().is_some_and(|live| crate::output_arrangement::arrangement_instance_matches(live, instance)),
                                    )
                            }) {
                                return Err(
                                    "Timeline cue is unavailable in the active arrangement.".into(),
                                );
                            }
                        }
                    }
                }
                "recording" if transport.playback_rate == 1.0 => (),
                "recording" => {
                    return Err("Timeline recording must run at normal forward speed.".into())
                }
                "stopped" if transport.started_at_ms.is_none() => (),
                _ => return Err("Timeline transport mode is invalid.".into()),
            }
        }
        let soundtrack_plan = match &timeline_transport {
            Some(transport) if matches!(transport.mode.as_str(), "playback" | "recording") => {
                crate::native_soundtrack::plan_from_snapshot(snapshot)?
            }
            _ => None,
        };
        let (timeline_cues, builds, completion) =
            derive_presentation_schedule(presentation.as_ref())?;
        Ok(AutomationPresentationUpdate {
            timeline_transport,
            authored_timeline,
            soundtrack_plan,
            presentation,
            timeline_cues,
            builds,
            completion,
        })
    }

    pub(crate) fn apply_presentation(&mut self, update: AutomationPresentationUpdate) {
        let now = self.monotonic_origin.elapsed();
        self.apply_presentation_at(update, Utc::now().timestamp_millis(), now);
    }

    /// What the soundtrack must do now, or `None` when it must be silent.
    pub(crate) fn soundtrack_target(&self) -> Option<crate::native_soundtrack::SoundtrackTarget> {
        self.soundtrack_target_at(self.monotonic_origin.elapsed())
    }

    pub(crate) fn current_timeline_playhead(&self) -> Option<Value> {
        let head = self
            .authored_timeline
            .playhead(self.monotonic_origin.elapsed())?;
        Some(json!({
            "generation": head.generation,
            "running": head.running,
            "positionSeconds": head.position,
            "playbackRate": head.playback_rate,
        }))
    }

    fn soundtrack_target_at(
        &self,
        now: Duration,
    ) -> Option<crate::native_soundtrack::SoundtrackTarget> {
        self.active_session.as_ref()?;
        let plan = self.soundtrack_plan.clone()?;
        let head = self.authored_timeline.playhead(now)?;
        Some(crate::native_soundtrack::SoundtrackTarget {
            plan,
            generation: head.generation,
            playing: head.running,
            position: head.position,
            playback_rate: head.playback_rate,
        })
    }

    pub(crate) fn presentation_requires_refresh(&self) -> bool {
        self.active_session.is_some() && !self.presentation_ready_for_session
    }

    pub(crate) fn accepts_due_event(&self, event: &Value) -> bool {
        let Some(session_id) = event.get("sessionId").and_then(Value::as_str) else {
            return false;
        };
        if self.active_session.as_deref() != Some(session_id) {
            return false;
        }
        if let Some(generation) = event.get("timelineGeneration") {
            return generation.as_str().is_some_and(|generation| {
                self.authored_timeline.running_generation() == Some(generation)
            });
        }
        if event.get("source").and_then(Value::as_str) == Some("timeline")
            && self.authored_timeline.active()
        {
            return false;
        }
        match event.get("liveKey") {
            None => true,
            Some(Value::String(live_key)) => {
                self.presentation_ready_for_session && live_key == &self.live_key
            }
            Some(_) => false,
        }
    }

    fn apply_presentation_at(
        &mut self,
        update: AutomationPresentationUpdate,
        wall_now_ms: i64,
        monotonic_now: Duration,
    ) {
        self.authored_timeline.update(
            update.timeline_transport,
            update.authored_timeline,
            wall_now_ms,
            monotonic_now,
        );
        self.soundtrack_plan = update.soundtrack_plan;
        let live_key = update
            .presentation
            .as_ref()
            .map(|context| context.live_key.as_str())
            .unwrap_or_default();
        let restart_key = update
            .presentation
            .as_ref()
            .map(|context| context.restart_key.as_str())
            .unwrap_or_default();
        let restart_identity_changed = restart_key != self.presentation_restart_key;
        if restart_identity_changed {
            self.live_key = live_key.to_string();
            self.presentation_restart_key = restart_key.to_string();
            self.executed_timeline_cues.clear();
            self.emitted_build_steps.clear();
            self.build_step_started.clear();
            self.completion_emitted = false;
            self.completion_media = None;
            self.completion_armed_at = Instant::now();
            self.pending_cue_actions = update
                .presentation
                .as_ref()
                .filter(|context| context.timeline_started_at_ms.is_some())
                .map(|context| context.cue_actions.clone())
                .filter(|actions| !actions.is_empty());
        }
        if let Some(context) = &update.presentation {
            if let Some(started_at_ms) = context
                .build_step_started_at_ms_by_step
                .get(&0)
                .copied()
                .or(context.timeline_started_at_ms)
            {
                self.build_step_started.insert(
                    0,
                    monotonic_deadline(started_at_ms, wall_now_ms, monotonic_now),
                );
            }
            if context.build_step > 0 {
                if !context
                    .build_step_started_at_ms_by_step
                    .contains_key(&context.build_step)
                {
                    self.build_step_started
                        .entry(context.build_step)
                        .or_insert(monotonic_now);
                }
                for build in context.builds.as_array().into_iter().flatten() {
                    if let Some(step) = build
                        .get("anchorStep")
                        .and_then(Value::as_u64)
                        .filter(|step| *step <= context.build_step)
                    {
                        self.build_step_started.entry(step).or_insert(monotonic_now);
                    }
                }
            }
            for (&step, &started_at_ms) in &context.build_step_started_at_ms_by_step {
                self.build_step_started.insert(
                    step,
                    monotonic_deadline(started_at_ms, wall_now_ms, monotonic_now),
                );
            }
        }
        self.presentation = update.presentation;
        self.timeline_cues = update
            .timeline_cues
            .into_iter()
            .map(|cue| TimelineCue {
                id: cue.id,
                due_at: monotonic_deadline(cue.due_at_ms, wall_now_ms, monotonic_now),
                action: cue.action,
                cue_id: cue.cue_id,
            })
            .collect();
        self.builds = update
            .builds
            .into_iter()
            .filter_map(|build| {
                Some(ScheduledBuild {
                    step: build.step,
                    due_at: match build.anchor_step {
                        Some(anchor) => self
                            .build_step_started
                            .get(&anchor)?
                            .saturating_add(build.delay),
                        None => monotonic_deadline(build.due_at_ms, wall_now_ms, monotonic_now),
                    },
                })
            })
            .collect();
        let previous = self
            .completion
            .as_ref()
            .map(|completion| (completion.delay, completion.due_at));
        self.completion = update.completion.map(|completion| {
            let delay = add_duration_seconds(Duration::ZERO, completion.delay_seconds);
            // A timer set or changed on the slide that is already on air stays unarmed until the next take
            // (a new timeline start); an unchanged one keeps its deadline.
            let due_at = match previous {
                _ if restart_identity_changed => {
                    monotonic_deadline(completion.due_at_ms, wall_now_ms, monotonic_now)
                }
                Some((previous_delay, due_at)) if previous_delay == delay => due_at,
                _ => {
                    self.completion_emitted = true;
                    monotonic_now.saturating_add(delay)
                }
            };
            ScheduledCompletion {
                due_at,
                cue_id: completion.cue_id,
                media_end: completion.media_end,
                delay,
                foreground_video_paths: completion.foreground_video_paths,
            }
        });
        self.presentation_ready_for_session = self.active_session.is_some();
    }

    pub fn update(&mut self, snapshot: &Value) -> Result<Value, String> {
        let document = snapshot
            .as_object()
            .ok_or_else(|| "Automation schedule must be an object.".to_string())?;
        if document
            .get("cancelPendingMarkerLayer")
            .is_some_and(|layer| !matches!(layer.as_str(), Some("media" | "music")))
        {
            return Err("Marker cancellation layer must be media or music.".into());
        }
        let session_id = non_empty_string(document.get("sessionId"), "Automation session id")?;
        let sequence = document
            .get("sequence")
            .and_then(Value::as_u64)
            .ok_or_else(|| "Automation sequence must be a non-negative integer.".to_string())?;

        if self.retired_sessions.contains(&session_id) {
            return Ok(json!({ "ok": true, "stale": true }));
        }
        if self.active_session.as_deref() == Some(session_id.as_str()) && sequence <= self.sequence
        {
            return Ok(json!({ "ok": true, "stale": true }));
        }

        let cancel_pending_delays = document
            .get("cancelPendingDelays")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if self.active_session.as_deref() != Some(session_id.as_str()) {
            let first_session = self.active_session.is_none();
            if let Some(previous) = self.active_session.replace(session_id) {
                self.retired_sessions.insert(previous);
                self.reset_presentation();
            }
            self.delayed_actions.clear();
            self.accepted_delay_ids.clear();
            self.presentation_ready_for_session = first_session && self.presentation.is_some();
        }
        self.sequence = sequence;
        if document
            .get("cancelPendingMarkers")
            .and_then(Value::as_bool)
            == Some(true)
        {
            self.delayed_actions.retain(|entry| {
                entry.marker_playback.as_ref().is_none_or(|marker| {
                    document
                        .get("cancelPendingMarkerLayer")
                        .and_then(Value::as_str)
                        .is_some_and(|layer| marker.layer != layer)
                })
            });
        }
        if cancel_pending_delays {
            self.delayed_actions.clear();
            self.reset_presentation();
        }

        Ok(json!({ "ok": true, "stale": false }))
    }

    fn tick_at(&mut self, wall_now_ms: i64, monotonic_now: Duration) -> Vec<Value> {
        let mut due = Vec::new();

        let mut pending = Vec::with_capacity(self.delayed_actions.len());
        for mut delayed in self.delayed_actions.drain(..) {
            let ready = delayed.marker_playback.as_mut().map_or_else(
                || delayed.due_at <= monotonic_now,
                |marker| {
                    crate::native_render::measured_marker_sample(
                        &marker.path,
                        &if marker.layer == "music" {
                            json!(format!("audio:{}", marker.selection_id))
                        } else {
                            marker.selection_id.clone()
                        },
                        marker.requested_at,
                    )
                    .is_some_and(|(position, iteration)| {
                        let ready = position >= marker.seconds
                            && marker
                                .emitted_loop
                                .is_none_or(|emitted| marker.repeat_on_loop && iteration > emitted);
                        if ready {
                            marker.emitted_loop = Some(iteration);
                        }
                        ready
                    })
                },
            );
            if ready {
                due.push(json!({
                    "kind": "actions",
                    "source": "delay",
                    "id": delayed.id,
                    "depth": delayed.depth,
                    "actions": delayed.actions
                }));
                if delayed
                    .marker_playback
                    .as_ref()
                    .is_some_and(|marker| marker.repeat_on_loop || marker.marker_id.is_some())
                {
                    pending.push(delayed);
                }
            } else {
                pending.push(delayed);
            }
        }
        self.delayed_actions = pending;
        self.prune_delay_history();

        if self.active_session.is_some()
            && self.presentation_ready_for_session
            && !self.live_key.is_empty()
        {
            if let Some(actions) = self.pending_cue_actions.take() {
                due.push(json!({
                    "kind": "actions",
                    "source": "cue",
                    "id": format!("cue:{}", self.presentation_restart_key),
                    "liveKey": self.live_key,
                    "actions": actions
                }));
            }
            for cue in self
                .timeline_cues
                .iter()
                .filter(|_| !self.authored_timeline.active())
            {
                if cue.due_at > monotonic_now || !self.executed_timeline_cues.insert(cue.id.clone())
                {
                    continue;
                }
                if let Some(action) = &cue.action {
                    due.push(json!({
                        "kind": "actions",
                        "source": "timeline",
                        "id": cue.id,
                        "liveKey": self.live_key,
                        "actions": [action]
                    }));
                } else if let Some(cue_id) = &cue.cue_id {
                    let mut event = json!({
                        "kind": "cue",
                        "source": "timeline",
                        "id": cue.id,
                        "liveKey": self.live_key,
                        "cueId": cue_id,
                        "advanceFromCurrent": false
                    });
                    attach_cue_target(
                        &mut event,
                        self.presentation.as_ref(),
                        &self.live_key,
                        Some(cue_id),
                        false,
                    );
                    due.push(event);
                }
            }

            for cue in self.authored_timeline.tick(monotonic_now) {
                let mut event = if let Some(action) =
                    cue.get("action").filter(|value| value.is_object())
                {
                    json!({"kind":"actions", "source":"timeline", "id":cue["id"], "liveKey":self.live_key, "actions":[action]})
                } else {
                    json!({"kind":"cue", "source":"timeline", "id":cue["id"], "liveKey":self.live_key, "cueId":cue["cueId"], "advanceFromCurrent":false})
                };
                if event["kind"] == "cue" {
                    if let Some(target) = self.presentation.as_ref().and_then(|context| {
                        context.slides.iter().find(|slide| {
                            (Some(slide.slide_id.as_str()) == cue["cueId"].as_str()
                                || slide.source_cue_id.as_deref() == cue["cueId"].as_str())
                                && cue.get("cueInstanceId").and_then(Value::as_str).is_none_or(
                                    |instance| {
                                        slide.instance_id.as_deref().is_some_and(|live| {
                                            crate::output_arrangement::arrangement_instance_matches(
                                                live, instance,
                                            )
                                        })
                                    },
                                )
                        })
                    }) {
                        event["targetSlideId"] = json!(target.slide_id);
                        event["targetCueInstanceId"] = target
                            .instance_id
                            .as_ref()
                            .map_or(Value::Null, |id| json!(id));
                    }
                }
                event["timelineGeneration"] = self
                    .authored_timeline
                    .running_generation()
                    .map_or(Value::Null, |generation| json!(generation));
                due.push(event);
            }

            for build in &self.builds {
                if build.due_at <= monotonic_now && self.emitted_build_steps.insert(build.step) {
                    self.build_step_started
                        .entry(build.step)
                        .or_insert(build.due_at);
                    due.push(json!({
                        "kind": "build",
                        "liveKey": self.live_key,
                        "step": build.step,
                        "startedAtMs": wall_timestamp_at_deadline(build.due_at, wall_now_ms, monotonic_now)
                    }));
                }
            }

            if !self.completion_emitted {
                if let Some(completion) = &self.completion {
                    let ready = if completion.media_end {
                        self.completion_media.as_ref().is_some_and(|media| {
                            crate::native_render::measured_media_ended(
                                &media.path,
                                &media.selection_id,
                                media.requested_at,
                            )
                        })
                    } else if !completion.foreground_video_paths.is_empty() {
                        // ProPresenter holds a slide's foreground video on its last frame
                        // for the slide timer, so the delay starts at the decoded end.
                        self.completion_media
                            .as_ref()
                            .and_then(|media| {
                                crate::native_render::measured_media_end(
                                    &media.path,
                                    &media.selection_id,
                                    media.requested_at,
                                )
                            })
                            .is_some_and(|ended| {
                                ended
                                    .saturating_duration_since(self.monotonic_origin)
                                    .saturating_add(completion.delay)
                                    <= monotonic_now
                            })
                    } else {
                        completion.due_at <= monotonic_now
                    };
                    if ready {
                        self.completion_emitted = true;
                        let mut event = json!({
                            "kind": "cue",
                            "source": "completion",
                            "id": format!("completion:{}", self.live_key),
                            "liveKey": self.live_key,
                            "advanceFromCurrent": true
                        });
                        if let Some(cue_id) = &completion.cue_id {
                            event["cueId"] = json!(cue_id);
                        }
                        attach_cue_target(
                            &mut event,
                            self.presentation.as_ref(),
                            &self.live_key,
                            completion.cue_id.as_deref(),
                            true,
                        );
                        due.push(event);
                    }
                }
            }
        }
        if let Some(session_id) = &self.active_session {
            for event in &mut due {
                event["sessionId"] = json!(session_id);
            }
        }
        due
    }

    /// The running slide-timer countdown of the live slide, or `None` while nothing is counting down: an
    /// elapsed-time completion that has a slide to go to and does not wait for a video to end.
    pub fn completion_countdown(&self) -> Option<Value> {
        self.completion_countdown_at(self.monotonic_origin.elapsed())
    }

    fn completion_countdown_at(&self, monotonic_now: Duration) -> Option<Value> {
        let completion = self.completion.as_ref()?;
        if self.completion_emitted
            || completion.media_end
            || !completion.foreground_video_paths.is_empty()
            || self.live_key.is_empty()
            || !self.presentation_ready_for_session
        {
            return None;
        }
        let mut target = json!({});
        attach_cue_target(
            &mut target,
            self.presentation.as_ref(),
            &self.live_key,
            completion.cue_id.as_deref(),
            true,
        );
        target.get("targetSlideId")?;
        Some(json!({
            "liveKey": self.live_key,
            "delaySeconds": completion.delay.as_secs_f64(),
            "remainingSeconds": completion.due_at.saturating_sub(monotonic_now).as_secs_f64()
        }))
    }

    /// Stops the pending slide timer of the live slide for good: only a new take of the slide arms it again.
    /// The saved slide is not touched. A key that is no longer live is answered as `cancelled: false`.
    pub fn cancel_completion(&mut self, live_key: &Value) -> Result<Value, String> {
        let live_key = live_key
            .as_str()
            .filter(|key| !key.is_empty())
            .ok_or("Slide completion live key must be a non-empty string.")?;
        let cancelled =
            live_key == self.live_key && self.completion.is_some() && !self.completion_emitted;
        if cancelled {
            self.completion_emitted = true;
        }
        Ok(json!({ "cancelled": cancelled }))
    }

    pub fn tick_now(&mut self) -> Vec<Value> {
        self.tick_at(
            Utc::now().timestamp_millis(),
            self.monotonic_origin.elapsed(),
        )
    }

    pub fn schedule_actions(&mut self, request: &Value) -> Result<Value, String> {
        let now = self.monotonic_origin.elapsed();
        self.schedule_actions_at(request, now)
    }

    pub(crate) fn marker_jump_target(
        &self,
        path: &str,
        selection: &Value,
        layer: &str,
        id: &str,
    ) -> Result<(usize, f64), String> {
        if self.active_session.is_none() {
            return Err("Playback marker session is unavailable.".into());
        }
        self.delayed_actions
            .iter()
            .enumerate()
            .find_map(|(index, entry)| {
                let marker = entry.marker_playback.as_ref()?;
                (marker.path == std::path::Path::new(path)
                    && &marker.selection_id == selection
                    && marker.layer == layer
                    && marker.marker_id.as_deref() == Some(id))
                .then_some((index, marker.seconds))
            })
            .ok_or_else(|| "Playback marker is unavailable in this selection.".into())
    }

    pub(crate) fn jump_marker(&mut self, index: usize, iteration: u64) -> Value {
        let entry = &mut self.delayed_actions[index];
        entry
            .marker_playback
            .as_mut()
            .expect("validated marker target")
            .emitted_loop = Some(iteration);
        json!({
            "kind": "actions", "source": "delay", "id": entry.id,
            "depth": entry.depth, "actions": entry.actions,
            "sessionId": self.active_session
        })
    }

    fn schedule_actions_at(
        &mut self,
        request: &Value,
        monotonic_now: Duration,
    ) -> Result<Value, String> {
        let document = request
            .as_object()
            .ok_or_else(|| "Delayed automation request must be an object.".to_string())?;
        let session_id = non_empty_string(document.get("sessionId"), "Automation session id")?;
        if self.active_session.as_deref() != Some(session_id.as_str()) {
            return Ok(json!({ "ok": true, "stale": true }));
        }
        let request_id =
            non_empty_string(document.get("requestId"), "Delayed automation request id")?;
        let delay_seconds = finite_number(document.get("delaySeconds"))
            .ok_or_else(|| "Delayed automation duration must be finite.".to_string())?;
        if delay_seconds < 0.0 {
            return Err("Delayed automation duration must be non-negative.".to_string());
        }
        let depth = document
            .get("depth")
            .and_then(Value::as_u64)
            .filter(|depth| *depth <= 8)
            .ok_or_else(|| {
                "Delayed automation depth must be an integer from 0 through 8.".to_string()
            })?;
        let actions = document
            .get("actions")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| "Delayed automation actions must be an array.".to_string())?;
        if (actions.is_empty() && !document.contains_key("markerPlayback"))
            || actions.iter().any(|action| !action.is_object())
        {
            return Err("Delayed automation actions must contain action objects.".to_string());
        }
        let marker_playback = document.get("markerPlayback").map(|marker| {
            let path = non_empty_string(marker.get("path"), "Marker playback path")?;
            let selection_id = marker.get("selectionId").filter(|id| id.as_u64().is_some() || id.as_str().is_some_and(|id| !id.is_empty())).cloned().ok_or_else(|| "Marker playback selection must be a non-negative integer or non-empty string.".to_string())?;
            let repeat_on_loop = match marker.get("repeatOnLoop") { None => false, Some(Value::Bool(value)) => *value, Some(_) => return Err("Marker repeatOnLoop must be a boolean.".into()) };
            let layer = match marker.get("layer") { None => "media", Some(Value::String(value)) if matches!(value.as_str(), "media" | "music") => value.as_str(), Some(_) => return Err("Marker layer must be media or music.".into()) };
            let marker_id = marker.get("markerId").map(|id| non_empty_string(Some(id), "Playback marker id")).transpose()?;
            Ok::<_, String>(MarkerPlaybackDeadline { marker_id, layer: layer.into(), path: path.into(), selection_id, seconds: delay_seconds, requested_at: Instant::now(), repeat_on_loop, emitted_loop: None })
        }).transpose()?;
        if !self.accepted_delay_ids.insert(request_id.clone()) {
            return Ok(json!({ "ok": true, "stale": false }));
        }
        if self.delayed_actions.len() >= MAX_PENDING_DELAYS {
            self.accepted_delay_ids.remove(&request_id);
            return Err("Too many delayed automation requests are pending.".to_string());
        }
        self.delayed_actions.push(DelayedActions {
            id: request_id,
            due_at: add_duration_seconds(monotonic_now, delay_seconds),
            depth,
            actions,
            marker_playback,
        });
        self.delayed_actions.sort_by_key(|entry| entry.due_at);
        Ok(json!({ "ok": true, "stale": false }))
    }

    fn prune_delay_history(&mut self) {
        if self.accepted_delay_ids.len() <= MAX_DELAY_ID_HISTORY {
            return;
        }
        let pending: HashSet<&str> = self
            .delayed_actions
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        let mut completed_to_keep = MAX_DELAY_ID_HISTORY.saturating_sub(pending.len());
        self.accepted_delay_ids.retain(|id| {
            if pending.contains(id.as_str()) {
                true
            } else if completed_to_keep > 0 {
                completed_to_keep -= 1;
                true
            } else {
                false
            }
        });
    }

    fn reset_presentation(&mut self) {
        self.authored_timeline
            .update(None, Value::Null, 0, Duration::ZERO);
        self.build_step_started.clear();
        self.live_key.clear();
        self.presentation_restart_key.clear();
        self.timeline_cues.clear();
        self.executed_timeline_cues.clear();
        self.builds.clear();
        self.emitted_build_steps.clear();
        self.completion = None;
        self.completion_emitted = false;
        self.completion_media = None;
        self.pending_cue_actions = None;
        self.presentation = None;
        self.soundtrack_plan = None;
        self.presentation_ready_for_session = false;
    }
}

fn build_presentation_context(
    snapshot: &Value,
) -> Result<Option<AutomationPresentationContext>, String> {
    let document = snapshot.as_object().ok_or_else(|| {
        "Output snapshot must be an object for automation cue resolution.".to_string()
    })?;
    let Some(item) = document.get("liveItem").filter(|item| item.is_object()) else {
        return Ok(None);
    };
    let Some(item_id) = item
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return Ok(None);
    };
    let Some(live_slide_id) = document
        .get("liveSlideId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return Ok(None);
    };
    let live_instance_id = document
        .get("liveCueInstanceId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let live_playlist_entry_id = document
        .get("livePlaylistEntryId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let timeline_started_at_ms = document.get("timelineStartedAtMs").and_then(json_i64);
    let build_step = safe_integer(document.get("liveBuildStep")).unwrap_or(0);
    let build_step_started_at_ms_by_step = parse_build_step_clock(
        document.get("liveBuildStepStartedAtMsByStep"),
        build_step,
        timeline_started_at_ms,
    )?;
    let resolved = resolve_live_arrangement_slides(item, live_instance_id, live_playlist_entry_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|slide| {
            !slide
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let current_index = live_slide_index(&resolved, live_slide_id, live_instance_id);
    let slides = resolved
        .iter()
        .filter_map(|slide| {
            let slide_id = slide.get("id")?.as_str()?.to_string();
            Some(AutomationCueTarget {
                slide_id,
                instance_id: optional_non_empty_string(slide.get("arrangementInstanceId")),
                source_cue_id: optional_non_empty_string(slide.get("sourceCueId")),
            })
        })
        .collect();
    let timeline = item.get("timeline").and_then(Value::as_object);
    let timeline_offset_seconds = timeline
        .and_then(|timeline| finite_number(timeline.get("timecodeOffsetSeconds")))
        .unwrap_or(0.0);
    let timeline_cues = timeline
        .and_then(|timeline| timeline.get("cues"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    if !timeline_cues.is_array() {
        return Err("Live presentation timeline cues must be an array.".to_string());
    }
    let live_slide = document
        .get("liveSlide")
        .filter(|slide| slide.is_object())
        .unwrap_or(&Value::Null);
    let composed;
    let build_slide = if live_slide.get("primaryTextLayerId").is_some() {
        let input: crate::output_scene::OutputSceneStateInput =
            crate::output_scene::parse_scene_state(snapshot)
                .map_err(|error| format!("Invalid authored slide automation snapshot: {error}"))?;
        let theme = crate::output_scene::resolve_content_theme_id(&input)
            .and_then(|id| crate::output_scene::resolve_look_theme_slide(&input.themes, &id));
        composed = crate::slide_composition::compose_slide(live_slide, theme);
        &composed
    } else {
        live_slide
    };
    let slide_transition = crate::output_scene::effective_slide_transition_value(
        document.get("liveSlide"),
        document.get("liveItem"),
        document.get("slideTransition"),
    )?;
    let builds = build_relative_builds(build_slide, slide_transition.as_ref())?;
    let completion = build_relative_completion(
        live_slide,
        document
            .get("media")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default(),
    );
    let cue_actions = build_cue_actions(live_slide)?;
    let live_key = format!(
        "{item_id}:{live_slide_id}:{}",
        live_instance_id.unwrap_or_default()
    );
    Ok(Some(AutomationPresentationContext {
        build_step,
        restart_key: format!(
            "{live_key}:{}:{}",
            live_playlist_entry_id.unwrap_or_default(),
            timeline_started_at_ms.map_or_else(String::new, |value| value.to_string())
        ),
        live_key,
        current_index,
        slides,
        timeline_started_at_ms,
        build_step_started_at_ms_by_step,
        timeline_offset_seconds,
        timeline_cues,
        builds,
        completion,
        cue_actions,
    }))
}

fn build_cue_actions(slide: &Value) -> Result<Vec<Value>, String> {
    let Some(actions) = slide.get("actions") else {
        return Ok(Vec::new());
    };
    let actions = actions
        .as_array()
        .ok_or_else(|| "Live slide actions must be an array.".to_string())?;
    if actions.iter().any(|action| !action.is_object()) {
        return Err("Live slide actions must contain action objects.".to_string());
    }
    let mut skipped_primary_slide = false;
    Ok(actions
        .iter()
        .filter(|action| {
            let is_primary = !skipped_primary_slide
                && action.get("type").and_then(Value::as_str) == Some("presentation-slide");
            if is_primary {
                skipped_primary_slide = true;
                false
            } else {
                true
            }
        })
        .cloned()
        .collect())
}

fn build_relative_builds(slide: &Value, slide_transition: Option<&Value>) -> Result<Value, String> {
    let slide_transition_seconds = slide_transition
        .map(crate::native_transitions::resolve)
        .transpose()?
        .flatten()
        .map_or(0.0, |spec| spec.duration_seconds);
    let mut objects = Vec::new();
    for layer in slide
        .get("layers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|layer| layer.get("hidden").and_then(Value::as_bool) != Some(true))
    {
        objects.extend(["build", "buildOut"].into_iter().filter_map(|key| {
            let build = layer.get(key).filter(|build| build.is_object())?;
            Some((build, build.get("transition")))
        }));
        let inherited_transition = layer
            .get("build")
            .and_then(Value::as_object)
            .and_then(|build| build.get("transition"));
        objects.extend(
            layer
                .get("textReveal")
                .and_then(|reveal| reveal.get("children"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|child| child.as_object().map(|_| (child, inherited_transition))),
        );
    }
    objects.sort_by_key(|(build, _)| safe_integer(build.get("step")).unwrap_or(0));
    let mut builds = Vec::new();
    for (build, _) in &objects {
        let Some(step) = safe_integer(build.get("step")) else {
            continue;
        };
        if build.get("start").and_then(Value::as_str) != Some("after-previous")
            || objects.iter().any(|(other, _)| {
                safe_integer(other.get("step")) == Some(step)
                    && other.get("start").and_then(Value::as_str) != Some("after-previous")
            })
        {
            continue;
        }
        let anchor = objects
            .iter()
            .filter_map(|(other, _)| safe_integer(other.get("step")))
            .filter(|previous| *previous < step)
            .max()
            .unwrap_or(0);
        let anchor_completion_seconds = objects.iter().try_fold(
            0.0_f64,
            |longest, (other, inherited_transition)| -> Result<f64, String> {
                if safe_integer(other.get("step")) != Some(anchor) {
                    return Ok(longest);
                }
                Ok(longest.max(relative_build_completion_seconds(
                    other,
                    *inherited_transition,
                )?))
            },
        )?;
        let step_zero_has_object = anchor == 0
            && objects
                .iter()
                .any(|(other, _)| safe_integer(other.get("step")) == Some(0));
        let anchor_completion_seconds = if anchor == 0 && !step_zero_has_object {
            anchor_completion_seconds.max(slide_transition_seconds)
        } else {
            anchor_completion_seconds
        };
        let delay_seconds = relative_build_delay_seconds(build)? + anchor_completion_seconds;
        if !delay_seconds.is_finite() {
            return Err("Object Build relative delay must be finite.".into());
        }
        builds.push(json!({
            "delaySeconds": delay_seconds,
            "step": step,
            "anchorStep": anchor
        }));
    }
    Ok(Value::Array(builds))
}

fn relative_build_completion_seconds(
    build: &Value,
    transition: Option<&Value>,
) -> Result<f64, String> {
    // The scheduler already includes an after-previous delay in this step's start time.
    let delay_seconds = if build.get("start").and_then(Value::as_str) == Some("after-previous") {
        0.0
    } else {
        relative_build_delay_seconds(build)?
    };
    let transition_seconds = transition
        .map(|transition| {
            crate::native_transitions::resolve(transition)
                .map_err(|error| format!("Invalid object Build transition: {error}"))
        })
        .transpose()?
        .flatten()
        .map_or(0.0, |spec| spec.duration_seconds);
    Ok(delay_seconds + transition_seconds)
}

fn relative_build_delay_seconds(build: &Value) -> Result<f64, String> {
    match build.get("delaySeconds") {
        None => Ok(0.0),
        Some(delay) => finite_number(Some(delay))
            .filter(|delay| *delay >= 0.0)
            .ok_or_else(|| "Object Build delaySeconds must be finite and nonnegative.".into()),
    }
}

pub(crate) fn build_relative_completion(slide: &Value, media: &[Value]) -> Value {
    let media_end = slide.get("completionAction").and_then(Value::as_str) == Some("ON_MEDIA_END");
    let enabled = slide
        .get("completionAction")
        .and_then(Value::as_str)
        .is_some_and(|action| action.ends_with("AFTER_TIME"));
    let delay_seconds = if media_end {
        Some(0.0)
    } else {
        numeric_value(slide.get("completionTimeSeconds"))
            .filter(|seconds| enabled && *seconds > 0.0)
    };
    let Some(delay_seconds) = delay_seconds else {
        return Value::Null;
    };
    let mut completion = json!({ "delaySeconds": delay_seconds, "mediaEnd": media_end });
    if let Some(cue_id) = optional_non_empty_string(slide.get("completionTargetCueId")) {
        completion["cueId"] = json!(cue_id);
    }
    if let Some(target_type) = optional_non_empty_string(slide.get("completionTargetType")) {
        completion["targetType"] = json!(target_type);
    }
    if let Some(instance) = optional_non_empty_string(slide.get("completionTargetInstanceId")) {
        completion["instanceId"] = json!(instance);
    }
    if !media_end {
        let paths = foreground_video_paths(slide, media);
        if !paths.is_empty() {
            completion["foregroundVideoPaths"] = json!(paths);
        }
    }
    completion
}

fn foreground_video_paths(slide: &Value, media: &[Value]) -> Vec<String> {
    let mut paths = Vec::new();
    for action in slide
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if action.get("type").and_then(Value::as_str) != Some("media")
            || action.get("enabled").and_then(Value::as_bool) == Some(false)
        {
            continue;
        }
        let Some(target) = optional_non_empty_string(action.get("targetId")) else {
            continue;
        };
        let Some(item) = media.iter().find(|item| {
            item.get("id").and_then(Value::as_str) == Some(target.as_str())
                || item.get("sourceId").and_then(Value::as_str) == Some(target.as_str())
        }) else {
            continue;
        };
        let details = action.get("details");
        let loops = details
            .and_then(|details| details.get("playback"))
            .and_then(Value::as_str)
            .is_some_and(|playback| playback.contains("LOOP"));
        let behavior = details
            .and_then(|details| details.get("backgroundBehavior"))
            .and_then(Value::as_str)
            .filter(|behavior| matches!(*behavior, "background" | "foreground"))
            .or_else(|| item.get("backgroundBehavior").and_then(Value::as_str));
        let Some(path) = optional_non_empty_string(item.get("sourcePath")) else {
            continue;
        };
        if item.get("type").and_then(Value::as_str) == Some("video")
            && !loops
            && behavior != Some("background")
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    }
    paths
}

pub(crate) fn validate_slide_completion(slide: &Value) -> Result<(), String> {
    let action = slide.get("completionAction").and_then(Value::as_str);
    if action.is_some_and(|action| {
        !matches!(
            action,
            "AFTER_TIME" | "COMPLETION_ACTION_TYPE_AFTER_TIME" | "ON_MEDIA_END"
        )
    }) {
        return Ok(());
    }
    if matches!(
        action,
        Some("AFTER_TIME" | "COMPLETION_ACTION_TYPE_AFTER_TIME")
    ) && !finite_number(slide.get("completionTimeSeconds")).is_some_and(|seconds| seconds > 0.0)
    {
        return Err("Slide completion requires a positive finite delay.".to_string());
    }
    if action == Some("ON_MEDIA_END")
        && slide
            .get("completionTimeSeconds")
            .is_some_and(|value| !value.is_null())
    {
        return Err("Media-end completion cannot carry an elapsed-time deadline.".to_string());
    }
    if let Some(target) = slide.get("completionTargetType") {
        if !target.as_str().is_some_and(|target| {
            matches!(
                target,
                "NONE" | "NEXT" | "FIRST" | "RANDOM" | "CUE" | "UNKNOWN"
            )
        }) {
            return Err("Slide completion target type is invalid.".to_string());
        }
        if target == "CUE"
            && optional_non_empty_string(slide.get("completionTargetCueId")).is_none()
            && optional_non_empty_string(slide.get("completionTargetInstanceId")).is_none()
        {
            return Err("Slide completion cue target is required.".to_string());
        }
    }
    Ok(())
}

fn derive_presentation_schedule(
    presentation: Option<&AutomationPresentationContext>,
) -> Result<
    (
        Vec<PreparedTimelineCue>,
        Vec<PreparedBuild>,
        Option<PreparedCompletion>,
    ),
    String,
> {
    let Some(presentation) = presentation else {
        return Ok((Vec::new(), Vec::new(), None));
    };
    Ok((
        parse_timeline_cues(
            Some(&presentation.timeline_cues),
            presentation.timeline_started_at_ms,
            presentation.timeline_offset_seconds,
        )?,
        parse_builds(
            Some(&presentation.builds),
            presentation.timeline_started_at_ms,
        )?,
        parse_completion(
            Some(&presentation.completion),
            presentation.timeline_started_at_ms,
        )?,
    ))
}

fn attach_cue_target(
    event: &mut Value,
    presentation: Option<&AutomationPresentationContext>,
    live_key: &str,
    cue_id: Option<&str>,
    advance_from_current: bool,
) {
    let Some(presentation) = presentation.filter(|context| context.live_key == live_key) else {
        return;
    };
    let candidate_start = if advance_from_current {
        presentation
            .current_index
            .map_or(0, |index| index.saturating_add(1))
    } else {
        0
    };
    let candidates = presentation
        .slides
        .get(candidate_start..)
        .unwrap_or_default();
    let completion = &presentation.completion;
    let target_type = completion
        .get("targetType")
        .and_then(Value::as_str)
        .map(|value| {
            value
                .strip_prefix("COMPLETION_TARGET_TYPE_")
                .unwrap_or(value)
        });
    if advance_from_current && matches!(target_type, Some("NONE" | "UNKNOWN")) {
        return;
    }
    let explicit = advance_from_current
        .then(|| completion.get("instanceId").and_then(Value::as_str))
        .flatten();
    let target = if let Some(instance) = explicit {
        presentation
            .slides
            .iter()
            .find(|slide| slide.instance_id.as_deref() == Some(instance))
    } else if advance_from_current && target_type == Some("FIRST") {
        presentation.slides.first()
    } else if advance_from_current && target_type == Some("NEXT") {
        candidates.first()
    } else if advance_from_current && target_type == Some("CUE") && cue_id.is_none() {
        None
    } else if advance_from_current && target_type == Some("RANDOM") {
        let candidates = presentation
            .slides
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != presentation.current_index)
            .map(|(_, slide)| slide)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            None
        } else {
            candidates
                .get((uuid::Uuid::new_v4().as_u128() % candidates.len() as u128) as usize)
                .copied()
        }
    } else {
        cue_id
            .and_then(|cue_id| {
                candidates
                    .iter()
                    .find(|slide| slide.source_cue_id.as_deref() == Some(cue_id))
                    .or_else(|| {
                        presentation
                            .slides
                            .iter()
                            .find(|slide| slide.source_cue_id.as_deref() == Some(cue_id))
                    })
            })
            .or_else(|| {
                cue_id.filter(|_| advance_from_current).and_then(|cue_id| {
                    candidates
                        .iter()
                        .find(|slide| slide.slide_id == cue_id)
                        .or_else(|| {
                            presentation
                                .slides
                                .iter()
                                .find(|slide| slide.slide_id == cue_id)
                        })
                })
            })
            .or_else(|| cue_id.is_none().then(|| candidates.first()).flatten())
    };
    let Some(target) = target else {
        return;
    };
    event["targetSlideId"] = json!(target.slide_id);
    event["targetCueInstanceId"] = target
        .instance_id
        .as_ref()
        .map_or(Value::Null, |instance_id| json!(instance_id));
}

fn parse_timeline_cues(
    value: Option<&Value>,
    started_at_ms: Option<i64>,
    offset_seconds: f64,
) -> Result<Vec<PreparedTimelineCue>, String> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| "Automation timeline cues must be an array.".to_string())?;
    let Some(started_at_ms) = started_at_ms else {
        return Ok(Vec::new());
    };
    let mut cues = Vec::new();
    for value in values {
        let Some(document) = value.as_object() else {
            continue;
        };
        let id = string_value(document.get("id"));
        let Some(time_seconds) = finite_number(document.get("timeSeconds")) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        cues.push(PreparedTimelineCue {
            id,
            due_at_ms: add_seconds(started_at_ms, time_seconds - offset_seconds),
            action: document
                .get("action")
                .filter(|action| action.is_object())
                .cloned(),
            cue_id: optional_non_empty_string(document.get("cueId")),
        });
    }
    Ok(cues)
}

fn parse_builds(
    value: Option<&Value>,
    started_at_ms: Option<i64>,
) -> Result<Vec<PreparedBuild>, String> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| "Automation builds must be an array.".to_string())?;
    let Some(started_at_ms) = started_at_ms else {
        return Ok(Vec::new());
    };
    Ok(values
        .iter()
        .filter_map(|value| {
            let document = value.as_object()?;
            let step = document.get("step")?.as_u64()?;
            let delay_seconds = finite_number(document.get("delaySeconds"))?.max(0.0);
            Some(PreparedBuild {
                step,
                due_at_ms: add_seconds(started_at_ms, delay_seconds),
                anchor_step: document.get("anchorStep").and_then(Value::as_u64),
                delay: Duration::from_secs_f64(delay_seconds.min(u32::MAX as f64)),
            })
        })
        .collect())
}

fn parse_completion(
    value: Option<&Value>,
    started_at_ms: Option<i64>,
) -> Result<Option<PreparedCompletion>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let document = value
        .as_object()
        .ok_or_else(|| "Automation completion must be an object or null.".to_string())?;
    let Some(started_at_ms) = started_at_ms else {
        return Ok(None);
    };
    let delay_seconds = finite_number(document.get("delaySeconds"))
        .ok_or_else(|| "Automation completion delay must be finite.".to_string())?
        .max(0.0);
    let foreground_video_paths = match document.get("foregroundVideoPaths") {
        None => Vec::new(),
        Some(paths) => paths
            .as_array()
            .ok_or_else(|| "Automation completion video paths must be an array.".to_string())?
            .iter()
            .map(|path| {
                path.as_str()
                    .filter(|path| !path.is_empty())
                    .map(std::path::PathBuf::from)
                    .ok_or_else(|| {
                        "Automation completion video paths must be non-empty strings.".to_string()
                    })
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    Ok(Some(PreparedCompletion {
        due_at_ms: add_seconds(started_at_ms, delay_seconds),
        cue_id: optional_non_empty_string(document.get("cueId")),
        media_end: document.get("mediaEnd").and_then(Value::as_bool) == Some(true),
        delay_seconds,
        foreground_video_paths,
    }))
}

fn add_seconds(epoch_ms: i64, seconds: f64) -> i64 {
    epoch_ms.saturating_add((seconds * 1000.0).round() as i64)
}

fn duration_from_millis(milliseconds: i64) -> Duration {
    Duration::from_millis(u64::try_from(milliseconds.max(0)).unwrap_or(u64::MAX))
}

fn add_duration_seconds(base: Duration, seconds: f64) -> Duration {
    let milliseconds = (seconds.max(0.0) * 1_000.0).round();
    let duration = if milliseconds >= u64::MAX as f64 {
        Duration::MAX
    } else {
        Duration::from_millis(milliseconds as u64)
    };
    base.saturating_add(duration)
}

fn monotonic_deadline(due_at_ms: i64, wall_now_ms: i64, monotonic_now: Duration) -> Duration {
    monotonic_now.saturating_add(duration_from_millis(due_at_ms.saturating_sub(wall_now_ms)))
}

fn wall_timestamp_at_deadline(
    deadline: Duration,
    wall_now_ms: i64,
    monotonic_now: Duration,
) -> i64 {
    let delta = if deadline >= monotonic_now {
        i64::try_from((deadline - monotonic_now).as_millis()).unwrap_or(i64::MAX)
    } else {
        -i64::try_from((monotonic_now - deadline).as_millis()).unwrap_or(i64::MAX)
    };
    wall_now_ms.saturating_add(delta)
}

fn parse_build_step_clock(
    value: Option<&Value>,
    current_step: u64,
    timeline_started_at_ms: Option<i64>,
) -> Result<std::collections::BTreeMap<u64, i64>, String> {
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    let Some(value) = value else {
        return Ok(std::collections::BTreeMap::new());
    };
    let entries = value
        .as_object()
        .ok_or("Live build step clock must be an object.")?;
    let mut clock = std::collections::BTreeMap::new();
    for (step, started_at_ms) in entries {
        let step_number = step
            .parse::<u64>()
            .ok()
            .filter(|number| number.to_string() == *step && *number <= MAX_SAFE_INTEGER)
            .filter(|number| *number <= current_step)
            .ok_or("Live build step clock has an invalid or future step number.")?;
        let started_at_ms = safe_integer(Some(started_at_ms))
            .filter(|number| *number <= MAX_SAFE_INTEGER)
            .and_then(|number| i64::try_from(number).ok())
            .ok_or("Live build step clock timestamp must be a non-negative safe integer.")?;
        clock.insert(step_number, started_at_ms);
    }
    if let Some(timeline_started_at_ms) = timeline_started_at_ms {
        if clock.get(&0) != Some(&timeline_started_at_ms) {
            return Err("Live build step zero must match the timeline start timestamp.".into());
        }
    }
    if current_step > 0 && !clock.contains_key(&current_step) {
        return Err("The live build step must have an authoritative start timestamp.".into());
    }
    Ok(clock)
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
}

fn finite_number(value: Option<&Value>) -> Option<f64> {
    let value = value?.as_f64()?;
    value.is_finite().then_some(value)
}

fn numeric_value(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    let number = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()))?;
    number.is_finite().then_some(number)
}

fn safe_integer(value: Option<&Value>) -> Option<u64> {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    let number = numeric_value(value)?;
    (number >= 0.0 && number <= MAX_SAFE_INTEGER && number.fract() == 0.0).then(|| number as u64)
}

fn string_value(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn optional_non_empty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn non_empty_string(value: Option<&Value>, label: &str) -> Result<String, String> {
    optional_non_empty_string(value).ok_or_else(|| format!("{label} must be a non-empty string."))
}
