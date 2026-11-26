//! The serial mutation queues of `tauri-bridge-queues.ts`. One worker thread per workspace path (and one for the
//! settings) runs its jobs in submission order, so a host call never overlaps the previous call of the same lane
//! and reads its base revision only when it starts. Nothing here runs on the UI thread.

use crate::locked::Locked;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use futures::channel::oneshot;
use serde_json::Value;

use super::revisions::Revisions;

type Job = Box<dyn FnOnce() + Send + 'static>;

const SETTINGS_LANE: &str = "\0settings";
const OPERATOR_LANE: &str = "\0operator-cue";

/// The result of a lane job, awaited on the foreground.
pub type Answer = oneshot::Receiver<Result<Value, String>>;

#[derive(Clone, Default)]
pub struct Lanes {
    revisions: Revisions,
    workers: Arc<Mutex<HashMap<String, Sender<Job>>>>,
    latest_cue: Arc<AtomicU64>,
}

impl Lanes {
    /// A newer manual take replaces queued takes; workspace edits keep their submission order.
    pub fn latest_cue(
        &self,
        path: &str,
        before_save: bool,
        operation: impl FnOnce() -> Result<Value, String> + Send + 'static,
    ) -> oneshot::Receiver<Result<Option<Value>, String>> {
        let latest = self.latest_cue.clone();
        let request = latest.fetch_add(1, Ordering::AcqRel) + 1;
        // A dependent take still waits for any clear already submitted to the operator lane.
        let operator_ready = (!before_save).then(|| self.operator(|| Ok(())));
        self.task(if before_save { OPERATOR_LANE } else { path }, move || {
            if let Some(ready) = operator_ready {
                futures::executor::block_on(ready)
                    .map_err(|_| "The operator command worker stopped.")??;
            }
            if latest.load(Ordering::Acquire) != request {
                return Ok(None);
            }
            operation().map(Some)
        })
    }

    pub fn cancel_pending_cues(&self) {
        self.latest_cue.fetch_add(1, Ordering::AcqRel);
    }

    pub fn operator<T: Send + 'static>(
        &self,
        operation: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> oneshot::Receiver<Result<T, String>> {
        self.task(OPERATOR_LANE, operation)
    }

    /// Serial work that must follow workspace edits but does not return a workspace revision.
    pub fn command(
        &self,
        path: &str,
        operation: impl FnOnce() -> Result<Value, String> + Send + 'static,
    ) -> Answer {
        self.task(path, operation)
    }

    pub fn task<T: Send + 'static>(
        &self,
        path: &str,
        operation: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> oneshot::Receiver<Result<T, String>> {
        let (reply, answer) = oneshot::channel();
        self.submit(
            path,
            Box::new(move || {
                let _ = reply.send(operation());
            }),
        );
        answer
    }
    pub fn revisions(&self) -> &Revisions {
        &self.revisions
    }

    fn submit(&self, key: &str, job: Job) {
        let mut workers = self.workers.locked();
        if !workers.contains_key(key) {
            let (sender, receiver) = channel::<Job>();
            let spawned = thread::Builder::new()
                .name("workspace-lane".into())
                .spawn(move || {
                    while let Ok(job) = receiver.recv() {
                        // A panicking job must not end the lane: its reply is dropped, so the caller sees a
                        // stopped worker, and later jobs still run.
                        let _ = panic::catch_unwind(AssertUnwindSafe(job));
                    }
                });
            // Without a worker the job is dropped, which drops its reply; callers report a stopped worker.
            if spawned.is_err() {
                return;
            }
            workers.insert(key.to_string(), sender);
        }
        // The worker never ends while the map holds its sender, so a send only fails if it was never started.
        let _ = workers[key].send(job);
    }

    /// `invokeWorkspaceMutation`: runs `operation` after every earlier job of this workspace, with the revision
    /// accepted so far, and remembers the revision of its response under both spellings of the path.
    pub fn workspace(
        &self,
        path: &str,
        operation: impl FnOnce(u64) -> Result<Value, String> + Send + 'static,
    ) -> Answer {
        let (reply, answer) = oneshot::channel();
        let (revisions, path) = (self.revisions.clone(), path.to_string());
        self.submit(
            &path.clone(),
            Box::new(move || {
                let result = operation(revisions.workspace(&path));
                if let Ok(value) = &result {
                    revisions.remember(&path, value);
                }
                // The caller may have gone away, for example when the window closed.
                let _ = reply.send(result);
            }),
        );
        answer
    }

    /// `invokeSettingsMutation`. The settings features of Phase 14 use it.
    #[allow(dead_code)]
    pub fn settings(
        &self,
        operation: impl FnOnce(u64) -> Result<Value, String> + Send + 'static,
    ) -> Answer {
        let (reply, answer) = oneshot::channel();
        let revisions = self.revisions.clone();
        self.submit(
            SETTINGS_LANE,
            Box::new(move || {
                let result = operation(revisions.settings());
                if let Ok(value) = &result {
                    revisions.remember_settings(value);
                }
                let _ = reply.send(result);
            }),
        );
        answer
    }
}
