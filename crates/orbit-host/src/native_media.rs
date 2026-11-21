use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{Cursor, Read, Write},
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime},
};

use image::ImageReader;
use serde_json::Value;
use ureq::unversioned::{
    resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver},
    transport::{DefaultConnector, NextTimeout},
};
use url::Url;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REDIRECTS: u32 = 5;
const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_AUDIO_BYTES: u64 = 256 * 1024 * 1024;
const MAX_VIDEO_BYTES: u64 = 256 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PREFETCH_URLS: usize = 32;
const MAX_ACTIVE_DOWNLOADS: usize = 4;
const MAX_QUEUED_DOWNLOADS: usize = 128;
const MAX_DOWNLOAD_ATTEMPTS: u32 = 6;
const ABANDONED_CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const MEDIA_CACHE_DIR: &str = "presenter-orbit-native-media-v1";
const MEDIA_KEYS: &[&str] = &[
    "backgroundMediaPath",
    "backgroundMediaUrl",
    "audioPath",
    "audioUrl",
    "mediaFillSourcePath",
    "mediaFillUrl",
    "sourcePath",
    "sourceUrl",
    "mediaPath",
    "mediaUrl",
    "mediaSourcePath",
];

#[derive(Clone)]
struct CacheEntry {
    path: PathBuf,
    bytes: u64,
    last_used: u64,
}

struct CacheState {
    entries: HashMap<String, CacheEntry>,
    in_flight: HashSet<String>,
    queued: VecDeque<String>,
    failures: HashMap<String, FailureState>,
    active_workers: usize,
    clock: u64,
    bytes: u64,
}

struct FailureState {
    retry_after: Instant,
    attempts: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoteAssetKind {
    Audio,
    Image,
    Video,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VideoContainer {
    IsoBaseMedia,
    Ebml,
    Avi,
}

#[derive(Clone)]
pub struct NativeMediaCache {
    root: Arc<PathBuf>,
    state: Arc<Mutex<CacheState>>,
    allow_private_networks: bool,
}

#[derive(Debug)]
struct MediaResolver {
    allow_private_networks: bool,
}

impl Resolver for MediaResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let resolved = DefaultResolver::default().resolve(uri, config, timeout)?;
        if self.allow_private_networks {
            return Ok(resolved);
        }
        let mut public = self.empty();
        for address in &resolved {
            if is_public_media_ip(address.ip()) {
                public.push(*address);
            }
        }
        if public.is_empty() {
            Err(ureq::Error::HostNotFound)
        } else {
            Ok(public)
        }
    }
}

impl NativeMediaCache {
    fn new() -> Self {
        let base = std::env::temp_dir().join(MEDIA_CACHE_DIR);
        cleanup_abandoned_cache_sessions(&base);
        enforce_global_cache_budget(&base, None, 0);
        let root = base.join(uuid::Uuid::new_v4().to_string());
        let _ = fs::create_dir_all(&root);
        Self {
            root: Arc::new(root),
            state: Arc::new(Mutex::new(CacheState {
                entries: HashMap::new(),
                in_flight: HashSet::new(),
                queued: VecDeque::new(),
                failures: HashMap::new(),
                active_workers: 0,
                clock: 0,
                bytes: 0,
            })),
            allow_private_networks: cfg!(test),
        }
    }

    fn resolve(&self, value: &Value) -> Option<PathBuf> {
        let raw = value.as_str()?.trim();
        if raw.is_empty() {
            return None;
        }
        if raw.len() >= 7 && raw[..7].eq_ignore_ascii_case("file://") {
            return Url::parse(raw).ok()?.to_file_path().ok();
        }
        let url = validate_remote_url(raw).ok()?;
        let mut state = self.state.lock().ok()?;
        let now = next_clock(&mut state);
        let Some(entry) = state.entries.get(url.as_str()).cloned() else {
            return None;
        };
        if !entry.path.is_file() {
            state.bytes = state.bytes.saturating_sub(entry.bytes);
            state.entries.remove(url.as_str());
            return None;
        }
        if let Some(entry) = state.entries.get_mut(url.as_str()) {
            entry.last_used = now;
        }
        Some(entry.path.clone())
    }

    fn prefetch_content(&self, content: &Value) -> bool {
        let mut urls = Vec::new();
        let mut seen = HashSet::new();
        collect_media_urls(content, &mut urls, &mut seen);
        urls.into_iter()
            .take(MAX_PREFETCH_URLS)
            .fold(false, |pending, url| self.prefetch(&url) || pending)
    }

    fn prefetch(&self, raw_url: &str) -> bool {
        let Ok(url) = validate_remote_url(raw_url) else {
            return false;
        };
        let key = url.as_str().to_string();
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state
            .entries
            .get(&key)
            .is_some_and(|entry| entry.path.is_file())
        {
            return false;
        }
        if state.in_flight.contains(&key) {
            return true;
        }
        if let Some(failure) = state.failures.get(&key) {
            if failure.attempts >= MAX_DOWNLOAD_ATTEMPTS {
                return false;
            }
            if failure.retry_after > Instant::now() {
                return true;
            }
        }
        if let Some(entry) = state.entries.remove(&key) {
            state.bytes = state.bytes.saturating_sub(entry.bytes);
        }
        if state.queued.len() >= MAX_QUEUED_DOWNLOADS {
            return true;
        }
        state.in_flight.insert(key.clone());
        state.queued.push_back(key);
        drop(state);
        self.pump_workers();
        true
    }

    fn pump_workers(&self) {
        loop {
            let Some(key) = (|| {
                let mut state = self.state.lock().ok()?;
                if state.active_workers >= MAX_ACTIVE_DOWNLOADS {
                    return None;
                }
                let key = state.queued.pop_front()?;
                state.active_workers += 1;
                Some(key)
            })() else {
                return;
            };
            let Ok(url) = Url::parse(&key) else {
                self.finish_download(&key, Err("Queued media URL became invalid.".to_string()));
                continue;
            };
            let cache = self.clone();
            thread::spawn(move || {
                let result = cache.download(&url);
                cache.finish_download(&key, result);
            });
        }
    }

    fn download(&self, url: &Url) -> Result<(PathBuf, u64), String> {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .user_agent("Presenter Orbit Native Media/1.0")
            .build();
        let agent = ureq::Agent::with_parts(
            config,
            DefaultConnector::default(),
            MediaResolver {
                allow_private_networks: self.allow_private_networks,
            },
        );
        let mut request_url = url.clone();
        let mut redirects = 0_u32;
        let (mut response, response_url) = loop {
            let response = agent
                .get(request_url.as_str())
                .call()
                .map_err(|error| format!("Native media request failed: {error}"))?;
            let status = response.status().as_u16();
            if !matches!(status, 301 | 302 | 303 | 307 | 308) {
                break (response, request_url);
            }
            if redirects >= MAX_REDIRECTS {
                return Err("Native media redirect limit was exceeded.".to_string());
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "Native media redirect did not include a location.".to_string())?;
            let redirected = request_url
                .join(location)
                .map_err(|error| format!("Native media redirect is invalid: {error}"))?;
            validate_remote_url(redirected.as_str())?;
            if request_url == redirected {
                return Err("Native media redirect looped to the same URL.".to_string());
            }
            request_url = redirected;
            redirects += 1;
        };
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(format!(
                "Native media request failed with HTTP status {status}."
            ));
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let kind = classify_remote_asset(&response_url, &content_type)?;
        let byte_limit = match kind {
            RemoteAssetKind::Audio => MAX_AUDIO_BYTES,
            RemoteAssetKind::Image => MAX_IMAGE_BYTES,
            RemoteAssetKind::Video => MAX_VIDEO_BYTES,
        };
        if response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|length| length == 0 || length > byte_limit)
        {
            return Err(format!(
                "Native media response exceeds the {} size limit.",
                asset_kind_name(kind)
            ));
        }
        let hash = cache_key(url.as_str());
        fs::create_dir_all(&*self.root).map_err(|error| {
            format!("Native media cache directory could not be created: {error}")
        })?;
        let temporary = self
            .root
            .join(format!(".{hash}.{}.tmp", uuid::Uuid::new_v4()));
        let byte_count = stream_response_to_file(&mut response, &temporary, byte_limit, kind)?;
        let extension = match kind {
            RemoteAssetKind::Audio => {
                if let Err(error) = validate_audio_file(&temporary) {
                    let _ = fs::remove_file(&temporary);
                    return Err(error);
                }
                remote_audio_extension(&response_url, &content_type)
            }
            RemoteAssetKind::Image => {
                let bytes = fs::read(&temporary).map_err(|error| {
                    let _ = fs::remove_file(&temporary);
                    format!("Native media cache validation read failed: {error}")
                })?;
                if let Err(error) = validate_image_bytes(&bytes) {
                    let _ = fs::remove_file(&temporary);
                    return Err(error);
                }
                if content_type == "image/svg+xml" || is_svg_bytes(&bytes) {
                    "svg"
                } else {
                    "asset"
                }
            }
            RemoteAssetKind::Video => {
                let container = match validate_video_file(&temporary) {
                    Ok(container) => container,
                    Err(error) => {
                        let _ = fs::remove_file(&temporary);
                        return Err(error);
                    }
                };
                if let Err(error) =
                    validate_video_container_hint(container, &response_url, &content_type)
                {
                    let _ = fs::remove_file(&temporary);
                    return Err(error);
                }
                remote_video_extension(container, &response_url, &content_type)
            }
        };
        let path = self
            .root
            .join(format!("{hash}-{}.{extension}", uuid::Uuid::new_v4()));
        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("Native media cache commit failed: {error}"));
        }
        Ok((path, byte_count))
    }

    fn finish_download(&self, key: &str, result: Result<(PathBuf, u64), String>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.in_flight.remove(key);
        state.active_workers = state.active_workers.saturating_sub(1);
        let Ok((path, bytes)) = result else {
            let attempts = state
                .failures
                .get(key)
                .map_or(0, |failure| failure.attempts);
            let backoff_seconds = 2_u64.pow(attempts.min(5)).min(60);
            state.failures.insert(
                key.to_string(),
                FailureState {
                    retry_after: Instant::now() + Duration::from_secs(backoff_seconds),
                    attempts: attempts.saturating_add(1),
                },
            );
            drop(state);
            self.pump_workers();
            return;
        };
        state.failures.remove(key);
        let preserve_path = path.clone();
        let external_bytes = self.root.parent().map_or(0, |base| {
            enforce_global_cache_budget(
                base,
                Some(self.root.as_path()),
                state.bytes.saturating_add(bytes),
            )
        });
        let cache_limit = MAX_CACHE_BYTES.saturating_sub(external_bytes);
        let mut failed_evictions = HashSet::new();
        while state.bytes.saturating_add(bytes) > cache_limit {
            let Some(oldest_key) = state
                .entries
                .iter()
                .filter(|(key, _)| !failed_evictions.contains(*key))
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            let Some(entry) = state.entries.get(&oldest_key).cloned() else {
                continue;
            };
            if fs::remove_file(&entry.path).is_ok() || !entry.path.exists() {
                state.entries.remove(&oldest_key);
                state.bytes = state.bytes.saturating_sub(entry.bytes);
            } else {
                failed_evictions.insert(oldest_key);
            }
        }
        if state.bytes.saturating_add(bytes) > cache_limit {
            drop(state);
            let _ = fs::remove_file(preserve_path);
            self.pump_workers();
            return;
        }
        let last_used = next_clock(&mut state);
        state.bytes = state.bytes.saturating_add(bytes);
        state.entries.insert(
            key.to_string(),
            CacheEntry {
                path,
                bytes,
                last_used,
            },
        );
        drop(state);
        self.pump_workers();
    }
}

static GLOBAL_CACHE: OnceLock<NativeMediaCache> = OnceLock::new();

pub fn prefetch_content(content: &Value) -> bool {
    global_cache().prefetch_content(content)
}

pub fn prefetch_media_url(raw_url: &str) -> bool {
    global_cache().prefetch(raw_url)
}

pub fn resolve_media_path(value: &Value) -> Option<PathBuf> {
    let raw = value.as_str()?.trim();
    if raw.is_empty() || raw.contains("://") {
        return global_cache().resolve(value);
    }
    Some(PathBuf::from(raw))
}

/// A long-lived process must keep its cache directory's modification time current, or another session's
/// startup cleanup removes it as abandoned.
pub(crate) fn keep_cache_session_alive() {
    if let Some(cache) = GLOBAL_CACHE.get() {
        let marker = cache.root.join(".keepalive");
        if fs::write(&marker, b"").is_ok() {
            let _ = fs::remove_file(marker);
        }
    }
}

fn global_cache() -> &'static NativeMediaCache {
    GLOBAL_CACHE.get_or_init(NativeMediaCache::new)
}

fn next_clock(state: &mut CacheState) -> u64 {
    state.clock = state.clock.wrapping_add(1);
    state.clock
}

fn cache_key(value: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn validate_image_bytes(bytes: &[u8]) -> Result<(), String> {
    let is_svg = is_svg_bytes(bytes);
    if !is_svg {
        let mut reader = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|error| format!("Native media image format could not be detected: {error}"))?;
        reader.no_limits();
        reader
            .decode()
            .map(|_| ())
            .map_err(|error| format!("Native media image could not be decoded: {error}"))
    } else {
        let options = resvg::usvg::Options::default();
        let tree = resvg::usvg::Tree::from_data(bytes, &options)
            .map_err(|error| format!("Native media SVG could not be decoded: {error}"))?;
        let size = tree.size().to_int_size();
        if size.width() == 0 || size.height() == 0 {
            return Err("Native media SVG dimensions are invalid.".to_string());
        }
        Ok(())
    }
}

fn classify_remote_asset(url: &Url, content_type: &str) -> Result<RemoteAssetKind, String> {
    if content_type.starts_with("audio/")
        || content_type == "application/ogg"
        || content_type == "application/octet-stream" && url_audio_extension(url).is_some()
    {
        return Ok(RemoteAssetKind::Audio);
    }
    if content_type.starts_with("image/") {
        return Ok(RemoteAssetKind::Image);
    }
    if content_type.starts_with("video/")
        || content_type == "application/mp4"
        || content_type == "application/octet-stream" && url_video_extension(url).is_some()
    {
        return Ok(RemoteAssetKind::Video);
    }
    Err("Native media response is not a supported audio, image, or video.".to_string())
}

fn asset_kind_name(kind: RemoteAssetKind) -> &'static str {
    match kind {
        RemoteAssetKind::Audio => "audio",
        RemoteAssetKind::Image => "image",
        RemoteAssetKind::Video => "video",
    }
}

fn validate_audio_file(path: &Path) -> Result<(), String> {
    let file = File::open(path)
        .map_err(|error| format!("Native media audio could not be inspected: {error}"))?;
    rodio::Decoder::try_from(file)
        .map(|_| ())
        .map_err(|error| format!("Native media audio could not be decoded: {error}"))
}

fn url_audio_extension(url: &Url) -> Option<&'static str> {
    let extension = Path::new(url.path())
        .extension()
        .and_then(|value| value.to_str())?;
    match extension.to_ascii_lowercase().as_str() {
        "aac" => Some("aac"),
        "aif" | "aiff" => Some("aiff"),
        "caf" => Some("caf"),
        "flac" => Some("flac"),
        "m4a" => Some("m4a"),
        "mp3" => Some("mp3"),
        "oga" | "ogg" | "opus" => Some("ogg"),
        "wav" | "wave" => Some("wav"),
        _ => None,
    }
}

fn remote_audio_extension(url: &Url, content_type: &str) -> &'static str {
    match content_type {
        "audio/aac" => "aac",
        "audio/aiff" | "audio/x-aiff" => "aiff",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/ogg" | "application/ogg" => "ogg",
        "audio/wav" | "audio/wave" | "audio/x-wav" => "wav",
        _ => url_audio_extension(url).unwrap_or("audio"),
    }
}

fn stream_response_to_file(
    response: &mut ureq::http::Response<ureq::Body>,
    temporary: &Path,
    byte_limit: u64,
    kind: RemoteAssetKind,
) -> Result<u64, String> {
    let result = (|| {
        let mut file = File::create(temporary)
            .map_err(|error| format!("Native media cache write failed: {error}"))?;
        let mut reader = response
            .body_mut()
            .as_reader()
            .take(byte_limit.saturating_add(1));
        let byte_count = std::io::copy(&mut reader, &mut file)
            .map_err(|error| format!("Native media response could not be read: {error}"))?;
        file.flush()
            .map_err(|error| format!("Native media cache write failed: {error}"))?;
        if byte_count == 0 || byte_count > byte_limit {
            return Err(format!(
                "Native media response exceeds the {} size limit.",
                asset_kind_name(kind)
            ));
        }
        Ok(byte_count)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn url_video_extension(url: &Url) -> Option<&'static str> {
    let extension = Path::new(url.path())
        .extension()
        .and_then(|value| value.to_str())?;
    match extension.to_ascii_lowercase().as_str() {
        "mp4" => Some("mp4"),
        "mov" => Some("mov"),
        "m4v" => Some("m4v"),
        "webm" => Some("webm"),
        "mkv" => Some("mkv"),
        "avi" => Some("avi"),
        _ => None,
    }
}

fn remote_video_extension(
    container: VideoContainer,
    url: &Url,
    content_type: &str,
) -> &'static str {
    match container {
        VideoContainer::IsoBaseMedia => match content_type {
            "video/quicktime" => "mov",
            "video/mp4" | "application/mp4" => "mp4",
            _ => url_video_extension(url)
                .filter(|extension| matches!(*extension, "mp4" | "mov" | "m4v"))
                .unwrap_or("mp4"),
        },
        VideoContainer::Ebml => match content_type {
            "video/webm" => "webm",
            "video/x-matroska" => "mkv",
            _ => url_video_extension(url)
                .filter(|extension| matches!(*extension, "webm" | "mkv"))
                .unwrap_or("mkv"),
        },
        VideoContainer::Avi => "avi",
    }
}

fn validate_video_file(path: &Path) -> Result<VideoContainer, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("Native media video could not be inspected: {error}"))?;
    let mut prefix = [0_u8; 16];
    let length = file
        .read(&mut prefix)
        .map_err(|error| format!("Native media video could not be inspected: {error}"))?;
    let prefix = &prefix[..length];
    let is_iso_media = prefix.len() >= 12 && &prefix[4..8] == b"ftyp";
    let is_ebml = prefix.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]);
    let is_avi = prefix.len() >= 12 && &prefix[..4] == b"RIFF" && &prefix[8..12] == b"AVI ";
    if is_iso_media {
        Ok(VideoContainer::IsoBaseMedia)
    } else if is_ebml {
        Ok(VideoContainer::Ebml)
    } else if is_avi {
        Ok(VideoContainer::Avi)
    } else {
        Err("Native media video container is not supported.".to_string())
    }
}

fn validate_video_container_hint(
    container: VideoContainer,
    url: &Url,
    content_type: &str,
) -> Result<(), String> {
    let mime_matches = match content_type {
        "video/mp4" | "application/mp4" | "video/quicktime" => {
            container == VideoContainer::IsoBaseMedia
        }
        "video/webm" | "video/x-matroska" => container == VideoContainer::Ebml,
        "video/x-msvideo" => container == VideoContainer::Avi,
        _ => true,
    };
    let extension_matches = match url_video_extension(url) {
        Some("mp4" | "mov" | "m4v") => container == VideoContainer::IsoBaseMedia,
        Some("webm" | "mkv") => container == VideoContainer::Ebml,
        Some("avi") => container == VideoContainer::Avi,
        _ => true,
    };
    if mime_matches && extension_matches {
        Ok(())
    } else {
        Err("Native media video type does not match its container.".to_string())
    }
}

fn is_svg_bytes(bytes: &[u8]) -> bool {
    let trimmed = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let trimmed = trimmed
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .collect::<Vec<_>>();
    trimmed.starts_with(b"<svg")
        || trimmed.starts_with(b"<?xml") && trimmed.windows(4).any(|part| part == b"<svg")
}

fn cleanup_abandoned_cache_sessions(base: &Path) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_dir() && !file_type.is_file() {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
            continue;
        };
        if SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age >= ABANDONED_CACHE_MAX_AGE)
        {
            if file_type.is_dir() {
                let _ = fs::remove_dir_all(path);
            } else {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn enforce_global_cache_budget(
    base: &Path,
    current_root: Option<&Path>,
    required_current_bytes: u64,
) -> u64 {
    let Ok(entries) = fs::read_dir(base) else {
        return 0;
    };
    let mut candidates = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if current_root.is_some_and(|current| path == current) {
                return None;
            }
            let file_type = entry.file_type().ok()?;
            if file_type.is_symlink() {
                return None;
            }
            let bytes = cache_path_bytes(&path);
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((path, file_type.is_dir(), bytes, modified))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.3.cmp(&right.3).then_with(|| left.0.cmp(&right.0)));
    let mut external_bytes = candidates.iter().fold(0_u64, |total, (_, _, bytes, _)| {
        total.saturating_add(*bytes)
    });
    for (path, is_directory, bytes, _) in candidates {
        if external_bytes.saturating_add(required_current_bytes) <= MAX_CACHE_BYTES {
            break;
        }
        let removed = if is_directory {
            fs::remove_dir_all(path).is_ok()
        } else {
            fs::remove_file(path).is_ok()
        };
        if removed {
            external_bytes = external_bytes.saturating_sub(bytes);
        }
    }
    external_bytes
}

fn cache_path_bytes(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.file_type().is_symlink() {
        return 0;
    }
    if metadata.is_file() {
        return metadata.len();
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries.flatten().fold(0_u64, |total, entry| {
        total.saturating_add(cache_path_bytes(&entry.path()))
    })
}

fn validate_remote_url(raw_url: &str) -> Result<Url, String> {
    let url = Url::parse(raw_url.trim()).map_err(|error| format!("Invalid media URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Native media URL must use HTTP or HTTPS.".to_string());
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("Native media URL must include a host.".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Native media URL credentials are not supported.".to_string());
    }
    Ok(url)
}

fn is_public_media_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [first, second, third, _] = ip.octets();
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_multicast()
                || first == 0
                || first == 100 && (64..=127).contains(&second)
                || first == 192 && second == 0 && third == 0
                || first == 198 && matches!(second, 18 | 19)
                || first >= 240)
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_media_ip(IpAddr::V4(mapped));
            }
            let first = ip.segments()[0];
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                || first & 0xfe00 == 0xfc00
                || first & 0xffc0 == 0xfe80
                || first & 0xffc0 == 0xfec0
                || ip.segments()[..2] == [0x2001, 0x0db8])
        }
    }
}

fn collect_media_urls(value: &Value, output: &mut Vec<String>, seen: &mut HashSet<String>) {
    match value {
        Value::Object(object) => {
            for key in MEDIA_KEYS {
                if let Some(url) = object
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|url| validate_remote_url(url).is_ok())
                {
                    if seen.insert(url.to_string()) {
                        output.push(url.to_string());
                    }
                }
            }
            for (key, value) in object {
                if !MEDIA_KEYS.iter().any(|candidate| candidate == &key) {
                    collect_media_urls(value, output, seen);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_media_urls(value, output, seen);
            }
        }
        _ => {}
    }
}
