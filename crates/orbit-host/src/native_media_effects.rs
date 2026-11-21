//! ProPresenter's media effect library (`GLSL/*.rvfx`) for native output.
//!
//! Effects are identified by their ProPresenter UUID and keep the variables of
//! the library descriptors. Each effect is an Orbit reimplementation on the CPU;
//! ProPresenter's shaders are not used. A media asset carries an ordered chain
//! of effects (`mediaEffects`) that is applied after its color adjustments.

use image::RgbaImage;

use serde_json::{json, Value};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    AlphaAdjust,
    AdjustColor,
    RgbAdjust,
    Blur,
    CrtMonitor,
    ColorBlobs,
    ColorFilter,
    ColorInvert,
    ColorKey,
    ColorPosterize,
    ColorSwap,
    Dots,
    EdgeBlur,
    EdgeOutline,
    Emboss,
    GrayInvert,
    Halftone,
    HeatSignature,
    Hexagon,
    ImageDistortion,
    Kaleidoscope1,
    Kaleidoscope2,
    LensDistortion,
    OldFilm,
    Picasso,
    Pixelize,
    Plasma,
    RadialBlur,
    RippleCenter,
    RippleSide,
    Sepia,
    Shake,
    Stars,
    Strobe,
    Tile,
    Toon,
    TwoColor,
    Vignette,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VariableKind {
    Number {
        min: f64,
        max: f64,
    },
    Integer {
        min: f64,
        max: f64,
    },
    /// An opaque RGB color, stored in Orbit as `#rrggbb`.
    Color,
}

/// One adjustable effect setting; `value` is the library value (RGB for colors).
pub struct Variable {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: VariableKind,
    pub value: [f64; 3],
}

const fn number(
    name: &'static str,
    label: &'static str,
    min: f64,
    max: f64,
    value: f64,
) -> Variable {
    Variable {
        name,
        label,
        kind: VariableKind::Number { min, max },
        value: [value, 0.0, 0.0],
    }
}

const fn integer(
    name: &'static str,
    label: &'static str,
    min: f64,
    max: f64,
    value: f64,
) -> Variable {
    Variable {
        name,
        label,
        kind: VariableKind::Integer { min, max },
        value: [value, 0.0, 0.0],
    }
}

const fn color(name: &'static str, label: &'static str, value: [f64; 3]) -> Variable {
    Variable {
        name,
        label,
        kind: VariableKind::Color,
        value,
    }
}

pub struct Entry {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: EffectKind,
    pub variables: &'static [Variable],
}

const WHITE: [f64; 3] = [1.0, 1.0, 1.0];

/// ProPresenter's media effect library, keyed by its render UUIDs. Background
/// Blur and Background Invert are object fill effects (EDT-17), not media effects.
#[rustfmt::skip]
pub const CATALOG: &[Entry] = &[
    Entry { id: "56C8A640-8BFF-4AFF-BBA5-23134683DB2F", name: "Alpha Adjust", kind: EffectKind::AlphaAdjust, variables: &[number("amount", "Amount", 0.0, 1.0, 1.0)] },
    Entry { id: "D4C199A9-64D7-4CCB-8C26-443E7A9C5C03", name: "Adjust Color", kind: EffectKind::AdjustColor, variables: &[
        number("hue", "Hue", -std::f64::consts::PI, std::f64::consts::PI, 0.0),
        number("saturation", "Saturation", 0.0, 2.0, 1.0),
        number("brightness", "Brightness", -1.0, 1.0, 0.0),
        number("contrast", "Contrast", 0.0, 2.0, 1.0),
    ] },
    Entry { id: "453B8725-AAB6-427A-8FCE-570B8AB52790", name: "RGB Adjust", kind: EffectKind::RgbAdjust, variables: &[
        number("red", "Red", 0.0, 1.0, 1.0), number("green", "Green", 0.0, 1.0, 1.0), number("blue", "Blue", 0.0, 1.0, 1.0),
    ] },
    Entry { id: "D84CE4A3-D1C3-4F7C-AA75-4C645A3621F2", name: "Blur", kind: EffectKind::Blur, variables: &[number("blurAmount", "Intensity", 0.0, 1.0, 0.5)] },
    Entry { id: "0AD74569-93D8-4849-8E07-A05CCFB15620", name: "CRT Monitor", kind: EffectKind::CrtMonitor, variables: &[
        number("strength", "Strength", 0.5, 1.7, 1.2), number("uPixelSize", "Size", 1.5, 5.0, 3.0),
    ] },
    Entry { id: "0630A39C-69E9-4403-B428-3138939673A3", name: "Color Blobs", kind: EffectKind::ColorBlobs, variables: &[
        number("blendValue", "Blend Amount", 0.01, 1.0, 0.5), number("speed", "Speed", 0.01, 1.0, 0.5),
    ] },
    Entry { id: "85A535EF-4EFC-4105-B8C7-C65C53376B87", name: "Color Filter", kind: EffectKind::ColorFilter, variables: &[color("pickedColor", "Blend Color", [1.0, 0.0, 0.0])] },
    Entry { id: "EDF55F11-1A1E-4BF3-9718-FA2D3731626D", name: "Color Invert", kind: EffectKind::ColorInvert, variables: &[] },
    Entry { id: "469387A2-681A-458E-9414-DAB953C1564F", name: "Color Key", kind: EffectKind::ColorKey, variables: &[
        color("srcColor", "Color", WHITE), number("tolerance", "Tolerance", 0.01, 1.0, 0.5),
    ] },
    Entry { id: "F888F8FF-605A-4B2E-A20D-6FFA7C3E6E00", name: "Color Posterize", kind: EffectKind::ColorPosterize, variables: &[
        number("numOfColors", "Color Count", 1.0, 14.0, 7.0), number("gammaValue", "Gamma", 0.1, 1.1, 0.5),
    ] },
    Entry { id: "7511AE15-3C2D-4840-BD66-DDC0C4A3DABE", name: "Color Swap", kind: EffectKind::ColorSwap, variables: &[
        number("tolerance", "Tolerance", 0.01, 1.0, 0.5), color("srcColor", "Old Color", WHITE), color("dstColor", "New Color", WHITE),
    ] },
    Entry { id: "A6FD4AC9-6C4C-47C5-9FC0-AAD551C4642E", name: "Dots", kind: EffectKind::Dots, variables: &[
        number("outlineAmount", "Intensity", 0.0, 1.0, 0.5), number("colorLevel", "Color", 0.0, 1.0, 0.5),
    ] },
    Entry { id: "AF83B133-D6A1-487C-BD9E-8579FA49B4E0", name: "Edge Blur", kind: EffectKind::EdgeBlur, variables: &[
        number("blurAmount", "Amount", 0.0, 1.0, 0.5), number("blurRadius", "Radius", 0.0, 1.0, 0.25),
    ] },
    Entry { id: "86BA6E3E-358F-4DA2-93C4-6A2C0EC831DA", name: "Edge Outline", kind: EffectKind::EdgeOutline, variables: &[
        number("outlineAmount", "Intensity", 0.0, 1.0, 0.5), number("colorLevel", "Color", 0.0, 1.0, 0.5),
    ] },
    Entry { id: "D2C1718A-571B-4844-BD72-7C13193E8F26", name: "Emboss", kind: EffectKind::Emboss, variables: &[
        number("lightX", "Light X", 0.0, 1.0, 0.0), number("lightY", "Light Y", 0.0, 1.0, 1.0),
    ] },
    Entry { id: "4FDA3849-C39D-4269-AADA-B963944900B7", name: "Gray Invert", kind: EffectKind::GrayInvert, variables: &[] },
    Entry { id: "ADF80D5E-FD27-47BD-AFA6-8D4167364567", name: "Halftone", kind: EffectKind::Halftone, variables: &[
        number("sharpnessValue", "Sharpness", 0.0, 1.0, 0.5), number("gcrValue", "Gray Components", 0.0, 2.0, 1.0), number("ucrValue", "Color Removal", 0.0, 1.0, 0.5),
    ] },
    Entry { id: "F3F254AF-BF83-412D-B802-51E42A25FDF9", name: "Heat Signature", kind: EffectKind::HeatSignature, variables: &[] },
    Entry { id: "690831F6-A8A4-4B70-9AEB-E9A5E7FBDF63", name: "Hexagon", kind: EffectKind::Hexagon, variables: &[number("hexSize", "Size", 10.0, 100.0, 50.0)] },
    Entry { id: "8A0F0EB7-1F75-484B-8367-C390D94EA6C0", name: "Image Distortion", kind: EffectKind::ImageDistortion, variables: &[number("scale", "Scale", 1.0, 2.0, 1.5)] },
    Entry { id: "17A42D1F-BC36-48D6-B9F6-110CB3AA93F9", name: "Kaleidoscope 1", kind: EffectKind::Kaleidoscope1, variables: &[
        number("speed", "Speed", 0.0, 1.0, 0.5), integer("numSides", "Sides", 2.0, 20.0, 7.0),
        number("centerX", "Center X", -1.0, 1.0, 0.0), number("centerY", "Center Y", -1.0, 1.0, 0.0),
    ] },
    Entry { id: "47BAC53A-761B-478D-AC74-2CA7A8CFB7D5", name: "Kaleidoscope 2", kind: EffectKind::Kaleidoscope2, variables: &[
        number("speed", "Speed", 0.0, 1.0, 0.5), integer("numSides", "Sides", 4.0, 20.0, 7.0),
        number("centerX", "Center X", -1.0, 1.0, 0.0), number("centerY", "Center Y", -1.0, 1.0, 0.0),
    ] },
    Entry { id: "291C7C20-07C5-42D2-92C9-45C173D69898", name: "Lens Distortion", kind: EffectKind::LensDistortion, variables: &[
        number("lensSize", "Lens Size", 0.0, 2.0, 1.0), number("colorDistort", "Color Distortion", 0.0, 2.0, 1.0),
    ] },
    Entry { id: "18CB8C8C-F23D-44B0-98BE-ECEB22B29D35", name: "Old Film", kind: EffectKind::OldFilm, variables: &[
        number("sepiaValue", "Sepia Effect", 0.0, 1.0, 0.5), number("noiseValue", "Noise Level", 0.0, 1.0, 0.5),
        number("scratchValue", "Scratches", 0.0, 1.0, 0.5), number("radius", "Vignette", 0.0, 1.0, 0.5),
        number("frequency", "Frequency", 0.0, 1.0, 0.5), number("intensity", "Intensity", 0.0, 0.2, 0.02),
    ] },
    Entry { id: "9A5ED7A4-8339-43C2-BABB-180AC20D4C07", name: "Picasso", kind: EffectKind::Picasso, variables: &[number("speed", "Animate", 0.0, 1.0, 0.0)] },
    Entry { id: "54E8E7F0-2302-4CBE-A63D-02D56915A79A", name: "Pixelize", kind: EffectKind::Pixelize, variables: &[number("pixelSize", "Size", 0.01, 1.0, 0.5)] },
    Entry { id: "E19D17CE-03B7-4C58-BD56-FF79C6875651", name: "Plasma", kind: EffectKind::Plasma, variables: &[
        number("blendValue", "Blend", 0.01, 1.0, 0.5), number("speed", "Speed", 0.01, 1.0, 0.5),
    ] },
    Entry { id: "6BC757C8-3F76-4E7E-885A-7A7C944465A5", name: "Radial Blur", kind: EffectKind::RadialBlur, variables: &[
        number("blendValue", "Blend", 0.01, 1.0, 0.5), number("speed", "Speed", 0.01, 1.0, 0.5),
        number("iterations", "Iterations", 4.0, 24.0, 10.0), number("decay", "Decay", 0.8, 1.05, 0.97),
    ] },
    Entry { id: "D0C7FA30-C4D6-4F6D-86D9-CA27099D56EC", name: "Ripple Center", kind: EffectKind::RippleCenter, variables: &[number("speed", "Speed", 0.0, 2.0, 1.0)] },
    Entry { id: "2A34B3FB-0AC5-4C7D-B07E-05FE83D796B4", name: "Ripple Side", kind: EffectKind::RippleSide, variables: &[number("speed", "Speed", 0.0, 2.0, 1.0)] },
    Entry { id: "06157C8E-D971-4F06-9ECA-2663664E22A6", name: "Sepia", kind: EffectKind::Sepia, variables: &[] },
    Entry { id: "5472C6A3-59A0-4ED5-AE13-0059E2D185C9", name: "Shake", kind: EffectKind::Shake, variables: &[
        number("frequency", "Frequency", 0.0, 1.0, 0.5), number("intensity", "Intensity", 0.0, 1.0, 0.5),
    ] },
    Entry { id: "55BB6AA1-9133-4CE4-991B-7BF5149E6834", name: "Stars", kind: EffectKind::Stars, variables: &[
        number("speed", "Speed", 0.0, 20.0, 10.0), number("blendValue", "Blend", 0.0, 1.0, 0.5),
    ] },
    Entry { id: "DA373B28-6AD6-499D-8AD4-11E2ADC73475", name: "Strobe", kind: EffectKind::Strobe, variables: &[
        number("frequency", "Frequency", 0.0, 1.0, 0.5), number("strobeLength", "Length", 0.0, 1.0, 0.05),
        number("strobeBlend", "Blend", 0.0, 1.0, 1.0), number("randomness", "Randomness", 0.0, 1.0, 0.5),
        color("strobeColor", "Color", WHITE),
    ] },
    Entry { id: "4FF40112-9D47-43BA-A381-B685D13F5A89", name: "Tile", kind: EffectKind::Tile, variables: &[
        number("cells", "Cells", 1.0, 32.0, 1.0), integer("gridSize", "Size", 0.0, 20.0, 1.0),
    ] },
    Entry { id: "3A7A9FFF-F027-4EFD-94CC-429A491485C7", name: "Toon", kind: EffectKind::Toon, variables: &[
        integer("colorLevels", "Number of colors", 1.0, 16.0, 8.0), number("saturation", "Saturation", 0.0, 1.0, 0.5),
    ] },
    Entry { id: "258D3830-8496-4712-AAAE-A68BCA23D805", name: "Two Color", kind: EffectKind::TwoColor, variables: &[
        color("darkColor", "Dark Color", WHITE), color("brightColor", "Bright Color", WHITE),
    ] },
    Entry { id: "42E83016-41DC-43AB-8061-9A93256DDA46", name: "Vignette", kind: EffectKind::Vignette, variables: &[number("radius", "Radius", 0.0, 100.0, 50.0)] },
];

/// The longest chain a media asset may carry.
pub const MAX_CHAIN: usize = 16;

pub fn catalog_entry(id: &str) -> Option<&'static Entry> {
    CATALOG
        .iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(id))
}

fn color_hex(rgb: [f64; 3]) -> String {
    crate::native_transitions::color_hex(rgb)
}

/// Finds an effect variable by name or label. ProPresenter files name variables
/// by their label ("Hue"), while descriptors use camel-case names ("hue").
pub fn find_variable(entry: &Entry, name: &str) -> Option<&'static Variable> {
    let entry = catalog_entry(entry.id)?;
    entry.variables.iter().find(|variable| {
        variable.name.eq_ignore_ascii_case(name) || variable.label.eq_ignore_ascii_case(name)
    })
}

/// Reads a variable value from its Orbit form, if valid.
pub fn variable_value(variable: &Variable, value: &Value) -> Option<[f64; 3]> {
    match variable.kind {
        VariableKind::Number { min, max } => value
            .as_f64()
            .filter(|number| number.is_finite() && (min..=max).contains(number))
            .map(|number| [number, 0.0, 0.0]),
        VariableKind::Integer { min, max } => value
            .as_f64()
            .filter(|number| number.fract() == 0.0 && (min..=max).contains(number))
            .map(|number| [number, 0.0, 0.0]),
        VariableKind::Color => value
            .as_str()
            .and_then(crate::native_transitions::parse_color_hex),
    }
}

/// The Orbit form of a variable value.
pub fn variable_json(variable: &Variable, value: [f64; 3]) -> Value {
    match variable.kind {
        VariableKind::Color => json!(color_hex(value)),
        _ => json!(value[0]),
    }
}

/// The catalogue as the renderer lists it.
pub fn catalog_value() -> Value {
    Value::Array(
        CATALOG
            .iter()
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "name": entry.name,
                    "variables": entry.variables.iter().map(|variable| {
                        let mut value = json!({
                            "name": variable.name,
                            "label": variable.label,
                            "value": variable_json(variable, variable.value),
                        });
                        match variable.kind {
                            VariableKind::Number { min, max } | VariableKind::Integer { min, max } => {
                                value["type"] = json!(if matches!(variable.kind, VariableKind::Integer { .. }) { "integer" } else { "number" });
                                value["min"] = json!(min);
                                value["max"] = json!(max);
                            }
                            VariableKind::Color => value["type"] = json!("color"),
                        }
                        value
                    }).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

/// Validates a media effect chain: `[{effectId, enabled, variables?}]`.
pub fn validate_chain(value: &Value) -> Result<(), String> {
    let chain = value.as_array().ok_or("mediaEffects must be an array.")?;
    if chain.len() > MAX_CHAIN {
        return Err(format!(
            "A media effect chain holds at most {MAX_CHAIN} effects."
        ));
    }
    for effect in chain {
        let object = effect
            .as_object()
            .ok_or("A media effect must be an object.")?;
        if let Some(key) = object
            .keys()
            .find(|key| !matches!(key.as_str(), "effectId" | "enabled" | "variables"))
        {
            return Err(format!("Unsupported media effect field: {key}."));
        }
        let id = object
            .get("effectId")
            .and_then(Value::as_str)
            .ok_or("A media effect requires an effectId.")?;
        let entry = catalog_entry(id).ok_or_else(|| format!("Unknown media effect: {id}."))?;
        if !object.get("enabled").is_some_and(Value::is_boolean) {
            return Err(format!("{} requires a boolean enabled flag.", entry.name));
        }
        if let Some(variables) = object.get("variables") {
            let variables = variables
                .as_object()
                .ok_or("Media effect variables must be an object.")?;
            for (name, value) in variables {
                let variable = entry
                    .variables
                    .iter()
                    .find(|variable| variable.name == name)
                    .ok_or_else(|| format!("{} has no {name} variable.", entry.name))?;
                if variable_value(variable, value).is_none() {
                    return Err(match variable.kind {
                        VariableKind::Number { min, max } => {
                            format!(
                                "{} {} must be between {min} and {max}.",
                                entry.name, variable.label
                            )
                        }
                        VariableKind::Integer { min, max } => format!(
                            "{} {} must be a whole number between {min} and {max}.",
                            entry.name, variable.label
                        ),
                        VariableKind::Color => {
                            format!("{} {} must be a #rrggbb color.", entry.name, variable.label)
                        }
                    });
                }
            }
        }
    }
    Ok(())
}

/// An enabled effect with every variable resolved, in catalogue order.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedEffect {
    pub kind: EffectKind,
    pub values: Vec<[f64; 3]>,
}

impl ResolvedEffect {
    fn number(&self, index: usize) -> f32 {
        self.values[index][0] as f32
    }

    fn color(&self, index: usize) -> [f32; 3] {
        self.values[index].map(|value| value as f32)
    }

    /// Whether the effect changes over time and so needs a new frame each tick.
    pub fn animates(&self) -> bool {
        match self.kind {
            EffectKind::ColorBlobs
            | EffectKind::OldFilm
            | EffectKind::Plasma
            | EffectKind::RadialBlur
            | EffectKind::Shake
            | EffectKind::Stars
            | EffectKind::Strobe => true,
            EffectKind::RippleCenter
            | EffectKind::RippleSide
            | EffectKind::Picasso
            | EffectKind::Kaleidoscope1
            | EffectKind::Kaleidoscope2 => self.number(0) > 0.0,
            _ => false,
        }
    }
}

/// Resolves a validated chain, dropping disabled effects.
pub fn resolve_chain(value: &Value) -> Result<Vec<ResolvedEffect>, String> {
    validate_chain(value)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|effect| effect["enabled"] == true)
        .filter_map(|effect| {
            let entry = catalog_entry(effect["effectId"].as_str()?)?;
            let values = entry
                .variables
                .iter()
                .map(|variable| {
                    effect
                        .get("variables")
                        .and_then(|variables| variables.get(variable.name))
                        .and_then(|value| variable_value(variable, value))
                        .unwrap_or(variable.value)
                })
                .collect();
            Some(ResolvedEffect {
                kind: entry.kind,
                values,
            })
        })
        .collect())
}

/// A straight-alpha RGBA image in `0..=1`.
#[derive(Clone)]
struct Buffer {
    width: usize,
    height: usize,
    pixels: Vec<[f32; 4]>,
}

impl Buffer {
    fn from_image(image: &RgbaImage) -> Self {
        let (width, height) = (image.width() as usize, image.height() as usize);
        let bytes = image.as_raw();
        map(width, height, |x, y| {
            let offset = (y * width + x) * 4;
            [0, 1, 2, 3].map(|channel| bytes[offset + channel] as f32 * (1.0 / 255.0))
        })
    }

    fn into_image(self) -> RgbaImage {
        let mut bytes = vec![0u8; self.pixels.len() * 4];
        let rows = rows_per_thread(self.height);
        std::thread::scope(|scope| {
            for (chunk, pixels) in bytes
                .chunks_mut(rows * self.width * 4)
                .zip(self.pixels.chunks(rows * self.width))
            {
                scope.spawn(move || {
                    for (output, pixel) in chunk.chunks_mut(4).zip(pixels) {
                        for channel in 0..4 {
                            output[channel] = (pixel[channel].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                        }
                    }
                });
            }
        });
        RgbaImage::from_raw(self.width as u32, self.height as u32, bytes)
            .expect("buffer dimensions match its pixels")
    }

    fn at(&self, x: isize, y: isize) -> [f32; 4] {
        let x = x.clamp(0, self.width as isize - 1) as usize;
        let y = y.clamp(0, self.height as isize - 1) as usize;
        self.pixels[y * self.width + x]
    }

    /// Bilinear sample at pixel-centre coordinates, clamped to the edges.
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        let (xi, yi) = (x0 as isize, y0 as isize);
        let mix = |a: [f32; 4], b: [f32; 4], t: f32| {
            [0, 1, 2, 3].map(|channel| a[channel] + (b[channel] - a[channel]) * t)
        };
        mix(
            mix(self.at(xi, yi), self.at(xi + 1, yi), tx),
            mix(self.at(xi, yi + 1), self.at(xi + 1, yi + 1), tx),
            ty,
        )
    }

    /// Sample that is transparent outside the image.
    fn sample_inside(&self, x: f32, y: f32) -> [f32; 4] {
        if x < 0.0 || y < 0.0 || x > self.width as f32 || y > self.height as f32 {
            return [0.0; 4];
        }
        self.sample(x, y)
    }

    fn largest_side(&self) -> f32 {
        self.width.max(self.height) as f32
    }
}

fn rows_per_thread(height: usize) -> usize {
    let threads = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(height.max(1));
    height.div_ceil(threads).max(1)
}

/// Computes every output pixel from its coordinates, across all CPU threads.
fn map(width: usize, height: usize, pixel: impl Fn(usize, usize) -> [f32; 4] + Sync) -> Buffer {
    let mut pixels = vec![[0.0; 4]; width * height];
    if width > 0 && height > 0 {
        let rows = rows_per_thread(height);
        std::thread::scope(|scope| {
            for (chunk_index, chunk) in pixels.chunks_mut(rows * width).enumerate() {
                let pixel = &pixel;
                scope.spawn(move || {
                    for (offset, value) in chunk.iter_mut().enumerate() {
                        let index = chunk_index * rows * width + offset;
                        *value = pixel(index % width, index / width);
                    }
                });
            }
        });
    }
    Buffer {
        width,
        height,
        pixels,
    }
}

fn luminance(color: [f32; 4]) -> f32 {
    0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2]
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn with_rgb(color: [f32; 4], rgb: [f32; 3]) -> [f32; 4] {
    [rgb[0], rgb[1], rgb[2], color[3]]
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|channel| a[channel] + (b[channel] - a[channel]) * t)
}

fn rgb(color: [f32; 4]) -> [f32; 3] {
    [color[0], color[1], color[2]]
}

/// Screen blend, used by the generated overlays.
fn screen(base: [f32; 3], light: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|channel| 1.0 - (1.0 - base[channel]) * (1.0 - light[channel]))
}

fn hash(x: i64, y: i64, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h as f32 / 4_294_967_296.0
}

fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (smoothstep(0.0, 1.0, x - x0), smoothstep(0.0, 1.0, y - y0));
    let (xi, yi) = (x0 as i64, y0 as i64);
    let top = hash(xi, yi, seed) + (hash(xi + 1, yi, seed) - hash(xi, yi, seed)) * tx;
    let bottom =
        hash(xi, yi + 1, seed) + (hash(xi + 1, yi + 1, seed) - hash(xi, yi + 1, seed)) * tx;
    top + (bottom - top) * ty
}

/// Blur with radius in pixels. Large radii blur a box-averaged reduced copy and
/// sample it back bilinearly, which is indistinguishable for a soft blur.
fn blur(source: &Buffer, radius: f32) -> Buffer {
    let factor = (radius / 6.0).floor() as usize;
    if factor < 2 {
        return blur_full(source, radius);
    }
    let (width, height) = (
        source.width.div_ceil(factor),
        source.height.div_ceil(factor),
    );
    let reduced = map(width, height, |x, y| {
        let mut sum = [0.0; 4];
        let mut count = 0.0;
        for sy in y * factor..((y + 1) * factor).min(source.height) {
            for sx in x * factor..((x + 1) * factor).min(source.width) {
                let pixel = source.pixels[sy * source.width + sx];
                for channel in 0..3 {
                    sum[channel] += pixel[channel] * pixel[3];
                }
                sum[3] += pixel[3];
                count += 1.0;
            }
        }
        let alpha = sum[3] / count;
        if sum[3] > 0.0 {
            [sum[0] / sum[3], sum[1] / sum[3], sum[2] / sum[3], alpha]
        } else {
            [0.0; 4]
        }
    });
    let blurred = blur_full(&reduced, radius / factor as f32);
    let factor = factor as f32;
    map(source.width, source.height, |x, y| {
        blurred.sample((x as f32 + 0.5) / factor, (y as f32 + 0.5) / factor)
    })
}

/// Separable box blur on premultiplied colors; three passes approximate a Gaussian.
fn blur_full(source: &Buffer, radius: f32) -> Buffer {
    let radius = radius.round() as usize;
    if radius == 0 {
        return source.clone();
    }
    let premultiplied = Buffer {
        pixels: source
            .pixels
            .iter()
            .map(|pixel| {
                [
                    pixel[0] * pixel[3],
                    pixel[1] * pixel[3],
                    pixel[2] * pixel[3],
                    pixel[3],
                ]
            })
            .collect(),
        ..*source
    };
    let mut buffer = premultiplied;
    for _ in 0..3 {
        buffer = transpose(&box_rows(&transpose(&box_rows(&buffer, radius)), radius));
    }
    for pixel in &mut buffer.pixels {
        if pixel[3] > 0.0 {
            for channel in 0..3 {
                pixel[channel] /= pixel[3];
            }
        }
    }
    buffer
}

/// Box filter along each row with a sliding window sum.
fn box_rows(source: &Buffer, radius: usize) -> Buffer {
    let width = source.width;
    let mut pixels = vec![[0.0; 4]; source.pixels.len()];
    let rows = rows_per_thread(source.height);
    std::thread::scope(|scope| {
        for (output, input) in pixels
            .chunks_mut(rows * width)
            .zip(source.pixels.chunks(rows * width))
        {
            scope.spawn(move || {
                for (out_row, row) in output.chunks_mut(width).zip(input.chunks(width)) {
                    let mut sum = [0.0f32; 4];
                    for pixel in &row[..radius.min(width)] {
                        for channel in 0..4 {
                            sum[channel] += pixel[channel];
                        }
                    }
                    for x in 0..width {
                        if x + radius < width {
                            for channel in 0..4 {
                                sum[channel] += row[x + radius][channel];
                            }
                        }
                        if x > radius {
                            for channel in 0..4 {
                                sum[channel] -= row[x - radius - 1][channel];
                            }
                        }
                        let count = ((x + radius + 1).min(width) - x.saturating_sub(radius)) as f32;
                        out_row[x] = sum.map(|value| value / count);
                    }
                }
            });
        }
    });
    Buffer {
        width,
        height: source.height,
        pixels,
    }
}

fn transpose(source: &Buffer) -> Buffer {
    map(source.height, source.width, |x, y| {
        source.pixels[x * source.width + y]
    })
}

/// Sobel gradient of luminance.
fn gradient(source: &Buffer, x: usize, y: usize) -> (f32, f32) {
    let (x, y) = (x as isize, y as isize);
    let l = |dx: isize, dy: isize| luminance(source.at(x + dx, y + dy));
    let gx = l(1, -1) + 2.0 * l(1, 0) + l(1, 1) - l(-1, -1) - 2.0 * l(-1, 0) - l(-1, 1);
    let gy = l(-1, 1) + 2.0 * l(0, 1) + l(1, 1) - l(-1, -1) - 2.0 * l(0, -1) - l(1, -1);
    (gx, gy)
}

fn hue_rotate(color: [f32; 3], radians: f32) -> [f32; 3] {
    let (sin, cos) = radians.sin_cos();
    let m = [
        [
            0.213 + cos * 0.787 - sin * 0.213,
            0.715 - cos * 0.715 - sin * 0.715,
            0.072 - cos * 0.072 + sin * 0.928,
        ],
        [
            0.213 - cos * 0.213 + sin * 0.143,
            0.715 + cos * 0.285 + sin * 0.140,
            0.072 - cos * 0.072 - sin * 0.283,
        ],
        [
            0.213 - cos * 0.213 - sin * 0.787,
            0.715 - cos * 0.715 + sin * 0.715,
            0.072 + cos * 0.928 + sin * 0.072,
        ],
    ];
    m.map(|row| (row[0] * color[0] + row[1] * color[1] + row[2] * color[2]).clamp(0.0, 1.0))
}

fn saturate(color: [f32; 3], amount: f32) -> [f32; 3] {
    let gray = 0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2];
    color.map(|channel| (gray + (channel - gray) * amount).clamp(0.0, 1.0))
}

fn sepia(color: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = color;
    [
        (0.393 * r + 0.769 * g + 0.189 * b).min(1.0),
        (0.349 * r + 0.686 * g + 0.168 * b).min(1.0),
        (0.272 * r + 0.534 * g + 0.131 * b).min(1.0),
    ]
}

/// Applies an enabled effect chain to an image at output time `seconds`.
pub fn apply_chain(image: &RgbaImage, chain: &[ResolvedEffect], seconds: f64) -> RgbaImage {
    let mut buffer = Buffer::from_image(image);
    for effect in chain {
        buffer = apply(&buffer, effect, seconds as f32);
    }
    buffer.into_image()
}

fn apply(source: &Buffer, effect: &ResolvedEffect, t: f32) -> Buffer {
    let (width, height) = (source.width, source.height);
    let (w, h) = (width as f32, height as f32);
    let side = source.largest_side();
    let scale = side / 1920.0;
    let (cx, cy) = (w / 2.0, h / 2.0);
    let half_diagonal = (cx * cx + cy * cy).sqrt().max(1.0);
    let n = |index| effect.number(index);
    let point = |f: &(dyn Fn([f32; 4], usize, usize) -> [f32; 4] + Sync)| {
        map(width, height, |x, y| f(source.pixels[y * width + x], x, y))
    };
    let warp = |f: &(dyn Fn(f32, f32) -> (f32, f32) + Sync)| {
        map(width, height, |x, y| {
            let (sx, sy) = f(x as f32 + 0.5, y as f32 + 0.5);
            source.sample(sx, sy)
        })
    };
    match effect.kind {
        EffectKind::AlphaAdjust => point(&|c, _, _| [c[0], c[1], c[2], c[3] * n(0)]),
        EffectKind::AdjustColor => point(&|c, _, _| {
            let rotated = hue_rotate(rgb(c), n(0));
            let saturated = saturate(rotated, n(1));
            with_rgb(
                c,
                saturated.map(|channel| {
                    (((channel + n(2)).clamp(0.0, 1.0) - 0.5) * n(3) + 0.5).clamp(0.0, 1.0)
                }),
            )
        }),
        EffectKind::RgbAdjust => point(&|c, _, _| [c[0] * n(0), c[1] * n(1), c[2] * n(2), c[3]]),
        EffectKind::Blur => blur(source, n(0) * 0.03 * side / 2.0),
        EffectKind::CrtMonitor => {
            let cell = (n(1) * scale * 2.0).max(1.0);
            point(&|c, x, y| {
                let scan = 0.65 + 0.35 * (PI * (y as f32 % cell) / cell).sin();
                let channel = ((x as f32 / (cell / 3.0)) as usize) % 3;
                let mut mask = [0.75; 3];
                mask[channel] = 1.0;
                with_rgb(
                    c,
                    [0, 1, 2].map(|index| (c[index] * mask[index] * scan * n(0)).min(1.0)),
                )
            })
        }
        EffectKind::ColorBlobs => {
            let time = t * n(1) * 2.0;
            let blobs: Vec<([f32; 2], [f32; 3])> = (0..4)
                .map(|index| {
                    let phase = index as f32 * 1.7;
                    (
                        [
                            0.5 + 0.35 * (time * (0.6 + index as f32 * 0.13) + phase).sin(),
                            0.5 + 0.35 * (time * (0.5 + index as f32 * 0.17) + phase * 1.3).cos(),
                        ],
                        hue_rotate([1.0, 0.25, 0.25], index as f32 * TAU / 4.0 + time * 0.3),
                    )
                })
                .collect();
            point(&|c, x, y| {
                let (u, v) = ((x as f32 + 0.5) / w, (y as f32 + 0.5) / h);
                let mut light = [0.0; 3];
                for (center, color) in &blobs {
                    let d = ((u - center[0]).powi(2) + (v - center[1]).powi(2)) / 0.06;
                    let weight = (-d).exp();
                    light = screen(light, color.map(|channel| channel * weight));
                }
                with_rgb(c, mix3(rgb(c), screen(rgb(c), light), n(0)))
            })
        }
        EffectKind::ColorFilter => {
            let filter = effect.color(0);
            point(&|c, _, _| [c[0] * filter[0], c[1] * filter[1], c[2] * filter[2], c[3]])
        }
        EffectKind::ColorInvert => point(&|c, _, _| [1.0 - c[0], 1.0 - c[1], 1.0 - c[2], c[3]]),
        EffectKind::ColorKey => {
            let key = effect.color(0);
            let tolerance = n(1);
            point(&|c, _, _| {
                let distance =
                    ((c[0] - key[0]).powi(2) + (c[1] - key[1]).powi(2) + (c[2] - key[2]).powi(2))
                        .sqrt()
                        / 3f32.sqrt();
                [
                    c[0],
                    c[1],
                    c[2],
                    c[3] * smoothstep(tolerance * 0.5, tolerance, distance),
                ]
            })
        }
        EffectKind::ColorPosterize => {
            let (levels, gamma) = (n(0).max(1.0), n(1));
            point(&|c, _, _| {
                with_rgb(
                    c,
                    rgb(c).map(|channel| {
                        let curved = channel.max(0.0).powf(gamma);
                        ((curved * levels).floor().min(levels - 1.0) / (levels - 1.0).max(1.0))
                            .powf(1.0 / gamma)
                    }),
                )
            })
        }
        EffectKind::ColorSwap => {
            let (tolerance, from, to) = (n(0), effect.color(1), effect.color(2));
            point(&|c, _, _| {
                let distance = ((c[0] - from[0]).powi(2)
                    + (c[1] - from[1]).powi(2)
                    + (c[2] - from[2]).powi(2))
                .sqrt()
                    / 3f32.sqrt();
                let weight = 1.0 - smoothstep(tolerance * 0.5, tolerance, distance);
                with_rgb(
                    c,
                    [0, 1, 2].map(|index| {
                        (c[index] + (to[index] - from[index]) * weight).clamp(0.0, 1.0)
                    }),
                )
            })
        }
        EffectKind::Dots => {
            let cell = ((6.0 + 18.0 * n(0)) * scale).max(2.0);
            map(width, height, |x, y| {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let (ix, iy) = ((px / cell).floor(), (py / cell).floor());
                let center = ((ix + 0.5) * cell, (iy + 0.5) * cell);
                let color = source.sample(center.0.min(w - 0.5), center.1.min(h - 0.5));
                let lum = luminance(color);
                let radius = cell * 0.5 * lum.sqrt() * 1.15;
                let distance = ((px - center.0).powi(2) + (py - center.1).powi(2)).sqrt();
                let coverage = 1.0 - smoothstep(radius - 0.75, radius + 0.75, distance);
                let dot = mix3([lum; 3], rgb(color), n(1));
                [
                    dot[0] * coverage,
                    dot[1] * coverage,
                    dot[2] * coverage,
                    color[3],
                ]
            })
        }
        EffectKind::EdgeBlur => {
            let blurred = blur(source, n(0) * 0.03 * side / 2.0);
            let inner = 1.0 - n(1);
            map(width, height, |x, y| {
                let distance = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2))
                    .sqrt()
                    / half_diagonal;
                let weight = smoothstep(inner - 0.25, inner + 0.25, distance);
                let (sharp, soft) = (source.pixels[y * width + x], blurred.pixels[y * width + x]);
                [0, 1, 2, 3]
                    .map(|channel| sharp[channel] + (soft[channel] - sharp[channel]) * weight)
            })
        }
        EffectKind::EdgeOutline => map(width, height, |x, y| {
            let c = source.pixels[y * width + x];
            let (gx, gy) = gradient(source, x, y);
            let edge = ((gx * gx + gy * gy).sqrt() * 2.0).min(1.0);
            let base = mix3([luminance(c); 3], rgb(c), n(1));
            with_rgb(
                c,
                base.map(|channel| channel * (1.0 - edge * (0.5 + n(0))))
                    .map(|value| value.max(0.0)),
            )
        }),
        EffectKind::Emboss => {
            let light = (n(0) * 2.0 - 1.0, n(1) * 2.0 - 1.0);
            map(width, height, |x, y| {
                let (gx, gy) = gradient(source, x, y);
                let value = (0.5 + (gx * light.0 + gy * light.1) * 0.5).clamp(0.0, 1.0);
                with_rgb(source.pixels[y * width + x], [value; 3])
            })
        }
        EffectKind::GrayInvert => point(&|c, _, _| with_rgb(c, [1.0 - luminance(c); 3])),
        EffectKind::Halftone => {
            let cell = (8.0 * scale).max(3.0);
            let softness = (1.0 - n(0)) * cell * 0.3 + 0.5;
            let (gcr, ucr) = (n(1), n(2));
            let angles = [15f32, 75.0, 0.0, 45.0].map(|degrees| degrees.to_radians().sin_cos());
            map(width, height, |x, y| {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let c = source.pixels[y * width + x];
                let (mut cyan, mut magenta, mut yellow) = (1.0 - c[0], 1.0 - c[1], 1.0 - c[2]);
                let black = (cyan.min(magenta).min(yellow) * gcr).min(1.0);
                cyan = (cyan - black * ucr).max(0.0);
                magenta = (magenta - black * ucr).max(0.0);
                yellow = (yellow - black * ucr).max(0.0);
                let coverage = |ink: f32, (sin, cos): (f32, f32)| {
                    let (u, v) = (px * cos + py * sin, -px * sin + py * cos);
                    let (fu, fv) = (
                        u / cell - (u / cell).floor() - 0.5,
                        v / cell - (v / cell).floor() - 0.5,
                    );
                    let distance = (fu * fu + fv * fv).sqrt() * cell;
                    let radius = ink.sqrt() * cell * 0.7;
                    1.0 - smoothstep(radius - softness, radius + softness, distance)
                };
                let k = coverage(black, angles[3]);
                [
                    (1.0 - coverage(cyan, angles[0])) * (1.0 - k),
                    (1.0 - coverage(magenta, angles[1])) * (1.0 - k),
                    (1.0 - coverage(yellow, angles[2])) * (1.0 - k),
                    c[3],
                ]
            })
        }
        EffectKind::HeatSignature => {
            const STOPS: [[f32; 3]; 6] = [
                [0.0, 0.0, 0.0],
                [0.1, 0.0, 0.6],
                [0.7, 0.0, 0.7],
                [1.0, 0.1, 0.0],
                [1.0, 0.9, 0.0],
                [1.0, 1.0, 1.0],
            ];
            point(&|c, _, _| {
                let position = luminance(c).clamp(0.0, 1.0) * 5.0;
                let index = (position.floor() as usize).min(4);
                with_rgb(
                    c,
                    mix3(STOPS[index], STOPS[index + 1], position - index as f32),
                )
            })
        }
        EffectKind::Hexagon => {
            let size = (n(0) * scale).max(2.0) / 2.0;
            map(width, height, |x, y| {
                // Pointy-top axial coordinates, rounded to the nearest hexagon.
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let q = (3f32.sqrt() / 3.0 * px - py / 3.0) / size;
                let r = (2.0 / 3.0 * py) / size;
                let (mut rx, mut ry, rz) = (q.round(), r.round(), (-q - r).round());
                let (dx, dy, dz) = ((rx - q).abs(), (ry - r).abs(), (rz + q + r).abs());
                if dx > dy && dx > dz {
                    rx = -ry - rz;
                } else if dy > dz {
                    ry = -rx - rz;
                }
                let center_x = size * 3f32.sqrt() * (rx + ry / 2.0);
                let center_y = size * 1.5 * ry;
                source.sample(center_x.clamp(0.5, w - 0.5), center_y.clamp(0.5, h - 0.5))
            })
        }
        EffectKind::ImageDistortion => {
            let power = n(0);
            let radius = cx.min(cy);
            warp(&|x, y| {
                let (dx, dy) = ((x - cx) / radius, (y - cy) / radius);
                let distance = (dx * dx + dy * dy).sqrt();
                if distance >= 1.0 || distance == 0.0 {
                    return (x, y);
                }
                let factor = distance.powf(power) / distance;
                (cx + dx * factor * radius, cy + dy * factor * radius)
            })
        }
        EffectKind::Kaleidoscope1 | EffectKind::Kaleidoscope2 => {
            let sides = n(1).max(2.0);
            let segment = TAU / sides;
            let center = (cx + n(2) * cx, cy + n(3) * cy);
            let spin = t * n(0);
            let rings = effect.kind == EffectKind::Kaleidoscope2;
            map(width, height, |x, y| {
                let (dx, dy) = (x as f32 + 0.5 - center.0, y as f32 + 0.5 - center.1);
                let mut radius = (dx * dx + dy * dy).sqrt();
                let mut angle = (dy.atan2(dx) + spin).rem_euclid(segment);
                if angle > segment / 2.0 {
                    angle = segment - angle;
                }
                if rings {
                    // Mirror the distance too, so the pattern repeats in rings.
                    let period = side * 0.25;
                    let phase = (radius / period).rem_euclid(2.0);
                    radius = if phase > 1.0 { 2.0 - phase } else { phase } * period;
                    angle -= spin * 2.0;
                }
                let (sin, cos) = angle.sin_cos();
                let (sx, sy) = (center.0 + cos * radius, center.1 + sin * radius);
                // Mirror-repeat outside the image.
                let fold = |value: f32, limit: f32| {
                    let phase = value.rem_euclid(limit * 2.0);
                    if phase > limit {
                        limit * 2.0 - phase
                    } else {
                        phase
                    }
                };
                source.sample(fold(sx, w), fold(sy, h))
            })
        }
        EffectKind::LensDistortion => {
            let strength = 0.25 * n(0);
            let spread = 0.06 * n(1);
            map(width, height, |x, y| {
                let (dx, dy) = (
                    (x as f32 + 0.5 - cx) / half_diagonal,
                    (y as f32 + 0.5 - cy) / half_diagonal,
                );
                let r2 = dx * dx + dy * dy;
                let channel = |index: usize| {
                    let k = strength * (1.0 + (index as f32 - 1.0) * spread * 4.0);
                    let factor = 1.0 / (1.0 + k * r2) * (1.0 + k * 0.5);
                    source.sample_inside(
                        cx + dx * factor * half_diagonal,
                        cy + dy * factor * half_diagonal,
                    )
                };
                let (red, green, blue) = (channel(0), channel(1), channel(2));
                [red[0], green[1], blue[2], green[3]]
            })
        }
        EffectKind::OldFilm => {
            let frame = (t * 24.0) as i64;
            let jitter = (hash(frame, 1, 3) - 0.5) * n(5) * h;
            let flicker = 1.0 + (hash(frame, 2, 3) - 0.5) * n(4) * 0.4;
            let scratches: Vec<f32> = (0..(n(2) * 6.0).round() as i64)
                .filter(|index| hash(frame, *index, 5) < 0.7)
                .map(|index| hash(frame, index, 7) * w)
                .collect();
            let vignette = 1.0 - n(3);
            map(width, height, |x, y| {
                let c = source.sample(x as f32 + 0.5, y as f32 + 0.5 + jitter);
                let mut color = mix3(rgb(c), sepia(rgb(c)), n(0));
                let grain = (hash(x as i64, y as i64, frame as u32) - 0.5) * n(1) * 0.35;
                let scratch = scratches
                    .iter()
                    .any(|position| (x as f32 + 0.5 - position).abs() < 0.75 * scale.max(1.0));
                let distance = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2))
                    .sqrt()
                    / half_diagonal;
                let shade = 1.0 - smoothstep(vignette * 0.8, vignette * 0.8 + 0.6, distance) * 0.8;
                color = color.map(|channel| ((channel + grain) * flicker * shade).clamp(0.0, 1.0));
                if scratch {
                    color = mix3(color, [0.9; 3], 0.6);
                }
                with_rgb(c, color)
            })
        }
        EffectKind::Picasso => {
            let time = t * n(0);
            let amplitude = 0.006 * side;
            let frequency = 8.0 / side;
            map(width, height, |x, y| {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let ox = (noise(px * frequency + time, py * frequency, 11) - 0.5) * 2.0 * amplitude;
                let oy = (noise(px * frequency, py * frequency + time, 13) - 0.5) * 2.0 * amplitude;
                let c = source.sample(px + ox, py + oy);
                let painted = saturate(rgb(c), 1.3).map(|channel| (channel * 6.0).round() / 6.0);
                with_rgb(c, painted)
            })
        }
        EffectKind::Pixelize => {
            let block = (n(0) * 0.06 * side).max(1.0);
            map(width, height, |x, y| {
                let bx = ((x as f32 / block).floor() + 0.5) * block;
                let by = ((y as f32 / block).floor() + 0.5) * block;
                source.sample(bx.min(w - 0.5), by.min(h - 0.5))
            })
        }
        EffectKind::Plasma => {
            let time = t * n(1) * 3.0;
            point(&|c, x, y| {
                let (u, v) = ((x as f32 + 0.5) / side * 8.0, (y as f32 + 0.5) / side * 8.0);
                let value = (u + time).sin()
                    + (v * 1.3 - time * 0.7).sin()
                    + ((u + v + time) * 0.7).sin()
                    + ((u * u + v * v).sqrt() * 0.9 - time).sin();
                let phase = value * PI / 2.0;
                let plasma = [
                    phase.sin(),
                    (phase + TAU / 3.0).sin(),
                    (phase + 2.0 * TAU / 3.0).sin(),
                ]
                .map(|channel| channel * 0.5 + 0.5);
                with_rgb(c, mix3(rgb(c), screen(rgb(c), plasma), n(0)))
            })
        }
        EffectKind::RadialBlur => {
            let time = t * n(1) * 2.0;
            let center = (cx + 0.1 * w * time.sin(), cy + 0.1 * h * time.cos());
            let iterations = n(2).round().max(1.0) as usize;
            let decay = n(3);
            map(width, height, |x, y| {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let mut sum = [0.0; 4];
                let mut total = 0.0;
                let mut weight = 1.0;
                for step in 0..iterations {
                    let k = step as f32 / iterations as f32 * 0.15;
                    let sample = source.sample(px + (center.0 - px) * k, py + (center.1 - py) * k);
                    for channel in 0..4 {
                        sum[channel] += sample[channel] * weight;
                    }
                    total += weight;
                    weight *= decay;
                }
                let blurred = sum.map(|value| value / total);
                let c = source.pixels[y * width + x];
                [0, 1, 2, 3].map(|channel| c[channel] + (blurred[channel] - c[channel]) * n(0))
            })
        }
        EffectKind::RippleCenter => warp(&|x, y| {
            let (dx, dy) = (x - cx, y - cy);
            let radius = (dx * dx + dy * dy).sqrt().max(1e-3);
            let offset = (radius / side * 80.0 - t * n(0) * 6.0).sin() * 0.004 * side;
            (x + dx / radius * offset, y + dy / radius * offset)
        }),
        EffectKind::RippleSide => warp(&|x, y| {
            (
                x + (y / side * 40.0 + t * n(0) * 4.0).sin() * 0.006 * side,
                y,
            )
        }),
        EffectKind::Sepia => point(&|c, _, _| with_rgb(c, sepia(rgb(c)))),
        EffectKind::Shake => {
            let rate = t * (1.0 + n(0) * 20.0);
            let amplitude = n(1) * 0.03 * side;
            let offset = (
                (noise(rate, 0.5, 17) - 0.5) * 2.0 * amplitude,
                (noise(0.5, rate, 19) - 0.5) * 2.0 * amplitude,
            );
            warp(&|x, y| (x + offset.0, y + offset.1))
        }
        EffectKind::Stars => {
            let travel = t * n(0) * 0.02;
            point(&|c, x, y| {
                let (dx, dy) = ((x as f32 + 0.5 - cx) / side, (y as f32 + 0.5 - cy) / side);
                let radius = (dx * dx + dy * dy).sqrt().max(1e-4);
                let angle = dy.atan2(dx);
                let mut light = 0.0f32;
                for layer in 0..3 {
                    // Stars stream outwards: depth wraps every unit of travel.
                    let depth = (travel + layer as f32 / 3.0).fract();
                    let ring = radius / (depth * depth + 0.02) * 0.4;
                    let cell_angle = (angle / TAU * 90.0).floor();
                    let cell_ring = (ring * 6.0).floor();
                    let star = hash(cell_angle as i64, cell_ring as i64, layer + 23);
                    if star > 0.94 {
                        let fade =
                            smoothstep(0.0, 0.3, depth) * (1.0 - smoothstep(0.85, 1.0, depth));
                        let within = 1.0 - ((ring * 6.0).fract() - 0.5).abs() * 2.0;
                        light = light.max(within.powi(6) * fade);
                    }
                }
                with_rgb(c, rgb(c).map(|channel| (channel + light * n(1)).min(1.0)))
            })
        }
        EffectKind::Strobe => {
            let rate = 0.5 + n(0) * 10.0;
            let cycle = (t * rate).floor();
            let jitter = (hash(cycle as i64, 3, 29) - 0.5) * n(3);
            let phase = (t * rate).fract() + jitter;
            let on = phase.rem_euclid(1.0) < n(1);
            let flash = effect.color(4);
            point(&|c, _, _| {
                if on {
                    with_rgb(c, mix3(rgb(c), flash, n(2)))
                } else {
                    c
                }
            })
        }
        EffectKind::Tile => {
            let cells = n(0).max(1.0);
            let grid = n(1) * scale;
            map(width, height, |x, y| {
                let (cell_w, cell_h) = (w / cells, h / cells);
                let (lx, ly) = ((x as f32 + 0.5) % cell_w, (y as f32 + 0.5) % cell_h);
                if grid > 0.0
                    && (lx < grid / 2.0
                        || ly < grid / 2.0
                        || cell_w - lx < grid / 2.0
                        || cell_h - ly < grid / 2.0)
                {
                    return [0.0, 0.0, 0.0, 1.0];
                }
                source.sample(lx * cells, ly * cells)
            })
        }
        EffectKind::Toon => {
            let levels = n(0).max(1.0);
            map(width, height, |x, y| {
                let c = source.pixels[y * width + x];
                let (gx, gy) = gradient(source, x, y);
                let edge = smoothstep(0.35, 0.6, (gx * gx + gy * gy).sqrt());
                let toned = saturate(rgb(c), 1.0 + n(1))
                    .map(|channel| ((channel * levels).round() / levels).min(1.0) * (1.0 - edge));
                with_rgb(c, toned)
            })
        }
        EffectKind::TwoColor => {
            let (dark, bright) = (effect.color(0), effect.color(1));
            point(&|c, _, _| with_rgb(c, mix3(dark, bright, luminance(c).clamp(0.0, 1.0))))
        }
        EffectKind::Vignette => {
            let inner = n(0) / 100.0;
            point(&|c, x, y| {
                let distance = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2))
                    .sqrt()
                    / half_diagonal;
                let shade = 1.0 - smoothstep(inner, inner + 0.5, distance);
                with_rgb(c, rgb(c).map(|channel| channel * shade))
            })
        }
    }
}
