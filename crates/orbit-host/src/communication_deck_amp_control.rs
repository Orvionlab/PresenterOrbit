use super::super::{DeckCommand, DeckState, DeckTransport, ProtocolContext};
use super::media::{extended_reply, name};

pub(super) fn process(
    frame: &[u8],
    context: &mut ProtocolContext,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Option<Vec<u8>>, String> {
    let reply = match frame {
        [0xa0, 0x0f | 0x2a | 0x2b] | [0xa2, 0x0e, ..] => folder(frame, context),
        [0xa0, 0x16] => loaded(handler),
        [0xa2, 0x25, ..] => clip_start(&frame[2..], handler),
        [0x61, 0x20, range] => status(*range, context, handler),
        [0x61, 0x0c, mask] => position(*mask, handler),
        _ => return Ok(None),
    };
    Ok(Some(reply.unwrap_or_else(|_| vec![0x11, 0x11])))
}

fn named_payload(value: &str) -> Vec<u8> {
    let mut payload = (value.len() as u16).to_be_bytes().to_vec();
    payload.extend(value.as_bytes());
    payload
}

fn extended_name(data: &[u8]) -> Result<&[u8], String> {
    let [high, low, name_high, name_low, bytes @ ..] = data else {
        return Err("AMP name lengths are incomplete.".into());
    };
    if usize::from(u16::from_be_bytes([*high, *low])) != bytes.len() + 2
        || usize::from(u16::from_be_bytes([*name_high, *name_low])) != bytes.len()
    {
        return Err("AMP name lengths do not match the request.".into());
    }
    Ok(bytes)
}

fn folder(frame: &[u8], context: &mut ProtocolContext) -> Result<Vec<u8>, String> {
    let folder = context
        .amp_folder
        .as_deref()
        .ok_or("No AMP media bin is configured.")?;
    match frame {
        [0xa0, 0x0f] => extended_reply(0x0f, named_payload(folder)),
        [0xa0, 0x2a] => {
            let reply = extended_reply(0x2a, named_payload(folder))?;
            context.amp_folder_listed = true;
            Ok(reply)
        }
        [0xa0, 0x2b] if context.amp_folder_listed => Ok(vec![0x80, 0x2b]),
        [0xa2, 0x0e, data @ ..] => {
            if extended_name(data)? != folder.as_bytes() {
                return Err("The requested folder is outside the selected AMP media bin.".into());
            }
            context.catalog_cursor = 0;
            context.catalog_ids.clear();
            Ok(vec![0x10, 0x01])
        }
        _ => Err("Restart AMP folder enumeration with List First Folder.".into()),
    }
}

fn loaded(
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    let state = handler(DeckCommand::Status)?;
    let Some(id) = state.selected_clip_id else {
        return Ok(vec![0x80, 0x16]);
    };
    if name(id.as_bytes())? != id {
        return Err("The loaded AMP clip name is invalid.".into());
    }
    extended_reply(0x16, named_payload(&id))
}

fn clip_start(
    data: &[u8],
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    let id = name(extended_name(data)?)?;
    let state = handler(DeckCommand::Catalog { start: 0, count: 0 })?;
    if !state.catalog.iter().any(|clip| clip.id == id) {
        return Ok(vec![0x80, 0x25]);
    }
    let state = handler(DeckCommand::ClipInfo { id: id.clone() })?;
    let position = state
        .catalog
        .iter()
        .find(|clip| clip.id == id)
        .and_then(|clip| clip.source_start)
        .ok_or("The AMP source start is unavailable.")?;
    let mut reply = vec![0x84, 0x25];
    reply.extend(position.to_sony_time_data()?);
    Ok(reply)
}

fn status(
    range: u8,
    context: &ProtocolContext,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    let start = usize::from(range >> 4);
    let count = usize::from(range & 0xf);
    if count == 0 || start + count > 15 {
        return Err("AMP status range is invalid.".into());
    }
    let state = handler(DeckCommand::Status)?;
    let mut status = [0u8; 15];
    status[1] = 0x80
        | match state.transport {
            DeckTransport::Playing => 0x01,
            DeckTransport::Stopped | DeckTransport::Paused => 0x20,
            DeckTransport::FastForward => 0x04,
            DeckTransport::Rewind => 0x08,
            DeckTransport::Variable { .. } => 0x01,
            DeckTransport::Unknown => return Err("AMP transport state is unavailable.".into()),
        };
    if state.selected_clip_id.is_some() {
        status[2] = 0x81;
        status[2] |= match state.transport {
            DeckTransport::Paused | DeckTransport::Stopped => 0x02,
            DeckTransport::FastForward => 0x20,
            DeckTransport::Rewind => 0x24,
            DeckTransport::Variable { speed_q16 } => 0x08 | if speed_q16 < 0 { 0x04 } else { 0 },
            _ => 0,
        };
        if context
            .amp_cue
            .as_ref()
            .is_some_and(|(id, _)| Some(id) == state.selected_clip_id.as_ref())
        {
            status[3] = 0x01;
        }
    }
    let mut reply = vec![0x70 | count as u8, 0x20];
    reply.extend(&status[start..start + count]);
    Ok(reply)
}

fn position(
    mask: u8,
    handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
) -> Result<Vec<u8>, String> {
    if !matches!(
        mask,
        1 | 2 | 3 | 4 | 0x10 | 0x20 | 0x30 | 0x11 | 0x22 | 0x33 | 0x40 | 0x80
    ) {
        return Err("Unsupported AMP time-data mask.".into());
    }
    if !matches!(mask, 1 | 3 | 4) {
        return Ok(vec![0x70, 0x0d]);
    }
    let state = handler(DeckCommand::Status)?;
    let Some(position) = state.position else {
        return Ok(vec![0x70, 0x0d]);
    };
    let mut reply = vec![0x74, 0x04];
    reply.extend(position.to_sony_time_data()?);
    Ok(reply)
}
