//! `core/globalcache.ts`, `core/rosstalk.ts`, and `core/communication-input.ts`: the payload encoders of output
//! devices and the validation of input devices (RossTalk, Sony BVW, AMP, VDCP, Art-Net).

use serde_json::{json, Map, Value};

fn positive(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        !v.is_empty()
            && !v.starts_with('0')
            && v.chars().all(|c| c.is_ascii_digit())
            && v.parse::<u64>().is_ok()
    })
}

fn string_record(value: &str) -> Option<Map<String, Value>> {
    let parsed: Value = serde_json::from_str(value).ok()?;
    let record = parsed.as_object()?.clone();
    record.values().all(Value::is_string).then_some(record)
}

pub const GLOBALCACHE_COMMANDS: [&str; 4] = ["on", "off", "onThenOff", "offThenOn"];

/// `newGlobalCacheCommand`.
pub fn new_globalcache(command: &str) -> String {
    let mut value = json!({"command": command, "module": "1", "output": "1"});
    if command == "onThenOff" || command == "offThenOn" {
        value["intervalSeconds"] = json!("1");
    }
    value.to_string()
}

/// `encodeGlobalCacheCommand`.
pub fn encode_globalcache(value: &str) -> Result<Value, &'static str> {
    const INVALID: &str = "settings.devices.globalCacheInvalid";
    let draft = string_record(value).ok_or(INVALID)?;
    let command = draft
        .get("command")
        .and_then(Value::as_str)
        .filter(|c| GLOBALCACHE_COMMANDS.contains(c))
        .ok_or(INVALID)?;
    let timed = command == "onThenOff" || command == "offThenOn";
    let fields: Vec<&str> = ["command", "module", "output"]
        .into_iter()
        .chain(timed.then_some("intervalSeconds"))
        .collect();
    let get = |key: &str| draft.get(key).and_then(Value::as_str);
    if draft.len() != fields.len()
        || !fields.iter().all(|f| draft.contains_key(*f))
        || !positive(get("module"))
    {
        return Err(INVALID);
    }
    let module: u64 = get("module").and_then(|m| m.parse().ok()).ok_or(INVALID)?;
    let output = get("output").unwrap_or("");
    if module > 255
        || !(output.len() == 1 && ('1'..='6').contains(&output.chars().next().unwrap_or('0')))
    {
        return Err(INVALID);
    }
    let mut encoded = json!({"protocol": "globalcache-relay", "command": command, "module": module, "output": output.parse::<u64>().map_err(|_| INVALID)?});
    if timed {
        let interval = get("intervalSeconds").unwrap_or("");
        let (whole, fraction) = interval
            .split_once('.')
            .map_or((interval, ""), |(a, b)| (a, b));
        let whole_ok = whole == "0"
            || (!whole.is_empty()
                && !whole.starts_with('0')
                && whole.chars().all(|c| c.is_ascii_digit()));
        let fraction_ok = fraction.len() <= 3
            && fraction.chars().all(|c| c.is_ascii_digit())
            && (interval.contains('.') == !fraction.is_empty());
        if !whole_ok || !fraction_ok {
            return Err(INVALID);
        }
        let ms = (interval.parse::<f64>().map_err(|_| INVALID)? * 1000.).round();
        if !(1. ..=86_400_000.).contains(&ms) {
            return Err(INVALID);
        }
        encoded["intervalMs"] = json!(ms as u64);
    }
    Ok(encoded)
}

pub const ROSSTALK_COMMANDS: [&str; 5] = ["FTB", "KEYCUT", "KEYAUTO", "GPI", "CC"];

/// `newRossTalkCommand`.
pub fn new_rosstalk(command: &str) -> String {
    let fields = match command {
        "GPI" => json!({"input": "1"}),
        "CC" => json!({"bank": "1", "control": "1"}),
        "FTB" => json!({}),
        _ => json!({"busSource": "ME", "bus": "1", "key": "1", "state": "toggle"}),
    };
    let mut value = json!({"command": command});
    value
        .as_object_mut()
        .expect("object")
        .extend(fields.as_object().cloned().unwrap_or_default());
    value.to_string()
}

/// `encodeRossTalkCommand` for `rosstalk-carbonite` and `rosstalk-acuity`: the wire text with CRLF.
pub fn encode_rosstalk(value: &str, encoding: &str) -> Result<String, &'static str> {
    const INVALID: &str = "settings.devices.rossTalkInvalid";
    if encoding != "rosstalk-carbonite" && encoding != "rosstalk-acuity" {
        return Err(INVALID);
    }
    let command = string_record(value).ok_or(INVALID)?;
    let name = command
        .get("command")
        .and_then(Value::as_str)
        .filter(|c| ROSSTALK_COMMANDS.contains(c))
        .ok_or(INVALID)?;
    let get = |k: &str| command.get(k).and_then(Value::as_str);
    let exact = |fields: &[&str]| {
        command
            .keys()
            .all(|k| k == "command" || fields.contains(&k.as_str()))
            && fields.iter().all(|f| command.contains_key(*f))
    };
    let wire = match name {
        "FTB" if exact(&[]) => Some("FTB".to_string()),
        "GPI" if exact(&["input"]) && positive(get("input")) => {
            Some(format!("GPI {:0>2}", get("input").unwrap_or("")))
        }
        "CC" if exact(&["bank", "control"])
            && positive(get("bank"))
            && positive(get("control")) =>
        {
            Some(format!(
                "CC {}:{:0>2}",
                get("bank").unwrap_or(""),
                get("control").unwrap_or("")
            ))
        }
        "KEYCUT" | "KEYAUTO"
            if exact(&["busSource", "bus", "key", "state"])
                && positive(get("key"))
                && ["toggle", "on", "off"].contains(&get("state").unwrap_or("")) =>
        {
            let suffix = if get("state") == Some("toggle") {
                String::new()
            } else {
                get("state").unwrap_or("").to_uppercase()
            };
            let (source, bus, key) = (
                get("busSource").unwrap_or(""),
                get("bus"),
                get("key").unwrap_or(""),
            );
            if encoding == "rosstalk-acuity" {
                (source == "ME" && positive(bus))
                    .then(|| format!("{name}{suffix} {}:{key}", bus.unwrap_or("")))
            } else if !["ME", "MME", "MSC"].contains(&source)
                || !(positive(bus) || (source == "ME" && bus == Some("P/P")))
            {
                None
            } else {
                Some(format!(
                    "{name}{suffix} {source}:{}:{key}",
                    bus.unwrap_or("")
                ))
            }
        }
        _ => None,
    };
    wire.map(|w| format!("{w}\r\n")).ok_or(INVALID)
}

const INPUT_INVALID: &str = "settings.devices.inputInvalid";

fn in_range(value: &Value, min: u64, max: u64) -> bool {
    value.as_u64().is_some_and(|n| (min..=max).contains(&n))
}

fn ipv4(address: &str) -> bool {
    let parts: Vec<&str> = address.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            (*p == "0" || (!p.starts_with('0') && !p.is_empty()))
                && p.len() <= 3
                && p.chars().all(|c| c.is_ascii_digit())
                && p.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}

fn artnet_error(device: &Value, input: &Value) -> Option<&'static str> {
    let object = input.as_object()?;
    let field = if object.contains_key("baseAddress") {
        "baseAddress"
    } else {
        "controlChannel"
    };
    let expected = if object.contains_key("oemCode") { 5 } else { 4 };
    let ok = device["transport"] == "udp"
        && device["port"] == 6454
        && ipv4(device["address"].as_str().unwrap_or(""))
        && object.len() == expected
        && object.keys().all(|k| {
            ["protocol", "universe", field, "autoReconnect", "oemCode"].contains(&k.as_str())
        })
        && input["autoReconnect"].is_boolean()
        && (!object.contains_key("oemCode") || in_range(&input["oemCode"], 0, 65535))
        && in_range(&input["universe"], 0, 32767)
        && in_range(
            &input[field],
            1,
            if field == "baseAddress" { 496 } else { 512 },
        );
    (!ok).then_some("settings.devices.artnetInvalid")
}

fn deck_error(input: &Value) -> Option<&'static str> {
    let object = input.as_object()?;
    let allowed = [
        "protocol",
        "frameRate",
        "dropFrame",
        "mediaLibraryId",
        "shortNames",
        "virtualPortId",
        "unitAddress",
    ];
    if object.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Some(INPUT_INVALID);
    }
    let rate = &input["frameRate"];
    let pair = format!("{}/{}", rate["numerator"], rate["denominator"]);
    let rate_ok = rate.as_object().is_some_and(|r| r.len() == 2)
        && ["25/1", "30/1", "30000/1001"].contains(&pair.as_str())
        && input["dropFrame"].is_boolean()
        && (input["dropFrame"] != true || rate["denominator"] == 1001);
    let vdcp = input["protocol"] == "vdcp";
    let port_ok = if vdcp {
        in_range(&input["virtualPortId"], 1, 127) && in_range(&input["unitAddress"], 0, 15)
    } else {
        !object.contains_key("virtualPortId") && !object.contains_key("unitAddress")
    };
    if !rate_ok || !port_ok {
        return Some(INPUT_INVALID);
    }
    if !vdcp && input["protocol"] != "amp" {
        return (object.contains_key("mediaLibraryId") || object.contains_key("shortNames"))
            .then_some(INPUT_INVALID);
    }
    (!(input["mediaLibraryId"].is_string() && input["shortNames"].is_boolean()))
        .then_some(INPUT_INVALID)
}

/// `communicationInputError`: `None` for an output device.
pub fn input_error(device: &Value) -> Option<&'static str> {
    let input = &device["input"];
    if input.is_null() {
        return None;
    }
    if !input.is_object()
        || !["rosstalk", "sony-bvw", "amp", "vdcp", "artnet"]
            .iter()
            .any(|p| input["protocol"] == *p)
    {
        return Some(INPUT_INVALID);
    }
    if input["protocol"] == "artnet" {
        return artnet_error(device, input);
    }
    let address = device["address"].as_str().unwrap_or("");
    let address_ok = !address.is_empty()
        && address
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '.' || c == ':');
    if device["transport"] != "serial" && !address_ok {
        return Some(INPUT_INVALID);
    }
    if input["protocol"] == "rosstalk" {
        return (!(device["transport"] == "tcp"
            && input.as_object().is_some_and(|o| o.len() == 1)))
        .then_some(INPUT_INVALID);
    }
    if input["protocol"] == "amp" && device["transport"] != "tcp" {
        return Some(INPUT_INVALID);
    }
    deck_error(input)
}

/// `withArtNetMap`: a single control channel (17) or a presentation fixture (base address 1).
pub fn with_artnet_map(input: &Value, fixture: bool) -> Value {
    let mut next = input.clone();
    if let Some(o) = next.as_object_mut() {
        o.remove("controlChannel");
        o.remove("baseAddress");
    }
    next[if fixture {
        "baseAddress"
    } else {
        "controlChannel"
    }] = json!(if fixture { 1 } else { 17 });
    next
}

/// `withArtNetDiscovery`: enabling it adds an OEM code that stays invalid until the operator enters one.
pub fn with_artnet_discovery(input: &Value, enabled: bool) -> Value {
    let mut next = input.clone();
    if let Some(o) = next.as_object_mut() {
        o.remove("oemCode");
    }
    if enabled {
        next["oemCode"] = Value::Null;
    }
    next
}

/// `withCommunicationInputProtocol`: `output` removes the input; the others get their default settings.
pub fn with_input_protocol(device: &Value, protocol: &str) -> Value {
    let mut next = device.clone();
    if protocol == "output" {
        if let Some(o) = next.as_object_mut() {
            o.remove("input");
        }
        return next;
    }
    if protocol == "artnet" {
        next["transport"] = json!("udp");
        next["port"] = json!(6454);
        next["input"] = json!({"protocol": protocol, "universe": 0, "controlChannel": 17, "autoReconnect": true});
        return next;
    }
    next["transport"] = json!("tcp");
    next["input"] = if protocol == "rosstalk" {
        json!({"protocol": protocol})
    } else {
        let mut deck = json!({"protocol": protocol, "frameRate": {"numerator": 25, "denominator": 1}, "dropFrame": false});
        if protocol == "vdcp" || protocol == "amp" {
            deck["mediaLibraryId"] = json!("");
            deck["shortNames"] = json!(false);
        }
        if protocol == "vdcp" {
            deck["virtualPortId"] = json!(1);
            deck["unitAddress"] = json!(0);
        }
        deck
    };
    next
}
