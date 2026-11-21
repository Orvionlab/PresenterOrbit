use super::{
    frame_dimensions_allowed, json_number, layer_pixel_bounds, scene_layer_uses_backdrop,
    RenderTarget, SceneLayer, SceneLayerKind, SceneSlide,
};
use crate::native_transitions::{bilinear, write_pixel};
use serde_json::{json, Value};

pub(super) struct Transform {
    center: (f64, f64),
    sin: f64,
    cos: f64,
    x_sign: f64,
    y_sign: f64,
    aspect: f64,
}

impl Transform {
    pub(super) fn new(
        output: &RenderTarget,
        slide: &SceneSlide,
        layer: &SceneLayer,
    ) -> Option<Self> {
        let number = |key| json_number(layer.source.get(key)).filter(|value| value.is_finite());
        let angle = number("rotation").unwrap_or(0.).rem_euclid(360.);
        let flipped = |key| layer.source.get(key).and_then(Value::as_bool) == Some(true);
        let (flip_x, flip_y) = (flipped("flipX"), flipped("flipY"));
        if angle == 0. && !flip_x && !flip_y {
            return None;
        }
        let [x, y, width, height] = object_bounds(slide, layer);
        let sx = output.width as f64 / slide.width;
        let sy = output.height as f64 / slide.height;
        let center = ((x + width / 2.) * sx, (y + height / 2.) * sy);
        let (sin, cos) = angle.to_radians().sin_cos();
        Some(Self {
            center,
            sin,
            cos,
            x_sign: if flip_x { -1. } else { 1. },
            y_sign: if flip_y { -1. } else { 1. },
            aspect: sx / sy,
        })
    }

    fn forward(&self, x: f64, y: f64) -> (f64, f64) {
        let x = (x - self.center.0) * self.x_sign;
        let y = (y - self.center.1) * self.y_sign;
        (
            self.center.0 + x * self.cos - y * self.sin * self.aspect,
            self.center.1 + x * self.sin / self.aspect + y * self.cos,
        )
    }

    fn inverse(&self, x: f64, y: f64) -> (f64, f64) {
        let x = x - self.center.0;
        let y = y - self.center.1;
        (
            self.center.0 + (x * self.cos + y * self.sin * self.aspect) * self.x_sign,
            self.center.1 + (-x * self.sin / self.aspect + y * self.cos) * self.y_sign,
        )
    }

    pub(super) fn draw(
        &self,
        output: &mut RenderTarget,
        slide: &SceneSlide,
        layer: &SceneLayer,
        draw: impl FnOnce(&mut RenderTarget<'_>, &SceneSlide, &SceneLayer) -> bool,
    ) -> bool {
        let (width, height) = (output.width, output.height);
        let region = self.source_region(width, height);
        let (rw, rh) = (region.width, region.height);
        let mut pixels = vec![0; rw * rh * 4];
        let backdrop = scene_layer_uses_backdrop(slide, layer);
        // Backdrop fills sample the backdrop in the object's untransformed coordinate space.
        let original = if backdrop {
            for y in 0..rh {
                for x in 0..rw {
                    let (tx, ty) =
                        self.forward(x as f64 + region.x + 0.5, y as f64 + region.y + 0.5);
                    if let Some(source) = pixel_offset(width, height, tx, ty) {
                        let offset = (y * rw + x) * 4;
                        pixels[offset..offset + 4]
                            .copy_from_slice(&output.pixels[source..source + 4]);
                    }
                }
            }
            Some(pixels.clone())
        } else {
            None
        };
        let mut scratch = RenderTarget {
            pixels: &mut pixels,
            width: rw,
            height: rh,
            dynamic_media: output.dynamic_media,
            scroll_elapsed_seconds: output.scroll_elapsed_seconds,
        };
        let rendered_text = if region.x == 0. && region.y == 0. && rw == width && rh == height {
            draw(&mut scratch, slide, layer)
        } else {
            let (slide, layer) = region.rebase(slide, layer, width, height);
            draw(&mut scratch, &slide, &layer)
        };
        output.dynamic_media |= scratch.dynamic_media;
        let Some(bounds) = layer_pixel_bounds(&pixels, rw, rh, backdrop) else {
            return rendered_text;
        };
        let mut left = width;
        let mut top = height;
        let (mut right, mut bottom) = (0, 0);
        for x in [bounds.x as f64 - 1., (bounds.x + bounds.width) as f64 + 1.] {
            for y in [bounds.y as f64 - 1., (bounds.y + bounds.height) as f64 + 1.] {
                let (x, y) = self.forward(x + region.x, y + region.y);
                left = left.min(x.floor() as usize);
                top = top.min(y.floor() as usize);
                right = right.max(x.ceil() as usize);
                bottom = bottom.max(y.ceil() as usize);
            }
        }
        for y in top..bottom.min(height) {
            for x in left..right.min(width) {
                let (sx, sy) = self.inverse(x as f64 + 0.5, y as f64 + 0.5);
                let (sx, sy) = (sx - region.x, sy - region.y);
                let offset = (y * width + x) * 4;
                let destination = &mut output.pixels[offset..offset + 4];
                if let Some(original) = &original {
                    if let Some(source) = pixel_offset(rw, rh, sx, sy) {
                        let result = &pixels[source..source + 4];
                        if result != &original[source..source + 4] {
                            destination.copy_from_slice(result);
                        }
                    }
                } else {
                    let color = bilinear(&pixels, rw, rh, sx - 0.5, sy - 0.5);
                    if color[3] > 0. {
                        let mut source = [0; 4];
                        write_pixel(&mut source, color);
                        super::blend_pixel(output.pixels, width, x, y, source, 1.);
                    }
                }
            }
        }
        rendered_text
    }

    fn source_region(&self, width: usize, height: usize) -> SourceRegion {
        let mut left: f64 = f64::INFINITY;
        let mut top = left;
        let mut right = f64::NEG_INFINITY;
        let mut bottom = right;
        for x in [0.5, width as f64 - 0.5] {
            for y in [0.5, height as f64 - 0.5] {
                let (x, y) = self.inverse(x, y);
                left = left.min((x - 0.5).floor());
                top = top.min((y - 0.5).floor());
                right = right.max((x + 0.5).ceil());
                bottom = bottom.max((y + 0.5).ceil());
            }
        }
        let region = SourceRegion {
            x: left,
            y: top,
            width: (right - left) as usize,
            height: (bottom - top) as usize,
        };
        if frame_dimensions_allowed(region.width as u64, region.height as u64) {
            region
        } else {
            SourceRegion {
                x: 0.,
                y: 0.,
                width,
                height,
            }
        }
    }
}

struct SourceRegion {
    x: f64,
    y: f64,
    width: usize,
    height: usize,
}

impl SourceRegion {
    // Rasterize the inverse viewport at the original scale so pixels outside the
    // slide can still rotate into view. Rebase absolute coordinates to its origin.
    fn rebase(
        &self,
        slide: &SceneSlide,
        layer: &SceneLayer,
        width: usize,
        height: usize,
    ) -> (SceneSlide, SceneLayer) {
        let (dx, dy) = (
            self.x * slide.width / width as f64,
            self.y * slide.height / height as f64,
        );
        let [x, y, w, h] = object_bounds(slide, layer);
        let mut layer = layer.clone();
        let bounds = json!({"x": x - dx, "y": y - dy, "width": w, "height": h});
        layer.source.insert("bounds".into(), bounds);
        if layer
            .source
            .get("pathCoordinateSpace")
            .and_then(Value::as_str)
            != Some("local")
        {
            if let Some(points) = layer
                .source
                .get_mut("pathPoints")
                .and_then(Value::as_array_mut)
            {
                for point in points.iter_mut().filter_map(Value::as_object_mut) {
                    shift_point(point, dx, dy);
                    for handle in ["q0", "q1"] {
                        if let Some(handle) = point.get_mut(handle).and_then(Value::as_object_mut) {
                            shift_point(handle, dx, dy);
                        }
                    }
                }
            }
        }
        let mut slide = slide.clone();
        slide.width *= self.width as f64 / width as f64;
        slide.height *= self.height as f64 / height as f64;
        (slide, layer)
    }
}

fn object_bounds(slide: &SceneSlide, layer: &SceneLayer) -> [f64; 4] {
    let defaults = if layer.kind == SceneLayerKind::Text {
        [0.14, 0.22, 0.72, 0.56]
    } else {
        [0., 0., 1., 1.]
    };
    let bounds = layer.source.get("bounds");
    std::array::from_fn(|i| {
        let (key, span) = [
            ("x", slide.width),
            ("y", slide.height),
            ("width", slide.width),
            ("height", slide.height),
        ][i];
        bounds
            .and_then(|bounds| json_number(bounds.get(key)))
            .unwrap_or(defaults[i] * span)
    })
}

fn shift_point(point: &mut serde_json::Map<String, Value>, dx: f64, dy: f64) {
    for (key, delta) in [("x", dx), ("y", dy)] {
        if let Some(coord) = json_number(point.get(key)) {
            point.insert(key.into(), json!(coord - delta));
        }
    }
}

fn pixel_offset(width: usize, height: usize, x: f64, y: f64) -> Option<usize> {
    (x >= 0. && x < width as f64 && y >= 0. && y < height as f64)
        .then(|| (y as usize * width + x as usize) * 4)
}
