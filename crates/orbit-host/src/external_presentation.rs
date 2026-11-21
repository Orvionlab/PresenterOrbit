//! Native control of presentations that are played by an external application
//! (Microsoft PowerPoint on Windows and macOS, Keynote on macOS), the way
//! ProPresenter triggers PowerPoint and Keynote files and relies on the
//! application to play them.

use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

#[path = "external_presentation_macos.rs"]
mod macos;
#[cfg(windows)]
#[path = "external_presentation_windows.rs"]
mod windows_powerpoint;

const POWERPOINT_EXTENSIONS: [&str; 6] = ["ppt", "pptx", "pptm", "pps", "ppsx", "ppsm"];
const EXPORTABLE_POWERPOINT_EXTENSIONS: [&str; 4] = ["ppt", "pptx", "pps", "ppsx"];
const KEYNOTE_EXTENSIONS: [&str; 1] = ["key"];
const METHODS: [&str; 3] = [
    "startExternalPresentation",
    "controlExternalPresentation",
    "getExternalPresentationStatus",
];

pub fn keynote_source_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Keynote source path must be absolute.".into());
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if extension.as_deref() != Some("key") {
        return Err("Keynote source must have a .key extension.".into());
    }
    let path = path
        .canonicalize()
        .map_err(|error| format!("Could not read Keynote source: {error}"))?;
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("Could not read Keynote source: {error}"))?;
    if metadata.is_file() {
        if metadata.len() == 0 {
            return Err("Keynote source file is empty.".into());
        }
    } else if metadata.is_dir() {
        if !is_keynote_package(&path) {
            return Err(
                "Keynote package directory does not contain a recognized document index.".into(),
            );
        }
    } else {
        return Err("Keynote source must be a file or package directory.".into());
    }
    Ok(path)
}

fn is_keynote_package(path: &Path) -> bool {
    for name in [
        "index.apxl",
        "index.apxl.gz",
        "presentation.apxl",
        "presentation.apxl.gz",
        "Index.zip",
    ] {
        if has_nonempty_regular_file(&path.join(name)) {
            return true;
        }
    }
    let index = path.join("Index");
    std::fs::symlink_metadata(&index)
        .is_ok_and(|metadata| !metadata.file_type().is_symlink() && metadata.is_dir())
        && has_nonempty_regular_file(&index.join("Document.iwa"))
}

fn has_nonempty_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        !metadata.file_type().is_symlink() && metadata.is_file() && metadata.len() > 0
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Application {
    PowerPoint,
    Keynote,
}

impl Application {
    pub fn label(self) -> &'static str {
        match self {
            Application::PowerPoint => "Microsoft PowerPoint",
            Application::Keynote => "Keynote",
        }
    }
}

/// One-based slide position inside a running slideshow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub current: u32,
    pub count: u32,
    /// The current slide still has animation builds to reveal, so Next on the last slide is not the end.
    pub builds_remaining: bool,
    /// Builds of the current slide are revealed, so Previous on the first slide still has an effect.
    pub builds_revealed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Next,
    Previous,
    GoTo(u32),
    Stop,
}

impl Command {
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or("External presentation command requires a type.")?;
        match kind {
            "next" => Ok(Command::Next),
            "previous" => Ok(Command::Previous),
            "stop" => Ok(Command::Stop),
            "goto" => {
                let slide = value
                    .get("slide")
                    .and_then(Value::as_u64)
                    .filter(|slide| *slide >= 1 && *slide <= u64::from(u32::MAX))
                    .ok_or("External presentation go-to requires a slide number of at least 1.")?;
                Ok(Command::GoTo(slide as u32))
            }
            other => Err(format!("Unknown external presentation command '{other}'.")),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalPresentationStatus {
    pub running: bool,
    pub application: Option<String>,
    pub file: Option<String>,
    pub current_slide: Option<u32>,
    pub slide_count: Option<u32>,
    pub error: Option<String>,
}

pub(crate) const MAX_EXPORT_SLIDES: i32 = 500;
const MAX_EXPORT_WIDTH: u32 = 1920;
const MAX_EXPORT_HEIGHT: u32 = 1080;
const MIN_EXPORT_DIMENSION: u32 = 16;

/// The PNG size for a slide, fitted inside 1920×1080 with its aspect ratio kept.
pub(crate) fn fit_image_size(slide_width: f64, slide_height: f64) -> Result<(u32, u32), String> {
    if !slide_width.is_finite()
        || !slide_height.is_finite()
        || slide_width <= 0.0
        || slide_height <= 0.0
    {
        return Err("PowerPoint slide dimensions must be positive finite values.".into());
    }
    let scale = (f64::from(MAX_EXPORT_WIDTH) / slide_width)
        .min(f64::from(MAX_EXPORT_HEIGHT) / slide_height);
    let width = (slide_width * scale).round() as u32;
    let height = (slide_height * scale).round() as u32;
    if width < MIN_EXPORT_DIMENSION || height < MIN_EXPORT_DIMENSION {
        return Err(
            "PowerPoint slide aspect ratio is outside the supported PNG dimensions.".into(),
        );
    }
    Ok((width, height))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedSlideImage {
    pub slide_number: u32,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub notes: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedSlideImages {
    pub slides: Vec<ExportedSlideImage>,
    pub hidden_slide_count: u32,
}

fn validate_image_export_paths(
    source: &Path,
    destination: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    if !source.is_absolute() {
        return Err("PowerPoint source path must be absolute.".into());
    }
    let source = source
        .canonicalize()
        .map_err(|error| format!("Could not read PowerPoint source: {error}"))?;
    if !source.is_file() {
        return Err("PowerPoint source must be a file.".into());
    }
    if std::fs::metadata(&source)
        .map_err(|error| format!("Could not read PowerPoint source: {error}"))?
        .len()
        == 0
    {
        return Err("PowerPoint source file is empty.".into());
    }
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !EXPORTABLE_POWERPOINT_EXTENSIONS.contains(&extension.as_str()) {
        return Err(format!(
            "Unsupported PowerPoint export file type '.{extension}'. Supported types are .ppt, .pptx, .pps, and .ppsx."
        ));
    }
    if !destination.is_absolute() {
        return Err("PowerPoint export destination must be absolute.".into());
    }
    if destination
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("PowerPoint export destination cannot contain '..'.".into());
    }
    for ancestor in destination.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor)
            .map_err(|error| format!("PowerPoint export destination is unavailable: {error}"))?;
        if is_link_or_reparse_point(&metadata) {
            return Err("PowerPoint export destination cannot contain symbolic links.".into());
        }
    }
    if !destination.is_dir() {
        return Err("PowerPoint export destination must be an existing directory.".into());
    }
    let destination = destination
        .canonicalize()
        .map_err(|error| format!("Could not resolve PowerPoint export destination: {error}"))?;
    if std::fs::read_dir(&destination)
        .map_err(|error| format!("Could not read PowerPoint export destination: {error}"))?
        .next()
        .is_some()
    {
        return Err("PowerPoint export destination must be empty.".into());
    }
    Ok((source, destination))
}

pub(super) fn is_link_or_reparse_point(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    false
}

/// Platform controller for one external slideshow at a time.
pub trait Backend: Send {
    fn supports(&self, application: Application) -> Result<(), String>;
    fn start(&mut self, application: Application, path: &Path) -> Result<Position, String>;
    fn next(&mut self) -> Result<Position, String>;
    fn previous(&mut self) -> Result<Position, String>;
    fn go_to(&mut self, slide: u32) -> Result<Position, String>;
    /// `None` means the slideshow ended outside Orbit and the backend released it.
    fn position(&mut self) -> Result<Option<Position>, String>;
    fn stop(&mut self) -> Result<(), String>;
    fn export_images(&mut self, _: &Path, _: &Path) -> Result<ExportedSlideImages, String> {
        Err("PowerPoint slide image export is not supported by this backend.".into())
    }
}

pub fn resolve_source(source: &str) -> Result<(PathBuf, Application), String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("External presentation requires a presentation file.".into());
    }
    let path = if source
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
    {
        url::Url::parse(source)
            .map_err(|error| format!("External presentation file URL is invalid: {error}"))?
            .to_file_path()
            .map_err(|()| "External presentation file URL is not a local file.".to_string())?
    } else if source.contains("://") {
        return Err(
            "External presentations must be local presentation files; remote URLs are not supported."
                .into(),
        );
    } else {
        PathBuf::from(source)
    };
    if !path.is_absolute() {
        return Err("External presentation file path must be absolute.".into());
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let application = if POWERPOINT_EXTENSIONS.contains(&extension.as_str()) {
        Application::PowerPoint
    } else if KEYNOTE_EXTENSIONS.contains(&extension.as_str()) {
        Application::Keynote
    } else {
        return Err(format!(
            "Unsupported external presentation file type '.{extension}'. Supported types are PowerPoint (.ppt, .pptx, .pptm, .pps, .ppsx, .ppsm) and Keynote (.key)."
        ));
    };
    let path = match application {
        Application::Keynote => keynote_source_path(&path)?,
        Application::PowerPoint if path.is_file() => path,
        Application::PowerPoint => {
            return Err(format!(
                "External presentation file was not found: {}",
                path.display()
            ))
        }
    };
    Ok((path, application))
}

struct Session {
    application: Application,
    file: String,
    position: Position,
}

struct Inner {
    backend: Box<dyn Backend>,
    session: Option<Session>,
    error: Option<String>,
}

pub struct ExternalPresentationService {
    inner: Mutex<Inner>,
}

impl Default for ExternalPresentationService {
    fn default() -> Self {
        Self::new(platform_backend())
    }
}

#[cfg(windows)]
fn platform_backend() -> Box<dyn Backend> {
    Box::new(windows_powerpoint::PowerPointBackend::default())
}

#[cfg(target_os = "macos")]
fn platform_backend() -> Box<dyn Backend> {
    Box::new(macos::ScriptingBackend::default())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_backend() -> Box<dyn Backend> {
    Box::new(UnsupportedBackend)
}

#[cfg(not(any(windows, target_os = "macos")))]
struct UnsupportedBackend;

#[cfg(not(any(windows, target_os = "macos")))]
impl Backend for UnsupportedBackend {
    fn supports(&self, _: Application) -> Result<(), String> {
        Err("External presentations are only supported on Windows (PowerPoint) and macOS (Keynote and PowerPoint).".into())
    }
    fn start(&mut self, application: Application, _: &Path) -> Result<Position, String> {
        self.supports(application).map(|()| unreachable!())
    }
    fn next(&mut self) -> Result<Position, String> {
        self.supports(Application::PowerPoint)
            .map(|()| unreachable!())
    }
    fn previous(&mut self) -> Result<Position, String> {
        self.next()
    }
    fn go_to(&mut self, _: u32) -> Result<Position, String> {
        self.next()
    }
    fn position(&mut self) -> Result<Option<Position>, String> {
        Ok(None)
    }
    fn stop(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn export_images(&mut self, _: &Path, _: &Path) -> Result<ExportedSlideImages, String> {
        Err("PowerPoint slide image export is only supported on Windows.".into())
    }
}

impl ExternalPresentationService {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                backend,
                session: None,
                error: None,
            }),
        }
    }

    pub fn export_images(
        &self,
        source: &Path,
        destination: &Path,
    ) -> Result<ExportedSlideImages, String> {
        let (source, destination) = validate_image_export_paths(source, destination)?;
        self.lock()?.backend.export_images(&source, &destination)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, String> {
        self.inner
            .lock()
            .map_err(|_| "External presentation service is unavailable.".to_string())
    }

    pub fn start(&self, source: &str) -> Result<ExternalPresentationStatus, String> {
        let mut inner = self.lock()?;
        let result = start_session(&mut inner, source);
        finish(&mut inner, result)
    }

    pub fn command(&self, command: &Command) -> Result<ExternalPresentationStatus, String> {
        let mut inner = self.lock()?;
        let result = run_command(&mut inner, command);
        finish(&mut inner, result)
    }

    pub fn status(&self) -> Result<ExternalPresentationStatus, String> {
        let mut inner = self.lock()?;
        if inner.session.is_some() {
            match inner.backend.position() {
                Ok(Some(position)) => {
                    if let Some(session) = inner.session.as_mut() {
                        session.position = position;
                    }
                }
                Ok(None) => inner.session = None,
                Err(error) => inner.error = Some(error),
            }
        }
        Ok(snapshot(&inner))
    }
}

fn start_session(inner: &mut Inner, source: &str) -> Result<(), String> {
    let (path, application) = resolve_source(source)?;
    inner.backend.supports(application)?;
    if inner.session.is_some() {
        inner.backend.stop()?;
        inner.session = None;
    }
    let position = inner.backend.start(application, &path)?;
    inner.session = Some(Session {
        application,
        file: path.display().to_string(),
        position,
    });
    Ok(())
}

fn run_command(inner: &mut Inner, command: &Command) -> Result<(), String> {
    let Some(session) = inner.session.as_ref() else {
        return match command {
            Command::Stop => Ok(()),
            _ => Err("No external presentation is running.".into()),
        };
    };
    let position = session.position;
    let next = match command {
        Command::Stop => {
            inner.backend.stop()?;
            inner.session = None;
            return Ok(());
        }
        Command::Next if position.current >= position.count && !position.builds_remaining => {
            return Ok(())
        }
        Command::Previous if position.current <= 1 && !position.builds_revealed => return Ok(()),
        Command::Next => inner.backend.next()?,
        Command::Previous => inner.backend.previous()?,
        Command::GoTo(slide) if *slide > position.count => {
            return Err(format!(
                "Slide {slide} is outside the presentation ({} slides).",
                position.count
            ))
        }
        Command::GoTo(slide) => inner.backend.go_to(*slide)?,
    };
    if let Some(session) = inner.session.as_mut() {
        session.position = next;
    }
    Ok(())
}

fn finish(
    inner: &mut Inner,
    result: Result<(), String>,
) -> Result<ExternalPresentationStatus, String> {
    match result {
        Ok(()) => {
            inner.error = None;
            Ok(snapshot(inner))
        }
        Err(error) => {
            inner.error = Some(error.clone());
            Err(error)
        }
    }
}

fn snapshot(inner: &Inner) -> ExternalPresentationStatus {
    ExternalPresentationStatus {
        running: inner.session.is_some(),
        application: inner
            .session
            .as_ref()
            .map(|session| session.application.label().to_string()),
        file: inner.session.as_ref().map(|session| session.file.clone()),
        current_slide: inner
            .session
            .as_ref()
            .map(|session| session.position.current),
        slide_count: inner.session.as_ref().map(|session| session.position.count),
        error: inner.error.clone(),
    }
}

pub fn handles(method: &str) -> bool {
    METHODS.contains(&method)
}

pub fn invoke(
    service: &ExternalPresentationService,
    method: &str,
    args: &[Value],
) -> Result<Value, String> {
    let status = match method {
        "startExternalPresentation" => service.start(
            args.first()
                .and_then(Value::as_str)
                .ok_or("External presentation source must be a string.")?,
        )?,
        "controlExternalPresentation" => service.command(&Command::from_value(
            args.first()
                .ok_or("External presentation command is required.")?,
        )?)?,
        "getExternalPresentationStatus" => service.status()?,
        other => return Err(format!("Unknown external presentation method '{other}'.")),
    };
    Ok(json!(status))
}
