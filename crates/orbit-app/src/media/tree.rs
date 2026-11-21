use super::*;
use gpui::{Div, Stateful};
use orbit_ui::controls::{list_row_without_hover as list_row, ContextMenu, InputEvent, MenuItem};

pub(super) struct LibraryRename {
    pub(super) id: String,
    pub(super) input: Entity<InputState>,
    _subscription: Subscription,
}

#[derive(Clone)]
pub(super) struct LibraryDrag {
    pub id: String,
    pub audio: bool,
    pub title: String,
}
#[derive(Clone)]
pub(crate) struct AssetDrag {
    pub(crate) id: String,
    pub(super) title: String,
    pub(crate) workspace: String,
    pub(crate) slide_media: std::sync::Arc<[String]>,
}
pub(super) struct DragLabel(pub String);
impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        div()
            .px(theme.metrics.space_sm)
            .py(theme.metrics.space_2xs)
            .bg(theme.colors.popover_bg)
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_sm)
            .child(self.0.clone())
    }
}
fn visible(libraries: &[Value], audio: bool) -> Vec<(Value, usize)> {
    fn branch(
        libraries: &[Value],
        parent: &str,
        depth: usize,
        audio: bool,
        seen: &mut std::collections::HashSet<String>,
        rows: &mut Vec<(Value, usize)>,
    ) {
        for library in libraries.iter().filter(|library| {
            text(library, "id") != "workspace"
                && text(library, "parentId") == parent
                && (library["mediaType"].is_null()
                    || library["mediaType"] == if audio { "audio" } else { "graphics" })
        }) {
            let id = text(library, "id");
            if !seen.insert(id.to_string()) {
                continue;
            }
            rows.push((library.clone(), depth));
            if library["collapsed"] != true {
                branch(libraries, id, depth + 1, audio, seen, rows);
            }
        }
    }
    let mut rows = vec![];
    branch(libraries, "", 0, audio, &mut Default::default(), &mut rows);
    rows
}
impl MediaBin {
    pub(super) fn file_unassigned(&mut self, cx: &mut Context<Self>) {
        if self.workspace.is_empty() {
            return;
        }
        let unassigned = self
            .media
            .iter()
            .any(|item| text(item, "libraryId").is_empty())
            || self
                .backgrounds
                .iter()
                .any(|item| text(item, "libraryId").is_empty());
        if !unassigned {
            return;
        }
        let mut libraries = self.libraries.clone();
        if !libraries
            .iter()
            .any(|library| library["id"] == "default-graphics")
        {
            libraries.push(json!({"id":"default-graphics","title":"Default","type":"playlist","mediaType":"graphics","parentId":"","collapsed":false,"itemFiles":[]}));
        }
        let mut media = self.media.clone();
        for item in &mut media {
            if text(item, "libraryId").is_empty() {
                item["libraryId"] = json!(if item["type"] == "audio" {
                    "default-audio"
                } else {
                    "default-graphics"
                });
            }
        }
        if media
            .iter()
            .any(|item| item["libraryId"] == "default-audio")
            && !libraries
                .iter()
                .any(|library| library["id"] == "default-audio")
        {
            libraries.push(json!({"id":"default-audio","title":"Default","type":"playlist","mediaType":"audio","parentId":"","collapsed":false,"itemFiles":[]}));
        }
        let patch = json!({"media":media,"mediaLibraries":libraries});
        let runtime = self.runtime.clone();
        let result =
            crate::automation::save::optimistic_content(&runtime, patch, cx, |this, result, cx| {
                if let Err(error) = result {
                    this.runtime.status.update(cx, |status, cx| {
                        status.message = error;
                        cx.notify();
                    });
                }
            });
        if let Err(error) = result {
            self.runtime.status.update(cx, |status, cx| {
                status.message = error;
                cx.notify();
            });
            return;
        }
        self.media = media;
        self.libraries = libraries;
        let mut backgrounds = self.backgrounds.clone();
        for background in &mut backgrounds {
            if text(background, "libraryId").is_empty() {
                background["libraryId"] = json!("default-graphics");
            }
        }
        if backgrounds != self.backgrounds {
            self.backgrounds = backgrounds.clone();
            self.runtime.output.update(cx, |output, cx| {
                output.edit_live(|live| live["backgrounds"] = json!(backgrounds));
                cx.notify();
            });
        }
    }
    fn move_library(
        &mut self,
        source: &str,
        target: &str,
        placement: &str,
        audio: bool,
        cx: &mut Context<Self>,
    ) {
        match orbit_host::plan_media_library_move(
            &self.libraries,
            source,
            target,
            placement,
            if audio { "audio" } else { "graphics" },
        ) {
            Ok(libraries) => self.mutate("saveMediaLibraries", vec![json!(libraries)], cx),
            Err(error) => self.runtime.status.update(cx, |status, cx| {
                status.message = error;
                cx.notify();
            }),
        }
    }
    fn toggle_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        let mut libraries = self.libraries.clone();
        if let Some(folder) = libraries.iter_mut().find(|library| library["id"] == id) {
            folder["collapsed"] = json!(folder["collapsed"] != true);
            self.mutate("saveMediaLibraries", vec![json!(libraries)], cx);
        }
    }
    fn create_library(
        &mut self,
        kind: &str,
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.edits_blocked() || self.workspace.is_empty() {
            return;
        }
        let titles: Vec<_> = self
            .libraries
            .iter()
            .map(|library| text(library, "title"))
            .collect();
        let title = crate::collections::model::unique_title(title, &titles);
        let id = format!("media-library-{}", uuid::Uuid::new_v4());
        let mut libraries = self.libraries.clone();
        libraries.push(json!({
            "id": id, "title": title, "type": kind,
            "mediaType": if self.audio { "audio" } else { "graphics" },
            "parentId": "", "collapsed": false, "itemFiles": []
        }));
        *self.library_for_mut(self.audio) = id;
        self.mutate("saveMediaLibraries", vec![json!(libraries)], cx);
        self.begin_library_rename(self.library_for(self.audio).clone(), window, cx);
        cx.notify();
    }

    pub(super) fn begin_library_rename(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(library) = self.libraries.iter().find(|item| item["id"] == id) else {
            return;
        };
        let input = input_state(text(library, "title").to_string(), window, cx);
        let owner = id.clone();
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. })
                    && this
                        .library_rename
                        .as_ref()
                        .is_some_and(|rename| rename.id == owner)
                {
                    let title = input.read(cx).value().trim().to_string();
                    this.library_rename = None;
                    if !title.is_empty() {
                        let mut libraries = this.libraries.clone();
                        if let Some(library) =
                            libraries.iter_mut().find(|library| library["id"] == owner)
                        {
                            library["title"] = json!(title);
                            this.mutate("saveMediaLibraries", vec![json!(libraries)], cx);
                        }
                    }
                    cx.notify();
                }
            },
        );
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        self.library_rename = Some(LibraryRename {
            id,
            input,
            _subscription: subscription,
        });
    }
    /// The user confirmed: the library and its media leave the bin; the asset files stay in the workspace.
    pub(super) fn delete_confirmed(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.confirm_delete.take() else {
            return;
        };
        // Output must not keep pointing at media this delete removes.
        let family = model::library_family(&self.libraries, &id);
        let mut media = self.media.clone();
        media.extend(self.backgrounds.clone());
        self.runtime.output.update(cx, |output, cx| {
            output.edit_live(|live| {
                for (layer, key) in [("media", "backgroundId"), ("music", "audioId")] {
                    if model::in_family(&media, &family, &live[key]) {
                        crate::cues::actions::clear_patch(live, layer);
                    }
                }
            });
            cx.notify();
        });
        if model::in_family(&self.media, &family, &json!(self.selected)) {
            self.playlist.stop();
            self.audition(false, cx);
        }
        self.mutate(
            "mutateMediaLibrary",
            vec![json!({"type":"delete","libraryId":id})],
            cx,
        );
        cx.notify();
    }
    /// Whether live output or the audition plays media the delete of this library removes.
    pub(super) fn plays_from(&self, id: &str, cx: &gpui::App) -> bool {
        let family = model::library_family(&self.libraries, id);
        let live = &self.runtime.output.read(cx).live;
        model::in_family(&self.media, &family, &live["backgroundId"])
            || model::in_family(&self.backgrounds, &family, &live["backgroundId"])
            || model::in_family(&self.media, &family, &live["audioId"])
            || (self.preview_playing
                && model::in_family(&self.media, &family, &json!(self.selected)))
    }
    /// Deleting a playlist or folder removes its media from the bin, so the user confirms first.
    pub(super) fn delete_dialog(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let title = self
            .libraries
            .iter()
            .find(|library| library["id"].as_str() == self.confirm_delete.as_deref())
            .map(|library| text(library, "title").to_string())?;
        let t = *cx.global::<Translator>();
        let playing = self.plays_from(self.confirm_delete.as_deref()?, cx);
        let weak = cx.weak_entity();
        let close = move |_: &mut Window, cx: &mut gpui::App| {
            let _ = weak.update(cx, |this, cx| {
                this.confirm_delete = None;
                cx.notify();
            });
        };
        Some(
            Dialog::new(
                "media-delete-dialog",
                t.t("app.name"),
                t.t("common.cancel"),
                move |_, window, cx| close(window, cx),
            )
            .child(t.format("media.library.deleteConfirm", &[("title", &title)]))
            .when(playing, |dialog| {
                dialog.child(t.t("media.library.deletePlaying"))
            })
            .footer(
                ActionButton::new("media-delete-cancel", t.t("common.cancel"))
                    .variant(ButtonVariant::Secondary)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_delete = None;
                        cx.notify();
                    })),
            )
            .footer(
                ActionButton::new("media-delete-ok", t.t("common.delete"))
                    .variant(ButtonVariant::Danger)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.delete_confirmed(cx))),
            )
            .into_any_element(),
        )
    }
    pub(super) fn library_menu(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let (position, create) = self.library_menu?;
        let t = *cx.global::<Translator>();
        let options = if create {
            vec![
                ("playlist", "media.create.playlist"),
                ("folder", "media.create.folder"),
                ("smart-playlist", "media.create.smartPlaylist"),
            ]
        } else {
            vec![
                ("duplicate", "common.duplicate"),
                ("rename", "common.rename"),
                ("delete", "common.delete"),
            ]
        };
        let entries = options
            .into_iter()
            .map(|(action, key)| {
                let weak = cx.weak_entity();
                let library = self
                    .library_menu_id
                    .clone()
                    .unwrap_or_else(|| self.context_library().clone());
                MenuItem::new(t.t(key))
                    .disabled(self.edits_blocked())
                    .on_click(move |_, window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.library_menu = None;
                            if action == "rename" {
                                this.begin_library_rename(library.clone(), window, cx);
                            } else if action == "smart-playlist" {
                                this.smart_library(cx);
                            } else if create {
                                this.create_library(action, &t.t(key), window, cx);
                            } else if !create && action == "delete" {
                                this.confirm_delete = Some(library.clone());
                            } else {
                                this.mutate(
                                    "mutateMediaLibrary",
                                    vec![json!({"type":action,"libraryId":library})],
                                    cx,
                                );
                            }
                            cx.notify();
                        });
                    })
                    .into()
            })
            .collect();
        let weak = cx.weak_entity();
        Some(
            ContextMenu::new("media-library-menu", position, entries, move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.library_menu = None;
                    cx.notify();
                });
            })
            .into_any_element(),
        )
    }
    /// The rows of one library tree: the graphics sidebar and the audio utility tab draw the same rows.
    pub(super) fn library_rows(
        &mut self,
        audio: bool,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let mut rows_out = vec![];
        let rows = visible(&self.libraries, audio);
        for (library, depth) in rows {
            let id = text(&library, "id").to_string();
            let title = text(&library, "title").to_string();
            let selected = self.library_for(audio) == &id;
            let drag = LibraryDrag {
                id: id.clone(),
                audio,
                title: title.clone(),
            };
            let collapse = id.clone();
            let drop = id.clone();
            let over = id.clone();
            let add = id.clone();
            let menu_id = id.clone();
            let key_id = id.clone();
            let label_id = id.clone();
            let row_id = id.clone();
            let folder = library["type"] == "folder";
            let collapsed = library["collapsed"] == true;
            let mut row = list_row(
                SharedString::from(format!("media-library-{id}")),
                selected,
                cx,
            )
            .relative()
            .debug_selector(move || format!("media-library-row-{row_id}"))
            .hover(|style| {
                style.bg(if selected {
                    theme.colors.selected
                } else {
                    theme.colors.panel_bg
                })
            })
            .pl(theme.metrics.space_xs + theme.metrics.space_lg * depth as f32);
            let disclosure = if folder {
                orbit_ui::controls::NativeButton::new(SharedString::from(format!("collapse-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("media-folder-toggle-{id}")
                    })
                    .accessibility_label(t.t(if collapsed {
                        "media.folder.open"
                    } else {
                        "media.folder.close"
                    }))
                    .size(theme.metrics.space_lg)
                    .flex_none()
                    .focus_visible(|style| style.border_1().border_color(theme.colors.focus_ring))
                    .child(
                        orbit_ui::icon(if collapsed {
                            "chevron-right"
                        } else {
                            "chevron-down"
                        })
                        .text_color(theme.colors.text_muted),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_folder(&collapse, cx);
                    }))
                    .into_any_element()
            } else {
                div()
                    .w(theme.metrics.space_lg)
                    .flex_none()
                    .into_any_element()
            };
            row = row.child(disclosure);
            let label = if let Some(rename) = self
                .library_rename
                .as_ref()
                .filter(|rename| rename.id == id)
            {
                TextInput::new("media-library-rename", &rename.input)
                    .compact()
                    .into_any_element()
            } else {
                div().truncate().child(title).into_any_element()
            };
            row = row.when(
                self.library_drop
                    .as_ref()
                    .is_some_and(|(target, _)| target == &id),
                |row| {
                    let mode = self.library_drop.as_ref().map(|(_, mode)| mode.as_str());
                    row.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .when(mode == Some("inside"), |line| {
                                line.debug_selector(|| "media-library-drop-inside".to_string())
                                    .inset_0()
                                    .border_2()
                                    .border_color(theme.colors.accent)
                            })
                            .when(mode != Some("inside"), |line| {
                                line.debug_selector(|| "media-library-drop-line".to_string())
                                    .h(theme.metrics.space_2xs)
                                    .bg(theme.colors.accent)
                                    .when(mode == Some("before"), |line| line.top_0())
                                    .when(mode == Some("after"), |line| line.bottom_0())
                            }),
                    )
                },
            );
            row = row
                .role(gpui::Role::Button)
                .aria_label(text(&library, "title").to_string())
                .aria_selected(selected)
                .tab_index(0)
                .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        this.library_rename = None;
                        cx.notify();
                    }
                    if this.library_rename.is_none()
                        && matches!(event.keystroke.key.as_str(), "enter" | "space")
                    {
                        if folder {
                            this.toggle_folder(&key_id, cx);
                        } else {
                            this.audio = audio;
                            this.library_menu_id = None;
                            *this.library_for_mut(audio) = key_id.clone();
                            this.rebuild();
                            cx.notify();
                        }
                    }
                }))
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        this.audio = audio;
                        this.library_menu_id = Some(menu_id.clone());
                        this.menu_position = None;
                        this.library_menu = Some((event.position, false));
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(
                    orbit_ui::icon(if folder {
                        "folder"
                    } else if library["type"] == "smart" {
                        "bulb"
                    } else if audio {
                        "list"
                    } else {
                        "polaroid"
                    })
                    .text_color(theme.colors.text_muted),
                )
                .child(
                    div()
                        .debug_selector(move || format!("media-library-label-{label_id}"))
                        .flex_1()
                        .min_w_0()
                        .child(label),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if folder {
                        this.toggle_folder(&id, cx);
                        return;
                    }
                    this.audio = audio;
                    this.library_menu_id = None;
                    *this.library_for_mut(audio) = id.clone();
                    this.selected.clear();
                    this.rebuild();
                    cx.notify();
                }))
                .on_drag(drag, |drag, _, _, cx| {
                    cx.new(|_| DragLabel(drag.title.clone()))
                })
                .on_drag_move::<LibraryDrag>(cx.listener(
                    move |this, event: &gpui::DragMoveEvent<LibraryDrag>, _, cx| {
                        if event.drag(cx).audio != audio
                            || !event.bounds.contains(&event.event.position)
                        {
                            return;
                        }
                        cx.stop_propagation();
                        if event.drag(cx).id == over {
                            this.library_drop = None;
                            cx.notify();
                            return;
                        }
                        let height = f32::from(event.bounds.size.height);
                        let offset = f32::from(event.event.position.y - event.bounds.origin.y);
                        let ratio = if height > 0. { offset / height } else { 0. };
                        let mode = if folder && ratio > 0.28 && ratio < 0.72 {
                            "inside"
                        } else if ratio < 0.5 {
                            "before"
                        } else {
                            "after"
                        };
                        this.library_drop = Some((over.clone(), mode.into()));
                        cx.notify();
                    },
                ))
                .on_drop::<LibraryDrag>(cx.listener(move |this, drag: &LibraryDrag, _, cx| {
                    cx.stop_propagation();
                    if drag.audio != audio || drag.id == drop {
                        return;
                    }
                    let Some((target, placement)) = this
                        .library_drop
                        .take()
                        .filter(|(target, _)| target == &drop)
                    else {
                        return;
                    };
                    cx.notify();
                    this.move_library(&drag.id, &target, &placement, audio, cx);
                }))
                .on_drop::<AssetDrag>(cx.listener(move |this, drag: &AssetDrag, _, cx| {
                    this.mutate(
                        "mutateMediaLibrary",
                        vec![json!({"type":"addAsset","mediaId":drag.id,"libraryId":add})],
                        cx,
                    )
                }));
            rows_out.push(row.into_any_element());
        }
        rows_out
    }
    /// Escape, drag feedback, and the drop that moves a library to the root, shared by both library trees.
    pub(super) fn library_surface(
        &mut self,
        body: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        body.capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
            if event.keystroke.key == "escape" {
                this.library_rename = None;
                this.library_menu = None;
                this.library_drop = None;
                cx.stop_propagation();
                cx.notify();
            }
        }))
        .on_drag_move::<LibraryDrag>(cx.listener(|this, _, _, cx| {
            this.library_drop = None;
            cx.notify();
        }))
        .on_mouse_up_out(
            gpui::MouseButton::Left,
            cx.listener(|_, _, _, cx| {
                // Outside handlers run during capture, before a child row consumes the drop.
                let weak = cx.weak_entity();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if this.library_drop.take().is_some() {
                            cx.notify();
                        }
                    });
                });
            }),
        )
        .on_drop::<LibraryDrag>(cx.listener(|this, drag: &LibraryDrag, _, cx| {
            this.move_library(&drag.id, "", "root", drag.audio, cx)
        }))
        .into_any_element()
    }
    pub(super) fn sidebar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let mut body = div()
            .id("media-libraries-scroll")
            .debug_selector(|| "media-libraries".to_string())
            .flex()
            .flex_col()
            .min_h_0()
            .overflow_y_scroll()
            .w(gpui::px(self.sidebar_width))
            .flex_none()
            .border_r_1()
            .border_color(theme.colors.border);
        let key = "media.graphics";
        body = body.child(PanelHeader::new(t.t(key)).action(
            IconButton::new("create-false", "plus", t.t("media.action.createGraphics")).on_click(
                cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                    this.audio = false;
                    this.menu_position = None;
                    this.library_menu = Some((event.position(), true));
                    cx.notify();
                }),
            ),
        ));
        for row in self.library_rows(false, cx) {
            body = body.child(row);
        }
        // Deliberately below the graphics libraries, unlike React: the maintainer asked for it here.
        let video_inputs_selected = self.library == crate::media::video_inputs::VIDEO_INPUT_LIBRARY;
        body = body.child(
            list_row("media-library-video-inputs", video_inputs_selected, cx)
                .pl(theme.metrics.space_xs * 2. + theme.metrics.space_lg)
                .hover(|style| {
                    style.bg(if video_inputs_selected {
                        theme.colors.selected
                    } else {
                        theme.colors.panel_bg
                    })
                })
                .cursor_pointer()
                .child(orbit_ui::icon("video").text_color(theme.colors.text_muted))
                .child(t.t("videoInputs.title"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.audio = false;
                    this.library = crate::media::video_inputs::VIDEO_INPUT_LIBRARY.to_string();
                    this.rebuild();
                    cx.notify();
                })),
        );
        self.library_surface(body, cx)
    }
}
