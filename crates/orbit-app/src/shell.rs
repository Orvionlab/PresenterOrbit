use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use orbit_host::{
    DesktopIntegration, DisplayBounds, Host, HostEvents, HostParts, Monitor, PhysicalPosition,
    Window, WindowSystem,
};
use serde_json::Value;

use crate::desktop::{AutoLaunchItem, GpuiDesktop, LoginItem, Tray, APP_NAME};
use crate::windows::{self, Kind, Registry, Request};

/// Work the host hands to the GPUI foreground from its own threads.
pub enum ForegroundMessage {
    Event { event: String, payload: Value },
    Task(Box<dyn FnOnce() + Send + 'static>),
    Windows(WindowCommand),
}

/// How long a host thread waits for the foreground to carry out a window change.
const FOREGROUND_TIMEOUT: Duration = Duration::from_secs(10);

/// A window change that needs the GPUI foreground. `Show` answers on `reply`, so the host call that asked for it
/// can fail when the window cannot be shown.
pub enum WindowCommand {
    Prepare,
    Show {
        kind: Kind,
        reply: Sender<Result<(), String>>,
    },
    Hide(Kind),
    MinimizeMain,
    RestoreMain,
    /// Brings the main window back if it is hidden, as when the tray icon is turned off.
    RestoreIfHidden,
    Quit,
}

fn send(
    sender: &UnboundedSender<ForegroundMessage>,
    message: ForegroundMessage,
) -> Result<(), String> {
    sender
        .unbounded_send(message)
        .map_err(|_| "The application foreground has stopped.".to_string())
}

struct GpuiEvents(UnboundedSender<ForegroundMessage>);

impl HostEvents for GpuiEvents {
    fn emit(&self, event: &str, payload: Value) -> Result<(), String> {
        send(
            &self.0,
            ForegroundMessage::Event {
                event: event.to_string(),
                payload,
            },
        )
    }

    // One process with one store, so the Tauri window labels do not select a receiver.
    fn emit_to(&self, _label: &str, event: &str, payload: Value) -> Result<(), String> {
        self.emit(event, payload)
    }
}

struct GpuiWindows {
    sender: UnboundedSender<ForegroundMessage>,
    registry: Arc<Registry>,
    outputs: Arc<crate::native_outputs::NativeOutputs>,
    /// A controller host has no output windows: they belong to the output process.
    controller: bool,
}

const NOT_HERE: &str = "Output windows belong to the output process.";

impl GpuiWindows {
    fn local<T>(&self, call: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        if self.controller {
            return Err(NOT_HERE.into());
        }
        call()
    }

    fn command(&self, command: WindowCommand) -> Result<(), String> {
        send(&self.sender, ForegroundMessage::Windows(command))
    }
}

impl WindowSystem for GpuiWindows {
    fn get_window(&self, label: &str) -> Option<Window> {
        self.outputs.get(label)
    }
    fn available_monitors(&self) -> Result<Vec<Monitor>, String> {
        self.outputs.monitors(false)
    }
    fn primary_monitor(&self) -> Result<Option<Monitor>, String> {
        Ok(self.outputs.monitors(true)?.into_iter().next())
    }
    fn cursor_position(&self) -> Result<PhysicalPosition<f64>, String> {
        self.local(|| self.outputs.cursor())
    }
    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send + 'static>) -> Result<(), String> {
        send(&self.sender, ForegroundMessage::Task(task))
    }
    fn output_windows_use_main_thread(&self) -> bool {
        cfg!(target_os = "macos")
    }
    fn create_windowed_output_window(&self, label: &str, title: &str) -> Result<Window, String> {
        self.local(|| self.outputs.create(label, title, None, None))
    }
    fn create_display_output_window(
        &self,
        label: &str,
        title: &str,
        bounds: DisplayBounds,
    ) -> Result<Window, String> {
        self.local(|| self.outputs.create(label, title, Some(bounds), None))
    }
    fn create_output_control_window(
        &self,
        screen_id: &str,
        label: &str,
        owner_label: &str,
        bounds: DisplayBounds,
    ) -> Result<Window, String> {
        self.local(|| {
            self.outputs.create(
                label,
                "Close output",
                Some(bounds),
                Some((screen_id, owner_label)),
            )
        })
    }
    fn prepare_auxiliary_windows(&self) -> Result<Value, String> {
        self.command(WindowCommand::Prepare)?;
        Ok(windows::ok())
    }
    fn open_auxiliary_window(&self, request: Value) -> Result<Value, String> {
        let request = Request::parse(request)?;
        // The window is created on demand, as the Tauri registry does, then shown now or once it is ready.
        self.command(WindowCommand::Prepare)?;
        if self.registry.request(&request)? {
            let (reply, shown) = channel();
            self.command(WindowCommand::Show {
                kind: request.kind,
                reply,
            })?;
            shown
                .recv_timeout(FOREGROUND_TIMEOUT)
                .map_err(|_| "The auxiliary window could not be shown in time.".to_string())??;
        }
        Ok(windows::ok())
    }
    fn close_auxiliary_window(&self, label: &Value) -> Result<Value, String> {
        let kind = windows::parse_kind(label)?;
        self.registry.close(kind)?;
        self.command(WindowCommand::Hide(kind))?;
        Ok(windows::ok())
    }
    fn minimize_main_window(&self) -> Result<(), String> {
        self.command(WindowCommand::MinimizeMain)
    }
}

/// What the foreground shares with the host: which auxiliary windows are ready, and whether closing to the tray
/// is on.
pub struct Services {
    pub registry: Arc<Registry>,
    pub minimize_to_tray: Arc<AtomicBool>,
    pub outputs: Arc<crate::native_outputs::NativeOutputs>,
}

pub fn build_host_with(
    sender: UnboundedSender<ForegroundMessage>,
    data_directory_override: Option<PathBuf>,
    tray: Arc<dyn Tray>,
    login: Arc<dyn LoginItem>,
    controller: bool,
) -> (Host, Services) {
    let services = Services {
        registry: Arc::new(Registry::default()),
        minimize_to_tray: Arc::new(AtomicBool::new(false)),
        outputs: Arc::new(crate::native_outputs::NativeOutputs::default()),
    };
    let desktop: Arc<dyn DesktopIntegration> = Arc::new(GpuiDesktop::new(
        tray,
        login,
        services.minimize_to_tray.clone(),
    ));
    let main_thread = std::thread::current().id();
    let mut states = if controller {
        let mut states = orbit_host::host_states_controller(main_thread);
        states.insert(crate::output_process::OutputRoute::default());
        states
    } else {
        orbit_host::host_states(main_thread)
    };
    // A private data directory (the tests) keeps new workspaces out of the user's real workspaces folder.
    if let Some(directory) = &data_directory_override {
        states.insert(orbit_host::WorkspacesFolder(directory.join("Workspaces")));
    }
    let quit_sender = sender.clone();
    let host = Host::new(HostParts {
        events: Arc::new(GpuiEvents(sender.clone())),
        windows: Arc::new(GpuiWindows {
            sender,
            registry: services.registry.clone(),
            outputs: services.outputs.clone(),
            controller,
        }),
        desktop,
        updater: Arc::new(crate::updates::GpuiUpdater::new({
            let sender = quit_sender;
            move || send(&sender, ForegroundMessage::Windows(WindowCommand::Quit))
        })),
        version: env!("CARGO_PKG_VERSION").to_string(),
        runtime: "gpui",
        app_data_dir: data_directory_override.map_or_else(crate::carry_over::data_directory, Ok),
        states,
    });
    services.outputs.attach(host.clone());
    (host, services)
}

/// The host of the application: program output lives in the output process, not here.
pub fn build_controller_host(
    sender: UnboundedSender<ForegroundMessage>,
    data_directory_override: Option<PathBuf>,
) -> (Host, Services) {
    build_host_with(
        sender.clone(),
        data_directory_override,
        system_tray(sender),
        Arc::new(AutoLaunchItem::new(APP_NAME)),
        true,
    )
}

/// Starts the link to the output process once the host knows the rendering bit depth. The link reports to the
/// foreground through the host event channel.
pub fn start_output_link(
    host: &Host,
    sender: UnboundedSender<ForegroundMessage>,
    rendering_bit_depth: String,
) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    start_output_link_with(host, sender, rendering_bit_depth, |events| {
        Arc::new(crate::output_process::client::ProcessSpawner {
            executable,
            extra_arguments: Vec::new(),
            events,
        })
    })
}

pub fn start_output_link_with(
    host: &Host,
    sender: UnboundedSender<ForegroundMessage>,
    rendering_bit_depth: String,
    spawner: impl FnOnce(
        crate::output_process::client::EventSink,
    ) -> Arc<dyn crate::output_process::client::Spawner>,
) -> Result<(), String> {
    use crate::output_process::client::Config;
    let route = host
        .try_state::<crate::output_process::OutputRoute>()
        .ok_or("This host does not route output to a process.")?;
    let diagnostic_host = host.clone();
    let events: crate::output_process::client::EventSink = Arc::new(move |event, payload| {
        let diagnostic = match event {
            "output:display-error" | "output:link-error" => Some((
                "output-link-error",
                orbit_host::diagnostics::LogLevel::Error,
            )),
            "output:link-changed" => {
                Some(("output-link-state", orbit_host::diagnostics::LogLevel::Info))
            }
            _ => None,
        };
        if let Some((event, level)) = diagnostic {
            orbit_host::record_diagnostic(
                &diagnostic_host,
                event,
                level,
                "output",
                Some(&payload.to_string()),
            );
        }
        let _ = send(
            &sender,
            ForegroundMessage::Event {
                event: event.to_string(),
                payload,
            },
        );
    });
    let started = route.start(|| {
        Ok(Config {
            directory: crate::output_process::protocol::data_directory(
                &host.path().app_data_dir()?,
            ),
            rendering_bit_depth,
            spawner: spawner(events.clone()),
            events: events.clone(),
        })
    })?;
    // The link may attach before it is stored; this tells the foreground to look again.
    if started {
        events(
            "output:link-changed",
            serde_json::json!({ "attached": false, "started": true }),
        );
    }
    Ok(())
}

#[cfg(any(windows, target_os = "macos"))]
fn system_tray(sender: UnboundedSender<ForegroundMessage>) -> Arc<dyn Tray> {
    crate::desktop::tray_icon_backend::MainThreadTray::new(sender)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn system_tray(_: UnboundedSender<ForegroundMessage>) -> Arc<dyn Tray> {
    Arc::new(crate::desktop::UnsupportedTray)
}
