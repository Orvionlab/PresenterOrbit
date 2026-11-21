//! One browser process owns its platform main loop. GPUI stalls cannot block its callbacks or pipe reader.
use super::capture_protocol::{
    deliver, encode, interpret, read_packet, write_packet, Captured, Header, Pending, Update,
    Wanted, MAX_FRAME, MAX_HEADER,
};
use crate::locked::Locked;
use image::DynamicImage;
use orbit_host::web_capture::{WebCapture, MAX_SIZE};
use std::{
    collections::HashMap,
    io::{BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
#[path = "capture_linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "capture_macos.rs"]
mod platform;

/// The output process runs AppKit on its main thread without GPUI.
#[cfg(target_os = "macos")]
pub(super) fn initialize_app() -> Result<(), String> {
    platform::initialize()
}
#[cfg(target_os = "macos")]
pub(super) fn pump_app() {
    platform::pump()
}

static SERVICE: OnceLock<Arc<Capture>> = OnceLock::new();
pub fn install(directory: Result<PathBuf, String>) {
    let service = SERVICE.get_or_init(|| {
        Arc::new(Capture {
            directory,
            sessions: Mutex::default(),
            child: Mutex::default(),
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            live_pages: std::sync::atomic::AtomicUsize::new(0),
        })
    });
    orbit_host::web_capture::install(service.clone());
}
pub fn shutdown() {
    if let Some(service) = SERVICE.get() {
        service.stopped.store(true, Ordering::Release);
        if let Some(mut child) = service.child.locked().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
struct Capture {
    live_pages: std::sync::atomic::AtomicUsize,
    directory: Result<PathBuf, String>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    child: Mutex<Option<Child>>,
    started: AtomicBool,
    stopped: AtomicBool,
}
struct Session {
    wanted: Wanted,
    used: Mutex<Instant>,
    latest: Mutex<Option<Arc<DynamicImage>>>,
    error: Mutex<Option<String>>,
}
impl WebCapture for Capture {
    fn frame(&self, url: &url::Url, size: (u32, u32)) -> Result<Option<Arc<DynamicImage>>, String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("The web capture helper has stopped.".into());
        }
        self.directory.as_ref().map_err(Clone::clone)?;
        let key = format!("{}x{} {url}", size.0, size.1);
        let session = self
            .sessions
            .locked()
            .entry(key.clone())
            .or_insert_with(|| {
                Arc::new(Session {
                    wanted: Wanted {
                        key,
                        url: url.to_string(),
                        width: size.0,
                        height: size.1,
                    },
                    used: Mutex::new(Instant::now()),
                    latest: Mutex::default(),
                    error: Mutex::default(),
                })
            })
            .clone();
        *session.used.locked() = Instant::now();
        if !self.started.swap(true, Ordering::AcqRel) {
            let service = SERVICE
                .get()
                .ok_or("Web capture is not installed.")?
                .clone();
            if let Err(error) = std::thread::Builder::new()
                .name("orbit-web-capture-pipe".into())
                .spawn(move || {
                    if let Err(error) = serve(&service) {
                        if !service.stopped.load(Ordering::Acquire) {
                            fail(&service, error);
                        }
                    }
                })
            {
                self.started.store(false, Ordering::Release);
                return Err(error.to_string());
            }
        }
        if let Some(error) = session.error.locked().clone() {
            return Err(error);
        }
        let frame = session.latest.locked().clone();
        Ok(frame)
    }
}
fn fail(service: &Capture, error: String) {
    eprintln!("[web-capture] {error}");
    for session in service.sessions.locked().values() {
        *session.error.locked() = Some(error.clone());
    }
    service.stopped.store(true, Ordering::Release);
    if let Some(mut child) = service.child.locked().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
fn serve(service: &Arc<Capture>) -> Result<(), String> {
    if service.stopped.load(Ordering::Acquire) {
        return Ok(());
    }
    let directory = service
        .directory
        .as_ref()
        .map_err(Clone::clone)?
        .join("web-fills");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut child = Command::new(std::env::current_exe().map_err(|error| error.to_string())?)
        .arg("--web-fill-helper")
        .arg(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("The web capture helper could not start: {error}"))?;
    let mut input = child
        .stdin
        .take()
        .ok_or("The web capture helper has no command pipe.")?;
    let output = child
        .stdout
        .take()
        .ok_or("The web capture helper has no frame pipe.")?;
    {
        let mut owner = service.child.locked();
        if service.stopped.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        *owner = Some(child);
    }
    let writer = service.clone();
    std::thread::Builder::new()
        .name("orbit-web-capture-requests".into())
        .spawn(move || {
            while !writer.stopped.load(Ordering::Acquire) {
                let wanted = {
                    let mut sessions = writer.sessions.locked();
                    sessions.retain(|_, session| {
                        session.used.locked().elapsed() < Duration::from_secs(5)
                    });
                    sessions
                        .values()
                        .map(|session| session.wanted.clone())
                        .collect::<Vec<_>>()
                };
                let sent = (|| -> Result<(), String> {
                    let bytes = serde_json::to_vec(&wanted).map_err(|error| error.to_string())?;
                    if bytes.len() > MAX_HEADER {
                        return Err(
                            "The web capture request exceeds the pipe message limit.".into()
                        );
                    }
                    write_packet(&mut input, &bytes)?;
                    input.flush().map_err(|error| error.to_string())
                })();
                if let Err(error) = sent {
                    if !writer.stopped.load(Ordering::Acquire) {
                        fail(
                            &writer,
                            format!("The web capture command pipe failed: {error}"),
                        );
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
        .map_err(|error| error.to_string())?;
    let mut output = BufReader::new(output);
    while !service.stopped.load(Ordering::Acquire) {
        let bytes = read_packet(&mut output, MAX_HEADER)?;
        let header: Header = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Invalid web capture frame header: {error}"))?;
        let bytes = read_packet(&mut output, MAX_FRAME)?;
        if let Some(pages) = header.pages {
            if !bytes.is_empty() {
                return Err("A browser status message contains unexpected pixels.".into());
            }
            service.live_pages.store(pages, Ordering::Release);
            continue;
        }
        let session = service.sessions.locked().get(&header.key).cloned();
        if let Some(session) = session {
            match interpret(header, bytes)? {
                Update::Error(error) => *session.error.locked() = Some(error),
                Update::Frame(image) => *session.latest.locked() = Some(Arc::new(image)),
            }
        }
    }
    Ok(())
}
pub fn run_helper() -> Result<(), String> {
    let directory = std::env::args_os()
        .nth(2)
        .map(PathBuf::from)
        .ok_or("The browser data directory is required.")?;
    let stopped = Arc::new(AtomicBool::new(false));
    let wanted = Arc::new(Mutex::new(Vec::<Wanted>::new()));
    let pending: Pending = Arc::default();
    let (stop_read, requests) = (stopped.clone(), wanted.clone());
    std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        loop {
            let result = read_packet(&mut input, MAX_HEADER).and_then(|bytes| {
                serde_json::from_slice::<Vec<Wanted>>(&bytes).map_err(|error| error.to_string())
            });
            match result {
                Ok(next) => *requests.locked() = next,
                Err(error) => {
                    eprintln!("[web-fill-helper] {error}");
                    stop_read.store(true, Ordering::Release);
                    break;
                }
            }
        }
    });
    let (stop_decode, frames) = (stopped.clone(), pending.clone());
    let decoder = std::thread::spawn(move || -> Result<(), String> {
        let mut output = std::io::stdout().lock();
        while !stop_decode.load(Ordering::Acquire) {
            let frames = std::mem::take(&mut *frames.locked());
            for (key, captured) in frames {
                let (header, bytes) = encode(key, captured)?;
                let header = serde_json::to_vec(&header).map_err(|error| error.to_string())?;
                for packet in [&header, &bytes] {
                    write_packet(&mut output, packet)?;
                }
                output.flush().map_err(|error| error.to_string())?;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        Ok(())
    });
    let result = (|| {
        platform::initialize()?;
        let mut context = wry::WebContext::new(Some(directory));
        let mut pages = HashMap::<String, platform::Page>::new();
        let mut reported_pages = usize::MAX;
        while !stopped.load(Ordering::Acquire) {
            if decoder.is_finished() {
                return Err("The browser frame writer stopped.".into());
            }
            let requested = wanted.locked().clone();
            pages.retain(|key, _| requested.iter().any(|wanted| &wanted.key == key));
            for request in requested {
                if let Err(error) = orbit_host::web_capture::page_url(&request.url) {
                    deliver(&pending, &request.key, Captured::Error(error));
                    continue;
                }
                if request.width == 0
                    || request.height == 0
                    || request.width > MAX_SIZE.0
                    || request.height > MAX_SIZE.1
                {
                    deliver(
                        &pending,
                        &request.key,
                        Captured::Error("Invalid web capture dimensions.".into()),
                    );
                    continue;
                }
                if !pages.contains_key(&request.key) {
                    match platform::Page::new(&mut context, &request) {
                        Ok(page) => {
                            pages.insert(request.key.clone(), page);
                        }
                        Err(error) => {
                            deliver(&pending, &request.key, Captured::Error(error));
                            continue;
                        }
                    }
                }
                pages
                    .get_mut(&request.key)
                    .unwrap()
                    .capture(&request, &pending);
            }
            if pages.len() != reported_pages {
                reported_pages = pages.len();
                deliver(&pending, "", Captured::Pages(reported_pages));
            }
            platform::pump();
            std::thread::sleep(Duration::from_millis(15));
        }
        Ok::<_, String>(())
    })();
    stopped.store(true, Ordering::Release);
    let written = decoder
        .join()
        .map_err(|_| "The browser frame writer panicked.".to_string())?;
    result.and(written)
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
