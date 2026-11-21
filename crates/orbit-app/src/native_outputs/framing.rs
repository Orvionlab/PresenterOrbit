//! Length-prefixed packets: a little-endian `u32` length followed by that many bytes. Shared by the web capture
//! helper pipe and the output process socket.
use std::io::{Read, Write};

pub fn read_packet(reader: &mut impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut size = [0; 4];
    reader
        .read_exact(&mut size)
        .map_err(|error| format!("The pipe closed: {error}"))?;
    let size = u32::from_le_bytes(size) as usize;
    if size > limit {
        return Err("A pipe message exceeds its size limit.".into());
    }
    let mut bytes = vec![0; size];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

pub fn write_packet(writer: &mut impl Write, packet: &[u8]) -> Result<(), String> {
    writer
        .write_all(&(packet.len() as u32).to_le_bytes())
        .and_then(|_| writer.write_all(packet))
        .map_err(|error| error.to_string())
}
