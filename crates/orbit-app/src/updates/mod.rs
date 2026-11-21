//! Application updates, on the update feed and signature format of the Tauri app.
//!
//! The feed is a static `latest.json` per platform and architecture (`<base>/windows-x64/latest.json`) with the
//! Tauri updater fields: `version`, `notes`, and `platforms.<os>-<arch>` with `url` and `signature`. The download
//! is verified against the minisign public key built into the executable (`PRESENTER_ORBIT_UPDATE_PUBLIC_KEY`,
//! the Tauri `.pub` file contents) before anything runs, so a Tauri build can update to this one and this one to
//! later builds. Without an endpoint and key at build time the updater reports `unconfigured`.

mod dialog;
mod install;

pub use dialog::StartupUpdate;

use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use orbit_host::{Host, Updater};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_MANIFEST_BYTES: u64 = 1 << 20;
const MAX_DOWNLOAD_BYTES: u64 = 2 << 30;

#[derive(Clone, Debug)]
pub struct Config {
    pub endpoint: String,
    pub public_key: String,
    /// The manifest platform keys to try, most specific first.
    pub targets: Vec<String>,
}

impl Config {
    /// The endpoint and key this build was made with, if it was made with them.
    pub fn from_build() -> Option<Self> {
        let base = option_env!("PRESENTER_ORBIT_UPDATE_BASE_URL")?
            .trim()
            .trim_end_matches('/');
        let public_key = option_env!("PRESENTER_ORBIT_UPDATE_PUBLIC_KEY")?.trim();
        if base.is_empty() || public_key.is_empty() {
            return None;
        }
        let (folder, os, arch, installer) = platform();
        Some(Self {
            endpoint: format!("{base}/{folder}/latest.json"),
            public_key: public_key.to_string(),
            targets: vec![format!("{os}-{arch}-{installer}"), format!("{os}-{arch}")],
        })
    }
}

/// The feed folder, the manifest OS and architecture keys, and the installer kind of this build.
fn platform() -> (String, &'static str, &'static str, &'static str) {
    let (folder_os, os, installer) = if cfg!(windows) {
        ("windows", "windows", "nsis")
    } else if cfg!(target_os = "macos") {
        ("macos", "darwin", "app")
    } else {
        ("linux", "linux", "appimage")
    };
    let (folder_arch, arch) = if cfg!(target_arch = "aarch64") {
        ("arm64", "aarch64")
    } else {
        ("x64", "x86_64")
    };
    (format!("{folder_os}-{folder_arch}"), os, arch, installer)
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    platforms: HashMap<String, Artifact>,
}

#[derive(Deserialize)]
struct Artifact {
    url: String,
    signature: String,
}

#[derive(Clone, Debug)]
pub struct Pending {
    pub version: String,
    notes: Option<String>,
    url: String,
    signature: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    status: &'static str,
    startup_check: bool,
    current_version: String,
    version: Option<String>,
    notes: Option<String>,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
}

pub struct AppUpdater {
    config: Option<Config>,
    current: Version,
    busy: AtomicBool,
    pending: Mutex<Option<Pending>>,
    status: Mutex<Status>,
}

struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(concat!("Presenter Orbit/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

fn fetch_manifest(endpoint: &str) -> Result<Manifest, String> {
    let mut response = agent(REQUEST_TIMEOUT)
        .get(endpoint)
        .call()
        .map_err(|error| format!("The update feed could not be read: {error}"))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_MANIFEST_BYTES)
        .read_to_vec()
        .map_err(|error| format!("The update feed could not be read: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("The update feed is invalid: {error}"))
}

/// The update in `manifest` when it is newer than `current` and has an artifact for one of `targets`.
fn select(
    manifest: Manifest,
    current: &Version,
    targets: &[String],
) -> Result<Option<Pending>, String> {
    let version = Version::parse(manifest.version.trim_start_matches('v'))
        .map_err(|error| format!("The update feed has an invalid version: {error}"))?;
    if &version <= current {
        return Ok(None);
    }
    let artifact = targets
        .iter()
        .find_map(|target| manifest.platforms.get(target))
        .ok_or_else(|| {
            format!(
                "The update feed has no build for this system ({}).",
                targets.join(", ")
            )
        })?;
    Ok(Some(Pending {
        version: version.to_string(),
        notes: manifest.notes.filter(|notes| !notes.trim().is_empty()),
        url: artifact.url.clone(),
        signature: artifact.signature.clone(),
    }))
}

fn base64_text(value: &str) -> Result<String, String> {
    let bytes = STANDARD
        .decode(value.trim())
        .map_err(|error| format!("The update key or signature is not valid base64: {error}"))?;
    String::from_utf8(bytes).map_err(|_| "The update key or signature is not text.".to_string())
}

/// Checks `data` against a minisign signature, both in the Tauri encoding (base64 of the minisign file text).
fn verify(data: &[u8], signature: &str, public_key: &str) -> Result<(), String> {
    let key = PublicKey::decode(&base64_text(public_key)?)
        .map_err(|error| format!("The update public key is invalid: {error}"))?;
    let signature = Signature::decode(&base64_text(signature)?)
        .map_err(|error| format!("The update signature is invalid: {error}"))?;
    key.verify(data, &signature, true)
        .map_err(|error| format!("The update signature does not match the download: {error}"))
}

impl AppUpdater {
    pub fn new(config: Option<Config>) -> Self {
        let current =
            Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is semver");
        Self::with_version(config, current)
    }

    fn with_version(config: Option<Config>, current: Version) -> Self {
        let status = Status {
            status: if config.is_some() {
                "idle"
            } else {
                "unconfigured"
            },
            startup_check: false,
            current_version: current.to_string(),
            version: None,
            notes: None,
            downloaded: 0,
            total: None,
            error: None,
        };
        Self {
            config,
            current,
            busy: AtomicBool::new(false),
            pending: Mutex::new(None),
            status: Mutex::new(status),
        }
    }

    fn snapshot(&self) -> Value {
        json!(*self.status.lock().expect("update status lock"))
    }

    fn change(&self, change: impl FnOnce(&mut Status)) {
        change(&mut self.status.lock().expect("update status lock"));
    }

    fn begin(&self) -> Result<BusyGuard<'_>, String> {
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err("An update operation is already running.".into());
        }
        Ok(BusyGuard(&self.busy))
    }

    fn check_now(&self, startup_check: bool) -> Result<Value, String> {
        let Some(config) = &self.config else {
            return Ok(self.snapshot());
        };
        let _busy = self.begin()?;
        self.pending.lock().expect("pending update lock").take();
        self.change(|status| {
            *status = Status {
                status: "checking",
                startup_check,
                error: None,
                version: None,
                notes: None,
                downloaded: 0,
                total: None,
                ..status.clone()
            }
        });
        match fetch_manifest(&config.endpoint)
            .and_then(|manifest| select(manifest, &self.current, &config.targets))
        {
            Ok(update) => {
                self.change(|status| {
                    status.status = if update.is_some() {
                        "available"
                    } else {
                        "current"
                    };
                    status.version = update.as_ref().map(|update| update.version.clone());
                    status.notes = update.as_ref().and_then(|update| update.notes.clone());
                });
                *self.pending.lock().expect("pending update lock") = update;
            }
            Err(error) => self.change(|status| {
                status.status = "error";
                status.error = Some(error);
            }),
        }
        Ok(self.snapshot())
    }

    /// Downloads the pending update, verifies it, and hands the bytes to `launch`. A failure leaves the update
    /// pending so the operator can try again; nothing runs unless the signature matched.
    fn install_now(
        &self,
        launch: impl FnOnce(Vec<u8>, &str) -> Result<(), String>,
    ) -> Result<Value, String> {
        let Some(config) = &self.config else {
            return Err("Updates are not configured for this build.".into());
        };
        let _busy = self.begin()?;
        let pending = self
            .pending
            .lock()
            .expect("pending update lock")
            .take()
            .ok_or("Check for an available update before installing.")?;
        self.change(|status| {
            status.status = "downloading";
            status.downloaded = 0;
            status.error = None;
        });
        let result = self.download(&pending).and_then(|bytes| {
            verify(&bytes, &pending.signature, &config.public_key)?;
            self.change(|status| status.status = "installing");
            launch(bytes, &pending.version)
        });
        if let Err(error) = result {
            self.change(|status| {
                status.status = "error";
                status.error = Some(error);
            });
            *self.pending.lock().expect("pending update lock") = Some(pending);
        }
        Ok(self.snapshot())
    }

    fn download(&self, pending: &Pending) -> Result<Vec<u8>, String> {
        let mut response = agent(DOWNLOAD_TIMEOUT)
            .get(&pending.url)
            .call()
            .map_err(|error| format!("The update could not be downloaded: {error}"))?;
        let total = response.body().content_length();
        self.change(|status| status.total = total);
        let mut reader = response
            .body_mut()
            .with_config()
            .limit(MAX_DOWNLOAD_BYTES)
            .reader();
        let mut bytes = Vec::with_capacity(total.unwrap_or(0).min(MAX_DOWNLOAD_BYTES) as usize);
        let mut chunk = vec![0_u8; 64 * 1024];
        loop {
            let read = reader
                .read(&mut chunk)
                .map_err(|error| format!("The update download was interrupted: {error}"))?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
            self.change(|status| status.downloaded += read as u64);
        }
        Ok(bytes)
    }
}

/// The updater of the application: the shared state plus the closing of the application after an install.
pub struct GpuiUpdater {
    pub(crate) core: AppUpdater,
    quit: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
}

impl GpuiUpdater {
    pub fn new(quit: impl Fn() -> Result<(), String> + Send + Sync + 'static) -> Self {
        Self {
            core: AppUpdater::new(Config::from_build()),
            quit: Box::new(quit),
        }
    }
}

impl Updater for GpuiUpdater {
    fn status(&self) -> Value {
        self.core.snapshot()
    }

    fn check(&self, _host: &Host, startup_check: bool) -> Result<Value, String> {
        self.core.check_now(startup_check)
    }

    fn install(&self, host: &Host) -> Result<Value, String> {
        self.core.install_now(|bytes, version| {
            // The signature matched; from here the application gives way to the installer.
            install::launch(bytes, version)?;
            orbit_host::shutdown_outputs(host);
            (self.quit)()
        })
    }
}

/// Whether the startup check runs: release builds only, and only when the operator has it on.
pub fn startup_check_enabled(general_settings: &Value) -> bool {
    !cfg!(debug_assertions) && general_settings["checkForUpdatesOnStartup"] == true
}
