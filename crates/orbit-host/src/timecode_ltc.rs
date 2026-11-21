//! Linear timecode (SMPTE 12M-1) biphase-mark decoder for one audio channel.
//!
//! Every bit cell starts with a level transition and a `1` has a second one in
//! the middle of the cell. Only the spacing of transitions is used, so signal
//! polarity does not matter. Transitions are found with a Schmitt trigger whose
//! hysteresis follows the signal peak, and a cell length estimate adapts to the
//! signal so tape-speed variation and unknown frame rates are tolerated.

use crate::timecode::{FrameRate, Timecode};

const FRAME_BITS: u32 = 80;
const SYNC_WORD: u128 = 0x3FFD;
/// The sync word with its bit order reversed, as read when the signal plays backwards.
const REVERSED_SYNC_WORD: u128 = 0xBFFC;
const SYNC_MASK: u128 = 0xFFFF;
const FRAME_MASK: u128 = (1 << FRAME_BITS) - 1;
/// Signals quieter than this peak (about -48 dBFS) carry no timecode.
const MIN_PEAK: f32 = 0.004;
const MIN_HYSTERESIS: f32 = 0.002;
/// Fraction of the signal peak that a transition must cross.
const HYSTERESIS_RATIO: f32 = 0.3;
const PEAK_DECAY_SECONDS: f64 = 0.5;
/// A transition interval below this share of a cell is half a cell.
const SHORT_LIMIT: f64 = 0.75;
/// Longer than this share of a cell is not biphase-mark data.
const LONG_LIMIT: f64 = 1.5;
const BIT_PERIOD_SMOOTHING: f64 = 0.1;
/// Frame rates the cell estimate may settle on, with margin.
const MIN_FPS: f64 = 23.0;
const MAX_FPS: f64 = 31.0;
const START_FPS: f64 = 25.0;
/// Estimated and nominal frame rate must agree within this ratio.
const RATE_TOLERANCE: f64 = 0.08;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LtcFrame {
    pub timecode: Timecode,
    /// Fractional sample index at which the frame's last bit cell ended.
    pub end_sample: f64,
    /// The frame's bits arrived last-to-first, so the signal plays backwards.
    pub reverse: bool,
}

pub struct LtcDecoder {
    sample_rate: f64,
    format: Option<FrameRate>,
    sample_index: u64,
    peak: f32,
    peak_decay: f32,
    state: i8,
    previous: f32,
    last_edge: Option<f64>,
    bit_period: f64,
    pending_short: bool,
    register: u128,
    valid_bits: u32,
}

impl LtcDecoder {
    /// Decode using an explicitly selected SMPTE format when supplied. This
    /// overrides inferred nominal/drop-frame semantics while transition timing
    /// still drives frame synchronization.
    pub fn with_format(sample_rate: u32, format: Option<FrameRate>) -> Self {
        let sample_rate = f64::from(sample_rate.max(8000));
        Self {
            sample_rate,
            format,
            sample_index: 0,
            peak: 0.0,
            peak_decay: (-1.0 / (PEAK_DECAY_SECONDS * sample_rate)).exp() as f32,
            state: 0,
            previous: 0.0,
            last_edge: None,
            bit_period: sample_rate / (START_FPS * f64::from(FRAME_BITS)),
            pending_short: false,
            register: 0,
            valid_bits: 0,
        }
    }

    pub fn push(&mut self, samples: impl IntoIterator<Item = f32>) -> Vec<LtcFrame> {
        let mut frames = Vec::new();
        for sample in samples {
            if let Some(frame) = self.sample(sample) {
                frames.push(frame);
            }
        }
        frames
    }

    fn sample(&mut self, value: f32) -> Option<LtcFrame> {
        let index = self.sample_index as f64;
        self.sample_index += 1;
        let value = if value.is_finite() { value } else { 0.0 };
        self.peak = (self.peak * self.peak_decay).max(value.abs());
        let previous = std::mem::replace(&mut self.previous, value);
        if self.peak < MIN_PEAK {
            self.state = 0;
            self.last_edge = None;
            self.desynchronize();
            return None;
        }
        let threshold = (self.peak * HYSTERESIS_RATIO).max(MIN_HYSTERESIS);
        let next = if value > threshold {
            1
        } else if value < -threshold {
            -1
        } else {
            self.state
        };
        if next == self.state {
            return None;
        }
        let first = self.state == 0;
        self.state = next;
        if first {
            return None;
        }
        let level = f32::from(next) * threshold;
        let fraction = f64::from((level - previous) / (value - previous)).clamp(0.0, 1.0);
        let edge = index - 1.0 + fraction;
        let interval = self.last_edge.replace(edge).map(|last| edge - last)?;
        self.interval(interval, edge)
    }

    fn desynchronize(&mut self) {
        self.pending_short = false;
        self.register = 0;
        self.valid_bits = 0;
    }

    fn interval(&mut self, length: f64, edge: f64) -> Option<LtcFrame> {
        let cell = self.bit_period;
        if length >= cell * LONG_LIMIT {
            self.desynchronize();
            return None;
        }
        if length < cell * SHORT_LIMIT {
            if self.pending_short {
                self.pending_short = false;
                return self.bit(true, edge);
            }
            self.pending_short = true;
            return None;
        }
        if self.pending_short {
            self.desynchronize();
            return None;
        }
        let slowest = self.sample_rate / (MIN_FPS * f64::from(FRAME_BITS));
        let fastest = self.sample_rate / (MAX_FPS * f64::from(FRAME_BITS));
        self.bit_period = (cell + (length - cell) * BIT_PERIOD_SMOOTHING).clamp(fastest, slowest);
        self.bit(false, edge)
    }

    fn bit(&mut self, value: bool, edge: f64) -> Option<LtcFrame> {
        self.register = ((self.register << 1) | u128::from(value)) & FRAME_MASK;
        self.valid_bits = (self.valid_bits + 1).min(FRAME_BITS);
        if self.valid_bits < FRAME_BITS {
            return None;
        }
        let reverse = if self.register & SYNC_MASK == SYNC_WORD {
            false
        } else if (self.register >> (FRAME_BITS - 16)) & SYNC_MASK == REVERSED_SYNC_WORD {
            true
        } else {
            return None;
        };
        let timecode = self.parse(reverse)?;
        Some(LtcFrame {
            timecode,
            end_sample: edge,
            reverse,
        })
    }

    /// A backwards frame holds frame bit `n` at register bit `n`; a forward one at `79 - n`.
    fn field(&self, start: u32, length: u32, reverse: bool) -> u32 {
        (0..length).fold(0, |value, offset| {
            let bit = start + offset;
            let shift = if reverse { bit } else { FRAME_BITS - 1 - bit };
            value | (((self.register >> shift) & 1) as u32) << offset
        })
    }

    fn rate(&self, reverse: bool) -> Option<FrameRate> {
        if let Some(format) = self.format {
            return Some(format);
        }
        let estimate = self.sample_rate / (self.bit_period * f64::from(FRAME_BITS));
        let nearest = [24.0_f64, 25.0, 30.0]
            .into_iter()
            .min_by(|a, b| (estimate - a).abs().total_cmp(&(estimate - b).abs()))?;
        if (estimate / nearest - 1.0).abs() > RATE_TOLERANCE {
            return None;
        }
        let dropped = self.field(10, 1, reverse) == 1;
        match (nearest as u32, dropped) {
            (30, true) => Some(FrameRate::Fps2997Drop),
            (_, true) => None,
            (24, false) => Some(FrameRate::Fps24),
            (25, false) => Some(FrameRate::Fps25),
            _ => Some(FrameRate::Fps30),
        }
    }

    fn parse(&self, reverse: bool) -> Option<Timecode> {
        let rate = self.rate(reverse)?;
        let field = |start, length| self.field(start, length, reverse);
        Timecode::new(
            field(48, 4) + 10 * field(56, 2),
            field(32, 4) + 10 * field(40, 3),
            field(16, 4) + 10 * field(24, 3),
            field(0, 4) + 10 * field(8, 2),
            rate,
        )
    }
}

/// Time of the end of the frame: a frame's timecode names its first instant
/// and the decoder only knows the frame once its last bit has ended.
pub fn seconds_at_frame_end(timecode: &Timecode) -> f64 {
    timecode.seconds() + 1.0 / timecode.rate.frames_per_second()
}
