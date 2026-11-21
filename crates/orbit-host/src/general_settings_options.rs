//! Media import defaults and slide labels stored in the general settings.

use serde_json::{json, Map, Value};
use std::sync::RwLock;

const SCALES: &[&str] = &["fit", "fill", "stretch", "blur"];
const BEHAVIORS: &[&str] = &["foreground", "background"];
const PLAYBACKS: &[&str] = &["stop", "loop"];
const VIDEO_ENDS: &[&str] = &["hold", "black", "clear", "fade-black", "fade-clear"];

const MEDIA_IMPORT_OPTIONS: [(&str, &[&str], &str); 7] = [
    ("importScaleForeground", SCALES, "fit"),
    ("importScaleBackground", SCALES, "fit"),
    ("importImageBehavior", BEHAVIORS, "foreground"),
    ("importVideoBehavior", BEHAVIORS, "foreground"),
    ("importVideoPlayback", PLAYBACKS, "stop"),
    ("importVideoEndBehavior", VIDEO_ENDS, "hold"),
    ("importAudioPlayback", PLAYBACKS, "stop"),
];

static IMPORT_DEFAULTS: RwLock<Option<Value>> = RwLock::new(None);

fn default_slide_labels() -> Value {
    json!([
        { "id": "blank-slide", "name": "Blank Slide", "color": "#6b7280" },
        { "id": "instrumental", "name": "Instrumental", "color": "#2563eb" },
        { "id": "repeat", "name": "Repeat", "color": "#16a34a" },
        { "id": "spoken-word", "name": "Spoken Word", "color": "#d97706" }
    ])
}

pub fn normalize_into(configured: &Map<String, Value>, normalized: &mut Map<String, Value>) {
    for (key, options, default) in MEDIA_IMPORT_OPTIONS {
        let value = configured
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| options.contains(value))
            .unwrap_or(default);
        normalized.insert(key.to_string(), json!(value));
    }
    normalized.insert(
        "autoEnableScreensAtLaunch".to_string(),
        json!(configured
            .get("autoEnableScreensAtLaunch")
            .and_then(Value::as_bool)
            .unwrap_or(true)),
    );
    normalized.insert(
        "ignoreBackgroundColors".to_string(),
        json!(configured
            .get("ignoreBackgroundColors")
            .and_then(Value::as_bool)
            .unwrap_or(false)),
    );
    let labels = match configured.get("slideLabels").and_then(Value::as_array) {
        None => default_slide_labels(),
        Some(labels) => Value::Array(
            labels
                .iter()
                .filter_map(|label| {
                    let field = |key: &str| label.get(key).and_then(Value::as_str);
                    let id = field("id").filter(|id| !id.is_empty())?;
                    Some(json!({ "id": id, "name": field("name")?, "color": field("color")? }))
                })
                .collect(),
        ),
    };
    normalized.insert("slideLabels".to_string(), labels);
}

/// Remembers the import defaults so newly created media items can pick them up.
pub fn set(general_settings: &Value) {
    if let Ok(mut defaults) = IMPORT_DEFAULTS.write() {
        *defaults = Some(crate::settings::normalize_general_settings_value(
            general_settings,
        ));
    }
}

pub fn apply(item: &mut Map<String, Value>) {
    let general = IMPORT_DEFAULTS
        .read()
        .ok()
        .and_then(|defaults| defaults.clone());
    if let Some(general) = general {
        apply_with(item, &general);
    }
}

/// Fills in the configured import defaults on a new media item. Values that equal the
/// built-in behavior are not written, and a field the item already carries wins.
pub fn apply_with(item: &mut Map<String, Value>, general: &Value) {
    let setting = |key: &str| {
        let default = MEDIA_IMPORT_OPTIONS
            .iter()
            .find(|option| option.0 == key)
            .map_or("", |option| option.2);
        general
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_string()
    };
    let fill = |item: &mut Map<String, Value>, key: &str, value: String, built_in: &str| {
        if !item.contains_key(key) && value != built_in {
            item.insert(key.to_string(), json!(value));
        }
    };
    match item.get("type").and_then(Value::as_str) {
        Some(kind @ ("image" | "video")) => {
            let kind = kind.to_string();
            let behavior_key = if kind == "image" {
                "importImageBehavior"
            } else {
                "importVideoBehavior"
            };
            fill(
                item,
                "backgroundBehavior",
                setting(behavior_key),
                "foreground",
            );
            let background =
                item.get("backgroundBehavior").and_then(Value::as_str) == Some("background");
            let scale = setting(if background {
                "importScaleBackground"
            } else {
                "importScaleForeground"
            });
            fill(item, "backgroundScale", scale, "fit");
            if kind == "video" {
                fill(
                    item,
                    "backgroundPlayback",
                    setting("importVideoPlayback"),
                    "stop",
                );
                fill(
                    item,
                    "backgroundEndBehavior",
                    setting("importVideoEndBehavior"),
                    "hold",
                );
            }
        }
        Some("audio") => fill(
            item,
            "audioPlayback",
            setting("importAudioPlayback"),
            "stop",
        ),
        _ => {}
    }
}
