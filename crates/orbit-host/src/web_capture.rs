//! Live web page fills for native output.
//!
//! The shell that owns a browser engine installs a [`WebCapture`]. Compositor
//! threads only read the latest frame it has produced.

use image::DynamicImage;
use std::sync::{Arc, OnceLock};

/// The largest page raster a web fill renders at, in physical pixels.
pub const MAX_SIZE: (u32, u32) = (1920, 1080);

pub trait WebCapture: Send + Sync + 'static {
    /// Returns the latest capture of `url` at `size`, starting the page when it is not
    /// loaded yet. `Ok(None)` means the page has not produced a frame yet.
    fn frame(&self, url: &url::Url, size: (u32, u32)) -> Result<Option<Arc<DynamicImage>>, String>;
}

static CAPTURE: OnceLock<Arc<dyn WebCapture>> = OnceLock::new();

pub fn install(capture: Arc<dyn WebCapture>) {
    let _ = CAPTURE.set(capture);
}

/// Validates a web fill address: only absolute http(s) pages are loaded.
pub fn page_url(value: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(value.trim())
        .map_err(|_| format!("Web fill address {value:?} is not a URL."))?;
    match url.scheme() {
        "http" | "https" if url.host_str().is_some() => Ok(url),
        _ => Err(format!(
            "Web fill address {value:?} must be an http or https page."
        )),
    }
}

/// Returns the latest capture of `url` at `width`×`height`, starting the page when it
/// is not loaded yet. `Ok(None)` means the page has not produced a frame yet.
pub fn frame(url: &str, width: u32, height: u32) -> Result<Option<Arc<DynamicImage>>, String> {
    let url = page_url(url)?;
    let Some(capture) = CAPTURE.get() else {
        return Err("Web fills need the desktop host.".into());
    };
    let size = (width.clamp(1, MAX_SIZE.0), height.clamp(1, MAX_SIZE.1));
    capture.frame(&url, size)
}
