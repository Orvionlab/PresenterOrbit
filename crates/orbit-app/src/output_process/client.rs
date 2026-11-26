//! The UI side of the output process: finds a running one or starts one, attaches as its only controller, sends
//! host calls to it, and restarts it when it dies. Connection failures reach the operator as
//! `output:link-error` events; attach changes are reported as `output:link-changed`.

use std::collections::HashMap;
use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::protocol::{
    read_endpoint, read_message, write_message, Endpoint, FromOutput, ToOutput, VERSION,
};
use crate::locked::Locked;

/// How long a host call waits for the output process to answer.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a freshly started output process may take to publish its endpoint.
const START_TIMEOUT: Duration = Duration::from_secs(15);
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);
const EXIT_LOCKED: i32 = 3;

pub type EventSink = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// A started output process, however it was started.
pub trait Spawned: Send {
    fn exit_code(&mut self) -> Option<i32>;
    fn kill(&mut self);
}

impl Spawned for Child {
    fn exit_code(&mut self) -> Option<i32> {
        self.try_wait()
            .ok()
            .flatten()
            .map(|s| s.code().unwrap_or(-1))
    }
    fn kill(&mut self) {
        let _ = Child::kill(self);
        let _ = self.wait();
    }
}

pub trait Spawner: Send + Sync + 'static {
    fn spawn(&self, directory: &Path) -> Result<Box<dyn Spawned>, String>;
}

/// Starts `<executable> --output-process --data-dir <directory>` so it outlives this process.
pub struct ProcessSpawner {
    pub executable: PathBuf,
    pub extra_arguments: Vec<String>,
    /// Reports that the output process could not be detached from this process's job.
    pub events: EventSink,
}

#[cfg(windows)]
mod flags {
    pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    pub const DETACHED_PROCESS: u32 = 0x0000_0008;
    pub const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
}

impl ProcessSpawner {
    fn command(&self, directory: &Path) -> Result<Command, String> {
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("output-process.log"))
            .map_err(|error| format!("The output process log could not be opened: {error}"))?;
        let mut command = Command::new(&self.executable);
        command
            .arg("--output-process")
            .arg("--data-dir")
            .arg(directory)
            .args(&self.extra_arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log));
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        Ok(command)
    }
}

impl Spawner for ProcessSpawner {
    fn spawn(&self, directory: &Path) -> Result<Box<dyn Spawned>, String> {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let detached = flags::CREATE_NEW_PROCESS_GROUP | flags::DETACHED_PROCESS;
            let mut command = self.command(directory)?;
            command.creation_flags(detached | flags::CREATE_BREAKAWAY_FROM_JOB);
            match command.spawn() {
                Ok(child) => Ok(Box::new(child)),
                // ERROR_ACCESS_DENIED: the job forbids breakaway, so the output process stays in the job.
                Err(error) if error.raw_os_error() == Some(5) => {
                    (self.events)(
                        "output:display-error",
                        json!("The output process could not leave this application's job object, so it ends when the job ends."),
                    );
                    let mut command = self.command(directory)?;
                    command.creation_flags(detached);
                    command
                        .spawn()
                        .map(|child| Box::new(child) as Box<dyn Spawned>)
                        .map_err(|error| format!("The output process could not start: {error}"))
                }
                Err(error) => Err(format!("The output process could not start: {error}")),
            }
        }
        #[cfg(not(windows))]
        self.command(directory)?
            .spawn()
            .map(|child| Box::new(child) as Box<dyn Spawned>)
            .map_err(|error| format!("The output process could not start: {error}"))
    }
}

/// 0.5, 1, 2, 5 seconds, then every 10 seconds.
pub fn backoff(attempt: usize) -> Duration {
    Duration::from_millis(
        [500, 1000, 2000, 5000]
            .get(attempt)
            .copied()
            .unwrap_or(10_000),
    )
}

pub struct Config {
    pub directory: PathBuf,
    pub rendering_bit_depth: String,
    pub spawner: Arc<dyn Spawner>,
    pub events: EventSink,
}

struct Connection {
    writer: Mutex<TcpStream>,
    waiters: Mutex<HashMap<u64, Sender<Result<Value, String>>>>,
    closed: AtomicBool,
    has_scene: bool,
}

#[derive(Default)]
struct State {
    connection: Option<Arc<Connection>>,
    child: Option<Box<dyn Spawned>>,
}

struct Inner {
    config: Config,
    state: Mutex<State>,
    changed: Condvar,
    next_id: AtomicU64,
    quitting: AtomicBool,
}

pub struct OutputLink {
    inner: Arc<Inner>,
    supervisor: Mutex<Option<std::thread::JoinHandle<()>>>,
}

enum Attach {
    Attached(Arc<Connection>),
    /// This controller's previous socket has not finished detaching on the server yet.
    Draining,
    /// Another controller holds the output process.
    Foreign(u32),
    Rejected(String),
    /// Nothing is listening at the endpoint.
    Refused,
}

impl Inner {
    fn emit(&self, event: &str, payload: Value) {
        (self.config.events)(event, payload);
    }

    fn fail(&self, error: impl std::fmt::Display) {
        self.emit("output:link-error", json!(error.to_string()));
    }

    fn quitting(&self) -> bool {
        self.quitting.load(Ordering::Acquire)
    }

    fn attach(self: &Arc<Self>, endpoint: &Endpoint) -> Result<Attach, String> {
        let address = SocketAddr::from(([127, 0, 0, 1], endpoint.port));
        let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_secs(2)) else {
            return Ok(Attach::Refused);
        };
        let _ = stream.set_nodelay(true);
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|error| error.to_string())?;
        write_message(
            &mut stream,
            &ToOutput::Hello {
                version: VERSION,
                token: endpoint.token.clone(),
                controller_pid: std::process::id(),
                rendering_bit_depth: self.config.rendering_bit_depth.clone(),
            },
        )?;
        match read_message::<FromOutput>(&mut stream)? {
            FromOutput::Welcome { has_scene, .. } => {
                let _ = stream.set_read_timeout(None);
                let reader = stream.try_clone().map_err(|error| error.to_string())?;
                let connection = Arc::new(Connection {
                    writer: Mutex::new(stream),
                    waiters: Mutex::new(HashMap::new()),
                    closed: AtomicBool::new(false),
                    has_scene,
                });
                let inner = self.clone();
                let owned = connection.clone();
                std::thread::Builder::new()
                    .name("orbit-output-link-reader".into())
                    .spawn(move || inner.read(reader, owned))
                    .map_err(|error| error.to_string())?;
                Ok(Attach::Attached(connection))
            }
            FromOutput::Busy { controller_pid } if controller_pid == std::process::id() => {
                Ok(Attach::Draining)
            }
            FromOutput::Busy { controller_pid } => Ok(Attach::Foreign(controller_pid)),
            FromOutput::Reject { reason } => Ok(Attach::Rejected(reason)),
            other => Err(format!("The output process answered hello with {other:?}.")),
        }
    }

    fn read(self: Arc<Self>, mut stream: TcpStream, connection: Arc<Connection>) {
        let error = loop {
            match read_message::<FromOutput>(&mut stream) {
                Ok(FromOutput::Result { id, ok, error }) => {
                    if let Some(waiter) = connection.waiters.locked().remove(&id) {
                        let _ = waiter.send(error.map_or(Ok(ok.unwrap_or(Value::Null)), Err));
                    }
                }
                Ok(FromOutput::Event { event, payload }) => self.emit(&event, payload),
                Ok(_) => {}
                Err(error) => break error,
            }
        };
        connection.closed.store(true, Ordering::Release);
        // Other requests can still own the writer; close both halves to unblock them and release the lease.
        let _ = stream.shutdown(Shutdown::Both);
        for (_, waiter) in connection.waiters.locked().drain() {
            let _ = waiter.send(Err(format!(
                "The output process connection closed: {error}"
            )));
        }
        if !self.quitting() {
            self.fail(format!("The output process connection closed: {error}"));
        }
        let _guard = self.state.locked();
        self.changed.notify_all();
    }

    /// Reads the endpoint and attaches, or starts the output process and waits for its endpoint.
    fn connect(self: &Arc<Self>) -> Result<Attach, String> {
        let directory = &self.config.directory;
        if let Some(endpoint) = read_endpoint(directory)? {
            match self.attach(&endpoint)? {
                Attach::Refused => {}
                other => return Ok(other),
            }
        }
        if self.quitting() {
            return Ok(Attach::Refused);
        }
        let spawned = self.config.spawner.spawn(directory)?;
        self.state.locked().child = Some(spawned);
        let deadline = Instant::now() + START_TIMEOUT;
        let mut shutdown_deadline = None;
        while Instant::now() < deadline {
            // Once spawned, let the child publish its socket so a concurrent quit can shut it down normally.
            if self.quitting()
                && Instant::now()
                    >= *shutdown_deadline.get_or_insert_with(|| Instant::now() + SHUTDOWN_WAIT)
            {
                break;
            }
            if let Some(endpoint) = read_endpoint(directory)? {
                match self.attach(&endpoint)? {
                    Attach::Refused => {}
                    other => return Ok(other),
                }
            }
            let exited = self
                .state
                .locked()
                .child
                .as_mut()
                .and_then(|c| c.exit_code());
            // Exit code 3 means another controller won the start; its endpoint follows.
            if let Some(code) = exited.filter(|code| *code != EXIT_LOCKED) {
                self.state.locked().child = None;
                return Err(format!(
                    "The output process exited with code {code} while starting."
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err("The output process did not publish its endpoint in time.".into())
    }

    fn supervise(self: Arc<Self>) {
        let mut attempt = 0;
        while !self.quitting() {
            match self.connect() {
                Ok(Attach::Attached(connection)) => {
                    attempt = 0;
                    self.state.locked().connection = Some(connection.clone());
                    self.emit(
                        "output:link-changed",
                        json!({ "attached": true, "hasScene": connection.has_scene }),
                    );
                    let mut state = self.state.locked();
                    while !connection.closed.load(Ordering::Acquire) && !self.quitting() {
                        state = self
                            .changed
                            .wait_timeout(state, Duration::from_millis(200))
                            .map(|(state, _)| state)
                            .unwrap_or_else(|error| error.into_inner().0);
                    }
                    if self.quitting() {
                        return;
                    }
                    state.connection = None;
                    drop(state);
                    self.emit("output:link-changed", json!({ "attached": false }));
                }
                Ok(Attach::Draining) => {
                    self.emit(
                        "output:link-changed",
                        json!({ "attached": false, "reconnecting": true }),
                    );
                }
                Ok(Attach::Foreign(pid)) => {
                    self.fail(format!(
                        "Another Presenter Orbit (process {pid}) controls the output. This one only previews."
                    ));
                    self.emit(
                        "output:link-changed",
                        json!({ "attached": false, "foreign": true }),
                    );
                    return;
                }
                Ok(Attach::Rejected(reason)) => {
                    self.fail(format!(
                        "The output process refused this application: {reason}"
                    ));
                    self.emit(
                        "output:link-changed",
                        json!({ "attached": false, "foreign": true }),
                    );
                    return;
                }
                Ok(Attach::Refused) | Err(_) if self.quitting() => return,
                Ok(Attach::Refused) => self.fail("The output process refused the connection."),
                Err(error) => self.fail(error),
            }
            // A pause before the next start, sliced so a quit is not held up by it.
            let wake = Instant::now() + backoff(attempt);
            attempt += 1;
            while Instant::now() < wake && !self.quitting() {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

impl OutputLink {
    pub fn start(config: Config) -> Arc<Self> {
        let inner = Arc::new(Inner {
            config,
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            next_id: AtomicU64::new(1),
            quitting: AtomicBool::new(false),
        });
        let supervisor = inner.clone();
        let handle = std::thread::Builder::new()
            .name("orbit-output-link".into())
            .spawn(move || supervisor.supervise())
            .expect("the output link thread starts");
        Arc::new(Self {
            inner,
            supervisor: Mutex::new(Some(handle)),
        })
    }

    pub fn attached(&self) -> bool {
        self.inner
            .state
            .locked()
            .connection
            .as_ref()
            .is_some_and(|connection| !connection.closed.load(Ordering::Acquire))
    }

    /// One host call on the output process. Blocks until it answers.
    pub fn invoke(&self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        let connection = self
            .inner
            .state
            .locked()
            .connection
            .clone()
            .ok_or("The output process is not connected.")?;
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = channel();
        {
            let mut waiters = connection.waiters.locked();
            if connection.closed.load(Ordering::Acquire) {
                return Err("The output process connection is closed.".into());
            }
            waiters.insert(id, reply);
        }
        let sent = write_message(
            &mut *connection.writer.locked(),
            &ToOutput::Invoke {
                id,
                method: method.to_string(),
                args,
            },
        );
        if let Err(error) = sent {
            connection.waiters.locked().remove(&id);
            return Err(format!("The output process could not be reached: {error}"));
        }
        match answer.recv_timeout(CALL_TIMEOUT) {
            Ok(result) => result,
            Err(_) => {
                connection.waiters.locked().remove(&id);
                Err(format!(
                    "The output process did not answer '{method}' in time."
                ))
            }
        }
    }

    /// Ends the output process: asks it to shut down, waits briefly, and kills a process this link started if
    /// it is still running.
    pub fn shutdown(&self) {
        let mut supervisor = self.supervisor.locked();
        let Some(handle) = supervisor.take() else {
            return;
        };
        {
            let _state = self.inner.state.locked();
            self.inner.quitting.store(true, Ordering::Release);
            self.inner.changed.notify_all();
        }
        // Let any in-flight attach finish before taking ownership of the socket and child.
        let _ = handle.join();
        let connection = self.inner.state.locked().connection.take();
        if let Some(connection) = &connection {
            let _ = write_message(&mut *connection.writer.locked(), &ToOutput::Shutdown);
            let _ = connection.writer.locked().flush();
        }
        let deadline = Instant::now() + SHUTDOWN_WAIT;
        loop {
            let gone = match self.inner.state.locked().child.as_mut() {
                Some(child) => child.exit_code().is_some(),
                None => connection
                    .as_ref()
                    .is_none_or(|c| c.closed.load(Ordering::Acquire)),
            };
            if gone || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if let Some(mut child) = self.inner.state.locked().child.take() {
            if child.exit_code().is_none() {
                child.kill();
            }
        }
    }
}
