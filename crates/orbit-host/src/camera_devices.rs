use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    process::{Command, Stdio},
};

const MAX_DEVICES: usize = 64;
const MAX_NAME_CHARS: usize = 256;
const MAX_LISTING_BYTES: u64 = 256 * 1024;

/// A capture device (as the OS lists it) plus the one format FFmpeg is asked to deliver.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraSource {
    pub device_name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

pub fn host() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOs
    } else {
        Platform::Linux
    }
}

pub fn validate_source(source: &CameraSource) -> Result<(), String> {
    if source.device_name.trim().is_empty()
        || source.device_name.chars().count() > MAX_NAME_CHARS
        || source.device_name.chars().any(char::is_control)
    {
        return Err("Camera device name is required and cannot contain control characters.".into());
    }
    if !(16..=4096).contains(&source.width)
        || !(16..=4096).contains(&source.height)
        || u64::from(source.width) * u64::from(source.height) > 8_294_400
        || !(1..=120).contains(&source.fps)
    {
        return Err("Camera width, height and frame rate are outside the supported range.".into());
    }
    Ok(())
}

/// FFmpeg input options that open the device; the caller appends the common output options.
pub fn input_args(platform: Platform, source: &CameraSource) -> Vec<String> {
    let size = format!("{}x{}", source.width, source.height);
    let fps = source.fps.to_string();
    let (format, device) = match platform {
        Platform::Windows => ("dshow", format!("video={}", source.device_name)),
        Platform::MacOs => ("avfoundation", source.device_name.clone()),
        Platform::Linux => ("v4l2", source.device_name.clone()),
    };
    let mut args: Vec<String> = [
        "-f",
        format,
        "-video_size",
        size.as_str(),
        "-framerate",
        fps.as_str(),
        "-i",
    ]
    .map(String::from)
    .into();
    args.push(device);
    args
}

fn strip_log_prefix(line: &str) -> &str {
    if line.starts_with('[') {
        if let Some(end) = line.find("] ") {
            if line[..end].contains(" @ ") {
                return &line[end + 2..];
            }
        }
    }
    line
}

fn finish(mut names: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names.retain(|name| {
        !name.is_empty() && name.chars().count() <= MAX_NAME_CHARS && seen.insert(name.clone())
    });
    names.truncate(MAX_DEVICES);
    names
}

/// Handles FFmpeg's tagged `"name" (video)` lines and the older untagged section layout.
pub fn parse_dshow(output: &str) -> Vec<String> {
    let mut in_video_section = false;
    let mut names = Vec::new();
    for line in output.lines().map(strip_log_prefix) {
        if line.contains("DirectShow video devices") {
            in_video_section = true;
        } else if line.contains("DirectShow audio devices") {
            in_video_section = false;
        }
        let Some(rest) = line.trim().strip_prefix('"') else {
            continue;
        };
        let Some(end) = rest.rfind('"') else { continue };
        let is_video = match rest[end + 1..].trim() {
            "" => in_video_section,
            tag => tag.contains("video"),
        };
        if is_video {
            names.push(rest[..end].to_string());
        }
    }
    finish(names)
}

pub fn parse_avfoundation(output: &str) -> Vec<String> {
    let mut in_video_section = false;
    let mut names = Vec::new();
    for line in output.lines().map(strip_log_prefix) {
        if line.contains("AVFoundation video devices") {
            in_video_section = true;
        } else if line.contains("AVFoundation audio devices") {
            in_video_section = false;
        }
        let Some(entry) = line.trim().strip_prefix('[') else {
            continue;
        };
        let Some((index, name)) = entry.split_once("] ") else {
            continue;
        };
        if in_video_section
            && !index.is_empty()
            && index.bytes().all(|byte| byte.is_ascii_digit())
            && !name.starts_with("Capture screen")
        {
            names.push(name.trim().to_string());
        }
    }
    finish(names)
}

/// Accepts directory entry names such as `video0`; metadata-only nodes are not distinguished.
pub fn parse_v4l2(entries: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut indexes: Vec<u32> = entries
        .into_iter()
        .filter_map(|entry| entry.strip_prefix("video")?.parse().ok())
        .collect();
    indexes.sort_unstable();
    finish(
        indexes
            .into_iter()
            .map(|index| format!("/dev/video{index}"))
            .collect(),
    )
}

pub fn list_devices() -> Result<Vec<String>, String> {
    let platform = host();
    if platform == Platform::Linux {
        let entries = std::fs::read_dir("/dev")
            .map_err(|error| format!("Camera devices could not be listed: {error}"))?
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok());
        return Ok(parse_v4l2(entries));
    }
    if !ffmpeg_sidecar::command::ffmpeg_is_installed() {
        return Err(
            "FFmpeg is unavailable; camera discovery requires the installed FFmpeg.".into(),
        );
    }
    let list_args: &[&str] = match platform {
        Platform::Windows => &["-f", "dshow", "-list_devices", "true", "-i", "dummy"],
        _ => &["-f", "avfoundation", "-list_devices", "true", "-i", ""],
    };
    let mut command = Command::new(ffmpeg_sidecar::paths::ffmpeg_path());
    command
        .args(["-hide_banner", "-nostdin"])
        .args(list_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("FFmpeg could not list camera devices: {error}"))?;
    let mut listing = Vec::new();
    child
        .stderr
        .take()
        .ok_or("FFmpeg device listing is unavailable.")?
        .take(MAX_LISTING_BYTES)
        .read_to_end(&mut listing)
        .map_err(|error| format!("FFmpeg device listing failed: {error}"))?;
    // The listing always ends with a non-zero exit (the dummy input cannot open), so only the text matters.
    let _ = child.kill();
    let _ = child.wait();
    let listing = String::from_utf8_lossy(&listing);
    Ok(if platform == Platform::Windows {
        parse_dshow(&listing)
    } else {
        parse_avfoundation(&listing)
    })
}
