use serde_json::Value;

use crate::{DisplayBounds, Host, Monitor, PhysicalPosition, Window};

/// Window creation, lookup, and the main-thread hop, provided by the shell.
pub trait WindowSystem: Send + Sync + 'static {
    fn get_window(&self, label: &str) -> Option<Window>;
    fn available_monitors(&self) -> Result<Vec<Monitor>, String>;
    fn primary_monitor(&self) -> Result<Option<Monitor>, String>;
    fn cursor_position(&self) -> Result<PhysicalPosition<f64>, String>;
    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send + 'static>) -> Result<(), String>;
    /// Dedicated native window loops do not need a hop through the application UI.
    fn output_windows_use_main_thread(&self) -> bool {
        true
    }
    /// A resizable output window. Uses the main thread when `output_windows_use_main_thread` is true.
    fn create_windowed_output_window(&self, label: &str, title: &str) -> Result<Window, String>;
    /// A borderless, always-on-top window covering `bounds`, on the shell's output window thread.
    fn create_display_output_window(
        &self,
        label: &str,
        title: &str,
        bounds: DisplayBounds,
    ) -> Result<Window, String>;
    /// The small overlay control that opens above a display output.
    fn create_output_control_window(
        &self,
        screen_id: &str,
        label: &str,
        owner_label: &str,
        bounds: DisplayBounds,
    ) -> Result<Window, String>;
    fn prepare_auxiliary_windows(&self) -> Result<Value, String>;
    fn open_auxiliary_window(&self, request: Value) -> Result<Value, String>;
    fn close_auxiliary_window(&self, label: &Value) -> Result<Value, String>;
    fn minimize_main_window(&self) -> Result<(), String>;
}

/// Tray and login-item behavior behind the general settings.
pub trait DesktopIntegration: Send + Sync + 'static {
    fn apply_preferences(&self, previous: &Value, next: &Value) -> Result<(), String>;
}

/// Application updates behind the update bridge methods.
pub trait Updater: Send + Sync + 'static {
    fn status(&self) -> Value;
    fn check(&self, host: &Host, startup_check: bool) -> Result<Value, String>;
    fn install(&self, host: &Host) -> Result<Value, String>;
}
