use super::{
    seed,
    support::{self, click, context, Scenario},
};
use gpui::{AppContext, TestAppContext};
use serde_json::{json, Value};

#[gpui::test]
fn theme_previews_assign_to_collections_and_undo_a_whole_presentation(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, seed);
    let snapshot = app.snapshot();
    let library = snapshot["libraries"][0]["id"].as_str().unwrap();
    let playlist = snapshot["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|playlist| playlist["title"] == "Sunday")
        .unwrap();
    let playlist = playlist["id"].as_str().unwrap();
    let item = snapshot["library"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "Sunday service")
        .unwrap();
    let item = item["id"].as_str().unwrap();
    let slides = |app: &Scenario| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == item)
            .unwrap()["slides"]
            .clone()
    };
    let before = slides(&app);
    let workspace = json!(app.workspace.0.join("workspace.json"));
    let revision = app
        .host
        .invoke("readWorkspaceSnapshot", vec![workspace.clone()])
        .unwrap()["revision"]
        .clone();
    let error = app
        .host
        .invoke(
            "mutatePresentationLibrary",
            vec![
                workspace,
                json!({
                    "type":"mutateItem", "itemId":item,
                    "operation":{"type":"setSlideDocument", "expectedSlides":false, "slides":before}
                }),
                revision,
            ],
        )
        .unwrap_err();
    assert!(error.contains("Presentation slides changed"));
    assert_eq!(slides(&app), before);

    let runtime = app.runtime(cx);
    let (thumbnail, assignment) = cx.update(|cx| {
        let content = runtime.content.read(cx);
        let recent =
            crate::theme_picker::known_recents(&content.recent_themes, &content.themes).len();
        content
            .themes
            .iter()
            .flat_map(|theme| {
                theme
                    .slides
                    .iter()
                    .map(move |(slide, _)| format!("{}::{slide}", theme.id))
            })
            .enumerate()
            .find_map(|(index, assignment)| {
                (before[0]["presentationThemeId"] != assignment)
                    .then(|| (format!("theme-picker-all-{}", recent + index), assignment))
            })
            .unwrap()
    });
    let mut visual = app.visual(cx);
    let open = |visual: &mut gpui::VisualTestContext, row: &str| {
        context(visual, row);
        if row.starts_with("document-") {
            let resize = visual.debug_bounds("menu-item-Resize").unwrap();
            let themes = visual.debug_bounds("menu-item-Themes").unwrap();
            assert!(resize.bottom() <= themes.top());
        }
        click(visual, "menu-item-Themes");
    };

    click(&mut visual, &format!("library-{library}"));
    open(&mut visual, "document-0");
    assert!(visual
        .debug_bounds(Box::leak(thumbnail.clone().into_boxed_str()))
        .is_some());
    click(&mut visual, "dialog-close");

    click(&mut visual, &format!("playlist-{playlist}"));
    open(&mut visual, "document-0");
    click(&mut visual, &thumbnail);
    app.wait(cx, |app, _| {
        slides(app)
            .as_array()
            .unwrap()
            .iter()
            .all(|slide| slide["presentationThemeId"] == assignment)
    });
    let after = slides(&app);
    let history_ready = |app: &Scenario, cx: &mut TestAppContext, undo| {
        cx.update(|cx| {
            app.main
                .view
                .read(cx)
                .documents
                .read(cx)
                .can_edit_theme_history(undo, cx)
        })
    };
    app.wait(cx, |app, cx| history_ready(app, cx, true));
    support::primary_shortcut(&mut visual, "z");
    app.wait(cx, |app, cx| {
        slides(app) == before && history_ready(app, cx, false)
    });
    let redo = if cfg!(target_os = "macos") {
        "shift-z"
    } else {
        "y"
    };
    support::primary_shortcut(&mut visual, redo);
    app.wait(cx, |app, cx| {
        slides(app) == after && history_ready(app, cx, true)
    });

    open(&mut visual, &format!("library-{library}"));
    click(&mut visual, "theme-picker-recent-0");
    app.wait(cx, |app, _| {
        app.snapshot()["libraries"][0]["presentationThemeId"] == assignment
    });

    open(&mut visual, &format!("playlist-{playlist}"));
    click(&mut visual, "theme-picker-recent-0");
    app.wait(cx, |app, _| {
        app.snapshot()["playlist"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| {
                candidate["id"] == playlist && candidate["presentationThemeId"] == assignment
            })
    });
}

#[gpui::test]
fn linked_placeholder_takes_its_own_arrangement_and_edits_slide_labels(cx: &mut TestAppContext) {
    let arrangement = "a039ef8e-b9c7-4c11-b788-3b9726a262b6";
    let mut app = Scenario::new(cx, true, |host| {
        seed(host);
        let settings = host.invoke("getSettings", vec![]).unwrap();
        let path = settings["presenterOrbitWorkspacePath"].clone();
        let mut labels = settings["generalSettings"]["slideLabels"]
            .as_array()
            .unwrap()
            .clone();
        labels.push(json!({"id":"plain","name":"Plain","color":"transparent"}));
        let payload =
            crate::settings::persist::general_payload(&settings, &json!({"slideLabels":labels}));
        host.invoke(
            "saveSettings",
            vec![payload, settings["settingsRevision"].clone()],
        )
        .unwrap();
        let settings = host.invoke("getSettings", vec![]).unwrap();
        let screen = settings["screenConfig"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, config)| config["type"] == "audience")
            .unwrap()
            .0;
        let loaded = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        host.invoke(
            "mutateScreenConfig",
            vec![
                path.clone(),
                json!({"type":"update","screenId":screen,
            "patch":{"contentDestination":"announcements","lookLayers":{"announcements":true}}}),
                settings["settingsRevision"].clone(),
                loaded["revision"].clone(),
            ],
        )
        .unwrap();
        let settings = host.invoke("getSettings", vec![]).unwrap();
        let loaded = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let added = host
            .invoke(
                "mutateScreenConfig",
                vec![
                    path.clone(),
                    json!({"type":"add","screenType":"audience","baseLabel":"Main preview"}),
                    settings["settingsRevision"].clone(),
                    loaded["revision"].clone(),
                ],
            )
            .unwrap();
        let current = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        host.invoke(
            "mutateScreenConfig",
            vec![
                path.clone(),
                json!({"type":"update","screenId":added["createdScreenId"],
                "patch":{"lookLayers":{"announcements":false}}}),
                added["settingsRevision"].clone(),
                current["revision"].clone(),
            ],
        )
        .unwrap();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let playlist = snapshot["workspace"]["playlist"]
            .as_array()
            .unwrap()
            .iter()
            .find(|playlist| playlist["title"] == "Sunday")
            .unwrap();
        let item = &playlist["items"][0]["itemId"];
        let created = host
            .invoke(
                "mutatePresentationLibrary",
                vec![
                    path.clone(),
                    json!({"type":"createArrangement","itemId":item,"name":"Closing only",
                "newArrangementId":arrangement,"sectionIds":["section-2"]}),
                    snapshot["revision"].clone(),
                ],
            )
            .unwrap();
        let selected = host.invoke("mutatePresentationLibrary", vec![
            path.clone(),
            json!({"type":"setSelectedArrangement","itemId":item,"arrangementId":arrangement}),
            created["revision"].clone(),
        ]).unwrap();
        host.invoke("mutatePresentationPlaylist", vec![
            path,
            json!({"type":"createEntry","playlistId":playlist["id"],"entryType":"placeholder","title":"Linked item"}),
            selected["revision"].clone(),
        ]).unwrap();
    });
    let snapshot = app.snapshot();
    let playlist = snapshot["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|playlist| playlist["title"] == "Sunday")
        .unwrap();
    let playlist_id = playlist["id"].as_str().unwrap();
    let entry = playlist["items"][1]["id"].as_str().unwrap();
    let item_id = &playlist["items"][0]["itemId"];
    let runtime = app.runtime(cx);
    app.wait(cx, |_, cx| {
        cx.update(|cx| runtime.output.read(cx).committed_revision > 0)
    });
    let mut visual = app.visual(cx);
    click(&mut visual, &format!("playlist-{playlist_id}"));
    click(&mut visual, "document-1");
    click(&mut visual, &format!("button-cue-link-{entry}"));
    visual.simulate_input("Opening line");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("search-result-0").is_some()
    });
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-1").is_some()
    });

    click(&mut visual, "cue-thumbnail-0");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            let live = &runtime.output.read(cx).live;
            live["livePlaylistEntryId"] == entry && live["liveSlide"]["content"] == "Opening line"
        })
    });
    context(&mut visual, "cue-thumbnail-0");
    click(&mut visual, "menu-item-Label");
    click(&mut visual, "menu-item-Repeat");
    app.wait(cx, |app, cx| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == *item_id && item["slides"][0]["label"] == "Repeat")
            && cx.update(|cx| runtime.output.read(cx).live["liveSlide"]["label"] == "Repeat")
    });

    click(&mut visual, &format!("button-cue-arrangement-{entry}"));
    click(&mut visual, "button-cue-arrangement");
    click(&mut visual, "menu-item-Closing only");
    app.wait(cx, |app, cx| {
        let mut visual = app.visual(cx);
        visual.debug_bounds("cue-thumbnail-0").is_some()
            && visual.debug_bounds("cue-thumbnail-1").is_none()
    });
    click(&mut visual, "cue-thumbnail-0");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            let live = &runtime.output.read(cx).live;
            live["livePlaylistEntryId"] == entry && live["liveSlide"]["content"] == "Closing line"
        })
    });
    let release = support::hold_workspace_writes(&app, cx);
    context(&mut visual, "cue-thumbnail-0");
    click(&mut visual, "menu-item-Label");
    click(&mut visual, "menu-item-Plain");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-label-0").is_some()
    });
    assert!(app.snapshot()["library"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == *item_id && item["slides"][1]["label"].is_null()));
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["id"] == *item_id
                    && item["slides"][1]["label"] == "Plain"
                    && item["slides"][1]["labelColor"] == "transparent"
            })
    });

    click(&mut visual, "button-cue-arrangement");
    click(&mut visual, "menu-item-Master");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-1").is_some()
    });
    click(&mut visual, "cue-thumbnail-0");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-live-0").is_some()
    });
    click(&mut visual, "button-cue-destination");
    click(&mut visual, "menu-item-Announcements");
    app.wait(cx, |app, _| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == *item_id && item["destination"] == "announcements")
    });
    let (screen, main_screen) = cx.update(|cx| {
        let screens =
            crate::output::view::screens(runtime.output.read(cx).snapshot.as_ref().unwrap());
        let main = screens
            .iter()
            .find(|(_, config)| {
                config["type"] == "audience" && config["contentDestination"] != "announcements"
            })
            .unwrap()
            .0
            .clone();
        runtime.output.update(cx, |output, cx| {
            output.selected_screen = main.clone();
            cx.notify();
        });
        let announcement = screens
            .into_iter()
            .find(|(_, config)| config["contentDestination"] == "announcements")
            .unwrap()
            .0;
        (announcement, main)
    });
    for (index, text) in [(1, "Closing line"), (0, "Opening line")] {
        click(&mut visual, &format!("cue-thumbnail-{index}"));
        let live_cue = Box::leak(format!("cue-announcement-live-{index}").into_boxed_str());
        app.wait(cx, |app, cx| {
            cx.update(|cx| {
                runtime.output.read(cx).live["announcements"]["slide"]["content"] == text
            }) && app.visual(cx).debug_bounds(live_cue).is_some()
                && crate::state::runtime::output_call(
                    &app.host,
                    "getOutputContent",
                    vec![json!(screen)],
                )
                .is_ok_and(|scene| scene["text"] == text)
                && crate::state::runtime::output_call(
                    &app.host,
                    "getOutputContent",
                    vec![json!(main_screen)],
                )
                .is_ok_and(|scene| {
                    scene["text"] == "Opening line" && scene["announcementOverlay"]["text"] == text
                })
        });
        assert!(visual.debug_bounds("cue-live-0").is_some());
        assert!(visual
            .debug_bounds("look-layer-announcements-live")
            .is_some());
        assert!(visual.debug_bounds("program-preview").is_some());
        assert!(visual.debug_bounds("announcement-preview").is_none());
        let scene = crate::state::runtime::output_call(
            &app.host,
            "getOutputContent",
            vec![json!(main_screen)],
        )
        .unwrap();
        let frame = orbit_host::native_compositor::NativeCompositor::new()
            .try_render_layered_for("announcement-overlay", &scene, 320, 180)
            .unwrap();
        let announcement = frame
            .layers
            .iter()
            .position(|layer| layer.id.starts_with("announcement-") && layer.bounds.is_some())
            .unwrap();
        assert!(!frame.layers[announcement].replace);
        assert!(frame.layers[..announcement].iter().any(|layer| {
            (layer.id.starts_with("theme-layer-") || layer.id == "fallback-text")
                && layer.bounds.is_some()
        }));
    }
    click(&mut visual, "look-layer-announcements");
    app.wait(cx, |app, cx| {
        app.visual(cx)
            .debug_bounds("look-layer-announcements-live")
            .is_none()
            && crate::state::runtime::output_call(
                &app.host,
                "getOutputContent",
                vec![json!(main_screen)],
            )
            .is_ok_and(|scene| {
                scene["announcementOverlay"].is_null() && scene["text"] == "Opening line"
            })
    });
    let release = support::hold_workspace_writes(&app, cx);
    click(&mut visual, "button-cue-destination");
    click(&mut visual, "menu-item-Presentation");
    click(&mut visual, "cue-thumbnail-1");
    app.wait(cx, |_, cx| {
        cx.update(|cx| runtime.output.read(cx).live["liveSlide"]["content"] == "Closing line")
    });
    assert!(app.snapshot()["library"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == *item_id && item["destination"] == "announcements"));
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == *item_id && item["destination"].is_null())
    });
}

#[gpui::test]
fn single_presentation_mode_shows_playlist_markers_and_links_placeholder(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, seed);
    let snapshot = app.snapshot();
    let playlist = snapshot["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .position(|playlist| playlist["title"] == "Sunday")
        .unwrap();
    let playlist_id = snapshot["playlist"][playlist]["id"].as_str().unwrap();
    let original = snapshot["playlist"][playlist]["items"][0]["itemId"].clone();
    let items = |app: &Scenario| app.snapshot()["playlist"][playlist]["items"].clone();
    let mut visual = app.visual(cx);
    click(&mut visual, &format!("playlist-{playlist_id}"));
    click(&mut visual, "button-document-add");
    click(&mut visual, "menu-item-New header");
    app.wait(cx, |app, _| items(app)[1]["type"] == "header");
    click(&mut visual, "button-document-add");
    click(&mut visual, "menu-item-New placeholder");
    app.wait(cx, |app, _| items(app)[2]["type"] == "placeholder");
    let header = Box::leak(
        format!(
            "cue-playlist-heading-{}",
            items(&app)[1]["id"].as_str().unwrap()
        )
        .into_boxed_str(),
    );
    let entry = items(&app)[2]["id"].as_str().unwrap().to_string();
    let search = Box::leak(format!("button-cue-link-{entry}").into_boxed_str());
    let create = Box::leak(format!("button-cue-create-{entry}").into_boxed_str());
    let unlink: &'static str = Box::leak(format!("button-cue-unlink-{entry}").into_boxed_str());
    click(&mut visual, "document-0");
    visual.simulate_keystrokes("down");
    visual.run_until_parked();
    assert!(visual.debug_bounds(header).is_some());
    assert!(visual.debug_bounds("cue-thumbnail-0").is_none());
    visual.simulate_keystrokes("down");
    visual.run_until_parked();
    assert!(visual.debug_bounds(search).is_some());
    assert!(visual.debug_bounds(create).is_some());
    assert!(visual.debug_bounds(unlink).is_none());
    assert!(visual
        .debug_bounds("document-icon-2-square-dashed")
        .is_some());
    assert!(visual.debug_bounds("cue-thumbnail-0").is_none());

    click(&mut visual, search);
    visual.simulate_input("Opening line");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("search-result-0").is_some()
    });
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, cx| {
        items(app)[2]["linkedItemId"] == original
            && app.visual(cx).debug_bounds("cue-thumbnail-0").is_some()
    });
    let arrangement = Box::leak(format!("button-cue-arrangement-{entry}").into_boxed_str());
    let notes = Box::leak(format!("button-cue-notes-{entry}").into_boxed_str());
    let tools = [
        unlink,
        "button-cue-auto-advance",
        "cue-advance-separator",
        notes,
        arrangement,
        "cue-destination-separator",
        "button-cue-destination",
        "button-cue-transition",
    ]
    .map(|selector| visual.debug_bounds(selector).unwrap());
    assert!(tools.iter().all(|bounds| bounds.size.width > gpui::px(0.)));
    assert!(tools
        .windows(2)
        .all(|buttons| buttons[0].left() < buttons[1].left()));
    assert!(
        visual.debug_bounds("button-cue-new-slide").unwrap().left()
            < visual.debug_bounds("transition-mode-slide").unwrap().left()
    );
    assert!(visual.debug_bounds(search).is_none());
    assert!(visual.debug_bounds(create).is_none());
    assert!(visual.debug_bounds("document-icon-2-library").is_some());
    click(&mut visual, arrangement);
    click(&mut visual, "button-cue-arrangement");
    click(&mut visual, "menu-item-New Arrangement");
    visual.simulate_input("Linked arrangement");
    click(&mut visual, "button-cue-form-save");
    app.wait(cx, |app, _| items(app)[2]["arrangementId"].is_string());
    click(&mut visual, "button-cue-arrangement");
    click(&mut visual, "menu-item-Master");
    app.wait(cx, |app, _| items(app)[2]["arrangementId"].is_null());
    click(&mut visual, unlink);
    app.wait(cx, |app, cx| {
        items(app)[2]["linkedItemId"].is_null() && app.visual(cx).debug_bounds(create).is_some()
    });
    assert!(visual.debug_bounds(search).is_some());
    assert!(visual.debug_bounds(unlink).is_none());
    assert!(visual
        .debug_bounds("document-icon-2-square-dashed")
        .is_some());
    click(&mut visual, create);
    visual.simulate_input("Placeholder presentation");
    click(&mut visual, "button-new-presentation-create");
    app.wait(cx, |app, _| {
        let items = items(app);
        let snapshot = app.snapshot();
        items.as_array().unwrap().len() == 3
            && snapshot["library"].as_array().unwrap().iter().any(|item| {
                item["title"] == "Placeholder presentation"
                    && item["id"] == items[2]["linkedItemId"]
            })
    });

    let path = json!(app.workspace.0.join("workspace.json"));
    let snapshot = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap();
    let workspace = &snapshot["workspace"];
    let rejected = app.host.invoke(
        "mutatePresentationLibrary",
        vec![
            path,
            json!({"type":"createPresentation","title":"Invalid placeholder",
            "libraryId":workspace["libraries"][0]["id"],"content":"","splitMode":"paragraph",
            "splitMarker":"//","splitUnits":1,"playlistId":playlist_id,
            "placeholderId":items(&app)[1]["id"]}),
            snapshot["revision"].clone(),
        ],
    );
    assert!(rejected.unwrap_err().contains("placeholder was not found"));
}

#[gpui::test]
fn selected_slides_move_and_use_the_context_menu_in_a_continuous_playlist(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, |host| {
        seed(host);
        let settings = host.invoke("getSettings", vec![]).unwrap();
        let path = settings["presenterOrbitWorkspacePath"].clone();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let item = &snapshot["workspace"]["library"][0];
        let mut slides = item["slides"].as_array().unwrap().clone();
        for (id, content) in [("third", "Third line"), ("fourth", "Fourth line")] {
            let mut slide = slides[0].clone();
            slide["id"] = json!(id);
            slide["content"] = json!(content);
            slides.push(slide);
        }
        host.invoke(
            "mutatePresentationLibrary",
            vec![
                path,
                json!({"type":"mutateItem","itemId":item["id"],"operation":{
                    "type":"setSlideDocument","slides":slides,"title":item["title"]
                }}),
                snapshot["revision"].clone(),
            ],
        )
        .unwrap();
        let payload = crate::settings::persist::general_payload(
            &settings,
            &json!({"useContinuousPlaylist":true}),
        );
        host.invoke(
            "saveSettings",
            vec![payload, settings["settingsRevision"].clone()],
        )
        .unwrap();
    });
    let original = app.snapshot();
    let item = &original["library"][0];
    let ids: Vec<_> = item["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slide| slide["id"].clone())
        .collect();
    let playlist = original["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|playlist| playlist["title"] == "Sunday")
        .unwrap();
    let mut visual = app.visual(cx);
    click(
        &mut visual,
        &format!("playlist-{}", playlist["id"].as_str().unwrap()),
    );
    click(&mut visual, "document-0");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-3").is_some()
    });
    let select_slides = |visual: &mut gpui::VisualTestContext| {
        click(visual, "cue-thumbnail-0");
        for (selector, modifiers) in [
            (
                "cue-thumbnail-2",
                gpui::Modifiers {
                    shift: true,
                    ..Default::default()
                },
            ),
            (
                "cue-thumbnail-1",
                gpui::Modifiers {
                    control: true,
                    ..Default::default()
                },
            ),
        ] {
            let at = visual.debug_bounds(selector).unwrap().center();
            visual.simulate_click(at, modifiers);
            visual.run_until_parked();
        }
    };
    select_slides(&mut visual);
    let runtime = app.runtime(cx);
    assert_eq!(
        cx.update(|cx| runtime.output.read(cx).live["liveSlideId"].clone()),
        ids[0]
    );
    assert!(visual.debug_bounds("cue-selection-0").is_some());
    assert!(visual.debug_bounds("cue-selection-1").is_none());
    assert!(visual.debug_bounds("cue-selection-2").is_some());
    let first = visual.debug_bounds("cue-thumbnail-0").unwrap();
    let second = visual.debug_bounds("cue-thumbnail-1").unwrap();
    let gap = gpui::point((first.right() + second.left()) / 2., first.center().y);
    visual.simulate_click(gap, Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("cue-selection-0").is_none());
    assert!(visual.debug_bounds("cue-selection-2").is_none());
    assert_eq!(
        cx.update(|cx| runtime.output.read(cx).live["liveSlideId"].clone()),
        ids[0]
    );
    select_slides(&mut visual);
    click(&mut visual, "region-media");
    assert!(visual.debug_bounds("cue-selection-2").is_none());
    select_slides(&mut visual);
    let release = support::hold_workspace_writes(&app, cx);
    context(&mut visual, "cue-thumbnail-2");
    click(&mut visual, "menu-item-Disable");
    for index in [1, 2] {
        click(&mut visual, &format!("cue-thumbnail-{index}"));
        app.wait(cx, |_, cx| {
            cx.update(|cx| runtime.output.read(cx).live["liveSlideId"] == ids[index])
        });
    }
    assert_ne!(app.snapshot()["library"][0]["slides"][2]["disabled"], true);
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        let data = app.snapshot();
        let slides = &data["library"][0]["slides"];
        slides[0]["disabled"] == true
            && slides[1]["disabled"] != true
            && slides[2]["disabled"] == true
    });
    for (key, index) in [("right", 3), ("left", 1)] {
        visual.simulate_keystrokes(key);
        app.wait(cx, |_, cx| {
            cx.update(|cx| runtime.output.read(cx).live["liveSlideId"] == ids[index])
        });
    }
    select_slides(&mut visual);
    app.wait(cx, |_, cx| {
        cx.update(|cx| runtime.output.read(cx).live["liveSlideId"] == ids[0])
    });

    let from = visual.debug_bounds("cue-thumbnail-2").unwrap().center();
    let last = visual.debug_bounds("cue-thumbnail-3").unwrap();
    let to = gpui::point(last.right() - gpui::px(3.), last.center().y);
    let release = support::hold_workspace_writes(&app, cx);
    visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
    visual.simulate_mouse_move(to, Some(gpui::MouseButton::Left), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("cue-reorder-marker").is_some());
    visual.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("cue-selection-2").is_some());
    assert!(visual.debug_bounds("cue-selection-3").is_some());
    assert_eq!(app.snapshot()["library"][0]["slides"][0]["id"], ids[0]);
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        app.snapshot()["library"][0]["slides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|slide| &slide["id"])
            .eq([&ids[1], &ids[3], &ids[0], &ids[2]])
    });
    visual.run_until_parked();
    context(&mut visual, "cue-thumbnail-3");
    click(&mut visual, "menu-item-Copy");
    context(&mut visual, "cue-thumbnail-1");
    click(&mut visual, "menu-item-Paste");
    app.wait(cx, |app, _| {
        app.snapshot()["library"][0]["slides"]
            .as_array()
            .unwrap()
            .len()
            == 6
    });
    let workspace = app.snapshot();
    let slides = workspace["library"][0]["slides"].as_array().unwrap();
    assert_eq!(slides[2]["content"], item["slides"][0]["content"]);
    assert_eq!(slides[3]["content"], item["slides"][2]["content"]);
    assert_ne!(slides[2]["id"], ids[0]);
    assert_ne!(slides[3]["id"], ids[2]);

    let path = json!(app.workspace.0.join("workspace.json"));
    let loaded = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap();
    let rejected = app.host.invoke(
        "mutatePresentationLibrary",
        vec![
            path,
            json!({"type":"mutateItem","itemId":item["id"],"operation":{
                "type":"reorderSlides","targetIndex":0,"slideIds":[ids[0], "missing-slide"]
            }}),
            loaded["revision"].clone(),
        ],
    );
    assert!(rejected.is_err());
    assert_eq!(
        app.snapshot()["library"][0]["slides"],
        workspace["library"][0]["slides"]
    );

    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-4").is_some()
    });
    click(&mut visual, "cue-thumbnail-2");
    let last = visual.debug_bounds("cue-thumbnail-3").unwrap().center();
    visual.simulate_click(
        last,
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    visual.run_until_parked();
    context(&mut visual, "cue-thumbnail-3");
    let release = support::hold_workspace_writes(&app, cx);
    click(&mut visual, "menu-item-Delete");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-4").is_none()
            && app.visual(cx).debug_bounds("cue-thumbnail-3").is_some()
    });
    assert_eq!(
        app.snapshot()["library"][0]["slides"],
        workspace["library"][0]["slides"]
    );
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        app.snapshot()["library"][0]["slides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|slide| &slide["id"])
            .eq([&ids[1], &ids[3], &ids[0], &ids[2]])
    });
}

#[gpui::test]
fn continuous_playlist_focus_and_forward_header_steps(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, |host| {
        seed(host);
        let settings = host.invoke("getSettings", vec![]).unwrap();
        let path = settings["presenterOrbitWorkspacePath"].clone();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let playlist = snapshot["workspace"]["playlist"]
            .as_array()
            .unwrap()
            .iter()
            .find(|playlist| playlist["title"] == "Sunday")
            .unwrap();
        let header = host.invoke("mutatePresentationPlaylist", vec![
            path.clone(), json!({"type":"createEntry", "playlistId":playlist["id"], "entryType":"header", "title":"Second section"}),
            snapshot["revision"].clone(),
        ]).unwrap();
        let text = vec!["Second item"; 25].join("\n\n");
        host.invoke("mutatePresentationLibrary", vec![path, json!({
            "type":"createPresentation", "libraryId":snapshot["workspace"]["libraries"][0]["id"],
            "title":"Other presentation", "content":text, "splitMode":"paragraph", "splitMarker":"",
            "splitUnits":1, "playlistId":playlist["id"]
        }), header["revision"].clone()]).unwrap();
        let payload = crate::settings::persist::general_payload(
            &settings,
            &json!({"useContinuousPlaylist":true}),
        );
        host.invoke(
            "saveSettings",
            vec![payload, settings["settingsRevision"].clone()],
        )
        .unwrap();
    });
    let snapshot = app.snapshot();
    let playlist = snapshot["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|playlist| playlist["title"] == "Sunday")
        .unwrap();
    let ids: Vec<_> = playlist["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    let heading = |index: usize| {
        &*Box::leak(format!("cue-playlist-heading-{}", ids[index]).into_boxed_str()) as &'static str
    };
    let first = heading(0);
    let marker = heading(1);
    let last = heading(2);
    let row = Box::leak(format!("cue-playlist-slide-row-{}-0", ids[0]).into_boxed_str());
    let runtime = app.runtime(cx);
    let selected = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            runtime
                .ui
                .read(cx)
                .selected_documents
                .first()
                .and_then(|entry| entry.index)
        })
    };
    let mut visual = app.visual(cx);
    click(
        &mut visual,
        &format!("playlist-{}", playlist["id"].as_str().unwrap()),
    );
    click(&mut visual, "document-0");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds(last).is_some()
            && cx.update(|cx| runtime.output.read(cx).committed_revision > 0)
    });
    let top = visual.debug_bounds(first).unwrap().top();
    click(&mut visual, marker);
    assert_eq!(selected(cx), Some(1));
    assert_eq!(visual.debug_bounds(first).unwrap().top(), top);
    click(&mut visual, last);
    assert_eq!(selected(cx), Some(2));
    assert_eq!(visual.debug_bounds(first).unwrap().top(), top);
    let area = visual.debug_bounds(row).unwrap();
    visual.simulate_click(
        gpui::point(area.center().x, area.top() + gpui::px(2.)),
        Default::default(),
    );
    visual.run_until_parked();
    assert_eq!(selected(cx), Some(0));
    assert_eq!(visual.debug_bounds(first).unwrap().top(), top);
    let live = |cx: &mut TestAppContext| cx.update(|cx| runtime.output.read(cx).live.clone());
    let advance = |visual: &mut gpui::VisualTestContext| visual.simulate_keystrokes("right");
    let first_item = snapshot["library"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == playlist["items"][0]["itemId"])
        .unwrap();
    advance(&mut visual);
    app.wait(cx, |_, cx| {
        live(cx)["liveSlideId"] == first_item["slides"][0]["id"]
    });
    advance(&mut visual);
    app.wait(cx, |_, cx| {
        live(cx)["liveSlideId"] == first_item["slides"][1]["id"]
    });
    advance(&mut visual);
    app.wait(cx, |_, cx| selected(cx) == Some(1));
    visual.run_until_parked();
    let viewport = |visual: &mut gpui::VisualTestContext| {
        visual.debug_bounds("cue-list-viewport").unwrap().top()
    };
    assert_eq!(
        visual.debug_bounds(marker).unwrap().top(),
        viewport(&mut visual)
    );
    assert_eq!(live(cx)["liveSlideId"], first_item["slides"][1]["id"]);
    advance(&mut visual);
    app.wait(cx, |app, cx| {
        let mut visual = app.visual(cx);
        selected(cx) == Some(2)
            && visual
                .debug_bounds(last)
                .is_some_and(|bounds| bounds.top() == viewport(&mut visual))
    });
    assert_eq!(live(cx)["liveSlideId"], first_item["slides"][1]["id"]);
    advance(&mut visual);
    app.wait(cx, |_, cx| live(cx)["livePlaylistEntryId"] == ids[2]);
    click(&mut visual, "document-0");
    app.wait(cx, |app, cx| {
        let mut visual = app.visual(cx);
        visual
            .debug_bounds(first)
            .is_some_and(|bounds| bounds.top() == viewport(&mut visual))
    });
}

fn seed_playlist(host: &orbit_host::Host, content: &str) {
    seed(host);
    let path = host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
    let snapshot = host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap();
    let playlist_id = snapshot["workspace"]["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["title"] == "Sunday")
        .unwrap()["id"]
        .clone();
    host.invoke("mutatePresentationLibrary", vec![path.clone(), json!({
            "type":"createPresentation", "libraryId":snapshot["workspace"]["libraries"][0]["id"],
            "title":"Other presentation", "content":content, "splitMode":"paragraph", "splitMarker":"", "splitUnits":1, "playlistId":playlist_id
        }), snapshot["revision"].clone()]).unwrap();
}

#[gpui::test]
fn continuous_playlist_follows_settings_toolbar_menu_and_restart(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, |host| {
        seed_playlist(host, &vec!["Other text"; 25].join("\n\n"));
        let settings = host.invoke("getSettings", vec![]).unwrap();
        let payload = crate::settings::persist::general_payload(
            &settings,
            &json!({"useContinuousPlaylist":true}),
        );
        host.invoke(
            "saveSettings",
            vec![payload, settings["settingsRevision"].clone()],
        )
        .unwrap();
    });
    let snapshot = app.snapshot();
    let playlist = snapshot["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["title"] == "Sunday")
        .unwrap();
    let playlist_row =
        Box::leak(format!("playlist-{}", playlist["id"].as_str().unwrap()).into_boxed_str());
    let headings: Vec<_> = playlist["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            Box::leak(
                format!("cue-playlist-heading-{}", entry["id"].as_str().unwrap()).into_boxed_str(),
            ) as &'static str
        })
        .collect();
    let mut visual = app.visual(cx);
    click(&mut visual, playlist_row);
    app.wait(cx, |app, cx| {
        headings
            .iter()
            .all(|heading| app.visual(cx).debug_bounds(heading).is_some())
    });
    let runtime = app.runtime(cx);
    let window = cx.add_window(move |window, cx| {
        let view = cx.new(|cx| crate::settings::SettingsView::new(runtime, cx));
        orbit_ui::controls::Root::new(view, window, cx)
    });
    let mut settings = gpui::VisualTestContext::from_window(window.into(), cx);
    settings.run_until_parked();
    click(&mut settings, "general-continuous-playlist");
    app.wait(cx, |app, cx| {
        app.host.invoke("getSettings", vec![]).unwrap()["generalSettings"]["useContinuousPlaylist"]
            == false
            && app.visual(cx).debug_bounds(headings[1]).is_none()
    });
    click(&mut visual, "button-cue-continuous");
    app.wait(cx, |app, cx| {
        app.host.invoke("getSettings", vec![]).unwrap()["generalSettings"]["useContinuousPlaylist"]
            == true
            && headings
                .iter()
                .all(|heading| app.visual(cx).debug_bounds(heading).is_some())
    });
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    visual.update(|window, cx| assert!(commands.run("view-continuous", false, window, cx)));
    visual.run_until_parked();
    app.wait(cx, |app, cx| {
        app.host.invoke("getSettings", vec![]).unwrap()["generalSettings"]["useContinuousPlaylist"]
            == false
            && app.visual(cx).debug_bounds(headings[1]).is_none()
    });
    click(&mut settings, "general-continuous-playlist");
    app.wait(cx, |app, cx| {
        app.host.invoke("getSettings", vec![]).unwrap()["generalSettings"]["useContinuousPlaylist"]
            == true
            && headings
                .iter()
                .all(|heading| app.visual(cx).debug_bounds(heading).is_some())
    });
    app.restart(cx);
    let mut visual = app.visual(cx);
    click(&mut visual, playlist_row);
    app.wait(cx, |app, cx| {
        headings
            .iter()
            .all(|heading| app.visual(cx).debug_bounds(heading).is_some())
    });
    let second_entry = playlist["items"][1]["id"].as_str().unwrap();
    let prior_top = visual.debug_bounds(headings[1]).unwrap().top();
    click(&mut visual, "document-1");
    app.wait(cx, |app, cx| {
        app.visual(cx)
            .debug_bounds(headings[1])
            .is_some_and(|bounds| bounds.top() < prior_top)
    });
    for selector in [
        "button-cue-destination",
        "button-cue-auto-advance",
        "transition-controls-item",
    ] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    for action in ["cue-arrangement", "cue-notes"] {
        let selector = Box::leak(format!("button-{action}-{second_entry}").into_boxed_str());
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    click(
        &mut visual,
        &format!("button-cue-arrangement-{second_entry}"),
    );
    let second_item = &playlist["items"][1]["itemId"];
    let runtime = app.runtime(cx);
    assert!(cx.update(|cx| runtime
        .ui
        .read(cx)
        .arrangement_editor
        .as_ref()
        .is_some_and(|editor| json!(editor.item_id) == *second_item)));
    click(&mut visual, "button-cue-arrangement");
    let add = visual.debug_bounds("menu-item-New Arrangement").unwrap();
    let edit = visual.debug_bounds("menu-item-Edit Arrangements").unwrap();
    assert!(add.top() < edit.top());
    click(&mut visual, "menu-item-New Arrangement");
    visual.simulate_input("Evening arrangement");
    let release = support::hold_workspace_writes(&app, cx);
    click(&mut visual, "button-cue-form-save");
    let remove = "button-cue-arrangement-remove-0";
    assert!(visual.debug_bounds(remove).is_some());
    click(&mut visual, remove);
    click(&mut visual, remove);
    let item_id = second_item.as_str().unwrap();
    let (arrangement_id, sections) = cx.update(|cx| {
        let content = runtime.content.read(cx);
        let item = content.items.get(item_id).unwrap();
        let arrangement = item.arrangements.last().unwrap();
        assert_eq!(arrangement.name, "Evening arrangement");
        let expected: Vec<_> = crate::collections::arrangements::presentation_sections(item)
            .into_iter()
            .skip(2)
            .map(|section| section.id)
            .collect();
        assert_eq!(arrangement.section_ids, expected);
        (arrangement.id.clone(), expected)
    });
    assert!(!app.snapshot()["library"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["id"] == item_id)
        .flat_map(|item| item["arrangements"].as_array().into_iter().flatten())
        .any(|arrangement| arrangement["id"] == arrangement_id));
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        let snapshot = app.snapshot();
        snapshot["library"].as_array().unwrap().iter().any(|item| {
            item["id"] == item_id
                && item["arrangements"].as_array().is_some_and(|arrangements| {
                    arrangements.iter().any(|arrangement| {
                        arrangement["id"] == arrangement_id
                            && arrangement["sectionIds"] == json!(sections)
                    })
                })
        }) && snapshot["playlist"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|playlist| playlist["items"].as_array().into_iter().flatten())
            .any(|entry| entry["id"] == second_entry && entry["arrangementId"] == arrangement_id)
    });
    let path = json!(app.workspace.0.join("workspace.json"));
    let revision = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap()["revision"]
        .clone();
    let error = app.host.invoke("mutatePresentationLibrary", vec![
        path.clone(), json!({"type":"createArrangement", "itemId":item_id, "name":"Invalid id", "newArrangementId":"not-a-uuid"}),
        revision.clone(),
    ]).unwrap_err();
    assert!(error.contains("Arrangement id must be a UUID"), "{error}");
    let error = app.host.invoke("mutatePresentationLibrary", vec![
        path.clone(), json!({"type":"removeArrangementSection", "itemId":item_id, "arrangementId":arrangement_id,
            "index":0, "expectedSectionIds":[]}),
        revision,
    ]).unwrap_err();
    assert!(error.contains("Arrangement sections changed"), "{error}");
    click(&mut visual, "button-cue-arrangement");
    for action in ["Rename", "Duplicate", "Move up", "Move down", "Delete"] {
        let selector = Box::leak(format!("menu-item-{action}").into_boxed_str());
        assert!(visual.debug_bounds(selector).is_none(), "{action}");
    }
    click(&mut visual, "menu-item-Edit Arrangements");
    assert!(visual.debug_bounds("arrangement-manager-list").is_some());
    click(&mut visual, "button-arrangement-manager-add");
    visual.simulate_input("Second arrangement");
    click(&mut visual, "button-cue-form-save");
    let id_for_name = |cx: &mut TestAppContext, name: &str| {
        cx.update(|cx| {
            runtime
                .content
                .read(cx)
                .items
                .get(item_id)
                .unwrap()
                .arrangements
                .iter()
                .find(|arrangement| arrangement.name == name)
                .unwrap()
                .id
                .clone()
        })
    };
    let second = id_for_name(cx, "Second arrangement");
    click(&mut visual, "button-arrangement-manager-duplicate");
    let copy = id_for_name(cx, "Second arrangement Copy");
    click(
        &mut visual,
        &format!("arrangement-manager-row-{arrangement_id}"),
    );
    click(&mut visual, "button-arrangement-manager-rename");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("Renamed arrangement");
    click(&mut visual, "button-cue-form-save");
    assert_eq!(id_for_name(cx, "Renamed arrangement"), arrangement_id);
    support::drag_row(
        &mut visual,
        &format!("arrangement-manager-row-{second}"),
        &format!("arrangement-manager-row-{arrangement_id}"),
        false,
        "arrangement-manager-drop",
    );
    let copy_row = Box::leak(format!("arrangement-manager-row-{copy}").into_boxed_str());
    click(&mut visual, copy_row);
    click(&mut visual, "button-arrangement-manager-remove");
    assert!(visual.debug_bounds(copy_row).is_none());
    click(&mut visual, "dialog-close");
    app.wait(cx, |app, _| {
        let snapshot = app.snapshot();
        let arrangements = snapshot["library"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == item_id)
            .unwrap()["arrangements"]
            .as_array()
            .unwrap();
        let first = arrangements
            .iter()
            .position(|a| a["id"] == arrangement_id && a["name"] == "Renamed arrangement");
        let second = arrangements.iter().position(|a| a["id"] == second);
        second
            .zip(first)
            .is_some_and(|(second, first)| second < first)
            && arrangements.iter().all(|a| a["id"] != copy)
    });
    click(
        &mut visual,
        &format!("button-cue-arrangement-{second_entry}"),
    );
    click(&mut visual, &format!("button-cue-notes-{second_entry}"));
    assert!(visual.debug_bounds("cue-operator-notes").is_some());
    click(&mut visual, &format!("button-cue-notes-{second_entry}"));
    click(&mut visual, "button-document-add");
    click(&mut visual, "menu-item-New placeholder");
    let entries = |app: &Scenario| {
        app.snapshot()["playlist"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == playlist["id"])
            .unwrap()["items"]
            .clone()
    };
    app.wait(cx, |app, _| entries(app)[2]["type"] == "placeholder");
    let placeholder = entries(&app)[2]["id"].as_str().unwrap().to_string();
    click(&mut visual, "document-2");
    let link = Box::leak(format!("button-cue-link-{placeholder}").into_boxed_str());
    app.wait(cx, |app, cx| app.visual(cx).debug_bounds(link).is_some());
    click(&mut visual, link);
    visual.simulate_input("Other presentation");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("search-result-0").is_some()
    });
    click(&mut visual, "button-search-add");
    let unlink = Box::leak(format!("button-cue-unlink-{placeholder}").into_boxed_str());
    app.wait(cx, |app, cx| {
        entries(app)[2]["linkedItemId"] == *second_item
            && app.visual(cx).debug_bounds(unlink).is_some()
    });
    click(
        &mut visual,
        &format!("button-cue-arrangement-{placeholder}"),
    );
    click(&mut visual, "button-cue-arrangement");
    click(&mut visual, "menu-item-Second arrangement");
    app.wait(cx, |app, _| {
        let entries = entries(app);
        entries[2]["arrangementId"] == second && entries[1]["arrangementId"] == arrangement_id
    });
    let revision = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap()["revision"]
        .clone();
    let rejected = app.host.invoke(
        "mutatePresentationPlaylist",
        vec![
            path,
            json!({"type":"setEntryArrangement","playlistId":playlist["id"],"entryId":placeholder,"arrangementId":"missing"}),
            revision,
        ],
    );
    assert!(rejected.unwrap_err().contains("arrangement was not found"));
    click(&mut visual, unlink);
    app.wait(cx, |app, _| {
        let entry = &entries(app)[2];
        entry["linkedItemId"].is_null() && entry["arrangementId"].is_null()
    });
    let library_row = Box::leak(
        format!(
            "library-{}",
            snapshot["libraries"][0]["id"].as_str().unwrap()
        )
        .into_boxed_str(),
    );
    click(&mut visual, library_row);
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-0").is_some()
            && app.visual(cx).debug_bounds(headings[0]).is_none()
    });
    assert_eq!(
        app.host.invoke("getSettings", vec![]).unwrap()["generalSettings"]["useContinuousPlaylist"],
        true
    );
}

#[gpui::test]
fn playlist_item_menu_edits_hides_restores_and_preserves_library_documents(
    cx: &mut TestAppContext,
) {
    let mut app =
        Scenario::new(cx, false, |host| {
            seed_playlist(host, "Other text");
            let path =
                host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
            let snapshot = host
                .invoke("readWorkspaceSnapshot", vec![path.clone()])
                .unwrap();
            let item = &snapshot["workspace"]["library"][0];
            host.invoke("mutatePresentationLibrary", vec![path, json!({
            "type":"mutateItem","itemId":item["id"],"operation":{
                "type":"setSlideGroup","slideId":item["slides"][0]["id"],"groupId":"verse"
            }
        }), snapshot["revision"].clone()]).unwrap();
        });
    let snapshot = app.snapshot();
    let playlist = snapshot["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["title"] == "Sunday")
        .unwrap();
    let playlist_id = playlist["id"].as_str().unwrap().to_string();
    assert_eq!(playlist["items"].as_array().unwrap().len(), 2);
    let item_id = playlist["items"][0]["itemId"].as_str().unwrap().to_string();
    let playlist_row = Box::leak(format!("playlist-{playlist_id}").into_boxed_str());
    let runtime = app.runtime(cx);
    let mut visual = app.visual(cx);
    click(&mut visual, playlist_row);
    context(&mut visual, "document-0");
    for label in [
        "Hide item",
        "Add to",
        "Arrangement",
        "Destination",
        "Edit",
        "Resize",
        "Reflow",
        "Copy",
        "Paste",
        "Delete",
        "Duplicate",
        "Show in library",
    ] {
        assert!(
            visual
                .debug_bounds(Box::leak(format!("menu-item-{label}").into_boxed_str()))
                .is_some(),
            "{label}"
        );
    }
    assert!(visual.debug_bounds("menu-item-Move to").is_none());
    assert!(visual.debug_bounds("menu-item-Rename").is_none());
    click(&mut visual, "menu-item-Destination");
    click(&mut visual, "menu-item-Announcements");
    app.wait(cx, |app, _| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == item_id && item["destination"] == "announcements")
    });
    context(&mut visual, "document-0");
    click(&mut visual, "menu-item-Resize");
    click(&mut visual, "menu-item-1280 × 720");
    app.wait(cx, |app, _| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["id"] == item_id
                    && item["slides"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|slide| slide["width"] == 1280 && slide["height"] == 720)
            })
    });
    context(&mut visual, "document-0");
    click(&mut visual, "menu-item-Edit");
    app.wait(cx, |app, cx| {
        cx.update(|cx| app.main.view.read(cx).canvas_open)
    });
    assert_eq!(
        cx.update(|cx| runtime.ui.read(cx).selected_library_item_id.clone()),
        item_id
    );
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let assert_cue_fits = |visual: &mut gpui::VisualTestContext| {
        let tile = visual.debug_bounds("cue-thumbnail-0").unwrap();
        let text = visual.debug_bounds("cue-copy-0").unwrap();
        let footer = visual.debug_bounds("cue-footer-0").unwrap();
        assert!(text.size.height > gpui::px(20.));
        assert!(text.bottom() <= footer.top() + gpui::px(1.));
        assert!(footer.bottom() <= tile.bottom() + gpui::px(1.));
        tile.size.width
    };
    let show = |visual: &mut gpui::VisualTestContext| {
        visual.update(|window, cx| {
            assert!(commands.run("show-workspace", false, window, cx));
            window.simulate_next_frame(cx);
        });
    };
    show(&mut visual);
    let show_width = assert_cue_fits(&mut visual);
    visual.update(|window, cx| assert!(commands.run("view-continuous", false, window, cx)));
    let heading = Box::leak(
        format!(
            "cue-playlist-heading-{}",
            playlist["items"][0]["id"].as_str().unwrap()
        )
        .into_boxed_str(),
    );
    app.wait(cx, |app, cx| app.visual(cx).debug_bounds(heading).is_some());
    context(&mut visual, "document-0");
    click(&mut visual, "menu-item-Reflow");
    app.wait(cx, |app, cx| {
        cx.update(|cx| app.main.view.read(cx).cue_thumbnails.read(cx).reflow_open())
    });
    visual.run_until_parked();
    let reflow_width = assert_cue_fits(&mut visual);
    assert!(visual.debug_bounds(heading).is_none());
    assert!(visual.debug_bounds("cue-thumbnail-1").is_some());
    assert!(visual.debug_bounds("cue-thumbnail-2").is_none());
    for button in [
        "button-reflow-undo",
        "button-reflow-redo",
        "button-reflow-edit",
        "button-cue-form-save",
    ] {
        assert!(visual.debug_bounds(button).is_none());
    }
    click(&mut visual, "cue-thumbnail-1");
    assert!(visual.debug_bounds("reflow-focus-1").is_some());
    assert!(visual.debug_bounds("reflow-focus-0").is_none());
    let row = visual.debug_bounds("reflow-text-row-0").unwrap();
    let font = visual.debug_bounds("reflow-font-slider").unwrap();
    visual.simulate_click(
        gpui::point(font.right() - gpui::px(1.), font.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("reflow-text-row-0")
            .unwrap()
            .size
            .height
            > row.size.height
    );
    let zoom = visual.debug_bounds("reflow-zoom-slider").unwrap();
    visual.simulate_click(
        gpui::point(zoom.left() + gpui::px(1.), zoom.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    let small = visual.debug_bounds("cue-thumbnail-0").unwrap().size.width;
    visual.simulate_click(
        gpui::point(zoom.right() - gpui::px(1.), zoom.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("cue-thumbnail-0").unwrap().size.width > small * 3.);
    click(&mut visual, "document-1");
    assert!(visual.debug_bounds("cue-thumbnail-1").is_none());
    click(&mut visual, "document-0");
    click(&mut visual, "cue-thumbnail-0");
    visual.simulate_keystrokes("home right right");
    click(&mut visual, "button-reflow-split");
    app.wait(cx, |app, cx| {
        let snapshot = app.snapshot();
        let slides = &snapshot["library"][0]["slides"];
        slides[0]["content"] == "Op"
            && slides[1]["content"] == "ening line"
            && app.visual(cx).debug_bounds("reflow-text-row-2").is_some()
    });
    show(&mut visual);
    app.wait(cx, |app, cx| {
        !cx.update(|cx| app.main.view.read(cx).cue_thumbnails.read(cx).reflow_open())
    });
    assert_eq!(assert_cue_fits(&mut visual), show_width);
    assert!(show_width > reflow_width);
    context(&mut visual, "document-0");
    click(&mut visual, "menu-item-Show in library");
    assert!(cx.update(|cx| runtime.ui.read(cx).selected_playlist_id.is_empty()));
    assert_eq!(
        cx.update(|cx| runtime.ui.read(cx).selected_library_item_id.clone()),
        item_id
    );
    click(&mut visual, playlist_row);
    let release = support::hold_workspace_writes(&app, cx);
    context(&mut visual, "document-0");
    click(&mut visual, "menu-item-Hide item");
    assert!(visual.debug_bounds("document-0").is_none());
    assert!(visual.debug_bounds("document-1").is_some());
    assert!(visual
        .debug_bounds("button-playlist-hidden-items")
        .is_some());
    release.send(()).unwrap();
    let entries = |app: &Scenario| -> Value {
        app.snapshot()["playlist"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == playlist_id)
            .unwrap()["items"]
            .clone()
    };
    app.wait(cx, |app, _| entries(app)[0]["hidden"] == true);
    click(&mut visual, "document-1");
    visual.simulate_keystrokes("up");
    assert_eq!(
        cx.update(|cx| runtime.ui.read(cx).selected_documents[0].index),
        Some(1)
    );
    app.restart(cx);
    let mut visual = app.visual(cx);
    click(&mut visual, playlist_row);
    assert!(visual.debug_bounds("document-0").is_none());
    click(&mut visual, "button-playlist-hidden-items");
    click(&mut visual, "menu-item-Unhide item");
    click(&mut visual, "menu-item-Sunday service");
    app.wait(cx, |app, cx| {
        entries(app)[0]["hidden"].is_null() && app.visual(cx).debug_bounds("document-0").is_some()
    });
    for row in ["document-0", "document-1"] {
        context(&mut visual, row);
        click(&mut visual, "menu-item-Hide item");
    }
    app.wait(cx, |app, _| {
        entries(app)
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["hidden"] == true)
    });
    click(&mut visual, "button-playlist-hidden-items");
    let release = support::hold_workspace_writes(&app, cx);
    click(&mut visual, "menu-item-Unhide all items");
    assert!(visual.debug_bounds("document-0").is_some());
    assert!(visual.debug_bounds("document-1").is_some());
    assert!(entries(&app)
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["hidden"] == true));
    release.send(()).unwrap();
    app.wait(cx, |app, cx| {
        entries(app)
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["hidden"].is_null())
            && app.visual(cx).debug_bounds("document-0").is_some()
            && app.visual(cx).debug_bounds("document-1").is_some()
    });
    context(&mut visual, "document-0");
    click(&mut visual, "menu-item-Copy");
    context(&mut visual, "document-1");
    click(&mut visual, "menu-item-Paste");
    app.wait(cx, |app, _| entries(app).as_array().unwrap().len() == 3);
    context(&mut visual, "document-0");
    click(&mut visual, "menu-item-Duplicate");
    app.wait(cx, |app, _| entries(app).as_array().unwrap().len() == 4);
    context(&mut visual, "document-0");
    click(&mut visual, "menu-item-Delete");
    app.wait(cx, |app, _| entries(app).as_array().unwrap().len() == 3);
    assert_eq!(app.snapshot()["library"].as_array().unwrap().len(), 2);
    let path = json!(app.workspace.0.join("workspace.json"));
    let revision = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap()["revision"]
        .clone();
    for hidden in [json!("yes"), Value::Null] {
        let entry_id = entries(&app)[0]["id"].clone();
        let error = app.host.invoke("mutatePresentationPlaylist", vec![path.clone(), json!({"type":"setEntriesHidden", "playlistId":playlist_id, "entryIds":[entry_id], "hidden":hidden}), revision.clone()]).unwrap_err();
        assert!(error.contains("visibility must be a boolean"), "{error}");
    }
    let before = entries(&app);
    let error = app.host.invoke("mutatePresentationPlaylist", vec![path,
        json!({"type":"setEntriesHidden", "playlistId":playlist_id, "entryIds":[before[0]["id"], "missing-entry"], "hidden":true}), revision]).unwrap_err();
    assert!(error.contains("entry was not found"), "{error}");
    assert_eq!(entries(&app), before);
}

#[gpui::test]
fn playlist_header_settings_trigger_and_round_trip(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, |host| {
        seed(host);
        let settings = host.invoke("getSettings", vec![]).unwrap();
        let path = settings["presenterOrbitWorkspacePath"].clone();
        for (name, patch) in [
            (
                "Elapsed timer",
                json!({"type":"elapsed","elapsedStartSeconds":12,"durationSeconds":12}),
            ),
            (
                "Clock timer",
                json!({"type":"countdown-to-time","targetTimeSeconds":3600,"durationSeconds":3600}),
            ),
        ] {
            let snapshot = host
                .invoke("readWorkspaceSnapshot", vec![path.clone()])
                .unwrap();
            host.invoke(
                "mutateCountdownTimers",
                vec![
                    path.clone(),
                    json!({"type":"create","baseName":name,"patch":patch}),
                    snapshot["revision"].clone(),
                ],
            )
            .unwrap();
        }
        let payload = crate::settings::persist::general_payload(
            &settings,
            &json!({"useContinuousPlaylist": true}),
        );
        host.invoke(
            "saveSettings",
            vec![payload, settings["settingsRevision"].clone()],
        )
        .unwrap();
    });
    let snapshot = app.snapshot();
    let playlist = snapshot["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["title"] == "Sunday")
        .unwrap();
    let entries = |app: &Scenario| {
        app.snapshot()["playlist"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == playlist["id"])
            .unwrap()["items"]
            .clone()
    };
    let timer = snapshot["countdownTimers"][0]["id"].as_str().unwrap();
    let timer_index = |name: &str| {
        snapshot["countdownTimers"]
            .as_array()
            .unwrap()
            .iter()
            .position(|timer| timer["name"] == name)
            .unwrap()
    };
    let elapsed_index = timer_index("Elapsed timer");
    let clock_index = timer_index("Clock timer");
    let mut visual = app.visual(cx);
    click(
        &mut visual,
        &format!("playlist-{}", playlist["id"].as_str().unwrap()),
    );
    click(&mut visual, "button-document-add");
    click(&mut visual, "menu-item-New header");
    app.wait(cx, |app, _| entries(app)[1]["type"] == "header");
    let id = entries(&app)[1]["id"].as_str().unwrap().to_string();
    let settings = Box::leak(format!("button-cue-header-settings-{id}").into_boxed_str());
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds(settings).is_some()
    });
    click(&mut visual, settings);
    let gear = visual.debug_bounds(settings).unwrap();
    let popover = visual
        .debug_bounds("playlist-header-settings-popover")
        .unwrap();
    assert!(popover.origin.y >= gear.bottom());
    assert_eq!(popover.right(), gear.right());
    click(&mut visual, "document-0");
    assert!(visual
        .debug_bounds("playlist-header-settings-popover")
        .is_none());
    click(&mut visual, settings);
    click(&mut visual, "playlist-header-name");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("Service start");
    click(&mut visual, "playlist-header-timer");
    click(&mut visual, "select-option-1");
    click(&mut visual, "playlist-header-timer-command");
    assert!(visual.debug_bounds("select-option-2").is_some());
    assert!(visual.debug_bounds("select-option-3").is_none());
    click(&mut visual, "select-option-0");
    click(&mut visual, "playlist-header-set-configuration");
    click(&mut visual, "playlist-header-config-durationSeconds");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("00:02:30");
    click(&mut visual, "playlist-header-color");
    let settings_color = orbit_ui::controls::STANDARD_COLORS[5];
    click(&mut visual, &format!("color-swatch-{settings_color}"));
    click(&mut visual, "playlist-header-color");
    assert!(visual.debug_bounds("button-playlist-header-save").is_none());
    assert!(visual
        .debug_bounds("button-playlist-header-cancel")
        .is_none());
    app.wait(cx, |app, _| {
        entries(app)[1]["title"] == "Service start"
            && entries(app)[1]["color"] == settings_color
            && entries(app)[1]["actions"][0]["targetId"] == timer
            && entries(app)[1]["actions"][0]["details"]["configuration"]["durationSeconds"].as_f64()
                == Some(150.)
    });
    assert_eq!(
        app.snapshot()["countdownTimers"][0],
        snapshot["countdownTimers"][0]
    );
    let path = json!(app.workspace.0.join("workspace.json"));
    let revision = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap()["revision"]
        .clone();
    let malformed = app.host.invoke("mutatePresentationPlaylist", vec![
        path,
        json!({"type": "setHeader", "playlistId": playlist["id"], "entryId": id,
            "title": "Service start", "color": settings_color, "timerId": timer, "timerCommand": "start",
            "timerConfiguration": {"type": "countdown", "durationSeconds": -1, "allowsOverrun": false}}),
        revision,
    ]).unwrap_err();
    assert!(
        malformed.contains("duration must be non-negative"),
        "{malformed}"
    );
    click(&mut visual, &format!("button-cue-header-trigger-{id}"));
    let runtime = app.runtime(cx);
    app.wait(cx, |app, cx| {
        app.snapshot()["countdownTimers"][0]["durationSeconds"].as_f64() == Some(150.)
            && cx.update(|cx| runtime.output.read(cx).live["countdownTimerRuns"][timer].is_object())
    });

    click(&mut visual, settings);
    click(&mut visual, "playlist-header-timer");
    click(&mut visual, &format!("select-option-{}", elapsed_index + 1));
    let configuration =
        |app: &Scenario| entries(app)[1]["actions"][0]["details"]["configuration"].clone();
    app.wait(cx, |app, cx| {
        let value = configuration(app);
        value["type"] == "elapsed"
            && value["elapsedStartSeconds"].as_f64() == Some(12.)
            && value["elapsedEndSeconds"].is_null()
            && app
                .visual(cx)
                .debug_bounds("playlist-header-config-elapsedEndSeconds")
                .is_some()
    });
    click(
        &mut visual,
        "form-input-playlist-header-config-elapsedEndSeconds",
    );
    visual.simulate_input("00:01:00");
    app.wait(cx, |app, _| {
        configuration(app)["elapsedEndSeconds"].as_f64() == Some(60.)
    });
    support::primary_shortcut(&mut visual, "a backspace");
    app.wait(cx, |app, _| {
        configuration(app)["elapsedEndSeconds"].is_null()
    });
    click(&mut visual, "playlist-header-timer");
    click(&mut visual, &format!("select-option-{}", clock_index + 1));
    assert!(visual
        .debug_bounds("playlist-header-config-elapsedEndSeconds")
        .is_none());
    click(&mut visual, "playlist-header-config-period");
    assert!(visual.debug_bounds("select-option-2").is_some());
    click(&mut visual, "select-option-1");
    click(
        &mut visual,
        "form-input-playlist-header-config-targetTimeSeconds",
    );
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("12:05:00");
    app.wait(cx, |app, _| {
        let value = configuration(app);
        value["targetTimeSeconds"].as_f64() == Some(43_500.)
            && value["format"]["targetTimePeriod"] == "pm"
    });
    click(&mut visual, "document-0");
    click(&mut visual, settings);
    assert!(visual
        .debug_bounds("playlist-header-config-period")
        .is_some());
    click(&mut visual, "playlist-header-config-period");
    click(&mut visual, "select-option-0");
    app.wait(cx, |app, _| {
        configuration(app)["targetTimeSeconds"].as_f64() == Some(300.)
    });
    click(&mut visual, "playlist-header-config-period");
    click(&mut visual, "select-option-1");
    app.wait(cx, |app, _| {
        configuration(app)["targetTimeSeconds"].as_f64() == Some(43_500.)
    });
    click(&mut visual, "playlist-header-config-period");
    click(&mut visual, "select-option-2");
    app.wait(cx, |app, _| {
        let value = configuration(app);
        value["targetTimeSeconds"].as_f64() == Some(43_500.)
            && value["format"]["targetTimePeriod"] == "24h"
    });
    click(&mut visual, "playlist-header-config-period");
    click(&mut visual, "select-option-1");
    app.wait(cx, |app, _| {
        configuration(app)["format"]["targetTimePeriod"] == "pm"
    });
    click(&mut visual, "document-0");
    let clock_timer = &snapshot["countdownTimers"][clock_index];
    assert_eq!(app.snapshot()["countdownTimers"][clock_index], *clock_timer);
    context(&mut visual, "document-1");
    assert!(visual.debug_bounds("menu-control-Change color").is_some());
    click(&mut visual, "playlist-menu-header-color");
    let color = orbit_ui::controls::STANDARD_COLORS[4];
    click(&mut visual, &format!("color-swatch-{color}"));
    assert!(visual.debug_bounds("menu-control-Change color").is_some());
    click(&mut visual, "document-0");
    app.wait(cx, |app, cx| {
        entries(app)[1]["color"] == color
            && app
                .visual(cx)
                .debug_bounds("menu-control-Change color")
                .is_none()
    });
    assert_eq!(
        entries(&app)[1]["actions"][0]["targetId"],
        clock_timer["id"]
    );
    context(&mut visual, "document-1");
    click(&mut visual, "menu-item-Hide item");
    app.wait(cx, |app, cx| {
        entries(app)[1]["hidden"] == true && app.visual(cx).debug_bounds(settings).is_none()
    });

    let transfer = support::Folder::new();
    let exported = transfer.0.join("exported");
    orbit_host::export_pro_presenter_workspace_to(
        &app.workspace.0.join("workspace.json"),
        &exported,
    )
    .unwrap();
    let libraries: Vec<_> = std::fs::read_dir(exported.join("Libraries"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let selection = orbit_host::ImportSelection::parse(
        &json!({"libraries":libraries,"categories":["playlists","timers"]}),
    )
    .unwrap();
    let imported = orbit_host::import_pro_presenter_workspace_media(
        &exported,
        &transfer.0.join("imported/workspace.json"),
        &json!({}),
        None,
        true,
        &selection,
    )
    .unwrap();
    let header = imported["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["items"].as_array().into_iter().flatten())
        .find(|entry| entry["title"] == "Service start")
        .unwrap();
    assert_eq!(header["color"], color);
    assert_eq!(header["actions"][0]["value"], "start");
    assert_eq!(
        header["actions"][0]["details"]["configuration"]["targetTimeSeconds"].as_f64(),
        Some(43_500.)
    );
    assert_eq!(
        header["actions"][0]["details"]["configuration"]["format"]["targetTimePeriod"],
        "pm"
    );
    assert!(imported["countdownTimers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|timer| timer["id"] == header["actions"][0]["targetId"]));
}
