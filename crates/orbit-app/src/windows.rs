//! The auxiliary window contract of `docs/auxiliary-windows.md`, ported from `src-tauri/src/auxiliary_windows.rs`
//! and `src/shared/auxiliary-windows.ts`. This file is the host-facing, thread-safe part: the registry of which
//! window is ready, requested, and on which panel. The GPUI windows themselves live in `auxiliary.rs`.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Settings,
    Screens,
    About,
    Workspaces,
    #[serde(rename = "workspace-prompt")]
    WorkspacePrompt,
}

impl Kind {
    // `WorkspacePrompt` follows `Workspaces`, the window it belongs to.
    pub const ALL: [Self; 5] = [
        Self::Settings,
        Self::Screens,
        Self::About,
        Self::Workspaces,
        Self::WorkspacePrompt,
    ];

    /// Settings and About name a page; workspace prompts carry their request in the same field.
    pub fn accepts_panel(self) -> bool {
        matches!(self, Self::Settings | Self::About | Self::WorkspacePrompt)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Screens => "screens",
            Self::About => "about",
            Self::Workspaces => "workspaces",
            Self::WorkspacePrompt => "workspace-prompt",
        }
    }

    /// The operating system's name for the window, as in the Tauri registry.
    pub fn title(self) -> &'static str {
        match self {
            Self::Settings => "Presenter Orbit Settings",
            Self::Screens => "Presenter Orbit Screen Settings",
            Self::About => "About Presenter Orbit",
            Self::Workspaces => "Manage Workspaces",
            Self::WorkspacePrompt => "Workspace",
        }
    }

    /// Initial size in logical pixels.
    pub fn size(self) -> (f32, f32) {
        match self {
            Self::Settings | Self::Screens => (1180., 760.),
            Self::About => (760., 600.),
            Self::Workspaces => (460., 420.),
            Self::WorkspacePrompt => (400., 260.),
        }
    }

    pub fn minimum_size(self) -> (f32, f32) {
        match self {
            Self::Settings | Self::Screens => (760., 520.),
            Self::About => (420., 360.),
            Self::Workspaces => (360., 300.),
            Self::WorkspacePrompt => (320., 240.),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub kind: Kind,
    pub panel: Option<String>,
}

impl Request {
    pub fn parse(request: Value) -> Result<Self, String> {
        let request: Self = serde_json::from_value(request).map_err(|error| error.to_string())?;
        if request.panel.is_some() && !request.kind.accepts_panel() {
            return Err("This window does not accept a panel.".to_string());
        }
        if request.kind == Kind::About
            && request
                .panel
                .as_deref()
                .is_some_and(|panel| panel != "licenses")
        {
            return Err("The About window only accepts the licenses page.".into());
        }
        if request.kind == Kind::WorkspacePrompt && request.panel.is_none() {
            return Err("A workspace prompt needs a request.".to_string());
        }
        Ok(request)
    }
}

pub fn parse_kind(kind: &Value) -> Result<Kind, String> {
    serde_json::from_value(kind.clone()).map_err(|error| error.to_string())
}

#[derive(Debug, Default)]
pub struct WindowState {
    pub ready: bool,
    pub requested: bool,
    pub panel: Option<String>,
}

impl WindowState {
    /// Records an open request and returns whether the window can be shown now.
    pub fn request(&mut self, panel: Option<String>) -> bool {
        self.requested = true;
        self.panel = panel;
        self.ready
    }

    /// Records readiness and returns whether a request is waiting for it.
    pub fn mark_ready(&mut self) -> bool {
        self.ready = true;
        self.requested
    }
}

/// The panel a settings window shows when the request names none.
pub const DEFAULT_PANEL: &str = "general";

#[derive(Default)]
pub struct Registry(Mutex<HashMap<Kind, WindowState>>);

impl Registry {
    fn with<T>(&self, kind: Kind, action: impl FnOnce(&mut WindowState) -> T) -> Result<T, String> {
        let mut states = self
            .0
            .lock()
            .map_err(|_| "The auxiliary window lifecycle is unavailable.".to_string())?;
        Ok(action(states.entry(kind).or_default()))
    }

    pub fn request(&self, request: &Request) -> Result<bool, String> {
        self.with(request.kind, |state| state.request(request.panel.clone()))
    }

    pub fn mark_ready(&self, kind: Kind) -> Result<bool, String> {
        self.with(kind, WindowState::mark_ready)
    }

    pub fn close(&self, kind: Kind) -> Result<(), String> {
        self.with(kind, |state| state.requested = false)
    }

    pub fn panel(&self, kind: Kind) -> Result<String, String> {
        self.with(kind, |state| {
            state
                .panel
                .clone()
                .unwrap_or_else(|| DEFAULT_PANEL.to_string())
        })
    }
}

pub fn ok() -> Value {
    json!({ "ok": true })
}
