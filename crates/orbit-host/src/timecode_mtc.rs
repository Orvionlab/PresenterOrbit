//! MIDI time code: quarter-frame messages (`F1 dd`) and the universal
//! real-time full-frame SysEx (`F0 7F dev 01 01 hr mn sc fr F7`).

use crate::timecode::{FrameRate, Timecode};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MtcEvent {
    /// A complete timecode assembled from eight consecutive quarter frames.
    /// `seconds` is the media time of the last quarter frame.
    Running { timecode: Timecode, seconds: f64 },
    /// A locate to a new position, sent while the transport is stopped.
    Locate { timecode: Timecode, seconds: f64 },
}

#[derive(Default)]
pub struct MtcParser {
    pieces: [u8; 8],
    last: Option<u8>,
    /// Pieces collected in the current direction, starting from piece 0 or 7.
    count: u8,
    reverse: bool,
}

fn rate_from_code(code: u8) -> FrameRate {
    match code & 3 {
        0 => FrameRate::Fps24,
        1 => FrameRate::Fps25,
        2 => FrameRate::Fps2997Drop,
        _ => FrameRate::Fps30,
    }
}

impl MtcParser {
    pub fn message(&mut self, message: &[u8]) -> Option<MtcEvent> {
        match message {
            [0xF1, data] if *data < 0x80 => self.quarter_frame(*data),
            [0xF0, 0x7F, _, 0x01, 0x01, hours, minutes, seconds, frames, 0xF7] => {
                self.count = 0;
                let rate = rate_from_code(hours >> 5);
                let timecode = Timecode::new(
                    u32::from(hours & 0x1F),
                    u32::from(*minutes),
                    u32::from(*seconds),
                    u32::from(*frames),
                    rate,
                )?;
                Some(MtcEvent::Locate {
                    timecode,
                    seconds: timecode.seconds(),
                })
            }
            _ => None,
        }
    }

    fn quarter_frame(&mut self, data: u8) -> Option<MtcEvent> {
        let (piece, value) = (data >> 4, data & 0x0F);
        let step = self.last.map(|last| (piece + 8 - last) % 8);
        self.last = Some(piece);
        // A set is eight consecutive pieces ascending from 0, or descending from 7 when the
        // transport runs backwards; a gap or a change of direction discards it.
        let continues = self.count > 0 && step == Some(if self.reverse { 7 } else { 1 });
        if continues {
            self.count += 1;
        } else if piece == 0 || piece == 7 {
            (self.reverse, self.count) = (piece == 7, 1);
        } else {
            self.count = 0;
            return None;
        }
        self.pieces[usize::from(piece)] = value;
        if self.count < 8 {
            return None;
        }
        self.count = 0;
        let pieces = self.pieces;
        let rate = rate_from_code(pieces[7] >> 1);
        let timecode = Timecode::new(
            u32::from(pieces[6] | ((pieces[7] & 1) << 4)),
            u32::from(pieces[4] | (pieces[5] << 4)),
            u32::from(pieces[2] | (pieces[3] << 4)),
            u32::from(pieces[0] | (pieces[1] << 4)),
            rate,
        )?;
        // Forward, the set names the frame that began with piece 0, seven quarter frames ago.
        // Backwards, piece 0 arrives last, at that frame's first instant.
        let seconds = timecode.seconds()
            + if self.reverse {
                0.0
            } else {
                1.75 / rate.frames_per_second()
            };
        Some(MtcEvent::Running { timecode, seconds })
    }
}
