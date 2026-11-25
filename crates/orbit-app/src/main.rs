// The default text style in canvas/editing.rs is one json! literal, which exceeds the default limit.
#![recursion_limit = "256"]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod appearance;
mod automation;
mod auxiliary;
mod bibles;
mod canvas;
mod carry_over;
mod charts;
mod collections;
mod cues;
mod desktop;
#[cfg(all(debug_assertions, feature = "dev-hot-reload"))]
mod dev_hot_reload;
mod external;
mod foreground;
mod gap;
mod imports;
mod keys;
mod layout;
mod locked;
mod main_window;
mod mcp;
mod media;
mod native_outputs;
mod native_window;
mod output;
mod output_process;
mod panic_hook;
mod services;
mod settings;
mod shell;

mod showcase;
mod startup;
mod state;

mod theme_picker;
mod tickers;
mod tokens;
mod transfers;
mod transitions;
mod updates;
mod view;
mod windows;
mod workspaces;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use futures::{channel::mpsc::unbounded, StreamExt};
use gpui::{
    point, px, size, App, AppContext, Bounds, Context, Entity, Render, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions,
};
use orbit_host::Host;
use orbit_ui::controls::Root;
use orbit_ui::{init_theme, AppearanceSettings, Assets, Translator};

use auxiliary::AuxiliaryWindows;
use foreground::MainWindow;
use native_window::PlatformControl;
use shell::ForegroundMessage;
use showcase::ShowcaseView;
use tokens::TokenView;
use view::{AppView, ShellServices};

/// The Tauri app's initial and minimum main window size, in logical pixels.
const MAIN_SIZE: (f32, f32) = (1440., 920.);
const MAIN_MINIMUM_SIZE: (f32, f32) = (1120., 720.);

#[derive(Debug, Default, PartialEq)]
struct Arguments {
    workspace: Option<PathBuf>,
    tokens: bool,
    showcase: bool,
    data_dir: Option<PathBuf>,
}

/// `--workspace <path>` opens that workspace instead of the one in settings. `--data-dir <path>` keeps settings and
/// app data in that folder instead of the user's data folder (and skips the settings import). `--tokens` also
/// opens the token screen.
fn parse_arguments(mut arguments: impl Iterator<Item = String>) -> Result<Arguments, String> {
    let mut parsed = Arguments::default();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--workspace" => {
                parsed.workspace = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--workspace requires a path.".to_string())?,
                ))
            }
            "--data-dir" => {
                parsed.data_dir = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--data-dir requires a path.".to_string())?,
                ))
            }
            "--tokens" => parsed.tokens = true,
            "--showcase" => parsed.showcase = true,
            other => return Err(format!("Unknown argument '{other}'.")),
        }
    }
    Ok(parsed)
}

fn startup_arguments() -> Result<Arguments, String> {
    let arguments = std::env::args().skip(1);
    // dx 0.7 splits --args on whitespace, including quoted Windows paths.
    #[cfg(all(debug_assertions, feature = "dev-hot-reload"))]
    let arguments = {
        let extra = match std::env::var("PRESENTER_ORBIT_HOT_ARGS") {
            Ok(json) => dev_hot_reload::arguments(&json)?,
            Err(std::env::VarError::NotPresent) => Vec::new(),
            Err(error) => return Err(format!("Invalid PRESENTER_ORBIT_HOT_ARGS: {error}")),
        };
        arguments.chain(extra)
    };
    parse_arguments(arguments)
}

fn open_window<V: Render + 'static>(
    cx: &mut App,
    title: &'static str,
    offset: f32,
    build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
) -> WindowHandle<V> {
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: bounds.origin + point(px(offset), px(offset)),
            ..bounds
        })),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| cx.new(|cx| build(window, cx)))
        .expect("a window opens")
}

/// A window under `gpui-base`'s root, which binds Tab traversal.
fn open_rooted_window<V: Render + 'static>(
    cx: &mut App,
    title: &'static str,
    offset: f32,
    build: impl FnOnce(&mut Window, &mut Context<V>) -> V + 'static,
) {
    open_window(cx, title, offset, |window, cx: &mut Context<Root>| {
        let view = cx.new(|cx| build(window, cx));
        Root::new(view, window, cx)
    });
}

/// The main window is created hidden: the saved preferences decide whether it shows, minimizes, or goes to the
/// tray, and the first frame must not appear before that is known.
pub fn open_main_window(
    cx: &mut App,
    host: Host,
    workspace: Option<PathBuf>,
    services: ShellServices,
) -> Result<MainWindow, String> {
    open_main_window_sized(cx, host, workspace, services, MAIN_SIZE)
}

pub fn open_main_window_sized(
    cx: &mut App,
    host: Host,
    workspace: Option<PathBuf>,
    services: ShellServices,
    (width, height): (f32, f32),
) -> Result<MainWindow, String> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(width), px(height)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Presenter Orbit".into()),
            ..Default::default()
        }),
        show: false,
        focus: false,
        window_min_size: Some(size(px(MAIN_MINIMUM_SIZE.0), px(MAIN_MINIMUM_SIZE.1))),
        ..Default::default()
    };
    let built: Rc<RefCell<Option<Entity<AppView>>>> = Rc::default();
    let capture = built.clone();
    let window = cx
        .open_window(options, move |window, cx| {
            let view = cx.new(|cx| AppView::new(host, workspace, services, window, cx));
            *capture.borrow_mut() = Some(view.clone());
            cx.new(|cx| Root::new(view, window, cx))
        })
        .map_err(|error| error.to_string())?;
    let view = built
        .borrow_mut()
        .take()
        .ok_or_else(|| "The main window did not build its view.".to_string())?;
    Ok(MainWindow {
        window: window.into(),
        view,
    })
}

/// The bundled sample themes and stage layouts: `resources/Sample Content` next to the executable, the layout the
/// Tauri bundle installs. Only a development build may use the source tree instead.
fn sample_content_root(executable: &std::path::Path) -> Result<PathBuf, String> {
    // Next to the executable; inside a macOS bundle, in `Contents/Resources`.
    let installed = executable
        .parent()
        .into_iter()
        .flat_map(|folder| {
            [
                folder.join("resources").join("Sample Content"),
                folder.join("../Resources/resources/Sample Content"),
            ]
        })
        .find(|folder| folder.is_dir());
    if let Some(folder) = installed {
        return Ok(folder);
    }
    if cfg!(debug_assertions) {
        return Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/Sample Content"));
    }
    Err("The bundled Sample Content folder is missing next to the executable.".into())
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--mcp") {
        if let Err(error) = mcp::run() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    orbit_host::initialize_sample_content(
        std::env::current_exe()
            .map_err(|error| error.to_string())
            .and_then(|executable| sample_content_root(&executable))
            .expect("Sample content location failed"),
    )
    .expect("Sample content initialization failed");
    if native_outputs::run_capture_helper() {
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--output-process") {
        std::process::exit(output_process::run_process(std::env::args().skip(2)));
    }
    #[cfg(all(feature = "web-fill-acceptance", not(test)))]
    if std::env::args().nth(1).as_deref() == Some("--web-fill-acceptance") {
        native_outputs::acceptance::run();
        return;
    }
    let arguments = match startup_arguments() {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let (sender, mut receiver) = unbounded::<ForegroundMessage>();
    let import = if arguments.data_dir.is_none() {
        carry_over::import_for_first_start()
    } else {
        Ok(false)
    };
    let (host, host_services) =
        shell::build_controller_host(sender.clone(), arguments.data_dir.clone());
    match import {
        Ok(true) => orbit_host::record_diagnostic(
            &host,
            "settings-imported",
            orbit_host::diagnostics::LogLevel::Info,
            "startup",
            Some("Imported the settings of the Tauri version."),
        ),
        Ok(false) => {}
        Err(error) => orbit_host::record_diagnostic(
            &host,
            "settings-import-failed",
            orbit_host::diagnostics::LogLevel::Error,
            "startup",
            Some(&error),
        ),
    }
    panic_hook::install(host.clone());
    let shutdown_host = host.clone();
    let shutdown_outputs = host_services.outputs.clone();

    gpui_platform::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            foreground::stop_output_on_quit(cx, host.clone());
            cx.set_global(Translator::new("en"));
            init_theme(cx, AppearanceSettings::default(), cx.window_appearance());
            let control: Rc<dyn native_window::WindowControl> = Rc::new(PlatformControl::new(cx));
            AuxiliaryWindows::install(
                cx,
                host.clone(),
                host_services.registry.clone(),
                control.clone(),
            );
            let services = ShellServices {
                control,
                minimize_to_tray: host_services.minimize_to_tray.clone(),
            };
            let startup_host = host.clone();
            let update_host = host.clone();
            let link_sender = sender.clone();
            let main = open_main_window(cx, host, arguments.workspace, services)
                .expect("the main window opens");
            let view = main.view.downgrade();
            cx.spawn(async move |cx| {
                let answer = cx
                    .background_spawn(async move {
                        let general = orbit_host::start_host(&startup_host)?;
                        orbit_host::start_tickers(&startup_host)
                            .map_err(|error| error.to_string())?;
                        let depth = general["renderingBitDepth"]
                            .as_str()
                            .ok_or("The general settings have no rendering bit depth.")?;
                        shell::start_output_link(&startup_host, link_sender, depth.to_string())?;
                        Ok::<_, String>(updates::startup_check_enabled(&general))
                    })
                    .await;
                match answer {
                    Err(error) => {
                        let _ = view.update(cx, |view, cx| view.fail_host_startup(error, cx));
                    }
                    Ok(true) => {
                        let host = update_host;
                        let status = cx
                            .background_spawn(async move { host.updater().check(&host, true) })
                            .await;
                        match status {
                            Ok(status) => {
                                let _ = view.update(cx, |view, cx| view.offer_update(status, cx));
                            }
                            Err(error) => eprintln!("[app-updates] {error}"),
                        }
                    }
                    Ok(false) => {}
                }
            })
            .detach();
            if let Err(error) = AuxiliaryWindows::prepare(cx) {
                eprintln!("[auxiliary-windows] {error}");
            }
            if arguments.tokens {
                open_window(cx, "Presenter Orbit tokens", 40.0, TokenView::new);
            }
            if arguments.showcase {
                open_rooted_window(cx, "Presenter Orbit showcase", 80.0, ShowcaseView::new);
            }
            let main_id = main.window.window_id();
            cx.on_window_closed(move |cx, closed| {
                foreground::handle_window_closed(cx, main_id, closed)
            })
            .detach();
            cx.activate(true);
            cx.spawn(async move |cx| {
                while let Some(message) = receiver.next().await {
                    cx.update(|cx| foreground::handle_message(cx, &main, message));
                }
            })
            .detach();
        });
    orbit_host::stop_host(&shutdown_host);
    shutdown_outputs.shutdown();
}
