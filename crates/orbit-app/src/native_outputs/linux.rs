//! X11/XWayland outputs have their own event loop; WebKitGTK lives in the capture helper.
use crate::locked::Locked;
use orbit_host::{
    CloseRequest, DisplayBounds, Host, Monitor, PhysicalPosition, PhysicalSize, WeakHost, Window,
    WindowBackend, WindowEvent, WindowEventHandler,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle,
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent as Event},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    platform::x11::{EventLoopBuilderExtX11, WindowAttributesExtX11},
    window::{Window as Native, WindowId, WindowLevel},
};

type Task = Box<dyn FnOnce(&mut Loop, &ActiveEventLoop) + Send>;
type Registry = Arc<Mutex<HashMap<String, Window>>>;
#[derive(Default)]
pub struct NativeOutputs {
    registry: Registry,
    proxy: OnceLock<Result<EventLoopProxy<Task>, String>>,
    host: Mutex<Option<WeakHost>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl NativeOutputs {
    pub fn attach(&self, host: Host) {
        super::capture_process::install(host.path().app_data_dir());
        *self.host.locked() = Some(host.downgrade());
    }
    fn proxy(&self) -> Result<EventLoopProxy<Task>, String> {
        self.proxy
            .get_or_init(|| {
                let registry = self.registry.clone();
                let host = self.host.locked().clone();
                let (reply, answer) = mpsc::sync_channel(1);
                let thread = std::thread::Builder::new()
                    .name("orbit-output-windows".into())
                    .spawn(move || {
                        let result = EventLoop::<Task>::with_user_event()
                            .with_x11()
                            .with_any_thread(true)
                            .build();
                        match result {
                            Err(error) => {
                                let _ = reply.send(Err(error.to_string()));
                            }
                            Ok(event_loop) => {
                                let _ = reply.send(Ok(event_loop.create_proxy()));
                                let mut state = Loop {
                                    registry,
                                    host,
                                    windows: HashMap::new(),
                                    topology: None,
                                    next_scan: Instant::now(),
                                    next_chrome: Instant::now(),
                                };
                                if let Err(error) = event_loop.run_app(&mut state) {
                                    eprintln!("[output-windows] {error}");
                                }
                                state.destroy_all();
                            }
                        }
                    })
                    .map_err(|error| error.to_string())?;
                *self.thread.locked() = Some(thread);
                answer
                    .recv()
                    .map_err(|_| "The output event loop stopped during startup.".to_string())?
            })
            .clone()
    }
    fn call<T: Send + 'static>(
        &self,
        task: impl FnOnce(&mut Loop, &ActiveEventLoop) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.proxy()?
            .send_event(Box::new(move |state, event_loop| {
                let _ = reply.send(task(state, event_loop));
            }))
            .map_err(|_| "The output event loop has stopped.".to_string())?;
        answer
            .recv()
            .map_err(|_| "The output event loop stopped before replying.".to_string())?
    }
    pub fn get(&self, label: &str) -> Option<Window> {
        self.registry.lock().ok()?.get(label).cloned()
    }
    pub fn monitors(&self, primary: bool) -> Result<Vec<Monitor>, String> {
        self.call(move |_, event_loop| {
            Ok(if primary {
                event_loop
                    .primary_monitor()
                    .into_iter()
                    .map(monitor)
                    .collect()
            } else {
                event_loop.available_monitors().map(monitor).collect()
            })
        })
    }
    pub fn cursor(&self) -> Result<PhysicalPosition<f64>, String> {
        cursor()
    }
    pub fn create(
        &self,
        label: &str,
        title: &str,
        bounds: Option<DisplayBounds>,
        control: Option<(&str, &str)>,
    ) -> Result<Window, String> {
        let (label, title) = (label.to_string(), title.to_string());
        let control = control.map(|(screen, owner)| (screen.to_string(), owner.to_string()));
        let proxy = self.proxy()?;
        self.call(move |state, event_loop| {
            if let Some(window) = state.registry.locked().get(&label).cloned() {
                return Ok(window);
            }
            if let Some((_, owner)) = &control {
                if !state.registry.locked().contains_key(owner) {
                    return Err("The display output owner is unavailable.".into());
                }
            }
            let borderless = bounds.is_some();
            let size = if control.is_some() {
                PhysicalSize::new(38, 38)
            } else {
                bounds
                    .map(|b| PhysicalSize::new(b.width, b.height))
                    .unwrap_or(PhysicalSize::new(960, 540))
            };
            let mut attributes = Native::default_attributes()
                .with_title(title)
                .with_visible(false)
                .with_active(false)
                .with_decorations(false)
                .with_resizable(!borderless)
                .with_inner_size(winit::dpi::PhysicalSize::new(size.width, size.height));
            if borderless {
                attributes = attributes
                    .with_override_redirect(true)
                    .with_window_level(WindowLevel::AlwaysOnTop);
            }
            if let Some(bounds) = bounds {
                attributes =
                    attributes.with_position(winit::dpi::PhysicalPosition::new(bounds.x, bounds.y));
            }
            let native = event_loop
                .create_window(attributes)
                .map_err(|error| error.to_string())?;
            let raw = Handles(
                native
                    .window_handle()
                    .map_err(|error| error.to_string())?
                    .as_raw(),
                native
                    .display_handle()
                    .map_err(|error| error.to_string())?
                    .as_raw(),
            );
            let backend = Arc::new(Backend {
                label: label.clone(),
                handles: raw,
                proxy,
                size: Mutex::new((
                    PhysicalSize::new(native.inner_size().width, native.inner_size().height),
                    PhysicalSize::new(native.outer_size().width, native.outer_size().height),
                )),
                alive: Arc::new(AtomicBool::new(true)),
                visible: AtomicBool::new(false),
                handlers: Mutex::default(),
            });
            let window = Window::new(backend.clone());
            state
                .registry
                .locked()
                .insert(label.clone(), window.clone());
            state.windows.insert(
                native.id(),
                Entry {
                    native,
                    backend,
                    control: control.map(|(screen, _)| screen),
                    auto_chrome: !borderless,
                },
            );
            Ok(window)
        })
    }
    pub fn shutdown(&self) {
        super::capture_process::shutdown();
        if let Some(Ok(proxy)) = self.proxy.get() {
            let _ = proxy.send_event(Box::new(|state, event_loop| {
                state.destroy_all();
                event_loop.exit();
            }));
        }
        if let Some(thread) = self.thread.locked().take() {
            let _ = thread.join();
        }
    }
}

struct Handles(RawWindowHandle, RawDisplayHandle);
// Only the event-loop thread operates the native owners. GPU threads read these immutable handles.
unsafe impl Send for Handles {}
unsafe impl Sync for Handles {}
struct Backend {
    label: String,
    handles: Handles,
    proxy: EventLoopProxy<Task>,
    size: Mutex<(PhysicalSize<u32>, PhysicalSize<u32>)>,
    alive: Arc<AtomicBool>,
    visible: AtomicBool,
    handlers: Mutex<Vec<WindowEventHandler>>,
}
impl Backend {
    fn act(&self, action: impl FnOnce(&mut Loop, WindowId) + Send + 'static) -> Result<(), String> {
        let label = self.label.clone();
        let alive = self.alive.clone();
        self.proxy
            .send_event(Box::new(move |state, _| {
                if !alive.load(Ordering::Acquire) {
                    return;
                }
                let id = state
                    .windows
                    .iter()
                    .find(|(_, entry)| entry.backend.label == label)
                    .map(|(id, _)| *id);
                if let Some(id) = id {
                    action(state, id);
                }
            }))
            .map_err(|_| "The output event loop has stopped.".into())
    }
    fn native(&self, action: impl FnOnce(&Native) + Send + 'static) -> Result<(), String> {
        self.act(move |state, id| action(&state.windows[&id].native))
    }
    fn emit(&self, event: WindowEvent) {
        for handler in self.handlers.locked().iter() {
            handler(&event);
        }
    }
}
impl HasWindowHandle for Backend {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(HandleError::Unavailable);
        }
        Ok(unsafe { WindowHandle::borrow_raw(self.handles.0) })
    }
}
impl HasDisplayHandle for Backend {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(unsafe { DisplayHandle::borrow_raw(self.handles.1) })
    }
}
impl WindowBackend for Backend {
    fn label(&self) -> String {
        self.label.clone()
    }
    fn show(&self) -> Result<(), String> {
        self.visible.store(true, Ordering::Release);
        self.native(|window| window.set_visible(true))
    }
    fn hide(&self) -> Result<(), String> {
        self.visible.store(false, Ordering::Release);
        self.native(|window| window.set_visible(false))
    }
    fn close(&self) -> Result<(), String> {
        self.act(|state, id| state.destroy(id))
    }
    fn set_focus(&self) -> Result<(), String> {
        self.native(Native::focus_window)
    }
    fn maximize(&self) -> Result<(), String> {
        self.native(|window| window.set_maximized(true))
    }
    fn unmaximize(&self) -> Result<(), String> {
        self.native(|window| window.set_maximized(false))
    }
    fn is_visible(&self) -> Result<bool, String> {
        Ok(self.visible.load(Ordering::Acquire))
    }
    fn set_title(&self, title: &str) -> Result<(), String> {
        let title = title.to_string();
        self.native(move |window| window.set_title(&title))
    }
    fn set_position(&self, position: PhysicalPosition<i32>) -> Result<(), String> {
        self.native(move |window| {
            window.set_outer_position(winit::dpi::PhysicalPosition::new(position.x, position.y))
        })
    }
    fn set_size(&self, size: PhysicalSize<u32>) -> Result<(), String> {
        self.native(move |window| {
            let _ =
                window.request_inner_size(winit::dpi::PhysicalSize::new(size.width, size.height));
        })
    }
    fn set_always_on_top(&self, top: bool) -> Result<(), String> {
        self.native(move |window| {
            window.set_window_level(if top {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            });
            if top {
                raise(window);
            }
        })
    }
    fn inner_size(&self) -> Result<PhysicalSize<u32>, String> {
        Ok(self.size.locked().0)
    }
    fn outer_size(&self) -> Result<PhysicalSize<u32>, String> {
        Ok(self.size.locked().1)
    }
    fn native_window_pointer(&self) -> Result<*mut std::ffi::c_void, String> {
        Err("X11 output surfaces use raw window/display handles.".into())
    }
    fn realize(&self) -> Result<(), String> {
        self.window_handle()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
    fn on_window_event(&self, handler: WindowEventHandler) {
        self.handlers.locked().push(handler);
    }
}

struct Entry {
    native: Native,
    backend: Arc<Backend>,
    control: Option<String>,
    auto_chrome: bool,
}
impl Entry {
    fn refresh_chrome(&self) -> Result<(), String> {
        if !self.auto_chrome {
            return Ok(());
        }
        let active = self.native.has_focus();
        let hovered = active && pointer_over(&self.native)?;
        let visible = super::window_controls_visible(active, hovered);
        if self.native.is_decorated() != visible {
            self.native.set_decorations(visible);
        }
        Ok(())
    }
}
struct Loop {
    registry: Registry,
    host: Option<WeakHost>,
    windows: HashMap<WindowId, Entry>,
    topology: Option<Vec<String>>,
    next_scan: Instant,
    next_chrome: Instant,
}
impl Loop {
    fn destroy(&mut self, id: WindowId) {
        if let Some(entry) = self.windows.remove(&id) {
            entry.backend.alive.store(false, Ordering::Release);
            self.registry.locked().remove(&entry.backend.label);
            let handlers = std::mem::take(&mut *entry.backend.handlers.locked());
            drop(entry.native);
            for handler in handlers {
                handler(&WindowEvent::Destroyed);
            }
        }
    }
    fn destroy_all(&mut self) {
        for id in self.windows.keys().copied().collect::<Vec<_>>() {
            self.destroy(id);
        }
    }
}
impl ApplicationHandler<Task> for Loop {
    fn resumed(&mut self, _: &ActiveEventLoop) {}
    fn user_event(&mut self, event_loop: &ActiveEventLoop, task: Task) {
        task(self, event_loop);
    }
    fn window_event(&mut self, _: &ActiveEventLoop, id: WindowId, event: Event) {
        let Some(entry) = self.windows.get(&id) else {
            return;
        };
        match event {
            Event::Destroyed => self.destroy(id),
            Event::CloseRequested => {
                let event = WindowEvent::CloseRequested {
                    api: CloseRequest::default(),
                };
                let handlers = entry.backend.handlers.locked();
                for handler in handlers.iter() {
                    handler(&event);
                }
                let WindowEvent::CloseRequested { api } = event else {
                    unreachable!()
                };
                let prevented = api.is_prevented();
                drop(handlers);
                if !prevented {
                    self.destroy(id);
                }
            }
            Event::Resized(size) => {
                let size = PhysicalSize::new(size.width.max(1), size.height.max(1));
                let outer = entry.native.outer_size();
                let changed = {
                    let mut cached = entry.backend.size.locked();
                    let changed = cached.0 != size;
                    *cached = (size, PhysicalSize::new(outer.width, outer.height));
                    changed
                };
                if changed {
                    entry.backend.emit(WindowEvent::Resized(size));
                }
            }
            Event::ScaleFactorChanged { .. } => {
                let size = entry.native.inner_size();
                let size = PhysicalSize::new(size.width.max(1), size.height.max(1));
                entry.backend.size.locked().0 = size;
                entry.backend.emit(WindowEvent::ScaleFactorChanged {
                    new_inner_size: size,
                });
            }
            Event::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } if entry.control.is_some() => {
                if let Some(host) = self.host.as_ref().and_then(WeakHost::upgrade) {
                    let screen = entry.control.clone().unwrap();
                    std::thread::spawn(move || {
                        if let Err(error) =
                            host.invoke("closeDisplayOutput", vec![serde_json::json!(screen)])
                        {
                            let _ = host.emit("output:display-error", error);
                        }
                    });
                }
            }
            Event::RedrawRequested if entry.control.is_some() => draw_control(&entry.native),
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if Instant::now() >= self.next_chrome {
            self.next_chrome = Instant::now() + Duration::from_millis(100);
            for entry in self.windows.values() {
                if let Err(error) = entry.refresh_chrome() {
                    eprintln!("[output-window-chrome] {error}");
                }
            }
        }
        if Instant::now() >= self.next_scan {
            self.next_scan = Instant::now() + Duration::from_millis(500);
            let topology = event_loop
                .available_monitors()
                .map(|m| {
                    format!(
                        "{:?}:{:?}:{:?}:{}",
                        m.name(),
                        m.position(),
                        m.size(),
                        m.scale_factor()
                    )
                })
                .collect::<Vec<_>>();
            if self.topology.as_ref() != Some(&topology) {
                let changed = self.topology.is_some();
                self.topology = Some(topology);
                if changed {
                    let host = self.host.as_ref().and_then(WeakHost::upgrade);
                    std::thread::spawn(move || {
                        if let Some(host) = host {
                            if let Err(error) = host.refresh_output_displays() {
                                let _ = host.emit("output:display-error", error);
                            }
                        }
                    });
                }
            }
        }
        let wake = if self.windows.values().any(|entry| entry.auto_chrome) {
            self.next_scan.min(self.next_chrome)
        } else {
            self.next_scan
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(wake));
    }
}
fn monitor(m: winit::monitor::MonitorHandle) -> Monitor {
    let position = m.position();
    let size = m.size();
    Monitor {
        name: m.name(),
        position: PhysicalPosition::new(position.x, position.y),
        size: PhysicalSize::new(size.width, size.height),
        scale_factor: m.scale_factor(),
    }
}
fn cursor() -> Result<PhysicalPosition<f64>, String> {
    let xlib = x11_dl::xlib::Xlib::open().map_err(|error| error.to_string())?;
    unsafe {
        let display = (xlib.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return Err(
                "Global output cursor coordinates require X11 or XWayland (DISPLAY).".into(),
            );
        }
        let root = (xlib.XDefaultRootWindow)(display);
        let (mut root_return, mut child, mut rx, mut ry, mut wx, mut wy, mut mask) =
            (0, 0, 0, 0, 0, 0, 0);
        let ok = (xlib.XQueryPointer)(
            display,
            root,
            &mut root_return,
            &mut child,
            &mut rx,
            &mut ry,
            &mut wx,
            &mut wy,
            &mut mask,
        );
        (xlib.XCloseDisplay)(display);
        if ok == 0 {
            return Err("The output cursor is on another X11 screen.".into());
        }
        Ok(PhysicalPosition::new(rx as f64, ry as f64))
    }
}
fn x11(
    window: &Native,
    action: impl FnOnce(&x11_dl::xlib::Xlib, *mut x11_dl::xlib::Display, x11_dl::xlib::Window),
) {
    let (Ok(handle), Ok(display), Ok(xlib)) = (
        window.window_handle(),
        window.display_handle(),
        x11_dl::xlib::Xlib::open(),
    ) else {
        return;
    };
    if let (RawWindowHandle::Xlib(handle), RawDisplayHandle::Xlib(display)) =
        (handle.as_raw(), display.as_raw())
    {
        if let Some(display) = display.display {
            action(&xlib, display.as_ptr().cast(), handle.window);
        }
    }
}
fn raise(window: &Native) {
    x11(window, |xlib, display, window| unsafe {
        (xlib.XRaiseWindow)(display, window);
        (xlib.XFlush)(display);
    });
}
fn pointer_over(window: &Native) -> Result<bool, String> {
    let position = window.outer_position().map_err(|error| error.to_string())?;
    let size = window.outer_size();
    let mut hovered = None;
    // Include the window manager's titlebar, which is outside the client-area cursor events.
    x11(window, |xlib, display, window| unsafe {
        let (mut root, mut child, mut rx, mut ry, mut wx, mut wy, mut mask) = (0, 0, 0, 0, 0, 0, 0);
        let same_screen = (xlib.XQueryPointer)(
            display, window, &mut root, &mut child, &mut rx, &mut ry, &mut wx, &mut wy, &mut mask,
        );
        hovered = Some(
            same_screen != 0
                && i64::from(rx) >= i64::from(position.x)
                && i64::from(ry) >= i64::from(position.y)
                && i64::from(rx) < i64::from(position.x) + i64::from(size.width)
                && i64::from(ry) < i64::from(position.y) + i64::from(size.height),
        );
    });
    hovered.ok_or_else(|| "The X11 output pointer could not be queried.".into())
}
fn draw_control(window: &Native) {
    x11(window, |xlib, display, window| unsafe {
        // Specialist output control matches the existing black/white close affordance.
        let gc = (xlib.XCreateGC)(display, window, 0, std::ptr::null_mut());
        (xlib.XSetForeground)(display, gc, 0);
        (xlib.XFillRectangle)(display, window, gc, 0, 0, 38, 38);
        (xlib.XSetForeground)(display, gc, 0xffffff);
        (xlib.XSetLineAttributes)(display, gc, 2, 0, 1, 0);
        (xlib.XDrawLine)(display, window, gc, 12, 12, 26, 26);
        (xlib.XDrawLine)(display, window, gc, 26, 12, 12, 26);
        (xlib.XFreeGC)(display, gc);
        (xlib.XFlush)(display);
    });
}
