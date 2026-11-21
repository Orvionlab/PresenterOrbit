//! Server-Sent Events push channel for `GET /api/v1/events`.
//!
//! Every stream runs on its own thread so the single accept loop keeps serving
//! other requests. Streams wait on one shared [`EventHub`]: status updates bump
//! its generation, and `RemoteServer::stop` bumps its run so every stream ends.

use std::{
    io::{self, Write},
    net::TcpStream,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread,
    time::{Duration, Instant},
};

use serde_json::{json, Value};

use super::{write_json_response, CORS_HEADERS};

pub(super) const MAX_EVENT_STREAMS: usize = 4;
const STREAM_WRITE_TIMEOUT: Duration = Duration::from_secs(3);
const STOP_WAIT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
pub(super) struct StreamTiming {
    /// Longest wait before the status is composed again without a wake-up, so
    /// catalog changes that bypass the status handle are still pushed.
    pub recheck: Duration,
    /// Longest silence before a comment line is written to detect dead clients.
    pub keepalive: Duration,
    /// Shortest gap between two compositions, so a burst of updates coalesces.
    pub min_gap: Duration,
}

pub(super) const STREAM_TIMING: StreamTiming = StreamTiming {
    recheck: Duration::from_secs(1),
    keepalive: Duration::from_secs(15),
    min_gap: Duration::from_millis(100),
};

struct HubState {
    generation: u64,
    run: u64,
    active: usize,
}

pub(super) struct EventHub {
    state: Mutex<HubState>,
    changed: Condvar,
}

#[derive(Debug, PartialEq, Eq)]
enum Wake {
    Closed,
    Changed,
    Timeout,
}

/// Holds one of the `MAX_EVENT_STREAMS` slots until dropped.
pub(super) struct StreamSlot {
    hub: Arc<EventHub>,
    run: u64,
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        self.hub.lock().active -= 1;
        self.hub.changed.notify_all();
    }
}

impl EventHub {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(HubState {
                generation: 0,
                run: 0,
                active: 0,
            }),
            changed: Condvar::new(),
        }
    }

    // The state is plain counters that are never left half-updated, so a
    // poisoned lock still holds consistent values.
    fn lock(&self) -> MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn notify_status_changed(&self) {
        self.lock().generation += 1;
        self.changed.notify_all();
    }

    pub(super) fn try_acquire(self: &Arc<Self>) -> Option<StreamSlot> {
        let mut state = self.lock();
        if state.active >= MAX_EVENT_STREAMS {
            return None;
        }
        state.active += 1;
        Some(StreamSlot {
            hub: Arc::clone(self),
            run: state.run,
        })
    }

    /// Ends every open stream and waits a bounded time for their threads to
    /// release their slots. Streams opened afterwards belong to a new run.
    pub(super) fn close_streams(&self, timeout: Duration) {
        let mut state = self.lock();
        state.run += 1;
        self.changed.notify_all();
        let _ = self
            .changed
            .wait_timeout_while(state, timeout, |state| state.active > 0)
            .unwrap_or_else(PoisonError::into_inner);
    }

    pub(super) fn stop_streams(&self) {
        self.close_streams(STOP_WAIT);
    }

    pub(super) fn generation(&self) -> u64 {
        self.lock().generation
    }

    fn wait(&self, run: u64, seen_generation: u64, timeout: Duration) -> Wake {
        let (state, _) = self
            .changed
            .wait_timeout_while(self.lock(), timeout, |state| {
                state.run == run && state.generation == seen_generation
            })
            .unwrap_or_else(PoisonError::into_inner);
        if state.run != run {
            Wake::Closed
        } else if state.generation != seen_generation {
            Wake::Changed
        } else {
            Wake::Timeout
        }
    }

    /// Sleeps for `duration` unless the run ends first; returns whether it ended.
    fn pause(&self, run: u64, duration: Duration) -> bool {
        let (state, _) = self
            .changed
            .wait_timeout_while(self.lock(), duration, |state| state.run == run)
            .unwrap_or_else(PoisonError::into_inner);
        state.run != run
    }

    fn is_closed(&self, run: u64) -> bool {
        self.lock().run != run
    }
}

fn comparable(status: &Value) -> Value {
    let mut status = status.clone();
    if let Some(object) = status.as_object_mut() {
        object.remove("serverTimeMs");
    }
    status
}

fn write_status_event<W: Write>(writer: &mut W, status: &Value) -> io::Result<()> {
    let data = serde_json::to_string(status)?;
    if data.contains(['\n', '\r']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Status JSON must stay on a single event data line.",
        ));
    }
    writer.write_all(format!("event: status\ndata: {data}\n\n").as_bytes())?;
    writer.flush()
}

fn write_keepalive<W: Write>(writer: &mut W) -> io::Result<()> {
    writer.write_all(b": keepalive\n\n")?;
    writer.flush()
}

fn write_stream_head<W: Write>(writer: &mut W) -> io::Result<()> {
    writer.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n{CORS_HEADERS}\r\n"
        )
        .as_bytes(),
    )?;
    writer.flush()
}

/// Writes the initial status event, then a new one whenever the composed status
/// changes (ignoring `serverTimeMs`). Returns `Ok` when the run ends and `Err`
/// when a write fails.
pub(super) fn run_event_stream<W: Write>(
    writer: &mut W,
    slot: &StreamSlot,
    timing: StreamTiming,
    mut compose: impl FnMut() -> Value,
) -> io::Result<()> {
    let hub = &slot.hub;
    let run = slot.run;
    if hub.is_closed(run) {
        return Ok(());
    }
    let mut seen_generation = hub.generation();
    let status = compose();
    let mut last = comparable(&status);
    write_status_event(writer, &status)?;
    let mut last_write = Instant::now();
    loop {
        let until_keepalive =
            (last_write + timing.keepalive).saturating_duration_since(Instant::now());
        if hub.wait(run, seen_generation, timing.recheck.min(until_keepalive)) == Wake::Closed {
            return Ok(());
        }
        seen_generation = hub.generation();
        let status = compose();
        let current = comparable(&status);
        if current != last {
            write_status_event(writer, &status)?;
            last = current;
            last_write = Instant::now();
        } else if last_write.elapsed() >= timing.keepalive {
            write_keepalive(writer)?;
            last_write = Instant::now();
        }
        if hub.pause(run, timing.min_gap) {
            return Ok(());
        }
    }
}

pub(super) fn serve_event_stream<F>(stream: TcpStream, hub: &Arc<EventHub>, compose: F)
where
    F: FnMut() -> Value + Send + 'static,
{
    serve_event_stream_with(stream, hub, STREAM_TIMING, compose);
}

pub(super) fn serve_event_stream_with<F>(
    mut stream: TcpStream,
    hub: &Arc<EventHub>,
    timing: StreamTiming,
    compose: F,
) where
    F: FnMut() -> Value + Send + 'static,
{
    let Some(slot) = hub.try_acquire() else {
        write_json_response(
            &mut stream,
            503,
            &json!({ "error": "Too many event streams are open." }),
        );
        return;
    };
    if let Err(error) = stream.set_write_timeout(Some(STREAM_WRITE_TIMEOUT)) {
        write_json_response(
            &mut stream,
            500,
            &json!({ "error": format!("Event stream could not be configured: {error}") }),
        );
        return;
    }
    let _ = stream.set_nodelay(true);
    let mut error_stream = match stream.try_clone() {
        Ok(error_stream) => error_stream,
        Err(error) => {
            write_json_response(
                &mut stream,
                500,
                &json!({ "error": format!("Event stream could not be configured: {error}") }),
            );
            return;
        }
    };
    let spawned = thread::Builder::new()
        .name("presenter-orbit-remote-events".to_string())
        .spawn(move || {
            if write_stream_head(&mut stream).is_ok() {
                let _ = run_event_stream(&mut stream, &slot, timing, compose);
            }
        });
    if let Err(error) = spawned {
        write_json_response(
            &mut error_stream,
            503,
            &json!({ "error": format!("Event stream thread could not start: {error}") }),
        );
    }
}
