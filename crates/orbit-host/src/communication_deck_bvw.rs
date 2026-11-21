use super::{DeckCommand, DeckState, DeckTimecode, ProtocolContext};

pub(super) fn process(
    frame: &[u8],
    context: &mut ProtocolContext,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    let Some((&command_1, rest)) = frame.split_first() else {
        return Err("Sony BVW command frame is empty.".into());
    };
    let data_count = usize::from(command_1 & 0x0F);
    let expected_len = data_count + 3;
    if frame.len() != expected_len {
        return Err("Sony BVW command length does not match its data-count nibble.".into());
    }
    let command_2 = rest[0];
    let data = &rest[1..rest.len() - 1];
    if additive_checksum(&frame[..frame.len() - 1]) != frame[frame.len() - 1] {
        return Err("Sony BVW command checksum is invalid.".into());
    }
    if command_1 >> 4 == 0x06 {
        return sense(command_2, data, context, handler);
    }
    if command_1 >> 4 != 0x02 {
        return Ok(nak());
    }

    let command = match (command_2, data) {
        (0x00, []) => DeckCommand::Stop,
        (0x01, []) => DeckCommand::Play,
        (0x10, []) => DeckCommand::FastForward,
        (0x20, []) => DeckCommand::Rewind,
        (0x31, [frame, seconds, minutes, hours]) => {
            let position = DeckTimecode::from_sony_time_data(
                [*frame, *seconds, *minutes, *hours],
                context.frame_rate,
                context.drop_frame,
            );
            match position {
                Ok(position) => DeckCommand::Seek { position },
                Err(_) => return Ok(nak()),
            }
        }
        _ => return Ok(nak()),
    };
    match handler(command) {
        Ok(_) => Ok(ack()),
        Err(_) => Ok(nak()),
    }
}

fn sense(
    command: u8,
    data: &[u8],
    context: &ProtocolContext,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    match (command, data) {
        (0x0C, [mask]) if *mask == 0x01 => {
            let state = match handler(DeckCommand::Status) {
                Ok(state) => state,
                Err(_) => return Ok(nak()),
            };
            let Some(position) = state.position else {
                return Ok(nak());
            };
            let Ok(time_data) = position.to_sony_time_data() else {
                return Ok(nak());
            };
            Ok(data_response(0x04, &time_data))
        }
        (0x20, [range]) if *range == 0x11 => {
            let state = match handler(DeckCommand::Status) {
                Ok(state) => state,
                Err(_) => return Ok(nak()),
            };
            let value = match state.transport {
                DeckTransport::Playing => 0x01,
                DeckTransport::FastForward => 0x04,
                DeckTransport::Rewind => 0x08,
                DeckTransport::Stopped | DeckTransport::Paused => 0x20,
                _ => return Ok(nak()),
            };
            Ok(data_response(0x20, &[value]))
        }
        _ => {
            let _ = context;
            Ok(nak())
        }
    }
}

use super::DeckTransport;

fn additive_checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

fn ack() -> Vec<u8> {
    vec![0x10, 0x01, 0x11]
}

fn nak() -> Vec<u8> {
    vec![0x11, 0x12, 0x01, 0x24]
}

fn data_response(command: u8, data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(data.len() + 3);
    frame.push(0x70 | data.len() as u8);
    frame.push(command);
    frame.extend_from_slice(data);
    frame.push(additive_checksum(&frame));
    frame
}
