//! Resolved, credential-free linked text from the controller. No workspace or settings access.
use serde_json::{json, Value};
use std::{sync::Mutex, time::Instant};
#[derive(Default)]
pub struct LinkedServices(Mutex<Option<(Value, Instant)>>);
impl LinkedServices {
    pub fn accept(&self, value: Value) -> Result<Value, String> {
        if value
            .as_object()
            .is_none_or(|snapshot| snapshot.len() != 1 || !snapshot.contains_key("ticker"))
        {
            return Err("Invalid linked service snapshot.".into());
        }
        if !value["ticker"].is_null() {
            let mut definition = value["ticker"].clone();
            for key in ["generation", "elapsedSeconds", "paused", "sourceError"] {
                definition
                    .as_object_mut()
                    .ok_or("Invalid ticker snapshot.")?
                    .remove(key);
            }
            crate::native_ticker::validate_collection(&json!([definition]))?;
            if value["ticker"]["elapsedSeconds"]
                .as_f64()
                .is_none_or(|n| !n.is_finite() || n < 0.)
                || !value["ticker"]["paused"].is_boolean()
            {
                return Err("Invalid ticker clock.".into());
            }
        }
        *self
            .0
            .lock()
            .map_err(|_| "Linked service state is unavailable.")? = Some((value, Instant::now()));
        Ok(Value::Null)
    }
    pub fn snapshot(&self) -> Result<Option<Value>, String> {
        let state = self
            .0
            .lock()
            .map_err(|_| "Linked service state is unavailable.")?;
        Ok(state
            .as_ref()
            .map(|(v, at)| advance(v, at.elapsed().as_secs_f64())))
    }
}
fn advance(value: &Value, seconds: f64) -> Value {
    let mut v = value.clone();
    if v["ticker"]["paused"] == false {
        if let Some(elapsed) = v["ticker"]["elapsedSeconds"].as_f64() {
            v["ticker"]["elapsedSeconds"] = json!(elapsed + seconds);
        }
    }
    v
}
