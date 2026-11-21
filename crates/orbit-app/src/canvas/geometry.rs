use serde_json::{json, Value};

pub const ZOOM_STEPS: [f32; 7] = [0.25, 0.5, 0.75, 1., 1.25, 1.5, 2.];
pub const HANDLES: [&str; 8] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn minus(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Bounds {
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x <= self.x + self.width && p.y <= self.y + self.height
    }
    pub fn value(self) -> Value {
        json!({"x":self.x as i64,"y":self.y as i64,"width":self.width as i64,"height":self.height as i64})
    }
    pub fn handle(self, handle: &str) -> Point {
        Point::new(
            self.x
                + if handle.contains('w') {
                    0.
                } else if handle.contains('e') {
                    self.width
                } else {
                    self.width / 2.
                },
            self.y
                + if handle.contains('n') {
                    0.
                } else if handle.contains('s') {
                    self.height
                } else {
                    self.height / 2.
                },
        )
    }
}
fn number(value: &Value) -> Option<f32> {
    value.as_f64().filter(|n| n.is_finite()).map(|n| n as f32)
}
pub fn slide_size(slide: &Value) -> Point {
    Point::new(
        number(&slide["width"])
            .filter(|n| *n != 0.)
            .unwrap_or(1920.)
            .max(1.),
        number(&slide["height"])
            .filter(|n| *n != 0.)
            .unwrap_or(1080.)
            .max(1.),
    )
}
pub fn layer_bounds(layer: &Value, slide: Point) -> Bounds {
    let b = &layer["bounds"];
    Bounds {
        x: number(&b["x"]).unwrap_or(slide.x * 0.14),
        y: number(&b["y"]).unwrap_or(slide.y * 0.22),
        width: number(&b["width"])
            .filter(|n| *n > 0.)
            .unwrap_or(slide.x * 0.72),
        height: number(&b["height"])
            .filter(|n| *n > 0.)
            .unwrap_or(slide.y * 0.56),
    }
}
// JavaScript Math.round rounds negative halves towards positive infinity.
pub fn round(n: f32) -> f32 {
    (n + 0.5).floor()
}
pub fn move_bounds(b: Bounds, d: Point) -> Bounds {
    Bounds {
        x: round(b.x + d.x),
        y: round(b.y + d.y),
        width: round(b.width),
        height: round(b.height),
    }
}
pub fn resize_bounds(b: Bounds, handle: &str, d: Point) -> Bounds {
    let mut next = b;
    if handle.contains('w') {
        let dx = d.x.min(b.width - 8.);
        next.x += dx;
        next.width -= dx;
    }
    if handle.contains('e') {
        next.width = (b.width + d.x).max(8.);
    }
    if handle.contains('n') {
        let dy = d.y.min(b.height - 8.);
        next.y += dy;
        next.height -= dy;
    }
    if handle.contains('s') {
        next.height = (b.height + d.y).max(8.);
    }
    Bounds {
        x: round(next.x),
        y: round(next.y),
        width: round(next.width),
        height: round(next.height),
    }
}
pub fn patch_bounds(layer: &Value, slide: Point, next: Bounds) -> Value {
    let mut layer = layer.clone();
    let old = layer_bounds(&layer, slide);
    let local = layer["pathCoordinateSpace"] == "local";
    if next.width != old.width || next.height != old.height {
        if let Some(points) = layer["pathPoints"].as_array_mut() {
            let scale = if points.len() >= 2 {
                Point::new(next.width / old.width, next.height / old.height)
            } else {
                Point::new(1., 1.)
            };
            for point in points {
                for key in ["", "q0", "q1"] {
                    let target = if key.is_empty() {
                        &mut *point
                    } else {
                        if point.get(key).is_none_or(Value::is_null) {
                            continue;
                        }
                        &mut point[key]
                    };
                    let coordinate = |key: &str, origin: f32, factor: f32| {
                        number(&target[key])
                            .map(|n| if local { n } else { n - origin })
                            .unwrap_or(0.)
                            * factor
                    };
                    let (x, y) = (
                        coordinate("x", old.x, scale.x),
                        coordinate("y", old.y, scale.y),
                    );
                    target["x"] = json!(x);
                    target["y"] = json!(y);
                }
            }
            layer["pathCoordinateSpace"] = json!("local");
        }
    } else if !local {
        // A move translates absolute points in f64 so saved coordinates gain no float noise.
        let delta = (next.x as f64 - old.x as f64, next.y as f64 - old.y as f64);
        if let Some(points) = layer["pathPoints"].as_array_mut() {
            for point in points {
                for key in ["", "q0", "q1"] {
                    let target = if key.is_empty() {
                        &mut *point
                    } else {
                        if point.get(key).is_none_or(Value::is_null) {
                            continue;
                        }
                        &mut point[key]
                    };
                    for (axis, shift) in [("x", delta.0), ("y", delta.1)] {
                        if let Some(n) = target[axis].as_f64() {
                            target[axis] = json!(n + shift);
                        }
                    }
                }
            }
        }
    }
    layer["bounds"] = next.value();
    layer
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasView {
    pub zoom: f32,
    pub pan: Point,
    pub scale_factor: f32,
    pub viewport: Point,
    pub slide: Point,
    pub fit: bool,
}
impl CanvasView {
    pub fn new(slide: Point) -> Self {
        Self {
            zoom: 0.59,
            pan: Point::default(),
            scale_factor: 1.,
            viewport: Point::default(),
            slide,
            fit: true,
        }
    }
    pub fn origin(self) -> Point {
        Point::new(
            (self.viewport.x - self.slide.x * self.zoom) / 2. + self.pan.x,
            (self.viewport.y - self.slide.y * self.zoom) / 2. + self.pan.y,
        )
    }
    pub fn to_slide(self, p: Point) -> Point {
        let p = p.minus(self.origin());
        Point::new(p.x / self.zoom, p.y / self.zoom)
    }
    pub fn to_view(self, p: Point) -> Point {
        let o = self.origin();
        Point::new(o.x + p.x * self.zoom, o.y + p.y * self.zoom)
    }
    pub fn fit_zoom(&mut self) {
        self.zoom = normalize_zoom(
            ((self.viewport.x - 88.).max(1.) / self.slide.x)
                .min((self.viewport.y - 72.).max(1.) / self.slide.y)
                .min(1.),
        );
        self.pan = Point::default();
        self.fit = true;
    }
    pub fn set_zoom(&mut self, zoom: f32) {
        let center = Point::new(self.viewport.x / 2., self.viewport.y / 2.);
        let anchor = self.to_slide(center);
        self.zoom = normalize_zoom(zoom);
        self.pan = Point::new(
            (self.slide.x / 2. - anchor.x) * self.zoom,
            (self.slide.y / 2. - anchor.y) * self.zoom,
        );
        self.fit = false;
        self.clamp_pan();
    }
    pub fn clamp_pan(&mut self) {
        let x = ((self.slide.x * self.zoom - self.viewport.x) / 2.).max(0.) + 1500.;
        let y = ((self.slide.y * self.zoom - self.viewport.y) / 2.).max(0.) + 1500.;
        self.pan.x = self.pan.x.clamp(-x, x);
        self.pan.y = self.pan.y.clamp(-y, y);
    }
    pub fn raster(self) -> (u32, u32) {
        let factor = (self.zoom * self.scale_factor).min(2.);
        let w = self.slide.x * factor;
        let h = self.slide.y * factor;
        let shrink = (8192. / w.max(h))
            .min((33_554_432. / (w * h)).sqrt())
            .min(1.);
        (
            (w * shrink).round().max(1.) as u32,
            (h * shrink).round().max(1.) as u32,
        )
    }
}
pub fn normalize_zoom(n: f32) -> f32 {
    if n.is_finite() && n > 0. {
        n.clamp(0.1, 2.)
    } else {
        0.59
    }
}
pub fn hit_layer(slide: &Value, view: CanvasView, p: Point) -> Option<usize> {
    let p = view.to_slide(p);
    let p = Point::new(round(p.x), round(p.y));
    slide["layers"]
        .as_array()?
        .iter()
        .rposition(|l| l["hidden"] != true && layer_bounds(l, view.slide).contains(p))
}
pub fn hit_handle(bounds: Bounds, view: CanvasView, p: Point) -> Option<&'static str> {
    HANDLES.into_iter().find(|h| {
        let c = view.to_view(bounds.handle(h));
        (c.x - p.x).abs() <= 5. && (c.y - p.y).abs() <= 5.
    })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Gesture {
    #[default]
    Idle,
    PendingMove {
        start: Point,
        bounds: Bounds,
    },
    Moving {
        start: Point,
        bounds: Bounds,
        current: Bounds,
    },
    Resizing {
        start: Point,
        bounds: Bounds,
        current: Bounds,
        handle: &'static str,
    },
    Panning {
        start: Point,
        pan: Point,
    },
}
impl Gesture {
    pub fn current(&self) -> Option<Bounds> {
        match self {
            Self::PendingMove { bounds, .. } => Some(*bounds),
            Self::Moving { current, .. } | Self::Resizing { current, .. } => Some(*current),
            _ => None,
        }
    }
    pub fn update(&mut self, p: Point, view: &mut CanvasView) {
        match self.clone() {
            Self::PendingMove { start, bounds } if (p.x - start.x).hypot(p.y - start.y) >= 3. => {
                let d = p.minus(start);
                *self = Self::Moving {
                    start,
                    bounds,
                    current: move_bounds(bounds, Point::new(d.x / view.zoom, d.y / view.zoom)),
                };
            }
            Self::Moving { start, bounds, .. } => {
                let d = p.minus(start);
                *self = Self::Moving {
                    start,
                    bounds,
                    current: move_bounds(bounds, Point::new(d.x / view.zoom, d.y / view.zoom)),
                };
            }
            Self::Resizing {
                start,
                bounds,
                handle,
                ..
            } => {
                let d = p.minus(start);
                *self = Self::Resizing {
                    start,
                    bounds,
                    handle,
                    current: resize_bounds(
                        bounds,
                        handle,
                        Point::new(d.x / view.zoom, d.y / view.zoom),
                    ),
                };
            }
            Self::Panning { start, pan } => {
                let d = p.minus(start);
                view.pan = Point::new(pan.x + d.x, pan.y + d.y);
                view.clamp_pan();
            }
            _ => {}
        }
    }
    pub fn finish(&mut self) -> Option<(Bounds, bool)> {
        match std::mem::take(self) {
            Self::Moving { current, .. } => Some((current, false)),
            Self::Resizing { current, .. } => Some((current, true)),
            _ => None,
        }
    }
}
