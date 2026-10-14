//! The long-running housekeeping loops shared by the desktop app and the headless server. Each runs forever:
//! spawn it on the host's runtime and let the host decide what to do with the events.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::guard::adapters;
use crate::manager::{GuardStatus, Manager, SharedSettings};
use crate::model::{AddRequest, AddSource};
use crate::netctl::{NetControl, Refresh};
use crate::watch;

/// What the watch folder did with a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    Added(String),
    Invalid(String),
}

/// Every 5 s: `.torrent` files dropped in the watch folder are added once, then renamed.
pub async fn watch_folder(manager: Arc<Manager>, settings: SharedSettings, on_event: impl Fn(WatchEvent) + Send) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let dir = settings.lock().unwrap().watch_dir.clone();
        let Some(dir) = dir else { continue };
        for f in watch::scan(&dir, SystemTime::now()) {
            let name = f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match manager.add(AddRequest::new(AddSource::FileBytes(f.bytes)), None, Vec::new()).await {
                Ok(_) => {
                    let _ = watch::mark(&f.path, watch::Outcome::Added);
                    on_event(WatchEvent::Added(name));
                }
                // Not a torrent: wait until the file has stopped changing, then set it aside.
                Err(crate::engine::EngineError::InvalidInput(_)) if f.settled => {
                    let _ = watch::mark(&f.path, watch::Outcome::Invalid);
                    on_event(WatchEvent::Invalid(name));
                }
                Err(e) => eprintln!("watch folder: {name}: {e}"),
            }
        }
    }
}

/// Every 2 s while enabled, so a dropped VPN adapter is noticed quickly. `on_change` fires when the status changes.
pub async fn vpn_guard(manager: Arc<Manager>, settings: SharedSettings, on_change: impl Fn(&GuardStatus) + Send) {
    let mut last: Option<GuardStatus> = None;
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let enabled = settings.lock().unwrap().vpn_guard.enabled;
        // Adapter enumeration blocks briefly, so keep it off the async threads.
        let list = if enabled { tokio::task::spawn_blocking(adapters).await.unwrap_or_default() } else { Vec::new() };
        let Ok(status) = manager.guard_check(&list).await else { continue };
        if last.as_ref() != Some(&status) {
            on_change(&status);
            last = Some(status);
        }
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A little after start and then every 6 hours: refresh IP lists older than a week.
pub async fn ip_filter_refresh(net: Arc<NetControl>, settings: SharedSettings) {
    tokio::time::sleep(Duration::from_secs(20)).await;
    loop {
        let s = settings.lock().unwrap().clone();
        if s.ip_filter.enabled {
            if let Err(e) = net.apply(&s, Refresh::IfDue, unix_now()).await {
                eprintln!("ip list refresh failed: {e}");
            }
        }
        tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
    }
}
