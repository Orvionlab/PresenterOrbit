use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{IpAddr, TcpStream},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::super::Backend;

#[derive(Default)]
pub(in super::super) struct Access(Mutex<HashMap<IpAddr, usize>>);

pub(super) struct SessionAccess {
    access: Arc<Access>,
    peer: IpAddr,
}

impl Access {
    pub(super) fn authorize(self: &Arc<Self>, peer: IpAddr) -> Result<SessionAccess, String> {
        let mut peers = self
            .0
            .lock()
            .map_err(|_| "Remote image access is unavailable.")?;
        *peers.entry(peer).or_default() += 1;
        Ok(SessionAccess {
            access: Arc::clone(self),
            peer,
        })
    }

    fn allows(&self, peer: IpAddr) -> bool {
        self.0.lock().is_ok_and(|peers| peers.contains_key(&peer))
    }
}

impl Drop for SessionAccess {
    fn drop(&mut self) {
        if let Ok(mut peers) = self.access.0.lock() {
            if let Some(count) = peers.get_mut(&self.peer) {
                *count -= 1;
                if *count == 0 {
                    peers.remove(&self.peer);
                }
            }
        }
    }
}

const MAX_HEADERS: usize = 8192;
const HEADER_TIMEOUT: Duration = Duration::from_secs(5);

pub(in super::super) fn serve_thumbnail(
    stream: &mut TcpStream,
    backend: &dyn Backend,
    access: &Access,
) -> bool {
    let deadline = Instant::now() + HEADER_TIMEOUT;
    let mut bytes = [0; MAX_HEADERS];
    let target = loop {
        let Ok(count) = stream.peek(&mut bytes) else {
            return false;
        };
        if let Some(end) = bytes[..count].windows(2).position(|part| part == b"\r\n") {
            let Ok(line) = std::str::from_utf8(&bytes[..end]) else {
                return false;
            };
            let mut parts = line.split(' ');
            let (Some("GET"), Some(target), Some("HTTP/1.1" | "HTTP/1.0"), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return false;
            };
            if !target.starts_with("/v1/")
                || !target.split('?').next().is_some_and(|path| {
                    path.contains("/thumbnail/")
                        || path.ends_with("/thumbnail")
                        || path.ends_with("/icon")
                })
            {
                return false;
            }
            break target.to_string();
        }
        if count == 0 || count == MAX_HEADERS || Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let accept = match read_headers(stream, deadline) {
        Ok(accept) => accept,
        Err(()) => {
            write_response(
                stream,
                "400 Bad Request",
                "text/plain",
                b"Invalid image request.",
            );
            return true;
        }
    };
    if !stream
        .peer_addr()
        .is_ok_and(|peer| access.allows(peer.ip()))
    {
        trace_prop_image(&target, "403 Forbidden");
        write_response(
            stream,
            "403 Forbidden",
            "text/plain",
            b"An active authenticated Remote connection is required.",
        );
        return true;
    }
    match super::thumbnail::handle(backend, target.trim_start_matches('/'), accept.as_deref()) {
        Ok(image) => {
            trace_prop_image(&target, "200 OK");
            write_response(stream, "200 OK", image.content_type, &image.body)
        }
        Err(error) => {
            trace_prop_image(&target, &format!("400 Bad Request: {error}"));
            write_response(
                stream,
                "400 Bad Request",
                "text/plain",
                b"The requested slide image is unavailable.",
            )
        }
    }
    true
}

fn trace_prop_image(target: &str, status: &str) {
    #[cfg(debug_assertions)]
    if std::env::var_os("PRESENTER_ORBIT_REMOTE_TRACE").is_some() && target.starts_with("/v1/prop/")
    {
        eprintln!(
            "[modern-remote] prop HTTP GET {} {status}",
            target.split('?').next().unwrap_or(target)
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (target, status);
}

fn read_headers(stream: &mut TcpStream, deadline: Instant) -> Result<Option<String>, ()> {
    let mut bytes = [0; MAX_HEADERS];
    let mut used = 0;
    loop {
        let remaining = deadline.checked_duration_since(Instant::now()).ok_or(())?;
        stream.set_read_timeout(Some(remaining)).map_err(|_| ())?;
        let count = stream.read(&mut bytes[used..]).map_err(|_| ())?;
        if count == 0 {
            return Err(());
        }
        used += count;
        if let Some(end) = bytes[..used]
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
        {
            let header = std::str::from_utf8(&bytes[..end]).map_err(|_| ())?;
            let mut accept = Vec::new();
            for line in header.split("\r\n").skip(1) {
                let (name, value) = line.split_once(':').ok_or(())?;
                if name.eq_ignore_ascii_case("accept") {
                    accept.push(value.trim());
                }
            }
            return Ok((!accept.is_empty()).then(|| accept.join(",")));
        }
        if used == MAX_HEADERS {
            return Err(());
        }
    }
}

fn write_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let header = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    if stream.write_all(header.as_bytes()).is_ok() {
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }
}
