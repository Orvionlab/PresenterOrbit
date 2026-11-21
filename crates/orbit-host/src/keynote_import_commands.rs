use super::*;

pub(super) fn run(state: &AppState, args: &[Value]) -> Result<Value, String> {
    if !cfg!(target_os = "macos") {
        return Err("Keynote presentation cues require macOS with Keynote installed.".into());
    }
    let path = active_workspace(state, argument(args, 0)?)?;
    let library = required_id(args, 1)?;
    let playlist = required_id(args, 2)?;
    let expected = expected_workspace_revision(args, 3)?;
    {
        let mut io = lock_workspace_io(state)?;
        io.require_revision(&path, expected)?;
        keynote_import::require_destination(
            &workspace::read_workspace(&path)?,
            &library,
            &playlist,
        )?;
    }
    let Some(source) = FileDialog::new()
        .set_title("Add Keynote to playlist")
        .add_filter("Keynote", &["key"])
        .pick_file()
    else {
        return Ok(json!({"canceled":true}));
    };
    let mut io = lock_workspace_io(state)?;
    io.require_revision(&path, expected)?;
    let (workspace, item_id, entry_id) =
        keynote_import::insert(&path, &library, &playlist, &source)?;
    Ok(
        json!({"canceled":false,"workspace":workspace,"revision":io.advance_revision(&path),
        "itemId":item_id,"playlistEntryId":entry_id}),
    )
}

fn required_id(args: &[Value], index: usize) -> Result<String, String> {
    argument(args, index)?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Keynote cue library and playlist identifiers are required.".into())
}
