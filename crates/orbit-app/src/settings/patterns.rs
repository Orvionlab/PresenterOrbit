//! `TestPatternsPanelBridge.tsx` and `shared/test-pattern.ts`. A pattern is shown by updating the `testPattern` of
//! each targeted screen with `mutateScreenConfig`; the output process draws it from the scene.

use super::ui::Field;
use gpui::{prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{ActionButton, CheckboxField, ColorPicker, Select, SelectOption};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::ui::{action_row, heading, muted, stack};
use super::view::SettingsView;

pub const KINDS: [&str; 12] = [
    "blend-grid",
    "color-bars",
    "focus",
    "gray-scale",
    "black",
    "white",
    "custom",
    "lines",
    "radar",
    "logo-bounce",
    "text",
    "audio-video-sync",
];

/// `createTestPattern`: the defaults follow ProPresenter's test pattern configuration.
pub fn create_pattern(kind: &str) -> Value {
    let motion = |extra: Value| {
        let mut base =
            json!({"kind": kind, "color": "#ffff00", "background": "#000000", "speed": 50});
        base.as_object_mut()
            .unwrap()
            .extend(extra.as_object().cloned().unwrap_or_default());
        base
    };
    match kind {
        "blend-grid" => {
            json!({"kind": kind, "spacing": 40, "centerCircle": true, "centerLines": true, "inverted": false, "grid": true})
        }
        "focus" => json!({"kind": kind, "grid": true, "spacing": 40}),
        "black" => json!({"kind": kind, "level": 0}),
        "white" => json!({"kind": kind, "level": 100}),
        "custom" => json!({"kind": kind, "color": "#0000ff"}),
        "lines" => motion(json!({})),
        "radar" => motion(json!({"grid": true, "spacing": 40})),
        "logo-bounce" => json!({"kind": kind, "speed": 50}),
        "text" => json!({"kind": kind, "text": "Presenter Orbit", "font": "Arial"}),
        _ => json!({"kind": kind}),
    }
}

/// The screens a target names: every screen, every screen of a type, or one screen.
pub fn target_ids(configs: &Value, target: &str) -> Vec<String> {
    if target.is_empty() {
        return Vec::new();
    }
    configs
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(id, config)| target == "all" || target == id.as_str() || config["type"] == target)
        .map(|(id, _)| id.clone())
        .collect()
}

/// The allowed range of a numeric field.
pub fn range(kind: &str, key: &str) -> (f64, f64) {
    match key {
        "spacing" => (2., 4096.),
        "speed" => (0., 100.),
        "level" if kind == "black" => (0., 50.),
        "level" => (50., 100.),
        _ => (0., f64::MAX),
    }
}

#[derive(Default)]
pub struct Patterns {
    pub target: String,
    pub draft: Option<Value>,
    pub error: Option<String>,
}

impl SettingsView {
    pub(super) fn pattern_draft(&self) -> Value {
        self.patterns
            .draft
            .clone()
            .unwrap_or_else(|| create_pattern("blend-grid"))
    }

    pub(super) fn pattern_edit(&mut self, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        let mut draft = self.pattern_draft();
        change(&mut draft);
        self.patterns.draft = Some(draft);
        cx.notify();
    }

    pub(super) fn pattern_target(&mut self, target: &str, cx: &mut Context<Self>) {
        self.patterns.target = target.to_string();
        let configs = self.screen_configs(cx);
        if let Some(current) = configs
            .get(target)
            .map(|c| c["testPattern"].clone())
            .filter(|p| p.is_object())
        {
            self.patterns.draft = Some(current);
        }
        cx.notify();
    }

    /// Shows the draft on the target screens, or clears them.
    pub(super) fn pattern_apply(&mut self, show: bool, cx: &mut Context<Self>) {
        let t = *cx.global::<Translator>();
        let ids = target_ids(&self.screen_configs(cx), &self.patterns.target);
        if ids.is_empty() {
            self.patterns.error = Some(t.t("testPatterns.screenRequired"));
            cx.notify();
            return;
        }
        self.patterns.error = None;
        let pattern = if show {
            self.pattern_draft()
        } else {
            Value::Null
        };
        for id in ids {
            self.update_screen(&id, json!({"testPattern": pattern}), cx);
        }
        cx.notify();
    }

    pub(super) fn patterns_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let configs = self.screen_configs(cx);
        let draft = self.pattern_draft();
        let kind = draft["kind"].as_str().unwrap_or("blend-grid").to_string();
        let weak = cx.weak_entity();
        let mut targets = vec![SelectOption::new(String::new(), t.t("common.none"))];
        for value in ["all", "audience", "stage"] {
            targets.push(SelectOption::new(
                value.to_string(),
                t.t(&format!("testPatterns.target.{value}")),
            ));
        }
        for (id, config) in configs.as_object().into_iter().flatten() {
            targets.push(SelectOption::new(
                id.clone(),
                config["label"].as_str().unwrap_or(id).to_string(),
            ));
        }
        let (target_weak, kind_weak) = (weak.clone(), weak.clone());
        let warning = cx.theme().colors.warning;
        let mut panel = stack(cx)
            .id("settings-test-patterns")
            .child(heading(t.t("testPatterns.title"), cx))
            .child(muted(t.t("testPatterns.help"), cx))
            .child(Field::new(
                t.t("testPatterns.screen"),
                Select::new("pattern-target", targets, self.patterns.target.clone()).on_change(
                    move |value, _, cx| {
                        target_weak
                            .update(cx, |view, cx| view.pattern_target(&value, cx))
                            .ok();
                    },
                ),
            ))
            .child(Field::new(
                t.t("testPatterns.pattern"),
                Select::new(
                    "pattern-kind",
                    KINDS
                        .iter()
                        .map(|k| {
                            SelectOption::new(k.to_string(), t.t(&format!("testPatterns.kind.{k}")))
                        })
                        .collect(),
                    kind.clone(),
                )
                .on_change(move |kind, _, cx| {
                    kind_weak
                        .update(cx, |view, cx| {
                            let old = view.pattern_draft();
                            let mut next = create_pattern(&kind);
                            for key in ["identify", "logo"] {
                                if !old[key].is_null() {
                                    next[key] = old[key].clone();
                                }
                            }
                            view.patterns.draft = Some(next);
                            cx.notify();
                        })
                        .ok();
                }),
            ));
        let fields: Vec<(String, Value)> = draft
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| !["kind", "identify", "logo"].contains(&key.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (key, value) in fields {
            let label = t.t(&format!("testPatterns.{key}"));
            let id = SharedString::from(format!("pattern-{key}"));
            panel = panel.child(match &value {
                Value::Bool(on) => {
                    let (weak, key) = (weak.clone(), key.clone());
                    CheckboxField::new(id, label, *on)
                        .on_change(move |checked, _, cx| {
                            let key = key.clone();
                            weak.update(cx, |view, cx| {
                                view.pattern_edit(|d| d[&key] = json!(checked), cx)
                            })
                            .ok();
                        })
                        .into_any_element()
                }
                Value::Number(number) => {
                    let (min, max) = range(&kind, &key);
                    let key2 = key.clone();
                    self.number_field(
                        &format!("pattern-{key}"),
                        label,
                        &number.to_string(),
                        (min, max),
                        1.,
                        window,
                        cx,
                        move |view, text, cx| {
                            if let Some(n) = text
                                .trim()
                                .parse::<f64>()
                                .ok()
                                .filter(|n| n.is_finite() && *n >= min && *n <= max)
                            {
                                view.pattern_edit(|d| d[&key2] = json!(n), cx);
                            }
                        },
                    )
                }
                Value::String(text) if text.starts_with('#') => {
                    let (weak, key) = (weak.clone(), key.clone());
                    Field::new(
                        label.clone(),
                        ColorPicker::new(id, label, text.clone()).on_change(move |color, _, cx| {
                            let key = key.clone();
                            weak.update(cx, |view, cx| {
                                view.pattern_edit(|d| d[&key] = json!(color), cx)
                            })
                            .ok();
                        }),
                    )
                    .into_any_element()
                }
                other => {
                    let key2 = key.clone();
                    self.text_field(
                        &format!("pattern-{key}"),
                        label,
                        other.as_str().unwrap_or(""),
                        window,
                        cx,
                        move |view, text, cx| view.pattern_edit(|d| d[&key2] = json!(text), cx),
                    )
                }
            });
        }
        let identify_weak = weak.clone();
        let logo = draft["logo"].as_str().unwrap_or("").to_string();
        let logo_weak = weak.clone();
        panel
            .child(
                CheckboxField::new(
                    "pattern-identify",
                    t.t("testPatterns.identify"),
                    draft["identify"] == json!(true),
                )
                .on_change(move |on, _, cx| {
                    identify_weak
                        .update(cx, |view, cx| {
                            view.pattern_edit(|d| d["identify"] = json!(on), cx)
                        })
                        .ok();
                }),
            )
            .child(Field::new(
                t.t("testPatterns.logo"),
                Select::new(
                    "pattern-logo",
                    {
                        let mut options = vec![
                            SelectOption::new(String::new(), t.t("common.none")),
                            SelectOption::new("orbit".to_string(), "Presenter Orbit".to_string()),
                        ];
                        if !logo.is_empty() && logo != "orbit" {
                            options.push(SelectOption::new("custom".to_string(), logo.clone()));
                        }
                        options
                    },
                    if logo.is_empty() || logo == "orbit" {
                        logo.clone()
                    } else {
                        "custom".to_string()
                    },
                )
                .on_change(move |value, _, cx| {
                    logo_weak
                        .update(cx, |view, cx| {
                            if value == "custom" {
                                return;
                            }
                            view.pattern_edit(
                                |d| {
                                    if value.is_empty() {
                                        if let Some(o) = d.as_object_mut() {
                                            o.remove("logo");
                                        }
                                    } else {
                                        d["logo"] = json!(value);
                                    }
                                },
                                cx,
                            )
                        })
                        .ok();
                }),
            ))
            .child(
                action_row(cx)
                    .child(
                        ActionButton::new("pattern-import-logo", t.t("testPatterns.importLogo"))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.host_call(
                                    "selectLogoImage",
                                    Vec::new(),
                                    cx,
                                    |view, result, cx| match result {
                                        Ok(r) if r["canceled"] != json!(true) => {
                                            let path =
                                                r["filePath"].as_str().unwrap_or("").to_string();
                                            view.pattern_edit(|d| d["logo"] = json!(path), cx);
                                        }
                                        Ok(_) => {}
                                        Err(error) => view.patterns.error = Some(error),
                                    },
                                );
                            })),
                    )
                    .child(
                        ActionButton::new("pattern-show", t.t("testPatterns.show"))
                            .disabled(self.patterns.target.is_empty())
                            .on_click(cx.listener(|view, _, _, cx| view.pattern_apply(true, cx))),
                    )
                    .child(
                        ActionButton::new("pattern-clear", t.t("testPatterns.clear"))
                            .on_click(cx.listener(|view, _, _, cx| view.pattern_apply(false, cx))),
                    ),
            )
            .children(
                self.patterns
                    .error
                    .clone()
                    .map(|e| gpui::div().id("pattern-error").text_color(warning).child(e)),
            )
            .into_any_element()
    }
}
