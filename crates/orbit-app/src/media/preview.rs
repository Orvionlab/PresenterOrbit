use super::*;
impl MediaBin {
    pub(super) fn preview_seek(&mut self, seconds: f64, cx: &mut Context<Self>) {
        if !seconds.is_finite() || seconds < 0. {
            return;
        }
        self.preview_position = seconds;
        self.preview_epoch = std::time::Instant::now();
        self.preview_offset = seconds;
        if !self.preview_playing {
            cx.notify();
            return;
        }
        let preview = self.preview.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    preview
                        .lock()
                        .map_err(|_| "Preview audio is unavailable.".to_string())?
                        .seek(seconds)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.runtime.status.update(cx, |status, cx| {
                        status.message = error;
                        cx.notify();
                    });
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn preview_toggle(&mut self, cx: &mut Context<Self>) {
        if !self.preview_playing {
            self.preview_playing = true;
            self.preview_offset = self.preview_position;
            self.preview_epoch = std::time::Instant::now();
            self.audition(true, cx);
        } else {
            self.preview_playing = false;
            let preview = self.preview.clone();
            cx.spawn(async move |_, cx| {
                cx.background_spawn(async move {
                    if let Ok(preview) = preview.lock() {
                        preview.pause(true);
                    }
                })
                .await;
            })
            .detach();
        }
        cx.notify();
    }
    pub(super) fn preview_tick(&mut self, cx: &mut Context<Self>) {
        if !self.preview_playing {
            return;
        }
        let position = self.preview_offset + self.preview_epoch.elapsed().as_secs_f64();
        let duration = self
            .details
            .get(&self.selected)
            .and_then(|details| details.metadata["durationSeconds"].as_f64());
        if let Some(duration) = duration.filter(|duration| position >= *duration) {
            self.preview_position = duration;
            self.preview_playing = false;
            self.audition(false, cx);
        } else {
            self.preview_position = position;
        }
        cx.notify();
    }
    pub(super) fn preview_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = *cx.global::<Translator>();
        let duration = self
            .details
            .get(&self.selected)
            .and_then(|details| details.metadata["durationSeconds"].as_f64())
            .unwrap_or(f64::MAX);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .child(
                        IconButton::new(
                            "asset-preview-play",
                            if self.preview_playing {
                                "player-pause"
                            } else {
                                "player-play"
                            },
                            t.t(if self.preview_playing {
                                "automation.macros.pause"
                            } else {
                                "automation.macros.play"
                            }),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.preview_toggle(cx))),
                    )
                    .child(
                        IconButton::new(
                            "asset-preview-stop",
                            "player-stop",
                            t.t("media.stopPreviewAudio"),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.preview_playing = false;
                            this.preview_position = 0.;
                            this.audition(false, cx);
                            cx.notify();
                        })),
                    ),
            )
            .child(self.numeric(
                "previewPosition",
                "media.thumbnail.time",
                self.preview_position,
                (0., duration),
                window,
                cx,
            ))
            .child(format!(
                "{:.1} / {}",
                self.preview_position,
                if duration == f64::MAX {
                    t.t("common.unknown")
                } else {
                    format!("{duration:.1}")
                }
            ))
            .into_any_element()
    }
}
