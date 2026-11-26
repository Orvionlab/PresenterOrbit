use super::*;
use gpui::{Bounds, MouseButton, Pixels, Point};
use orbit_ui::controls::{data_color, tooltip_text};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy)]
pub(super) struct TrimRange {
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Copy)]
enum TrimEdge {
    Start,
    End,
}

pub(super) struct TrimDrag {
    edge: TrimEdge,
    initial: TrimRange,
    range: TrimRange,
}

pub(super) struct MarkerDrag {
    id: String,
    origin: Pixels,
    initial: f64,
    position: Option<f64>,
}

impl TrimRange {
    pub(super) fn from_asset(item: &Value, duration: f64) -> Self {
        let trim = &item[trim_field(item)];
        let end = trim["outSeconds"]
            .as_f64()
            .unwrap_or(duration)
            .min(duration);
        let start = trim["inSeconds"].as_f64().unwrap_or(0.);
        if start < end {
            Self { start, end }
        } else {
            Self {
                start: 0.,
                end: duration,
            }
        }
    }

    fn move_edge(self, edge: TrimEdge, seconds: f64, duration: f64) -> Self {
        let gap = (duration / 2.).min(0.01);
        match edge {
            TrimEdge::Start => Self {
                start: seconds.min((self.end - gap).max(0.)),
                ..self
            },
            TrimEdge::End => Self {
                end: seconds.max((self.start + gap).min(duration)),
                ..self
            },
        }
    }
}

fn trim_field(item: &Value) -> &'static str {
    if item["type"] == "video" {
        "mediaTransport"
    } else {
        "audioPreviewTrim"
    }
}

fn time_at(bounds: Bounds<Pixels>, position: Point<Pixels>, duration: f64) -> Option<f64> {
    (bounds.size.width > gpui::px(0.)).then(|| {
        let fraction = f32::from(position.x - bounds.left()) / f32::from(bounds.size.width);
        f64::from(fraction.clamp(0., 1.)) * duration
    })
}

impl MediaBin {
    fn finish_preview_trim(&mut self, time: Option<f64>, duration: f64, cx: &mut Context<Self>) {
        let Some(drag) = self.preview_trim_drag.take() else {
            return;
        };
        cx.notify();
        let Some(time) = time else { return };
        self.commit_preview_trim(
            drag.initial.move_edge(drag.edge, time, duration),
            duration,
            cx,
        );
    }

    pub(super) fn commit_preview_trim(
        &mut self,
        range: TrimRange,
        duration: f64,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.media.iter().find(|item| item["id"] == self.selected) else {
            return;
        };
        let field = trim_field(item);
        let current = TrimRange::from_asset(item, duration);
        if (current.start, current.end) == (range.start, range.end) || self.edits_blocked() {
            return;
        }
        let mut updated = item.get(field).cloned().unwrap_or_else(|| {
            if field == "mediaTransport" {
                json!({"rate":1,"delaySeconds":0,"retrigger":"restart"})
            } else {
                json!({})
            }
        });
        updated["inSeconds"] = json!(range.start);
        updated["outSeconds"] = if range.end < duration {
            json!(range.end)
        } else {
            Value::Null
        };
        self.inputs.remove("preview-in-point");
        self.inputs.remove("preview-out-point");
        self.patch_asset(field, updated, cx);
        self.preview_seek(self.preview_position.clamp(range.start, range.end), cx);
    }

    pub(super) fn displayed_preview_trim(&self, item: &Value, duration: f64) -> TrimRange {
        self.preview_trim_drag
            .as_ref()
            .map_or_else(|| TrimRange::from_asset(item, duration), |drag| drag.range)
    }

    fn finish_marker_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.preview_marker_drag.take() else {
            return;
        };
        if let Some(seconds) = drag.position {
            self.move_preview_marker(&drag.id, seconds, cx);
        } else {
            self.preview_seek(drag.initial, cx);
        }
        cx.notify();
    }

    pub(super) fn preview_timeline(
        &self,
        item: &Value,
        duration: f64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let m = theme.metrics;
        let marker_size = m.space_lg + m.space_xs;
        let saved = TrimRange::from_asset(item, duration);
        let range = self.displayed_preview_trim(item, duration);
        let percent = |seconds: f64| gpui::relative((seconds / duration).clamp(0., 1.) as f32);
        let bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
        let motion_marker = bounds.clone();
        let (paint, press, motion, release, release_out) = (
            bounds.clone(),
            bounds.clone(),
            bounds.clone(),
            bounds.clone(),
            bounds,
        );
        let markers = self
            .marker_draft
            .as_ref()
            .unwrap_or(&item["markers"])
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, marker)| {
                let id = text(marker, "id").to_owned();
                let seconds = marker["timeSeconds"]
                    .as_f64()
                    .filter(|seconds| (0. ..=duration).contains(seconds))?;
                let display_time = self
                    .preview_marker_drag
                    .as_ref()
                    .filter(|drag| drag.id == id)
                    .and_then(|drag| drag.position)
                    .unwrap_or(seconds);
                let label = format!(
                    "{} ({})",
                    text(marker, "title"),
                    crate::automation::form::format_time(display_time)
                );
                let color = marker["color"]
                    .as_str()
                    .map(data_color)
                    .unwrap_or(theme.colors.accent);
                Some(
                    div()
                        .id(SharedString::from(format!("preview-marker-{id}")))
                        .debug_selector(move || format!("preview-marker-{index}"))
                        .absolute()
                        .left(percent(display_time))
                        .top_0()
                        .ml(-marker_size / 2.)
                        .size(marker_size)
                        .cursor_pointer()
                        .child(
                            orbit_ui::icon("media-marker-tag")
                                .size(marker_size)
                                .text_color(color),
                        )
                        .tooltip(tooltip_text(label))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                if this.edits_blocked() {
                                    this.preview_seek(seconds, cx);
                                } else {
                                    this.preview_marker_drag = Some(MarkerDrag {
                                        id: id.clone(),
                                        origin: event.position.x,
                                        initial: seconds,
                                        position: None,
                                    });
                                }
                                cx.stop_propagation();
                            }),
                        ),
                )
            })
            .collect::<Vec<_>>();
        div()
            .id("preview-timeline-controls")
            .flex_none()
            .flex()
            .flex_col()
            .mx(m.space_xs)
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    let Some(drag) = this.preview_marker_drag.as_mut() else {
                        return;
                    };
                    if event.pressed_button != Some(MouseButton::Left) {
                        this.preview_marker_drag = None;
                        cx.notify();
                        return;
                    }
                    let Some(area) = motion_marker.get() else {
                        return;
                    };
                    let delta = event.position.x - drag.origin;
                    if area.size.width > gpui::px(0.)
                        && (drag.position.is_some() || delta.abs() >= m.space_2xs)
                    {
                        let fraction = f64::from(f32::from(delta) / f32::from(area.size.width));
                        drag.position =
                            Some((drag.initial + duration * fraction).clamp(0., duration));
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_marker_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_marker_drag(cx)),
            )
            .child(div().relative().h(marker_size).children(markers))
            .child(
                div()
                    .id("preview-timeline")
                    .debug_selector(|| "preview-timeline".into())
                    .relative()
                    .h(m.space_lg)
                    .rounded(m.radius_sm)
                    .bg(theme.colors.preview_matte_bg)
                    .cursor_pointer()
                    .child(
                        gpui::canvas(move |area, _, _| paint.set(Some(area)), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(percent(range.start))
                            .w(percent(range.end - range.start))
                            .bg(theme.colors.accent_soft),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(percent(self.preview_position))
                            .debug_selector(|| "preview-playhead".into())
                            .w(m.space_2xs)
                            .bg(theme.colors.accent),
                    )
                    .children(
                        [
                            (
                                range.start,
                                TrimEdge::Start,
                                "preview-trim-start",
                                t.t("media.preview.trimStart"),
                            ),
                            (
                                range.end,
                                TrimEdge::End,
                                "preview-trim-end",
                                t.t("media.preview.trimEnd"),
                            ),
                        ]
                        .into_iter()
                        .map(|(seconds, edge, id, label)| {
                            div()
                                .id(id)
                                .debug_selector(move || id.into())
                                .absolute()
                                .left(percent(seconds))
                                .top_0()
                                .bottom_0()
                                .ml(-m.space_xs)
                                .w(m.space_sm)
                                .border_1()
                                .border_color(theme.colors.border_strong)
                                .rounded(m.radius_xs)
                                .bg(theme.colors.control_bg)
                                .cursor(gpui::CursorStyle::ResizeLeftRight)
                                .tooltip(tooltip_text(label))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        if !this.edits_blocked() {
                                            this.preview_trim_drag = Some(TrimDrag {
                                                edge,
                                                initial: saved,
                                                range: saved,
                                            });
                                            cx.notify();
                                        }
                                        cx.stop_propagation();
                                    }),
                                )
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                            let Some(area) = press.get() else { return };
                            let Some(time) = time_at(area, event.position, duration) else {
                                return;
                            };
                            this.preview_seek(time, cx);
                        }),
                    )
                    .on_mouse_move(
                        cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                            if event.pressed_button != Some(MouseButton::Left) {
                                if this.preview_trim_drag.take().is_some() {
                                    cx.notify();
                                }
                                return;
                            }
                            if let Some((drag, time)) = this.preview_trim_drag.as_mut().zip(
                                motion
                                    .get()
                                    .and_then(|area| time_at(area, event.position, duration)),
                            ) {
                                drag.range = drag.initial.move_edge(drag.edge, time, duration);
                                cx.notify();
                            }
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseUpEvent, _, cx| {
                            this.finish_preview_trim(
                                release
                                    .get()
                                    .and_then(|area| time_at(area, event.position, duration)),
                                duration,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseUpEvent, _, cx| {
                            this.finish_preview_trim(
                                release_out
                                    .get()
                                    .and_then(|area| time_at(area, event.position, duration)),
                                duration,
                                cx,
                            );
                        }),
                    ),
            )
    }
}
