use serde_json::{json, Value};

pub(crate) fn parse(frame: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(frame).map_err(|_| "RossTalk commands must be ASCII.")?;
    if !text.is_ascii() {
        return Err("RossTalk commands must be ASCII.".into());
    }
    match text {
        "UP" => Ok(json!({"command":"previousItem"})),
        "DOWN" => Ok(json!({"command":"nextItem"})),
        "NEXT" => Ok(json!({"command":"nextSlide"})),
        "CLRA" | "CLFB 0" | "CLFB 0000" => Ok(json!({"command":"clearAll"})),
        _ => {
            let (buffer, layer) = text
                .strip_prefix("CLFB ")
                .and_then(|value| value.split_once(':'))
                .ok_or("Unsupported RossTalk device command.")?;
            if !matches!(buffer, "0" | "0000") {
                return Err("Only the ProPresenter framebuffer zero is supported.".into());
            }
            let command = match layer {
                "0" => "clearAll",
                "1" => "clearVideoInput",
                "2" => "clearMedia",
                "3" => "clearSlide",
                "4" => "clearProps",
                _ => return Err("Unsupported ProPresenter clear layer.".into()),
            };
            Ok(json!({"command":command}))
        }
    }
}

pub(crate) fn decoder() -> crate::communication_input::Decoder {
    let mut pending = Vec::new();
    Box::new(move |bytes| {
        if bytes.is_empty() && !pending.is_empty() {
            return Err("Incomplete RossTalk command timed out.".into());
        }
        let mut frames = Vec::new();
        for &byte in bytes {
            if !byte.is_ascii_graphic() && !matches!(byte, b' ' | b'\r' | b'\n') {
                return Err("Invalid RossTalk command framing.".into());
            }
            pending.push(byte);
            if pending.len() > 256 {
                return Err("RossTalk command exceeds 256 bytes.".into());
            }
            if byte == b'\n' {
                if !pending.ends_with(b"\r\n") || pending.len() < 3 {
                    return Err("RossTalk commands require CR/LF framing.".into());
                }
                pending.truncate(pending.len() - 2);
                frames.push(std::mem::take(&mut pending));
            }
        }
        Ok(frames)
    })
}
