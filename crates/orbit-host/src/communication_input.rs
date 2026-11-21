use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{IpAddr, TcpListener, UdpSocket},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(crate) struct Reply {
    pub bytes: Vec<u8>,
    pub error: Option<String>,
    pub close: bool,
}
impl From<Vec<u8>> for Reply {
    fn from(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            error: None,
            close: false,
        }
    }
}
pub(crate) type Exchange = Box<dyn FnMut(&str, &[u8]) -> Result<Reply, String> + Send>;
pub(crate) type ExchangeFactory = Arc<dyn Fn() -> Exchange + Send + Sync>;
pub(crate) type Decoder = Box<dyn FnMut(&[u8]) -> Result<Vec<Vec<u8>>, String> + Send>;
pub(crate) type DecoderFactory = Arc<dyn Fn() -> Decoder + Send + Sync>;
pub(crate) type Changed = Arc<dyn Fn(Value) + Send + Sync>;
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct Session {
    device: Value,
    workspace: String,
    alive: Arc<AtomicBool>,
    gate: Arc<Mutex<()>>,
    status: Arc<Mutex<Value>>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    fn stop(&mut self) {
        self.alive.store(false, Ordering::Release);
        if let Ok(_gate) = self.gate.lock() {}
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
pub(crate) struct Inputs {
    sessions: HashMap<String, Session>,
}

impl Inputs {
    pub(crate) fn status(&self) -> Value {
        Value::Array(
            self.sessions
                .values()
                .filter_map(|session| session.status.lock().ok().map(|status| status.clone()))
                .collect(),
        )
    }

    pub(crate) fn accepts(&self, workspace: &str, device: &Value, identity: &str) -> bool {
        device["id"]
            .as_str()
            .and_then(|id| self.sessions.get(id))
            .is_some_and(|session| {
                session.workspace == workspace
                    && session.device == *device
                    && session.alive.load(Ordering::Acquire)
                    && session.status.lock().is_ok_and(|status| {
                        status["sessionId"] == identity && status["listening"] == true
                    })
            })
    }

    pub(crate) fn stop(&mut self, id: &str) {
        if let Some(mut session) = self.sessions.remove(id) {
            session.stop();
        }
    }

    pub(crate) fn stop_all(&mut self) {
        for (_, mut session) in self.sessions.drain() {
            session.stop();
        }
    }

    pub(crate) fn start(
        &mut self,
        workspace: &str,
        device: Value,
        factory: DecoderFactory,
        exchange: ExchangeFactory,
        changed: Changed,
    ) -> Result<Value, String> {
        crate::communication::validate(&device)?;
        let id = device["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("A communication device identity is required.")?
            .to_string();
        if self
            .sessions
            .get(&id)
            .is_some_and(|session| !session.alive.load(Ordering::Acquire))
        {
            self.stop(&id);
        }
        if self.sessions.contains_key(&id) {
            return Err(
                "The communication input is already connected. Disconnect before reconnecting."
                    .into(),
            );
        }
        let endpoint = if device["transport"] == "serial" {
            Endpoint::Serial(crate::communication_serial::open(&device)?)
        } else {
            let ip: IpAddr = device["address"]
                .as_str()
                .ok_or("A local bind address is required.")?
                .parse()
                .map_err(|_| "Communication inputs require an explicit local IP bind address.")?;
            let port = device["port"]
                .as_u64()
                .ok_or("An input port is required.")? as u16;
            if device["transport"] == "tcp" {
                let listener = TcpListener::bind((ip, port)).map_err(|error| error.to_string())?;
                listener
                    .set_nonblocking(true)
                    .map_err(|error| error.to_string())?;
                Endpoint::Tcp(listener)
            } else {
                if device["input"]["protocol"] == "artnet" {
                    Endpoint::ArtNet(device.clone())
                } else {
                    let socket = UdpSocket::bind((ip, port)).map_err(|error| error.to_string())?;
                    socket
                        .set_read_timeout(Some(Duration::from_millis(100)))
                        .map_err(|error| error.to_string())?;
                    Endpoint::Udp(socket)
                }
            }
        };
        let identity = SEQUENCE.fetch_add(1, Ordering::Relaxed).to_string();
        let listening = !matches!(&endpoint, Endpoint::ArtNet(_));
        let reconnecting = !listening && device["input"]["autoReconnect"] == true;
        let status = Arc::new(Mutex::new(
            json!({"deviceId":id,"sessionId":identity,"workspacePath":workspace,"listening":listening,"reconnecting":reconnecting,"received":0,"error":null}),
        ));
        let alive = Arc::new(AtomicBool::new(true));
        let gate = Arc::new(Mutex::new(()));
        let worker = Worker {
            alive: alive.clone(),
            gate: gate.clone(),
            status: status.clone(),
            exchange,
            changed,
        };
        let thread = thread::Builder::new()
            .name("production-input".into())
            .spawn(move || worker.run(endpoint, factory))
            .map_err(|error| error.to_string())?;
        let result = status
            .lock()
            .map_err(|_| "Communication status is unavailable.")?
            .clone();
        self.sessions.insert(
            id,
            Session {
                device,
                workspace: workspace.into(),
                alive,
                gate,
                status,
                thread: Some(thread),
            },
        );
        Ok(result)
    }
}

impl Drop for Inputs {
    fn drop(&mut self) {
        self.stop_all();
    }
}
enum Endpoint {
    Tcp(TcpListener),
    Udp(UdpSocket),
    Serial(Box<dyn serialport::SerialPort>),
    ArtNet(Value),
}

struct Worker {
    alive: Arc<AtomicBool>,
    gate: Arc<Mutex<()>>,
    status: Arc<Mutex<Value>>,
    exchange: ExchangeFactory,
    changed: Changed,
}
impl Worker {
    fn notify(&self, error: Option<String>, received: bool) {
        if let Ok(mut status) = self.status.lock() {
            status["error"] = error.map_or(Value::Null, Value::String);
            if received {
                status["received"] = json!(status["received"].as_u64().unwrap_or(0) + 1);
            }
            (self.changed)(status.clone());
        }
    }
    fn exchange(&self, handler: &mut Exchange, frame: &[u8]) -> Result<Reply, String> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| "Communication input is unavailable.")?;
        if !self.alive.load(Ordering::Acquire) {
            return Err("Communication input was disconnected.".into());
        }
        let identity = self
            .status
            .lock()
            .map_err(|_| "Communication input status is unavailable.")?["sessionId"]
            .as_str()
            .ok_or("Communication input session is unavailable.")?
            .to_string();
        let result = handler(&identity, frame);
        self.notify(
            match &result {
                Ok(reply) => reply.error.clone(),
                Err(error) => Some(error.clone()),
            },
            true,
        );
        result
    }
    fn run(&self, endpoint: Endpoint, factory: DecoderFactory) {
        match endpoint {
            Endpoint::ArtNet(device) => artnet_socket::run(self, &device, factory),
            Endpoint::Tcp(listener) => {
                // A single controller owns this connection; commands retain wire order.
                while self.alive.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            if let Err(error) = stream
                                .set_read_timeout(Some(Duration::from_millis(100)))
                                .and_then(|_| {
                                    stream.set_write_timeout(Some(Duration::from_secs(2)))
                                })
                            {
                                self.notify(Some(error.to_string()), false);
                                continue;
                            }
                            self.stream(&mut stream, factory());
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20))
                        }
                        Err(error) => {
                            self.notify(Some(error.to_string()), false);
                            break;
                        }
                    }
                }
            }
            Endpoint::Serial(mut port) => {
                if let Err(error) = port.set_timeout(Duration::from_millis(100)) {
                    self.notify(Some(error.to_string()), false);
                } else {
                    self.stream(&mut port, factory());
                }
            }
            Endpoint::Udp(socket) => {
                let mut bytes = [0; 65536];
                let mut peers = HashMap::new();
                while self.alive.load(Ordering::Acquire) {
                    match socket.recv_from(&mut bytes) {
                        Ok((size, peer)) => {
                            if !peers.contains_key(&peer) && peers.len() >= 16 {
                                self.notify(Some("The communication input has reached its 16 UDP controller limit. Reconnect to reset controllers.".into()),false);
                                continue;
                            }
                            let handler = peers.entry(peer).or_insert_with(|| (self.exchange)());
                            let mut decoder = factory();
                            match decoder(&bytes[..size]) {
                                Ok(frames) if frames.len() == 1 && decoder(&[]).is_ok() => {
                                    match self.exchange(handler, &frames[0]) {
                                        Ok(reply) if !reply.bytes.is_empty() => {
                                            if let Err(error) = socket.send_to(&reply.bytes, peer) {
                                                self.notify(Some(error.to_string()), false);
                                            }
                                        }
                                        Ok(_) => {}
                                        Err(error) => self.notify(Some(error), false),
                                    }
                                }
                                Ok(_) => self.notify(
                                    Some(
                                        "A UDP datagram must contain exactly one complete command."
                                            .into(),
                                    ),
                                    false,
                                ),
                                Err(error) => self.notify(Some(error), false),
                            }
                        }
                        Err(error) if timeout(&error) => {}
                        Err(error) => {
                            self.notify(Some(error.to_string()), false);
                            break;
                        }
                    }
                }
            }
        }
        self.alive.store(false, Ordering::Release);
        if let Ok(mut status) = self.status.lock() {
            status["listening"] = json!(false);
            status["reconnecting"] = json!(false);
            (self.changed)(status.clone());
        }
    }
    fn stream(&self, stream: &mut (impl Read + Write), mut decode: Decoder) {
        let mut bytes = [0; 4096];
        let mut last_data = Instant::now();
        let mut exchange = (self.exchange)();
        while self.alive.load(Ordering::Acquire) {
            match stream.read(&mut bytes) {
                Ok(0) => break,
                Ok(size) => {
                    last_data = Instant::now();
                    let frames = match decode(&bytes[..size]) {
                        Ok(frames) => frames,
                        Err(error) => {
                            self.notify(Some(error), false);
                            break;
                        }
                    };
                    for frame in frames {
                        match self.exchange(&mut exchange, &frame) {
                            Ok(reply) => {
                                if let Err(error) = stream.write_all(&reply.bytes) {
                                    self.notify(Some(error.to_string()), false);
                                    return;
                                }
                                if reply.close {
                                    return;
                                }
                            }
                            Err(error) => {
                                self.notify(Some(error), false);
                                return;
                            }
                        }
                    }
                }
                Err(error) if timeout(&error) => {
                    if last_data.elapsed() > Duration::from_secs(2) {
                        // Empty input flush asks the decoder to reject an unfinished frame.
                        if let Err(error) = decode(&[]) {
                            self.notify(Some(error), false);
                            break;
                        }
                    }
                }
                Err(error) => {
                    self.notify(Some(error.to_string()), false);
                    break;
                }
            }
        }
    }
}
fn timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

#[path = "communication_artnet_socket.rs"]
mod artnet_socket;
