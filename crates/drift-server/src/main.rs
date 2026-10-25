//! drift-server: Drift without a window. Same engine, queue and rules; controlled over HTTP (see docs/HEADLESS.md).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use drift_core::background::{self, WatchEvent};
use drift_core::engine::rqbit::{RqbitConfig, RqbitEngine};
use drift_core::engine::TorrentEngine;
use drift_core::events::completions;
use drift_core::library::JsonMetaStore;
use drift_core::manager::{GuardStatus, Manager, SharedSettings};
use drift_core::netctl::{self, IpFilterService, NetControl};
use drift_core::search::config::Secrets;
use drift_core::service::{write_playlist, Service};
use drift_core::settings::{JsonFileStore, SettingsStore};
use drift_server::{router, Event, Shared};
use tokio::sync::broadcast;

const HELP: &str = "drift-server: Drift without a window

  --data-dir <folder>   where settings, session and token live (default: per-user data folder)
  --bind <addr:port>    where to listen (default 127.0.0.1:8793)
  --ui-dir <folder>     the built web UI to serve at / (default: a 'ui' folder next to this program)
  --show-token          print the access token and exit

The access token is created on first start and kept in <data-dir>/server.json.";

struct Args {
    data_dir: PathBuf,
    bind: SocketAddr,
    ui_dir: Option<PathBuf>,
    show_token: bool,
}

fn default_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("drift-server")
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { data_dir: default_data_dir(), bind: SocketAddr::from(([127, 0, 0, 1], 8793)), ui_dir: None, show_token: false };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--data-dir" => a.data_dir = value("--data-dir")?.into(),
            "--bind" => a.bind = value("--bind")?.parse().map_err(|_| "--bind must look like 127.0.0.1:8793".to_string())?,
            "--ui-dir" => a.ui_dir = Some(value("--ui-dir")?.into()),
            "--show-token" => a.show_token = true,
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other}\n\n{HELP}")),
        }
    }
    if a.ui_dir.is_none() {
        a.ui_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("ui"))).filter(|p| p.join("index.html").exists());
    }
    Ok(a)
}

/// The token lives next to the settings; created once, 256 random bits.
fn load_or_create_token(dir: &std::path::Path) -> Result<(String, bool), String> {
    let path = dir.join("server.json");
    if let Ok(bytes) = std::fs::read(&path) {
        if let Some(t) = serde_json::from_slice::<serde_json::Value>(drift_core::jsonfile::strip_bom(&bytes)).ok().and_then(|v| v["token"].as_str().map(String::from)).filter(|t| t.len() >= 32) {
            return Ok((t, false));
        }
    }
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| format!("couldn't get random bytes for the token: {e}"))?;
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    drift_core::jsonfile::write_atomic(&path, serde_json::json!({ "token": token }).to_string().as_bytes()).map_err(|e| format!("couldn't save the token: {e}"))?;
    Ok((token, true))
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("drift-server: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args = parse_args()?;
    let (token, fresh) = load_or_create_token(&args.data_dir)?;
    if args.show_token {
        println!("{token}");
        return Ok(());
    }
    let _lock = drift_core::lock::DataDirLock::acquire(&args.data_dir)
        .map_err(|e| format!("can't use the data folder {}: {e}", args.data_dir.display()))?
        .ok_or("another Drift (the app or another server) is already using this data folder")?;

    let data_dir = args.data_dir.clone();
    let store: Arc<dyn SettingsStore> = Arc::new(JsonFileStore::new(data_dir.join("settings.json")));
    let mut settings = store.load();
    if settings.download_dir.is_none() {
        settings.download_dir = Some(data_dir.join("downloads"));
        // Only a first run writes the file; an unreadable existing one must not be replaced by stand-in values.
        if !data_dir.join("settings.json").exists() {
            let _ = store.save(&settings);
        }
    }
    let download_dir = settings.download_dir.clone().unwrap_or_else(|| data_dir.join("downloads"));
    let net_cfg = netctl::network_config(&settings, IpFilterService::new(data_dir.join("ipfilter")).active_file(&settings.ip_filter));
    let rqbit = Arc::new(
        RqbitEngine::new(RqbitConfig {
            output_dir: download_dir,
            listen: net_cfg.listen_port.map(|p| SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, p))),
            dht: net_cfg.dht,
            upnp: net_cfg.upnp,
            persistence_dir: Some(data_dir.join("session")),
            proxy: net_cfg.proxy,
            blocklist_file: net_cfg.blocklist_file,
        })
        .await
        .map_err(|e| format!("couldn't start the torrent engine: {e}"))?,
    );
    let engine: Arc<dyn TorrentEngine> = rqbit.clone();

    let settings: SharedSettings = Arc::new(Mutex::new(settings));
    let manager = Arc::new(Manager::new(engine.clone(), Arc::new(JsonMetaStore::new(data_dir.join("library.json"))), settings.clone()));
    manager.sync_limits().await.map_err(|e| e.to_string())?;
    let net = Arc::new(NetControl::new(engine.clone(), data_dir.join("ipfilter")));
    let svc = Arc::new(Service {
        manager: manager.clone(),
        engine: engine.clone(),
        net: net.clone(),
        store,
        settings: settings.clone(),
        secrets: Secrets::open(data_dir.join("secrets.json")),
        // Next to the program: the first-party search sources, installed into the data folder at start.
        plugins: drift_core::plugins::Host::at_with_bundled(&data_dir, std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("bundled-plugins"))).as_deref()),
        data_dir,
    });
    svc.switch_off_unapproved_plugins();
    svc.migrate_search_to_plugins().await;
    let (tx, _) = broadcast::channel::<Event>(16);
    let shared = Arc::new(Shared { svc, token: token.clone(), events: tx.clone(), guard: Mutex::new(GuardStatus { state: drift_core::guard::Verdict::Off, paused: 0 }), ui_dir: args.ui_dir.clone() });

    // Adds that were still waiting for their details when the server last stopped.
    {
        let manager = manager.clone();
        tokio::spawn(async move { manager.resume_pending().await });
    }

    // Snapshots every 2 s (1 s while somebody is listening), completion detection and playlists.
    {
        let (tx, settings, engine) = (tx.clone(), settings.clone(), engine.clone());
        let mut prev = Vec::new();
        tokio::spawn(drift_core::ticker::run(
            manager.clone(),
            {
                let tx = tx.clone();
                move || Duration::from_secs(if tx.receiver_count() > 0 { 1 } else { 5 })
            },
            move |list| {
                let stats: Vec<_> = list.iter().map(|v| v.stats.clone()).collect();
                for t in completions(&prev, &stats) {
                    let _ = tx.send(Event::new("completed", &serde_json::json!({ "id": t.id, "name": t.name })));
                    let playlists = {
                        let s = settings.lock().unwrap();
                        s.media_player && s.playlist_on_complete
                    };
                    if playlists {
                        let (engine, id) = (engine.clone(), t.id.clone());
                        tokio::spawn(async move {
                            let _ = write_playlist(engine.as_ref(), &id).await;
                        });
                    }
                }
                if tx.receiver_count() > 0 {
                    let _ = tx.send(Event::new("snapshot", &list));
                }
                prev = stats;
            },
        ));
    }
    {
        let (tx, shared) = (tx.clone(), shared.clone());
        tokio::spawn(background::vpn_guard(manager.clone(), settings.clone(), move |status| {
            *shared.guard.lock().unwrap() = status.clone();
            let _ = tx.send(Event::new("guard", status));
        }));
    }
    {
        let tx = tx.clone();
        tokio::spawn(background::watch_folder(manager.clone(), settings.clone(), move |e| {
            let payload = match e {
                WatchEvent::Added(name) => serde_json::json!({ "added": name }),
                WatchEvent::Invalid(name) => serde_json::json!({ "invalid": name }),
            };
            let _ = tx.send(Event::new("watch", &payload));
        }));
    }
    {
        let tx = tx.clone();
        let plugins = shared.svc.plugins.clone();
        tokio::spawn(background::postprocess(manager.clone(), plugins, settings.clone(), move |n| {
            let _ = tx.send(Event::new("plugin_notice", &serde_json::json!({ "text": n.text, "error": n.error })));
        }));
    }
    tokio::spawn(drift_core::simulator::run(manager.clone(), shared.svc.plugins.clone(), settings.clone()));
    tokio::spawn(background::ip_filter_refresh(net, settings));

    let listener = tokio::net::TcpListener::bind(args.bind).await.map_err(|e| format!("can't listen on {}: {e}", args.bind))?;
    println!("drift-server {} listening on http://{}", drift_core::VERSION, args.bind);
    if fresh {
        println!("Access token (shown once here; `--show-token` prints it again): {token}");
    }
    match &args.ui_dir {
        Some(d) => println!("Web UI: {}", d.display()),
        None => println!("No web UI folder found; only the API is served (use --ui-dir)."),
    }
    if !args.bind.ip().is_loopback() {
        eprintln!("WARNING: listening beyond this computer over plain http. The token and your torrent list can be read on the network; put it behind a VPN or an HTTPS reverse proxy.");
    }
    axum::serve(listener, router(shared))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| e.to_string())?;
    rqbit.shutdown().await;
    Ok(())
}
