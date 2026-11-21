use gpui::TestAppContext;
use serde_json::{json, Value};

use super::support::{click, Folder, Scenario};

#[gpui::test]
fn imported_slide_formatting_stays_out_of_the_theme_editor_catalog(cx: &mut TestAppContext) {
    let mut app = Scenario::new(cx, false, |_| {});
    let existing = app.snapshot()["themes"].as_array().unwrap().len();
    assert!(existing > 0);
    let source = Folder::new();
    let library = source.0.join("Libraries/Test Library");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(
        library.join("Layered.pro"),
        include_bytes!("../../../../tests/fixtures/propresenter/layered.pro"),
    )
    .unwrap();
    let import = |categories: Value| {
        app.host
            .invoke(
                "selectProPresenterWorkspace",
                vec![
                    json!(source.0),
                    json!({"libraries":["Test Library"],"categories":categories}),
                ],
            )
            .unwrap()["workspace"]
            .clone()
    };
    let presentations_only = import(json!([]));
    assert_eq!(presentations_only["stats"]["importedThemeCount"], 0);
    assert_eq!(
        orbit_host::authoring::reusable_themes(&presentations_only["themes"]).count(),
        existing,
    );
    let native = source.0.join("Themes/Native");
    std::fs::create_dir_all(&native).unwrap();
    std::fs::write(
        native.join("Theme"),
        include_bytes!("../../../../tests/fixtures/propresenter/Theme"),
    )
    .unwrap();
    let workspace = import(json!(["themes"]));
    assert_eq!(workspace["stats"]["importedThemeCount"], 1);
    let theme = orbit_host::authoring::reusable_themes(&workspace["themes"])
        .next()
        .unwrap();
    assert_eq!(theme["sourceType"], "propresenter");
    assert_eq!(
        orbit_host::authoring::reusable_themes(&workspace["themes"]).count(),
        1
    );
    let slide = workspace["library"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|item| item["slides"].as_array().unwrap())
        .find(|slide| {
            slide["presentationThemeId"]
                .as_str()
                .is_some_and(|assignment| {
                    orbit_host::authoring::is_imported_layout_assignment(
                        &workspace["themes"],
                        assignment,
                    )
                })
                && slide["presentationThemeSnapshot"]["layers"]
                    .as_array()
                    .is_some_and(|layers| !layers.is_empty())
        })
        .expect("The fixture has custom slide objects.");
    let assignment = slide["presentationThemeId"].as_str().unwrap();
    let composed = orbit_host::authoring::compose(
        slide,
        orbit_host::authoring::slide_theme(&workspace, slide),
    )
    .unwrap();
    assert!(!composed["layers"].as_array().unwrap().is_empty());
    let mut legacy = workspace.clone();
    for theme in legacy["themes"].as_array_mut().unwrap() {
        theme.as_object_mut().unwrap().remove("presentationOnly");
    }
    legacy["lastUsedPresentationThemeId"] = json!(assignment);
    let legacy = crate::collections::model::Snapshot::parse(&legacy).unwrap();
    assert_eq!(legacy.themes.len(), 1);
    assert!(legacy
        .last_used_theme
        .as_deref()
        .unwrap()
        .starts_with(&legacy.themes[0].id));

    app.restart(cx);
    let commands = cx.update(|cx| app.main.view.read(cx).commands.clone());
    let mut visual = app.visual(cx);
    visual.update(|window, cx| assert!(commands.run("open-theme-canvas", false, window, cx)));
    visual.run_until_parked();
    click(&mut visual, "theme-select");
    assert!(visual.debug_bounds("select-option-0").is_some());
    assert!(visual.debug_bounds("select-option-1").is_none());
}
