//! Controller-only reviewed imports and exports. No writer is forwarded to program output.
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

pub struct Transfers {
    runtime: Runtime,
    export: bool,
    form: Form,
    status: String,
    busy: bool,
    close: std::rc::Rc<dyn Fn(&mut gpui::App)>,
}
fn form(this: &mut Transfers) -> &mut Form {
    &mut this.form
}
pub fn handout(workspace: &Value, draft: &Value) -> Result<Value, String> {
    let mut request = draft.clone();
    let kind = text(draft, "targetType");
    let id = text(draft, "targetId");
    let arrangement = text(draft, "arrangementId");
    if kind == "selection" {
        let ids = entries(&draft["targetIds"]);
        if ids.is_empty()
            || !id.is_empty()
            || !arrangement.is_empty()
            || ids.iter().enumerate().any(|(i, id)| {
                ids[..i].contains(id)
                    || !entries(&workspace["library"])
                        .iter()
                        .any(|v| v["id"] == *id)
            })
        {
            return Err("handout.invalidTarget".into());
        }
    } else {
        let collection = match kind.as_str() {
            "presentation" => "library",
            "playlist" => "playlist",
            _ => return Err("handout.invalidTarget".into()),
        };
        let target = entries(&workspace[collection])
            .into_iter()
            .find(|v| v["id"] == id)
            .ok_or("handout.invalidTarget")?;
        if !arrangement.is_empty()
            && (kind == "playlist"
                || !entries(&target["arrangements"])
                    .iter()
                    .any(|v| v["id"] == arrangement))
        {
            return Err("handout.invalidTarget".into());
        }
    }
    request["arrangementId"] = if arrangement.is_empty() {
        Value::Null
    } else {
        json!(arrangement)
    };
    Ok(request)
}
pub fn presentation_export(workspace: &Value, draft: &Value) -> Result<Value, String> {
    let item = entries(&workspace["library"])
        .into_iter()
        .find(|v| v["id"] == draft["itemId"])
        .ok_or("presentationExport.choose")?;
    let arrangement = text(draft, "arrangementId");
    if !arrangement.is_empty()
        && !entries(&item["arrangements"])
            .iter()
            .any(|v| v["id"] == arrangement)
    {
        return Err("handout.invalidTarget".into());
    }
    if !matches!(text(draft, "format").as_str(), "text" | "images" | "pdf")
        || (draft["format"] != "text" && draft["includeNotes"] == true)
    {
        return Err("presentationExport.notes".into());
    }
    let mut request = draft.clone();
    request["arrangementId"] = if arrangement.is_empty() {
        Value::Null
    } else {
        json!(arrangement)
    };
    Ok(request)
}
impl Transfers {
    pub fn new(
        runtime: Runtime,
        close: impl Fn(&mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = runtime.ui.read(cx).selected_library_item_id.clone();
        let mut form = Form::default();
        form.reset(json!({"targetType":"presentation","targetId":id,"arrangementId":null,"view":"thumbnails","paper":"a4","columns":2,"includeInformation":true,"includePresentationNotes":false,"includeDate":false,"includeSlideNumbers":true,"includeSlideLabels":true,"includeSlideNotes":false,"includeDisabled":false}));
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
            export: false,
            form,
            status: String::new(),
            busy: false,
            close,
        }
    }
    pub fn export(
        runtime: Runtime,
        close: impl Fn(&mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::new(runtime, close, cx);
        this.export = true;
        this.form.reset(json!({"itemId":this.runtime.ui.read(cx).selected_library_item_id,"format":"text","arrangementId":null,"includeDisabled":false,"includeNotes":false}));
        this
    }
    fn run(&mut self, method: &'static str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let content = self.runtime.content.read(cx);
        let path = content.workspace_path.clone();
        let request = if self.export {
            presentation_export(&content.workspace, &self.form.draft)
        } else {
            handout(&content.workspace, &self.form.draft)
        };
        let request = match request {
            Ok(v) if !path.is_empty() => v,
            Ok(_) => {
                self.status = "status.import.chooseWorkspace".into();
                return;
            }
            Err(e) => {
                self.status = e;
                return;
            }
        };
        self.busy = true;
        self.status.clear();
        let runtime = self.runtime.clone();
        let draft = self.form.draft.clone();
        cx.spawn(async move |this, cx| {
            let result = call(&runtime, cx, method, vec![json!(path), request], false).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if this.form.draft != draft {
                    this.status = "ticker.retired".into();
                } else {
                    this.status = match result {
                        Err(e) => e,
                        Ok(v) => {
                            if v["canceled"] == true {
                                "handout.canceled".into()
                            } else {
                                let t = Runtime::translator(cx);
                                if v["printSubmitted"] == true {
                                    if let Some(job) = v["printJobId"].as_u64() {
                                        t.format(
                                            "handout.printerSubmitted",
                                            &[
                                                ("job", &job.to_string()),
                                                ("pages", &v["pageCount"].to_string()),
                                            ],
                                        )
                                    } else {
                                        t.t("handout.submitted")
                                    }
                                } else {
                                    t.format(
                                        if this.export {
                                            "presentationExport.saved"
                                        } else {
                                            "handout.saved"
                                        },
                                        &[("path", &text(&v, "destinationPath"))],
                                    )
                                }
                            }
                        }
                    };
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
impl Render for Transfers {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Runtime::translator(cx);
        let m = cx.theme().metrics;
        let workspace = self.runtime.content.read(cx).workspace.clone();
        let mut body = div()
            .id("transfers-body")
            .flex()
            .flex_col()
            .gap(m.space_sm)
            .max_h(window.viewport_size().height - m.row_height * 6.)
            .overflow_y_scroll();
        let specs = if self.export {
            let item = entries(&workspace["library"])
                .into_iter()
                .find(|v| v["id"] == self.form.draft["itemId"])
                .unwrap_or(Value::Null);
            let mut arrangements = vec![(String::new(), t.t("presentationExport.master"))];
            arrangements.extend(choices(&item["arrangements"], "name"));
            let mut specs = vec![
                Spec::new(
                    "itemId",
                    t.t("presentationExport.presentation"),
                    Kind::Hook(
                        choices(&workspace["library"], "title"),
                        |f, _, v| {
                            f.set("itemId", json!(v));
                            f.set("arrangementId", Value::Null);
                        },
                        text(&self.form.draft, "itemId"),
                    ),
                ),
                Spec::new(
                    "format",
                    t.t("presentationExport.format"),
                    Kind::Hook(
                        ["text", "images", "pdf"]
                            .map(|v| (v.into(), t.t(&format!("presentationExport.{v}"))))
                            .to_vec(),
                        |f, _, v| {
                            f.set("format", json!(v));
                            f.set("includeNotes", json!(false));
                        },
                        text(&self.form.draft, "format"),
                    ),
                ),
                Spec::new(
                    "arrangementId",
                    t.t("presentationExport.order"),
                    Kind::Choice(arrangements),
                ),
                Spec::new(
                    "includeDisabled",
                    t.t("presentationExport.disabled"),
                    Kind::Check,
                ),
            ];
            if self.form.draft["format"] == "text" {
                specs.push(Spec::new(
                    "includeNotes",
                    t.t("presentationExport.notes"),
                    Kind::Check,
                ));
            }
            specs
        } else {
            let mut specs = vec![Spec::new(
                "targetType",
                t.t("handout.target"),
                Kind::Hook(
                    [
                        ("presentation", "handout.presentation"),
                        ("playlist", "handout.playlist"),
                        ("selection", "handout.selection"),
                    ]
                    .map(|(v, k)| (v.into(), t.t(k)))
                    .to_vec(),
                    |f, _, v| {
                        f.set("targetType", json!(v));
                        f.set("targetId", json!(""));
                        f.set("targetIds", json!([]));
                        f.set("arrangementId", Value::Null);
                    },
                    text(&self.form.draft, "targetType"),
                ),
            )];
            if self.form.draft["targetType"] == "selection" {
                for item in entries(&workspace["library"]) {
                    specs.push(Spec::new(
                        "targetIds",
                        text(&item, "title"),
                        Kind::Member(text(&item, "id")),
                    ));
                }
            } else {
                specs.push(Spec::new(
                    "targetId",
                    t.t("handout.target"),
                    Kind::Hook(
                        choices(
                            &workspace[if self.form.draft["targetType"] == "playlist" {
                                "playlist"
                            } else {
                                "library"
                            }],
                            "title",
                        ),
                        |f, _, v| {
                            f.set("targetId", json!(v));
                            f.set("arrangementId", Value::Null);
                        },
                        text(&self.form.draft, "targetId"),
                    ),
                ));
                if self.form.draft["targetType"] == "presentation" {
                    let item = entries(&workspace["library"])
                        .into_iter()
                        .find(|v| v["id"] == self.form.draft["targetId"])
                        .unwrap_or(Value::Null);
                    let mut opts = vec![(String::new(), t.t("presentationExport.master"))];
                    opts.extend(choices(&item["arrangements"], "name"));
                    specs.push(Spec::new(
                        "arrangementId",
                        t.t("presentationExport.order"),
                        Kind::Choice(opts),
                    ));
                }
            }
            specs.extend([
                Spec::new(
                    "view",
                    t.t("handout.view"),
                    Kind::Hook(
                        [
                            ("thumbnails", "handout.thumbnails"),
                            ("outline", "handout.outline"),
                        ]
                        .map(|(v, k)| (v.into(), t.t(k)))
                        .to_vec(),
                        |f, _, v| {
                            f.set("view", json!(v));
                            f.set("includeSlideNotes", json!(false));
                        },
                        text(&self.form.draft, "view"),
                    ),
                ),
                Spec::new(
                    "paper",
                    t.t("handout.paper"),
                    Kind::Choice(
                        [("a4", "handout.a4"), ("letter", "handout.letter")]
                            .map(|(v, k)| (v.into(), t.t(k)))
                            .to_vec(),
                    ),
                ),
            ]);
            if self.form.draft["view"] == "thumbnails" {
                specs.push(Spec::new("columns", t.t("handout.columns"), Kind::Number));
            }
            for (field, key) in [
                ("includeInformation", "handout.information"),
                ("includePresentationNotes", "handout.presentationNotes"),
                ("includeDate", "handout.date"),
                ("includeSlideNumbers", "handout.numbers"),
                ("includeSlideLabels", "handout.labels"),
                ("includeDisabled", "handout.disabled"),
            ] {
                specs.push(Spec::new(field, t.t(key), Kind::Check));
            }
            if self.form.draft["view"] == "outline" {
                specs.push(Spec::new(
                    "includeSlideNotes",
                    t.t("handout.notes"),
                    Kind::Check,
                ));
            }
            specs
        };
        for spec in specs {
            body = body.child(self.form.field("transfer", &spec, form, window, cx));
        }
        let actions = if self.export {
            vec![("exportPresentation", "presentationExport.export")]
        } else {
            vec![
                ("exportHandout", "handout.export"),
                ("previewHandout", "handout.preview"),
                ("printHandout", "handout.print"),
            ]
        };
        for (method, key) in actions {
            body = body.child(
                ActionButton::new(SharedString::from(method), t.t(key))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.run(method, cx))),
            );
        }
        let close = self.close.clone();
        let busy = self.busy;
        Dialog::new(
            "transfers",
            t.t(if self.export {
                "presentationExport.title"
            } else {
                "handout.title"
            }),
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
