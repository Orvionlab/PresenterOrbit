//! Song metadata (CCLI) of a presentation: `song-metadata-controller.ts` and `shared/song-metadata.ts`.

use gpui::{App, Task};
use serde_json::{json, Value};
use std::sync::Arc;

use super::model::Item;
use crate::state::runtime::Runtime;

/// The text of the five editor fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub title: String,
    pub author: String,
    pub publisher: String,
    pub copyright_year: String,
    pub song_number: String,
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or("").to_string()
}

fn number(value: &Value) -> String {
    value.as_u64().map(|n| n.to_string()).unwrap_or_default()
}

/// `songMetadataDraft`.
pub fn draft(item: &Item) -> Draft {
    let ccli = &item.value["ccli"];
    Draft {
        title: text(&ccli["title"]),
        author: text(&ccli["author"]),
        publisher: text(&ccli["publisher"]),
        copyright_year: number(&ccli["copyrightYear"]),
        song_number: number(&ccli["songNumber"]),
    }
}

/// `applySongMetadata`: the item with its `ccli` from the draft. A year or number must be empty or a whole number
/// from 0 to 4294967295; anything else is the translation key `songMetadata.numberInvalid`.
pub fn apply(item: &Value, draft: &Draft) -> Result<Value, &'static str> {
    let mut ccli = item["ccli"].as_object().cloned().unwrap_or_default();
    ccli.insert("title".into(), json!(draft.title.trim()));
    ccli.insert("author".into(), json!(draft.author.trim()));
    ccli.insert("publisher".into(), json!(draft.publisher.trim()));
    for (key, value) in [
        ("copyrightYear", &draft.copyright_year),
        ("songNumber", &draft.song_number),
    ] {
        let value = value.trim();
        if value.is_empty() {
            ccli.remove(key);
        } else if value.chars().all(|c| c.is_ascii_digit()) {
            let digits = value.trim_start_matches('0');
            match if digits.is_empty() {
                Ok(0)
            } else {
                digits.parse::<u64>()
            } {
                Ok(parsed) if parsed <= 4_294_967_295 => {
                    ccli.insert(key.into(), json!(parsed));
                }
                _ => return Err("songMetadata.numberInvalid"),
            }
        } else {
            return Err("songMetadata.numberInvalid");
        }
    }
    let mut next = item.clone();
    next["ccli"] = Value::Object(ccli);
    Ok(next)
}

/// `formatSongMetadata`: the linked text the metadata produces.
pub fn format(ccli: &Value) -> String {
    if !ccli.is_object() || ccli["display"] == false {
        return String::new();
    }
    let copyright = [&ccli["copyrightYear"], &ccli["publisher"]]
        .iter()
        .filter_map(|value| match value {
            Value::String(text) if !text.is_empty() => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    let author = if ccli["author"].is_null() {
        &ccli["artistCredits"]
    } else {
        &ccli["author"]
    };
    [
        text(&ccli["title"]),
        match author {
            Value::String(text) => text.clone(),
            _ => String::new(),
        },
        copyright,
        match &ccli["songNumber"] {
            Value::Null => String::new(),
            number => format!(
                "CCLI Song #{}",
                number
                    .as_u64()
                    .map_or_else(|| number.to_string(), |n| n.to_string())
            ),
        },
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

/// `saveSongMetadata`: saves the item through the workspace lane and adopts the item the host accepted. It fails
/// with a translation key when the workspace or the presentation changed meanwhile.
pub fn save(
    runtime: &Runtime,
    cx: &mut App,
    item_id: &str,
    draft: &Draft,
) -> Task<Result<(), String>> {
    let runtime = runtime.clone();
    let (workspace, item) = {
        let content = runtime.content.read(cx);
        (
            content.workspace_path.clone(),
            content.items.get(item_id).cloned(),
        )
    };
    let draft = draft.clone();
    let item_id = item_id.to_string();
    cx.spawn(async move |cx| {
        if workspace.is_empty() {
            return Err("songMetadata.workspaceRequired".to_string());
        }
        let Some(item) = item else {
            return Err("songMetadata.stale".to_string());
        };
        apply(&item.value, &draft).map_err(str::to_string)?;
        let host = runtime.host.clone();
        let path = workspace.clone();
        let target = item_id.clone();
        let original_ccli = item.value["ccli"].clone();
        let answer = runtime
            .lanes
            .workspace(&workspace, move |revision| {
                let current = host.invoke("readWorkspaceSnapshot", vec![json!(path)])?;
                let current = current["workspace"]["library"]
                    .as_array()
                    .and_then(|items| items.iter().find(|item| item["id"] == target))
                    .ok_or("songMetadata.stale")?;
                if current["ccli"] != original_ccli {
                    return Err("songMetadata.stale".into());
                }
                let sent = apply(current, &draft).map_err(str::to_string)?;
                host.invoke("saveLibraryItem", vec![json!(path), sent, json!(revision)])
            })
            .await
            .map_err(|_| "The workspace queue stopped.".to_string())??;
        let accepted = answer["workspace"]["library"]
            .as_array()
            .and_then(|list| {
                list.iter()
                    .find(|candidate| candidate["id"] == item_id.as_str())
            })
            .ok_or("songMetadata.responseInvalid")?
            .clone();
        let parsed = Item::parse(&accepted)?;
        cx.update(|cx| {
            let content = runtime.content.read(cx);
            if content.workspace_path != workspace
                || !content
                    .items
                    .get(&item_id)
                    .is_some_and(|current| Arc::ptr_eq(&current.value, &item.value))
            {
                return Err("songMetadata.stale".to_string());
            }
            runtime.content.update(cx, |content, cx| {
                let items = content.items.map_item(&item_id, |_| parsed.clone());
                content.replace_items(items);
                cx.notify();
            });
            Ok(())
        })
    })
}
