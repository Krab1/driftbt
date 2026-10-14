#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use drift_core::engine::fake::FakeEngine;
use drift_core::engine::rqbit::{RqbitConfig, RqbitEngine};
use drift_core::engine::TorrentEngine;
use drift_core::settings::{JsonFileStore, Settings, SettingsStore};
use drift_core::ticker;
use tauri::{Emitter, Manager, RunEvent};

pub struct AppState {
    pub engine: Arc<dyn TorrentEngine>,
    pub store: Arc<dyn SettingsStore>,
    pub settings: Mutex<Settings>,
    /// Kept to flush state on exit; `None` when running the fake demo engine.
    rqbit: Option<Arc<RqbitEngine>>,
}

// Composition root: engine, store and the ticker are wired here and nowhere else.
fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let store: Arc<dyn SettingsStore> = Arc::new(JsonFileStore::new(data_dir.join("settings.json")));
            let mut settings = store.load();
            if settings.download_dir.is_none() {
                settings.download_dir = app.path().download_dir().ok();
                let _ = store.save(&settings);
            }
            let download_dir = settings.download_dir.clone().unwrap_or_else(|| data_dir.join("downloads"));

            // DRIFT_FAKE=<n> runs the UI against n fake torrents, no network.
            let demo = std::env::var("DRIFT_FAKE").ok().and_then(|v| v.parse::<usize>().ok());
            let (engine, rqbit): (Arc<dyn TorrentEngine>, Option<Arc<RqbitEngine>>) = match demo {
                Some(n) => {
                    let fake = Arc::new(FakeEngine::demo(n));
                    let stepper = fake.clone();
                    tauri::async_runtime::spawn(async move {
                        let mut t = tokio::time::interval(Duration::from_secs(1));
                        loop {
                            t.tick().await;
                            stepper.step();
                        }
                    });
                    (fake, None)
                }
                None => {
                    let real = tauri::async_runtime::block_on(RqbitEngine::new(RqbitConfig {
                        output_dir: download_dir,
                        listen: None,
                        dht: true,
                        upnp: false,
                        persistence_dir: Some(data_dir.join("session")),
                    }))
                    .map_err(|e| format!("couldn't start the torrent engine: {e}"))?;
                    let real = Arc::new(real);
                    (real.clone(), Some(real))
                }
            };

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(ticker::run(engine.clone(), Duration::from_secs(1), move |list| {
                let _ = handle.emit("snapshot", &list);
            }));

            app.manage(AppState { engine, store, settings: Mutex::new(settings), rqbit });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_torrents,
            commands::add_torrent,
            commands::pause_torrent,
            commands::resume_torrent,
            commands::remove_torrent,
            commands::get_settings,
            commands::set_settings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Drift");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            if let Some(rq) = handle.state::<AppState>().rqbit.clone() {
                tauri::async_runtime::block_on(rq.shutdown());
            }
        }
    });
}
