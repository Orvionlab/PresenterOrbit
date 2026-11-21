use serde_json::{json, Value};
use std::path::Path;
use uuid::Uuid;

pub fn require_destination(
    workspace: &Value,
    library_id: &str,
    playlist_id: &str,
) -> Result<(), String> {
    if library_id.is_empty()
        || !workspace["libraries"]
            .as_array()
            .ok_or("Workspace libraries are required.")?
            .iter()
            .any(|library| library["id"].as_str() == Some(library_id))
    {
        return Err("Choose an existing destination library for the Keynote cue.".into());
    }
    if playlist_id.is_empty()
        || !workspace["playlist"]
            .as_array()
            .ok_or("Workspace playlists are required.")?
            .iter()
            .any(|playlist| {
                playlist["id"].as_str() == Some(playlist_id) && playlist["type"] != "folder"
            })
    {
        return Err(
            "Choose an existing playlist, rather than a folder, for the Keynote cue.".into(),
        );
    }
    Ok(())
}

pub fn insert(
    workspace_path: &Path,
    library_id: &str,
    playlist_id: &str,
    source: &Path,
) -> Result<(Value, String, String), String> {
    let current = crate::workspace::read_workspace(workspace_path)?;
    require_destination(&current, library_id, playlist_id)?;
    let source = crate::external_presentation::keynote_source_path(source)?;
    let title = source
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .ok_or("Keynote file name must be nonempty Unicode.")?;
    let url = url::Url::from_file_path(&source)
        .map_err(|_| "Could not create a local Keynote file URL.")?;
    let item_id = Uuid::new_v4().to_string();
    let entry_id = Uuid::new_v4().to_string();
    let theme_id = Uuid::new_v4().to_string();
    let mut libraries = required_array(&current, "libraries")?;
    let library = libraries
        .iter_mut()
        .find(|entry| entry["id"].as_str() == Some(library_id))
        .ok_or("Destination library is unavailable.")?;
    library["itemIds"]
        .as_array_mut()
        .ok_or("Destination library itemIds are required.")?
        .push(json!(item_id));
    let mut playlists = required_array(&current, "playlist")?;
    let playlist = playlists
        .iter_mut()
        .find(|entry| entry["id"].as_str() == Some(playlist_id))
        .ok_or("Destination playlist is unavailable.")?;
    playlist["items"]
        .as_array_mut()
        .ok_or("Destination playlist entries are required.")?
        .push(json!({"id":entry_id,"type":"presentation","itemId":item_id}));
    playlist["itemIds"]
        .as_array_mut()
        .ok_or("Destination playlist itemIds are required.")?
        .push(json!(item_id));
    let mut items = required_array(&current, "library")?;
    items.push(json!({"id":item_id,"title":title,"sourceType":"presenter-orbit","sourcePath":"",
        "importedFrom":{"format":"keynote-external","filePath":source},
        "slides":[{"id":Uuid::new_v4().to_string(),"name":title,"label":1,"content":"",
            "width":1920,"height":1080,"layers":[],"background":"#000000","backgroundEnabled":true,
            "presentationThemeId":format!("{theme_id}::external"),
            "notes":format!("Keynote: {}", source.display()),
            "actions":[{"id":Uuid::new_v4().to_string(),"type":"external-presentation","enabled":true,
                "title":title,"details":{"url":url.as_str()}}]}]}));
    let mut themes = required_array(&current, "themes")?;
    themes.push(json!({"id":theme_id,"title":format!("Keynote / {title}"),"editable":true,
        "schemaVersion":2,"sourceType":"presenter-orbit","sourcePath":"","size":{"width":1920,"height":1080},
        "slides":[{"id":"external","name":"External presentation","width":1920,"height":1080,
            "background":"#000000","backgroundEnabled":true,"layers":[]}]}));
    let saved = crate::workspace::save_workspace(
        workspace_path,
        &json!({"libraries":libraries,"library":items,"playlist":playlists,"themes":themes,
            "lastUsedPresentationThemeId":current["lastUsedPresentationThemeId"]}),
    )?;
    Ok((saved, item_id, entry_id))
}

fn required_array(workspace: &Value, key: &str) -> Result<Vec<Value>, String> {
    workspace[key]
        .as_array()
        .cloned()
        .ok_or_else(|| format!("Workspace {key} must be an array."))
}
