//! The output process: a second `Host` that owns the windows, NDI senders, audio sinks, and program compositor,
//! and takes its scene from one controller over loopback TCP. It outlives the controller for a grace period, so
//! a UI crash leaves the last accepted picture on air.

use std::net::{TcpListener, TcpStream};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use orbit_host::{
    DesktopIntegration, DisplayBounds, Host, HostEvents, HostParts, Monitor, PhysicalPosition,
    Updater, Window, WindowSystem,
};
use serde_json::{json, Value};

use super::protocol::{
    read_message, remove_endpoint, write_endpoint, write_message, Endpoint, FromOutput, ToOutput,
    EVENTS_FORWARDED, INVOKE_ALLOWED, LOCK_FILE, VERSION,
};
use crate::locked::Locked;
use crate::native_outputs::NativeOutputs;

pub const DEFAULT_GRACE: Duration = Duration::from_secs(30 * 60);
/// A freshly started output process that is never attached exits after this long.
const INITIAL_WAIT: Duration = Duration::from_secs(30);
/// A connection that does not say hello in this time is dropped.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PENDING_EVENTS: usize = 64;
pub const EXIT_LOCKED: i32 = 3;

type Task = Box<dyn FnOnce() + Send + 'static>;

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub data_dir: PathBuf,
    /// A test seam: no monitors and no windows, so no test opens a window on a display.
    pub inert: bool,
    pub grace: Duration,
}

/// `--data-dir <path>` is required. `--inert` and `--grace-seconds <n>` are hidden test seams.
pub fn parse_options(mut arguments: impl Iterator<Item = String>) -> Result<Options, String> {
    let (mut data_dir, mut inert, mut grace) = (None, false, DEFAULT_GRACE);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--data-dir" => {
                data_dir = Some(PathBuf::from(
                    arguments.next().ok_or("--data-dir requires a path.")?,
                ))
            }
            "--inert" => inert = true,
            "--grace-seconds" => {
                grace = Duration::from_secs(
                    arguments
                        .next()
                        .and_then(|seconds| seconds.parse().ok())
                        .ok_or("--grace-seconds requires a whole number.")?,
                )
            }
            other => return Err(format!("Unknown output process argument '{other}'.")),
        }
    }
    Ok(Options {
        data_dir: data_dir.ok_or("--data-dir is required.")?,
        inert,
        grace,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Waiting(Instant),
    Controlled,
    Orphaned(Instant),
    Exiting,
}

/// Who controls the output process, and when it should end. Time comes in as an argument, so tests use a fake clock.
pub struct Lifecycle {
    phase: Phase,
    initial_wait: Duration,
    grace: Duration,
}

impl Lifecycle {
    pub fn new(now: Instant, initial_wait: Duration, grace: Duration) -> Self {
        Self {
            phase: Phase::Waiting(now),
            initial_wait,
            grace,
        }
    }

    /// False when another controller holds the output, or the process is ending.
    pub fn attach(&mut self) -> bool {
        let free = matches!(self.phase, Phase::Waiting(_) | Phase::Orphaned(_));
        if free {
            self.phase = Phase::Controlled;
        }
        free
    }

    pub fn detach(&mut self, now: Instant) {
        if self.phase == Phase::Controlled {
            self.phase = Phase::Orphaned(now);
        }
    }

    pub fn shutdown(&mut self) {
        self.phase = Phase::Exiting;
    }

    pub fn poll(&mut self, now: Instant, has_outputs: bool) -> Phase {
        match self.phase {
            Phase::Waiting(since) if now.duration_since(since) >= self.initial_wait => {
                self.phase = Phase::Exiting
            }
            Phase::Orphaned(since) if now.duration_since(since) >= self.grace || !has_outputs => {
                self.phase = Phase::Exiting
            }
            _ => {}
        }
        self.phase
    }
}

type Sink = Arc<Mutex<TcpStream>>;

pub(super) struct Control {
    lifecycle: Lifecycle,
    controller: Option<u32>,
    sink: Option<Sink>,
    /// Events raised while nobody is attached, delivered at the next attach.
    pending: Vec<(String, Value)>,
}

/// Forwards the allowed host events to the controller and drops the rest.
pub(super) struct OutputEvents(pub Arc<Mutex<Control>>);

impl HostEvents for OutputEvents {
    fn emit(&self, event: &str, payload: Value) -> Result<(), String> {
        if !EVENTS_FORWARDED.contains(&event) {
            return Ok(());
        }
        let mut control = self.0.locked();
        match control.sink.clone() {
            Some(sink) => {
                drop(control);
                let message = FromOutput::Event {
                    event: event.to_string(),
                    payload,
                };
                write_message(&mut *sink.locked(), &message)
            }
            None => {
                if control.pending.len() < MAX_PENDING_EVENTS {
                    control.pending.push((event.to_string(), payload));
                }
                Ok(())
            }
        }
    }

    fn emit_to(&self, _label: &str, event: &str, payload: Value) -> Result<(), String> {
        self.emit(event, payload)
    }
}

struct OutputWindows {
    outputs: Arc<NativeOutputs>,
    inert: bool,
    tasks: Sender<Task>,
}

impl OutputWindows {
    fn live<T>(&self, call: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        if self.inert {
            return Err("The output process runs without displays.".into());
        }
        call()
    }
}

impl WindowSystem for OutputWindows {
    fn get_window(&self, label: &str) -> Option<Window> {
        self.outputs.get(label)
    }
    fn available_monitors(&self) -> Result<Vec<Monitor>, String> {
        if self.inert {
            return Ok(Vec::new());
        }
        self.outputs.monitors(false)
    }
    fn primary_monitor(&self) -> Result<Option<Monitor>, String> {
        if self.inert {
            return Ok(None);
        }
        Ok(self.outputs.monitors(true)?.into_iter().next())
    }
    fn cursor_position(&self) -> Result<PhysicalPosition<f64>, String> {
        self.live(|| self.outputs.cursor())
    }
    fn run_on_main_thread(&self, task: Task) -> Result<(), String> {
        self.tasks
            .send(task)
            .map_err(|_| "The output process main thread has stopped.".to_string())
    }
    fn output_windows_use_main_thread(&self) -> bool {
        cfg!(target_os = "macos")
    }
    fn create_windowed_output_window(&self, label: &str, title: &str) -> Result<Window, String> {
        self.live(|| self.outputs.create(label, title, None, None))
    }
    fn create_display_output_window(
        &self,
        label: &str,
        title: &str,
        bounds: DisplayBounds,
    ) -> Result<Window, String> {
        self.live(|| self.outputs.create(label, title, Some(bounds), None))
    }
    fn create_output_control_window(
        &self,
        screen_id: &str,
        label: &str,
        owner_label: &str,
        bounds: DisplayBounds,
    ) -> Result<Window, String> {
        self.live(|| {
            self.outputs.create(
                label,
                "Close output",
                Some(bounds),
                Some((screen_id, owner_label)),
            )
        })
    }
    fn prepare_auxiliary_windows(&self) -> Result<Value, String> {
        Err("The output process has no auxiliary windows.".into())
    }
    fn open_auxiliary_window(&self, _: Value) -> Result<Value, String> {
        Err("The output process has no auxiliary windows.".into())
    }
    fn close_auxiliary_window(&self, _: &Value) -> Result<Value, String> {
        Err("The output process has no auxiliary windows.".into())
    }
    fn minimize_main_window(&self) -> Result<(), String> {
        Err("The output process has no main window.".into())
    }
}

struct NoDesktop;

impl DesktopIntegration for NoDesktop {
    fn apply_preferences(&self, _: &Value, _: &Value) -> Result<(), String> {
        Ok(())
    }
}

struct NoUpdater;

impl Updater for NoUpdater {
    fn status(&self) -> Value {
        json!({ "status": "unavailable" })
    }
    fn check(&self, _: &Host, _: bool) -> Result<Value, String> {
        Err("The output process does not update the application.".into())
    }
    fn install(&self, _: &Host) -> Result<Value, String> {
        Err("The output process does not update the application.".into())
    }
}

pub struct Shared {
    pub host: Host,
    /// An inert process holds no outputs, so only the grace ends an orphan; a test cannot hold a window.
    inert: bool,
    token: String,
    control: Arc<Mutex<Control>>,
    /// True once a scene was accepted, so an attaching controller knows there is something on air.
    scene_accepted: AtomicBool,
    /// The last routing applied to the windows, so an unchanged one never recreates them.
    applied_routing: Mutex<Option<Value>>,
    #[cfg(test)]
    pub routing_attempts: std::sync::atomic::AtomicU64,
}

/// The `screenConfig` an `updateOutputScene` request carries, in a full snapshot or in a patch.
fn requested_routing(request: &Value) -> Option<Value> {
    let carrier = if request["kind"] == "patch" {
        &request["patch"]
    } else {
        &request["snapshot"]
    };
    carrier
        .get("screenConfig")
        .filter(|value| value.is_object())
        .cloned()
}

fn scene_accepted(result: &Value) -> bool {
    result["ok"] == true && result["stale"] != true && result["cacheMiss"] != true
}

impl Shared {
    fn invoke(&self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        if !INVOKE_ALLOWED.contains(&method) {
            return Err(format!("The output process does not accept '{method}'."));
        }
        if method == "controlPlayback"
            && !args
                .first()
                .is_some_and(super::protocol::transport_command_allowed)
        {
            return Err("The output process runs only clock commands for playback.".into());
        }
        let routing = if method == "updateOutputScene" {
            args.first().and_then(requested_routing)
        } else {
            None
        };
        let result = catch_unwind(AssertUnwindSafe(|| self.host.invoke(method, args)))
            .map_err(|_| format!("The output process failed while running '{method}'."))??;
        if method == "updateOutputScene" && scene_accepted(&result) {
            self.scene_accepted.store(true, Ordering::Release);
            if let Some(routing) = routing {
                self.apply_routing(routing);
            }
        }
        Ok(result)
    }

    /// Routing follows the scene: a changed `screenConfig` opens, moves, or closes the windows.
    fn apply_routing(&self, routing: Value) {
        let mut applied = self.applied_routing.locked();
        if applied.as_ref() == Some(&routing) {
            return;
        }
        #[cfg(test)]
        self.routing_attempts.fetch_add(1, Ordering::Relaxed);
        match self.host.invoke(
            "setDisplayOutputVisibility",
            vec![json!([]), json!(true), routing.clone()],
        ) {
            Ok(_) => *applied = Some(routing),
            Err(error) => {
                let _ = self.host.emit("output:display-error", error);
            }
        }
    }

    fn hello(&self, version: u32, token: &str, bit_depth: &str) -> Result<(), String> {
        if version != VERSION {
            return Err(format!(
                "The output process speaks protocol {VERSION}, not {version}."
            ));
        }
        if token != self.token {
            return Err("The output process token does not match.".into());
        }
        orbit_host::start_output_host(&self.host, bit_depth)
    }

    fn serve(&self, mut stream: TcpStream) {
        // An accepted socket inherits the listener's non-blocking mode on Windows.
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(HELLO_TIMEOUT));
        let controller_pid = match read_message::<ToOutput>(&mut stream) {
            Ok(ToOutput::Hello {
                version,
                token,
                controller_pid,
                rendering_bit_depth,
            }) => match self.hello(version, &token, &rendering_bit_depth) {
                Ok(()) => controller_pid,
                Err(reason) => {
                    let _ = write_message(&mut stream, &FromOutput::Reject { reason });
                    return;
                }
            },
            _ => return,
        };
        let Ok(writer) = stream.try_clone() else {
            return;
        };
        let sink: Sink = Arc::new(Mutex::new(writer));
        // Publish the sink only while owning its writer, so events cannot overtake Welcome or pending events.
        let mut hello_writer = sink.locked();
        let pending = {
            let mut control = self.control.locked();
            if !control.lifecycle.attach() {
                let busy = FromOutput::Busy {
                    controller_pid: control.controller.unwrap_or_default(),
                };
                drop(control);
                let _ = write_message(&mut *hello_writer, &busy);
                return;
            }
            control.controller = Some(controller_pid);
            control.sink = Some(sink.clone());
            std::mem::take(&mut control.pending)
        };
        let welcome = FromOutput::Welcome {
            version: VERSION,
            pid: std::process::id(),
            has_scene: self.scene_accepted.load(Ordering::Acquire),
        };
        let mut live = write_message(&mut *hello_writer, &welcome).is_ok();
        for (event, payload) in pending {
            live = live
                && write_message(&mut *hello_writer, &FromOutput::Event { event, payload }).is_ok();
        }
        drop(hello_writer);
        let _ = stream.set_read_timeout(None);
        let mut shutdown = false;
        if live {
            loop {
                let message = match read_message::<ToOutput>(&mut stream) {
                    Ok(message) => message,
                    Err(error) => {
                        orbit_host::record_diagnostic(
                            &self.host,
                            "output-controller-read-failed",
                            orbit_host::diagnostics::LogLevel::Error,
                            "output",
                            Some(&error),
                        );
                        break;
                    }
                };
                let reply = match message {
                    ToOutput::Invoke { id, method, args } => {
                        let (ok, error) = match self.invoke(&method, args) {
                            Ok(value) => (Some(value), None),
                            Err(error) => (None, Some(error)),
                        };
                        FromOutput::Result { id, ok, error }
                    }
                    ToOutput::Ping => FromOutput::Pong,
                    ToOutput::Shutdown => {
                        shutdown = true;
                        break;
                    }
                    ToOutput::Hello { .. } => FromOutput::Reject {
                        reason: "The controller already said hello.".into(),
                    },
                };
                if write_message(&mut *sink.locked(), &reply).is_err() {
                    break;
                }
            }
        }
        let _ = stream.shutdown(std::net::Shutdown::Both);
        let mut control = self.control.locked();
        control.controller = None;
        control.sink = None;
        if shutdown {
            control.lifecycle.shutdown();
        } else {
            control.lifecycle.detach(Instant::now());
        }
    }
}

/// Runs the output process until it is told to stop or its grace ends. The returned code is the exit code.
pub fn run(options: Options) -> i32 {
    match start(options, |host| crate::panic_hook::install(host.clone())) {
        Ok(started) => started.run(),
        Err(code) => code,
    }
}

pub struct Started {
    pub shared: Arc<Shared>,
    listener: TcpListener,
    queue: Receiver<Task>,
    outputs: Arc<NativeOutputs>,
    lock: std::fs::File,
    data_dir: PathBuf,
}

impl Started {
    pub fn run(self) -> i32 {
        main_loop(&self.listener, &self.shared, &self.queue);
        // A real process closes its sockets by exiting; a test stand-in must close the controller's.
        if let Some(sink) = self.shared.control.locked().sink.take() {
            let _ = sink.locked().shutdown(std::net::Shutdown::Both);
        }
        orbit_host::stop_output_host(&self.shared.host);
        self.outputs.shutdown();
        remove_endpoint(&self.data_dir);
        drop(self.lock);
        0
    }
}

fn failed(error: impl std::fmt::Display) -> i32 {
    eprintln!("[output-process] {error}");
    1
}

/// Takes the process lock, binds the socket, builds the host, and publishes the endpoint. `Err` is the exit code.
pub(super) fn start(options: Options, on_host: impl FnOnce(&Host)) -> Result<Started, i32> {
    std::fs::create_dir_all(&options.data_dir).map_err(failed)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(options.data_dir.join(LOCK_FILE))
        .map_err(failed)?;
    if lock.try_lock().is_err() {
        return Err(EXIT_LOCKED);
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(failed)?;
    listener.set_nonblocking(true).map_err(failed)?;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let control = Arc::new(Mutex::new(Control {
        lifecycle: Lifecycle::new(Instant::now(), INITIAL_WAIT, options.grace),
        controller: None,
        sink: None,
        pending: Vec::new(),
    }));
    let (tasks, queue) = channel::<Task>();
    let outputs = Arc::new(NativeOutputs::default());
    let host = Host::new(HostParts {
        events: Arc::new(OutputEvents(control.clone())),
        windows: Arc::new(OutputWindows {
            outputs: outputs.clone(),
            inert: options.inert,
            tasks,
        }),
        desktop: Arc::new(NoDesktop),
        updater: Arc::new(NoUpdater),
        version: env!("CARGO_PKG_VERSION").to_string(),
        runtime: "gpui-output",
        app_data_dir: Ok(options.data_dir.clone()),
        states: orbit_host::host_states(std::thread::current().id()),
    });
    outputs.attach(host.clone());
    on_host(&host);
    orbit_host::start_output_tickers(&host).map_err(failed)?;
    write_endpoint(
        &options.data_dir,
        &Endpoint {
            port: listener.local_addr().map_err(failed)?.port(),
            token: token.clone(),
            pid: std::process::id(),
            protocol_version: VERSION,
        },
    )
    .map_err(failed)?;
    Ok(Started {
        shared: Arc::new(Shared {
            host,
            inert: options.inert,
            token,
            control,
            scene_accepted: AtomicBool::new(false),
            applied_routing: Mutex::new(None),
            #[cfg(test)]
            routing_attempts: std::sync::atomic::AtomicU64::new(0),
        }),
        listener,
        queue,
        outputs,
        lock,
        data_dir: options.data_dir,
    })
}

/// The main thread serves main-thread tasks and accepts connections; each connection runs on its own thread.
fn main_loop(listener: &TcpListener, shared: &Arc<Shared>, queue: &Receiver<Task>) {
    loop {
        #[cfg(target_os = "macos")]
        crate::native_outputs::pump_app();
        match queue.recv_timeout(Duration::from_millis(10)) {
            Ok(task) => {
                let _ = catch_unwind(AssertUnwindSafe(task));
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let shared = shared.clone();
                let _ = std::thread::Builder::new()
                    .name("orbit-output-connection".into())
                    .spawn(move || {
                        if catch_unwind(AssertUnwindSafe(|| shared.serve(stream))).is_err() {
                            let mut control = shared.control.locked();
                            control.controller = None;
                            control.sink = None;
                            control.lifecycle.detach(Instant::now());
                        }
                    });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => eprintln!("[output-process] accept failed: {error}"),
        }
        let has_outputs = shared.inert || orbit_host::has_physical_outputs(&shared.host);
        let phase = shared
            .control
            .locked()
            .lifecycle
            .poll(Instant::now(), has_outputs);
        if phase == Phase::Exiting {
            return;
        }
    }
}
