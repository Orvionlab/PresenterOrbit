//! `ActionListEditor.tsx`: the ordered actions of a macro or a slide, edited in the JSON draft
//! of a [`Form`]. The fields an action type shows come from [`specs`], which is pure; [`render`] lays them out with
//! the enable, order, and remove buttons.

use gpui::{div, prelude::*, Context, SharedString, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::form::{get, Form, Kind, Spec};
use super::validate::{target_options, ACTION_TYPES, CLEAR_LAYERS};

const TIMER_COMMANDS: [&str; 6] = [
    "start",
    "stop",
    "reset",
    "reset-and-start",
    "stop-and-reset",
    "increment",
];

/// `addAction` of the editor: a new action clears the slide layer until the operator chooses another type.
pub fn new_action(id: String) -> Value {
    json!({"id": id, "type": "clear", "targetLayer": "slide", "enabled": true, "delaySeconds": 0})
}

fn choices(values: &[&str]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|value| (value.to_string(), value.to_string()))
        .collect()
}

fn collection_target(kind: &str) -> bool {
    matches!(
        kind,
        "clear-group"
            | "macro"
            | "media"
            | "video-input"
            | "audio-input"
            | "message"
            | "presentation"
            | "playlist"
            | "prop"
            | "stage"
            | "audience-look"
            | "mask"
            | "timer"
            | "communication"
    )
}

/// Replaces an action by one of another type, keeping its identity, enabled flag, and delay.
fn change_type(form: &mut Form, path: &str, kind: &str) {
    let action_path = path.trim_end_matches(".type");
    let current = get(&form.draft, action_path).clone();
    let mut next = json!({"id": current["id"], "type": kind});
    for key in ["enabled", "delaySeconds"] {
        if let Some(value) = current.get(key) {
            next[key] = value.clone();
        }
    }
    if kind == "capture" {
        next["details"] = json!({"start": {"presetIdentification": ""}});
    }
    form.set(action_path, next);
}

fn capture_command(form: &mut Form, path: &str, command: &str) {
    form.set(
        path,
        if command == "stop" {
            json!({"stop": {"showsAlertBeforeStopping": false}})
        } else {
            json!({"start": {"presetIdentification": ""}})
        },
    );
}

fn destination(form: &mut Form, path: &str, chosen: &str) {
    form.set(path, json!(chosen.parse::<u64>().unwrap_or(0)));
}

/// The fields of the action at `path` (`actions.0`), in order.
pub fn specs(
    path: &str,
    action: &Value,
    source: &Value,
    excluded_macro: &str,
    t: Translator,
) -> Vec<Spec> {
    let kind = action["type"].as_str().unwrap_or("");
    let at = |field: &str| format!("{path}.{field}");
    let label = |key: &str| t.t(key);
    let mut specs = vec![Spec::new(
        at("type"),
        label("automation.macros.type"),
        Kind::Hook(choices(&ACTION_TYPES), change_type, kind.to_string()),
    )];
    if collection_target(kind) {
        let mut options = vec![(String::new(), label("automation.macros.chooseTarget"))];
        let entries = target_options(source, kind, excluded_macro);
        options.extend(entries.iter().map(|entry| {
            let id = entry["id"].as_str().unwrap_or("");
            (
                id.to_string(),
                entry["title"]
                    .as_str()
                    .or_else(|| entry["name"].as_str())
                    .unwrap_or(id)
                    .to_string(),
            )
        }));
        let target = action["targetId"].as_str().unwrap_or("");
        if !target.is_empty() && !entries.iter().any(|entry| entry["id"] == target) {
            options.push((
                target.to_string(),
                format!("{target} ({})", label("automation.macros.targetInvalid")),
            ));
        }
        specs.push(Spec::new(
            at("targetId"),
            label("automation.macros.target"),
            Kind::Choice(options),
        ));
    }
    match kind {
        "clear" => specs.push(Spec::new(
            at("targetLayer"),
            label("automation.macros.target"),
            Kind::Choice(choices(&CLEAR_LAYERS)),
        )),
        "timer" | "prop" => {
            let options: &[&str] = if kind == "timer" {
                &TIMER_COMMANDS
            } else {
                &["show", "clear"]
            };
            specs.push(Spec::new(
                at("value"),
                label("automation.macros.command"),
                Kind::Choice(choices(options)),
            ));
            if kind == "timer" && action["value"] == "increment" {
                specs.push(Spec::new(
                    at("details.incrementSeconds"),
                    label("automation.macros.increment"),
                    Kind::Number,
                ));
            }
        }
        "transport" => specs.extend([
            Spec::new(
                at("details.play"),
                label("automation.macros.play"),
                Kind::Check,
            ),
            Spec::new(
                at("details.pause"),
                label("automation.macros.pause"),
                Kind::Check,
            ),
            Spec::new(
                at("details.jumpToTime.time"),
                label("media.markers.time"),
                Kind::Number,
            ),
        ]),
        "audio-input" => specs.extend([
            Spec::new(
                at("details.behaviorMode"),
                label("audioInputs.mode"),
                Kind::Choice(choices(&["", "off", "on", "auto-off", "auto-on"])),
            ),
            Spec::new(
                at("details.levelDb"),
                label("audioInputs.level"),
                Kind::Number,
            ),
        ]),
        "communication" => specs.extend([
            Spec::new(
                at("details.payloadEncoding"),
                label("settings.devices.encoding"),
                Kind::Choice(choices(&[
                    "utf8",
                    "hex",
                    "midi-note",
                    "globalcache-relay",
                    "rosstalk",
                    "rosstalk-hex",
                ])),
            ),
            Spec::new(at("value"), label("settings.devices.payload"), Kind::Text),
        ]),
        "capture" => {
            let stop = action["details"].get("stop").is_some();
            specs.push(Spec::new(
                at("details"),
                label("automation.macros.command"),
                Kind::Hook(
                    choices(&["start", "stop"]),
                    capture_command,
                    if stop { "stop" } else { "start" }.to_string(),
                ),
            ));
            if stop {
                specs.push(Spec::new(
                    at("details.stop.showsAlertBeforeStopping"),
                    label("capture.confirmBeforeStop"),
                    Kind::Check,
                ));
            } else {
                let mut options = vec![(String::new(), label("capture.activeSettings"))];
                options.extend(
                    source["recordingPresets"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|preset| {
                            (
                                preset["id"].as_str().unwrap_or("").to_string(),
                                preset["title"].as_str().unwrap_or("").to_string(),
                            )
                        }),
                );
                specs.push(Spec::new(
                    at("details.start.presetIdentification"),
                    label("capture.preset"),
                    Kind::Choice(options),
                ));
            }
        }
        "media" => {
            let video = source["media"].as_array().is_some_and(|media| {
                media
                    .iter()
                    .any(|item| item["id"] == action["targetId"] && item["type"] == "video")
            });
            if video {
                let seed = json!({"inSeconds": 0, "outSeconds": null, "rate": 1, "delaySeconds": 0, "retrigger": "restart"});
                let field = |name: &str, key: &str, kind: Kind| {
                    let spec = Spec::new(
                        at(&format!("details.mediaTransport.{name}")),
                        label(key),
                        kind,
                    )
                    .seed(at("details.mediaTransport"), seed.clone());
                    if name == "outSeconds" {
                        spec.keep_null()
                    } else {
                        spec
                    }
                };
                specs.extend([
                    field("inSeconds", "media.transport.inSeconds", Kind::Number),
                    field("outSeconds", "media.transport.outSeconds", Kind::Number),
                    field("rate", "media.transport.rate", Kind::Number),
                    field("delaySeconds", "media.transport.delaySeconds", Kind::Number),
                    field(
                        "retrigger",
                        "media.transport.retrigger",
                        Kind::Choice(choices(&["restart", "continue", "automatic"])),
                    ),
                ]);
            }
        }
        "presentation" | "playlist" => {
            for (field, key) in [
                ("details.itemId", "automation.targets.occurrence"),
                ("details.arrangementId", "automation.targets.arrangement"),
                ("details.slideId", "automation.targets.slide"),
                ("details.cueInstanceId", "automation.targets.slide"),
            ] {
                specs.push(Spec::new(at(field), label(key), Kind::Text));
            }
            if kind == "playlist" {
                specs.extend([
                    Spec::new(
                        at("details.selectPlaylist"),
                        label("automation.targets.selectPlaylist"),
                        Kind::On,
                    ),
                    Spec::new(
                        at("details.alwaysRetrigger"),
                        label("automation.targets.alwaysRetrigger"),
                        Kind::On,
                    ),
                ]);
            } else {
                specs.push(Spec::new(
                    at("details.contentDestination"),
                    label("automation.targets.destination"),
                    Kind::Hook(
                        vec![
                            ("0".into(), label("automation.targets.destinationAudience")),
                            (
                                "1".into(),
                                label("automation.targets.destinationAnnouncements"),
                            ),
                        ],
                        destination,
                        action["details"]["contentDestination"]
                            .as_u64()
                            .unwrap_or(0)
                            .to_string(),
                    ),
                ));
            }
        }
        _ => {}
    }
    specs.push(Spec::new(
        at("delaySeconds"),
        label("automation.macros.delay"),
        Kind::Number,
    ));
    specs.push(Spec::new(
        at("enabled"),
        label("automation.macros.enabled"),
        Kind::On,
    ));
    specs
}

/// Moves the action at `index` of the array at `prefix` by `offset`; a move past either end does nothing.
pub fn move_action(form: &mut Form, prefix: &str, index: usize, offset: isize) {
    let mut actions = get(&form.draft, prefix)
        .as_array()
        .cloned()
        .unwrap_or_default();
    if let Some(target) = index
        .checked_add_signed(offset)
        .filter(|target| *target < actions.len())
    {
        actions.swap(index, target);
        form.set(prefix, Value::Array(actions));
        form.refresh();
    }
}

pub fn remove_action(form: &mut Form, prefix: &str, index: usize) {
    let mut actions = get(&form.draft, prefix)
        .as_array()
        .cloned()
        .unwrap_or_default();
    if index < actions.len() {
        actions.remove(index);
        form.set(prefix, Value::Array(actions));
        form.refresh();
    }
}

pub fn add_action(form: &mut Form, prefix: &str, id: String) {
    let mut actions = get(&form.draft, prefix)
        .as_array()
        .cloned()
        .unwrap_or_default();
    actions.push(new_action(id));
    form.set(prefix, Value::Array(actions));
    form.refresh();
}

/// The editor for the actions at `prefix` of the form's draft.
#[allow(clippy::too_many_arguments)]
pub fn render<T: 'static>(
    form: &mut Form,
    id: &str,
    prefix: &str,
    source: &Value,
    excluded_macro: &str,
    owner: fn(&mut T) -> &mut Form,
    window: &mut Window,
    cx: &mut Context<T>,
) -> gpui::Div {
    let t = *cx.global::<Translator>();
    let gap = cx.theme().metrics.space_xs;
    let actions = get(&form.draft, prefix)
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut column = div().flex().flex_col().gap(gap);
    for (index, action) in actions.iter().enumerate() {
        let path = format!("{prefix}.{index}");
        let mut block = div()
            .flex()
            .flex_col()
            .gap(gap)
            .pb(gap)
            .border_b_1()
            .border_color(cx.theme().colors.border_soft);
        for spec in specs(&path, action, source, excluded_macro, t) {
            block = block.child(form.field(id, &spec, owner, window, cx));
        }
        let row_id = format!("{id}-{prefix}-{index}");
        let mut buttons = div().flex().gap(gap);
        for (name, label, offset) in [
            ("up", "automation.macros.up", -1isize),
            ("down", "automation.macros.down", 1),
            ("remove", "automation.macros.removeAction", 0),
        ] {
            let prefix = prefix.to_string();
            buttons = buttons.child(
                ActionButton::new(SharedString::from(format!("{row_id}-{name}")), t.t(label))
                    .variant(if offset == 0 {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Secondary
                    })
                    .on_click(cx.listener(move |this: &mut T, _, _, cx| {
                        let form = owner(this);
                        if offset == 0 {
                            remove_action(form, &prefix, index);
                        } else {
                            move_action(form, &prefix, index, offset);
                        }
                        cx.notify();
                    })),
            );
        }
        column = column.child(block.child(buttons));
    }
    let prefix = prefix.to_string();
    column.child(
        ActionButton::new(
            SharedString::from(format!("{id}-{prefix}-add")),
            t.t("automation.macros.addAction"),
        )
        .variant(ButtonVariant::Secondary)
        .on_click(cx.listener(move |this: &mut T, _, _, cx| {
            add_action(owner(this), &prefix, uuid::Uuid::new_v4().to_string());
            cx.notify();
        })),
    )
}
