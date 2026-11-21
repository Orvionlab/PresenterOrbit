//! `MediaSettingsPanel.tsx`, `MediaSearchPathsPage.tsx`, `MediaCleanupPanelBridge.tsx`, and
//! `chooseMediaSearchPath` of `settings-panel-controller.ts`. The relink workflow itself runs in
//! `media::relink`; these controls only edit the saved general settings it reads.

use gpui::{
    div, prelude::*, AnyElement, Context, Entity, MouseButton, MouseDownEvent, Pixels, Point,
    SharedString, Subscription, Window,
};
use orbit_ui::controls::{
    input_state, ActionButton, ButtonVariant, CheckboxField, ContextMenu, InputEvent, InputState,
    MenuItem, SelectOption, TextInput,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Map, Value};

use super::rows::{column, group, link_row, select_row, setting_row, subpage_header, toggle_row};
use super::ui::{heading, muted};
use super::view::SettingsView;

/// `FOREGROUND_AUDIO_BEHAVIORS`.
pub const FOREGROUND_AUDIO_BEHAVIORS: [&str; 5] = [
    "ignore",
    "clear",
    "clear-if-audio",
    "pause",
    "pause-if-audio",
];

pub const SEARCH_PATHS_PAGE: &str = "searchPaths";

pub struct Rename {
    path: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

#[derive(Default)]
pub struct MediaSettings {
    pub locations: Option<Vec<Value>>,
    pub error: Option<String>,
    pub menu: Option<(String, Point<Pixels>)>,
    pub renaming: Option<Rename>,
    requested: bool,
}

/// One line of the search paths list.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchPathRow {
    pub path: String,
    pub label: String,
    pub detail: String,
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect()
}

fn names(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// The rows of `MediaSearchPathsPage`: the known locations that are not hidden, then the custom folders that
/// are enabled. A rename replaces the label of the location kind or the path itself.
pub fn search_path_rows(
    locations: &[Value],
    enabled: &[String],
    hidden: &[String],
    names: &Map<String, Value>,
    kind_label: &dyn Fn(&str) -> String,
) -> Vec<SearchPathRow> {
    let known: Vec<&str> = locations
        .iter()
        .filter_map(|location| location["path"].as_str())
        .collect();
    let named = |path: &str| names.get(path).and_then(Value::as_str).map(str::to_string);
    let mut rows: Vec<SearchPathRow> = locations
        .iter()
        .filter_map(|location| {
            let path = location["path"].as_str()?;
            (!hidden.iter().any(|item| item == path)).then(|| SearchPathRow {
                path: path.to_string(),
                label: named(path)
                    .unwrap_or_else(|| kind_label(location["kind"].as_str().unwrap_or(""))),
                detail: path.to_string(),
            })
        })
        .collect();
    rows.extend(
        enabled
            .iter()
            .filter(|path| !known.contains(&path.as_str()))
            .map(|path| {
                let name = named(path);
                SearchPathRow {
                    path: path.clone(),
                    label: name.clone().unwrap_or_else(|| path.clone()),
                    detail: if name.is_some() {
                        path.clone()
                    } else {
                        String::new()
                    },
                }
            }),
    );
    rows
}

/// `toggle`: the enabled paths after one checkbox.
pub fn toggled_paths(enabled: &[String], path: &str, checked: bool) -> Vec<String> {
    if checked {
        enabled.iter().cloned().chain([path.to_string()]).collect()
    } else {
        enabled
            .iter()
            .filter(|item| *item != path)
            .cloned()
            .collect()
    }
}

fn without_name(names: &Map<String, Value>, path: &str) -> Map<String, Value> {
    names
        .iter()
        .filter(|(key, _)| *key != path)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// `remove`: a known location is hidden instead of forgotten, so it does not come back from the host's list.
pub fn removal_patch(
    enabled: &[String],
    hidden: &[String],
    names: &Map<String, Value>,
    known: bool,
    path: &str,
) -> Value {
    let mut patch = json!({
        "mediaSearchPaths": toggled_paths(enabled, path, false),
        "mediaSearchPathNames": without_name(names, path),
    });
    if known {
        patch["hiddenMediaSearchPaths"] = json!(toggled_paths(hidden, path, true));
    }
    patch
}

/// `rename`: an empty name removes the custom name.
pub fn rename_patch(names: &Map<String, Value>, path: &str, name: &str) -> Value {
    let name = name.trim();
    let mut next = without_name(names, path);
    if !name.is_empty() {
        next.insert(path.to_string(), json!(name));
    }
    json!({ "mediaSearchPathNames": next })
}

/// `addSearchPath`: nothing to save when the folder is already listed and visible.
pub fn added_patch(enabled: &[String], hidden: &[String], path: &str) -> Option<Value> {
    let listed = enabled.iter().any(|item| item == path);
    if listed && !hidden.iter().any(|item| item == path) {
        return None;
    }
    Some(json!({
        "mediaSearchPaths": if listed {
            enabled.to_vec()
        } else {
            toggled_paths(enabled, path, true)
        },
        "hiddenMediaSearchPaths": hidden.iter().filter(|item| *item != path).collect::<Vec<_>>(),
    }))
}

impl SettingsView {
    fn media_general(&self, cx: &gpui::App) -> Value {
        self.settings(cx)["generalSettings"].clone()
    }

    fn load_search_locations(&mut self, cx: &mut Context<Self>) {
        if self.media_settings.requested {
            return;
        }
        self.media_settings.requested = true;
        self.host_call(
            "listMediaSearchLocations",
            Vec::new(),
            cx,
            |view, result, cx| {
                match result {
                    Ok(list) => view.media_settings.locations = list.as_array().cloned(),
                    Err(error) => view.media_settings.error = Some(error),
                }
                cx.notify();
            },
        );
    }

    fn add_search_path(&mut self, cx: &mut Context<Self>) {
        self.media_settings.error = None;
        self.host_call(
            "selectMediaSearchPath",
            Vec::new(),
            cx,
            |view, result, cx| {
                match result {
                    Ok(chosen) if chosen["canceled"] == json!(true) => {}
                    Ok(chosen) => {
                        let Some(path) = chosen["path"].as_str() else {
                            view.media_settings.error =
                                Some("The folder dialog returned no path.".into());
                            cx.notify();
                            return;
                        };
                        let general = view.media_general(cx);
                        let patch = added_patch(
                            &strings(&general["mediaSearchPaths"]),
                            &strings(&general["hiddenMediaSearchPaths"]),
                            path,
                        );
                        if let Some(patch) = patch {
                            view.patch_general(patch, cx);
                        }
                    }
                    Err(error) => view.media_settings.error = Some(error),
                }
                cx.notify();
            },
        );
    }

    fn begin_search_path_rename(
        &mut self,
        path: &str,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = input_state(label.to_string(), window, cx);
        let owner = path.to_string();
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    return;
                }
                let owns = this
                    .media_settings
                    .renaming
                    .as_ref()
                    .is_some_and(|rename| rename.path == owner);
                if !owns {
                    return;
                }
                this.media_settings.renaming = None;
                let general = this.media_general(cx);
                let name = input.read(cx).value().to_string();
                this.patch_general(
                    rename_patch(&names(&general["mediaSearchPathNames"]), &owner, &name),
                    cx,
                );
                cx.notify();
            },
        );
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        self.media_settings.menu = None;
        self.media_settings.renaming = Some(Rename {
            path: path.to_string(),
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn search_paths_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.load_search_locations(cx);
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let general = self.media_general(cx);
        let (enabled, hidden, saved_names) = (
            strings(&general["mediaSearchPaths"]),
            strings(&general["hiddenMediaSearchPaths"]),
            names(&general["mediaSearchPathNames"]),
        );
        let kind_label = |kind: &str| t.t(&format!("settings.media.location.{kind}"));
        let locations = self.media_settings.locations.clone().unwrap_or_default();
        let rows = search_path_rows(&locations, &enabled, &hidden, &saved_names, &kind_label);
        let renaming = self
            .media_settings
            .renaming
            .as_ref()
            .map(|rename| (rename.path.clone(), rename.input.clone()));
        let items: Vec<AnyElement> = rows
            .iter()
            .map(|row| {
                let (path, checked) = (row.path.clone(), enabled.contains(&row.path));
                let menu_path = row.path.clone();
                let toggle_weak = cx.weak_entity();
                let toggle_enabled = enabled.clone();
                let label: AnyElement = match &renaming {
                    Some((owner, input)) if *owner == row.path => {
                        let escape_owner = owner.clone();
                        div()
                            .flex_1()
                            .on_key_down(cx.listener(
                                move |view, event: &gpui::KeyDownEvent, _, cx| {
                                    if event.keystroke.key == "escape"
                                        && view
                                            .media_settings
                                            .renaming
                                            .as_ref()
                                            .is_some_and(|rename| rename.path == escape_owner)
                                    {
                                        view.media_settings.renaming = None;
                                        cx.notify();
                                    }
                                },
                            ))
                            .child(
                                TextInput::new("media-search-path-rename", input)
                                    .label(t.t("settings.media.renameSearchPath")),
                            )
                            .into_any_element()
                    }
                    _ => CheckboxField::new(
                        SharedString::from(format!("media-search-path-{path}")),
                        row.label.clone(),
                        checked,
                    )
                    .on_change(move |on, _, cx| {
                        let next = toggled_paths(&toggle_enabled, &path, on);
                        toggle_weak
                            .update(cx, |view, cx| {
                                view.patch_general(json!({ "mediaSearchPaths": next }), cx)
                            })
                            .ok();
                    })
                    .into_any_element(),
                };
                div()
                    .id(SharedString::from(format!("media-search-row-{}", row.path)))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(theme.metrics.space_md)
                    .min_h(
                        theme.metrics.control_height
                            + theme.metrics.space_md
                            + theme.metrics.space_2xs,
                    )
                    .px(theme.metrics.space_md)
                    .py(theme.metrics.space_sm)
                    .text_size(theme.metrics.font_sm)
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                            view.media_settings.menu = Some((menu_path.clone(), event.position));
                            cx.notify();
                        }),
                    )
                    .child(label)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.colors.text_muted)
                            .child(row.detail.clone()),
                    )
                    .into_any_element()
            })
            .collect();
        let menu = self.media_settings.menu.clone().map(|(path, position)| {
            let known = locations
                .iter()
                .any(|location| location["path"] == path.as_str());
            let label = rows
                .iter()
                .find(|row| row.path == path)
                .map(|row| row.label.clone())
                .unwrap_or_default();
            let (rename_weak, remove_weak, close) =
                (cx.weak_entity(), cx.weak_entity(), cx.weak_entity());
            let (rename_path, remove_path) = (path.clone(), path);
            let (hidden, enabled, saved_names) =
                (hidden.clone(), enabled.clone(), saved_names.clone());
            ContextMenu::new(
                "media-search-path-menu",
                position,
                vec![
                    MenuItem::new(t.t("settings.media.renameSearchPath"))
                        .icon("pencil")
                        .on_click(move |_, window, cx| {
                            rename_weak
                                .update(cx, |view, cx| {
                                    view.begin_search_path_rename(&rename_path, &label, window, cx)
                                })
                                .ok();
                        })
                        .into(),
                    MenuItem::new(t.t("settings.media.removeSearchPath"))
                        .icon("trash")
                        .on_click(move |_, _, cx| {
                            let patch =
                                removal_patch(&enabled, &hidden, &saved_names, known, &remove_path);
                            remove_weak
                                .update(cx, |view, cx| {
                                    view.media_settings.menu = None;
                                    view.patch_general(patch, cx)
                                })
                                .ok();
                        })
                        .into(),
                ],
                move |_, _, cx| {
                    let close = close.clone();
                    cx.defer(move |cx| {
                        close
                            .update(cx, |view, cx| {
                                view.media_settings.menu = None;
                                cx.notify();
                            })
                            .ok();
                    });
                },
            )
        });
        let weak = cx.weak_entity();
        column(cx)
            .id("settings-media-search-paths")
            .child(subpage_header(
                "media-search-paths-back",
                t.t("settings.media.back"),
                t.t("settings.media.searchPaths"),
                move |_, cx| {
                    weak.update(cx, |view, cx| view.close_page(cx)).ok();
                },
                cx,
            ))
            .child(muted(t.t("settings.media.searchPathsDescription"), cx))
            .child(group(items, cx))
            .child(
                super::ui::action_row(cx).child(
                    ActionButton::new("media-search-path-add", t.t("settings.media.addSearchPath"))
                        .icon("folder")
                        .on_click(cx.listener(|view, _, _, cx| view.add_search_path(cx))),
                ),
            )
            .children(
                self.media_settings
                    .error
                    .clone()
                    .map(|error| div().text_color(theme.colors.warning).child(error)),
            )
            .children(menu)
            .into_any_element()
    }

    /// `MediaCleanupWorkspace`: the scan row and what its review found.
    fn cleanup_rows(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let available = !self.runtime.content.read(cx).workspace_path.is_empty();
        let busy = self.services.cleanup_busy;
        let mut section = div().flex().flex_col().child(setting_row(
            t.t("media.cleanup.title"),
            Some(t.t("media.cleanup.description").into()),
            ActionButton::new("media-cleanup-scan", t.t("media.cleanup.scan"))
                .variant(ButtonVariant::Secondary)
                .disabled(busy || !available)
                .on_click(cx.listener(|view, _, _, cx| view.run_cleanup(false, cx))),
            cx,
        ));
        let mut details = div()
            .flex()
            .flex_col()
            .gap(theme.metrics.space_sm)
            .px(theme.metrics.space_md)
            .pb(theme.metrics.space_sm)
            .text_size(theme.metrics.font_sm);
        let mut any = false;
        if let Some(review) = self.services.review.clone() {
            any = true;
            let files = review["files"].as_array().cloned().unwrap_or_default();
            details = if files.is_empty() {
                details.child(muted(t.t("media.cleanup.empty"), cx))
            } else {
                details.children(files.iter().map(|file| {
                    div().child(format!(
                        "{} ({} {})",
                        file["relativePath"].as_str().unwrap_or(""),
                        file["bytes"],
                        t.t("capture.bytes")
                    ))
                }))
            };
            details = details.child(
                div().child(
                    ActionButton::new("media-cleanup-confirm", t.t("media.cleanup.confirm"))
                        .variant(ButtonVariant::Secondary)
                        .disabled(busy || files.is_empty())
                        .on_click(cx.listener(|view, _, _, cx| view.run_cleanup(true, cx))),
                ),
            );
        }
        if let Some(path) = self.services.recovery.clone() {
            any = true;
            details = details.child(format!("{} {path}", t.t("media.cleanup.recovery")));
        }
        if let Some(error) = self.services.cleanup_error.clone() {
            any = true;
            details = details.child(div().text_color(theme.colors.warning).child(error));
        }
        if any {
            section = section.child(details);
        }
        section.into_any_element()
    }

    fn foreground_audio_row(
        &self,
        key: &'static str,
        label_key: &str,
        general: &Value,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let weak = cx.weak_entity();
        select_row(
            SharedString::from(format!("media-{key}")),
            t.t(label_key),
            FOREGROUND_AUDIO_BEHAVIORS
                .iter()
                .map(|value| {
                    SelectOption::new(
                        value.to_string(),
                        t.t(&format!("settings.media.foregroundAudio.{value}")),
                    )
                })
                .collect(),
            general[key].as_str().unwrap_or("ignore").to_string(),
            move |value, _, cx| {
                weak.update(cx, |view, cx| view.patch_general(json!({ key: value }), cx))
                    .ok();
            },
            cx,
        )
    }

    pub(super) fn media_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.page == Some(SEARCH_PATHS_PAGE) {
            return self.search_paths_page(cx);
        }
        let t = *cx.global::<Translator>();
        let general = self.media_general(cx);
        let flag = |key: &str, default: bool| general[key].as_bool().unwrap_or(default);
        let toggle = |id: &'static str,
                      key: &'static str,
                      label: &str,
                      description: &str,
                      checked: bool,
                      cx: &mut Context<Self>| {
            let weak = cx.weak_entity();
            toggle_row(
                id,
                t.t(label),
                Some(t.t(description).into()),
                checked,
                move |value, _, cx| {
                    weak.update(cx, |view, cx| view.patch_general(json!({ key: value }), cx))
                        .ok();
                },
                cx,
            )
        };
        let weak = cx.weak_entity();
        let management = group(
            [
                toggle(
                    "media-manage-automatically",
                    "manageMediaAutomatically",
                    "settings.media.manageAutomatically",
                    "settings.media.manageAutomaticallyDescription",
                    flag("manageMediaAutomatically", true),
                    cx,
                ),
                toggle(
                    "media-relink-missing",
                    "relinkMissingMedia",
                    "settings.media.relinkMissing",
                    "settings.media.relinkMissingDescription",
                    flag("relinkMissingMedia", false),
                    cx,
                ),
                link_row(
                    "media-search-paths-link",
                    None,
                    t.t("settings.media.searchPaths"),
                    move |_, cx| {
                        weak.update(cx, |view, cx| view.open_page(SEARCH_PATHS_PAGE, cx))
                            .ok();
                    },
                    cx,
                ),
                self.cleanup_rows(cx),
            ],
            cx,
        );
        let playback = group(
            [toggle(
                "media-alt-click",
                "altClickSuppressesVideoAutoStart",
                "settings.media.altClick",
                "settings.media.altClickDescription",
                flag("altClickSuppressesVideoAutoStart", false),
                cx,
            )],
            cx,
        );
        let foreground = group(
            [
                self.foreground_audio_row(
                    "foregroundAudioPresentation",
                    "settings.media.foregroundAudioPresentation",
                    &general,
                    cx,
                ),
                self.foreground_audio_row(
                    "foregroundAudioAnnouncements",
                    "settings.media.foregroundAudioAnnouncements",
                    &general,
                    cx,
                ),
            ],
            cx,
        );
        column(cx)
            .id("settings-media")
            .child(heading(t.t("settings.nav.media"), cx))
            .child(heading(t.t("settings.media.management"), cx))
            .child(management)
            .child(heading(t.t("settings.media.videoPlayback"), cx))
            .child(playback)
            .child(heading(t.t("settings.media.foregroundAudio"), cx))
            .child(foreground)
            .into_any_element()
    }
}
