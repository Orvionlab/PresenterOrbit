//! The pipe protocol between the application and its web capture helper, and the pixel conversions of the helper.
//! Every message is a little-endian `u32` length followed by that many bytes; a frame is a JSON header packet and
//! a pixel packet.
pub(super) use super::framing::{read_packet, write_packet};
use crate::locked::Locked;
use image::DynamicImage;
use orbit_host::web_capture::MAX_SIZE;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub(super) const MAX_HEADER: usize = 64 * 1024;
pub(super) const MAX_FRAME: usize = MAX_SIZE.0 as usize * MAX_SIZE.1 as usize * 4;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Wanted {
    pub key: String,
    pub url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Header {
    pub pages: Option<usize>,
    pub key: String,
    pub width: u32,
    pub height: u32,
    pub error: Option<String>,
}

pub(super) enum Captured {
    Pages(usize),
    Encoded(Vec<u8>, bool),
    Bgra {
        width: u32,
        height: u32,
        stride: usize,
        bytes: Vec<u8>,
    },
    Error(String),
}

pub(super) type Pending = Arc<Mutex<HashMap<String, Captured>>>;

pub(super) fn deliver(pending: &Pending, key: &str, captured: Captured) {
    pending.locked().insert(key.to_string(), captured);
}

fn straight(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        0
    } else {
        (u16::from(channel) * 255 / u16::from(alpha)).min(255) as u8
    }
}

pub(super) fn decode(captured: Captured) -> Result<image::RgbaImage, String> {
    match captured {
        Captured::Pages(_) => Err("A browser status is not a pixel frame.".into()),
        Captured::Error(error) => Err(error),
        Captured::Encoded(bytes, associated) => {
            let mut image = image::load_from_memory(&bytes)
                .map_err(|error| error.to_string())?
                .into_rgba8();
            if associated {
                for pixel in image.pixels_mut() {
                    for channel in 0..3 {
                        pixel[channel] = straight(pixel[channel], pixel[3]);
                    }
                }
            }
            Ok(image)
        }
        Captured::Bgra {
            width,
            height,
            stride,
            bytes,
        } => {
            if stride < width as usize * 4 || bytes.len() < stride * height as usize {
                return Err("The browser returned invalid pixel rows.".into());
            }
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for row in bytes.chunks(stride).take(height as usize) {
                for pixel in row[..width as usize * 4].as_chunks::<4>().0.iter() {
                    #[cfg(target_endian = "little")]
                    let (r, g, b, a) = (pixel[2], pixel[1], pixel[0], pixel[3]);
                    #[cfg(target_endian = "big")]
                    let (r, g, b, a) = (pixel[1], pixel[2], pixel[3], pixel[0]);
                    rgba.extend([straight(r, a), straight(g, a), straight(b, a), a]);
                }
            }
            image::RgbaImage::from_raw(width, height, rgba)
                .ok_or("The browser returned invalid image dimensions.".into())
        }
    }
}

/// The helper's reply for one capture: a header and its pixels. A page that fails is an error header, but a
/// frame over the size limit is a protocol error that ends the helper.
pub(super) fn encode(key: String, captured: Captured) -> Result<(Header, Vec<u8>), String> {
    if let Captured::Pages(pages) = captured {
        let header = Header {
            pages: Some(pages),
            key,
            width: 0,
            height: 0,
            error: None,
        };
        return Ok((header, Vec::new()));
    }
    match decode(captured) {
        Ok(image) => {
            let (width, height) = image.dimensions();
            if width > MAX_SIZE.0 || height > MAX_SIZE.1 {
                return Err("The browser capture exceeds the maximum frame size.".into());
            }
            let header = Header {
                pages: None,
                key,
                width,
                height,
                error: None,
            };
            Ok((header, image.into_raw()))
        }
        Err(error) => {
            let header = Header {
                pages: None,
                key,
                width: 0,
                height: 0,
                error: Some(error),
            };
            Ok((header, Vec::new()))
        }
    }
}

/// What a header and its pixels say about one page.
pub(super) enum Update {
    Error(String),
    Frame(DynamicImage),
}

/// Reads a non-status reply. Invalid dimensions or a pixel count that does not match them mean the helper is
/// broken, which is an error for the whole service.
pub(super) fn interpret(header: Header, bytes: Vec<u8>) -> Result<Update, String> {
    if let Some(error) = header.error {
        return Ok(Update::Error(error));
    }
    if header.width == 0
        || header.height == 0
        || header.width > MAX_SIZE.0
        || header.height > MAX_SIZE.1
    {
        return Err("The web capture helper returned invalid dimensions.".into());
    }
    // `from_raw` accepts a longer buffer and ignores the excess, which would hide a header that disagrees with
    // its pixels.
    if bytes.len() != header.width as usize * header.height as usize * 4 {
        return Err("The web capture helper returned an invalid frame length.".into());
    }
    image::RgbaImage::from_raw(header.width, header.height, bytes)
        .map(|image| Update::Frame(DynamicImage::ImageRgba8(image)))
        .ok_or_else(|| "The web capture helper returned an invalid frame length.".into())
}
