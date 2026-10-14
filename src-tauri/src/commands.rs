//! Thin IPC handlers: validate, call core, map errors to plain-language messages.

use std::path::Path;
use std::sync::atomic::Ordering;

use drift_core::engine::EngineError;
use drift_core::model::{AddRequest, AddSource, TorrentDetails, TorrentId, TorrentPreview, TorrentView};
use drift_core::queue::Move;
use drift_core::rules::Goal;
use drift_core::settings::Settings;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::platform::{self, OpenPayload};
use crate::AppState;

#[derive(Serialize)]
pub struct ApiError {
    code: &'static str,
    message: String,
}

impl From<EngineError> for ApiError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::NotFound => ApiError { code: "not_found", message: "That torrent no longer exists.".into() },
            EngineError::InvalidInput(_) => ApiError {
                code: "invalid_input",
                message: "That isn't a valid magnet link or torrent file.".into(),
            },
            EngineError::Timeout => ApiError {
                code: "timeout",
                message: "Couldn't fetch the torrent info. Check the link, or try again in a moment.".into(),
            },
            EngineError::Backend(raw) => {
                eprintln!("engine error: {raw}");
                ApiError { code: "engine", message: "The torrent engine hit a problem. Try again.".into() }
            }
        }
    }
}

type ApiResult<T> = Result<T, ApiError>;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AddInput {
    Magnet {
        value: String,
        #[serde(default)]
        only_files: Option<Vec<usize>>,
    },
    File {
        bytes: Vec<u8>,
        #[serde(default)]
        only_files: Option<Vec<usize>>,
    },
}

impl AddInput {
    fn into_parts(self) -> (AddSource, Option<Vec<usize>>) {
        match self {
            AddInput::Magnet { value, only_files } => (AddSource::Magnet(value.trim().to_string()), only_files),
            AddInput::File { bytes, only_files } => (AddSource::FileBytes(bytes), only_files),
        }
    }
}

#[tauri::command]
pub async fn list_torrents(state: State<'_, AppState>) -> ApiResult<Vec<TorrentView>> {
    Ok(state.manager.snapshot().await?)
}

#[tauri::command]
pub async fn inspect_torrent(state: State<'_, AppState>, input: AddInput) -> ApiResult<TorrentPreview> {
    Ok(state.engine.inspect(input.into_parts().0).await?)
}

#[tauri::command]
pub async fn add_torrent(state: State<'_, AppState>, input: AddInput, category: Option<String>) -> ApiResult<TorrentId> {
    let (source, only_files) = input.into_parts();
    let mut req = AddRequest::new(source);
    req.only_files = only_files;
    // The manager picks the save folder (category, else the default) and the queue slot.
    Ok(state.manager.add(req, category, Vec::new()).await?)
}

#[tauri::command]
pub async fn pause_torrent(state: State<'_, AppState>, id: TorrentId) -> ApiResult<()> {
    Ok(state.manager.pause(&id).await?)
}

#[tauri::command]
pub async fn resume_torrent(state: State<'_, AppState>, id: TorrentId) -> ApiResult<()> {
    Ok(state.manager.resume(&id).await?)
}

#[tauri::command]
pub async fn remove_torrent(state: State<'_, AppState>, id: TorrentId, delete_files: bool) -> ApiResult<()> {
    Ok(state.manager.remove(&id, delete_files).await?)
}

#[tauri::command]
pub fn set_torrent_category(state: State<'_, AppState>, id: TorrentId, category: Option<String>) {
    state.manager.set_category(&id, category);
}

#[tauri::command]
pub fn set_torrent_tags(state: State<'_, AppState>, id: TorrentId, tags: Vec<String>) {
    state.manager.set_tags(&id, tags);
}

/// `None` goes back to the default seeding goal.
#[tauri::command]
pub fn set_torrent_goal(state: State<'_, AppState>, id: TorrentId, goal: Option<Goal>) {
    state.manager.set_goal(&id, goal);
}

#[tauri::command]
pub async fn move_in_queue(state: State<'_, AppState>, id: TorrentId, to: Move) -> ApiResult<()> {
    Ok(state.manager.move_in_queue(&id, to).await?)
}

#[tauri::command]
pub async fn set_files(state: State<'_, AppState>, id: TorrentId, selected: Vec<usize>) -> ApiResult<()> {
    Ok(state.engine.set_files(&id, selected).await?)
}

/// Tells the backend which torrent's details the UI shows (or `None`), and returns them right away;
/// after that they arrive as `details` events while the window is visible.
#[tauri::command]
pub async fn watch_details(state: State<'_, AppState>, id: Option<TorrentId>) -> ApiResult<Option<TorrentDetails>> {
    *state.watched.lock().unwrap() = id.clone();
    match id {
        Some(id) => Ok(Some(state.engine.details(&id).await?)),
        None => Ok(None),
    }
}

#[tauri::command]
pub fn set_visible(state: State<'_, AppState>, visible: bool) {
    state.visible.store(visible, Ordering::Relaxed);
}

/// Magnets or files the OS opened the app with before the UI was ready.
#[tauri::command]
pub fn take_pending_open(state: State<'_, AppState>) -> OpenPayload {
    std::mem::take(&mut *state.pending_open.lock().unwrap())
}

#[tauri::command]
pub async fn reveal_torrent(state: State<'_, AppState>, id: TorrentId) -> ApiResult<()> {
    let d = state.engine.details(&id).await?;
    let dir = Path::new(&d.save_path);
    let target = d.files.first().map(|f| dir.join(&f.path)).unwrap_or_else(|| dir.to_path_buf());
    platform::reveal(&target, dir);
    Ok(())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub async fn set_settings(state: State<'_, AppState>, settings: Settings) -> ApiResult<Settings> {
    let fail = |what: &str, e: &dyn std::fmt::Display| {
        eprintln!("{what}: {e}");
        ApiError { code: "settings", message: "Couldn't save your settings. Check that the app data folder is writable.".into() }
    };
    state.store.save(&settings).map_err(|e| fail("settings save failed", &e))?;
    let before = std::mem::replace(&mut *state.settings.lock().unwrap(), settings.clone());

    state.engine.set_limits(settings.effective_limits()).await?;
    if before.start_with_windows != settings.start_with_windows {
        platform::autostart::set(settings.start_with_windows).map_err(|e| fail("autostart change failed", &e))?;
    }
    Ok(settings)
}
