//! Self-update from GitHub Releases. Checks are quiet; installing always waits for the user's click.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::AppState;

#[derive(Serialize, Clone, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

/// The update found by the last check, kept until the user installs it.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

fn updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    #[allow(unused_mut)]
    let mut b = app.updater_builder();
    // Debug builds can point at a local test server (plain http is refused in release builds).
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("DRIFT_UPDATE_URL") {
        b = b.endpoints(vec![url.parse().map_err(|e| format!("{e}"))?]).map_err(|e| e.to_string())?;
    }
    b.build().map_err(|e| e.to_string())
}

/// `Ok(None)` means you're up to date.
pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let found = updater(app)?.check().await.map_err(|e| {
        eprintln!("update check failed: {e}");
        "Couldn't check for updates. Check your connection and try again.".to_string()
    })?;
    let info = found.as_ref().map(|u| UpdateInfo { version: u.version.clone(), notes: u.body.clone() });
    *app.state::<Pending>().0.lock().unwrap() = found;
    Ok(info)
}

/// Downloads the update, then lets the installer run only once this process is really gone. The stock plugin
/// starts the installer first and exits second; if the exit is slow the installer's own check finds Drift still
/// running and stops with "Failed to kill Drift". Here a detached helper waits, ends this process if it is still
/// there, and only then starts the installer, which restarts Drift when it finishes.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let update = app.state::<Pending>().0.lock().unwrap().take().ok_or("There's no update waiting. Check for updates first.")?;
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| {
        eprintln!("update download failed: {e}");
        "The update couldn't be downloaded. Try again later.".to_string()
    })?;
    #[cfg(windows)]
    {
        launch_after_exit(&update.version, &bytes).map_err(|e| {
            eprintln!("update launch failed: {e}");
            "The update couldn't be started. Try again later.".to_string()
        })?;
        app.exit(0);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        update.install(bytes).map_err(|e| {
            eprintln!("update install failed: {e}");
            "The update couldn't be installed. Try again later.".to_string()
        })
    }
}

#[cfg(windows)]
fn launch_after_exit(version: &str, installer: &[u8]) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    // The version comes from a signed manifest, but it still names a folder, so keep it to safe characters.
    let safe: String = version.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-').collect();
    let dir = std::env::temp_dir().join(format!("Drift-{safe}-update"));
    std::fs::create_dir_all(&dir)?;
    let exe = dir.join("Drift-setup.exe");
    std::fs::write(&exe, installer)?;
    let script = format!("ping -n 4 127.0.0.1 >nul & taskkill /F /PID {} >nul 2>&1 & \"{}\" /P /R /UPDATE", std::process::id(), exe.display());
    std::process::Command::new("cmd").raw_arg(format!("/C \"{script}\"")).creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS).spawn()?;
    Ok(())
}

/// A little after start and then daily, if the user hasn't turned checking off.
pub async fn background(app: AppHandle) {
    tokio::time::sleep(Duration::from_secs(45)).await;
    loop {
        if app.state::<AppState>().settings.lock().unwrap().check_updates {
            if let Ok(Some(info)) = check(&app).await {
                let _ = app.emit("update-available", info);
            }
        }
        tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
    }
}
