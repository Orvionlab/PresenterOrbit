use serde_json::Value;

pub fn reviewed_export<T>(
    confirm: impl FnOnce() -> bool,
    choose_destination: impl FnOnce() -> Option<T>,
    execute: impl FnOnce(T) -> Result<Value, String>,
) -> Result<Value, String> {
    if !confirm() {
        return Ok(serde_json::json!({ "canceled": true }));
    }
    let Some(destination) = choose_destination() else {
        return Ok(serde_json::json!({ "canceled": true }));
    };
    execute(destination)
}

pub(crate) fn present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
        Value::String(value) => !value.is_empty(),
        _ => true,
    }
}

fn omitted(value: &Value, reference: &str, keys: &[&str], findings: &mut Vec<String>) {
    for key in keys {
        if value.get(*key).is_some_and(present) {
            findings.push(format!(
                "{reference}/{key}: not included in the ProPresenter export"
            ));
        }
    }
}

fn primary_text_layer_index(slide: &Value) -> Option<usize> {
    let layers = slide.get("layers").and_then(Value::as_array)?;
    let primary_id = slide.get("primaryTextLayerId").and_then(Value::as_str);
    primary_id
        .and_then(|id| layers.iter().position(|layer| layer["id"] == id))
        .or_else(|| {
            layers
                .iter()
                .position(|layer| matches!(layer["role"].as_str(), Some("primary" | "slide-text")))
        })
        .or_else(|| {
            layers.iter().position(|layer| {
                layer["type"]
                    .as_str()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("text"))
            })
        })
}

/// `layered` slides are presentation slides, whose layers and their order are exported.
fn slide_losses(
    value: &Value,
    layer_source: Option<&Value>,
    reference: &str,
    layered: bool,
    findings: &mut Vec<String>,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    let style_source = layer_source.unwrap_or(value);
    let primary_index =
        primary_text_layer_index(style_source).or_else(|| primary_text_layer_index(value));
    let slide_width = style_source.get("width").and_then(Value::as_f64);
    let slide_height = style_source.get("height").and_then(Value::as_f64);
    let primary_layer = primary_index.and_then(|index| {
        style_source
            .get("layers")
            .and_then(|layers| layers.get(index))
            .or_else(|| value.get("layers").and_then(|layers| layers.get(index)))
    });
    let scroller_encoded = layered
        && primary_layer.is_some_and(|layer| {
            let width = slide_width.unwrap_or(1920.0);
            let height = slide_height.unwrap_or(1080.0);
            !super::propresenter_proto::layer_export::is_plain_primary(layer, width, height)
        });
    let text_style = super::propresenter_proto::portable_text_style(style_source);
    super::propresenter_proto::layer_export::text_style_losses(
        &text_style,
        reference,
        slide_width,
        primary_layer.and_then(|layer| layer.get("bounds")),
        scroller_encoded,
        findings,
    );
    if let Some(style) = value["notesTextStyle"].as_object() {
        paragraph_style_losses(style, &format!("{reference}/notesTextStyle"), "", findings);
    }
    for key in ["notesRuns", "contentRuns"] {
        rtf_run_losses(&value[key], &format!("{reference}/{key}"), findings);
    }
    if layered {
        for (index, layer) in style_source["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            rtf_run_losses(
                &layer["textRuns"],
                &format!("{reference}/layers/{index}/textRuns"),
                findings,
            );
        }
    }
    if layered {
        super::propresenter_proto::layer_build_export::slide_losses(value, reference, findings);
        super::propresenter_proto::cue_export::action_losses(value, reference, findings);
    }
    for (key, value) in object {
        // The layered cue export writes these cue, group and action fields.
        if layered
            && matches!(
                key.as_str(),
                "themeLayerOrder"
                    | "actions"
                    | "notes"
                    | "notesRuns"
                    | "notesTextStyle"
                    | "mediaBackgroundId"
                    | "groupId"
                    | "groupName"
                    | "groupColor"
                    | "sectionId"
                    | "sourceGroupId"
                    | "completionAction"
                    | "completionActionId"
                    | "completionTargetType"
                    | "completionTargetCueId"
                    | "completionTimeSeconds"
                    | "triggerTimeSeconds"
            )
        {
            continue;
        }
        if layered && key == "layers" {
            let layers = style_source.get("layers").unwrap_or(value);
            for (index, layer) in layers.as_array().into_iter().flatten().enumerate() {
                let layer_reference = format!("{reference}/layers/{index}");
                if Some(index) == primary_index {
                    super::propresenter_proto::layer_export::losses_without_text_style(
                        layer,
                        &layer_reference,
                        slide_width,
                        findings,
                    );
                } else {
                    super::propresenter_proto::layer_export::losses_with_slide_width(
                        layer,
                        &layer_reference,
                        slide_width,
                        findings,
                    );
                }
                if Some(index) != primary_index
                    && layer["type"]
                        .as_str()
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("text"))
                {
                    if let Some(style) = layer["textStyle"].as_object() {
                        paragraph_style_losses(
                            style,
                            &format!("{reference}/layers/{index}/textStyle"),
                            "",
                            findings,
                        );
                    }
                }
            }
            continue;
        }
        if present(value)
            && !matches!(
                key.as_str(),
                "id" | "sourceCueId"
                    | "content"
                    | "contentRuns"
                    | "text"
                    | "label"
                    | "disabled"
                    | "width"
                    | "height"
                    | "background"
                    | "backgroundEnabled"
                    | "textStyle"
                    | "primaryTextLayerId"
                    | "sourcePath"
                    | "sourceUrl"
                    | "sourceId"
                    | "autoClearDuration"
            )
        {
            findings.push(format!(
                "{reference}/{key}: not included in the portable text-slide export"
            ));
        }
    }
}

fn rtf_run_losses(runs: &Value, reference: &str, findings: &mut Vec<String>) {
    for (index, run) in runs.as_array().into_iter().flatten().enumerate() {
        if !run["text"].is_string() {
            findings.push(format!("{reference}/{index}: run text is required"));
            continue;
        }
        let style = &run["style"];
        let underline_variant = style["underlineStyle"].as_str().unwrap_or("single");
        let underline_pattern = style["underlinePattern"].as_str().unwrap_or("solid");
        if (style["underlineByWord"] == true
            && (underline_variant != "single" || underline_pattern != "solid"))
            || (underline_variant != "single" && underline_pattern != "solid")
        {
            findings.push(format!("{reference}/{index}: combined underline style, pattern and by-word setting cannot be written to RTF"));
        }
        if style["strikethroughStyle"] == "thick"
            || style["strikethroughByWord"] == true
            || style["strikethroughPattern"]
                .as_str()
                .is_some_and(|pattern| pattern != "solid")
            || style["strikethroughColor"].as_str().is_some()
        {
            findings.push(format!("{reference}/{index}: strikethrough style, pattern, by-word setting or color cannot be written to RTF"));
        }
    }
}

fn paragraph_style_losses(
    style: &serde_json::Map<String, Value>,
    reference: &str,
    prefix: &str,
    findings: &mut Vec<String>,
) {
    for (field, reason) in crate::native_text_layout::invalid_paragraph_fields(style) {
        findings.push(format!("{reference}/{prefix}{field}: {reason}"));
    }
    let paragraph_count = style
        .get("textParagraphStyles")
        .and_then(Value::as_array)
        .map_or(1, |paragraphs| paragraphs.len().min(2000))
        .max(1);
    let justified_tabs = (0..paragraph_count).any(|index| {
        let paragraph = crate::native_text_layout::resolve_paragraph_style(style, index);
        paragraph
            .get(crate::native_text_tabs::STOPS_FIELD)
            .and_then(Value::as_array)
            .is_some_and(|stops| {
                stops
                    .iter()
                    .any(|stop| stop.get("alignment").and_then(Value::as_str) == Some("justify"))
            })
    });
    if justified_tabs {
        findings.push(format!(
            "{reference}/{}textTabStops: justified tab stops keep their positions but export as left-aligned stops because the ProPresenter RTF tab types do not include justified alignment",
            prefix
        ));
    }
    if style
        .get("textParagraphStyles")
        .and_then(Value::as_array)
        .is_some_and(|paragraphs| {
            paragraphs
                .iter()
                .any(|paragraph| paragraph.get("textAbsoluteRightEdge").is_some())
        })
    {
        findings.push(format!(
            "{reference}/{prefix}textParagraphStyles: RTF cannot preserve absolute paragraph end positions; conversion to right insets requires a known text-box width and resizing changes their positions"
        ));
    }
}

fn paragraph_losses(slide: &Value, reference: &str, findings: &mut Vec<String>) {
    let style = super::propresenter_proto::portable_text_style(slide);
    paragraph_style_losses(&style, reference, "", findings);
}

pub fn export_losses(workspace: &Value) -> Vec<String> {
    let mut findings = Vec::new();
    let mut library_names = crate::workspace_export::Names::default();
    for library in workspace["libraries"].as_array().into_iter().flatten() {
        if let Some(title) = library["title"].as_str() {
            let exported = library_names.folder(title, "Library");
            if exported != title {
                findings.push(format!(
                    "library/{title}: exported folder name is {exported}"
                ));
            }
        }
        let mut item_names = crate::workspace_export::Names::default();
        for id in library["itemIds"].as_array().into_iter().flatten() {
            if let Some(item) = workspace["library"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|item| item["id"] == *id)
            {
                if let Some(title) = item["title"].as_str() {
                    let exported = item_names.folder(title, "Presentation");
                    if exported != title {
                        findings.push(format!("presentation/{title}: ProPresenter displays the exported filename {exported}"));
                    }
                }
            }
        }
    }
    let mut theme_names = crate::workspace_export::Names::default();
    for theme in workspace["themes"].as_array().into_iter().flatten() {
        if let Some(title) = theme["title"].as_str() {
            let exported = theme_names.folder(title, "Theme");
            if exported != title {
                findings.push(format!("theme/{title}: exported folder name is {exported}"));
            }
        }
    }
    let mut effect_preset_names = crate::workspace_export::Names::default();
    for preset in workspace["mediaEffectPresets"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let Some(title) = preset["title"].as_str() {
            let exported = effect_preset_names.preset(title, "Effect");
            if exported != title {
                findings.push(format!(
                    "effect preset/{title}: ProPresenter displays the exported filename {exported}"
                ));
            }
        }
    }
    for (index, playlist) in workspace["playlist"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        omitted(
            playlist,
            &format!("playlist/{index}"),
            &["targetedLayerId", "hotkey", "presentationThemeId"],
            &mut findings,
        );
        for (entry_index, entry) in playlist["items"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            omitted(
                entry,
                &format!("playlist/{index}/items/{entry_index}"),
                &["actions", "color", "tagIds", "musicKey"],
                &mut findings,
            );
        }
    }
    for collection in ["library", "themes", "props"] {
        for (index, item) in workspace
            .get(collection)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let reference = format!(
                "{collection}/{}",
                item.get("id").and_then(Value::as_str).unwrap_or("unnamed")
            );
            let reference = format!("{reference}[{index}]");
            let structure: &[&str] = if collection == "library" {
                super::propresenter_proto::cue_export::arrangement_losses(
                    item,
                    &reference,
                    &mut findings,
                );
                &["themeId", "presentationThemeId"]
            } else {
                &[
                    "groups",
                    "arrangements",
                    "selectedArrangementId",
                    "themeId",
                    "presentationThemeId",
                ]
            };
            omitted(item, &reference, structure, &mut findings);
            let portable = if collection == "library" {
                super::content_themes::compose_item(workspace.as_object().unwrap(), item)
            } else {
                item.clone()
            };
            for (slide_index, slide) in portable["slides"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                paragraph_losses(
                    slide,
                    &format!("{reference}/slides/{slide_index}"),
                    &mut findings,
                );
            }
            for (slide_index, slide) in item
                .get("slides")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                slide_losses(
                    slide,
                    portable["slides"].get(slide_index),
                    &format!("{reference}/slides/{slide_index}"),
                    matches!(collection, "library" | "themes"),
                    &mut findings,
                );
                if collection == "themes" {
                    // Template slides carry their name; the cue fields below do not apply.
                    findings.retain(|finding| {
                        finding
                            != &format!(
                                "{reference}/slides/{slide_index}/name: not included in the portable text-slide export"
                            )
                    });
                }
            }
        }
    }
    for collection in ["props", "masks", "stageLayouts"] {
        for (index, item) in workspace
            .get(collection)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let slide = item.get("slide").unwrap_or(item);
            paragraph_losses(slide, &format!("{collection}/{index}/slide"), &mut findings);
            if item.get("slide").is_some() {
                slide_losses(
                    slide,
                    None,
                    &format!("{collection}/{index}/slide"),
                    collection != "stageLayouts",
                    &mut findings,
                );
            } else {
                omitted(
                    item,
                    &format!("{collection}/{index}"),
                    &["layers", "actions", "textStyle", "build", "transition"],
                    &mut findings,
                );
            }
        }
    }
    for (index, item) in workspace
        .get("macros")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        omitted(
            item,
            &format!("macros/{index}"),
            &[
                "actions",
                "enabled",
                "color",
                "hotkey",
                "imageData",
                "collectionIds",
            ],
            &mut findings,
        );
    }
    for (index, group) in workspace
        .get("clearGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        if group["layers"]
            .as_array()
            .is_some_and(|layers| layers.iter().any(|layer| layer == "stage-messages"))
        {
            findings.push(format!(
                "clearGroups/{index}/layers/stage-messages: Orbit-only stage message clearing is not part of ProPresenter clear groups"
            ));
        }
    }
    if workspace.get("media").is_some_and(present)
        || workspace.get("mediaLibraries").is_some_and(present)
    {
        let mut bin_losses = std::collections::BTreeSet::new();
        for kind in ["graphics", "audio"] {
            bin_losses.extend(super::propresenter_proto::media_bin_export::losses(
                workspace, kind,
            ));
        }
        findings.extend(
            bin_losses
                .into_iter()
                .map(|finding| format!("workspace/media: {finding}")),
        );
    }
    omitted(
        workspace,
        "workspace",
        &[
            "screenConfig",
            "audienceLooks",
            "scheduledEvents",
            "plans",
            "triggerBindings",
            "communicationDevices",
            "capturePresets",
            "recordingPresets",
        ],
        &mut findings,
    );
    findings
}

fn executable_capture_action(action: &Value) -> bool {
    let details = &action["details"];
    let start = details.get("start");
    let stop = details.get("stop");
    match (start, stop) {
        (Some(start), None) => start
            .get("presetIdentification")
            .and_then(Value::as_str)
            .is_some(),
        (None, Some(stop)) => {
            stop == true
                || stop
                    .get("showsAlertBeforeStopping")
                    .and_then(Value::as_bool)
                    .is_some()
        }
        _ => false,
    }
}

/// Report retained action contracts which the automation executor cannot run.
pub fn import_findings(value: &Value, reference: &str, findings: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            if let Some(actions) = object.get("actions").and_then(Value::as_array) {
                for (index, action) in actions.iter().enumerate() {
                    if let Some(kind) = action.get("type").and_then(Value::as_str) {
                        if kind == "capture" && !executable_capture_action(action) {
                            findings.push(format!("{reference}/actions/{index}: capture action requires an exact recording preset or an explicit stop command"));
                        }
                        if kind == "audio-input" && action.get("targetId").is_none() {
                            findings.push(format!("{reference}/actions/{index}: imported audio-input action requires an explicit native device/channel mapping in Audio settings before use"));
                        }
                        if matches!(kind, "unknown") {
                            findings.push(format!("{reference}/actions/{index}: action '{kind}' is retained but is not executable in Orbit"));
                        }
                    }
                }
            }
            if let Some(name) = object.get("unsupportedTransition") {
                findings.push(format!(
                    "{reference}/transition: ProPresenter transition {name} is not in the built-in library Orbit supports; it was not imported"
                ));
            }
            if let Some(names) = object.get("unsupportedTransitionVariables") {
                findings.push(format!(
                    "{reference}/transition: transition variables {names} are unknown or out of range; the library values are used"
                ));
            }
            if let Some(effects) = object.get("unsupportedMediaEffects") {
                findings.push(format!(
                    "{reference}: media effects {effects} are not in the library Orbit renders or have invalid variables; they were not imported"
                ));
            }
            if let Some(names) = object.get("unsupportedMediaEffectVariables") {
                findings.push(format!(
                    "{reference}: media effect variables {names} are unknown or out of range; the library values are used"
                ));
            }
            if let Some(path) = object.get("unsupportedMediaEffectSource") {
                findings.push(format!(
                    "{reference}: effects of media bin item {path} were not imported because the file was not found"
                ));
            }
            if let Some(reason) = object.get("unsupportedMediaTransport") {
                findings.push(format!(
                    "{reference}: video transport was not imported: {reason}"
                ));
            }
            if let Some(path) = object.get("unsupportedMediaTransportSource") {
                findings.push(format!("{reference}: video transport of media bin item {path} was not imported because the file was not found"));
            }
            if let Some(path) = object.get("duplicateMediaEffectSource") {
                findings.push(format!(
                    "{reference}: media bin item {path} appears more than once; only its first effect chain was imported"
                ));
            }
            if let Some(url) = object.get("unsupportedWebUrl") {
                findings.push(format!(
                    "{reference}/fill: web fill address {url} is not an http or https page; the object stays empty"
                ));
            }
            if let Some(fields) = object.get("unsupportedParagraphFields") {
                findings.push(format!(
                    "{reference}: paragraph settings {fields} were not imported"
                ));
            }
            if let Some(content) = object.get("unsupportedSourceContent") {
                findings.push(format!(
                    "{reference}: ProPresenter content {content} was not imported"
                ));
            }
            if let Some(fields) = object.get("unsupportedFillFields") {
                findings.push(format!(
                    "{reference}/fill: unsupported ProPresenter fields {fields} were not converted"
                ));
            }
            for (key, child) in object {
                import_findings(child, &format!("{reference}/{key}"), findings);
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                import_findings(child, &format!("{reference}/{index}"), findings);
            }
        }
        _ => {}
    }
}
