use super::{
    geometry::Point,
    view::{History, SlideCanvas},
};
use gpui::Context;
use serde_json::json;
use std::sync::Arc;
pub struct NodeDrag {
    slide: Arc<serde_json::Value>,
    index: usize,
    row: usize,
    control: Option<&'static str>,
    start: Point,
    point: Point,
    changed: bool,
}
impl SlideCanvas {
    pub(super) fn node_down(&mut self, p: Point, cx: &mut Context<Self>) -> bool {
        if self.editor.tab != "object" || self.saving {
            return false;
        }
        let Some(index) = self.selected else {
            return false;
        };
        let layer = &self.slide["layers"][index];
        if layer["locked"] == true || layer["hidden"] == true {
            return false;
        }
        let bounds = super::geometry::layer_bounds(layer, self.view.slide);
        let local = layer["pathCoordinateSpace"] == "local";
        let origin = if local {
            Point::new(bounds.x, bounds.y)
        } else {
            Point::default()
        };
        let Some(points) = layer["pathPoints"].as_array() else {
            return false;
        };
        for (row, point) in points.iter().enumerate() {
            for control in [Some("q0"), Some("q1"), None] {
                let target = control.map_or(point, |key| &point[key]);
                let (Some(x), Some(y)) = (target["x"].as_f64(), target["y"].as_f64()) else {
                    continue;
                };
                let position = self
                    .view
                    .to_view(Point::new(origin.x + x as f32, origin.y + y as f32));
                if (p.x - position.x).abs() <= 5. && (p.y - position.y).abs() <= 5. {
                    self.editor.node_drag = Some(NodeDrag {
                        slide: self.slide.clone(),
                        index,
                        row,
                        control,
                        start: self.view.to_slide(p),
                        point: Point::new(x as f32, y as f32),
                        changed: false,
                    });
                    cx.notify();
                    return true;
                }
            }
        }
        false
    }
    pub(super) fn node_motion(&mut self, p: Point, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.editor.node_drag.as_mut() else {
            return false;
        };
        let delta = self.view.to_slide(p).minus(drag.start);
        let point = Point::new(
            super::geometry::round(drag.point.x + delta.x),
            super::geometry::round(drag.point.y + delta.y),
        );
        let mut slide = (*drag.slide).clone();
        let target = &mut slide["layers"][drag.index]["pathPoints"][drag.row];
        let target = if let Some(key) = drag.control {
            &mut target[key]
        } else {
            target
        };
        target["x"] = json!(point.x);
        target["y"] = json!(point.y);
        drag.changed = point != drag.point;
        self.set_slide(Arc::new(slide));
        self.request_raster(false, cx);
        cx.notify();
        true
    }
    pub(super) fn node_up(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.editor.node_drag.take() else {
            return false;
        };
        if drag.changed {
            if self.editor.base.is_none() {
                self.editor.base = Some(drag.slide);
            }
            self.save((*self.slide).clone(), History::Edit, cx);
        } else if self.editor.base.is_some() {
            self.draft((*self.slide).clone(), cx);
        }
        cx.notify();
        true
    }
    pub(super) fn node_cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(drag) = self.editor.node_drag.take() {
            self.set_slide(drag.slide);
            self.request_raster(false, cx);
        }
    }
}
