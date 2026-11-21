//! Native output windows run independently of the GPUI foreground.

#[cfg(all(not(test), any(windows, target_os = "macos", target_os = "linux")))]
fn window_controls_visible(active: bool, hovered: bool) -> bool {
    active && hovered
}

#[cfg(all(not(test), windows))]
mod logic;
#[cfg(all(windows, not(test)))]
mod web_capture;
#[cfg(all(windows, not(test)))]
mod win32;
#[cfg(all(windows, not(test)))]
mod win32_chrome;
#[cfg(all(windows, not(test)))]
pub use win32::NativeOutputs;

#[cfg(all(target_os = "linux", not(test)))]
mod linux;
#[cfg(all(target_os = "linux", not(test)))]
pub use linux::NativeOutputs;
#[cfg(all(target_os = "macos", not(test)))]
pub(super) mod macos;
#[cfg(all(target_os = "macos", not(test)))]
pub use macos::NativeOutputs;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod capture_process;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod capture_protocol;
pub mod framing;

#[cfg(target_os = "macos")]
pub(crate) use capture_process::{initialize_app, pump_app};

pub fn run_capture_helper() -> bool {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if std::env::args().nth(1).as_deref() == Some("--web-fill-helper") {
        if let Err(error) = capture_process::run_helper() {
            eprintln!("[web-fill-helper] {error}");
            std::process::exit(1);
        }
        return true;
    }
    false
}

#[cfg(all(feature = "web-fill-acceptance", not(test)))]
pub mod acceptance;

#[cfg(any(test, not(any(windows, target_os = "macos", target_os = "linux"))))]
mod unsupported {
    use orbit_host::{DisplayBounds, Host, Monitor, PhysicalPosition, Window};

    #[derive(Default)]
    pub struct NativeOutputs(());

    fn unavailable<T>() -> Result<T, String> {
        #[cfg(test)]
        let message = "Native output windows are disabled in the headless test harness.";
        #[cfg(not(test))]
        let message = "The GPUI native output adapter is not implemented on this platform.";
        Err(message.into())
    }

    impl NativeOutputs {
        pub fn get(&self, _: &str) -> Option<Window> {
            None
        }
        pub fn monitors(&self, _: bool) -> Result<Vec<Monitor>, String> {
            #[cfg(test)]
            {
                Ok(Vec::new())
            }
            #[cfg(not(test))]
            {
                unavailable()
            }
        }
        pub fn cursor(&self) -> Result<PhysicalPosition<f64>, String> {
            unavailable()
        }
        pub fn create(
            &self,
            _: &str,
            _: &str,
            _: Option<DisplayBounds>,
            _: Option<(&str, &str)>,
        ) -> Result<Window, String> {
            unavailable()
        }
        pub fn attach(&self, _: Host) {}
        pub fn shutdown(&self) {}
    }
}

#[cfg(any(test, not(any(windows, target_os = "macos", target_os = "linux"))))]
pub use unsupported::NativeOutputs;
