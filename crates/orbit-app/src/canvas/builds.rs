use serde_json::{json, Value};
pub(super) const CUT: &str = "AB29D07B-E9E2-4E0A-93BD-AD3EA58120FA";
pub(super) const DISSOLVE: &str = "EC52A828-AD85-4602-B70C-1DEE7C904DB6";
fn step(value: &Value) -> Option<u64> {
    value
        .as_f64()
        .filter(|n| n.fract() == 0. && *n > 0. && *n <= 9_007_199_254_740_991.)
        .map(|n| n as u64)
}
fn last(layer: &Value) -> u64 {
    layer["textReveal"]["children"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|child| step(&child["step"]))
        .chain(step(&layer["build"]["step"]))
        .max()
        .unwrap_or(0)
}
fn reconcile_out(layer: &mut Value) {
    if step(&layer["buildOut"]["step"]).is_some_and(|s| s <= last(layer)) {
        layer["buildOut"]["step"] = json!(last(layer) + 1);
    }
}
pub fn patch(layer: &Value, slide: &Value, field: &str, value: Value) -> Result<Value, String> {
    let mut next = layer.clone();
    if field == "buildDelivery" {
        let mode = value.as_str().ok_or("Choose text delivery.")?;
        if !super::editing::is_text(layer)
            || !matches!(mode, "whole-object" | "paragraph" | "underline")
        {
            return Err("Text delivery requires a text object.".into());
        }
        if mode == "whole-object" {
            next["textReveal"] = Value::Null;
            if next["buildOut"].is_object() {
                next["buildOut"]["step"] = json!(last(&next) + 1);
            }
            return Ok(next);
        }
        if layer["textReveal"]["mode"] == mode {
            reconcile_out(&mut next);
            return Ok(next);
        }
        let layers = slide["layers"].as_array();
        let primary = layers
            .and_then(|layers| {
                layers
                    .iter()
                    .find(|candidate| candidate["id"] == slide["primaryTextLayerId"])
            })
            .or_else(|| {
                layers.and_then(|layers| {
                    layers.iter().find(|candidate| {
                        matches!(candidate["role"].as_str(), Some("primary" | "slide-text"))
                    })
                })
            })
            .or_else(|| {
                layers.and_then(|layers| {
                    layers
                        .iter()
                        .find(|candidate| super::editing::is_text(candidate))
                })
            });
        let primary = primary.is_some_and(|primary| primary["id"] == layer["id"]);
        let raw = if primary {
            slide["content"]
                .as_str()
                .or_else(|| slide["text"].as_str())
                .or_else(|| layer["text"].as_str())
                .unwrap_or("")
        } else {
            layer["text"].as_str().unwrap_or("")
        };
        let style = super::editing::style(slide, Some(layer));
        let text = super::editing::display_text(raw, &style);
        let count = if mode == "paragraph" {
            super::editing::paragraphs(&text)
                .iter()
                .filter(|line| !line.is_empty())
                .count()
                .saturating_sub(1)
        } else if text.is_empty() {
            0
        } else if style["underline"] == true {
            1
        } else {
            let runs = if primary {
                slide["contentRuns"]
                    .as_array()
                    .or_else(|| layer["textRuns"].as_array())
            } else {
                layer["textRuns"].as_array()
            };
            if let Some(runs) = runs.filter(|runs| {
                runs.iter()
                    .filter_map(|run| run["text"].as_str())
                    .collect::<String>()
                    == text
            }) {
                let mut count = 0;
                let mut previous = false;
                for run in runs {
                    let nonempty = run["text"].as_str().is_some_and(|text| !text.is_empty());
                    let underline = run["style"]["underline"] == true;
                    if nonempty {
                        if underline && !previous {
                            count += 1;
                        }
                        previous = underline;
                    }
                }
                count
            } else {
                0
            }
        };
        let prefix = if mode == "paragraph" {
            "orbit-paragraph-build:"
        } else {
            "orbit-underline-build:"
        };
        let start = step(&layer["build"]["step"]).unwrap_or(0);
        if start + count as u64 > 9_007_199_254_740_991 {
            return Err("Text reveal build step exceeds the supported range.".into());
        }
        let id = layer["id"].as_str().ok_or("Object identity is required.")?;
        next["textReveal"] = json!({"mode":mode,"fromIndex":0,"children":(0..count).map(|index|json!({"id":format!("{prefix}{id}:{index}"),"index":index,"step":start+index as u64+1,"start":"on-click","delaySeconds":0})).collect::<Vec<_>>()});
        reconcile_out(&mut next);
        return Ok(next);
    }
    let (key, prefix) = if field.starts_with("buildOut") {
        ("buildOut", "buildOut")
    } else {
        ("build", "build")
    };
    let suffix = field
        .strip_prefix(prefix)
        .ok_or("Unsupported build field.")?;
    if suffix == "Enabled" {
        if !value.is_boolean() {
            return Err("Build enabled state must be a boolean.".into());
        }
        next[key] = if value == true {
            json!({"step":if key=="buildOut"{last(layer)+1}else{1},"start":"on-click","delaySeconds":0})
        } else {
            Value::Null
        };
    } else {
        if !next[key].is_object() {
            return Err("Enable the build before editing its fields.".into());
        }
        match suffix {
            "Step" => {
                let step = step(&value).ok_or("Build steps must be positive integers.")?;
                if key == "buildOut" && step <= last(layer) {
                    return Err("Build out must follow build in and text reveal.".into());
                }
                next[key]["step"] = json!(step);
                if next[key]["start"] == "with-slide" {
                    next[key]["start"] = json!("on-click");
                }
            }
            "Start" => {
                if !matches!(value.as_str(), Some("on-click" | "after-previous")) {
                    return Err("Invalid build start.".into());
                }
                if step(&next[key]["step"]).is_none() {
                    if !next[key]["step"].is_null() && next[key]["step"] != 0 {
                        return Err(
                            "Object build step must be repaired before changing its start.".into(),
                        );
                    }
                    next[key]["step"] = json!(if key == "buildOut" {
                        last(layer) + 1
                    } else {
                        1
                    });
                }
                next[key]["start"] = value;
            }
            "Delay" => {
                let n = super::editing::number(&value)?;
                if n < 0. {
                    return Err("Build delay must be nonnegative.".into());
                }
                next[key]["delaySeconds"] = value;
            }
            "Transition" => {
                if value.is_object() {
                    validate_transition(&value)?;
                    next[key]["transition"] = value;
                } else {
                    let effect = match value.as_str().ok_or("Choose a transition.")? {
                        "cut" => CUT.to_string(),
                        "dissolve" => DISSOLVE.to_string(),
                        id => id.to_uppercase(),
                    };
                    if effect == CUT {
                        next[key]["transition"] = json!({"effectId":CUT,"durationSeconds":0});
                    } else if next[key]["transition"]["effectId"] != effect {
                        let duration = if next[key]["transition"]["effectId"] != CUT {
                            next[key]["transition"]["durationSeconds"]
                                .as_f64()
                                .filter(|n| *n >= 0. && *n <= 10.)
                                .unwrap_or(0.6)
                        } else {
                            0.6
                        };
                        next[key]["transition"] =
                            json!({"effectId":effect,"durationSeconds":duration});
                    }
                    validate_transition(&next[key]["transition"])?;
                }
            }
            "TransitionDuration" => {
                if next[key]["transition"]["effectId"].is_null()
                    || next[key]["transition"]["effectId"] == CUT
                {
                    return Err("Choose an effect before its duration.".into());
                }
                next[key]["transition"]["durationSeconds"] = value;
                validate_transition(&next[key]["transition"])?;
            }
            "TransitionDirection" => next[key]["transition"]["direction"] = value,
            _ => return Err("Unsupported build field.".into()),
        }
    }
    if key == "build" && matches!(suffix, "Enabled" | "Step" | "Start") {
        let old = step(&layer[key]["step"]).unwrap_or(0);
        let new = step(&next[key]["step"]).unwrap_or(0);
        let id = layer["id"].as_str().unwrap_or("");
        let mode = next["textReveal"]["mode"].as_str().unwrap_or("");
        let generated_prefix = match mode {
            "paragraph" => Some(format!("orbit-paragraph-build:{id}:")),
            "underline" => Some(format!("orbit-underline-build:{id}:")),
            _ => None,
        };
        if let Some((prefix, children)) = generated_prefix
            .as_deref()
            .zip(next["textReveal"]["children"].as_array_mut())
        {
            let generated = children.iter().all(|child| {
                child["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(prefix))
            });
            for child in children {
                let child_step = step(&child["step"]);
                if generated {
                    let child_step = child_step.ok_or("Text reveal step is invalid.")?;
                    let shifted = child_step as i64 + new as i64 - old as i64;
                    if shifted <= 0 || shifted > 9_007_199_254_740_991 {
                        return Err("Text reveal step is out of range.".into());
                    }
                    child["step"] = json!(shifted);
                } else if child_step.is_some_and(|step| step <= new) {
                    return Err("Build in must precede imported reveal steps.".into());
                }
            }
        }
        reconcile_out(&mut next);
    }
    if next["buildOut"]["step"]
        .as_u64()
        .is_some_and(|step| step > 9_007_199_254_740_991)
    {
        return Err("Build out step exceeds the supported range.".into());
    }
    Ok(next)
}
pub fn validate_transition(value: &Value) -> Result<(), String> {
    let id = value["effectId"]
        .as_str()
        .ok_or("A transition effect identity is required.")?;
    let parts = id.split('-').collect::<Vec<_>>();
    if parts.len() != 5
        || parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .any(|(part, len)| part.len() != len || !part.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return Err("Invalid transition effect identity.".into());
    }
    let duration = super::editing::number(&value["durationSeconds"])?;
    if !(0. ..=10.).contains(&duration) {
        return Err("Transition duration must be between 0 and 10 seconds.".into());
    }
    if id == CUT
        && (duration != 0. || value.get("direction").is_some() || value.get("variables").is_some())
    {
        return Err("Cut transitions cannot have a duration or variables.".into());
    }
    Ok(())
}
