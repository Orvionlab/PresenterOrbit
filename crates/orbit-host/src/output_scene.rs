//! Typed boundary and canonical builder for native output scenes.
//!
//! The renderer collects live UI state and resolves asynchronous RSS data,
//! then sends one strict state snapshot. Rust owns the per-screen decisions:
//! looks, theme precedence, props, stage, masks, background/audio visibility,
//! and linked-text context. Workspace state is not reloaded here; the nested
//! presentation values are deliberately kept as JSON domain payloads.

use std::{borrow::Cow, collections::BTreeMap};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::output_arrangement::{live_slide_index, resolve_live_arrangement_slides};
use crate::output_canvas::{CanvasRegion, CanvasViewport, ScreenCanvasConfig};
use crate::projector_correction::ProjectorCorrection;
#[path = "stage_scene_resources.rs"]
mod stage_scene_resources;

pub const OUTPUT_SCENE_SNAPSHOT_VERSION: u64 = 2;

fn default_true() -> bool {
    true
}

fn empty_array() -> Value {
    Value::Array(Vec::new())
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputSceneLookLayers {
    #[serde(default = "default_true")]
    pub music: bool,
    #[serde(default = "default_true")]
    pub messages: bool,
    #[serde(default = "default_true")]
    pub props: bool,
    #[serde(default = "default_true")]
    pub announcements: bool,
    #[serde(default = "default_true")]
    pub slide: bool,
    #[serde(default = "default_true")]
    pub media: bool,
    #[serde(default = "default_true")]
    pub video_input: bool,
}

impl Default for OutputSceneLookLayers {
    fn default() -> Self {
        Self {
            music: true,
            messages: true,
            props: true,
            announcements: true,
            slide: true,
            media: true,
            video_input: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputSceneLookThemes {
    #[serde(default)]
    pub mask_id: String,
    #[serde(default)]
    pub presentation_theme_id: String,
}

/// Strict serializable scene DTO consumed by all native output targets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputScene {
    #[serde(default)]
    pub output_frozen: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_fill_signal: Option<crate::key_fill::KeyFillSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_fill_source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_fill_partner_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projector_correction: Option<ProjectorCorrection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_viewport: Option<CanvasViewport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_delay: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_pattern: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_destination: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub look_layers: OutputSceneLookLayers,
    #[serde(default)]
    pub look_themes: OutputSceneLookThemes,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_theme_slide: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub announcement_overlay: Option<Box<OutputScene>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ccli_slide: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_input_slide: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slide_transition: Option<Value>,
    /// Media, prop and message changes with their library transitions; see [`layer_transition_context`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_transitions: Option<Value>,
    /// The fade of the last applied Audience Look: its duration and a token that changes per application.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look_transition: Option<Value>,
    /// Identifies the shown slide occurrence; slide fill videos restart when it changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slide_entry_key: Option<String>,
    /// Separates object build playback from the slide fill clock, including same-cue replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_build_entry_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline_started_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub object_build_step_started_at_ms_by_step: BTreeMap<u64, i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prop_slides: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_slide: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_slide: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_message: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_slide: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_text_context: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_css: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_media_hardware_decoding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_desaturate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_brightness: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_contrast: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_hue: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_saturation: Option<f64>,
    /// The selected media's effect chain (`mediaEffects`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_effects: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_media_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_media_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_selection_id: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_playback_state: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_transport: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_scale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_playback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_end_behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_transition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_selection_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_settings: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_inputs: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_input_context: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_audio_routing: Option<Value>,
    /// Whether the selected audio loops (its `audioPlayback`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_loop: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_loop_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_loop_time: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_crossfade_seconds: Option<f64>,
    /// The shown video's own audio route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_audio_routing: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_playback_paused: Option<BTreeMap<String, bool>>,
}

/// Screen state consumed by the native per-screen builder. Renderer-owned
/// screen settings that do not affect output selection are accepted in the
/// flattened remainder but never forwarded into a native scene.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputScreenConfigInput {
    #[serde(rename = "type", default)]
    pub screen_type: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub look_layers: Option<Map<String, Value>>,
    #[serde(default)]
    pub look_themes: Option<Map<String, Value>>,
    #[serde(default)]
    pub stage_layout_id: Option<String>,
    #[serde(default)]
    pub projector_correction: Option<ProjectorCorrection>,
    #[serde(flatten)]
    pub _other: Map<String, Value>,
}

/// The only production input accepted by `updateOutputScene`.
///
/// Arrays contain already-loaded presentation domain values. They are not
/// workspace caches: the renderer supplies the current snapshot so Rust can
/// make deterministic per-screen choices without duplicating editor state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputSceneStateInput<'a> {
    #[serde(default)]
    pub output_frozen: bool,
    /// Controller workspace identity for music EOF; never used to read or write a workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_workspace_path: Option<String>,
    #[serde(default)]
    pub active_audience_look_id: Option<String>,
    #[serde(default)]
    pub focused_presentation: Option<Value>,
    #[serde(default)]
    pub focused_playlist_id: Option<String>,
    #[serde(default)]
    pub video_inputs: Vec<Value>,
    #[serde(default)]
    pub live_video_input_id: Option<String>,
    #[serde(default)]
    pub announcements: Option<Value>,
    #[serde(rename = "activeActionSlide", default)]
    pub _active_action_slide: Option<Value>,
    /// Operator navigation state kept in the snapshot so cue commands can read it; it does not affect scenes.
    #[serde(rename = "presentationNextAnchor", default)]
    pub _presentation_next_anchor: Option<Value>,
    /// Controller deadlines used to guard delayed prop clears; they do not affect rendering.
    #[serde(rename = "remotePropDeadlines", default)]
    pub _remote_prop_deadlines: BTreeMap<String, Option<f64>>,
    #[serde(default)]
    pub live_item: Option<Value>,
    #[serde(rename = "liveItemId", default)]
    pub _live_item_id: Option<String>,
    #[serde(default)]
    pub live_slide: Option<Value>,
    #[serde(default)]
    pub live_slide_id: Option<String>,
    #[serde(default)]
    pub live_cue_instance_id: Option<String>,
    #[serde(rename = "timelineStartedAtMs", default)]
    pub _timeline_started_at_ms: Option<i64>,
    #[serde(default)]
    pub live_build_step: f64,
    #[serde(default)]
    pub live_build_step_started_at_ms_by_step: BTreeMap<u64, i64>,
    #[serde(default)]
    pub live_prop_ids: Vec<String>,
    #[serde(default)]
    pub live_playlist_id: Option<String>,
    #[serde(default)]
    pub live_playlist_entry_id: Option<String>,
    #[serde(default)]
    pub library: Vec<Value>,
    #[serde(default)]
    pub libraries: Vec<Value>,
    #[serde(default)]
    pub playlist: Vec<Value>,
    #[serde(default)]
    pub screen_config: BTreeMap<String, OutputScreenConfigInput>,
    #[serde(default)]
    pub group_settings: BTreeMap<String, Value>,
    #[serde(default)]
    pub output_canvas_sizes: BTreeMap<String, Value>,
    #[serde(default)]
    pub timeline_transport: Option<Value>,
    #[serde(default)]
    pub themes: Cow<'a, [Value]>,
    #[serde(default)]
    pub props: Vec<Value>,
    #[serde(default)]
    pub stage_layouts: Vec<Value>,
    #[serde(default)]
    pub stage_screen_assignments: Vec<Value>,
    #[serde(default)]
    pub masks: Vec<Value>,
    #[serde(default)]
    pub ccli_template: Option<Value>,
    #[serde(default)]
    pub ccli_display_enabled: Option<bool>,
    #[serde(default)]
    pub ccli_display_location: CcliDisplayLocation,
    #[serde(default)]
    pub ccli_license_number: Option<String>,
    #[serde(default)]
    pub background_id: Option<String>,
    #[serde(default)]
    pub backgrounds: Vec<Value>,
    #[serde(default)]
    pub media: Vec<Value>,
    #[serde(default)]
    pub logo_image_path: String,
    #[serde(default)]
    pub logo_image_url: String,
    #[serde(default)]
    pub ignore_background_colors: bool,
    #[serde(default)]
    pub background_scale: String,
    #[serde(default)]
    pub background_behavior: Option<String>,
    #[serde(default)]
    pub background_playback: Option<String>,
    #[serde(default)]
    pub background_end_behavior: String,
    #[serde(default)]
    pub background_playback_state: Option<Value>,
    #[serde(default)]
    pub background_selection_id: Option<Value>,
    /// A media action's own effect chain, applied while its selection is current.
    #[serde(default)]
    pub background_effects_override: Option<Value>,
    #[serde(default)]
    pub background_transport_override: Option<Value>,
    #[serde(default)]
    pub background_transition: Option<String>,
    #[serde(default)]
    pub audio_id: Option<String>,
    #[serde(default)]
    pub audio_selection_id: Option<u64>,
    #[serde(default)]
    pub selected_audio: Option<Value>,
    #[serde(default)]
    pub media_libraries: Vec<Value>,
    #[serde(rename = "activeMessageId", default)]
    pub _active_message_id: Option<String>,
    #[serde(rename = "activeMessageInstanceId", default)]
    pub _active_message_instance_id: Option<String>,
    #[serde(default)]
    pub active_message_text: String,
    #[serde(default)]
    pub active_message_definition: Option<Value>,
    #[serde(default)]
    pub stage_message: Option<Value>,
    #[serde(default = "empty_array")]
    pub countdown_timers: Value,
    #[serde(default = "empty_object")]
    pub countdown_timer_runs: Value,
    #[serde(default = "empty_array")]
    pub audio_inputs: Value,
    #[serde(default)]
    pub audio_settings: Option<Value>,
    #[serde(default)]
    pub transport_playback_paused: Option<BTreeMap<String, bool>>,
    /// The workspace's default slide transition.
    #[serde(default)]
    pub slide_transition: Option<Value>,
    /// The workspace's default transitions of the media, prop and message layers.
    #[serde(default)]
    pub layer_transitions: Option<Value>,
}

pub(crate) fn parse_scene_state(
    value: &Value,
) -> Result<OutputSceneStateInput<'_>, serde_json::Error> {
    use serde::de::Error;
    let object = value
        .as_object()
        .ok_or_else(|| serde_json::Error::custom("Output scene state must be an object."))?;
    let themes = match object.get("themes") {
        Some(themes) => themes
            .as_array()
            .ok_or_else(|| serde_json::Error::custom("Output scene themes must be an array."))?
            .as_slice(),
        None => &[],
    };
    let fields = object
        .iter()
        .filter(|(key, _)| key.as_str() != "themes")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let mut input: OutputSceneStateInput = serde_json::from_value(Value::Object(fields))?;
    input.themes = Cow::Borrowed(themes);
    Ok(input)
}

pub fn build_next_output_scene_state(value: &Value) -> Result<Option<Map<String, Value>>, String> {
    let mut input: OutputSceneStateInput =
        parse_scene_state(value).map_err(|error| format!("Invalid output scene state: {error}"))?;
    let Some(next_slide) = next_enabled_slide_for_live_item(&input) else {
        return Ok(None);
    };
    input.live_slide_id = next_slide
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string);
    input.live_cue_instance_id = next_slide
        .get("arrangementInstanceId")
        .and_then(Value::as_str)
        .map(str::to_string);
    input.live_slide = Some(next_slide);
    input.live_build_step = 0.0;
    input.live_build_step_started_at_ms_by_step.clear();
    input._timeline_started_at_ms = None;
    build_output_scene_state_from_input(&input).map(Some)
}

#[derive(Debug, Clone, PartialEq)]
pub enum OutputSceneUpdateV2 {
    Full {
        session_id: String,
        client_sequence: u64,
        base_revision: Option<u64>,
        snapshot: Value,
    },
    Patch {
        session_id: String,
        client_sequence: u64,
        base_revision: u64,
        patch: Value,
    },
}

/// Decode the strict v2 renderer update envelope. Raw scene DTOs continue to
/// use [`build_output_scene`] and are intentionally not accepted here.
pub fn parse_output_scene_update(value: &Value) -> Result<OutputSceneUpdateV2, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "Output scene update must be an object.".to_string())?;
    let version = object
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Output scene update version must be an integer.".to_string())?;
    if version != OUTPUT_SCENE_SNAPSHOT_VERSION {
        return Err(format!(
            "Unsupported output scene update version {version}; expected {OUTPUT_SCENE_SNAPSHOT_VERSION}."
        ));
    }
    let session_id = object
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|session_id| !session_id.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "Output scene update sessionId must be a non-empty string.".to_string())?;
    let client_sequence = object
        .get("clientSequence")
        .and_then(Value::as_u64)
        .filter(|sequence| *sequence > 0)
        .ok_or_else(|| {
            "Output scene update clientSequence must be a positive integer.".to_string()
        })?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "Output scene update kind must be 'full' or 'patch'.".to_string())?;

    match kind {
        "full" => {
            reject_fields(
                object,
                &[
                    "version",
                    "kind",
                    "sessionId",
                    "clientSequence",
                    "baseRevision",
                    "snapshot",
                ],
            )?;
            let base_revision = optional_positive_revision(object.get("baseRevision"), "full")?;
            let snapshot = object
                .get("snapshot")
                .cloned()
                .ok_or_else(|| "Full output scene updates require a snapshot.".to_string())?;
            if !snapshot.is_object() {
                return Err("Full output scene snapshot must be an object.".to_string());
            }
            Ok(OutputSceneUpdateV2::Full {
                session_id,
                client_sequence,
                base_revision,
                snapshot,
            })
        }
        "patch" => {
            reject_fields(
                object,
                &[
                    "version",
                    "kind",
                    "sessionId",
                    "clientSequence",
                    "baseRevision",
                    "patch",
                ],
            )?;
            let base_revision = optional_positive_revision(object.get("baseRevision"), "patch")?
                .ok_or_else(|| {
                    "Output scene patch baseRevision must be a positive integer.".to_string()
                })?;
            let patch = object
                .get("patch")
                .cloned()
                .ok_or_else(|| "Patch output scene updates require a patch object.".to_string())?;
            if !patch.is_object() {
                return Err("Output scene patch must be an object.".to_string());
            }
            validate_patch_fields(&patch)?;
            Ok(OutputSceneUpdateV2::Patch {
                session_id,
                client_sequence,
                base_revision,
                patch,
            })
        }
        _ => Err("Output scene update kind must be 'full' or 'patch'.".to_string()),
    }
}

fn optional_positive_revision(value: Option<&Value>, kind: &str) -> Result<Option<u64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    value
        .as_u64()
        .filter(|revision| *revision > 0)
        .map(Some)
        .ok_or_else(|| format!("Output scene {kind} baseRevision must be a positive integer."))
}

pub fn merge_output_scene_patch(base: &Value, patch: &Value) -> Result<Value, String> {
    validate_patch_fields(patch)?;
    let mut merged = base
        .as_object()
        .cloned()
        .ok_or_else(|| "Cached output scene snapshot must be an object.".to_string())?;
    let patch = patch
        .as_object()
        .ok_or_else(|| "Output scene patch must be an object.".to_string())?;
    merged.extend(patch.clone());
    Ok(Value::Object(merged))
}

pub fn snapshot_hash(value: &Value) -> String {
    hash_json(value)
}

pub fn patched_snapshot_hash(base_hash: &str, patch_fingerprint: &str) -> String {
    hash_bytes(format!("{base_hash}:{patch_fingerprint}").as_bytes())
}

/// Fingerprint an update payload for same-sequence idempotence and conflict
/// detection. This is native protocol metadata; the renderer never computes
/// or persists it.
pub fn update_fingerprint(kind: &str, payload: &Value) -> String {
    let value = json!({ "kind": kind, "payload": payload });
    hash_json(&value)
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3_u64);
    }
    format!("{hash:016x}")
}

struct CanonicalJson<'a>(&'a Value);

impl Serialize for CanonicalJson<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self.0 {
            Value::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(&CanonicalJson(value))?;
                }
                sequence.end()
            }
            Value::Object(values) => {
                let mut keys = values.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                let mut object = serializer.serialize_map(Some(keys.len()))?;
                for key in keys {
                    object.serialize_entry(key, &CanonicalJson(&values[key]))?;
                }
                object.end()
            }
            value => value.serialize(serializer),
        }
    }
}

struct JsonHash(u64);
impl std::io::Write for JsonHash {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3_u64);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn hash_json(value: &Value) -> String {
    let mut hash = JsonHash(0xcbf29ce484222325);
    serde_json::to_writer(&mut hash, &CanonicalJson(value))
        .expect("JSON values and the in-memory hash writer cannot fail");
    format!("{:016x}", hash.0)
}

const OUTPUT_SCENE_PATCH_FIELDS: &[&str] = &[
    "outputFrozen",
    "activeAudienceLookId",
    "focusedPresentation",
    "focusedPlaylistId",
    "videoInputs",
    "liveVideoInputId",
    "activeActionSlide",
    "liveItem",
    "liveItemId",
    "announcements",
    "liveSlide",
    "liveSlideId",
    "liveCueInstanceId",
    "timelineStartedAtMs",
    "liveBuildStep",
    "liveBuildStepStartedAtMsByStep",
    "livePropIds",
    "remotePropDeadlines",
    "livePlaylistId",
    "livePlaylistEntryId",
    "library",
    "libraries",
    "playlist",
    "screenConfig",
    "themes",
    "props",
    "stageLayouts",
    "stageScreenAssignments",
    "masks",
    "ccliTemplate",
    "ccliDisplayEnabled",
    "ccliDisplayLocation",
    "ccliLicenseNumber",
    "backgroundId",
    "backgrounds",
    "media",
    "logoImagePath",
    "logoImageUrl",
    "ignoreBackgroundColors",
    "backgroundScale",
    "backgroundBehavior",
    "backgroundPlayback",
    "backgroundEndBehavior",
    "backgroundPlaybackState",
    "backgroundSelectionId",
    "backgroundTransportOverride",
    "backgroundTransition",
    "audioId",
    "audioSelectionId",
    "groupSettings",
    "outputCanvasSizes",
    "timelineTransport",
    "selectedAudio",
    "mediaLibraries",
    "activeMessageId",
    "activeMessageInstanceId",
    "activeMessageText",
    "activeMessageDefinition",
    "stageMessage",
    "countdownTimers",
    "countdownTimerRuns",
    "audioSettings",
    "audioInputs",
    "transportPlaybackPaused",
    "slideTransition",
    "layerTransitions",
];

/// Whether an `updateOutputScene` patch may carry this field. Other fields travel only in a full snapshot.
pub fn patch_field_supported(field: &str) -> bool {
    OUTPUT_SCENE_PATCH_FIELDS.contains(&field)
}

fn validate_patch_fields(value: &Value) -> Result<(), String> {
    let fields = value
        .as_object()
        .ok_or_else(|| "Output scene patch must be an object.".to_string())?;
    if let Some(field) = fields
        .keys()
        .find(|field| !OUTPUT_SCENE_PATCH_FIELDS.contains(&field.as_str()))
    {
        return Err(format!(
            "Output scene patch field '{field}' is not supported."
        ));
    }
    Ok(())
}

fn reject_fields(value: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    if let Some(field) = value
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(format!("Unknown output scene update field '{field}'."));
    }
    Ok(())
}

/// Validate and canonicalize a raw scene used by explicit native callers
/// such as the window creation smoke. Production renderer updates use
/// [`build_output_scene_state`] instead.
pub fn build_output_scene(source_screen_id: &str, value: &Value) -> Result<Value, String> {
    let scene: OutputScene = OutputScene::deserialize(value).map_err(|error| {
        format!("Output scene for '{source_screen_id}' does not match the native contract: {error}")
    })?;
    validate_scene(&scene, source_screen_id)?;
    serde_json::to_value(scene).map_err(|error| {
        format!("Output scene for '{source_screen_id}' could not be serialized: {error}")
    })
}

/// Build all canonical scenes from the one renderer state snapshot.
pub fn build_output_scene_state(value: &Value) -> Result<Map<String, Value>, String> {
    let mut input = parse_scene_state(value).map_err(|error| {
        format!("Output scene state does not match the native contract: {error}")
    })?;
    let has_layout = value["screenConfig"]
        .as_object()
        .into_iter()
        .flatten()
        .any(|(_, config)| {
            config
                .get("outputLayout")
                .is_some_and(|layout| !layout.is_null())
        });
    if has_layout {
        let expanded = crate::output_layout::expand(&value["screenConfig"])?;
        for (id, config) in expanded.as_object().into_iter().flatten() {
            if value["screenConfig"].get(id).is_none() {
                input.output_canvas_sizes.insert(
                    id.clone(),
                    json!({"width": config["ndiWidth"], "height": config["ndiHeight"]}),
                );
            }
        }
        input.screen_config = serde_json::from_value(expanded)
            .map_err(|error| format!("Expanded output screen configuration is invalid: {error}"))?;
    }
    validate_build_step_clock(&input)?;

    build_output_scene_state_from_input(&input)
}

fn validate_build_step_clock(input: &OutputSceneStateInput) -> Result<(), String> {
    const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
    if !input.live_build_step.is_finite()
        || input.live_build_step < 0.0
        || input.live_build_step.fract() != 0.0
        || input.live_build_step > MAX_SAFE_INTEGER as f64
    {
        return Err("Live build step must be a non-negative safe integer.".into());
    }
    let current_step = input.live_build_step as u64;
    for (&step, &started_at_ms) in &input.live_build_step_started_at_ms_by_step {
        if step > MAX_SAFE_INTEGER as u64 || step > current_step {
            return Err(
                "Live build step clock must not contain future or unsafe step numbers.".into(),
            );
        }
        if !(0..=MAX_SAFE_INTEGER).contains(&started_at_ms) {
            return Err(
                "Live build step clock timestamps must be non-negative safe integers.".into(),
            );
        }
    }
    if let Some(timeline_started_at_ms) = input
        ._timeline_started_at_ms
        .filter(|_| input.live_slide.is_some())
    {
        if !(0..=MAX_SAFE_INTEGER).contains(&timeline_started_at_ms)
            || input.live_build_step_started_at_ms_by_step.get(&0) != Some(&timeline_started_at_ms)
        {
            return Err("Live build step zero must match the timeline start timestamp.".into());
        }
    }
    if input.live_slide.is_some()
        && input._timeline_started_at_ms.is_some()
        && !input
            .live_build_step_started_at_ms_by_step
            .contains_key(&current_step)
    {
        return Err("The live build step must have an authoritative start timestamp.".into());
    }
    Ok(())
}

fn build_output_scene_state_from_input(
    input: &OutputSceneStateInput,
) -> Result<Map<String, Value>, String> {
    for library in &input.media_libraries {
        crate::workspace::validate_audio_playlist_options(library)?;
    }
    let screen_config_value = serde_json::to_value(&input.screen_config)
        .map_err(|error| format!("Screen configuration could not be serialized: {error}"))?;
    let canvas_configs = crate::output_canvas::parse_screen_config_map(&screen_config_value)?;
    let needs_prop_slides = input.screen_config.values().any(|config| {
        config.screen_type.as_deref() != Some("stage")
            && read_look_layer(config.look_layers.as_ref(), "props")
    });
    let live_prop_slides = needs_prop_slides.then(|| {
        input
            .props
            .iter()
            .filter(|prop| {
                prop.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| input.live_prop_ids.iter().any(|live_id| live_id == id))
            })
            .filter_map(|prop| prop.get("slide"))
            .collect::<Vec<_>>()
    });

    let mut outputs: Map<String, Value> = input
        .screen_config
        .iter()
        .map(|(screen_id, config)| {
            build_screen_scene(input, screen_id, config, live_prop_slides.as_deref())
                .and_then(|scene| serde_json::to_value(scene).map_err(|error| error.to_string()))
                .map(|scene| (screen_id.clone(), scene))
        })
        .collect::<Result<_, String>>()?;
    stage_scene_resources::attach(input, &mut outputs)?;
    apply_canvas_views(input, &canvas_configs, &mut outputs)?;
    crate::key_fill::apply_scenes(
        &screen_config_value,
        &input.output_canvas_sizes,
        &mut outputs,
    )?;
    Ok(outputs)
}

fn apply_canvas_views(
    input: &OutputSceneStateInput,
    canvas_configs: &BTreeMap<String, ScreenCanvasConfig>,
    outputs: &mut Map<String, Value>,
) -> Result<(), String> {
    for (screen_id, config) in canvas_configs {
        if let Some(view) = &config.view {
            let follower_value = outputs
                .get(screen_id)
                .ok_or_else(|| format!("Output scene for '{screen_id}' was not built."))?;
            let follower: OutputScene = serde_json::from_value(follower_value.clone())
                .map_err(|error| format!("Output scene for '{screen_id}' is invalid: {error}"))?;
            let source_config = canvas_configs
                .get(&view.source_screen_id)
                .expect("validated canvas sources must exist");
            let source_value = outputs.get(&view.source_screen_id).ok_or_else(|| {
                format!(
                    "Output scene for '{}' was not built.",
                    view.source_screen_id
                )
            })?;
            let mut scene: OutputScene =
                serde_json::from_value(source_value.clone()).map_err(|error| {
                    format!(
                        "Output scene for '{}' is invalid: {error}",
                        view.source_screen_id
                    )
                })?;
            let canvas = match source_config.size {
                Some(size) => size,
                None => crate::output_canvas::parse_output_size(
                    input.output_canvas_sizes.get(&view.source_screen_id),
                )
                .map_err(|error| {
                    format!(
                        "Canvas source '{}' has no usable ordinary output size: {error}",
                        view.source_screen_id
                    )
                })?,
            };
            let region = view.region.unwrap_or(CanvasRegion {
                x: 0,
                y: 0,
                width: canvas.width,
                height: canvas.height,
            });
            scene.canvas_viewport = Some(CanvasViewport::new(canvas, region)?);
            scene.projector_correction = follower.projector_correction;
            scene.label = follower.label;
            scene.test_pattern = follower.test_pattern;
            scene.video_delay = follower.video_delay;
            outputs.insert(
                screen_id.clone(),
                serde_json::to_value(scene).map_err(|error| error.to_string())?,
            );
        } else if let Some(canvas) = config.size {
            let value = outputs
                .get(screen_id)
                .ok_or_else(|| format!("Output scene for '{screen_id}' was not built."))?;
            let mut scene: OutputScene = OutputScene::deserialize(value)
                .map_err(|error| format!("Output scene for '{screen_id}' is invalid: {error}"))?;
            scene.canvas_viewport = Some(CanvasViewport::new(
                canvas,
                CanvasRegion {
                    x: 0,
                    y: 0,
                    width: canvas.width,
                    height: canvas.height,
                },
            )?);
            outputs.insert(
                screen_id.clone(),
                serde_json::to_value(scene).map_err(|error| error.to_string())?,
            );
        }
    }
    Ok(())
}

/// The live slide identity and its effective transition: the slide's own, then the
/// presentation's, then the workspace default, as ProPresenter resolves them. Absent when
/// the screen shows no slide layer or nothing in reach defines a transition.
fn slide_transition_context(
    input: &OutputSceneStateInput,
    slide_visible: bool,
) -> Result<Option<Value>, String> {
    let item = input.live_item.as_ref();
    let defined = |value: Option<&Value>| value.filter(|value| !value.is_null()).cloned();
    let item_slides_define = item
        .and_then(|item| item.get("slides"))
        .and_then(Value::as_array)
        .is_some_and(|slides| {
            slides
                .iter()
                .any(|slide| defined(slide.get("transition")).is_some())
        });
    let presentation = defined(item.and_then(|item| item.get("transition")));
    let workspace = defined(input.slide_transition.as_ref());
    if !slide_visible || !(item_slides_define || presentation.is_some() || workspace.is_some()) {
        return Ok(None);
    }
    let transition = effective_slide_transition_value(
        input.live_slide.as_ref(),
        item,
        input.slide_transition.as_ref(),
    )?;
    Ok(Some(json!({
        "key": slide_entry_key(input),
        "transition": transition.unwrap_or(Value::Null),
    })))
}

pub(crate) fn effective_slide_transition_value(
    live_slide: Option<&Value>,
    live_item: Option<&Value>,
    workspace_transition: Option<&Value>,
) -> Result<Option<Value>, String> {
    let defined = |value: Option<&Value>| value.filter(|value| !value.is_null()).cloned();
    let transition = live_slide
        .and_then(|slide| defined(slide.get("transition")))
        .or_else(|| live_item.and_then(|item| defined(item.get("transition"))))
        .or_else(|| defined(workspace_transition));
    if let Some(value) = &transition {
        crate::native_transitions::validate(value)?;
    }
    Ok(transition)
}

/// The media, prop and message changes a screen animates. Each layer has a key that
/// changes when its content changes and the library transition the change plays: a
/// media selection's own transition, then the workspace default of its background or
/// foreground layer; a prop's own transition, then the props default; the messages
/// default. A layer is omitted when nothing in reach defines a transition.
fn layer_transition_context(
    input: &OutputSceneStateInput,
    background: &BackgroundOutput,
    props_visible: bool,
    messages_visible: bool,
) -> Result<Option<Value>, String> {
    let defaults = input
        .layer_transitions
        .as_ref()
        .filter(|value| !value.is_null());
    crate::native_transitions::validate_layer_defaults(defaults)?;
    // A cut default plays like no default, so it does not keep the layer tracked.
    let default = |layer: &str| {
        defaults
            .and_then(|value| value.get(layer))
            .filter(|value| matches!(crate::native_transitions::resolve(value), Ok(Some(_))))
            .cloned()
    };
    let mut layers = Map::new();

    let media_shown = !background.media_type.is_empty();
    let selection = crate::native_transitions::media_transition(
        input.background_transition.as_deref().unwrap_or("none"),
    )?;
    let media_defined = default("background").is_some()
        || default("foreground").is_some()
        || selection.is_some()
        || input.media.iter().any(|media| {
            media
                .get("backgroundTransition")
                .and_then(Value::as_str)
                .is_some_and(|value| value != "none")
        });
    if background.visible && media_defined {
        let transition = selection
            .or_else(|| default(&background.behavior))
            .filter(|_| media_shown);
        layers.insert(
            "media".into(),
            json!({
                "key": media_shown.then(|| json!([background.media_url, background.selection_id])),
                "transition": transition,
            }),
        );
    }

    if props_visible
        && (default("props").is_some()
            || input
                .props
                .iter()
                .any(|prop| prop.get("transition").is_some_and(|value| !value.is_null())))
    {
        let mut key = Vec::new();
        let mut transitions = Map::new();
        for prop in input
            .props
            .iter()
            .filter(|prop| prop.get("slide").is_some())
        {
            let Some(id) = prop.get("id").and_then(Value::as_str) else {
                continue;
            };
            if !input.live_prop_ids.iter().any(|live| live == id) {
                continue;
            }
            let transition = prop
                .get("transition")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| default("props"));
            if let Some(value) = &transition {
                crate::native_transitions::validate(value)?;
            }
            key.push(json!(id));
            transitions.insert(id.to_string(), transition.unwrap_or(Value::Null));
        }
        layers.insert(
            "props".into(),
            json!({ "key": key, "transitions": transitions }),
        );
    }

    if let Some(transition) = default("messages").filter(|_| messages_visible) {
        let shown =
            input._active_message_id.is_some() && !input.active_message_text.trim().is_empty();
        layers.insert(
            "messages".into(),
            json!({
                "key": input._active_message_instance_id.as_ref().filter(|_| shown),
                "transition": transition,
            }),
        );
    }
    Ok((!layers.is_empty()).then_some(Value::Object(layers)))
}

/// The live slide occurrence: its presentation item plus cue instance or slide id.
fn slide_entry_key(input: &OutputSceneStateInput) -> Option<String> {
    input.live_slide.as_ref().map(|slide| {
        format!(
            "{}:{}",
            input
                .live_item
                .as_ref()
                .and_then(|item| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default(),
            input
                .live_cue_instance_id
                .as_deref()
                .or_else(|| slide.get("id").and_then(Value::as_str))
                .unwrap_or_default()
        )
    })
}

fn object_build_entry_key(input: &OutputSceneStateInput) -> Option<String> {
    Some(format!(
        "{}:{}",
        slide_entry_key(input)?,
        input._timeline_started_at_ms?
    ))
}

fn announcements_input<'a>(
    input: &OutputSceneStateInput<'a>,
    visible: bool,
) -> OutputSceneStateInput<'a> {
    let mut routed = input.clone();
    let player = input.announcements.as_ref().filter(|_| visible);
    routed.live_item = player.and_then(|value| value.get("item")).cloned();
    routed.live_slide = player.and_then(|value| value.get("slide")).cloned();
    routed.live_slide_id = routed
        .live_slide
        .as_ref()
        .and_then(|slide| string_field(slide, "id"));
    routed.live_cue_instance_id = routed
        .live_slide
        .as_ref()
        .and_then(|slide| string_field(slide, "arrangementInstanceId"));
    routed.live_playlist_id = None;
    routed.live_playlist_entry_id = None;
    routed.live_build_step = 0.0;
    routed.live_build_step_started_at_ms_by_step.clear();
    routed._timeline_started_at_ms = None;
    routed.live_prop_ids.clear();
    routed.background_id = None;
    routed.background_selection_id = player.and_then(|value| value.get("selectionId")).cloned();
    routed.background_playback_state = player.map(|value| json!({
        "selectionId": value["selectionId"], "clockRevision": value["clockRevision"], "currentTime": value["currentTime"],
        "capturedAt": value["capturedAt"], "status": if value["paused"] == true { "paused" } else { "playing" }
    }));
    routed.selected_audio = None;
    routed.active_message_text.clear();
    routed.transport_playback_paused = Some(BTreeMap::from([
        (
            "media".to_string(),
            player.is_none_or(|value| value["paused"] == true),
        ),
        ("music".to_string(), true),
    ]));
    routed
}

fn build_screen_scene(
    input: &OutputSceneStateInput,
    screen_id: &str,
    config: &OutputScreenConfigInput,
    live_prop_slides: Option<&[&Value]>,
) -> Result<OutputScene, String> {
    let mut scene = build_screen_scene_base(input, screen_id, config, live_prop_slides)?;
    if config.screen_type.as_deref() != Some("stage")
        && scene.content_destination.as_deref() != Some("announcements")
        && announcements_visible(input, screen_id, config)
        && input
            .announcements
            .as_ref()
            .is_some_and(|player| player["slide"].is_object())
    {
        let mut config = config.clone();
        config._other = Map::from_iter([("contentDestination".into(), json!("announcements"))]);
        config.look_layers = json!({
            "slide":true, "media":scene.look_layers.media, "props":false,
            "videoInput":false, "messages":false, "music":false,
        })
        .as_object()
        .cloned();
        if let Some(themes) = &mut config.look_themes {
            themes.remove("maskId");
        }
        let mut overlay = build_screen_scene_base(input, screen_id, &config, None)?;
        overlay.background_css = Some("transparent".into());
        if let Some(slide) = overlay.presentation_theme_slide.as_mut() {
            slide["backgroundEnabled"] = json!(false);
        }
        scene.announcement_overlay = Some(Box::new(overlay));
    }
    Ok(scene)
}

fn announcements_visible(
    input: &OutputSceneStateInput,
    screen_id: &str,
    config: &OutputScreenConfigInput,
) -> bool {
    read_look_layer(config.look_layers.as_ref(), "announcements")
        || input
            .announcements
            .as_ref()
            .is_some_and(|player| player["visibleScreenId"] == screen_id)
}

fn build_screen_scene_base(
    input: &OutputSceneStateInput,
    screen_id: &str,
    config: &OutputScreenConfigInput,
    live_prop_slides: Option<&[&Value]>,
) -> Result<OutputScene, String> {
    let test_pattern = config
        ._other
        .get("testPattern")
        .filter(|value| !value.is_null())
        .cloned();
    if let Some(pattern) = &test_pattern {
        crate::test_pattern::validate(pattern)?;
    }
    if config
        ._other
        .get("contentDestination")
        .is_some_and(|value| !matches!(value.as_str(), Some("main" | "announcements")))
    {
        return Err(format!(
            "Screen '{screen_id}' content destination must be main or announcements."
        ));
    }
    let routed;
    let input = if config.screen_type.as_deref() != Some("stage")
        && config
            ._other
            .get("contentDestination")
            .and_then(Value::as_str)
            == Some("announcements")
    {
        routed = announcements_input(input, announcements_visible(input, screen_id, config));
        &routed
    } else {
        input
    };
    let look_layers = read_look_layers(config.look_layers.as_ref());
    let raw_look_themes = read_look_themes(config.look_themes.as_ref());
    let is_stage = config.screen_type.as_deref() == Some("stage");
    let content_theme_id = resolve_content_theme_id(input);
    let effective_theme_id = if raw_look_themes.presentation_theme_id.is_empty() {
        content_theme_id.unwrap_or_default()
    } else {
        raw_look_themes.presentation_theme_id.clone()
    };
    let look_themes = OutputSceneLookThemes {
        mask_id: raw_look_themes.mask_id,
        presentation_theme_id: effective_theme_id.clone(),
    };

    let stage_slide = if is_stage {
        resolve_stage_layout(
            config.stage_layout_id.as_deref(),
            screen_id,
            &input.stage_screen_assignments,
            &input.stage_layouts,
        )
        .and_then(|layout| layout.get("slide"))
        .cloned()
    } else {
        None
    };

    let has_stage_layout = stage_slide.is_some();
    let stage_background = stage_slide.as_ref().and_then(|slide| {
        (slide.get("backgroundEnabled").and_then(Value::as_bool) != Some(false))
            .then(|| slide.get("background").cloned())
            .flatten()
    });

    let mut presentation_theme_slide = if !has_stage_layout
        && look_layers.slide
        && input.live_slide.is_some()
    {
        let theme = if raw_look_themes.presentation_theme_id.is_empty() {
            effective_theme_id
                .split_once("::")
                .and_then(|(theme_id, slide_id)| {
                    input
                        .themes
                        .iter()
                        .find(|theme| theme["id"].as_str() == Some(theme_id))
                        .and_then(|theme| theme["slides"].as_array())
                        .and_then(|slides| {
                            slides
                                .iter()
                                .find(|slide| slide["id"].as_str() == Some(slide_id))
                        })
                        .cloned()
                })
                .or_else(|| {
                    input
                        .live_slide
                        .as_ref()?
                        .get("presentationThemeSnapshot")
                        .cloned()
                })
        } else {
            resolve_look_theme_slide(&input.themes, &effective_theme_id)
        };
        let slide = input.live_slide.as_ref().expect("Live slide is present.");
        if slide.get("primaryTextLayerId").is_some() || slide.get("presentationThemeId").is_some() {
            let mut composed = crate::slide_composition::compose_slide(slide, theme);
            composed
                .as_object_mut()
                .expect("Composed slide is an object.")
                .insert("buildStep".to_string(), json!(input.live_build_step));
            Some(composed)
        } else {
            theme
                .map(|mut theme| {
                    theme["buildStep"] = json!(input.live_build_step);
                    theme
                })
                .or_else(|| fallback_live_slide(input.live_slide.as_ref(), input.live_build_step))
        }
    } else {
        None
    };
    let text = if !has_stage_layout && look_layers.slide {
        input
            .live_slide
            .as_ref()
            .map(|slide| string_field(slide, "content").or_else(|| string_field(slide, "text")))
            .flatten()
            .unwrap_or_default()
    } else {
        String::new()
    };

    let props_visible = !is_stage
        && look_layers.props
        && config
            ._other
            .get("contentDestination")
            .and_then(Value::as_str)
            != Some("announcements");
    let prop_slides = if !props_visible {
        Vec::new()
    } else {
        live_prop_slides
            .expect("visible audience prop slides are selected before building screens")
            .iter()
            .map(|slide| (*slide).clone())
            .collect()
    };

    let mask_slide = if is_stage || look_themes.mask_id.is_empty() {
        None
    } else {
        input
            .masks
            .iter()
            .find(|mask| {
                mask.get("id").and_then(Value::as_str) == Some(look_themes.mask_id.as_str())
            })
            .and_then(|mask| mask.get("slide"))
            .cloned()
    };

    let next_slide = next_slide_for_live_item(input);
    if input
        .background_behavior
        .as_deref()
        .is_some_and(|value| !matches!(value, "background" | "foreground"))
    {
        return Err("Background behavior must be background or foreground.".into());
    }
    let next_arrangement_slide = next_arrangement_slide_for_live_item(input);
    if let Some(effects_override) = input
        .background_effects_override
        .as_ref()
        .filter(|value| !value.is_null())
    {
        let object = effects_override
            .as_object()
            .filter(|object| object.len() == 2)
            .ok_or("backgroundEffectsOverride requires selectionId and chain.")?;
        if !object.get("selectionId").is_some_and(Value::is_number) {
            return Err("backgroundEffectsOverride requires a numeric selectionId.".into());
        }
        crate::native_media_effects::validate_chain(
            object
                .get("chain")
                .ok_or("backgroundEffectsOverride requires a chain.")?,
        )?;
    }
    if let Some(value) = input
        .background_transport_override
        .as_ref()
        .filter(|value| !value.is_null())
    {
        let object = value
            .as_object()
            .ok_or("Media transport override must be an object.")?;
        if object
            .keys()
            .any(|key| !["selectionId", "transport", "behavior"].contains(&key.as_str()))
            || !object.get("selectionId").is_some_and(Value::is_number)
        {
            return Err("Media transport override requires an exact numeric selection id.".into());
        }
        crate::output_playback::validate_media_transport(
            object
                .get("transport")
                .ok_or("Media transport override requires transport settings.")?,
        )?;
        if object
            .get("behavior")
            .is_some_and(|value| !matches!(value.as_str(), Some("background" | "foreground")))
        {
            return Err(
                "Media transport override behavior must be background or foreground.".into(),
            );
        }
    }
    for media in &input.media {
        validate_media_color_effects(media)?;
        if let Some(effect) = media.get("mediaDesaturate") {
            if !matches!(
                media.get("type").and_then(Value::as_str),
                Some("image") | Some("video")
            ) || !effect.is_boolean()
            {
                return Err(
                    "Desaturation requires an image/video asset and a boolean setting.".into(),
                );
            }
        }
    }
    let background = build_background_output(input, !has_stage_layout && look_layers.media);
    if uses_slide_media_background(input) && !background.media_type.is_empty() {
        if let Some(slide) = presentation_theme_slide
            .as_mut()
            .and_then(Value::as_object_mut)
        {
            slide.insert("backgroundEnabled".into(), json!(false));
        }
    }
    if let Some(transport) = &background.transport {
        if background.media_type != "video" {
            return Err("Media transport settings require a video source.".into());
        }
        crate::output_playback::validate_media_transport(transport)?;
    }
    let selected_audio = input.selected_audio.as_ref();
    let audio_library =
        if let Some(playlist) = selected_audio.and_then(|audio| audio.get("playlistId")) {
            let audio = selected_audio.ok_or("Audio playlist requires a selected asset.")?;
            Some(
                input
                    .media_libraries
                    .iter()
                    .find(|library| {
                        playlist.is_string()
                            && library["id"] == *playlist
                            && library["mediaType"] == "audio"
                            && library["type"] == "playlist"
                            && (audio["libraryId"] == library["id"]
                                || library["itemFiles"]
                                    .as_array()
                                    .is_some_and(|files| files.contains(&audio["assetFile"])))
                    })
                    .ok_or("Selected audio playlist is invalid or does not contain the asset.")?,
            )
        } else {
            input.media_libraries.iter().find(|library| {
                selected_audio.is_some_and(|audio| audio["libraryId"] == library["id"])
            })
        };
    if let Some(routing) = selected_audio.and_then(|audio| audio.get("audioRouting")) {
        crate::native_audio::validate_audio_routing(routing)?;
    }
    let (audio_url, audio_path) = if look_layers.music {
        (
            string_field(selected_audio.unwrap_or(&Value::Null), "sourceUrl").unwrap_or_default(),
            string_field(selected_audio.unwrap_or(&Value::Null), "sourcePath").unwrap_or_default(),
        )
    } else {
        (String::new(), String::new())
    };

    let stage_message = if is_stage {
        input.stage_message.clone()
    } else {
        None
    };
    let linked_text_context =
        build_linked_text_context(input, next_slide.as_ref(), next_arrangement_slide.as_ref())?;
    let label = config
        .label
        .as_deref()
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| if is_stage { "Stage" } else { "Audience" }.to_string());

    let slide_visible = !has_stage_layout && look_layers.slide;
    let slide_transition = slide_transition_context(input, slide_visible)?;
    let layer_transitions = layer_transition_context(
        input,
        &background,
        props_visible,
        !is_stage && look_layers.messages,
    )?;
    let video_input_slide =
        if !is_stage && look_layers.video_input && input.live_video_input_id.is_some() {
            let canvas = input
                .output_canvas_sizes
                .get(screen_id)
                .ok_or("Active video input requires output canvas dimensions.")?;
            let dimension = |key| {
                canvas
                    .get(key)
                    .and_then(Value::as_u64)
                    .and_then(|size| u32::try_from(size).ok())
                    .filter(|size| *size > 0)
                    .ok_or("Video input canvas dimensions must be positive integers.")
            };
            crate::video_input::scene(
                &input.video_inputs,
                input.live_video_input_id.as_deref(),
                dimension("width")?,
                dimension("height")?,
            )?
        } else {
            None
        };
    let mut scene = OutputScene {
        output_frozen: input.output_frozen,
        key_fill_signal: None,
        key_fill_source_id: None,
        key_fill_partner_id: None,
        projector_correction: config.projector_correction,
        canvas_viewport: None,
        slide_transition,
        layer_transitions,
        look_transition: config
            ._other
            .get("lookTransition")
            .filter(|value| value.is_object())
            .cloned(),
        slide_entry_key: slide_entry_key(input).filter(|_| slide_visible),
        ccli_slide: (!is_stage && slide_visible)
            .then(|| ccli_slide_for_live_item(input))
            .flatten(),
        object_build_entry_key: object_build_entry_key(input).filter(|_| slide_visible),
        timeline_started_at_ms: input._timeline_started_at_ms.filter(|_| slide_visible),
        object_build_step_started_at_ms_by_step: if slide_visible
            && object_build_entry_key(input).is_some()
        {
            input.live_build_step_started_at_ms_by_step.clone()
        } else {
            BTreeMap::new()
        },
        test_pattern,
        content_destination: if is_stage {
            None
        } else {
            config
                ._other
                .get("contentDestination")
                .and_then(Value::as_str)
                .map(str::to_string)
        },
        label: Some(label),
        text: Some(text),
        look_layers,
        look_themes,
        presentation_theme_slide,
        announcement_overlay: None,
        video_input_slide,
        prop_slides: Some(prop_slides),
        mask_slide,
        stage_slide,
        stage_message,
        message_text: Some(
            if read_look_layer(config.look_layers.as_ref(), "messages") {
                input.active_message_text.clone()
            } else {
                String::new()
            },
        ),
        message_slide: if input._active_message_id.is_some()
            && !input.active_message_text.trim().is_empty()
            && read_look_layer(config.look_layers.as_ref(), "messages")
        {
            match input
                .active_message_definition
                .as_ref()
                .and_then(|message| message.get("themeId"))
                .and_then(Value::as_str)
            {
                Some(id) if !id.is_empty() => {
                    let slides = input
                        .themes
                        .iter()
                        .find(|theme| theme.get("id").and_then(Value::as_str) == Some(id))
                        .and_then(|theme| theme.get("slides"))
                        .and_then(Value::as_array)
                        .filter(|slides| !slides.is_empty())
                        .ok_or_else(|| format!("Message theme is unavailable: {id}"))?;
                    let slide_id = input
                        .active_message_definition
                        .as_ref()
                        .and_then(|message| message.get("themeSlideId"))
                        .and_then(Value::as_str);
                    Some(
                        match slide_id {
                            Some(slide_id) => slides.iter().find(|slide| {
                                slide.get("id").and_then(Value::as_str) == Some(slide_id)
                            }),
                            None => slides.first(),
                        }
                        .cloned()
                        .ok_or_else(|| {
                            format!("Message theme slide is unavailable: {id}/{slide_id:?}")
                        })?,
                    )
                }
                _ => None,
            }
        } else {
            None
        },
        linked_text_context: Some(linked_text_context),
        video_delay: match crate::output_delay::frames(config._other.get("videoDelay"))? {
            0 => None,
            frames => Some(frames),
        },
        background_css: if stage_background.is_some() {
            None
        } else {
            Some(background.css)
        },
        background: stage_background,
        background_media_type: Some(background.media_type),
        background_media_hardware_decoding: background.hardware_decoding,
        background_desaturate: background.desaturate,
        background_brightness: background.brightness,
        background_contrast: background.contrast,
        background_hue: background.hue,
        background_saturation: background.saturation,
        background_effects: background.effects,
        background_media_path: Some(background.media_path),
        background_media_url: Some(background.media_url),
        background_selection_id: Some(background.selection_id),
        background_behavior: Some(background.behavior),
        background_playback_state: background.playback_state,
        background_transport: background.transport,
        background_scale: Some(background.scale),
        background_playback: Some(background.playback),
        background_end_behavior: Some(background.end_behavior),
        background_transition: Some(background.transition),
        background_audio_routing: background.audio_routing,
        audio_url: Some(audio_url),
        audio_id: selected_audio
            .and_then(|audio| audio.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        audio_selection_id: input.audio_selection_id,
        audio_path: Some(audio_path),
        audio_settings: input.audio_settings.clone(),
        audio_inputs: if input.audio_inputs.as_array().is_some_and(Vec::is_empty) {
            None
        } else {
            Some(input.audio_inputs.clone())
        },
        audio_input_context: audio_input_context(input),
        music_loop: selected_audio
            .and_then(|audio| audio.get("audioPlayback"))
            .and_then(Value::as_str)
            .map(|playback| matches!(playback, "loop" | "loop-count" | "loop-time"))
            .filter(|loops| *loops && read_look_layer(config.look_layers.as_ref(), "music")),
        music_loop_count: selected_audio
            .filter(|audio| {
                audio["audioPlayback"] == "loop-count"
                    && read_look_layer(config.look_layers.as_ref(), "music")
            })
            .and_then(|audio| audio["audioLoopCount"].as_u64()),
        music_loop_time: selected_audio
            .filter(|audio| {
                audio["audioPlayback"] == "loop-time"
                    && read_look_layer(config.look_layers.as_ref(), "music")
            })
            .and_then(|audio| audio["audioLoopTime"].as_f64()),
        music_crossfade_seconds: audio_library
            .and_then(|library| library["audioCrossfadeSeconds"].as_f64()),
        music_audio_routing: if read_look_layer(config.look_layers.as_ref(), "music") {
            selected_audio
                .and_then(|audio| audio.get("audioRouting"))
                .cloned()
        } else {
            None
        },
        transport_playback_paused: input.transport_playback_paused.clone(),
    };
    if let Some(color) = config._other.get("screenColor").and_then(Value::as_str) {
        if scene
            .background_media_path
            .as_deref()
            .is_none_or(str::is_empty)
            && scene
                .background_media_url
                .as_deref()
                .is_none_or(str::is_empty)
        {
            scene.background_css = Some(color.to_string());
        }
    }
    if input.ignore_background_colors {
        strip_background_colors(&mut scene);
    }
    Ok(scene)
}

/// Drops presentation, slide, prop, mask and message background colors so only media shows behind content.
fn strip_background_colors(scene: &mut OutputScene) {
    if scene.background_css.is_some() {
        scene.background_css = Some("#000000".to_string());
    }
    scene.background = None;
    let slides = [
        scene.presentation_theme_slide.as_mut(),
        scene.video_input_slide.as_mut(),
        scene.mask_slide.as_mut(),
        scene.stage_slide.as_mut(),
        scene.message_slide.as_mut(),
    ];
    for slide in slides
        .into_iter()
        .flatten()
        .chain(scene.prop_slides.iter_mut().flatten())
    {
        if let Some(slide) = slide.as_object_mut() {
            slide.insert("backgroundEnabled".to_string(), json!(false));
        }
    }
}

struct BackgroundOutput {
    visible: bool,
    audio_routing: Option<Value>,
    effects: Option<Value>,
    desaturate: Option<bool>,
    brightness: Option<f64>,
    contrast: Option<f64>,
    hue: Option<f64>,
    saturation: Option<f64>,
    transport: Option<Value>,
    css: String,
    media_type: String,
    hardware_decoding: Option<String>,
    media_path: String,
    media_url: String,
    selection_id: Value,
    behavior: String,
    playback_state: Option<Value>,
    scale: String,
    playback: String,
    end_behavior: String,
    transition: String,
}

fn audio_input_context(input: &OutputSceneStateInput) -> Option<Value> {
    if input.audio_inputs.as_array().is_some_and(Vec::is_empty) {
        return None;
    }
    let audience_layer = |layer: &str| {
        input.screen_config.iter().any(|(id, config)| {
            if config.screen_type.as_deref() == Some("stage")
                || id.starts_with("stage-")
                || config._other.get("enabled") == Some(&json!(false))
            {
                return false;
            }
            let source = effective_canvas_source_config(input, config);
            read_look_layer(source.look_layers.as_ref(), layer)
        })
    };
    let video_input_id = input
        .live_video_input_id
        .as_deref()
        .filter(|_| audience_layer("videoInput"));
    let foreground = build_background_output(input, audience_layer("media"));
    let foreground_path = (foreground.behavior == "foreground" && foreground.media_type == "video")
        .then_some(foreground.media_path);
    Some(json!({"videoInputId":video_input_id,"foregroundPath":foreground_path}))
}

fn effective_canvas_source_config<'a>(
    input: &'a OutputSceneStateInput,
    config: &'a OutputScreenConfigInput,
) -> &'a OutputScreenConfigInput {
    let source_screen_id = config
        ._other
        .get("canvasView")
        .filter(|value| !value.is_null())
        .and_then(|view| view.get("sourceScreenId"))
        .and_then(Value::as_str);
    source_screen_id
        .and_then(|source_screen_id| input.screen_config.get(source_screen_id))
        .unwrap_or(config)
}

fn uses_slide_media_background(input: &OutputSceneStateInput) -> bool {
    input.live_slide.as_ref().is_some_and(|slide| {
        slide["mediaBackgroundOnly"] == true
            && string_field(slide, "mediaBackgroundId").is_some_and(|id| !id.is_empty())
    })
}

fn build_background_output(input: &OutputSceneStateInput, media_visible: bool) -> BackgroundOutput {
    if !media_visible {
        return BackgroundOutput {
            visible: false,
            audio_routing: None,
            effects: None,
            desaturate: None,
            brightness: None,
            contrast: None,
            hue: None,
            saturation: None,
            transport: None,
            css: "#000".to_string(),
            media_type: String::new(),
            hardware_decoding: None,
            media_path: String::new(),
            media_url: String::new(),
            selection_id: json!(""),
            behavior: "background".to_string(),
            playback_state: None,
            scale: "fit".to_string(),
            playback: "stop".to_string(),
            end_behavior: "hold".to_string(),
            transition: "none".to_string(),
        };
    }

    let background_id = input
        .live_slide
        .as_ref()
        .and_then(|slide| string_field(slide, "mediaBackgroundId"))
        .filter(|id| !id.is_empty())
        .or_else(|| input.background_id.clone());
    let (default_scale, default_playback, default_end_behavior) = (
        non_empty_or(&input.background_scale, "fit"),
        non_empty_or(
            input.background_playback.as_deref().unwrap_or_default(),
            "stop",
        ),
        non_empty_or(&input.background_end_behavior, "hold"),
    );
    let selection_id = input
        .background_selection_id
        .clone()
        .unwrap_or_else(|| json!(""));
    let playback_state = input
        .background_playback_state
        .as_ref()
        .filter(|state| match input.background_selection_id.as_ref() {
            Some(selection_id) => state
                .get("selectionId")
                .is_some_and(|state_id| state_id == selection_id),
            None => state.get("selectionId").is_none_or(Value::is_null),
        })
        .cloned();

    let mut css = "#000".to_string();
    let mut transport = None;
    let mut effects = None;
    let mut desaturate = None;
    let mut brightness = None;
    let mut contrast = None;
    let mut hue = None;
    let mut saturation = None;
    let mut audio_routing = None;
    let mut media_type = String::new();
    let mut media_path = String::new();
    let mut media_url = String::new();
    let mut behavior = "foreground".to_string();
    let mut scale = default_scale;
    let mut playback = default_playback;
    let mut end_behavior = default_end_behavior;

    if background_id.as_deref() == Some("__presenter-orbit-logo") {
        if !input.logo_image_url.is_empty() {
            media_type = "image".to_string();
            media_path = input.logo_image_path.clone();
            media_url = input.logo_image_url.clone();
        }
    } else if let Some(media) = background_id.as_deref().and_then(|id| {
        input.media.iter().find(|media| {
            media.get("id").and_then(Value::as_str) == Some(id)
                && matches!(
                    media.get("type").and_then(Value::as_str),
                    Some("image") | Some("video")
                )
                && string_field(media, "sourceUrl").is_some_and(|source_url| !source_url.is_empty())
        })
    }) {
        behavior = if string_field(media, "backgroundBehavior").as_deref() == Some("background") {
            "background".to_string()
        } else {
            "foreground".to_string()
        };
        media_type = string_field(media, "type").unwrap_or_default();
        if background_id == input.background_id {
            if let Some(value) = &input.background_behavior {
                behavior = value.clone();
            }
        }
        audio_routing = media.get("audioRouting").cloned();
        let transport_override = input
            .background_transport_override
            .as_ref()
            .filter(|value| {
                background_id == input.background_id
                    && value.get("selectionId").and_then(Value::as_f64).is_some()
                    && value.get("selectionId").and_then(Value::as_f64)
                        == input
                            .background_selection_id
                            .as_ref()
                            .and_then(Value::as_f64)
            });
        transport = transport_override
            .and_then(|value| value.get("transport"))
            .or_else(|| media.get("mediaTransport"))
            .cloned();
        if let Some(value) = transport_override
            .and_then(|value| value.get("behavior"))
            .and_then(Value::as_str)
        {
            behavior = value.to_string();
        }
        desaturate = media.get("mediaDesaturate").and_then(Value::as_bool);
        // A media action's own chain replaces the asset's while its selection is current.
        effects = input
            .background_effects_override
            .as_ref()
            .filter(|value| {
                value.get("selectionId").and_then(Value::as_f64).is_some()
                    && value.get("selectionId").and_then(Value::as_f64)
                        == input
                            .background_selection_id
                            .as_ref()
                            .and_then(Value::as_f64)
            })
            .and_then(|value| value.get("chain"))
            .or_else(|| media.get("mediaEffects"))
            .filter(|chain| chain.as_array().is_some_and(|chain| !chain.is_empty()))
            .cloned();
        brightness = media.get("mediaBrightness").and_then(Value::as_f64);
        contrast = media.get("mediaContrast").and_then(Value::as_f64);
        hue = media.get("mediaHue").and_then(Value::as_f64);
        saturation = media.get("mediaSaturation").and_then(Value::as_f64);
        media_path = string_field(media, "sourcePath").unwrap_or_default();
        media_url = string_field(media, "sourceUrl").unwrap_or_default();
        scale = transport_override
            .and_then(|overrides| string_field(overrides, "scale"))
            .or_else(|| string_field(media, "backgroundScale").filter(|value| !value.is_empty()))
            .unwrap_or(scale);
        playback = string_field(media, "backgroundPlayback")
            .filter(|value| !value.is_empty())
            .unwrap_or(playback);
        end_behavior = string_field(media, "backgroundEndBehavior")
            .filter(|value| !value.is_empty())
            .unwrap_or(end_behavior);
    } else if let Some(background) = background_id.as_deref().and_then(|id| {
        input
            .backgrounds
            .iter()
            .find(|background| background.get("id").and_then(Value::as_str) == Some(id))
    }) {
        css = string_field(background, "css").unwrap_or_else(|| "#000".to_string());
    }

    if let Some(slide) = input
        .live_slide
        .as_ref()
        .filter(|slide| string_field(slide, "mediaBackgroundId").is_some_and(|id| !id.is_empty()))
    {
        if uses_slide_media_background(input) {
            behavior = "background".to_string();
        } else if let Some(value) = string_field(slide, "mediaBackgroundBehavior") {
            behavior = value;
        }
        if let Some(value) = string_field(slide, "mediaBackgroundScale") {
            scale = value;
        }
    }
    BackgroundOutput {
        visible: true,
        audio_routing,
        effects,
        desaturate,
        brightness,
        contrast,
        hue,
        saturation,
        transport,
        css,
        media_type,
        hardware_decoding: background_id
            .as_deref()
            .and_then(|id| input.media.iter().find(|media| media["id"] == id))
            .and_then(|media| string_field(media, "mediaHardwareDecoding")),
        media_path,
        media_url,
        selection_id,
        behavior,
        playback_state,
        scale,
        playback,
        end_behavior,
        transition: input
            .background_transition
            .clone()
            .unwrap_or_else(|| "none".to_string()),
    }
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn read_look_layers(value: Option<&Map<String, Value>>) -> OutputSceneLookLayers {
    OutputSceneLookLayers {
        music: read_look_layer(value, "music"),
        messages: read_look_layer(value, "messages"),
        props: read_look_layer(value, "props"),
        announcements: read_look_layer(value, "announcements"),
        slide: read_look_layer(value, "slide"),
        media: read_look_layer(value, "media"),
        video_input: read_look_layer(value, "videoInput"),
    }
}

fn read_look_layer(value: Option<&Map<String, Value>>, key: &str) -> bool {
    value
        .and_then(|layers| layers.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn read_look_themes(value: Option<&Map<String, Value>>) -> OutputSceneLookThemes {
    OutputSceneLookThemes {
        mask_id: value
            .and_then(|themes| themes.get("maskId"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        presentation_theme_id: value
            .and_then(|themes| themes.get("presentationThemeId"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

pub(crate) fn resolve_content_theme_id(input: &OutputSceneStateInput) -> Option<String> {
    let item_id = input
        .live_item
        .as_ref()
        .and_then(|item| item.get("id"))
        .and_then(Value::as_str)?;
    let slide = input.live_slide.as_ref()?.as_object()?;
    if let Some(theme_id) = explicit_theme_assignment(slide) {
        return Some(theme_id);
    }

    let playlist_id = input.live_playlist_id.as_deref().unwrap_or_default();
    if let Some(theme_id) = resolve_playlist_theme(&input.playlist, playlist_id) {
        return Some(theme_id);
    }

    input
        .libraries
        .iter()
        .find(|library| {
            library
                .get("itemIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(item_id)))
        })
        .and_then(|library| library.as_object())
        .and_then(explicit_theme_assignment)
}

fn resolve_playlist_theme(playlists: &[Value], playlist_id: &str) -> Option<String> {
    if playlist_id.is_empty() {
        return None;
    }
    let mut current_id = playlist_id.to_string();
    let mut visited = Vec::new();
    loop {
        if visited.iter().any(|id| id == &current_id) {
            return None;
        }
        visited.push(current_id.clone());
        let playlist = playlists.iter().find(|playlist| {
            playlist.get("id").and_then(Value::as_str) == Some(current_id.as_str())
        })?;
        if let Some(theme_id) = playlist.as_object().and_then(explicit_theme_assignment) {
            return Some(theme_id);
        }
        current_id = playlist
            .get("parentId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if current_id.is_empty() {
            return None;
        }
    }
}

fn explicit_theme_assignment(object: &Map<String, Value>) -> Option<String> {
    object
        .get("presentationThemeId")
        .map(|value| value.as_str().unwrap_or_default().to_string())
}

pub(crate) fn resolve_look_theme_slide(themes: &[Value], selection: &str) -> Option<Value> {
    if selection.is_empty() {
        return None;
    }
    let (theme_id, slide_id) = selection
        .split_once("::")
        .map_or((Some(selection), None), |(theme, slide)| {
            (Some(theme), Some(slide))
        });
    if let Some(theme_id) = theme_id {
        if let Some(theme) = themes
            .iter()
            .find(|theme| theme.get("id").and_then(Value::as_str) == Some(theme_id))
        {
            let slides = theme.get("slides").and_then(Value::as_array)?;
            if slide_id.is_none() {
                return slides.first().cloned();
            }
            return slides
                .iter()
                .find(|slide| slide.get("id").and_then(Value::as_str) == slide_id)
                .cloned()
                .or_else(|| slides.first().cloned());
        }
    }
    let slide_id = slide_id.unwrap_or(selection);
    themes.iter().find_map(|theme| {
        theme
            .get("slides")
            .and_then(Value::as_array)
            .and_then(|slides| {
                slides
                    .iter()
                    .find(|slide| slide.get("id").and_then(Value::as_str) == Some(slide_id))
                    .cloned()
            })
    })
}

fn fallback_live_slide(slide: Option<&Value>, build_step: f64) -> Option<Value> {
    let mut slide = slide?.as_object()?.clone();
    let layers = slide.get("layers").and_then(Value::as_array)?;
    if layers.is_empty() {
        return None;
    }
    slide.insert("buildStep".to_string(), json!(build_step));
    Some(Value::Object(slide))
}

pub fn resolve_stage_layout<'a>(
    configured_id: Option<&str>,
    screen_id: &str,
    assignments: &[Value],
    layouts: &'a [Value],
) -> Option<&'a Value> {
    let assigned_id = assignments
        .iter()
        .find(|assignment| assignment.get("screenId").and_then(Value::as_str) == Some(screen_id))
        .and_then(|assignment| assignment.get("layoutId"))
        .and_then(Value::as_str);
    let mut available = layouts.iter().filter(|layout| {
        layout["id"].as_str().is_some_and(|id| !id.is_empty()) && layout["slide"].is_object()
    });
    configured_id
        .into_iter()
        .chain(assigned_id)
        .find_map(|id| available.clone().find(|layout| layout["id"] == id))
        .or_else(|| available.next())
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CcliDisplayLocation {
    #[default]
    First,
    Last,
    FirstAndLast,
    All,
}

fn ccli_slide_for_live_item(input: &OutputSceneStateInput) -> Option<Value> {
    let item = input.live_item.as_ref()?;
    if input.ccli_display_enabled == Some(false)
        || crate::native_compositor::format_ccli_text(item.get("ccli")).is_empty()
    {
        return None;
    }
    let live_id = input.live_slide.as_ref()?.get("id")?.as_str()?;
    let slides = resolve_live_arrangement_slides(
        item,
        input.live_cue_instance_id.as_deref(),
        input.live_playlist_entry_id.as_deref(),
    )?;
    let active = live_slide_index(&slides, live_id, input.live_cue_instance_id.as_deref())?;
    let enabled = |slide: &Value| slide["disabled"] != true;
    let show = match input.ccli_display_location {
        CcliDisplayLocation::First => slides.iter().position(enabled) == Some(active),
        CcliDisplayLocation::Last => slides.iter().rposition(enabled) == Some(active),
        CcliDisplayLocation::FirstAndLast => {
            slides.iter().position(enabled) == Some(active)
                || slides.iter().rposition(enabled) == Some(active)
        }
        CcliDisplayLocation::All => true,
    };
    show.then(|| {
        input
            .ccli_template
            .clone()
            .unwrap_or_else(crate::ccli_template::default_slide)
    })
}

fn next_slide_for_live_item(input: &OutputSceneStateInput) -> Option<Value> {
    let item = input.live_item.as_ref()?.as_object()?;
    let current_id = input
        .live_slide
        .as_ref()?
        .get("id")
        .and_then(Value::as_str)?;
    let slides = item.get("slides")?.as_array()?;
    let index = slides
        .iter()
        .position(|slide| slide.get("id").and_then(Value::as_str) == Some(current_id))?;
    slides.get(index + 1).cloned()
}

fn next_arrangement_slide_for_live_item(input: &OutputSceneStateInput) -> Option<Value> {
    let item = input.live_item.as_ref()?;
    let slides = resolve_live_arrangement_slides(
        item,
        input.live_cue_instance_id.as_deref(),
        input.live_playlist_entry_id.as_deref(),
    )?;
    let current_id = input
        .live_slide
        .as_ref()?
        .get("id")
        .and_then(Value::as_str)?;
    let index = live_slide_index(&slides, current_id, input.live_cue_instance_id.as_deref())?;
    slides.get(index + 1).cloned()
}

fn next_enabled_slide_for_live_item(input: &OutputSceneStateInput) -> Option<Value> {
    let item = input.live_item.as_ref()?;
    let slides = resolve_live_arrangement_slides(
        item,
        input.live_cue_instance_id.as_deref(),
        input.live_playlist_entry_id.as_deref(),
    )
    .or_else(|| item.get("slides").and_then(Value::as_array).cloned())?;
    let current_id = input
        .live_slide
        .as_ref()?
        .get("id")
        .and_then(Value::as_str)?;
    let index = live_slide_index(&slides, current_id, input.live_cue_instance_id.as_deref())?;
    slides
        .into_iter()
        .skip(index + 1)
        .find(|slide| slide.get("disabled").and_then(Value::as_bool) != Some(true))
}

fn build_linked_text_context(
    input: &OutputSceneStateInput,
    next_slide: Option<&Value>,
    next_arrangement_slide: Option<&Value>,
) -> Result<Map<String, Value>, String> {
    let current_slide = input.live_slide.as_ref();
    let mut context = Map::new();
    let chart_path = match current_slide.and_then(|slide| slide.get("chordChart")) {
        Some(chart) => {
            let chart: crate::chord_chart::ChordChart = serde_json::from_value(chart.clone())
                .map_err(|error| format!("Invalid live chord chart: {error}"))?;
            let path = chart
                .page_paths
                .get(chart.page)
                .ok_or("Live chord chart page is outside the document.")?;
            if path.is_empty() {
                return Err("Live chord chart page path is required.".into());
            }
            json!(path)
        }
        None => Value::Null,
    };
    context.insert("chordChartPath".to_string(), chart_path);
    if current_slide
        .and_then(|slide| slide.get("chordChart"))
        .is_none()
        && [current_slide, input.live_item.as_ref()]
            .into_iter()
            .flatten()
            .any(|document| {
                document
                    .get("chordChartUrl")
                    .and_then(Value::as_str)
                    .is_some_and(|url| !url.is_empty())
            })
    {
        context.insert(
            "chordChartError".into(),
            json!("Imported chord chart references require explicit chart import."),
        );
    }
    let video = |source: &OutputSceneStateInput| {
        let background = build_background_output(source, true);
        if background.media_type != "video" {
            return Value::Null;
        }
        let mut snapshot = background
            .playback_state
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        snapshot.insert("selectionId".into(), background.selection_id);
        snapshot.insert("looping".into(), json!(background.playback == "loop"));
        Value::Object(snapshot)
    };
    context.insert(
        "videoCountdowns".into(),
        json!({
            "presentation": video(input),
            "announcements": video(&announcements_input(input, true))
        }),
    );
    context.insert(
        "videoInputActivity".into(),
        input
            .video_inputs
            .iter()
            .filter_map(|video| video.get("id").and_then(Value::as_str))
            .map(|id| {
                (
                    id.to_string(),
                    json!(input.live_video_input_id.as_deref() == Some(id)),
                )
            })
            .collect::<Map<_, _>>()
            .into(),
    );
    // A timer that waits for media has no end the scene can state, so it adds no countdown.
    if let (Some(slide), Some(started)) = (current_slide, input._timeline_started_at_ms) {
        let completion = crate::native_automation::build_relative_completion(slide, &input.media);
        if let Some(delay) = completion["delaySeconds"].as_f64().filter(|_| {
            completion["mediaEnd"] != true && completion.get("foregroundVideoPaths").is_none()
        }) {
            context.insert(
                "slideTimer".into(),
                json!({"dueAtMs": started + (delay * 1000.0).round() as i64}),
            );
        }
    }
    context.insert(
        "videoInputSources".into(),
        input
            .video_inputs
            .iter()
            .filter_map(|video| {
                let source = ["ndiSource", "cameraSource"]
                    .into_iter()
                    .find_map(|key| Some((key, video.get(key)?)))?;
                let scale = match video.get("scale").and_then(Value::as_str)? {
                    "fit" => "Scale to Fit",
                    "fill" => "Scale to Fill",
                    "stretch" => "Stretch to Fill",
                    _ => return None,
                };
                Some((
                    video.get("id").and_then(Value::as_str)?.to_string(),
                    json!({ source.0: source.1, "mediaScale": scale }),
                ))
            })
            .collect::<Map<_, _>>()
            .into(),
    );
    context.insert("playlistText".to_string(), build_playlist_text(input));
    if let Some(item) = input.live_item.as_ref().and_then(build_linked_item) {
        context.insert("item".to_string(), Value::Object(item));
    }
    if let Some(slide) =
        current_slide.and_then(|slide| build_colored_linked_slide(slide, &input.group_settings))
    {
        context.insert("slide".to_string(), Value::Object(slide));
    }
    if let Some(slide) =
        next_slide.and_then(|slide| build_colored_linked_slide(slide, &input.group_settings))
    {
        context.insert("nextSlide".to_string(), Value::Object(slide));
    }
    if let Some(slide) = next_arrangement_slide
        .and_then(|slide| build_colored_linked_slide(slide, &input.group_settings))
    {
        context.insert("nextArrangementSlide".to_string(), Value::Object(slide));
    }
    let item = input.live_item.as_ref();
    context.insert(
        "operatorNotes".to_string(),
        json!(string_field(item.unwrap_or(&Value::Null), "operatorNotes").unwrap_or_default()),
    );
    context.insert(
        "currentSlideText".to_string(),
        json!(linked_slide_text(current_slide)),
    );
    context.insert(
        "nextSlideText".to_string(),
        json!(linked_slide_text(next_slide)),
    );
    context.insert(
        "presentationName".to_string(),
        json!(string_field(item.unwrap_or(&Value::Null), "title").unwrap_or_default()),
    );
    context.insert(
        "slideNotes".to_string(),
        json!(string_field(current_slide.unwrap_or(&Value::Null), "notes").unwrap_or_default()),
    );
    context.insert(
        "nextSlideNotes".to_string(),
        json!(string_field(next_slide.unwrap_or(&Value::Null), "notes").unwrap_or_default()),
    );
    context.insert(
        "slideLabel".to_string(),
        json!(string_field(current_slide.unwrap_or(&Value::Null), "label").unwrap_or_default()),
    );
    context.insert(
        "nextSlideLabel".to_string(),
        json!(string_field(next_slide.unwrap_or(&Value::Null), "label").unwrap_or_default()),
    );
    let mut ccli = item
        .and_then(|item| item.get("ccli"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if input.ccli_display_enabled == Some(false) {
        ccli.insert("display".into(), Value::Bool(false));
    }
    if let Some(number) = &input.ccli_license_number {
        context.insert("ccliLicenseNumber".into(), json!(number));
    }
    context.insert("ccli".into(), Value::Object(ccli));
    context.insert(
        "stageMessageText".to_string(),
        json!(
            string_field(input.stage_message.as_ref().unwrap_or(&Value::Null), "text")
                .unwrap_or_default()
        ),
    );
    context.insert(
        "countdownTimers".to_string(),
        input.countdown_timers.clone(),
    );
    context.insert(
        "countdownTimerRuns".to_string(),
        input.countdown_timer_runs.clone(),
    );
    Ok(context)
}

#[derive(Debug, Clone)]
struct LinkedPlaylistEntry {
    entry_id: String,
    kind: &'static str,
    item_id: Option<String>,
    title: String,
    arrangement_id: Option<String>,
}

fn build_playlist_text(input: &OutputSceneStateInput) -> Value {
    let Some(playlist_id) = input.live_playlist_id.as_deref() else {
        return empty_object();
    };
    let Some(item_id) = input
        .live_item
        .as_ref()
        .and_then(|item| item.get("id"))
        .and_then(Value::as_str)
    else {
        return empty_object();
    };
    let Some(playlist) = input
        .playlist
        .iter()
        .find(|playlist| playlist.get("id").and_then(Value::as_str) == Some(playlist_id))
    else {
        return empty_object();
    };
    let entries = playlist
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(normalize_linked_playlist_entry)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let index = match input.live_playlist_entry_id.as_deref() {
        Some(entry_id) => entries.iter().position(|entry| entry.entry_id == entry_id),
        None => entries.iter().position(|entry| {
            entry.kind == "presentation" && entry.item_id.as_deref() == Some(item_id)
        }),
    };
    let Some(index) = index else {
        return empty_object();
    };

    let title_for_entry = |entry: Option<&LinkedPlaylistEntry>| {
        let Some(entry) = entry else {
            return String::new();
        };
        if entry.kind == "header" {
            return entry.title.clone();
        }
        entry
            .item_id
            .as_deref()
            .and_then(|id| {
                input
                    .library
                    .iter()
                    .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
            })
            .and_then(|item| string_field(item, "title"))
            .unwrap_or_default()
    };
    let current = entries.get(index);
    let next = entries
        .iter()
        .skip(index + 1)
        .find(|entry| entry.kind == "presentation");
    let title_with_arrangement = |entry: Option<&LinkedPlaylistEntry>| {
        let title = title_for_entry(entry);
        let item = entry
            .and_then(|entry| entry.item_id.as_deref())
            .and_then(|id| {
                input
                    .library
                    .iter()
                    .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
            });
        let id = entry
            .and_then(|entry| entry.arrangement_id.as_deref())
            .or_else(|| {
                item.and_then(|item| item.get("selectedArrangementId"))
                    .and_then(Value::as_str)
            });
        let name = item
            .and_then(|item| item.get("arrangements"))
            .and_then(Value::as_array)
            .and_then(|arrangements| {
                arrangements
                    .iter()
                    .find(|arrangement| arrangement.get("id").and_then(Value::as_str) == id)
            })
            .and_then(|arrangement| arrangement.get("name"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty());
        name.map(|name| format!("{title} ({name})"))
            .unwrap_or(title)
    };
    let current_header = entries[..=index]
        .iter()
        .rev()
        .find(|entry| entry.kind == "header");
    let next_header = entries
        .iter()
        .skip(index + 1)
        .find(|entry| entry.kind == "header");
    let parent_id = playlist
        .get("parentId")
        .and_then(Value::as_str)
        .filter(|parent_id| {
            input
                .playlist
                .iter()
                .any(|candidate| candidate.get("id").and_then(Value::as_str) == Some(parent_id))
        });
    let parent_title = parent_id
        .and_then(|parent_id| {
            input
                .playlist
                .iter()
                .find(|candidate| candidate.get("id").and_then(Value::as_str) == Some(parent_id))
        })
        .and_then(|parent| string_field(parent, "title"))
        .unwrap_or_default();

    json!({
        "Current": title_for_entry(current),
        "Next": title_for_entry(next),
        "Current With Arrangement": title_with_arrangement(current),
        "Next With Arrangement": title_with_arrangement(next),
        "Current Header": title_for_entry(current_header),
        "Next Header": title_for_entry(next_header),
        "Parent Playlist": parent_title,
    })
}

fn normalize_linked_playlist_entry(entry: &Value) -> Option<LinkedPlaylistEntry> {
    if entry["hidden"] == true {
        return None;
    }
    let entry_id = string_field(entry, "id").unwrap_or_default();
    let raw_type = string_field(entry, "type").unwrap_or_default();
    let kind = match raw_type.as_str() {
        "header" => "header",
        "placeholder" => "placeholder",
        "presentation" | "" => "presentation",
        _ => "presentation",
    };
    if kind == "presentation" {
        let item_id = first_non_empty_field(entry, &["itemId", "id"])?;
        return Some(LinkedPlaylistEntry {
            entry_id,
            kind,
            item_id: Some(item_id),
            title: String::new(),
            arrangement_id: string_field(entry, "arrangementId"),
        });
    }
    let title = first_non_empty_field(entry, &["title", "name"]).unwrap_or_else(|| {
        if kind == "header" {
            "Header"
        } else {
            "Placeholder"
        }
        .to_string()
    });
    Some(LinkedPlaylistEntry {
        entry_id,
        kind,
        item_id: None,
        title,
        arrangement_id: None,
    })
}

fn first_non_empty_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| string_field(value, key))
        .find(|value| !value.is_empty())
}

fn build_linked_item(item: &Value) -> Option<Map<String, Value>> {
    let object = item.as_object()?;
    let slides = object
        .get("slides")
        .and_then(Value::as_array)
        .map(|slides| {
            slides
                .iter()
                .filter_map(|slide| slide.get("id").cloned())
                .map(|id| json!({ "id": id }))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(Map::from_iter([
        (
            "title".to_string(),
            json!(string_field(item, "title").unwrap_or_default()),
        ),
        (
            "operatorNotes".to_string(),
            json!(string_field(item, "operatorNotes").unwrap_or_default()),
        ),
        ("slides".to_string(), Value::Array(slides)),
    ]))
}

fn build_colored_linked_slide(
    slide: &Value,
    groups: &BTreeMap<String, Value>,
) -> Option<Map<String, Value>> {
    let mut linked = build_linked_slide(slide)?;
    if let Some(color) = slide
        .get("groupId")
        .and_then(Value::as_str)
        .and_then(|id| groups.get(id))
        .and_then(|group| group.get("color"))
    {
        linked.insert("groupColor".to_string(), color.clone());
    }
    Some(linked)
}

fn build_linked_slide(slide: &Value) -> Option<Map<String, Value>> {
    let object = slide.as_object()?;
    let mut linked = Map::new();
    for key in [
        "id",
        "content",
        "text",
        "notes",
        "notesRuns",
        "notesTextStyle",
        "layers",
        "label",
        "groupId",
        "groupName",
        "groupColor",
        "labelColor",
    ] {
        if let Some(value) = object.get(key) {
            linked.insert(key.to_string(), value.clone());
        }
    }
    Some(linked)
}

fn linked_slide_text(slide: Option<&Value>) -> String {
    slide
        .and_then(|slide| string_field(slide, "content"))
        .filter(|text| !text.is_empty())
        .or_else(|| slide.and_then(|slide| string_field(slide, "text")))
        .unwrap_or_default()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Null => None,
        value => Some(value.to_string()),
    })
}

fn validate_scene(scene: &OutputScene, source_screen_id: &str) -> Result<(), String> {
    if let Some(overlay) = &scene.announcement_overlay {
        if overlay.announcement_overlay.is_some() {
            return Err("An announcement overlay cannot contain another overlay.".into());
        }
        validate_scene(overlay, source_screen_id)?;
    }
    if scene
        .music_crossfade_seconds
        .is_some_and(|seconds| !seconds.is_finite() || !(0.0..=10.0).contains(&seconds))
    {
        return Err("Music crossfade must be between 0 and 10 seconds.".into());
    }
    if scene
        .object_build_entry_key
        .as_deref()
        .is_some_and(|key| key.trim().is_empty())
    {
        return Err("Object build playback requires a nonempty occurrence key.".into());
    }
    if let Some(correction) = scene.projector_correction {
        correction.validate().map_err(|error| {
            format!("Output scene for '{source_screen_id}' has an invalid projector correction: {error}")
        })?;
    }
    if let Some(viewport) = scene.canvas_viewport {
        viewport.validate().map_err(|error| {
            format!("Output scene for '{source_screen_id}' has an invalid canvas viewport: {error}")
        })?;
    }
    if scene
        .background_brightness
        .is_some_and(|value| !value.is_finite() || !(-1.0..=1.0).contains(&value))
        || scene
            .background_contrast
            .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
        || scene
            .background_hue
            .is_some_and(|value| !value.is_finite() || !(-180.0..=180.0).contains(&value))
        || scene
            .background_saturation
            .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
    {
        return Err("Output media color adjustment is out of range.".into());
    }
    if let Some(transport) = &scene.background_transport {
        crate::output_playback::validate_media_transport(transport)?;
    }
    for routing in [&scene.music_audio_routing, &scene.background_audio_routing]
        .into_iter()
        .flatten()
    {
        crate::native_audio::validate_audio_routing(routing)?;
    }
    if let Some(slide) = scene.presentation_theme_slide.as_ref() {
        validate_slide(slide, source_screen_id, "presentationThemeSlide")?;
    }
    if let Some(slide) = scene.ccli_slide.as_ref() {
        crate::ccli_template::validate(slide)?;
    }
    if let Some(slide) = scene.video_input_slide.as_ref() {
        validate_slide(slide, source_screen_id, "videoInputSlide")?;
    }
    if let Some(slides) = scene.prop_slides.as_ref() {
        for (index, slide) in slides.iter().enumerate() {
            validate_slide(slide, source_screen_id, &format!("propSlides[{index}]"))?;
        }
    }
    if let Some(slide) = scene.mask_slide.as_ref() {
        validate_slide(slide, source_screen_id, "maskSlide")?;
    }
    if let Some(slide) = scene.stage_slide.as_ref() {
        validate_slide(slide, source_screen_id, "stageSlide")?;
    }
    let content = serde_json::to_value(scene).map_err(|error| error.to_string())?;
    crate::native_compositor::validate_object_links(&content)?;
    crate::native_compositor::validate_chord_chart(&content)?;
    crate::native_compositor::validate_stage_previews(&content)
}

fn validate_slide(value: &Value, source_screen_id: &str, field: &str) -> Result<(), String> {
    if value.is_object() {
        crate::native_compositor::validate_object_links(value)
    } else {
        Err(format!(
            "Output scene for '{source_screen_id}' has a non-object {field}."
        ))
    }
}

pub(crate) fn validate_media_color_effects(media: &Value) -> Result<(), String> {
    if let Some(chain) = media.get("mediaEffects") {
        if !matches!(
            media.get("type").and_then(Value::as_str),
            Some("image") | Some("video")
        ) {
            return Err("Media effects require an image/video asset.".into());
        }
        crate::native_media_effects::validate_chain(chain)?;
    }
    for key in [
        "mediaBrightness",
        "mediaContrast",
        "mediaHue",
        "mediaSaturation",
    ] {
        if let Some(effect) = media.get(key) {
            if !matches!(
                media.get("type").and_then(Value::as_str),
                Some("image") | Some("video")
            ) || !effect.as_f64().is_some_and(|value| {
                value.is_finite()
                    && (if key == "mediaBrightness" {
                        -1.0..=1.0
                    } else if key == "mediaHue" {
                        -180.0..=180.0
                    } else {
                        0.0..=2.0
                    })
                    .contains(&value)
            }) {
                return Err("Color adjustments require an image/video asset and a finite value within the effect range.".into());
            }
        }
    }
    Ok(())
}
