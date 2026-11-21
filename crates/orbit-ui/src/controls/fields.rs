use gpui::{
    div, prelude::*, relative, AnyElement, App, ElementId, Entity, Focusable, Pixels, SharedString,
    Window,
};
use gpui_base::input::{Input, InputState, TextareaState};
use gpui_base::slider::SliderState;
use gpui_base::{InputBase, Slider, SliderIndicator, SliderThumb, SliderTrack};

use super::focus_ring;
use crate::{ActiveTheme, Translator};

pub fn filter_state(
    value: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let placeholder = cx.global::<Translator>().t("common.filterPlaceholder");
    let value = value.into();
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value)
            .placeholder(placeholder)
    })
}

#[derive(IntoElement)]
pub struct FilterInput {
    id: ElementId,
    state: Entity<InputState>,
    label: Option<SharedString>,
    disabled: bool,
}

impl FilterInput {
    pub fn new(id: impl Into<ElementId>, state: &Entity<InputState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            label: None,
            disabled: false,
        }
    }
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for FilterInput {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let label = self
            .label
            .unwrap_or_else(|| cx.global::<Translator>().t("common.filterItems").into());
        div()
            .flex()
            .items_center()
            .gap(theme.metrics.space_xs)
            .w_full()
            .min_w_0()
            .child(crate::icon("filter").text_color(if self.disabled {
                theme.colors.disabled_text
            } else {
                theme.colors.text_muted
            }))
            .child(
                div().flex_1().min_w_0().child(
                    TextInput::new(self.id, &self.state)
                        .compact()
                        .label(label)
                        .disabled(self.disabled),
                ),
            )
    }
}

/// The editing state of a [`TextInput`]. Keep it in the owning view and subscribe to `InputEvent`.
pub fn input_state(
    value: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let value = value.into();
    cx.new(|cx| InputState::new(window, cx).default_value(value))
}

/// The editing state of a masked [`TextInput`]. `InputState::set_masked` shows or hides the text later.
pub fn password_state(
    value: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let value = value.into();
    cx.new(|cx| {
        InputState::new(window, cx)
            .masked(true)
            .default_value(value)
    })
}

/// The editing state of a [`Textarea::grow`] textarea, which is as tall as its wrapped text within the row limits.
pub fn growing_textarea_state(
    value: impl Into<SharedString>,
    min_rows: usize,
    max_rows: usize,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TextareaState> {
    let value = value.into();
    cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(min_rows, max_rows)
            .default_value(value)
    })
}

/// The editing state of a [`Textarea`].
pub fn textarea_state(
    value: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TextareaState> {
    let value = value.into();
    cx.new(|cx| TextareaState::new(window, cx).default_value(value))
}

/// The state of a [`RangeInput`]. Subscribe to `SliderEvent`.
pub fn range_state(min: f32, max: f32, step: f32, value: f32, cx: &mut App) -> Entity<SliderState> {
    cx.new(|_| {
        SliderState::new()
            .min(min)
            .max(max)
            .step(step)
            .default_value(value)
    })
}

/// A label above its control. `inline` puts it to the left, as inspectors do.
#[derive(IntoElement)]
pub struct Field {
    label: SharedString,
    control: AnyElement,
    inline: bool,
    label_width: Option<gpui::Pixels>,
}

impl Field {
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Self {
            label: label.into(),
            control: control.into_any_element(),
            inline: false,
            label_width: None,
        }
    }

    /// Align labels across a group of inline inspector rows.
    pub fn label_width(mut self, width: impl Into<gpui::Pixels>) -> Self {
        self.label_width = Some(width.into());
        self
    }

    pub fn inline(mut self) -> Self {
        self.inline = true;
        self
    }
}

impl RenderOnce for Field {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .min_w_0()
            .text_size(theme.metrics.font_md)
            .text_color(theme.colors.text)
            .map(|this| {
                if self.inline {
                    this.items_center().gap(theme.metrics.space_sm)
                } else {
                    this.flex_col().gap(theme.metrics.space_xs)
                }
            })
            .child(
                div()
                    .flex_none()
                    .when_some(self.label_width, |this, width| this.w(width))
                    .child(self.label),
            )
            .child(div().flex_1().min_w_0().child(self.control))
    }
}

/// A read-only value with a label.
#[derive(IntoElement)]
pub struct OutputField {
    label: SharedString,
    value: SharedString,
}

impl OutputField {
    pub fn new(label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

impl RenderOnce for OutputField {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Field::new(self.label, div().child(self.value))
    }
}

/// The frame shared by text fields.
pub(super) fn input_frame(
    id: ElementId,
    label: Option<SharedString>,
    focused: bool,
    disabled: bool,
    compact: bool,
    cx: &App,
) -> InputBase {
    let theme = *cx.theme();
    InputBase::new(id)
        .focused(focused)
        .disabled(disabled)
        .when_some(label, |this, label| this.accessibility_label(label))
        .styles(|styles| {
            styles
                .focused(|style| focus_ring(style, cx))
                .disabled(|style| style.opacity(0.55))
        })
        .flex()
        .items_center()
        .w_full()
        .min_w_0()
        .px(if compact {
            theme.metrics.space_snug
        } else {
            theme.metrics.field_padding_x
        })
        .border_1()
        .border_color(theme.colors.control_border)
        .rounded(if compact {
            theme.metrics.radius_sm
        } else {
            theme.metrics.radius_md
        })
        .bg(theme.colors.control_bg)
        .text_color(theme.colors.text)
        .text_size(if compact {
            theme.metrics.font_sm
        } else {
            theme.metrics.font_md
        })
}

#[derive(IntoElement)]
pub struct TextInput {
    id: ElementId,
    state: Entity<InputState>,
    label: Option<SharedString>,
    disabled: bool,
    compact: bool,
    focus_ring: bool,
}

impl TextInput {
    pub fn new(id: impl Into<ElementId>, state: &Entity<InputState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            label: None,
            disabled: false,
            compact: false,
            focus_ring: true,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The inspector and toolbar size.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Inline rename fields can retain their caret and neutral border without an outer focus outline.
    pub fn focus_ring(mut self, enabled: bool) -> Self {
        self.focus_ring = enabled;
        self
    }
}

impl RenderOnce for TextInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let metrics = cx.theme().metrics;
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        input_frame(
            self.id,
            self.label,
            focused,
            self.disabled,
            self.compact,
            cx,
        )
        .when(!self.focus_ring, |frame| {
            frame.styles(|styles| styles.focused(|style| style.shadow(Vec::new())))
        })
        .h(if self.compact {
            metrics.control_height_compact
        } else {
            metrics.control_height
        })
        .child(Input::new(&self.state))
    }
}

#[derive(IntoElement)]
pub struct Textarea {
    id: ElementId,
    state: Entity<TextareaState>,
    label: Option<SharedString>,
    disabled: bool,
    rows: f32,
    grow: bool,
    text_size: Option<Pixels>,
}

impl Textarea {
    pub fn new(id: impl Into<ElementId>, state: &Entity<TextareaState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            label: None,
            disabled: false,
            rows: 3.,
            grow: false,
            text_size: None,
        }
    }

    /// For a state made by [`growing_textarea_state`]: the frame is as tall as the text instead of `rows` tall.
    pub fn grow(mut self) -> Self {
        self.grow = true;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn rows(mut self, rows: f32) -> Self {
        self.rows = rows;
        self
    }

    pub fn text_size(mut self, size: Pixels) -> Self {
        self.text_size = Some(size);
        self
    }
}

impl RenderOnce for Textarea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let metrics = cx.theme().metrics;
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        input_frame(self.id, self.label, focused, self.disabled, false, cx)
            .items_start()
            .py(metrics.space_xs)
            .when_some(self.text_size, |frame, size| frame.text_size(size))
            .when(!self.grow, |frame| frame.h(metrics.row_height * self.rows))
            .child(gpui_base::input::Textarea::new(&self.state))
    }
}

#[derive(IntoElement)]
pub struct RangeInput {
    state: Entity<SliderState>,
    label: Option<SharedString>,
    disabled: bool,
    scrub: bool,
}

impl RangeInput {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            label: None,
            disabled: false,
            scrub: false,
        }
    }

    /// A straight progress track and playhead for media transport.
    pub fn scrub(mut self) -> Self {
        self.scrub = true;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for RangeInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        // gpui-base rounds the value to the step but keeps the raw pointer position as the percentage, so the
        // thumb would rest between stops. Re-seat the state on its (already snapped) value.
        let state = self.state.read(cx);
        let (min, max, value) = (state.min_value(), state.max_value(), state.value());
        if max > min
            && (state.percentage().end - ((value.end() - min) / (max - min)).clamp(0., 1.)).abs()
                > 1e-6
        {
            self.state
                .update(cx, |state, cx| state.set_value(value.end(), window, cx));
        }
        let percent = self.state.read(cx).percentage().end;
        let (bar, thumb) = (theme.metrics.space_xs, theme.metrics.space_md);
        let thumb_width = if self.scrub {
            theme.metrics.space_2xs
        } else {
            thumb
        };
        Slider::new(&self.state)
            .when_some(self.label, |this, label| this.accessibility_label(label))
            .disabled(self.disabled)
            .when(self.disabled && !self.scrub, |this| this.opacity(0.55))
            .h(theme.metrics.row_height)
            .w_full()
            .child(
                SliderTrack::new(&self.state)
                    .disabled(self.disabled)
                    .flex()
                    .items_center()
                    .size_full()
                    .child(
                        SliderIndicator::new(&self.state)
                            .relative()
                            .w_full()
                            .h(bar)
                            .rounded(theme.metrics.radius_pill)
                            .bg(if self.scrub {
                                theme.colors.text_muted
                            } else {
                                theme.colors.control_border
                            })
                            .child(
                                div()
                                    .debug_selector(|| "range-fill".into())
                                    .absolute()
                                    .left_0()
                                    .h_full()
                                    .w(relative(percent))
                                    .rounded(theme.metrics.radius_pill)
                                    .bg(theme.colors.accent),
                            )
                            .child(
                                SliderThumb::new(&self.state)
                                    .disabled(self.disabled)
                                    .absolute()
                                    .top((bar - thumb) / 2.)
                                    .debug_selector(|| "range-thumb".into())
                                    .left(relative(percent))
                                    .ml(thumb_width / -2.)
                                    .w(thumb_width)
                                    .h(thumb)
                                    .when(!self.scrub, |thumb| {
                                        thumb.rounded(theme.metrics.radius_pill)
                                    })
                                    .when(!self.scrub, |thumb| thumb.border_1())
                                    .border_color(theme.colors.control_border)
                                    .bg(theme.colors.accent_text),
                            ),
                    ),
            )
    }
}
/// Whether keyboard focus is in a control that consumes typing, or in a dialog. Global shortcuts must not run
/// then, as the React shell ignores shortcuts in form controls and while a dialog is open.
pub fn text_entry_focused(window: &gpui::Window) -> bool {
    const CONSUMING: [&str; 6] = [
        "Input",
        "NumberInput",
        "Select",
        "Combobox",
        "TimeField",
        "Dialog",
    ];
    window
        .context_stack()
        .iter()
        .any(|context| CONSUMING.iter().any(|name| context.contains(name)))
}
