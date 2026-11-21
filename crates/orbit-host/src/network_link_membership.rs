use crate::Host;
use std::{net::IpAddr, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::network_link::{NetworkLinkPeer, NetworkLinkSettings};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JoinRequest {
    host: String,
    port: u16,
    password: String,
    group: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Invitation {
    protocol: String,
    id: String,
    title: String,
    port: u16,
    password: String,
    group: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JoinReply {
    ok: bool,
    protocol: String,
    id: String,
    title: String,
    group: String,
}

pub fn handles(method: &str) -> bool {
    matches!(method, "discoverNetworkLinkPeers" | "joinNetworkLinkGroup")
}

pub fn run(app: &Host, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "discoverNetworkLinkPeers" => {
            serde_json::to_value(crate::network_link_discovery::discover(app)?)
                .map_err(|error| error.to_string())
        }
        "joinNetworkLinkGroup" => {
            let request: JoinRequest =
                serde_json::from_value(args.first().ok_or("A join request is required.")?.clone())
                    .map_err(|error| format!("Invalid Network Link join request: {error}"))?;
            serde_json::to_value(join(app, request)?).map_err(|error| error.to_string())
        }
        _ => Err("Unknown Network Link membership command.".into()),
    }
}

fn check_group(settings: &NetworkLinkSettings, group: &str) -> Result<(), String> {
    if (settings.enabled || !settings.peers.is_empty()) && settings.group != group {
        return Err("Leave the current Network Link group before joining another group.".into());
    }
    Ok(())
}

fn add_peer(
    mut settings: NetworkLinkSettings,
    group: &str,
    peer: NetworkLinkPeer,
) -> Result<NetworkLinkSettings, String> {
    check_group(&settings, group)?;
    settings.enabled = true;
    settings.group = group.into();
    let existing = settings.peers.iter().position(|current| {
        current.id == peer.id || (current.host == peer.host && current.port == peer.port)
    });
    if let Some(index) = existing {
        settings.peers[index] = peer;
    } else {
        settings.peers.push(peer);
    }
    crate::network_link::validate_settings(settings)
}

fn validate_identity(id: &str) -> Result<(), String> {
    let parsed =
        uuid::Uuid::parse_str(id).map_err(|_| "A valid Network Link identity is required.")?;
    if parsed.hyphenated().to_string() != id {
        return Err("A canonical Network Link identity is required.".into());
    }
    Ok(())
}

fn join(app: &Host, request: JoinRequest) -> Result<NetworkLinkSettings, String> {
    let state = app.state::<crate::AppState>();
    let (id, settings) = {
        let link = state
            .network_link
            .lock()
            .map_err(|_| "Network Link state is unavailable.")?;
        (link.identity().to_owned(), link.settings())
    };
    check_group(&settings, &request.group)?;
    let peer = NetworkLinkPeer {
        id: uuid::Uuid::new_v4().to_string(),
        title: request.host.clone(),
        host: request.host.clone(),
        port: request.port,
        password: request.password.clone(),
    };
    crate::network_link::validate_settings(NetworkLinkSettings {
        enabled: true,
        group: request.group.clone(),
        peers: vec![peer],
    })?;
    let (port, password) = state
        .remote_server
        .lock()
        .map_err(|_| "Remote server is unavailable.")?
        .link_connection()?
        .ok_or("Enable password-protected Remote Control before joining a Network Link group.")?;
    let invitation = Invitation {
        protocol: "orbit-link-membership/1".into(),
        id: id.clone(),
        title: crate::network_link_discovery::title(),
        port,
        password,
        group: request.group.clone(),
    };
    let mut endpoint = url::Url::parse("http://localhost/").map_err(|error| error.to_string())?;
    let endpoint_host = if request
        .host
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_ipv6())
    {
        format!("[{}]", request.host)
    } else {
        request.host.clone()
    };
    endpoint
        .set_host(Some(&endpoint_host))
        .map_err(|_| "Invalid peer host.")?;
    endpoint
        .set_port(Some(request.port))
        .map_err(|_| "Invalid peer port.")?;
    endpoint.set_path("/api/v1/link/join");
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(3)))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut response = agent
        .post(endpoint.as_str())
        .header(
            "Authorization",
            &format!(
                "Basic {}",
                crate::remote::base64_basic(&format!("presenter-orbit:{}", request.password))
            ),
        )
        .header("Content-Type", "application/json")
        .send(serde_json::to_vec(&invitation).map_err(|error| error.to_string())?)
        .map_err(|_| "Network Link group connection failed.".to_string())?;
    if response.status().as_u16() != 200 {
        return Err(format!(
            "Network Link group connection was rejected (HTTP {}).",
            response.status().as_u16()
        ));
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(4096)
        .read_to_vec()
        .map_err(|_| "Network Link group reply could not be read.".to_string())?;
    let reply: JoinReply = serde_json::from_slice(&body)
        .map_err(|_| "Network Link group reply is invalid.".to_string())?;
    validate_identity(&reply.id)?;
    if !reply.ok
        || reply.protocol != "orbit-link-membership/1"
        || reply.group != request.group
        || reply.id == id
    {
        return Err("Network Link group reply does not match the requested group or peer.".into());
    }
    let peer = NetworkLinkPeer {
        id: reply.id,
        title: reply.title,
        host: request.host,
        port: request.port,
        password: request.password,
    };
    crate::network_link_commands::update_settings(app, &state, |current| {
        add_peer(current, &request.group, peer)
    })
}

pub fn accept(app: &Host, source: IpAddr, body: &[u8]) -> Result<Value, String> {
    if body.len() > 8192 || source.is_unspecified() || source.is_multicast() {
        return Err("Invalid Network Link group invitation.".into());
    }
    let invitation: Invitation = serde_json::from_slice(body)
        .map_err(|_| "Invalid Network Link group invitation.".to_string())?;
    validate_identity(&invitation.id)?;
    if invitation.protocol != "orbit-link-membership/1" {
        return Err("Unsupported Network Link membership protocol.".into());
    }
    let state = app.state::<crate::AppState>();
    let id = state
        .network_link
        .lock()
        .map_err(|_| "Network Link is unavailable.")?
        .identity()
        .to_owned();
    if invitation.id == id {
        return Err("A computer cannot join itself.".into());
    }
    let peer = NetworkLinkPeer {
        id: invitation.id,
        title: invitation.title,
        host: source.to_string(),
        port: invitation.port,
        password: invitation.password,
    };
    let settings = crate::network_link_commands::update_settings(app, &state, |current| {
        add_peer(current, &invitation.group, peer)
    })?;
    serde_json::to_value(JoinReply {
        ok: true,
        protocol: "orbit-link-membership/1".into(),
        id,
        title: crate::network_link_discovery::title(),
        group: settings.group,
    })
    .map_err(|error| error.to_string())
}
