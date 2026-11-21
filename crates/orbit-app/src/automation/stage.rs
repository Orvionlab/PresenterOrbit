//! The Stage utility tab (`StageOutputPanel.tsx`, `stage-content-controller.ts`): which layout each stage screen
//! shows, new layouts, and the stage message. Screen edits are `mutateScreenConfig` intents, layout and message
//! edits are `mutateStageContent` intents; a message edit also reaches the operator's live state so the stage
//! screens show it.

use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, relative, Context, Entity, Role, SharedString, Subscription, Window,
};
use orbit_ui::controls::{
    textarea_state, ActionButton, ButtonVariant, EmptyState, Field, IconButton, InputEvent,
    PanelHeader, Popover, Textarea, TextareaState,
};
use orbit_ui::{icon, ActiveTheme, FrameSize, Translator};
use serde_json::{json, Value};

use super::form::{get, Form, Kind, Spec};
use super::props::Opener;
use crate::canvas::DocumentKind;
use crate::output::preview::{Frames, Preview, Source};
use crate::state::runtime::Runtime;

/// `getStageEntries`: the stage screens of a screen configuration, in its order.
pub fn stage_screens(config: &Value) -> Vec<(String, Value)> {
    config
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(id, screen)| screen["type"] == "stage" || id.starts_with("stage-"))
        .map(|(id, screen)| (id.clone(), screen.clone()))
        .collect()
}

/// The saved assignments with one screen's layout set.
fn with_assignment(assignments: &Value, screen_id: &str, layout_id: &str) -> Value {
    let mut next: Vec<Value> = assignments
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["screenId"] != screen_id)
        .cloned()
        .collect();
    next.push(json!({"screenId": screen_id, "layoutId": layout_id}));
    Value::Array(next)
}

/// Shows the layout at once in the row and the stage output, then saves it; a rejected save restores the old list.
fn assign_layout(runtime: &Runtime, screen_id: &str, layout_id: &str, cx: &mut gpui::App) {
    let previous = runtime.content.read(cx).workspace["stageScreenAssignments"].clone();
    let next = with_assignment(&previous, screen_id, layout_id);
    let set = |value: Value| {
        let runtime = runtime.clone();
        move |cx: &mut gpui::App| {
            // An automation stage action keeps its own copy in the live state, which wins over the workspace.
            runtime.output.update(cx, |output, cx| {
                if output.live.get("stageScreenAssignments").is_some() {
                    output.edit_live(|live| live["stageScreenAssignments"] = value.clone());
                    cx.notify();
                }
            });
            runtime.content.update(cx, |content, cx| {
                Arc::make_mut(&mut content.workspace)["stageScreenAssignments"] = value;
                cx.notify();
            });
        }
    };
    set(next)(cx);
    crate::output::windows::mutate_config_with(
        runtime,
        json!({"type": "setStageAssignment", "screenId": screen_id, "layoutId": layout_id}),
        Some(Box::new(set(previous))),
        cx,
    );
}

pub struct StagePanel {
    runtime: Runtime,
    open: Opener,
    status: String,
    busy: bool,
    form: Form,
    /// The screen being renamed.
    renaming: Option<String>,
    menu: Option<String>,
    picker: Option<String>,
    /// Its own bitmap cache, so thumbnails never queue behind the cue grid's stills.
    frames: Frames,
    /// Row thumbnails by screen id and picker thumbnails by layout id; the picker's are dropped when it closes.
    previews: HashMap<String, Entity<Preview>>,
    creating: bool,
    message: Option<(Entity<TextareaState>, Subscription)>,
    cleared: bool,
}

fn title_form(panel: &mut StagePanel) -> &mut Form {
    &mut panel.form
}

const PICKER_COLUMNS: usize = 3;
const PICKER_WIDTH: f32 = 396.;
const PICKER_MAX_HEIGHT: f32 = 460.;
const THUMB_WIDTH: f32 = 136.;

/// A layout preview on a matte, outlined in the selection color when it is the assigned one.
fn thumbnail(
    preview: Option<Entity<Preview>>,
    current: bool,
    cx: &mut Context<StagePanel>,
) -> gpui::Div {
    let theme = *cx.theme();
    div()
        .relative()
        .w_full()
        .aspect_ratio(16. / 9.)
        .overflow_hidden()
        .rounded(theme.metrics.radius_xs)
        .border_1()
        .border_color(if current {
            theme.colors.selected
        } else {
            theme.colors.border_soft
        })
        .bg(theme.colors.preview_matte_bg)
        .children(preview.map(|preview| div().absolute().size_full().child(preview)))
}

fn small(
    id: impl Into<SharedString>,
    label: String,
    variant: ButtonVariant,
    cx: &mut Context<StagePanel>,
    act: impl Fn(&mut StagePanel, &mut Context<StagePanel>) + 'static,
) -> ActionButton {
    ActionButton::new(id.into(), label)
        .variant(variant)
        .on_click(cx.listener(move |panel, _, _, cx| act(panel, cx)))
}

impl StagePanel {
    pub fn new(runtime: Runtime, open: Opener, cx: &mut Context<Self>) -> Self {
        cx.observe(&runtime.content, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&runtime.output, |_, _, cx| cx.notify()).detach();
        let mut form = Form::default();
        form.reset(json!({"title": ""}));
        Self {
            runtime,
            open,
            status: String::new(),
            busy: false,
            form,
            renaming: None,
            menu: None,
            picker: None,
            frames: Frames::default(),
            previews: HashMap::new(),
            creating: false,
            message: None,
            cleared: false,
        }
    }

    fn screens(&self, cx: &Context<Self>) -> Value {
        let local = self.runtime.content.read(cx).workspace["screenConfig"].clone();
        match local.as_object() {
            Some(local) if !local.is_empty() => Value::Object(local.clone()),
            _ => self.runtime.output.read(cx).settings["screenConfig"].clone(),
        }
    }

    /// One stage content mutation. The message of the answer also becomes the live stage message.
    fn stage(&mut self, mutation: Value, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status.clear();
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let outcome = super::save::mutation(&runtime, cx, "mutateStageContent", mutation).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match outcome {
                    Ok(response) => {
                        let message = response["workspace"]["stageMessage"].clone();
                        if !message.is_null() {
                            crate::cues::actions::edit_native(&this.runtime, cx, move |live| {
                                live["stageMessage"] = message;
                                Ok(())
                            });
                        }
                    }
                    Err(error) => this.status = error,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn set_picker(&mut self, picker: Option<String>, cx: &mut Context<Self>) {
        self.previews.retain(|key, _| !key.starts_with("pick:"));
        self.picker = picker;
        cx.notify();
    }

    /// The thumbnail of a layout, created the first time it is shown. `None` when the layout has no valid size.
    fn preview(
        &mut self,
        key: String,
        layout: &Value,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Preview>> {
        // The test dispatcher never parks while native stills render on their own threads.
        if cfg!(test) {
            return None;
        }
        let slide = layout["slide"].clone();
        let size = FrameSize {
            width: slide["width"].as_f64().filter(|w| *w >= 1.)?.ceil() as u32,
            height: slide["height"].as_f64().filter(|h| *h >= 1.)?.ceil() as u32,
        };
        // The key carries the layout's content, so an edited layout never shows a stale bitmap.
        let mut hasher = DefaultHasher::new();
        slide.to_string().hash(&mut hasher);
        let source = Source::StageLayout {
            key: format!(
                "stage-layout:{}:{}:{:x}",
                self.runtime.content.read(cx).workspace_path,
                layout["id"].as_str()?,
                hasher.finish()
            ),
            slide: Arc::new(slide),
        };
        let (host, frames) = (self.runtime.host.clone(), self.frames.clone());
        let preview = self
            .previews
            .entry(key)
            .or_insert_with(|| cx.new(|cx| Preview::new(host, frames, cx)))
            .clone();
        preview.update(cx, |preview, cx| {
            preview.set_source(Some(source), Some(size), 0, cx)
        });
        Some(preview)
    }

    fn screen_patch(&mut self, screen_id: &str, patch: Value, cx: &mut Context<Self>) {
        let screens = self.screens(cx);
        let Some(current) = screens.get(screen_id) else {
            return;
        };
        let mutation = crate::settings::video::update_mutation(screen_id, current, patch);
        crate::output::windows::mutate_config(&self.runtime, mutation, cx);
    }

    /// The stage message field keeps its own state; each edit is saved as typed, in order, as React does.
    fn message_field(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        if let Some((state, _)) = &self.message {
            return state.clone();
        }
        let text = if std::mem::take(&mut self.cleared) {
            ""
        } else {
            text
        };
        let state = textarea_state(text.to_string(), window, cx);
        let subscription = cx.subscribe(&state, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = state.read(cx).value().to_string();
                this.stage_unguarded(json!({"type": "setMessageText", "text": text}), cx);
            }
        });
        self.message = Some((state.clone(), subscription));
        state
    }

    /// Typing is never dropped because an earlier save is still running; the lane keeps the order.
    fn stage_unguarded(&mut self, mutation: Value, cx: &mut Context<Self>) {
        self.busy = false;
        self.stage(mutation, cx);
    }
}

impl Render for StagePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t: Translator = Runtime::translator(cx);
        let theme = *cx.theme();
        let gap = theme.metrics.space_xs;
        let secondary = ButtonVariant::Secondary;
        let workspace = self.runtime.content.read(cx).workspace.clone();
        let layouts = workspace["stageLayouts"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        let screens = stage_screens(&self.screens(cx));
        let message = workspace["stageMessage"].clone();
        let visible = message["state"] == "visible";
        let workspace_open = !self.runtime.content.read(cx).workspace_path.is_empty();
        let mut column = div()
            .id("stage-panel")
            .debug_selector(|| "stage-panel".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(gap)
            .p(gap)
            .flex_1()
            .min_h_0();
        column = column.child(
            PanelHeader::new(t.t("utilityPanels.stage"))
                .prominent()
                .action(
                    IconButton::new("stage-create", "plus", t.t("stage.newLayout"))
                        .large_icon()
                        .disabled(!workspace_open)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.creating = !this.creating;
                            this.form.reset(
                                json!({"title": Runtime::translator(cx).t("stage.newLayout")}),
                            );
                            cx.notify();
                        })),
                ),
        );
        if self.creating {
            column = column
                .child(self.form.field(
                    "stage-new-layout",
                    &Spec::new("title", t.t("stage.newLayout"), Kind::Text),
                    title_form,
                    window,
                    cx,
                ))
                .child(small(
                    "stage-new-layout-save",
                    t.t("toolbar.editor.stage"),
                    ButtonVariant::Primary,
                    cx,
                    |this, cx| {
                        let title = get(&this.form.draft, "title")
                            .as_str()
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if !title.is_empty() {
                            this.creating = false;
                            this.stage(json!({"type":"createLayout", "title": title}), cx);
                        }
                    },
                ));
        }
        let mut screen_list = div()
            .id("stage-screens")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if screens.is_empty() {
            screen_list = screen_list.child(EmptyState::new(t.t("stage.noScreens")));
        }
        let deletable = screens.len() > 1;
        for (id, screen) in screens {
            let label = screen["label"]
                .as_str()
                .filter(|label| !label.is_empty())
                .unwrap_or(&id)
                .to_string();
            let weak = cx.weak_entity();
            let menu_id = id.clone();
            let chosen =
                crate::settings::video_extra::effective_stage_layout(&screen, &workspace, &id);
            let selected = chosen.and_then(|layout| layout["id"].as_str());
            let title = chosen
                .and_then(|layout| layout["title"].as_str())
                .map_or_else(|| t.t("stage.noLayout"), str::to_string);
            let open = chosen.is_some() && self.picker.as_deref() == Some(id.as_str());
            let shown = chosen.and_then(|layout| self.preview(format!("row:{id}"), layout, cx));
            let trigger = div()
                .id(SharedString::from(format!("stage-picker-{id}")))
                .debug_selector(|| format!("stage-picker-{id}"))
                .role(Role::Button)
                .aria_label(t.t("stage.theme"))
                .w_full()
                .flex()
                .items_center()
                .gap(theme.metrics.space_xs)
                .when(chosen.is_some(), |trigger| {
                    trigger
                        .cursor_pointer()
                        .on_click(cx.listener({
                            let id = id.clone();
                            move |this, _, _, cx| {
                                let next = (this.picker.as_deref() != Some(id.as_str()))
                                    .then(|| id.clone());
                                this.set_picker(next, cx);
                            }
                        }))
                        .child(
                            icon("arrows-up-down")
                                .text_color(theme.colors.text_muted)
                                .flex_none(),
                        )
                })
                .child(div().flex_1().min_w_0().child(thumbnail(shown, open, cx)));
            let mut popover =
                Popover::new(SharedString::from(format!("stage-popover-{id}")), trigger)
                    .anchor(gpui::Anchor::TopRight)
                    .on_dismiss({
                        let weak = weak.clone();
                        move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| this.set_picker(None, cx));
                        }
                    });
            if open {
                let cells = layouts
                    .iter()
                    .filter(|layout| layout["slide"].is_object())
                    .filter_map(|layout| {
                        let value = layout["id"].as_str().filter(|id| !id.is_empty())?;
                        let name = layout["title"].as_str().unwrap_or("").to_string();
                        let preview = self.preview(format!("pick:{value}"), layout, cx);
                        let current = Some(value) == selected;
                        let (weak, screen_id, chosen) =
                            (weak.clone(), id.clone(), value.to_string());
                        Some(
                            div()
                                .id(SharedString::from(format!("stage-pick-{id}-{value}")))
                                .debug_selector(|| format!("stage-pick-{id}-{value}"))
                                .role(Role::Button)
                                .aria_label(name.clone())
                                .aria_selected(current)
                                .w(relative(1. / PICKER_COLUMNS as f32))
                                .flex_none()
                                .p(gap)
                                .cursor_pointer()
                                .on_click(move |_, _, cx| {
                                    let (screen_id, chosen) = (screen_id.clone(), chosen.clone());
                                    let _ = weak.update(cx, |this, cx| {
                                        this.set_picker(None, cx);
                                        assign_layout(&this.runtime, &screen_id, &chosen, cx)
                                    });
                                })
                                .child(thumbnail(preview, current, cx))
                                .child(
                                    div()
                                        .pt(theme.metrics.space_2xs)
                                        .text_size(theme.metrics.font_sm)
                                        .text_color(theme.colors.text)
                                        .truncate()
                                        .child(name),
                                ),
                        )
                    })
                    .collect::<Vec<_>>();
                popover = popover.content(
                    div()
                        .id(SharedString::from(format!("stage-picker-panel-{id}")))
                        .role(Role::Dialog)
                        .aria_label(t.t("stage.theme"))
                        .w(px(PICKER_WIDTH))
                        .max_h(px(PICKER_MAX_HEIGHT))
                        .overflow_y_scroll()
                        .flex()
                        .flex_wrap()
                        .p(gap)
                        .border_1()
                        .border_color(theme.colors.border)
                        .rounded(theme.metrics.radius_md)
                        .bg(theme.colors.popover_bg)
                        .shadow(theme.shadow_soft())
                        .children(cells),
                );
            }
            let mut row = div()
                .id(SharedString::from(format!("stage-row-{id}")))
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |this, _, _, cx| {
                        this.menu = Some(menu_id.clone());
                        cx.notify();
                    }),
                )
                .flex()
                .items_center()
                .gap(gap)
                .py(gap)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().truncate().child(label.clone()))
                        .child(
                            div()
                                .truncate()
                                .text_size(theme.metrics.font_sm)
                                .text_color(theme.colors.text_muted)
                                .child(title),
                        ),
                )
                .child(div().flex_none().w(px(THUMB_WIDTH)).child(popover));
            if self.menu.as_deref() == Some(id.as_str()) {
                row = row.child(small(
                    format!("stage-rename-{id}"),
                    t.t("common.rename"),
                    secondary,
                    cx,
                    {
                        let (id, label) = (id.clone(), label.clone());
                        move |this, cx| {
                            this.renaming = Some(id.clone());
                            this.form.reset(json!({"title": label}));
                            cx.notify();
                        }
                    },
                ));
                row = row.child(
                    small(
                        format!("stage-delete-{id}"),
                        t.t("common.delete"),
                        ButtonVariant::Danger,
                        cx,
                        {
                            let id = id.clone();
                            move |this, cx| {
                                crate::output::windows::mutate_config(
                                    &this.runtime,
                                    json!({"type": "delete", "screenId": id}),
                                    cx,
                                )
                            }
                        },
                    )
                    .disabled(!deletable),
                );
                if let Some((layout, layout_id)) = chosen.zip(selected) {
                    let (owner, slide, opener) = (
                        layout_id.to_string(),
                        layout["slide"]["id"].as_str().unwrap_or("").to_string(),
                        self.open.clone(),
                    );
                    row = row.child(small(
                        format!("stage-configure-{id}"),
                        t.t("stage.configureScreen"),
                        secondary,
                        cx,
                        move |_, cx| opener(DocumentKind::Stage, &owner, &slide, cx),
                    ));
                }
            }
            screen_list = screen_list.child(row);
            if self.renaming.as_deref() == Some(id.as_str()) {
                screen_list = screen_list.child(
                    div()
                        .flex()
                        .gap(gap)
                        .child(self.form.field(
                            "stage-name",
                            &Spec::new("title", t.t("common.rename"), Kind::Text),
                            title_form,
                            window,
                            cx,
                        ))
                        .child(small(
                            format!("stage-rename-save-{id}"),
                            t.t("automation.macros.save"),
                            ButtonVariant::Primary,
                            cx,
                            {
                                let id = id.clone();
                                move |this, cx| {
                                    let label = get(&this.form.draft, "title")
                                        .as_str()
                                        .unwrap_or("")
                                        .trim()
                                        .to_string();
                                    this.renaming = None;
                                    if !label.is_empty() {
                                        this.screen_patch(&id, json!({"label": label}), cx);
                                    }
                                    this.form.reset(json!({"title": ""}));
                                    cx.notify();
                                }
                            },
                        ))
                        .child(small(
                            format!("stage-rename-cancel-{id}"),
                            t.t("common.cancel"),
                            secondary,
                            cx,
                            |this, cx| {
                                this.renaming = None;
                                this.form.reset(json!({"title": ""}));
                                cx.notify();
                            },
                        )),
                );
            }
        }
        column = column.child(screen_list);
        let area = self.message_field(message["text"].as_str().unwrap_or(""), window, cx);
        column = column.child(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(gap)
                .child(Field::new(t.t("stage.message"), Textarea::new("stage-message", &area).rows(2.)))
                .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(gap)
                    .child(small("stage-message-clear", t.t("stage.clear"), ButtonVariant::Primary, cx, |this, cx| {
                        // The field is rebuilt empty on the next frame.
                        this.message = None;
                        this.cleared = true;
                        this.stage_unguarded(json!({"type": "clearMessage"}), cx);
                    }))
                    .child(small(
                        "stage-message-toggle",
                        t.t(if visible { "stage.hide" } else { "stage.show" }),
                        ButtonVariant::Primary,
                        cx,
                        move |this, cx| {
                            this.stage_unguarded(
                                json!({"type": "setMessageState", "state": if visible { "hidden" } else { "visible" }}),
                                cx,
                            )
                        },
                    )),
            ),
        );
        if !self.status.is_empty() {
            column = column.child(
                div()
                    .id("stage-status")
                    .role(gpui::Role::Alert)
                    .child(t.t(&self.status)),
            );
        }
        column
    }
}
