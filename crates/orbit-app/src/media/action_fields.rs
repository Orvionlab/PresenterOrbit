use super::*;
use orbit_ui::controls::{CheckboxField, Field};
impl MediaBin {
    fn action_choices(
        &mut self,
        (marker, index): (usize, usize),
        field: &'static str,
        label: &str,
        value: &str,
        choices: Vec<SelectOption<String>>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let weak = cx.weak_entity();
        Field::new(
            cx.global::<Translator>().t(label),
            Select::new(
                SharedString::from(format!("action-choice-{marker}-{index}-{field}")),
                choices,
                value.to_string(),
            )
            .compact()
            .disabled(self.edits_blocked())
            .on_change(move |value, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.marker_patch(marker, Some(index), field, json!(value), cx);
                    this.inputs.clear();
                });
            }),
        )
        .inline()
        .into_any_element()
    }
    fn action_check(
        &self,
        marker: usize,
        index: usize,
        field: &'static str,
        label: &str,
        value: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let weak = cx.weak_entity();
        CheckboxField::new(
            SharedString::from(format!("action-check-{marker}-{index}-{field}")),
            cx.global::<Translator>().t(label),
            value,
        )
        .disabled(self.edits_blocked())
        .on_change(move |value, _, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.marker_patch(marker, Some(index), field, json!(value), cx)
            });
        })
        .into_any_element()
    }
    pub(super) fn marker_action_fields(
        &mut self,
        marker: usize,
        index: usize,
        action: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let kind = text(action, "type");
        let mut body = div().flex().flex_col().gap(cx.theme().metrics.space_xs);
        let weak = cx.weak_entity();
        body = body.child(Field::new(
            cx.global::<Translator>().t("automation.macros.type"),
            Select::new(
                SharedString::from(format!("action-type-{marker}-{index}")),
                markers::ACTION_TYPES
                    .iter()
                    .map(|kind| SelectOption::new(kind.to_string(), kind.to_string()))
                    .collect(),
                kind.to_string(),
            )
            .disabled(self.edits_blocked())
            .on_change(move |kind, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if let Some(action) = this
                        .marker_draft
                        .as_mut()
                        .and_then(|draft| draft.get_mut(marker))
                        .and_then(|marker| marker["actions"].get_mut(index))
                    {
                        let mut next = json!({"id":action["id"],"type":kind});
                        for key in ["enabled", "delaySeconds"] {
                            if let Some(value) = action.get(key) {
                                next[key] = value.clone();
                            }
                        }
                        if kind == "capture" {
                            next["details"] = json!({"start":{"presetIdentification":""}});
                        }
                        *action = next;
                        this.inputs.clear();
                        cx.notify();
                    }
                });
            }),
        ));
        if let Some(collection) = action_collection(kind) {
            let mut options = vec![SelectOption::new(
                String::new(),
                cx.global::<Translator>()
                    .t("automation.macros.chooseTarget"),
            )];
            let entries = self.source[collection]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for item in &entries {
                options.push(SelectOption::new(
                    text(item, "id").to_string(),
                    item["title"]
                        .as_str()
                        .or_else(|| item["name"].as_str())
                        .unwrap_or_else(|| text(item, "id"))
                        .to_string(),
                ));
            }
            let target = text(action, "targetId");
            if !target.is_empty() && !entries.iter().any(|item| item["id"] == target) {
                options.push(SelectOption::new(
                    target.to_string(),
                    format!(
                        "{} ({})",
                        target,
                        cx.global::<Translator>()
                            .t("automation.macros.targetInvalid")
                    ),
                ));
            }
            body = body.child(self.action_choices(
                (marker, index),
                "targetId",
                "automation.macros.target",
                target,
                options,
                cx,
            ));
        }
        if kind == "clear" {
            body = body.child(self.action_choices(
                (marker, index),
                "targetLayer",
                "automation.macros.target",
                action["targetLayer"].as_str().unwrap_or("all"),
                choices(&[
                    "all",
                    "slide",
                    "background",
                    "prop",
                    "audio",
                    "messages",
                    "video-input",
                ]),
                cx,
            ));
        }
        if matches!(kind, "timer" | "prop") {
            let options: &[&str] = if kind == "timer" {
                &[
                    "start",
                    "stop",
                    "reset",
                    "reset-and-start",
                    "stop-and-reset",
                    "increment",
                ]
            } else {
                &["show", "clear"]
            };
            body = body.child(self.action_choices(
                (marker, index),
                "value",
                "automation.macros.command",
                action["value"].as_str().unwrap_or(options[0]),
                choices(options),
                cx,
            ));
        }
        if kind == "timer" && action["value"] == "increment" {
            body = body.child(self.marker_field(
                (marker, Some(index), "details.incrementSeconds"),
                "automation.macros.increment",
                action["details"]["incrementSeconds"].clone(),
                true,
                window,
                cx,
            ));
        }
        if kind == "transport" {
            body = body
                .child(self.action_check(
                    marker,
                    index,
                    "details.play",
                    "automation.macros.play",
                    action["details"]["play"] == true,
                    cx,
                ))
                .child(self.action_check(
                    marker,
                    index,
                    "details.pause",
                    "automation.macros.pause",
                    action["details"]["pause"] == true,
                    cx,
                ));
            body = body.child(self.marker_field(
                (marker, Some(index), "details.jumpToTime.time"),
                "media.markers.time",
                action["details"]["jumpToTime"]["time"].clone(),
                true,
                window,
                cx,
            ));
        }
        if kind == "audio-input" {
            body = body.child(self.action_choices(
                (marker, index),
                "details.behaviorMode",
                "audioInputs.mode",
                text(&action["details"], "behaviorMode"),
                choices(&["", "off", "on", "auto-off", "auto-on"]),
                cx,
            ));
            body = body.child(self.marker_field(
                (marker, Some(index), "details.levelDb"),
                "audioInputs.level",
                action["details"]["levelDb"].clone(),
                true,
                window,
                cx,
            ));
        }
        if kind == "communication" {
            body = body.child(
                self.action_choices(
                    (marker, index),
                    "details.payloadEncoding",
                    "settings.devices.encoding",
                    action["details"]["payloadEncoding"]
                        .as_str()
                        .unwrap_or("utf8"),
                    choices(&[
                        "utf8",
                        "hex",
                        "midi-note",
                        "globalcache-relay",
                        "rosstalk",
                        "rosstalk-hex",
                    ]),
                    cx,
                ),
            );
            body = body.child(self.marker_field(
                (marker, Some(index), "value"),
                "settings.devices.payload",
                action["value"].clone(),
                false,
                window,
                cx,
            ));
        }
        if kind == "capture" {
            let stop = action["details"].get("stop").is_some();
            let weak = cx.weak_entity();
            body = body.child(Field::new(
                cx.global::<Translator>().t("automation.macros.command"),
                Select::new(
                    SharedString::from(format!("capture-command-{marker}-{index}")),
                    choices(&["start", "stop"]),
                    if stop { "stop" } else { "start" }.to_string(),
                )
                .on_change(move |value, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.marker_patch(
                            marker,
                            Some(index),
                            "details",
                            if value == "stop" {
                                json!({"stop":{"showsAlertBeforeStopping":false}})
                            } else {
                                json!({"start":{"presetIdentification":""}})
                            },
                            cx,
                        )
                    });
                }),
            ));
            if stop {
                body = body.child(self.action_check(
                    marker,
                    index,
                    "details.stop.showsAlertBeforeStopping",
                    "capture.confirmBeforeStop",
                    action["details"]["stop"]["showsAlertBeforeStopping"] == true,
                    cx,
                ));
            } else {
                let options = std::iter::once(SelectOption::new(
                    String::new(),
                    cx.global::<Translator>().t("capture.activeSettings"),
                ))
                .chain(
                    self.source["recordingPresets"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|preset| {
                            SelectOption::new(
                                text(preset, "id").to_string(),
                                text(preset, "title").to_string(),
                            )
                        }),
                )
                .collect();
                body = body.child(self.action_choices(
                    (marker, index),
                    "details.start.presetIdentification",
                    "capture.preset",
                    text(&action["details"]["start"], "presetIdentification"),
                    options,
                    cx,
                ));
            }
        }
        if kind == "media"
            && self
                .media
                .iter()
                .any(|item| item["id"] == action["targetId"] && item["type"] == "video")
        {
            for (field, key, neutral) in [
                (
                    "details.mediaTransport.inSeconds",
                    "media.transport.inSeconds",
                    json!(0),
                ),
                (
                    "details.mediaTransport.outSeconds",
                    "media.transport.outSeconds",
                    Value::Null,
                ),
                (
                    "details.mediaTransport.rate",
                    "media.transport.rate",
                    json!(1),
                ),
                (
                    "details.mediaTransport.delaySeconds",
                    "media.transport.delaySeconds",
                    json!(0),
                ),
            ] {
                let path = format!("/{}", field.replace('.', "/"));
                body = body.child(self.marker_field(
                    (marker, Some(index), field),
                    key,
                    action.pointer(&path).cloned().unwrap_or(neutral),
                    true,
                    window,
                    cx,
                ));
            }
            body = body.child(
                self.action_choices(
                    (marker, index),
                    "details.mediaTransport.retrigger",
                    "media.transport.retrigger",
                    action["details"]["mediaTransport"]["retrigger"]
                        .as_str()
                        .unwrap_or("restart"),
                    choices(&["restart", "continue", "automatic"]),
                    cx,
                ),
            );
        }
        if matches!(kind, "presentation" | "playlist") {
            for (field, label) in [
                ("details.itemId", "automation.targets.occurrence"),
                ("details.arrangementId", "automation.targets.arrangement"),
                ("details.slideId", "automation.targets.slide"),
                ("details.cueInstanceId", "automation.targets.slide"),
            ] {
                let path = format!("/{}", field.replace('.', "/"));
                body = body.child(self.marker_field(
                    (marker, Some(index), field),
                    label,
                    action.pointer(&path).cloned().unwrap_or_else(|| json!("")),
                    false,
                    window,
                    cx,
                ));
            }
            if kind == "playlist" {
                body = body
                    .child(self.action_check(
                        marker,
                        index,
                        "details.selectPlaylist",
                        "automation.targets.selectPlaylist",
                        action["details"]["selectPlaylist"] != false,
                        cx,
                    ))
                    .child(self.action_check(
                        marker,
                        index,
                        "details.alwaysRetrigger",
                        "automation.targets.alwaysRetrigger",
                        action["details"]["alwaysRetrigger"] != false,
                        cx,
                    ));
            } else {
                body = body.child(
                    self.marker_field(
                        (marker, Some(index), "details.contentDestination"),
                        "automation.targets.destination",
                        action["details"]["contentDestination"]
                            .as_u64()
                            .map(|value| json!(value))
                            .unwrap_or_else(|| json!(0)),
                        true,
                        window,
                        cx,
                    ),
                );
            }
        }
        body.child(
            self.marker_field(
                (marker, Some(index), "delaySeconds"),
                "automation.macros.delay",
                action
                    .get("delaySeconds")
                    .cloned()
                    .unwrap_or_else(|| json!(0)),
                true,
                window,
                cx,
            ),
        )
        .into_any_element()
    }
    pub(super) fn marker_actions_valid(&self, draft: &Value) -> bool {
        draft.as_array().into_iter().flatten().all(|marker| {
            marker["actions"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|action| {
                    let kind = text(action, "type");
                    if !markers::ACTION_TYPES.contains(&kind)
                        || (kind == "communication"
                            && action.get("targetId").is_none()
                            && action.get("value").is_none())
                    {
                        return true;
                    }
                    if let Some(collection) = action_collection(kind) {
                        if !self.source[collection].as_array().is_some_and(|items| {
                            items.iter().any(|item| {
                                item["id"] == action["targetId"]
                                    || (!item["sourceId"].is_null()
                                        && item["sourceId"] == action["targetId"])
                            })
                        }) {
                            return false;
                        }
                    }
                    if kind == "clear"
                        && ![
                            "all",
                            "slide",
                            "background",
                            "prop",
                            "audio",
                            "messages",
                            "video-input",
                        ]
                        .contains(&action["targetLayer"].as_str().unwrap_or("all"))
                    {
                        return false;
                    }
                    if kind == "media" {
                        if let Some(transport) = action["details"].get("mediaTransport") {
                            if orbit_host::validate_preview_media_transport(transport).is_err() {
                                return false;
                            }
                        }
                    }
                    if kind == "transport" {
                        if let Some(jump) = action["details"].get("jumpToTime") {
                            if jump["time"]
                                .as_f64()
                                .is_none_or(|time| !time.is_finite() || time < 0.)
                            {
                                return false;
                            }
                        }
                    }
                    true
                })
        })
    }
}
fn choices(values: &[&str]) -> Vec<SelectOption<String>> {
    values
        .iter()
        .map(|value| SelectOption::new(value.to_string(), value.to_string()))
        .collect()
}
fn action_collection(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "clear-group" => "clearGroups",
        "macro" => "macros",
        "media" => "media",
        "video-input" => "videoInputs",
        "audio-input" => "audioInputs",
        "message" => "messages",
        "presentation" => "library",
        "playlist" => "playlist",
        "prop" => "props",
        "stage" => "stageLayouts",
        "audience-look" => "audienceLooks",
        "mask" => "masks",
        "timer" => "countdownTimers",
        "communication" => "communicationDevices",
        _ => return None,
    })
}
