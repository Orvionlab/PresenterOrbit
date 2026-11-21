use crate::cues::actions;
use gpui::{
    div, prelude::*, Context, Entity, FocusHandle, MouseButton, Pixels, Point, Render,
    Subscription, WeakEntity, Window,
};
use orbit_ui::controls::NativeButton as Button;
use orbit_ui::controls::{
    tooltip_text, ContextMenu, EmptyState, MenuEntry, MenuItem, Select, SelectOption,
};
use orbit_ui::{ActiveTheme, FrameSize, Translator};
use serde_json::Value;

use super::preview::{Preview, Source};
use crate::collections::model::natural_cmp;
use crate::state::runtime::Runtime;

/// `getPreviewScreenEntries` / `getPreviewScreenEntry`: audience first, then stage; prefer an enabled screen.
pub fn screens(snapshot: &Value) -> Vec<(String, Value)> {
    let Some(config) = snapshot["screenConfig"].as_object() else {
        return Vec::new();
    };
    let mut screens = config
        .iter()
        .map(|(id, config)| (id.clone(), config.clone()))
        .collect::<Vec<_>>();
    screens.sort_by(|(a, first), (b, second)| {
        let stage =
            |id: &str, config: &Value| config["type"] == "stage" || id.starts_with("stage-");
        stage(a, first)
            .cmp(&stage(b, second))
            .then_with(|| natural_cmp(a, b))
    });
    screens
}

pub fn selected_screen<'a>(
    screens: &'a [(String, Value)],
    selected: &str,
) -> Option<&'a (String, Value)> {
    screens
        .iter()
        .find(|(id, _)| id == selected)
        .or_else(|| screens.iter().find(|(_, config)| config["enabled"] == true))
        .or(screens.first())
}

/// The documented `normalizeOutputSize` default.
pub fn screen_size(config: &Value) -> FrameSize {
    let (width, height) = if config["canvasSize"].is_object() {
        (
            &config["canvasSize"]["width"],
            &config["canvasSize"]["height"],
        )
    } else if config["targetType"] == "ndi" {
        (&config["ndiWidth"], &config["ndiHeight"])
    } else {
        (&config["width"], &config["height"])
    };
    match (width.as_f64(), height.as_f64()) {
        (Some(width), Some(height))
            if width.is_finite()
                && height.is_finite()
                && width.round() > 0.
                && height.round() > 0. =>
        {
            FrameSize {
                width: width.round() as u32,
                height: height.round() as u32,
            }
        }
        _ => FrameSize {
            width: 1920,
            height: 1080,
        },
    }
}

pub fn screen_size_for_displays(config: &Value, displays: &Value) -> FrameSize {
    if config["canvasSize"].is_object() {
        return screen_size(config);
    }
    if config["targetType"] == "display" {
        if let Some(display) = displays.as_array().and_then(|displays| {
            displays
                .iter()
                .find(|display| display["id"] == config["targetId"])
        }) {
            return screen_size(&display["size"]);
        }
    }
    screen_size(config)
}

pub struct OutputView {
    runtime: Runtime,
    preview: Entity<Preview>,
    meters: Entity<super::meters::ProgramMeters>,
    _subscription: Subscription,
    menu: Option<Point<Pixels>>,
    focus: FocusHandle,
    owner: WeakEntity<crate::view::AppView>,
    clear_bounds: std::rc::Rc<std::cell::Cell<gpui::Bounds<Pixels>>>,
}

impl OutputView {
    pub fn new(
        runtime: Runtime,
        owner: WeakEntity<crate::view::AppView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let preview = cx.new(|cx| Preview::new(runtime.host.clone(), runtime.frames.clone(), cx));
        let meters = cx.new(|cx| super::meters::ProgramMeters::new(&runtime, cx));
        let subscription = cx.observe(&runtime.output, |_, _, cx| cx.notify());
        Self {
            runtime,
            preview,
            meters,
            _subscription: subscription,
            menu: None,
            focus: cx.focus_handle(),
            owner,
            clear_bounds: Default::default(),
        }
    }
}

impl OutputView {
    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn clear_menu(&self, cx: &mut Context<Self>) -> Option<ContextMenu> {
        let position = self.menu?;
        let t = *cx.global::<Translator>();
        let output = self.runtime.output.read(cx);
        let mut entries: Vec<MenuEntry> = Vec::new();
        if let Some(groups) = output
            .snapshot
            .as_ref()
            .and_then(|s| s["clearGroups"].as_array())
        {
            for group in groups.iter().filter(|g| g["hiddenInPreview"] != true) {
                let Some(id) = group["id"].as_str().map(str::to_owned) else {
                    continue;
                };
                let title = group["title"].as_str().unwrap_or(&id).to_owned();
                let weak = cx.weak_entity();
                entries.push(
                    MenuItem::new(title)
                        .on_click(move |_, window, cx| {
                            weak.update(cx, |view, cx| {
                                view.close_menu(window, cx);
                                actions::clear_group(&view.runtime, &id, cx);
                            })
                            .ok();
                        })
                        .into(),
                );
            }
        }
        let weak = cx.weak_entity();
        entries.push(
            MenuItem::new(t.t("presentation.command.clearToLogo"))
                .on_click(move |_, window, cx| {
                    weak.update(cx, |view, cx| {
                        view.close_menu(window, cx);
                        actions::clear(&view.runtime, "logo", cx);
                    })
                    .ok();
                })
                .into(),
        );
        let weak = cx.weak_entity();
        entries.push(
            MenuItem::new(t.t("clearGroups.configure"))
                .on_click(move |_, window, cx| {
                    weak.update(cx, |view, cx| {
                        view.close_menu(window, cx);
                        crate::view::request_auxiliary(
                            view.runtime.host.clone(),
                            serde_json::json!({"kind":"settings","panel":"groups"}),
                            view.owner.clone(),
                            cx,
                        );
                    })
                    .ok();
                })
                .into(),
        );
        let weak = cx.weak_entity();
        Some(ContextMenu::new(
            "preview-clear-menu",
            position,
            entries,
            move |_, window, cx| {
                weak.update(cx, |view, cx| view.close_menu(window, cx)).ok();
            },
        ))
    }
}

impl Render for OutputView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let output = self.runtime.output.read(cx);
        let entries = output
            .snapshot
            .as_ref()
            .map_or_else(Vec::new, |snapshot| screens(snapshot));
        let selected = selected_screen(&entries, &output.selected_screen);
        let source = selected.map(|(id, _)| Source::Output(id.clone()));
        let size = selected.map(|(_, config)| screen_size_for_displays(config, &output.displays));
        let selected_id = selected.map(|(id, _)| id.clone());
        let revision = output.committed_revision;
        let error = output
            .link_error
            .clone()
            .or_else(|| output.error.clone())
            .or_else(|| output.device_error.clone());
        let settings = output.settings.clone();
        let frozen = output.live["outputFrozen"] == true;
        self.meters
            .update(cx, |meters, cx| meters.set_layout(&settings, cx));
        self.preview
            .update(cx, |view, cx| view.set_source(source, size, revision, cx));
        let runtime = self.runtime.clone();
        let settings_host = self.runtime.host.clone();
        let settings_owner = self.owner.clone();
        let options = entries
            .iter()
            .map(|(id, config)| {
                SelectOption::new(
                    id.clone(),
                    config["label"].as_str().unwrap_or(id).to_string(),
                )
            })
            .collect();
        let ready = revision > 0;
        let clear_runtime = self.runtime.clone();
        let clear_label = t.t("presentation.command.clearAll");
        let clear_bounds = self.clear_bounds.clone();
        let clear = Button::new("cue-clear-all")
            .debug_selector(|| "cue-clear-all".into())
            .track_focus(&self.focus)
            .accessibility_label(clear_label.clone())
            .tooltip(tooltip_text(clear_label.clone()))
            .disabled(!ready)
            .h_full()
            .flex_none()
            .w(theme.metrics.control_height)
            .min_w_0()
            .bg(theme.colors.border)
            .text_color(theme.colors.text)
            .hover(|s| {
                s.bg(orbit_ui::color::mix_srgb(
                    theme.colors.danger,
                    0.42,
                    theme.colors.panel_bg,
                ))
            })
            .focus_visible(|s| s.border_2().border_color(theme.colors.focus_ring))
            .child(
                orbit_ui::icon("circle-x")
                    .size(theme.metrics.control_height_compact)
                    .text_color(theme.colors.text),
            )
            .child(
                gpui::canvas(
                    move |bounds, _, _| clear_bounds.set(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .on_click(move |_, _, cx| actions::clear(&clear_runtime, "all", cx))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|view, event: &gpui::MouseDownEvent, _, cx| {
                    view.menu = Some(event.position);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "contextmenu"
                    || (event.keystroke.key == "f10" && event.keystroke.modifiers.shift)
                {
                    view.menu = Some(view.clear_bounds.get().bottom_left());
                    cx.stop_propagation();
                    cx.notify();
                }
            }));
        let menu = self.clear_menu(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .overflow_hidden()
                    .child(
                        div()
                            .debug_selector(|| "program-preview".into())
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .relative()
                            .child(self.preview.clone())
                            .child(self.meters.clone()),
                    )
                    .child(clear)
                    .child(crate::cues::layer_rail(&self.runtime, cx)),
            )
            .when_some(error, |element, error| {
                element.child(
                    div()
                        .id("output-clear-error")
                        .role(gpui::Role::Alert)
                        .child(EmptyState::new(t.t(&error))),
                )
            })
            .child(
                div()
                    .flex_none()
                    .border_t_1()
                    .border_color(theme.colors.border_soft)
                    .pl(theme.metrics.space_xs)
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .debug_selector(|| "preview-screen-controls".into())
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .child(
                                div().flex_1().min_w_0().child(
                                    Select::new("preview-screen", options, selected_id)
                                        .compact()
                                        .disabled(entries.is_empty())
                                        .label(t.t("output.previewScreen"))
                                        .on_change(move |id, _, cx| {
                                            runtime.output.update(cx, |output, cx| {
                                                output.selected_screen = id;
                                                cx.notify();
                                            })
                                        }),
                                ),
                            )
                            .child(
                                orbit_ui::controls::IconButton::new(
                                    "preview-screen-settings",
                                    "settings",
                                    t.t("settings.nav.video"),
                                )
                                .on_click(move |_, _, cx| {
                                    crate::view::request_auxiliary(
                                        settings_host.clone(),
                                        serde_json::json!({"kind":"settings","panel":"video"}),
                                        settings_owner.clone(),
                                        cx,
                                    )
                                }),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(gpui::relative(0.28))
                            .min_w_0()
                            .border_l_1()
                            .border_color(theme.colors.border_soft)
                            .child(
                                preview_action(
                                    "preview-clear-logo",
                                    t.t("presentation.command.clearToLogo"),
                                    "polaroid",
                                    ready,
                                    false,
                                    cx,
                                )
                                .tooltip(tooltip_text(t.t("presentation.command.clearToLogo")))
                                .on_click(cx.listener(
                                    |this, _, _, cx| actions::clear(&this.runtime, "logo", cx),
                                )),
                            ),
                    )
                    .child(
                        div().flex_none().w(gpui::relative(0.28)).min_w_0().child(
                            preview_action(
                                "preview-freeze",
                                t.t("output.freeze"),
                                "player-pause",
                                ready,
                                frozen,
                                cx,
                            )
                            .tooltip(tooltip_text(t.t(if frozen {
                                "output.unfreeze"
                            } else {
                                "output.freeze"
                            })))
                            .on_click(cx.listener(|this, _, _, cx| {
                                super::scene::toggle_freeze(&this.runtime, cx);
                            })),
                        ),
                    ),
            )
            .when_some(menu, |el, menu| el.child(menu))
    }
}

fn preview_action(
    id: &'static str,
    label: String,
    icon: &'static str,
    enabled: bool,
    selected: bool,
    cx: &gpui::App,
) -> Button {
    let theme = cx.theme();
    Button::new(id)
        .debug_selector(move || id.into())
        .accessibility_label(label.clone())
        .disabled(!enabled)
        .selected(selected)
        .w_full()
        .min_w_0()
        .h(theme.metrics.control_height_compact)
        .flex()
        .items_center()
        .justify_center()
        .gap(theme.metrics.space_xs)
        .px(theme.metrics.space_xs)
        .bg(if selected {
            theme.colors.selected
        } else {
            theme.colors.control_bg
        })
        .text_color(theme.colors.text)
        .text_size(theme.metrics.font_sm)
        .hover(|style| {
            style.bg(if selected {
                theme.colors.selected
            } else {
                theme.colors.control_bg_hover
            })
        })
        .focus_visible(|style| style.border_2().border_color(theme.colors.focus_ring))
        .child(orbit_ui::icon(icon))
        .child(div().min_w_0().truncate().child(label))
}
