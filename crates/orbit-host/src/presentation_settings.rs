use serde_json::{json, Value};

pub(crate) fn patch(item: &mut Value, field: &str, value: &Value) -> Result<(), String> {
    match field {
        "title" | "author" | "publisher" | "artistCredits" | "album" => {
            let text = value.as_str().ok_or("Song metadata must be text.")?;
            if item["ccli"].is_null() {
                item["ccli"] = json!({});
            }
            let ccli = item["ccli"]
                .as_object_mut()
                .ok_or("CCLI must be an object.")?;
            ccli.insert(field.into(), json!(text.trim()));
        }
        "copyrightYear" | "songNumber" => {
            if !value.is_null()
                && !value
                    .as_u64()
                    .is_some_and(|number| number <= u64::from(u32::MAX))
            {
                return Err(
                    "CCLI year and song number must be unsigned 32-bit integers or null.".into(),
                );
            }
            if item["ccli"].is_null() {
                item["ccli"] = json!({});
            }
            let ccli = item["ccli"]
                .as_object_mut()
                .ok_or("CCLI must be an object.")?;
            if value.is_null() {
                ccli.remove(field);
            } else {
                ccli.insert(field.into(), value.clone());
            }
        }
        "display" | "artwork" => {
            if field == "display" && !value.is_boolean() {
                return Err("CCLI display must be a boolean.".into());
            }
            if field == "artwork" && !value.is_null() {
                crate::macro_image::validate_data(
                    value
                        .as_str()
                        .ok_or("Artwork must be base64 image data or null.")?,
                )?;
            }
            if item["ccli"].is_null() {
                item["ccli"] = json!({});
            }
            let ccli = item["ccli"]
                .as_object_mut()
                .ok_or("CCLI must be an object.")?;
            if value.is_null() {
                ccli.remove(field);
            } else {
                ccli.insert(field.into(), value.clone());
            }
        }
        "size" | "background" | "backgroundEnabled" => {
            match field {
                "size" => {
                    let size = value
                        .as_object()
                        .ok_or("Presentation size must be an object.")?;
                    if size.len() != 2
                        || ["width", "height"].iter().any(|key| {
                            !size
                                .get(*key)
                                .and_then(Value::as_f64)
                                .is_some_and(|n| n.is_finite() && n > 0. && n <= 16384.)
                        })
                    {
                        return Err(
                            "Presentation dimensions must be positive and at most 16384 pixels."
                                .into(),
                        );
                    }
                }
                "background" => {
                    let color = value
                        .as_str()
                        .ok_or("Presentation background must be a color.")?;
                    if color != "transparent"
                        && !(matches!(color.len(), 7 | 9)
                            && color.starts_with('#')
                            && color[1..].bytes().all(|b| b.is_ascii_hexdigit()))
                    {
                        return Err(
                            "Presentation background must be a hex color or transparent.".into(),
                        );
                    }
                }
                _ if !value.is_boolean() => {
                    return Err("Presentation background state must be a boolean.".into())
                }
                _ => {}
            }
            let slides = item["slides"]
                .as_array_mut()
                .ok_or("Presentation slides must be an array.")?;
            for slide in slides {
                apply(slide, field, value)?;
            }
            item[field] = value.clone();
        }
        _ => return Err("Unsupported presentation setting.".into()),
    }
    Ok(())
}

fn apply(slide: &mut Value, field: &str, value: &Value) -> Result<(), String> {
    let slide = slide
        .as_object_mut()
        .ok_or("Presentation slides must be objects.")?;
    if field == "size" {
        slide.insert("width".into(), value["width"].clone());
        slide.insert("height".into(), value["height"].clone());
    } else {
        slide.insert(field.into(), value.clone());
    }
    Ok(())
}

pub(crate) fn apply_defaults(item: &Value, slide: &mut Value) -> Result<(), String> {
    for field in ["size", "background", "backgroundEnabled"] {
        if let Some(value) = item.get(field) {
            apply(slide, field, value)?;
        }
    }
    Ok(())
}

pub(crate) fn shortcut(value: &Value) -> Result<String, String> {
    let key = value
        .as_str()
        .ok_or("Slide shortcut must be text.")?
        .trim()
        .to_ascii_lowercase();
    if !key.is_empty() && !(key.len() == 1 && key.bytes().all(|b| b.is_ascii_alphabetic())) {
        return Err("Slide shortcut must be a single letter from A to Z.".into());
    }
    Ok(key)
}
