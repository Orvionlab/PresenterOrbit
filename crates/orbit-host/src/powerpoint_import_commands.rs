use super::*;
use crate::Host;

pub(super) fn run(app: &Host, state: &AppState, args: &[Value]) -> Result<Value, String> {
    if !cfg!(any(windows, target_os = "macos")) {
        return Err(
            "PowerPoint image import requires Microsoft PowerPoint installed on Windows or macOS."
                .into(),
        );
    }
    let path = active_workspace(state, argument(args, 0)?)?;
    let library = argument(args, 1)?;
    let library = library
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("A destination library is required.")?;
    let expected_revision = expected_workspace_revision(args, 2)?;
    {
        let mut io = lock_workspace_io(state)?;
        io.require_revision(&path, expected_revision)?;
        powerpoint_import::require_library(&workspace::read_workspace(&path)?, library)?;
    }
    let Some(source) = FileDialog::new()
        .set_title("Import PowerPoint as images")
        .add_filter("PowerPoint", &["ppt", "pptx", "pps", "ppsx"])
        .pick_file()
    else {
        return Ok(json!({"canceled":true}));
    };
    let mut directory = powerpoint_import::ImportDirectory::create(&path)?;
    let images = app
        .state::<external_presentation::ExternalPresentationService>()
        .export_images(&source, &directory.path)?;
    let mut io = lock_workspace_io(state)?;
    io.require_revision(&path, expected_revision)?;
    let (workspace, item_id) =
        powerpoint_import::insert_images(&path, library, &source, &directory, &images)?;
    directory.commit();
    Ok(
        json!({"canceled":false,"workspace":workspace,"revision":io.advance_revision(&path),
        "itemId":item_id,"slideCount":images.slides.len(),"hiddenSlideCount":images.hidden_slide_count}),
    )
}
