use crate::{
    desktop::{LoginItem, Tray},
    foreground::MainWindow,
    native_window::WindowControl,
    shell::{ForegroundMessage, Services},
};
use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext};
use orbit_host::Host;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

pub struct Folder(pub PathBuf);
impl Folder {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!("orbit-e2e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Desktop;
impl Tray for Desktop {
    fn configure(&self, _: bool) -> Result<(), String> {
        Ok(())
    }
}
impl LoginItem for Desktop {
    fn set_enabled(&self, _: bool) -> Result<(), String> {
        Ok(())
    }
}
struct Control;
impl WindowControl for Control {
    fn show(&self, _: &gpui::Window) -> Result<(), String> {
        Ok(())
    }
    fn hide(&self, _: &gpui::Window) -> Result<(), String> {
        Ok(())
    }
    fn minimize(&self, _: &gpui::Window) -> Result<(), String> {
        Ok(())
    }
    fn maximize(&self, _: &gpui::Window) -> Result<(), String> {
        Ok(())
    }
    fn restore(&self, _: &gpui::Window) -> Result<(), String> {
        Ok(())
    }
    fn is_visible(&self, _: &gpui::Window) -> Result<bool, String> {
        Ok(true)
    }
    fn is_minimized(&self, _: &gpui::Window) -> Result<bool, String> {
        Ok(false)
    }
}

pub struct Scenario {
    pub host: Host,
    pub main: MainWindow,
    pub workspace: Folder,
    data: Folder,
    events: UnboundedReceiver<ForegroundMessage>,
    services: Services,
    controller: bool,
}
impl Scenario {
    pub fn new(cx: &mut TestAppContext, controller: bool, seed: impl FnOnce(&Host)) -> Self {
        static CONTENT: std::sync::Once = std::sync::Once::new();
        CONTENT.call_once(|| {
            orbit_host::initialize_sample_content(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/Sample Content"),
            )
            .unwrap()
        });
        cx.executor().allow_parking();
        let data = Folder::new();
        let workspace = Folder::new();
        std::fs::write(
            data.0.join("settings.json"),
            json!({"presenterOrbitWorkspacePath": workspace.0.join("workspace.json"),"audioSettings":{"main":{"enabled":false},"sdiNdi":{"enabled":false}}}).to_string(),
        )
        .unwrap();
        let (host, events, services) = Self::host(&data, controller);
        seed(&host);
        let main = Self::window(cx, &host, &services);
        let mut app = Self {
            host,
            main,
            workspace,
            data,
            events,
            services,
            controller,
        };
        app.ready(cx);
        app
    }
    fn host(
        data: &Folder,
        controller: bool,
    ) -> (Host, UnboundedReceiver<ForegroundMessage>, Services) {
        let (sender, receiver) = unbounded();
        let (host, services) = crate::shell::build_host_with(
            sender.clone(),
            Some(data.0.clone()),
            Arc::new(Desktop),
            Arc::new(Desktop),
            controller,
        );
        if controller {
            let executable = PathBuf::from(
                std::env::var_os("PRESENTER_ORBIT_E2E_EXECUTABLE")
                    .expect("Run npm run test:e2e to locate the output executable"),
            );
            assert!(
                executable.is_file(),
                "E2E output executable is missing: {}",
                executable.display(),
            );
            crate::shell::start_output_link_with(&host, sender, "normal".into(), |events| {
                Arc::new(crate::output_process::client::ProcessSpawner {
                    executable,
                    extra_arguments: vec!["--inert".into()],
                    events,
                })
            })
            .unwrap();
        }
        (host, receiver, services)
    }
    fn window(cx: &mut TestAppContext, host: &Host, services: &Services) -> MainWindow {
        cx.update(|cx| {
            cx.set_global(orbit_ui::Translator::new("en"));
            orbit_ui::init_theme(cx, Default::default(), gpui::WindowAppearance::Dark);
            let control = std::rc::Rc::new(Control);
            crate::auxiliary::AuxiliaryWindows::install(
                cx,
                host.clone(),
                services.registry.clone(),
                control.clone(),
            );
            crate::open_main_window_sized(
                cx,
                host.clone(),
                None,
                super::super::ShellServices {
                    control,
                    minimize_to_tray: services.minimize_to_tray.clone(),
                },
                (1440., 920.),
            )
            .unwrap()
        })
    }
    pub fn runtime(&self, cx: &mut TestAppContext) -> crate::state::runtime::Runtime {
        cx.update(|cx| self.main.view.read(cx).runtime.clone())
    }
    pub fn visual(&self, cx: &TestAppContext) -> VisualTestContext {
        VisualTestContext::from_window(self.main.window, cx)
    }
    pub fn pump(&mut self, cx: &mut TestAppContext) {
        self.pump_until(cx, Instant::now() + Duration::from_secs(20), false);
    }
    fn pump_until(&mut self, cx: &mut TestAppContext, deadline: Instant, animated: bool) {
        let drain = |cx: &mut TestAppContext| {
            let mut ticks = 0;
            let mut slowest = Duration::ZERO;
            let slice_started = Instant::now();
            loop {
                let started = Instant::now();
                if !cx.executor().tick() {
                    break;
                }
                ticks += 1;
                slowest = slowest.max(started.elapsed());
                // A playing preview always has more work; sample its observable result between bounded slices.
                if animated && slice_started.elapsed() >= Duration::from_millis(10) {
                    break;
                }
                if Instant::now() >= deadline {
                    let diagnostic = cx.update(|cx| {
                        let runtime = &self.main.view.read(cx).runtime;
                        let output = runtime.output.read(cx);
                        json!({"outputError":output.error,"deviceError":output.device_error,"revision":output.committed_revision,
                            "contentGeneration":runtime.content.read(cx).items_generation,"requests":runtime.sent.borrow().len()})
                    });
                    panic!(
                        "E2E executor did not become idle before the wait deadline: {diagnostic}, ticks={ticks}, slowest={slowest:?}"
                    );
                }
            }
        };
        drain(cx);
        // Check the awaited state between host events, even if new events keep arriving.
        if let Ok(message) = self.events.try_recv() {
            cx.update(|cx| crate::foreground::handle_message(cx, &self.main, message));
            drain(cx);
        }
    }
    #[track_caller]
    pub fn wait(
        &mut self,
        cx: &mut TestAppContext,
        done: impl Fn(&Self, &mut TestAppContext) -> bool,
    ) {
        self.wait_with(cx, done, false);
    }
    #[track_caller]
    pub fn wait_animated(
        &mut self,
        cx: &mut TestAppContext,
        done: impl Fn(&Self, &mut TestAppContext) -> bool,
    ) {
        self.wait_with(cx, done, true);
    }
    #[track_caller]
    fn wait_with(
        &mut self,
        cx: &mut TestAppContext,
        done: impl Fn(&Self, &mut TestAppContext) -> bool,
        animated: bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.pump_until(cx, deadline, animated);
            if done(self, cx) {
                return;
            }
            if Instant::now() >= deadline {
                let runtime = self.runtime(cx);
                let diagnostic=cx.update(|cx|json!({"status":runtime.status.read(cx).message,"outputError":runtime.output.read(cx).error,"linkError":runtime.output.read(cx).link_error,"deviceError":runtime.output.read(cx).device_error,"audioId":runtime.output.read(cx).live["audioId"],"audioSelectionId":runtime.output.read(cx).live["audioSelectionId"]}));
                let log = std::fs::read_to_string(
                    crate::output_process::protocol::data_directory(&self.data.0)
                        .join("output-process.log"),
                );
                panic!("E2E condition timed out: {diagnostic}; output log: {log:?}");
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn ready(&mut self, cx: &mut TestAppContext) {
        self.wait(cx, |app, cx| {
            app.visual(cx)
                .update(|window, cx| window.simulate_next_frame(cx));
            let runtime = app.runtime(cx);
            cx.update(|cx| {
                !runtime.content.read(cx).workspace_path.is_empty()
                    && !app.main.view.read(cx).first_page_pending
            })
        });
    }
    pub fn snapshot(&self) -> Value {
        self.host
            .invoke(
                "readWorkspaceSnapshot",
                vec![json!(self.workspace.0.join("workspace.json"))],
            )
            .unwrap()["workspace"]
            .clone()
    }
    pub fn restart(&mut self, cx: &mut TestAppContext) {
        cx.update(|cx| {
            for window in cx.windows() {
                window
                    .update(cx, |_, window, _| window.remove_window())
                    .unwrap();
            }
        });
        self.stop();
        let (host, events, services) = Self::host(&self.data, self.controller);
        self.host = host;
        self.events = events;
        self.services = services;
        self.main = Self::window(cx, &self.host, &self.services);
        self.ready(cx);
    }
    fn stop(&self) {
        if let Some(route) = self
            .host
            .try_state::<crate::output_process::OutputRoute>()
            .and_then(|route| route.0.get())
        {
            route.shutdown();
        }
        orbit_host::stop_host(&self.host);
        self.services.outputs.shutdown();
    }
}
impl Drop for Scenario {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn primary_shortcut(visual: &mut VisualTestContext, keys: &str) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    visual.simulate_keystrokes(&format!("{modifier}-{keys}"));
}

pub fn click(visual: &mut VisualTestContext, selector: &str) {
    let selector = Box::leak(selector.to_string().into_boxed_str());
    let point = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("'{selector}' was not painted"))
        .center();
    visual.simulate_click(point, Modifiers::default());
    visual.run_until_parked();
}

pub fn hold_workspace_writes(
    app: &Scenario,
    cx: &mut TestAppContext,
) -> std::sync::mpsc::Sender<()> {
    let runtime = app.runtime(cx);
    let path = cx.update(|cx| runtime.content.read(cx).workspace_path.clone());
    let (release, blocked) = std::sync::mpsc::channel();
    let _answer = runtime.lanes.command(&path, move || {
        blocked.recv().map_err(|error| error.to_string())?;
        Ok(Value::Null)
    });
    release
}
pub fn context(visual: &mut VisualTestContext, selector: &str) {
    let selector = Box::leak(selector.to_string().into_boxed_str());
    let point = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("'{selector}' was not painted"))
        .center();
    visual.simulate_mouse_move(point, None, Modifiers::default());
    visual.simulate_mouse_down(point, MouseButton::Right, Modifiers::default());
    visual.simulate_mouse_up(point, MouseButton::Right, Modifiers::default());
    visual.run_until_parked();
}

pub fn drag_row(
    visual: &mut VisualTestContext,
    source: &str,
    target: &str,
    after: bool,
    indicator: &str,
) {
    let source = Box::leak(source.to_string().into_boxed_str());
    let target = Box::leak(target.to_string().into_boxed_str());
    let indicator = Box::leak(indicator.to_string().into_boxed_str());
    let from = visual.debug_bounds(source).unwrap().center();
    let bounds = visual.debug_bounds(target).unwrap();
    let to = gpui::point(
        bounds.center().x,
        if after {
            bounds.bottom() - gpui::px(2.)
        } else {
            bounds.top() + gpui::px(2.)
        },
    );
    visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(
        from + gpui::point(gpui::px(10.), gpui::px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds(indicator).is_some());
    visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    let source = visual.debug_bounds(source).unwrap();
    let target = visual.debug_bounds(target).unwrap();
    assert!(if after {
        source.top() >= target.bottom()
    } else {
        source.bottom() <= target.top()
    });
}
