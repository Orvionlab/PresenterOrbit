//! The macro palette, the audience message list, and the countdown timer list of the utility panel, with their
//! authoring editors. Every run button is the same semantic action React runs (`macro`, `message`,
//! `clear messages`, `timer`), so it goes through the one runner. Editing works on a JSON draft in a [`Form`];
//! saving validates like React, then queues one workspace write.

use gpui::{div, prelude::*, px, Context, Entity, FocusHandle, KeyDownEvent, SharedString, Window};
use orbit_ui::controls::{
    list_row, ActionButton, ButtonVariant, CollectionAction, CollectionOption, CollectionPicker,
    EmptyState, Field, FilterInput, IconButton, InputEvent, InputState, PanelHeader,
};
use orbit_ui::{ActiveTheme, Translator};
use serde_json::{json, Value};

use super::action_list;
use super::editors;
use super::form::Form;
use super::macros::{self, Clipboard, Library};
use super::validate;
use crate::state::runtime::Runtime;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Macros,
    Messages,
    Timers,
}

struct Editor {
    panel: Panel,
    /// The saved entry the editor opened, `Null` for a new one.
    original: Value,
    /// The `(id, name)` of the message tokens at the start, to follow a rename.
    tokens: Vec<(String, String)>,
}

#[derive(Clone)]
struct MessageDrag {
    id: String,
    title: String,
}

impl Render for MessageDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px(theme.metrics.space_sm)
            .py(theme.metrics.space_2xs)
            .bg(theme.colors.popover_bg)
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_sm)
            .child(self.title.clone())
    }
}

use super::save::Build;

type PanelEdit = (
    &'static str,
    &'static str,
    Box<dyn Fn(&mut Panels, &str, &mut Context<Panels>)>,
);
pub struct Panels {
    runtime: Runtime,
    pub panel: Panel,
    form: Form,
    editor: Option<Editor>,
    status: String,
    busy: bool,
    pub collection: String,
    clipboard: Option<Clipboard>,
    hotkey_focus: FocusHandle,
    menu: Option<String>,
    show_after_save: bool,
    message_autosave: Option<Value>,
    message_drop: Option<(String, bool)>,
    filter: Option<Entity<InputState>>,
    query: String,
    table: bool,
    details_open: bool,
    images:
        std::collections::HashMap<String, (String, Result<std::sync::Arc<gpui::Image>, String>)>,
}

fn draft_form(panels: &mut Panels) -> &mut Form {
    &mut panels.form
}

fn button(
    id: impl Into<SharedString>,
    label: String,
    variant: ButtonVariant,
    cx: &mut Context<Panels>,
    act: impl Fn(&mut Panels, &mut Context<Panels>) + 'static,
) -> ActionButton {
    ActionButton::new(id.into(), label)
        .variant(variant)
        .on_click(cx.listener(move |panels, _, _, cx| act(panels, cx)))
}

impl Panels {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        cx.observe(&runtime.content, |this, _, cx| {
            this.images.clear();
            cx.notify();
        })
        .detach();
        cx.observe(&runtime.output, |_, _, cx| cx.notify()).detach();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;
            if this
                .update(cx, |this, cx| {
                    if this.panel == Panel::Timers {
                        cx.notify();
                    }
                    this.autosave_message(cx);
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
        Self {
            runtime,
            panel: Panel::Macros,
            form: Form::default(),
            editor: None,
            status: String::new(),
            busy: false,
            collection: String::new(),
            clipboard: None,
            hotkey_focus: cx.focus_handle(),
            menu: None,
            show_after_save: false,
            message_autosave: None,
            message_drop: None,
            filter: None,
            query: String::new(),
            table: false,
            details_open: true,
            images: std::collections::HashMap::new(),
        }
    }

    pub fn run(&self, actions: Vec<Value>, cx: &mut Context<Self>) {
        crate::automation::run_operator(&self.runtime, actions, cx);
    }

    fn workspace(&self, cx: &Context<Self>) -> std::sync::Arc<Value> {
        self.runtime.content.read(cx).workspace.clone()
    }

    fn rows(&self, cx: &Context<Self>) -> (Vec<(String, String)>, &'static str) {
        let workspace = self.workspace(cx);
        let (key, empty) = match self.panel {
            Panel::Macros => ("macros", "automation.macros.empty"),
            Panel::Messages => ("messages", "automation.messages.empty"),
            Panel::Timers => ("countdownTimers", "timer.empty"),
        };
        let mut entries: Vec<Value> = if self.panel == Panel::Macros {
            macros::collection_macros(&Library::of(&workspace), &self.active_collection(cx))
                .unwrap_or_default()
        } else {
            workspace[key].as_array().cloned().unwrap_or_default()
        };
        if self.panel == Panel::Messages && self.editor.is_some() {
            if let Some(entry) = entries
                .iter_mut()
                .find(|entry| entry["id"] == self.form.draft["id"])
            {
                *entry = self.form.draft.clone();
            } else if self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.original.is_null())
            {
                entries.push(self.form.draft.clone());
            }
        }
        let query = self.query.trim().to_lowercase();
        let rows = entries
            .iter()
            .filter(|entry| {
                self.panel != Panel::Macros
                    || entry["title"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query)
            })
            .filter_map(|entry| {
                Some((
                    entry["id"].as_str()?.to_string(),
                    entry["title"]
                        .as_str()
                        .or_else(|| entry["name"].as_str())?
                        .to_string(),
                ))
            })
            .collect();
        (rows, empty)
    }

    fn active_collection(&self, cx: &Context<Self>) -> String {
        macros::active_collection(
            Library::of(&self.workspace(cx)).collections,
            &self.collection,
        )
    }

    /// The actions a button of a row sends.
    pub fn action(panel: Panel, id: &str, command: &str, live_message: &Value) -> Value {
        match (panel, command) {
            (Panel::Macros, _) => {
                json!({"id": format!("palette-{id}"), "type": "macro", "targetId": id})
            }
            (Panel::Messages, "clear") => {
                json!({"id": format!("clear-{id}"), "type": "clear", "targetLayer": "messages",
                "details": {"expectedMessageInstanceId": live_message}})
            }
            (Panel::Messages, _) => {
                json!({"id": format!("show-{id}"), "type": "message", "targetId": id})
            }
            (Panel::Timers, command) => {
                json!({"id": format!("timer-{id}"), "type": "timer", "targetId": id, "value": command})
            }
        }
    }

    // ----- saving -----

    fn autosave_message(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self
            .editor
            .as_ref()
            .filter(|editor| editor.panel == Panel::Messages)
        else {
            return;
        };
        if self.busy {
            return;
        }
        let mut draft = self.form.draft.clone();
        editors::sync_token_names(&mut draft, &editor.tokens);
        let saved = editors::message_to_save(&draft);
        if saved == editor.original
            || self.message_autosave.as_ref() == Some(&saved)
            || validate::message_error(&self.workspace(cx), &draft).is_some()
        {
            return;
        }
        self.message_autosave = Some(saved);
        self.save(cx);
    }

    /// Runs one save through [`super::save::commit`]; one save runs at a time.
    fn commit(&mut self, needs_settings: bool, close: bool, build: Build, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status.clear();
        let submitted = self.form.draft.clone();
        super::save::commit(
            &self.runtime,
            needs_settings,
            build,
            cx,
            move |this, outcome, cx| this.finished(outcome, close, &submitted, cx),
        );
        cx.notify();
    }

    fn finished(
        &mut self,
        outcome: Result<(), String>,
        close: bool,
        submitted: &Value,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        let unchanged = &self.form.draft == submitted;
        if outcome.is_ok()
            && self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.panel == Panel::Messages)
            && self.form.draft["id"] == submitted["id"]
        {
            if let Some(editor) = &mut self.editor {
                if let Some(saved) = self.runtime.content.read(cx).workspace["messages"]
                    .as_array()
                    .and_then(|rows| rows.iter().find(|row| row["id"] == self.form.draft["id"]))
                {
                    editor.original = saved.clone();
                    let mut canonical = submitted.clone();
                    editors::sync_token_names(&mut canonical, &editor.tokens);
                    if unchanged && editors::message_to_save(&canonical) == *saved {
                        self.form.set("tokenValues", saved["tokenValues"].clone());
                        editor.tokens = saved["tokens"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|token| {
                                (
                                    token["id"].as_str().unwrap_or("").to_string(),
                                    token["name"].as_str().unwrap_or("").to_string(),
                                )
                            })
                            .collect();
                    }
                }
            }
        }
        match outcome {
            Ok(()) if close && &self.form.draft == submitted => self.close_editor(),
            Ok(()) => {
                if self.show_after_save {
                    self.show_after_save = false;
                    if unchanged {
                        let id = submitted["id"].as_str().unwrap_or("").to_string();
                        self.run(
                            vec![Self::action(Panel::Messages, &id, "show", &Value::Null)],
                            cx,
                        );
                    } else {
                        self.show_after_save = true;
                        self.save(cx);
                    }
                }
            }
            Err(error) => {
                self.show_after_save = false;
                self.status = error;
            }
        }
        cx.notify();
    }

    fn close_editor(&mut self) {
        self.editor = None;
        self.show_after_save = false;
        self.message_autosave = None;
        self.status.clear();
        self.form.reset(Value::Null);
    }

    fn save_change(&mut self, change: Result<macros::Change, String>, cx: &mut Context<Self>) {
        match change {
            Ok(change) => {
                let patch = change.patch();
                if let Err(error) = super::save::optimistic_content(
                    &self.runtime,
                    patch,
                    cx,
                    |this, outcome, cx| {
                        if let Err(error) = outcome {
                            this.status = error;
                        }
                        cx.notify();
                    },
                ) {
                    self.status = error;
                }
                cx.notify();
            }
            Err(error) => {
                self.status = error;
                cx.notify();
            }
        }
    }

    // ----- macros -----

    fn open(&mut self, panel: Panel, original: Value, draft: Value) {
        self.status.clear();
        self.show_after_save = false;
        self.message_autosave = None;
        self.editor = Some(Editor {
            panel,
            tokens: draft["tokens"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|token| {
                    (
                        token["id"].as_str().unwrap_or("").to_string(),
                        token["name"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect(),
            original,
        });
        self.form.reset(draft);
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        self.autosave_message(cx);
        let id = uuid::Uuid::new_v4().to_string();
        match self.panel {
            Panel::Macros => {
                let active = self.active_collection(cx);
                let memberships = if active.is_empty() {
                    json!([])
                } else {
                    json!([active])
                };
                self.open(
                    Panel::Macros,
                    Value::Null,
                    json!({"id": id, "title": "", "actions": [], "collectionIds": memberships}),
                );
                self.form.request_focus("title");
            }
            Panel::Messages => {
                self.open(
                    Panel::Messages,
                    Value::Null,
                    json!({"id": id, "title": "", "text": "", "clearType": "manual"}),
                );
                self.details_open = true;
                self.form.request_focus("title");
            }
            Panel::Timers => self.create_timer(cx),
        }
        cx.notify();
    }

    fn edit(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.editor.is_some() && self.form.draft["id"] == id {
            return;
        }
        self.autosave_message(cx);
        let workspace = self.workspace(cx);
        let key = match self.panel {
            Panel::Macros => "macros",
            Panel::Messages => "messages",
            Panel::Timers => "countdownTimers",
        };
        if let Some(entry) = workspace[key]
            .as_array()
            .and_then(|list| list.iter().find(|entry| entry["id"] == id))
        {
            self.open(self.panel, entry.clone(), entry.clone());
        }
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let (panel, original) = (editor.panel, editor.original.clone());
        let mut draft = self.form.draft.clone();
        match panel {
            Panel::Macros => {
                let new_id = uuid::Uuid::new_v4().to_string();
                let selected = self.collection.clone();
                self.commit(
                    true,
                    true,
                    Box::new(move |runtime, settings, cx| {
                        let facts = super::facts(runtime, cx);
                        let mut workspace = (*facts.workspace).clone();
                        let ensured =
                            macros::ensure_default(&Library::of(&workspace), true, &new_id);
                        if let Some(change) = &ensured {
                            workspace["macros"] = json!(change.macros);
                            workspace["macroCollections"] = json!(change.collections);
                        }
                        let collections = Library::of(&workspace).collections.to_vec();
                        if draft["collectionIds"].as_array().is_none_or(Vec::is_empty) {
                            draft["collectionIds"] =
                                json!([macros::active_collection(&collections, &selected)]);
                        }
                        let facts = super::runner::Facts {
                            workspace: std::sync::Arc::new(workspace.clone()),
                            ..facts
                        };
                        if let Some(error) = validate::macro_error(&facts, settings, &draft) {
                            return Err(error);
                        }
                        let mut change = macros::save(
                            &Library::of(&workspace),
                            &draft,
                            validate::canonical_shortcut,
                        );
                        if ensured.is_some() && change.collections.is_none() {
                            change.collections = Some(collections);
                        }
                        Ok(change.patch())
                    }),
                    cx,
                );
            }
            Panel::Messages => {
                let tokens = self
                    .editor
                    .as_ref()
                    .map(|e| e.tokens.clone())
                    .unwrap_or_default();
                editors::sync_token_names(&mut draft, &tokens);
                self.commit(
                    false,
                    false,
                    Box::new(move |runtime, _, cx| {
                        let workspace = runtime.content.read(cx).workspace.clone();
                        if let Some(error) = validate::message_error(&workspace, &draft) {
                            return Err(error);
                        }
                        let mut messages = workspace["messages"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default();
                        let current = messages.iter().position(|entry| entry["id"] == draft["id"]);
                        let found = current.map_or(Value::Null, |index| messages[index].clone());
                        if found != original {
                            return Err("automation.messages.stale".into());
                        }
                        let saved = editors::message_to_save(&draft);
                        match current {
                            Some(index) => messages[index] = saved,
                            None => messages.push(saved),
                        }
                        Ok(json!({"messages": messages}))
                    }),
                    cx,
                );
            }
            Panel::Timers => self.save_timer(original, draft, cx),
        }
    }

    // ----- timers -----

    fn mutate_timer(
        &mut self,
        mutation: Value,
        then: impl FnOnce(&mut Self, Value, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status.clear();
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let outcome =
                super::save::mutation(&runtime, cx, "mutateCountdownTimers", mutation).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match outcome {
                    Ok(response) => {
                        let removed: Vec<String> = response["removedTimerIds"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|id| id.as_str().map(str::to_string))
                            .collect();
                        if !removed.is_empty() {
                            crate::cues::actions::edit_native(&this.runtime, cx, move |live| {
                                for id in &removed {
                                    if let Some(runs) = live["countdownTimerRuns"].as_object_mut() {
                                        runs.remove(id);
                                    }
                                }
                                Ok(())
                            });
                        }
                        then(this, response, cx);
                    }
                    Err(error) => this.status = error,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn create_timer(&mut self, cx: &mut Context<Self>) {
        let base = Runtime::translator(cx).t("timer.defaultName");
        self.mutate_timer(
            json!({"type": "create", "baseName": base}),
            |this, response, cx| {
                let id = response["createdTimerId"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                this.edit(&id, cx);
            },
            cx,
        );
    }

    fn save_timer(&mut self, original: Value, draft: Value, cx: &mut Context<Self>) {
        let mut draft = draft;
        if let Some(name) = draft["name"].as_str() {
            draft["name"] = json!(name.trim());
        }
        let Some(patch) = editors::timer_patch(&original, &draft) else {
            self.close_editor();
            cx.notify();
            return;
        };
        let id = original["id"].clone();
        self.mutate_timer(
            json!({"type": "update", "timerId": id, "patch": patch}),
            |this, _, _| this.close_editor(),
            cx,
        );
    }

    fn move_timer(&mut self, id: &str, offset: isize, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.rows(cx).0.into_iter().map(|(id, _)| id).collect();
        let Some(index) = ids.iter().position(|known| known == id) else {
            return;
        };
        let Some(target) = index
            .checked_add_signed(offset)
            .filter(|target| *target < ids.len())
        else {
            return;
        };
        self.mutate_timer(
            json!({"type": "reorder", "timerId": id, "targetTimerId": ids[target],
                "placement": if offset < 0 { "before" } else { "after" }}),
            |_, _, _| {},
            cx,
        );
    }

    fn delete_timer(&mut self, id: &str, cx: &mut Context<Self>) {
        self.mutate_timer(json!({"type": "delete", "timerId": id}), |_, _, _| {}, cx);
    }

    // ----- list operations -----

    fn reorder_message(&mut self, source: &str, target: &str, after: bool, cx: &mut Context<Self>) {
        if source == target || self.busy {
            return;
        }
        let (source, target) = (source.to_string(), target.to_string());
        self.commit(
            false,
            false,
            Box::new(move |runtime, _, cx| {
                let mut messages = runtime.content.read(cx).workspace["messages"]
                    .as_array()
                    .cloned()
                    .ok_or("Audience messages must be an array.")?;
                let from = messages
                    .iter()
                    .position(|message| message["id"] == source)
                    .ok_or("automation.messages.missing")?;
                let message = messages.remove(from);
                let to = messages
                    .iter()
                    .position(|message| message["id"] == target)
                    .ok_or("automation.messages.missing")?;
                messages.insert(to + usize::from(after), message);
                Ok(json!({"messages": messages}))
            }),
            cx,
        );
    }

    fn delete(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.panel == Panel::Messages && self.form.draft["id"] == id {
            if self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.original.is_null())
            {
                self.close_editor();
                cx.notify();
                return;
            }
            self.close_editor();
        }
        let id = id.to_string();
        match self.panel {
            Panel::Macros => {
                let workspace = self.workspace(cx);
                let change = macros::delete(&Library::of(&workspace), &id);
                self.save_change(change, cx);
            }
            Panel::Messages => self.commit(
                false,
                false,
                Box::new(move |runtime, _, cx| {
                    let workspace = runtime.content.read(cx).workspace.clone();
                    let mut messages = workspace["messages"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let before = messages.len();
                    messages.retain(|entry| entry["id"] != id.as_str());
                    if messages.len() == before {
                        return Err("automation.messages.missing".into());
                    }
                    Ok(json!({"messages": messages}))
                }),
                cx,
            ),
            Panel::Timers => self.delete_timer(&id, cx),
        }
    }

    fn duplicate_macro(&mut self, id: &str, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        if let Some(entry) = Library::of(&workspace)
            .macros
            .iter()
            .find(|entry| entry["id"] == id)
        {
            let copy = macros::duplicate(entry, || uuid::Uuid::new_v4().to_string());
            self.open(Panel::Macros, Value::Null, copy);
        }
        cx.notify();
    }

    fn reorder_macro(&mut self, id: &str, direction: i32, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        match macros::reorder(
            &Library::of(&workspace),
            id,
            direction,
            &self.active_collection(cx),
        ) {
            Ok(Some(change)) => self.save_change(Ok(change), cx),
            Ok(None) => {}
            Err(error) => {
                self.status = error;
                cx.notify();
            }
        }
    }

    fn copy_macro(&mut self, id: &str, cut: bool, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        let path = self.runtime.content.read(cx).workspace_path.clone();
        match macros::copy(
            &Library::of(&workspace),
            &path,
            id,
            &self.active_collection(cx),
            cut,
        ) {
            Ok(clipboard) => self.clipboard = Some(clipboard),
            Err(error) => self.status = error,
        }
        cx.notify();
    }

    fn paste_macro(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        let path = self.runtime.content.read(cx).workspace_path.clone();
        let result = macros::paste(
            &Library::of(&workspace),
            &path,
            self.clipboard.as_ref(),
            &self.active_collection(cx),
            || uuid::Uuid::new_v4().to_string(),
            validate::canonical_shortcut,
        );
        match result {
            Ok((_, change)) => {
                if self
                    .clipboard
                    .as_ref()
                    .is_some_and(|clipboard| clipboard.cut)
                {
                    self.clipboard = None;
                }
                self.save_change(Ok(change), cx);
            }
            Err(error) => {
                self.status = error;
                cx.notify();
            }
        }
    }

    fn ensure_default_collection(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        if self.busy || self.runtime.content.read(cx).workspace_path.is_empty() {
            return;
        }
        if let Some(change) = macros::ensure_default(
            &Library::of(&workspace),
            false,
            &uuid::Uuid::new_v4().to_string(),
        ) {
            self.save_change(Ok(change), cx);
        }
    }

    fn collection_action(&mut self, action: CollectionAction, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        let library = Library::of(&workspace);
        match action {
            CollectionAction::Select(id) => {
                self.collection = id;
                cx.notify();
            }
            CollectionAction::Create => {
                let id = uuid::Uuid::new_v4().to_string();
                let (_, change) =
                    macros::create_collection(&library, &id, macros::new_collection_title());
                self.collection = id;
                self.save_change(Ok(change), cx);
            }
            CollectionAction::Rename { id, title } => {
                self.save_change(macros::rename_collection(&library, &id, &title), cx)
            }
            CollectionAction::Duplicate(id) => {
                let new_id = uuid::Uuid::new_v4().to_string();
                let change = macros::duplicate_collection(&library, &id, &new_id);
                if change.is_ok() {
                    self.collection = new_id;
                }
                self.save_change(change, cx);
            }
            CollectionAction::Delete(id) => {
                self.save_change(macros::delete_collection(&library, &id), cx)
            }
            CollectionAction::Reorder {
                source,
                target,
                after,
            } => {
                let mut order: Vec<String> = library
                    .collections
                    .iter()
                    .filter_map(|entry| entry["id"].as_str().map(str::to_string))
                    .collect();
                let Some(from) = order.iter().position(|id| id == &source) else {
                    self.status = "automation.macros.collections.targetInvalid".into();
                    cx.notify();
                    return;
                };
                let id = order.remove(from);
                let Some(to) = order.iter().position(|id| id == &target) else {
                    self.status = "automation.macros.collections.targetInvalid".into();
                    cx.notify();
                    return;
                };
                order.insert(to + usize::from(after), id);
                self.save_change(macros::reorder_collections(&library, &order), cx);
            }
        }
    }

    // ----- rendering -----

    fn collection_bar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let t = Runtime::translator(cx);
        let workspace = self.workspace(cx);
        let library = Library::of(&workspace);
        let active = self.active_collection(cx);
        let options = library
            .collections
            .iter()
            .map(|entry| {
                let mut option = CollectionOption::new(
                    entry["id"].as_str().unwrap_or(""),
                    entry["title"].as_str().unwrap_or(""),
                );
                option.delete = library.collections.len() > 1;
                option
            })
            .collect();
        let owner = cx.weak_entity();
        div().child(
            PanelHeader::new("")
                .prominent()
                .title_control(
                    CollectionPicker::new(
                        "macro-collections",
                        t.t("automation.macros.collections.title"),
                        options,
                        active,
                        t.t("automation.macros.collections.confirmDelete"),
                    )
                    .disabled(self.runtime.content.read(cx).workspace_path.is_empty())
                    .on_action(move |action, _, cx| {
                        owner
                            .update(cx, |this, cx| this.collection_action(action, cx))
                            .ok();
                    }),
                )
                .action(
                    IconButton::new("macro-create", "plus", t.t("automation.macros.new"))
                        .large_icon()
                        .disabled(
                            self.busy || self.runtime.content.read(cx).workspace_path.is_empty(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
                ),
        )
    }
    fn status_line(&self, t: Translator) -> Option<gpui::AnyElement> {
        (!self.status.is_empty()).then(|| {
            div()
                .id("automation-status")
                .role(gpui::Role::Alert)
                .child(t.t(&self.status))
                .into_any_element()
        })
    }

    fn macro_footer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let t = Runtime::translator(cx);
        let input = self
            .filter
            .get_or_insert_with(|| {
                let input = orbit_ui::controls::filter_state("", window, cx);
                cx.subscribe(&input, |view, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        view.query = input.read(cx).value().to_string();
                        cx.notify();
                    }
                })
                .detach();
                input
            })
            .clone();
        let theme = *cx.theme();
        div()
            .id("macro-footer")
            .debug_selector(|| "macro-footer".into())
            .flex()
            .flex_none()
            .p(theme.metrics.space_xs)
            .border_t_1()
            .border_color(theme.colors.border_soft)
            .bg(theme.colors.panel_bg)
            .items_center()
            .gap(cx.theme().metrics.space_xs)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(FilterInput::new("macro-search", &input)),
            )
            .child(
                IconButton::new("macro-grid", "layout-grid", t.t("cue.view.grid"))
                    .selected(!self.table)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.table = false;
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new("macro-table", "list", t.t("cue.view.table"))
                    .selected(self.table)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.table = true;
                        cx.notify();
                    })),
            )
    }

    fn messages_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = Runtime::translator(cx);
        let theme = *cx.theme();
        let workspace = self.workspace(cx);
        let (rows, empty) = self.rows(cx);
        let live = self.runtime.output.read(cx).live.clone();
        let disabled = self.busy || self.runtime.content.read(cx).workspace_path.is_empty();
        let mut list = div()
            .id("messages-list")
            .flex()
            .flex_col()
            .on_drag_move::<MessageDrag>(cx.listener(|this, _, _, cx| {
                if this.message_drop.take().is_some() {
                    cx.notify();
                }
            }))
            .min_h(theme.metrics.control_height * 3.);
        if rows.is_empty() {
            list = list.child(EmptyState::new(t.t(empty)));
        }
        for (id, title) in rows {
            let selected = self.editor.is_some() && self.form.draft["id"] == id;
            let new = selected
                && self
                    .editor
                    .as_ref()
                    .is_some_and(|editor| editor.original.is_null());
            let title = if title.trim().is_empty() {
                t.t("automation.messages.new")
            } else {
                title
            };
            let live = live["activeMessageId"] == id;
            let edit_id = id.clone();
            let over = id.clone();
            let target = id.clone();
            let selector = format!("message-{id}");
            let drag = MessageDrag {
                id: id.clone(),
                title: title.clone(),
            };
            list = list.child(
                list_row(SharedString::from(format!("message-{id}")), live, cx)
                    .relative()
                    .debug_selector(move || selector.clone())
                    .when(new, |row| row.debug_selector(|| "message-draft".into()))
                    .when(selected, |row| row.bg(theme.colors.selected))
                    .when_some(
                        self.message_drop
                            .as_ref()
                            .filter(|(target, _)| target == &id && cx.has_active_drag()),
                        |row, (_, after)| {
                            row.child(
                                div()
                                    .debug_selector(|| "message-drop-line".into())
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .h(theme.metrics.space_2xs)
                                    .bg(theme.colors.accent)
                                    .when(*after, |line| line.bottom_0())
                                    .when(!*after, |line| line.top_0()),
                            )
                        },
                    )
                    .child(title)
                    .when(!disabled && !new, |row| {
                        row.on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    })
                    .on_drag_move::<MessageDrag>(cx.listener(
                        move |this, event: &gpui::DragMoveEvent<MessageDrag>, _, cx| {
                            if !event.bounds.contains(&event.event.position) || new || this.busy {
                                return;
                            }
                            cx.stop_propagation();
                            this.message_drop = (event.drag(cx).id != over).then(|| {
                                (
                                    over.clone(),
                                    event.event.position.y > event.bounds.center().y,
                                )
                            });
                            cx.notify();
                        },
                    ))
                    .on_drop::<MessageDrag>(cx.listener(move |this, drag: &MessageDrag, _, cx| {
                        cx.stop_propagation();
                        if let Some((id, after)) =
                            this.message_drop.take().filter(|(id, _)| id == &target)
                        {
                            this.reorder_message(&drag.id, &id, after, cx);
                        }
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.busy {
                            this.edit(&edit_id, cx);
                        }
                    }))
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            this.menu = Some(id.clone());
                            cx.notify();
                        }),
                    ),
            );
        }
        if let Some(id) = self.menu.clone() {
            list = list.child(button(
                "message-delete",
                t.t("common.delete"),
                ButtonVariant::Danger,
                cx,
                move |this, cx| {
                    this.menu = None;
                    this.delete(&id, cx);
                },
            ));
        }
        let mut details = div()
            .id("message-details")
            .debug_selector(|| "message-details".into())
            .flex()
            .flex_col()
            .gap(theme.metrics.space_sm)
            .p(theme.metrics.space_sm);
        if self.editor.is_some() {
            let draft = self.form.draft.clone();
            for spec in editors::message_specs(&draft, &workspace, t) {
                let selector = format!("message-field-{}", spec.path);
                details = details.child(
                    div()
                        .debug_selector(move || selector.clone())
                        .child(
                            self.form
                                .field("message-detail", &spec, draft_form, window, cx),
                        ),
                );
            }
            details = details.child(button(
                "message-token-add",
                t.t("automation.messages.addToken"),
                ButtonVariant::Secondary,
                cx,
                |this, cx| {
                    editors::add_token(&mut this.form, uuid::Uuid::new_v4().to_string());
                    cx.notify();
                },
            ));
            for index in 0..draft["tokens"].as_array().map_or(0, Vec::len) {
                details = details.child(button(
                    format!("message-token-remove-{index}"),
                    t.t("automation.messages.removeToken"),
                    ButtonVariant::Danger,
                    cx,
                    move |this, cx| {
                        editors::remove_token(&mut this.form, index);
                        cx.notify();
                    },
                ));
            }
        } else {
            details = details.child(EmptyState::new(t.t("automation.messages.selectMessage")));
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                PanelHeader::new(t.t("automation.messages.title"))
                    .prominent()
                    .action(
                        IconButton::new("message-create", "plus", t.t("automation.messages.new"))
                            .large_icon()
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
                    ),
            )
            .child(list)
            .child(
                div()
                    .p(theme.metrics.space_sm)
                    .border_y_1()
                    .border_color(theme.colors.border)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(t.t("automation.messages.details"))
                    .child(
                        ActionButton::new(
                            "message-details-toggle",
                            t.t("automation.messages.edit"),
                        )
                        .variant(ButtonVariant::Ghost)
                        .compact()
                        .icon(if self.details_open {
                            "chevron-up"
                        } else {
                            "chevron-down"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.details_open = !this.details_open;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .id("message-detail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(self.details_open, |region| region.child(details)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p(theme.metrics.space_xs)
                    .child(
                        IconButton::new(
                            "message-clear",
                            "player-stop",
                            t.t("automation.messages.clear"),
                        )
                        .disabled(disabled || live["activeMessageId"].is_null())
                        .on_click(cx.listener(|this, _, _, cx| {
                            let live = this.runtime.output.read(cx).live["activeMessageInstanceId"]
                                .clone();
                            this.run(vec![Self::action(Panel::Messages, "", "clear", &live)], cx);
                        })),
                    )
                    .child(
                        ActionButton::new("message-show", t.t("automation.messages.show"))
                            .icon("send")
                            .disabled(disabled || self.editor.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_after_save = true;
                                this.save(cx);
                            })),
                    ),
            )
            .children(self.status_line(t))
            .into_any_element()
    }

    fn editor_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = Runtime::translator(cx);
        let workspace = self.workspace(cx);
        let panel = self
            .editor
            .as_ref()
            .map_or(Panel::Macros, |editor| editor.panel);
        let gap = cx.theme().metrics.space_xs;
        let draft = self.form.draft.clone();
        let (specs, title) = match panel {
            Panel::Macros => (
                editors::macro_specs(&draft, Library::of(&workspace).collections, t),
                "automation.macros.edit",
            ),
            Panel::Messages => (
                editors::message_specs(&draft, &workspace, t),
                "automation.messages.edit",
            ),
            Panel::Timers => (editors::timer_specs(&draft, t), "automation.macros.edit"),
        };
        let mut body = div()
            .id("automation-editor")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap(gap)
            .p(gap)
            .child(div().child(t.t(title)));
        let mut inserted_hotkey = false;
        for spec in &specs {
            body = body.child(self.form.field("editor", spec, draft_form, window, cx));
            if panel == Panel::Macros && spec.path == "title" && !inserted_hotkey {
                inserted_hotkey = true;
                body = body.child(self.hotkey_row(&draft, t, cx));
            }
        }
        if panel == Panel::Messages {
            body = body.child(button(
                "token-add",
                t.t("automation.messages.addToken"),
                ButtonVariant::Secondary,
                cx,
                |this, cx| {
                    editors::add_token(&mut this.form, uuid::Uuid::new_v4().to_string());
                    cx.notify();
                },
            ));
            for index in 0..draft["tokens"].as_array().map_or(0, Vec::len) {
                body = body.child(button(
                    format!("token-remove-{index}"),
                    t.t("automation.messages.removeToken"),
                    ButtonVariant::Danger,
                    cx,
                    move |this, cx| {
                        editors::remove_token(&mut this.form, index);
                        cx.notify();
                    },
                ));
            }
        }
        if panel == Panel::Macros {
            body = body.child(self.image_row(&draft, t, cx));
            let id = draft["id"].as_str().unwrap_or("").to_string();
            body = body.child(action_list::render(
                &mut self.form,
                "editor-actions",
                "actions",
                &workspace,
                &id,
                draft_form,
                window,
                cx,
            ));
        }
        let save_label = t.t(if panel == Panel::Messages {
            "automation.messages.save"
        } else {
            "automation.macros.save"
        });
        body = body.children(self.status_line(t)).child(
            div()
                .flex()
                .gap(gap)
                .child(
                    button(
                        "editor-save",
                        save_label,
                        ButtonVariant::Primary,
                        cx,
                        |this, cx| this.save(cx),
                    )
                    .disabled(self.busy),
                )
                .child(button(
                    "editor-cancel",
                    t.t("common.cancel"),
                    ButtonVariant::Secondary,
                    cx,
                    |this, cx| {
                        this.close_editor();
                        cx.notify();
                    },
                )),
        );
        body.into_any_element()
    }

    /// `selectMacroImage`: a picture replaces the built-in icon; removing it brings the icon back.
    fn image_row(
        &mut self,
        draft: &Value,
        t: Translator,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let gap = cx.theme().metrics.space_xs;
        let has_image = draft["imageData"].is_string();
        let mut row = div().flex().gap(gap).child(button(
            "macro-image",
            t.t("automation.macros.image"),
            ButtonVariant::Secondary,
            cx,
            |this, cx| {
                let (host, path) = (
                    this.runtime.host.clone(),
                    this.runtime.content.read(cx).workspace_path.clone(),
                );
                cx.spawn(async move |this, cx| {
                    let picked = cx
                        .background_spawn(async move {
                            host.invoke("selectMacroImage", vec![json!(path)])
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        match picked {
                            Ok(image) if image["imageData"].is_string() => {
                                this.form.set("imageData", image["imageData"].clone());
                                this.form.set("imageType", image["imageType"].clone());
                            }
                            Ok(_) => {}
                            Err(error) => this.status = error,
                        }
                        cx.notify();
                    });
                })
                .detach();
            },
        ));
        if has_image {
            row = row.child(button(
                "macro-image-remove",
                t.t("automation.macros.imageRemove"),
                ButtonVariant::Secondary,
                cx,
                |this, cx| {
                    this.form.set("imageData", Value::Null);
                    this.form.set("imageType", Value::Null);
                    cx.notify();
                },
            ));
        }
        row.into_any_element()
    }

    fn hotkey_row(
        &mut self,
        draft: &Value,
        t: Translator,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = *cx.theme();
        let hotkey = draft["hotkey"].as_str().unwrap_or("");
        let shown = if hotkey.is_empty() {
            t.t("settings.keys.none")
        } else {
            crate::keys::format_shortcut_label(hotkey)
        };
        let focus = self.hotkey_focus.clone();
        div()
            .flex()
            .items_center()
            .gap(theme.metrics.space_xs)
            .child(
                Field::new(
                    t.t("automation.macros.hotkey"),
                    div()
                        .id("macro-hotkey")
                        .track_focus(&focus)
                        .tab_stop(true)
                        .w(px(160.))
                        .h(theme.metrics.control_height)
                        .px(theme.metrics.field_padding_x)
                        .flex()
                        .items_center()
                        .bg(theme.colors.field_bg)
                        .border_1()
                        .border_color(theme.colors.control_border)
                        .rounded(theme.metrics.radius_sm)
                        .child(shown)
                        .on_click(cx.listener({
                            let focus = focus.clone();
                            move |_, _, window, cx| window.focus(&focus, cx)
                        }))
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                            cx.stop_propagation();
                            if event.is_held {
                                return;
                            }
                            if let Some(shortcut) =
                                crate::keys::shortcut_from_keystroke(&event.keystroke)
                            {
                                this.form.set("hotkey", json!(shortcut));
                                cx.notify();
                            }
                        })),
                )
                .inline(),
            )
            .child(button(
                "macro-hotkey-clear",
                t.t("settings.general.clear"),
                ButtonVariant::Secondary,
                cx,
                |this, cx| {
                    this.form.set("hotkey", Value::Null);
                    cx.notify();
                },
            ))
            .into_any_element()
    }
}

impl Render for Panels {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Runtime::translator(cx);
        if self.panel == Panel::Macros && self.editor.is_none() {
            self.ensure_default_collection(cx);
        }
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.panel != self.panel && editor.panel != Panel::Messages)
        {
            self.close_editor();
        }
        if self.panel == Panel::Messages {
            return self.messages_view(window, cx);
        }
        if self.panel == Panel::Macros
            && self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.panel == Panel::Macros)
        {
            return self.editor_view(window, cx);
        }
        let gap = cx.theme().metrics.space_xs;
        let (rows, empty) = self.rows(cx);
        let live = self.runtime.output.read(cx).live.clone();
        let panel = self.panel;
        let workspace_open = !self.runtime.content.read(cx).workspace_path.is_empty();
        let new_label = t.t(match panel {
            Panel::Macros => "automation.macros.new",
            Panel::Messages => "automation.messages.new",
            Panel::Timers => "timer.new",
        });
        let header = PanelHeader::new(t.t(if panel == Panel::Timers {
            "timer.countdowns"
        } else {
            "automation.messages.title"
        }))
        .prominent()
        .action(
            IconButton::new("automation-new", "plus", new_label)
                .large_icon()
                .disabled(!workspace_open || self.busy)
                .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
        );
        let mut column = div().flex().flex_col().flex_1().min_h_0();
        if panel == Panel::Macros {
            column = column.child(self.collection_bar(window, cx));
        }
        column = column
            .when(panel != Panel::Macros, |column| column.child(header))
            .children(self.status_line(t));
        if rows.is_empty() {
            return if panel == Panel::Macros {
                column
                    .child(div().flex_1())
                    .child(self.macro_footer(window, cx))
                    .into_any_element()
            } else {
                column.child(EmptyState::new(t.t(empty))).into_any_element()
            };
        }
        let buttons: &[(&str, &str, &str)] = match panel {
            Panel::Macros => &[("run", "run", "automation.macros.trigger")],
            Panel::Messages => &[
                ("show", "show", "automation.messages.show"),
                ("clear", "clear", "automation.messages.clear"),
            ],
            Panel::Timers => &[
                ("start", "toggle", "timer.start"),
                ("reset", "reset", "timer.reset"),
            ],
        };
        let this = cx.entity();
        let workspace = self.workspace(cx);
        let table = self.table;
        let footer = (panel == Panel::Macros).then(|| self.macro_footer(window, cx));
        column
            .child(
                div()
                    .id("automation-rows")
                    .flex()
                    .when(panel == Panel::Macros && !table, |list| {
                        list.flex_wrap().content_start().gap(gap).p(gap)
                    })
                    .when(panel != Panel::Macros || table, |list| list.flex_col())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows.into_iter().map(|(id, title)| {
                        let timer_value = (panel == Panel::Timers).then(|| {
                            let timer = workspace["countdownTimers"]
                                .as_array()
                                .and_then(|list| list.iter().find(|entry| entry["id"] == id));
                            timer
                                .map(|timer| {
                                    let seconds = super::countdown::milliseconds(
                                        timer,
                                        &live["countdownTimerRuns"][&id],
                                        super::announcements::now_ms(),
                                    );
                                    let value = super::countdown::display(seconds);
                                    let positive = value.trim_start_matches('\u{2212}');
                                    let padded = if positive.matches(':').count() == 1 {
                                        format!("00:{positive:0>5}")
                                    } else {
                                        positive.to_string()
                                    };
                                    format!(
                                        "{}{padded}",
                                        if seconds < 0. { "\u{2212}" } else { "" }
                                    )
                                })
                                .unwrap_or_default()
                        });
                        let on_air =
                            panel == Panel::Messages && live["activeMessageId"] == id.as_str();
                        let menu_id = id.clone();
                        let mut row =
                            list_row(SharedString::from(format!("automation-{id}")), on_air, cx)
                                .when(panel == Panel::Timers, |row| {
                                    row.min_h(cx.theme().metrics.control_height)
                                })
                                .on_mouse_down(
                                    gpui::MouseButton::Right,
                                    cx.listener(move |this, _, _, cx| {
                                        this.menu = Some(menu_id.clone());
                                        cx.notify();
                                    }),
                                )
                                .when(panel == Panel::Timers, |row| {
                                    let id = id.clone();
                                    row.child(
                                        IconButton::new(
                                            SharedString::from(format!("timer-expand-{id}")),
                                            "chevron-right",
                                            t.t("timer.expand"),
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                if this.editor.is_some()
                                                    && this.form.draft["id"] == id
                                                {
                                                    this.close_editor();
                                                    cx.notify();
                                                } else {
                                                    this.edit(&id, cx);
                                                }
                                            }),
                                        ),
                                    )
                                })
                                .when(panel != Panel::Macros, |row| {
                                    row.child(
                                        div().flex_1().min_w_0().truncate().child(title.clone()),
                                    )
                                })
                                .children(timer_value.map(|value| {
                                    div().font_weight(gpui::FontWeight::BOLD).child(value)
                                }));
                        if panel == Panel::Macros {
                            let entry = workspace["macros"]
                                .as_array()
                                .and_then(|list| list.iter().find(|entry| entry["id"] == id));
                            let run_id = id.clone();
                            row = row
                                .debug_selector({
                                    let id = id.clone();
                                    move || format!("macro-entry-{id}")
                                })
                                .when(!table, |row| {
                                    row.flex_col()
                                        .h_auto()
                                        .py(cx.theme().metrics.space_sm)
                                })
                                .when(table, |row| {
                                    let metrics = cx.theme().metrics;
                                    row.flex_row().flex_wrap().h_auto()
                                        .min_h(metrics.control_height + metrics.space_sm * 3.)
                                        .py(metrics.space_sm).gap(metrics.space_sm)
                                        .border_b_1().border_color(cx.theme().colors.border_soft)
                                })
                                .child(
                                    div()
                                        .debug_selector({
                                            let id = id.clone();
                                            move || format!("macro-icon-{id}")
                                        })
                                        .when(!table, |icon| icon.flex_none()
                                            .size(cx.theme().metrics.space_lg * 4.)
                                            .rounded(cx.theme().metrics.radius_sm))
                                        .when(table, |icon| icon.flex_none()
                                            .size(cx.theme().metrics.control_height + cx.theme().metrics.space_sm)
                                            .rounded(cx.theme().metrics.radius_sm))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(entry
                                            .and_then(|entry| entry["color"].as_str())
                                            .map(orbit_ui::controls::data_color)
                                            .unwrap_or(cx.theme().colors.control_bg))
                                         .child({
                                            if let Some(data) = entry.and_then(|entry| entry["imageData"].as_str()) {
                                                let cached = self.images.entry(id.clone()).or_insert_with(|| (String::new(), Err(String::new())));
                                                if cached.0 != data {
                                                    cached.0 = data.to_string();
                                                    cached.1 = orbit_host::decode_macro_image_data(data).and_then(|bytes| {
                                                        let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { gpui::ImageFormat::Png }
                                                            else if bytes.starts_with(b"\xff\xd8\xff") { gpui::ImageFormat::Jpeg }
                                                            else { return Err(t.t("automation.macros.imageUnavailable")); };
                                                        Ok(std::sync::Arc::new(gpui::Image::from_bytes(format, bytes)))
                                                    });
                                                }
                                                match &cached.1 {
                                                    Ok(image) => gpui::img(image.clone()).size(cx.theme().metrics.space_lg * 2.).object_fit(gpui::ObjectFit::Contain).into_any_element(),
                                                    Err(_) => div().id(SharedString::from(format!("macro-image-error-{id}"))).role(gpui::Role::Alert).child(t.t("automation.macros.imageUnavailable")).into_any_element(),
                                                }
                                            } else {
                                            let kind = entry.and_then(|entry| entry["imageType"].as_str()).unwrap_or("ImageTypeDefault");
                                            match kind {
                                                "ImageTypeDefault" => orbit_ui::icon("square-letter-m").size(cx.theme().metrics.space_lg * 2.).text_color(cx.theme().colors.text).into_any_element(),
                                                "ImageTypePlay" => orbit_ui::icon("player-play").text_color(cx.theme().colors.text).into_any_element(),
                                                "ImageTypeTimer" => orbit_ui::icon("clock").text_color(cx.theme().colors.text).into_any_element(),
                                                _ => div().text_size(cx.theme().metrics.font_3xl).child(editors::icon_catalogue(t).into_iter().find(|(id, _)| id == kind)
                                                    .map(|(_, label)| label.split_whitespace().next().unwrap_or("").to_string())
                                                    .unwrap_or_else(|| t.t("automation.macros.iconUnknown"))).into_any_element(),
                                            }
                                            }
                                        }),
                                )
                                .child(div().debug_selector({
                                    let id = id.clone();
                                    move || format!("macro-title-{id}")
                                }).min_w_0().truncate().when(table, |title| title.flex_1()).when(!table, |title| title.w_full().text_center()).child(title.clone()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.run(
                                        vec![Self::action(
                                            Panel::Macros,
                                            &run_id,
                                            "run",
                                            &Value::Null,
                                        )],
                                        cx,
                                    );
                                }));
                        } else {
                            for (name, command, label) in buttons {
                                let (id, command) = (id.clone(), command.to_string());
                                let live_message = live["activeMessageInstanceId"].clone();
                                row = row.child(
                                    IconButton::new(
                                        SharedString::from(format!("{name}-{id}")),
                                        match *name {
                                            "start" | "show" => "player-play",
                                            "stop" => "player-pause",
                                            "reset" => "rotate-clockwise",
                                            _ => "player-stop",
                                        },
                                        t.t(label),
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.run(
                                                vec![Self::action(
                                                    this.panel,
                                                    &id,
                                                    &command,
                                                    &live_message,
                                                )],
                                                cx,
                                            );
                                        },
                                    )),
                                );
                            }
                        }
                        let mut edits: Vec<PanelEdit> = vec![
                            ("edit", "common.edit", Box::new(|p, id, cx| p.edit(id, cx))),
                            (
                                "delete",
                                "common.delete",
                                Box::new(|p, id, cx| p.delete(id, cx)),
                            ),
                        ];
                        match panel {
                            Panel::Macros => {
                                edits.push((
                                    "copy",
                                    "common.duplicate",
                                    Box::new(|p, id, cx| p.duplicate_macro(id, cx)),
                                ));
                                edits.push((
                                    "up",
                                    "automation.macros.up",
                                    Box::new(|p, id, cx| p.reorder_macro(id, -1, cx)),
                                ));
                                edits.push((
                                    "down",
                                    "automation.macros.down",
                                    Box::new(|p, id, cx| p.reorder_macro(id, 1, cx)),
                                ));
                                edits.push((
                                    "clip-copy",
                                    "common.copy",
                                    Box::new(|p, id, cx| p.copy_macro(id, false, cx)),
                                ));
                                edits.push((
                                    "clip-cut",
                                    "common.cut",
                                    Box::new(|p, id, cx| p.copy_macro(id, true, cx)),
                                ));
                                if self.clipboard.is_some() {
                                    edits.push((
                                        "clip-paste",
                                        "common.paste",
                                        Box::new(|p, _, cx| p.paste_macro(cx)),
                                    ));
                                }
                            }
                            Panel::Timers => {
                                edits.push((
                                    "up",
                                    "automation.macros.up",
                                    Box::new(|p, id, cx| p.move_timer(id, -1, cx)),
                                ));
                                edits.push((
                                    "down",
                                    "automation.macros.down",
                                    Box::new(|p, id, cx| p.move_timer(id, 1, cx)),
                                ));
                            }
                            Panel::Messages => {}
                        }
                        if self.menu.as_deref() == Some(id.as_str()) {
                            if panel == Panel::Macros {
                                if let Some(entry) = workspace["macros"]
                                    .as_array()
                                    .and_then(|rows| rows.iter().find(|entry| entry["id"] == id))
                                {
                                    row = row.child(macros::action_summary(&workspace, entry));
                                }
                            }
                            for (name, label, act) in edits {
                                let (this, id) = (this.clone(), id.clone());
                                let act = std::rc::Rc::new(act);
                                row = row.child(
                                    ActionButton::new(
                                        SharedString::from(format!("{name}-{id}")),
                                        t.t(label),
                                    )
                                    .variant(if name == "delete" {
                                        ButtonVariant::Danger
                                    } else {
                                        ButtonVariant::Secondary
                                    })
                                    .on_click(
                                        move |_, _, cx| {
                                            cx.stop_propagation();
                                            let (id, act) = (id.clone(), act.clone());
                                            this.update(cx, |panels, cx| act(panels, &id, cx))
                                        },
                                    ),
                                );
                            }
                        }
                        let expanded = panel == Panel::Timers
                            && self.editor.is_some()
                            && self.form.draft["id"] == id;
                        div().flex().flex_col().when(panel == Panel::Macros, |column| column.flex_none().when(table, |column| column.w_full()).when(!table, |column| column.w(cx.theme().metrics.space_lg * 6.))).child(row).when(expanded, |column| {
                            column.child(self.editor_view(window, cx))
                        })
                    })),
            )
            .children(footer)
            .into_any_element()
    }
}
