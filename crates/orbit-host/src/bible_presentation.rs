use crate::bible::{Passage, PassageRequest, Store};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::{collections::HashSet, path::Path};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    library_id: String,
    playlist_id: Option<String>,
    presentation_theme_id: String,
    text_layer_ids: Vec<String>,
    reference_layer_id: Option<String>,
    passage: PassageRequest,
    options: Options,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    show_verse_numbers: bool,
    break_on_verse: bool,
    show_translation: bool,
    reference_mode: ReferenceMode,
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
enum ReferenceMode {
    Verse,
    PassageEach,
    PassageLast,
    None,
}

pub fn create(
    path: &Path,
    store: &Store,
    request: &Request,
) -> Result<(Value, String, Option<String>), String> {
    let current = crate::workspace::read_workspace(path)?;
    let document = current.as_object().ok_or("Workspace must be an object.")?;
    let layout = crate::workspace::content_themes::layout(document, &request.presentation_theme_id)
        .ok_or("Choose an available scripture theme layout.")?;
    validate_mapping(&layout, request)?;
    let passage = store.passage(&request.passage)?;
    let slides = generate_slides(&layout, &passage, request)?;
    let mut libraries = array(&current, "libraries")?;
    let library = libraries
        .iter_mut()
        .find(|entry| entry["id"].as_str() == Some(&request.library_id))
        .ok_or("Choose an existing library for the scripture presentation.")?;
    let item_id = Uuid::new_v4().to_string();
    library["itemIds"]
        .as_array_mut()
        .ok_or("Library itemIds must be an array.")?
        .push(json!(item_id));
    let mut playlist = array(&current, "playlist")?;
    let entry_id = append_to_playlist(&mut playlist, request.playlist_id.as_deref(), &item_id)?;
    let primary = passage
        .translations
        .first()
        .ok_or("Select at least one translation.")?;
    let mut items = array(&current, "library")?;
    items.push(json!({"id":item_id,"title":passage.reference,"sourceType":"presenter-orbit","sourcePath":"",
        "bibleReference":{"bookKey":passage.book_id,"book":passage.book_name,
            "chapterStart":passage.chapter_start,"chapterEnd":passage.chapter_end,
            "verseStart":passage.verse_start,"verseEnd":passage.verse_end,
            "translation":primary.translation.abbreviation,
            "translations":passage.translations.iter().map(|entry| &entry.translation).collect::<Vec<_>>()},
        "slides":slides}));
    let saved = crate::workspace::save_workspace(
        path,
        &json!({"libraries":libraries,"library":items,
        "playlist":playlist,"lastUsedPresentationThemeId":request.presentation_theme_id}),
    )?;
    Ok((saved, item_id, entry_id))
}

fn validate_mapping(layout: &Value, request: &Request) -> Result<(), String> {
    if request.text_layer_ids.is_empty()
        || request.text_layer_ids.len() != request.passage.translation_ids.len()
    {
        return Err("Map one distinct theme text object to each selected translation.".into());
    }
    if request.options.reference_mode == ReferenceMode::Verse && !request.options.break_on_verse {
        return Err("Per-verse references require a slide break on each verse.".into());
    }
    let text_ids = layout["layers"]
        .as_array()
        .ok_or("The scripture theme requires text objects.")?
        .iter()
        .filter_map(Value::as_object)
        .filter(|layer| crate::scene::is_text_layer(layer))
        .filter_map(|layer| layer.get("id").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    let mut used = HashSet::new();
    for id in request
        .text_layer_ids
        .iter()
        .chain(request.reference_layer_id.iter())
    {
        if !text_ids.contains(id.as_str()) || !used.insert(id) {
            return Err("Scripture text and reference mappings must use distinct existing theme text objects.".into());
        }
    }
    Ok(())
}

fn generate_slides(
    layout: &Value,
    passage: &Passage,
    request: &Request,
) -> Result<Vec<Value>, String> {
    let count = passage
        .translations
        .first()
        .ok_or("Select at least one translation.")?
        .verses
        .len();
    if count == 0 || count > 500 {
        return Err("Select a passage containing 1–500 verses.".into());
    }
    let width = dimension(layout, "width")?;
    let height = dimension(layout, "height")?;
    let mut renderer = crate::native_render::NativeRenderRuntime::new();
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < count {
        let mut end = start;
        while end < count {
            let reference = reference_for_slide(passage, request, start, end + 1 == count);
            let texts = slide_texts(passage, request, start..end + 1, &reference);
            if !texts_fit(layout, request, &texts, &reference, &mut renderer)? {
                break;
            }
            end += 1;
            if request.options.break_on_verse {
                break;
            }
        }
        if end == start {
            return Err("A scripture verse or its attribution does not fit the mapped theme text objects. Enlarge the objects or reduce their font size.".into());
        }
        ranges.push(start..end);
        start = end;
    }
    ranges.iter().enumerate().map(|(index, range)| {
        let reference = reference_for_slide(passage, request, range.start, index + 1 == ranges.len());
        let texts = slide_texts(passage, request, range.clone(), &reference);
        let mut overrides = Map::new();
        for (translation_index, text) in texts.iter().enumerate() {
            overrides.insert(request.text_layer_ids[translation_index].clone(), json!({"text":text,"textStyle":{"linkedTextEnabled":false}}));
        }
        if let Some(id) = &request.reference_layer_id {
            overrides.insert(id.clone(), json!({"text":reference,"textStyle":{"linkedTextEnabled":false}}));
        }
        Ok(json!({"id":Uuid::new_v4().to_string(),"name":reference,"label":index + 1,
            "content":texts[0],"width":width,"height":height,
            "presentationThemeId":request.presentation_theme_id,
            "primaryTextLayerId":request.text_layer_ids[0],"themeLayerOverrides":overrides,"layers":[]}))
    }).collect()
}

fn slide_texts(
    passage: &Passage,
    request: &Request,
    range: std::ops::Range<usize>,
    reference: &str,
) -> Vec<String> {
    passage
        .translations
        .iter()
        .enumerate()
        .map(|(index, translation)| {
            let mut text = translation.verses[range.clone()]
                .iter()
                .map(|verse| {
                    if !request.options.show_verse_numbers {
                        return verse.text.clone();
                    }
                    let number = if passage.chapter_start == passage.chapter_end {
                        verse.verse.to_string()
                    } else {
                        format!("{}:{}", verse.chapter, verse.verse)
                    };
                    format!("{number} {}", verse.text)
                })
                .collect::<Vec<_>>()
                .join("\n");
            if request.options.show_translation {
                text.push_str(&format!("\n{}", translation.translation.abbreviation));
            }
            if let Some(notice) = translation
                .translation
                .license
                .notice
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                text.push_str(&format!("\n{notice}"));
            }
            if index == 0 && request.reference_layer_id.is_none() && !reference.is_empty() {
                text.push_str(&format!("\n{reference}"));
            }
            text
        })
        .collect()
}

fn texts_fit(
    layout: &Value,
    request: &Request,
    texts: &[String],
    reference: &str,
    renderer: &mut crate::native_render::NativeRenderRuntime,
) -> Result<bool, String> {
    let layers = layout["layers"]
        .as_array()
        .ok_or("The scripture theme requires text objects.")?;
    for (id, text) in request
        .text_layer_ids
        .iter()
        .zip(texts.iter().map(String::as_str))
        .chain(request.reference_layer_id.iter().map(|id| (id, reference)))
    {
        if text.is_empty() {
            continue;
        }
        let layer = layers
            .iter()
            .find(|layer| layer["id"].as_str() == Some(id))
            .ok_or("Mapped scripture text object is missing.")?;
        let bounds = &layer["bounds"];
        let width = dimension(bounds, "width")?;
        let height = dimension(bounds, "height")?;
        let mut style = layout["textStyle"].as_object().cloned().unwrap_or_default();
        if let Some(overrides) = layer["textStyle"].as_object() {
            style.extend(overrides.clone());
        }
        let size = style
            .get("fontSize")
            .and_then(Value::as_f64)
            .unwrap_or(90.0)
            .max(1.0);
        let text = crate::native_compositor::apply_capitalization(
            text,
            style
                .get("capitalization")
                .and_then(Value::as_str)
                .unwrap_or("None"),
        );
        let text = crate::native_compositor::apply_line_transform(
            &text,
            style
                .get("lineTransform")
                .and_then(Value::as_str)
                .unwrap_or("None"),
            style.get("lineTransformDelimiter").and_then(Value::as_str),
        );
        if !renderer.text_fits_box(
            &text,
            width.round() as usize,
            height.round() as usize,
            size as f32,
            &style,
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn reference_for_slide(passage: &Passage, request: &Request, index: usize, last: bool) -> String {
    match request.options.reference_mode {
        ReferenceMode::Verse => {
            let verse = &passage.translations[0].verses[index];
            format!("{} {}:{}", verse.book_name, verse.chapter, verse.verse)
        }
        ReferenceMode::PassageEach => passage.reference.clone(),
        ReferenceMode::PassageLast if last => passage.reference.clone(),
        _ => String::new(),
    }
}

fn dimension(layout: &Value, field: &str) -> Result<f64, String> {
    layout[field]
        .as_f64()
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| format!("The scripture theme requires a positive {field}."))
}

fn array(workspace: &Value, field: &str) -> Result<Vec<Value>, String> {
    workspace[field]
        .as_array()
        .cloned()
        .ok_or_else(|| format!("Workspace {field} must be an array."))
}

fn append_to_playlist(
    playlists: &mut [Value],
    playlist_id: Option<&str>,
    item_id: &str,
) -> Result<Option<String>, String> {
    let Some(id) = playlist_id else {
        return Ok(None);
    };
    let playlist = playlists
        .iter_mut()
        .find(|entry| entry["id"].as_str() == Some(id) && entry["type"] != "folder")
        .ok_or("Choose an existing destination playlist, rather than a folder.")?;
    let entry_id = Uuid::new_v4().to_string();
    playlist["items"]
        .as_array_mut()
        .ok_or("Playlist entries must be an array.")?
        .push(json!({"id":entry_id,"type":"presentation","itemId":item_id}));
    playlist["itemIds"]
        .as_array_mut()
        .ok_or("Playlist itemIds must be an array.")?
        .push(json!(item_id));
    Ok(Some(entry_id))
}
