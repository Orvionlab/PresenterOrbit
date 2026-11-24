use std::path::PathBuf;

use gpui::{div, prelude::*, AnyElement, Context, Window};
use orbit_ui::{
    controls::{ActionButton, ButtonVariant, Dialog},
    ActiveTheme, Translator,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{call, choices, entries, form, Imports, Kind, Runtime, Spec};

pub(super) enum FileImport {
    Choosing,
    Text,
    Files(Vec<PathBuf>),
    Done(FileReport),
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum PickedFile {
    Text { file_path: PathBuf, text: String },
    Files { paths: Vec<PathBuf> },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Destination<'a> {
    sources: &'a [PathBuf],
    library_id: &'a str,
    playlist_id: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FileReport {
    imported_files: usize,
    warnings: Vec<String>,
    failures: Vec<String>,
}

pub(super) fn text_fields(t: Translator) -> [Spec; 9] {
    [
        Spec::new("title", t.t("textImport.presentationTitle"), Kind::Text),
        Spec::new("content", t.t("cue.editSlides"), Kind::Multiline),
        Spec::new("author", t.t("textImport.author"), Kind::Text),
        Spec::new("operatorNotes", t.t("cue.operatorNotes"), Kind::Multiline),
        Spec::new(
            "splitMode",
            t.t("textImport.splitBy"),
            Kind::Choice(
                [
                    ("paragraph", "textImport.paragraphs"),
                    ("line", "textImport.lines"),
                    ("marker", "textImport.marker"),
                ]
                .into_iter()
                .map(|(value, key)| (value.into(), t.t(key)))
                .collect(),
            ),
        ),
        Spec::new("splitMarker", t.t("textImport.marker"), Kind::Text),
        Spec::new("splitUnits", t.t("textImport.perSlide"), Kind::Number),
        Spec::new(
            "createPlaylist",
            t.t("textImport.createPlaylist"),
            Kind::Check,
        ),
        Spec::new("playlistTitle", t.t("textImport.playlistTitle"), Kind::Text),
    ]
}

impl Imports {
    pub fn picking_files(&self) -> bool {
        self.busy && matches!(self.file, Some(FileImport::Choosing))
    }

    pub fn pick_file(&mut self, pro_presenter_only: bool, cx: &mut Context<Self>) {
        self.file = Some(FileImport::Choosing);
        let content = self.runtime.content.read(cx);
        let scope = content.workspace_path.clone();
        if scope.is_empty() {
            self.status = "status.import.chooseWorkspace".into();
            cx.notify();
            return;
        }
        let library = content
            .library(&self.runtime.ui.read(cx).selected_library_id)
            .or_else(|| content.libraries.first());
        self.form.draft["libraryId"] = json!(library.map(|library| &library.id));
        self.form.draft["playlistId"] = json!("");
        self.busy = true;
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = call(
                &runtime,
                cx,
                "selectPresentationImportFiles",
                vec![json!(pro_presenter_only)],
                false,
            )
            .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if runtime.content.read(cx).workspace_path != scope {
                    return;
                }
                let picked = result.and_then(|value| {
                    if value["canceled"] == true {
                        return Ok(None);
                    }
                    serde_json::from_value::<PickedFile>(value)
                        .map(Some)
                        .map_err(|error| error.to_string())
                });
                match picked {
                    Ok(None) => (this.close)(cx),
                    Ok(Some(PickedFile::Text { file_path, text })) if !text.trim().is_empty() => {
                        let t = Runtime::translator(cx);
                        let title = file_path
                            .file_stem()
                            .and_then(|name| name.to_str())
                            .map(str::to_string)
                            .unwrap_or_else(|| t.t("collections.default.textFilePresentation"));
                        this.form.draft["title"] = json!(title);
                        this.form.draft["content"] = json!(text.trim());
                        this.form.refresh();
                        this.file = Some(FileImport::Text);
                    }
                    Ok(Some(PickedFile::Text { .. })) => {
                        this.status = "status.import.textFileEmpty".into()
                    }
                    Ok(Some(PickedFile::Files { paths })) if paths.is_empty() => (this.close)(cx),
                    Ok(Some(PickedFile::Files { paths })) => {
                        this.file = Some(FileImport::Files(paths))
                    }
                    Err(error) => this.status = error,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn import_file(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(FileImport::Files(paths)) = &self.file else {
            self.create_text(cx);
            return;
        };
        let Some(library_id) = self.form.draft["libraryId"]
            .as_str()
            .filter(|id| !id.is_empty())
        else {
            self.status = "presentationExport.choose".into();
            cx.notify();
            return;
        };
        let args = vec![
            json!(self.runtime.content.read(cx).workspace_path),
            json!(Destination {
                sources: paths,
                library_id,
                playlist_id: self.form.draft["playlistId"]
                    .as_str()
                    .filter(|id| !id.is_empty()),
            }),
        ];
        self.busy = true;
        self.status.clear();
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let answer = call(&runtime, cx, "importPresentationFiles", args, true)
                .await
                .and_then(|value| {
                    serde_json::from_value::<FileReport>(value).map_err(|error| error.to_string())
                });
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match answer {
                    Ok(report) if report.warnings.is_empty() && report.failures.is_empty() => {
                        (this.close)(cx)
                    }
                    Ok(report) => this.file = Some(FileImport::Done(report)),
                    Err(error) => this.status = error,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn file_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = Runtime::translator(cx);
        let theme = *cx.theme();
        let m = theme.metrics;
        let busy = self.busy;
        if self.picking_files() {
            return div().into_any_element();
        }
        let close = self.close.clone();
        let mut dialog = Dialog::new(
            "import-file",
            t.t("fileImport.title"),
            t.t("common.close"),
            move |_, _, cx| {
                if !busy {
                    close(cx);
                }
            },
        );
        let mut body = div()
            .id("file-import-body")
            .flex()
            .flex_col()
            .gap(m.space_sm)
            .max_h(window.viewport_size().height * 0.65)
            .overflow_y_scroll();
        let selectable = matches!(self.file, Some(FileImport::Text | FileImport::Files(_)));
        let finished = matches!(self.file, Some(FileImport::Done(_)));
        if busy {
            body = body.child(t.t("fileImport.working"));
        } else if let Some(FileImport::Done(report)) = &self.file {
            body = body
                .child(t.format(
                    "fileImport.result",
                    &[
                        ("count", &report.imported_files.to_string()),
                        (
                            "total",
                            &(report.imported_files + report.failures.len()).to_string(),
                        ),
                    ],
                ))
                .children(
                    report
                        .failures
                        .iter()
                        .chain(&report.warnings)
                        .map(|warning| {
                            div()
                                .text_color(theme.colors.warning)
                                .child(warning.clone())
                        }),
                );
        } else if selectable {
            let workspace = &self.runtime.content.read(cx).workspace;
            let libraries = choices(&workspace["libraries"], "title");
            let playlists = std::iter::once((String::new(), t.t("bible.noPlaylist")))
                .chain(
                    entries(&workspace["playlist"])
                        .into_iter()
                        .filter(|playlist| playlist["type"] != "folder")
                        .map(|playlist| {
                            (
                                super::text(&playlist, "id"),
                                super::text(&playlist, "title"),
                            )
                        }),
                )
                .collect();
            body = body.child(self.form.field(
                "file-import",
                &Spec::new(
                    "libraryId",
                    t.t("textImport.library"),
                    Kind::Choice(libraries),
                ),
                form,
                window,
                cx,
            ));
            if self.form.draft["createPlaylist"] != true {
                body = body.child(self.form.field(
                    "file-import",
                    &Spec::new(
                        "playlistId",
                        t.t("handout.playlist"),
                        Kind::Choice(playlists),
                    ),
                    form,
                    window,
                    cx,
                ));
            }
            if let Some(FileImport::Files(paths)) = &self.file {
                let summary = match paths.as_slice() {
                    [path] => path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string()),
                    _ => t.format(
                        "fileImport.selection",
                        &[("count", &paths.len().to_string())],
                    ),
                };
                body = body.child(div().text_color(theme.colors.text_muted).child(summary));
            } else {
                for spec in text_fields(t) {
                    if spec.path != "playlistTitle" || self.form.draft["createPlaylist"] == true {
                        body = body.child(self.form.field("file-import", &spec, form, window, cx));
                    }
                }
            }
        }
        if !self.status.is_empty() {
            body = body.child(
                div()
                    .text_color(theme.colors.warning)
                    .child(t.t(&self.status)),
            );
        }
        let close = self.close.clone();
        dialog = dialog.footer(
            ActionButton::new(
                "file-import-close",
                t.t(if finished {
                    "common.done"
                } else {
                    "common.cancel"
                }),
            )
            .variant(ButtonVariant::Secondary)
            .disabled(busy)
            .on_click(move |_, _, cx| close(cx)),
        );
        if selectable && !busy {
            dialog = dialog.footer(
                ActionButton::new("file-import-confirm", t.t("fileImport.confirm"))
                    .on_click(cx.listener(|this, _, _, cx| this.import_file(cx))),
            );
        }
        dialog.child(body).into_any_element()
    }
}
