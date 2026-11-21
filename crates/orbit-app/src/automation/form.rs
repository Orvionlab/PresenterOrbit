//! A small JSON-draft form. The authoring editors (macros, messages, timers, slide actions) edit
//! the same JSON the workspace stores, so a field is a dotted path into the draft plus a control kind. Text fields
//! keep their editing state here and write the typed value back; structural edits call [`Form::refresh`] so the
//! controls are rebuilt from the draft.

use std::collections::HashMap;

use gpui::{prelude::*, AnyElement, Context, SharedString, Subscription, Window};
use orbit_ui::controls::{
    input_state, textarea_state, CheckboxField, Field, InputEvent, InputState, Select,
    SelectOption, TextInput, Textarea, TextareaState,
};
use serde_json::{json, Map, Value};

use gpui::Entity;

pub enum Kind {
    Text,
    Multiline,
    Number,
    /// Seconds shown and typed as `HH:MM:SS`; an empty field clears the value, a malformed one changes nothing.
    Time,
    ClockTime(ClockPeriod),
    /// A stored timestamp edited as local `YYYY-MM-DDTHH:MM[:SS]`; typed text is stored with the local offset.
    /// Strings separated by commas.
    Check,
    /// A check box that is on unless the draft says `false`.
    On,
    /// `(value, label)` pairs; the label is already translated.
    Choice(Vec<(String, String)>),
    /// A choice whose change is applied by the function (path, chosen value) instead of being stored, for edits that
    /// replace more than one field. The controls are rebuilt afterwards.
    Hook(Vec<(String, String)>, fn(&mut Form, &str, &str), String),
    /// Membership of this id in the array at the path.
    Member(String),
}

pub struct Spec {
    pub path: String,
    pub label: String,
    pub kind: Kind,
    /// Written first when the value at the path is absent: `(path, value)` of the object that owns the field.
    pub seed: Option<(String, Value)>,
    /// An emptied number is stored as `null` instead of being removed.
    pub keep_null: bool,
    pub placeholder: Option<String>,
    pub stacked: bool,
}

impl Spec {
    pub fn new(path: impl Into<String>, label: impl Into<String>, kind: Kind) -> Self {
        Self {
            path: path.into(),
            label: label.into(),
            kind,
            seed: None,
            keep_null: false,
            placeholder: None,
            stacked: false,
        }
    }

    pub fn seed(mut self, path: impl Into<String>, value: Value) -> Self {
        self.seed = Some((path.into(), value));
        self
    }

    pub fn keep_null(mut self) -> Self {
        self.keep_null = true;
        self
    }
}

fn step<'a>(value: &'a Value, part: &str) -> &'a Value {
    match part.parse::<usize>() {
        Ok(index) if value.is_array() => &value[index],
        _ => &value[part],
    }
}

/// The value at a dotted path; a missing step is `Null`.
pub fn get<'a>(root: &'a Value, path: &str) -> &'a Value {
    path.split('.').fold(root, step)
}

/// Writes `value` at a dotted path, creating objects on the way. `Null` removes the key. A step through an array
/// that does not exist leaves the draft unchanged.
pub fn set(root: &mut Value, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut target = root;
    for (position, part) in parts.iter().enumerate() {
        let last = position + 1 == parts.len();
        if let (Ok(index), true) = (part.parse::<usize>(), target.is_array()) {
            let Some(slot) = target.get_mut(index) else {
                return;
            };
            if last {
                *slot = value;
                return;
            }
            target = slot;
            continue;
        }
        if !target.is_object() {
            *target = Value::Object(Map::new());
        }
        let Some(object) = target.as_object_mut() else {
            return;
        };
        if last {
            if value.is_null() {
                object.remove(*part);
            } else {
                object.insert(part.to_string(), value);
            }
            return;
        }
        target = object.entry(part.to_string()).or_insert_with(|| json!({}));
    }
}

/// Writes an explicit `null` at a dotted path of existing objects.
fn set_null(root: &mut Value, path: &str) {
    set(root, path, json!(0));
    let mut target = root;
    for part in path.split('.') {
        let next = if target.is_array() {
            part.parse::<usize>()
                .ok()
                .and_then(|index| target.get_mut(index))
        } else {
            target.get_mut(part)
        };
        match next {
            Some(next) => target = next,
            None => return,
        }
    }
    *target = Value::Null;
}

/// What a field edit needs of its [`Spec`], cloneable into the change handler.
#[derive(Clone)]
struct SpecRef {
    path: String,
    seed: Option<(String, Value)>,
    keep_null: bool,
}

/// `formatTimeInput`: seconds as `HH:MM:SS`.
pub fn format_time(seconds: f64) -> String {
    let total = seconds.floor() as i64;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        total / 60 % 60,
        total % 60
    )
}

/// `parseTimeInput`: `HH:MM[:SS]` as seconds, `None` when malformed.
pub fn parse_time(text: &str) -> Option<i64> {
    let parts: Vec<i64> = text
        .trim()
        .split(':')
        .map(|part| part.parse::<u32>().ok().map(i64::from))
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [hours, minutes] => Some(hours * 3600 + minutes * 60),
        [hours, minutes, seconds] => Some(hours * 3600 + minutes * 60 + seconds),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClockPeriod {
    Am,
    Pm,
    FullDay,
}

impl ClockPeriod {
    pub fn from_timer(timer: &Value) -> Self {
        match timer["format"]["targetTimePeriod"].as_str() {
            Some("am") => Self::Am,
            Some("pm") => Self::Pm,
            _ => Self::FullDay,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Am => "am",
            Self::Pm => "pm",
            Self::FullDay => "24h",
        }
    }

    pub fn retarget(self, seconds: f64) -> f64 {
        match self {
            Self::Am => seconds % 43_200.,
            Self::Pm => seconds % 43_200. + 43_200.,
            Self::FullDay => seconds,
        }
    }

    fn display_seconds(self, seconds: f64) -> f64 {
        if self == Self::FullDay {
            return seconds;
        }
        let half_day = seconds % 43_200.;
        if half_day < 3600. {
            half_day + 43_200.
        } else {
            half_day
        }
    }

    fn parse(self, value: &str) -> Option<i64> {
        let seconds = parse_time(value)?;
        match self {
            Self::FullDay => (seconds < 86_400).then_some(seconds),
            Self::Am | Self::Pm => (3600..46_800)
                .contains(&seconds)
                .then(|| seconds % 43_200 + if self == Self::Pm { 43_200 } else { 0 }),
        }
    }
}

/// What a number field holds: nothing, a finite integer or fraction, or the raw text when it is not a number so
/// validation can refuse it.
pub fn number_value(text: &str) -> Value {
    let text = text.trim();
    if text.is_empty() {
        return Value::Null;
    }
    match text.parse::<f64>() {
        Ok(number) if number.is_finite() && number.fract() == 0. && number.abs() < 1e15 => {
            json!(number as i64)
        }
        Ok(number) if number.is_finite() => json!(number),
        _ => json!(text),
    }
}

fn shown(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[derive(Default)]
pub struct Form {
    pub draft: Value,
    focus: Option<String>,
    inputs: HashMap<String, (Entity<InputState>, Subscription)>,
    areas: HashMap<String, (Entity<TextareaState>, Subscription)>,
}

impl Form {
    pub fn time_input_valid(&self, path: &str, cx: &gpui::App) -> bool {
        self.inputs.get(path).is_none_or(|(input, _)| {
            let value = input.read(cx).value();
            value.trim().is_empty()
                || if path == "targetTimeSeconds" {
                    ClockPeriod::from_timer(&self.draft)
                        .parse(value.as_ref())
                        .is_some()
                } else {
                    parse_time(value.as_ref()).is_some()
                }
        })
    }

    pub fn reset(&mut self, draft: Value) {
        self.draft = draft;
        self.focus = None;
        self.refresh();
    }

    pub fn request_focus(&mut self, path: &str) {
        self.focus = Some(path.to_string());
    }

    /// Rebuilds the text controls from the draft after a structural edit.
    pub fn refresh(&mut self) {
        self.inputs.clear();
        self.areas.clear();
    }

    pub fn set(&mut self, path: &str, value: Value) {
        set(&mut self.draft, path, value);
    }

    /// A field edit: seeds the owning object, then writes (or, for `keep_null`, nulls) the value.
    fn write(&mut self, spec: &SpecRef, value: Value) {
        if let Some((seed_path, seed)) = &spec.seed {
            if get(&self.draft, seed_path).is_null() {
                set(&mut self.draft, seed_path, seed.clone());
            }
        }
        if value.is_null() && spec.keep_null {
            set_null(&mut self.draft, &spec.path);
        } else {
            set(&mut self.draft, &spec.path, value);
        }
    }

    /// Renders one field. `owner` finds this form inside the view `T` that holds it.
    pub fn field<T: 'static>(
        &mut self,
        id: &str,
        spec: &Spec,
        owner: fn(&mut T) -> &mut Form,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> AnyElement {
        let value = get(&self.draft, &spec.path).clone();
        let key = SharedString::from(format!("{id}-{}", spec.path));
        let weak = cx.weak_entity();
        let path = spec.path.clone();
        let target = SpecRef {
            path: spec.path.clone(),
            seed: spec.seed.clone(),
            keep_null: spec.keep_null,
        };
        match &spec.kind {
            Kind::Text | Kind::Number | Kind::Time | Kind::ClockTime(_) => {
                let numeric = matches!(spec.kind, Kind::Number);
                let clock = match spec.kind {
                    Kind::ClockTime(period) => Some(period),
                    _ => None,
                };
                let timed = matches!(spec.kind, Kind::Time) || clock.is_some();
                let input = match self.inputs.get(&spec.path) {
                    Some((input, _)) => input.clone(),
                    None => {
                        let initial = match (timed, value.as_f64()) {
                            (true, Some(seconds)) => format_time(
                                clock.map_or(seconds, |period| period.display_seconds(seconds)),
                            ),
                            _ => shown(&value),
                        };
                        let input = input_state(initial, window, cx);
                        if let Some(placeholder) = &spec.placeholder {
                            input.update(cx, |input, cx| {
                                input.set_placeholder(placeholder.clone(), window, cx)
                            });
                        }
                        let target = target.clone();
                        let subscription = cx.subscribe_in(
                            &input,
                            window,
                            move |this, input, event: &InputEvent, _, cx| {
                                if !matches!(event, InputEvent::Change) {
                                    return;
                                }
                                let text = input.read(cx).value().to_string();
                                let value = if timed {
                                    if text.trim().is_empty() {
                                        Value::Null
                                    } else if let Some(seconds) = clock.map_or_else(
                                        || parse_time(&text),
                                        |period| period.parse(&text),
                                    ) {
                                        json!(seconds)
                                    } else {
                                        cx.notify();
                                        return;
                                    }
                                } else if numeric {
                                    number_value(&text)
                                } else {
                                    json!(text)
                                };
                                owner(this).write(&target, value);
                                cx.notify();
                            },
                        );
                        self.inputs
                            .insert(spec.path.clone(), (input.clone(), subscription));
                        input
                    }
                };
                if self.focus.as_deref() == Some(spec.path.as_str()) {
                    self.focus = None;
                    input.update(cx, |input, cx| {
                        input.focus(window, cx);
                        input.select_all(window, cx);
                    });
                }
                let selector = key.clone();
                Field::new(
                    spec.label.clone(),
                    gpui::div()
                        .debug_selector(move || format!("form-input-{selector}"))
                        .child(TextInput::new(key, &input).compact()),
                )
                .when(!spec.stacked, Field::inline)
                .into_any_element()
            }
            Kind::Multiline => {
                let area = match self.areas.get(&spec.path) {
                    Some((area, _)) => area.clone(),
                    None => {
                        let area = textarea_state(shown(&value), window, cx);
                        let path = spec.path.clone();
                        let subscription = cx.subscribe_in(
                            &area,
                            window,
                            move |this, area, event: &InputEvent, _, cx| {
                                if matches!(event, InputEvent::Change) {
                                    let text = area.read(cx).value().to_string();
                                    owner(this).set(&path, json!(text));
                                    cx.notify();
                                }
                            },
                        );
                        self.areas
                            .insert(spec.path.clone(), (area.clone(), subscription));
                        area
                    }
                };
                Field::new(spec.label.clone(), Textarea::new(key, &area).rows(3.))
                    .into_any_element()
            }
            Kind::Check | Kind::On => {
                let on_by_default = matches!(spec.kind, Kind::On);
                let checked = if on_by_default {
                    value != json!(false)
                } else {
                    value == json!(true)
                };
                CheckboxField::new(key, spec.label.clone(), checked)
                    .on_change(move |on, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            owner(this).write(&target, json!(on));
                            cx.notify();
                        });
                    })
                    .into_any_element()
            }
            Kind::Member(id) => {
                let member = value
                    .as_array()
                    .is_some_and(|ids| ids.iter().any(|x| x == id.as_str()));
                let id = id.clone();
                CheckboxField::new(key, spec.label.clone(), member)
                    .on_change(move |on, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            let form = owner(this);
                            let mut ids = get(&form.draft, &path)
                                .as_array()
                                .cloned()
                                .unwrap_or_default();
                            ids.retain(|x| x != id.as_str());
                            if on {
                                ids.push(json!(id));
                            }
                            form.set(&path, Value::Array(ids));
                            cx.notify();
                        });
                    })
                    .into_any_element()
            }
            Kind::Choice(options) => {
                let current = shown(&value);
                Field::new(
                    spec.label.clone(),
                    Select::new(
                        key,
                        options
                            .iter()
                            .map(|(value, label)| SelectOption::new(value.clone(), label.clone()))
                            .collect(),
                        current,
                    )
                    .compact()
                    .on_change(move |chosen: String, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            owner(this).write(&target, json!(chosen));
                            cx.notify();
                        });
                    }),
                )
                .inline()
                .into_any_element()
            }
            Kind::Hook(options, apply, current) => {
                let apply = *apply;
                let current = current.clone();
                Field::new(
                    spec.label.clone(),
                    Select::new(
                        key,
                        options
                            .iter()
                            .map(|(value, label)| SelectOption::new(value.clone(), label.clone()))
                            .collect(),
                        current,
                    )
                    .compact()
                    .on_change(move |chosen: String, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            let form = owner(this);
                            apply(form, &path, &chosen);
                            form.refresh();
                            cx.notify();
                        });
                    }),
                )
                .inline()
                .into_any_element()
            }
        }
    }
}
