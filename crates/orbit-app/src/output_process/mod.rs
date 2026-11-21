//! Program output in its own process, so a crash of the UI process does not take output off air.
//! `protocol` is the wire format, `server` runs in the output process (`orbit-app --output-process`), and
//! `client` is the UI side link that starts, attaches to, and supervises it.

pub mod client;

pub mod protocol;

/// The link to the output process, kept as host state. Empty until the host has started; a host without this
/// state keeps its output in its own process (the tests).
#[derive(Default)]
pub struct OutputRoute(pub std::sync::OnceLock<std::sync::Arc<client::OutputLink>>);
pub mod server;

/// Entry of `orbit-app --output-process <arguments>`. The returned code is the process exit code.
pub fn run_process(arguments: impl Iterator<Item = String>) -> i32 {
    let options = match server::parse_options(arguments) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("[output-process] {error}");
            return 2;
        }
    };
    #[cfg(windows)]
    // The output windows are created on a thread of this process, and nothing else sets its awareness.
    unsafe {
        use windows::Win32::UI::HiDpi::{
            AreDpiAwarenessContextsEqual, GetThreadDpiAwarenessContext,
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        let wanted = DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2;
        // Setting fails with access denied once awareness is fixed, for example by the executable manifest.
        if SetProcessDpiAwarenessContext(wanted).is_err()
            && !AreDpiAwarenessContextsEqual(GetThreadDpiAwarenessContext(), wanted).as_bool()
        {
            eprintln!("[output-process] The process is not per-monitor DPI aware (v2).");
            return 1;
        }
    }
    #[cfg(target_os = "macos")]
    if let Err(error) = crate::native_outputs::initialize_app() {
        eprintln!("[output-process] {error}");
        return 1;
    }
    server::run(options)
}
