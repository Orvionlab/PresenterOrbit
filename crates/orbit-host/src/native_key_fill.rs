use crate::{
    native_compositor::{NativeFrame, NativeLayerBounds, NativeLayerFrame, NativeLayeredFrame},
    AppState, WindowedOutput,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct DisplayPairs {
    pairs: HashMap<String, DisplayPair>,
    sequence: u64,
}

struct DisplayPair {
    identity: String,
    delay: crate::key_fill::PairDelay,
}

pub struct DisplaySetup {
    app: Host,
    pub members: HashSet<String>,
    sources: Vec<String>,
    complete: bool,
}
use crate::Host;

impl DisplaySetup {
    pub fn begin(app: &Host, config: &Value) -> Result<Self, String> {
        let mut setup = Self {
            app: app.clone(),
            members: HashSet::new(),
            sources: Vec::new(),
            complete: false,
        };
        for (id, screen) in config
            .as_object()
            .ok_or("Screen configuration must be an object.")?
        {
            if screen.get("targetType").and_then(Value::as_str) == Some("display")
                && screen.get("enabled").and_then(Value::as_bool) == Some(true)
            {
                if let Some(pair) = crate::key_fill::parse_optional(screen.get("keyFill"))? {
                    setup.members.insert(id.clone());
                    setup.members.insert(pair.key_screen_id);
                    setup.sources.push(id.clone());
                }
            }
        }
        for id in &setup.members {
            if let Some(window) = app.get_window(&crate::display_window_label(id)) {
                window.hide().map_err(|error| error.to_string())?;
            }
        }
        Ok(setup)
    }

    pub fn finish(&mut self, state: &AppState) -> Result<(), String> {
        {
            let outputs = state
                .display_outputs
                .lock()
                .map_err(|_| "Display output state is unavailable.")?;
            let surfaces = state
                .native_output_surfaces
                .lock()
                .map_err(|_| "Native surface state is unavailable.")?;
            if self.members.iter().any(|id| {
                !outputs.get(id).is_some_and(|output| {
                    output.active && surfaces.contains_key(&output.window_label)
                })
            }) {
                return Err("Key/fill display setup is incomplete.".into());
            }
        }
        for id in &self.sources {
            let output = state
                .display_outputs
                .lock()
                .map_err(|_| "Display output state is unavailable.")?
                .get(id)
                .cloned()
                .ok_or("Key/fill display setup is incomplete.")?;
            present(state, &output, None)?;
        }
        self.complete = true;
        Ok(())
    }
}

impl Drop for DisplaySetup {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        let state = self.app.state::<AppState>();
        if let Ok(mut outputs) = state.display_outputs.lock() {
            for id in &self.members {
                if let Some(output) = outputs.get_mut(id) {
                    output.active = false;
                }
            }
        }
        for id in &self.members {
            if let Some(window) = self.app.get_window(&crate::display_window_label(id)) {
                let _ = window.hide();
            }
        }
    }
}

impl DisplayPairs {
    pub fn reconcile_scenes(&mut self, scenes: &HashMap<String, Value>) {
        self.pairs.retain(|source, pair| {
            scenes.get(source).is_some_and(|scene| {
                scene
                    .get("keyFillPartnerId")
                    .and_then(Value::as_str)
                    .zip(scene.get("keyFillSignal").and_then(Value::as_str))
                    .is_some_and(|(partner, signal)| {
                        pair.identity == format!("{source}:{partner}:{signal}")
                    })
            })
        });
    }

    pub fn reconcile_config(&mut self, config: &Value) -> Result<(), String> {
        let screens = config
            .as_object()
            .ok_or("Screen configuration must be an object.")?;
        let mut active = HashSet::new();
        for (source, screen) in screens {
            if screen.get("enabled").and_then(Value::as_bool) == Some(true)
                && screen.get("targetType").and_then(Value::as_str) == Some("display")
            {
                if let Some(pair) = crate::key_fill::parse_optional(screen.get("keyFill"))? {
                    let signal = match pair.fill_mode {
                        crate::key_fill::FillMode::Straight => "fill-straight",
                        crate::key_fill::FillMode::Premultiplied => "fill-premultiplied",
                    };
                    active.insert(format!("{source}:{}:{signal}", pair.key_screen_id));
                }
            }
        }
        self.pairs.retain(|_, pair| active.contains(&pair.identity));
        Ok(())
    }
}

pub fn claim(
    state: &AppState,
    output: &WindowedOutput,
    visited: &mut HashSet<String>,
) -> Result<bool, String> {
    if output.target_id.is_none() {
        return Ok(true);
    }
    let content = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.")?;
    Ok(claim_scene(content.get(&output.source_screen_id), visited))
}

fn claim_scene(scene: Option<&Value>, visited: &mut HashSet<String>) -> bool {
    match scene
        .and_then(|scene| scene.get("keyFillSourceId"))
        .and_then(Value::as_str)
    {
        Some(source) => visited.insert(source.to_owned()),
        None => true,
    }
}

pub fn present(
    state: &AppState,
    output: &WindowedOutput,
    generation: Option<u64>,
) -> Result<bool, String> {
    if output.target_id.is_none() {
        return Ok(false);
    }
    let (source_id, partner_id, source_content, live_pairs) = {
        let scenes = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.")?;
        let Some(source_id) = scenes
            .get(&output.source_screen_id)
            .and_then(|scene| scene.get("keyFillSourceId"))
            .and_then(Value::as_str)
        else {
            return Ok(false);
        };
        let source = scenes
            .get(source_id)
            .ok_or("Key/fill source scene is unavailable.")?;
        let partner = source
            .get("keyFillPartnerId")
            .and_then(Value::as_str)
            .ok_or("Key/fill partner is unavailable.")?;
        (
            source_id.to_owned(),
            partner.to_owned(),
            source.clone(),
            scenes
                .values()
                .filter_map(|scene| {
                    scene
                        .get("keyFillSourceId")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .collect::<HashSet<_>>(),
        )
    };
    let outputs = state
        .display_outputs
        .lock()
        .map_err(|_| "Display output state is unavailable.")?;
    let fill = outputs
        .values()
        .find(|candidate| candidate.active && candidate.source_screen_id == source_id)
        .cloned();
    let key = outputs
        .values()
        .find(|candidate| candidate.active && candidate.source_screen_id == partner_id)
        .cloned();
    drop(outputs);
    // Window creation is sequential. Publish neither member until both surfaces are ready.
    let (Some(fill), Some(key)) = (fill, key) else {
        let existed = state
            .native_key_fill_pairs
            .lock()
            .map_err(|_| "Native key/fill state is unavailable.")?
            .pairs
            .remove(&source_id)
            .is_some();
        if existed {
            let windows = state
                .native_output_surfaces
                .lock()
                .map_err(|_| "Native surface state is unavailable.")?
                .values()
                .map(crate::native_output::NativeOutputSurface::coordinated_window)
                .filter(|window| {
                    window.label() == crate::display_window_label(&source_id)
                        || window.label() == crate::display_window_label(&partner_id)
                })
                .collect::<Vec<_>>();
            for window in windows {
                let _ = window.hide();
            }
            return Err("An active key/fill display partner disappeared.".into());
        }
        return Ok(true);
    };
    if fill.bounds == key.bounds {
        return Err("Key/fill requires distinct resolved display targets.".into());
    }
    let (dimensions, video_decode_size) = {
        let surfaces = state
            .native_output_surfaces
            .lock()
            .map_err(|_| "Native surface state is unavailable.")?;
        let fill_size = surfaces
            .get(&fill.window_label)
            .ok_or("Fill surface is unavailable.")?
            .dimensions();
        let key_size = surfaces
            .get(&key.window_label)
            .ok_or("Key surface is unavailable.")?
            .dimensions();
        if fill_size != key_size {
            return Err("Key/fill display surfaces must have matching dimensions.".into());
        }
        let largest = surfaces
            .values()
            .map(crate::native_output::NativeOutputSurface::dimensions)
            .fold(fill_size, |size, next| {
                (size.0.max(next.0), size.1.max(next.1))
            });
        (fill_size, largest)
    };
    let is_current = || {
        generation.is_none_or(|generation| {
            state.output_render_requested.load(Ordering::Acquire) == generation
        })
    };
    if !is_current() {
        return Ok(true);
    }
    let content = crate::canonical_output_content(state, &source_id, source_content)?;
    let delay = crate::output_delay::frames(content.get("videoDelay"))?;
    let captured = Instant::now();
    let frames = {
        let mut compositor = state
            .native_compositor
            .lock()
            .map_err(|_| "Native compositor is unavailable.")?;
        compositor.set_video_max_size(Some(video_decode_size));
        compositor.try_render_key_fill_pair_for(
            &source_id,
            &content,
            dimensions.0 as usize,
            dimensions.1 as usize,
        )?
    };
    if !is_current() {
        return Ok(true);
    }
    let mut pairs = state
        .native_key_fill_pairs
        .lock()
        .map_err(|_| "Native key/fill state is unavailable.")?;
    pairs.pairs.retain(|source, _| live_pairs.contains(source));
    let identity = format!(
        "{source_id}:{partner_id}:{}",
        content
            .get("keyFillSignal")
            .and_then(Value::as_str)
            .ok_or("Fill signal is unavailable.")?
    );
    let pair = pairs
        .pairs
        .entry(source_id.clone())
        .or_insert_with(|| DisplayPair {
            identity: identity.clone(),
            delay: Default::default(),
        });
    if pair.identity != identity {
        pair.identity = identity.clone();
        pair.delay.clear();
    }
    let delayed = pair.delay.push(
        (frames.0.flatten(), frames.1.flatten()),
        delay,
        Duration::from_secs_f64(1.0 / 60.0),
        captured,
    )?;
    pairs.sequence = pairs.sequence.saturating_add(1);
    let sequence = pairs.sequence;
    let fill_frame = signal_layer(delayed.0, &format!("{identity}:fill"), sequence);
    let key_frame = signal_layer(delayed.1, &format!("{identity}:key"), sequence);
    if !is_current() {
        pairs.pairs.remove(&source_id);
        return Ok(true);
    }
    let mut surfaces = state
        .native_output_surfaces
        .lock()
        .map_err(|_| "Native surface state is unavailable.")?;
    let windows = [
        surfaces
            .get(&fill.window_label)
            .ok_or("Fill surface is unavailable.")?
            .coordinated_window(),
        surfaces
            .get(&key.window_label)
            .ok_or("Key surface is unavailable.")?
            .coordinated_window(),
    ];
    let submitted = (|| {
        surfaces
            .get_mut(&fill.window_label)
            .ok_or("Fill surface is unavailable.")?
            .present_layered(&fill_frame, 0)?;
        surfaces
            .get_mut(&key.window_label)
            .ok_or("Key surface is unavailable.")?
            .present_layered(&key_frame, 0)?;
        Ok::<_, String>(())
    })();
    drop(surfaces);
    drop(pairs);
    let submitted = submitted.and_then(|_| {
        for window in &windows {
            window.show().map_err(|error| error.to_string())?;
            crate::keep_display_window_on_top(window)?;
        }
        Ok(())
    });
    if let Err(error) = submitted {
        for window in &windows {
            let _ = window.hide();
        }
        return Err(error);
    }
    Ok(true)
}

fn signal_layer(frame: NativeFrame, id: &str, sequence: u64) -> NativeLayeredFrame {
    NativeLayeredFrame {
        width: frame.width,
        height: frame.height,
        dynamic_media: frame.dynamic_media,
        layers: vec![NativeLayerFrame {
            id: id.into(),
            visual_key: sequence,
            bounds: Some(NativeLayerBounds {
                x: 0,
                y: 0,
                width: frame.width,
                height: frame.height,
            }),
            pixels: Arc::new(frame.pixels),
            dynamic_media: frame.dynamic_media,
            replace: true,
        }],
    }
}
