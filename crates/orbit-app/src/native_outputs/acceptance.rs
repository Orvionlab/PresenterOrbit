//! Opt-in browser evidence. It is never run during normal startup or a unit-test invocation.
use futures::{
    channel::{mpsc::unbounded, oneshot},
    StreamExt,
};
use gpui::{div, App, AppContext, Context, Render, Window, WindowOptions};
use image::DynamicImage;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::capture_process as capture;
#[cfg(windows)]
use super::web_capture as capture;

struct View;
impl Render for View {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
    }
}
fn wait<T>(label: &str, mut check: impl FnMut() -> Result<Option<T>, String>) -> Result<T, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(value) = check()? {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(format!("Timed out waiting for {label}."));
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}
fn pixel(image: &DynamicImage) -> [u8; 4] {
    image
        .to_rgba8()
        .get_pixel(image.width() / 2, image.height() / 2)
        .0
}
fn red(pixel: [u8; 4]) -> bool {
    pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40 && pixel[3] == 255
}
fn fixture() -> Result<String, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let base = format!(
        "http://{}/",
        listener.local_addr().map_err(|error| error.to_string())?
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut request = [0; 4096];
                let read = stream.read(&mut request).unwrap_or(0);
                if request[..read].starts_with(b"GET /hang ") {
                    std::thread::sleep(Duration::from_secs(60));
                    return;
                }
                let body = if request[..read].starts_with(b"GET /live ") {
                    "<html><body style='margin:0;background:#0000ff'><script>let on=false;setInterval(()=>{on=!on;document.body.style.background=on?'#00ff00':'#0000ff'},150)</script></body></html>"
                } else if request[..read].starts_with(b"GET /alpha ") {
                    "<html><body style='margin:0;background:transparent'><div style='width:50vw;height:50vh;background:rgba(255,0,0,0.5)'></div></body></html>"
                } else {
                    "<html><body style='margin:0;background:#ff0000'></body></html>"
                };
                let _=write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            });
        }
    });
    Ok(base)
}
fn exercise(
    host: &orbit_host::Host,
    base: &str,
    stall: oneshot::Sender<()>,
    started: mpsc::Receiver<()>,
    finished: Arc<std::sync::atomic::AtomicBool>,
) -> Result<Value, String> {
    let begin = Instant::now();
    let (width, height) = (320, 180);
    let monitors = host.available_monitors()?;
    if monitors.is_empty() {
        return Err("No monitor is available to exercise the native output event loop.".into());
    }
    let native = host
        .windows()
        .create_windowed_output_window("acceptance-native-output", "Native output acceptance")?;
    native.set_size(orbit_host::PhysicalSize::new(width, height))?;
    native.show()?;
    let first = wait("red browser pixels", || {
        Ok(orbit_host::web_capture::frame(base, width, height)?.filter(|image| red(pixel(image))))
    })?;
    if first.width() != width || first.height() != height {
        return Err(format!(
            "Expected physical raster {width}x{height}, got {}x{}.",
            first.width(),
            first.height()
        ));
    }
    let live = format!("{base}live");
    let (mut green, mut blue) = (false, false);
    wait("independent script colors", || {
        orbit_host::web_capture::frame(base, width, height)?;
        if let Some(image) = orbit_host::web_capture::frame(&live, width, height)? {
            let p = pixel(&image);
            green |= p[1] > 200 && p[0] < 40 && p[2] < 40;
            blue |= p[2] > 200 && p[0] < 40 && p[1] < 40;
        }
        Ok((green && blue).then_some(()))
    })?;
    wait("two native browser owners", || {
        Ok((capture::page_count() == 2).then_some(()))
    })?;
    for invalid in [
        "file:///private/test.html",
        "javascript:alert(1)",
        "about:blank",
    ] {
        if orbit_host::web_capture::frame(invalid, width, height).is_ok() {
            return Err(format!("The browser accepted a forbidden URL: {invalid}"));
        }
    }
    let alpha = format!("{base}alpha");
    let image = wait("transparent browser pixels", || {
        Ok(
            orbit_host::web_capture::frame(&alpha, width, height)?.filter(|image| {
                let rgba = image.to_rgba8();
                let clear = rgba
                    .get_pixel(image.width() * 3 / 4, image.height() * 3 / 4)
                    .0;
                let ink = rgba.get_pixel(image.width() / 4, image.height() / 4).0;
                clear[3] == 0
                    && ink[0] > 240
                    && ink[1] < 10
                    && ink[2] < 10
                    && (120..=135).contains(&ink[3])
            }),
        )
    })?;
    let ink = image
        .to_rgba8()
        .get_pixel(image.width() / 4, image.height() / 4)
        .0;
    // Match the old no-timeout contract: a hung page must not hold up other pages or GPUI.
    orbit_host::web_capture::frame(&format!("{base}hang"), width, height)?;
    stall.send(()).map_err(|_| "The GPUI stall task stopped.")?;
    started
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "GPUI never entered the requested stall.")?;
    let (mut changes, mut previous) = (0, None);
    let stalled = Instant::now();
    while stalled.elapsed() < Duration::from_secs(9) {
        orbit_host::web_capture::frame(&format!("{base}hang"), width, height)?;
        if let Some(image) = orbit_host::web_capture::frame(&live, width, height)? {
            let p = pixel(&image);
            if previous.is_some_and(|old| old != p) {
                changes += 1;
            }
            previous = Some(p);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    if finished.load(std::sync::atomic::Ordering::Acquire) {
        return Err("The GPUI stall ended before the nine-second capture sample completed.".into());
    }
    if changes < 4 {
        return Err(format!(
            "Web capture did not keep changing during the GPUI stall: {changes} changes."
        ));
    }
    wait("idle browser retirement", || {
        Ok((capture::page_count() == 0).then_some(()))
    })?;
    wait("retired page reopening", || {
        Ok(orbit_host::web_capture::frame(base, width, height)?.filter(|image| red(pixel(image))))
    })?;
    capture::close_all();
    wait("explicit browser owner shutdown", || {
        Ok((capture::page_count() == 0).then_some(()))
    })?;
    native.close()?;
    Ok(
        json!({"schemaVersion":2,"platform":std::env::consts::OS,"requestedSize":[width,height],"capturedSize":[first.width(),first.height()],"independentPages":2,"observedColors":["red","green","blue"],"forbiddenUrlsRejected":true,"transparentOverlay":true,"overlayAlpha":ink[3],"gpuiStallSeconds":10,"colorChangesDuringStall":changes,"hungPageDoesNotBlockOtherPages":true,"idleRetirement":true,"reopenedAfterRetirement":true,"explicitShutdown":true,"nativeWindowEventLoopCoexistsWithGpui":true,"physicalOutputParity":"hardware acceptance pending","elapsedSeconds":begin.elapsed().as_secs_f64()}),
    )
}
pub fn run() {
    let directory = std::env::temp_dir().join(format!(
        "orbit-gpui-web-acceptance-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).expect("acceptance data directory");
    let base = fixture().expect("acceptance HTTP server");
    let (sender, mut receiver) = unbounded();
    let (host, services) = crate::shell::build_host(sender, Some(directory));
    let shutdown = services.outputs.clone();
    let report = Arc::new(Mutex::new(None));
    let saved = report.clone();
    let (stall, requested) = oneshot::channel();
    let (entered, started) = mpsc::sync_channel(1);
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_finished = finished.clone();
    let (done, completed) = oneshot::channel();
    gpui_platform::application().run(move |cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| View))
            .expect("acceptance GPUI window");
        cx.spawn(async move |cx| {
            while let Some(message) = receiver.next().await {
                if let crate::shell::ForegroundMessage::Task(task) = message {
                    cx.update(|_| task());
                }
            }
        })
        .detach();
        cx.spawn(async move |cx| {
            if requested.await.is_ok() {
                cx.update(|_| {
                    let _ = entered.send(());
                    std::thread::sleep(Duration::from_secs(10));
                    finished.store(true, std::sync::atomic::Ordering::Release);
                });
            }
        })
        .detach();
        std::thread::spawn(move || {
            let result = exercise(&host, &base, stall, started, worker_finished);
            *saved.lock().unwrap() = Some(result);
            let _ = done.send(());
        });
        cx.spawn(async move |cx| {
            let _ = completed.await;
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    shutdown.shutdown();
    let result = report
        .lock()
        .unwrap()
        .take()
        .expect("acceptance worker result");
    match result {
        Ok(report) => println!("{}", serde_json::to_string_pretty(&report).unwrap()),
        Err(error) => {
            eprintln!("GPUI browser acceptance failed: {error}");
            std::process::exit(1);
        }
    }
}
