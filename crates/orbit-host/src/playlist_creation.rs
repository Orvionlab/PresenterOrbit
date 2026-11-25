use super::*;

#[derive(serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Entry {
    Header {
        title: String,
        color: Option<String>,
    },
    Placeholder {
        title: String,
        linked_item_id: Option<String>,
        arrangement_id: Option<String>,
    },
    Presentation {
        item_id: String,
        arrangement_id: Option<String>,
        theme_layout_id: Option<String>,
        size: Option<Size>,
    },
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Size {
    width: u32,
    height: u32,
}

pub(super) fn build_entries(
    document: &Map<String, Value>,
    input: &Value,
) -> Result<(Vec<Value>, Vec<Value>), String> {
    let input: Vec<Entry> = serde_json::from_value(input.clone())
        .map_err(|error| format!("Invalid playlist entries: {error}"))?;
    let mut library = value_array(document, "library")?;
    let mut entries = Vec::new();
    for entry in input {
        let entry = match entry {
            Entry::Header { title, color } => {
                let mut entry = named_entry("header", &title, &entries)?;
                let color = color.as_deref().unwrap_or("#64748b");
                validate_playlist_header_color(color)?;
                entry["color"] = json!(color);
                entry
            }
            Entry::Placeholder {
                title,
                linked_item_id,
                arrangement_id,
            } => {
                let mut entry = named_entry("placeholder", &title, &entries)?;
                if let Some(id) = linked_item_id {
                    entry["linkedItemId"] = json!(id);
                    set_arrangement(&mut entry, presentation(&mut library, &id)?, arrangement_id)?;
                } else if arrangement_id.is_some() {
                    return Err("Selecting an arrangement requires a linked presentation.".into());
                }
                entry
            }
            Entry::Presentation {
                item_id,
                arrangement_id,
                theme_layout_id,
                size,
            } => {
                let item = presentation(&mut library, &item_id)?;
                let mut entry = json!({
                    "id":unique_presentation_entry_id(&item_id, &entries),
                    "type":"presentation","itemId":item_id
                });
                set_arrangement(&mut entry, item, arrangement_id)?;
                if let Some(assignment) = theme_layout_id {
                    if content_themes::layout(document, &assignment).is_none() {
                        return Err("Choose an available theme layout.".into());
                    }
                    let mut slides = presentation_item_slides(item)?;
                    for slide in &mut slides {
                        content_themes::assign(document, slide, &assignment)?;
                    }
                    set_presentation_item_slides(item, slides)?;
                }
                if let Some(size) = size {
                    crate::presentation_settings::patch(
                        item,
                        "size",
                        &json!({"width":size.width,"height":size.height}),
                    )?;
                }
                entry
            }
        };
        entries.push(entry);
    }
    Ok((entries, library))
}

fn named_entry(kind: &str, title: &str, entries: &[Value]) -> Result<Value, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Playlist entry title is required.".into());
    }
    Ok(json!({"id":unique_playlist_entry_id(kind, title, entries),"type":kind,"title":title}))
}

fn presentation<'a>(library: &'a mut [Value], id: &str) -> Result<&'a mut Value, String> {
    library
        .iter_mut()
        .find(|item| item["id"] == id)
        .ok_or_else(|| format!("Presentation library item '{id}' was not found."))
}

fn set_arrangement(
    entry: &mut Value,
    item: &Value,
    arrangement: Option<String>,
) -> Result<(), String> {
    if let Some(id) = arrangement {
        if !item["arrangements"].as_array().is_some_and(|arrangements| {
            arrangements
                .iter()
                .any(|arrangement| arrangement["id"] == id)
        }) {
            return Err("Presentation arrangement was not found.".into());
        }
        entry["arrangementId"] = json!(id);
    }
    Ok(())
}
