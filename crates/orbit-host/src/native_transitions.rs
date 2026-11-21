//! Slide and layer transitions for native output.
//!
//! The catalogue mirrors ProPresenter's transition library (`GLSL/*.rvtx`):
//! effects are identified by their ProPresenter UUID so imported transitions
//! keep their identity. Every effect is reimplemented on the CPU from the
//! outgoing and incoming layer planes; ProPresenter's shaders are not used.

use serde_json::{json, Value};
use std::f64::consts::{PI, TAU};
use std::time::Instant;

/// How an effect places its outgoing and incoming planes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionKind {
    Cut,
    Dissolve,
    Fade,
    FadeThrough(FadeColor),
    FadeGray,
    WarpFade,
    WaveDissolve,
    Push,
    MoveIn,
    Iris,
    Flip,
    FlyIn,
    Mosaic,
    Ripple,
    Swap,
    Wipe,
    Reveal,
    Amoeba,
    SquareWipe,
    DispersionBlur,
    NoisyZoom,
    ZoomIn,
    ColorBurn,
    ColorPush,
    ColorWarp,
    FilmBurn,
    CrossHatch,
    Cube,
    Door,
    DoorWipe,
    KaleidoscopeWipe,
    RandomPixels,
    RandomSquares,
    RandomSquaresFlicker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeColor {
    Black,
    White,
    Bright,
    Dark,
}

pub struct CatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub kind: TransitionKind,
    /// ProPresenter direction bitmask of the allowed directions; zero when the effect has none.
    pub directions: u32,
    /// ProPresenter's default direction (bitmask value).
    pub default_direction: u32,
}

const ALL_EDGES: u32 = 495;
const SIDES: u32 = 170;

const fn entry(
    id: &'static str,
    name: &'static str,
    category: &'static str,
    kind: TransitionKind,
    directions: u32,
    default_direction: u32,
) -> CatalogEntry {
    CatalogEntry {
        id,
        name,
        category,
        kind,
        directions,
        default_direction,
    }
}

/// ProPresenter's transition library, keyed by its render UUIDs.
#[rustfmt::skip]
pub const CATALOG: &[CatalogEntry] = &[
    entry("AB29D07B-E9E2-4E0A-93BD-AD3EA58120FA", "Cut", "None", TransitionKind::Cut, 0, 0),
    entry("EC52A828-AD85-4602-B70C-1DEE7C904DB6", "Dissolve", "Dissolves", TransitionKind::Dissolve, 0, 0),
    entry("99BDD1C3-EE98-4E80-A8DE-3699CE9F338E", "Fade", "Dissolves", TransitionKind::Fade, 0, 0),
    entry("01E95287-E84D-4638-9E28-18C8735ABE47", "Fade Black", "Dissolves", TransitionKind::FadeThrough(FadeColor::Black), 0, 0),
    entry("802181F9-E290-4FA9-A9ED-06CAA2306A12", "Fade Bright", "Dissolves", TransitionKind::FadeThrough(FadeColor::Bright), 0, 0),
    entry("E4D84DA7-E8E3-43B5-820E-8D883416D355", "Fade Dark", "Dissolves", TransitionKind::FadeThrough(FadeColor::Dark), 0, 0),
    entry("C30699D4-3249-4B51-A809-191662294ED0", "Fade Gray", "Dissolves", TransitionKind::FadeGray, 0, 0),
    entry("C2123A39-357E-4626-9257-783AF40918FC", "Fade White", "Dissolves", TransitionKind::FadeThrough(FadeColor::White), 0, 0),
    entry("72091328-AA13-4312-86FD-D54A5FC10A9C", "Warp Fade", "Dissolves", TransitionKind::WarpFade, 0, 0),
    entry("0540598C-5CED-4F80-AB0E-3566FEA85BCF", "Wave Dissolve", "Dissolves", TransitionKind::WaveDissolve, 0, 0),
    entry("B400F82E-CF9D-4700-89FD-F3889DAA06C5", "Push", "Movements", TransitionKind::Push, SIDES, 2),
    entry("31B698F6-500B-4097-B209-C6F811A99A7C", "Move In", "Movements", TransitionKind::MoveIn, ALL_EDGES, 1),
    entry("F694B97E-2C1C-4888-B6AE-0619617A16CF", "Iris", "Movements", TransitionKind::Iris, 0, 0),
    entry("82DE5438-1851-4011-929C-D59A292D9C79", "Flip", "Movements", TransitionKind::Flip, 0, 0),
    entry("ADF02083-C3B8-4A47-BC9A-BA57A887C404", "Fly In", "Movements", TransitionKind::FlyIn, 511, 1),
    entry("9BE0AB3A-9239-4976-BA34-417EADB27995", "Mosaic", "Movements", TransitionKind::Mosaic, 0, 0),
    entry("47A0842A-E064-4282-854A-27224E48D7B0", "Ripple", "Movements", TransitionKind::Ripple, 0, 0),
    entry("6E087942-135D-4239-9093-98AD4BA86274", "Swap", "Movements", TransitionKind::Swap, 0, 0),
    entry("7084A71F-C12C-4EEA-8D90-A2D30FB0B256", "Wipe", "Wipes", TransitionKind::Wipe, ALL_EDGES, 1),
    entry("3EA3BC78-9A06-480D-8E6E-3A51A47BD0D6", "Reveal", "Wipes", TransitionKind::Reveal, ALL_EDGES, 1),
    entry("B5E9AD29-DCEC-4AD2-A78D-3C49A101E1D2", "Amoeba", "Wipes", TransitionKind::Amoeba, 0, 0),
    entry("DF000B14-0F80-4A8B-86A5-903564032437", "Square Wipe", "Wipes", TransitionKind::SquareWipe, ALL_EDGES, 1),
    entry("F1475C78-329A-4897-B5C0-266C6433CCCC", "Dispersion Blur", "Blurs", TransitionKind::DispersionBlur, 0, 0),
    entry("F6AF1CC0-F049-40DC-926E-8CA350C2909D", "Noisy Zoom", "Blurs", TransitionKind::NoisyZoom, 0, 0),
    entry("928B442C-4912-42F6-B526-60921E0F0D40", "Zoom In", "Blurs", TransitionKind::ZoomIn, 511, 1),
    entry("2B855B65-6ABC-4F65-8198-A19A5EF97821", "Color Burn", "Color", TransitionKind::ColorBurn, 0, 0),
    entry("E958E7B0-E5C2-4C10-9B22-D1FE4FEA1E49", "Color Push", "Color", TransitionKind::ColorPush, 0, 0),
    entry("1EC1B05C-4E79-460C-9429-4129D9094E73", "Color Warp", "Color", TransitionKind::ColorWarp, 0, 0),
    entry("2CB0361E-5E77-4E7F-AEDB-EA6673F743EF", "Film Burn", "Color", TransitionKind::FilmBurn, 0, 0),
    entry("5584E51F-5C92-47B5-9B65-3C2540C1F20C", "Cross Hatch", "Objects", TransitionKind::CrossHatch, 0, 0),
    entry("CD545FC3-70FA-4120-8A48-29EFE903BD10", "Cube", "Objects", TransitionKind::Cube, 0, 0),
    entry("DAA798F1-359A-46C3-AEC6-5C3DFE93635A", "Door", "Objects", TransitionKind::Door, 0, 0),
    entry("645A6350-128C-48B9-AA9D-D7D85ABEF81E", "Door Wipe", "Objects", TransitionKind::DoorWipe, 0, 0),
    entry("5717B340-8ED9-457C-A6EE-74A8D2679E43", "Kaleidoscope Wipe", "Objects", TransitionKind::KaleidoscopeWipe, 0, 0),
    entry("DF2C1B6C-5B62-49A1-B72E-C80CB3C0CA75", "Random Pixels", "Objects", TransitionKind::RandomPixels, 0, 0),
    entry("39030834-E94F-417E-A8B4-4870C9C7D413", "Random Squares", "Objects", TransitionKind::RandomSquares, 0, 0),
    entry("E4650E1F-B42E-4492-9824-BE4D8E27E833", "Random Squares Flicker", "Objects", TransitionKind::RandomSquaresFlicker, 0, 0),
];

/// ProPresenter's `EffectDirection` bitmask values and their Orbit names.
pub const DIRECTIONS: &[(u32, &str)] = &[
    (1, "top-left"),
    (2, "top"),
    (4, "top-right"),
    (8, "left"),
    (16, "center"),
    (32, "right"),
    (64, "bottom-left"),
    (128, "bottom"),
    (256, "bottom-right"),
];

/// How an effect variable is edited and stored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VariableKind {
    Number {
        min: f64,
        max: f64,
    },
    /// An opaque RGB color, stored in Orbit as `#rrggbb`.
    Color,
}

/// One adjustable effect setting from the library descriptor.
pub struct VariableSpec {
    pub effect_id: &'static str,
    pub name: &'static str,
    pub label: &'static str,
    pub kind: VariableKind,
    /// The value the library effect uses (RGB for colors, the first item for numbers).
    pub value: [f64; 3],
    /// ProPresenter's reset value.
    pub default_value: [f64; 3],
}

const fn number(
    effect_id: &'static str,
    name: &'static str,
    label: &'static str,
    (min, max): (f64, f64),
    value: f64,
    default_value: f64,
) -> VariableSpec {
    VariableSpec {
        effect_id,
        name,
        label,
        kind: VariableKind::Number { min, max },
        value: [value, 0.0, 0.0],
        default_value: [default_value, 0.0, 0.0],
    }
}

/// The library variables other than direction, from ProPresenter's `.rvtx` descriptors.
#[rustfmt::skip]
pub const VARIABLES: &[VariableSpec] = &[
    number("B5E9AD29-DCEC-4AD2-A78D-3C49A101E1D2", "scale", "Scale", (10.0, 100.0), 40.0, 40.0),
    VariableSpec {
        effect_id: "2B855B65-6ABC-4F65-8198-A19A5EF97821",
        name: "burnColor",
        label: "Burn Color",
        kind: VariableKind::Color,
        value: [0.9, 0.24, 0.1],
        default_value: [0.5, 0.5, 0.5],
    },
    number("1EC1B05C-4E79-460C-9429-4129D9094E73", "zoom", "Zoom", (10.01, 40.1), 20.5, 20.05),
    number("1EC1B05C-4E79-460C-9429-4129D9094E73", "size", "Size", (0.01, 1.1), 0.5, 0.05),
    number("1EC1B05C-4E79-460C-9429-4129D9094E73", "colorSeparation", "Color Separation", (0.01, 4.1), 0.85, 0.85),
    number("F1475C78-329A-4897-B5C0-266C6433CCCC", "size", "Radius", (0.0, 2.0), 0.5, 0.5),
    number("F1475C78-329A-4897-B5C0-266C6433CCCC", "angle", "Angle", (0.0, 4.0), 0.5, 2.3),
    number("39030834-E94F-417E-A8B4-4870C9C7D413", "squareSize", "Size", (1.0, 10.0), 2.5, 2.5),
];

/// The variables of one library effect.
pub fn effect_variables(effect_id: &str) -> impl Iterator<Item = &'static VariableSpec> + '_ {
    VARIABLES
        .iter()
        .filter(move |variable| variable.effect_id.eq_ignore_ascii_case(effect_id))
}

/// Formats an RGB color in `0..=1` as `#rrggbb`.
pub fn color_hex(rgb: [f64; 3]) -> String {
    let byte = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(rgb[0]),
        byte(rgb[1]),
        byte(rgb[2])
    )
}

/// Parses `#rrggbb` into an RGB color in `0..=1`.
pub fn parse_color_hex(value: &str) -> Option<[f64; 3]> {
    let hex = value.strip_prefix('#').filter(|hex| hex.len() == 6)?;
    let channel = |index: usize| {
        u8::from_str_radix(hex.get(index..index + 2)?, 16)
            .ok()
            .map(|byte| byte as f64 / 255.0)
    };
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// Reads a variable's value from an Orbit `variables` value, if it is valid.
pub fn variable_value(variable: &VariableSpec, value: &Value) -> Option<[f64; 3]> {
    match variable.kind {
        VariableKind::Number { min, max } => value
            .as_f64()
            .filter(|number| number.is_finite() && (min..=max).contains(number))
            .map(|number| [number, 0.0, 0.0]),
        VariableKind::Color => value.as_str().and_then(parse_color_hex),
    }
}

/// The Orbit form of a variable value.
pub fn variable_json(variable: &VariableSpec, value: [f64; 3]) -> Value {
    match variable.kind {
        VariableKind::Number { .. } => json!(value[0]),
        VariableKind::Color => json!(color_hex(value)),
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
                    "category": entry.category,
                    "directions": DIRECTIONS
                        .iter()
                        .filter(|(bitmask, _)| entry.directions & bitmask != 0)
                        .map(|(_, name)| *name)
                        .collect::<Vec<_>>(),
                    "defaultDirection": direction_name(entry.default_direction),
                    "variables": effect_variables(entry.id)
                        .map(|variable| {
                            let mut value = json!({
                                "name": variable.name,
                                "label": variable.label,
                                "value": variable_json(variable, variable.value),
                            });
                            match variable.kind {
                                VariableKind::Number { min, max } => {
                                    value["type"] = json!("number");
                                    value["min"] = json!(min);
                                    value["max"] = json!(max);
                                }
                                VariableKind::Color => value["type"] = json!("color"),
                            }
                            value
                        })
                        .collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

pub fn catalog_entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG
        .iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(id))
}

pub fn direction_name(bitmask: u32) -> Option<&'static str> {
    DIRECTIONS
        .iter()
        .find(|(value, _)| *value == bitmask)
        .map(|(_, name)| *name)
}

pub fn direction_bitmask(name: &str) -> Option<u32> {
    DIRECTIONS
        .iter()
        .find(|(_, value)| *value == name)
        .map(|(bitmask, _)| *bitmask)
}

/// A transition resolved for native rendering.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionSpec {
    pub kind: TransitionKind,
    pub duration_seconds: f64,
    /// Unit vector (screen coordinates, y down) pointing to the edge the incoming slide enters from.
    pub side: (f64, f64),
    /// Every variable of the effect, authored or at its library value.
    pub variables: Vec<(&'static str, [f64; 3])>,
}

impl TransitionSpec {
    fn variable(&self, name: &str) -> [f64; 3] {
        self.variables
            .iter()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| *value)
            .unwrap_or_else(|| panic!("{name} is not a variable of this effect"))
    }
}

/// Validates an Orbit transition value (`effectId`, `durationSeconds`, optional
/// `direction` and optional `variables` that override library values).
pub fn validate(value: &Value) -> Result<(), String> {
    let object = value.as_object().ok_or("A transition must be an object.")?;
    if let Some(key) = object.keys().find(|key| {
        !matches!(
            key.as_str(),
            "effectId" | "durationSeconds" | "direction" | "variables"
        )
    }) {
        return Err(format!("Unsupported transition field: {key}."));
    }
    let id = object
        .get("effectId")
        .and_then(Value::as_str)
        .ok_or("A transition requires an effectId.")?;
    let entry = catalog_entry(id).ok_or_else(|| format!("Unknown transition effect: {id}."))?;
    let duration = object
        .get("durationSeconds")
        .and_then(Value::as_f64)
        .ok_or("A transition requires durationSeconds.")?;
    if !duration.is_finite() || !(0.0..=10.0).contains(&duration) {
        return Err("Transition duration must be between 0 and 10 seconds.".into());
    }
    if let Some(direction) = object.get("direction") {
        let bitmask = direction
            .as_str()
            .and_then(direction_bitmask)
            .ok_or("Transition direction is invalid.")?;
        if entry.directions & bitmask == 0 {
            return Err(format!(
                "{} does not support the {} direction.",
                entry.name,
                direction.as_str().unwrap_or_default()
            ));
        }
    }
    if let Some(variables) = object.get("variables") {
        let variables = variables
            .as_object()
            .ok_or("Transition variables must be an object.")?;
        for (name, value) in variables {
            let variable = effect_variables(entry.id)
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
                    VariableKind::Color => {
                        format!("{} {} must be a #rrggbb color.", entry.name, variable.label)
                    }
                });
            }
        }
    }
    Ok(())
}

/// Resolves the native rendering for a validated transition value. Cut and
/// zero durations render as a cut (`None`).
pub fn resolve(value: &Value) -> Result<Option<TransitionSpec>, String> {
    validate(value)?;
    let entry = catalog_entry(value["effectId"].as_str().unwrap_or_default())
        .ok_or("Unknown transition effect.")?;
    let duration_seconds = value["durationSeconds"].as_f64().unwrap_or_default();
    if entry.kind == TransitionKind::Cut || duration_seconds <= 0.0 {
        return Ok(None);
    }
    let bitmask = value
        .get("direction")
        .and_then(Value::as_str)
        .and_then(direction_bitmask)
        .unwrap_or(entry.default_direction);
    let variables = effect_variables(entry.id)
        .map(|variable| {
            let authored = value
                .get("variables")
                .and_then(|variables| variables.get(variable.name))
                .and_then(|value| variable_value(variable, value));
            (variable.name, authored.unwrap_or(variable.value))
        })
        .collect();
    Ok(Some(TransitionSpec {
        kind: entry.kind,
        duration_seconds,
        side: side_vector(bitmask),
        variables,
    }))
}

/// The library duration of ProPresenter's effects (`.rvtx` default duration).
pub const LIBRARY_DURATION_SECONDS: f64 = 0.6;
pub const DISSOLVE_ID: &str = "EC52A828-AD85-4602-B70C-1DEE7C904DB6";
pub const CUT_ID: &str = "AB29D07B-E9E2-4E0A-93BD-AD3EA58120FA";

/// How many recently picked default transitions the transition picker keeps.
pub const RECENT_LIMIT: usize = 6;

/// `picked` moved to the front of `recents`, once, capped at [`RECENT_LIMIT`].
pub fn remember_recent(recents: &[String], picked: &str) -> Vec<String> {
    std::iter::once(picked.to_string())
        .chain(
            recents
                .iter()
                .filter(|id| !id.eq_ignore_ascii_case(picked))
                .cloned(),
        )
        .take(RECENT_LIMIT)
        .collect()
}

/// The recents of a workspace must be known effects, at most [`RECENT_LIMIT`] of them, without repeats.
pub fn validate_recents(value: Option<&Value>) -> Result<(), String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let ids = value
        .as_array()
        .ok_or("Recent transitions must be an array.")?;
    if ids.len() > RECENT_LIMIT {
        return Err("Too many recent transitions.".into());
    }
    let mut seen = Vec::new();
    for id in ids {
        let id = id
            .as_str()
            .ok_or("A recent transition must be an effect id.")?;
        catalog_entry(id).ok_or("A recent transition is not a library effect.")?;
        if seen.contains(&id) {
            return Err("Recent transitions must not repeat.".into());
        }
        seen.push(id);
    }
    Ok(())
}

/// A rounded card on a dark plane, as a straight-alpha BGRA plane for the previews.
fn preview_plane(width: usize, height: usize, card: [u8; 3]) -> Vec<u8> {
    let (w, h) = (width as f64, height as f64);
    let (half_w, half_h) = (w * 0.28, h * 0.30);
    let radius = h * 0.12;
    let mut plane = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let dx = (x as f64 + 0.5 - w / 2.0).abs();
            let dy = (y as f64 + 0.5 - h / 2.0).abs();
            let (qx, qy) = (dx - half_w + radius, dy - half_h + radius);
            let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
            let [r, g, b] = if distance <= 0.0 { card } else { [24, 24, 28] };
            plane.extend([b, g, r, 255]);
        }
    }
    plane
}

/// The effect at `progress` (0 to 1) between two contrasting sample cards, for the transition picker. A cut
/// shows the outgoing card until half way. The frame comes from the same blend the output renders.
pub fn render_preview(
    effect_id: &str,
    progress: f64,
    width: usize,
    height: usize,
) -> Result<crate::native_compositor::NativeFrame, String> {
    if !(1..=1024).contains(&width) || !(1..=1024).contains(&height) {
        return Err("Preview size must be between 1 and 1024 pixels.".into());
    }
    if !(0.0..=1.0).contains(&progress) {
        return Err("Preview progress must be between 0 and 1.".into());
    }
    let from = preview_plane(width, height, [255, 146, 43]);
    let to = preview_plane(width, height, [140, 70, 230]);
    let spec = resolve(&json!({"effectId": effect_id, "durationSeconds": 1.0}))?;
    let pixels = match spec {
        Some(spec) => {
            let mut output = vec![0; width * height * 4];
            blend(&spec, progress, &from, &to, &mut output, width, height);
            output
        }
        None if progress < 0.5 => from,
        None => to,
    };
    Ok(crate::native_compositor::NativeFrame {
        width,
        height,
        pixels,
        dynamic_media: false,
    })
}

/// Reads a media selection's transition: `none` defers to the layer default (`None`);
/// `cut`, `dissolve` (at the library duration) and `fade:<seconds>:<effect>` name a
/// library transition, the effect by UUID or name at its default direction.
pub fn media_transition(value: &str) -> Result<Option<Value>, String> {
    let transition = match value {
        "none" => return Ok(None),
        "cut" => json!({ "effectId": CUT_ID, "durationSeconds": 0.0 }),
        "dissolve" => {
            json!({ "effectId": DISSOLVE_ID, "durationSeconds": LIBRARY_DURATION_SECONDS })
        }
        _ => {
            let invalid = || format!("Media transition '{value}' is not a library transition.");
            let mut parts = value.splitn(3, ':');
            if parts.next() != Some("fade") {
                return Err(invalid());
            }
            let duration = parts
                .next()
                .and_then(|seconds| seconds.parse::<f64>().ok())
                .ok_or_else(invalid)?;
            let effect = parts.next().ok_or_else(invalid)?;
            let entry = catalog_entry(effect)
                .or_else(|| {
                    CATALOG
                        .iter()
                        .find(|entry| entry.name.eq_ignore_ascii_case(effect))
                })
                .ok_or_else(invalid)?;
            json!({ "effectId": entry.id, "durationSeconds": duration })
        }
    };
    validate(&transition)?;
    Ok(Some(transition))
}

/// The layers with a workspace default transition, as ProPresenter's global layer transitions.
pub const DEFAULT_LAYERS: [&str; 4] = ["background", "foreground", "props", "messages"];

/// Validates the workspace's `layerTransitions`: a library transition per named layer.
pub fn validate_layer_defaults(value: Option<&Value>) -> Result<(), String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let layers = value
        .as_object()
        .ok_or("Layer transitions must be an object.")?;
    for (layer, transition) in layers {
        if !DEFAULT_LAYERS.contains(&layer.as_str()) {
            return Err(format!("Unknown transition layer '{layer}'."));
        }
        validate(transition)?;
    }
    Ok(())
}

/// Whether a scene's slide or layer change animates, so a prerendered frame must not replace it.
pub fn scene_animates(content: &Value) -> bool {
    let animates = |value: &Value| !value.is_null() && matches!(resolve(value), Ok(Some(_)));
    let layers = content.get("layerTransitions").and_then(Value::as_object);
    content
        .get("slideTransition")
        .and_then(|context| context.get("transition"))
        .is_some_and(animates)
        || layers.is_some_and(|layers| {
            layers.values().any(|context| {
                context.get("transition").is_some_and(animates)
                    || context
                        .get("transitions")
                        .and_then(Value::as_object)
                        .is_some_and(|transitions| transitions.values().any(animates))
            })
        })
}

fn side_vector(bitmask: u32) -> (f64, f64) {
    let x = if bitmask & (1 | 8 | 64) != 0 {
        -1.0
    } else if bitmask & (4 | 32 | 256) != 0 {
        1.0
    } else {
        0.0
    };
    let y = if bitmask & (1 | 2 | 4) != 0 {
        -1.0
    } else if bitmask & (64 | 128 | 256) != 0 {
        1.0
    } else {
        0.0
    };
    (x, y)
}

/// A running transition on one output.
pub struct ActiveTransition {
    pub spec: TransitionSpec,
    pub started: Instant,
    pub from: Vec<u8>,
}

impl ActiveTransition {
    pub fn progress(&self, now: Instant) -> f64 {
        (now.saturating_duration_since(self.started).as_secs_f64() / self.spec.duration_seconds)
            .clamp(0.0, 1.0)
    }
}

/// Premultiplied color in the planes' B, G, R, A channel order.
type Rgba = [f64; 4];

fn ease(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    ease(((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0))
}

/// Premultiplied sample of a straight-alpha BGRA plane, transparent outside it.
fn sample(plane: &[u8], width: usize, height: usize, x: isize, y: isize) -> Rgba {
    if x as usize >= width || y as usize >= height {
        return [0.0; 4];
    }
    let offset = (y as usize * width + x as usize) * 4;
    let alpha = plane[offset + 3] as f64 * (1.0 / 255.0);
    let scale = alpha * (1.0 / 255.0);
    [
        plane[offset] as f64 * scale,
        plane[offset + 1] as f64 * scale,
        plane[offset + 2] as f64 * scale,
        alpha,
    ]
}

/// Integer floor; `f64::floor` is a library call on baseline x86-64.
fn floor(value: f64) -> isize {
    let truncated = value as isize;
    truncated - ((truncated as f64) > value) as isize
}

/// Nearest premultiplied sample at fractional pixel-index coordinates.
fn nearest(plane: &[u8], width: usize, height: usize, x: f64, y: f64) -> Rgba {
    sample(plane, width, height, floor(x + 0.5), floor(y + 0.5))
}

/// Bilinear premultiplied sample at fractional pixel-index coordinates.
pub(crate) fn bilinear(plane: &[u8], width: usize, height: usize, x: f64, y: f64) -> Rgba {
    if !x.is_finite() || !y.is_finite() {
        return [0.0; 4];
    }
    let (xi, yi) = (floor(x), floor(y));
    let (tx, ty) = (x - xi as f64, y - yi as f64);
    let row = |yi: isize| {
        let left = sample(plane, width, height, xi, yi);
        if tx == 0.0 {
            return left;
        }
        add(
            scaled(left, 1.0 - tx),
            scaled(sample(plane, width, height, xi + 1, yi), tx),
        )
    };
    let top = row(yi);
    if ty == 0.0 {
        return top;
    }
    add(scaled(top, 1.0 - ty), scaled(row(yi + 1), ty))
}

/// Applies a fade-through-color to one premultiplied pixel at visibility `p`.
fn fade_through(color: Rgba, fade: FadeColor, p: f64) -> Rgba {
    let alpha = color[3];
    let mut rgb = [color[0], color[1], color[2]];
    for value in &mut rgb {
        *value = if p < 0.5 {
            match fade {
                FadeColor::Black | FadeColor::Dark => 0.0,
                FadeColor::White | FadeColor::Bright => alpha,
            }
        } else {
            let remaining = (1.0 - p) * 2.0;
            match fade {
                FadeColor::Black => *value * (1.0 - remaining),
                FadeColor::White => *value + (alpha - *value) * remaining,
                FadeColor::Bright => (*value + remaining * alpha).min(alpha),
                FadeColor::Dark => (*value - remaining * alpha).max(0.0),
            }
        };
    }
    let scale = if p < 0.5 { p * 2.0 } else { 1.0 };
    [
        rgb[0] * scale,
        rgb[1] * scale,
        rgb[2] * scale,
        alpha * scale,
    ]
}

fn grayscale(color: Rgba, amount: f64) -> Rgba {
    let gray = 0.0722 * color[0] + 0.7152 * color[1] + 0.2126 * color[2];
    [
        color[0] + (gray - color[0]) * amount,
        color[1] + (gray - color[1]) * amount,
        color[2] + (gray - color[2]) * amount,
        color[3],
    ]
}

fn scaled(color: Rgba, weight: f64) -> Rgba {
    color.map(|value| value * weight)
}

fn add(a: Rgba, b: Rgba) -> Rgba {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

fn mix(a: Rgba, b: Rgba, amount: f64) -> Rgba {
    add(scaled(a, 1.0 - amount), scaled(b, amount))
}

fn over(top: Rgba, bottom: Rgba) -> Rgba {
    add(top, scaled(bottom, 1.0 - top[3]))
}

/// Deterministic per-cell noise in `[0, 1)`.
fn hash(x: i64, y: i64, seed: u32) -> f64 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h as f64 / 4_294_967_296.0
}

/// Smooth value noise in `[0, 1]` over a lattice of unit cells.
fn value_noise(x: f64, y: f64, seed: u32) -> f64 {
    let (xi, yi) = (floor(x), floor(y));
    let (tx, ty) = (ease(x - xi as f64), ease(y - yi as f64));
    let (xi, yi) = (xi as i64, yi as i64);
    let top = hash(xi, yi, seed) * (1.0 - tx) + hash(xi + 1, yi, seed) * tx;
    let bottom = hash(xi, yi + 1, seed) * (1.0 - tx) + hash(xi + 1, yi + 1, seed) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// Which slide plane a geometric piece shows.
#[derive(Clone, Copy)]
enum Plane {
    From,
    To,
}

type Vec3 = [f64; 3];

fn rotate_y(vector: Vec3, angle: f64) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    [
        vector[0] * cos + vector[2] * sin,
        vector[1],
        -vector[0] * sin + vector[2] * cos,
    ]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A rectangle of a slide plane placed in 3D. The screen is the plane z = 0,
/// centred on the origin, with y down and z pointing away from the viewer.
struct Quad {
    plane: Plane,
    /// World position of plane coordinate (0, 0).
    origin: Vec3,
    /// World step per plane pixel along x and y.
    u: Vec3,
    v: Vec3,
    /// Plane x range shown by this quad (half-open).
    x_range: (f64, f64),
    opacity: f64,
}

impl Quad {
    /// A piece of `plane` turned by `angle` about the vertical axis through `pivot`.
    /// `offset` is plane coordinate (0, 0) relative to the pivot before turning,
    /// and `axis` is the unturned direction of the plane's x axis.
    fn turned(
        plane: Plane,
        pivot: Vec3,
        offset: Vec3,
        axis: Vec3,
        angle: f64,
        x_range: (f64, f64),
        opacity: f64,
    ) -> Self {
        let offset = rotate_y(offset, angle);
        Quad {
            plane,
            origin: [
                pivot[0] + offset[0],
                pivot[1] + offset[1],
                pivot[2] + offset[2],
            ],
            u: rotate_y(axis, angle),
            v: [0.0, 1.0, 0.0],
            x_range,
            opacity,
        }
    }

    /// Intersects the viewing ray through screen point (`sx`, `sy`); returns the
    /// plane coordinates and the ray depth, culling the back face.
    fn hit(&self, sx: f64, sy: f64, distance: f64, height: f64) -> Option<(f64, f64, f64)> {
        let ray = [sx, sy, distance];
        if dot(cross(self.u, self.v), ray) <= 0.0 {
            return None;
        }
        // Solve origin + px*u + py*v = k*ray - (0, 0, distance) for px, py, k.
        let target = [-self.origin[0], -self.origin[1], -distance - self.origin[2]];
        let negative_ray = [-ray[0], -ray[1], -ray[2]];
        let det = dot(self.u, cross(self.v, negative_ray));
        if det.abs() < 1e-9 {
            return None;
        }
        let px = dot(target, cross(self.v, negative_ray)) / det;
        let py = dot(self.u, cross(target, negative_ray)) / det;
        let k = dot(self.u, cross(self.v, target)) / det;
        (k > 0.0 && px >= self.x_range.0 && px < self.x_range.1 && (0.0..height).contains(&py))
            .then_some((px, py, k))
    }
}

/// Per-frame values shared by every pixel of one blended frame.
struct Frame<'a> {
    kind: TransitionKind,
    from: &'a [u8],
    to: &'a [u8],
    width: usize,
    height: usize,
    w: f64,
    h: f64,
    /// Linear progress.
    t: f64,
    /// Eased progress.
    e: f64,
    /// Effect intensity, peaking halfway.
    peak: f64,
    side: (f64, f64),
    incoming_offset: (isize, isize),
    outgoing_offset: (isize, isize),
    /// Viewer distance for the 3D effects.
    distance: f64,
    quads: Vec<Quad>,
    /// Dispersion Blur's tap directions before the per-pixel jitter.
    dispersion: [(f64, f64); DISPERSION_TAPS],
    /// Numeric effect variables, in the order the effect reads them.
    numbers: [f64; 3],
    /// Color Burn's color (B, G, R).
    burn_color: [f64; 3],
}

const DISPERSION_TAPS: usize = 6;

/// Camera distance in screen widths; keeps turning planes in front of the viewer.
const VIEW_DISTANCE: f64 = 2.0;

impl<'a> Frame<'a> {
    fn new(
        spec: &TransitionSpec,
        t: f64,
        from: &'a [u8],
        to: &'a [u8],
        width: usize,
        height: usize,
    ) -> Self {
        let e = ease(t);
        let (w, h) = (width as f64, height as f64);
        let (side_x, side_y) = spec.side;
        let shift = |amount: f64| {
            (
                (side_x * amount * w).round() as isize,
                (side_y * amount * h).round() as isize,
            )
        };
        let half_w = w / 2.0;
        let corner = [-half_w, -h / 2.0, 0.0];
        let x_axis = [1.0, 0.0, 0.0];
        let quads = match spec.kind {
            TransitionKind::Flip if e < 0.5 => vec![Quad::turned(
                Plane::From,
                [0.0; 3],
                corner,
                x_axis,
                PI * e,
                (0.0, w),
                1.0,
            )],
            TransitionKind::Flip => vec![Quad::turned(
                Plane::To,
                [0.0; 3],
                corner,
                x_axis,
                PI * e - PI,
                (0.0, w),
                1.0,
            )],
            TransitionKind::Cube => {
                // The incoming slide sits on the right face and turns to the front.
                let pivot = [0.0, 0.0, half_w];
                let angle = PI / 2.0 * e;
                vec![
                    Quad::turned(
                        Plane::From,
                        pivot,
                        [-half_w, -h / 2.0, -half_w],
                        x_axis,
                        angle,
                        (0.0, w),
                        1.0,
                    ),
                    Quad::turned(
                        Plane::To,
                        pivot,
                        [half_w, -h / 2.0, -half_w],
                        [0.0, 0.0, 1.0],
                        angle,
                        (0.0, w),
                        1.0,
                    ),
                ]
            }
            TransitionKind::Door | TransitionKind::DoorWipe => {
                // Two halves hinged on the outer edges; Door opens the outgoing
                // slide towards the viewer, Door Wipe closes the incoming one.
                let (doors, opening, behind) = if spec.kind == TransitionKind::Door {
                    (Plane::From, e, Plane::To)
                } else {
                    (Plane::To, 1.0 - e, Plane::From)
                };
                let angle = PI / 2.0 * opening;
                let behind = if spec.kind == TransitionKind::Door {
                    Quad::turned(
                        behind,
                        [0.0, 0.0, (1.0 - e) * half_w],
                        corner,
                        x_axis,
                        0.0,
                        (0.0, w),
                        e,
                    )
                } else {
                    Quad::turned(behind, [0.0; 3], corner, x_axis, 0.0, (0.0, w), 1.0 - e)
                };
                vec![
                    behind,
                    Quad::turned(
                        doors,
                        [-half_w, 0.0, 0.0],
                        [0.0, -h / 2.0, 0.0],
                        x_axis,
                        angle,
                        (0.0, half_w),
                        1.0,
                    ),
                    Quad::turned(
                        doors,
                        [half_w, 0.0, 0.0],
                        [-w, -h / 2.0, 0.0],
                        x_axis,
                        -angle,
                        (half_w, w),
                        1.0,
                    ),
                ]
            }
            _ => Vec::new(),
        };
        Frame {
            kind: spec.kind,
            from,
            to,
            width,
            height,
            w,
            h,
            t,
            e,
            peak: (PI * t).sin(),
            side: spec.side,
            incoming_offset: shift(1.0 - e),
            outgoing_offset: shift(-e),
            distance: VIEW_DISTANCE * w,
            quads,
            dispersion: std::array::from_fn(|tap| {
                let angle = match spec.kind {
                    TransitionKind::DispersionBlur => spec.variable("angle")[0],
                    _ => 0.0,
                };
                let (sin, cos) = (angle + TAU * tap as f64 / DISPERSION_TAPS as f64).sin_cos();
                (cos, sin)
            }),
            numbers: match spec.kind {
                TransitionKind::Amoeba => [spec.variable("scale")[0], 0.0, 0.0],
                TransitionKind::ColorWarp => [
                    spec.variable("zoom")[0],
                    spec.variable("size")[0],
                    spec.variable("colorSeparation")[0],
                ],
                TransitionKind::DispersionBlur => [spec.variable("size")[0], 0.0, 0.0],
                TransitionKind::RandomSquares => [spec.variable("squareSize")[0], 0.0, 0.0],
                _ => [0.0; 3],
            },
            burn_color: match spec.kind {
                TransitionKind::ColorBurn => {
                    let [red, green, blue] = spec.variable("burnColor");
                    [blue, green, red]
                }
                _ => [0.0; 3],
            },
        }
    }

    fn plane(&self, plane: Plane) -> &[u8] {
        match plane {
            Plane::From => self.from,
            Plane::To => self.to,
        }
    }

    fn a(&self, x: isize, y: isize) -> Rgba {
        sample(self.from, self.width, self.height, x, y)
    }

    fn b(&self, x: isize, y: isize) -> Rgba {
        sample(self.to, self.width, self.height, x, y)
    }

    fn at(&self, plane: Plane, x: f64, y: f64) -> Rgba {
        bilinear(self.plane(plane), self.width, self.height, x, y)
    }

    /// Samples a plane scaled by `scale` about the screen point (`cx`, `cy`),
    /// averaging `taps` samples along the line towards that point.
    fn zoomed(
        &self,
        plane: Plane,
        x: f64,
        y: f64,
        (cx, cy): (f64, f64),
        scale: f64,
        blur: f64,
    ) -> Rgba {
        const TAPS: usize = 4;
        let (dx, dy) = (x + 0.5 - cx, y + 0.5 - cy);
        if blur <= 0.0 {
            return self.at(plane, cx + dx / scale - 0.5, cy + dy / scale - 0.5);
        }
        // The blur hides sampling detail, so its taps sample the nearest pixel.
        let pixels = self.plane(plane);
        let mut total = [0.0; 4];
        for tap in 0..TAPS {
            let tap_scale = scale * (1.0 + blur * tap as f64 / TAPS as f64);
            total = add(
                total,
                nearest(
                    pixels,
                    self.width,
                    self.height,
                    cx + dx / tap_scale - 0.5,
                    cy + dy / tap_scale - 0.5,
                ),
            );
        }
        scaled(total, 1.0 / TAPS as f64)
    }

    /// Samples one color channel of both planes with a per-channel displacement.
    fn split_channels(&self, displacement: impl Fn(usize) -> (Rgba, Rgba)) -> (Rgba, Rgba) {
        let mut a = [0.0; 4];
        let mut b = [0.0; 4];
        for channel in 0..3 {
            let (from, to) = displacement(channel);
            a[channel] = from[channel];
            b[channel] = to[channel];
            a[3] += from[3] / 3.0;
            b[3] += to[3] / 3.0;
        }
        (a, b)
    }

    fn pixel(&self, x: usize, y: usize) -> Rgba {
        let (xi, yi) = (x as isize, y as isize);
        let (fx, fy) = (x as f64, y as f64);
        let (u, v) = ((fx + 0.5) / self.w, (fy + 0.5) / self.h);
        let (cx, cy) = (self.w / 2.0, self.h / 2.0);
        let (dx, dy) = (fx + 0.5 - cx, fy + 0.5 - cy);
        let norm = self.w.max(self.h);
        let (side_x, side_y) = self.side;
        let (t, e, peak) = (self.t, self.e, self.peak);
        let a = || self.a(xi, yi);
        let b = || self.b(xi, yi);
        match self.kind {
            TransitionKind::Cut => b(),
            TransitionKind::Dissolve => mix(a(), b(), e),
            TransitionKind::Fade => add(
                scaled(a(), (1.0 - 2.0 * t).max(0.0)),
                scaled(b(), (2.0 * t - 1.0).max(0.0)),
            ),
            TransitionKind::FadeThrough(fade) => {
                add(fade_through(a(), fade, 1.0 - e), fade_through(b(), fade, e))
            }
            TransitionKind::FadeGray => add(
                scaled(grayscale(a(), 1.0 - smoothstep(0.6, 1.0, 1.0 - e)), 1.0 - e),
                scaled(grayscale(b(), 1.0 - smoothstep(0.6, 1.0, e)), e),
            ),
            TransitionKind::WarpFade => {
                // Both slides swirl around the centre, strongest near it.
                let radius = (dx * dx + dy * dy).sqrt() / (cx * cx + cy * cy).sqrt();
                let falloff = (1.0 - radius).max(0.0).powi(2);
                let swirl = |angle: f64, plane| {
                    let (sin, cos) = (angle * falloff).sin_cos();
                    self.at(
                        plane,
                        cx + dx * cos - dy * sin - 0.5,
                        cy + dx * sin + dy * cos - 0.5,
                    )
                };
                mix(
                    swirl(2.0 * e, Plane::From),
                    swirl(-2.0 * (1.0 - e), Plane::To),
                    e,
                )
            }
            TransitionKind::WaveDissolve => {
                let wave = (v * 4.0 * TAU + t * 2.0 * TAU).sin();
                let offset = 0.04 * self.w * peak * wave;
                let amount = (e * 1.5 - 0.25 + 0.25 * (u * 2.0 * TAU).sin()).clamp(0.0, 1.0);
                mix(
                    self.at(Plane::From, fx + offset, fy),
                    self.at(Plane::To, fx - offset, fy),
                    amount,
                )
            }
            TransitionKind::Push => add(
                self.a(xi - self.outgoing_offset.0, yi - self.outgoing_offset.1),
                self.b(xi - self.incoming_offset.0, yi - self.incoming_offset.1),
            ),
            // The outgoing slide fades too, so it cannot linger under a transparent incoming one.
            TransitionKind::MoveIn => over(
                self.b(xi - self.incoming_offset.0, yi - self.incoming_offset.1),
                scaled(a(), 1.0 - e),
            ),
            TransitionKind::Reveal => over(
                self.a(xi - self.outgoing_offset.0, yi - self.outgoing_offset.1),
                scaled(b(), e),
            ),
            TransitionKind::Wipe => {
                let along_x = if side_x < 0.0 { u } else { 1.0 - u };
                let along_y = if side_y < 0.0 { v } else { 1.0 - v };
                let edges = side_x.abs() + side_y.abs();
                let distance = (along_x * side_x.abs() + along_y * side_y.abs()) / edges.max(1.0);
                let front = e * 1.2 - 0.1;
                let reveal = 1.0 - smoothstep(front - 0.1, front + 0.1, distance);
                mix(a(), b(), reveal)
            }
            TransitionKind::Iris => {
                let (cu, cv) = (u - 0.5, v - 0.5);
                let outside = (((cu * cu + cv * cv).sqrt() - e + 0.05) * 20.0).clamp(0.0, 1.0);
                mix(b(), a(), outside)
            }
            TransitionKind::Flip
            | TransitionKind::Cube
            | TransitionKind::Door
            | TransitionKind::DoorWipe => self.project(dx, dy),
            TransitionKind::FlyIn => {
                // The incoming slide flies in from the far distance along the direction.
                let depth = (1.0 - e) * 2.0 * self.w;
                let scale = self.distance / (self.distance + depth);
                let incoming = (
                    cx + side_x * self.w * 0.6 * (1.0 - e),
                    cy + side_y * self.h * 0.6 * (1.0 - e),
                );
                let outgoing_scale = self.distance / (self.distance + e * self.w);
                over(
                    scaled(
                        self.zoomed(Plane::To, fx, fy, incoming, scale, 0.0),
                        smoothstep(0.0, 0.3, t),
                    ),
                    scaled(
                        self.zoomed(Plane::From, fx, fy, (cx, cy), outgoing_scale, 0.0),
                        1.0 - e,
                    ),
                )
            }
            TransitionKind::Mosaic => {
                // Tiles turn over one by one, at random moments.
                let columns = 8.0;
                let rows = (columns * self.h / self.w).round().max(1.0);
                let (tile_w, tile_h) = (self.w / columns, self.h / rows);
                let (column, row) = (
                    floor((fx + 0.5) / tile_w) as f64,
                    floor((fy + 0.5) / tile_h) as f64,
                );
                let delay = hash(column as i64, row as i64, 1) * 0.5;
                let turn = ease(((t - delay) / 0.5).clamp(0.0, 1.0));
                let width = (PI * turn).cos().abs();
                let tile_center = (column + 0.5) * tile_w;
                let local = fx + 0.5 - tile_center;
                if local.abs() > width * tile_w / 2.0 {
                    return [0.0; 4];
                }
                let plane = if turn < 0.5 { Plane::From } else { Plane::To };
                self.at(plane, tile_center + local / width - 0.5, fy)
            }
            TransitionKind::Ripple => {
                let radius = (dx * dx + dy * dy).sqrt();
                let wave = 0.02 * norm * peak * (radius / norm * 60.0 - t * 25.0).sin();
                let (nx, ny) = if radius > 0.0 {
                    (dx / radius, dy / radius)
                } else {
                    (0.0, 0.0)
                };
                mix(
                    self.at(Plane::From, fx + nx * wave, fy + ny * wave),
                    self.at(Plane::To, fx + nx * wave, fy + ny * wave),
                    e,
                )
            }
            TransitionKind::Swap => {
                // The slides trade places, passing each other side by side in depth.
                let spread = (PI * e).sin() * 0.55 * self.w;
                let place = |plane, offset: f64, depth: f64, opacity: f64| {
                    let scale = self.distance / (self.distance + depth);
                    scaled(
                        self.zoomed(plane, fx, fy, (cx + offset, cy), scale, 0.0),
                        opacity,
                    )
                };
                let outgoing = place(
                    Plane::From,
                    -spread,
                    e * 0.6 * self.w,
                    (2.0 * (1.0 - e)).min(1.0),
                );
                let incoming = place(
                    Plane::To,
                    spread,
                    (1.0 - e) * 0.6 * self.w,
                    (2.0 * e).min(1.0),
                );
                if e < 0.5 {
                    over(outgoing, incoming)
                } else {
                    over(incoming, outgoing)
                }
            }
            TransitionKind::Amoeba => {
                // The library scale of 40 spreads four blobs across the width.
                let scale = self.numbers[0] / 10.0;
                let (nu, nv) = (u * scale, v * scale * self.h / self.w);
                let noise =
                    value_noise(nu, nv, 3) * 0.65 + value_noise(nu * 2.3, nv * 2.3, 5) * 0.35;
                let front = e * 1.4 - 0.2;
                mix(
                    a(),
                    b(),
                    1.0 - smoothstep(front - 0.05, front + 0.05, noise),
                )
            }
            TransitionKind::SquareWipe => {
                let (ox, oy) = ((0.5 + side_x * 0.5) * self.w, (0.5 + side_y * 0.5) * self.h);
                let reach = (ox.max(self.w - ox)).max(oy.max(self.h - oy));
                let distance = (fx + 0.5 - ox).abs().max((fy + 0.5 - oy).abs()) / reach;
                let front = e * 1.1 - 0.05;
                mix(
                    a(),
                    b(),
                    1.0 - smoothstep(front - 0.05, front + 0.05, distance),
                )
            }
            TransitionKind::DispersionBlur => {
                // Samples scatter over a disc that widens and closes again.
                const TAPS: usize = DISPERSION_TAPS;
                let radius = self.numbers[0] * 0.08 * norm * peak;
                let (jitter_sin, jitter_cos) =
                    (TAU * hash(x as i64, y as i64, 7) / TAPS as f64).sin_cos();
                let mut from = [0.0; 4];
                let mut to = [0.0; 4];
                for (tap, (tap_x, tap_y)) in self.dispersion.iter().enumerate() {
                    let distance = radius * ((tap as f64 + 0.5) / TAPS as f64).sqrt();
                    let ox = (tap_x * jitter_cos - tap_y * jitter_sin) * distance;
                    let oy = (tap_x * jitter_sin + tap_y * jitter_cos) * distance;
                    let (sx, sy) = (floor(fx + ox + 0.5), floor(fy + oy + 0.5));
                    from = add(from, self.a(sx, sy));
                    to = add(to, self.b(sx, sy));
                }
                mix(from, to, e).map(|value| value / TAPS as f64)
            }
            TransitionKind::NoisyZoom => {
                let outgoing =
                    self.zoomed(Plane::From, fx, fy, (cx, cy), 1.0 + 0.5 * e, 0.1 * peak);
                let incoming = self.zoomed(
                    Plane::To,
                    fx,
                    fy,
                    (cx, cy),
                    1.0 / (1.0 + 0.5 * (1.0 - e)),
                    0.1 * peak,
                );
                let color = mix(outgoing, incoming, e);
                let grain =
                    (hash(x as i64, y as i64, (t * 60.0) as u32) - 0.5) * 0.4 * peak * color[3];
                [
                    color[0] + grain,
                    color[1] + grain,
                    color[2] + grain,
                    color[3],
                ]
                .map(|value| value.max(0.0))
            }
            TransitionKind::ZoomIn => {
                let anchor = ((0.5 + side_x * 0.5) * self.w, (0.5 + side_y * 0.5) * self.h);
                mix(
                    self.zoomed(Plane::From, fx, fy, anchor, 1.0 + 2.0 * e, 0.2 * peak),
                    self.zoomed(
                        Plane::To,
                        fx,
                        fy,
                        anchor,
                        1.0 / (1.0 + 2.0 * (1.0 - e)),
                        0.2 * peak,
                    ),
                    e,
                )
            }
            TransitionKind::ColorBurn => {
                let burn = |color: Rgba| {
                    let alpha = color[3];
                    if alpha <= 0.0 {
                        return color;
                    }
                    let mut burned = color;
                    for channel in 0..3 {
                        let straight = color[channel] / alpha;
                        let target =
                            (1.0 - (1.0 - straight) / self.burn_color[channel].max(0.01)).max(0.0);
                        burned[channel] = (straight + (target - straight) * peak) * alpha;
                    }
                    burned
                };
                mix(burn(a()), burn(b()), e)
            }
            TransitionKind::ColorPush => {
                // A push from the right whose red channel leads and blue trails.
                let (from, to) = self.split_channels(|channel| {
                    let lag = [0.25, 0.125, 0.0][channel];
                    let offset = ease((t * 1.25 - lag).clamp(0.0, 1.0)) * self.w;
                    (
                        self.at(Plane::From, fx + offset, fy),
                        self.at(Plane::To, fx + offset - self.w, fy),
                    )
                });
                add(from, to)
            }
            TransitionKind::ColorWarp => {
                let [zoom, size, separation] = self.numbers;
                let amplitude = size * 0.04 * norm * peak;
                let wave_x = amplitude * (v * zoom + t * TAU).sin();
                let wave_y = amplitude * (u * zoom + t * TAU).cos();
                let (from, to) = self.split_channels(|channel| {
                    let factor = 1.0 + (channel as f64 - 1.0) * separation * 0.5;
                    let (ox, oy) = (wave_x * factor, wave_y * factor);
                    (
                        self.at(Plane::From, fx + ox, fy + oy),
                        self.at(Plane::To, fx + ox, fy + oy),
                    )
                });
                mix(from, to, e)
            }
            TransitionKind::FilmBurn => {
                // A drifting warm light leak flares up halfway.
                let leak = value_noise(u * 3.0 + t * 2.0, v * 2.0, 9);
                let glow = (peak * peak * leak * 1.6).min(0.9);
                let color = mix(a(), b(), e);
                [
                    color[0] + 0.25 * glow,
                    color[1] + 0.6 * glow,
                    color[2] + glow,
                    color[3] + glow - color[3] * glow,
                ]
            }
            TransitionKind::CrossHatch => {
                let period = norm / 8.0;
                let stripe = |value: f64| {
                    let cycles = value / period;
                    ((cycles - floor(cycles) as f64) - 0.5).abs() * 2.0
                };
                let distance = stripe(fx + fy + 0.5).min(stripe(fx - fy + self.h));
                let front = e * 1.3 - 0.15;
                mix(
                    a(),
                    b(),
                    1.0 - smoothstep(front - 0.075, front + 0.075, distance),
                )
            }
            TransitionKind::KaleidoscopeWipe => {
                let segment = TAU / 6.0;
                let angle = (dy.atan2(dx) + e * PI / 3.0).rem_euclid(segment) / segment;
                let fold = (angle - 0.5).abs() * 2.0;
                let radius = (dx * dx + dy * dy).sqrt() / (cx * cx + cy * cy).sqrt();
                let distance = 0.5 * fold + 0.5 * radius;
                let front = e * 1.2 - 0.1;
                mix(
                    a(),
                    b(),
                    1.0 - smoothstep(front - 0.05, front + 0.05, distance),
                )
            }
            TransitionKind::RandomPixels => {
                if hash(x as i64, y as i64, 11) < t {
                    b()
                } else {
                    a()
                }
            }
            TransitionKind::RandomSquares | TransitionKind::RandomSquaresFlicker => {
                // Square size is a percentage of the width; Flicker has no variable and
                // uses the library's 2.5%.
                let percent = if self.kind == TransitionKind::RandomSquares {
                    self.numbers[0]
                } else {
                    2.5
                };
                let size = (self.w * percent / 100.0).max(1.0);
                let (column, row) = ((fx / size) as i64, (fy / size) as i64);
                let seed = if self.kind == TransitionKind::RandomSquaresFlicker {
                    (t * 24.0) as u32
                } else {
                    13
                };
                if hash(column, row, seed) < t {
                    b()
                } else {
                    a()
                }
            }
        }
    }

    /// Composites the frame's 3D quads at one screen point, far to near.
    fn project(&self, sx: f64, sy: f64) -> Rgba {
        let mut hits = [(0.0, [0.0; 4]); 3];
        let mut count = 0;
        for quad in self.quads.iter().filter(|quad| quad.opacity > 0.0) {
            let Some((px, py, depth)) = quad.hit(sx, sy, self.distance, self.h) else {
                continue;
            };
            hits[count] = (
                depth,
                scaled(self.at(quad.plane, px - 0.5, py - 0.5), quad.opacity),
            );
            count += 1;
        }
        let hits = &mut hits[..count];
        hits.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        hits.iter()
            .fold([0.0; 4], |below, (_, color)| over(*color, below))
    }
}

/// Blends the outgoing and incoming straight-alpha planes into `output` at linear progress `t`.
pub fn blend(
    spec: &TransitionSpec,
    t: f64,
    from: &[u8],
    to: &[u8],
    output: &mut [u8],
    width: usize,
    height: usize,
) {
    blend_frame(spec, t, from, to, output, width, height, false);
}

/// Object Dissolves retain linear opacity while geometric effects share the catalogue's easing.
pub(crate) fn blend_object(
    spec: &TransitionSpec,
    t: f64,
    from: &[u8],
    to: &[u8],
    output: &mut [u8],
    width: usize,
    height: usize,
) {
    blend_frame(spec, t, from, to, output, width, height, true);
}

#[allow(clippy::too_many_arguments)]
fn blend_frame(
    spec: &TransitionSpec,
    t: f64,
    from: &[u8],
    to: &[u8],
    output: &mut [u8],
    width: usize,
    height: usize,
    object: bool,
) {
    if width == 0 || height == 0 {
        return;
    }
    let mut frame = Frame::new(spec, t, from, to, width, height);
    if object && spec.kind == TransitionKind::Dissolve {
        frame.e = t;
    }
    let threads = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(height);
    let rows_per_chunk = height.div_ceil(threads);
    std::thread::scope(|scope| {
        for (chunk_index, chunk) in output.chunks_mut(rows_per_chunk * width * 4).enumerate() {
            let frame = &frame;
            scope.spawn(move || {
                for (row, line) in chunk.chunks_mut(width * 4).enumerate() {
                    let y = chunk_index * rows_per_chunk + row;
                    for (x, pixel) in line.chunks_mut(4).enumerate() {
                        write_pixel(pixel, frame.pixel(x, y));
                    }
                }
            });
        }
    });
}

pub(crate) fn write_pixel(pixel: &mut [u8], color: Rgba) {
    let alpha = color[3].clamp(0.0, 1.0);
    // Values are non-negative, so adding a half and truncating rounds them.
    let coverage = (alpha * 255.0 + 0.5) as u8;
    if coverage == 0 {
        pixel.copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    let unpremultiply = 255.0 / alpha;
    for channel in 0..3 {
        pixel[channel] = ((color[channel] * unpremultiply).clamp(0.0, 255.0) + 0.5) as u8;
    }
    pixel[3] = coverage;
}
