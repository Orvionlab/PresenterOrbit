//! `GroupsSettingsPanel.tsx`, `group-settings-utils.ts`, `group-settings-mutation-persistence.ts`, and the group
//! half of `settings-actions.ts`. Every edit is one `mutateGroupSettings` intent; the host owns naming and ids.

use gpui::{
    div, prelude::*, AnyElement, Context, Entity, MouseButton, MouseDownEvent, Pixels, Point,
    SharedString, Subscription, Window,
};
use orbit_ui::controls::{
    input_state, ActionButton, ColorPicker, ContextMenu, InputEvent, InputState, MenuItem,
    TextInput,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::persist;
use super::view::SettingsView;
use crate::collections::model::slug_id;
use crate::keys::normalize_shortcut_key;

const COLOR_COLUMN: Pixels = gpui::px(72.);
const KEY_COLUMN: Pixels = gpui::px(54.);
pub const DEFAULT_COLOR: &str = "#0f7db7";
const NO_GROUP: &str = "no-group";

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub color: String,
    pub key: String,
}

fn trimmed(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_string(),
        Value::Null => String::new(),
        Value::Bool(false) => String::new(),
        other => other.to_string().trim().to_string(),
    }
}

/// `isGeneratedSlideMetadataLabel`: eight or more digits, or `new-slide` with an optional copy suffix.
pub fn is_generated_label(value: &str) -> bool {
    let label = value.trim();
    if label.len() >= 8 && label.chars().all(|character| character.is_ascii_digit()) {
        return true;
    }
    let lower = label.to_lowercase();
    match lower.strip_prefix("new-slide") {
        Some("") | Some("-copy") => true,
        Some(rest) => rest
            .strip_prefix("-copy-")
            .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())),
        None => false,
    }
}

/// `isIgnoredGroupName`: `Slide 1` to `Slide 25` and `Verse 7` and `Verse 8`, in any case.
pub fn is_ignored_group_name(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    let numbered = |prefix: &str, valid: &dyn Fn(&str) -> bool| {
        lower
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace) && valid(rest.trim_start()))
    };
    numbered("slide", &|digits| {
        matches!(digits.parse::<u32>(), Ok(1..=25))
            && !digits.starts_with('0')
            && digits.chars().all(|c| c.is_ascii_digit())
    }) || numbered("verse", &|digits| digits == "7" || digits == "8")
}

/// `normalizeGroupSettings`: entries without a usable id or with an ignored or generated name are dropped; a
/// missing colour or key takes its default.
pub fn normalize_group_settings(value: &Value) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let Some(entries) = value.as_object() else {
        return groups;
    };
    for (key, group) in entries {
        let data = group.as_object().cloned().unwrap_or_default();
        let name = {
            let named = data.get("name").map(trimmed).unwrap_or_default();
            if named.is_empty() {
                key.trim().to_string()
            } else {
                named
            }
        };
        let source = data
            .get("id")
            .map(trimmed)
            .filter(|id| !id.is_empty())
            .or_else(|| (!key.is_empty()).then(|| key.clone()))
            .unwrap_or_else(|| name.clone());
        let id = slug_id(&source);
        if id.is_empty()
            || id == NO_GROUP
            || is_generated_label(&id)
            || is_generated_label(&name)
            || is_ignored_group_name(&name)
        {
            continue;
        }
        let color = data
            .get("color")
            .and_then(Value::as_str)
            .filter(|color| !color.is_empty())
            .unwrap_or(DEFAULT_COLOR)
            .to_string();
        let key = data
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let group = Group {
            id: id.clone(),
            name,
            color,
            key,
        };
        match groups.iter_mut().find(|existing| existing.id == id) {
            Some(existing) => *existing = group,
            None => groups.push(group),
        }
    }
    groups
}

/// `getSlideGroupId`.
pub fn slide_group_id(slide: &Value) -> String {
    let value = if slide.get("groupId").is_some() {
        &slide["groupId"]
    } else {
        &slide["groupName"]
    };
    let text = trimmed(value);
    slug_id(if is_generated_label(&text) { "" } else { &text })
}

/// `getKnownGroups`: the configured groups, then the groups that slides use.
pub fn known_groups(group_settings: &Value, library: &Value) -> Vec<String> {
    let groups = normalize_group_settings(group_settings);
    let mut ids: Vec<String> = groups
        .iter()
        .filter(|group| !is_generated_label(&group.id))
        .map(|group| group.id.clone())
        .collect();
    for item in library.as_array().into_iter().flatten() {
        for slide in item["slides"].as_array().into_iter().flatten() {
            let id = slide_group_id(slide);
            let name = groups
                .iter()
                .find(|group| group.id == id)
                .map_or(id.as_str(), |group| group.name.as_str());
            if !id.is_empty()
                && id != NO_GROUP
                && !is_ignored_group_name(name)
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
    }
    ids
}

/// The rows of the panel: known groups with their settings.
pub fn group_rows(group_settings: &Value, library: &Value) -> Vec<Group> {
    let groups = normalize_group_settings(group_settings);
    known_groups(group_settings, library)
        .into_iter()
        .map(|id| {
            groups
                .iter()
                .find(|group| group.id == id)
                .cloned()
                .unwrap_or(Group {
                    name: id.clone(),
                    id,
                    color: DEFAULT_COLOR.into(),
                    key: String::new(),
                })
        })
        .collect()
}

/// `clearDeletedGroupAssignments`: the labels slides get when their group is deleted.
pub fn deleted_group_labels(
    library: &Value,
    group_id: &str,
    label: impl Fn(usize) -> String,
) -> Vec<Value> {
    let mut labels = Vec::new();
    for item in library.as_array().into_iter().flatten() {
        for (index, slide) in item["slides"].as_array().into_iter().flatten().enumerate() {
            if slide_group_id(slide) == group_id {
                labels.push(json!({
                    "itemId": item["id"],
                    "slideId": slide["id"],
                    "label": label(index + 1),
                }));
            }
        }
    }
    labels
}

pub struct Rename {
    id: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

#[derive(Default)]
pub struct Groups {
    pub renaming: Option<Rename>,
    pub menu: Option<(String, Point<Pixels>)>,
}

impl SettingsView {
    fn group_source(&self, cx: &gpui::App) -> (Value, Value) {
        let content = self.runtime.content.read(cx);
        let workspace = &content.workspace;
        let settings = self.settings(cx);
        let own = settings["groupSettings"].clone();
        (own, workspace["library"].clone())
    }

    pub(super) fn mutate_groups(
        &mut self,
        mutation: Value,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<Value, String>> {
        persist::mutate(&self.runtime, cx, "mutateGroupSettings", mutation)
    }

    pub(super) fn update_group(&mut self, id: &str, patch: Value, cx: &mut Context<Self>) {
        let task = self.mutate_groups(json!({"type": "update", "groupId": id, "patch": patch}), cx);
        self.report(task, cx);
    }

    pub(super) fn add_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let t = *cx.global::<Translator>();
        let task = self.mutate_groups(
            json!({
                "type": "add",
                "baseName": t.t("settings.groups.defaultName"),
                "color": DEFAULT_COLOR,
                "key": "",
            }),
            cx,
        );
        self.groups.menu = None;
        cx.spawn_in(window, async move |view, cx| {
            let result = task.await;
            view.update_in(cx, |view, window, cx| {
                match &result {
                    Ok(accepted) => {
                        if let Some(id) = accepted["createdGroupId"].as_str() {
                            view.begin_group_rename(id, window, cx);
                        }
                        view.status = None;
                    }
                    Err(error) => {
                        let t = *cx.global::<Translator>();
                        view.status = Some(
                            t.format("status.workspace.saveFailed", &[("error", error)])
                                .into(),
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn begin_group_rename(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (source, library) = self.group_source(cx);
        let Some(group) = group_rows(&source, &library)
            .into_iter()
            .find(|group| group.id == id)
        else {
            return;
        };
        let input = input_state(group.name.clone(), window, cx);
        let owner = id.to_string();
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                    && this
                        .groups
                        .renaming
                        .as_ref()
                        .is_some_and(|rename| rename.id == owner)
                {
                    let name = input.read(cx).value().to_string();
                    this.commit_group_rename(&owner, &name, cx);
                }
            },
        );
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        self.groups.menu = None;
        self.groups.renaming = Some(Rename {
            id: id.to_string(),
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// `renameGroupSetting`: an empty, unchanged, unknown, or ignored name only ends the edit.
    pub(super) fn commit_group_rename(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        self.groups.renaming = None;
        let (source, library) = self.group_source(cx);
        let name = name.trim().to_string();
        let current = normalize_group_settings(&source)
            .into_iter()
            .find(|group| group.id == id);
        let exists = current.is_some()
            || group_rows(&source, &library)
                .iter()
                .any(|group| group.id == id);
        if !exists
            || name.is_empty()
            || current.as_ref().is_some_and(|group| group.name == name)
            || is_ignored_group_name(&name)
        {
            cx.notify();
            return;
        }
        let task = self.mutate_groups(json!({"type": "rename", "groupId": id, "name": name}), cx);
        self.report(task, cx);
        cx.notify();
    }

    pub(super) fn delete_group(&mut self, id: &str, cx: &mut Context<Self>) {
        self.groups.menu = None;
        self.groups.renaming = None;
        let (_, library) = self.group_source(cx);
        let t = *cx.global::<Translator>();
        let has_workspace = !self.runtime.content.read(cx).workspace_path.is_empty();
        let labels = if has_workspace {
            deleted_group_labels(&library, id, |number| {
                t.format("cue.defaultSlide", &[("number", &number.to_string())])
            })
        } else {
            Vec::new()
        };
        let task = self.mutate_groups(
            json!({"type": "delete", "groupId": id, "slideLabels": labels}),
            cx,
        );
        self.report(task, cx);
        cx.notify();
    }

    fn group_row(
        &mut self,
        group: &Group,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let id = group.id.clone();
        let name: AnyElement = match &self.groups.renaming {
            Some(rename) if rename.id == group.id => {
                let owner = id.clone();
                div()
                    .flex_1()
                    .on_key_down(cx.listener(move |view, event: &gpui::KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape"
                            && view.groups.renaming.as_ref().is_some_and(|r| r.id == owner)
                        {
                            view.groups.renaming = None;
                            cx.notify();
                        }
                    }))
                    .child(TextInput::new(
                        SharedString::from(format!("group-rename-{id}")),
                        &rename.input,
                    ))
                    .into_any_element()
            }
            _ => {
                let rename_id = id.clone();
                div()
                    .id(SharedString::from(format!("group-name-{id}")))
                    .flex_1()
                    .min_w_0()
                    .cursor_pointer()
                    .child(group.name.clone())
                    .on_click(
                        cx.listener(move |view, event: &gpui::ClickEvent, window, cx| {
                            if event.click_count() >= 2 {
                                view.begin_group_rename(&rename_id, window, cx);
                            }
                        }),
                    )
                    .into_any_element()
            }
        };
        let (color_id, key_id) = (id.clone(), id.clone());
        let weak = cx.weak_entity();
        let key_state = self.group_key_input(&group.id, &group.key, window, cx);
        let menu_id = id.clone();
        div()
            .id(SharedString::from(format!("group-row-{id}")))
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.metrics.space_sm)
            .min_h(theme.metrics.control_height)
            .px(theme.metrics.space_sm)
            .rounded(theme.metrics.radius_sm)
            .bg(theme.colors.panel_bg_soft)
            .text_size(theme.metrics.font_sm)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                    view.groups.menu = Some((menu_id.clone(), event.position));
                    cx.notify();
                }),
            )
            .child(name)
            .child(
                div().w(COLOR_COLUMN).flex().justify_center().child(
                    div().w(gpui::px(54.)).child(
                        ColorPicker::new(
                            SharedString::from(format!("group-color-{color_id}")),
                            group.name.clone(),
                            group.color.clone(),
                        )
                        .on_change(move |color, _, cx| {
                            weak.update(cx, |view, cx| {
                                view.update_group(&color_id, json!({"color": color}), cx)
                            })
                            .ok();
                        }),
                    ),
                ),
            )
            .child(
                div().w(KEY_COLUMN).flex().justify_center().child(
                    div().w(gpui::px(42.)).child(
                        TextInput::new(
                            SharedString::from(format!("group-key-{key_id}")),
                            &key_state,
                        )
                        .compact(),
                    ),
                ),
            )
            .into_any_element()
    }

    fn group_key_input(
        &mut self,
        id: &str,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let owner = id.to_string();
        let commit = std::rc::Rc::new(
            move |view: &mut SettingsView, text: String, cx: &mut Context<SettingsView>| {
                let key = normalize_shortcut_key(&text);
                view.update_group(&owner, json!({"key": key}), cx);
            },
        );
        self.field_state(&format!("group-key-{id}"), key, window, cx, commit, false)
    }

    pub(super) fn groups_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let (source, library) = self.group_source(cx);
        let rows: Vec<AnyElement> = group_rows(&source, &library)
            .iter()
            .map(|group| self.group_row(group, window, cx))
            .collect();
        let menu = self.groups.menu.clone().map(|(id, position)| {
            let (weak, close) = (cx.weak_entity(), cx.weak_entity());
            ContextMenu::new(
                "group-menu",
                position,
                vec![MenuItem::new(t.t("settings.groups.delete"))
                    .on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| view.delete_group(&id, cx)).ok();
                    })
                    .into()],
                move |_, _, cx| {
                    let close = close.clone();
                    cx.defer(move |cx| {
                        close
                            .update(cx, |view, cx| {
                                view.groups.menu = None;
                                cx.notify();
                            })
                            .ok();
                    });
                },
            )
        });
        let header = |label: String, width: Option<Pixels>| {
            div()
                .when_some(width, |this, width| {
                    this.w(width).flex().justify_center().flex_none()
                })
                .when(width.is_none(), |this| this.flex_1().min_w_0())
                .child(label.to_uppercase())
        };
        div()
            .id("settings-groups")
            .flex()
            .flex_col()
            .size_full()
            .gap(theme.metrics.panel_gap)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(theme.metrics.space_sm)
                    .min_h(theme.metrics.header_height)
                    .px(theme.metrics.space_sm)
                    .text_size(theme.metrics.font_xs)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(theme.colors.text_strong)
                    .child(header(t.t("settings.groups.name"), None))
                    .child(header(t.t("settings.groups.color"), Some(COLOR_COLUMN)))
                    .child(header(t.t("settings.groups.key"), Some(KEY_COLUMN))),
            )
            .child(
                div()
                    .id("settings-groups-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(gpui::px(1.))
                    .children(rows),
            )
            .child(
                ActionButton::new("groups-add", t.t("settings.groups.add"))
                    .on_click(cx.listener(|view, _, window, cx| view.add_group(window, cx))),
            )
            .children(menu)
            .into_any_element()
    }
}
