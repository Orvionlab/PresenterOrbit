use super::{
    geometry::ZOOM_STEPS,
    guides::{Grid, Guides},
    view::SlideCanvas,
};
use gpui::{
    div, prelude::*, AnyElement, App, Context, Entity, Pixels, Point, Rgba, Subscription, Window,
};
use orbit_ui::{
    controls::{
        input_state, CheckboxField, ColorPicker, ContextMenu, Field, IconButton, InputEvent,
        InputState, MenuEntry, MenuItem, NumberInput, Popover,
    },
    ActiveTheme, Translator,
};

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Guides,
    Grid,
    Zoom(Point<Pixels>),
}

struct Input {
    state: Entity<InputState>,
    _subscription: Subscription,
}

pub(super) struct CanvasTools {
    pub checkerboard: bool,
    pub grid: Grid,
    pub guides: Guides,
    panel: Option<Panel>,
    grid_inputs: Option<[Input; 2]>,
    zoom_input: Option<Input>,
}

impl Default for CanvasTools {
    fn default() -> Self {
        Self {
            checkerboard: true,
            grid: Grid::default(),
            guides: Guides::default(),
            panel: None,
            grid_inputs: None,
            zoom_input: None,
        }
    }
}

impl CanvasTools {
    pub fn editing_zoom(&self) -> bool {
        self.zoom_input.is_some()
    }

    pub fn close_controls(&mut self) {
        self.panel = None;
        self.grid_inputs = None;
        self.zoom_input = None;
    }
}

fn color_value(custom: &Option<String>, default: Rgba) -> String {
    custom.clone().unwrap_or_else(|| {
        let [r, g, b] = [default.r, default.g, default.b].map(|c| (c * 255.).round() as u8);
        format!("#{r:02x}{g:02x}{b:02x}")
    })
}

fn percent(zoom: f32) -> String {
    let value = format!("{:.1}", zoom * 100.);
    value.strip_suffix(".0").unwrap_or(&value).into()
}

impl SlideCanvas {
    fn set_canvas_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        self.tools.panel = (self.tools.panel != Some(panel)).then_some(panel);
        if self.tools.panel != Some(Panel::Grid) {
            self.tools.grid_inputs = None;
        }
        cx.notify();
    }

    fn dismiss_canvas_panel(&mut self, cx: &mut Context<Self>) {
        self.tools.panel = None;
        self.tools.grid_inputs = None;
        cx.notify();
    }

    pub(super) fn canvas_toolbar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let owner = cx.weak_entity();
        let guides_button = IconButton::new(
            "canvas-guide-options",
            "arrows-horizontal",
            t.t("canvas.guides"),
        )
        .selected(
            self.tools.panel == Some(Panel::Guides)
                || self.tools.guides.slide
                || self.tools.guides.theme,
        )
        .on_click(cx.listener(|view, _, _, cx| view.set_canvas_panel(Panel::Guides, cx)));
        let mut guides = Popover::new("canvas-guides-popover", guides_button)
            .anchor(gpui::Anchor::BottomRight)
            .on_dismiss(move |_, _, cx| {
                owner
                    .update(cx, |view, cx| view.dismiss_canvas_panel(cx))
                    .ok();
            });
        if self.tools.panel == Some(Panel::Guides) {
            guides = guides.content(self.guide_fields(cx));
        }
        let owner = cx.weak_entity();
        let grid_button = IconButton::new("canvas-grid-options", "layout-grid", t.t("canvas.grid"))
            .selected(self.tools.panel == Some(Panel::Grid) || self.tools.grid.enabled)
            .on_click(cx.listener(|view, _, _, cx| view.set_canvas_panel(Panel::Grid, cx)));
        let mut grid = Popover::new("canvas-grid-popover", grid_button)
            .anchor(gpui::Anchor::BottomRight)
            .on_dismiss(move |_, _, cx| {
                owner
                    .update(cx, |view, cx| view.dismiss_canvas_panel(cx))
                    .ok();
            });
        if self.tools.panel == Some(Panel::Grid) {
            grid = grid.content(self.grid_fields(window, cx));
        }
        let mut toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.metrics.space_2xs)
            .child(
                IconButton::new(
                    "canvas-checkerboard",
                    "checkerboard",
                    t.t("canvas.checkerboard"),
                )
                .selected(self.tools.checkerboard)
                .on_click(cx.listener(|view, _, _, cx| {
                    view.tools.checkerboard = !view.tools.checkerboard;
                    cx.notify();
                })),
            )
            .child(guides)
            .child(grid)
            .child(
                IconButton::new("canvas-guides", "ruler-2", t.t("theme.rulers.toggle"))
                    .selected(!self.editor.guides_hidden)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.editor.guides_hidden = !view.editor.guides_hidden;
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new("canvas-zoom", "search", t.t("canvas.zoom"))
                    .disabled(self.saving)
                    .on_click(cx.listener(|view, _, window, cx| view.toggle_zoom_menu(window, cx))),
            );
        let value = div()
            .id("canvas-zoom-value")
            .w(theme.metrics.font_sm * 5.)
            .flex_none();
        let fit = self.view.fit;
        toolbar = toolbar.child(if let Some(input) = &self.tools.zoom_input {
            value
                .debug_selector(|| "canvas-inline-zoom".into())
                .child(
                    NumberInput::new("canvas-inline-zoom-value", &input.state)
                        .label(t.t("canvas.customZoom"))
                        .suffix("%"),
                )
                .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        view.tools.zoom_input = None;
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
        } else {
            value
                .debug_selector(move || {
                    if fit {
                        "canvas-zoom-fit".into()
                    } else {
                        "canvas-zoom-percent".into()
                    }
                })
                .text_color(if fit {
                    theme.colors.accent
                } else {
                    theme.colors.text
                })
                .when(!self.saving, |value| {
                    value.cursor_pointer().on_click(
                        cx.listener(|view, _, window, cx| view.toggle_zoom_menu(window, cx)),
                    )
                })
                .child(format!("{}%", percent(self.view.zoom)))
        });
        toolbar.into_any_element()
    }

    fn toggle_zoom_menu(&mut self, window: &Window, cx: &mut Context<Self>) {
        if matches!(self.tools.panel, Some(Panel::Zoom(_))) {
            self.dismiss_canvas_panel(cx);
        } else {
            self.set_canvas_panel(Panel::Zoom(window.mouse_position()), cx);
        }
    }

    pub(super) fn canvas_zoom_menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let Some(Panel::Zoom(position)) = self.tools.panel else {
            return None;
        };
        let t = *cx.global::<Translator>();
        let mut entries: Vec<MenuEntry> = ZOOM_STEPS
            .into_iter()
            .map(|zoom| {
                let owner = cx.weak_entity();
                MenuItem::new(format!("{}%", percent(zoom)))
                    .checked(!self.view.fit && self.view.zoom == zoom)
                    .on_click(move |_, _, cx| {
                        owner.update(cx, |view, cx| view.zoom(zoom, cx)).ok();
                    })
                    .into()
            })
            .collect();
        let owner = cx.weak_entity();
        entries.push(
            MenuItem::new(t.t("canvas.customZoomMenu"))
                .on_click(move |_, window, cx| {
                    owner
                        .update(cx, |view, cx| view.edit_canvas_zoom(window, cx))
                        .ok();
                })
                .into(),
        );
        entries.push(MenuEntry::Separator);
        let owner = cx.weak_entity();
        entries.push(
            MenuItem::new(t.t("theme.zoom.fit"))
                .checked(self.view.fit)
                .on_click(move |_, _, cx| {
                    owner
                        .update(cx, |view, cx| {
                            view.view.fit_zoom();
                            view.request_raster(false, cx);
                            cx.notify();
                        })
                        .ok();
                })
                .into(),
        );
        let owner = cx.weak_entity();
        Some(
            ContextMenu::new("canvas-zoom-menu", position, entries, move |_, _, cx| {
                owner
                    .update(cx, |view, cx| view.dismiss_canvas_panel(cx))
                    .ok();
            })
            .width(cx.theme().metrics.control_height * 4.)
            .into_any_element(),
        )
    }

    fn edit_canvas_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = input_state(percent(self.view.zoom), window, cx);
        let subscription =
            cx.subscribe_in(&state, window, |view, state, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                    || !view.tools.editing_zoom()
                {
                    return;
                }
                view.tools.zoom_input = None;
                let value = state
                    .read(cx)
                    .value()
                    .trim()
                    .replace(',', ".")
                    .parse::<f32>();
                match value {
                    Ok(value) if (10. ..=200.).contains(&value) => view.zoom(value / 100., cx),
                    _ => view.status = cx.global::<Translator>().t("canvas.zoomRange"),
                }
                cx.notify();
            });
        state.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.tools.zoom_input = Some(Input {
            state,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn guide_fields(&self, cx: &Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let mut panel =
            options_panel("canvas-guide-fields", cx).w(theme.metrics.control_height * 10.);
        type GuideColor = fn(&mut Guides) -> &mut Option<String>;
        let colors: [(&str, &Option<String>, Rgba, GuideColor); 3] = [
            (
                "canvas.slideGuides",
                &self.tools.guides.slide_color,
                theme.colors.slide_guide,
                |g| &mut g.slide_color,
            ),
            (
                "canvas.themeGuides",
                &self.tools.guides.theme_color,
                theme.colors.object_guide,
                |g| &mut g.theme_color,
            ),
            (
                "canvas.alignmentGuides",
                &self.tools.guides.alignment_color,
                theme.colors.object_guide,
                |g| &mut g.alignment_color,
            ),
        ];
        let guide_color =
            |(key, value, default, set): (&str, &Option<String>, Rgba, GuideColor)| {
                let label = t.t(key);
                let owner = cx.weak_entity();
                Field::new(
                    label.clone(),
                    div()
                        .w(theme.metrics.control_height_compact * 2.)
                        .debug_selector({
                            let selector = format!("canvas-swatch-{key}");
                            move || selector.clone()
                        })
                        .child(
                            ColorPicker::new(key.to_string(), label, color_value(value, default))
                                .anchor(gpui::Anchor::BottomLeft)
                                .close_on_selection()
                                .on_change(move |color, _, cx| {
                                    owner
                                        .update(cx, |view, cx| {
                                            *set(&mut view.tools.guides) = Some(color);
                                            cx.notify();
                                        })
                                        .ok();
                                }),
                        ),
                )
                .inline()
                .label_width(theme.metrics.control_height * 5.)
            };
        type GuideToggle = fn(&mut Guides) -> &mut bool;
        let checks: [(&str, bool, GuideToggle); 6] = [
            ("canvas.showSlideGuides", self.tools.guides.slide, |g| {
                &mut g.slide
            }),
            ("canvas.showThemeGuides", self.tools.guides.theme, |g| {
                &mut g.theme
            }),
            ("canvas.relativeSpacing", self.tools.guides.spacing, |g| {
                &mut g.spacing
            }),
            ("canvas.relativeSizing", self.tools.guides.sizing, |g| {
                &mut g.sizing
            }),
            ("canvas.centerGuides", self.tools.guides.centers, |g| {
                &mut g.centers
            }),
            ("canvas.edgeGuides", self.tools.guides.edges, |g| {
                &mut g.edges
            }),
        ];
        for (index, (key, enabled, set)) in checks.into_iter().enumerate() {
            if let Some(color) = colors.get(index) {
                if index > 0 {
                    panel = panel.child(
                        div()
                            .h(theme.metrics.space_2xs / 2.)
                            .bg(theme.colors.border),
                    );
                }
                panel = panel.child(guide_color(*color));
            }
            let owner = cx.weak_entity();
            panel = panel.child(
                div()
                    .flex()
                    .debug_selector(move || format!("checkbox-{key}"))
                    .child(CheckboxField::new(key, t.t(key), enabled).on_change(
                        move |value, _, cx| {
                            owner
                                .update(cx, |view, cx| {
                                    *set(&mut view.tools.guides) = value;
                                    cx.notify();
                                })
                                .ok();
                        },
                    )),
            );
        }
        panel.into_any_element()
    }

    fn grid_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let labels = [
            ("canvas.gridSpacing", 1, 8192),
            ("canvas.gridSubdivisions", 1, 20),
        ];
        let values = [self.tools.grid.spacing, self.tools.grid.subdivisions];
        let inputs = self.tools.grid_inputs.get_or_insert_with(|| {
            std::array::from_fn(|index| {
                let (_, min, max) = labels[index];
                let state = input_state(values[index].to_string(), window, cx);
                let subscription = cx.subscribe_in(
                    &state,
                    window,
                    move |view, input, event: &InputEvent, window, cx| {
                        let value = input
                            .read(cx)
                            .value()
                            .trim()
                            .parse::<u32>()
                            .ok()
                            .filter(|v| (min..=max).contains(v));
                        let target = if index == 0 {
                            &mut view.tools.grid.spacing
                        } else {
                            &mut view.tools.grid.subdivisions
                        };
                        match event {
                            InputEvent::Change => {
                                if let Some(value) = value {
                                    *target = value;
                                    cx.notify();
                                }
                            }
                            InputEvent::Blur | InputEvent::PressEnter { .. } if value.is_none() => {
                                let valid = target.to_string();
                                input.update(cx, |state, cx| state.set_value(valid, window, cx));
                            }
                            _ => {}
                        }
                    },
                );
                Input {
                    state,
                    _subscription: subscription,
                }
            })
        });
        let owner = cx.weak_entity();
        let color_owner = owner.clone();
        let label_width = theme.metrics.control_height * 3.5;
        let mut panel = options_panel("canvas-grid-fields", cx)
            .w(theme.metrics.control_height * 7.5)
            .child(
                div()
                    .debug_selector(|| "canvas-grid-enable-row".into())
                    .child(
                        Field::new(
                            t.t("canvas.grid"),
                            CheckboxField::new(
                                "canvas-grid-enabled",
                                t.t("canvas.enableGrid"),
                                self.tools.grid.enabled,
                            )
                            .on_change(move |value, _, cx| {
                                owner
                                    .update(cx, |view, cx| {
                                        view.tools.grid.enabled = value;
                                        cx.notify();
                                    })
                                    .ok();
                            }),
                        )
                        .inline()
                        .label_width(label_width),
                    ),
            )
            .child(
                Field::new(
                    t.t("theme.color"),
                    div().w(theme.metrics.control_height_compact * 2.).child(
                        ColorPicker::new(
                            "canvas-grid-color",
                            t.t("theme.color"),
                            color_value(&self.tools.grid.color, theme.colors.text_muted),
                        )
                        .anchor(gpui::Anchor::BottomLeft)
                        .close_on_selection()
                        .on_change(move |color, _, cx| {
                            color_owner
                                .update(cx, |view, cx| {
                                    view.tools.grid.color = Some(color);
                                    cx.notify();
                                })
                                .ok();
                        }),
                    ),
                )
                .inline()
                .label_width(label_width),
            );
        for (input, (key, min, max)) in inputs.iter().zip(labels) {
            let label = t.t(key);
            panel = panel.child(
                Field::new(
                    label.clone(),
                    NumberInput::new(key, &input.state)
                        .label(label)
                        .range(f64::from(min), f64::from(max))
                        .stepper(),
                )
                .inline()
                .label_width(label_width),
            );
        }
        panel.into_any_element()
    }
}

fn options_panel(id: &'static str, cx: &App) -> gpui::Stateful<gpui::Div> {
    let theme = cx.theme();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .flex_col()
        .gap(theme.metrics.space_xs)
        .p(theme.metrics.space_sm)
        .border_1()
        .border_color(theme.colors.border)
        .rounded(theme.metrics.radius_sm)
        .bg(theme.colors.popover_bg)
        .text_color(theme.colors.text)
        .text_size(theme.metrics.font_sm)
}
