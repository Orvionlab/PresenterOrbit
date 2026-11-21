//! Native RSS state used by program output.
//!
//! The renderer owns authoring data, but output RSS is a native concern. This
//! module keeps a process-local, last-good cache and resolves only the theme
//! slide selected for a native output scene. The raw renderer snapshot is
//! never mutated.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde_json::{Map, Value};

use crate::rss::{self, RssItem};

/// A successful output RSS value is reused for this long before a background
/// revalidation is requested.
pub const OUTPUT_RSS_CACHE_TTL: Duration = Duration::from_secs(30);
/// Failed requests are not retried on every output update.
pub const OUTPUT_RSS_ERROR_BACKOFF: Duration = Duration::from_secs(15);

static NEXT_SERVICE_EPOCH: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RssFetchToken {
    service_epoch: u64,
    generation: u64,
}

#[derive(Clone, Debug)]
pub struct RssFetchRequest {
    pub url: String,
    pub token: RssFetchToken,
}

pub struct RssFetchResult {
    pub url: String,
    pub token: RssFetchToken,
    pub result: Result<Vec<RssItem>, String>,
}

pub type BatchCompletion = Arc<dyn Fn(Vec<RssFetchResult>) + Send + Sync + 'static>;

struct RssRefreshJob {
    requests: Vec<RssFetchRequest>,
    completion: BatchCompletion,
}

enum RssServiceMessage {
    Refresh(RssRefreshJob),
    Stop,
}

/// A single bounded worker owns all output RSS network requests. Keeping the
/// join handle in the service makes shutdown deterministic instead of
/// leaving a detached request thread behind during app teardown.
pub struct OutputRssService {
    sender: SyncSender<RssServiceMessage>,
    stop: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl OutputRssService {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let join = thread::Builder::new()
            .name("presenter-orbit-rss-output".to_string())
            .spawn(move || run_worker(receiver, worker_stop))
            .expect("RSS output worker should start");
        Self {
            sender,
            stop,
            join: Mutex::new(Some(join)),
        }
    }

    pub fn submit(
        &self,
        requests: Vec<RssFetchRequest>,
        completion: BatchCompletion,
    ) -> Result<(), String> {
        if requests.is_empty() {
            return Ok(());
        }
        if self.stop.load(Ordering::Acquire) {
            return Err("RSS output worker is stopping.".to_string());
        }
        let result = self
            .sender
            .try_send(RssServiceMessage::Refresh(RssRefreshJob {
                requests,
                completion,
            }))
            .map_err(|error| match error {
                TrySendError::Full(_) => "RSS output worker queue is full.".to_string(),
                TrySendError::Disconnected(_) => "RSS output worker is unavailable.".to_string(),
            });
        if result.is_ok() && self.stop.load(Ordering::Acquire) {
            return Err("RSS output worker is stopping.".to_string());
        }
        result
    }

    /// Stop and join the worker. Callers must not hold output/protocol/cache
    /// locks while invoking this method.
    pub fn shutdown(&self) {
        if !self.stop.swap(true, Ordering::AcqRel) {
            // Do not use a blocking send here. If the worker is in a bounded
            // queue-draining loop, a full queue could otherwise make shutdown
            // wait forever even though the stop flag already tells the worker
            // to exit after its current request.
            let _ = self.sender.try_send(RssServiceMessage::Stop);
        }
        // Even if another caller already set the stop flag, it may still be
        // joining the worker. Taking the handle here guarantees every caller
        // observes a completed join rather than returning early.
        if let Ok(mut join) = self.join.lock() {
            if let Some(handle) = join.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for OutputRssService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_worker(receiver: Receiver<RssServiceMessage>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        let Ok(message) = receiver.recv() else {
            break;
        };
        match message {
            RssServiceMessage::Stop => break,
            RssServiceMessage::Refresh(job) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let mut results = Vec::with_capacity(job.requests.len());
                for request in job.requests {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let result = rss::fetch_items(&request.url);
                    results.push(RssFetchResult {
                        url: request.url,
                        token: request.token,
                        result,
                    });
                }
                if results.is_empty() {
                    continue;
                }
                (job.completion)(results);
            }
        }
    }
}

#[derive(Clone, Debug)]
struct RssCacheEntry {
    items: Option<Vec<RssItem>>,
    fetched_at: Option<Instant>,
    retry_after: Option<Instant>,
}

impl Default for RssCacheEntry {
    fn default() -> Self {
        Self {
            items: None,
            fetched_at: None,
            retry_after: None,
        }
    }
}

/// Native output RSS cache. The owning application state protects this value
/// with a mutex; network requests never run while that mutex is held.
#[derive(Debug)]
pub struct OutputRssCache {
    entries: HashMap<String, RssCacheEntry>,
    in_flight: HashMap<String, RssFetchToken>,
    service_epoch: u64,
    next_generation: u64,
}

impl Default for OutputRssCache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            in_flight: HashMap::new(),
            service_epoch: NEXT_SERVICE_EPOCH.fetch_add(1, Ordering::Relaxed),
            next_generation: 0,
        }
    }
}

impl OutputRssCache {
    pub fn snapshot(&self) -> OutputRssSnapshot {
        self.snapshot_at(Instant::now())
    }

    pub fn begin_fetch(&mut self, url: &str, now: Instant) -> Option<RssFetchToken> {
        if self.in_flight.contains_key(url) {
            return None;
        }

        if let Some(entry) = self.entries.get(url) {
            if entry
                .fetched_at
                .is_some_and(|fetched_at| now.duration_since(fetched_at) < OUTPUT_RSS_CACHE_TTL)
            {
                return None;
            }
            if entry
                .retry_after
                .is_some_and(|retry_after| now < retry_after)
            {
                return None;
            }
        }

        self.next_generation = self.next_generation.saturating_add(1).max(1);
        let token = RssFetchToken {
            service_epoch: self.service_epoch,
            generation: self.next_generation,
        };
        self.in_flight.insert(url.to_string(), token.clone());
        Some(token)
    }

    pub fn complete_success(
        &mut self,
        url: &str,
        token: &RssFetchToken,
        items: Vec<RssItem>,
        now: Instant,
    ) -> bool {
        if self.in_flight.get(url) != Some(token) || token.service_epoch != self.service_epoch {
            return false;
        }
        self.in_flight.remove(url);
        self.entries.insert(
            url.to_string(),
            RssCacheEntry {
                items: Some(items),
                fetched_at: Some(now),
                retry_after: None,
            },
        );
        true
    }

    pub fn complete_error(&mut self, url: &str, token: &RssFetchToken, now: Instant) -> bool {
        if self.in_flight.get(url) != Some(token) || token.service_epoch != self.service_epoch {
            return false;
        }
        self.in_flight.remove(url);
        let entry = self.entries.entry(url.to_string()).or_default();
        entry.retry_after = Some(now + OUTPUT_RSS_ERROR_BACKOFF);
        true
    }

    fn snapshot_at(&self, now: Instant) -> OutputRssSnapshot {
        OutputRssSnapshot {
            entries: self.entries.clone(),
            now,
        }
    }
}

#[derive(Clone, Debug)]
pub struct OutputRssSnapshot {
    entries: HashMap<String, RssCacheEntry>,
    now: Instant,
}

pub fn resolve_ticker(mut ticker: Value, cache: &OutputRssSnapshot) -> (Value, Vec<String>) {
    let Some(source) = ticker
        .get("source")
        .filter(|source| source["type"] == "rss")
    else {
        return (ticker, Vec::new());
    };
    let Some(url) = source["url"].as_str().and_then(normalize_url) else {
        ticker["text"] = "".into();
        ticker["sourceError"] = "Ticker RSS URL is invalid.".into();
        return (ticker, Vec::new());
    };
    let style = serde_json::json!({"linkedTextRssFormat":if source["format"] == "title" { "title" } else { "full" },"linkedTextRssDelimiter":source["delimiter"],"linkedTextRssTitleSeparateLine":false,"linkedTextRssDescriptionSeparateLine":false}).as_object().unwrap().clone();
    let pending = match cache.lookup(&url) {
        RssLookup::Fresh(items) | RssLookup::Stale(items) => {
            let text = format_items(items, &style);
            if text.len() > 65_536 {
                ticker["text"] = "".into();
                ticker["sourceError"] = "Ticker source exceeds 65536 bytes.".into();
            } else {
                ticker["text"] = text.into();
                ticker["sourceError"] = Value::Null;
            }
            if matches!(cache.lookup(&url), RssLookup::Stale(_)) {
                vec![url]
            } else {
                Vec::new()
            }
        }
        RssLookup::Missing => {
            ticker["text"] = "".into();
            ticker["sourceError"] = "RSS feed has no cached items.".into();
            vec![url]
        }
    };
    (ticker, pending)
}

/// Apply cached RSS values to the already-selected native scenes. This keeps
/// the full authoring theme tree out of the RSS path: output-scene selection
/// has already reduced it to the presentation slides that are actually used.
pub fn resolve_output_scenes(
    mut scenes: Map<String, Value>,
    cache: &OutputRssSnapshot,
) -> (Map<String, Value>, Vec<String>) {
    let mut pending = BTreeSet::new();
    for scene in scenes.values_mut() {
        let Some(slide) = scene.get("presentationThemeSlide").cloned() else {
            continue;
        };
        let (resolved_slide, slide_pending) = resolve_theme_slide(slide, cache);
        pending.extend(slide_pending);
        if let Some(scene_object) = scene.as_object_mut() {
            scene_object.insert("presentationThemeSlide".to_string(), resolved_slide);
        }
    }
    (scenes, pending.into_iter().collect())
}

impl OutputRssSnapshot {
    fn lookup(&self, url: &str) -> RssLookup<'_> {
        let Some(entry) = self.entries.get(url) else {
            return RssLookup::Missing;
        };
        let Some(items) = entry.items.as_deref() else {
            return RssLookup::Missing;
        };
        if entry
            .fetched_at
            .is_some_and(|fetched_at| self.now.duration_since(fetched_at) < OUTPUT_RSS_CACHE_TTL)
        {
            RssLookup::Fresh(items)
        } else {
            RssLookup::Stale(items)
        }
    }
}

enum RssLookup<'a> {
    Fresh(&'a [RssItem]),
    Stale(&'a [RssItem]),
    Missing,
}

/// Resolve RSS only in a selected presentation theme slide. The returned
/// URLs are normalized and deduplicated targets that need a fetch or
/// revalidation.
pub fn resolve_theme_slide(slide: Value, cache: &OutputRssSnapshot) -> (Value, Vec<String>) {
    let Some(mut slide_object) = slide.as_object().cloned() else {
        return (slide, Vec::new());
    };
    let mut pending = BTreeSet::new();
    let slide_style = slide_object
        .get("textStyle")
        .and_then(Value::as_object)
        .cloned();

    if let Some(style) = slide_style.as_ref() {
        let mut resolved_style = style.clone();
        apply_rss_style(style, &mut resolved_style, cache, &mut pending);
        slide_object.insert("textStyle".to_string(), Value::Object(resolved_style));
    }

    if let Some(layers) = slide_object.get("layers").and_then(Value::as_array) {
        let resolved_layers = layers
            .iter()
            .map(|layer| {
                let Some(layer_object) = layer.as_object() else {
                    return layer.clone();
                };
                let Some(layer_style) = layer_object.get("textStyle").and_then(Value::as_object)
                else {
                    return layer.clone();
                };

                let mut effective_style = slide_style.clone().unwrap_or_default();
                effective_style.extend(layer_style.clone());
                let mut resolved_layer = layer_object.clone();
                let mut resolved_style = layer_style.clone();
                apply_rss_style(&effective_style, &mut resolved_style, cache, &mut pending);
                resolved_layer.insert("textStyle".to_string(), Value::Object(resolved_style));
                Value::Object(resolved_layer)
            })
            .collect();
        slide_object.insert("layers".to_string(), Value::Array(resolved_layers));
    }

    (Value::Object(slide_object), pending.into_iter().collect())
}

fn apply_rss_style(
    effective_style: &Map<String, Value>,
    target_style: &mut Map<String, Value>,
    cache: &OutputRssSnapshot,
    pending: &mut BTreeSet<String>,
) {
    if effective_style
        .get("linkedTextEnabled")
        .and_then(Value::as_bool)
        == Some(false)
        || effective_style
            .get("linkedTextSource")
            .and_then(Value::as_str)
            != Some("RSS")
    {
        return;
    }
    let Some(raw_url) = effective_style
        .get("linkedTextRssUrl")
        .and_then(Value::as_str)
    else {
        return;
    };
    let Some(url) = normalize_url(raw_url) else {
        return;
    };

    match cache.lookup(&url) {
        RssLookup::Fresh(items) | RssLookup::Stale(items) => {
            target_style.insert(
                "linkedTextRssItems".to_string(),
                Value::String(serde_json::to_string(items).unwrap_or_else(|_| "[]".to_string())),
            );
            target_style.insert(
                "linkedTextRssContent".to_string(),
                Value::String(format_items(items, effective_style)),
            );
            if matches!(cache.lookup(&url), RssLookup::Stale(_)) {
                pending.insert(url);
            }
        }
        RssLookup::Missing => {
            pending.insert(url);
        }
    }
}

fn normalize_url(raw_url: &str) -> Option<String> {
    rss::validate_url(raw_url).ok().map(|url| url.to_string())
}

fn format_items(items: &[RssItem], style: &Map<String, Value>) -> String {
    let title_format = style.get("linkedTextRssFormat").and_then(Value::as_str) == Some("title");
    let delimiter = match style.get("linkedTextRssDelimiter") {
        Some(Value::Bool(false)) => String::new(),
        Some(Value::Null) | None => "•".to_string(),
        Some(value) => js_string(value),
    };
    let title_separate_line = style
        .get("linkedTextRssTitleSeparateLine")
        .and_then(Value::as_bool)
        != Some(false);
    let description_separate_line = style
        .get("linkedTextRssDescriptionSeparateLine")
        .and_then(Value::as_bool)
        != Some(false);
    let item_delimiter = if title_format {
        if title_separate_line {
            format!("\n{delimiter}\n")
        } else {
            delimiter
        }
    } else if description_separate_line {
        format!("\n{delimiter}\n")
    } else {
        delimiter
    };

    items
        .iter()
        .filter_map(|item| {
            if title_format {
                (!item.title.is_empty()).then_some(item.title.clone())
            } else {
                let value = if title_separate_line {
                    [item.title.as_str(), item.description.as_str()]
                        .into_iter()
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    [item.title.as_str(), item.description.as_str()]
                        .into_iter()
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                (!value.is_empty()).then_some(value)
            }
        })
        .collect::<Vec<_>>()
        .join(&item_delimiter)
}

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        // JavaScript's String(array) uses Array#join: null elements become
        // empty fields (unlike String(null)), while nested arrays recurse.
        Value::Array(values) => values
            .iter()
            .map(js_array_element_string)
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn js_array_element_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Array(values) => values
            .iter()
            .map(js_array_element_string)
            .collect::<Vec<_>>()
            .join(","),
        _ => js_string(value),
    }
}
