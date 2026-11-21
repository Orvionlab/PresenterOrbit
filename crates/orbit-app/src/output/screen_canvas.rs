//! Screen canvas settings and the native output preview. Geometry follows screen-canvas.ts.
use crate::state::runtime::Runtime;
use gpui::{
    div, prelude::*, px, Context, Entity, EventEmitter, Render, SharedString, Subscription, Window,
};
use orbit_ui::{
    controls::{
        input_state, ActionButton, ButtonVariant, Dialog, EmptyState, Field, InputEvent,
        InputState, NumberInput, Select, SelectOption,
    },
    ActiveTheme, Translator,
};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanvasDraft {
    pub mode: String,
    pub source: String,
    pub width: String,
    pub height: String,
    pub x: String,
    pub y: String,
}
fn text(value: &Value) -> String {
    if value.is_null() {
        String::new()
    } else {
        value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string())
    }
}
impl CanvasDraft {
    pub fn from_config(config: &Value) -> Self {
        let view = &config["canvasView"];
        let region = &view["region"];
        let size = if region.is_object() {
            region
        } else {
            &config["canvasSize"]
        };
        Self {
            mode: if view.is_object() {
                if region.is_object() {
                    "tile"
                } else {
                    "mirror"
                }
            } else if config["canvasSize"].is_object() {
                "canvas"
            } else {
                "independent"
            }
            .into(),
            source: view["sourceScreenId"].as_str().unwrap_or("").into(),
            width: text(&size["width"]),
            height: text(&size["height"]),
            x: if region["x"].is_null() {
                "0".into()
            } else {
                text(&region["x"])
            },
            y: if region["y"].is_null() {
                "0".into()
            } else {
                text(&region["y"])
            },
        }
    }
    pub fn patch(&self) -> Value {
        let number = |value: &str| {
            value
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(|value| json!(value))
                .unwrap_or(Value::Null)
        };
        match self.mode.as_str() {
            "independent" => json!({"canvasSize":null,"canvasView":null}),
            "canvas" => {
                json!({"canvasSize":{"width":number(&self.width),"height":number(&self.height)},"canvasView":null})
            }
            "mirror" => json!({"canvasSize":null,"canvasView":{"sourceScreenId":self.source}}),
            "tile" => {
                json!({"canvasSize":null,"canvasView":{"sourceScreenId":self.source,"region":{"width":number(&self.width),"height":number(&self.height),"x":number(&self.x),"y":number(&self.y)}}})
            }
            _ => json!({"canvasSize":{"width":null,"height":null},"canvasView":null}),
        }
    }
}
fn valid_size(size: &Value) -> bool {
    let (width, height) = (size["width"].as_f64(), size["height"].as_f64());
    match (width, height) {
        (Some(w), Some(h)) => {
            [w, h]
                .iter()
                .all(|value| value.fract() == 0. && *value > 0. && *value <= 8192.)
                && w * h <= 33_554_432.
        }
        _ => false,
    }
}
pub fn canvas_error(configs: &Value) -> Option<&'static str> {
    for (id, config) in configs.as_object()? {
        if !config["canvasSize"].is_null()
            && (!valid_size(&config["canvasSize"]) || !config["canvasView"].is_null())
        {
            return Some("screenSettings.canvas.invalidSize");
        }
        let view = &config["canvasView"];
        if view.is_null() {
            continue;
        }
        let Some(source_id) = view["sourceScreenId"].as_str() else {
            return Some("screenSettings.canvas.invalidSource");
        };
        let Some(source) = configs.get(source_id) else {
            return Some("screenSettings.canvas.invalidSource");
        };
        if source_id == id || !source["canvasView"].is_null() || source["type"] != config["type"] {
            return Some("screenSettings.canvas.invalidSource");
        }
        let region = &view["region"];
        if region.is_null() {
            continue;
        }
        let size = &source["canvasSize"];
        let (x, y) = (region["x"].as_f64(), region["y"].as_f64());
        if size.is_null()
            || !valid_size(region)
            || [x, y]
                .iter()
                .any(|value| value.is_none_or(|value| value.fract() != 0. || value < 0.))
            || x.unwrap() + region["width"].as_f64().unwrap() > size["width"].as_f64().unwrap_or(0.)
            || y.unwrap() + region["height"].as_f64().unwrap()
                > size["height"].as_f64().unwrap_or(0.)
        {
            return Some("screenSettings.canvas.invalidRegion");
        }
    }
    None
}
pub struct ScreenCanvas {
    runtime: Runtime,
    selected: String,
    draft: CanvasDraft,
    original: Value,
    inputs: HashMap<&'static str, Entity<InputState>>,
    subscriptions: Vec<Subscription>,
    _runtime_subscriptions: Vec<Subscription>,
    preview: Entity<super::preview::Preview>,
    busy: bool,
    error: String,
    path: String,
}
pub struct Closed;
impl EventEmitter<Closed> for ScreenCanvas {}
impl ScreenCanvas {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let preview = cx.new(|cx| {
            super::preview::Preview::new(runtime.host.clone(), runtime.frames.clone(), cx)
        });
        let subscriptions = vec![
            cx.observe(&runtime.content, |_, _, cx| cx.notify()),
            cx.observe(&runtime.output, |_, _, cx| cx.notify()),
        ];
        Self {
            runtime,
            _runtime_subscriptions: subscriptions,
            selected: String::new(),
            draft: CanvasDraft::default(),
            original: Value::Null,
            inputs: HashMap::new(),
            subscriptions: Vec::new(),
            preview,
            busy: false,
            error: String::new(),
            path: String::new(),
        }
    }
    fn configs(&self, cx: &gpui::App) -> Value {
        let output = self.runtime.output.read(cx);
        let mut configs = output.settings["screenConfig"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        if let Some(local) = self.runtime.content.read(cx).workspace["screenConfig"].as_object() {
            configs.extend(local.clone());
        }
        Value::Object(configs)
    }
    fn load(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let configs = self.configs(cx);
        let Some(config) = configs.get(&id) else {
            self.error = "screenSettings.canvas.invalidSource".into();
            cx.notify();
            return;
        };
        self.selected = id;
        self.original = config.clone();
        self.draft = CanvasDraft::from_config(config);
        self.inputs.clear();
        self.subscriptions.clear();
        self.error.clear();
        for key in ["width", "height", "x", "y"] {
            let value = match key {
                "width" => &self.draft.width,
                "height" => &self.draft.height,
                "x" => &self.draft.x,
                _ => &self.draft.y,
            };
            let input = input_state(value.clone(), window, cx);
            self.subscriptions.push(cx.subscribe(
                &input,
                move |view, state, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = state.read(cx).value().to_string();
                        match key {
                            "width" => view.draft.width = value,
                            "height" => view.draft.height = value,
                            "x" => view.draft.x = value,
                            _ => view.draft.y = value,
                        };
                        view.error.clear();
                        cx.notify();
                    }
                },
            ));
            self.inputs.insert(key, input);
        }
        self.path = self.runtime.content.read(cx).workspace_path.clone();
        cx.notify();
    }
    fn proposed(&self, cx: &gpui::App) -> Value {
        let mut configs = self.configs(cx);
        if let Some(config) = configs.get_mut(&self.selected) {
            config
                .as_object_mut()
                .unwrap()
                .extend(self.draft.patch().as_object().unwrap().clone());
        }
        configs
    }
    fn apply(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some(error) = canvas_error(&self.proposed(cx)) {
            self.error = error.into();
            cx.notify();
            return;
        }
        if self.runtime.content.read(cx).workspace_path.is_empty() {
            self.error = "transition.workspaceRequired".into();
            cx.notify();
            return;
        }
        let runtime = self.runtime.clone();
        let path = runtime.content.read(cx).workspace_path.clone();
        let scope = path.clone();
        let host = runtime.host.clone();
        let revisions = runtime.lanes.revisions().clone();
        let screen = self.selected.clone();
        let patch = self.draft.patch();
        let expected = self.original.clone();
        let answer=runtime.lanes.workspace(&path,move|revision|{
            let current=host.invoke("getSettings",vec![])?;let workspace=host.invoke("readWorkspaceSnapshot",vec![json!(scope)])?;
            let actual=workspace["workspace"]["screenConfig"].get(&screen).or_else(||current["screenConfig"].get(&screen)).ok_or("The screen was removed.")?;
            if actual!=&expected{return Err("The screen configuration changed while editing.".into());}
            let result=host.invoke("mutateScreenConfig",vec![json!(scope),json!({"type":"update","screenId":screen,"patch":patch}),json!(revisions.settings()),json!(revision)])?;
            revisions.remember(&scope,&result);revisions.remember_settings(&result);
            Ok(json!({"snapshot":host.invoke("readWorkspaceSnapshot",vec![json!(scope)])?,"settings":host.invoke("getSettings",vec![])?,"runtimeError":result["runtimeError"]}))
        });
        self.busy = true;
        self.error.clear();
        cx.spawn(async move |view, cx| {
            let result = answer
                .await
                .map_err(|_| "The screen canvas worker stopped.".to_string())
                .and_then(|result| result);
            let result = cx
                .background_spawn(async move {
                    let result = result?;
                    Ok::<_, String>((
                        crate::collections::model::Snapshot::parse(
                            &result["snapshot"]["workspace"],
                        )?,
                        result["settings"].clone(),
                        result["runtimeError"].as_str().unwrap_or("").to_string(),
                    ))
                })
                .await;
            view.update(cx, |view, cx| {
                view.busy = false;
                if runtime.content.read(cx).workspace_path != path {
                    cx.notify();
                    return;
                }
                match result {
                    Ok((snapshot, settings, error)) => {
                        runtime.content.update(cx, |content, cx| {
                            content.load(snapshot);
                            cx.notify();
                        });
                        runtime.output.update(cx, |output, cx| {
                            output.set_settings(settings);
                            cx.notify();
                        });
                        view.inputs.clear();
                        view.error = error;
                    }
                    Err(error) => view.error = error,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}
impl Render for ScreenCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let configs = self.configs(cx);
        if self.path != self.runtime.content.read(cx).workspace_path {
            self.selected.clear();
            self.inputs.clear();
        }
        if self.selected.is_empty() {
            if let Some((id, _)) = configs
                .as_object()
                .and_then(|configs| configs.iter().next())
            {
                self.load(id.clone(), window, cx);
            }
        } else if self.inputs.is_empty() && !self.busy {
            self.load(self.selected.clone(), window, cx);
        }
        let weak = cx.weak_entity();
        let mut dialog = Dialog::new(
            "screen-canvas-dialog",
            t.t("screenSettings.canvas.title"),
            t.t("common.close"),
            move |_, window, cx| {
                weak.update(cx, |view, cx| {
                    if !view.busy {
                        view.preview
                            .update(cx, |preview, cx| preview.suspend(window, cx));
                        cx.emit(Closed);
                    }
                })
                .ok();
            },
        )
        .width(theme.metrics.control_height * 24.);
        let options = configs
            .as_object()
            .into_iter()
            .flatten()
            .map(|(id, config)| {
                SelectOption::new(
                    id.clone(),
                    config["label"].as_str().unwrap_or(id).to_string(),
                )
            })
            .collect();
        let weak = cx.weak_entity();
        dialog = dialog.child(
            Select::new("canvas-screen", options, self.selected.clone())
                .label(t.t("screenSettings.canvas.source"))
                .disabled(self.busy)
                .on_change(move |id, window, cx| {
                    weak.update(cx, |view, cx| view.load(id, window, cx)).ok();
                }),
        );
        if self.selected.is_empty() {
            return dialog.child(EmptyState::new(t.t("screenSettings.canvas.chooseSource")));
        }
        let weak = cx.weak_entity();
        dialog = dialog.child(
            Select::new(
                "screen-canvas-mode",
                ["independent", "canvas", "mirror", "tile"]
                    .iter()
                    .map(|mode| {
                        SelectOption::new(
                            mode.to_string(),
                            t.t(&format!("screenSettings.canvas.{mode}")),
                        )
                    })
                    .collect(),
                self.draft.mode.clone(),
            )
            .label(t.t("screenSettings.canvas.mode"))
            .disabled(self.busy)
            .on_change(move |mode, _, cx| {
                weak.update(cx, |view, cx| {
                    view.draft.mode = mode;
                    view.error.clear();
                    cx.notify();
                })
                .ok();
            }),
        );
        if matches!(self.draft.mode.as_str(), "mirror" | "tile") {
            let mut sources = vec![SelectOption::new(
                String::new(),
                t.t("screenSettings.canvas.chooseSource"),
            )];
            sources.extend(
                configs
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(id, config)| {
                        *id != &self.selected
                            && config["canvasView"].is_null()
                            && config["type"] == configs[&self.selected]["type"]
                    })
                    .map(|(id, config)| {
                        SelectOption::new(
                            id.clone(),
                            config["label"].as_str().unwrap_or(id).to_string(),
                        )
                    }),
            );
            let weak = cx.weak_entity();
            dialog = dialog.child(
                Select::new("canvas-source-screen", sources, self.draft.source.clone())
                    .label(t.t("screenSettings.canvas.source"))
                    .disabled(self.busy)
                    .on_change(move |source, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.draft.source = source;
                            view.error.clear();
                            cx.notify();
                        })
                        .ok();
                    }),
            );
        }
        if matches!(self.draft.mode.as_str(), "canvas" | "tile") {
            let mut fields = div().flex().gap(theme.metrics.space_sm);
            for key in ["x", "y", "width", "height"] {
                if self.draft.mode == "canvas" && matches!(key, "x" | "y") {
                    continue;
                }
                if let Some(input) = self.inputs.get(key) {
                    fields = fields.child(Field::new(
                        t.t(&format!("screenSettings.canvas.{key}")),
                        NumberInput::new(SharedString::from(format!("canvas-{key}")), input)
                            .range(if key == "x" || key == "y" { 0. } else { 1. }, 8192.)
                            .step(1.)
                            .disabled(self.busy),
                    ));
                }
            }
            dialog = dialog.child(fields);
        }
        let proposed = self.proposed(cx);
        let error = canvas_error(&proposed);
        if error.is_none() {
            let patch = self.draft.patch();
            let size = if self.draft.mode == "canvas" {
                &patch["canvasSize"]
            } else {
                &configs[&self.draft.source]["canvasSize"]
            };
            if let (Some(w), Some(h)) = (size["width"].as_f64(), size["height"].as_f64()) {
                let scale = (420. / w).min(120. / h) as f32;
                let mut diagram = div()
                    .relative()
                    .w(px(w as f32 * scale))
                    .h(px(h as f32 * scale))
                    .border_1()
                    .border_color(theme.colors.border_strong)
                    .bg(theme.colors.panel_bg);
                let r = &patch["canvasView"]["region"];
                // A field the operator is still typing parses to null, so the region is drawn once all four are numbers.
                if let (Some(x), Some(y), Some(width), Some(height)) = (
                    r["x"].as_f64(),
                    r["y"].as_f64(),
                    r["width"].as_f64(),
                    r["height"].as_f64(),
                ) {
                    diagram = diagram.child(
                        div()
                            .absolute()
                            .left(px(x as f32 * scale))
                            .top(px(y as f32 * scale))
                            .w(px(width as f32 * scale))
                            .h(px(height as f32 * scale))
                            .bg(theme.colors.selected)
                            .border_1()
                            .border_color(theme.colors.accent),
                    );
                }
                dialog = dialog.child(diagram);
            }
        }
        let output = self.runtime.output.read(cx);
        let source_size = super::view::screen_size(&configs[&self.selected]);
        let revision = output.committed_revision;
        self.preview.update(cx, |preview, cx| {
            preview.set_source(
                Some(super::preview::Source::Output(self.selected.clone())),
                Some(source_size),
                revision,
                cx,
            )
        });
        dialog = dialog.child(
            div()
                .h(theme.metrics.row_height * 8.)
                .child(self.preview.clone()),
        );
        if let Some(error) = error {
            dialog = dialog.child(div().text_color(theme.colors.warning).child(t.t(error)));
        }
        if !self.error.is_empty() {
            dialog = dialog.child(div().text_color(theme.colors.warning).child(
                if self.error.starts_with("screenSettings.")
                    || self.error.starts_with("transition.")
                {
                    t.t(&self.error)
                } else {
                    self.error.clone()
                },
            ));
        }
        let weak = cx.weak_entity();
        dialog.footer(
            ActionButton::new("apply-screen-canvas", t.t("screenSettings.canvas.apply"))
                .variant(ButtonVariant::Primary)
                .disabled(self.busy || error.is_some())
                .on_click(move |_, _, cx| {
                    weak.update(cx, |view, cx| view.apply(cx)).ok();
                }),
        )
    }
}
