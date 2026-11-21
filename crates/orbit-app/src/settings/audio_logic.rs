//! `audio-settings-utils.ts` and `audio-settings-actions.ts`: the audio settings the panel shows and the edits it
//! makes. Everything is JSON, as the host stores it, so a saved edit equals what React saves.

use serde_json::{json, Map, Value};

pub const MAX_BUS_CHANNELS: usize = 16;
pub const MAX_OUTPUT_CHANNELS: usize = 16;

fn record(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// A number as JSON, an integer when it is one, so equal values compare equal.
fn number(value: f64) -> Value {
    if value.fract() == 0. && value.abs() < 9.0e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// JavaScript `Number(value)`: numbers and numeric strings, `NaN` otherwise.
fn js_number(value: &Value) -> f64 {
    match value {
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::String(text) => text.trim().parse().unwrap_or(f64::NAN),
        Value::Bool(flag) => f64::from(u8::from(*flag)),
        Value::Null => 0.,
        _ => f64::NAN,
    }
}

fn default_channel(number: usize) -> Value {
    let channels = match number {
        1 => json!({"1": true, "2": false}),
        2 => json!({"1": false, "2": true}),
        _ => json!({}),
    };
    json!({
        "id": format!("pro-{number}"),
        "label": format!("Pro Channel {number}"),
        "channels": channels,
        "modes": {"mute": false, "solo": false, "tone": false},
    })
}

fn default_routing() -> Value {
    json!({"deviceChannels": 2, "proChannels": [default_channel(1), default_channel(2)]})
}

/// `DEFAULT_AUDIO_SETTINGS`.
pub fn default_audio_settings() -> Value {
    json!({
        "main": {"device": "system", "deviceLabel": "", "level": 0, "delay": 0, "routing": default_routing()},
        "sdiNdi": {"enabled": true, "level": 0, "delay": 0, "routing": default_routing()},
        "inspector": {"device": "system", "deviceLabel": "", "level": 0, "routing": default_routing()},
    })
}

fn pro_channel_number(id: &Value) -> usize {
    let Some(id) = id.as_str() else {
        return 0;
    };
    let digits = id.strip_prefix("pro-").filter(|digits| {
        !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
    });
    digits
        .and_then(|digits| digits.parse::<usize>().ok())
        .filter(|number| (1..=MAX_BUS_CHANNELS).contains(number))
        .unwrap_or(0)
}

fn clamp_device_channels(value: &Value) -> f64 {
    let requested = js_number(value);
    let requested = if requested.is_nan() || requested == 0. {
        2.
    } else {
        requested
    };
    requested.min(MAX_OUTPUT_CHANNELS as f64).max(1.)
}

/// `asBooleanChannelRecord`: keys that are whole numbers from 1 to `max` with boolean values.
fn boolean_channels(value: &Value, max: f64) -> Map<String, Value> {
    record(value)
        .into_iter()
        .filter(|(key, enabled)| {
            key.parse::<f64>()
                .is_ok_and(|key| key.fract() == 0. && key >= 1. && key <= max)
                && enabled.is_boolean()
        })
        .collect()
}

/// `getAudioRouting`.
pub fn get_audio_routing(settings: &Value) -> Value {
    let routing = settings.get("routing").cloned().unwrap_or(Value::Null);
    let incoming: Vec<Map<String, Value>> = routing["proChannels"]
        .as_array()
        .map(|channels| channels.iter().map(record).collect())
        .unwrap_or_default();
    let count = incoming
        .iter()
        .map(|channel| pro_channel_number(channel.get("id").unwrap_or(&Value::Null)))
        .chain([2, incoming.len()])
        .max()
        .unwrap_or(2)
        .min(MAX_BUS_CHANNELS);
    let device_channels = clamp_device_channels(&routing["deviceChannels"]);
    let channels: Vec<Value> = (1..=count)
        .map(|index| {
            let default = default_channel(index);
            let incoming = incoming
                .iter()
                .find(|channel| channel.get("id") == default.get("id"))
                .cloned()
                .unwrap_or_default();
            let mut merged = record(&default);
            merged.extend(incoming.clone());
            let mut channels = record(&default["channels"]);
            channels.extend(boolean_channels(
                incoming.get("channels").unwrap_or(&Value::Null),
                device_channels,
            ));
            let mut modes = record(&default["modes"]);
            modes.extend(
                record(incoming.get("modes").unwrap_or(&Value::Null))
                    .into_iter()
                    .filter(|(key, enabled)| {
                        ["mute", "solo", "tone"].contains(&key.as_str()) && enabled.is_boolean()
                    }),
            );
            merged.insert("channels".into(), Value::Object(channels));
            merged.insert("modes".into(), Value::Object(modes));
            Value::Object(merged)
        })
        .collect();
    json!({"deviceChannels": number(device_channels), "proChannels": channels})
}

/// `resizeAudioRoutingBuses`: an invalid count returns the routing unchanged.
pub fn resize_buses(routing: &Value, count: f64) -> Value {
    if count.fract() != 0. || count.is_nan() || count < 2. || count > MAX_BUS_CHANNELS as f64 {
        return routing.clone();
    }
    let count = count as usize;
    let mut channels = routing["proChannels"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let existing = channels.len();
    if count <= existing {
        channels.truncate(count);
    } else {
        for index in existing..count {
            let mut added = default_channel(index + 1);
            added["channels"] = json!({});
            channels.push(added);
        }
    }
    let mut next = routing.clone();
    next["proChannels"] = Value::Array(channels);
    next
}

/// `resizeAudioRoutingDestinations`.
pub fn resize_destinations(routing: &Value, count: f64) -> Value {
    if count.fract() != 0. || count.is_nan() || count < 1. || count > MAX_OUTPUT_CHANNELS as f64 {
        return routing.clone();
    }
    let mut next = routing.clone();
    next["deviceChannels"] = number(count);
    let channels: Vec<Value> = routing["proChannels"]
        .as_array()
        .map(|channels| {
            channels
                .iter()
                .map(|channel| {
                    let mut channel = channel.clone();
                    channel["channels"] =
                        Value::Object(boolean_channels(&channel["channels"], count));
                    channel
                })
                .collect()
        })
        .unwrap_or_default();
    next["proChannels"] = Value::Array(channels);
    next
}

fn merge_section(value: &Value, defaults: &Value) -> Value {
    let mut merged = record(defaults);
    let section = record(value);
    merged.extend(section.clone());
    if defaults.get("deviceLabel").is_some() {
        let label = section
            .get("deviceLabel")
            .filter(|label| label.is_string())
            .unwrap_or(&defaults["deviceLabel"])
            .clone();
        merged.insert("deviceLabel".into(), label);
    }
    merged.insert("routing".into(), get_audio_routing(value));
    Value::Object(merged)
}

/// `getAudioSettings` of the saved settings object.
pub fn get_audio_settings(settings: &Value) -> Value {
    let defaults = default_audio_settings();
    let audio = &settings["audioSettings"];
    json!({
        "main": merge_section(&audio["main"], &defaults["main"]),
        "sdiNdi": merge_section(&audio["sdiNdi"], &defaults["sdiNdi"]),
        "inspector": merge_section(&audio["inspector"], &defaults["inspector"]),
    })
}

pub type Device = (String, String);

/// `getSelectedAudioDeviceLabel`.
pub fn selected_device_label(
    devices: &[Device],
    device: &str,
    translate: Option<&dyn Fn(&str) -> String>,
) -> String {
    match device {
        "none" => translate.map_or("None".to_string(), |t| t("settings.audio.device.none")),
        "system" => translate.map_or("System Default".to_string(), |t| {
            t("settings.audio.device.systemDefault")
        }),
        id => devices
            .iter()
            .find(|(candidate, _)| candidate == id)
            .map(|(_, label)| label.clone())
            .unwrap_or_default(),
    }
}

/// `getAudioDeviceOptions`: none, the system default, every listed device, and a saved device that is gone.
pub fn device_options(
    devices: &[Device],
    selected: Option<&str>,
    saved_label: &str,
    t: &dyn Fn(&str) -> String,
) -> Vec<Device> {
    let mut options = vec![
        ("none".to_string(), t("settings.audio.device.none")),
        (
            "system".to_string(),
            t("settings.audio.device.systemDefault"),
        ),
    ];
    options.extend(
        devices
            .iter()
            .filter(|(id, _)| !id.is_empty() && id != "default" && id != "none")
            .map(|(id, label)| {
                (
                    id.clone(),
                    if label.is_empty() {
                        id.clone()
                    } else {
                        label.clone()
                    },
                )
            }),
    );
    if let Some(selected) = selected.filter(|selected| !selected.is_empty()) {
        if !options.iter().any(|(value, _)| value == selected) {
            options.push((
                selected.to_string(),
                if saved_label.is_empty() {
                    t("settings.audio.device.saved")
                } else {
                    saved_label.to_string()
                },
            ));
        }
    }
    options
}

/// `updateAudioSetting`: the audio settings after one field of one group changed.
pub fn update_setting(
    audio: &Value,
    group: &str,
    field: &str,
    value: Value,
    devices: &[Device],
) -> Value {
    let mut next = audio.clone();
    let mut section = record(&audio[group]);
    section.insert(field.to_string(), value.clone());
    if matches!(group, "main" | "inspector") && field == "device" {
        section.insert(
            "deviceLabel".into(),
            json!(selected_device_label(
                devices,
                value.as_str().unwrap_or(""),
                None
            )),
        );
    }
    next[group] = Value::Object(section);
    next
}

fn update_channel(
    audio: &Value,
    group: &str,
    channel_id: &str,
    change: impl Fn(&mut Value),
) -> Value {
    let mut routing = get_audio_routing(&audio[group]);
    if let Some(channels) = routing["proChannels"].as_array_mut() {
        for channel in channels
            .iter_mut()
            .filter(|channel| channel["id"] == channel_id)
        {
            change(channel);
        }
    }
    let mut next = audio.clone();
    let mut section = record(&audio[group]);
    section.insert("routing".into(), routing);
    next[group] = Value::Object(section);
    next
}

/// `updateAudioRoutingChannel`: toggles one routing cell.
pub fn toggle_channel(
    audio: &Value,
    group: &str,
    channel_id: &str,
    device_channel: usize,
) -> Value {
    update_channel(audio, group, channel_id, |channel| {
        let key = device_channel.to_string();
        let current = channel["channels"][&key] == json!(true);
        channel["channels"][&key] = json!(!current);
    })
}

/// `updateAudioRoutingMode`: toggles mute, solo, or tone.
pub fn toggle_mode(audio: &Value, group: &str, channel_id: &str, mode: &str) -> Value {
    update_channel(audio, group, channel_id, |channel| {
        let current = channel["modes"][mode] == json!(true);
        channel["modes"][mode] = json!(!current);
    })
}
