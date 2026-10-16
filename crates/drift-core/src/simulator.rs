//! Simulator plugins (kind `simulator`, see `docs/PLUGINS.md`): the app tells them which torrents exist and the user
//! switches simulation on or off per torrent. What a plugin does with that is its own business; nothing here knows.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use crate::manager::{Manager, SharedSettings};
use crate::model::{TorrentId, TorrentState, TorrentView};
use crate::plugins::Host;

/// One torrent as a simulator plugin sees it (`torrents_updated.torrents[]`, `selection_changed.torrent`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SimTorrent {
    pub info_hash: String,
    pub name: String,
    pub status: TorrentState,
    pub total_size: u64,
    pub have_bytes: u64,
    pub uploaded_bytes: u64,
    /// The engine doesn't count bytes received separately, so this equals `have_bytes`.
    pub downloaded_bytes: u64,
    pub trackers: Vec<String>,
    /// Not known to the engine today, so never sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub torrent_file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_dir: Option<String>,
}

/// What the engine says about a torrent beyond the list row: its trackers and where its content lives.
type Extra = (Vec<String>, Option<String>);

impl SimTorrent {
    fn new(v: &TorrentView, extra: Option<&Extra>) -> Self {
        let s = &v.stats;
        Self {
            info_hash: s.id.0.clone(),
            name: s.name.clone(),
            status: s.state,
            total_size: s.total_bytes,
            have_bytes: s.progress_bytes,
            uploaded_bytes: s.uploaded_bytes,
            downloaded_bytes: s.progress_bytes,
            trackers: extra.map(|e| e.0.clone()).unwrap_or_default(),
            torrent_file_path: None,
            content_dir: extra.and_then(|e| e.1.clone()),
        }
    }
}

async fn extra(manager: &Manager, id: &TorrentId) -> Option<Extra> {
    let d = manager.details(id).await.ok()?;
    let dir = std::path::Path::new(&d.save_path).join(&d.stats.name);
    Some((d.trackers, Some(dir.to_string_lossy().into_owned())))
}

/// One torrent by info hash, or `None` if Drift doesn't have it.
pub async fn torrent(manager: &Manager, info_hash: &str) -> Option<SimTorrent> {
    let id = TorrentId(info_hash.to_ascii_lowercase());
    let v = manager.snapshot().await.ok()?.into_iter().find(|v| v.stats.id == id)?;
    let e = extra(manager, &id).await;
    Some(SimTorrent::new(&v, e.as_ref()))
}

/// Every second: sends `torrents_updated` to each enabled simulator when the list differs from what it was last sent
/// (so a burst of changes is one message per second at most), and to a plugin whose process was just (re)started.
/// Also ends simulators that were switched off. A plugin that doesn't take the message is tried again next second.
pub async fn run(manager: Arc<Manager>, plugins: Arc<Host>, settings: SharedSettings) {
    let mut sent: HashMap<String, (u32, Vec<SimTorrent>)> = HashMap::new();
    let mut extras: HashMap<TorrentId, Extra> = HashMap::new();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let enabled = settings.lock().unwrap().plugins.clone();
        plugins.stop_unwanted_simulators(&enabled).await;
        let ids = plugins.simulator_ids(&enabled);
        sent.retain(|id, _| ids.contains(id));
        if ids.is_empty() {
            extras.clear();
            continue;
        }
        let Ok(views) = manager.snapshot().await else { continue };
        extras.retain(|id, _| views.iter().any(|v| &v.stats.id == id));
        for v in &views {
            // Magnets have no details until their metadata arrives; asked again next round.
            if !extras.contains_key(&v.stats.id) {
                if let Some(e) = extra(&manager, &v.stats.id).await {
                    extras.insert(v.stats.id.clone(), e);
                }
            }
        }
        let list: Vec<SimTorrent> = views.iter().map(|v| SimTorrent::new(v, extras.get(&v.stats.id))).collect();
        for id in ids {
            if sent.get(&id).is_some_and(|(g, l)| *g == plugins.generation(&id) && *l == list) {
                continue;
            }
            if plugins.torrents_updated(&id, &serde_json::json!({ "torrents": list })).await {
                sent.insert(id.clone(), (plugins.generation(&id), list.clone()));
            }
        }
    }
}
