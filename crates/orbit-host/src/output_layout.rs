//! Logical screens own their canvas; device records become temporary physical canvas followers.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub use crate::key_fill::FillMode;
use crate::output_canvas::{CanvasRegion, CanvasSize};
pub use crate::projector_correction::BlendMode;
use crate::projector_correction::{Point, ProjectorCorners, ProjectorCorrection};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutMode {
    Single,
    Mirror,
    Grouped,
    EdgeBlend,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceTarget {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub frame_rate: f64,
    pub format: String,
    pub video_delay: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub gamma: f64,
    pub black: f64,
    pub brightness: f64,
    pub contrast: f64,
}

impl OutputColor {
    pub fn validate(self) -> Result<(), String> {
        for value in [
            self.red,
            self.green,
            self.blue,
            self.black,
            self.brightness,
            self.contrast,
        ] {
            range(value, -1., 1., "Output color adjustment")?;
        }
        range(self.gamma, -2., 2., "Output gamma")
    }

    pub fn apply(self, value: f64, channel: usize) -> f64 {
        let level = [self.blue, self.green, self.red][channel];
        let value = (value * (1. + level))
            .clamp(0., 1.)
            .powf(2_f64.powf(-self.gamma));
        ((value - 0.5) * (1. + self.contrast) + 0.5 + self.black + self.brightness).clamp(0., 1.)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlendCurve {
    pub mode: BlendMode,
    pub radius: f64,
    pub intensity: f64,
    pub gamma: f64,
    pub black_level: f64,
}

impl Default for BlendCurve {
    fn default() -> Self {
        Self {
            mode: BlendMode::Linear,
            radius: 1.,
            intensity: 1.,
            gamma: 0.,
            black_level: 0.,
        }
    }
}

impl BlendCurve {
    pub fn validate(self) -> Result<(), String> {
        range(self.radius, 0.01, 1., "Blend radius")?;
        range(self.intensity, 0., 2., "Blend intensity")?;
        range(self.gamma, -2., 2., "Blend gamma")?;
        range(self.black_level, 0., 1., "Blend black level")
    }

    pub fn weight(self, position: f64) -> f64 {
        let t = (position / self.radius).clamp(0., 1.);
        let t = match self.mode {
            BlendMode::Linear => t,
            BlendMode::Quadratic if t < 0.5 => 2. * t * t,
            BlendMode::Quadratic => 1. - 2. * (1. - t) * (1. - t),
            BlendMode::Cubic => t * t * (3. - 2. * t),
        };
        (t.powf(2_f64.powf(-self.gamma)) * self.intensity + self.black_level).clamp(0., 1.)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlendSeam {
    pub width: usize,
    pub left: BlendCurve,
    pub right: BlendCurve,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AlphaOutput {
    pub mode: crate::key_fill::FillMode,
    pub target: DeviceTarget,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputDevice {
    pub id: String,
    pub label: String,
    pub target: DeviceTarget,
    pub color: OutputColor,
    pub corner_pin: Option<ProjectorCorners>,
    pub alpha: Option<AlphaOutput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputLayout {
    pub mode: LayoutMode,
    pub rows: usize,
    pub columns: usize,
    pub tile_width: usize,
    pub tile_height: usize,
    pub outputs: Vec<OutputDevice>,
    pub blends: Vec<BlendSeam>,
    pub screen_color: Option<String>,
}

pub fn neutral_corners() -> ProjectorCorners {
    ProjectorCorners {
        top_left: Point { x: 0., y: 0. },
        top_right: Point { x: 1., y: 0. },
        bottom_right: Point { x: 1., y: 1. },
        bottom_left: Point { x: 0., y: 1. },
    }
}

pub fn neutral_correction() -> ProjectorCorrection {
    ProjectorCorrection {
        edges: crate::projector_correction::EdgeFractions {
            left: 0.,
            right: 0.,
            top: 0.,
            bottom: 0.,
        },
        blend_mode: BlendMode::Linear,
        blend_gamma: 1.,
        gain: crate::projector_correction::RgbGain {
            red: 1.,
            green: 1.,
            blue: 1.,
        },
        black_level: 0.,
        corners: neutral_corners(),
        color: None,
        edge_curves: None,
    }
}

impl DeviceTarget {
    pub fn placeholder(width: usize, height: usize) -> Self {
        Self {
            kind: "placeholder".into(),
            id: String::new(),
            name: String::new(),
            width,
            height,
            frame_rate: 60.,
            format: "sdr".into(),
            video_delay: 0,
            source_type: None,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if !matches!(self.kind.as_str(), "placeholder" | "display" | "ndi") {
            return Err("Output target must be placeholder, display or ndi.".into());
        }
        if (self.kind == "display" && self.id.trim().is_empty())
            || (self.kind == "ndi" && self.name.trim().is_empty())
        {
            return Err(
                "An assigned output target requires an explicit display ID or NDI name.".into(),
            );
        }
        CanvasSize {
            width: self.width,
            height: self.height,
        }
        .validate()?;
        range(self.frame_rate, 1., 120., "Output frame rate")?;
        crate::native_hdr::OutputFormat::parse(&self.format)?;
        crate::output_delay::validate_size(self.width, self.height, self.video_delay)
    }

    fn config(&self, parent: &Value, label: &str) -> Value {
        json!({"type": parent["type"], "label": label, "enabled": parent["enabled"] == true && self.kind != "placeholder",
            "targetType": self.kind, "targetId": self.id, "ndiName": self.name,
            "ndiWidth": self.width, "ndiHeight": self.height, "ndiFramerate": self.frame_rate,
            "ndiAlphaMode": "none", "outputFormat": self.format, "videoDelay": self.video_delay})
    }
}

impl OutputLayout {
    pub fn from_screen(screen_id: &str, config: &Value) -> Result<Self, String> {
        if let Some(value) = config.get("outputLayout").filter(|v| !v.is_null()) {
            let layout: Self = serde_json::from_value(value.clone())
                .map_err(|error| format!("Output layout is invalid: {error}"))?;
            layout.validate()?;
            return Ok(layout);
        }
        let width = config["ndiWidth"].as_u64().unwrap_or(1920) as usize;
        let height = config["ndiHeight"].as_u64().unwrap_or(1080) as usize;
        let mut target = DeviceTarget::placeholder(width, height);
        if config["targetType"] == "ndi" {
            target.kind = "ndi".into();
            target.name = config["ndiName"].as_str().unwrap_or("").into();
        } else if let Some(id) = config["targetId"].as_str().filter(|id| !id.is_empty()) {
            target.kind = "display".into();
            target.id = id.into();
        }
        target.format = config["outputFormat"].as_str().unwrap_or("sdr").into();
        target.frame_rate = config["ndiFramerate"].as_f64().unwrap_or(60.);
        target.video_delay = crate::output_delay::frames(config.get("videoDelay"))?;
        Ok(Self {
            mode: LayoutMode::Single,
            rows: 1,
            columns: 1,
            tile_width: width,
            tile_height: height,
            outputs: vec![OutputDevice {
                id: uuid::Uuid::new_v5(
                    &uuid::Uuid::NAMESPACE_URL,
                    format!("presenter-orbit-output:{screen_id}:primary").as_bytes(),
                )
                .to_string(),
                label: config["label"].as_str().unwrap_or("").into(),
                target,
                color: OutputColor::default(),
                corner_pin: None,
                alpha: None,
            }],
            blends: Vec::new(),
            screen_color: None,
        })
    }

    pub fn resize(
        &mut self,
        mode: LayoutMode,
        rows: usize,
        columns: usize,
        count: usize,
    ) -> Result<(), String> {
        let count = match mode {
            LayoutMode::Single => 1,
            LayoutMode::Grouped => rows
                .checked_mul(columns)
                .ok_or("Output grid is too large.")?,
            _ => count,
        };
        if !(1..=16).contains(&count) || !(1..=16).contains(&rows) || !(1..=16).contains(&columns) {
            return Err("Output layouts support between 1 and 16 devices.".into());
        }
        self.mode = mode;
        self.rows = if mode == LayoutMode::Grouped { rows } else { 1 };
        self.columns = if mode == LayoutMode::Grouped {
            columns
        } else {
            count
        };
        self.outputs.truncate(count);
        while self.outputs.len() < count {
            self.outputs.push(OutputDevice {
                id: uuid::Uuid::new_v4().to_string(),
                label: String::new(),
                target: DeviceTarget::placeholder(self.tile_width, self.tile_height),
                color: OutputColor::default(),
                corner_pin: None,
                alpha: None,
            });
        }
        let seams = if mode == LayoutMode::EdgeBlend {
            count - 1
        } else {
            0
        };
        self.blends.truncate(seams);
        self.blends.resize(
            seams,
            BlendSeam {
                width: self.tile_width.min(400) / 2,
                left: BlendCurve::default(),
                right: BlendCurve::default(),
            },
        );
        self.validate()
    }

    pub fn canvas(&self) -> Result<CanvasSize, String> {
        let width = match self.mode {
            LayoutMode::Single | LayoutMode::Mirror => self.tile_width,
            LayoutMode::Grouped => self.tile_width * self.columns,
            LayoutMode::EdgeBlend => {
                self.tile_width * self.outputs.len()
                    - self.blends.iter().map(|b| b.width).sum::<usize>()
            }
        };
        CanvasSize {
            width,
            height: if self.mode == LayoutMode::Grouped {
                self.tile_height * self.rows
            } else {
                self.tile_height
            },
        }
        .validate()
    }

    pub fn region(&self, index: usize) -> CanvasRegion {
        let (x, y) = match self.mode {
            LayoutMode::Single | LayoutMode::Mirror => (0, 0),
            LayoutMode::Grouped => (
                (index % self.columns) * self.tile_width,
                (index / self.columns) * self.tile_height,
            ),
            LayoutMode::EdgeBlend => (
                index * self.tile_width
                    - self
                        .blends
                        .iter()
                        .take(index)
                        .map(|b| b.width)
                        .sum::<usize>(),
                0,
            ),
        };
        CanvasRegion {
            x,
            y,
            width: self.tile_width,
            height: self.tile_height,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(1..=16).contains(&self.outputs.len())
            || !(1..=16).contains(&self.rows)
            || !(1..=16).contains(&self.columns)
        {
            return Err("Output layouts require 1 to 16 devices, rows and columns.".into());
        }
        if (self.mode == LayoutMode::Single && self.outputs.len() != 1)
            || (self.mode == LayoutMode::Grouped && self.rows * self.columns != self.outputs.len())
            || self.blends.len()
                != if self.mode == LayoutMode::EdgeBlend {
                    self.outputs.len() - 1
                } else {
                    0
                }
        {
            return Err(
                "Output layout device and blend counts do not match the arrangement.".into(),
            );
        }
        CanvasSize {
            width: self.tile_width,
            height: self.tile_height,
        }
        .validate()?;
        for seam in &self.blends {
            if seam.width == 0 || seam.width >= self.tile_width.div_ceil(2) {
                return Err(
                    "Blend overlap must be positive and less than half a device width.".into(),
                );
            }
            seam.left.validate()?;
            seam.right.validate()?;
        }
        self.canvas()?;
        if self
            .screen_color
            .as_ref()
            .is_some_and(|color| crate::native_compositor::parse_color(color).is_none())
        {
            return Err("Screen color must be a supported color value.".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut targets = std::collections::HashSet::new();
        for device in &self.outputs {
            if device.id.trim().is_empty() || !ids.insert(&device.id) {
                return Err("Output device IDs must be nonempty and unique.".into());
            }
            device.target.validate()?;
            device.color.validate()?;
            if let Some(corners) = device.corner_pin {
                corners.validate()?;
            }
            for target in std::iter::once(&device.target)
                .chain(device.alpha.as_ref().map(|alpha| &alpha.target))
            {
                target.validate()?;
                if target.kind != "placeholder"
                    && !targets.insert((
                        &target.kind,
                        if target.kind == "ndi" {
                            &target.name
                        } else {
                            &target.id
                        },
                    ))
                {
                    return Err("An output destination cannot be assigned to more than one device in a layout.".into());
                }
            }
            if let Some(alpha) = &device.alpha {
                if alpha.target.kind != "placeholder"
                    && device.target.kind != "placeholder"
                    && (alpha.target.kind != device.target.kind
                        || alpha.target.width != device.target.width
                        || alpha.target.height != device.target.height
                        || alpha.target.frame_rate != device.target.frame_rate
                        || alpha.target.video_delay != device.target.video_delay
                        || alpha.target.format != "sdr"
                        || device.target.format != "sdr")
                {
                    return Err("Fill and key targets require the same output kind, SDR size, rate and delay.".into());
                }
            }
        }
        Ok(())
    }
}

pub fn physical_id(screen: &str, device: &str) -> String {
    format!("{screen}::output::{device}")
}

pub fn expand(configs: &Value) -> Result<Value, String> {
    let mut expanded = configs
        .as_object()
        .ok_or("Screen configuration must be an object.")?
        .clone();
    for (id, config) in configs.as_object().into_iter().flatten() {
        if config.get("outputLayout").is_none_or(Value::is_null) {
            continue;
        }
        let layout = OutputLayout::from_screen(id, config)?;
        let canvas = serde_json::to_value(layout.canvas()?).map_err(|error| error.to_string())?;
        if config
            .get("canvasSize")
            .is_some_and(|value| !value.is_null() && *value != canvas)
        {
            return Err("A logical screen canvas must match its device layout.".into());
        }
        if config.get("canvasView").is_some_and(|v| !v.is_null())
            || config.get("keyFill").is_some_and(|v| !v.is_null())
            || config
                .get("projectorCorrection")
                .is_some_and(|v| !v.is_null())
        {
            return Err("A device layout cannot also use independent canvas, key/fill or projector routing.".into());
        }
        let mut source = config.clone();
        let source_fields = source
            .as_object_mut()
            .ok_or("Screen configuration must be an object.")?;
        source_fields.remove("outputLayout");
        source_fields.remove("outputFormat");
        source["targetType"] = json!("virtual");
        source["canvasSize"] = json!(layout.canvas()?);
        source["screenColor"] = json!(layout.screen_color);
        expanded.insert(id.clone(), source);
        for (index, device) in layout.outputs.iter().enumerate() {
            let device_id = physical_id(id, &device.id);
            if expanded.contains_key(&device_id) {
                return Err("A physical output ID conflicts with an existing screen.".into());
            }
            let mut physical = device.target.config(config, &device.label);
            physical["canvasView"] = json!({"sourceScreenId": id, "region": layout.region(index)});
            let mut correction = neutral_correction();
            correction.color = Some(device.color);
            correction.corners = device.corner_pin.unwrap_or_else(neutral_corners);
            let mut curves = [None; 4];
            if layout.mode == LayoutMode::EdgeBlend {
                if index > 0 {
                    correction.edges.left =
                        layout.blends[index - 1].width as f64 / layout.tile_width as f64;
                    curves[0] = Some(layout.blends[index - 1].right);
                }
                if let Some(seam) = layout.blends.get(index) {
                    correction.edges.right = seam.width as f64 / layout.tile_width as f64;
                    curves[1] = Some(seam.left);
                }
            }
            correction.edge_curves = Some(curves);
            if device.corner_pin.is_some()
                || device.color != OutputColor::default()
                || curves.iter().any(Option::is_some)
            {
                physical["projectorCorrection"] = json!(correction);
            }
            if let Some(alpha) = &device.alpha {
                physical["ndiAlphaMode"] = json!(match alpha.mode {
                    crate::key_fill::FillMode::Straight => "straight",
                    crate::key_fill::FillMode::Premultiplied => "premultiplied",
                });
                if alpha.target.kind != "placeholder" && device.target.kind != "placeholder" {
                    let key_id = format!("{device_id}::key");
                    physical["keyFill"] = json!({"keyScreenId": key_id, "fillMode": alpha.mode});
                    if expanded
                        .insert(key_id, alpha.target.config(config, &device.label))
                        .is_some()
                    {
                        return Err(
                            "A physical key output ID conflicts with an existing screen.".into(),
                        );
                    }
                }
            }
            expanded.insert(device_id, physical);
        }
    }
    Ok(Value::Object(expanded))
}

fn range(value: f64, min: f64, max: f64, name: &str) -> Result<(), String> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!("{name} must be between {min} and {max}."))
    }
}
