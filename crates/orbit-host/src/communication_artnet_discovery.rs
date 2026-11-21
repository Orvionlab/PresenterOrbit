use std::net::Ipv4Addr;

pub(super) fn reply(
    bytes: &[u8],
    ip: Ipv4Addr,
    universe: u16,
    oem: u16,
    title: &str,
    counter: u16,
) -> Result<Option<Vec<u8>>, String> {
    if bytes.len() < 14
        || &bytes[..8] != b"Art-Net\0"
        || bytes[8..10] != [0, 0x20]
        || u16::from_be_bytes([bytes[10], bytes[11]]) < 14
    {
        return Err("Invalid ArtPoll packet.".into());
    }
    if bytes[12] & 0x20 != 0 {
        // Art-Net specifies zero for missing extension fields in a minimum-length poll.
        let field = |index| bytes.get(index).copied().unwrap_or(0);
        let top = u16::from_be_bytes([field(14), field(15)]);
        let bottom = u16::from_be_bytes([field(16), field(17)]);
        if !(bottom..=top).contains(&universe) {
            return Ok(None);
        }
    }
    let mut packet = vec![0; 239];
    packet[..8].copy_from_slice(b"Art-Net\0");
    packet[9] = 0x21;
    packet[10..14].copy_from_slice(&ip.octets());
    packet[14..16].copy_from_slice(&6454u16.to_le_bytes());
    packet[17] = 1;
    packet[18] = (universe >> 8) as u8;
    packet[19] = ((universe >> 4) & 15) as u8;
    packet[20..22].copy_from_slice(&oem.to_be_bytes());
    packet[23] = 0xd0;
    text(&mut packet[26..44], "Presenter Orbit");
    text(&mut packet[44..108], title);
    text(
        &mut packet[108..172],
        &format!("#0001 [{:04}] Ready", counter % 10000),
    );
    packet[173] = 1;
    packet[174] = 0x80;
    packet[190] = (universe & 15) as u8;
    packet[200] = 2;
    packet[207..211].copy_from_slice(&ip.octets());
    packet[211] = 1;
    packet[212] = 8;
    packet[213] = 0xb0;
    Ok(Some(packet))
}
fn text(target: &mut [u8], value: &str) {
    let count = target.len() - 1;
    for (out, byte) in target.iter_mut().take(count).zip(value.bytes()) {
        *out = if byte.is_ascii() && byte != 0 {
            byte
        } else {
            b'?'
        };
    }
}
