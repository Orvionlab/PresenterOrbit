//! `KeyFillFields.tsx`, `ProjectorCorrectionFields.tsx`, `OutputCompositionFields.tsx`, `key-fill.ts`, and
//! `projector-correction.ts`. Each is one `mutateScreenConfig` update of `keyFill` or `projectorCorrection`; the
//! host validates again.

use super::ui::Field;
use gpui::{prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant, CheckboxField, Select, SelectOption};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::ui::{action_row, heading, muted, stack};
use super::view::SettingsView;

/// `keyFillOwner`: the screen whose key output is `key_screen`.
pub fn key_fill_owner(configs: &Value, key_screen: &str) -> Option<String> {
    configs
        .as_object()?
        .iter()
        .find(|(_, config)| config["keyFill"]["keyScreenId"] == key_screen)
        .map(|(id, _)| id.clone())
}

/// The key output choices of a screen: other audience screens with the same target type and enabled state.
pub fn key_fill_choices(configs: &Value, screen_id: &str) -> Vec<(String, String)> {
    let config = &configs[screen_id];
    configs
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(id, screen)| id.as_str() != screen_id && screen["type"] == "audience")
        .filter(|(_, screen)| {
            matches!(config["targetType"].as_str(), Some("display" | "ndi"))
                && screen["targetType"] == config["targetType"]
                && (screen["enabled"] == json!(true)) == (config["enabled"] == json!(true))
        })
        .map(|(id, screen)| {
            (
                id.clone(),
                screen["label"].as_str().unwrap_or(id).to_string(),
            )
        })
        .collect()
}

pub fn create_projector_correction() -> Value {
    json!({
        "edges": {"left": 0, "right": 0, "top": 0, "bottom": 0},
        "blendMode": "linear", "blendGamma": 2.2,
        "gain": {"red": 1, "green": 1, "blue": 1}, "blackLevel": 0,
        "corners": {"topLeft": {"x": 0, "y": 0}, "topRight": {"x": 1, "y": 0},
                    "bottomRight": {"x": 1, "y": 1}, "bottomLeft": {"x": 0, "y": 1}},
    })
}

/// `projectorCorrectionError`: a translation key.
pub fn projector_error(value: &Value) -> Option<&'static str> {
    let in_range = |v: &Value, min: f64, max: f64| {
        v.as_f64()
            .is_some_and(|v| v.is_finite() && v >= min && v <= max)
    };
    let edges = &value["edges"];
    let edge = |key: &str| edges[key].as_f64().unwrap_or(f64::NAN);
    if !["left", "right", "top", "bottom"]
        .iter()
        .all(|k| in_range(&edges[*k], 0., 0.5))
        || edge("left") + edge("right") >= 1.
        || edge("top") + edge("bottom") >= 1.
        || !["linear", "quadratic", "cubic"]
            .iter()
            .any(|m| value["blendMode"] == *m)
        || !in_range(&value["blendGamma"], 0.1, 5.)
        || !["red", "green", "blue"]
            .iter()
            .all(|k| in_range(&value["gain"][*k], 0., 2.))
        || !in_range(&value["blackLevel"], 0., 0.5)
    {
        return Some("screenSettings.projector.invalidValues");
    }
    let names = ["topLeft", "topRight", "bottomRight", "bottomLeft"];
    let points: Vec<(f64, f64)> = names
        .iter()
        .map(|n| {
            (
                value["corners"][*n]["x"].as_f64().unwrap_or(f64::NAN),
                value["corners"][*n]["y"].as_f64().unwrap_or(f64::NAN),
            )
        })
        .collect();
    if points
        .iter()
        .any(|(x, y)| !(0. ..=1.).contains(x) || !(0. ..=1.).contains(y))
    {
        return Some("screenSettings.projector.invalidCorners");
    }
    for i in 0..4 {
        let (a, b, c) = (points[i], points[(i + 1) % 4], points[(i + 2) % 4]);
        if (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0) <= 0.000001 {
            return Some("screenSettings.projector.invalidCorners");
        }
    }
    None
}

#[derive(Default)]
pub struct Composition {
    /// Drafts per screen id; applied with the Apply buttons.
    pub key: std::collections::HashMap<String, Value>,
    pub projector: std::collections::HashMap<String, Option<Value>>,
}

impl SettingsView {
    fn projector_draft(&self, id: &str, config: &Value) -> Option<Value> {
        self.composition
            .projector
            .get(id)
            .cloned()
            .unwrap_or_else(|| {
                config["projectorCorrection"]
                    .is_object()
                    .then(|| config["projectorCorrection"].clone())
            })
    }

    pub(super) fn key_fill_draft(&self, id: &str, config: &Value) -> Value {
        self.composition
            .key
            .get(id)
            .cloned()
            .unwrap_or_else(|| config["keyFill"].clone())
    }

    pub(super) fn set_projector(&mut self, id: &str, draft: Option<Value>, cx: &mut Context<Self>) {
        self.composition.projector.insert(id.to_string(), draft);
        cx.notify();
    }

    pub(super) fn apply_key_fill(&mut self, id: &str, cx: &mut Context<Self>) {
        let configs = self.screen_configs(cx);
        let draft = self.key_fill_draft(id, &configs[id]);
        self.update_screen(id, json!({"keyFill": draft}), cx);
        self.composition.key.remove(id);
    }

    pub(super) fn apply_projector(&mut self, id: &str, cx: &mut Context<Self>) {
        let configs = self.screen_configs(cx);
        let draft = self.projector_draft(id, &configs[id]);
        if draft.as_ref().and_then(projector_error).is_some() {
            return;
        }
        self.update_screen(id, json!({"projectorCorrection": draft}), cx);
        self.composition.projector.remove(id);
    }

    pub(super) fn composition_fields(
        &mut self,
        id: &str,
        config: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let configs = self.screen_configs(cx);
        let weak = cx.weak_entity();
        let mut body = stack(cx);
        if config["type"] == "audience" {
            body = body.child(heading(t.t("screenSettings.keyFill.title"), cx));
            if let Some(owner) = key_fill_owner(&configs, id) {
                body = body.child(muted(
                    t.format(
                        "screenSettings.keyFill.inherits",
                        &[(
                            "label",
                            configs[owner.as_str()]["label"].as_str().unwrap_or(&owner),
                        )],
                    ),
                    cx,
                ));
            } else {
                let draft = self.key_fill_draft(id, config);
                let key = draft["keyScreenId"].as_str().unwrap_or("").to_string();
                let mut options = vec![SelectOption::new(
                    String::new(),
                    t.t("screenSettings.keyFill.disabled"),
                )];
                options.extend(
                    key_fill_choices(&configs, id)
                        .into_iter()
                        .map(|(i, l)| SelectOption::new(i, l)),
                );
                let (key_weak, mode_weak, screen) = (weak.clone(), weak.clone(), id.to_string());
                let mode_screen = screen.clone();
                let mode = draft["fillMode"]
                    .as_str()
                    .unwrap_or("premultiplied")
                    .to_string();
                body = body
                    .child(muted(t.t("screenSettings.keyFill.help"), cx))
                    .child(Field::new(
                        t.t("screenSettings.keyFill.keyOutput"),
                        Select::new(
                            SharedString::from(format!("keyfill-{id}")),
                            options,
                            key.clone(),
                        )
                        .on_change(move |value, _, cx| {
                            let screen = screen.clone();
                            key_weak
                                .update(cx, |view, cx| {
                                    let next = if value.is_empty() {
                                        Value::Null
                                    } else {
                                        json!({"keyScreenId": value, "fillMode": "premultiplied"})
                                    };
                                    view.composition.key.insert(screen, next);
                                    cx.notify();
                                })
                                .ok();
                        }),
                    ));
                if !key.is_empty() {
                    body = body.child(Field::new(
                        t.t("screenSettings.keyFill.fillMode"),
                        Select::new(
                            SharedString::from(format!("keyfill-mode-{id}")),
                            vec![
                                SelectOption::new(
                                    "straight".to_string(),
                                    t.t("screenSettings.keyFill.straight"),
                                ),
                                SelectOption::new(
                                    "premultiplied".to_string(),
                                    t.t("screenSettings.keyFill.premultiplied"),
                                ),
                            ],
                            mode,
                        )
                        .on_change(move |value, _, cx| {
                            let screen = mode_screen.clone();
                            mode_weak
                                .update(cx, |view, cx| {
                                    let current = view
                                        .composition
                                        .key
                                        .get(&screen)
                                        .cloned()
                                        .unwrap_or(Value::Null);
                                    if current.is_object() {
                                        let mut next = current;
                                        next["fillMode"] = json!(value);
                                        view.composition.key.insert(screen, next);
                                        cx.notify();
                                    }
                                })
                                .ok();
                        }),
                    ));
                }
                let apply_id = id.to_string();
                body = body.child(
                    ActionButton::new(
                        SharedString::from(format!("keyfill-apply-{id}")),
                        t.t("screenSettings.keyFill.apply"),
                    )
                    .on_click(
                        cx.listener(move |view, _, _, cx| view.apply_key_fill(&apply_id, cx)),
                    ),
                );
            }
        }
        let paired = config["keyFill"].is_object() || key_fill_owner(&configs, id).is_some();
        if !paired {
            body = body.child(self.projector_fields(id, config, window, cx));
        }
        body.into_any_element()
    }

    fn projector_fields(
        &mut self,
        id: &str,
        config: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let draft = self.projector_draft(id, config);
        let weak = cx.weak_entity();
        let toggle_id = id.to_string();
        let mut body = stack(cx)
            .child(heading(t.t("screenSettings.projector.title"), cx))
            .child(
                CheckboxField::new(
                    SharedString::from(format!("projector-enabled-{id}")),
                    t.t("screenSettings.projector.enabled"),
                    draft.is_some(),
                )
                .on_change(move |on, _, cx| {
                    let toggle_id = toggle_id.clone();
                    weak.update(cx, |view, cx| {
                        view.set_projector(&toggle_id, on.then(create_projector_correction), cx)
                    })
                    .ok();
                }),
            );
        let Some(value) = draft else {
            let clear_id = id.to_string();
            let saved = config["projectorCorrection"].is_object();
            return body
                .children(saved.then(|| {
                    ActionButton::new(
                        SharedString::from(format!("projector-clear-{id}")),
                        t.t("screenSettings.keyFill.apply"),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.update_screen(&clear_id, json!({"projectorCorrection": null}), cx);
                        view.composition.projector.remove(&clear_id);
                    }))
                }))
                .into_any_element();
        };
        body = body.child(muted(t.t("screenSettings.projector.help"), cx));
        let fields: [(&str, &[&str], &str, f64, f64); 4] = [
            ("edges", &["left", "right", "top", "bottom"], "", 0., 0.5),
            ("gain", &["red", "green", "blue"], "", 0., 2.),
            (
                "blendGamma",
                &[],
                "screenSettings.projector.blendGamma",
                0.1,
                5.,
            ),
            (
                "blackLevel",
                &[],
                "screenSettings.projector.blackLevel",
                0.,
                0.5,
            ),
        ];
        for (group, keys, label, min, max) in fields {
            let paths: Vec<(Vec<String>, String)> = if keys.is_empty() {
                vec![(vec![group.to_string()], label.to_string())]
            } else {
                keys.iter()
                    .map(|k| {
                        (
                            vec![group.to_string(), k.to_string()],
                            format!("screenSettings.projector.{k}"),
                        )
                    })
                    .collect()
            };
            for (path, label) in paths {
                let current = path
                    .iter()
                    .fold(&value, |v, k| &v[k.as_str()])
                    .as_f64()
                    .unwrap_or(0.);
                let screen = id.to_string();
                let edit_value = value.clone();
                body = body.child(self.number_field(
                    &format!("projector-{id}-{}", path.join("-")),
                    t.t(&label),
                    &current.to_string(),
                    (min, max),
                    0.01,
                    window,
                    cx,
                    move |view, text, cx| {
                        if let Ok(n) = text.trim().parse::<f64>() {
                            let mut next = edit_value.clone();
                            let mut slot = &mut next;
                            for k in &path {
                                slot = &mut slot[k.as_str()];
                            }
                            *slot = json!(n);
                            view.set_projector(&screen, Some(next), cx);
                        }
                    },
                ));
            }
        }
        let blend_weak = cx.weak_entity();
        let blend_value = value.clone();
        let blend_screen = id.to_string();
        body = body.child(Field::new(
            t.t("screenSettings.projector.blendMode"),
            Select::new(
                SharedString::from(format!("projector-blend-{id}")),
                ["linear", "quadratic", "cubic"]
                    .iter()
                    .map(|m| {
                        SelectOption::new(
                            m.to_string(),
                            t.t(&format!("screenSettings.projector.{m}")),
                        )
                    })
                    .collect(),
                value["blendMode"].as_str().unwrap_or("linear").to_string(),
            )
            .on_change(move |mode, _, cx| {
                let mut next = blend_value.clone();
                next["blendMode"] = json!(mode);
                let screen = blend_screen.clone();
                blend_weak
                    .update(cx, |view, cx| view.set_projector(&screen, Some(next), cx))
                    .ok();
            }),
        ));
        for corner in ["topLeft", "topRight", "bottomRight", "bottomLeft"] {
            for axis in ["x", "y"] {
                let (screen, edit_value) = (id.to_string(), value.clone());
                let current = value["corners"][corner][axis].as_f64().unwrap_or(0.);
                body = body.child(self.number_field(
                    &format!("projector-{id}-{corner}-{axis}"),
                    format!(
                        "{} {}",
                        t.t(&format!("screenSettings.projector.{corner}")),
                        axis
                    ),
                    &current.to_string(),
                    (0., 1.),
                    0.01,
                    window,
                    cx,
                    move |view, text, cx| {
                        if let Ok(n) = text.trim().parse::<f64>() {
                            let mut next = edit_value.clone();
                            next["corners"][corner][axis] = json!(n);
                            view.set_projector(&screen, Some(next), cx);
                        }
                    },
                ));
            }
        }
        let error = projector_error(&value);
        if error.is_none() {
            body = body.child(super::video_preview::polygon(
                super::video_preview::corner_points(&value),
                cx.theme().colors.accent,
                160.,
                90.,
            ));
        }
        let (apply_id, reset_id) = (id.to_string(), id.to_string());
        body.child(
            action_row(cx)
                .child(
                    ActionButton::new(
                        SharedString::from(format!("projector-apply-{id}")),
                        t.t("screenSettings.keyFill.apply"),
                    )
                    .disabled(error.is_some())
                    .on_click(
                        cx.listener(move |view, _, _, cx| view.apply_projector(&apply_id, cx)),
                    ),
                )
                .child(
                    ActionButton::new(
                        SharedString::from(format!("projector-reset-{id}")),
                        t.t("screenSettings.projector.reset"),
                    )
                    .variant(ButtonVariant::Secondary)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.set_projector(&reset_id, Some(create_projector_correction()), cx)
                    })),
                ),
        )
        .children(error.map(|key| muted(t.t(key), cx)))
        .into_any_element()
    }
}
