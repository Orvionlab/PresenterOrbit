//! Records every panic in the host diagnostics log and on stderr before the default behavior runs.
//! The output process installs this hook too, so a panic there is recorded as well. A panic in the UI process
//! no longer ends program output.

use orbit_host::{diagnostics::LogLevel, Host};
use std::panic::{self, PanicHookInfo};

fn describe(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-text panic payload");
    let location = info
        .location()
        .map_or_else(|| "unknown location".to_string(), ToString::to_string);
    let thread = std::thread::current();
    format!(
        "thread '{}' panicked at {location}: {message}",
        thread.name().unwrap_or("<unnamed>")
    )
}

pub fn install(host: Host) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let message = describe(info);
        eprintln!("[panic] {message}");
        orbit_host::record_diagnostic(&host, "panic", LogLevel::Error, "app", Some(&message));
        previous(info);
    }));
}
