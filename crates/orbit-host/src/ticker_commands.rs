use super::{
    active_workspace, argument, expected_workspace_revision, lock_workspace_io, workspace, AppState,
};
use crate::native_ticker::{self, TickerAction};
use serde_json::{json, Value};
use std::time::Instant;

pub fn control(app: &Host, state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let _io = lock_workspace_io(state)?;
    _io.require_active(&path)?;
    let document = workspace::read_workspace(&path)?;
    let tickers = native_ticker::validate_collection(
        document
            .get("tickers")
            .ok_or("Workspace tickers are required.")?,
    )?;
    let action: TickerAction = serde_json::from_value(argument(args, 1)?)
        .map_err(|error| format!("Invalid ticker action: {error}"))?;
    let ticker = native_ticker::runtime()
        .lock()
        .map_err(|_| "Ticker service is unavailable.")?
        .control(&path, &tickers, action, Instant::now())?;
    resolve(app, state, ticker)
}
pub fn status(app: &Host, state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let _io = lock_workspace_io(state)?;
    _io.require_active(&path)?;
    resolve(app, state, native_ticker::snapshot(&path)?)
}
use crate::Host;
pub fn save(state: &AppState, args: &[Value]) -> Result<Value, String> {
    let path = active_workspace(state, argument(args, 0)?)?;
    let tickers = argument(args, 1)?;
    let definitions = native_ticker::validate_collection(&tickers)?;
    let mut io = lock_workspace_io(state)?;
    io.require_revision(&path, expected_workspace_revision(args, 2)?)?;
    let mut runtime = native_ticker::runtime()
        .lock()
        .map_err(|_| "Ticker service is unavailable.")?;
    let saved = workspace::save_workspace(&path, &json!({"tickers":tickers}))?;
    runtime.reconcile(&path, &definitions);
    Ok(json!({"workspace":saved,"revision":io.advance_revision(&path)}))
}

pub(crate) fn resolve(app: &Host, state: &AppState, ticker: Value) -> Result<Value, String> {
    let cache = state
        .rss_output
        .lock()
        .map_err(|_| "Output RSS state is unavailable.")?
        .snapshot();
    let (ticker, pending) = crate::rss_output::resolve_ticker(ticker, &cache);
    crate::schedule_output_rss_refreshes(app, pending);
    Ok(ticker)
}

pub(crate) fn execute_remote(
    app: &Host,
    state: &AppState,
    payload: &Value,
) -> Result<Value, String> {
    let mut action = payload
        .as_object()
        .ok_or("Ticker command must be an object.")?
        .clone();
    action.remove("command");
    let workspace = action
        .remove("workspacePath")
        .ok_or("Ticker workspacePath is required.")?;
    let result = control(app, state, &[workspace, Value::Object(action)]);
    Ok(match result {
        Ok(ticker) => json!({"ok":true,"status":"applied","changed":true,"ticker":ticker}),
        Err(error) => json!({"ok":false,"status":"invalid-target","changed":false,"error":error}),
    })
}
