use super::model::{self, Document};
use crate::collections::arrangements::edit_item;
use crate::state::runtime::{Kind, Persist, Runtime};
use gpui::{App, AppContext};
use serde_json::{json, Value};

pub fn error(runtime: &Runtime, cx: &mut App, error: String) {
    runtime.status.update(cx, |status, cx| {
        status.message = error.clone();
        cx.notify();
    });
    runtime.output.update(cx, |output, cx| {
        output.error = Some(error);
        cx.notify();
    });
}

pub fn take(
    runtime: &Runtime,
    document: &Document,
    index: usize,
    reorder_pending: bool,
    cx: &mut App,
) {
    if document.cues.get(index).is_none() {
        return;
    }
    if runtime.output.read(cx).committed_revision == 0 {
        return;
    }
    let content = runtime.content.read(cx);
    let announcing = content
        .items
        .get(&document.item.id)
        .is_some_and(|item| super::flow::announces(&item.value));
    if announcing {
        announce(runtime, document, index, cx);
        return;
    }
    let library = runtime
        .content
        .read(cx)
        .library_id_for_item(&document.item.id)
        .to_string();
    let (command, mut payload) = cue_payload(document, index, &library);
    let timing = if reorder_pending {
        // Master cues are unique by slide ID. Their playlist occurrence IDs contain a position.
        if document.arrangement.is_empty() {
            payload["cueInstanceId"] = Value::Null;
        }
        CueTiming::Reordering
    } else {
        CueTiming::Latest
    };
    run_native(runtime, cx, command, payload, false, timing);
}

/// A presentation whose destination is announcements goes to the independent announcements player, timed by
/// its own auto advance.
fn announce(runtime: &Runtime, document: &Document, index: usize, cx: &mut App) {
    use crate::automation::announcements as player;
    let cue = &document.cues[index];
    let mut shown = (*document.item.value).clone();
    shown["selectedArrangementId"] = json!("");
    shown["arrangements"] = json!([]);
    let (interval, looped) = super::flow::player_timing(&shown);
    match player::select_operator_slide(
        &shown,
        &document.cues,
        cue["id"].as_str().unwrap_or_default(),
        &cue["arrangementInstanceId"],
        player::now_ms(),
        uuid::Uuid::new_v4().to_string(),
        player::next_revision(),
    ) {
        Ok(mut started) => {
            started["player"]["intervalSeconds"] = json!(interval);
            started["player"]["loop"] = json!(looped);
            started["playlistId"] = json!(document.playlist);
            runtime.output.update(cx, |output, cx| {
                if let Some((id, _)) = output.snapshot.as_ref().and_then(|snapshot| {
                    let screens = crate::output::view::screens(snapshot);
                    crate::output::view::selected_screen(&screens, &output.selected_screen)
                        .filter(|(_, config)| config["type"] != "stage")
                        .cloned()
                }) {
                    started["visibleScreenId"] = json!(id);
                }
                output.edit_live(|live| live["announcements"] = started);
                cx.notify();
            });
            player::tick(runtime, cx);
        }
        Err(failure) => error(runtime, cx, failure),
    }
}

pub fn take_editor_slide(runtime: &Runtime, item_id: &str, index: usize, cx: &mut App) {
    let content = runtime.content.read(cx);
    let Some(item) = content.items.get(item_id) else {
        return;
    };
    let Some(slide) = item.value["slides"]
        .as_array()
        .and_then(|slides| slides.get(index))
    else {
        return;
    };
    let payload = json!({"presentationId":item_id,"slideId":slide["id"],"libraryId":content.library_id_for_item(item_id),"cueInstanceId":null});
    run_native(runtime, cx, "libraryCue", payload, false, CueTiming::Latest);
}

fn cue_payload(document: &Document, index: usize, library: &str) -> (&'static str, Value) {
    let cue = &document.cues[index];
    let mut payload = json!({"presentationId":document.item.id,"slideId":cue["id"],"cueInstanceId":cue.get("arrangementInstanceId").cloned().unwrap_or(Value::Null)});
    let command = if document.entry.is_empty() {
        payload["libraryId"] = json!(library);
        if !document.arrangement.is_empty() {
            payload["arrangementId"] = json!(document.arrangement);
        }
        "libraryCue"
    } else {
        payload["playlistId"] = json!(document.playlist);
        payload["playlistEntryId"] = json!(document.entry);
        "playlistCue"
    };
    (command, payload)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CueTiming {
    Ordered,
    Latest,
    Reordering,
}

pub(crate) fn run_native(
    runtime: &Runtime,
    cx: &mut App,
    command: &'static str,
    payload: Value,
    cross_documents: bool,
    timing: CueTiming,
) {
    let path = runtime.content.read(cx).workspace_path.clone();
    let before_save = timing == CueTiming::Reordering
        || (timing == CueTiming::Latest
            && payload["presentationId"]
                .as_str()
                .is_some_and(|item| runtime.content.read(cx).cue_flags_pending(item)));
    let workspace_revision = runtime.lanes.revisions().workspace(&path);
    let host = runtime.host.clone();
    let selection = runtime.ui.read(cx).selected_library_item_id.clone();
    let occurrence = selected_occurrence(runtime, cx);
    let fallback = cross_documents
        .then(|| model::seed(runtime.content.read(cx), runtime.ui.read(cx)))
        .flatten();
    let library = fallback
        .as_ref()
        .map(|seed| {
            runtime
                .content
                .read(cx)
                .library_id_for_item(&seed.item.id)
                .to_string()
        })
        .unwrap_or_default();
    let expected = path.clone();
    let publish = !cx
        .try_global::<crate::automation::remote::Suppressed>()
        .is_some_and(|s| s.0);
    let mut live = (*runtime.output.read(cx).live).clone();
    let workspace = runtime.content.read(cx).workspace.clone();
    let lanes = runtime.lanes.clone();
    let execute = move || {
        // A direct take can use the committed cue while its order or flags are being saved.
        let mut payload = payload;
        payload["workspacePath"] = json!(expected);
        let response = if before_save {
            host.execute_operator_cue_before_save(command, &payload, workspace_revision)?
        } else {
            host.execute_cue_command(command, &payload)?
        };
        if cross_documents && response["status"] == "not-ready" {
            let Some(seed) = fallback else {
                return Err("Select a presentation to navigate.".into());
            };
            let document = seed.resolve()?;
            let target = navigation_target(
                &document.cues,
                &document.item.id,
                &response["presentationNextAnchor"],
                command == "nextSlide",
            );
            if let Some(index) = target {
                let (command, mut payload) = cue_payload(&document, index, &library);
                payload["workspacePath"] = json!(expected);
                return host.execute_cue_command(command, &payload);
            }
            return Ok(json!({"ok":true,"changed":false,"revision":response["revision"]}));
        }
        if response["changed"] == true && publish {
            if let Some(patch) = response["patch"].as_object() {
                live.as_object_mut()
                    .ok_or("Live state must be an object.")?
                    .extend(patch.clone());
            }
            if let Some(cue) = crate::automation::link::cue(&workspace, &live)? {
                let args = vec![json!(expected), cue];
                if before_save {
                    drop(lanes.command(&expected, move || host.invoke("sendNetworkLinkCue", args)));
                } else {
                    host.invoke("sendNetworkLinkCue", args)?;
                }
            }
        }
        Ok(response)
    };
    let answer = if timing != CueTiming::Ordered {
        runtime.lanes.latest_cue(&path, before_save, execute)
    } else {
        runtime.lanes.task(&path, move || execute().map(Some))
    };
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        let result = answer
            .await
            .unwrap_or_else(|_| Err("The cue command worker stopped.".into()));
        cx.update(|cx| {
            if runtime.content.read(cx).workspace_path != path {
                return;
            }
            match result {
                Ok(Some(response)) => {
                    if response["patch"].is_object() {
                        runtime.output.update(cx, |output, cx| {
                            if response["revision"]
                                .as_u64()
                                .is_some_and(|revision| revision >= output.committed_revision)
                            {
                                if let Err(error) = output.accept_native_patch(&response) {
                                    output.error = Some(error);
                                }
                                cx.notify();
                            }
                        });
                    }
                    if cross_documents
                        && response["changed"] == false
                        && response["revision"].as_u64()
                            == Some(runtime.output.read(cx).committed_revision)
                        && runtime.ui.read(cx).selected_library_item_id == selection
                        && selected_occurrence(&runtime, cx) == occurrence
                    {
                        cross_document(&runtime, if command == "nextSlide" { 1 } else { -1 }, cx);
                    }
                }
                Ok(None) => {}
                Err(failure) => error(&runtime, cx, failure),
            }
        });
    })
    .detach();
}

pub fn navigate(runtime: &Runtime, direction: i8, cx: &mut App) {
    if crate::external::step(runtime, direction, cx) {
        return;
    }
    if direction > 0 && step_from_playlist_header(runtime, cx) {
        return;
    }
    run_native(
        runtime,
        cx,
        if direction > 0 {
            "nextSlide"
        } else {
            "previousSlide"
        },
        json!({}),
        true,
        CueTiming::Ordered,
    );
}

fn selected_occurrence(runtime: &Runtime, cx: &App) -> Option<(String, usize)> {
    let ui = runtime.ui.read(cx);
    Some((
        ui.selected_playlist_id.clone(),
        ui.selected_documents.first()?.index?,
    ))
}

pub(super) fn continuous(runtime: &Runtime, cx: &App) -> bool {
    crate::settings::persist::shown(runtime, cx)["generalSettings"]["useContinuousPlaylist"] == true
}

fn step_from_playlist_header(runtime: &Runtime, cx: &mut App) -> bool {
    let Some((playlist, index)) =
        selected_occurrence(runtime, cx).filter(|_| continuous(runtime, cx))
    else {
        return false;
    };
    let content = runtime.content.read(cx);
    let ui = runtime.ui.read(cx);
    let Some(entry) = content
        .playlist(&playlist)
        .and_then(|list| list.entries.get(index))
        .filter(|entry| entry.raw["hidden"] != true)
    else {
        return false;
    };
    let live = &runtime.output.read(cx).live;
    let live_playlist = live["livePlaylistId"]
        .as_str()
        .or_else(|| live["presentationNextAnchor"]["playlistId"].as_str());
    let live_entry = live["livePlaylistEntryId"]
        .as_str()
        .or_else(|| live["presentationNextAnchor"]["playlistEntryId"].as_str());
    if live_playlist == Some(playlist.as_str()) && live_entry == Some(entry.id.as_str()) {
        return false;
    }
    let seed = entry
        .linked_document()
        .and_then(|_| model::seed(content, ui));
    if let Some(seed) = seed {
        take_document_edge(runtime, seed, 1, true, cx);
    } else {
        reveal_next_playlist_header(runtime, &playlist, index, cx);
    }
    true
}

fn reveal_next_playlist_header(runtime: &Runtime, playlist: &str, current: usize, cx: &mut App) {
    runtime.edit_ui(cx, |content, ui| {
        let next = content.playlist(playlist).and_then(|list| {
            list.entries
                .iter()
                .enumerate()
                .skip(current + 1)
                .find(|(_, entry)| entry.raw["hidden"] != true)
        });
        if let Some((index, entry)) = next {
            ui.selected_playlist_id = playlist.into();
            if crate::collections::selection::click_entry_row(
                content,
                ui,
                &entry.id,
                index,
                Default::default(),
            ) {
                ui.playlist_reveal = Some((playlist.into(), entry.id.clone()));
            }
        }
    });
}
/// The cue after (or before) the retained next anchor, or the first (last) enabled cue when the anchor
/// is not in this document.
pub(super) fn navigation_target(
    cues: &[Value],
    item_id: &str,
    anchor: &Value,
    forward: bool,
) -> Option<usize> {
    let start = (anchor["itemId"] == item_id)
        .then(|| {
            cues.iter().position(|cue| {
                if !anchor["cueInstanceId"].is_null() {
                    cue["arrangementInstanceId"] == anchor["cueInstanceId"]
                } else {
                    cue["id"] == anchor["slideId"]
                }
            })
        })
        .flatten();
    let enabled = cues
        .iter()
        .enumerate()
        .filter(|(_, cue)| cue["disabled"] != true)
        .map(|(index, _)| index);
    if forward {
        enabled
            .into_iter()
            .find(|index| start.is_none_or(|start| *index > start))
    } else {
        enabled
            .rev()
            .find(|index| start.is_none_or(|start| *index < start))
    }
}
fn cross_document(runtime: &Runtime, direction: i8, cx: &mut App) {
    if direction > 0 && continuous(runtime, cx) {
        let live = &runtime.output.read(cx).live;
        let playlist = live["livePlaylistId"]
            .as_str()
            .or_else(|| live["presentationNextAnchor"]["playlistId"].as_str());
        let entry = live["livePlaylistEntryId"]
            .as_str()
            .or_else(|| live["presentationNextAnchor"]["playlistEntryId"].as_str());
        if let Some((playlist, index)) = playlist.zip(entry).and_then(|(playlist, id)| {
            let list = runtime.content.read(cx).playlist(playlist)?;
            list.entries
                .iter()
                .position(|entry| entry.id == id)
                .map(|index| (playlist.to_string(), index))
        }) {
            reveal_next_playlist_header(runtime, &playlist, index, cx);
            return;
        }
    }
    let Some((id, index, playlist, library)) = model::adjacent_document(runtime, cx, direction)
    else {
        return;
    };
    runtime.edit_ui(cx, |content, ui| {
        ui.selected_playlist_id = playlist;
        ui.selected_library_id = library;
        if let Some(index) = index {
            if let Some(entry) = content
                .playlist(&ui.selected_playlist_id)
                .and_then(|playlist| playlist.entries.get(index))
            {
                crate::collections::selection::click_entry_row(
                    content,
                    ui,
                    &entry.id,
                    index,
                    Default::default(),
                );
            }
        } else if let Some(library) = content.library(&ui.selected_library_id) {
            if let Some(index) = content
                .sorted_document_ids(library)
                .iter()
                .position(|item| item == &id)
            {
                crate::collections::selection::click_document_row(
                    content,
                    ui,
                    &id,
                    false,
                    index,
                    Default::default(),
                );
            }
        }
    });
    let Some(seed) = model::seed(runtime.content.read(cx), runtime.ui.read(cx)) else {
        return;
    };
    take_document_edge(runtime, seed, direction, false, cx);
}

fn take_document_edge(
    runtime: &Runtime,
    seed: model::DocumentSeed,
    direction: i8,
    header_step: bool,
    cx: &mut App,
) {
    let workspace = runtime.content.read(cx).workspace_path.clone();
    let revision = runtime.output.read(cx).committed_revision;
    let id = seed.item.id.clone();
    let occurrence = selected_occurrence(runtime, cx);
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        let result = cx.background_spawn(async move { seed.resolve() }).await;
        cx.update(|cx| {
            if runtime.content.read(cx).workspace_path != workspace
                || runtime.ui.read(cx).selected_library_item_id != id
                || selected_occurrence(&runtime, cx) != occurrence
                || runtime.output.read(cx).committed_revision != revision
            {
                return;
            }
            match result {
                Ok(document) => {
                    let index = if direction > 0 {
                        document.cues.iter().position(|cue| cue["disabled"] != true)
                    } else {
                        document
                            .cues
                            .iter()
                            .rposition(|cue| cue["disabled"] != true)
                    };
                    if let Some(index) = index {
                        take(&runtime, &document, index, false, cx);
                    } else if header_step {
                        if let Some((playlist, index)) = &occurrence {
                            reveal_next_playlist_header(&runtime, playlist, *index, cx);
                        }
                    }
                }
                Err(failure) => error(&runtime, cx, failure),
            }
        });
    })
    .detach();
}

pub fn clear_patch(live: &mut Value, layer: &str) {
    match layer {
        "slide" => {
            for key in [
                "liveItemId",
                "liveSlideId",
                "liveItem",
                "liveSlide",
                "liveCueInstanceId",
                "livePlaylistId",
                "livePlaylistEntryId",
            ] {
                live[key] = Value::Null;
            }
        }
        "media" | "background" => {
            live["backgroundId"] = json!("");
            live["backgroundSelectionId"] =
                json!(live["backgroundSelectionId"].as_u64().unwrap_or(0) + 1);
            live["backgroundPlaybackState"] = Value::Null;
            live["transportPlaybackPaused"]["media"] = json!(false);
        }
        "music" | "audio" => {
            live["audioId"] = Value::Null;
        }
        "videoInput" | "video-input" => {
            live["liveVideoInputId"] = Value::Null;
        }
        "stage-messages" => {
            live["stageMessage"] = json!({"text":"","state":"cleared"});
        }
        "props" | "prop" | "messages" | "announcements" => {
            let layer = if layer == "prop" { "props" } else { layer };
            if let Some(screens) = live["screenConfig"].as_object_mut() {
                for screen in screens.values_mut() {
                    screen["lookLayers"][layer] = json!(false);
                }
            }
        }
        _ => {}
    }
}
pub(crate) fn edit_native(
    runtime: &Runtime,
    cx: &mut App,
    edit: impl FnOnce(&mut Value) -> Result<(), String> + Send + 'static,
) {
    let path = runtime.content.read(cx).workspace_path.clone();
    let expected = path.clone();
    let host = runtime.host.clone();
    let answer = runtime
        .lanes
        .command(&path, move || host.edit_operator_output(&expected, edit));
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        let result = answer
            .await
            .unwrap_or_else(|_| Err("The output command worker stopped.".into()));
        cx.update(|cx| {
            if runtime.content.read(cx).workspace_path != path {
                return;
            }
            match result {
                Ok(response) => runtime.output.update(cx, |output, cx| {
                    if response["revision"]
                        .as_u64()
                        .is_some_and(|revision| revision >= output.committed_revision)
                    {
                        if let Err(error) = output.accept_native_patch(&response) {
                            output.error = Some(error);
                        }
                        cx.notify();
                    }
                }),
                Err(failure) => error(&runtime, cx, failure),
            }
        });
    })
    .detach();
}

pub fn clear(runtime: &Runtime, layer: &str, cx: &mut App) {
    if layer == "announcements" {
        edit_native(runtime, cx, |live| {
            live["announcements"] = Value::Null;
            Ok(())
        });
        return;
    }
    if matches!(layer, "props" | "messages") {
        crate::output::windows::mutate_config(
            runtime,
            json!({"type":"clearLookLayer","layerId":layer}),
            cx,
        );
        return;
    }
    let layer = layer.to_string();
    let logo = runtime.output.read(cx).settings["generalSettings"]["logoImageUrl"]
        .as_str()
        .is_some_and(|url| !url.is_empty());
    edit_native(runtime, cx, move |live| {
        if layer == "all" || layer == "logo" {
            for layer in ["slide", "media", "music", "videoInput"] {
                clear_patch(live, layer);
            }
            live["liveBuildStepStartedAtMsByStep"] = json!({});
            live["transportPlaybackPaused"] = json!({"media":false,"music":false});
            if layer != "logo" || !logo {
                clear_patch(live, "stage-messages");
            }
        } else {
            clear_patch(live, &layer);
        }
        if matches!(layer.as_str(), "all" | "logo" | "slide") {
            live["presentationNextAnchor"] = Value::Null;
        }
        if layer == "logo" && logo {
            live["backgroundId"] = json!("__presenter-orbit-logo");
            live["backgroundScale"] = json!("fit");
            live["backgroundPlayback"] = json!("stop");
            if let Some(screens) = live["screenConfig"].as_object_mut() {
                for screen in screens.values_mut() {
                    screen["lookLayers"]["media"] = json!(true);
                }
            }
        }
        Ok(())
    });
}
pub fn clear_group_patch(live: &mut Value, group: &Value) -> Result<(), String> {
    let layers = group["layers"]
        .as_array()
        .ok_or("The clear group has no layer list.")?;
    for layer in layers {
        match layer
            .as_str()
            .ok_or("A clear-group layer is not a string.")?
        {
            "slide" if group["stopPresentationTimeline"] == false => {
                if let Some(screens) = live["screenConfig"].as_object_mut() {
                    for screen in screens.values_mut() {
                        screen["lookLayers"]["slide"] = json!(false);
                    }
                }
            }
            "slide" => {
                live["presentationNextAnchor"] = if group["clearPresentationNext"] == true
                    || live["liveItemId"].is_null()
                {
                    Value::Null
                } else {
                    json!({"itemId":live["liveItemId"],"slideId":live["liveSlideId"],"cueInstanceId":live["liveCueInstanceId"],"playlistId":live["livePlaylistId"],"playlistEntryId":live["livePlaylistEntryId"]})
                };
                clear_patch(live, "slide");
                live["timelineStartedAtMs"] = Value::Null;
                live["timelineTransport"] = Value::Null;
                live["liveBuildStepStartedAtMsByStep"] = json!({});
            }
            "announcements" if group["stopAnnouncementsTimeline"] != false => {
                live["announcements"] = Value::Null;
            }
            "audio-effects" => {}
            layer @ ("background" | "prop" | "audio" | "messages" | "stage-messages"
            | "announcements" | "video-input") => clear_patch(live, layer),
            _ => return Err("The clear group contains an unsupported output layer.".into()),
        }
    }
    Ok(())
}
pub fn clear_group(runtime: &Runtime, id: &str, cx: &mut App) {
    let output = runtime.output.read(cx);
    let Some(snapshot) = output.snapshot.clone() else {
        return;
    };
    let Some(group) = snapshot["clearGroups"]
        .as_array()
        .and_then(|groups| groups.iter().find(|group| group["id"] == id))
        .cloned()
    else {
        error(runtime, cx, "The clear group is unavailable.".into());
        return;
    };
    edit_native(runtime, cx, move |live| clear_group_patch(live, &group));
}

pub fn mutate(runtime: &Runtime, item_id: &str, operation: Value, cx: &mut App) {
    mutate_checked(runtime, item_id, operation, cx).detach();
}

pub fn set_slide_labels(
    runtime: &Runtime,
    cx: &mut App,
    item: &str,
    slides: Vec<String>,
    label: Option<(String, String)>,
) {
    let value = label
        .as_ref()
        .map(|(name, color)| json!({"name":name,"color":color}));
    let mutation = json!({
        "type":"mutateItem","itemId":item,
        "operation":{"type":"setSlidesLabel","slideIds":slides,"label":value}
    });
    let item = item.to_string();
    runtime
        .save_preview(cx, Kind::Library, mutation, move |content| {
            edit_item(content, &item, |item| {
                for slide in item["slides"].as_array_mut().into_iter().flatten() {
                    if !slide["id"]
                        .as_str()
                        .is_some_and(|id| slides.iter().any(|slide| slide == id))
                    {
                        continue;
                    }
                    if let Some(fields) = slide.as_object_mut() {
                        if let Some((name, color)) = &label {
                            fields.insert("label".into(), json!(name));
                            fields.insert("labelColor".into(), json!(color));
                        } else {
                            fields.remove("label");
                            fields.remove("labelColor");
                        }
                    }
                }
            });
        })
        .detach();
}

pub fn delete_selected(runtime: &Runtime, item: &str, ids: Vec<String>, cx: &mut App) {
    let item = item.to_string();
    let operation = json!({"type":"deleteSlides","slideIds":ids});
    let mutation = json!({"type":"mutateItem","itemId":item,"operation":operation});
    runtime
        .save_preview(cx, Kind::Library, mutation, move |content| {
            let Some(current) = content.items.get(&item) else {
                return;
            };
            let mut preview = (*current.value).clone();
            if orbit_host::authoring::preview_slide_deletion(&mut preview, &operation).is_ok() {
                edit_item(content, &item, |value| *value = preview);
            }
        })
        .detach();
}

pub fn set_selected_enabled(
    runtime: &Runtime,
    item: &str,
    ids: Vec<String>,
    enabled: bool,
    cx: &mut App,
) {
    let operation = match ids.as_slice() {
        [id] => json!({"type":"setSlideEnabled","slideId":id,"enabled":enabled}),
        _ => json!({"type":"setSlidesEnabled","slideIds":ids,"enabled":enabled}),
    };
    let refresh = runtime.output.read(cx).live["liveItemId"] == item;
    let item = item.to_string();
    let mutation = json!({"type":"mutateItem","itemId":item,"operation":operation});
    runtime
        .save_preview(cx, Kind::Library, mutation, move |content| {
            edit_item(content, &item, |value| {
                if let Some(slides) = value["slides"].as_array_mut() {
                    for slide in slides
                        .iter_mut()
                        .filter(|slide| ids.iter().any(|id| slide["id"] == *id))
                    {
                        if let Some(object) = slide.as_object_mut() {
                            if enabled {
                                object.remove("disabled");
                            } else {
                                object.insert("disabled".into(), json!(true));
                            }
                        }
                    }
                }
            });
        })
        .detach();
    if refresh {
        refresh_live(runtime, cx);
    }
}

/// `mutate` that first applies `change` to the item's local value, and puts the previous value back when the host
/// rejects the mutation.
pub fn drop_media(runtime: &Runtime, item_id: &str, operation: Value, cx: &mut App) {
    let id = item_id.to_string();
    let mutation = json!({"type":"mutateItem","itemId":id,"operation":operation});
    runtime
        .save_preview(cx, Kind::Library, mutation, move |content| {
            let workspace = content.workspace.clone();
            let Some(item) = content.items.get(&id) else {
                return;
            };
            let mut preview = item.value.as_ref().clone();
            if orbit_host::authoring::preview_media_edit(&workspace, &mut preview, &operation)
                .is_ok()
            {
                edit_item(content, &id, |item| *item = preview);
            }
        })
        .detach();
}

pub fn mutate_optimistic(
    runtime: &Runtime,
    item_id: &str,
    operation: Value,
    change: impl FnOnce(&mut Value) + 'static,
    cx: &mut App,
) {
    mutate_optimistic_checked(runtime, item_id, operation, change, cx).detach();
}

/// `mutate_optimistic` that reports whether the host accepted the mutation.
pub fn mutate_optimistic_checked(
    runtime: &Runtime,
    item_id: &str,
    operation: Value,
    change: impl FnOnce(&mut Value) + 'static,
    cx: &mut App,
) -> gpui::Task<bool> {
    let Some(before) = runtime
        .content
        .read(cx)
        .items
        .get(item_id)
        .map(|item| (*item.value).clone())
    else {
        return cx.spawn(async move |_| false);
    };
    let id = item_id.to_string();
    runtime.content.update(cx, |content, cx| {
        edit_item(content, &id, change);
        cx.notify();
    });
    let request = Persist::new(
        Kind::Library,
        json!({"type":"mutateItem","itemId":id,"operation":operation}),
        |_, _| {},
    );
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        if runtime.persist(cx, request).await.is_some() {
            return true;
        }
        cx.update(|cx| {
            runtime.content.update(cx, |content, cx| {
                edit_item(content, &id, |value| *value = before);
                cx.notify();
            })
        });
        false
    })
}

/// `None` when the host rejected the mutation or the workspace changed meanwhile.
pub fn mutate_checked(
    runtime: &Runtime,
    item_id: &str,
    operation: Value,
    cx: &mut App,
) -> gpui::Task<Option<Value>> {
    let id = item_id.to_string();
    let request = Persist::new(
        Kind::Library,
        json!({"type":"mutateItem","itemId":id,"operation":operation}),
        |_, _| {},
    );
    let runtime = runtime.clone();
    cx.spawn(async move |cx| runtime.persist(cx, request).await)
}

/// Authoring commits refresh a pinned live item and retain its exact arrangement occurrence where possible.
pub fn refresh_live(runtime: &Runtime, cx: &mut App) {
    let live = runtime.output.read(cx).live.clone();
    let Some(id) = live["liveItemId"].as_str() else {
        return;
    };
    let content = runtime.content.read(cx);
    let Some(item) = content.items.get(id).cloned() else {
        clear(runtime, "slide", cx);
        return;
    };
    let entry = live["livePlaylistId"]
        .as_str()
        .and_then(|id| content.playlist(id))
        .and_then(|playlist| {
            playlist
                .entries
                .iter()
                .find(|entry| Some(entry.id.as_str()) == live["livePlaylistEntryId"].as_str())
        });
    let arrangement = entry
        .map(|entry| entry.arrangement_id.as_str())
        .unwrap_or_else(|| model::selected_arrangement_id(&item.value));
    let arrangement = arrangement.to_string();
    let occurrence = entry.map(|entry| entry.id.clone());
    let playlist = live["livePlaylistId"].clone();
    let entry_id = live["livePlaylistEntryId"].clone();
    edit_native(runtime, cx, move |current| {
        if current["liveItemId"] != item.id
            || current["livePlaylistId"] != playlist
            || current["livePlaylistEntryId"] != entry_id
        {
            return Ok(());
        }
        let cues =
            orbit_host::Host::presentation_cues(&item.value, &arrangement, occurrence.as_deref())?;
        let cue = cues
            .iter()
            .find(|cue| {
                !current["liveCueInstanceId"].is_null()
                    && cue["arrangementInstanceId"] == current["liveCueInstanceId"]
            })
            .or_else(|| cues.iter().find(|cue| cue["id"] == current["liveSlideId"]))
            .or_else(|| cues.iter().find(|cue| cue["disabled"] != true));
        let mut item_value = (*item.value).clone();
        item_value["selectedArrangementId"] = json!(arrangement);
        orbit_host::Host::refresh_live_slide_media(current, cue.unwrap_or(&Value::Null))?;
        current["liveItem"] = item_value;
        current["liveSlide"] = cue.cloned().unwrap_or(Value::Null);
        current["liveSlideId"] = cue.map_or(Value::Null, |cue| cue["id"].clone());
        current["liveCueInstanceId"] = cue
            .and_then(|cue| cue.get("arrangementInstanceId"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(())
    });
}
