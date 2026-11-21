//! Macro, message, and timer fields as [`Spec`] lists over the JSON draft.

use orbit_ui::Translator;
use serde_json::{json, Value};

use super::form::{get, Form, Kind, Spec};

const ICONS: &str = include_str!("../../../orbit-host/assets/macro-icons.json");

/// `macroIconCatalogue`: `(id, label)` of the built-in icons, with a symbol or a translated name as the label.
pub fn icon_catalogue(t: Translator) -> Vec<(String, String)> {
    let data: Value = serde_json::from_str(ICONS).unwrap_or(Value::Null);
    let mut icons = Vec::new();
    for entry in data["symbols"].as_str().unwrap_or("").split('|') {
        if let Some((name, symbol)) = entry.split_once(':') {
            let label = t.t(&format!("automation.macros.icon.{name}"));
            icons.push((format!("ImageType{name}"), format!("{symbol} {label}")));
        }
    }
    for (index, name) in data["digits"].as_str().unwrap_or("").split('|').enumerate() {
        icons.push((format!("ImageType{name}"), ((index + 1) % 10).to_string()));
    }
    for letter in data["letters"].as_str().unwrap_or("").chars() {
        icons.push((format!("ImageTypeLetter{letter}"), letter.to_string()));
    }
    icons
}

fn choose_icon(form: &mut Form, _path: &str, id: &str) {
    form.set("imageType", json!(id));
    form.set("imageData", Value::Null);
}

pub fn macro_specs(draft: &Value, collections: &[Value], t: Translator) -> Vec<Spec> {
    let mut specs = vec![
        Spec::new("title", t.t("automation.macros.name"), Kind::Text),
        Spec::new(
            "triggerOnStartup",
            t.t("automation.macros.startup"),
            Kind::Check,
        ),
    ];
    for collection in collections {
        if let Some(id) = collection["id"].as_str() {
            specs.push(Spec::new(
                "collectionIds",
                collection["title"].as_str().unwrap_or(id),
                Kind::Member(id.to_string()),
            ));
        }
    }
    let current = if draft["imageData"].is_string() {
        String::new()
    } else {
        draft["imageType"]
            .as_str()
            .unwrap_or("ImageTypeDefault")
            .to_string()
    };
    specs.push(Spec::new(
        "imageType",
        t.t("automation.macros.builtInIcon"),
        Kind::Hook(icon_catalogue(t), choose_icon, current),
    ));
    specs
}

fn choose_theme(form: &mut Form, _path: &str, chosen: &str) {
    let (theme, slide) = chosen.split_once('\n').unwrap_or(("", ""));
    form.set(
        "themeId",
        if theme.is_empty() {
            Value::Null
        } else {
            json!(theme)
        },
    );
    form.set(
        "themeSlideId",
        if slide.is_empty() {
            Value::Null
        } else {
            json!(slide)
        },
    );
}

fn set_clear_type(form: &mut Form, _path: &str, chosen: &str) {
    form.set("clearType", json!(chosen));
    if chosen != "after-time" {
        form.set("clearAfterSeconds", Value::Null);
    }
}

pub fn message_specs(draft: &Value, workspace: &Value, t: Translator) -> Vec<Spec> {
    let themes = workspace["themes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut options = vec![(String::new(), t.t("automation.messages.defaultTheme"))];
    for theme in themes {
        for (index, slide) in theme["slides"].as_array().into_iter().flatten().enumerate() {
            options.push((
                format!(
                    "{}\n{}",
                    theme["id"].as_str().unwrap_or(""),
                    slide["id"].as_str().unwrap_or("")
                ),
                format!(
                    "{} / {}",
                    theme["title"]
                        .as_str()
                        .or(theme["id"].as_str())
                        .unwrap_or(""),
                    slide["name"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| (index + 1).to_string())
                ),
            ));
        }
    }
    let theme_id = draft["themeId"].as_str().unwrap_or("");
    let theme_value = if theme_id.is_empty() {
        String::new()
    } else {
        let slide = draft["themeSlideId"]
            .as_str()
            .map(str::to_string)
            .or_else(|| {
                themes
                    .iter()
                    .find(|theme| theme["id"] == theme_id)
                    .and_then(|theme| theme["slides"][0]["id"].as_str().map(str::to_string))
            });
        format!("{theme_id}\n{}", slide.unwrap_or_default())
    };
    let clear_type = draft["clearType"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| {
            if draft["clearAfterSeconds"]
                .as_f64()
                .is_some_and(|seconds| seconds != 0.)
            {
                "after-time"
            } else {
                "manual"
            }
            .into()
        });
    let mut specs = vec![
        Spec::new("title", t.t("automation.messages.name"), Kind::Text),
        Spec::new("text", t.t("automation.messages.text"), Kind::Multiline),
        Spec::new(
            "themeId",
            t.t("automation.messages.theme"),
            Kind::Hook(options, choose_theme, theme_value),
        ),
    ];
    let timers: Vec<(String, String)> =
        std::iter::once((String::new(), t.t("automation.messages.chooseTimer")))
            .chain(
                workspace["countdownTimers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|timer| {
                        (
                            timer["id"].as_str().unwrap_or("").to_string(),
                            timer["name"].as_str().unwrap_or("").to_string(),
                        )
                    }),
            )
            .collect();
    for (index, token) in draft["tokens"].as_array().into_iter().flatten().enumerate() {
        let at = |field: &str| format!("tokens.{index}.{field}");
        specs.push(Spec::new(
            at("name"),
            t.t("automation.messages.tokenName"),
            Kind::Text,
        ));
        specs.push(Spec::new(
            at("type"),
            t.t("automation.messages.tokenType"),
            Kind::Choice(
                ["text", "clock", "timer"]
                    .iter()
                    .map(|kind| {
                        (
                            kind.to_string(),
                            t.t(&format!("automation.messages.{kind}Token")),
                        )
                    })
                    .collect(),
            ),
        ));
        match token["type"].as_str() {
            Some("text") => specs.push(Spec::new(
                format!("tokenValues.{}", token["name"].as_str().unwrap_or("")),
                t.t("automation.messages.tokenValue"),
                Kind::Text,
            )),
            Some("timer") => specs.push(Spec::new(
                at("timerId"),
                t.t("automation.messages.timer"),
                Kind::Choice(timers.clone()),
            )),
            _ => {}
        }
    }
    specs.push(Spec::new(
        "clearType",
        t.t("automation.messages.clearPolicy"),
        Kind::Hook(
            [
                ("manual", "manual"),
                ("after-time", "afterTime"),
                ("after-timers", "afterTimers"),
            ]
            .iter()
            .map(|(value, key)| {
                (
                    value.to_string(),
                    t.t(&format!("automation.messages.{key}")),
                )
            })
            .collect(),
            set_clear_type,
            clear_type.clone(),
        ),
    ));
    if clear_type == "after-time" {
        specs.push(Spec::new(
            "clearAfterSeconds",
            t.t("automation.messages.seconds"),
            Kind::Number,
        ));
    }
    specs
}

pub fn add_token(form: &mut Form, id: String) {
    let mut tokens = get(&form.draft, "tokens")
        .as_array()
        .cloned()
        .unwrap_or_default();
    tokens.push(json!({"id": id, "name": "", "type": "text"}));
    form.set("tokens", Value::Array(tokens));
    form.refresh();
}

pub fn remove_token(form: &mut Form, index: usize) {
    let mut tokens = get(&form.draft, "tokens")
        .as_array()
        .cloned()
        .unwrap_or_default();
    if index < tokens.len() {
        tokens.remove(index);
        form.set("tokens", Value::Array(tokens));
        form.refresh();
    }
}

/// A renamed token keeps its text value, and a removed token drops it (`MessageEditorFields`). `opened` holds the
/// `(id, name)` pairs the editor started with.
pub fn sync_token_names(draft: &mut Value, opened: &[(String, String)]) {
    let tokens = draft["tokens"].as_array().cloned().unwrap_or_default();
    let Some(values) = draft["tokenValues"].as_object_mut() else {
        return;
    };
    for (id, old) in opened {
        match tokens.iter().find(|token| token["id"] == id.as_str()) {
            Some(token) => {
                let name = token["name"].as_str().unwrap_or("");
                if name != old && !values.contains_key(name) {
                    if let Some(value) = values.remove(old) {
                        values.insert(name.to_string(), value);
                    }
                }
            }
            None => {
                values.remove(old);
            }
        }
    }
    if values.is_empty() && opened.is_empty() {
        draft
            .as_object_mut()
            .map(|draft| draft.remove("tokenValues"));
    }
}

/// `saveMessage`: manual and after-timers messages carry no duration; the title is trimmed.
pub fn message_to_save(draft: &Value) -> Value {
    let mut saved = draft.clone();
    saved["title"] = json!(draft["title"].as_str().unwrap_or("").trim());
    if matches!(draft["clearType"].as_str(), Some("manual" | "after-timers")) {
        if let Some(object) = saved.as_object_mut() {
            object.remove("clearAfterSeconds");
        }
    }
    saved
}

fn set_timer_type(form: &mut Form, _path: &str, kind: &str) {
    let seconds = {
        use chrono::Timelike;
        chrono::Local::now().num_seconds_from_midnight()
    };
    let patch = super::validate::timer_type_patch(kind, seconds);
    for (key, value) in patch.as_object().into_iter().flatten() {
        form.set(key, value.clone());
    }
    if form.draft["format"]["targetTimePeriod"].is_string() {
        form.set("format.targetTimePeriod", Value::Null);
    }
    // The fields of the other types do not apply any more.
    for key in [
        "durationSeconds",
        "targetTimeSeconds",
        "elapsedStartSeconds",
        "elapsedEndSeconds",
    ] {
        if !patch
            .as_object()
            .is_some_and(|fields| fields.contains_key(key))
        {
            form.set(key, Value::Null);
        }
    }
}

pub fn timer_specs(draft: &Value, t: Translator) -> Vec<Spec> {
    let mut specs = vec![Spec::new("name", t.t("timer.name"), Kind::Text)];
    specs.extend(timer_configuration_specs(draft, t));
    specs
}

pub fn timer_configuration_specs(draft: &Value, t: Translator) -> Vec<Spec> {
    let kind = draft["type"].as_str().unwrap_or("countdown");
    let mut specs = vec![Spec::new(
        "type",
        t.t("timer.type"),
        Kind::Hook(
            [
                ("countdown", "timer.type.countdown"),
                ("countdown-to-time", "timer.type.countdownToTime"),
                ("elapsed", "timer.type.elapsed"),
            ]
            .iter()
            .map(|(value, key)| (value.to_string(), t.t(key)))
            .collect(),
            set_timer_type,
            kind.to_string(),
        ),
    )];
    match kind {
        "countdown-to-time" => specs.push(Spec::new(
            "targetTimeSeconds",
            t.t("timer.targetTime"),
            Kind::Time,
        )),
        "elapsed" => specs.extend([
            Spec::new("elapsedStartSeconds", t.t("timer.startAt"), Kind::Time),
            Spec::new("elapsedEndSeconds", t.t("timer.endAt"), Kind::Time),
        ]),
        _ => specs.push(Spec::new(
            "durationSeconds",
            t.t("timer.duration"),
            Kind::Time,
        )),
    }
    specs.push(Spec::new(
        "allowsOverrun",
        t.t("timer.allowOverrun"),
        Kind::Check,
    ));
    specs
}

const TIMER_FIELDS: [&str; 8] = [
    "name",
    "allowsOverrun",
    "durationSeconds",
    "type",
    "targetTimeSeconds",
    "elapsedEndSeconds",
    "elapsedStartSeconds",
    "format",
];

/// The `update` patch of an edited timer: every definition field that differs from the saved one, with `null` for
/// the optional fields that were cleared. `None` when nothing changed.
pub fn timer_patch(original: &Value, draft: &Value) -> Option<Value> {
    let mut patch = serde_json::Map::new();
    for key in TIMER_FIELDS {
        if original[key] != draft[key] {
            patch.insert(key.to_string(), draft[key].clone());
        }
    }
    if patch.get("durationSeconds").is_some_and(Value::is_null) {
        patch.remove("durationSeconds");
    }
    (!patch.is_empty()).then(|| Value::Object(patch))
}
