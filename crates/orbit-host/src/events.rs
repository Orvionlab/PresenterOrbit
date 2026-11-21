use serde_json::Value;

/// Delivers host events to whichever shell runs the host.
pub trait HostEvents: Send + Sync + 'static {
    /// Sends an event to every listener.
    fn emit(&self, event: &str, payload: Value) -> Result<(), String>;

    /// Sends an event to the listener registered under `label`, such as the main window.
    fn emit_to(&self, label: &str, event: &str, payload: Value) -> Result<(), String>;
}
