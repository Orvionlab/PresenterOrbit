use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

/// The normalized item shape shared by the editor bridge and native output.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RssItem {
    pub title: String,
    pub description: String,
}

/// Fetch an RSS or Atom feed through the cross-platform Rust HTTP client.
///
/// The renderer consumes only the normalized `items` array. Keeping the HTTP
/// request in the Tauri host means this works on Windows, macOS, and Linux
/// without requiring an operating-system HTTP executable.
pub fn fetch_feed(raw_url: &str) -> Result<Value, String> {
    Ok(json!({ "items": fetch_items(raw_url)? }))
}

/// Fetch and normalize an RSS or Atom feed for native consumers.
pub fn fetch_items(raw_url: &str) -> Result<Vec<RssItem>, String> {
    let url = validate_url(raw_url)?;
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .max_redirects(5)
        .max_redirects_will_error(true)
        .user_agent("Presenter Orbit RSS/1.0")
        .build()
        .new_agent();
    let mut response = agent
        .get(url.as_str())
        .call()
        .map_err(|error| format!("RSS request failed: {error}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("RSS request failed with HTTP status {status}."));
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|error| format!("RSS response could not be read: {error}"))?;
    let body =
        String::from_utf8(body).map_err(|_| "RSS response is not valid UTF-8.".to_string())?;
    Ok(parse_items(&body))
}

pub fn validate_url(raw_url: &str) -> Result<Url, String> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return Err("RSS URL must not be empty.".to_string());
    }
    let parsed = Url::parse(trimmed).map_err(|error| format!("Invalid RSS URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("RSS URL must use HTTP or HTTPS.".to_string());
    }
    if parsed.host_str().is_none() {
        return Err("RSS URL must include a host.".to_string());
    }
    if parsed.username() != "" || parsed.password().is_some() {
        return Err("RSS URL credentials are not supported.".to_string());
    }
    Ok(parsed)
}

fn parse_items(xml: &str) -> Vec<RssItem> {
    let mut items = Vec::new();
    let mut offset = 0;
    while let Some((_, tag_end, tag_name)) = find_element(xml, offset, &["item", "entry"]) {
        let close = format!("</{tag_name}");
        let lower = xml.to_ascii_lowercase();
        let Some(end) = lower[tag_end..].find(&close).map(|index| tag_end + index) else {
            break;
        };
        let block = &xml[tag_end..end];
        let title = decode_xml_field(read_xml_field(block, &["title"]));
        let description = decode_xml_field(read_xml_field(
            block,
            &["description", "summary", "content", "encoded"],
        ));
        if !title.is_empty() || !description.is_empty() {
            items.push(RssItem { title, description });
        }
        offset = end + close.len();
    }
    items
}

fn find_element(xml: &str, offset: usize, names: &[&str]) -> Option<(usize, usize, String)> {
    let lower = xml.to_ascii_lowercase();
    let mut cursor = offset;
    while let Some(relative_start) = lower[cursor..].find('<') {
        let start = cursor + relative_start;
        let tag_end = lower[start..].find('>')? + start;
        let tag = lower[start + 1..tag_end].trim_start();
        if !tag.starts_with('/') && !tag.starts_with('!') && !tag.starts_with('?') {
            let name = tag.split_whitespace().next()?.trim_end_matches('/');
            if names.iter().any(|candidate| local_name(name) == *candidate) {
                return Some((start, tag_end + 1, name.to_string()));
            }
        }
        cursor = tag_end + 1;
    }
    None
}

fn read_xml_field<'a>(block: &'a str, names: &[&str]) -> Option<&'a str> {
    let mut cursor = 0;
    while let Some((_, content_start, tag_name)) = find_element(block, cursor, names) {
        let lower = block.to_ascii_lowercase();
        let close = format!("</{tag_name}");
        if let Some(content_end) = lower[content_start..]
            .find(&close)
            .map(|index| content_start + index)
        {
            return Some(&block[content_start..content_end]);
        }
        cursor = content_start;
    }
    None
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn decode_xml_field(value: Option<&str>) -> String {
    let value = value.unwrap_or_default();
    let value = value
        .replace("<![CDATA[", "")
        .replace("]]>", "")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    let mut text = String::new();
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    decode_numeric_entities(&text)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn decode_numeric_entities(value: &str) -> String {
    let mut decoded = String::new();
    let mut remaining = value;
    while let Some(start) = remaining.find("&#") {
        decoded.push_str(&remaining[..start]);
        let entity = &remaining[start + 2..];
        let Some(end) = entity.find(';') else {
            decoded.push_str(&remaining[start..]);
            return decoded;
        };
        let (radix, digits) = entity[..end]
            .strip_prefix(['x', 'X'])
            .map(|digits| (16, digits))
            .unwrap_or((10, &entity[..end]));
        if let Ok(codepoint) = u32::from_str_radix(digits, radix) {
            if let Some(character) = char::from_u32(codepoint) {
                decoded.push(character);
                remaining = &entity[end + 1..];
                continue;
            }
        }
        decoded.push_str(&remaining[start..start + 2 + end + 1]);
        remaining = &entity[end + 1..];
    }
    decoded.push_str(remaining);
    decoded
}
