use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum TickerSource {
    File {
        path: String,
    },
    Rss {
        url: String,
        format: String,
        delimiter: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TickerDefinition {
    pub id: String,
    pub title: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<TickerSource>,
    pub speed: f64,
    pub direction: String,
    pub repeat: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_delay_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_distance: Option<f64>,
}

pub fn validate_collection(value: &Value) -> Result<Vec<TickerDefinition>, String> {
    let definitions: Vec<TickerDefinition> = serde_json::from_value(value.clone())
        .map_err(|error| format!("Invalid ticker collection: {error}"))?;
    let mut ids = HashSet::new();
    for entry in &definitions {
        match &entry.source {
            Some(TickerSource::File { path })
                if !Path::new(path).is_absolute()
                    || !Path::new(path)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt")) =>
            {
                return Err("Ticker file must be an absolute .txt path.".into())
            }
            Some(TickerSource::Rss {
                url,
                format,
                delimiter,
            }) => {
                crate::rss::validate_url(url)?;
                if !matches!(format.as_str(), "title" | "titleDescription")
                    || delimiter.len() > 1024
                {
                    return Err("Ticker RSS format or delimiter is invalid.".into());
                }
            }
            _ => {}
        }
        if entry.id.trim().is_empty()
            || !ids.insert(entry.id.clone())
            || entry.title.trim().is_empty()
            || (entry.source.is_none() && entry.text.trim().is_empty())
            || entry.text.len() > 65_536
            || !entry.speed.is_finite()
            || !(1.0..=100.0).contains(&entry.speed)
            || !matches!(entry.direction.as_str(), "left" | "right" | "up" | "down")
            || entry.loop_delay_seconds.is_some_and(|delay| {
                !entry.repeat || !delay.is_finite() || delay <= 0.0 || delay > 3600.0
            })
            || entry
                .start_position
                .as_deref()
                .is_some_and(|start| !matches!(start, "automatic" | "offscreen"))
            || entry.repeat_distance.is_some_and(|distance| {
                !distance.is_finite() || !(0.0..=10000.0).contains(&distance)
            })
        {
            return Err(
                "Ticker requires unique id, title, text, speed 1–100 and a supported direction."
                    .into(),
            );
        }
    }
    Ok(definitions)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum TickerAction {
    Play { id: String },
    Pause { generation: u64 },
    Resume { generation: u64 },
    Clear { generation: u64 },
    Seek { generation: u64, seconds: f64 },
}

struct FileSource {
    watch: u64,
    initial: String,
}
impl Drop for FileSource {
    fn drop(&mut self) {
        let _ = file_service().stop(self.watch);
    }
}
fn file_service() -> &'static crate::local_text::LocalTextService {
    static SERVICE: OnceLock<crate::local_text::LocalTextService> = OnceLock::new();
    SERVICE.get_or_init(crate::local_text::LocalTextService::default)
}
fn watch_source(source: &Option<TickerSource>) -> Result<Option<FileSource>, String> {
    match source {
        Some(TickerSource::File { path }) => {
            let initial = crate::local_text::read_text(Path::new(path))?;
            validate_source_text(&initial)?;
            Ok(Some(FileSource {
                watch: file_service().start(vec![path.clone()])?,
                initial,
            }))
        }
        _ => Ok(None),
    }
}
fn validate_source_text(text: &str) -> Result<(), String> {
    if text.len() > 65_536 {
        Err("Ticker source exceeds 65536 bytes.".into())
    } else {
        Ok(())
    }
}

struct Run {
    file: Option<FileSource>,
    source_error: Option<String>,
    workspace: PathBuf,
    definition: TickerDefinition,
    generation: u64,
    elapsed: f64,
    started: Option<Instant>,
}

impl Run {
    fn seconds(&self, now: Instant) -> f64 {
        self.elapsed
            + self
                .started
                .map_or(0.0, |start| now.duration_since(start).as_secs_f64())
    }
    fn snapshot(&self, now: Instant) -> Value {
        let mut value =
            serde_json::to_value(&self.definition).expect("Ticker definition is serializable");
        if let Some(file) = &self.file {
            let result = file_service().snapshot(file.watch);
            let resolved = result
                .as_ref()
                .ok()
                .and_then(|entries| entries.as_array())
                .and_then(|entries| entries.first());
            let text = resolved
                .and_then(|entry| entry["text"].as_str())
                .unwrap_or(&file.initial);
            let error = resolved
                .and_then(|entry| entry["error"].as_str())
                .map(str::to_string)
                .or_else(|| result.as_ref().err().cloned())
                .or_else(|| validate_source_text(text).err());
            value["text"] = if error.is_some() { "" } else { text }.into();
            value["sourceError"] = error.map_or(Value::Null, Value::String);
        } else if let Some(error) = &self.source_error {
            value["text"] = "".into();
            value["sourceError"] = error.clone().into();
        }
        value["generation"] = self.generation.into();
        value["elapsedSeconds"] = self.seconds(now).into();
        value["paused"] = self.started.is_none().into();
        value
    }
}

#[derive(Default)]
pub struct TickerRuntime {
    run: Option<Run>,
    generation: u64,
}

impl TickerRuntime {
    pub fn reconcile(&mut self, path: &Path, definitions: &[TickerDefinition]) {
        let Some(run) = self.run.as_mut().filter(|run| run.workspace == path) else {
            return;
        };
        if let Some(definition) = definitions
            .iter()
            .find(|entry| entry.id == run.definition.id)
        {
            if run.definition.source != definition.source {
                match watch_source(&definition.source) {
                    Ok(file) => {
                        run.file = file;
                        run.source_error = None;
                    }
                    Err(error) => {
                        run.file = None;
                        run.source_error = Some(error);
                    }
                }
            }
            run.definition = definition.clone();
        } else {
            self.run = None;
        }
    }

    pub fn control(
        &mut self,
        path: &Path,
        definitions: &[TickerDefinition],
        action: TickerAction,
        now: Instant,
    ) -> Result<Value, String> {
        if let TickerAction::Play { id } = action {
            let definition = definitions
                .iter()
                .find(|entry| entry.id == id)
                .ok_or("Ticker does not exist.")?
                .clone();
            let file = watch_source(&definition.source)?;
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("Ticker generation exhausted.")?;
            self.run = Some(Run {
                file,
                source_error: None,
                workspace: path.into(),
                definition,
                generation: self.generation,
                elapsed: 0.0,
                started: Some(now),
            });
        } else {
            let run = self
                .run
                .as_mut()
                .filter(|run| run.workspace == path)
                .ok_or("Ticker session is retired.")?;
            let generation = match action {
                TickerAction::Pause { generation }
                | TickerAction::Resume { generation }
                | TickerAction::Clear { generation }
                | TickerAction::Seek { generation, .. } => generation,
                TickerAction::Play { .. } => unreachable!(),
            };
            if run.generation != generation {
                return Err("Ticker session is retired.".into());
            }
            match action {
                TickerAction::Pause { .. } => {
                    run.elapsed = run.seconds(now);
                    run.started = None;
                }
                TickerAction::Resume { .. } => {
                    if run.started.is_none() {
                        run.started = Some(now);
                    }
                }
                TickerAction::Clear { .. } => self.run = None,
                TickerAction::Seek { seconds, .. } => {
                    if !seconds.is_finite() || seconds < 0.0 || seconds > 86_400.0 {
                        return Err("Ticker seek must be between 0 and 86400 seconds.".into());
                    }
                    run.elapsed = seconds;
                    if run.started.is_some() {
                        run.started = Some(now);
                    }
                }
                TickerAction::Play { .. } => unreachable!(),
            }
        }
        Ok(self.snapshot(path, now))
    }
    pub fn snapshot(&self, path: &Path, now: Instant) -> Value {
        self.run
            .as_ref()
            .filter(|run| run.workspace == path)
            .map_or(Value::Null, |run| run.snapshot(now))
    }
}

static RUNTIME: OnceLock<Mutex<TickerRuntime>> = OnceLock::new();

pub fn runtime() -> &'static Mutex<TickerRuntime> {
    RUNTIME.get_or_init(|| Mutex::new(TickerRuntime::default()))
}
pub fn snapshot(path: &Path) -> Result<Value, String> {
    Ok(runtime()
        .lock()
        .map_err(|_| "Ticker service is unavailable.")?
        .snapshot(path, Instant::now()))
}
pub fn clear() -> Result<(), String> {
    runtime()
        .lock()
        .map_err(|_| "Ticker service is unavailable.")?
        .run = None;
    Ok(())
}

pub fn apply_text_style(
    style: &mut serde_json::Map<String, Value>,
    context: &Value,
) -> Option<f64> {
    if !is_enabled_ticker(style) {
        return None;
    }
    let ticker = context.get("ticker")?.as_object()?;
    for (source, target) in [
        ("speed", "scrollSpeed"),
        ("direction", "scrollDirection"),
        ("repeat", "scrollRepeat"),
    ] {
        style.insert(target.into(), ticker.get(source)?.clone());
    }
    style.insert("scrollingEnabled".into(), Value::Bool(true));
    if let Some(start) = ticker.get("startPosition").and_then(Value::as_str) {
        style.insert(
            "scrollStart".into(),
            if start == "offscreen" {
                "Off Screen"
            } else {
                "Automatic"
            }
            .into(),
        );
    }
    if let Some(distance) = ticker.get("repeatDistance") {
        style.insert("scrollRepeatDistance".into(), distance.clone());
    }
    style.remove("scrollLoopDelaySeconds");
    if let Some(delay) = ticker.get("loopDelaySeconds") {
        style.insert("scrollLoopDelaySeconds".into(), delay.clone());
        style.insert("scrollStart".into(), "Off Screen".into());
    }
    ticker.get("elapsedSeconds")?.as_f64()
}

pub fn is_enabled_ticker(style: &serde_json::Map<String, Value>) -> bool {
    style.get("linkedTextSource").and_then(Value::as_str) == Some("Ticker")
        && style.get("linkedTextEnabled").and_then(Value::as_bool) != Some(false)
}
