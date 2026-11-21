use super::*;
use orbit_ui::controls::{
    AngleDial, Disclosure, RangeInput, SegmentedControl, SegmentedOption, SliderEvent,
};

fn take(fields: &mut Vec<Spec>, key: &str) -> Option<Spec> {
    fields
        .iter()
        .position(|field| field.key == key)
        .map(|index| fields.remove(index))
}

fn group(tab: &str, key: &str) -> usize {
    if tab == "shape" {
        if key.starts_with("stroke") {
            1
        } else if key.starts_with("shadow") || key == "shapeShadowEnabled" {
            2
        } else if key.starts_with("feathering") || key == "shapeFeatheringEnabled" {
            3
        } else if key == "shapeVisibilityEnabled" {
            4
        } else if matches!(key, "name" | "text" | "locked") {
            5
        } else {
            0
        }
    } else {
        if key.starts_with("stroke") {
            3
        } else if key.starts_with("shadow") {
            4
        } else if key.starts_with("linesOnly") {
            5
        } else if key.starts_with("scroll") {
            6
        } else if key.starts_with("textList") {
            7
        } else if key.starts_with("linkedText")
            || key.starts_with("systemClock")
            || key.starts_with("videoCountdown")
            || key == "preserveNotesFormat"
        {
            8
        } else if key.starts_with("textGradient")
            || matches!(
                key,
                "textFillMode" | "textColor" | "backgroundBlurAmount" | "backgroundBlurSaturation"
            )
            || key.starts_with("mediaFill")
        {
            1
        } else if matches!(
            key,
            "fontFamily"
                | "fontSize"
                | "fontWeight"
                | "bold"
                | "italic"
                | "underline"
                | "strikethrough"
                | "capitalization"
        ) {
            0
        } else if matches!(
            key,
            "scaling"
                | "lineTransform"
                | "lineTransformDelimiter"
                | "textHorizontalAlign"
                | "textVerticalAlign"
        ) {
            2
        } else {
            9
        }
    }
}

impl SlideCanvas {
    fn object_field(
        &mut self,
        field: Spec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = field.key.clone();
        let caption = match key.as_str() {
            "strokeStyle" => Some("theme.text.style"),
            "strokeWidth" => Some("theme.width"),
            "shadowBlur" => Some("theme.blur"),
            "shadowOpacity" => Some("theme.opacity"),
            "shadowAngle" => Some("theme.angle"),
            "shadowOffset" => Some("theme.offset"),
            "x" => Some("theme.gpui.x"),
            "y" => Some("theme.gpui.y"),
            "width" => Some("theme.width"),
            "height" => Some("theme.height"),
            "rotation" => Some("theme.rotation"),
            _ => None,
        }
        .map(|key| cx.global::<Translator>().t(key));
        let color = matches!(field.kind, Kind::Color);
        let control = self.inspector_field(field, window, cx);
        div()
            .min_w_0()
            .debug_selector(move || format!("object-field-{key}"))
            .when(color, |row| row.flex().justify_end())
            .child(
                div()
                    .when(color, |control| control.w(cx.theme().metrics.space_lg * 3.))
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(cx.theme().metrics.space_2xs)
                    .child(control)
                    .children(caption.map(|caption| {
                        div()
                            .text_center()
                            .min_w_0()
                            .truncate()
                            .text_size(cx.theme().metrics.font_xs)
                            .text_color(cx.theme().colors.text_muted)
                            .child(caption)
                    })),
            )
            .into_any_element()
    }

    fn text_alignment(&mut self, field: Spec, cx: &mut Context<Self>) -> AnyElement {
        let Kind::Choice(options) = &field.kind else {
            return div().into_any_element();
        };
        let t = *cx.global::<Translator>();
        let options = options
            .iter()
            .map(|(value, label)| {
                SegmentedOption::icon(
                    value.clone(),
                    match value.as_str() {
                        "center" => "align-center",
                        "right" => "align-right",
                        "justify" => "align-justified",
                        "top" => "layout-align-top",
                        "middle" => "layout-align-middle",
                        "bottom" => "layout-align-bottom",
                        _ => "align-left",
                    },
                    label
                        .strip_prefix("i18n:")
                        .map(|key| t.t(key))
                        .unwrap_or_else(|| label.clone()),
                )
            })
            .collect();
        let value = field.value.as_str().unwrap_or("").to_string();
        let weak = cx.weak_entity();
        SegmentedControl::new(
            SharedString::from(format!("object-{}", field.key)),
            t.t(&field.label),
            options,
            value,
        )
        .on_change(move |value, _, cx| {
            let _ = weak.update(cx, |view, cx| {
                view.patch_inspector(&field, json!(value), cx)
            });
        })
        .into_any_element()
    }
    fn inspector_range(
        &mut self,
        spec: Spec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(value) = spec
            .value
            .as_f64()
            .filter(|value| value.is_finite() && (0. ..=100.).contains(value))
        else {
            let t = *cx.global::<Translator>();
            return orbit_ui::controls::OutputField::new(
                t.t(&spec.label),
                t.t("theme.gpui.invalidValue"),
            )
            .into_any_element();
        };
        let value = value as f32;
        let state = if let Some(state) = self.editor.ranges.get(&spec.key) {
            state.clone()
        } else {
            let state = orbit_ui::controls::range_state(0., 100., 1., value, cx);
            let field = spec.clone();
            let scope = self.editor.scope.clone();
            self.editor.subscriptions.push(cx.subscribe(
                &state,
                move |view, state, event: &SliderEvent, cx| {
                    if view.editor.scope == scope && matches!(event, SliderEvent::Change(_)) {
                        view.patch_inspector(&field, json!(state.read(cx).value().end()), cx);
                    }
                },
            ));
            self.editor.ranges.insert(spec.key.clone(), state.clone());
            state
        };
        if (state.read(cx).value().end() - value).abs() > 0.01 {
            state.update(cx, |state, cx| state.set_value(value, window, cx));
        }
        let label = cx.global::<Translator>().t(&spec.label);
        let key = spec.key.clone();
        let slider = RangeInput::new(&state)
            .disabled(self.document_controls_disabled())
            .label(label.clone());
        if key != "opacity" {
            return div()
                .min_w_0()
                .debug_selector(move || format!("object-{key}-control"))
                .child(slider)
                .into_any_element();
        }
        div()
            .debug_selector(|| "object-opacity-control".into())
            .border_t_1()
            .border_color(cx.theme().colors.border)
            .pt(cx.theme().metrics.space_sm)
            .pb(cx.theme().metrics.space_xs)
            .child(
                Field::new(
                    label,
                    div()
                        .flex()
                        .items_center()
                        .gap(cx.theme().metrics.space_xs)
                        .child(div().flex_1().min_w_0().child(slider))
                        .child(
                            div()
                                .flex_none()
                                .w(cx.theme().metrics.space_lg * 2.)
                                .text_right()
                                .text_size(cx.theme().metrics.font_sm)
                                .child(format!("{value:.0}%")),
                        ),
                )
                .inline()
                .label_width(cx.theme().metrics.space_lg * 4.),
            )
            .into_any_element()
    }

    fn inspector_pair(
        &mut self,
        fields: &mut Vec<Spec>,
        keys: [&str; 2],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut row = div()
            .grid()
            .grid_cols(2)
            .gap(cx.theme().metrics.space_xs)
            .min_w_0();
        for key in keys {
            if let Some(field) = take(fields, key) {
                row = row.child(div().min_w_0().child(self.object_field(field, window, cx)));
            }
        }
        row.into_any_element()
    }

    fn object_alignment(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let mut row = div().flex().gap(cx.theme().metrics.space_2xs).flex_wrap();
        for (axis, icon) in [
            ("left", "align-left"),
            ("center", "align-center"),
            ("right", "align-right"),
            ("top", "layout-align-top"),
            ("middle", "layout-align-middle"),
            ("bottom", "layout-align-bottom"),
        ] {
            row = row.child(
                IconButton::new(
                    SharedString::from(format!("object-align-{axis}")),
                    icon,
                    t.t(&format!("theme.gpui.align.{axis}")),
                )
                .disabled(
                    self.saving
                        || self
                            .selected
                            .is_some_and(|index| self.slide["layers"][index]["locked"] == true),
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    let Some(index) = view.selected else {
                        return;
                    };
                    let mut bounds =
                        geometry::layer_bounds(&view.slide["layers"][index], view.view.slide);
                    match axis {
                        "left" => bounds.x = 0.,
                        "center" => bounds.x = (view.view.slide.x - bounds.width) / 2.,
                        "right" => bounds.x = view.view.slide.x - bounds.width,
                        "top" => bounds.y = 0.,
                        "middle" => bounds.y = (view.view.slide.y - bounds.height) / 2.,
                        _ => bounds.y = view.view.slide.y - bounds.height,
                    }
                    let mut slide = (*view.slide).clone();
                    slide["layers"][index] =
                        geometry::patch_bounds(&slide["layers"][index], view.view.slide, bounds);
                    view.editor.scope.clear();
                    view.draft(slide, cx);
                })),
            );
        }
        row.into_any_element()
    }

    fn object_transform(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(index) = self.selected else {
            return div().into_any_element();
        };
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let layer = &self.slide["layers"][index];
        let disabled = self.document_controls_disabled() || layer["locked"] == true;
        let angle = layer["rotation"].as_f64().unwrap_or(0.).rem_euclid(360.);
        let mut buttons = div().flex().gap(theme.metrics.space_2xs);
        for (key, icon, label) in [
            ("flipX", "flip-horizontal", "theme.align.flipHorizontal"),
            ("flipY", "flip-vertical", "theme.align.flipVertical"),
        ] {
            let selected = layer[key] == true;
            let spec = Spec::new(key, Target::Layer, Kind::Bool, json!(selected));
            buttons = buttons.child(
                IconButton::new(
                    SharedString::from(format!("object-{key}")),
                    icon,
                    t.t(label),
                )
                .selected(selected)
                .disabled(disabled)
                .on_click(cx.listener(move |view, _, _, cx| {
                    let value = view
                        .selected
                        .is_some_and(|i| view.slide["layers"][i][key] == true);
                    view.patch_inspector(&spec, json!(!value), cx);
                })),
            );
        }
        let rotation = Spec::new(
            "rotation",
            Target::Layer,
            Kind::Number(0., 360.),
            editing::numeric(angle),
        )
        .named("theme.rotation");
        let field = self.object_field(rotation.clone(), window, cx);
        let dial = AngleDial::new("object-rotation-dial", t.t("theme.rotation"), angle)
            .disabled(disabled)
            .on_change(cx.listener(move |view, angle: &f64, window, cx| {
                if view.selected.is_some_and(|i| {
                    view.slide["layers"][i]["rotation"].as_f64().unwrap_or(0.) == *angle
                }) {
                    return;
                }
                if let Some(input) = view.editor.inputs.get("rotation") {
                    input.update(cx, |input, cx| {
                        input.set_value(angle.to_string(), window, cx)
                    });
                }
                view.patch_inspector(&rotation, editing::numeric(*angle), cx);
            }));
        Field::new(
            t.t("theme.transform"),
            div()
                .flex()
                .min_w_0()
                .justify_end()
                .gap(theme.metrics.space_xs)
                .child(buttons)
                .child(dial)
                .child(div().w(theme.metrics.space_lg * 5.).min_w_0().child(field)),
        )
        .inline()
        .label_width(theme.metrics.space_lg * 6.)
        .into_any_element()
    }

    pub(super) fn object_inspector(
        &mut self,
        mut specs: Vec<Spec>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let tab = self.editor.tab.clone();
        let mut body = div()
            .id("object-inspector-fields")
            .debug_selector(|| "object-inspector-fields".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(theme.metrics.space_xs)
            .p(theme.metrics.space_sm);
        if tab == "shape" {
            body = body.child(
                div()
                    .border_b_1()
                    .border_color(theme.colors.border)
                    .pb(theme.metrics.space_sm)
                    .mb(theme.metrics.space_xs)
                    .child(self.object_alignment(cx)),
            );
            for (label, keys) in [
                ("theme.position", ["x", "y"]),
                ("theme.size", ["width", "height"]),
            ] {
                body = body.child(
                    Field::new(
                        t.t(label),
                        div().flex().justify_end().child(
                            div()
                                .w(theme.metrics.space_lg * 10. + theme.metrics.space_xs)
                                .min_w_0()
                                .child(self.inspector_pair(&mut specs, keys, window, cx)),
                        ),
                    )
                    .inline()
                    .label_width(theme.metrics.space_lg * 6.),
                );
            }
            body = body.child(self.object_transform(window, cx));
            if let Some(opacity) = take(&mut specs, "opacity") {
                body = body.child(self.inspector_range(opacity, window, cx));
            }
        }
        if tab == "text" {
            let mut styles = div().flex().gap(theme.metrics.space_xs);
            for (action, key) in [
                ("style-copy", "theme.inspector.copyStyle"),
                ("style-paste", "theme.inspector.pasteStyle"),
            ] {
                styles = styles.child(
                    ActionButton::new(action, t.t(key))
                        .compact()
                        .disabled(
                            self.saving
                                || action == "style-paste" && self.editor.style_clipboard.is_none(),
                        )
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.editor_action(action, window, cx)
                        })),
                );
            }
            body = body.child(styles);
        }
        if tab == "build" {
            for (prefix, label) in [
                ("build", "build.enabled"),
                ("buildOut", "build.out.enabled"),
            ] {
                let enabled = take(&mut specs, &format!("{prefix}Enabled"));
                let active = enabled.as_ref().is_some_and(|field| field.value == true);
                let mut section = div()
                    .flex()
                    .flex_col()
                    .gap(theme.metrics.space_xs)
                    .py(theme.metrics.space_xs)
                    .border_b_1()
                    .border_color(theme.colors.border);
                if active {
                    if let Some(field) = enabled {
                        section = section.child(self.object_field(field, window, cx));
                    }
                } else if let Some(field) = enabled {
                    section = section.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(t.t(label))
                            .child(
                                ActionButton::new(
                                    SharedString::from(format!("{prefix}-add")),
                                    t.t("theme.inspector.addBuild"),
                                )
                                .compact()
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.patch_inspector(&field, json!(true), cx)
                                    },
                                )),
                            ),
                    );
                }
                let mut rest = Vec::new();
                for field in specs {
                    if field.key.starts_with(prefix)
                        && (prefix == "buildOut" || !field.key.starts_with("buildOut"))
                    {
                        section = section.child(self.object_field(field, window, cx));
                    } else {
                        rest.push(field);
                    }
                }
                specs = rest;
                body = body.child(section);
            }
            for field in specs {
                body = body.child(self.object_field(field, window, cx));
            }
            body = body
                .child(self.build_order(cx))
                .child(self.editor_collections("build", window, cx));
            return body.into_any_element();
        }
        let sections: &[(&str, Option<&str>)] = if tab == "shape" {
            &[
                ("theme.fill", Some("fillEnabled")),
                ("theme.stroke", Some("strokeEnabled")),
                ("theme.shadow", Some("shapeShadowEnabled")),
                ("theme.feathering", Some("shapeFeatheringEnabled")),
                ("theme.visibility", Some("shapeVisibilityEnabled")),
                ("theme.object", None),
            ]
        } else {
            &[
                ("theme.text.font", None),
                ("theme.color", None),
                ("theme.text.scaling", None),
                ("theme.stroke", Some("strokeEnabled")),
                ("theme.shadow", Some("shadowEnabled")),
                ("theme.text.linesOnly", Some("linesOnlyEnabled")),
                ("theme.text.scrolling", Some("scrollingEnabled")),
                ("theme.text.listStyle", None),
                ("theme.text.linkedText", Some("linkedTextEnabled")),
                ("theme.text.paragraphScope", None),
            ]
        };
        for (number, (label, toggle)) in sections.iter().enumerate() {
            let mut fields: Vec<_> = specs
                .iter()
                .filter(|field| group(&tab, &field.key) == number)
                .cloned()
                .collect();
            if fields.is_empty() {
                continue;
            }
            let mut section = div()
                .flex()
                .flex_col()
                .gap(theme.metrics.space_xs)
                .py(theme.metrics.space_xs)
                .border_t_1()
                .border_color(theme.colors.border);
            let enabled = if tab == "text" && number == 7 {
                let field = fields
                    .iter()
                    .find(|field| field.key == "textListStyle")
                    .cloned();
                if let Some(field) = field {
                    let enabled = field
                        .value
                        .as_str()
                        .is_some_and(|style| style != "none" && !style.is_empty());
                    let weak = cx.weak_entity();
                    section = section.child(
                        CheckboxField::new("object-list-enabled", t.t(label), enabled).on_change(
                            move |enabled, _, cx| {
                                let _ = weak.update(cx, |view, cx| {
                                    view.patch_inspector(
                                        &field,
                                        json!(if enabled { "disc" } else { "none" }),
                                        cx,
                                    )
                                });
                            },
                        ),
                    );
                    enabled
                } else {
                    false
                }
            } else if let Some(key) = toggle {
                if let Some(mut field) = take(&mut fields, key) {
                    let enabled = field.value == true;
                    field.label = (*label).into();
                    let toggle = self.object_field(field, window, cx);
                    if tab == "shape" && number == 0 && enabled {
                        let mut header = div()
                            .flex()
                            .items_center()
                            .gap(theme.metrics.space_sm)
                            .child(div().flex_none().child(toggle));
                        if let Some(fill_type) = take(&mut fields, "fillType") {
                            header = header.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.object_field(fill_type, window, cx)),
                            );
                        }
                        section = section.child(header);
                    } else if tab == "shape" && number == 3 && enabled {
                        let mut header = div()
                            .flex()
                            .items_center()
                            .gap(theme.metrics.space_sm)
                            .child(div().flex_none().child(toggle));
                        if let Some(mut amount) = take(&mut fields, "featheringAmount") {
                            amount.label = "theme.feathering".into();
                            header = header.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.inspector_range(amount, window, cx)),
                            );
                        }
                        let _ = take(&mut fields, "featheringStyle");
                        section = section.child(header);
                    } else {
                        section = section.child(toggle);
                    }
                    enabled
                } else {
                    false
                }
            } else {
                true
            };
            if enabled {
                if tab == "text" && number == 0 {
                    if let Some(font) = take(&mut fields, "fontFamily") {
                        section = section.child(self.object_field(font, window, cx));
                    }
                    section = section.child(self.inspector_pair(
                        &mut fields,
                        ["fontWeight", "fontSize"],
                        window,
                        cx,
                    ));
                    let mut formats = div().flex().gap(theme.metrics.space_2xs);
                    for key in ["bold", "italic", "underline", "strikethrough"] {
                        if let Some(field) = take(&mut fields, key) {
                            let active = field.value == true;
                            formats = formats.child(
                                IconButton::new(
                                    SharedString::from(format!("object-format-{key}")),
                                    key,
                                    t.t(&field.label),
                                )
                                .selected(active)
                                .disabled(self.saving)
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.patch_inspector(&field, json!(!active), cx)
                                    },
                                )),
                            );
                        }
                    }
                    section = section.child(formats);
                }
                if tab == "text" && number == 1 {
                    if let Some(mode) = take(&mut fields, "textFillMode") {
                        let color = take(&mut fields, "textColor")
                            .filter(|field| self.show_object_field(field));
                        section = section.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(theme.metrics.space_sm)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .max_w(theme.metrics.control_height * 6.)
                                        .child(self.object_field(mode, window, cx)),
                                )
                                .children(color.map(|field| {
                                    div()
                                        .flex_none()
                                        .child(self.object_field(field, window, cx))
                                })),
                        );
                    }
                }
                let compact_rows: &[&[&str]] =
                    if tab == "shape" && number == 1 || tab == "text" && number == 3 {
                        &[&["strokeStyle", "strokeWidth", "strokeColor"]]
                    } else if tab == "shape" && number == 2 || tab == "text" && number == 4 {
                        &[
                            &["shadowBlur", "shadowOpacity", "shadowColor"],
                            &["shadowAngle", "shadowOffset"],
                        ]
                    } else {
                        &[]
                    };
                for keys in compact_rows {
                    let mut row = div()
                        .flex()
                        .items_start()
                        .justify_end()
                        .gap(theme.metrics.space_sm);
                    for key in *keys {
                        if let Some(field) = take(&mut fields, key) {
                            let color = matches!(field.kind, Kind::Color);
                            row = row.child(
                                div()
                                    .min_w_0()
                                    .when(color, |field| field.flex_none())
                                    .when(!color, |field| {
                                        field.flex_none().w(theme.metrics.space_lg * 4.)
                                    })
                                    .child(self.object_field(field, window, cx)),
                            );
                        }
                    }
                    section = section.child(row);
                }
                for field in fields {
                    if self.show_object_field(&field) {
                        section = section.child(
                            if matches!(
                                field.key.as_str(),
                                "textHorizontalAlign" | "textVerticalAlign"
                            ) {
                                self.text_alignment(field, cx)
                            } else {
                                self.object_field(field, window, cx)
                            },
                        );
                    }
                }
                if tab == "shape" && number == 4 {
                    if let Some(index) = self.selected {
                        section = section.child(self.visibility_controls(index, window, cx));
                    }
                }
                if tab == "shape" && number == 0 || tab == "text" && number == 1 {
                    section = section.child(self.editor_collections("media", window, cx));
                }
                if tab == "text" && number == 8 {
                    section = section.child(self.editor_collections("linked", window, cx));
                }
            }
            if tab == "text" && number == 9 || tab == "shape" && number == 5 {
                let extra = self.editor_collections(
                    if tab == "text" { "paragraph" } else { "path" },
                    window,
                    cx,
                );
                let key = format!("inspector-{tab}-advanced");
                let open = self
                    .editor
                    .open_sections
                    .get(&key)
                    .copied()
                    .unwrap_or(false);
                let weak = cx.weak_entity();
                body = body.child(
                    Disclosure::new(
                        SharedString::from(key.clone()),
                        t.t("theme.inspector.advanced"),
                        open,
                    )
                    .child(section)
                    .child(extra)
                    .on_toggle(move |open, _, cx| {
                        let _ = weak.update(cx, |view, cx| {
                            view.editor.open_sections.insert(key.clone(), open);
                            cx.notify();
                        });
                    }),
                );
            } else {
                body = body.child(section);
            }
        }
        body.into_any_element()
    }

    fn show_object_field(&self, field: &Spec) -> bool {
        let Some(index) = self.selected else {
            return true;
        };
        let layer = &self.slide["layers"][index];
        let style = editing::style(&self.slide, Some(layer));
        match field.key.as_str() {
            "fillColor" => layer["fillType"]
                .as_str()
                .is_none_or(|kind| kind == "color"),
            "fillStartColor" | "fillEndColor" | "fillAngle" => layer["fillType"] == "gradient",
            "backgroundEffect" => layer["fillType"] == "background-effect",
            "backgroundBlurAmount" | "backgroundBlurSaturation" => {
                if self.editor.tab == "text" {
                    style["textFillMode"] == "Background Blur"
                } else {
                    layer["fillType"] == "background-effect"
                        && layer["backgroundEffect"] != "invert"
                }
            }
            key if key.starts_with("capture") => layer["fillType"] == "capture-status-color",
            "textColor" => style["textFillMode"]
                .as_str()
                .is_none_or(|kind| kind == "Text Color"),
            key if key.starts_with("textGradient") => style["textFillMode"] == "Text Gradient",
            "webUrl" => layer["fillType"] == "web" || layer["type"] == "Web",
            "previewScreenId" => layer["fillType"] == "screen-preview",
            "fillObjectId" => layer["fillType"] == "slide-object",
            "dataFillSource" => matches!(
                layer["fillType"].as_str(),
                Some("slide-object" | "group-color" | "label-color" | "chord-chart")
            ),
            key if key.starts_with("media") && !key.starts_with("mediaFill") => {
                layer["fillType"] == "media"
                    && (!matches!(
                        key,
                        "mediaCropTop" | "mediaCropRight" | "mediaCropBottom" | "mediaCropLeft"
                    ) || layer["mediaCrop"] == true)
            }
            _ => true,
        }
    }

    fn build_order(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let mut entries = Vec::new();
        for (index, layer) in self.slide["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            for (key, label) in [
                ("build", "build.enabled"),
                ("buildOut", "build.out.enabled"),
            ] {
                if let Some(step) = layer[key]["step"].as_u64() {
                    entries.push((
                        step,
                        index,
                        label,
                        layer["name"].as_str().unwrap_or("").to_string(),
                    ));
                }
            }
        }
        entries.sort_by_key(|(step, index, _, _)| (*step, *index));
        let mut body = div()
            .flex()
            .flex_col()
            .min_h(cx.theme().metrics.row_height * 4.)
            .child(PanelHeader::new(t.t("theme.inspector.buildOrder")));
        for (step, index, label, name) in entries {
            body = body.child(
                list_row(
                    SharedString::from(format!("build-order-{index}-{label}")),
                    self.selected == Some(index),
                    cx,
                )
                .child(format!("{step}. {name} · {}", t.t(label)))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.selected = Some(index);
                    view.editor.scope.clear();
                    cx.notify();
                })),
            );
        }
        body.into_any_element()
    }
}
