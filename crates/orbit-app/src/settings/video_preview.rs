//! `DisplayRelationPreview.tsx`, `ScreenOutputPreview.tsx`, `VideoOutputDetails.tsx` (display map and details), and
//! the projector corner preview.

use gpui::{div, prelude::*, px, AnyElement, Context, SharedString, Window};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::ui::muted;
use super::view::SettingsView;
use crate::output::preview::{Preview, Source};

/// A display rectangle in map coordinates: left, top, width, height scaled into `width` pixels, negative offsets
/// kept (portrait and stacked layouts), and a missing size dropped rather than invented.
pub fn display_rects(displays: &Value, width: f32) -> Vec<(String, [f32; 4])> {
    let boxes: Vec<(String, f64, f64, f64, f64)> = displays
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let b = &d["bounds"];
            let (w, h) = (b["width"].as_f64()?, b["height"].as_f64()?);
            (w > 0. && h > 0.).then(|| {
                (
                    d["id"].as_str().unwrap_or("").to_string(),
                    b["x"].as_f64().unwrap_or(0.),
                    b["y"].as_f64().unwrap_or(0.),
                    w,
                    h,
                )
            })
        })
        .collect();
    let left = boxes.iter().map(|b| b.1).fold(f64::INFINITY, f64::min);
    let top = boxes.iter().map(|b| b.2).fold(f64::INFINITY, f64::min);
    let right = boxes
        .iter()
        .map(|b| b.1 + b.3)
        .fold(f64::NEG_INFINITY, f64::max);
    if boxes.is_empty() || right <= left {
        return Vec::new();
    }
    let scale = f64::from(width) / (right - left);
    boxes
        .into_iter()
        .map(|(id, x, y, w, h)| {
            (
                id,
                [
                    ((x - left) * scale) as f32,
                    ((y - top) * scale) as f32,
                    (w * scale) as f32,
                    (h * scale) as f32,
                ],
            )
        })
        .collect()
}

/// The outline of the projector correction corners in a unit square.
pub fn corner_points(correction: &Value) -> Vec<(f32, f32)> {
    ["topLeft", "topRight", "bottomRight", "bottomLeft"]
        .iter()
        .map(|c| {
            (
                correction["corners"][*c]["x"].as_f64().unwrap_or(0.) as f32,
                correction["corners"][*c]["y"].as_f64().unwrap_or(0.) as f32,
            )
        })
        .collect()
}

pub fn polygon(points: Vec<(f32, f32)>, color: gpui::Rgba, width: f32, height: f32) -> AnyElement {
    div()
        .w(px(width))
        .h(px(height))
        .child(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let mut builder = gpui::PathBuilder::stroke(px(1.));
                    for (index, (x, y)) in points.iter().chain(points.first()).enumerate() {
                        let point = gpui::point(
                            bounds.origin.x + bounds.size.width * *x,
                            bounds.origin.y + bounds.size.height * *y,
                        );
                        if index == 0 {
                            builder.move_to(point);
                        } else {
                            builder.line_to(point);
                        }
                    }
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, color);
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

impl SettingsView {
    /// The map of connected displays; the selected one is the screen's target, and a click chooses another.
    pub(super) fn display_map(
        &mut self,
        screen_id: &str,
        config: &Value,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let displays = self.runtime.output.read(cx).displays.clone();
        let rects = display_rects(&displays, 240.);
        if rects.is_empty() {
            return muted(t.t("screenSettings.displayLayoutUnavailable"), cx).into_any_element();
        }
        let target = config["targetId"].as_str().unwrap_or("");
        let mut map = div().relative().w(px(240.)).h(px(rects
            .iter()
            .map(|(_, r)| r[1] + r[3])
            .fold(0., f32::max)));
        for (id, [x, y, w, h]) in rects {
            let selected = config["targetType"] != "ndi" && id == target;
            let (screen, display) = (screen_id.to_string(), id.clone());
            map = map.child(
                div()
                    .id(SharedString::from(format!("display-map-{screen_id}-{id}")))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(w))
                    .h(px(h))
                    .border_1()
                    .border_color(theme.colors.border)
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.colors.selected))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.update_screen(
                            &screen,
                            json!({"targetType": "display", "targetId": display}),
                            cx,
                        )
                    })),
            );
        }
        let details = displays
            .as_array()
            .into_iter()
            .flatten()
            .find(|d| d["id"] == target)
            .map(|d| {
                let size = d["size"]
                    .as_object()
                    .map(|s| format!("{} x {}", s["width"], s["height"]))
                    .unwrap_or_else(|| t.t("common.unknown"));
                format!(
                    "{}  {size}{}",
                    d["label"].as_str().unwrap_or(""),
                    if d["primary"] == json!(true) {
                        "  *"
                    } else {
                        ""
                    }
                )
            })
            .unwrap_or_else(|| t.t("screenSettings.noDisplaySelected"));
        div()
            .flex()
            .flex_col()
            .gap(theme.metrics.space_xs)
            .child(map)
            .child(div().child(details))
            .into_any_element()
    }

    /// `ScreenOutputPreview`: the picture the output shows for this screen, from the same frames the main
    /// preview uses, in a matte at the output's aspect ratio. A screen that is off shows black.
    pub(super) fn screen_preview(
        &mut self,
        screen_id: &str,
        config: &Value,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = window;
        let theme = *cx.theme();
        let source_size = crate::output::view::screen_size_for_displays(
            config,
            &self.runtime.output.read(cx).displays,
        );
        let aspect = source_size.width as f32 / source_size.height as f32;
        let picture = if enabled {
            let (host, frames) = (self.runtime.host.clone(), self.runtime.frames.clone());
            let preview = self
                .video
                .previews
                .entry(screen_id.to_string())
                .or_insert_with(|| cx.new(|cx| Preview::new(host, frames, cx)))
                .clone();
            let revision = self.runtime.output.read(cx).committed_revision;
            let id = screen_id.to_string();
            preview.update(cx, |preview, cx| {
                preview.set_source(Some(Source::Output(id)), Some(source_size), revision, cx)
            });
            div().size_full().child(preview).into_any_element()
        } else {
            div().size_full().into_any_element()
        };
        div()
            .id(SharedString::from(format!("screen-preview-{screen_id}")))
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .aspect_ratio(16. / 7.2)
            .min_h(px(132.))
            .overflow_hidden()
            .border_1()
            .border_color(theme.colors.border_strong)
            .rounded(theme.metrics.radius_md)
            .bg(theme.colors.preview_matte_bg)
            .child(
                div()
                    .h_full()
                    .aspect_ratio(aspect)
                    .bg(orbit_ui::theme::PROGRAM_PREVIEW_BG)
                    .child(picture),
            )
            .into_any_element()
    }
}
