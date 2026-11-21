//! ProPresenter `Slide.Element.Build` encoding for layered slide exports.

use super::*;

fn transition_message(value: &Value) -> Option<Vec<u8>> {
    crate::native_transitions::validate(value).ok()?;
    super::encode_transition(value)
}

/// Encodes the supported `build` fields for one element, retaining the proto's element UUID link.
pub(super) fn build_message(layer: &Value, element_id: &str) -> Option<Vec<u8>> {
    build_message_for(layer, element_id, "build")
}

pub(super) fn build_message_for(layer: &Value, element_id: &str, key: &str) -> Option<Vec<u8>> {
    let build = layer.get(key)?.as_object()?;
    let mut start = build
        .get("start")
        .and_then(Value::as_str)
        .and_then(|start| match start {
            "on-click" => Some(0),
            "with-previous" => Some(1),
            "after-previous" => Some(2),
            "with-slide" => Some(3),
            _ => None,
        });
    if start.is_none()
        && build.get("start").is_none()
        && build.get("step").and_then(Value::as_u64).is_some()
    {
        start = Some(0);
    }
    let delay = build.get("delaySeconds").and_then(Value::as_f64);
    let transition = build.get("transition").and_then(transition_message);
    if start.is_none() && delay.is_none() && transition.is_none() {
        return None;
    }

    let mut message = Vec::new();
    put_uuid(1, &object_build_id(element_id, key), &mut message);
    put_uuid(2, element_id, &mut message);
    if let Some(start) = start {
        // START_ON_CLICK is zero, so write it explicitly to preserve authored presence.
        put_varint(3, start, &mut message);
    }
    if let Some(delay) = delay {
        put_f64(4, delay, &mut message);
    }
    if let Some(transition) = transition {
        put_message(5, transition, &mut message);
    }
    Some(message)
}

/// Adds losses for build values that cannot be written as a schema-backed `Build`.
pub(super) fn losses(layer: &Value, reference: &str, findings: &mut Vec<String>) {
    for key in ["build", "buildOut"] {
        object_losses(layer, key, reference, findings);
    }
    if let Some(reveal) = layer.get("textReveal").filter(|value| !value.is_null()) {
        let valid = matches!(reveal["mode"].as_str(), Some("paragraph" | "underline"))
            && reveal
                .get("fromIndex")
                .is_none_or(|value| value.as_u64().is_some_and(|n| n <= u32::MAX as u64))
            && reveal["children"].as_array().is_some_and(|children| {
                children.iter().all(|child| {
                    child["id"].as_str().is_some_and(|id| !id.is_empty())
                        && child["index"]
                            .as_u64()
                            .is_some_and(|n| n <= u32::MAX as u64)
                        && (authored_build_step(child).is_some()
                            || (child.get("step").is_none()
                                && child["index"].as_u64().is_some_and(|index| {
                                    index
                                        < reveal["fromIndex"]
                                            .as_u64()
                                            .unwrap_or(0)
                                            .saturating_sub(1)
                                })))
                })
            });
        if !valid {
            findings.push(format!("{reference}/textReveal: requires a reveal mode and indexed child builds with UUIDs and steps"));
        }
        if let Some(object) = reveal.as_object() {
            for (key, value) in object {
                if !["mode", "fromIndex", "children"].contains(&key.as_str())
                    && super::super::import_export_report::present(value)
                {
                    findings.push(format!(
                        "{reference}/textReveal/{key}: not included in the layered slide export"
                    ));
                }
            }
        }
        for (index, child) in reveal["children"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let wrapped = json!({"build":child});
            if super::super::import_export_report::present(&child["transition"]) {
                findings.push(format!("{reference}/textReveal/children/{index}/transition: ChildBuild has no independent transition field"));
            }
            let mut child_findings = Vec::new();
            object_losses(
                &wrapped,
                "build",
                &format!("{reference}/textReveal/children/{index}"),
                &mut child_findings,
            );
            findings.extend(
                child_findings.into_iter().filter(|value| {
                    !value.contains("/build/id:") && !value.contains("/build/index:")
                }),
            );
        }
    }
}

fn object_losses(layer: &Value, build_key: &str, reference: &str, findings: &mut Vec<String>) {
    let lost = |key: &str, reason: &str, findings: &mut Vec<String>| {
        findings.push(format!("{reference}/{build_key}/{key}: {reason}"));
    };
    let Some(build) = layer.get(build_key).and_then(Value::as_object) else {
        if super::super::import_export_report::present(&layer[build_key]) {
            findings.push(format!(
                "{reference}/{build_key}: build settings must be an object with a supported start, delay, or transition"
            ));
        }
        return;
    };

    for (key, value) in build {
        let supported = match key.as_str() {
            "start" => matches!(
                value.as_str(),
                Some("on-click" | "with-previous" | "after-previous" | "with-slide")
            ),
            "step" => value.as_u64().is_some_and(|step| {
                step > 0
                    || build
                        .get("start")
                        .is_some_and(|start| start == "with-slide")
            }),
            "delaySeconds" => value.as_f64().is_some(),
            "transition" => transition_message(value).is_some(),
            _ => false,
        };
        if supported || !super::super::import_export_report::present(value) {
            continue;
        }
        if key == "transition" {
            lost(
                key,
                "must be a valid ProPresenter library transition",
                findings,
            );
        } else {
            lost(key, "not included in the layered slide export", findings);
        }
    }
}

fn object_build_id(element_id: &str, key: &str) -> String {
    normalized_uuid(&format!(
        "build-{}:{}",
        if key == "buildOut" { "out" } else { "in" },
        normalized_uuid(element_id)
    ))
}

pub(super) fn encode_reveal(layer: &Value, output: &mut Vec<u8>) {
    let reveal = &layer["textReveal"];
    let mode = match reveal["mode"].as_str() {
        Some("paragraph") => 1,
        Some("underline") => 2,
        _ => return,
    };
    put_varint(5, mode, output);
    if let Some(index) = reveal["fromIndex"]
        .as_u64()
        .filter(|n| *n <= u32::MAX as u64)
    {
        put_varint(8, index, output);
    }
    for child in reveal["children"].as_array().into_iter().flatten() {
        let (Some(id), Some(index)) = (
            child["id"].as_str(),
            child["index"].as_u64().filter(|n| *n <= u32::MAX as u64),
        ) else {
            continue;
        };
        let mut message = Vec::new();
        put_uuid(1, &normalized_uuid(id), &mut message);
        let start = match child["start"].as_str() {
            Some("with-previous") => 1,
            Some("after-previous") => 2,
            Some("with-slide") => 3,
            _ => 0,
        };
        put_varint(2, start, &mut message);
        if let Some(delay) = child["delaySeconds"].as_f64() {
            put_f64(3, delay, &mut message);
        }
        put_varint(4, index, &mut message);
        put_message(7, message, output);
    }
}

pub(super) fn decode_builds(
    element: &[Field],
    steps: &BTreeMap<String, u64>,
    layer: &mut Map<String, Value>,
) -> Vec<String> {
    let mut unsupported = Vec::new();
    for (field, key) in [(2, "build"), (3, "buildOut")] {
        if let Some(build) = nested_field(element, field) {
            if uuid(&build, 1).is_none() {
                unsupported.push(format!("{key} without a UUID"));
            }
            let mut value = Map::new();
            value.insert(
                "start".into(),
                json!(slide_build_start(varint_field(&build, 3).unwrap_or(0))),
            );
            if let Some(delay) = f64_field(&build, 4) {
                value.insert("delaySeconds".into(), json!(delay));
            }
            if let Some(transition) = nested_field(&build, 5) {
                insert_transition(&mut value, &transition);
            }
            if let Some(step) = uuid(&build, 1).and_then(|id| steps.get(&id).copied()) {
                value.insert("step".into(), json!(step));
            }
            if value.contains_key("step")
                && value
                    .get("transition")
                    .is_some_and(object_transition_unrendered)
            {
                unsupported.push(format!(
                    "{key} transition is retained but has invalid or unavailable catalogue metadata"
                ));
            }
            layer.insert(key.into(), Value::Object(value));
        }
    }
    let mode = match varint_field(element, 5).unwrap_or(0) {
        0 => None,
        1 => Some("paragraph"),
        2 => Some("underline"),
        _ => {
            unsupported.push("unknown text reveal mode".into());
            None
        }
    };
    if let Some(mode) = mode {
        let skipped_children = varint_field(element, 8).unwrap_or(0).saturating_sub(1);
        let mut children = Vec::new();
        for child in nested_fields(element, 7) {
            let Some(id) = uuid(&child, 1) else {
                unsupported.push("child build without a UUID".into());
                continue;
            };
            let mut value = json!({"id":id,"index":varint_field(&child, 4).unwrap_or(0),"start":slide_build_start(varint_field(&child, 2).unwrap_or(0))});
            if let Some(delay) = f64_field(&child, 3) {
                value["delaySeconds"] = json!(delay);
            }
            if let Some(step) = steps.get(&id) {
                value["step"] = json!(step);
            } else if varint_field(&child, 4).unwrap_or(0) >= skipped_children {
                unsupported.push("child build missing from element build order".into());
            }
            children.push(value);
        }
        layer.insert("textReveal".into(), json!({"mode":mode,"fromIndex":varint_field(element, 8).unwrap_or(0),"children":children}));
    } else if nested_fields(element, 7).next().is_some() {
        unsupported.push("child builds without a text reveal mode".into());
    }
    unsupported
}

fn object_transition_unrendered(value: &Value) -> bool {
    crate::native_transitions::resolve(value).is_err()
}

fn authored_build_step(build: &Value) -> Option<u64> {
    build["step"]
        .as_u64()
        .filter(|step| *step > 0 || build["start"] == "with-slide")
}

fn ordered_builds(slide: &Value) -> Vec<(String, &Value, String)> {
    let mut builds = Vec::new();
    for (index, layer) in slide["layers"].as_array().into_iter().flatten().enumerate() {
        let Some(id) = layer["id"].as_str().filter(|id| !id.is_empty()) else {
            continue;
        };
        for key in ["build", "buildOut"] {
            if authored_build_step(&layer[key]).is_some() {
                builds.push((
                    object_build_id(id, key),
                    &layer[key],
                    format!("layers/{index}/{key}"),
                ));
            }
        }
        for (child_index, child) in layer["textReveal"]["children"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if authored_build_step(child).is_some() {
                if let Some(id) = child["id"].as_str().filter(|id| !id.is_empty()) {
                    builds.push((
                        normalized_uuid(id),
                        child,
                        format!("layers/{index}/textReveal/children/{child_index}"),
                    ));
                }
            }
        }
    }
    builds.sort_by_key(|(_, build, _)| (authored_build_step(build), build["start"] != "on-click"));
    builds
}

pub(super) fn order(slide: &Value) -> Vec<String> {
    ordered_builds(slide)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect()
}

pub(super) fn grouped_layer(slide: &Value, layer: &Value) -> Value {
    let mut value = layer.clone();
    let ordered = ordered_builds(slide);
    let follower = |id: &str, build: &Value| {
        authored_build_step(build).is_some_and(|step| {
            step > 0
                && ordered
                    .iter()
                    .find(|(_, candidate, _)| authored_build_step(candidate) == Some(step))
                    .is_some_and(|(head, _, _)| head != id)
        })
    };
    if let Some(id) = layer["id"].as_str() {
        for key in ["build", "buildOut"] {
            if follower(&object_build_id(id, key), &layer[key]) {
                value[key]["start"] = json!("with-previous");
            }
        }
    }
    if let Some(children) = value["textReveal"]["children"].as_array_mut() {
        for child in children {
            if child["id"]
                .as_str()
                .is_some_and(|id| follower(&normalized_uuid(id), child))
            {
                child["start"] = json!("with-previous");
            }
        }
    }
    value
}

pub(crate) fn slide_losses(slide: &Value, reference: &str, findings: &mut Vec<String>) {
    let mut identifiers = std::collections::BTreeSet::new();
    for (id, _, path) in ordered_builds(slide) {
        if !identifiers.insert(id) {
            findings.push(format!(
                "{reference}/{path}: duplicate build UUID cannot identify a unique build row"
            ));
        }
    }
}

pub(super) fn decode_steps(base: &[Field]) -> (BTreeMap<String, u64>, Vec<String>) {
    let mut builds = BTreeMap::new();
    for element in nested_fields(base, 1) {
        for field in [2, 3] {
            if let Some(build) = nested_field(&element, field) {
                if let Some(id) = uuid(&build, 1) {
                    builds.insert(id, varint_field(&build, 3).unwrap_or(0));
                }
            }
        }
        for child in nested_fields(&element, 7) {
            if let Some(id) = uuid(&child, 1) {
                builds.insert(id, varint_field(&child, 2).unwrap_or(0));
            }
        }
    }
    let mut steps = BTreeMap::new();
    let mut unsupported = Vec::new();
    let mut step = 0;
    for bytes in bytes_fields(base, 2) {
        let Some(id) = decode_message(bytes)
            .ok()
            .and_then(|fields| string_field(&fields, 1))
        else {
            unsupported.push("malformed element build order UUID".into());
            continue;
        };
        if steps.contains_key(&id) {
            unsupported.push("duplicate element build order UUID".into());
            continue;
        }
        let Some(start) = builds.get(&id) else {
            unsupported.push("element build order references an unavailable build".into());
            continue;
        };
        let current = match start {
            0 | 2 => {
                step += 1;
                step
            }
            1 => step,
            3 => 0,
            _ => {
                unsupported.push("unsupported element build start".into());
                continue;
            }
        };
        steps.insert(id, current);
    }
    (steps, unsupported)
}
