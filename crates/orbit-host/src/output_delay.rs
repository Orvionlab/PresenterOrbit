use crate::native_compositor::NativeFrame;
use serde_json::Value;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const MAX_DELAY_FRAMES: usize = 120;
const MAX_BUFFER_BYTES: usize = 128 * 1024 * 1024;

pub fn frames(value: Option<&Value>) -> Result<usize, String> {
    match value {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .filter(|v| *v <= MAX_DELAY_FRAMES as u64)
            .map(|v| v as usize)
            .ok_or_else(|| "Video delay must be an integer from 0 to 120 frames.".into()),
    }
}

pub fn validate_size(width: usize, height: usize, delay: usize) -> Result<(), String> {
    if delay == 0 {
        return Ok(());
    }
    let bytes = width
        .checked_mul(height)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| "Video delay dimensions overflow.".to_string())?;
    if delay > MAX_DELAY_FRAMES
        || bytes
            .checked_mul(delay + 2)
            .is_none_or(|v| v > MAX_BUFFER_BYTES)
    {
        return Err("Video delay exceeds the 128 MiB buffer limit for this output size.".into());
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct OutputDelay {
    key: Option<(usize, usize, usize, Duration)>,
    queue: VecDeque<(Instant, NativeFrame)>,
    last_sample: Option<Instant>,
}

impl OutputDelay {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn push(
        &mut self,
        frame: NativeFrame,
        delay: usize,
        interval: Duration,
        now: Instant,
    ) -> Result<Option<NativeFrame>, String> {
        if delay == 0 {
            self.clear();
            return Ok(Some(frame));
        }
        let bytes = frame
            .width
            .checked_mul(frame.height)
            .and_then(|v| v.checked_mul(4))
            .ok_or_else(|| "Video delay dimensions overflow.".to_string())?;
        validate_size(frame.width, frame.height, delay)?;
        if interval.is_zero() || frame.pixels.len() != bytes {
            return Err(
                "Video delay requires a valid BGRA frame and positive frame interval.".into(),
            );
        }
        let key = (frame.width, frame.height, delay, interval);
        if self.key != Some(key) {
            self.clear();
            self.key = Some(key);
        }
        // Content mutations between clock ticks replace the newest sample; they do not shorten the delay.
        if self
            .last_sample
            .is_some_and(|last| now.saturating_duration_since(last) < interval)
        {
            if let Some((captured, newest)) = self.queue.back_mut() {
                *captured = now;
                *newest = frame;
            }
        } else {
            self.queue.push_back((now, frame));
            self.last_sample = Some(now);
        }
        let duration = interval
            .checked_mul(delay as u32)
            .ok_or_else(|| "Video delay duration overflow.".to_string())?;
        let mut ready = None;
        while self
            .queue
            .front()
            .is_some_and(|(captured, _)| now.saturating_duration_since(*captured) >= duration)
        {
            ready = self.queue.pop_front().map(|(_, frame)| frame);
        }
        // At most delay+2 slots survive even if callers submit irregular timestamps.
        if self.queue.len() > delay + 2 {
            return Err("Video delay queue exceeded its frame bound.".into());
        }
        Ok(ready)
    }
}
