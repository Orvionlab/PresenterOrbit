use super::{DeckClip, DeckCommand, DeckState, DeckTimecode, DeckTransport, ProtocolContext};

const ACK: u8 = 0x04;
const NAK: u8 = 0x05;
const NAK_UNDEFINED: u8 = 0x01;
const MAX_CATALOG_IDS: usize = u16::MAX as usize;
const MAX_FRAME_BYTES: usize = 100;
const MAX_LIST_ID_BYTES: usize = 80;
const MAX_VARIABLE_ID_BYTES: usize = MAX_FRAME_BYTES - 6;

pub(super) fn process(
    frame: &[u8],
    context: &mut ProtocolContext,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    let (command_1, command_2, data) = validate_frame(frame)?;
    let Some(unit_address) = context.unit_address else {
        return Ok(nak(NAK_UNDEFINED));
    };
    if command_1 & 0x0F != unit_address {
        return Ok(nak(NAK_UNDEFINED));
    }

    match (command_1 >> 4, command_2, data) {
        // The port number is a signed-magnitude signal port ID; this runtime
        // intentionally exposes only configured output ports.
        (3, 0x01, [port, security]) => {
            let Some(configured_port) = context.virtual_port_id else {
                return Ok(frame_response(command_1, 0x81, &[0])?);
            };
            let requested_port = decode_signal_port(*port);
            let granted = requested_port == Some(configured_port) && *security <= 1;
            if granted {
                context.port_open = true;
                context.port_selected = false;
            }
            Ok(frame_response(command_1, 0x81, &[u8::from(granted)])?)
        }
        (2, 0x21, [port]) => {
            if configured_port_matches(context, *port) && context.port_open {
                context.port_open = false;
                context.port_selected = false;
                Ok(vec![ACK])
            } else {
                Ok(nak(NAK_UNDEFINED))
            }
        }
        (2, 0x22, [port]) => {
            if configured_port_matches(context, *port) && context.port_open {
                context.port_selected = true;
                Ok(vec![ACK])
            } else {
                Ok(nak(NAK_UNDEFINED))
            }
        }
        (1, command, []) if context.port_selected => {
            let operation = match command {
                0x00 => DeckCommand::Stop,
                0x01 => DeckCommand::Play,
                0x04 => DeckCommand::Pause,
                0x06 => DeckCommand::Play,
                _ => return Ok(nak(NAK_UNDEFINED)),
            };
            run_command(operation, handler)
        }
        (1, 0x08, [a, b, c]) if context.port_selected => {
            let raw = (i32::from(*a) << 16) | (i32::from(*b) << 8) | i32::from(*c);
            let speed_q16 = if raw & 0x80_0000 != 0 {
                raw | !0xFF_FFFF
            } else {
                raw
            };
            run_command(DeckCommand::VariablePlay { speed_q16 }, handler)
        }
        (2, 0x24, id_bytes) if context.port_selected => {
            let Some(id) = decode_fixed_id(id_bytes) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            run_command(
                DeckCommand::CueClip {
                    id,
                    position: None,
                    duration: None,
                },
                handler,
            )
        }
        (0xA, 0x24, id_bytes) if context.port_selected => {
            let Some(id) = decode_variable_id(id_bytes) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            run_command(
                DeckCommand::CueClip {
                    id,
                    position: None,
                    duration: None,
                },
                handler,
            )
        }
        (2, 0x25, data) if context.port_selected => {
            if data.len() != 16 {
                return Ok(nak(NAK_UNDEFINED));
            }
            let Some(id) = decode_fixed_id(&data[..8]) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            let position = match decode_time(&data[8..12], context) {
                Ok(position) => position,
                Err(reply) => return Ok(reply),
            };
            let duration = match decode_time(&data[12..16], context) {
                Ok(duration) => duration,
                Err(reply) => return Ok(reply),
            };
            run_command(
                DeckCommand::CueClip {
                    id,
                    position: Some(position),
                    duration: Some(duration),
                },
                handler,
            )
        }
        (0xA, 0x25, data) if context.port_selected => {
            let Some((id, time_data)) = decode_variable_id_prefix(data, 8) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            let position = match decode_time(&time_data[..4], context) {
                Ok(position) => position,
                Err(reply) => return Ok(reply),
            };
            let duration = match decode_time(&time_data[4..], context) {
                Ok(duration) => duration,
                Err(reply) => return Ok(reply),
            };
            run_command(
                DeckCommand::CueClip {
                    id,
                    position: Some(position),
                    duration: Some(duration),
                },
                handler,
            )
        }
        (3, 0x05, [bitmap]) if context.port_selected => {
            if bitmap & !0x01 != 0 {
                return Ok(nak(NAK_UNDEFINED));
            }
            let state = match handler(DeckCommand::Status) {
                Ok(state) => state,
                Err(_) => return Ok(nak(NAK_UNDEFINED)),
            };
            let Some(port_id) = context.virtual_port_id else {
                return Ok(nak(NAK_UNDEFINED));
            };
            let Some(state_byte) = port_state_byte(state.transport) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            Ok(frame_response(
                command_1,
                0x85,
                &[*bitmap, state_byte, encode_signal_port(port_id)],
            )?)
        }
        (3, 0x06, [time_type]) if context.port_selected => {
            let state = match handler(DeckCommand::Status) {
                Ok(state) => state,
                Err(_) => return Ok(nak(NAK_UNDEFINED)),
            };
            let Some(position) = state.position else {
                return Ok(nak(NAK_UNDEFINED));
            };
            let position = match *time_type {
                0 => {
                    let Some(duration) = state.duration else {
                        return Ok(nak(NAK_UNDEFINED));
                    };
                    match remaining_time(duration, position) {
                        Some(remaining) => remaining,
                        None => return Ok(nak(NAK_UNDEFINED)),
                    }
                }
                1 => {
                    let Some(selected_id) = state.selected_clip_id.as_deref() else {
                        return Ok(nak(NAK_UNDEFINED));
                    };
                    let Some(source_start) = state
                        .catalog
                        .iter()
                        .find(|clip| clip.id == selected_id)
                        .and_then(|clip| clip.source_start)
                    else {
                        return Ok(nak(NAK_UNDEFINED));
                    };
                    match add_time(source_start, position) {
                        Some(som_position) => som_position,
                        None => return Ok(nak(NAK_UNDEFINED)),
                    }
                }
                2 => position,
                _ => return Ok(nak(NAK_UNDEFINED)),
            };
            let Ok(encoded) = encode_time(position, context) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            let mut response = vec![*time_type];
            response.extend_from_slice(&encoded);
            Ok(frame_response(command_1, 0x86, &response)?)
        }
        (3 | 0xB, 0x07, []) if context.port_selected => {
            let state = match handler(DeckCommand::Status) {
                Ok(state) => state,
                Err(_) => return Ok(nak(NAK_UNDEFINED)),
            };
            let active = is_active(state.transport);
            let mut data = vec![u8::from(active)];
            if active {
                let Some(id) = state.selected_clip_id.as_deref() else {
                    return Ok(nak(NAK_UNDEFINED));
                };
                if command_1 >> 4 == 0xB {
                    let Some(id) = encode_variable_id(id) else {
                        return Ok(nak(NAK_UNDEFINED));
                    };
                    data.extend_from_slice(&id);
                } else {
                    let Some(id) = encode_fixed_id(id) else {
                        return Ok(nak(NAK_UNDEFINED));
                    };
                    data.extend_from_slice(&id);
                }
            }
            Ok(frame_response(sense_reply_unit(command_1), 0x87, &data)?)
        }
        (3, 0x08, []) => Ok(nak(NAK_UNDEFINED)),
        (3 | 0xB, 0x11, []) => {
            let state = match handler(DeckCommand::Catalog {
                start: 0,
                count: MAX_CATALOG_IDS as u32,
            }) {
                Ok(state) => state,
                Err(_) => return Ok(nak(NAK_UNDEFINED)),
            };
            let Some(ids) = snapshot_ids(state.catalog) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            context.catalog_ids = ids;
            context.catalog_cursor = 0;
            context.catalog_variable_ids = command_1 >> 4 == 0xB;
            list_ids(sense_reply_unit(command_1), 0x91, context, 0)
        }
        (3 | 0xB, 0x02, []) => {
            context.catalog_variable_ids = command_1 >> 4 == 0xB;
            let start = context.catalog_cursor;
            list_ids(sense_reply_unit(command_1), 0x82, context, start)
        }
        (3, 0x14, id_bytes) => {
            let Some(id) = decode_fixed_id(id_bytes) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            let state = match handler(DeckCommand::ClipInfo { id: id.clone() }) {
                Ok(state) => state,
                Err(_) => return Ok(nak(NAK_UNDEFINED)),
            };
            let Some(duration) = state
                .catalog
                .iter()
                .find(|clip| clip.id == id)
                .and_then(|clip| clip.duration)
            else {
                return Ok(nak(NAK_UNDEFINED));
            };
            let Ok(duration) = encode_time(duration, context) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            Ok(frame_response(
                sense_reply_unit(command_1),
                0x94,
                &duration,
            )?)
        }
        (0xB, 0x14, id_data) => {
            let Some(id) = decode_variable_id(id_data) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            let state = match handler(DeckCommand::ClipInfo { id: id.clone() }) {
                Ok(state) => state,
                Err(_) => return Ok(nak(NAK_UNDEFINED)),
            };
            let Some(duration) = state
                .catalog
                .iter()
                .find(|clip| clip.id == id)
                .and_then(|clip| clip.duration)
            else {
                return Ok(nak(NAK_UNDEFINED));
            };
            let Ok(duration) = encode_time(duration, context) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            Ok(frame_response(
                sense_reply_unit(command_1),
                0x94,
                &duration,
            )?)
        }
        (3, 0x16, id_data) => id_request(id_data, false, command_1, handler),
        (0xB, 0x16, id_data) => id_request(id_data, true, command_1, handler),
        _ => Ok(nak(NAK_UNDEFINED)),
    }
}

fn validate_frame(frame: &[u8]) -> Result<(u8, u8, &[u8]), String> {
    if frame.len() < 5 || frame.len() > MAX_FRAME_BYTES || frame[0] != 0x02 {
        return Err(
            "VDCP frame must start with STX and contain at least two command bytes.".into(),
        );
    }
    let byte_count = usize::from(frame[1]);
    if !(2..=97).contains(&byte_count) || frame.len() != byte_count + 3 {
        return Err("VDCP byte count does not match the frame length.".into());
    }
    if frame[2..]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
        != 0
    {
        return Err("VDCP checksum is invalid.".into());
    }
    Ok((frame[2], frame[3], &frame[4..frame.len() - 1]))
}

fn run_command(
    command: DeckCommand,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    match handler(command) {
        Ok(_) => Ok(vec![ACK]),
        Err(_) => Ok(nak(NAK_UNDEFINED)),
    }
}

fn nak(code: u8) -> Vec<u8> {
    vec![NAK, code]
}

fn frame_response(command_1: u8, command_2: u8, data: &[u8]) -> Result<Vec<u8>, String> {
    let byte_count = data.len() + 2;
    if !(2..=97).contains(&byte_count) || data.len() + 5 > MAX_FRAME_BYTES {
        return Err("VDCP response data exceeds the frame byte-count limit.".into());
    }
    let mut response = Vec::with_capacity(data.len() + 5);
    response.push(0x02);
    response.push(byte_count as u8);
    response.push(command_1);
    response.push(command_2);
    response.extend_from_slice(data);
    let checksum = response[2..]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_sub(*byte));
    response.push(checksum);
    Ok(response)
}

fn decode_signal_port(value: u8) -> Option<i8> {
    let magnitude = i16::from(value & 0x7F);
    if magnitude == 0 {
        return None;
    }
    let signed = if value & 0x80 == 0 {
        magnitude
    } else {
        -magnitude
    };
    i8::try_from(signed).ok()
}

fn encode_signal_port(value: i8) -> u8 {
    value.unsigned_abs() | if value < 0 { 0x80 } else { 0 }
}

fn configured_port_matches(context: &ProtocolContext, value: u8) -> bool {
    context
        .virtual_port_id
        .is_some_and(|port_id| decode_signal_port(value) == Some(port_id))
}

fn decode_fixed_id(bytes: &[u8]) -> Option<String> {
    if bytes.len() != 8 || bytes.iter().any(|byte| !(0x20..=0x7E).contains(byte)) {
        return None;
    }
    let id = String::from_utf8(bytes.to_vec()).ok()?;
    let id = id.trim_end_matches(' ');
    if id.is_empty() {
        return None;
    }
    Some(id.to_owned())
}

fn encode_fixed_id(id: &str) -> Option<[u8; 8]> {
    if id.is_empty() || id.len() > 8 || !id.is_ascii() || id.ends_with(' ') {
        return None;
    }
    if id.bytes().any(|byte| !(0x20..=0x7E).contains(&byte)) {
        return None;
    }
    let mut output = [b' '; 8];
    output[..id.len()].copy_from_slice(id.as_bytes());
    Some(output)
}

fn decode_variable_id(bytes: &[u8]) -> Option<String> {
    let (id, remaining) = decode_variable_id_prefix(bytes, 0)?;
    if !remaining.is_empty() {
        return None;
    }
    Some(id)
}

fn decode_variable_id_prefix(bytes: &[u8], trailing_bytes: usize) -> Option<(String, &[u8])> {
    let length = usize::from(*bytes.first()?);
    if length == 0 || length > MAX_VARIABLE_ID_BYTES || bytes.len() != length + 1 + trailing_bytes {
        return None;
    }
    let id_bytes = &bytes[1..1 + length];
    if id_bytes.iter().any(|byte| !(0x21..=0x7E).contains(byte)) {
        return None;
    }
    let id = std::str::from_utf8(id_bytes).ok()?.to_owned();
    Some((id, &bytes[1 + length..]))
}

fn encode_variable_id(id: &str) -> Option<Vec<u8>> {
    if id.is_empty()
        || id.len() > MAX_VARIABLE_ID_BYTES
        || !id.is_ascii()
        || id.bytes().any(|byte| !(0x21..=0x7E).contains(&byte))
    {
        return None;
    }
    let mut output = Vec::with_capacity(id.len() + 1);
    output.push(u8::try_from(id.len()).ok()?);
    output.extend_from_slice(id.as_bytes());
    Some(output)
}

fn snapshot_ids(clips: Vec<DeckClip>) -> Option<Vec<String>> {
    if clips.len() > MAX_CATALOG_IDS {
        return None;
    }
    let mut ids = Vec::with_capacity(clips.len());
    for clip in clips {
        encode_variable_id(&clip.id)?;
        if clip.id.len() > MAX_LIST_ID_BYTES - 1 {
            return None;
        }
        if ids.iter().any(|id| id == &clip.id) {
            return None;
        }
        ids.push(clip.id);
    }
    Some(ids)
}

fn list_ids(
    command_1: u8,
    reply_command: u8,
    context: &mut ProtocolContext,
    start: u32,
) -> Result<Vec<u8>, String> {
    let start = usize::try_from(start).map_err(|_| "VDCP catalog cursor is out of range.")?;
    if start > context.catalog_ids.len() {
        return Ok(nak(NAK_UNDEFINED));
    }
    let mut data = vec![0, 0];
    let mut page_end = start;
    if context.catalog_variable_ids {
        while page_end < context.catalog_ids.len() {
            let Some(encoded) = encode_variable_id(&context.catalog_ids[page_end]) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            if data.len() - 2 + encoded.len() > MAX_LIST_ID_BYTES {
                break;
            }
            data.extend_from_slice(&encoded);
            page_end += 1;
        }
    } else {
        page_end = start.saturating_add(10).min(context.catalog_ids.len());
        for id in &context.catalog_ids[start..page_end] {
            let Some(bytes) = encode_fixed_id(id) else {
                return Ok(nak(NAK_UNDEFINED));
            };
            data.extend_from_slice(&bytes);
        }
    }
    if page_end == start && start < context.catalog_ids.len() {
        return Ok(nak(NAK_UNDEFINED));
    }
    let remaining = context.catalog_ids.len() - page_end;
    data[0] = (remaining >> 8) as u8;
    data[1] = remaining as u8;
    context.catalog_cursor = page_end as u32;
    frame_response(command_1, reply_command, &data)
}

fn decode_time(bytes: &[u8], context: &ProtocolContext) -> Result<DeckTimecode, Vec<u8>> {
    if bytes.len() != 4 {
        return Err(nak(NAK_UNDEFINED));
    }
    let drop_frame = bytes[0] & 0x40 != 0;
    if drop_frame != context.drop_frame {
        return Err(nak(NAK_UNDEFINED));
    }
    let Ok(nominal) = context.frame_rate.nominal_fps() else {
        return Err(nak(NAK_UNDEFINED));
    };
    let frame_byte = bytes[0] & 0x3F;
    let Ok(hour) = super::from_bcd(bytes[3]) else {
        return Err(nak(NAK_UNDEFINED));
    };
    let Ok(minute) = super::from_bcd(bytes[2]) else {
        return Err(nak(NAK_UNDEFINED));
    };
    let Ok(second) = super::from_bcd(bytes[1]) else {
        return Err(nak(NAK_UNDEFINED));
    };
    let Ok(frame_value) = super::from_bcd(frame_byte) else {
        return Err(nak(NAK_UNDEFINED));
    };
    let (frames, drop_frame) = if nominal == 50 {
        if drop_frame {
            return Err(nak(NAK_UNDEFINED));
        }
        let field = u32::from(bytes[0] >> 7);
        (
            frame_value
                .checked_mul(2)
                .and_then(|v| v.checked_add(field))
                .ok_or_else(|| nak(NAK_UNDEFINED))?,
            false,
        )
    } else {
        if bytes[0] & 0x80 != 0 {
            return Err(nak(NAK_UNDEFINED));
        }
        (frame_value, drop_frame)
    };
    let Ok(timecode) =
        DeckTimecode::from_components(hour, minute, second, frames, context.frame_rate, drop_frame)
    else {
        return Err(nak(NAK_UNDEFINED));
    };
    Ok(timecode)
}

fn encode_time(time: DeckTimecode, context: &ProtocolContext) -> Result<[u8; 4], String> {
    if time.frame_rate != context.frame_rate || time.drop_frame != context.drop_frame {
        return Err("VDCP timecode does not match the connection timebase.".into());
    }
    let mut bytes = time.to_bcd()?;
    if time.frame_rate.nominal_fps()? == 50 {
        let (_, _, _, frames) = time.components()?;
        bytes[0] = super::bcd(frames / 2)? | (((frames % 2) as u8) << 7);
    }
    if time.drop_frame {
        bytes[0] |= 0x40;
    }
    Ok(bytes)
}

fn id_request(
    id_data: &[u8],
    variable: bool,
    command_1: u8,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    let id = if variable {
        decode_variable_id(id_data)
    } else {
        decode_fixed_id(id_data)
    };
    let Some(id) = id else {
        return Ok(nak(NAK_UNDEFINED));
    };
    let state = match handler(DeckCommand::Catalog {
        start: 0,
        count: MAX_CATALOG_IDS as u32,
    }) {
        Ok(state) => state,
        Err(_) => return Ok(nak(NAK_UNDEFINED)),
    };
    let present = state.catalog.iter().any(|clip| clip.id == id);
    Ok(frame_response(
        sense_reply_unit(command_1),
        0x96,
        &[u8::from(present), 0],
    )?)
}

fn sense_reply_unit(command_1: u8) -> u8 {
    0x30 | (command_1 & 0x0F)
}

fn remaining_time(duration: DeckTimecode, position: DeckTimecode) -> Option<DeckTimecode> {
    if duration.frame_rate != position.frame_rate
        || duration.drop_frame != position.drop_frame
        || position.frame_number > duration.frame_number
    {
        return None;
    }
    Some(DeckTimecode {
        frame_number: duration.frame_number - position.frame_number,
        frame_rate: duration.frame_rate,
        drop_frame: duration.drop_frame,
    })
}

fn add_time(left: DeckTimecode, right: DeckTimecode) -> Option<DeckTimecode> {
    if left.frame_rate != right.frame_rate || left.drop_frame != right.drop_frame {
        return None;
    }
    Some(DeckTimecode {
        frame_number: left.frame_number.checked_add(right.frame_number)?,
        frame_rate: left.frame_rate,
        drop_frame: left.drop_frame,
    })
}

fn port_state_byte(transport: DeckTransport) -> Option<u8> {
    match transport {
        DeckTransport::Stopped => Some(0x01),
        DeckTransport::Playing => Some(0x04),
        DeckTransport::Paused => Some(0x08),
        DeckTransport::FastForward | DeckTransport::Rewind => Some(0x24),
        DeckTransport::Variable { .. } => Some(0x24),
        DeckTransport::Unknown => None,
    }
}

fn is_active(transport: DeckTransport) -> bool {
    matches!(
        transport,
        DeckTransport::Playing
            | DeckTransport::Paused
            | DeckTransport::FastForward
            | DeckTransport::Rewind
            | DeckTransport::Variable { .. }
    )
}
