//! Ephemeral linked-file contents; never merge these updates into persisted workspace documents.
use super::runtime::Runtime;
use gpui::{App, AppContext};
use serde_json::{json, Value};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

#[derive(Default)]
pub struct TextFiles {
    pub entries: Arc<Value>,
    pub preview: Arc<Value>,
    pub error: String,
}
fn file_style(value: &Value) -> Option<&str> {
    (value["linkedTextSource"] == "File" && value["linkedTextEnabled"] != false)
        .then(|| value["linkedTextFilePath"].as_str())
        .flatten()
        .filter(|path| !path.is_empty())
}
fn paths(value: &Value, result: &mut BTreeSet<String>) {
    match value {
        Value::Object(fields) => {
            if let Some(path) = file_style(value) {
                result.insert(path.to_string());
            }
            for child in fields.values() {
                paths(child, result);
            }
        }
        Value::Array(values) => {
            for child in values {
                paths(child, result);
            }
        }
        _ => {}
    }
}
pub fn overlay(value: &mut Value, entries: &Value) {
    if entries.as_array().is_none_or(Vec::is_empty) || (!value.is_object() && !value.is_array()) {
        return;
    }
    if let Some(entry) = file_style(value).and_then(|path| {
        entries
            .as_array()?
            .iter()
            .find(|entry| entry["filePath"] == path)
    }) {
        value["linkedTextFileContent"] = entry["text"].clone();
        value["linkedTextFileError"] = entry["error"].clone();
    }
    match value {
        Value::Object(fields) => {
            for child in fields.values_mut() {
                overlay(child, entries);
            }
        }
        Value::Array(values) => {
            for child in values {
                overlay(child, entries);
            }
        }
        _ => {}
    }
}
pub fn start(runtime: &Runtime, cx: &mut App) {
    let files = runtime.text_files.downgrade();
    let content = runtime.content.downgrade();
    let output = runtime.output.downgrade();
    let host = runtime.host.clone();
    cx.spawn(async move |cx| {
        let mut signature = (String::new(), Vec::<String>::new());
        let mut watch = None;
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let request = cx.update(|cx| {
                let content = content.upgrade()?;
                let output = output.upgrade()?;
                let files = files.upgrade()?;
                Some((
                    content.read(cx).workspace_path.clone(),
                    content.read(cx).workspace.clone(),
                    output.read(cx).live.clone(),
                    files.read(cx).preview.clone(),
                ))
            });
            let Some((workspace, document, live, preview)) = request else {
                break;
            };
            let request = cx
                .background_spawn(async move {
                    let mut paths_set = BTreeSet::new();
                    paths(&document, &mut paths_set);
                    paths(&live, &mut paths_set);
                    paths(&preview, &mut paths_set);
                    (workspace, paths_set.into_iter().collect::<Vec<_>>())
                })
                .await;
            let changed = request != signature;
            signature = request.clone();
            let host_call = host.clone();
            let prior = watch.take();
            let requested_workspace = request.0.clone();
            let (id, entries) = cx
                .background_spawn(async move {
                    let mut id = prior;
                    let entries = (|| -> Result<Value, String> {
                        if changed {
                            if let Some(prior) = id {
                                host_call.invoke("stopLocalTextWatch", vec![json!(prior)])?;
                                id = None;
                            }
                            if !request.0.is_empty() && !request.1.is_empty() {
                                id = Some(
                                    host_call
                                        .invoke(
                                            "startLocalTextWatch",
                                            vec![json!(request.0), json!(request.1)],
                                        )?
                                        .as_u64()
                                        .ok_or("Invalid local text watch identity.")?,
                                );
                            }
                        }
                        match id {
                            Some(id) => host_call.invoke("readLocalTextWatch", vec![json!(id)]),
                            None => Ok(json!([])),
                        }
                    })();
                    (id, entries)
                })
                .await;
            watch = id;
            if changed && entries.is_err() {
                signature = (String::new(), Vec::new());
            }
            cx.update(|cx| {
                let Some(content) = content.upgrade() else {
                    return;
                };
                if content.read(cx).workspace_path != requested_workspace {
                    return;
                }
                let Some(files) = files.upgrade() else {
                    return;
                };
                let changed = files.update(cx, |files, cx| {
                    let (entries, error) = match entries {
                        Ok(entries) => {
                            let error = entries
                                .as_array()
                                .into_iter()
                                .flatten()
                                .find_map(|entry| entry["error"].as_str())
                                .unwrap_or("")
                                .to_string();
                            (Arc::new(entries), error)
                        }
                        Err(error) => (Arc::new(json!([])), error),
                    };
                    let changed = entries != files.entries || error != files.error;
                    if changed {
                        files.entries = entries;
                        files.error = error;
                        cx.notify();
                    }
                    changed
                });
                if changed {
                    if let Some(output) = output.upgrade() {
                        output.update(cx, |output, cx| {
                            output.edit_live(|_| {});
                            cx.notify();
                        });
                    }
                }
            });
        }
        if let Some(id) = watch {
            cx.background_spawn(async move { host.invoke("stopLocalTextWatch", vec![json!(id)]) })
                .await
                .ok();
        }
    })
    .detach();
}
