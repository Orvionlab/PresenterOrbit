//! The audio bin of the utility panel: audio playlists above the tracks of the selected one (`AudioBinBridge`).
use super::*;

pub(super) struct AudioFade {
    library: String,
    saved: f64,
    state: Entity<orbit_ui::controls::SliderState>,
    _subscription: Subscription,
}

/// The Audio tab of the right panel. It draws the audio half of the shared media bin.
pub struct AudioBin {
    media: Entity<MediaBin>,
    /// The height of the playlists section, set by the shell from the layout.
    pub playlists_height: f32,
    _subscription: Subscription,
}

impl AudioBin {
    pub fn new(media: Entity<MediaBin>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&media, |_, _, cx| cx.notify());
        Self {
            media,
            playlists_height: 120.,
            _subscription: subscription,
        }
    }
}

impl Render for AudioBin {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = self.playlists_height;
        self.media
            .update(cx, |media, cx| media.audio_panel(height, window, cx))
    }
}

impl MediaBin {
    fn audio_option(&mut self, id: &str, key: &str, value: Value, cx: &mut Context<Self>) {
        let mut libraries = self.libraries.clone();
        if let Some(library) = libraries.iter_mut().find(|library| library["id"] == id) {
            library[key] = value;
            self.mutate("saveMediaLibraries", vec![json!(libraries)], cx);
        }
    }
    pub(super) fn follow_audio_playlist(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let request = this.update(cx, |this, cx| {
                if !this.playlist.active() {
                    return None;
                }
                let output = this.runtime.output.read(cx);
                let screen = output.snapshot.as_ref().and_then(|snapshot| {
                    crate::output::view::screens(snapshot)
                        .into_iter()
                        .find(|(_, config)| config["type"] == "audience")
                        .map(|(id, _)| id)
                })?;
                Some((
                    this.runtime.host.clone(),
                    this.workspace.clone(),
                    output.live["audioSelectionId"].clone(),
                    screen,
                ))
            });
            let Ok(request) = request else {
                break;
            };
            let Some((host, workspace, selection, screen)) = request else {
                continue;
            };
            let answer = cx
                .background_spawn(async move {
                    crate::state::runtime::output_call(
                        &host,
                        "getNativeMusicPlayback",
                        vec![json!(screen)],
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.workspace != workspace
                    || this.runtime.output.read(cx).live["audioSelectionId"] != selection
                {
                    return;
                }
                match answer {
                    Ok(answer) => this.runtime.output.update(cx, |output, cx| {
                        let mut live = (*output.live).clone();
                        this.playlist.reconcile(
                            &this.workspace,
                            &this.media,
                            &this.libraries,
                            &mut live,
                            &answer["playback"],
                        );
                        if live != *output.live {
                            output.edit_live(|current| *current = live);
                            cx.notify();
                        }
                    }),
                    Err(error) => this.runtime.status.update(cx, |status, cx| {
                        if status.message != error {
                            status.message = error;
                            cx.notify();
                        }
                    }),
                }
            });
        })
        .detach();
    }
    pub(super) fn filter_audio_rows(&mut self) {
        let query = self.audio_filter_query.trim().to_lowercase();
        self.audio_visible = self
            .audio_rows
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                text(item, "title")
                    .to_lowercase()
                    .contains(&query)
                    .then_some(index)
            })
            .collect();
    }

    fn audio_filter_control(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let input = self
            .audio_filter_input
            .get_or_insert_with(|| {
                let input =
                    orbit_ui::controls::filter_state(self.audio_filter_query.clone(), window, cx);
                self._subscriptions.push(cx.subscribe(
                    &input,
                    |view, input, event: &orbit_ui::controls::InputEvent, cx| {
                        if matches!(event, orbit_ui::controls::InputEvent::Change) {
                            view.audio_filter_query = input.read(cx).value().to_string();
                            view.filter_audio_rows();
                            cx.notify();
                        }
                    },
                ));
                input
            })
            .clone();
        let library = self
            .libraries
            .iter()
            .find(|library| library["id"] == self.audio_library && library["type"] == "playlist")
            .cloned();
        let t = *cx.global::<Translator>();
        let fade = library.as_ref().map(|library| {
            let id = text(library, "id").to_string();
            if self
                .audio_fade
                .as_ref()
                .is_none_or(|fade| fade.library != id)
            {
                let state = orbit_ui::controls::range_state(
                    0.,
                    10.,
                    0.1,
                    library["audioCrossfadeSeconds"].as_f64().unwrap_or(0.) as f32,
                    cx,
                );
                let owner = id.clone();
                let subscription = cx.subscribe(
                    &state,
                    move |this, _, event: &orbit_ui::controls::SliderEvent, cx| {
                        if let orbit_ui::controls::SliderEvent::Release(value) = event {
                            if let Some(library) = this
                                .libraries
                                .iter()
                                .find(|library| library["id"] == owner)
                                .cloned()
                            {
                                this.audio_option(
                                    text(&library, "id"),
                                    "audioCrossfadeSeconds",
                                    json!(value.end()),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    },
                );
                self.audio_fade = Some(AudioFade {
                    library: id,
                    saved: library["audioCrossfadeSeconds"].as_f64().unwrap_or(0.),
                    state: state.clone(),
                    _subscription: subscription,
                });
            }
            if let Some(fade) = &mut self.audio_fade {
                let seconds = library["audioCrossfadeSeconds"].as_f64().unwrap_or(0.);
                if fade.saved != seconds {
                    fade.saved = seconds;
                    fade.state.update(cx, |slider, cx| {
                        slider.set_value(seconds as f32, window, cx)
                    });
                }
            }
            self.audio_fade.as_ref().unwrap().state.clone()
        });
        let theme = *cx.theme();
        div()
            .debug_selector(|| "audio-filter-bar".into())
            .flex_none()
            .flex()
            .items_center()
            .gap(theme.metrics.space_xs)
            .p(theme.metrics.space_xs)
            .border_t_1()
            .border_color(theme.colors.border_soft)
            .bg(theme.colors.panel_bg)
            .child(
                IconButton::new(
                    "audio-shuffle",
                    "arrows-shuffle",
                    t.t("media.audioPlaylist.shuffle"),
                )
                .selected(
                    library
                        .as_ref()
                        .is_some_and(|library| library["audioShuffle"] == true),
                )
                .disabled(library.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(library) = library.clone() {
                        this.audio_option(
                            text(&library, "id"),
                            "audioShuffle",
                            json!(library["audioShuffle"] != true),
                            cx,
                        );
                    }
                })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(orbit_ui::controls::FilterInput::new("audio-filter", &input)),
            )
            .when_some(fade, |bar, state| {
                let seconds = state.read(cx).value().end();
                bar.child(orbit_ui::icon("transition-right").text_color(theme.colors.text_muted))
                    .child(
                        div()
                            .debug_selector(|| "audio-crossfade-control".into())
                            .w(theme.metrics.space_lg * 5.)
                            .child(
                                orbit_ui::controls::RangeInput::new(&state)
                                    .label(t.t("media.audioPlaylist.crossfade")),
                            ),
                    )
                    .child(
                        div()
                            .text_size(theme.metrics.font_xs)
                            .child(format!("{seconds:.1}s")),
                    )
            })
            .into_any_element()
    }

    pub(super) fn audio_panel(
        &mut self,
        height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.refresh(cx);
        let t = *cx.global::<Translator>();
        let theme = *cx.theme();
        let collapsed = self.audio_collapsed;
        let playlists = div()
            .flex()
            .flex_col()
            .flex_none()
            .min_h_0()
            .when(!collapsed, |section| section.h(gpui::px(height)))
            .child(
                PanelHeader::new(t.t("media.audio"))
                    .prominent()
                    .action(
                        IconButton::new("audio-create", "plus", t.t("media.action.createAudio"))
                            .large_icon()
                            .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                                this.audio = true;
                                this.menu_position = None;
                                this.library_menu = Some((event.position(), true));
                                cx.notify();
                            })),
                    )
                    .action(
                        IconButton::new(
                            "audio-playlists-toggle",
                            if collapsed {
                                "chevron-down"
                            } else {
                                "chevron-up"
                            },
                            t.t("media.audioPlaylistsToggle"),
                        )
                        .large_icon()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.audio_collapsed = !this.audio_collapsed;
                            cx.notify();
                        })),
                    ),
            )
            .when(!collapsed, |section| {
                let rows = self.library_rows(true, cx);
                let tree = div()
                    .id("audio-libraries-scroll")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows);
                section.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .child(self.library_surface(tree, cx)),
                )
            });
        let library = self
            .libraries
            .iter()
            .find(|library| library["id"] == self.audio_library && library["type"] == "playlist")
            .cloned();
        let count = self.audio_visible.len();
        let title = if count == 1 {
            t.t("media.audioItems.one")
        } else {
            t.format("media.audioItems.many", &[("count", &count.to_string())])
        };
        let items_header = PanelHeader::new(title).prominent();
        let items = if count > 0 {
            gpui::uniform_list(
                "audio-items",
                count,
                cx.processor(|this, range, _, cx| this.audio_item_rows(range, cx)),
            )
            .track_scroll(&self.audio_scroll)
            .flex_1()
            .min_h_0()
            .into_any_element()
        } else if !self.audio_rows.is_empty() {
            div()
                .flex_1()
                .min_h_0()
                .child(orbit_ui::controls::EmptyState::new(
                    t.t("collections.filter.empty"),
                ))
                .into_any_element()
        } else if let Some(library) = library {
            let id = text(&library, "id").to_string();
            div()
                .flex()
                .flex_1()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(theme.metrics.space_sm)
                .text_color(theme.colors.text_muted)
                .text_size(theme.metrics.font_sm)
                .child(
                    div()
                        .id("audio-import")
                        .debug_selector(|| "audio-import".to_string())
                        .role(gpui::Role::Button)
                        .aria_label(t.t("media.audioImport"))
                        .tab_index(0)
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(gpui::px(80.))
                        .border_1()
                        .border_color(theme.colors.border_strong)
                        .bg(theme.colors.panel_bg_soft)
                        .text_color(theme.colors.text)
                        .cursor_pointer()
                        .hover(|button| button.bg(theme.colors.selected))
                        .child(orbit_ui::icon("arrow-down").size(gpui::px(36.)))
                        .on_click(cx.listener(move |this, _, _, cx| this.import_audio(&id, cx))),
                )
                .child(t.t("media.audioImport"))
                .into_any_element()
        } else {
            div()
                .id("audio-no-playlist")
                .debug_selector(|| "audio-no-playlist".to_string())
                .flex_1()
                .child(orbit_ui::controls::EmptyState::new(t.t("media.noPlaylist")))
                .into_any_element()
        };
        let painted = self.audio_grid_bounds.clone();
        let items = div()
            .id("audio-items-gesture")
            .relative()
            .flex()
            .flex_1()
            .min_h_0()
            .child(items)
            .child(
                gpui::canvas(
                    move |bounds, _, _| *painted.borrow_mut() = Some(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                this.graphics_move(event.position, cx)
            }))
            .on_drag_move::<tree::AssetDrag>(cx.listener(
                |this, event: &gpui::DragMoveEvent<tree::AssetDrag>, _, cx| {
                    this.graphics_move(event.event.position, cx)
                },
            ))
            .on_drop::<tree::AssetDrag>(cx.listener(|this, _, _, cx| {
                this.graphics_finish(cx);
                cx.stop_propagation();
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.graphics_finish(cx)),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this
                        .graphics_press
                        .as_ref()
                        .is_some_and(|press| press.audio)
                    {
                        this.graphics_cancel(cx);
                    }
                }),
            )
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.graphics_cancel(cx);
                    cx.stop_propagation();
                }
            }));
        div()
            .id("audio-bin")
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(theme.colors.panel_bg)
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_sm)
            .child(playlists)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(theme.colors.border_strong)
                    .child(items_header)
                    .child(items)
                    .child(self.audio_filter_control(window, cx)),
            )
            .into_any_element()
    }

    /// `importAudioFiles`: pick audio files natively and add them to the playlist.
    fn import_audio(&mut self, library: &str, cx: &mut Context<Self>) {
        if self.busy || self.workspace.is_empty() {
            return;
        }
        let (host, workspace, library) = (
            self.runtime.host.clone(),
            self.workspace.clone(),
            library.to_string(),
        );
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { host.invoke("selectAudioFiles", vec![]) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.workspace != workspace {
                    return;
                }
                match result {
                    Ok(result) if result["canceled"] != true => {
                        let mut media = this.media.clone();
                        media.extend(
                            result["media"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .cloned()
                                .map(|mut item| {
                                    item["libraryId"] = json!(library);
                                    item
                                }),
                        );
                        this.mutate("saveMediaSettings", vec![json!(media)], cx);
                    }
                    Err(error) => this.runtime.status.update(cx, |status, cx| {
                        status.message = error;
                        cx.notify();
                    }),
                    _ => {}
                }
            });
        })
        .detach();
    }
}
