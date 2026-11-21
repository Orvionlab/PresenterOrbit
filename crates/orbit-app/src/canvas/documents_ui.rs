use super::{
    catalog::{Field as Spec, Kind, Target},
    content_document::{open_document, DocumentKind},
    view::{History, SlideCanvas},
};
use crate::collections::song_metadata;
use gpui::{div, prelude::*, px, AnyElement, App, Context, Window};
use orbit_ui::{
    controls::{
        input_state, ActionButton, ButtonVariant, Field, IconButton, PanelHeader, Select,
        SelectOption,
    },
    ActiveTheme, Translator,
};
use serde_json::{json, Value};

pub(super) fn presentation_size(root: &Value, slide: &Value) -> String {
    let size = root.get("size").unwrap_or(slide);
    let dimension = |key: &str| {
        size[key]
            .as_f64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| size[key].to_string())
    };
    format!("{}x{}", dimension("width"), dimension("height"))
}
pub(super) fn presentation_sizes(root: &Value, slide: &Value) -> Vec<(String, String)> {
    let current = presentation_size(root, slide);
    let mut sizes: Vec<String> = ["1920x1080", "1280x720", "3840x2160"]
        .into_iter()
        .map(str::to_string)
        .collect();
    if !sizes.contains(&current) {
        sizes.push(current);
    }
    sizes
        .into_iter()
        .map(|size| (size.clone(), size.replace('x', " × ")))
        .collect()
}

pub struct DocumentOperation {
    pub(super) method: &'static str,
    pub(super) mutation: Value,
    pub(super) created: Option<&'static str>,
}
impl SlideCanvas {
    pub(super) fn new_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.kind, DocumentKind::Stage | DocumentKind::Mask) {
            let mask = self.kind == DocumentKind::Mask;
            let base =
                cx.global::<Translator>()
                    .t(if mask { "mask.mask" } else { "stage.newLayout" });
            let layouts = self.document_choices(cx);
            let title = std::iter::once(base.clone())
                .chain((1..).map(|index| {
                    if mask {
                        format!("{base}-{index}")
                    } else {
                        format!("{base} {}", index + 1)
                    }
                }))
                .find(|title| layouts.iter().all(|(_, _, existing)| existing != title))
                .unwrap_or(base);
            self.run_document_operation(
                DocumentOperation {
                    method: if mask {
                        "mutateMasks"
                    } else {
                        "mutateStageContent"
                    },
                    mutation: if mask {
                        json!({"type":"create","baseTitle":title})
                    } else {
                        json!({"type":"createLayout","title":title})
                    },
                    created: Some(if mask {
                        "createdMaskId"
                    } else {
                        "createdLayoutId"
                    }),
                },
                cx,
            );
            return;
        }
        if self.kind == DocumentKind::Prop {
            let title = cx.global::<Translator>().t("props.new");
            let folder = self.selected_prop_collection(cx);
            self.run_document_operation(
                DocumentOperation {
                    method: "mutateProps",
                    mutation: json!({"type":"create","baseTitle":title,"folder":folder}),
                    created: Some("createdPropId"),
                },
                cx,
            );
            return;
        }
        self.editor.dialog = Some(("new-theme".into(), input_state("", window, cx)));
        cx.notify();
    }
    pub fn editor_label(&self) -> &'static str {
        self.kind.label()
    }
    pub(super) fn document_choices(&self, cx: &App) -> Vec<(String, String, String)> {
        let content = self.runtime.content.read(cx);
        if self.kind == DocumentKind::Theme {
            return content
                .themes
                .iter()
                .map(|theme| {
                    (
                        theme.id.clone(),
                        theme
                            .slides
                            .first()
                            .map(|(id, _)| id.clone())
                            .unwrap_or_default(),
                        theme.title.clone(),
                    )
                })
                .collect();
        }
        if self.kind == DocumentKind::Slide {
            return content
                .items
                .iter()
                .filter_map(|item| {
                    Some((
                        item.id.clone(),
                        item.value["slides"]
                            .as_array()?
                            .first()
                            .and_then(|slide| slide["id"].as_str())
                            .unwrap_or("")
                            .to_string(),
                        item.title.clone(),
                    ))
                })
                .collect();
        }
        let folder = (self.kind == DocumentKind::Prop).then(|| self.selected_prop_collection(cx));
        let mut choices: Vec<_> = content.workspace[self.kind.collection()]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|document| {
                folder
                    .as_ref()
                    .is_none_or(|folder| document["folder"].as_str().unwrap_or("") == folder)
            })
            .filter_map(|document| {
                Some((
                    document["id"].as_str()?.into(),
                    document["slide"]["id"].as_str()?.into(),
                    document["title"].as_str()?.into(),
                ))
            })
            .collect();
        if self.kind == DocumentKind::Mask {
            if let Some(order) = &self.editor.rows.mask_order {
                choices.sort_by_key(|(id, _, _)| {
                    order
                        .iter()
                        .position(|candidate| candidate == id)
                        .unwrap_or(usize::MAX)
                });
            }
        }
        choices
    }
    pub(super) fn layout_choices(&self, cx: &App) -> Vec<SelectOption<String>> {
        let content = self.runtime.content.read(cx);
        if self.kind == DocumentKind::Theme {
            return content
                .themes
                .iter()
                .find(|theme| theme.id == self.theme_id)
                .map(|theme| {
                    theme
                        .slides
                        .iter()
                        .map(|(id, name)| SelectOption::new(id.clone(), name.clone()))
                        .collect()
                })
                .unwrap_or_default();
        }
        if self.kind == DocumentKind::Slide {
            return content
                .items
                .get(&self.theme_id)
                .and_then(|item| item.value["slides"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|slide| {
                    Some(SelectOption::new(
                        slide["id"].as_str()?.to_string(),
                        slide["name"]
                            .as_str()
                            .or_else(|| slide["label"].as_str())
                            .unwrap_or(slide["id"].as_str()?)
                            .to_string(),
                    ))
                })
                .collect();
        }
        vec![SelectOption::new(
            self.slide_id.clone(),
            self.slide["name"]
                .as_str()
                .unwrap_or(&self.slide_id)
                .to_string(),
        )]
    }
    pub(super) fn document_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.kind == DocumentKind::Theme {
            return self.editor_sidebar(window, cx);
        }
        if self.kind == DocumentKind::Prop {
            return self.prop_sidebar(cx);
        }
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        if self.kind == DocumentKind::Ccli {
            let sidebar = div()
                .id("ccli-objects")
                .debug_selector(|| "ccli-objects".into())
                .w(px(self.sidebar_width))
                .flex_none()
                .min_h_0()
                .overflow_y_scroll()
                .bg(theme.colors.panel_bg)
                .border_r_1()
                .border_color(theme.colors.border)
                .child(self.object_list(cx));
            return self.track_rows(sidebar, cx).into_any_element();
        }
        let title = if self.kind == DocumentKind::Slide {
            self.runtime
                .content
                .read(cx)
                .items
                .get(&self.theme_id)
                .map(|item| item.title.clone())
                .unwrap_or_default()
        } else if self.kind == DocumentKind::Stage {
            t.t("stage.layouts")
        } else {
            t.t("mask.masks")
        };
        let mut header = PanelHeader::new(title);
        if self.kind != DocumentKind::Slide {
            header = header.action(
                IconButton::new("new-editor-document", "plus", t.t("common.new"))
                    .disabled(
                        self.document_controls_disabled()
                            || self.runtime.content.read(cx).workspace_path.is_empty(),
                    )
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.new_document(window, cx);
                    })),
            );
        }
        let mut slides_header_row = None;
        let theme_assignment = (self.kind == DocumentKind::Slide && !self.slide_id.is_empty())
            .then(|| self.slide_theme_select(cx));
        if self.kind == DocumentKind::Slide {
            let slides_header = PanelHeader::new(t.t("theme.slides")).action(
                IconButton::new("editor-slide-insert", "plus", t.t("common.add"))
                    .disabled(self.saving || self.theme_id.is_empty())
                    .on_click(cx.listener(|view, _, _, cx| {
                        let slide = view.slide_id.clone();
                        view.slide_structure("insert", &slide, cx)
                    })),
            );
            slides_header_row = Some(slides_header);
        }
        let mask_focus = (self.kind == DocumentKind::Mask).then(|| {
            self.editor
                .rows
                .mask_focus
                .get_or_insert_with(|| cx.focus_handle())
                .clone()
        });
        let sidebar = div()
            .id("authoring-sidebar")
            .w(px(self.sidebar_width))
            .flex_none()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(theme.colors.panel_bg)
            .border_r_1()
            .border_color(theme.colors.border)
            .when_some(mask_focus, |sidebar, focus| sidebar.track_focus(&focus))
            .child(header)
            .children(theme_assignment);
        let upper = div().flex().flex_col().flex_1().min_h_0();
        let upper = if self.kind == DocumentKind::Slide {
            upper.children(slides_header_row).child(
                div()
                    .id("authoring-slides-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.slide_list(cx)),
            )
        } else {
            upper.child(
                div()
                    .id("authoring-layout-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.document_slide_list(cx)),
            )
        };
        let sidebar = sidebar.child(self.sidebar_panels(upper, cx));
        self.track_rows(sidebar, cx).into_any_element()
    }

    fn prop_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let disabled = self.document_controls_disabled();
        let options = self
            .prop_collections(cx)
            .into_iter()
            .map(|folder| {
                let label = if folder.is_empty() {
                    t.t("props.defaultCollection")
                } else {
                    folder.clone()
                };
                SelectOption::new(folder, label)
            })
            .collect();
        let owner = cx.weak_entity();
        let header = div()
            .flex()
            .items_center()
            .gap(theme.metrics.space_xs)
            .p(theme.metrics.space_xs)
            .border_b_1()
            .border_color(theme.colors.border)
            .child(
                div()
                    .debug_selector(|| "canvas-prop-collection-picker".into())
                    .flex_1()
                    .min_w_0()
                    .child(
                        Select::new(
                            "canvas-prop-collection",
                            options,
                            self.selected_prop_collection(cx),
                        )
                        .compact()
                        .label(t.t("props.collection"))
                        .disabled(disabled)
                        .on_change(move |folder, _, cx| {
                            owner
                                .update(cx, |view, cx| {
                                    if folder != view.selected_prop_collection(cx) {
                                        view.open_prop_collection(&folder, cx);
                                    }
                                })
                                .ok();
                        }),
                    ),
            )
            .child(
                IconButton::new("new-editor-document", "plus", t.t("props.new"))
                    .disabled(disabled || self.runtime.content.read(cx).workspace_path.is_empty())
                    .on_click(cx.listener(|view, _, window, cx| view.new_document(window, cx))),
            );
        let upper = div()
            .id("authoring-props-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(self.document_slide_list(cx));
        let sidebar = div()
            .id("authoring-sidebar")
            .w(px(self.sidebar_width))
            .flex_none()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(theme.colors.panel_bg)
            .border_r_1()
            .border_color(theme.colors.border)
            .child(header)
            .child(self.sidebar_panels(upper, cx));
        self.track_rows(sidebar, cx).into_any_element()
    }
    /// `data-slide-theme-select`: the theme layout the open slide uses.
    fn slide_theme_select(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let content = self.runtime.content.read(cx);
        let assigned = content
            .items
            .get(&self.theme_id)
            .and_then(|item| item.value["slides"].as_array())
            .and_then(|slides| slides.iter().find(|slide| slide["id"] == self.slide_id))
            .and_then(|slide| slide["presentationThemeId"].as_str())
            .unwrap_or("")
            .to_string();
        let mut options =
            vec![SelectOption::new(String::new(), t.t("theme.assignment.choose")).disabled(true)];
        for theme in &content.themes {
            for (slide, name) in &theme.slides {
                options.push(SelectOption::new(
                    format!("{}::{slide}", theme.id),
                    format!("{}: {name}", theme.title),
                ));
            }
        }
        if !assigned.is_empty() && !options.iter().any(|option| option.value == assigned) {
            let imported = orbit_host::authoring::is_imported_layout_assignment(
                &content.workspace["themes"],
                &assigned,
            );
            options.push(
                SelectOption::new(
                    assigned.clone(),
                    if imported {
                        t.t("theme.assignment.importedLayout")
                    } else {
                        format!("{} ({})", assigned, t.t("theme.assignment.missing"))
                    },
                )
                .disabled(true),
            );
        }
        let weak = cx.weak_entity();
        div()
            .px(cx.theme().metrics.space_xs)
            .pb(cx.theme().metrics.space_xs)
            .child(
                Select::new("slide-theme-select", options, assigned)
                    .compact()
                    .label(t.t("theme.theme"))
                    .disabled(self.document_controls_disabled())
                    .on_change(move |assignment, _, cx| {
                        weak.update(cx, |view, cx| view.assign_slide_theme(assignment, cx))
                            .ok();
                    }),
            )
            .into_any_element()
    }
    fn assign_slide_theme(&mut self, assignment: String, cx: &mut Context<Self>) {
        let mutation = json!({"type":"mutateItem","itemId":self.theme_id,"operation":{"type":"setSlidesTheme","slideIds":[self.slide_id],"assignment":assignment}});
        self.run_document_operation(
            DocumentOperation {
                method: "mutatePresentationLibrary",
                mutation,
                created: None,
            },
            cx,
        );
    }
    pub(super) fn move_slide(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if self.kind == DocumentKind::Mask {
            let mut order: Vec<_> = self
                .document_choices(cx)
                .into_iter()
                .map(|(id, _, _)| id)
                .collect();
            let (Some(id), Some(target)) = (order.get(from).cloned(), order.get(to).cloned())
            else {
                return;
            };
            if super::sidebar::moved(&mut order, from, to) {
                self.editor.rows.mask_order = Some(order);
                self.mutate_mask(
                    json!({"type":"reorder","maskId":id,"targetMaskId":target,"before":to < from}),
                    cx,
                );
                cx.notify();
            }
            return;
        }
        if self.kind == DocumentKind::Theme {
            return self.move_theme_slide(from, to, cx);
        }
        let slides = self.layout_choices(cx);
        let Some(slide) = slides.get(from) else {
            return;
        };
        if to >= slides.len() || to == from {
            return;
        }
        let mutation = json!({"type":"mutateItem","itemId":self.theme_id,"operation":{"type":"reorderSlide","slideId":slide.value,"targetIndex":to}});
        self.run_document_operation(
            DocumentOperation {
                method: "mutatePresentationLibrary",
                mutation,
                created: None,
            },
            cx,
        );
    }

    pub(super) fn mask_clipboard(&self, cx: &App) -> Option<&Value> {
        self.editor
            .rows
            .mask_clipboard
            .as_ref()
            .filter(|(workspace, _)| workspace == &self.runtime.content.read(cx).workspace_path)
            .map(|(_, mask)| mask)
    }

    pub(super) fn mask_action(
        &mut self,
        id: &str,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.kind != DocumentKind::Mask || self.saving || self.editor.pending_operation.is_some()
        {
            return;
        }
        let content = self.runtime.content.read(cx);
        let Some(mask) = content.workspace["masks"]
            .as_array()
            .and_then(|masks| masks.iter().find(|mask| mask["id"] == id))
        else {
            return;
        };
        match action {
            "copy" => {
                let mut copy = mask.clone();
                if self.theme_id == id {
                    copy["slide"] = (*self.slide).clone();
                    copy["size"] =
                        json!({"width":self.slide["width"], "height":self.slide["height"]});
                }
                self.editor.rows.mask_clipboard = Some((content.workspace_path.clone(), copy));
            }
            "paste" => {
                if let Some(mask) = self.mask_clipboard(cx) {
                    self.mutate_mask(json!({"type":"paste","maskId":id,"mask":mask}), cx);
                }
            }
            "rename" => {
                let Some(title) = mask["title"].as_str().map(str::to_owned) else {
                    return;
                };
                let input = input_state(title, window, cx);
                input.update(cx, |state, cx| {
                    state.focus(window, cx);
                    state.select_all(window, cx);
                });
                self.editor.dialog = Some((format!("rename-mask:{id}"), input));
                cx.notify();
            }
            "duplicate" | "delete" => self.mutate_mask(json!({"type":action,"maskId":id}), cx),
            _ => {}
        }
    }

    pub(super) fn mutate_mask(&mut self, mutation: Value, cx: &mut Context<Self>) {
        let created = matches!(mutation["type"].as_str(), Some("duplicate" | "paste"))
            .then_some("createdMaskId");
        self.run_document_operation(
            DocumentOperation {
                method: "mutateMasks",
                mutation,
                created,
            },
            cx,
        );
    }
    pub(super) fn slide_structure(&mut self, action: &str, slide: &str, cx: &mut Context<Self>) {
        let Some(root) = self.root_document(cx) else {
            return;
        };
        let index = root["slides"].as_array().and_then(|slides| {
            slides
                .iter()
                .position(|slide_item| slide_item["id"] == slide)
        });
        let count = root["slides"].as_array().map_or(0, Vec::len);
        let (operation, created) = match action {
            "insert" => {
                let mut operation = json!({"type":"insertBlankSlide"});
                if !slide.is_empty() {
                    operation["afterSlideId"] = json!(slide);
                }
                (operation, Some("createdSlideId"))
            }
            "delete" => (json!({"type":"deleteSlide","slideId":slide}), None),
            "up" if index.is_some_and(|index| index > 0) => (
                json!({"type":"reorderSlide","slideId":slide,"targetIndex":index.unwrap()-1}),
                None,
            ),
            "down" if index.is_some_and(|index| index + 1 < count) => (
                json!({"type":"reorderSlide","slideId":slide,"targetIndex":index.unwrap()+1}),
                None,
            ),
            _ => return,
        };
        self.run_document_operation(
            DocumentOperation {
                method: "mutatePresentationLibrary",
                mutation: json!({"type":"mutateItem","itemId":self.theme_id,"operation":operation}),
                created,
            },
            cx,
        );
    }
    pub(super) fn root_document(&self, cx: &App) -> Option<Value> {
        let content = self.runtime.content.read(cx);
        if self.kind == DocumentKind::Slide {
            self.editor.document_preview.clone().or_else(|| {
                content
                    .items
                    .get(&self.theme_id)
                    .map(|item| (*item.value).clone())
            })
        } else {
            content.workspace[self.kind.collection()]
                .as_array()?
                .iter()
                .find(|document| document["id"] == self.theme_id)
                .cloned()
        }
    }
    pub(super) fn document_fields(&self, cx: &App) -> Vec<Spec> {
        if self.kind == DocumentKind::Theme || self.selected.is_some() {
            return Vec::new();
        }
        let Some(root) = self.root_document(cx) else {
            return Vec::new();
        };
        let tab = self.document_tab();
        let mut fields = Vec::new();
        if self.kind != DocumentKind::Slide {
            fields.push(
                Spec::new(
                    "documentTitle",
                    Target::Document,
                    Kind::Text,
                    root["title"].clone(),
                )
                .named(if self.kind == DocumentKind::Slide {
                    "newPresentation.name"
                } else {
                    "theme.name"
                }),
            );
        }
        if self.kind == DocumentKind::Slide && tab == "presentation" {
            for (key, label, kind, value) in [
                (
                    "presentation:size",
                    "theme.size",
                    Kind::Choice(presentation_sizes(&root, &self.slide)),
                    json!(presentation_size(&root, &self.slide)),
                ),
                (
                    "presentation:display",
                    "gpui.editor.ccliDisplay",
                    Kind::Bool,
                    json!(root["ccli"]["display"] != false),
                ),
                (
                    "presentation:artistCredits",
                    "gpui.editor.artist",
                    Kind::Text,
                    json!(root["ccli"]["artistCredits"].as_str().unwrap_or("")),
                ),
                (
                    "presentation:album",
                    "gpui.editor.album",
                    Kind::Text,
                    json!(root["ccli"]["album"].as_str().unwrap_or("")),
                ),
            ] {
                fields.push(Spec::new(key, Target::Document, kind, value).named(label));
            }
        }
        if self.kind == DocumentKind::Slide && tab == "presentation" {
            if let Some(mut item) = self
                .runtime
                .content
                .read(cx)
                .items
                .get(&self.theme_id)
                .cloned()
            {
                item.value = std::sync::Arc::new(root.clone());
                let draft = song_metadata::draft(&item);
                for (key, label, value) in [
                    ("ccli:title", "gpui.editor.songTitle", draft.title),
                    ("ccli:author", "songMetadata.author", draft.author),
                    ("ccli:publisher", "gpui.editor.publisher", draft.publisher),
                    (
                        "ccli:copyrightYear",
                        "gpui.editor.year",
                        draft.copyright_year,
                    ),
                    (
                        "ccli:songNumber",
                        "gpui.editor.songNumber",
                        draft.song_number,
                    ),
                ] {
                    fields.push(
                        Spec::new(key, Target::Document, Kind::Text, json!(value)).named(label),
                    );
                }
            }
        }
        if self.kind == DocumentKind::Slide && tab == "slide" {
            let slide = root["slides"]
                .as_array()
                .and_then(|slides| slides.iter().find(|slide| slide["id"] == self.slide_id))
                .cloned()
                .unwrap_or(Value::Null);
            let groups = self
                .runtime
                .output
                .read(cx)
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot["groupSettings"].as_object())
                .cloned()
                .unwrap_or_default();
            let mut options = vec![(
                String::new(),
                cx.global::<Translator>().t("cue.menu.noGroup"),
            )];
            options.extend(groups.iter().map(|(id, setting)| {
                (
                    id.clone(),
                    setting["name"].as_str().unwrap_or(id).to_string(),
                )
            }));
            fields.push(
                Spec::new(
                    "slideShortcut",
                    Target::Document,
                    Kind::Text,
                    json!(slide["shortcut"].as_str().unwrap_or("")),
                )
                .named("gpui.editor.slideShortcut"),
            );
            fields.push(
                Spec::new(
                    "slideEnabled",
                    Target::Document,
                    Kind::Bool,
                    json!(slide["disabled"] != true),
                )
                .named("gpui.editor.slideEnabled"),
            );
            fields.push(
                Spec::new(
                    "slideGroup",
                    Target::Document,
                    Kind::Choice(options),
                    json!(crate::cues::model::group_id(&slide)),
                )
                .named("gpui.editor.slideLabel"),
            );
            fields.push(
                Spec::new(
                    "labelColor",
                    Target::Slide,
                    Kind::Color,
                    self.slide.get("labelColor").cloned().unwrap_or(json!("")),
                )
                .named("theme.objectLink.labelColor"),
            );
        }
        if self.kind == DocumentKind::Slide && tab == "build" {
            let fake = json!({"layers":[{"id":"transition","type":"Shape","build":{"step":1,"start":"on-click","delaySeconds":0,"transition":self.slide["transition"]}}]});
            for mut field in
                super::catalog::fields("build", &fake, Some(0), None, &self.editor.transitions)
            {
                if field.key.starts_with("buildTransition")
                    || field.key.starts_with("buildVariable:")
                {
                    field.key = field.key.replacen("build", "slide", 1);
                    field.target = Target::Slide;
                    if field.key == "slideTransition" {
                        field.label = "transition.effect".into();
                        if let Kind::Choice(options) = &mut field.kind {
                            options.insert(
                                0,
                                (
                                    String::new(),
                                    cx.global::<Translator>().t("transition.none"),
                                ),
                            );
                        }
                        if self.slide["transition"].is_null() {
                            field.value = json!("");
                        }
                    }
                    fields.push(field);
                }
            }
        }
        fields
    }
    pub(super) fn document_tabs(&self) -> bool {
        self.kind == DocumentKind::Slide && self.selected.is_none()
    }
    pub(super) fn document_tab(&self) -> &'static str {
        match self.editor.doc_tab.as_str() {
            "presentation" => "presentation",
            "build" => "build",
            _ => "slide",
        }
    }
    pub(super) fn document_tab_widgets(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let weak = cx.weak_entity();
        match self.document_tab() {
            "presentation" => {
                let transition = match self.editor.item_transition.clone() {
                    Some(transition) => transition,
                    None => {
                        let runtime = self.runtime.clone();
                        let transition = cx.new(|cx| {
                            crate::transitions::TransitionDefaults::new(
                                crate::transitions::Kind::Slide,
                                runtime,
                                cx,
                            )
                        });
                        self.editor.item_transition = Some(transition.clone());
                        transition
                    }
                };
                let item = self.theme_id.clone();
                let preview = self
                    .root_document(cx)
                    .map_or(Value::Null, |root| root["transition"].clone());
                let target = self.theme_id.clone();
                let owner = cx.weak_entity();
                transition.update(cx, |transition, _| {
                    transition.inspector = true;
                    transition.preview = Some(preview);
                    transition.on_change = Some(std::rc::Rc::new(move |value, cx| {
                        owner
                            .update(cx, |view, cx| {
                                if view.theme_id == target {
                                    view.set_inspector_transition(value, false, cx);
                                }
                            })
                            .ok();
                    }));
                    if transition.item_id.as_deref() != Some(&item) {
                        transition.item_id = Some(item);
                    }
                });
                let mut artwork = div().flex().gap(theme.metrics.space_xs).child(
                    ActionButton::new(
                        "presentation-artwork-choose",
                        t.t("gpui.editor.chooseArtwork"),
                    )
                    .disabled(self.document_controls_disabled())
                    .on_click({
                        let weak = weak.clone();
                        move |_, _, cx| {
                            weak.update(cx, |view, cx| view.choose_artwork(cx)).ok();
                        }
                    }),
                );
                if self
                    .root_document(cx)
                    .is_some_and(|root| root["ccli"]["artwork"].is_string())
                {
                    artwork = artwork.child(
                        ActionButton::new("presentation-artwork-remove", t.t("common.remove"))
                            .disabled(self.document_controls_disabled())
                            .on_click({
                                let weak = weak.clone();
                                move |_, _, cx| {
                                    weak.update(cx, |view, cx| {
                                        let spec = Spec::new(
                                            "presentation:artwork",
                                            Target::Document,
                                            Kind::Text,
                                            Value::Null,
                                        );
                                        view.patch_document_field(&spec, &Value::Null, cx);
                                    })
                                    .ok();
                                }
                            }),
                    );
                }
                vec![
                    Field::new(t.t("gpui.editor.presentationTransition"), transition)
                        .into_any_element(),
                    artwork.into_any_element(),
                ]
            }
            "slide" => {
                let chart = self
                    .root_document(cx)
                    .and_then(|root| {
                        root["slides"]
                            .as_array()?
                            .iter()
                            .find(|slide| slide["id"] == self.slide_id)
                            .map(|slide| slide["chordChart"].clone())
                    })
                    .unwrap_or(Value::Null);
                let mut chart_box = div().flex().items_start().gap(theme.metrics.space_sm);
                let mut preview = div()
                    .flex_none()
                    .w(theme.metrics.control_height * 2.)
                    .h(theme.metrics.control_height * 2.)
                    .bg(theme.colors.control_bg)
                    .border_1()
                    .border_color(theme.colors.border);
                if let Some(page) = chart["page"]
                    .as_u64()
                    .and_then(|index| chart["pagePaths"][index as usize].as_str())
                {
                    preview = preview.child(
                        gpui::img(std::path::PathBuf::from(page))
                            .object_fit(gpui::ObjectFit::Contain)
                            .w(theme.metrics.control_height * 2.)
                            .h(theme.metrics.control_height * 2.),
                    );
                }
                chart_box = chart_box.child(preview);
                let mut buttons = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(theme.metrics.space_xs)
                    .child(
                        ActionButton::new("slide-chart-choose", t.t("gpui.editor.chooseChart"))
                            .disabled(self.document_controls_disabled())
                            .on_click({
                                let weak = weak.clone();
                                move |_, _, cx| {
                                    weak.update(cx, |view, cx| view.choose_chord_chart(cx)).ok();
                                }
                            }),
                    );
                buttons = buttons.child(
                    ActionButton::new("slide-chart-clear", t.t("common.remove"))
                        .variant(ButtonVariant::Ghost)
                        .disabled(self.saving || chart.is_null())
                        .on_click({
                            let weak = weak.clone();
                            move |_, _, cx| {
                                weak.update(cx, |view, cx| view.save_chord_chart(&Value::Null, cx))
                                    .ok();
                            }
                        }),
                );
                vec![
                    Field::new(t.t("chordChart.title"), chart_box.child(buttons))
                        .into_any_element(),
                ]
            }
            _ => Vec::new(),
        }
    }
    fn save_chord_chart(&mut self, chart: &Value, cx: &mut Context<Self>) {
        let Some(mut root) = self.root_document(cx) else {
            return;
        };
        if self.editor.base.is_some() || self.pending_saves > 0 {
            if let Err(error) = store_editor_slide(
                &mut root,
                &self.slide,
                &self.runtime.content.read(cx).workspace,
            ) {
                self.status = error;
                cx.notify();
                return;
            }
        }
        match crate::charts::mutation(&root, &self.slide_id, chart) {
            Ok(mutation) => self.run_document_operation(
                DocumentOperation {
                    method: "mutatePresentationLibrary",
                    mutation,
                    created: None,
                },
                cx,
            ),
            Err(key) => {
                self.status = cx.global::<Translator>().t(&key);
                cx.notify();
            }
        }
    }
    fn choose_artwork(&mut self, cx: &mut Context<Self>) {
        let runtime = self.runtime.clone();
        let path = runtime.content.read(cx).workspace_path.clone();
        let current = (self.theme_id.clone(), self.slide_id.clone());
        cx.spawn(async move |view, cx| {
            let answer = crate::services::call(
                &runtime,
                cx,
                "choosePresentationArtwork",
                vec![json!(path)],
                false,
            )
            .await;
            view.update(cx, |view, cx| {
                if (view.theme_id.clone(), view.slide_id.clone()) != current
                    || runtime.content.read(cx).workspace_path != path
                {
                    return;
                }
                match answer {
                    Ok(answer) if answer["canceled"] == true => {}
                    Ok(answer) => {
                        let spec = Spec::new(
                            "presentation:artwork",
                            Target::Document,
                            Kind::Text,
                            Value::Null,
                        );
                        view.patch_document_field(&spec, &answer["artwork"], cx);
                    }
                    Err(error) => {
                        view.status = error;
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }
    fn choose_chord_chart(&mut self, cx: &mut Context<Self>) {
        let runtime = self.runtime.clone();
        let path = runtime.content.read(cx).workspace_path.clone();
        let current = (self.theme_id.clone(), self.slide_id.clone());
        cx.spawn(async move |view, cx| {
            let answer =
                crate::services::call(&runtime, cx, "chooseChordChart", vec![json!(path)], false)
                    .await;
            view.update(cx, |view, cx| {
                if (view.theme_id.clone(), view.slide_id.clone()) != current {
                    return;
                }
                match answer {
                    Ok(answer) if answer["canceled"] == true => {}
                    Ok(answer) => view.save_chord_chart(&answer["chart"], cx),
                    Err(error) => {
                        view.status = error;
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn patch_document_field(
        &mut self,
        spec: &Spec,
        value: &Value,
        cx: &mut Context<Self>,
    ) -> bool {
        if spec.target != Target::Document {
            return false;
        }
        let Some(root) = self.root_document(cx) else {
            return true;
        };
        let (method, mutation) = match spec.key.as_str() {
            "documentTitle" => {
                let title = value.as_str().unwrap_or("").trim();
                if title.is_empty() {
                    self.status = cx.global::<Translator>().t("theme.gpui.nameRequired");
                    cx.notify();
                    return true;
                }
                if root["title"] == title {
                    return true;
                }
                match self.kind {
                    DocumentKind::Slide => (
                        "mutatePresentationLibrary",
                        json!({"type":"mutateItem","itemId":self.theme_id,"operation":{"type":"setTitle","title":title}}),
                    ),
                    DocumentKind::Prop => (
                        "mutateProps",
                        json!({"type":"rename","propId":self.theme_id,"title":title}),
                    ),
                    DocumentKind::Mask => (
                        "mutateMasks",
                        json!({"type":"updateDocument","maskId":self.theme_id,"title":title}),
                    ),
                    DocumentKind::Stage => {
                        let mut layout = root;
                        layout["title"] = json!(title);
                        (
                            "mutateStageContent",
                            json!({"type":"replaceLayout","layoutId":self.theme_id,"layout":layout}),
                        )
                    }
                    DocumentKind::Theme | DocumentKind::Ccli => return true,
                }
            }
            key if key.starts_with("presentation:") => {
                let mut field = &key["presentation:".len()..];
                let mut value = value.clone();
                if field == "size" {
                    let dimensions = value
                        .as_str()
                        .and_then(|size| size.split_once('x'))
                        .and_then(|(width, height)| {
                            Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?))
                        });
                    let Some((width, height)) = dimensions else {
                        self.status = cx.global::<Translator>().t("theme.gpui.invalidValue");
                        cx.notify();
                        return true;
                    };
                    value = json!({"width":width,"height":height});
                }
                if matches!(field, "width" | "height") {
                    let mut size = root.get("size").cloned().unwrap_or_else(
                        || json!({"width":self.slide["width"],"height":self.slide["height"]}),
                    );
                    size[field] = value;
                    value = size;
                    field = "size";
                }
                (
                    "mutatePresentationLibrary",
                    json!({"type":"mutateItem","itemId":self.theme_id,"operation":{"type":"setPresentationSetting","field":field,"value":value}}),
                )
            }
            "slideShortcut" => (
                "mutatePresentationLibrary",
                json!({"type":"mutateItem","itemId":self.theme_id,"operation":{"type":"setSlideShortcut","slideId":self.slide_id,"shortcut":value}}),
            ),
            "slideEnabled" => (
                "mutatePresentationLibrary",
                json!({"type":"mutateItem","itemId":self.theme_id,"operation":{"type":"setSlideEnabled","slideId":self.slide_id,"enabled":value}}),
            ),
            "slideGroup" => (
                "mutatePresentationLibrary",
                json!({"type":"mutateItem","itemId":self.theme_id,"operation":{"type":"setSlideGroup","slideId":self.slide_id,"groupId":if value.as_str().is_some_and(str::is_empty){Value::Null}else{value.clone()}}}),
            ),
            key if key.starts_with("ccli:") => {
                let Some(mut item) = self
                    .runtime
                    .content
                    .read(cx)
                    .items
                    .get(&self.theme_id)
                    .cloned()
                else {
                    return true;
                };
                item.value = std::sync::Arc::new(root.clone());
                let mut draft = song_metadata::draft(&item);
                let text = value.as_str().unwrap_or("").to_string();
                match &key["ccli:".len()..] {
                    "title" => draft.title = text,
                    "author" => draft.author = text,
                    "publisher" => draft.publisher = text,
                    "copyrightYear" => draft.copyright_year = text,
                    "songNumber" => draft.song_number = text,
                    _ => return true,
                }
                if draft == song_metadata::draft(&item) {
                    return true;
                }
                match song_metadata::apply(&item.value, &draft) {
                    Ok(next) => (
                        "mutatePresentationLibrary",
                        json!({"type":"mutateItem","itemId":self.theme_id,"operation":{"type":"setPresentationSetting","field":&key["ccli:".len()..],"value":next["ccli"][&key["ccli:".len()..]]}}),
                    ),
                    Err(key) => {
                        self.status = cx.global::<Translator>().t(key);
                        cx.notify();
                        return true;
                    }
                }
            }
            _ => {
                self.status = "Unsupported authoring document field.".into();
                cx.notify();
                return true;
            }
        };
        let operation = DocumentOperation {
            method,
            mutation,
            created: None,
        };
        self.run_document_operation(operation, cx);
        true
    }
    fn preview_document_operation(&mut self, operation: DocumentOperation, cx: &mut Context<Self>) {
        self.flush_rich(cx);
        let Some(mut root) = self.root_document(cx) else {
            return;
        };
        if self.editor.base.is_some() || self.pending_saves > 0 {
            if let Err(error) = store_editor_slide(
                &mut root,
                &self.slide,
                &self.runtime.content.read(cx).workspace,
            ) {
                self.status = error;
                cx.notify();
                return;
            }
        }
        let setting = &operation.mutation["operation"];
        if let Err(error) = preview_item_operation(&mut root, setting) {
            self.status = error;
            cx.notify();
            return;
        }
        let raw = root["slides"]
            .as_array()
            .and_then(|slides| slides.iter().find(|slide| slide["id"] == self.slide_id));
        let Some(raw) = raw else {
            self.status = "The authoring slide was removed.".into();
            cx.notify();
            return;
        };
        let theme =
            orbit_host::authoring::slide_theme(&self.runtime.content.read(cx).workspace, raw);
        let preview = match orbit_host::authoring::compose(raw, theme) {
            Ok(slide) => slide,
            Err(error) => {
                self.status = error;
                cx.notify();
                return;
            }
        };
        if self.editor.document_preview.is_none() {
            self.editor.document_before_slide = Some(self.slide.clone());
        }
        self.editor.document_preview = Some(root);
        self.set_slide(std::sync::Arc::new(preview));
        if self
            .editor
            .document_operations
            .back()
            .is_some_and(|pending| {
                let pending = &pending.mutation;
                let next = &operation.mutation;
                pending["itemId"] == next["itemId"]
                    && ["type", "field", "slideId"]
                        .iter()
                        .all(|key| pending["operation"][key] == next["operation"][key])
            })
        {
            self.editor.document_operations.pop_back();
        }
        self.editor.document_operations.push_back(operation);
        self.request_raster(false, cx);
        cx.notify();
        if !self.saving && self.pending_saves == 0 {
            if self.editor.base.is_some() {
                self.save((*self.slide).clone(), History::Edit, cx);
            } else if let Some(operation) = self.editor.document_operations.pop_front() {
                self.save_document_operation(operation, cx);
            }
        }
    }

    pub(super) fn save_inspector_slide(&mut self, slide: Value, cx: &mut Context<Self>) {
        let Some(mut root) = self.root_document(cx) else {
            return;
        };
        if let Err(error) =
            store_editor_slide(&mut root, &slide, &self.runtime.content.read(cx).workspace)
        {
            self.status = error;
            cx.notify();
            return;
        }
        self.editor.document_history = true;
        self.run_document_operation(DocumentOperation {
            method: "mutatePresentationLibrary",
            mutation: json!({"type":"mutateItem","itemId":self.theme_id,"operation":{"type":"setSlideDocument","slides":root["slides"],"title":root["title"]}}),
            created: None,
        }, cx);
    }

    pub(super) fn set_inspector_transition(
        &mut self,
        transition: Value,
        slide: bool,
        cx: &mut Context<Self>,
    ) {
        let mut operation = json!({"type":"setTransition","transition":transition});
        if slide {
            operation["slideId"] = json!(self.slide_id);
        }
        self.run_document_operation(
            DocumentOperation {
                method: "mutatePresentationLibrary",
                mutation: json!({"type":"mutateItem","itemId":self.theme_id,"operation":operation}),
                created: None,
            },
            cx,
        );
    }

    fn refresh_document_preview(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if self.editor.document_preview.take().is_none() {
            return Ok(());
        }
        if !self.editor.document_operations.is_empty() {
            let mut root = self
                .root_document(cx)
                .ok_or("The presentation was removed.")?;
            for operation in &self.editor.document_operations {
                preview_item_operation(&mut root, &operation.mutation["operation"])?;
            }
            let raw = root["slides"]
                .as_array()
                .and_then(|slides| slides.iter().find(|slide| slide["id"] == self.slide_id))
                .ok_or("The authoring slide was removed.")?;
            let theme =
                orbit_host::authoring::slide_theme(&self.runtime.content.read(cx).workspace, raw);
            let slide = orbit_host::authoring::compose(raw, theme)?;
            self.editor.document_preview = Some(root);
            self.set_slide(std::sync::Arc::new(slide));
        } else {
            if self.editor.document_history {
                if let Some(before) = self.editor.document_before_slide.as_ref() {
                    self.undo.push(before.clone());
                    self.redo.clear();
                }
                self.editor.document_history = false;
            }
            self.editor.document_before_slide = None;
            let document =
                open_document(&self.runtime, self.kind, &self.theme_id, &self.slide_id, cx)?;
            self.set_slide(document.slide());
            self.document = Some(document);
        }
        self.request_raster(false, cx);
        Ok(())
    }

    pub(super) fn run_document_operation(
        &mut self,
        operation: DocumentOperation,
        cx: &mut Context<Self>,
    ) {
        let kind = operation.mutation["operation"]["type"].as_str();
        if operation.method == "mutatePresentationLibrary"
            && operation.created.is_none()
            && matches!(
                kind,
                Some(
                    "setPresentationSetting"
                        | "setSlideDocument"
                        | "setSlideEnabled"
                        | "setSlideGroup"
                        | "setSlideShortcut"
                        | "setTransition"
                )
            )
            && operation.mutation["operation"]["field"] != "artwork"
        {
            self.preview_document_operation(operation, cx);
        } else {
            self.save_document_operation(operation, cx);
        }
    }

    fn save_document_operation(
        &mut self,
        mut operation: DocumentOperation,
        cx: &mut Context<Self>,
    ) {
        self.flush_rich(cx);
        if self.saving || self.pending_saves > 0 || self.editor.base.is_some() {
            self.editor.pending_operation = Some(operation);
            if !self.saving {
                self.save((*self.slide).clone(), History::Edit, cx);
            }
            return;
        }
        if operation.method == "mutatePresentationLibrary"
            && operation.mutation["operation"]["type"] == "setSlideDocument"
            && operation.created.is_none()
        {
            let edited = operation.mutation["operation"]["slides"]
                .as_array()
                .and_then(|slides| slides.iter().find(|slide| slide["id"] == self.slide_id))
                .cloned();
            let latest = self
                .runtime
                .content
                .read(cx)
                .items
                .get(&self.theme_id)
                .and_then(|item| item.value["slides"].as_array())
                .cloned();
            let Some((edited, mut slides)) = edited.zip(latest) else {
                self.status = "The authoring slide was removed.".into();
                cx.notify();
                return;
            };
            let Some(target) = slides.iter_mut().find(|slide| slide["id"] == self.slide_id) else {
                self.status = "The authoring slide was removed.".into();
                cx.notify();
                return;
            };
            *target = edited;
            operation.mutation["operation"]["slides"] = json!(slides);
        }
        let runtime = self.runtime.clone();
        let path = runtime.content.read(cx).workspace_path.clone();
        let scope = path.clone();
        let host = runtime.host.clone();
        let revisions = runtime.lanes.revisions().clone();
        let kind = self.kind;
        let current = (self.theme_id.clone(), self.slide_id.clone());
        let mask_reorder =
            operation.method == "mutateMasks" && operation.mutation["type"] == "reorder";
        let removed_current_mask = operation.method == "mutateMasks"
            && operation.mutation["type"] == "delete"
            && operation.mutation["maskId"] == current.0;
        let removed_index = if removed_current_mask {
            self.document_choices(cx)
                .iter()
                .position(|(id, _, _)| id == &current.0)
        } else {
            None
        };
        let previewing = self.editor.document_preview.is_some();
        let answer = runtime.lanes.workspace(&path, move |revision| {
            let mut mutation = operation.mutation;
            if operation.method == "mutateStageContent" && mutation["type"] == "replaceLayout" {
                let latest = host.invoke("readWorkspaceSnapshot", vec![json!(scope)])?;
                let mut layout = latest["workspace"]["stageLayouts"]
                    .as_array()
                    .and_then(|layouts| {
                        layouts
                            .iter()
                            .find(|layout| layout["id"] == mutation["layoutId"])
                    })
                    .cloned()
                    .ok_or("The stage layout was removed.")?;
                layout["title"] = mutation["layout"]["title"].clone();
                mutation["layout"] = layout;
            }
            let mut result = host.invoke(
                operation.method,
                vec![json!(scope), mutation, json!(revision)],
            )?;
            revisions.remember(&scope, &result);
            if matches!(
                operation.method,
                "mutateProps" | "mutateMasks" | "mutateStageContent"
            ) {
                // These mutations return saved sections; the document loader needs a complete snapshot.
                let canonical = host.invoke("readWorkspaceSnapshot", vec![json!(scope)])?;
                result["workspace"] = canonical["workspace"].clone();
            }
            let selected = match operation.created {
                Some(key) => Some(
                    result[key]
                        .as_str()
                        .ok_or("The native creation returned no document identity.")?
                        .to_string(),
                ),
                None => None,
            };
            let selected_slide = result["selectedSlideId"].clone();
            let mut result = result;
            result["editorCreatedId"] = json!(selected);
            result["editorSelectedSlide"] = selected_slide;
            Ok(result)
        });
        self.saving = true;
        self.save_id += 1;
        let nonce = self.save_id;
        cx.spawn(async move |view, cx| {
            let result = answer
                .await
                .map_err(|_| "The document operation worker stopped.".to_string())
                .and_then(|result| result);
            let result = cx
                .background_spawn(async move {
                    let result = result?;
                    Ok::<_, String>((
                        crate::collections::model::Snapshot::parse(&result["workspace"])?,
                        result["editorCreatedId"].as_str().map(str::to_string),
                        result["editorSelectedSlide"].as_str().map(str::to_string),
                    ))
                })
                .await;
            view.update(cx, |view, cx| {
                if nonce != view.save_id || runtime.content.read(cx).workspace_path != path {
                    return;
                }
                view.saving = false;
                if mask_reorder {
                    view.editor.rows.mask_order = None;
                }
                match result {
                    Ok((snapshot, created, selected_slide)) => {
                        runtime.content.update(cx, |content, cx| {
                            content.apply_document(snapshot);
                            cx.notify();
                        });
                        if kind != DocumentKind::Slide {
                            crate::cues::refresh_live(&runtime, cx);
                        }
                        if removed_current_mask {
                            let choices = view.document_choices(cx);
                            let target = removed_index
                                .and_then(|index| choices.get(index).or_else(|| choices.last()));
                            let (owner, slide) = target
                                .map_or(("", ""), |(id, slide, _)| (id.as_str(), slide.as_str()));
                            view.open_kind(kind, owner, slide, cx);
                            view.finish_document_save(cx);
                            cx.notify();
                            return;
                        }
                        let target = if kind == DocumentKind::Slide {
                            if let Some(slide) = created.or(selected_slide) {
                                (current.0.clone(), slide)
                            } else {
                                let slides = view.layout_choices(cx);
                                if slides.iter().any(|slide| slide.value == current.1) {
                                    current.clone()
                                } else if slides.is_empty() {
                                    (current.0.clone(), String::new())
                                } else {
                                    view.status =
                                        "The native slide operation returned no selected slide."
                                            .into();
                                    cx.notify();
                                    return;
                                }
                            }
                        } else if let Some(id) = created {
                            let Some((owner, slide, _)) = view
                                .document_choices(cx)
                                .into_iter()
                                .find(|(owner, _, _)| owner == &id)
                            else {
                                view.status =
                                    "The created authoring document is unavailable.".into();
                                cx.notify();
                                return;
                            };
                            (owner, slide)
                        } else {
                            current.clone()
                        };
                        if current.0.is_empty()
                            || current.1.is_empty()
                            || (kind == DocumentKind::Slide
                                && !view
                                    .layout_choices(cx)
                                    .iter()
                                    .any(|slide| slide.value == current.1))
                        {
                            view.rich = None;
                            view.open_kind(kind, &target.0, &target.1, cx);
                            view.finish_document_save(cx);
                        } else {
                            if target != current {
                                view.editor.navigate = Some((kind, target.0, target.1));
                            }
                            match open_document(&runtime, kind, &current.0, &current.1, cx) {
                                Ok(document) => {
                                    view.set_slide(document.slide());
                                    view.document = Some(document);
                                    if !previewing {
                                        view.editor.scope.clear();
                                    }
                                    view.status.clear();
                                    view.flush_rich(cx);
                                    if view.editor.base.is_some() {
                                        view.save((*view.slide).clone(), History::Edit, cx);
                                    } else {
                                        view.finish_document_save(cx);
                                    }
                                }
                                Err(error) => view.status = error,
                            }
                        }
                    }
                    Err(error) => {
                        if view.editor.document_preview.is_some() {
                            view.editor.document_operations.clear();
                            view.editor.document_history = false;
                            if let Err(restore_error) = view.refresh_document_preview(cx) {
                                view.status = format!("{error} {restore_error}");
                            } else {
                                view.status = error;
                            }
                            view.editor.scope.clear();
                        } else {
                            view.status = error;
                        }
                    }
                }
                view.report_background_save_error(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    pub(super) fn finish_document_save(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.refresh_document_preview(cx) {
            self.editor.document_operations.clear();
            self.status = error;
            cx.notify();
            return;
        }
        if let Some(operation) = self.editor.document_operations.pop_front() {
            self.save_document_operation(operation, cx);
            return;
        }
        if let Some(operation) = self.editor.pending_operation.take() {
            self.save_document_operation(operation, cx);
            return;
        }
        self.finish_slide_action(cx);
        if let Some((kind, owner, slide)) = self.editor.navigate.take() {
            self.open_kind(kind, &owner, &slide, cx);
        }
        self.emit_close_if_requested(cx);
    }
}

fn store_editor_slide(root: &mut Value, edited: &Value, workspace: &Value) -> Result<(), String> {
    let raw = root["slides"]
        .as_array_mut()
        .and_then(|slides| slides.iter_mut().find(|slide| slide["id"] == edited["id"]))
        .ok_or("The authoring slide was removed.")?;
    let theme = orbit_host::authoring::slide_theme(workspace, raw);
    *raw = orbit_host::authoring::store(raw, edited, theme.as_ref())?;
    Ok(())
}

fn preview_item_operation(root: &mut Value, operation: &Value) -> Result<(), String> {
    let kind = operation["type"]
        .as_str()
        .ok_or("Document operation type is required.")?;
    if kind == "setPresentationSetting" {
        return orbit_host::authoring::patch_presentation_setting(
            root,
            operation["field"]
                .as_str()
                .ok_or("Presentation setting field is required.")?,
            operation
                .get("value")
                .ok_or("Presentation setting value is required.")?,
        );
    }
    if kind == "setSlideDocument" {
        let slides = operation["slides"]
            .as_array()
            .ok_or("Presentation slides must be an array.")?;
        root["slides"] = json!(slides);
        return Ok(());
    }
    if kind == "setTransition" {
        let transition = operation
            .get("transition")
            .ok_or("Transition is required.")?;
        if !transition.is_null() {
            orbit_host::native_transitions::validate(transition)?;
        }
        let target = if let Some(id) = operation.get("slideId") {
            root["slides"]
                .as_array_mut()
                .and_then(|slides| slides.iter_mut().find(|slide| slide["id"] == *id))
                .ok_or("Transition slide was removed.")?
        } else {
            root
        };
        let target = target
            .as_object_mut()
            .ok_or("Transition target must be an object.")?;
        if transition.is_null() {
            target.remove("transition");
        } else {
            target.insert("transition".into(), transition.clone());
        }
        return Ok(());
    }
    let slides = root["slides"]
        .as_array_mut()
        .ok_or("Presentation slides must be an array.")?;
    let index = slides
        .iter()
        .position(|slide| slide["id"] == operation["slideId"])
        .ok_or("The authoring slide was removed.")?;
    if kind == "setSlideShortcut" {
        let shortcut = operation["shortcut"]
            .as_str()
            .ok_or("Slide shortcut must be text.")?
            .trim()
            .to_ascii_lowercase();
        if !shortcut.is_empty()
            && !(shortcut.len() == 1 && shortcut.bytes().all(|key| key.is_ascii_alphabetic()))
        {
            return Err("Slide shortcut must be a single letter from A to Z.".into());
        }
        if !shortcut.is_empty()
            && slides.iter().enumerate().any(|(i, slide)| {
                i != index
                    && slide["shortcut"]
                        .as_str()
                        .is_some_and(|key| key.eq_ignore_ascii_case(&shortcut))
            })
        {
            return Err("Another slide already uses this shortcut.".into());
        }
        let slide = slides[index]
            .as_object_mut()
            .ok_or("Slide must be an object.")?;
        if shortcut.is_empty() {
            slide.remove("shortcut");
        } else {
            slide.insert("shortcut".into(), json!(shortcut));
        }
        return Ok(());
    }
    let slide = slides[index]
        .as_object_mut()
        .ok_or("Slide must be an object.")?;
    match kind {
        "setSlideEnabled" => {
            let enabled = operation["enabled"]
                .as_bool()
                .ok_or("Slide enabled state must be a boolean.")?;
            if enabled {
                slide.remove("disabled");
            } else {
                slide.insert("disabled".into(), json!(true));
            }
        }
        "setSlideGroup" => {
            let group = operation.get("groupId").ok_or("Slide group is required.")?;
            if group.is_null() {
                for key in ["groupId", "groupName", "sectionId"] {
                    slide.remove(key);
                }
            } else {
                let id = group
                    .as_str()
                    .filter(|id| !id.trim().is_empty())
                    .ok_or("Slide group must be a non-empty string or null.")?;
                slide.insert("groupId".into(), json!(id.trim()));
            }
        }
        _ => return Err("Unsupported document preview operation.".into()),
    }
    Ok(())
}
