//! The Looks editor of the Video panel, after ProPresenter's Looks window: the presets on the left, and a grid of
//! the audience screens against the layers a look sets on each. Presets are the workspace's `audienceLooks`;
//! applying one writes the screens' own look state (`output::looks`). This is the only place a screen's look is
//! edited; the per-screen settings no longer carry it.

use super::ui::muted;
use gpui::{
    div, prelude::*, AnyElement, Context, Entity, MouseButton, Pixels, Point, SharedString,
    Subscription, Window,
};
use orbit_ui::controls::{
    input_state, list_row, range_state, ActionButton, ButtonVariant, CheckboxField, ContextMenu,
    IconButton, InputEvent, InputState, MenuItem, PanelHeader, RangeInput, Select, SelectOption,
    SliderEvent, SliderState, TextInput,
};
use orbit_ui::{icon, ActiveTheme, Translator};
use serde_json::{json, Value};

use super::video::{group_entries, look_layers};
use super::video_extra::{look_themes, theme_slide_options};
use super::view::SettingsView;

/// The rows of the grid in ProPresenter's order: the mask and theme selectors among the layer checkboxes.
const ROWS: [&str; 9] = [
    "maskId",
    "music",
    "messages",
    "props",
    "announcements",
    "presentationThemeId",
    "slide",
    "media",
    "videoInput",
];

/// `captureAudienceLook`: the current look of every audience screen.
pub fn capture(configs: &Value, id: &str) -> Value {
    let screens: Vec<Value> = group_entries(configs, "audience")
        .into_iter()
        .map(|(screen, config)| {
            let themes = look_themes(&config);
            json!({"screenId": screen, "lookLayers": look_layers(&config), "maskId": themes["maskId"], "presentationThemeId": themes["presentationThemeId"]})
        })
        .collect();
    json!({"id": id, "title": "", "screenLooks": screens})
}

/// Shows every audience screen: adds an entry, from the screen's current look, for each the look has none for,
/// drops entries of screens that are no longer audience screens, and shows what an entry leaves unset (a layer, the
/// mask or the theme) as the screen's current value, which is what applying the look leaves in place.
pub(crate) fn complete(look: &mut Value, configs: &Value) {
    let current = capture(configs, "")["screenLooks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut entries: Vec<Value> = Vec::new();
    for own in look["screenLooks"].as_array().into_iter().flatten() {
        let Some(screen) = current.iter().find(|c| c["screenId"] == own["screenId"]) else {
            continue;
        };
        let mut layers = screen["lookLayers"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        layers.extend(own["lookLayers"].as_object().cloned().unwrap_or_default());
        let pick = |key: &str| {
            if own[key].is_null() {
                screen[key].clone()
            } else {
                own[key].clone()
            }
        };
        entries.push(json!({"screenId": own["screenId"], "lookLayers": layers, "maskId": pick("maskId"), "presentationThemeId": pick("presentationThemeId")}));
    }
    let missing: Vec<Value> = current
        .into_iter()
        .filter(|entry| {
            !entries
                .iter()
                .any(|own| own["screenId"] == entry["screenId"])
        })
        .collect();
    entries.extend(missing);
    look["screenLooks"] = json!(entries);
}

pub fn duplicate(look: &Value, id: &str) -> Value {
    let mut copy = look.clone();
    copy["id"] = json!(id);
    copy["title"] = json!(format!("{} (copy)", look["title"].as_str().unwrap_or("")));
    copy
}

fn known_theme(themes: &Value, id: &str) -> bool {
    theme_slide_options(themes)
        .iter()
        .any(|(value, _)| value == id)
}

/// `saveAudienceLook` and `validateAudienceLookActivation` checks: the reason, or `None`.
pub fn look_error(
    look: &Value,
    configs: &Value,
    themes: &Value,
    masks: &Value,
) -> Option<&'static str> {
    if look["title"].as_str().is_none_or(|t| t.trim().is_empty()) {
        return Some("An Audience Look name is required.");
    }
    let entries = look["screenLooks"].as_array().cloned().unwrap_or_default();
    let audience: Vec<String> = group_entries(configs, "audience")
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    if (!audience.is_empty() && entries.is_empty())
        || entries
            .iter()
            .any(|e| !audience.iter().any(|id| e["screenId"] == id.as_str()))
    {
        return Some("An Audience Look must reference available audience screens.");
    }
    let mut seen = std::collections::HashSet::new();
    if !entries
        .iter()
        .all(|e| seen.insert(e["screenId"].as_str().unwrap_or("").to_string()))
    {
        return Some("Audience Look screen identities must be unique.");
    }
    for entry in &entries {
        if let Some(mask) = entry["maskId"].as_str().filter(|m| !m.is_empty()) {
            if !masks
                .as_array()
                .is_some_and(|l| l.iter().any(|m| m["id"] == mask))
            {
                return Some("Audience Look mask is unavailable.");
            }
        }
        if let Some(theme) = entry["presentationThemeId"]
            .as_str()
            .filter(|t| !t.is_empty())
        {
            if !known_theme(themes, theme) {
                return Some("Audience Look theme is unavailable.");
            }
        }
    }
    None
}

#[derive(Default)]
pub struct Looks {
    /// The selected preset, updated immediately as its controls change.
    pub draft: Option<Value>,
    pub error: Option<String>,
    menu: Option<(Point<Pixels>, Value)>,
    save_menu: Option<Point<Pixels>>,
    save_anchor: Option<Point<Pixels>>,
    editing_live: bool,
    rename: Option<(Entity<InputState>, Subscription)>,
    workspace: String,
    /// The transition slider of each edited preset.
    pub(super) transitions: std::collections::HashMap<String, Entity<SliderState>>,
}

fn seconds_label(seconds: f64) -> String {
    format!("{seconds:.1}s")
}

impl SettingsView {
    pub(super) fn look_edit(&mut self, change: impl FnOnce(&mut Value), cx: &mut Context<Self>) {
        if let Some(draft) = self.looks.draft.as_mut() {
            change(draft);
            if self.looks.editing_live {
                let look = self.looks.draft.clone();
                if let Some(look) = look {
                    let configs = self.screen_configs(cx);
                    self.looks.error =
                        crate::output::looks::edit(&self.runtime, &configs, &look, cx).err();
                }
                cx.notify();
            } else {
                self.look_save(cx);
            }
        }
    }

    fn look_list(&self, cx: &gpui::App) -> Vec<Value> {
        crate::output::looks::presets(&self.runtime, cx)
    }

    /// Opens a saved preset in the grid, with a column for every audience screen.
    pub(super) fn look_select(&mut self, look: &Value, cx: &mut Context<Self>) {
        self.look_finish_rename(cx);
        self.looks.editing_live = false;
        let mut draft = look.clone();
        complete(&mut draft, &self.screen_configs(cx));
        self.looks
            .transitions
            .remove(look["id"].as_str().unwrap_or(""));
        self.looks.draft = Some(draft);
        self.looks.error = None;
        cx.notify();
    }

    /// Reflect edits before the ordered workspace write completes.
    pub(super) fn looks_persist(
        &mut self,
        list: Vec<Value>,
        keep: Option<Value>,
        cx: &mut Context<Self>,
    ) {
        self.looks.draft = keep;
        self.looks.error = None;
        let runtime = self.runtime.clone();
        let result = crate::automation::save::optimistic_content(
            &runtime,
            json!({"audienceLooks": list}),
            cx,
            |view, result, cx| {
                if let Err(error) = result {
                    view.looks.error = Some(error);
                    let selected = view.looks.draft.as_ref().map(|d| d["id"].clone());
                    view.looks.draft = view
                        .look_list(cx)
                        .into_iter()
                        .find(|l| Some(&l["id"]) == selected.as_ref());
                }
                cx.notify();
            },
        );
        if let Err(error) = result {
            self.looks.error = Some(error);
        } else if let Some(look) = self.looks.draft.clone().filter(|look| {
            crate::output::looks::live_id(&runtime, cx).as_deref() == look["id"].as_str()
        }) {
            self.look_activate(&look, cx);
        }
        cx.notify();
    }

    fn look_rename(&mut self, look: &Value, window: &mut Window, cx: &mut Context<Self>) {
        self.look_select(look, cx);
        let input = input_state(look["title"].as_str().unwrap_or("").to_string(), window, cx);
        let subscription =
            cx.subscribe_in(&input, window, |view, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                    && view
                        .looks
                        .rename
                        .as_ref()
                        .is_some_and(|(active, _)| active == input)
                {
                    view.look_finish_rename(cx);
                }
            });
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.looks.rename = Some((input, subscription));
    }

    fn look_finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some((input, _)) = self.looks.rename.take() else {
            return;
        };
        let title = input.read(cx).value().trim().to_string();
        if !title.is_empty()
            && self
                .looks
                .draft
                .as_ref()
                .is_some_and(|draft| draft["title"] != title)
        {
            self.look_edit(|draft| draft["title"] = json!(title), cx);
        }
        cx.notify();
    }

    /// A new preset from the screens as they are now.
    pub(super) fn look_create(&mut self, cx: &mut Context<Self>) {
        self.look_finish_rename(cx);
        let t = *cx.global::<Translator>();
        let number = self.look_list(cx).len() + 1;
        let mut look = capture(&self.screen_configs(cx), &uuid::Uuid::new_v4().to_string());
        look["title"] =
            json!(t.format("audienceLooks.newName", &[("number", &number.to_string())]));
        self.looks.editing_live = false;
        self.looks.draft = Some(look);
        self.look_save(cx);
    }

    pub(super) fn look_save(&mut self, cx: &mut Context<Self>) {
        let Some(mut look) = self.looks.draft.clone() else {
            return;
        };
        let (configs, workspace) = (
            self.screen_configs(cx),
            self.runtime.content.read(cx).workspace.clone(),
        );
        complete(&mut look, &configs);
        if let Some(error) = look_error(&look, &configs, &workspace["themes"], &workspace["masks"])
        {
            self.looks.error = Some(error.into());
            cx.notify();
            return;
        }
        look["title"] = json!(look["title"].as_str().unwrap_or("").trim());
        let mut list = self.look_list(cx);
        match list.iter_mut().find(|l| l["id"] == look["id"]) {
            Some(slot) => *slot = look.clone(),
            None => list.push(look.clone()),
        }
        self.looks_persist(list, Some(look), cx);
    }

    pub(super) fn look_delete(&mut self, id: &str, cx: &mut Context<Self>) {
        let list = self.look_list(cx);
        if !list.iter().any(|l| l["id"] == id) {
            self.looks.error = Some("Audience Look is unavailable.".into());
            cx.notify();
            return;
        }
        if crate::output::looks::live_id(&self.runtime, cx).as_deref() == Some(id) {
            self.runtime.output.update(cx, |output, cx| {
                output.audience_look = None;
                output.edit_live(|live| live["activeAudienceLookId"] = Value::Null);
                cx.notify();
            });
        }
        let list: Vec<Value> = list.into_iter().filter(|l| l["id"] != id).collect();
        let keep = list.first().cloned().map(|mut look| {
            complete(&mut look, &self.screen_configs(cx));
            look
        });
        self.looks.transitions.remove(id);
        self.looks.rename = None;
        self.looks.editing_live = false;
        self.looks_persist(list, keep, cx);
    }

    /// `activateAudienceLook`: screens the look would not change are skipped.
    pub(super) fn look_activate(&mut self, look: &Value, cx: &mut Context<Self>) {
        let (configs, workspace) = (
            self.screen_configs(cx),
            self.runtime.content.read(cx).workspace.clone(),
        );
        if let Some(error) = look_error(look, &configs, &workspace["themes"], &workspace["masks"]) {
            self.looks.error = Some(error.into());
            cx.notify();
            return;
        }
        match crate::output::looks::activate(&self.runtime, &configs, look, cx) {
            Ok(()) => self.looks.error = None,
            Err(error) => self.looks.error = Some(error),
        }
        cx.notify();
    }

    fn look_open_live(&mut self, cx: &mut Context<Self>) {
        self.look_finish_rename(cx);
        if crate::output::looks::live_look(&self.runtime, cx).is_none() {
            if let Some(mut look) = self.look_list(cx).into_iter().next() {
                complete(&mut look, &self.screen_configs(cx));
                self.look_activate(&look, cx);
            }
        }
        if let Some(look) =
            crate::output::looks::live_look(&self.runtime, cx).map(|active| active.current.clone())
        {
            self.look_select(&look, cx);
            self.looks.editing_live = true;
            cx.notify();
        }
    }

    fn look_reset_live(&mut self, cx: &mut Context<Self>) {
        let saved =
            crate::output::looks::live_look(&self.runtime, cx).map(|active| active.saved.clone());
        if let Some(mut saved) = saved {
            complete(&mut saved, &self.screen_configs(cx));
            self.look_activate(&saved, cx);
            self.look_open_live(cx);
        }
    }

    fn look_save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.looks.draft.clone() else {
            return;
        };
        let copy = duplicate(&draft, &uuid::Uuid::new_v4().to_string());
        self.looks.editing_live = false;
        self.looks.draft = Some(copy.clone());
        self.look_save(cx);
        self.look_activate(&copy, cx);
        self.look_rename(&copy, window, cx);
    }

    fn look_save_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let position = self.looks.save_menu?;
        let t = *cx.global::<Translator>();
        let mut entries = Vec::new();
        for (label, action) in [
            ("audienceLooks.save", "save"),
            ("audienceLooks.resetPreset", "reset"),
            ("audienceLooks.saveAs", "saveAs"),
        ] {
            let weak = cx.weak_entity();
            entries.push(
                MenuItem::new(t.t(label))
                    .on_click(move |_, window, cx| {
                        weak.update(cx, |view, cx| match action {
                            "save" => view.look_save(cx),
                            "reset" => view.look_reset_live(cx),
                            "saveAs" => view.look_save_as(window, cx),
                            _ => unreachable!(),
                        })
                        .ok();
                    })
                    .into(),
            );
        }
        let weak = cx.weak_entity();
        Some(
            ContextMenu::new("look-save-menu", position, entries, move |_, _, cx| {
                weak.update(cx, |view, cx| {
                    view.looks.save_menu = None;
                    cx.notify();
                })
                .ok();
            })
            .into_any_element(),
        )
    }

    fn transition_slider(
        &mut self,
        id: &str,
        seconds: f64,
        cx: &mut Context<Self>,
    ) -> Entity<SliderState> {
        if let Some(state) = self.looks.transitions.get(id) {
            return state.clone();
        }
        let state = range_state(
            0.,
            orbit_host::audience_looks::MAX_TRANSITION_SECONDS as f32,
            0.1,
            seconds as f32,
            cx,
        );
        cx.subscribe(&state, |this, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(value) = event else {
                return;
            };
            let seconds = (f64::from(value.end()) * 10.).round() / 10.;
            this.look_edit(|d| d["transitionSeconds"] = json!(seconds), cx);
        })
        .detach();
        self.looks.transitions.insert(id.to_string(), state.clone());
        state
    }

    fn presets_column(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let list = self.look_list(cx);
        let live = crate::output::looks::live_id(&self.runtime, cx);
        let live_title = crate::output::looks::live_look(&self.runtime, cx)
            .map(|active| {
                format!(
                    "{}{}",
                    active.current["title"].as_str().unwrap_or(""),
                    if active.current != active.saved {
                        "*"
                    } else {
                        ""
                    }
                )
            })
            .or_else(|| {
                list.iter()
                    .find(|look| look["id"].as_str() == live.as_deref())
                    .and_then(|look| look["title"].as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let editing = self
            .looks
            .draft
            .as_ref()
            .and_then(|d| d["id"].as_str())
            .map(str::to_string);
        let mut column = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(theme.metrics.space_lg * 9.)
            .border_r_1()
            .border_color(theme.colors.border)
            .child(
                list_row("looks-live", self.looks.editing_live, cx)
                    .debug_selector(|| "looks-live".into())
                    .justify_between()
                    .when(!self.looks.editing_live, |row| {
                        row.bg(theme.colors.control_bg)
                    })
                    .on_click(cx.listener(|view, _, _, cx| view.look_open_live(cx)))
                    .child(t.t("audienceLooks.live"))
                    .child(div().child(live_title)),
            )
            .child(
                PanelHeader::new(t.t("audienceLooks.presets"))
                    .contrast()
                    .action(
                        IconButton::new("look-create", "plus", t.t("audienceLooks.create"))
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.look_create(cx);
                                if let Some(look) = view.looks.draft.clone() {
                                    view.look_rename(&look, window, cx);
                                }
                            })),
                    ),
            );
        for look in &list {
            let id = look["id"].as_str().unwrap_or("").to_string();
            let pick = look.clone();
            let menu = look.clone();
            column = column.child(
                list_row(
                    SharedString::from(format!("look-row-{id}")),
                    !self.looks.editing_live && editing.as_deref() == Some(&id),
                    cx,
                )
                .when(
                    self.looks.editing_live || editing.as_deref() != Some(&id),
                    |row| row.bg(theme.colors.control_bg),
                )
                .debug_selector(move || format!("look-row-{id}"))
                .on_click(
                    cx.listener(move |view, event: &gpui::ClickEvent, window, cx| {
                        view.look_finish_rename(cx);
                        let pick = view
                            .look_list(cx)
                            .into_iter()
                            .find(|look| look["id"] == pick["id"])
                            .unwrap_or_else(|| pick.clone());
                        if event.click_count() >= 2 {
                            view.look_rename(&pick, window, cx);
                        } else {
                            view.look_select(&pick, cx);
                        }
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        view.looks.menu = Some((event.position, menu.clone()));
                        cx.notify();
                    }),
                )
                .child(look["title"].as_str().unwrap_or("").to_string()),
            );
        }
        column.into_any_element()
    }

    fn looks_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (position, look) = self.looks.menu.clone()?;
        let t = *cx.global::<Translator>();
        let mut entries = Vec::new();
        for (label, action) in [
            ("audienceLooks.makeLive", "live"),
            ("common.duplicate", "duplicate"),
            ("common.rename", "rename"),
            ("common.delete", "delete"),
        ] {
            let weak = cx.weak_entity();
            let look = look.clone();
            entries.push(
                MenuItem::new(t.t(label))
                    .on_click(move |_, window, cx| {
                        weak.update(cx, |view, cx| match action {
                            "live" => {
                                let mut look = look.clone();
                                complete(&mut look, &view.screen_configs(cx));
                                view.look_activate(&look, cx);
                            }
                            "duplicate" => {
                                view.looks.draft =
                                    Some(duplicate(&look, &uuid::Uuid::new_v4().to_string()));
                                view.look_save(cx);
                            }
                            "rename" => view.look_rename(&look, window, cx),
                            "delete" => view.look_delete(look["id"].as_str().unwrap_or(""), cx),
                            _ => unreachable!(),
                        })
                        .ok();
                    })
                    .into(),
            );
        }
        let weak = cx.weak_entity();
        Some(
            ContextMenu::new("look-menu", position, entries, move |_, _, cx| {
                weak.update(cx, |view, cx| {
                    view.looks.menu = None;
                    cx.notify();
                })
                .ok();
            })
            .into_any_element(),
        )
    }

    /// The grid of one preset: a column per audience screen, a row per layer or selector.
    fn looks_grid(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let draft = self.looks.draft.clone()?;
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let configs = self.screen_configs(cx);
        let workspace = self.runtime.content.read(cx).workspace.clone();
        let id = draft["id"].as_str().unwrap_or("").to_string();
        let screens = group_entries(&configs, "audience");
        let slides = theme_slide_options(&workspace["themes"]);
        let entries = draft["screenLooks"].as_array().cloned().unwrap_or_default();
        let none = || SelectOption::new(String::new(), t.t("audienceLooks.none"));
        let label_width = theme.metrics.space_lg * 8.;
        let column_width = theme.metrics.space_lg * 7.;

        let mut grid = div().flex().flex_col().gap(theme.metrics.space_xs).child(
            div()
                .flex()
                .items_end()
                .child(div().flex_none().w(label_width))
                .children(screens.iter().map(|(_, config)| {
                    div()
                        .flex_none()
                        .w(column_width)
                        .text_center()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.colors.text_strong)
                        .child(config["label"].as_str().unwrap_or("").to_string())
                })),
        );
        for row in ROWS {
            let mut line = div()
                .flex()
                .items_center()
                .min_h(theme.metrics.control_height_compact)
                .child(
                    div()
                        .flex_none()
                        .w(label_width)
                        .pr(theme.metrics.space_sm)
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(theme.metrics.space_snug)
                        .child(
                            icon(match row {
                                "maskId" => "circle-off",
                                "music" => "music",
                                "messages" => "send",
                                "props" => "stack-front",
                                "announcements" => "speakerphone",
                                "presentationThemeId" | "slide" => "notes",
                                "media" => "polaroid",
                                _ => "video",
                            })
                            .size(theme.metrics.space_lg),
                        )
                        .text_color(theme.colors.text)
                        .text_size(theme.metrics.font_sm)
                        .child(if row == "presentationThemeId" {
                            t.t("audienceLooks.presentation")
                        } else {
                            t.t(&format!(
                                "screenSettings.lookLayer.{}",
                                if row == "maskId" { "mask" } else { row }
                            ))
                        }),
                );
            for (screen, _) in &screens {
                let Some(index) = entries
                    .iter()
                    .position(|e| e["screenId"] == screen.as_str())
                else {
                    continue;
                };
                let entry = &entries[index];
                let weak = cx.weak_entity();
                let key = SharedString::from(format!("look-{id}-{screen}-{row}"));
                let selector = key.to_string();
                let cell: AnyElement = if row == "maskId" || row == "presentationThemeId" {
                    let mut options = vec![none()];
                    if row == "maskId" {
                        options.extend(workspace["masks"].as_array().into_iter().flatten().map(
                            |m| {
                                SelectOption::new(
                                    m["id"].as_str().unwrap_or("").to_string(),
                                    m["title"].as_str().unwrap_or("").to_string(),
                                )
                            },
                        ));
                    } else {
                        options.extend(
                            slides
                                .iter()
                                .map(|(v, l)| SelectOption::new(v.clone(), l.clone())),
                        );
                    }
                    Select::new(key, options, entry[row].as_str().unwrap_or("").to_string())
                        .compact()
                        .on_change(move |value, _, cx| {
                            weak.update(cx, |v, cx| {
                                v.look_edit(|d| d["screenLooks"][index][row] = json!(value), cx)
                            })
                            .ok();
                        })
                        .into_any_element()
                } else {
                    CheckboxField::new(
                        key,
                        format!(
                            "{} {}",
                            screen,
                            t.t(&format!("screenSettings.lookLayer.{row}"))
                        ),
                        entry["lookLayers"][row] == json!(true),
                    )
                    .bare()
                    .on_change(move |on, _, cx| {
                        weak.update(cx, |v, cx| {
                            v.look_edit(
                                |d| d["screenLooks"][index]["lookLayers"][row] = json!(on),
                                cx,
                            )
                        })
                        .ok();
                    })
                    .into_any_element()
                };
                line = line.child(
                    div()
                        .flex_none()
                        .w(column_width)
                        .debug_selector(move || selector)
                        .px(theme.metrics.space_sm)
                        .flex()
                        .justify_center()
                        .child(cell),
                );
            }
            grid = grid.child(line);
        }

        let seconds = orbit_host::audience_looks::transition_seconds(&draft);
        let slider = self.transition_slider(&id, seconds, cx);
        let shown = f64::from(slider.read(cx).value().end());
        let dirty = self.looks.editing_live
            && crate::output::looks::live_look(&self.runtime, cx)
                .is_some_and(|active| active.current != active.saved);
        let name = if let Some((input, _)) = &self.looks.rename {
            div()
                .id("look-rename-container")
                .on_mouse_down_out(cx.listener(|view, _, _, cx| view.look_finish_rename(cx)))
                .child(TextInput::new("look-rename", input).focus_ring(false))
                .into_any_element()
        } else {
            div()
                .id("look-title")
                .debug_selector(|| "look-title".into())
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.colors.text_strong)
                .cursor_pointer()
                .on_click(
                    cx.listener(move |view, event: &gpui::ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            if let Some(draft) = view.looks.draft.clone() {
                                let live = view.looks.editing_live;
                                view.look_rename(&draft, window, cx);
                                view.looks.editing_live = live;
                            }
                        }
                    }),
                )
                .child(format!(
                    "{}{}",
                    draft["title"].as_str().unwrap_or(""),
                    if dirty { "*" } else { "" }
                ))
                .into_any_element()
        };
        Some(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(theme.metrics.control_gap)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(theme.metrics.space_sm)
                        .child(div().flex_1().min_w_0().child(name))
                        .when(self.looks.editing_live, |row| {
                            let weak = cx.weak_entity();
                            row.child(
                                div()
                                    .flex_none()
                                    .relative()
                                    .child(
                                        gpui::canvas(
                                            move |bounds, _, cx| {
                                                weak.update(cx, |view, _| {
                                                    view.looks.save_anchor = Some(gpui::point(
                                                        bounds.left(),
                                                        bounds.bottom(),
                                                    ));
                                                })
                                                .ok();
                                            },
                                            |_, _, _, _| {},
                                        )
                                        .absolute()
                                        .size_full(),
                                    )
                                    .child(
                                        ActionButton::new(
                                            "look-save-options",
                                            t.t("audienceLooks.save"),
                                        )
                                        .compact()
                                        .variant(ButtonVariant::Secondary)
                                        .on_click(
                                            cx.listener(|view, _, _, cx| {
                                                view.looks.save_menu =
                                                    view.looks.save_anchor.map(|anchor| {
                                                        gpui::point(
                                                            anchor.x,
                                                            anchor.y + cx.theme().metrics.space_2xs,
                                                        )
                                                    });
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            )
                        }),
                )
                .child(grid)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(theme.metrics.space_sm)
                        .mt(theme.metrics.space_sm)
                        .pt(theme.metrics.space_sm)
                        .border_t_1()
                        .border_color(theme.colors.border)
                        .child(
                            icon("square-half")
                                .size(theme.metrics.space_lg)
                                .text_color(theme.colors.text),
                        )
                        .child(
                            div()
                                .debug_selector(|| "look-transition".into())
                                .w(theme.metrics.space_lg * 9.)
                                .child(
                                    RangeInput::new(&slider).label(t.t("audienceLooks.transition")),
                                ),
                        )
                        .child(
                            div()
                                .id("look-transition-value")
                                .text_size(theme.metrics.font_xs)
                                .child(seconds_label(shown)),
                        ),
                )
                .into_any_element(),
        )
    }

    fn sync_looks(&mut self, cx: &mut Context<Self>) {
        let path = self.runtime.content.read(cx).workspace_path.clone();
        if self.looks.workspace != path {
            self.looks = Looks {
                workspace: path.clone(),
                ..Default::default()
            };
            let list = self.look_list(cx);
            if list.is_empty() && !path.is_empty() {
                let mut look = capture(&self.screen_configs(cx), "default-look");
                look["title"] = json!("Default");
                self.looks_persist(vec![look.clone()], Some(look), cx);
            } else if let Some(look) = list
                .iter()
                .find(|look| {
                    look["id"].as_str()
                        == crate::output::looks::live_id(&self.runtime, cx).as_deref()
                })
                .or(list.first())
            {
                self.look_select(look, cx);
            }
        }
        if self.looks.editing_live {
            self.looks.draft = crate::output::looks::live_look(&self.runtime, cx)
                .map(|active| active.current.clone());
        }
        if let Some(mut draft) = self.looks.draft.clone() {
            complete(&mut draft, &self.screen_configs(cx));
            self.looks.draft = Some(draft);
        }
    }

    pub(super) fn looks_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        self.sync_looks(cx);
        let menu = self.looks_menu(cx);
        let save_menu = self.look_save_menu(cx);
        let presets = self.presets_column(cx);
        let grid = self.looks_grid(cx);
        let no_audience = group_entries(&self.screen_configs(cx), "audience").is_empty();
        div()
            .id("settings-audience-looks")
            .flex()
            .flex_col()
            .gap(theme.metrics.control_gap)
            .child(
                div()
                    .flex()
                    .gap(theme.metrics.space_sm)
                    .child(presets)
                    .children(grid),
            )
            .children(no_audience.then(|| muted(t.t("audienceLooks.noScreens"), cx)))
            .children(self.looks.error.clone().map(|e| {
                div()
                    .id("look-error")
                    .text_color(theme.colors.warning)
                    .child(e)
            }))
            .children(menu)
            .children(save_menu)
            .into_any_element()
    }
}
