use super::*;
use orbit_ui::controls::{
    data_color, list_row, ButtonVariant, CheckboxField, ColorPicker, ContextMenu, Field,
    InputEvent, MenuEntry, MenuItem,
};

const MARKER_COLORS: [&str; 6] = [
    "#59bbff", "#ffb449", "#b894ff", "#66d7a4", "#ff83ab", "#54d7d7",
];

#[derive(Clone, Copy, PartialEq)]
pub(super) enum MarkerEditor {
    Name,
    Color,
    Time,
    Actions,
}

#[derive(Clone, Copy)]
enum MarkerCommand {
    Edit(MarkerEditor),
    Move,
    Repeat,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
    AddAction(&'static str),
}

impl MediaBin {
    pub(super) fn marker_patch(
        &mut self,
        index: usize,
        action: Option<usize>,
        field: &str,
        value: Value,
        cx: &mut Context<Self>,
    ) {
        let default_transport = if field.starts_with("details.mediaTransport.") {
            self.marker_draft.as_ref().and_then(|draft| draft.get(index)).and_then(|marker| action.and_then(|index| marker["actions"].get(index))).and_then(|action| self.media.iter().find(|item| item["id"] == action["targetId"])).and_then(|item| item.get("mediaTransport")).cloned().unwrap_or_else(|| json!({"inSeconds":0,"outSeconds":null,"rate":1,"delaySeconds":0,"retrigger":"restart"}))
        } else {
            Value::Null
        };
        let Some(marker) = self
            .marker_draft
            .as_mut()
            .and_then(|draft| draft.get_mut(index))
        else {
            return;
        };
        if let Some(action) = action {
            if field.starts_with("details.mediaTransport.")
                && marker["actions"][action]["details"]["mediaTransport"].is_null()
            {
                marker["actions"][action]["details"]["mediaTransport"] = default_transport;
            }
            let mut target = &mut marker["actions"][action];
            let mut parts = field.split('.').peekable();
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    if value.is_null() && field != "details.mediaTransport.outSeconds" {
                        if let Some(object) = target.as_object_mut() {
                            object.remove(part);
                        }
                    } else {
                        target[part] = value;
                    }
                    break;
                }
                if target[part].is_null() {
                    target[part] = json!({});
                }
                let Some(next) = target.get_mut(part).filter(|next| next.is_object()) else {
                    return;
                };
                target = next;
            }
        } else {
            marker[field] = value;
        }
        self.save_markers(cx);
    }
    pub(super) fn marker_field(
        &mut self,
        (index, action, field): (usize, Option<usize>, &'static str),
        label: &'static str,
        value: Value,
        numeric: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = format!("marker-{index}-{action:?}-{field}");
        let selector = key.clone();
        let input = if let Some((input, _)) = self.inputs.get(&key) {
            input.clone()
        } else {
            let input = input_state(
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string()),
                window,
                cx,
            );
            let owner = self.selected.clone();
            let subscription = cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, _, cx| {
                    if this.selected != owner
                        || !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                    {
                        return;
                    }
                    let text = input.read(cx).value().to_string();
                    let value = if numeric && text.trim().is_empty() {
                        Value::Null
                    } else if numeric {
                        text.parse::<f64>()
                            .ok()
                            .filter(|value| value.is_finite())
                            .map(|value| json!(value))
                            .unwrap_or_else(|| json!(text))
                    } else {
                        json!(text)
                    };
                    this.marker_patch(index, action, field, value, cx);
                },
            );
            if action.is_none() && field == "title" {
                input.update(cx, |state, cx| {
                    state.focus(window, cx);
                    state.select_all(window, cx);
                });
            }
            self.inputs
                .insert(key.clone(), (input.clone(), subscription));
            input
        };
        Field::new(
            cx.global::<Translator>().t(label),
            div().debug_selector(move || selector.clone()).child(
                TextInput::new(SharedString::from(key), &input)
                    .compact()
                    .disabled(self.edits_blocked()),
            ),
        )
        .inline()
        .into_any_element()
    }
    pub(super) fn save_markers(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.media.iter().find(|item| item["id"] == self.selected) else {
            return;
        };
        let current = item.get("markers").cloned().unwrap_or_else(|| json!([]));
        let Some(draft) = self.marker_draft.clone() else {
            return;
        };
        if self.marker_baseline.as_ref() != Some(&current) {
            self.marker_status = "media.markers.failed".into();
            cx.notify();
            return;
        }
        if !valid_markers(&draft, item) || !self.marker_actions_valid(&draft) {
            self.marker_status = "media.markers.incomplete".into();
            cx.notify();
            return;
        }
        if current != draft {
            self.marker_baseline = Some(draft.clone());
            self.patch_asset("markers", draft, cx);
        }
        self.marker_status.clear();
        cx.notify();
    }

    pub(super) fn reset_marker_editor(&mut self) {
        self.marker_draft = None;
        self.marker_baseline = None;
        self.marker_status.clear();
        self.marker_menu = None;
        self.marker_editor = None;
        self.selected_marker = None;
    }

    fn marker_command(&mut self, id: Option<&str>, command: MarkerCommand, cx: &mut Context<Self>) {
        let Some(markers) = self.marker_draft.as_mut().and_then(Value::as_array_mut) else {
            return;
        };
        self.marker_menu = None;
        let index = id.and_then(|id| markers.iter().position(|marker| marker["id"] == id));
        if matches!(command, MarkerCommand::Paste) {
            if let Some(mut copy) = self.marker_clipboard.clone() {
                renew_marker_ids(&mut copy);
                copy["timeSeconds"] = json!(self.preview_position);
                self.selected_marker = copy["id"].as_str().map(str::to_owned);
                markers.insert(index.map_or(markers.len(), |index| index + 1), copy);
                self.inputs.retain(|key, _| !key.starts_with("marker-"));
                self.save_markers(cx);
            }
            return;
        }
        let Some((id, index)) = id.zip(index) else {
            return;
        };
        match command {
            MarkerCommand::Edit(editor) => {
                self.inputs.retain(|key, _| !key.starts_with("marker-"));
                self.marker_editor = Some((id.to_owned(), editor));
                cx.notify();
                return;
            }
            MarkerCommand::Move => markers[index]["timeSeconds"] = json!(self.preview_position),
            MarkerCommand::Repeat => {
                let repeat = markers[index]["repeatOnLoop"] != false;
                markers[index]["repeatOnLoop"] = json!(!repeat);
            }
            MarkerCommand::Copy | MarkerCommand::Cut => {
                self.marker_clipboard = Some(markers[index].clone());
                if matches!(command, MarkerCommand::Copy) {
                    cx.notify();
                    return;
                }
                markers.remove(index);
            }
            MarkerCommand::Duplicate => {
                let mut copy = markers[index].clone();
                renew_marker_ids(&mut copy);
                self.selected_marker = copy["id"].as_str().map(str::to_owned);
                markers.insert(index + 1, copy);
            }
            MarkerCommand::Paste => return,
            MarkerCommand::Delete => {
                markers.remove(index);
            }
            MarkerCommand::AddAction(kind) => {
                let Some(actions) = markers[index]["actions"].as_array_mut() else {
                    return;
                };
                let mut action = json!({"id":uuid::Uuid::new_v4().to_string(),"type":kind,"enabled":true,"delaySeconds":0});
                if kind == "clear" {
                    action["targetLayer"] = json!("slide");
                }
                if kind == "capture" {
                    action["details"] = json!({"start":{"presetIdentification":""}});
                }
                actions.push(action);
                self.marker_editor = Some((id.to_owned(), MarkerEditor::Actions));
            }
        }
        self.inputs.retain(|key, _| !key.starts_with("marker-"));
        self.save_markers(cx);
    }

    fn marker_menu(
        &self,
        draft: &Value,
        item: &Value,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let (id, point) = self.marker_menu.as_ref()?;
        let marker = id
            .as_ref()
            .and_then(|id| draft.as_array()?.iter().find(|marker| marker["id"] == *id));
        let t = *cx.global::<Translator>();
        let owner = cx.weak_entity();
        let action = |label: String, command, disabled| {
            let owner = owner.clone();
            let id = id.clone();
            MenuItem::new(label)
                .disabled(disabled)
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.marker_command(id.as_deref(), command, cx)
                    });
                })
        };
        let media_kind = |kind| matches!(kind, "media" | "video-input" | "audio-input");
        let actions = |media| {
            ACTION_TYPES
                .iter()
                .copied()
                .filter(|kind| media_kind(*kind) == media)
                .map(|kind| {
                    action(
                        t.t(&format!("media.markerAction.{kind}")),
                        MarkerCommand::AddAction(kind),
                        false,
                    )
                    .into()
                })
                .collect()
        };
        let disabled = self.edits_blocked();
        let paste = action(
            t.t("common.paste"),
            MarkerCommand::Paste,
            disabled || self.marker_clipboard.is_none(),
        );
        let mut entries: Vec<MenuEntry> = Vec::new();
        if let Some(marker) = marker {
            entries.extend([
                action(
                    t.t("common.rename"),
                    MarkerCommand::Edit(MarkerEditor::Name),
                    disabled,
                )
                .into(),
                action(
                    t.t("media.markers.changeColor"),
                    MarkerCommand::Edit(MarkerEditor::Color),
                    disabled,
                )
                .into(),
                action(
                    t.t("media.markers.moveToPlayhead"),
                    MarkerCommand::Move,
                    disabled,
                )
                .into(),
            ]);
            entries.push(
                action(
                    t.t("media.markers.editTime"),
                    MarkerCommand::Edit(MarkerEditor::Time),
                    disabled,
                )
                .into(),
            );
            if allows_repeat(item) {
                entries.push(
                    action(
                        t.t("media.markers.repeatOnLoop"),
                        MarkerCommand::Repeat,
                        disabled,
                    )
                    .checked(marker["repeatOnLoop"] != false)
                    .into(),
                );
            }
            entries.extend([
                MenuEntry::Separator,
                MenuEntry::Submenu {
                    label: t.t("automation.macros.addAction").into(),
                    disabled,
                    entries: actions(false),
                },
                MenuEntry::Submenu {
                    label: t.t("media.markers.addMediaAction").into(),
                    disabled,
                    entries: actions(true),
                },
                action(
                    t.t("media.markers.editActions"),
                    MarkerCommand::Edit(MarkerEditor::Actions),
                    disabled || marker["actions"].as_array().is_none_or(Vec::is_empty),
                )
                .into(),
                MenuEntry::Separator,
                action(t.t("common.cut"), MarkerCommand::Cut, disabled).into(),
                action(t.t("common.copy"), MarkerCommand::Copy, false).into(),
                paste.into(),
                action(t.t("common.duplicate"), MarkerCommand::Duplicate, disabled).into(),
                action(t.t("common.delete"), MarkerCommand::Delete, disabled).into(),
            ]);
        } else {
            entries.push(paste.into());
        }
        Some(
            ContextMenu::new("media-marker-context", *point, entries, move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.marker_menu = None;
                    cx.notify();
                });
            })
            .into_any_element(),
        )
    }

    pub(super) fn markers(
        &mut self,
        item: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = *cx.global::<Translator>();
        let original = item.get("markers").cloned().unwrap_or_else(|| json!([]));
        self.marker_baseline.get_or_insert_with(|| original.clone());
        let draft = self.marker_draft.get_or_insert(original).clone();
        let allow_repeat = allows_repeat(item);
        let theme = *cx.theme();
        let mut body = div()
            .id("media-markers-list")
            .min_h(theme.metrics.row_height * 5.)
            .flex()
            .flex_col()
            .gap(theme.metrics.space_xs)
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    this.marker_menu = Some((None, event.position));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                ActionButton::new("marker-add", t.t("media.markers.add"))
                    .compact()
                    .disabled(self.edits_blocked())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut marker = new_marker(
                            cx.global::<Translator>().t("media.markers.new"),
                            allow_repeat,
                        );
                        let position = this.preview_position;
                        marker["timeSeconds"] = json!(position);
                        if let Some(item) =
                            this.media.iter().find(|item| item["id"] == this.selected)
                        {
                            let transport = &item["mediaTransport"];
                            if position < transport["inSeconds"].as_f64().unwrap_or(0.)
                                || transport["outSeconds"]
                                    .as_f64()
                                    .is_some_and(|end| position >= end)
                            {
                                marker["repeatOnLoop"] = json!(false);
                            }
                        }
                        this.selected_marker = marker["id"].as_str().map(str::to_owned);
                        if let Some(markers) =
                            this.marker_draft.as_mut().and_then(Value::as_array_mut)
                        {
                            let last = markers
                                .last()
                                .and_then(|marker| marker["color"].as_str())
                                .and_then(|color| {
                                    MARKER_COLORS.iter().position(|entry| *entry == color)
                                });
                            let first = last.map_or(0, |index| (index + 1) % MARKER_COLORS.len());
                            let color = MARKER_COLORS
                                .iter()
                                .cycle()
                                .skip(first)
                                .take(MARKER_COLORS.len())
                                .find(|color| {
                                    !markers.iter().any(|marker| marker["color"] == **color)
                                })
                                .copied()
                                .unwrap_or(MARKER_COLORS[first]);
                            marker["color"] = json!(color);
                            markers.push(marker);
                            this.save_markers(cx);
                        }
                    })),
            );
        for (index, marker) in draft.as_array().into_iter().flatten().enumerate() {
            let id = text(marker, "id").to_owned();
            let menu_id = id.clone();
            let row_id = id.clone();
            let label = text(marker, "title").to_owned();
            let time = marker["timeSeconds"].as_f64().unwrap_or(0.);
            let color = marker["color"]
                .as_str()
                .map(data_color)
                .unwrap_or(theme.colors.accent);
            let selected = self.selected_marker.as_deref() == Some(id.as_str());
            let selector = format!("media-marker-{index}");
            body = body.child(
                list_row(
                    SharedString::from(format!("media-marker-{id}")),
                    selected,
                    cx,
                )
                .debug_selector(move || selector.clone())
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        this.selected_marker = Some(menu_id.clone());
                        this.marker_menu = Some((Some(menu_id.clone()), event.position));
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected_marker = Some(row_id.clone());
                    this.preview_seek(time, cx);
                }))
                .child(orbit_ui::icon("media-marker-tag").text_color(color))
                .child(crate::automation::form::format_time(time))
                .child(div().min_w_0().truncate().child(label)),
            );
            let editor = self
                .marker_editor
                .as_ref()
                .and_then(|(owner, editor)| (owner == &id).then_some(*editor));
            match editor {
                Some(MarkerEditor::Name) => {
                    body = body.child(self.marker_field(
                        (index, None, "title"),
                        "media.markers.title",
                        marker["title"].clone(),
                        false,
                        window,
                        cx,
                    ));
                    continue;
                }
                Some(MarkerEditor::Time) => {
                    body = body.child(self.marker_field(
                        (index, None, "timeSeconds"),
                        "media.markers.time",
                        marker["timeSeconds"].clone(),
                        true,
                        window,
                        cx,
                    ));
                    continue;
                }
                Some(MarkerEditor::Color) => {
                    let weak = cx.weak_entity();
                    body = body.child(
                        ColorPicker::new(
                            SharedString::from(format!("marker-color-{index}")),
                            t.t("theme.color"),
                            marker["color"].as_str().unwrap_or("#ffaa00").to_owned(),
                        )
                        .on_change(move |color, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.marker_patch(index, None, "color", json!(color), cx)
                            });
                        }),
                    );
                    continue;
                }
                Some(MarkerEditor::Actions) => {}
                None => continue,
            }
            for (action_index, action) in marker["actions"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                body =
                    body.child(self.marker_action_fields(index, action_index, action, window, cx));
                let weak = cx.weak_entity();
                body = body.child(
                    CheckboxField::new(
                        SharedString::from(format!("marker-action-enabled-{index}-{action_index}")),
                        t.t("automation.macros.enabled"),
                        action["enabled"] != false,
                    )
                    .disabled(self.edits_blocked())
                    .on_change(move |value, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.marker_patch(
                                index,
                                Some(action_index),
                                "enabled",
                                json!(value),
                                cx,
                            )
                        });
                    }),
                );
                for (offset, key) in [
                    (-1, "automation.macros.up"),
                    (1, "automation.macros.down"),
                    (0, "automation.macros.remove"),
                ] {
                    body = body.child(
                        ActionButton::new(
                            SharedString::from(format!("action-{index}-{action_index}-{offset}")),
                            t.t(key),
                        )
                        .variant(ButtonVariant::Ghost)
                        .compact()
                        .disabled(self.edits_blocked())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(actions) = this
                                .marker_draft
                                .as_mut()
                                .and_then(|draft| draft.get_mut(index))
                                .and_then(|marker| marker["actions"].as_array_mut())
                            {
                                if offset == 0 {
                                    actions.remove(action_index);
                                } else if let Some(target) = action_index
                                    .checked_add_signed(offset)
                                    .filter(|target| *target < actions.len())
                                {
                                    actions.swap(action_index, target);
                                }
                                this.inputs.retain(|key, _| !key.starts_with("marker-"));
                                this.save_markers(cx);
                            }
                        })),
                    );
                }
            }
        }
        body = body.children(self.marker_menu(&draft, item, cx));
        if !self.marker_status.is_empty() {
            body = body.child(
                div()
                    .id("marker-status")
                    .role(gpui::Role::Status)
                    .child(t.t(&self.marker_status)),
            );
        }
        body.into_any_element()
    }
}
fn renew_marker_ids(marker: &mut Value) {
    marker["id"] = json!(uuid::Uuid::new_v4().to_string());
    if let Some(actions) = marker["actions"].as_array_mut() {
        for action in actions {
            action["id"] = json!(uuid::Uuid::new_v4().to_string());
        }
    }
}

pub(super) const ACTION_TYPES: &[&str] = &[
    "clear",
    "clear-group",
    "macro",
    "media",
    "video-input",
    "audio-input",
    "message",
    "presentation",
    "playlist",
    "prop",
    "stage",
    "audience-look",
    "mask",
    "timer",
    "transport",
    "communication",
    "capture",
];
fn valid_markers(draft: &Value, item: &Value) -> bool {
    let Some(markers) = draft.as_array() else {
        return false;
    };
    let mut ids = std::collections::HashSet::new();
    markers.iter().all(|marker| {
        let id = text(marker, "id");
        let time = marker["timeSeconds"].as_f64();
        let mut action_ids = std::collections::HashSet::new();
        !id.trim().is_empty()
            && ids.insert(id)
            && !text(marker, "title").trim().is_empty()
            && time.is_some_and(|time| {
                time.is_finite()
                    && time >= 0.
                    && (marker["repeatOnLoop"] == false
                        || item["mediaTransport"].is_null()
                        || (time >= item["mediaTransport"]["inSeconds"].as_f64().unwrap_or(0.)
                            && item["mediaTransport"]["outSeconds"]
                                .as_f64()
                                .is_none_or(|end| time < end)))
            })
            && marker.get("color").is_none_or(|color| {
                color.as_str().is_some_and(|color| {
                    color.len() == 7
                        && color.starts_with('#')
                        && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            })
            && marker.get("repeatOnLoop").is_none_or(Value::is_boolean)
            && marker["actions"].as_array().is_some_and(|actions| {
                actions.iter().all(|action| {
                    !text(action, "id").trim().is_empty()
                        && action_ids.insert(text(action, "id"))
                        && action.get("delaySeconds").is_none_or(|delay| {
                            delay
                                .as_f64()
                                .is_some_and(|delay| delay.is_finite() && delay >= 0.)
                        })
                })
            })
    })
}

/// React offers the repeat-on-loop switch only for video and for audio that loops.
pub(super) fn allows_repeat(item: &Value) -> bool {
    item["type"] == "video" || item["audioPlayback"] == "loop"
}

pub(super) fn new_marker(title: String, allow_repeat: bool) -> Value {
    json!({"id":uuid::Uuid::new_v4().to_string(),"title":title,"timeSeconds":0,"repeatOnLoop":allow_repeat,"actions":[]})
}
