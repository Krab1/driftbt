//! Thin IPC handlers: validate, call core, map errors to plain-language messages.

use drift_core::engine::EngineError;
use drift_core::model::{AddRequest, AddSource, TorrentId, TorrentStats};
use drift_core::settings::Settings;
use serde::{Deserialize, Serialize};
use tauri::State;

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
    Magnet { value: String },
    File { bytes: Vec<u8> },
}

#[tauri::command]
pub async fn list_torrents(state: State<'_, AppState>) -> ApiResult<Vec<TorrentStats>> {
    Ok(state.engine.list().await?)
}

#[tauri::command]
pub async fn add_torrent(state: State<'_, AppState>, input: AddInput) -> ApiResult<TorrentId> {
    let source = match input {
        AddInput::Magnet { value } => AddSource::Magnet(value.trim().to_string()),
        AddInput::File { bytes } => AddSource::FileBytes(bytes),
    };
    let mut req = AddRequest::new(source);
    req.output_dir = state.settings.lock().unwrap().download_dir.clone();
    Ok(state.engine.add(req).await?)
}

#[tauri::command]
pub async fn pause_torrent(state: State<'_, AppState>, id: TorrentId) -> ApiResult<()> {
    Ok(state.engine.pause(&id).await?)
}

#[tauri::command]
pub async fn resume_torrent(state: State<'_, AppState>, id: TorrentId) -> ApiResult<()> {
    Ok(state.engine.resume(&id).await?)
}

#[tauri::command]
pub async fn remove_torrent(state: State<'_, AppState>, id: TorrentId, delete_files: bool) -> ApiResult<()> {
    Ok(state.engine.remove(&id, delete_files).await?)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_settings(state: State<'_, AppState>, settings: Settings) -> ApiResult<Settings> {
    state.store.save(&settings).map_err(|e| {
        eprintln!("settings save failed: {e}");
        ApiError { code: "settings", message: "Couldn't save your settings. Check that the app data folder is writable.".into() }
    })?;
    *state.settings.lock().unwrap() = settings.clone();
    Ok(settings)
}
