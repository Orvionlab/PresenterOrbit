//! Text and number fields of the panels. A field commits on Enter and on blur, as the React
//! `CommittedInput` does, and is rebuilt when the saved value it shows changes under it.

use std::collections::HashMap;
use std::rc::Rc;

use super::ui::Field;
use gpui::{prelude::*, AnyElement, Context, Entity, SharedString, Subscription, Window};
use orbit_ui::controls::{
    input_state, password_state, IconButton, InputEvent, InputState, NumberInput, TextInput,
};
use orbit_ui::ActiveTheme as _;

use super::view::SettingsView;

pub type Commit = Rc<dyn Fn(&mut SettingsView, String, &mut Context<SettingsView>)>;

pub struct FieldInput {
    state: Entity<InputState>,
    /// What the field was built from, so a changed saved value rebuilds it.
    source: String,
    _subscription: Subscription,
}

#[derive(Default)]
pub struct Inputs(
    HashMap<String, FieldInput>,
    std::collections::HashSet<String>,
    /// Set by a panel while it builds a disabled group of fields.
    pub bool,
);

impl SettingsView {
    pub(super) fn field_state(
        &mut self,
        key: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        commit: Commit,
        masked: bool,
    ) -> Entity<InputState> {
        if let Some(existing) = self.inputs.0.get(key) {
            if existing.source == value {
                return existing.state.clone();
            }
        }
        let state = if masked {
            password_state(value.to_string(), window, cx)
        } else {
            input_state(value.to_string(), window, cx)
        };
        let source = value.to_string();
        let compare = source.clone();
        let subscription = cx.subscribe_in(
            &state,
            window,
            move |this, state, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    return;
                }
                let text = state.read(cx).value().to_string();
                if text != compare {
                    commit(this, text, cx);
                }
            },
        );
        self.inputs.0.insert(
            key.to_string(),
            FieldInput {
                state: state.clone(),
                source,
                _subscription: subscription,
            },
        );
        state
    }

    /// A labelled text field that commits on Enter and blur.
    pub(super) fn text_field(
        &mut self,
        key: &str,
        label: impl Into<SharedString>,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        commit: impl Fn(&mut SettingsView, String, &mut Context<SettingsView>) + 'static,
    ) -> AnyElement {
        let control = self.text_control(key, value, window, cx, commit);
        Field::new(label, control).into_any_element()
    }

    /// The text field of [`Self::text_field`] without its label, for rows that place the label themselves.
    pub(super) fn text_control(
        &mut self,
        key: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        commit: impl Fn(&mut SettingsView, String, &mut Context<SettingsView>) + 'static,
    ) -> AnyElement {
        let state = self.field_state(key, value, window, cx, Rc::new(commit), false);
        TextInput::new(SharedString::from(key.to_string()), &state)
            .disabled(self.inputs.2)
            .into_any_element()
    }

    /// A labelled number field with steppers; typed text commits on Enter and blur, a stepper press at once.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn number_field(
        &mut self,
        key: &str,
        label: impl Into<SharedString>,
        value: &str,
        range: (f64, f64),
        step: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
        commit: impl Fn(&mut SettingsView, String, &mut Context<SettingsView>) + 'static,
    ) -> AnyElement {
        let control = self.number_control(key, value, range, step, window, cx, commit);
        Field::new(label, control).into_any_element()
    }

    /// The number field of [`Self::number_field`] without its label.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn number_control(
        &mut self,
        key: &str,
        value: &str,
        range: (f64, f64),
        step: f64,
        window: &mut Window,
        cx: &mut Context<Self>,
        commit: impl Fn(&mut SettingsView, String, &mut Context<SettingsView>) + 'static,
    ) -> AnyElement {
        let commit: Commit = Rc::new(commit);
        let state = self.field_state(key, value, window, cx, commit.clone(), false);
        let weak = cx.weak_entity();
        NumberInput::new(SharedString::from(key.to_string()), &state)
            .range(range.0, range.1)
            .step(step)
            .stepper()
            .disabled(self.inputs.2)
            .on_step(move |value, _, cx| {
                let text = value.to_string();
                let commit = commit.clone();
                weak.update(cx, |view, cx| commit(view, text, cx)).ok();
            })
            .into_any_element()
    }
}

impl SettingsView {
    /// A masked field with a show and hide button, committing on Enter and blur.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn password_field(
        &mut self,
        key: &str,
        label: impl Into<SharedString>,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        commit: impl Fn(&mut SettingsView, String, &mut Context<SettingsView>) + 'static,
    ) -> AnyElement {
        let label: SharedString = label.into();
        let control = self.password_control(key, label.clone(), value, window, cx, commit);
        Field::new(label, control).into_any_element()
    }

    /// The password field of [`Self::password_field`] without its label; `label` names the reveal button.
    pub(super) fn password_control(
        &mut self,
        key: &str,
        label: impl Into<SharedString>,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        commit: impl Fn(&mut SettingsView, String, &mut Context<SettingsView>) + 'static,
    ) -> AnyElement {
        let t = *cx.global::<orbit_ui::Translator>();
        let label: SharedString = label.into();
        let state = self.field_state(key, value, window, cx, Rc::new(commit), true);
        let revealed = self.inputs.1.contains(key);
        state.update(cx, |state, cx| state.set_masked(!revealed, window, cx));
        let owner = key.to_string();
        let toggle = IconButton::new(
            SharedString::from(format!("{key}-reveal")),
            if revealed { "eye-off" } else { "eye" },
            format!(
                "{}: {label}",
                t.t(if revealed {
                    "settings.remote.hidePassword"
                } else {
                    "settings.remote.showPassword"
                })
            ),
        )
        .selected(revealed)
        .on_click(cx.listener(move |view, _, _, cx| {
            if !view.inputs.1.remove(&owner) {
                view.inputs.1.insert(owner.clone());
            }
            cx.notify();
        }));
        gpui::div()
            .flex()
            .items_center()
            .gap(cx.theme().metrics.space_xs)
            .child(gpui::div().flex_1().child(
                TextInput::new(SharedString::from(key.to_string()), &state).disabled(self.inputs.2),
            ))
            .child(toggle)
            .into_any_element()
    }
}
