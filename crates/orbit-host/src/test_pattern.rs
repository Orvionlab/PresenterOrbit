use crate::native_render::NativeRenderRuntime;
use image::{imageops::FilterType, DynamicImage, GenericImageView};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{path::Path, sync::OnceLock, time::Instant};

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Pattern {
    #[serde(rename = "blend-grid", rename_all = "camelCase")]
    Grid {
        spacing: usize,
        center_circle: bool,
        center_lines: bool,
        inverted: bool,
        #[serde(default = "enabled")]
        grid: bool,
    },
    #[serde(rename = "color-bars")]
    Bars {},
    #[serde(rename = "focus")]
    Focus {
        #[serde(default)]
        grid: bool,
        #[serde(default = "default_spacing")]
        spacing: usize,
    },
    #[serde(rename = "gray-scale")]
    Grayscale {},
    #[serde(rename = "black")]
    Black { level: f64 },
    #[serde(rename = "white")]
    White { level: f64 },
    #[serde(rename = "custom")]
    Custom { color: String },
    #[serde(rename = "lines")]
    Lines {
        color: String,
        background: String,
        speed: f64,
    },
    #[serde(rename = "radar")]
    Radar {
        color: String,
        background: String,
        grid: bool,
        spacing: usize,
        speed: f64,
    },
    #[serde(rename = "logo-bounce")]
    LogoBounce { speed: f64 },
    #[serde(rename = "text")]
    Text { text: String, font: String },
    #[serde(rename = "audio-video-sync")]
    AudioVideoSync {},
}

/// Options ProPresenter applies on top of every pattern.
struct Overlay {
    identify: bool,
    logo: Option<String>,
}

fn enabled() -> bool {
    true
}

fn default_spacing() -> usize {
    40
}

fn parse(value: &Value) -> Result<(Pattern, Overlay), String> {
    let mut pattern = value
        .as_object()
        .cloned()
        .ok_or("Invalid test pattern: expected an object.")?;
    let identify = match pattern.remove("identify") {
        None => false,
        Some(Value::Bool(value)) => value,
        Some(_) => return Err("Test pattern identify must be a boolean.".into()),
    };
    let logo = match pattern.remove("logo") {
        None => None,
        Some(Value::String(value)) if value == "orbit" => Some(value),
        Some(Value::String(value)) if Path::new(&value).is_absolute() => Some(value),
        Some(_) => {
            return Err("Test pattern logo must be \"orbit\" or an absolute image path.".into())
        }
    };
    let pattern: Pattern = serde_json::from_value(Value::Object(pattern))
        .map_err(|error| format!("Invalid test pattern: {error}"))?;
    let speed = |speed: &f64| speed.is_finite() && (0.0..=100.0).contains(speed);
    let spacing = |spacing: &usize| (2..=4096).contains(spacing);
    match &pattern {
        Pattern::Grid { spacing: value, .. }
        | Pattern::Focus { spacing: value, .. }
        | Pattern::Radar { spacing: value, .. }
            if !spacing(value) =>
        {
            return Err("Test pattern grid spacing must be between 2 and 4096 pixels.".into())
        }
        Pattern::Black { level } if !level.is_finite() || !(0.0..=50.0).contains(level) => {
            return Err("Black test pattern level must be between 0 and 50.".into())
        }
        Pattern::White { level } if !level.is_finite() || !(50.0..=100.0).contains(level) => {
            return Err("White test pattern level must be between 50 and 100.".into())
        }
        Pattern::Custom { color } if rgb(color).is_none() => {
            return Err("Custom test pattern requires a six-digit hex color.".into())
        }
        Pattern::Lines {
            color, background, ..
        }
        | Pattern::Radar {
            color, background, ..
        } if rgb(color).is_none() || rgb(background).is_none() => {
            return Err("Test pattern colors must be six-digit hex colors.".into())
        }
        Pattern::Lines { speed: value, .. }
        | Pattern::Radar { speed: value, .. }
        | Pattern::LogoBounce { speed: value }
            if !speed(value) =>
        {
            return Err("Test pattern speed must be between 0 and 100.".into())
        }
        Pattern::Text { text, font } if text.trim().is_empty() || font.trim().is_empty() => {
            return Err("Text test pattern requires display text and a font.".into())
        }
        _ => {}
    }
    Ok((pattern, Overlay { identify, logo }))
}

pub fn validate(value: &Value) -> Result<(), String> {
    parse(value).map(|_| ())
}

/// Animated patterns must be redrawn every output frame.
pub fn is_animated(value: &Value) -> bool {
    matches!(
        value.get("kind").and_then(Value::as_str),
        Some("lines" | "radar" | "logo-bounce" | "audio-video-sync")
    )
}

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Seconds on the shared clock that all outputs and the sync beep follow.
pub fn clock_seconds() -> f64 {
    epoch().elapsed().as_secs_f64()
}

/// The audio/video sync pattern flashes for this part of every second.
pub const SYNC_FLASH_SECONDS: f64 = 0.1;

/// Output content whose screen shows the audio/video sync pattern plays its beep.
pub fn plays_sync_beep(content: &Value) -> bool {
    content
        .get("testPattern")
        .and_then(|pattern| pattern.get("kind"))
        .and_then(Value::as_str)
        == Some("audio-video-sync")
}

fn rgb(color: &str) -> Option<[u8; 3]> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[4..6], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[0..2], 16).ok()?,
    ])
}

fn orbit_logo() -> &'static DynamicImage {
    static LOGO: OnceLock<DynamicImage> = OnceLock::new();
    LOGO.get_or_init(|| {
        image::load_from_memory(include_bytes!("../assets/logo.png"))
            .expect("bundled application icon decodes")
    })
}

fn logo_image(logo: &str, renderer: &mut NativeRenderRuntime) -> Result<DynamicImage, String> {
    if logo == "orbit" {
        return Ok(orbit_logo().clone());
    }
    Ok((*renderer.image_frame(Path::new(logo))?.image).clone())
}

/// Copy `image` scaled to `size` pixels high with its top-left corner at `left`, `top`.
fn blit(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    image: &DynamicImage,
    left: f64,
    top: f64,
    size: u32,
) {
    let scale = size as f64 / image.height().max(1) as f64;
    let scaled = image.resize_exact(
        ((image.width() as f64 * scale).round() as u32).max(1),
        size.max(1),
        FilterType::Triangle,
    );
    for (x, y, pixel) in scaled.pixels() {
        let (px, py) = (left as isize + x as isize, top as isize + y as isize);
        if px < 0 || py < 0 || px as usize >= width || py as usize >= height {
            continue;
        }
        let index = (py as usize * width + px as usize) * 4;
        let alpha = pixel[3] as u32;
        for (channel, source) in [(0, 2), (1, 1), (2, 0)] {
            let current = pixels[index + channel] as u32;
            pixels[index + channel] =
                ((pixel[source] as u32 * alpha + current * (255 - alpha)) / 255) as u8;
        }
    }
}

fn logo_size(width: usize, height: usize) -> u32 {
    (width.min(height) / 5).max(1) as u32
}

/// Triangle wave in `0..=1` used for bouncing positions.
fn bounce(distance: f64) -> f64 {
    let phase = distance.rem_euclid(2.0);
    if phase > 1.0 {
        2.0 - phase
    } else {
        phase
    }
}

fn speed_factor(speed: f64) -> f64 {
    speed / 50.0
}

pub fn render(
    value: &Value,
    width: usize,
    height: usize,
    seconds: f64,
    label: &str,
    renderer: &mut NativeRenderRuntime,
) -> Result<Vec<u8>, String> {
    let (pattern, overlay) = parse(value)?;
    let size = width
        .checked_mul(height)
        .and_then(|size| size.checked_mul(4))
        .filter(|size| *size <= 256 * 1024 * 1024)
        .ok_or("Test pattern dimensions exceed the native frame limit.")?;
    if width == 0 || height == 0 {
        return Err("Test pattern dimensions must be positive.".into());
    }
    let mut pixels = vec![0; size];
    let bars = [
        [255, 255, 255],
        [0, 255, 255],
        [255, 255, 0],
        [0, 255, 0],
        [255, 0, 255],
        [0, 0, 255],
        [255, 0, 0],
        [0, 0, 0],
    ];
    let cx = width as f64 / 2.0;
    let cy = height as f64 / 2.0;
    let radar_angle = match &pattern {
        Pattern::Radar { speed, .. } => (seconds * speed_factor(*speed) * std::f64::consts::TAU
            / 4.0)
            .rem_euclid(std::f64::consts::TAU),
        _ => 0.0,
    };
    let flash =
        matches!(pattern, Pattern::AudioVideoSync {}) && seconds.fract() < SYNC_FLASH_SECONDS;
    for y in 0..height {
        for x in 0..width {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            let on_grid = |spacing: usize| x % spacing == 0 || y % spacing == 0;
            let color = match &pattern {
                Pattern::Bars {} => bars[(x * 8 / width).min(7)],
                Pattern::Grayscale {} => {
                    let level = ((x * 16 / width).min(15) * 255 / 15) as u8;
                    [level; 3]
                }
                Pattern::Black { level } | Pattern::White { level } => {
                    [(*level * 255.0 / 100.0).round() as u8; 3]
                }
                Pattern::Custom { color } => rgb(color).expect("validated color"),
                Pattern::Grid {
                    spacing,
                    center_circle,
                    center_lines,
                    inverted,
                    grid,
                } => {
                    let radius = width.min(height) as f64 / 3.0;
                    let circle = *center_circle && (dx.hypot(dy) - radius).abs() < 1.0;
                    let line = (*grid && on_grid(*spacing))
                        || circle
                        || (*center_lines && (x == width / 2 || y == height / 2));
                    [if line != *inverted { 255 } else { 0 }; 3]
                }
                Pattern::Focus { grid, spacing } => {
                    let ring = (dx.hypot(dy) / 2.0).floor() as usize;
                    [if ring % 2 == 0 || (*grid && on_grid(*spacing)) {
                        255
                    } else {
                        0
                    }; 3]
                }
                Pattern::Lines {
                    color,
                    background,
                    speed,
                } => {
                    let travel = seconds * speed_factor(*speed) / 4.0;
                    let column = (bounce(travel) * (width - 1) as f64).round() as usize;
                    let row = (bounce(travel * 0.75) * (height - 1) as f64).round() as usize;
                    let thickness = (width.min(height) / 200).max(1);
                    if x.abs_diff(column) < thickness || y.abs_diff(row) < thickness {
                        rgb(color).expect("validated color")
                    } else {
                        rgb(background).expect("validated color")
                    }
                }
                Pattern::Radar {
                    color,
                    background,
                    grid,
                    spacing,
                    ..
                } => {
                    let foreground = rgb(color).expect("validated color");
                    let back = rgb(background).expect("validated color");
                    if *grid && on_grid(*spacing) {
                        foreground
                    } else {
                        // The sweep leaves a fading trail behind its leading edge.
                        let angle = dy.atan2(dx).rem_euclid(std::f64::consts::TAU);
                        let behind = (radar_angle - angle).rem_euclid(std::f64::consts::TAU);
                        let strength = (1.0 - behind / (std::f64::consts::PI / 2.0)).max(0.0);
                        std::array::from_fn(|channel| {
                            (back[channel] as f64
                                + (foreground[channel] as f64 - back[channel] as f64) * strength)
                                .round() as u8
                        })
                    }
                }
                Pattern::LogoBounce { .. } | Pattern::Text { .. } => [0, 0, 0],
                Pattern::AudioVideoSync {} => {
                    if flash {
                        [255; 3]
                    } else {
                        // A sweep hand completes one revolution per second between flashes.
                        let angle = (dx.atan2(-dy)).rem_euclid(std::f64::consts::TAU);
                        let hand = (angle - seconds.fract() * std::f64::consts::TAU).abs() < 0.03;
                        let ring = (dx.hypot(dy) - width.min(height) as f64 / 3.0).abs() < 2.0;
                        if hand || ring {
                            [255; 3]
                        } else {
                            [0; 3]
                        }
                    }
                }
            };
            pixels[(y * width + x) * 4..(y * width + x) * 4 + 4]
                .copy_from_slice(&[color[0], color[1], color[2], 255]);
        }
    }
    let size = logo_size(width, height);
    match &pattern {
        Pattern::LogoBounce { speed } => {
            let image = logo_image(overlay.logo.as_deref().unwrap_or("orbit"), renderer)?;
            let logo_width = image.width() as f64 * size as f64 / image.height().max(1) as f64;
            let travel = seconds * speed_factor(*speed) / 3.0;
            let left = bounce(travel) * (width as f64 - logo_width).max(0.0);
            let top = bounce(travel * 0.7) * (height as f64 - size as f64).max(0.0);
            blit(&mut pixels, width, height, &image, left, top, size);
        }
        Pattern::Text { text, font } => {
            renderer.draw_text_box(
                &mut pixels,
                width,
                height,
                text,
                0,
                0,
                width,
                height,
                (height as f32 / 8.0).max(1.0),
                [255, 255, 255, 255],
                json!({"fontFamily": font, "textHorizontalAlign": "center", "textVerticalAlign": "middle"})
                    .as_object()
                    .expect("text style object"),
            );
        }
        _ => {
            if let Some(logo) = &overlay.logo {
                let image = logo_image(logo, renderer)?;
                let logo_width = image.width() as f64 * size as f64 / image.height().max(1) as f64;
                blit(
                    &mut pixels,
                    width,
                    height,
                    &image,
                    cx - logo_width / 2.0,
                    cy - size as f64 / 2.0,
                    size,
                );
            }
        }
    }
    if overlay.identify && !label.trim().is_empty() {
        let band = (height / 6).max(1);
        renderer.draw_text_box(
            &mut pixels,
            width,
            height,
            label,
            0,
            (height - band) as isize,
            width,
            band,
            (band as f32 / 2.0).max(1.0),
            [0, 255, 255, 255],
            json!({"fontFamily": "Arial", "textHorizontalAlign": "center", "textVerticalAlign": "middle"})
                .as_object()
                .expect("identify style object"),
        );
    }
    Ok(pixels)
}
