use super::*;
use orbit_ui::controls::{CheckboxField, ColorPicker, Field, InputEvent};

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
        self.marker_status.clear();
        cx.notify();
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
                    if this.selected != owner || !matches!(event, InputEvent::Change) {
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
            self.inputs
                .insert(key.clone(), (input.clone(), subscription));
            input
        };
        Field::new(
            cx.global::<Translator>().t(label),
            TextInput::new(SharedString::from(key), &input)
                .compact()
                .disabled(self.edits_blocked()),
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
        if self.marker_baseline.as_ref() != Some(&current)
            || !valid_markers(&draft, item)
            || !self.marker_actions_valid(&draft)
        {
            self.marker_status = "media.markers.failed".into();
            cx.notify();
            return;
        }
        self.patch_asset("markers", draft, cx);
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
        let mut body = div().flex().flex_col().gap(cx.theme().metrics.space_xs);
        for (index, marker) in draft.as_array().into_iter().flatten().enumerate() {
            for (field, label, numeric) in [
                ("title", "media.markers.title", false),
                ("timeSeconds", "media.markers.time", true),
            ] {
                body = body.child(self.marker_field(
                    (index, None, field),
                    label,
                    marker[field].clone(),
                    numeric,
                    window,
                    cx,
                ));
            }
            let weak = cx.weak_entity();
            body = body.child(
                ColorPicker::new(
                    SharedString::from(format!("marker-color-{index}")),
                    t.t("theme.color"),
                    marker["color"].as_str().unwrap_or("#ffaa00").to_string(),
                )
                .on_change(move |color, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.marker_patch(index, None, "color", json!(color), cx)
                    });
                }),
            );
            if allow_repeat {
                let weak = cx.weak_entity();
                body = body.child(
                    CheckboxField::new(
                        SharedString::from(format!("marker-repeat-{index}")),
                        t.t("media.markers.repeatOnLoop"),
                        marker["repeatOnLoop"] != false,
                    )
                    .disabled(self.edits_blocked())
                    .on_change(move |value, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.marker_patch(index, None, "repeatOnLoop", json!(value), cx)
                        });
                    }),
                );
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
                                this.inputs.clear();
                                cx.notify();
                            }
                        })),
                    );
                }
            }
            body = body.child(ActionButton::new(SharedString::from(format!("marker-add-action-{index}")), t.t("automation.macros.addAction")).disabled(self.edits_blocked()).on_click(cx.listener(move |this, _, _, cx| {
                if let Some(actions) = this.marker_draft.as_mut().and_then(|draft| draft.get_mut(index)).and_then(|marker| marker["actions"].as_array_mut()) { actions.push(json!({"id":uuid::Uuid::new_v4().to_string(),"type":"clear","targetLayer":"slide","enabled":true,"delaySeconds":0})); cx.notify(); }
            })));
            body = body.child(
                ActionButton::new(
                    SharedString::from(format!("marker-remove-{index}")),
                    t.t("automation.macros.remove"),
                )
                .disabled(self.edits_blocked())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(markers) = this.marker_draft.as_mut().and_then(Value::as_array_mut)
                    {
                        markers.remove(index);
                        this.inputs.clear();
                        cx.notify();
                    }
                })),
            );
        }
        body = body
            .child(
                ActionButton::new("marker-add", t.t("media.markers.add"))
                    .disabled(self.edits_blocked())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(markers) =
                            this.marker_draft.as_mut().and_then(Value::as_array_mut)
                        {
                            markers.push(new_marker(
                                cx.global::<Translator>().t("media.markers.title"),
                                allow_repeat,
                            ));
                            cx.notify();
                        }
                    })),
            )
            .child(
                ActionButton::new("markers-save", t.t("automation.macros.save"))
                    .disabled(self.edits_blocked())
                    .on_click(cx.listener(|this, _, _, cx| this.save_markers(cx))),
            );
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
