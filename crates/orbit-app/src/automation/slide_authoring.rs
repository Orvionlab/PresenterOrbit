//! Slide action lists and completion (`SlideActionsBridge.tsx`, `SlideCompletionBridge.tsx`,
//! `slide-actions-controller.ts`, `slide-completion-controller.ts`): the actions that run when a slide goes live,
//! and what happens when it finishes. Both are saved as a `setSlideDocument` of the presentation, which keeps
//! every other field of the slides, and the dialog opens on the slide selected in the cue editor.

use gpui::{div, prelude::*, Context, SharedString, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant, Dialog};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Map, Value};

use super::action_list;
use super::form::{get, Form, Kind, Spec};
use super::runner::Facts;
use crate::state::runtime::{Kind as Collection, Persist, Runtime};

const COMPLETION_FIELDS: [&str; 6] = [
    "completionAction",
    "completionTimeSeconds",
    "completionTargetCueId",
    "completionTargetType",
    "completionTargetInstanceId",
    "completionActionId",
];
const MODES: [&str; 3] = ["OFF", "AFTER_TIME", "ON_MEDIA_END"];

/// `readSlideCompletion`: the completion fields a slide carries.
pub fn read_completion(slide: &Value) -> Value {
    let mut fields = Map::new();
    for key in COMPLETION_FIELDS {
        if !slide[key].is_null() {
            fields.insert(key.to_string(), slide[key].clone());
        }
    }
    Value::Object(fields)
}

/// `completionMode`: a slide ProPresenter wrote without a Slide Timer reads as off.
pub fn completion_mode(slide: &Value) -> String {
    let action = slide["completionAction"].as_str().unwrap_or("");
    if action.ends_with("AFTER_TIME") {
        return "AFTER_TIME".into();
    }
    let bare = ["_FIRST", "_LAST", "_AFTER_ACTION"]
        .iter()
        .any(|suffix| action.ends_with(suffix));
    let untimed = bare
        && slide["completionTargetType"].as_str().unwrap_or("NONE") == "NONE"
        && slide["completionTimeSeconds"].is_null()
        && slide["completionActionId"]
            .as_str()
            .is_none_or(str::is_empty);
    if untimed || action.is_empty() {
        "OFF".into()
    } else {
        action.into()
    }
}

fn label(slide: &Value) -> String {
    ["label", "name", "content"]
        .iter()
        .find_map(|key| slide[*key].as_str().filter(|text| !text.is_empty()))
        .or_else(|| slide["id"].as_str())
        .unwrap_or("")
        .to_string()
}

/// `completionTargets`: the enabled slides, by id.
pub fn targets(item: &Value) -> Vec<(String, String)> {
    item["slides"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|slide| slide["disabled"] != true)
        .map(|slide| (slide["id"].as_str().unwrap_or("").to_string(), label(slide)))
        .collect()
}

/// `completionOccurrences`: the enabled occurrences of the item's arrangement, numbered.
pub fn occurrences(item: &Value) -> Vec<(String, String)> {
    let arrangement = crate::cues::model::selected_arrangement_id(item);
    let cues = orbit_host::Host::presentation_cues(item, arrangement, None).unwrap_or_default();
    cues.iter()
        .filter(|slide| slide["disabled"] != true && !slide["arrangementInstanceId"].is_null())
        .enumerate()
        .map(|(index, slide)| {
            (
                slide["arrangementInstanceId"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                format!("{}: {}", index + 1, label(slide)),
            )
        })
        .collect()
}

/// The target a slide shows in the choice: an occurrence, a type, or a slide.
pub fn target_value(slide: &Value) -> String {
    if let Some(instance) = slide["completionTargetInstanceId"]
        .as_str()
        .filter(|id| !id.is_empty())
    {
        return format!("instance:{instance}");
    }
    match slide["completionTargetType"].as_str() {
        Some(kind @ ("FIRST" | "RANDOM")) => format!("type:{kind}"),
        _ => slide["completionTargetCueId"]
            .as_str()
            .unwrap_or("")
            .to_string(),
    }
}

/// The completion a mode, delay text, and target choice ask for, as the editor builds it.
pub fn request(mode: &str, seconds: &str, target: &str) -> Value {
    let mut fields = Map::new();
    fields.insert("completionAction".into(), json!(mode));
    if mode == "AFTER_TIME" {
        // A delay that is not a number stays text, so validation refuses it.
        fields.insert(
            "completionTimeSeconds".into(),
            super::form::number_value(seconds),
        );
    }
    if !target.is_empty() && !target.starts_with("type:") && !target.starts_with("instance:") {
        fields.insert("completionTargetCueId".into(), json!(target));
    }
    fields.insert(
        "completionTargetType".into(),
        json!(if let Some(kind) = target.strip_prefix("type:") {
            kind
        } else if target.is_empty() {
            "NEXT"
        } else {
            "CUE"
        }),
    );
    if let Some(instance) = target.strip_prefix("instance:") {
        fields.insert("completionTargetInstanceId".into(), json!(instance));
    }
    Value::Object(fields)
}

/// `validateCompletionRequest` and `validateCompletionTarget`.
pub fn completion_error(item: &Value, requested: &Value) -> Option<&'static str> {
    let mode = requested["completionAction"].as_str().unwrap_or("");
    if !MODES.contains(&mode) {
        return Some("completion.modeInvalid");
    }
    let seconds = requested["completionTimeSeconds"].as_f64();
    if mode == "AFTER_TIME" && seconds.is_none_or(|seconds| !seconds.is_finite() || seconds <= 0.) {
        return Some("completion.delayInvalid");
    }
    if mode == "OFF" {
        return None;
    }
    if mode == "ON_MEDIA_END" && !requested["completionTimeSeconds"].is_null() {
        return Some("completion.delayInvalid");
    }
    let invalid = Some("completion.targetInvalid");
    if let Some(cue) = requested["completionTargetCueId"]
        .as_str()
        .filter(|id| !id.is_empty())
    {
        let found = item["slides"].as_array().is_some_and(|slides| {
            slides.iter().any(|slide| {
                slide["disabled"] != true && (slide["id"] == cue || slide["sourceCueId"] == cue)
            })
        });
        if !found {
            return invalid;
        }
    }
    let kind = requested["completionTargetType"].as_str();
    if kind.is_some_and(|kind| !["NEXT", "FIRST", "RANDOM", "CUE"].contains(&kind)) {
        return invalid;
    }
    let cue = requested["completionTargetCueId"]
        .as_str()
        .is_some_and(|id| !id.is_empty());
    let instance = requested["completionTargetInstanceId"]
        .as_str()
        .filter(|id| !id.is_empty());
    if kind == Some("CUE") && !cue && instance.is_none() {
        return invalid;
    }
    if instance.is_some_and(|wanted| !occurrences(item).iter().any(|(id, _)| id == wanted)) {
        return invalid;
    }
    None
}

/// What a save writes for the completion: nothing at all when it is off.
pub fn completion_to_save(requested: &Value) -> Value {
    if requested["completionAction"] == "OFF" {
        json!({})
    } else {
        read_completion(requested)
    }
}

/// `validateActions`: unique non-empty ids, then each action's own validation.
pub fn actions_error(actions: &[Value], facts: &Facts) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    for action in actions {
        let id = action["id"].as_str().unwrap_or("");
        if id.is_empty() || !seen.insert(id.to_string()) {
            return Some("automation.slideActions.invalidIds".into());
        }
        if let Some(error) = super::validate::action_error(action, facts, "") {
            return Some(error);
        }
    }
    None
}

/// The item with the slide's actions and completion replaced; every other field of every slide is kept.
pub fn with_changes(item: &Value, slide_id: &str, actions: &[Value], completion: &Value) -> Value {
    let mut next = item.clone();
    for slide in next["slides"].as_array_mut().into_iter().flatten() {
        if slide["id"] != slide_id {
            continue;
        }
        if actions.is_empty() && slide["actions"].is_null() {
            // An absent list stays absent.
        } else {
            slide["actions"] = json!(actions);
        }
        if let Some(object) = slide.as_object_mut() {
            for key in COMPLETION_FIELDS {
                object.remove(key);
            }
            for (key, value) in completion.as_object().into_iter().flatten() {
                object.insert(key.clone(), value.clone());
            }
        }
    }
    next
}

pub struct SlideAuthoring {
    runtime: Runtime,
    item_id: String,
    slide_id: String,
    form: Form,
    /// The saved actions and completion the dialog opened from, for the stale check.
    opened: (Value, Value),
    status: String,
    busy: bool,
    on_close: std::rc::Rc<dyn Fn(&mut gpui::App)>,
}

fn form_of(dialog: &mut SlideAuthoring) -> &mut Form {
    &mut dialog.form
}

impl SlideAuthoring {
    pub fn new(
        runtime: Runtime,
        item_id: String,
        slide_id: String,
        on_close: impl Fn(&mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let item = runtime
            .content
            .read(cx)
            .items
            .get(&item_id)
            .map(|item| (*item.value).clone())
            .unwrap_or(Value::Null);
        let slide = item["slides"]
            .as_array()
            .and_then(|slides| slides.iter().find(|slide| slide["id"] == slide_id.as_str()))
            .cloned()
            .unwrap_or(Value::Null);
        let mut form = Form::default();
        form.reset(json!({
            "actions": slide["actions"].as_array().cloned().unwrap_or_default(),
            "mode": completion_mode(&slide),
            "seconds": slide["completionTimeSeconds"].as_f64().map(|s| s.to_string()).unwrap_or_default(),
            "target": target_value(&slide),
        }));
        Self {
            runtime,
            item_id,
            slide_id,
            opened: (
                slide["actions"]
                    .as_array()
                    .map_or(json!([]), |actions| json!(actions)),
                read_completion(&slide),
            ),
            form,
            status: String::new(),
            busy: false,
            on_close: std::rc::Rc::new(on_close),
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let content = self.runtime.content.read(cx);
        let path = content.workspace_path.clone();
        let item = content
            .items
            .get(&self.item_id)
            .map(|item| (*item.value).clone());
        let slide = item.as_ref().and_then(|item| {
            item["slides"]
                .as_array()?
                .iter()
                .find(|slide| slide["id"] == self.slide_id.as_str())
                .cloned()
        });
        let (Some(item), Some(slide)) = (item, slide) else {
            self.status = "automation.slideActions.stale".into();
            cx.notify();
            return;
        };
        let current = (
            slide["actions"]
                .as_array()
                .map_or(json!([]), |actions| json!(actions)),
            read_completion(&slide),
        );
        let result = (|| {
            if path.is_empty() {
                return Err("automation.slideActions.workspaceRequired".to_string());
            }
            if current != self.opened {
                return Err("automation.slideActions.stale".to_string());
            }
            let actions: Vec<Value> = get(&self.form.draft, "actions")
                .as_array()
                .cloned()
                .unwrap_or_default();
            let facts = super::facts(&self.runtime, cx);
            if let Some(error) = actions_error(&actions, &facts) {
                return Err(error);
            }
            let mode = get(&self.form.draft, "mode")
                .as_str()
                .unwrap_or("OFF")
                .to_string();
            let seconds = get(&self.form.draft, "seconds")
                .as_str()
                .unwrap_or("")
                .to_string();
            let target = get(&self.form.draft, "target")
                .as_str()
                .unwrap_or("")
                .to_string();
            let requested = request(&mode, &seconds, &target);
            // An imported mode the editor has no fields for is kept unless the operator changes it.
            let completion = if MODES.contains(&mode.as_str()) || mode != completion_mode(&slide) {
                if let Some(error) = completion_error(&item, &requested) {
                    return Err(error.to_string());
                }
                completion_to_save(&requested)
            } else {
                read_completion(&slide)
            };
            let next = with_changes(&item, &self.slide_id, &actions, &completion);
            if next == item {
                return Ok(None);
            }
            Ok(Some(json!({"type": "mutateItem", "itemId": self.item_id,
                "operation": {"type": "setSlideDocument", "title": item["title"].as_str().unwrap_or(""),
                    "slides": next["slides"]}})))
        })();
        match result {
            Ok(None) => (self.on_close.clone())(cx),
            Ok(Some(mutation)) => {
                self.busy = true;
                self.status.clear();
                let runtime = self.runtime.clone();
                cx.spawn(async move |this, cx| {
                    let accepted = runtime
                        .persist(cx, Persist::new(Collection::Library, mutation, |_, _| {}))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.busy = false;
                        if accepted.is_some() {
                            (this.on_close.clone())(cx);
                        } else {
                            this.status = "automation.slideActions.saveFailed".into();
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(error) => self.status = error,
        }
        cx.notify();
    }
}

impl Render for SlideAuthoring {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t: Translator = Runtime::translator(cx);
        let gap = cx.theme().metrics.space_xs;
        let content = self.runtime.content.read(cx);
        let item = content
            .items
            .get(&self.item_id)
            .map(|item| (*item.value).clone())
            .unwrap_or(Value::Null);
        let workspace = content.workspace.clone();
        let mode = get(&self.form.draft, "mode")
            .as_str()
            .unwrap_or("OFF")
            .to_string();
        let mut modes = vec![
            ("OFF".to_string(), t.t("completion.off")),
            ("AFTER_TIME".to_string(), t.t("completion.afterTime")),
            ("ON_MEDIA_END".to_string(), t.t("completion.mediaEnd")),
        ];
        if !MODES.contains(&mode.as_str()) {
            modes.push((
                mode.clone(),
                format!("{} {mode}", t.t("completion.unsupported")),
            ));
        }
        let mut body = div()
            .id("slide-authoring")
            .flex()
            .flex_col()
            .gap(gap)
            .max_h(gpui::px(560.))
            .overflow_y_scroll()
            .child(div().child(t.t("automation.slideActions.execution")));
        body = body.child(action_list::render(
            &mut self.form,
            "slide-actions",
            "actions",
            &workspace,
            "",
            form_of,
            window,
            cx,
        ));
        body = body
            .child(div().child(t.t("completion.title")))
            .child(div().child(t.t("completion.timing")))
            .child(self.form.field(
                "completion",
                &Spec::new("mode", t.t("completion.mode"), Kind::Choice(modes)),
                form_of,
                window,
                cx,
            ));
        if mode == "AFTER_TIME" || mode == "ON_MEDIA_END" {
            if mode == "AFTER_TIME" {
                body = body.child(self.form.field(
                    "completion",
                    &Spec::new("seconds", t.t("completion.seconds"), Kind::Text),
                    form_of,
                    window,
                    cx,
                ));
            } else {
                body = body.child(div().child(t.t("completion.mediaEndHelp")));
            }
            let mut choices = vec![
                (String::new(), t.t("completion.next")),
                ("type:FIRST".to_string(), t.t("completion.first")),
                ("type:RANDOM".to_string(), t.t("completion.random")),
            ];
            choices.extend(targets(&item));
            choices.extend(occurrences(&item).into_iter().map(|(id, text)| {
                (
                    format!("instance:{id}"),
                    format!("{} {text}", t.t("completion.instance")),
                )
            }));
            let chosen = get(&self.form.draft, "target")
                .as_str()
                .unwrap_or("")
                .to_string();
            if !chosen.is_empty() && !choices.iter().any(|(value, _)| *value == chosen) {
                choices.push((chosen.clone(), chosen));
            }
            body = body
                .child(self.form.field(
                    "completion",
                    &Spec::new("target", t.t("completion.target"), Kind::Choice(choices)),
                    form_of,
                    window,
                    cx,
                ))
                .child(div().child(t.t("completion.targetsHelp")))
                .child(div().child(t.t("completion.instanceHelp")));
        }
        if !self.status.is_empty() {
            body = body.child(
                div()
                    .id("slide-authoring-status")
                    .role(gpui::Role::Alert)
                    .child(t.t(&self.status)),
            );
        }
        body = body.child(
            div()
                .flex()
                .gap(gap)
                .child(
                    ActionButton::new("slide-authoring-save", t.t("automation.macros.save"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                )
                .child({
                    let close = self.on_close.clone();
                    ActionButton::new(
                        SharedString::from("slide-authoring-cancel"),
                        t.t("common.cancel"),
                    )
                    .variant(ButtonVariant::Secondary)
                    .on_click(move |_, _, cx| close(cx))
                }),
        );
        let close = self.on_close.clone();
        Dialog::new(
            "slide-authoring",
            t.t("automation.slideActions.title"),
            t.t("common.close"),
            move |_, _, cx| close(cx),
        )
        .width(gpui::px(560.))
        .child(body)
    }
}
