//! The workspace prompt window, a port of `WorkspacePromptWindow.tsx` and `WorkspaceNameDialog.tsx`: it names a new,
//! renamed, or duplicated workspace and confirms a delete. Manage Workspaces opens it through [`super::request_prompt`].

use gpui::{div, prelude::*, App, Context, Entity, SharedString, Subscription, Window};
use orbit_ui::controls::{
    input_state, ActionButton, ButtonVariant, Dialog, EmptyState, Field, InputEvent, InputState,
    TextInput,
};
use orbit_ui::{ActiveTheme, Translator};

use super::{Prompt, SharedWorkspaces, Workspaces};
use crate::auxiliary::AuxiliaryWindows;
use crate::windows::Kind;

/// The name field of the prompt that is showing, which a new request replaces.
struct NameField {
    state: Entity<InputState>,
    _enter: Subscription,
}

pub struct PromptView {
    workspaces: Entity<Workspaces>,
    request: Option<Prompt>,
    /// The name of the workspace the request names, or empty for a new one.
    name: String,
    /// Counts requests, so each one builds its own name field and clears the last error.
    generation: u64,
    field: Option<(u64, NameField)>,
    error: Option<String>,
    busy: bool,
}

impl PromptView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            workspaces: cx.global::<SharedWorkspaces>().0.clone(),
            request: None,
            name: String::new(),
            generation: 0,
            field: None,
            error: None,
            busy: false,
        }
    }

    /// `onPromptRequested`: a new request replaces whatever the window showed before.
    pub fn request(&mut self, value: &str, cx: &mut Context<Self>) {
        self.generation += 1;
        self.busy = false;
        self.request = None;
        self.error = None;
        let t = *cx.global::<Translator>();
        let parsed = Prompt::parse(value).and_then(|request| {
            let path = match &request {
                Prompt::New => None,
                Prompt::Rename(path) | Prompt::Duplicate(path) | Prompt::Delete(path) => Some(path),
            };
            let name = match path {
                None => String::new(),
                Some(path) => self
                    .workspaces
                    .read(cx)
                    .items
                    .iter()
                    .find(|item| &item.path == path)
                    .map(|item| item.name.clone())
                    .ok_or_else(|| t.t("workspace.missing"))?,
            };
            Ok((request, name))
        });
        match parsed {
            Ok((request, name)) => {
                self.request = Some(request);
                self.name = name;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn close(cx: &mut App) {
        let dismissed = cx
            .global::<AuxiliaryWindows>()
            .dismiss(Kind::WorkspacePrompt)
            .and_then(|_| AuxiliaryWindows::hide(cx, Kind::WorkspacePrompt));
        if let Err(error) = dismissed {
            eprintln!("[workspaces] {error}");
        }
    }

    pub(super) fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.request.clone() else {
            return;
        };
        let name = match (&request, &self.field) {
            (Prompt::Delete(_), _) => String::new(),
            (_, Some((_, field))) => field.state.read(cx).value().trim().to_string(),
            _ => return,
        };
        if self.busy || (!matches!(request, Prompt::Delete(_)) && name.is_empty()) {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let weak = cx.weak_entity();
        let done = move |result: Result<(), String>, cx: &mut App| {
            let failed = weak
                .update(cx, |view, cx| {
                    view.busy = false;
                    view.error = result.clone().err();
                    cx.notify();
                })
                .is_err();
            if result.is_ok() || failed {
                Self::close(cx);
            }
        };
        self.workspaces.update(cx, |workspaces, cx| match &request {
            Prompt::New => workspaces.create(&name, cx, done),
            Prompt::Rename(path) => workspaces.rename(path, &name, cx, done),
            Prompt::Duplicate(path) => workspaces.duplicate(path, &name, cx, done),
            Prompt::Delete(path) => workspaces.delete(path, cx, done),
        });
    }

    fn name_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((generation, field)) = &self.field {
            if *generation == self.generation {
                return field.state.clone();
            }
        }
        let state = input_state(self.name.clone(), window, cx);
        let enter = cx.subscribe(&state, |view, _, event: &InputEvent, cx| {
            match event {
                InputEvent::PressEnter { .. } => view.submit(cx),
                // The submit button is disabled while the name is empty.
                InputEvent::Change => cx.notify(),
                _ => {}
            }
        });
        let focus = gpui::Focusable::focus_handle(state.read(cx), cx);
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        self.field = Some((
            self.generation,
            NameField {
                state: state.clone(),
                _enter: enter,
            },
        ));
        state
    }
}

impl Render for PromptView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let close = |_: (), _: &mut Window, cx: &mut App| Self::close(cx);
        let Some(request) = self.request.clone() else {
            // A request that could not be read shows why, in a window that can be closed.
            return Dialog::new(
                "workspace-prompt-error",
                t.t("workspace.manageTitle"),
                t.t("common.close"),
                close,
            )
            .standalone()
            .child(EmptyState::new(
                self.error
                    .clone()
                    .unwrap_or_else(|| t.t("workspace.missing")),
            ))
            .into_any_element();
        };
        let (title, submit_label) = match &request {
            Prompt::New => (t.t("workspace.newTitle"), t.t("workspace.create")),
            Prompt::Rename(_) => (t.t("workspace.renameTitle"), t.t("common.rename")),
            Prompt::Duplicate(_) => (t.t("workspace.duplicateTitle"), t.t("workspace.duplicate")),
            Prompt::Delete(_) => (t.t("workspace.remove"), t.t("workspace.remove")),
        };
        let deleting = matches!(request, Prompt::Delete(_));
        let body = if deleting {
            div().child(t.format("workspace.removeConfirm", &[("name", &self.name)]))
        } else {
            let state = self.name_field(window, cx);
            div().child(Field::new(
                t.t("workspace.name"),
                TextInput::new("workspace-name", &state),
            ))
        };
        let empty = !deleting
            && self
                .field
                .as_ref()
                .is_none_or(|(_, field)| field.state.read(cx).value().trim().is_empty());
        let weak = cx.weak_entity();
        Dialog::new(
            SharedString::from(if deleting {
                "workspace-delete"
            } else {
                "workspace-name"
            }),
            title,
            t.t("common.cancel"),
            close,
        )
        .standalone()
        .footer(
            ActionButton::new("workspace-prompt-cancel", t.t("common.cancel"))
                .variant(ButtonVariant::Secondary)
                .on_click(move |_, _, cx| Self::close(cx)),
        )
        .footer(
            ActionButton::new("workspace-prompt-submit", submit_label)
                .variant(if deleting {
                    ButtonVariant::Danger
                } else {
                    ButtonVariant::Primary
                })
                .disabled(self.busy || empty)
                .on_click(move |_, _, cx| {
                    weak.update(cx, |view, cx| view.submit(cx)).ok();
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme.metrics.space_md)
                .child(body)
                .children(
                    self.error
                        .clone()
                        .map(|error| div().text_color(theme.colors.danger).child(error)),
                ),
        )
        .into_any_element()
    }
}
