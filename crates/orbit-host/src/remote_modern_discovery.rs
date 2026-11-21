use std::time::{Duration, Instant};

use mdns_sd::{DaemonEvent, ServiceDaemon, ServiceInfo};

const SERVICE_TYPE: &str = "_proapiv1ws._tcp.local.";
const INSTANCE_PREFIX: &str = "Presenter Orbit";
const ANNOUNCE_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) struct Discovery {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Discovery {
    pub(super) fn start(port: u16, name: &str) -> Result<Self, String> {
        let service = service_info(port, name)?;
        let requested_fullname = service.get_fullname().to_string();
        let daemon = ServiceDaemon::new()
            .map_err(|error| format!("mDNS discovery could not start: {error}"))?;
        let monitor = match daemon.monitor() {
            Ok(monitor) => monitor,
            Err(error) => {
                stop_daemon(&daemon);
                return Err(format!("mDNS discovery could not be monitored: {error}"));
            }
        };
        if let Err(error) = daemon.register(service) {
            stop_daemon(&daemon);
            return Err(format!("mDNS service registration failed: {error}"));
        }
        let fullname = match wait_for_announcement(|timeout| {
            monitor.recv_timeout(timeout).map_err(|_| {
                "mDNS service advertisement was not confirmed within 3 seconds.".to_string()
            })
        }) {
            Ok(fullname) => fullname,
            Err(error) => {
                let _ = daemon.unregister(&requested_fullname);
                stop_daemon(&daemon);
                return Err(error);
            }
        };
        Ok(Self { daemon, fullname })
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        stop_daemon(&self.daemon);
    }
}

fn service_info(port: u16, name: &str) -> Result<ServiceInfo, String> {
    if port == 0 {
        return Err("A nonzero Remote port is required for mDNS discovery.".into());
    }
    let title = name;
    let instance = instance_name(&title);
    let hostname = format!("{}.local.", hostname_label(&title));
    ServiceInfo::new(
        SERVICE_TYPE,
        &instance,
        &hostname,
        "",
        port,
        &[("path", "/v1/ws")][..],
    )
    .map(ServiceInfo::enable_addr_auto)
    .map_err(|error| format!("mDNS service description is invalid: {error}"))
}

fn instance_name(title: &str) -> String {
    let title: String = title
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let title = title.trim();
    let mut name = if title.is_empty() {
        INSTANCE_PREFIX.to_string()
    } else {
        title.to_string()
    };
    truncate_utf8(&mut name, 63);
    while name.ends_with(' ') {
        name.pop();
    }
    name
}

fn hostname_label(title: &str) -> String {
    let mut label = String::from("presenter-orbit-");
    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            label.push(character.to_ascii_lowercase());
        } else if !label.is_empty() && !label.ends_with('-') {
            label.push('-');
        }
    }
    label.truncate(63);
    while label.ends_with('-') {
        label.pop();
    }
    label
}

fn truncate_utf8(value: &mut String, max_bytes: usize) {
    if value.len() > max_bytes {
        let mut boundary = max_bytes;
        while !value.is_char_boundary(boundary) {
            boundary -= 1;
        }
        value.truncate(boundary);
    }
}

fn wait_for_announcement(
    mut receive: impl FnMut(Duration) -> Result<DaemonEvent, String>,
) -> Result<String, String> {
    let deadline = Instant::now() + ANNOUNCE_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("mDNS service advertisement was not confirmed within 3 seconds.".into());
        }
        let event = receive(remaining)?;
        match event {
            DaemonEvent::Announce(fullname, _) if is_orbit_service(&fullname) => {
                return Ok(fullname)
            }
            DaemonEvent::Error(error) => {
                return Err(format!("mDNS service advertisement failed: {error}"))
            }
            _ => {}
        }
    }
}

fn is_orbit_service(fullname: &str) -> bool {
    let fullname = fullname.to_ascii_lowercase();
    let service_suffix = format!(".{SERVICE_TYPE}");
    let Some(instance) = fullname.strip_suffix(&service_suffix) else {
        return false;
    };
    !instance.is_empty()
}

fn stop_daemon(daemon: &ServiceDaemon) {
    if let Ok(stopped) = daemon.shutdown() {
        let _ = stopped.recv_timeout(Duration::from_secs(2));
    }
}
