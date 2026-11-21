use super::{
    catalog::{Field as Spec, Kind, Target},
    view::SlideCanvas,
};
use gpui::{div, prelude::*, AnyElement, Context, SharedString, Window};
use orbit_ui::{
    controls::{EmptyState, Field, IconButton, PanelHeader, Select, SelectOption},
    ActiveTheme, Translator,
};
use serde_json::{json, Value};

impl SlideCanvas {
    pub(super) fn document_inspector(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let mut body = div()
            .id("document-inspector-fields")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .p(theme.metrics.space_sm)
            .gap(theme.metrics.space_sm);
        let mut fields = self.document_fields(cx);
        macro_rules! append {
            ($keys:expr, $body:expr) => {{
                $keys.iter().fold($body, |body, key| {
                    if let Some(index) = fields.iter().position(|field| field.key == *key) {
                        body.child(self.inspector_field(fields.remove(index), window, cx))
                    } else {
                        body
                    }
                })
            }};
        }
        match self.document_tab() {
            "presentation" => {
                body = body.children(
                    self.root_document(cx)
                        .map(|root| self.background_field(Some(&root), cx)),
                );
                body = body.child(separator(cx));
                body = append!(&["presentation:size"], body);
                let mut widgets = self.document_tab_widgets(window, cx).into_iter();
                if let Some(transition) = widgets.next() {
                    body = body.child(separator(cx)).child(transition);
                }
                let display = self
                    .root_document(cx)
                    .is_some_and(|root| root["ccli"]["display"] != false);
                let weak = cx.weak_entity();
                body = body.child(separator(cx)).child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(PanelHeader::new(t.t("gpui.editor.copyright")))
                        .child(
                            orbit_ui::controls::CheckboxField::new(
                                "ccli-display",
                                t.t("gpui.editor.ccliDisplay"),
                                display,
                            )
                            .disabled(self.document_controls_disabled())
                            .on_change(move |value, _, cx| {
                                weak.update(cx, |view, cx| {
                                    view.patch_document_field(
                                        &Spec::new(
                                            "presentation:display",
                                            Target::Document,
                                            Kind::Bool,
                                            json!(display),
                                        ),
                                        &json!(value),
                                        cx,
                                    );
                                })
                                .ok();
                            }),
                        ),
                );
                body = append!(
                    &[
                        "ccli:title",
                        "presentation:artistCredits",
                        "ccli:author",
                        "presentation:album",
                        "ccli:publisher",
                        "ccli:songNumber",
                        "ccli:copyrightYear"
                    ],
                    body
                );
                body = body.child(self.artwork_preview(cx));
                if let Some(artwork) = widgets.next() {
                    body = body.child(artwork);
                }
            }
            "slide" => {
                body = append!(&["slideEnabled"], body).child(separator(cx));
                body = append!(&["slideShortcut"], body).child(separator(cx));
                let group = fields
                    .iter()
                    .position(|field| field.key == "slideGroup")
                    .map(|index| fields.remove(index));
                let color = fields
                    .iter()
                    .position(|field| field.key == "labelColor")
                    .map(|index| fields.remove(index));
                if let (Some(group), Some(color)) = (group, color) {
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(theme.metrics.space_xs)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.inspector_field(group, window, cx)),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .w(theme.metrics.control_height * 2.)
                                    .child(self.inspector_field(color, window, cx)),
                            ),
                    );
                }
                body = body.child(separator(cx));
                body = body.child(self.background_field(None, cx));
                if let Some(chart) = self.document_tab_widgets(window, cx).into_iter().next() {
                    body = body.child(separator(cx)).child(chart);
                }
                for kind in ["media", "video-input"] {
                    body = body
                        .child(separator(cx))
                        .child(self.document_action_section(kind, window, cx));
                }
            }
            "build" => {
                let transition = match &self.editor.slide_transition {
                    Some(transition) => transition.clone(),
                    None => {
                        let runtime = self.runtime.clone();
                        let transition = cx.new(|cx| {
                            crate::transitions::TransitionDefaults::new(
                                crate::transitions::Kind::Slide,
                                runtime,
                                cx,
                            )
                        });
                        self.editor.slide_transition = Some(transition.clone());
                        transition
                    }
                };
                let preview = self.slide["transition"].clone();
                let expected = (self.theme_id.clone(), self.slide_id.clone());
                let owner = cx.weak_entity();
                transition.update(cx, |transition, _| {
                    transition.preview = Some(preview);
                    transition.on_change = Some(std::rc::Rc::new(move |value, cx| {
                        owner
                            .update(cx, |view, cx| {
                                if (view.theme_id.clone(), view.slide_id.clone()) == expected {
                                    view.set_inspector_transition(value, true, cx);
                                }
                            })
                            .ok();
                    }));
                    transition.item_id = Some(self.theme_id.clone());
                    transition.slide_id = Some(self.slide_id.clone());
                    transition.inspector = true;
                });
                body = body.child(transition);
            }
            _ => {}
        }
        body.into_any_element()
    }

    fn document_action_section(
        &mut self,
        kind: &'static str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let label = t.t(if kind == "media" {
            "gpui.editor.mediaAction"
        } else {
            "gpui.editor.videoInputAction"
        });
        let weak = cx.weak_entity();
        let mut section = div().flex().flex_col().gap(theme.metrics.space_xs).child(
            PanelHeader::new(label.clone()).action(
                IconButton::new(
                    SharedString::from(format!("add-{kind}-action")),
                    "plus",
                    label.clone(),
                )
                .disabled(self.document_controls_disabled())
                .on_click(move |_, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.editor.action_picker = if view.editor.action_picker == kind {
                            String::new()
                        } else {
                            kind.into()
                        };
                        cx.notify();
                    })
                    .ok();
                }),
            ),
        );
        let content = self.runtime.content.read(cx);
        let source = content.workspace.clone();
        let targets = crate::automation::validate::target_options(&source, kind, "");
        let options: Vec<SelectOption<String>> = targets
            .iter()
            .filter_map(|target| {
                Some(SelectOption::new(
                    target["id"].as_str()?.to_string(),
                    target["title"]
                        .as_str()
                        .or_else(|| target["name"].as_str())
                        .unwrap_or("")
                        .to_string(),
                ))
            })
            .collect();
        let actions: Vec<Value> = self.slide["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|action| action["type"] == kind)
            .cloned()
            .collect();
        for action in actions {
            let Some(id) = action["id"].as_str() else {
                continue;
            };
            let id = id.to_string();
            let target = action["targetId"].as_str().unwrap_or("").to_string();
            let mut choices = options.clone();
            if !choices.iter().any(|option| option.value == target) {
                choices.push(SelectOption::new(target.clone(), target.clone()).disabled(true));
            }
            let weak = cx.weak_entity();
            let change_id = id.clone();
            let select = Select::new(
                SharedString::from(format!("slide-action-{id}")),
                choices,
                target,
            )
            .compact()
            .disabled(self.document_controls_disabled())
            .label(label.clone())
            .on_change(move |target, _, cx| {
                weak.update(cx, |view, cx| {
                    view.patch_inline_action(kind, Some(&change_id), Some(&target), cx)
                })
                .ok();
            });
            let weak = cx.weak_entity();
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.metrics.space_xs)
                    .child(div().flex_1().min_w_0().child(select))
                    .child(
                        IconButton::new(
                            SharedString::from(format!("remove-action-{id}")),
                            "x",
                            t.t("common.remove"),
                        )
                        .disabled(self.document_controls_disabled())
                        .on_click(move |_, _, cx| {
                            weak.update(cx, |view, cx| {
                                view.patch_inline_action(kind, Some(&id), None, cx)
                            })
                            .ok();
                        }),
                    ),
            );
        }
        if self.editor.action_picker == kind {
            if options.is_empty() {
                section = section.child(EmptyState::new(t.t("gpui.editor.noActionTargets")));
            } else {
                let weak = cx.weak_entity();
                let mut choices = vec![SelectOption::new(
                    String::new(),
                    t.t("automation.macros.chooseTarget"),
                )];
                choices.extend(options);
                section = section.child(
                    Select::new(
                        SharedString::from(format!("new-{kind}-action")),
                        choices,
                        String::new(),
                    )
                    .label(label)
                    .compact()
                    .disabled(self.document_controls_disabled())
                    .on_change(move |target, _, cx| {
                        weak.update(cx, |view, cx| {
                            view.patch_inline_action(kind, None, Some(&target), cx)
                        })
                        .ok();
                    }),
                );
            }
        }
        section.into_any_element()
    }

    fn patch_inline_action(
        &mut self,
        kind: &str,
        id: Option<&str>,
        target: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if self.document_controls_disabled() || target == Some("") {
            return;
        }
        if let Some(target) = target {
            let source = &self.runtime.content.read(cx).workspace;
            if !crate::automation::validate::target_options(source, kind, "")
                .iter()
                .any(|entry| entry["id"] == target)
            {
                self.status = cx
                    .global::<Translator>()
                    .t("automation.macros.targetInvalid");
                cx.notify();
                return;
            }
        }
        match patch_action(&self.slide, kind, id, target) {
            Ok(slide) => {
                self.editor.action_picker.clear();
                self.save_inspector_slide(slide, cx);
            }
            Err(error) => {
                self.status = error;
                cx.notify();
            }
        }
    }

    fn artwork_preview(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let data = self
            .root_document(cx)
            .and_then(|root| root["ccli"]["artwork"].as_str().map(str::to_string));
        if data.is_none() {
            self.editor.artwork_data.clear();
            self.editor.artwork_image = None;
        }
        if let Some(data) = data.filter(|data| self.editor.artwork_data != *data) {
            self.editor.artwork_data = data.clone();
            self.editor.artwork_image = None;
            cx.spawn(async move |view, cx| {
                let current = data.clone();
                let result = cx
                    .background_spawn(async move {
                        let bytes = orbit_host::decode_macro_image_data(&data)?;
                        let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                            gpui::ImageFormat::Png
                        } else if bytes.starts_with(b"\xff\xd8\xff") {
                            gpui::ImageFormat::Jpeg
                        } else {
                            return Err("Artwork must be PNG or JPEG.".to_string());
                        };
                        Ok(std::sync::Arc::new(gpui::Image::from_bytes(format, bytes)))
                    })
                    .await;
                view.update(cx, |view, cx| {
                    if view.editor.artwork_data == current {
                        view.editor.artwork_image = Some(result);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        let mut preview = div()
            .w(theme.metrics.control_height * 3.)
            .h(theme.metrics.control_height * 3.)
            .border_1()
            .border_color(theme.colors.border)
            .bg(theme.colors.preview_matte_bg);
        if let Some(Ok(image)) = &self.editor.artwork_image {
            if self
                .root_document(cx)
                .is_some_and(|root| root["ccli"]["artwork"].is_string())
            {
                preview = preview.child(
                    gpui::img(image.clone())
                        .size_full()
                        .object_fit(gpui::ObjectFit::Contain),
                );
            }
        }
        if let Some(Err(error)) = &self.editor.artwork_image {
            preview = preview.child(error.clone());
        }
        Field::new(cx.global::<Translator>().t("gpui.editor.artwork"), preview)
            .inline()
            .into_any_element()
    }
}

fn separator(cx: &gpui::App) -> impl IntoElement {
    div()
        .w_full()
        .border_t_1()
        .border_color(cx.theme().colors.border)
}

fn patch_action(
    slide: &Value,
    kind: &str,
    id: Option<&str>,
    target: Option<&str>,
) -> Result<Value, String> {
    if !matches!(kind, "media" | "video-input") {
        return Err("Unsupported inline action type.".into());
    }
    let mut slide = slide.clone();
    if slide.get("actions").is_none() {
        slide["actions"] = json!([]);
    }
    let actions = slide["actions"]
        .as_array_mut()
        .ok_or("Slide actions must be an array.")?;
    if let Some(id) = id {
        let index = actions
            .iter()
            .position(|action| action["id"] == id && action["type"] == kind)
            .ok_or("The slide action was removed.")?;
        if let Some(target) = target {
            actions[index]["targetId"] = json!(target);
        } else {
            actions.remove(index);
        }
    } else if let Some(target) = target {
        actions.push(json!({"id":uuid::Uuid::new_v4().to_string(),"type":kind,"targetId":target,"enabled":true,"delaySeconds":0}));
    }
    Ok(slide)
}
