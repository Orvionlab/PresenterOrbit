use super::{
    catalog,
    content_document::DocumentKind,
    editing,
    view::{History, SlideCanvas},
};
use gpui::{div, prelude::*, AnyElement, Context, Entity, Subscription, Window};
use orbit_ui::{
    controls::{
        ActionButton, ButtonVariant, Disclosure, RichText, RichTextEvent, RunDocument, Select,
        SelectOption,
    },
    ActiveTheme, Translator,
};
use serde_json::{json, Value};
use std::sync::Arc;

pub struct RichPanel {
    text: Entity<RichText>,
    notes: Entity<RichText>,
    text_revision: u64,
    notes_revision: u64,
    corpus_generation: Option<u64>,
    _subscriptions: Vec<Subscription>,
}
impl SlideCanvas {
    pub(super) fn rich_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.kind != DocumentKind::Slide || self.document.is_none() {
            return None;
        }
        let t = *cx.global::<Translator>();
        if self.rich.is_none() {
            let text =
                self.slide.get("contentRuns").cloned().unwrap_or_else(
                    || json!([{"text":self.slide["content"].as_str().unwrap_or("")}]),
                );
            let notes =
                self.slide.get("notesRuns").cloned().unwrap_or_else(
                    || json!([{"text":self.slide["notes"].as_str().unwrap_or("")}]),
                );
            let (text, notes) = match (RunDocument::new(text), RunDocument::new(notes)) {
                (Ok(text), Ok(notes)) => (text, notes),
                (Err(error), _) | (_, Err(error)) => {
                    self.status = error;
                    return None;
                }
            };
            let text =
                cx.new(|cx| RichText::new(text, t.t("slideEditor.slideText"), false, window, cx));
            let notes = cx.new(|cx| {
                RichText::new(notes, t.t("theme.linkedText.slideNotes"), true, window, cx)
            });
            let mut subscriptions = Vec::new();
            for (notes_field, entity) in [(false, &text), (true, &notes)] {
                let key = self.document.as_ref().unwrap().key();
                subscriptions.push(cx.subscribe(
                    entity,
                    move |view, _, event: &RichTextEvent, cx| {
                        if view
                            .document
                            .as_ref()
                            .is_none_or(|document| document.key() != key)
                        {
                            return;
                        }
                        match event {
                            RichTextEvent::Change(runs) => {
                                view.rich_changed(notes_field, runs.clone(), false, cx)
                            }
                            RichTextEvent::Commit(runs) => {
                                view.rich_changed(notes_field, runs.clone(), true, cx)
                            }
                        }
                    },
                ));
            }
            self.rich = Some(RichPanel {
                text,
                notes,
                text_revision: 0,
                notes_revision: 0,
                corpus_generation: None,
                _subscriptions: subscriptions,
            });
        }
        let generation = self.runtime.content.read(cx).items_generation;
        let rich = self.rich.as_mut().unwrap();
        if rich.corpus_generation != Some(generation) {
            rich.corpus_generation = Some(generation);
            let corpus = self.runtime.content.read(cx).workspace["library"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|item| item["slides"].as_array().into_iter().flatten())
                .filter_map(|slide| slide["content"].as_str().map(str::to_string))
                .collect();
            rich.text
                .update(cx, |view, cx| view.set_spelling_corpus(corpus, cx));
        }
        let text = rich.text.clone();
        let notes = rich.notes.clone();
        for field in [&text, &notes] {
            field.update(cx, |view, cx| {
                view.set_titled(false, cx);
                view.set_disabled(self.saving, cx);
            });
        }
        let theme = *cx.theme();
        let (text_owner, notes_owner) = (cx.weak_entity(), cx.weak_entity());
        let mut actions = div().flex().flex_wrap().gap(theme.metrics.space_xs);
        for (action, key) in [
            ("undo", "slideEditor.undo"),
            ("redo", "slideEditor.redo"),
            ("restore", "slideEditor.restoreTheme"),
            ("reflow", "toolbar.reflow"),
            ("show", "slideEditor.show"),
            ("actions", "automation.slideActions.title"),
        ] {
            actions = actions.child(
                ActionButton::new(action, t.t(key))
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.saving)
                    .on_click(cx.listener(move |view, _, _, cx| view.slide_action(action, cx))),
            );
        }
        Some(
            div()
                .flex_none()
                .max_h(theme.metrics.row_height * 12.)
                .id("slide-rich-fields")
                .overflow_y_scroll()
                .border_t_1()
                .border_color(theme.colors.border)
                .child(actions)
                .child(
                    Disclosure::new(
                        "slide-text-panel",
                        t.t("slideEditor.slideText"),
                        !self.editor.text_closed,
                    )
                    .on_toggle(move |open, _, cx| {
                        text_owner
                            .update(cx, |view, cx| {
                                view.editor.text_closed = !open;
                                cx.notify();
                            })
                            .ok();
                    })
                    .child(text),
                )
                .child(
                    Disclosure::new(
                        "slide-notes-panel",
                        t.t("theme.linkedText.slideNotes"),
                        !self.editor.notes_closed,
                    )
                    .on_toggle(move |open, _, cx| {
                        notes_owner
                            .update(cx, |view, cx| {
                                view.editor.notes_closed = !open;
                                cx.notify();
                            })
                            .ok();
                    })
                    .child(notes)
                    .child(self.note_paragraph_panel(window, cx)),
                )
                .into_any_element(),
        )
    }
    /// `NoteParagraphControls`: the paragraph styles of the slide notes, one paragraph at a time.
    fn note_paragraph_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Disclosure {
        let t = *cx.global::<Translator>();
        let count = editing::paragraphs(self.slide["notes"].as_str().unwrap_or("")).len();
        self.editor.note_paragraph = self.editor.note_paragraph.min(count - 1);
        let selected = self.editor.note_paragraph;
        let weak = cx.weak_entity();
        let options = (0..count)
            .map(|index| {
                SelectOption::new(
                    index.to_string(),
                    format!("{} {}", t.t("theme.text.paragraph"), index + 1),
                )
            })
            .collect();
        let mut body = div()
            .flex()
            .flex_col()
            .gap(cx.theme().metrics.space_xs)
            .child(
                Select::new("note-paragraph", options, selected.to_string())
                    .label(t.t("theme.text.paragraph"))
                    .on_change(move |value: String, _, cx| {
                        weak.update(cx, |view, cx| {
                            if let Ok(index) = value.parse::<usize>() {
                                view.editor.note_paragraph = index;
                                view.editor.scope.clear();
                                cx.notify();
                            }
                        })
                        .ok();
                    }),
            );
        if self.editor.note_open {
            for spec in catalog::note_fields(&self.slide, selected) {
                body = body.child(self.inspector_field(spec, window, cx));
            }
        }
        let weak = cx.weak_entity();
        Disclosure::new(
            "note-paragraphs",
            t.t("theme.text.paragraphScope"),
            self.editor.note_open,
        )
        .on_toggle(move |open, _, cx| {
            weak.update(cx, |view, cx| {
                view.editor.note_open = open;
                cx.notify();
            })
            .ok();
        })
        .child(body)
    }
    pub(super) fn flush_rich(&mut self, cx: &mut Context<Self>) {
        let Some(rich) = &self.rich else {
            return;
        };
        let changes = [
            (
                false,
                rich.text.read(cx).revision() > rich.text_revision,
                rich.text.read(cx).value(),
            ),
            (
                true,
                rich.notes.read(cx).revision() > rich.notes_revision,
                rich.notes.read(cx).value(),
            ),
        ];
        for (notes, changed, runs) in changes {
            if changed {
                self.rich_changed(notes, runs, false, cx);
            }
        }
    }
    fn rich_changed(&mut self, notes: bool, runs: Value, commit: bool, cx: &mut Context<Self>) {
        if self.saving || self.kind != DocumentKind::Slide {
            return;
        }
        match orbit_host::authoring::text_commit(&self.slide, runs, notes) {
            Ok(slide) => {
                if slide != *self.slide {
                    if self.editor.base.is_none() {
                        self.editor.base = Some(self.slide.clone());
                        self.redo.clear();
                    }
                    self.editor.edit_nonce += 1;
                    self.set_slide(Arc::new(slide));
                    self.request_raster(false, cx);
                }
                if let Some(rich) = &mut self.rich {
                    if notes {
                        rich.notes_revision = rich.notes.read(cx).revision();
                    } else {
                        rich.text_revision = rich.text.read(cx).revision();
                    }
                }
                if commit {
                    self.flush_rich(cx);
                    if self.editor.base.is_some() {
                        self.save((*self.slide).clone(), History::Edit, cx);
                    }
                }
            }
            Err(error) => self.status = error,
        }
        cx.notify();
    }
    pub(super) fn slide_action(&mut self, action: &str, cx: &mut Context<Self>) {
        self.flush_rich(cx);
        if self.saving {
            return;
        }
        match action {
            "undo" => {
                if let Some(base) = self.editor.base.take() {
                    self.redo.push(self.slide.clone());
                    self.set_slide(base);
                    self.rich = None;
                    self.selected = None;
                    self.editor.reset();
                    self.request_raster(false, cx);
                } else if let Some(slide) = self.undo.last() {
                    self.save((**slide).clone(), History::Undo, cx);
                }
            }
            "redo" => {
                if let Some(slide) = self.redo.last() {
                    self.save((**slide).clone(), History::Redo, cx);
                }
            }
            "restore" => {
                let content = self.runtime.content.read(cx);
                let source = content
                    .items
                    .get(&self.theme_id)
                    .and_then(|item| item.value["slides"].as_array())
                    .and_then(|slides| slides.iter().find(|slide| slide["id"] == self.slide_id));
                if let Some(source) = source {
                    let theme = orbit_host::authoring::slide_theme(&content.workspace, source);
                    let restored =
                        orbit_host::authoring::store(source, &self.slide, theme.as_ref()).and_then(
                            |mut stored| {
                                let fields = stored
                                    .as_object_mut()
                                    .ok_or("The stored slide must be an object.")?;
                                fields.remove("themeLayerOverrides");
                                fields.remove("themeLayerOrder");
                                orbit_host::authoring::compose(&stored, theme)
                            },
                        );
                    match restored {
                        Ok(next) => {
                            self.rich = None;
                            self.draft(next, cx);
                        }
                        Err(error) => self.status = error,
                    }
                }
            }
            "show" | "reflow" | "actions" => {
                self.editor.after_slide_action = Some(action.to_string());
                if self.editor.base.is_some() {
                    self.save((*self.slide).clone(), History::Edit, cx);
                } else {
                    self.finish_slide_action(cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }
    pub(super) fn finish_slide_action(&mut self, cx: &mut Context<Self>) {
        let Some(action) = self.editor.after_slide_action.take() else {
            return;
        };
        if action == "actions" {
            cx.emit(super::view::CanvasEvent::SlideActions {
                item: self.theme_id.clone(),
                slide: self.slide_id.clone(),
            });
        } else if action == "show" {
            let raw = self
                .runtime
                .content
                .read(cx)
                .items
                .get(&self.theme_id)
                .and_then(|item| item.value["slides"].as_array())
                .and_then(|slides| slides.iter().position(|slide| slide["id"] == self.slide_id));
            if let Some(index) = raw {
                crate::cues::take_editor_slide(&self.runtime, &self.theme_id, index, cx);
            }
        } else {
            cx.emit(super::view::CanvasEvent::Reflow {
                item: self.theme_id.clone(),
                slide: self.slide_id.clone(),
            });
        }
    }
}
