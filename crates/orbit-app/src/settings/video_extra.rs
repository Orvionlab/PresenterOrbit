//! The rest of the Video panel: NDI mode and the stage layout
//! (`VideoOutputDetails.tsx`, `VideoOutputSettingsBodies.tsx`, `screen-output-utils.ts`).

use super::ui::Field;
use gpui::{prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::controls::{Select, SelectOption};
use orbit_ui::Translator;
use serde_json::{json, Map, Value};

use super::ui::stack;
use super::view::SettingsView;

/// `NDI_RESOLUTION_PRESETS` without the custom entry: value, width, height, frame rate.
pub const NDI_PRESETS: [(&str, u32, u32, f64); 9] = [
    ("720p59.94", 1280, 720, 59.94),
    ("1080p30", 1920, 1080, 30.),
    ("1080p50", 1920, 1080, 50.),
    ("1080p59.94", 1920, 1080, 59.94),
    ("1080p60", 1920, 1080, 60.),
    ("4Kp25", 3840, 2160, 25.),
    ("4Kp30", 3840, 2160, 30.),
    ("4Kp50", 3840, 2160, 50.),
    ("4Kp60", 3840, 2160, 60.),
];

/// `updateNdiResolution`: a preset also sets the size and frame rate; `custom` changes the mode only.
pub fn ndi_mode_patch(value: &str) -> Value {
    match NDI_PRESETS.iter().find(|(name, ..)| *name == value) {
        Some((_, width, height, fps)) => {
            json!({"ndiResolution": value, "ndiWidth": width, "ndiHeight": height, "ndiFramerate": fps})
        }
        None => json!({"ndiResolution": value}),
    }
}

/// A custom NDI size or frame rate: a finite number in range (the size is whole pixels).
pub fn custom_value(text: &str, min: f64, max: f64) -> Option<f64> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= min && *v <= max)
}

/// `getScreenLookThemes`.
pub fn look_themes(config: &Value) -> Map<String, Value> {
    let mut themes: Map<String, Value> = [("maskId", ""), ("presentationThemeId", "")]
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect();
    if let Some(saved) = config["lookThemes"].as_object() {
        themes.extend(saved.clone());
    }
    themes
}

/// The theme slide choices: `themeId::slideId` per slide, grouped by theme title in the label.
pub fn theme_slide_options(themes: &Value) -> Vec<(String, String)> {
    let mut options = Vec::new();
    for theme in themes.as_array().into_iter().flatten() {
        let title = theme["title"].as_str().unwrap_or("");
        let id = theme["id"].as_str().unwrap_or("");
        let slides: Vec<Value> = match theme["slides"].as_array().filter(|s| !s.is_empty()) {
            Some(slides) => slides.clone(),
            None => vec![json!({"id": "", "name": title})],
        };
        for (index, slide) in slides.iter().enumerate() {
            let slide_id = slide["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("slide-1");
            let name = slide["name"]
                .as_str()
                .or_else(|| slide["title"].as_str())
                .map_or_else(|| format!("{}", index + 1), str::to_string);
            options.push((format!("{id}::{slide_id}"), format!("{title}: {name}")));
        }
    }
    options
}

pub fn effective_stage_layout<'a>(
    config: &Value,
    workspace: &'a Value,
    screen_id: &str,
) -> Option<&'a Value> {
    orbit_host::resolve_stage_layout(
        config["stageLayoutId"].as_str(),
        screen_id,
        workspace["stageScreenAssignments"]
            .as_array()
            .map_or(&[], Vec::as_slice),
        workspace["stageLayouts"]
            .as_array()
            .map_or(&[], Vec::as_slice),
    )
}

impl SettingsView {
    pub(super) fn video_extra_fields(
        &mut self,
        id: &str,
        config: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let workspace = self.runtime.content.read(cx).workspace.clone();
        let mut body = stack(cx);
        if config["targetType"] == "ndi" {
            let current = config["ndiResolution"]
                .as_str()
                .unwrap_or("1080p60")
                .to_string();
            let weak = cx.weak_entity();
            let screen = id.to_string();
            let mut options: Vec<SelectOption<String>> = NDI_PRESETS
                .iter()
                .map(|(n, ..)| SelectOption::new(n.to_string(), n.to_string()))
                .collect();
            options.push(SelectOption::new(
                "custom".to_string(),
                "Custom Size...".to_string(),
            ));
            body = body.child(Field::new(
                t.t("screenSettings.ndiMode"),
                Select::new(
                    SharedString::from(format!("screen-ndi-mode-{id}")),
                    options,
                    current.clone(),
                )
                .on_change(move |value, _, cx| {
                    let screen = screen.clone();
                    weak.update(cx, |view, cx| {
                        view.update_screen(&screen, ndi_mode_patch(&value), cx)
                    })
                    .ok();
                }),
            ));
        }
        if config["targetType"] == "ndi" && config["ndiResolution"] == "custom" {
            for (key, label, min, max) in [
                ("ndiWidth", "capture.width", 16., 8192.),
                ("ndiHeight", "capture.height", 16., 8192.),
                ("ndiFramerate", "capture.fps", 1., 120.),
            ] {
                let screen = id.to_string();
                body = body.child(self.number_field(
                    &format!("screen-{key}-{id}"),
                    t.t(label),
                    &config[key].as_f64().unwrap_or(0.).to_string(),
                    (min, max),
                    1.,
                    window,
                    cx,
                    move |view, text, cx| {
                        if let Some(n) = custom_value(&text, min, max) {
                            view.update_screen(&screen, json!({ key: n }), cx);
                        }
                    },
                ));
            }
        }
        if let Some(selected) = effective_stage_layout(config, &workspace, id)
            .filter(|_| config["type"] == "stage" && config["canvasView"].is_null())
            .and_then(|layout| layout["id"].as_str())
        {
            let weak = cx.weak_entity();
            let screen = id.to_string();
            let options = workspace["stageLayouts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|layout| layout["slide"].is_object())
                .filter_map(|layout| {
                    let id = layout["id"].as_str().filter(|id| !id.is_empty())?;
                    Some(SelectOption::new(
                        id.to_string(),
                        layout["title"].as_str().unwrap_or("").to_string(),
                    ))
                })
                .collect();
            body = body.child(Field::new(
                t.t("stage.theme"),
                Select::new(
                    SharedString::from(format!("screen-stage-layout-{id}")),
                    options,
                    selected.to_string(),
                )
                .on_change(move |value, _, cx| {
                    let screen = screen.clone();
                    weak.update(cx, |view, cx| {
                        view.update_screen(&screen, json!({"stageLayoutId": value}), cx)
                    })
                    .ok();
                }),
            ));
        }
        body.into_any_element()
    }
}
