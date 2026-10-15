//! Adapter over `librqbit` 9.0.1. This is the ONLY file allowed to use librqbit (CLAUDE.md rule 2).

use std::borrow::Cow;
use std::collections::HashSet;
use std::net::{Ipv6Addr, SocketAddr};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use librqbit::api::TorrentIdOrHash;
use librqbit::http_api_types::PeerStatsFilter;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ConnectionOptions, ListenerOptions, ManagedTorrent, Session,
    SessionOptions, SessionPersistenceConfig, TorrentStatsState,
};

use super::{EngineError, OpenFile, Result, TorrentEngine};
use crate::errors::plain_error;
use crate::model::*;

#[derive(Clone)]
pub struct RqbitConfig {
    /// Default download folder.
    pub output_dir: PathBuf,
    /// Listen address; `None` lets librqbit pick (dual-stack, random port).
    pub listen: Option<SocketAddr>,
    pub dht: bool,
    pub upnp: bool,
    /// Where the torrent list is saved so it survives restarts; `None` disables persistence.
    pub persistence_dir: Option<PathBuf>,
    /// `socks5://[user:pass@]host:port`.
    pub proxy: Option<String>,
    /// Normalised IP filter list (`name:start-end` lines).
    pub blocklist_file: Option<PathBuf>,
}

impl Default for RqbitConfig {
    fn default() -> Self {
        Self { output_dir: PathBuf::new(), listen: None, dht: true, upnp: false, persistence_dir: None, proxy: None, blocklist_file: None }
    }
}

impl RqbitConfig {
    /// Only the settings that need a new session when they change.
    fn network_key(&self) -> (Option<SocketAddr>, bool, bool, &Option<String>, &Option<PathBuf>) {
        (self.listen, self.dht, self.upnp, &self.proxy, &self.blocklist_file)
    }
}

pub struct RqbitEngine {
    session: RwLock<Arc<Session>>,
    cfg: Mutex<RqbitConfig>,
    limits: Mutex<Limits>,
    /// Serialises session rebuilds.
    rebuild: tokio::sync::Mutex<()>,
    geo: crate::geoip::GeoIp,
}

/// Everything needed to add a torrent again exactly as it was.
struct Saved {
    hash: librqbit::dht::Id20,
    bytes: Vec<u8>,
    paused: bool,
    only_files: Option<Vec<usize>>,
    output: PathBuf,
    files: Vec<PathBuf>,
    name: String,
}

/// `file:///C:/dir/a%20b.txt`, which is how the engine takes a list from disk.
fn file_url(p: &Path) -> String {
    let path = p.display().to_string().replace('\\', "/").replace('%', "%25").replace(' ', "%20");
    format!("file:///{}", path.trim_start_matches('/'))
}

/// A skipped file is only written where it shares a download piece with a file you picked, so it should be a mostly
/// empty (sparse) file that takes almost no disk. librqbit tries to mark files sparse on Windows but its call to the
/// OS leaves out an argument the call requires, so it never takes effect and a skipped 600 MB file ends up taking
/// 600 MB. This wraps librqbit's file storage and marks a file sparse just before its first write, when it is still
/// empty (files you picked are given their full length up front and are left alone).
#[cfg(windows)]
mod sparse_storage {
    use std::fs::OpenOptions;
    use std::io::IoSlice;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, RwLock, Weak};

    use librqbit::api::TorrentIdOrHash;
    use librqbit::dht::Id20;
    use librqbit::storage::filesystem::FilesystemStorageFactory;
    use librqbit::storage::{BoxStorageFactory, StorageFactory, TorrentStorage};
    use librqbit::{ManagedTorrentShared, Session, TorrentMetadata};

    /// Where to find the session (and so each torrent's folder) once files are written.
    pub type SessionLink = Arc<RwLock<Weak<Session>>>;

    #[derive(Clone)]
    pub struct SparseFactory(pub SessionLink);

    impl StorageFactory for SparseFactory {
        type Storage = Box<dyn TorrentStorage>;

        fn create(&self, shared: &ManagedTorrentShared, metadata: &TorrentMetadata) -> anyhow::Result<Box<dyn TorrentStorage>> {
            let files: Vec<PathBuf> = metadata.file_infos.iter().map(|f| f.relative_filename.clone()).collect();
            Ok(Box::new(SparseStorage {
                inner: Box::new(FilesystemStorageFactory::default().create(shared, metadata)?),
                hash: shared.info_hash,
                marked: Arc::new(files.iter().map(|_| Mutex::new(false)).collect()),
                files: Arc::new(files),
                link: self.0.clone(),
            }))
        }

        /// The session saver only accepts the filesystem storage; this is that storage (same files, same layout).
        fn is_type_id(&self, type_id: std::any::TypeId) -> bool {
            type_id == std::any::TypeId::of::<FilesystemStorageFactory>()
        }

        fn clone_box(&self) -> BoxStorageFactory {
            Box::new(self.clone())
        }
    }

    pub struct SparseStorage {
        inner: Box<dyn TorrentStorage>,
        hash: Id20,
        files: Arc<Vec<PathBuf>>,
        /// Per file: already looked at. Held while marking, so a second writer waits for the mark before it writes.
        marked: Arc<Vec<Mutex<bool>>>,
        link: SessionLink,
    }

    impl SparseStorage {
        fn before_write(&self, file_id: usize) {
            let Some(done) = self.marked.get(file_id) else { return };
            let mut done = done.lock().unwrap();
            if *done {
                return;
            }
            let Some(session) = self.link.read().unwrap().upgrade() else { return };
            let Some(handle) = session.get(TorrentIdOrHash::Hash(self.hash)) else { return };
            // A file you picked is written whole in the end, and is better off as an ordinary file.
            if handle.only_files().is_none_or(|only| only.contains(&file_id)) {
                *done = true;
                return;
            }
            let path: &Path = &handle.output_folder().join(&self.files[file_id]);
            *done = true;
            if let Ok(f) = OpenOptions::new().write(true).open(path) {
                if f.metadata().is_ok_and(|m| m.len() == 0) {
                    let _ = crate::fsmove::sparse::mark_sparse(&f);
                }
            }
        }
    }

    impl TorrentStorage for SparseStorage {
        fn init(&mut self, shared: &ManagedTorrentShared, metadata: &TorrentMetadata) -> anyhow::Result<()> {
            self.inner.init(shared, metadata)
        }

        fn pread_exact(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
            self.inner.pread_exact(file_id, offset, buf)
        }

        fn pwrite_all(&self, file_id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
            self.before_write(file_id);
            self.inner.pwrite_all(file_id, offset, buf)
        }

        fn pwrite_all_vectored(&self, file_id: usize, offset: u64, bufs: [IoSlice<'_>; 2]) -> anyhow::Result<usize> {
            self.before_write(file_id);
            // Through the trait object itself: the `Box` impl in librqbit would split this into two writes.
            (*self.inner).pwrite_all_vectored(file_id, offset, bufs)
        }

        fn remove_file(&self, file_id: usize, filename: &Path) -> anyhow::Result<()> {
            self.inner.remove_file(file_id, filename)
        }

        fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
            self.inner.remove_directory_if_empty(path)
        }

        fn ensure_file_length(&self, file_id: usize, length: u64) -> anyhow::Result<()> {
            self.inner.ensure_file_length(file_id, length)
        }

        fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
            Ok(Box::new(SparseStorage {
                inner: self.inner.take()?,
                hash: self.hash,
                files: self.files.clone(),
                marked: self.marked.clone(),
                link: self.link.clone(),
            }))
        }
    }
}

async fn build_session(cfg: &RqbitConfig) -> Result<Arc<Session>> {
    let mut opts = SessionOptions::default();
    #[cfg(windows)]
    let link: sparse_storage::SessionLink = Arc::new(RwLock::new(std::sync::Weak::new()));
    #[cfg(windows)]
    {
        opts.default_storage_factory = Some(Box::new(sparse_storage::SparseFactory(link.clone())));
    }
    let proxied = cfg.proxy.is_some();
    // Behind a proxy only outgoing TCP can be proxied: DHT (UDP) and listening would expose the real address.
    if !cfg.dht || proxied {
        opts.dht = None;
    }
    if !proxied {
        let mut listen = ListenerOptions::default();
        if let Some(addr) = cfg.listen {
            listen.listen_addr = addr;
        }
        listen.enable_upnp_port_forwarding = cfg.upnp;
        opts.listen = Some(listen);
    }
    if let Some(url) = &cfg.proxy {
        opts.connect = Some(ConnectionOptions { proxy_url: Some(url.clone()), ..Default::default() });
    }
    if let Some(file) = &cfg.blocklist_file {
        opts.blocklist_url = Some(file_url(file));
    }
    if let Some(dir) = &cfg.persistence_dir {
        opts.persistence = Some(SessionPersistenceConfig::Json { folder: Some(dir.clone()) });
        opts.fastresume = true;
    }
    let session = Session::new_with_opts(cfg.output_dir.clone(), opts).await.map_err(backend)?;
    #[cfg(windows)]
    {
        *link.write().unwrap() = Arc::downgrade(&session);
    }
    Ok(session)
}

impl RqbitEngine {
    pub async fn new(cfg: RqbitConfig) -> Result<Self> {
        let session = build_session(&cfg).await?;
        Ok(Self {
            session: RwLock::new(session),
            cfg: Mutex::new(cfg),
            limits: Mutex::new(Limits::default()),
            rebuild: tokio::sync::Mutex::new(()),
            geo: crate::geoip::GeoIp::embedded(),
        })
    }

    fn session(&self) -> Arc<Session> {
        self.session.read().unwrap().clone()
    }

    /// Stop all networking and flush state; call before dropping if the list must be restored next run.
    pub async fn shutdown(&self) {
        self.session().stop().await;
    }

    /// Address the engine actually listens on (useful for tests and the port-check UI).
    pub fn listen_addr(&self) -> Option<SocketAddr> {
        self.session().listen_addr()
    }

    fn saved(&self, id: &TorrentId) -> Result<Saved> {
        let h = self.handle(id)?;
        let (bytes, files) = h.with_metadata(|m| (m.torrent_bytes.to_vec(), m.file_infos.iter().map(|f| f.relative_filename.clone()).collect::<Vec<_>>())).map_err(backend)?;
        if bytes.is_empty() {
            return Err(EngineError::Backend("torrent file bytes are not available".into()));
        }
        Ok(Saved { hash: h.info_hash(), bytes, paused: stats_of(&h).state == TorrentState::Paused, only_files: h.only_files(), output: h.output_folder().to_path_buf(), files, name: h.name().unwrap_or_default() })
    }

    /// Adds a torrent back, existing files being checked rather than refused. `output` overrides the save folder.
    async fn add_saved(&self, s: Saved, output: Option<PathBuf>) -> Result<()> {
        let opts = AddTorrentOptions {
            paused: s.paused,
            overwrite: true,
            only_files: s.only_files,
            output_folder: Some(output.unwrap_or(s.output).to_string_lossy().into_owned()),
            ..Default::default()
        };
        self.session().add_torrent(AddTorrent::from_bytes(s.bytes), Some(opts)).await.map_err(backend)?;
        Ok(())
    }

    fn handle(&self, id: &TorrentId) -> Result<Arc<ManagedTorrent>> {
        let hash = librqbit::dht::Id20::from_str(&id.0).map_err(|_| EngineError::NotFound)?;
        self.session().get(TorrentIdOrHash::Hash(hash)).ok_or(EngineError::NotFound)
    }

    fn apply_limits(session: &Session, l: Limits) {
        session.ratelimits.set_download_bps(l.down_bps.and_then(NonZeroU32::new));
        session.ratelimits.set_upload_bps(l.up_bps.and_then(NonZeroU32::new));
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

/// The folder name a torrent gets: its name with the characters Windows forbids replaced.
fn folder_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if r#"/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().trim_end_matches('.').trim();
    if clean.is_empty() { "torrent".into() } else { clean.into() }
}

/// The folder a torrent's files go in: a multi-file torrent gets its own folder named after it inside `base`, a
/// single file goes straight into `base` unless `nest_single` asks for a folder too. (librqbit does this itself only
/// when it chooses the folder; when we give it one it uses it as is, which used to spill a torrent's `game/`,
/// `lib/`... straight into the save folder.)
fn torrent_folder(base: &Path, name: &str, files: usize, nest_single: bool) -> PathBuf {
    if files <= 1 && !nest_single {
        return base.to_path_buf();
    }
    base.join(folder_name(name))
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
        error_detail: s.error.clone(),
        total_bytes: s.total_bytes,
        progress_bytes: s.progress_bytes,
        uploaded_bytes: s.uploaded_bytes,
        down_bps,
        up_bps,
        peers_live,
        eta_secs,
        tracker: h.shared().trackers.iter().min().and_then(|u| u.host_str().map(str::to_owned)),
    }
}

#[async_trait]
impl TorrentEngine for RqbitEngine {
    async fn add(&self, req: AddRequest) -> Result<TorrentId> {
        validate(&req.source)?;
        // librqbit resolves magnet metadata before noticing a torrent is already managed; short-circuit.
        if let AddSource::Magnet(m) = &req.source {
            let hash = librqbit::Magnet::parse(m).ok().and_then(|m| m.as_id20());
            if let Some(h) = hash.and_then(|h| self.session().get(TorrentIdOrHash::Hash(h))) {
                return Ok(id_of(&h));
            }
        }
        // Saving the torrent list is a file swap that Windows sometimes refuses for a moment (a virus scanner
        // looking at the new file, say). Trying again almost always works, and the user never sees the hiccup.
        // Where the files go: needs the torrent's name and file count, which a list-only look gives.
        let output_folder = match &req.output_dir {
            None => None,
            Some(base) => Some(match self.inspect(req.source.clone()).await {
                Ok(p) => torrent_folder(base, &p.name, p.files.len(), req.folder_for_single),
                // A magnet nobody answers yet: keep it out of the save folder's root until the name is known.
                Err(EngineError::Timeout) => {
                    let hash = match &req.source {
                        AddSource::Magnet(m) => librqbit::Magnet::parse(m).ok().and_then(|m| m.as_id20()).map(|h| h.as_string()).unwrap_or_default(),
                        _ => String::new(),
                    };
                    base.join(format!("magnet-{}", hash.chars().take(8).collect::<String>()))
                }
                Err(e) => return Err(e),
            }),
        };
        let mut last = String::new();
        for attempt in 0..4 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            let opts = AddTorrentOptions {
                paused: req.paused,
                only_files: req.only_files.clone(),
                output_folder: output_folder.clone().map(|p| p.to_string_lossy().into_owned()),
                initial_peers: (!req.peers.is_empty()).then(|| req.peers.clone()),
                ..Default::default()
            };
            match self.session().add_torrent(to_add(req.source.clone()), Some(opts)).await {
                Ok(AddTorrentResponse::Added(_, h) | AddTorrentResponse::AlreadyManaged(_, h)) => return Ok(id_of(&h)),
                Ok(AddTorrentResponse::ListOnly(_)) => return Err(EngineError::Backend("unexpected list-only response".into())),
                Err(e) => {
                    last = format!("{e:#}");
                    if !last.to_lowercase().contains("persistence") {
                        break;
                    }
                }
            }
        }
        Err(EngineError::Backend(last))
    }

    async fn inspect(&self, source: AddSource) -> Result<TorrentPreview> {
        validate(&source)?;
        let opts = AddTorrentOptions { list_only: true, ..Default::default() };
        let session = self.session();
        let fut = session.add_torrent(to_add(source), Some(opts));
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

    async fn open_file(&self, id: &TorrentId, file: usize) -> Result<OpenFile> {
        let h = self.handle(id)?;
        let stream = h.stream(file).await.map_err(backend)?;
        let len = stream.len();
        Ok(OpenFile { len, reader: Box::new(Box::pin(stream)) })
    }

    async fn list(&self) -> Result<Vec<TorrentStats>> {
        Ok(self.session().with_torrents(|it| it.map(|(_, h)| stats_of(h)).collect()))
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
                        country: self.geo.country_of_addr(&addr),
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
        self.session().pause(&h).await.map_err(backend)
    }

    async fn resume(&self, id: &TorrentId) -> Result<()> {
        let h = self.handle(id)?;
        self.session().unpause(&h).await.map_err(backend)
    }

    async fn remove(&self, id: &TorrentId, delete_files: bool) -> Result<()> {
        let h = self.handle(id)?;
        self.session()
            .delete(TorrentIdOrHash::Hash(h.info_hash()), delete_files)
            .await
            .map_err(backend)
    }

    async fn recheck(&self, id: &TorrentId) -> Result<()> {
        let saved = self.saved(id)?;
        let session = self.session();
        session.delete(TorrentIdOrHash::Hash(saved.hash), false).await.map_err(backend)?;
        self.add_saved(saved, None).await
    }

    async fn relocate(&self, id: &TorrentId, dest: &Path) -> Result<()> {
        let saved = self.saved(id)?;
        // A single file that already sits in a folder of its own stays in one at the new place.
        let nested = saved.files.len() == 1 && saved.output.file_name().is_some_and(|f| *f == *folder_name(&saved.name));
        let dest = torrent_folder(dest, &saved.name, saved.files.len(), nested);
        let dest = dest.as_path();
        if saved.output == dest {
            return Ok(());
        }
        let session = self.session();
        // Deleting from the session (not from disk) closes the files so they can be moved.
        session.delete(TorrentIdOrHash::Hash(saved.hash), false).await.map_err(backend)?;
        if let Err(e) = crate::fsmove::move_files(&saved.output, dest, &saved.files) {
            // Nothing moved: carry on where it was.
            let _ = self.add_saved(saved, None).await;
            return Err(EngineError::Backend(e.to_string()));
        }
        self.add_saved(saved, Some(dest.to_path_buf())).await
    }

    async fn set_files(&self, id: &TorrentId, selected: Vec<usize>) -> Result<()> {
        let h = self.handle(id)?;
        let set: HashSet<usize> = selected.into_iter().collect();
        self.session().update_only_files(&h, &set).await.map_err(backend)
    }

    async fn set_limits(&self, limits: Limits) -> Result<()> {
        *self.limits.lock().unwrap() = limits;
        Self::apply_limits(&self.session(), limits);
        Ok(())
    }

    async fn set_network(&self, n: NetworkConfig) -> Result<()> {
        let _only_one_rebuild_at_a_time = self.rebuild.lock().await;
        let old_cfg = self.cfg.lock().unwrap().clone();
        let mut new_cfg = old_cfg.clone();
        new_cfg.listen = n.listen_port.map(|p| SocketAddr::from((Ipv6Addr::UNSPECIFIED, p)));
        new_cfg.dht = n.dht;
        new_cfg.upnp = n.upnp;
        new_cfg.proxy = n.proxy.filter(|p| !p.trim().is_empty());
        new_cfg.blocklist_file = n.blocklist_file;
        if new_cfg.network_key() == old_cfg.network_key() {
            return Ok(());
        }
        if let Some(p) = &new_cfg.proxy {
            let ok = reqwest::Url::parse(p).is_ok_and(|u| u.scheme() == "socks5" && u.host_str().is_some() && u.port().is_some());
            if !ok {
                return Err(EngineError::InvalidInput("not a valid socks5://host:port proxy address".into()));
            }
        }

        // The old session must release its port and state files before a new one can take them over.
        self.session().stop().await;
        let session = match build_session(&new_cfg).await {
            Ok(s) => s,
            Err(e) => {
                // Put the previous settings back so the app isn't left without an engine.
                let restored = build_session(&old_cfg).await?;
                Self::apply_limits(&restored, *self.limits.lock().unwrap());
                *self.session.write().unwrap() = restored;
                return Err(e);
            }
        };
        Self::apply_limits(&session, *self.limits.lock().unwrap());
        *self.session.write().unwrap() = session;
        *self.cfg.lock().unwrap() = new_cfg;
        Ok(())
    }

    async fn network_status(&self) -> Result<NetworkStatus> {
        Ok(NetworkStatus { listen_port: self.session().listen_addr().map(|a| a.port()) })
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
            ..Default::default()
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

        let engine = RqbitEngine::new(RqbitConfig { output_dir: dl_dir.clone(), listen: Some(loopback), dht: false, upnp: false, persistence_dir: None, ..Default::default() })
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
        // Playlists and "Show in folder" rely on save_path + file path being the real file on disk.
        for f in &d.files {
            let on_disk = std::path::Path::new(&d.save_path).join(&f.path);
            assert!(on_disk.exists(), "{} is not where details say it is", on_disk.display());
        }
        // The stream reader returns the real bytes, seekable.
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let mut open = engine.open_file(&id, 1).await.unwrap();
        assert_eq!(open.len, b.len() as u64);
        open.reader.seek(std::io::SeekFrom::Start(1000)).await.unwrap();
        let mut buf = vec![0u8; 64];
        open.reader.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, &b[1000..1064]);

        let _ = std::fs::remove_dir_all(&root);
    }


    // ---- M5: IP filter and network settings -------------------------------------------------------

    struct Seeder {
        _session: Arc<Session>,
        torrent: Vec<u8>,
        addr: SocketAddr,
    }

    /// A real seeding session on loopback; `blocklist` makes it refuse the listed addresses.
    async fn seeder(dir: &Path, blocklist: Option<&Path>) -> Seeder {
        let file = dir.join("seedme.bin");
        std::fs::write(&file, payload()).unwrap();
        let torrent = create_torrent(&file, CreateTorrentOptions::default(), &BlockingSpawner::new(1))
            .await
            .unwrap()
            .as_bytes()
            .unwrap();
        let loopback: SocketAddr = ([127, 0, 0, 1], 0).into();
        let opts = SessionOptions {
            dht: None,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions { listen_addr: loopback, ..Default::default() }),
            blocklist_url: blocklist.map(file_url),
            ..Default::default()
        };
        let session = Session::new_with_opts(dir.to_path_buf(), opts).await.unwrap();
        session
            .add_torrent(AddTorrent::from_bytes(torrent.clone()), Some(AddTorrentOptions { overwrite: true, ..Default::default() }))
            .await
            .unwrap();
        let port = session.listen_addr().expect("seeder listens").port();
        Seeder { _session: session, torrent: torrent.to_vec(), addr: SocketAddr::from(([127, 0, 0, 1], port)) }
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("drift-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn engine_cfg(dl: &Path, blocklist: Option<&Path>) -> RqbitConfig {
        RqbitConfig {
            output_dir: dl.to_path_buf(),
            listen: Some(([127, 0, 0, 1], 0).into()),
            dht: false,
            blocklist_file: blocklist.map(Path::to_path_buf),
            ..Default::default()
        }
    }

    async fn download_from(engine: &RqbitEngine, s: &Seeder) -> TorrentId {
        let mut req = AddRequest::new(AddSource::FileBytes(s.torrent.clone()));
        req.peers = vec![s.addr];
        engine.add(req).await.unwrap()
    }

    async fn state_after(engine: &RqbitEngine, id: &TorrentId, secs: u64) -> (TorrentState, u64) {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let t = engine.list().await.unwrap().into_iter().find(|t| &t.id == id).unwrap();
            if t.state == TorrentState::Seeding || Instant::now() >= deadline {
                return (t.state, t.progress_bytes);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// A damaged file is found by a recheck (progress drops, good pieces stay); a repaired one checks out clean.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn recheck_finds_damage_and_clears_a_repaired_file() {
        let root = scratch("recheck");
        let (seed_dir, dl) = (root.join("seed"), root.join("dl"));
        std::fs::create_dir_all(&seed_dir).unwrap();
        std::fs::create_dir_all(&dl).unwrap();
        let sd = seeder(&seed_dir, None).await;
        let engine = RqbitEngine::new(engine_cfg(&dl, None)).await.unwrap();
        let id = download_from(&engine, &sd).await;
        assert_eq!(state_after(&engine, &id, 60).await.0, TorrentState::Seeding);
        let original = payload();
        let sd_len = || original.len() as u64;

        // Pause so the files are closed, then damage the middle of the file without changing its size.
        engine.pause(&id).await.unwrap();
        let file = dl.join("seedme.bin");
        let mut bad = std::fs::read(&file).unwrap();
        let mid = bad.len() / 2;
        bad[mid..mid + 64].fill(0xAA);
        std::fs::write(&file, &bad).unwrap();

        engine.recheck(&id).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let found = loop {
            let t = engine.list().await.unwrap().into_iter().find(|t| t.id == id).unwrap();
            // While the check runs a paused torrent also reads as Paused with no progress, so wait for real numbers.
            if t.state == TorrentState::Paused && t.progress_bytes > 0 && t.progress_bytes < t.total_bytes {
                break t.progress_bytes;
            }
            assert!(Instant::now() < deadline, "recheck never reported the damage, last {:?} {}/{}", t.state, t.progress_bytes, t.total_bytes);
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        assert!(found > 0 && found < sd_len(), "undamaged pieces are kept, the damaged one is not");

        // Put the good bytes back (a repair): the next check finds everything and the torrent seeds again.
        std::fs::write(&file, &original).unwrap();
        engine.recheck(&id).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let t = engine.list().await.unwrap().into_iter().find(|t| t.id == id).unwrap();
            if t.state == TorrentState::Paused && t.progress_bytes == t.total_bytes {
                break;
            }
            assert!(Instant::now() < deadline, "the repaired file never checked out clean");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        engine.resume(&id).await.unwrap();
        assert_eq!(state_after(&engine, &id, 30).await.0, TorrentState::Seeding);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Four files in two folders, two picked: only those come down; a deleted picked file is fetched again after a
    /// recheck, and the files that were never picked stay absent throughout.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn only_picked_files_download_and_a_recheck_refetches_a_deleted_one_but_not_the_skipped() {
        let root = scratch("pick-recheck");
        let (seed_dir, dl) = (root.join("seed"), root.join("dl"));
        let content = seed_dir.join("pack");
        std::fs::create_dir_all(content.join("docs")).unwrap();
        std::fs::create_dir_all(content.join("media")).unwrap();
        std::fs::create_dir_all(&dl).unwrap();
        // Each file is a whole number of 64 KiB pieces, so no piece straddles two files and "skipped" means untouched.
        let body = |salt: u8| -> Vec<u8> { (0..256 * 1024u32).map(|i| (i.wrapping_mul(31) >> 3) as u8 ^ salt).collect() };
        let names = ["docs/a.bin", "docs/b.bin", "media/c.bin", "media/d.bin"];
        for (n, name) in names.iter().enumerate() {
            std::fs::write(content.join(name), body(n as u8 + 1)).unwrap();
        }
        let torrent = create_torrent(&content, CreateTorrentOptions { piece_length: Some(64 * 1024), ..Default::default() }, &BlockingSpawner::new(1))
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
        let engine = RqbitEngine::new(engine_cfg(&dl, None)).await.unwrap();

        // Pick a.bin (docs) and c.bin (media); skip b.bin and d.bin.
        let mut req = AddRequest::new(AddSource::FileBytes(torrent.to_vec()));
        req.peers = vec![SocketAddr::from(([127, 0, 0, 1], seeder_port))];
        req.only_files = Some(vec![0, 2]);
        let id = engine.add(req).await.unwrap();

        let wait = |what: &'static str, f: Box<dyn Fn(&TorrentDetails) -> bool + Send>| {
            let engine = &engine;
            let id = id.clone();
            async move {
                let deadline = Instant::now() + Duration::from_secs(60);
                loop {
                    let d = engine.details(&id).await.unwrap();
                    if f(&d) {
                        return d;
                    }
                    assert!(Instant::now() < deadline, "timed out waiting for: {what}; files: {:?}", d.files.iter().map(|f| (f.path.clone(), f.selected, f.progress_bytes)).collect::<Vec<_>>());
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        };
        let full = |f: &FileDetails| f.progress_bytes == f.size && f.size > 0;
        let picked = [0usize, 2];

        // 1) the picked files complete and are on disk with the right bytes
        let d = wait("picked files complete", Box::new(move |d| picked.iter().all(|&i| d.files.get(i).is_some_and(full)))).await;
        assert_eq!(d.files.len(), 4);
        for (i, f) in d.files.iter().enumerate() {
            assert_eq!(f.path.replace('\\', "/"), names[i].to_string(), "file order");
            assert_eq!(f.selected, picked.contains(&i));
        }
        let on_disk = |i: usize| dl.join("pack").join(names[i]);
        // What is physically in the download folder: every file with its size, how many bytes are non-zero, and
        // whether the name is there at all.
        let dump = |stage: &str| {
            println!("--- {stage} ---");
            for (i, n) in names.iter().enumerate() {
                match std::fs::read(on_disk(i)) {
                    Ok(b) => println!("  {n:<12} on disk, size {:>7}, non-zero bytes {:>7}, matches original: {}", b.len(), b.iter().filter(|&&x| x != 0).count(), b == body(i as u8 + 1)),
                    Err(_) => println!("  {n:<12} NOT on disk"),
                }
            }
        };
        for &i in &picked {
            assert_eq!(std::fs::read(on_disk(i)).unwrap(), body(i as u8 + 1), "{} content", names[i]);
        }
        // 2) the skipped files got nothing: no progress, and no data on disk
        let skipped_untouched = |dd: &TorrentDetails| {
            for i in [1usize, 3] {
                assert_eq!(dd.files[i].progress_bytes, 0, "{} must not download", names[i]);
                let p = on_disk(i);
                assert!(!p.exists() || std::fs::read(&p).unwrap().iter().all(|&b| b == 0), "{} has data on disk", p.display());
            }
        };
        skipped_untouched(&d);
        dump("1. after first download (picked: a.bin, c.bin)");
        let seeding_state = engine.details(&id).await.unwrap().stats.state;
        println!("after first download: state {seeding_state:?}, skipped on disk: {:?}", [1usize, 3].map(|i| on_disk(i).exists()));

        // Delete one picked file (c.bin), pause so the files are closed, and recheck.
        engine.pause(&id).await.unwrap();
        dump("2. before delete");
        std::fs::remove_file(on_disk(2)).unwrap();
        dump("3. after deleting c.bin, before recheck");
        engine.recheck(&id).await.unwrap();
        let d = wait("recheck sees c.bin missing", Box::new(|d| d.files.get(2).is_some_and(|f| f.progress_bytes == 0) && d.files.first().is_some_and(full))).await;
        dump("4. after recheck, paused");
        assert!(!on_disk(2).exists() || std::fs::metadata(on_disk(2)).unwrap().len() == 0 || d.files[2].progress_bytes == 0);
        assert!(d.files[0].selected && !d.files[1].selected && d.files[2].selected && !d.files[3].selected, "selection survives the recheck");

        // 3) resume: the deleted file comes back whole; 4) the skipped ones are still not downloaded
        engine.resume(&id).await.unwrap();
        // A recheck re-adds the torrent and, with no DHT or tracker on loopback, nothing tells it where a seeder is
        // (a real swarm finds peers again by itself), so a second seeder dials the downloader instead.
        let dl_port = engine.network_status().await.unwrap().listen_port.expect("downloader listens");
        let dialer_opts = SessionOptions { dht: None, disable_local_service_discovery: true, listen: Some(ListenerOptions { listen_addr: loopback, ..Default::default() }), ..Default::default() };
        let dialer = Session::new_with_opts(seed_dir.clone(), dialer_opts).await.unwrap();
        dialer
            .add_torrent(
                AddTorrent::from_bytes(torrent.clone()),
                Some(AddTorrentOptions { overwrite: true, initial_peers: Some(vec![SocketAddr::from(([127, 0, 0, 1], dl_port))]), ..Default::default() }),
            )
            .await
            .unwrap();
        let d = wait("c.bin downloaded again", Box::new(|d| d.files.get(2).is_some_and(full) && d.files.first().is_some_and(full))).await;
        assert_eq!(std::fs::read(on_disk(2)).unwrap(), body(3), "c.bin is back, byte for byte");
        assert_eq!(std::fs::read(on_disk(0)).unwrap(), body(1));
        skipped_untouched(&d);
        assert!(d.files[0].selected && !d.files[1].selected && d.files[2].selected && !d.files[3].selected);
        dump("5. after resume and re-download");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Skipped file sandwiched between two picked ones, with sizes that don't line up with the pieces: the pieces on
    /// the boundaries carry a little of the skipped file. It gets its full length but must not take that space on disk.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_skipped_file_next_to_a_picked_one_takes_almost_no_disk() {
        let root = scratch("edge");
        let (seed_dir, dl) = (root.join("seed"), root.join("dl"));
        let content = seed_dir.join("pack");
        std::fs::create_dir_all(&content).unwrap();
        std::fs::create_dir_all(&dl).unwrap();
        let sizes = [100_000usize, 3_000_000, 100_000];
        let names = ["a.bin", "b.bin", "c.bin"];
        for (i, (n, sz)) in names.iter().zip(sizes).enumerate() {
            std::fs::write(content.join(n), (0..sz).map(|k| ((k * 31) >> 3) as u8 ^ (i as u8 + 1)).collect::<Vec<u8>>()).unwrap();
        }
        let torrent = create_torrent(&content, CreateTorrentOptions { piece_length: Some(64 * 1024), ..Default::default() }, &BlockingSpawner::new(1)).await.unwrap().as_bytes().unwrap();
        let loopback: SocketAddr = ([127, 0, 0, 1], 0).into();
        let seeder = Session::new_with_opts(
            seed_dir.clone(),
            SessionOptions { dht: None, disable_local_service_discovery: true, listen: Some(ListenerOptions { listen_addr: loopback, ..Default::default() }), ..Default::default() },
        )
        .await
        .unwrap();
        seeder.add_torrent(AddTorrent::from_bytes(torrent.clone()), Some(AddTorrentOptions { overwrite: true, ..Default::default() })).await.unwrap();
        let port = seeder.listen_addr().expect("seeder listens").port();
        let engine = RqbitEngine::new(engine_cfg(&dl, None)).await.unwrap();
        let mut req = AddRequest::new(AddSource::FileBytes(torrent.to_vec()));
        req.peers = vec![SocketAddr::from(([127, 0, 0, 1], port))];
        req.only_files = Some(vec![0, 2]);
        let id = engine.add(req).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let d = engine.details(&id).await.unwrap();
            if d.files[0].progress_bytes == d.files[0].size && d.files[2].progress_bytes == d.files[2].size {
                break;
            }
            assert!(Instant::now() < deadline, "timed out");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let d = engine.details(&id).await.unwrap();
        for (i, n) in names.iter().enumerate() {
            let p = dl.join("pack").join(n);
            match std::fs::read(&p) {
                Ok(b) => println!("{n}: app progress {:>8}, on disk size {:>8} (torrent size {:>8}), non-zero bytes {:>8}", d.files[i].progress_bytes, b.len(), sizes[i], b.iter().filter(|&&x| x != 0).count()),
                Err(_) => println!("{n}: app progress {:>8}, NOT on disk", d.files[i].progress_bytes),
            }
        }
        // The skipped file holds only the two boundary pieces' worth of data (under 200 KB), and the disk knows it.
        let skipped = dl.join("pack").join("b.bin");
        assert!(d.files[1].progress_bytes < 200_000);
        #[cfg(windows)]
        {
            let used = crate::fsmove::sparse::allocated_bytes(&skipped);
            println!("b.bin allocated on disk: {used} of {}", sizes[1]);
            assert!(used < 1_000_000, "the skipped file takes {used} bytes of disk for {} bytes of data", d.files[1].progress_bytes);
        }
        assert!(std::fs::metadata(&skipped).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Moving a finished torrent: files change folder, the torrent is checked there and is seeding again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn relocate_moves_the_files_and_the_torrent_follows() {
        let root = scratch("relocate");
        let (seed_dir, dl, dest) = (root.join("seed"), root.join("dl"), root.join("done"));
        for d in [&seed_dir, &dl, &dest] {
            std::fs::create_dir_all(d).unwrap();
        }
        let sd = seeder(&seed_dir, None).await;
        let engine = RqbitEngine::new(engine_cfg(&dl, None)).await.unwrap();
        let id = download_from(&engine, &sd).await;
        assert_eq!(state_after(&engine, &id, 60).await.0, TorrentState::Seeding);

        engine.relocate(&id, &dest).await.unwrap();
        assert!(dest.join("seedme.bin").exists() && !dl.join("seedme.bin").exists());
        assert_eq!(std::fs::read(dest.join("seedme.bin")).unwrap(), payload());
        assert_eq!(state_after(&engine, &id, 30).await.0, TorrentState::Seeding, "checked at the new place and seeding");
        assert_eq!(Path::new(&engine.details(&id).await.unwrap().save_path), dest);

        // A file already in the way stops the move and the torrent stays where it was.
        let back = root.join("elsewhere");
        std::fs::create_dir_all(&back).unwrap();
        std::fs::write(back.join("seedme.bin"), b"someone else's").unwrap();
        assert!(engine.relocate(&id, &back).await.is_err());
        assert!(dest.join("seedme.bin").exists());
        assert_eq!(state_after(&engine, &id, 30).await.0, TorrentState::Seeding);
        assert_eq!(Path::new(&engine.details(&id).await.unwrap().save_path), dest);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn multi_file_torrents_get_a_folder_named_after_them_and_single_files_do_not() {
        let base = Path::new("E:\\hh");
        assert_eq!(torrent_folder(base, "Game v1.0", 5, false), PathBuf::from("E:\\hh\\Game v1.0"));
        assert_eq!(torrent_folder(base, "one.iso", 1, false), PathBuf::from("E:\\hh"));
        assert_eq!(torrent_folder(base, "one.iso", 1, true), PathBuf::from("E:\\hh\\one.iso"), "a single file gets a folder when asked");
        assert_eq!(torrent_folder(base, "a/b:c*", 2, false), PathBuf::from("E:\\hh\\a_b_c_"), "characters Windows forbids are replaced");
        assert_eq!(torrent_folder(base, "  ..  ", 2, false), PathBuf::from("E:\\hh\\torrent"));
    }

    /// With a save folder given (a category, or Settings), a multi-file torrent must not spill its folders into the
    /// save folder's root: its files go in `<save folder>/<torrent name>/`. Moving it keeps that shape.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_multi_file_torrent_lands_in_its_own_folder_and_moves_as_one() {
        let root = scratch("ownfolder");
        let (seed_dir, dl, shelf, dest) = (root.join("seed"), root.join("dl"), root.join("shelf"), root.join("done"));
        let content = seed_dir.join("album");
        for d in [&content, &dl, &shelf, &dest] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(content.join("a.bin"), payload()).unwrap();
        std::fs::write(content.join("b.bin"), payload().into_iter().rev().collect::<Vec<u8>>()).unwrap();
        let torrent = create_torrent(&content, CreateTorrentOptions::default(), &BlockingSpawner::new(1)).await.unwrap().as_bytes().unwrap();
        let loopback: SocketAddr = ([127, 0, 0, 1], 0).into();
        let seeder = Session::new_with_opts(
            seed_dir.clone(),
            SessionOptions { dht: None, disable_local_service_discovery: true, listen: Some(ListenerOptions { listen_addr: loopback, ..Default::default() }), ..Default::default() },
        )
        .await
        .unwrap();
        seeder.add_torrent(AddTorrent::from_bytes(torrent.clone()), Some(AddTorrentOptions { overwrite: true, ..Default::default() })).await.unwrap();
        let seeder_addr = SocketAddr::from(([127, 0, 0, 1], seeder.listen_addr().expect("seeder listens").port()));

        let engine = RqbitEngine::new(engine_cfg(&dl, None)).await.unwrap();
        let mut req = AddRequest::new(AddSource::FileBytes(torrent.to_vec()));
        req.output_dir = Some(shelf.clone());
        req.peers = vec![seeder_addr];
        let id = engine.add(req).await.unwrap();
        assert_eq!(state_after(&engine, &id, 60).await.0, TorrentState::Seeding);
        assert!(shelf.join("album").join("a.bin").exists(), "files are inside the torrent's own folder");
        assert!(!shelf.join("a.bin").exists(), "nothing is spilled into the save folder itself");
        assert_eq!(Path::new(&engine.details(&id).await.unwrap().save_path), shelf.join("album"));

        engine.relocate(&id, &dest).await.unwrap();
        assert!(dest.join("album").join("a.bin").exists() && !shelf.join("album").join("a.bin").exists());
        assert!(!dest.join("a.bin").exists());
        assert_eq!(state_after(&engine, &id, 30).await.0, TorrentState::Seeding);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// With "folder for single files" on, one file lands in `<save folder>/<name>/` and moving keeps that; with it
    /// off (the default) it goes straight into the save folder.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_single_file_gets_its_own_folder_only_when_asked() {
        let root = scratch("singlefolder");
        let (seed_dir, dl, shelf, dest) = (root.join("seed"), root.join("dl"), root.join("shelf"), root.join("done"));
        for d in [&seed_dir, &dl, &shelf, &dest] {
            std::fs::create_dir_all(d).unwrap();
        }
        let file = seed_dir.join("one.bin");
        std::fs::write(&file, payload()).unwrap();
        let torrent = create_torrent(&file, CreateTorrentOptions::default(), &BlockingSpawner::new(1)).await.unwrap().as_bytes().unwrap();
        let loopback: SocketAddr = ([127, 0, 0, 1], 0).into();
        let seeder = Session::new_with_opts(
            seed_dir.clone(),
            SessionOptions { dht: None, disable_local_service_discovery: true, listen: Some(ListenerOptions { listen_addr: loopback, ..Default::default() }), ..Default::default() },
        )
        .await
        .unwrap();
        seeder.add_torrent(AddTorrent::from_bytes(torrent.clone()), Some(AddTorrentOptions { overwrite: true, ..Default::default() })).await.unwrap();
        let seeder_addr = SocketAddr::from(([127, 0, 0, 1], seeder.listen_addr().expect("seeder listens").port()));

        let engine = RqbitEngine::new(engine_cfg(&dl, None)).await.unwrap();
        let mut req = AddRequest::new(AddSource::FileBytes(torrent.to_vec()));
        req.output_dir = Some(shelf.clone());
        req.peers = vec![seeder_addr];
        req.folder_for_single = true;
        let id = engine.add(req).await.unwrap();
        assert_eq!(state_after(&engine, &id, 60).await.0, TorrentState::Seeding);
        assert!(shelf.join("one.bin").join("one.bin").exists(), "inside a folder named after the torrent");
        engine.relocate(&id, &dest).await.unwrap();
        assert!(dest.join("one.bin").join("one.bin").exists() && !dest.join("one.bin").is_file(), "moving keeps the folder");
        assert_eq!(state_after(&engine, &id, 30).await.0, TorrentState::Seeding);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn file_urls_survive_spaces_and_percent_signs() {
        let u = file_url(Path::new("C:\\Users\\a b\\100%\\list.txt"));
        assert_eq!(u, "file:///C:/Users/a%20b/100%25/list.txt");
        let parsed = reqwest::Url::parse(&u).unwrap();
        assert_eq!(parsed.to_file_path().unwrap(), PathBuf::from("C:\\Users\\a b\\100%\\list.txt"));
    }

    /// M5-05, outgoing: a peer inside a blocked range is never talked to. The unblocked control proves the
    /// seeder was reachable, so the block (and nothing else) is what stopped the transfer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_blocked_range_stops_outgoing_connections_and_an_unblocked_peer_works() {
        let root = scratch("ipf-out");
        let seed_dir = root.join("seed");
        std::fs::create_dir_all(&seed_dir).unwrap();
        let sd = seeder(&seed_dir, None).await;

        let list = root.join("block.txt");
        std::fs::write(&list, "x:127.0.0.1-127.0.0.1\n").unwrap();
        let blocked = RqbitEngine::new(engine_cfg(&root.join("dl-blocked"), Some(&list))).await.unwrap();
        let id = download_from(&blocked, &sd).await;
        let (state, progress) = state_after(&blocked, &id, 4).await;
        assert_ne!(state, TorrentState::Seeding);
        assert_eq!(progress, 0, "nothing may arrive from a blocked address");
        assert_eq!(blocked.details(&id).await.unwrap().peers.len(), 0);

        let open = RqbitEngine::new(engine_cfg(&root.join("dl-open"), None)).await.unwrap();
        let id = download_from(&open, &sd).await;
        assert_eq!(state_after(&open, &id, 30).await.0, TorrentState::Seeding, "control: the same seeder is reachable");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M5-05, incoming: the seeder's own filter refuses a connection that comes from a blocked address.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_blocked_range_refuses_incoming_connections() {
        let root = scratch("ipf-in");
        let seed_dir = root.join("seed");
        std::fs::create_dir_all(&seed_dir).unwrap();
        let list = root.join("block.txt");
        std::fs::write(&list, "x:127.0.0.0-127.255.255.255\n").unwrap();
        let sd = seeder(&seed_dir, Some(&list)).await;

        let dl = RqbitEngine::new(engine_cfg(&root.join("dl"), None)).await.unwrap();
        let id = download_from(&dl, &sd).await;
        let (state, progress) = state_after(&dl, &id, 4).await;
        assert_ne!(state, TorrentState::Seeding);
        assert_eq!(progress, 0, "the seeder must refuse a downloader inside its blocked range");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Changing network settings restarts the session; the torrent list and the speed limits come back,
    /// and a bad setting leaves a working engine behind.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn network_changes_rebuild_the_session_and_keep_torrents_and_limits() {
        let root = scratch("net");
        let seed_dir = root.join("seed");
        std::fs::create_dir_all(&seed_dir).unwrap();
        let sd = seeder(&seed_dir, None).await;
        let mut cfg = engine_cfg(&root.join("dl"), None);
        cfg.persistence_dir = Some(root.join("state"));
        let engine = RqbitEngine::new(cfg).await.unwrap();

        let mut req = AddRequest::new(AddSource::FileBytes(sd.torrent.clone()));
        req.paused = true;
        let id = engine.add(req).await.unwrap();
        engine.set_limits(Limits { down_bps: Some(500_000), up_bps: Some(100_000) }).await.unwrap();

        // a fixed port, found by binding and releasing one
        let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        let base = NetworkConfig { listen_port: Some(port), dht: false, ..Default::default() };
        engine.set_network(base.clone()).await.unwrap();
        assert_eq!(engine.network_status().await.unwrap().listen_port, Some(port), "the new port is in use");
        assert!(engine.list().await.unwrap().iter().any(|t| t.id == id), "the torrent survived the restart");
        assert_eq!(engine.session().ratelimits.get_download_bps().map(|n| n.get()), Some(500_000));
        assert_eq!(engine.session().ratelimits.get_upload_bps().map(|n| n.get()), Some(100_000));

        // same settings again: nothing is restarted
        let before = Arc::as_ptr(&engine.session());
        engine.set_network(base.clone()).await.unwrap();
        assert_eq!(before, Arc::as_ptr(&engine.session()), "unchanged settings must not rebuild");

        // a proxy turns incoming connections off (nothing may listen outside it)
        let proxied = NetworkConfig { proxy: Some("socks5://127.0.0.1:9".into()), ..base.clone() };
        engine.set_network(proxied).await.unwrap();
        assert_eq!(engine.network_status().await.unwrap().listen_port, None);
        assert!(engine.list().await.unwrap().iter().any(|t| t.id == id));

        // invalid input is rejected before anything restarts
        let bad = NetworkConfig { proxy: Some("http://nope".into()), ..base.clone() };
        assert!(matches!(engine.set_network(bad).await, Err(EngineError::InvalidInput(_))));

        // a list that can't be loaded fails the change but leaves the previous engine running
        let missing = NetworkConfig { blocklist_file: Some(root.join("does-not-exist.txt")), ..base.clone() };
        assert!(engine.set_network(missing).await.is_err());
        assert!(engine.list().await.unwrap().iter().any(|t| t.id == id), "the engine is still usable");
        assert_eq!(engine.session().ratelimits.get_download_bps().map(|n| n.get()), Some(500_000));
        engine.shutdown().await;
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
            ..Default::default()
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
            ..Default::default()
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
