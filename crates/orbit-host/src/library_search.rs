use std::collections::HashSet;

use serde_json::{json, Value};

#[derive(Default)]
pub struct LibrarySearchIndex {
    session_id: String,
    generation: u64,
    entries: Vec<String>,
    retired_session_ids: HashSet<String>,
}

impl LibrarySearchIndex {
    pub fn replace(
        &mut self,
        session_id: &str,
        generation: u64,
        library: &Value,
    ) -> Result<Value, String> {
        if session_id.trim().is_empty() || generation == 0 {
            return Err("Library search index identity is invalid.".to_string());
        }
        if self.retired_session_ids.contains(session_id)
            || (self.session_id == session_id && generation < self.generation)
        {
            return Ok(json!({
                "ok": false,
                "stale": true,
                "generation": self.generation,
                "itemCount": self.entries.len(),
            }));
        }
        if self.session_id == session_id && generation == self.generation {
            return Ok(json!({
                "ok": true,
                "stale": false,
                "generation": self.generation,
                "itemCount": self.entries.len(),
            }));
        }
        let items = library
            .as_array()
            .ok_or_else(|| "Library search index items must be an array.".to_string())?;
        let entries = items
            .iter()
            .map(library_item_search_text)
            .collect::<Result<Vec<_>, _>>()?;
        if !self.session_id.is_empty() && self.session_id != session_id {
            self.retired_session_ids.insert(self.session_id.clone());
        }
        self.session_id = session_id.to_string();
        self.generation = generation;
        self.entries = entries;
        Ok(json!({
            "ok": true,
            "stale": false,
            "generation": generation,
            "itemCount": self.entries.len(),
        }))
    }

    pub fn search(&self, session_id: &str, generation: u64, query: &str) -> Value {
        if self.session_id != session_id || self.generation != generation {
            return json!({
                "ok": false,
                "stale": true,
                "generation": self.generation,
                "query": query,
                "ordinals": [],
            });
        }
        let terms = query
            .trim()
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let ordinals = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(ordinal, haystack)| {
                terms
                    .iter()
                    .all(|term| haystack.contains(term))
                    .then_some(ordinal)
            })
            .collect::<Vec<_>>();
        json!({
            "ok": true,
            "stale": false,
            "generation": generation,
            "query": query,
            "ordinals": ordinals,
        })
    }
}

fn library_item_search_text(item: &Value) -> Result<String, String> {
    let item = item
        .as_object()
        .ok_or_else(|| "Library search items must be objects.".to_string())?;
    let mut values = vec![
        string_field(item.get("title")),
        string_field(item.get("subtitle")),
        string_field(item.get("author")),
        string_field(item.get("operatorNotes")),
    ];
    if let Some(slides) = item.get("slides") {
        let slides = slides
            .as_array()
            .ok_or_else(|| "Library search item slides must be an array.".to_string())?;
        values.extend(slides.iter().map(|slide| {
            slide
                .as_object()
                .map(|slide| string_field(slide.get("content")))
                .unwrap_or_default()
        }));
    }
    Ok(values.join(" ").to_lowercase())
}

fn string_field(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
