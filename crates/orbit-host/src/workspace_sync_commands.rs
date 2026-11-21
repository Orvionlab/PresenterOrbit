use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    repository: String,
    direction: String,
    scopes: Vec<String>,
    replace: bool,
}

pub(super) fn select_repository() -> Result<Value, String> {
    let selected = FileDialog::new()
        .set_title("Choose dedicated Orbit synchronization repository")
        .pick_folder();
    Ok(json!({ "path": selected }))
}

pub(super) fn run(
    app: &Host,
    state: &AppState,
    args: &[Value],
    apply: bool,
) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let request: Request = serde_json::from_value(argument(args, 1)?)
        .map_err(|error| format!("Invalid synchronization request: {error}"))?;
    let mut io = lock_workspace_io(state)?;
    io.require_active(&path)?;
    let repository = Path::new(&request.repository);
    if !apply {
        return workspace_sync::preview(
            &path,
            repository,
            &request.direction,
            &request.scopes,
            request.replace,
        );
    }
    let token = argument(args, 2)?;
    let token = token
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or("A synchronization preview token is required.")?;
    let workspace = workspace_sync::apply(
        &path,
        repository,
        &request.direction,
        &request.scopes,
        request.replace,
        token,
    )?;
    let revision = if request.direction == "download" {
        io.advance_revision(&path)
    } else {
        io.revision(&path)
    };
    if request.direction == "download" {
        app.emit(
            "workspace-sync-changed",
            json!({
                "workspacePath": path, "revision": revision,
                "scopes": request.scopes, "workspace": workspace,
            }),
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(json!({ "workspace": workspace, "revision": revision }))
}
