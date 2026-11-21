use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicalPosition<T> {
    pub x: T,
    pub y: T,
}

impl<T> PhysicalPosition<T> {
    pub fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicalSize<T> {
    pub width: T,
    pub height: T,
}

impl<T> PhysicalSize<T> {
    pub fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl DisplayBounds {
    pub fn from_monitor(monitor: &Monitor) -> Self {
        let position = monitor.position();
        let size = monitor.size();
        Self {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Monitor {
    pub name: Option<String>,
    pub position: PhysicalPosition<i32>,
    pub size: PhysicalSize<u32>,
    pub scale_factor: f64,
}

impl Monitor {
    pub fn name(&self) -> Option<&String> {
        self.name.as_ref()
    }

    pub fn position(&self) -> &PhysicalPosition<i32> {
        &self.position
    }

    pub fn size(&self) -> &PhysicalSize<u32> {
        &self.size
    }

    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }
}

/// Lets a window event handler veto the close.
#[derive(Default)]
pub struct CloseRequest(AtomicBool);

impl CloseRequest {
    pub fn prevent_close(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_prevented(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub enum WindowEvent {
    Resized(PhysicalSize<u32>),
    ScaleFactorChanged { new_inner_size: PhysicalSize<u32> },
    CloseRequested { api: CloseRequest },
    Destroyed,
}

pub type WindowEventHandler = Box<dyn Fn(&WindowEvent) + Send + 'static>;

/// What the host needs from one native window. A window system implements it.
pub trait WindowBackend: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static {
    fn label(&self) -> String;
    fn show(&self) -> Result<(), String>;
    fn hide(&self) -> Result<(), String>;
    fn close(&self) -> Result<(), String>;
    fn set_focus(&self) -> Result<(), String>;
    fn maximize(&self) -> Result<(), String>;
    fn unmaximize(&self) -> Result<(), String>;
    fn is_visible(&self) -> Result<bool, String>;
    fn set_title(&self, title: &str) -> Result<(), String>;
    fn set_position(&self, position: PhysicalPosition<i32>) -> Result<(), String>;
    fn set_size(&self, size: PhysicalSize<u32>) -> Result<(), String>;
    fn set_always_on_top(&self, always_on_top: bool) -> Result<(), String>;
    fn inner_size(&self) -> Result<PhysicalSize<u32>, String>;
    fn outer_size(&self) -> Result<PhysicalSize<u32>, String>;
    /// The HWND on Windows and the NSView on macOS.
    fn native_window_pointer(&self) -> Result<*mut std::ffi::c_void, String>;
    /// Creates the native handle that GPU surfaces need, on platforms where hidden windows lack one.
    fn realize(&self) -> Result<(), String>;
    fn on_window_event(&self, handler: WindowEventHandler);
}

/// A shared handle to a window owned by the shell.
#[derive(Clone)]
pub struct Window(Arc<dyn WindowBackend>);

impl Window {
    pub fn new(backend: Arc<dyn WindowBackend>) -> Self {
        Self(backend)
    }
}

impl Deref for Window {
    type Target = dyn WindowBackend;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.0.display_handle()
    }
}
