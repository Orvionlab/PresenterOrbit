//! HDR / color-space model for native display outputs: settings vocabulary,
//! platform validation, and the reference transfer-function math that the GPU
//! present shader in `native_hdr_gpu.rs` mirrors.

use serde_json::Value;
use std::sync::OnceLock;

/// BT.2408 reference white for SDR graphics white inside an HDR signal.
pub const SDR_REFERENCE_WHITE_NITS: f32 = 203.0;
/// Nominal HLG display peak used for the BT.2100 inverse OOTF.
pub const HLG_NOMINAL_PEAK_NITS: f32 = 1000.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Sdr,
    Hlg,
    Pq,
}

impl OutputFormat {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "sdr" => Ok(Self::Sdr),
            "hlg" => Ok(Self::Hlg),
            "pq" => Ok(Self::Pq),
            _ => Err("Screen output format must be sdr, hlg or pq.".to_string()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sdr => "sdr",
            Self::Hlg => "hlg",
            Self::Pq => "pq",
        }
    }

    pub fn is_hdr(self) -> bool {
        self != Self::Sdr
    }

    pub fn shader_mode(self) -> u32 {
        match self {
            Self::Sdr => 0,
            Self::Pq => 1,
            Self::Hlg => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BitDepth {
    Normal,
    Enhanced,
}

impl BitDepth {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "normal" => Ok(Self::Normal),
            "enhanced" => Ok(Self::Enhanced),
            _ => Err("Rendering bit depth must be normal or enhanced.".to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    Windows,
    MacAppleSilicon,
    MacIntel,
    Other,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Self::MacAppleSilicon
        } else if cfg!(target_os = "macos") {
            Self::MacIntel
        } else {
            Self::Other
        }
    }
}

/// Rejects HDR formats the target, bit depth or platform cannot render.
pub fn validate_output_format(
    format: OutputFormat,
    target_type: &str,
    bit_depth: BitDepth,
    platform: Platform,
) -> Result<(), String> {
    if !format.is_hdr() {
        return Ok(());
    }
    if target_type != "display" {
        return Err(
            "HDR output formats are only available for display outputs; NDI, capture and streaming stay SDR."
                .to_string(),
        );
    }
    if bit_depth != BitDepth::Enhanced {
        return Err(
            "HDR output formats require the Enhanced (10-bit) rendering bit depth.".to_string(),
        );
    }
    match (platform, format) {
        (Platform::Windows, OutputFormat::Hlg) => {
            Err("Rec. 2100 HLG is not available on Windows display outputs; use PQ.".to_string())
        }
        (Platform::MacIntel, _) => Err("HDR output is not available on Intel Macs.".to_string()),
        (Platform::Other, _) => {
            Err("HDR display output is not available on this platform.".to_string())
        }
        _ => Ok(()),
    }
}

/// Validates every screen's persisted `outputFormat` against `bit_depth`.
pub fn validate_screens(
    screens: &Value,
    bit_depth: BitDepth,
    platform: Platform,
) -> Result<(), String> {
    if !screens.is_object() {
        return Ok(());
    }
    let expanded = crate::output_layout::expand(screens)?;
    let screens = expanded
        .as_object()
        .ok_or("Screen configuration must be an object.")?;
    for (screen_id, screen) in screens {
        let Some(format) = screen.get("outputFormat") else {
            continue;
        };
        let format = format
            .as_str()
            .ok_or_else(|| "Screen output format must be sdr, hlg or pq.".to_string())
            .and_then(OutputFormat::parse)?;
        let target_type = screen
            .get("targetType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        validate_output_format(format, target_type, bit_depth, platform)
            .map_err(|error| format!("Screen '{screen_id}': {error}"))?;
    }
    Ok(())
}

/// Reads the configured bit depth from a raw `generalSettings` object; an
/// absent value is the documented default `normal`.
pub fn configured_bit_depth(general_settings: &Value) -> Result<BitDepth, String> {
    match general_settings.get("renderingBitDepth") {
        None => Ok(BitDepth::Normal),
        Some(value) => value
            .as_str()
            .ok_or_else(|| "Rendering bit depth must be normal or enhanced.".to_string())
            .and_then(BitDepth::parse),
    }
}

static APPLIED_BIT_DEPTH: OnceLock<BitDepth> = OnceLock::new();

/// Records the startup bit depth. Changing it requires a restart, so it is
/// written once per process.
pub fn set_applied_bit_depth(bit_depth: BitDepth) {
    let _ = APPLIED_BIT_DEPTH.set(bit_depth);
}

/// Whether a process that applied `applied` can serve a controller that wants `requested`. The depth is fixed
/// for the life of a process, so a second request for another depth is refused instead of ignored.
pub fn bit_depth_compatible(applied: Option<BitDepth>, requested: BitDepth) -> Result<(), String> {
    match applied {
        Some(applied) if applied != requested => Err(format!(
            "The output process renders at {applied:?} bit depth and keeps it until it restarts; the application asks for {requested:?}."
        )),
        _ => Ok(()),
    }
}

pub fn applied_bit_depth_if_set() -> Option<BitDepth> {
    APPLIED_BIT_DEPTH.get().copied()
}

pub fn applied_bit_depth() -> BitDepth {
    APPLIED_BIT_DEPTH.get().copied().unwrap_or(BitDepth::Normal)
}

/// What an output surface actually applied, reported to the operator.
#[derive(Clone, Debug, PartialEq)]
pub struct HdrStatus {
    pub format: OutputFormat,
    pub surface_format: String,
    pub color_space: String,
    pub monitor_hdr: Option<bool>,
    pub monitor_max_nits: Option<f32>,
}

impl HdrStatus {
    pub fn summary(&self) -> String {
        if !self.format.is_hdr() {
            return format!("SDR, {}", self.surface_format);
        }
        let monitor = match self.monitor_hdr {
            Some(true) => match self.monitor_max_nits {
                Some(nits) => format!("display reports HDR ({nits:.0} nits peak)"),
                None => "display reports HDR".to_string(),
            },
            Some(false) => "display is NOT in HDR mode".to_string(),
            None => "display HDR state unknown".to_string(),
        };
        format!(
            "{} active: {}, {}, {monitor}",
            self.format.as_str().to_uppercase(),
            self.surface_format,
            self.color_space
        )
    }
}

// CPU reference for the GPU present shader, checked by the tests.
