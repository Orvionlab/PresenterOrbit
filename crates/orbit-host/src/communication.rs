use serde_json::{json, Value};
use std::{
    io::Write,
    net::{TcpStream, ToSocketAddrs, UdpSocket},
    time::Duration,
};

pub(crate) fn validate(device: &Value) -> Result<(), String> {
    crate::communication_input_config::validate(device)?;
    if device["transport"] == "midi" {
        return crate::communication_midi::validate(device).map(|_| ());
    }
    if device["transport"] == "serial" {
        return crate::communication_serial::validate(device);
    }
    if !matches!(device["transport"].as_str(), Some("tcp" | "udp")) {
        return Err("Only TCP, UDP, and serial devices are supported.".into());
    }
    let address = device["address"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or("A device host address is required.")?;
    if address.trim() != address || address.chars().any(char::is_whitespace) {
        return Err("The device host address must not contain whitespace.".into());
    }
    if !device["port"]
        .as_u64()
        .is_some_and(|port| (1..=65535).contains(&port))
    {
        return Err("Communication device port must be between 1 and 65535.".into());
    }
    Ok(())
}

pub(crate) fn validate_changed(previous: &Value, next: &Value) -> Result<(), String> {
    let devices = next["communicationDevices"]
        .as_array()
        .ok_or("Communication devices must be an array.")?;
    let mut ids = std::collections::HashSet::new();
    for device in devices {
        let id = device["id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or("A device id is required.")?;
        if !ids.insert(id) {
            return Err("Device ids must be unique.".into());
        }
        let unchanged = previous["communicationDevices"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry == device));
        if unchanged {
            continue;
        }
        if !device["title"]
            .as_str()
            .is_some_and(|title| !title.trim().is_empty())
        {
            return Err("A device name is required.".into());
        }
        validate(device)?;
    }
    Ok(())
}

pub(crate) fn send(device: Value, payload: Value) -> Result<Value, String> {
    if device.get("input").is_some() {
        return Err("Receive-only devices cannot send communication actions.".into());
    }
    if device["transport"] == "midi" {
        return crate::communication_midi::send(&device, &payload);
    }
    if payload["protocol"] == "globalcache-relay" {
        return crate::communication_globalcache::send(&device, payload);
    }
    validate(&device)?;
    let payload = payload_bytes(&payload)?;
    if device["transport"] == "serial" {
        crate::communication_serial::send(&device, &payload)?;
        return Ok(json!({"ok":true,"bytesSent":payload.len()}));
    }
    let host = device["address"].as_str().expect("validated address");
    let port = device["port"].as_u64().expect("validated port") as u16;
    let address = (host, port)
        .to_socket_addrs()
        .map_err(|error| error.to_string())?
        .next()
        .ok_or("The device host resolved to no address.")?;
    if device["transport"] == "udp" {
        let socket = UdpSocket::bind(if address.is_ipv6() {
            "[::]:0"
        } else {
            "0.0.0.0:0"
        })
        .map_err(|error| error.to_string())?;
        let sent = socket
            .send_to(&payload, address)
            .map_err(|error| error.to_string())?;
        if sent != payload.len() {
            return Err("The complete UDP payload was not sent.".into());
        }
    } else {
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(|error| error.to_string())?;
        stream
            .write_all(&payload)
            .map_err(|error| error.to_string())?;
    }
    Ok(json!({"ok":true,"bytesSent":payload.len()}))
}

fn payload_bytes(payload: &Value) -> Result<Vec<u8>, String> {
    let bytes = match payload {
        Value::String(text) => text.as_bytes().to_vec(),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|byte| u8::try_from(byte).ok())
                    .ok_or_else(|| "Payload bytes must be integers between 0 and 255.".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err("A text or byte payload is required.".into()),
    };
    if bytes.is_empty() {
        return Err("A non-empty payload is required.".into());
    }
    Ok(bytes)
}
