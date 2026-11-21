use super::{click, seed, support, Scenario};
use gpui::TestAppContext;
use serde_json::{json, Value};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn call(app: &Scenario, method: &str, args: Vec<Value>) -> Value {
    crate::state::runtime::output_call(&app.host, method, args).unwrap()
}

fn frames(app: &Scenario) -> u64 {
    call(app, "getCaptureStatus", vec![])["framesWritten"]
        .as_u64()
        .unwrap()
}

fn recorded_pixel(path: &Path, seconds: f64) -> [u8; 4] {
    let scene = json!({"presentationThemeSlide":{"width":64,"height":36,"layers":[{
        "type":"Video","mediaType":"video","sourcePath":path,
        "bounds":{"x":0,"y":0,"width":64,"height":36},
        "mediaPlaybackState":{"status":"paused","currentTime":seconds,"clockRevision":seconds.to_bits()}
    }]}});
    let mut compositor = orbit_host::native_compositor::NativeCompositor::new_static_preview();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let frame = compositor
            .try_render_preview_for("freeze-recording", &scene, 64, 36)
            .unwrap();
        if !frame.dynamic_media {
            let offset = (18 * 64 + 32) * 4;
            return frame.pixels[offset..offset + 4].try_into().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "The recorded frame did not decode"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[gpui::test]
fn preview_freeze_holds_recorded_output_while_cues_and_logo_change(cx: &mut TestAppContext) {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/gpui-output-freeze.json"
    ))
    .unwrap();
    let files = support::Folder::new();
    let logo = files.0.join("logo.svg");
    std::fs::write(&logo, fixture["logoSvg"].as_str().unwrap()).unwrap();
    let mut app = Scenario::new(cx, true, |host| {
        seed(host);
        let settings = host.invoke("getSettings", vec![]).unwrap();
        host.invoke("saveSettings", vec![json!({"generalSettings":{
            "logoImagePath":logo,"logoImageUrl":url::Url::from_file_path(&logo).unwrap().as_str()
        }}),settings["settingsRevision"].clone()]).unwrap();
        let path = settings["presenterOrbitWorkspacePath"].clone();
        let snapshot = host
            .invoke("readWorkspaceSnapshot", vec![path.clone()])
            .unwrap();
        let item = &snapshot["workspace"]["library"][0];
        let mut slides = item["slides"].as_array().unwrap().clone();
        for (slide, color) in slides
            .iter_mut()
            .zip(fixture["backgrounds"].as_array().unwrap())
        {
            slide["background"] = json!(color);
            slide["backgroundEnabled"] = json!(true);
            slide["layers"] = json!([]);
            slide.as_object_mut().unwrap().remove("primaryTextLayerId");
            slide["text"] = json!("");
            slide["content"] = json!("");
        }
        host.invoke(
            "mutatePresentationLibrary",
            vec![
                path,
                json!({"type":"mutateItem","itemId":item["id"],"operation":{
                    "type":"setSlideDocument","title":item["title"],"slides":slides
                }}),
                snapshot["revision"].clone(),
            ],
        )
        .unwrap();
    });
    let runtime = app.runtime(cx);
    let mut visual = app.visual(cx);
    let chooser = visual.debug_bounds("preview-screen-controls").unwrap();
    let clear = visual.debug_bounds("preview-clear-logo").unwrap();
    let freeze = visual.debug_bounds("preview-freeze").unwrap();
    assert!(chooser.size.width > clear.size.width);
    assert!((f32::from(clear.size.width - freeze.size.width)).abs() <= 1.);
    let sidebar = visual.debug_bounds("region-right").unwrap();
    assert!((f32::from(sidebar.right() - freeze.right())).abs() <= 1.);
    click(&mut visual, "document-0");
    app.wait(cx, |app, cx| {
        app.visual(cx).debug_bounds("cue-thumbnail-1").is_some()
    });
    click(&mut visual, "cue-thumbnail-0");
    visual.simulate_keystrokes("enter");
    let screen = cx.update(|cx| {
        crate::output::view::screens(runtime.output.read(cx).snapshot.as_ref().unwrap())[0]
            .0
            .clone()
    });
    app.wait(cx, |app, _| {
        call(app, "getOutputContent", vec![json!(screen)])["presentationThemeSlide"]["background"]
            == fixture["backgrounds"][0]
    });
    let preset = json!({"preset":{"id":"freeze","title":"Freeze flow","sourceScreenId":screen,
        "width":64,"height":36,"fps":10,"bitrateKbps":1000,"outputDirectory":files.0}});
    let recording = call(&app, "startCaptureResolved", vec![preset.clone()]);
    app.wait(cx, |app, _| frames(app) >= 3);
    click(&mut visual, "preview-freeze");
    app.wait(cx, |app, _| {
        call(app, "getOutputContent", vec![json!(screen)])["outputFrozen"] == true
    });
    assert_eq!(call(&app, "getNdiStatus", vec![])["frozen"], true);
    let held_start = frames(&app);
    click(&mut visual, "cue-thumbnail-1");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, _| {
        call(app, "getOutputContent", vec![json!(screen)])["presentationThemeSlide"]["background"]
            == fixture["backgrounds"][1]
    });
    let preview = app.host.render_output_bgra_frame(&screen, 64, 36).unwrap();
    assert_eq!(
        &preview.pixels[(18 * 64 + 32) * 4..(18 * 64 + 32) * 4 + 4],
        &[255, 0, 0, 255]
    );
    click(&mut visual, "preview-clear-logo");
    app.wait(cx, |app, _| {
        call(app, "getOutputContent", vec![json!(screen)])["backgroundMediaPath"]
            == logo.to_string_lossy().as_ref()
    });
    let preview = app.host.render_output_bgra_frame(&screen, 64, 36).unwrap();
    assert_eq!(
        &preview.pixels[(18 * 64 + 32) * 4..(18 * 64 + 32) * 4 + 4],
        &[0, 255, 0, 255]
    );
    click(&mut visual, "look-layer-media");
    app.wait(cx, |app, _| {
        call(app, "getOutputContent", vec![json!(screen)])["backgroundMediaPath"] == ""
    });
    click(&mut visual, "cue-thumbnail-1");
    visual.simulate_keystrokes("enter");
    app.wait(cx, |app, _| {
        call(app, "getOutputContent", vec![json!(screen)])["presentationThemeSlide"]["background"]
            == fixture["backgrounds"][1]
    });
    app.wait(cx, |app, _| frames(app) >= held_start + 5);
    let held_end = frames(&app);
    let accepted = call(
        &app,
        "updateOutputScene",
        vec![
            json!({"version":2,"kind":"patch","sessionId":"freeze-validation","clientSequence":1,
        "baseRevision":9_007_199_254_740_991_u64,"patch":{}}),
        ],
    );
    for value in [json!(null), json!("true"), json!(1)] {
        let mut invalid = accepted["snapshot"].clone();
        invalid["outputFrozen"] = value;
        assert!(crate::state::runtime::output_call(&app.host,"updateOutputScene",vec![json!({"version":2,"kind":"full",
            "sessionId":"invalid-freeze","clientSequence":1,"baseRevision":accepted["revision"],"snapshot":invalid})]).is_err());
    }
    click(&mut visual, "preview-freeze");
    app.wait(cx, |app, _| {
        call(app, "getOutputContent", vec![json!(screen)])["outputFrozen"] == false
    });
    assert_eq!(call(&app, "getNdiStatus", vec![])["frozen"], false);
    let resumed_start = frames(&app);
    app.wait(cx, |app, _| frames(app) >= resumed_start + 5);
    let resumed_end = frames(&app);
    let stopped = call(&app, "stopCapture", vec![]);
    assert!(stopped["error"].is_null(), "{stopped}");
    let path = Path::new(recording["filePath"].as_str().unwrap());
    let red = recorded_pixel(path, 0.);
    assert!(red[2] > 220 && red[0] < 30, "{red:?}");
    let held = recorded_pixel(path, (held_start + held_end) as f64 / 20.);
    assert!(held[2] > 220 && held[0] < 30, "{held:?}");
    let resumed = recorded_pixel(path, (resumed_start + resumed_end) as f64 / 20.);
    assert!(resumed[0] > 220 && resumed[2] < 30, "{resumed:?}");
    click(&mut visual, "preview-freeze");
    app.wait(cx, |app, _| {
        call(app, "getOutputContent", vec![json!(screen)])["outputFrozen"] == true
    });
    let error = crate::state::runtime::output_call(&app.host, "startCaptureResolved", vec![preset])
        .unwrap_err();
    assert!(error.contains("Resume output"), "{error}");
    app.restart(cx);
    assert!(app.snapshot().get("outputFrozen").is_none());
    let runtime = app.runtime(cx);
    assert_eq!(
        cx.update(|cx| runtime.output.read(cx).live["outputFrozen"].clone()),
        false
    );
}
