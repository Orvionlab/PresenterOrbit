use gpui::TestAppContext;
use serde_json::{json, Value};

use super::support::{Folder, Scenario};

#[gpui::test]
fn presentation_file_batch_uses_one_destination_and_reports_failures(cx: &mut TestAppContext) {
    let app = Scenario::new(cx, false, super::seed);
    let transfer = Folder::new();
    let source = transfer.0.join("Media.pro");
    std::fs::write(
        &source,
        include_bytes!("../../../../tests/fixtures/propresenter/media.pro"),
    )
    .unwrap();
    let missing = transfer.0.join("Missing.pro");
    let text = transfer.0.join("Imported reading.txt");
    std::fs::write(
        &text,
        "First paragraph

Second paragraph",
    )
    .unwrap();

    let path = json!(app.workspace.0.join("workspace.json"));
    let snapshot = app
        .host
        .invoke("readWorkspaceSnapshot", vec![path.clone()])
        .unwrap();
    let snapshot = app
        .host
        .invoke(
            "mutatePresentationLibrary",
            vec![
                path.clone(),
                json!({"type": "create", "baseTitle": "Import destination"}),
                snapshot["revision"].clone(),
            ],
        )
        .unwrap();
    let workspace = &snapshot["workspace"];
    let library_id = &snapshot["createdLibraryId"];
    let playlist_id = &workspace["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["title"] == "Sunday")
        .unwrap()["id"];
    let args = |sources: Value, playlist_id: &Value| {
        vec![
            path.clone(),
            json!({"sources": sources, "libraryId": library_id, "playlistId": playlist_id}),
            snapshot["revision"].clone(),
        ]
    };
    assert!(app
        .host
        .invoke(
            "importPresentationFiles",
            args(json!(["relative.pro"]), playlist_id)
        )
        .unwrap_err()
        .contains("absolute"));
    assert!(app
        .host
        .invoke(
            "importPresentationFiles",
            args(json!([source]), &json!("missing"))
        )
        .unwrap_err()
        .contains("playlist"));
    let answer = app
        .host
        .invoke(
            "importPresentationFiles",
            args(json!([source, missing, text]), playlist_id),
        )
        .unwrap();
    assert_eq!(answer["importedFiles"], 2);
    let failures = answer["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].as_str().unwrap().contains("Missing.pro"));
    assert!(answer["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning.as_str().unwrap().contains("is unavailable")));

    let imported = app.snapshot();
    let destination = imported["libraries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|library| library["id"] == *library_id)
        .unwrap();
    let items = destination["itemIds"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let presentations = imported["library"].as_array().unwrap();
    assert!(presentations
        .iter()
        .any(|item| item["id"] == items[0] && !item["slides"].as_array().unwrap().is_empty()));
    let reading = presentations
        .iter()
        .find(|item| item["id"] == items[1])
        .unwrap();
    assert_eq!(reading["title"], "Imported reading");
    assert_eq!(reading["slides"].as_array().unwrap().len(), 2);
    let playlist = imported["playlist"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == *playlist_id)
        .unwrap();
    assert!(playlist["itemIds"].as_array().unwrap().ends_with(items));
}

#[gpui::test]
fn propresenter_workspace_import_warns_about_partial_outputs_and_relative_media(
    cx: &mut TestAppContext,
) {
    let app = Scenario::new(cx, false, |host| {
        let settings = host.invoke("getSettings", vec![]).unwrap();
        host.invoke(
            "saveSettings",
            vec![
                json!({"groupSettings":{
                    "1-1-1": {"name":"1-1-1","key":""},
                    "custom-verse": {"name":"Verse 8","key":"V"}
                }}),
                settings["settingsRevision"].clone(),
            ],
        )
        .unwrap();
    });
    let source = Folder::new();
    let library = source.0.join("Libraries/Imported");
    std::fs::create_dir_all(&library).unwrap();

    // Keep each protobuf string the same byte length so enclosing fields stay valid.
    let mut presentation =
        include_bytes!("../../../../tests/fixtures/propresenter/authentic-layered-21.pro").to_vec();
    for file in ["orbit-int12-image.png", "orbit-int12-video.mp4"] {
        let original = format!(r"C:\Users\Demo\Documents\ProPresenter\Media\Assets\{file}");
        let folder = format!(
            "Media/Assets/{}",
            "a".repeat(original.len() - file.len() - "Media/Assets//".len())
        );
        let relative = format!("{folder}/{file}");
        let offsets: Vec<_> = presentation
            .windows(original.len())
            .enumerate()
            .filter_map(|(offset, bytes)| (bytes == original.as_bytes()).then_some(offset))
            .collect();
        assert!(!offsets.is_empty());
        for offset in offsets {
            presentation[offset..offset + original.len()].copy_from_slice(relative.as_bytes());
        }
        if file.ends_with(".png") {
            std::fs::create_dir_all(source.0.join(folder)).unwrap();
            std::fs::write(
                source.0.join(&relative),
                include_bytes!("../../../../tests/fixtures/propresenter/orbit-int12-image.png"),
            )
            .unwrap();
        }
    }
    std::fs::write(library.join("Media.pro"), presentation).unwrap();

    // Workspace.screen -> single arrangement -> device -> subscreen_unit_rect.
    fn message(field: u8, bytes: &[u8]) -> Vec<u8> {
        assert!(bytes.len() < 128);
        [vec![(field << 3) | 2, bytes.len() as u8], bytes.to_vec()].concat()
    }
    let first_cue = b"cue-one";
    let second_cue = b"cue-two";
    let verse = [
        message(1, b"571eed42-9b52-4c48-9fd1-879153887992"),
        message(2, b"Verse 8"),
    ]
    .concat();
    let cue_group = [
        message(1, &verse),
        message(2, &message(1, first_cue)),
        message(2, &message(1, second_cue)),
    ]
    .concat();
    let named_cues = [
        message(3, b"Named cues"),
        message(12, &cue_group),
        message(
            13,
            &[message(1, first_cue), message(2, b"Slide 1")].concat(),
        ),
        message(
            13,
            &[
                message(1, second_cue),
                message(2, b"Slide 2"),
                message(
                    10,
                    &[vec![0x48, 11], message(3, &message(2, b"Slide 1"))].concat(),
                ),
            ]
            .concat(),
        ),
    ]
    .concat();
    std::fs::write(library.join("Named cues.pro"), named_cues).unwrap();
    let unused_group = [
        message(1, b"625bbbd3-6d76-44ad-84ab-9392fd2a6e5e"),
        message(2, b"42"),
    ]
    .concat();
    std::fs::create_dir_all(source.0.join("Configuration")).unwrap();
    std::fs::write(
        source.0.join("Configuration/Groups"),
        [message(1, &verse), message(1, &unused_group)].concat(),
    )
    .unwrap();
    let size = [
        vec![9],
        0.5_f64.to_le_bytes().to_vec(),
        vec![17],
        1_f64.to_le_bytes().to_vec(),
    ]
    .concat();
    let rectangle = [message(1, &[]), message(2, &size)].concat();
    let screen = [
        message(1, b"Partial screen"),
        message(3, &message(1, &message(8, &rectangle))),
        vec![0x48, 1],
    ]
    .concat();
    std::fs::write(source.0.join("Workspace"), message(1, &screen)).unwrap();

    let result = app
        .host
        .invoke(
            "selectProPresenterWorkspace",
            vec![
                json!(source.0),
                json!({"libraries":["Imported"], "categories":["settings","groups"]}),
            ],
        )
        .unwrap();
    let workspace = &result["workspace"];
    let presentations = workspace["library"].as_array().unwrap();
    assert_eq!(presentations.len(), 2);
    let slides = &presentations
        .iter()
        .find(|item| item["title"] == "Named cues")
        .unwrap()["slides"];
    assert_eq!(slides[0]["name"], "Slide 1");
    assert!(slides[0].get("label").is_none());
    assert_eq!(slides[1]["name"], "Slide 2");
    assert_eq!(slides[1]["label"], "Slide 1");
    assert_eq!(slides[0]["groupId"], "verse-8");
    assert_eq!(slides[1]["groupId"], "verse-8");
    let groups = workspace["groupSettings"].as_object().unwrap();
    assert_eq!(groups["verse-8"]["name"], "Verse 8");
    assert_eq!(groups["verse-8"]["key"], "V");
    assert_eq!(groups["42"]["name"], "42");
    for id in [
        "1-1-1",
        "custom-verse",
        "571eed42-9b52-4c48-9fd1-879153887992",
    ] {
        assert!(!groups.contains_key(id), "unexpected group {id}");
    }
    assert_eq!(workspace["screenConfig"]["audience-1"]["enabled"], false);
    let warnings = workspace["importWarnings"].as_array().unwrap();
    assert!(warnings.iter().any(|warning| warning
        .as_str()
        .unwrap()
        .contains("partial hardware output target")));
    assert!(warnings.iter().any(|warning| {
        let warning = warning.as_str().unwrap();
        warning.contains("orbit-int12-video.mp4") && warning.contains("is unavailable")
    }));
    assert!(!warnings.iter().any(|warning| {
        let warning = warning.as_str().unwrap();
        warning.contains("orbit-int12-image.png") && warning.contains("is unavailable")
    }));
    assert!(workspace["library"]
        .to_string()
        .contains("ProPresenter Imports"));
}

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
