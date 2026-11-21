//! Timecode input configuration is owned by the controller host.
use super::view::SettingsView;
use crate::automation::form::{Form, Kind, Spec};
use gpui::{div, prelude::*, Context, Window};
use orbit_ui::{controls::ActionButton, ActiveTheme};
use serde_json::Value;
#[derive(Default)]
pub struct Timecode {
    form: Form,
    loaded: bool,
    devices: Value,
    status: Value,
    error: String,
}
fn form(view: &mut SettingsView) -> &mut Form {
    &mut view.timecode.form
}
impl SettingsView {
    fn refresh_timecode(&mut self, cx: &mut Context<Self>) {
        self.host_call("listTimecodeDevices", vec![], cx, |view, result, cx| {
            match result {
                Ok(v) => view.timecode.devices = v,
                Err(e) => view.timecode.error = e,
            }
            cx.notify();
        });
        self.host_call("getTimecodeStatus", vec![], cx, |view, result, cx| {
            match result {
                Ok(v) => view.timecode.status = v,
                Err(e) => view.timecode.error = e,
            }
            cx.notify();
        });
    }
    pub(super) fn timecode_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        if !self.timecode.loaded {
            self.timecode.loaded = true;
            self.host_call("getTimecodeInputConfig", vec![], cx, |view, result, cx| {
                match result {
                    Ok(v) => view.timecode.form.reset(v),
                    Err(e) => view.timecode.error = e,
                }
                cx.notify();
            });
            self.refresh_timecode(cx);
        }
        if let Some(status) = cx
            .try_global::<crate::automation::timecode::Chase>()
            .map(|s| s.status.clone())
            .filter(Value::is_object)
        {
            self.timecode.status = status;
        }
        let t = crate::state::runtime::Runtime::translator(cx);
        let mut panel = div()
            .flex()
            .flex_col()
            .gap(cx.theme().metrics.space_xs)
            .child(
                div()
                    .text_size(cx.theme().metrics.font_md)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(cx.theme().colors.text_strong)
                    .child(t.t("automation.timecode.title")),
            );
        let sources = ["none", "ltc", "mtc"]
            .iter()
            .map(|s| {
                (
                    s.to_string(),
                    t.t(&format!("automation.timecode.source.{s}")),
                )
            })
            .collect();
        let audio = crate::services::entries(&self.timecode.devices["audioInputs"])
            .iter()
            .filter_map(|d| d["name"].as_str().map(|s| (s.to_string(), s.to_string())))
            .collect();
        let midi = self.timecode.devices["midiInputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d.as_str().map(|s| (s.to_string(), s.to_string())))
            .collect();
        let mut playlists = vec![(
            String::new(),
            t.t("automation.timecode.selectedPresentation"),
        )];
        playlists.extend(crate::services::choices(
            &self.runtime.content.read(cx).workspace["playlist"],
            "name",
        ));
        for spec in [
            Spec::new(
                "source",
                t.t("automation.timecode.source"),
                Kind::Choice(sources),
            ),
            Spec::new(
                "audioDevice",
                t.t("automation.timecode.audioDevice"),
                Kind::Choice(audio),
            ),
            Spec::new(
                "audioChannel",
                t.t("automation.timecode.audioChannel"),
                Kind::Number,
            ),
            Spec::new(
                "midiPort",
                t.t("automation.timecode.midiPort"),
                Kind::Choice(midi),
            ),
            Spec::new(
                "targetPlaylistId",
                t.t("automation.timecode.playlist"),
                Kind::Choice(playlists),
            ),
            Spec::new(
                "format",
                t.t("automation.timecode.format"),
                Kind::Choice(
                    ["auto", "24", "25", "29.97df", "29.97ndf", "30"]
                        .iter()
                        .map(|s| (s.to_string(), s.to_string()))
                        .collect(),
                ),
            ),
            Spec::new(
                "offsetSeconds",
                t.t("automation.timecode.inputOffset"),
                Kind::Number,
            ),
        ] {
            panel =
                panel.child(
                    self.timecode
                        .form
                        .field("timecode-settings", &spec, form, window, cx),
                );
        }
        panel
            .child(
                super::ui::action_row(cx)
                    .child(
                        ActionButton::new("timecode-refresh", t.t("automation.timecode.refresh"))
                            .on_click(cx.listener(|view, _, _, cx| view.refresh_timecode(cx))),
                    )
                    .child(
                        ActionButton::new("timecode-apply", t.t("automation.timecode.apply"))
                            .on_click(cx.listener(|view, _, _, cx| {
                                let config = view.timecode.form.draft.clone();
                                view.host_call(
                                    "setTimecodeInputConfig",
                                    vec![config],
                                    cx,
                                    |view, result, cx| {
                                        match result {
                                            Ok(v) => {
                                                view.timecode.status = v;
                                                view.timecode.error.clear();
                                            }
                                            Err(e) => view.timecode.error = e,
                                        }
                                        cx.notify();
                                    },
                                );
                            })),
                    ),
            )
            .child(
                self.timecode.status["error"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            )
            .child(
                self.timecode.status["label"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            )
            .child(self.timecode.error.clone())
    }
}
