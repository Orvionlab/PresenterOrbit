use super::*;

pub(super) fn choose(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let workspace_path = active_workspace(state, argument(args, 0)?)?;
    let revision = {
        let mut io = lock_workspace_io(state)?;
        io.require_active(&workspace_path)?;
        io.revision(&workspace_path)
    };
    let Some(source) = FileDialog::new()
        .set_title("Choose chord chart")
        .add_filter("Chord chart", &["pdf", "png", "jpg", "jpeg"])
        .pick_file()
    else {
        return Ok(json!({"canceled": true}));
    };
    let mut io = lock_workspace_io(state)?;
    io.require_revision(&workspace_path, revision)?;
    let directory = workspace_path
        .parent()
        .ok_or("Workspace directory is required.")?;
    let chart = chord_chart::import(directory, &source)?;
    Ok(json!({"canceled": false, "chart": chart}))
}

pub(super) fn choose_artwork(state: &AppState, args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("Artwork selection requires a workspace path.".into());
    }
    let path = active_workspace(state, argument(args, 0)?)?;
    let revision = {
        let mut io = lock_workspace_io(state)?;
        io.require_active(&path)?;
        io.revision(&path)
    };
    let Some(source) = FileDialog::new()
        .set_title("Choose presentation artwork")
        .add_filter("Image", &["png", "jpg", "jpeg"])
        .pick_file()
    else {
        return Ok(json!({"canceled":true}));
    };
    let metadata = std::fs::metadata(&source).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 4 * 1024 * 1024 {
        return Err("Artwork must be an image smaller than 4 MiB.".into());
    }
    let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
    let image = macro_image::validate(&bytes)?;
    lock_workspace_io(state)?.require_revision(&path, revision)?;
    Ok(json!({"canceled":false,"artwork":image["imageData"]}))
}
