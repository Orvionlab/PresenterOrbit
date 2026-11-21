use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const MAX_TEXT_BYTES: u64 = 1024 * 1024;

struct Watch {
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Value>>,
    worker: JoinHandle<()>,
}

#[derive(Default)]
pub struct LocalTextService {
    next_id: AtomicU64,
    watches: Mutex<HashMap<u64, Watch>>,
}

impl LocalTextService {
    pub fn start(&self, paths: Vec<String>) -> Result<u64, String> {
        if paths.is_empty()
            || paths.len() > 256
            || paths.iter().any(|path| !Path::new(path).is_absolute())
        {
            return Err("Local text watching requires 1 to 256 absolute file paths.".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let stop = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(json!([])));
        let worker_stop = stop.clone();
        let worker_snapshot = snapshot.clone();
        let worker = thread::Builder::new()
            .name("presenter-orbit-local-text".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    let entries = paths
                        .iter()
                        .map(|path| match read_text(Path::new(path)) {
                            Ok(text) => json!({"filePath": path, "text": text, "error": null}),
                            Err(error) => json!({"filePath": path, "text": "", "error": error}),
                        })
                        .collect::<Vec<_>>();
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    if let Ok(mut current) = worker_snapshot.lock() {
                        *current = json!(entries);
                    }
                    thread::park_timeout(Duration::from_millis(500));
                }
            })
            .map_err(|error| error.to_string())?;
        self.watches
            .lock()
            .map_err(|_| "Local text watch lock failed.")?
            .insert(
                id,
                Watch {
                    stop,
                    snapshot,
                    worker,
                },
            );
        Ok(id)
    }

    pub fn snapshot(&self, id: u64) -> Result<Value, String> {
        let watches = self
            .watches
            .lock()
            .map_err(|_| "Local text watch lock failed.")?;
        let watch = watches.get(&id).ok_or("Local text watch has stopped.")?;
        let snapshot = watch
            .snapshot
            .lock()
            .map_err(|_| "Local text snapshot lock failed.")?
            .clone();
        Ok(snapshot)
    }

    pub fn stop(&self, id: u64) -> Result<(), String> {
        let watch = self
            .watches
            .lock()
            .map_err(|_| "Local text watch lock failed.")?
            .remove(&id);
        if let Some(watch) = watch {
            watch.stop.store(true, Ordering::Release);
            watch.worker.thread().unpark();
            watch
                .worker
                .join()
                .map_err(|_| "Local text watch worker failed.")?;
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        let ids = self
            .watches
            .lock()
            .map(|watches| watches.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for id in ids {
            let _ = self.stop(id);
        }
    }
}

pub fn read_text(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|error| format!("Cannot open local text file: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Cannot inspect local text file: {error}"))?;
    if !metadata.is_file() {
        return Err("Local text source is not a regular file.".into());
    }
    if metadata.len() > MAX_TEXT_BYTES {
        return Err("Local text source exceeds 1 MiB.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_TEXT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read local text file: {error}"))?;
    if bytes.len() as u64 > MAX_TEXT_BYTES {
        return Err("Local text source exceeds 1 MiB.".into());
    }
    let contents = String::from_utf8(bytes).map_err(|_| "Local text source is not valid UTF-8.")?;
    if contents.contains('\0') {
        return Err("Local text source contains invalid null characters.".into());
    }
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("rtf"))
    {
        if !contents.trim_start().starts_with("{\\rtf") || !contents.trim_end().ends_with('}') {
            return Err("Local text source is not a valid RTF document.".into());
        }
        Ok(crate::strip_rtf(&contents))
    } else {
        Ok(contents)
    }
}
