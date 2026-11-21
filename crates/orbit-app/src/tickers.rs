use crate::{
    automation::form::{Form, Kind, Spec},
    services::{call, entries, text},
    state::runtime::Runtime,
};
use gpui::{div, prelude::*, Context, SharedString, Window};
use orbit_ui::{
    controls::{ActionButton, Dialog},
    ActiveTheme,
};
use serde_json::{json, Value};
pub struct Tickers {
    runtime: Runtime,
    form: Form,
    original: Value,
    status: String,
    live: Value,
    busy: bool,
    close: std::rc::Rc<dyn Fn(&mut gpui::App)>,
}
fn form(this: &mut Tickers) -> &mut Form {
    &mut this.form
}
pub fn replacement(
    collection: &Value,
    original: &Value,
    draft: &Value,
    delete: bool,
) -> Result<Value, String> {
    let mut all = entries(collection);
    let index = all.iter().position(|v| v["id"] == draft["id"]);
    if index.map(|i| &all[i]).unwrap_or(&Value::Null) != original {
        return Err("ticker.retired".into());
    }
    if delete {
        all.retain(|v| v["id"] != draft["id"]);
    } else if let Some(i) = index {
        all[i] = draft.clone();
    } else {
        all.push(draft.clone());
    }
    Ok(json!(all))
}
impl Tickers {
    pub fn new(
        runtime: Runtime,
        close: impl Fn(&mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            runtime,
            form: Form::default(),
            original: Value::Null,
            status: String::new(),
            live: Value::Null,
            busy: false,
            close: std::rc::Rc::new(close),
        };
        this.new_draft();
        this.run("getTickerStatus", Value::Null, cx);
        this
    }
    fn new_draft(&mut self) {
        self.original = Value::Null;
        self.form.reset(json!({"id":uuid::Uuid::new_v4().to_string(),"title":"","text":"","speed":50,"direction":"left","repeat":true}));
    }
    fn run(&mut self, method: &'static str, payload: Value, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status.clear();
        let runtime = self.runtime.clone();
        let path = runtime.content.read(cx).workspace_path.clone();
        let before = runtime.content.read(cx).workspace["tickers"].clone();
        cx.spawn(async move |this, cx| {
            let args = match method {
                "selectPresentationTextFile" => vec![],
                "getTickerStatus" => vec![json!(path)],
                _ => vec![json!(path), payload],
            };
            let result = call(&runtime, cx, method, args, method == "saveTickers").await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Err(e) => this.status = e,
                    Ok(v) => match method {
                        "saveTickers" => {
                            if entries(&v["workspace"]["tickers"]).is_empty()
                                && !v["workspace"]["tickers"].is_array()
                            {
                                this.status = "ticker.invalid".into();
                            } else {
                                this.original = entries(&v["workspace"]["tickers"])
                                    .into_iter()
                                    .find(|v| v["id"] == this.form.draft["id"])
                                    .unwrap_or(Value::Null);
                            }
                        }
                        "selectPresentationTextFile" => {
                            if v["canceled"] != true {
                                if !text(&v, "filePath").to_lowercase().ends_with(".txt") {
                                    this.status = "ticker.textFileRequired".into();
                                } else if v["text"]
                                    .as_str()
                                    .is_none_or(|s| s.trim().is_empty() || s.len() > 65536)
                                {
                                    this.status = "ticker.invalid".into();
                                } else if runtime.content.read(cx).workspace["tickers"] != before {
                                    this.status = "ticker.retired".into();
                                } else {
                                    this.form.set("text", v["text"].clone());
                                    this.form.refresh();
                                }
                            }
                        }
                        _ => {
                            this.live = v;
                            if let Some(e) = this.live["sourceError"].as_str() {
                                this.status = e.into();
                            }
                        }
                    },
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn save(&mut self, delete: bool, cx: &mut Context<Self>) {
        let mut draft = self.form.draft.clone();
        if let Some(object) = draft.as_object_mut() {
            object.remove("seconds");
        }
        match replacement(
            &self.runtime.content.read(cx).workspace["tickers"],
            &self.original,
            &draft,
            delete,
        ) {
            Ok(v) => self.run("saveTickers", v, cx),
            Err(e) => {
                self.status = e;
                cx.notify();
            }
        }
    }
}
impl Render for Tickers {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Runtime::translator(cx);
        let gap = cx.theme().metrics.space_xs;
        let collection = self.runtime.content.read(cx).workspace["tickers"].clone();
        let mut body = div()
            .id("tickers-body")
            .flex()
            .flex_col()
            .gap(gap)
            .max_h(window.viewport_size().height - cx.theme().metrics.row_height * 6.)
            .overflow_y_scroll();
        for (i, v) in entries(&collection).into_iter().enumerate() {
            body = body.child(
                ActionButton::new(SharedString::from(format!("ticker-{i}")), text(&v, "title"))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.original = v.clone();
                        this.form.reset(v.clone());
                        cx.notify();
                    })),
            );
        }
        for (id, key) in [
            ("new", "ticker.new"),
            ("duplicate", "common.duplicate"),
            ("import", "ticker.importText"),
            ("save", "common.save"),
            ("delete", "common.delete"),
        ] {
            body = body.child(
                ActionButton::new(id, t.t(key))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        match id {
                            "new" => this.new_draft(),
                            "duplicate" => {
                                let saved =
                                    entries(&this.runtime.content.read(cx).workspace["tickers"])
                                        .into_iter()
                                        .find(|v| v["id"] == this.form.draft["id"]);
                                if let Some(mut saved) = saved {
                                    saved["id"] = json!(uuid::Uuid::new_v4().to_string());
                                    this.original = Value::Null;
                                    this.form.reset(saved);
                                } else {
                                    this.status = "ticker.invalid".into();
                                }
                            }
                            "import" => this.run("selectPresentationTextFile", Value::Null, cx),
                            "save" => this.save(false, cx),
                            _ => this.save(true, cx),
                        }
                        cx.notify();
                    })),
            );
        }
        let source = text(&self.form.draft["source"], "type");
        let options = [
            ("", "ticker.text"),
            ("file", "ticker.file"),
            ("rss", "ticker.rssUrl"),
        ]
        .map(|(v, k)| (v.to_string(), t.t(k)))
        .to_vec();
        fn source_change(form: &mut Form, _: &str, value: &str) {
            form.set(
                "source",
                match value {
                    "file" => json!({"type":"file","path":""}),
                    "rss" => json!({"type":"rss","url":"","format":"title","delimiter":" | "}),
                    _ => Value::Null,
                },
            );
        }
        let mut specs = vec![
            Spec::new("title", t.t("ticker.name"), Kind::Text),
            Spec::new("text", t.t("ticker.text"), Kind::Multiline),
            Spec::new(
                "source",
                t.t("ticker.source"),
                Kind::Hook(options, source_change, source.clone()),
            ),
            Spec::new("speed", t.t("ticker.speed"), Kind::Number),
            Spec::new(
                "direction",
                t.t("ticker.direction"),
                Kind::Choice(
                    ["left", "right", "up", "down"]
                        .iter()
                        .map(|v| (v.to_string(), t.t(&format!("ticker.{v}"))))
                        .collect(),
                ),
            ),
            Spec::new("repeat", t.t("ticker.repeat"), Kind::Check),
            Spec::new(
                "loopDelaySeconds",
                t.t("ticker.loopDelaySeconds"),
                Kind::Number,
            ),
            Spec::new(
                "startPosition",
                t.t("ticker.delayStartsOffscreen"),
                Kind::Choice(vec![
                    ("automatic".into(), t.t("common.automatic")),
                    ("offscreen".into(), t.t("ticker.delayStartsOffscreen")),
                ]),
            ),
            Spec::new(
                "repeatDistance",
                t.t("ticker.overrideDistance"),
                Kind::Number,
            ),
            Spec::new("seconds", t.t("ticker.seconds"), Kind::Number),
        ];
        if source == "file" {
            specs.push(Spec::new("source.path", t.t("ticker.filePath"), Kind::Text));
        } else if source == "rss" {
            specs.extend([
                Spec::new("source.url", t.t("ticker.rssUrl"), Kind::Text),
                Spec::new(
                    "source.format",
                    t.t("ticker.rssFormat"),
                    Kind::Choice(vec![
                        ("title".into(), t.t("ticker.rssTitle")),
                        ("titleDescription".into(), t.t("ticker.rssFull")),
                    ]),
                ),
                Spec::new("source.delimiter", t.t("ticker.rssDelimiter"), Kind::Text),
            ]);
        }
        for spec in specs {
            body = body.child(self.form.field("ticker", &spec, form, window, cx));
        }
        for (action, key) in [
            ("play", "transport.play"),
            ("pause", "transport.pause"),
            ("resume", "transport.play"),
            ("clear", "common.clear"),
            ("seek", "ticker.seek"),
            ("status", "common.refresh"),
        ] {
            body=body.child(ActionButton::new(action,t.t(key)).disabled(self.busy).on_click(cx.listener(move|this,_,_,cx|{let payload=if action=="play" {json!({"type":action,"id":this.form.draft["id"]})}else if action=="seek"{json!({"type":action,"generation":this.live["generation"],"seconds":this.form.draft["seconds"]})}else{json!({"type":action,"generation":this.live["generation"]})};this.run(if action=="status"{"getTickerStatus"}else{"controlTicker"},payload,cx)})));
        }
        // Position is native; never synthesize it from the dialog's lifetime.
        body = body.child(div().child(format!(
            "{} {}",
            text(&self.live, "title"),
            self.live["elapsedSeconds"]
        )));
        let close = self.close.clone();
        Dialog::new(
            "tickers",
            t.t("ticker.title"),
            t.t("common.close"),
            move |_, _, cx| close(cx),
        )
        .child(
            body.child(
                div()
                    .id("ticker-error")
                    .role(gpui::Role::Alert)
                    .child(t.t(&self.status)),
            ),
        )
    }
}
