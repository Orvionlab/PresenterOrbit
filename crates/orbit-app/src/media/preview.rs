use super::preview_timeline::TrimRange;
use super::*;

enum PreviewJump {
    Start,
    Offset(f64),
    End,
}

impl MediaBin {
    pub(super) fn preview_duration(&self) -> Option<f64> {
        self.details
            .get(&self.selected)
            .and_then(|details| details.metadata["durationSeconds"].as_f64())
            .filter(|duration| duration.is_finite() && *duration > 0.)
    }

    pub(super) fn preview_frame_time(&self) -> f64 {
        let Some(duration) = self.preview_duration() else {
            return self.preview_position;
        };
        if self.preview_position < duration {
            return self.preview_position;
        }
        let fps = self
            .details
            .get(&self.selected)
            .and_then(|details| details.metadata["fps"].as_f64());
        (duration - fps.map_or(0.01, |fps| 1. / fps)).max(0.)
    }

    pub(super) fn preview_range(&self) -> Option<TrimRange> {
        let duration = self.preview_duration()?;
        let item = self.media.iter().find(|item| item["id"] == self.selected)?;
        Some(TrimRange::from_asset(item, duration))
    }

    fn preview_jump(&mut self, jump: &PreviewJump, cx: &mut Context<Self>) {
        let Some(range) = self.preview_range() else {
            return;
        };
        let target = match jump {
            PreviewJump::Start => range.start,
            PreviewJump::End => range.end,
            PreviewJump::Offset(seconds) => {
                (self.preview_position + seconds).clamp(range.start, range.end)
            }
        };
        self.preview_seek(target, cx);
    }

    pub(super) fn preview_seek(&mut self, seconds: f64, cx: &mut Context<Self>) {
        if !seconds.is_finite() || seconds < 0. {
            return;
        }
        let mut seconds = self
            .preview_duration()
            .map_or(seconds, |duration| seconds.min(duration));
        if let Some(range) = self.preview_range().filter(|_| self.preview_playing) {
            seconds = seconds.clamp(range.start, range.end);
            if seconds >= range.end {
                self.audition(false, cx);
            }
        }
        self.preview_position = seconds;
        self.preview_epoch = std::time::Instant::now();
        self.preview_offset = seconds;
        if self.preview_playing {
            self.audition(true, cx);
        }
        cx.notify();
    }

    fn preview_toggle(&mut self, cx: &mut Context<Self>) {
        if self.preview_playing {
            self.audition(false, cx);
        } else {
            if let Some(range) = self.preview_range().filter(|range| {
                self.preview_position < range.start || self.preview_position >= range.end
            }) {
                self.preview_position = range.start;
            }
            self.preview_playing = true;
            self.preview_offset = self.preview_position;
            self.preview_epoch = std::time::Instant::now();
            self.audition(true, cx);
        }
        cx.notify();
    }

    pub(super) fn preview_tick(&mut self, cx: &mut Context<Self>) {
        if !self.preview_playing {
            return;
        }
        let position = self.preview_offset + self.preview_epoch.elapsed().as_secs_f64();
        if let Some(range) = self.preview_range().filter(|range| position >= range.end) {
            self.preview_position = range.end;
            self.audition(false, cx);
        } else {
            self.preview_position = position;
        }
        cx.notify();
    }

    pub(super) fn preview_controls(
        &mut self,
        item: &Value,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let duration = self.preview_duration();
        let range = duration.map(|duration| TrimRange::from_asset(item, duration));
        let clock = crate::automation::form::format_time;
        let remaining = range
            .map(|range| format!("-{}", clock((range.end - self.preview_position).max(0.))))
            .unwrap_or_else(|| t.t("common.unknown"));
        let icon_button = |id, icon, label, jump| {
            IconButton::new(id, icon, t.t(label))
                .disabled(range.is_none())
                .on_click(cx.listener(move |this, _, _, cx| this.preview_jump(&jump, cx)))
        };
        let commands = div()
            .flex()
            .flex_none()
            .items_center()
            .child(icon_button(
                "asset-preview-start",
                "media-jump-start",
                "media.preview.start",
                PreviewJump::Start,
            ))
            .child(icon_button(
                "asset-preview-back",
                "media-step-back",
                "media.preview.back",
                PreviewJump::Offset(-1.),
            ))
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
            .child(icon_button(
                "asset-preview-back-15",
                "media-rewind-15",
                "media.preview.rewind15",
                PreviewJump::Offset(-15.),
            ))
            .child(icon_button(
                "asset-preview-forward-15",
                "media-forward-15",
                "media.preview.forward15",
                PreviewJump::Offset(15.),
            ))
            .child(icon_button(
                "asset-preview-forward",
                "media-step-forward",
                "media.preview.forward",
                PreviewJump::Offset(1.),
            ))
            .child(icon_button(
                "asset-preview-end",
                "media-jump-end",
                "media.preview.end",
                PreviewJump::End,
            ));
        let toolbar = div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .justify_center()
            .gap(theme.metrics.space_xs)
            .border_t_1()
            .border_color(theme.colors.border_soft)
            .pt(theme.metrics.space_xs)
            .child(div().flex_1().min_w_0())
            .child(commands)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_right()
                    .text_size(theme.metrics.font_sm)
                    .text_color(theme.colors.text_muted)
                    .child(format!("{} / {remaining}", clock(self.preview_position))),
            );
        let timeline = duration.map(|duration| self.preview_timeline(item, duration, cx));
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(theme.metrics.space_xs)
            .children(timeline)
            .child(toolbar)
            .into_any_element()
    }
}
