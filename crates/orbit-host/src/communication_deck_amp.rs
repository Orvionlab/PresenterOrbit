//! Grass Valley AMP over TCP: newline-terminated ASCII messages, a four-digit decimal length and
//! Sony 9-pin commands as hex without a checksum. Replies are the 9-pin reply in hex without a
//! checksum, and `1111` for a rejected command, as the AMP clients we could check expect.

use super::{bvw, AmpSession, DeckCommand, DeckState, ProtocolContext};

#[path = "communication_deck_amp_control.rs"]
mod control;
#[path = "communication_deck_amp_media.rs"]
mod media;

const ACK: &[u8] = b"1001";
const NAK: &[u8] = b"1111";

pub(super) fn process(
    line: &[u8],
    context: &mut ProtocolContext,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    let line = std::str::from_utf8(line).map_err(|_| "AMP message is not ASCII.")?;
    if !line.is_ascii() {
        return Err("AMP message is not ASCII.".into());
    }
    let line = line.strip_suffix('\r').unwrap_or(line);
    let (Some(kind), Some(count), Some(body)) = (line.get(..4), line.get(4..8), line.get(8..))
    else {
        return Err("AMP message is shorter than its header.".into());
    };
    if !count.bytes().all(|byte| byte.is_ascii_digit()) || count.parse() != Ok(body.len()) {
        return Err("AMP message length does not match its body.".into());
    }
    match kind {
        "CRAT" => {
            if context.amp_session.is_some() {
                return Ok(NAK.to_vec());
            }
            let named = body.len() > 3
                && body.starts_with('2')
                && body[1..3].bytes().all(|byte| byte.is_ascii_digit())
                && body[3..].bytes().all(|byte| byte.is_ascii_graphic())
                && body[1..3].parse() == Ok(body.len() - 3);
            context.amp_session = if named {
                Some(AmpSession::Channel(body[3..].into()))
            } else if body == "4" {
                Some(AmpSession::ChannelLess)
            } else {
                return Err("AMP channel request is malformed.".into());
            };
            Ok(ACK.to_vec())
        }
        "STOP" if body.is_empty() => {
            if !matches!(
                context.amp_session,
                Some(AmpSession::Channel(_) | AmpSession::ChannelLess)
            ) {
                return Ok(NAK.to_vec());
            }
            context.amp_session = Some(AmpSession::Closed);
            context.amp_cue = None;
            context.catalog_ids.clear();
            context.catalog_cursor = 0;
            Ok(ACK.to_vec())
        }
        "CMDS" => {
            if body.len() % 2 != 0 || !body.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("AMP command is not whole hex bytes.".into());
            }
            let mut frame: Vec<u8> = (0..body.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&body[index..index + 2], 16).unwrap())
                .collect();
            let channel =
                matches!(&context.amp_session, Some(AmpSession::Channel(name)) if !name.is_empty());
            let catalog_command = matches!(frame.as_slice(), [0xa0, 0x0f | 0x2a | 0x2b])
                || matches!(
                    frame.as_slice(),
                    [0xa2, 0x0e | 0x14 | 0x25, ..] | [0xa0, 0x14 | 0x15] | [0xa1, 0x15, _]
                );
            if !channel
                && !(matches!(context.amp_session, Some(AmpSession::ChannelLess))
                    && catalog_command)
            {
                return Ok(NAK.to_vec());
            }
            if let Some(reply) = control::process(&frame, context, handler)? {
                return Ok(hex(&reply));
            }
            if let Some(reply) = media::process(&frame, context, handler)? {
                return Ok(hex(&reply));
            }
            frame.push(frame.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
            let reply = bvw::process(&frame, context, handler)?;
            if reply.first() == Some(&0x11) {
                return Ok(NAK.to_vec());
            }
            Ok(reply[..reply.len() - 1]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
                .into_bytes())
        }
        _ => Err(format!("Unsupported AMP message: {kind}")),
    }
}

fn hex(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .into_bytes()
}
