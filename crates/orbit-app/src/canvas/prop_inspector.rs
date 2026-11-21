use std::rc::Rc;

use gpui::{div, prelude::*, AnyElement, Context, Window};
use orbit_ui::{
    controls::{input_state, ActionButton, ButtonVariant, Dialog, Field, Select, SelectOption},
    ActiveTheme, Translator,
};
use serde_json::{json, Value};

use super::{content_document::DocumentKind, documents_ui::DocumentOperation, view::SlideCanvas};
use crate::{automation::props::AUTO_CLEAR_SECONDS, transitions};

impl SlideCanvas {
    pub(super) fn prop_options(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let prop = self.root_document(cx)?;
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let disabled = self.document_controls_disabled();
        let label = t.t("props.autoClear.title");
        let after = |seconds: &str| t.format("props.autoClear.after", &[("seconds", seconds)]);
        let mut options = vec![SelectOption::new(
            String::new(),
            t.t("props.autoClear.never"),
        )];
        options.extend(AUTO_CLEAR_SECONDS.map(|seconds| {
            let value = seconds.to_string();
            SelectOption::new(value.clone(), after(&value))
        }));
        let selected = prop["autoClearSeconds"]
            .as_f64()
            .map(|n| n.to_string())
            .unwrap_or_default();
        if !options.iter().any(|option| option.value == selected) {
            options.push(SelectOption::new(selected.clone(), after(&selected)));
        }
        options.push(SelectOption::new(
            "custom".into(),
            t.t("cue.autoAdvance.custom"),
        ));
        let weak = cx.weak_entity();
        let id = self.theme_id.clone();
        let workspace = self.workspace.clone();
        let auto_clear = Select::new("prop-editor-auto-clear", options, selected.clone())
            .label(label.clone())
            .compact()
            .disabled(disabled)
            .on_change(move |value, window, cx| {
                weak.update(cx, |view, cx| {
                    if view.kind != DocumentKind::Prop
                        || view.theme_id != id
                        || view.workspace != workspace
                    {
                        return;
                    }
                    match value.as_str() {
                        "custom" => {
                            let initial = if selected.is_empty() { "5" } else { &selected };
                            let input = input_state(initial.to_string(), window, cx);
                            input.update(cx, |state, cx| {
                                state.focus(window, cx);
                                state.select_all(window, cx);
                            });
                            view.editor.dialog = Some(("prop-auto-clear".into(), input));
                            cx.notify();
                        }
                        "" => view.set_prop_auto_clear(None, cx),
                        seconds => {
                            if let Ok(seconds) = seconds.parse() {
                                view.set_prop_auto_clear(Some(seconds), cx);
                            }
                        }
                    }
                })
                .ok();
            });
        let separator = || {
            div()
                .h(theme.metrics.space_2xs / 2.)
                .bg(theme.colors.border_soft)
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(theme.metrics.space_sm)
                .child(separator())
                .child(t.t("transition.title"))
                .child(
                    ActionButton::new(
                        "prop-editor-transition",
                        t.t("gpui.editor.chooseTransition"),
                    )
                    .variant(ButtonVariant::Secondary)
                    .compact()
                    .disabled(disabled)
                    .on_click(
                        cx.listener(|view, _, window, cx| view.open_prop_transition(window, cx)),
                    ),
                )
                .child(separator())
                .child(
                    div()
                        .debug_selector(|| "prop-editor-auto-clear-field".into())
                        .child(
                            Field::new(label, auto_clear)
                                .inline()
                                .label_width(theme.metrics.space_lg * 4.),
                        ),
                )
                .into_any_element(),
        )
    }

    fn open_prop_transition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let runtime = self.runtime.clone();
        let owner = cx.weak_entity();
        let id = self.theme_id.clone();
        let target = id.clone();
        let workspace = self.workspace.clone();
        self.editor.prop_transition = Some(cx.new(|cx| {
            let mut editor = transitions::TransitionDefaults::new(transitions::Kind::Props, runtime, cx);
            editor.edit_prop(id, Rc::new(move |transition, cx| {
                owner.update(cx, |view, cx| {
                    if view.kind == DocumentKind::Prop && view.theme_id == target && view.workspace == workspace {
                        view.update_prop_option(json!({"type":"setTransition","propId":target,"transition":transition}), cx);
                    }
                }).ok();
            }), window, cx);
            editor
        }));
        cx.notify();
    }

    pub(super) fn prop_transition_dialog(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let editor = self.editor.prop_transition.clone()?;
        let t = *cx.global::<Translator>();
        let owner = cx.weak_entity();
        Some(
            Dialog::new(
                "canvas-prop-transition-editor",
                t.t("transition.picker.title"),
                t.t("common.close"),
                move |_, _, cx| {
                    owner
                        .update(cx, |view, cx| {
                            view.editor.prop_transition = None;
                            cx.notify();
                        })
                        .ok();
                },
            )
            .width(transitions::TransitionDefaults::picker_width(window))
            .flush()
            .child(editor)
            .into_any_element(),
        )
    }

    pub(super) fn set_prop_auto_clear(&mut self, seconds: Option<f64>, cx: &mut Context<Self>) {
        self.update_prop_option(
            json!({"type":"setAutoClear","propId":self.theme_id,"seconds":seconds}),
            cx,
        );
    }

    fn update_prop_option(&mut self, mutation: Value, cx: &mut Context<Self>) {
        self.run_document_operation(
            DocumentOperation {
                method: "mutateProps",
                mutation,
                created: None,
            },
            cx,
        );
    }
}
