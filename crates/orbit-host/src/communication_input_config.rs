use serde_json::Value;

pub(crate) fn validate(device: &Value) -> Result<(), String> {
    let Some(input) = device.get("input") else {
        return Ok(());
    };
    let input = input
        .as_object()
        .ok_or("Communication input must be an object.")?;
    let protocol = input
        .get("protocol")
        .and_then(Value::as_str)
        .ok_or("An input protocol is required.")?;
    if protocol == "artnet" {
        return crate::communication_artnet::validate(device);
    }
    if !matches!(protocol, "rosstalk" | "sony-bvw" | "vdcp" | "amp") {
        return Err("Unsupported communication input protocol.".into());
    }
    if protocol == "rosstalk" {
        if device["transport"] != "tcp" || input.len() != 1 {
            return Err("RossTalk input requires TCP and no deck parameters.".into());
        }
    } else {
        if protocol == "amp" && device["transport"] != "tcp" {
            return Err("AMP input requires TCP.".into());
        }
        if input.keys().any(|key| {
            !matches!(
                key.as_str(),
                "protocol"
                    | "frameRate"
                    | "dropFrame"
                    | "mediaLibraryId"
                    | "shortNames"
                    | "virtualPortId"
                    | "unitAddress"
            )
        }) {
            return Err("Unknown deck input parameter.".into());
        }
        if protocol == "vdcp" {
            if !device["input"]["virtualPortId"]
                .as_u64()
                .is_some_and(|port| (1..=127).contains(&port))
                || !device["input"]["unitAddress"]
                    .as_u64()
                    .is_some_and(|unit| unit <= 15)
            {
                return Err(
                    "VDCP requires an output port from 1 to 127 and a unit address from 0 to 15."
                        .into(),
                );
            }
        } else if input.contains_key("virtualPortId") || input.contains_key("unitAddress") {
            return Err("VDCP addressing is not valid for this input protocol.".into());
        }
        let rate = &device["input"]["frameRate"];
        let numerator = rate["numerator"]
            .as_u64()
            .ok_or("An explicit frame rate is required.")?;
        let denominator = rate["denominator"]
            .as_u64()
            .ok_or("An explicit frame rate denominator is required.")?;
        if !matches!((numerator, denominator), (25, 1) | (30, 1) | (30000, 1001))
            || rate.as_object().is_none_or(|rate| rate.len() != 2)
        {
            return Err("Supported deck rates are 25, 30 and 30000/1001 fps.".into());
        }
        let drop = device["input"]["dropFrame"]
            .as_bool()
            .ok_or("An explicit drop-frame mode is required.")?;
        if drop && (numerator, denominator) != (30000, 1001) {
            return Err("Drop frame requires 30000/1001 fps.".into());
        }
        if matches!(protocol, "vdcp" | "amp") {
            if !device["input"]["mediaLibraryId"].is_string()
                || !device["input"]["shortNames"].is_boolean()
            {
                return Err(
                    "Media-deck input requires a media bin and an explicit short-name mode.".into(),
                );
            }
        } else if input.contains_key("mediaLibraryId") || input.contains_key("shortNames") {
            return Err("Sony BVW controls the live clip without a media catalog.".into());
        }
    }
    Ok(())
}
