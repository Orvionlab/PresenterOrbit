//! Showing, minimizing, and restoring the main window, including the tray rules of
//! `src-tauri/src/desktop_lifecycle.rs`.

use gpui::Window;

use crate::desktop::StartMode;
use crate::native_window::WindowControl;

/// Minimize, or hide to the tray when that preference is on.
pub fn minimize(
    window: &Window,
    control: &dyn WindowControl,
    minimize_to_tray: bool,
) -> Result<(), String> {
    if minimize_to_tray {
        control.hide(window)
    } else {
        control.minimize(window)
    }
}

/// Brings the window back from the taskbar, the tray, or a hidden state, and focuses it.
pub fn restore(window: &Window, control: &dyn WindowControl) -> Result<(), String> {
    control.restore(window)?;
    window.activate_window();
    Ok(())
}

/// The main window is created hidden so the saved preferences decide how it first appears.
pub fn apply_start_mode(
    window: &Window,
    control: &dyn WindowControl,
    mode: StartMode,
) -> Result<(), String> {
    match mode {
        StartMode::Shown => {
            window.activate_window();
            Ok(())
        }
        StartMode::Minimized => {
            window.activate_window();
            control.minimize(window)
        }
        StartMode::Tray => Ok(()),
    }
}

/// A user minimizing the window while the tray preference is on moves it to the tray.
pub fn hide_if_minimized_to_tray(
    window: &Window,
    control: &dyn WindowControl,
    minimize_to_tray: bool,
) -> Result<(), String> {
    if minimize_to_tray && control.is_minimized(window)? && control.is_visible(window)? {
        control.hide(window)?;
    }
    Ok(())
}
