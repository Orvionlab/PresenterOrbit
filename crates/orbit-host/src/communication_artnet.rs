use crate::communication_input::{Decoder, DecoderFactory, ExchangeFactory};
use crate::Host;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    net::Ipv4Addr,
    sync::Arc,
    time::{Duration, Instant},
};
static CONTROLLERS: AtomicU64 = AtomicU64::new(1);

pub(crate) fn validate(device: &Value) -> Result<(), String> {
    let input = device["input"]
        .as_object()
        .ok_or("Art-Net input must be an object.")?;
    let field = if input.contains_key("baseAddress") {
        "baseAddress"
    } else {
        "controlChannel"
    };
    if input.len() != if input.contains_key("oemCode") { 5 } else { 4 }
        || input.keys().any(|key| {
            !matches!(
                key.as_str(),
                "protocol" | "universe" | "autoReconnect" | "oemCode"
            ) && key != field
        })
        || !input.get("autoReconnect").is_some_and(Value::is_boolean)
        || input
            .get("oemCode")
            .is_some_and(|value| !value.as_u64().is_some_and(|code| code <= 65535))
        || device["transport"] != "udp"
        || device["port"] != 6454
        || device["address"]
            .as_str()
            .and_then(|address| address.parse::<Ipv4Addr>().ok())
            .is_none()
        || !input
            .get("universe")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= 32767)
        || !input
            .get(field)
            .and_then(Value::as_u64)
            .is_some_and(|value| {
                (1..=if field == "baseAddress" { 496 } else { 512 }).contains(&value)
            })
    {
        return Err("Art-Net requires UDP port 6454, a local IPv4 address, universe 0–32767 and either control channel 1–512 or fixture base address 1–496.".into());
    }
    Ok(())
}

pub(crate) fn decoder() -> Decoder {
    Box::new(|bytes| {
        Ok(if bytes.is_empty() {
            Vec::new()
        } else {
            vec![bytes.to_vec()]
        })
    })
}

struct Receiver {
    universe: u16,
    channel: usize,
    sequence: Option<(u8, Instant)>,
    physical: Option<u8>,
    control: u8,
    fixture: Option<Fixture>,
    slots: Vec<(u8, u8)>,
}
struct Fixture {
    base: usize,
    previous: [u8; 16],
}

impl Receiver {
    fn new(universe: u16, channel: usize) -> Self {
        Self {
            universe,
            channel,
            sequence: None,
            physical: None,
            control: 0,
            fixture: None,
            slots: Vec::new(),
        }
    }

    fn fixture(universe: u16, base: usize) -> Self {
        let mut receiver = Self::new(universe, base + 16);
        receiver.fixture = Some(Fixture {
            base,
            previous: [0; 16],
        });
        receiver
    }

    fn receive(&mut self, bytes: &[u8], now: Instant) -> Result<Option<&'static str>, String> {
        self.slots.clear();
        if bytes.len() < 12 || &bytes[..8] != b"Art-Net\0" {
            return Err("Invalid Art-Net packet header.".into());
        }
        if u16::from_le_bytes([bytes[8], bytes[9]]) != 0x5000 {
            return Ok(None);
        }
        if u16::from_be_bytes([bytes[10], bytes[11]]) < 14 || bytes.len() < 18 {
            return Err("Invalid ArtDmx protocol version or header.".into());
        }
        let length = u16::from_be_bytes([bytes[16], bytes[17]]) as usize;
        if !(2..=512).contains(&length)
            || length % 2 != 0
            || bytes.len() < 18 + length
            || bytes[15] > 127
        {
            return Err(
                "ArtDmx requires a complete, even payload of 2–512 slots and a 15-bit universe."
                    .into(),
            );
        }
        if u16::from_le_bytes([bytes[14], bytes[15]]) != self.universe {
            return Ok(None);
        }
        if self.channel > length {
            return Err("ArtDmx packet does not contain the configured control channel.".into());
        }
        // One physical source owns each UDP controller session; merging control commands is unsafe.
        if self.physical.is_some_and(|physical| physical != bytes[13]) {
            return Err(
                "Multiple physical Art-Net sources are not supported on one controller session."
                    .into(),
            );
        }
        self.physical = Some(bytes[13]);
        let sequence = bytes[12];
        if sequence != 0 {
            if let Some((previous, received)) = self.sequence {
                let distance = (sequence as u16 + 255 - previous as u16) % 255;
                if now.duration_since(received) < Duration::from_millis(2500)
                    && (distance == 0 || distance > 127)
                {
                    return Ok(None);
                }
            }
            self.sequence = Some((sequence, now));
        } else {
            self.sequence = None;
        }
        if bytes[17 + self.channel] > 80 {
            return Err("Art-Net control values must be 0–80; zero rearms a command.".into());
        }
        if let Some(fixture) = &mut self.fixture {
            for index in 0..16 {
                let value = bytes[17 + fixture.base + index];
                let changed = value != fixture.previous[index];
                // Preserve the item selector when changing playlists in the same frame.
                let playlist_changed = index == 1 && self.slots.iter().any(|(slot, _)| *slot == 1);
                if value != 0 && (changed || playlist_changed) {
                    self.slots.push((index as u8 + 1, value));
                }
                fixture.previous[index] = value;
            }
        }
        let value = bytes[17 + self.channel];
        let control = if value == 0 {
            0
        } else if value <= 80 {
            (value - 1) / 5 + 1
        } else {
            17
        };
        if control == self.control {
            return Ok(None);
        }
        self.control = control;
        match control {
            0 => Ok(None),
            1 => Ok(Some("clearAll")),
            2 => Ok(Some("clearSlide")),
            3 => Ok(Some("clearMedia")),
            4 => Ok(Some("clearMessages")),
            5 => Ok(Some("clearProps")),
            6 => Ok(Some("clearAudio")),
            7 => Ok(Some("clearLogo")),
            8 => Ok(Some("transportBeginning")),
            9 => Ok(Some("transportPlay")),
            10 => Ok(Some("transportPause")),
            11 => Ok(Some("transportToggle")),
            12 => Ok(Some("previousSlide")),
            13 => Ok(Some("nextSlide")),
            14 => Ok(Some("timelineStart")),
            15 => Ok(Some("timelineStop")),
            16 => Ok(Some("timelineBeginning")),
            _ => Err("Art-Net control values must be 0–80; zero rearms a command.".into()),
        }
    }
}

pub(crate) fn factories(
    app: &Host,
    path: &str,
    device: &Value,
) -> Result<(DecoderFactory, ExchangeFactory), String> {
    validate(device)?;
    let universe = device["input"]["universe"]
        .as_u64()
        .ok_or("An Art-Net universe is required.")? as u16;
    let base = device["input"]["baseAddress"]
        .as_u64()
        .map(|value| value as usize);
    let channel = if let Some(base) = base {
        base + 16
    } else {
        device["input"]["controlChannel"]
            .as_u64()
            .ok_or("An Art-Net control channel is required.")? as usize
    };
    let app = app.clone();
    let path = path.to_string();
    let id = device["id"].clone();
    Ok((
        Arc::new(decoder),
        Arc::new(move || {
            let controller = CONTROLLERS.fetch_add(1, Ordering::Relaxed).to_string();
            let mut receiver = match base {
                Some(base) => Receiver::fixture(universe, base),
                None => Receiver::new(universe, channel),
            };
            let app = app.clone();
            let path = path.clone();
            let id = id.clone();
            Box::new(move |session, bytes| {
                let command = receiver.receive(bytes, Instant::now())?;
                if command.is_some() || !receiver.slots.is_empty() {
                    let command = if base.is_some() {
                        json!({"command":"artnetFixture", "slots":receiver.slots, "controlCommand":command})
                    } else {
                        json!({"command":command})
                    };
                    let mut event = command;
                    event["workspacePath"] = json!(path);
                    event["deviceId"] = id.clone();
                    event["sessionId"] = json!(session);
                    event["controllerId"] = json!(controller);
                    app.emit_to("main", "communication-input", event)
                        .map_err(|error| error.to_string())?;
                }
                Ok(Vec::new().into())
            })
        }),
    ))
}
