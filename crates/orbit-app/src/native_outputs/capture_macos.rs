use super::{deliver, Captured, Pending, Wanted};
use objc2::{rc::Retained, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEventMask, NSImage,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSDate, NSDefaultRunLoopMode, NSError, NSPoint, NSRect, NSSize,
};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
use raw_window_handle::{
    AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use wry::WebViewExtMacOS;
pub fn initialize() -> Result<(), String> {
    let mtm =
        MainThreadMarker::new().ok_or("The capture helper must run on the AppKit main thread.")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.finishLaunching();
    Ok(())
}
pub fn pump() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    for _ in 0..256 {
        let event = unsafe {
            app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&NSDate::distantPast()),
                NSDefaultRunLoopMode,
                true,
            )
        };
        let Some(event) = event else {
            break;
        };
        app.sendEvent(&event);
    }
    app.updateWindows();
}
struct Parent(Retained<NSWindow>);
impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let view = self.0.contentView().ok_or(HandleError::Unavailable)?;
        let ptr = std::ptr::NonNull::new(Retained::as_ptr(&view) as *mut std::ffi::c_void)
            .ok_or(HandleError::Unavailable)?;
        Ok(unsafe {
            WindowHandle::borrow_raw(RawWindowHandle::AppKit(AppKitWindowHandle::new(ptr)))
        })
    }
}
impl Drop for Parent {
    fn drop(&mut self) {
        self.0.close();
    }
}
pub struct Page {
    webview: wry::WebView,
    _parent: Parent,
    capturing: Arc<AtomicBool>,
}
impl Page {
    pub fn new(context: &mut wry::WebContext, wanted: &Wanted) -> Result<Self, String> {
        let mtm = MainThreadMarker::new()
            .ok_or("The capture helper must run on the AppKit main thread.")?;
        let parent = Parent(unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(
                    NSPoint::new(0., 0.),
                    NSSize::new(wanted.width as f64, wanted.height as f64),
                ),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        });
        unsafe {
            parent.0.setReleasedWhenClosed(false);
        }
        let scale = parent.0.backingScaleFactor();
        parent.0.setContentSize(NSSize::new(
            wanted.width as f64 / scale,
            wanted.height as f64 / scale,
        ));
        let webview = wry::WebViewBuilder::new_with_web_context(context)
            .with_url(&wanted.url)
            .with_transparent(true)
            .with_background_color((0, 0, 0, 0))
            .with_bounds(wry::Rect {
                position: wry::dpi::LogicalPosition::new(0., 0.).into(),
                size: wry::dpi::LogicalSize::new(
                    wanted.width as f64 / scale,
                    wanted.height as f64 / scale,
                )
                .into(),
            })
            .build_as_child(&parent)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            webview,
            _parent: parent,
            capturing: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn capture(&mut self, wanted: &Wanted, pending: &Pending) {
        if self.capturing.swap(true, Ordering::AcqRel) {
            return;
        }
        let (pending, key, done) = (pending.clone(), wanted.key.clone(), self.capturing.clone());
        let mtm = MainThreadMarker::new().expect("the browser helper owns AppKit");
        let configuration = WKSnapshotConfiguration::new(mtm);
        unsafe {
            configuration.setAfterScreenUpdates(false);
        }
        let handler = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            if let Some(tiff) = unsafe { image.as_ref() }.and_then(NSImage::TIFFRepresentation) {
                deliver(&pending, &key, Captured::Encoded(tiff.to_vec(), true));
            } else {
                deliver(
                    &pending,
                    &key,
                    Captured::Error(
                        unsafe { error.as_ref() }
                            .map(|error| error.to_string())
                            .unwrap_or_else(|| "The browser returned no snapshot.".into()),
                    ),
                );
            }
            done.store(false, Ordering::Release);
        });
        let view = self.webview.webview();
        let view: &WKWebView = &view;
        unsafe {
            view.takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &handler);
        }
    }
}
