//! Moves files and folders to the operating-system trash so deletions stay recoverable.

use std::path::Path;

#[cfg(windows)]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{BOOL, PCWSTR};
    use windows::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
        SHFILEOPSTRUCTW,
    };

    // pFrom is a list of paths terminated by an extra NUL. The shell rejects the verbatim prefix that
    // `canonicalize` gives the registered workspace paths (error 124, invalid name).
    let shell_path = shell_path(&path.to_string_lossy());
    let from: Vec<u16> = std::ffi::OsStr::new(&shell_path)
        .encode_wide()
        .chain([0, 0])
        .collect();
    let mut operation = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_NOERRORUI.0 | FOF_SILENT.0) as u16,
        ..Default::default()
    };
    let result = unsafe { SHFileOperationW(&mut operation) };
    if result != 0 {
        return Err(format!(
            "Could not move {} to the Recycle Bin (error {result}).",
            path.display()
        ));
    }
    if operation.fAnyOperationsAborted != BOOL(0) {
        return Err(format!(
            "Moving {} to the Recycle Bin was cancelled.",
            path.display()
        ));
    }
    Ok(())
}

/// The spelling of a path the Windows shell accepts: no verbatim (`\\?\`, `\\?\UNC\`) prefix.
#[cfg(windows)]
fn shell_path(path: &str) -> String {
    if let Some(share) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{share}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    }
}

#[cfg(target_os = "macos")]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};

    let path = path
        .to_str()
        .ok_or_else(|| "The path is not valid UTF-8.".to_string())?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|error| error.localizedDescription().to_string())
}

// shortcut: delegates to GIO's freedesktop.org trash; implement the spec directly if desktops without gio must be supported.
#[cfg(target_os = "linux")]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    let output = std::process::Command::new("gio")
        .arg("trash")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| {
            format!(
                "Could not run gio to move {} to the trash: {error}",
                path.display()
            )
        })?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}
