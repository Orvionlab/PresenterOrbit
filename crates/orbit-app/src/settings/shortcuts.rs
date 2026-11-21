//! `KeyMappingsSettingsPanel.tsx`, `key-mapping-utils.ts` (items and labels), and
//! `key-mapping-mutation-persistence.ts`. A key press in a shortcut field becomes one `mutateKeyMappings` intent;
//! the host moves a shortcut away from the command that held it.

use std::collections::HashMap;

use gpui::{div, prelude::*, AnyElement, Context, FocusHandle, KeyDownEvent, SharedString, Window};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::persist;
use super::view::SettingsView;
use crate::keys::{
    format_shortcut_label, shortcut_from_keystroke, theme_shape_command_id, KeyMappings,
    PRESENTATION_MENU_ITEMS, THEME_SHAPES,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Heading(String),
    Command { id: String, label: String },
}

/// The commands `updateKeyMapping` accepts: the defaults of `DEFAULT_KEY_MAPPINGS`.
pub fn is_default_command(id: &str) -> bool {
    PRESENTATION_MENU_ITEMS
        .iter()
        .flatten()
        .any(|command| *command == id)
        || THEME_SHAPES
            .iter()
            .any(|(shape, _)| theme_shape_command_id(shape) == id)
}

/// `KEY_MAPPING_SETTINGS_ITEMS` followed by the macros, labelled through `t`.
pub fn items(macros: &Value, t: impl Fn(&str) -> String) -> Vec<Item> {
    let mut items = vec![Item::Heading(t("settings.keys.presentationShortcuts"))];
    for command in PRESENTATION_MENU_ITEMS.iter().flatten() {
        items.push(Item::Command {
            id: command.to_string(),
            label: t(&format!("presentation.command.{command}")),
        });
    }
    items.push(Item::Heading(t("settings.keys.themeShortcuts")));
    for (shape, _) in THEME_SHAPES {
        items.push(Item::Command {
            id: theme_shape_command_id(shape),
            label: t(&format!("theme.shapeCommand.{shape}")),
        });
    }
    let macros = macros.as_array().cloned().unwrap_or_default();
    if !macros.is_empty() {
        items.push(Item::Heading("Macros".to_string()));
        for macro_ in macros {
            items.push(Item::Command {
                id: format!("macro:{}", macro_["id"].as_str().unwrap_or("")),
                label: macro_["title"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    items
}

#[derive(Default)]
pub struct Shortcuts {
    focus: HashMap<String, FocusHandle>,
}

impl SettingsView {
    /// `updateKeyMapping`: only the known commands can be assigned.
    pub(super) fn set_shortcut(&mut self, command: &str, shortcut: &str, cx: &mut Context<Self>) {
        if !is_default_command(command) {
            return;
        }
        let task = persist::mutate(
            &self.runtime,
            cx,
            "mutateKeyMappings",
            json!({"type": "set", "commandId": command, "shortcut": shortcut}),
        );
        self.report(task, cx);
    }

    fn shortcut_row(
        &mut self,
        command: &str,
        label: &str,
        shortcut: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let focus = self
            .shortcuts
            .focus
            .entry(command.to_string())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let owner = command.to_string();
        let shown = if shortcut.is_empty() {
            t.t("settings.keys.none")
        } else {
            format_shortcut_label(shortcut)
        };
        div()
            .id(SharedString::from(format!("shortcut-row-{command}")))
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.metrics.space_sm)
            .min_h(theme.metrics.control_height)
            .px(theme.metrics.space_sm)
            .rounded(theme.metrics.radius_sm)
            .bg(theme.colors.panel_bg_soft)
            .text_size(theme.metrics.font_sm)
            .child(div().flex_1().min_w_0().truncate().child(label.to_string()))
            .child(
                div()
                    .id(SharedString::from(format!("shortcut-{command}")))
                    .track_focus(&focus)
                    .tab_stop(true)
                    .w(gpui::px(148.))
                    .flex_none()
                    .h(theme.metrics.control_height)
                    .px(theme.metrics.field_padding_x)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.colors.field_bg)
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(theme.metrics.radius_sm)
                    .text_color(if shortcut.is_empty() {
                        theme.colors.text_subtle
                    } else {
                        theme.colors.text_strong
                    })
                    .child(shown)
                    .on_click(cx.listener({
                        let focus = focus.clone();
                        move |_, _, window, cx| window.focus(&focus, cx)
                    }))
                    .on_key_down(cx.listener(move |view, event: &KeyDownEvent, _, cx| {
                        cx.stop_propagation();
                        if matches!(event.keystroke.key.as_str(), "backspace" | "delete") {
                            view.set_shortcut(&owner, "", cx);
                        } else if let Some(shortcut) = shortcut_from_keystroke(&event.keystroke) {
                            view.set_shortcut(&owner, &shortcut, cx);
                        }
                    })),
            )
            .into_any_element()
    }

    pub(super) fn keys_panel(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let settings = self.settings(cx);
        let mappings = KeyMappings::from_settings(&settings);
        let macros = self.runtime.content.read(cx).workspace["macros"].clone();
        let mut rows: Vec<AnyElement> = Vec::new();
        for item in items(&macros, |key| t.t(key)) {
            rows.push(match item {
                Item::Heading(label) => div()
                    .mt(theme.metrics.space_md)
                    .mb(theme.metrics.space_snug)
                    .text_size(theme.metrics.font_sm)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.colors.text_strong)
                    .child(label)
                    .into_any_element(),
                Item::Command { id, label } => {
                    let shortcut = mappings.shortcut(&id).unwrap_or("").to_string();
                    self.shortcut_row(&id, &label, &shortcut, cx)
                }
            });
        }
        div()
            .id("settings-keys")
            .flex()
            .flex_col()
            .gap(theme.metrics.panel_gap)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(theme.metrics.space_sm)
                    .min_h(theme.metrics.header_height)
                    .text_size(theme.metrics.font_xs)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(theme.colors.text_strong)
                    .child(
                        div()
                            .flex_1()
                            .child(t.t("settings.keys.command").to_uppercase()),
                    )
                    .child(
                        div()
                            .w(gpui::px(148.))
                            .flex()
                            .justify_center()
                            .child(t.t("settings.keys.shortcut").to_uppercase()),
                    ),
            )
            .child(
                div()
                    .id("settings-keys-list")
                    .flex()
                    .flex_col()
                    .gap(gpui::px(1.))
                    .children(rows),
            )
            .into_any_element()
    }
}
