//! Native cue identities mapped to Network Link's authored indexes.
use serde_json::{json, Value};
pub fn cue(workspace: &Value, live: &Value) -> Result<Option<Value>, String> {
    let mut items = workspace["library"].as_array().into_iter().flatten();
    let Some(item) = items.find(|i| i["id"] == live["liveItemId"]) else {
        return Ok(None);
    };
    let playlists = workspace["playlist"].as_array().into_iter().flatten();
    if let Some((playlist_index, playlist)) = playlists
        .enumerate()
        .find(|(_, p)| p["id"] == live["livePlaylistId"])
    {
        let entries = playlist["items"].as_array().into_iter().flatten();
        let (entry_index, entry) = entries
            .enumerate()
            .find(|(_, e)| e["id"] == live["livePlaylistEntryId"])
            .ok_or("Network Link playlist occurrence is unavailable.")?;
        let arrangement = live["liveItem"]["selectedArrangementId"]
            .as_str()
            .or_else(|| entry["arrangementId"].as_str())
            .unwrap_or_else(|| crate::cues::model::selected_arrangement_id(item));
        let cues = orbit_host::Host::presentation_cues(item, arrangement, entry["id"].as_str())?;
        let index = index(&cues, live)?;
        return Ok(Some(
            json!({"type":"playlist","playlistIndex":playlist_index,"entryIndex":entry_index,"slideIndex":index}),
        ));
    }
    let libraries = workspace["libraries"].as_array().into_iter().flatten();
    let Some((library_index, presentation_index)) = libraries.enumerate().find_map(|(i, l)| {
        l["itemIds"]
            .as_array()
            .and_then(|ids| ids.iter().position(|id| *id == item["id"]))
            .map(|p| (i, p))
    }) else {
        return Err("Network Link library membership is unavailable.".into());
    };
    let cues = orbit_host::Host::presentation_cues(
        item,
        crate::cues::model::selected_arrangement_id(item),
        None,
    )?;
    Ok(Some(
        json!({"type":"library","libraryIndex":library_index,"presentationIndex":presentation_index,"slideIndex":index(&cues,live)?}),
    ))
}
fn index(cues: &[Value], live: &Value) -> Result<usize, String> {
    cues.iter()
        .position(|s| {
            s["id"] == live["liveSlideId"]
                && (live["liveCueInstanceId"].is_null()
                    || s["arrangementInstanceId"] == live["liveCueInstanceId"])
        })
        .ok_or_else(|| "Network Link cue occurrence is unavailable.".into())
}
