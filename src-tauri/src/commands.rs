//! Thin IPC handlers: validate, call core, map errors to plain-language messages.

use std::path::Path;
use std::sync::atomic::Ordering;

use drift_core::errors::ApiError;
use drift_core::guard::Adapter;
use drift_core::manager::{GuardStatus, LimitsStatus};
use drift_core::netctl::FilterStatus;
use drift_core::playlist;
use drift_core::service::AddInput;
use drift_core::model::{TorrentDetails, TorrentId, TorrentPreview, TorrentView};
use drift_core::queue::Move;
use drift_core::rules::Goal;
use drift_core::search::SearchReport;
use drift_core::settings::Settings;
use tauri::State;

use crate::platform::{self, OpenPayload};
use crate::AppState;

type ApiResult<T> = Result<T, ApiError>;

#[tauri::command]
pub async fn list_torrents(state: State<'_, AppState>) -> ApiResult<Vec<TorrentView>> {
    Ok(state.manager.snapshot().await?)
}

#[tauri::command]
pub async fn inspect_torrent(state: State<'_, AppState>, input: AddInput) -> ApiResult<TorrentPreview> {
    state.svc.inspect(input).await
}

#[tauri::command]
pub async fn add_torrent(state: State<'_, AppState>, input: AddInput, category: Option<String>) -> ApiResult<TorrentId> {
    state.svc.add(input, category).await
}

#[tauri::command]
pub async fn pause_torrent(state: State<'_, AppState>, id: TorrentId) -> ApiResult<()> {
    Ok(state.manager.pause(&id).await?)
}

#[tauri::command]
pub async fn move_torrent(state: State<'_, AppState>, id: TorrentId, dest: String) -> ApiResult<()> {
    Ok(state.manager.relocate(&id, Path::new(dest.trim())).await?)
}

#[tauri::command]
pub async fn recheck_torrent(state: State<'_, AppState>, id: TorrentId) -> ApiResult<()> {
    Ok(state.manager.recheck(&id).await?)
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

pub use drift_core::service::write_playlist;

#[tauri::command]
pub async fn create_playlist(state: State<'_, AppState>, id: TorrentId) -> ApiResult<String> {
    match write_playlist(state.engine.as_ref(), &id).await? {
        Some(p) => Ok(p.display().to_string()),
        None => Err(ApiError { code: "no_playlist", message: "A playlist needs at least two video or audio files.".into() }),
    }
}

/// Plays a file in the default media player while it downloads: the torrent is resumed and moved to the
/// front of the queue, and the player is pointed at the loopback stream server through a one-line playlist.
#[tauri::command]
pub async fn play_torrent(state: State<'_, AppState>, id: TorrentId, file: Option<usize>) -> ApiResult<()> {
    let no_media = || ApiError { code: "no_media", message: "This torrent has no video or audio files to play.".into() };
    let d = state.engine.details(&id).await?;
    let chosen = match file {
        Some(i) => d.files.iter().find(|f| f.index == i),
        None => d.files.iter().filter(|f| playlist::is_media(&f.path)).max_by_key(|f| f.size),
    }
    .ok_or_else(no_media)?;
    if !playlist::is_media(&chosen.path) {
        return Err(no_media());
    }
    state.manager.prioritize(&id).await?;

    let engine = state.engine.clone();
    let server = state
        .stream
        .get_or_try_init(|| drift_core::stream::StreamServer::start(engine))
        .await
        .map_err(|e| {
            eprintln!("stream server failed to start: {e}");
            ApiError { code: "stream", message: "Couldn't start playback. Try again.".into() }
        })?;
    let name = chosen.path.rsplit(['/', '\\']).next().unwrap_or("video");
    let url = server.url(&id, chosen.index, name);
    let dir = state.data_dir.join("streams");
    let list = dir.join(format!("{}.m3u", &id.0[..id.0.len().min(16)]));
    std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&list, format!("#EXTM3U
{url}
")))
        .map_err(|e| {
            eprintln!("stream playlist write failed: {e}");
            ApiError { code: "stream", message: "Couldn't start playback. Try again.".into() }
        })?;
    platform::open_default(&list);
    Ok(())
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
pub fn list_adapters() -> Vec<Adapter> {
    drift_core::guard::adapters()
}

#[tauri::command]
pub async fn network_status(state: State<'_, AppState>) -> ApiResult<drift_core::model::NetworkStatus> {
    Ok(state.engine.network_status().await?)
}

#[tauri::command]
pub fn limits_status(state: State<'_, AppState>) -> LimitsStatus {
    state.manager.limits_status()
}

#[tauri::command]
pub fn guard_status(state: State<'_, AppState>) -> GuardStatus {
    state.guard_status.lock().unwrap().clone()
}

#[tauri::command]
pub fn filter_status(state: State<'_, AppState>) -> FilterStatus {
    let s = state.settings.lock().unwrap().clone();
    state.net.filter_status(&s)
}

/// Downloads every enabled IP list again now and restarts the engine's session if the merged list changed.
#[tauri::command]
pub async fn update_ip_lists(state: State<'_, AppState>) -> ApiResult<FilterStatus> {
    state.svc.update_ip_lists().await
}

#[tauri::command]
pub async fn search_torrents(state: State<'_, AppState>, text: String) -> ApiResult<SearchReport> {
    state.svc.search(&text).await
}

#[tauri::command]
pub fn list_plugins(state: State<'_, AppState>) -> Vec<drift_core::plugins::PluginInfo> {
    state.svc.list_plugins()
}

#[tauri::command]
pub fn search_keys_set(state: State<'_, AppState>) -> Vec<String> {
    state.svc.search_keys_set()
}

#[tauri::command]
pub fn set_search_key(state: State<'_, AppState>, id: String, key: String) -> ApiResult<()> {
    state.svc.set_search_key(&id, &key)
}

/// The maximize button's rectangle in physical window pixels (left, top, right, bottom); `None` hides it.
#[tauri::command]
pub fn set_caption_rect(state: State<'_, AppState>, rect: Option<[i32; 4]>) {
    *state.caption_rect.lock().unwrap() = rect;
}

#[tauri::command]
pub async fn check_update(app: tauri::AppHandle) -> ApiResult<Option<crate::updates::UpdateInfo>> {
    crate::updates::check(&app).await.map_err(|message| ApiError { code: "update", message })
}

#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> ApiResult<()> {
    crate::updates::install(&app).await.map_err(|message| ApiError { code: "update", message })
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub async fn set_settings(state: State<'_, AppState>, settings: Settings) -> ApiResult<Settings> {
    let before = state.svc.apply_settings(settings.clone()).await?;
    if before.start_with_windows != settings.start_with_windows {
        platform::autostart::set(settings.start_with_windows).map_err(|e| {
            eprintln!("autostart change failed: {e}");
            ApiError { code: "settings", message: "Couldn't save your settings. Check that the app data folder is writable.".into() }
        })?;
    }
    Ok(settings)
}
