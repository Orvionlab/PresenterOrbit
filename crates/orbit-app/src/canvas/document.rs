use crate::state::runtime::Runtime;
use gpui::{App, AppContext, Task};
use serde_json::{json, Value};
use std::rc::Rc;
use std::sync::Arc;

pub trait CanvasDocument {
    fn key(&self) -> String;
    fn slide(&self) -> Arc<Value>;
    fn commit(&self, slide: Value, cx: &mut App) -> Task<Result<Rc<dyn CanvasDocument>, String>>;
}

/// The inspector writes into these fields by key, which only works when each is absent or an object.
const OBJECT_FIELDS: [&str; 5] = [
    "build",
    "buildOut",
    "textReveal",
    "textStyle",
    "captureStatusColors",
];

/// Rejects a slide whose layer list or layer fields the editing code could not write into.
pub(super) fn validate_layer_shapes(slide: &Value) -> Result<(), String> {
    let absent_or_object = |value: &Value| value.is_null() || value.is_object();
    if !absent_or_object(&slide["textStyle"])
        || !absent_or_object(&slide["transition"])
        || !absent_or_object(&slide["transition"]["variables"])
    {
        return Err("The slide text style and transition must be objects.".into());
    }
    for layer in slide["layers"]
        .as_array()
        .ok_or("The slide has no layer list.")?
    {
        if !layer.is_object() {
            return Err("Canvas layers must be objects.".into());
        }
        for field in OBJECT_FIELDS {
            if !absent_or_object(&layer[field]) {
                return Err(format!("The layer field '{field}' must be an object."));
            }
        }
        for field in ["build", "buildOut"] {
            let transition = &layer[field]["transition"];
            if !absent_or_object(transition) || !absent_or_object(&transition["variables"]) {
                return Err(format!("The layer {field} transition must be an object."));
            }
        }
    }
    Ok(())
}

pub struct ThemeDocument {
    runtime: Runtime,
    workspace: String,
    theme: Arc<Value>,
    slide: Arc<Value>,
}
impl ThemeDocument {
    pub fn open(
        runtime: &Runtime,
        theme_id: &str,
        slide_id: &str,
        cx: &App,
    ) -> Result<Rc<dyn CanvasDocument>, String> {
        let content = runtime.content.read(cx);
        let theme = content.workspace["themes"]
            .as_array()
            .and_then(|themes| themes.iter().find(|theme| theme["id"] == theme_id))
            .ok_or("The selected theme is unavailable.")?;
        let slide = theme["slides"]
            .as_array()
            .and_then(|slides| slides.iter().find(|slide| slide["id"] == slide_id))
            .ok_or("The selected theme slide is unavailable.")?;
        validate_layer_shapes(slide)?;
        for layer in slide["layers"].as_array().into_iter().flatten() {
            if !layer.is_object() || layer["id"].as_str().is_none_or(str::is_empty) {
                return Err("Canvas layers must have an identity.".into());
            }
            if let Some(points) = layer.get("pathPoints").filter(|points| !points.is_null()) {
                if !points.as_array().is_some_and(|points| {
                    points.iter().all(|point| {
                        point.is_object()
                            && ["q0", "q1"].into_iter().all(|key| {
                                point
                                    .get(key)
                                    .is_none_or(|control| control.is_null() || control.is_object())
                            })
                    })
                }) {
                    return Err("Canvas path points and controls must be objects.".into());
                }
            }
        }
        Ok(Rc::new(Self {
            runtime: runtime.clone(),
            workspace: content.workspace_path.clone(),
            theme: Arc::new(theme.clone()),
            slide: Arc::new(slide.clone()),
        }))
    }
}
impl CanvasDocument for ThemeDocument {
    fn key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.workspace, self.theme["id"], self.slide["id"]
        )
    }
    fn slide(&self) -> Arc<Value> {
        self.slide.clone()
    }
    fn commit(&self, slide: Value, cx: &mut App) -> Task<Result<Rc<dyn CanvasDocument>, String>> {
        let runtime = self.runtime.clone();
        let expected = self.theme.clone();
        let Some(slide_id) = self.slide["id"].as_str().map(str::to_string) else {
            return Task::ready(Err("The theme slide has no identity.".into()));
        };
        let mut theme = (*self.theme).clone();
        let Some(target) = theme["slides"].as_array_mut().and_then(|slides| {
            slides
                .iter_mut()
                .find(|candidate| candidate["id"] == slide_id)
        }) else {
            return Task::ready(Err("The theme slide is unavailable.".into()));
        };
        let size_changed = target["width"] != slide["width"] || target["height"] != slide["height"];
        let next_size = json!({"width":slide["width"],"height":slide["height"]});
        *target = slide;
        if size_changed {
            theme["size"] = next_size;
        }
        Self::replace_document(&runtime, Some(expected), theme, &slide_id, true, cx)
    }
}
impl ThemeDocument {
    pub fn replace(
        runtime: &Runtime,
        expected: Option<Arc<Value>>,
        theme: Value,
        slide_id: &str,
        cx: &mut App,
    ) -> Task<Result<Rc<dyn CanvasDocument>, String>> {
        Self::replace_document(runtime, expected, theme, slide_id, false, cx)
    }

    fn replace_document(
        runtime: &Runtime,
        expected: Option<Arc<Value>>,
        mut theme: Value,
        slide_id: &str,
        single_slide: bool,
        cx: &mut App,
    ) -> Task<Result<Rc<dyn CanvasDocument>, String>> {
        let runtime = runtime.clone();
        let workspace = runtime.content.read(cx).workspace_path.clone();
        crate::state::text_files::overlay(&mut theme, &runtime.text_files.read(cx).entries);
        let Some(theme_id) = theme["id"].as_str().map(str::to_string) else {
            return Task::ready(Err("The theme has no identity.".into()));
        };
        let slide_id = slide_id.to_string();
        let host = runtime.host.clone();
        let path = workspace.clone();
        let revisions = runtime.lanes.revisions().clone();
        let requested_theme = theme_id.clone();
        let requested_slide = slide_id.clone();
        let answer = runtime.lanes.workspace(&workspace, move |revision| {
            let current = host.invoke("readWorkspaceSnapshot", vec![json!(path)])?;
            let current_theme = current["workspace"]["themes"]
                .as_array()
                .and_then(|themes| themes.iter().find(|theme| theme["id"] == requested_theme));
            if single_slide {
                let current_theme = current_theme.ok_or("The theme was removed.")?;
                let expected = expected
                    .as_deref()
                    .ok_or("The original theme is required.")?;
                let find = |theme: &Value| {
                    theme["slides"]
                        .as_array()
                        .and_then(|slides| {
                            slides.iter().find(|slide| slide["id"] == requested_slide)
                        })
                        .cloned()
                };
                let mut current_properties = current_theme.clone();
                let mut expected_properties = expected.clone();
                current_properties
                    .as_object_mut()
                    .ok_or("The theme must be an object.")?
                    .remove("slides");
                expected_properties
                    .as_object_mut()
                    .ok_or("The theme must be an object.")?
                    .remove("slides");
                if current_properties != expected_properties
                    || find(current_theme) != find(expected)
                {
                    return Err("The theme changed during this canvas edit.".into());
                }
                let edited = find(&theme).ok_or("The edited theme slide is unavailable.")?;
                let size = theme["size"].clone();
                let size_changed = size != expected["size"];
                theme = current_theme.clone();
                let target = theme["slides"]
                    .as_array_mut()
                    .and_then(|slides| {
                        slides
                            .iter_mut()
                            .find(|slide| slide["id"] == requested_slide)
                    })
                    .ok_or("The theme slide was removed.")?;
                *target = edited;
                if size_changed {
                    theme["size"] = size;
                }
            } else if current_theme != expected.as_deref() {
                return Err("The theme changed during this canvas edit.".into());
            }
            let saved = host.invoke("saveTheme", vec![json!(path), theme, json!(revision)])?;
            revisions.remember(&path, &saved);
            host.invoke("readWorkspaceSnapshot", vec![json!(path)])
        });
        cx.spawn(async move |cx| {
            let response = answer
                .await
                .map_err(|_| "The canvas save worker stopped.".to_string())??;
            let snapshot = cx
                .background_spawn(async move {
                    crate::collections::model::Snapshot::parse(&response["workspace"])
                })
                .await?;
            cx.update(|cx| {
                if runtime.content.read(cx).workspace_path != workspace {
                    return Err("The canvas workspace changed while saving.".into());
                }
                runtime.content.update(cx, |content, cx| {
                    content.apply_document(snapshot);
                    cx.notify();
                });
                crate::cues::refresh_live(&runtime, cx);
                Self::open(&runtime, &theme_id, &slide_id, cx)
            })
        })
    }
}
