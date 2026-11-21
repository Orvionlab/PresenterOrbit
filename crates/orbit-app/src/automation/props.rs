//! The Props utility tab (`RightInspectorPropsPanel.tsx`, `prop-collection-controller.ts`): prop collections, the
//! props of a collection that go on and off air, and the prop edits. Showing and clearing is the semantic `prop`
//! action, so single mode and auto-clear behave as for a macro. Every edit is one `mutateProps` intent.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{div, prelude::*, Context, Entity, SharedString, Window};
use orbit_ui::controls::{
    ActionButton, ButtonVariant, CollectionAction, CollectionOption, CollectionPicker, ContextMenu,
    Dialog, EmptyState, FilterInput, IconButton, InputEvent, InputState, MenuEntry, MenuItem,
    NativeButton, PanelHeader, RangeInput, SegmentedControl, SegmentedOption, SliderState,
};
use orbit_ui::ActiveTheme;
use serde_json::{json, Value};

use super::form::{get, Form, Kind, Spec};
use crate::canvas::DocumentKind;
use crate::state::runtime::Runtime;

/// Opens the authoring canvas on a document: kind, document id, slide id.
pub type Opener = Rc<dyn Fn(DocumentKind, &str, &str, &mut gpui::App)>;

pub struct EditCollection(pub String);
impl gpui::EventEmitter<EditCollection> for PropsPanel {}

pub(crate) const AUTO_CLEAR_SECONDS: [u32; 7] = [5, 10, 15, 20, 30, 45, 60];

/// The collections of the tab: named folders, plus the implicit default collection (`""`) pinned first when a
/// prop has no folder or there is no named folder at all.
pub fn collections(props: &[Value], folders: &[Value]) -> Vec<String> {
    let mut named: Vec<String> = Vec::new();
    for name in folders
        .iter()
        .filter_map(Value::as_str)
        .chain(props.iter().filter_map(|prop| prop["folder"].as_str()))
    {
        if !name.is_empty() && !named.iter().any(|known| known == name) {
            named.push(name.to_string());
        }
    }
    let unfiled = props
        .iter()
        .any(|prop| prop["folder"].as_str().is_none_or(str::is_empty));
    if named.is_empty() || unfiled {
        named.insert(0, String::new());
    }
    named
}

/// The selected collection, or the first one when the selection is stale.
pub fn active(collections: &[String], selected: &str) -> String {
    if collections.iter().any(|known| known == selected) {
        selected.to_string()
    } else {
        collections.first().cloned().unwrap_or_default()
    }
}

/// `setPropLive`: what a click on a prop sends. A live prop is cleared; otherwise it is shown, alone in single
/// mode, with the prop's auto-clear time.
pub fn toggle_action(prop: &Value, live: bool, single: bool, id: String) -> Value {
    let seconds = prop["autoClearSeconds"].as_f64().filter(|s| *s > 0.);
    json!({"id": id, "type": "prop", "targetId": prop["id"],
        "value": if live { "clear" } else { "show" }, "title": prop["title"],
        "details": {"singleMode": single, "autoClearEnabled": seconds.is_some(),
            "autoClearSeconds": seconds}})
}

#[derive(Clone)]
struct PropDrag {
    id: String,
    title: String,
}

impl Render for PropDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .bg(theme.colors.popover_bg)
            .text_color(theme.colors.text)
            .px(theme.metrics.space_sm)
            .py(theme.metrics.space_xs)
            .child(self.title.clone())
    }
}

pub struct PropsPanel {
    runtime: Runtime,
    open: Opener,
    collection: String,
    single: HashMap<String, bool>,
    menu: Option<(String, gpui::Point<gpui::Pixels>)>,
    transition_editor: Option<Entity<crate::transitions::TransitionDefaults>>,
    drop_target: Option<(String, bool)>,
    clipboard: Option<Value>,
    status: String,
    /// The prop whose name is being edited.
    renaming: Option<String>,
    form: Form,
    filter: Option<Entity<InputState>>,
    query: String,
    grid: bool,
    size: Entity<SliderState>,
    transitions: Entity<crate::transitions::TransitionDefaults>,
}

fn name_form(panel: &mut PropsPanel) -> &mut Form {
    &mut panel.form
}

fn small(
    id: impl Into<SharedString>,
    label: String,
    variant: ButtonVariant,
    cx: &mut Context<PropsPanel>,
    act: impl Fn(&mut PropsPanel, &mut Context<PropsPanel>) + 'static,
) -> ActionButton {
    ActionButton::new(id.into(), label)
        .variant(variant)
        .compact()
        .on_click(cx.listener(move |panel, _, _, cx| act(panel, cx)))
}

impl PropsPanel {
    pub fn new(runtime: Runtime, open: Opener, cx: &mut Context<Self>) -> Self {
        let size = orbit_ui::controls::range_state(0., 1., 0.01, 0.5, cx);
        cx.observe(&size, |_, _, cx| cx.notify()).detach();
        let transitions = cx.new(|cx| {
            crate::transitions::TransitionDefaults::new(
                crate::transitions::Kind::Props,
                runtime.clone(),
                cx,
            )
        });
        cx.observe(&runtime.content, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&runtime.output, |_, _, cx| cx.notify()).detach();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;
            if this
                .update(cx, |this, cx| {
                    if this.runtime.output.read(cx).live["remotePropDeadlines"]
                        .as_object()
                        .is_some_and(|map| {
                            map.values()
                                .any(|value| value.as_f64().is_some_and(|deadline| deadline > 0.))
                        })
                    {
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
        Self {
            runtime,
            open,
            collection: String::new(),
            single: HashMap::new(),
            menu: None,
            transition_editor: None,
            drop_target: None,
            clipboard: None,
            status: String::new(),
            renaming: None,
            form: Form::default(),
            filter: None,
            query: String::new(),
            grid: true,
            size,
            transitions,
        }
    }

    fn filter_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
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
        let t = Runtime::translator(cx);
        let owner = cx.weak_entity();
        div()
            .debug_selector(|| "props-filter-bar".into())
            .flex_none()
            .p(theme.metrics.space_xs)
            .border_t_1()
            .border_color(theme.colors.border_soft)
            .bg(theme.colors.panel_bg)
            .flex()
            .flex_col()
            .gap(theme.metrics.space_xs)
            .child(FilterInput::new("props-filter", &input))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .items_center()
                    .gap(theme.metrics.space_xs)
                    .child(
                        div()
                            .debug_selector(|| "props-transition-control".into())
                            .flex_none()
                            .child(self.transitions.clone()),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(theme.metrics.space_xs)
                            .child(
                                div()
                                    .debug_selector(|| "props-view-controls".into())
                                    .flex_none()
                                    .child(
                                        SegmentedControl::new(
                                            "props-view",
                                            t.t("props.view.label"),
                                            vec![
                                                SegmentedOption::icon(
                                                    true,
                                                    "layout-grid",
                                                    t.t("media.view.grid"),
                                                ),
                                                SegmentedOption::icon(
                                                    false,
                                                    "list",
                                                    t.t("media.view.list"),
                                                ),
                                            ],
                                            self.grid,
                                        )
                                        .on_change(
                                            move |grid, _, cx| {
                                                owner
                                                    .update(cx, |this, cx| {
                                                        this.grid = grid;
                                                        cx.notify();
                                                    })
                                                    .ok();
                                            },
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "props-size-slider".into())
                                    .flex_none()
                                    .w(theme.metrics.space_lg * 4.)
                                    .child(
                                        RangeInput::new(&self.size)
                                            .disabled(!self.grid)
                                            .label(t.t("props.size")),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn workspace(&self, cx: &Context<Self>) -> std::sync::Arc<Value> {
        self.runtime.content.read(cx).workspace.clone()
    }

    fn lists(&self, cx: &Context<Self>) -> (Vec<Value>, Vec<Value>, Vec<String>, String) {
        let workspace = self.workspace(cx);
        let props = workspace["props"].as_array().cloned().unwrap_or_default();
        let folders = workspace["propFolders"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let all = collections(&props, &folders);
        let current = active(&all, &self.collection);
        (props, folders, all, current)
    }

    fn mutate(
        &mut self,
        mutation: Value,
        then: impl FnOnce(&mut Self, Value) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.status.clear();
        let plan = match orbit_host::plan_props_mutation(&self.workspace(cx), &mutation) {
            Ok(plan) => plan,
            Err(error) => {
                self.status = error;
                cx.notify();
                return;
            }
        };
        let response = json!({"workspace":plan.workspace.clone(), "createdPropId":plan.created_prop_id, "createdFolder":plan.created_folder, "deletedPropId":plan.deleted_prop_id});
        if !plan.changed {
            then(self, response);
            cx.notify();
            return;
        }
        if let Err(error) = super::save::optimistic_content(
            &self.runtime,
            plan.workspace,
            cx,
            |this, outcome, cx| {
                if let Err(error) = outcome {
                    this.status = error;
                }
                cx.notify();
            },
        ) {
            self.status = error;
        } else {
            then(self, response);
        }
        cx.notify();
    }
    fn send(&mut self, mutation: Value, cx: &mut Context<Self>) {
        self.mutate(mutation, |_, _| {}, cx);
    }

    fn toggle(&mut self, prop: &Value, cx: &mut Context<Self>) {
        let live = self.runtime.output.read(cx).live["livePropIds"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| *id == prop["id"]));
        let single = self
            .single
            .get(&self.collection_now(cx))
            .copied()
            .unwrap_or(true);
        let action = toggle_action(prop, live, single, uuid::Uuid::new_v4().to_string());
        crate::automation::run_operator(&self.runtime, vec![action], cx);
    }

    fn collection_now(&self, cx: &Context<Self>) -> String {
        self.lists(cx).3
    }

    fn create_prop(&mut self, cx: &mut Context<Self>) {
        let folder = self.collection_now(cx);
        // The stored name is English, as in React.
        self.mutate(
            json!({"type": "create", "baseTitle": "New prop", "folder": folder}),
            |this, response| {
                if let Some(id) = response["createdPropId"].as_str() {
                    this.renaming = Some(id.to_string());
                    let title = response["workspace"]["props"]
                        .as_array()
                        .and_then(|props| props.iter().find(|prop| prop["id"] == id))
                        .map(|prop| prop["title"].clone());
                    if let Some(title) = title {
                        this.form.reset(json!({"title":title}));
                        this.form.request_focus("title");
                    }
                }
            },
            cx,
        );
    }

    fn create_folder(&mut self, cx: &mut Context<Self>) {
        self.mutate(
            json!({"type": "createFolder", "baseTitle": "New Collection"}),
            |this, response| {
                if let Some(folder) = response["createdFolder"].as_str() {
                    this.collection = folder.to_string();
                }
            },
            cx,
        );
    }

    fn collection_action(&mut self, action: CollectionAction, cx: &mut Context<Self>) {
        match action {
            CollectionAction::Select(id) => {
                self.collection = id;
                cx.notify();
            }
            CollectionAction::Create => self.create_folder(cx),
            CollectionAction::Rename { id, title } => {
                let selected = self.collection_now(cx) == id;
                let title = title.trim().to_string();
                self.mutate(
                    json!({"type":"renameFolder", "folder":id, "title":title}),
                    move |this, _| {
                        if selected {
                            this.collection = title;
                        }
                    },
                    cx,
                );
            }
            CollectionAction::Duplicate(id) => self.mutate(
                json!({"type":"duplicateFolder", "folder":id}),
                |this, response| {
                    if let Some(folder) = response["createdFolder"].as_str() {
                        this.collection = folder.to_string();
                    }
                },
                cx,
            ),
            CollectionAction::Delete(id) => {
                self.send(json!({"type":"deleteFolder", "folder":id}), cx)
            }
            CollectionAction::Reorder {
                source,
                target,
                after,
            } => {
                let mut order: Vec<String> = self
                    .lists(cx)
                    .2
                    .into_iter()
                    .filter(|id| !id.is_empty())
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
                self.send(json!({"type":"reorderFolders", "folders":order}), cx);
            }
        }
    }
    fn edit_transition(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let runtime = self.runtime.clone();
        let owner = cx.weak_entity();
        let target = id.clone();
        let editor = cx.new(|cx| {
            let mut editor = crate::transitions::TransitionDefaults::new(crate::transitions::Kind::Props, runtime, cx);
            editor.edit_prop(id, Rc::new(move |transition, cx| {
                owner.update(cx, |panel, cx| {
                    panel.send(json!({"type":"setTransition","propId":target,"transition":transition}), cx);
                }).ok();
            }), window, cx);
            editor
        });
        self.transition_editor = Some(editor);
        cx.notify();
    }

    fn toggle_auto_clear(&mut self, id: &str, cx: &mut Context<Self>) {
        super::run(
            &self.runtime,
            vec![json!({
                "id": uuid::Uuid::new_v4().to_string(), "type":"prop", "targetId":id, "value":"show",
                "details":{"toggleAutoClearPause":true}
            })],
            0,
            cx,
        );
    }

    fn row_menu(
        &self,
        prop: &Value,
        all: &[String],
        position: gpui::Point<gpui::Pixels>,
        cx: &Context<Self>,
    ) -> ContextMenu {
        let t = Runtime::translator(cx);
        let id = prop["id"].as_str().unwrap_or_default().to_string();
        let owner = cx.weak_entity();
        let mutation = |label: String, intent: Value| {
            let weak = owner.clone();
            MenuItem::new(label).on_click(move |_, _, cx| {
                weak.update(cx, |panel, cx| panel.send(intent.clone(), cx))
                    .ok();
            })
        };
        let folder = prop["folder"].as_str().unwrap_or_default();
        let destinations = |label, kind| {
            let entries: Vec<_> = all
                .iter()
                .filter(|name| kind != "move" || name.as_str() != folder)
                .map(|name| {
                    mutation(
                        if name.is_empty() {
                            t.t("props.defaultCollection")
                        } else {
                            name.clone()
                        },
                        json!({"type":kind,"propId":id,"folder":name}),
                    )
                    .into()
                })
                .collect();
            MenuEntry::Submenu {
                label: t.t(label).into(),
                disabled: entries.is_empty(),
                entries,
            }
        };
        let mut seconds = vec![None];
        seconds.extend(AUTO_CLEAR_SECONDS.map(Some));
        let configured = prop["autoClearSeconds"].as_f64();
        let auto_clear = seconds
            .into_iter()
            .map(|value| {
                let label = value.map_or_else(
                    || t.t("props.autoClear.never"),
                    |seconds| {
                        t.format(
                            "props.autoClear.after",
                            &[("seconds", &seconds.to_string())],
                        )
                    },
                );
                mutation(
                    label,
                    json!({"type":"setAutoClear","propId":id,"seconds":value}),
                )
                .checked(value.map(f64::from) == configured)
                .into()
            })
            .collect();
        let edit = MenuItem::new(t.t("common.edit")).on_click({
            let opener = self.open.clone();
            let slide = prop["slide"]["id"].as_str().unwrap_or_default().to_string();
            let id = id.clone();
            move |_, _, cx| opener(DocumentKind::Prop, &id, &slide, cx)
        });
        let transition = MenuItem::new(t.t("transition.picker.title")).on_click({
            let owner = owner.clone();
            let id = id.clone();
            move |_, window, cx| {
                owner
                    .update(cx, |panel, cx| {
                        panel.edit_transition(id.clone(), window, cx)
                    })
                    .ok();
            }
        });
        let clipboard = |cut: bool| {
            let owner = owner.clone();
            let prop = prop.clone();
            MenuItem::new(t.t(if cut { "common.cut" } else { "common.copy" })).on_click(
                move |_, _, cx| {
                    owner
                        .update(cx, |panel, cx| {
                            panel.clipboard = Some(prop.clone());
                            if cut {
                                panel.send(json!({"type":"delete","propId":prop["id"]}), cx);
                            }
                            cx.notify();
                        })
                        .ok();
                },
            )
        };
        let rename = MenuItem::new(t.t("common.rename")).on_click({
            let owner = owner.clone();
            let id = id.clone();
            let title = prop["title"].clone();
            move |_, _, cx| {
                owner
                    .update(cx, |panel, cx| {
                        panel.renaming = Some(id.clone());
                        panel.form.reset(json!({"title":title}));
                        panel.form.request_focus("title");
                        cx.notify();
                    })
                    .ok();
            }
        });
        let entries = vec![
            destinations("automation.macros.addTo", "duplicate"),
            destinations("automation.macros.moveTo", "move"),
            MenuEntry::Separator,
            edit.into(),
            transition.into(),
            MenuEntry::Submenu {
                label: t.t("props.autoClear.title").into(),
                disabled: false,
                entries: auto_clear,
            },
            MenuEntry::Separator,
            clipboard(true).into(),
            clipboard(false).into(),
            mutation(
                t.t("common.paste"),
                json!({"type":"paste","prop":self.clipboard,"folder":folder}),
            )
            .disabled(self.clipboard.is_none())
            .into(),
            mutation(
                t.t("common.duplicate"),
                json!({"type":"duplicate","propId":id,"folder":folder}),
            )
            .into(),
            rename.into(),
            MenuEntry::Separator,
            mutation(t.t("common.delete"), json!({"type":"delete","propId":id})).into(),
        ];
        ContextMenu::new("prop-menu", position, entries, move |_, _, cx| {
            owner
                .update(cx, |panel, cx| {
                    panel.menu = None;
                    cx.notify();
                })
                .ok();
        })
    }
}

impl Render for PropsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Runtime::translator(cx);
        let gap = cx.theme().metrics.space_xs;
        let (props, _, all, current) = self.lists(cx);
        let workspace_open = !self.runtime.content.read(cx).workspace_path.is_empty();
        let secondary = ButtonVariant::Secondary;
        let options = all
            .iter()
            .map(|name| {
                let mut option = CollectionOption::new(
                    name.clone(),
                    if name.is_empty() {
                        t.t("props.defaultCollection")
                    } else {
                        name.clone()
                    },
                );
                option.rename = !name.is_empty();
                option.delete = !name.is_empty();
                option.movable = !name.is_empty();
                option
            })
            .collect();
        let weak = cx.weak_entity();
        let single = self.single.get(&current).copied().unwrap_or(true);
        let single_key = current.clone();
        let mut header = PanelHeader::new("").prominent().title_control(
            div().debug_selector(|| "props-header".into()).child(
                CollectionPicker::new(
                    "prop-collections",
                    t.t("props.collections.title"),
                    options,
                    current.clone(),
                    t.t("props.collections.confirmDelete"),
                )
                .disabled(!workspace_open)
                .on_action(move |action, _, cx| {
                    weak.update(cx, |this, cx| this.collection_action(action, cx))
                        .ok();
                }),
            ),
        );
        header = header
            .action(
                IconButton::new("prop-single", "stack-front", t.t("props.singleMode"))
                    .large_icon()
                    .selected(single)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.single.insert(single_key.clone(), !single);
                        cx.notify();
                    })),
            )
            .action(
                IconButton::new("prop-edit-collection", "pencil", t.t("appMenu.propsEditor"))
                    .large_icon()
                    .disabled(!workspace_open)
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.emit(EditCollection(this.collection_now(cx)));
                    })),
            )
            .action(
                IconButton::new("prop-create", "plus", t.t("props.create"))
                    .large_icon()
                    .on_click(cx.listener(|this, _, _, cx| this.create_prop(cx)))
                    .disabled(!workspace_open),
            );
        if self.clipboard.is_some() {
            header = header.action(small(
                "prop-paste",
                t.t("common.paste"),
                secondary,
                cx,
                |this, cx| {
                    if let Some(prop) = this.clipboard.clone() {
                        let folder = this.collection_now(cx);
                        this.send(json!({"type": "paste", "prop": prop, "folder": folder}), cx);
                    }
                },
            ));
        }
        let live = self.runtime.output.read(cx).live["livePropIds"].clone();
        let query = self.query.trim().to_lowercase();
        let shown: Vec<Value> = props
            .into_iter()
            .filter(|prop| prop["folder"].as_str().unwrap_or("") == current)
            .filter(|prop| {
                prop["title"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&query)
            })
            .collect();
        let menu = self.menu.as_ref().and_then(|(id, position)| {
            shown
                .iter()
                .find(|prop| prop["id"] == *id)
                .map(|prop| self.row_menu(prop, &all, *position, cx))
        });
        let transition_editor = self.transition_editor.clone().map(|editor| {
            let owner = cx.weak_entity();
            Dialog::new(
                "prop-transition-editor",
                t.t("transition.picker.title"),
                t.t("common.close"),
                move |_, _, cx| {
                    owner
                        .update(cx, |panel, cx| {
                            panel.transition_editor = None;
                            cx.notify();
                        })
                        .ok();
                },
            )
            .width(crate::transitions::TransitionDefaults::picker_width(window))
            .flush()
            .child(editor)
        });
        let filter = self.filter_bar(window, cx);
        let mut column = div().flex().flex_col().flex_1().min_h_0().child(header);
        if !self.status.is_empty() {
            column = column.child(
                div()
                    .id("props-status")
                    .role(gpui::Role::Alert)
                    .child(t.t(&self.status)),
            );
        }
        if shown.is_empty() {
            return column
                .child(div().flex_1().min_h_0().child(EmptyState::new(t.t(
                    if self.query.trim().is_empty() {
                        "props.empty"
                    } else {
                        "collections.filter.empty"
                    },
                ))))
                .child(filter)
                .into_any_element();
        }
        let theme = *cx.theme();
        if !self.grid {
            column = column.child(
                div()
                    .debug_selector(|| "props-table-header".into())
                    .flex()
                    .items_center()
                    .gap(gap)
                    .px(theme.metrics.field_padding_x)
                    .h(theme.metrics.header_height)
                    .text_size(theme.metrics.font_xs)
                    .text_color(theme.colors.text_muted)
                    .border_b_1()
                    .border_color(theme.colors.border_soft)
                    .child(
                        div()
                            .flex_none()
                            .w(theme.metrics.space_lg * 3.)
                            .child(orbit_ui::icon("polaroid")),
                    )
                    .child(div().flex_1().min_w_0().child(t.t("media.inspector.name")))
                    .child(
                        div()
                            .flex_none()
                            .w(theme.metrics.space_lg * 5.)
                            .child(t.t("props.autoClear.title")),
                    ),
            );
        }
        let columns = (5. - 3. * self.size.read(cx).value().end()).round() as u16;
        let mut list = div()
            .id("props-rows")
            .debug_selector(|| "props-rows".into())
            .when(self.grid, |list| {
                list.grid().grid_cols(columns).content_start().items_start()
            })
            .when(!self.grid, |list| list.flex().flex_col())
            .gap(theme.metrics.field_padding_x)
            .p(theme.metrics.field_padding_x)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for prop in shown {
            let id = prop["id"].as_str().unwrap_or("").to_string();
            let on_air = live
                .as_array()
                .is_some_and(|ids| ids.iter().any(|known| *known == prop["id"]));
            let idle = prop["autoClearSeconds"].as_f64().filter(|s| *s > 0.);
            let title = prop["title"].as_str().unwrap_or("").to_string();
            let deadline = self.runtime.output.read(cx).live["remotePropDeadlines"][&id].as_f64();
            let paused = deadline.is_some_and(|deadline| deadline < 0.);
            let remaining = deadline
                .filter(|_| on_air)
                .map(|deadline| {
                    if paused {
                        -deadline / 1000.
                    } else {
                        ((deadline - super::announcements::now_ms()) / 1000.).max(0.)
                    }
                })
                .or(idle);
            let timer = remaining.map(|seconds| {
                let target = id.clone();
                let color = if paused {
                    theme.colors.text_muted
                } else {
                    theme.colors.text_strong
                };
                NativeButton::new(SharedString::from(format!("prop-timer-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("prop-timer-{id}")
                    })
                    .accessibility_label(t.t(if paused {
                        "automation.timeline.resume"
                    } else {
                        "timer.pause"
                    }))
                    .disabled(!on_air || deadline.is_none())
                    .flex()
                    .items_center()
                    .when(self.grid, |button| {
                        button
                            .absolute()
                            .top(gap)
                            .left(gap)
                            .h(theme.metrics.space_md)
                            .gap(theme.metrics.space_2xs)
                            .text_size(theme.metrics.font_xs)
                            .child(
                                orbit_ui::icon("clock-x")
                                    .size(theme.metrics.space_md)
                                    .text_color(color),
                            )
                    })
                    .text_color(color)
                    .child(format!("{seconds:.1}s"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_auto_clear(&target, cx);
                    }))
                    .into_any_element()
            });
            let background = (prop["slide"]["backgroundEnabled"] != false)
                .then(|| {
                    prop["slide"]["background"]
                        .as_str()
                        .and_then(crate::cues::parse_hex_color)
                })
                .flatten();
            let checker = background.is_none_or(|color| color.a < 1.);
            let toggled = prop.clone();
            let menu_id = id.clone();
            let mut timer = timer;
            let mut thumbnail = div()
                .debug_selector({
                    let id = id.clone();
                    move || format!("prop-preview-{id}")
                })
                .relative()
                .when(self.grid, |preview| preview.w_full())
                .when(!self.grid, |preview| {
                    preview.flex_none().w(theme.metrics.space_lg * 3.)
                })
                .aspect_ratio(16. / 9.)
                .overflow_hidden()
                .border_1()
                .border_color(if on_air {
                    theme.colors.live
                } else {
                    theme.colors.border_strong
                })
                .rounded(theme.metrics.radius_sm)
                .when(checker, |preview| {
                    preview.child(crate::media::transparency_background(
                        &format!("prop-{id}"),
                        cx,
                    ))
                })
                .when_some(background, |preview, color| {
                    preview.child(div().absolute().inset_0().bg(color))
                })
                .child(
                    NativeButton::new(SharedString::from(format!("prop-toggle-{id}")))
                        .accessibility_label(title.clone())
                        .absolute()
                        .inset_0()
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle(&toggled, cx))),
                )
                .when(!prop["transition"].is_null(), |preview| {
                    preview.child(
                        div()
                            .debug_selector({
                                let id = id.clone();
                                move || format!("prop-transition-{id}")
                            })
                            .absolute()
                            .bottom(gap)
                            .right(gap)
                            .bg(theme.colors.popover_bg)
                            .rounded(theme.metrics.radius_xs)
                            .child(
                                orbit_ui::icon("transition-right").text_color(theme.colors.accent),
                            ),
                    )
                });
            if self.grid {
                thumbnail = thumbnail.children(timer.take());
            }
            let marker = self
                .drop_target
                .as_ref()
                .filter(|(target, _)| *target == id && cx.has_active_drag());
            let grid = self.grid;
            let mut row = div()
                .id(SharedString::from(format!("prop-{id}")))
                .debug_selector({ let id = id.clone(); move || format!("prop-row-{id}") })
                .relative()
                .flex()
                .flex_none()
                .when(grid, |row| row.flex_col().w_full().min_w_0())
                .when(!grid, |row| row.flex_row().items_center().w_full().flex_wrap())
                .gap(gap)
                .on_mouse_down(gpui::MouseButton::Right, cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.menu = Some((menu_id.clone(), event.position));
                    cx.notify();
                }))
                .on_drag(PropDrag { id: id.clone(), title: title.clone() }, |drag, _, _, cx| cx.new(|_| drag.clone()))
                .on_drag_move::<PropDrag>({
                    let id = id.clone();
                    cx.listener(move |this, event: &gpui::DragMoveEvent<PropDrag>, _, cx| {
                        if !event.bounds.contains(&event.event.position) { return; }
                        let after = if grid {
                            event.event.position.x > event.bounds.center().x
                        } else {
                            event.event.position.y > event.bounds.center().y
                        };
                        let target = (event.drag(cx).id != id).then(|| (id.clone(), after));
                        if target != this.drop_target { this.drop_target = target; cx.notify(); }
                    })
                })
                .on_drop::<PropDrag>({
                    let id = id.clone();
                    cx.listener(move |this, source: &PropDrag, _, cx| {
                        if let Some((_, after)) = this.drop_target.take().filter(|(target, _)| *target == id) {
                            this.send(json!({"type":"reorder","propId":source.id,"targetPropId":id,"placement": if after { "after" } else { "before" }}), cx);
                        }
                        cx.notify();
                    })
                })
                .child(thumbnail)
                .child(div()
                    .id(SharedString::from(format!("prop-title-{id}")))
                    .when(grid, |title| title.text_center())
                    .when(!grid, |title| title.flex_1().min_w_0())
                    .truncate().child(title)
                    .on_click(cx.listener({
                        let prop = prop.clone();
                        move |this, _, _, cx| this.toggle(&prop, cx)
                    })))
                .when(!grid, |row| row.child(div().flex_none().w(theme.metrics.space_lg * 5.)
                    .text_size(theme.metrics.font_sm)
                    .child(timer.unwrap_or_else(|| div().child(t.t("props.autoClear.never")).into_any_element()))))
                .when_some(marker, |row, (_, after)| row.child(div()
                    .debug_selector(|| "prop-drop-indicator".into())
                    .absolute().bg(theme.colors.accent)
                    .when(grid, |line| line.top_0().bottom_0().w(theme.metrics.space_2xs)
                        .when(*after, |line| line.right(-gap)).when(!after, |line| line.left(-gap)))
                    .when(!grid, |line| line.left_0().right_0().h(theme.metrics.space_2xs)
                        .when(*after, |line| line.bottom(-gap)).when(!after, |line| line.top(-gap)))));
            if self.renaming.as_deref() == Some(id.as_str()) {
                row = row
                    .child(self.form.field(
                        "prop-name",
                        &Spec::new("title", t.t("common.rename"), Kind::Text),
                        name_form,
                        window,
                        cx,
                    ))
                    .child(small(
                        format!("prop-rename-save-{id}"),
                        t.t("automation.macros.save"),
                        ButtonVariant::Primary,
                        cx,
                        {
                            let id = id.clone();
                            move |this, cx| {
                                let title = get(&this.form.draft, "title")
                                    .as_str()
                                    .unwrap_or("")
                                    .trim()
                                    .to_string();
                                this.renaming = None;
                                if !title.is_empty() {
                                    this.send(
                                        json!({"type": "rename", "propId": id, "title": title}),
                                        cx,
                                    );
                                }
                                cx.notify();
                            }
                        },
                    ))
                    .child(small(
                        format!("prop-rename-cancel-{id}"),
                        t.t("common.cancel"),
                        secondary,
                        cx,
                        |this, cx| {
                            this.renaming = None;
                            cx.notify();
                        },
                    ));
            }
            list = list.child(row);
        }
        column
            .child(list)
            .child(filter)
            .children(menu)
            .children(transition_editor)
            .into_any_element()
    }
}
