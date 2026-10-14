//! Adapter over `librqbit` 9.0.1. This is the ONLY file allowed to use librqbit (CLAUDE.md rule 2).

use std::borrow::Cow;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use librqbit::api::TorrentIdOrHash;
use librqbit::http_api_types::PeerStatsFilter;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerOptions, ManagedTorrent, Session,
    SessionOptions, SessionPersistenceConfig, TorrentStatsState,
};

use super::{EngineError, Result, TorrentEngine};
use crate::errors::plain_error;
use crate::model::*;

pub struct RqbitConfig {
    /// Default download folder.
    pub output_dir: PathBuf,
    /// Listen address; `None` lets librqbit pick (dual-stack, random port).
    pub listen: Option<SocketAddr>,
    pub dht: bool,
    pub upnp: bool,
    /// Where the torrent list is saved so it survives restarts; `None` disables persistence.
    pub persistence_dir: Option<PathBuf>,
}

pub struct RqbitEngine {
    session: Arc<Session>,
}

impl RqbitEngine {
    pub async fn new(cfg: RqbitConfig) -> Result<Self> {
        let mut opts = SessionOptions::default();
        if !cfg.dht {
            opts.dht = None;
        }
        let mut listen = ListenerOptions::default();
        if let Some(addr) = cfg.listen {
            listen.listen_addr = addr;
        }
        listen.enable_upnp_port_forwarding = cfg.upnp;
        opts.listen = Some(listen);
        if let Some(dir) = cfg.persistence_dir {
            opts.persistence = Some(SessionPersistenceConfig::Json { folder: Some(dir) });
            opts.fastresume = true;
        }
        let session = Session::new_with_opts(cfg.output_dir, opts).await.map_err(backend)?;
        Ok(Self { session })
    }

    /// Stop all networking and flush state; call before dropping if the list must be restored next run.
    pub async fn shutdown(&self) {
        self.session.stop().await;
    }

    /// Address the engine actually listens on (useful for tests and the port-check UI).
    pub fn listen_addr(&self) -> Option<SocketAddr> {
        self.session.listen_addr()
    }

    fn handle(&self, id: &TorrentId) -> Result<Arc<ManagedTorrent>> {
        let hash = librqbit::dht::Id20::from_str(&id.0).map_err(|_| EngineError::NotFound)?;
        self.session.get(TorrentIdOrHash::Hash(hash)).ok_or(EngineError::NotFound)
    }
}

fn backend(e: impl std::fmt::Display) -> EngineError {
    EngineError::Backend(format!("{e:#}"))
}

/// Reject garbage before it reaches librqbit so the UI gets `InvalidInput`, not a generic engine error.
fn validate(source: &AddSource) -> Result<()> {
    let bad = |what: &str| Err(EngineError::InvalidInput(what.into()));
    match source {
        AddSource::Magnet(m) => match librqbit::Magnet::parse(m).ok().and_then(|m| m.as_id20()) {
            Some(_) => Ok(()),
            None => bad("not a valid magnet link or info-hash"),
        },
        AddSource::FileBytes(b) => match librqbit::torrent_from_bytes(b) {
            Ok(_) => Ok(()),
            Err(_) => bad("not a valid .torrent file"),
        },
        AddSource::Url(u) if u.starts_with("http://") || u.starts_with("https://") => Ok(()),
        AddSource::Url(_) => bad("only http(s) torrent URLs are supported"),
    }
}

fn to_add(source: AddSource) -> AddTorrent<'static> {
    match source {
        AddSource::Magnet(s) | AddSource::Url(s) => AddTorrent::Url(Cow::Owned(s)),
        AddSource::FileBytes(b) => AddTorrent::from_bytes(b),
    }
}

fn id_of(h: &ManagedTorrent) -> TorrentId {
    TorrentId(h.info_hash().as_string())
}

fn stats_of(h: &ManagedTorrent) -> TorrentStats {
    let s = h.stats();
    let state = match s.state {
        TorrentStatsState::Initializing { paused: true } => TorrentState::Paused,
        TorrentStatsState::Initializing { paused: false } => TorrentState::Checking,
        TorrentStatsState::Live if s.finished => TorrentState::Seeding,
        TorrentStatsState::Live => TorrentState::Downloading,
        TorrentStatsState::Paused => TorrentState::Paused,
        TorrentStatsState::Error => TorrentState::Error,
    };
    let (down_bps, up_bps, peers_live) = match &s.live {
        Some(l) => (
            l.download_speed.as_bytes(),
            l.upload_speed.as_bytes(),
            l.snapshot.peer_stats.live,
        ),
        None => (0, 0, 0),
    };
    // librqbit's own ETA field is private, so derive it from the current speed.
    let remaining = s.total_bytes.saturating_sub(s.progress_bytes);
    let eta_secs = (state == TorrentState::Downloading && down_bps > 0).then(|| remaining / down_bps);
    let id = id_of(h);
    TorrentStats {
        name: h.name().unwrap_or_else(|| id.0.clone()),
        id,
        state,
        error: plain_error(s.error.as_deref()),
        total_bytes: s.total_bytes,
        progress_bytes: s.progress_bytes,
        uploaded_bytes: s.uploaded_bytes,
        down_bps,
        up_bps,
        peers_live,
        eta_secs,
    }
}

#[async_trait]
impl TorrentEngine for RqbitEngine {
    async fn add(&self, req: AddRequest) -> Result<TorrentId> {
        validate(&req.source)?;
        // librqbit resolves magnet metadata before noticing a torrent is already managed; short-circuit.
        if let AddSource::Magnet(m) = &req.source {
            let hash = librqbit::Magnet::parse(m).ok().and_then(|m| m.as_id20());
            if let Some(h) = hash.and_then(|h| self.session.get(TorrentIdOrHash::Hash(h))) {
                return Ok(id_of(&h));
            }
        }
        let add = to_add(req.source);
        let opts = AddTorrentOptions {
            paused: req.paused,
            only_files: req.only_files,
            output_folder: req.output_dir.map(|p| p.to_string_lossy().into_owned()),
            initial_peers: (!req.peers.is_empty()).then_some(req.peers),
            ..Default::default()
        };
        match self.session.add_torrent(add, Some(opts)).await.map_err(backend)? {
            AddTorrentResponse::Added(_, h) | AddTorrentResponse::AlreadyManaged(_, h) => Ok(id_of(&h)),
            AddTorrentResponse::ListOnly(_) => Err(EngineError::Backend("unexpected list-only response".into())),
        }
    }

    async fn inspect(&self, source: AddSource) -> Result<TorrentPreview> {
        validate(&source)?;
        let opts = AddTorrentOptions { list_only: true, ..Default::default() };
        let fut = self.session.add_torrent(to_add(source), Some(opts));
        let resp = tokio::time::timeout(Duration::from_secs(30), fut)
            .await
            .map_err(|_| EngineError::Timeout)?
            .map_err(backend)?;
        let AddTorrentResponse::ListOnly(l) = resp else {
            return Err(EngineError::Backend("unexpected response to a list-only add".into()));
        };
        let files: Vec<PreviewFile> = l
            .info
            .iter_file_details()
            .enumerate()
            .map(|(index, f)| PreviewFile { index, path: f.filename.to_pathbuf().to_string_lossy().into_owned(), size: f.len })
            .collect();
        let id = TorrentId(l.info_hash.as_string());
        Ok(TorrentPreview {
            name: l.info.name().map(|n| n.into_owned()).unwrap_or_else(|| id.0.clone()),
            total_bytes: files.iter().map(|f| f.size).sum(),
            id,
            files,
        })
    }

    async fn list(&self) -> Result<Vec<TorrentStats>> {
        Ok(self.session.with_torrents(|it| it.map(|(_, h)| stats_of(h)).collect()))
    }

    async fn details(&self, id: &TorrentId) -> Result<TorrentDetails> {
        let h = self.handle(id)?;
        let stats = stats_of(&h);
        let selected = h.only_files();
        let progress = h.stats().file_progress;
        // Magnets have no metadata yet: report an empty file list instead of failing.
        let files = h
            .with_metadata(|m| {
                m.file_infos
                    .iter()
                    .enumerate()
                    .map(|(i, f)| FileDetails {
                        index: i,
                        path: f.relative_filename.to_string_lossy().into_owned(),
                        size: f.len,
                        progress_bytes: progress.get(i).copied().unwrap_or(0),
                        selected: selected.as_ref().is_none_or(|s| s.contains(&i)),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut peers: Vec<PeerInfo> = h
            .live()
            .map(|live| {
                live.per_peer_stats_snapshot(PeerStatsFilter::default())
                    .peers
                    .into_iter()
                    .map(|(addr, p)| PeerInfo {
                        addr,
                        client: p.client_name,
                        transport: p.conn_kind.map(|k| format!("{k:?}").to_lowercase()).unwrap_or_default(),
                        downloaded_bytes: p.counters.fetched_bytes,
                        uploaded_bytes: p.counters.uploaded_bytes,
                    })
                    .collect()
            })
            .unwrap_or_default();
        peers.sort_by_key(|p| std::cmp::Reverse(p.downloaded_bytes + p.uploaded_bytes));
        let mut trackers: Vec<String> = h.shared().trackers.iter().map(|u| u.to_string()).collect();
        trackers.sort();
        Ok(TorrentDetails { stats, files, peers, trackers, save_path: h.output_folder().to_string_lossy().into_owned() })
    }

    async fn pause(&self, id: &TorrentId) -> Result<()> {
        let h = self.handle(id)?;
        self.session.pause(&h).await.map_err(backend)
    }

    async fn resume(&self, id: &TorrentId) -> Result<()> {
        let h = self.handle(id)?;
        self.session.unpause(&h).await.map_err(backend)
    }

    async fn remove(&self, id: &TorrentId, delete_files: bool) -> Result<()> {
        let h = self.handle(id)?;
        self.session
            .delete(TorrentIdOrHash::Hash(h.info_hash()), delete_files)
            .await
            .map_err(backend)
    }

    async fn set_files(&self, id: &TorrentId, selected: Vec<usize>) -> Result<()> {
        let h = self.handle(id)?;
        let set: HashSet<usize> = selected.into_iter().collect();
        self.session.update_only_files(&h, &set).await.map_err(backend)
    }

    async fn set_limits(&self, limits: Limits) -> Result<()> {
        self.session.ratelimits.set_download_bps(limits.down_bps.and_then(NonZeroU32::new));
        self.session.ratelimits.set_upload_bps(limits.up_bps.and_then(NonZeroU32::new));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use librqbit::spawn_utils::BlockingSpawner;
    use librqbit::{create_torrent, CreateTorrentOptions};

    use super::*;

    fn payload() -> Vec<u8> {
        (0..3 * 1024 * 1024u32).map(|i| (i.wrapping_mul(31) >> 3) as u8).collect()
    }

    /// A real seeder session and our engine, talking over loopback: no internet, no DHT, no trackers.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn downloads_a_torrent_from_a_loopback_peer() {
        let root = std::env::temp_dir().join(format!("drift-rqbit-test-{}", std::process::id()));
        let (seed_dir, dl_dir) = (root.join("seed"), root.join("dl"));
        std::fs::create_dir_all(&seed_dir).unwrap();
        std::fs::create_dir_all(&dl_dir).unwrap();
        let data = payload();
        let file = seed_dir.join("payload.bin");
        std::fs::write(&file, &data).unwrap();

        let torrent = create_torrent(&file, CreateTorrentOptions::default(), &BlockingSpawner::new(1))
            .await
            .unwrap();
        let torrent_bytes = torrent.as_bytes().unwrap();

        let loopback: SocketAddr = ([127, 0, 0, 1], 0).into();
        let mut seeder_opts = SessionOptions { dht: None, disable_local_service_discovery: true, ..Default::default() };
        seeder_opts.listen = Some(ListenerOptions { listen_addr: loopback, ..Default::default() });
        let seeder = Session::new_with_opts(seed_dir.clone(), seeder_opts).await.unwrap();
        seeder
            .add_torrent(
                AddTorrent::from_bytes(torrent_bytes.clone()),
                Some(AddTorrentOptions { overwrite: true, ..Default::default() }),
            )
            .await
            .unwrap();
        let seeder_addr = seeder.listen_addr().expect("seeder listens");

        let engine = RqbitEngine::new(RqbitConfig {
            output_dir: dl_dir.clone(),
            listen: Some(loopback),
            dht: false,
            upnp: false,
            persistence_dir: None,
        })
        .await
        .unwrap();
        let mut req = AddRequest::new(AddSource::FileBytes(torrent_bytes.to_vec()));
        req.peers = vec![SocketAddr::from(([127, 0, 0, 1], seeder_addr.port()))];
        let id = engine.add(req).await.unwrap();
        assert_eq!(engine.add(AddRequest::new(AddSource::Magnet(id.0.clone()))).await.unwrap(), id);

        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let s = engine.list().await.unwrap().into_iter().find(|s| s.id == id).unwrap();
            if s.state == TorrentState::Seeding {
                break;
            }
            assert!(Instant::now() < deadline, "timed out, last state {:?} {}/{}", s.state, s.progress_bytes, s.total_bytes);
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        assert_eq!(std::fs::read(dl_dir.join("payload.bin")).unwrap(), data);
        let d = engine.details(&id).await.unwrap();
        assert_eq!(d.files.len(), 1);
        assert!(d.files[0].selected);

        engine.pause(&id).await.unwrap();
        assert_eq!(engine.details(&id).await.unwrap().stats.state, TorrentState::Paused);
        engine.resume(&id).await.unwrap();
        engine.remove(&id, false).await.unwrap();
        assert!(engine.list().await.unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Multi-file torrent over loopback: download only one file, then widen the selection while running.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn file_selection_limits_then_widens_the_download() {
        let root = std::env::temp_dir().join(format!("drift-select-test-{}", std::process::id()));
        let (seed_dir, dl_dir) = (root.join("seed"), root.join("dl"));
        let content = seed_dir.join("album");
        std::fs::create_dir_all(&content).unwrap();
        std::fs::create_dir_all(&dl_dir).unwrap();
        let (a, b) = (payload(), payload().into_iter().rev().collect::<Vec<u8>>());
        std::fs::write(content.join("a.bin"), &a).unwrap();
        std::fs::write(content.join("b.bin"), &b).unwrap();

        let torrent = create_torrent(&content, CreateTorrentOptions::default(), &BlockingSpawner::new(1))
            .await
            .unwrap()
            .as_bytes()
            .unwrap();
        let loopback: SocketAddr = ([127, 0, 0, 1], 0).into();
        let seeder_opts = SessionOptions {
            dht: None,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions { listen_addr: loopback, ..Default::default() }),
            ..Default::default()
        };
        let seeder = Session::new_with_opts(seed_dir.clone(), seeder_opts).await.unwrap();
        seeder
            .add_torrent(AddTorrent::from_bytes(torrent.clone()), Some(AddTorrentOptions { overwrite: true, ..Default::default() }))
            .await
            .unwrap();
        let seeder_port = seeder.listen_addr().expect("seeder listens").port();

        let engine = RqbitEngine::new(RqbitConfig { output_dir: dl_dir.clone(), listen: Some(loopback), dht: false, upnp: false, persistence_dir: None })
            .await
            .unwrap();
        let mut req = AddRequest::new(AddSource::FileBytes(torrent.to_vec()));
        req.peers = vec![SocketAddr::from(([127, 0, 0, 1], seeder_port))];
        req.only_files = Some(vec![0]);
        let id = engine.add(req).await.unwrap();

        let wait_for = |what: &'static str, f: Box<dyn Fn(&TorrentDetails) -> bool + Send>| {
            let engine = &engine;
            let id = id.clone();
            async move {
                let deadline = Instant::now() + Duration::from_secs(60);
                loop {
                    let d = engine.details(&id).await.unwrap();
                    if f(&d) {
                        return d;
                    }
                    assert!(Instant::now() < deadline, "timed out waiting for: {what}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        };

        // First: only file 0 is selected and gets downloaded; file 1 stays untouched.
        let d = wait_for("file 0 complete", Box::new(|d| d.files.first().is_some_and(|f| f.progress_bytes == f.size && f.size > 0))).await;
        assert_eq!(d.files.len(), 2);
        assert!(d.files[0].selected && !d.files[1].selected, "selection should be [true, false]");
        // A piece spanning the file boundary legitimately brings in a little of file 1, never all of it.
        assert!(d.files[1].progress_bytes < d.files[1].size, "unselected file must not fully download");
        assert!(!d.save_path.is_empty());

        // Then widen the selection while the torrent is live.
        engine.set_files(&id, vec![0, 1]).await.unwrap();
        let d = wait_for("both files complete", Box::new(|d| d.files.iter().all(|f| f.progress_bytes == f.size))).await;
        assert!(d.files.iter().all(|f| f.selected));
        assert_eq!(d.stats.state, TorrentState::Seeding);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Add-dialog preview and input validation, no network needed (the torrent file carries its metadata).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inspect_reads_files_and_rejects_garbage() {
        let root = std::env::temp_dir().join(format!("drift-inspect-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("inspect.bin");
        std::fs::write(&file, payload()).unwrap();
        let torrent = create_torrent(&file, CreateTorrentOptions::default(), &BlockingSpawner::new(1))
            .await
            .unwrap()
            .as_bytes()
            .unwrap();

        let engine = RqbitEngine::new(RqbitConfig {
            output_dir: root.clone(),
            listen: Some(([127, 0, 0, 1], 0).into()),
            dht: false,
            upnp: false,
            persistence_dir: None,
        })
        .await
        .unwrap();

        let p = engine.inspect(AddSource::FileBytes(torrent.to_vec())).await.unwrap();
        assert_eq!(p.name, "inspect.bin");
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.total_bytes, payload().len() as u64);
        assert!(engine.list().await.unwrap().is_empty(), "inspect must not start the torrent");

        for bad in [
            AddSource::Magnet("definitely not a magnet".into()),
            AddSource::FileBytes(b"not a torrent".to_vec()),
            AddSource::Url("ftp://example.org/x.torrent".into()),
        ] {
            assert!(matches!(engine.inspect(bad.clone()).await, Err(EngineError::InvalidInput(_))));
            assert!(matches!(engine.add(AddRequest::new(bad)).await, Err(EngineError::InvalidInput(_))));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M1-07: the torrent list and its paused state survive a restart.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn torrent_list_survives_restart() {
        let root = std::env::temp_dir().join(format!("drift-persist-test-{}", std::process::id()));
        let (src, dl, state) = (root.join("src"), root.join("dl"), root.join("state"));
        for d in [&src, &dl, &state] {
            std::fs::create_dir_all(d).unwrap();
        }
        let file = src.join("persist.bin");
        std::fs::write(&file, payload()).unwrap();
        let torrent = create_torrent(&file, CreateTorrentOptions::default(), &BlockingSpawner::new(1))
            .await
            .unwrap()
            .as_bytes()
            .unwrap();

        let cfg = || RqbitConfig {
            output_dir: dl.clone(),
            listen: Some(([127, 0, 0, 1], 0).into()),
            dht: false,
            upnp: false,
            persistence_dir: Some(state.clone()),
        };
        let first = RqbitEngine::new(cfg()).await.unwrap();
        let mut req = AddRequest::new(AddSource::FileBytes(torrent.to_vec()));
        req.paused = true;
        let id = first.add(req).await.unwrap();
        first.shutdown().await;
        drop(first);

        let second = RqbitEngine::new(cfg()).await.unwrap();
        let list = second.list().await.unwrap();
        assert_eq!(list.len(), 1, "torrent list was not restored");
        assert_eq!(list[0].id, id);
        let deadline = Instant::now() + Duration::from_secs(10);
        while second.list().await.unwrap()[0].state != TorrentState::Paused {
            assert!(Instant::now() < deadline, "restored torrent never reported Paused");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        second.shutdown().await;

        let _ = std::fs::remove_dir_all(&root);
    }
}
