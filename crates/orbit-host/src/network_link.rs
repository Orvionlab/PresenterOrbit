use crate::Host;
use std::{
    collections::HashMap,
    net::IpAddr,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;
use uuid::Uuid;

const PROTOCOL: &str = "orbit-link/3";
const MAX_PEERS: usize = 32;
const MAX_PENDING_CUES: usize = 64;
const MAX_PENDING_PER_PEER: usize = 64;
const MAX_INBOUND_SENDERS: usize = 64;
const MAX_ENVELOPE_BYTES: usize = 48 * 1024;
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);
const CONTROL_HTTP_TIMEOUT: Duration = Duration::from_secs(35);
// Do not execute a cue that waited behind slow peers long enough to become stale.
const MAX_CUE_AGE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkLinkPeer {
    pub id: String,
    pub title: String,
    pub host: String,
    pub port: u16,
    pub password: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkLinkSettings {
    pub enabled: bool,
    pub group: String,
    pub peers: Vec<NetworkLinkPeer>,
}

impl Default for NetworkLinkSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            group: String::new(),
            peers: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NetworkLinkMediaAction {
    Trigger,
    Play,
    Pause,
    Stop,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NetworkLinkShowControl {
    Timer,
    Prop,
    Macro,
    Message,
    StageLayout,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NetworkLinkCue {
    Playlist {
        playlist_index: u32,
        entry_index: u32,
        slide_index: u32,
    },
    Library {
        library_index: u32,
        presentation_index: u32,
        slide_index: u32,
    },
    Media {
        bin_index: u32,
        item_index: u32,
        action: NetworkLinkMediaAction,
    },
    ShowControl {
        control: NetworkLinkShowControl,
        #[serde(skip_serializing_if = "Option::is_none")]
        index: Option<u32>,
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        values: Option<Value>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkLinkStatus {
    pub enabled: bool,
    pub group: String,
    pub peers: Vec<NetworkLinkPeerStatus>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkLinkPeerStatus {
    pub id: String,
    pub state: PeerDeliveryState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerDeliveryState {
    Idle,
    Sending,
    Delivered,
    Error,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinkEnvelope {
    protocol: String,
    group: String,
    sender_id: String,
    sequence: u64,
    cue: NetworkLinkCue,
}

struct LinkJob {
    generation: u64,
    status_sequence: u64,
    envelope: LinkEnvelope,
    peers: Vec<NetworkLinkPeer>,
    queued_at: Instant,
}

struct PeerJob {
    generation: u64,
    status_sequence: u64,
    envelope: LinkEnvelope,
    peer: NetworkLinkPeer,
    queued_at: Instant,
}

#[derive(Default)]
struct LinkRuntime {
    peer_status: HashMap<String, NetworkLinkPeerStatus>,
    last_inbound_sequence: HashMap<String, u64>,
    discard_through: HashMap<(String, String), u64>,
    published_sequence: HashMap<String, u64>,
    next_status_sequence: u64,
}

pub struct NetworkLinkService {
    network_enabled: bool,
    settings: NetworkLinkSettings,
    sender_id: String,
    sequence: u64,
    generation: Arc<AtomicU64>,
    runtime: Arc<Mutex<LinkRuntime>>,
    queue: SyncSender<LinkJob>,
}

impl NetworkLinkService {
    pub fn new() -> Self {
        let (queue, receiver) = mpsc::sync_channel::<LinkJob>(MAX_PENDING_CUES);
        let generation = Arc::new(AtomicU64::new(0));
        let runtime = Arc::new(Mutex::new(LinkRuntime::default()));
        let worker_generation = Arc::clone(&generation);
        let worker_runtime = Arc::clone(&runtime);
        thread::Builder::new()
            .name("orbit-network-link".into())
            .spawn(move || {
                let mut peer_queues: HashMap<String, SyncSender<PeerJob>> = HashMap::new();
                while let Ok(job) = receiver.recv() {
                    if worker_generation.load(Ordering::Acquire) != job.generation {
                        continue;
                    }
                    let active_peer_ids = job.peers.iter().map(|peer| peer.id.clone()).collect::<std::collections::HashSet<_>>();
                    peer_queues.retain(|peer_id, _| active_peer_ids.contains(peer_id));
                    for peer in job.peers {
                        if worker_generation.load(Ordering::Acquire) != job.generation {
                            break;
                        }
                        if job.queued_at.elapsed() > MAX_CUE_AGE {
                            set_peer_status(&worker_runtime, job.generation, &worker_generation, NetworkLinkPeerStatus {
                                id: peer.id,
                                state: PeerDeliveryState::Error,
                                error: Some("Queued cue expired before delivery.".into()),
                                sequence: Some(job.status_sequence),
                            });
                            continue;
                        }
                        if ensure_peer_queue(&mut peer_queues, &peer.id, &worker_runtime, &worker_generation).is_err() {
                            set_peer_status(&worker_runtime, job.generation, &worker_generation, NetworkLinkPeerStatus {
                                id: peer.id,
                                state: PeerDeliveryState::Error,
                                error: Some("Network Link peer worker could not start.".into()),
                                sequence: Some(job.status_sequence),
                            });
                            continue;
                        }
                        let peer_job = PeerJob {
                            generation: job.generation,
                            status_sequence: job.status_sequence,
                            envelope: job.envelope.clone(),
                            peer: peer.clone(),
                            queued_at: job.queued_at,
                        };
                        let first_try = peer_queues.get(&peer.id).expect("peer queue was created").try_send(peer_job);
                        let send_result = match first_try {
                            Err(TrySendError::Disconnected(peer_job)) => {
                                peer_queues.remove(&peer.id);
                                if ensure_peer_queue(&mut peer_queues, &peer.id, &worker_runtime, &worker_generation).is_ok() {
                                    peer_queues.get(&peer.id).expect("recreated peer queue").try_send(peer_job)
                                } else {
                                    Err(TrySendError::Disconnected(peer_job))
                                }
                            }
                            other => other,
                        };
                        match send_result {
                            Ok(()) => {},
                            Err(TrySendError::Full(peer_job)) => {
                                discard_failed_peer_queue(&worker_runtime, &worker_generation, peer_job.generation, &peer.id, &peer_job.envelope.sender_id);
                                set_peer_status(&worker_runtime, job.generation, &worker_generation, NetworkLinkPeerStatus {
                                    id: peer.id,
                                    state: PeerDeliveryState::Error,
                                    error: Some("Network Link peer queue is full; queued commands for this sender were discarded.".into()),
                                    sequence: Some(job.status_sequence),
                                });
                            }
                            Err(TrySendError::Disconnected(_)) => {
                                peer_queues.remove(&peer.id);
                                set_peer_status(&worker_runtime, job.generation, &worker_generation, NetworkLinkPeerStatus {
                                    id: peer.id,
                                    state: PeerDeliveryState::Error,
                                    error: Some("Network Link peer worker is unavailable.".into()),
                                    sequence: Some(job.status_sequence),
                                });
                            }
                        }
                    }
                }
            })
            .expect("Network Link worker should start");
        Self {
            network_enabled: true,
            settings: NetworkLinkSettings::default(),
            sender_id: Uuid::new_v4().to_string(),
            sequence: 0,
            generation,
            runtime,
            queue,
        }
    }

    pub fn configure(&mut self, settings: NetworkLinkSettings) -> Result<(), String> {
        let settings = validate_settings(settings)?;
        if self.settings != settings {
            self.generation.fetch_add(1, Ordering::AcqRel);
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Network Link status is unavailable.")?;
            runtime.peer_status.clear();
            runtime.discard_through.clear();
            runtime.last_inbound_sequence.clear();
            runtime.published_sequence.clear();
            for peer in &settings.peers {
                runtime.peer_status.insert(
                    peer.id.clone(),
                    NetworkLinkPeerStatus {
                        id: peer.id.clone(),
                        state: PeerDeliveryState::Idle,
                        error: None,
                        sequence: None,
                    },
                );
            }
            self.settings = settings;
        }
        Ok(())
    }

    pub fn settings(&self) -> NetworkLinkSettings {
        self.settings.clone()
    }

    pub(crate) fn identity(&self) -> &str {
        &self.sender_id
    }

    pub fn set_network_enabled(&mut self, enabled: bool) {
        if self.network_enabled != enabled {
            self.network_enabled = enabled;
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub fn can_send(&self) -> bool {
        self.network_enabled && self.settings.enabled && !self.settings.peers.is_empty()
    }

    pub fn status(&self) -> Result<NetworkLinkStatus, String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "Network Link status is unavailable.")?;
        Ok(NetworkLinkStatus {
            enabled: self.settings.enabled,
            group: self.settings.group.clone(),
            peers: self
                .settings
                .peers
                .iter()
                .map(|peer| {
                    runtime
                        .peer_status
                        .get(&peer.id)
                        .cloned()
                        .unwrap_or(NetworkLinkPeerStatus {
                            id: peer.id.clone(),
                            state: PeerDeliveryState::Idle,
                            error: None,
                            sequence: None,
                        })
                })
                .collect(),
        })
    }

    pub fn send(&mut self, cue: NetworkLinkCue) -> Result<Value, String> {
        if !self.network_enabled || !self.settings.enabled || self.settings.peers.is_empty() {
            return Ok(json!({"queued": false}));
        }
        let sequence = self
            .sequence
            .checked_add(1)
            .ok_or("Network Link sequence is exhausted.")?;
        let envelope = LinkEnvelope {
            protocol: PROTOCOL.into(),
            group: self.settings.group.clone(),
            sender_id: self.sender_id.clone(),
            sequence,
            cue,
        };
        let result = self.enqueue_envelope(envelope)?;
        if result["queued"] == true {
            self.sequence = sequence;
        }
        Ok(result)
    }

    fn forward(&mut self, envelope: LinkEnvelope) -> Result<Value, String> {
        if !self.network_enabled || !self.settings.enabled || self.settings.peers.is_empty() {
            return Ok(json!({"queued": false}));
        }
        if envelope.protocol != PROTOCOL || envelope.group != self.settings.group {
            return Err("Network Link envelope does not match the active group.".into());
        }
        self.enqueue_envelope(envelope)
    }

    fn enqueue_envelope(&mut self, envelope: LinkEnvelope) -> Result<Value, String> {
        let encoded = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
        if encoded.len() > MAX_ENVELOPE_BYTES {
            return Err("Network Link envelope exceeds the 48 KiB size limit.".into());
        }
        let generation = self.generation.load(Ordering::Acquire);
        let job = LinkJob {
            generation,
            status_sequence: 0,
            envelope,
            peers: self.settings.peers.clone(),
            queued_at: Instant::now(),
        };
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "Network Link status is unavailable.")?;
        let status_sequence = runtime
            .next_status_sequence
            .checked_add(1)
            .ok_or("Network Link status sequence is exhausted.")?;
        for peer in &self.settings.peers {
            runtime.peer_status.insert(
                peer.id.clone(),
                NetworkLinkPeerStatus {
                    id: peer.id.clone(),
                    state: PeerDeliveryState::Sending,
                    error: None,
                    sequence: Some(status_sequence),
                },
            );
        }
        let mut job = job;
        job.status_sequence = status_sequence;
        let sender_id = job.envelope.sender_id.clone();
        let envelope_sequence = job.envelope.sequence;
        match self.queue.try_send(job) {
            Ok(()) => {
                runtime.next_status_sequence = status_sequence;
                runtime
                    .published_sequence
                    .entry(sender_id)
                    .and_modify(|published| *published = (*published).max(envelope_sequence))
                    .or_insert(envelope_sequence);
                Ok(json!({"queued": true}))
            }
            Err(TrySendError::Full(_)) => {
                set_send_error(
                    &mut runtime,
                    &self.settings.peers,
                    status_sequence,
                    "Network Link queue is full.",
                );
                Err("Network Link queue is full.".into())
            }
            Err(TrySendError::Disconnected(_)) => {
                set_send_error(
                    &mut runtime,
                    &self.settings.peers,
                    status_sequence,
                    "Network Link worker is unavailable.",
                );
                Err("Network Link worker is unavailable.".into())
            }
        }
    }

    pub fn invalidate_pending(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut runtime) = self.runtime.lock() {
            for status in runtime.peer_status.values_mut() {
                if status.state == PeerDeliveryState::Sending {
                    status.state = PeerDeliveryState::Idle;
                    status.error = None;
                    status.sequence = None;
                }
            }
        }
    }

    fn claim_incoming(&mut self, envelope: &LinkEnvelope) -> Result<bool, String> {
        self.validate_incoming(envelope)?;
        if envelope.sender_id == self.sender_id {
            return Ok(false);
        }
        let sender = envelope.sender_id.clone();
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "Network Link status is unavailable.")?;
        if runtime
            .last_inbound_sequence
            .get(&sender)
            .is_some_and(|last| envelope.sequence <= *last)
        {
            return Ok(false);
        }
        if !runtime.last_inbound_sequence.contains_key(&sender)
            && runtime.last_inbound_sequence.len() >= MAX_INBOUND_SENDERS
        {
            return Err("Network Link has reached its inbound peer limit.".into());
        }
        runtime
            .last_inbound_sequence
            .insert(sender, envelope.sequence);
        Ok(true)
    }

    fn is_duplicate(&self, envelope: &LinkEnvelope) -> Result<bool, String> {
        self.validate_incoming(envelope)?;
        if envelope.sender_id == self.sender_id {
            return Ok(true);
        }
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "Network Link status is unavailable.")?;
        Ok(runtime
            .last_inbound_sequence
            .get(&envelope.sender_id)
            .is_some_and(|last| envelope.sequence <= *last))
    }

    fn validate_incoming(&self, envelope: &LinkEnvelope) -> Result<(), String> {
        if !self.network_enabled || !self.settings.enabled {
            return Err("Network Link is disabled.".into());
        }
        if envelope.protocol != PROTOCOL {
            return Err("Unsupported Network Link protocol.".into());
        }
        if envelope.group != self.settings.group {
            return Err("Network Link group does not match.".into());
        }
        if envelope.sequence == 0 {
            return Err("Network Link sender or sequence is invalid.".into());
        }
        Uuid::parse_str(&envelope.sender_id)
            .ok()
            .filter(|id| id.to_string() == envelope.sender_id)
            .ok_or("Network Link sender identity must be a canonical UUID.")?;
        Ok(())
    }
}

pub fn validate_settings(settings: NetworkLinkSettings) -> Result<NetworkLinkSettings, String> {
    if settings.group.chars().count() > 128
        || settings.group.chars().any(char::is_control)
        || settings.group.trim() != settings.group
    {
        return Err("Network Link group must be at most 128 characters with no surrounding whitespace or control characters.".into());
    }
    if settings.enabled && settings.group.is_empty() {
        return Err("An enabled Network Link requires a group name.".into());
    }
    if settings.peers.len() > MAX_PEERS {
        return Err(format!("Network Link supports at most {MAX_PEERS} peers."));
    }
    let mut ids = std::collections::HashSet::new();
    for peer in &settings.peers {
        if peer.id.trim().is_empty()
            || peer.id.len() > 128
            || peer.title.trim().is_empty()
            || peer.title.len() > 128
            || peer.host.trim().is_empty()
            || peer.port == 0
            || peer.password.is_empty()
            || peer.password.len() > 1024
            || peer.password.chars().any(char::is_control)
            || !ids.insert(&peer.id)
        {
            return Err(
                "Network Link peers require unique ids, titles, hosts, valid ports, and passwords."
                    .into(),
            );
        }
        peer_url(peer)?;
    }
    Ok(settings)
}

fn peer_url(peer: &NetworkLinkPeer) -> Result<Url, String> {
    let host = peer.host.trim();
    if host != peer.host || host.chars().any(char::is_control) || host.contains('/') {
        return Err("Network Link peer host must be a bare HTTP hostname or IP address.".into());
    }
    let host = if host.starts_with('[') && host.ends_with(']') {
        host.to_string()
    } else if host
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_ipv6())
    {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let url = Url::parse(&format!("http://{host}:{}/api/v1/link/cue", peer.port))
        .map_err(|_| "Network Link peer host is invalid.".to_string())?;
    if url.scheme() != "http"
        || url.host_str().is_none()
        || url.port() != Some(peer.port)
        || url.path() != "/api/v1/link/cue"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Network Link peer host must be a bare HTTP hostname or IP address.".into());
    }
    Ok(url)
}

fn send_to_peer(
    agent: &ureq::Agent,
    peer: &NetworkLinkPeer,
    envelope: &LinkEnvelope,
) -> Result<(), String> {
    let url = peer_url(peer)?;
    let authorization = format!(
        "Basic {}",
        crate::remote::base64_basic(&format!("presenter-orbit:{}", peer.password))
    );
    let mut response = agent
        .post(url.as_str())
        .header("Authorization", &authorization)
        .header("Content-Type", "application/json")
        .send(serde_json::to_vec(envelope).map_err(|error| error.to_string())?)
        .map_err(|_| "Connection failed.".to_string())?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("Peer returned HTTP {status}."));
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(4096)
        .read_to_vec()
        .map_err(|_| "Peer response could not be read.".to_string())?;
    let result: Value = serde_json::from_slice(&body)
        .map_err(|_| "Peer did not confirm the cue outcome.".to_string())?;
    if result.get("ok").and_then(Value::as_bool) != Some(true)
        || !matches!(
            result.get("status").and_then(Value::as_str),
            Some("applied" | "dispatched" | "duplicate")
        )
    {
        return Err("Peer did not confirm the cue outcome.".into());
    }
    if let Some(error) = result.get("forwardError").and_then(Value::as_str) {
        return Err(format!(
            "Peer applied the command but group forwarding failed: {error}"
        ));
    }
    Ok(())
}

fn network_link_agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .user_agent("Presenter Orbit Network Link/1.0")
        .build()
        .new_agent()
}

fn run_peer_queue(
    receiver: mpsc::Receiver<PeerJob>,
    runtime: Arc<Mutex<LinkRuntime>>,
    generation: Arc<AtomicU64>,
) {
    let cue_agent = network_link_agent(HTTP_TIMEOUT);
    let control_agent = network_link_agent(CONTROL_HTTP_TIMEOUT);
    loop {
        let job = match receiver.recv_timeout(Duration::from_secs(60)) {
            Ok(job) => job,
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if generation.load(Ordering::Acquire) != job.generation {
            continue;
        }
        let sender_id = job.envelope.sender_id.clone();
        let cutoff = runtime
            .lock()
            .ok()
            .and_then(|runtime| {
                runtime
                    .discard_through
                    .get(&(job.peer.id.clone(), sender_id.clone()))
                    .copied()
            })
            .unwrap_or(0);
        if job.envelope.sequence <= cutoff {
            set_peer_status(
                &runtime,
                job.generation,
                &generation,
                NetworkLinkPeerStatus {
                    id: job.peer.id,
                    state: PeerDeliveryState::Error,
                    error: Some(
                        "Queued cue was discarded after an earlier delivery failure.".into(),
                    ),
                    sequence: Some(job.status_sequence),
                },
            );
            continue;
        }
        if job.queued_at.elapsed() > MAX_CUE_AGE {
            set_peer_status(
                &runtime,
                job.generation,
                &generation,
                NetworkLinkPeerStatus {
                    id: job.peer.id,
                    state: PeerDeliveryState::Error,
                    error: Some("Queued cue expired before delivery.".into()),
                    sequence: Some(job.status_sequence),
                },
            );
            continue;
        }
        let agent = if requires_control_ack(&job.envelope.cue) {
            &control_agent
        } else {
            &cue_agent
        };
        let result = send_to_peer(agent, &job.peer, &job.envelope);
        if result.is_err() {
            discard_failed_peer_queue(
                &runtime,
                &generation,
                job.generation,
                &job.peer.id,
                &sender_id,
            );
        }
        set_peer_status(
            &runtime,
            job.generation,
            &generation,
            match result {
                Ok(()) => NetworkLinkPeerStatus {
                    id: job.peer.id,
                    state: PeerDeliveryState::Delivered,
                    error: None,
                    sequence: Some(job.status_sequence),
                },
                Err(error) => NetworkLinkPeerStatus {
                    id: job.peer.id,
                    state: PeerDeliveryState::Error,
                    error: Some(error),
                    sequence: Some(job.status_sequence),
                },
            },
        );
    }
}

fn ensure_peer_queue(
    queues: &mut HashMap<String, SyncSender<PeerJob>>,
    peer_id: &str,
    runtime: &Arc<Mutex<LinkRuntime>>,
    generation: &Arc<AtomicU64>,
) -> Result<(), String> {
    if queues.contains_key(peer_id) {
        return Ok(());
    }
    let (sender, receiver) = mpsc::sync_channel(MAX_PENDING_PER_PEER);
    let runtime = Arc::clone(runtime);
    let generation = Arc::clone(generation);
    thread::Builder::new()
        .name(format!("orbit-link-peer-{}", Uuid::new_v4()))
        .spawn(move || run_peer_queue(receiver, runtime, generation))
        .map_err(|_| "Network Link peer worker could not start.")?;
    queues.insert(peer_id.to_string(), sender);
    Ok(())
}

fn requires_control_ack(cue: &NetworkLinkCue) -> bool {
    match cue {
        NetworkLinkCue::ShowControl {
            control:
                NetworkLinkShowControl::Macro
                | NetworkLinkShowControl::Prop
                | NetworkLinkShowControl::Message
                | NetworkLinkShowControl::StageLayout,
            ..
        } => true,
        NetworkLinkCue::ShowControl {
            control: NetworkLinkShowControl::Timer,
            action,
            ..
        } => action == "configure",
        _ => false,
    }
}

fn discard_failed_peer_queue(
    runtime: &Mutex<LinkRuntime>,
    generation: &AtomicU64,
    job_generation: u64,
    peer_id: &str,
    sender_id: &str,
) {
    if let Ok(mut runtime) = runtime.lock() {
        if generation.load(Ordering::Acquire) == job_generation {
            let cutoff = runtime
                .published_sequence
                .get(sender_id)
                .copied()
                .unwrap_or(0);
            runtime
                .discard_through
                .entry((peer_id.to_string(), sender_id.to_string()))
                .and_modify(|existing| *existing = (*existing).max(cutoff))
                .or_insert(cutoff);
        }
    }
}

fn set_peer_status(
    runtime: &Mutex<LinkRuntime>,
    job_generation: u64,
    generation: &AtomicU64,
    status: NetworkLinkPeerStatus,
) {
    if generation.load(Ordering::Acquire) != job_generation {
        return;
    }
    if let Ok(mut runtime) = runtime.lock() {
        if generation.load(Ordering::Acquire) == job_generation {
            let is_current = runtime
                .peer_status
                .get(&status.id)
                .and_then(|current| current.sequence)
                .is_none_or(|sequence| status.sequence.is_none_or(|incoming| incoming >= sequence));
            if is_current {
                runtime.peer_status.insert(status.id.clone(), status);
            }
        }
    }
}

pub(crate) fn map_cue(
    workspace: &Value,
    workspace_path: &Path,
    cue: NetworkLinkCue,
) -> Result<Value, String> {
    map_cue_with_snapshot(workspace, workspace_path, &cue, None)
}

pub(crate) fn map_cue_with_snapshot(
    workspace: &Value,
    workspace_path: &Path,
    cue: &NetworkLinkCue,
    snapshot: Option<&Value>,
) -> Result<Value, String> {
    match cue {
        NetworkLinkCue::Playlist {
            playlist_index,
            entry_index,
            slide_index,
        } => map_playlist_cue(
            workspace,
            workspace_path,
            *playlist_index,
            *entry_index,
            *slide_index,
        ),
        NetworkLinkCue::Library {
            library_index,
            presentation_index,
            slide_index,
        } => map_library_cue(
            workspace,
            workspace_path,
            *library_index,
            *presentation_index,
            *slide_index,
        ),
        NetworkLinkCue::Media { .. } | NetworkLinkCue::ShowControl { .. } => {
            crate::network_link_controls::map_cue(workspace, workspace_path, cue, snapshot)
        }
    }
}

fn map_playlist_cue(
    workspace: &Value,
    workspace_path: &Path,
    playlist_index: u32,
    entry_index: u32,
    slide_index: u32,
) -> Result<Value, String> {
    let path = workspace_path
        .to_str()
        .ok_or("Workspace path must be valid Unicode for Network Link.")?;
    let playlist = workspace
        .get("playlist")
        .and_then(Value::as_array)
        .and_then(|items| items.get(playlist_index as usize))
        .ok_or("Network Link playlist index is unavailable.")?;
    if playlist.get("type").and_then(Value::as_str) == Some("folder") {
        return Err("Network Link cue targets a playlist folder.".into());
    }
    let entry = playlist
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| items.get(entry_index as usize))
        .ok_or("Network Link playlist entry index is unavailable.")?;
    if entry.get("type").and_then(Value::as_str) != Some("presentation") {
        return Err("Network Link cue targets a non-presentation playlist entry.".into());
    }
    let entry_id = entry["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Network Link playlist entry identity is invalid.")?;
    let playlist_id = playlist["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Network Link playlist identity is invalid.")?;
    let item = workspace
        .get("library")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find(|item| item["id"] == entry["itemId"]))
        .ok_or("Network Link presentation is unavailable.")?;
    let item_id = item["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Network Link presentation identity is invalid.")?;
    let mut selected = item.clone();
    if let Some(arrangement_id) = entry.get("arrangementId").and_then(Value::as_str) {
        if !item["arrangements"].as_array().is_some_and(|arrangements| {
            arrangements
                .iter()
                .any(|arrangement| arrangement["id"].as_str() == Some(arrangement_id))
        }) {
            return Err("Network Link playlist arrangement is unavailable.".into());
        }
        selected["selectedArrangementId"] = json!(arrangement_id);
    }
    let slides =
        crate::output_arrangement::resolve_live_arrangement_slides(&selected, None, Some(entry_id))
            .ok_or("Network Link playlist arrangement is unavailable.")?;
    let slide = slides
        .get(slide_index as usize)
        .ok_or("Network Link slide index is unavailable.")?;
    if slide.get("disabled").and_then(Value::as_bool) == Some(true) {
        return Err("Network Link cue targets a disabled slide.".into());
    }
    let slide_id = slide["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Network Link slide identity is invalid.")?;
    Ok(json!({
        "command":"playlistCue",
        "workspacePath":path,
        "playlistId":playlist_id,
        "playlistEntryId":entry_id,
        "presentationId":item_id,
        "slideId":slide_id,
        "cueInstanceId":slide.get("arrangementInstanceId").cloned().unwrap_or(Value::Null),
    }))
}

fn map_library_cue(
    workspace: &Value,
    workspace_path: &Path,
    library_index: u32,
    presentation_index: u32,
    slide_index: u32,
) -> Result<Value, String> {
    let path = workspace_path
        .to_str()
        .ok_or("Workspace path must be valid Unicode for Network Link.")?;
    let library = workspace
        .get("libraries")
        .and_then(Value::as_array)
        .and_then(|libraries| libraries.get(library_index as usize))
        .ok_or("Network Link library index is unavailable.")?;
    let library_id = library["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Network Link library identity is invalid.")?;
    let presentation_id = library
        .get("itemIds")
        .and_then(Value::as_array)
        .and_then(|ids| ids.get(presentation_index as usize))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("Network Link presentation index is unavailable in the selected library.")?;
    let item = workspace
        .get("library")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find(|item| item["id"] == presentation_id))
        .ok_or("Network Link presentation is unavailable.")?;
    let item_id = item["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Network Link presentation identity is invalid.")?;

    let mut selected = item.clone();
    let selected_arrangement_id = match item.get("selectedArrangementId") {
        None => None,
        Some(Value::String(id)) if id.is_empty() => {
            selected
                .as_object_mut()
                .ok_or("Network Link presentation is invalid.")?
                .remove("selectedArrangementId");
            None
        }
        Some(Value::String(id)) if !id.is_empty() => {
            if !item
                .get("arrangements")
                .and_then(Value::as_array)
                .is_some_and(|arrangements| {
                    arrangements
                        .iter()
                        .any(|arrangement| arrangement["id"].as_str() == Some(id))
                })
            {
                return Err("Network Link selected arrangement is unavailable.".into());
            }
            Some(id.as_str())
        }
        Some(_) => return Err("Network Link selected arrangement identity is invalid.".into()),
    };
    let slides = crate::output_arrangement::resolve_live_arrangement_slides(&selected, None, None)
        .ok_or("Network Link presentation arrangement is unavailable.")?;
    let slide = slides
        .get(slide_index as usize)
        .ok_or("Network Link slide index is unavailable in the selected arrangement.")?;
    if slide.get("disabled").and_then(Value::as_bool) == Some(true) {
        return Err("Network Link cue targets a disabled slide.".into());
    }
    let slide_id = slide["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Network Link slide identity is invalid.")?;
    let mut payload = json!({
        "command":"libraryCue",
        "workspacePath":path,
        "libraryId":library_id,
        "presentationId":item_id,
        "playlistEntryId":Value::Null,
        "slideId":slide_id,
        "cueInstanceId":slide.get("arrangementInstanceId").cloned().unwrap_or(Value::Null),
    });
    if let Some(arrangement_id) = selected_arrangement_id {
        payload["arrangementId"] = json!(arrangement_id);
    }
    Ok(payload)
}

pub fn receive_and_apply(app: &Host, body: &[u8]) -> Result<Value, String> {
    if body.len() > MAX_ENVELOPE_BYTES {
        return Err("Network Link envelope is too large.".into());
    }
    let envelope: LinkEnvelope = serde_json::from_slice(body)
        .map_err(|error| format!("Invalid Network Link envelope: {error}"))?;
    let state = app.state::<crate::AppState>();
    let duplicate = state
        .network_link
        .lock()
        .map_err(|_| "Network Link state is unavailable.")?
        .is_duplicate(&envelope)?;
    if duplicate {
        return Ok(json!({"ok":true,"status":"duplicate","changed":false}));
    }
    let path = state
        .active_workspace
        .lock()
        .map_err(|_| "Workspace state is unavailable.")?
        .clone()
        .ok_or("An active workspace is required for Network Link.")?;
    let workspace_io = crate::lock_workspace_io(&state)?;
    workspace_io.require_active(&path)?;
    let workspace = crate::workspace::read_workspace(&path)?;
    let snapshot = crate::lock_output_snapshot_state(&state.output_snapshot)?
        .cache
        .as_ref()
        .map(|cached| cached.snapshot.clone())
        .ok_or("Native output has not accepted a snapshot for Network Link controls.")?;
    let payload = map_cue_with_snapshot(&workspace, &path, &envelope.cue, Some(&snapshot))?;
    drop(workspace_io);

    let accepted = state
        .network_link
        .lock()
        .map_err(|_| "Network Link state is unavailable.")?
        .claim_incoming(&envelope)?;
    if !accepted {
        return Ok(json!({"ok":true,"status":"duplicate","changed":false}));
    }

    let command = payload
        .get("command")
        .and_then(Value::as_str)
        .ok_or("Network Link cue command is invalid.")?;
    let applied = if matches!(
        command,
        "macro" | "message" | "prop" | "stageLayout" | "timerDefinition" | "messageDefinition"
    ) {
        state
            .remote_status
            .dispatch_network_link_command(app, &payload)
    } else {
        crate::execute_native_remote_command(app, command, &payload)
    };
    let mut result = match applied {
        Ok(result) => result,
        Err(error) => return Err(error),
    };
    if result.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Network Link command was not applied.")
            .to_string());
    }
    if result.get("ok").and_then(Value::as_bool) == Some(true)
        && result.get("status").and_then(Value::as_str) == Some("unchanged")
    {
        result["status"] = json!("applied");
    }
    let forwarded = state
        .network_link
        .lock()
        .map_err(|_| "Network Link state is unavailable.")?
        .forward(envelope);
    match forwarded {
        Ok(forwarded) => result["forwarded"] = forwarded["queued"].clone(),
        Err(error) => {
            result["forwarded"] = json!(false);
            result["forwardError"] = json!(error);
        }
    }
    Ok(result)
}

fn set_send_error(
    runtime: &mut LinkRuntime,
    peers: &[NetworkLinkPeer],
    sequence: u64,
    error: &str,
) {
    for peer in peers {
        runtime.peer_status.insert(
            peer.id.clone(),
            NetworkLinkPeerStatus {
                id: peer.id.clone(),
                state: PeerDeliveryState::Error,
                error: Some(error.into()),
                sequence: Some(sequence),
            },
        );
    }
}
