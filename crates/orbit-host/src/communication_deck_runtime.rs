use crate::Host;
use crate::{
    communication_deck::*,
    communication_input::{DecoderFactory, ExchangeFactory, Reply},
    AppState,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Instant};

pub(crate) fn factories(
    app: &Host,
    path: &str,
    device: &Value,
    name: &str,
) -> Result<(DecoderFactory, ExchangeFactory), String> {
    let protocol = Protocol::from_name(name)?;
    let rate = FrameRate {
        numerator: device["input"]["frameRate"]["numerator"]
            .as_u64()
            .ok_or("Frame rate is required.")? as u32,
        denominator: device["input"]["frameRate"]["denominator"]
            .as_u64()
            .ok_or("Frame rate denominator is required.")? as u32,
    };
    let drop = device["input"]["dropFrame"]
        .as_bool()
        .ok_or("Drop frame configuration is required.")?;
    let mut initial = ProtocolContext::new(rate, drop);
    if name == "vdcp" {
        initial = initial.with_virtual_port_id(
            device["input"]["virtualPortId"]
                .as_i64()
                .ok_or("VDCP port is required.")? as i8,
        )?;
        initial = initial.with_unit_address(
            device["input"]["unitAddress"]
                .as_u64()
                .ok_or("VDCP unit address is required.")? as u8,
        )?;
    }
    let factory: DecoderFactory = Arc::new(move || {
        let mut decoder = FrameDecoder::new(protocol, rate, drop);
        Box::new(move |bytes| {
            if bytes.is_empty() && !decoder.is_empty() {
                return Err("Incomplete deck command timed out.".into());
            }
            decoder.push(bytes)?;
            let mut frames = Vec::new();
            while let Some(frame) = decoder.next()? {
                frames.push(frame);
            }
            Ok(frames)
        })
    });
    let app = app.clone();
    let path = path.to_string();
    let device = device.clone();
    // Validate the selected catalog before opening a socket or serial port.
    {
        let state = app.state::<AppState>();
        let io = crate::lock_workspace_io(&state)?;
        let workspace = crate::workspace::read_workspace(std::path::Path::new(&path))?;
        catalog(&workspace, &device)?;
        if name == "amp" {
            let bins = crate::remote_media::catalog(&workspace)?;
            let folder = bins
                .as_array()
                .and_then(|bins| {
                    bins.iter()
                        .find(|bin| bin["id"] == device["input"]["mediaLibraryId"])
                })
                .and_then(|bin| bin["title"].as_str())
                .ok_or("The AMP media-bin name is unavailable.")?;
            if folder.is_empty()
                || folder.len() > 255
                || folder
                    .bytes()
                    .any(|byte| !byte.is_ascii_graphic() && byte != b' ')
            {
                return Err("AMP media-bin names require 1 to 255 printable ASCII bytes.".into());
            }
            initial.amp_folder = Some(folder.into());
        }
        std::mem::drop(io);
    }
    let exchange: ExchangeFactory = Arc::new(move || {
        let app = app.clone();
        let path = path.clone();
        let device = device.clone();
        let mut context = initial.clone();
        Box::new(move |_, frame| {
            let mut failure = None;
            let bytes = protocol.process(frame, &mut context, &mut |command| {
                let result = execute(&app, &path, &device, command);
                if let Err(error) = &result {
                    failure = Some(error.clone());
                }
                result
            })?;
            Ok(Reply {
                bytes,
                error: failure,
                close: matches!(context.amp_session, Some(AmpSession::Closed)),
            })
        })
    });
    Ok((factory, exchange))
}

fn timing(device: &Value) -> (FrameRate, bool) {
    (
        FrameRate {
            numerator: device["input"]["frameRate"]["numerator"]
                .as_u64()
                .expect("validated frame rate") as u32,
            denominator: device["input"]["frameRate"]["denominator"]
                .as_u64()
                .expect("validated denominator") as u32,
        },
        device["input"]["dropFrame"]
            .as_bool()
            .expect("validated drop frame"),
    )
}
fn timecode(seconds: f64, device: &Value) -> Result<DeckTimecode, String> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err("The native deck position is invalid.".into());
    }
    let (frame_rate, drop_frame) = timing(device);
    Ok(DeckTimecode {
        frame_number: (seconds * f64::from(frame_rate.numerator)
            / f64::from(frame_rate.denominator))
        .floor() as u64,
        frame_rate,
        drop_frame,
    })
}
fn seconds(time: DeckTimecode, device: &Value) -> Result<f64, String> {
    let (rate, drop) = timing(device);
    if time.frame_rate != rate || time.drop_frame != drop {
        return Err("Deck timecode does not match the configured frame rate.".into());
    }
    Ok(time.frame_number as f64 * f64::from(rate.denominator) / f64::from(rate.numerator))
}

/// Sony BVW controls the live clip; VDCP and AMP serve a media bin.
fn controls_live_clip(device: &Value) -> bool {
    device["input"]["protocol"] == "sony-bvw"
}

fn catalog(workspace: &Value, device: &Value) -> Result<Vec<(String, Value)>, String> {
    if controls_live_clip(device) {
        return Ok(Vec::new());
    }
    let bin = device["input"]["mediaLibraryId"]
        .as_str()
        .ok_or("A media bin is required.")?;
    let bins = crate::remote_media::catalog(workspace)?;
    let selected = bins
        .as_array()
        .and_then(|bins| bins.iter().find(|entry| entry["id"] == bin))
        .ok_or("The configured media bin is unavailable.")?;
    let assets = workspace["media"]
        .as_array()
        .ok_or("Workspace media must be an array.")?;
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for item in selected["items"]
        .as_array()
        .ok_or("The media bin must contain items.")?
    {
        let asset = assets
            .iter()
            .find(|asset| asset["id"] == item["id"])
            .ok_or("Catalog media is unavailable.")?;
        if asset["type"] != "video" {
            continue;
        }
        let title = asset["title"]
            .as_str()
            .filter(|title| !title.is_empty())
            .ok_or("Deck clips require nonempty media names.")?;
        if !title.is_ascii()
            || title
                .bytes()
                .any(|byte| !byte.is_ascii_graphic() && byte != b' ')
        {
            return Err("Deck clip names must be printable ASCII.".into());
        }
        if device["input"]["protocol"] == "vdcp"
            && device["input"]["shortNames"] == false
            && title.len() > 79
        {
            return Err("VDCP full clip names must fit the 79-byte catalog entry limit. Enable eight-character names or rename the media.".into());
        }
        if device["input"]["protocol"] == "amp" && title.trim_end_matches(' ') != title {
            return Err("AMP clip names cannot end with padding spaces.".into());
        }
        let limit = if device["input"]["shortNames"] == true {
            8
        } else if device["input"]["protocol"] == "amp" {
            32
        } else {
            255
        };
        let id = if limit == 8 {
            title
                .chars()
                .take(8)
                .collect::<String>()
                .trim_end_matches(' ')
                .to_string()
        } else {
            title.to_string()
        };
        if id.trim().is_empty() || title.len() > 255 || id.len() > limit || !seen.insert(id.clone())
        {
            return Err("Deck clip names must fit the configured length and be unique within the selected bin.".into());
        }
        result.push((id, asset.clone()));
    }
    Ok(result)
}

fn snapshot(state: &AppState) -> Result<Value, String> {
    crate::lock_output_snapshot_state(&state.output_snapshot)?
        .cache
        .as_ref()
        .map(|cache| cache.snapshot.clone())
        .ok_or("Native output has not accepted a snapshot yet.".into())
}

fn status(app: &Host, path: &str, device: &Value) -> Result<DeckState, String> {
    let state = app.state::<AppState>();
    let path = crate::active_workspace(&state, json!(path))?;
    let io = crate::lock_workspace_io(&state)?;
    io.require_active(&path)?;
    let workspace = crate::workspace::read_workspace(&path)?;
    let catalog = catalog(&workspace, device)?;
    let accepted = snapshot(&state)?;
    let mut selected = accepted["backgroundId"]
        .as_str()
        .and_then(|id| {
            workspace["media"]
                .as_array()?
                .iter()
                .find(|asset| asset["id"] == id && asset["type"] == "video")
        })
        .cloned();
    if !controls_live_clip(device)
        && selected
            .as_ref()
            .is_some_and(|asset| !catalog.iter().any(|(_, item)| item["id"] == asset["id"]))
    {
        selected = None;
    }
    let mut clock = Value::Null;
    if let Some(asset) = &selected {
        let ids = crate::main_transport_source_ids(&state)?;
        let content = state
            .output_content
            .lock()
            .map_err(|_| "Output state is unavailable.")?;
        let source = ids.iter().find_map(|id| {
            content
                .get(id)
                .filter(|scene| {
                    scene["backgroundSelectionId"] == accepted["backgroundSelectionId"]
                        && scene["backgroundMediaPath"] == asset["sourcePath"]
                })
                .map(|scene| (id.clone(), scene.clone()))
        });
        drop(content);
        if let Some((id, scene)) = source {
            let scene = state
                .output_playback
                .lock()
                .map_err(|_| "Output playback is unavailable.")?
                .canonical_content(&id, &scene, Instant::now());
            clock = scene["backgroundPlaybackState"].clone();
        }
    }
    let transport = match clock["status"].as_str() {
        Some("playing") => {
            let rate = clock["rate"]
                .as_f64()
                .ok_or("The native playback rate is unavailable.")?;
            if rate == 1.0 {
                DeckTransport::Playing
            } else if rate == 4.0 {
                DeckTransport::FastForward
            } else if rate == -4.0 {
                DeckTransport::Rewind
            } else {
                DeckTransport::Variable {
                    speed_q16: (rate * 65536.0).round() as i32,
                }
            }
        }
        Some("paused") => {
            if clock["deckStopped"] == true {
                DeckTransport::Stopped
            } else {
                DeckTransport::Paused
            }
        }
        Some("ended") => DeckTransport::Stopped,
        _ => {
            if selected.is_some() {
                DeckTransport::Unknown
            } else {
                DeckTransport::Stopped
            }
        }
    };
    let selected_clip_id = selected.as_ref().and_then(|asset| {
        catalog
            .iter()
            .find(|(_, item)| item["id"] == asset["id"])
            .map(|(id, _)| id.clone())
            .or_else(|| asset["title"].as_str().map(str::to_string))
    });
    Ok(DeckState {
        transport,
        position: clock["currentTime"]
            .as_f64()
            .map(|value| timecode(value, device))
            .transpose()?,
        duration: clock["duration"]
            .as_f64()
            .map(|value| timecode(value, device))
            .transpose()?,
        selected_clip_id,
        catalog: catalog
            .iter()
            .map(|(id, asset)| DeckClip {
                id: id.clone(),
                title: asset["title"].as_str().map(str::to_string),
                duration: None,
                source_start: None,
            })
            .collect(),
    })
}

pub(crate) fn execute(
    app: &Host,
    path: &str,
    device: &Value,
    command: DeckCommand,
) -> Result<DeckState, String> {
    if let DeckCommand::ClipInfo { id } = &command {
        let state = app.state::<AppState>();
        let path = crate::active_workspace(&state, json!(path))?;
        let io = crate::lock_workspace_io(&state)?;
        io.require_active(&path)?;
        let workspace = crate::workspace::read_workspace(&path)?;
        let asset = catalog(&workspace, device)?
            .into_iter()
            .find(|(name, _)| name == id)
            .map(|(_, asset)| asset)
            .ok_or("The requested clip is unavailable.")?;
        let metadata = crate::media_metadata::probe(&asset["sourcePath"])?;
        let duration = metadata["durationSeconds"]
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or("The clip has no measured duration.")?;
        drop(io);
        let mut result = status(
            app,
            path.to_str().ok_or("Workspace path must be Unicode.")?,
            device,
        )?;
        let clip = result
            .catalog
            .iter_mut()
            .find(|clip| clip.id == *id)
            .ok_or("The clip catalog changed.")?;
        clip.duration = Some(timecode(duration, device)?);
        clip.source_start = Some(timecode(0.0, device)?);
        return Ok(result);
    }
    match command {
        DeckCommand::ClipInfo { .. } => unreachable!("clip metadata was handled above"),
        DeckCommand::Catalog { .. } | DeckCommand::Status => {}
        DeckCommand::CueClip {
            id,
            position,
            duration,
        } => cue(app, path, device, &id, position, duration)?,
        DeckCommand::Seek { position } => seek(app, path, device, seconds(position, device)?)?,
        DeckCommand::Stop => rate(app, path, device, 0.0, true)?,
        DeckCommand::Pause => rate(app, path, device, 0.0, false)?,
        DeckCommand::Play => rate(app, path, device, 1.0, false)?,
        DeckCommand::FastForward => rate(app, path, device, 4.0, false)?,
        DeckCommand::Rewind => rate(app, path, device, -4.0, false)?,
        DeckCommand::VariablePlay { speed_q16 } => {
            rate(app, path, device, f64::from(speed_q16) / 65536.0, false)?
        }
    }
    status(app, path, device)
}

fn live_target(app: &Host, path: &str, device: &Value) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let path = crate::active_workspace(&state, json!(path))?;
    let io = crate::lock_workspace_io(&state)?;
    io.require_active(&path)?;
    let workspace = crate::workspace::read_workspace(&path)?;
    let accepted = snapshot(&state)?;
    let asset = workspace["media"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|asset| asset["id"] == accepted["backgroundId"] && asset["type"] == "video")
        })
        .ok_or("No live video clip is selected.")?;
    if !controls_live_clip(device)
        && !catalog(&workspace, device)?
            .iter()
            .any(|(_, item)| item["id"] == asset["id"])
    {
        return Err("The live clip is outside the configured media bin.".into());
    }
    Ok(
        json!({"workspacePath":path,"id":asset["id"],"selectionId":accepted["backgroundSelectionId"]}),
    )
}

fn seek(app: &Host, path: &str, device: &Value, seconds: f64) -> Result<(), String> {
    let mut target = live_target(app, path, device)?;
    target["action"] = json!("seek");
    target["position"] = json!(0);
    target["sourceSeconds"] = json!(seconds);
    let result = crate::remote_playback_commands::execute(app, &app.state::<AppState>(), &target)?;
    require_ok(&result)
}
fn require_ok(result: &Value) -> Result<(), String> {
    if result["ok"] == true {
        Ok(())
    } else {
        Err(result["error"]
            .as_str()
            .unwrap_or("The native deck command failed.")
            .to_string())
    }
}

fn rate(app: &Host, path: &str, device: &Value, rate: f64, stopped: bool) -> Result<(), String> {
    if !rate.is_finite() || !(-4.0..=4.0).contains(&rate) || (rate != 0.0 && rate.abs() < 0.1) {
        return Err("Unsupported deck shuttle rate.".into());
    }
    if status(app, path, device)?.duration.is_none() {
        return Err("The native clip duration is unavailable.".into());
    }
    let mut target = live_target(app, path, device)?;
    target["action"] = json!(if rate == 0.0 { "pause" } else { "play" });
    require_ok(&crate::remote_controls::execute(
        app,
        &app.state::<AppState>(),
        "media",
        &target,
    )?)?;
    let state = app.state::<AppState>();
    let path = crate::active_workspace(&state, json!(path))?;
    let io = crate::lock_workspace_io(&state)?;
    io.require_active(&path)?;
    let mut coordinator = crate::lock_output_snapshot_state(&state.output_snapshot)?;
    let accepted = coordinator
        .cache
        .as_ref()
        .ok_or("Native output is unavailable.")?;
    if accepted.snapshot["backgroundId"] != target["id"]
        || accepted.snapshot["backgroundSelectionId"] != target["selectionId"]
    {
        return Err("The live playback occurrence changed.".into());
    }
    let ids = crate::main_transport_source_ids(&state)?;
    let content = state
        .output_content
        .lock()
        .map_err(|_| "Output state is unavailable.")?;
    let ids = ids
        .into_iter()
        .filter(|id| {
            content
                .get(id)
                .is_some_and(|scene| scene["backgroundSelectionId"] == target["selectionId"])
        })
        .collect::<Vec<_>>();
    drop(content);
    if ids.is_empty() {
        return Err("No native playback target matches the live clip.".into());
    }
    let mut payloads = std::collections::HashMap::new();
    let mut changed = false;
    {
        let mut playback = state
            .output_playback
            .lock()
            .map_err(|_| "Native playback is unavailable.")?;
        let now = Instant::now();
        for id in ids {
            let result = playback.set_deck_transport(
                &id,
                crate::output_playback::PlaybackLayer::Media,
                rate,
                stopped,
                now,
            )?;
            changed |= result.changed;
            payloads.insert(id, result.payload);
        }
    }
    let revision = crate::anchor_native_playback_revision(&mut coordinator, true, changed);
    let command = crate::output_playback::PlaybackCommand {
        layer: crate::output_playback::PlaybackLayer::Media,
        action: if rate == 0.0 {
            crate::output_playback::PlaybackAction::Pause
        } else {
            crate::output_playback::PlaybackAction::Play
        },
        position: 0.0,
    };
    crate::propagate_playback_source_mutation(&state, command, changed, &mut payloads)?;
    crate::mark_native_playback_propagated(&mut coordinator, revision);
    Ok(())
}

fn cue(
    app: &Host,
    path: &str,
    device: &Value,
    id: &str,
    position: Option<DeckTimecode>,
    duration: Option<DeckTimecode>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let path = crate::active_workspace(&state, json!(path))?;
    let io = crate::lock_workspace_io(&state)?;
    io.require_active(&path)?;
    let workspace = crate::workspace::read_workspace(&path)?;
    let asset = catalog(&workspace, device)?
        .into_iter()
        .find(|(name, _)| name == id)
        .map(|(_, asset)| asset)
        .ok_or("The requested clip is unavailable in the configured bin.")?;
    let metadata = crate::media_metadata::probe(&asset["sourcePath"])?;
    let measured = metadata["durationSeconds"]
        .as_f64()
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .ok_or("The clip has no measured duration.")?;
    let start = position
        .map(|value| seconds(value, device))
        .transpose()?
        .unwrap_or(0.0);
    let end = duration
        .map(|value| seconds(value, device).map(|duration| start + duration))
        .transpose()?;
    if end.is_some_and(|end| end <= start) {
        return Err("A deck cue duration must be positive.".into());
    }
    crate::output_playback::validate_media_transport_duration(start, end, measured)?;
    let rss = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.")?
        .snapshot();
    let mut output = crate::lock_output_snapshot_state(&state.output_snapshot)?;
    let mut accepted = output
        .cache
        .as_ref()
        .ok_or("Native output is unavailable.")?
        .snapshot
        .clone();
    let media = accepted["media"]
        .as_array_mut()
        .ok_or("Native media must be an array.")?;
    let accepted_asset = media
        .iter_mut()
        .find(|item| item["id"] == asset["id"] && item["type"] == "video")
        .ok_or("The clip is unavailable in the native snapshot.")?;
    *accepted_asset = asset.clone();
    let action = crate::NativeDueMediaAction {
        target_id: asset["id"]
            .as_str()
            .ok_or("Media identity is required.")?
            .into(),
        playback: "stop".into(),
        end_behavior: "hold".into(),
        transition: "none".into(),
        behavior: asset["backgroundBehavior"].as_str().map(str::to_string),
        scale: asset["backgroundScale"].as_str().map(str::to_string),
        effects: None,
        transport: Some(
            json!({"inSeconds":start,"outSeconds":end,"rate":1.0,"delaySeconds":0.0,"retrigger":"restart"}),
        ),
    };
    let (next, mut patch) = crate::select_media_in_output_snapshot(&accepted, &action)?
        .ok_or("The clip is unavailable in the native snapshot.")?;
    let (mut next, paused) = crate::set_transport_paused_in_output_snapshot(
        &next,
        crate::output_playback::PlaybackLayer::Media,
        true,
    )?;
    next["backgroundPlaybackState"] = json!({"selectionId":next["backgroundSelectionId"],"status":"paused","currentTime":start,"duration":measured});
    patch["backgroundPlaybackState"] = next["backgroundPlaybackState"].clone();
    patch["transportPlaybackPaused"] = paused;
    let (revision, pending) =
        crate::commit_native_output_snapshot(&state, &mut output, next, &rss, None, None)?;
    drop(output);
    crate::schedule_output_rss_refreshes(app, pending);
    let _ = app.emit_to(
        "main",
        "native-output-state-patch",
        json!({"source":"remote","revision":revision,"patch":patch}),
    );
    Ok(())
}
