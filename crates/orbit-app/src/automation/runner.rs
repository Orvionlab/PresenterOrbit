//! The action runner: `runActions` and the handlers of `features/app/automation-action-*.ts`.
//!
//! The runner is a pure interpreter over the output snapshot (the live state) and the workspace facts. It never
//! calls the host: whatever has to happen outside the snapshot is a `Step`, and the driver in `execute.rs`
//! performs steps in order, once. A cue is a step of its own because the host takes the output lock to run it.

use std::collections::VecDeque;

use serde_json::{json, Value};

use crate::collections::model::Items;

/// `runActions` stops expanding macros past this depth.
pub const MAX_DEPTH: u32 = 8;

/// The workspace content an action reads.
#[derive(Clone)]
pub struct Facts {
    pub workspace: std::sync::Arc<Value>,
    pub items: Items,
    pub libraries: Vec<crate::collections::model::Library>,
    pub workspace_path: String,
    /// Allocates the identity of a message that goes on air.
    pub new_id: fn() -> String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// A presentation or playlist cue, executed by the host cue command.
    Cue {
        command: &'static str,
        payload: Value,
    },
    /// Actions whose own delay has not passed yet, or that wait for a native timer.
    Delay {
        actions: Vec<Value>,
        seconds: f64,
        depth: u32,
    },
    Timer {
        timer: Value,
        command: String,
        increment: f64,
        configuration: Option<Value>,
    },
    Communication {
        device: Value,
        payload: String,
        details: Value,
    },
    Capture {
        action: Value,
    },
    ExternalPresentation {
        url: String,
    },
    AudioInput {
        id: String,
        overrides: Value,
    },
    /// Markers an action carries for the media it just put on air, scheduled once the batch has set the selection.
    Markers {
        layer: &'static str,
        markers: Value,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub action_id: String,
    pub kind: String,
    pub message: String,
}

#[derive(Default)]
pub struct Run {
    pub steps: Vec<Step>,
    pub failures: Vec<Failure>,
}

/// What is left to do after a batch stopped at a cue.
pub type Queue = VecDeque<(u32, Value)>;

pub fn queue(actions: &[Value], depth: u32) -> Queue {
    actions
        .iter()
        .cloned()
        .map(|action| (depth, action))
        .collect()
}

/// Runs queued actions against the live state until the queue is empty or a cue has to run first. The cue is
/// the last entry of `run.steps` in that case. Returns whether the queue is empty.
pub fn run_batch(live: &mut Value, facts: &Facts, pending: &mut Queue, run: &mut Run) -> bool {
    while let Some((depth, action)) = pending.pop_front() {
        if depth > MAX_DEPTH {
            if action["enabled"] != false {
                fail(
                    run,
                    &action,
                    "Automation nesting depth limit of 8 was exceeded.",
                );
            }
            continue;
        }
        if action["enabled"] == false {
            continue;
        }
        if !marker_guard_holds(live, facts, &action) {
            continue;
        }
        let delay = action["delaySeconds"].as_f64().unwrap_or(0.0);
        if delay > 0.0 {
            let mut now = action.clone();
            now["delaySeconds"] = json!(0);
            run.steps.push(Step::Delay {
                actions: vec![now],
                seconds: delay,
                depth,
            });
            continue;
        }
        let before = run.steps.len();
        let mut nested = Vec::new();
        if let Err(message) = handle(live, facts, &action, depth, run, &mut nested) {
            fail(run, &action, &message);
        }
        let marker = marker_details(&action);
        for entry in nested.into_iter().rev() {
            let mut entry = entry;
            if let Some(marker) = &marker {
                for key in MARKER_KEYS {
                    entry.1["details"][key] = marker[key].clone();
                }
            }
            pending.push_front(entry);
        }
        if matches!(
            run.steps.get(before),
            Some(Step::Cue { .. } | Step::Capture { .. })
        ) {
            return false;
        }
    }
    true
}

fn fail(run: &mut Run, action: &Value, message: &str) {
    run.failures.push(Failure {
        action_id: action["id"].as_str().unwrap_or_default().into(),
        kind: action["type"].as_str().unwrap_or_default().into(),
        message: message.into(),
    });
}

const MARKER_KEYS: [&str; 4] = [
    "expectedMarkerMediaId",
    "expectedMarkerLayer",
    "expectedMarkerWorkspacePath",
    "expectedMarkerSelectionId",
];

fn marker_details(action: &Value) -> Option<Value> {
    action["details"]
        .get("expectedMarkerSelectionId")
        .is_some()
        .then(|| action["details"].clone())
}

/// An action carrying a marker identity runs only while that exact media selection is still on air in the
/// same workspace; a stale marker is skipped, never run late.
pub fn marker_guard_holds(live: &Value, facts: &Facts, action: &Value) -> bool {
    let Some(details) = marker_details(action) else {
        return true;
    };
    let music = details["expectedMarkerLayer"] == "music";
    let (id, selection) = if music {
        (&live["audioId"], &live["audioSelectionId"])
    } else {
        (&live["backgroundId"], &live["backgroundSelectionId"])
    };
    details["expectedMarkerWorkspacePath"] == facts.workspace_path.as_str()
        && facts.workspace["media"].as_array().is_some_and(|media| {
            media
                .iter()
                .any(|asset| asset["id"] == details["expectedMarkerMediaId"])
        })
        && *id == details["expectedMarkerMediaId"]
        && *selection == details["expectedMarkerSelectionId"]
}

fn handle(
    live: &mut Value,
    facts: &Facts,
    action: &Value,
    depth: u32,
    run: &mut Run,
    nested: &mut Vec<(u32, Value)>,
) -> Result<(), String> {
    let target = action["targetId"].as_str().unwrap_or("");
    match action["type"].as_str().unwrap_or("") {
        "video-input" => {
            if target.is_empty() {
                return Err("videoInputs.missing".into());
            }
            if !facts.workspace["videoInputs"]
                .as_array()
                .is_some_and(|inputs| inputs.iter().any(|input| input["id"] == target))
            {
                return Err("videoInputs.missing".into());
            }
            live["liveVideoInputId"] = json!(target);
        }
        "clear" => clear(live, action),
        "clear-group" => clear_group(live, action)?,
        "macro" => {
            let macros = facts.workspace["macros"].as_array();
            let actions = macros
                .and_then(|macros| macros.iter().find(|entry| entry["id"] == target))
                .and_then(|entry| entry["actions"].as_array())
                .cloned()
                .unwrap_or_default();
            nested.extend(actions.into_iter().map(|action| (depth + 1, action)));
        }
        "presentation" => presentation(live, facts, action, run)?,
        "playlist" => playlist(live, facts, action, depth, run, nested)?,
        "presentation-slide" => {
            if action["details"]["slide"].is_object() {
                live["activeActionSlide"] = action["details"]["slide"].clone();
                for key in [
                    "liveItemId",
                    "liveSlideId",
                    "liveCueInstanceId",
                    "livePlaylistId",
                ] {
                    live[key] = Value::Null;
                }
                live["liveBuildStepStartedAtMsByStep"] = json!({});
                show_look_layer(live, "slide");
            }
        }
        "slide-destination" => {
            let destination = action["value"].as_str().unwrap_or("");
            if !destination.contains("NO_CHANGE") {
                let stage_only = destination.contains("STAGE_ONLY");
                for_each_screen(live, |screen| {
                    let is_stage = screen["type"] == "stage";
                    screen["lookLayers"]["slide"] = json!(!stage_only || is_stage);
                });
            }
        }
        "mask" => {
            if !target.is_empty() {
                for_each_screen(live, |screen| {
                    screen["lookThemes"]["maskId"] = json!(target)
                });
            }
        }
        "audience-look" => audience_look(live, facts, target)?,
        "media" => media(live, facts, action, run)?,
        "prop" => prop(live, action, run),
        "stage" => stage(live, action),
        "message" => message(live, facts, action),
        "audio-input" => audio_input(facts, action, run)?,
        "external-presentation" => {
            let details = &action["details"];
            let url = details["url"]["absoluteString"]
                .as_str()
                .or_else(|| details["url"].as_str())
                .unwrap_or("");
            run.steps
                .push(Step::ExternalPresentation { url: url.into() });
        }
        "capture" => run.steps.push(Step::Capture {
            action: action.clone(),
        }),
        "communication" => communication(facts, action, run),
        "transport" => transport(live, action),
        "timer" => timer(facts, action, run),
        other => return Err(format!("Unknown action type {other:?}.")),
    }
    Ok(())
}

fn for_each_screen(live: &mut Value, mut edit: impl FnMut(&mut Value)) {
    if let Some(screens) = live["screenConfig"].as_object_mut() {
        for screen in screens.values_mut() {
            edit(screen);
        }
    }
}

pub fn show_look_layer(live: &mut Value, layer: &str) {
    for_each_screen(live, |screen| screen["lookLayers"][layer] = json!(true));
}

fn clear_slide(live: &mut Value) {
    crate::cues::actions::clear_patch(live, "slide");
    live["activeActionSlide"] = Value::Null;
    // The host refuses a build clock that disagrees with the timeline start, so they end together.
    live["timelineStartedAtMs"] = Value::Null;
    live["timelineTransport"] = Value::Null;
    live["liveBuildStepStartedAtMsByStep"] = json!({});
}

fn clear_message(live: &mut Value) {
    live["activeMessageId"] = Value::Null;
    live["activeMessageInstanceId"] = Value::Null;
    live["activeMessageText"] = json!("");
}

fn clear(live: &mut Value, action: &Value) {
    let layer = action["targetLayer"]
        .as_str()
        .filter(|layer| !layer.is_empty())
        .unwrap_or("all");
    if matches!(layer, "all" | "slide") {
        live["presentationNextAnchor"] = Value::Null;
    }
    match layer {
        "all" => {
            clear_slide(live);
            live["liveVideoInputId"] = Value::Null;
            live["livePropIds"] = json!([]);
            live["audioId"] = Value::Null;
            clear_message(live);
        }
        "prop" => live["livePropIds"] = json!([]),
        "video-input" => live["liveVideoInputId"] = Value::Null,
        "audio" => live["audioId"] = Value::Null,
        "background" => live["backgroundId"] = Value::Null,
        "announcements" => live["announcements"] = Value::Null,
        "messages" => {
            // A timed clear of one message must not clear the message that replaced it.
            let expected = &action["details"]["expectedMessageInstanceId"];
            if expected.is_string() && live["activeMessageInstanceId"] != *expected {
                return;
            }
            clear_message(live);
        }
        "slide" => clear_slide(live),
        _ => {}
    }
}

fn clear_group(live: &mut Value, action: &Value) -> Result<(), String> {
    let id = action["targetId"].as_str().unwrap_or("");
    if id.is_empty() {
        return Ok(());
    }
    let group = live["clearGroups"]
        .as_array()
        .and_then(|groups| groups.iter().find(|group| group["id"] == id))
        .cloned();
    match group {
        Some(group) => crate::cues::actions::clear_group_patch(live, &group),
        None => {
            if live["liveSlide"]["sourceGroupId"] == id {
                for key in [
                    "liveItemId",
                    "liveSlideId",
                    "liveCueInstanceId",
                    "livePlaylistId",
                ] {
                    live[key] = Value::Null;
                }
            }
            Ok(())
        }
    }
}

fn identity<'a>(entries: &'a [Value], id: &str) -> Option<&'a Value> {
    if id.is_empty() {
        return None;
    }
    let exact: Vec<_> = entries.iter().filter(|entry| entry["id"] == id).collect();
    if !exact.is_empty() {
        return (exact.len() == 1).then(|| exact[0]);
    }
    let imported: Vec<_> = entries
        .iter()
        .filter(|entry| entry["sourceId"] == id)
        .collect();
    (imported.len() == 1).then(|| imported[0])
}

pub(super) struct Jump {
    item_id: String,
    library_id: String,
    arrangement: String,
    slide_id: String,
    instance: Value,
}

/// `resolveActionJumpTarget` for a presentation: the first enabled cue matching the action's occurrence and
/// slide of the (arrangement of the) presentation.
pub(super) fn jump_target(
    facts: &Facts,
    item_ref: &str,
    arrangement: Option<&str>,
    entry: Option<&str>,
    details: &Value,
) -> Result<Option<Jump>, String> {
    let imported = || {
        let mut found = facts
            .items
            .iter()
            .filter(|item| item.value["sourceId"] == item_ref);
        let first = found.next()?;
        found.next().is_none().then_some(first)
    };
    let Some(item) = facts
        .items
        .get(item_ref)
        .or_else(imported)
        .filter(|_| !item_ref.is_empty())
        .map(|item| &*item.value)
    else {
        return Ok(None);
    };
    let arrangement = arrangement
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| crate::cues::model::selected_arrangement_id(item));
    if !arrangement.is_empty() {
        let known = item["arrangements"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if identity(known, arrangement).is_none() {
            return Ok(None);
        }
    }
    let arrangement = item["arrangements"]
        .as_array()
        .and_then(|list| identity(list, arrangement))
        .and_then(|found| found["id"].as_str())
        .unwrap_or("");
    let cues = orbit_host::Host::presentation_cues(item, arrangement, entry)?;
    let occurrence = details["cueInstanceId"].as_str().unwrap_or("");
    let slide = details["slideId"].as_str().unwrap_or("");
    let found = cues.iter().find(|cue| {
        cue["disabled"] != true
            && (occurrence.is_empty() || cue["arrangementInstanceId"] == occurrence)
            && (slide.is_empty() || cue["id"] == slide)
    });
    Ok(found.map(|cue| Jump {
        item_id: item["id"].as_str().unwrap_or_default().into(),
        library_id: facts
            .libraries
            .iter()
            .find(|library| library.item_ids.iter().any(|id| item["id"] == id.as_str()))
            .map(|library| library.id.clone())
            .unwrap_or_default(),
        arrangement: arrangement.into(),
        slide_id: cue["id"].as_str().unwrap_or_default().into(),
        instance: cue
            .get("arrangementInstanceId")
            .cloned()
            .unwrap_or(Value::Null),
    }))
}

fn presentation(
    live: &mut Value,
    facts: &Facts,
    action: &Value,
    run: &mut Run,
) -> Result<(), String> {
    let details = &action["details"];
    let arrangement = details["arrangementId"].as_str();
    let Some(jump) = jump_target(
        facts,
        action["targetId"].as_str().unwrap_or(""),
        arrangement,
        None,
        details,
    )?
    else {
        return Ok(());
    };
    if details["contentDestination"] == 1 {
        return announce(live, facts, &jump);
    }
    show_look_layer(live, "slide");
    let mut payload = json!({"presentationId":jump.item_id,"slideId":jump.slide_id,"cueInstanceId":jump.instance,"libraryId":jump.library_id});
    if !jump.arrangement.is_empty() {
        payload["arrangementId"] = json!(jump.arrangement);
    }
    run.steps.push(Step::Cue {
        command: "libraryCue",
        payload,
    });
    Ok(())
}

/// The announcements destination: the slide goes to the independent player, which starts manual on that slide.
fn announce(live: &mut Value, facts: &Facts, jump: &Jump) -> Result<(), String> {
    let item = facts
        .items
        .get(&jump.item_id)
        .map(|item| (*item.value).clone())
        .ok_or("automation.macros.targetInvalid")?;
    let cues = orbit_host::Host::presentation_cues(&item, &jump.arrangement, None)?;
    let mut shown = item;
    shown["selectedArrangementId"] = json!("");
    shown["arrangements"] = json!([]);
    live["announcements"] = super::announcements::select_slide(
        &shown,
        &cues,
        &jump.slide_id,
        &jump.instance,
        super::announcements::now_ms(),
        (facts.new_id)(),
        super::announcements::next_revision(),
    )?;
    Ok(())
}

fn playlist(
    live: &mut Value,
    facts: &Facts,
    action: &Value,
    depth: u32,
    run: &mut Run,
    nested: &mut Vec<(u32, Value)>,
) -> Result<(), String> {
    let id = action["targetId"].as_str().unwrap_or("");
    if id.is_empty() {
        return Ok(());
    }
    let playlists = facts.workspace["playlist"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let Some(playlist) = identity(playlists, id) else {
        return Ok(());
    };
    let wanted = action["details"]["itemId"].as_str().unwrap_or("");
    let entries: Vec<&Value> = playlist["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| {
            if wanted.is_empty() {
                matches!(entry["type"].as_str(), Some("presentation" | "cue"))
            } else {
                entry["id"] == wanted
            }
        })
        .collect();
    let entry = if !wanted.is_empty() && entries.len() != 1 {
        None
    } else {
        entries.first().copied()
    };
    let Some(entry) =
        entry.filter(|entry| matches!(entry["type"].as_str(), Some("presentation" | "cue")))
    else {
        return Ok(());
    };
    if entry["type"] == "cue" {
        let details = &action["details"];
        let Some(actions) = entry["actions"].as_array() else {
            return Ok(());
        };
        if details["arrangementId"].is_string()
            || details["cueInstanceId"].is_string()
            || details["slideId"].is_string()
        {
            return Ok(());
        }
        nested.extend(actions.iter().cloned().map(|action| (depth + 1, action)));
        return Ok(());
    }
    let entry_id = entry["id"].as_str().unwrap_or("");
    let playlist_id = playlist["id"].as_str().unwrap_or("");
    let arrangement = action["details"]["arrangementId"]
        .as_str()
        .or(entry["arrangementId"].as_str());
    let Some(jump) = jump_target(
        facts,
        entry["itemId"].as_str().unwrap_or(""),
        arrangement,
        Some(entry_id),
        &action["details"],
    )?
    else {
        return Ok(());
    };
    if action["details"]["alwaysRetrigger"] == false
        && live["livePlaylistId"] == playlist_id
        && live["liveItemId"] == jump.item_id.as_str()
        && live["livePlaylistEntryId"] == entry_id
    {
        return Ok(());
    }
    show_look_layer(live, "slide");
    let mut payload = json!({"presentationId":jump.item_id,"slideId":jump.slide_id,"cueInstanceId":jump.instance,"playlistId":playlist_id,"playlistEntryId":entry_id});
    if !jump.arrangement.is_empty() {
        payload["arrangementId"] = json!(jump.arrangement);
    }
    run.steps.push(Step::Cue {
        command: "playlistCue",
        payload,
    });
    Ok(())
}

fn audience_look(live: &mut Value, facts: &Facts, id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Ok(());
    }
    let Some(look) = facts.workspace["audienceLooks"]
        .as_array()
        .and_then(|looks| looks.iter().find(|look| look["id"] == id))
    else {
        return Ok(());
    };
    for screen_look in look["screenLooks"].as_array().into_iter().flatten() {
        let Some(screen) = screen_look["screenId"]
            .as_str()
            .and_then(|id| live["screenConfig"].get_mut(id))
        else {
            continue;
        };
        let patch = orbit_host::audience_looks::screen_patch(screen, screen_look);
        for key in ["lookLayers", "lookThemes"] {
            screen[key] = patch[key].clone();
        }
        screen["lookTransition"] = orbit_host::audience_looks::transition_patch(look);
    }
    live["activeAudienceLookId"] = json!(id);
    Ok(())
}

fn source_matches(asset: &Value, id: &str) -> bool {
    !id.is_empty() && (asset["id"] == id || asset["sourceId"] == id)
}

fn media(live: &mut Value, facts: &Facts, action: &Value, run: &mut Run) -> Result<(), String> {
    let id = action["targetId"].as_str().unwrap_or("");
    let Some(asset) = facts.workspace["media"]
        .as_array()
        .and_then(|media| media.iter().find(|asset| source_matches(asset, id)))
    else {
        return Ok(());
    };
    let details = &action["details"];
    if details.get("unsupportedMediaTransport").is_some()
        || (details.get("mediaTransport").is_some()
            && (asset["type"] != "video" || !details["mediaTransport"].is_object()))
    {
        return Err(
            "Media action transport requires complete valid video transport settings.".into(),
        );
    }
    if asset["type"] == "audio" {
        live["audioId"] = asset["id"].clone();
        live["audioSelectionId"] = json!(live["audioSelectionId"].as_u64().unwrap_or(0) + 1);
        live["musicPlaybackState"] = Value::Null;
        live["transportPlaybackPaused"]["music"] = json!(false);
        if asset.get("markers").is_none_or(Value::is_null) {
            run.steps.push(Step::Markers {
                layer: "music",
                markers: details["markers"].clone(),
            });
        }
        return Ok(());
    }
    let selection = live["backgroundSelectionId"].as_u64().unwrap_or(0) + 1;
    let playback = if details["playback"].to_string().contains("LOOP") {
        "loop"
    } else {
        "stop"
    };
    let end = details["endBehavior"].as_str().unwrap_or("");
    let end_behavior = if end.contains("FADE_TO_BLACK") {
        "fade-black"
    } else if end.contains("FADE_TO_CLEAR") {
        "fade-clear"
    } else if end.contains("STOP_ON_BLACK") {
        "black"
    } else if end.contains("STOP_ON_CLEAR") {
        "clear"
    } else {
        "hold"
    };
    let duration = details["transitionDuration"]
        .as_f64()
        .filter(|value| *value != 0.0)
        .or_else(|| details["transition"]["durationSeconds"].as_f64())
        .unwrap_or(0.0);
    let effect = details["transition"]["effect"]["renderId"]
        .as_str()
        .or_else(|| details["transition"]["effect"]["name"].as_str())
        .unwrap_or("dissolve");
    live["backgroundId"] = asset["id"].clone();
    live["backgroundSelectionId"] = json!(selection);
    live["backgroundPlaybackState"] = json!({"selectionId":selection,"status":"playing"});
    let transport = details
        .get("mediaTransport")
        .or_else(|| asset.get("mediaTransport"))
        .filter(|value| value.is_object());
    live["backgroundTransportOverride"] = match transport {
        Some(transport) => {
            let mut over = json!({"selectionId":selection,"transport":transport});
            if matches!(
                details["backgroundBehavior"].as_str(),
                Some("background" | "foreground")
            ) {
                over["behavior"] = details["backgroundBehavior"].clone();
            }
            over
        }
        None => Value::Null,
    };
    live["backgroundEffectsOverride"] = match details["mediaEffects"].as_array() {
        Some(chain) => json!({"selectionId":selection,"chain":chain}),
        None => Value::Null,
    };
    live["transportPlaybackPaused"]["media"] = json!(false);
    show_look_layer(live, "media");
    live["backgroundPlayback"] = json!(playback);
    live["backgroundEndBehavior"] = json!(end_behavior);
    live["backgroundTransition"] = match details["backgroundTransition"].as_str() {
        Some(transition) => json!(transition),
        None if duration > 0.0 => json!(format!("fade:{duration}:{effect}")),
        None => json!("none"),
    };
    if matches!(
        details["backgroundBehavior"].as_str(),
        Some("background" | "foreground")
    ) {
        live["backgroundBehavior"] = details["backgroundBehavior"].clone();
    }
    if matches!(
        details["backgroundScale"].as_str(),
        Some("fit" | "fill" | "stretch")
    ) {
        live["backgroundScale"] = details["backgroundScale"].clone();
    }
    if asset.get("markers").is_none_or(Value::is_null) {
        run.steps.push(Step::Markers {
            layer: "media",
            markers: details["markers"].clone(),
        });
    }
    Ok(())
}

fn prop(live: &mut Value, action: &Value, run: &mut Run) {
    let id = action["targetId"].as_str().unwrap_or("");
    if id.is_empty() {
        return;
    }
    let mut current: Vec<Value> = live["livePropIds"].as_array().cloned().unwrap_or_default();
    if action["details"]["toggleAutoClearPause"] == true {
        if current.iter().any(|entry| entry == id) {
            if let Some(deadline) = live["remotePropDeadlines"][id].as_f64() {
                let now = super::announcements::now_ms();
                if deadline < 0. {
                    arm_prop_clear(live, action, run, id, now, -deadline / 1000.);
                } else {
                    // A paused countdown holds negative remaining milliseconds in the deadline map.
                    live["remotePropDeadlines"][id] = json!(-(deadline - now).max(1.));
                }
            }
        }
        return;
    }
    let clearing = action["value"] == "clear";
    if let Some(expected) = action["details"].get("expectedPropDeadline") {
        if live["remotePropDeadlines"][id] != *expected {
            return;
        }
    }
    if clearing {
        current.retain(|entry| entry != id);
    } else if action["details"]["singleMode"] == true {
        current = vec![json!(id)];
    } else if !current.iter().any(|entry| entry == id) {
        current.push(json!(id));
    }
    if !live["remotePropDeadlines"].is_object() {
        live["remotePropDeadlines"] = json!({});
    }
    if let Some(deadlines) = live["remotePropDeadlines"].as_object_mut() {
        deadlines.retain(|key, _| current.iter().any(|id| id == key));
        deadlines.remove(id);
    }
    live["livePropIds"] = Value::Array(current);
    show_look_layer(live, "props");
    let seconds = action["details"]["autoClearSeconds"]
        .as_f64()
        .unwrap_or(0.0);
    if !clearing && action["details"]["autoClearEnabled"] == true && seconds > 0.0 {
        arm_prop_clear(
            live,
            action,
            run,
            id,
            super::announcements::now_ms(),
            seconds,
        );
    }
}

fn arm_prop_clear(
    live: &mut Value,
    action: &Value,
    run: &mut Run,
    id: &str,
    now: f64,
    seconds: f64,
) {
    live["remotePropDeadlines"][id] = json!(now + seconds * 1000.);
    let mut clear = action.clone();
    clear["value"] = json!("clear");
    clear["details"]["expectedPropDeadline"] = live["remotePropDeadlines"][id].clone();
    clear["details"]["autoClearEnabled"] = json!(false);
    clear["details"]
        .as_object_mut()
        .map(|details| details.remove("toggleAutoClearPause"));
    run.steps.push(Step::Delay {
        actions: vec![clear],
        seconds,
        depth: 0,
    });
}

fn stage(live: &mut Value, action: &Value) {
    let id = action["targetId"].as_str().unwrap_or("");
    if id.is_empty() {
        return;
    }
    let assigned: Vec<Value> = action["details"]["assignments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry.is_object())
        .map(|entry| {
            (
                entry["screen"].as_str().unwrap_or(""),
                entry["layout"]
                    .as_str()
                    .filter(|l| !l.is_empty())
                    .unwrap_or(id),
            )
        })
        .filter(|(screen, layout)| !screen.is_empty() && !layout.is_empty())
        .map(|(screen, layout)| json!({"screenId":screen,"layoutId":layout}))
        .collect();
    // Screens the action does not assign keep their layout.
    let mut kept: Vec<Value> = live["stageScreenAssignments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| {
            !assigned
                .iter()
                .any(|next| next["screenId"] == entry["screenId"])
        })
        .cloned()
        .collect();
    kept.extend(assigned);
    live["selectedStageLayoutId"] = json!(id);
    live["stageScreenAssignments"] = Value::Array(kept);
}

fn message(live: &mut Value, facts: &Facts, action: &Value) {
    let id = action["targetId"].as_str().unwrap_or("");
    let Some(message) = facts.workspace["messages"].as_array().and_then(|messages| {
        messages
            .iter()
            .find(|message| message["id"] == id && !id.is_empty())
    }) else {
        return;
    };
    let mut values = message["tokenValues"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(overrides) = action["details"]["tokenValues"].as_object() {
        values.extend(overrides.clone());
    }
    let mut text = message["text"].as_str().unwrap_or("").to_string();
    for (name, value) in &values {
        text = text.replace(&format!("{{{name}}}"), value.as_str().unwrap_or(""));
    }
    let mut definition = message.clone();
    definition["tokenValues"] = Value::Object(values);
    live["activeMessageId"] = message["id"].clone();
    live["activeMessageInstanceId"] = json!((facts.new_id)());
    live["activeMessageText"] = json!(text);
    live["activeMessageDefinition"] = definition;
    show_look_layer(live, "messages");
}

pub(super) fn audio_input(facts: &Facts, action: &Value, run: &mut Run) -> Result<(), String> {
    let inputs = facts.workspace["audioInputs"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let id = action["targetId"].as_str().unwrap_or("");
    let input = if id.is_empty() {
        action["details"]["index"]
            .as_u64()
            .and_then(|index| inputs.get(index as usize))
    } else {
        inputs.iter().find(|input| input["id"] == id)
    };
    let Some(input) = input else {
        return if id.is_empty() {
            Ok(())
        } else {
            Err("audioInputs.invalid".into())
        };
    };
    let details = &action["details"];
    let mut overrides = json!({});
    if let Some(mode) = details.get("behaviorMode").filter(|mode| !mode.is_null()) {
        if !matches!(mode.as_str(), Some("off" | "on" | "auto-off" | "auto-on")) {
            return Err("audioInputs.invalid".into());
        }
        overrides["behaviorMode"] = mode.clone();
    }
    if let Some(level) = details.get("levelDb") {
        match level.as_f64() {
            Some(level) if (-60.0..=6.0).contains(&level) => overrides["levelDb"] = json!(level),
            _ => return Err("audioInputs.invalid".into()),
        }
    }
    run.steps.push(Step::AudioInput {
        id: input["id"].as_str().unwrap_or_default().into(),
        overrides,
    });
    Ok(())
}

fn communication(facts: &Facts, action: &Value, run: &mut Run) {
    let details = &action["details"];
    let identification = &details["deviceIdentification"];
    let target = action["targetId"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| {
            identification["uuid"]
                .as_str()
                .or_else(|| identification["string"].as_str())
                .unwrap_or("")
                .to_string()
        });
    let device = facts.workspace["communicationDevices"]
        .as_array()
        .and_then(|devices| {
            devices
                .iter()
                .find(|device| device["id"] == target.as_str())
        })
        .cloned()
        .unwrap_or(Value::Null);
    let payload = match action["value"].as_str() {
        Some(value) => value.to_string(),
        None => details["commands"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|command| command["value"].as_str().unwrap_or(""))
            .collect(),
    };
    let mut sent = details.clone();
    sent["actionId"] = action["id"].clone();
    sent["deviceId"] = json!(target);
    run.steps.push(Step::Communication {
        device,
        payload,
        details: sent,
    });
}

fn transport(live: &mut Value, action: &Value) {
    let command = &action["details"];
    let paused = !command["pause"].is_null() && command["pause"] != false;
    if let Some(time) = command["jumpToTime"]["time"].as_f64() {
        live["backgroundPlaybackState"] = json!({
            "selectionId": live["backgroundSelectionId"].clone(),
            "status": if paused { "paused" } else { "playing" },
            "currentTime": time,
        });
    }
    live["transportPlaybackPaused"]["media"] = json!(paused);
}

const TIMER_COMMANDS: [&str; 6] = [
    "start",
    "stop",
    "reset",
    "reset-and-start",
    "stop-and-reset",
    "increment",
];

fn timer(facts: &Facts, action: &Value, run: &mut Run) {
    let id = action["targetId"].as_str().unwrap_or("");
    let Some(timer) = facts.workspace["countdownTimers"]
        .as_array()
        .and_then(|timers| timers.iter().find(|timer| timer["id"] == id))
    else {
        return;
    };
    let command = action["value"]
        .as_str()
        .filter(|value| TIMER_COMMANDS.contains(value))
        .unwrap_or("start");
    run.steps.push(Step::Timer {
        timer: timer.clone(),
        command: command.into(),
        increment: action["details"]["incrementSeconds"]
            .as_f64()
            .unwrap_or(0.0),
        configuration: action["details"]
            .get("configuration")
            .filter(|value| value.is_object())
            .cloned(),
    });
}
