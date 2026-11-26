use super::{
    input_state, rich_text_model::RunDocument, ActionButton, ButtonVariant, CheckboxField,
    ColorPicker, Disclosure, Field, InputEvent, InputState, NumberInput, PanelHeader, Select,
    SelectOption, TextInput,
};
use crate::{ActiveTheme, Translator};
use gpui::{
    div, prelude::*, px, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    FontStyle, FontWeight, HighlightStyle, Render, SharedString, StrikethroughStyle, Subscription,
    UnderlineStyle, Window,
};
use gpui_base::input::{
    Editor, EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Redo,
    Rope, Undo,
};
use serde_json::{json, Value};
use std::{cell::RefCell, ops::Range, rc::Rc};

struct Buffer {
    document: RunDocument,
    before: Option<RunDocument>,
    edit: Option<(usize, usize, usize)>,
    error: String,
}
struct Highlights(Rc<RefCell<Buffer>>);
impl InputHighlighter for Highlights {
    fn language(&self) -> SharedString {
        "orbit-rich-text".into()
    }
    fn update(
        &mut self,
        edit: Option<InputEdit>,
        text: &Rope,
        _: bool,
        _: &mut Window,
        _: &mut Context<EditorState>,
    ) {
        let mut buffer = self.0.borrow_mut();
        let Some(edit) = edit else {
            return;
        };
        let text = text.to_string();
        if buffer.document.text() == text {
            return;
        }
        if buffer.before.is_none() {
            buffer.before = Some(buffer.document.clone());
        }
        let result = text
            .get(edit.start_byte..edit.new_end_byte)
            .ok_or("Invalid input edit.")
            .and_then(|inserted| {
                buffer.edit = Some((edit.start_byte, edit.old_end_byte, inserted.chars().count()));
                buffer
                    .document
                    .replace(edit.start_byte..edit.old_end_byte, inserted)
                    .map_err(|_| "Invalid rich text edit.")
            });
        if let Err(error) = result {
            buffer.error = error.into();
        }
    }
    fn styles(
        &self,
        range: &Range<usize>,
        _: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let buffer = self.0.borrow();
        let mut offset = 0;
        let mut result = Vec::new();
        for run in &buffer.document.runs {
            let end = offset + run["text"].as_str().unwrap().len();
            let start = range.start.max(offset);
            let stop = range.end.min(end);
            if start < stop {
                let style = &run["style"];
                result.push((
                    start..stop,
                    HighlightStyle {
                        color: style["textColor"]
                            .as_str()
                            .and_then(super::rich_text_model::run_color)
                            .map(|color| {
                                let mut rgba = super::data_color(&color);
                                rgba.a = f32::from(super::hex_alpha(&color)) / 255.;
                                rgba.into()
                            }),
                        font_weight: (style["bold"] == true).then_some(FontWeight::BOLD),
                        font_style: (style["italic"] == true).then_some(FontStyle::Italic),
                        underline: (style["underline"] == true).then_some(UnderlineStyle {
                            thickness: px(1.),
                            color: None,
                            wavy: false,
                        }),
                        strikethrough: (style["strikethrough"] == true).then_some(
                            StrikethroughStyle {
                                thickness: px(1.),
                                color: None,
                            },
                        ),
                        ..Default::default()
                    },
                ));
            }
            offset = end;
        }
        result
    }
    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}
pub enum RichTextEvent {
    Change(Value),
    Commit(Value),
}
#[derive(Clone)]
struct Snapshot {
    document: RunDocument,
    selection: Range<usize>,
}
pub struct RichText {
    input: Entity<EditorState>,
    buffer: Rc<RefCell<Buffer>>,
    label: String,
    titled: bool,
    notes: bool,
    focus: FocusHandle,
    font: Entity<InputState>,
    size: Entity<InputState>,
    spacing: Entity<InputState>,
    decoration_open: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    composition: Option<Snapshot>,
    last_typing: Option<usize>,
    committed: Value,
    disabled: bool,
    spelling_words: std::sync::Arc<Vec<String>>,
    spelling_target: Option<(Range<usize>, String)>,
    spelling_suggestions: Vec<String>,
    spelling_request: u64,
    spelling_corpus_request: u64,
    revision: u64,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<RichTextEvent> for RichText {}
impl RichText {
    /// `NoteDecorationControls`: superscript, letter spacing and the underline and strikethrough variants of the
    /// selection. The shown values are those of the selection's first character.
    fn decoration_controls(&self, t: Translator, cx: &mut Context<Self>) -> Disclosure {
        let theme = *cx.theme();
        let start = self.input.read(cx).selected_range().start;
        let style = self
            .buffer
            .borrow()
            .document
            .style_at(start)
            .cloned()
            .unwrap_or_else(|| json!({}));
        let weak = cx.weak_entity();
        let choice = |key: &str, value: &str| SelectOption::new(value.to_string(), t.t(key));
        let mut body = div().flex().flex_col().gap(theme.metrics.space_xs);
        body = body.child(Field::new(
            t.t("theme.text.superscript"),
            Select::new(
                "note-superscript",
                vec![
                    choice("theme.option.none", "0"),
                    choice("theme.text.superscript.super", "1"),
                    choice("theme.text.superscript.sub", "-1"),
                ],
                style["superscript"].as_i64().unwrap_or(0).to_string(),
            )
            .compact()
            .disabled(self.disabled)
            .on_change({
                let weak = weak.clone();
                move |value: String, _, cx| {
                    if let Ok(level) = value.parse::<i64>() {
                        weak.update(cx, |view, cx| view.format("superscript", json!(level), cx))
                            .ok();
                    }
                }
            }),
        ));
        body = body.child(Field::new(
            t.t("theme.text.letterSpacing"),
            NumberInput::new("note-letter-spacing", &self.spacing)
                .step(0.1)
                .disabled(self.disabled),
        ));
        for key in ["underline", "strikethrough"] {
            let field = |suffix: &str| format!("{key}{suffix}");
            let styles = ["single", "thick", "double"]
                .map(|value| choice(&format!("theme.text.decoration.{value}"), value));
            let patterns = ["solid", "dot", "dash", "dash-dot", "dash-dot-dot"]
                .map(|value| choice(&format!("theme.text.decoration.{value}"), value));
            let current = |suffix: &str, default: &str| {
                style[field(suffix)].as_str().unwrap_or(default).to_string()
            };
            body = body
                .child(t.t(&format!("theme.text.decoration.{key}")))
                .child(Field::new(
                    t.t("theme.text.style"),
                    Select::new(
                        SharedString::from(format!("note-{key}-style")),
                        styles.to_vec(),
                        current("Style", "single"),
                    )
                    .compact()
                    .disabled(self.disabled)
                    .on_change({
                        let weak = weak.clone();
                        let name = field("Style");
                        move |value: String, _, cx| {
                            let name = name.clone();
                            weak.update(cx, |view, cx| view.format(&name, json!(value), cx))
                                .ok();
                        }
                    }),
                ))
                .child(Field::new(
                    t.t("theme.text.decoration.pattern"),
                    Select::new(
                        SharedString::from(format!("note-{key}-pattern")),
                        patterns.to_vec(),
                        current("Pattern", "solid"),
                    )
                    .compact()
                    .disabled(self.disabled)
                    .on_change({
                        let weak = weak.clone();
                        let name = field("Pattern");
                        move |value: String, _, cx| {
                            let name = name.clone();
                            weak.update(cx, |view, cx| view.format(&name, json!(value), cx))
                                .ok();
                        }
                    }),
                ))
                .child(
                    CheckboxField::new(
                        SharedString::from(format!("note-{key}-by-word")),
                        t.t("theme.text.decoration.byWord"),
                        style[field("ByWord")] == true,
                    )
                    .disabled(self.disabled)
                    .on_change({
                        let weak = weak.clone();
                        let name = field("ByWord");
                        move |value, _, cx| {
                            let name = name.clone();
                            weak.update(cx, |view, cx| view.format(&name, json!(value), cx))
                                .ok();
                        }
                    }),
                )
                .child(
                    ColorPicker::new(
                        SharedString::from(format!("note-{key}-color")),
                        t.t("theme.color"),
                        style[field("Color")]
                            .as_str()
                            .or_else(|| style["textColor"].as_str())
                            .unwrap_or("#ffffff")
                            .to_string(),
                    )
                    .compact()
                    .disabled(self.disabled)
                    .on_change({
                        let weak = weak.clone();
                        let name = field("Color");
                        move |value, _, cx| {
                            let name = name.clone();
                            weak.update(cx, |view, cx| view.format(&name, json!(value), cx))
                                .ok();
                        }
                    }),
                );
        }
        Disclosure::new(
            "note-decoration",
            t.t("theme.text.decoration"),
            self.decoration_open,
        )
        .on_toggle(move |open, _, cx| {
            weak.update(cx, |view, cx| {
                view.decoration_open = open;
                cx.notify();
            })
            .ok();
        })
        .child(body)
    }
    pub fn new(
        document: RunDocument,
        label: String,
        notes: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let committed = document.value(notes);
        let text = document.text();
        let buffer = Rc::new(RefCell::new(Buffer {
            document,
            before: None,
            edit: None,
            error: String::new(),
        }));
        let shared = buffer.clone();
        let input = cx.new(|cx| {
            let mut input = EditorState::new(window, cx)
                .default_value(text)
                .language("orbit-rich-text")
                .line_number(false)
                .auto_close(false)
                .smart_indent(false)
                .soft_wrap(true)
                .folding(false);
            input.set_highlighter_factory(
                Rc::new(move |_: &str| {
                    Some(Box::new(Highlights(shared.clone())) as Box<dyn InputHighlighter>)
                }),
                cx,
            );
            input.set_searchable(false, cx);
            input.set_accessibility(
                gpui_base::input::InputAccessibility::new(None)
                    .with_label(Some(label.clone().into())),
            );
            input
        });
        let font = input_state("", window, cx);
        let size = input_state("16", window, cx);
        let spacing = input_state("0", window, cx);
        let focus = cx.focus_handle();
        let handle = window.window_handle();
        let subscriptions = vec![
            cx.observe(&input, |_, _, cx| cx.notify()),
            cx.subscribe(&input, move |view, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let marked = handle
                        .update(cx, |_, window, cx| {
                            input.update(cx, |input, cx| {
                                input.marked_text_range(window, cx).is_some()
                            })
                        })
                        .unwrap_or(false);
                    view.changed(marked, cx);
                }
            }),
            cx.on_focus_out(&focus, window, |view, _, _, cx| view.commit(cx)),
            cx.subscribe(&font, |view, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Blur) {
                    let value = state.read(cx).value().trim().to_string();
                    if !value.is_empty() {
                        view.format("fontFamily", json!(value), cx);
                    }
                }
            }),
            cx.subscribe(&spacing, |view, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    if let Ok(value) = state.read(cx).value().parse::<f64>() {
                        if value.is_finite() {
                            view.format("letterSpacing", json!(value), cx);
                        }
                    }
                }
            }),
            cx.subscribe(&size, |view, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    if let Ok(value) = state.read(cx).value().parse::<f64>() {
                        if value.is_finite() && (8. ..=200.).contains(&value) {
                            view.format("fontSize", json!(value), cx);
                        }
                    }
                }
            }),
        ];
        Self {
            input,
            buffer,
            label,
            titled: true,
            notes,
            focus,
            font,
            size,
            spacing,
            decoration_open: false,
            undo: Vec::new(),
            redo: Vec::new(),
            composition: None,
            last_typing: None,
            committed,
            disabled: false,
            spelling_words: Default::default(),
            spelling_target: None,
            spelling_suggestions: Vec::new(),
            spelling_request: 0,
            spelling_corpus_request: 0,
            revision: 0,
            _subscriptions: subscriptions,
        }
    }
    pub fn set_spelling_corpus(&mut self, texts: Vec<String>, cx: &mut Context<Self>) {
        self.spelling_request += 1;
        self.spelling_corpus_request += 1;
        let sequence = self.spelling_corpus_request;
        self.spelling_target = None;
        self.spelling_suggestions.clear();
        cx.spawn(async move |this, cx| {
            let words = cx
                .background_spawn(async move {
                    texts
                        .iter()
                        .flat_map(|text| {
                            super::spelling::words(text)
                                .into_iter()
                                .map(|(_, word)| word)
                        })
                        .filter(|word| word.chars().count() >= 3)
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.spelling_corpus_request == sequence {
                    this.spelling_words = std::sync::Arc::new(words);
                    this.spelling_target = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn replace_spelling(
        &mut self,
        range: Range<usize>,
        word: &str,
        suggestion: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled
            || self.composition.is_some()
            || self.buffer.borrow().document.text().get(range.clone()) != Some(word)
        {
            return;
        }
        self.input.update(cx, |input, cx| {
            input.set_selected_range(range, cx);
            input.replace_text_in_range(None, suggestion, window, cx);
        });
        self.commit(cx);
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn value(&self) -> Value {
        self.buffer.borrow().document.value(self.notes)
    }
    /// Whether the field shows its own title; an owner with its own heading hides it.
    pub fn set_titled(&mut self, titled: bool, cx: &mut Context<Self>) {
        if self.titled != titled {
            self.titled = titled;
            cx.notify();
        }
    }
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            self.input
                .update(cx, |input, cx| input.set_disabled(disabled, cx));
            cx.notify();
        }
    }
    fn changed(&mut self, marked: bool, cx: &mut Context<Self>) {
        let (before, edit) = {
            let mut buffer = self.buffer.borrow_mut();
            (buffer.before.take(), buffer.edit.take())
        };
        let Some(before) = before else {
            return;
        };
        let before = Snapshot {
            document: before,
            selection: edit
                .map(|(start, end, _)| start..end)
                .unwrap_or_else(|| self.input.read(cx).selected_range()),
        };
        if marked {
            if self.composition.is_none() {
                self.composition = Some(before);
            }
            self.last_typing = None;
        } else if let Some(base) = self.composition.take() {
            if base.document.text() == self.buffer.borrow().document.text() {
                self.buffer.borrow_mut().document = base.document;
            } else {
                self.undo.push(base);
            }
            self.last_typing = None;
        } else {
            let typing = edit.filter(|(start, end, count)| start == end && *count == 1);
            if typing.is_none_or(|(start, _, _)| self.last_typing != Some(start)) {
                self.undo.push(before);
            }
            self.last_typing = typing.map(|(start, _, _)| {
                self.buffer.borrow().document.text()[start..]
                    .chars()
                    .next()
                    .map_or(start, |c| start + c.len_utf8())
            });
        }
        self.redo.clear();
        self.revision += 1;
        cx.emit(RichTextEvent::Change(self.value()));
        cx.notify();
    }
    fn finish_composition(&mut self) {
        if let Some(before) = self.composition.take() {
            if before.document != self.buffer.borrow().document {
                self.undo.push(before);
            }
        }
        self.last_typing = None;
    }
    fn format(&mut self, field: &str, value: Value, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let range = self.input.read(cx).selected_range();
        if range.is_empty() {
            return;
        }
        self.finish_composition();
        let before = Snapshot {
            document: self.buffer.borrow().document.clone(),
            selection: range.clone(),
        };
        if matches!(field, "underline" | "strikethrough") {
            let other = if field == "underline" {
                "strikethrough"
            } else {
                "underline"
            };
            let enabled = self
                .buffer
                .borrow()
                .document
                .style_at(range.start)
                .is_some_and(|style| style[other] == true);
            let other_result =
                self.buffer
                    .borrow_mut()
                    .document
                    .format(range.clone(), other, json!(enabled));
            if let Err(error) = other_result {
                self.buffer.borrow_mut().error = error;
                return;
            }
        }
        let result = self
            .buffer
            .borrow_mut()
            .document
            .format(range, field, value);
        match result {
            Ok(()) => {
                self.undo.push(before);
                self.redo.clear();
                self.revision += 1;
                self.input.update(cx, |input, cx| input.refresh(cx));
                cx.emit(RichTextEvent::Change(self.value()));
                self.commit(cx);
            }
            Err(error) => self.buffer.borrow_mut().error = error,
        }
        cx.notify();
    }
    fn history(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.finish_composition();
        let target = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        let Some(target) = target else {
            return;
        };
        let selection = target.selection.clone();
        let current = Snapshot {
            document: std::mem::replace(&mut self.buffer.borrow_mut().document, target.document),
            selection: self.input.read(cx).selected_range(),
        };
        if redo {
            self.undo.push(current);
        } else {
            self.redo.push(current);
        }
        self.buffer.borrow_mut().before = None;
        let text = self.buffer.borrow().document.text();
        self.input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_selected_range(selection, cx);
        });
        self.revision += 1;
        cx.emit(RichTextEvent::Change(self.value()));
        cx.notify();
    }
    pub fn commit(&mut self, cx: &mut Context<Self>) {
        self.finish_composition();
        let next = self.value();
        if next != self.committed && self.buffer.borrow().error.is_empty() {
            self.committed = next.clone();
            cx.emit(RichTextEvent::Commit(next));
        }
    }
}
impl Render for RichText {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let mut tools = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme.metrics.space_2xs);
        for (field, key, value) in [
            ("bold", "slideEditor.bold", true),
            ("italic", "slideEditor.italic", true),
            ("underline", "slideEditor.underline", true),
            ("strikethrough", "slideEditor.strikethrough", true),
            ("bold", "slideEditor.normal", false),
        ] {
            if self.notes && field == "strikethrough" {
                continue;
            }
            tools = tools.child(
                ActionButton::new(SharedString::from(key), t.t(key))
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.disabled)
                    .on_click(
                        cx.listener(move |view, _, _, cx| view.format(field, json!(value), cx)),
                    ),
            );
        }
        let target = super::spelling::word_range(
            &self.buffer.borrow().document.text(),
            self.input.read(cx).selected_range(),
        );
        if target != self.spelling_target {
            self.spelling_target = target.clone();
            self.spelling_suggestions.clear();
            self.spelling_request += 1;
            let sequence = self.spelling_request;
            let words = self.spelling_words.clone();
            let word = target.as_ref().map(|(_, w)| w.clone());
            cx.spawn(async move |this, cx| {
                let suggestions = cx
                    .background_spawn(async move {
                        word.map_or_else(Vec::new, |word| {
                            super::spelling::suggestions(&words, &word)
                        })
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.spelling_request == sequence {
                        this.spelling_suggestions = suggestions;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        if let Some((range, word)) = target {
            for (index, suggestion) in self.spelling_suggestions.clone().into_iter().enumerate() {
                let range = range.clone();
                let word = word.clone();
                let label = suggestion.clone();
                tools = tools.child(
                    ActionButton::new(SharedString::from(format!("spelling-{index}")), label)
                        .variant(ButtonVariant::Ghost)
                        .disabled(self.disabled)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.replace_spelling(range.clone(), &word, &suggestion, window, cx)
                        })),
                );
            }
        }
        let weak = cx.weak_entity();
        tools = tools
            .child(
                Field::new(
                    t.t("slideEditor.font"),
                    div()
                        .w(theme.metrics.control_height * 4.)
                        .child(TextInput::new("rich-font", &self.font).disabled(self.disabled)),
                )
                .inline(),
            )
            .child(
                Field::new(
                    t.t("slideEditor.size"),
                    div().w(theme.metrics.control_height * 2.5).child(
                        NumberInput::new("rich-size", &self.size)
                            .range(8., 200.)
                            .disabled(self.disabled),
                    ),
                )
                .inline(),
            )
            .child(
                ColorPicker::new("rich-color", t.t("slideEditor.color"), "#ffffff")
                    .compact()
                    .disabled(self.disabled)
                    .on_change(move |color, _, cx| {
                        weak.update(cx, |view, cx| view.format("textColor", json!(color), cx))
                            .ok();
                    }),
            );
        let decoration = self.notes.then(|| self.decoration_controls(t, cx));
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .track_focus(&self.focus)
            .children(self.titled.then(|| PanelHeader::new(self.label.clone())))
            .child(tools)
            .children(decoration)
            .capture_key_down(cx.listener(|view, _, window, cx| {
                if view.composition.is_some()
                    && !view.input.update(cx, |input, cx| {
                        input.marked_text_range(window, cx).is_some()
                    })
                {
                    view.finish_composition();
                }
            }))
            .capture_action(cx.listener(|view, _: &Undo, window, cx| {
                if view.input.read(cx).focus_handle(cx).is_focused(window) {
                    view.history(false, window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|view, _: &Redo, window, cx| {
                if view.input.read(cx).focus_handle(cx).is_focused(window) {
                    view.history(true, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .h(theme.metrics.row_height * 5.)
                    .min_h_0()
                    .bg(theme.colors.field_bg)
                    .border_1()
                    .border_color(theme.colors.control_border)
                    .when(
                        self.input.read(cx).focus_handle(cx).is_focused(window),
                        |frame| super::focus_ring(frame, cx),
                    )
                    .child(Editor::new(&self.input)),
            )
            .when(!self.buffer.borrow().error.is_empty(), |body| {
                body.child(self.buffer.borrow().error.clone())
            })
    }
}
