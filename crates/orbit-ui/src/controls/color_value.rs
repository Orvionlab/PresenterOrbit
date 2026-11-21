//! Port of `src/renderer/ui/color-picker-color-math.ts`. Values are `#rrggbb` or `#rrggbbaa`; ProPresenter's hex
//! format puts alpha first.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

fn is_hex(text: &str) -> bool {
    text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn normalize_hex(value: &str, fallback: &str) -> String {
    let color = value.trim();
    match color.strip_prefix('#') {
        Some(digits) if matches!(digits.len(), 6 | 8) && is_hex(digits) => color.to_string(),
        _ => fallback.to_string(),
    }
}

pub fn is_transparent(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("transparent")
}

fn channel(hex: &str) -> u8 {
    u8::from_str_radix(hex, 16).unwrap_or_default()
}

pub fn hex_to_rgb(color: &str) -> Rgb {
    let color = normalize_hex(color, "#000000");
    Rgb {
        red: channel(&color[1..3]),
        green: channel(&color[3..5]),
        blue: channel(&color[5..7]),
    }
}

pub fn hex_alpha(color: &str) -> u8 {
    let color = normalize_hex(color, "");
    if color.len() == 9 {
        channel(&color[7..9])
    } else {
        255
    }
}

/// `#rrggbb`, with an alpha suffix only below full opacity.
pub fn rgb_to_hex(rgb: Rgb, alpha: u8) -> String {
    let Rgb { red, green, blue } = rgb;
    if alpha == 255 {
        format!("#{red:02x}{green:02x}{blue:02x}")
    } else {
        format!("#{red:02x}{green:02x}{blue:02x}{alpha:02x}")
    }
}

/// `#AARRGGBB` in upper case.
pub fn format_propresenter_hex(color: &str, alpha: u8) -> String {
    let color = normalize_hex(color, "#000000");
    format!("#{alpha:02X}{}", color[1..7].to_uppercase())
}

/// Accepts `AARRGGBB` or `RRGGBB`, with or without `#`.
pub fn parse_hex_input(value: &str) -> Option<(String, u8)> {
    let digits = value.trim().trim_start_matches('#');
    if !is_hex(digits) {
        return None;
    }
    match digits.len() {
        8 => {
            let alpha = channel(&digits[..2]);
            let suffix = if alpha < 255 { &digits[..2] } else { "" };
            Some((format!("#{}{suffix}", &digits[2..]), alpha))
        }
        6 => Some((format!("#{digits}"), 255)),
        _ => None,
    }
}

pub fn rgb_to_hue(rgb: Rgb) -> i32 {
    let (r, g, b) = (
        f64::from(rgb.red) / 255.,
        f64::from(rgb.green) / 255.,
        f64::from(rgb.blue) / 255.,
    );
    let max = r.max(g).max(b);
    let delta = max - r.min(g).min(b);
    if delta == 0. {
        return 0;
    }
    if max == r {
        // JavaScript's `%` keeps the sign, as Rust's does.
        return ((60. * (((g - b) / delta) % 6.)).round() as i32 + 360) % 360;
    }
    if max == g {
        return (60. * ((b - r) / delta + 2.)).round() as i32;
    }
    (60. * ((r - g) / delta + 4.)).round() as i32
}

/// Saturation and value, both 0 to 100.
pub fn rgb_to_hsv(rgb: Rgb) -> (i32, i32) {
    let (r, g, b) = (
        f64::from(rgb.red) / 255.,
        f64::from(rgb.green) / 255.,
        f64::from(rgb.blue) / 255.,
    );
    let max = r.max(g).max(b);
    let delta = max - r.min(g).min(b);
    let saturation = if max == 0. { 0. } else { delta / max };
    (
        (saturation * 100.).round() as i32,
        (max * 100.).round() as i32,
    )
}

pub fn hsv_to_rgb(hue: f64, saturation: f64, value: f64) -> Rgb {
    let hue = hue.rem_euclid(360.);
    let s = saturation.clamp(0., 100.) / 100.;
    let v = value.clamp(0., 100.) / 100.;
    let chroma = v * s;
    let second = chroma * (1. - ((hue / 60.) % 2. - 1.).abs());
    let matched = v - chroma;
    let (r, g, b) = match hue {
        h if h < 60. => (chroma, second, 0.),
        h if h < 120. => (second, chroma, 0.),
        h if h < 180. => (0., chroma, second),
        h if h < 240. => (0., second, chroma),
        h if h < 300. => (second, 0., chroma),
        _ => (chroma, 0., second),
    };
    let to_channel = |value: f64| ((value + matched) * 255.).round() as u8;
    Rgb {
        red: to_channel(r),
        green: to_channel(g),
        blue: to_channel(b),
    }
}

/// The color a new playlist header gets, as `collection-playlist-entry-actions.ts` writes it. It is data in the
/// workspace, like every user-authored color, and not part of the theme.
pub const DEFAULT_HEADER_COLOR: &str = "#64748b";

/// A user-authored `#rrggbb` color as a GPUI color. An invalid value is black, as [`hex_to_rgb`] reads it.
pub fn data_color(color: &str) -> gpui::Rgba {
    let rgb = hex_to_rgb(color);
    crate::color::opaque_rgb24(
        (u32::from(rgb.red) << 16) | (u32::from(rgb.green) << 8) | u32::from(rgb.blue),
    )
}

pub const STANDARD_COLORS: [&str; 9] = [
    "#ffffff", "#808080", "#000000", "#ff0000", "#008000", "#0000ff", "#ffff00", "#ffa500",
    "#800080",
];

pub const AVAILABLE_COLORS: [&str; 140] = [
    "#000000", "#696969", "#808080", "#a9a9a9", "#c0c0c0", "#d3d3d3", "#dcdcdc", "#f5f5f5",
    "#ffffff", "#bc8f8f", "#cd5c5c", "#a52a2a", "#b22222", "#f08080", "#800000", "#8b0000",
    "#ff0000", "#fffafa", "#ffe4e1", "#fa8072", "#ff6347", "#e9967a", "#ff7f50", "#ff4500",
    "#ffa07a", "#a0522d", "#fff5ee", "#d2691e", "#8b4513", "#f4a460", "#ffdab9", "#cd853f",
    "#faf0e6", "#ffe4c4", "#ff8c00", "#deb887", "#d2b48c", "#faebd7", "#ffdead", "#ffebcd",
    "#ffefd5", "#ffe4b5", "#ffa500", "#f5deb3", "#fdf5e6", "#fffaf0", "#b8860b", "#daa520",
    "#fff8dc", "#ffd700", "#f0e68c", "#fffacd", "#eee8aa", "#bdb76b", "#f5f5dc", "#fafad2",
    "#808000", "#ffff00", "#ffffe0", "#fffff0", "#6b8e23", "#9acd32", "#556b2f", "#adff2f",
    "#7fff00", "#7cfc00", "#8fbc8f", "#228b22", "#32cd32", "#90ee90", "#98fb98", "#006400",
    "#008000", "#00ff00", "#f0fff0", "#2e8b57", "#3cb371", "#00ff7f", "#f5fffa", "#00fa9a",
    "#66cdaa", "#7fffd4", "#40e0d0", "#20b2aa", "#48d1cc", "#2f4f4f", "#afeeee", "#008080",
    "#008b8b", "#00ffff", "#00ffff", "#e0ffff", "#f0ffff", "#00ced1", "#5f9ea0", "#b0e0e6",
    "#add8e6", "#00bfff", "#87ceeb", "#87cefa", "#4682b4", "#f0f8ff", "#1e90ff", "#708090",
    "#778899", "#b0c4de", "#6495ed", "#4169e1", "#191970", "#e6e6fa", "#000080", "#00008b",
    "#0000cd", "#0000ff", "#f8f8ff", "#6a5acd", "#483d8b", "#7b68ee", "#9370db", "#8a2be2",
    "#4b0082", "#9932cc", "#9400d3", "#ba55d3", "#d8bfd8", "#dda0dd", "#ee82ee", "#800080",
    "#8b008b", "#ff00ff", "#ff00ff", "#da70d6", "#c71585", "#ff1493", "#ff69b4", "#fff0f5",
    "#db7093", "#dc143c", "#ffc0cb", "#ffb6c1",
];
