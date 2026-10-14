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

/// Downloads and runs the installer found by the last check. The installer closes this app and restarts it.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let update = app.state::<Pending>().0.lock().unwrap().take().ok_or("There's no update waiting. Check for updates first.")?;
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| {
        eprintln!("update install failed: {e}");
        "The update couldn't be installed. Try again later.".to_string()
    })
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
