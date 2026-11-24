//! Carries out what the host asked for from its own threads: events, tasks, and window changes.

use gpui::{AnyWindowHandle, App, Entity, WindowId};
use serde_json::Value;

use crate::appearance;
use crate::auxiliary::AuxiliaryWindows;
use crate::output_process::OutputRoute;
use crate::shell::{ForegroundMessage, WindowCommand};
use crate::view::{AppView, HostEvent};

/// The main window and its view, which window commands and events address.
#[derive(Clone)]
pub struct MainWindow {
    pub window: AnyWindowHandle,
    pub view: Entity<AppView>,
}

pub fn stop_output_on_quit(cx: &App, host: orbit_host::Host) {
    cx.on_app_quit(move |_| {
        // AppKit exits without returning from the event loop. Finish shutting down the
        // output process inside the quit callback so a normal quit does not orphan it.
        if let Some(link) = host
            .try_state::<OutputRoute>()
            .and_then(|route| route.0.get())
        {
            link.shutdown();
        }
        std::future::ready(())
    })
    .detach();
}

pub fn handle_message(cx: &mut App, main: &MainWindow, message: ForegroundMessage) {
    match message {
        ForegroundMessage::Task(task) => task(),
        ForegroundMessage::Event { event, payload } => handle_event(cx, main, event, payload),
        ForegroundMessage::Windows(command) => handle_window_command(cx, main, command),
    }
}

fn handle_event(cx: &mut App, main: &MainWindow, event: String, payload: Value) {
    let appearance_payload =
        (event == "settings-changed").then(|| main.view.read(cx).settings_preview(&payload, cx));
    if let Err(error) =
        appearance::apply_host_event(cx, &event, appearance_payload.as_ref().unwrap_or(&payload))
    {
        eprintln!("Presenter Orbit ignored the '{event}' event: {error}");
    }
    main.view.update(cx, |view, cx| {
        view.on_host_event(&event, &payload, cx);
        cx.emit(HostEvent { event, payload });
    });
}

fn handle_window_command(cx: &mut App, main: &MainWindow, command: WindowCommand) {
    let result = match command {
        WindowCommand::Prepare => AuxiliaryWindows::prepare(cx),
        WindowCommand::Show { kind, reply } => {
            let shown = AuxiliaryWindows::show(cx, kind);
            // The asking host call may have given up; its answer is then no longer needed.
            let _ = reply.send(shown.clone());
            shown
        }
        WindowCommand::Hide(kind) => AuxiliaryWindows::hide(cx, kind),
        WindowCommand::MinimizeMain => with_main(cx, main, |view, window| view.minimize(window)),
        WindowCommand::RestoreMain => with_main(cx, main, |view, window| view.restore(window)),
        WindowCommand::RestoreIfHidden => {
            with_main(cx, main, |view, window| view.restore_if_hidden(window))
        }
        WindowCommand::Quit => {
            cx.quit();
            Ok(())
        }
    };
    if let Err(error) = result {
        eprintln!("[windows] {error}");
    }
}

fn with_main(
    cx: &mut App,
    main: &MainWindow,
    action: impl FnOnce(&AppView, &gpui::Window),
) -> Result<(), String> {
    let view = main.view.clone();
    main.window
        .update(cx, |_, window, cx| action(view.read(cx), window))
        .map_err(|error| error.to_string())
}

/// Closing the main window destroys the auxiliary windows, which would otherwise keep the application alive
/// hidden. The application ends when no window is left.
pub fn handle_window_closed(cx: &mut App, main: WindowId, closed: WindowId) {
    if closed == main {
        AuxiliaryWindows::destroy_all(cx);
    }
    if cx.windows().is_empty() {
        cx.quit();
    }
}
