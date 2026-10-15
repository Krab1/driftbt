//! The long-running housekeeping loops shared by the desktop app and the headless server. Each runs forever:
//! spawn it on the host's runtime and let the host decide what to do with the events.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::guard::adapters;
use crate::manager::{GuardStatus, Manager, SharedSettings};
use crate::model::{AddRequest, AddSource, TorrentId};
use crate::plugins::{Action, Host};
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

/// What the post-processing plugins did, for the host to show: a plain sentence and whether it went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginNotice {
    pub text: String,
    pub error: bool,
}

/// Every 3 s: tells the enabled `postprocess` plugins about torrents that just finished and carries out what they
/// ask for. Seeding never waits for this and a failing plugin only produces a notice. A torrent still being moved by
/// the built-in "move when finished" waits until that is done, so the plugin sees its final folder.
pub async fn postprocess(manager: Arc<Manager>, plugins: Arc<Host>, settings: SharedSettings, on_notice: impl Fn(PluginNotice) + Send) {
    // Torrents to tell plugins about, with how many times looking them up has failed.
    let mut waiting: Vec<(TorrentId, u8)> = Vec::new();
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        waiting.extend(manager.take_finished().into_iter().map(|id| (id, 0)));
        let enabled = settings.lock().unwrap().plugins.clone();
        if !plugins.has_postprocessors(&enabled) {
            waiting.clear();
            continue;
        }
        for (id, tries) in std::mem::take(&mut waiting) {
            if manager.is_moving(&id) {
                waiting.push((id, tries));
                continue;
            }
            let Ok(info) = manager.finished_info(&id).await else {
                // The engine may be busy (for example restarting after a network change): try again a few times.
                if tries < 5 {
                    waiting.push((id, tries + 1));
                }
                continue;
            };
            for (plugin, result) in plugins.completed(&enabled, &info).await {
                match result {
                    Err(()) => on_notice(PluginNotice { text: format!("{plugin} couldn't process {}. The torrent keeps seeding.", info.name), error: true }),
                    Ok(actions) => {
                        for a in actions {
                            match a {
                                Action::Notify(m) => on_notice(PluginNotice { text: format!("{plugin}: {m}"), error: false }),
                                Action::Move(dest) => {
                                    if manager.relocate(&id, &dest).await.is_err() {
                                        on_notice(PluginNotice { text: format!("{plugin} asked to move {}, but it couldn't be moved.", info.name), error: true });
                                    }
                                }
                            }
                        }
                    }
                }
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
