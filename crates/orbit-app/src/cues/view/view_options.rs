use super::*;
use orbit_host::cue_view_options::{CueViewOptions, FontStyle, Horizontal, Truncate, Vertical};
use orbit_ui::controls::{CheckboxField, ColorPicker, NumberInput};

#[derive(Default)]
pub(super) struct ViewOptions {
    pub(super) shown: CueViewOptions,
    open: bool,
    fonts: Vec<String>,
    size: Option<(Mode, u16, Entity<InputState>, Subscription)>,
}

impl CueView {
    pub(super) fn sync_view_options(&mut self, cx: &mut Context<Self>) {
        let settings = self.runtime.output.read(cx);
        let shown = settings.settings_drafts.shown(&settings.settings);
        let options = CueViewOptions::read(
            &shown["generalSettings"]["cueViewOptions"],
            self.view_mode(),
        );
        if self.view_options.shown != options {
            let regroup = self.view_options.shown.group_slides != options.group_slides;
            self.view_options.shown = options;
            if regroup {
                self.rebuild_rows(cx);
            }
        }
    }

    fn save_view_option(&mut self, mode: Mode, key: &str, value: Value, cx: &mut Context<Self>) {
        let settings = crate::settings::persist::shown(&self.runtime, cx);
        let mut options = settings["generalSettings"]["cueViewOptions"].clone();
        if options[mode.key()][key] == value {
            return;
        }
        options[mode.key()][key] = value;
        let payload = crate::settings::persist::general_payload(
            &settings,
            &json!({"cueViewOptions": options}),
        );
        let task = crate::settings::persist::save(&self.runtime, cx, payload);
        cx.spawn(async move |view, cx| {
            if let Err(error) = task.await {
                view.update(cx, |view, cx| actions::error(&view.runtime, cx, error))
                    .ok();
            }
        })
        .detach();
    }

    pub(super) fn view_options_button(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = cx.global::<Translator>().t("cue.view.options");
        let toggle = cx.weak_entity();
        let dismiss = toggle.clone();
        let mut popover = Popover::new(
            "cue-view-options",
            IconButton::new("cue-view-options-button", "dots", label)
                .selected(self.view_options.open)
                .on_click(move |_, window, cx| {
                    toggle
                        .update(cx, |view, cx| {
                            view.view_options.open = !view.view_options.open;
                            if view.view_options.open && view.view_options.fonts.is_empty() {
                                view.view_options.fonts = window.text_system().all_font_names();
                            }
                            cx.notify();
                        })
                        .ok();
                }),
        )
        .anchor(gpui::Anchor::BottomRight)
        .on_dismiss(move |_, _, cx| {
            dismiss
                .update(cx, |view, cx| {
                    view.view_options.open = false;
                    cx.notify();
                })
                .ok();
        });
        if self.view_options.open {
            popover = popover.content(self.view_options_panel(window, cx));
        }
        popover.into_any_element()
    }

    fn view_options_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let mode = self.mode;
        let options = self.view_options.shown.clone();
        let owner = cx.weak_entity();
        let update = move |key: &'static str| {
            let owner = owner.clone();
            move |value: Value, cx: &mut App| {
                owner
                    .update(cx, |view, cx| view.save_view_option(mode, key, value, cx))
                    .ok();
            }
        };
        let checkbox = |key, label, checked| {
            let save = update(key);
            let id = format!("cue-view-{key}");
            div()
                .debug_selector({
                    let id = id.clone();
                    move || id.clone()
                })
                .child(
                    CheckboxField::new(id, t.t(label), checked)
                        .on_change(move |value, _, cx| save(json!(value), cx)),
                )
        };
        let color =
            |key: &'static str, label: &'static str, value: Option<&String>, default, disabled| {
                let save = update(key);
                let label = t.t(label);
                let value = value.cloned().unwrap_or_else(|| color_hex(default));
                Field::new(
                    label.clone(),
                    ColorPicker::new(format!("cue-view-{key}"), label, value)
                        .disabled(disabled)
                        .on_change(move |value, _, cx| save(json!(value), cx)),
                )
                .inline()
            };
        let mut panel = div()
            .debug_selector(|| "cue-view-options-panel".into())
            .w(theme.metrics.control_height * 10.)
            .max_h(window.viewport_size().height * 0.8)
            .id("cue-view-options-scroll")
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(theme.metrics.space_sm)
            .p(theme.metrics.panel_padding)
            .rounded(theme.metrics.radius_md)
            .bg(theme.colors.popover_bg)
            .border_1()
            .border_color(theme.colors.border)
            .shadow(theme.shadow_soft())
            .child(checkbox(
                "groupSlides",
                "cue.view.groupSlides",
                options.group_slides,
            ))
            .when(mode == Mode::Easy, |panel| {
                panel.child(checkbox(
                    "showMediaAndColors",
                    "cue.view.mediaAndColors",
                    options.show_media_and_colors,
                ))
            })
            .when(mode != Mode::Easy, |panel| {
                panel.child(checkbox(
                    "useThumbnailColor",
                    "cue.view.useThumbnailColor",
                    options.use_thumbnail_color,
                ))
            })
            .child(color(
                "thumbnailColor",
                "cue.view.thumbnailColor",
                options.thumbnail_color.as_ref(),
                theme.colors.preview_matte_bg,
                mode != Mode::Easy && !options.use_thumbnail_color,
            ));
        if mode == Mode::Grid {
            return panel;
        }
        if mode == Mode::Table {
            panel = panel.child(color(
                "tableColor",
                "cue.view.tableColor",
                options.table_color.as_ref(),
                theme.colors.stage_bg,
                false,
            ));
        }
        let save_family = update("fontFamily");
        let fonts = std::iter::once(SelectOption::new(None, t.t("cue.view.systemFont")))
            .chain(
                self.view_options
                    .fonts
                    .iter()
                    .map(|font| SelectOption::new(Some(font.clone()), font.clone())),
            )
            .collect();
        let save_style = update("fontStyle");
        let save_truncate = update("truncate");
        let save_horizontal = update("horizontal");
        let save_vertical = update("vertical");
        panel
            .child(Field::new(
                t.t("cue.view.font"),
                Select::new("cue-view-font", fonts, Some(options.font_family))
                    .on_change(move |value, _, cx| save_family(json!(value), cx)),
            ))
            .child(
                div()
                    .flex()
                    .gap(theme.metrics.space_sm)
                    .items_end()
                    .child(
                        div().flex_1().min_w_0().child(Field::new(
                            t.t("cue.view.fontStyle"),
                            Select::new(
                                "cue-view-font-style",
                                [
                                    (FontStyle::Regular, "cue.view.regular"),
                                    (FontStyle::Bold, "cue.view.bold"),
                                    (FontStyle::Italic, "cue.view.italic"),
                                    (FontStyle::BoldItalic, "cue.view.boldItalic"),
                                ]
                                .into_iter()
                                .map(|(value, key)| SelectOption::new(value, t.t(key)))
                                .collect(),
                                options.font_style,
                            )
                            .on_change(move |value, _, cx| save_style(json!(value), cx)),
                        )),
                    )
                    .child(
                        div()
                            .w(theme.metrics.control_height * 3.)
                            .child(self.font_size_field(mode, window, cx)),
                    ),
            )
            .child(color(
                "textColor",
                "cue.view.textColor",
                options.text_color.as_ref(),
                theme.colors.text_strong,
                false,
            ))
            .child(Field::new(
                t.t("cue.view.alignment"),
                div()
                    .flex()
                    .flex_col()
                    .gap(theme.metrics.space_xs)
                    .child(
                        SegmentedControl::new(
                            "cue-view-horizontal",
                            t.t("cue.view.alignment"),
                            vec![
                                SegmentedOption::icon(
                                    Horizontal::Left,
                                    "align-left",
                                    t.t("cue.view.alignLeft"),
                                ),
                                SegmentedOption::icon(
                                    Horizontal::Center,
                                    "align-center",
                                    t.t("cue.view.alignCenter"),
                                ),
                                SegmentedOption::icon(
                                    Horizontal::Right,
                                    "align-right",
                                    t.t("cue.view.alignRight"),
                                ),
                                SegmentedOption::icon(
                                    Horizontal::Justify,
                                    "align-justified",
                                    t.t("cue.view.justify"),
                                ),
                            ],
                            options.horizontal,
                        )
                        .on_change(move |value, _, cx| save_horizontal(json!(value), cx)),
                    )
                    .child(
                        SegmentedControl::new(
                            "cue-view-vertical",
                            t.t("cue.view.verticalAlignment"),
                            vec![
                                SegmentedOption::icon(
                                    Vertical::Top,
                                    "layout-align-top",
                                    t.t("cue.view.alignTop"),
                                ),
                                SegmentedOption::icon(
                                    Vertical::Middle,
                                    "layout-align-middle",
                                    t.t("cue.view.alignMiddle"),
                                ),
                                SegmentedOption::icon(
                                    Vertical::Bottom,
                                    "layout-align-bottom",
                                    t.t("cue.view.alignBottom"),
                                ),
                            ],
                            options.vertical,
                        )
                        .on_change(move |value, _, cx| save_vertical(json!(value), cx)),
                    ),
            ))
            .child(
                Field::new(
                    t.t("cue.view.truncate"),
                    Select::new(
                        "cue-view-truncate",
                        [
                            (Truncate::Start, "cue.view.truncateStart"),
                            (Truncate::Middle, "cue.view.truncateMiddle"),
                            (Truncate::End, "cue.view.truncateEnd"),
                        ]
                        .into_iter()
                        .map(|(value, key)| SelectOption::new(value, t.t(key)))
                        .collect(),
                        options.truncate,
                    )
                    .on_change(move |value, _, cx| save_truncate(json!(value), cx)),
                )
                .inline(),
            )
    }

    fn font_size_field(
        &mut self,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let size = self
            .view_options
            .shown
            .font_size
            .unwrap_or(if mode == Mode::Table { 13 } else { 14 });
        if !self
            .view_options
            .size
            .as_ref()
            .is_some_and(|(m, n, _, _)| *m == mode && *n == size)
        {
            let state = input_state(size.to_string(), window, cx);
            let subscription =
                cx.subscribe_in(&state, window, move |view, state, event, window, cx| {
                    if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        let input = state.read(cx).value();
                        if let Ok(value) = input.trim().parse::<u16>() {
                            let value = value.clamp(8, 72);
                            state.update(cx, |state, cx| {
                                state.set_value(value.to_string(), window, cx)
                            });
                            if size != value {
                                view.save_view_option(mode, "fontSize", json!(value), cx);
                            }
                        } else {
                            state.update(cx, |state, cx| {
                                state.set_value(size.to_string(), window, cx)
                            });
                        }
                    }
                });
            self.view_options.size = Some((mode, size, state, subscription));
        }
        let state = &self
            .view_options
            .size
            .as_ref()
            .expect("Font size input is initialized")
            .2;
        let owner = cx.weak_entity();
        let label = cx.global::<Translator>().t("cue.view.fontSize");
        Field::new(
            label.clone(),
            NumberInput::new("cue-view-font-size", state)
                .label(label)
                .range(8., 72.)
                .stepper()
                .on_step(move |value, _, cx| {
                    owner
                        .update(cx, |view, cx| {
                            view.save_view_option(mode, "fontSize", json!(value as u16), cx)
                        })
                        .ok();
                }),
        )
    }
}

pub(super) fn color_hex(color: gpui::Rgba) -> String {
    let byte = |channel: f32| (channel * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(color.r),
        byte(color.g),
        byte(color.b)
    )
}
