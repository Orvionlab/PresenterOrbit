//! Native import dialogs use the controller's existing reviewed host commands.
use crate::{
    automation::form::{Form, Kind, Spec},
    services::{call, call_after, choices, entries, text},
    state::runtime::Runtime,
};
use gpui::{div, prelude::*, Context, SharedString, Window};
use orbit_ui::{
    controls::{ActionButton, ButtonVariant, Dialog},
    ActiveTheme,
};
use serde_json::{json, Value};

mod clipboard;
mod files;

pub struct Imports {
    runtime: Runtime,
    form: Form,
    clipboard: Option<clipboard::ClipboardImport>,
    file: Option<files::FileImport>,
    busy: bool,
    status: String,
    /// The titles an import found in its destination library, while the operator decides what to do with them.
    duplicates: Vec<String>,
    close: std::rc::Rc<dyn Fn(&mut gpui::App)>,
}
fn form(this: &mut Imports) -> &mut Form {
    &mut this.form
}
pub fn request(workspace: &Value, draft: &Value, method: &str) -> Result<Vec<Value>, String> {
    let key = if method == "exportPresentationBundle" {
        "itemId"
    } else {
        "libraryId"
    };
    let collection = if key == "itemId" {
        "library"
    } else {
        "libraries"
    };
    if !entries(&workspace[collection])
        .iter()
        .any(|v| v["id"] == draft[key])
    {
        return Err("presentationExport.choose".into());
    }
    let mut args = vec![draft[key].clone()];
    if method == "importKeynotePresentation" {
        if !entries(&workspace["playlist"])
            .iter()
            .any(|v| v["id"] == draft["playlistId"] && v["type"] != "folder")
        {
            return Err("appMenu.keynoteSelectPlaylist".into());
        }
        args.push(draft["playlistId"].clone());
    }
    Ok(args)
}
/// The titles an import stopped at (`{ canceled: false, duplicates }`), if it did.
pub fn duplicate_titles(answer: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(titles) = answer.get("duplicates") else {
        return Ok(None);
    };
    titles
        .as_array()
        .ok_or("portableBundle.incomplete")?
        .iter()
        .map(|title| {
            title
                .as_str()
                .map(str::to_string)
                .ok_or("portableBundle.incomplete")
        })
        .collect::<Result<_, _>>()
        .map(Some)
        .map_err(str::to_string)
}
pub fn imported_item(answer: &Value, draft: &Value, method: &str) -> Result<(), String> {
    let w = &answer["workspace"];
    for key in ["library", "libraries", "playlist", "themes"] {
        if !w[key].is_array() {
            return Err("portableBundle.incomplete".into());
        }
    }
    if !entries(&w["library"])
        .iter()
        .any(|v| v["id"] == answer["itemId"])
        || !entries(&w["libraries"]).iter().any(|v| {
            v["id"] == draft["libraryId"] && entries(&v["itemIds"]).contains(&answer["itemId"])
        })
    {
        return Err("portableBundle.incomplete".into());
    }
    if method == "importKeynotePresentation"
        && !entries(&w["playlist"]).iter().any(|p| {
            p["id"] == draft["playlistId"]
                && entries(&p["items"]).iter().any(|e| {
                    e["id"] == answer["playlistEntryId"]
                        && e["type"] == "presentation"
                        && e["itemId"] == answer["itemId"]
                })
        })
    {
        return Err("portableBundle.incomplete".into());
    }
    Ok(())
}
fn text_mutation(workspace: &Value, draft: &Value) -> Result<Value, String> {
    if draft["title"].as_str().is_none_or(|s| s.trim().is_empty()) {
        return Err("newPresentation.nameRequired".into());
    }
    if !entries(&workspace["libraries"])
        .iter()
        .any(|library| library["id"] == draft["libraryId"])
    {
        return Err("presentationExport.choose".into());
    }
    let mut mutation = json!({"type":"createPresentation"});
    for key in [
        "libraryId",
        "title",
        "content",
        "author",
        "operatorNotes",
        "splitMode",
        "splitMarker",
        "splitUnits",
    ] {
        mutation[key] = draft[key].clone();
    }
    if draft["createPlaylist"] == true {
        mutation["playlistTitle"] = draft["playlistTitle"].clone();
    }
    Ok(mutation)
}
impl Imports {
    pub fn new(
        runtime: Runtime,
        close: impl Fn(&mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let ui = runtime.ui.read(cx);
        let mut form = Form::default();
        form.reset(
            json!({"itemId":ui.selected_library_item_id,"libraryId":ui.selected_library_id,"playlistId":ui.selected_playlist_id,"title":"","content":"","author":"","operatorNotes":"","splitMode":"paragraph","splitMarker":"//","splitUnits":1}),
        );
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
            clipboard: None,
            file: None,
            busy: false,
            status: String::new(),
            duplicates: Vec::new(),
            close,
        }
    }
    fn create_text(&mut self, cx: &mut Context<Self>) {
        let draft = self.form.draft.clone();
        if self.busy {
            return;
        }
        let mutation = text_mutation(&self.runtime.content.read(cx).workspace, &draft).and_then(
            |mut mutation| {
                if let Some(clipboard) = &self.clipboard {
                    clipboard.configure_mutation(&mut mutation, &draft, cx)?;
                } else if matches!(self.file, Some(files::FileImport::Text))
                    && mutation.get("playlistTitle").is_none()
                    && draft["playlistId"]
                        .as_str()
                        .is_some_and(|id| !id.is_empty())
                {
                    mutation["playlistId"] = draft["playlistId"].clone();
                }
                Ok(mutation)
            },
        );
        let mutation = match mutation {
            Ok(mutation) => mutation,
            Err(error) => {
                self.status = error;
                cx.notify();
                return;
            }
        };
        self.busy = true;
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let answer = runtime
                .persist(
                    cx,
                    crate::state::runtime::Persist::new(
                        crate::state::runtime::Kind::Library,
                        mutation,
                        |_, _| {},
                    ),
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if answer.is_some() {
                    (this.close)(cx);
                } else {
                    this.status = runtime.status.read(cx).message.clone();
                    cx.notify();
                }
            });
        })
        .detach();
    }
    /// `duplicate` answers the question of an import that stopped at duplicate titles.
    fn run(
        &mut self,
        method: &'static str,
        duplicate: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        let content = self.runtime.content.read(cx);
        let path = content.workspace_path.clone();
        if path.is_empty() {
            self.status = "status.import.chooseWorkspace".into();
            return;
        }
        let mut args = match request(&content.workspace, &self.form.draft, method) {
            Ok(v) => v,
            Err(e) => {
                self.status = e;
                cx.notify();
                return;
            }
        };
        args.insert(0, json!(path));
        let write = method != "exportPresentationBundle";
        self.busy = true;
        self.status.clear();
        let runtime = self.runtime.clone();
        let draft = self.form.draft.clone();
        cx.spawn(async move |this, cx| {
            let trailing = duplicate
                .map(|choice| vec![json!(choice)])
                .unwrap_or_default();
            let answer = call_after(&runtime, cx, method, args, write, trailing)
                .await
                .and_then(|v| {
                    if matches!(
                        method,
                        "importKeynotePresentation" | "importPowerPointImages"
                    ) && v["canceled"] != true
                    {
                        imported_item(&v, &draft, method)?;
                    }
                    Ok(v)
                });
            if !this
                .update(cx, |this, cx| {
                    if this.form.draft != draft {
                        this.busy = false;
                        this.status = "ticker.retired".into();
                        cx.notify();
                        false
                    } else {
                        true
                    }
                })
                .unwrap_or(false)
            {
                return;
            }
            if let Ok(v) = &answer {
                if v["canceled"] != true
                    && matches!(
                        method,
                        "importKeynotePresentation" | "importPowerPointImages"
                    )
                {
                    let id = text(v, "itemId");
                    let playlist = if method == "importKeynotePresentation" {
                        text(&draft, "playlistId")
                    } else {
                        String::new()
                    };
                    let index = entries(&v["workspace"]["playlist"])
                        .iter()
                        .find(|p| p["id"] == playlist)
                        .and_then(|p| {
                            entries(&p["items"])
                                .iter()
                                .position(|e| e["id"] == v["playlistEntryId"])
                        });
                    cx.update(|cx| {
                        runtime.ui.update(cx, |ui, cx| {
                            ui.selected_library_id = text(&draft, "libraryId");
                            ui.selected_library_ids = vec![ui.selected_library_id.clone()];
                            ui.selected_library_item_id = id.clone();
                            ui.selected_playlist_id = playlist.clone();
                            ui.selected_playlist_ids = if playlist.is_empty() {
                                vec![]
                            } else {
                                vec![playlist.clone()]
                            };
                            ui.selected_documents =
                                vec![crate::state::slices::Selected { item_id: id, index }];
                            ui.selected_document_source = Some(if playlist.is_empty() {
                                crate::state::slices::Source::Library
                            } else {
                                crate::state::slices::Source::Playlist
                            });
                            ui.selected_document_playlist_id = playlist;
                            ui.library_anchor = None;
                            ui.playlist_anchor = None;
                            ui.document_anchor = None;
                            cx.notify();
                        })
                    });
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.status = if this.form.draft != draft {
                    "ticker.retired".into()
                } else {
                    match answer {
                        Err(e) => e,
                        // A failure leaves the question as it was; any answer settles it.
                        Ok(v) => match duplicate_titles(&v) {
                            Err(error) => error,
                            Ok(titles) => {
                                this.duplicates = titles.unwrap_or_default();
                                if !this.duplicates.is_empty() {
                                    String::new()
                                } else if v["canceled"] == true {
                                    "portableBundle.canceled".into()
                                } else if write {
                                    "portableBundle.imported".into()
                                } else {
                                    "portableBundle.exported".into()
                                }
                            }
                        },
                    }
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
impl Render for Imports {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.clipboard.is_some() {
            return self.clipboard_dialog(window, cx).into_any_element();
        }
        if self.file.is_some() {
            return self.file_dialog(window, cx);
        }
        let t = Runtime::translator(cx);
        let m = cx.theme().metrics;
        let workspace = self.runtime.content.read(cx).workspace.clone();
        let mut body = div()
            .id("import-body")
            .flex()
            .flex_col()
            .gap(m.space_sm)
            .max_h(window.viewport_size().height - m.row_height * 6.)
            .overflow_y_scroll();
        for spec in [
            Spec::new(
                "itemId",
                t.t("presentationExport.presentation"),
                Kind::Choice(choices(&workspace["library"], "title")),
            ),
            Spec::new(
                "libraryId",
                t.t("portableBundle.destination"),
                Kind::Choice(choices(&workspace["libraries"], "title")),
            ),
        ] {
            body = body.child(self.form.field("imports", &spec, form, window, cx));
        }
        body = body.child(self.form.field(
            "imports",
            &Spec::new(
                "playlistId",
                t.t("handout.playlist"),
                Kind::Choice(choices(
                    &json!(entries(&workspace["playlist"])
                            .into_iter()
                            .filter(|p| p["type"] != "folder")
                            .collect::<Vec<_>>()),
                    "title",
                )),
            ),
            form,
            window,
            cx,
        ));
        for spec in files::text_fields(t) {
            body = body.child(self.form.field("text-import", &spec, form, window, cx));
        }
        body = body.child(
            ActionButton::new("text-import-create", t.t("appMenu.newPresentation"))
                .disabled(self.busy)
                .on_click(cx.listener(|this, _, _, cx| this.create_text(cx))),
        );
        let questions = self.duplicates.clone();
        let asking = !questions.is_empty();
        if asking {
            body = body
                .child(t.format(
                    "portableBundle.duplicate.message",
                    &[("titles", &questions.join(", "))],
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(m.control_gap)
                        .child(
                            ActionButton::new("duplicate-cancel", t.t("common.cancel"))
                                .variant(ButtonVariant::Secondary)
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.duplicates.clear();
                                    this.status = "portableBundle.canceled".into();
                                    cx.notify();
                                })),
                        )
                        .children(
                            [
                                ("useExisting", ButtonVariant::Secondary),
                                ("keepBoth", ButtonVariant::Secondary),
                                ("replace", ButtonVariant::Danger),
                            ]
                            .map(|(choice, variant)| {
                                ActionButton::new(
                                    SharedString::from(format!("duplicate-{choice}")),
                                    t.t(&format!("portableBundle.duplicate.{choice}")),
                                )
                                .variant(variant)
                                .disabled(self.busy)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.run("importPresentationBundle", Some(choice), cx)
                                    },
                                ))
                            }),
                        ),
                );
        }
        for (method, key) in [
            ("importPresentationBundle", "portableBundle.import"),
            ("exportPresentationBundle", "portableBundle.export"),
            (
                "importKeynotePresentation",
                "appMenu.importKeynotePresentation",
            ),
            ("importPowerPointImages", "appMenu.importPowerPointImages"),
        ] {
            if asking {
                break;
            }
            body = body.child(
                ActionButton::new(SharedString::from(method), t.t(key))
                    .disabled(
                        self.busy
                            || (method == "importKeynotePresentation"
                                && !cfg!(target_os = "macos"))
                            || (method == "importPowerPointImages"
                                && !cfg!(any(target_os = "windows", target_os = "macos"))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.run(method, None, cx))),
            );
        }
        let close = self.close.clone();
        let busy = self.busy;
        Dialog::new(
            "imports",
            t.t("portableBundle.title"),
            t.t("common.close"),
            move |_, _, cx| {
                if !busy {
                    close(cx)
                }
            },
        )
        .child(body.child(t.t(&self.status)))
        .into_any_element()
    }
}
