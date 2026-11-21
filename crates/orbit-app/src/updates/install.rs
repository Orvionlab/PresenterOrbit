//! Starting a verified update artifact. Windows runs the NSIS installer the way the Tauri updater does
//! (`/P /UPDATE /R`: passive progress window, update mode, relaunch). The macOS and Linux paths are written but
//! have not been run on those systems.

#[cfg(windows)]
pub fn launch(bytes: Vec<u8>, version: &str) -> Result<(), String> {
    use windows::core::{w, HSTRING};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;

    if !bytes.starts_with(b"MZ") {
        return Err("The update is not a Windows installer.".into());
    }
    let directory = std::env::temp_dir().join(format!(
        "presenter-orbit-update-{version}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let installer = directory.join(format!("Presenter Orbit_{version}_setup.exe"));
    std::fs::write(&installer, bytes).map_err(|error| error.to_string())?;
    // The installer asks for elevation (per-machine install), which only ShellExecute can do.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(installer.as_os_str()),
            w!("/P /UPDATE /R"),
            None,
            SW_SHOW,
        )
    };
    if result.0 as isize <= 32 {
        return Err(format!(
            "The installer could not be started (code {}).",
            result.0 as isize
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn launch(bytes: Vec<u8>, version: &str) -> Result<(), String> {
    use std::process::Command;

    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let app = executable
        .ancestors()
        .nth(3)
        .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
        .ok_or("The application is not running from an .app bundle.")?
        .to_path_buf();
    // Next to the bundle, so the swap is a rename on one volume.
    let work = app
        .parent()
        .ok_or("The application bundle has no folder.")?
        .join(format!(".presenter-orbit-update-{version}"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|error| error.to_string())?;
    let archive = work.join("update.app.tar.gz");
    std::fs::write(&archive, bytes).map_err(|error| error.to_string())?;
    let extracted = work.join("new");
    std::fs::create_dir_all(&extracted).map_err(|error| error.to_string())?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&extracted)
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("The update archive could not be extracted.".into());
    }
    let new_app = std::fs::read_dir(&extracted)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .ok_or("The update archive holds no application.")?;
    let previous = work.join("previous.app");
    std::fs::rename(&app, &previous).map_err(|error| error.to_string())?;
    if let Err(error) = std::fs::rename(&new_app, &app) {
        let _ = std::fs::rename(&previous, &app);
        return Err(error.to_string());
    }
    let _ = std::fs::remove_dir_all(&work);
    Command::new("open")
        .arg("-n")
        .arg(&app)
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn launch(bytes: Vec<u8>, version: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let appimage = std::env::var_os("APPIMAGE")
        .map(std::path::PathBuf::from)
        .ok_or(
        "Updates install into the AppImage; other installations update with their package manager.",
    )?;
    let staged = appimage.with_extension(format!("update-{version}"));
    std::fs::write(&staged, bytes).map_err(|error| error.to_string())?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
        .map_err(|error| error.to_string())?;
    std::fs::rename(&staged, &appimage).map_err(|error| error.to_string())?;
    std::process::Command::new(&appimage)
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}
