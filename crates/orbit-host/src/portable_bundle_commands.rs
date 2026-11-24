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
        None => {
            let (workspace, warnings) = workspace::import_pro_presenter_archive(
                &path,
                &source,
                &library_id,
                None,
                duplicate,
            )?;
            report = warnings;
            workspace
        }
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
            .set_title(if extension == "pptx" {
                "PowerPoint import report"
            } else {
                "ProPresenter import report"
            })
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

const PRO_PRESENTER: &[&str] = &["pro", "probundle", "proplaylist"];
const TEXT: &[&str] = &["txt", "rtf", "md", "csv"];

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension().is_some_and(|extension| {
        extensions
            .iter()
            .any(|kind| extension.eq_ignore_ascii_case(kind))
    })
}

pub(super) fn select_files(pro_presenter_only: bool) -> Result<Value, String> {
    let extensions = if pro_presenter_only {
        PRO_PRESENTER.to_vec()
    } else {
        [TEXT, PRO_PRESENTER].concat()
    };
    let Some(paths) = FileDialog::new()
        .set_title("Import presentation files")
        .add_filter(
            if pro_presenter_only {
                "ProPresenter files"
            } else {
                "Presentation files"
            },
            &extensions,
        )
        .pick_files()
    else {
        return Ok(json!({"canceled": true}));
    };
    if paths.iter().any(|path| !has_extension(path, &extensions)) {
        return Err("Unsupported presentation file type.".into());
    }
    if let [path] = paths.as_slice() {
        if has_extension(path, TEXT) {
            return Ok(
                json!({"type": "text", "filePath": path, "text": local_text::read_text(path)?}),
            );
        }
    }
    Ok(json!({"type": "files", "paths": paths}))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PresentationFilesImport {
    sources: Vec<PathBuf>,
    library_id: String,
    playlist_id: Option<String>,
}

pub(super) fn import_files(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let request: PresentationFilesImport =
        serde_json::from_value(argument(args, 1)?).map_err(|error| error.to_string())?;
    if request.sources.is_empty()
        || request.sources.iter().any(|source| {
            !source.is_absolute()
                || !(has_extension(source, PRO_PRESENTER) || has_extension(source, TEXT))
        })
        || request.library_id.trim().is_empty()
        || request
            .playlist_id
            .as_ref()
            .is_some_and(|id| id.trim().is_empty())
    {
        return Err(
            "Presentation import requires supported absolute file paths and valid destination IDs."
                .into(),
        );
    }
    let revision = expected_workspace_revision(args, 2)?;
    let mut io = lock_workspace_io(state)?;
    io.require_revision(&path, revision)?;
    let mut current = workspace::read_workspace(&path)?;
    if !current["libraries"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|library| library["id"] == request.library_id)
    {
        return Err("Selected destination library is missing.".into());
    }
    if let Some(id) = &request.playlist_id {
        if !current["playlist"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|playlist| playlist["id"] == *id && playlist["type"] != "folder")
        {
            return Err("Selected destination playlist is missing.".into());
        }
    }
    let mut imported_files = 0;
    let mut warnings = Vec::new();
    let mut failures = Vec::new();
    for source in &request.sources {
        let result = if has_extension(source, PRO_PRESENTER) {
            workspace::import_pro_presenter_archive(
                &path,
                source,
                &request.library_id,
                request.playlist_id.as_deref(),
                Duplicate::KeepBoth,
            )
        } else {
            import_text_file(&path, source, &request).map(|current| (current, Vec::new()))
        };
        match result {
            Ok((updated, notices)) => {
                current = updated;
                imported_files += 1;
                warnings.extend(notices);
            }
            Err(error) => failures.push(format!("{}: {error}", source.display())),
        }
    }
    let revision = if imported_files > 0 {
        io.advance_revision(&path)
    } else {
        revision
    };
    Ok(
        json!({"workspace": current, "importedFiles": imported_files, "warnings": warnings, "failures": failures, "revision": revision}),
    )
}

fn import_text_file(
    workspace_path: &Path,
    source: &Path,
    request: &PresentationFilesImport,
) -> Result<Value, String> {
    let text = local_text::read_text(source)?;
    if text.trim().is_empty() {
        return Err("The selected file contains no presentation text.".into());
    }
    let title = source
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or("Presentation file name is invalid.")?;
    let mut mutation = json!({"type": "createPresentation", "libraryId": request.library_id,
        "title": title, "content": text.trim(), "splitMode": "paragraph", "splitMarker": "", "splitUnits": 1});
    if let Some(id) = &request.playlist_id {
        mutation["playlistId"] = json!(id);
    }
    Ok(workspace::mutate_presentation_library(workspace_path, &mutation)?.workspace)
}
