//! The conditional visibility section of the shape inspector (`ObjectVisibilityControls`).
use super::{
    catalog::{self, Field as Spec, Kind, Target},
    SlideCanvas,
};
use crate::canvas::object_fields;
use gpui::{div, prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant, IconButton};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

const KINDS: [&str; 6] = [
    "timer",
    "element",
    "video",
    "audio",
    "capture",
    "video-input",
];

impl SlideCanvas {
    pub(super) fn visibility_controls(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let gap = cx.theme().metrics.space_xs;
        let layer = self.slide["layers"][index].clone();
        let link = layer["visibilityLink"].clone();
        let mut body = div().flex().flex_col().gap(gap).child(
            self.inspector_field(
                Spec::new(
                    "visibilityEnabled",
                    Target::Object,
                    Kind::Bool,
                    json!(link.is_object()),
                )
                .named("theme.visibility.conditional"),
                window,
                cx,
            ),
        );
        if !link.is_object() {
            return body.into_any_element();
        }
        let timers: Vec<(String, String)> = self.runtime.content.read(cx).workspace
            ["countdownTimers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|timer| {
                Some((timer["id"].as_str()?.into(), timer["name"].as_str()?.into()))
            })
            .collect();
        let inputs: Vec<(String, String)> = self.runtime.content.read(cx).workspace["videoInputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|input| {
                Some((
                    input["id"].as_str()?.into(),
                    input["title"].as_str()?.into(),
                ))
            })
            .collect();
        let objects: Vec<(String, String)> = self.slide["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(position, _)| *position != index)
            .filter_map(|(_, other)| {
                let id = other["id"].as_str()?;
                Some((id.into(), other["name"].as_str().unwrap_or(id).into()))
            })
            .collect();
        body = body.child(
            self.inspector_field(
                Spec::new(
                    "visibilityCriterion",
                    Target::Object,
                    catalog::labeled(&[
                        ("all", "theme.visibility.all"),
                        ("any", "theme.visibility.any"),
                        ("none", "theme.visibility.none"),
                    ]),
                    link["criterion"].clone(),
                )
                .named("theme.visibility.showWhen"),
                window,
                cx,
            ),
        );
        let conditions = link["conditions"].as_array().cloned().unwrap_or_default();
        for (row, condition) in conditions.iter().enumerate() {
            let kind = object_fields::kind(condition);
            let available = |name: &str| match name {
                "timer" => !timers.is_empty(),
                "element" => !objects.is_empty(),
                "video-input" => !inputs.is_empty(),
                _ => true,
            };
            let kinds = KINDS
                .into_iter()
                .filter(|name| *name == kind || available(name))
                .map(|name| {
                    (
                        name.to_string(),
                        format!("i18n:theme.visibility.kind.{name}"),
                    )
                })
                .collect();
            body = body.child(
                self.inspector_field(
                    Spec::new(
                        &format!("visibilityKind:{row}"),
                        Target::Object,
                        Kind::Choice(kinds),
                        json!(kind),
                    )
                    .named("theme.visibility.kind"),
                    window,
                    cx,
                ),
            );
            // A reference that no longer resolves stays selectable under its stored name, as in the renderer.
            let reference = |key: &str, name: &str, known: &[(String, String)]| {
                let mut options = known.to_vec();
                let id = condition[key].as_str().unwrap_or("");
                if !options.iter().any(|(option, _)| option == id) {
                    options.insert(
                        0,
                        (
                            id.to_string(),
                            condition[name]
                                .as_str()
                                .filter(|n| !n.is_empty())
                                .unwrap_or(id)
                                .into(),
                        ),
                    );
                }
                Kind::Choice(options)
            };
            match kind {
                "timer" => {
                    body = body.child(
                        self.inspector_field(
                            Spec::new(
                                &format!("visibilityTimer:{row}"),
                                Target::Object,
                                reference("timerId", "timerName", &timers),
                                condition["timerId"].clone(),
                            )
                            .named("theme.linkedText.countdownTimer"),
                            window,
                            cx,
                        ),
                    );
                }
                "element" => {
                    body = body.child(
                        self.inspector_field(
                            Spec::new(
                                &format!("visibilityObject:{row}"),
                                Target::Object,
                                reference("objectId", "objectName", &objects),
                                condition["objectId"].clone(),
                            )
                            .named("theme.visibility.element"),
                            window,
                            cx,
                        ),
                    );
                }
                "video-input" => {
                    let id = condition["inputId"].as_str().unwrap_or("");
                    let mut options = inputs.clone();
                    if !options.iter().any(|(option, _)| option == id) {
                        options.insert(
                            0,
                            (
                                id.to_string(),
                                t.format(
                                    "theme.visibility.unmappedInput",
                                    &[("index", &condition["inputIndex"].to_string())],
                                ),
                            ),
                        );
                    }
                    body = body.child(
                        self.inspector_field(
                            Spec::new(
                                &format!("visibilityInput:{row}"),
                                Target::Object,
                                Kind::Choice(options),
                                json!(id),
                            )
                            .named("theme.visibility.videoInput"),
                            window,
                            cx,
                        ),
                    );
                }
                _ => {}
            }
            let criteria = object_fields::criteria(kind)
                .into_iter()
                .map(|criterion| {
                    (
                        criterion.to_string(),
                        format!("i18n:theme.visibility.{criterion}"),
                    )
                })
                .collect();
            body = body.child(
                self.inspector_field(
                    Spec::new(
                        &format!("visibilityCondition:{row}"),
                        Target::Object,
                        Kind::Choice(criteria),
                        condition["criterion"].clone(),
                    )
                    .named("theme.visibility.condition"),
                    window,
                    cx,
                ),
            );
            let weak = cx.weak_entity();
            let remove = Spec::new(
                &format!("visibilityRemove:{row}"),
                Target::Object,
                Kind::Bool,
                Value::Null,
            );
            body = body.child(
                IconButton::new(
                    SharedString::from(format!("visibility-remove-{row}")),
                    "x",
                    t.t("theme.linkedText.removeTrigger"),
                )
                .on_click(move |_, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.patch_inspector(&remove, Value::Null, cx)
                    })
                    .ok();
                }),
            );
        }
        let weak = cx.weak_entity();
        let add = Spec::new("visibilityAdd", Target::Object, Kind::Bool, Value::Null);
        body = body.child(
            ActionButton::new("visibility-add", t.t("theme.visibility.addCondition"))
                .variant(ButtonVariant::Ghost)
                .on_click(move |_, _, cx| {
                    weak.update(cx, |view, cx| view.patch_inspector(&add, Value::Null, cx))
                        .ok();
                }),
        );
        body.into_any_element()
    }
}
