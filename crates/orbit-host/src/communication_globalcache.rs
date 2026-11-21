use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Command {
    protocol: String,
    command: String,
    module: u8,
    output: u8,
    interval_ms: Option<u32>,
}

type EndpointLocks = HashMap<SocketAddr, Weak<Mutex<()>>>;
static ENDPOINTS: OnceLock<Mutex<EndpointLocks>> = OnceLock::new();

fn parse(value: Value) -> Result<Command, String> {
    let command: Command = serde_json::from_value(value)
        .map_err(|error| format!("Invalid GlobalCache command: {error}"))?;
    let timed = matches!(command.command.as_str(), "onThenOff" | "offThenOn");
    if command.protocol != "globalcache-relay"
        || command.module == 0
        || !(1..=6).contains(&command.output)
        || !matches!(
            command.command.as_str(),
            "on" | "off" | "onThenOff" | "offThenOn"
        )
        || (timed
            && !command
                .interval_ms
                .is_some_and(|interval| (1..=86_400_000).contains(&interval)))
        || (!timed && command.interval_ms.is_some())
    {
        return Err("Invalid GlobalCache relay parameters.".into());
    }
    Ok(command)
}

fn endpoint_lock(address: SocketAddr) -> Result<Arc<Mutex<()>>, String> {
    let mut endpoints = ENDPOINTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "GlobalCache connection registry is unavailable.")?;
    endpoints.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = endpoints.get(&address).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(Mutex::new(()));
    endpoints.insert(address, Arc::downgrade(&lock));
    Ok(lock)
}

fn response(stream: &mut TcpStream) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut bytes = Vec::new();
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("GlobalCache response timed out.")?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|error| error.to_string())?;
        let mut byte = [0];
        stream
            .read_exact(&mut byte)
            .map_err(|error| format!("GlobalCache response failed: {error}"))?;
        if byte[0] == b'\r' {
            break;
        }
        if !byte[0].is_ascii_graphic() && byte[0] != b' ' {
            return Err("Invalid GlobalCache response framing.".into());
        }
        bytes.push(byte[0]);
        if bytes.len() > 256 {
            return Err("GlobalCache response is too large.".into());
        }
    }
    String::from_utf8(bytes).map_err(|_| "GlobalCache response is not ASCII.".into())
}

fn set_state(stream: &mut TcpStream, command: &Command, state: u8) -> Result<usize, String> {
    let frame = format!("setstate,{}:{},{}\r", command.module, command.output, state);
    stream
        .write_all(frame.as_bytes())
        .map_err(|error| format!("GlobalCache write failed: {error}"))?;
    let reply = response(stream)?;
    let expected = format!("state,{}:{},{}", command.module, command.output, state);
    if reply != expected {
        return Err(format!(
            "GlobalCache did not acknowledge the requested relay state: {reply}"
        ));
    }
    Ok(frame.len())
}

pub(crate) fn send(device: &Value, payload: Value) -> Result<Value, String> {
    crate::communication::validate(device)?;
    if device["transport"] != "tcp" {
        return Err("GlobalCache relay control requires TCP.".into());
    }
    let command = parse(payload)?;
    let address = (
        device["address"].as_str().expect("validated host"),
        device["port"].as_u64().expect("validated port") as u16,
    )
        .to_socket_addrs()
        .map_err(|error| error.to_string())?
        .next()
        .ok_or("The device host resolved to no address.")?;
    // GC-100 permits one connection; serialize whole operations per resolved endpoint.
    let connection = endpoint_lock(address)?;
    let _operation = connection
        .try_lock()
        .map_err(|_| "GlobalCache device is busy with another operation.")?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    let initial = u8::from(matches!(command.command.as_str(), "on" | "onThenOff"));
    let mut sent = set_state(&mut stream, &command, initial)?;
    let mut steps = 1;
    if let Some(interval) = command.interval_ms {
        std::thread::sleep(Duration::from_millis(u64::from(interval)));
        sent += set_state(&mut stream, &command, 1 - initial)?;
        steps = 2;
    }
    Ok(json!({"ok":true,"acknowledged":true,"bytesSent":sent,"steps":steps}))
}
