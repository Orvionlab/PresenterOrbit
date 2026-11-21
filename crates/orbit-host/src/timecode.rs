use serde::Serialize;

/// SMPTE frame rates carried by LTC and MTC. MTC does not distinguish 29.97
/// non-drop from 30, so that rate can only be selected explicitly for LTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum FrameRate {
    #[serde(rename = "24")]
    Fps24,
    #[serde(rename = "25")]
    Fps25,
    #[serde(rename = "29.97df")]
    Fps2997Drop,
    #[serde(rename = "29.97ndf")]
    Fps2997NonDrop,
    #[serde(rename = "30")]
    Fps30,
}

impl FrameRate {
    pub fn nominal(self) -> u32 {
        match self {
            Self::Fps24 => 24,
            Self::Fps25 => 25,
            Self::Fps2997Drop | Self::Fps2997NonDrop | Self::Fps30 => 30,
        }
    }

    /// Real frames per second of the signal.
    pub fn frames_per_second(self) -> f64 {
        match self {
            Self::Fps2997Drop | Self::Fps2997NonDrop => 30000.0 / 1001.0,
            other => f64::from(other.nominal()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timecode {
    pub hours: u32,
    pub minutes: u32,
    pub seconds: u32,
    pub frames: u32,
    pub rate: FrameRate,
}

impl Timecode {
    /// Rejects field values that cannot exist at the frame rate, including the
    /// dropped frame numbers 0 and 1 at the start of non-tenth minutes.
    pub fn new(
        hours: u32,
        minutes: u32,
        seconds: u32,
        frames: u32,
        rate: FrameRate,
    ) -> Option<Self> {
        let dropped = rate == FrameRate::Fps2997Drop && seconds == 0 && minutes % 10 != 0;
        (hours < 24
            && minutes < 60
            && seconds < 60
            && frames < rate.nominal()
            && !(dropped && frames < 2))
            .then_some(Self {
                hours,
                minutes,
                seconds,
                frames,
                rate,
            })
    }

    /// Number of frames elapsed since 00:00:00:00.
    pub fn frame_count(&self) -> u64 {
        let nominal = u64::from(self.rate.nominal());
        let total_minutes = u64::from(self.hours) * 60 + u64::from(self.minutes);
        let count =
            (total_minutes * 60 + u64::from(self.seconds)) * nominal + u64::from(self.frames);
        if self.rate == FrameRate::Fps2997Drop {
            // Two frame numbers are skipped every minute except every tenth minute.
            count - 2 * (total_minutes - total_minutes / 10)
        } else {
            count
        }
    }

    /// Media time of the start of this frame.
    pub fn seconds(&self) -> f64 {
        self.frame_count() as f64 / self.rate.frames_per_second()
    }

    /// Apply a time offset while keeping the resulting label on a frame
    /// boundary. The returned label wraps at the rate's 24-hour frame count.
    pub fn offset_by_seconds(&self, offset: f64) -> Self {
        if offset == 0.0 {
            return *self;
        }
        let nominal = u64::from(self.rate.nominal());
        let day_frames = nominal * 24 * 60 * 60
            - if self.rate == FrameRate::Fps2997Drop {
                2 * (24 * 60 - 24 * 6)
            } else {
                0
            };
        let shifted = (self.frame_count() as f64 + offset * self.rate.frames_per_second())
            .round()
            .rem_euclid(day_frames as f64) as u64;
        Self::from_frame_count(shifted, self.rate)
    }

    pub fn label(&self) -> String {
        let separator = if self.rate == FrameRate::Fps2997Drop {
            ';'
        } else {
            ':'
        };
        format!(
            "{:02}:{:02}:{:02}{separator}{:02}",
            self.hours, self.minutes, self.seconds, self.frames
        )
    }

    pub fn from_frame_count(count: u64, rate: FrameRate) -> Self {
        let nominal = u64::from(rate.nominal());
        let mut count = count;
        if rate == FrameRate::Fps2997Drop {
            let per_ten_minutes = nominal * 60 * 10 - 18;
            let tens = count / per_ten_minutes;
            let rest = count % per_ten_minutes;
            let dropped = if rest < 2 {
                0
            } else {
                (rest - 2) / (nominal * 60 - 2)
            };
            count += 18 * tens + 2 * dropped;
        }
        Self {
            hours: (count / (nominal * 3600) % 24) as u32,
            minutes: (count / (nominal * 60) % 60) as u32,
            seconds: (count / nominal % 60) as u32,
            frames: (count % nominal) as u32,
            rate,
        }
    }
}
