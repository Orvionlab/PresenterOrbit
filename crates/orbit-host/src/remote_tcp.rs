use super::{modern, Backend};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_FRAME: usize = 64 * 1024;
const MAX_CLIENTS: usize = 8;
const TICK: Duration = Duration::from_millis(100);
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

pub(in super::super) struct TcpServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TcpServer {
    pub(in super::super) fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }

    pub(in super::super) fn start(
        &mut self,
        backend: Arc<dyn Backend>,
        port: u16,
    ) -> Result<(), String> {
        let listener = TcpListener::bind(("0.0.0.0", port))
            .map_err(|error| format!("TCP/IP server could not start: {error}"))?;
        self.serve(listener, backend)
    }

    fn serve(&mut self, listener: TcpListener, backend: Arc<dyn Backend>) -> Result<(), String> {
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = Arc::clone(&stop);
        self.thread = Some(
            thread::Builder::new()
                .name("orbit-tcp".into())
                .spawn(move || {
                    let clients = Arc::new(AtomicUsize::new(0));
                    while !stop.load(Ordering::Acquire) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                if clients.fetch_add(1, Ordering::AcqRel) >= MAX_CLIENTS {
                                    clients.fetch_sub(1, Ordering::AcqRel);
                                    continue;
                                }
                                let (backend, stop, clients) = (
                                    Arc::clone(&backend),
                                    Arc::clone(&stop),
                                    Arc::clone(&clients),
                                );
                                let worker_clients = Arc::clone(&clients);
                                if thread::Builder::new()
                                    .name("orbit-tcp-client".into())
                                    .spawn(move || {
                                        struct Slot(Arc<AtomicUsize>);
                                        impl Drop for Slot {
                                            fn drop(&mut self) {
                                                self.0.fetch_sub(1, Ordering::AcqRel);
                                            }
                                        }
                                        let _slot = Slot(worker_clients);
                                        serve_connection(stream, backend.as_ref(), &stop);
                                    })
                                    .is_err()
                                {
                                    clients.fetch_sub(1, Ordering::AcqRel);
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(TICK)
                            }
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|error| error.to_string())?,
        );
        Ok(())
    }

    pub(in super::super) fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for TcpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn request(session: &mut modern::Session, backend: &dyn Backend, bytes: &[u8]) -> Value {
    let mut message = match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(object)) => Value::Object(object),
        _ => return json!({"url":"", "error":"A JSON object is required."}),
    };
    let original = message["url"].as_str().unwrap_or("").to_string();
    if message.get("method").is_none() {
        message["method"] = json!(if message.get("body").is_some() {
            "POST"
        } else {
            "GET"
        });
    }
    match session.receive_plain(backend, message) {
        Ok(mut reply) => {
            reply["url"] = json!(original);
            reply
        }
        Err(error) => json!({"url":original,"error":error}),
    }
}

fn send(stream: &mut TcpStream, reply: &Value) -> std::io::Result<()> {
    if reply.get("data") == Some(&Value::Null) {
        return Ok(());
    }
    stream.write_all(reply.to_string().as_bytes())?;
    stream.write_all(b"\r\n")
}

fn serve_connection(mut stream: TcpStream, backend: &dyn Backend, stop: &AtomicBool) {
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(TICK)).is_err()
        || stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .is_err()
    {
        return;
    }
    let mut session = modern::Session::new("");
    let mut frame = Vec::new();
    let mut bytes = [0; 4096];
    let mut frame_started = Instant::now();
    let mut last_tick = Instant::now();
    while !stop.load(Ordering::Acquire) {
        match stream.read(&mut bytes) {
            Ok(0) => return,
            Ok(count) => {
                for byte in &bytes[..count] {
                    if frame.is_empty() {
                        frame_started = Instant::now();
                    }
                    if *byte == b'\n' {
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        let reply = request(&mut session, backend, &frame);
                        frame.clear();
                        if send(&mut stream, &reply).is_err() {
                            return;
                        }
                    } else {
                        frame.push(*byte);
                        if frame.len() > MAX_FRAME {
                            return;
                        }
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
        if !frame.is_empty() && frame_started.elapsed() >= FRAME_TIMEOUT {
            return;
        }
        if last_tick.elapsed() >= Duration::from_millis(500) {
            last_tick = Instant::now();
            for reply in session.tick(backend) {
                if send(&mut stream, &reply).is_err() {
                    return;
                }
            }
        }
    }
}
