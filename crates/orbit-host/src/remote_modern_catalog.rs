use std::collections::HashSet;

use serde_json::{json, Value};
use uuid::Uuid;

pub(super) fn identity(source_id: &str, name: &str, index: usize) -> Value {
    json!({ "index": index, "name": name, "uuid": api_uuid(source_id).hyphenated().to_string() })
}

pub(super) fn matches_id(source_id: &str, name: &str, index: usize, selector: &str) -> bool {
    Uuid::parse_str(selector)
        .ok()
        .is_some_and(|selector_uuid| selector_uuid == api_uuid(source_id))
        || name == selector
        || selector.parse::<usize>().ok() == Some(index)
}

fn api_uuid(source_id: &str) -> Uuid {
    Uuid::parse_str(source_id)
        .unwrap_or_else(|_| Uuid::new_v5(&Uuid::NAMESPACE_URL, source_id.as_bytes()))
}

pub(super) fn libraries(status: &Value) -> Result<Value, String> {
    let libraries = required_array(status, "libraries", "Status libraries are unavailable.")?;
    libraries
        .iter()
        .enumerate()
        .map(|(index, library)| api_id(library, index, "Library"))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

pub(super) fn library(status: &Value, selector: &str) -> Result<Value, String> {
    let (_, library) = resolve_library(status, selector)?;
    let presentations = required_array(
        library,
        "presentations",
        "Library presentations are unavailable.",
    )?;
    let items = presentations
        .iter()
        .enumerate()
        .map(|(index, presentation)| api_id(presentation, index, "Presentation"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "updateType": "all", "items": items }))
}

pub(super) fn playlists(status: &Value) -> Result<Value, String> {
    let playlists = required_array(status, "playlists", "Status playlists are unavailable.")?;
    let mut seen = HashSet::new();
    let mut roots = build_playlist_nodes(playlists, None, &mut seen, &mut Vec::new())?;
    if seen.len() != playlists.len() {
        return Err("A playlist folder parent is unavailable or cyclic.".into());
    }
    roots.shrink_to_fit();
    Ok(Value::Array(roots))
}

pub(super) fn playlist(status: &Value, selector: &str) -> Result<Value, String> {
    let playlist = resolve_playlist(status, selector)?;
    let items = required_array(
        playlist
            .value
            .get("modern")
            .ok_or("Playlist items are unavailable.")?,
        "items",
        "Playlist items are unavailable.",
    )?;
    let mut result = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        result.push(playlist_item(status, item, index)?);
    }
    Ok(json!({
        "id": api_id(playlist.value, playlist.api_index, "Playlist")?,
        "items": result
    }))
}

pub(super) fn presentation(status: &Value, selector: &str) -> Result<Value, String> {
    let (library, presentation, index) = resolve_presentation_source(status, selector)?;
    Ok(json!({
        "presentation": presentation_details(library, presentation, index)?
    }))
}

pub(super) fn resolve_presentation(
    status: &Value,
    selector: &str,
) -> Result<super::super::data::Resolved, String> {
    let (library, presentation, _) = resolve_presentation_source(status, selector)?;
    resolved_library_presentation(library, presentation)
}

pub(super) fn resolve_playlist_presentation(
    status: &Value,
    playlist_selector: &str,
    item_index: usize,
) -> Result<super::super::data::Resolved, String> {
    let playlist = resolve_playlist(status, playlist_selector)?;
    let modern_items = required_array(
        playlist
            .value
            .get("modern")
            .ok_or("Playlist items are unavailable.")?,
        "items",
        "Playlist items are unavailable.",
    )?;
    let item = modern_items
        .get(item_index)
        .ok_or("The playlist item is unavailable.")?;
    if item.get("type").and_then(Value::as_str) != Some("presentation") {
        return Err("The playlist item is not a presentation.".into());
    }
    let entry_id = required_string(item, "id", "Playlist entry identity is required.")?;
    let presentation_id = required_string(
        item,
        "presentationId",
        "Playlist presentation identity is required.",
    )?;
    let entry_index = required_array(
        playlist.value,
        "entries",
        "Playlist entries are unavailable.",
    )?
    .iter()
    .position(|entry| entry.get("id").and_then(Value::as_str) == Some(entry_id))
    .ok_or("The playlist presentation is unavailable.")?;
    let entry = &playlist.value["entries"][entry_index];
    if entry.get("presentationId").and_then(Value::as_str) != Some(presentation_id) {
        return Err("The playlist presentation has changed. Refresh the remote.".into());
    }
    required_string(entry, "title", "Presentation title is required.")?;
    let path = format!("{}:{entry_index}", playlist.source_index);
    Ok(super::super::data::Resolved {
        path,
        slides: required_array(entry, "slides", "Presentation slides are unavailable.")?.to_vec(),
        cue: json!({
            "command": "playlistCue",
            "playlistId": playlist.value["id"],
            "playlistEntryId": entry["id"],
            "presentationId": entry["presentationId"]
        }),
    })
}

fn resolve_library<'a>(status: &'a Value, selector: &str) -> Result<(usize, &'a Value), String> {
    let libraries = required_array(status, "libraries", "Status libraries are unavailable.")?;
    let matches = libraries
        .iter()
        .enumerate()
        .filter(|(index, library)| matches_item(library, *index, selector));
    one_match(matches, "Library")
}

struct PlaylistReference<'a> {
    source_index: usize,
    api_index: usize,
    value: &'a Value,
}

fn resolve_playlist<'a>(
    status: &'a Value,
    selector: &str,
) -> Result<PlaylistReference<'a>, String> {
    let playlists = required_array(status, "playlists", "Status playlists are unavailable.")?;
    let candidates = playlist_candidates(playlists)?;
    let mut matches = candidates
        .into_iter()
        .filter(|playlist| matches_item(playlist.value, playlist.api_index, selector));
    let Some(result) = matches.next() else {
        return Err("Playlist is unavailable.".into());
    };
    if matches.next().is_some() {
        return Err("The playlist identifier is ambiguous.".into());
    }
    Ok(result)
}

fn resolve_presentation_source<'a>(
    status: &'a Value,
    selector: &str,
) -> Result<(&'a Value, &'a Value, usize), String> {
    let libraries = required_array(status, "libraries", "Status libraries are unavailable.")?;
    let mut matches = Vec::new();
    for library in libraries {
        for (index, presentation) in required_array(
            library,
            "presentations",
            "Library presentations are unavailable.",
        )?
        .iter()
        .enumerate()
        {
            if matches_item(presentation, index, selector) {
                matches.push((library, presentation, index));
            }
        }
    }
    match matches.as_slice() {
        [result] => Ok(*result),
        [] => Err("Presentation is unavailable.".into()),
        _ => Err("The presentation identifier is ambiguous.".into()),
    }
}

fn resolved_library_presentation(
    library: &Value,
    presentation: &Value,
) -> Result<super::super::data::Resolved, String> {
    let library_id = required_string(library, "id", "Library identity is required.")?;
    let library_title = required_string(library, "title", "Library title is required.")?;
    let presentation_id =
        required_string(presentation, "id", "Presentation identity is required.")?;
    let title = required_string(presentation, "title", "Presentation title is required.")?;
    let mut cue = json!({
        "command": "libraryCue",
        "libraryId": library_id,
        "presentationId": presentation_id,
        "playlistEntryId": null
    });
    if let Some(arrangement_id) = presentation["arrangementId"]
        .as_str()
        .filter(|id| !id.is_empty())
    {
        cue["arrangementId"] = json!(arrangement_id);
    }
    Ok(super::super::data::Resolved {
        path: format!("{library_title}/{title}.pro"),
        slides: required_array(
            presentation,
            "slides",
            "Presentation slides are unavailable.",
        )?
        .to_vec(),
        cue,
    })
}

fn presentation_details(
    library: &Value,
    presentation: &Value,
    index: usize,
) -> Result<Value, String> {
    let id = api_id(presentation, index, "Presentation")?;
    let modern = required_object(
        presentation,
        "modern",
        "Modern presentation details are unavailable.",
    )?;
    let group_models = presentation_groups(modern)?;
    let arrangements = presentation_arrangements(modern, &group_models)?;
    let current_arrangement = selected_arrangement_uuid(modern)?;
    let total_cues = group_models
        .iter()
        .map(|group| group["slides"].as_array().map_or(0, Vec::len))
        .sum::<usize>();
    let groups = group_models
        .iter()
        .map(|group| {
            json!({
                "uuid": group["uuid"],
                "name": group["name"],
                "color": group["color"],
                "slides": group["slides"]
            })
        })
        .collect::<Vec<_>>();
    let library_title = required_string(library, "title", "Library title is required.")?;
    let title = required_string(presentation, "title", "Presentation title is required.")?;
    let has_timeline = match modern.get("timeline") {
        None | Some(Value::Null) => false,
        Some(Value::Object(_)) => true,
        Some(_) => return Err("Presentation timeline is invalid.".into()),
    };
    Ok(json!({
        "id": id,
        "groups": groups,
        "arrangements": arrangements,
        "current_arrangement": current_arrangement,
        "destination": "presentation",
        "has_timeline": has_timeline,
        "is_authorized": true,
        "presentation_path": format!("{library_title}/{title}.pro"),
        "total_cues": total_cues,
        "type": "standard"
    }))
}

fn presentation_groups(modern: &Value) -> Result<Vec<Value>, String> {
    let slides = required_array(modern, "slides", "Presentation slides are unavailable.")?;
    let settings = required_array(
        modern,
        "groupSettings",
        "Presentation group settings are unavailable.",
    )?;
    let mut groups: Vec<Value> = Vec::new();
    for (slide_index, slide) in slides.iter().enumerate() {
        let section_id = required_string(
            slide,
            "sectionId",
            "Presentation slide section is required.",
        )?;
        let source_id = required_string(
            slide,
            "groupSourceId",
            "Presentation group identity is required.",
        )?;
        let (name, color) = group_metadata(settings, Some(source_id), slide)?;
        let slide = presentation_slide(slide)?;
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group["_sectionId"].as_str() == Some(section_id))
        {
            if group["_sourceId"] != source_id {
                return Err("Presentation group identity is inconsistent.".into());
            }
            if group["name"].as_str().unwrap_or_default().is_empty() && !name.is_empty() {
                group["name"] = json!(name);
            } else if !name.is_empty() && group["name"].as_str() != Some(name.as_str()) {
                return Err("Presentation group names are inconsistent.".into());
            }
            if group["color"].is_null() && !color.is_null() {
                group["color"] = color;
            } else if !color.is_null() && group["color"] != color {
                return Err("Presentation group colors are inconsistent.".into());
            }
            group["slides"].as_array_mut().unwrap().push(slide);
            add_source_section_id(group, source_section_id(slides.get(slide_index))?)?;
        } else {
            let mut group = json!({
                "_sourceId": source_id,
                "_sectionId": section_id,
                "_sourceSectionIds": [],
                "name": name,
                "color": color,
                "slides": [slide]
            });
            add_source_section_id(&mut group, source_section_id(slides.get(slide_index))?)?;
            groups.push(group);
        }
    }
    for (index, group) in groups.iter_mut().enumerate() {
        let source_id = required_string(
            group,
            "_sourceId",
            "Presentation group identity is required.",
        )?;
        let name = required_string(group, "name", "Presentation group name is required.")?;
        let color = rgba_color(group.get("color"))?;
        let uuid = identity(source_id, name, index)["uuid"].clone();
        group["uuid"] = uuid;
        group["color"] = color;
    }
    Ok(groups)
}

fn source_section_id(slide: Option<&Value>) -> Result<Option<&str>, String> {
    match slide.and_then(|slide| slide.get("sourceSectionId")) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(section_id)) if !section_id.is_empty() => Ok(Some(section_id)),
        Some(Value::String(_)) => Ok(None),
        Some(_) => Err("Presentation source section identity must be a string.".into()),
    }
}

fn add_source_section_id(group: &mut Value, section_id: Option<&str>) -> Result<(), String> {
    if let Some(section_id) = section_id {
        let source_ids = group
            .get_mut("_sourceSectionIds")
            .and_then(Value::as_array_mut)
            .ok_or("Presentation group section aliases are unavailable.")?;
        if !source_ids
            .iter()
            .any(|candidate| candidate.as_str() == Some(section_id))
        {
            source_ids.push(json!(section_id));
        }
    }
    Ok(())
}

fn presentation_slide(slide: &Value) -> Result<Value, String> {
    let size = required_object(slide, "size", "Presentation slide size is required.")?;
    // Missing sizes follow the shared slide composition default.
    let width = slide_dimension(size, "width", 1920.0)?;
    let height = slide_dimension(size, "height", 1080.0)?;
    let enabled = slide
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or("Presentation slide enabled state is required.")?;
    Ok(json!({
        "enabled": enabled,
        "label": optional_slide_text(slide, "label")?,
        "notes": optional_slide_text(slide, "notes")?,
        "size": { "width": width, "height": height },
        "text": optional_slide_text(slide, "text")?
    }))
}

// Orbit slides may omit label, notes and text, and labels may be numeric; the API reports text.
fn optional_slide_text(slide: &Value, field: &str) -> Result<String, String> {
    match slide.get(field) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(Value::Number(number)) if field == "label" => Ok(number.to_string()),
        Some(_) => Err(format!("Presentation slide {field} must be text.")),
    }
}

fn presentation_arrangements(modern: &Value, groups: &[Value]) -> Result<Vec<Value>, String> {
    let arrangements = required_array(
        modern,
        "arrangements",
        "Presentation arrangements are unavailable.",
    )?;
    arrangements
        .iter()
        .enumerate()
        .map(|(index, arrangement)| {
            let source_id =
                required_string(arrangement, "id", "Arrangement identity is required.")?;
            let name = required_string(arrangement, "name", "Arrangement name is required.")?;
            let sections = required_array(
                arrangement,
                "sectionIds",
                "Arrangement sections are unavailable.",
            )?;
            let mut group_ids = Vec::with_capacity(sections.len());
            let mut total_cues = 0usize;
            for section in sections {
                let section = section
                    .as_str()
                    .ok_or("Arrangement section identity must be a string.")?;
                let matches = groups
                    .iter()
                    .filter(|group| {
                        group["_sectionId"].as_str() == Some(section)
                            || group["_sourceSectionIds"].as_array().is_some_and(|ids| {
                                ids.iter().any(|id| id.as_str() == Some(section))
                            })
                    })
                    .collect::<Vec<_>>();
                let [group] = matches.as_slice() else {
                    return Err(if matches.is_empty() {
                        "Arrangement references an unavailable group.".into()
                    } else {
                        "Arrangement section identity is ambiguous.".into()
                    });
                };
                group_ids.push(group["uuid"].clone());
                total_cues += group["slides"].as_array().map_or(0, Vec::len);
            }
            Ok(json!({
                "groups": group_ids,
                "id": identity(source_id, name, index),
                "total_cues": total_cues
            }))
        })
        .collect()
}

fn selected_arrangement_uuid(modern: &Value) -> Result<String, String> {
    let Some(selected) = modern.get("selectedArrangementId") else {
        return Ok(String::new());
    };
    let Some(selected) = selected.as_str().filter(|id| !id.is_empty()) else {
        return if selected.is_null() {
            Ok(String::new())
        } else {
            Err("Selected arrangement identity is invalid.".into())
        };
    };
    let source_arrangements = required_array(
        modern,
        "arrangements",
        "Presentation arrangements are unavailable.",
    )?;
    source_arrangements
        .iter()
        .enumerate()
        .find(|(_, arrangement)| arrangement["id"].as_str() == Some(selected))
        .map(|(index, arrangement)| {
            Ok::<String, String>(
                identity(
                    selected,
                    required_string(arrangement, "name", "Arrangement name is required.")?,
                    index,
                )["uuid"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
        })
        .transpose()
        // Like native output, a selected arrangement that no longer exists plays the master order.
        .map(Option::unwrap_or_default)
}

fn group_metadata(
    settings: &[Value],
    group_id: Option<&str>,
    slide: &Value,
) -> Result<(String, Value), String> {
    let slide_name = slide.get("groupName").and_then(Value::as_str);
    let by_id = group_id
        .map(|id| {
            settings
                .iter()
                .filter(|setting| setting["id"].as_str() == Some(id))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let candidates = if !by_id.is_empty() {
        by_id
    } else {
        slide_name
            .map(|name| {
                settings
                    .iter()
                    .filter(|setting| setting["name"].as_str() == Some(name))
                    .collect()
            })
            .unwrap_or_default()
    };
    let setting = match candidates.as_slice() {
        [setting] => Some(*setting),
        [] => None,
        _ => return Err("Presentation group settings are ambiguous.".into()),
    };
    let name = setting
        .and_then(|setting| setting.get("name"))
        .and_then(Value::as_str)
        .or(slide_name)
        .unwrap_or_default()
        .to_string();
    let color = setting
        .and_then(|setting| setting.get("color"))
        .or_else(|| slide.get("groupColor"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok((name, color))
}

fn playlist_item(status: &Value, item: &Value, index: usize) -> Result<Value, String> {
    let kind = required_string(item, "type", "Playlist item type is required.")?;
    if !matches!(kind, "presentation" | "header" | "placeholder") {
        return Err("Playlist item type is unsupported.".into());
    }
    let id = required_string(item, "id", "Playlist item identity is required.")?;
    let name = required_string(item, "title", "Playlist item title is required.")?;
    let api_id = if kind == "header" {
        json!({ "index": index, "name": name, "uuid": "" })
    } else {
        identity(id, name, index)
    };
    let mut result = json!({
        "id": api_id,
        "type": kind,
        "is_hidden": item.get("isHidden").and_then(Value::as_bool).ok_or("Playlist item visibility is required.")?,
        "is_pco": item.get("isPco").and_then(Value::as_bool).ok_or("Playlist item PCO state is required.")?
    });
    if kind == "header" {
        result["header_color"] = rgba_color(item.get("headerColor"))?;
    } else if kind == "presentation" {
        let presentation_id = required_string(
            item,
            "presentationId",
            "Playlist presentation identity is required.",
        )?;
        let (_, presentation, presentation_index) =
            resolve_presentation_by_source_id(status, presentation_id)?;
        let presentation_title =
            required_string(presentation, "title", "Presentation title is required.")?;
        let presentation_uuid =
            identity(presentation_id, presentation_title, presentation_index)["uuid"].clone();
        result["target_uuid"] = presentation_uuid.clone();
        let mut info = json!({ "presentation_uuid": presentation_uuid });
        if let Some(arrangement_id) = item
            .get("arrangementId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            let modern = required_object(
                presentation,
                "modern",
                "Modern presentation details are unavailable.",
            )?;
            let arrangements = required_array(
                modern,
                "arrangements",
                "Presentation arrangements are unavailable.",
            )?;
            // An arrangement that no longer exists plays the master order, so it has no arrangement info.
            if let Some((arrangement_index, arrangement)) = arrangements
                .iter()
                .enumerate()
                .find(|(_, arrangement)| arrangement["id"].as_str() == Some(arrangement_id))
            {
                let arrangement_name =
                    required_string(arrangement, "name", "Arrangement name is required.")?;
                info["arrangement_name"] = json!(arrangement_name);
                info["arrangement_uuid"] =
                    identity(arrangement_id, arrangement_name, arrangement_index)["uuid"].clone();
            }
        }
        result["presentation_info"] = info;
    } else {
        result["target_uuid"] = Value::Null;
    }
    Ok(result)
}

fn resolve_presentation_by_source_id<'a>(
    status: &'a Value,
    source_id: &str,
) -> Result<(&'a Value, &'a Value, usize), String> {
    let libraries = required_array(status, "libraries", "Status libraries are unavailable.")?;
    let mut matches = Vec::new();
    for library in libraries {
        for (index, presentation) in required_array(
            library,
            "presentations",
            "Library presentations are unavailable.",
        )?
        .iter()
        .enumerate()
        {
            if presentation.get("id").and_then(Value::as_str) == Some(source_id) {
                matches.push((library, presentation, index));
            }
        }
    }
    match matches.as_slice() {
        [result] => Ok(*result),
        [] => Err("Playlist presentation is unavailable in the library.".into()),
        _ => Err("Playlist presentation identity is ambiguous.".into()),
    }
}

fn playlist_candidates<'a>(playlists: &'a [Value]) -> Result<Vec<PlaylistReference<'a>>, String> {
    let mut sibling_indexes = std::collections::HashMap::<String, usize>::new();
    let mut candidates = Vec::new();
    for (source_index, playlist) in playlists.iter().enumerate() {
        let kind = playlist_kind(playlist)?;
        let parent = playlist_parent_id(playlist)?
            .unwrap_or_default()
            .to_string();
        let index = sibling_indexes.entry(parent).or_default();
        if kind != "folder" {
            candidates.push(PlaylistReference {
                source_index,
                api_index: *index,
                value: playlist,
            });
        }
        *index += 1;
    }
    Ok(candidates)
}

fn build_playlist_nodes(
    playlists: &[Value],
    parent_id: Option<&str>,
    seen: &mut HashSet<usize>,
    ancestors: &mut Vec<String>,
) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    for (source_index, playlist) in playlists.iter().enumerate() {
        if playlist_parent_id(playlist)? != parent_id {
            continue;
        }
        if !seen.insert(source_index) {
            return Err("Playlist folder structure is cyclic.".into());
        }
        let source_id = required_string(playlist, "id", "Playlist identity is required.")?;
        let title = required_string(playlist, "title", "Playlist title is required.")?;
        let index = result.len();
        match playlist_kind(playlist)? {
            "folder" => {
                if ancestors.iter().any(|ancestor| ancestor == source_id) {
                    return Err("Playlist folder structure is cyclic.".into());
                }
                ancestors.push(source_id.to_string());
                let children = build_playlist_nodes(playlists, Some(source_id), seen, ancestors)?;
                ancestors.pop();
                result.push(json!({
                    "id": identity(source_id, title, index),
                    "type": "group",
                    "children": children
                }));
            }
            "playlist" => result.push(json!({
                "id": identity(source_id, title, index),
                "type": "playlist",
                "children": []
            })),
            _ => return Err("Playlist type is unsupported.".into()),
        }
    }
    Ok(result)
}

fn playlist_kind(playlist: &Value) -> Result<&str, String> {
    let kind = playlist
        .get("modern")
        .and_then(|modern| modern.get("type"))
        .and_then(Value::as_str)
        .or_else(|| playlist.get("type").and_then(Value::as_str))
        .unwrap_or("playlist");
    match kind {
        "folder" => Ok("folder"),
        "playlist" => Ok("playlist"),
        _ => Err("Playlist type is unsupported.".into()),
    }
}

fn playlist_parent_id(playlist: &Value) -> Result<Option<&str>, String> {
    match playlist
        .get("modern")
        .and_then(|modern| modern.get("parentId"))
    {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(parent)) if parent.is_empty() => Ok(None),
        Some(Value::String(parent)) => Ok(Some(parent)),
        Some(_) => Err("Playlist parent identity must be a string.".into()),
    }
}

fn api_id(item: &Value, index: usize, kind: &str) -> Result<Value, String> {
    Ok(identity(
        required_string(item, "id", &format!("{kind} identity is required."))?,
        required_string(item, "title", &format!("{kind} title is required."))?,
        index,
    ))
}

fn matches_item(item: &Value, index: usize, selector: &str) -> bool {
    let (Some(id), Some(name)) = (
        item.get("id").and_then(Value::as_str),
        item.get("title").and_then(Value::as_str),
    ) else {
        return false;
    };
    matches_id(id, name, index, selector)
}

fn one_match<'a>(
    mut matches: impl Iterator<Item = (usize, &'a Value)>,
    kind: &str,
) -> Result<(usize, &'a Value), String> {
    let Some(result) = matches.next() else {
        return Err(format!("{kind} is unavailable."));
    };
    if matches.next().is_some() {
        let kind = kind.to_lowercase();
        return Err(format!("The {kind} identifier is ambiguous."));
    }
    Ok(result)
}

fn required_array<'a>(value: &'a Value, field: &str, message: &str) -> Result<&'a [Value], String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| message.to_string())
}

fn required_object<'a>(value: &'a Value, field: &str, message: &str) -> Result<&'a Value, String> {
    value
        .get(field)
        .filter(|candidate| candidate.is_object())
        .ok_or_else(|| message.to_string())
}

fn required_string<'a>(value: &'a Value, field: &str, message: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| message.to_string())
}

fn slide_dimension(value: &Value, field: &str, default: f64) -> Result<f64, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(default),
        Some(_) => required_dimension(value, field),
    }
}

fn required_dimension(value: &Value, field: &str) -> Result<f64, String> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number > 0.0)
        .ok_or_else(|| format!("Presentation slide {field} must be finite and positive."))
}

fn rgba_color(value: Option<&Value>) -> Result<Value, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(Value::Null);
    };
    if let Some(color) = value.as_object() {
        let channel = |name| {
            color
                .get(name)
                .and_then(Value::as_f64)
                .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
                .ok_or_else(|| format!("Presentation color {name} must be between 0 and 1."))
        };
        return Ok(json!({
            "red": channel("red")?,
            "green": channel("green")?,
            "blue": channel("blue")?,
            "alpha": channel("alpha")?
        }));
    }
    let color = value
        .as_str()
        .ok_or("Presentation color must be an RGBA object or hex string.")?;
    if color.eq_ignore_ascii_case("transparent") {
        return Ok(json!({ "red": 0.0, "green": 0.0, "blue": 0.0, "alpha": 0.0 }));
    }
    let hex = color.strip_prefix('#').unwrap_or(color);
    if !matches!(hex.len(), 6 | 8) {
        return Err("Presentation color must use six or eight hexadecimal digits.".into());
    }
    let rgb = crate::native_transitions::parse_color_hex(&format!("#{}", &hex[..6]))
        .ok_or("Presentation color is invalid.")?;
    let alpha = if hex.len() == 8 {
        u8::from_str_radix(&hex[6..8], 16).map_err(|_| "Presentation color alpha is invalid.")?
            as f64
            / 255.0
    } else {
        1.0
    };
    Ok(json!({ "red": rgb[0], "green": rgb[1], "blue": rgb[2], "alpha": alpha }))
}
