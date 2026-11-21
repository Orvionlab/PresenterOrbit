use midir::{Ignore, MidiInput, MidiInputConnection};
use serde::Serialize;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const CLIENT_NAME: &str = "Presenter Orbit";
/// midir has no hotplug notification on any backend, so ports are rescanned.
const SCAN_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiInputStatus {
    pub enabled: bool,
    pub devices: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct MidiNote {
    pub channel: u8,
    pub pitch: u8,
    pub velocity: u8,
}

/// Note-on with velocity zero is a note-off by MIDI convention.
pub fn note_on(message: &[u8]) -> Option<MidiNote> {
    match message {
        [status, pitch, velocity, ..]
            if status & 0xf0 == 0x90 && *pitch < 0x80 && (1..0x80).contains(velocity) =>
        {
            Some(MidiNote {
                channel: (status & 0x0f) + 1,
                pitch: *pitch,
                velocity: *velocity,
            })
        }
        _ => None,
    }
}

pub enum MidiInputEvent {
    Note(MidiNote),
    Status(MidiInputStatus),
}

pub type MidiInputSink = Arc<dyn Fn(MidiInputEvent) + Send + Sync>;

struct Worker {
    stop: Sender<()>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
pub struct MidiInputService {
    worker: Mutex<Option<Worker>>,
    status: Arc<Mutex<MidiInputStatus>>,
}

impl MidiInputService {
    pub fn status(&self) -> Result<MidiInputStatus, String> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| "MIDI input status is unavailable.".to_string())
    }

    pub fn set_enabled(
        &self,
        sink: &MidiInputSink,
        enabled: bool,
    ) -> Result<MidiInputStatus, String> {
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| "MIDI input service is unavailable.".to_string())?;
        if enabled == worker.is_some() {
            return self.status();
        }
        if let Some(Worker { stop, handle }) = worker.take() {
            drop(stop);
            handle
                .join()
                .map_err(|_| "MIDI input watcher stopped unexpectedly.".to_string())?;
            publish(sink, &self.status, MidiInputStatus::default());
            return self.status();
        }
        let (stop, stopped) = mpsc::channel();
        let status = self.status.clone();
        let watcher_sink = sink.clone();
        publish(
            sink,
            &status,
            MidiInputStatus {
                enabled: true,
                ..MidiInputStatus::default()
            },
        );
        let handle = thread::Builder::new()
            .name("presenter-orbit-midi-input".into())
            .spawn(move || watch(&watcher_sink, &status, &stopped))
            .map_err(|error| format!("MIDI input watcher could not start: {error}"))?;
        *worker = Some(Worker { stop, handle });
        self.status()
    }
}

fn publish(sink: &MidiInputSink, shared: &Mutex<MidiInputStatus>, next: MidiInputStatus) {
    let Ok(mut status) = shared.lock() else {
        return;
    };
    if *status != next {
        *status = next.clone();
        drop(status);
        sink(MidiInputEvent::Status(next));
    }
}

struct Connection {
    id: String,
    name: String,
    _connection: MidiInputConnection<()>,
}

fn watch(sink: &MidiInputSink, status: &Mutex<MidiInputStatus>, stopped: &mpsc::Receiver<()>) {
    let mut connections = Vec::new();
    let scanner = MidiInput::new(CLIENT_NAME);
    loop {
        let errors = match &scanner {
            Ok(scanner) => scan(sink, scanner, &mut connections),
            Err(error) => vec![format!("MIDI input is unavailable: {error}")],
        };
        publish(
            sink,
            status,
            MidiInputStatus {
                enabled: true,
                devices: connections.iter().map(|entry| entry.name.clone()).collect(),
                error: (!errors.is_empty()).then(|| errors.join(" ")),
            },
        );
        if !matches!(
            stopped.recv_timeout(SCAN_INTERVAL),
            Err(RecvTimeoutError::Timeout)
        ) {
            return;
        }
    }
}

fn scan(
    sink: &MidiInputSink,
    scanner: &MidiInput,
    connections: &mut Vec<Connection>,
) -> Vec<String> {
    let ports = scanner.ports();
    let ids = ports.iter().map(|port| port.id()).collect::<Vec<_>>();
    connections.retain(|entry| ids.contains(&entry.id));
    let mut errors = Vec::new();
    for (port, id) in ports.iter().zip(ids) {
        if connections.iter().any(|entry| entry.id == id) {
            continue;
        }
        let name = match scanner.port_name(port) {
            Ok(name) => name,
            Err(error) => {
                errors.push(format!("A MIDI input name could not be read: {error}"));
                continue;
            }
        };
        let mut input = match MidiInput::new(CLIENT_NAME) {
            Ok(input) => input,
            Err(error) => {
                errors.push(format!("{name} could not be opened: {error}"));
                continue;
            }
        };
        input.ignore(Ignore::All);
        let note_sink = sink.clone();
        match input.connect(
            port,
            "presenter-orbit-triggers",
            move |_, message, _| {
                if let Some(note) = note_on(message) {
                    note_sink(MidiInputEvent::Note(note));
                }
            },
            (),
        ) {
            Ok(connection) => connections.push(Connection {
                id,
                name,
                _connection: connection,
            }),
            Err(error) => errors.push(format!("{name} could not be opened: {error}")),
        }
    }
    errors
}
