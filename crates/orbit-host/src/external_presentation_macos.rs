// Compiled on every platform so the script builders stay unit-tested; only macOS selects it at runtime.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::{
    fit_image_size, Application, Backend, ExportedSlideImage, ExportedSlideImages, Position,
    MAX_EXPORT_SLIDES,
};
use std::path::Path;
use std::process::Command;

/// Escapes text for an AppleScript string literal.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn powerpoint_start(path: &Path) -> String {
    format!(
        r#"tell application "Microsoft PowerPoint"
  activate
  open POSIX file {path}
  set thePresentation to active presentation
  run slide show (slide show settings of thePresentation)
  set theView to slide show view of slide show window 1
  return ((current show position of theView) as string) & "," & ((count of slides of thePresentation) as string)
end tell"#,
        path = quote(&path.display().to_string())
    )
}

/// Saves a PowerPoint presentation as PDF and lists each slide as `number, hidden, notes`. Fields are separated by
/// U+001F and slides by U+001E, which slide notes do not contain. PowerPoint for Mac has no AppleScript command
/// that exports one slide image, so the PDF is rasterized natively instead.
fn powerpoint_export_pdf(source: &Path, pdf: &Path) -> String {
    format!(
        r#"set fieldSeparator to character id 31
set slideSeparator to character id 30
tell application "Microsoft PowerPoint"
  set sourceName to (POSIX file {source}) as string
  repeat with candidate in presentations
    if (full name of candidate) is sourceName then return "already-open"
  end repeat
  open POSIX file {source}
  set thePresentation to active presentation
  try
    set listing to ""
    repeat with slideNumber from 1 to count of slides of thePresentation
      set theSlide to slide slideNumber of thePresentation
      set noteText to ""
      -- A slide without a notes body placeholder has no notes.
      try
        set noteText to content of text range of text frame of place holder 2 of notes page of theSlide
      end try
      set listing to listing & slideNumber & fieldSeparator & (hidden of slide show transition of theSlide) & fieldSeparator & noteText & slideSeparator
    end repeat
    save thePresentation in (POSIX file {pdf}) as save as PDF
    close thePresentation saving no
    return listing
  on error errorMessage number errorNumber
    try
      close thePresentation saving no
    end try
    error errorMessage number errorNumber
  end try
end tell"#,
        source = quote(&source.display().to_string()),
        pdf = quote(&pdf.display().to_string()),
    )
}

const POWERPOINT_QUIT_IF_IDLE: &str =
    r#"tell application "Microsoft PowerPoint" to if (count of presentations) is 0 then quit"#;

#[derive(Debug, PartialEq)]
struct ExportSlide {
    number: u32,
    hidden: bool,
    notes: String,
}

fn parse_export_listing(output: &str) -> Result<Vec<ExportSlide>, String> {
    if output.trim() == "already-open" {
        return Err(
            "This presentation is already open in PowerPoint. Close it there before exporting slide images.".into(),
        );
    }
    let slides = output
        .split('\u{1e}')
        .filter(|record| !record.trim().is_empty())
        .map(|record| {
            let mut fields = record.splitn(3, '\u{1f}');
            let (Some(number), Some(hidden), Some(notes)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return Err(format!("Unexpected PowerPoint slide listing '{record}'."));
            };
            Ok(ExportSlide {
                number: number
                    .trim()
                    .parse()
                    .map_err(|_| format!("Unexpected PowerPoint slide number '{number}'."))?,
                hidden: match hidden {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(format!(
                            "Unexpected PowerPoint hidden-slide value '{hidden}'."
                        ))
                    }
                },
                notes: notes.replace('\r', "\n"),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if slides.is_empty() {
        return Err("PowerPoint presentation contains no slides.".into());
    }
    if slides.len() > MAX_EXPORT_SLIDES as usize {
        return Err(format!(
            "PowerPoint presentation has {} slides; slide image export is limited to {MAX_EXPORT_SLIDES}.",
            slides.len()
        ));
    }
    Ok(slides)
}

/// Writes one `Slide-NNNN.png` per visible slide from the PowerPoint PDF. Depending on PowerPoint's settings the PDF
/// contains every slide or only the visible ones; any other page count cannot be matched to slides and is an error.
fn write_slide_images(
    pdf: &[u8],
    slides: &[ExportSlide],
    destination: &Path,
) -> Result<ExportedSlideImages, String> {
    let visible = slides
        .iter()
        .filter(|slide| !slide.hidden)
        .collect::<Vec<_>>();
    let mut exported = Vec::new();
    crate::chord_chart_pdf::for_each_pdf_page(
        pdf,
        "PowerPoint PDF export",
        MAX_EXPORT_SLIDES as usize,
        |width, height| fit_image_size(width.into(), height.into()),
        |index, page_count, image| {
            let slide = if page_count == visible.len() {
                visible[index]
            } else if page_count == slides.len() {
                &slides[index]
            } else {
                return Err(format!(
                    "PowerPoint exported {page_count} PDF pages for {} slides ({} visible).",
                    slides.len(),
                    visible.len()
                ));
            };
            if slide.hidden {
                return Ok(());
            }
            let path = destination.join(format!("Slide-{:04}.png", slide.number));
            image
                .save_with_format(&path, image::ImageFormat::Png)
                .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
            exported.push(ExportedSlideImage {
                slide_number: slide.number,
                path,
                width: image.width(),
                height: image.height(),
                notes: slide.notes.clone(),
            });
            Ok(())
        },
    )?;
    Ok(ExportedSlideImages {
        slides: exported,
        hidden_slide_count: (slides.len() - visible.len()) as u32,
    })
}

fn keynote_start(path: &Path) -> String {
    format!(
        r#"tell application "Keynote"
  set sourceFile to (POSIX file {path}) as alias
  set existingDocumentIDs to {{}}
  repeat with candidate in documents
    set end of existingDocumentIDs to (id of candidate as string)
    try
      if (POSIX path of (file of candidate as alias)) is (POSIX path of sourceFile) then return "already-open"
    end try
  end repeat
  if playing then return "busy"
  set theDocument to open sourceFile
  try
    set documentID to id of theDocument as string
    if existingDocumentIDs contains documentID then return "already-open"
    start theDocument
    return documentID & tab & ((slide number of current slide of theDocument) as string) & "," & ((count of slides of theDocument) as string)
  on error errorMessage number errorNumber
    try
      stop theDocument
    end try
    try
      close theDocument saving no
    end try
    error errorMessage number errorNumber
  end try
end tell"#,
        path = quote(&path.display().to_string())
    )
}

fn powerpoint_position(step: &str) -> String {
    format!(
        r#"tell application "Microsoft PowerPoint"
  if (count of slide show windows) is 0 then return "ended"
  set theView to slide show view of slide show window 1
  {step}
  return ((current show position of theView) as string) & "," & ((count of slides of active presentation) as string)
end tell"#
    )
}

fn keynote_position(document_id: &str, step: &str) -> String {
    let step = if step.is_empty() {
        String::new()
    } else {
        format!("  tell theDocument to {step}\n")
    };
    format!(
        r#"tell application "Keynote"
  if not (exists document id {document_id}) then return "ended"
  set theDocument to document id {document_id}
  if not playing then return "ended"
{step}  return ((slide number of current slide of theDocument) as string) & "," & ((count of slides of theDocument) as string)
end tell"#,
        document_id = quote(document_id),
        step = step,
    )
}

fn step_script(session: &Session, step: &str) -> Result<String, String> {
    let application = session.application;
    match application {
        Application::PowerPoint => Ok(powerpoint_position(step)),
        Application::Keynote => Ok(keynote_position(
            session
                .document_id
                .as_deref()
                .ok_or_else(|| "The Keynote session has no owned document.".to_string())?,
            step,
        )),
    }
}

fn next_step(application: Application) -> &'static str {
    match application {
        Application::PowerPoint => "go to next slide theView",
        Application::Keynote => "show next",
    }
}

fn previous_step(application: Application) -> &'static str {
    match application {
        Application::PowerPoint => "go to previous slide theView",
        Application::Keynote => "show previous",
    }
}

fn go_to_step(application: Application, slide: u32) -> String {
    match application {
        Application::PowerPoint => format!("go to slide theView number {slide}"),
        Application::Keynote => format!("show slide {slide}"),
    }
}

fn stop_script(
    application: Application,
    document_id: Option<&str>,
    quit_application: bool,
) -> Result<String, String> {
    match application {
        Application::PowerPoint => Ok(format!(
            r#"tell application "Microsoft PowerPoint"
  if (count of slide show windows) > 0 then exit slide show (slide show view of slide show window 1)
  close active presentation saving no
  {quit}
end tell"#,
            quit = if quit_application {
                "if (count of presentations) is 0 then quit"
            } else {
                ""
            }
        )),
        Application::Keynote => {
            let document_id = document_id
                .ok_or_else(|| "The Keynote session has no owned document.".to_string())?;
            Ok(format!(
                r#"tell application "Keynote"
  if exists document id {document_id} then
    set theDocument to document id {document_id}
    try
      stop theDocument
    end try
    close theDocument saving no
  end if
  {quit}
end tell"#,
                document_id = quote(document_id),
                quit = if quit_application {
                    "if (count of documents) is 0 then quit"
                } else {
                    ""
                }
            ))
        }
    }
}

fn running_script(application: Application) -> String {
    format!(
        r#"tell application "System Events" to return (name of processes) contains {}"#,
        quote(match application {
            Application::PowerPoint => "Microsoft PowerPoint",
            Application::Keynote => "Keynote",
        })
    )
}

fn parse_position(output: &str) -> Result<Option<Position>, String> {
    let output = output.trim();
    if output == "ended" {
        return Ok(None);
    }
    let (current, count) = output
        .split_once(',')
        .ok_or_else(|| format!("Unexpected presentation application response '{output}'."))?;
    let parse = |value: &str| {
        value
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("Unexpected presentation application response '{output}'."))
    };
    // The scripting interfaces expose no build progress, so the last slide is treated as fully revealed.
    Ok(Some(Position {
        current: parse(current)?,
        count: parse(count)?,
        builds_remaining: false,
        builds_revealed: false,
    }))
}

fn run_script(script: &str) -> Result<String, String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|error| format!("Could not run osascript: {error}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if message.is_empty() {
        "The presentation application rejected the request.".to_string()
    } else {
        message
    })
}

struct Session {
    application: Application,
    launched: bool,
    document_id: Option<String>,
}

#[derive(Default)]
pub struct ScriptingBackend {
    session: Option<Session>,
}

impl ScriptingBackend {
    fn step(&mut self, step: impl FnOnce(Application) -> String) -> Result<Position, String> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| "No external presentation is running.".to_string())?;
        let script = step_script(session, &step(session.application))?;
        parse_position(&run_script(&script)?)?.ok_or_else(|| "The slideshow has ended.".to_string())
    }
}

fn parse_keynote_start_response(output: &str) -> Result<(String, String), String> {
    let output = output.trim();
    if output == "already-open" {
        return Err(
            "This Keynote presentation is already open. Close it in Keynote before starting it from Orbit."
                .into(),
        );
    }
    if output == "busy" {
        return Err("Keynote is already playing a slideshow.".into());
    }
    let (document_id, position) = output
        .split_once('\t')
        .ok_or_else(|| format!("Unexpected Keynote response '{output}'."))?;
    if document_id.is_empty() {
        return Err("Keynote did not return the opened document ID.".into());
    }
    Ok((document_id.to_string(), position.to_string()))
}

impl Backend for ScriptingBackend {
    fn supports(&self, _: Application) -> Result<(), String> {
        Ok(())
    }

    fn start(&mut self, application: Application, path: &Path) -> Result<Position, String> {
        let launched = run_script(&running_script(application))? != "true";
        match application {
            Application::PowerPoint => {
                self.session = Some(Session {
                    application,
                    launched,
                    document_id: None,
                });
                match run_script(&powerpoint_start(path)).and_then(|output| {
                    parse_position(&output)?
                        .ok_or_else(|| "The slideshow did not start.".to_string())
                }) {
                    Ok(position) => Ok(position),
                    Err(error) => {
                        let _ = self.stop();
                        Err(error)
                    }
                }
            }
            Application::Keynote => {
                let output = run_script(&keynote_start(path))?;
                let (document_id, position_output) = parse_keynote_start_response(&output)?;
                self.session = Some(Session {
                    application,
                    launched,
                    document_id: Some(document_id),
                });
                match parse_position(&position_output) {
                    Ok(Some(position)) => Ok(position),
                    Ok(None) => {
                        let _ = self.stop();
                        Err("The slideshow did not start.".into())
                    }
                    Err(error) => {
                        let _ = self.stop();
                        Err(error)
                    }
                }
            }
        }
    }

    fn next(&mut self) -> Result<Position, String> {
        self.step(|application| next_step(application).to_string())
    }

    fn previous(&mut self) -> Result<Position, String> {
        self.step(|application| previous_step(application).to_string())
    }

    fn go_to(&mut self, slide: u32) -> Result<Position, String> {
        self.step(|application| go_to_step(application, slide))
    }

    fn position(&mut self) -> Result<Option<Position>, String> {
        let Some(session) = self.session.as_ref() else {
            return Ok(None);
        };
        let script = step_script(session, "")?;
        let position = parse_position(&run_script(&script)?)?;
        if position.is_none() {
            self.stop()?;
        }
        Ok(position)
    }

    fn stop(&mut self) -> Result<(), String> {
        let Some(session) = self.session.take() else {
            return Ok(());
        };
        run_script(&stop_script(
            session.application,
            session.document_id.as_deref(),
            session.launched,
        )?)
        .map(|_| ())
    }

    fn export_images(
        &mut self,
        source: &Path,
        destination: &Path,
    ) -> Result<ExportedSlideImages, String> {
        let launched = run_script(&running_script(Application::PowerPoint))? != "true";
        let pdf = destination.join("PowerPoint-export.pdf");
        let export = run_script(&powerpoint_export_pdf(source, &pdf)).and_then(|output| {
            let slides = parse_export_listing(&output)?;
            let bytes = std::fs::read(&pdf)
                .map_err(|error| format!("PowerPoint did not create the PDF export: {error}"))?;
            write_slide_images(&bytes, &slides, destination)
        });
        // The intermediate PDF must not stay behind in the managed import folder.
        let cleanup = match std::fs::remove_file(&pdf) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(format!(
                "Could not remove the PowerPoint PDF export: {error}"
            )),
            _ => Ok(()),
        };
        let quit = if launched {
            run_script(POWERPOINT_QUIT_IF_IDLE).map(|_| ())
        } else {
            Ok(())
        };
        match (export, cleanup.and(quit)) {
            (Ok(images), Ok(())) => Ok(images),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => Err(format!("{error} Cleanup also failed: {cleanup}")),
        }
    }
}
