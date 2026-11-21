use super::{deliver, Captured, Pending, Wanted};
use gtk::prelude::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
use wry::{WebViewBuilderExtUnix, WebViewExtUnix};
pub fn initialize() -> Result<(), String> {
    gtk::init().map_err(|error| error.to_string())
}
pub fn pump() {
    for _ in 0..256 {
        if !gtk::events_pending() {
            break;
        }
        gtk::main_iteration_do(false);
    }
}
pub struct Page {
    webview: wry::WebView,
    _parent: Parent,
    capturing: Arc<AtomicBool>,
}
impl Page {
    pub fn new(context: &mut wry::WebContext, wanted: &Wanted) -> Result<Self, String> {
        let window = Parent(gtk::Window::new(gtk::WindowType::Popup));
        window
            .0
            .set_default_size(wanted.width as i32, wanted.height as i32);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.0.add(&container);
        container.show();
        let webview = wry::WebViewBuilder::new_with_web_context(context)
            .with_url(&wanted.url)
            .with_transparent(true)
            .with_background_color((0, 0, 0, 0))
            .build_gtk(&container)
            .map_err(|error| error.to_string())?;
        webview.webview().show();
        window.0.realize();
        Ok(Self {
            webview,
            _parent: window,
            capturing: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn capture(&mut self, wanted: &Wanted, pending: &Pending) {
        if self.capturing.swap(true, Ordering::AcqRel) {
            return;
        }
        let webview = self.webview.webview();
        let scale = webview.scale_factor().max(1);
        webview.size_allocate(&gtk::Allocation::new(
            0,
            0,
            (wanted.width as f64 / scale as f64).round().max(1.) as i32,
            (wanted.height as f64 / scale as f64).round().max(1.) as i32,
        ));
        let (pending, key, done) = (pending.clone(), wanted.key.clone(), self.capturing.clone());
        webview.snapshot(
            SnapshotRegion::Visible,
            SnapshotOptions::NONE,
            None::<&webkit2gtk::gio::Cancellable>,
            move |result| {
                match result {
                    Ok(surface) => match cairo::ImageSurface::try_from(surface) {
                        Ok(surface) => {
                            let (width, height, stride) = (
                                surface.width() as u32,
                                surface.height() as u32,
                                surface.stride() as usize,
                            );
                            if let Err(error) = surface.with_data(|bytes| {
                                deliver(
                                    &pending,
                                    &key,
                                    Captured::Bgra {
                                        width,
                                        height,
                                        stride,
                                        bytes: bytes.to_vec(),
                                    },
                                )
                            }) {
                                deliver(&pending, &key, Captured::Error(error.to_string()));
                            }
                        }
                        Err(_) => deliver(
                            &pending,
                            &key,
                            Captured::Error("The browser snapshot is not an image surface.".into()),
                        ),
                    },
                    Err(error) => deliver(&pending, &key, Captured::Error(error.to_string())),
                }
                done.store(false, Ordering::Release);
            },
        );
    }
}
struct Parent(gtk::Window);
impl Drop for Parent {
    fn drop(&mut self) {
        self.0.close();
    }
}
