use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ndi_source: Option<crate::ndi_input::NdiSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<[u16; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing: Option<AudioInputRouting>,
    pub level_db: f32,
    pub delay_ms: u32,
    pub latency_ms: u32,
}

pub(crate) const MAX_AUDIO_INPUT_BUSES: usize = 16;
pub(crate) const MAX_AUDIO_SOURCE_CHANNELS: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioInputRouting {
    pub source_channels: u16,
    pub bus_channels: u16,
    pub routes: Vec<AudioInputRoute>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioInputRoute {
    pub input: u16,
    pub bus: u16,
    pub level_db: f32,
    pub muted: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InputRoutingPlan {
    pub source_channels: usize,
    pub bus_channels: usize,
    routes: Vec<AudioInputRoutePlan>,
}

#[derive(Clone, Debug, PartialEq)]
struct AudioInputRoutePlan {
    input: usize,
    bus: usize,
    gain: f32,
    muted: bool,
}

impl InputRoutingPlan {
    pub(crate) fn map_channel_frame(
        &self,
        sample: impl Fn(usize) -> f32,
        output: &mut [f32; MAX_AUDIO_INPUT_BUSES],
    ) {
        output[..self.bus_channels].fill(0.0);
        for route in &self.routes {
            if !route.muted {
                output[route.bus] += sample(route.input) * route.gain;
            }
        }
    }
}

impl CaptureConfig {
    pub fn validate(&self) -> Result<(), String> {
        match (&self.device_name, &self.ndi_source) {
            (Some(name), None) if !name.trim().is_empty() => {}
            (None, Some(source)) => crate::ndi_input::validate_source(source)?,
            _ => return Err("Select exactly one local device or NDI audio source.".into()),
        }
        match (&self.channels, &self.routing) {
            (Some(channels), None)
                if channels
                    .iter()
                    .all(|channel| (1..=MAX_AUDIO_SOURCE_CHANNELS as u16).contains(channel)) => {}
            (None, Some(routing)) => routing.validate()?,
            (Some(_), Some(_)) => {
                return Err("Audio input must use either stereo channels or bus routing.".into())
            }
            _ => return Err("Audio input requires stereo channels or bus routing.".into()),
        }
        if !self.level_db.is_finite()
            || !(-60.0..=6.0).contains(&self.level_db)
            || self.delay_ms > 2000
            || !(10..=200).contains(&self.latency_ms)
        {
            return Err("Audio input level, delay or latency is out of range.".into());
        }
        Ok(())
    }

    pub(crate) fn routing_plan(&self) -> InputRoutingPlan {
        if let Some(routing) = &self.routing {
            return routing.plan();
        }
        let channels = self.channels.expect("validated stereo channel mapping");
        InputRoutingPlan {
            source_channels: channels.iter().copied().max().unwrap_or(1) as usize,
            bus_channels: 2,
            routes: channels
                .into_iter()
                .enumerate()
                .map(|(bus, input)| AudioInputRoutePlan {
                    input: input as usize - 1,
                    bus,
                    gain: 1.0,
                    muted: false,
                })
                .collect(),
        }
    }
}

impl AudioInputRouting {
    fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_AUDIO_SOURCE_CHANNELS as u16).contains(&self.source_channels)
            || !(1..=MAX_AUDIO_INPUT_BUSES as u16).contains(&self.bus_channels)
            || self.routes.len() > MAX_AUDIO_SOURCE_CHANNELS * MAX_AUDIO_INPUT_BUSES
        {
            return Err(
                "Audio input routing channel counts or route count are out of range.".into(),
            );
        }
        let mut pairs = std::collections::HashSet::new();
        for route in &self.routes {
            if !(1..=self.source_channels).contains(&route.input)
                || !(1..=self.bus_channels).contains(&route.bus)
                || !route.level_db.is_finite()
                || !(-60.0..=6.0).contains(&route.level_db)
                || !pairs.insert((route.input, route.bus))
            {
                return Err("Audio input channel routes must be unique and within range.".into());
            }
        }
        Ok(())
    }

    pub(crate) fn plan(&self) -> InputRoutingPlan {
        InputRoutingPlan {
            source_channels: self.source_channels as usize,
            bus_channels: self.bus_channels as usize,
            routes: self
                .routes
                .iter()
                .map(|route| AudioInputRoutePlan {
                    input: route.input as usize - 1,
                    bus: route.bus as usize - 1,
                    gain: 10.0_f32.powf(route.level_db / 20.0),
                    muted: route.muted,
                })
                .collect(),
        }
    }
}

pub fn validate_collection(value: &Value) -> Result<(), String> {
    let inputs = value.as_array().ok_or("audioInputs must be an array.")?;
    if inputs.len() > 16 {
        return Err("At most 16 audio inputs are supported.".into());
    }
    let mut ids = std::collections::HashSet::new();
    for input in inputs {
        let id = input["id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .ok_or("Audio input ID is required.")?;
        if !ids.insert(id)
            || !input["title"]
                .as_str()
                .is_some_and(|title| !title.trim().is_empty())
        {
            return Err("Audio input names and unique IDs are required.".into());
        }
        if (input.get("capture").is_some() || input.get("behaviorMode").is_some())
            && !matches!(
                input["behaviorMode"].as_str(),
                Some("off" | "on" | "auto-on" | "auto-off")
            )
        {
            return Err("Audio input mode must be off, on, auto-on or auto-off.".into());
        }
        if let Some(capture) = input.get("capture") {
            serde_json::from_value::<CaptureConfig>(capture.clone())
                .map_err(|error| error.to_string())?
                .validate()?;
            let links = input["linkedVideoInputIds"]
                .as_array()
                .ok_or("Audio input video links must be an array.")?;
            if links
                .iter()
                .any(|link| !link.as_str().is_some_and(|id| !id.is_empty()))
            {
                return Err("Audio input video links must contain IDs.".into());
            }
        }
    }
    Ok(())
}

pub fn enabled(input: &Value, content: &Value) -> bool {
    match input["behaviorMode"].as_str() {
        Some("on") => true,
        Some("auto-off") => content["foregroundAudioActive"] != true,
        Some("auto-on") => content["audioInputContext"]["videoInputId"]
            .as_str()
            .is_some_and(|id| {
                input["linkedVideoInputIds"]
                    .as_array()
                    .is_some_and(|links| links.iter().any(|link| link.as_str() == Some(id)))
            }),
        _ => false,
    }
}
