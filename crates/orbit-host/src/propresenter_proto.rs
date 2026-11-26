//! Small, dependency-free ProPresenter protobuf reader/writer.
//!
//! The schemas in `vendor/ProPresenter7-Proto` are intentionally not generated
//! into the Tauri binary.  ProPresenter documents are protobuf wire messages,
//! and the importer only needs a stable subset of those messages.  This module
//! implements that subset directly using the field numbers from the vendored
//! schemas.  Unknown fields are skipped, so documents produced by newer
//! ProPresenter versions remain readable when they retain the known fields.

use std::{collections::BTreeMap, path::Path};

use serde_json::{json, Map, Value};
use uuid::Uuid;

#[derive(Clone, Debug)]
enum WireValue {
    Varint(u64),
    Fixed64(u64),
    Bytes(Vec<u8>),
    Fixed32(u32),
}

#[derive(Clone, Debug)]
struct Field {
    number: u32,
    value: WireValue,
}

fn decode_message(bytes: &[u8]) -> Result<Vec<Field>, String> {
    let mut index = 0;
    let mut fields = Vec::new();
    while index < bytes.len() {
        let key = read_varint(bytes, &mut index)?;
        let number = (key >> 3) as u32;
        let wire_type = (key & 0x07) as u8;
        if number == 0 {
            return Err("ProPresenter protobuf contains an invalid field number.".to_string());
        }
        let value = match wire_type {
            0 => WireValue::Varint(read_varint(bytes, &mut index)?),
            1 => {
                let end = index.checked_add(8).ok_or_else(|| {
                    "ProPresenter protobuf has an invalid fixed64 field.".to_string()
                })?;
                if end > bytes.len() {
                    return Err("ProPresenter protobuf has a truncated fixed64 field.".to_string());
                }
                let mut buffer = [0_u8; 8];
                buffer.copy_from_slice(&bytes[index..end]);
                index = end;
                WireValue::Fixed64(u64::from_le_bytes(buffer))
            }
            2 => {
                let length = read_varint(bytes, &mut index)? as usize;
                let end = index.checked_add(length).ok_or_else(|| {
                    "ProPresenter protobuf has an invalid length-delimited field.".to_string()
                })?;
                if end > bytes.len() {
                    return Err(
                        "ProPresenter protobuf has a truncated length-delimited field.".to_string(),
                    );
                }
                let value = bytes[index..end].to_vec();
                index = end;
                WireValue::Bytes(value)
            }
            5 => {
                let end = index.checked_add(4).ok_or_else(|| {
                    "ProPresenter protobuf has an invalid fixed32 field.".to_string()
                })?;
                if end > bytes.len() {
                    return Err("ProPresenter protobuf has a truncated fixed32 field.".to_string());
                }
                let mut buffer = [0_u8; 4];
                buffer.copy_from_slice(&bytes[index..end]);
                index = end;
                WireValue::Fixed32(u32::from_le_bytes(buffer))
            }
            // Groups are deprecated protobuf wire constructs and are not part
            // of the ProPresenter schemas we decode. They can still occur in
            // extension data written by older producers, so skip an unknown
            // group safely instead of rejecting the complete document.
            3 => {
                skip_group(bytes, &mut index, number, 0)?;
                continue;
            }
            4 => {
                return Err(
                    "ProPresenter protobuf contains an unexpected end-group field.".to_string(),
                )
            }
            _ => return Err("ProPresenter protobuf contains an unknown wire type.".to_string()),
        };
        fields.push(Field { number, value });
    }
    Ok(fields)
}

/// Protobuf implementations reject deeper group nesting than this.
const MAX_GROUP_DEPTH: usize = 100;

fn skip_group(
    bytes: &[u8],
    index: &mut usize,
    group_number: u32,
    depth: usize,
) -> Result<(), String> {
    if depth >= MAX_GROUP_DEPTH {
        return Err("ProPresenter protobuf nests groups too deeply.".to_string());
    }
    loop {
        let key = read_varint(bytes, index)?;
        let number = (key >> 3) as u32;
        let wire_type = (key & 0x07) as u8;
        if number == 0 {
            return Err("ProPresenter protobuf contains an invalid field number.".to_string());
        }
        if wire_type == 4 {
            return if number == group_number {
                Ok(())
            } else {
                Err("ProPresenter protobuf contains a mismatched end-group field.".to_string())
            };
        }
        skip_wire_value(bytes, index, number, wire_type, depth)?;
    }
}

fn skip_wire_value(
    bytes: &[u8],
    index: &mut usize,
    number: u32,
    wire_type: u8,
    depth: usize,
) -> Result<(), String> {
    match wire_type {
        0 => {
            read_varint(bytes, index)?;
        }
        1 => skip_bytes(bytes, index, 8, "fixed64")?,
        2 => {
            let length = read_varint(bytes, index)? as usize;
            skip_bytes(bytes, index, length, "length-delimited")?;
        }
        3 => skip_group(bytes, index, number, depth + 1)?,
        5 => skip_bytes(bytes, index, 4, "fixed32")?,
        4 => {
            return Err("ProPresenter protobuf contains an unexpected end-group field.".to_string())
        }
        _ => return Err("ProPresenter protobuf contains an unknown wire type.".to_string()),
    }
    Ok(())
}

fn skip_bytes(bytes: &[u8], index: &mut usize, length: usize, kind: &str) -> Result<(), String> {
    let end = index
        .checked_add(length)
        .ok_or_else(|| format!("ProPresenter protobuf has an invalid {kind} field."))?;
    if end > bytes.len() {
        return Err(format!(
            "ProPresenter protobuf has a truncated {kind} field."
        ));
    }
    *index = end;
    Ok(())
}

fn read_varint(bytes: &[u8], index: &mut usize) -> Result<u64, String> {
    let mut value = 0_u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes
            .get(*index)
            .ok_or_else(|| "ProPresenter protobuf has a truncated varint.".to_string())?;
        *index += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("ProPresenter protobuf contains an overlong varint.".to_string())
}

fn bytes_field<'a>(fields: &'a [Field], number: u32) -> Option<&'a [u8]> {
    fields
        .iter()
        .find_map(|field| {
            (field.number == number).then(|| match &field.value {
                WireValue::Bytes(value) => value.as_slice(),
                _ => &[] as &[u8],
            })
        })
        .filter(|value| !value.is_empty())
}

fn bytes_fields<'a>(fields: &'a [Field], number: u32) -> impl Iterator<Item = &'a [u8]> {
    fields.iter().filter_map(move |field| {
        if field.number != number {
            return None;
        }
        match &field.value {
            WireValue::Bytes(value) => Some(value.as_slice()),
            _ => None,
        }
    })
}

fn string_field(fields: &[Field], number: u32) -> Option<String> {
    bytes_field(fields, number).and_then(|value| String::from_utf8(value.to_vec()).ok())
}

fn varint_field(fields: &[Field], number: u32) -> Option<u64> {
    fields.iter().find_map(|field| {
        if field.number != number {
            return None;
        }
        match field.value {
            WireValue::Varint(value) => Some(value),
            _ => None,
        }
    })
}

fn f64_field(fields: &[Field], number: u32) -> Option<f64> {
    fields.iter().find_map(|field| {
        if field.number != number {
            return None;
        }
        match field.value {
            WireValue::Fixed64(value) => Some(f64::from_bits(value)),
            _ => None,
        }
    })
}

fn bool_field(fields: &[Field], number: u32) -> Option<bool> {
    varint_field(fields, number).map(|value| value != 0)
}

fn nested_field(fields: &[Field], number: u32) -> Option<Vec<Field>> {
    // Empty embedded messages still select a protobuf oneof (headers, placeholders, empty folders).
    bytes_fields(fields, number)
        .next()
        .and_then(|value| decode_message(value).ok())
}

fn nested_fields(fields: &[Field], number: u32) -> impl Iterator<Item = Vec<Field>> + '_ {
    bytes_fields(fields, number).filter_map(|value| decode_message(value).ok())
}

fn uuid(fields: &[Field], number: u32) -> Option<String> {
    nested_field(fields, number)
        .and_then(|value| string_field(&value, 1))
        .or_else(|| string_field(fields, number))
}

/// Names the entries of `named` whose field occurs in `fields`.
fn present_fields(fields: &[Field], named: &[(u32, &str)]) -> Vec<String> {
    named
        .iter()
        .filter(|(number, _)| fields.iter().any(|field| field.number == *number))
        .map(|(_, name)| name.to_string())
        .collect()
}

/// Lists source content this decoder does not convert on the object that owns it, so that
/// `import_export_report::import_findings` discloses it instead of dropping it silently.
fn note_unsupported(object: &mut Map<String, Value>, descriptions: Vec<String>) {
    if descriptions.is_empty() {
        return;
    }
    if let Value::Array(list) = object
        .entry("unsupportedSourceContent")
        .or_insert_with(|| json!([]))
    {
        list.extend(descriptions.into_iter().map(Value::from));
    }
}

/// Reports an enum value outside the vendored schemas; the caller keeps the owning object
/// but must not act on the value.
fn unknown_enum(name: &str, value: u64, max: u64, unsupported: &mut Vec<String>) -> bool {
    let unknown = value > max;
    if unknown {
        unsupported.push(format!("{name} value {value}"));
    }
    unknown
}

/// Newest ProPresenter release whose documents (the authentic 21.4.2 fixtures) and schemas
/// (the vendored 21.4) were compared with this decoder.
const VERIFIED_APPLICATION_VERSION: [u64; 3] = [21, 4, 2];

/// Describes the writer recorded in a document's `ApplicationInfo` (field 1) when it is a
/// ProPresenter release newer than the verified one, whose added fields are skipped.
pub fn newer_application_version(bytes: &[u8]) -> Option<String> {
    let fields = decode_message(bytes).ok()?;
    let info = nested_field(&fields, 1)?;
    // ApplicationInfo.Application 1 is ProPresenter; other applications number differently.
    if varint_field(&info, 3) != Some(1) {
        return None;
    }
    let version = nested_field(&info, 4)?;
    let part = |field| varint_field(&version, field).unwrap_or(0);
    let parts = [part(1), part(2), part(3)];
    (parts > VERIFIED_APPLICATION_VERSION).then(|| {
        format!(
            "ProPresenter {}.{}.{} (build {})",
            parts[0],
            parts[1],
            parts[2],
            string_field(&version, 4).unwrap_or_default()
        )
    })
}

/// Plain text of an RTF document with tabs reduced to spaces, for notes and labels.
fn rtf_to_text(data: &[u8]) -> String {
    rtf_text(data, false).into_iter().map(|(c, _)| c).collect()
}

/// Bold, italic, underline, strikethrough, color, font and size runs of the layer text, which add
/// up to `rtf_to_layer_text`. `None` when the formatting is the same throughout the text. A font
/// or size that is the same throughout is the font of the text layer and stays out of the runs.
fn rtf_to_layer_runs(data: &[u8]) -> Option<Vec<Value>> {
    rtf_to_runs(data, false)
}

fn rtf_to_runs(data: &[u8], notes: bool) -> Option<Vec<Value>> {
    let mut characters = rtf_layer_document(data).characters;
    let fonts = rtf_font_table(&String::from_utf8_lossy(data));
    let same_font = characters
        .windows(2)
        .all(|pair| pair[0].1.font == pair[1].1.font);
    let same_size = characters
        .windows(2)
        .all(|pair| pair[0].1.size == pair[1].1.size);
    for (_, style) in &mut characters {
        if same_font && !notes {
            style.font = None;
        }
        if same_size && !notes {
            style.size = None;
        }
    }
    if !notes && characters.windows(2).all(|pair| pair[0].1 == pair[1].1) {
        return None;
    }
    let mut runs: Vec<(String, RtfStyle)> = Vec::new();
    for (character, style) in characters {
        match runs.last_mut() {
            Some((text, last)) if *last == style => text.push(character),
            _ => runs.push((character.to_string(), style)),
        }
    }
    Some(
        runs.into_iter()
            .map(|(text, style)| {
                let mut format = Map::new();
                for (key, on) in [
                    ("bold", style.bold),
                    ("italic", style.italic),
                    ("underline", style.underline),
                    ("strikethrough", style.strikethrough),
                ] {
                    if on {
                        format.insert(key.to_string(), json!(true));
                    }
                }
                for (key, variant, pattern, by_word, color) in [
                    (
                        "underline",
                        style.underline_style,
                        style.underline_pattern,
                        style.underline_by_word,
                        style.underline_color,
                    ),
                    ("strikethrough", style.strikethrough_style, 0, false, None),
                ] {
                    if variant > 1 {
                        format.insert(
                            format!("{key}Style"),
                            json!(["single", "single", "thick", "double"][variant as usize]),
                        );
                    }
                    if pattern > 0 {
                        format.insert(
                            format!("{key}Pattern"),
                            json!(
                                ["solid", "dot", "dash", "dash-dot", "dash-dot-dot"]
                                    [pattern as usize]
                            ),
                        );
                    }
                    if by_word {
                        format.insert(format!("{key}ByWord"), json!(true));
                    }
                    if let Some([red, green, blue]) = color {
                        format.insert(
                            format!("{key}Color"),
                            json!(format!("#{red:02x}{green:02x}{blue:02x}")),
                        );
                    }
                }
                if let Some(spacing) = style.letter_spacing {
                    format.insert("letterSpacing".to_string(), json!(spacing));
                }
                if let Some(level) = style.superscript {
                    format.insert("superscript".to_string(), json!(level));
                }
                if let Some([red, green, blue]) = style.color {
                    format.insert(
                        "textColor".to_string(),
                        json!(format!("#{red:02x}{green:02x}{blue:02x}")),
                    );
                }
                if let Some(family) = style.font.and_then(|font| fonts.get(&font)) {
                    format.insert("fontFamily".to_string(), json!(family));
                }
                if let Some(half_points) = style.size {
                    format.insert("fontSize".to_string(), json!(f64::from(half_points) / 2.0));
                }
                if format.is_empty() {
                    json!({ "text": text })
                } else {
                    json!({ "text": text, "style": format })
                }
            })
            .collect(),
    )
}

/// Character formatting that an RTF group scope carries; the rest of the RTF formatting stays dropped.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RtfStyle {
    bold: bool,
    italic: bool,
    underline: bool,
    underline_style: u8,
    underline_pattern: u8,
    underline_by_word: bool,
    underline_color: Option<[u8; 3]>,
    strikethrough: bool,
    strikethrough_style: u8,
    letter_spacing: Option<f32>,
    superscript: Option<i8>,
    color: Option<[u8; 3]>,
    /// `\fN` font table index and `\fsN` size in half-points.
    font: Option<u32>,
    size: Option<u32>,
}

type RtfChar = (char, RtfStyle);

#[derive(Clone, Debug, Default, PartialEq)]
struct RtfParagraph {
    values: Map<String, Value>,
    tab_alignment: Option<&'static str>,
    tab_leader: Option<&'static str>,
    absolute_tabs: Vec<(f64, &'static str, Option<&'static str>)>,
    bar_tabs: Vec<f64>,
    unsupported_tab_alignment_pending: bool,
    unsupported_tab_stops: bool,
    list_override: Option<i32>,
    list_level: u8,
    legacy_list: Option<Map<String, Value>>,
    line_spacing: Option<i64>,
    line_spacing_multiple: bool,
    apple_line_spacing: Option<i64>,
    apple_minimum_line_height: Option<i64>,
    apple_maximum_line_height: Option<i64>,
    default_tab_interval_disabled: bool,
    paragraph_reset: bool,
    unsupported: Vec<String>,
}

impl RtfParagraph {
    fn style(
        &self,
        lists: &RtfListTables,
        proto_levels: &[Map<String, Value>],
    ) -> (Map<String, Value>, Vec<String>) {
        let mut style = self.values.clone();
        let mut unsupported = self.unsupported.clone();
        if !self.bar_tabs.is_empty() {
            let bars = json!(self.bar_tabs);
            match crate::native_text_tabs::parse_bar_stops(&bars) {
                Ok(_) => {
                    style.insert(crate::native_text_tabs::BAR_STOPS_FIELD.to_string(), bars);
                }
                Err(reason) => unsupported.push(reason),
            }
        }
        if !self.absolute_tabs.is_empty() {
            style.insert(
                crate::native_text_tabs::STOPS_FIELD.to_string(),
                Value::Array(
                    self.absolute_tabs
                        .iter()
                        .map(|(location, alignment, leader)| {
                            let mut stop = json!({"position": location, "alignment": alignment});
                            if let Some(leader) = leader {
                                stop["leader"] = json!(leader);
                            }
                            stop
                        })
                        .collect(),
                ),
            );
        }
        if self.unsupported_tab_stops {
            style.insert(crate::native_text_tabs::STOPS_FIELD.to_string(), json!([]));
            unsupported.push(
                "RTF justified tab stops are not represented by the supported RTF tab alignments; no tab stops were imported".to_string(),
            );
        }
        if self.default_tab_interval_disabled {
            style.remove(crate::native_text_tabs::INTERVAL_FIELD);
        }
        if let Some(list) = &self.legacy_list {
            style.extend(list.clone());
        }
        if self.list_override == Some(0) {
            style.insert(
                crate::native_text_list::STYLE_FIELD.to_string(),
                json!("none"),
            );
        } else if let Some(override_id) = self.list_override {
            let level = usize::from(self.list_level);
            let table_level = lists
                .overrides
                .get(&override_id)
                .and_then(|id| lists.lists.get(id))
                .and_then(|levels| levels.get(level));
            let mut list_style = match table_level {
                Some(level) => rtf_level_style(level),
                None => proto_levels.get(level).cloned(),
            };
            if let Some(mut list_style) = list_style.take() {
                list_style.insert("textListLevel".to_string(), json!(self.list_level));
                style.extend(list_style);
            } else {
                style.insert(
                    crate::native_text_list::STYLE_FIELD.to_string(),
                    json!("none"),
                );
                unsupported.push(format!("RTF list override {override_id} level {} has an unsupported or missing definition", self.list_level));
            }
        } else if self.paragraph_reset && self.legacy_list.is_none() {
            style.insert(
                crate::native_text_list::STYLE_FIELD.to_string(),
                json!("none"),
            );
        }
        (style, unsupported)
    }
}

fn rtf_level_style(level: &RtfListLevel) -> Option<Map<String, Value>> {
    let name = level.style.as_deref()?;
    let mut style = Map::new();
    style.insert(
        crate::native_text_list::STYLE_FIELD.to_string(),
        json!(name),
    );
    match crate::native_text_list::ListStyle::parse(name)? {
        crate::native_text_list::ListStyle::Decimal
        | crate::native_text_list::ListStyle::LowerAlpha
        | crate::native_text_list::ListStyle::UpperAlpha
        | crate::native_text_list::ListStyle::LowerRoman
        | crate::native_text_list::ListStyle::UpperRoman => {
            style.insert(
                crate::native_text_list::PREFIX_FIELD.to_string(),
                json!(level.prefix.as_deref().unwrap_or_default()),
            );
            style.insert(
                crate::native_text_list::POSTFIX_FIELD.to_string(),
                json!(level.postfix.as_deref().unwrap_or_default()),
            );
            style.insert(
                crate::native_text_list::START_FIELD.to_string(),
                json!(level.start.unwrap_or(1)),
            );
        }
        _ => {}
    }
    crate::native_text_list::invalid_list_fields(&style)
        .is_empty()
        .then_some(style)
}

#[derive(Clone, Debug, Default)]
struct RtfListLevel {
    style: Option<String>,
    prefix: Option<String>,
    postfix: Option<String>,
    start: Option<i64>,
}

#[derive(Clone, Debug, Default)]
struct RtfListTables {
    lists: BTreeMap<i32, Vec<RtfListLevel>>,
    overrides: BTreeMap<i32, i32>,
}

#[derive(Clone, Debug)]
struct RtfDocument {
    characters: Vec<RtfChar>,
    paragraphs: Vec<RtfParagraph>,
    lists: RtfListTables,
}

/// The text read so far with the formatting in force for each character. Groups restore the
/// formatting they were opened with.
struct RtfOutput {
    characters: Vec<RtfChar>,
    style: RtfStyle,
    paragraph: RtfParagraph,
    final_paragraph: RtfParagraph,
    default_tab_interval: f64,
    paragraphs: Vec<RtfParagraph>,
    saved: Vec<(RtfStyle, RtfParagraph)>,
    colors: Vec<Option<[u8; 3]>>,
    ends_with_paragraph_break: bool,
}

impl RtfOutput {
    fn push(&mut self, character: char) {
        self.characters.push((character, self.style));
        self.final_paragraph = self.paragraph.clone();
        self.ends_with_paragraph_break = false;
    }
}

/// The entries of the `\colortbl` group in order; entry 0 is the automatic color.
fn rtf_color_table(rtf: &str) -> Vec<Option<[u8; 3]>> {
    let Some(start) = rtf.find("{\\colortbl") else {
        return Vec::new();
    };
    let table = &rtf[start + "{\\colortbl".len()..];
    let table = &table[..table.find('}').unwrap_or(table.len())];
    let channel = |entry: &str, name: &str| {
        let value = entry.split_once(name)?.1;
        let digits = value
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        digits.parse::<u8>().ok()
    };
    table
        .split(';')
        .map(|entry| {
            Some([
                channel(entry, "\\red")?,
                channel(entry, "\\green")?,
                channel(entry, "\\blue")?,
            ])
        })
        .collect()
}

/// The `\fonttbl` entries by font number, for both `{\f0 Name;}` groups and flat `\f0\fnil Name;`.
fn rtf_font_table(rtf: &str) -> BTreeMap<u32, String> {
    let Some(start) = rtf.find("{\\fonttbl") else {
        return BTreeMap::new();
    };
    let mut depth = 0;
    let mut table = String::new();
    for character in rtf[start + "{\\fonttbl".len()..].chars() {
        match character {
            '{' => depth += 1,
            '}' if depth == 0 => break,
            '}' => depth -= 1,
            _ => table.push(character),
        }
    }
    table
        .split(';')
        .filter_map(|entry| {
            let (_, rest) = entry.split_once("\\f")?;
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            let number = rest[..digits].parse().ok()?;
            let rest = &rest[digits..];
            let name = match rest.rsplit_once('\\') {
                Some((_, control)) => control
                    .trim_start_matches(|c: char| c.is_ascii_alphanumeric())
                    .trim_start_matches(' '),
                None => rest.trim_start_matches(' '),
            };
            (!name.is_empty()).then(|| (number, name.to_string()))
        })
        .collect()
}

fn rtf_text(data: &[u8], keep_tabs: bool) -> Vec<RtfChar> {
    rtf_parse(data, keep_tabs).characters
}

fn rtf_layer_document(data: &[u8]) -> RtfDocument {
    rtf_parse(data, true)
}

fn rtf_group_end(value: &str, start: usize) -> Option<usize> {
    let bytes = value.as_bytes();
    if bytes.get(start) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                index += 1;
                if bytes.get(index) == Some(&b'\'') {
                    index = (index + 3).min(bytes.len());
                } else if bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
                    while bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
                        index += 1;
                    }
                    if bytes.get(index) == Some(&b'-') {
                        index += 1;
                    }
                    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                        index += 1;
                    }
                    if bytes.get(index) == Some(&b' ') {
                        index += 1;
                    }
                } else {
                    index += usize::from(index < bytes.len());
                }
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn rtf_group_children(group: &str) -> Vec<&str> {
    let bytes = group.as_bytes();
    let mut children = Vec::new();
    let mut index = 1;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\\' {
            index += 1;
            if bytes.get(index) == Some(&b'\'') {
                index = (index + 3).min(bytes.len());
            } else if bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
                while bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
                    index += 1;
                }
                while bytes
                    .get(index)
                    .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'-')
                {
                    index += 1;
                }
            } else {
                index += usize::from(index < bytes.len());
            }
            continue;
        }
        if bytes[index] == b'{' {
            if let Some(end) = rtf_group_end(group, index) {
                if let Some(child) = group.get(index..end) {
                    children.push(child);
                }
                index = end;
                continue;
            }
        }
        index += 1;
    }
    children
}

fn rtf_direct_controls(group: &str) -> Vec<(String, Option<i64>)> {
    let bytes = group.as_bytes();
    let mut controls = Vec::new();
    let mut index = usize::from(bytes.first() == Some(&b'{'));
    let end = bytes.len().saturating_sub(1);
    let mut depth = 1i32;
    while index < end {
        match bytes[index] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'\\' => {
                index += 1;
                if bytes.get(index) == Some(&b'\'') {
                    index = (index + 3).min(end);
                    continue;
                }
                if !bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
                    index += usize::from(index < end);
                    continue;
                }
                let name_start = index;
                while bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
                    index += 1;
                }
                let name = String::from_utf8_lossy(&bytes[name_start..index]).to_string();
                let value_start = index;
                if bytes.get(index) == Some(&b'-') {
                    index += 1;
                }
                while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                    index += 1;
                }
                let value = (index > value_start)
                    .then(|| {
                        String::from_utf8_lossy(&bytes[value_start..index])
                            .parse::<i64>()
                            .ok()
                    })
                    .flatten();
                if depth == 1 {
                    controls.push((name, value));
                }
                if bytes.get(index) == Some(&b' ') {
                    index += 1;
                }
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    controls
}

fn rtf_find_destination<'a>(rtf: &'a str, name: &str) -> Option<&'a str> {
    let bytes = rtf.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'{' {
            if let Some(end) = rtf_group_end(rtf, index) {
                let group = rtf.get(index..end)?;
                if rtf_direct_controls(group)
                    .iter()
                    .any(|(control, _)| control == name)
                {
                    return Some(group);
                }
            }
        }
        index += 1;
    }
    None
}

fn rtf_direct_value(group: &str, name: &str) -> Option<i64> {
    rtf_direct_controls(group)
        .into_iter()
        .find_map(|(control, value)| (control == name).then_some(value).flatten())
}

fn rtf_level_units(group: &str, name: &str) -> Option<Vec<u16>> {
    let bytes = group.as_bytes();
    let needle = format!("\\{name}");
    let start = group.find(&needle)? + needle.len();
    let mut index = start;
    if bytes.get(index) == Some(&b' ') {
        index += 1;
    }
    let mut units = Vec::new();
    while index < bytes.len().saturating_sub(1) && bytes[index] != b';' {
        match bytes[index] {
            b'\\' if bytes.get(index + 1) == Some(&b'\'') => {
                let high = (*bytes.get(index + 2)? as char).to_digit(16)?;
                let low = (*bytes.get(index + 3)? as char).to_digit(16)?;
                units.push((high * 16 + low) as u16);
                index += 4;
            }
            b'\\' => {
                index += 1;
                if bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
                    let name_start = index;
                    while bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
                        index += 1;
                    }
                    let control = &bytes[name_start..index];
                    let number_start = index;
                    if bytes.get(index) == Some(&b'-') {
                        index += 1;
                    }
                    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                        index += 1;
                    }
                    if control == b"u" {
                        let number = String::from_utf8_lossy(&bytes[number_start..index])
                            .parse::<i32>()
                            .ok()?;
                        units.push((if number < 0 { number + 65_536 } else { number }) as u16);
                        if bytes.get(index) == Some(&b' ') {
                            index += 1;
                        }
                        if bytes.get(index) == Some(&b'?') {
                            index += 1;
                        }
                    } else if bytes.get(index) == Some(&b' ') {
                        index += 1;
                    }
                } else {
                    let escaped = *bytes.get(index)?;
                    if matches!(escaped, b'\\' | b'{' | b'}' | b';') {
                        units.push(u16::from(escaped));
                    }
                    index += 1;
                }
            }
            byte if byte.is_ascii() => {
                units.push(u16::from(byte));
                index += 1;
            }
            _ => {
                let character = group.get(index..)?.chars().next()?;
                units.extend(character.encode_utf16(&mut [0; 2]).iter().copied());
                index += character.len_utf8();
            }
        }
    }
    Some(units)
}

fn parse_rtf_list_level(group: &str) -> RtfListLevel {
    let style = match rtf_direct_value(group, "levelnfc") {
        Some(0) => Some("decimal"),
        Some(1) => Some("upper-roman"),
        Some(2) => Some("lower-roman"),
        Some(3) => Some("upper-alpha"),
        Some(4) => Some("lower-alpha"),
        Some(23) => None,
        Some(255) => None,
        _ => None,
    };
    let pattern = rtf_group_children(group)
        .into_iter()
        .find(|child| {
            rtf_direct_controls(child)
                .iter()
                .any(|(name, _)| name == "leveltext")
        })
        .and_then(|child| rtf_level_units(child, "leveltext"));
    let positions = rtf_group_children(group)
        .into_iter()
        .find(|child| {
            rtf_direct_controls(child)
                .iter()
                .any(|(name, _)| name == "levelnumbers")
        })
        .and_then(|child| rtf_level_units(child, "levelnumbers"));
    let mut prefix = None;
    let mut postfix = None;
    let mut resolved_style = style.map(str::to_string);
    if let Some(pattern) = pattern {
        let (length, pattern) = pattern
            .split_first()
            .map(|(length, units)| (*length as usize, units))
            .unwrap_or((0, &[]));
        if length <= pattern.len() {
            let pattern = &pattern[..length];
            if resolved_style.is_some() {
                let placeholders = positions
                    .unwrap_or_default()
                    .into_iter()
                    .map(usize::from)
                    .collect::<Vec<_>>();
                if placeholders.len() == 1
                    && placeholders[0] > 0
                    && placeholders[0] <= pattern.len()
                {
                    let at = placeholders[0] - 1;
                    if pattern[at] == 0 {
                        let before = String::from_utf16(&pattern[..at]).ok();
                        let after = String::from_utf16(&pattern[at + 1..]).ok();
                        if let Some(before) = before.filter(|value| !value.is_empty()) {
                            prefix = Some(before);
                        }
                        if let Some(after) = after.filter(|value| !value.is_empty()) {
                            postfix = Some(after);
                        }
                    } else {
                        resolved_style = None;
                    }
                } else {
                    resolved_style = None;
                }
            } else {
                resolved_style = match String::from_utf16(pattern).ok().as_deref() {
                    Some("□") | Some("☐") => Some("box".to_string()),
                    Some("✓") => Some("check".to_string()),
                    Some("◆") => Some("diamond".to_string()),
                    Some("•") | Some("●") => Some("disc".to_string()),
                    Some("◦") => Some("circle".to_string()),
                    Some("▪") | Some("■") => Some("square".to_string()),
                    Some("-") => Some("hyphen".to_string()),
                    _ => None,
                };
            }
        }
    }
    RtfListLevel {
        style: resolved_style,
        prefix,
        postfix,
        start: rtf_direct_value(group, "levelstartat"),
    }
}

fn parse_rtf_list_tables(rtf: &str) -> RtfListTables {
    let mut tables = RtfListTables::default();
    if let Some(group) = rtf_find_destination(rtf, "listtable") {
        for list in rtf_group_children(group) {
            if !rtf_direct_controls(list)
                .iter()
                .any(|(name, _)| name == "list")
            {
                continue;
            }
            let Some(id) =
                rtf_direct_value(list, "listid").and_then(|value| i32::try_from(value).ok())
            else {
                continue;
            };
            let levels = rtf_group_children(list)
                .into_iter()
                .filter(|child| {
                    rtf_direct_controls(child)
                        .iter()
                        .any(|(name, _)| name == "listlevel")
                })
                .map(parse_rtf_list_level)
                .collect::<Vec<_>>();
            tables.lists.insert(id, levels);
        }
    }
    if let Some(group) = rtf_find_destination(rtf, "listoverridetable") {
        for item in rtf_group_children(group) {
            if !rtf_direct_controls(&item)
                .iter()
                .any(|(name, _)| name == "listoverride")
            {
                continue;
            }
            let Some(id) =
                rtf_direct_value(&item, "listid").and_then(|value| i32::try_from(value).ok())
            else {
                continue;
            };
            let Some(override_id) =
                rtf_direct_value(&item, "ls").and_then(|value| i32::try_from(value).ok())
            else {
                continue;
            };
            tables.overrides.insert(override_id, id);
        }
    }
    tables
}

fn rtf_parse(data: &[u8], keep_tabs: bool) -> RtfDocument {
    let input = String::from_utf8_lossy(data);
    let rtf = input
        .find("{\\rtf")
        .map(|index| &input[index..])
        .unwrap_or(input.as_ref());
    let characters = rtf.chars().collect::<Vec<_>>();
    let mut output = RtfOutput {
        characters: Vec::new(),
        style: RtfStyle::default(),
        paragraph: RtfParagraph::default(),
        final_paragraph: RtfParagraph::default(),
        default_tab_interval: 36.0,
        paragraphs: Vec::new(),
        saved: Vec::new(),
        colors: rtf_color_table(rtf),
        ends_with_paragraph_break: false,
    };
    let mut groups = vec![false];
    let mut index = 0;

    while index < characters.len() {
        let character = characters[index];
        match character {
            '{' => {
                let inherited_skip = *groups.last().unwrap_or(&false);
                if keep_tabs && rtf_destination_name(&characters, index).as_deref() == Some("pn") {
                    if let Some(group) = rtf_group_from_chars(&characters, index) {
                        if let Some((list, _)) = rtf_legacy_numbering(&group) {
                            output.paragraph.legacy_list = Some(list);
                        } else {
                            output.paragraph.legacy_list = Some(Map::from_iter([(
                                crate::native_text_list::STYLE_FIELD.to_string(),
                                json!("none"),
                            )]));
                            output.paragraph.unsupported.push(
                                "legacy RTF paragraph numbering has an unsupported number style"
                                    .to_string(),
                            );
                        }
                    }
                }
                groups.push(
                    inherited_skip || rtf_destination_starts_at(&characters, index, keep_tabs),
                );
                output.saved.push((output.style, output.paragraph.clone()));
            }
            '}' => {
                if groups.len() > 1 {
                    groups.pop();
                    if let Some((style, paragraph)) = output.saved.pop() {
                        output.style = style;
                        output.paragraph = paragraph;
                    }
                }
            }
            '\\' => {
                index = consume_rtf_escape(&characters, index, &mut groups, &mut output, keep_tabs);
            }
            '\n' | '\r' => {}
            value if !groups.last().copied().unwrap_or(false) => output.push(value),
            _ => {}
        }
        index += 1;
    }

    let mut characters = cleanup_rtf_text(&output.characters, keep_tabs);
    let mut paragraphs = output.paragraphs;
    if keep_tabs && output.ends_with_paragraph_break {
        if characters
            .last()
            .is_some_and(|(character, _)| *character == '\n')
        {
            characters.pop();
        }
    } else {
        paragraphs.push(output.final_paragraph);
    }
    let count = characters
        .iter()
        .filter(|(character, _)| *character == '\n')
        .count()
        + 1;
    paragraphs.truncate(count);
    while paragraphs.len() < count {
        paragraphs.push(RtfParagraph::default());
    }
    RtfDocument {
        characters,
        paragraphs,
        lists: parse_rtf_list_tables(rtf),
    }
}

fn rtf_destination_name(characters: &[char], index: usize) -> Option<String> {
    if characters.get(index) != Some(&'{') || characters.get(index + 1) != Some(&'\\') {
        return None;
    }
    let slash = if characters.get(index + 2) == Some(&'*') {
        index + 3
    } else {
        index + 1
    };
    (characters.get(slash) == Some(&'\\'))
        .then(|| rtf_control_name(characters, slash))
        .flatten()
}

/// Layer text drops the `\listtext` groups that hold the rendered list marker, because list
/// markers come from the imported list style. Notes and labels keep them as ordinary text.
fn rtf_destination_starts_at(characters: &[char], index: usize, layer_text: bool) -> bool {
    if characters.get(index + 1) != Some(&'\\') {
        return false;
    }
    if characters.get(index + 2) == Some(&'*') {
        return true;
    }
    let name = rtf_control_name(characters, index + 1);
    if layer_text && matches!(name.as_deref(), Some("listtext" | "pntext" | "pn")) {
        return true;
    }
    matches!(
        name.as_deref(),
        Some(
            "colortbl"
                | "datastore"
                | "fonttbl"
                | "generator"
                | "info"
                | "listtable"
                | "pict"
                | "revtbl"
                | "stylesheet"
                | "themedata"
        )
    )
}

fn rtf_group_from_chars(characters: &[char], start: usize) -> Option<String> {
    if characters.get(start) != Some(&'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut index = start;
    while index < characters.len() {
        match characters[index] {
            '\\' => {
                index += 1;
                if characters.get(index) == Some(&'\'') {
                    index = (index + 3).min(characters.len());
                } else if characters.get(index).is_some_and(char::is_ascii_alphabetic) {
                    while characters.get(index).is_some_and(char::is_ascii_alphabetic) {
                        index += 1;
                    }
                    if characters.get(index) == Some(&'-') {
                        index += 1;
                    }
                    while characters.get(index).is_some_and(char::is_ascii_digit) {
                        index += 1;
                    }
                    if characters.get(index) == Some(&' ') {
                        index += 1;
                    }
                } else {
                    index += usize::from(index < characters.len());
                }
                continue;
            }
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(characters[start..=index].iter().collect());
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn rtf_legacy_numbering(group: &str) -> Option<(Map<String, Value>, u8)> {
    let controls = rtf_direct_controls(group);
    let level = controls
        .iter()
        .find_map(|(name, value)| (name == "pnlvl").then_some(*value).flatten())
        .and_then(|level| u8::try_from(level.saturating_sub(1)).ok())
        .or_else(|| {
            controls
                .iter()
                .any(|(name, _)| name == "pnlvlblt")
                .then_some(0)
        })?;
    let prefix = rtf_group_children(group)
        .into_iter()
        .find(|child| {
            rtf_direct_controls(child)
                .iter()
                .any(|(name, _)| name == "pntxtb")
        })
        .map(|child| {
            rtf_text(child.as_bytes(), true)
                .into_iter()
                .map(|(character, _)| character)
                .collect::<String>()
        })
        .unwrap_or_default();
    let postfix = rtf_group_children(group)
        .into_iter()
        .find(|child| {
            rtf_direct_controls(child)
                .iter()
                .any(|(name, _)| name == "pntxta")
        })
        .map(|child| {
            rtf_text(child.as_bytes(), true)
                .into_iter()
                .map(|(character, _)| character)
                .collect::<String>()
        })
        .unwrap_or_default();
    let style = if controls.iter().any(|(name, _)| name == "pnlvlblt") {
        match prefix.trim() {
            "□" | "☐" => "box",
            "✓" => "check",
            "◆" => "diamond",
            "•" | "●" => "disc",
            "◦" => "circle",
            "▪" | "■" => "square",
            "-" => "hyphen",
            _ => return None,
        }
    } else if controls.iter().any(|(name, _)| name == "pndec") {
        "decimal"
    } else if controls.iter().any(|(name, _)| name == "pnlcltr") {
        "lower-alpha"
    } else if controls.iter().any(|(name, _)| name == "pnucltr") {
        "upper-alpha"
    } else if controls.iter().any(|(name, _)| name == "pnlcrm") {
        "lower-roman"
    } else if controls.iter().any(|(name, _)| name == "pnucrm") {
        "upper-roman"
    } else {
        return None;
    };
    let mut values = Map::new();
    values.insert(
        crate::native_text_list::STYLE_FIELD.to_string(),
        json!(style),
    );
    if matches!(
        style,
        "decimal" | "lower-alpha" | "upper-alpha" | "lower-roman" | "upper-roman"
    ) {
        values.insert(
            crate::native_text_list::PREFIX_FIELD.to_string(),
            json!(prefix),
        );
        values.insert(
            crate::native_text_list::POSTFIX_FIELD.to_string(),
            json!(postfix),
        );
        if let Some(start) = controls
            .iter()
            .find_map(|(name, value)| (name == "pnstart").then_some(*value).flatten())
        {
            values.insert(
                crate::native_text_list::START_FIELD.to_string(),
                json!(start),
            );
        }
    }
    values.insert("textListLevel".to_string(), json!(level));
    Some((values, level))
}

fn consume_rtf_escape(
    characters: &[char],
    index: usize,
    groups: &mut [bool],
    output: &mut RtfOutput,
    keep_tabs: bool,
) -> usize {
    let Some(next) = characters.get(index + 1).copied() else {
        return index;
    };
    let skipping = groups.last().copied().unwrap_or(false);
    match next {
        '\\' | '{' | '}' if !skipping => {
            output.push(next);
            index + 1
        }
        '\'' => {
            let value = characters
                .get(index + 2)
                .and_then(|first| first.to_digit(16))
                .zip(
                    characters
                        .get(index + 3)
                        .and_then(|second| second.to_digit(16)),
                )
                .and_then(|(first, second)| char::from_u32(first * 16 + second));
            if !skipping {
                if let Some(value) = value {
                    output.push(value);
                }
            }
            index + 3
        }
        '*' => {
            if let Some(group) = groups.last_mut() {
                *group = true;
            }
            index + 1
        }
        value if value.is_ascii_alphabetic() => {
            consume_rtf_control(characters, index, skipping, output, keep_tabs)
        }
        _ => index,
    }
}

fn consume_rtf_control(
    characters: &[char],
    index: usize,
    skipping: bool,
    output: &mut RtfOutput,
    keep_tabs: bool,
) -> usize {
    let Some(name) = rtf_control_name(characters, index) else {
        return index;
    };
    let mut end = index + 1 + name.len();
    let mut parameter = String::new();
    while let Some(value) = characters.get(end).copied() {
        if value == '-' || value.is_ascii_digit() {
            parameter.push(value);
            end += 1;
        } else {
            break;
        }
    }
    if characters.get(end) == Some(&' ') {
        end += 1;
    }
    if !skipping {
        match name.as_str() {
            "par" => {
                output.push('\n');
                output.paragraphs.push(output.paragraph.clone());
                output.ends_with_paragraph_break = true;
            }
            "line" => {
                output.push('\u{2028}');
            }
            "tab" => output.push(if keep_tabs { '\t' } else { ' ' }),
            "plain" => output.style = RtfStyle::default(),
            "pard" => {
                output.paragraph = RtfParagraph::default();
                output.paragraph.paragraph_reset = true;
                output.paragraph.values.insert(
                    crate::native_text_tabs::INTERVAL_FIELD.to_string(),
                    json!(output.default_tab_interval),
                );
            }
            "ql" | "qr" | "qc" | "qj" | "qnatural" => {
                output.paragraph.values.insert(
                    "textHorizontalAlign".to_string(),
                    json!(match name.as_str() {
                        "ql" => "left",
                        "qr" => "right",
                        "qj" => "justify",
                        "qnatural" => "natural",
                        _ => "center",
                    }),
                );
            }
            "sb" | "sa" | "fi" | "li" | "ri" => {
                if let Ok(value) = parameter.parse::<i64>() {
                    let key = match name.as_str() {
                        "sb" => "textParagraphSpacingBefore",
                        "sa" => "textParagraphSpacingAfter",
                        "fi" => "textFirstLineIndent",
                        "li" => "textLeftIndent",
                        _ => "textRightIndent",
                    };
                    let points = value as f64 / 20.0;
                    output
                        .paragraph
                        .values
                        .insert(key.to_string(), json!(points));
                }
            }
            "deftab" => {
                if let Ok(value) = parameter.parse::<i64>() {
                    output.default_tab_interval = value as f64 / 20.0;
                    output.paragraph.values.insert(
                        crate::native_text_tabs::INTERVAL_FIELD.to_string(),
                        json!(output.default_tab_interval),
                    );
                    output.paragraph.default_tab_interval_disabled = false;
                }
            }
            "pardeftab" => {
                if let Ok(value) = parameter.parse::<i64>() {
                    if value == 0 {
                        output
                            .paragraph
                            .values
                            .remove(crate::native_text_tabs::INTERVAL_FIELD);
                        output.paragraph.default_tab_interval_disabled = true;
                    } else if value > 0 {
                        output.paragraph.values.insert(
                            crate::native_text_tabs::INTERVAL_FIELD.to_string(),
                            json!(value as f64 / 20.0),
                        );
                        output.paragraph.default_tab_interval_disabled = false;
                    } else {
                        output
                            .paragraph
                            .unsupported
                            .push("Apple RTF default tab interval cannot be negative".to_string());
                        output.paragraph.default_tab_interval_disabled = true;
                    }
                }
            }
            "tql" | "tqr" | "tqc" | "tqdec" => {
                output.paragraph.unsupported_tab_alignment_pending = false;
                output.paragraph.tab_alignment = Some(match name.as_str() {
                    "tqr" => "right",
                    "tqc" => "center",
                    "tqdec" => "decimal",
                    _ => "left",
                });
            }
            "tqj" => {
                output.paragraph.tab_alignment = None;
                output.paragraph.unsupported_tab_alignment_pending = true;
            }
            "tx" => {
                if let Ok(value) = parameter.parse::<i64>() {
                    if output.paragraph.unsupported_tab_alignment_pending {
                        output.paragraph.unsupported_tab_stops = true;
                        output.paragraph.unsupported_tab_alignment_pending = false;
                    } else if output.paragraph.unsupported_tab_stops {
                        output.paragraph.unsupported_tab_stops = true;
                    } else {
                        let alignment = output.paragraph.tab_alignment.take().unwrap_or("left");
                        let leader = output.paragraph.tab_leader.take();
                        output.paragraph.absolute_tabs.push((
                            value as f64 / 20.0,
                            alignment,
                            leader,
                        ));
                    }
                }
            }
            "sl" => {
                output.paragraph.line_spacing = parameter.parse::<i64>().ok();
                apply_rtf_line_spacing(&mut output.paragraph);
            }
            "slmult" => {
                output.paragraph.line_spacing_multiple = parameter == "1";
                apply_rtf_line_spacing(&mut output.paragraph);
            }
            "slleading" => {
                output.paragraph.apple_line_spacing = parameter.parse::<i64>().ok();
                apply_rtf_line_spacing(&mut output.paragraph);
            }
            "slminimum" => {
                output.paragraph.apple_minimum_line_height = parameter.parse::<i64>().ok();
                apply_rtf_line_spacing(&mut output.paragraph);
            }
            "slmaximum" => {
                output.paragraph.apple_maximum_line_height = parameter.parse::<i64>().ok();
                apply_rtf_line_spacing(&mut output.paragraph);
            }
            "ls" => {
                output.paragraph.list_override = parameter.parse::<i32>().ok();
            }
            "ilvl" => {
                if let Ok(level) = parameter.parse::<u8>() {
                    output.paragraph.list_level = level;
                }
            }
            "tb" => {
                if let Ok(value) = parameter.parse::<i64>() {
                    output.paragraph.bar_tabs.push(value as f64 / 20.0);
                } else {
                    output
                        .paragraph
                        .unsupported
                        .push("RTF bar tab position is invalid".into());
                }
            }
            "tldot" | "tleq" | "tlhyph" | "tlth" | "tlul" => {
                output.paragraph.tab_leader = Some(match name.as_str() {
                    "tldot" => "dot",
                    "tleq" => "equal",
                    "tlhyph" => "hyphen",
                    "tlth" => "thick",
                    _ => "underline",
                });
            }
            "b" => output.style.bold = parameter != "0",
            "i" => output.style.italic = parameter != "0",
            "ul" | "ulw" | "ulth" | "uldb" | "uld" | "uldash" | "uldashd" | "uldashdd" => {
                output.style.underline = parameter != "0";
                output.style.underline_style = match name.as_str() {
                    "ulth" => 2,
                    "uldb" => 3,
                    _ => 1,
                };
                output.style.underline_pattern = match name.as_str() {
                    "uld" => 1,
                    "uldash" => 2,
                    "uldashd" => 3,
                    "uldashdd" => 4,
                    _ => 0,
                };
                output.style.underline_by_word = name == "ulw";
            }
            "ulnone" => {
                output.style.underline = false;
                output.style.underline_style = 0;
                output.style.underline_pattern = 0;
                output.style.underline_by_word = false;
            }
            "ulc" => {
                output.style.underline_color = parameter
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| output.colors.get(index).copied().flatten())
            }
            "super" => output.style.superscript = Some(1),
            "sub" => output.style.superscript = Some(-1),
            "nosupersub" => output.style.superscript = Some(0),
            "expndtw" => {
                output.style.letter_spacing =
                    parameter.parse::<f32>().ok().map(|value| value / 20.0)
            }
            "strike" | "striked" => {
                output.style.strikethrough = parameter != "0";
                output.style.strikethrough_style = if name == "striked" { 3 } else { 1 };
            }
            "f" => output.style.font = parameter.parse().ok(),
            "fs" => output.style.size = parameter.parse().ok().filter(|size| *size > 0),
            "cf" => {
                output.style.color = parameter
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| output.colors.get(index).copied().flatten());
            }
            "u" => {
                if let Ok(value) = parameter.parse::<i32>() {
                    let codepoint = if value < 0 { value + 65_536 } else { value };
                    if let Some((value, next_end)) =
                        rtf_surrogate_pair(characters, end, codepoint as u32)
                    {
                        output.push(value);
                        end = next_end;
                    } else if let Some(value) = char::from_u32(codepoint as u32) {
                        output.push(value);
                    }
                }
                if characters.get(end) == Some(&'?') {
                    end += 1;
                }
            }
            _ => {}
        }
    }
    end.saturating_sub(1)
}

fn apply_rtf_line_spacing(paragraph: &mut RtfParagraph) {
    for key in [
        "textLineHeightMultiple",
        "textLineSpacing",
        "textMinimumLineHeight",
        "textMaximumLineHeight",
    ] {
        paragraph.values.remove(key);
    }
    if let Some(value) = paragraph.line_spacing {
        if paragraph.line_spacing_multiple {
            paragraph.values.insert(
                "textLineHeightMultiple".to_string(),
                json!(value as f64 / 240.0),
            );
        } else if value < 0 {
            let height = value.unsigned_abs() as f64 / 20.0;
            paragraph
                .values
                .insert("textMinimumLineHeight".to_string(), json!(height));
            paragraph
                .values
                .insert("textMaximumLineHeight".to_string(), json!(height));
        } else if value > 0 {
            paragraph.values.insert(
                "textMinimumLineHeight".to_string(),
                json!(value as f64 / 20.0),
            );
        } else {
            paragraph
                .values
                .insert("textLineHeightMultiple".to_string(), json!(1.0));
            paragraph
                .values
                .insert("textMinimumLineHeight".to_string(), json!(0.0));
            paragraph
                .values
                .insert("textMaximumLineHeight".to_string(), json!(0.0));
        }
    }
    if let Some(value) = paragraph.apple_line_spacing {
        paragraph
            .values
            .insert("textLineSpacing".to_string(), json!(value as f64 / 20.0));
    }
    if let Some(value) = paragraph.apple_minimum_line_height {
        paragraph.values.insert(
            "textMinimumLineHeight".to_string(),
            json!(value as f64 / 20.0),
        );
    }
    if let Some(value) = paragraph.apple_maximum_line_height {
        paragraph.values.insert(
            "textMaximumLineHeight".to_string(),
            json!(value as f64 / 20.0),
        );
    }
}

fn rtf_control_name(characters: &[char], slash_index: usize) -> Option<String> {
    let mut name = String::new();
    for value in characters.iter().skip(slash_index + 1).copied() {
        if !value.is_ascii_alphabetic() {
            break;
        }
        name.push(value);
    }
    (!name.is_empty()).then_some(name)
}

fn rtf_surrogate_pair(characters: &[char], end: usize, high: u32) -> Option<(char, usize)> {
    if !(0xd800..=0xdbff).contains(&high) {
        return None;
    }
    let start = end + usize::from(characters.get(end) == Some(&'?'));
    if characters.get(start) != Some(&'\\') || characters.get(start + 1) != Some(&'u') {
        return None;
    }
    let mut next_end = start + 2;
    while characters
        .get(next_end)
        .is_some_and(|c| c.is_ascii_digit() || *c == '-')
    {
        next_end += 1;
    }
    let parameter = characters[start + 2..next_end]
        .iter()
        .collect::<String>()
        .parse::<i32>()
        .ok()?;
    let low = if parameter < 0 {
        parameter + 65_536
    } else {
        parameter
    } as u32;
    if !(0xdc00..=0xdfff).contains(&low) {
        return None;
    }
    if characters.get(next_end) == Some(&' ') {
        next_end += 1;
    }
    Some((
        char::from_u32(0x10000 + ((high - 0xd800) << 10) + low - 0xdc00)?,
        next_end,
    ))
}

fn cleanup_rtf_text(value: &[RtfChar], keep_tabs: bool) -> Vec<RtfChar> {
    if keep_tabs {
        return value
            .iter()
            .filter(|(character, _)| !character.is_control() || matches!(character, '\n' | '\t'))
            .copied()
            .collect();
    }
    let strip = |c: char| c.is_whitespace() && !(keep_tabs && c == '\t');
    let mut cleaned: Vec<RtfChar> = Vec::new();
    for line in value
        .iter()
        .filter(|(character, _)| !character.is_control() || matches!(character, '\n' | '\t'))
        .copied()
        .collect::<Vec<_>>()
        .split(|(character, _)| *character == '\n')
    {
        let start = line
            .iter()
            .position(|(c, _)| !strip(*c))
            .unwrap_or(line.len());
        let end = line
            .iter()
            .rposition(|(c, _)| !strip(*c))
            .map_or(start, |i| i + 1);
        let line = &line[start..end];
        if line.is_empty()
            || rtf_formatting_artifact(&line.iter().map(|(c, _)| *c).collect::<String>())
        {
            continue;
        }
        if let Some(&(_, style)) = cleaned.last() {
            cleaned.push(('\n', style));
        }
        cleaned.extend_from_slice(line);
    }
    cleaned
}

fn rtf_formatting_artifact(line: &str) -> bool {
    line.chars().all(|character| {
        matches!(
            character,
            '*' | ';'
                | ':'
                | '.'
                | ','
                | '-'
                | '_'
                | '\''
                | '"'
                | '`'
                | '~'
                | '^'
                | '|'
                | '\\'
                | '/'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | ' '
        )
    })
}

fn color(fields: &[Field]) -> Option<String> {
    let red = f32_field(fields, 1).unwrap_or(0.0).clamp(0.0, 1.0);
    let green = f32_field(fields, 2).unwrap_or(0.0).clamp(0.0, 1.0);
    let blue = f32_field(fields, 3).unwrap_or(0.0).clamp(0.0, 1.0);
    let alpha = f32_field(fields, 4).unwrap_or(1.0).clamp(0.0, 1.0);
    if alpha == 0.0 {
        return Some("transparent".to_string());
    }
    let channel = |value: f32| format!("{:02x}", (value * 255.0).round() as u8);
    let base = format!("#{}{}{}", channel(red), channel(green), channel(blue));
    if alpha >= 1.0 {
        Some(base)
    } else {
        Some(format!("{}{}", base, channel(alpha)))
    }
}

fn f32_field(fields: &[Field], number: u32) -> Option<f32> {
    fields.iter().find_map(|field| {
        if field.number != number {
            return None;
        }
        match field.value {
            WireValue::Fixed32(value) => Some(f32::from_bits(value)),
            _ => None,
        }
    })
}

fn decode_url_value(fields: &[Field], number: u32) -> Option<String> {
    nested_field(fields, number).and_then(|value| {
        string_field(&value, 1)
            .or_else(|| string_field(&value, 2))
            .or_else(|| nested_field(&value, 4).and_then(|local| string_field(&local, 2)))
            .or_else(|| nested_field(&value, 5).and_then(|external| string_field(&external, 3)))
    })
}

fn presentation_slide(action: &[Field]) -> Option<Value> {
    // Action.SlideType field 2 is PresentationSlide, whose field 1 is Slide.
    let slide_action = nested_field(action, 23)?;
    let presentation_slide = nested_field(&slide_action, 2)?;
    let base_slide = nested_field(&presentation_slide, 1)?;
    Some(decode_base_slide_with_runs(&base_slide, None))
}

/// A slide without character formatting runs, for the themes, props, masks and stage layouts that
/// do not edit per-run formatting.
fn decode_base_slide(base_slide: &[Field], name: Option<String>) -> Value {
    let mut slide = decode_base_slide_with_runs(base_slide, name);
    if let Some(object) = slide.as_object_mut() {
        object.remove("contentRuns");
    }
    slide
}

fn decode_base_slide_with_runs(base_slide: &[Field], name: Option<String>) -> Value {
    let width = nested_field(&base_slide, 6).and_then(|size| f64_field(&size, 1));
    let height = nested_field(&base_slide, 6).and_then(|size| f64_field(&size, 2));
    let background_enabled = bool_field(&base_slide, 4);
    let background = nested_field(&base_slide, 5).and_then(|value| color(&value));
    let mut primary_text = None;
    let mut primary_template = false;
    let mut primary_runs = None;
    let mut layers = Vec::new();
    let (build_steps, mut unreadable) = layer_build_export::decode_steps(base_slide);
    for (index, element) in bytes_fields(&base_slide, 1).enumerate() {
        let Ok(element) = decode_message(element) else {
            unreadable.push(format!("slide object {} could not be read", index + 1));
            continue;
        };
        let Some(graphics) = nested_field(&element, 1) else {
            unreadable.push(format!(
                "slide object {} has no readable element",
                index + 1
            ));
            continue;
        };
        let info = varint_field(&element, 4).unwrap_or(0);
        let is_text = info & 2 != 0;
        let mut layer = decode_graphics_element(&graphics, index, is_text);
        // Data links and build metadata belong to the Slide.Element wrapper,
        // not Graphics.Element.  Keep the portable values that the renderer
        // can execute instead of silently discarding them.
        let mut unsupported = layer_build_export::decode_builds(&element, &build_steps, &mut layer);
        for link in nested_fields(&element, 6) {
            if !layer_link_export::decode_data_link(&link, &mut layer) {
                unsupported.extend(link.iter().map(|field| {
                    format!(
                        "data link {} has no Orbit equivalent",
                        data_link_name(field.number)
                    )
                }));
            }
        }
        note_unsupported(&mut layer, unsupported);
        if let Some(scroller) = nested_field(&element, 9) {
            layer_text_settings_export::decode_text_scroller(&scroller, &mut layer, width);
        }
        let template_text = info & 1 != 0;
        if (primary_text.is_none() || (template_text && !primary_template))
            && layer.get("role").and_then(Value::as_str) == Some("slide-text")
        {
            primary_text = Some((layer["id"].clone(), layer["text"].clone()));
            primary_runs = layer.get("textRuns").cloned();
            primary_template = template_text;
        }
        layers.push(Value::Object(layer));
    }
    let mut slide = Map::new();
    note_unsupported(&mut slide, unreadable);
    if let Some(name) = name {
        slide.insert("name".to_string(), json!(name));
    }
    let content = primary_text
        .as_ref()
        .map(|(_, text)| text.clone())
        .unwrap_or_else(|| json!(""));
    if let Some((id, _)) = primary_text {
        for layer in &mut layers {
            if layer["id"] == id {
                layer.as_object_mut().unwrap().remove("textRuns");
            }
        }
        slide.insert("primaryTextLayerId".to_string(), id);
    }
    if let Some(id) = uuid(base_slide, 7) {
        slide.insert("sourceSlideId".to_string(), json!(id));
    }
    slide.insert("content".to_string(), content.clone());
    if let Some(runs) = primary_runs {
        slide.insert("contentRuns".to_string(), runs);
    }
    // Theme and mask consumers use `text` as their canonical slide preview
    // field, while presentations still use `content`. Keep both aliases so
    // imported documents work across the renderer surfaces.
    slide.insert("text".to_string(), json!(content));
    if let Some(value) = background_enabled {
        slide.insert("backgroundEnabled".to_string(), json!(value));
    }
    if let Some(value) = background {
        slide.insert("background".to_string(), json!(value));
    }
    if let Some(value) = width {
        slide.insert("width".to_string(), json!(value));
    }
    if let Some(value) = height {
        slide.insert("height".to_string(), json!(value));
    }
    if !layers.is_empty() {
        // ProPresenter stores the frontmost element first; Orbit paints back to front.
        slide.insert(
            "themeLayerOrder".into(),
            json!(layers
                .iter()
                .rev()
                .map(|layer| &layer["id"])
                .collect::<Vec<_>>()),
        );
        slide.insert("layers".to_string(), Value::Array(layers));
    }
    Value::Object(slide)
}

/// `Slide.Element.DataLink` property names by field number, for import reports.
fn data_link_name(number: u32) -> String {
    const NAMES: [(u32, &str); 37] = [
        (1, "ticker"),
        (2, "alternate_text"),
        (3, "timer_text"),
        (4, "clock_text"),
        (5, "chord_chart"),
        (6, "output_screen"),
        (7, "pco_live"),
        (8, "alternate_fill"),
        (9, "visibility_link"),
        (10, "slide_text"),
        (11, "stage_message"),
        (12, "video_countdown"),
        (13, "slide_image"),
        (14, "ccli_text"),
        (15, "group_name"),
        (16, "group_color"),
        (17, "presentation_notes"),
        (18, "playlist_item"),
        (19, "auto_advance_time_remaining"),
        (20, "capture_status_text"),
        (21, "capture_status_color"),
        (22, "slide_count"),
        (23, "audio_countdown"),
        (24, "presentation"),
        (25, "slide_label_text"),
        (26, "slide_label_color"),
        (27, "rss_feed"),
        (28, "file_feed"),
        (29, "chord_pro_chart"),
        (30, "playback_marker_text"),
        (32, "playback_marker_color"),
        (33, "timecode_text"),
        (34, "timecode_status"),
        (35, "message_text"),
        (36, "key_value_text"),
        (37, "key_value_fill"),
        (38, "zone"),
    ];
    NAMES
        .iter()
        .find(|(candidate, _)| *candidate == number)
        .map_or_else(|| format!("field {number}"), |(_, name)| name.to_string())
}

fn slide_build_start(value: u64) -> &'static str {
    match value {
        1 => "with-previous",
        2 => "after-previous",
        3 => "with-slide",
        _ => "on-click",
    }
}

fn decode_graphics_element(fields: &[Field], index: usize, is_text: bool) -> Map<String, Value> {
    let text = nested_field(fields, 13).filter(|_| is_text);
    let fill = nested_field(fields, 9);
    let path = nested_field(fields, 8);
    let media = fill.as_ref().and_then(|value| nested_field(value, 3));
    let media_type = media.as_ref().map(|fields| media_type(fields));
    let element_type = if text.is_some() {
        "Text"
    } else if media_type.is_some() {
        match media_type.as_deref() {
            Some("video") => "Video",
            Some("audio") => "Audio",
            Some("live-video") => "Live Video",
            Some("web") => "Shape",
            _ => "Media",
        }
    } else if path.is_some() {
        "Shape"
    } else {
        "Layer"
    };
    let id = uuid(fields, 1).unwrap_or_else(|| format!("layer-{}", index + 1));
    let name = string_field(fields, 2).unwrap_or_else(|| format!("{} {}", element_type, index + 1));
    let mut unsupported = Vec::new();
    let mut layer = Map::new();
    layer.insert("id".to_string(), json!(id));
    layer.insert("name".to_string(), json!(name));
    layer.insert("type".to_string(), json!(element_type));
    layer.insert(
        "role".to_string(),
        json!(if element_type == "Text" {
            "slide-text"
        } else {
            "custom"
        }),
    );
    if let Some(value) = bool_field(fields, 16) {
        layer.insert("hidden".to_string(), json!(value));
    }
    if let Some(value) = bool_field(fields, 6) {
        layer.insert("locked".to_string(), json!(value));
    }
    if let Some(value) = f64_field(fields, 4) {
        layer.insert("rotation".to_string(), json!(value));
    }
    if let Some(mode @ 1..=3) = varint_field(fields, 15) {
        layer.insert("flipX".to_string(), json!(mode & 2 != 0));
        layer.insert("flipY".to_string(), json!(mode & 1 != 0));
    }
    if let Some(value) = f64_field(fields, 5) {
        layer.insert("opacity".to_string(), json!(opacity_percent(value)));
    }
    if let Some(bounds) = nested_field(fields, 3) {
        let mut value = Map::new();
        if let Some(origin) = nested_field(&bounds, 1) {
            value.insert("x".to_string(), json!(f64_field(&origin, 1).unwrap_or(0.0)));
            value.insert("y".to_string(), json!(f64_field(&origin, 2).unwrap_or(0.0)));
        }
        if let Some(size) = nested_field(&bounds, 2) {
            if let Some(width) = f64_field(&size, 1) {
                value.insert("width".to_string(), json!(width));
            }
            if let Some(height) = f64_field(&size, 2) {
                value.insert("height".to_string(), json!(height));
            }
        }
        if !value.is_empty() {
            layer.insert("bounds".to_string(), Value::Object(value));
        }
    }
    if let Some(value) = path {
        unsupported.extend(layer_export::decode_path(&value, &mut layer));
    }
    if let Some(fill) = fill {
        decode_fill(&fill, &mut layer);
    }
    if let Some(stroke) = nested_field(fields, 10) {
        layer.insert(
            "strokeEnabled".to_string(),
            json!(bool_field(&stroke, 5).unwrap_or(false)),
        );
        if let Some(width) = f64_field(&stroke, 2) {
            layer.insert("strokeWidth".to_string(), json!(width));
        }
        if let Some(value) = nested_field(&stroke, 3).and_then(|value| color(&value)) {
            layer.insert("stroke".to_string(), json!(value.clone()));
            layer.insert("strokeColor".to_string(), json!(value));
        }
        if let Some(style) = varint_field(&stroke, 1) {
            layer.insert("strokeStyle".to_string(), json!(stroke_style(style)));
        }
    }
    if let Some(shadow) = nested_field(fields, 11) {
        if let Some(value) = bool_field(&shadow, 7) {
            layer.insert("shadowEnabled".to_string(), json!(value));
        }
        for (field, key) in [
            (2, "shadowAngle"),
            (3, "shadowOffset"),
            (4, "shadowBlur"),
            (6, "shadowOpacity"),
        ] {
            if let Some(value) = f64_field(&shadow, field) {
                layer.insert(
                    key.to_string(),
                    json!(if key == "shadowOpacity" {
                        opacity_percent(value)
                    } else {
                        value
                    }),
                );
            }
        }
        if let Some(value) = nested_field(&shadow, 5).and_then(|value| color(&value)) {
            layer.insert("shadowColor".to_string(), json!(value));
        }
    }
    if let Some(text) = text {
        let rtf = bytes_field(&text, 5);
        let rtf_document = rtf.map(rtf_layer_document);
        let text_value = rtf_document
            .as_ref()
            .map(|document| {
                document
                    .characters
                    .iter()
                    .map(|(character, _)| *character)
                    .collect()
            })
            .or_else(|| string_field(&text, 1))
            .unwrap_or_default();
        layer.insert("text".to_string(), json!(text_value));
        if let Some(runs) = rtf.and_then(rtf_to_layer_runs) {
            layer.insert("textRuns".to_string(), json!(runs));
        }
        if let Some(document) = rtf_document.as_ref() {
            let mut style = decode_text_style(&text).unwrap_or_default();
            if let Some(color) = document
                .characters
                .first()
                .and_then(|(_, first)| first.color)
                .filter(|color| {
                    document
                        .characters
                        .iter()
                        .all(|(_, style)| style.color == Some(*color))
                })
            {
                style.insert(
                    "textColor".into(),
                    json!(format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])),
                );
            }
            let proto_levels = decode_text_list_levels_from_text(&text);
            apply_rtf_paragraph_styles(&mut style, document, &proto_levels, &text_value);
            if let Some(unsupported) = style.remove("unsupportedParagraphFields") {
                layer.insert("unsupportedParagraphFields".to_string(), unsupported);
            }
            if !style.is_empty() {
                layer.insert("textStyle".to_string(), Value::Object(style));
            }
        } else if let Some(mut style) = decode_text_style(&text) {
            if let Some(unsupported) = style.remove("unsupportedParagraphFields") {
                layer.insert("unsupportedParagraphFields".to_string(), unsupported);
            }
            layer.insert("textStyle".to_string(), Value::Object(style));
        }
        if text
            .iter()
            .any(|field| field.number == 5 && !matches!(field.value, WireValue::Bytes(_)))
        {
            unsupported.push("text content with an unexpected wire type".to_string());
        }
        let attributes = nested_field(&text, 3).unwrap_or_default();
        for (field, name) in [(4, "underline"), (9, "strikethrough")] {
            if let Some(line) = nested_field(&attributes, field) {
                if varint_field(&line, 1).is_some_and(|value| value > 3)
                    || varint_field(&line, 2).is_some_and(|value| value > 4)
                    || line.iter().any(|value| !matches!(value.number, 1..=3))
                {
                    unsupported.push(format!("{name} contains an unsupported style or pattern"));
                }
            }
        }
        if varint_field(&attributes, 8).is_some_and(|value| !matches!(value as u32 as i32, -1..=1))
        {
            unsupported.push("superscript levels beyond one are not rendered".into());
        }
        if attributes.iter().any(|field| field.number == 2)
            && !varint_field(&attributes, 2).is_some_and(|value| matches!(value, 0..=4))
        {
            unsupported.push("text capitalization".to_string());
        }
        let mut style = layer
            .remove("textStyle")
            .and_then(|style| style.as_object().cloned())
            .unwrap_or_default();
        layer_text_paint_export::decode_text_fill_style(&attributes, &mut style, &mut unsupported);
        if !style.is_empty() {
            layer.insert("textStyle".into(), Value::Object(style));
        }
        if attributes.iter().any(|field| field.number == 18)
            && nested_field(&attributes, 18)
                .and_then(|effect| decode_background_effect(&effect))
                .is_none()
        {
            unsupported.push("text background effect".to_string());
        }
        if text.iter().any(|field| field.number == 10)
            && !matches!(varint_field(&text, 10), Some(0..=4))
        {
            unsupported.push("text line transform".to_string());
        }
        if text.iter().any(|field| field.number == 11)
            && layer_link_export::string_field_including_empty(&text, 11).is_none()
        {
            unsupported.push("text transform delimiter".to_string());
        }
        unsupported.extend(present_fields(&text, &[(13, "alternate texts")]));
        if text.iter().any(|field| field.number == 7)
            && !varint_field(&text, 7).is_some_and(|value| value <= 4)
        {
            unsupported.push("text scaling".to_string());
        }
        if text.iter().any(|field| field.number == 8) {
            match nested_field(&text, 8) {
                Some(margins)
                    if margins.iter().all(|field| {
                        (1..=4).contains(&field.number)
                            && f64_field(&margins, field.number)
                                .is_some_and(|value| value.is_finite() && value >= 0.0)
                    }) => {}
                _ => unsupported.push("text margins have invalid values or fields".to_string()),
            }
        }
        if nested_field(&text, 12).is_some_and(|chords| bool_field(&chords, 1) == Some(true)) {
            unsupported.push("chord notation".to_string());
        }
    }
    unsupported.extend(layer_export::decode_feathering(fields, &mut layer));
    if let Some(mask) = nested_field(fields, 14).filter(|mask| !mask.is_empty()) {
        let mask_style = match varint_field(&mask, 4).unwrap_or(0) {
            0 => Some("Full Width"),
            1 => Some("Line Width"),
            2 => Some("Max Line Width"),
            _ => None,
        };
        let valid = mask.iter().all(|field| (1..=6).contains(&field.number))
            && (!mask.iter().any(|field| field.number == 1) || bool_field(&mask, 1).is_some())
            && [2, 3, 5, 6].into_iter().all(|field| {
                !mask.iter().any(|item| item.number == field)
                    || f64_field(&mask, field).is_some_and(f64::is_finite)
            });
        if let Some(mask_style) = mask_style.filter(|_| valid) {
            let style = layer
                .entry("textStyle".to_string())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .unwrap();
            style.insert(
                "linesOnlyEnabled".into(),
                json!(bool_field(&mask, 1).unwrap_or(false)),
            );
            style.insert("linesOnlyStyle".into(), json!(mask_style));
            for (field, key) in [
                (2, "linesOnlyHeightOffset"),
                (3, "linesOnlyVerticalOffset"),
                (5, "linesOnlyWidthOffset"),
                (6, "linesOnlyHorizontalOffset"),
            ] {
                style.insert(key.into(), json!(f64_field(&mask, field).unwrap_or(0.0)));
            }
        } else {
            unsupported.push("text line mask".to_string());
        }
    }
    if let Some(media) = media.as_deref() {
        unsupported.extend(live_video_source_losses(media));
    }
    note_unsupported(&mut layer, unsupported);
    if let Some(media) = media {
        decode_media_style(&media, &mut layer, "media");
        if layer.get("mediaType").and_then(Value::as_str) == Some("web") {
            decode_web_fill(&media, &mut layer);
        }
    }
    layer
}

/// ProPresenter web fills are a `Media` with web content; Orbit renders them as live pages.
fn decode_web_fill(media: &[Field], layer: &mut Map<String, Value>) {
    let url = decode_url_value(media, 2)
        .or_else(|| nested_field(media, 8).and_then(|web| decode_url_value(&web, 2)));
    for key in ["mediaType", "mediaUrl", "mediaSourcePath"] {
        layer.remove(key);
    }
    if layer.get("fillType").and_then(Value::as_str) == Some("media") {
        layer.insert("fillType".into(), json!("web"));
        layer.insert("fill".into(), json!("Web"));
    }
    match url {
        Some(url) if crate::web_capture::page_url(&url).is_ok() => {
            layer.insert("webUrl".into(), json!(url.trim()));
        }
        Some(url) => {
            layer.insert("unsupportedWebUrl".into(), json!(url));
        }
        None => {}
    }
}

fn opacity_percent(value: f64) -> f64 {
    if (0.0..=1.0).contains(&value) {
        value * 100.0
    } else {
        value
    }
}

fn shape_name(value: Option<u64>) -> Option<&'static str> {
    match value {
        Some(1) => Some("rectangle"),
        Some(2) => Some("ellipse"),
        Some(3) => Some("triangle"),
        Some(4) => Some("right-triangle"),
        Some(5) => Some("rhombus"),
        Some(6) => Some("star"),
        Some(7) => Some("polygon"),
        Some(9) => Some("arrow"),
        Some(10) => Some("double-arrow"),
        Some(11) => Some("rounded-rect"),
        Some(8) => Some("custom"),
        _ => None,
    }
}

fn stroke_style(value: u64) -> &'static str {
    match value {
        1 => "square",
        2 => "short",
        3 => "long",
        _ => "solid",
    }
}

/// Maps ProPresenter's normalized blur amount and saturation offset to Orbit's
/// blur radius in slide pixels and saturation percentage.
fn decode_background_effect(effect: &[Field]) -> Option<(&'static str, Map<String, Value>)> {
    let mut values = Map::new();
    if let Some(blur) = nested_field(effect, 2) {
        let amount = f64_field(&blur, 2).unwrap_or(0.0).clamp(0.0, 1.0) * 40.0;
        let saturation = (1.0 + f64_field(&blur, 1).unwrap_or(0.0)).clamp(0.0, 2.0) * 100.0;
        values.insert("backgroundBlurAmount".into(), json!(amount));
        values.insert("backgroundBlurSaturation".into(), json!(saturation));
        return Some(("blur", values));
    }
    effect
        .iter()
        .any(|field| field.number == 3)
        .then_some(("invert", values))
}

fn gradient_fill_losses(gradient: &[Field]) -> Vec<Value> {
    let mut losses = Vec::new();
    let has = |number| gradient.iter().any(|field| field.number == number);
    for field in gradient
        .iter()
        .filter(|field| !matches!(field.number, 1..=4))
    {
        losses.push(json!(format!(
            "gradient field {} is not represented by Orbit",
            field.number
        )));
    }
    if has(2) && f64_field(gradient, 2).is_none_or(|angle| !angle.is_finite()) {
        losses.push(json!("gradient angle is invalid"));
    }

    let stops = nested_fields(gradient, 4).collect::<Vec<_>>();
    if bytes_fields(gradient, 4).count() != stops.len() {
        losses.push(json!("gradient color stop payload is malformed"));
    }
    for stop in &stops {
        for field in stop.iter().filter(|field| !matches!(field.number, 1..=3)) {
            losses.push(json!(format!(
                "gradient color stop field {} is not represented by Orbit shape fills",
                field.number
            )));
        }
        if nested_field(stop, 1)
            .and_then(|fields| color(&fields))
            .is_none()
        {
            losses.push(json!("gradient color stop color is invalid"));
        }
    }
    losses
}

fn decode_fill(fields: &[Field], layer: &mut Map<String, Value>) {
    let effect = nested_field(fields, 5).and_then(|effect| decode_background_effect(&effect));
    let mut unsupported = fields
        .iter()
        .filter(|field| !matches!(field.number, 1..=4) && (field.number != 5 || effect.is_none()))
        .map(|field| json!(field.number))
        .collect::<Vec<_>>();
    if [1, 2, 3, 5]
        .into_iter()
        .filter(|number| fields.iter().any(|field| field.number == *number))
        .count()
        > 1
    {
        unsupported.push(json!("multiple fill type values"));
    }
    if fields.iter().any(|field| field.number == 1)
        && nested_field(fields, 1)
            .and_then(|fields| color(&fields))
            .is_none()
    {
        unsupported.push(json!("solid fill color is malformed"));
    }
    if fields.iter().any(|field| field.number == 3) && nested_field(fields, 3).is_none() {
        unsupported.push(json!("media fill payload is malformed"));
    }
    if fields.iter().any(|field| field.number == 4) && bool_field(fields, 4).is_none() {
        unsupported.push(json!("fill enabled flag is malformed"));
    }
    if fields.iter().any(|field| field.number == 2) {
        if let Some(gradient) = nested_field(fields, 2) {
            unsupported.extend(gradient_fill_losses(&gradient));
        } else {
            unsupported.push(json!("gradient payload is malformed"));
        }
    }
    if !unsupported.is_empty() {
        layer.insert("unsupportedFillFields".to_string(), json!(unsupported));
    }
    let enabled = bool_field(fields, 4).unwrap_or(false);
    layer.insert("fillEnabled".to_string(), json!(enabled));
    if let Some(value) = nested_field(fields, 1).and_then(|value| color(&value)) {
        layer.insert("fill".to_string(), json!(value));
        layer.insert("fillType".to_string(), json!("color"));
        layer.insert("fillColors".to_string(), json!([value]));
    } else if let Some(gradient) = nested_field(fields, 2) {
        // ProPresenter blends only the first two stops and ignores type, length, positions and blend points.
        let colors = nested_fields(&gradient, 4)
            .filter_map(|stop| nested_field(&stop, 1).and_then(|value| color(&value)))
            .take(2)
            .collect::<Vec<_>>();
        layer.insert("fillType".to_string(), json!("gradient"));
        layer.insert("fill".to_string(), json!("Gradient"));
        layer.insert("fillColors".to_string(), json!(colors));
        if let Some(angle) = f64_field(&gradient, 2) {
            layer.insert("fillAngle".to_string(), json!(angle));
        }
    } else if nested_field(fields, 3).is_some_and(|media| media_type(&media) == "live-video") {
        layer.insert("fillType".to_string(), json!("video-input"));
        layer.insert("fill".to_string(), json!(""));
        layer.insert("fillColors".to_string(), json!([]));
    } else if fields.iter().any(|field| field.number == 3) {
        layer.insert("fillType".to_string(), json!("media"));
        layer.insert("fill".to_string(), json!("Media"));
    } else if let Some((kind, values)) = effect {
        layer.insert("fillType".to_string(), json!("background-effect"));
        layer.insert("backgroundEffect".to_string(), json!(kind));
        layer.extend(values);
    }
    if !enabled && layer.contains_key("fillType") {
        layer.insert("fillType".into(), json!("none"));
    }
}

/// ParagraphStyle mirrors NSParagraphStyle, so its lengths are points in the same coordinate
/// space as the font size and import unchanged. `first_line_head_indent` and `head_indent` are
/// both measured from the leading margin, while Orbit stores the first line relative to the left
/// indent. A negative `tail_indent` is a trailing inset; a positive one is an absolute right edge. A zero
/// `line_height_multiple` means no multiple. Fields the source does not carry stay unset. Values
/// outside the range Orbit validates are dropped and listed under `unsupportedParagraphFields`.
/// A zero `default_tab_interval` means the system default and is not imported.
fn decode_paragraph_spacing(paragraph: &[Field]) -> Map<String, Value> {
    let mut style = Map::new();
    let mut unsupported = Vec::new();
    let mut put = |key: &str, value: Option<f64>| {
        if let Some(value) = value {
            style.insert(key.to_string(), json!(value));
        }
    };
    put(
        "textLineHeightMultiple",
        f64_field(paragraph, 5).filter(|value| *value > 0.0),
    );
    put(
        "textMaximumLineHeight",
        f64_field(paragraph, 6).filter(|value| *value > 0.0),
    );
    put(
        "textMinimumLineHeight",
        f64_field(paragraph, 7).filter(|value| *value > 0.0),
    );
    put("textLineSpacing", f64_field(paragraph, 8));
    put("textParagraphSpacingBefore", f64_field(paragraph, 10));
    put("textParagraphSpacingAfter", f64_field(paragraph, 9));
    let head = f64_field(paragraph, 3);
    put("textLeftIndent", head);
    put(
        "textFirstLineIndent",
        f64_field(paragraph, 2).map(|first| first - head.unwrap_or(0.0)),
    );
    let tail = f64_field(paragraph, 4);
    put(
        "textRightIndent",
        tail.filter(|tail| *tail < 0.0).map(|tail| -tail),
    );
    put("textAbsoluteRightEdge", tail.filter(|tail| *tail > 0.0));
    put(
        crate::native_text_tabs::INTERVAL_FIELD,
        f64_field(paragraph, 12).filter(|value| *value != 0.0),
    );
    for (field, reason) in crate::native_text_layout::invalid_paragraph_fields(&style) {
        style.remove(field);
        unsupported.push(json!(reason));
        // The first-line indent is relative to the left indent it was computed from.
        if field == "textLeftIndent" && style.remove("textFirstLineIndent").is_some() {
            unsupported.push(json!(
                "textFirstLineIndent is relative to the textLeftIndent that was not imported."
            ));
        }
    }
    decode_tab_stops(paragraph, &mut style, &mut unsupported);
    decode_text_list(paragraph, &mut style, &mut unsupported);
    if !unsupported.is_empty() {
        style.insert("unsupportedParagraphFields".to_string(), json!(unsupported));
    }
    style
}

/// Both ProPresenter and Orbit measure tab stops from the paragraph's leading box margin.
/// Stops are imported whole; unsupported alignment, invalid positions, duplicates and excessive
/// counts remain explicit diagnostics.
fn decode_tab_stops(
    paragraph: &[Field],
    style: &mut Map<String, Value>,
    unsupported: &mut Vec<Value>,
) {
    let stops = nested_fields(paragraph, 11).collect::<Vec<_>>();
    if stops.is_empty() {
        return;
    }
    let mut entries = Vec::new();
    let mut reasons = Vec::new();
    for stop in &stops {
        let location = f64_field(stop, 1).unwrap_or(0.0);
        let alignment = match varint_field(stop, 2).unwrap_or(0) {
            0 | 4 => "left",
            1 => "right",
            2 => "center",
            3 => "justify",
            _other => {
                let kind = "unknown";
                reasons.push(format!(
                    "tab_stops: the {kind} alignment of the stop at {location} is not supported"
                ));
                continue;
            }
        };
        entries.push(json!({"position": location, "alignment": alignment}));
    }
    let entries = Value::Array(entries);
    reasons.extend(crate::native_text_tabs::parse_tab_stops(&entries).err());
    if reasons.is_empty() {
        style.insert(crate::native_text_tabs::STOPS_FIELD.to_string(), entries);
        return;
    }
    unsupported.extend(reasons.into_iter().map(Value::from));
    unsupported.push(json!("tab_stops: none of the stops were imported"));
}

/// ProPresenter keeps which paragraphs are list items in the RTF; Orbit lists are layer-wide, so
/// the first level (`text_list`, else the first `text_lists` entry) applies to every non-empty
/// Paragraph-level list definitions. Unsupported marker types and values outside Orbit's
/// validation bounds are dropped and reported, never guessed or clamped.
fn decode_text_list_levels(
    paragraph: &[Field],
    unsupported: &mut Vec<Value>,
) -> Vec<Map<String, Value>> {
    use crate::native_text_list::{
        invalid_list_fields, POSTFIX_FIELD, PREFIX_FIELD, START_FIELD, STYLE_FIELD,
    };
    let single = nested_field(paragraph, 13);
    let mut levels = nested_fields(paragraph, 14).collect::<Vec<_>>();
    if let Some(single) = single {
        if levels.is_empty() {
            levels.push(single);
        } else {
            levels[0] = single;
        }
    }
    levels
        .iter()
        .enumerate()
        .map(|(index, list)| {
            let mut candidate = Map::new();
            if bool_field(list, 1) != Some(true) {
                return candidate;
            }
            let name = match varint_field(list, 2).unwrap_or(0) {
                0 => "box",
                1 => "check",
                3 => "diamond",
                4 => "disc",
                2 => "circle",
                6 => "square",
                5 => "hyphen",
                7 => "decimal",
                8 => "lower-alpha",
                9 => "upper-alpha",
                10 => "lower-roman",
                11 => "upper-roman",
                other => {
                    let kind = match other {
                        0 => "box",
                        1 => "check",
                        3 => "diamond",
                        _ => "unknown",
                    };
                    unsupported.push(json!(format!(
                        "text_list level {index}: the {kind} number type is not supported"
                    )));
                    return candidate;
                }
            };
            candidate.insert(STYLE_FIELD.to_string(), json!(name));
            for (field, key) in [(3, PREFIX_FIELD), (4, POSTFIX_FIELD)] {
                if let Some(text) = string_field(list, field).filter(|text| !text.is_empty()) {
                    candidate.insert(key.to_string(), json!(text));
                }
            }
            if let Some(start) = varint_field(list, 5).filter(|start| *start != 0) {
                candidate.insert(START_FIELD.to_string(), json!(start as i64));
            }
            for (field, reason) in invalid_list_fields(&candidate) {
                candidate.remove(field);
                unsupported.push(json!(format!("text_list level {index}: {reason}")));
            }
            candidate
        })
        .collect()
}

fn decode_text_list(
    paragraph: &[Field],
    style: &mut Map<String, Value>,
    unsupported: &mut Vec<Value>,
) {
    if let Some(first) = decode_text_list_levels(paragraph, unsupported).first() {
        style.extend(first.clone());
    }
}

fn decode_text_list_levels_from_text(text: &[Field]) -> Vec<Map<String, Value>> {
    let Some(paragraph) = nested_field(text, 3).and_then(|attributes| nested_field(&attributes, 6))
    else {
        return Vec::new();
    };
    decode_text_list_levels(&paragraph, &mut Vec::new())
}

/// RTF carries the membership and style of each paragraph while the protobuf paragraph carries
/// only a layer-wide default. Keep only RTF paragraph properties in dense per-paragraph patches.
fn apply_rtf_paragraph_styles(
    base: &mut Map<String, Value>,
    document: &RtfDocument,
    proto_levels: &[Map<String, Value>],
    text: &str,
) {
    const PARAGRAPH_KEYS: &[&str] = &[
        "textHorizontalAlign",
        "textLineHeightMultiple",
        "textLineSpacing",
        "textMinimumLineHeight",
        "textMaximumLineHeight",
        "textParagraphSpacingBefore",
        "textParagraphSpacingAfter",
        "textFirstLineIndent",
        "textLeftIndent",
        "textRightIndent",
        "textAbsoluteRightEdge",
        crate::native_text_tabs::INTERVAL_FIELD,
        crate::native_text_tabs::STOPS_FIELD,
        crate::native_text_tabs::BAR_STOPS_FIELD,
        crate::native_text_list::STYLE_FIELD,
        crate::native_text_list::PREFIX_FIELD,
        crate::native_text_list::POSTFIX_FIELD,
        crate::native_text_list::START_FIELD,
        "textListLevel",
    ];
    let expected = text.chars().filter(|character| *character == '\n').count() + 1;
    let mut paragraphs = document
        .paragraphs
        .iter()
        .take(expected)
        .cloned()
        .collect::<Vec<_>>();
    paragraphs.resize_with(expected, RtfParagraph::default);
    // The paragraph override model has no numeric sentinel for "no automatic tab interval".
    // Move a protobuf default into only the paragraphs that inherit it, so an explicit Apple
    // `\pardeftab0` can clear it without weakening the interval validator.
    let inherited_tab_interval = base.remove(crate::native_text_tabs::INTERVAL_FIELD);
    let mut patches = Vec::with_capacity(expected);
    let mut unsupported = base
        .remove("unsupportedParagraphFields")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    for (index, paragraph) in paragraphs.iter().enumerate() {
        let (mut parsed, mut reasons) = paragraph.style(&document.lists, proto_levels);
        if paragraph.paragraph_reset {
            // RTF \pard resets paragraph properties. The standard default tab interval is 720
            // twips (36 points); the remaining values are their neutral Orbit equivalents.
            let mut reset = Map::new();
            for (key, value) in [
                ("textHorizontalAlign", json!("left")),
                ("textLineHeightMultiple", json!(1.0)),
                ("textLineSpacing", json!(0.0)),
                ("textMinimumLineHeight", json!(0.0)),
                ("textMaximumLineHeight", json!(0.0)),
                ("textParagraphSpacingBefore", json!(0.0)),
                ("textParagraphSpacingAfter", json!(0.0)),
                ("textFirstLineIndent", json!(0.0)),
                ("textLeftIndent", json!(0.0)),
                ("textRightIndent", json!(0.0)),
                (crate::native_text_tabs::INTERVAL_FIELD, json!(36.0)),
                (crate::native_text_tabs::STOPS_FIELD, json!([])),
                (crate::native_text_list::STYLE_FIELD, json!("none")),
            ] {
                reset.insert(key.to_string(), value);
            }
            if base.contains_key(crate::native_text_tabs::BAR_STOPS_FIELD) {
                reset.insert(
                    crate::native_text_tabs::BAR_STOPS_FIELD.to_string(),
                    json!([]),
                );
            }
            if base.contains_key("textAbsoluteRightEdge")
                && !paragraph.values.contains_key("textRightIndent")
            {
                reset.remove("textRightIndent");
            }
            reset.extend(parsed);
            parsed = reset;
        }
        if paragraph.default_tab_interval_disabled {
            parsed.remove(crate::native_text_tabs::INTERVAL_FIELD);
        } else if !paragraph.paragraph_reset
            && !parsed.contains_key(crate::native_text_tabs::INTERVAL_FIELD)
        {
            if let Some(value) = inherited_tab_interval.as_ref() {
                parsed.insert(
                    crate::native_text_tabs::INTERVAL_FIELD.to_string(),
                    value.clone(),
                );
            }
        }
        let mut patch = Map::new();
        for key in PARAGRAPH_KEYS {
            if let Some(value) = parsed.get(*key) {
                patch.insert((*key).to_string(), value.clone());
            }
        }
        let mut effective: Map<String, Value> = base
            .iter()
            .filter(|(key, _)| key.as_str() != "textParagraphStyles")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if patch
            .get(crate::native_text_list::STYLE_FIELD)
            .and_then(Value::as_str)
            .is_some_and(|name| {
                matches!(
                    name,
                    "none" | "box" | "check" | "diamond" | "disc" | "circle" | "square" | "hyphen"
                )
            })
        {
            for key in [
                crate::native_text_list::PREFIX_FIELD,
                crate::native_text_list::POSTFIX_FIELD,
                crate::native_text_list::START_FIELD,
            ] {
                effective.remove(key);
            }
        }
        if patch.contains_key("textAbsoluteRightEdge") {
            effective.remove("textRightIndent");
        } else if patch.contains_key("textRightIndent") {
            effective.remove("textAbsoluteRightEdge");
        }
        effective.extend(patch.clone());
        for (key, reason) in crate::native_text_layout::invalid_paragraph_fields(&effective) {
            if matches!(key, "textMinimumLineHeight" | "textMaximumLineHeight") {
                patch.remove("textMinimumLineHeight");
                patch.remove("textMaximumLineHeight");
            } else {
                patch.remove(key);
            }
            reasons.push(reason);
        }
        if !reasons.is_empty() {
            unsupported.extend(
                reasons
                    .into_iter()
                    .map(|reason| json!(format!("paragraph {index}: {reason}"))),
            );
        }
        patches.push(Value::Object(patch));
    }
    if patches
        .iter()
        .any(|patch| patch.as_object().is_some_and(|patch| !patch.is_empty()))
    {
        base.insert("textParagraphStyles".to_string(), Value::Array(patches));
    }
    if !unsupported.is_empty() {
        base.insert(
            "unsupportedParagraphFields".to_string(),
            Value::Array(unsupported),
        );
    }
}

fn decode_text_style(text: &[Field]) -> Option<Map<String, Value>> {
    let attributes = nested_field(text, 3).unwrap_or_default();
    let mut style = Map::new();
    if let Some(transform) = varint_field(text, 10).and_then(|value| match value {
        0 => Some("None"),
        1 => Some("Remove line returns"),
        2 => Some("One word per line"),
        3 => Some("One character per line"),
        4 => Some("Replace line returns"),
        _ => None,
    }) {
        style.insert("lineTransform".into(), json!(transform));
    }
    if let Some(delimiter) = layer_link_export::string_field_including_empty(text, 11)
        .or_else(|| (varint_field(text, 10) == Some(4)).then(String::new))
    {
        style.insert("lineTransformDelimiter".into(), json!(delimiter));
    }
    if let Some(font) = nested_field(&attributes, 1) {
        for (field, key) in [(1, "fontFamily"), (9, "fontFamily"), (10, "fontFace")] {
            if let Some(value) = string_field(&font, field) {
                style.insert(key.to_string(), json!(value));
            }
        }
        if let Some(value) = f64_field(&font, 2) {
            style.insert("fontSize".to_string(), json!(value));
        }
        // These proto3 scalars default to false; preserve that explicit base instead of
        // inheriting Orbit's bold authoring default underneath otherwise regular RTF runs.
        style.insert(
            "italic".to_string(),
            json!(bool_field(&font, 4).unwrap_or(false)),
        );
        style.insert(
            "bold".to_string(),
            json!(bool_field(&font, 8).unwrap_or(false)),
        );
    }
    if let Some(value) = f64_field(&attributes, 7) {
        style.insert("letterSpacing".into(), json!(value));
    }
    if let Some(value) = varint_field(&attributes, 8) {
        style.insert("superscript".into(), json!(value as u32 as i32));
    }
    if let Some(value) = bool_field(text, 9) {
        style.insert("superscriptStandardized".into(), json!(value));
    }
    if let Some(value) = nested_field(&attributes, 3).and_then(|value| color(&value)) {
        style.insert("textColor".to_string(), json!(value));
    }
    if let Some(value) = nested_field(&attributes, 12).and_then(|value| color(&value)) {
        style.insert("strokeColor".to_string(), json!(value));
    }
    if let Some(value) = varint_field(&attributes, 2) {
        let capitalization = match value {
            0 => Some("None"),
            1 => Some("All Caps"),
            2 => Some("Small Caps"),
            3 => Some("Title Case"),
            4 => Some("Start Case"),
            _ => None,
        };
        if let Some(capitalization) = capitalization {
            style.insert("capitalization".to_string(), json!(capitalization));
        }
    }
    if let Some((kind, values)) =
        nested_field(&attributes, 18).and_then(|effect| decode_background_effect(&effect))
    {
        let mode = if kind == "blur" {
            "Background Blur"
        } else {
            "Background Invert"
        };
        style.insert("textFillMode".to_string(), json!(mode));
        style.extend(values);
    }
    if let Some(value) = f64_field(&attributes, 11) {
        style.insert("strokeWidth".to_string(), json!(value));
        style.insert("strokeEnabled".to_string(), json!(value > 0.0));
    }
    if let Some(shadow) = nested_field(text, 4) {
        if let Some(value) = bool_field(&shadow, 7) {
            style.insert("shadowEnabled".to_string(), json!(value));
        }
        for (field, key) in [
            (2, "shadowAngle"),
            (3, "shadowOffset"),
            (4, "shadowBlur"),
            (6, "shadowOpacity"),
        ] {
            if let Some(value) = f64_field(&shadow, field) {
                style.insert(
                    key.to_string(),
                    json!(if key == "shadowOpacity" {
                        opacity_percent(value)
                    } else {
                        value
                    }),
                );
            }
        }
        if let Some(value) = nested_field(&shadow, 5).and_then(|value| color(&value)) {
            style.insert("shadowColor".to_string(), json!(value));
        }
    }
    for (field, color_field, key) in [(4, 5, "underline"), (9, 10, "strikethrough")] {
        if let Some(line) = nested_field(&attributes, field) {
            let variant = varint_field(&line, 1).unwrap_or(0);
            style.insert(key.to_string(), json!(variant != 0));
            if let Some(name) = ["none", "single", "thick", "double"].get(variant as usize) {
                if variant > 1 {
                    style.insert(format!("{key}Style"), json!(name));
                }
            }
            let pattern = varint_field(&line, 2).unwrap_or(0);
            if let Some(name) =
                ["solid", "dot", "dash", "dash-dot", "dash-dot-dot"].get(pattern as usize)
            {
                if pattern != 0 {
                    style.insert(format!("{key}Pattern"), json!(name));
                }
            }
            if bool_field(&line, 3) == Some(true) {
                style.insert(format!("{key}ByWord"), json!(true));
            }
        }
        if let Some(value) = nested_field(&attributes, color_field).and_then(|value| color(&value))
        {
            style.insert(format!("{key}Color"), json!(value));
        }
    }
    if let Some(paragraph) = nested_field(&attributes, 6) {
        let value = varint_field(&paragraph, 1).unwrap_or(0);
        style.insert(
            "textHorizontalAlign".to_string(),
            json!(match value {
                0 => "left",
                4 => "natural",
                1 => "right",
                3 => "justify",
                _ => "center",
            }),
        );
        style.extend(decode_paragraph_spacing(&paragraph));
    }
    if let Some(value) = varint_field(text, 6) {
        style.insert(
            "textVerticalAlign".to_string(),
            json!(match value {
                0 => "top",
                2 => "bottom",
                _ => "middle",
            }),
        );
    }
    if let Some(value) = varint_field(text, 7) {
        let scaling = match value {
            0 => Some("None"),
            1 => Some("Container up or down"),
            2 => Some("Text down"),
            3 => Some("Text up"),
            4 => Some("Text up or down"),
            _ => None,
        };
        if let Some(scaling) = scaling {
            style.insert("scaling".to_string(), json!(scaling));
        }
    }
    if let Some(margins) = nested_field(text, 8) {
        for (field, key) in [
            (1, "textMarginLeft"),
            (2, "textMarginRight"),
            (3, "textMarginTop"),
            (4, "textMarginBottom"),
        ] {
            if let Some(value) =
                f64_field(&margins, field).filter(|value| value.is_finite() && *value >= 0.0)
            {
                style.insert(key.into(), json!(value));
            }
        }
    }
    if style.is_empty() {
        None
    } else {
        Some(style)
    }
}

fn media_type(fields: &[Field]) -> String {
    if fields.iter().any(|field| field.number == 4) {
        "audio".to_string()
    } else if fields.iter().any(|field| field.number == 5) {
        "image".to_string()
    } else if fields.iter().any(|field| field.number == 6) {
        "video".to_string()
    } else if fields.iter().any(|field| field.number == 7) {
        "live-video".to_string()
    } else if fields.iter().any(|field| field.number == 8) {
        "web".to_string()
    } else {
        "media".to_string()
    }
}

/// Scaling, alignment, flips, crop, rotation and effects reach the layer through
/// `decode_media_style`; the remaining drawing options have no Orbit setting.
fn live_video_drawing_is_mapped(drawing: &[Field]) -> bool {
    varint_field(drawing, 1) != Some(3)
        && bool_field(drawing, 9) != Some(true)
        && varint_field(drawing, 10).unwrap_or(0) == 0
        && !has_nonempty_field(drawing, 7)
}

fn has_nonempty_field(fields: &[Field], number: u32) -> bool {
    nested_field(fields, number).is_some_and(|nested| !nested.is_empty())
}

/// What ProPresenter writes for untouched live video audio: unity volume and eight channels
/// that alternate between its two outputs, none muted, limited or with a volume of its own.
fn live_video_audio_is_default(audio: &[Field]) -> bool {
    finite_f64_field(audio, 1).is_none_or(|volume| volume == 1.0)
        && !bool_field(audio, 3).unwrap_or(false)
        && nested_fields(audio, 2).all(|channel| {
            let index = varint_field(&channel, 1).unwrap_or(0);
            let outputs = nested_fields(&channel, 5)
                .map(|output| varint_field(&output, 2).unwrap_or(0))
                .collect::<Vec<_>>();
            !bool_field(&channel, 2).unwrap_or(false)
                && !bool_field(&channel, 4).unwrap_or(false)
                && !channel.iter().any(|field| field.number == 3)
                && outputs == [index % 2]
        })
}

fn live_video_source_losses(fields: &[Field]) -> Vec<String> {
    if media_type(fields) != "live-video" {
        return Vec::new();
    }
    let Some(properties) = nested_field(fields, 7) else {
        return vec!["live video type properties are malformed".into()];
    };
    let mut unsupported = Vec::new();
    for field in &properties {
        if !matches!(field.number, 1..=3) {
            unsupported.push(format!(
                "live video properties field {} is not represented by Orbit",
                field.number
            ));
        }
    }
    for (field, label) in [(1, "drawing"), (2, "audio")] {
        if !properties.iter().any(|entry| entry.number == field) {
            continue;
        }
        let Some(settings) = nested_field(&properties, field) else {
            unsupported.push(format!(
                "live video {label} properties have an invalid wire type"
            ));
            continue;
        };
        if label == "drawing" && !live_video_drawing_is_mapped(&settings) {
            unsupported.push(
                "live video drawing uses custom scaling, inverted alpha or a native rotation, which Orbit does not apply".into(),
            );
        } else if label == "audio" && !live_video_audio_is_default(&settings) {
            unsupported.push(
                "live video audio settings were not imported because Orbit plays live input audio through its audio inputs".into(),
            );
        }
    }
    if properties.iter().any(|field| field.number == 3) {
        let Some(live_video) = nested_field(&properties, 3) else {
            unsupported.push("live video device configuration has an invalid wire type".into());
            return unsupported;
        };
        for (field, label) in [(1, "video"), (2, "audio")] {
            if live_video.iter().any(|entry| entry.number == field) {
                if nested_field(&live_video, field).is_some_and(|settings| !settings.is_empty()) {
                    unsupported.push(format!(
                        "live video {label} device settings have no Orbit representation"
                    ));
                } else if nested_field(&live_video, field).is_none() {
                    unsupported.push(format!(
                        "live video {label} device settings have an invalid wire type"
                    ));
                }
            }
        }
        if live_video.iter().any(|field| field.number == 3) {
            if varint_field(&live_video, 3).is_some() {
                unsupported.push("live video source index has no Orbit representation".into());
            } else {
                unsupported.push("live video source index has an invalid wire type".into());
            }
        }
        for field in &live_video {
            if !(1..=3).contains(&field.number) {
                unsupported.push(format!(
                    "live video source field {} is not represented by Orbit",
                    field.number
                ));
            }
        }
    }
    unsupported
}

fn decode_media_style(fields: &[Field], layer: &mut Map<String, Value>, prefix: &str) {
    layer.insert(format!("{prefix}Type"), json!(media_type(fields)));
    if let Some(id) = uuid(fields, 1) {
        layer.insert(format!("{prefix}Id"), json!(id));
    }
    if let Some(url) = decode_url_value(fields, 2) {
        layer.insert(format!("{prefix}Url"), json!(url.clone()));
        layer.insert(format!("{prefix}SourcePath"), json!(url));
    }
    let properties = nested_field(fields, 5)
        .or_else(|| nested_field(fields, 6))
        .or_else(|| nested_field(fields, 4))
        .or_else(|| nested_field(fields, 7))
        .or_else(|| nested_field(fields, 8));
    if let Some(properties) = properties {
        let audio_type = media_type(fields) == "audio";
        let live_type = media_type(fields) == "live-video";
        let drawing = (!audio_type)
            .then(|| nested_field(&properties, 1))
            .flatten();
        // A live video layer has no transport; field 3 holds its device.
        let transport =
            nested_field(&properties, if audio_type { 2 } else { 3 }).filter(|_| !live_type);
        let audio =
            nested_field(&properties, if audio_type { 1 } else { 2 }).filter(|_| !live_type);
        if let Some(volume) = audio.as_ref().and_then(|audio| f64_field(audio, 1)) {
            layer.insert(format!("{prefix}Volume"), json!(volume * 100.0));
        }
        if let Some(drawing) = drawing {
            if let Some(value) = varint_field(&drawing, 1) {
                let scale = match value {
                    0 => "Scale to Fit",
                    1 => "Scale to Fill",
                    2 => "Stretch to Fill",
                    _ => "",
                };
                if !scale.is_empty() {
                    layer.insert(format!("{prefix}Scale"), json!(scale));
                }
            }
            if let Some(value) = varint_field(&drawing, 2) {
                let alignment = match value {
                    0 => "Middle Center",
                    1 => "Top Left",
                    2 => "Top Center",
                    3 => "Top Right",
                    4 => "Middle Right",
                    5 => "Bottom Right",
                    6 => "Bottom Center",
                    7 => "Bottom Left",
                    8 => "Middle Left",
                    _ => "",
                };
                if !alignment.is_empty() {
                    layer.insert(format!("{prefix}Alignment"), json!(alignment));
                }
            }
            let effects = nested_fields(&drawing, 12).collect::<Vec<_>>();
            if !effects.is_empty() {
                let mut unsupported = Vec::new();
                let chain =
                    decode_media_effect_chain(effects.iter().map(Vec::as_slice), &mut unsupported);
                if !chain.is_empty() {
                    layer.insert(format!("{prefix}Effects"), Value::Array(chain));
                }
                if !unsupported.is_empty() {
                    layer.insert("unsupportedMediaEffects".to_string(), json!(unsupported));
                }
            }
            if let Some(value) = bool_field(&drawing, 3) {
                layer.insert(format!("{prefix}FlipX"), json!(value));
            }
            if let Some(value) = bool_field(&drawing, 4) {
                layer.insert(format!("{prefix}FlipY"), json!(value));
            }
            if let Some(alpha) = varint_field(&drawing, 15).and_then(|alpha| match alpha {
                1 => Some("Straight"),
                2 => Some("Premultiplied"),
                _ => None,
            }) {
                layer.insert(format!("{prefix}AlphaType"), json!(alpha));
            }
            if let Some(value) = f64_field(&drawing, 6) {
                layer.insert(format!("{prefix}Rotation"), json!(value));
            }
            if let Some(value) = bool_field(&drawing, 13) {
                layer.insert(format!("{prefix}Crop"), json!(value));
            }
            if let Some(crop) = nested_field(&drawing, 14) {
                for (field, key) in [(1, "Left"), (2, "Right"), (3, "Top"), (4, "Bottom")] {
                    if let Some(value) = f64_field(&crop, field) {
                        layer.insert(format!("{prefix}Crop{key}"), json!(value));
                    }
                }
            }
        }
        if let Some(transport) = transport {
            if let Some(value) = varint_field(&transport, 12) {
                layer.insert(
                    format!("{prefix}Playback"),
                    json!(if value == 0 { "Stop" } else { "Loop" }),
                );
            }
            for (field, key) in [
                (7, "RampIn"),
                (8, "RampOut"),
                (13, "LoopTime"),
                (14, "TimesToLoop"),
            ] {
                if let Some(value) = if field == 14 {
                    varint_field(&transport, field).map(|v| v as f64)
                } else {
                    f64_field(&transport, field)
                } {
                    layer.insert(format!("{prefix}{key}"), json!(value));
                }
            }
        }
    }
}

/// Decode a Template.Document (the `Theme` file found in a ProPresenter
/// Themes folder) into the Orbit theme contract.
pub fn decode_template(bytes: &[u8], source: &Path, fallback_title: &str) -> Option<Value> {
    let fields = decode_message(bytes).ok()?;
    let slides = nested_fields(&fields, 3)
        .filter_map(|slide| {
            let base = nested_field(&slide, 1)?;
            let name = string_field(&slide, 2);
            Some(decode_base_slide(&base, name))
        })
        .enumerate()
        .map(|(index, mut slide)| {
            if let Some(object) = slide.as_object_mut() {
                object.insert("id".to_string(), json!(format!("slide-{}", index + 1)));
                if object
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .is_empty()
                {
                    object.insert(
                        "name".to_string(),
                        json!(format!("Theme Slide {}", index + 1)),
                    );
                }
                let text = object
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Text");
                object.insert("text".to_string(), json!(text));
                object.insert(
                    "background".to_string(),
                    object
                        .get("background")
                        .cloned()
                        .unwrap_or_else(|| json!("#202020")),
                );
                object.insert(
                    "backgroundEnabled".to_string(),
                    object
                        .get("backgroundEnabled")
                        .cloned()
                        .unwrap_or_else(|| json!(true)),
                );
                let width = object
                    .get("width")
                    .and_then(Value::as_f64)
                    .unwrap_or(1920.0)
                    .max(320.0);
                let height = object
                    .get("height")
                    .and_then(Value::as_f64)
                    .unwrap_or(1080.0)
                    .max(240.0);
                object.insert("width".to_string(), json!(width));
                object.insert("height".to_string(), json!(height));
            }
            slide
        })
        .collect::<Vec<_>>();
    if slides.is_empty() {
        return None;
    }
    Some(json!({
        "title": fallback_title,
        "sourcePath": source.to_string_lossy(),
        "sourceType": "propresenter",
        "editable": false,
        "schemaVersion": 2,
        "size": { "width": 1920, "height": 1080 },
        "slides": slides,
    }))
}

/// Decode a TimersDocument from Configuration/Timers.  ProPresenter stores
/// timers in one container, but accepting a document with no timers returns
/// `None` so unrelated configuration files are ignored by the importer.
pub fn decode_timers(bytes: &[u8]) -> Option<Vec<Value>> {
    let fields = decode_message(bytes).ok()?;
    let timers = nested_fields(&fields, 3)
        .filter_map(|timer| {
            let id = uuid(&timer, 1)?;
            let name = string_field(&timer, 2).filter(|value| !value.trim().is_empty())?;
            let mut value = timer_configuration(&nested_field(&timer, 10)?)?;
            value.insert("id".to_string(), json!(id));
            value.insert("name".to_string(), json!(name));
            Some(Value::Object(value))
        })
        .collect::<Vec<_>>();
    (!timers.is_empty()).then_some(timers)
}

fn timer_configuration(configuration: &[Field]) -> Option<Map<String, Value>> {
    let allows_overrun = bool_field(configuration, 4).unwrap_or(false);
    let mut value = Map::new();
    value.insert("allowsOverrun".to_string(), json!(allows_overrun));
    if let Some(countdown) = nested_field(configuration, 1) {
        let duration = f64_field(&countdown, 1)?;
        if !duration.is_finite() || duration < 0.0 {
            return None;
        }
        value.insert("type".to_string(), json!("countdown"));
        value.insert("durationSeconds".to_string(), json!(duration));
    } else if let Some(countdown) = nested_field(configuration, 2) {
        let time_of_day = match countdown.iter().rev().find(|field| field.number == 1) {
            None => 0.0,
            Some(Field {
                value: WireValue::Fixed64(value),
                ..
            }) => f64::from_bits(*value),
            Some(_) => return None,
        };
        if !time_of_day.is_finite() || !(0.0..86_400.0).contains(&time_of_day) {
            return None;
        }
        let period = match countdown.iter().rev().find(|field| field.number == 2) {
            None => 0,
            Some(Field {
                value: WireValue::Varint(value),
                ..
            }) => *value,
            Some(_) => return None,
        };
        let target = match period {
            0 => time_of_day % 43_200.0,
            1 => time_of_day % 43_200.0 + 43_200.0,
            2 => time_of_day,
            _ => return None,
        };
        value.insert("type".to_string(), json!("countdown-to-time"));
        value.insert("durationSeconds".to_string(), json!(target));
        value.insert("targetTimeSeconds".to_string(), json!(target));
        if period < 2 {
            value.insert(
                "format".into(),
                json!({"targetTimePeriod": if period == 0 { "am" } else { "pm" }}),
            );
        }
    } else if let Some(elapsed) = nested_field(configuration, 3) {
        let start = f64_field(&elapsed, 1)?;
        if !start.is_finite() || start < 0.0 {
            return None;
        }
        value.insert("type".to_string(), json!("elapsed"));
        let has_end = bool_field(&elapsed, 3).unwrap_or(false);
        let end = if has_end {
            let end = f64_field(&elapsed, 2)?;
            if !end.is_finite() || end < 0.0 {
                return None;
            }
            Some(end)
        } else {
            None
        };
        value.insert("durationSeconds".to_string(), json!(end.unwrap_or(start)));
        value.insert("elapsedStartSeconds".to_string(), json!(start));
        if let Some(end) = end {
            value.insert("elapsedEndSeconds".to_string(), json!(end));
        }
    } else {
        return None;
    }
    Some(value)
}

/// Encode Orbit countdown timer records as a ProPresenter TimersDocument.
pub fn encode_timers(timers: &[Value]) -> Vec<u8> {
    let mut document = Vec::new();
    for timer in timers {
        let mut value = Vec::new();
        put_uuid(
            1,
            timer.get("id").and_then(Value::as_str).unwrap_or(""),
            &mut value,
        );
        put_string(
            2,
            timer.get("name").and_then(Value::as_str).unwrap_or("Timer"),
            &mut value,
        );
        let mut configuration = Vec::new();
        put_varint(
            4,
            u64::from(
                timer
                    .get("allowsOverrun")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ),
            &mut configuration,
        );
        match timer
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("countdown")
        {
            "countdown-to-time" => {
                let target = timer
                    .get("targetTimeSeconds")
                    .or_else(|| timer.get("durationSeconds"))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let countdown = encode_countdown_clock(target, &timer["format"]);
                put_message(2, countdown, &mut configuration);
            }
            "elapsed" => {
                let mut elapsed = Vec::new();
                put_f64(
                    1,
                    timer
                        .get("elapsedStartSeconds")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                    &mut elapsed,
                );
                if let Some(end) = timer.get("elapsedEndSeconds").and_then(Value::as_f64) {
                    put_f64(2, end, &mut elapsed);
                    put_varint(3, 1, &mut elapsed);
                }
                put_message(3, elapsed, &mut configuration);
            }
            _ => {
                let mut countdown = Vec::new();
                put_f64(
                    1,
                    timer
                        .get("durationSeconds")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                    &mut countdown,
                );
                put_message(1, countdown, &mut configuration);
            }
        }
        put_message(10, configuration, &mut value);
        put_message(3, value, &mut document);
    }
    document
}

fn encode_countdown_clock(target: f64, format: &Value) -> Vec<u8> {
    let period = match format["targetTimePeriod"].as_str() {
        Some("am") if target < 43_200. => 0,
        Some("pm") if target >= 43_200. => 1,
        _ => 2,
    };
    let time = if period == 2 {
        target
    } else {
        target % 43_200.
    };
    let mut countdown = Vec::new();
    put_f64(1, time, &mut countdown);
    put_varint(2, period, &mut countdown);
    countdown
}

/// Decode ProMask records from either a MaskCollection (field 1 `collection`)
/// or a ProPresenterWorkspace (field 4 `masks`) container.
pub fn decode_masks(bytes: &[u8], source: &Path) -> Option<Vec<Value>> {
    let fields = decode_message(bytes).ok()?;
    let collection = nested_fields(&fields, 1)
        .filter(|entry| {
            nested_field(entry, 1)
                .and_then(|base| uuid(&base, 7))
                .is_some()
        })
        .collect::<Vec<_>>();
    let containers = if collection.is_empty() {
        nested_fields(&fields, 4).collect::<Vec<_>>()
    } else {
        collection
    };
    let masks = containers
        .into_iter()
        .enumerate()
        .filter_map(|(index, mask)| {
            let base = nested_field(&mask, 1)?;
            let title = string_field(&mask, 2).unwrap_or_else(|| format!("Mask {}", index + 1));
            let id = uuid(&base, 7).unwrap_or_else(|| format!("mask-{}", index + 1));
            let mut slide = decode_base_slide(&base, Some(title.clone()));
            if let Some(object) = slide.as_object_mut() {
                object.insert("text".to_string(), json!(""));
                object.insert("content".to_string(), json!(""));
                object.insert("background".to_string(), json!("transparent"));
                object.insert("backgroundEnabled".to_string(), json!(false));
                let width = object
                    .get("width")
                    .and_then(Value::as_f64)
                    .unwrap_or(1920.0)
                    .max(320.0);
                let height = object
                    .get("height")
                    .and_then(Value::as_f64)
                    .unwrap_or(1080.0)
                    .max(240.0);
                object.insert("width".to_string(), json!(width));
                object.insert("height".to_string(), json!(height));
            }
            Some(json!({
                "id": id,
                "title": title,
                "sourcePath": source.to_string_lossy(),
                "sourceType": "propresenter",
                "editable": true,
                "schemaVersion": 1,
                "size": {
                    "width": nested_field(&base, 6).and_then(|value| f64_field(&value, 1)).unwrap_or(1920.0),
                    "height": nested_field(&base, 6).and_then(|value| f64_field(&value, 2)).unwrap_or(1080.0)
                },
                "slide": slide,
            }))
        })
        .collect::<Vec<_>>();
    (!masks.is_empty()).then_some(masks)
}

fn encode_masks_with(
    masks: &[Value],
    relative_media: Option<&BTreeMap<String, String>>,
) -> Vec<u8> {
    let mut collection = Vec::new();
    for mask in masks {
        let mut pro_mask = Vec::new();
        let slide = mask.get("slide").unwrap_or(mask);
        let encoded_cue = encode_slide(slide, 0, relative_media);
        let Some(cue) = decode_message(&encoded_cue).ok() else {
            continue;
        };
        let Some(action) = nested_field(&cue, 10) else {
            continue;
        };
        let Some(slide_type) = nested_field(&action, 23) else {
            continue;
        };
        let Some(presentation) = nested_field(&slide_type, 2) else {
            continue;
        };
        let Some(base) = bytes_field(&presentation, 1) else {
            continue;
        };
        put_message(1, base.to_vec(), &mut pro_mask);
        put_string(
            2,
            mask.get("title").and_then(Value::as_str).unwrap_or("Mask"),
            &mut pro_mask,
        );
        put_message(1, pro_mask, &mut collection);
    }
    collection
}

/// Workspace mask records live in ProPresenterWorkspace field 4; each mask slide exports its
/// own layers with `relative_media` as `encode_presentation_with_media` does.
pub fn encode_workspace_masks(
    masks: &[Value],
    relative_media: &BTreeMap<String, String>,
) -> Vec<u8> {
    let collection = decode_message(&encode_masks_with(masks, Some(relative_media)))
        .expect("Encoded masks must be valid protobuf");
    let mut workspace = Vec::new();
    for mask in bytes_fields(&collection, 1) {
        put_message(4, mask.to_vec(), &mut workspace);
    }
    workspace
}

/// Decode a Stage.Document containing Stage.Layout records.
pub fn decode_stage(bytes: &[u8], source: &Path) -> Option<Vec<Value>> {
    let fields = decode_message(bytes).ok()?;
    let layouts = nested_fields(&fields, 2)
        .enumerate()
        .filter_map(|(index, layout)| {
            let slide = nested_field(&layout, 3)?;
            let title =
                string_field(&layout, 2).unwrap_or_else(|| format!("Stage Layout {}", index + 1));
            let id = uuid(&layout, 1).unwrap_or_else(|| format!("stage-layout-{}", index + 1));
            Some(json!({
                "id": id,
                "title": title,
                "sourcePath": source.to_string_lossy(),
                "sourceType": "propresenter",
                "slide": decode_base_slide(&slide, Some(title)),
            }))
        })
        .collect::<Vec<_>>();
    (!layouts.is_empty()).then_some(layouts)
}

/// Encode Orbit stage layouts as a ProPresenter Stage.Document.
pub fn encode_stage(layouts: &[Value]) -> Vec<u8> {
    let mut document = Vec::new();
    for layout in layouts {
        let slide = layout.get("slide").unwrap_or(layout);
        let encoded_cue = encode_slide(slide, 0, None);
        let Some(cue) = decode_message(&encoded_cue).ok() else {
            continue;
        };
        let Some(action) = nested_field(&cue, 10) else {
            continue;
        };
        let Some(slide_type) = nested_field(&action, 23) else {
            continue;
        };
        let Some(presentation) = nested_field(&slide_type, 2) else {
            continue;
        };
        let Some(base) = bytes_field(&presentation, 1) else {
            continue;
        };
        let mut stage_layout = Vec::new();
        if let Some(id) = layout.get("id").and_then(Value::as_str) {
            put_uuid(1, id, &mut stage_layout);
        }
        put_string(
            2,
            layout
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Stage Layout"),
            &mut stage_layout,
        );
        put_message(3, base.to_vec(), &mut stage_layout);
        put_message(2, stage_layout, &mut document);
    }
    document
}

/// Decode PropDocument cue slides and their collection folders.
pub fn decode_props(bytes: &[u8], source: &Path) -> Option<Vec<Value>> {
    let fields = decode_message(bytes).ok()?;
    let fallback_title = source
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Prop");
    let mut folders = Map::new();
    let mut single_prop_folders = Vec::new();
    let mut cues = nested_fields(&fields, 2).collect::<Vec<_>>();
    // ProPresenter 19 added prop collections; their cues (field 5) follow the document cues.
    for collection in nested_fields(&fields, 4) {
        let name = string_field(&collection, 2).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        if bool_field(&collection, 4) == Some(true) {
            single_prop_folders.push(name.clone());
        }
        for item in nested_fields(&collection, 3) {
            if let Some(id) = uuid(&item, 1) {
                folders.insert(id, Value::String(name.clone()));
            }
        }
        for cue in nested_fields(&collection, 5) {
            let id = uuid(&cue, 1);
            if id.is_some() && cues.iter().any(|existing| uuid(existing, 1) == id) {
                continue;
            }
            if let Some(id) = id {
                folders.insert(id, Value::String(name.clone()));
            }
            cues.push(cue);
        }
    }
    let props = cues
        .into_iter()
        .enumerate()
        .flat_map(|(index, cue)| {
            let cue_id = uuid(&cue, 1).unwrap_or_else(|| format!("prop-{}", index + 1));
            let title = string_field(&cue, 2)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    if index == 0 {
                        fallback_title.to_string()
                    } else {
                        format!("{fallback_title} {}", index + 1)
                    }
                });
            let folder = folders
                .get(&cue_id)
                .and_then(Value::as_str)
                .unwrap_or_default();
            nested_fields(&cue, 10)
                .filter_map(|action| {
                    let slide_type = nested_field(&action, 23)?;
                    let prop = nested_field(&slide_type, 3)?;
                    let base = nested_field(&prop, 1)?;
                    let id = format!("prop-{cue_id}");
                    let mut value = Map::new();
                    value.insert("id".to_string(), json!(id));
                    if let Some(source_id) = uuid(&cue, 1) {
                        value.insert("sourceId".to_string(), json!(source_id));
                    }
                    value.insert("title".to_string(), json!(&title));
                    value.insert("folder".to_string(), json!(folder));
                    value.insert("sourcePath".to_string(), json!(source.to_string_lossy()));
                    value.insert("sourceType".to_string(), json!("propresenter"));
                    let mut slide = decode_base_slide(&base, Some(title.clone()));
                    if let Some(object) = slide.as_object_mut() {
                        object.entry("layers").or_insert_with(|| json!([]));
                        object.insert(
                            "id".to_string(),
                            json!(uuid(&base, 7).unwrap_or_else(|| format!("{id}-slide"))),
                        );
                    }
                    if let Some(duration) = prop_auto_clear_seconds(&prop) {
                        value.insert("autoClearSeconds".to_string(), json!(duration));
                    }
                    if let Some(transition) = nested_field(&prop, 2) {
                        insert_transition(&mut value, &transition);
                    }
                    value.insert("slide".to_string(), slide);
                    if single_prop_folders.iter().any(|name| name == folder) {
                        note_unsupported(
                            &mut value,
                            vec!["collection shows only one prop at a time".to_string()],
                        );
                    }
                    Some(Value::Object(value))
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    (!props.is_empty()).then_some(props)
}

/// `PropSlide` auto-clear (fields 3 and 4, added in ProPresenter 19) only applies when enabled.
fn prop_auto_clear_seconds(prop: &[Field]) -> Option<f64> {
    f64_field(prop, 4).filter(|_| bool_field(prop, 3) == Some(true))
}

/// The props layer transition of a ProPresenter PropDocument (field 3) as `transition`,
/// or `unsupportedTransition` when it is not a library transition Orbit plays.
pub fn decode_props_transition(bytes: &[u8]) -> Option<Map<String, Value>> {
    let transition = nested_field(&decode_message(bytes).ok()?, 3)?;
    let mut object = Map::new();
    insert_transition(&mut object, &transition);
    Some(object)
}

/// ProPresenter's global layer transitions from `AppPreferences.proPref` (`Key=Value;`
/// lines). The background and foreground layers store a `TransitionStoreType`: Cut (0),
/// Auto (1, the library Dissolve at the stored auto duration or else its library duration)
/// or Advanced (2, a base64 `Transition`); the message layer stores its base64 `Transition`.
/// Returns Orbit's `layerTransitions` and the transitions that could not be imported.
pub fn decode_layer_transition_preferences(text: &str) -> (Map<String, Value>, Vec<String>) {
    let preferences = text
        .lines()
        .filter_map(|line| line.trim().strip_suffix(';')?.split_once('='))
        .collect::<BTreeMap<_, _>>();
    let mut layers = Map::new();
    let mut unsupported = Vec::new();
    // A stored base64 `Transition`, or what kept it from importing.
    let stored = |layer: &str, value: Option<&&str>, unsupported: &mut Vec<String>| {
        let mut object = Map::new();
        if let Some(fields) = value
            .and_then(|value| crate::macro_image::decode_data(value).ok())
            .and_then(|bytes| decode_message(&bytes).ok())
        {
            insert_transition(&mut object, &fields);
        }
        if let Some(names) = object.get("unsupportedTransitionVariables") {
            unsupported.push(format!("{layer} layer transition variables {names}"));
        }
        let transition = object.remove("transition");
        if transition.is_none() {
            unsupported.push(format!(
                "{layer} layer transition {}",
                object
                    .get("unsupportedTransition")
                    .cloned()
                    .unwrap_or_else(|| json!("cannot be read"))
            ));
        }
        transition
    };
    for (key, layer) in [("Background", "background"), ("Foreground", "foreground")] {
        let Some(store) = preferences.get(format!("Global{key}TransitionType").as_str()) else {
            continue;
        };
        match *store {
            "0" => {
                layers.insert(
                    layer.to_string(),
                    json!({ "effectId": crate::native_transitions::CUT_ID, "durationSeconds": 0.0 }),
                );
            }
            "1" => {
                let duration =
                    match preferences.get(format!("Global{key}AutoTransitionDuration").as_str()) {
                        Some(value) => value
                            .parse::<f64>()
                            .ok()
                            .filter(|value| value.is_finite() && (0.0..=10.0).contains(value)),
                        None => Some(crate::native_transitions::LIBRARY_DURATION_SECONDS),
                    };
                match duration {
                    Some(duration) => {
                        layers.insert(
                            layer.to_string(),
                            json!({ "effectId": crate::native_transitions::DISSOLVE_ID, "durationSeconds": duration }),
                        );
                    }
                    None => unsupported.push(format!("{layer} layer Auto transition duration")),
                }
            }
            "2" => {
                let value = preferences.get(format!("Global{key}AdvancedTransition").as_str());
                if let Some(transition) = stored(layer, value, &mut unsupported) {
                    layers.insert(layer.to_string(), transition);
                }
            }
            other => unsupported.push(format!("{layer} layer transition type {other}")),
        }
    }
    if let Some(value) = preferences.get("GlobalMessageTransition") {
        if let Some(transition) = stored("messages", Some(value), &mut unsupported) {
            layers.insert("messages".to_string(), transition);
        }
    }
    if preferences.keys().any(|key| key.starts_with("GlobalBible")) {
        unsupported
            .push("Bible layer transition (Orbit shows Bible text as presentation slides)".into());
    }
    (layers, unsupported)
}

/// `encode_props` with each prop slide's own layers, as `encode_presentation_with_media` writes them.
pub fn encode_props_with_media(
    props: &[Value],
    layer_transition: Option<&Value>,
    relative_media: &BTreeMap<String, String>,
) -> Vec<u8> {
    encode_props_with(props, layer_transition, Some(relative_media))
}

fn encode_props_with(
    props: &[Value],
    layer_transition: Option<&Value>,
    relative_media: Option<&BTreeMap<String, String>>,
) -> Vec<u8> {
    let mut document = Vec::new();
    let mut collections: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (index, prop) in props.iter().enumerate() {
        let slide = prop.get("slide").unwrap_or(prop);
        let encoded_cue = encode_slide(slide, index, relative_media);
        let Some(cue_fields) = decode_message(&encoded_cue).ok() else {
            continue;
        };
        let Some(action) = nested_field(&cue_fields, 10) else {
            continue;
        };
        let Some(slide_type) = nested_field(&action, 23) else {
            continue;
        };
        let Some(presentation) = nested_field(&slide_type, 2) else {
            continue;
        };
        let Some(base) = bytes_field(&presentation, 1) else {
            continue;
        };
        let id = prop.get("id").and_then(Value::as_str).unwrap_or("prop");
        let cue_id = normalized_uuid(id);
        let title = prop.get("title").and_then(Value::as_str).unwrap_or("Prop");
        let mut prop_slide = Vec::new();
        put_message(1, base.to_vec(), &mut prop_slide);
        if let Some(duration) = prop
            .get("autoClearSeconds")
            .or_else(|| slide.get("autoClearDuration"))
            .and_then(Value::as_f64)
        {
            put_varint(3, 1, &mut prop_slide);
            put_f64(4, duration, &mut prop_slide);
        }
        if let Some(transition) = prop.get("transition").and_then(encode_transition) {
            put_message(2, transition, &mut prop_slide);
        }
        let mut prop_type = Vec::new();
        put_message(3, prop_slide, &mut prop_type);
        let mut action_value = Vec::new();
        put_uuid(1, "", &mut action_value);
        put_string(2, title, &mut action_value);
        put_varint(6, 1, &mut action_value);
        put_varint(9, 15, &mut action_value); // ACTION_TYPE_PROP_SLIDE
        put_message(23, prop_type, &mut action_value);
        let mut cue = Vec::new();
        put_uuid(1, &cue_id, &mut cue);
        put_string(2, title, &mut cue);
        put_message(10, action_value, &mut cue);
        put_varint(12, 1, &mut cue);
        put_message(2, cue, &mut document);
        if let Some(folder) = prop
            .get("folder")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            collections
                .entry(folder.to_string())
                .or_default()
                .push(cue_id);
        }
    }
    for (name, ids) in collections {
        let mut collection = Vec::new();
        put_uuid(1, "", &mut collection);
        put_string(2, &name, &mut collection);
        for id in ids {
            let mut item = Vec::new();
            put_uuid(1, &id, &mut item);
            put_message(3, item, &mut collection);
        }
        put_message(4, collection, &mut document);
    }
    if let Some(transition) = layer_transition.and_then(encode_transition) {
        put_message(3, transition, &mut document);
    }
    document
}

pub fn decode_groups(bytes: &[u8]) -> Option<Vec<Value>> {
    let fields = decode_message(bytes).ok()?;
    let groups = nested_fields(&fields, 1)
        .filter_map(|group| {
            let id = uuid(&group, 1)?;
            let name = string_field(&group, 2).filter(|value| !value.trim().is_empty())?;
            let mut value = Map::new();
            value.insert("id".to_string(), json!(id));
            value.insert("name".to_string(), json!(name));
            if let Some(color) = nested_field(&group, 3).and_then(|value| color(&value)) {
                value.insert("color".to_string(), json!(color));
            }
            if let Some(key) = nested_field(&group, 4)
                .and_then(|hot_key| varint_field(&hot_key, 1))
                .and_then(group_hot_key)
            {
                value.insert("key".to_string(), json!(key));
            }
            Some(Value::Object(value))
        })
        .collect::<Vec<_>>();
    (!groups.is_empty()).then_some(groups)
}

fn group_hot_key(code: u64) -> Option<String> {
    match code {
        1..=26 => Some(((b'A' + (code as u8 - 1)) as char).to_string()),
        27..=36 => Some(((b'0' + (code as u8 - 27)) as char).to_string()),
        65..=84 => Some(format!("F{}", code - 64)),
        86 => Some("RETURN".to_string()),
        87 => Some("TAB".to_string()),
        88 => Some("SPACE".to_string()),
        89 => Some("DELETE".to_string()),
        90 => Some("ESCAPE".to_string()),
        103 => Some("HOME".to_string()),
        104 => Some("PAGE_UP".to_string()),
        106 => Some("END".to_string()),
        107 => Some("PAGE_DOWN".to_string()),
        108 => Some("LEFT_ARROW".to_string()),
        109 => Some("RIGHT_ARROW".to_string()),
        110 => Some("DOWN_ARROW".to_string()),
        111 => Some("UP_ARROW".to_string()),
        _ => None,
    }
}

pub fn encode_groups(groups: &[Value]) -> Vec<u8> {
    let mut document = Vec::new();
    for group in groups {
        let mut value = Vec::new();
        put_uuid(
            1,
            group.get("id").and_then(Value::as_str).unwrap_or(""),
            &mut value,
        );
        put_string(
            2,
            group.get("name").and_then(Value::as_str).unwrap_or("Group"),
            &mut value,
        );
        if let Some(color) = group.get("color").and_then(Value::as_str) {
            put_color(3, color, &mut value);
        }
        put_message(1, value, &mut document);
    }
    document
}

/// Decode the portable keyboard/MIDI subset of a ProPresenter KeyMappingDocument.
/// The previous importer intentionally read the common mappings and the
/// Windows mappings, leaving macOS-only mappings out of the portable contract.
pub fn decode_key_mappings(bytes: &[u8]) -> Option<Vec<Value>> {
    let fields = decode_message(bytes).ok()?;
    let bindings = nested_fields(&fields, 2)
        .chain(nested_fields(&fields, 4))
        .filter_map(|mapping| decode_key_mapping(&mapping))
        .collect::<Vec<_>>();
    // ProPresenter writes a document with only application info when no keys are mapped.
    (!bindings.is_empty() || fields.iter().all(|field| (1..=4).contains(&field.number)))
        .then_some(bindings)
}

fn decode_key_mapping(fields: &[Field]) -> Option<Value> {
    let (target_type, target_key) = if let Some(menu_item) =
        string_field(fields, 100).filter(|value| !value.trim().is_empty())
    {
        ("command", menu_item)
    } else {
        [
            (101, "clear-group"),
            (102, "cue"),
            (103, "group"),
            (104, "macro"),
            (105, "prop"),
            (106, "timer"),
        ]
        .into_iter()
        .find_map(|(field, target_type)| {
            collection_element_uuid(fields, field).map(|target_id| (target_type, target_id))
        })?
    };

    let keyboard = nested_field(fields, 1);
    let key = keyboard
        .as_ref()
        .and_then(|value| string_field(value, 1))
        .unwrap_or_default()
        .trim()
        .to_string();
    let modifiers = keyboard
        .as_ref()
        .map(|value| {
            varint_fields(value, 2)
                .filter_map(key_mapping_modifier)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let shortcut_key = if key.chars().count() == 1 {
        key.to_ascii_uppercase()
    } else {
        key
    };
    let shortcut = if shortcut_key.is_empty() {
        String::new()
    } else if modifiers.is_empty() {
        shortcut_key
    } else {
        format!("{}+{}", modifiers.join("+"), shortcut_key)
    };

    let midi = nested_field(fields, 2).and_then(|value| {
        let channel = varint_field(&value, 1).and_then(|value| i32::try_from(value).ok())?;
        let pitch = varint_field(&value, 2).and_then(|value| i32::try_from(value).ok())?;
        let velocity = varint_field(&value, 3)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(0);
        Some(json!({ "channel": channel, "pitch": pitch, "velocity": velocity }))
    });
    if shortcut.is_empty() && midi.is_none() {
        return None;
    }

    let input_id = if shortcut.is_empty() {
        let midi = midi.as_ref()?.as_object()?;
        format!(
            "{}:{}",
            midi.get("channel").and_then(Value::as_i64)?,
            midi.get("pitch").and_then(Value::as_i64)?
        )
    } else {
        shortcut.clone()
    };
    let mut value = Map::new();
    value.insert(
        "id".to_string(),
        json!(format!("{target_type}:{target_key}:{input_id}")),
    );
    value.insert("type".to_string(), json!(target_type));
    if target_type == "command" {
        value.insert("menuItem".to_string(), json!(target_key));
        let command = match target_key.as_str() {
            "MAIN_MENU_PRESENTATION_NEW_SLIDE_title" => Some("newSlide"),
            "MAIN_MENU_PRESENTATION_NEXT_SLIDE_title" => Some("nextSlide"),
            "MAIN_MENU_PRESENTATION_PREVIOUS_SLIDE_title" => Some("previousSlide"),
            "MAIN_MENU_PRESENTATION_NEXT_BACKGROUND_title" => Some("nextBackground"),
            "MAIN_MENU_PRESENTATION_PREVIOUS_BACKGROUND_title" => Some("previousBackground"),
            "MAIN_MENU_PRESENTATION_CLEAR_SLIDE_title" => Some("clearSlide"),
            "MAIN_MENU_PRESENTATION_CLEAR_MEDIA_title" => Some("clearMedia"),
            "MAIN_MENU_PRESENTATION_CLEAR_PROPS_title" => Some("clearProps"),
            "MAIN_MENU_PRESENTATION_CLEAR_AUDIO_title" => Some("clearAudio"),
            "MAIN_MENU_PRESENTATION_CLEAR_MESSAGES_title" => Some("clearMessages"),
            "MAIN_MENU_PRESENTATION_CLEAR_ANNOUNCEMENTS_title" => Some("clearAnnouncements"),
            "MAIN_MENU_PRESENTATION_CLEAR_TO_LOGO_title" => Some("clearToLogo"),
            _ => None,
        };
        if let Some(command) = command {
            value.insert("targetId".to_string(), json!(command));
        }
    } else {
        value.insert("targetId".to_string(), json!(target_key));
    }
    if !shortcut.is_empty() {
        value.insert("shortcut".to_string(), json!(shortcut));
    }
    if let Some(midi) = midi {
        value.insert("midi".to_string(), midi);
    }
    Some(Value::Object(value))
}

fn collection_element_uuid(fields: &[Field], number: u32) -> Option<String> {
    let element = nested_field(fields, number)?;
    uuid(&element, 1)
}

fn varint_fields<'a>(fields: &'a [Field], number: u32) -> impl Iterator<Item = u64> + 'a {
    fields
        .iter()
        .filter_map(move |field| {
            if field.number != number {
                return None;
            }
            match &field.value {
                WireValue::Varint(value) => Some(vec![*value]),
                // Proto3 repeated enums are commonly encoded as packed
                // length-delimited varints, while older files may use the
                // unpacked form handled above.
                WireValue::Bytes(value) => Some(packed_varints(value)),
                _ => None,
            }
        })
        .flatten()
}

fn packed_varints(bytes: &[u8]) -> Vec<u64> {
    let mut values = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let Ok(value) = read_varint(bytes, &mut index) else {
            return Vec::new();
        };
        values.push(value);
    }
    values
}

fn key_mapping_modifier(value: u64) -> Option<&'static str> {
    match value {
        0 => Some("Meta"),
        1 => Some("Shift"),
        2 => Some("Alt"),
        3 => Some("Ctrl"),
        _ => None,
    }
}

/// Decode the portable communication-device records used by ProPresenter's
/// cloud communication document. Hardware fields are also read when present
/// so newer/local documents can retain the same renderer contract.
pub fn decode_communications(bytes: &[u8]) -> Option<Vec<Value>> {
    let fields = decode_message(bytes).ok()?;
    let devices = nested_fields(&fields, 1)
        .filter_map(decode_communication_device)
        .collect::<Vec<_>>();
    (!devices.is_empty()).then_some(devices)
}

fn decode_communication_device(fields: Vec<Field>) -> Option<Value> {
    let id = uuid(&fields, 1)?;
    let mut value = Map::new();
    value.insert("id".to_string(), json!(&id));
    value.insert(
        "title".to_string(),
        json!(string_field(&fields, 2)
            .filter(|value| !value.is_empty())
            .unwrap_or(id)),
    );
    if let Some(transport) = communication_transport(&fields) {
        value.insert("transport".to_string(), json!(transport));
    }
    if let Some(serial) = nested_field(&fields, 7) {
        // Proto3 omitted enum fields mean zero; unsupported handshakes stay explicit.
        let speed = varint_field(&serial, 3).unwrap_or(0);
        let baud_rate = match speed {
            0 => 1200,
            1 => 1800,
            2 => 2400,
            3 => 4800,
            4 => 7200,
            5 => 9600,
            7 => 14400,
            8 => 19200,
            9 => 28800,
            10 => 38400,
            11 => 57600,
            12 => 62500,
            13 => 76800,
            14 => 115200,
            15 => 230400,
            _ => 0,
        };
        let parity = match varint_field(&serial, 5).unwrap_or(0) {
            0 => "none".to_string(),
            1 => "odd".into(),
            2 => "even".into(),
            other => format!("unsupported-{other}"),
        };
        let input = varint_field(&serial, 7).unwrap_or(0);
        let output = varint_field(&serial, 8).unwrap_or(0);
        let flow_control = match (input, output) {
            (0, 0) => "none".to_string(),
            (1, 1) => "hardware".into(),
            _ => format!("unsupported-{input}-{output}"),
        };
        value.insert(
            "serial".into(),
            json!({
                "path":string_field(&serial, 1).unwrap_or_default(),
                "baudRate":baud_rate,
            "dataBits":match varint_field(&serial, 4).unwrap_or(0) { value @ 0..=3 => value + 5, _ => 0 },
                "parity":parity,
            "stopBits":match varint_field(&serial, 6).unwrap_or(0) { value @ 0..=1 => value + 1, _ => 0 },
                "flowControl":flow_control,
            }),
        );
    }
    if let Some(network) = nested_field(&fields, 8).or_else(|| nested_field(&fields, 9)) {
        if let Some(address) = string_field(&network, 3).filter(|value| !value.is_empty()) {
            value.insert("address".to_string(), json!(address));
        }
        if let Some(port) = varint_field(&network, 4).filter(|value| *value > 0) {
            value.insert("port".to_string(), json!(port));
        }
    }
    if bool_field(&fields, 5) == Some(true) {
        value.insert("connected".to_string(), json!(true));
    }
    if let Some(device_type) = varint_field(&fields, 3).and_then(communication_device_type) {
        value.insert("deviceType".to_string(), json!(device_type));
    }
    Some(Value::Object(value))
}

fn communication_transport(fields: &[Field]) -> Option<&'static str> {
    if nested_field(fields, 8).is_some() {
        Some("tcp")
    } else if nested_field(fields, 9).is_some() {
        Some("udp")
    } else if nested_field(fields, 11).is_some() {
        Some("midi")
    } else if nested_field(fields, 7).is_some() {
        Some("serial")
    } else {
        None
    }
}

fn communication_device_type(value: u64) -> Option<&'static str> {
    Some(match value {
        0 => "DEVICE_TYPE_DMX",
        1 => "DEVICE_TYPE_MIDI",
        2 => "DEVICE_TYPE_AMP",
        3 => "DEVICE_TYPE_CITP",
        4 => "DEVICE_TYPE_GLOBAL_CACHE",
        5 => "DEVICE_TYPE_GVG100",
        6 => "DEVICE_TYPE_ROSSTALK",
        7 => "DEVICE_TYPE_SONY_BVS",
        8 => "DEVICE_TYPE_SONY_BVW",
        9 => "DEVICE_TYPE_VDCP",
        10 => "DEVICE_TYPE_ALL_AMERICAN_8000",
        11 => "DEVICE_TYPE_ALL_AMERICAN_9000",
        12 => "DEVICE_TYPE_DAKTRONICS",
        13 => "DEVICE_TYPE_DAKTRONICS_TV",
        14 => "DEVICE_TYPE_ELECTRO_MECH",
        15 => "DEVICE_TYPE_EVERSAN",
        16 => "DEVICE_TYPE_FAIR_PLAY_MP70",
        17 => "DEVICE_TYPE_HARRIS",
        18 => "DEVICE_TYPE_JUGS",
        19 => "DEVICE_TYPE_OES",
        20 => "DEVICE_TYPE_SCOREBIRD",
        21 => "DEVICE_TYPE_SCOREBOT",
        22 => "DEVICE_TYPE_STALKER",
        23 => "DEVICE_TYPE_STAT_CREW",
        _ => return None,
    })
}

/// Decode the ProPresenter Workspace document that contains output screens,
/// audience looks, input devices, stage mappings and recording presets.
///
/// This intentionally returns the same portable shape consumed by the Orbit
/// workspace as the previous importer. Unknown protobuf fields remain
/// ignored, which keeps imports from newer ProPresenter versions resilient.
pub fn decode_workspace_config(
    bytes: &[u8],
    warnings: &mut Vec<String>,
) -> Result<Option<Value>, String> {
    let fields = decode_message(bytes)?;
    if !fields.iter().any(|field| matches!(field.number, 1..=11)) {
        return Ok(None);
    }

    let mut screen_ids = BTreeMap::new();
    let mut screen_config = Map::new();
    for (index, screen) in nested_fields(&fields, 1).enumerate() {
        let source_id = uuid(&screen, 7).unwrap_or_else(|| format!("screen-source-{}", index + 1));
        let is_stage = varint_field(&screen, 2) == Some(2);
        let id = format!(
            "{}-{}",
            if is_stage { "stage" } else { "audience" },
            index + 1
        );
        screen_ids.insert(source_id.clone(), id.clone());
        let (arrangement, arrangement_fields, screens_field) =
            if let Some(value) = nested_field(&screen, 3) {
                ("single", value, 1)
            } else if let Some(value) = nested_field(&screen, 4) {
                ("combined", value, 1)
            } else if let Some(value) = nested_field(&screen, 5) {
                ("edge-blend", value, 2)
            } else {
                ("single", Vec::new(), 1)
            };
        let output_screens = nested_fields(&arrangement_fields, screens_field)
            .map(decode_output_screen)
            .collect::<Vec<_>>();
        let first_display = output_screens
            .first()
            .and_then(|screen| screen.get("output").and_then(Value::as_object));
        let mode = first_display
            .and_then(|display| display.get("mode"))
            .and_then(Value::as_object);
        let display_type = first_display
            .and_then(|display| display.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let display_name = first_display
            .and_then(|display| display.get("name"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("");
        let label = string_field(&screen, 1).unwrap_or_else(|| id.clone());
        let mut config = Map::new();
        config.insert(
            "type".to_string(),
            json!(if is_stage { "stage" } else { "audience" }),
        );
        config.insert("label".to_string(), json!(label));
        // `rendering_enabled` only exists from 21.4. Screens without it import disabled on
        // purpose, so an import never opens outputs on displays this machine may not have.
        config.insert(
            "enabled".to_string(),
            json!(bool_field(&screen, 9).unwrap_or(false)),
        );
        config.insert(
            "targetType".to_string(),
            json!(if display_type.contains("NDI") {
                "ndi"
            } else {
                "display"
            }),
        );
        config.insert(
            "ndiName".to_string(),
            json!(if display_name.is_empty() {
                format!(
                    "Presenter Orbit {}",
                    string_field(&screen, 1).unwrap_or(id.clone())
                )
            } else {
                display_name.to_string()
            }),
        );
        config.insert(
            "ndiWidth".to_string(),
            json!(mode
                .and_then(|value| value.get("width"))
                .and_then(Value::as_u64)
                .unwrap_or(1920)),
        );
        config.insert(
            "ndiHeight".to_string(),
            json!(mode
                .and_then(|value| value.get("height"))
                .and_then(Value::as_u64)
                .unwrap_or(1080)),
        );
        config.insert(
            "ndiFramerate".to_string(),
            json!(mode
                .and_then(|value| value.get("refreshRate"))
                .and_then(Value::as_f64)
                .unwrap_or(60.0)),
        );
        config.insert(
            "videoDelay".to_string(),
            json!(first_display
                .and_then(|display| display.get("videoDelay"))
                .and_then(Value::as_i64)
                .unwrap_or(0)),
        );
        config.insert("sourceScreenId".to_string(), json!(source_id));
        config.insert("arrangement".to_string(), json!(arrangement));
        config.insert("outputScreens".to_string(), json!(output_screens));
        match screen_config::decode(&screen, &arrangement_fields, arrangement) {
            Ok(Some(layout)) => {
                config.insert(
                    "canvasSize".to_string(),
                    serde_json::to_value(layout.canvas()?).map_err(|error| error.to_string())?,
                );
                config.insert(
                    "outputLayout".to_string(),
                    serde_json::to_value(layout).map_err(|error| error.to_string())?,
                );
            }
            Ok(None) => {}
            Err(error) => {
                config.insert("enabled".into(), json!(false));
                warnings.push(format!(
                    "Screen '{label}' was imported disabled without its hardware layout: {error}"
                ));
            }
        }
        config.insert(
            "lookLayers".to_string(),
            json!({
                "music": true, "messages": true, "props": true,
                "announcements": true, "slide": true, "media": true,
                "videoInput": true
            }),
        );
        config.insert(
            "lookThemes".to_string(),
            json!({ "maskId": "", "presentationThemeId": "" }),
        );
        screen_config.insert(id, Value::Object(config));
    }

    if let Some(live_look) = nested_field(&fields, 3) {
        for screen_look in nested_fields(&live_look, 3) {
            let Some(source_id) = uuid(&screen_look, 1) else {
                continue;
            };
            let Some(id) = screen_ids.get(&source_id) else {
                continue;
            };
            let Some(config) = screen_config.get_mut(id).and_then(Value::as_object_mut) else {
                continue;
            };
            config.insert(
                "lookLayers".to_string(),
                json!({
                    "music": true,
                    "props": bool_field(&screen_look, 10).unwrap_or(true)
                        && bool_field(&screen_look, 2).unwrap_or(true),
                    "media": bool_field(&screen_look, 4).unwrap_or(true),
                    "videoInput": bool_field(&screen_look, 3).unwrap_or(true),
                    "slide": bool_field(&screen_look, 7).unwrap_or(true),
                    "messages": bool_field(&screen_look, 11).unwrap_or(true),
                    "announcements": bool_field(&screen_look, 9).unwrap_or(true)
                }),
            );
            config.insert(
                "lookThemes".to_string(),
                json!({ "maskId": uuid(&screen_look, 8).unwrap_or_default(), "presentationThemeId": "" }),
            );
            let mut source = Map::new();
            if let Some(path) = decode_url_value(&screen_look, 5) {
                source.insert(
                    "templateDocumentPath".to_string(),
                    json!({ "absoluteString": path }),
                );
            }
            if let Some(slide_id) = uuid(&screen_look, 6) {
                source.insert("templateSlideId".to_string(), json!(slide_id));
            }
            config.insert("lookSource".to_string(), Value::Object(source));
        }
    }

    let mappings = nested_fields(&fields, 6)
        .filter_map(|entry| {
            let screen = nested_field(&entry, 1)?;
            let layout = nested_field(&entry, 2)?;
            let screen_source_id = collection_element_id(&screen)?;
            let layout_id = collection_element_id(&layout)?;
            Some(json!({
                "screenId": screen_ids.get(&screen_source_id).cloned().unwrap_or(screen_source_id),
                "layoutId": layout_id
            }))
        })
        .collect::<Vec<_>>();

    let audience_looks = nested_fields(&fields, 2)
        .filter_map(|look| {
            let id = uuid(&look, 1)?;
            let title = string_field(&look, 2).unwrap_or_default();
            if title.is_empty() {
                return None;
            }
            let screen_looks = nested_fields(&look, 3)
                .filter_map(|screen_look| {
                    let source_id = uuid(&screen_look, 1)?;
                    let screen_id = screen_ids.get(&source_id).cloned().unwrap_or(source_id);
                    let mut value = Map::new();
                    value.insert("screenId".to_string(), json!(screen_id));
                    value.insert(
                        "lookLayers".to_string(),
                        json!({
                            "props": bool_field(&screen_look, 10).unwrap_or(true)
                                && bool_field(&screen_look, 2).unwrap_or(true),
                            "media": bool_field(&screen_look, 4).unwrap_or(true),
                            "videoInput": bool_field(&screen_look, 3).unwrap_or(true),
                            "slide": bool_field(&screen_look, 7).unwrap_or(true),
                            "messages": bool_field(&screen_look, 11).unwrap_or(true),
                            "announcements": bool_field(&screen_look, 9).unwrap_or(true)
                        }),
                    );
                    if let Some(mask_id) = uuid(&screen_look, 8) {
                        value.insert("maskId".to_string(), json!(mask_id));
                    }
                    Some(Value::Object(value))
                })
                .collect::<Vec<_>>();
            Some(json!({ "id": id, "title": title, "screenLooks": screen_looks }))
        })
        .collect::<Vec<_>>();

    let video_inputs = nested_fields(&fields, 5)
        .enumerate()
        .map(|(index, input)| {
            let device = nested_field(&input, 3);
            json!({
                "id": uuid(&input, 1).unwrap_or_else(|| format!("video-input-{}", index + 1)),
                "title": string_field(&input, 2).unwrap_or_else(|| format!("Video Input {}", index + 1)),
                "deviceName": device.as_ref().and_then(|value| string_field(value, 2).or_else(|| string_field(value, 3))).unwrap_or_default(),
                "audioType": audio_device_type(varint_field(&input, 8).unwrap_or(0)),
                "alphaType": alpha_type(varint_field(&input, 9).unwrap_or(0))
            })
        })
        .collect::<Vec<_>>();

    let audio_inputs = nested_fields(&fields, 11)
        .enumerate()
        .map(|(index, input)| {
            let device_name = nested_field(&input, 3)
                .or_else(|| nested_field(&input, 4))
                .and_then(|value| string_field(&value, 1).or_else(|| string_field(&value, 2)))
                .unwrap_or_default();
            let behavior = nested_field(&input, 5);
            let behavior_mode = behavior
                .as_ref()
                .map(|value| {
                    if nested_message_present(value, 3) {
                        "auto-on"
                    } else if nested_message_present(value, 4) {
                        "auto-off"
                    } else if nested_message_present(value, 1) {
                        "on"
                    } else {
                        "off"
                    }
                })
                .unwrap_or("off");
            json!({
                "id": uuid(&input, 1).unwrap_or_else(|| format!("audio-input-{}", index + 1)),
                "title": string_field(&input, 2).unwrap_or_else(|| format!("Audio Input {}", index + 1)),
                "behaviorMode": behavior_mode,
                "deviceName": device_name
            })
        })
        .collect::<Vec<_>>();

    let recording_presets = nested_field(&fields, 9)
        .map(|settings| {
            let active = nested_field(&settings, 3);
            let active_id = active.as_ref().and_then(|preset| uuid(preset, 1));
            let mut presets = nested_fields(&settings, 2).map(|preset| (preset, false)).collect::<Vec<_>>();
            if let Some(active) = active {
                if let Some((_, selected)) = presets.iter_mut().find(|(preset, _)| active_id.is_some() && uuid(preset, 1) == active_id) {
                    *selected = true;
                } else {
                    presets.push((active, true));
                }
            }
            presets.into_iter()
                .enumerate()
                .map(|(index, (preset, active))| {
                    let streams = nested_fields(&preset, 3)
                        .map(|stream| {
                            let encoder = nested_field(&stream, 3);
                            let mut stream_value = json!({
                                "id": uuid(&stream, 1).unwrap_or_default(),
                                "width": encoder.as_ref().and_then(|value| varint_field(value, 2)).unwrap_or(0),
                                "height": encoder.as_ref().and_then(|value| varint_field(value, 3)).unwrap_or(0),
                                "codec": encoder.as_ref().and_then(|value| varint_field(value, 1)).map(recording_codec).unwrap_or("")
                            });
                            // `video_bitrate` is in bit/s (2500000 in an authentic 2500 Kbit/s preset).
                            if let Some(bitrate) = encoder.as_ref().and_then(|value| varint_field(value, 6)).filter(|bitrate| *bitrate > 0) {
                                stream_value["bitrateKbps"] = json!(bitrate / 1000);
                            }
                            if let Some(fps) = encoder.as_ref().and_then(|value| varint_field(value, 5)).and_then(recording_frame_rate) {
                                stream_value["fps"] = json!(fps);
                            }
                            if let Some(screen) = nested_field(&stream, 2)
                                .and_then(|source| uuid(&source, 1))
                                .and_then(|source| screen_ids.get(&source))
                            {
                                stream_value["sourceScreenId"] = json!(screen);
                            }
                            if bool_field(&stream, 6) == Some(true) {
                                stream_value["audioMap"] = recording_audio_map(&stream);
                            }
                            stream_value
                        })
                        .collect::<Vec<_>>();
                    json!({
                        "id": uuid(&preset, 1).unwrap_or_else(|| format!("recording-preset-{}", index + 1)),
                        "title": string_field(&preset, 2).unwrap_or_else(|| format!("Recording Preset {}", index + 1)),
                        "streams": streams,
                        "active": active
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(Some(json!({
        "screenConfig": Value::Object(screen_config),
        "stageScreenAssignments": mappings,
        "audienceLooks": audience_looks,
        "videoInputs": video_inputs,
        "audioInputs": audio_inputs,
        "recordingPresets": recording_presets
    })))
}

fn collection_element_id(fields: &[Field]) -> Option<String> {
    uuid(fields, 1).or_else(|| string_field(fields, 2))
}

fn nested_message_present(fields: &[Field], number: u32) -> bool {
    fields.iter().any(|field| {
        field.number == number
            && matches!(&field.value, WireValue::Bytes(value) if decode_message(value).is_ok())
    })
}

fn decode_output_screen(fields: Vec<Field>) -> Value {
    let mut value = Map::new();
    value.insert(
        "id".to_string(),
        json!(uuid(&fields, 1).unwrap_or_default()),
    );
    value.insert(
        "label".to_string(),
        json!(string_field(&fields, 2).unwrap_or_default()),
    );
    if let Some(bounds) = nested_field(&fields, 4) {
        value.insert("bounds".to_string(), decode_rect(&bounds));
    }
    if let Some(display) = nested_field(&fields, 14) {
        value.insert("output".to_string(), decode_output_display(&display));
    }
    Value::Object(value)
}

fn decode_rect(fields: &[Field]) -> Value {
    let mut value = Map::new();
    if let Some(origin) = nested_field(fields, 1) {
        value.insert(
            "origin".to_string(),
            json!({
                "x": f64_field(&origin, 1).unwrap_or(0.0),
                "y": f64_field(&origin, 2).unwrap_or(0.0)
            }),
        );
    }
    if let Some(size) = nested_field(fields, 2) {
        value.insert(
            "size".to_string(),
            json!({
                "width": f64_field(&size, 1).unwrap_or(0.0),
                "height": f64_field(&size, 2).unwrap_or(0.0)
            }),
        );
    }
    Value::Object(value)
}

fn decode_output_display(fields: &[Field]) -> Value {
    let mut value = Map::new();
    for (field, key) in [
        (1, "name"),
        (2, "model"),
        (3, "serial"),
        (4, "deviceName"),
        (5, "vendor"),
        (10, "renderId"),
    ] {
        if let Some(text) = string_field(fields, field) {
            value.insert(key.to_string(), json!(text));
        }
    }
    if let Some(kind) = varint_field(fields, 8) {
        value.insert("type".to_string(), json!(output_display_type(kind)));
    }
    if let Some(mode) = nested_field(fields, 9) {
        value.insert(
            "mode".to_string(),
            json!({
                "name": string_field(&mode, 1).unwrap_or_default(),
                "width": varint_field(&mode, 2).unwrap_or(0),
                "height": varint_field(&mode, 3).unwrap_or(0),
                "refreshRate": f64_field(&mode, 4).unwrap_or(0.0),
                "interlaced": bool_field(&mode, 5).unwrap_or(false)
            }),
        );
    }
    if let Some(delay) = varint_field(fields, 12) {
        value.insert("videoDelay".to_string(), json!(delay as i64));
    }
    Value::Object(value)
}

fn output_display_type(value: u64) -> &'static str {
    match value {
        1 => "TYPE_SCREEN",
        2 => "TYPE_CARD",
        3 => "TYPE_NDI",
        4 => "TYPE_SYPHON",
        5 => "TYPE_CUSTOM",
        _ => "TYPE_UNKNOWN",
    }
}

fn audio_device_type(value: u64) -> &'static str {
    match value {
        1 => "AUDIO_DEVICE_TYPE_NONE",
        2 => "AUDIO_DEVICE_TYPE_ALTERNATE",
        _ => "AUDIO_DEVICE_TYPE_DEFAULT",
    }
}

fn alpha_type(value: u64) -> &'static str {
    match value {
        1 => "ALPHA_TYPE_STRAIGHT",
        2 => "ALPHA_TYPE_PREMULTIPLIED",
        _ => "ALPHA_TYPE_UNKNOWN",
    }
}

/// `Recording.Stream.audio_map` as per recording channel (by 0-based `channel_index`) the
/// 1-based ProPresenter channels mapped into it; unlisted recording channels stay silent.
fn recording_audio_map(stream: &[Field]) -> Value {
    let mut channels: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for map in nested_fields(stream, 5) {
        let sources = channels
            .entry(varint_field(&map, 1).unwrap_or(0))
            .or_default();
        for source in varint_fields(&map, 2) {
            if !sources.contains(&(source + 1)) {
                sources.push(source + 1);
            }
        }
    }
    let count = channels.keys().next_back().map_or(0, |last| last + 1);
    Value::Array(
        (0..count)
            .map(|channel| json!(channels.get(&channel).cloned().unwrap_or_default()))
            .collect(),
    )
}

/// Whole frame rates of `Recording.Stream.Encoder.FrameRate`; 29.97 and 59.94 have none.
fn recording_frame_rate(value: u64) -> Option<u32> {
    match value {
        1 => Some(24),
        2 => Some(25),
        4 => Some(30),
        5 => Some(50),
        7 => Some(60),
        _ => None,
    }
}

fn recording_codec(value: u64) -> &'static str {
    match value {
        1 => "CODEC_H264",
        2 => "CODEC_H265",
        3 => "CODEC_PRORES_422_PROXY",
        4 => "CODEC_PRORES_422_LT",
        5 => "CODEC_PRORES_422",
        6 => "CODEC_PRORES_422_HQ",
        7 => "CODEC_PRORES_4444",
        8 => "CODEC_PRORES_4444_XQ",
        9 => "CODEC_HAP",
        10 => "CODEC_HAP_ALPHA",
        11 => "CODEC_HAP_Q",
        12 => "CODEC_HAP_Q_ALPHA",
        13 => "CODEC_NOTCH",
        14 => "CODEC_H264_SOFTWARE",
        15 => "CODEC_H265_SOFTWARE",
        _ => "CODEC_AUTOMATIC",
    }
}

pub fn decode_messages(bytes: &[u8]) -> Option<Vec<Value>> {
    let fields = decode_message(bytes).ok()?;
    let messages = nested_fields(&fields, 2)
        .filter_map(|message| {
            let id = uuid(&message, 1)?;
            let title = string_field(&message, 2).filter(|value| !value.trim().is_empty())?;
            let mut value = Map::new();
            value.insert("id".to_string(), json!(id));
            value.insert("title".to_string(), json!(title));
            value.insert(
                "text".to_string(),
                json!(string_field(&message, 10).unwrap_or_default()),
            );
            if let Some(seconds) = f64_field(&message, 3).filter(|value| *value > 0.0) {
                value.insert("clearAfterSeconds".to_string(), json!(seconds));
            }
            if bool_field(&message, 4) == Some(true) {
                value.insert("visibleOnNetwork".to_string(), json!(true));
            }
            if let Some(clear_type) = varint_field(&message, 9) {
                value.insert(
                    "clearType".to_string(),
                    json!(match clear_type {
                        1 => "after-time",
                        2 => "after-timers",
                        _ => "manual",
                    }),
                );
            }
            let tokens = nested_fields(&message, 12)
                .filter_map(|token| {
                    let name = string_field(&token, 5)?;
                    let text = nested_field(&token, 2).and_then(|value| string_field(&value, 1));
                    Some((name, text.unwrap_or_default()))
                })
                .collect::<BTreeMap<_, _>>();
            if !tokens.is_empty() {
                value.insert("tokenValues".to_string(), json!(tokens));
            }
            if let Some(template) = nested_field(&message, 6) {
                if let Some(template_id) = uuid(&template, 1) {
                    value.insert("templateId".to_string(), json!(template_id));
                }
                if let Some(slide_id) = uuid(&template, 3) {
                    value.insert("templateSlideId".to_string(), json!(slide_id));
                }
            }
            let tokens = nested_fields(&message, 11)
                .filter_map(decode_message_token)
                .collect::<Vec<_>>();
            if !tokens.is_empty() {
                value.insert("tokens".to_string(), json!(tokens));
            }
            Some(Value::Object(value))
        })
        .collect::<Vec<_>>();
    // A document that only carries ProPresenter's application header has no messages yet.
    let header_only =
        fields.iter().any(|field| field.number == 1) && nested_fields(&fields, 2).next().is_none();
    (!messages.is_empty() || header_only).then_some(messages)
}

fn decode_message_token(token: Vec<Field>) -> Option<Value> {
    let id = uuid(&token, 1)?;
    if let Some(text) = nested_field(&token, 2) {
        let name = string_field(&text, 1).filter(|value| !value.trim().is_empty())?;
        return Some(json!({ "id": id, "name": name, "type": "text" }));
    }
    if let Some(timer) = nested_field(&token, 3) {
        let name = string_field(&timer, 1).filter(|value| !value.trim().is_empty())?;
        let mut value = json!({ "id": id, "name": name, "type": "timer" });
        if let Some(timer_id) = uuid(&timer, 2) {
            value["timerId"] = json!(timer_id);
        }
        return Some(value);
    }
    if nested_message_present(&token, 4) {
        return None;
    }
    None
}

/// Messages reference their template by theme slide UUID and name, as ProPresenter writes them.
pub fn encode_messages(messages: &[Value], themes: &[Value]) -> Vec<u8> {
    let mut document = Vec::new();
    for message in messages {
        let mut value = Vec::new();
        put_uuid(
            1,
            message.get("id").and_then(Value::as_str).unwrap_or(""),
            &mut value,
        );
        put_string(
            2,
            message
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Message"),
            &mut value,
        );
        if let Some(seconds) = message.get("clearAfterSeconds").and_then(Value::as_f64) {
            put_f64(3, seconds, &mut value);
        }
        if let Some(visible) = message.get("visibleOnNetwork").and_then(Value::as_bool) {
            put_varint(4, u64::from(visible), &mut value);
        }
        let clear_type = match message.get("clearType").and_then(Value::as_str) {
            Some("after-time") => 1,
            Some("after-timers") => 2,
            _ => 0,
        };
        if let Some(slide) = message_theme_slide(message, themes) {
            let mut template = Vec::new();
            put_uuid(3, &exported_slide_id(slide, 0), &mut template);
            if let Some(name) = slide.get("name").and_then(Value::as_str) {
                put_string(4, name, &mut template);
            }
            put_message(6, template, &mut value);
        }
        put_varint(9, clear_type, &mut value);
        put_string(
            10,
            message.get("text").and_then(Value::as_str).unwrap_or(""),
            &mut value,
        );
        if let Some(tokens) = message.get("tokenValues").and_then(Value::as_object) {
            for (name, text) in tokens {
                let mut token = Vec::new();
                put_string(5, name, &mut token);
                let mut token_text = Vec::new();
                put_string(1, text.as_str().unwrap_or_default(), &mut token_text);
                put_message(2, token_text, &mut token);
                put_message(12, token, &mut value);
            }
        }
        put_message(2, value, &mut document);
    }
    document
}

fn message_theme_slide<'a>(message: &Value, themes: &'a [Value]) -> Option<&'a Value> {
    let theme_id = message.get("themeId").and_then(Value::as_str)?;
    let slides = themes
        .iter()
        .find(|theme| theme.get("id").and_then(Value::as_str) == Some(theme_id))?
        .get("slides")?
        .as_array()?;
    match message.get("themeSlideId").and_then(Value::as_str) {
        Some(slide_id) => slides
            .iter()
            .find(|slide| slide.get("id").and_then(Value::as_str) == Some(slide_id)),
        None => slides.first(),
    }
}

/// Imported slides keep their ProPresenter UUID so exported references stay resolvable.
fn exported_slide_id(slide: &Value, index: usize) -> String {
    slide
        .get("sourceSlideId")
        .and_then(Value::as_str)
        .map_or_else(|| format!("slide-{}", index + 1), str::to_string)
}

pub fn decode_macro_collections(bytes: &[u8]) -> Option<Vec<Value>> {
    let fields = decode_message(bytes).ok()?;
    nested_fields(&fields, 3).map(|collection| {
        let mut order = nested_fields(&collection, 3).map(|item| uuid(&item, 1)).collect::<Option<Vec<_>>>()?;
        for id in nested_fields(&collection, 4).filter_map(|macro_value| uuid(&macro_value, 1)) {
            if !order.contains(&id) {
                order.push(id);
            }
        }
        Some(json!({"id": uuid(&collection, 1)?, "title": string_field(&collection, 2)?, "macroOrder": order}))
    }).collect()
}

pub fn decode_macros(bytes: &[u8]) -> Option<Vec<Value>> {
    let fields = decode_message(bytes).ok()?;
    let mut collection_ids = BTreeMap::<String, Vec<String>>::new();
    // ProPresenter 19 added the macros themselves to `MacroCollection.macros` (field 4).
    let mut embedded = Vec::new();
    for collection in nested_fields(&fields, 3) {
        let Some(collection_id) = uuid(&collection, 1) else {
            continue;
        };
        let members = nested_fields(&collection, 3)
            .filter_map(|item| uuid(&item, 1))
            .chain(nested_fields(&collection, 4).filter_map(|macro_value| uuid(&macro_value, 1)));
        for macro_id in members {
            let ids = collection_ids.entry(macro_id).or_default();
            if !ids.contains(&collection_id) {
                ids.push(collection_id.clone());
            }
        }
        embedded.extend(nested_fields(&collection, 4));
    }
    let mut macros = Vec::new();
    let mut seen = Vec::new();
    for macro_value in nested_fields(&fields, 2).chain(embedded) {
        let Some(id) = uuid(&macro_value, 1) else {
            continue;
        };
        if seen.contains(&id) {
            continue;
        }
        seen.push(id.clone());
        let Some(title) = string_field(&macro_value, 2).filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let mut value = Map::new();
        value.insert("id".to_string(), json!(id.clone()));
        value.insert("title".to_string(), json!(title));
        if let Some(color) = nested_field(&macro_value, 3).and_then(|value| color(&value)) {
            value.insert("color".to_string(), json!(color));
        }
        let actions = nested_fields(&macro_value, 4)
            .enumerate()
            .map(|(index, action)| decode_action(&action, index))
            .collect::<Vec<_>>();
        if !actions.is_empty() {
            value.insert("actions".to_string(), Value::Array(actions));
        }
        if let Some(trigger) = bool_field(&macro_value, 5).filter(|value| *value) {
            value.insert("triggerOnStartup".to_string(), json!(trigger));
        }
        if let Some(image_type) = varint_field(&macro_value, 6) {
            value.insert("imageType".to_string(), json!(macro_image_type(image_type)));
        }
        if let Some(image_data) = bytes_field(&macro_value, 7) {
            value.insert("imageData".to_string(), json!(base64_encode(image_data)));
        }
        if let Some(ids) = collection_ids.get(&id).filter(|ids| !ids.is_empty()) {
            value.insert("collectionIds".to_string(), json!(ids));
        }
        macros.push(Value::Object(value));
    }
    (!macros.is_empty()).then_some(macros)
}

fn macro_image_type(value: u64) -> String {
    const TYPES: &[&str] = &[
        "ImageTypeDefault",
        "ImageTypeOne",
        "ImageTypeTwo",
        "ImageTypeThree",
        "ImageTypeFour",
        "ImageTypeFive",
        "ImageTypeSix",
        "ImageTypeSeven",
        "ImageTypeEight",
        "ImageTypeNine",
        "ImageTypeZero",
        "ImageTypeArrow",
        "ImageTypeAudio",
        "ImageTypeBell",
        "ImageTypeBulb",
        "ImageTypeCloud",
        "ImageTypeCupcake",
        "ImageTypeExclamation",
        "ImageTypeFlask",
        "ImageTypeFlower",
        "ImageTypeGlasses",
        "ImageTypeHashtag",
        "ImageTypeHat",
        "ImageTypeHeart",
        "ImageTypeMegaphone",
        "ImageTypeMessage",
        "ImageTypePaperclip",
        "ImageTypePlay",
        "ImageTypeSlide",
        "ImageTypeStar",
        "ImageTypeSun",
        "ImageTypeSunglasses",
        "ImageTypeTarget",
        "ImageTypeTimer",
        "ImageTypeVideoInput",
        "ImageTypeXClear",
        "ImageTypeLetterA",
        "ImageTypeLetterB",
        "ImageTypeLetterC",
        "ImageTypeLetterD",
        "ImageTypeLetterE",
        "ImageTypeLetterF",
        "ImageTypeLetterG",
        "ImageTypeLetterH",
        "ImageTypeLetterI",
        "ImageTypeLetterJ",
        "ImageTypeLetterK",
        "ImageTypeLetterL",
        "ImageTypeLetterM",
        "ImageTypeLetterN",
        "ImageTypeLetterO",
        "ImageTypeLetterP",
        "ImageTypeLetterQ",
        "ImageTypeLetterR",
        "ImageTypeLetterS",
        "ImageTypeLetterT",
        "ImageTypeLetterU",
        "ImageTypeLetterV",
        "ImageTypeLetterW",
        "ImageTypeLetterX",
        "ImageTypeLetterY",
        "ImageTypeLetterZ",
        "ImageTypeCustom",
    ];
    TYPES
        .get(value as usize)
        .copied()
        .unwrap_or("ImageTypeDefault")
        .to_string()
}

pub fn encode_macros(macros: &[Value]) -> Vec<u8> {
    let mut document = Vec::new();
    for macro_value in macros {
        let mut value = Vec::new();
        put_uuid(
            1,
            macro_value.get("id").and_then(Value::as_str).unwrap_or(""),
            &mut value,
        );
        put_string(
            2,
            macro_value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Macro"),
            &mut value,
        );
        if let Some(trigger) = macro_value.get("triggerOnStartup").and_then(Value::as_bool) {
            put_varint(5, u64::from(trigger), &mut value);
        }
        if let Some(image_type) = macro_value.get("imageType").and_then(Value::as_u64) {
            put_varint(6, image_type, &mut value);
        }
        put_message(2, value, &mut document);
    }
    document
}

const CLEAR_GROUP_IMAGE_TYPES: [&str; 33] = [
    "ImageTypeCustom",
    "ImageTypeOne",
    "ImageTypeTwo",
    "ImageTypeThree",
    "ImageTypeFour",
    "ImageTypeFive",
    "ImageTypeSix",
    "ImageTypeSeven",
    "ImageTypeEight",
    "ImageTypeNine",
    "ImageTypeZero",
    "ImageTypeAll",
    "ImageTypeMegahorn",
    "ImageTypePlay",
    "ImageTypeBulb",
    "ImageTypeSunglasses",
    "ImageTypeArrow",
    "ImageTypeTarget",
    "ImageTypeStar",
    "ImageTypeSun",
    "ImageTypeBell",
    "ImageTypePaperclip",
    "ImageTypeFlask",
    "ImageTypeEyeglasses",
    "ImageTypeCupcake",
    "ImageTypeSlide",
    "ImageTypeHat",
    "ImageTypeFlower",
    "ImageTypeHeart",
    "ImageTypeMessage",
    "ImageTypeAudio",
    "ImageTypeCloud",
    "ImageTypeExclamation",
];

pub fn is_clear_group_image_type(name: &str) -> bool {
    CLEAR_GROUP_IMAGE_TYPES.contains(&name)
}

/// Decode Configuration/ClearGroups. Layer targets and icons Orbit cannot
/// represent are returned as findings instead of being mapped to a guess.
pub fn decode_clear_groups(bytes: &[u8]) -> Option<(Vec<Value>, Vec<String>)> {
    let fields = decode_message(bytes).ok()?;
    let mut groups = Vec::new();
    let mut findings = Vec::new();
    for group in nested_fields(&fields, 2) {
        let id = uuid(&group, 1)?;
        let title = string_field(&group, 2).filter(|title| !title.trim().is_empty())?;
        let mut layers = Vec::new();
        for target in nested_fields(&group, 3) {
            let layer = varint_field(&target, 2).unwrap_or(0);
            // Clear > Announcements is encoded as layer ALL with the announcements destination.
            let value = match (layer, varint_field(&target, 4).unwrap_or(0)) {
                (0, 1) => "announcements",
                (1, _) => "audio",
                (2, _) => "background",
                (3, _) => "video-input",
                (4, _) => "prop",
                (5, _) => "slide",
                (7, _) => "messages",
                (8, _) => "audio-effects",
                _ => {
                    findings.push(format!(
                        "clear group {title}: unsupported layer {}",
                        clear_layer_value(layer)
                    ));
                    continue;
                }
            };
            if !layers.contains(&value) {
                layers.push(value);
            }
        }
        if layers.is_empty() {
            findings.push(format!("clear group {title}: no supported layers"));
            continue;
        }
        let timelines = varint_fields(&group, 9).collect::<Vec<_>>();
        let mut value = json!({
            "id": id,
            "title": title,
            "layers": layers,
            "hiddenInPreview": bool_field(&group, 4).unwrap_or(false),
            "stopPresentationTimeline": timelines.contains(&0),
            "stopAnnouncementsTimeline": timelines.contains(&1),
            "clearPresentationNext": bool_field(&group, 10).unwrap_or(false),
        });
        let image_type = varint_field(&group, 6).unwrap_or(0);
        match CLEAR_GROUP_IMAGE_TYPES.get(image_type as usize) {
            Some(&"ImageTypeCustom") => {
                if let Some(data) = bytes_field(&group, 5).filter(|data| !data.is_empty()) {
                    match crate::macro_image::validate(data) {
                        Ok(_) => {
                            value["imageType"] = json!("ImageTypeCustom");
                            value["imageData"] = json!(base64_encode(data));
                        }
                        Err(error) => {
                            findings.push(format!("clear group {title}: custom icon {error}"))
                        }
                    }
                }
            }
            Some(name) => value["imageType"] = json!(name),
            None => findings.push(format!(
                "clear group {title}: unsupported icon type {image_type}"
            )),
        }
        let tinted = bool_field(&group, 7).unwrap_or(false);
        match nested_field(&group, 8).and_then(|tint| color(&tint)) {
            Some(tint) if tint.starts_with('#') => {
                value["iconTinted"] = json!(tinted);
                value["iconTint"] = json!(tint);
            }
            _ if tinted => {
                findings.push(format!("clear group {title}: transparent icon tint"));
            }
            _ => {}
        }
        groups.push(value);
    }
    Some((groups, findings))
}

pub fn encode_clear_groups(groups: &[Value]) -> Result<Vec<u8>, String> {
    let mut document = Vec::new();
    for group in groups {
        let mut value = Vec::new();
        put_uuid(1, group["id"].as_str().unwrap_or_default(), &mut value);
        put_string(2, group["title"].as_str().unwrap_or_default(), &mut value);
        for layer in group["layers"].as_array().into_iter().flatten() {
            let (target_layer, destination) = match layer.as_str() {
                Some("announcements") => (0, 1),
                Some("audio") => (1, 0),
                Some("background") => (2, 0),
                Some("video-input") => (3, 0),
                Some("prop") => (4, 0),
                Some("slide") => (5, 0),
                Some("messages") => (7, 0),
                Some("audio-effects") => (8, 0),
                // Stage messages are an Orbit-only layer; export reports the loss.
                _ => continue,
            };
            let mut target = Vec::new();
            if target_layer != 0 {
                put_varint(2, target_layer, &mut target);
            }
            if destination != 0 {
                put_varint(4, destination, &mut target);
            }
            put_message(3, target, &mut value);
        }
        if group["hiddenInPreview"].as_bool() == Some(true) {
            put_varint(4, 1, &mut value);
        }
        let image_type = group["imageType"].as_str();
        if image_type == Some("ImageTypeCustom") {
            let data = group["imageData"]
                .as_str()
                .ok_or("Custom clear group icon data is required.")?;
            put_bytes(5, &crate::macro_image::decode_data(data)?, &mut value);
        }
        if let Some(index) = image_type
            .and_then(|name| {
                CLEAR_GROUP_IMAGE_TYPES
                    .iter()
                    .position(|entry| *entry == name)
            })
            .filter(|index| *index != 0)
        {
            put_varint(6, index as u64, &mut value);
        }
        if group["iconTinted"].as_bool() == Some(true) {
            put_varint(7, 1, &mut value);
        }
        if let Some(tint) = group["iconTint"].as_str() {
            put_color(8, tint, &mut value);
        }
        let mut timelines = Vec::new();
        if group["stopPresentationTimeline"].as_bool() != Some(false) {
            encode_varint(0, &mut timelines);
        }
        if group["stopAnnouncementsTimeline"].as_bool() != Some(false) {
            encode_varint(1, &mut timelines);
        }
        if !timelines.is_empty() {
            put_bytes(9, &timelines, &mut value);
        }
        if group["clearPresentationNext"].as_bool() == Some(true) {
            put_varint(10, 1, &mut value);
        }
        put_message(2, value, &mut document);
    }
    Ok(document)
}

fn parse_cue(fields: &[Field], index: usize, group: Option<&Map<String, Value>>) -> Value {
    let mut slide = None;
    let actions = nested_fields(fields, 10)
        .enumerate()
        .map(|(action_index, action)| {
            if slide.is_none() && varint_field(&action, 9) == Some(11) {
                slide = presentation_slide(&action);
            }
            decode_action(&action, action_index)
        })
        .collect::<Vec<_>>();
    let mut value = slide.unwrap_or_else(|| json!({ "content": "" }));
    if let Some(object) = value.as_object_mut() {
        object.insert("id".to_string(), json!(format!("slide-{}", index + 1)));
        if let Some(value) = uuid(fields, 1) {
            object.insert("sourceCueId".to_string(), json!(value));
        }
        if let Some(value) = string_field(fields, 2).filter(|value| !is_generated_cue_name(value)) {
            object.insert("name".to_string(), json!(value));
        }
        if let Some(value) = bool_field(fields, 12) {
            object.insert("disabled".to_string(), json!(!value));
        }
        if let Some(value) = f64_field(&nested_field(fields, 7).unwrap_or_default(), 1) {
            object.insert("triggerTimeSeconds".to_string(), json!(value));
        }
        if let Some(value) = f64_field(fields, 13) {
            object.insert("completionTimeSeconds".to_string(), json!(value));
        }
        if let Some(value) = varint_field(fields, 5) {
            let mut unsupported = Vec::new();
            if !unknown_enum("cue completion action", value, 3, &mut unsupported) {
                object.insert(
                    "completionAction".to_string(),
                    json!(completion_action(value)),
                );
            }
            note_unsupported(object, unsupported);
        }
        if let Some(value) = uuid(fields, 4).filter(|value| !is_nil_uuid(value)) {
            object.insert("completionTargetCueId".to_string(), json!(value));
        }
        if let Some(value) = varint_field(fields, 3) {
            let target = match value {
                0 => "NONE",
                1 => "NEXT",
                2 => "RANDOM",
                3 => "CUE",
                4 => "FIRST",
                _ => "UNKNOWN",
            };
            object.insert("completionTargetType".to_string(), json!(target));
        }
        if let Some(value) = uuid(fields, 6).filter(|value| !is_nil_uuid(value)) {
            object.insert("completionActionId".to_string(), json!(value));
        }
        if let Some(value) = group {
            for (key, group_value) in value {
                object.insert(key.clone(), group_value.clone());
            }
        }
        if let Some(notes) = cue_notes(fields) {
            let document = rtf_layer_document(&notes);
            let text: String = document
                .characters
                .iter()
                .map(|(character, _)| character)
                .collect();
            object.insert("notes".to_string(), json!(text));
            let mut paragraph_style = Map::new();
            apply_rtf_paragraph_styles(&mut paragraph_style, &document, &[], &text);
            if !paragraph_style.is_empty() {
                object.insert("notesTextStyle".to_string(), Value::Object(paragraph_style));
            }
            if let Some(runs) = rtf_to_runs(&notes, true) {
                object.insert("notesRuns".to_string(), json!(runs));
            }
        }
        if let Some(transition) = nested_fields(fields, 10).find_map(|action| {
            nested_field(&action, 23)
                .and_then(|slide| nested_field(&slide, 2))
                .and_then(|slide| nested_field(&slide, 5))
        }) {
            insert_transition(object, &transition);
        }
        // ProPresenter keeps the slide label on the slide action; Orbit keeps it on the slide.
        if let Some(details) = actions
            .iter()
            .find(|action| action["type"] == "presentation-slide")
            .and_then(|action| action["details"].as_object())
        {
            for key in ["label", "labelColor"] {
                if let Some(value) = details.get(key) {
                    object.insert(key.to_string(), value.clone());
                }
            }
        }
        if !actions.is_empty() {
            object.insert("actions".to_string(), Value::Array(actions));
        }
    }
    value
}

/// Converts a ProPresenter transition to Orbit's `{effectId, durationSeconds, direction,
/// variables}`. Effects outside ProPresenter's built-in library, or durations Orbit cannot
/// play, are kept by name as `unsupportedTransition` so the import report discloses them.
/// Variables at their library value are omitted; unknown or out-of-range variables are
/// listed as `unsupportedTransitionVariables`.
fn insert_transition(object: &mut Map<String, Value>, fields: &[Field]) {
    let effect = nested_field(fields, 3).unwrap_or_default();
    let render_id = string_field(&effect, 4).unwrap_or_default();
    let name = string_field(&effect, 3).unwrap_or_else(|| render_id.clone());
    let duration = f64_field(fields, 1).unwrap_or(0.0);
    let Some(entry) = crate::native_transitions::catalog_entry(&render_id)
        .filter(|_| duration.is_finite() && (0.0..=10.0).contains(&duration))
    else {
        object.insert("unsupportedTransition".to_string(), json!(name));
        return;
    };
    let mut transition = Map::new();
    transition.insert("effectId".to_string(), json!(entry.id));
    transition.insert("durationSeconds".to_string(), json!(duration));
    let direction = nested_fields(&effect, 7)
        .filter(|variable| {
            string_field(variable, 1).is_some_and(|name| name.eq_ignore_ascii_case("direction"))
        })
        .find_map(|variable| nested_field(&variable, 6))
        .and_then(|direction| varint_field(&direction, 1))
        .and_then(|bitmask| u32::try_from(bitmask).ok())
        .filter(|bitmask| entry.directions & bitmask != 0)
        .and_then(crate::native_transitions::direction_name);
    if let Some(direction) = direction {
        transition.insert("direction".to_string(), json!(direction));
    }
    let mut variables = Map::new();
    let mut unsupported = Vec::new();
    for variable in nested_fields(&effect, 7) {
        let name = string_field(&variable, 1).unwrap_or_default();
        if name.eq_ignore_ascii_case("direction") {
            continue;
        }
        // ProPresenter files name variables by label; descriptors by camel-case name.
        let spec = crate::native_transitions::effect_variables(entry.id).find(|spec| {
            spec.name.eq_ignore_ascii_case(&name) || spec.label.eq_ignore_ascii_case(&name)
        });
        match spec.and_then(|spec| Some((spec, decode_effect_variable(spec, &variable)?))) {
            Some((spec, value)) => {
                if !same_variable_value(value, spec.value) {
                    variables.insert(
                        spec.name.to_string(),
                        crate::native_transitions::variable_json(spec, value),
                    );
                }
            }
            None => unsupported.push(json!(name)),
        }
    }
    if !variables.is_empty() {
        transition.insert("variables".to_string(), Value::Object(variables));
    }
    if !unsupported.is_empty() {
        object.insert(
            "unsupportedTransitionVariables".to_string(),
            Value::Array(unsupported),
        );
    }
    object.insert("transition".to_string(), Value::Object(transition));
}

/// Reads one effect variable as Orbit's value; numbers may be stored as int, float or double.
fn decode_effect_variable(
    spec: &crate::native_transitions::VariableSpec,
    variable: &[Field],
) -> Option<[f64; 3]> {
    let value = match spec.kind {
        crate::native_transitions::VariableKind::Number { min, max } => {
            let number = if let Some(int) = nested_field(variable, 3) {
                varint_field(&int, 1).map(|value| value as i32 as f64)
            } else if let Some(float) = nested_field(variable, 4) {
                f32_field(&float, 1).map(f64::from)
            } else {
                nested_field(variable, 7).and_then(|double| f64_field(&double, 1))
            }
            .unwrap_or(0.0);
            // Floats carry single precision; snap to the range the descriptor allows.
            let tolerance = 1e-5 * (max - min);
            if !number.is_finite() || number < min - tolerance || number > max + tolerance {
                return None;
            }
            [number.clamp(min, max), 0.0, 0.0]
        }
        crate::native_transitions::VariableKind::Color => {
            let color = nested_field(&nested_field(variable, 5)?, 1).unwrap_or_default();
            let channel = |number| f64::from(f32_field(&color, number).unwrap_or(0.0));
            let rgb = [channel(1), channel(2), channel(3)];
            if rgb.iter().any(|value| !(0.0..=1.0).contains(value)) {
                return None;
            }
            rgb
        }
    };
    Some(value)
}

/// Whether two variable values are equal at the precision Orbit stores them.
fn same_variable_value(left: [f64; 3], right: [f64; 3]) -> bool {
    left.iter()
        .zip(right)
        .all(|(left, right)| (left - right).abs() < 0.5 / 255.0)
}

/// Decodes a `Presets/Transition` file (`Transition.Preset`) as an Orbit transition
/// favorite; `None` when the file is not a preset of a library effect.
pub fn decode_transition_preset(bytes: &[u8]) -> Option<Value> {
    let fields = decode_message(bytes).ok()?;
    let id = uuid(&fields, 1).filter(|id| !id.trim().is_empty())?;
    let name = string_field(&fields, 2).filter(|name| !name.trim().is_empty())?;
    let mut object = Map::new();
    insert_transition(&mut object, &nested_field(&fields, 3)?);
    let transition = object.remove("transition")?;
    let mut preset = json!({ "id": id, "name": name, "transition": transition });
    if let Some(names) = object.remove("unsupportedTransitionVariables") {
        preset["unsupportedTransitionVariables"] = names;
    }
    Some(preset)
}

/// Writes an Orbit transition favorite as ProPresenter's `Transition.Preset`.
pub fn encode_transition_preset(preset: &Value) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    put_uuid(1, preset.get("id")?.as_str()?, &mut bytes);
    put_string(2, preset.get("name")?.as_str()?, &mut bytes);
    put_message(3, encode_transition(preset.get("transition")?)?, &mut bytes);
    Some(bytes)
}

/// Writes an Orbit transition as ProPresenter's built-in library effect.
fn encode_transition(value: &Value) -> Option<Vec<u8>> {
    let entry = crate::native_transitions::catalog_entry(value.get("effectId")?.as_str()?)?;
    let mut effect = Vec::new();
    put_varint(2, 1, &mut effect);
    put_string(3, entry.name, &mut effect);
    put_string(4, entry.id, &mut effect);
    put_string(5, entry.name, &mut effect);
    put_string(6, entry.category, &mut effect);
    if let Some(bitmask) = value
        .get("direction")
        .and_then(Value::as_str)
        .and_then(crate::native_transitions::direction_bitmask)
    {
        let mut direction = Vec::new();
        put_varint(1, u64::from(bitmask), &mut direction);
        put_varint(2, u64::from(entry.default_direction), &mut direction);
        put_varint(3, u64::from(entry.directions), &mut direction);
        let mut variable = Vec::new();
        put_string(1, "Direction", &mut variable);
        put_message(6, direction, &mut variable);
        put_message(7, variable, &mut effect);
    }
    for spec in crate::native_transitions::effect_variables(entry.id) {
        let value = value
            .get("variables")
            .and_then(|variables| variables.get(spec.name))
            .and_then(|value| crate::native_transitions::variable_value(spec, value))
            .unwrap_or(spec.value);
        let mut variable = Vec::new();
        put_string(1, spec.label, &mut variable);
        match spec.kind {
            crate::native_transitions::VariableKind::Number { min, max } => {
                let mut number = Vec::new();
                put_f32(1, value[0] as f32, &mut number);
                put_f32(2, spec.default_value[0] as f32, &mut number);
                put_f32(3, min as f32, &mut number);
                put_f32(4, max as f32, &mut number);
                put_message(4, number, &mut variable);
            }
            crate::native_transitions::VariableKind::Color => {
                let color = |rgb: [f64; 3]| {
                    let mut color = Vec::new();
                    for (number, channel) in rgb.into_iter().enumerate() {
                        put_f32(number as u32 + 1, channel as f32, &mut color);
                    }
                    put_f32(4, 1.0, &mut color);
                    color
                };
                let mut effect_color = Vec::new();
                put_message(1, color(value), &mut effect_color);
                put_message(2, color(spec.default_value), &mut effect_color);
                put_message(5, effect_color, &mut variable);
            }
        }
        put_message(7, variable, &mut effect);
    }
    let mut transition = Vec::new();
    put_f64(1, value.get("durationSeconds")?.as_f64()?, &mut transition);
    put_message(3, effect, &mut transition);
    Some(transition)
}

fn is_nil_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|value| value.is_nil())
}

fn completion_action(value: u64) -> &'static str {
    match value {
        1 => "COMPLETION_ACTION_TYPE_LAST",
        2 => "COMPLETION_ACTION_TYPE_AFTER_ACTION",
        3 => "COMPLETION_ACTION_TYPE_AFTER_TIME",
        _ => "COMPLETION_ACTION_TYPE_FIRST",
    }
}

fn is_generated_cue_name(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_digit() || character.is_whitespace())
}

fn cue_notes(fields: &[Field]) -> Option<Vec<u8>> {
    nested_fields(fields, 10).find_map(|action| {
        let presentation = nested_field(&action, 23).and_then(|slide| nested_field(&slide, 2))?;
        let notes = nested_field(&presentation, 2)?;
        bytes_field(&notes, 1).map(<[u8]>::to_vec)
    })
}

fn decode_action(fields: &[Field], index: usize) -> Value {
    let action_type = varint_field(fields, 9).unwrap_or(0);
    let mut value = Map::new();
    value.insert(
        "id".to_string(),
        json!(uuid(fields, 1).unwrap_or_else(|| format!("action-{}", index + 1))),
    );
    if let Some(name) = string_field(fields, 2) {
        value.insert("title".to_string(), json!(name));
    }
    value.insert("type".to_string(), json!(action_contract_type(action_type)));
    if bool_field(fields, 6) == Some(false) {
        value.insert("enabled".to_string(), json!(false));
    }
    if let Some(delay) = f64_field(fields, 4).filter(|value| *value > 0.0) {
        value.insert("delaySeconds".to_string(), json!(delay));
    }
    if let Some(duration) = f64_field(fields, 8).filter(|value| *value > 0.0) {
        value.insert("durationSeconds".to_string(), json!(duration));
    }
    let mut details = Map::new();
    let mut unsupported = Vec::new();
    match action_type {
        25 => {
            if let Some(capture) = nested_field(fields, 43) {
                if let Some(start) = bytes_fields(&capture, 1).next() {
                    let preset_id = decode_message(start).ok().and_then(|start| {
                        match start.iter().find(|field| field.number == 1) {
                            Some(Field {
                                value: WireValue::Bytes(bytes),
                                ..
                            }) => decode_message(bytes).ok().and_then(|identification| {
                                if identification.iter().any(|field| field.number == 1) {
                                    uuid(&identification, 1).map(|id| {
                                        if is_nil_uuid(&id) {
                                            String::new()
                                        } else {
                                            id
                                        }
                                    })
                                } else {
                                    Some(String::new())
                                }
                            }),
                            Some(_) => None,
                            None => Some(String::new()),
                        }
                    });
                    details.insert(
                        "start".into(),
                        json!({
                            "presetIdentification": preset_id
                        }),
                    );
                }
                if let Some(stop) = bytes_fields(&capture, 2).next() {
                    details.insert("stop".into(), json!({
                        "showsAlertBeforeStopping": decode_message(stop).ok().and_then(|stop| match stop.iter().find(|field| field.number == 1) {
                            Some(Field { value: WireValue::Varint(value), .. }) => Some(*value != 0),
                            None => Some(false),
                            Some(_) => None,
                        })
                    }));
                }
            }
        }
        11 => {
            if let Some(slide) = presentation_slide(fields) {
                details.insert("slide".to_string(), slide);
            }
        }
        15 => {
            if let Some(prop) = nested_field(fields, 23).and_then(|value| nested_field(&value, 3)) {
                if let Some(base) = nested_field(&prop, 1) {
                    if let Some(id) = uuid(&base, 7) {
                        value.insert("targetId".to_string(), json!(id));
                    }
                    details.insert("slide".to_string(), decode_base_slide(&base, None));
                }
                if let Some(seconds) = prop_auto_clear_seconds(&prop) {
                    details.insert("durationSeconds".to_string(), json!(seconds));
                }
            }
        }
        2 | 12 | 13 => {
            if let Some(media) = nested_field(fields, 20).or_else(|| nested_field(fields, 24)) {
                decode_media_action(&media, &mut value);
            }
        }
        3 => {
            if let Some(timer) = nested_field(fields, 25) {
                if let Some(id) = nested_field(&timer, 4).and_then(|value| uuid(&value, 1)) {
                    value.insert("targetId".to_string(), json!(id));
                }
                let timer_action = varint_field(&timer, 2).unwrap_or(0);
                // An unknown timer action must not run as "start".
                if unknown_enum("timer action", timer_action, 5, &mut unsupported) {
                    value.insert("type".to_string(), json!("unknown"));
                }
                value.insert("value".to_string(), json!(timer_action_value(timer_action)));
                if let Some(value) = f64_field(&timer, 11) {
                    details.insert("incrementSeconds".to_string(), json!(value));
                }
                if timer.iter().any(|field| field.number == 10) {
                    if let Some(mut configuration) =
                        nested_field(&timer, 10).and_then(|value| timer_configuration(&value))
                    {
                        // Set Configuration replaces the whole definition, so absent optional
                        // fields are explicit removals in the countdown timer patch.
                        for key in [
                            "targetTimeSeconds",
                            "elapsedStartSeconds",
                            "elapsedEndSeconds",
                        ] {
                            configuration.entry(key).or_insert(Value::Null);
                        }
                        details.insert("configuration".to_string(), Value::Object(configuration));
                    } else {
                        value.insert("type".to_string(), json!("unknown"));
                        unsupported.push(
                            "timer configuration is malformed or contains an unsupported timer value"
                                .to_string(),
                        );
                    }
                }
            }
        }
        5 => {
            if let Some(clear) = nested_field(fields, 26) {
                let layer = varint_field(&clear, 2).unwrap_or(0);
                let destination = varint_field(&clear, 4).unwrap_or(0);
                // An unknown layer must not run as "clear all".
                if unknown_enum("clear target layer", layer, 8, &mut unsupported)
                    | unknown_enum("clear destination", destination, 1, &mut unsupported)
                {
                    value.insert("type".to_string(), json!("unknown"));
                } else {
                    // ProPresenter's Clear > Announcement writes layer ALL with the
                    // announcements destination; it must not clear the main output.
                    value.insert(
                        "targetLayer".to_string(),
                        json!(if layer == 0 && destination == 1 {
                            "announcements"
                        } else {
                            clear_layer_value(layer)
                        }),
                    );
                    details.insert(
                        "contentDestination".to_string(),
                        json!(content_destination_value(destination)),
                    );
                }
            }
        }
        6 => {
            if let Some(prop) = nested_field(fields, 28) {
                if let Some(id) = nested_field(&prop, 3).and_then(|value| uuid(&value, 1)) {
                    value.insert("targetId".to_string(), json!(id));
                }
                if let Some(trigger) = nested_field(&prop, 4) {
                    details.insert(
                        "autoClearEnabled".to_string(),
                        json!(bool_field(&trigger, 1).unwrap_or(false)),
                    );
                    details.insert(
                        "autoClearFollowsPropDuration".to_string(),
                        json!(bool_field(&trigger, 2).unwrap_or(false)),
                    );
                    if let Some(seconds) = f64_field(&trigger, 3) {
                        details.insert("autoClearSeconds".to_string(), json!(seconds));
                    }
                }
                value.insert(
                    "value".to_string(),
                    json!(if nested_message_present(&prop, 5) {
                        "clear"
                    } else {
                        "show"
                    }),
                );
            }
        }
        7 => {
            if let Some(mask) = nested_field(fields, 29) {
                if let Some(id) = nested_field(&mask, 3).and_then(|value| uuid(&value, 1)) {
                    value.insert("targetId".to_string(), json!(id));
                }
            }
        }
        8 => {
            if let Some(message) = nested_field(fields, 30) {
                if let Some(id) = nested_field(&message, 1).and_then(|value| uuid(&value, 1)) {
                    value.insert("targetId".to_string(), json!(id));
                }
                let tokens = nested_fields(&message, 3)
                    .filter_map(|token| {
                        Some((
                            string_field(&token, 5)?,
                            nested_field(&token, 2)
                                .and_then(|value| string_field(&value, 1))
                                .unwrap_or_default(),
                        ))
                    })
                    .collect::<BTreeMap<_, _>>();
                if !tokens.is_empty() {
                    details.insert("tokenValues".to_string(), json!(tokens));
                }
            }
        }
        14 => {
            if let Some(document) = nested_field(fields, 34) {
                if let Some(id) = nested_field(&document, 1).and_then(|value| uuid(&value, 1)) {
                    value.insert("targetId".to_string(), json!(id));
                }
                if let Some(id) = uuid(&document, 2) {
                    details.insert("arrangementId".to_string(), json!(id));
                }
                details.insert(
                    "contentDestination".to_string(),
                    json!(varint_field(&document, 3).unwrap_or(0)),
                );
            }
        }
        26 => {
            if let Some(playlist) = nested_field(fields, 17) {
                if let Some(id) = uuid(&playlist, 1) {
                    value.insert("targetId".to_string(), json!(id));
                }
                if let Some(id) = uuid(&playlist, 3) {
                    details.insert("itemId".to_string(), json!(id));
                }
                details.insert(
                    "selectPlaylist".to_string(),
                    json!(bool_field(&playlist, 5).unwrap_or(false)),
                );
                details.insert(
                    "alwaysRetrigger".to_string(),
                    json!(bool_field(&playlist, 6).unwrap_or(false)),
                );
            }
        }
        22 => {
            if let Some(destination) = nested_field(fields, 39) {
                details.insert(
                    "slideTarget".to_string(),
                    json!(varint_field(&destination, 1).unwrap_or(0)),
                );
            }
        }
        23 => {
            if let Some(macro_value) = nested_field(fields, 40) {
                if let Some(id) = nested_field(&macro_value, 1).and_then(|value| uuid(&value, 1)) {
                    value.insert("targetId".to_string(), json!(id));
                }
            }
        }
        24 => {
            if let Some(clear_group) = nested_field(fields, 41) {
                if let Some(id) =
                    nested_field(&clear_group, 1).and_then(|value| collection_element_id(&value))
                {
                    value.insert("targetId".to_string(), json!(id));
                }
            }
        }
        18 | 10 => {
            if let Some(look) = nested_field(fields, 37).or_else(|| nested_field(fields, 33)) {
                if let Some(id) = nested_field(&look, 1).and_then(|value| uuid(&value, 1)) {
                    value.insert("targetId".to_string(), json!(id));
                }
            }
        }
        1 => {
            if let Some(stage) = nested_field(fields, 27) {
                details.insert(
                    "slideTarget".to_string(),
                    json!(varint_field(&stage, 5).unwrap_or(0)),
                );
                details.insert("stageScreenAssignments".to_string(), json!(nested_fields(&stage, 4).map(|assignment| {
                    json!({
                        "screenId": nested_field(&assignment, 1).and_then(|value| uuid(&value, 1)).unwrap_or_default(),
                        "layoutId": nested_field(&assignment, 2).and_then(|value| uuid(&value, 1)).unwrap_or_default()
                    })
                }).collect::<Vec<_>>()));
            }
        }
        17 => {
            if let Some(external) = nested_field(fields, 36) {
                if let Some(url) = decode_url_value(&external, 1) {
                    details.insert("url".to_string(), json!(url));
                }
            }
        }
        19 => {
            if let Some(input) = nested_field(fields, 38) {
                if bool_field(&input, 2) == Some(true) {
                    if let Some(mode) = nested_field(&input, 3) {
                        let mode = if nested_message_present(&mode, 1) {
                            "on"
                        } else if nested_message_present(&mode, 3) {
                            "auto-on"
                        } else if nested_message_present(&mode, 4) {
                            "auto-off"
                        } else {
                            "off"
                        };
                        details.insert("behaviorMode".into(), json!(mode));
                    }
                }
                if bool_field(&input, 4) == Some(true) {
                    if let Some(volume) =
                        f64_field(&input, 5).filter(|volume| volume.is_finite() && *volume >= 0.0)
                    {
                        details.insert(
                            "levelDb".into(),
                            json!(if volume == 0.0 {
                                -60.0
                            } else {
                                20.0 * volume.log10()
                            }),
                        );
                    }
                }
                details.insert(
                    "index".to_string(),
                    json!(varint_field(&input, 1).unwrap_or(0)),
                );
                details.insert(
                    "overrideMode".to_string(),
                    json!(bool_field(&input, 2).unwrap_or(false)),
                );
                details.insert(
                    "overrideVolume".to_string(),
                    json!(bool_field(&input, 4).unwrap_or(false)),
                );
                if let Some(volume) = f64_field(&input, 5) {
                    details.insert("volume".to_string(), json!(volume));
                }
            }
        }
        4 => {
            if let Some(communication) = nested_field(fields, 32) {
                details.insert(
                    "format".to_string(),
                    json!(string_field(&communication, 2).unwrap_or_default()),
                );
                details.insert(
                    "description".to_string(),
                    json!(string_field(&communication, 3).unwrap_or_default()),
                );
            }
        }
        _ => {}
    }
    note_unsupported(&mut details, unsupported);
    if let Some(label) = nested_field(fields, 3) {
        if let Some(text) = string_field(&label, 2) {
            details.insert("label".to_string(), json!(text));
        }
        if let Some(color) = nested_field(&label, 3).and_then(|value| color(&value)) {
            details.insert("labelColor".to_string(), json!(color));
        }
    }
    if !details.is_empty() {
        value.insert("details".to_string(), Value::Object(details));
    }
    Value::Object(value)
}

fn decode_media_action(media: &[Field], action: &mut Map<String, Value>) {
    let element = nested_field(media, 5);
    let source_url = element
        .as_ref()
        .and_then(|value| decode_url_value(value, 2));
    let target_id = source_url
        .clone()
        .or_else(|| element.as_ref().and_then(|value| uuid(value, 1)))
        .unwrap_or_default();
    action.insert("targetId".to_string(), json!(target_id));

    // The previous importer only exposed the media details when the action
    // directly addresses a URL. Keep that shape here so both hosts hand the
    // renderer the same action contract for background/empty media actions.
    let Some(source_url) = source_url else {
        return;
    };

    let mut details = Map::new();
    details.insert("sourceUrl".to_string(), json!(source_url));
    if let Some(element) = element.as_ref() {
        details.insert("mediaType".to_string(), json!(media_action_type(element)));
    }

    if let Some(transport) = decode_video_transport(media) {
        match transport {
            Ok(transport) => {
                details.insert("mediaTransport".into(), transport);
            }
            Err(error) => {
                details.insert("unsupportedMediaTransport".into(), json!(error));
            }
        }
    }
    match varint_field(media, 10).unwrap_or(0) {
        0 => {
            details.insert("backgroundBehavior".into(), json!("background"));
        }
        1 => {
            details.insert("backgroundBehavior".into(), json!("foreground"));
        }
        _ => {}
    }
    let playback = element
        .as_ref()
        .and_then(|value| media_element_playback(value))
        .or_else(|| {
            nested_field(media, 7)
                .and_then(|video| varint_field(&video, 1).map(media_playback_value))
        })
        .unwrap_or_default();
    details.insert("playback".to_string(), json!(playback));

    let video = nested_field(media, 7);
    details.insert(
        "loopTime".to_string(),
        json!(video
            .as_ref()
            .and_then(|value| finite_f64_field(value, 3))
            .unwrap_or(0.0)),
    );
    details.insert(
        "timesToLoop".to_string(),
        json!(video
            .as_ref()
            .and_then(|value| varint_field(value, 4))
            .unwrap_or(0)),
    );
    details.insert(
        "softLoop".to_string(),
        json!(video
            .as_ref()
            .and_then(|value| bool_field(value, 5))
            .unwrap_or(false)),
    );
    details.insert(
        "softLoopDuration".to_string(),
        json!(video
            .as_ref()
            .and_then(|value| finite_f64_field(value, 6))
            .unwrap_or(0.0)),
    );
    details.insert(
        "layerType".to_string(),
        json!(varint_field(media, 10)
            .map(media_layer_type_value)
            .unwrap_or_default()),
    );
    details.insert(
        "alwaysRetrigger".to_string(),
        json!(bool_field(media, 11).unwrap_or(false)),
    );
    details.insert(
        "selectedEffectPresetId".to_string(),
        json!(uuid(media, 2).unwrap_or_default()),
    );

    let end_behavior = element
        .as_ref()
        .and_then(|value| media_element_end_behavior(value))
        .or_else(|| {
            video
                .as_ref()
                .and_then(|value| varint_field(value, 2).map(media_end_behavior_value))
        })
        .unwrap_or_default();
    details.insert("endBehavior".to_string(), json!(end_behavior));
    let transition = decode_media_transition(media);
    // A transition outside the library is reported and the layer default plays instead.
    let supported = transition.get("unsupportedTransition").is_none();
    details.insert(
        "transitionDuration".to_string(),
        json!(finite_f64_field(media, 1)
            .filter(|_| supported)
            .unwrap_or(0.0)),
    );
    details.insert("transition".to_string(), transition);

    // Visual library effects on the element or the action form the action's chain;
    // any other effect (for example an audio effect) stays listed and unsupported.
    let element_effects = nested_field(media, 5)
        .map(|element| media_element_effects(&element))
        .unwrap_or_default();
    let all_effects = element_effects
        .into_iter()
        .chain(nested_fields(media, 4))
        .collect::<Vec<_>>();
    let (library, other): (Vec<_>, Vec<_>) = all_effects.iter().partition(|effect| {
        string_field(effect, 4)
            .is_some_and(|id| crate::native_media_effects::catalog_entry(&id).is_some())
    });
    let mut unsupported_variables = Vec::new();
    let chain = decode_media_effect_chain(
        library.iter().map(|effect| effect.as_slice()),
        &mut unsupported_variables,
    );
    if !chain.is_empty() {
        details.insert("mediaEffects".to_string(), Value::Array(chain));
    }
    if !unsupported_variables.is_empty() {
        details.insert(
            "unsupportedMediaEffectVariables".to_string(),
            json!(unsupported_variables),
        );
    }
    let effects = other
        .iter()
        .map(|effect| decode_effect(effect))
        .filter(|effect| {
            effect
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| !name.is_empty())
        })
        .map(Value::Object)
        .collect::<Vec<_>>();
    details.insert("effects".to_string(), Value::Array(effects));
    details.insert(
        "markers".to_string(),
        Value::Array(decode_media_markers(media)),
    );
    action.insert("details".to_string(), Value::Object(details));
}

fn finite_f64_field(fields: &[Field], number: u32) -> Option<f64> {
    f64_field(fields, number).filter(|value| value.is_finite())
}

fn decode_video_transport(media: &[Field]) -> Option<Result<Value, String>> {
    let element = nested_field(media, 5)?;
    let video = nested_field(&element, 6)?;
    let transport = nested_field(&video, 3)?;
    Some((|| {
        let number = |field| match f64_field(&transport, field) {
            Some(value) => Ok(value),
            None if field != 1 && !transport.iter().any(|value| value.number == field) => Ok(0.0),
            None => Err(format!(
                "ProPresenter video transport field {field} requires a number."
            )),
        };
        let end = number(3)?;
        let retrigger_value = match varint_field(&transport, 15) {
            Some(value) => value,
            None if !transport.iter().any(|field| field.number == 15) => 0,
            None => return Err("ProPresenter video retrigger requires an enum value.".to_string()),
        };
        let retrigger = match retrigger_value {
            1 => "restart",
            2 => "continue",
            0 if bool_field(media, 11) == Some(true) => "restart",
            0 | 3 => "automatic",
            value => {
                return Err(format!(
                    "Unsupported ProPresenter video retrigger setting: {value}"
                ))
            }
        };
        let value = json!({
            "inSeconds":number(2)?, "outSeconds":if end == 0.0 {Value::Null} else {json!(end)},
            "rate":number(1)?, "delaySeconds":0.0, "retrigger":retrigger
        });
        crate::output_playback::validate_media_transport(&value)?;
        Ok(value)
    })())
}

fn media_element_playback(element: &[Field]) -> Option<String> {
    nested_field(element, 6)
        .and_then(|video| nested_field(&video, 3))
        .and_then(|transport| varint_field(&transport, 12))
        .map(media_playback_value)
}

fn media_element_end_behavior(element: &[Field]) -> Option<String> {
    nested_field(element, 6)
        .and_then(|video| nested_field(&video, 4))
        .and_then(|properties| varint_field(&properties, 4))
        .map(media_end_behavior_value)
}

fn media_action_type(element: &[Field]) -> &'static str {
    if element.iter().any(|field| field.number == 4) {
        "audio"
    } else if element.iter().any(|field| field.number == 6) {
        "video"
    } else if element.iter().any(|field| field.number == 5) {
        "image"
    } else if element.iter().any(|field| field.number == 7) {
        "live-video"
    } else {
        "media"
    }
}

fn media_playback_value(value: u64) -> String {
    match value {
        0 => "PLAYBACK_BEHAVIOR_STOP".to_string(),
        1 => "PLAYBACK_BEHAVIOR_LOOP".to_string(),
        2 => "PLAYBACK_BEHAVIOR_LOOP_FOR_COUNT".to_string(),
        3 => "PLAYBACK_BEHAVIOR_LOOP_FOR_TIME".to_string(),
        _ => value.to_string(),
    }
}

fn media_end_behavior_value(value: u64) -> String {
    match value {
        0 => "END_BEHAVIOR_STOP".to_string(),
        1 => "END_BEHAVIOR_STOP_ON_BLACK".to_string(),
        2 => "END_BEHAVIOR_STOP_ON_CLEAR".to_string(),
        3 => "END_BEHAVIOR_FADE_TO_BLACK".to_string(),
        4 => "END_BEHAVIOR_FADE_TO_CLEAR".to_string(),
        _ => value.to_string(),
    }
}

fn media_layer_type_value(value: u64) -> String {
    match value {
        0 => "LAYER_TYPE_BACKGROUND".to_string(),
        1 => "LAYER_TYPE_FOREGROUND".to_string(),
        2 => "LAYER_TYPE_FILL".to_string(),
        3 => "LAYER_TYPE_INPUT".to_string(),
        _ => value.to_string(),
    }
}

fn decode_media_transition(media: &[Field]) -> Value {
    let mut transition = Map::new();
    if let Some(duration) = finite_f64_field(&nested_field(media, 3).unwrap_or_default(), 1)
        .filter(|value| *value > 0.0)
    {
        transition.insert("durationSeconds".to_string(), json!(duration));
    }
    if let Some(fields) = nested_field(media, 3) {
        if let Some(effect) = nested_field(&fields, 3) {
            let effect = decode_effect(&effect);
            let name = effect
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if effect
                .get("renderId")
                .and_then(Value::as_str)
                .is_some_and(|id| crate::native_transitions::catalog_entry(id).is_some())
            {
                transition.insert("effect".to_string(), Value::Object(effect));
            } else if !name.is_empty() {
                return json!({ "unsupportedTransition": name });
            }
        }
    }
    Value::Object(transition)
}

/// Converts ProPresenter `Effect` messages into an Orbit media effect chain.
/// Effects outside the media library, and variables that are unknown or out of
/// range, are listed in `unsupported` (the effect keeps its other variables).
fn decode_media_effect_chain<'a>(
    effects: impl Iterator<Item = &'a [Field]>,
    unsupported: &mut Vec<String>,
) -> Vec<Value> {
    use crate::native_media_effects::{catalog_entry, find_variable, variable_json, VariableKind};
    let mut chain = Vec::new();
    for effect in effects {
        let render_id = string_field(effect, 4).unwrap_or_default();
        let name = string_field(effect, 3).unwrap_or_else(|| render_id.clone());
        let Some(entry) = catalog_entry(&render_id) else {
            unsupported.push(format!("effect {name}"));
            continue;
        };
        let mut variables = Map::new();
        for variable in nested_fields(effect, 7) {
            let label = string_field(&variable, 1).unwrap_or_default();
            let Some(spec) = find_variable(entry, &label) else {
                unsupported.push(format!("{} variable {label}", entry.name));
                continue;
            };
            let value = match spec.kind {
                VariableKind::Color => nested_field(&variable, 5).map(|color| {
                    let color = nested_field(&color, 1).unwrap_or_default();
                    let channel = |number| f64::from(f32_field(&color, number).unwrap_or(0.0));
                    [channel(1), channel(2), channel(3)]
                }),
                _ => decode_effect_number(&variable).map(|number| [number, 0.0, 0.0]),
            };
            let json = value
                .filter(|value| value.iter().all(|channel| channel.is_finite()))
                .map(|value| match spec.kind {
                    // Single-precision floats may sit just outside the descriptor range.
                    VariableKind::Number { min, max } | VariableKind::Integer { min, max }
                        if (value[0] - value[0].clamp(min, max)).abs() <= 1e-5 * (max - min) =>
                    {
                        variable_json(spec, [value[0].clamp(min, max), 0.0, 0.0])
                    }
                    _ => variable_json(spec, value),
                })
                .filter(|json| crate::native_media_effects::variable_value(spec, json).is_some());
            match json {
                Some(json) if json != variable_json(spec, spec.value) => {
                    variables.insert(spec.name.to_string(), json);
                }
                Some(_) => {}
                None => unsupported.push(format!("{} variable {label}", entry.name)),
            }
        }
        let mut item =
            json!({ "effectId": entry.id, "enabled": bool_field(effect, 2).unwrap_or(false) });
        if !variables.is_empty() {
            item["variables"] = Value::Object(variables);
        }
        chain.push(item);
    }
    chain
}

/// Reads an int, float or double effect variable value; proto3 omits zero values.
fn decode_effect_number(variable: &[Field]) -> Option<f64> {
    if let Some(int) = nested_field(variable, 3) {
        return Some(varint_field(&int, 1).map_or(0.0, |value| value as i32 as f64));
    }
    if let Some(float) = nested_field(variable, 4) {
        return Some(f32_field(&float, 1).map_or(0.0, f64::from));
    }
    nested_field(variable, 7).map(|double| f64_field(&double, 1).unwrap_or(0.0))
}

/// The effects stored on a media element's drawing properties.
fn media_element_effects(element: &[Field]) -> Vec<Vec<Field>> {
    [5, 6, 7]
        .iter()
        .find_map(|number| nested_field(element, *number))
        .and_then(|properties| nested_field(&properties, 1))
        .map(|drawing| nested_fields(&drawing, 12).collect())
        .unwrap_or_default()
}

/// Writes an Orbit media effect chain item as a ProPresenter `Effect`.
fn encode_media_effect(item: &Value) -> Option<Vec<u8>> {
    use crate::native_media_effects::{catalog_entry, variable_value, VariableKind};
    let entry = catalog_entry(item.get("effectId")?.as_str()?)?;
    let mut effect = Vec::new();
    put_uuid(1, &Uuid::new_v4().to_string(), &mut effect);
    put_varint(2, u64::from(item.get("enabled")?.as_bool()?), &mut effect);
    put_string(3, entry.name, &mut effect);
    put_string(4, entry.id, &mut effect);
    put_string(5, entry.name, &mut effect);
    for spec in entry.variables {
        let value = item
            .get("variables")
            .and_then(|variables| variables.get(spec.name))
            .and_then(|value| variable_value(spec, value))
            .unwrap_or(spec.value);
        let mut variable = Vec::new();
        put_string(1, spec.label, &mut variable);
        match spec.kind {
            VariableKind::Number { min, max } => {
                let mut number = Vec::new();
                put_f32(1, value[0] as f32, &mut number);
                put_f32(2, spec.value[0] as f32, &mut number);
                put_f32(3, min as f32, &mut number);
                put_f32(4, max as f32, &mut number);
                put_message(4, number, &mut variable);
            }
            VariableKind::Integer { min, max } => {
                let mut number = Vec::new();
                put_varint(1, value[0] as i64 as u64, &mut number);
                put_varint(2, spec.value[0] as i64 as u64, &mut number);
                put_varint(3, min as i64 as u64, &mut number);
                put_varint(4, max as i64 as u64, &mut number);
                put_message(3, number, &mut variable);
            }
            VariableKind::Color => {
                let color = |rgb: [f64; 3]| {
                    let mut color = Vec::new();
                    for (number, channel) in rgb.into_iter().enumerate() {
                        put_f32(number as u32 + 1, channel as f32, &mut color);
                    }
                    put_f32(4, 1.0, &mut color);
                    color
                };
                let mut effect_color = Vec::new();
                put_message(1, color(value), &mut effect_color);
                put_message(2, color(spec.value), &mut effect_color);
                put_message(5, effect_color, &mut variable);
            }
        }
        put_message(7, variable, &mut effect);
    }
    Some(effect)
}

/// Decodes a `Presets/Effect` file (`Effect.Preset`) as an Orbit media effect preset.
pub fn decode_effect_preset(bytes: &[u8], unsupported: &mut Vec<String>) -> Option<Value> {
    let fields = decode_message(bytes).ok()?;
    let id = uuid(&fields, 1).filter(|id| Uuid::parse_str(id).is_ok())?;
    let title = string_field(&fields, 2).filter(|title| !title.trim().is_empty())?;
    let effects = nested_fields(&fields, 3).collect::<Vec<_>>();
    if effects.is_empty() {
        return None;
    }
    let chain = decode_media_effect_chain(effects.iter().map(Vec::as_slice), unsupported);
    Some(json!({
        "id": id,
        "title": title,
        "effects": { "mediaHue": 0.0, "mediaSaturation": 1.0, "mediaBrightness": 0.0, "mediaContrast": 1.0 },
        "mediaEffects": chain,
    }))
}

/// Writes an Orbit media effect preset as ProPresenter's `Effect.Preset`. Orbit's
/// four color adjustments lead the chain as an Adjust Color effect when they are
/// not neutral.
pub fn encode_effect_preset(preset: &Value) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    put_uuid(1, preset.get("id")?.as_str()?, &mut bytes);
    put_string(2, preset.get("title")?.as_str()?, &mut bytes);
    let adjustments = preset.get("effects")?;
    let number = |key: &str| adjustments.get(key).and_then(Value::as_f64);
    let (hue, saturation, brightness, contrast) = (
        number("mediaHue")?,
        number("mediaSaturation")?,
        number("mediaBrightness")?,
        number("mediaContrast")?,
    );
    if (hue, saturation, brightness, contrast) != (0.0, 1.0, 0.0, 1.0) {
        let adjust = json!({
            "effectId": "D4C199A9-64D7-4CCB-8C26-443E7A9C5C03",
            "enabled": true,
            "variables": {
                "hue": hue.to_radians(),
                "saturation": saturation,
                "brightness": brightness,
                "contrast": contrast,
            },
        });
        put_message(3, encode_media_effect(&adjust)?, &mut bytes);
    }
    for item in preset
        .get("mediaEffects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        put_message(3, encode_media_effect(item)?, &mut bytes);
    }
    Some(bytes)
}

/// Per-asset audio settings of the media cues in a ProPresenter media or audio bin document
/// (`Playlists/Media`, `Playlists/Audio`), keyed like [`decode_media_bin_effects`]:
/// `audioRouting` unless `Media.AudioProperties` is ProPresenter's default (unity volume,
/// identity mapping, nothing muted or limited), and `audioPlayback` from an audio cue's
/// playback behavior. Returns per cue the path, the settings and what could not be imported.
pub fn decode_media_bin_audio(bytes: &[u8]) -> Vec<(String, Map<String, Value>, Vec<String>)> {
    let mut routes = Vec::new();
    if let Some(root) = decode_message(bytes)
        .ok()
        .and_then(|fields| nested_field(&fields, 3))
    {
        collect_media_bin_audio(&root, &mut routes);
    }
    routes
}

fn collect_media_bin_audio(
    playlist: &[Field],
    routes: &mut Vec<(String, Map<String, Value>, Vec<String>)>,
) {
    for entry in nested_field(playlist, 13)
        .map(|children| nested_fields(&children, 1).collect::<Vec<_>>())
        .unwrap_or_default()
    {
        let Some(cue) = nested_field(&entry, 5) else {
            continue;
        };
        for media in nested_fields(&cue, 10).filter_map(|action| nested_field(&action, 20)) {
            let Some(element) = nested_field(&media, 5) else {
                continue;
            };
            let url = nested_field(&element, 2).unwrap_or_default();
            let path = nested_field(&url, 4)
                .and_then(|local| string_field(&local, 2))
                .or_else(|| string_field(&url, 1))
                .unwrap_or_default();
            if path.is_empty() {
                continue;
            }
            let mut settings = Map::new();
            let mut unsupported = Vec::new();
            // `Media.AudioTypeProperties.audio` (field 4.1) or `VideoTypeProperties.audio` (6.2).
            if let Some(properties) = nested_field(&element, 4)
                .and_then(|audio| nested_field(&audio, 1))
                .or_else(|| nested_field(&element, 6).and_then(|video| nested_field(&video, 2)))
            {
                if let Some(route) = decode_audio_properties(&properties, &mut unsupported) {
                    settings.insert("audioRouting".into(), route);
                }
            }
            // `Action.MediaType.Audio.playback_behavior` (field 8.1).
            if let Some(audio) = nested_field(&media, 8) {
                if audio.iter().any(|field| field.number == 3) {
                    match varint_field(&audio, 3).filter(|value| *value <= u32::MAX as u64) {
                        Some(count) => {
                            settings.insert("audioLoopCount".into(), json!(count));
                        }
                        None => unsupported.push("audio loop count is invalid".into()),
                    }
                }
                if audio.iter().any(|field| field.number == 2) {
                    match f64_field(&audio, 2).filter(|value| value.is_finite() && *value >= 0.0) {
                        Some(time) => {
                            settings.insert("audioLoopTime".into(), json!(time));
                        }
                        None => unsupported.push("audio loop time is invalid".into()),
                    }
                }
                match varint_field(&audio, 1).unwrap_or(0) {
                    0 => {}
                    1 => {
                        settings.insert("audioPlayback".into(), json!("loop"));
                    }
                    2 => match varint_field(&audio, 3)
                        .filter(|value| *value > 0 && *value <= u32::MAX as u64)
                    {
                        Some(count) => {
                            settings.insert("audioPlayback".into(), json!("loop-count"));
                            settings.insert("audioLoopCount".into(), json!(count));
                        }
                        None => unsupported.push("audio loop count is missing or invalid".into()),
                    },
                    3 => match f64_field(&audio, 2)
                        .filter(|value| value.is_finite() && *value > 0.0)
                    {
                        Some(time) => {
                            settings.insert("audioPlayback".into(), json!("loop-time"));
                            settings.insert("audioLoopTime".into(), json!(time));
                        }
                        None => unsupported.push("audio loop time is missing or invalid".into()),
                    },
                    other => unsupported.push(format!("audio playback behavior {other}")),
                }
            }
            if !settings.is_empty() || !unsupported.is_empty() {
                routes.push((path, settings, unsupported));
            }
        }
    }
    for child in nested_fields(playlist, 9) {
        collect_media_bin_audio(&child, routes);
    }
    if let Some(playlists) = nested_field(playlist, 12) {
        for child in nested_fields(&playlists, 1) {
            collect_media_bin_audio(&child, routes);
        }
    }
}

/// Orbit's per-asset route from `Media.AudioProperties`. The master volume is linear (1.0 is
/// unity, as ProPresenter writes it by default); channel and output indexes count from zero.
/// Outputs past Orbit's two ProPresenter channels and invalid per-channel dB values are listed as
/// unsupported.
fn decode_audio_properties(properties: &[Field], unsupported: &mut Vec<String>) -> Option<Value> {
    let volume = finite_f64_field(properties, 1).unwrap_or(1.0);
    let gain_db = if volume > 0.0 {
        20.0 * volume.log10()
    } else {
        -60.0
    };
    if !(-60.0..=12.0).contains(&gain_db) {
        unsupported.push(format!("audio volume {volume}"));
    }
    let mut channels = Vec::new();
    let mut default_mapping = true;
    for channel in nested_fields(properties, 2) {
        let index = varint_field(&channel, 1).unwrap_or(0);
        if index >= 8 {
            unsupported.push(format!("audio source channel {}", index + 1));
            continue;
        }
        let muted = bool_field(&channel, 2).unwrap_or(false);
        let compress_limit = bool_field(&channel, 4).unwrap_or(false);
        let channel_volume_present = channel.iter().any(|field| field.number == 3);
        let channel_gain_db = match finite_f64_field(&channel, 3) {
            Some(gain_db) if (-60.0..=12.0).contains(&gain_db) => Some(gain_db),
            Some(gain_db) => {
                unsupported.push(format!("audio channel {} volume {gain_db}", index + 1));
                None
            }
            None if channel_volume_present => {
                unsupported.push(format!("audio channel {} volume is invalid", index + 1));
                None
            }
            None => Some(0.0),
        };
        let mut outputs = Vec::new();
        for output in nested_fields(&channel, 5) {
            let target = varint_field(&output, 2).unwrap_or(0) as i64;
            if (0..2).contains(&target) {
                outputs.push(json!(target + 1));
            } else {
                unsupported.push(format!("audio channel {} output {}", index + 1, target + 1));
            }
        }
        default_mapping &= !muted
            && !compress_limit
            && outputs == [json!(index + 1)]
            && channel_gain_db == Some(0.0);
        let mut route = json!({ "sourceChannel": index + 1, "outputs": outputs, "muted": muted });
        if let Some(gain_db) = channel_gain_db.filter(|gain_db| *gain_db != 0.0) {
            route["gainDb"] = json!(gain_db);
        }
        if compress_limit {
            route["compressLimit"] = json!(true);
        }
        channels.push(route);
    }
    let gain_db = gain_db.clamp(-60.0, 12.0);
    if channels.is_empty() || (default_mapping && gain_db.abs() < 1e-9) {
        return None;
    }
    Some(json!({ "gainDb": (gain_db * 100.0).round() / 100.0, "channels": channels }))
}

/// ProPresenter's smart media playlists in `Playlists/Media`. Each is linked to a folder
/// (`Playlist.smart_directory`) and keeps the cues of its last folder sync, so the files it
/// held are known even when they have since disappeared. Returns per playlist its UUID,
/// name, folder, import behavior and the synced files (cue UUID, name, absolute path and
/// library effect chain).
pub fn decode_media_smart_folders(bytes: &[u8]) -> Vec<Value> {
    let mut folders = Vec::new();
    if let Some(root) = decode_message(bytes)
        .ok()
        .and_then(|fields| nested_field(&fields, 3))
    {
        collect_media_smart_folders(&root, &mut folders);
    }
    folders
}

fn collect_media_smart_folders(playlist: &[Field], folders: &mut Vec<Value>) {
    if let Some(directory) = nested_field(playlist, 14) {
        let folder = nested_field(&directory, 1)
            .and_then(|url| string_field(&url, 1))
            .unwrap_or_default();
        let mut items = Vec::new();
        for entry in nested_field(playlist, 13)
            .map(|children| nested_fields(&children, 1).collect::<Vec<_>>())
            .unwrap_or_default()
        {
            let Some(cue) = nested_field(&entry, 5) else {
                continue;
            };
            for media in nested_fields(&cue, 10).filter_map(|action| nested_field(&action, 20)) {
                let Some(element) = nested_field(&media, 5) else {
                    continue;
                };
                let Some(path) = nested_field(&element, 2)
                    .and_then(|url| string_field(&url, 1))
                    .filter(|path| !path.is_empty())
                else {
                    continue;
                };
                let effects = media_element_effects(&element)
                    .into_iter()
                    .chain(nested_fields(&media, 4))
                    .collect::<Vec<_>>();
                let mut unsupported = Vec::new();
                let chain =
                    decode_media_effect_chain(effects.iter().map(Vec::as_slice), &mut unsupported);
                let mut item = json!({
                    "sourceId": uuid(&cue, 1).unwrap_or_default(),
                    "title": string_field(&cue, 2).unwrap_or_default(),
                    "sourcePath": path,
                });
                if !chain.is_empty() {
                    item["mediaEffects"] = Value::Array(chain);
                }
                if !unsupported.is_empty() {
                    item["unsupportedMediaEffects"] = json!(unsupported);
                }
                items.push(item);
            }
        }
        folders.push(json!({
            "sourceId": uuid(playlist, 1).unwrap_or_default(),
            "title": string_field(playlist, 2).unwrap_or_default(),
            "sourcePath": folder,
            "backgroundBehavior": if varint_field(&directory, 2) == Some(1) { "foreground" } else { "background" },
            "items": items,
        }));
    }
    for child in nested_fields(playlist, 9) {
        collect_media_smart_folders(&child, folders);
    }
    if let Some(playlists) = nested_field(playlist, 12) {
        for child in nested_fields(&playlists, 1) {
            collect_media_smart_folders(&child, folders);
        }
    }
}

/// Effect chains of the media cues in a ProPresenter media bin document
/// (`Playlists/Media`), keyed by the media path relative to the ProPresenter
/// folder when known, otherwise by its absolute path.
pub fn decode_media_bin_effects(bytes: &[u8]) -> Vec<(String, Vec<Value>, Vec<String>)> {
    let Ok(fields) = decode_message(bytes) else {
        return Vec::new();
    };
    let mut chains = Vec::new();
    if let Some(root) = nested_field(&fields, 3) {
        collect_media_bin_effects(&root, &mut chains);
    }
    chains
}

fn media_bin_action_path(media: &[Field]) -> Option<String> {
    let element = nested_field(media, 5)?;
    let url = nested_field(&element, 2)?;
    nested_field(&url, 4)
        .and_then(|local| string_field(&local, 2))
        .or_else(|| string_field(&url, 1))
}

fn collect_media_bin_effects(
    playlist: &[Field],
    chains: &mut Vec<(String, Vec<Value>, Vec<String>)>,
) {
    collect_media_bin_actions(playlist, &mut |action| {
        let Some(media) = nested_field(action, 20) else {
            return;
        };
        let Some(path) = media_bin_action_path(&media).filter(|path| !path.is_empty()) else {
            return;
        };
        let Some(element) = nested_field(&media, 5) else {
            return;
        };
        let effects = media_element_effects(&element)
            .into_iter()
            .chain(nested_fields(&media, 4))
            .collect::<Vec<_>>();
        if effects.is_empty() {
            return;
        }
        let mut unsupported = Vec::new();
        let chain = decode_media_effect_chain(effects.iter().map(Vec::as_slice), &mut unsupported);
        chains.push((path, chain, unsupported));
    });
}

/// Custom video thumbnail times from a media bin. ProPresenter stores
/// `Media.VideoProperties.thumbnail_position` in seconds and -1 when the thumbnail is unset.
pub fn decode_media_bin_thumbnails(bytes: &[u8]) -> Vec<(String, f64)> {
    let mut thumbnails = Vec::new();
    if let Some(root) = decode_message(bytes)
        .ok()
        .and_then(|fields| nested_field(&fields, 3))
    {
        collect_media_bin_actions(&root, &mut |action| {
            let Some(media) = nested_field(action, 20) else {
                return;
            };
            let Some(path) = media_bin_action_path(&media).filter(|path| !path.is_empty()) else {
                return;
            };
            let seconds = nested_field(&media, 5)
                .and_then(|element| nested_field(&element, 6))
                .and_then(|video| nested_field(&video, 4))
                .and_then(|properties| f64_field(&properties, 3))
                .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
            if let Some(seconds) = seconds {
                thumbnails.push((path, seconds));
            }
        });
    }
    thumbnails
}

pub fn decode_media_bin_transports(bytes: &[u8]) -> Vec<(String, Result<Value, String>)> {
    let mut transports = Vec::new();
    if let Some(root) = decode_message(bytes)
        .ok()
        .and_then(|fields| nested_field(&fields, 3))
    {
        collect_media_bin_actions(&root, &mut |action| {
            let Some(media) = nested_field(action, 20) else {
                return;
            };
            let Some(path) = media_bin_action_path(&media).filter(|path| !path.is_empty()) else {
                return;
            };
            let Some(transport) = decode_video_transport(&media) else {
                return;
            };
            let transport = transport.and_then(|mut transport| {
                let delay = match f64_field(action, 4) {
                    Some(value) => value,
                    None if !action.iter().any(|field| field.number == 4) => 0.0,
                    None => return Err("ProPresenter media delay requires a number.".to_string()),
                };
                transport["delaySeconds"] = json!(delay);
                crate::output_playback::validate_media_transport(&transport)?;
                let behavior = match varint_field(&media, 10).unwrap_or(0) {
                    0 => "background",
                    1 => "foreground",
                    value => return Err(format!("Unsupported ProPresenter video layer: {value}")),
                };
                let playback = nested_field(&media, 5).and_then(|element| media_element_playback(&element));
                let playback = if playback.is_some_and(|value| value.contains("LOOP")) {"loop"} else {"stop"};
                Ok(json!({"mediaTransport":transport,"backgroundBehavior":behavior,"backgroundPlayback":playback}))
            });
            transports.push((path, transport));
        });
    }
    transports
}

fn collect_media_bin_actions(playlist: &[Field], visit: &mut dyn FnMut(&[Field])) {
    if let Some(children) = nested_field(playlist, 13) {
        for entry in nested_fields(&children, 1) {
            if let Some(cue) = nested_field(&entry, 5) {
                for action in nested_fields(&cue, 10) {
                    visit(&action);
                }
            }
        }
    }
    for child in nested_fields(playlist, 9) {
        collect_media_bin_actions(&child, visit);
    }
    if let Some(playlists) = nested_field(playlist, 12) {
        for child in nested_fields(&playlists, 1) {
            collect_media_bin_actions(&child, visit);
        }
    }
}

fn decode_effect(fields: &[Field]) -> Map<String, Value> {
    let raw_name = string_field(fields, 3).unwrap_or_default();
    let render_id = string_field(fields, 4).unwrap_or_default();
    let mut effect = Map::new();
    effect.insert(
        "name".to_string(),
        json!(if raw_name.is_empty() {
            render_id.clone()
        } else {
            raw_name
        }),
    );
    if !render_id.is_empty() {
        effect.insert("renderId".to_string(), json!(render_id));
    }
    if let Some(category) = string_field(fields, 6).filter(|value| !value.is_empty()) {
        effect.insert("category".to_string(), json!(category));
    }
    if bool_field(fields, 2) == Some(false) {
        effect.insert("enabled".to_string(), json!(false));
    }
    effect
}

fn decode_media_markers(media: &[Field]) -> Vec<Value> {
    nested_fields(media, 12)
        .enumerate()
        .filter_map(|(index, marker)| {
            let time_seconds = finite_f64_field(&marker, 2)?;
            let actions = nested_fields(&marker, 5)
                .enumerate()
                .map(|(action_index, action)| decode_action(&action, action_index))
                .collect::<Vec<_>>();
            let mut value = json!({
                "id": uuid(&marker, 1).unwrap_or_else(|| format!("marker-{}", index + 1)),
                "timeSeconds": time_seconds,
                "title": string_field(&marker, 4)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "Playback marker".to_string()),
                "actions": actions,
            });
            if let Some(color) = nested_field(&marker, 3).and_then(|fields| color(&fields)) {
                value["color"] = json!(color);
            }
            Some(value)
        })
        .collect()
}

fn action_contract_type(value: u64) -> &'static str {
    match value {
        1 => "stage",
        2 | 12 | 13 => "media",
        3 => "timer",
        4 => "communication",
        5 => "clear",
        6 | 15 => "prop",
        7 => "mask",
        8 => "message",
        10 | 18 => "audience-look",
        11 => "presentation-slide",
        14 => "presentation",
        17 => "external-presentation",
        19 => "audio-input",
        20 | 21 | 26 => "playlist",
        22 => "slide-destination",
        23 => "macro",
        24 => "clear-group",
        25 => "capture",
        _ => "unknown",
    }
}

fn timer_action_value(value: u64) -> &'static str {
    match value {
        1 => "stop",
        2 => "reset",
        3 => "reset-and-start",
        4 => "stop-and-reset",
        5 => "increment",
        _ => "start",
    }
}

fn clear_layer_value(value: u64) -> &'static str {
    match value {
        1 => "audio",
        2 => "background",
        3 => "video-input",
        4 => "prop",
        5 => "slide",
        6 => "logo",
        7 => "messages",
        8 => "audio-effects",
        _ => "all",
    }
}

fn content_destination_value(value: u64) -> &'static str {
    match value {
        1 => "CONTENT_DESTINATION_ANNOUNCEMENTS",
        _ => "CONTENT_DESTINATION_GLOBAL",
    }
}
fn parse_metadata(fields: &[Field]) -> Map<String, Value> {
    let mut metadata = Map::new();
    if let Some(value) = nested_field(fields, 14) {
        let mut ccli = Map::new();
        for (field, key) in [
            (1, "author"),
            (2, "artistCredits"),
            (3, "title"),
            (4, "publisher"),
            (8, "album"),
        ] {
            if let Some(value) = string_field(&value, field) {
                ccli.insert(key.to_string(), json!(value));
            }
        }
        for (field, key) in [(5, "copyrightYear"), (6, "songNumber")] {
            if let Some(value) = varint_field(&value, field) {
                ccli.insert(key.to_string(), json!(value));
            }
        }
        if let Some(value) = bool_field(&value, 7) {
            ccli.insert("display".to_string(), json!(value));
        }
        if let Some(value) = bytes_field(&value, 9) {
            ccli.insert("artwork".to_string(), json!(base64_encode(value)));
        }
        if !ccli.is_empty() {
            metadata.insert("ccli".to_string(), Value::Object(ccli));
        }
    }
    if let Some(value) = nested_field(fields, 15) {
        let mut bible = Map::new();
        if let Some(value) = varint_field(&value, 1) {
            bible.insert("bookIndex".to_string(), json!(value));
        }
        for (field, key) in [
            (2, "book"),
            (5, "translation"),
            (6, "translationDisplayAbbreviation"),
            (8, "bookKey"),
        ] {
            if let Some(value) = string_field(&value, field) {
                bible.insert(key.to_string(), json!(value));
            }
        }
        if let Some(value) = nested_field(&value, 3) {
            if let Some(start) = varint_field(&value, 1) {
                bible.insert("chapterStart".to_string(), json!(start));
            }
            if let Some(end) = varint_field(&value, 2) {
                bible.insert("chapterEnd".to_string(), json!(end));
            }
        }
        if let Some(value) = nested_field(&value, 4) {
            if let Some(start) = varint_field(&value, 1) {
                bible.insert("verseStart".to_string(), json!(start));
            }
            if let Some(end) = varint_field(&value, 2) {
                bible.insert("verseEnd".to_string(), json!(end));
            }
        }
        if !bible.is_empty() {
            metadata.insert("bibleReference".to_string(), Value::Object(bible));
        }
    }
    if let Some(value) = nested_field(fields, 9)
        .and_then(|value| string_field(&value, 1).or_else(|| string_field(&value, 2)))
    {
        metadata.insert("chordChartUrl".to_string(), json!(value));
    }
    if let Some(value) = string_field(fields, 22) {
        metadata.insert("musicKey".to_string(), json!(value));
    } else if let Some(value) = nested_field(fields, 23)
        .and_then(|value| string_field(&value, 2).or_else(|| string_field(&value, 1)))
    {
        metadata.insert("musicKey".to_string(), json!(value));
    }
    if let Some(value) = f64_field(fields, 20) {
        metadata.insert("slideShowDurationSeconds".to_string(), json!(value));
    }
    if let Some(value) = varint_field(fields, 19) {
        metadata.insert(
            "contentDestination".to_string(),
            json!(if value == 1 {
                "announcements"
            } else {
                "global"
            }),
        );
    }
    metadata
}

/// Decode a ProPresenter Presentation document into the workspace's item
/// contract.  A document is accepted only when the protobuf root parses and
/// has at least one known presentation field (name, uuid, or cue).
pub fn decode_presentation(bytes: &[u8], source: &Path) -> Option<Value> {
    let fields = decode_message(bytes).ok()?;
    let title = string_field(&fields, 3).unwrap_or_default();
    let (groups, groups_by_cue) = parse_groups(&fields);
    let cues = nested_fields(&fields, 13)
        .enumerate()
        .map(|(index, cue)| {
            let group = uuid(&cue, 1).and_then(|id| groups_by_cue.get(&id));
            parse_cue(&cue, index, group)
        })
        .collect::<Vec<_>>();
    // A cue that cannot be read must not silently shift the cues after it.
    if cues.len() != bytes_fields(&fields, 13).count() {
        return None;
    }
    if title.is_empty() && cues.is_empty() && uuid(&fields, 2).is_none() {
        return None;
    }
    let mut item = Map::new();
    item.insert("title".to_string(), json!(title));
    item.insert(
        "importSourcePath".to_string(),
        json!(source.to_string_lossy()),
    );
    item.insert("slides".to_string(), Value::Array(cues));
    item.insert(
        "arrangements".to_string(),
        json!(parse_arrangements(&fields)),
    );
    item.insert("groups".to_string(), Value::Array(groups));
    if let Some(value) = uuid(&fields, 2) {
        item.insert("sourceId".to_string(), json!(value));
    }
    if let Some(value) = uuid(&fields, 10) {
        item.insert("selectedArrangementId".to_string(), json!(value));
    }
    if let Some(value) = string_field(&fields, 7) {
        item.insert(
            "operatorNotes".to_string(),
            json!(rtf_to_text(value.as_bytes())),
        );
    }
    if let Some(timeline) = parse_timeline(&fields) {
        item.insert("timeline".to_string(), timeline);
    }
    if let Some(transition) = nested_field(&fields, 18) {
        insert_transition(&mut item, &transition);
    }
    item.extend(parse_metadata(&fields));
    Some(Value::Object(item))
}

fn parse_groups(fields: &[Field]) -> (Vec<Value>, BTreeMap<String, Map<String, Value>>) {
    let mut groups = Vec::new();
    let mut cue_groups = BTreeMap::new();
    for cue_group in nested_fields(fields, 12) {
        let Some(group) = nested_field(&cue_group, 1) else {
            continue;
        };
        let Some(id) = uuid(&group, 1) else {
            continue;
        };
        let name = string_field(&group, 2)
            .or_else(|| string_field(&group, 6))
            .unwrap_or_else(|| format!("Group {}", groups.len() + 1));
        let group_id = group_id(&name);
        let mut group_value = Map::new();
        group_value.insert("id".to_string(), json!(group_id));
        group_value.insert("name".to_string(), json!(name));
        group_value.insert("sourceId".to_string(), json!(id));
        if let Some(value) = nested_field(&group, 3).and_then(|value| color(&value)) {
            group_value.insert("color".to_string(), json!(value));
        }
        groups.push(Value::Object(group_value.clone()));
        let mut slide_group = Map::new();
        slide_group.insert("groupId".to_string(), json!(group_id));
        slide_group.insert("groupName".to_string(), json!(name));
        slide_group.insert("sectionId".to_string(), json!(id));
        slide_group.insert("sourceGroupId".to_string(), json!(id));
        if let Some(value) = group_value.get("color") {
            slide_group.insert("groupColor".to_string(), value.clone());
        }
        for cue_identifier in nested_fields(&cue_group, 2) {
            if let Some(cue_id) = uuid(&cue_identifier, 1) {
                cue_groups.insert(cue_id, slide_group.clone());
            }
        }
    }
    (groups, cue_groups)
}

fn group_id(name: &str) -> String {
    let mut result = String::new();
    let mut pending_separator = false;
    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !result.is_empty() {
                result.push('-');
            }
            result.push(character.to_ascii_lowercase());
            pending_separator = false;
        } else {
            pending_separator = true;
        }
    }
    if result.is_empty() {
        "group".to_string()
    } else {
        result
    }
}

fn parse_timeline(fields: &[Field]) -> Option<Value> {
    let timeline = nested_field(fields, 17)?;
    let mut cues = nested_fields(&timeline, 11).collect::<Vec<_>>();
    if cues.is_empty() {
        cues = nested_fields(&timeline, 1).collect();
    }
    let cues = cues
        .into_iter()
        .enumerate()
        .filter_map(|(index, cue)| {
            let time = f64_field(&cue, 1)?;
            let cue_id = uuid(&cue, 2);
            let action = nested_field(&cue, 4).map(|value| decode_action(&value, index));
            if cue_id.is_none() && action.is_none() {
                return None;
            }
            let title = string_field(&cue, 3)
                .or_else(|| {
                    action.as_ref().and_then(|value| {
                        value
                            .get("title")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                })
                .unwrap_or_else(|| "Timeline cue".to_string());
            let mut value = Map::new();
            value.insert(
                "id".to_string(),
                json!(cue_id
                    .clone()
                    .unwrap_or_else(|| format!("timeline-{}", index + 1))),
            );
            value.insert("timeSeconds".to_string(), json!(time));
            value.insert("title".to_string(), json!(title));
            if let Some(cue_id) = cue_id {
                value.insert("cueId".to_string(), json!(cue_id));
            }
            if let Some(action) = action {
                value.insert("action".to_string(), action);
            }
            Some(Value::Object(value))
        })
        .collect::<Vec<_>>();
    let soundtrack = nested_field(&timeline, 8).map(|action| {
        let action = decode_action(&action, 0);
        let source_url = action["details"]["sourceUrl"]
            .as_str()
            .filter(|url| !url.is_empty())
            .map(str::to_string);
        json!({ "sourceUrl": source_url })
    });
    if cues.is_empty() && soundtrack.is_none() {
        return None;
    }
    let mut value = Map::new();
    value.insert("cues".to_string(), Value::Array(cues));
    if let Some(duration) = f64_field(&timeline, 5) {
        value.insert("durationSeconds".to_string(), json!(duration));
    }
    value.insert(
        "loop".to_string(),
        json!(bool_field(&timeline, 6) == Some(true)),
    );
    if bool_field(&timeline, 9) == Some(true) {
        value.insert("timecodeEnabled".to_string(), json!(true));
        value.insert(
            "timecodeOffsetSeconds".to_string(),
            json!(f64_field(&timeline, 10).unwrap_or(0.0)),
        );
    }
    if let Some(soundtrack) = soundtrack {
        value.insert("importedSoundtrack".to_string(), soundtrack);
    }
    Some(Value::Object(value))
}

fn parse_arrangements(fields: &[Field]) -> Vec<Value> {
    nested_fields(fields, 11)
        .enumerate()
        .map(|(index, value)| {
            let id = uuid(&value, 1).unwrap_or_else(|| format!("arrangement-{}", index + 1));
            let name =
                string_field(&value, 2).unwrap_or_else(|| format!("Arrangement {}", index + 1));
            let section_ids = nested_fields(&value, 3)
                .filter_map(|value| string_field(&value, 1))
                .collect::<Vec<_>>();
            json!({ "id": id, "name": name, "sectionIds": section_ids })
        })
        .collect()
}

/// Decode all playlists contained in a PlaylistDocument. A document may hold
/// a tree of folders and playlists, rather than a single flat playlist.
pub fn decode_playlists(bytes: &[u8]) -> Vec<Value> {
    let Ok(fields) = decode_message(bytes) else {
        return Vec::new();
    };
    let Some(root) = nested_field(&fields, 3) else {
        return Vec::new();
    };
    let mut playlists = Vec::new();
    collect_playlists(&root, "", &mut playlists);
    playlists
}

fn collect_playlists(fields: &[Field], parent: &str, playlists: &mut Vec<Value>) {
    let mut items = Vec::new();
    let mut unsupported = Vec::new();
    if let Some(children) = nested_field(fields, 13) {
        for (index, entry) in bytes_fields(&children, 1).enumerate() {
            let Ok(entry) = decode_message(entry) else {
                unsupported.push(format!("playlist item {} could not be read", index + 1));
                continue;
            };
            match decode_playlist_item(&entry, index) {
                Some(item) => items.push(item),
                None => unsupported.push(format!(
                    "playlist item {} has an unsupported type ({})",
                    index + 1,
                    entry
                        .iter()
                        .map(|field| field.number)
                        .find(|number| !matches!(number, 1 | 2 | 7 | 9))
                        .map_or_else(|| "none".to_string(), |number| format!("field {number}"))
                )),
            }
        }
    }

    let kind = varint_field(fields, 3).unwrap_or(0);
    let is_folder = kind == 2;
    let included = kind != 4 && (is_folder || kind == 1 || nested_field(fields, 13).is_some());
    let id =
        uuid(fields, 1).unwrap_or_else(|| format!("imported-playlist-{}", playlists.len() + 1));
    if included {
        let mut playlist = Map::new();
        playlist.insert("id".to_string(), json!(id));
        playlist.insert(
            "type".to_string(),
            json!(if is_folder { "folder" } else { "playlist" }),
        );
        playlist.insert("parentId".to_string(), json!(parent));
        if let Some(source_id) = uuid(fields, 1) {
            playlist.insert("sourceId".to_string(), json!(source_id));
        }
        playlist.insert(
            "title".to_string(),
            json!(string_field(fields, 2).unwrap_or_else(|| "Playlist".to_string())),
        );
        if let Some(expanded) = bool_field(fields, 4) {
            playlist.insert("expanded".to_string(), json!(expanded));
        }
        if let Some(targeted_layer_id) = uuid(fields, 5) {
            playlist.insert("targetedLayerId".to_string(), json!(targeted_layer_id));
        }
        if let Some(path) = decode_url(fields, 6).strip_prefix("file:///") {
            playlist.insert("smartDirectoryPath".to_string(), json!(path));
        }
        if let Some(timecode_enabled) = bool_field(fields, 10) {
            playlist.insert("timecodeEnabled".to_string(), json!(timecode_enabled));
        }
        if let Some(timing) = varint_field(fields, 11) {
            playlist.insert("timing".to_string(), json!(timing));
        }
        if let Some(startup) = nested_field(fields, 16).and_then(|value| bool_field(&value, 1)) {
            playlist.insert("startupTrigger".to_string(), json!(startup));
        }
        playlist.insert("items".to_string(), Value::Array(items));
        note_unsupported(&mut playlist, unsupported);
        playlists.push(Value::Object(playlist));
    }

    let parent = if included && is_folder {
        id.as_str()
    } else {
        parent
    };
    for child in nested_fields(fields, 9) {
        collect_playlists(&child, parent, playlists);
    }
    if let Some(playlist_array) = nested_field(fields, 12) {
        for child in nested_fields(&playlist_array, 1) {
            collect_playlists(&child, parent, playlists);
        }
    }
}

fn decode_playlist_item(entry: &[Field], index: usize) -> Option<Value> {
    let mut item = Map::new();
    item.insert(
        "id".to_string(),
        json!(uuid(entry, 1).unwrap_or_else(|| format!("playlist-item-{}", index + 1))),
    );
    item.insert(
        "title".to_string(),
        json!(string_field(entry, 2).unwrap_or_default()),
    );
    if let Some(hidden) = bool_field(entry, 9) {
        item.insert("isHidden".to_string(), json!(hidden));
    }
    let tag_ids = nested_fields(entry, 7)
        .filter_map(|tag| string_field(&tag, 1))
        .collect::<Vec<_>>();
    if !tag_ids.is_empty() {
        item.insert("tagIds".to_string(), json!(tag_ids));
    }
    if let Some(presentation) = nested_field(entry, 4) {
        item.insert("type".to_string(), json!("presentation"));
        item.insert(
            "documentPath".to_string(),
            json!(decode_url(&presentation, 1)),
        );
        if let Some(value) = uuid(&presentation, 2) {
            item.insert("arrangementId".to_string(), json!(value));
        }
        playlist_document_reference(&presentation, &mut item);
        if let Some(destination) = varint_field(&presentation, 3) {
            item.insert("contentDestination".into(), json!(destination));
        }
    } else if nested_field(entry, 3).is_some() {
        item.insert("type".to_string(), json!("header"));
        if let Some(color) = nested_field(entry, 3)
            .and_then(|header| nested_field(&header, 1))
            .and_then(|value| color(&value))
        {
            item.insert("color".to_string(), json!(color));
        }
        if let Some(header) = nested_field(entry, 3) {
            let actions = nested_fields(&header, 2)
                .enumerate()
                .map(|(index, action)| decode_action(&action, index))
                .collect::<Vec<_>>();
            if !actions.is_empty() {
                item.insert("actions".into(), json!(actions));
            }
        }
    } else if nested_field(entry, 5).is_some() {
        item.insert("type".to_string(), json!("cue"));
    } else if let Some(placeholder) = nested_field(entry, 8) {
        item.insert("type".to_string(), json!("placeholder"));
        if let Some(linked) = nested_field(&placeholder, 1) {
            if let Some(presentation) = nested_field(&linked, 4) {
                item.insert(
                    "documentPath".to_string(),
                    json!(decode_url(&presentation, 1)),
                );
                playlist_document_reference(&presentation, &mut item);
            }
        }
    } else {
        return None;
    }
    Some(Value::Object(item))
}

fn playlist_document_reference(presentation: &[Field], item: &mut Map<String, Value>) {
    if let Some(local) = nested_field(presentation, 1).and_then(|url| nested_field(&url, 4)) {
        if varint_field(&local, 1) == Some(10) {
            if let Some(path) = string_field(&local, 2) {
                item.insert("documentRelativePath".into(), json!(path));
            }
        }
    }
}

fn decode_url(fields: &[Field], number: u32) -> String {
    nested_field(fields, number)
        .and_then(|value| string_field(&value, 1).or_else(|| string_field(&value, 2)))
        .unwrap_or_default()
}

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}

fn encode_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn field_key(number: u32, wire_type: u8, output: &mut Vec<u8>) {
    encode_varint((u64::from(number) << 3) | u64::from(wire_type), output);
}

fn put_varint(number: u32, value: u64, output: &mut Vec<u8>) {
    field_key(number, 0, output);
    encode_varint(value, output);
}

fn put_f64(number: u32, value: f64, output: &mut Vec<u8>) {
    field_key(number, 1, output);
    output.extend_from_slice(&value.to_le_bytes());
}

fn put_bytes(number: u32, value: &[u8], output: &mut Vec<u8>) {
    field_key(number, 2, output);
    encode_varint(value.len() as u64, output);
    output.extend_from_slice(value);
}

fn put_string(number: u32, value: &str, output: &mut Vec<u8>) {
    put_bytes(number, value.as_bytes(), output);
}

fn put_message(number: u32, value: Vec<u8>, output: &mut Vec<u8>) {
    put_bytes(number, &value, output);
}

fn put_uuid(number: u32, value: &str, output: &mut Vec<u8>) {
    let mut uuid = Vec::new();
    let value = normalized_uuid(value);
    put_string(1, &value, &mut uuid);
    put_message(number, uuid, output);
}

fn normalized_uuid(value: &str) -> String {
    value
        .parse::<Uuid>()
        .map(|value| value.to_string())
        .unwrap_or_else(|_| {
            if value.is_empty() {
                Uuid::new_v4().to_string()
            } else {
                Uuid::new_v5(
                    &Uuid::NAMESPACE_URL,
                    format!("presenter-orbit:{value}").as_bytes(),
                )
                .to_string()
            }
        })
}

fn put_color(number: u32, value: &str, output: &mut Vec<u8>) {
    let Some([blue, green, red, alpha]) = crate::native_compositor::parse_color(value) else {
        return;
    };
    let mut color = Vec::new();
    for (field, channel) in [(1, red), (2, green), (3, blue), (4, alpha)] {
        put_f32(field, f32::from(channel) / 255.0, &mut color);
    }
    put_message(number, color, output);
}

fn put_f32(number: u32, value: f32, output: &mut Vec<u8>) {
    field_key(number, 5, output);
    output.extend_from_slice(&value.to_le_bytes());
}

fn escape_rtf(value: &str) -> String {
    let mut result = String::new();
    for character in value.chars() {
        match character {
            '\\' | '{' | '}' => {
                result.push('\\');
                result.push(character);
            }
            '\t' => result.push_str("\\tab "),
            '\n' => result.push_str("\\par "),
            '\u{2028}' => result.push_str("\\line "),
            character if character.is_ascii() => result.push(character),
            character => {
                for unit in character.encode_utf16(&mut [0; 2]) {
                    result.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    result
}

#[derive(Clone, Default)]
struct RtfExportList {
    id: u32,
    formats: BTreeMap<u8, Map<String, Value>>,
    levels: BTreeMap<u8, Map<String, Value>>,
}

#[derive(Clone)]
struct RtfExportCounter {
    format: Map<String, Value>,
    next: i64,
}

fn export_list_format(style: &Map<String, Value>) -> Option<(u8, Map<String, Value>)> {
    if !crate::native_text_list::invalid_list_fields(style).is_empty() {
        return None;
    }
    let name = style.get(crate::native_text_list::STYLE_FIELD)?.as_str()?;
    if name == "none" {
        return None;
    }
    let list_style = crate::native_text_list::ListStyle::parse(name)?;
    let level = style
        .get("textListLevel")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let level = u8::try_from(level).ok().filter(|level| *level <= 8)?;
    let mut format = Map::new();
    format.insert(
        crate::native_text_list::STYLE_FIELD.to_string(),
        json!(name),
    );
    if list_style.is_numbered() {
        format.insert(
            crate::native_text_list::PREFIX_FIELD.to_string(),
            style
                .get(crate::native_text_list::PREFIX_FIELD)
                .cloned()
                .unwrap_or_else(|| json!("")),
        );
        format.insert(
            crate::native_text_list::POSTFIX_FIELD.to_string(),
            style
                .get(crate::native_text_list::POSTFIX_FIELD)
                .cloned()
                .unwrap_or_else(|| json!("")),
        );
        format.insert(
            crate::native_text_list::START_FIELD.to_string(),
            style
                .get(crate::native_text_list::START_FIELD)
                .cloned()
                .unwrap_or_else(|| json!(1)),
        );
    }
    Some((level, format))
}

fn list_number_format(style: &str) -> Option<u8> {
    Some(match style {
        "upper-roman" => 1,
        "lower-roman" => 2,
        "upper-alpha" => 3,
        "lower-alpha" => 4,
        "decimal" => 0,
        "box" | "check" | "diamond" | "disc" | "circle" | "square" | "hyphen" => 23,
        "none" => 255,
        _ => return None,
    })
}

fn escape_rtf_list_text(value: &str) -> String {
    escape_rtf(value).replace(';', "\\;")
}

fn encode_rtf_list_level(list_id: u32, level: u8, style: &Map<String, Value>) -> String {
    let name = style[crate::native_text_list::STYLE_FIELD]
        .as_str()
        .unwrap_or("decimal");
    let Some(number_format) = list_number_format(name) else {
        return String::new();
    };
    let marker = match name {
        "box" => "□",
        "check" => "✓",
        "diamond" => "◆",
        "disc" => "•",
        "circle" => "◦",
        "square" => "▪",
        "hyphen" => "-",
        _ => "",
    };
    let prefix = style
        .get(crate::native_text_list::PREFIX_FIELD)
        .and_then(Value::as_str)
        .unwrap_or_default();
    let postfix = style
        .get(crate::native_text_list::POSTFIX_FIELD)
        .and_then(Value::as_str)
        .unwrap_or_default();
    let start = style
        .get(crate::native_text_list::START_FIELD)
        .and_then(Value::as_i64)
        .unwrap_or(1);
    let pattern_len = if number_format == 255 {
        0
    } else if number_format == 23 {
        marker.encode_utf16().count()
    } else {
        prefix.encode_utf16().count() + 1 + postfix.encode_utf16().count()
    };
    let template_id = list_id * 9 + u32::from(level);
    let mut result = format!("{{\\listlevel\\levelnfc{number_format}\\levelstartat{start}\\leveltemplateid{template_id}\\levelfollow1");
    if number_format == 255 {
        result.push_str("{\\leveltext\\'00;}{\\levelnumbers;}");
    } else if number_format == 23 {
        result.push_str(&format!(
            "{{\\leveltext\\'{pattern_len:02x}{};}}",
            escape_rtf(marker)
        ));
    } else {
        let number_position = u8::try_from(prefix.encode_utf16().count() + 1).unwrap_or(1);
        result.push_str(&format!(
            "{{\\leveltext\\'{pattern_len:02x}{}\\'00{};}}{{\\levelnumbers\\'{number_position:02x};}}",
            escape_rtf_list_text(prefix),
            escape_rtf_list_text(postfix),
        ));
    }
    result.push('}');
    result
}

fn rtf_list_tables(lists: &[RtfExportList]) -> String {
    if lists.is_empty() {
        return String::new();
    }
    let mut table = String::from("{\\*\\listtable");
    for list in lists {
        let max_level = list.levels.keys().copied().max().unwrap_or(0);
        table.push_str(&format!(
            "{{\\list\\listtemplateid{}\\listhybrid\\listid{}",
            list.id, list.id
        ));
        for level in 0..=max_level {
            let style = list.levels.get(&level).cloned().unwrap_or_else(|| {
                Map::from_iter([(
                    crate::native_text_list::STYLE_FIELD.to_string(),
                    json!("none"),
                )])
            });
            table.push_str(&encode_rtf_list_level(list.id, level, &style));
        }
        table.push('}');
    }
    table.push('}');
    table.push_str("{\\*\\listoverridetable");
    for list in lists {
        table.push_str(&format!(
            "{{\\listoverride\\listid{}\\listoverridecount0\\ls{}}}",
            list.id, list.id
        ));
    }
    table.push('}');
    table
}

fn rtf_paragraph_list_assignments(
    styles: &[Map<String, Value>],
    paragraphs: &[&str],
) -> (Vec<RtfExportList>, Vec<Option<(u32, u8)>>) {
    let mut lists = Vec::<RtfExportList>::new();
    let mut assignments = Vec::with_capacity(styles.len());
    let mut active = None;
    let mut counters: [Option<RtfExportCounter>; 9] = std::array::from_fn(|_| None);
    for (style, text) in styles.iter().zip(paragraphs) {
        let Some((level, format)) = export_list_format(style) else {
            assignments.push(None);
            continue;
        };
        let unchanged = counters[level as usize]
            .as_ref()
            .is_some_and(|counter| counter.format == format);
        if !unchanged {
            let start = format
                .get(crate::native_text_list::START_FIELD)
                .and_then(Value::as_i64)
                .unwrap_or(1);
            counters[level as usize] = Some(RtfExportCounter {
                format: format.clone(),
                next: start,
            });
        }
        if !text.is_empty() {
            for counter in counters.iter_mut().skip(level as usize + 1) {
                *counter = None;
            }
        }
        let needs_new = active.is_none_or(|index: usize| {
            lists[index]
                .formats
                .get(&level)
                .is_some_and(|existing| existing != &format)
        });
        if needs_new {
            let id = u32::try_from(lists.len() + 1).unwrap_or(u32::MAX);
            let mut list = RtfExportList {
                id,
                ..RtfExportList::default()
            };
            for (index, counter) in counters.iter().enumerate() {
                if let Some(counter) = counter {
                    let mut level_style = counter.format.clone();
                    if level_style.contains_key(crate::native_text_list::START_FIELD) {
                        level_style.insert(
                            crate::native_text_list::START_FIELD.to_string(),
                            json!(counter.next),
                        );
                    }
                    list.formats.insert(index as u8, counter.format.clone());
                    list.levels.insert(index as u8, level_style);
                }
            }
            let counter = counters[level as usize]
                .as_ref()
                .expect("counter was initialized");
            let mut level_style = format.clone();
            if level_style.contains_key(crate::native_text_list::START_FIELD) {
                level_style.insert(
                    crate::native_text_list::START_FIELD.to_string(),
                    json!(counter.next),
                );
            }
            list.formats.insert(level, format.clone());
            list.levels.insert(level, level_style);
            lists.push(list);
            active = Some(lists.len() - 1);
        }
        let list = &mut lists[active.expect("new list exists")];
        if !list.formats.contains_key(&level) {
            let counter = counters[level as usize]
                .as_ref()
                .expect("counter was initialized");
            let mut level_style = format.clone();
            if level_style.contains_key(crate::native_text_list::START_FIELD) {
                level_style.insert(
                    crate::native_text_list::START_FIELD.to_string(),
                    json!(counter.next),
                );
            }
            list.formats.insert(level, format.clone());
            list.levels.insert(level, level_style);
        }
        assignments.push(Some((list.id, level)));
        if !text.is_empty() {
            if let Some(counter) = counters[level as usize].as_mut() {
                counter.next += 1;
            }
        }
    }
    (lists, assignments)
}

/// The RTF body of the slide text, the font table entries its runs add after `\f0` and the color
/// table entries they add after `\cf1`. Runs that no longer spell the text are not exported.
fn rtf_body(
    text: &str,
    runs: &[Value],
    base_style: &Map<String, Value>,
    text_width: Option<f64>,
) -> (String, String, String, String) {
    let spelled: String = runs.iter().filter_map(|run| run["text"].as_str()).collect();
    let plain_run = json!({"text": text});
    let valid_runs = !runs.is_empty() && spelled == text;
    let runs = if valid_runs {
        runs
    } else {
        std::slice::from_ref(&plain_run)
    };
    let mut fonts = Vec::new();
    let mut colors = Vec::new();
    let mut run_controls = Vec::with_capacity(runs.len());
    for run in runs {
        let style = &run["style"];
        let mut controls = String::new();
        if let Some(family) = style["fontFamily"].as_str() {
            let number = fonts
                .iter()
                .position(|font| font == family)
                .unwrap_or_else(|| {
                    fonts.push(family.to_string());
                    fonts.len() - 1
                });
            controls.push_str(&format!("\\f{}", number + 1));
        }
        if let Some(size) = style["fontSize"].as_f64() {
            controls.push_str(&format!("\\fs{}", (size * 2.0).round().max(1.0)));
        }
        for (key, control) in [("bold", "\\b"), ("italic", "\\i")] {
            if style[key] == true {
                controls.push_str(control);
            }
        }
        if style["underline"] == true {
            let control = match (
                style["underlineStyle"].as_str(),
                style["underlinePattern"].as_str(),
                style["underlineByWord"].as_bool(),
            ) {
                (_, _, Some(true)) => "\\ulw",
                (Some("thick"), _, _) => "\\ulth",
                (Some("double"), _, _) => "\\uldb",
                (_, Some("dot"), _) => "\\uld",
                (_, Some("dash"), _) => "\\uldash",
                (_, Some("dash-dot"), _) => "\\uldashd",
                (_, Some("dash-dot-dot"), _) => "\\uldashdd",
                _ => "\\ul",
            };
            controls.push_str(control);
        }
        if style["strikethrough"] == true {
            controls.push_str(if style["strikethroughStyle"] == "double" {
                "\\striked"
            } else {
                "\\strike"
            });
        }
        match style["superscript"].as_i64() {
            Some(value) if value > 0 => controls.push_str("\\super"),
            Some(value) if value < 0 => controls.push_str("\\sub"),
            Some(_) => controls.push_str("\\nosupersub"),
            _ => {}
        }
        if let Some(spacing) = style["letterSpacing"]
            .as_f64()
            .filter(|value| value.is_finite())
        {
            controls.push_str(&format!("\\expndtw{}", (spacing * 20.0).round() as i64));
        }
        if let Some(rgb) = style["underlineColor"]
            .as_str()
            .and_then(crate::native_compositor::parse_color)
            .map(|[b, g, r, _]| u32::from_be_bytes([0, r, g, b]))
        {
            colors.push(format!(
                "\\red{}\\green{}\\blue{};",
                rgb >> 16,
                (rgb >> 8) & 255,
                rgb & 255
            ));
            controls.push_str(&format!("\\ulc{}", colors.len() + 1));
        }
        if let Some(rgb) = style["textColor"]
            .as_str()
            .and_then(crate::native_compositor::parse_color)
            .map(|[b, g, r, _]| u32::from_be_bytes([0, r, g, b]))
        {
            colors.push(format!(
                "\\red{}\\green{}\\blue{};",
                rgb >> 16,
                (rgb >> 8) & 255,
                rgb & 255
            ));
            controls.push_str(&format!("\\cf{}", colors.len() + 1));
        }
        if !controls.is_empty() {
            controls.push(' ');
        }
        run_controls.push(controls);
    }
    let fonts = fonts
        .iter()
        .enumerate()
        .map(|(index, font)| format!("{{\\f{} {};}}", index + 1, escape_rtf(font)))
        .collect();
    let colors = colors.concat();
    let mut paragraphs = text.split('\n').collect::<Vec<_>>();
    if paragraphs.is_empty() {
        paragraphs.push("");
    }
    let styles = (0..paragraphs.len())
        .map(|index| crate::native_text_layout::resolve_paragraph_style(base_style, index))
        .collect::<Vec<_>>();
    let (lists, list_assignments) = rtf_paragraph_list_assignments(&styles, &paragraphs);
    let paragraph_tables = rtf_list_tables(&lists);
    let mut styled_body = String::new();
    let mut offset = 0usize;
    let run_ranges = runs
        .iter()
        .filter_map(|run| {
            let content = run["text"].as_str()?;
            let range = (offset, offset + content.len(), run);
            offset += content.len();
            Some(range)
        })
        .collect::<Vec<_>>();
    offset = 0;
    for (index, paragraph) in paragraphs.iter().enumerate() {
        let mut rtf_style = styles[index].clone();
        if let Some(edge) = rtf_style
            .get("textAbsoluteRightEdge")
            .and_then(Value::as_f64)
        {
            if base_style
                .get("textAbsoluteRightEdge")
                .and_then(Value::as_f64)
                == Some(edge)
            {
                rtf_style.remove("textRightIndent");
            } else if let Some(width) = text_width {
                rtf_style.insert("textRightIndent".into(), json!((width - edge).max(0.0)));
            }
        }
        let (_, controls) = encode_paragraph_style(&rtf_style);
        styled_body.push_str("\\pard");
        styled_body.push_str(&controls);
        if let Some((list_id, level)) = list_assignments[index] {
            styled_body.push_str(&format!("\\ls{list_id}\\ilvl{level}"));
        } else {
            styled_body.push_str("\\ls0");
        }
        styled_body.push(' ');
        let start = offset;
        let has_separator = index + 1 < paragraphs.len();
        let end = start + paragraph.len();
        for (run_index, (run_start, run_end, run)) in run_ranges.iter().enumerate() {
            let from = (*run_start).max(start);
            let to = (*run_end).min(end);
            if from >= to {
                continue;
            }
            let content = &run["text"].as_str().unwrap()[from - *run_start..to - *run_start];
            styled_body.push('{');
            styled_body.push_str(&run_controls[run_index]);
            styled_body.push_str(&escape_rtf(content));
            styled_body.push('}');
        }
        if has_separator {
            // Newline-separated Orbit text represents separate paragraphs. `\pard` only resets
            // paragraph formatting; it does not terminate the previous paragraph.
            let run = run_ranges
                .iter()
                .position(|(start, finish, _)| *start <= end && end < *finish);
            if let Some(run) = run {
                styled_body.push('{');
                styled_body.push_str(&run_controls[run]);
            }
            styled_body.push_str("\\par ");
            if run.is_some() {
                styled_body.push('}');
            }
        }
        if text.ends_with('\n') && index + 1 == paragraphs.len() {
            // A final newline means the text has a final empty paragraph as well as the
            // separator already emitted after the preceding paragraph.
            styled_body.push_str("\\par ");
        }
        offset = end + usize::from(has_separator);
    }
    (fonts, colors, paragraph_tables, styled_body)
}

pub(crate) fn selected_primary_text_layer(slide: &Value) -> Option<&Value> {
    let layers = slide.get("layers").and_then(Value::as_array)?;
    let primary_id = slide.get("primaryTextLayerId").and_then(Value::as_str);
    primary_id
        .and_then(|id| layers.iter().find(|layer| layer["id"].as_str() == Some(id)))
        .or_else(|| {
            layers
                .iter()
                .find(|layer| matches!(layer["role"].as_str(), Some("primary" | "slide-text")))
        })
        .or_else(|| {
            layers.iter().find(|layer| {
                layer["type"]
                    .as_str()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("text"))
            })
        })
}

pub(crate) fn portable_text_style(slide: &Value) -> Map<String, Value> {
    let mut style = slide
        .get("textStyle")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(layer_style) =
        selected_primary_text_layer(slide).and_then(|layer| layer["textStyle"].as_object())
    {
        style.extend(layer_style.clone());
    }
    style
}

fn encode_text_element_with_style(
    text: &str,
    runs: &[Value],
    width: f64,
    height: f64,
    text_color: &str,
    style: &Map<String, Value>,
) -> Vec<u8> {
    let mut point = Vec::new();
    put_f64(1, width * 0.12, &mut point);
    put_f64(2, height * 0.18, &mut point);
    let mut size = Vec::new();
    put_f64(1, width * 0.76, &mut size);
    put_f64(2, height * 0.64, &mut size);
    let mut bounds = Vec::new();
    put_message(1, point, &mut bounds);
    put_message(2, size, &mut bounds);

    let mut graphics = Vec::new();
    put_uuid(1, "", &mut graphics);
    put_string(2, "Text", &mut graphics);
    put_message(3, bounds, &mut graphics);
    put_f64(5, 1.0, &mut graphics);
    put_message(
        13,
        layer_export::text_value(
            text,
            runs,
            &Value::Null,
            style,
            text_color,
            &BTreeMap::new(),
            Some(width * 0.76),
        ),
        &mut graphics,
    );

    let mut element = Vec::new();
    put_message(1, graphics, &mut element);
    put_varint(4, 2, &mut element); // INFO_IS_TEXT_ELEMENT
    element
}

fn encode_paragraph_style(style: &Map<String, Value>) -> (Vec<u8>, String) {
    let mut paragraph = Vec::new();
    let alignment = match style.get("textHorizontalAlign").and_then(Value::as_str) {
        Some("left") => 0,
        Some("right") => 1,
        Some("justify") => 3,
        Some("natural") => 4,
        Some("center") | None => 2,
        Some(_) => 2,
    };
    put_varint(1, alignment, &mut paragraph);
    let number = |key: &str| style.get(key).and_then(Value::as_f64);
    let left = number("textLeftIndent").unwrap_or(0.0);
    let first = number("textFirstLineIndent").unwrap_or(0.0);
    let right = number("textRightIndent").unwrap_or(0.0);
    if let Some(value) = number("textLineHeightMultiple") {
        put_f64(5, value, &mut paragraph);
    }
    for (field, key) in [
        (6, "textMaximumLineHeight"),
        (7, "textMinimumLineHeight"),
        (8, "textLineSpacing"),
        (10, "textParagraphSpacingBefore"),
        (9, "textParagraphSpacingAfter"),
    ] {
        if let Some(value) = number(key) {
            put_f64(field, value, &mut paragraph);
        }
    }
    if number("textLeftIndent").is_some() {
        put_f64(3, left, &mut paragraph);
    }
    if number("textFirstLineIndent").is_some() || number("textLeftIndent").is_some() {
        put_f64(2, left + first, &mut paragraph);
    }
    if let Some(edge) = number("textAbsoluteRightEdge") {
        put_f64(4, edge, &mut paragraph);
    } else if number("textRightIndent").is_some() {
        put_f64(4, -right, &mut paragraph);
    }
    if let Some(interval) = number(crate::native_text_tabs::INTERVAL_FIELD) {
        put_f64(12, interval, &mut paragraph);
    }
    if let Some(stops) = style
        .get(crate::native_text_tabs::STOPS_FIELD)
        .and_then(Value::as_array)
    {
        if crate::native_text_tabs::parse_tab_stops(&Value::Array(stops.clone())).is_ok() {
            for stop in stops {
                let position = stop["position"].as_f64().unwrap_or_default();
                let alignment = match stop["alignment"].as_str() {
                    Some("left") => Some(0),
                    Some("right") => Some(1),
                    Some("center") => Some(2),
                    Some("justify") => Some(3),
                    Some("decimal") => None,
                    _ => continue,
                };
                if let Some(alignment) = alignment {
                    let mut encoded = Vec::new();
                    put_f64(1, position, &mut encoded);
                    put_varint(2, alignment, &mut encoded);
                    put_message(11, encoded, &mut paragraph);
                }
            }
        }
    }
    if let Some(list) = encode_proto_text_list(style) {
        put_message(13, list, &mut paragraph);
    }
    let mut levels = BTreeMap::<u8, Map<String, Value>>::new();
    if let Some((level, format)) = export_list_format(style) {
        levels.insert(level, format);
    }
    if let Some(paragraphs) = style.get("textParagraphStyles").and_then(Value::as_array) {
        for index in 0..paragraphs.len() {
            let resolved = crate::native_text_layout::resolve_paragraph_style(style, index);
            if let Some((level, format)) = export_list_format(&resolved) {
                levels.entry(level).or_insert(format);
            }
        }
    }
    if let Some(maximum) = levels.keys().copied().max() {
        for level in 0..=maximum {
            let encoded = levels
                .get(&level)
                .and_then(encode_proto_text_list)
                .unwrap_or_default();
            put_message(14, encoded, &mut paragraph);
        }
    }
    (paragraph, encode_rtf_paragraph_style(style))
}

fn encode_proto_text_list(style: &Map<String, Value>) -> Option<Vec<u8>> {
    if !crate::native_text_list::invalid_list_fields(style).is_empty() {
        return None;
    }
    let number_type = style
        .get(crate::native_text_list::STYLE_FIELD)
        .and_then(Value::as_str)
        .and_then(|list_style| match list_style {
            "box" => Some(0),
            "check" => Some(1),
            "diamond" => Some(3),
            "circle" => Some(2),
            "disc" => Some(4),
            "hyphen" => Some(5),
            "square" => Some(6),
            "decimal" => Some(7),
            "lower-alpha" => Some(8),
            "upper-alpha" => Some(9),
            "lower-roman" => Some(10),
            "upper-roman" => Some(11),
            _ => None,
        })?;
    let mut list = Vec::new();
    put_varint(1, 1, &mut list);
    put_varint(2, number_type, &mut list);
    for (key, field) in [
        (crate::native_text_list::PREFIX_FIELD, 3),
        (crate::native_text_list::POSTFIX_FIELD, 4),
    ] {
        if let Some(value) = style.get(key).and_then(Value::as_str) {
            put_string(field, value, &mut list);
        }
    }
    if let Some(start) = style
        .get(crate::native_text_list::START_FIELD)
        .and_then(Value::as_u64)
    {
        put_varint(5, start, &mut list);
    }
    Some(list)
}

fn encode_rtf_paragraph_style(style: &Map<String, Value>) -> String {
    let mut rtf = match style.get("textHorizontalAlign").and_then(Value::as_str) {
        Some("left") => String::from("\\ql"),
        Some("right") => String::from("\\qr"),
        Some("justify") => String::from("\\qj"),
        Some("natural") => String::from("\\qnatural"),
        _ => String::from("\\qc"),
    };
    let number = |key: &str| style.get(key).and_then(Value::as_f64);
    let left = number("textLeftIndent").unwrap_or(0.0);
    let first = number("textFirstLineIndent").unwrap_or(0.0);
    let right = number("textRightIndent").unwrap_or(0.0);
    if let Some(value) = number("textLineHeightMultiple") {
        rtf.push_str(&format!("\\sl{}\\slmult1", (240.0 * value).round() as i64));
    } else if let Some(minimum) = number("textMinimumLineHeight") {
        let maximum = number("textMaximumLineHeight").unwrap_or(0.0);
        if maximum > 0.0 && (maximum - minimum).abs() < 1e-6 {
            rtf.push_str(&format!("\\sl-{}\\slmult0", twips(minimum)));
        } else {
            rtf.push_str(&format!("\\sl{}\\slmult0", twips(minimum)));
        }
    }
    for (key, control) in [
        ("textLineSpacing", "\\slleading"),
        ("textMinimumLineHeight", "\\slminimum"),
        ("textMaximumLineHeight", "\\slmaximum"),
    ] {
        if let Some(value) = number(key) {
            rtf.push_str(&format!("{control}{}", twips(value)));
        }
    }
    for (key, control) in [
        ("textParagraphSpacingBefore", "\\sb"),
        ("textParagraphSpacingAfter", "\\sa"),
    ] {
        if let Some(value) = number(key) {
            rtf.push_str(&format!("{control}{}", twips(value)));
        }
    }
    if number("textLeftIndent").is_some() {
        rtf.push_str(&format!("\\li{}", twips(left)));
    }
    if number("textFirstLineIndent").is_some() {
        rtf.push_str(&format!("\\fi{}", twips(first)));
    }
    if number("textRightIndent").is_some() {
        rtf.push_str(&format!("\\ri{}", twips(right)));
    }
    if let Some(interval) = number(crate::native_text_tabs::INTERVAL_FIELD) {
        rtf.push_str(&format!("\\deftab{}", twips(interval)));
    } else {
        rtf.push_str("\\pardeftab0");
    }
    if let Some(stops) = style
        .get(crate::native_text_tabs::STOPS_FIELD)
        .and_then(Value::as_array)
    {
        if crate::native_text_tabs::parse_tab_stops(&Value::Array(stops.clone())).is_ok() {
            for stop in stops {
                let position = stop["position"].as_f64().unwrap_or_default();
                rtf.push_str(match stop["leader"].as_str() {
                    Some("dot") => "\\tldot",
                    Some("equal") => "\\tleq",
                    Some("hyphen") => "\\tlhyph",
                    Some("thick") => "\\tlth",
                    Some("underline") => "\\tlul",
                    _ => "",
                });
                rtf.push_str(match stop["alignment"].as_str() {
                    Some("right") => "\\tqr",
                    Some("center") => "\\tqc",
                    Some("decimal") => "\\tqdec",
                    Some("justify") => "\\tql",
                    _ => "\\tql",
                });
                rtf.push_str(&format!("\\tx{}", twips(position)));
            }
        }
    }
    if let Some(bars) = style.get(crate::native_text_tabs::BAR_STOPS_FIELD) {
        if let Ok(bars) = crate::native_text_tabs::parse_bar_stops(bars) {
            for position in bars {
                rtf.push_str(&format!("\\tb{}", twips(position)));
            }
        }
    }
    rtf
}

fn twips(points: f64) -> i64 {
    (points * 20.0).round() as i64
}

/// `layered` carries the show-relative media paths of a presentation, whose slides with layers of
/// their own export those layers; without it the slide exports its text only.
fn encode_slide(
    slide: &Value,
    index: usize,
    layered: Option<&BTreeMap<String, String>>,
) -> Vec<u8> {
    let width = slide.get("width").and_then(Value::as_f64).unwrap_or(1920.0);
    let height = slide
        .get("height")
        .and_then(Value::as_f64)
        .unwrap_or(1080.0);
    let text = slide
        .get("content")
        .or_else(|| slide.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut base = Vec::new();
    let light_background = slide["background"]
        .as_str()
        .and_then(crate::native_compositor::parse_color)
        .map(|[b, g, r, _]| u32::from_be_bytes([0, r, g, b]))
        .is_some_and(|rgb| {
            299 * (rgb >> 16) + 587 * ((rgb >> 8) & 255) + 114 * (rgb & 255) > 128_000
        });
    let text_color = if light_background {
        "#000000"
    } else {
        "#ffffff"
    };
    let elements = layered
        .and_then(|relative| {
            layer_export::slide_elements(slide, width, height, text_color, relative)
        })
        .unwrap_or_else(|| {
            vec![encode_text_element_with_style(
                text,
                slide["contentRuns"].as_array().map_or(&[], Vec::as_slice),
                width,
                height,
                text_color,
                &portable_text_style(slide),
            )]
        });
    for element in elements {
        put_message(1, element, &mut base);
    }
    if layered.is_some() {
        for id in layer_build_export::order(slide) {
            put_uuid(2, &id, &mut base);
        }
    }
    put_varint(
        4,
        u64::from(
            slide
                .get("backgroundEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        ),
        &mut base,
    );
    if let Some(background) = slide.get("background").and_then(Value::as_str) {
        put_color(5, background, &mut base);
    }
    let mut size = Vec::new();
    put_f64(1, width, &mut size);
    put_f64(2, height, &mut size);
    put_message(6, size, &mut base);
    put_uuid(7, &exported_slide_id(slide, index), &mut base);

    let mut presentation_slide = Vec::new();
    put_message(1, base, &mut presentation_slide);
    if layered.is_some() {
        cue_export::encode_notes(slide, &mut presentation_slide);
    }
    if let Some(transition) = slide.get("transition").and_then(encode_transition) {
        put_message(5, transition, &mut presentation_slide);
    }
    let mut slide_type = Vec::new();
    put_message(2, presentation_slide, &mut slide_type);

    let imported_action = slide["actions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|action| action["type"] == "presentation-slide")
        .filter(|_| layered.is_some());
    let mut action = Vec::new();
    put_uuid(
        1,
        imported_action
            .and_then(|action| action["id"].as_str())
            .unwrap_or_default(),
        &mut action,
    );
    put_string(
        2,
        imported_action
            .and_then(|action| action["title"].as_str())
            .or_else(|| slide.get("label").and_then(Value::as_str))
            .unwrap_or("Slide"),
        &mut action,
    );
    let slide_label: Map<_, _> = ["label", "labelColor"]
        .into_iter()
        .filter_map(|key| {
            slide
                .get(key)
                .filter(|value| value.is_string())
                .map(|value| (key.to_string(), value.clone()))
        })
        .collect();
    if let Some(details) = (!slide_label.is_empty())
        .then_some(&slide_label)
        .or_else(|| imported_action.and_then(|action| action["details"].as_object()))
    {
        // Invalid label colors are reported by the export review; the label text still exports.
        let _ = marker_action_export::encode_label(details, &mut action);
    }
    put_varint(
        6,
        u64::from(
            !slide
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
        &mut action,
    );
    put_varint(9, 11, &mut action); // ACTION_TYPE_PRESENTATION_SLIDE
    put_message(23, slide_type, &mut action);
    let actions = match layered {
        Some(relative) => cue_export::encode_actions(slide, action, relative),
        None => vec![action],
    };

    let mut cue = Vec::new();
    if let Some(value) = slide.get("sourceCueId").and_then(Value::as_str) {
        put_uuid(1, value, &mut cue);
    } else {
        put_uuid(1, &format!("slide-{}", index + 1), &mut cue);
    }
    put_string(
        2,
        slide["name"]
            .as_str()
            .or_else(|| slide["label"].as_str())
            .unwrap_or("Slide"),
        &mut cue,
    );
    for action in actions {
        put_message(10, action, &mut cue);
    }
    put_varint(
        12,
        u64::from(
            !slide
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
        &mut cue,
    );
    if layered.is_some() {
        cue_export::encode_cue_settings(slide, &mut cue);
    }
    cue
}

/// Encode a workspace item as a valid ProPresenter Presentation document. Slides with layers of
/// their own export them (text, shapes, fills, image and video media); `relative_media` maps the
/// file URL of each exported media file to its path relative to the show folder. Fields the
/// layer export cannot write are listed by `layer_export::losses`.
pub fn encode_presentation_with_media(
    item: &Value,
    relative_media: &BTreeMap<String, String>,
) -> Vec<u8> {
    let mut document = Vec::new();
    if let Some(source_id) = item.get("sourceId").and_then(Value::as_str) {
        put_uuid(2, source_id, &mut document);
    }
    put_string(
        3,
        item.get("title")
            .and_then(Value::as_str)
            .unwrap_or("Presentation"),
        &mut document,
    );
    if let Some(notes) = item.get("operatorNotes").and_then(Value::as_str) {
        put_string(7, notes, &mut document);
    }
    if let Some(ccli) = item.get("ccli").and_then(Value::as_object) {
        let mut value = Vec::new();
        for (field, key) in [
            (1, "author"),
            (2, "artistCredits"),
            (3, "title"),
            (4, "publisher"),
            (8, "album"),
        ] {
            if let Some(text) = ccli.get(key).and_then(Value::as_str) {
                put_string(field, text, &mut value);
            }
        }
        for (field, key) in [(5, "copyrightYear"), (6, "songNumber")] {
            if let Some(number) = ccli.get(key).and_then(Value::as_u64) {
                put_varint(field, number, &mut value);
            }
        }
        if let Some(display) = ccli.get("display").and_then(Value::as_bool) {
            put_varint(7, u64::from(display), &mut value);
        }
        put_message(14, value, &mut document);
    }
    if let Some(key) = item.get("musicKey").and_then(Value::as_str) {
        put_string(22, key, &mut document);
    }
    if let Some(duration) = item.get("slideShowDurationSeconds").and_then(Value::as_f64) {
        put_f64(20, duration, &mut document);
    }
    if let Some(transition) = item.get("transition").and_then(encode_transition) {
        put_message(18, transition, &mut document);
    }
    let mut cue_ids = Vec::new();
    for (index, slide) in item["slides"].as_array().into_iter().flatten().enumerate() {
        let cue = encode_slide(slide, index, Some(relative_media));
        let fields = decode_message(&cue).expect("Encoded cue must be valid protobuf");
        cue_ids.push(
            bytes_field(&fields, 1)
                .expect("Encoded cue must have an identity")
                .to_vec(),
        );
        put_message(13, cue, &mut document);
    }
    let groups = cue_export::cue_groups(item);
    groups.encode(&cue_ids, &mut document);
    cue_export::encode_arrangements(item, &groups, &mut document);
    document
}

/// `encode_template` with the slides' own layers, as `encode_presentation_with_media` writes them.
pub fn encode_template_with_media(
    theme: &Value,
    relative_media: &BTreeMap<String, String>,
) -> Vec<u8> {
    let mut document = Vec::new();
    if let Some(slides) = theme.get("slides").and_then(Value::as_array) {
        for (index, slide) in slides.iter().enumerate() {
            let encoded_cue = encode_slide(slide, index, Some(relative_media));
            let Some(cue) = decode_message(&encoded_cue).ok() else {
                continue;
            };
            let Some(action) = nested_field(&cue, 10) else {
                continue;
            };
            let Some(slide_type) = nested_field(&action, 23) else {
                continue;
            };
            let Some(presentation) = nested_field(&slide_type, 2) else {
                continue;
            };
            let Some(base) = bytes_field(&presentation, 1) else {
                continue;
            };
            let mut template_slide = Vec::new();
            put_message(1, base.to_vec(), &mut template_slide);
            put_string(
                2,
                slide
                    .get("name")
                    .or_else(|| slide.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("Theme Slide"),
                &mut template_slide,
            );
            put_message(3, template_slide, &mut document);
        }
    }
    document
}

pub fn encode_workspace_playlist(
    roots: &[Value],
    presentation_paths: &Map<String, Value>,
    destination: &Path,
) -> Result<Vec<u8>, String> {
    let mut relative_paths = BTreeMap::new();
    for url in presentation_paths.values() {
        let url = url.as_str().ok_or("Export presentation URL is required.")?;
        let file = url::Url::parse(url)
            .map_err(|error| error.to_string())?
            .to_file_path()
            .map_err(|_| "Export presentation URL must be local.")?;
        let relative = file
            .strip_prefix(destination)
            .map_err(|_| "Export presentation must remain inside the workspace.")?;
        relative_paths.insert(
            url.to_string(),
            relative.to_string_lossy().replace('\\', "/"),
        );
    }
    Ok(encode_playlist_document(
        &json!({"id":Uuid::new_v4().to_string(),"title":"Library","type":"root","children":roots}),
        presentation_paths,
        &relative_paths,
    ))
}

fn encode_playlist_document(
    playlist: &Value,
    presentation_paths: &Map<String, Value>,
    relative_paths: &BTreeMap<String, String>,
) -> Vec<u8> {
    let mut document = Vec::new();
    put_varint(2, 1, &mut document); // PlaylistDocument.TYPE_PRESENTATION
    put_message(
        3,
        encode_playlist_node(playlist, presentation_paths, relative_paths),
        &mut document,
    );
    document
}

fn encode_playlist_url(path: &str, relative_paths: &BTreeMap<String, String>) -> Vec<u8> {
    let mut url = Vec::new();
    put_string(1, path, &mut url);
    if let Some(path) = relative_paths.get(path) {
        let mut local = Vec::new();
        put_varint(1, 10, &mut local); // URL.LocalRelativePath.ROOT_SHOW
        put_string(2, path, &mut local);
        put_message(4, local, &mut url);
    }
    url
}

fn encode_playlist_node(
    playlist: &Value,
    presentation_paths: &Map<String, Value>,
    relative_paths: &BTreeMap<String, String>,
) -> Vec<u8> {
    let mut root = Vec::new();
    if let Some(id) = playlist
        .get("sourceId")
        .or_else(|| playlist.get("id"))
        .and_then(Value::as_str)
    {
        put_uuid(1, id, &mut root);
    }
    put_string(
        2,
        playlist
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Playlist"),
        &mut root,
    );
    let folder = playlist["type"] == "folder" || playlist["type"] == "root";
    put_varint(
        3,
        if playlist["type"] == "root" {
            4
        } else if folder {
            2
        } else {
            1
        },
        &mut root,
    );
    if let Some(expanded) = playlist["expanded"].as_bool() {
        put_varint(4, u64::from(expanded), &mut root);
    }
    if let Some(enabled) = playlist["timecodeEnabled"].as_bool() {
        put_varint(10, u64::from(enabled), &mut root);
    }
    if let Some(timing) = playlist["timing"].as_u64() {
        put_varint(11, timing, &mut root);
    }
    if let Some(trigger) = playlist["startupTrigger"].as_bool() {
        let mut startup = Vec::new();
        put_varint(1, u64::from(trigger), &mut startup);
        put_message(16, startup, &mut root);
    }
    if folder {
        let mut children = Vec::new();
        for child in playlist["children"].as_array().into_iter().flatten() {
            put_message(
                1,
                encode_playlist_node(child, presentation_paths, relative_paths),
                &mut children,
            );
        }
        put_message(12, children, &mut root);
        return root;
    }
    let mut items = Vec::new();
    if let Some(entries) = playlist.get("items").and_then(Value::as_array) {
        for entry in entries {
            let mut item = Vec::new();
            if let Some(id) = entry.get("id").and_then(Value::as_str) {
                put_uuid(1, id, &mut item);
            }
            put_string(
                2,
                entry
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Playlist Item"),
                &mut item,
            );
            if let Some(hidden) = entry
                .get("hidden")
                .or_else(|| entry.get("isHidden"))
                .and_then(Value::as_bool)
            {
                put_varint(9, u64::from(hidden), &mut item);
            }
            if entry.get("type").and_then(Value::as_str) == Some("header") {
                let mut header = Vec::new();
                if let Some(color) = entry["color"].as_str() {
                    put_color(1, color, &mut header);
                }
                for action in entry["actions"].as_array().into_iter().flatten() {
                    if let Ok(action) = marker_action_export::encode_marker_action(action) {
                        put_message(2, action, &mut header);
                    }
                }
                put_message(3, header, &mut item);
            } else if entry["type"] == "placeholder" {
                let mut placeholder = Vec::new();
                if let Some(path) = entry["linkedItemId"]
                    .as_str()
                    .and_then(|id| presentation_paths.get(id))
                    .and_then(Value::as_str)
                {
                    let url = encode_playlist_url(path, relative_paths);
                    let mut presentation = Vec::new();
                    put_message(1, url, &mut presentation);
                    let mut linked = Vec::new();
                    put_uuid(1, "", &mut linked);
                    put_message(4, presentation, &mut linked);
                    put_message(1, linked, &mut placeholder);
                }
                put_message(8, placeholder, &mut item);
            } else if let Some(item_id) = entry.get("itemId").and_then(Value::as_str) {
                if let Some(path) = presentation_paths.get(item_id).and_then(Value::as_str) {
                    let url = encode_playlist_url(path, relative_paths);
                    let mut presentation = Vec::new();
                    put_message(1, url, &mut presentation);
                    if let Some(arrangement) = entry.get("arrangementId").and_then(Value::as_str) {
                        put_uuid(
                            2,
                            &cue_export::arrangement_uuid(item_id, arrangement),
                            &mut presentation,
                        );
                    }
                    if let Some(destination) = entry["contentDestination"].as_u64() {
                        put_varint(3, destination, &mut presentation);
                    }
                    put_message(4, presentation, &mut item);
                }
            }
            put_message(1, item, &mut items);
        }
    }
    put_message(13, items, &mut root);
    root
}

#[path = "propresenter_proto/layer_media_export.rs"]
mod layer_media_export;

#[path = "propresenter_proto/layer_link_export.rs"]
mod layer_link_export;

#[path = "propresenter_proto/cue_export.rs"]
pub(crate) mod cue_export;
#[path = "propresenter_proto/layer_text_settings_export.rs"]
mod layer_text_settings_export;

#[path = "propresenter_proto/marker_action_export.rs"]
mod marker_action_export;
#[path = "propresenter_proto/media_bin_export.rs"]
pub(crate) mod media_bin_export;

#[path = "propresenter_proto/layer_build_export.rs"]
pub(crate) mod layer_build_export;

#[path = "propresenter_proto/layer_text_paint_export.rs"]
mod layer_text_paint_export;

#[path = "propresenter_proto/layer_export.rs"]
pub(crate) mod layer_export;

#[path = "propresenter_proto/screen_config.rs"]
pub(crate) mod screen_config;
