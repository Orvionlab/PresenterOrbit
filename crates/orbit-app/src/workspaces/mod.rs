//! The registered workspaces: the list the toolbar switcher, the General settings, and the Manage Workspaces
//! window share, and the commands that change it. It ports `WorkspaceActions` and `useWorkspaceList`, and the
//! activation of `workspace-loader-controller.ts`: whichever window switches, the main window's state is
//! replaced from the host's answer.

use gpui::{App, AppContext, Context, Entity, Global};
use orbit_ui::Translator;
use serde_json::{json, Value};

use crate::collections::model::Snapshot;
use crate::state::runtime::Runtime;

pub mod manage;
pub mod prompt;

/// One registered workspace, as `OrbitWorkspaceListItem`.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    pub path: String,
    pub active: bool,
    pub missing: bool,
}

impl Item {
    /// The folder that holds the manifest, as `workspaceFolder` shows it.
    pub fn folder(&self) -> String {
        let path = crate::settings::path_for_display(&self.path);
        path.trim_end_matches("workspace.json")
            .trim_end_matches(['\\', '/'])
            .to_string()
    }
}

fn parse_items(list: &Value) -> Result<Vec<Item>, String> {
    list.as_array()
        .ok_or("The workspace list is not an array.")?
        .iter()
        .map(|entry| {
            Ok(Item {
                name: entry["name"]
                    .as_str()
                    .ok_or("A workspace has no name.")?
                    .to_string(),
                path: entry["path"]
                    .as_str()
                    .ok_or("A workspace has no path.")?
                    .to_string(),
                active: entry["active"]
                    .as_bool()
                    .ok_or("A workspace has no active flag.")?,
                missing: entry["missing"]
                    .as_bool()
                    .ok_or("A workspace has no missing flag.")?,
            })
        })
        .collect()
}

pub struct Workspaces {
    runtime: Runtime,
    pub items: Vec<Item>,
    /// The failure of the last list read.
    pub error: Option<String>,
    /// The name of the workspace being opened. While it is set, the switching popup blocks the window.
    pub switching: Option<String>,
    refresh_id: u64,
}

/// The list of the application, which every window reads.
#[derive(Clone)]
pub struct SharedWorkspaces(pub Entity<Workspaces>);

impl Global for SharedWorkspaces {}

impl Workspaces {
    pub fn new(runtime: Runtime) -> Self {
        Self {
            runtime,
            items: Vec::new(),
            error: None,
            switching: None,
            refresh_id: 0,
        }
    }

    pub fn active(&self) -> Option<&Item> {
        self.items.iter().find(|item| item.active)
    }

    /// `useWorkspaceList.refresh`. An older answer never replaces a newer one.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refresh_id += 1;
        let id = self.refresh_id;
        let host = self.runtime.host.clone();
        cx.spawn(async move |this, cx| {
            let listed = cx
                .background_spawn(async move { host.invoke("listOrbitWorkspaces", Vec::new()) })
                .await;
            this.update(cx, |this, cx| {
                if this.refresh_id != id {
                    return;
                }
                match listed.and_then(|listed| parse_items(&listed["workspaces"])) {
                    Ok(items) => {
                        this.items = items;
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Runs a command on a background thread and hands the answer back.
    fn call(
        &self,
        method: &'static str,
        args: Vec<Value>,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Result<Value, String>, &mut Context<Self>) + 'static,
    ) {
        let host = self.runtime.host.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { host.invoke(method, args) })
                .await;
            this.update(cx, |this, cx| then(this, result, cx)).ok();
        })
        .detach();
    }

    fn report(&self, key: &str, error: &str, cx: &mut App) {
        let message = cx.global::<Translator>().format(key, &[("error", error)]);
        self.runtime.status.update(cx, |status, cx| {
            status.message = message;
            cx.notify();
        });
    }

    /// `switchOrbitWorkspace`: the popup shows until the host answered and the content was replaced.
    pub fn switch(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(item) = self.items.iter().find(|item| item.path == path) else {
            return;
        };
        if item.active || item.missing || self.switching.is_some() {
            return;
        }
        self.switching = Some(item.name.clone());
        cx.notify();
        self.call(
            "switchOrbitWorkspace",
            vec![json!(path)],
            cx,
            |this, result, cx| match result {
                Ok(result) => this.activated(result, cx),
                Err(error) => {
                    this.switching = None;
                    this.report("status.workspace.switchFailed", &error, cx);
                    cx.notify();
                }
            },
        );
    }

    /// `createOrbitWorkspace`. A failure is also the caller's to show, in its own prompt.
    pub fn create(
        &mut self,
        name: &str,
        cx: &mut Context<Self>,
        done: impl FnOnce(Result<(), String>, &mut App) + 'static,
    ) {
        self.call(
            "createOrbitWorkspace",
            vec![json!(name)],
            cx,
            |this, result, cx| match result {
                Ok(result) => {
                    this.activated(result, cx);
                    done(Ok(()), cx);
                }
                Err(error) => {
                    this.report("status.workspace.createFailed", &error, cx);
                    done(Err(error), cx);
                }
            },
        );
    }

    pub fn rename(
        &mut self,
        path: &str,
        name: &str,
        cx: &mut Context<Self>,
        done: impl FnOnce(Result<(), String>, &mut App) + 'static,
    ) {
        self.change(
            "renameOrbitWorkspace",
            vec![json!(path), json!(name)],
            cx,
            done,
        );
    }

    pub fn duplicate(
        &mut self,
        path: &str,
        name: &str,
        cx: &mut Context<Self>,
        done: impl FnOnce(Result<(), String>, &mut App) + 'static,
    ) {
        self.change(
            "duplicateOrbitWorkspace",
            vec![json!(path), json!(name)],
            cx,
            done,
        );
    }

    /// Moves the workspace folder to the trash. The host refuses the active workspace.
    pub fn delete(
        &mut self,
        path: &str,
        cx: &mut Context<Self>,
        done: impl FnOnce(Result<(), String>, &mut App) + 'static,
    ) {
        self.change("deleteOrbitWorkspace", vec![json!(path)], cx, done);
    }

    /// The commands that answer with the changed list.
    fn change(
        &mut self,
        method: &'static str,
        args: Vec<Value>,
        cx: &mut Context<Self>,
        done: impl FnOnce(Result<(), String>, &mut App) + 'static,
    ) {
        self.call(method, args, cx, |this, result, cx| {
            let changed = result.and_then(|result| parse_items(&result["workspaces"]));
            match changed {
                Ok(items) => {
                    // Older list reads that are still running must not undo this one.
                    this.refresh_id += 1;
                    this.items = items;
                    this.error = None;
                    cx.notify();
                    done(Ok(()), cx);
                }
                Err(error) => done(Err(error), cx),
            }
        });
    }

    /// `selectOrbitWorkspaceFolder`: choose a folder and open it as a workspace.
    pub fn choose_folder(&mut self, cx: &mut Context<Self>) {
        self.call(
            "selectOrbitWorkspaceFolder",
            Vec::new(),
            cx,
            |this, result, cx| match result {
                Ok(result) if result["canceled"] == json!(true) => {}
                Ok(result) => this.activated(result, cx),
                Err(error) => this.report("status.workspace.selectionFailed", &error, cx),
            },
        );
    }

    /// `actions.manage`: opens the Manage Workspaces window.
    pub fn manage(&mut self, cx: &mut Context<Self>) {
        self.call(
            "openAuxiliaryWindow",
            vec![json!({ "kind": "workspaces" })],
            cx,
            |this, result, cx| {
                if let Err(error) = result {
                    let t = *cx.global::<Translator>();
                    let message = format!("{}: {error}", t.t("window.openFailed"));
                    this.runtime.status.update(cx, |status, cx| {
                        status.message = message;
                        cx.notify();
                    });
                }
            },
        );
    }

    pub fn reveal(&mut self, path: &str, cx: &mut Context<Self>) {
        self.call(
            "showItemInFolder",
            vec![json!(path)],
            cx,
            |this, result, cx| {
                if let Err(error) = result {
                    this.report("window.openFailed", &error, cx);
                }
            },
        );
    }

    /// `applySelectedWorkspace`: the activation the host answered replaces the content of every panel. The
    /// effective screen configuration needs no copy: the scene reads it from the loaded workspace and the saved
    /// settings, exactly as the host derives it. The loaded content restarts the output scene (`output::scene`).
    pub fn activated(&mut self, result: Value, cx: &mut Context<Self>) {
        let runtime = self.runtime.clone();
        // The host ended any recording of its own; the one in the output process ends with the workspace too.
        let host = runtime.host.clone();
        cx.spawn(async move |this, cx| {
            let stopped = cx
                .background_spawn(async move { crate::output::capture::stop(&host) })
                .await;
            if let Err(error) = stopped {
                this.update(cx, |this, cx| {
                    this.report("status.workspace.selectionFailed", &error, cx)
                })
                .ok();
            }
        })
        .detach();
        runtime.lanes.revisions().remember_selected(&result);
        runtime.lanes.revisions().remember_settings(&result);
        cx.spawn(async move |this, cx| {
            let parsed = cx
                .background_spawn(async move {
                    Snapshot::parse(&result["workspace"]).map(|snapshot| {
                        let count =
                            |field: &str| result["workspace"][field].as_array().map_or(0, Vec::len);
                        let counts = [
                            snapshot.libraries.len(),
                            snapshot.items.iter().count(),
                            snapshot.playlists.len(),
                            snapshot.themes.len(),
                            count("props"),
                            count("media"),
                        ];
                        (snapshot, counts)
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                this.switching = None;
                match parsed {
                    Ok((snapshot, counts)) => {
                        let t = *cx.global::<Translator>();
                        let [libraries, items, playlists, themes, props, media] =
                            counts.map(|count| count.to_string());
                        runtime.content.update(cx, |content, cx| {
                            content.load(snapshot);
                            cx.notify();
                        });
                        runtime.edit_ui(cx, |content, ui| ui.select_loaded(content));
                        runtime.status.update(cx, |status, cx| {
                            status.message = t.format(
                                "status.workspace.loaded",
                                &[
                                    ("libraries", &libraries),
                                    ("items", &items),
                                    ("playlists", &playlists),
                                    ("themes", &themes),
                                    ("props", &props),
                                    ("media", &media),
                                    ("scanned", "0"),
                                    ("skipped", "0"),
                                ],
                            );
                            cx.notify();
                        });
                    }
                    Err(error) => this.report("status.workspace.selectionFailed", &error, cx),
                }
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Opens a Manage Workspaces prompt window: naming, duplicating, renaming, or confirming a delete.
pub fn request_prompt(host: orbit_host::Host, request: Prompt, cx: &mut App) {
    cx.spawn(async move |cx| {
        let opened = cx
            .background_spawn(async move {
                host.invoke(
                    "openAuxiliaryWindow",
                    vec![json!({ "kind": "workspace-prompt", "panel": request.encode() })],
                )
            })
            .await;
        if let Err(error) = opened {
            eprintln!("[workspaces] {error}");
        }
    })
    .detach();
}

/// `WorkspacePromptRequest`, carried to the prompt window as one string: the mode, then the path.
#[derive(Clone, Debug, PartialEq)]
pub enum Prompt {
    New,
    Rename(String),
    Duplicate(String),
    Delete(String),
}

impl Prompt {
    pub fn encode(&self) -> String {
        match self {
            Self::New => "new".into(),
            Self::Rename(path) => format!("rename:{path}"),
            Self::Duplicate(path) => format!("duplicate:{path}"),
            Self::Delete(path) => format!("delete:{path}"),
        }
    }

    /// `parseWorkspacePromptRequest`
    pub fn parse(value: &str) -> Result<Self, String> {
        if value == "new" {
            return Ok(Self::New);
        }
        let unknown = || format!("Unknown workspace prompt request: {value}");
        let (mode, path) = value.split_once(':').ok_or_else(unknown)?;
        if path.is_empty() {
            return Err(unknown());
        }
        match mode {
            "rename" => Ok(Self::Rename(path.into())),
            "duplicate" => Ok(Self::Duplicate(path.into())),
            "delete" => Ok(Self::Delete(path.into())),
            _ => Err(unknown()),
        }
    }
}
