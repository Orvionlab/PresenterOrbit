use super::super::{DeckCommand, DeckState, DeckTimecode, ProtocolContext};

const NAK: [u8; 2] = [0x11, 0x11];

pub(super) fn process(
    frame: &[u8],
    context: &mut ProtocolContext,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Option<Vec<u8>>, String> {
    let [kind, code, data @ ..] = frame else {
        return Err("AMP command header is incomplete.".into());
    };
    if !matches!((*kind >> 4, *code), (4, 0x14) | (0xa, 0x14 | 0x15))
        && !(*kind >> 4 == 2 && *code == 0x31 && matches!(*kind & 0xf, 8 | 0xc | 0xa | 0xe))
    {
        return Ok(None);
    }
    let count = *kind & 0xf;
    let extended = matches!((*kind >> 4, count), (2 | 4, 0xa | 0xe) | (4 | 0xa, 2));
    let data = if extended {
        let [high, low, payload @ ..] = data else {
            return Err("AMP extended length is missing.".into());
        };
        if usize::from(u16::from_be_bytes([*high, *low])) != payload.len() {
            return Err("AMP extended length does not match its payload.".into());
        }
        payload
    } else {
        if usize::from(count) != data.len() {
            return Err("AMP byte count does not match its payload.".into());
        }
        data
    };
    let reply = if *kind >> 4 == 0xa {
        list(*code, count, data, context, handler)
    } else {
        cue(count, data, context, handler)
    };
    Ok(Some(reply.unwrap_or_else(|_| NAK.to_vec())))
}

pub(super) fn name(bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty()
        || bytes.len() > 32
        || bytes
            .iter()
            .any(|byte| !byte.is_ascii_graphic() && *byte != b' ')
    {
        return Err("AMP clip names require 1 to 32 printable ASCII bytes.".into());
    }
    let name = std::str::from_utf8(bytes)
        .map_err(|_| "Invalid AMP clip name.")?
        .trim_end_matches(' ');
    if name.is_empty() {
        return Err("AMP clip name is empty.".into());
    }
    Ok(name.to_string())
}

pub(super) fn extended_reply(code: u8, payload: Vec<u8>) -> Result<Vec<u8>, String> {
    let length = u16::try_from(payload.len()).map_err(|_| "AMP reply is too large.")?;
    let mut reply = vec![0x82, code];
    reply.extend(length.to_be_bytes());
    reply.extend(payload);
    Ok(reply)
}

fn list(
    code: u8,
    count: u8,
    data: &[u8],
    context: &mut ProtocolContext,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    let (extended, requested) = match (code, count, data) {
        (0x14, 0, []) | (0x15, 0, []) => (false, 1),
        (0x14, 2, []) => (true, 1),
        (0x15, 1, [number]) if *number > 0 && *number <= 20 => (true, usize::from(*number)),
        _ => return Err("Unsupported AMP catalog request.".into()),
    };
    let first = code == 0x14;
    let start = if first {
        0
    } else {
        context.catalog_cursor as usize
    };
    let state = handler(DeckCommand::Catalog {
        start: start as u32,
        count: requested as u32,
    })?;
    let ids: Vec<String> = state.catalog.iter().map(|clip| clip.id.clone()).collect();
    if !first && ids != context.catalog_ids {
        return Err("AMP catalog changed; restart the listing.".into());
    }
    let selected = ids.iter().skip(start).take(requested);
    let mut payload = Vec::new();
    let mut returned = 0;
    for id in selected {
        if name(id.as_bytes())? != *id || (!extended && id.len() > 8) {
            return Err("AMP clip name does not fit the requested name format.".into());
        }
        if extended {
            payload.extend((id.len() as u16).to_be_bytes());
            payload.extend(id.as_bytes());
        } else {
            payload.extend(id.as_bytes());
            payload.resize(8, b' ');
        }
        returned += 1;
    }
    let reply = if returned == 0 {
        vec![0x80, 0x14]
    } else if extended {
        let mut reply = extended_reply(0x14, payload)?;
        reply[0] = 0x8a;
        reply
    } else {
        let mut reply = vec![0x88, 0x14];
        reply.extend(payload);
        reply
    };
    context.catalog_cursor = (start + returned) as u32;
    context.catalog_ids = ids;
    Ok(reply)
}

fn cue(
    count: u8,
    data: &[u8],
    context: &mut ProtocolContext,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    if count == 2 && data.is_empty() {
        let state = handler(DeckCommand::Status)?;
        let (id, position) = context.amp_cue.as_ref().ok_or("No AMP cue is loaded.")?;
        if state.selected_clip_id.as_ref() != Some(id) {
            return Err("The AMP cue is no longer selected.".into());
        }
        let mut payload = (id.len() as u16).to_be_bytes().to_vec();
        payload.extend(position.to_sony_time_data()?);
        payload.extend(id.as_bytes());
        return extended_reply(0x14, payload);
    }
    let (id, position) = match count {
        0 | 4 => {
            let state = handler(DeckCommand::Status)?;
            let id = state.selected_clip_id.ok_or("No AMP clip is selected.")?;
            let position = if count == 4 {
                Some(time(data, context)?)
            } else {
                None
            };
            (id, position)
        }
        8 => (name(data)?, None),
        0xc => (name(&data[4..])?, Some(time(&data[..4], context)?)),
        0xa | 0xe => {
            let (kind, rest) = if count == 0xe {
                let (kind, rest) = data.split_first().ok_or("AMP cue type is missing.")?;
                (*kind, rest)
            } else {
                (0xa, data)
            };
            let [high, low, rest @ ..] = rest else {
                return Err("AMP cue name length is missing.".into());
            };
            let (position, bytes) = match kind {
                0xa => (None, rest),
                0xe if rest.len() >= 4 => (Some(time(&rest[..4], context)?), &rest[4..]),
                _ => return Err("Unsupported AMP cue type.".into()),
            };
            if usize::from(u16::from_be_bytes([*high, *low])) != bytes.len() {
                return Err("AMP cue must contain exactly one named clip.".into());
            }
            (name(bytes)?, position)
        }
        _ => return Err("Unsupported AMP preset request.".into()),
    };
    let start = position.unwrap_or(DeckTimecode {
        frame_number: 0,
        frame_rate: context.frame_rate,
        drop_frame: context.drop_frame,
    });
    handler(DeckCommand::CueClip {
        id: id.clone(),
        position,
        duration: None,
    })?;
    context.amp_cue = Some((id, start));
    Ok(vec![0x10, 0x01])
}

fn time(bytes: &[u8], context: &ProtocolContext) -> Result<DeckTimecode, String> {
    DeckTimecode::from_sony_time_data(
        bytes
            .try_into()
            .map_err(|_| "AMP timecode must contain four bytes.")?,
        context.frame_rate,
        context.drop_frame,
    )
}
