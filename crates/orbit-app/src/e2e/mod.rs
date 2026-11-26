//! Operator workflows through the real UI, host, workspace files, and output process on GPUI's test platform.
mod mcp;
mod output_freeze;
mod playlist_menu;
mod stage_import;
mod support;
mod theme_import;
mod video_outputs;
use gpui::{AppContext, TestAppContext};
use serde_json::{json, Value};
use support::{click, context, Scenario};

fn looks_window(app: &Scenario, cx: &mut TestAppContext) -> gpui::VisualTestContext {
    let runtime = app.runtime(cx);
    let window = cx.add_window(move |window, cx| {
        let view = cx.new(|cx| crate::settings::SettingsView::new(runtime, cx));
        view.update(cx, |view, cx| view.select(crate::settings::LOOKS_PANEL, cx));
        orbit_ui::controls::Root::new(view, window, cx)
    });
    let visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual
}

#[gpui::test]
fn audience_looks_autosave_context_actions_and_reopen(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, seed);
    assert_eq!(app.snapshot()["audienceLooks"][0]["title"], "Default");
    let runtime = app.runtime(cx);
    cx.update(|cx| {
        crate::output::windows::mutate_config(
            &runtime,
            json!({"type":"add", "screenType":"audience", "baseLabel":"Screen"}),
            cx,
        )
    });
    app.wait(cx, |app, _| {
        app.snapshot()["screenConfig"]
            .as_object()
            .is_some_and(|screens| !screens.is_empty())
    });
    let screen = app.snapshot()["screenConfig"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    let mut visual = looks_window(&app, cx);
    assert!(visual.debug_bounds("look-title").is_some());
    for selector in [
        "button-look-save",
        "button-look-revert",
        "button-look-activate",
        "button-look-duplicate",
        "button-look-delete",
    ] {
        assert!(visual.debug_bounds(selector).is_none());
    }
    // Default can be taken from the toolbar before its first edit materializes the screen columns.
    cx.update(|cx| {
        let configs = crate::settings::persist::shown(&runtime, cx)["screenConfig"].clone();
        let look = runtime.content.read(cx).workspace["audienceLooks"][0].clone();
        crate::output::looks::activate(&runtime, &configs, &look, cx).unwrap();
    });
    context(&mut visual, "look-row-default-look");
    click(&mut visual, "menu-item-Duplicate");
    let duplicate = cx.update(|cx| runtime.content.read(cx).workspace["audienceLooks"][1].clone());
    let id = duplicate["id"].as_str().unwrap().to_string();
    let row = format!("look-row-{id}");
    context(&mut visual, &row);
    click(&mut visual, "menu-item-Rename");
    visual.simulate_input("Broadcast");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert!(
        cx.update(|cx| runtime.content.read(cx).workspace["audienceLooks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|look| look["title"] == "Broadcast"))
    );
    let release = support::hold_workspace_writes(&app, cx);
    let transition = visual.debug_bounds("look-transition").unwrap();
    visual.simulate_click(
        gpui::point(
            transition.left() + transition.size.width * 0.7,
            transition.center().y,
        ),
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    click(&mut visual, &format!("look-{id}-{screen}-props"));
    click(&mut visual, &format!("look-{id}-{screen}-media"));
    assert!(cx.update(|cx| {
        let content = runtime.content.read(cx);
        let look = content.workspace["audienceLooks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["id"] == id)
            .unwrap();
        look["transitionSeconds"]
            .as_f64()
            .is_some_and(|seconds| seconds > 5.)
            && look["screenLooks"][0]["lookLayers"]["props"] == false
            && look["screenLooks"][0]["lookLayers"]["media"] == false
    }));
    context(&mut visual, &row);
    click(&mut visual, "menu-item-Make live");
    assert!(cx.update(|cx| runtime.output.read(cx).live["activeAudienceLookId"] == id));
    assert!(cx.update(
        |cx| crate::settings::persist::shown(&runtime, cx)["screenConfig"][&screen]["lookLayers"]
            ["props"]
            == false
    ));
    // Live edits affect the screens immediately but keep the saved preset intact until Save.
    click(&mut visual, "looks-live");
    click(&mut visual, &format!("look-{id}-{screen}-media"));
    assert!(cx.update(|cx| {
        let look = crate::output::looks::live_look(&runtime, cx).unwrap();
        look.current != look.saved
            && look.current["screenLooks"][0]["lookLayers"]["media"] == true
            && runtime.content.read(cx).workspace["audienceLooks"][1]["screenLooks"][0]
                ["lookLayers"]["media"]
                == false
    }));
    click(&mut visual, "button-look-save-options");
    assert!(
        visual.debug_bounds("menu-item-Save").unwrap().top()
            >= visual
                .debug_bounds("button-look-save-options")
                .unwrap()
                .bottom()
    );
    click(&mut visual, "menu-item-Revert to preset");
    assert!(cx.update(|cx| {
        let look = crate::output::looks::live_look(&runtime, cx).unwrap();
        look.current == look.saved
            && crate::settings::persist::shown(&runtime, cx)["screenConfig"][&screen]["lookLayers"]
                ["media"]
                == false
    }));
    click(&mut visual, &format!("look-{id}-{screen}-media"));
    click(&mut visual, "button-look-save-options");
    click(&mut visual, "menu-item-Save");
    assert!(cx.update(|cx| {
        let look = crate::output::looks::live_look(&runtime, cx).unwrap();
        look.current == look.saved
            && runtime.content.read(cx).workspace["audienceLooks"][1]["screenLooks"][0]
                ["lookLayers"]["media"]
                == true
    }));
    click(&mut visual, &format!("look-{id}-{screen}-props"));
    click(&mut visual, "button-look-save-options");
    click(&mut visual, "menu-item-Save as…");
    visual.simulate_input("Evening live");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    let copy = cx.update(|cx| runtime.content.read(cx).workspace["audienceLooks"][2].clone());
    assert_eq!(copy["title"], "Evening live");
    assert_eq!(copy["screenLooks"][0]["lookLayers"]["props"], true);
    assert!(cx.update(
        |cx| runtime.content.read(cx).workspace["audienceLooks"][1]["screenLooks"][0]["lookLayers"]
            ["props"]
            == false
    ));
    context(
        &mut visual,
        &format!("look-row-{}", copy["id"].as_str().unwrap()),
    );
    click(&mut visual, "menu-item-Delete");
    click(&mut visual, &row);
    let position = visual.debug_bounds("look-title").unwrap().center();
    visual.simulate_event(gpui::MouseDownEvent {
        position,
        button: gpui::MouseButton::Left,
        click_count: 2,
        first_mouse: false,
        modifiers: gpui::Modifiers::default(),
    });
    visual.simulate_event(gpui::MouseUpEvent {
        position,
        button: gpui::MouseButton::Left,
        click_count: 2,
        modifiers: gpui::Modifiers::default(),
    });
    visual.simulate_input("Broadcast renamed");
    let fade = visual.debug_bounds("look-transition").unwrap();
    visual.simulate_click(
        gpui::point(fade.center().x, fade.bottom() + gpui::px(20.)),
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    assert!(cx.update(
        |cx| runtime.content.read(cx).workspace["audienceLooks"][1]["title"] == "Broadcast renamed"
    ));
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        app.snapshot()["screenConfig"][&screen]["lookLayers"]["media"] == true
            && app.snapshot()["audienceLooks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| {
                    l["title"] == "Broadcast renamed"
                        && l["screenLooks"][0]["lookLayers"]["props"] == false
                })
    });
    context(&mut visual, "look-row-default-look");
    click(&mut visual, "menu-item-Delete");
    app.wait(cx, |app, _| {
        app.snapshot()["audienceLooks"].as_array().unwrap().len() == 1
    });
    app.restart(cx);
    let mut visual = looks_window(&app, cx);
    assert!(visual
        .debug_bounds(Box::leak(row.into_boxed_str()))
        .is_some());
    assert_eq!(
        app.snapshot()["audienceLooks"][0]["title"],
        "Broadcast renamed"
    );
    click(&mut visual, "button-look-create");
    visual.simulate_input("Evening");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, _| {
        app.snapshot()["audienceLooks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["title"] == "Evening")
    });
}

#[gpui::test]
fn prop_and_macro_collections_support_context_actions_drag_and_reopening(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, |host| {
        seed(host);
        let path =
            host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
        let loaded = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let saved = host.invoke("saveWorkspaceContent", vec![path.clone(), json!({
            "macroCollections": [{"id":"first", "title":"First Collection"}],
            "macros": [{"id":"first-cue", "title":"First cue", "collectionIds":["first"], "actions":[]}]
        }), loaded["revision"].clone()]).unwrap();
        let folder = host
            .invoke(
                "mutateProps",
                vec![
                    path.clone(),
                    json!({"type":"createFolder", "baseTitle":"First Collection"}),
                    saved["revision"].clone(),
                ],
            )
            .unwrap();
        let prop = host
            .invoke(
                "mutateProps",
                vec![
                    path.clone(),
                    json!({"type":"create", "baseTitle":"First prop", "folder":"First Collection"}),
                    folder["revision"].clone(),
                ],
            )
            .unwrap();
        let derived = host.invoke(
            "mutateProps",
            vec![
                path.clone(),
                json!({"type":"create", "baseTitle":"Derived prop", "folder":"Derived Collection"}),
                prop["revision"].clone(),
            ],
        )
        .unwrap();
        let mut blank = prop["workspace"]["props"][0]["slide"].clone();
        blank.as_object_mut().unwrap().remove("layers");
        blank["name"] = json!("First prop");
        blank["text"] = json!("");
        blank["backgroundEnabled"] = json!(false);
        host.invoke(
            "mutateProps",
            vec![
                path,
                json!({
                    "type":"updateDocument", "propId":prop["createdPropId"], "slide":blank
                }),
                derived["revision"].clone(),
            ],
        )
        .unwrap();
    });
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut visual = app.visual(cx);
    for (command, scope, key, is_props) in [
        (
            "view-macros",
            "macro-collections",
            "macroCollections",
            false,
        ),
        ("view-props", "prop-collections", "propFolders", true),
    ] {
        visual.update(|window, cx| assert!(commands.run(command, false, window, cx)));
        visual.run_until_parked();
        assert!(visual
            .debug_bounds("button-macro-collections-manage")
            .is_none());
        assert!(visual
            .debug_bounds("button-prop-collections-manage")
            .is_none());
        if is_props {
            let edit = visual.debug_bounds("button-prop-edit-collection").unwrap();
            let add = visual.debug_bounds("button-prop-create").unwrap();
            assert!(edit.right() <= add.left());
            let props = app.snapshot();
            let first_id = props["props"][0]["id"].as_str().unwrap();
            let derived_id = props["props"][1]["id"].as_str().unwrap();
            let first: &'static str =
                Box::leak(format!("authoring-prop-{first_id}").into_boxed_str());
            let derived: &'static str =
                Box::leak(format!("authoring-prop-{derived_id}").into_boxed_str());
            click(&mut visual, "button-prop-edit-collection");
            assert!(visual.debug_bounds(first).is_some());
            assert!(visual.debug_bounds(derived).is_none());
            assert!(visual
                .debug_bounds("button-canvas-create-document")
                .is_none());
            let runtime = app.runtime(cx);
            let current_prop = |cx: &mut TestAppContext| {
                cx.update(|cx| runtime.content.read(cx).workspace["props"][0].clone())
            };
            assert!(visual.debug_bounds("canvas-background-enabled").is_some());
            assert!(visual
                .debug_bounds("media-transparency-slide-canvas")
                .is_some());
            click(&mut visual, "canvas-background-enabled");
            assert!(visual
                .debug_bounds("media-transparency-slide-canvas")
                .is_none());
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(700));
            app.wait(cx, |_, cx| {
                current_prop(cx)["slide"]["backgroundEnabled"] == true
            });
            click(&mut visual, "prop-editor-auto-clear-field");
            click(&mut visual, "select-option-2");
            app.wait(cx, |_, cx| current_prop(cx)["autoClearSeconds"] == 10.);
            click(&mut visual, "prop-editor-auto-clear-field");
            click(&mut visual, "select-option-8");
            visual.simulate_input("12.5");
            click(&mut visual, "button-theme-dialog-submit");
            app.wait(cx, |_, cx| current_prop(cx)["autoClearSeconds"] == 12.5);
            click(&mut visual, "prop-editor-auto-clear-field");
            click(&mut visual, "select-option-0");
            app.wait(cx, |_, cx| current_prop(cx)["autoClearSeconds"].is_null());
            click(&mut visual, "button-prop-editor-transition");
            click(&mut visual, "transition-effect-0");
            app.wait(cx, |_, cx| current_prop(cx)["transition"].is_object());
            click(&mut visual, "transition-use-default");
            app.wait(cx, |_, cx| current_prop(cx)["transition"].is_null());
            click(&mut visual, "dialog-close");
            click(&mut visual, "button-theme-text-tool");
            assert!(visual
                .debug_bounds("prop-editor-auto-clear-field")
                .is_none());
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(700));
            app.wait(cx, |app, _| {
                app.snapshot()["props"][0]["slide"]["layers"]
                    .as_array()
                    .is_some_and(|layers| layers.len() == 1)
            });
            click(&mut visual, "canvas-prop-collection-picker");
            click(&mut visual, "select-option-1");
            assert!(visual.debug_bounds(first).is_none());
            assert!(visual.debug_bounds(derived).is_some());
            click(&mut visual, "button-new-editor-document");
            app.wait(cx, |app, cx| {
                app.snapshot()["props"].as_array().unwrap().len() == 3
                    && app
                        .visual(cx)
                        .debug_bounds("button-new-editor-document")
                        .is_some()
            });
            assert_eq!(
                app.snapshot()["props"][2]["slide"]["backgroundEnabled"],
                false
            );
            let added: &'static str = Box::leak(
                format!(
                    "authoring-prop-{}",
                    app.snapshot()["props"][2]["id"].as_str().unwrap()
                )
                .into_boxed_str(),
            );
            app.wait(cx, |app, cx| app.visual(cx).debug_bounds(added).is_some());
            click(&mut visual, derived);
            click(&mut visual, "button-theme-text-tool");
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(700));
            app.wait(cx, |app, _| {
                app.snapshot()["props"][1]["slide"]["layers"]
                    .as_array()
                    .is_some_and(|layers| layers.len() == 2)
            });
            click(&mut visual, "canvas-prop-collection-picker");
            click(&mut visual, "select-option-0");
            assert!(visual.debug_bounds(first).is_some());
            visual.update(|window, cx| assert!(commands.run("close-canvas", false, window, cx)));
            visual.run_until_parked();
        }
        click(&mut visual, &format!("collection-trigger-{scope}"));
        let trigger: &'static str =
            Box::leak(format!("collection-trigger-{scope}").into_boxed_str());
        let popup: &'static str = Box::leak(format!("collection-popup-{scope}").into_boxed_str());
        assert!(
            visual.debug_bounds(trigger).unwrap().size.width
                < visual.debug_bounds(popup).unwrap().size.width
        );
        let first = if is_props {
            "First Collection"
        } else {
            "first"
        };
        context(&mut visual, &format!("collection-row-{scope}-{first}"));
        click(&mut visual, "menu-item-Rename");
        let rename_field: &'static str =
            Box::leak(format!("collection-rename-{scope}").into_boxed_str());
        assert!(
            visual.debug_bounds(rename_field).is_some(),
            "The rename input must open"
        );
        visual.simulate_input("Renamed collection");
        visual.simulate_keystrokes("enter");
        assert!(
            visual.debug_bounds(rename_field).is_none(),
            "Enter must submit the collection name"
        );
        let first = if is_props {
            "Renamed collection"
        } else {
            "first"
        };
        let first_row: &'static str =
            Box::leak(format!("collection-row-{scope}-{first}").into_boxed_str());
        app.wait(cx, |app, cx| {
            let saved = app.snapshot();
            let runtime = app.runtime(cx);
            if cx.update(|cx| runtime.content.read(cx).workspace[key] != saved[key]) {
                return false;
            }
            (if is_props {
                saved[key][0] == "Renamed collection"
            } else {
                saved[key][0]["title"] == "Renamed collection"
            }) && app.visual(cx).debug_bounds(first_row).is_some()
        });
        let release = support::hold_workspace_writes(&app, cx);
        click(&mut visual, &format!("button-collection-add-{scope}"));
        assert!(visual.debug_bounds(rename_field).is_some());
        visual.simulate_input("Fresh collection");
        visual.simulate_keystrokes("enter");
        let runtime = app.runtime(cx);
        assert_eq!(
            cx.update(|cx| runtime.content.read(cx).workspace[key]
                .as_array()
                .unwrap()
                .len()),
            2
        );
        assert_eq!(app.snapshot()[key].as_array().unwrap().len(), 1);
        release.send(()).unwrap();
        app.wait(cx, |app, _| {
            app.snapshot()[key]
                .as_array()
                .is_some_and(|items| items.len() == 2)
        });
        let saved = app.snapshot();
        let new = if is_props {
            saved[key][1].as_str().unwrap()
        } else {
            saved[key][1]["id"].as_str().unwrap()
        }
        .to_string();
        let new_row: &'static str =
            Box::leak(format!("collection-row-{scope}-{new}").into_boxed_str());
        app.wait(cx, |app, cx| app.visual(cx).debug_bounds(new_row).is_some());
        if is_props {
            click(&mut visual, new_row);
            click(&mut visual, "button-prop-edit-collection");
            assert!(visual
                .debug_bounds("button-canvas-create-document")
                .is_some());
            click(&mut visual, "button-new-editor-document");
            app.wait(cx, |app, _| {
                app.snapshot()["props"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|prop| prop["folder"] == new)
            });
            app.wait(cx, |app, cx| {
                app.visual(cx)
                    .debug_bounds("button-canvas-create-document")
                    .is_none()
            });
            visual.update(|window, cx| assert!(commands.run("close-canvas", false, window, cx)));
            visual.run_until_parked();
            click(&mut visual, &format!("collection-trigger-{scope}"));
        }
        context(&mut visual, first_row);
        click(&mut visual, "menu-item-Duplicate");
        app.wait(cx, |app, _| {
            app.snapshot()[key]
                .as_array()
                .is_some_and(|items| items.len() == 3)
        });
        let saved = app.snapshot();
        let copied = if is_props {
            saved[key][1].as_str().unwrap()
        } else {
            saved[key][1]["id"].as_str().unwrap()
        }
        .to_string();
        let copied_row: &'static str =
            Box::leak(format!("collection-row-{scope}-{copied}").into_boxed_str());
        app.wait(cx, |app, cx| {
            app.visual(cx).debug_bounds(copied_row).is_some()
        });
        if is_props {
            assert_eq!(
                saved["props"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|prop| prop["folder"] == copied)
                    .count(),
                1
            );
        } else {
            assert!(saved["macros"][0]["collectionIds"]
                .as_array()
                .unwrap()
                .contains(&json!(copied)));
        }
        let target = if is_props {
            format!("collection-row-{scope}-Derived Collection")
        } else {
            first_row.to_string()
        };
        let release = support::hold_workspace_writes(&app, cx);
        support::drag_row(
            &mut visual,
            copied_row,
            &target,
            is_props,
            &format!("collection-drop-{scope}"),
        );
        let runtime = app.runtime(cx);
        let shown = cx.update(|cx| runtime.content.read(cx).workspace[key].clone());
        if is_props {
            assert_eq!(shown.as_array().unwrap().last(), Some(&json!(copied)));
            assert_ne!(
                app.snapshot()[key].as_array().unwrap().last(),
                Some(&json!(copied))
            );
        } else {
            assert_eq!(shown[0]["id"], copied);
            assert_ne!(app.snapshot()[key][0]["id"], copied);
        }
        release.send(()).unwrap();
        app.wait(cx, |app, cx| {
            let saved = app.snapshot();
            let runtime = app.runtime(cx);
            if cx.update(|cx| runtime.content.read(cx).workspace[key] != saved[key]) {
                return false;
            }
            if is_props {
                saved[key].as_array().unwrap().last() == Some(&json!(copied))
                    && saved[key]
                        .as_array()
                        .unwrap()
                        .contains(&json!("Derived Collection"))
            } else {
                saved[key][0]["id"] == copied
            }
        });
        context(&mut visual, new_row);
        click(&mut visual, "menu-item-Delete");
        click(
            &mut visual,
            &format!("button-collection-delete-confirm-{scope}"),
        );
        app.wait(cx, |app, cx| {
            let saved = app.snapshot();
            let runtime = app.runtime(cx);
            if cx.update(|cx| runtime.content.read(cx).workspace[key] != saved[key]) {
                return false;
            }
            !saved[key].as_array().unwrap().iter().any(|entry| {
                if is_props {
                    entry == &new
                } else {
                    entry["id"] == new
                }
            }) && app.visual(cx).debug_bounds(new_row).is_none()
        });
        if is_props {
            let path = app.host.invoke("getSettings", vec![]).unwrap()
                ["presenterOrbitWorkspacePath"]
                .clone();
            let saved = app
                .host
                .invoke("readWorkspaceSnapshot", vec![path.clone()])
                .unwrap();
            for folders in [json!(["unknown"]), json!([copied, copied])] {
                assert!(app
                    .host
                    .invoke(
                        "mutateProps",
                        vec![
                            path.clone(),
                            json!({"type":"reorderFolders", "folders": folders}),
                            saved["revision"].clone()
                        ]
                    )
                    .is_err());
            }
        }
    }
    let saved = app.snapshot();
    app.restart(cx);
    let reopened = app.snapshot();
    for key in ["props", "propFolders", "macros", "macroCollections"] {
        assert_eq!(saved[key], reopened[key]);
    }
}

#[gpui::test]
fn props_views_size_and_default_transition_reach_output_and_persist(cx: &mut TestAppContext) {
    use orbit_ui::{ActiveTheme, AppearanceSettings, ThemeSetting, UiDensity};
    let mut app = Scenario::new(cx, true, |host| {
        seed(host);
        let path =
            host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        host.invoke(
            "saveWorkspaceContent",
            vec![
                path,
                json!({"layerTransitions": {
                    "background": crate::transitions::fade(1.7),
                    "messages": crate::transitions::fade(0.8)
                }}),
                snapshot["revision"].clone(),
            ],
        )
        .unwrap();
    });
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut visual = app.visual(cx);
    visual.update(|window, cx| assert!(commands.run("view-props", false, window, cx)));
    visual.run_until_parked();
    let release = support::hold_workspace_writes(&app, cx);
    click(&mut visual, "button-prop-create");
    let runtime = app.runtime(cx);
    let id = cx.update(|cx| {
        runtime.content.read(cx).workspace["props"][0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    });
    visual.simulate_input("Immediate prop");
    click(&mut visual, &format!("button-prop-rename-save-{id}"));
    assert_eq!(
        cx.update(|cx| runtime.content.read(cx).workspace["props"][0]["title"].clone()),
        "Immediate prop"
    );
    assert!(app.snapshot()["props"].as_array().unwrap().is_empty());
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        app.snapshot()["props"]
            .as_array()
            .is_some_and(|props| props.len() == 1)
    });
    let id = app.snapshot()["props"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let preview: &'static str = Box::leak(format!("prop-preview-{id}").into_boxed_str());
    app.wait(cx, |app, cx| app.visual(cx).debug_bounds(preview).is_some());
    let size = visual.debug_bounds("props-size-slider").unwrap();
    visual.simulate_click(
        gpui::point(size.left() + gpui::px(2.), size.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    let small = visual.debug_bounds(preview).unwrap().size.width;
    let spacing = visual.update(|_, cx| f32::from(cx.theme().metrics.field_padding_x));
    let grid_width = visual.debug_bounds("props-rows").unwrap().size.width;
    assert!((f32::from(small) * 5. + spacing * 6. - f32::from(grid_width)).abs() < 2.);
    visual.simulate_click(
        gpui::point(size.right() - gpui::px(2.), size.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    let large = visual.debug_bounds(preview).unwrap().size.width;
    assert!((f32::from(large) * 2. + spacing * 3. - f32::from(grid_width)).abs() < 2.);
    assert!(large > small * 2.);
    let views = visual.debug_bounds("props-view-controls").unwrap();
    visual.simulate_click(
        gpui::point(views.left() + views.size.width * 0.75, views.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("props-table-header").is_some());
    assert!(visual.debug_bounds(preview).unwrap().size.width < large);
    visual.simulate_click(
        gpui::point(views.left() + views.size.width * 0.25, views.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("props-table-header").is_none());
    assert_eq!(visual.debug_bounds(preview).unwrap().size.width, large);
    for (theme, density, locale, scale) in [
        (ThemeSetting::Dark, UiDensity::Compact, "en", 1.),
        (ThemeSetting::Light, UiDensity::Spacious, "nl", 2.),
        (ThemeSetting::Red, UiDensity::Comfortable, "nl", 1.5),
    ] {
        visual.update(|window, cx| {
            cx.set_global(orbit_ui::Translator::new(locale));
            orbit_ui::set_appearance_settings(
                cx,
                AppearanceSettings {
                    theme,
                    density,
                    ..Default::default()
                },
            );
            window.set_scale_factor(scale);
        });
        visual.run_until_parked();
        let footer = visual.debug_bounds("props-filter-bar").unwrap();
        let views = visual.debug_bounds("props-view-controls").unwrap();
        let size = visual.debug_bounds("props-size-slider").unwrap();
        assert!(views.right() <= size.left());
        for bounds in [views, size] {
            assert!(footer.contains(&bounds.origin));
            assert!(footer.contains(&gpui::point(
                bounds.right() - gpui::px(1.),
                bounds.bottom() - gpui::px(1.)
            )));
        }
    }
    let transition = visual.debug_bounds("props-transition-control").unwrap();
    visual.simulate_click(transition.center(), Default::default());
    visual.run_until_parked();
    app.wait(cx, |app, _| {
        app.snapshot()["layerTransitions"]["props"]["effectId"]
            == orbit_host::native_transitions::DISSOLVE_ID
    });
    click(&mut visual, preview);
    let runtime = app.runtime(cx);
    app.wait(cx, |app, cx| {
        app.host
            .state::<crate::output_process::OutputRoute>()
            .0
            .get()
            .unwrap()
            .attached()
            && cx.update(|cx| {
                runtime.output.read(cx).live["livePropIds"]
                    .as_array()
                    .is_some_and(|ids| ids.iter().any(|live| live == &id))
            })
    });
    let screen = cx.update(|cx| {
        crate::output::view::screens(runtime.output.read(cx).snapshot.as_ref().unwrap())[0]
            .0
            .clone()
    });
    app.wait(cx, |app, _| {
        crate::state::runtime::output_call(&app.host, "getOutputContent", vec![json!(screen)])
            .unwrap()["layerTransitions"]["props"]["transitions"][&id]["effectId"]
            == orbit_host::native_transitions::DISSOLVE_ID
    });
    visual.update(|_, cx| cx.set_global(orbit_ui::Translator::new("en")));
    visual.run_until_parked();
    context(&mut visual, preview);
    click(&mut visual, "menu-item-Transitions");
    assert!(visual.debug_bounds("transition-picker-panel").is_some());
    app.wait(cx, |app, cx| {
        app.visual(cx)
            .debug_bounds("transition-effect-preview-0")
            .is_some()
    });
    click(&mut visual, "transition-effect-0");
    app.wait(cx, |app, _| {
        app.snapshot()["props"][0]["transition"].is_object()
    });
    click(&mut visual, "transition-use-default");
    app.wait(cx, |app, _| {
        app.snapshot()["props"][0]["transition"].is_null()
    });
    click(&mut visual, "dialog-close");
    context(&mut visual, preview);
    click(&mut visual, "menu-item-Auto-clear");
    click(&mut visual, "menu-item-After 5 seconds");
    click(&mut visual, preview);
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.output.read(cx).live["livePropIds"]
                .as_array()
                .is_some_and(|ids| !ids.iter().any(|live| live == &id))
        })
    });
    click(&mut visual, preview);
    let timer = format!("prop-timer-{id}");
    let deadline = |cx: &mut TestAppContext| {
        cx.update(|cx| runtime.output.read(cx).live["remotePropDeadlines"][&id].as_f64())
    };
    app.wait(cx, |_, cx| {
        deadline(cx).is_some_and(|deadline| deadline > 0.)
    });
    let original = deadline(cx).unwrap();
    click(&mut visual, &timer);
    app.wait(cx, |_, cx| {
        deadline(cx).is_some_and(|deadline| deadline < 0.)
            && crate::automation::announcements::now_ms() > original
    });
    assert!(cx.update(|cx| runtime.output.read(cx).live["livePropIds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|live| live == &id)));
    click(&mut visual, &timer);
    app.wait(cx, |_, cx| {
        deadline(cx).is_some_and(|deadline| deadline > original)
    });
    context(&mut visual, preview);
    click(&mut visual, "menu-item-Duplicate");
    app.wait(cx, |app, _| {
        app.snapshot()["props"]
            .as_array()
            .is_some_and(|props| props.len() == 2)
    });
    let second = app.snapshot()["props"][1]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let views = visual.debug_bounds("props-view-controls").unwrap();
    visual.simulate_click(
        gpui::point(views.left() + views.size.width * 0.75, views.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    support::drag_row(
        &mut visual,
        &format!("prop-row-{second}"),
        &format!("prop-row-{id}"),
        false,
        "prop-drop-indicator",
    );
    app.wait(cx, |app, _| app.snapshot()["props"][0]["id"] == second);
    let transition = visual.debug_bounds("props-transition-control").unwrap();
    visual.simulate_click(
        gpui::point(transition.left() + gpui::px(12.), transition.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    app.wait(cx, |app, _| {
        app.snapshot()["layerTransitions"]["props"].is_null()
    });
    assert_eq!(
        app.snapshot()["layerTransitions"]["background"],
        crate::transitions::fade(1.7)
    );
    assert_eq!(
        app.snapshot()["layerTransitions"]["messages"],
        crate::transitions::fade(0.8)
    );
    let transition = visual.debug_bounds("props-transition-control").unwrap();
    visual.simulate_click(transition.center(), Default::default());
    visual.run_until_parked();
    app.wait(cx, |app, _| {
        app.snapshot()["layerTransitions"]["props"]["effectId"]
            == orbit_host::native_transitions::DISSOLVE_ID
    });
    app.restart(cx);
    assert_eq!(
        app.snapshot()["layerTransitions"]["props"]["effectId"],
        orbit_host::native_transitions::DISSOLVE_ID
    );
}

#[gpui::test]
fn audience_messages_appear_autosave_and_survive_reopening(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, seed);
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut visual = app.visual(cx);
    visual.update(|window, cx| {
        assert!(commands.run("view-messages", false, window, cx));
    });
    visual.run_until_parked();
    click(&mut visual, "button-message-create");
    assert!(visual.debug_bounds("message-draft").is_some());
    assert!(visual.debug_bounds("message-field-title").is_some());
    visual.simulate_input("Welcome");
    visual.simulate_keystrokes("tab");
    visual.simulate_input("Please silence your phone");
    app.wait(cx, |app, cx| {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        app.snapshot()["messages"]
            .as_array()
            .is_some_and(|messages| {
                messages.iter().any(|message| {
                    message["title"] == "Welcome" && message["text"] == "Please silence your phone"
                })
            })
            && app.visual(cx).debug_bounds("message-draft").is_none()
    });
    let id = app.snapshot()["messages"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let row: &'static str = Box::leak(format!("message-{id}").into_boxed_str());
    assert!(app.visual(cx).debug_bounds(row).is_some());
    let runtime = app.runtime(cx);
    assert!(cx.update(|cx| runtime.output.read(cx).live["activeMessageId"].is_null()));
    click(&mut visual, "message-field-title");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("Renamed welcome");
    visual.simulate_keystrokes("tab");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("The service begins shortly");
    visual.update(|window, cx| {
        assert!(commands.run("view-audio", false, window, cx));
    });
    visual.run_until_parked();
    app.wait(cx, |app, cx| {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        app.snapshot()["messages"][0]["title"] == "Renamed welcome"
            && app.snapshot()["messages"][0]["text"] == "The service begins shortly"
    });
    visual.update(|window, cx| {
        assert!(commands.run("view-messages", false, window, cx));
    });
    visual.run_until_parked();
    click(&mut visual, "button-message-create");
    visual.simulate_input("Second message");
    visual.simulate_keystrokes("tab");
    visual.simulate_input("Second message text");
    app.wait(cx, |app, cx| {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        app.snapshot()["messages"]
            .as_array()
            .is_some_and(|messages| messages.len() == 2)
            && app.visual(cx).debug_bounds("message-draft").is_none()
    });
    let second = app.snapshot()["messages"][1]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let second_row: &'static str = Box::leak(format!("message-{second}").into_boxed_str());
    for (source, target, after, first) in [
        (row, second_row, true, &second),
        (row, second_row, false, &id),
        (second_row, row, false, &second),
    ] {
        let from = visual.debug_bounds(source).unwrap().center();
        let target = visual.debug_bounds(target).unwrap();
        let to = gpui::point(
            target.center().x,
            if after {
                target.bottom() - gpui::px(2.)
            } else {
                target.top() + gpui::px(2.)
            },
        );
        visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
        visual.simulate_mouse_move(
            from + gpui::point(gpui::px(10.), gpui::px(0.)),
            Some(gpui::MouseButton::Left),
            Default::default(),
        );
        visual.simulate_mouse_move(to, Some(gpui::MouseButton::Left), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("message-drop-line").is_some());
        visual.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
        visual.run_until_parked();
        app.wait(cx, |app, cx| {
            app.snapshot()["messages"][0]["id"] == *first
                && app
                    .visual(cx)
                    .debug_bounds(if first == &id { row } else { second_row })
                    .unwrap()
                    .top()
                    < app
                        .visual(cx)
                        .debug_bounds(if first == &id { second_row } else { row })
                        .unwrap()
                        .top()
        });
    }
    app.restart(cx);
    assert_eq!(app.snapshot()["messages"][0]["id"], second);
    assert_eq!(app.snapshot()["messages"][1]["id"], id);
    assert_eq!(app.snapshot()["messages"][1]["title"], "Renamed welcome");
    assert_eq!(
        app.snapshot()["messages"][1]["text"],
        "The service begins shortly"
    );
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut visual = app.visual(cx);
    visual.update(|window, cx| {
        assert!(commands.run("view-messages", false, window, cx));
    });
    visual.run_until_parked();
    click(&mut visual, row);
    click(&mut visual, "button-message-show");
    let runtime = app.runtime(cx);
    app.wait(cx, |_, cx| {
        cx.update(|cx| runtime.output.read(cx).live["activeMessageId"] == id)
    });
    click(&mut visual, "button-message-clear");
    app.wait(cx, |_, cx| {
        cx.update(|cx| runtime.output.read(cx).live["activeMessageId"].is_null())
    });
    context(&mut visual, row);
    click(&mut visual, "button-message-delete");
    app.wait(cx, |app, cx| {
        app.snapshot()["messages"]
            .as_array()
            .is_some_and(|messages| messages.len() == 1 && messages[0]["id"] == second)
            && app.visual(cx).debug_bounds(row).is_none()
    });
    click(&mut visual, "button-message-create");
    context(&mut visual, "message-draft");
    click(&mut visual, "button-message-delete");
    assert!(app.visual(cx).debug_bounds("message-draft").is_none());
}

fn audio_fixture(path: &std::path::Path, frequency: f32) {
    let rate = 48_000_u32;
    let count = rate * 8;
    let bytes = count * 2;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&bytes.to_le_bytes());
    for i in 0..count {
        let sample =
            ((i as f32 * frequency * std::f32::consts::TAU / rate as f32).sin() * 6000.) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, wav).unwrap();
}

#[gpui::test]
fn audio_shuffle_crossfade_controls_persist_and_advance_before_the_end(cx: &mut TestAppContext) {
    let files = support::Folder::new();
    let paths: Vec<_> = (0..3)
        .map(|i| {
            let path = files.0.join(format!("Track {i}.wav"));
            audio_fixture(&path, 220. + i as f32 * 110.);
            path
        })
        .collect();
    let mut app = Scenario::new(cx, true, |host| {
        seed(host);
        let settings = host.invoke("getSettings", vec![]).unwrap();
        let path = settings["presenterOrbitWorkspacePath"].clone();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let mut library = snapshot["workspace"]["mediaLibraries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|library| library["id"] == "default-audio")
            .unwrap()
            .clone();
        library["audioPlaylistMode"] = json!("next");
        for bad in [json!(-1), json!(11), json!("invalid")] {
            let mut invalid = library.clone();
            invalid["audioCrossfadeSeconds"] = bad;
            assert!(host
                .invoke(
                    "saveMediaLibrary",
                    vec![path.clone(), invalid, snapshot["revision"].clone()]
                )
                .is_err());
        }
        let saved = host
            .invoke(
                "saveMediaLibrary",
                vec![path.clone(), library, snapshot["revision"].clone()],
            )
            .unwrap();
        let assets:Vec<_>=paths.iter().enumerate().map(|(i,path)|json!({"id":format!("audio-{i}"),"title":format!("Track {i}"),"type":"audio","libraryId":"default-audio","sourcePath":path})).collect();
        let saved = host
            .invoke(
                "saveMediaSettings",
                vec![path.clone(), json!(assets), saved["revision"].clone()],
            )
            .unwrap();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let mut playlist = snapshot["workspace"]["mediaLibraries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|library| library["id"] == "default-audio")
            .unwrap()
            .clone();
        playlist["id"] = json!("audio-playlist");
        playlist["title"] = json!("Shared tracks");
        playlist["itemFiles"] = json!(snapshot["workspace"]["media"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["assetFile"].clone())
            .collect::<Vec<_>>());
        host.invoke(
            "saveMediaLibrary",
            vec![path, playlist, saved["revision"].clone()],
        )
        .unwrap();
    });
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut visual = app.visual(cx);
    visual.update(|window, cx| {
        commands.run("view-audio", false, window, cx);
    });
    visual.run_until_parked();
    click(&mut visual, "media-library-row-audio-playlist");
    let release = support::hold_workspace_writes(&app, cx);
    support::drag_row(
        &mut visual,
        "media-item-audio-2",
        "media-item-audio-0",
        false,
        "media-item-drop-line",
    );
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        app.snapshot()["mediaLibraries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|library| {
                library["id"] == "audio-playlist"
                    && library["itemFiles"][0]
                        == app.snapshot()["media"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|item| item["id"] == "audio-2")
                            .unwrap()["assetFile"]
            })
    });
    context(&mut visual, "media-item-audio-0");
    let mut previous = None;
    for label in [
        "Open File Location",
        "Playback Behavior",
        "Inspector...",
        "Rename",
        "Delete",
    ] {
        let selector = Box::leak(format!("menu-item-{label}").into_boxed_str());
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("Missing audio menu item {label}"));
        if let Some(top) = previous {
            assert!(bounds.top() > top);
        }
        previous = Some(bounds.top());
    }
    assert!(visual.debug_bounds("menu-item-Copy").is_none());
    click(&mut visual, "menu-item-Playback Behavior");
    click(&mut visual, "menu-item-Stop");
    app.wait(cx, |app, _| {
        app.snapshot()["media"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "audio-0" && item["audioPlayback"] == "stop")
    });
    context(&mut visual, "media-item-audio-0");
    click(&mut visual, "menu-item-Playback Behavior");
    click(&mut visual, "menu-item-Next");
    app.wait(cx, |app, _| {
        app.snapshot()["media"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "audio-0" && item["audioPlayback"] == "next")
    });
    click(&mut visual, "button-audio-shuffle");
    app.wait(cx, |app, _| {
        app.snapshot()["mediaLibraries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|library| library["id"] == "audio-playlist" && library["audioShuffle"] == true)
    });
    let slider = visual.debug_bounds("audio-crossfade-control").unwrap();
    visual.simulate_click(
        gpui::point(slider.left() + slider.size.width * 0.25, slider.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    app.wait(cx, |app, _| {
        app.snapshot()["mediaLibraries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|library| {
                library["id"] == "audio-playlist"
                    && library["audioCrossfadeSeconds"]
                        .as_f64()
                        .is_some_and(|value| value > 2. && value < 3.)
            })
    });
    assert!(visual.debug_bounds("button-audio-playlist-play").is_none());
    click(&mut visual, "media-item-audio-0");
    let runtime = app.runtime(cx);
    app.wait(cx, |app, cx| {
        app.host
            .state::<crate::output_process::OutputRoute>()
            .0
            .get()
            .unwrap()
            .attached()
            && cx.update(|cx| !runtime.output.read(cx).live["audioId"].is_null())
    });
    let first = cx.update(|cx| runtime.output.read(cx).live["audioId"].clone());
    let screen = cx.update(|cx| {
        crate::output::view::screens(runtime.output.read(cx).snapshot.as_ref().unwrap())[0]
            .0
            .clone()
    });
    let remaining = std::cell::Cell::new(0.);
    app.wait(cx, |app, cx| {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        let state = crate::state::runtime::output_call(
            &app.host,
            "getNativeMusicPlayback",
            vec![json!(screen)],
        )
        .unwrap();
        if cx.update(|cx| runtime.output.read(cx).live["audioId"] == first) {
            if let (Some(duration), Some(position)) = (
                state["playback"]["duration"].as_f64(),
                state["playback"]["currentTime"].as_f64(),
            ) {
                remaining.set(duration - position);
            }
            false
        } else {
            true
        }
    });
    assert!(
        remaining.get() > 1.,
        "The next track must start during the crossfade window, not after EOF"
    );
    let second = cx.update(|cx| runtime.output.read(cx).live["audioId"].clone());
    assert_ne!(second, first);
    crate::state::runtime::output_call(
        &app.host,
        "controlPlayback",
        vec![json!({"layer":"music","action":"seek","position":0.8})],
    )
    .unwrap();
    app.wait(cx, |_, cx| {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        cx.update(|cx| runtime.output.read(cx).live["audioId"] != second)
    });
    let third = cx.update(|cx| runtime.output.read(cx).live["audioId"].clone());
    assert_ne!(third, first);
    assert_ne!(third, second);
    let content =
        crate::state::runtime::output_call(&app.host, "getOutputContent", vec![json!(screen)])
            .unwrap();
    assert!(content["musicCrossfadeSeconds"]
        .as_f64()
        .is_some_and(|seconds| seconds > 2.));
    context(&mut visual, "media-item-audio-1");
    click(&mut visual, "menu-item-Playback Behavior");
    click(&mut visual, "menu-item-Stop");
    app.wait(cx, |app, _| {
        app.snapshot()["media"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "audio-1" && item["audioPlayback"] == "stop")
    });
    click(&mut visual, "media-item-audio-1");
    app.wait(cx, |app, _| {
        crate::state::runtime::output_call(&app.host, "getOutputContent", vec![json!(screen)])
            .unwrap()["audioId"]
            == "audio-1"
    });
    crate::state::runtime::output_call(
        &app.host,
        "controlPlayback",
        vec![json!({"layer":"music","action":"seek","position":0.95})],
    )
    .unwrap();
    app.wait(cx, |app, cx| {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        let state = crate::state::runtime::output_call(
            &app.host,
            "getNativeMusicPlayback",
            vec![json!(screen)],
        )
        .unwrap();
        state["playback"]["status"] == "ended"
    });
    assert_eq!(
        cx.update(|cx| runtime.output.read(cx).live["audioId"].clone()),
        "audio-1"
    );
    click(&mut visual, "media-item-audio-0");
    app.wait(cx, |app, _| {
        let content =
            crate::state::runtime::output_call(&app.host, "getOutputContent", vec![json!(screen)])
                .unwrap();
        content["audioId"] == "audio-0"
            && content["musicCrossfadeSeconds"]
                .as_f64()
                .is_some_and(|seconds| seconds > 2.)
    });
    let accepted = crate::state::runtime::output_call(
        &app.host,
        "updateOutputScene",
        vec![json!({
            "version":2,"kind":"patch","sessionId":"e2e-audio-validation","clientSequence":1,
            "baseRevision":9_007_199_254_740_991_u64,"patch":{}
        })],
    )
    .unwrap();
    assert_eq!(accepted["cacheMiss"], true);
    for playlist in [json!(null), json!(5), json!("missing-playlist")] {
        let mut invalid = accepted["snapshot"].clone();
        invalid["selectedAudio"]["playlistId"] = playlist;
        let error = crate::state::runtime::output_call(&app.host,"updateOutputScene",vec![json!({
            "version":2,"kind":"full","sessionId":"e2e-audio-invalid","clientSequence":1,"baseRevision":accepted["revision"],"snapshot":invalid
        })]).unwrap_err();
        assert!(
            error.contains("Selected audio playlist is invalid"),
            "{error}"
        );
    }
    app.restart(cx);
    let saved = app.snapshot();
    let library = saved["mediaLibraries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|library| library["id"] == "audio-playlist")
        .unwrap();
    assert_eq!(library["audioShuffle"], true);
    assert!(library["audioCrossfadeSeconds"].as_f64().unwrap() > 2.);
    let tracks = saved["media"].as_array().unwrap();
    assert_eq!(
        library["itemFiles"][0],
        tracks.iter().find(|item| item["id"] == "audio-2").unwrap()["assetFile"]
    );
    assert_eq!(
        tracks.iter().find(|item| item["id"] == "audio-1").unwrap()["audioPlayback"],
        "stop"
    );
}

fn seed(host: &orbit_host::Host) {
    let settings = host.invoke("getSettings", vec![]).unwrap();
    let path = settings["presenterOrbitWorkspacePath"].clone();
    let loaded = host.invoke("loadWorkspace", vec![path.clone()]).unwrap();
    host.invoke("mutatePresentationLibrary",vec![path,json!({"type":"createPresentation","libraryId":loaded["workspace"]["libraries"][0]["id"],"title":"Sunday service","content":"Opening line\n\nClosing line","splitMode":"paragraph","splitMarker":"","splitUnits":1,"playlistTitle":"Sunday"}),loaded["revision"].clone()]).unwrap();
}

#[gpui::test]
fn new_presentation_saves_the_chosen_theme_and_size(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, seed);
    let runtime = app.runtime(cx);
    let mut visual = app.visual(cx);
    support::primary_shortcut(&mut visual, "n");
    visual.run_until_parked();
    assert!(visual.debug_bounds("theme-picker-preview").is_some());
    let (thumbnail, theme_id) = cx.update(|cx| {
        let content = runtime.content.read(cx);
        let current = &runtime
            .ui
            .read(cx)
            .new_presentation
            .as_ref()
            .unwrap()
            .draft
            .theme_id;
        let (index, theme) = content
            .themes
            .iter()
            .filter(|theme| !theme.slides.is_empty())
            .enumerate()
            .find(|(_, theme)| {
                current
                    .split_once("::")
                    .is_none_or(|(id, _)| theme.id != id)
            })
            .unwrap();
        let recents =
            crate::theme_picker::known_recents(&content.recent_themes, &content.themes).len();
        (
            format!("theme-picker-all-{}", recents + index),
            theme.id.clone(),
        )
    });
    visual.simulate_input("Sized presentation");
    visual.run_until_parked();
    click(&mut visual, "button-new-presentation-theme-trigger");
    click(&mut visual, &thumbnail);
    assert!(visual.debug_bounds("theme-picker-panel").is_none());
    let assignment = cx.update(|cx| {
        runtime
            .ui
            .read(cx)
            .new_presentation
            .as_ref()
            .unwrap()
            .draft
            .theme_id
            .clone()
    });
    assert_eq!(assignment.split_once("::").unwrap().0, theme_id);
    click(&mut visual, "new-presentation-size");
    click(&mut visual, "select-option-3");
    click(&mut visual, "button-new-presentation-create");
    app.wait(cx, |app, _| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["title"] == "Sized presentation")
    });
    let snapshot = app.snapshot();
    let created = snapshot["library"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "Sized presentation")
        .unwrap();
    assert_eq!(created["size"], json!({"width":1024,"height":768}));
    assert_eq!(created["slides"][0]["width"], 1024);
    assert_eq!(created["slides"][0]["height"], 768);
    assert_eq!(created["slides"][0]["presentationThemeId"], assignment);

    let path = json!(app.workspace.0.join("workspace.json"));
    let loaded = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap();
    let rejected = app.host.invoke(
        "mutatePresentationLibrary",
        vec![
            path,
            json!({"type":"createPresentation","title":"Invalid size",
                "libraryId":snapshot["libraries"][0]["id"],"content":"","splitMode":"paragraph",
                "splitMarker":"//","splitUnits":1,"presentationThemeId":assignment,
                "size":{"width":0,"height":768}
            }),
            loaded["revision"].clone(),
        ],
    );
    assert!(rejected.unwrap_err().contains("Presentation dimensions"));
    assert_eq!(app.snapshot()["library"], snapshot["library"]);
}

#[gpui::test]
fn clipboard_import_edits_slide_breaks_and_saves_to_the_chosen_playlist(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, seed);
    let playlist_option = app.snapshot()["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|playlist| playlist["type"] != "folder")
        .position(|playlist| playlist["title"] == "Sunday")
        .unwrap()
        + 1;
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut visual = app.visual(cx);
    visual.update(|window, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            "First line\r\nSecond line\r\n\r\nThird line".into(),
        ));
        assert!(commands.run("import-clipboard", false, window, cx));
    });
    visual.run_until_parked();
    click(&mut visual, "select-clipboard-splitMode-trigger");
    click(&mut visual, "select-option-1");
    click(&mut visual, "select-clipboard-size-trigger");
    click(&mut visual, "select-option-3");
    click(&mut visual, "select-clipboard-playlistId-trigger");
    click(&mut visual, &format!("select-option-{playlist_option}"));
    click(&mut visual, "button-clipboard-edit");
    assert!(visual.debug_bounds("clipboard-slide-text-2").is_some());

    click(&mut visual, "form-input-clipboard-title");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("From clipboard");
    click(&mut visual, "form-input-clipboard-artist");
    visual.simulate_input("Test artist");
    click(&mut visual, "clipboard-slide-text-0");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("Café suite");
    visual.simulate_keystrokes("left left left left left");
    click(&mut visual, "button-clipboard-split");
    click(&mut visual, "button-clipboard-import");
    app.wait(cx, |app, _| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["title"] == "From clipboard")
    });
    let snapshot = app.snapshot();
    let item = snapshot["library"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "From clipboard")
        .unwrap();
    assert_eq!(item["size"], json!({"width":1024,"height":768}));
    assert_eq!(item["ccli"]["artistCredits"], "Test artist");
    assert_eq!(item["ccli"]["display"], false);
    let slides = item["slides"].as_array().unwrap();
    let contents: Vec<_> = slides
        .iter()
        .map(|slide| slide["content"].as_str().unwrap())
        .collect();
    assert_eq!(contents, ["Café ", "suite", "Second line", "Third line"]);
    let playlist = snapshot["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|playlist| playlist["title"] == "Sunday")
        .unwrap();
    assert!(playlist["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["itemId"] == item["id"]));

    let path = json!(app.workspace.0.join("workspace.json"));
    let loaded = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap();
    let rejected = app.host.invoke(
        "mutatePresentationLibrary",
        vec![
            path,
            json!({"type":"createPresentation","title":"Invalid clipboard slides",
                "libraryId":snapshot["libraries"][0]["id"], "content":"",
                "splitMode":"paragraph","splitMarker":"","splitUnits":1,
                "slideContents":[99]
            }),
            loaded["revision"].clone(),
        ],
    );
    assert!(rejected.unwrap_err().contains("slide content must be text"));
    assert_eq!(app.snapshot()["library"], snapshot["library"]);

    visual.update(|window, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            "One\nTwo\n\nThree\n\nFour".into(),
        ));
        assert!(commands.run("import-clipboard", false, window, cx));
    });
    visual.run_until_parked();
    click(&mut visual, "form-input-clipboard-splitUnits");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("2");
    visual.run_until_parked();
    click(&mut visual, "button-clipboard-import");
    app.wait(cx, |app, _| {
        app.snapshot()["library"]
            .as_array()
            .is_some_and(|items| items.len() == 3)
    });
    let snapshot = app.snapshot();
    let direct = snapshot["library"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] != "From clipboard" && item["title"] != "Sunday service")
        .unwrap();
    let contents: Vec<_> = direct["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slide| slide["content"].as_str().unwrap())
        .collect();
    assert_eq!(contents, ["One\nTwo\nThree", "Four"]);
}

#[gpui::test]
fn collection_tree_reorders_stay_visible_between_saves(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, |host| {
        seed(host);
        let path =
            host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
        let mut revision = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap()["revision"]
            .clone();
        for method in ["mutatePresentationLibrary", "mutatePresentationPlaylist"] {
            for title in ["Alpha", "Bravo", "Charlie"] {
                revision = host
                    .invoke(
                        method,
                        vec![
                            path.clone(),
                            json!({"type":"create","baseTitle":title}),
                            revision,
                        ],
                    )
                    .unwrap()["revision"]
                    .clone();
            }
        }
    });
    let runtime = app.runtime(cx);
    let mut visual = app.visual(cx);
    for (field, prefix) in [("libraries", "library"), ("playlist", "playlist")] {
        let before = app.snapshot();
        let originals = before[field].as_array().unwrap();
        let base = originals.len() - 3;
        let [a, b, c] =
            std::array::from_fn(|offset| originals[base + offset]["id"].as_str().unwrap());
        let [row_a, row_b, row_c] = [a, b, c]
            .map(|id| Box::leak(format!("{prefix}-{id}").into_boxed_str()) as &'static str);
        let row_top =
            |visual: &mut gpui::VisualTestContext, id| visual.debug_bounds(id).unwrap().top();
        let first = support::hold_workspace_writes(&app, cx);
        support::drag_row(
            &mut visual,
            row_c,
            row_a,
            false,
            "collection-tree-insertion-line",
        );
        assert!(row_top(&mut visual, row_c) < row_top(&mut visual, row_a));
        let second = support::hold_workspace_writes(&app, cx);
        support::drag_row(
            &mut visual,
            row_b,
            row_c,
            false,
            "collection-tree-insertion-line",
        );
        assert!(row_top(&mut visual, row_b) < row_top(&mut visual, row_c));
        assert_eq!(app.snapshot()[field][base]["id"], a);
        first.send(()).unwrap();
        app.wait(cx, |_, cx| {
            cx.update(|cx| runtime.content.read(cx).workspace[field][base]["id"] == c)
        });
        assert!(row_top(&mut visual, row_b) < row_top(&mut visual, row_c));
        assert!(row_top(&mut visual, row_c) < row_top(&mut visual, row_a));
        second.send(()).unwrap();
        app.wait(cx, |app, _| {
            let saved = app.snapshot();
            saved[field][base]["id"] == b
                && saved[field][base + 1]["id"] == c
                && saved[field][base + 2]["id"] == a
        });
    }
}

#[gpui::test]
fn library_authoring_saves_and_reopens_the_workspace(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, seed);
    let mut visual = app.visual(cx);
    context(&mut visual, "document-0");
    click(&mut visual, "menu-item-Rename");
    visual.simulate_input("Opening worship");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, _| {
        app.snapshot()["library"][0]["title"] == "Opening worship"
    });
    context(&mut visual, "document-0");
    click(&mut visual, "menu-item-Duplicate");
    app.wait(cx, |app, _| {
        app.snapshot()["library"].as_array().unwrap().len() == 2
    });
    let saved = app.snapshot();
    assert_eq!(saved["library"][0]["slides"].as_array().unwrap().len(), 2);
    assert_eq!(
        saved["playlist"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["title"] == "Sunday")
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    app.restart(cx);
    assert_eq!(app.snapshot()["library"], saved["library"]);
    let runtime = app.runtime(cx);
    assert!(cx.update(|cx| runtime
        .content
        .read(cx)
        .items
        .iter()
        .any(|item| item.title == "Opening worship")));
    assert!(app.visual(cx).debug_bounds("document-1").is_some());
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut visual = app.visual(cx);
    click(&mut visual, "document-0");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-0").is_some()
    });
    click(&mut visual, "cue-thumbnail-0");
    let first = visual.debug_bounds("cue-thumbnail-0").unwrap().center();
    let next = visual.debug_bounds("cue-thumbnail-1").unwrap();
    let after = gpui::point(next.right() - gpui::px(3.), next.bottom() - gpui::px(3.));
    visual.simulate_mouse_down(first, gpui::MouseButton::Left, Default::default());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    visual.run_until_parked();
    assert!(visual.debug_bounds("cue-reorder-marker").is_none());
    visual.simulate_mouse_move(after, Some(gpui::MouseButton::Left), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("cue-reorder-marker").is_some());
    visual.simulate_mouse_move(first, Some(gpui::MouseButton::Left), Default::default());
    visual.simulate_mouse_up(first, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("cue-reorder-marker").is_none());
    let item_id = cx.update(|cx| runtime.ui.read(cx).selected_library_item_id.clone());
    let original = app.snapshot()["library"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == item_id)
        .unwrap()["slides"]
        .clone();
    let release = support::hold_workspace_writes(&app, cx);
    for index in 2..4 {
        click(&mut visual, "button-cue-new-slide");
        let tile = Box::leak(format!("cue-thumbnail-{index}").into_boxed_str());
        app.wait(cx, |app, cx| app.visual(cx).debug_bounds(tile).is_some());
    }
    let preview = cx
        .update(|cx| runtime.content.read(cx).items.get(&item_id).unwrap().value["slides"].clone());
    assert_eq!(preview[2]["content"], "");
    assert_eq!(preview[3]["content"], "");
    assert_ne!(preview[2]["id"], preview[3]["id"]);
    assert_eq!(
        preview[3]["presentationThemeId"],
        original[1]["presentationThemeId"]
    );
    assert_eq!(
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == item_id)
            .unwrap()["slides"],
        original
    );
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["id"] == item_id
                    && item["slides"][2]["id"] == preview[2]["id"]
                    && item["slides"][3]["id"] == preview[3]["id"]
            })
    });
    click(&mut visual, "cue-thumbnail-0");
    visual.update(|window, cx| assert!(commands.run("open-slide-canvas", false, window, cx)));
    visual.run_until_parked();
    let assert_background_row = |visual: &mut gpui::VisualTestContext| {
        let toggle = visual.debug_bounds("canvas-background-enabled").unwrap();
        let color = visual.debug_bounds("canvas-background-color").unwrap();
        assert_eq!(toggle.center().y, color.center().y);
        assert!(toggle.right() <= color.left());
    };
    assert_background_row(&mut visual);
    let tabs = visual.debug_bounds("canvas-inspector-tabs").unwrap();
    visual.simulate_click(
        gpui::point(tabs.left() + tabs.size.width * 0.16, tabs.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert_background_row(&mut visual);
    let axes = [
        "canvas-readout-X",
        "canvas-readout-Y",
        "canvas-readout-W",
        "canvas-readout-H",
    ];
    let readout_slots =
        |visual: &mut gpui::VisualTestContext| axes.map(|axis| visual.debug_bounds(axis).unwrap());
    let slots = readout_slots(&mut visual);
    let cursor = visual.debug_bounds("canvas-viewport").unwrap();
    visual.simulate_mouse_move(slots[0].center(), None, Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-readout-X-empty").is_some());
    assert!(visual.debug_bounds("canvas-readout-Y-empty").is_some());
    assert!(visual.debug_bounds("canvas-readout-W-empty").is_some());
    assert!(visual.debug_bounds("canvas-readout-H-empty").is_some());
    visual.simulate_mouse_move(cursor.center(), None, Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-readout-X-number").is_some());
    assert!(visual.debug_bounds("canvas-readout-Y-number").is_some());
    let left = gpui::point(cursor.left() + gpui::px(30.), cursor.center().y);
    visual.simulate_mouse_move(left, None, Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-readout-X-negative").is_some());
    assert_eq!(readout_slots(&mut visual), slots);
    click(&mut visual, "button-theme-shape-tool");
    click(&mut visual, "canvas-shape-rectangle");
    visual.simulate_mouse_move(slots[0].center(), None, Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-readout-X-empty").is_some());
    assert!(visual.debug_bounds("canvas-readout-Y-empty").is_some());
    assert!(visual.debug_bounds("canvas-readout-W-number").is_some());
    assert!(visual.debug_bounds("canvas-readout-H-number").is_some());
    assert_eq!(readout_slots(&mut visual), slots);
    let x = visual.debug_bounds("object-field-x").unwrap();
    let y = visual.debug_bounds("object-field-y").unwrap();
    assert_eq!(x.top(), y.top());
    assert!(x.right() <= y.left());
    click(&mut visual, "button-object-align-left");
    click(&mut visual, "button-object-flipX");
    click(&mut visual, "button-object-flipY");
    click(&mut visual, "button-object-flipY");
    let dial = visual.debug_bounds("object-rotation-dial").unwrap();
    let top = gpui::point(dial.center().x, dial.top() + gpui::px(3.));
    let right = gpui::point(dial.right() + gpui::px(2.), dial.center().y);
    visual.simulate_mouse_down(top, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
    visual.simulate_mouse_move(right, Some(gpui::MouseButton::Left), Default::default());
    visual.simulate_mouse_up(right, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
    click(&mut visual, "object-field-strokeStyle");
    click(&mut visual, "select-option-2");
    click(&mut visual, "object-field-strokeEnabled");
    assert!(visual.debug_bounds("object-field-strokeWidth").is_none());
    click(&mut visual, "object-field-shapeShadowEnabled");
    click(&mut visual, "object-field-shapeFeatheringEnabled");
    assert!(visual
        .debug_bounds("object-field-featheringStyle")
        .is_none());
    let feather = visual
        .debug_bounds("object-featheringAmount-control")
        .unwrap();
    visual.simulate_click(feather.center(), Default::default());
    visual.run_until_parked();
    let opacity = visual.debug_bounds("object-opacity-control").unwrap();
    visual.simulate_click(opacity.center(), Default::default());
    visual.run_until_parked();
    let tabs = visual.debug_bounds("canvas-inspector-tabs").unwrap();
    visual.simulate_click(
        gpui::point(tabs.left() + tabs.size.width * 0.5, tabs.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    let mode = visual.debug_bounds("object-field-textFillMode").unwrap();
    let color = visual.debug_bounds("object-field-textColor").unwrap();
    assert_eq!(mode.center().y, color.center().y);
    assert!(mode.right() <= color.left());
    click(&mut visual, "button-object-format-bold");
    visual.simulate_click(
        gpui::point(tabs.left() + tabs.size.width * 0.83, tabs.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    click(&mut visual, "button-build-add");
    click(&mut visual, "button-buildOut-add");
    assert!(visual.debug_bounds("object-field-buildStep").is_some());
    assert!(visual.debug_bounds("object-field-buildOutStep").is_some());
    visual.update(|window, cx| assert!(commands.run("close-canvas", false, window, cx)));
    visual.run_until_parked();
    app.wait(cx, |app, _| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|item| item["slides"].as_array().unwrap())
            .any(|slide| {
                slide["layers"]
                    .as_array()
                    .is_some_and(|layers| layers.iter().any(|layer| layer["shape"] == "rectangle"))
            })
    });
    let snapshot = app.snapshot();
    let shape = snapshot["library"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|item| item["slides"].as_array().unwrap())
        .flat_map(|slide| slide["layers"].as_array().into_iter().flatten())
        .find(|layer| layer["shape"] == "rectangle")
        .unwrap();
    assert_eq!(shape["bounds"]["x"].as_f64(), Some(0.), "{shape}");
    assert_eq!(shape["rotation"].as_f64(), Some(90.), "{shape}");
    assert_eq!(shape["flipX"], true, "{shape}");
    assert_eq!(shape["flipY"], false, "{shape}");
    let palette = orbit_ui::controls::STANDARD_COLORS;
    let transformed = json!({
        "width":100, "height":100, "backgroundEnabled":false,
        "layers":[{
            "id":"test", "type":"Shape", "shape":"rectangle",
            "bounds":{"x":20,"y":35,"width":60,"height":30},
            "fillType":"gradient","fillColors":[palette[3],palette[5]],"fillAngle":0,
            "rotation":shape["rotation"], "flipX":shape["flipX"]
        }]
    });
    let still = |slide: &Value| {
        orbit_host::native_compositor::render_slide_still_over(
            slide,
            &json!({"backgroundCss":"transparent"}),
            100,
            100,
        )
        .unwrap()
    };
    let rotated = still(&transformed);
    let mut unrotated = transformed.clone();
    unrotated["layers"][0]["rotation"] = json!(0);
    let source = still(&unrotated);
    let pixel = |frame: &orbit_host::native_compositor::NativeFrame, x: usize, y: usize| {
        let offset = (y * frame.width + x) * 4;
        <[u8; 4]>::try_from(&frame.pixels[offset..offset + 4]).unwrap()
    };
    assert_eq!(pixel(&rotated, 50, 25), pixel(&source, 25, 49));
    assert_eq!(pixel(&rotated, 25, 50)[3], 0);
    unrotated["layers"][0]["flipX"] = json!(false);
    let original = still(&unrotated);
    assert_eq!(pixel(&source, 25, 49), pixel(&original, 74, 49));
    assert_ne!(pixel(&original, 25, 49), pixel(&original, 74, 49));
    unrotated["layers"][0]["rotation"] = json!(90);
    unrotated["layers"][0]["bounds"] = json!({"x":-10,"y":30,"width":40,"height":40});
    assert_eq!(pixel(&still(&unrotated), 9, 34)[3], 255);
    assert_eq!(shape["strokeEnabled"], false, "{shape}");
    assert_eq!(shape["strokeStyle"], "short", "{shape}");
    assert_eq!(shape["shadowOpacity"].as_f64(), Some(75.), "{shape}");
    assert_eq!(shape["featheringEnabled"], true, "{shape}");
    assert!(
        shape["featheringAmount"]
            .as_f64()
            .is_some_and(|amount| amount > 10. && amount < 90.),
        "{shape}"
    );
    assert_eq!(shape["textStyle"]["bold"], false, "{shape}");
    assert_eq!(shape["build"]["step"], 1, "{shape}");
    assert_eq!(shape["buildOut"]["step"], 2, "{shape}");
    assert!(
        shape["opacity"]
            .as_f64()
            .is_some_and(|opacity| opacity > 10. && opacity < 90.),
        "{shape}"
    );
    let edited = app.snapshot()["library"].clone();
    app.restart(cx);
    assert_eq!(app.snapshot()["library"], edited);
    let path = json!(app.workspace.0.join("workspace.json"));
    let snapshot = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap();
    let item = snapshot["workspace"]["library"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["slides"].as_array().unwrap().iter().any(|slide| {
                slide["layers"]
                    .as_array()
                    .is_some_and(|layers| layers.iter().any(|layer| layer["shape"] == "rectangle"))
            })
        })
        .unwrap();
    let mut slides = item["slides"].clone();
    let slide = slides
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|slide| {
            slide["layers"]
                .as_array()
                .is_some_and(|layers| layers.iter().any(|layer| layer["shape"] == "rectangle"))
        })
        .unwrap();
    let prototype = slide["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|layer| layer["shape"] == "rectangle")
        .unwrap()
        .clone();
    for (style, amount, enabled) in [
        ("inside", 17.5, true),
        ("center", 33.25, false),
        ("outside", 68.125, true),
    ] {
        let mut layer = prototype.clone();
        layer["id"] = json!(format!("feather-{style}"));
        layer["name"] = json!(format!("Feather {style}"));
        layer["featheringStyle"] = json!(style);
        layer["featheringAmount"] = json!(amount);
        layer["featheringEnabled"] = json!(enabled);
        slide["layers"].as_array_mut().unwrap().push(layer);
    }
    app.host.invoke("mutatePresentationLibrary",vec![path,json!({"type":"mutateItem","itemId":item["id"],"operation":{"type":"setSlideDocument","title":item["title"],"slides":slides}}),snapshot["revision"].clone()]).unwrap();
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
        &json!({"libraries":libraries,"categories":["themes","media","playlists"]}),
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
    let imported_layers: Vec<_> = imported["library"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|item| item["slides"].as_array().unwrap())
        .flat_map(|slide| {
            let composed = orbit_host::authoring::compose(
                slide,
                orbit_host::authoring::slide_theme(&imported, slide),
            )
            .unwrap();
            composed["layers"].as_array().cloned().unwrap_or_default()
        })
        .collect();
    for (style, amount, enabled) in [
        ("inside", 17.5, true),
        ("center", 33.25, false),
        ("outside", 68.125, true),
    ] {
        let layer = imported_layers
            .iter()
            .find(|layer| layer["name"] == format!("Feather {style}"))
            .unwrap_or_else(|| panic!("Imported {style} layer missing: {imported_layers:?}"));
        assert_eq!(layer["rotation"].as_f64(), Some(90.), "{layer}");
        assert_eq!(layer["flipX"], true, "{layer}");
        assert_eq!(layer["featheringStyle"], style, "{layer}");
        assert_eq!(layer["featheringEnabled"], enabled, "{layer}");
        assert!(
            (layer["featheringAmount"].as_f64().unwrap() - amount).abs() < 0.000001,
            "{layer}"
        );
    }

    app.restart(cx);
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut visual = app.visual(cx);
    visual.update(|window, cx| assert!(commands.run("open-stage-canvas", false, window, cx)));
    visual.run_until_parked();
    let before = app.snapshot();
    let layouts = before["stageLayouts"].as_array().unwrap();
    let tile = |layout: &Value| -> &'static str {
        Box::leak(format!("authoring-stage-{}", layout["id"].as_str().unwrap()).into_boxed_str())
    };
    assert!(visual.debug_bounds(tile(&layouts[0])).is_some());
    assert!(visual.debug_bounds(tile(&layouts[1])).is_some());
    assert!(visual.debug_bounds("canvas-document-picker").is_none());
    click(&mut visual, "button-new-editor-document");
    app.wait(cx, |app, _| {
        app.snapshot()["stageLayouts"].as_array().unwrap().len() == layouts.len() + 1
    });
    let created = app.snapshot()["stageLayouts"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds(tile(&created)).is_some()
    });
    assert_eq!(app.snapshot()["themes"], before["themes"]);
    assert_background_row(&mut visual);
    click(&mut visual, "canvas-background-enabled");
    assert_background_row(&mut visual);
    assert!(visual
        .debug_bounds("media-transparency-stage-canvas")
        .is_some());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(700));
    app.wait(cx, |app, _| {
        app.snapshot()["stageLayouts"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["slide"]["backgroundEnabled"]
            == false
    });
    click(&mut visual, tile(&layouts[0]));
    assert_background_row(&mut visual);
    click(&mut visual, "button-theme-text-tool");
    let tabs = visual.debug_bounds("canvas-inspector-tabs").unwrap();
    visual.simulate_click(
        gpui::point(tabs.left() + tabs.size.width * 0.80, tabs.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("object-field-fontSize").is_some());
    assert!(visual.debug_bounds("button-build-add").is_none());
    let mut compositor = orbit_host::native_compositor::NativeCompositor::new_static_preview();
    let mut stage = json!({
        "width":640,"height":360,"backgroundEnabled":false,
        "content":"Unrelated presentation text",
        "layers":[{"id":"current","type":"Text","bounds":{"x":0,"y":0,"width":640,"height":180},
            "textStyle":{"fontSize":30,"linkedTextEnabled":true,"linkedTextSource":"Current Slide"}}]
    });
    let mut painted_pixels = |stage: &Value| {
        let (frames, _) = compositor
            .render_canvas(
                stage,
                None,
                640,
                360,
                None,
                orbit_host::native_compositor::CanvasRenderKind::Stage,
            )
            .unwrap();
        assert_eq!(frames[0].pixels[3], 0);
        frames[0]
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] > 0)
            .count()
    };
    let small = painted_pixels(&stage);
    stage["layers"][0]["textStyle"]["fontSize"] = json!(90);
    assert!(small > 0);
    assert!(painted_pixels(&stage) > small);

    visual.update(|window, cx| assert!(commands.run("open-mask-canvas", false, window, cx)));
    visual.run_until_parked();
    let before = app.snapshot();
    let masks = before["masks"].as_array().unwrap();
    assert_eq!(masks.len(), 1);
    assert_eq!(masks[0]["slide"]["layers"], json!([]));
    assert_eq!(masks[0]["slide"]["backgroundEnabled"], false);
    let default_tile: &'static str =
        Box::leak(format!("authoring-mask-{}", masks[0]["id"].as_str().unwrap()).into_boxed_str());
    assert!(visual.debug_bounds(default_tile).is_some());
    assert!(visual.debug_bounds("canvas-document-picker").is_none());
    assert!(visual
        .debug_bounds("media-transparency-mask-canvas")
        .is_some());
    click(&mut visual, "button-canvas-checkerboard");
    assert!(visual
        .debug_bounds("media-transparency-mask-canvas")
        .is_none());
    click(&mut visual, "button-canvas-checkerboard");
    assert!(visual
        .debug_bounds("media-transparency-mask-canvas")
        .is_some());

    click(&mut visual, "button-canvas-guide-options");
    assert!(visual.debug_bounds("canvas-guides-overlay").is_none());
    let guide_panel = visual.debug_bounds("canvas-guide-fields").unwrap();
    for key in ["slideGuides", "themeGuides", "alignmentGuides"] {
        let swatch = Box::leak(format!("canvas-swatch-canvas.{key}").into_boxed_str());
        assert!(visual.debug_bounds(swatch).unwrap().size.width < guide_panel.size.width / 3.);
    }
    let guide_swatch = "canvas-swatch-canvas.slideGuides";
    for _ in 0..2 {
        click(&mut visual, guide_swatch);
        let trigger = visual.debug_bounds(guide_swatch).unwrap();
        let palette = visual.debug_bounds("color-popover").unwrap();
        assert!(palette.bottom() <= trigger.top());
        click(&mut visual, "color-swatch-#ffff00");
        assert!(visual.debug_bounds("color-popover").is_none());
        assert!(visual.debug_bounds("canvas-guide-fields").is_some());
    }
    let guide = visual
        .debug_bounds("checkbox-canvas.showSlideGuides")
        .unwrap();
    visual.simulate_click(
        gpui::point(guide.left() + gpui::px(8.), guide.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-guides-overlay").is_none());
    visual.simulate_click(
        gpui::point(guide.left() + gpui::px(8.), guide.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-guides-overlay").is_none());
    click(&mut visual, "button-canvas-guide-options");

    click(&mut visual, "button-canvas-grid-options");
    let grid = visual.debug_bounds("canvas-grid-enable-row").unwrap();
    visual.simulate_click(
        gpui::point(grid.right() - gpui::px(76.), grid.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-grid-overlay").is_some());
    visual.simulate_click(
        gpui::point(grid.right() - gpui::px(76.), grid.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-grid-overlay").is_none());
    click(&mut visual, "button-canvas-grid-options");

    assert!(visual.debug_bounds("canvas-zoom-fit").is_some());
    click(&mut visual, "button-canvas-zoom");
    assert!(visual.debug_bounds("menu-item-25%").is_some());
    click(&mut visual, "menu-item-Custom…");
    assert!(visual.debug_bounds("canvas-inline-zoom").is_some());
    visual.simulate_input("72.5");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-inline-zoom").is_none());
    assert!(visual.debug_bounds("canvas-zoom-percent").is_some());
    click(&mut visual, "button-canvas-zoom");
    click(&mut visual, "menu-item-Fit");
    assert!(visual.debug_bounds("canvas-zoom-fit").is_some());
    assert_eq!(app.snapshot()["masks"], before["masks"]);
    for tool in [
        "theme-text-tool",
        "theme-shape-tool",
        "theme-pen",
        "theme-new-image",
    ] {
        assert!(visual
            .debug_bounds(Box::leak(format!("button-{tool}").into_boxed_str()))
            .is_some());
    }
    for tool in [
        "theme-new-web",
        "theme-new-ndi",
        "theme-camera",
        "theme-style-copy",
        "theme-style-paste",
    ] {
        assert!(visual
            .debug_bounds(Box::leak(format!("button-{tool}").into_boxed_str()))
            .is_none());
    }
    click(&mut visual, "button-new-editor-document");
    app.wait(cx, |app, _| {
        app.snapshot()["masks"].as_array().unwrap().len() == 2
    });
    let new_mask = app.snapshot()["masks"][1].clone();
    let new_tile: &'static str =
        Box::leak(format!("authoring-mask-{}", new_mask["id"].as_str().unwrap()).into_boxed_str());
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds(new_tile).is_some()
    });
    assert_eq!(app.snapshot()["themes"], before["themes"]);
    click(&mut visual, default_tile);
    click(&mut visual, "button-theme-shape-tool");
    click(&mut visual, "canvas-shape-rectangle");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(350));
    app.wait(cx, |app, _| {
        app.snapshot()["masks"][0]["slide"]["layers"]
            .as_array()
            .unwrap()
            .len()
            == 1
    });
    let previous_x = app.snapshot()["masks"][0]["slide"]["layers"][0]["bounds"]["x"].clone();
    let layer = visual.debug_bounds("canvas-selected-bounds").unwrap();
    let slide = visual.debug_bounds("canvas-slide-bounds").unwrap();
    visual.simulate_mouse_down(layer.center(), gpui::MouseButton::Left, Default::default());
    visual.simulate_mouse_move(
        gpui::point(
            slide.left() + slide.size.width * 0.32,
            slide.top() + slide.size.height * 0.36,
        ),
        Some(gpui::MouseButton::Left),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-guides-overlay").is_none());
    visual.simulate_mouse_move(
        slide.center(),
        Some(gpui::MouseButton::Left),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-guides-overlay").is_some());
    let edge = gpui::point(
        slide.right() - layer.size.width / 2.,
        slide.top() + slide.size.height * 0.36,
    );
    visual.simulate_mouse_move(edge, Some(gpui::MouseButton::Left), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-guides-overlay").is_some());
    visual.simulate_mouse_up(edge, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-guides-overlay").is_none());
    app.wait(cx, |app, _| {
        app.snapshot()["masks"][0]["slide"]["layers"][0]["bounds"]["x"] != previous_x
    });
    let saved = app.snapshot();
    let render_mask = |index: usize| {
        orbit_host::native_compositor::render_mask_still(&saved["masks"][index]["slide"], 320, 180)
            .unwrap()
    };
    assert!(render_mask(1).pixels.iter().all(|channel| *channel == 0));
    let shape = render_mask(0);
    assert_eq!(shape.pixels[3], 0);
    assert!(shape
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[3] > 0));

    let from = visual.debug_bounds(new_tile).unwrap().center();
    let top = visual.debug_bounds(default_tile).unwrap();
    let target = gpui::point(top.center().x, top.top() + gpui::px(4.));
    let release = support::hold_workspace_writes(&app, cx);
    visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-reorder-marker").is_none());
    visual.simulate_mouse_move(target, Some(gpui::MouseButton::Left), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("canvas-reorder-marker").is_some());
    visual.simulate_mouse_up(target, gpui::MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert!(
        visual.debug_bounds(new_tile).unwrap().top()
            < visual.debug_bounds(default_tile).unwrap().top()
    );
    assert_eq!(app.snapshot()["masks"][1]["id"], new_mask["id"]);
    release.send(()).unwrap();
    let runtime = app.runtime(cx);
    let mask_state = |cx: &mut TestAppContext| {
        cx.update(|cx| runtime.content.read(cx).workspace["masks"].clone())
    };
    app.wait(cx, |_, cx| mask_state(cx)[0]["id"] == new_mask["id"]);
    assert_eq!(app.snapshot()["masks"][0]["id"], new_mask["id"]);

    context(&mut visual, default_tile);
    click(&mut visual, "menu-item-Rename");
    visual.simulate_input("Main mask");
    click(&mut visual, "button-theme-dialog-submit");
    app.wait(cx, |_, cx| mask_state(cx)[1]["title"] == "Main mask");
    context(&mut visual, default_tile);
    click(&mut visual, "menu-item-Copy");
    context(&mut visual, new_tile);
    click(&mut visual, "menu-item-Paste");
    app.wait(cx, |_, cx| mask_state(cx).as_array().unwrap().len() == 3);
    let pasted = app.snapshot()["masks"][1].clone();
    assert_eq!(
        pasted["slide"]["layers"],
        saved["masks"][0]["slide"]["layers"]
    );
    assert_ne!(pasted["slide"]["id"], saved["masks"][0]["slide"]["id"]);
    context(&mut visual, new_tile);
    click(&mut visual, "menu-item-Duplicate");
    app.wait(cx, |_, cx| mask_state(cx).as_array().unwrap().len() == 4);
    let duplicate = app.snapshot()["masks"][1].clone();
    assert_eq!(duplicate["slide"]["layers"], json!([]));
    let duplicate_tile = format!("authoring-mask-{}", duplicate["id"].as_str().unwrap());
    context(&mut visual, &duplicate_tile);
    click(&mut visual, "menu-item-Delete");
    app.wait(cx, |_, cx| mask_state(cx).as_array().unwrap().len() == 3);
    assert_eq!(app.snapshot()["masks"][1]["id"], pasted["id"]);

    click(&mut visual, default_tile);
    click(&mut visual, "button-theme-text-tool");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(350));
    app.wait(cx, |_, cx| {
        mask_state(cx)[2]["slide"]["layers"]
            .as_array()
            .unwrap()
            .len()
            == 2
    });
    let original_bottom = mask_state(cx)[2]["slide"]["layers"][0]["id"].clone();
    context(&mut visual, "canvas-object-1");
    for item in ["Rename", "Hide", "Lock", "Delete"] {
        assert!(visual
            .debug_bounds(Box::leak(format!("menu-item-{item}").into_boxed_str()))
            .is_some());
    }
    assert!(visual.debug_bounds("menu-item-Move up").is_none());
    assert!(visual.debug_bounds("menu-item-Move down").is_none());
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();

    let source = visual.debug_bounds("canvas-object-1").unwrap().center();
    let bottom = visual.debug_bounds("canvas-object-0").unwrap();
    let target = gpui::point(bottom.center().x, bottom.bottom() - gpui::px(2.));
    visual.simulate_mouse_down(source, gpui::MouseButton::Left, Default::default());
    visual.simulate_mouse_move(target, Some(gpui::MouseButton::Left), Default::default());
    visual.simulate_mouse_up(target, gpui::MouseButton::Left, Default::default());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(350));
    app.wait(cx, |app, _| {
        app.snapshot()["masks"][2]["slide"]["layers"][1]["id"] == original_bottom
    });
}

#[gpui::test]
fn presentation_take_next_and_clear_reach_a_real_output_process(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, true, |host| {
        seed(host);
        let path =
            host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let mut invalid_template = snapshot["workspace"]["ccliTemplate"].clone();
        assert_eq!(invalid_template["layers"].as_array().unwrap().len(), 1);
        invalid_template["layers"][0]["textStyle"]["linkedTextEnabled"] = json!(false);
        assert!(host
            .invoke(
                "saveWorkspaceContent",
                vec![
                    path.clone(),
                    json!({"ccliTemplate": invalid_template}),
                    snapshot["revision"].clone()
                ]
            )
            .unwrap_err()
            .contains("must remain linked"));
        let legacy = host
            .invoke(
                "saveWorkspaceContent",
                vec![
                    path.clone(),
                    json!({"scheduledEvents":[{"id":"retired-calendar-event"}]}),
                    snapshot["revision"].clone(),
                ],
            )
            .unwrap();
        host.invoke("mutatePresentationLibrary", vec![path, json!({
            "type": "mutateItem", "itemId": snapshot["workspace"]["library"][0]["id"],
            "operation": {"type": "setPresentationSetting", "field": "title", "value": "Sunday worship"}
        }), legacy["revision"].clone()]).unwrap();
    });
    assert_eq!(
        app.snapshot()["scheduledEvents"][0]["id"],
        "retired-calendar-event"
    );
    let runtime = app.runtime(cx);
    app.wait(cx, |app, cx| {
        app.host
            .state::<crate::output_process::OutputRoute>()
            .0
            .get()
            .unwrap()
            .attached()
            && cx.update(|cx| {
                let output = runtime.output.read(cx);
                output.snapshot.is_some() && output.committed_revision > 0
            })
    });
    let screen = cx.update(|cx| {
        crate::output::view::screens(runtime.output.read(cx).snapshot.as_ref().unwrap())[0]
            .0
            .clone()
    });
    let output = |app: &Scenario| -> Value {
        crate::state::runtime::output_call(&app.host, "getOutputContent", vec![json!(screen)])
            .unwrap()
    };
    let mut visual = app.visual(cx);
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let settings_runtime = runtime.clone();
    let settings_window = cx.add_window(move |window, cx| {
        let view = cx.new(|cx| crate::settings::SettingsView::new(settings_runtime, cx));
        orbit_ui::controls::Root::new(view, window, cx)
    });
    let mut settings = gpui::VisualTestContext::from_window(settings_window.into(), cx);
    settings.run_until_parked();
    click(&mut settings, "button-general-ccli-layout");
    visual.run_until_parked();
    assert!(visual.debug_bounds("ccli-objects").is_some());
    assert!(visual.debug_bounds("authoring-layout-scroll").is_none());
    for tool in [
        "theme-text-tool",
        "theme-shape-tool",
        "theme-pen",
        "theme-new-image",
        "theme-new-video-input",
        "theme-new-web",
    ] {
        assert!(visual
            .debug_bounds(Box::leak(format!("button-{tool}").into_boxed_str()))
            .is_some());
    }
    assert!(visual.debug_bounds("button-theme-new-ndi").is_none());
    click(&mut visual, "canvas-object-0");
    visual.simulate_keystrokes("delete");
    assert!(visual.debug_bounds("canvas-object-0").is_some());
    click(&mut visual, "button-theme-shape-tool");
    click(&mut visual, "canvas-shape-rectangle");
    visual.update(|window, cx| assert!(commands.run("close-canvas", false, window, cx)));
    app.wait(cx, |app, _| {
        app.snapshot()["ccliTemplate"]["layers"]
            .as_array()
            .is_some_and(|layers| layers.len() == 2)
    });
    let saved_template = app.snapshot()["ccliTemplate"].clone();
    assert_eq!(
        saved_template["layers"][0]["textStyle"]["linkedTextSource"],
        "CCLI"
    );
    click(&mut visual, "document-0");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-1").is_some()
    });
    click(&mut visual, "cue-thumbnail-0");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, _| {
        let scene = output(app);
        scene["text"] == "Opening line"
            && scene["ccliSlide"] == saved_template
            && scene["linkedTextContext"]["ccli"]["title"] == "Sunday worship"
    });
    let mut scene = output(&app);
    scene["ccliSlide"]["layers"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    let frame = orbit_host::native_compositor::render_scene_still(&scene, 320, 180).unwrap();
    let mut without_ccli = scene;
    without_ccli.as_object_mut().unwrap().remove("ccliSlide");
    let without_ccli =
        orbit_host::native_compositor::render_scene_still(&without_ccli, 320, 180).unwrap();
    assert_ne!(frame.pixels, without_ccli.pixels);
    visual.simulate_keystrokes("right");
    app.wait(cx, |app, _| {
        let scene = output(app);
        scene["text"] == "Closing line" && scene["ccliSlide"].is_null()
    });
    click(&mut settings, "general-ccli-license");
    settings.simulate_input("001234");
    settings.simulate_keystrokes("enter");
    click(&mut settings, "select-general-ccli-placement-trigger");
    click(&mut settings, "select-option-1");
    app.wait(cx, |app, _| {
        let scene = output(app);
        scene["ccliSlide"] == saved_template
            && scene["linkedTextContext"]["ccliLicenseNumber"] == "001234"
            && app.host.invoke("getSettings", vec![]).unwrap()["generalSettings"]
                ["ccliDisplayLocation"]
                == "last"
    });
    visual.simulate_keystrokes("left");
    app.wait(cx, |app, _| {
        let scene = output(app);
        scene["text"] == "Opening line" && scene["ccliSlide"].is_null()
    });
    for option in ["select-option-2", "select-option-3"] {
        click(&mut settings, "select-general-ccli-placement-trigger");
        click(&mut settings, option);
        app.wait(cx, |app, _| output(app)["ccliSlide"] == saved_template);
    }
    click(&mut settings, "general-ccli-enabled");
    app.wait(cx, |app, _| {
        let scene = output(app);
        scene["ccliSlide"].is_null() && scene["linkedTextContext"]["ccli"]["display"] == false
    });
    click(&mut settings, "general-ccli-enabled");
    click(&mut settings, "select-general-ccli-placement-trigger");
    click(&mut settings, "select-option-0");
    for _ in 0..32 {
        visual.simulate_keystrokes("left right");
    }
    visual.simulate_keystrokes("left");
    app.wait(cx, |app, cx| {
        output(app)["text"] == "Opening line"
            && cx.update(|cx| {
                let output = runtime.output.read(cx);
                output.error.is_none()
                    && output.device_error.is_none()
                    && output.link_error.is_none()
            })
    });
    assert!(app
        .host
        .state::<crate::output_process::OutputRoute>()
        .0
        .get()
        .unwrap()
        .attached());
    let (theme_id, theme_slide_id, item_id, slide_id) = cx.update(|cx| {
        let content = runtime.content.read(cx);
        let theme = &content.themes[0];
        (
            theme.id.clone(),
            theme.slides[0].0.clone(),
            runtime.ui.read(cx).selected_library_item_id.clone(),
            runtime.output.read(cx).live["liveSlideId"].clone(),
        )
    });
    cx.update(|cx| {
        crate::cues::actions::mutate(
            &runtime,
            &item_id,
            json!({"type":"setSlidesTheme", "slideIds":[slide_id],
            "assignment":format!("{theme_id}::{theme_slide_id}")}),
            cx,
        )
    });
    app.wait(cx, |app, _| {
        app.snapshot()["library"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == item_id)
            .unwrap()["slides"]
            .as_array()
            .unwrap()
            .iter()
            .any(|slide| {
                slide["id"] == slide_id
                    && slide["presentationThemeId"] == format!("{theme_id}::{theme_slide_id}")
            })
    });
    click(&mut visual, "cue-thumbnail-0");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, _| {
        output(app)["lookThemes"]["presentationThemeId"] == format!("{theme_id}::{theme_slide_id}")
    });
    let original_layer_count = output(&app)["presentationThemeSlide"]["layers"]
        .as_array()
        .unwrap()
        .len();
    visual.update(|window, cx| assert!(commands.run("open-theme-canvas", false, window, cx)));
    visual.run_until_parked();
    click(&mut visual, "button-theme-shape-tool");
    click(&mut visual, "canvas-shape-rectangle");
    visual.update(|window, cx| assert!(commands.run("close-canvas", false, window, cx)));
    visual.run_until_parked();
    app.wait(cx, |app, _| {
        output(app)["presentationThemeSlide"]["layers"]
            .as_array()
            .is_some_and(|layers| {
                layers.len() > original_layer_count
                    && layers.iter().any(|layer| layer["shape"] == "rectangle")
            })
    });
    let controls = visual.debug_bounds("transition-mode-slide").unwrap();
    visual.simulate_click(controls.center(), Default::default());
    visual.run_until_parked();
    app.wait(cx, |app, _| {
        output(app)["slideTransition"]["transition"]["effectId"]
            == orbit_host::native_transitions::DISSOLVE_ID
    });
    app.wait(cx, |app, _| {
        app.snapshot()["slideTransition"]["effectId"] == orbit_host::native_transitions::DISSOLVE_ID
    });
    let release = support::hold_workspace_writes(&app, cx);
    let controls = visual.debug_bounds("transition-mode-slide").unwrap();
    visual.simulate_click(
        gpui::point(controls.left() + gpui::px(12.), controls.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    app.wait(cx, |app, _| output(app)["slideTransition"].is_null());
    assert_eq!(
        app.snapshot()["slideTransition"]["effectId"],
        orbit_host::native_transitions::DISSOLVE_ID,
        "The output must cut before the workspace write is released"
    );
    release.send(()).unwrap();
    app.wait(cx, |app, _| app.snapshot()["slideTransition"].is_null());
    let item_id = cx.update(|cx| runtime.ui.read(cx).selected_library_item_id.clone());
    let slide_id = cx.update(|cx| runtime.output.read(cx).live["liveSlideId"].clone());
    cx.update(|cx| {
        crate::cues::actions::mutate(
            &runtime,
            &item_id,
            json!({"type":"setTransition", "transition":crate::transitions::fade(1.3)}),
            cx,
        )
    });
    app.wait(cx, |app, _| {
        output(app)["slideTransition"]["transition"]["durationSeconds"] == 1.3
    });
    cx.update(|cx| crate::cues::actions::mutate(&runtime, &item_id,
        json!({"type":"setTransition", "slideId":slide_id, "transition":{"effectId":orbit_host::native_transitions::CUT_ID, "durationSeconds":0}}), cx));
    app.wait(cx, |app, _| {
        output(app)["slideTransition"]["transition"]["effectId"]
            == orbit_host::native_transitions::CUT_ID
    });
    for slide in [Some(slide_id), None] {
        let mut operation = json!({"type":"setTransition", "transition":null});
        if let Some(slide) = slide {
            operation["slideId"] = slide;
        }
        cx.update(|cx| crate::cues::actions::mutate(&runtime, &item_id, operation, cx));
    }
    app.wait(cx, |app, _| output(app)["slideTransition"].is_null());
    visual.update(|window, cx| window.simulate_next_frame(cx));
    let controls = visual.debug_bounds("transition-mode-slide").unwrap();
    visual.simulate_click(controls.center(), Default::default());
    visual.run_until_parked();
    app.wait(cx, |app, _| {
        output(app)["slideTransition"]["transition"]["effectId"]
            == orbit_host::native_transitions::DISSOLVE_ID
    });
    // Interleave real native edits at the exact revision boundary, rather than depending on thread timing.
    struct ConcurrentEdits<'a> {
        host: &'a orbit_host::Host,
        workspace: String,
        calls: std::cell::Cell<usize>,
    }
    impl crate::output::protocol::Target for ConcurrentEdits<'_> {
        fn call(&self, method: &str, args: Vec<Value>) -> Result<Value, String> {
            let call = self.calls.get() + 1;
            self.calls.set(call);
            self.host.edit_operator_output(&self.workspace, |live| {
                live["activeMessageText"] = json!(format!("Concurrent edit {call}"));
                Ok(())
            })?;
            self.host.invoke(method, args)
        }
    }
    let (mut pending, workspace) = cx.update(|cx| {
        runtime.output.update(cx, |state, cx| {
            state.edit_live(|live| live["stageMessage"] = json!("Pending scene edit"));
            cx.notify();
            (
                (**state.snapshot.as_ref().unwrap()).clone(),
                runtime.content.read(cx).workspace_path.clone(),
            )
        })
    });
    pending["stageMessage"] = json!("Pending scene edit");
    let target = ConcurrentEdits {
        host: &app.host,
        workspace,
        calls: std::cell::Cell::new(0),
    };
    let delivery = crate::output::protocol::Protocol::default()
        .send(
            &target,
            crate::output::protocol::Snapshot {
                state: std::sync::Arc::new(pending),
                theme_source: std::sync::Arc::new(json!({})),
            },
        )
        .unwrap();
    let crate::output::protocol::Delivery::Contended(patch) = delivery else {
        panic!("Concurrent native edits must yield before accepting an outdated scene");
    };
    assert_eq!(patch["activeMessageText"], "Concurrent edit 3");
    assert_eq!(target.calls.get(), 3);
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            let state = runtime.output.read(cx);
            state.error.is_none()
                && state.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot["stageMessage"] == "Pending scene edit"
                        && snapshot["activeMessageText"] == "Concurrent edit 3"
                })
        })
    });
    visual.simulate_keystrokes("f2");
    app.wait(cx, |app, _| output(app)["text"] == "");
    assert!(cx.update(|cx| runtime.output.read(cx).error.is_none()));
    click(&mut visual, "cue-thumbnail-0");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, _| output(app)["text"] == "Opening line");

    let release = support::hold_workspace_writes(&app, cx);
    click(&mut visual, "button-cue-new-slide");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-2").is_some()
    });
    click(&mut visual, "cue-thumbnail-1");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, _| output(app)["text"] == "Closing line");
    visual.simulate_keystrokes("left");
    app.wait(cx, |app, _| output(app)["text"] == "Opening line");
    visual.simulate_keystrokes("right");
    app.wait(cx, |app, _| output(app)["text"] == "Closing line");
    visual.simulate_keystrokes("f2");
    app.wait(cx, |app, _| output(app)["text"] == "");
    click(&mut visual, "cue-thumbnail-0");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, _| output(app)["text"] == "Opening line");
    click(&mut visual, "cue-thumbnail-2");
    visual.simulate_keystrokes("enter f1");
    app.wait(cx, |app, _| output(app)["text"] == "");
    assert_eq!(
        app.snapshot()["library"][0]["slides"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        app.snapshot()["library"][0]["slides"]
            .as_array()
            .unwrap()
            .len()
            == 3
    });
    assert!(cx.update(|cx| runtime.output.read(cx).live["liveSlideId"].is_null()));
}

#[gpui::test]
fn media_panel_and_filters_survive_operator_navigation(cx: &mut TestAppContext) {
    use orbit_ui::{set_appearance_settings, AppearanceSettings, ThemeSetting, UiDensity};
    let mut app = Scenario::new(cx, false, |host| {
        seed(host);
        let path =
            host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let macros: Vec<_> = (0..4).map(|index| json!({
            "id": format!("row-macro-{index}"), "title": format!("Macro {index}"),
            "imageType": "ImageTypeDefault", "collectionIds": ["row-collection"], "actions": []
        })).collect();
        host.invoke(
            "saveWorkspaceContent",
            vec![
                path.clone(),
                json!({"macros": macros,
                    "macroCollections": [{"id":"row-collection", "title":"Default Collection"}]
                }),
                snapshot["revision"].clone(),
            ],
        )
        .unwrap();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let mut libraries = snapshot["workspace"]["mediaLibraries"]
            .as_array()
            .unwrap()
            .clone();
        for media_type in ["graphics", "audio"] {
            for (suffix, kind) in [
                ("playlist", "playlist"),
                ("folder", "folder"),
                ("outer", "folder"),
            ] {
                libraries.push(json!({"id":format!("{media_type}-{suffix}"),"title":format!("{media_type} {suffix}"),"type":kind,"mediaType":media_type,"parentId":"","collapsed":false,"itemFiles":[]}));
            }
        }
        host.invoke(
            "saveMediaLibraries",
            vec![path, json!(libraries), snapshot["revision"].clone()],
        )
        .unwrap();
    });
    let mut visual = app.visual(cx);
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    for (command, media_type) in [("view-media", "graphics"), ("view-audio", "audio")] {
        visual.update(|window, cx| assert!(commands.run(command, false, window, cx)));
        visual.run_until_parked();
        for (source, target) in [("playlist", "folder"), ("folder", "outer")] {
            let source_id = format!("{media_type}-{source}");
            let target_id = format!("{media_type}-{target}");
            let source_selector: &'static str =
                Box::leak(format!("media-library-row-{source_id}").into_boxed_str());
            let target_selector: &'static str =
                Box::leak(format!("media-library-row-{target_id}").into_boxed_str());
            let from = visual.debug_bounds(source_selector).unwrap().center();
            let to = visual.debug_bounds(target_selector).unwrap().center();
            visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
            visual.simulate_mouse_move(
                from + gpui::point(gpui::px(10.), gpui::px(0.)),
                Some(gpui::MouseButton::Left),
                Default::default(),
            );
            visual.simulate_mouse_move(to, Some(gpui::MouseButton::Left), Default::default());
            visual.run_until_parked();
            assert!(visual.debug_bounds("media-library-drop-inside").is_some());
            let release = support::hold_workspace_writes(&app, cx);
            visual.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
            visual.run_until_parked();
            assert!(
                visual.debug_bounds(source_selector).unwrap().top()
                    >= visual.debug_bounds(target_selector).unwrap().bottom()
            );
            assert!(!app.snapshot()["mediaLibraries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|library| library["id"] == source_id && library["parentId"] == target_id));
            release.send(()).unwrap();
            app.wait(cx, |app, _| {
                app.snapshot()["mediaLibraries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|library| library["id"] == source_id && library["parentId"] == target_id)
            });
            app.wait(cx, |app, cx| {
                app.visual(cx)
                    .debug_bounds(source_selector)
                    .is_some_and(|source| {
                        app.visual(cx)
                            .debug_bounds(target_selector)
                            .is_some_and(|target| {
                                source.left() == target.left() && source.top() >= target.bottom()
                            })
                    })
            });
        }
    }
    for (theme, density, scale) in [
        (ThemeSetting::Dark, UiDensity::Compact, 1.),
        (ThemeSetting::Light, UiDensity::Spacious, 2.),
        (ThemeSetting::Red, UiDensity::Comfortable, 1.),
    ] {
        visual.update(|window, cx| {
            set_appearance_settings(
                cx,
                AppearanceSettings {
                    theme,
                    density,
                    ..Default::default()
                },
            );
            window.set_scale_factor(scale);
        });
        visual.run_until_parked();
        let center = visual.debug_bounds("region-center").unwrap().size.height;
        let media = visual.debug_bounds("region-media").unwrap().size.height;
        visual.update(|window, cx| {
            assert!(commands.run("toggle-media", false, window, cx));
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("region-media").is_none());
        assert!(visual.debug_bounds("divider-MediaHeight").is_none());
        assert!(
            f32::from(visual.debug_bounds("region-center").unwrap().size.height - center - media)
                .abs()
                < 2.
        );
        visual.update(|window, cx| {
            assert!(commands.run("view-media", false, window, cx));
        });
        visual.run_until_parked();
        visual.simulate_input("No matching media");
        assert!(visual.debug_bounds("media-view-controls").is_some());
        support::primary_shortcut(&mut visual, "a");
        visual.simulate_keystrokes("backspace");
        assert!(visual.debug_bounds("region-media").is_some());
        visual.update(|window, cx| assert!(commands.run("view-macros", false, window, cx)));
        visual.run_until_parked();
        click(&mut visual, "button-macro-table");
        let first = visual.debug_bounds("macro-entry-row-macro-0").unwrap();
        let second = visual.debug_bounds("macro-entry-row-macro-1").unwrap();
        let icon = visual.debug_bounds("macro-icon-row-macro-0").unwrap();
        let title = visual.debug_bounds("macro-title-row-macro-0").unwrap();
        assert!(first.bottom() <= second.top());
        assert_eq!(first.left(), second.left());
        assert_eq!(first.size.width, second.size.width);
        assert!(first.size.width > icon.size.width * 3.);
        assert_eq!(icon.size.width, icon.size.height);
        assert!(icon.bottom() <= first.bottom());
        assert!(icon.right() <= title.left());
        assert!(title.right() <= first.right());
        assert!(title.top() >= first.top() && title.bottom() <= first.bottom());
        assert!(visual.debug_bounds("button-macro-footer-paste").is_none());
        click(&mut visual, "button-macro-grid");
        let first = visual.debug_bounds("macro-entry-row-macro-0").unwrap();
        let second = visual.debug_bounds("macro-entry-row-macro-1").unwrap();
        let icon = visual.debug_bounds("macro-icon-row-macro-0").unwrap();
        let title = visual.debug_bounds("macro-title-row-macro-0").unwrap();
        assert_eq!(icon.size.width, icon.size.height);
        assert!(icon.bottom() <= title.top());
        assert!(first.left() <= icon.left() && icon.right() <= first.right());
        assert!(first.top() <= icon.top() && title.bottom() <= first.bottom());
        assert!(first.left() <= title.left() && title.right() <= first.right());
        assert!(first.right() <= second.left() || first.bottom() <= second.top());
        visual.simulate_mouse_move(title.center(), None, Default::default());
        visual.run_until_parked();
        assert_eq!(visual.debug_bounds("macro-entry-row-macro-0"), Some(first));
        click(&mut visual, "button-macro-table");
    }
    context(&mut visual, "macro-entry-row-macro-0");
    click(&mut visual, "button-clip-copy-row-macro-0");
    click(&mut visual, "button-clip-paste-row-macro-0");
    app.wait(cx, |app, _| {
        app.snapshot()["macros"]
            .as_array()
            .is_some_and(|macros| macros.len() == 5)
    });
    click(&mut visual, "button-macro-create");
    visual.simulate_input("Immediate macro");
    click(&mut visual, "button-editor-save");
    app.wait(cx, |app, _| {
        app.snapshot()["macros"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["title"] == "Immediate macro")
    });
    app.pump(cx);
    assert_eq!(app.snapshot()["library"][0]["title"], "Sunday service");
    let libraries = app.snapshot()["mediaLibraries"].clone();
    app.restart(cx);
    assert_eq!(app.snapshot()["mediaLibraries"], libraries);
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    app.visual(cx)
        .update(|window, cx| assert!(commands.run("open-licenses", false, window, cx)));
    let license_window = std::cell::Cell::new(None);
    app.wait(cx, |_, cx| {
        for window in cx.update(|cx| cx.windows()) {
            let mut visual = gpui::VisualTestContext::from_window(window, cx);
            if visual.debug_bounds("licenses-filter").is_some() {
                license_window.set(Some(window));
                return true;
            }
        }
        false
    });
    let mut licenses = gpui::VisualTestContext::from_window(license_window.get().unwrap(), cx);
    let application = format!(
        "license-package-Presenter Orbit-{}",
        env!("CARGO_PKG_VERSION")
    );
    let application_selector: &'static str = Box::leak(application.clone().into_boxed_str());
    app.wait(cx, |_, cx| {
        gpui::VisualTestContext::from_window(license_window.get().unwrap(), cx)
            .debug_bounds(application_selector)
            .is_some()
    });
    click(&mut licenses, &application);
    assert!(licenses.debug_bounds("license-original-text").is_some());
    assert!(licenses.debug_bounds("license-text-unavailable").is_none());
    click(&mut licenses, "button-about-back");
    // The generated catalog is the same artifact embedded in packaged and development builds.
    let catalog: Value = serde_json::from_str(include_str!(concat!(
        env!("OUT_DIR"),
        "/dependency-notices.json"
    )))
    .unwrap();
    let package = catalog["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| {
            package["files"].as_array().unwrap().is_empty() && package["license"].as_str().is_some()
        })
        .unwrap();
    click(&mut licenses, "licenses-filter");
    licenses.simulate_input(package["name"].as_str().unwrap());
    licenses.run_until_parked();
    assert!(licenses
        .debug_bounds(Box::leak(application.into_boxed_str()))
        .is_none());
    click(
        &mut licenses,
        &format!(
            "license-package-{}-{}",
            package["name"].as_str().unwrap(),
            package["version"].as_str().unwrap()
        ),
    );
    assert!(licenses.debug_bounds("license-declaration").is_some());
    assert!(licenses.debug_bounds("license-text-unavailable").is_some());
    assert!(licenses.debug_bounds("license-original-text").is_none());
    assert!(licenses.debug_bounds("button-license-source").is_some());
    click(&mut licenses, "button-about-back");
    assert!(licenses.debug_bounds("licenses-filter").is_some());
    click(&mut licenses, "button-about-back");
    assert!(licenses
        .debug_bounds("button-about-licenses-open")
        .is_some());
}

#[gpui::test]
fn audio_inspector_trim_reopens_and_preview_jumps_respect_the_range(cx: &mut TestAppContext) {
    let files = support::Folder::new();
    let wav = files.0.join("Trimmed preview.wav");
    audio_fixture(&wav, 440.);
    let mut app = Scenario::new(cx, false, |host| {
        let path =
            host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
        let revision = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap()["revision"]
            .clone();
        let mut asset = json!({"id":"trim-audio","title":"Trim audio","type":"audio","libraryId":"default-audio","sourcePath":wav});
        asset["audioPreviewTrim"] = json!({"inSeconds":3,"outSeconds":1});
        assert!(host
            .invoke(
                "saveMediaSettings",
                vec![path.clone(), json!([asset]), revision.clone()]
            )
            .is_err());
        asset.as_object_mut().unwrap().remove("audioPreviewTrim");
        host.invoke("saveMediaSettings", vec![path, json!([asset]), revision])
            .unwrap();
    });
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let runtime = app.runtime(cx);
    let mut visual = app.visual(cx);
    visual.update(|window, cx| commands.run("view-audio", false, window, cx));
    visual.run_until_parked();

    context(&mut visual, "media-item-trim-audio");
    click(&mut visual, "menu-item-Inspector...");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("preview-timeline").is_some()
    });
    let timeline = visual.debug_bounds("preview-timeline").unwrap();
    let start = gpui::point(timeline.left() + gpui::px(1.), timeline.center().y);
    let target = gpui::point(timeline.left() + timeline.size.width * 0.25, start.y);
    visual.simulate_mouse_down(start, gpui::MouseButton::Left, Default::default());
    visual.simulate_mouse_move(target, Some(gpui::MouseButton::Left), Default::default());
    visual.simulate_mouse_up(target, gpui::MouseButton::Left, Default::default());
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"][0]["audioPreviewTrim"]["inSeconds"]
                .as_f64()
                .is_some_and(|time| (time - 2.).abs() < 0.04)
        })
    });
    click(&mut visual, "dialog-close");
    context(&mut visual, "media-item-trim-audio");
    click(&mut visual, "menu-item-Inspector...");
    click(&mut visual, "button-asset-preview-forward-15");
    let end = visual.debug_bounds("preview-playhead").unwrap();
    assert!((end.left() - timeline.right()).abs() < gpui::px(2.));
    click(&mut visual, "button-asset-preview-back-15");
    let playhead = visual.debug_bounds("preview-playhead").unwrap();
    assert!((playhead.left() - target.x).abs() < gpui::px(2.));
}

#[gpui::test]
fn media_inspector_keeps_tabs_and_close_visible_while_audio_routing_scrolls(
    cx: &mut TestAppContext,
) {
    let files = support::Folder::new();
    let wav = files.0.join("Inspector audio.wav");
    audio_fixture(&wav, 440.);
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/propresenter/orbit-int12-video.mp4");
    let video = files.0.join("Inspector video.mp4");
    let muxed = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(source)
        .arg("-i")
        .arg(wav)
        .args(["-c:v", "copy", "-c:a", "aac", "-shortest"])
        .arg(&video)
        .output()
        .unwrap();
    assert!(
        muxed.status.success(),
        "{}",
        String::from_utf8_lossy(&muxed.stderr)
    );
    let mut app = Scenario::new(cx, false, |host| {
        let path =
            host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let channels: Vec<_> = (1..=6)
            .map(|source| json!({"sourceChannel":source,"outputs":[1],"muted":false}))
            .collect();
        host.invoke(
            "saveMediaSettings",
            vec![
                path,
                json!([{
                    "id":"inspector-video","title":"Inspector video","type":"video",
                    "mediaTransport":{"inSeconds":0,"outSeconds":null,"rate":1,"delaySeconds":10,"retrigger":"automatic"},
                    "libraryId":"default-graphics","sourcePath":video,
                    "audioRouting":{"gainDb":0,"channels":channels},
                    "markers":[{"id":"inspector-marker","title":"Beat","timeSeconds":1.5,"repeatOnLoop":false,"actions":[]}]
                }]),
                snapshot["revision"].clone(),
            ],
        )
        .unwrap();
    });
    let mut visual = app.visual(cx);
    visual.simulate_resize(gpui::size(gpui::px(760.), gpui::px(500.)));
    visual.run_until_parked();
    context(&mut visual, "media-item-inspector-video");
    let menu_order = [
        "Inspector...",
        "Transitions...",
        "Behaviour",
        "Scaling",
        "Playback Behavior",
        "End Behavior",
        "Cut",
        "Copy",
        "Paste",
        "Duplicate",
        "Rename",
        "Delete",
        "Open File Location",
    ]
    .map(|label| {
        let selector = Box::leak(format!("menu-item-{label}").into_boxed_str());
        visual.debug_bounds(selector).unwrap().top()
    });
    assert!(menu_order.windows(2).all(|rows| rows[0] < rows[1]));
    click(&mut visual, "menu-item-Inspector...");

    app.wait(cx, |app, cx| {
        app.visual(cx)
            .debug_bounds("inspector-audio-meters")
            .is_some()
    });
    let tabs = visual.debug_bounds("media-inspector-tab-bar").unwrap();
    let close = visual.debug_bounds("dialog-close").unwrap();
    let viewport = visual.update(|window, _| window.viewport_size());
    assert!(close.top() > gpui::px(0.));
    assert!(tabs.top() < close.bottom() && close.top() < tabs.bottom());
    let settings = visual.debug_bounds("media-inspector-scroll").unwrap();
    let preview = visual.debug_bounds("media-inspector-preview").unwrap();
    assert!(preview.right() < settings.left());
    let metrics = visual.update(|_, cx| orbit_ui::ActiveTheme::theme(cx).metrics);
    assert!((tabs.left() - settings.left() - metrics.space_sm).abs() <= gpui::px(1.));
    let tab_width = metrics.control_height_compact;
    let tab = |visual: &mut gpui::VisualTestContext, index: f32| {
        let point = gpui::point(tabs.left() + tab_width * (index + 0.5), tabs.center().y);
        visual.simulate_click(point, Default::default());
        visual.run_until_parked();
    };
    let initial_defaults = app.snapshot()["layerTransitions"].clone();
    let catalog = app.host.invoke("getTransitionCatalog", vec![]).unwrap();
    let fade = catalog
        .as_array()
        .unwrap()
        .iter()
        .position(|effect| effect["id"] == orbit_host::native_transitions::DISSOLVE_ID)
        .unwrap();
    let selector = Box::leak(format!("transition-effect-{fade}").into_boxed_str());
    tab(&mut visual, 2.);
    click(&mut visual, "button-media-inspector-transition");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds(selector).is_some()
    });
    click(&mut visual, selector);
    let runtime = app.runtime(cx);
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            let workspace = &runtime.content.read(cx).workspace;
            workspace["media"][0]["backgroundTransition"]
                .as_str()
                .and_then(|value| {
                    orbit_host::native_transitions::media_transition(value)
                        .ok()
                        .flatten()
                })
                .is_some_and(|transition| {
                    transition["effectId"] == orbit_host::native_transitions::DISSOLVE_ID
                })
                && workspace["layerTransitions"] == initial_defaults
        })
    });
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    let slider = visual.debug_bounds("media-transition-duration").unwrap();
    let position = gpui::point(slider.left() + slider.size.width * 0.55, slider.center().y);
    visual.simulate_click(position, Default::default());
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"][0]["backgroundTransition"]
                .as_str()
                .and_then(|value| {
                    orbit_host::native_transitions::media_transition(value)
                        .ok()
                        .flatten()
                })
                .is_some_and(|transition| {
                    transition["durationSeconds"]
                        .as_f64()
                        .is_some_and(|duration| duration > 2.)
                })
        })
    });
    click(&mut visual, "button-media-transition-clear");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"][0]["backgroundTransition"] == "none"
        })
    });
    tab(&mut visual, 1.);
    assert_eq!(visual.debug_bounds("media-inspector-tab-bar"), Some(tabs));
    tab(&mut visual, 4.);
    assert!(visual.debug_bounds("button-route-add").is_none());
    let slider = visual
        .debug_bounds("routing-slider-route:0:gainDb")
        .unwrap();
    let point = gpui::point(slider.left() + slider.size.width * 0.25, slider.center().y);
    visual.simulate_mouse_down(point, gpui::MouseButton::Left, Default::default());
    visual.simulate_mouse_up(point, gpui::MouseButton::Left, Default::default());
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"][0]["audioRouting"]["channels"][0]["gainDb"]
                .as_f64()
                .is_some_and(|gain| gain < -20.)
        })
    });
    click(&mut visual, "button-route-configure");
    let scroll = visual.debug_bounds("media-inspector-scroll").unwrap();
    assert!(scroll.bottom() < viewport.height);
    assert!(visual
        .debug_bounds("button-route-add")
        .is_none_or(|button| button.center().y >= scroll.bottom()));
    visual.simulate_event(gpui::ScrollEvent {
        position: scroll.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-2000.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
        source: gpui::ScrollSource::Pointer,
    });
    visual.run_until_parked();
    let add = visual.debug_bounds("button-route-add").unwrap();
    assert!(scroll.contains(&add.center()));
    assert_eq!(visual.debug_bounds("media-inspector-tab-bar"), Some(tabs));
    assert_eq!(visual.debug_bounds("dialog-close"), Some(close));
    click(&mut visual, "button-route-add");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"][0]["audioRouting"]["channels"]
                .as_array()
                .is_some_and(|channels| channels.len() == 7)
        })
    });
    assert_eq!(
        app.snapshot()["media"][0]["audioRouting"]["channels"][6]["sourceChannel"],
        7
    );

    click(&mut visual, "dialog-close");
    context(&mut visual, "media-item-inspector-video");
    click(&mut visual, "menu-item-Inspector...");
    assert_eq!(visual.debug_bounds("media-inspector-tab-bar"), Some(tabs));
    assert!(visual
        .debug_bounds("button-media.thumbnail.capture")
        .is_some());
    app.wait(cx, |app, cx| {
        app.visual(cx)
            .debug_bounds("media-inspector-thumbnail")
            .is_some()
    });
    click(&mut visual, "inspector-action-name");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("Renamed inspector video");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"][0]["title"] == "Renamed inspector video"
        })
    });
    let timeline = visual.debug_bounds("preview-timeline").unwrap();
    let position = gpui::point(
        timeline.left() + timeline.size.width * 0.3,
        timeline.center().y,
    );
    visual.simulate_click(position, Default::default());
    visual.run_until_parked();
    app.wait(cx, |app, cx| {
        app.visual(cx)
            .debug_bounds("media-inspector-current-frame")
            .is_some()
    });
    let release_writes = support::hold_workspace_writes(&app, cx);
    click(&mut visual, "button-media.thumbnail.capture");
    assert_eq!(visual.debug_bounds("preview-timeline"), Some(timeline));
    visual.simulate_click(
        gpui::point(
            timeline.left() + timeline.size.width * 0.6,
            timeline.center().y,
        ),
        Default::default(),
    );
    visual.run_until_parked();
    app.wait(cx, |app, cx| {
        app.visual(cx)
            .debug_bounds("media-inspector-current-frame")
            .is_some()
    });
    click(&mut visual, "button-media.thumbnail.capture");
    release_writes.send(()).unwrap();
    app.wait(cx, |app, cx| {
        cx.update(|cx| {
            let workspace = &runtime.content.read(cx).workspace;
            workspace["media"][0]["thumbnailTimeSeconds"]
                .as_f64()
                .is_some_and(|seconds| (seconds - 2.4).abs() < 0.01)
                && workspace["media"][0]["thumbnailFile"].is_string()
        }) && app
            .visual(cx)
            .debug_bounds("media-inspector-thumbnail")
            .is_some()
    });
    let saved = app.snapshot();
    assert_eq!(saved["media"][0]["title"], "Renamed inspector video");
    let workspace = app.host.invoke("getSettings", vec![]).unwrap();
    let thumbnail =
        std::path::Path::new(workspace["presenterOrbitWorkspacePath"].as_str().unwrap())
            .parent()
            .unwrap()
            .join("Media")
            .join(saved["media"][0]["thumbnailFile"].as_str().unwrap());
    assert_eq!(image::image_dimensions(thumbnail).unwrap(), (960, 540));
    assert!((saved["media"][0]["thumbnailTimeSeconds"].as_f64().unwrap() - 2.4).abs() < 0.01);
    assert!(visual.debug_bounds("preview-marker-0").is_some());
    let timeline = visual.debug_bounds("preview-timeline").unwrap();
    for (edge, fraction) in [("preview-trim-start", 0.25), ("preview-trim-end", 0.75)] {
        let handle = visual.debug_bounds(edge).unwrap();
        let inset = gpui::px(1.);
        let x = handle
            .center()
            .x
            .max(timeline.left() + inset)
            .min(timeline.right() - inset);
        let start = gpui::point(x, handle.center().y);
        let end = gpui::point(timeline.left() + timeline.size.width * fraction, start.y);
        visual.simulate_mouse_down(start, gpui::MouseButton::Left, Default::default());
        visual.simulate_mouse_move(end, Some(gpui::MouseButton::Left), Default::default());
        visual.simulate_mouse_up(end, gpui::MouseButton::Left, Default::default());
        app.wait(cx, |_, cx| {
            cx.update(|cx| {
                let transport = &runtime.content.read(cx).workspace["media"][0]["mediaTransport"];
                let key = if fraction < 0.5 {
                    "inSeconds"
                } else {
                    "outSeconds"
                };
                transport[key]
                    .as_f64()
                    .is_some_and(|value| (value - f64::from(fraction) * 4.).abs() < 0.03)
            })
        });
    }
    tab(&mut visual, 1.);
    assert!(visual.debug_bounds("button-transport-save").is_none());
    let scroll = visual.debug_bounds("media-inspector-scroll").unwrap();
    visual.simulate_event(gpui::ScrollEvent {
        position: scroll.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-200.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
        source: gpui::ScrollSource::Pointer,
    });
    visual.run_until_parked();
    assert!(scroll.contains(
        &visual
            .debug_bounds("mediaTransport.delaySeconds")
            .unwrap()
            .center()
    ));
    click(&mut visual, "mediaTransport.delaySeconds");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_keystrokes("backspace");
    click(&mut visual, "preview-in-point");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            let content = runtime.content.read(cx);
            let transport = &content.workspace["media"][0]["mediaTransport"];
            transport["delaySeconds"] == 0. && transport["retrigger"] == "automatic"
        })
    });
    click(&mut visual, "preview-in-point");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("00:00:00.500");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"][0]["mediaTransport"]["inSeconds"] == 0.5
        })
    });
    let updated = visual.debug_bounds("preview-trim-start").unwrap();
    assert!(
        (updated.center().x - (timeline.left() + timeline.size.width * 0.125)).abs() < gpui::px(2.)
    );
    click(&mut visual, "button-asset-preview-start");
    let start = visual.debug_bounds("preview-playhead").unwrap();
    assert!((start.left() - (timeline.left() + timeline.size.width * 0.125)).abs() < gpui::px(2.));
    click(&mut visual, "button-asset-preview-end");
    let end = visual.debug_bounds("preview-playhead").unwrap();
    assert!((end.left() - (timeline.left() + timeline.size.width * 0.75)).abs() < gpui::px(2.));
    tab(&mut visual, 4.);
    assert_eq!(visual.debug_bounds("media-inspector-tab-bar"), Some(tabs));
    assert!(visual
        .debug_bounds("button-media.thumbnail.capture")
        .is_none());
    tab(&mut visual, 5.);
    context(&mut visual, "media-marker-0");
    click(&mut visual, "menu-item-Rename");
    click(&mut visual, "marker-0-None-title");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("Chorus");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"][0]["markers"][0]["title"] == "Chorus"
        })
    });
    context(&mut visual, "media-marker-0");
    click(&mut visual, "menu-item-Move to playhead");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"][0]["markers"][0]["timeSeconds"]
                .as_f64()
                .is_some_and(|time| (time - 3.).abs() < 0.04)
        })
    });
    context(&mut visual, "media-marker-0");
    click(&mut visual, "menu-item-Duplicate");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            let content = runtime.content.read(cx);
            let markers = &content.workspace["media"][0]["markers"];
            markers.as_array().is_some_and(|markers| markers.len() == 2)
                && markers[0]["id"] != markers[1]["id"]
        })
    });
    context(&mut visual, "media-marker-1");
    click(&mut visual, "menu-item-Delete");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"][0]["markers"]
                .as_array()
                .is_some_and(|markers| markers.len() == 1)
        })
    });
    context(&mut visual, "media-marker-0");
    click(&mut visual, "menu-item-Add action");
    click(&mut visual, "menu-item-Clear");
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"][0]["markers"][0]["actions"][0]["type"]
                == "clear"
        })
    });
    assert_eq!(app.snapshot()["media"][0]["markers"][0]["title"], "Chorus");
    for (count, fraction) in [(2, 0.35), (3, 0.5)] {
        visual.simulate_click(
            gpui::point(
                timeline.left() + timeline.size.width * fraction,
                timeline.center().y,
            ),
            Default::default(),
        );
        visual.run_until_parked();
        click(&mut visual, "button-marker-add");
        app.wait(cx, |_, cx| {
            cx.update(|cx| {
                runtime.content.read(cx).workspace["media"][0]["markers"]
                    .as_array()
                    .is_some_and(|markers| markers.len() == count)
            })
        });
    }
    let saved = app.snapshot();
    let markers = saved["media"][0]["markers"].as_array().unwrap();
    assert!(markers[0]["color"].is_string() && markers[1]["color"].is_string());
    assert_ne!(markers[0]["color"], markers[1]["color"]);
    let from = visual.debug_bounds("preview-marker-2").unwrap().center();
    let to = gpui::point(timeline.left() + timeline.size.width * 0.25, from.y);
    visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
    visual.simulate_mouse_move(to, Some(gpui::MouseButton::Left), Default::default());
    visual.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            let content = runtime.content.read(cx);
            let markers = content.workspace["media"][0]["markers"].as_array().unwrap();
            markers[0]["id"] == "inspector-marker"
                && markers[0]["timeSeconds"]
                    .as_f64()
                    .is_some_and(|time| (time - 1.).abs() < 0.04)
                && markers
                    .is_sorted_by_key(|marker| marker["timeSeconds"].as_f64().unwrap().to_bits())
        })
    });
}

#[gpui::test]
fn media_inspector_adds_edits_and_manages_linked_effect_presets(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, |host| {
        seed(host);
        let path =
            host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let image = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/propresenter/orbit-int12-image.png");
        host.invoke(
            "saveMediaSettings",
            vec![
                path,
                json!([{"id":"preset-image","title":"Preset image","type":"image",
                    "libraryId":"default-graphics","sourcePath":image}]),
                snapshot["revision"].clone(),
            ],
        )
        .unwrap();
    });
    let runtime = app.runtime(cx);
    let stored =
        |cx: &mut TestAppContext| cx.update(|cx| runtime.content.read(cx).workspace.clone());
    let mut visual = app.visual(cx);
    context(&mut visual, "media-item-preset-image");
    click(&mut visual, "menu-item-Inspector...");
    let tabs = visual.debug_bounds("media-inspector-tab-bar").unwrap();
    let width = visual.update(|_, cx| {
        orbit_ui::ActiveTheme::theme(cx)
            .metrics
            .control_height_compact
    });
    visual.simulate_click(
        gpui::point(tabs.left() + width * 3.5, tabs.center().y),
        Default::default(),
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("effect-preset-name").is_none());
    click(&mut visual, "button-media-preset");
    assert!(visual.debug_bounds("menu-item-No selection").is_some());
    assert!(visual.debug_bounds("menu-item-Save as…").is_none());
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    click(&mut visual, "button-effect-add");
    click(&mut visual, "menu-item-Alpha Adjust");
    app.wait(cx, |_, cx| {
        stored(cx)["media"][0]["mediaEffects"][0]["enabled"] == true
    });

    let slider = visual
        .debug_bounds("effect-slider-effect:0:amount")
        .unwrap();
    let position = gpui::point(slider.left() + slider.size.width * 0.55, slider.center().y);
    visual.simulate_click(position, Default::default());
    app.wait(cx, |_, cx| {
        stored(cx)["media"][0]["mediaEffects"][0]["variables"]["amount"]
            .as_f64()
            .is_some_and(|amount| (0.4..0.7).contains(&amount))
    });
    click(&mut visual, "button-media-preset");
    click(&mut visual, "menu-item-Save as…");
    click(&mut visual, "effect-preset-name");
    visual.simulate_input("Test effect");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |_, cx| {
        let saved = stored(cx);
        saved["mediaEffectPresets"][0]["title"] == "Test effect"
            && saved["media"][0]["mediaEffectPresetId"] == saved["mediaEffectPresets"][0]["id"]
    });

    click(&mut visual, "button-effect-add");
    click(&mut visual, "menu-item-Adjust Color");
    app.wait(cx, |_, cx| {
        stored(cx)["media"][0]["mediaEffects"]
            .as_array()
            .is_some_and(|effects| effects.len() == 2)
    });
    let original = stored(cx)["media"][0]["mediaEffects"][0]["effectId"].clone();
    let from = visual.debug_bounds("media-effect-drag-0").unwrap().center();
    let to = visual.debug_bounds("media-effect-row-1").unwrap().center();
    visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
    visual.simulate_mouse_move(
        from + gpui::point(gpui::px(10.), gpui::px(0.)),
        Some(gpui::MouseButton::Left),
        Default::default(),
    );
    visual.simulate_mouse_move(to, Some(gpui::MouseButton::Left), Default::default());
    visual.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
    app.wait(cx, |_, cx| {
        stored(cx)["media"][0]["mediaEffects"][1]["effectId"] == original
    });
    click(&mut visual, "button-media-preset");
    click(&mut visual, "menu-item-Save");
    app.wait(cx, |_, cx| {
        stored(cx)["mediaEffectPresets"][0]["mediaEffects"]
            .as_array()
            .is_some_and(|effects| effects.len() == 2)
    });

    click(&mut visual, "button-media-preset");
    assert!(visual.debug_bounds("menu-item-Save").is_none());
    let row = visual.debug_bounds("menu-item-Test effect").unwrap();
    visual.simulate_mouse_move(row.center(), None, Default::default());
    visual.run_until_parked();
    click(&mut visual, "menu-item-Rename");
    click(&mut visual, "effect-preset-name");
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_input("Renamed effect");
    click(&mut visual, "button-effect-preset-confirm");
    app.wait(cx, |_, cx| {
        stored(cx)["mediaEffectPresets"][0]["title"] == "Renamed effect"
    });

    click(&mut visual, "button-media-preset");
    click(&mut visual, "menu-item-Unlink preset");
    app.wait(cx, |_, cx| {
        let saved = stored(cx);
        saved["media"][0]["mediaEffectPresetId"].is_null()
            && saved["media"][0]["mediaEffects"]
                .as_array()
                .is_some_and(|effects| effects.len() == 2)
    });
    click(&mut visual, "button-media-preset");
    click(&mut visual, "menu-item-Renamed effect");
    app.wait(cx, |_, cx| {
        stored(cx)["media"][0]["mediaEffectPresetId"].is_string()
    });
    click(&mut visual, "button-media-preset");
    let row = visual.debug_bounds("menu-item-Renamed effect").unwrap();
    visual.simulate_mouse_move(row.center(), None, Default::default());
    visual.run_until_parked();
    click(&mut visual, "menu-item-Delete");
    click(&mut visual, "button-effect-preset-confirm");
    app.wait(cx, |_, cx| {
        let saved = stored(cx);
        saved["mediaEffectPresets"] == json!([])
            && saved["media"][0]["mediaEffectPresetId"].is_null()
            && saved["media"][0]["mediaEffects"]
                .as_array()
                .is_some_and(|effects| effects.len() == 2)
    });
    context(&mut visual, "media-effect-row-0");
    click(&mut visual, "menu-item-Remove");
    app.wait(cx, |_, cx| {
        stored(cx)["media"][0]["mediaEffects"]
            .as_array()
            .is_some_and(|effects| effects.len() == 1)
    });
    click(&mut visual, "button-media-preset");
    click(&mut visual, "menu-item-Clear effects");
    app.wait(cx, |_, cx| {
        stored(cx)["media"][0]["mediaEffects"] == json!([])
    });
    assert_eq!(app.snapshot()["mediaEffectPresets"], json!([]));
}

#[gpui::test]
fn dropping_media_copies_assets_filters_the_bin_and_survives_restart(cx: &mut TestAppContext) {
    use gpui::{ExternalPaths, FileDropEvent};
    let source = support::Folder::new();
    let image = source.0.join("Sunday background.png");
    image::RgbaImage::from_pixel(16, 9, image::Rgba([24, 48, 72, 255]))
        .save(&image)
        .unwrap();
    let original = std::fs::read(&image).unwrap();
    let mut app = Scenario::new(cx, false, |host| {
        seed(host);
        let path =
            host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let mut libraries = snapshot["workspace"]["mediaLibraries"]
            .as_array()
            .unwrap()
            .clone();
        libraries.push(json!({"id":"content-folder","title":"Folder","type":"folder","mediaType":"graphics","parentId":"","collapsed":false,"itemFiles":[]}));
        host.invoke(
            "saveMediaLibraries",
            vec![path, json!(libraries), snapshot["revision"].clone()],
        )
        .unwrap();
    });
    let mut visual = app.visual(cx);
    let position = visual.debug_bounds("region-media").unwrap().center();
    visual.simulate_event(FileDropEvent::Entered {
        position,
        paths: ExternalPaths(vec![image.clone()].into()),
    });
    visual.simulate_event(FileDropEvent::Submit { position });
    visual.run_until_parked();
    app.wait(cx, |app, _| {
        app.snapshot()["media"]
            .as_array()
            .is_some_and(|items| items.len() == 1)
    });
    let asset = app.snapshot()["media"][0].clone();
    let managed = std::path::PathBuf::from(asset["sourcePath"].as_str().unwrap());
    assert!(std::fs::canonicalize(&managed)
        .unwrap()
        .starts_with(std::fs::canonicalize(app.workspace.0.join("Media")).unwrap()));
    assert_eq!(std::fs::read(&managed).unwrap(), original);
    let selector =
        Box::leak(format!("media-item-{}", asset["id"].as_str().unwrap()).into_boxed_str());
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds(selector).is_some()
    });
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    visual.update(|window, cx| {
        commands.run("view-media", false, window, cx);
    });
    visual.run_until_parked();
    visual.simulate_input("unmatched");
    visual.run_until_parked();
    assert!(visual.debug_bounds(selector).is_none());
    support::primary_shortcut(&mut visual, "a");
    visual.simulate_keystrokes("backspace");
    visual.run_until_parked();
    assert!(visual.debug_bounds(selector).is_some());
    assert!(visual.debug_bounds("media-root-false").is_none());
    click(&mut visual, "media-library-row-content-folder");
    assert!(visual.debug_bounds(selector).is_some());
    click(&mut visual, "media-folder-toggle-content-folder");
    assert!(visual.debug_bounds(selector).is_some());
    context(&mut visual, "media-library-row-default-graphics");
    click(&mut visual, "menu-item-Rename");
    visual.simulate_input("Media Default");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, _| {
        app.snapshot()["mediaLibraries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|library| {
                library["id"] == "default-graphics" && library["title"] == "Media Default"
            })
    });
    app.pump(cx);
    context(&mut visual, "media-library-row-default-graphics");
    click(&mut visual, "menu-item-Duplicate");
    app.wait(cx, |app, _| {
        app.snapshot()["media"].as_array().unwrap().len() == 2
    });
    let copied = app.snapshot()["mediaLibraries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|library| {
            library["id"] != "default-graphics"
                && library["mediaType"] == "graphics"
                && library["type"] == "playlist"
        })
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let copied_row: &'static str =
        Box::leak(format!("media-library-row-{copied}").into_boxed_str());
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds(copied_row).is_some()
    });
    context(&mut visual, copied_row);
    click(&mut visual, "menu-item-Delete");
    click(&mut visual, "button-media-delete-ok");
    app.wait(cx, |app, _| {
        app.snapshot()["media"].as_array().unwrap().len() == 1
            && !app.snapshot()["mediaLibraries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|library| library["id"] == copied)
    });
    std::fs::remove_file(&image).unwrap();
    app.restart(cx);
    assert_eq!(
        app.snapshot()["media"][0]["sourcePath"],
        asset["sourcePath"]
    );
    assert_eq!(std::fs::read(&managed).unwrap(), original);
    assert!(app.visual(cx).debug_bounds(selector).is_some());
    let path = json!(app.workspace.0.join("workspace.json"));
    let snapshot = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap();
    let mut unassigned = snapshot["workspace"]["media"].clone();
    unassigned[0]["libraryId"] = json!("");
    app.host
        .invoke(
            "saveMediaSettings",
            vec![path, unassigned, snapshot["revision"].clone()],
        )
        .unwrap();
    app.restart(cx);
    app.wait(cx, |app, _| {
        app.snapshot()["media"][0]["libraryId"] == "default-graphics"
    });
    assert!(app.visual(cx).debug_bounds(selector).is_some());
    assert!(app.visual(cx).debug_bounds("media-root-false").is_none());

    let mut visual = app.visual(cx);
    let extra: Vec<_> = [("Gold", [255, 180, 0, 255]), ("Blue", [0, 80, 255, 255])]
        .into_iter()
        .map(|(name, rgba)| {
            let image = source.0.join(format!("{name}.png"));
            image::RgbaImage::from_pixel(16, 9, image::Rgba(rgba))
                .save(&image)
                .unwrap();
            image
        })
        .collect();
    let position = visual.debug_bounds("region-media").unwrap().center();
    visual.simulate_event(FileDropEvent::Entered {
        position,
        paths: ExternalPaths(extra.into()),
    });
    visual.simulate_event(FileDropEvent::Submit { position });
    app.wait(cx, |app, _| {
        app.snapshot()["media"].as_array().unwrap().len() == 3
    });
    let media = app.snapshot()["media"]
        .as_array()
        .unwrap()
        .iter()
        .map(|asset| {
            let id = asset["id"].as_str().unwrap().to_string();
            let selector: &'static str = Box::leak(format!("media-item-{id}").into_boxed_str());
            (id, selector)
        })
        .collect::<Vec<_>>();
    app.wait(cx, |app, cx| {
        media
            .iter()
            .all(|(_, key)| app.visual(cx).debug_bounds(key).is_some())
    });
    let mut media = media;
    media.sort_by_key(|(_, key)| {
        let position = visual.debug_bounds(key).unwrap().origin;
        (position.y, position.x)
    });
    click(&mut visual, "document-0");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-1").is_some()
    });
    click(&mut visual, media[0].1);
    let last = visual.debug_bounds(media[2].1).unwrap().center();
    visual.simulate_click(
        last,
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    visual.run_until_parked();
    let middle = visual.debug_bounds(media[1].1).unwrap().center();
    visual.simulate_click(
        middle,
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
    );
    visual.run_until_parked();
    let runtime = app.runtime(cx);
    assert_eq!(
        cx.update(|cx| runtime.output.read(cx).live["backgroundId"].clone()),
        media[0].0
    );
    let drag_to = |visual: &mut gpui::VisualTestContext, to, marker| {
        let from = visual.debug_bounds(media[2].1).unwrap().center();
        visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
        visual.simulate_mouse_move(
            from + gpui::point(gpui::px(12.), gpui::px(0.)),
            Some(gpui::MouseButton::Left),
            Default::default(),
        );
        visual.simulate_mouse_move(to, Some(gpui::MouseButton::Left), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds(marker).is_some());
        visual.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
        visual.run_until_parked();
    };
    let first = visual.debug_bounds("cue-thumbnail-0").unwrap();
    let second = visual.debug_bounds("cue-thumbnail-1").unwrap();
    let gap = gpui::point((first.right() + second.left()) / 2., first.center().y);
    let original = app.snapshot()["library"][0].clone();
    let item_id = original["id"].as_str().unwrap();
    let preview_slides = |cx: &mut TestAppContext| {
        cx.update(|cx| runtime.content.read(cx).items.get(item_id).unwrap().value["slides"].clone())
    };
    let release = support::hold_workspace_writes(&app, cx);
    drag_to(&mut visual, gap, "cue-media-drop-insert");
    app.wait(cx, |app, cx| {
        preview_slides(cx).as_array().unwrap().len() == 4
            && app.visual(cx).debug_bounds("cue-thumbnail-3").is_some()
    });
    assert_eq!(app.snapshot()["library"][0], original);
    let inserted = preview_slides(cx);
    assert_eq!(inserted[0], original["slides"][0]);
    assert_eq!(inserted[1]["mediaBackgroundId"], media[0].0);
    assert_eq!(inserted[2]["mediaBackgroundId"], media[2].0);
    assert!(inserted[1]["mediaBackgroundOnly"].is_null());
    assert!(inserted[2]["mediaBackgroundOnly"].is_null());
    assert_eq!(inserted[3], original["slides"][1]);
    let target = visual.debug_bounds("cue-thumbnail-0").unwrap().center();
    drag_to(&mut visual, target, "cue-media-drop-background");
    assert_eq!(preview_slides(cx)[0]["mediaBackgroundId"], media[2].0);
    assert_eq!(app.snapshot()["library"][0], original);
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        app.snapshot()["library"][0]["slides"][0]["mediaBackgroundId"] == media[2].0
    });
    let mut expected = inserted;
    expected[0]["mediaBackgroundId"] = json!(media[2].0);
    assert_eq!(app.snapshot()["library"][0]["slides"], expected);
    let workspace = app.snapshot();
    let asset = workspace["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["id"] == media[2].0)
        .unwrap();
    let still = app
        .host
        .render_cue_still(
            &workspace,
            &workspace["library"][0],
            &expected[0],
            "",
            320,
            180,
        )
        .unwrap();
    let picture = image::open(asset["sourcePath"].as_str().unwrap())
        .unwrap()
        .to_rgba8();
    let [r, g, b, a] = picture.get_pixel(0, 0).0;
    let background = [b, g, r, a];
    assert_eq!(still.pixels[..4], background);
    assert!(still
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| *pixel != background));
    let stored = app
        .host
        .invoke(
            "readWorkspaceSnapshot",
            vec![json!(app.workspace.0.join("workspace.json"))],
        )
        .unwrap();
    assert!(app
        .host
        .invoke(
            "mutatePresentationLibrary",
            vec![
                json!(app.workspace.0.join("workspace.json")),
                json!({
                    "type":"mutateItem","itemId":workspace["library"][0]["id"],"operation":{
                        "type":"insertMediaSlides","slideId":expected[0]["id"],"placement":"before",
                        "mediaIds":[media[0].0,"missing-media"]
                    }
                }),
                stored["revision"].clone()
            ]
        )
        .is_err());
    assert_eq!(app.snapshot()["library"][0]["slides"], expected);
    click(&mut visual, "document-0");
    let first = visual.debug_bounds("cue-thumbnail-0").unwrap();
    let second = visual.debug_bounds("cue-thumbnail-1").unwrap();
    let gap = gpui::point((first.right() + second.left()) / 2., first.center().y);
    drag_to(&mut visual, gap, "cue-media-drop-insert");
    app.wait(cx, |app, _| {
        app.snapshot()["library"][0]["slides"]
            .as_array()
            .unwrap()
            .len()
            == 5
    });
    let after_deselect = app.snapshot()["library"][0]["slides"].clone();
    assert_eq!(after_deselect[1]["mediaBackgroundId"], media[2].0);
    assert_eq!(after_deselect[2], expected[1]);
    context(&mut visual, "cue-thumbnail-0");
    let selectors = [
        "menu-item-Edit Slide",
        "menu-item-Disable",
        "menu-item-Themes",
        "menu-item-Transitions…",
        "menu-item-Hot Key…",
        "menu-item-Go to Next Timer…",
        "menu-item-Add Action",
        "menu-item-Add Media Action",
        "menu-item-Group",
        "menu-item-Label",
        "menu-item-Copy Text Style",
        "menu-item-Paste Text Style",
        "menu-item-Cut",
        "menu-item-Copy",
        "menu-item-Paste",
        "menu-item-Delete",
    ];
    let rows = selectors.map(|selector| visual.debug_bounds(selector).unwrap().top());
    assert!(rows.windows(2).all(|rows| rows[0] < rows[1]));
    assert!(visual.debug_bounds("menu-item-Quick Edit").is_none());

    click(&mut visual, "menu-item-Add Media Action");
    let asset = app.snapshot()["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["id"] == media[0].0)
        .unwrap()
        .clone();
    click(
        &mut visual,
        &format!("menu-item-{}", asset["title"].as_str().unwrap()),
    );
    app.wait(cx, |app, _| {
        app.snapshot()["library"][0]["slides"][0]["actions"][0]["targetId"] == media[0].0
    });
    let path = std::path::Path::new(asset["sourcePath"].as_str().unwrap());
    let filename = path.file_name().unwrap().to_str().unwrap();
    let edit = format!("menu-item-Edit Action: {filename}");
    context(&mut visual, "cue-thumbnail-0");
    click(&mut visual, &edit);
    click(&mut visual, "menu-item-Scaling");
    click(&mut visual, "menu-item-Scale to Fill");
    app.wait(cx, |app, _| {
        app.snapshot()["library"][0]["slides"][0]["actions"][0]["details"]["backgroundScale"]
            == "fill"
    });
    let workspace = app.snapshot();
    let stored_asset = workspace["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stored| stored["id"] == asset["id"])
        .unwrap();
    assert_eq!(stored_asset["backgroundScale"], asset["backgroundScale"]);
    context(&mut visual, "cue-thumbnail-0");
    click(&mut visual, &edit);
    click(
        &mut visual,
        "menu-item-Convert Media Action to Slide Element",
    );
    app.wait(cx, |app, _| {
        let workspace = app.snapshot();
        let slide = &workspace["library"][0]["slides"][0];
        slide["actions"] == json!([]) && slide["layers"][0]["mediaScale"] == "Scale to Fill"
    });
    let drop_asset = workspace["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["id"] == media[2].0)
        .unwrap();
    let drop_filename = std::path::Path::new(drop_asset["sourcePath"].as_str().unwrap())
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    context(&mut visual, "cue-thumbnail-0");
    click(
        &mut visual,
        &format!("menu-item-Remove Action: {drop_filename}"),
    );
    app.wait(cx, |app, _| {
        app.snapshot()["library"][0]["slides"][0]["mediaBackgroundId"].is_null()
    });
}

#[gpui::test]
fn media_thumbnails_and_video_start_preserve_the_outgoing_picture(cx: &mut TestAppContext) {
    use gpui::{ExternalPaths, FileDropEvent};
    let source = support::Folder::new();
    let image = source.0.join("Still.png");
    image::RgbaImage::from_pixel(3840, 2160, image::Rgba([24, 48, 72, 255]))
        .save(&image)
        .unwrap();
    let transparent = source.0.join("Transparent.png");
    image::RgbaImage::from_fn(16, 9, |x, _| {
        image::Rgba([240, 120, 60, if x < 8 { 255 } else { 0 }])
    })
    .save(&transparent)
    .unwrap();
    let mut app = Scenario::new(cx, true, seed);
    let runtime = app.runtime(cx);
    cx.update(|cx| {
        crate::output::windows::mutate_config(
            &runtime,
            json!({"type":"add","screenType":"audience","baseLabel":"Media output"}),
            cx,
        )
    });
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.output.read(cx).settings["screenConfig"]
                .as_object()
                .is_some_and(|screens| screens.values().any(|config| config["type"] == "audience"))
        })
    });

    let canvas_screen = cx.update(|cx| {
        runtime.output.read(cx).settings["screenConfig"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, screen)| screen["type"] == "audience")
            .unwrap()
            .0
            .clone()
    });
    cx.update(|cx| {
        crate::output::windows::mutate_config(
            &runtime,
            json!({"type":"update", "screenId":canvas_screen,
            "patch":{"canvasSize":{"width":1920,"height":1080}}}),
            cx,
        )
    });
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime
                .output
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| {
                    snapshot["screenConfig"][&canvas_screen]["canvasSize"]["width"] == 1920
                })
        })
    });

    let svg = source.0.join("Native thumbnail.svg");
    std::fs::write(&svg, r##"<svg xmlns="http://www.w3.org/2000/svg" width="90" height="160"><rect width="90" height="160" fill="#ff0000"/></svg>"##).unwrap();
    let video = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/propresenter/orbit-int12-video.mp4");
    let mut visual = app.visual(cx);
    let position = visual.debug_bounds("region-media").unwrap().center();
    visual.simulate_event(FileDropEvent::Entered {
        position,
        paths: ExternalPaths(vec![image, transparent, svg, video].into()),
    });
    visual.simulate_event(FileDropEvent::Submit { position });
    visual.run_until_parked();
    app.wait(cx, |_, cx| {
        cx.update(|cx| {
            runtime.content.read(cx).workspace["media"]
                .as_array()
                .is_some_and(|media| media.len() == 4)
        })
    });
    let path = json!(app.workspace.0.join("workspace.json"));
    let snapshot = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap();
    let mut media = snapshot["workspace"]["media"].clone();
    let clip = media
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|item| item["type"] == "video")
        .unwrap();
    let thumbnail = app.workspace.0.join("Media/Thumbnails/stored.png");
    std::fs::create_dir_all(thumbnail.parent().unwrap()).unwrap();
    image::RgbaImage::from_pixel(16, 9, image::Rgba([90, 120, 150, 255]))
        .save(&thumbnail)
        .unwrap();
    clip["thumbnailFile"] = json!("Thumbnails/stored.png");
    clip["thumbnailTimeSeconds"] = json!(0.2);
    clip["backgroundTransition"] = json!("fade:0.5:Dissolve");
    clip["backgroundBehavior"] = json!("background");
    clip["backgroundPlayback"] = json!("loop");
    clip["mediaHardwareDecoding"] = json!("disabled");
    clip["mediaTransport"] = json!({"inSeconds":0.2,"outSeconds":4.000005,"rate":1,"delaySeconds":1,"retrigger":"restart"});
    app.host
        .invoke(
            "saveMediaSettings",
            vec![path, media, snapshot["revision"].clone()],
        )
        .unwrap();
    app.restart(cx);
    let loaded = app.snapshot();
    let stored = loaded["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "video")
        .unwrap();
    let preview = app
        .host
        .render_media_thumbnail(
            app.workspace.0.join("workspace.json").to_str().unwrap(),
            stored,
        )
        .unwrap();
    assert_eq!(preview.get_pixel(0, 0).0, [90, 120, 150, 255]);
    assert!(app
        .host
        .render_media_thumbnail(
            app.workspace.0.join("workspace.json").to_str().unwrap(),
            &json!({"type":"video"})
        )
        .is_err());
    std::fs::remove_file(thumbnail).unwrap();
    app.restart(cx);
    for item in app.snapshot()["media"].as_array().unwrap() {
        let selector =
            Box::leak(format!("media-thumbnail-{}", item["id"].as_str().unwrap()).into_boxed_str());
        assert!(
            app.visual(cx).debug_bounds(selector).is_some(),
            "missing thumbnail: {selector}"
        );
    }
    assert!(app.visual(cx).debug_bounds("first-page-loading").is_none());
    assert!(app
        .visual(cx)
        .debug_bounds("program-preview-ready")
        .is_some());
    let prepared_media = app.snapshot()["media"].as_array().unwrap().clone();
    let transparent_item = prepared_media
        .iter()
        .find(|item| item["title"] == "Transparent")
        .unwrap();
    let transparent_id = transparent_item["id"].as_str().unwrap();
    let checker: &'static str =
        Box::leak(format!("media-transparency-{transparent_id}").into_boxed_str());
    assert!(app.visual(cx).debug_bounds(checker).is_some());
    assert!(app
        .visual(cx)
        .debug_bounds("button-media.missing.title")
        .is_none());
    let thumbnail = app
        .host
        .render_media_thumbnail(
            app.workspace.0.join("workspace.json").to_str().unwrap(),
            transparent_item,
        )
        .unwrap();
    assert_eq!(thumbnail.get_pixel(0, 0)[3], 255);
    assert_eq!(thumbnail.get_pixel(thumbnail.width() - 1, 0)[3], 0);
    let still_id = prepared_media
        .iter()
        .find(|item| item["title"] == "Still")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let opaque_item = prepared_media
        .iter()
        .find(|item| item["id"] == still_id)
        .unwrap();
    let opaque_thumbnail = app
        .host
        .render_media_thumbnail(
            app.workspace.0.join("workspace.json").to_str().unwrap(),
            opaque_item,
        )
        .unwrap();
    assert!(
        opaque_thumbnail.pixels().all(|pixel| pixel[3] == 255),
        "Opaque thumbnail alpha: {:?}, size: {:?}",
        opaque_thumbnail.pixels().map(|pixel| pixel[3]).min(),
        opaque_thumbnail.dimensions()
    );
    let mut thumbnails = app.visual(cx);
    let opaque_checker: &'static str =
        Box::leak(format!("media-transparency-{still_id}").into_boxed_str());
    assert!(thumbnails.debug_bounds(opaque_checker).is_none());
    let views = thumbnails.debug_bounds("media-view-controls").unwrap();
    thumbnails.simulate_click(
        gpui::point(views.left() + views.size.width * 0.75, views.center().y),
        Default::default(),
    );
    thumbnails.run_until_parked();
    assert!(thumbnails.debug_bounds("media-table-header").is_some());
    assert!(thumbnails.debug_bounds(checker).is_some());
    assert!(thumbnails.debug_bounds(opaque_checker).is_none());
    thumbnails.simulate_click(
        gpui::point(views.left() + views.size.width * 0.25, views.center().y),
        Default::default(),
    );
    thumbnails.run_until_parked();
    assert!(thumbnails.debug_bounds("media-table-header").is_none());
    let mut smallest = gpui::px(f32::MAX);
    let mut largest = gpui::px(0.);
    for fraction in [0., 0.5, 1., 0.5] {
        let slider = thumbnails.debug_bounds("media-size-slider").unwrap();
        thumbnails.simulate_click(
            gpui::point(
                slider.left() + gpui::px(4.) + (slider.size.width - gpui::px(8.)) * fraction,
                slider.center().y,
            ),
            gpui::Modifiers::default(),
        );
        thumbnails.run_until_parked();
        for item in &prepared_media {
            let id = item["id"].as_str().unwrap();
            let selector = Box::leak(format!("media-thumbnail-{id}").into_boxed_str());
            let Some(picture) = thumbnails.debug_bounds(selector) else {
                continue;
            };
            let viewport = thumbnails
                .debug_bounds(Box::leak(
                    format!("media-thumbnail-viewport-{id}").into_boxed_str(),
                ))
                .unwrap();
            assert!(
                (picture.size.height - viewport.size.height).abs() <= gpui::px(2.),
                "Thumbnail must fill its viewport height"
            );
            assert!(
                (picture.size.width - viewport.size.width).abs() <= gpui::px(2.),
                "Thumbnail must fill its viewport width"
            );
            if id == still_id {
                smallest = smallest.min(picture.size.height);
                largest = largest.max(picture.size.height);
            }
        }
    }
    assert!(
        largest > smallest * 2.,
        "The thumbnail must resize with the media zoom control"
    );

    app.wait(cx, |app, _| {
        let local = app
            .host
            .invoke("getMediaPreparationStatus", vec![])
            .unwrap();
        let remote =
            crate::state::runtime::output_call(&app.host, "getMediaPreparationStatus", vec![])
                .unwrap();
        [local, remote].into_iter().all(|status| {
            status["pending"] == false
                && prepared_media.iter().all(|item| {
                    status["ready"].as_array().unwrap().iter().any(|entry| {
                        entry["path"] == item["sourcePath"]
                            && (item["type"] != "video"
                                || entry["startSeconds"].as_f64() == Some(0.2))
                    })
                })
        })
    });
    let status = app
        .host
        .invoke("getMediaPreparationStatus", vec![])
        .unwrap();
    assert!(status["bytes"].as_u64().unwrap() <= status["budgetBytes"].as_u64().unwrap());
    assert_eq!(status["errors"], json!([]));
    for invalid in [
        json!([{"path":"relative.mp4","kind":"video","startSeconds":0}]),
        json!([{"path":prepared_media[0]["sourcePath"],"kind":"audio","startSeconds":0}]),
        json!([{"path":prepared_media[0]["sourcePath"],"kind":"video","startSeconds":-1}]),
        json!([{"path":prepared_media[0]["sourcePath"],"kind":"image","startSeconds":0,"unexpected":true}]),
    ] {
        assert!(
            crate::state::runtime::output_call(&app.host, "prepareMedia", vec![invalid]).is_err()
        );
    }
    let initial_image_hits = status["imageHits"].as_u64().unwrap();
    let initial_video_hits = status["videoHits"].as_u64().unwrap();
    let clip = app.snapshot()["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "video")
        .unwrap()
        .clone();
    let clip_id = clip["id"].as_str().unwrap();
    let screen = app.snapshot()["screenConfig"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, config)| config["type"] == "audience")
        .unwrap()
        .0
        .clone();
    let scene = |app: &Scenario| {
        let mut content =
            crate::state::runtime::output_call(&app.host, "getOutputContent", vec![json!(screen)])?;
        let playback = crate::state::runtime::output_call(
            &app.host,
            "getNativeMediaPlayback",
            vec![json!(screen)],
        )?;
        content["backgroundPlaybackState"] = playback["playback"].clone();
        Ok::<Value, String>(content)
    };
    let compositor =
        std::cell::RefCell::new(orbit_host::native_compositor::NativeCompositor::new_preview());
    let render = |app: &Scenario| {
        compositor
            .borrow_mut()
            .try_render_layered_for("media-workflow", &scene(app).unwrap(), 64, 36)
            .unwrap()
            .flatten()
    };
    let still = app.snapshot()["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["title"] == "Still")
        .unwrap()
        .clone();
    let mut visual = app.visual(cx);
    click(
        &mut visual,
        &format!("media-item-{}", still["id"].as_str().unwrap()),
    );
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, _| {
        scene(app).is_ok_and(|scene| scene["backgroundMediaType"] == "image")
    });
    let outgoing = render(&app);
    assert_eq!(scene(&app).unwrap()["canvasViewport"]["width"], 1920);
    app.wait(cx, |app, _| {
        app.host
            .invoke("getOutputContent", vec![json!(screen)])
            .is_ok_and(|content| content["backgroundMediaType"] == "image")
    });
    let square = app.host.render_output_bgra_frame(&screen, 32, 32).unwrap();
    assert_eq!(
        &square.pixels[..4],
        &[0, 0, 0, 0],
        "A full canvas must retain transparent letterboxing for a different consumer aspect ratio"
    );

    let center = (16 * square.width + 16) * 4;
    assert_eq!(
        &square.pixels[center..center + 4],
        &[72, 48, 24, 255],
        "The fitted canvas must preserve its image inside the letterbox"
    );
    assert!(
        app.host
            .invoke("getMediaPreparationStatus", vec![])
            .unwrap()["imageHits"]
            .as_u64()
            .unwrap()
            > initial_image_hits
    );
    click(&mut visual, &format!("media-item-{transparent_id}"));
    app.wait(cx, |app, _| {
        scene(app).is_ok_and(|scene| scene["backgroundMediaPath"] == transparent_item["sourcePath"])
    });
    assert_ne!(
        render(&app).pixels,
        outgoing.pixels,
        "Taking another image must replace the previously scaled bitmap"
    );
    click(
        &mut visual,
        &format!("media-item-{}", still["id"].as_str().unwrap()),
    );
    app.wait(cx, |app, _| {
        scene(app).is_ok_and(|scene| scene["backgroundMediaPath"] == still["sourcePath"])
    });
    assert_eq!(
        render(&app).pixels,
        outgoing.pixels,
        "Returning to a cached still must restore its original pixels"
    );
    let clip_selector = Box::leak(format!("media-item-{clip_id}").into_boxed_str());
    let position = visual.debug_bounds(clip_selector).unwrap().center();
    let activate_media = |visual: &mut gpui::VisualTestContext,
                          position: gpui::Point<gpui::Pixels>| {
        visual.update(|window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    position,
                    button: gpui::MouseButton::Left,
                    modifiers: gpui::Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                    position,
                    button: gpui::MouseButton::Left,
                    modifiers: gpui::Modifiers::default(),
                    click_count: 1,
                }),
                cx,
            );
        })
    };
    let prepared =
        crate::state::runtime::output_call(&app.host, "getMediaPreparationStatus", vec![]).unwrap();
    activate_media(&mut visual, position);
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| scene["backgroundMediaType"] == "video")
    });
    assert_eq!(
        scene(&app).unwrap()["backgroundMediaHardwareDecoding"],
        "disabled"
    );
    let playback = scene(&app).unwrap()["backgroundPlaybackState"].clone();
    assert_eq!(playback["outSeconds"], 4.);
    assert!(playback["error"].is_null());
    let activated =
        crate::state::runtime::output_call(&app.host, "getMediaPreparationStatus", vec![]).unwrap();
    assert_eq!(
        activated["metadataLoads"], prepared["metadataLoads"],
        "Taking prepared media must reuse its probed metadata"
    );
    assert!(
        activated["metadataHits"].as_u64().unwrap() > prepared["metadataHits"].as_u64().unwrap()
    );
    let waiting = render(&app);
    assert_eq!(
        waiting.pixels, outgoing.pixels,
        "The previous picture must remain while video is delayed or decoding"
    );
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| scene["backgroundPlaybackState"]["delayed"] == false)
    });
    let first = render(&app);
    assert_eq!(
        first.pixels, outgoing.pixels,
        "Loading must not consume the fade before the first incoming frame"
    );
    app.wait_animated(cx, |app, _| render(app).pixels != outgoing.pixels);
    assert!(
        app.host
            .invoke("getMediaPreparationStatus", vec![])
            .unwrap()["videoHits"]
            .as_u64()
            .unwrap()
            > initial_video_hits,
        "Prepared video was not consumed: scene={}, cache={}",
        scene(&app).unwrap(),
        app.host
            .invoke("getMediaPreparationStatus", vec![])
            .unwrap()
    );
    let selection = scene(&app).unwrap()["backgroundSelectionId"].clone();
    let video_hits = app
        .host
        .invoke("getMediaPreparationStatus", vec![])
        .unwrap()["videoHits"]
        .as_u64()
        .unwrap();
    activate_media(&mut visual, position);
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| scene["backgroundSelectionId"] != selection)
    });
    render(&app);
    app.wait_animated(cx, |app, _| {
        render(app);
        app.host
            .invoke("getMediaPreparationStatus", vec![])
            .unwrap()["videoHits"]
            .as_u64()
            .unwrap()
            > video_hits
    });
    image::RgbaImage::from_pixel(32, 18, image::Rgba([11, 22, 33, 255]))
        .save(std::path::Path::new(still["sourcePath"].as_str().unwrap()))
        .unwrap();
    let still_selector =
        Box::leak(format!("media-item-{}", still["id"].as_str().unwrap()).into_boxed_str());
    let still_position = visual.debug_bounds(still_selector).unwrap().center();
    activate_media(&mut visual, still_position);
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| scene["backgroundMediaType"] == "image")
    });
    let refreshed = render(&app);
    let pixel = (18 * 64 + 32) * 4;
    assert_eq!(
        &refreshed.pixels[pixel..pixel + 4],
        &[33, 22, 11, 255],
        "A changed source must not reuse stale cached pixels"
    );
    let metadata_copy = source.0.join("metadata-cache.mp4");
    std::fs::copy(clip["sourcePath"].as_str().unwrap(), &metadata_copy).unwrap();
    let counts = || {
        app.host
            .invoke("getMediaPreparationStatus", vec![])
            .unwrap()
    };
    let first_metadata = app
        .host
        .invoke("probeMediaMetadata", vec![json!(metadata_copy)])
        .unwrap();
    let first_loads = counts()["metadataLoads"].as_u64().unwrap();
    assert_eq!(
        app.host
            .invoke("probeMediaMetadata", vec![json!(metadata_copy)])
            .unwrap(),
        first_metadata
    );
    assert_eq!(counts()["metadataLoads"].as_u64().unwrap(), first_loads);
    let modified = std::fs::metadata(&metadata_copy)
        .unwrap()
        .modified()
        .unwrap()
        + std::time::Duration::from_secs(2);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&metadata_copy)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(
        app.host
            .invoke("probeMediaMetadata", vec![json!(metadata_copy)])
            .unwrap(),
        first_metadata
    );
    assert_eq!(
        counts()["metadataLoads"].as_u64().unwrap(),
        first_loads + 1,
        "Changed media must not reuse stale metadata"
    );

    cx.update(|cx| {
        for window in cx.windows() {
            window
                .update(cx, |_, window, _| window.remove_window())
                .unwrap();
        }
    });
}

#[gpui::test]
fn slide_and_media_takes_follow_their_layer_transitions(cx: &mut TestAppContext) {
    let files = support::Folder::new();
    let front = files.0.join("Foreground.png");
    let back = files.0.join("Background.png");
    image::RgbaImage::from_pixel(16, 9, image::Rgba([11, 22, 33, 255]))
        .save(&front)
        .unwrap();
    image::RgbaImage::from_pixel(16, 9, image::Rgba([44, 55, 66, 255]))
        .save(&back)
        .unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/gpui-output-freeze.json"
    ))
    .unwrap();
    let mut app =
        Scenario::new(cx, true, |host| {
            seed(host);
            let path =
                host.invoke("getSettings", vec![]).unwrap()["presenterOrbitWorkspacePath"].clone();
            let snapshot = host
                .invoke("readWorkspaceSnapshot", vec![path.clone()])
                .unwrap();
            let assets = host.invoke("saveMediaSettings", vec![path.clone(), json!([
            {"id":"front", "title":"Foreground", "libraryId":"default-graphics",
                "type":"image", "sourcePath":front, "backgroundBehavior":"foreground",
                "backgroundTransition":"cut"},
            {"id":"back", "title":"Background", "libraryId":"default-graphics",
                "type":"image", "sourcePath":back, "backgroundBehavior":"background"}
        ]), snapshot["revision"].clone()]).unwrap();
            let item = &snapshot["workspace"]["library"][0];
            let mut slides = item["slides"].as_array().unwrap().clone();
            slides[0]["background"] = fixture["backgrounds"][0].clone();
            slides[0]["backgroundEnabled"] = json!(true);
            slides[0]["shortcut"] = json!("A");
            slides[0]["groupId"] = json!("verse");
            slides[1]["mediaBackgroundId"] = json!("front");
            slides[1]["shortcut"] = json!("S");
            slides[1]["groupId"] = json!("chorus");
            let mut background = slides[1].clone();
            background["id"] = json!("background-slide");
            background["content"] = json!("Background slide");
            background["shortcut"] = json!("D");
            background["mediaBackgroundId"] = json!("back");
            slides.push(background);
            let mut action = slides[2].clone();
            action["id"] = json!("media-action-slide");
            action["content"] = json!("Media action slide");
            action["shortcut"] = json!("F");
            action.as_object_mut().unwrap().remove("mediaBackgroundId");
            action["actions"] = json!([
                {"id":"play-back", "type":"media", "targetId":"back",
                    "enabled":true, "delaySeconds":0,
                    "details":{"backgroundBehavior":"foreground"}},
                {"id":"clear-music", "type":"clear", "targetLayer":"audio",
                    "enabled":true, "delaySeconds":0}
            ]);
            slides.push(action);
            let saved = host.invoke("mutatePresentationLibrary", vec![path.clone(), json!({
            "type":"mutateItem", "itemId":item["id"],
            "operation":{"type":"setSlideDocument","title":item["title"],"slides":slides}
        }), assets["revision"].clone()]).unwrap();
            host.invoke(
                "saveWorkspaceContent",
                vec![
                    path,
                    json!({
                        "layerTransitions":{"background":crate::transitions::fade(1.3)}
                    }),
                    saved["revision"].clone(),
                ],
            )
            .unwrap();
        });
    let runtime = app.runtime(cx);
    app.wait(cx, |_, cx| {
        cx.update(|cx| runtime.output.read(cx).committed_revision > 0)
    });
    let screen = cx.update(|cx| {
        crate::output::view::screens(runtime.output.read(cx).snapshot.as_ref().unwrap())[0]
            .0
            .clone()
    });
    let scene = |app: &Scenario| {
        crate::state::runtime::output_call(&app.host, "getOutputContent", vec![json!(screen)])
    };
    let compositor =
        std::cell::RefCell::new(orbit_host::native_compositor::NativeCompositor::new_preview());
    let render = |app: &Scenario| {
        compositor
            .borrow_mut()
            .try_render_layered_for("slide-and-media-transitions", &scene(app).unwrap(), 64, 36)
            .unwrap()
            .flatten()
    };
    let mut visual = app.visual(cx);
    assert!(visual
        .debug_bounds("media-layer-foreground-front")
        .is_some());
    assert!(visual.debug_bounds("media-layer-background-back").is_none());
    assert!(visual.debug_bounds("media-layer-foreground-back").is_none());
    click(&mut visual, "document-0");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-2").is_some()
    });
    assert!(visual.debug_bounds("cue-copy-1").is_some());
    assert!(visual.debug_bounds("cue-picture-1").is_none());
    assert!(visual.debug_bounds("cue-media-foreground-1").is_none());
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let show = |visual: &mut gpui::VisualTestContext, mode: &str| {
        visual.update(|window, cx| {
            assert!(commands.run(mode, false, window, cx));
            window.simulate_next_frame(cx);
        });
        visual.run_until_parked();
    };
    let button = "button-cue-view-options-button";
    click(&mut visual, button);
    assert!(visual
        .debug_bounds("select-cue-view-font-trigger")
        .is_some());
    click(&mut visual, "cue-view-showMediaAndColors");
    click(&mut visual, "cue-view-groupSlides");
    assert!(
        visual.debug_bounds("cue-thumbnail-1").unwrap().top()
            > visual.debug_bounds("cue-thumbnail-0").unwrap().top()
    );
    assert!(visual.debug_bounds("cue-picture-1").is_none());
    click(&mut visual, "select-cue-view-truncate-trigger");
    click(&mut visual, "select-option-1");
    click(&mut visual, button);
    show(&mut visual, "view-grid");
    assert!(visual.debug_bounds("cue-picture-1").is_some());
    assert!(visual.debug_bounds("cue-copy-1").is_none());
    click(&mut visual, button);
    assert!(visual
        .debug_bounds("select-cue-view-font-trigger")
        .is_none());
    click(&mut visual, "cue-view-useThumbnailColor");
    click(&mut visual, button);
    show(&mut visual, "view-table");
    assert!(visual.debug_bounds("cue-picture-1").is_some());
    assert!(visual.debug_bounds("cue-copy-1").is_some());
    click(&mut visual, button);
    assert!(visual
        .debug_bounds("select-cue-view-font-trigger")
        .is_some());
    assert!(visual.debug_bounds("cue-view-useThumbnailColor").is_some());
    click(&mut visual, button);
    app.wait(cx, |app, _| {
        let settings = app.host.invoke("getSettings", vec![]).unwrap();
        let options = &settings["generalSettings"]["cueViewOptions"];
        options["easy"]["showMediaAndColors"] == true
            && options["easy"]["groupSlides"] == true
            && options["easy"]["truncate"] == "middle"
            && options["grid"]["useThumbnailColor"] == true
            && options["table"]["useThumbnailColor"] == false
    });
    show(&mut visual, "view-easy");
    assert!(visual.debug_bounds("cue-media-foreground-1").is_some());
    assert!(visual.debug_bounds("cue-media-background-2").is_some());
    assert!(visual.debug_bounds("cue-media-background-0").is_none());
    assert!(visual.debug_bounds("cue-media-foreground-0").is_none());
    let controls = visual.debug_bounds("transition-mode-slide").unwrap();
    visual.simulate_click(controls.center(), Default::default());
    click(&mut visual, "cue-thumbnail-0");
    visual.simulate_keystrokes("enter");
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| {
            scene["text"] == "Opening line"
                && scene["slideTransition"]["transition"] == crate::transitions::fade(0.6)
        })
    });
    app.wait_animated(cx, |app, _| !render(app).dynamic_media);
    let text_frame = render(&app);
    assert_eq!(&text_frame.pixels[..4], &[0, 0, 255, 255]);

    visual.simulate_keystrokes("right");
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| {
            scene["text"] == "Closing line" && scene["backgroundBehavior"] == "foreground"
        })
    });
    let first = render(&app);
    assert!(first.dynamic_media);
    assert!(
        first.pixels == text_frame.pixels,
        "The fade must start from the text slide"
    );
    app.wait_animated(cx, |app, _| {
        let frame = render(app);
        frame.dynamic_media && frame.pixels != text_frame.pixels
    });
    app.wait_animated(cx, |app, _| !render(app).dynamic_media);
    let media_frame = render(&app);
    assert_eq!(&media_frame.pixels[..4], &[33, 22, 11, 255]);

    visual.simulate_keystrokes("left");
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| {
            scene["text"] == "Opening line" && scene["backgroundMediaType"] == ""
        })
    });
    assert!(
        render(&app).pixels == media_frame.pixels,
        "The return fade must start from the slide with foreground media"
    );
    app.wait_animated(cx, |app, _| !render(app).dynamic_media);
    assert!(
        render(&app).pixels == text_frame.pixels,
        "The return fade must finish on the text slide"
    );

    visual.simulate_click(
        gpui::point(controls.left() + gpui::px(12.), controls.center().y),
        Default::default(),
    );
    click(&mut visual, "cue-thumbnail-1");
    visual.simulate_keystrokes("enter");
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| {
            scene["text"] == "Closing line" && scene["slideTransition"]["transition"].is_null()
        })
    });
    let cut = render(&app);
    assert!(!cut.dynamic_media);
    assert!(
        cut.pixels == media_frame.pixels,
        "A cut must immediately show the slide and foreground media"
    );

    let foreground = scene(&app).unwrap();
    let item = app.snapshot()["library"][0].clone();
    cx.update(|cx| {
        crate::cues::actions::drop_media(
            &runtime,
            item["id"].as_str().unwrap(),
            json!({"type":"setSlideMedia", "slideId":item["slides"][1]["id"], "mediaId":"back"}),
            cx,
        );
    });
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| {
            scene["text"] == "Closing line"
                && scene["backgroundBehavior"] == "background"
                && scene["backgroundSelectionId"] != foreground["backgroundSelectionId"]
                && scene["layerTransitions"]["media"]["transition"] == crate::transitions::fade(1.3)
        })
    });
    assert!(visual.debug_bounds("cue-media-foreground-1").is_none());
    assert!(visual.debug_bounds("cue-media-background-1").is_some());
    let background_fade = render(&app);
    assert!(background_fade.dynamic_media);
    assert_eq!(&background_fade.pixels[..4], &[33, 22, 11, 255]);
    app.wait_animated(cx, |app, _| !render(app).dynamic_media);
    assert_eq!(&render(&app).pixels[..4], &[66, 55, 44, 255]);

    let release = support::hold_workspace_writes(&app, cx);
    let asset_name = cx.update(|cx| {
        let content = runtime.content.read(cx);
        let asset = content.workspace["media"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| asset["id"] == "back")
            .unwrap();
        std::path::Path::new(asset["sourcePath"].as_str().unwrap())
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    });
    let take_with_role = |visual: &mut gpui::VisualTestContext, thumbnail: &str, role: &str| {
        context(visual, thumbnail);
        click(visual, &format!("menu-item-Edit Action: {asset_name}"));
        click(visual, "menu-item-Behaviour");
        click(visual, &format!("menu-item-{role}"));
        click(visual, thumbnail);
        visual.simulate_keystrokes("enter");
    };
    take_with_role(&mut visual, "cue-thumbnail-2", "Foreground");
    app.wait(cx, |app, cx| {
        app.visual(cx)
            .debug_bounds("cue-media-foreground-2")
            .is_some()
    });
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| {
            scene["text"] == "Background slide" && scene["backgroundBehavior"] == "foreground"
        })
    });
    assert_ne!(
        app.snapshot()["library"][0]["slides"][2]["mediaBackgroundBehavior"],
        "foreground"
    );
    orbit_host::start_tickers(&app.host).unwrap();
    take_with_role(&mut visual, "cue-thumbnail-3", "Background");
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| {
            scene["text"] == "Media action slide"
                && scene["backgroundBehavior"] == "background"
                && scene["backgroundMediaType"] == "image"
        })
    });
    assert_ne!(
        app.snapshot()["library"][0]["slides"][3]["actions"][0]["details"]["backgroundBehavior"],
        "background"
    );
    visual.simulate_keystrokes("f3");
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| {
            scene["text"] == "Media action slide" && scene["backgroundMediaType"] == ""
        })
    });
    visual.simulate_keystrokes("f1");
    app.wait_animated(cx, |app, _| {
        scene(app).is_ok_and(|scene| scene["text"] == "" && scene["backgroundMediaType"] == "")
    });
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        let slides = &app.snapshot()["library"][0]["slides"];
        slides[2]["mediaBackgroundBehavior"] == "foreground"
            && slides[3]["actions"][0]["details"]["backgroundBehavior"] == "background"
    });
}
