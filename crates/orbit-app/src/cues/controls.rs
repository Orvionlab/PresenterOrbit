use super::actions;
use crate::state::runtime::Runtime;
use gpui::{div, prelude::*, AnyElement, App, SharedString};
use orbit_ui::controls::tooltip_text;
use orbit_ui::controls::NativeButton as Button;
use orbit_ui::{icon, ActiveTheme, Translator};

pub fn layer_rail(runtime: &Runtime, cx: &mut App) -> AnyElement {
    let t = *cx.global::<Translator>();
    let theme = *cx.theme();
    let output = runtime.output.read(cx);
    let screen = output.snapshot.as_ref().and_then(|snapshot| {
        crate::output::view::selected_screen(
            &crate::output::view::screens(snapshot),
            &output.selected_screen,
        )
        .map(|(_, config)| config.clone())
    });
    let mut rail = div()
        .debug_selector(|| "output-layer-rail".into())
        .flex()
        .flex_col()
        .flex_none()
        .w(theme.metrics.control_height + theme.metrics.space_sm * 2.)
        .h_full()
        .min_h_0()
        .gap(gpui::px(1.))
        .bg(theme.colors.border_strong);
    for (layer, key, glyph) in [
        ("music", "music", "music"),
        ("messages", "message", "send"),
        ("props", "props", "stack-front"),
        ("announcements", "announcement", "speakerphone"),
        ("slide", "slide", "notes"),
        ("media", "media", "polaroid"),
        ("videoInput", "videoInput", "video"),
    ] {
        let live = &output.live;
        let content = match layer {
            "slide" => !live["liveSlideId"].is_null(),
            "media" => live["backgroundId"]
                .as_str()
                .is_some_and(|id| !id.is_empty()),
            "music" => live["audioId"].as_str().is_some_and(|id| !id.is_empty()),
            "props" => live["livePropIds"]
                .as_array()
                .is_some_and(|ids| !ids.is_empty()),
            "messages" => !live["activeMessageId"].is_null(),
            "announcements" => !live["announcements"].is_null(),
            "videoInput" => !live["liveVideoInputId"].is_null(),
            _ => false,
        };
        let active = content
            && screen
                .as_ref()
                .is_some_and(|config| config["lookLayers"][layer] != false);
        let label = t.t(&format!("output.layer.{key}"));
        let accessible = t
            .t(if active {
                "output.layer.clear"
            } else {
                "output.layer.cleared"
            })
            .replace("{layer}", &label);
        let runtime = runtime.clone();
        rail = rail.child(
            Button::new(SharedString::from(format!("look-layer-{layer}")))
                .debug_selector(move || format!("look-layer-{layer}"))
                .accessibility_label(accessible.clone())
                .tooltip(tooltip_text(accessible))
                .disabled(output.committed_revision == 0 || screen.is_none() || !active)
                .flex_1()
                .min_h_0()
                .w_full()
                .bg(if active {
                    theme.colors.danger
                } else {
                    theme.colors.panel_bg
                })
                .text_color(if active {
                    theme.colors.text_strong
                } else {
                    theme.colors.text_muted
                })
                .focus_visible(|s| s.border_2().border_color(theme.colors.focus_ring))
                .hover(|s| {
                    s.bg(if active {
                        theme.colors.danger
                    } else {
                        theme.colors.control_bg_hover
                    })
                })
                .child(icon(glyph).text_color(if active {
                    theme.colors.text_strong
                } else {
                    theme.colors.text_muted
                }))
                .on_click(move |_, _, cx| actions::clear(&runtime, layer, cx)),
        );
    }
    rail.into_any_element()
}
