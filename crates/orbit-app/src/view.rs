use std::path::PathBuf;
#[cfg(test)]
#[path = "e2e/mod.rs"]
mod e2e;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, App, Bounds, Context, Div, Entity, EventEmitter, FocusHandle, MouseButton,
    MouseMoveEvent, Pixels, Point, SharedString, Stateful, Subscription, WeakEntity, Window,
};
use orbit_host::Host;
use orbit_ui::controls::{
    BarButton, BrandButton, BusyDialog, ContextMenu, Dialog, EmptyState, MenuEntry, MenuItem,
    PanelHeader, TabItem, TabList, ToolButton,
};
use orbit_ui::{follow_theme, set_appearance_settings, ActiveTheme, Translator};
use serde_json::{json, Value};

use crate::collections::documents::{self, DocumentsView};
use crate::collections::search::{self, SearchView};
use crate::collections::tree_actions;
use crate::collections::trees::TreesView;
use crate::desktop::APP_NAME;
use crate::keys::{
    dispatch_key_down, format_shortcut_label, Commands, KeyMappings, NEW_PRESENTATION,
};
use crate::layout::{
    self, next_value, viewport_or_fallback, Drag, Layout, LayoutKey, Resize, MENU_BAR_HEIGHT,
    SHELL_TOP_HEIGHT, TOOL_BAR_HEIGHT,
};
use crate::main_window;
use crate::native_window::WindowControl;
use crate::startup::{self, Startup};
use crate::state::lane::Lanes;
use crate::state::runtime::Runtime;

pub struct HostEvent {
    pub event: String,
    pub payload: Value,
}

/// What the main window needs from the application besides the host.
#[derive(Clone)]
pub struct ShellServices {
    pub control: Rc<dyn WindowControl>,
    pub minimize_to_tray: Arc<AtomicBool>,
}

enum Status {
    Loading,
    NoWorkspace,
    Ready,
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuId {
    File,
    Edit,
    Presentation,
    View,
    Screens,
    Window,
    Help,
    More,
    Audience,
    Stage,
    Looks,
    Workspace,
}

impl MenuId {
    const ALL: [Self; 7] = [
        Self::File,
        Self::Edit,
        Self::Presentation,
        Self::Screens,
        Self::View,
        Self::Window,
        Self::Help,
    ];

    fn label_key(self) -> &'static str {
        match self {
            Self::File => "appMenu.file",
            Self::Edit => "appMenu.edit",
            Self::Presentation => "appMenu.presentation",
            Self::View => "appMenu.view",
            Self::Screens => "appMenu.screens",
            Self::Window => "appMenu.window",
            Self::Help => "appMenu.help",
            Self::More => "toolbar.more",
            Self::Audience => "common.audience",
            Self::Stage => "common.stage",
            Self::Looks => "audienceLooks.title",
            Self::Workspace => "settings.general.workspace",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Edit => "edit",
            Self::Presentation => "presentation",
            Self::View => "view",
            Self::Screens => "screens",
            Self::Window => "window",
            Self::Help => "help",
            Self::More => "more",
            Self::Audience => "audience",
            Self::Stage => "stage",
            Self::Looks => "looks",
            Self::Workspace => "workspace",
        }
    }
}

struct OpenMenu {
    id: MenuId,
    anchor: Point<Pixels>,
}

#[derive(Clone, Copy)]
struct DragState {
    resize: Resize,
    start_value: f32,
    start: (f32, f32),
}

const UTILITY_PANELS: [(&str, &str); 6] = [
    ("audio", "media.audio"),
    ("stage", "utilityPanels.stage"),
    ("timers", "utilityPanels.timers"),
    ("messages", "automation.messages.title"),
    ("props", "utilityPanels.props"),
    ("macros", "automation.macros.title"),
];
const TRANSPORT_TABS: [(&str, &str); 2] = [
    ("media", "output.layer.media"),
    ("music", "output.layer.music"),
];

/// The width of a divider's hit area, and how far it reaches past the edge it sits on.
const DIVIDER: f32 = 7.;
const MENU_WIDTH: f32 = 338.;
const WORKSPACE_MENU_WIDTH: f32 = 224.;
/// The longest workspace name the toolbar button shows before it ends in an ellipsis.
const WORKSPACE_LABEL_CHARS: usize = 16;

fn truncated(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit - 1).collect();
    format!("{kept}…")
}

pub struct AppView {
    host: Host,
    runtime: Runtime,
    workspaces: Entity<crate::workspaces::Workspaces>,
    media: Entity<crate::media::MediaBin>,
    media_visible: bool,
    audio_bin: Entity<crate::media::AudioBin>,
    /// Applies the foreground media audio behavior; it follows the live state by itself.
    _foreground_audio: Entity<crate::media::foreground_audio::ForegroundAudio>,
    automation: Entity<crate::automation::panels::Panels>,
    props: Entity<crate::automation::props::PropsPanel>,
    stage: Entity<crate::automation::stage::StagePanel>,
    transport: Entity<crate::automation::transport::Transport>,
    trees: Entity<TreesView>,
    documents: Entity<DocumentsView>,
    search: Entity<SearchView>,
    output_preview: Entity<crate::output::view::OutputView>,
    cue_thumbnails: Entity<crate::output::thumbnails::Thumbnails>,
    cue_forms: Entity<crate::cues::Forms>,
    canvas: Entity<crate::canvas::SlideCanvas>,
    canvas_open: bool,
    screen_canvas: Entity<crate::output::screen_canvas::ScreenCanvas>,
    screen_canvas_open: bool,
    tickers: Option<Entity<crate::tickers::Tickers>>,
    bibles: Option<Entity<crate::bibles::Bibles>>,
    transfers: Option<Entity<crate::transfers::Transfers>>,
    imports: Option<Entity<crate::imports::Imports>>,
    charts: Option<Entity<crate::charts::Charts>>,
    external: Entity<crate::external::Session>,
    timeline: Option<Entity<crate::automation::timeline::Timeline>>,
    announcements: Option<Entity<crate::automation::announcements::Announcements>>,
    slide_authoring: Option<Entity<crate::automation::slide_authoring::SlideAuthoring>>,
    confirm: Entity<crate::automation::confirm::Confirm>,
    startup_update: Entity<crate::updates::StartupUpdate>,
    workspace: Option<PathBuf>,
    status: Status,
    first_page_pending: bool,
    services: ShellServices,
    layout: Layout,
    layout_loaded: bool,
    layout_path: Option<PathBuf>,
    drag: Option<DragState>,
    menu: Option<OpenMenu>,
    commands: Commands,
    mappings: KeyMappings,
    focus: FocusHandle,
    window_error: Option<String>,
    host_error: Option<String>,
    transport_tab: &'static str,
    utility_tab: &'static str,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<HostEvent> for AppView {}

impl AppView {
    fn open_slide_authoring(
        &mut self,
        item: String,
        slide: String,
        section: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.weak_entity();
        let runtime = self.runtime.clone();
        self.slide_authoring = Some(cx.new(|cx| {
            crate::automation::slide_authoring::SlideAuthoring::new(
                runtime,
                item,
                slide,
                section,
                move |cx| {
                    let _ = owner.update(cx, |view, cx| {
                        view.slide_authoring = None;
                        cx.notify();
                    });
                },
                cx,
            )
        }));
        cx.notify();
    }
    pub fn new(
        host: Host,
        workspace: Option<PathBuf>,
        services: ShellServices,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let layout_path = match host.path().app_data_dir() {
            Ok(directory) => Some(layout::layout_path(&directory)),
            Err(error) => {
                eprintln!("Presenter Orbit cannot store panel sizes: {error}");
                None
            }
        };
        let layout = Layout::default();
        let (control, to_tray) = (services.control.clone(), services.minimize_to_tray.clone());
        let mut subscriptions: Vec<Subscription> = follow_theme(window, cx).into();
        subscriptions.push(window.observe_window_visibility(move |_, window, _| {
            if let Err(error) = main_window::hide_if_minimized_to_tray(
                window,
                control.as_ref(),
                to_tray.load(Ordering::Acquire),
            ) {
                eprintln!("[main-window] {error}");
            }
        }));
        let focus = cx.focus_handle();
        let runtime = Runtime::new(host.clone(), Lanes::default(), cx);
        let confirm = cx.new(|_| crate::automation::confirm::Confirm::default());
        let startup_update = cx.new(|_| crate::updates::StartupUpdate::new(runtime.clone()));
        crate::automation::start(&runtime, confirm.clone(), cx);
        crate::services::start(runtime.clone(), cx);
        let external = crate::external::start(runtime.clone(), cx);
        cx.observe(&external, |_, _, cx| cx.notify()).detach();
        cx.set_global(crate::settings::SharedRuntime(runtime.clone()));
        let workspaces = cx.new(|_| crate::workspaces::Workspaces::new(runtime.clone()));
        cx.set_global(crate::workspaces::SharedWorkspaces(workspaces.clone()));
        cx.observe(&workspaces, |_, _, cx| cx.notify()).detach();
        let media = cx.new(|cx| crate::media::MediaBin::new(runtime.clone(), cx));
        subscriptions.push(
            cx.subscribe(&media, |_, _, _: &crate::media::SidebarChanged, cx| {
                cx.notify()
            }),
        );
        let audio_bin = cx.new(|cx| crate::media::AudioBin::new(media.clone(), cx));
        let foreground_audio = crate::media::foreground_audio::attach(runtime.clone(), cx);
        let automation = cx.new(|cx| crate::automation::panels::Panels::new(runtime.clone(), cx));
        let opener: crate::automation::props::Opener = {
            let weak = cx.weak_entity();
            std::rc::Rc::new(move |kind, owner, slide, cx| {
                let (owner, slide) = (owner.to_string(), slide.to_string());
                let _ = weak.update(cx, |view, cx| {
                    view.canvas
                        .update(cx, |canvas, cx| canvas.open_kind(kind, &owner, &slide, cx));
                    view.canvas_open = true;
                    cx.notify();
                });
            })
        };
        let props = {
            let opener = opener.clone();
            cx.new(|cx| crate::automation::props::PropsPanel::new(runtime.clone(), opener, cx))
        };
        subscriptions.push(cx.subscribe(
            &props,
            |view, _, event: &crate::automation::props::EditCollection, cx| {
                view.canvas.update(cx, |canvas, cx| {
                    canvas.open_prop_collection(&event.0, cx);
                });
                view.canvas_open = true;
                cx.notify();
            },
        ));
        let stage =
            cx.new(|cx| crate::automation::stage::StagePanel::new(runtime.clone(), opener, cx));
        let transport =
            cx.new(|cx| crate::automation::transport::Transport::new(runtime.clone(), cx));
        let trees = cx.new(|cx| TreesView::new(runtime.clone(), cx));
        let documents = cx.new(|cx| DocumentsView::new(runtime.clone(), cx));
        let search_view = cx.new(|cx| SearchView::new(runtime.clone(), cx));
        let output_owner = cx.weak_entity();
        let output_preview =
            cx.new(|cx| crate::output::view::OutputView::new(runtime.clone(), output_owner, cx));
        let cue_thumbnails =
            cx.new(|cx| crate::output::thumbnails::Thumbnails::new(runtime.clone(), cx));
        let cue_forms = cx.new(|cx| crate::cues::Forms::new(cue_thumbnails.clone(), cx));
        let canvas = cx.new(|cx| crate::canvas::SlideCanvas::new(runtime.clone(), cx));
        let screen_canvas =
            cx.new(|cx| crate::output::screen_canvas::ScreenCanvas::new(runtime.clone(), cx));
        subscriptions.push(cx.subscribe(
            &screen_canvas,
            |view, _, _: &crate::output::screen_canvas::Closed, cx| {
                view.screen_canvas_open = false;
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe(
            &canvas,
            |view, _, event: &crate::canvas::CanvasEvent, cx| {
                if let crate::canvas::CanvasEvent::SlideActions { item, slide } = event {
                    view.open_slide_authoring(item.clone(), slide.clone(), None, cx);
                    return;
                }
                if let crate::canvas::CanvasEvent::Reflow { item, slide } = event {
                    view.runtime.edit_ui(cx, |content, ui| {
                        ui.selected_library_id = content.library_id_for_item(item).to_string();
                        ui.selected_library_item_id = item.clone();
                        ui.selected_documents.clear();
                        ui.selected_playlist_id.clear();
                    });
                    view.cue_thumbnails.update(cx, |cues, cx| {
                        cues.request_reflow(item.clone(), slide.clone(), cx)
                    });
                }
                view.canvas_open = false;
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe(
            &cue_thumbnails,
            |view, _, event: &crate::cues::SlideMenuRequest, cx| match event {
                crate::cues::SlideMenuRequest::Actions {
                    item,
                    slide,
                    section,
                } => {
                    view.open_slide_authoring(item.clone(), slide.clone(), *section, cx);
                }
                crate::cues::SlideMenuRequest::InspectMedia(id) => {
                    view.media_visible = true;
                    view.media
                        .update(cx, |media, cx| media.open_inspector(id, cx));
                    cx.notify();
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &cue_thumbnails,
            |view, _, event: &crate::cues::EditSlide, cx| {
                view.canvas.update(cx, |canvas, cx| {
                    canvas.open_kind(
                        crate::canvas::DocumentKind::Slide,
                        &event.item,
                        &event.slide,
                        cx,
                    )
                });
                view.canvas_open = true;
                cx.notify();
            },
        ));
        subscriptions.push(cx.observe(&runtime.content, |this, _, cx| {
            search::sync(&this.runtime, cx)
        }));
        subscriptions.push(cx.observe(&runtime.search, |this, _, cx| {
            search::sync(&this.runtime, cx)
        }));
        for subscription in [
            cx.observe(&runtime.content, |this, _, cx| {
                crate::output::scene::sync(&this.runtime, cx);
                this.media.update(cx, |media, cx| media.auto_relink(cx));
                if this.first_page_pending {
                    cx.notify();
                }
            }),
            cx.observe(&runtime.ui, |this, _, cx| {
                crate::output::scene::sync(&this.runtime, cx)
            }),
            cx.observe(&runtime.output, |this, _, cx| {
                crate::output::scene::sync(&this.runtime, cx);
                this.media.update(cx, |media, cx| media.auto_relink(cx));
                if this.first_page_pending {
                    cx.notify();
                }
            }),
        ] {
            subscriptions.push(subscription);
        }
        let mut view = Self {
            host,
            runtime,
            media,
            media_visible: true,
            audio_bin,
            _foreground_audio: foreground_audio,
            automation,
            trees,
            documents,
            search: search_view,
            output_preview,
            cue_thumbnails,
            cue_forms,
            canvas,
            canvas_open: false,
            screen_canvas,
            screen_canvas_open: false,
            tickers: None,
            bibles: None,
            transfers: None,
            imports: None,
            charts: None,
            external,
            timeline: None,
            announcements: None,
            slide_authoring: None,
            confirm,
            startup_update,
            props,
            stage,
            transport,
            workspaces,
            workspace,
            status: Status::Loading,
            first_page_pending: false,
            services,
            layout,
            layout_loaded: false,
            layout_path,
            drag: None,
            menu: None,
            commands: Commands::default(),
            mappings: KeyMappings::default(),
            focus,
            window_error: None,
            host_error: None,
            transport_tab: TRANSPORT_TABS[0].0,
            utility_tab: "props",
            _subscriptions: subscriptions,
        };
        view.register_commands(cx);
        cx.set_global(crate::automation::inputs::Commands {
            window: window.window_handle(),
            commands: view.commands.clone(),
        });
        view.load(window, cx);
        view
    }

    /// The commands the shell itself can run. Features register theirs as they arrive.
    fn register_commands(&self, cx: &mut Context<Self>) {
        for action in [
            "undo",
            "redo",
            "cut",
            "copy",
            "paste",
            "duplicate",
            "delete",
            "selectAll",
            "deselectAll",
        ] {
            let weak = cx.weak_entity();
            self.commands.register(action, false, move |window, cx| {
                let _ = weak.update(cx, |view, cx| {
                    if view.canvas_open {
                        view.canvas
                            .update(cx, |canvas, cx| canvas.edit_command(action, window, cx));
                    } else if matches!(action, "undo" | "redo")
                        && !view.cue_thumbnails.read(cx).reflow_open()
                    {
                        view.documents.update(cx, |documents, cx| {
                            documents.edit_theme_history(action == "undo", cx)
                        });
                    } else {
                        view.cue_thumbnails
                            .update(cx, |cues, cx| cues.edit_command(action, window, cx));
                    }
                });
            });
        }
        for (command, mode) in [
            ("view-continuous", "continuous"),
            ("view-easy", "easy"),
            ("view-grid", "grid"),
            ("view-table", "table"),
        ] {
            let cues = self.cue_thumbnails.clone();
            self.commands.register(command, false, move |_, cx| {
                cues.update(cx, |cues, cx| cues.set_view(mode, cx))
            });
        }
        for (command, tab) in [
            ("view-audio", "audio"),
            ("view-stage", "stage"),
            ("view-timers", "timers"),
            ("view-messages", "messages"),
            ("view-props", "props"),
            ("view-macros", "macros"),
        ] {
            let weak = cx.weak_entity();
            self.commands.register(command, false, move |_, cx| {
                let _ = weak.update(cx, |view, cx| {
                    view.utility_tab = tab;
                    cx.notify();
                });
            });
        }
        for (command, announcement, library) in [
            ("showTriggeredPresentation", false, false),
            ("showTriggeredAnnouncement", true, false),
            ("revealInLibrary", false, true),
        ] {
            let runtime = self.runtime.clone();
            let commands = self.commands.clone();
            self.commands.register(command, false, move |window, cx| {
                let live = runtime.output.read(cx).live.clone();
                let id = if announcement {
                    live["announcements"]["item"]["id"].as_str()
                } else {
                    live["liveItemId"].as_str()
                };
                let Some(id) = id else {
                    return;
                };
                runtime.edit_ui(cx, |content, ui| {
                    if content.items.get(id).is_none() {
                        return;
                    }
                    use crate::collections::selection::{self, Modifiers};
                    let playlist = (!library && !announcement)
                        .then(|| live["livePlaylistId"].as_str())
                        .flatten()
                        .and_then(|playlist| content.playlist(playlist));
                    let entry = playlist.and_then(|playlist| {
                        playlist.entries.iter().position(|entry| {
                            entry.item_id == id
                                && live["livePlaylistEntryId"]
                                    .as_str()
                                    .is_none_or(|live_id| entry.id == live_id)
                        })
                    });
                    if let (Some(playlist), Some(index)) = (playlist, entry) {
                        selection::select_playlist(
                            content,
                            ui,
                            &playlist.id,
                            None,
                            Modifiers::default(),
                        );
                        selection::click_document_row(
                            content,
                            ui,
                            id,
                            true,
                            index,
                            Modifiers::default(),
                        );
                    } else {
                        let library_id = content.library_id_for_item(id);
                        selection::select_library(
                            content,
                            ui,
                            library_id,
                            None,
                            Modifiers::default(),
                        );
                        selection::click_document_row(
                            content,
                            ui,
                            id,
                            false,
                            0,
                            Modifiers::default(),
                        );
                    }
                });
                commands.run("show-workspace", false, window, cx);
            });
        }
        let runtime = self.runtime.clone();
        self.commands.register("stageOnly", false, move |_, cx| {
            crate::automation::run_operator(&runtime, vec![json!({"id":uuid::Uuid::new_v4().to_string(),"type":"slide-destination","value":"STAGE_ONLY"})], cx);
        });
        let weak = cx.weak_entity();
        self.commands.register("toggle-media", false, move |_, cx| {
            let _ = weak.update(cx, |view, cx| {
                view.media_visible = !view.media_visible;
                cx.notify();
            });
        });
        let weak = cx.weak_entity();
        self.commands
            .register("view-media", false, move |window, cx| {
                let _ = weak.update(cx, |view, cx| {
                    view.media_visible = true;
                    view.media
                        .update(cx, |media, cx| media.focus_filter(window, cx));
                    cx.notify();
                });
            });
        for (id, stage) in [("toggle-audience", false), ("toggle-stage", true)] {
            let runtime = self.runtime.clone();
            self.commands.register(id, false, move |_, cx| {
                crate::output::windows::toggle_group(&runtime, stage, cx);
            });
        }
        for (id, clipboard) in [("import-files", false), ("import-clipboard", true)] {
            let weak = cx.weak_entity();
            let runtime = self.runtime.clone();
            self.commands.register(id, false, move |_, cx| {
                let owner = weak.clone();
                let _ = weak.update(cx, |view, cx| {
                    if view
                        .imports
                        .as_ref()
                        .is_some_and(|import| import.read(cx).picking_files())
                    {
                        return;
                    }
                    view.imports = Some(cx.new(|cx| {
                        let mut imports = crate::imports::Imports::new(
                            runtime.clone(),
                            move |cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.imports = None;
                                    cx.notify();
                                });
                            },
                            cx,
                        );
                        if clipboard {
                            imports.load_clipboard(cx);
                        } else {
                            imports.pick_file(false, cx);
                        }
                        imports
                    }));
                    cx.notify();
                });
            });
        }
        let weak = cx.weak_entity();
        self.commands
            .register("open-reflow", false, move |window, cx| {
                let _ = weak.update(cx, |view, cx| {
                    if view.canvas_open
                        && !view
                            .canvas
                            .update(cx, |canvas, cx| canvas.suspend(window, cx))
                    {
                        return;
                    }
                    view.canvas_open = false;
                    view.cue_thumbnails
                        .update(cx, |cues, cx| cues.command("editCue", window, cx));
                    cx.notify();
                });
            });
        let weak = cx.weak_entity();
        self.commands
            .register("open-more", false, move |window, cx| {
                let _ = weak.update(cx, |view, cx| {
                    view.menu = if view
                        .menu
                        .as_ref()
                        .is_some_and(|menu| menu.id == MenuId::More)
                    {
                        None
                    } else {
                        Some(OpenMenu {
                            id: MenuId::More,
                            anchor: window.mouse_position(),
                        })
                    };
                    cx.notify();
                });
            });
        let weak = cx.weak_entity();
        self.commands
            .register("show-workspace", false, move |window, cx| {
                let _ = weak.update(cx, |view, cx| {
                    view.canvas_open = !view
                        .canvas
                        .update(cx, |canvas, cx| canvas.suspend(window, cx));
                    view.cue_thumbnails
                        .update(cx, |cues, cx| cues.close_form(cx));
                    cx.notify();
                });
            });

        let weak = cx.weak_entity();
        let runtime = self.runtime.clone();
        self.commands.register("open-tickers", false, move |_, cx| {
            let owner = weak.clone();
            let _ = weak.update(cx, |view, cx| {
                view.tickers = Some(cx.new(|cx| {
                    crate::tickers::Tickers::new(
                        runtime.clone(),
                        move |cx| {
                            let _ = owner.update(cx, |view, cx| {
                                view.tickers = None;
                                cx.notify();
                            });
                        },
                        cx,
                    )
                }));
                cx.notify();
            });
        });

        let weak = cx.weak_entity();
        let runtime = self.runtime.clone();
        self.commands.register("open-bibles", false, move |_, cx| {
            let owner = weak.clone();
            let _ = weak.update(cx, |view, cx| {
                view.bibles = Some(cx.new(|cx| {
                    crate::bibles::Bibles::new(
                        runtime.clone(),
                        move |cx| {
                            let _ = owner.update(cx, |view, cx| {
                                view.bibles = None;
                                cx.notify();
                            });
                        },
                        cx,
                    )
                }));
                cx.notify();
            });
        });

        let weak = cx.weak_entity();
        let runtime = self.runtime.clone();
        self.commands
            .register("open-handouts", false, move |_, cx| {
                let owner = weak.clone();
                let _ = weak.update(cx, |view, cx| {
                    view.transfers = Some(cx.new(|cx| {
                        crate::transfers::Transfers::new(
                            runtime.clone(),
                            move |cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.transfers = None;
                                    cx.notify();
                                });
                            },
                            cx,
                        )
                    }));
                    cx.notify();
                });
            });

        let weak = cx.weak_entity();
        let runtime = self.runtime.clone();
        self.commands.register("open-bundles", false, move |_, cx| {
            let owner = weak.clone();
            let _ = weak.update(cx, |view, cx| {
                view.imports = Some(cx.new(|cx| {
                    crate::imports::Imports::new(
                        runtime.clone(),
                        move |cx| {
                            let _ = owner.update(cx, |view, cx| {
                                view.imports = None;
                                cx.notify();
                            });
                        },
                        cx,
                    )
                }));
                cx.notify();
            });
        });

        let weak = cx.weak_entity();
        let runtime = self.runtime.clone();
        self.commands.register("open-charts", false, move |_, cx| {
            let owner = weak.clone();
            let _ = weak.update(cx, |view, cx| {
                view.charts = Some(cx.new(|cx| {
                    crate::charts::Charts::new(
                        runtime.clone(),
                        move |cx| {
                            let _ = owner.update(cx, |view, cx| {
                                view.charts = None;
                                cx.notify();
                            });
                        },
                        cx,
                    )
                }));
                cx.notify();
            });
        });

        let weak = cx.weak_entity();
        let runtime = self.runtime.clone();
        self.commands
            .register("open-presentation-export", false, move |_, cx| {
                let owner = weak.clone();
                let _ = weak.update(cx, |view, cx| {
                    view.transfers = Some(cx.new(|cx| {
                        crate::transfers::Transfers::export(
                            runtime.clone(),
                            move |cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.transfers = None;
                                    cx.notify();
                                });
                            },
                            cx,
                        )
                    }));
                    cx.notify();
                });
            });

        let weak = cx.weak_entity();
        self.commands
            .register("open-slide-actions", false, move |_, cx| {
                weak.update(cx, |view, cx| {
                    let Some((item, slide)) = view.cue_thumbnails.read(cx).selected_location(cx)
                    else {
                        return;
                    };
                    view.open_slide_authoring(item, slide, None, cx);
                })
                .ok();
            });
        let weak = cx.weak_entity();
        let runtime = self.runtime.clone();
        self.commands
            .register("open-announcements", false, move |_, cx| {
                let owner = weak.clone();
                weak.update(cx, |view, cx| {
                    let selected = view.runtime.ui.read(cx).selected_library_item_id.clone();
                    let runtime = runtime.clone();
                    view.announcements = Some(cx.new(|cx| {
                        crate::automation::announcements::Announcements::new(
                            runtime,
                            &selected,
                            move |cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.announcements = None;
                                    cx.notify();
                                });
                            },
                            cx,
                        )
                    }));
                    cx.notify();
                })
                .ok();
            });
        let weak = cx.weak_entity();
        self.commands
            .register("open-timeline", false, move |_, cx| {
                let owner = weak.clone();
                let _ = weak.update(cx, |view, cx| {
                    view.timeline = Some(cx.new(|cx| {
                        crate::automation::timeline::Timeline::new(
                            view.runtime.clone(),
                            move |cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.timeline = None;
                                    cx.notify();
                                });
                            },
                            cx,
                        )
                    }));
                    cx.notify();
                });
            });
        let weak = cx.weak_entity();
        self.commands
            .register("open-screen-canvas", false, move |_, cx| {
                weak.update(cx, |view, cx| {
                    view.screen_canvas_open = true;
                    cx.notify();
                })
                .ok();
            });
        for (command, kind) in [
            ("open-slide-canvas", crate::canvas::DocumentKind::Slide),
            ("open-prop-canvas", crate::canvas::DocumentKind::Prop),
            ("open-mask-canvas", crate::canvas::DocumentKind::Mask),
            ("open-stage-canvas", crate::canvas::DocumentKind::Stage),
            ("open-ccli", crate::canvas::DocumentKind::Ccli),
        ] {
            let weak = cx.weak_entity();
            self.commands.register(command, false, move |_, cx| {
                weak.update(cx, |view, cx| {
                    if kind == crate::canvas::DocumentKind::Slide
                        && view.cue_thumbnails.read(cx).reflow_open()
                    {
                        view.cue_thumbnails
                            .update(cx, |cues, cx| cues.reflow_edit(cx));
                        return;
                    }
                    let selected = if kind == crate::canvas::DocumentKind::Slide {
                        view.cue_thumbnails.read(cx).selected_location(cx)
                    } else {
                        None
                    };
                    view.canvas.update(cx, |canvas, cx| {
                        if let Some((owner, slide)) = selected {
                            canvas.open_kind(kind, &owner, &slide, cx);
                        } else {
                            canvas.open_editor(kind, cx);
                        }
                    });
                    view.canvas_open = true;
                    cx.notify();
                })
                .ok();
            });
        }
        let weak = cx.weak_entity();
        self.commands
            .register("open-theme-canvas", false, move |_, cx| {
                weak.update(cx, |view, cx| {
                    view.canvas
                        .update(cx, |canvas, cx| canvas.open_selected(cx));
                    view.canvas_open = true;
                    cx.notify();
                })
                .ok();
            });
        let weak = cx.weak_entity();
        self.commands
            .register("close-canvas", false, move |window, cx| {
                weak.update(cx, |view, cx| {
                    view.canvas_open = !view
                        .canvas
                        .update(cx, |canvas, cx| canvas.suspend(window, cx));
                    cx.notify();
                })
                .ok();
            });
        let open = |commands: &Commands, id: &str, request: Value, view: WeakEntity<Self>| {
            let host = self.host.clone();
            commands.register(id, false, move |_, cx| {
                request_auxiliary(host.clone(), request.clone(), view.clone(), cx)
            });
        };
        let view = cx.weak_entity();
        open(
            &self.commands,
            "open-settings",
            json!({ "kind": "settings", "panel": "general" }),
            view.clone(),
        );
        open(
            &self.commands,
            "open-key-settings",
            json!({ "kind": "settings", "panel": "keys" }),
            view.clone(),
        );
        open(
            &self.commands,
            "open-timecode",
            json!({"kind":"settings","panel":"devices"}),
            view.clone(),
        );
        open(
            &self.commands,
            "openLooks",
            json!({ "kind": "settings", "panel": crate::settings::LOOKS_PANEL }),
            view.clone(),
        );
        open(
            &self.commands,
            "open-screens",
            json!({ "kind": "screens" }),
            view.clone(),
        );
        open(
            &self.commands,
            "open-about",
            json!({ "kind": "about" }),
            view.clone(),
        );
        open(
            &self.commands,
            "open-licenses",
            json!({"kind":"about","panel":"licenses"}),
            view,
        );
        let runtime = self.runtime.clone();
        self.commands.register("new-library", false, {
            let runtime = runtime.clone();
            move |_, cx| tree_actions::create_library(&runtime, cx).detach()
        });
        self.commands.register("new-playlist", false, {
            let runtime = runtime.clone();
            move |_, cx| {
                let title = tree_actions::translator(cx).t("collections.default.newPlaylist");
                tree_actions::create_playlist(&runtime, cx, title, None).detach()
            }
        });
        self.commands.register(NEW_PRESENTATION, false, {
            let runtime = runtime.clone();
            move |_, cx| documents::open_new_presentation(&runtime, cx)
        });
        self.commands.register("open-search", false, move |_, cx| {
            search::open_search(&runtime, cx)
        });
        let (control, to_tray) = (
            self.services.control.clone(),
            self.services.minimize_to_tray.clone(),
        );
        let (minimize_control, minimize_flag) = (control.clone(), to_tray.clone());
        self.commands
            .register("window-minimize", false, move |window, _| {
                if let Err(error) = main_window::minimize(
                    window,
                    minimize_control.as_ref(),
                    minimize_flag.load(Ordering::Acquire),
                ) {
                    eprintln!("[main-window] {error}");
                }
            });
        let maximize_control = control.clone();
        self.commands
            .register("window-maximize", false, move |window, _| {
                if let Err(error) = maximize_control.maximize(window) {
                    eprintln!("[main-window] {error}");
                }
            });
        self.commands
            .register("window-restore", false, move |window, _| {
                if let Err(error) = main_window::restore(window, control.as_ref()) {
                    eprintln!("[main-window] {error}");
                }
            });
        for (command, direction) in [("nextBackground", 1), ("previousBackground", -1)] {
            let media = self.media.downgrade();
            self.commands.register(command, false, move |_, cx| {
                let _ = media.update(cx, |media, cx| media.step_program(direction, cx));
            });
        }
        for command in [
            "newSlide",
            "nextSlide",
            "previousSlide",
            "clearAll",
            "clearSlide",
            "clearMedia",
            "clearProps",
            "clearAudio",
            "clearMessages",
            "clearAnnouncements",
            "clearToLogo",
            "editCue",
            "takeCue",
        ] {
            let view = self.cue_thumbnails.downgrade();
            self.commands.register(
                command,
                matches!(command, "nextSlide" | "previousSlide"),
                move |window, cx| {
                    view.update(cx, |view, cx| view.command(command, window, cx))
                        .ok();
                },
            );
        }
    }

    /// The answer of the startup update check.
    pub fn offer_update(&mut self, status: Value, cx: &mut Context<Self>) {
        self.startup_update
            .update(cx, |dialog, cx| dialog.offer(status, cx));
    }

    pub fn fail_host_startup(&mut self, error: String, cx: &mut Context<Self>) {
        self.host_error = Some(error.clone());
        self.first_page_pending = false;
        startup::first_page::finish(cx);
        self.status = Status::Failed(error);
        cx.notify();
    }

    /// Restores a main window that is hidden or minimized, as the tray icon does.
    pub fn restore(&self, window: &Window) {
        if let Err(error) = main_window::restore(window, self.services.control.as_ref()) {
            eprintln!("[main-window] {error}");
        }
    }

    /// Minimizes, or hides to the tray when that preference is on.
    pub fn minimize(&self, window: &Window) {
        if let Err(error) = main_window::minimize(
            window,
            self.services.control.as_ref(),
            self.services.minimize_to_tray.load(Ordering::Acquire),
        ) {
            eprintln!("[main-window] {error}");
        }
    }

    pub fn restore_if_hidden(&self, window: &Window) {
        if let Ok(false) = self.services.control.is_visible(window) {
            self.restore(window);
        }
    }

    /// A host event for the shell itself. The state runtime of Phase 6 takes the rest.
    pub(crate) fn settings_preview(&self, settings: &Value, cx: &App) -> Value {
        self.runtime.output.read(cx).settings_drafts.shown(settings)
    }

    pub fn on_host_event(&mut self, event: &str, payload: &Value, cx: &mut Context<Self>) {
        crate::automation::timecode::event(&self.runtime, event, payload, cx);
        crate::automation::inputs::event(&self.runtime, event, payload, cx);
        let revisions = self.runtime.lanes.revisions();
        match event {
            "presenter-orbit:music-playback-ended" => self
                .media
                .update(cx, |media, cx| media.ended(payload.clone(), cx)),
            "output:displays-changed" => {
                crate::output::windows::refresh_displays(&self.runtime, cx);
            }
            "output:link-error" => {
                self.runtime.output.update(cx, |output, cx| {
                    output.link_error = payload.as_str().map(str::to_string);
                    cx.notify();
                });
            }
            "output:display-error" => {
                self.runtime.output.update(cx, |output, cx| {
                    output.device_error = payload.as_str().map(str::to_string);
                    cx.notify();
                });
            }
            "output-closed" => crate::output::windows::closed(&self.runtime, payload, cx),
            "output:link-changed" => {
                self.runtime.output.update(cx, |output, cx| {
                    output.link_changed();
                    if payload["attached"] == true {
                        output.link_error = None;
                    }
                    cx.notify();
                });
                crate::output::windows::refresh_displays(&self.runtime, cx);
            }
            // A settings event older than the known revision is dropped, as `onSettingsChanged` does.
            "settings-changed" => {
                if revisions.accept_settings_event(payload) {
                    self.mappings = KeyMappings::from_settings(payload);
                    self.runtime.output.update(cx, |output, cx| {
                        output.set_settings(payload.clone());
                        cx.notify();
                    });
                    cx.notify();
                }
            }
            "native-output-state-patch" => {
                self.runtime.output.update(cx, |output, cx| {
                    if let Err(error) = output.accept_native_patch(payload) {
                        output.error = Some(error);
                    }
                    cx.notify();
                });
            }
            "output:scene-committed" => {
                self.runtime.output.update(cx, |output, cx| {
                    match payload.as_u64().filter(|revision| *revision > 0) {
                        Some(revision) => {
                            output.committed_revision = output.committed_revision.max(revision)
                        }
                        None => {
                            output.error =
                                Some("The output scene event has no positive revision.".into())
                        }
                    }
                    cx.notify();
                });
            }
            "workspace-sync-changed" => {
                if let Err(error) = crate::settings::sync::accept(&self.runtime, payload, cx) {
                    self.runtime.status.update(cx, |status, cx| {
                        status.message = error;
                        cx.notify();
                    });
                }
            }
            "audio-inputs-changed" => {
                if let Err(error) =
                    crate::settings::audio_routing::accept_change(&self.runtime, payload, cx)
                {
                    self.runtime.status.update(cx, |status, cx| {
                        status.message = error;
                        cx.notify();
                    });
                }
                revisions.accept_workspace_event(payload, true);
            }
            // The registry changed, here or in another window of the application.
            "workspace-list-changed" => self
                .workspaces
                .update(cx, |workspaces, cx| workspaces.refresh(cx)),
            "settings-panel-requested" => request_auxiliary(
                self.host.clone(),
                json!({"kind":"settings","panel":payload.as_str().unwrap_or("general")}),
                cx.weak_entity(),
                cx,
            ),
            "timeline-soundtrack-status" => {
                if let Some(error) = payload["error"].as_str() {
                    crate::cues::actions::error(&self.runtime, cx, error.to_string());
                }
            }
            "remote-command" => crate::automation::remote::event(&self.runtime, payload, cx),
            "workspace-mcp-mutated" => self.runtime.accept_mcp_mutation(payload, cx),
            "automation-due" => crate::automation::on_due(&self.runtime, payload, cx),
            "trigger-bindings-changed" => {
                revisions.accept_workspace_event(payload, false);
            }
            _ => {}
        }
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.first_page_pending = false;
        self.status = Status::Loading;
        cx.notify();
        let (host, workspace) = (self.host.clone(), self.workspace.clone());
        let revisions = self.runtime.lanes.revisions().clone();
        cx.spawn_in(window, async move |view, cx| {
            let startup = cx
                .background_spawn(
                    async move { startup::load(&host, workspace.as_deref(), &revisions) },
                )
                .await;
            view.update_in(cx, |view, window, cx| view.apply(startup, window, cx))
        })
        .detach();
    }

    fn apply(&mut self, startup: Startup, window: &mut Window, cx: &mut Context<Self>) {
        crate::output::windows::refresh_displays(&self.runtime, cx);
        if let Some(settings) = startup.output_settings.as_ref() {
            self.runtime.output.update(cx, |output, cx| {
                output.set_settings((**settings).clone());
                cx.notify();
            });
        }
        cx.set_global(Translator::for_setting(startup.language.as_deref()));
        if let Some(appearance) = startup.appearance {
            set_appearance_settings(cx, appearance);
        }
        self.mappings = startup.mappings;
        // A window that failed to start is always shown, so the failure is visible.
        let mode = if startup.workspace.is_err() {
            crate::desktop::StartMode::Shown
        } else {
            startup.desktop.start()
        };
        if let Err(error) =
            main_window::apply_start_mode(window, self.services.control.as_ref(), mode)
        {
            eprintln!("[main-window] {error}");
        }
        window.focus(&self.focus, cx);
        // The first settings read registers the workspace it opened.
        self.workspaces
            .update(cx, |workspaces, cx| workspaces.refresh(cx));
        self.status = match startup.workspace {
            Ok(Some(snapshot)) => {
                self.runtime.content.update(cx, |content, cx| {
                    content.load(snapshot);
                    cx.notify();
                });
                self.runtime
                    .edit_ui(cx, |content, ui| ui.select_loaded(content));
                Status::Ready
            }
            Ok(None) => Status::NoWorkspace,
            Err(error) => Status::Failed(error),
        };
        if let Some(error) = &self.host_error {
            self.status = Status::Failed(error.clone());
        }
        self.first_page_pending = matches!(self.status, Status::Ready);
        if self.first_page_pending {
            startup::first_page::begin(cx.weak_entity(), window, cx);
        }
        cx.notify();
    }

    fn start_drag(&mut self, resize: Resize, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.drag = Some(DragState {
            resize,
            start_value: self.layout.get(resize.key()),
            start: (f32::from(position.x), f32::from(position.y)),
        });
        cx.notify();
    }

    fn drag_to(&mut self, position: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else { return };
        let size = window.viewport_size();
        let viewport = viewport_or_fallback(f32::from(size.width), f32::from(size.height));
        let next = next_value(Drag {
            resize: drag.resize,
            start_value: drag.start_value,
            start: drag.start,
            current: (f32::from(position.x), f32::from(position.y)),
            left_top: SHELL_TOP_HEIGHT,
        });
        self.layout = self.layout.resized(drag.resize, next, viewport);
        cx.notify();
    }

    /// The sizes are stored when the drag ends, not on every move.
    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_none() {
            return;
        }
        if let Some(path) = self.layout_path.clone() {
            let layout = self.layout;
            cx.background_spawn(async move {
                if let Err(error) = layout::save(&path, layout) {
                    eprintln!("{error}");
                }
            })
            .detach();
        }
        cx.notify();
    }

    fn failure(
        error: &str,
        t: Translator,
        heading: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("startup-failure")
            .flex()
            .flex_col()
            .gap(cx.theme().metrics.space_sm)
            .child(div().text_size(heading).child(t.t("errorBoundary.title")))
            .child(div().child(t.t("errorBoundary.description")))
            .child(div().child(t.t("errorBoundary.details")))
            .child(
                div()
                    .text_size(cx.theme().metrics.font_sm)
                    .child(error.to_string()),
            )
            .child(
                div()
                    .id("startup-reload")
                    .cursor_pointer()
                    .underline()
                    .on_click(cx.listener(|view, _, window, cx| view.load(window, cx)))
                    .child(t.t("errorBoundary.reload")),
            )
    }

    fn menu_entries(&self, id: MenuId, t: Translator, cx: &App) -> Vec<MenuEntry> {
        let editor = if self.canvas_open {
            match self.canvas.read(cx).kind() {
                crate::canvas::DocumentKind::Theme => Some("open-theme-canvas"),
                crate::canvas::DocumentKind::Prop => Some("open-prop-canvas"),
                crate::canvas::DocumentKind::Mask => Some("open-mask-canvas"),
                crate::canvas::DocumentKind::Stage => Some("open-stage-canvas"),
                crate::canvas::DocumentKind::Ccli => Some("open-ccli"),
                crate::canvas::DocumentKind::Slide => Some("open-slide-canvas"),
            }
        } else if self.cue_thumbnails.read(cx).song_editor_open() {
            Some("open-ccli")
        } else {
            None
        };
        let command = |label: String, command: &str, shortcut: Option<&str>| -> MenuEntry {
            let commands = self.commands.clone();
            let target = command.to_string();
            let item = MenuItem::new(label)
                .checked(editor == Some(command))
                .disabled(!self.commands.is_registered(command))
                .on_click(move |_, window, cx| {
                    commands.run(&target, false, window, cx);
                });
            let shortcut = self
                .mappings
                .shortcut(command)
                .map(format_shortcut_label)
                .or_else(|| shortcut.map(str::to_owned));
            match shortcut {
                Some(shortcut) if !shortcut.is_empty() => item.meta(shortcut),
                _ => item,
            }
            .into()
        };
        let output_group = |stage: bool| -> MenuEntry {
            let entries = self
                .runtime
                .output
                .read(cx)
                .snapshot
                .as_ref()
                .map(|snapshot| crate::output::view::screens(snapshot))
                .unwrap_or_default()
                .into_iter()
                .filter(|(id, config)| {
                    (config["type"] == "stage" || id.starts_with("stage-")) == stage
                })
                .map(|(id, config)| {
                    let runtime = self.runtime.clone();
                    let enabled = config["enabled"] == true;
                    MenuItem::new(config["label"].as_str().unwrap_or(&id).to_string())
                        .checked(enabled)
                        .on_click(move |_, _, cx| {
                            crate::output::windows::set_enabled(&runtime, id.clone(), !enabled, cx)
                        })
                        .into()
                })
                .collect::<Vec<_>>();
            MenuEntry::Submenu {
                label: t
                    .t(if stage {
                        "common.stage"
                    } else {
                        "common.audience"
                    })
                    .into(),
                disabled: entries.is_empty(),
                entries,
            }
        };
        match id {
            MenuId::Looks => {
                let looks = crate::output::looks::presets(&self.runtime, cx);
                let live = crate::output::looks::live_id(&self.runtime, cx);
                let mut entries: Vec<MenuEntry> = looks
                    .into_iter()
                    .map(|look| {
                        let runtime = self.runtime.clone();
                        let checked = look["id"].as_str() == live.as_deref();
                        MenuItem::new(look["title"].as_str().unwrap_or("").to_string())
                            .checked(checked)
                            .on_click(move |_, _, cx| {
                                let applied = crate::output::looks::screen_configs(&runtime, cx)
                                    .and_then(|configs| {
                                        crate::output::looks::activate(
                                            &runtime, &configs, &look, cx,
                                        )
                                    });
                                if let Err(error) = applied {
                                    runtime.status.update(cx, |status, cx| {
                                        status.message = error;
                                        cx.notify();
                                    });
                                }
                            })
                            .into()
                    })
                    .collect();
                if entries.is_empty() {
                    entries.push(
                        MenuItem::new(t.t("audienceLooks.empty"))
                            .disabled(true)
                            .into(),
                    );
                }
                entries.extend([
                    MenuEntry::Separator,
                    command(t.t("audienceLooks.settings"), "openLooks", None),
                ]);
                entries
            }
            MenuId::Audience => vec![output_group(false)],
            MenuId::Stage => vec![output_group(true)],
            MenuId::More => vec![
                command(t.t("toolbar.editor.stage"), "open-stage-canvas", None),
                command(t.t("toolbar.editor.theme"), "open-theme-canvas", None),
                command(t.t("toolbar.editor.ccli"), "open-ccli", None),
                command(t.t("toolbar.editor.props"), "open-prop-canvas", None),
                command(t.t("toolbar.editor.mask"), "open-mask-canvas", None),
            ],
            MenuId::File => vec![
                command(
                    t.t("appMenu.newPresentation"),
                    NEW_PRESENTATION,
                    Some("Ctrl+N"),
                ),
                command(t.t("appMenu.newPlaylist"), "new-playlist", None),
                command(t.t("appMenu.newLibrary"), "new-library", None),
                MenuEntry::Submenu {
                    label: t.t("appMenu.import").into(),
                    disabled: false,
                    entries: vec![
                        command(t.t("appMenu.importFiles"), "import-files", None),
                        command(t.t("appMenu.importClipboard"), "import-clipboard", None),
                        command(t.t("appMenu.importPowerPoint"), "open-bundles", None),
                        command(t.t("portableBundle.import"), "open-bundles", None),
                        command(
                            t.t("appMenu.importKeynotePresentation"),
                            "open-bundles",
                            None,
                        ),
                        command(t.t("appMenu.importPowerPointImages"), "open-bundles", None),
                    ],
                },
                MenuEntry::Separator,
                MenuEntry::Submenu {
                    label: t.t("appMenu.export").into(),
                    disabled: false,
                    entries: vec![
                        command(
                            t.t("presentationExport.title"),
                            "open-presentation-export",
                            None,
                        ),
                        command(t.t("portableBundle.export"), "open-bundles", None),
                    ],
                },
                MenuEntry::Separator,
                command(t.t("handout.print"), "open-handouts", None),
            ],
            MenuId::Edit => {
                let mut entries = Vec::new();
                for action in [
                    "undo",
                    "redo",
                    "-",
                    "cut",
                    "copy",
                    "paste",
                    "duplicate",
                    "delete",
                    "selectAll",
                    "deselectAll",
                ] {
                    if action == "-" {
                        entries.push(MenuEntry::Separator);
                        continue;
                    }
                    let commands = self.commands.clone();
                    let allowed = if self.canvas_open {
                        self.canvas.read(cx).can_edit(action)
                    } else {
                        !matches!(action, "undo" | "redo")
                            || (!self.cue_thumbnails.read(cx).reflow_open()
                                && self
                                    .documents
                                    .read(cx)
                                    .can_edit_theme_history(action == "undo", cx))
                    };
                    let mut item = MenuItem::new(t.t(&format!("appMenu.edit.{action}")))
                        .disabled(!allowed)
                        .on_click(move |_, window, cx| {
                            commands.run(action, false, window, cx);
                        });
                    if let Some(shortcut) = self.mappings.shortcut(action) {
                        item = item.meta(format_shortcut_label(shortcut));
                    }
                    entries.push(item.into());
                }
                entries.extend([
                    MenuEntry::Separator,
                    command(t.t("toolbar.search"), "open-search", Some("Ctrl+F")),
                    MenuEntry::Separator,
                    command(t.t("settings.nav.keys"), "open-key-settings", Some("Alt+K")),
                ]);
                entries
            }
            MenuId::Presentation => {
                let mut entries: Vec<MenuEntry> = [
                    "newSlide",
                    "nextSlide",
                    "previousSlide",
                    "nextBackground",
                    "previousBackground",
                ]
                .into_iter()
                .map(|id| {
                    command(
                        t.t(&format!("presentation.command.{id}")),
                        id,
                        self.mappings
                            .shortcut(id)
                            .map(format_shortcut_label)
                            .as_deref(),
                    )
                })
                .collect();
                let groups = self.runtime.content.read(cx).workspace["clearGroups"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let mut clears = vec![command(
                    t.t("presentation.command.clearAll"),
                    "clearAll",
                    self.mappings.shortcut("clearAll"),
                )];
                clears.extend(groups.iter().filter_map(|group| {
                    let id = group["id"].as_str()?.to_owned();
                    let title = group["title"].as_str()?.to_owned();
                    let runtime = self.runtime.clone();
                    Some(
                        MenuItem::new(title)
                            .on_click(move |_, _, cx| {
                                crate::cues::actions::clear_group(&runtime, &id, cx)
                            })
                            .into(),
                    )
                }));
                clears.push(command(
                    t.t("presentation.command.clearToLogo"),
                    "clearToLogo",
                    self.mappings.shortcut("clearToLogo"),
                ));
                entries.push(MenuEntry::Submenu {
                    label: t.t("clearGroups.title").into(),
                    disabled: false,
                    entries: clears,
                });
                for id in [
                    "clearSlide",
                    "clearMedia",
                    "clearProps",
                    "clearAudio",
                    "clearMessages",
                    "clearAnnouncements",
                ] {
                    entries.push(command(
                        t.t(&format!("presentation.command.{id}")),
                        id,
                        self.mappings
                            .shortcut(id)
                            .map(format_shortcut_label)
                            .as_deref(),
                    ));
                }
                entries.push(MenuEntry::Separator);
                for id in [
                    "showTriggeredPresentation",
                    "showTriggeredAnnouncement",
                    "revealInLibrary",
                    "stageOnly",
                ] {
                    entries.push(command(
                        t.t(&format!("presentation.command.{id}")),
                        id,
                        self.mappings
                            .shortcut(id)
                            .map(format_shortcut_label)
                            .as_deref(),
                    ));
                }
                entries
            }
            MenuId::View => {
                let mut entries = vec![
                    command(t.t("toolbar.show"), "show-workspace", Some("Ctrl+S")),
                    command(
                        t.t("appMenu.presentationEditor"),
                        "open-slide-canvas",
                        Some("Ctrl+E"),
                    ),
                    command(t.t("appMenu.reflowEditor"), "open-reflow", Some("Ctrl+R")),
                    command(
                        t.t("bible.dialogTitle"),
                        "open-bibles",
                        Some("Ctrl+Alt+Shift+B"),
                    ),
                    MenuEntry::Separator,
                ];
                let commands = self.commands.clone();
                entries.push(
                    MenuItem::new(t.t("appMenu.continuousPlaylist"))
                        .checked(self.cue_thumbnails.read(cx).continuous(cx))
                        .on_click(move |_, window, cx| {
                            commands.run("view-continuous", false, window, cx);
                        })
                        .into(),
                );
                entries.push(MenuEntry::Submenu {
                    label: t.t("appMenu.slideView").into(),
                    disabled: false,
                    entries: [
                        ("easy", "view-easy"),
                        ("grid", "view-grid"),
                        ("table", "view-table"),
                    ]
                    .into_iter()
                    .map(|(mode, id)| {
                        let commands = self.commands.clone();
                        MenuItem::new(t.t(&format!("cue.view.{mode}")))
                            .checked(self.cue_thumbnails.read(cx).view_mode() == mode)
                            .on_click(move |_, window, cx| {
                                commands.run(id, false, window, cx);
                            })
                            .into()
                    })
                    .collect(),
                });
                for (tab, label, shortcut) in [
                    ("audio", "media.audio", "Ctrl+Shift+A"),
                    ("stage", "utilityPanels.stage", "Alt+S"),
                    ("timers", "utilityPanels.timers", "Ctrl+Shift+C"),
                    ("messages", "automation.messages.title", "Ctrl+M"),
                    ("props", "utilityPanels.props", "Ctrl+Shift+P"),
                    ("macros", "automation.macros.title", "Ctrl+Shift+M"),
                ] {
                    entries.push(command(t.t(label), &format!("view-{tab}"), Some(shortcut)));
                }
                entries.extend([
                    MenuEntry::Separator,
                    command(
                        t.t("automation.timecode.title"),
                        "open-timecode",
                        Some("Ctrl+Shift+T"),
                    ),
                    command(t.t("appMenu.mediaBin"), "view-media", Some("Ctrl+Shift+V")),
                    MenuEntry::Separator,
                    command(t.t("testPatterns.title"), "open-screens", None),
                ]);
                let output = self.runtime.output.read(cx);
                let previews = output
                    .snapshot
                    .as_ref()
                    .map(|snapshot| crate::output::view::screens(snapshot))
                    .unwrap_or_default();
                let selected =
                    crate::output::view::selected_screen(&previews, &output.selected_screen)
                        .map(|(id, _)| id.clone());
                entries.push(MenuEntry::Submenu {
                    label: t.t("appMenu.preview").into(),
                    disabled: previews.is_empty(),
                    entries: previews
                        .into_iter()
                        .map(|(id, config)| {
                            let runtime = self.runtime.clone();
                            MenuItem::new(config["label"].as_str().unwrap_or(&id).to_owned())
                                .checked(selected.as_deref() == Some(id.as_str()))
                                .on_click(move |_, _, cx| {
                                    runtime.output.update(cx, |output, cx| {
                                        output.selected_screen = id.clone();
                                        cx.notify();
                                    })
                                })
                                .into()
                        })
                        .collect(),
                });
                entries
            }
            MenuId::Screens => vec![
                command(t.t("common.audience"), "toggle-audience", None),
                command(t.t("common.stage"), "toggle-stage", None),
                MenuEntry::Separator,
                command(t.t("screens.configure"), "open-screens", None),
                command(t.t("capture.title"), "open-screens", None),
                command(t.t("testPatterns.title"), "open-screens", None),
            ],
            MenuId::Window => vec![
                command(t.t("window.restore"), "window-restore", None),
                command(t.t("window.minimize"), "window-minimize", None),
                command(t.t("window.maximize"), "window-maximize", None),
                MenuEntry::Separator,
                MenuEntry::Submenu {
                    label: t.t("window.newOutput").into(),
                    disabled: self.runtime.output.read(cx).snapshot.is_none(),
                    entries: self
                        .runtime
                        .output
                        .read(cx)
                        .snapshot
                        .as_ref()
                        .map(|snapshot| crate::output::view::screens(snapshot))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(id, config)| {
                            let runtime = self.runtime.clone();
                            MenuItem::new(config["label"].as_str().unwrap_or(&id).to_string())
                                .on_click(move |_, _, cx| {
                                    crate::output::windows::create_windowed(
                                        &runtime,
                                        id.clone(),
                                        cx,
                                    )
                                })
                                .into()
                        })
                        .collect(),
                },
            ],
            MenuId::Workspace => {
                let mut entries: Vec<MenuEntry> = self
                    .workspaces
                    .read(cx)
                    .items
                    .iter()
                    .map(|item| {
                        let (workspaces, path) = (self.workspaces.clone(), item.path.clone());
                        MenuItem::new(item.name.clone())
                            .checked(item.active)
                            .disabled(item.missing)
                            .on_click(move |_, _, cx| {
                                workspaces.update(cx, |workspaces, cx| workspaces.switch(&path, cx))
                            })
                            .into()
                    })
                    .collect();
                let workspaces = self.workspaces.clone();
                entries.push(MenuEntry::Separator);
                entries.push(
                    MenuItem::new(t.t("workspace.manageMenu"))
                        .on_click(move |_, _, cx| {
                            workspaces.update(cx, |workspaces, cx| workspaces.manage(cx))
                        })
                        .into(),
                );
                entries
            }
            MenuId::Help => vec![
                command(t.t("settings.title"), "open-settings", None),
                command(t.t("settings.nav.keys"), "open-key-settings", None),
                MenuEntry::Separator,
                command(t.t("about.menuItem"), "open-about", None),
                command(t.t("about.licenses"), "open-licenses", None),
            ],
        }
    }

    fn menu_bar(&self, t: Translator, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let open = self.menu.as_ref().map(|menu| menu.id);
        div()
            .id("app-menu-bar")
            .debug_selector(|| "region-menu-bar".to_string())
            .flex()
            .flex_none()
            .items_stretch()
            .h(px(MENU_BAR_HEIGHT))
            .min_w_0()
            .overflow_hidden()
            .bg(theme.colors.surface_bg)
            .border_b_1()
            .border_color(theme.colors.border_strong)
            .child(
                div()
                    .flex()
                    .items_stretch()
                    .min_w_0()
                    .px(theme.metrics.space_sm)
                    .children(MenuId::ALL.into_iter().map(|id| {
                        let name = id.name();
                        div()
                            .flex()
                            .h_full()
                            .debug_selector(move || format!("menu-{name}"))
                            .child(
                                BarButton::new(
                                    SharedString::from(format!("menu-{}", id.name())),
                                    t.t(id.label_key()),
                                )
                                .active(open == Some(id))
                                .on_click({
                                    let view = cx.weak_entity();
                                    move |bounds: Bounds<Pixels>, _, cx| {
                                        view.update(cx, |view, cx| {
                                            let closing = view
                                                .menu
                                                .as_ref()
                                                .is_some_and(|menu| menu.id == id);
                                            view.menu = (!closing).then(|| OpenMenu {
                                                id,
                                                anchor: bounds.bottom_left(),
                                            });
                                            cx.notify();
                                        })
                                        .ok();
                                    }
                                }),
                            )
                    })),
            )
    }

    fn output_menu(
        &self,
        stage: bool,
        cx: &Context<Self>,
    ) -> impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static {
        let weak = cx.weak_entity();
        move |anchor, _, cx| {
            let _ = weak.update(cx, |view, cx| {
                view.menu = Some(OpenMenu {
                    id: if stage {
                        MenuId::Stage
                    } else {
                        MenuId::Audience
                    },
                    anchor,
                });
                cx.notify();
            });
        }
    }

    /// The live Audience Look on the toolbar, and the menu that switches it.
    fn look_switcher(&self, t: Translator, cx: &mut Context<Self>) -> impl IntoElement {
        let live = crate::output::looks::live_id(&self.runtime, cx);
        let name = crate::output::looks::presets(&self.runtime, cx)
            .into_iter()
            .find(|look| look["id"].as_str() == live.as_deref())
            .and_then(|look| look["title"].as_str().map(str::to_string))
            .unwrap_or_else(|| t.t("audienceLooks.title"));
        let name = if crate::output::looks::live_look(&self.runtime, cx)
            .is_some_and(|look| look.current != look.saved)
        {
            format!("{name}*")
        } else {
            name
        };
        let open = self
            .menu
            .as_ref()
            .is_some_and(|menu| menu.id == MenuId::Looks);
        let weak = cx.weak_entity();
        ToolButton::new(
            "tool-looks",
            "layout-grid",
            truncated(&name, WORKSPACE_LABEL_CHARS),
        )
        .active(open)
        .on_click_bounds(move |bounds, _, cx| {
            weak.update(cx, |view, cx| {
                let closing = view
                    .menu
                    .as_ref()
                    .is_some_and(|menu| menu.id == MenuId::Looks);
                view.menu = (!closing).then(|| OpenMenu {
                    id: MenuId::Looks,
                    anchor: gpui::point(bounds.left(), bounds.bottom()),
                });
                cx.notify();
            })
            .ok();
        })
    }

    /// `WorkspaceSwitcher.tsx`: the active workspace on the toolbar, and the menu that switches it.
    fn workspace_switcher(&self, t: Translator, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.workspaces.read(cx).active().map_or_else(
            || t.t("settings.general.workspace"),
            |item| item.name.clone(),
        );
        let open = self
            .menu
            .as_ref()
            .is_some_and(|menu| menu.id == MenuId::Workspace);
        let weak = cx.weak_entity();
        ToolButton::new(
            "tool-workspace",
            "folder",
            truncated(&name, WORKSPACE_LABEL_CHARS),
        )
        .active(open)
        .on_click(move |_, window, cx| {
            // The menu hangs from the toolbar's right end, where this button is.
            let x = window.viewport_size().width - px(WORKSPACE_MENU_WIDTH + 10.);
            weak.update(cx, |view, cx| {
                let closing = view
                    .menu
                    .as_ref()
                    .is_some_and(|menu| menu.id == MenuId::Workspace);
                if !closing {
                    view.workspaces
                        .update(cx, |workspaces, cx| workspaces.refresh(cx));
                }
                view.menu = (!closing).then(|| OpenMenu {
                    id: MenuId::Workspace,
                    anchor: gpui::point(x, px(MENU_BAR_HEIGHT + TOOL_BAR_HEIGHT)),
                });
                cx.notify();
            })
            .ok();
        })
    }

    fn tool_bar(&self, t: Translator, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let commands = self.commands.clone();
        let group = || {
            div()
                .flex()
                .items_stretch()
                .gap(theme.metrics.control_gap)
                .min_w_0()
                .py(theme.metrics.space_xs)
                .px(theme.metrics.panel_padding)
        };
        let editing =
            self.canvas_open && self.canvas.read(cx).kind() == crate::canvas::DocumentKind::Slide;
        let authoring = self.canvas_open || self.cue_thumbnails.read(cx).song_editor_open();
        let reflowing = !authoring && self.cue_thumbnails.read(cx).reflow_open();
        let tool = |id: &'static str, icon: &'static str, label: String, command: &'static str| {
            assert!(
                self.commands.is_registered(command),
                "Unregistered toolbar command: {command}"
            );
            let commands = self.commands.clone();
            ToolButton::new(id, icon, label).on_click(move |_, window, cx| {
                commands.run(command, false, window, cx);
            })
        };
        div()
            .id("tool-bar")
            .debug_selector(|| "region-tool-bar".to_string())
            .flex()
            .flex_none()
            .items_stretch()
            .h(px(TOOL_BAR_HEIGHT))
            .min_w_0()
            .bg(theme.colors.panel_bg_soft)
            .border_b_1()
            .border_color(theme.colors.border_strong)
            .child(
                group()
                    .flex_1()
                    .items_center()
                    .child(BrandButton::new("brand", APP_NAME, "settings").on_click(
                        move |_, window, cx| {
                            commands.run("open-settings", false, window, cx);
                        },
                    ))
                    .child(
                        ToolButton::new("tool-search", "search", t.t("toolbar.search")).on_click({
                            let commands = self.commands.clone();
                            move |_, window, cx| {
                                commands.run("open-search", false, window, cx);
                            }
                        }),
                    )
                    .child(
                        ToolButton::new("tool-show", "player-play", t.t("toolbar.show"))
                            .active(!authoring && !reflowing)
                            .on_click({
                                let commands = self.commands.clone();
                                move |_, window, cx| {
                                    commands.run("show-workspace", false, window, cx);
                                }
                            }),
                    )
                    .child(
                        ToolButton::new("tool-edit", "edit", t.t("toolbar.edit"))
                            .active(editing)
                            .on_click({
                                let commands = self.commands.clone();
                                move |_, window, cx| {
                                    commands.run("open-slide-canvas", false, window, cx);
                                }
                            }),
                    )
                    .child(
                        tool(
                            "tool-reflow",
                            "arrows-move-vertical",
                            t.t("toolbar.reflow"),
                            "open-reflow",
                        )
                        .active(reflowing),
                    )
                    .child(tool("tool-more", "dots", t.t("toolbar.more"), "open-more")),
            )
            .child(
                group()
                    .flex_1()
                    .justify_end()
                    .child(
                        tool(
                            "tool-media",
                            "polaroid",
                            t.t("media.graphics"),
                            "toggle-media",
                        )
                        .active(self.media_visible && !editing)
                        .disabled(editing),
                    )
                    .child(self.look_switcher(t, cx))
                    .child(
                        tool(
                            "tool-audience",
                            "circle",
                            t.t("common.audience"),
                            "toggle-audience",
                        )
                        .live(crate::output::windows::group_live(&self.runtime, false, cx))
                        .on_context_menu(self.output_menu(false, cx)),
                    )
                    .child(
                        tool("tool-stage", "circle", t.t("common.stage"), "toggle-stage")
                            .live(crate::output::windows::group_live(&self.runtime, true, cx))
                            .on_context_menu(self.output_menu(true, cx)),
                    )
                    .child(self.workspace_switcher(t, cx)),
            )
    }

    /// A region whose feature arrives in a later phase: its real header and an empty state.
    fn utility_content(&self, t: Translator, cx: &mut Context<Self>) -> gpui::AnyElement {
        use crate::automation::panels::Panel;
        let panel = match self.utility_tab {
            "macros" => Panel::Macros,
            "messages" => Panel::Messages,
            "timers" => Panel::Timers,
            "audio" => return self.audio_bin_panel(cx),
            "props" => return self.props.clone().into_any_element(),
            "stage" => return self.stage.clone().into_any_element(),
            _ => return self.unavailable(t).into_any_element(),
        };
        self.automation.update(cx, |panels, cx| {
            if panels.panel != panel {
                panels.panel = panel;
                cx.notify();
            }
        });
        self.automation.clone().into_any_element()
    }

    fn audio_bin_panel(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = *cx.global::<Translator>();
        let height = self.layout.get(LayoutKey::AudioPlaylistsHeight);
        self.audio_bin.update(cx, |audio, cx| {
            if audio.playlists_height != height {
                audio.playlists_height = height;
                cx.notify();
            }
        });
        let collapsed = self.media.read(cx).audio_collapsed;
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(self.audio_bin.clone())
            .when(!collapsed, |panel| {
                panel.child(
                    self.divider(Resize::AudioPlaylists, t.t("collections.resizeLists"), cx)
                        .left_0()
                        .right_0()
                        .top(px(height - DIVIDER / 2. + 0.5)),
                )
            })
            .into_any_element()
    }

    fn unavailable(&self, t: Translator) -> impl IntoElement {
        EmptyState::new(t.t("shell.region.unavailable"))
    }

    fn divider(&self, resize: Resize, label: String, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = *cx.theme();
        let active = self.drag.is_some_and(|drag| drag.resize == resize);
        let vertical = !resize.is_horizontal_divider();
        div()
            .id(SharedString::from(format!("divider-{resize:?}")))
            .debug_selector(move || format!("divider-{resize:?}"))
            .absolute()
            .flex_none()
            .when(vertical, |this| this.w(px(DIVIDER)).cursor_col_resize())
            .when(!vertical, |this| this.h(px(DIVIDER)).cursor_row_resize())
            .when(active, |this| this.bg(theme.colors.accent))
            .tooltip(orbit_ui::controls::tooltip_text(label))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                    view.start_drag(resize, event.position, cx);
                    cx.stop_propagation();
                }),
            )
    }

    fn left_browser(&self, t: Translator, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let top = self.layout.get(LayoutKey::LeftBrowserTopHeight);
        div()
            .debug_selector(|| "region-left".to_string())
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(self.layout.get(LayoutKey::LeftWidth)))
            .min_h_0()
            .border_r_1()
            .border_color(theme.colors.border_strong)
            .bg(theme.colors.panel_bg)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .h(px(top))
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.trees.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(theme.colors.border_strong)
                    .child(self.documents.clone()),
            )
            .child(
                self.divider(Resize::LeftBrowserTop, t.t("collections.resizeLists"), cx)
                    .left_0()
                    .right_0()
                    .top(px(top - DIVIDER / 2. + 0.5)),
            )
    }

    fn cue_workspace(&self, width: f32, t: Translator, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let center_min = center_minimum_width(width);
        div()
            .debug_selector(|| "region-center".to_string())
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(center_min))
            .min_h_0()
            .bg(theme.colors.stage_bg)
            .border_r_1()
            .border_color(theme.colors.border_strong)
            .when(self.canvas_open, |el| {
                let commands = self.commands.clone();
                el.child(
                    PanelHeader::new(t.t(self.canvas.read(cx).editor_label())).action(
                        ToolButton::new("close-theme-canvas", "arrow-left", t.t("common.close"))
                            .on_click(move |_, window, cx| {
                                commands.run("close-canvas", false, window, cx);
                            }),
                    ),
                )
            })
            .child(div().flex_1().min_h_0().child(if self.canvas_open {
                self.canvas.clone().into_any_element()
            } else {
                self.cue_thumbnails.clone().into_any_element()
            }))
    }

    fn right_inspector(&self, t: Translator, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let strip = || div().flex_none().h(px(DIVIDER));
        let view = cx.weak_entity();
        div()
            .debug_selector(|| "region-right".to_string())
            .flex()
            .flex_col()
            .flex_none()
            .w(px(self.layout.get(LayoutKey::RightWidth)))
            .min_h_0()
            .relative()
            .bg(theme.colors.panel_bg)
            .child(
                div()
                    .flex_none()
                    .h(px(self.layout.get(LayoutKey::RightOutputHeight)))
                    .bg(theme.colors.preview_matte_bg)
                    .child(self.output_preview.clone()),
            )
            .child(
                strip().child(
                    self.divider(Resize::RightOutput, t.t("output.resizePreview"), cx)
                        .left_0()
                        .right_0()
                        .top(px(-3.)),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .h(px(self.layout.get(LayoutKey::RightTransportHeight)))
                    .min_h_0()
                    .child(tab_list(
                        &view,
                        "transport-tabs",
                        t.t("output.layers"),
                        &TRANSPORT_TABS,
                        self.transport_tab,
                        |view, tab| view.transport_tab = tab,
                        t,
                    ))
                    .child({
                        let layer = self.transport_tab;
                        self.transport.update(cx, |transport, cx| {
                            if transport.layer != layer {
                                transport.layer = layer;
                                cx.notify();
                            }
                        });
                        self.transport.clone()
                    }),
            )
            .child(
                strip()
                    .relative()
                    .bg(theme.colors.panel_bg_soft)
                    .border_b_1()
                    .border_color(theme.colors.border_soft)
                    .child(
                        self.divider(Resize::RightTransport, t.t("transport.resizePanel"), cx)
                            .left_0()
                            .right_0()
                            .top(px(0.)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(tab_list(
                        &view,
                        "utility-tabs",
                        t.t("utilityPanels.label"),
                        &UTILITY_PANELS,
                        self.utility_tab,
                        |view, tab| view.utility_tab = tab,
                        t,
                    ))
                    .child(self.utility_content(t, cx)),
            )
    }

    fn media_bin(&self, t: Translator, cx: &mut Context<Self>) -> impl IntoElement {
        let width = self.layout.get(LayoutKey::MediaSidebarWidth);
        let sidebar_visible = !self.media.read(cx).sidebar_collapsed;
        self.media.update(cx, |media, cx| {
            if media.sidebar_width != width {
                media.sidebar_width = width;
                cx.notify();
            }
        });
        div()
            .debug_selector(|| "region-media".to_string())
            .relative()
            .flex_none()
            .h(px(self.layout.get(LayoutKey::MediaHeight)))
            .min_w_0()
            .child(self.media.clone())
            .when(sidebar_visible, |el| {
                el.child(
                    self.divider(Resize::MediaSidebarWidth, t.t("media.resizeLists"), cx)
                        .absolute()
                        .left(px(width))
                        .top_0()
                        .bottom_0(),
                )
            })
    }

    fn shell(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let (left, right, media) = (
            self.layout.get(LayoutKey::LeftWidth),
            self.layout.get(LayoutKey::RightWidth),
            if self.media_visible {
                self.layout.get(LayoutKey::MediaHeight)
            } else {
                0.
            },
        );
        div()
            .id("app-shell")
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .bg(theme.colors.shell_bg)
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_md)
            .child(self.menu_bar(t, cx))
            .child(crate::external::strip(&self.runtime, &self.external, cx))
            .child(self.tool_bar(t, cx))
            .child(if self.canvas_open {
                let (sidebar, inspector) = (
                    self.layout.get(LayoutKey::ThemeSlideListWidth),
                    self.layout.get(LayoutKey::ThemeInspectorWidth),
                );
                self.canvas.update(cx, |canvas, cx| {
                    if canvas.sidebar_width != sidebar || canvas.inspector_width != inspector {
                        canvas.sidebar_width = sidebar;
                        canvas.inspector_width = inspector;
                        cx.notify();
                    }
                });
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(self.canvas.clone())
                    .child(
                        self.divider(
                            Resize::ThemeSlideListWidth,
                            t.t("shell.resizeEditorSidebar"),
                            cx,
                        )
                        .top_0()
                        .bottom_0()
                        .left(px(sidebar - 3.)),
                    )
                    .child(
                        self.divider(
                            Resize::ThemeInspectorWidth,
                            t.t("shell.resizeInspector"),
                            cx,
                        )
                        .top_0()
                        .bottom_0()
                        .right(px(inspector - 4.)),
                    )
                    .into_any_element()
            } else {
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_h_0()
                                    .child(self.left_browser(t, cx))
                                    .child(self.cue_workspace(width, t, cx)),
                            )
                            .when(self.media_visible, |this| this.child(self.media_bin(t, cx))),
                    )
                    .child(self.right_inspector(t, cx))
                    .child(
                        self.divider(Resize::LeftWidth, t.t("shell.resizeLibrary"), cx)
                            .top_0()
                            .bottom(px(media))
                            .left(px(left - 3.)),
                    )
                    .child(
                        self.divider(Resize::RightWidth, t.t("shell.resizeInspector"), cx)
                            .top_0()
                            .bottom_0()
                            .right(px(right - 4.)),
                    )
                    .when(self.media_visible, |this| {
                        this.child(
                            self.divider(Resize::MediaHeight, t.t("shell.resizeMedia"), cx)
                                .left_0()
                                .right(px(right))
                                .bottom(px(media - 4.)),
                        )
                    })
                    .into_any_element()
            })
    }
}

/// `minmax(480px, 1fr)` for the center column, relaxed by the two media queries of `responsive-ui.css`.
fn center_minimum_width(width: f32) -> f32 {
    if width <= 980. {
        320.
    } else if width <= 1280. {
        360.
    } else {
        480.
    }
}

pub fn request_auxiliary(
    host: Host,
    request: Value,
    view: WeakEntity<AppView>,
    cx: &mut gpui::App,
) {
    cx.spawn(async move |cx| {
        let result = cx
            .background_spawn(async move { host.invoke("openAuxiliaryWindow", vec![request]) })
            .await;
        if let Err(error) = result {
            view.update(cx, |view, cx| {
                view.window_error = Some(error);
                cx.notify();
            })
            .ok();
        }
    })
    .detach();
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The hidden startup window gets its drawable size only on its first frame.
        let viewport = window.viewport_size();
        if !self.layout_loaded && viewport.width > px(0.) && viewport.height > px(0.) {
            let viewport =
                viewport_or_fallback(f32::from(viewport.width), f32::from(viewport.height));
            self.layout = self
                .layout_path
                .as_deref()
                .map_or_else(Layout::default, |path| layout::load(path, viewport));
            self.layout_loaded = true;
        }
        if !self.canvas_open && self.canvas.read(cx).is_active() {
            self.canvas.update(cx, |canvas, cx| {
                canvas.suspend(window, cx);
            });
        }
        if self.first_page_pending {
            // Build and paint the shell behind the loading overlay before revealing it.
            let owner = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = owner.update(cx, |view, cx| {
                    let output = view.runtime.output.read(cx);
                    if view.first_page_pending
                        && (output.snapshot.is_some() || output.error.is_some())
                        && startup::first_page::ready(cx)
                    {
                        view.first_page_pending = false;
                        startup::first_page::finish(cx);
                        cx.notify();
                    }
                });
            });
        }
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let width = f32::from(window.viewport_size().width);
        let root = div()
            .id("app-root")
            .relative()
            .key_context("Shell")
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.colors.bg)
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_base);
        let view = cx.entity();
        let heading = theme.metrics.font_xl;
        match &self.status {
            Status::Loading => root
                .child(BusyDialog::new("workspace-loading", t.t("status.workspace.loading")).page())
                .into_any_element(),
            Status::Failed(error) => {
                let error = error.clone();
                root.p(theme.metrics.space_lg)
                    .child(Self::failure(&error, t, heading, cx))
                    .into_any_element()
            }
            Status::NoWorkspace | Status::Ready => {
                let menu = self.menu.as_ref().map(|menu| {
                    let id = menu.id;
                    ContextMenu::new(
                        SharedString::from(format!("app-menu-{}", id.name())),
                        menu.anchor,
                        self.menu_entries(id, t, cx),
                        close_menu(cx.weak_entity()),
                    )
                    .width(match id {
                        MenuId::Looks => theme.metrics.font_md * 22.,
                        MenuId::Workspace => px(WORKSPACE_MENU_WIDTH),
                        _ => px(MENU_WIDTH),
                    })
                });
                let error = self.window_error.clone().map(|error| {
                    Dialog::new(
                        "window-error",
                        t.t("window.openFailed"),
                        t.t("common.close"),
                        close_error(cx.weak_entity()),
                    )
                    .width(px(420.))
                    .child(div().child(error))
                });
                let key_view = view.clone();
                let runtime = self.runtime.clone();
                root.on_key_down(move |event, window, cx| {
                    let (commands, mappings, cues) = {
                        let view = key_view.read(cx);
                        if view.first_page_pending {
                            return;
                        }
                        (
                            view.commands.clone(),
                            view.mappings.clone(),
                            view.cue_thumbnails.clone(),
                        )
                    };
                    if dispatch_key_down(&commands, &mappings, event, window, cx)
                        || (!event.is_held
                            && !orbit_ui::controls::text_entry_focused(window)
                            && (crate::automation::run_hotkey(&runtime, &event.keystroke, cx)
                                || cues.update(cx, |cues, cx| {
                                    cues.group_hotkey(&event.keystroke, cx)
                                })))
                    {
                        cx.stop_propagation();
                    }
                })
                .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, window, cx| {
                    view.drag_to(event.position, window, cx)
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|view, _, _, cx| view.end_drag(cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|view, _, _, cx| view.end_drag(cx)),
                )
                .child(self.shell(width, cx))
                .children(menu)
                .child(self.search.clone())
                .children(error)
                .children(self.screen_canvas_open.then(|| self.screen_canvas.clone()))
                .children(self.tickers.clone())
                .children(self.bibles.clone())
                .children(self.transfers.clone())
                .children(self.imports.clone())
                .children(self.charts.clone())
                .child(self.cue_forms.clone())
                .children(self.announcements.clone())
                .children(self.timeline.clone())
                .children(self.slide_authoring.clone())
                .child(self.confirm.clone())
                .child(self.startup_update.clone())
                .children(self.workspaces.read(cx).switching.clone().map(|name| {
                    BusyDialog::new(
                        "workspace-switching",
                        t.format("workspace.switching", &[("name", &name)]),
                    )
                }))
                .when(self.first_page_pending, |root| {
                    root.child(
                        div()
                            .id("first-page-loading")
                            .debug_selector(|| "first-page-loading".into())
                            .absolute()
                            .inset_0()
                            .occlude()
                            .bg(theme.colors.bg)
                            .child(
                                BusyDialog::new(
                                    "workspace-loading",
                                    t.t("status.workspace.loading"),
                                )
                                .page(),
                            ),
                    )
                })
                .into_any_element()
            }
        }
    }
}

fn close_menu(view: WeakEntity<AppView>) -> impl Fn((), &mut Window, &mut gpui::App) + 'static {
    move |_, window, cx| {
        view.update(cx, |view, cx| {
            view.menu = None;
            // The menu held focus; give it back so keyboard focus is not lost with the menu.
            window.focus(&view.focus, cx);
            cx.notify();
        })
        .ok();
    }
}

fn close_error(view: WeakEntity<AppView>) -> impl Fn((), &mut Window, &mut gpui::App) + 'static {
    move |_, _, cx| {
        view.update(cx, |view, cx| {
            view.window_error = None;
            cx.notify();
        })
        .ok();
    }
}

fn tab_list(
    view: &WeakEntity<AppView>,
    id: &'static str,
    label: String,
    items: &[(&'static str, &'static str)],
    active: &'static str,
    set: fn(&mut AppView, &'static str),
    t: Translator,
) -> TabList<&'static str> {
    let items = items
        .iter()
        .map(|(value, key)| {
            let tab = TabItem::new(*value, t.t(key));
            match (id, *value) {
                ("utility-tabs", "audio") => tab.icon("music"),
                ("utility-tabs", "props") => tab.icon("stack-front"),
                ("utility-tabs", "timers") => tab.icon("clock"),
                ("utility-tabs", "stage") => tab.icon("microphone"),
                ("utility-tabs", "macros") => tab.icon("square-letter-m"),
                ("utility-tabs", "messages") => tab.icon("send"),
                ("transport-tabs", _) => tab.leading_icon("music"),
                _ => tab,
            }
        })
        .collect();
    let view = view.clone();
    TabList::new(id, label, items, active).on_change(move |tab, _, cx| {
        view.update(cx, |view, cx| {
            set(view, tab);
            cx.notify();
        })
        .ok();
    })
}
