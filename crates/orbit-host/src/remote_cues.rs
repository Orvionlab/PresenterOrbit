use serde_json::Value;

pub(crate) fn library_catalog(workspace: &Value) -> Result<Value, String> {
    let items = workspace["library"]
        .as_array()
        .ok_or("Workspace library must be an array.")?;
    let libraries = workspace["libraries"]
        .as_array()
        .ok_or("Workspace libraries must be an array.")?;
    let mut result = Vec::new();
    for library in libraries {
        let ids = library["itemIds"]
            .as_array()
            .ok_or("Library itemIds must be an array.")?;
        let mut presentations = Vec::new();
        for id in ids {
            let item = items
                .iter()
                .find(|item| item["id"] == *id)
                .ok_or("Library presentation is unavailable.")?;
            let slides = arrangement_slides(item, None)?;
            let arrangements = item.get("arrangements").and_then(Value::as_array).into_iter().flatten()
                .map(|arrangement| {
                    let id = arrangement["id"].as_str().ok_or("Arrangement identity is required.")?;
                    Ok(serde_json::json!({"id":id,"title":arrangement["name"],"slides":cue_summaries(&arrangement_slides(item, Some(id))?)}))
                }).collect::<Result<Vec<Value>, String>>()?;
            // A selected arrangement that no longer exists plays the master order, so cues carry none.
            let arrangement_id = item.get("selectedArrangementId").filter(|id| {
                arrangements
                    .iter()
                    .any(|arrangement| arrangement["id"] == **id)
            });
            presentations.push(serde_json::json!({"id":item["id"],"title":item["title"],"arrangementId":arrangement_id,"arrangements":arrangements,"slides":cue_summaries(&slides),"modern":modern_presentation_source(workspace, item)}));
        }
        result.push(serde_json::json!({"id":library["id"],"title":library["title"],"presentations":presentations}));
    }
    Ok(Value::Array(result))
}

fn arrangement_item(item: &Value, arrangement_id: Option<&str>) -> Result<Value, String> {
    let mut item = item.clone();
    if let Some(id) = arrangement_id {
        if !item["arrangements"].as_array().is_some_and(|arrangements| {
            arrangements
                .iter()
                .any(|entry| entry["id"].as_str() == Some(id))
        }) {
            return Err("Arrangement is unavailable in the presentation.".into());
        }
        item["selectedArrangementId"] = serde_json::json!(id);
    }
    Ok(item)
}

// Like the renderer, a persisted entry whose arrangement no longer exists plays the master order.
fn playlist_entry_item(item: &Value, arrangement_id: Option<&str>) -> Value {
    let mut item = item.clone();
    if let Some(id) = arrangement_id {
        item["selectedArrangementId"] = serde_json::json!(id);
    }
    item
}

fn arrangement_slides(item: &Value, arrangement_id: Option<&str>) -> Result<Vec<Value>, String> {
    crate::output_arrangement::resolve_live_arrangement_slides(
        &arrangement_item(item, arrangement_id)?,
        None,
        None,
    )
    .ok_or("Presentation arrangement is unavailable.".into())
}

fn cue_summaries(slides: &[Value]) -> Vec<Value> {
    slides.iter().enumerate().map(|(index,slide)|serde_json::json!({"id":slide["id"],"index":index+1,"cueInstanceId":slide.get("arrangementInstanceId"),"title":slide.get("label"),"text":slide.get("content"),"notes":slide.get("notes"),"disabled":slide.get("disabled").and_then(Value::as_bool)==Some(true)})).collect()
}

pub(crate) fn playlist_catalog(workspace: &Value) -> Result<Value, String> {
    let playlists = workspace["playlist"]
        .as_array()
        .ok_or("Workspace playlist must be an array.")?;
    let items = workspace["library"]
        .as_array()
        .ok_or("Workspace library must be an array.")?;
    let mut result = Vec::new();
    for playlist in playlists {
        let playlist_items = playlist["items"]
            .as_array()
            .ok_or("Playlist items must be an array.")?;
        let mut summaries = Vec::new();
        for entry in playlist_items {
            if entry["type"] != "presentation" {
                continue;
            }
            let id = entry["id"]
                .as_str()
                .ok_or("Playlist entry identity is required.")?;
            // A missing presentation is exposed as a placeholder in the modern catalog.
            let Some(item) = items.iter().find(|item| item["id"] == entry["itemId"]) else {
                continue;
            };
            let selected = playlist_entry_item(item, entry["arrangementId"].as_str());
            let slides = crate::output_arrangement::resolve_live_arrangement_slides(
                &selected,
                None,
                Some(id),
            )
            .ok_or("Playlist arrangement is unavailable.")?;
            let arrangement_title = selected["arrangements"]
                .as_array()
                .and_then(|arrangements| {
                    arrangements
                        .iter()
                        .find(|arrangement| arrangement["id"] == selected["selectedArrangementId"])
                })
                .map(|arrangement| &arrangement["name"]);
            summaries.push(serde_json::json!({"id":id,"presentationId":item["id"],"title":item["title"],"arrangementId":selected.get("selectedArrangementId"),"arrangementTitle":arrangement_title,"slides":cue_summaries(&slides)}));
        }
        result.push(
            serde_json::json!({"id":playlist["id"],"title":playlist["title"],"entries":summaries,"modern":{"type":playlist.get("type"),"parentId":playlist.get("parentId"),"items":modern_playlist_items(playlist_items, &items)?}}),
        );
    }
    Ok(Value::Array(result))
}

fn modern_presentation_source(workspace: &Value, item: &Value) -> Value {
    let composed = workspace
        .as_object()
        .map(|document| crate::workspace::content_themes::compose_item(document, item))
        .unwrap_or_else(|| item.clone());
    let source_slides = composed["slides"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let slides = source_slides
        .iter()
        .enumerate()
        .map(|(index, slide)| {
            let section_id = crate::output_arrangement::slide_section_id(source_slides, index);
            let group_source_id = source_slides[..=index]
                .iter()
                .rev()
                .find_map(crate::output_arrangement::slide_group_id)
                .unwrap_or_else(|| {
                    slide
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                });
            serde_json::json!({
                "id": slide.get("id"),
                "groupSourceId": group_source_id,
                "sectionId": section_id,
                "sourceSectionId": slide.get("sectionId"),
                "groupId": slide.get("groupId"),
                "groupName": slide.get("groupName"),
                "groupColor": slide.get("groupColor"),
                "enabled": slide.get("disabled").and_then(Value::as_bool) != Some(true),
                "label": slide.get("label"),
                "notes": slide.get("notes"),
                "size": { "width": slide.get("width"), "height": slide.get("height") },
                "text": slide.get("content")
            })
        })
        .collect::<Vec<_>>();
    let arrangements = item["arrangements"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|arrangement| {
            serde_json::json!({
                "id": arrangement.get("id"),
                "name": arrangement.get("name"),
                "sectionIds": arrangement.get("sectionIds")
            })
        })
        .collect::<Vec<_>>();
    let group_settings = workspace["groupSettings"]
        .as_object()
        .into_iter()
        .flat_map(|settings| settings.iter())
        .filter(|(id, setting)| {
            slides.iter().any(|slide| {
                slide["groupId"].as_str() == Some(id.as_str())
                    || setting.get("name").and_then(Value::as_str) == slide["groupName"].as_str()
            })
        })
        .map(|(id, setting)| {
            serde_json::json!({
                "id": id,
                "name": setting.get("name"),
                "color": setting.get("color")
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "slides": slides,
        "arrangements": arrangements,
        "groupSettings": group_settings,
        "selectedArrangementId": item.get("selectedArrangementId"),
        "timeline": item.get("timeline")
    })
}

fn modern_playlist_items(entries: &[Value], presentations: &[Value]) -> Result<Vec<Value>, String> {
    entries
        .iter()
        .map(|entry| {
            let item = if entry["type"] == "presentation" {
                presentations.iter().find(|item| item["id"] == entry["itemId"])
            } else {
                None
            };
            let kind = if entry["type"] == "presentation" && item.is_none() {
                serde_json::json!("placeholder")
            } else {
                entry["type"].clone()
            };
            Ok(serde_json::json!({
                "id": entry.get("id"),
                "type": kind,
                "title": item.map_or_else(|| entry.get("title"), |item| item.get("title")),
                "presentationId": item.map(|item| &item["id"]),
                "arrangementId": entry.get("arrangementId"),
                "arrangementTitle": item.and_then(|item| {
                    entry.get("arrangementId").and_then(Value::as_str).and_then(|id| {
                        item["arrangements"].as_array()?.iter().find(|arrangement| arrangement["id"] == id).and_then(|arrangement| arrangement.get("name"))
                    })
                }),
                "headerColor": entry.get("color"),
                "linkedPresentationId": entry.get("linkedItemId"),
                "isHidden": entry.get("hidden").or_else(|| entry.get("isHidden")).cloned().unwrap_or(serde_json::json!(false)),
                "isPco": entry.get("isPco").cloned().unwrap_or(serde_json::json!(false))
            }))
        })
        .collect()
}

pub(crate) fn select_playlist(
    snapshot: &Value,
    workspace: &Value,
    payload: &Value,
) -> Result<(Value, Value), String> {
    select_playlist_entry(snapshot, workspace, payload, false)
}

pub(crate) fn select_operator_playlist(
    snapshot: &Value,
    workspace: &Value,
    payload: &Value,
) -> Result<(Value, Value), String> {
    select_playlist_entry(snapshot, workspace, payload, true)
}

fn select_playlist_entry(
    snapshot: &Value,
    workspace: &Value,
    payload: &Value,
    operator: bool,
) -> Result<(Value, Value), String> {
    let playlist = workspace["playlist"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["id"] == payload["playlistId"])
        })
        .ok_or("Playlist is unavailable.")?;
    let entry = playlist["items"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|entry| {
                entry["id"] == payload["playlistEntryId"]
                    && (entry["type"] == "presentation"
                        || (operator
                            && entry["type"] == "placeholder"
                            && entry["linkedItemId"]
                                .as_str()
                                .is_some_and(|id| !id.is_empty())))
            })
        })
        .ok_or("Playlist occurrence is unavailable.")?;
    let item_id = if entry["type"] == "placeholder" {
        &entry["linkedItemId"]
    } else {
        &entry["itemId"]
    };
    if item_id != &payload["presentationId"] {
        return Err("The playlist presentation has changed. Refresh the remote.".into());
    }
    let item = workspace["library"]
        .as_array()
        .and_then(|items| items.iter().find(|item| &item["id"] == item_id))
        .ok_or("Presentation is unavailable.")?;
    let mut selected = snapshot.clone();
    // The operator shows Master for playlist entries without an explicit arrangement.
    let arrangement = entry["arrangementId"].as_str().or(operator.then_some(""));
    selected["liveItem"] = playlist_entry_item(item, arrangement);
    selected["livePlaylistId"] = playlist["id"].clone();
    selected["livePlaylistEntryId"] = entry["id"].clone();
    selected["liveCueInstanceId"] = Value::Null;
    let (next, mut patch) = select_cue(&selected, payload, operator)?;
    patch["livePlaylistId"] = playlist["id"].clone();
    patch["livePlaylistEntryId"] = entry["id"].clone();
    Ok((next, patch))
}

pub(crate) fn select_library(
    snapshot: &Value,
    workspace: &Value,
    payload: &Value,
    operator: bool,
) -> Result<(Value, Value), String> {
    let library = workspace["libraries"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == payload["libraryId"]))
        .ok_or("Library is unavailable.")?;
    if !library["itemIds"]
        .as_array()
        .is_some_and(|ids| ids.contains(&payload["presentationId"]))
    {
        return Err("Presentation is unavailable in the selected library.".into());
    }
    let item = workspace["library"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["id"] == payload["presentationId"])
        })
        .ok_or("Presentation is unavailable.")?;
    let mut selected = snapshot.clone();
    let arrangement_id = payload
        .get("arrangementId")
        .map(|value| {
            value
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or("Arrangement identity must be a nonempty string when supplied.")
        })
        .transpose()?;
    selected["liveItem"] = arrangement_item(item, arrangement_id)?;
    selected["livePlaylistEntryId"] = Value::Null;
    selected["livePlaylistId"] = Value::Null;
    selected["liveCueInstanceId"] = Value::Null;
    let (mut next, mut patch) = select_cue(&selected, payload, operator)?;
    next["livePlaylistId"] = Value::Null;
    patch["livePlaylistId"] = Value::Null;
    patch["livePlaylistEntryId"] = Value::Null;
    Ok((next, patch))
}

pub(crate) fn select(snapshot: &Value, payload: &Value) -> Result<(Value, Value), String> {
    select_cue(snapshot, payload, false)
}

fn select_cue(
    snapshot: &Value,
    payload: &Value,
    allow_disabled: bool,
) -> Result<(Value, Value), String> {
    let item = snapshot
        .get("liveItem")
        .filter(|value| value.is_object())
        .ok_or("No live presentation is available.")?;
    if payload["presentationId"] != item["id"]
        || payload["playlistEntryId"] != snapshot["livePlaylistEntryId"]
    {
        return Err(
            "The live presentation or playlist occurrence has changed. Refresh the remote.".into(),
        );
    }
    let slides = crate::output_arrangement::resolve_live_arrangement_slides(
        item,
        snapshot["liveCueInstanceId"].as_str(),
        snapshot["livePlaylistEntryId"].as_str(),
    )
    .ok_or("The live arrangement is unavailable.")?;
    let target = slides
        .iter()
        .find(|slide| {
            slide["id"] == payload["slideId"]
                && slide["arrangementInstanceId"] == payload["cueInstanceId"]
        })
        .ok_or("The requested cue occurrence is unavailable in the live arrangement.")?;
    if !allow_disabled && target["disabled"] == true {
        return Err("The requested cue is disabled.".into());
    }
    crate::select_remote_slide_in_output_snapshot(snapshot, target)?
        .ok_or("The requested cue could not be selected.".into())
}
