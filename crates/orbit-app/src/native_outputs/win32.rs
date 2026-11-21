use super::logic::{physical_size, window_spec, Coalescer, WindowSpec};
use super::win32_chrome as chrome;
use crate::locked::Locked;
use std::{
    cell::RefCell,
    collections::HashMap,
    num::NonZeroIsize,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock, Weak,
    },
    time::Duration,
};

use orbit_host::{
    CloseRequest, DisplayBounds, Host, Monitor, PhysicalPosition, PhysicalSize, WeakHost, Window,
    WindowBackend, WindowEvent, WindowEventHandler,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    Win32WindowHandle, WindowHandle,
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

type Task = Box<dyn FnOnce() + Send>;
type Registry = Mutex<HashMap<String, Window>>;
thread_local! {
    static OWNED: RefCell<HashMap<isize, Arc<NativeWindow>>> = RefCell::default();
}

#[derive(Default)]
pub struct NativeOutputs {
    registry: Arc<Registry>,
    sender: OnceLock<Result<mpsc::Sender<Task>, String>>,
    host: Mutex<Option<WeakHost>>,
    stopped: Arc<AtomicBool>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    observer: OnceLock<Result<(), String>>,
}

impl NativeOutputs {
    fn sender(&self) -> Result<mpsc::Sender<Task>, String> {
        self.sender
            .get_or_init(|| {
                let (sender, receiver) = mpsc::channel::<Task>();
                let (ready, started) = mpsc::sync_channel(1);
                let stopped = self.stopped.clone();
                let thread = std::thread::Builder::new()
                    .name("orbit-output-windows".into())
                    .spawn(move || unsafe {
                        // GPUI establishes process awareness; explicitly preserve physical pixels on this thread.
                        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
                        let instance = match GetModuleHandleW(None) {
                            Ok(instance) => instance.into(),
                            Err(error) => {
                                let _ = ready.send(Err(error.to_string()));
                                return;
                            }
                        };
                        let cursor = match LoadCursorW(None, IDC_ARROW) {
                            Ok(cursor) => cursor,
                            Err(error) => {
                                let _ = ready.send(Err(error.to_string()));
                                return;
                            }
                        };
                        let class = WNDCLASSW {
                            lpfnWndProc: Some(window_proc),
                            hInstance: instance,
                            lpszClassName: w!("PresenterOrbitNativeOutput"),
                            hCursor: cursor,
                            ..Default::default()
                        };
                        if RegisterClassW(&class) == 0 {
                            let _ = ready.send(Err(windows::core::Error::from_win32().to_string()));
                            return;
                        }
                        let _ = ready.send(Ok(()));
                        while !stopped.load(Ordering::Acquire) {
                            match receiver.recv_timeout(Duration::from_millis(8)) {
                                Ok(task) => run_task(task),
                                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                                Err(mpsc::RecvTimeoutError::Timeout) => {}
                            }
                            for task in receiver.try_iter() {
                                run_task(task);
                            }
                            pump_messages();
                        }
                        let windows: Vec<_> =
                            OWNED.with(|owned| owned.borrow().keys().copied().collect());
                        for hwnd in windows {
                            let _ = DestroyWindow(handle(hwnd));
                        }
                    })
                    .map_err(|error| error.to_string())?;
                *self.thread.locked() = Some(thread);
                started.recv().map_err(|_| {
                    "The output window thread stopped during startup.".to_string()
                })??;
                Ok(sender)
            })
            .clone()
    }

    pub fn attach(&self, host: Host) {
        super::web_capture::install(host.path().app_data_dir());
        *self.host.locked() = Some(host.downgrade());
    }

    pub fn shutdown(&self) {
        let windows = self.registry.locked().values().cloned().collect::<Vec<_>>();
        for window in windows {
            let _ = window.close();
        }
        self.host.locked().take();
        super::web_capture::shutdown();
        self.stopped.store(true, Ordering::Release);
        if let Some(thread) = self.thread.locked().take() {
            let _ = thread.join();
        }
    }

    pub fn get(&self, label: &str) -> Option<Window> {
        self.registry.lock().ok()?.get(label).cloned()
    }

    pub fn monitors(&self, primary_only: bool) -> Result<Vec<Monitor>, String> {
        self.observer
            .get_or_init(|| {
                self.create(
                    "native-output-monitor",
                    "Presenter Orbit display events",
                    Some(DisplayBounds {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    }),
                    None,
                )
                .map(|_| ())
            })
            .clone()?;
        enumerate_monitors(primary_only)
    }

    pub fn cursor(&self) -> Result<PhysicalPosition<f64>, String> {
        let mut position = POINT::default();
        unsafe { GetPhysicalCursorPos(&mut position) }.map_err(|error| error.to_string())?;
        Ok(PhysicalPosition::new(position.x as f64, position.y as f64))
    }

    pub fn create(
        &self,
        label: &str,
        title: &str,
        bounds: Option<DisplayBounds>,
        control: Option<(&str, &str)>,
    ) -> Result<Window, String> {
        if let Some(window) = self.get(label) {
            return Ok(window);
        }
        let sender = self.sender()?;
        let registry = self.registry.clone();
        let label = label.to_string();
        let title = title.to_string();
        let control = match control {
            Some((screen, owner)) => {
                let owner = self
                    .get(owner)
                    .ok_or("The display output owner window is unavailable.")?;
                Some((
                    screen.to_string(),
                    Some(owner.native_window_pointer()? as isize),
                ))
            }
            None => None,
        };
        let host = self
            .host
            .lock()
            .map_err(|_| "The output host is unavailable.")?
            .clone();
        let backend_sender = sender.clone();
        let (reply, answer) = mpsc::sync_channel(1);
        sender
            .send(Box::new(move || {
                let result = create_window(
                    label,
                    title,
                    bounds,
                    control,
                    host,
                    backend_sender,
                    &registry,
                );
                let _ = reply.send(result);
            }))
            .map_err(|_| "The output window thread has stopped.")?;
        answer
            .recv()
            .map_err(|_| "The output window thread stopped before creating a window.".to_string())?
    }
}

fn handle(value: isize) -> HWND {
    HWND(value as *mut _)
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

struct NativeWindow {
    hwnd: isize,
    label: String,
    sender: mpsc::Sender<Task>,
    registry: Weak<Registry>,
    alive: Arc<AtomicBool>,
    visible: AtomicBool,
    size: Mutex<(PhysicalSize<u32>, PhysicalSize<u32>)>,
    handlers: Mutex<Vec<WindowEventHandler>>,
    control: Option<(WeakHost, String)>,
    auto_chrome: bool,
    host: Option<WeakHost>,
}

impl NativeWindow {
    fn command(&self, action: impl FnOnce(HWND) + Send + 'static) -> Result<(), String> {
        if !self.alive.load(Ordering::Acquire) {
            return Err("The output window is closed.".into());
        }
        let hwnd = self.hwnd;
        let label = self.label.clone();
        let alive = self.alive.clone();
        self.sender
            .send(Box::new(move || {
                // Check identity as HWND values can be recycled after destruction.
                let valid = OWNED.with(|owned| {
                    owned
                        .borrow()
                        .get(&hwnd)
                        .is_some_and(|window| window.label == label)
                });
                if valid && alive.load(Ordering::Acquire) {
                    action(handle(hwnd));
                }
            }))
            .map_err(|_| "The output window thread has stopped.".into())
    }

    fn emit(&self, event: WindowEvent) {
        if let Ok(handlers) = self.handlers.lock() {
            for handler in handlers.iter() {
                handler(&event);
            }
        }
    }

    fn update_size(&self, scale_changed: bool) {
        let mut inner = RECT::default();
        let mut outer = RECT::default();
        unsafe {
            if GetClientRect(handle(self.hwnd), &mut inner).is_err()
                || GetWindowRect(handle(self.hwnd), &mut outer).is_err()
            {
                return;
            }
        }
        let inner = physical_size(inner.left, inner.top, inner.right, inner.bottom);
        let outer = physical_size(outer.left, outer.top, outer.right, outer.bottom);
        let changed = {
            let mut size = self.size.locked();
            let changed = size.0 != inner;
            *size = (inner, outer);
            changed
        };
        if !changed && !scale_changed {
            return;
        }
        self.emit(if scale_changed {
            WindowEvent::ScaleFactorChanged {
                new_inner_size: inner,
            }
        } else {
            WindowEvent::Resized(inner)
        });
    }
}

impl HasWindowHandle for NativeWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(HandleError::Unavailable);
        }
        let hwnd = NonZeroIsize::new(self.hwnd).ok_or(HandleError::Unavailable)?;
        // The window thread owns the HWND until close; querying it never dispatches to that thread.
        Ok(unsafe {
            WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(hwnd)))
        })
    }
}
impl HasDisplayHandle for NativeWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::windows())
    }
}
impl WindowBackend for NativeWindow {
    fn label(&self) -> String {
        self.label.clone()
    }
    fn show(&self) -> Result<(), String> {
        self.command(|hwnd| unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        })
    }
    fn hide(&self) -> Result<(), String> {
        self.command(|hwnd| unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        })
    }
    fn close(&self) -> Result<(), String> {
        self.command(|hwnd| unsafe {
            let _ = DestroyWindow(hwnd);
        })
    }
    fn set_focus(&self) -> Result<(), String> {
        self.command(|hwnd| unsafe {
            let _ = SetForegroundWindow(hwnd);
        })
    }
    fn maximize(&self) -> Result<(), String> {
        self.command(|hwnd| unsafe {
            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
        })
    }
    fn unmaximize(&self) -> Result<(), String> {
        self.command(|hwnd| unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        })
    }
    fn is_visible(&self) -> Result<bool, String> {
        Ok(self.visible.load(Ordering::Acquire))
    }
    fn set_title(&self, title: &str) -> Result<(), String> {
        let title = wide(title);
        self.command(move |hwnd| unsafe {
            let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
        })
    }
    fn set_position(&self, position: PhysicalPosition<i32>) -> Result<(), String> {
        self.command(move |hwnd| unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                position.x,
                position.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        })
    }
    fn set_size(&self, size: PhysicalSize<u32>) -> Result<(), String> {
        self.command(move |hwnd| unsafe {
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: size.width as i32,
                bottom: size.height as i32,
            };
            let style = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
            let ex = WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32);
            let _ = AdjustWindowRectExForDpi(&mut rect, style, false, ex, GetDpiForWindow(hwnd));
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        })
    }
    fn set_always_on_top(&self, top: bool) -> Result<(), String> {
        self.command(move |hwnd| unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(if top { HWND_TOPMOST } else { HWND_NOTOPMOST }),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        })
    }
    fn inner_size(&self) -> Result<PhysicalSize<u32>, String> {
        Ok(self
            .size
            .lock()
            .map_err(|_| "Output geometry is unavailable.")?
            .0)
    }
    fn outer_size(&self) -> Result<PhysicalSize<u32>, String> {
        Ok(self
            .size
            .lock()
            .map_err(|_| "Output geometry is unavailable.")?
            .1)
    }
    fn native_window_pointer(&self) -> Result<*mut std::ffi::c_void, String> {
        if !self.alive.load(Ordering::Acquire) {
            return Err("The output window is closed.".into());
        }
        Ok(self.hwnd as *mut _)
    }
    fn realize(&self) -> Result<(), String> {
        self.native_window_pointer().map(|_| ())
    }
    fn on_window_event(&self, handler: WindowEventHandler) {
        self.handlers.locked().push(handler);
    }
}

fn create_window(
    label: String,
    title: String,
    bounds: Option<DisplayBounds>,
    control: Option<(String, Option<isize>)>,
    host: Option<WeakHost>,
    sender: mpsc::Sender<Task>,
    registry: &Arc<Registry>,
) -> Result<Window, String> {
    let is_control = control.is_some();
    let WindowSpec { bounds, borderless } = window_spec(bounds, is_control);
    let owner = control.as_ref().and_then(|(_, owner)| *owner).map(handle);
    let title = wide(&title);
    let style = if borderless {
        WS_POPUP
    } else {
        WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0 & !WS_CAPTION.0)
    };
    let ex = if borderless {
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
    } else {
        WINDOW_EX_STYLE::default()
    };
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: bounds.width as i32,
        bottom: bounds.height as i32,
    };
    unsafe { AdjustWindowRectEx(&mut rect, style, false, ex) }
        .map_err(|error| error.to_string())?;
    let hwnd = unsafe {
        CreateWindowExW(
            ex,
            w!("PresenterOrbitNativeOutput"),
            PCWSTR(title.as_ptr()),
            style,
            bounds.x,
            bounds.y,
            rect.right - rect.left,
            rect.bottom - rect.top,
            owner,
            None,
            Some(
                GetModuleHandleW(None)
                    .map_err(|error| error.to_string())?
                    .into(),
            ),
            None,
        )
    }
    .map_err(|error| error.to_string())?;
    if !borderless {
        let sized = unsafe {
            let mut physical = RECT {
                left: 0,
                top: 0,
                right: bounds.width as i32,
                bottom: bounds.height as i32,
            };
            AdjustWindowRectExForDpi(&mut physical, style, false, ex, GetDpiForWindow(hwnd))
                .and_then(|_| {
                    SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        physical.right - physical.left,
                        physical.bottom - physical.top,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                })
        };
        if let Err(error) = sized {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(error.to_string());
        }
    }
    let backend = Arc::new(NativeWindow {
        hwnd: hwnd.0 as isize,
        label: label.clone(),
        sender,
        registry: Arc::downgrade(registry),
        alive: Arc::new(AtomicBool::new(true)),
        visible: AtomicBool::new(false),
        size: Mutex::new((
            PhysicalSize::new(bounds.width, bounds.height),
            PhysicalSize::new(
                (rect.right - rect.left) as u32,
                (rect.bottom - rect.top) as u32,
            ),
        )),
        handlers: Mutex::new(Vec::new()),
        control: control.and_then(|(screen, _)| host.clone().map(|host| (host, screen))),
        auto_chrome: !borderless,
        host,
    });
    OWNED.with(|owned| owned.borrow_mut().insert(hwnd.0 as isize, backend.clone()));
    backend.update_size(false);
    let window = Window::new(backend);
    registry.locked().insert(label, window.clone());
    if !borderless && unsafe { SetTimer(Some(hwnd), chrome::TIMER_ID, 100, None) } == 0 {
        let error = windows::core::Error::from_win32().to_string();
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(error);
    }
    Ok(window)
}

/// A panicking window task must not end the pump thread that every output window depends on. The panic is
/// already recorded by the process panic hook.
fn run_task(task: Task) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
}

pub(super) unsafe fn pump_messages() {
    let mut message = MSG::default();
    // Bound each batch so a busy browser cannot starve window commands or capture scheduling.
    for _ in 0..256 {
        if !PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            break;
        }
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
}

/// A panic must not unwind out of an `extern "system"` frame, which aborts the process and every window with it.
/// The message falls through to the default procedure instead.
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    std::panic::catch_unwind(|| window_proc_inner(hwnd, message, wparam, lparam))
        .unwrap_or_else(|_| DefWindowProcW(hwnd, message, wparam, lparam))
}

unsafe fn window_proc_inner(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let window = OWNED.with(|owned| owned.borrow().get(&(hwnd.0 as isize)).cloned());
    if let Some(window) = window {
        match message {
            WM_TIMER if window.auto_chrome && wparam.0 == chrome::TIMER_ID => {
                chrome::refresh(hwnd);
                return LRESULT(0);
            }
            WM_ACTIVATE | WM_MOUSEMOVE | WM_NCMOUSEMOVE if window.auto_chrome => {
                chrome::refresh(hwnd)
            }
            WM_SHOWWINDOW => {
                window.visible.store(wparam.0 != 0, Ordering::Release);
            }
            WM_SIZE if wparam.0 != SIZE_MINIMIZED as usize => {
                window.update_size(false);
            }
            WM_DPICHANGED => {
                let rect = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                window.update_size(true);
                return LRESULT(0);
            }
            WM_CLOSE => {
                let event = WindowEvent::CloseRequested {
                    api: CloseRequest::default(),
                };
                let mut prevented = false;
                if let Ok(handlers) = window.handlers.lock() {
                    for handler in handlers.iter() {
                        handler(&event);
                    }
                    if let WindowEvent::CloseRequested { api } = &event {
                        prevented = api.is_prevented();
                    }
                }
                if !prevented {
                    let _ = DestroyWindow(hwnd);
                }
                return LRESULT(0);
            }
            WM_DISPLAYCHANGE => {
                static REFRESH: Coalescer = Coalescer::new();
                REFRESH.changed();
                if let Some(host) = window.host.as_ref().and_then(WeakHost::upgrade) {
                    if REFRESH.start() {
                        let spawned = std::thread::Builder::new()
                            .name("orbit-display-change".into())
                            .spawn(move || loop {
                                let generation = REFRESH.generation();
                                if let Err(error) = host.refresh_output_displays() {
                                    eprintln!("[output-displays] {error}");
                                    let _ = host.emit("output:display-error", error);
                                }
                                if !REFRESH.finish(generation) {
                                    break;
                                }
                            });
                        if spawned.is_err() {
                            REFRESH.abandon();
                        }
                    }
                }
            }
            WM_LBUTTONUP if window.control.is_some() => {
                let (host, screen) = window.control.as_ref().unwrap();
                let Some(host) = host.upgrade() else {
                    return LRESULT(0);
                };
                let screen = screen.clone();
                // Host teardown takes GPU locks, so it must not run inside a native window callback.
                let _ = std::thread::Builder::new()
                    .name("orbit-close-output".into())
                    .spawn(move || {
                        if let Err(error) =
                            host.invoke("closeDisplayOutput", vec![serde_json::json!(screen)])
                        {
                            eprintln!("[output-control] {error}");
                        }
                    });
                return LRESULT(0);
            }
            WM_PAINT if window.control.is_some() => {
                // This specialist output overlay uses the same black/white close affordance as Tauri.
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let rect = RECT {
                    left: 0,
                    top: 0,
                    right: 38,
                    bottom: 38,
                };
                FillRect(dc, &rect, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                let pen = CreatePen(
                    PS_SOLID,
                    2,
                    windows::Win32::Foundation::COLORREF(0x00ffffff),
                );
                let old = SelectObject(dc, pen.into());
                let _ = MoveToEx(dc, 12, 12, None);
                let _ = LineTo(dc, 26, 26);
                let _ = MoveToEx(dc, 26, 12, None);
                let _ = LineTo(dc, 12, 26);
                SelectObject(dc, old);
                let _ = DeleteObject(pen.into());
                let _ = EndPaint(hwnd, &paint);
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                window.alive.store(false, Ordering::Release);
                window.visible.store(false, Ordering::Release);
                if let Some(registry) = window.registry.upgrade() {
                    registry.locked().remove(&window.label);
                }
                OWNED.with(|owned| owned.borrow_mut().remove(&(hwnd.0 as isize)));
                // Lifecycle closures hold window handles; release them when the native owner is gone.
                let handlers = std::mem::take(&mut *window.handlers.locked());
                for handler in handlers {
                    handler(&WindowEvent::Destroyed);
                }
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

fn enumerate_monitors(primary_only: bool) -> Result<Vec<Monitor>, String> {
    struct ResultSet {
        primary_only: bool,
        monitors: Vec<Monitor>,
        error: Option<String>,
    }
    unsafe extern "system" fn visit(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        let result = &mut *(data.0 as *mut ResultSet);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
            result.error = Some(windows::core::Error::from_win32().to_string());
            return false.into();
        }
        if !result.primary_only || info.monitorInfo.dwFlags & 1 != 0 {
            let rect = info.monitorInfo.rcMonitor;
            let length = info
                .szDevice
                .iter()
                .position(|ch| *ch == 0)
                .unwrap_or(info.szDevice.len());
            let mut dx = 96;
            let mut dy = 96;
            if let Err(error) = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) {
                result.error = Some(error.to_string());
                return false.into();
            }
            result.monitors.push(Monitor {
                name: Some(String::from_utf16_lossy(&info.szDevice[..length])),
                position: PhysicalPosition::new(rect.left, rect.top),
                size: PhysicalSize::new(
                    (rect.right - rect.left) as u32,
                    (rect.bottom - rect.top) as u32,
                ),
                scale_factor: dx as f64 / 96.,
            });
        }
        true.into()
    }
    let mut result = ResultSet {
        primary_only,
        monitors: Vec::new(),
        error: None,
    };
    let success = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(visit),
            LPARAM(&mut result as *mut _ as isize),
        )
    }
    .as_bool();
    if let Some(error) = result.error {
        return Err(error);
    }
    if !success {
        return Err(windows::core::Error::from_win32().to_string());
    }
    Ok(result.monitors)
}
