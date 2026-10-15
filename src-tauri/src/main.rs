#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod caption;
mod commands;
mod updates;
mod platform;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use drift_core::engine::fake::FakeEngine;
use drift_core::engine::rqbit::{RqbitConfig, RqbitEngine};
use drift_core::engine::TorrentEngine;
use drift_core::events::completions;
use drift_core::library::JsonMetaStore;
use drift_core::manager::{GuardStatus, Manager as TorrentManager, SharedSettings};
use drift_core::netctl::{self, NetControl};
use drift_core::background::{self, WatchEvent};
use drift_core::model::TorrentId;
use drift_core::settings::{JsonFileStore, SettingsStore};
use drift_core::ticker;
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use platform::{autostart, OpenPayload};

pub struct AppState {
    /// Everything the UI does to torrents goes through the manager (queue, categories, goals).
    pub manager: Arc<TorrentManager>,
    pub net: Arc<NetControl>,
    /// Latest VPN guard result, for the UI.
    pub guard_status: Mutex<GuardStatus>,
    pub data_dir: std::path::PathBuf,
    /// Started the first time something is played.
    pub stream: tokio::sync::OnceCell<drift_core::stream::StreamServer>,
    /// Direct engine access, only for what the manager has no opinion on (details, inspect, file selection).
    pub engine: Arc<dyn TorrentEngine>,
    /// Operations shared with the headless server (settings, search, IP lists).
    pub svc: Arc<drift_core::service::Service>,
    /// Where the page's maximize button is, so Windows can show snap layouts over it.
    pub caption_rect: caption::ButtonRect,
    pub settings: SharedSettings,
    /// Reported by the UI (`document.hidden`); drives how often we poll and push.
    pub visible: Arc<AtomicBool>,
    /// Torrent whose details panel is open, if any.
    pub watched: Arc<Mutex<Option<TorrentId>>>,
    /// Magnets/files the app was launched with, until the UI takes them.
    pub pending_open: Mutex<OpenPayload>,
    /// Kept to flush state on exit; `None` when running the fake demo engine.
    rqbit: Option<Arc<RqbitEngine>>,
}

#[derive(Serialize, Clone)]
struct Completed {
    id: TorrentId,
    name: String,
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    // The loops slow down while hidden; tell the UI to catch up right away.
    let _ = app.emit("window-shown", ());
}

/// Whether anyone can see the window. The webview keeps reporting `document.hidden == false` after the
/// host window is hidden to the tray, so the backend decides from the native window state, and also
/// honours the UI's own report (e.g. occluded or backgrounded).
fn window_active(app: &AppHandle, ui_visible: &AtomicBool) -> bool {
    let native = app
        .get_webview_window("main")
        .map(|w| w.is_visible().unwrap_or(true) && !w.is_minimized().unwrap_or(false))
        .unwrap_or(true);
    native && ui_visible.load(Ordering::Relaxed)
}

/// The window is built here (config has `create: false`) so we can start hidden, and so debug builds
/// can open a DevTools-protocol port with `DRIFT_CDP_PORT=<port>` for driving the real UI in tests.
fn create_main_window(app: &tauri::App, start_hidden: bool) -> tauri::Result<()> {
    let cfg = app.config().app.windows[0].clone();
    #[allow(unused_mut)]
    let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?.visible(!start_hidden);
    #[cfg(debug_assertions)]
    {
        // Debug-only knobs. Setting browser arguments replaces wry's defaults, so repeat them.
        let mut args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
        let port = std::env::var("DRIFT_CDP_PORT").ok();
        let extra = std::env::var("DRIFT_WV_ARGS").ok();
        if let Some(p) = &port {
            args.push_str(&format!(" --remote-debugging-port={p}"));
        }
        if let Some(e) = &extra {
            args.push(' ');
            args.push_str(e);
        }
        if port.is_some() || extra.is_some() {
            builder = builder.additional_browser_args(&args);
        }
    }
    let window = builder.build()?;
    let rect = app.state::<AppState>().caption_rect.clone();
    caption::install(&window, rect, app.handle().clone());
    Ok(())
}

fn create_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Drift", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &settings, &quit])?;
    let mut tray = TrayIconBuilder::new()
        .tooltip("Drift")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "settings" => {
                show_main(app);
                let _ = app.emit("open-settings", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

// Composition root: engine, store, plugins and the background loops are wired here and nowhere else.
fn main() {
    let app = tauri::Builder::default()
        // Must be first: a second launch (e.g. clicking a magnet link) forwards its arguments here and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            show_main(app);
            let payload = platform::open_payload(&argv[1.min(argv.len())..]);
            if !payload.is_empty() {
                let _ = app.emit("open-sources", payload);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let close_to_tray = window.app_handle().state::<AppState>().settings.lock().unwrap().close_to_tray;
                if close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            // The headless server uses the same kind of folder; never both on one session.
            match drift_core::lock::DataDirLock::acquire(&data_dir)? {
                Some(lock) => {
                    app.manage(lock);
                }
                None => return Err("Another Drift (the app or the headless server) is already using this data folder.".into()),
            }
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
                    // The IP filter list from the last run is loaded straight away; refreshing happens later.
                    let net = netctl::network_config(&settings, drift_core::netctl::IpFilterService::new(data_dir.join("ipfilter")).active_file(&settings.ip_filter));
                    let real = tauri::async_runtime::block_on(RqbitEngine::new(RqbitConfig {
                        output_dir: download_dir,
                        listen: net.listen_port.map(|p| std::net::SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, p))),
                        dht: net.dht,
                        upnp: net.upnp,
                        persistence_dir: Some(data_dir.join("session")),
                        proxy: net.proxy,
                        blocklist_file: net.blocklist_file,
                    }))
                    .map_err(|e| format!("couldn't start the torrent engine: {e}"))?;
                    let real = Arc::new(real);
                    (real.clone(), Some(real))
                }
            };

            // Keep the OS autostart entry in step with the saved setting.
            if autostart::is_enabled() != settings.start_with_windows {
                let _ = autostart::set(settings.start_with_windows);
            }

            let args: Vec<String> = std::env::args().skip(1).collect();
            let start_hidden = args.iter().any(|a| a == autostart::FLAG) && settings.start_minimized;
            let visible = Arc::new(AtomicBool::new(!start_hidden));
            let watched: Arc<Mutex<Option<TorrentId>>> = Arc::new(Mutex::new(None));

            // The state must be registered before any loop starts: they look it up on their first tick.
            let pending = platform::open_payload(&args);
            let settings: SharedSettings = Arc::new(Mutex::new(settings));
            let manager = Arc::new(TorrentManager::new(
                engine.clone(),
                Arc::new(JsonMetaStore::new(data_dir.join("library.json"))),
                settings.clone(),
            ));
            tauri::async_runtime::block_on(manager.sync_limits())?;
            let net = Arc::new(NetControl::new(engine.clone(), data_dir.join("ipfilter")));
            let svc = Arc::new(drift_core::service::Service {
                manager: manager.clone(),
                engine: engine.clone(),
                net: net.clone(),
                store,
                settings: settings.clone(),
                secrets: drift_core::search::config::Secrets::open(data_dir.join("secrets.json")),
                plugins: drift_core::plugins::Host::at(&data_dir),
                data_dir: data_dir.clone(),
            });
            let svc_settings = svc.settings.clone();
            app.manage(AppState {
                manager: manager.clone(),
                net: net.clone(),
                guard_status: Mutex::new(GuardStatus { state: drift_core::guard::Verdict::Off, paused: 0 }),
                data_dir: data_dir.clone(),
                stream: tokio::sync::OnceCell::new(),
                engine: engine.clone(),
                svc,
                caption_rect: Default::default(),
                settings,
                visible: visible.clone(),
                watched: watched.clone(),
                pending_open: Mutex::new(pending),
                rqbit,
            });

            // Snapshot loop: 1 s while the window is visible, 5 s otherwise. Also detects completions.
            {
                let handle = app.handle().clone();
                let vis = visible.clone();
                let period_handle = app.handle().clone();
                let mut prev = Vec::new();
                tauri::async_runtime::spawn(ticker::run(
                    manager.clone(),
                    move || Duration::from_secs(if window_active(&period_handle, &vis) { 1 } else { 5 }),
                    move |list| {
                        let state = handle.state::<AppState>();
                        let (notify, playlists) = {
                            let s = state.settings.lock().unwrap();
                            (s.notify_on_complete, s.playlist_on_complete)
                        };
                        let stats: Vec<_> = list.iter().map(|v| v.stats.clone()).collect();
                        for t in completions(&prev, &stats) {
                            let _ = handle.emit("completed", Completed { id: t.id.clone(), name: t.name.clone() });
                            if playlists {
                                let (engine, id) = (state.engine.clone(), t.id.clone());
                                tauri::async_runtime::spawn(async move {
                                    // Quietly does nothing for torrents without several media files.
                                    let _ = commands::write_playlist(engine.as_ref(), &id).await;
                                });
                            }
                            let focused = handle.get_webview_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
                            if notify && !focused {
                                let _ = handle.notification().builder().title("Download complete").body(&t.name).show();
                            }
                        }
                        let _ = handle.emit("snapshot", &list);
                        prev = stats;
                    },
                ));
            }

            // VPN guard, watch folder and IP list refresh: the loops live in core, shared with the headless server.
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(background::vpn_guard(manager.clone(), svc_settings.clone(), move |status| {
                    *handle.state::<AppState>().guard_status.lock().unwrap() = status.clone();
                    let _ = handle.emit("guard", status);
                }));
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(background::watch_folder(manager.clone(), svc_settings.clone(), move |e| {
                    let _ = handle.emit(
                        "watch",
                        match e {
                            WatchEvent::Added(name) => serde_json::json!({ "added": name }),
                            WatchEvent::Invalid(name) => serde_json::json!({ "invalid": name }),
                        },
                    );
                }));
                tauri::async_runtime::spawn(background::ip_filter_refresh(net.clone(), svc_settings.clone()));
            }

            // Details loop: 2 Hz, only for the watched torrent and only while the window is visible.
            {
                let handle = app.handle().clone();
                let (vis, watch, eng) = (visible.clone(), watched.clone(), engine.clone());
                tauri::async_runtime::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        let id = watch.lock().unwrap().clone();
                        if let (true, Some(id)) = (window_active(&handle, &vis), id) {
                            if let Ok(d) = eng.details(&id).await {
                                let _ = handle.emit("details", d);
                            }
                        }
                    }
                });
            }

            app.manage(updates::Pending::default());
            tauri::async_runtime::spawn(updates::background(app.handle().clone()));
            create_main_window(app, start_hidden)?;
            create_tray(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_torrents,
            commands::inspect_torrent,
            commands::add_torrent,
            commands::pause_torrent,
            commands::resume_torrent,
            commands::recheck_torrent,
            commands::move_torrent,
            commands::remove_torrent,
            commands::set_files,
            commands::set_torrent_category,
            commands::set_torrent_tags,
            commands::set_torrent_goal,
            commands::move_in_queue,
            commands::watch_details,
            commands::set_visible,
            commands::take_pending_open,
            commands::reveal_torrent,
            commands::create_playlist,
            commands::play_torrent,
            commands::list_adapters,
            commands::network_status,
            commands::limits_status,
            commands::guard_status,
            commands::filter_status,
            commands::update_ip_lists,
            commands::search_torrents,
            commands::search_keys_set,
            commands::list_plugins,
            commands::set_search_key,
            commands::set_caption_rect,
            commands::check_update,
            commands::install_update,
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
