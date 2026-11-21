//! Revision bookkeeping of `src/renderer/tauri-bridge-queues.ts`: the last revision the host accepted per
//! workspace path and for the global settings.

use crate::locked::Locked;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

#[derive(Default)]
struct Inner {
    workspaces: HashMap<String, u64>,
    settings: u64,
}

#[derive(Clone, Default)]
pub struct Revisions(Arc<Mutex<Inner>>);

fn revision_of(value: &Value, field: &str) -> Option<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .filter(|revision| *revision <= MAX_SAFE_INTEGER)
}

impl Revisions {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.locked()
    }

    /// An unknown path has revision 0, as in TypeScript.
    pub fn workspace(&self, path: &str) -> u64 {
        self.inner().workspaces.get(path).copied().unwrap_or(0)
    }

    /// Remembers `result.revision` under `path` unless it is older than what is known.
    pub fn remember_workspace(&self, path: &str, result: &Value) {
        let Some(revision) = revision_of(result, "revision") else {
            return;
        };
        let mut inner = self.inner();
        let current = inner.workspaces.get(path).copied().unwrap_or(0);
        if revision >= current {
            inner.workspaces.insert(path.to_string(), revision);
        }
    }

    /// The response can name the canonical path (on Windows it starts with `\\?\`) that the application switches
    /// to, so both spellings must know the revision.
    pub fn remember_selected(&self, result: &Value) {
        if result.get("canceled") == Some(&Value::Bool(true)) {
            return;
        }
        if let Some(path) = result["workspace"]["workspacePath"].as_str() {
            self.remember_workspace(path, result);
        }
    }

    /// Both of the above, as `invokeWorkspaceMutation`, `loadWorkspace`, and `readWorkspaceSnapshot` do.
    pub fn remember(&self, path: &str, result: &Value) {
        self.remember_workspace(path, result);
        self.remember_selected(result);
    }

    pub fn settings(&self) -> u64 {
        self.inner().settings
    }

    pub fn read_settings(result: &Value) -> Option<u64> {
        revision_of(result, "settingsRevision")
    }

    pub fn remember_settings(&self, result: &Value) {
        if let Some(revision) = Self::read_settings(result) {
            let mut inner = self.inner();
            if revision >= inner.settings {
                inner.settings = revision;
            }
        }
    }

    /// A settings event older than the known revision is dropped (`onSettingsChanged`).
    pub fn accept_settings_event(&self, payload: &Value) -> bool {
        match Self::read_settings(payload) {
            Some(revision) if revision < self.settings() => false,
            _ => {
                self.remember_settings(payload);
                true
            }
        }
    }

    /// `audio-inputs-changed` is dropped when older than the known workspace revision; `trigger-bindings-changed`
    /// and the like only record it.
    pub fn accept_workspace_event(&self, payload: &Value, drop_older: bool) -> bool {
        let (Some(path), Some(revision)) = (
            payload["workspacePath"].as_str(),
            revision_of(payload, "revision"),
        ) else {
            return true;
        };
        if drop_older && revision < self.workspace(path) {
            return false;
        }
        self.remember_workspace(path, payload);
        true
    }
}
