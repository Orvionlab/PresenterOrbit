//! Art-Net fixture packets resolve every index before changing selections or executing actions.
use crate::{services::entries, state::runtime::Runtime};
use gpui::{App, AppContext, Global};
use serde_json::{json, Value};
#[derive(Default)]
struct Contexts(std::collections::HashMap<String, Value>);
impl Global for Contexts {}
fn indexed(list: &[Value], value: u64) -> Result<&Value, String> {
    value
        .checked_sub(1)
        .and_then(|n| list.get(n as usize))
        .ok_or_else(|| format!("Art-Net index {value} is unavailable."))
}
pub fn slots(event: &Value) -> Result<std::collections::BTreeMap<u64, u64>, String> {
    let mut result = std::collections::BTreeMap::new();
    for pair in event["slots"]
        .as_array()
        .ok_or("Art-Net fixture channels are required.")?
    {
        let values = pair
            .as_array()
            .filter(|v| v.len() == 2)
            .ok_or("Invalid Art-Net fixture channel.")?;
        let slot = values[0]
            .as_u64()
            .filter(|s| (1..=16).contains(s))
            .ok_or("Invalid Art-Net fixture channel.")?;
        let value = values[1]
            .as_u64()
            .filter(|v| (1..=255).contains(v))
            .ok_or("Invalid Art-Net fixture channel.")?;
        if result.insert(slot, value).is_some() {
            return Err("Invalid or duplicate Art-Net fixture channel.".into());
        }
    }
    Ok(result)
}
pub fn reset(cx: &mut App) {
    cx.set_global(Contexts::default());
}
pub fn dispatch(runtime: &Runtime, event: &Value, cx: &mut App) -> Result<(), String> {
    let slots = slots(event)?;
    let key = json!([
        event["workspacePath"],
        event["deviceId"],
        event["sessionId"]
    ])
    .to_string();
    let mut context = cx
        .default_global::<Contexts>()
        .0
        .get(&key)
        .cloned()
        .unwrap_or_else(|| json!({}));
    let workspace = runtime.content.read(cx).workspace.clone();
    let mut selected = runtime.ui.read(cx).selected_library_item_id.clone();
    let mut playlist_id = runtime.ui.read(cx).selected_playlist_id.clone();
    let mut selected_index = runtime
        .ui
        .read(cx)
        .selected_documents
        .first()
        .and_then(|s| s.index);
    let playlists = entries(&workspace["playlist"]);
    if let Some(value) = slots.get(&1) {
        let roots = playlists
            .iter()
            .filter(|p| p["parentId"].as_str().is_none_or(str::is_empty))
            .cloned()
            .collect::<Vec<_>>();
        let playlist = indexed(&roots, *value)?;
        if playlist["type"] == "folder" {
            return Err("Art-Net playlist index selects a folder.".into());
        }
        playlist_id = playlist["id"]
            .as_str()
            .ok_or("Playlist identity is required.")?
            .to_string();
        selected.clear();
        selected_index = None;
    }
    if let Some(value) = slots.get(&2) {
        let list = playlists
            .iter()
            .find(|p| p["id"] == playlist_id)
            .ok_or("Art-Net playlist is unavailable.")?;
        let items = entries(&list["items"]);
        let entry = indexed(&items, *value)?;
        if entry["type"] != "presentation" {
            return Err("Art-Net playlist item is not an available presentation.".into());
        }
        selected = entry["itemId"]
            .as_str()
            .ok_or("Presentation identity is required.")?
            .to_string();
        selected_index = Some((*value - 1) as usize);
    }
    let libraries = entries(&workspace["mediaLibraries"]);
    let media = entries(&workspace["media"]);
    for (slot, tab, audio) in [(3, "graphics", false), (4, "audio", true)] {
        if let Some(value) = slots.get(&slot) {
            let roots = libraries
                .iter()
                .filter(|l| {
                    l["parentId"].as_str().is_none_or(str::is_empty)
                        && (l["mediaType"] == "audio") == audio
                })
                .cloned()
                .collect::<Vec<_>>();
            let library = indexed(&roots, *value)?;
            if library["type"] == "folder" {
                return Err("Art-Net cannot select a media folder.".into());
            }
            context[tab] = library["id"].clone();
        }
    }
    let props = entries(&workspace["props"]);
    let mut folders = entries(&workspace["propFolders"]);
    for prop in &props {
        let folder = prop["folder"].as_str().unwrap_or("");
        if !folders.iter().any(|v| v == folder) {
            folders.push(json!(folder));
        }
    }
    if let Some(value) = slots.get(&8) {
        context["props"] = indexed(&folders, *value)?.clone();
    }
    if let Some(value) = slots.get(&10) {
        context["macros"] =
            indexed(&entries(&workspace["macroCollections"]), *value)?["id"].clone();
    }
    let mut actions = Vec::new();
    let id = uuid::Uuid::new_v4().to_string();
    for (slot, tab, audio) in [(6, "graphics", false), (11, "audio", true)] {
        if let Some(value) = slots.get(&slot) {
            let library = context[tab]
                .as_str()
                .ok_or("Select the Art-Net media/audio playlist before triggering its cue.")?;
            if !libraries.iter().any(|l| l["id"] == library) {
                return Err("Art-Net media library is unavailable.".into());
            }
            let items = crate::media::model::selection(&media, &libraries, audio, library)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>();
            actions.push(json!({"id":id,"type":"media","targetId":indexed(&items,*value)?["id"]}));
        }
    }
    if let Some(value) = slots.get(&7) {
        let folder = context["props"].as_str().unwrap_or("");
        let list = props
            .iter()
            .filter(|p| p["folder"].as_str().unwrap_or("") == folder)
            .cloned()
            .collect::<Vec<_>>();
        actions.push(json!({"id":id,"type":"prop","targetId":indexed(&list,*value)?["id"],"details":{"singleMode":true}}));
    }
    if let Some(value) = slots.get(&9) {
        let library = super::macros::Library::of(&workspace);
        let collection = super::macros::active_collection(
            library.collections,
            context["macros"].as_str().unwrap_or(""),
        );
        let list = super::macros::collection_macros(&library, &collection)?;
        actions.push(json!({"id":id,"type":"macro","targetId":indexed(&list,*value)?["id"]}));
    }
    if let Some(value) = slots.get(&5) {
        let items = entries(&workspace["library"]);
        let item = items
            .iter()
            .find(|i| i["id"] == selected)
            .ok_or("Art-Net presentation is unavailable.")?;
        let entry = playlists
            .iter()
            .find(|p| p["id"] == playlist_id)
            .and_then(|p| p["items"].as_array())
            .and_then(|items| selected_index.and_then(|i| items.get(i)));
        let arrangement = entry
            .and_then(|e| e["arrangementId"].as_str())
            .unwrap_or_else(|| crate::cues::model::selected_arrangement_id(item));
        let cues = orbit_host::Host::presentation_cues(
            item,
            arrangement,
            entry.and_then(|e| e["id"].as_str()),
        )?;
        let cue = indexed(&cues, *value)?;
        if cue["disabled"] == true {
            return Err("Art-Net cue index is disabled.".into());
        }
        let mut action = json!({"id":id,"type":"presentation","targetId":item["id"],"details":{"slideId":cue["id"],"cueInstanceId":cue["arrangementInstanceId"]}});
        if let Some(entry) = entry {
            action["type"] = json!("playlist");
            action["targetId"] = json!(playlist_id);
            action["details"]["itemId"] = entry["id"].clone();
        }
        actions.insert(0, action);
    }
    for (slot, command) in [(14, "start"), (15, "stop"), (16, "reset")] {
        if let Some(value) = slots.get(&slot) {
            actions.push(json!({"id":id,"type":"timer","targetId":indexed(&entries(&workspace["countdownTimers"]),*value)?["id"],"value":command}));
        }
    }
    let transition = workspace["slideTransition"].clone();
    let layers = workspace["layerTransitions"].clone();
    let scope = runtime.content.read(cx).workspace_path.clone();
    let load = runtime.content.read(cx).load_generation;
    let (runtime, event) = (runtime.clone(), event.clone());
    cx.spawn(async move|cx|{
        let result:Result<(),String>=async{
            if slots.contains_key(&12)||slots.contains_key(&13){
                let mut transition=transition;
                if let Some(value)=slots.get(&12){let host=runtime.host.clone();let catalog=cx.background_spawn(async move{host.invoke("getTransitionCatalog",vec![])}).await?;let effect=indexed(catalog.as_array().ok_or("Invalid transition catalog.")?,*value)?;transition=json!({"effectId":effect["id"],"durationSeconds":transition["durationSeconds"].as_f64().unwrap_or(0.6)});if effect["defaultDirection"].is_string(){transition["direction"]=effect["defaultDirection"].clone();}}
                if !transition.is_object(){return Err("Select a global transition before setting its Art-Net duration.".into());}
                if let Some(value)=slots.get(&13){transition["durationSeconds"]=json!(*value as f64/10.);}
                let mut layers=layers;layers["foreground"]=transition.clone();
                if !cx.update(|cx|runtime.content.read(cx).workspace_path==scope && runtime.content.read(cx).load_generation==load){return Ok(());}
                super::save::content(&runtime,cx,json!({"slideTransition":transition,"layerTransitions":layers})).await?;
            }
            cx.update(|cx|{
                if runtime.content.read(cx).workspace_path!=scope || runtime.content.read(cx).load_generation!=load || !super::inputs::current(&runtime,&event,cx){return Ok(());}
                if ["library","playlist","media","mediaLibraries","props","propFolders","macros","macroCollections","countdownTimers"].iter().any(|key|runtime.content.read(cx).workspace[*key]!=workspace[*key]){return Err("Art-Net targets changed while the packet was being prepared. Rearm and retry.".into());}
                runtime.edit_ui(cx,|_,ui|{ui.selected_library_item_id=selected.clone();ui.selected_playlist_id=playlist_id.clone();if let Some(index)=selected_index{ui.selected_document_source=Some(crate::state::slices::Source::Playlist);ui.selected_document_playlist_id=playlist_id;ui.selected_documents=vec![crate::state::slices::Selected{item_id:selected,index:Some(index)}];}});
                cx.default_global::<Contexts>().0.insert(key,context);super::run_operator(&runtime,actions,cx);
                if let Some(command)=event["controlCommand"].as_str(){super::inputs::command(&runtime,command,cx)?;}Ok(())
            })
        }.await;
        if let Err(error)=result{cx.update(|cx|crate::cues::actions::error(&runtime,cx,error));}
    }).detach();
    Ok(())
}
