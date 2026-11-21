//! The program audio level bars on the left edge of the preview: one bar per main-output channel.
//! A specialist surface, so the stored React colors are mirrored here (`--orbit-output-meter-*`).

use std::time::Duration;

use gpui::{div, prelude::*, px, relative, rgb, rgba, Context, Rgba, Window};
use orbit_ui::ActiveTheme;
use serde_json::Value;

use crate::settings::audio_logic::get_audio_settings;
use crate::state::runtime::{output_call, Runtime};

const POLL: Duration = Duration::from_millis(50);
const HEIGHT: f32 = 126.;
const BAR_WIDTH: f32 = 5.;
/// The bar is green up to 64 %, yellow up to 84 %, then red.
const SEGMENTS: [(f32, f32); 3] = [(0., 0.64), (0.64, 0.84), (0.84, 1.)];
const DARK: [u32; 3] = [0x10290c, 0x28250a, 0x2f0b0b];
const YELLOW: u32 = 0xc7c627;

/// Bars follow the channel count of the main output routing.
pub fn bar_count(settings: &Value) -> usize {
    get_audio_settings(settings)["main"]["routing"]["deviceChannels"]
        .as_f64()
        .map_or(2, |count| count as usize)
}

/// Fill height of a bar from the host percent: the main level is added in dB like React's `getAnalyserLevel`
/// (60 dB spans the bar), except for silence, which stays empty.
pub fn bar_fill(percent: f32, gain_db: f32) -> f32 {
    if percent <= 0. {
        return 0.;
    }
    (percent + gain_db * 100. / 60.).clamp(0., 100.) / 100.
}

pub fn main_gain_db(settings: &Value) -> f32 {
    get_audio_settings(settings)["main"]["level"]
        .as_f64()
        .unwrap_or(0.) as f32
}

pub struct ProgramMeters {
    levels: Vec<f32>,
    bars: usize,
    gain_db: f32,
}

impl ProgramMeters {
    pub fn new(runtime: &Runtime, cx: &mut Context<Self>) -> Self {
        let host = runtime.host.clone();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(POLL).await;
            let host = host.clone();
            let levels = cx
                .background_spawn(
                    async move { output_call(&host, "getProgramAudioLevels", vec![]) },
                )
                .await;
            let alive = this.update(cx, |this, cx| {
                let levels = levels
                    .ok()
                    .and_then(|value| serde_json::from_value::<Vec<f32>>(value).ok())
                    .unwrap_or_default();
                if levels != this.levels {
                    this.levels = levels;
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
        Self {
            levels: Vec::new(),
            bars: 2,
            gain_db: 0.,
        }
    }

    pub fn set_layout(&mut self, settings: &Value, cx: &mut Context<Self>) {
        let (bars, gain_db) = (bar_count(settings), main_gain_db(settings));
        if (bars, gain_db) != (self.bars, self.gain_db) {
            (self.bars, self.gain_db) = (bars, gain_db);
            cx.notify();
        }
    }
}

fn bar(fill: f32, success: Rgba, danger: Rgba) -> impl IntoElement {
    let lit = [success, rgb(YELLOW), danger];
    let mut column = div()
        .flex_none()
        .flex()
        .flex_col_reverse()
        .w(px(BAR_WIDTH))
        .h_full()
        .overflow_hidden();
    for (index, (start, end)) in SEGMENTS.into_iter().enumerate() {
        let share = ((fill - start) / (end - start)).clamp(0., 1.);
        column = column.child(
            div()
                .flex_none()
                .relative()
                .w_full()
                .h(relative(end - start))
                .bg(rgb(DARK[index]))
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w_full()
                        .h(relative(share))
                        .bg(lit[index]),
                ),
        );
    }
    column
}

impl gpui::Render for ProgramMeters {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(4.))
            .flex()
            .items_center()
            .child(
                div()
                    .debug_selector(|| "program-audio-meters".into())
                    .flex()
                    .gap(px(2.))
                    .h(px(HEIGHT))
                    .px(px(2.))
                    .pt(px(4.))
                    .pb(px(2.))
                    .bg(rgba(0x000000b8))
                    .children((0..self.bars).map(|channel| {
                        let percent = self.levels.get(channel).copied().unwrap_or(0.);
                        bar(
                            bar_fill(percent, self.gain_db),
                            colors.success,
                            colors.danger,
                        )
                    })),
            )
    }
}
