use super::{
    geometry::{self, Point},
    view::SlideCanvas,
};
use gpui::{
    div, prelude::*, px, AnyElement, App, Bounds, Element, ElementId, Entity, GlobalElementId,
    Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseDownEvent,
    MouseExitEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollEvent, Window,
};
use orbit_ui::ActiveTheme;

const RULER_TOP: f32 = 18.;
const RULER_LEFT: f32 = 24.;

/// True when `p`, relative to the surface, lies under a ruler.
fn on_ruler(p: Point) -> bool {
    p.y < RULER_TOP || p.x < RULER_LEFT
}

/// One hit-test surface; content remains FrameView bitmaps, never per-object UI renderers.
pub struct Surface {
    owner: Entity<SlideCanvas>,
    inner: Option<AnyElement>,
}
impl Surface {
    pub fn new(owner: Entity<SlideCanvas>) -> Self {
        Self { owner, inner: None }
    }
}
impl IntoElement for Surface {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Surface {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;
    fn id(&self) -> Option<ElementId> {
        Some("slide-canvas-surface".into())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let view = self.owner.read(cx);
        let theme = *cx.theme();
        let origin = view.view.origin();
        let zoom = view.view.zoom;
        let slide = view.view.slide;
        let matte = view.document.as_ref().and_then(|_| {
            orbit_host::native_compositor::opaque_canvas_matte(&view.slide, view.render_kind())
        });
        let checkerboard = view.tools.checkerboard && matte.is_none();
        let mut content = div()
            .absolute()
            .debug_selector(|| "canvas-slide-bounds".into())
            .left(px(origin.x))
            .top(px(origin.y))
            .w(px(slide.x * zoom))
            .h(px(slide.y * zoom))
            .overflow_hidden()
            .bg(if checkerboard {
                theme.colors.preview_matte_bg
            } else {
                theme.colors.canvas_transparent_bg
            })
            .when_some(matte, |content, color| {
                content.bg(orbit_ui::color::opaque_rgb24(color))
            })
            .when(checkerboard, |content| {
                use super::content_document::DocumentKind;
                let id = match view.kind {
                    DocumentKind::Mask => "mask-canvas",
                    DocumentKind::Stage => "stage-canvas",
                    _ => "slide-canvas",
                };
                content.child(crate::media::transparency_background(id, cx))
            });
        if view.planes {
            content = content.child(div().absolute().size_full().child(view.frames[0].clone()));
        }
        {
            for i in 1..4 {
                let mut position = Point::default();
                let mut scale = Point::new(1., 1.);
                if i == 2 {
                    if let Some((original, current)) = view.plane_transform() {
                        scale = Point::new(
                            current.width / original.width,
                            current.height / original.height,
                        );
                        position = Point::new(
                            (current.x - original.x * scale.x) * zoom,
                            (current.y - original.y * scale.y) * zoom,
                        );
                    }
                }
                content = content.child(
                    div()
                        .absolute()
                        .left(px(position.x))
                        .top(px(position.y))
                        .w(px(slide.x * zoom * scale.x))
                        .h(px(slide.y * zoom * scale.y))
                        .child(view.frames[i].clone()),
                );
            }
        }
        if !view.planes {
            content = content.child(div().absolute().size_full().child(view.frames[0].clone()));
        }
        content = content.children(super::guides::overlay(view, cx));
        let mut root = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.colors.canvas_bg)
            .child(content);
        if let Some(index) = view.selected.filter(|index| {
            view.slide["layers"]
                .as_array()
                .is_some_and(|layers| *index < layers.len() && layers[*index]["hidden"] != true)
        }) {
            let bounds = view
                .gesture
                .current()
                .or_else(|| view.plane_transform().map(|(_, current)| current))
                .unwrap_or_else(|| geometry::layer_bounds(&view.slide["layers"][index], slide));
            let position = view.view.to_view(Point::new(bounds.x, bounds.y));
            root = root.child(
                div()
                    .absolute()
                    .debug_selector(|| "canvas-selected-bounds".into())
                    .left(px(position.x))
                    .top(px(position.y))
                    .w(px(bounds.width * zoom))
                    .h(px(bounds.height * zoom))
                    .border_2()
                    .border_color(theme.colors.selected),
            );
            for handle in geometry::HANDLES {
                let p = view.view.to_view(bounds.handle(handle));
                root = root.child(
                    div()
                        .absolute()
                        .left(px(p.x - 4.))
                        .top(px(p.y - 4.))
                        .size(px(8.))
                        .bg(theme.colors.panel_bg)
                        .border_1()
                        .border_color(theme.colors.selected),
                );
            }
        }
        let mut paths = Vec::new();
        if !view.editor.pen.is_empty() {
            let mut points = view.editor.pen.clone();
            if let Some(preview) = view.editor.pen_preview {
                points.push(preview);
            }
            paths.push(
                points
                    .into_iter()
                    .map(|point| view.view.to_view(point))
                    .collect::<Vec<_>>(),
            );
        }
        if view.editor.tab == "object" {
            if let Some(index) = view
                .selected
                .filter(|index| view.slide["layers"][*index]["hidden"] != true)
            {
                let layer = &view.slide["layers"][index];
                let bounds = geometry::layer_bounds(layer, slide);
                let local = layer["pathCoordinateSpace"] == "local";
                for point in layer["pathPoints"].as_array().into_iter().flatten() {
                    let to_view = |point: &serde_json::Value| {
                        let (x, y) = (point["x"].as_f64()?, point["y"].as_f64()?);
                        Some(view.view.to_view(Point::new(
                            x as f32 + if local { bounds.x } else { 0. },
                            y as f32 + if local { bounds.y } else { 0. },
                        )))
                    };
                    if let Some(node) = to_view(point) {
                        root = root.child(
                            div()
                                .absolute()
                                .left(px(node.x - 3.))
                                .top(px(node.y - 3.))
                                .size(px(6.))
                                .bg(theme.colors.accent),
                        );
                        for key in ["q0", "q1"] {
                            if let Some(control) = to_view(&point[key]) {
                                paths.push(vec![node, control]);
                                root = root.child(
                                    div()
                                        .absolute()
                                        .left(px(control.x - 3.))
                                        .top(px(control.y - 3.))
                                        .size(px(6.))
                                        .border_1()
                                        .border_color(theme.colors.accent)
                                        .bg(theme.colors.panel_bg),
                                );
                            }
                        }
                    }
                }
            }
        }
        for point in &view.editor.pen {
            let p = view.view.to_view(*point);
            root = root.child(
                div()
                    .absolute()
                    .left(px(p.x - 3.))
                    .top(px(p.y - 3.))
                    .size(px(6.))
                    .bg(theme.colors.accent),
            );
        }
        let color = theme.colors.accent;
        root = root.child(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    for points in paths {
                        let mut builder = gpui::PathBuilder::stroke(px(1.));
                        for (index, point) in points.into_iter().enumerate() {
                            let point = gpui::point(
                                bounds.origin.x + px(point.x),
                                bounds.origin.y + px(point.y),
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
                    }
                },
            )
            .absolute()
            .size_full(),
        );
        // Ruler units are slide coordinates; tick/handle sizes stay in logical pixels.
        if !view.editor.guides_hidden {
            root = root.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(RULER_TOP))
                    .bg(theme.colors.panel_bg_soft),
            );
            root = root.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .bottom_0()
                    .w(px(RULER_LEFT))
                    .bg(theme.colors.panel_bg_soft),
            );
            let step = [10., 20., 50., 100., 200., 500., 1000.]
                .into_iter()
                .find(|step| *step * zoom >= 50.)
                .unwrap_or(1000.);
            let first = view.view.to_slide(Point::new(RULER_LEFT, RULER_TOP));
            let last = view.view.to_slide(view.view.viewport);
            for (horizontal, start, end) in [(true, first.x, last.x), (false, first.y, last.y)] {
                let mut tick = (start / step).ceil() * step;
                while tick <= end {
                    let p = view.view.to_view(Point::new(tick, tick));
                    let label = div()
                        .absolute()
                        .text_size(px(9.))
                        .text_color(theme.colors.text_muted)
                        .child(format!("{}", geometry::round(tick)));
                    root = root.child(if horizontal {
                        label.left(px(p.x + 2.)).top(px(1.))
                    } else {
                        label.left(px(1.)).top(px(p.y + 2.))
                    });
                    root = root.child(if horizontal {
                        div()
                            .absolute()
                            .left(px(p.x))
                            .top(px(13.))
                            .w(px(1.))
                            .h(px(5.))
                            .bg(theme.colors.border_strong)
                    } else {
                        div()
                            .absolute()
                            .left(px(19.))
                            .top(px(p.y))
                            .w(px(5.))
                            .h(px(1.))
                            .bg(theme.colors.border_strong)
                    });
                    tick += step;
                }
            }
        }
        let mut inner = root.into_any_element();
        let layout = inner.request_layout(window, cx);
        self.inner = Some(inner);
        (layout, ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.owner.update(cx, |view, cx| {
            view.layout(
                Point::new(bounds.size.width.into(), bounds.size.height.into()),
                Point::new(bounds.origin.x.into(), bounds.origin.y.into()),
                window.scale_factor(),
                cx,
            )
        });
        self.inner.as_mut().unwrap().prepaint(window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.as_mut().unwrap().paint(window, cx);
        let owner = self.owner.clone();
        let hit = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase.bubble() && hit.is_hovered(window) {
                let capture = owner.update(cx, |view, cx| {
                    let p = Point::new(event.position.x.into(), event.position.y.into())
                        .minus(view.origin);
                    if !view.editor.guides_hidden && on_ruler(p) {
                        return true;
                    }
                    view.down(p, event.button, window, cx)
                });
                if capture {
                    window.capture_pointer(hit.id);
                    cx.stop_propagation();
                }
            }
        });
        let owner = self.owner.clone();
        let hit = hitbox.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase.capture() {
                let pointer = (hit.bounds.contains(&event.position) && hit.is_hovered(window))
                    .then_some(event.position);
                owner.update(cx, |view, cx| {
                    if view.canvas_pointer != pointer {
                        view.canvas_pointer = pointer;
                        cx.notify();
                    }
                });
            }
            if phase.bubble() && hit.is_hovered(window) {
                owner.update(cx, |view, cx| {
                    let p = Point::new(event.position.x.into(), event.position.y.into())
                        .minus(view.origin);
                    view.motion(p, cx);
                });
            }
        });
        let owner = self.owner.clone();
        window.on_mouse_event(move |_: &MouseExitEvent, phase, _, cx| {
            if phase.capture() {
                owner.update(cx, |view, cx| {
                    if view.canvas_pointer.take().is_some() {
                        cx.notify();
                    }
                });
            }
        });
        let owner = self.owner.clone();
        let hit = hitbox.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase.bubble()
                && hit.is_hovered(window)
                && matches!(
                    event.button,
                    gpui::MouseButton::Left | gpui::MouseButton::Middle
                )
            {
                owner.update(cx, |view, cx| {
                    let p = Point::new(event.position.x.into(), event.position.y.into())
                        .minus(view.origin);
                    view.up(p, cx);
                });
                window.release_pointer();
            }
        });
        let owner = self.owner.clone();
        let hit = hitbox.clone();
        window.on_mouse_event(move |event: &ScrollEvent, phase, window, cx| {
            if phase.bubble() && hit.is_hovered(window) {
                let delta = event.delta.pixel_delta(px(20.));
                owner.update(cx, |view, cx| {
                    view.wheel(
                        Point::new(delta.x.into(), delta.y.into()),
                        event.modifiers.control || event.modifiers.platform,
                        cx,
                    )
                });
                cx.stop_propagation();
            }
        });
    }
}
