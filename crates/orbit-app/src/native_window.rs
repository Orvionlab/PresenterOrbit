//! GPUI creates windows and can minimize, maximize, and activate them, but it has no call to hide or show one,
//! which the auxiliary windows and minimize to tray need. This is that call, on the platform handle.

use gpui::Window;

pub trait WindowControl {
    fn show(&self, window: &Window) -> Result<(), String>;
    fn hide(&self, window: &Window) -> Result<(), String>;
    fn minimize(&self, window: &Window) -> Result<(), String>;
    fn maximize(&self, window: &Window) -> Result<(), String>;
    fn restore(&self, window: &Window) -> Result<(), String>;
    fn is_visible(&self, window: &Window) -> Result<bool, String>;
    fn is_minimized(&self, window: &Window) -> Result<bool, String>;
}

pub struct PlatformControl {
    #[cfg(target_os = "macos")]
    foreground: gpui::ForegroundExecutor,
}

impl PlatformControl {
    pub fn new(_cx: &gpui::App) -> Self {
        Self {
            #[cfg(target_os = "macos")]
            foreground: _cx.foreground_executor().clone(),
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::{PlatformControl, WindowControl};

    const SW_HIDE: i32 = 0;
    const SW_SHOWNORMAL: i32 = 1;
    const SW_SHOWMINIMIZED: i32 = 2;
    const SW_SHOWMAXIMIZED: i32 = 3;
    const SW_RESTORE: i32 = 9;

    #[link(name = "user32")]
    extern "system" {
        fn ShowWindowAsync(hwnd: *mut c_void, command: i32) -> i32;
        fn IsWindowVisible(hwnd: *mut c_void) -> i32;
        fn IsIconic(hwnd: *mut c_void) -> i32;
    }

    fn handle(window: &Window) -> Result<*mut c_void, String> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| format!("The window handle is unavailable: {error}"))?;
        match handle.as_raw() {
            RawWindowHandle::Win32(win32) => Ok(win32.hwnd.get() as *mut c_void),
            _ => Err("The window is not a Win32 window.".to_string()),
        }
    }

    // Asynchronous, like GPUI's own calls: a synchronous one re-enters the window procedure while the window is
    // borrowed by the caller.
    fn show_with(window: &Window, command: i32) -> Result<(), String> {
        let hwnd = handle(window)?;
        // SAFETY: `hwnd` belongs to a live GPUI window that the caller holds, and the call has no other input.
        unsafe { ShowWindowAsync(hwnd, command) };
        Ok(())
    }

    impl WindowControl for PlatformControl {
        fn show(&self, window: &Window) -> Result<(), String> {
            show_with(window, SW_SHOWNORMAL)
        }
        fn hide(&self, window: &Window) -> Result<(), String> {
            show_with(window, SW_HIDE)
        }
        fn minimize(&self, window: &Window) -> Result<(), String> {
            show_with(window, SW_SHOWMINIMIZED)
        }
        fn maximize(&self, window: &Window) -> Result<(), String> {
            show_with(window, SW_SHOWMAXIMIZED)
        }
        fn restore(&self, window: &Window) -> Result<(), String> {
            show_with(window, SW_RESTORE)
        }
        fn is_visible(&self, window: &Window) -> Result<bool, String> {
            let hwnd = handle(window)?;
            // SAFETY: as in `show_with`.
            Ok(unsafe { IsWindowVisible(hwnd) } != 0)
        }
        fn is_minimized(&self, window: &Window) -> Result<bool, String> {
            let hwnd = handle(window)?;
            // SAFETY: as in `show_with`.
            Ok(unsafe { IsIconic(hwnd) } != 0)
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use gpui::Window;
    use objc2::rc::{Retained, Weak};
    use objc2_app_kit::{NSView, NSWindow};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::{PlatformControl, WindowControl};

    fn native_window(window: &Window) -> Result<Retained<NSWindow>, String> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| format!("The window handle is unavailable: {error}"))?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("The window is not an AppKit window.".to_string());
        };
        // SAFETY: GPUI provides a live NSView for the borrowed window on the main thread.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        view.window()
            .ok_or_else(|| "The AppKit view has no window.".to_string())
    }

    impl PlatformControl {
        fn schedule(
            &self,
            window: &Window,
            action: impl FnOnce(&NSWindow) + 'static,
        ) -> Result<(), String> {
            let native = Weak::from_retained(&native_window(window)?);
            // AppKit can call GPUI delegates during these operations. Run them after GPUI
            // has returned the borrowed app and window, and skip windows destroyed in between.
            self.foreground
                .spawn(async move {
                    if let Some(native) = native.load() {
                        action(&native);
                    }
                })
                .detach();
            Ok(())
        }
    }

    impl WindowControl for PlatformControl {
        fn show(&self, window: &Window) -> Result<(), String> {
            self.schedule(window, |native| {
                if native.isMiniaturized() {
                    native.deminiaturize(None);
                }
                native.orderFront(None);
            })
        }
        fn hide(&self, window: &Window) -> Result<(), String> {
            self.schedule(window, |native| native.orderOut(None))
        }
        fn minimize(&self, window: &Window) -> Result<(), String> {
            self.schedule(window, |native| native.miniaturize(None))
        }
        fn maximize(&self, window: &Window) -> Result<(), String> {
            self.schedule(window, |native| {
                if !native.isZoomed() {
                    native.zoom(None);
                }
            })
        }
        fn restore(&self, window: &Window) -> Result<(), String> {
            self.show(window)
        }
        fn is_visible(&self, window: &Window) -> Result<bool, String> {
            Ok(native_window(window)?.isVisible())
        }
        fn is_minimized(&self, window: &Window) -> Result<bool, String> {
            Ok(native_window(window)?.isMiniaturized())
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use gpui::Window;

    use super::{PlatformControl, WindowControl};

    fn unsupported<T>() -> Result<T, String> {
        Err("Native window controls are unavailable on this platform.".to_string())
    }

    impl WindowControl for PlatformControl {
        fn show(&self, _: &Window) -> Result<(), String> {
            unsupported()
        }
        fn hide(&self, _: &Window) -> Result<(), String> {
            unsupported()
        }
        fn minimize(&self, _: &Window) -> Result<(), String> {
            unsupported()
        }
        fn maximize(&self, _: &Window) -> Result<(), String> {
            unsupported()
        }
        fn restore(&self, _: &Window) -> Result<(), String> {
            unsupported()
        }
        fn is_visible(&self, _: &Window) -> Result<bool, String> {
            unsupported()
        }
        fn is_minimized(&self, _: &Window) -> Result<bool, String> {
            unsupported()
        }
    }
}
