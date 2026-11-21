use super::*;
use crate::workspace_import::{same_title, Duplicate};

/// Source picked by an import that stopped to ask how duplicate titles are handled, keyed by workspace.
static PENDING_DUPLICATE_IMPORT: Mutex<Option<(PathBuf, PathBuf)>> = Mutex::new(None);

pub(super) fn export(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let item = argument(args, 1)?
        .as_str()
        .ok_or("Presentation id is required.")?
        .to_string();
    let (revision, reviewed) = {
        let mut io = lock_workspace_io(state)?;
        io.require_active(&path)?;
        (
            io.revision(&path),
            portable_bundle::prepare(&workspace::read_workspace(&path)?, &item)?,
        )
    };
    let Some(destination) = FileDialog::new()
        .set_title("Export portable presentation bundle (new file)")
        .add_filter("Orbit presentation bundle", &["orbitbundle"])
        .set_file_name("presentation.orbitbundle")
        .save_file()
    else {
        return Ok(json!({"canceled":true}));
    };
    let mut io = lock_workspace_io(state)?;
    io.require_revision(&path, revision)?;
    let current = portable_bundle::prepare(&workspace::read_workspace(&path)?, &item)?;
    if current != reviewed {
        return Err("Presentation or assets changed after bundle review. Export again.".into());
    }
    portable_bundle::write(&current, &destination)?;
    Ok(json!({"canceled":false,"destinationPath":destination}))
}

pub(super) fn import(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let library_id = argument(args, 1)?
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("Destination library id is required.")?
        .to_string();
    let revision = expected_workspace_revision(args, 2)?;
    let duplicate = args
        .get(3)
        .and_then(Value::as_str)
        .map(Duplicate::parse)
        .transpose()?;
    {
        let mut io = lock_workspace_io(state)?;
        io.require_revision(&path, revision)?;
    }
    let source = match duplicate {
        Some(_) => PENDING_DUPLICATE_IMPORT
            .lock()
            .map_err(|_| "Pending import lock is poisoned.")?
            .take()
            .filter(|(workspace, _)| *workspace == path)
            .map(|(_, source)| source)
            .ok_or("No import is waiting for a duplicate choice.")?,
        None => {
            let Some(source) = FileDialog::new()
                .set_title("Import presentation bundle")
                .add_filter(
                    "Presentation bundle",
                    &["orbitbundle", "probundle", "proplaylist", "pptx"],
                )
                .pick_file()
            else {
                return Ok(json!({"canceled":true}));
            };
            source
        }
    };
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let bundle = if matches!(extension.as_str(), "probundle" | "proplaylist" | "pptx") {
        None
    } else {
        Some(portable_bundle::read(&source)?)
    };
    if duplicate.is_none() {
        let titles = match &bundle {
            Some(bundle) => portable_bundle::title(bundle)
                .map(str::to_string)
                .into_iter()
                .collect(),
            None if extension == "pptx" => vec![workspace::pptx_title(&source).to_string()],
            None => workspace::pro_presenter_archive_titles(&source)?,
        };
        let current = workspace::read_workspace(&path)?;
        let duplicates = titles
            .into_iter()
            .filter(|title| same_title(&current, &library_id, title).is_some())
            .collect::<Vec<_>>();
        if !duplicates.is_empty() {
            *PENDING_DUPLICATE_IMPORT
                .lock()
                .map_err(|_| "Pending import lock is poisoned.")? = Some((path, source));
            return Ok(json!({"canceled":false,"duplicates":duplicates}));
        }
    }
    let duplicate = duplicate.unwrap_or(Duplicate::KeepBoth);
    let mut io = lock_workspace_io(state)?;
    io.require_revision(&path, revision)?;
    let mut report = Vec::new();
    let workspace = match bundle {
        Some(bundle) => portable_bundle::import(&path, &bundle, &library_id, duplicate)?,
        None if extension == "pptx" => {
            let (workspace, losses) =
                workspace::import_pptx(&path, &source, &library_id, duplicate)?;
            report = losses;
            workspace
        }
        None => workspace::import_pro_presenter_archive(&path, &source, &library_id, duplicate)?,
    };
    let revision = io.advance_revision(&path);
    drop(io);
    if !report.is_empty() {
        let shown = report.iter().take(20).cloned().collect::<Vec<_>>().join(
            "
",
        );
        let more = report.len().saturating_sub(20);
        MessageDialog::new()
            .set_title("PowerPoint import report")
            .set_description(format!(
                "Not imported or approximated:
{shown}{}",
                if more > 0 {
                    format!(
                        "
(+{more} more, see the workspace import warnings)"
                    )
                } else {
                    String::new()
                }
            ))
            .set_level(MessageLevel::Info)
            .set_buttons(MessageButtons::Ok)
            .show();
    }
    Ok(json!({"canceled":false,"workspace":workspace,"revision":revision}))
}
