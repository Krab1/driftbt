//! Adapter over `librqbit` 9.0.1. This is the ONLY file allowed to use librqbit (CLAUDE.md rule 2).

use std::borrow::Cow;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use librqbit::api::TorrentIdOrHash;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerOptions, ManagedTorrent, Session,
    SessionOptions, TorrentStatsState,
};

use super::{EngineError, Result, TorrentEngine};
use crate::model::*;

pub struct RqbitConfig {
    /// Default download folder.
    pub output_dir: PathBuf,
    /// Listen address; `None` lets librqbit pick (dual-stack, random port).
    pub listen: Option<SocketAddr>,
    pub dht: bool,
    pub upnp: bool,
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
        let session = Session::new_with_opts(cfg.output_dir, opts).await.map_err(backend)?;
        Ok(Self { session })
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

fn id_of(h: &ManagedTorrent) -> TorrentId {
    TorrentId(h.info_hash().as_string())
}

fn stats_of(h: &ManagedTorrent) -> TorrentStats {
    let s = h.stats();
    let state = match s.state {
        TorrentStatsState::Initializing { .. } => TorrentState::Checking,
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
        error: s.error,
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
        // librqbit resolves magnet metadata before noticing a torrent is already managed; short-circuit.
        if let AddSource::Magnet(m) = &req.source {
            let hash = librqbit::Magnet::parse(m)
                .ok()
                .and_then(|m| m.as_id20())
                .ok_or_else(|| EngineError::InvalidInput("not a valid magnet link or info-hash".into()))?;
            if let Some(h) = self.session.get(TorrentIdOrHash::Hash(hash)) {
                return Ok(id_of(&h));
            }
        }
        let add = match req.source {
            AddSource::Magnet(s) | AddSource::Url(s) => AddTorrent::Url(Cow::Owned(s)),
            AddSource::FileBytes(b) => AddTorrent::from_bytes(b),
        };
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
        Ok(TorrentDetails { stats, files })
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

        let engine = RqbitEngine::new(RqbitConfig { output_dir: dl_dir.clone(), listen: Some(loopback), dht: false, upnp: false })
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
}
