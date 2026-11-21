//! Recording questions wait on an automation worker outside the workspace lane.

use std::collections::VecDeque;
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use gpui::{div, prelude::*, Context, Window};
use orbit_ui::controls::{ActionButton, ButtonVariant, Dialog};
use orbit_ui::{ActiveTheme, Translator};

use crate::state::runtime::Runtime;

/// How long a job waits for the operator before it gives up.
pub const WAIT: Duration = Duration::from_secs(300);

pub struct Request {
    /// The translation keys of the question and of the button that confirms it.
    pub message: &'static str,
    pub confirm: &'static str,
    answer: Sender<bool>,
}

/// The job side: asks and blocks.
#[derive(Clone)]
pub struct Confirmer(UnboundedSender<Request>);

impl Confirmer {
    pub fn ask(
        &self,
        message: &'static str,
        confirm: &'static str,
        wait: Duration,
    ) -> Result<bool, String> {
        let (answer, answered) = channel();
        self.0
            .unbounded_send(Request {
                message,
                confirm,
                answer,
            })
            .map_err(|_| "The confirmation dialog is unavailable.".to_string())?;
        answered
            .recv_timeout(wait)
            .map_err(|_| "The confirmation was not answered.".to_string())
    }
}

pub fn channel_pair() -> (Confirmer, UnboundedReceiver<Request>) {
    let (sender, receiver) = unbounded();
    (Confirmer(sender), receiver)
}

/// The UI side: the questions that wait, the first one shown.
#[derive(Default)]
pub struct Confirm {
    queue: VecDeque<Request>,
}

impl Confirm {
    pub fn push(&mut self, request: Request, cx: &mut Context<Self>) {
        self.queue.push_back(request);
        cx.notify();
    }

    pub fn answer(&mut self, accepted: bool, cx: &mut Context<Self>) {
        if let Some(request) = self.queue.pop_front() {
            // A job that stopped waiting has dropped its end; there is nobody left to tell.
            let _ = request.answer.send(accepted);
        }
        cx.notify();
    }
}

impl Render for Confirm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t: Translator = Runtime::translator(cx);
        let Some(request) = self.queue.front() else {
            return div().into_any_element();
        };
        let gap = cx.theme().metrics.space_xs;
        let weak = cx.weak_entity();
        let decline = move |_, _: &mut Window, cx: &mut gpui::App| {
            let _ = weak.update(cx, |this, cx| this.answer(false, cx));
        };
        Dialog::new(
            "confirm",
            t.t(request.confirm),
            t.t("common.close"),
            decline,
        )
        .width(gpui::px(420.))
        .close_on_backdrop(false)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(gap)
                .child(div().child(t.t(request.message)))
                .child(
                    div()
                        .flex()
                        .gap(gap)
                        .child(
                            ActionButton::new("confirm-accept", t.t(request.confirm))
                                .variant(ButtonVariant::Danger)
                                .on_click(cx.listener(|this, _, _, cx| this.answer(true, cx))),
                        )
                        .child(
                            ActionButton::new("confirm-decline", t.t("common.cancel"))
                                .variant(ButtonVariant::Secondary)
                                .on_click(cx.listener(|this, _, _, cx| this.answer(false, cx))),
                        ),
                ),
        )
        .into_any_element()
    }
}
