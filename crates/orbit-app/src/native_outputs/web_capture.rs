//! WebView2 and COM stay on their own thread; compositor callers only read the latest image.

use crate::locked::Locked;
use image::DynamicImage;
use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use std::{
    collections::HashMap,
    num::NonZeroIsize,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use windows::{
    core::w,
    Win32::{
        Foundation::HWND,
        System::Com::*,
        UI::{HiDpi::*, Shell::SHCreateMemStream, WindowsAndMessaging::*},
    },
};
use wry::{WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

static SERVICE: OnceLock<Arc<Capture>> = OnceLock::new();

pub fn install(directory: Result<PathBuf, String>) {
    let capture = SERVICE.get_or_init(|| {
        Arc::new(Capture {
            sessions: Mutex::default(),
            directory,
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            live_pages: std::sync::atomic::AtomicUsize::new(0),
        })
    });
    orbit_host::web_capture::install(capture.clone());
}

pub fn shutdown() {
    if let Some(service) = SERVICE.get() {
        service.stopped.store(true, Ordering::Release);
    }
}

struct Capture {
    live_pages: std::sync::atomic::AtomicUsize,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    directory: Result<PathBuf, String>,
    started: AtomicBool,
    stopped: AtomicBool,
}

struct Session {
    url: String,
    size: (u32, u32),
    used: Mutex<Instant>,
    frame: Mutex<Option<Arc<DynamicImage>>>,
    error: Mutex<Option<String>>,
    capturing: AtomicBool,
    // At most one pending PNG per page, even when decoding is slower than capture.
    encoded: Mutex<Option<Vec<u8>>>,
}

impl orbit_host::web_capture::WebCapture for Capture {
    fn frame(&self, url: &url::Url, size: (u32, u32)) -> Result<Option<Arc<DynamicImage>>, String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("Web capture has stopped.".into());
        }
        self.directory.as_ref().map_err(Clone::clone)?;
        let key = format!("{}x{} {url}", size.0, size.1);
        let session = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "Web capture sessions are unavailable.")?;
            sessions
                .entry(key)
                .or_insert_with(|| {
                    Arc::new(Session {
                        url: url.to_string(),
                        size,
                        used: Mutex::new(Instant::now()),
                        frame: Mutex::default(),
                        error: Mutex::default(),
                        capturing: AtomicBool::new(false),
                        encoded: Mutex::default(),
                    })
                })
                .clone()
        };
        *session
            .used
            .lock()
            .map_err(|_| "The web capture timestamp is unavailable.")? = Instant::now();
        if !self.started.swap(true, Ordering::AcqRel) {
            let service = SERVICE
                .get()
                .ok_or("Web capture is not installed.")?
                .clone();
            let worker = service.clone();
            if let Err(error) = std::thread::Builder::new()
                .name("orbit-web-capture".into())
                .spawn(move || run(worker))
            {
                self.started.store(false, Ordering::Release);
                return Err(format!("Web capture could not start: {error}"));
            }
            if let Err(error) = std::thread::Builder::new()
                .name("orbit-web-decode".into())
                .spawn(move || decode(service))
            {
                self.stopped.store(true, Ordering::Release);
                return Err(format!("Web capture decoding could not start: {error}"));
            }
        }
        if let Some(error) = session
            .error
            .lock()
            .map_err(|_| "Web capture status is unavailable.")?
            .clone()
        {
            return Err(error);
        }
        let frame = session
            .frame
            .lock()
            .map_err(|_| "The captured web frame is unavailable.")?
            .clone();
        Ok(frame)
    }
}

struct Parent(HWND);
impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let hwnd = NonZeroIsize::new(self.0 .0 as isize).ok_or(HandleError::Unavailable)?;
        Ok(unsafe {
            WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(hwnd)))
        })
    }
}
impl Drop for Parent {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}
struct Page {
    // Drop the browser before its native parent.
    webview: WebView,
    _parent: Parent,
}

fn page(context: &mut WebContext, session: &Session) -> Result<Page, String> {
    let parent = Parent(
        unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                w!("Presenter Orbit web fill"),
                WS_POPUP,
                0,
                0,
                session.size.0 as i32,
                session.size.1 as i32,
                None,
                None,
                None,
                None,
            )
        }
        .map_err(|error| error.to_string())?,
    );
    let webview = WebViewBuilder::new_with_web_context(context)
        .with_url(&session.url)
        .with_transparent(true)
        .with_background_color((0, 0, 0, 0))
        .with_additional_browser_args("--autoplay-policy=no-user-gesture-required")
        .with_bounds(wry::Rect {
            position: wry::dpi::PhysicalPosition::new(0, 0).into(),
            size: wry::dpi::PhysicalSize::new(session.size.0, session.size.1).into(),
        })
        .build_as_child(&parent)
        .map_err(|error| format!("Web fill page could not open: {error}"))?;
    Ok(Page {
        webview,
        _parent: parent,
    })
}

fn run(service: Arc<Capture>) {
    let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok();
    if let Err(error) = result {
        fail_all(
            &service,
            format!("Web capture COM initialization failed: {error}"),
        );
        return;
    }
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    if let Err(error) = run_pages(&service) {
        fail_all(&service, error);
    }
    unsafe {
        CoUninitialize();
    }
}

fn fail_all(service: &Capture, error: String) {
    eprintln!("[web-capture] {error}");
    if let Ok(sessions) = service.sessions.lock() {
        for session in sessions.values() {
            *session.error.locked() = Some(error.clone());
        }
    }
    service.stopped.store(true, Ordering::Release);
}

fn run_pages(service: &Capture) -> Result<(), String> {
    let directory = service
        .directory
        .as_ref()
        .map_err(Clone::clone)?
        .join("web-fills");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("The web fill data directory could not be created: {error}"))?;
    let mut context = WebContext::new(Some(directory));
    let mut pages = HashMap::<String, Page>::new();
    while !service.stopped.load(Ordering::Acquire) {
        let sessions = {
            let mut sessions = service
                .sessions
                .lock()
                .map_err(|_| "Web capture sessions are unavailable.")?;
            sessions.retain(|_, session| {
                session
                    .used
                    .lock()
                    .is_ok_and(|used| used.elapsed() < Duration::from_secs(5))
            });
            sessions.clone()
        };
        pages.retain(|key, _| sessions.contains_key(key));
        for (key, session) in sessions {
            if session.error.locked().is_some() {
                continue;
            }
            if !pages.contains_key(&key) {
                match page(&mut context, &session) {
                    Ok(page) => {
                        pages.insert(key.clone(), page);
                    }
                    Err(error) => {
                        *session.error.locked() = Some(error);
                        continue;
                    }
                }
            }
            if !session.capturing.swap(true, Ordering::AcqRel) {
                if let Err(error) = capture(&pages[&key].webview, session.clone()) {
                    session.capturing.store(false, Ordering::Release);
                    *session.error.locked() = Some(error);
                }
            }
        }
        unsafe {
            super::win32::pump_messages();
        }
        service.live_pages.store(pages.len(), Ordering::Release);
        std::thread::sleep(Duration::from_millis(15));
    }
    // Browsers and parents are dropped here, before COM is uninitialized.
    drop(pages);
    service.live_pages.store(0, Ordering::Release);
    Ok(())
}

#[cfg(feature = "web-fill-acceptance")]
pub fn page_count() -> usize {
    SERVICE
        .get()
        .map_or(0, |service| service.live_pages.load(Ordering::Acquire))
}
#[cfg(feature = "web-fill-acceptance")]
pub fn close_all() {
    if let Some(service) = SERVICE.get() {
        service.sessions.locked().clear();
    }
}

fn capture(webview: &WebView, session: Arc<Session>) -> Result<(), String> {
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    };
    unsafe {
        let controller = webview.controller();
        controller
            .SetIsVisible(true)
            .map_err(|error| error.to_string())?;
        let webview = controller
            .CoreWebView2()
            .map_err(|error| error.to_string())?;
        let stream = SHCreateMemStream(None).ok_or("A web capture stream could not be created.")?;
        let target = stream.clone();
        let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
            let captured = (|| -> Result<Vec<u8>, windows::core::Error> {
                result?;
                let mut stat = Default::default();
                target.Stat(&mut stat, STATFLAG_NONAME)?;
                target.Seek(0, STREAM_SEEK_SET, None)?;
                let mut bytes = vec![0; stat.cbSize as usize];
                let mut read = 0;
                target
                    .Read(
                        bytes.as_mut_ptr().cast(),
                        bytes.len() as u32,
                        Some(&mut read),
                    )
                    .ok()?;
                bytes.truncate(read as usize);
                Ok(bytes)
            })();
            match captured {
                Ok(bytes) => *session.encoded.locked() = Some(bytes),
                Err(error) => {
                    *session.error.locked() =
                        Some(format!("Web fill page could not be captured: {error}"))
                }
            }
            session.capturing.store(false, Ordering::Release);
            Ok(())
        }));
        webview
            .CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &handler,
            )
            .map_err(|error| error.to_string())
    }
}

fn decode(service: Arc<Capture>) {
    while !service.stopped.load(Ordering::Acquire) {
        let sessions: Vec<_> = match service.sessions.lock() {
            Ok(sessions) => sessions.values().cloned().collect(),
            Err(_) => return,
        };
        for session in sessions {
            let bytes = session.encoded.locked().take();
            if let Some(bytes) = bytes {
                match image::load_from_memory(&bytes) {
                    Ok(image) => *session.frame.locked() = Some(Arc::new(image)),
                    Err(error) => {
                        *session.error.locked() = Some(format!(
                            "The captured web image could not be decoded: {error}"
                        ))
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}
