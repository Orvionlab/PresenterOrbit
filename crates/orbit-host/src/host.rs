use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

use serde::Serialize;

use crate::{
    DesktopIntegration, HostEvents, Monitor, PhysicalPosition, Updater, Window, WindowSystem,
};

/// The services the host keeps for its lifetime, looked up by type.
#[derive(Default)]
pub struct HostStates(HashMap<TypeId, Box<dyn Any + Send + Sync>>);

impl HostStates {
    pub fn insert<T: Send + Sync + 'static>(&mut self, state: T) {
        self.0.insert(TypeId::of::<T>(), Box::new(state));
    }
}

pub struct HostParts {
    pub events: Arc<dyn HostEvents>,
    pub windows: Arc<dyn WindowSystem>,
    pub desktop: Arc<dyn DesktopIntegration>,
    pub updater: Arc<dyn Updater>,
    pub version: String,
    /// The shell's name, as reported in diagnostics.
    pub runtime: &'static str,
    pub app_data_dir: Result<PathBuf, String>,
    pub states: HostStates,
}

struct HostInner {
    events: Arc<dyn HostEvents>,
    windows: Arc<dyn WindowSystem>,
    desktop: Arc<dyn DesktopIntegration>,
    updater: Arc<dyn Updater>,
    version: String,
    runtime: &'static str,
    app_data_dir: Result<PathBuf, String>,
    states: HostStates,
}

/// A cheap handle to the running host, replacing the shell's application handle.
#[derive(Clone)]
pub struct Host(Arc<HostInner>);

/// A shell callback must not keep the host and its window system alive through each other.
#[derive(Clone)]
pub struct WeakHost(Weak<HostInner>);

impl WeakHost {
    pub fn upgrade(&self) -> Option<Host> {
        self.0.upgrade().map(Host)
    }
}

pub struct PathResolver<'a>(&'a Host);

impl PathResolver<'_> {
    pub fn app_data_dir(&self) -> Result<PathBuf, String> {
        self.0 .0.app_data_dir.clone()
    }
}

impl Host {
    pub fn downgrade(&self) -> WeakHost {
        WeakHost(Arc::downgrade(&self.0))
    }
    pub fn new(parts: HostParts) -> Self {
        Self(Arc::new(HostInner {
            events: parts.events,
            windows: parts.windows,
            desktop: parts.desktop,
            updater: parts.updater,
            version: parts.version,
            runtime: parts.runtime,
            app_data_dir: parts.app_data_dir,
            states: parts.states,
        }))
    }

    pub fn state<T: Send + Sync + 'static>(&self) -> &T {
        self.try_state()
            .unwrap_or_else(|| panic!("state not registered: {}", std::any::type_name::<T>()))
    }

    pub fn try_state<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.0
            .states
            .0
            .get(&TypeId::of::<T>())
            .and_then(|state| state.downcast_ref::<T>())
    }

    pub fn emit<S: Serialize>(&self, event: &str, payload: S) -> Result<(), String> {
        let payload = serde_json::to_value(payload).map_err(|error| error.to_string())?;
        self.0.events.emit(event, payload)
    }

    pub fn emit_to<S: Serialize>(
        &self,
        label: &str,
        event: &str,
        payload: S,
    ) -> Result<(), String> {
        let payload = serde_json::to_value(payload).map_err(|error| error.to_string())?;
        self.0.events.emit_to(label, event, payload)
    }

    pub fn windows(&self) -> &dyn WindowSystem {
        &*self.0.windows
    }

    pub fn desktop(&self) -> &dyn DesktopIntegration {
        &*self.0.desktop
    }

    pub fn updater(&self) -> &dyn Updater {
        &*self.0.updater
    }

    pub fn get_window(&self, label: &str) -> Option<Window> {
        self.0.windows.get_window(label)
    }

    pub fn available_monitors(&self) -> Result<Vec<Monitor>, String> {
        self.0.windows.available_monitors()
    }

    pub fn primary_monitor(&self) -> Result<Option<Monitor>, String> {
        self.0.windows.primary_monitor()
    }

    pub fn cursor_position(&self) -> Result<PhysicalPosition<f64>, String> {
        self.0.windows.cursor_position()
    }

    pub fn run_on_main_thread(&self, task: impl FnOnce() + Send + 'static) -> Result<(), String> {
        self.0.windows.run_on_main_thread(Box::new(task))
    }

    pub fn path(&self) -> PathResolver<'_> {
        PathResolver(self)
    }

    pub fn version(&self) -> &str {
        &self.0.version
    }

    pub fn runtime(&self) -> &'static str {
        self.0.runtime
    }
}
