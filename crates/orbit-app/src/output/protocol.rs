//! The v2 output protocol of `output-content-sync.ts` and `output-content-rebase.ts`.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};

use orbit_host::Host;
use serde_json::{json, Map, Value};

const NATIVE_KEYS: &[&str] = &[
    "outputFrozen",
    "activeActionSlide",
    "activeMessageId",
    "activeMessageInstanceId",
    "activeMessageText",
    "audioId",
    "audioSelectionId",
    "countdownTimerRuns",
    "backgroundEndBehavior",
    "backgroundId",
    "backgroundPlayback",
    "backgroundPlaybackState",
    "backgroundSelectionId",
    "backgroundEffectsOverride",
    "backgroundTransportOverride",
    "backgroundTransition",
    "liveBuildStep",
    "liveBuildStepStartedAtMsByStep",
    "livePropIds",
    "remotePropDeadlines",
    "liveVideoInputId",
    "liveItemId",
    "liveItem",
    "liveSlide",
    "liveCueInstanceId",
    "livePlaylistId",
    "livePlaylistEntryId",
    "presentationNextAnchor",
    "liveSlideId",
    "media",
    "mediaLibraries",
    "screenConfig",
    "stageMessage",
    "timelineStartedAtMs",
    "timelineTransport",
    "transportPlaybackPaused",
];

fn session_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "gpui-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            // A clock before the epoch only weakens the timestamp; the process id and counter keep ids unique.
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Where a scene goes: the UI's own host, or the output process.
pub trait Target {
    fn call(&self, method: &str, args: Vec<Value>) -> Result<Value, String>;
}

impl Target for Host {
    fn call(&self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        self.invoke(method, args)
    }
}

impl Target for crate::output_process::client::OutputLink {
    fn call(&self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        self.invoke(method, args)
    }
}

/// Shares the unchanged theme catalog while live scene fields are compared and sent.
#[derive(Clone)]
pub struct Snapshot {
    pub state: Arc<Value>,
    pub theme_source: Arc<Value>,
}

impl Snapshot {
    pub fn materialize(&self) -> Value {
        let mut value = (*self.state).clone();
        if let Some(themes) = self.theme_source.get("themes") {
            value["themes"] = themes.clone();
        }
        value
    }

    fn from_value(mut value: Value) -> Self {
        let themes = value
            .as_object_mut()
            .and_then(|value| value.remove("themes"));
        Self {
            state: Arc::new(value),
            theme_source: Arc::new(
                themes.map_or_else(|| json!({}), |themes| json!({"themes": themes})),
            ),
        }
    }

    fn merge_patch(&mut self, patch: &Map<String, Value>) {
        let mut patch = patch.clone();
        if let Some(themes) = patch.remove("themes") {
            self.theme_source = Arc::new(json!({"themes":themes}));
        }
        merge_patch(Arc::make_mut(&mut self.state), &patch);
    }
}

pub struct Protocol {
    session: String,
    sequence: u64,
    revision: u64,
    accepted: Option<Snapshot>,
    baseline: Option<Snapshot>,
}

impl Default for Protocol {
    fn default() -> Self {
        Self {
            session: session_id(),
            sequence: 0,
            revision: 0,
            accepted: None,
            baseline: None,
        }
    }
}

pub struct Accepted {
    pub revision: u64,
    pub snapshot: Snapshot,
    pub native_patch: Map<String, Value>,
}

pub enum Delivery {
    Accepted(Accepted),
    Contended(Map<String, Value>),
}

pub fn merge_patch(snapshot: &mut Value, patch: &Map<String, Value>) {
    snapshot
        .as_object_mut()
        .expect("output snapshot is an object")
        .extend(patch.clone());
    if patch.get("liveItemId") == Some(&Value::Null) {
        for key in ["liveItem", "liveSlide", "livePlaylistEntryId"] {
            snapshot[key] = Value::Null;
        }
    }
}

pub fn rebase(
    previous: Option<&Value>,
    current: &Value,
    native: &Value,
) -> (Value, Map<String, Value>) {
    let mut rebased = native.clone();
    let mut native_patch = Map::new();
    for (key, value) in current.as_object().expect("output snapshot is an object") {
        let locally_changed = previous.is_some_and(|previous| previous.get(key) != Some(value));
        if NATIVE_KEYS.contains(&key.as_str()) && !locally_changed && native.get(key).is_some() {
            if native.get(key) != Some(value) {
                native_patch.insert(key.clone(), native[key].clone());
            }
        } else if previous.is_none_or(|previous| previous.get(key) != Some(value)) {
            rebased[key] = value.clone();
        }
    }
    if rebased["liveItemId"].is_null() {
        for key in ["liveItem", "liveSlide", "livePlaylistEntryId"] {
            rebased[key] = Value::Null;
        }
    }
    (rebased, native_patch)
}

/// The top-level keys of `snapshot` that differ from `accepted`.
fn changed(accepted: &Snapshot, snapshot: &Snapshot) -> Map<String, Value> {
    let mut patch: Map<String, Value> = snapshot
        .state
        .as_object()
        .expect("output snapshot is an object")
        .iter()
        .filter(|(key, value)| accepted.state.get(*key) != Some(*value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if !Arc::ptr_eq(&accepted.theme_source, &snapshot.theme_source)
        && accepted.theme_source.get("themes") != snapshot.theme_source.get("themes")
    {
        if let Some(themes) = snapshot.theme_source.get("themes") {
            patch.insert("themes".into(), themes.clone());
        }
    }
    patch
}

impl Protocol {
    pub fn accept_native_patch(&mut self, revision: u64, patch: &Map<String, Value>) {
        if revision == 0 || revision < self.revision {
            return;
        }
        self.revision = revision;
        if let Some(snapshot) = self.accepted.as_mut().or(self.baseline.as_mut()) {
            snapshot.merge_patch(patch);
        }
    }

    /// Runs on the background executor. Contention asks the caller to yield and capture the latest scene after
    /// repeated revision conflicts. One caller owns this protocol until its acknowledgement arrives.
    pub fn send(&mut self, target: &impl Target, snapshot: Snapshot) -> Result<Delivery, String> {
        let authored = snapshot;
        let mut snapshot = authored.clone();
        let previous = self.accepted.clone().or_else(|| self.baseline.clone());
        if previous.is_none() {
            self.baseline = Some(snapshot.clone());
        }
        let mut native_patch = Map::new();
        for attempt in 0..=2 {
            // A change to a field the host does not accept in a patch (`presentationNextAnchor`, for one) is sent as
            // a full snapshot.
            let patch = self
                .accepted
                .as_ref()
                .map(|accepted| changed(accepted, &snapshot))
                .filter(|patch| {
                    patch
                        .keys()
                        .all(|key| orbit_host::output_patch_supports(key))
                });
            if patch.as_ref().is_some_and(Map::is_empty) {
                return Ok(Delivery::Accepted(Accepted {
                    revision: self.revision,
                    snapshot,
                    native_patch,
                }));
            }
            self.sequence += 1;
            let mut request =
                json!({ "version": 2, "sessionId": self.session, "clientSequence": self.sequence });
            if let Some(patch) = patch {
                request["kind"] = json!("patch");
                request["patch"] = Value::Object(patch);
                request["baseRevision"] = json!(self.revision);
            } else {
                request["kind"] = json!("full");
                request["snapshot"] = snapshot.materialize();
                if self.revision > 0 {
                    request["baseRevision"] = json!(self.revision);
                }
            }
            let result = match target.call("updateOutputScene", vec![request]) {
                Ok(result) => result,
                Err(error) => {
                    self.baseline = self.accepted.take().or_else(|| self.baseline.take());
                    return Err(error);
                }
            };
            if result["ok"] == true && result["stale"] != true && result["cacheMiss"] != true {
                let revision = result["revision"]
                    .as_u64()
                    .filter(|revision| *revision > 0)
                    .ok_or("The output scene acknowledgement has no positive revision.")?;
                self.revision = revision;
                self.accepted = Some(snapshot.clone());
                self.baseline = None;
                return Ok(Delivery::Accepted(Accepted {
                    revision,
                    snapshot,
                    native_patch,
                }));
            }
            self.baseline = self.accepted.take().or_else(|| self.baseline.take());
            if result["cacheMiss"] != true && result["stale"] != true {
                return Err("The native output scene could not be synchronized.".into());
            }
            if result["cacheMiss"] == true {
                if let Some(revision) = result["revision"].as_u64().filter(|revision| *revision > 0)
                {
                    self.revision = revision;
                    if result["snapshot"].is_object() {
                        let previous = previous
                            .as_ref()
                            .or(self.baseline.as_ref())
                            .map(Snapshot::materialize);
                        let (next, patch) = rebase(
                            previous.as_ref(),
                            &authored.materialize(),
                            &result["snapshot"],
                        );
                        snapshot = Snapshot::from_value(next);
                        native_patch = patch;
                    }
                } else {
                    self.revision = 0;
                }
            }
            if result["stale"] == true {
                self.session = session_id();
                self.sequence = 0;
            }
            if attempt == 2 {
                return Ok(Delivery::Contended(native_patch));
            }
        }
        unreachable!("the recovery loop always returns")
    }
}
