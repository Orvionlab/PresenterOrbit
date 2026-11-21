//! `ScreenOptionsPage.tsx` and `updateRenderingBitDepth` of `settings-panel-controller.ts`: the screen settings
//! sub-section of the Video panel. Auto-enable at launch is applied when the first workspace loads
//! (`output::windows::start_disabled`, in memory only), ignoring background colors by the scene, and the bit depth by the
//! output process at its start, so a changed depth offers a restart.

use gpui::{div, prelude::*, AnyElement, Context, SharedString};
use orbit_ui::controls::{ActionButton, ButtonVariant, Dialog, SelectOption};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::json;

use super::rows::{column, group, select_row, toggle_row};
use super::ui::heading;
use super::view::SettingsView;

#[derive(Default)]
pub struct ScreenOptions {
    /// The bit depth changed and the restart question is open.
    pub restart_prompt: bool,
}

/// Starts a new instance with the arguments of this one and ends this one. The new instance may start while this
/// one is still closing; nothing it needs is exclusive except the Network Link port, which it reports and
/// retries on its own.
pub fn restart_application(cx: &mut gpui::App) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    std::process::Command::new(executable)
        .args(std::env::args_os().skip(1))
        .spawn()
        .map_err(|error| error.to_string())?;
    cx.quit();
    Ok(())
}

impl SettingsView {
    pub(super) fn set_bit_depth(&mut self, value: String, cx: &mut Context<Self>) {
        self.patch_general(json!({ "renderingBitDepth": value }), cx);
        self.screen_options.restart_prompt = true;
        cx.notify();
    }

    fn restart_dialog(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let gap = cx.theme().metrics.space_xs;
        let weak = cx.weak_entity();
        let later = move |_: (), _: &mut gpui::Window, cx: &mut gpui::App| {
            weak.update(cx, |view, cx| {
                view.screen_options.restart_prompt = false;
                cx.notify();
            })
            .ok();
        };
        let dismiss = later.clone();
        Dialog::new(
            "restart-prompt",
            t.t("settings.screenOptions.title"),
            t.t("common.close"),
            dismiss,
        )
        .width(gpui::px(420.))
        .close_on_backdrop(false)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(gap)
                .child(div().child(t.t("settings.general.bitDepthRestart")))
                .child(
                    div()
                        .flex()
                        .gap(gap)
                        .child(
                            ActionButton::new("restart-now", t.t("settings.general.restartNow"))
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.screen_options.restart_prompt = false;
                                    if let Err(error) = restart_application(cx) {
                                        view.fail(error, cx);
                                    }
                                })),
                        )
                        .child(
                            ActionButton::new(
                                "restart-later",
                                t.t("settings.general.restartLater"),
                            )
                            .variant(ButtonVariant::Secondary)
                            .on_click(move |_, window, cx| later((), window, cx)),
                        ),
                ),
        )
        .into_any_element()
    }

    pub(super) fn screen_options_section(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let general = self.settings(cx)["generalSettings"].clone();
        let toggle = |id: &'static str,
                      key: &'static str,
                      label: &str,
                      description: &str,
                      default: bool,
                      cx: &mut Context<Self>| {
            let weak = cx.weak_entity();
            toggle_row(
                id,
                t.t(label),
                Some(t.t(description).into()),
                general[key].as_bool().unwrap_or(default),
                move |value, _, cx| {
                    weak.update(cx, |view, cx| view.patch_general(json!({ key: value }), cx))
                        .ok();
                },
                cx,
            )
        };
        let options = group(
            [
                toggle(
                    "screen-options-auto-enable",
                    "autoEnableScreensAtLaunch",
                    "settings.screenOptions.autoEnable",
                    "settings.screenOptions.autoEnableDescription",
                    true,
                    cx,
                ),
                toggle(
                    "screen-options-ignore-colors",
                    "ignoreBackgroundColors",
                    "settings.screenOptions.ignoreBackgroundColors",
                    "settings.screenOptions.ignoreBackgroundColorsDescription",
                    false,
                    cx,
                ),
            ],
            cx,
        );
        let weak = cx.weak_entity();
        let rendering = group(
            [select_row(
                "screen-options-bit-depth",
                t.t("settings.general.bitDepth"),
                vec![
                    SelectOption::new("normal".to_string(), t.t("settings.general.bitDepthNormal")),
                    SelectOption::new(
                        "enhanced".to_string(),
                        t.t("settings.general.bitDepthEnhanced"),
                    ),
                ],
                general["renderingBitDepth"].as_str().map(str::to_string),
                move |value, _, cx| {
                    weak.update(cx, |view, cx| view.set_bit_depth(value, cx))
                        .ok();
                },
                cx,
            )],
            cx,
        );
        let prompt = self
            .screen_options
            .restart_prompt
            .then(|| self.restart_dialog(cx));
        column(cx)
            .id(SharedString::from("settings-screen-options"))
            .child(options)
            .child(heading(t.t("settings.screenOptions.rendering"), cx))
            .child(rendering)
            .children(prompt)
            .into_any_element()
    }
}
