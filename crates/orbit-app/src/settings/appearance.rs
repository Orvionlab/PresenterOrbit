//! `AppearanceSettingsPanel.tsx`. Theme, accent, text scale, and density are general settings; the host's
//! `settings-changed` event makes every window follow them (`appearance::apply_host_event`).

use super::ui::Field;
use gpui::{div, prelude::*, AnyElement, Context, Entity, Window};
use orbit_ui::controls::{range_state, RangeInput, Select, SelectOption, SliderEvent, SliderState};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::json;

use super::ui::{stack, title};
use super::view::SettingsView;

/// `APP_THEME_OPTIONS`, `APP_ACCENT_OPTIONS`, `TEXT_SCALE_OPTIONS`, `UI_DENSITY_OPTIONS`.
pub const THEMES: [(&str, &str); 4] = [
    ("system", "settings.theme.system"),
    ("dark", "settings.theme.dark"),
    ("light", "settings.theme.light"),
    ("red", "settings.theme.red"),
];
pub const ACCENTS: [(&str, &str); 4] = [
    ("blue", "settings.accent.blue"),
    ("red", "settings.accent.red"),
    ("purple", "settings.accent.purple"),
    ("green", "settings.accent.green"),
];
pub const TEXT_SCALES: [(&str, &str); 4] = [
    ("small", "settings.textScale.small"),
    ("default", "settings.textScale.default"),
    ("large", "settings.textScale.large"),
    ("xlarge", "settings.textScale.xlarge"),
];
pub const DENSITIES: [(&str, &str); 3] = [
    ("compact", "settings.uiDensity.compact"),
    ("comfortable", "settings.uiDensity.comfortable"),
    ("spacious", "settings.uiDensity.spacious"),
];

/// The two option sliders, created with the panel; each stop is one choice, as in `AppearanceOptionSlider`.
pub struct Sliders {
    pub(super) text_scale: Entity<SliderState>,
    pub(super) density: Entity<SliderState>,
}

impl SettingsView {
    fn option_slider(
        &mut self,
        key: &'static str,
        table: &'static [(&'static str, &'static str)],
        current: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let index = table.iter().position(|(id, _)| *id == current).unwrap_or(0);
        let settings = self.settings(cx);
        let initial = |key: &str, table: &[(&str, &str)]| {
            let value = settings["generalSettings"][key].as_str().unwrap_or("");
            table.iter().position(|(id, _)| *id == value).unwrap_or(0)
        };
        let (text_scale_at, density_at) = (
            initial("textScale", &TEXT_SCALES),
            initial("uiDensity", &DENSITIES),
        );
        let sliders = self.appearance.get_or_insert_with(|| {
            let make = |key: &'static str,
                        table: &'static [(&str, &str)],
                        at: usize,
                        cx: &mut Context<Self>| {
                let state = range_state(0., (table.len() - 1) as f32, 1., at as f32, cx);
                // The subscriber is already leased while it runs, so it must use `this`, not a weak update.
                cx.subscribe(&state, move |this, _, event: &SliderEvent, cx| {
                    let SliderEvent::Change(value) = event else {
                        return;
                    };
                    let id = table[(value.end().round() as usize).min(table.len() - 1)].0;
                    this.patch_general(json!({ key: id }), cx);
                })
                .detach();
                state
            };
            Sliders {
                text_scale: make("textScale", &TEXT_SCALES, text_scale_at, cx),
                density: make("uiDensity", &DENSITIES, density_at, cx),
            }
        });
        let state = if key == "textScale" {
            sliders.text_scale.clone()
        } else {
            sliders.density.clone()
        };
        if state.read(cx).value().end().round() as usize != index {
            state.update(cx, |state, cx| state.set_value(index as f32, window, cx));
        }
        let label = t.t(if key == "textScale" {
            "settings.general.textScale"
        } else {
            "settings.general.uiDensity"
        });
        Field::new(
            label.clone(),
            div()
                .flex()
                .flex_col()
                .child(RangeInput::new(&state).label(label))
                .child(stop_labels(table, theme, t)),
        )
        .into_any_element()
    }

    pub(super) fn appearance_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let settings = self.settings(cx);
        let general = &settings["generalSettings"];
        let current = |key: &str| general[key].as_str().unwrap_or("").to_string();
        let choices = |table: &[(&str, &str)]| -> Vec<SelectOption<String>> {
            table
                .iter()
                .map(|(id, label)| SelectOption::new(id.to_string(), t.t(label)))
                .collect()
        };
        let setter = |key: &'static str, cx: &mut Context<Self>| {
            let weak = cx.weak_entity();
            move |value: String, _: &mut gpui::Window, cx: &mut gpui::App| {
                weak.update(cx, |view, cx| view.patch_general(json!({ key: value }), cx))
                    .ok();
            }
        };
        stack(cx)
            .id("settings-appearance")
            .child(title(t.t("settings.appearance.title"), cx))
            .child(Field::new(
                t.t("settings.appearance.theme"),
                Select::new("appearance-theme", choices(&THEMES), current("appTheme"))
                    .on_change(setter("appTheme", cx)),
            ))
            .child(Field::new(
                t.t("settings.appearance.accent"),
                Select::new("appearance-accent", choices(&ACCENTS), current("appAccent"))
                    .on_change(setter("appAccent", cx)),
            ))
            .child(self.option_slider("textScale", &TEXT_SCALES, &current("textScale"), window, cx))
            .child(self.option_slider("uiDensity", &DENSITIES, &current("uiDensity"), window, cx))
            .into_any_element()
    }
}

/// Labels sit at their stop's x on the full-width track: the first at the left end, the last at the right end,
/// the others centred on a zero-width anchor at `i / (n - 1)`.
fn stop_labels(table: &[(&str, &str)], theme: orbit_ui::Theme, t: Translator) -> impl IntoElement {
    let last = table.len() - 1;
    div()
        .relative()
        .h(theme.metrics.font_xs * 1.5)
        .text_size(theme.metrics.font_xs)
        .font_weight(gpui::FontWeight::NORMAL)
        .text_color(theme.colors.text_muted)
        .whitespace_nowrap()
        .children(table.iter().enumerate().map(|(i, (_, text))| {
            let anchor = div().absolute().top_0();
            match i {
                0 => anchor.left_0().child(t.t(text)),
                i if i == last => anchor.right_0().child(t.t(text)),
                i => anchor
                    .left(gpui::relative(i as f32 / last as f32))
                    .w_0()
                    .flex()
                    .justify_center()
                    .child(t.t(text)),
            }
        }))
}
