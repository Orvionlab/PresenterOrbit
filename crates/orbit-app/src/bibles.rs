use crate::{
    automation::form::{Form, Kind, Spec},
    services::{call, choices, entries, text},
    state::runtime::Runtime,
};
use gpui::{div, prelude::*, Context, SharedString, Window};
use orbit_ui::{
    controls::{ActionButton, CheckboxField, Dialog},
    ActiveTheme,
};
use serde_json::{json, Value};

pub struct Bibles {
    runtime: Runtime,
    form: Form,
    catalog: Value,
    books: Value,
    results: Value,
    passage: Value,
    loaded: Value,
    status: String,
    busy: bool,
    close: std::rc::Rc<dyn Fn(&mut gpui::App)>,
}
fn form(this: &mut Bibles) -> &mut Form {
    &mut this.form
}

pub fn passage(draft: &Value) -> Result<Value, String> {
    let ids = entries(&draft["translationIds"]);
    let start = draft["chapterStart"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or("bible.invalidRange")?;
    let end = draft["chapterEnd"]
        .as_u64()
        .filter(|n| *n >= start)
        .ok_or("bible.invalidRange")?;
    if ids.is_empty() || ids.len() > 8 || text(draft, "bookId").is_empty() {
        return Err("bible.invalidRange".into());
    }
    let mut request = json!({"translationIds":ids,"bookId":draft["bookId"],"chapterStart":start,"chapterEnd":end});
    for key in ["verseStart", "verseEnd"] {
        if !draft[key].is_null() && draft[key] != "" {
            let n = draft[key]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or("bible.invalidRange")?;
            request[key] = json!(n);
        }
    }
    if start == end
        && request["verseStart"]
            .as_u64()
            .zip(request["verseEnd"].as_u64())
            .is_some_and(|(a, b)| a > b)
    {
        return Err("bible.invalidRange".into());
    }
    Ok(request)
}
pub fn presentation(workspace: &Value, draft: &Value, loaded: &Value) -> Result<Value, String> {
    let invalid = || "bible.invalidRange".to_string();
    if loaded.is_null()
        || draft["translationIds"] != loaded["translationIds"]
        || text(draft, "libraryId").is_empty()
    {
        return Err(invalid());
    }
    let assignment = text(draft, "presentationThemeId");
    let (theme, slide) = assignment.split_once("::").ok_or_else(invalid)?;
    let themes = entries(&workspace["themes"]);
    let layout = themes
        .iter()
        .find(|v| v["id"] == theme)
        .and_then(|t| t["slides"].as_array())
        .and_then(|s| s.iter().find(|v| v["id"] == slide))
        .ok_or_else(invalid)?;
    let layers = entries(&layout["layers"]);
    let ids = entries(&draft["translationIds"]);
    let mapped: Vec<Value> = ids
        .iter()
        .map(|id| draft["mapping"][id.as_str().unwrap_or("")].clone())
        .collect();
    let valid = |id: &Value| {
        layers.iter().any(|l| {
            l["id"] == *id
                && (text(l, "type").eq_ignore_ascii_case("text")
                    || matches!(text(l, "role").as_str(), "slide-text" | "primary")
                    || text(l, "name").trim().eq_ignore_ascii_case("text")
                    || text(l, "id").eq_ignore_ascii_case("text"))
        })
    };
    if mapped.iter().any(|id| !valid(id))
        || mapped
            .iter()
            .enumerate()
            .any(|(i, id)| mapped[..i].contains(id))
    {
        return Err(invalid());
    }
    let reference = text(draft, "referenceLayerId");
    if !reference.is_empty() && (!valid(&json!(reference)) || mapped.contains(&json!(reference))) {
        return Err(invalid());
    }
    if draft["options"]["referenceMode"] == "verse" && draft["options"]["breakOnVerse"] != true {
        return Err(invalid());
    }
    let mut request = json!({"libraryId":draft["libraryId"],"presentationThemeId":assignment,"textLayerIds":mapped,"passage":loaded,"options":draft["options"]});
    for key in ["playlistId", "referenceLayerId"] {
        if !text(draft, key).is_empty() {
            request[key] = draft[key].clone();
        }
    }
    Ok(request)
}
impl Bibles {
    pub fn new(
        runtime: Runtime,
        close: impl Fn(&mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            runtime,
            form: Form::default(),
            catalog: Value::Null,
            books: Value::Null,
            results: Value::Null,
            passage: Value::Null,
            loaded: Value::Null,
            status: String::new(),
            busy: false,
            close: std::rc::Rc::new(close),
        };
        this.form.reset(json!({"translationIds":[],"chapterStart":1,"chapterEnd":1,"options":{"showVerseNumbers":true,"breakOnVerse":true,"showTranslation":true,"referenceMode":"verse"}}));
        this.run("listBibles", vec![], false, cx);
        this
    }
    fn run(&mut self, method: &'static str, args: Vec<Value>, write: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status.clear();
        let runtime = self.runtime.clone();
        let draft = self.form.draft.clone();
        cx.spawn(async move |this, cx| {
            let result = call(&runtime, cx, method, args, write).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if this.form.draft != draft {
                    this.status = "ticker.retired".into();
                    cx.notify();
                    return;
                }
                match result {
                    Err(e) => this.status = e,
                    Ok(v) => match method {
                        "listBibles" => {
                            let ids = entries(&v);
                            let chosen = entries(&this.form.draft["translationIds"])
                                .into_iter()
                                .filter(|id| ids.iter().any(|t| t["id"] == *id))
                                .collect::<Vec<_>>();
                            this.form.set("translationIds", json!(chosen));
                            this.catalog = v;
                        }
                        "getBibleBooks" => this.books = v,
                        "getBiblePassage" => {
                            this.loaded = passage(&draft).unwrap_or(Value::Null);
                            this.passage = v;
                        }
                        "resolveBibleReference" => {
                            for key in [
                                "bookId",
                                "chapterStart",
                                "chapterEnd",
                                "verseStart",
                                "verseEnd",
                            ] {
                                this.form.set(key, v[key].clone());
                            }
                            this.form.refresh();
                            this.load(cx);
                        }
                        "searchBible" => this.results = v,
                        "createBiblePresentation" => {
                            let id = text(&v, "itemId");
                            let playlist = text(&draft, "playlistId");
                            let content = runtime.content.read(cx);
                            let index = content.playlist(&playlist).and_then(|p| {
                                p.entries.iter().position(|entry| {
                                    entry.id == text(&v, "playlistEntryId") || entry.item_id == id
                                })
                            });
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
                                ui.selected_document_source = Some(if index.is_some() {
                                    crate::state::slices::Source::Playlist
                                } else {
                                    crate::state::slices::Source::Library
                                });
                                ui.selected_document_playlist_id = playlist;
                                ui.library_anchor = None;
                                ui.playlist_anchor = None;
                                ui.document_anchor = None;
                                cx.notify();
                            });
                            (this.close)(cx);
                        }
                        "openExternalUrl" => {}
                        _ => this.run("listBibles", vec![], false, cx),
                    },
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn load(&mut self, cx: &mut Context<Self>) {
        match passage(&self.form.draft) {
            Ok(v) => self.run("getBiblePassage", vec![v], false, cx),
            Err(e) => self.status = e,
        }
    }
}
impl Render for Bibles {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Runtime::translator(cx);
        let gap = cx.theme().metrics.space_xs;
        let ws = self.runtime.content.read(cx).workspace.clone();
        let path = self.runtime.content.read(cx).workspace_path.clone();
        let mut body = div()
            .id("bibles-body")
            .flex()
            .flex_col()
            .gap(gap)
            .max_h(window.viewport_size().height - cx.theme().metrics.row_height * 6.)
            .overflow_y_scroll();
        for (id, key, method) in [
            ("import", "bible.importPackage", "importBiblePackage"),
            ("install", "bible.installWeb", "installWebBible"),
            ("refresh", "common.refresh", "listBibles"),
        ] {
            body = body.child(
                ActionButton::new(id, t.t(key))
                    .disabled(self.busy)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.run(method, vec![], false, cx)),
                    ),
            );
        }
        for translation in entries(&self.catalog) {
            let id = text(&translation, "id");
            let weak = cx.weak_entity();
            let chosen = entries(&self.form.draft["translationIds"]);
            let checked = chosen.contains(&json!(id));
            let remove = id.clone();
            body = body.child(
                CheckboxField::new(
                    SharedString::from(format!("bible-{id}")),
                    text(&translation, "name"),
                    checked,
                )
                .disabled(self.busy || (!checked && chosen.len() >= 8))
                .on_change(move |on, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let mut ids = entries(&this.form.draft["translationIds"]);
                        ids.retain(|v| v != &json!(id));
                        if on {
                            ids.push(json!(id));
                        }
                        this.form.set("translationIds", json!(ids));
                        this.loaded = Value::Null;
                        this.passage = Value::Null;
                        this.results = Value::Null;
                        this.form.refresh();
                        cx.notify();
                    });
                }),
            );
            body = body.child(div().child(format!(
                "{} {}",
                text(&translation, "language"),
                text(&translation["license"], "notice")
            )));
            let url = text(&translation["license"], "url");
            body = body.child(
                ActionButton::new(
                    SharedString::from(format!("license-{remove}")),
                    text(&translation["license"], "name"),
                )
                .disabled(self.busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.run("openExternalUrl", vec![json!(url)], false, cx)
                })),
            );
            body = body.child(
                ActionButton::new(
                    SharedString::from(format!("remove-{remove}")),
                    t.t("common.delete"),
                )
                .disabled(self.busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.run("removeBible", vec![json!(remove)], false, cx)
                })),
            );
        }
        for spec in [
            Spec::new("reference", t.t("bible.referenceLookup"), Kind::Text),
            Spec::new("query", t.t("common.search"), Kind::Text),
        ] {
            body = body.child(self.form.field("bible", &spec, form, window, cx));
        }
        for (id, key, method) in [
            ("books", "bible.book", "getBibleBooks"),
            ("resolve", "bible.resolveReference", "resolveBibleReference"),
            ("search", "common.search", "searchBible"),
        ] {
            body=body.child(ActionButton::new(id,t.t(key)).disabled(self.busy).on_click(cx.listener(move|this,_,_,cx|{let primary=this.form.draft["translationIds"][0].clone(); let args=match method {"getBibleBooks"=>vec![primary],"resolveBibleReference"=>vec![primary,this.form.draft["reference"].clone()],_=>vec![json!({"translationId":primary,"query":this.form.draft["query"],"offset":0})]};this.run(method,args,false,cx)})));
        }
        for spec in [
            Spec::new(
                "bookId",
                t.t("bible.book"),
                Kind::Choice(choices(&self.books, "name")),
            ),
            Spec::new("chapterStart", t.t("bible.chapterStart"), Kind::Number),
            Spec::new("chapterEnd", t.t("bible.chapterEnd"), Kind::Number),
            Spec::new("verseStart", t.t("bible.verseStart"), Kind::Number),
            Spec::new("verseEnd", t.t("bible.verseEnd"), Kind::Number),
        ] {
            body = body.child(self.form.field("bible", &spec, form, window, cx));
        }
        body = body.child(
            ActionButton::new("load", t.t("bible.loadPassage"))
                .disabled(self.busy)
                .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
        );
        for (index, verse) in entries(&self.results["results"]).into_iter().enumerate() {
            body = body.child(
                ActionButton::new(
                    SharedString::from(format!("result-{index}")),
                    format!(
                        "{} {}:{} {}",
                        text(&verse, "bookName"),
                        verse["chapter"],
                        verse["verse"],
                        text(&verse, "text")
                    ),
                )
                .disabled(self.busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    for (k, v) in [
                        ("bookId", verse["bookId"].clone()),
                        ("chapterStart", verse["chapter"].clone()),
                        ("chapterEnd", verse["chapter"].clone()),
                        ("verseStart", verse["verse"].clone()),
                        ("verseEnd", verse["verse"].clone()),
                    ] {
                        this.form.set(k, v);
                    }
                    this.form.refresh();
                    this.load(cx)
                })),
            );
        }
        let offset = self.results["offset"].as_u64().unwrap_or(0)
            + self.results["limit"].as_u64().unwrap_or(20);
        body=body.child(ActionButton::new("more",t.t("common.next")).disabled(self.busy || offset>=self.results["total"].as_u64().unwrap_or(0)).on_click(cx.listener(move|this,_,_,cx|this.run("searchBible",vec![json!({"translationId":this.form.draft["translationIds"][0],"query":this.form.draft["query"],"offset":offset})],false,cx))));
        for translation in entries(&self.passage["translations"]) {
            body = body.child(div().child(text(&translation["translation"], "name")));
            for v in entries(&translation["verses"]) {
                body = body.child(div().child(format!(
                    "{}:{} {}",
                    v["chapter"],
                    v["verse"],
                    text(&v, "text")
                )));
            }
        }
        let themes = entries(&ws["themes"]);
        let theme_choices = themes
            .iter()
            .flat_map(|theme| {
                entries(&theme["slides"]).into_iter().map(|slide| {
                    (
                        format!("{}::{}", text(theme, "id"), text(&slide, "id")),
                        format!("{} / {}", text(theme, "title"), text(&slide, "name")),
                    )
                })
            })
            .collect();
        for spec in [
            Spec::new(
                "libraryId",
                t.t("bible.targetLibrary"),
                Kind::Choice(choices(&ws["libraries"], "title")),
            ),
            Spec::new(
                "playlistId",
                t.t("bible.targetPlaylist"),
                Kind::Choice(
                    std::iter::once((String::new(), t.t("common.none")))
                        .chain(choices(&ws["playlist"], "title"))
                        .collect(),
                ),
            ),
            Spec::new(
                "presentationThemeId",
                t.t("bible.presentationTheme"),
                Kind::Choice(theme_choices),
            ),
        ] {
            body = body.child(self.form.field("targets", &spec, form, window, cx));
        }
        let assignment = text(&self.form.draft, "presentationThemeId");
        let layers = themes
            .iter()
            .flat_map(|th| {
                entries(&th["slides"])
                    .into_iter()
                    .map(move |s| (format!("{}::{}", text(th, "id"), text(&s, "id")), s))
            })
            .find(|(id, _)| id == &assignment)
            .map(|(_, s)| choices(&s["layers"], "name"))
            .unwrap_or_default();
        for id in entries(&self.form.draft["translationIds"]) {
            let spec = Spec::new(
                format!("mapping.{}", id.as_str().unwrap_or("")),
                id.as_str().unwrap_or(""),
                Kind::Choice(layers.clone()),
            );
            body = body.child(self.form.field("mapping", &spec, form, window, cx));
        }
        for spec in [
            Spec::new(
                "referenceLayerId",
                t.t("bible.referenceLookup"),
                Kind::Choice(
                    std::iter::once((String::new(), t.t("common.none")))
                        .chain(layers)
                        .collect(),
                ),
            ),
            Spec::new(
                "options.showVerseNumbers",
                t.t("bible.showVerseNumbers"),
                Kind::Check,
            ),
            Spec::new(
                "options.breakOnVerse",
                t.t("bible.breakOnVerse"),
                Kind::Check,
            ),
            Spec::new(
                "options.showTranslation",
                t.t("bible.showTranslation"),
                Kind::Check,
            ),
            Spec::new(
                "options.referenceMode",
                t.t("bible.referenceLookup"),
                Kind::Choice(
                    ["verse", "passageEach", "passageLast", "none"]
                        .iter()
                        .map(|v| (v.to_string(), t.t(&format!("bible.referenceMode.{v}"))))
                        .collect(),
                ),
            ),
        ] {
            body = body.child(self.form.field("options", &spec, form, window, cx));
        }
        body = body.child(
            ActionButton::new("create-bible", t.t("bible.createPresentation"))
                .disabled(self.busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    match presentation(&ws, &this.form.draft, &this.loaded) {
                        Ok(v) => {
                            this.run("createBiblePresentation", vec![json!(path), v], true, cx)
                        }
                        Err(e) => {
                            this.status = e;
                            cx.notify();
                        }
                    }
                })),
        );
        let close = self.close.clone();
        Dialog::new(
            "bibles",
            t.t("bible.dialogTitle"),
            t.t("common.close"),
            move |_, _, cx| close(cx),
        )
        .child(
            body.child(
                div()
                    .id("bible-status")
                    .role(gpui::Role::Alert)
                    .child(t.t(&self.status)),
            ),
        )
    }
}
