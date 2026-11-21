//! AppKit owns output windows on the main thread; cached handles feed the host's independent GPU ticker.
use crate::locked::Locked;
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel, DefinedClass, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSNotificationCenter, NSNumber, NSObject, NSObjectProtocol,
    NSPoint, NSRect, NSSize, NSString, NSTimer,
};
use orbit_host::{
    CloseRequest, DisplayBounds, Host, Monitor, PhysicalPosition, PhysicalSize, WeakHost, Window,
    WindowBackend, WindowEvent, WindowEventHandler,
};
use raw_window_handle::{
    AppKitWindowHandle, DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle,
    RawWindowHandle, WindowHandle,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::c_void,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGDisplayModelNumber(display: u32) -> u32;
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
    fn CGDisplayBounds(display: u32) -> NSRect;
    fn CGMainDisplayID() -> u32;
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *const c_void) -> NSPoint;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: *const c_void);
}

thread_local! {
    static OWNED: RefCell<HashMap<String,Entry>> = RefCell::default();
    static OBSERVER: RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>> = RefCell::default();
}
#[derive(Clone)]
struct Display {
    monitor: Monitor,
    logical: NSRect,
    primary: bool,
}
#[derive(Default)]
pub struct NativeOutputs {
    registry: Arc<Mutex<HashMap<String, Window>>>,
    displays: Arc<Mutex<Vec<Display>>>,
    host: Mutex<Option<WeakHost>>,
}
impl NativeOutputs {
    pub fn attach(&self, host: Host) {
        super::capture_process::install(host.path().app_data_dir());
        *self.host.locked() = Some(host.downgrade());
    }
    fn host(&self) -> Result<WeakHost, String> {
        self.host
            .locked()
            .clone()
            .ok_or("The output host is unavailable.".into())
    }
    fn call<T: Send + 'static>(
        &self,
        task: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        if MainThreadMarker::new().is_some() {
            return task();
        }
        let (reply, answer) = mpsc::sync_channel(1);
        self.host()?
            .upgrade()
            .ok_or("The output host has stopped.")?
            .run_on_main_thread(move || {
                let _ = reply.send(task());
            })?;
        answer
            .recv()
            .map_err(|_| "The AppKit output task stopped before replying.".to_string())?
    }
    pub fn get(&self, label: &str) -> Option<Window> {
        self.registry.lock().ok()?.get(label).cloned()
    }
    pub fn monitors(&self, primary: bool) -> Result<Vec<Monitor>, String> {
        let displays = self.displays.clone();
        let host = self.host()?;
        self.call(move || {
            let mtm = MainThreadMarker::new().ok_or("AppKit needs the main thread.")?;
            let current = read_displays(mtm)?;
            *displays.locked() = current.clone();
            OBSERVER.with(|observer| {
                if observer.borrow().is_some() {
                    return;
                }
                let displays = displays.clone();
                let host = host.clone();
                let block = block2::RcBlock::new(move |_: NonNull<NSNotification>| {
                    if let Some(mtm) = MainThreadMarker::new() {
                        match read_displays(mtm) {
                            Ok(current) => *displays.locked() = current,
                            Err(error) => {
                                if let Some(host) = host.upgrade() {
                                    let _ = host.emit("output:display-error", error);
                                }
                                return;
                            }
                        }
                    }
                    if let Some(host) = host.upgrade() {
                        std::thread::spawn(move || {
                            if let Err(error) = host.refresh_output_displays() {
                                let _ = host.emit("output:display-error", error);
                            }
                        });
                    }
                });
                let token = unsafe {
                    NSNotificationCenter::defaultCenter()
                        .addObserverForName_object_queue_usingBlock(
                            Some(NSApplicationDidChangeScreenParametersNotification),
                            None,
                            None,
                            &block,
                        )
                };
                *observer.borrow_mut() = Some(token);
            });
            Ok(current
                .into_iter()
                .filter(|display| !primary || display.primary)
                .map(|display| display.monitor)
                .collect())
        })
    }
    pub fn cursor(&self) -> Result<PhysicalPosition<f64>, String> {
        let event = unsafe { CGEventCreate(std::ptr::null()) };
        if event.is_null() {
            return Err("The global output cursor could not be queried.".into());
        }
        let point = unsafe {
            let point = CGEventGetLocation(event);
            CFRelease(event);
            point
        };
        let displays = self.displays.locked();
        let display = displays
            .iter()
            .find(|display| contains(display.logical, point))
            .ok_or("The cursor is outside the connected output displays.")?;
        Ok(PhysicalPosition::new(
            point.x * display.monitor.scale_factor,
            point.y * display.monitor.scale_factor,
        ))
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
        let (label, title) = (label.to_string(), title.to_string());
        let control = control.map(|(screen, owner)| (screen.to_string(), owner.to_string()));
        let registry = self.registry.clone();
        let displays = self.displays.clone();
        let host = self.host()?;
        self.call(move || {
            let mtm = MainThreadMarker::new().ok_or("AppKit needs the main thread.")?;
            let current = read_displays(mtm)?;
            *displays.locked() = current;
            let display = choose_display(
                &displays.locked(),
                bounds.map(|b| PhysicalPosition::new(b.x, b.y)),
            )?;
            let control_id = control.as_ref().map(|(screen, _)| screen.clone());
            let physical = if control.is_some() {
                PhysicalSize::new(38, 38)
            } else {
                bounds
                    .map(|b| PhysicalSize::new(b.width, b.height))
                    .unwrap_or(PhysicalSize::new(960, 540))
            };
            let scale = display.monitor.scale_factor;
            let rect = NSRect::new(
                NSPoint::new(
                    display.logical.origin.x,
                    primary_height() - display.logical.origin.y - physical.height as f64 / scale,
                ),
                NSSize::new(
                    physical.width as f64 / scale,
                    physical.height as f64 / scale,
                ),
            );
            let style = if bounds.is_some() {
                NSWindowStyleMask::Borderless
            } else {
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable
                    | NSWindowStyleMask::FullSizeContentView
            };
            let native = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    rect,
                    style,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            unsafe {
                native.setReleasedWhenClosed(false);
            }
            native.setTitle(&NSString::from_str(&title));
            if bounds.is_none() {
                native.setFrame_display(rect, false);
                native.setTitleVisibility(NSWindowTitleVisibility::Hidden);
                native.setTitlebarAppearsTransparent(true);
                native.setMovableByWindowBackground(true);
                window_controls(&native);
            }
            native.setExcludedFromWindowsMenu(true);
            native.setHasShadow(false);
            native.setHidesOnDeactivate(false);
            native.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
            if bounds.is_some() {
                native.setLevel(NSFloatingWindowLevel);
            }
            let view = native
                .contentView()
                .ok_or("The output window has no content view.")?;
            let backend = Arc::new(Backend {
                label: label.clone(),
                view: Retained::as_ptr(&view) as isize,
                host: host.clone(),
                displays: displays.clone(),
                size: Mutex::new((physical, physical)),
                alive: Arc::new(AtomicBool::new(true)),
                visible: AtomicBool::new(false),
                top: AtomicBool::new(bounds.is_some()),
                handlers: Mutex::default(),
                registry: Arc::downgrade(&registry),
            });
            let delegate = Delegate::new(mtm, backend.clone(), control_id);
            native.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            if let Some((_, owner)) = control {
                let parent = OWNED
                    .with(|owned| owned.borrow().get(&owner).map(|entry| entry.native.clone()))
                    .ok_or("The display output owner is unavailable.")?;
                unsafe {
                    parent.addChildWindow_ordered(&native, NSWindowOrderingMode::Above);
                }
                let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str("xmark"),
                    Some(&NSString::from_str(&title)),
                )
                .ok_or("The native close symbol is unavailable.")?;
                let button = unsafe {
                    NSButton::buttonWithImage_target_action(
                        &image,
                        Some(&*delegate),
                        Some(sel!(closeOutput:)),
                        mtm,
                    )
                };
                button.setFrame(NSRect::new(NSPoint::new(0., 0.), rect.size));
                unsafe {
                    view.addSubview(&button);
                }
            }
            let window = Window::new(backend.clone());
            let timer = bounds.is_none().then(|| {
                let label = label.clone();
                let tick = block2::RcBlock::new(move |_: NonNull<NSTimer>| {
                    with_native(&label, window_controls);
                });
                unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.1, true, &tick) }
            });
            OWNED.with(|owned| {
                owned.borrow_mut().insert(
                    label.clone(),
                    Entry {
                        native,
                        _delegate: delegate,
                        timer,
                    },
                )
            });
            registry.locked().insert(label.clone(), window.clone());
            if let Some(bounds) = bounds {
                with_native(&label, |native| {
                    position(native, PhysicalPosition::new(bounds.x, bounds.y), &displays)
                });
            }
            geometry(&backend, false);
            Ok(window)
        })
    }
    pub fn shutdown(&self) {
        super::capture_process::shutdown();
        // Application::run has returned on this thread; close AppKit owners before dropping their views.
        if MainThreadMarker::new().is_some() {
            let windows = OWNED.with(|owned| {
                owned
                    .borrow()
                    .values()
                    .map(|entry| entry.native.clone())
                    .collect::<Vec<_>>()
            });
            for window in windows {
                window.close();
            }
            OWNED.with(|owned| owned.borrow_mut().clear());
            OBSERVER.with(|observer| {
                if let Some(token) = observer.borrow_mut().take() {
                    unsafe {
                        NSNotificationCenter::defaultCenter().removeObserver((*token).as_ref());
                    }
                }
            });
        }
    }
}
fn contains(rect: NSRect, point: NSPoint) -> bool {
    point.x >= rect.origin.x
        && point.y >= rect.origin.y
        && point.x < rect.origin.x + rect.size.width
        && point.y < rect.origin.y + rect.size.height
}
fn primary_height() -> f64 {
    unsafe { CGDisplayBounds(CGMainDisplayID()).size.height }
}
fn read_displays(mtm: MainThreadMarker) -> Result<Vec<Display>, String> {
    NSScreen::screens(mtm)
        .iter()
        .map(|screen| {
            let number = screen
                .deviceDescription()
                .objectForKey(&NSString::from_str("NSScreenNumber"))
                .ok_or("A screen has no display identifier.")?
                .downcast::<NSNumber>()
                .map_err(|_| "A screen display identifier is not numeric.")?;
            let id = number.unsignedIntValue();
            let scale = screen.backingScaleFactor();
            let logical = unsafe { CGDisplayBounds(id) };
            Ok(Display {
                monitor: Monitor {
                    name: Some(format!("Monitor #{}", unsafe { CGDisplayModelNumber(id) })),
                    position: PhysicalPosition::new(
                        (logical.origin.x * scale).round() as i32,
                        (logical.origin.y * scale).round() as i32,
                    ),
                    size: PhysicalSize::new(
                        (unsafe { CGDisplayPixelsWide(id) } as f64 * scale).round() as u32,
                        (unsafe { CGDisplayPixelsHigh(id) } as f64 * scale).round() as u32,
                    ),
                    scale_factor: scale,
                },
                logical,
                primary: id == unsafe { CGMainDisplayID() },
            })
        })
        .collect()
}
fn choose_display(
    displays: &[Display],
    point: Option<PhysicalPosition<i32>>,
) -> Result<Display, String> {
    displays
        .iter()
        .find(|display| {
            point.is_some_and(|p| {
                let b = DisplayBounds::from_monitor(&display.monitor);
                p.x >= b.x
                    && p.y >= b.y
                    && p.x < (b.x as i64 + b.width as i64) as i32
                    && p.y < (b.y as i64 + b.height as i64) as i32
            })
        })
        .or_else(|| displays.iter().find(|display| display.primary))
        .cloned()
        .ok_or("No output display is connected.".into())
}
struct Entry {
    native: Retained<NSWindow>,
    _delegate: Retained<Delegate>,
    timer: Option<Retained<NSTimer>>,
}
impl Drop for Entry {
    fn drop(&mut self) {
        if let Some(timer) = &self.timer {
            timer.invalidate();
        }
    }
}

fn window_controls(window: &NSWindow) {
    let pointer = window.mouseLocationOutsideOfEventStream();
    let size = window.frame().size;
    let hovered =
        pointer.x >= 0. && pointer.y >= 0. && pointer.x < size.width && pointer.y < size.height;
    let visible = super::window_controls_visible(window.isKeyWindow(), hovered);
    for kind in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = window.standardWindowButton(kind) {
            if button.isHidden() == visible {
                button.setHidden(!visible);
            }
        }
    }
}
struct Backend {
    label: String,
    view: isize,
    host: WeakHost,
    displays: Arc<Mutex<Vec<Display>>>,
    size: Mutex<(PhysicalSize<u32>, PhysicalSize<u32>)>,
    alive: Arc<AtomicBool>,
    visible: AtomicBool,
    top: AtomicBool,
    handlers: Mutex<Vec<WindowEventHandler>>,
    registry: std::sync::Weak<Mutex<HashMap<String, Window>>>,
}
fn with_native(label: &str, action: impl FnOnce(&NSWindow)) {
    let native = OWNED.with(|owned| owned.borrow().get(label).map(|entry| entry.native.clone()));
    if let Some(native) = native {
        action(&native);
    }
}
fn position(native: &NSWindow, p: PhysicalPosition<i32>, displays: &Mutex<Vec<Display>>) {
    if let Ok(display) = choose_display(&displays.locked(), Some(p)) {
        native.setFrameTopLeftPoint(NSPoint::new(
            p.x as f64 / display.monitor.scale_factor,
            primary_height() - p.y as f64 / display.monitor.scale_factor,
        ));
    }
}
impl Backend {
    fn act(&self, action: impl FnOnce(&NSWindow) + Send + 'static) -> Result<(), String> {
        let label = self.label.clone();
        let alive = self.alive.clone();
        self.host
            .upgrade()
            .ok_or("The output host has stopped.")?
            .run_on_main_thread(move || {
                if alive.load(Ordering::Acquire) {
                    with_native(&label, action);
                }
            })
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
        let view = NonNull::new(self.view as *mut c_void).ok_or(HandleError::Unavailable)?;
        Ok(unsafe {
            WindowHandle::borrow_raw(RawWindowHandle::AppKit(AppKitWindowHandle::new(view)))
        })
    }
}
impl HasDisplayHandle for Backend {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::appkit())
    }
}
impl WindowBackend for Backend {
    fn label(&self) -> String {
        self.label.clone()
    }
    fn show(&self) -> Result<(), String> {
        self.visible.store(true, Ordering::Release);
        self.act(NSWindow::orderFrontRegardless)
    }
    fn hide(&self) -> Result<(), String> {
        self.visible.store(false, Ordering::Release);
        self.act(|window| window.orderOut(None))
    }
    fn close(&self) -> Result<(), String> {
        self.act(NSWindow::close)
    }
    fn set_focus(&self) -> Result<(), String> {
        self.act(|window| window.makeKeyAndOrderFront(None))
    }
    fn maximize(&self) -> Result<(), String> {
        self.act(|window| {
            if !window.isZoomed() {
                window.zoom(None)
            }
        })
    }
    fn unmaximize(&self) -> Result<(), String> {
        self.act(|window| {
            if window.isZoomed() {
                window.zoom(None)
            }
        })
    }
    fn is_visible(&self) -> Result<bool, String> {
        Ok(self.visible.load(Ordering::Acquire))
    }
    fn set_title(&self, title: &str) -> Result<(), String> {
        let title = title.to_string();
        self.act(move |window| window.setTitle(&NSString::from_str(&title)))
    }
    fn set_position(&self, p: PhysicalPosition<i32>) -> Result<(), String> {
        let displays = self.displays.clone();
        self.act(move |window| position(window, p, &displays))
    }
    fn set_size(&self, size: PhysicalSize<u32>) -> Result<(), String> {
        self.act(move |window| {
            window.setContentSize(NSSize::new(
                size.width as f64 / window.backingScaleFactor(),
                size.height as f64 / window.backingScaleFactor(),
            ))
        })
    }
    fn set_always_on_top(&self, top: bool) -> Result<(), String> {
        if self.top.swap(top, Ordering::AcqRel) == top {
            return Ok(());
        }
        self.act(move |window| {
            window.setLevel(if top {
                NSFloatingWindowLevel
            } else {
                NSNormalWindowLevel
            })
        })
    }
    fn inner_size(&self) -> Result<PhysicalSize<u32>, String> {
        Ok(self.size.locked().0)
    }
    fn outer_size(&self) -> Result<PhysicalSize<u32>, String> {
        Ok(self.size.locked().1)
    }
    fn native_window_pointer(&self) -> Result<*mut c_void, String> {
        if self.alive.load(Ordering::Acquire) {
            Ok(self.view as *mut c_void)
        } else {
            Err("The output window is closed.".into())
        }
    }
    fn realize(&self) -> Result<(), String> {
        self.native_window_pointer().map(|_| ())
    }
    fn on_window_event(&self, handler: WindowEventHandler) {
        self.handlers.locked().push(handler);
    }
}
fn geometry(backend: &Backend, scaled: bool) {
    with_native(&backend.label, |window| {
        let Some(view) = window.contentView() else {
            return;
        };
        let inner = view.frame().size;
        let outer = window.frame().size;
        let scale = window.backingScaleFactor();
        let inner = PhysicalSize::new(
            (inner.width * scale).round().max(1.) as u32,
            (inner.height * scale).round().max(1.) as u32,
        );
        *backend.size.locked() = (
            inner,
            PhysicalSize::new(
                (outer.width * scale).round() as u32,
                (outer.height * scale).round() as u32,
            ),
        );
        backend.emit(if scaled {
            WindowEvent::ScaleFactorChanged {
                new_inner_size: inner,
            }
        } else {
            WindowEvent::Resized(inner)
        });
    });
}
struct Ivars {
    backend: Arc<Backend>,
    screen: Option<String>,
}
define_class!(
    #[unsafe(super=NSObject)]
    #[thread_kind=MainThreadOnly]
    #[ivars=Ivars]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowShouldClose:))]
        fn should_close(&self,_:&NSWindow)->bool {
            // The host may queue an explicit close after releasing the GPU surface.
            let event=WindowEvent::CloseRequested{api:CloseRequest::default()};
            let handlers=self.ivars().backend.handlers.locked();for handler in handlers.iter(){handler(&event);}
            let WindowEvent::CloseRequested{api}=event else{unreachable!()};!api.is_prevented()
        }
        #[unsafe(method(windowDidResize:))]
        fn resized(&self,_:&NSNotification){geometry(&self.ivars().backend,false);}
        #[unsafe(method(windowDidChangeBackingProperties:))]
        fn scaled(&self,_:&NSNotification){geometry(&self.ivars().backend,true);}
        #[unsafe(method(windowWillClose:))]
        fn closed(&self,_:&NSNotification){
            let backend=&self.ivars().backend;backend.alive.store(false,Ordering::Release);backend.visible.store(false,Ordering::Release);
            if let Some(registry)=backend.registry.upgrade(){registry.locked().remove(&backend.label);}
            let handlers=std::mem::take(&mut *backend.handlers.locked());for handler in handlers{handler(&WindowEvent::Destroyed);}
            let label=backend.label.clone();if let Some(host)=backend.host.upgrade(){let _=host.run_on_main_thread(move || {OWNED.with(|owned|owned.borrow_mut().remove(&label));});}
        }
    }
    impl Delegate {
        #[unsafe(method(closeOutput:))]
        fn close_output(&self,_:Option<&AnyObject>){if let (Some(host),Some(screen))=(self.ivars().backend.host.upgrade(),self.ivars().screen.clone()){std::thread::spawn(move ||{if let Err(error)=host.invoke("closeDisplayOutput",vec![serde_json::json!(screen)]){let _=host.emit("output:display-error",error);}});}}
    }
);
impl Delegate {
    fn new(mtm: MainThreadMarker, backend: Arc<Backend>, screen: Option<String>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { backend, screen });
        unsafe { msg_send![super(this), init] }
    }
}
