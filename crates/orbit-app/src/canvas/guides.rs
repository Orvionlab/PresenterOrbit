use super::{
    content_document::DocumentKind,
    geometry::{self, Bounds, Gesture, Point},
    view::SlideCanvas,
};
use gpui::{canvas, div, fill, prelude::*, px, AnyElement, App, Rgba};
use orbit_ui::ActiveTheme;

pub(super) struct Guides {
    pub slide: bool,
    pub theme: bool,
    pub spacing: bool,
    pub sizing: bool,
    pub centers: bool,
    pub edges: bool,
    pub slide_color: Option<String>,
    pub theme_color: Option<String>,
    pub alignment_color: Option<String>,
}

impl Default for Guides {
    fn default() -> Self {
        Self {
            slide: true,
            theme: true,
            spacing: true,
            sizing: true,
            centers: true,
            edges: true,
            slide_color: None,
            theme_color: None,
            alignment_color: None,
        }
    }
}

pub(super) struct Grid {
    pub enabled: bool,
    pub color: Option<String>,
    pub spacing: u32,
    pub subdivisions: u32,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            enabled: false,
            color: None,
            spacing: 120,
            subdivisions: 3,
        }
    }
}

type GuideColor = Rgba;

fn color(value: Option<&str>, fallback: GuideColor) -> GuideColor {
    value
        .and_then(crate::cues::parse_hex_color)
        .unwrap_or(fallback)
}

#[derive(Clone, Copy)]
struct Line(Point, Point);

fn outline(bounds: Bounds) -> [Line; 4] {
    let Point { x, y } = Point::new(bounds.x, bounds.y);
    let (right, bottom) = (x + bounds.width, y + bounds.height);
    [
        Line(Point::new(x, y), Point::new(right, y)),
        Line(Point::new(x, bottom), Point::new(right, bottom)),
        Line(Point::new(x, y), Point::new(x, bottom)),
        Line(Point::new(right, y), Point::new(right, bottom)),
    ]
}

fn slide_lines(slide: Point, object: Bounds, tolerance: f32) -> Vec<Line> {
    let touches = |line: f32, start: f32, extent: f32| {
        [start, start + extent / 2., start + extent]
            .into_iter()
            .any(|edge| (edge - line).abs() <= tolerance)
    };
    let vertical = [0., slide.x / 2., slide.x]
        .into_iter()
        .filter(|x| touches(*x, object.x, object.width))
        .map(|x| Line(Point::new(x, 0.), Point::new(x, slide.y)));
    let horizontal = [0., slide.y / 2., slide.y]
        .into_iter()
        .filter(|y| touches(*y, object.y, object.height))
        .map(|y| Line(Point::new(0., y), Point::new(slide.x, y)));
    vertical.chain(horizontal).collect()
}

/// Return an equal gap only when the selection fits between two neighbours on this axis.
fn equal_gap(
    min: f32,
    max: f32,
    neighbours: impl Iterator<Item = (f32, f32)>,
    tolerance: f32,
) -> Option<(f32, f32)> {
    let (mut before, mut after) = (f32::NEG_INFINITY, f32::INFINITY);
    for (start, end) in neighbours {
        if end <= min {
            before = before.max(end);
        }
        if start >= max {
            after = after.min(start);
        }
    }
    (before.is_finite() && after.is_finite() && ((min - before) - (after - max)).abs() <= tolerance)
        .then_some((before, after))
}

fn alignment(guides: &Guides, active: Bounds, others: &[Bounds], tolerance: f32) -> Vec<Line> {
    let mut lines = Vec::new();
    let near = |a: f32, b: f32| (a - b).abs() <= tolerance;
    let (left, top) = (active.x, active.y);
    let (right, bottom) = (left + active.width, top + active.height);
    let (cx, cy) = (left + active.width / 2., top + active.height / 2.);
    for other in others {
        let (ox, oy) = (other.x + other.width, other.y + other.height);
        let (ocx, ocy) = (other.x + other.width / 2., other.y + other.height / 2.);
        for x in [left, cx, right] {
            let show = if x == cx {
                guides.centers && near(x, ocx)
            } else {
                guides.edges && (near(x, other.x) || near(x, ox))
            };
            if show {
                lines.push(Line(
                    Point::new(x, top.min(other.y)),
                    Point::new(x, bottom.max(oy)),
                ));
            }
        }
        for y in [top, cy, bottom] {
            let show = if y == cy {
                guides.centers && near(y, ocy)
            } else {
                guides.edges && (near(y, other.y) || near(y, oy))
            };
            if show {
                lines.push(Line(
                    Point::new(left.min(other.x), y),
                    Point::new(right.max(ox), y),
                ));
            }
        }
        if guides.sizing {
            if near(active.width, other.width) {
                lines.extend([
                    Line(Point::new(left, top), Point::new(right, top)),
                    Line(Point::new(other.x, other.y), Point::new(ox, other.y)),
                ]);
            }
            if near(active.height, other.height) {
                lines.extend([
                    Line(Point::new(right, top), Point::new(right, bottom)),
                    Line(Point::new(ox, other.y), Point::new(ox, oy)),
                ]);
            }
        }
    }
    if guides.spacing {
        let horizontal = others
            .iter()
            .filter(|b| b.y <= cy && cy <= b.y + b.height)
            .map(|b| (b.x, b.x + b.width));
        if let Some((before, after)) = equal_gap(left, right, horizontal, tolerance) {
            lines.extend([
                Line(Point::new(before, cy), Point::new(left, cy)),
                Line(Point::new(right, cy), Point::new(after, cy)),
            ]);
        }
        let vertical = others
            .iter()
            .filter(|b| b.x <= cx && cx <= b.x + b.width)
            .map(|b| (b.y, b.y + b.height));
        if let Some((before, after)) = equal_gap(top, bottom, vertical, tolerance) {
            lines.extend([
                Line(Point::new(cx, before), Point::new(cx, top)),
                Line(Point::new(cx, bottom), Point::new(cx, after)),
            ]);
        }
    }
    lines
}

pub(super) fn overlay(view: &SlideCanvas, cx: &App) -> Option<AnyElement> {
    view.document.as_ref()?;
    let tools = &view.tools;
    let guides = &tools.guides;
    let theme = cx.theme();
    let slide = view.view.slide;
    let zoom = view.view.zoom;
    let origin = view.view.origin();
    let viewport = view.view.viewport;
    let mut batches = Vec::new();
    if guides.theme {
        let template = match view.kind {
            DocumentKind::Slide => orbit_host::authoring::slide_theme_ref(
                &view.runtime.content.read(cx).workspace,
                &view.slide,
            ),
            DocumentKind::Theme => Some(view.slide.as_ref()),
            _ => None,
        };
        if let Some(template) = template {
            let bounds = template["layers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|layer| layer["hidden"] != true)
                .flat_map(|layer| outline(geometry::layer_bounds(layer, slide)))
                .collect();
            batches.push((
                color(guides.theme_color.as_deref(), theme.colors.object_guide),
                bounds,
            ));
        }
    }
    if let Gesture::Moving { current, .. } | Gesture::Resizing { current, .. } = view.gesture {
        let others: Vec<_> = view.slide["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(index, layer)| Some(*index) != view.selected && layer["hidden"] != true)
            .map(|(_, layer)| geometry::layer_bounds(layer, slide))
            .collect();
        let lines = alignment(guides, current, &others, 1.5 / zoom);
        if !lines.is_empty() {
            batches.push((
                color(guides.alignment_color.as_deref(), theme.colors.object_guide),
                lines,
            ));
        }
        if guides.slide {
            let lines = slide_lines(slide, current, 1.5 / zoom);
            if !lines.is_empty() {
                batches.push((
                    color(guides.slide_color.as_deref(), theme.colors.slide_guide),
                    lines,
                ));
            }
        }
    }
    let grid = tools.grid.enabled.then(|| {
        (
            tools.grid.spacing as f32 * zoom,
            tools.grid.subdivisions,
            color(tools.grid.color.as_deref(), theme.colors.text_muted),
        )
    });
    if batches.is_empty() && grid.is_none() {
        return None;
    }
    Some(
        div()
            .absolute()
            .size_full()
            .debug_selector(move || {
                if grid.is_some() {
                    "canvas-grid-overlay".into()
                } else {
                    "canvas-guides-overlay".into()
                }
            })
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let mut draw = |line: Line, color: Rgba| {
                            let Line(a, b) = line;
                            // Keep the right and bottom strokes inside the clipped canvas.
                            let (mut x, mut y) = (a.x.min(b.x), a.y.min(b.y));
                            if a.x == b.x && x == slide.x * zoom {
                                x = (x - 1.).max(0.);
                            }
                            if a.y == b.y && y == slide.y * zoom {
                                y = (y - 1.).max(0.);
                            }
                            let area = gpui::Bounds::new(
                                bounds.origin + gpui::point(px(x), px(y)),
                                gpui::size(
                                    px((a.x - b.x).abs().max(1.)),
                                    px((a.y - b.y).abs().max(1.)),
                                ),
                            );
                            window.paint_quad(fill(area, color));
                        };
                        if let Some((spacing, subdivisions, color)) = grid {
                            let width = slide.x * zoom;
                            let height = slide.y * zoom;
                            for (start, end, vertical) in [
                                (-origin.x, (viewport.x - origin.x).min(width), true),
                                (-origin.y, (viewport.y - origin.y).min(height), false),
                            ] {
                                let fine = spacing / subdivisions as f32;
                                let (step, period) = if fine >= 4. {
                                    (fine, subdivisions)
                                } else {
                                    (spacing * (4. / spacing).ceil().max(1.), 1)
                                };
                                let start = (start.max(0.) / step).ceil() as u32;
                                let end = (end / step).floor() as u32;
                                for i in start..=end {
                                    let n = i as f32 * step;
                                    let mut color = color;
                                    color.a *= if i % period == 0 { 0.65 } else { 0.30 };
                                    let line = if vertical {
                                        Line(Point::new(n, 0.), Point::new(n, height))
                                    } else {
                                        Line(Point::new(0., n), Point::new(width, n))
                                    };
                                    draw(line, color);
                                }
                            }
                        }
                        for (color, lines) in batches {
                            for Line(a, b) in lines {
                                draw(
                                    Line(
                                        Point::new(a.x * zoom, a.y * zoom),
                                        Point::new(b.x * zoom, b.y * zoom),
                                    ),
                                    color,
                                );
                            }
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element(),
    )
}
