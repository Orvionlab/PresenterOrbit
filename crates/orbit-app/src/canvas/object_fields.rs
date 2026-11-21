//! Object-level field patches shared with the renderer's `getEditableObjectFieldPatch`: conditional visibility,
//! path closing and points, and timer color triggers. The fixtures in `tests/fixtures/gpui-theme-authoring.json`
//! (kind `object`) pin both implementations to the same results.
use serde_json::{json, Map, Value};

const TIMER_CRITERIA: [&str; 4] = [
    "has-time-remaining",
    "has-expired",
    "is-running",
    "not-running",
];

pub fn criteria(kind: &str) -> Vec<&'static str> {
    match kind {
        "element" => vec!["has-text", "has-no-text"],
        "capture" | "video-input" => vec!["active", "inactive"],
        "video" | "audio" => TIMER_CRITERIA
            .into_iter()
            .chain(["looping", "not-looping"])
            .collect(),
        _ => TIMER_CRITERIA.to_vec(),
    }
}

/// The kind of a stored condition; a condition without a kind is a timer condition.
pub fn kind(condition: &Value) -> &str {
    condition["kind"].as_str().unwrap_or("timer")
}

/// `Ok(None)` when the field is not an object-level field.
pub fn patch(layer: &Value, field: &str, value: &Value) -> Result<Option<Value>, String> {
    if field.starts_with("visibility") {
        return visibility(layer, field, value).map(Some);
    }
    if let Some(rest) = field.strip_prefix("timerTrigger:") {
        return trigger(layer, rest, value).map(Some);
    }
    if field == "pathClosed" {
        let mut next = layer.clone();
        next["closedPath"] = json!(value == &json!(true));
        return Ok(Some(next));
    }
    if let Some(rest) = field.strip_prefix("pathPoint:") {
        return path_point(layer, rest, value).map(Some);
    }
    Ok(None)
}

fn map(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(fields) => fields,
        _ => Map::new(),
    }
}

fn index(text: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .map_err(|_| "Unknown object field.".to_string())
}

fn trigger(layer: &Value, rest: &str, value: &Value) -> Result<Value, String> {
    let (row, key) = rest
        .split_once(':')
        .filter(|(_, key)| matches!(*key, "time" | "color"))
        .ok_or("Unknown object field.")?;
    let row = index(row)?;
    let mut next = layer.clone();
    let trigger = next["textStyle"]["linkedTextTimerColorTriggers"]
        .get_mut(row)
        .ok_or("Timer color trigger does not exist.")?;
    trigger[key] = if key == "time" {
        let time = value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .filter(|time| time.is_finite())
            .ok_or("Timer trigger time must be finite.")?;
        super::editing::numeric(time)
    } else {
        json!(value.as_str().ok_or("A timer trigger color is required.")?)
    };
    Ok(next)
}

fn path_point(layer: &Value, rest: &str, value: &Value) -> Result<Value, String> {
    let pieces = rest.split(':').collect::<Vec<_>>();
    let [row, target, axis] = pieces[..] else {
        return Err("Unknown object field.".into());
    };
    if !matches!(target, "anchor" | "q0" | "q1") || !matches!(axis, "x" | "y") {
        return Err("Unknown object field.".into());
    }
    let coordinate = value
        .as_f64()
        .filter(|number| number.is_finite())
        .ok_or("Path point coordinate is invalid.")?;
    let mut next = layer.clone();
    let point = next["pathPoints"]
        .get_mut(index(row)?)
        .ok_or("Path point coordinate is invalid.")?;
    let owner = if target == "anchor" {
        point
    } else {
        point
            .get_mut(target)
            .filter(|control| control.is_object())
            .ok_or("Path control point does not exist.")?
    };
    owner[axis] = super::editing::numeric(coordinate);
    Ok(next)
}

fn string<'a>(value: &'a Value, key: &str, message: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| message.to_string())
}

fn timer_reference(value: &Value) -> Result<Map<String, Value>, String> {
    const MESSAGE: &str = "A visibility condition requires a timer reference.";
    let id = string(value, "timerId", MESSAGE)?;
    let name = value["timerName"].as_str().ok_or(MESSAGE)?;
    Ok(map(json!({"timerId": id, "timerName": name})))
}

fn object_reference(value: &Value) -> Result<Map<String, Value>, String> {
    const MESSAGE: &str = "A visibility condition requires an object reference.";
    let id = string(value, "objectId", MESSAGE)?;
    let name = value["objectName"].as_str().ok_or(MESSAGE)?;
    Ok(map(json!({"objectId": id, "objectName": name})))
}

fn input_reference(value: &Value) -> Result<Map<String, Value>, String> {
    const MESSAGE: &str = "Invalid video input reference.";
    let id = string(value, "inputId", MESSAGE)?;
    let position = value["inputIndex"]
        .as_u64()
        .ok_or("Invalid video input index.")?;
    Ok(map(json!({"inputIndex": position, "inputId": id})))
}

fn make_condition(value: &Value) -> Result<Value, String> {
    let kind = if value.is_object() {
        value["kind"].as_str().unwrap_or("timer")
    } else {
        "timer"
    };
    let condition = match kind {
        "timer" => {
            let mut map = timer_reference(value)?;
            map.insert("criterion".into(), json!("has-time-remaining"));
            map
        }
        "element" => {
            let mut map = object_reference(value)?;
            map.insert("kind".into(), json!("element"));
            map.insert("criterion".into(), json!("has-text"));
            map
        }
        "video" | "audio" => map(json!({"kind": kind, "criterion": "has-time-remaining"})),
        "capture" => map(json!({"kind": "capture", "criterion": "active"})),
        "video-input" if value.get("inputIndex").is_some() => {
            let mut map = input_reference(value)?;
            map.insert("kind".into(), json!("video-input"));
            map.insert("criterion".into(), json!("active"));
            map
        }
        _ => return Err("Invalid visibility condition kind.".into()),
    };
    Ok(Value::Object(condition))
}

fn update_condition(control: &str, condition: &Value, value: &Value) -> Result<Value, String> {
    let mut next = condition.clone();
    let fields = match control {
        "Kind" => return make_condition(value),
        "Timer" => timer_reference(value)?,
        "Object" => object_reference(value)?,
        "Input" => input_reference(value)?,
        _ => {
            let criterion = value
                .as_str()
                .filter(|criterion| criteria(kind(condition)).contains(criterion))
                .ok_or("Invalid visibility condition criterion.")?;
            next["criterion"] = json!(criterion);
            return Ok(next);
        }
    };
    next.as_object_mut()
        .ok_or("Invalid visibility condition.")?
        .extend(fields);
    Ok(next)
}

fn visibility(layer: &Value, field: &str, value: &Value) -> Result<Value, String> {
    let mut next = layer.clone();
    if field == "visibilityEnabled" {
        let Some(enabled) = value.as_bool() else {
            return Err("Visibility enabled must be a boolean.".into());
        };
        if enabled {
            if !layer["visibilityLink"].is_object() {
                next["visibilityLink"] = json!({"criterion": "all", "conditions": []});
            }
        } else {
            next.as_object_mut()
                .ok_or("The object must be an object.")?
                .remove("visibilityLink");
        }
        return Ok(next);
    }
    if !layer["visibilityLink"].is_object() {
        return Err("Enable conditional visibility before editing its conditions.".into());
    }
    let link = &mut next["visibilityLink"];
    if field == "visibilityCriterion" {
        if !matches!(value.as_str(), Some("all" | "any" | "none")) {
            return Err("Invalid visibility criterion.".into());
        }
        link["criterion"] = value.clone();
        return Ok(next);
    }
    if field == "visibilityAdd" {
        let condition = make_condition(value)?;
        if !link["conditions"].is_array() {
            link["conditions"] = json!([]);
        }
        link["conditions"]
            .as_array_mut()
            .ok_or("Invalid visibility conditions.")?
            .push(condition);
        return Ok(next);
    }
    let (control, row) = field
        .strip_prefix("visibility")
        .and_then(|rest| rest.split_once(':'))
        .filter(|(control, _)| {
            matches!(
                *control,
                "Condition" | "Timer" | "Kind" | "Object" | "Input" | "Remove"
            )
        })
        .ok_or("Unknown visibility control.")?;
    let row = row
        .parse::<usize>()
        .map_err(|_| "Unknown visibility control.")?;
    let conditions = link["conditions"]
        .as_array_mut()
        .filter(|conditions| row < conditions.len())
        .ok_or("Visibility condition does not exist.")?;
    if control == "Remove" {
        conditions.remove(row);
    } else {
        conditions[row] = update_condition(control, &conditions[row], value)?;
    }
    Ok(next)
}
