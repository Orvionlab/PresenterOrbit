//! The Settings, Screens, and About windows. They are opened hidden at startup, closing hides them, and a hidden
//! window keeps its view, position, and size, exactly as `docs/auxiliary-windows.md` describes. Their content
//! arrives with Phase 14, except About.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, size, AnyWindowHandle, App, AppContext, Bounds, Context, Entity, Global,
    SharedString, Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use orbit_host::Host;
use orbit_ui::controls::{ActionButton, ButtonVariant, Dialog, EmptyState, Root};
use orbit_ui::{follow_theme, ActiveTheme, Translator};
use serde_json::json;

use crate::native_window::WindowControl;
use crate::windows::{Kind, Registry};

mod licenses;

/// The windows of one application. A global, so views and menus reach it without owning it.
pub struct AuxiliaryWindows {
    registry: Arc<Registry>,
    control: Rc<dyn WindowControl>,
    host: Host,
    entries: HashMap<Kind, Entry>,
    /// Windows that have been shown once. The first show applies the placement GPUI keeps for a window created
    /// hidden; later ones only make it visible again.
    shown: HashSet<Kind>,
    /// Windows that are open for the user. Closing one hides it and removes it here.
    open: HashSet<Kind>,
}

struct Entry {
    window: AnyWindowHandle,
    view: Entity<AuxiliaryView>,
}

impl Global for AuxiliaryWindows {}

impl AuxiliaryWindows {
    pub fn install(
        cx: &mut App,
        host: Host,
        registry: Arc<Registry>,
        control: Rc<dyn WindowControl>,
    ) {
        cx.set_global(Self {
            registry,
            control,
            host,
            entries: HashMap::new(),
            shown: HashSet::new(),
            open: HashSet::new(),
        });
    }

    /// Creates every window that does not exist yet, hidden. Each reports ready once its settings loaded.
    pub fn prepare(cx: &mut App) -> Result<(), String> {
        for kind in Kind::ALL {
            Self::ensure(cx, kind)?;
        }
        Ok(())
    }

    fn ensure(cx: &mut App, kind: Kind) -> Result<(), String> {
        if cx.global::<Self>().entries.contains_key(&kind) {
            return Ok(());
        }
        let (width, height) = kind.size();
        let (min_width, min_height) = kind.minimum_size();
        let host = cx.global::<Self>().host.clone();
        let built: Rc<RefCell<Option<Entity<AuxiliaryView>>>> = Rc::default();
        let capture = built.clone();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(width), px(height)),
                cx,
            ))),
            // No operating-system title bar: the dialog header moves the window. The title is still the
            // window's name for the taskbar.
            titlebar: Some(TitlebarOptions {
                title: Some(kind.title().into()),
                appears_transparent: true,
                ..Default::default()
            }),
            show: false,
            focus: false,
            is_resizable: true,
            window_min_size: Some(size(px(min_width), px(min_height))),
            ..Default::default()
        };
        let window = cx
            .open_window(options, move |window, cx| {
                let view = cx.new(|cx| AuxiliaryView::new(kind, host, window, cx));
                *capture.borrow_mut() = Some(view.clone());
                cx.new(|cx| Root::new(view, window, cx))
            })
            .map_err(|error| error.to_string())?;
        let view = built
            .borrow_mut()
            .take()
            .ok_or_else(|| "The auxiliary window did not build its view.".to_string())?;
        window
            .update(cx, |_, window, cx| {
                window.on_window_should_close(cx, move |window, cx| {
                    close_window(kind, window, cx);
                    false
                });
            })
            .map_err(|error| error.to_string())?;
        cx.global_mut::<Self>().entries.insert(
            kind,
            Entry {
                window: window.into(),
                view,
            },
        );
        Ok(())
    }

    /// Called by a window's view once its content and settings have loaded.
    pub fn report_ready(cx: &mut App, kind: Kind) -> Result<(), String> {
        let requested = cx.global::<Self>().registry.mark_ready(kind)?;
        if requested {
            Self::show(cx, kind)?;
        }
        Ok(())
    }

    /// Shows a prepared window on the panel its request named, and focuses it. The window keeps its position
    /// and size.
    pub fn show(cx: &mut App, kind: Kind) -> Result<(), String> {
        Self::ensure(cx, kind)?;
        let manager = cx.global::<Self>();
        let (panel, control) = (manager.registry.panel(kind)?, manager.control.clone());
        let first = !manager.shown.contains(&kind);
        let entry = manager
            .entries
            .get(&kind)
            .ok_or_else(|| "The auxiliary window is unavailable.".to_string())?;
        let (window, view) = (entry.window, entry.view.clone());
        view.update(cx, |view, cx| view.set_panel(&panel, cx));
        window
            .update(cx, |_, window, _| {
                if first {
                    window.activate_window();
                    Ok(())
                } else {
                    let shown = control.show(window);
                    window.activate_window();
                    shown
                }
            })
            .map_err(|error| error.to_string())??;
        let manager = cx.global_mut::<Self>();
        manager.shown.insert(kind);
        manager.open.insert(kind);
        Ok(())
    }

    pub fn hide(cx: &mut App, kind: Kind) -> Result<(), String> {
        let manager = cx.global::<Self>();
        let control = manager.control.clone();
        let window = manager
            .entries
            .get(&kind)
            .ok_or_else(|| "The auxiliary window is unavailable.".to_string())?
            .window;
        cx.global_mut::<Self>().open.remove(&kind);
        window
            .update(cx, |_, window, _| control.hide(window))
            .map_err(|error| error.to_string())?
    }

    /// Forgets the open request of a window its own content closes.
    pub fn dismiss(&self, kind: Kind) -> Result<(), String> {
        self.registry.close(kind)
    }

    /// Closing the main window destroys the auxiliary windows too.
    pub fn destroy_all(cx: &mut App) {
        let windows: Vec<AnyWindowHandle> = cx
            .global_mut::<Self>()
            .entries
            .drain()
            .map(|(_, entry)| entry.window)
            .collect();
        for window in windows {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    }
}

/// Closing from inside a window, by its close button or the system: hide it and forget the open request.
fn close_window(kind: Kind, window: &Window, cx: &mut App) {
    let manager = cx.global_mut::<AuxiliaryWindows>();
    manager.open.remove(&kind);
    if let Err(error) = manager
        .registry
        .close(kind)
        .and_then(|_| manager.control.hide(window))
    {
        eprintln!("[auxiliary-windows] {error}");
    }
}

enum Status {
    Loading,
    Ready,
    Failed(String),
}

enum AboutView {
    Overview,
    Licenses,
}

pub struct AuxiliaryView {
    kind: Kind,
    host: Host,
    panel: String,
    status: Status,
    about: AboutView,
    licenses: Option<Entity<licenses::Licenses>>,
    log_status: Option<SharedString>,
    settings: Option<Entity<crate::settings::SettingsView>>,
    manage: Option<Entity<crate::workspaces::manage::ManageView>>,
    prompt: Option<Entity<crate::workspaces::prompt::PromptView>>,
    _theme: [Subscription; 2],
}

impl AuxiliaryView {
    fn new(kind: Kind, host: Host, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            kind,
            host: host.clone(),
            panel: crate::windows::DEFAULT_PANEL.to_string(),
            status: Status::Loading,
            about: AboutView::Overview,
            licenses: (kind == Kind::About)
                .then(|| cx.new(|cx| licenses::Licenses::new(host.clone(), window, cx))),
            log_status: None,
            settings: None,
            manage: None,
            prompt: None,
            _theme: follow_theme(window, cx),
        };
        if cx
            .try_global::<crate::workspaces::SharedWorkspaces>()
            .is_some()
        {
            match kind {
                Kind::Workspaces => {
                    view.manage = Some(
                        cx.new(|cx| crate::workspaces::manage::ManageView::new(host.clone(), cx)),
                    )
                }
                Kind::WorkspacePrompt => {
                    view.prompt = Some(cx.new(crate::workspaces::prompt::PromptView::new))
                }
                _ => {}
            }
        }
        if matches!(kind, Kind::Settings | Kind::Screens) {
            view.settings = cx
                .try_global::<crate::settings::SharedRuntime>()
                .map(|shared| shared.0.clone())
                .map(|runtime| {
                    cx.new(|cx| {
                        if kind == Kind::Screens {
                            crate::settings::SettingsView::screens(runtime, cx)
                        } else {
                            crate::settings::SettingsView::new(runtime, cx)
                        }
                    })
                });
        }
        cx.spawn(async move |view, cx| {
            let settings = cx
                .background_spawn(async move { host.invoke("getSettings", Vec::new()) })
                .await;
            view.update(cx, |view, cx| {
                view.status = match settings {
                    Ok(_) => Status::Ready,
                    Err(error) => Status::Failed(error),
                };
                cx.notify();
            })
            .ok();
            // A failed load is still reported, so that opening the window shows the failure instead of nothing.
            cx.update(|cx| {
                if let Err(error) = AuxiliaryWindows::report_ready(cx, kind) {
                    eprintln!("[auxiliary-windows] {error}");
                }
            });
        })
        .detach();
        view
    }

    fn set_panel(&mut self, panel: &str, cx: &mut Context<Self>) {
        if self.kind == Kind::About {
            self.about = if panel == "licenses" {
                if let Some(licenses) = &self.licenses {
                    licenses.update(cx, |view, cx| view.reset(cx));
                }
                AboutView::Licenses
            } else {
                AboutView::Overview
            };
            cx.notify();
        }
        // A workspace prompt carries its request in the panel, and every request replaces the last.
        if let Some(prompt) = &self.prompt {
            prompt.update(cx, |prompt, cx| prompt.request(panel, cx));
        }
        if let Some(manage) = &self.manage {
            manage.update(cx, |manage, cx| manage.shown(cx));
        }
        if self.kind == Kind::Settings && self.panel != panel {
            self.panel = panel.to_string();
            if let Some(settings) = &self.settings {
                settings.update(cx, |settings, cx| settings.select(panel, cx));
            }
            cx.notify();
        }
    }

    fn open_logs(&mut self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        cx.spawn(async move |view, cx| {
            let opened = cx
                .background_spawn(async move { host.invoke("openLogFiles", Vec::new()) })
                .await;
            view.update(cx, |view, cx| {
                let t = *cx.global::<Translator>();
                view.log_status = match opened {
                    Ok(result) if result["ok"] == json!(true) => None,
                    _ => Some(t.t("about.logsFailed").into()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn about_body(&self, t: Translator, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = *cx.theme();
        match self.about {
            AboutView::Licenses => self.licenses.as_ref().map_or_else(
                || EmptyState::new(t.t("window.openFailed")).into_any_element(),
                |view| view.clone().into_any_element(),
            ),
            AboutView::Overview => div()
                .flex()
                .flex_col()
                .gap(theme.metrics.panel_gap)
                .child(
                    div()
                        .text_size(theme.metrics.font_2xl)
                        .text_color(theme.colors.text_strong)
                        .child("Presenter Orbit"),
                )
                .child(div().child(t.format("about.version", &[("version", self.host.version())])))
                .child(div().child(t.t("about.description")))
                .child(
                    div()
                        .flex()
                        .gap(theme.metrics.control_gap)
                        .child(
                            ActionButton::new("about-licenses-open", t.t("about.licenses"))
                                .on_click(cx.listener(|view, _, _, cx| {
                                    if let Some(licenses) = &view.licenses {
                                        licenses.update(cx, |view, cx| view.reset(cx));
                                    }
                                    view.about = AboutView::Licenses;
                                    cx.notify();
                                })),
                        )
                        .child(
                            ActionButton::new("about-open-logs", t.t("about.openLogs"))
                                .variant(ButtonVariant::Secondary)
                                .on_click(cx.listener(|view, _, _, cx| view.open_logs(cx))),
                        ),
                )
                .children(
                    self.log_status
                        .clone()
                        .map(|status| div().text_color(theme.colors.text_muted).child(status)),
                )
                .into_any_element(),
        }
    }
}

impl Render for AuxiliaryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let kind = self.kind;
        let title = match (kind, &self.about) {
            (Kind::Settings, _) => t.t("settings.title"),
            (Kind::Screens, _) => t.t("screenSettings.title"),
            (Kind::Workspaces, _) => t.t("workspace.manageTitle"),
            (Kind::WorkspacePrompt, _) => t.t("workspace.manageTitle"),
            (Kind::About, AboutView::Licenses) => t.t("about.licensesTitle"),
            (Kind::About, AboutView::Overview) => t.t("about.title"),
        };
        let mut dialog = Dialog::new(
            SharedString::from(format!("{}-window", kind.label())),
            title,
            t.t("common.close"),
            move |_, window, cx| close_window(kind, window, cx),
        )
        .standalone();
        if !matches!(kind, Kind::About | Kind::WorkspacePrompt)
            || matches!((kind, &self.about), (Kind::About, AboutView::Licenses))
        {
            dialog = dialog.flush();
        }
        if matches!((kind, &self.about), (Kind::About, AboutView::Licenses)) {
            dialog = dialog.action(
                ActionButton::new("about-back", t.t("about.back"))
                    .variant(ButtonVariant::Secondary)
                    .on_click(cx.listener(|view, _, _, cx| {
                        let detail = view
                            .licenses
                            .as_ref()
                            .is_some_and(|licenses| licenses.update(cx, |view, cx| view.back(cx)));
                        if !detail {
                            view.about = AboutView::Overview;
                        }
                        cx.notify();
                    })),
            );
        }
        let body: gpui::AnyElement = match (&self.status, kind) {
            (Status::Loading, _) => {
                EmptyState::new(t.t("status.workspace.loading")).into_any_element()
            }
            (Status::Failed(error), _) => div()
                .child(t.t("window.openFailed"))
                .child(div().child(error.clone()))
                .into_any_element(),
            (Status::Ready, Kind::About) => self.about_body(t, cx),
            (Status::Ready, Kind::Settings | Kind::Screens) => match &self.settings {
                Some(settings) => settings.clone().into_any_element(),
                None => EmptyState::new(t.t("window.openFailed")).into_any_element(),
            },
            (Status::Ready, Kind::Workspaces) => match &self.manage {
                Some(manage) => manage.clone().into_any_element(),
                None => EmptyState::new(t.t("window.openFailed")).into_any_element(),
            },
            (Status::Ready, Kind::WorkspacePrompt) => match &self.prompt {
                // The prompt draws its own dialog: its title and footer depend on the request.
                Some(prompt) => return prompt.clone().into_any_element(),
                None => EmptyState::new(t.t("window.openFailed")).into_any_element(),
            },
        };
        dialog.child(body).into_any_element()
    }
}
