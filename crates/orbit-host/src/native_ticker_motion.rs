use serde_json::{Map, Value};

pub(crate) struct TickerMotionFrame {
    pub offset: f64,
    pub second_offset: f64,
    pub repeated: bool,
    pub copies: usize,
}

pub(crate) fn frame(
    style: &Map<String, Value>,
    viewport: f64,
    extent: f64,
    scale: f64,
    elapsed: f64,
) -> Option<TickerMotionFrame> {
    let speed = style.get("scrollSpeed")?.as_f64()? * 5.0 * scale;
    if !speed.is_finite() || speed <= 0.0 || !elapsed.is_finite() {
        return None;
    }
    let forward = match style.get("scrollDirection")?.as_str()? {
        "left" | "up" => true,
        "right" | "down" => false,
        _ => return None,
    };
    let repeat = style.get("scrollRepeat")?.as_bool()?;
    let delay = style.get("scrollLoopDelaySeconds").and_then(Value::as_f64);
    let offscreen = style.get("scrollStart").and_then(Value::as_str) == Some("Off Screen")
        || (repeat && delay.is_some());
    let elapsed = elapsed.max(0.0);
    let elapsed = if let Some(delay) = delay.filter(|_| repeat) {
        let duration = (viewport + extent) / speed;
        let phase = elapsed.rem_euclid(duration + delay);
        if phase >= duration {
            return None;
        }
        phase
    } else {
        elapsed
    };
    let start = if offscreen {
        if forward {
            viewport
        } else {
            -extent
        }
    } else {
        0.0
    };
    let mut offset = start
        + if forward {
            -speed * elapsed
        } else {
            speed * elapsed
        };
    let repeated = repeat && delay.is_none();
    let (second_offset, copies) = if repeated {
        let gap = style
            .get("scrollRepeatDistance")
            .and_then(Value::as_f64)
            .unwrap_or(100.0)
            * scale;
        let period = extent + gap;
        if forward {
            if offset < -extent {
                offset += ((-extent - offset) / period).ceil() * period;
            }
            (
                offset + period,
                ((viewport - offset) / period).floor().max(0.0) as usize + 1,
            )
        } else {
            if !offscreen && offset < viewport {
                offset += ((viewport - offset) / period).floor() * period;
            }
            if offset > viewport {
                offset -= ((offset - viewport) / period).ceil() * period;
            }
            (
                offset - period,
                ((offset + extent) / period).floor().max(0.0) as usize + 1,
            )
        }
    } else {
        (offset, 1)
    };
    Some(TickerMotionFrame {
        offset,
        second_offset,
        repeated,
        copies,
    })
}
