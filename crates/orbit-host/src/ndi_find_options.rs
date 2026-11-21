//! NDI source-finder options (`NDIlib_find_create_t`) stored in the general settings.

use serde_json::{json, Map, Value};
use std::net::IpAddr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindOptions {
    pub show_local_sources: bool,
    pub groups: String,
    pub extra_ips: String,
}

impl Default for FindOptions {
    fn default() -> Self {
        Self {
            show_local_sources: true,
            groups: String::new(),
            extra_ips: String::new(),
        }
    }
}

fn parts(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

fn is_hostname(value: &str) -> bool {
    value.len() <= 253
        && value.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

/// Comma-separated group names without blanks; control characters are rejected.
pub fn normalize_groups(value: &str) -> Option<String> {
    let groups: Vec<&str> = parts(value).collect();
    if groups
        .iter()
        .any(|group| group.chars().any(char::is_control))
    {
        return None;
    }
    Some(groups.join(","))
}

/// Comma-separated IPv4/IPv6 addresses or plain hostnames; `None` when an entry is neither.
pub fn normalize_extra_ips(value: &str) -> Option<String> {
    let hosts: Vec<&str> = parts(value).collect();
    hosts
        .iter()
        .all(|host| host.parse::<IpAddr>().is_ok() || is_hostname(host))
        .then(|| hosts.join(","))
}

pub fn normalize_into(configured: &Map<String, Value>, normalized: &mut Map<String, Value>) {
    let text = |key: &str| configured.get(key).and_then(Value::as_str).unwrap_or("");
    normalized.insert(
        "ndiShowLocalSources".to_string(),
        json!(configured
            .get("ndiShowLocalSources")
            .and_then(Value::as_bool)
            .unwrap_or(true)),
    );
    normalized.insert(
        "ndiGroups".to_string(),
        json!(normalize_groups(text("ndiGroups")).unwrap_or_default()),
    );
    normalized.insert(
        "ndiExtraIps".to_string(),
        json!(normalize_extra_ips(text("ndiExtraIps")).unwrap_or_default()),
    );
}

/// Reads the options from normalized general settings.
pub fn from_settings(general: &Value) -> FindOptions {
    let text = |key: &str| general.get(key).and_then(Value::as_str).unwrap_or("");
    FindOptions {
        show_local_sources: general["ndiShowLocalSources"] != false,
        groups: text("ndiGroups").to_string(),
        extra_ips: text("ndiExtraIps").to_string(),
    }
}
