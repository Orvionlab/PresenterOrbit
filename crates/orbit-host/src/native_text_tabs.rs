//! Margin-based tab stops shared with the renderer. Each tab selects the first stop beyond the
//! current text cursor; the default interval continues beyond the last authored stop.

use serde_json::{Map, Value};

pub(crate) const STOPS_FIELD: &str = "textTabStops";
pub(crate) const BAR_STOPS_FIELD: &str = "textBarTabStops";
pub(crate) const INTERVAL_FIELD: &str = "textDefaultTabInterval";
const MAX_STOPS: usize = 32;
const MAX_POSITION: f64 = 2000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabAlignment {
    Left,
    Right,
    Center,
    Decimal,
    Justify,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TabStop {
    pub position: f64,
    pub alignment: TabAlignment,
    pub leader: Option<&'static str>,
}

/// Authored tab settings in slide pixels; they are active when there is a custom stop or a
/// default interval.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabStops {
    stops: Vec<TabStop>,
    interval: Option<f64>,
}

pub(crate) fn parse_tab_stops(value: &Value) -> Result<Vec<TabStop>, String> {
    let entries = value
        .as_array()
        .ok_or(format!("{STOPS_FIELD} must be a list of tab stops."))?;
    if entries.len() > MAX_STOPS {
        return Err(format!(
            "{STOPS_FIELD} holds at most {MAX_STOPS} tab stops."
        ));
    }
    let mut stops: Vec<TabStop> = Vec::new();
    for entry in entries {
        let object = entry
            .as_object()
            .filter(|object| {
                object
                    .keys()
                    .all(|key| matches!(key.as_str(), "position" | "alignment" | "leader"))
            })
            .ok_or(format!(
                "{STOPS_FIELD} entries must be objects with position and alignment."
            ))?;
        let position = object
            .get("position")
            .and_then(Value::as_f64)
            .filter(|position| (0.0..=MAX_POSITION).contains(position))
            .ok_or(format!(
                "{STOPS_FIELD} positions must be numbers between 0 and {MAX_POSITION}."
            ))?;
        let alignment = match object.get("alignment").and_then(Value::as_str) {
            Some("left") => TabAlignment::Left,
            Some("right") => TabAlignment::Right,
            Some("center") => TabAlignment::Center,
            Some("decimal") => TabAlignment::Decimal,
            Some("justify") => TabAlignment::Justify,
            _ => {
                return Err(format!(
                    "{STOPS_FIELD} alignment must be left, right, center, decimal or justify."
                ))
            }
        };
        let leader = match object.get("leader") {
            None => None,
            Some(Value::String(value)) => Some(match value.as_str() {
                "dot" => "dot",
                "equal" => "equal",
                "hyphen" => "hyphen",
                "thick" => "thick",
                "underline" => "underline",
                _ => return Err(format!("{STOPS_FIELD} leader is invalid.")),
            }),
            Some(_) => return Err(format!("{STOPS_FIELD} leader is invalid.")),
        };
        if stops.last().is_some_and(|last| position <= last.position) {
            return Err(format!("{STOPS_FIELD} positions must strictly increase."));
        }
        stops.push(TabStop {
            position,
            alignment,
            leader,
        });
    }
    Ok(stops)
}

/// The tab setting outside the supported range, if any. The default interval is checked with the
/// other bounded scalar fields.
pub(crate) fn invalid_tab_stops(object: &Map<String, Value>) -> Option<(&'static str, String)> {
    let reason = parse_tab_stops(object.get(STOPS_FIELD)?).err()?;
    Some((STOPS_FIELD, reason))
}

pub(crate) fn parse_bar_stops(value: &Value) -> Result<Vec<f64>, String> {
    let entries = value
        .as_array()
        .filter(|entries| entries.len() <= MAX_STOPS)
        .ok_or(format!(
            "{BAR_STOPS_FIELD} must be an array of at most {MAX_STOPS} positions."
        ))?;
    let mut positions = Vec::new();
    for entry in entries {
        let position = entry
            .as_f64()
            .filter(|value| value.is_finite() && (0.0..=MAX_POSITION).contains(value))
            .ok_or(format!(
                "{BAR_STOPS_FIELD} positions must be numbers between 0 and {MAX_POSITION}."
            ))?;
        if positions.last().is_some_and(|last| position <= *last) {
            return Err(format!(
                "{BAR_STOPS_FIELD} positions must strictly increase."
            ));
        }
        positions.push(position);
    }
    Ok(positions)
}

impl TabStops {
    pub fn from_style(style: &Map<String, Value>) -> Option<Self> {
        let stops = style
            .get(STOPS_FIELD)
            .and_then(|value| parse_tab_stops(value).ok())
            .unwrap_or_default();
        let interval = style.get(INTERVAL_FIELD).and_then(Value::as_f64);
        (!stops.is_empty() || interval.is_some()).then_some(Self { stops, interval })
    }

    /// The first authored stop beyond the cursor, then regular stops beyond the last authored one.
    pub fn next_after(&self, cursor: f64) -> Option<TabStop> {
        if let Some(stop) = self.stops.iter().find(|stop| stop.position > cursor) {
            return Some(*stop);
        }
        let interval = self.interval?;
        let last = self.stops.last().map_or(0.0, |stop| stop.position);
        let multiple = (cursor.max(last) / interval).floor() + 1.0;
        Some(TabStop {
            position: multiple * interval,
            alignment: TabAlignment::Left,
            leader: None,
        })
    }
}
