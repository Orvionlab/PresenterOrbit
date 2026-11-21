use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub name: String,
    pub path: String,
}

pub fn parse(value: &Value) -> Result<Vec<Entry>, String> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(value.clone())
        .map_err(|error| format!("Workspace registry is invalid: {error}"))
}

pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Workspace name is required.".to_string());
    }
    if name == "." || name == ".." || name.ends_with('.') {
        return Err("Workspace name is not valid.".to_string());
    }
    if name
        .chars()
        .any(|character| character.is_control() || "<>:\"/\\|?*".contains(character))
    {
        return Err("Workspace name cannot contain / \\ < > : \" | ? * characters.".to_string());
    }
    Ok(name.to_string())
}

fn name_taken(entries: &[Entry], name: &str, except_path: Option<&str>) -> bool {
    entries.iter().any(|entry| {
        Some(entry.path.as_str()) != except_path && entry.name.to_lowercase() == name.to_lowercase()
    })
}

pub fn require_unique_name(
    entries: &[Entry],
    name: &str,
    except_path: Option<&str>,
) -> Result<(), String> {
    if name_taken(entries, name, except_path) {
        return Err(format!("A workspace named \"{name}\" already exists."));
    }
    Ok(())
}

fn unique_name(entries: &[Entry], base: &str) -> String {
    let mut candidate = base.to_string();
    let mut suffix = 2;
    while name_taken(entries, &candidate, None) {
        candidate = format!("{base} {suffix}");
        suffix += 1;
    }
    candidate
}

/// Adds the workspace under a unique variant of `base_name` unless its path is already registered.
pub fn register(entries: &mut Vec<Entry>, path: &str, base_name: &str) {
    if entries.iter().any(|entry| entry.path == path) {
        return;
    }
    let name = unique_name(entries, base_name);
    entries.push(Entry {
        name,
        path: path.to_string(),
    });
}

pub fn add(entries: &mut Vec<Entry>, name: &str, path: &str) -> Result<(), String> {
    require_unique_name(entries, name, None)?;
    entries.push(Entry {
        name: name.to_string(),
        path: path.to_string(),
    });
    Ok(())
}

pub fn rename(entries: &mut [Entry], path: &str, name: &str) -> Result<(), String> {
    require_unique_name(entries, name, Some(path))?;
    let entry = entries
        .iter_mut()
        .find(|entry| entry.path == path)
        .ok_or_else(|| "Workspace is not registered.".to_string())?;
    entry.name = name.to_string();
    Ok(())
}

pub fn forget(
    entries: &mut Vec<Entry>,
    path: &str,
    active_path: Option<&str>,
) -> Result<(), String> {
    if Some(path) == active_path {
        return Err("The active workspace cannot be removed.".to_string());
    }
    let before = entries.len();
    entries.retain(|entry| entry.path != path);
    if entries.len() == before {
        return Err("Workspace is not registered.".to_string());
    }
    Ok(())
}

/// Copies a workspace folder, skipping an interrupted write transaction.
pub fn copy_folder(source: &Path, target: &Path, skip: &str) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.file_name() == skip {
            continue;
        }
        let destination = target.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            copy_folder(&entry.path(), &destination, skip)?;
        } else {
            fs::copy(entry.path(), destination).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
