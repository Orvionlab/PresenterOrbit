//! Microsoft PowerPoint control through COM automation (`PowerPoint.Application`).
//! All COM objects live on one dedicated STA thread owned by `PowerPointBackend`.

use super::{Application, Backend, ExportedSlideImage, ExportedSlideImages, Position};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use windows::core::{Interface, BSTR, GUID, PCWSTR};
use windows::Win32::System::Com::{
    CLSIDFromProgID, CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch,
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, DISPATCH_FLAGS, DISPATCH_METHOD,
    DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS, EXCEPINFO,
};
use windows::Win32::System::Ole::GetActiveObject;
use windows::Win32::System::Variant::VARIANT;

const PROG_ID: &str = "PowerPoint.Application";
const LOCALE_USER_DEFAULT: u32 = 0x0400;
const DISPID_PROPERTYPUT: i32 = -3;
const MSO_TRUE: i32 = -1;
const MSO_FALSE: i32 = 0;
const SLIDE_SHOW_DONE: i32 = 5;

#[derive(Clone)]
pub(super) struct Dispatch(IDispatch);

impl Dispatch {
    fn invoke(
        &self,
        name: &str,
        flags: DISPATCH_FLAGS,
        args: Vec<VARIANT>,
    ) -> Result<VARIANT, String> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut dispatch_id = 0i32;
        let describe = |error: windows::core::Error| format!("PowerPoint '{name}' failed: {error}");
        unsafe {
            self.0
                .GetIDsOfNames(
                    &GUID::zeroed(),
                    &PCWSTR(wide.as_ptr()),
                    1,
                    LOCALE_USER_DEFAULT,
                    &mut dispatch_id,
                )
                .map_err(describe)?;
        }
        // IDispatch takes positional arguments in reverse order.
        let mut reversed: Vec<VARIANT> = args.into_iter().rev().collect();
        let mut named_put = DISPID_PROPERTYPUT;
        let is_put = flags == DISPATCH_PROPERTYPUT;
        let params = DISPPARAMS {
            rgvarg: if reversed.is_empty() {
                std::ptr::null_mut()
            } else {
                reversed.as_mut_ptr()
            },
            rgdispidNamedArgs: if is_put {
                &mut named_put
            } else {
                std::ptr::null_mut()
            },
            cArgs: reversed.len() as u32,
            cNamedArgs: u32::from(is_put),
        };
        let mut result = VARIANT::default();
        let mut exception = EXCEPINFO::default();
        let outcome = unsafe {
            self.0.Invoke(
                dispatch_id,
                &GUID::zeroed(),
                LOCALE_USER_DEFAULT,
                flags,
                &params,
                Some(&mut result),
                Some(&mut exception),
                None,
            )
        };
        outcome.map_err(|error| {
            let detail = exception.bstrDescription.to_string();
            if detail.is_empty() {
                describe(error)
            } else {
                format!("PowerPoint '{name}' failed: {detail}")
            }
        })?;
        Ok(result)
    }

    pub(super) fn get(&self, name: &str) -> Result<Dispatch, String> {
        self.get_with(name, vec![])
    }

    fn get_with(&self, name: &str, args: Vec<VARIANT>) -> Result<Dispatch, String> {
        let flags = if args.is_empty() {
            DISPATCH_PROPERTYGET
        } else {
            DISPATCH_PROPERTYGET | DISPATCH_METHOD
        };
        let value = self.invoke(name, flags, args)?;
        IDispatch::try_from(&value)
            .map(Dispatch)
            .map_err(|error| format!("PowerPoint '{name}' did not return an object: {error}"))
    }

    pub(super) fn int(&self, name: &str) -> Result<i32, String> {
        let value = self.invoke(name, DISPATCH_PROPERTYGET, vec![])?;
        i32::try_from(&value)
            .map_err(|error| format!("PowerPoint '{name}' is not a number: {error}"))
    }

    fn float(&self, name: &str) -> Result<f64, String> {
        let value = self.invoke(name, DISPATCH_PROPERTYGET, vec![])?;
        f64::try_from(&value)
            .map_err(|error| format!("PowerPoint '{name}' is not a number: {error}"))
    }

    fn text(&self, name: &str) -> Result<String, String> {
        let value = self.invoke(name, DISPATCH_PROPERTYGET, vec![])?;
        BSTR::try_from(&value)
            .map(|text| text.to_string())
            .map_err(|error| format!("PowerPoint '{name}' is not text: {error}"))
    }

    pub(super) fn call(&self, name: &str, args: Vec<VARIANT>) -> Result<VARIANT, String> {
        self.invoke(name, DISPATCH_METHOD, args)
    }

    pub(super) fn call_object(&self, name: &str, args: Vec<VARIANT>) -> Result<Dispatch, String> {
        let value = self.call(name, args)?;
        IDispatch::try_from(&value)
            .map(Dispatch)
            .map_err(|error| format!("PowerPoint '{name}' did not return an object: {error}"))
    }

    pub(super) fn set(&self, name: &str, value: VARIANT) -> Result<(), String> {
        self.invoke(name, DISPATCH_PROPERTYPUT, vec![value])
            .map(|_| ())
    }
}

/// Attaches to a running PowerPoint when there is one; the flag tells whether Orbit had to start it.
pub(super) fn attach() -> Result<(Dispatch, bool), String> {
    let clsid = unsafe { CLSIDFromProgID(&windows::core::HSTRING::from(PROG_ID)) }
        .map_err(|error| format!("Microsoft PowerPoint is not installed: {error}"))?;
    let mut running: Option<windows::core::IUnknown> = None;
    if unsafe { GetActiveObject(&clsid, None, &mut running) }.is_ok() {
        if let Some(unknown) = running {
            let dispatch = unknown.cast::<IDispatch>().map_err(|error| {
                format!("Running PowerPoint does not support automation: {error}")
            })?;
            return Ok((Dispatch(dispatch), false));
        }
    }
    let dispatch: IDispatch = unsafe { CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER) }
        .map_err(|error| format!("Microsoft PowerPoint could not be started: {error}"))?;
    Ok((Dispatch(dispatch), true))
}

struct ComSession {
    application: Dispatch,
    presentation: Dispatch,
    window: Dispatch,
    launched: bool,
}

impl ComSession {
    fn view(&self) -> Result<Dispatch, String> {
        self.window.get("View")
    }

    fn position(&self) -> Result<Option<Position>, String> {
        let view = match self
            .presentation
            .get("SlideShowWindow")
            .and_then(|window| window.get("View"))
        {
            Ok(view) => view,
            Err(_) => return Ok(None),
        };
        if view
            .int("State")
            .map_err(|_| ())
            .is_ok_and(|state| state == SLIDE_SHOW_DONE)
        {
            return Ok(None);
        }
        let clicks = |name: &str| {
            let value = view.call(name, vec![])?;
            i32::try_from(&value)
                .map_err(|error| format!("PowerPoint '{name}' is not a number: {error}"))
        };
        let click_index = clicks("GetClickIndex")?;
        Ok(Some(Position {
            current: view.int("CurrentShowPosition")? as u32,
            count: self.presentation.get("Slides")?.int("Count")? as u32,
            builds_remaining: click_index < clicks("GetClickCount")?,
            builds_revealed: click_index > 0,
        }))
    }

    fn require_position(&self) -> Result<Position, String> {
        self.position()?
            .ok_or_else(|| "The PowerPoint slideshow has ended.".to_string())
    }

    /// Closes only the presentation Orbit opened; PowerPoint is quit only when Orbit started it and nothing else is open.
    fn close(self) -> Result<(), String> {
        let mut first_error = None;
        let mut note = |result: Result<(), String>| {
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        };
        if let Ok(view) = self
            .presentation
            .get("SlideShowWindow")
            .and_then(|window| window.get("View"))
        {
            note(view.call("Exit", vec![]).map(|_| ()));
        }
        // Exiting the show can already close a presentation that was opened without a window.
        if self.presentation.text("FullName").is_ok() {
            note(self.presentation.set("Saved", VARIANT::from(MSO_TRUE)));
            note(self.presentation.call("Close", vec![]).map(|_| ()));
        }
        if self.launched {
            match self
                .application
                .get("Presentations")
                .and_then(|list| list.int("Count"))
            {
                Ok(0) => note(self.application.call("Quit", vec![]).map(|_| ())),
                Ok(_) => {}
                Err(error) => note(Err(error)),
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

fn same_file(left: &str, right: &Path) -> bool {
    let left = Path::new(left)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(left));
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

use super::{fit_image_size, MAX_EXPORT_SLIDES};

fn export_with_powerpoint(
    source: &Path,
    destination: &Path,
) -> Result<ExportedSlideImages, String> {
    reject_encrypted_ooxml(source)?;
    let (application, launched) = attach()?;
    let result = export_with_application(&application, source, destination);
    let quit_result = if launched {
        match application
            .get("Presentations")
            .and_then(|presentations| presentations.int("Count"))
        {
            Ok(0) => application.call("Quit", vec![]).map(|_| ()),
            Ok(_) => Ok(()),
            Err(error) => Err(error),
        }
    } else {
        Ok(())
    };
    match (result, quit_result) {
        (Ok(images), Ok(())) => Ok(images),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!(
            "PowerPoint exported the slides but could not be closed: {error}"
        )),
        (Err(error), Err(cleanup)) => {
            Err(format!("{error} PowerPoint cleanup also failed: {cleanup}"))
        }
    }
}

fn reject_encrypted_ooxml(source: &Path) -> Result<(), String> {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(extension.as_deref(), Some("pptx" | "ppsx")) {
        return Ok(());
    }
    let mut signature = [0; 8];
    std::fs::File::open(source)
        .and_then(|mut file| file.read(&mut signature))
        .map_err(|error| format!("Could not read PowerPoint source: {error}"))?;
    if signature == [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1] {
        return Err(
            "PowerPoint OOXML slide image export does not support password-protected files.".into(),
        );
    }
    Ok(())
}

fn export_with_application(
    application: &Dispatch,
    source: &Path,
    destination: &Path,
) -> Result<ExportedSlideImages, String> {
    let presentations = application.get("Presentations")?;
    for index in 1..=presentations.int("Count")? {
        let existing = presentations.get_with("Item", vec![VARIANT::from(index)])?;
        if same_file(&existing.text("FullName")?, source) {
            return Err(format!(
                "{} is already open in PowerPoint. Close it there before exporting slide images.",
                source.display()
            ));
        }
    }

    let automation_security = application.int("AutomationSecurity")?;
    let display_alerts = application.int("DisplayAlerts")?;
    let open = (|| {
        application.set("DisplayAlerts", VARIANT::from(1))?;
        application.set("AutomationSecurity", VARIANT::from(3))?;
        presentations.call_object(
            "Open",
            vec![
                VARIANT::from(BSTR::from(crate::strip_windows_verbatim_prefix(
                    &source.display().to_string(),
                ))),
                VARIANT::from(MSO_TRUE),
                VARIANT::from(MSO_FALSE),
                VARIANT::from(MSO_FALSE),
            ],
        )
    })();
    let restore_security =
        application.set("AutomationSecurity", VARIANT::from(automation_security));
    let restore_alerts = application.set("DisplayAlerts", VARIANT::from(display_alerts));
    let presentation = match (open, restore_security, restore_alerts) {
        (Ok(presentation), Ok(()), Ok(())) => presentation,
        (Err(error), security, alerts) => {
            let restore_error = security.err().or_else(|| alerts.err());
            return Err(match restore_error {
                Some(restore_error) => format!(
                    "Could not open the PowerPoint presentation read-only; it may be password-protected or invalid: {error}. Could not restore PowerPoint application settings: {restore_error}"
                ),
                None => format!(
                    "Could not open the PowerPoint presentation read-only; it may be password-protected or invalid: {error}"
                ),
            });
        }
        (Ok(presentation), security, alerts) => {
            let close = presentation.call("Close", vec![]).map(|_| ());
            let restore_error = security.err().or_else(|| alerts.err());
            return match (restore_error, close) {
                (Some(error), Ok(())) => Err(format!(
                    "Could not restore PowerPoint application settings: {error}"
                )),
                (Some(error), Err(close_error)) => Err(format!(
                    "Could not restore PowerPoint application settings: {error}; the export presentation also could not be closed: {close_error}"
                )),
                (None, Err(error)) => Err(format!(
                    "The export presentation could not be closed: {error}"
                )),
                (None, Ok(())) => unreachable!(),
            };
        }
    };

    let export = export_presentation(&presentation, destination);
    let close = presentation.call("Close", vec![]).map(|_| ());
    match (export, close) {
        (Ok(images), Ok(())) => Ok(images),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!(
            "PowerPoint exported the slides but could not close the export presentation: {error}"
        )),
        (Err(error), Err(close_error)) => Err(format!(
            "{error} The export presentation also could not be closed: {close_error}"
        )),
    }
}

fn export_presentation(
    presentation: &Dispatch,
    destination: &Path,
) -> Result<ExportedSlideImages, String> {
    let slides = presentation.get("Slides")?;
    let count = slides.int("Count")?;
    if count <= 0 {
        return Err("PowerPoint presentation contains no slides.".into());
    }
    if count > MAX_EXPORT_SLIDES {
        return Err(format!(
            "PowerPoint presentation has {count} slides; slide image export is limited to {MAX_EXPORT_SLIDES}."
        ));
    }
    let page_setup = presentation.get("PageSetup")?;
    let (width, height) = fit_image_size(
        page_setup.float("SlideWidth")?,
        page_setup.float("SlideHeight")?,
    )?;
    let mut exported = Vec::new();
    let mut touched_paths = Vec::new();
    let mut hidden_slide_count = 0;
    let result = (|| {
        for slide_number in 1..=count {
            let slide = slides.get_with("Item", vec![VARIANT::from(slide_number)])?;
            if slide.get("SlideShowTransition")?.int("Hidden")? != 0 {
                hidden_slide_count += 1;
                continue;
            }
            let notes = slide_notes(&slide)?;
            let path = destination.join(format!("Slide-{slide_number:04}.png"));
            touched_paths.push(path.clone());
            slide.call(
                "Export",
                vec![
                    VARIANT::from(BSTR::from(crate::strip_windows_verbatim_prefix(
                        &path.display().to_string(),
                    ))),
                    VARIANT::from(BSTR::from("PNG")),
                    VARIANT::from(width as i32),
                    VARIANT::from(height as i32),
                ],
            )?;
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                format!("PowerPoint did not create {}: {error}", path.display())
            })?;
            if super::is_link_or_reparse_point(&metadata)
                || !metadata.is_file()
                || metadata.len() == 0
            {
                return Err(format!(
                    "PowerPoint produced an invalid slide image at {}.",
                    path.display()
                ));
            }
            exported.push(ExportedSlideImage {
                slide_number: slide_number as u32,
                path,
                width,
                height,
                notes,
            });
        }
        if exported.is_empty() {
            return Err("PowerPoint presentation contains no visible slides to export.".into());
        }
        Ok(ExportedSlideImages {
            slides: exported,
            hidden_slide_count,
        })
    })();
    if result.is_err() {
        for path in touched_paths {
            let _ = std::fs::remove_file(path);
        }
    }
    result
}

fn slide_notes(slide: &Dispatch) -> Result<String, String> {
    let shapes = slide.get("NotesPage")?.get("Shapes")?;
    for index in 1..=shapes.int("Count")? {
        let shape = shapes.get_with("Item", vec![VARIANT::from(index)])?;
        let Ok(placeholder) = shape.get("PlaceholderFormat") else {
            continue;
        };
        if placeholder.int("Type")? != 2 {
            continue;
        }
        let mut text = shape.get("TextFrame")?.get("TextRange")?.text("Text")?;
        while text.ends_with('\r') {
            text.pop();
        }
        return Ok(text);
    }
    Ok(String::new())
}

fn start_session(path: &Path) -> Result<ComSession, String> {
    let (application, launched) = attach()?;
    let open = || -> Result<ComSession, String> {
        let presentations = application.get("Presentations")?;
        for index in 1..=presentations.int("Count")? {
            let existing = presentations.get_with("Item", vec![VARIANT::from(index)])?;
            if same_file(&existing.text("FullName")?, path) {
                return Err(format!(
                    "{} is already open in PowerPoint. Close it there before starting it from Presenter Orbit.",
                    path.display()
                ));
            }
        }
        let presentation = presentations.call_object(
            "Open",
            vec![
                VARIANT::from(BSTR::from(path.display().to_string())),
                VARIANT::from(MSO_TRUE),
                VARIANT::from(MSO_FALSE),
                VARIANT::from(MSO_FALSE),
            ],
        )?;
        let window = match presentation
            .get("SlideShowSettings")
            .and_then(|settings| settings.call_object("Run", vec![]))
        {
            Ok(window) => window,
            Err(error) => {
                let _ = ComSession {
                    application: application.clone(),
                    presentation,
                    window: application.clone(),
                    launched,
                }
                .close();
                return Err(error);
            }
        };
        Ok(ComSession {
            application: application.clone(),
            presentation,
            window,
            launched,
        })
    };
    match open() {
        Ok(session) => Ok(session),
        Err(error) => {
            if launched
                && application
                    .get("Presentations")
                    .and_then(|list| list.int("Count"))
                    == Ok(0)
            {
                let _ = application.call("Quit", vec![]);
            }
            Err(error)
        }
    }
}

enum Request {
    Start(PathBuf),
    Export {
        source: PathBuf,
        destination: PathBuf,
        reply: Sender<Result<ExportedSlideImages, String>>,
    },
    Next,
    Previous,
    GoTo(u32),
    Position,
    Stop,
}

type Reply = Result<Option<Position>, String>;

fn serve(requests: mpsc::Receiver<(Request, Sender<Reply>)>) {
    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
        for (_, reply) in requests {
            let _ = reply.send(Err(
                "COM could not be initialized for PowerPoint automation.".into(),
            ));
        }
        return;
    }
    let mut session: Option<ComSession> = None;
    for (request, reply) in requests {
        let result = match request {
            Request::Export {
                source,
                destination,
                reply: export_reply,
            } => {
                let _ = export_reply.send(export_with_powerpoint(&source, &destination));
                Ok(None)
            }
            request => handle(&mut session, request),
        };
        let _ = reply.send(result);
    }
    if let Some(session) = session.take() {
        let _ = session.close();
    }
    unsafe { CoUninitialize() };
}

fn handle(session: &mut Option<ComSession>, request: Request) -> Reply {
    if let Request::Start(path) = request {
        if let Some(previous) = session.take() {
            previous.close()?;
        }
        let started = start_session(&path)?;
        let position = started.require_position();
        *session = Some(started);
        return position.map(Some);
    }
    if matches!(request, Request::Stop) {
        return session
            .take()
            .map_or(Ok(None), |active| active.close().map(|()| None));
    }
    let Some(active) = session.as_ref() else {
        return match request {
            Request::Position => Ok(None),
            Request::Export { .. } => unreachable!(),
            _ => Err("No external presentation is running.".into()),
        };
    };
    match request {
        Request::Position => {
            let position = active.position()?;
            if position.is_none() {
                if let Some(ended) = session.take() {
                    let _ = ended.close();
                }
            }
            Ok(position)
        }
        Request::Next => step(active, |view| view.call("Next", vec![])),
        Request::Previous => step(active, |view| view.call("Previous", vec![])),
        Request::GoTo(slide) => step(active, |view| {
            view.call("GotoSlide", vec![VARIANT::from(slide as i32)])
        }),
        Request::Start(_) | Request::Export { .. } | Request::Stop => unreachable!(),
    }
}

fn step(active: &ComSession, action: impl FnOnce(&Dispatch) -> Result<VARIANT, String>) -> Reply {
    action(&active.view()?)?;
    active.require_position().map(Some)
}

#[derive(Default)]
pub struct PowerPointBackend {
    worker: Option<(Sender<(Request, Sender<Reply>)>, JoinHandle<()>)>,
}

impl PowerPointBackend {
    fn send(&mut self, request: Request) -> Reply {
        if self.worker.is_none() {
            let (sender, receiver) = mpsc::channel();
            let handle = thread::Builder::new()
                .name("external-presentation-com".into())
                .spawn(move || serve(receiver))
                .map_err(|error| {
                    format!("Could not start the PowerPoint automation thread: {error}")
                })?;
            self.worker = Some((sender, handle));
        }
        let (sender, _) = self.worker.as_ref().expect("worker was just created");
        let (reply_sender, reply) = mpsc::channel();
        sender
            .send((request, reply_sender))
            .map_err(|_| "The PowerPoint automation thread stopped.".to_string())?;
        reply
            .recv()
            .map_err(|_| "The PowerPoint automation thread stopped.".to_string())?
    }

    fn position_of(&mut self, request: Request) -> Result<Position, String> {
        self.send(request)?
            .ok_or_else(|| "The PowerPoint slideshow has ended.".to_string())
    }

    fn export(&mut self, source: &Path, destination: &Path) -> Result<ExportedSlideImages, String> {
        let (sender, reply) = mpsc::channel();
        if self
            .send(Request::Export {
                source: source.to_path_buf(),
                destination: destination.to_path_buf(),
                reply: sender,
            })?
            .is_some()
        {
            return Err(
                "The PowerPoint automation thread returned an invalid export response.".into(),
            );
        }
        reply
            .recv()
            .map_err(|_| "The PowerPoint automation thread stopped.".to_string())?
    }
}

impl Drop for PowerPointBackend {
    fn drop(&mut self) {
        if let Some((sender, handle)) = self.worker.take() {
            drop(sender);
            let _ = handle.join();
        }
    }
}

impl Backend for PowerPointBackend {
    fn supports(&self, application: Application) -> Result<(), String> {
        match application {
            Application::PowerPoint => Ok(()),
            Application::Keynote => {
                Err("Keynote presentations can only be played on macOS.".into())
            }
        }
    }

    fn start(&mut self, _: Application, path: &Path) -> Result<Position, String> {
        self.position_of(Request::Start(path.to_path_buf()))
    }

    fn next(&mut self) -> Result<Position, String> {
        self.position_of(Request::Next)
    }

    fn previous(&mut self) -> Result<Position, String> {
        self.position_of(Request::Previous)
    }

    fn go_to(&mut self, slide: u32) -> Result<Position, String> {
        self.position_of(Request::GoTo(slide))
    }

    fn position(&mut self) -> Result<Option<Position>, String> {
        self.send(Request::Position)
    }

    fn stop(&mut self) -> Result<(), String> {
        self.send(Request::Stop).map(|_| ())
    }

    fn export_images(
        &mut self,
        source: &Path,
        destination: &Path,
    ) -> Result<ExportedSlideImages, String> {
        self.export(source, destination)
    }
}
