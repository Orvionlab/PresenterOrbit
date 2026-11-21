//! Chord charts remain slide metadata and use the normal slide-document mutation.
use crate::{
    automation::form::{Form, Kind, Spec},
    services::{call, choices, entries, text},
    state::runtime::Runtime,
};
use gpui::{div, prelude::*, Context, SharedString, Window};
use orbit_ui::{
    controls::{ActionButton, Dialog},
    ActiveTheme,
};
use serde_json::{json, Value};
pub struct Charts {
    runtime: Runtime,
    form: Form,
    original: Value,
    loaded_slide: String,
    undo: Option<Value>,
    busy: bool,
    status: String,
    close: std::rc::Rc<dyn Fn(&mut gpui::App)>,
}
fn form(this: &mut Charts) -> &mut Form {
    &mut this.form
}
pub fn mutation(item: &Value, slide: &str, chart: &Value) -> Result<Value, String> {
    if !chart.is_null() {
        let pages = chart["pagePaths"]
            .as_array()
            .ok_or("chordChart.invalidPage")?;
        let page = chart["page"].as_u64().ok_or("chordChart.invalidPage")?;
        if page >= pages.len() as u64 {
            return Err("chordChart.invalidPage".into());
        }
    }
    let mut slides = entries(&item["slides"]);
    let target = slides
        .iter_mut()
        .find(|v| v["id"] == slide)
        .ok_or("chordChart.stale")?;
    let object = target.as_object_mut().ok_or("chordChart.stale")?;
    object.remove("chordChartUrl");
    if chart.is_null() {
        object.remove("chordChart");
    } else {
        object.insert("chordChart".into(), chart.clone());
    }
    Ok(
        json!({"type":"mutateItem","itemId":item["id"],"operation":{"type":"setSlideDocument","title":item["title"],"slides":slides}}),
    )
}
impl Charts {
    pub fn new(
        runtime: Runtime,
        close: impl Fn(&mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut form = Form::default();
        form.reset(json!({"itemId":runtime.ui.read(cx).selected_library_item_id}));
        let close = std::rc::Rc::new(close);
        let closing = close.clone();
        let scope = runtime.content.read(cx).workspace_path.clone();
        cx.observe(&runtime.content, move |_, content, cx| {
            if content.read(cx).workspace_path != scope {
                closing(cx);
            }
        })
        .detach();
        Self {
            runtime,
            form,
            original: Value::Null,
            loaded_slide: String::new(),
            undo: None,
            busy: false,
            status: String::new(),
            close,
        }
    }
    fn run(&mut self, method: &'static str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let workspace = &self.runtime.content.read(cx).workspace;
        let item = entries(&workspace["library"])
            .into_iter()
            .find(|v| v["id"] == self.form.draft["itemId"]);
        let Some(item) = item else {
            self.status = "chordChart.stale".into();
            return;
        };
        let sid = text(&self.form.draft, "slideId");
        if !entries(&item["slides"]).iter().any(|v| v["id"] == sid) {
            self.status = "chordChart.stale".into();
            return;
        }
        let path = self.runtime.content.read(cx).workspace_path.clone();
        if path.is_empty() {
            self.status = "chordChart.stale".into();
            return;
        }
        if method == "load" {
            self.original = item.clone();
            self.loaded_slide = sid.clone();
            self.undo = None;
            self.form.set(
                "chart",
                entries(&item["slides"])
                    .iter()
                    .find(|v| v["id"] == sid)
                    .map(|v| v["chordChart"].clone())
                    .unwrap_or(Value::Null),
            );
            self.form.refresh();
            cx.notify();
            return;
        }
        let request = if method == "chooseChordChart" {
            None
        } else {
            if item != self.original || sid != self.loaded_slide {
                self.status = "chordChart.stale".into();
                return;
            }
            match mutation(
                &item,
                &sid,
                if method == "clear" {
                    &Value::Null
                } else if method == "undo" {
                    let Some(chart) = self.undo.as_ref() else {
                        return;
                    };
                    chart
                } else {
                    &self.form.draft["chart"]
                },
            ) {
                Ok(v) => Some(v),
                Err(e) => {
                    self.status = e;
                    return;
                }
            }
        };
        self.busy = true;
        let runtime = self.runtime.clone();
        let draft = self.form.draft.clone();
        cx.spawn(async move |this, cx| {
            let retired = std::rc::Rc::new(std::cell::Cell::new(false));
            let watching = retired.clone();
            let scope = path.clone();
            let _subscription = cx.update(|cx| {
                cx.observe(&runtime.content, move |content, cx| {
                    if content.read(cx).workspace_path != scope {
                        watching.set(true)
                    }
                })
            });
            let answer = if let Some(request) = request {
                let host = runtime.host.clone();
                let scope = path.clone();
                let expected = item.clone();
                let answer = runtime.lanes.workspace(&path, move |revision| {
                    let current = host.invoke("readWorkspaceSnapshot", vec![json!(scope)])?;
                    if !entries(&current["workspace"]["library"]).contains(&expected) {
                        return Err("chordChart.stale".into());
                    }
                    host.invoke(
                        "mutatePresentationLibrary",
                        vec![json!(scope), request, json!(revision)],
                    )
                });
                match answer.await {
                    Err(_) => Err("chordChart.saveFailed".into()),
                    Ok(Err(e)) => Err(e),
                    Ok(Ok(v)) => {
                        let workspace = v["workspace"].clone();
                        let snapshot = cx
                            .background_spawn(async move {
                                crate::collections::model::Snapshot::parse(&workspace)
                            })
                            .await;
                        match snapshot {
                            Err(e) => Err(e),
                            Ok(snapshot) => {
                                cx.update(|cx| {
                                    if !retired.get()
                                        && runtime.content.read(cx).workspace_path == path
                                    {
                                        runtime.content.update(cx, |content, cx| {
                                            content.apply_document(snapshot);
                                            cx.notify();
                                        });
                                    }
                                });
                                Ok(v)
                            }
                        }
                    }
                }
            } else {
                call(&runtime, cx, "chooseChordChart", vec![json!(path)], false).await
            };
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if retired.get() || this.form.draft != draft {
                    this.status = "chordChart.stale".into();
                } else {
                    match answer {
                        Err(e) => this.status = e,
                        Ok(v) => {
                            if v["canceled"] != true {
                                let previous = entries(&item["slides"])
                                    .iter()
                                    .find(|v| v["id"] == sid)
                                    .map(|v| v["chordChart"].clone())
                                    .unwrap_or(Value::Null);
                                this.original = item;
                                this.loaded_slide = sid;
                                if method != "chooseChordChart" {
                                    this.undo = if method == "undo" {
                                        None
                                    } else {
                                        Some(previous)
                                    };
                                }

                                if method == "chooseChordChart" {
                                    this.form.set("chart", v["chart"].clone());
                                    this.form.refresh();
                                } else {
                                    this.status = "chordChart.saved".into();
                                    this.original =
                                        entries(&runtime.content.read(cx).workspace["library"])
                                            .into_iter()
                                            .find(|v| v["id"] == draft["itemId"])
                                            .unwrap_or(Value::Null);
                                    this.form.set(
                                        "chart",
                                        entries(&this.original["slides"])
                                            .iter()
                                            .find(|v| v["id"] == this.loaded_slide)
                                            .map(|v| v["chordChart"].clone())
                                            .unwrap_or(Value::Null),
                                    );
                                    this.form.refresh();
                                }
                            }
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
impl Render for Charts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Runtime::translator(cx);
        let workspace = &self.runtime.content.read(cx).workspace;
        let item = entries(&workspace["library"])
            .into_iter()
            .find(|v| v["id"] == self.form.draft["itemId"])
            .unwrap_or(Value::Null);
        let mut body = div()
            .id("chart-body")
            .flex()
            .flex_col()
            .gap(cx.theme().metrics.space_sm)
            .max_h(window.viewport_size().height - cx.theme().metrics.row_height * 6.)
            .overflow_y_scroll();
        for spec in [
            Spec::new(
                "itemId",
                t.t("presentationExport.presentation"),
                Kind::Hook(
                    choices(&workspace["library"], "title"),
                    |f, _, v| {
                        f.set("itemId", json!(v));
                        f.set("slideId", Value::Null);
                        f.set("chart", Value::Null);
                    },
                    text(&self.form.draft, "itemId"),
                ),
            ),
            Spec::new(
                "slideId",
                t.t("chordChart.slide"),
                Kind::Choice(choices(&item["slides"], "id")),
            ),
            Spec::new(
                "chart.page",
                t.t("chordChart.page"),
                Kind::Choice(
                    entries(&self.form.draft["chart"]["pagePaths"])
                        .iter()
                        .enumerate()
                        .map(|(index, _)| (index.to_string(), (index + 1).to_string()))
                        .collect(),
                ),
            ),
        ] {
            body = body.child(self.form.field("chart", &spec, form, window, cx));
        }
        if let Some(page) = self.form.draft["chart"]["page"]
            .as_u64()
            .and_then(|i| self.form.draft["chart"]["pagePaths"][i as usize].as_str())
        {
            body = body.child(
                gpui::img(std::path::PathBuf::from(page))
                    .object_fit(gpui::ObjectFit::Contain)
                    .max_w_full()
                    .h(cx.theme().metrics.row_height * 8.),
            );
        }
        for (method, key) in [
            ("load", "common.refresh"),
            ("chooseChordChart", "chordChart.choose"),
            ("save", "common.save"),
            ("clear", "common.clear"),
            ("undo", "slideEditor.undo"),
        ] {
            body = body.child(
                ActionButton::new(SharedString::from(method), t.t(key))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.run(method, cx))),
            );
        }
        let close = self.close.clone();
        let busy = self.busy;
        Dialog::new(
            "charts",
            t.t("chordChart.title"),
            t.t("common.close"),
            move |_, _, cx| {
                if !busy {
                    close(cx)
                }
            },
        )
        .child(body.child(t.t(&self.status)))
    }
}
