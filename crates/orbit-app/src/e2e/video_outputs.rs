use super::{click, seed, support, Scenario};
use gpui::{AppContext, Modifiers, TestAppContext, VisualTestContext};
use serde_json::{json, Value};

fn window(app: &Scenario, cx: &mut TestAppContext) -> VisualTestContext {
    let runtime = app.runtime(cx);
    let window = cx.add_window(move |window, cx| {
        let view = cx.new(|cx| crate::settings::SettingsView::screens(runtime, cx));
        orbit_ui::controls::Root::new(view, window, cx)
    });
    let visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual
}

fn choose(visual: &mut VisualTestContext, selector: &str, index: usize) {
    click(visual, selector);
    click(visual, &format!("select-option-{index}"));
}

fn tab(visual: &mut VisualTestContext, index: usize) {
    let bounds = visual.debug_bounds("output-device-tabs").unwrap();
    visual.simulate_click(
        gpui::point(
            bounds.left() + bounds.size.width * (index as f32 + 0.5) / 4.,
            bounds.center().y,
        ),
        Modifiers::default(),
    );
    visual.run_until_parked();
}

fn number(visual: &mut VisualTestContext, selector: &str, value: &str) {
    click(visual, selector);
    support::primary_shortcut(visual, "a");
    visual.simulate_input(value);
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
}

fn assert_virtual_formats(visual: &mut VisualTestContext, submenu: &str) {
    let viewport = visual.update(|window, _| window.viewport_size());
    let parent = visual
        .debug_bounds(Box::leak(format!("menu-item-{submenu}").into_boxed_str()))
        .unwrap();
    let mut last_bottom = gpui::px(0.);
    for preset in [
        "720p59.94",
        "1080p30",
        "1080p50",
        "1080p59.94",
        "1080p60",
        "4Kp25",
        "4Kp30",
        "4Kp50",
        "4Kp60",
        "Custom Size…",
    ] {
        let selector = Box::leak(format!("menu-item-{preset}").into_boxed_str());
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{preset} is missing"));
        assert!(
            bounds.left() >= gpui::px(0.) && bounds.right() <= viewport.width,
            "{preset} {bounds:?}"
        );
        assert!(
            bounds.top() >= last_bottom && bounds.bottom() <= viewport.height,
            "{preset} {bounds:?}"
        );
        assert!(
            bounds.size.width < parent.size.width,
            "{preset} is as wide as the parent menu"
        );
        last_bottom = bounds.bottom();
    }
}

fn output(app: &Scenario, id: &str) -> Result<Value, String> {
    crate::state::runtime::output_call(&app.host, "getOutputContent", vec![json!(id)])
}

fn output_enabled(app: &Scenario, id: &str) -> Result<Option<bool>, String> {
    // A deliberately unmatched base reads the accepted scene without committing another scene.
    let state = crate::state::runtime::output_call(
        &app.host,
        "updateOutputScene",
        vec![json!({
            "version":2, "kind":"patch", "sessionId":"e2e-observe-screens", "clientSequence":1,
            "baseRevision":9_007_199_254_740_991_u64, "patch":{}
        })],
    )?;
    if state["cacheMiss"] != true {
        return Err("The screen-state probe unexpectedly committed a scene.".into());
    }
    Ok(state["snapshot"]["screenConfig"][id]["enabled"].as_bool())
}

#[gpui::test]
fn video_output_layouts_calibrate_deliver_and_round_trip(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, true, |host| {
        seed(host);
        let settings = host.invoke("getSettings", vec![]).unwrap();
        let path = settings["presenterOrbitWorkspacePath"].clone();
        let loaded = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let (id, config) = settings["screenConfig"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, config)| config["type"] == "audience")
            .unwrap();
        let layout = orbit_host::output_layout::OutputLayout::from_screen(id, config).unwrap();
        host.invoke(
            "mutateScreenConfig",
            vec![
                path,
                json!({"type":"update", "screenId":id,
            "patch":{"enabled":true, "outputLayout":layout}}),
                settings["settingsRevision"].clone(),
                loaded["revision"].clone(),
            ],
        )
        .unwrap();
        let settings = host.invoke("getSettings", vec![]).unwrap();
        let payload = crate::settings::persist::general_payload(
            &settings,
            &json!({"autoEnableScreensAtLaunch":false}),
        );
        host.invoke(
            "saveSettings",
            vec![payload, settings["settingsRevision"].clone()],
        )
        .unwrap();
    });
    let runtime = app.runtime(cx);
    app.wait(cx, |app, cx| {
        app.host
            .state::<crate::output_process::OutputRoute>()
            .0
            .get()
            .is_some_and(|route| route.attached())
            && cx.update(|cx| runtime.output.read(cx).snapshot.is_some())
    });
    let screen = cx.update(|cx| {
        crate::settings::persist::shown(&runtime, cx)["screenConfig"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, screen)| screen["type"] == "audience")
            .unwrap()
            .0
            .clone()
    });
    let mut visual = window(&app, cx);
    click(&mut visual, &format!("output-screen-{screen}"));
    app.wait(cx, |app, _| {
        app.snapshot()["screenConfig"][&screen]["outputLayout"]["outputs"][0]["target"]["kind"]
            == "placeholder"
            && output_enabled(app, &screen).is_ok_and(|enabled| enabled == Some(false))
    });
    assert_eq!(app.snapshot()["screenConfig"][&screen]["enabled"], true);
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut main_visual = app.visual(cx);
    let release = support::hold_workspace_writes(&app, cx);
    let switch = format!("screen-switch-{screen}");
    // Saved activation is on, but the launch policy kept the actual output off.
    main_visual.update(|window, cx| assert!(commands.run("toggle-audience", false, window, cx)));
    app.wait(cx, |app, _| {
        output_enabled(app, &screen).is_ok_and(|enabled| enabled == Some(true))
    });
    assert_eq!(app.snapshot()["screenConfig"][&screen]["enabled"], true);
    click(&mut visual, &switch);
    app.wait(cx, |app, _| {
        output_enabled(app, &screen).is_ok_and(|enabled| enabled == Some(false))
    });
    for _ in 0..3 {
        click(&mut visual, &switch);
    }
    app.wait(cx, |app, _| {
        output_enabled(app, &screen).is_ok_and(|enabled| enabled == Some(true))
    });
    assert_eq!(app.snapshot()["screenConfig"][&screen]["enabled"], true);
    for expected in [false, true, false] {
        main_visual
            .update(|window, cx| assert!(commands.run("toggle-audience", false, window, cx)));
        assert_eq!(
            cx.update(
                |cx| crate::settings::persist::shown(&runtime, cx)["screenConfig"][&screen]
                    ["enabled"]
                    .as_bool()
            ),
            Some(expected)
        );
    }
    app.wait(cx, |app, _| {
        output_enabled(app, &screen).is_ok_and(|enabled| enabled == Some(false))
    });
    app.wait(cx, |_, cx| {
        cx.update(|cx| !crate::output::windows::group_live(&runtime, false, cx))
    });
    release.send(()).unwrap();
    app.wait(cx, |app, cx| {
        cx.update(|cx| {
            runtime
                .output
                .read(cx)
                .settings_drafts
                .section_patch(&runtime.content.read(cx).workspace_path, "screenConfig")
                .is_empty()
        }) && app.snapshot()["screenConfig"][&screen]["enabled"] == false
    });
    cx.update(|cx| {
        runtime.output.update(cx, |output, cx| {
            output.set_displays(json!((-1..=1)
                .map(|index| json!({
                    "id":format!("test-monitor-{index}"),
                    "label":format!("System display {}", index + 2),
                    "primary":index == 0,
                    "size":{"width":1920,"height":1080},
                    "bounds":{"x":index * 1920,"y":0,"width":1920,"height":1080}
                }))
                .collect::<Vec<_>>()));
            cx.notify();
        })
    });
    visual.run_until_parked();
    click(&mut visual, "output-target-0-fill");
    click(&mut visual, "menu-item-New Placeholder");
    assert_virtual_formats(&mut visual, "New Placeholder");
    click(&mut visual, "menu-item-720p59.94");
    app.wait(cx, |app, _| {
        let target =
            &app.snapshot()["screenConfig"][&screen]["outputLayout"]["outputs"][0]["target"];
        target["kind"] == "placeholder" && target["width"] == 1280 && target["frameRate"] == 59.94
    });
    click(&mut visual, "output-target-0-fill");
    click(&mut visual, "menu-item-New Placeholder");
    click(&mut visual, "menu-item-Custom Size…");
    for (selector, value) in [
        ("output-format-width", "1600"),
        ("output-format-height", "900"),
        ("output-format-rate", "50"),
    ] {
        click(&mut visual, selector);
        support::primary_shortcut(&mut visual, "a");
        visual.simulate_input(value);
        visual.run_until_parked();
    }
    click(&mut visual, "button-output-format-apply");
    app.wait(cx, |app, _| {
        let target =
            &app.snapshot()["screenConfig"][&screen]["outputLayout"]["outputs"][0]["target"];
        target["width"] == 1600
            && target["height"] == 900
            && target["frameRate"].as_f64() == Some(50.)
    });
    // Monitor dimensions come from hardware selection; virtual destinations use a format popup.
    cx.update(|cx| {
        runtime.output.update(cx, |output, cx| {
            output.set_displays(json!([{
                "id":"e2e-monitor", "label":"E2E monitor", "primary":true,
                "size":{"width":1920,"height":1080},
                "bounds":{"x":0,"y":0,"width":1920,"height":1080}
            }]));
            cx.notify();
        })
    });
    visual.run_until_parked();
    click(&mut visual, "output-target-0-fill");
    click(&mut visual, "output-monitor-e2e-monitor");
    assert!(visual.debug_bounds("output-0-fill-fixed-size").is_some());
    for name in ["width", "height", "frameRate"] {
        assert!(visual
            .debug_bounds(Box::leak(
                format!("output-{screen}-0-fill-{name}-number").into_boxed_str()
            ))
            .is_none());
    }
    let release = support::hold_workspace_writes(&app, cx);
    choose(&mut visual, "output-layout-mode", 2);
    let grouped = cx.update(|cx| {
        crate::settings::persist::shown(&runtime, cx)["screenConfig"][&screen]["outputLayout"]
            .clone()
    });
    assert_eq!(grouped["mode"], "grouped");
    assert_eq!(grouped["outputs"].as_array().unwrap().len(), 4);
    for index in 0..4 {
        assert!(visual
            .debug_bounds(Box::leak(format!("output-device-{index}").into_boxed_str()))
            .is_some());
    }
    let ids: Vec<_> = grouped["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|device| {
            orbit_host::output_layout::physical_id(&screen, device["id"].as_str().unwrap())
        })
        .collect();
    release.send(()).unwrap();
    app.wait(cx, |app, _| {
        output(app, &ids[3]).is_ok_and(|scene| {
            scene["canvasViewport"]["x"] == 1920 && scene["canvasViewport"]["y"] == 1080
        })
    });
    assert_eq!(
        app.snapshot()["screenConfig"][&screen]["outputLayout"]["rows"],
        2
    );
    assert!(!app.snapshot()["screenConfig"]
        .as_object()
        .unwrap()
        .keys()
        .any(|id| id.contains("::output::")));

    // Mirroring uses the full logical image on each output, instead of slicing the canvas.
    choose(&mut visual, "output-layout-mode", 1);
    app.wait(cx, |app, _| {
        output(app, &ids[1]).is_ok_and(|scene| {
            scene["canvasViewport"]["x"] == 0 && scene["canvasViewport"]["canvasWidth"] == 1920
        })
    });
    choose(&mut visual, "output-layout-mode", 3);
    click(&mut visual, "output-screen-color-enabled");
    click(&mut visual, "output-screen-color");
    click(&mut visual, "color-swatch-#ffffff");
    click(&mut visual, "output-screen-color");
    click(&mut visual, "output-blend-0");
    number(
        &mut visual,
        &format!("output-{screen}-blend-0-width-number"),
        "240",
    );
    number(
        &mut visual,
        &format!("output-{screen}-blend-0-true-gamma-number"),
        "0.25",
    );
    click(&mut visual, "output-device-0");
    click(&mut visual, "output-target-0-fill");
    click(&mut visual, "menu-item-New NDI");
    assert_virtual_formats(&mut visual, "New NDI");
    click(&mut visual, "menu-item-1080p60");
    assert_eq!(
        cx.update(
            |cx| crate::settings::persist::shown(&runtime, cx)["screenConfig"][&screen]
                ["outputLayout"]["outputs"][0]["target"]["kind"]
                .clone()
        ),
        "ndi"
    );
    click(&mut visual, "button-output-size-0-fill");
    assert!(visual.debug_bounds("output-format-width").is_some());
    click(&mut visual, "button-output-format-cancel");
    tab(&mut visual, 1);
    number(
        &mut visual,
        &format!("output-{screen}-0-color-red-number"),
        "0.25",
    );
    tab(&mut visual, 2);
    click(&mut visual, "output-corner-enabled");
    number(
        &mut visual,
        &format!("output-{screen}-0-topLeft-x-number"),
        "-20",
    );
    tab(&mut visual, 3);
    choose(&mut visual, "output-alpha-mode", 1);
    click(&mut visual, "output-target-0-key");
    click(&mut visual, "menu-item-New NDI");
    click(&mut visual, "menu-item-1080p60");
    app.wait(cx, |app, _| {
        app.snapshot()["screenConfig"][&screen]["outputLayout"]["outputs"][0]["alpha"]["target"]
            ["kind"]
            == "ndi"
    });
    let key = format!("{}::key", ids[0]);
    app.wait(cx, |app, _| {
        output(app, &key).is_ok_and(|scene| scene["keyFillSignal"] == "key")
    });
    let saved = app.snapshot()["screenConfig"][&screen]["outputLayout"].clone();
    assert_eq!(saved["blends"][0]["width"], 240);
    assert_eq!(saved["blends"][0]["left"]["gamma"], 0.25);
    assert_eq!(saved["outputs"][0]["color"]["red"], 0.25);
    app.wait(cx, |app, _| {
        output(app, &ids[1]).is_ok_and(|scene| {
            scene["canvasViewport"]["canvasWidth"] == 3600 && scene["canvasViewport"]["x"] == 1680
        })
    });
    assert_eq!(
        output(&app, &ids[0]).unwrap()["keyFillSignal"],
        "fill-premultiplied"
    );

    app.wait(cx, |app, _| {
        app.host
            .invoke("getOutputContent", vec![json!(ids[1])])
            .is_ok_and(|scene| scene["backgroundCss"] == saved["screenColor"])
    });
    let left = app.host.render_output_bgra_frame(&ids[0], 32, 16).unwrap();
    let right = app.host.render_output_bgra_frame(&ids[1], 32, 16).unwrap();
    let sample = |frame: &Vec<u8>, x: usize| frame[(8 * 32 + x) * 4];
    assert!(
        sample(&left.pixels, 8) > 240 && sample(&left.pixels, 31) < 100,
        "left blend did not attenuate the overlapping edge"
    );
    assert!(
        sample(&right.pixels, 0) < 100 && sample(&right.pixels, 24) > 240,
        "right blend did not attenuate the overlapping edge"
    );

    // Invalid edits are rejected by the real host boundary without changing the saved workspace.
    for case in 0..4 {
        let mut invalid = saved.clone();
        match case {
            0 => invalid["rows"] = json!(0),
            1 => invalid["outputs"][1]["id"] = invalid["outputs"][0]["id"].clone(),
            2 => invalid["blends"][0]["width"] = json!(u64::MAX),
            _ => invalid["unexpected"] = json!(true),
        }
        let settings = app.host.invoke("getSettings", vec![]).unwrap();
        let path = settings["presenterOrbitWorkspacePath"].clone();
        let loaded = app
            .host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        assert!(app
            .host
            .invoke(
                "mutateScreenConfig",
                vec![
                    path,
                    json!({"type":"update", "screenId":screen, "patch":{"outputLayout":invalid}}),
                    settings["settingsRevision"].clone(),
                    loaded["revision"].clone()
                ]
            )
            .is_err());
        assert_eq!(
            app.snapshot()["screenConfig"][&screen]["outputLayout"],
            saved
        );
    }

    let transfer = support::Folder::new();
    let exported = transfer.0.join("exported");
    orbit_host::export_pro_presenter_workspace_to(
        &app.workspace.0.join("workspace.json"),
        &exported,
    )
    .unwrap();
    let selection =
        orbit_host::ImportSelection::parse(&json!({"libraries":[],"categories":["settings"]}))
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
    let imported_layout = imported["screenConfig"]
        .as_object()
        .unwrap()
        .values()
        .find(|config| config["outputLayout"]["mode"] == "edge-blend")
        .unwrap()["outputLayout"]
        .clone();
    assert_eq!(imported_layout["blends"], saved["blends"]);
    assert_eq!(
        imported_layout["outputs"][0]["target"],
        saved["outputs"][0]["target"]
    );
    assert_eq!(
        imported_layout["outputs"][0]["color"],
        saved["outputs"][0]["color"]
    );
    assert_eq!(
        imported_layout["outputs"][0]["alpha"],
        saved["outputs"][0]["alpha"]
    );
    let before = saved["outputs"][0]["cornerPin"]["topLeft"]["x"]
        .as_f64()
        .unwrap();
    let after = imported_layout["outputs"][0]["cornerPin"]["topLeft"]["x"]
        .as_f64()
        .unwrap();
    assert!((before - after).abs() < 0.000001);
    app.restart(cx);
    let mut visual = window(&app, cx);
    click(&mut visual, &format!("output-screen-{screen}"));
    assert!(visual.debug_bounds("output-blend-0").is_some());
    assert_eq!(
        app.snapshot()["screenConfig"][&screen]["outputLayout"],
        saved
    );
}

#[gpui::test]
fn stage_picker_defaults_and_switches_the_output_layout(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, true, |_| {});
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut main_visual = app.visual(cx);
    let layouts = app.snapshot()["stageLayouts"].as_array().unwrap().clone();
    let stage = "stage-1";
    app.wait(cx, |app, _| {
        output(app, stage).is_ok_and(|scene| scene["stageSlide"]["id"] == layouts[0]["slide"]["id"])
    });
    main_visual.update(|window, cx| assert!(commands.run("view-stage", false, window, cx)));
    main_visual.run_until_parked();
    click(&mut main_visual, "stage-picker-stage-1");
    assert!(main_visual.debug_bounds("stage-pick-stage-1-").is_none());
    let next_layout = layouts[1]["id"].as_str().unwrap();
    click(
        &mut main_visual,
        &format!("stage-pick-stage-1-{next_layout}"),
    );
    app.wait(cx, |app, _| {
        app.snapshot()["stageScreenAssignments"][0]["layoutId"] == next_layout
            && output(app, stage)
                .is_ok_and(|scene| scene["stageSlide"]["id"] == layouts[1]["slide"]["id"])
    });
    let mut thumbnail = layouts[0]["slide"].clone();
    thumbnail["layers"].as_array_mut().unwrap().extend([
        json!({
            "id":"capture-indicator", "type":"Shape", "shape":"rectangle",
            "x":0,"y":0,"width":100,"height":100,"fillType":"capture-status-color",
            "captureStatusColors":orbit_host::authoring::default_capture_status_colors(),
        }),
        json!({
            "id":"stage-screen-preview", "type":"Shape", "shape":"rectangle",
            "x":100,"y":0,"width":100,"height":100,"fillType":"screen-preview",
            "previewScreenId":"another-output",
        }),
    ]);
    orbit_host::native_compositor::render_stage_layout_still(&thumbnail, 128, 72).unwrap();

    let directory =
        crate::output_process::protocol::data_directory(&app.host.path().app_data_dir().unwrap());
    assert!(crate::output_process::protocol::read_endpoint(&directory)
        .unwrap()
        .is_some());
    cx.update(|cx| {
        crate::foreground::stop_output_on_quit(cx, app.host.clone());
        cx.shutdown();
    });
    assert!(crate::output_process::protocol::read_endpoint(&directory)
        .unwrap()
        .is_none());
    let (sender, _) = futures::channel::mpsc::unbounded();
    crate::shell::start_output_link_with(&app.host, sender, "normal".into(), |_| {
        panic!("a stopped controller must not start a new output link")
    })
    .unwrap();
}

#[gpui::test]
fn system_display_detection_remains_available_without_an_output_connection(
    cx: &mut TestAppContext,
) {
    let mut app = Scenario::new(cx, true, seed);
    let runtime = app.runtime(cx);
    app.host
        .state::<crate::output_process::OutputRoute>()
        .0
        .get()
        .unwrap()
        .shutdown();
    let options = app.host.invoke("getOutputOptions", vec![]).unwrap();
    assert!(options["displays"].is_array());
    app.host
        .emit(
            "output:link-error",
            json!("Another Presenter Orbit controls the output."),
        )
        .unwrap();
    app.wait(cx, |_, cx| {
        cx.update(|cx| runtime.output.read(cx).link_error.is_some())
    });
    cx.update(|cx| {
        runtime.output.update(cx, |output, _| {
            output.displays = std::sync::Arc::new(json!([{
                "id":"previous-display", "label":"Previously connected", "primary":true,
                "size":{"width":1920,"height":1080},
                "bounds":{"x":0,"y":0,"width":1920,"height":1080}
            }]));
        });
        crate::output::windows::refresh_displays(&runtime, cx);
    });
    app.wait(cx, |_, cx| {
        cx.update(|cx| *runtime.output.read(cx).displays == options["displays"])
    });
    assert_eq!(
        cx.update(|cx| runtime.output.read(cx).link_error.clone()),
        Some("Another Presenter Orbit controls the output.".into())
    );
    app.host.refresh_output_displays().unwrap();
    let mut settings = window(&app, cx);
    let screen = cx.update(|cx| {
        crate::settings::persist::shown(&runtime, cx)["screenConfig"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone()
    });
    click(&mut settings, &format!("output-screen-{screen}"));
    assert!(!app
        .host
        .state::<crate::output_process::OutputRoute>()
        .0
        .get()
        .unwrap()
        .attached());
}
