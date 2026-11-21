use super::document::{CanvasDocument, ThemeDocument};
use crate::state::runtime::Runtime;
use gpui::{App, AppContext, Task};
use serde_json::{json, Value};
use std::{rc::Rc, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DocumentKind {
    #[default]
    Theme,
    Slide,
    Prop,
    Mask,
    Stage,
    Ccli,
}
impl DocumentKind {
    pub fn collection(self) -> &'static str {
        match self {
            Self::Theme => "themes",
            Self::Slide => "library",
            Self::Prop => "props",
            Self::Mask => "masks",
            Self::Stage => "stageLayouts",
            Self::Ccli => "ccliTemplate",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "appMenu.themeEditor",
            Self::Slide => "appMenu.editSelectedSlide",
            Self::Prop => "appMenu.propsEditor",
            Self::Mask => "mask.editor",
            Self::Stage => "appMenu.stageEditor",
            Self::Ccli => "toolbar.editor.ccli",
        }
    }
}
pub fn open_document(
    runtime: &Runtime,
    kind: DocumentKind,
    owner: &str,
    slide: &str,
    cx: &App,
) -> Result<Rc<dyn CanvasDocument>, String> {
    if kind == DocumentKind::Theme {
        ThemeDocument::open(runtime, owner, slide, cx)
    } else {
        ContentDocument::open(runtime, kind, owner, slide, cx)
    }
}
struct ContentDocument {
    runtime: Runtime,
    workspace: String,
    kind: DocumentKind,
    source: Arc<Value>,
    slide: Arc<Value>,
    theme: Option<Value>,
}
impl ContentDocument {
    fn open(
        runtime: &Runtime,
        kind: DocumentKind,
        owner: &str,
        slide_id: &str,
        cx: &App,
    ) -> Result<Rc<dyn CanvasDocument>, String> {
        let content = runtime.content.read(cx);
        let source = if kind == DocumentKind::Slide {
            content.items.get(owner).map(|item| item.value.clone())
        } else if kind == DocumentKind::Ccli {
            Some(Arc::new(ccli_slide(&content.workspace)))
        } else {
            content.workspace[kind.collection()]
                .as_array()
                .and_then(|values| values.iter().find(|value| value["id"] == owner))
                .map(|value| Arc::new(value.clone()))
        }
        .ok_or("The authoring document is unavailable.")?;
        let raw = if kind == DocumentKind::Slide {
            source["slides"]
                .as_array()
                .and_then(|slides| slides.iter().find(|slide| slide["id"] == slide_id))
                .ok_or("The authoring slide is unavailable.")?
        } else if kind == DocumentKind::Ccli {
            &source
        } else {
            source
                .get("slide")
                .ok_or("The authoring document has no slide.")?
        };
        let theme = if kind == DocumentKind::Slide {
            orbit_host::authoring::slide_theme(&content.workspace, raw)
        } else {
            None
        };
        let mut slide = if kind == DocumentKind::Slide {
            orbit_host::authoring::compose(raw, theme.clone())?
        } else {
            raw.clone()
        };
        // The importer omits the layer list for empty prop and mask slides.
        if matches!(kind, DocumentKind::Prop | DocumentKind::Mask)
            && slide.is_object()
            && slide.get("layers").is_none()
        {
            slide["layers"] = json!([]);
        }
        super::document::validate_layer_shapes(&slide)?;
        if kind == DocumentKind::Mask {
            slide["background"] = json!("transparent");
            slide["backgroundEnabled"] = json!(false);
            if let Some(size) = source.get("size") {
                slide["width"] = size["width"].clone();
                slide["height"] = size["height"].clone();
            }
        }
        Ok(Rc::new(Self {
            runtime: runtime.clone(),
            workspace: content.workspace_path.clone(),
            kind,
            source,
            slide: Arc::new(slide),
            theme,
        }))
    }
}
impl CanvasDocument for ContentDocument {
    fn key(&self) -> String {
        format!(
            "{}:{:?}:{}:{}",
            self.workspace, self.kind, self.source["id"], self.slide["id"]
        )
    }
    fn slide(&self) -> Arc<Value> {
        self.slide.clone()
    }
    fn commit(
        &self,
        mut edited: Value,
        cx: &mut App,
    ) -> Task<Result<Rc<dyn CanvasDocument>, String>> {
        let runtime = self.runtime.clone();
        let path = self.workspace.clone();
        let scope = path.clone();
        let kind = self.kind;
        let expected = self.source.clone();
        let theme = self.theme.clone();
        let mut stored_theme = theme.clone();
        if let Some(theme) = &mut stored_theme {
            crate::state::text_files::overlay(theme, &runtime.text_files.read(cx).entries);
        }
        let (Some(owner), Some(slide_id)) = (
            self.source["id"].as_str().map(str::to_string),
            self.slide["id"].as_str().map(str::to_string),
        ) else {
            return Task::ready(Err("The authoring document has no identity.".into()));
        };
        let id = owner.clone();
        let sid = slide_id.clone();
        let host = runtime.host.clone();
        let revisions = runtime.lanes.revisions().clone();
        crate::state::text_files::overlay(&mut edited, &runtime.text_files.read(cx).entries);
        let answer=runtime.lanes.workspace(&path,move|revision|{
            let current=host.invoke("readWorkspaceSnapshot",vec![json!(scope)])?;
            let ccli;
            let source = if kind == DocumentKind::Ccli {
                ccli = ccli_slide(&current["workspace"]);
                &ccli
            } else {
                current["workspace"][kind.collection()].as_array().and_then(|documents|documents.iter().find(|document|document["id"]==id)).ok_or("The authoring document was removed.")?
            };
            if kind != DocumentKind::Slide && source!=expected.as_ref(){return Err("The authoring document changed during this edit.".into());}
            if kind==DocumentKind::Slide {
                let raw=source["slides"].as_array().and_then(|slides|slides.iter().find(|slide|slide["id"]==sid)).ok_or("The authoring slide was removed.")?;
                let before=expected["slides"].as_array().and_then(|slides|slides.iter().find(|slide|slide["id"]==sid));
                if Some(raw)!=before{return Err("The authoring slide changed during this edit.".into());}
                if orbit_host::authoring::slide_theme(&current["workspace"],raw)!=theme {return Err("The assigned theme changed during this edit.".into());}
            }
            let (method,mutation)=match kind {
                DocumentKind::Slide=>{
                    let mut slides=source["slides"].as_array().ok_or("The authoring document has no slides.")?.clone();let target=slides.iter_mut().find(|slide|slide["id"]==sid).ok_or("The authoring slide was removed.")?;
                    *target=orbit_host::authoring::store(target,&edited,theme.as_ref())?;
                    ("mutatePresentationLibrary",json!({"type":"mutateItem","itemId":id,"operation":{"type":"setSlideDocument","slides":slides,"title":source["title"]}}))
                },
                DocumentKind::Prop=>("mutateProps",json!({"type":"updateDocument","propId":id,"slide":edited})),
                DocumentKind::Mask=>("mutateMasks",json!({"type":"updateDocument","maskId":id,"size":{"width":edited["width"],"height":edited["height"]},"slide":edited})),
                DocumentKind::Stage=>{let mut layout=source.clone();layout["slide"]=edited;("mutateStageContent",json!({"type":"replaceLayout","layoutId":id,"layout":layout}))},
                DocumentKind::Ccli => ("saveWorkspaceContent", json!({"ccliTemplate": edited})),
                DocumentKind::Theme=>unreachable!(),
            };
            let saved=host.invoke(method,vec![json!(scope),mutation,json!(revision)])?;revisions.remember(&scope,&saved);
            if kind == DocumentKind::Slide { Ok(saved) } else { host.invoke("readWorkspaceSnapshot",vec![json!(scope)]) }
        });
        cx.spawn(async move |cx| {
            let result = answer
                .await
                .map_err(|_| "The authoring save worker stopped.".to_string())??;
            let snapshot = cx
                .background_spawn(async move {
                    crate::collections::model::Snapshot::parse(&result["workspace"])
                })
                .await?;
            cx.update(|cx| {
                if runtime.content.read(cx).workspace_path != path {
                    return Err("The authoring workspace changed while saving.".into());
                }
                runtime.content.update(cx, |content, cx| {
                    content.apply_document(snapshot);
                    cx.notify();
                });
                if kind != DocumentKind::Slide {
                    crate::cues::refresh_live(&runtime, cx);
                }
                open_document(&runtime, kind, &owner, &slide_id, cx)
            })
        })
    }
}

fn ccli_slide(workspace: &Value) -> Value {
    workspace
        .get("ccliTemplate")
        .cloned()
        .unwrap_or_else(orbit_host::ccli_template::default_slide)
}
