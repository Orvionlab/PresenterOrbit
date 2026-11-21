use super::*;
use gpui::{Pixels, Point, ScrollStrategy};
use std::time::Instant;

const LONG_PRESS: Duration = Duration::from_millis(450);

pub(super) struct GraphicsPress {
    pub id: String,
    library: String,
    pub audio: bool,
    workspace: String,
    started: Instant,
    pub active: bool,
    pub position: Point<Pixels>,
}
/// The host-confirmed lists while optimistic reorders are on screen, the baseline their saves merge against.
pub(super) struct Reorder {
    pub media: Vec<Value>,
    pub libraries: Vec<Value>,
}
impl MediaBin {
    /// Audio reorders start with the native drag; graphics retain their hold gesture.
    pub(super) fn graphics_begin(
        &mut self,
        id: String,
        position: Point<Pixels>,
        active: bool,
        audio: bool,
        cx: &mut Context<Self>,
    ) {
        if (self.busy && self.reorder.is_none()) || self.workspace.is_empty() {
            return;
        }
        if active {
            if let Some(press) = self.graphics_press.as_mut().filter(|press| press.id == id) {
                if audio || press.started.elapsed() >= LONG_PRESS {
                    press.active = true;
                    self.suppress_click = true;
                    cx.notify();
                }
            }
            return;
        }
        self.suppress_click = false;
        self.graphics_press = Some(GraphicsPress {
            id,
            library: self.library_for(audio).clone(),
            audio,
            workspace: self.workspace.clone(),
            started: Instant::now(),
            active: false,
            position,
        });
        cx.notify();
    }
    pub(super) fn graphics_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(press) = self.graphics_press.as_mut() else {
            return;
        };
        let library = if press.audio {
            &self.audio_library
        } else {
            &self.library
        };
        if &press.library != library || press.workspace != self.workspace {
            self.graphics_cancel(cx);
            return;
        }
        press.position = position;
        if !press.audio && !press.active && press.started.elapsed() >= LONG_PRESS {
            press.active = true;
            self.suppress_click = true;
        }
        if !press.active {
            return;
        }
        let audio = press.audio;
        let library = press.library.clone();
        let rows = if audio { &self.audio_rows } else { &self.rows };
        let bounds = if audio {
            &self.audio_card_bounds
        } else {
            &self.card_bounds
        }
        .borrow();
        let tiles = rows
            .iter()
            .enumerate()
            .filter(|(_, item)| item["type"] != "background")
            .filter_map(|(index, item)| {
                bounds.get(text(item, "id")).map(|bounds| (index, *bounds))
            });
        let Some(gap) = crate::gap::gap_at(position, tiles, audio || !self.grid) else {
            return;
        };
        let count = rows
            .iter()
            .filter(|item| item["type"] != "background")
            .count();
        let (target, after) = if gap >= count {
            (count - 1, true)
        } else {
            (gap, false)
        };
        let target = text(&rows[target], "id").to_string();
        self.graphics_drop = Some((target, after, library));
        cx.notify();
    }
    pub(super) fn graphics_tick(&mut self, cx: &mut Context<Self>) {
        let Some(position) = self.graphics_press.as_ref().map(|press| press.position) else {
            return;
        };
        self.graphics_move(position, cx);
        if !self
            .graphics_press
            .as_ref()
            .is_some_and(|press| press.active)
        {
            return;
        }
        let audio = self
            .graphics_press
            .as_ref()
            .is_some_and(|press| press.audio);
        let scroll = if audio {
            &self.audio_scroll
        } else {
            &self.grid_scroll
        };
        let Some(bounds) = *(if audio {
            &self.audio_grid_bounds
        } else {
            &self.grid_bounds
        })
        .borrow() else {
            return;
        };
        let zone = cx.theme().metrics.control_height;
        let top = scroll.0.borrow().base_handle.logical_scroll_top().0;
        let columns = if !audio && self.grid {
            self.grid_columns
        } else {
            1
        };
        let last = if audio {
            self.audio_visible.len()
        } else {
            self.visible.len()
        }
        .div_ceil(columns)
        .saturating_sub(1);
        if position.y < bounds.top() + zone && position.y >= bounds.top() {
            scroll.scroll_to_item(top.saturating_sub(1), ScrollStrategy::Top);
            cx.notify();
        } else if position.y > bounds.bottom() - zone && position.y <= bounds.bottom() {
            scroll.scroll_to_item((top + 1).min(last), ScrollStrategy::Top);
            cx.notify();
        }
    }
    pub(super) fn graphics_cancel(&mut self, cx: &mut Context<Self>) {
        self.suppress_click |= self
            .graphics_press
            .as_ref()
            .is_some_and(|press| press.active);
        self.graphics_press = None;
        self.graphics_drop = None;
        cx.notify();
    }
    pub(super) fn graphics_finish(&mut self, cx: &mut Context<Self>) {
        let Some(press) = self.graphics_press.take() else {
            return;
        };
        let target = self.graphics_drop.take();
        if !press.active
            || &press.library != self.library_for(press.audio)
            || press.workspace != self.workspace
        {
            cx.notify();
            return;
        }
        self.suppress_click = true;
        if let Some((target, after, library)) =
            target.filter(|(_, _, library)| library == self.library_for(press.audio))
        {
            if let Some((media, libraries)) = model::reorder_media(
                &self.media,
                &self.libraries,
                &library,
                press.audio,
                &press.id,
                &target,
                after,
            ) {
                if media != self.media || libraries != self.libraries {
                    self.reorder.get_or_insert_with(|| Reorder {
                        media: self.media.clone(),
                        libraries: self.libraries.clone(),
                    });
                    self.media = media;
                    self.libraries = libraries;
                    self.rebuild();
                    self.flush_reorder(cx);
                }
            }
        }
        cx.notify();
    }
    /// Sends the next optimistic list that differs from the host's; with none left the host has caught up.
    pub(super) fn flush_reorder(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(base) = self.reorder.as_ref() else {
            return;
        };
        if self.media != base.media {
            let media = json!(self.media);
            self.mutate_saved("saveMediaSettings", vec![media], cx);
        } else if self.libraries != base.libraries {
            let libraries = json!(self.libraries);
            self.mutate_saved("saveMediaLibraries", vec![libraries], cx);
        } else {
            self.reorder = None;
            self.source = Arc::new(Value::Null);
            self.refresh(cx);
        }
    }
}
