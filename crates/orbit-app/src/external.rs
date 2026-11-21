//! A controller-only external slideshow session. Program output never starts or controls office applications.
use crate::state::runtime::Runtime;
use gpui::{div, prelude::*, App, Entity, Global, SharedString};
use orbit_ui::{controls::ActionButton, ActiveTheme};
use serde_json::{json, Value};
#[derive(Default)]
pub struct Session {
    pub status: Value,
    pub error: Option<String>,
    dismissed: Option<String>,
    serial: u64,
    pending: usize,
}
pub struct External(pub Entity<Session>);
impl Global for External {}
impl Session {
    pub fn accept(&mut self, result: Result<Value, String>, serial: u64) {
        if serial != self.serial {
            return;
        }
        match result {
            Ok(v) => {
                self.error = v["error"]
                    .as_str()
                    .filter(|e| self.dismissed.as_deref() != Some(e))
                    .map(str::to_string);
                self.status = v;
            }
            Err(e) => self.error = Some(e),
        }
    }
    pub fn running(&self) -> bool {
        self.status["running"] == true
    }
    pub fn dismiss(&mut self) {
        self.dismissed = self.error.take();
    }
}
pub fn publish(status: &Value, error: Option<&str>, cx: &mut App) {
    if (status.is_null() && error.is_none()) || !cx.has_global::<External>() {
        return;
    }
    let session = cx.global::<External>().0.clone();
    session.update(cx, |s, cx| {
        s.serial += 1;
        s.dismissed = None;
        let result = if let Some(e) = error {
            Err(e.to_string())
        } else {
            Ok(status.clone())
        };
        s.accept(result, s.serial);
        cx.notify();
    });
}
pub fn start(runtime: Runtime, cx: &mut App) -> Entity<Session> {
    let session = cx.new(|_| Session::default());
    cx.set_global(External(session.clone()));
    let weak = session.downgrade();
    cx.spawn(async move |cx| loop {
        let Ok((serial, pending, running)) =
            weak.update(cx, |s, _| (s.serial, s.pending, s.running()))
        else {
            break;
        };
        if pending > 0 || !running {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(1))
                .await;
            continue;
        }
        let host = runtime.host.clone();
        let result = cx
            .background_spawn(async move { host.invoke("getExternalPresentationStatus", vec![]) })
            .await;
        if weak
            .update(cx, |s, cx| {
                s.accept(result, serial);
                cx.notify();
            })
            .is_err()
        {
            break;
        }
        cx.background_executor()
            .timer(std::time::Duration::from_secs(1))
            .await;
    })
    .detach();
    session
}
pub fn send(runtime: &Runtime, command: Value, cx: &mut App) {
    if !cx.has_global::<External>() {
        return;
    }
    let session = cx.global::<External>().0.clone();
    let serial = session.update(cx, |s, _| {
        s.serial += 1;
        s.pending += 1;
        s.dismissed = None;
        s.serial
    });
    let host = runtime.host.clone();
    let answer = runtime.lanes.command("external-presentation", move || {
        host.invoke("controlExternalPresentation", vec![command])
    });
    cx.spawn(async move |cx| {
        let result = answer
            .await
            .unwrap_or_else(|_| Err("External presentation command worker stopped.".into()));
        session.update(cx, |s, cx| {
            s.pending = s.pending.saturating_sub(1);
            s.accept(result, serial);
            cx.notify();
        });
    })
    .detach();
}
pub fn step(runtime: &Runtime, direction: i8, cx: &mut App) -> bool {
    if !cx.has_global::<External>() || !cx.global::<External>().0.read(cx).running() {
        return false;
    }
    send(
        runtime,
        json!({"type":if direction>0{"next"}else{"previous"}}),
        cx,
    );
    true
}
pub fn strip(runtime: &Runtime, session: &Entity<Session>, cx: &mut App) -> gpui::AnyElement {
    let t = Runtime::translator(cx);
    let theme = cx.theme();
    let state = session.read(cx);
    let mut bar = div()
        .flex()
        .items_center()
        .gap(theme.metrics.space_sm)
        .text_size(theme.metrics.font_sm);
    if !state.running() && state.error.is_none() {
        return bar.into_any_element();
    }
    bar = bar.p(theme.metrics.space_xs).bg(theme.colors.surface_bg);
    if state.running() {
        let file = state.status["file"]
            .as_str()
            .unwrap_or("")
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or("");
        bar = bar
            .child(crate::services::text(&state.status, "application"))
            .child(file.to_string())
            .child(format!(
                "{} / {}",
                state.status["currentSlide"], state.status["slideCount"]
            ));
        for action in ["previous", "next", "stop"] {
            let runtime = runtime.clone();
            bar = bar.child(
                ActionButton::new(
                    SharedString::from(format!("external-{action}")),
                    t.t(&format!("externalPresentation.{action}")),
                )
                .on_click(move |_, _, cx| send(&runtime, json!({"type":action}), cx)),
            );
        }
    }
    if let Some(error) = &state.error {
        let session = session.clone();
        bar = bar
            .child(
                div()
                    .id("external-error")
                    .role(gpui::Role::Alert)
                    .child(error.clone()),
            )
            .child(
                ActionButton::new("external-dismiss", t.t("externalPresentation.dismiss"))
                    .on_click(move |_, _, cx| {
                        session.update(cx, |s, cx| {
                            s.pending = s.pending.saturating_sub(1);
                            s.dismiss();
                            cx.notify();
                        })
                    }),
            );
    }
    bar.into_any_element()
}
