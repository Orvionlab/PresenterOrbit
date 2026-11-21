use crate::Host;
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 79, 66);
const PORT: u16 = 49322;
const PROBE: &[u8] = b"orbit-link-discover/1";
const LIMIT: usize = 2048;
static STARTED: OnceLock<Result<(), String>> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveredPeer {
    pub id: String,
    pub title: String,
    pub host: String,
    pub port: u16,
    pub group: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Advertisement {
    protocol: String,
    id: String,
    title: String,
    port: u16,
    group: String,
}

pub fn title() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|title| !title.trim().is_empty() && title.chars().count() <= 128)
        .unwrap_or_else(|| "Presenter Orbit".into())
}

pub fn start(app: &Host) {
    let result = STARTED.get_or_init(|| {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PORT))
            .map_err(|error| format!("Network Link discovery could not listen: {error}"))?;
        socket
            .join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED)
            .map_err(|error| format!("Network Link discovery could not join multicast: {error}"))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|error| error.to_string())?;
        let app = app.clone();
        thread::Builder::new()
            .name("orbit-link-discovery".into())
            .spawn(move || {
                let mut bytes = [0; LIMIT];
                loop {
                    let (length, source) = match socket.recv_from(&mut bytes) {
                        Ok(received) => received,
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                            ) =>
                        {
                            continue
                        }
                        Err(error) => {
                            eprintln!("[network-link-discovery] {error}");
                            break;
                        }
                    };
                    if &bytes[..length] != PROBE {
                        continue;
                    }
                    let state = app.state::<crate::AppState>();
                    let connection = state
                        .remote_server
                        .lock()
                        .ok()
                        .and_then(|remote| remote.link_connection().ok().flatten());
                    let Some((port, _)) = connection else {
                        continue;
                    };
                    let Ok(link) = state.network_link.lock() else {
                        continue;
                    };
                    let settings = link.settings();
                    let advertisement = Advertisement {
                        protocol: "orbit-link-discovery/1".into(),
                        id: link.identity().to_owned(),
                        title: title(),
                        port,
                        group: if settings.enabled {
                            settings.group
                        } else {
                            String::new()
                        },
                    };
                    if let Ok(bytes) = serde_json::to_vec(&advertisement) {
                        let _ = socket.send_to(&bytes, source);
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(())
    });
    if let Err(error) = result {
        eprintln!("[network-link-discovery] {error}");
    }
}

fn decode(bytes: &[u8], source: SocketAddr, own_id: &str) -> Result<DiscoveredPeer, String> {
    if bytes.len() >= LIMIT {
        return Err("Discovery reply is too large.".into());
    }
    let reply: Advertisement = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if reply.protocol != "orbit-link-discovery/1"
        || reply.id == own_id
        || uuid::Uuid::parse_str(&reply.id).is_err()
        || reply.port < 1024
        || reply.title.trim().is_empty()
        || reply.title.chars().count() > 128
        || reply.group.chars().count() > 128
        || reply.group.trim() != reply.group
        || reply.group.chars().any(char::is_control)
        || reply.title.chars().any(char::is_control)
        || source.ip().is_unspecified()
        || source.ip().is_multicast()
    {
        return Err("Discovery reply is invalid or belongs to this process.".into());
    }
    Ok(DiscoveredPeer {
        id: reply.id,
        title: reply.title,
        host: source.ip().to_string(),
        port: reply.port,
        group: reply.group,
    })
}

fn scan(
    own_id: &str,
    target: SocketAddr,
    duration: Duration,
) -> Result<Vec<DiscoveredPeer>, String> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|error| error.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| error.to_string())?;
    socket
        .set_multicast_ttl_v4(1)
        .map_err(|error| error.to_string())?;
    socket
        .send_to(PROBE, target)
        .map_err(|error| format!("Network Link discovery probe failed: {error}"))?;
    let deadline = Instant::now() + duration;
    let mut peers = HashMap::new();
    let mut bytes = [0; LIMIT];
    while Instant::now() < deadline {
        match socket.recv_from(&mut bytes) {
            Ok((length, source)) => {
                if let Ok(peer) = decode(&bytes[..length], source, own_id) {
                    if peers.len() < 64 || peers.contains_key(&peer.id) {
                        peers.insert(peer.id.clone(), peer);
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) => {}
            Err(error) => return Err(format!("Network Link discovery failed: {error}")),
        }
    }
    let mut peers: Vec<_> = peers.into_values().collect();
    peers.sort_by(|left, right| left.title.cmp(&right.title).then(left.id.cmp(&right.id)));
    Ok(peers)
}

pub fn discover(app: &Host) -> Result<Vec<DiscoveredPeer>, String> {
    start(app);
    if let Some(Err(error)) = STARTED.get() {
        return Err(error.clone());
    }
    let state = app.state::<crate::AppState>();
    let id = state
        .network_link
        .lock()
        .map_err(|_| "Network Link is unavailable.")?
        .identity()
        .to_owned();
    scan(
        &id,
        SocketAddr::from((GROUP, PORT)),
        Duration::from_millis(900),
    )
}
