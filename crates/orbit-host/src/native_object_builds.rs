//! Native per-object ProPresenter Build In/Out timing.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Instant,
};

use serde_json::{Map, Value};

use crate::native_transitions;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BuildVisual {
    pub visible: bool,
    /// Remaining visibility: Build In uses progress; Build Out uses 1 - progress.
    pub opacity: f32,
    pub active: bool,
    pub pending: bool,
    pub unsupported_transition: bool,
    pub effects: Vec<BuildEffect>,
    pub text_reveal: Option<TextRevealVisual>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BuildEffect {
    pub transition: native_transitions::TransitionSpec,
    pub progress: f32,
    pub outgoing: bool,
}

pub(crate) struct BuildStepClock {
    now_ms: f64,
    starts: BTreeMap<u64, f64>,
}

impl BuildStepClock {
    pub(crate) fn from_value(
        value: &Value,
        now_ms: f64,
        current_step: f64,
    ) -> Result<Self, String> {
        let invalid = || {
            "Object build clocks require safe integer step keys and nonnegative epoch milliseconds."
                .to_owned()
        };
        if !now_ms.is_finite()
            || now_ms < 0.0
            || now_ms.fract() != 0.0
            || now_ms > 9_007_199_254_740_991.0
            || !current_step.is_finite()
            || current_step < 0.0
            || current_step.fract() != 0.0
            || current_step > 9_007_199_254_740_991.0
        {
            return Err(invalid());
        }
        let fields = value.as_object().ok_or_else(invalid)?;
        let mut starts = BTreeMap::new();
        for (key, value) in fields {
            let step = key
                .parse::<u64>()
                .ok()
                .filter(|step| {
                    step.to_string() == *key
                        && *step <= 9_007_199_254_740_991
                        && *step as f64 <= current_step
                })
                .ok_or_else(invalid)?;
            let time = value
                .as_f64()
                .filter(|time| {
                    time.is_finite()
                        && *time >= 0.0
                        && time.fract() == 0.0
                        && *time <= 9_007_199_254_740_991.0
                })
                .ok_or_else(invalid)?;
            starts.insert(step, time);
        }
        if !starts.contains_key(&0) || !starts.contains_key(&(current_step as u64)) {
            return Err(invalid());
        }
        Ok(Self { now_ms, starts })
    }
}

impl BuildEffect {
    fn cache_value(&self) -> Value {
        serde_json::json!({
            "kind": format!("{:?}", self.transition.kind),
            "durationSeconds": self.transition.duration_seconds,
            "side": self.transition.side,
            "variables": self.transition.variables.iter()
                .map(|(name, values)| serde_json::json!([name, values]))
                .collect::<Vec<_>>(),
            "progress": self.progress,
            "outgoing": self.outgoing,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextRevealMode {
    Paragraph,
    Underline,
}

impl TextRevealMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Paragraph => "paragraph",
            Self::Underline => "underline",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextRevealVisual {
    pub mode: TextRevealMode,
    /// Paragraphs include the first one from Build In; underline ranges do not.
    pub initial_visible: usize,
    /// Child index to current opacity. Only children with an ordered step appear here.
    pub child_opacities: BTreeMap<usize, f32>,
    /// Active children are rendered separately so their text plane can use the object effect.
    pub child_effects: BTreeMap<usize, Vec<BuildEffect>>,
    /// Render only this child at its full-layout position for a separate effect plane.
    pub isolated_child: Option<usize>,
    pub active: bool,
    pub pending: bool,
}

impl TextRevealVisual {
    pub(crate) fn cache_value(&self) -> Value {
        let children = self
            .child_opacities
            .iter()
            .map(|(index, opacity)| (index.to_string(), serde_json::json!(opacity)))
            .collect::<Map<_, _>>();
        let effects = self
            .child_effects
            .iter()
            .map(|(index, effects)| {
                (
                    index.to_string(),
                    serde_json::json!(effects
                        .iter()
                        .map(BuildEffect::cache_value)
                        .collect::<Vec<_>>()),
                )
            })
            .collect::<Map<_, _>>();
        serde_json::json!({
            "mode": self.mode.as_str(),
            "initialVisible": self.initial_visible,
            "children": children,
            "childEffects": effects,
            "isolatedChild": self.isolated_child,
        })
    }
}

impl BuildVisual {
    fn shown() -> Self {
        Self {
            visible: true,
            opacity: 1.0,
            ..Self::default()
        }
    }

    fn hidden() -> Self {
        Self::default()
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum Direction {
    In,
    Out,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct BuildKey {
    namespace: String,
    occurrence: String,
    layer_id: String,
    child_id: Option<String>,
    direction: Direction,
}

#[derive(Clone, Debug)]
struct OccurrenceState {
    slide_entry_key: String,
    last_step: f64,
}

struct ValidatedLayer<'a> {
    source: &'a Map<String, Value>,
    id: Option<String>,
    build_in: Option<ValidatedBuild<'a>>,
    build_out: Option<ValidatedBuild<'a>>,
    text_reveal: Option<ValidatedTextReveal<'a>>,
}

struct ValidatedBuild<'a> {
    source: &'a Value,
    step: Option<f64>,
}

struct ValidatedTextReveal<'a> {
    mode: TextRevealMode,
    from_index: usize,
    children: Vec<ValidatedRevealChild<'a>>,
}

struct ValidatedRevealChild<'a> {
    source: &'a Value,
    id: String,
    index: usize,
    step: Option<f64>,
}

#[derive(Clone, Debug)]
enum BuildState {
    Waiting,
    Running {
        started: Instant,
        delay_seconds: f64,
        transition: Option<native_transitions::TransitionSpec>,
        unsupported_transition: bool,
    },
    Complete {
        unsupported_transition: bool,
    },
}

#[derive(Default)]
pub(crate) struct ObjectBuildTransitions {
    occurrences: HashMap<String, OccurrenceState>,
    states: HashMap<BuildKey, BuildState>,
}

impl ObjectBuildTransitions {
    pub(crate) fn reset_namespace(&mut self, namespace: &str) {
        self.occurrences.remove(namespace);
        self.states.retain(|key, _| key.namespace != namespace);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn evaluate_slide_with_clock<'a>(
        &mut self,
        namespace: &str,
        slide_entry_key: &str,
        build_step: f64,
        ignore_builds: bool,
        layers: impl IntoIterator<Item = &'a Map<String, Value>>,
        now: Instant,
        clock: Option<&BuildStepClock>,
    ) -> Result<Vec<BuildVisual>, String> {
        if !build_step.is_finite()
            || build_step < 0.0
            || build_step.fract() != 0.0
            || build_step > 9_007_199_254_740_991.0
        {
            return Err(format!(
                "Native build step must be a finite nonnegative safe integer; received {build_step}."
            ));
        }
        let source_layers = layers.into_iter().collect::<Vec<_>>();
        let layers = validate_layers(&source_layers)?;
        if ignore_builds {
            return Ok(layers
                .iter()
                .map(|input| {
                    if input.source.get("hidden").and_then(Value::as_bool) == Some(true) {
                        BuildVisual::hidden()
                    } else {
                        BuildVisual::shown()
                    }
                })
                .collect());
        }

        let step = build_step;
        // A new slide occurrence or step rewind has no transition history.
        let previous_step = self
            .occurrences
            .get(namespace)
            .filter(|state| state.slide_entry_key == slide_entry_key && step >= state.last_step)
            .map(|state| state.last_step);
        let new_occurrence = previous_step.is_none();
        if new_occurrence {
            self.reset_namespace(namespace);
        }

        let result = layers
            .iter()
            .map(|input| -> Result<BuildVisual, String> {
                if input.source.get("hidden").and_then(Value::as_bool) == Some(true) {
                    return Ok(BuildVisual::hidden());
                }
                let mut visual = match (&input.id, &input.build_in) {
                    (Some(id), Some(build)) => match build.step {
                        Some(build_step) => self.evaluate_build(
                            namespace,
                            slide_entry_key,
                            id,
                            None,
                            Direction::In,
                            build.source,
                            build.source,
                            build_step,
                            previous_step,
                            new_occurrence,
                            step,
                            now,
                            clock,
                        )?,
                        // Missing from ElementBuildOrder is not step zero.
                        None => BuildVisual::shown(),
                    },
                    _ => BuildVisual::shown(),
                };
                if let (Some(id), Some(build_out)) = (&input.id, &input.build_out) {
                    // An un-ordered Build Out has no activation step either.
                    if let Some(build_step) = build_out.step {
                        let out = self.evaluate_build(
                            namespace,
                            slide_entry_key,
                            id,
                            None,
                            Direction::Out,
                            build_out.source,
                            build_out.source,
                            build_step,
                            previous_step,
                            new_occurrence,
                            step,
                            now,
                            clock,
                        )?;
                        visual.visible &= out.visible;
                        visual.opacity *= out.opacity;
                        visual.pending |= out.pending;
                        visual.effects.extend(out.effects);
                        visual.active |= out.active;
                        visual.unsupported_transition |= out.unsupported_transition;
                    }
                }
                if let (Some(id), Some(reveal)) = (&input.id, &input.text_reveal) {
                    if reveal.children.iter().any(|child| child.step.is_some()) {
                        let no_transition = Value::Null;
                        let transition_source = input
                            .build_in
                            .as_ref()
                            .map_or(&no_transition, |build| build.source);
                        let (inherited, _) = resolve_transition(transition_source)?;
                        let inherited_effect = inherited.as_ref().is_some_and(|transition| {
                            transition.kind != native_transitions::TransitionKind::Cut
                                && transition.duration_seconds > 0.0
                        });
                        let delayed_child = reveal.children.iter().any(|child| {
                            child.step.is_some()
                                && child.source.as_object().is_some_and(|build| {
                                    build
                                        .get("delaySeconds")
                                        .and_then(Value::as_f64)
                                        .is_some_and(|delay| {
                                            delay > 0.0
                                                && build.get("start").and_then(Value::as_str)
                                                    != Some("after-previous")
                                        })
                                })
                        });
                        if inherited_effect || delayed_child {
                            let initial_visible = match reveal.mode {
                                TextRevealMode::Paragraph => reveal.from_index.max(1),
                                TextRevealMode::Underline => reveal.from_index.saturating_sub(1),
                            };
                            let mut text_reveal = TextRevealVisual {
                                mode: reveal.mode,
                                initial_visible,
                                child_opacities: BTreeMap::new(),
                                child_effects: BTreeMap::new(),
                                isolated_child: None,
                                active: false,
                                pending: false,
                            };
                            for child in &reveal.children {
                                let Some(child_step) = child.step else {
                                    continue;
                                };
                                let child_visual = self.evaluate_build(
                                    namespace,
                                    slide_entry_key,
                                    id,
                                    Some(&child.id),
                                    Direction::In,
                                    child.source,
                                    transition_source,
                                    child_step,
                                    previous_step,
                                    new_occurrence,
                                    step,
                                    now,
                                    clock,
                                )?;
                                if child_visual.effects.is_empty() {
                                    text_reveal
                                        .child_opacities
                                        .insert(child.index, child_visual.opacity);
                                } else {
                                    text_reveal.child_opacities.insert(child.index, 0.0);
                                    text_reveal
                                        .child_effects
                                        .insert(child.index, child_visual.effects);
                                }
                                text_reveal.active |= child_visual.active;
                                text_reveal.pending |= child_visual.pending;
                            }
                            if !text_reveal.child_opacities.is_empty() {
                                visual.active |= text_reveal.active;
                                visual.pending |= text_reveal.pending;
                                visual.text_reveal = Some(text_reveal);
                            }
                        }
                    }
                }
                Ok(visual)
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.occurrences.insert(
            namespace.to_owned(),
            OccurrenceState {
                slide_entry_key: slide_entry_key.to_owned(),
                last_step: step,
            },
        );
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_build(
        &mut self,
        namespace: &str,
        occurrence: &str,
        layer_id: &str,
        child_id: Option<&str>,
        direction: Direction,
        build: &Value,
        transition_source: &Value,
        target_step: f64,
        previous_step: Option<f64>,
        new_occurrence: bool,
        step: f64,
        now: Instant,
        clock: Option<&BuildStepClock>,
    ) -> Result<BuildVisual, String> {
        if let Some(clock) = clock {
            if target_step > step {
                return Ok(waiting_phase(direction));
            }
            let Some(started_ms) = clock.starts.get(&(target_step as u64)) else {
                return Ok(phase(direction, None, false, false, None));
            };
            let delay = if build["start"] == "after-previous" {
                0.0
            } else {
                build
                    .get("delaySeconds")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
            };
            let elapsed = (clock.now_ms - started_ms) / 1000.0;
            if elapsed < delay {
                let mut waiting = phase(direction, Some(0.0), false, false, None);
                waiting.pending = true;
                return Ok(waiting);
            }
            let (transition, unsupported) = resolve_transition(transition_source)?;
            let Some(transition) = transition else {
                return Ok(phase(direction, None, false, unsupported, None));
            };
            let progress = ((elapsed - delay) / transition.duration_seconds).clamp(0.0, 1.0);
            return Ok(if progress >= 1.0 {
                phase(direction, None, false, unsupported, None)
            } else {
                phase(
                    direction,
                    Some(progress as f32),
                    true,
                    unsupported,
                    Some(transition),
                )
            });
        }
        let key = BuildKey {
            namespace: namespace.to_owned(),
            occurrence: occurrence.to_owned(),
            layer_id: layer_id.to_owned(),
            child_id: child_id.map(str::to_owned),
            direction,
        };
        let starts_with_slide = build.get("start").and_then(Value::as_str) == Some("with-slide")
            && target_step == 0.0
            && new_occurrence
            && step == 0.0;
        let crossed =
            previous_step.is_some_and(|previous| previous < target_step && target_step <= step);
        let reached = step >= target_step;

        let state = self.states.entry(key).or_insert(BuildState::Waiting);
        if matches!(state, BuildState::Waiting) {
            if starts_with_slide || crossed {
                let start = build
                    .get("start")
                    .and_then(Value::as_str)
                    .unwrap_or("on-click");
                let delay_seconds = if start == "after-previous" {
                    0.0
                } else {
                    build
                        .get("delaySeconds")
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite() && *value > 0.0)
                        .unwrap_or(0.0)
                };
                let (transition, unsupported_transition) = resolve_transition(transition_source)?;
                if transition.is_none() && delay_seconds == 0.0 {
                    *state = BuildState::Complete {
                        unsupported_transition,
                    };
                } else {
                    *state = BuildState::Running {
                        started: now,
                        delay_seconds,
                        transition,
                        unsupported_transition,
                    };
                }
            } else if reached {
                // The compositor first saw the slide after this build step;
                // do not replay an old build from an unknown entry time.
                *state = BuildState::Complete {
                    unsupported_transition: false,
                };
            }
        }

        Ok(match state.clone() {
            BuildState::Waiting => waiting_phase(direction),
            BuildState::Complete {
                unsupported_transition,
            } => phase(direction, None, false, unsupported_transition, None),
            BuildState::Running {
                started,
                delay_seconds,
                transition,
                unsupported_transition,
            } => {
                let elapsed = now.saturating_duration_since(started).as_secs_f64();
                if elapsed < delay_seconds {
                    let mut visual =
                        phase(direction, Some(0.0), false, unsupported_transition, None);
                    visual.pending = true;
                    visual
                } else {
                    let Some(duration) = transition.as_ref().map(|value| value.duration_seconds)
                    else {
                        *state = BuildState::Complete {
                            unsupported_transition,
                        };
                        return Ok(phase(direction, None, false, unsupported_transition, None));
                    };
                    let progress = ((elapsed - delay_seconds) / duration).clamp(0.0, 1.0);
                    if progress >= 1.0 {
                        *state = BuildState::Complete {
                            unsupported_transition,
                        };
                        phase(direction, None, false, unsupported_transition, None)
                    } else {
                        phase(
                            direction,
                            Some(progress as f32),
                            true,
                            unsupported_transition,
                            transition,
                        )
                    }
                }
            }
        })
    }
}

fn validate_layers<'a>(
    layers: &'a [&'a Map<String, Value>],
) -> Result<Vec<ValidatedLayer<'a>>, String> {
    let mut build_layer_ids = HashSet::new();
    layers
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let source = *source;
            let build_in = validate_build(source, "build", index)?;
            let build_out = validate_build(source, "buildOut", index)?;
            let text_reveal = validate_text_reveal(source, index)?;
            let has_reveal_children = text_reveal
                .as_ref()
                .is_some_and(|reveal| !reveal.children.is_empty());
            let id = if build_in.is_some() || build_out.is_some() || has_reveal_children {
                let id = source
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.trim().is_empty())
                    .ok_or_else(|| {
                        format!("Layer {index} has Build metadata but no authored nonempty ID.")
                    })?
                    .to_owned();
                if !build_layer_ids.insert(id.clone()) {
                    return Err(format!("Build layer ID '{id}' is duplicated."));
                }
                Some(id)
            } else {
                source
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
            };
            Ok(ValidatedLayer {
                source,
                id,
                build_in,
                build_out,
                text_reveal,
            })
        })
        .collect()
}

fn validate_text_reveal<'a>(
    layer: &'a Map<String, Value>,
    layer_index: usize,
) -> Result<Option<ValidatedTextReveal<'a>>, String> {
    let Some(source) = layer.get("textReveal").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let reveal = source
        .as_object()
        .ok_or_else(|| format!("Layer {layer_index} textReveal metadata must be an object."))?;
    let mode = match reveal.get("mode").and_then(Value::as_str) {
        Some("paragraph") => TextRevealMode::Paragraph,
        Some("underline") => TextRevealMode::Underline,
        _ => return Err(format!("Layer {layer_index} textReveal/mode is invalid.")),
    };
    let from_index = reveal
        .get("fromIndex")
        .map(|value| {
            let index = value
                .as_u64()
                .filter(|index| *index <= 9_007_199_254_740_991);
            index
                .and_then(|index| usize::try_from(index).ok())
                .ok_or_else(|| format!("Layer {layer_index} textReveal/fromIndex is invalid."))
        })
        .transpose()?
        .unwrap_or(0);
    let Some(children) = reveal.get("children") else {
        return Ok(Some(ValidatedTextReveal {
            mode,
            from_index,
            children: Vec::new(),
        }));
    };
    let children = children
        .as_array()
        .ok_or_else(|| format!("Layer {layer_index} textReveal/children must be an array."))?;
    let mut child_ids = HashSet::new();
    let mut child_indices = HashSet::new();
    let mut validated = Vec::with_capacity(children.len());
    for (child_index, source) in children.iter().enumerate() {
        let path = format!("Layer {layer_index} textReveal/children/{child_index}");
        let child = source
            .as_object()
            .ok_or_else(|| format!("{path} metadata must be an object."))?;
        if child.contains_key("transition") {
            return Err(format!(
                "{path}/transition is unsupported; child builds inherit layer.build.transition."
            ));
        }
        let id = child
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| format!("{path}/id must be a nonempty authored ID."))?
            .to_owned();
        if !child_ids.insert(id.clone()) {
            return Err(format!("{path}/id '{id}' is duplicated."));
        }
        let index = child
            .get("index")
            .and_then(Value::as_u64)
            .filter(|index| *index <= 9_007_199_254_740_991)
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(|| format!("{path}/index must be a nonnegative safe integer."))?;
        if !child_indices.insert(index) {
            return Err(format!("{path}/index {index} is duplicated."));
        }
        let step = child
            .get("step")
            .map(|value| {
                value
                    .as_f64()
                    .filter(|step| {
                        step.is_finite()
                            && *step >= 0.0
                            && step.fract() == 0.0
                            && *step <= 9_007_199_254_740_991.0
                    })
                    .ok_or_else(|| format!("{path}/step must be a nonnegative safe integer."))
            })
            .transpose()?;
        if let Some(start) = child.get("start") {
            if !matches!(
                start.as_str(),
                Some("on-click" | "with-previous" | "after-previous" | "with-slide")
            ) {
                return Err(format!("{path}/start is invalid."));
            }
        }
        let start = child
            .get("start")
            .and_then(Value::as_str)
            .unwrap_or("on-click");
        if step == Some(0.0) && start != "with-slide" {
            return Err(format!("{path}/step zero requires a with-slide start."));
        }
        if step.is_some_and(|step| step > 0.0) && start == "with-slide" {
            return Err(format!("{path}/with-slide start requires step zero."));
        }
        if let Some(delay) = child.get("delaySeconds") {
            if !delay
                .as_f64()
                .is_some_and(|delay| delay.is_finite() && delay >= 0.0)
            {
                return Err(format!("{path}/delaySeconds is invalid."));
            }
        }
        validated.push(ValidatedRevealChild {
            source,
            id,
            index,
            step,
        });
    }
    Ok(Some(ValidatedTextReveal {
        mode,
        from_index,
        children: validated,
    }))
}

fn validate_build<'a>(
    layer: &'a Map<String, Value>,
    key: &str,
    layer_index: usize,
) -> Result<Option<ValidatedBuild<'a>>, String> {
    let Some(source) = layer.get(key).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let build = source
        .as_object()
        .ok_or_else(|| format!("Layer {layer_index} {key} metadata must be an object."))?;
    let step = build
        .get("step")
        .map(|value| {
            let step = value.as_f64().filter(|step| {
                step.is_finite()
                    && *step >= 0.0
                    && step.fract() == 0.0
                    && *step <= 9_007_199_254_740_991.0
            });
            step.ok_or_else(|| {
                format!("Layer {layer_index} {key}/step must be a nonnegative safe integer.")
            })
        })
        .transpose()?;
    if let Some(start) = build.get("start") {
        let valid = matches!(
            start.as_str(),
            Some("on-click" | "with-previous" | "after-previous" | "with-slide")
        );
        if !valid {
            return Err(format!("Layer {layer_index} {key}/start is invalid."));
        }
    }
    let start = build
        .get("start")
        .and_then(Value::as_str)
        .unwrap_or("on-click");
    if step == Some(0.0) && start != "with-slide" {
        return Err(format!(
            "Layer {layer_index} {key}/step zero requires a with-slide start."
        ));
    }
    if step.is_some_and(|step| step > 0.0) && start == "with-slide" {
        return Err(format!(
            "Layer {layer_index} {key}/with-slide start requires step zero."
        ));
    }
    if let Some(delay) = build.get("delaySeconds") {
        if !delay
            .as_f64()
            .is_some_and(|delay| delay.is_finite() && delay >= 0.0)
        {
            return Err(format!(
                "Layer {layer_index} {key}/delaySeconds is invalid."
            ));
        }
    }
    Ok(Some(ValidatedBuild { source, step }))
}

fn phase(
    direction: Direction,
    progress: Option<f32>,
    active: bool,
    unsupported_transition: bool,
    transition: Option<native_transitions::TransitionSpec>,
) -> BuildVisual {
    let opacity = match (direction, progress) {
        (Direction::In, None) => 1.0,
        (Direction::In, Some(progress)) => progress,
        (Direction::Out, None) => 0.0,
        (Direction::Out, Some(progress)) => 1.0 - progress,
    };
    let visible = match (direction, progress) {
        (Direction::In, None) => true,
        (Direction::In, Some(progress)) => progress > 0.0 || active,
        (Direction::Out, None) => false,
        (Direction::Out, Some(progress)) => progress < 1.0 || active,
    };
    BuildVisual {
        visible,
        opacity,
        active,
        pending: false,
        unsupported_transition,
        effects: if active {
            transition
                .map(|transition| BuildEffect {
                    transition,
                    progress: progress.unwrap_or(1.0),
                    outgoing: direction == Direction::Out,
                })
                .into_iter()
                .collect()
        } else {
            Vec::new()
        },
        text_reveal: None,
    }
}

fn waiting_phase(direction: Direction) -> BuildVisual {
    match direction {
        Direction::In => BuildVisual::hidden(),
        Direction::Out => BuildVisual::shown(),
    }
}

/// Object builds use the same validated catalogue as slide transitions.
fn resolve_transition(
    build: &Value,
) -> Result<(Option<native_transitions::TransitionSpec>, bool), String> {
    let Some(transition) = build.get("transition") else {
        return Ok((None, false));
    };
    match native_transitions::resolve(transition) {
        Ok(None) => Ok((None, false)),
        Ok(Some(spec)) => Ok((Some(spec), false)),
        Err(error) => Err(format!("Invalid object Build transition: {error}")),
    }
}
