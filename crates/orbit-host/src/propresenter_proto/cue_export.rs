//! Writes the cue structure the importer reads back: presentation groups and arrangements
//! (`parse_groups`, `parse_arrangements`), cue completion settings and notes (`parse_cue`)
//! and the cue's other actions (`decode_action`).

use super::*;
use crate::output_arrangement::slide_section_id;
use std::collections::BTreeSet;

/// Set by the workspace exporter on a slide whose `mediaBackgroundId` resolved to a workspace
/// media item; holds that item with its `sourceUrl` replaced by the exported copy.
pub(crate) const MEDIA_BACKGROUND: &str = "exportMediaBackground";

pub(super) struct CueGroups {
    /// ProPresenter group UUID, name, color and the indexes of its slides, in cue order.
    groups: Vec<(String, String, Option<String>, Vec<usize>)>,
}

/// One ProPresenter group per run of slides sharing an Orbit section, so the master order is
/// unchanged; slides without any group stay outside every group, as ProPresenter allows.
pub(super) fn cue_groups(item: &Value) -> CueGroups {
    let slides = item["slides"].as_array().map_or(&[][..], Vec::as_slice);
    let scope = item["id"].as_str().unwrap_or("presentation");
    let mut groups: Vec<(String, String, Option<String>, Vec<usize>)> = Vec::new();
    let mut current: Option<(String, usize)> = None;
    let mut used = BTreeSet::new();
    for (index, slide) in slides.iter().enumerate() {
        let group_id = (0..=index)
            .rev()
            .find_map(|candidate| crate::output_arrangement::slide_group_id(&slides[candidate]));
        let explicit = slide["sectionId"].as_str();
        if explicit.is_none() && group_id.is_none() {
            current = None;
            continue;
        }
        let key = explicit
            .map(str::to_string)
            .unwrap_or_else(|| slide_section_id(slides, index));
        if let Some((_, group)) = current.as_ref().filter(|(previous, _)| *previous == key) {
            groups[*group].3.push(index);
            continue;
        }
        // An imported group keeps its identity even when its cues are not consecutive.
        if let Some(group) = explicit
            .and_then(|id| Uuid::parse_str(id).ok())
            .and_then(|id| groups.iter().position(|group| group.0 == id.to_string()))
        {
            groups[group].3.push(index);
            current = Some((key, group));
            continue;
        }
        current = Some((key.clone(), groups.len()));
        let id = Uuid::parse_str(&key)
            .ok()
            .map(|id| id.to_string())
            .filter(|id| used.insert(id.clone()))
            .unwrap_or_else(|| derived_uuid(&format!("{scope}/group/{key}/{index}")));
        let declared_groups = item["groups"].as_array().map_or(&[][..], Vec::as_slice);
        let declared = declared_groups
            .iter()
            .find(|group| group["sourceId"] == json!(key))
            .or_else(|| {
                declared_groups
                    .iter()
                    .find(|group| group_id.as_deref().is_some_and(|id| group["id"] == id))
            });
        let name = slide["groupName"]
            .as_str()
            .or_else(|| declared.and_then(|group| group["name"].as_str()))
            .map(str::to_string)
            .or(group_id)
            .unwrap_or_default();
        let color = slide["groupColor"]
            .as_str()
            .or_else(|| declared.and_then(|group| group["color"].as_str()))
            .map(str::to_string);
        groups.push((id, name, color, vec![index]));
    }
    // Groups referenced only by arrangements keep their identity.
    for group in item["groups"].as_array().into_iter().flatten() {
        if let Some(id) = group["sourceId"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
        {
            if used.insert(id.to_string()) {
                groups.push((
                    id.to_string(),
                    group["name"].as_str().unwrap_or_default().to_string(),
                    group["color"].as_str().map(str::to_string),
                    Vec::new(),
                ));
            }
        }
    }
    CueGroups { groups }
}

impl CueGroups {
    pub(super) fn encode(&self, cue_ids: &[Vec<u8>], document: &mut Vec<u8>) {
        for (id, name, color, slides) in &self.groups {
            let mut group = Vec::new();
            put_uuid(1, id, &mut group);
            put_string(2, name, &mut group);
            if let Some(color) = color {
                put_color(3, color, &mut group);
            }
            let mut cue_group = Vec::new();
            put_message(1, group, &mut cue_group);
            for index in slides {
                put_message(2, cue_ids[*index].clone(), &mut cue_group);
            }
            put_message(12, cue_group, document);
        }
    }

    /// The ProPresenter groups that play an Orbit arrangement section, or `None` when the
    /// section's slides are not exactly a set of whole groups.
    pub(super) fn section_groups(&self, item: &Value, section: &str) -> Option<Vec<&str>> {
        let slides = item["slides"].as_array().map_or(&[][..], Vec::as_slice);
        let members = (0..slides.len())
            .filter(|index| {
                slide_section_id(slides, *index) == section
                    || slides[*index]["sectionId"].as_str() == Some(section)
            })
            .collect::<BTreeSet<_>>();
        if members.is_empty() {
            // A section without slides plays nothing in Orbit; only a declared group keeps it.
            return Some(
                self.groups
                    .iter()
                    .filter(|group| group.3.is_empty() && group.0 == section)
                    .map(|group| group.0.as_str())
                    .collect(),
            );
        }
        let groups = self
            .groups
            .iter()
            .filter(|group| group.3.iter().any(|index| members.contains(index)))
            .collect::<Vec<_>>();
        let covered = groups
            .iter()
            .flat_map(|group| group.3.iter().copied())
            .collect::<BTreeSet<_>>();
        (covered == members).then(|| groups.iter().map(|group| group.0.as_str()).collect())
    }
}

pub(super) fn derived_uuid(scope: &str) -> String {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("presenter-orbit:{scope}").as_bytes(),
    )
    .to_string()
}

fn arrangement_id(item: &Value, id: &str) -> String {
    arrangement_uuid(item["id"].as_str().unwrap_or("presentation"), id)
}

/// The ProPresenter UUID of an Orbit arrangement, shared by presentations and playlist entries.
pub(super) fn arrangement_uuid(item_id: &str, id: &str) -> String {
    Uuid::parse_str(id)
        .map(|id| id.to_string())
        .unwrap_or_else(|_| derived_uuid(&format!("{item_id}/arrangement/{id}")))
}

pub(super) fn encode_arrangements(item: &Value, groups: &CueGroups, document: &mut Vec<u8>) {
    for arrangement in item["arrangements"].as_array().into_iter().flatten() {
        let Some(id) = arrangement["id"].as_str() else {
            continue;
        };
        let mut encoded = Vec::new();
        put_uuid(1, &arrangement_id(item, id), &mut encoded);
        put_string(
            2,
            arrangement["name"].as_str().unwrap_or_default(),
            &mut encoded,
        );
        for section in arrangement["sectionIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            for group in groups.section_groups(item, section).into_iter().flatten() {
                let mut identifier = Vec::new();
                put_string(1, group, &mut identifier);
                put_message(3, identifier, &mut encoded);
            }
        }
        put_message(11, encoded, document);
    }
    if let Some(selected) = item["selectedArrangementId"].as_str().filter(|selected| {
        item["arrangements"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|arrangement| arrangement["id"] == *selected)
    }) {
        put_uuid(10, &arrangement_id(item, selected), document);
    }
}

/// Groups and arrangements that ProPresenter cannot express as whole groups.
pub(crate) fn arrangement_losses(item: &Value, reference: &str, findings: &mut Vec<String>) {
    let groups = cue_groups(item);
    let slides = item["slides"].as_array().map_or(&[][..], Vec::as_slice);
    for (index, group) in item["groups"].as_array().into_iter().flatten().enumerate() {
        let identified = group["sourceId"]
            .as_str()
            .is_some_and(|id| Uuid::parse_str(id).is_ok());
        let used = slides.iter().any(|slide| {
            group["id"].as_str().is_some_and(|id| {
                crate::output_arrangement::slide_group_id(slide).as_deref() == Some(id)
            })
        });
        if !identified && !used {
            findings.push(format!(
                "{reference}/groups/{index}: a group without slides or a ProPresenter identity is not exported"
            ));
        }
    }
    for (index, arrangement) in item["arrangements"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if arrangement["id"].as_str().is_none() {
            findings.push(format!(
                "{reference}/arrangements/{index}: an arrangement without an id is not exported"
            ));
        }
        for section in arrangement["sectionIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if groups.section_groups(item, section).is_none() {
                findings.push(format!(
                    "{reference}/arrangements/{index}/sectionIds/{section}: the section's slides are not a whole ProPresenter group"
                ));
            }
        }
    }
}

/// Writes the cue settings `parse_cue` reads besides its identity, name and enabled state.
pub(super) fn encode_cue_settings(slide: &Value, cue: &mut Vec<u8>) {
    if let Some(target) = slide["completionTargetType"]
        .as_str()
        .and_then(|target| match target {
            "NONE" => Some(0),
            "NEXT" => Some(1),
            "RANDOM" => Some(2),
            "CUE" => Some(3),
            "FIRST" => Some(4),
            _ => None,
        })
    {
        put_varint(3, target, cue);
    }
    if let Some(id) = slide["completionTargetCueId"].as_str() {
        put_uuid(4, id, cue);
    }
    if let Some(action) = slide["completionAction"]
        .as_str()
        .and_then(|action| match action {
            "COMPLETION_ACTION_TYPE_FIRST" => Some(0),
            "COMPLETION_ACTION_TYPE_LAST" => Some(1),
            "COMPLETION_ACTION_TYPE_AFTER_ACTION" => Some(2),
            "COMPLETION_ACTION_TYPE_AFTER_TIME" => Some(3),
            _ => None,
        })
    {
        put_varint(5, action, cue);
    }
    if let Some(id) = slide["completionActionId"].as_str() {
        put_uuid(6, id, cue);
    }
    if let Some(seconds) = slide["triggerTimeSeconds"]
        .as_f64()
        .filter(|seconds| seconds.is_finite())
    {
        let mut time = Vec::new();
        put_f64(1, seconds, &mut time);
        put_message(7, time, cue);
    }
    if let Some(seconds) = slide["completionTimeSeconds"]
        .as_f64()
        .filter(|seconds| seconds.is_finite())
    {
        put_f64(13, seconds, cue);
    }
}

pub(super) fn encode_notes(slide: &Value, presentation_slide: &mut Vec<u8>) {
    if let Some(notes) = slide["notes"]
        .as_str()
        .filter(|notes| !notes.is_empty() || slide["notesTextStyle"].is_object())
    {
        let mut encoded = Vec::new();
        let runs = slide["notesRuns"].as_array().map_or(&[][..], Vec::as_slice);
        let paragraph_style = slide["notesTextStyle"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let (fonts, colors, tables, body) = rtf_body(notes, runs, &paragraph_style, None);
        let rtf = format!("{{\\rtf1\\ansi\\uc1{{\\fonttbl{{\\f0 Arial;}}{fonts}}}{{\\colortbl;\\red0\\green0\\blue0;{colors}}}{tables}\\f0 {body}}}");
        put_bytes(1, rtf.as_bytes(), &mut encoded);
        put_message(2, encoded, presentation_slide);
    }
}

/// The cue's actions in their authored order, with the presentation slide action in place of
/// the imported one. Actions that cannot be written are named by `action_losses`.
pub(super) fn encode_actions(
    slide: &Value,
    slide_action: Vec<u8>,
    relative: &BTreeMap<String, String>,
) -> Vec<Vec<u8>> {
    let mut actions = Vec::new();
    let mut slide_action = Some(slide_action);
    for action in slide["actions"].as_array().into_iter().flatten() {
        if action["type"] == "presentation-slide" {
            if let Some(slide_action) = slide_action.take() {
                actions.push(slide_action);
            }
        } else if let Ok(encoded) = encode_action(action, relative) {
            actions.push(encoded);
        }
    }
    if let Some(slide_action) = slide_action {
        actions.insert(0, slide_action);
    }
    if let Some(media) = slide.get(MEDIA_BACKGROUND) {
        if let Ok(encoded) = media_bin_export::encode_media_cue_action(media, slide, relative) {
            actions.push(encoded);
        }
    }
    actions
}

fn encode_action(action: &Value, relative: &BTreeMap<String, String>) -> Result<Vec<u8>, String> {
    if action["type"] == "media" {
        return media_bin_export::encode_imported_media_action(action, relative);
    }
    marker_action_export::encode_marker_action(action).map_err(|error| match error {
        marker_action_export::MarkerActionExportError::UnsupportedVariant(reason)
        | marker_action_export::MarkerActionExportError::Invalid(reason) => reason,
    })
}

/// Names the slide actions and media background the cue export cannot write.
pub(crate) fn action_losses(slide: &Value, reference: &str, findings: &mut Vec<String>) {
    for (index, action) in slide["actions"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if action["type"] == "presentation-slide" {
            continue;
        }
        let result = if action["type"] == "media" {
            let losses = media_bin_export::imported_media_action_losses(action);
            if losses.is_empty() {
                // Probe the encoder with the show-relative copy the exporter would make.
                let mut probe = action.clone();
                let source = action["details"]["sourceUrl"].as_str().unwrap_or_default();
                let url = if source.starts_with("file://") {
                    source.to_string()
                } else {
                    url::Url::from_file_path(source)
                        .map_or_else(|_| source.to_string(), String::from)
                };
                probe["details"]["sourceUrl"] = json!(url);
                let relative = BTreeMap::from([(url, "Media/probe".to_string())]);
                media_bin_export::encode_imported_media_action(&probe, &relative)
                    .err()
                    .into_iter()
                    .collect()
            } else {
                losses
            }
        } else {
            encode_action(action, &BTreeMap::new())
                .err()
                .into_iter()
                .collect()
        };
        for reason in result {
            findings.push(format!("{reference}/actions/{index}: {reason}"));
        }
    }
}
