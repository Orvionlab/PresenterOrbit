//! Incoming media-deck protocol framing and shared command types.

#[path = "communication_deck_amp.rs"]
mod amp;
#[path = "communication_deck_bvw.rs"]
mod bvw;
#[path = "communication_deck_vdcp.rs"]
mod vdcp;

const MAX_BUFFERED_BYTES: usize = 65_536;
/// A header plus a 9-pin frame of at most 256 bytes as hex.
const MAX_AMP_LINE: usize = 8 + 512 + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    SonyBvw,
    Vdcp,
    Amp,
}

impl Protocol {
    pub fn from_name(name: &str) -> Result<Self, String> {
        match name {
            "sony-bvw" => Ok(Self::SonyBvw),
            "vdcp" => Ok(Self::Vdcp),
            "amp" => Ok(Self::Amp),
            _ => Err(format!("Unsupported media-deck protocol: {name}")),
        }
    }

    pub fn process(
        &self,
        frame: &[u8],
        context: &mut ProtocolContext,
        handler: &mut impl FnMut(DeckCommand) -> Result<DeckState, String>,
    ) -> Result<Vec<u8>, String> {
        match self {
            Self::SonyBvw => bvw::process(frame, context, handler),
            Self::Vdcp => vdcp::process(frame, context, handler),
            Self::Amp => amp::process(frame, context, handler),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl FrameRate {
    pub fn nominal_fps(self) -> Result<u32, String> {
        if self.numerator == 0 || self.denominator == 0 {
            return Err("Frame rate must be a non-zero rational value.".into());
        }
        let rounded = (u64::from(self.numerator) + u64::from(self.denominator) / 2)
            / u64::from(self.denominator);
        let nominal = u32::try_from(rounded).map_err(|_| "Frame rate is out of range.")?;
        if !matches!(
            (self.numerator, self.denominator, nominal),
            (24, 1, 24)
                | (25, 1, 25)
                | (30, 1, 30)
                | (50, 1, 50)
                | (60, 1, 60)
                | (24_000, 1_001, 24)
                | (30_000, 1_001, 30)
                | (60_000, 1_001, 60)
        ) {
            return Err("Unsupported explicit media-deck frame rate.".into());
        }
        Ok(nominal)
    }

    fn drop_frames_per_minute(self, drop_frame: bool) -> Result<u32, String> {
        if !drop_frame {
            return Ok(0);
        }
        if self
            == (Self {
                numerator: 30_000,
                denominator: 1_001,
            })
        {
            return Ok(2);
        }
        Err("Drop-frame timecode is supported only at 30000/1001 fps.".into())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeckTimecode {
    pub frame_number: u64,
    pub frame_rate: FrameRate,
    pub drop_frame: bool,
}

impl DeckTimecode {
    pub fn from_components(
        hours: u32,
        minutes: u32,
        seconds: u32,
        frames: u32,
        frame_rate: FrameRate,
        drop_frame: bool,
    ) -> Result<Self, String> {
        let nominal = frame_rate.nominal_fps()?;
        let drop = frame_rate.drop_frames_per_minute(drop_frame)?;
        if hours >= 24 || minutes >= 60 || seconds >= 60 || frames >= nominal {
            return Err("Timecode component is out of range for its configured frame rate.".into());
        }
        let total_minutes = hours * 60 + minutes;
        if drop_frame && minutes % 10 != 0 && seconds == 0 && frames < drop {
            return Err("The requested drop-frame timecode label does not exist.".into());
        }
        let nominal_count = u64::from(((hours * 60 + minutes) * 60 + seconds) * nominal + frames);
        let dropped = u64::from(drop * (total_minutes - total_minutes / 10));
        Ok(Self {
            frame_number: nominal_count - dropped,
            frame_rate,
            drop_frame,
        })
    }

    pub fn components(self) -> Result<(u32, u32, u32, u32), String> {
        let nominal = self.frame_rate.nominal_fps()?;
        let drop = self.frame_rate.drop_frames_per_minute(self.drop_frame)?;
        let frames_per_day = if self.drop_frame {
            u64::from(nominal) * 3_600 * 24 - u64::from(drop) * 9 * 24 * 6
        } else {
            u64::from(nominal) * 3_600 * 24
        };
        let frame_number = self.frame_number % frames_per_day;
        let mut low = 0u32;
        let mut high = 24 * 60;
        while low + 1 < high {
            let middle = (low + high) / 2;
            let dropped = u64::from(drop * (middle - middle / 10));
            let mut start = u64::from(middle) * 60 * u64::from(nominal) - dropped;
            if self.drop_frame && middle % 10 != 0 {
                start += u64::from(drop);
            }
            if start <= frame_number {
                low = middle;
            } else {
                high = middle;
            }
        }
        let minute = low;
        let dropped = u64::from(drop * (minute - minute / 10));
        let mut start = u64::from(minute) * 60 * u64::from(nominal) - dropped;
        if self.drop_frame && minute % 10 != 0 {
            start += u64::from(drop);
        }
        let mut in_minute = frame_number - start;
        if self.drop_frame && minute % 10 != 0 {
            in_minute += u64::from(drop);
        }
        let hours = minute / 60;
        let minutes = minute % 60;
        let seconds = (in_minute / u64::from(nominal)) as u32;
        let frames = (in_minute % u64::from(nominal)) as u32;
        Ok((hours, minutes, seconds, frames))
    }

    pub fn to_bcd(self) -> Result<[u8; 4], String> {
        let (hours, minutes, seconds, frames) = self.components()?;
        Ok([bcd(frames)?, bcd(seconds)?, bcd(minutes)?, bcd(hours)?])
    }

    pub fn from_bcd(
        bytes: [u8; 4],
        frame_rate: FrameRate,
        drop_frame: bool,
    ) -> Result<Self, String> {
        let frames = from_bcd(bytes[0])?;
        let seconds = from_bcd(bytes[1])?;
        let minutes = from_bcd(bytes[2])?;
        let hours = from_bcd(bytes[3])?;
        Self::from_components(hours, minutes, seconds, frames, frame_rate, drop_frame)
    }

    pub fn to_sony_time_data(self) -> Result<[u8; 4], String> {
        let mut bytes = self.to_bcd()?;
        if self.drop_frame {
            bytes[0] |= 0x40;
        }
        Ok(bytes)
    }

    pub fn from_sony_time_data(
        mut bytes: [u8; 4],
        frame_rate: FrameRate,
        configured_drop_frame: bool,
    ) -> Result<Self, String> {
        let drop_frame = bytes[0] & 0x40 != 0;
        if drop_frame != configured_drop_frame {
            return Err(
                "Sony timecode drop-frame flag conflicts with connection configuration.".into(),
            );
        }
        if bytes[0] & 0x80 != 0 {
            return Err("Sony timecode frame byte contains unsupported flags.".into());
        }
        bytes[0] &= 0xBF;
        Self::from_bcd(bytes, frame_rate, drop_frame)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeckClip {
    pub id: String,
    pub title: Option<String>,
    pub duration: Option<DeckTimecode>,
    pub source_start: Option<DeckTimecode>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeckTransport {
    Playing,
    Stopped,
    Paused,
    FastForward,
    Rewind,
    Variable { speed_q16: i32 },
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeckState {
    pub transport: DeckTransport,
    pub position: Option<DeckTimecode>,
    pub selected_clip_id: Option<String>,
    pub duration: Option<DeckTimecode>,
    pub catalog: Vec<DeckClip>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeckCommand {
    Play,
    Stop,
    Pause,
    FastForward,
    Rewind,
    VariablePlay {
        speed_q16: i32,
    },
    Seek {
        position: DeckTimecode,
    },
    CueClip {
        id: String,
        position: Option<DeckTimecode>,
        duration: Option<DeckTimecode>,
    },
    ClipInfo {
        id: String,
    },
    Catalog {
        start: u32,
        count: u32,
    },
    Status,
}

#[derive(Clone, Debug)]
pub enum AmpSession {
    Channel(String),
    ChannelLess,
    Closed,
}

#[derive(Clone, Debug)]
pub struct ProtocolContext {
    pub frame_rate: FrameRate,
    pub drop_frame: bool,
    pub unit_address: Option<u8>,
    pub virtual_port_id: Option<i8>,
    pub port_open: bool,
    pub port_selected: bool,
    pub catalog_cursor: u32,
    pub catalog_ids: Vec<String>,
    pub catalog_variable_ids: bool,
    pub amp_cue: Option<(String, DeckTimecode)>,
    pub amp_session: Option<AmpSession>,
    pub amp_folder: Option<String>,
    pub amp_folder_listed: bool,
}

impl ProtocolContext {
    pub fn new(frame_rate: FrameRate, drop_frame: bool) -> Self {
        Self {
            frame_rate,
            drop_frame,
            unit_address: None,
            virtual_port_id: None,
            port_open: false,
            port_selected: false,
            catalog_cursor: 0,
            catalog_ids: Vec::new(),
            catalog_variable_ids: false,
            amp_cue: None,
            amp_session: None,
            amp_folder: None,
            amp_folder_listed: false,
        }
    }

    pub fn with_unit_address(mut self, unit_address: u8) -> Result<Self, String> {
        if unit_address > 0x0F {
            return Err("VDCP unit address must be in the 0..=15 range.".into());
        }
        self.unit_address = Some(unit_address);
        Ok(self)
    }

    pub fn with_virtual_port_id(mut self, port_id: i8) -> Result<Self, String> {
        if !(1..=127).contains(&port_id) {
            return Err("VDCP output signal port must be in the 1..=127 range.".into());
        }
        self.virtual_port_id = Some(port_id);
        Ok(self)
    }
}

#[derive(Clone, Debug)]
pub struct FrameDecoder {
    protocol: Protocol,
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub fn new(protocol: Protocol, _frame_rate: FrameRate, _drop_frame: bool) -> Self {
        Self {
            protocol,
            buffer: Vec::new(),
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.buffer.len().saturating_add(bytes.len()) > MAX_BUFFERED_BYTES {
            self.buffer.clear();
            return Err("Media-deck input buffer exceeded its bounded capacity.".into());
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    pub fn next(&mut self) -> Result<Option<Vec<u8>>, String> {
        match self.protocol {
            Protocol::SonyBvw => self.next_counted(),
            Protocol::Vdcp => self.next_vdcp(),
            Protocol::Amp => self.next_line(),
        }
    }

    /// One AMP message without its newline terminator.
    fn next_line(&mut self) -> Result<Option<Vec<u8>>, String> {
        let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') else {
            if self.buffer.len() > MAX_AMP_LINE {
                self.buffer.clear();
                return Err("AMP message exceeds its maximum length.".into());
            }
            return Ok(None);
        };
        if end > MAX_AMP_LINE {
            self.buffer.clear();
            return Err("AMP message exceeds its maximum length.".into());
        }
        let line: Vec<_> = self.buffer.drain(..=end).collect();
        Ok(Some(line[..end].to_vec()))
    }

    fn next_counted(&mut self) -> Result<Option<Vec<u8>>, String> {
        if self.buffer.is_empty() {
            return Ok(None);
        }
        let count = usize::from(self.buffer[0] & 0x0F);
        let frame_len = 3 + count;
        if frame_len > 256 {
            self.buffer.clear();
            return Err("Media-deck frame length exceeds 256 bytes.".into());
        }
        if self.buffer.len() < frame_len {
            return Ok(None);
        }
        let frame: Vec<_> = self.buffer.drain(..frame_len).collect();
        let sum: u8 = frame[..frame.len() - 1]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        if sum != frame[frame.len() - 1] {
            return Err("Media-deck frame checksum is invalid.".into());
        }
        Ok(Some(frame))
    }

    fn next_vdcp(&mut self) -> Result<Option<Vec<u8>>, String> {
        if self.buffer.is_empty() {
            return Ok(None);
        }
        if self.buffer[0] != 0x02 {
            if let Some(start) = self.buffer.iter().position(|byte| *byte == 0x02) {
                self.buffer.drain(..start);
            } else {
                self.buffer.clear();
            }
            return Err("VDCP frame did not start with STX.".into());
        }
        if self.buffer.len() < 2 {
            return Ok(None);
        }
        let byte_count = usize::from(self.buffer[1]);
        if !(2..=97).contains(&byte_count) {
            self.buffer.drain(..1);
            return Err("VDCP frame exceeds the protocol's 100-byte maximum.".into());
        }
        let frame_len = byte_count + 3;
        if self.buffer.len() < frame_len {
            return Ok(None);
        }
        let frame: Vec<_> = self.buffer.drain(..frame_len).collect();
        let sum: u8 = frame[2..]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        if sum != 0 {
            return Err("VDCP frame checksum is invalid.".into());
        }
        Ok(Some(frame))
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

pub(crate) fn bcd(value: u32) -> Result<u8, String> {
    if value > 99 {
        return Err("Timecode value does not fit in BCD.".into());
    }
    Ok((((value / 10) << 4) | (value % 10)) as u8)
}

pub(crate) fn from_bcd(value: u8) -> Result<u32, String> {
    let tens = u32::from(value >> 4);
    let ones = u32::from(value & 0x0F);
    if tens > 9 || ones > 9 {
        return Err("Timecode contains an invalid BCD digit.".into());
    }
    Ok(tens * 10 + ones)
}
