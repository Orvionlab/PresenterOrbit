use gpui::TestAppContext;
use serde_json::{json, Value};

use super::support::{Folder, Scenario};

#[gpui::test]
fn propresenter_import_preserves_masks_and_opens_imported_masks(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, |_| {});
    let original = app.snapshot()["masks"][0].clone();
    let transfer = Folder::new();
    let source = transfer.0.join("exported");
    orbit_host::export_pro_presenter_workspace_to(&app.workspace.0.join("workspace.json"), &source)
        .unwrap();
    let import = |categories: Value| {
        app.host
            .invoke(
                "selectProPresenterWorkspace",
                vec![
                    json!(source),
                    json!({"libraries":[],"categories":categories}),
                ],
            )
            .unwrap()["workspace"]
            .clone()
    };
    let skipped = import(json!([]));
    assert_eq!(skipped["masks"], json!([original]));
    assert_eq!(skipped["stats"]["importedMaskCount"], 0);
    let workspace = import(json!(["masks"]));
    assert_eq!(workspace["stats"]["importedMaskCount"], 1);
    assert_eq!(workspace["masks"][0], original);
    let mask = &workspace["masks"][1];
    assert_eq!(mask["sourceType"], "propresenter");
    let id = mask["id"].as_str().unwrap();
    assert_eq!(mask["slide"]["id"], format!("{id}-slide"));
    assert!(mask["slide"]["layers"].is_array());
    let folder = app.workspace.0.join("Masks");
    let index: Value =
        serde_json::from_slice(&std::fs::read(folder.join("index.json")).unwrap()).unwrap();
    let mask_path = folder.join(index["masks"][1]["file"].as_str().unwrap());
    let mut old_mask: Value = serde_json::from_slice(&std::fs::read(&mask_path).unwrap()).unwrap();
    let slide = old_mask["slide"].as_object_mut().unwrap();
    slide.remove("id");
    slide.remove("layers");
    std::fs::write(mask_path, serde_json::to_vec(&old_mask).unwrap()).unwrap();
    app.restart(cx);
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut visual = app.visual(cx);
    visual.update(|window, cx| assert!(commands.run("open-mask-canvas", false, window, cx)));
    visual.run_until_parked();
    super::click(&mut visual, &format!("authoring-mask-{id}"));
    assert!(visual
        .debug_bounds("media-transparency-mask-canvas")
        .is_some());
    super::click(&mut visual, "button-theme-shape-tool");
    super::click(&mut visual, "canvas-shape-rectangle");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(350));
    app.wait(cx, |app, _| {
        app.snapshot()["masks"][1]["slide"]["layers"]
            .as_array()
            .is_some_and(|layers| layers.len() == 1)
    });
}

#[gpui::test]
fn propresenter_import_keeps_unselected_stage_layouts_visible_in_the_editor(
    cx: &mut TestAppContext,
) {
    let mut app = Scenario::new(cx, false, |_| {});
    let existing = app.snapshot()["stageLayouts"].as_array().unwrap().clone();
    assert!(!existing.is_empty());
    let source = Folder::new();
    let library = source.0.join("Libraries/Test Library");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(
        library.join("Layered.pro"),
        include_bytes!("../../../../tests/fixtures/propresenter/layered.pro"),
    )
    .unwrap();
    let import = |selection: Value| {
        app.host
            .invoke(
                "selectProPresenterWorkspace",
                vec![json!(source.0), selection],
            )
            .unwrap()["workspace"]
            .clone()
    };
    let without_stage_file =
        import(json!({"libraries":["Test Library"],"categories":["stageLayouts"]}));
    assert_eq!(without_stage_file["stageLayouts"], json!(existing));
    assert_eq!(without_stage_file["stats"]["importedStageLayoutCount"], 0);
    let config = source.0.join("Configuration");
    std::fs::create_dir(&config).unwrap();
    std::fs::write(
        config.join("Stage"),
        include_bytes!("../../../../tests/fixtures/propresenter/object-fill.stage"),
    )
    .unwrap();
    let without_stage_selection = import(json!({"libraries":["Test Library"],"categories":[]}));
    assert_eq!(without_stage_selection["stageLayouts"], json!(existing));
    let stage_id = "7c371df4-cc5d-4c20-8f01-6a298c53ebef";
    let selection = json!({
        "libraries":["Test Library"],
        "categories":["stageLayouts"],
        "items":{"stageLayouts":[stage_id]}
    });
    for _ in 0..2 {
        let imported = import(selection.clone());
        let layouts = imported["stageLayouts"].as_array().unwrap();
        assert_eq!(layouts.len(), existing.len() + 1);
        assert_eq!(layouts[..existing.len()], existing[..]);
        assert_eq!(layouts[existing.len()]["id"], stage_id);
        assert_eq!(
            layouts[existing.len()]["slide"]["id"],
            format!("{stage_id}-slide")
        );
        assert_eq!(imported["stats"]["importedStageLayoutCount"], 1);
    }
    // Older imports kept the embedded slide but did not give it an Orbit editing ID.
    let stage_path = app.workspace.0.join("Configuration/Stage/Stage.json");
    let mut stage: Value = serde_json::from_slice(&std::fs::read(&stage_path).unwrap()).unwrap();
    stage["stageLayouts"][existing.len()]["slide"]
        .as_object_mut()
        .unwrap()
        .remove("id");
    std::fs::write(stage_path, serde_json::to_vec(&stage).unwrap()).unwrap();
    app.restart(cx);
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut visual = app.visual(cx);
    visual.simulate_resize(gpui::size(gpui::px(1440.), gpui::px(3200.)));
    visual.update(|window, cx| assert!(commands.run("open-stage-canvas", false, window, cx)));
    visual.run_until_parked();
    let original = format!("authoring-stage-{}", existing[0]["id"].as_str().unwrap());
    assert!(visual
        .debug_bounds(Box::leak(original.into_boxed_str()))
        .is_some());
    let imported = format!("authoring-stage-{stage_id}");
    super::click(&mut visual, &imported);
    assert!(visual.debug_bounds("canvas-background-enabled").is_some());
}
