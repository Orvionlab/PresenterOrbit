use serde_json::{json, Value};
use std::path::Path;

pub fn choose(workspace_path: &Path) -> Result<Option<Value>, String> {
    crate::workspace::read_workspace(workspace_path)?;
    let Some(path) = rfd::FileDialog::new()
        .set_title("Choose icon image")
        .add_filter("Image", &["png", "jpg", "jpeg"])
        .pick_file()
    else {
        return Ok(None);
    };
    let metadata =
        std::fs::metadata(&path).map_err(|error| format!("Macro image is unavailable: {error}"))?;
    if !metadata.is_file() || metadata.len() > 4 * 1024 * 1024 {
        return Err("Macro image must be a file smaller than 4 MiB.".into());
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("Macro image could not be read: {error}"))?;
    Ok(Some(validate(&bytes)?))
}

pub fn validate(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("Macro image exceeds 4 MiB.".into());
    }
    let format =
        image::guess_format(bytes).map_err(|error| format!("Macro image is corrupt: {error}"))?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err("Macro images must be PNG or JPEG.".into());
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|error| format!("Macro image is corrupt or exceeds image bounds: {error}"))?;
    Ok(json!({"imageData": encode(bytes), "imageType": "ImageTypeCustom"}))
}

pub fn validate_data(data: &str) -> Result<(), String> {
    validate(&decode_data(data)?).map(|_| ())
}

/// Decode canonical base64 image data without validating the image format.
pub fn decode_data(data: &str) -> Result<Vec<u8>, String> {
    if data.len() > 5_592_408 || data.len() % 4 != 0 {
        return Err("Macro image base64 is invalid or exceeds 4 MiB.".into());
    }
    let mut decoded = Vec::with_capacity(data.len() / 4 * 3);
    for chunk in data.as_bytes().chunks_exact(4) {
        let mut value = 0u32;
        let mut padding = 0;
        for byte in chunk {
            let digit = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => {
                    padding += 1;
                    0
                }
                _ => return Err("Macro image base64 contains invalid bytes.".into()),
            };
            value = (value << 6) | u32::from(digit);
        }
        decoded.push((value >> 16) as u8);
        if padding < 2 {
            decoded.push((value >> 8) as u8);
        }
        if padding == 0 {
            decoded.push(value as u8);
        }
    }
    if encode(&decoded) != data {
        return Err("Macro image base64 is not canonical.".into());
    }
    Ok(decoded)
}

fn encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 3) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((second & 15) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}
