//! Tray icon, minimize to tray, and launch at login, ported from `src-tauri/src/desktop_lifecycle.rs`. The
//! general settings drive them: `DesktopIntegration::apply_preferences` runs on the host's thread when a settings
//! save changes `minimizeToTray` or `launchAtLogin`, and a failed change fails the save.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use orbit_host::DesktopIntegration;
use serde_json::Value;

pub const APP_NAME: &str = "Presenter Orbit";

/// What the general settings say about the window at startup, read once from the saved settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartupPreferences {
    pub start_minimized: bool,
    pub minimize_to_tray: bool,
}

impl StartupPreferences {
    pub fn from_settings(settings: &Value) -> Self {
        let general = &settings["generalSettings"];
        Self {
            start_minimized: general["startMinimized"] == true,
            minimize_to_tray: general["minimizeToTray"] == true,
        }
    }

    pub fn start(self) -> StartMode {
        match (self.start_minimized, self.minimize_to_tray) {
            (false, _) => StartMode::Shown,
            (true, false) => StartMode::Minimized,
            (true, true) => StartMode::Tray,
        }
    }
}

/// How the main window appears at startup, as `desktop_lifecycle::start` decides it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartMode {
    Shown,
    Minimized,
    Tray,
}

/// The tray icon. `configure` runs on the application's main thread and returns when it has finished.
pub trait Tray: Send + Sync {
    fn configure(&self, enabled: bool) -> Result<(), String>;
}

/// The operating system's login items.
pub trait LoginItem: Send + Sync {
    fn set_enabled(&self, enabled: bool) -> Result<(), String>;
}

pub struct GpuiDesktop {
    tray: Arc<dyn Tray>,
    login: Arc<dyn LoginItem>,
    minimize_to_tray: Arc<AtomicBool>,
}

impl GpuiDesktop {
    pub fn new(
        tray: Arc<dyn Tray>,
        login: Arc<dyn LoginItem>,
        minimize_to_tray: Arc<AtomicBool>,
    ) -> Self {
        Self {
            tray,
            login,
            minimize_to_tray,
        }
    }

    fn set_tray(&self, enabled: bool) -> Result<(), String> {
        self.tray.configure(enabled)?;
        self.minimize_to_tray.store(enabled, Ordering::Release);
        Ok(())
    }
}

impl DesktopIntegration for GpuiDesktop {
    fn apply_preferences(&self, previous: &Value, next: &Value) -> Result<(), String> {
        if previous["minimizeToTray"] != next["minimizeToTray"] {
            self.set_tray(next["minimizeToTray"] == true)?;
        }
        if previous["launchAtLogin"] != next["launchAtLogin"] {
            if let Err(error) = self.login.set_enabled(next["launchAtLogin"] == true) {
                self.set_tray(previous["minimizeToTray"] == true)?;
                return Err(format!("Could not update automatic startup: {error}"));
            }
        }
        Ok(())
    }
}

/// Registers the executable under the Run key (Windows), a launch agent (macOS), or an autostart entry (Linux).
/// The entry has its own name, so it does not replace the Tauri app's while both are installed.
pub struct AutoLaunchItem {
    name: String,
}

impl AutoLaunchItem {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }

    fn handle(&self) -> Result<auto_launch::AutoLaunch, String> {
        let path = std::env::current_exe()
            .map_err(|error| error.to_string())?
            .to_str()
            .ok_or_else(|| "The executable path is not valid text.".to_string())?
            .to_string();
        auto_launch::AutoLaunchBuilder::new()
            .set_app_name(&self.name)
            .set_app_path(&path)
            .build()
            .map_err(|error| error.to_string())
    }
}

impl LoginItem for AutoLaunchItem {
    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let item = self.handle()?;
        if enabled {
            item.enable()
        } else {
            item.disable()
        }
        .map_err(|error| error.to_string())
    }
}

#[cfg(any(windows, target_os = "macos"))]
pub mod tray_icon_backend {
    use std::cell::RefCell;
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use futures::channel::mpsc::UnboundedSender;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    use super::{Tray, APP_NAME};
    use crate::shell::{ForegroundMessage, WindowCommand};

    const TRAY_ID: &str = "presenter-orbit";
    const OPEN_ID: &str = "tray-open";
    const QUIT_ID: &str = "tray-quit";
    const ICON: &[u8] = include_bytes!("../assets/tray.png");
    const MAIN_THREAD_TIMEOUT: Duration = Duration::from_secs(10);

    thread_local! {
        // The icon belongs to the main thread, where its window and message loop live.
        static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
    }

    pub struct MainThreadTray {
        sender: Mutex<UnboundedSender<ForegroundMessage>>,
    }

    impl MainThreadTray {
        pub fn new(sender: UnboundedSender<ForegroundMessage>) -> Arc<Self> {
            Arc::new(Self {
                sender: Mutex::new(sender),
            })
        }

        fn sender(&self) -> Result<UnboundedSender<ForegroundMessage>, String> {
            self.sender
                .lock()
                .map(|sender| sender.clone())
                .map_err(|_| "The application foreground has stopped.".to_string())
        }
    }

    fn icon() -> Result<Icon, String> {
        let decoded = image::load_from_memory_with_format(ICON, image::ImageFormat::Png)
            .map_err(|error| error.to_string())?
            .into_rgba8();
        let (width, height) = decoded.dimensions();
        Icon::from_rgba(decoded.into_raw(), width, height).map_err(|error| error.to_string())
    }

    fn create(sender: &UnboundedSender<ForegroundMessage>) -> Result<TrayIcon, String> {
        let open = MenuItem::with_id(OPEN_ID, APP_NAME, true, None);
        let quit = MenuItem::with_id(QUIT_ID, "Quit", true, None);
        let menu = Menu::with_items(&[&open, &quit]).map_err(|error| error.to_string())?;
        let click = Mutex::new(sender.clone());
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                if let Ok(sender) = click.lock() {
                    let _ = sender
                        .unbounded_send(ForegroundMessage::Windows(WindowCommand::RestoreMain));
                }
            }
        }));
        let choices = Mutex::new(sender.clone());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let command = match event.id.as_ref() {
                OPEN_ID => WindowCommand::RestoreMain,
                QUIT_ID => WindowCommand::Quit,
                _ => return,
            };
            if let Ok(sender) = choices.lock() {
                let _ = sender.unbounded_send(ForegroundMessage::Windows(command));
            }
        }));
        TrayIconBuilder::new()
            .with_id(TRAY_ID)
            .with_icon(icon()?)
            .with_tooltip(APP_NAME)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|error| error.to_string())
    }

    /// Shows or hides the icon. Disabling restores a main window that was hidden to the tray, so it cannot be
    /// left without a way back.
    fn configure_on_main_thread(
        sender: &UnboundedSender<ForegroundMessage>,
        enabled: bool,
    ) -> Result<(), String> {
        TRAY.with(|slot| {
            let mut slot = slot.borrow_mut();
            if enabled && slot.is_none() {
                *slot = Some(create(sender)?);
            }
            if let Some(tray) = slot.as_ref() {
                tray.set_visible(enabled)
                    .map_err(|error| error.to_string())?;
            }
            Ok::<(), String>(())
        })?;
        if !enabled {
            sender
                .unbounded_send(ForegroundMessage::Windows(WindowCommand::RestoreIfHidden))
                .map_err(|_| "The application foreground has stopped.".to_string())?;
        }
        Ok(())
    }

    impl Tray for MainThreadTray {
        fn configure(&self, enabled: bool) -> Result<(), String> {
            let sender = self.sender()?;
            let (done, result) = channel();
            let task_sender = sender.clone();
            sender
                .unbounded_send(ForegroundMessage::Task(Box::new(move || {
                    let _ = done.send(configure_on_main_thread(&task_sender, enabled));
                })))
                .map_err(|_| "The application foreground has stopped.".to_string())?;
            result
                .recv_timeout(MAIN_THREAD_TIMEOUT)
                .map_err(|_| "The tray icon could not be updated on the main thread.".to_string())?
        }
    }
}

/// Linux has no GPUI-compatible tray yet; asking for one is an explicit error.
#[cfg(not(any(windows, target_os = "macos")))]
pub struct UnsupportedTray;

#[cfg(not(any(windows, target_os = "macos")))]
impl Tray for UnsupportedTray {
    fn configure(&self, enabled: bool) -> Result<(), String> {
        if enabled {
            Err("The tray icon is not available on this platform yet.".to_string())
        } else {
            Ok(())
        }
    }
}
