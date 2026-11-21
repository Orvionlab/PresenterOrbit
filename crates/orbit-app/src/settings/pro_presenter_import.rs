//! The ProPresenter import dialogs: choose what to import from the scanned folder, then read the result.
//! The host reports diagnostics as flat `location: message` lines; `group_diagnostics` folds them by reason.

use std::collections::BTreeSet;

use gpui::{div, prelude::*, px, AnyElement, Context, Window};
use orbit_ui::controls::{
    ActionButton, BusyDialog, ButtonVariant, CheckboxField, Dialog, Disclosure, IconButton,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::ui::{path_for_display, subheading};
use super::view::SettingsView;

/// Categories besides libraries, in the order the dialogs list them. The keys are the host's.
pub const CATEGORIES: [&str; 12] = [
    "playlists",
    "media",
    "themes",
    "stageLayouts",
    "props",
    "masks",
    "macros",
    "messages",
    "groups",
    "timers",
    "presets",
    "settings",
];

/// Lines shown per group before "and N more".
const LOCATION_LIMIT: usize = 100;

type Toggle = Box<dyn Fn(bool, &mut SettingsView, &mut Context<SettingsView>)>;

pub struct Item {
    pub id: String,
    pub name: String,
    pub checked: bool,
}

/// A library or category. Without `items` (settings) it is all or nothing and `count` is shown.
pub struct Choice {
    pub key: String,
    pub count: usize,
    pub checked: bool,
    pub items: Vec<Item>,
    pub open: bool,
}

impl Choice {
    pub fn selected(&self) -> usize {
        if self.items.is_empty() {
            return usize::from(self.checked) * self.count;
        }
        self.items.iter().filter(|item| item.checked).count()
    }

    pub fn is_checked(&self) -> bool {
        if self.items.is_empty() {
            self.checked
        } else {
            self.selected() > 0
        }
    }

    pub fn is_mixed(&self) -> bool {
        !self.items.is_empty() && (1..self.items.len()).contains(&self.selected())
    }

    pub fn set_all(&mut self, checked: bool) {
        self.checked = checked;
        for item in &mut self.items {
            item.checked = checked;
        }
    }
}

fn items_from(list: &Value) -> Vec<Item> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(Item {
                id: item["id"].as_str()?.to_string(),
                name: item["name"].as_str()?.to_string(),
                checked: true,
            })
        })
        .collect()
}

pub enum ProImport {
    Select {
        path: String,
        libraries: Vec<Choice>,
        categories: Vec<Choice>,
    },
    Importing,
    Done(Report),
}

pub struct Report {
    pub counts: Vec<(&'static str, usize)>,
    pub presentations: usize,
    pub groups: Vec<Group>,
    /// Referenced files that are not on this computer, as the ProPresenter files name them.
    pub missing: Vec<String>,
    pub partial: usize,
    pub skipped: u64,
    pub open: BTreeSet<usize>,
}

#[derive(Debug, PartialEq)]
pub struct Group {
    pub reason: String,
    pub locations: Vec<String>,
}

/// The selection the host validates: checked libraries and categories, and the checked ids of
/// every list that has an unchecked item.
pub fn selection(libraries: &[Choice], categories: &[Choice]) -> Value {
    let checked = |choices: &[Choice]| {
        choices
            .iter()
            .filter(|choice| choice.is_checked())
            .map(|choice| json!(choice.key))
            .collect::<Vec<_>>()
    };
    let ids = |choices: &[&Choice]| {
        choices
            .iter()
            .flat_map(|choice| &choice.items)
            .filter(|item| item.checked)
            .map(|item| json!(item.id))
            .collect::<Vec<_>>()
    };
    let narrowed = |choice: &Choice| choice.is_mixed();
    let mut items = serde_json::Map::new();
    if libraries.iter().any(narrowed) {
        let chosen = libraries.iter().filter(|library| library.is_checked());
        items.insert(
            "presentations".into(),
            json!(ids(&chosen.collect::<Vec<_>>())),
        );
    }
    for category in categories.iter().filter(|category| narrowed(category)) {
        items.insert(category.key.clone(), json!(ids(&[category])));
    }
    json!({ "libraries": checked(libraries), "categories": checked(categories), "items": items })
}

pub fn choices_from_inventory(inventory: &Value) -> (Vec<Choice>, Vec<Choice>) {
    let libraries = inventory["libraries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|library| {
            let items = items_from(&library["presentations"]);
            Some(Choice {
                key: library["title"].as_str()?.to_string(),
                count: items.len(),
                checked: true,
                items,
                open: false,
            })
        })
        .collect();
    let categories = CATEGORIES
        .iter()
        .filter_map(|key| {
            let count = inventory["counts"][key].as_u64()? as usize;
            (count > 0).then(|| {
                let items = items_from(&inventory["items"][key]);
                Choice {
                    key: key.to_string(),
                    count,
                    checked: true,
                    items,
                    open: false,
                }
            })
        })
        .collect();
    (libraries, categories)
}

/// Folds `location: message` diagnostics by reason. A `ProPresenter content [..]` line holds one reason per
/// list entry; quoted names and file URLs are replaced so that equal problems share a group.
pub fn group_diagnostics(warnings: &[&str], root: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for warning in warnings {
        let (location, message) = warning.split_once(": ").unwrap_or(("", warning));
        let location = display_location(location, root);
        for reason in reasons(message) {
            match groups.iter_mut().find(|group| group.reason == reason) {
                Some(group) => group.locations.push(location.clone()),
                None => groups.push(Group {
                    reason,
                    locations: vec![location.clone()],
                }),
            }
        }
    }
    groups.sort_by_key(|group| std::cmp::Reverse(group.locations.len()));
    groups
}

fn reasons(message: &str) -> Vec<String> {
    let listed = message
        .strip_prefix("ProPresenter content ")
        .and_then(|rest| rest.strip_suffix(" was not imported"))
        .and_then(|list| serde_json::from_str::<Vec<String>>(list).ok());
    match listed {
        Some(list) if !list.is_empty() => list,
        _ => vec![generalize(message)],
    }
}

fn generalize(message: &str) -> String {
    if let Some((head, tail)) = message.rsplit_once(": ") {
        let bytes = tail.as_bytes();
        if tail.starts_with(r"\\") || (bytes.len() > 2 && bytes[1] == b':' && bytes[2] == b'\\') {
            return format!("{head}: …");
        }
    }
    let mut out = String::new();
    let mut quoted = false;
    for part in message.split('"') {
        if quoted {
            out.push('…');
        } else {
            out.push_str(part);
        }
        quoted = !quoted;
    }
    out.split(' ')
        .map(|word| {
            if word.starts_with("file://") {
                "…"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The path relative to the ProPresenter folder, without the verbatim prefix, using `/`.
fn display_location(location: &str, root: &str) -> String {
    let location = path_for_display(location).replace('\\', "/");
    let root = path_for_display(root).replace('\\', "/");
    let root = root.trim_end_matches('/');
    match location.get(..root.len()) {
        Some(head) if !root.is_empty() && head.eq_ignore_ascii_case(root) => {
            location[root.len()..].trim_start_matches('/').to_string()
        }
        _ => location,
    }
}

/// Files or records that lost something: the owner of a location is what precedes its `/slides` part.
fn partly_imported(groups: &[Group]) -> usize {
    groups
        .iter()
        .flat_map(|group| &group.locations)
        .map(|location| location.split("/slides").next().unwrap_or(location))
        .collect::<BTreeSet<_>>()
        .len()
}

/// The reference in a `media <reference> is unavailable on this system` diagnostic.
fn missing_file(warning: &str) -> Option<&str> {
    let (_, message) = warning.split_once(": ")?;
    let rest = message.strip_prefix("media ")?;
    Some(&rest[..rest.find(" is unavailable on this system")?])
}

pub fn report_from(workspace: &Value) -> Report {
    let (missing, warnings): (Vec<&str>, Vec<&str>) = workspace["importWarnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .partition(|warning| missing_file(warning).is_some());
    let mut files = Vec::new();
    for reference in missing.into_iter().filter_map(missing_file) {
        let file = path_for_display(reference);
        if !files.contains(&file) {
            files.push(file);
        }
    }
    let groups = group_diagnostics(&warnings, workspace["importedFrom"].as_str().unwrap_or(""));
    let len = |key: &str| workspace[key].as_array().map_or(0, Vec::len);
    let counts = [
        ("libraries", len("libraries")),
        ("playlists", len("playlist")),
        ("media", len("media")),
        (
            "themes",
            workspace["stats"]["importedThemeCount"]
                .as_u64()
                .unwrap_or(0) as usize,
        ),
        (
            "stageLayouts",
            workspace["stats"]["importedStageLayoutCount"]
                .as_u64()
                .unwrap_or(0) as usize,
        ),
        ("props", len("props")),
        (
            "masks",
            workspace["stats"]["importedMaskCount"]
                .as_u64()
                .unwrap_or(0) as usize,
        ),
        ("macros", len("macros")),
        ("messages", len("messages")),
        ("groups", len("groups")),
        ("timers", len("countdownTimers")),
        (
            "presets",
            len("mediaEffectPresets") + len("transitionPresets"),
        ),
        (
            "settings",
            len("triggerBindings") + len("communicationDevices") + len("clearGroups"),
        ),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .collect();
    Report {
        counts,
        presentations: len("library"),
        missing: files,
        partial: partly_imported(&groups),
        skipped: workspace["stats"]["skippedFileCount"].as_u64().unwrap_or(0),
        groups,
        open: BTreeSet::new(),
    }
}

impl ProImport {
    fn choices_mut(&mut self, library: bool, index: usize) -> Option<&mut Choice> {
        match self {
            ProImport::Select {
                libraries,
                categories,
                ..
            } => if library { libraries } else { categories }.get_mut(index),
            _ => None,
        }
    }
}

impl SettingsView {
    pub(super) fn pro_import_dialog(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let weak = cx.weak_entity();
        let close = {
            let weak = weak.clone();
            move |_, _: &mut Window, cx: &mut gpui::App| {
                weak.update(cx, |view, cx| {
                    view.services.pro_import = None;
                    cx.notify();
                })
                .ok();
            }
        };
        let max_height = window.viewport_size().height * 0.6;
        let scroll =
            |id: &'static str, body: gpui::Div| body.id(id).max_h(max_height).overflow_y_scroll();
        let column = || {
            div()
                .flex()
                .flex_col()
                .gap(theme.metrics.space_sm)
                .min_w_0()
        };
        match self.services.pro_import.as_ref()? {
            ProImport::Importing => Some(
                BusyDialog::new("pro-import-busy", t.t("status.import.reading")).into_any_element(),
            ),
            ProImport::Select {
                path,
                libraries,
                categories,
            } => {
                let row = |id: String, label: String, choice: &Choice, on: Toggle, open: Toggle| {
                    let (weak, open_weak) = (weak.clone(), weak.clone());
                    let count = if choice.items.is_empty() {
                        choice.count.to_string()
                    } else {
                        format!("{}/{}", choice.selected(), choice.items.len())
                    };
                    let chevron = if choice.items.is_empty() {
                        div().size(theme.metrics.control_height_compact).flex_none()
                    } else {
                        let label = t.t(if choice.open {
                            "settings.import.select.collapse"
                        } else {
                            "settings.import.select.expand"
                        });
                        let opened = !choice.open;
                        div().flex_none().child(
                            IconButton::new(
                                format!("{id}-open"),
                                if choice.open {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                label,
                            )
                            .on_click(move |_, _, cx| {
                                let _ = open_weak.update(cx, |view, cx| open(opened, view, cx));
                            }),
                        )
                    };
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(theme.metrics.space_md)
                        .child(
                            div().flex().items_center().min_w_0().child(chevron).child(
                                CheckboxField::new(
                                    id,
                                    label,
                                    choice.is_checked() && !choice.is_mixed(),
                                )
                                .mixed(choice.is_mixed())
                                .on_change(move |value, _, cx| {
                                    let _ = weak.update(cx, |view, cx| on(value, view, cx));
                                }),
                            ),
                        )
                        .child(
                            div()
                                .text_size(theme.metrics.font_sm)
                                .text_color(theme.colors.text_muted)
                                .child(count),
                        )
                };
                let item_list = |kind: &'static str, index: usize, choice: &Choice| {
                    let mut items = column().gap(px(0.));
                    for (position, item) in choice.items.iter().enumerate() {
                        let weak = weak.clone();
                        items = items.child(
                            CheckboxField::new(
                                format!("pro-import-{kind}-{index}-item-{position}"),
                                item.name.clone(),
                                item.checked,
                            )
                            .label_style(theme.metrics.font_sm, gpui::FontWeight::NORMAL)
                            .on_change(move |value, _, cx| {
                                let _ = weak.update(cx, |view, cx| {
                                    if let Some(choice) =
                                        view.services.pro_import.as_mut().and_then(|import| {
                                            import.choices_mut(kind == "library", index)
                                        })
                                    {
                                        choice.items[position].checked = value;
                                    }
                                    cx.notify();
                                });
                            }),
                        );
                    }
                    div()
                        .id(format!("pro-import-{kind}-{index}-items"))
                        .pl(theme.metrics.control_height_compact + theme.metrics.space_md)
                        .max_h(px(220.))
                        .overflow_y_scroll()
                        .child(items)
                };
                let group =
                    |kind: &'static str, index: usize, choice: &Choice, label: String| {
                        let toggle =
                            move |check: bool,
                                  view: &mut SettingsView,
                                  cx: &mut Context<SettingsView>| {
                                if let Some(choice) =
                                    view.services.pro_import.as_mut().and_then(|import| {
                                        import.choices_mut(kind == "library", index)
                                    })
                                {
                                    choice.set_all(check);
                                }
                                cx.notify();
                            };
                        let open =
                            move |open: bool,
                                  view: &mut SettingsView,
                                  cx: &mut Context<SettingsView>| {
                                if let Some(choice) =
                                    view.services.pro_import.as_mut().and_then(|import| {
                                        import.choices_mut(kind == "library", index)
                                    })
                                {
                                    choice.open = open;
                                }
                                cx.notify();
                            };
                        column()
                            .gap(px(0.))
                            .child(row(
                                format!("pro-import-{kind}-{index}"),
                                label,
                                choice,
                                Box::new(toggle),
                                Box::new(open),
                            ))
                            .when(choice.open, |group| {
                                group.child(item_list(kind, index, choice))
                            })
                    };
                let toggle = |select: bool, on: &'static str| {
                    let label = t.t(on);
                    ActionButton::new(format!("pro-import-all-{select}"), label)
                        .variant(ButtonVariant::Secondary)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if let Some(ProImport::Select {
                                libraries,
                                categories,
                                ..
                            }) = view.services.pro_import.as_mut()
                            {
                                for choice in libraries.iter_mut().chain(categories.iter_mut()) {
                                    choice.set_all(select);
                                }
                            }
                            cx.notify();
                        }))
                };
                let mut body = column().child(
                    div()
                        .text_size(theme.metrics.font_sm)
                        .text_color(theme.colors.text_muted)
                        .child(t.format(
                            "settings.import.select.source",
                            &[("path", &path_for_display(path))],
                        )),
                );
                let mut list = column().child(subheading(t.t("settings.import.libraries"), cx));
                for (index, library) in libraries.iter().enumerate() {
                    list = list.child(group("library", index, library, library.key.clone()));
                }
                body = body.child(list);
                let mut others =
                    column().child(subheading(t.t("settings.import.select.other"), cx));
                for (index, category) in categories.iter().enumerate() {
                    others = others.child(group(
                        "category",
                        index,
                        category,
                        t.t(&format!("settings.import.category.{}", category.key)),
                    ));
                }
                if !categories.is_empty() {
                    body = body.child(others);
                }
                let any = libraries.iter().chain(categories).any(Choice::is_checked);
                Some(
                    Dialog::new(
                        "pro-import-select",
                        t.t("settings.import.select.title"),
                        t.t("common.close"),
                        close,
                    )
                    .width(px(520.))
                    .close_on_backdrop(false)
                    .child(scroll("pro-import-select-scroll", body))
                    .footer(toggle(true, "settings.import.select.all"))
                    .footer(toggle(false, "settings.import.select.none"))
                    .footer(
                        ActionButton::new("pro-import-cancel", t.t("common.cancel"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.services.pro_import = None;
                                cx.notify();
                            })),
                    )
                    .footer(
                        ActionButton::new("pro-import-run", t.t("settings.import.select.confirm"))
                            .disabled(!any)
                            .on_click(cx.listener(|view, _, _, cx| view.run_pro_import(cx))),
                    )
                    .into_any_element(),
                )
            }
            ProImport::Done(report) => {
                let mut body = column();
                let mut counts =
                    column().child(subheading(t.t("settings.import.result.imported"), cx));
                let line = |label: String, count: usize| {
                    div()
                        .flex()
                        .justify_between()
                        .gap(theme.metrics.space_md)
                        .child(div().child(label))
                        .child(
                            div()
                                .text_color(theme.colors.text_muted)
                                .child(count.to_string()),
                        )
                };
                for (key, count) in &report.counts {
                    counts = counts.child(line(
                        t.t(&format!("settings.import.category.{key}")),
                        *count,
                    ));
                    if *key == "libraries" {
                        counts = counts.child(line(
                            t.t("settings.import.result.presentations"),
                            report.presentations,
                        ));
                    }
                }
                body = body.child(counts);
                if report.partial == 0 && report.skipped == 0 {
                    body = body.child(div().child(t.t("settings.import.result.clean")));
                } else {
                    body = body
                        .child(div().child(t.format(
                            "settings.import.result.partial",
                            &[("count", &report.partial.to_string())],
                        )))
                        .child(div().child(t.format(
                            "settings.import.result.skipped",
                            &[("count", &report.skipped.to_string())],
                        )));
                }
                if !report.missing.is_empty() {
                    let mut lines = column();
                    for file in report.missing.iter().take(LOCATION_LIMIT) {
                        lines = lines.child(
                            div()
                                .text_size(theme.metrics.font_sm)
                                .text_color(theme.colors.text_muted)
                                .child(file.clone()),
                        );
                    }
                    if report.missing.len() > LOCATION_LIMIT {
                        lines =
                            lines.child(div().text_size(theme.metrics.font_sm).child(t.format(
                                "settings.import.result.more",
                                &[(
                                    "count",
                                    &(report.missing.len() - LOCATION_LIMIT).to_string(),
                                )],
                            )));
                    }
                    body = body.child(
                        column()
                            .child(subheading(
                                t.format(
                                    "settings.import.result.missing",
                                    &[("count", &report.missing.len().to_string())],
                                ),
                                cx,
                            ))
                            .child(
                                div()
                                    .text_color(theme.colors.text_muted)
                                    .child(t.t("settings.import.result.missingHelp")),
                            )
                            .child(lines),
                    );
                }
                if !report.groups.is_empty() {
                    let mut details =
                        column().child(subheading(t.t("settings.import.result.details"), cx));
                    for (index, group) in report.groups.iter().enumerate() {
                        let open = report.open.contains(&index);
                        let weak = weak.clone();
                        let mut disclosure = Disclosure::new(
                            format!("pro-import-group-{index}"),
                            t.format(
                                "settings.import.result.group",
                                &[
                                    ("reason", &group.reason),
                                    ("count", &group.locations.len().to_string()),
                                ],
                            ),
                            open,
                        )
                        .on_toggle(move |open, _, cx| {
                            let _ = weak.update(cx, |view, cx| {
                                if let Some(ProImport::Done(report)) =
                                    view.services.pro_import.as_mut()
                                {
                                    if open {
                                        report.open.insert(index);
                                    } else {
                                        report.open.remove(&index);
                                    }
                                }
                                cx.notify();
                            });
                        });
                        if open {
                            let mut lines = column().pl(theme.metrics.space_lg);
                            for location in group.locations.iter().take(LOCATION_LIMIT) {
                                lines = lines.child(
                                    div()
                                        .text_size(theme.metrics.font_sm)
                                        .text_color(theme.colors.text_muted)
                                        .child(location.clone()),
                                );
                            }
                            if group.locations.len() > LOCATION_LIMIT {
                                lines = lines.child(div().text_size(theme.metrics.font_sm).child(
                                    t.format(
                                        "settings.import.result.more",
                                        &[(
                                            "count",
                                            &(group.locations.len() - LOCATION_LIMIT).to_string(),
                                        )],
                                    ),
                                ));
                            }
                            disclosure = disclosure.child(lines);
                        }
                        details = details.child(disclosure);
                    }
                    body = body.child(details).child(
                        div()
                            .text_size(theme.metrics.font_sm)
                            .text_color(theme.colors.text_muted)
                            .child(t.t("settings.import.result.baseline")),
                    );
                }
                Some(
                    Dialog::new(
                        "pro-import-result",
                        t.t("settings.import.result.title"),
                        t.t("common.close"),
                        close,
                    )
                    .width(px(640.))
                    .close_on_backdrop(false)
                    .child(scroll("pro-import-result-scroll", body))
                    .footer(
                        ActionButton::new("pro-import-done", t.t("common.close")).on_click(
                            cx.listener(|view, _, _, cx| {
                                view.services.pro_import = None;
                                cx.notify();
                            }),
                        ),
                    )
                    .into_any_element(),
                )
            }
        }
    }
}
