use super::*;

pub(super) fn run(state: &AppState, args: &[Value], print: bool) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let request: handout::HandoutRequest = serde_json::from_value(argument(args, 1)?)
        .map_err(|error| format!("Invalid handout options: {error}"))?;
    let (revision, reviewed) = {
        let mut io = lock_workspace_io(state)?;
        io.require_active(&path)?;
        (
            io.revision(&path),
            handout::prepare(&workspace::read_workspace(&path)?, &request)?,
        )
    };
    let frames = handout::render(&reviewed, &request)?;
    #[cfg(windows)]
    if print {
        let submitted = crate::handout_print::print(&frames, || {
            let mut io = lock_workspace_io(state)?;
            io.require_revision(&path, revision)?;
            let current = handout::prepare(&workspace::read_workspace(&path)?, &request)?;
            if current != reviewed {
                return Err(
                    "Handout content or assets changed while choosing the printer. Print again."
                        .into(),
                );
            }
            Ok(())
        })?;
        return Ok(match submitted {
            Some((job_id, pages)) => {
                json!({"canceled":false,"printSubmitted":true,"pageCount":pages,"printJobId":job_id})
            }
            None => json!({"canceled":true}),
        });
    }
    #[cfg(not(windows))]
    let _ = frames;
    let Some(destination) = FileDialog::new()
        .set_title(if print {
            "Save handout PDF and submit to printer"
        } else {
            "Export handout PDF (new file)"
        })
        .add_filter("PDF handout", &["pdf"])
        .set_file_name("handout.pdf")
        .save_file()
    else {
        return Ok(json!({"canceled":true}));
    };
    let page_count = {
        let mut io = lock_workspace_io(state)?;
        io.require_revision(&path, revision)?;
        let current = handout::prepare(&workspace::read_workspace(&path)?, &request)?;
        if current != reviewed {
            return Err("Handout content or assets changed after preflight. Export again.".into());
        }
        let frames = handout::render(&current, &request)?;
        presentation_export::write_new(
            &destination,
            &presentation_export::encode_pdf_with_page_scale(&frames, 0.75)?,
        )?;
        frames.len()
    };
    #[cfg(not(windows))]
    if print {
        submit_print(&destination).map_err(|error| {
            format!(
                "PDF was saved at {}. Print submission failed: {error}",
                destination.display()
            )
        })?;
    }
    Ok(
        json!({"canceled":false,"destinationPath":destination,"pageCount":page_count,"printSubmitted":print}),
    )
}

#[cfg(any(not(windows), test))]
fn validate_print_pdf(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || !path.is_file() {
        return Err("Print requires a saved absolute PDF path.".into());
    }
    let header = std::fs::read(path).map_err(|error| error.to_string())?;
    if !header.starts_with(b"%PDF-") {
        return Err("Print file is not a PDF.".into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn submit_print(path: &Path) -> Result<(), String> {
    validate_print_pdf(path)?;
    let output = std::process::Command::new("lp")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| format!("Cannot start the system lp print service: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "System print service rejected the PDF: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

pub(super) fn preview(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let request: handout::HandoutRequest = serde_json::from_value(argument(args, 1)?)
        .map_err(|error| format!("Invalid handout options: {error}"))?;
    let io = lock_workspace_io(state)?;
    io.require_active(&path)?;
    let prepared = handout::prepare(&workspace::read_workspace(&path)?, &request)?;
    let frames = handout::render(&prepared, &request)?;
    let directory = std::env::temp_dir().join("presenter-orbit-handouts");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let destination = directory.join(format!("preview-{}.pdf", uuid::Uuid::new_v4()));
    presentation_export::write_new(
        &destination,
        &presentation_export::encode_pdf_with_page_scale(&frames, 0.75)?,
    )?;
    open::that(&destination).map_err(|error| format!("Cannot open handout preview: {error}"))?;
    Ok(json!({"canceled":false,"destinationPath":destination,"pageCount":frames.len()}))
}
