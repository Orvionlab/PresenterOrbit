use serde_json::{json, Map, Value};
use std::ops::Range;

pub(super) fn run_color(value: &str) -> Option<String> {
    let hex = super::normalize_hex(value, "");
    if !hex.is_empty() {
        return Some(hex);
    }
    let channels = value
        .trim()
        .strip_prefix("rgb(")
        .or_else(|| value.trim().strip_prefix("rgba("))?
        .strip_suffix(')')?;
    let mut channels = channels.split(',');
    let mut next = || channels.next()?.trim().parse::<u8>().ok();
    let (red, green, blue) = (next()?, next()?, next()?);
    Some(format!("#{red:02x}{green:02x}{blue:02x}"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunDocument {
    pub runs: Vec<Value>,
}
impl RunDocument {
    pub fn new(runs: Value) -> Result<Self, String> {
        let runs = runs
            .as_array()
            .ok_or("Text runs must be an array.")?
            .clone();
        for run in &runs {
            if !run["text"].is_string() || run.get("style").is_some_and(|style| !style.is_object())
            {
                return Err("Text runs require text and an optional style object.".into());
            }
        }
        Ok(Self { runs })
    }
    pub fn text(&self) -> String {
        self.runs
            .iter()
            .filter_map(|run| run["text"].as_str())
            .collect()
    }
    pub fn value(&self, notes: bool) -> Value {
        let runs = self
            .runs
            .iter()
            .filter_map(|run| {
                let text = run["text"].as_str()?;
                if text.is_empty() {
                    return None;
                }
                let mut next = json!({"text":text});
                let style: Map<String, Value> = run["style"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(key, value)| {
                        matches!(
                            key.as_str(),
                            "bold"
                                | "italic"
                                | "underline"
                                | "strikethrough"
                                | "fontFamily"
                                | "fontSize"
                                | "textColor"
                                | "letterSpacing"
                                | "superscript"
                                | "underlineStyle"
                                | "underlinePattern"
                                | "underlineByWord"
                                | "underlineColor"
                                | "strikethroughStyle"
                                | "strikethroughPattern"
                                | "strikethroughByWord"
                                | "strikethroughColor"
                        ) && (notes
                            || !value.is_null()
                                && **value != false
                                && **value != ""
                                && **value != 0)
                    })
                    .map(|(key, value)| {
                        let value = if !notes && key == "fontFamily" {
                            let name = value.as_str().unwrap_or("");
                            let name = name
                                .strip_prefix('"')
                                .or_else(|| name.strip_prefix('\''))
                                .unwrap_or(name);
                            let name = name
                                .strip_suffix('"')
                                .or_else(|| name.strip_suffix('\''))
                                .unwrap_or(name);
                            json!(name)
                        } else if !notes && key == "textColor" {
                            run_color(value.as_str().unwrap_or(""))
                                .map(|color| json!(&color[..7]))
                                .unwrap_or_else(|| value.clone())
                        } else {
                            value.clone()
                        };
                        (key.clone(), value)
                    })
                    .collect();
                if !style.is_empty() {
                    next["style"] = Value::Object(style);
                }
                Some(next)
            })
            .collect::<Vec<_>>();
        json!(runs)
    }
    pub(super) fn style_at(&self, offset: usize) -> Option<&Value> {
        let mut end = 0;
        self.runs
            .iter()
            .find(|run| {
                end += run["text"].as_str().unwrap().len();
                offset < end
            })
            .and_then(|run| run.get("style"))
    }
    fn span(&self, range: Range<usize>) -> Vec<Value> {
        let mut offset = 0;
        self.runs
            .iter()
            .filter_map(|run| {
                let text = run["text"].as_str().unwrap();
                let end = offset + text.len();
                let start = range.start.max(offset);
                let stop = range.end.min(end);
                let value = if start < stop {
                    let mut run = run.clone();
                    run["text"] = json!(&text[start - offset..stop - offset]);
                    Some(run)
                } else {
                    None
                };
                offset = end;
                value
            })
            .collect()
    }
    pub fn replace(&mut self, range: Range<usize>, inserted: &str) -> Result<(), String> {
        let text = self.text();
        if range.start > range.end || text.get(range.clone()).is_none() {
            return Err("Invalid text replacement boundary.".into());
        }
        let mut next = self.span(0..range.start);
        let inherited = self
            .runs
            .iter()
            .scan(0, |offset, run| {
                let start = *offset;
                *offset += run["text"].as_str().unwrap().len();
                Some((start, *offset, run))
            })
            .find(|(start, end, _)| {
                if !range.is_empty() {
                    *start <= range.start && *end > range.start
                } else if range.start == 0 {
                    *start == 0 && *end > 0
                } else {
                    *start < range.start && *end >= range.start
                }
            })
            .and_then(|(_, _, run)| run.get("style"))
            .cloned();
        if !inserted.is_empty() {
            let mut run = json!({"text":inserted});
            if let Some(style) = inherited {
                run["style"] = style;
            }
            next.push(run);
        }
        next.extend(self.span(range.end..text.len()));
        self.runs = next;
        self.coalesce();
        Ok(())
    }
    pub fn format(&mut self, range: Range<usize>, field: &str, value: Value) -> Result<(), String> {
        let text = self.text();
        if range.start == range.end {
            return Ok(());
        }
        if range.start > range.end || text.get(range.clone()).is_none() {
            return Err("Invalid text formatting boundary.".into());
        }
        let mut next = self.span(0..range.start);
        for mut run in self.span(range.clone()) {
            if !run["style"].is_object() {
                run["style"] = json!({});
            }
            run["style"][field] = value.clone();
            next.push(run);
        }
        next.extend(self.span(range.end..text.len()));
        self.runs = next;
        self.coalesce();
        Ok(())
    }
    fn coalesce(&mut self) {
        let mut next: Vec<Value> = Vec::new();
        for run in self.runs.drain(..) {
            if let Some(previous) = next
                .last_mut()
                .filter(|previous| previous.get("style") == run.get("style"))
            {
                previous["text"] = json!(format!(
                    "{}{}",
                    previous["text"].as_str().unwrap(),
                    run["text"].as_str().unwrap()
                ));
            } else {
                next.push(run);
            }
        }
        self.runs = next;
    }
}
