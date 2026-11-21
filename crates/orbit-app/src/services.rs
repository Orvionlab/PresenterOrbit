//! Controller-side feature calls. Workspace writers use the revision lane; output clocks stay in the output process.
use crate::state::runtime::Runtime;

/// Forward the controller ticker status to the output process.
pub fn start(runtime: Runtime, cx: &mut gpui::App) {
    cx.spawn(async move |cx| loop {
        let path = cx.update(|cx| runtime.content.read(cx).workspace_path.clone());
        let host = runtime.host.clone();
        let request_path = path.clone();
        let result = cx
            .background_spawn(async move {
                if request_path.is_empty() {
                    Ok(Value::Null)
                } else {
                    host.invoke("getTickerStatus", vec![json!(request_path)])
                }
            })
            .await;
        match result {
            Ok(ticker) => {
                if cx.update(|cx| runtime.content.read(cx).workspace_path == path) {
                    let host = runtime.host.clone();
                    if let Err(error) = cx
                        .background_spawn(async move {
                            crate::state::runtime::output_call(
                                &host,
                                "acceptLinkedServiceState",
                                vec![json!({"ticker":ticker})],
                            )
                        })
                        .await
                    {
                        cx.update(|cx| {
                            runtime.status.update(cx, |status, cx| {
                                status.message = error;
                                cx.notify();
                            })
                        });
                    }
                }
            }
            Err(error) => cx.update(|cx| {
                runtime.status.update(cx, |status, cx| {
                    status.message = error;
                    cx.notify();
                })
            }),
        }
        cx.background_executor()
            .timer(std::time::Duration::from_millis(250))
            .await;
    })
    .detach();
}
use gpui::{AppContext, AsyncApp};
use serde_json::{json, Value};

pub async fn call(
    runtime: &Runtime,
    cx: &AsyncApp,
    method: &'static str,
    args: Vec<Value>,
    write: bool,
) -> Result<Value, String> {
    call_after(runtime, cx, method, args, write, Vec::new()).await
}

/// [`call`] with arguments that follow the revision a write carries, such as the duplicate choice of a bundle
/// import.
pub async fn call_after(
    runtime: &Runtime,
    cx: &AsyncApp,
    method: &'static str,
    mut args: Vec<Value>,
    write: bool,
    trailing: Vec<Value>,
) -> Result<Value, String> {
    let path = cx.update(|cx| runtime.content.read(cx).workspace_path.clone());
    let retired = std::rc::Rc::new(std::cell::Cell::new(false));
    let watching = retired.clone();
    let scope = path.clone();
    let _subscription = cx.update(|cx| {
        cx.observe(&runtime.content, move |content, cx| {
            if content.read(cx).workspace_path != scope {
                watching.set(true);
            }
        })
    });
    let host = runtime.host.clone();
    let answer = if write {
        runtime
            .lanes
            .workspace(&path, move |revision| {
                args.push(json!(revision));
                args.extend(trailing);
                host.invoke(method, args)
            })
            .await
            .map_err(|_| "The workspace queue stopped.".to_string())?
    } else {
        cx.background_spawn(async move { host.invoke(method, args) })
            .await
    }?;
    if retired.get() || cx.update(|cx| runtime.content.read(cx).workspace_path != path) {
        return Err("The workspace changed while the request was running.".into());
    }
    // An import that stopped to ask about duplicate titles changed nothing, so nothing is reloaded.
    if write && answer["duplicates"].is_null() {
        let snapshot = runtime.read_snapshot(cx, &path).await?;
        if retired.get() {
            return Err("The workspace changed while the request was running.".into());
        }
        cx.update(|cx| {
            if !retired.get() && runtime.content.read(cx).workspace_path == path {
                runtime.content.update(cx, |content, cx| {
                    content.apply_canonical(snapshot);
                    cx.notify();
                });
            }
        });
    }
    Ok(answer)
}

pub fn entries(value: &Value) -> Vec<Value> {
    value.as_array().cloned().unwrap_or_default()
}
pub fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").to_string()
}
pub fn choices(value: &Value, label: &str) -> Vec<(String, String)> {
    entries(value)
        .iter()
        .map(|v| (text(v, "id"), text(v, label)))
        .collect()
}
