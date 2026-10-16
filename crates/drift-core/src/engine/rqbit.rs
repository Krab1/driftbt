//! Adapter over `librqbit` 9.0.1. This is the ONLY file allowed to use librqbit (CLAUDE.md rule 2).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::net::{Ipv6Addr, SocketAddr};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use librqbit::api::TorrentIdOrHash;
use librqbit::http_api_types::PeerStatsFilter;
use librqbit::limits::LimitsConfig;
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

/// What the storage of a session shares with the engine (Windows only; see `sparse_storage`).
#[cfg(windows)]
type Link = sparse_storage::SessionLink;
#[cfg(not(windows))]
type Link = ();

pub struct RqbitEngine {
    session: RwLock<Arc<Session>>,
    link: Mutex<Link>,
    cfg: Mutex<RqbitConfig>,
    limits: Mutex<Limits>,
    /// Serialises session rebuilds.
    rebuild: tokio::sync::Mutex<()>,
    /// Per-torrent speed limits in force, by info hash. librqbit only takes them when a torrent is added, so a change
    /// re-adds the torrent; a rebuilt session forgets them (they come back when the caller sets them again).
    torrent_limits: Mutex<HashMap<String, LimitsConfig>>,
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

/// librqbit keeps every file of a torrent open for writing for as long as the torrent exists, even when it is paused or
/// finished, and Windows will not start a program (or show its icon) while anything holds it open for writing. A
/// downloaded game's `.exe` then fails with "being used by another process". This is librqbit's file storage with that
/// fixed: files are read through read-only handles, and a file is opened for writing only while data is arriving for it
/// (the handle goes away a couple of seconds after the last write, and when the torrent is paused).
///
/// It also keeps a file you did not pick out of your folder. Such a file is only written where it shares a download
/// piece with a file you picked (a few hundred KB at its start and end). librqbit would create the whole file for that,
/// at full length, and its own attempt to make it sparse never takes effect on Windows. Here those bytes go into one small
/// hidden file in the torrent's folder (`.drift-skipped-…`), and the real file is only created when you pick it
/// (the bytes are then moved into it). A file that already exists is used as it is.
#[cfg(windows)]
mod sparse_storage {
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::fs::{File, OpenOptions};
    use std::os::windows::fs::{FileExt, MetadataExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
    use std::time::{Duration, Instant};

    use anyhow::Context;
    use librqbit::api::TorrentIdOrHash;
    use librqbit::dht::Id20;
    use librqbit::storage::filesystem::FilesystemStorageFactory;
    use librqbit::storage::{BoxStorageFactory, StorageFactory, TorrentStorage};
    use librqbit::{ManagedTorrentShared, Session, TorrentMetadata};

    /// How long a file stays open for writing after its last write.
    const WRITER_IDLE: Duration = Duration::from_secs(2);

    /// Where to find the session (and so each torrent's folder), plus what the storages of one session share: which
    /// torrents were checked for a clash, the hidden file of each torrent, and the storage of each torrent (so a change
    /// of picks reaches it). The session is built after the factory, so the link is filled in a moment later; torrents
    /// restored while the session is starting wait for it.
    #[derive(Clone, Default)]
    pub struct SessionLink(Arc<LinkInner>);

    #[derive(Default)]
    struct LinkInner {
        session: RwLock<Weak<Session>>,
        set: AtomicBool,
        /// Torrents whose files were already checked for a clash when first added.
        checked: Mutex<HashSet<Id20>>,
        /// One hidden file per torrent, shared by every storage the torrent goes through (a recheck makes a new one
        /// while the old one may still be finishing), so two of them never write to the same log.
        stores: Mutex<HashMap<Id20, Arc<Mutex<Option<Store>>>>>,
        files: Mutex<HashMap<Id20, Weak<Files>>>,
    }

    impl SessionLink {
        pub fn set(&self, session: &Arc<Session>) {
            *self.0.session.write().unwrap() = Arc::downgrade(session);
            self.0.set.store(true, Ordering::Release);
        }

        /// The session could not be built: whoever is waiting for it gives up.
        pub fn fail(&self) {
            self.0.set.store(true, Ordering::Release);
        }

        fn get(&self) -> anyhow::Result<Arc<Session>> {
            // A start with many torrents and a slow disk can take a while; reading before the session is known would
            // look like damaged files and throw away what the engine knows about finished pieces.
            for _ in 0..6000 {
                if self.0.set.load(Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            self.0.session.read().unwrap().upgrade().context("the session has shut down")
        }

        fn store_for(&self, hash: Id20) -> Arc<Mutex<Option<Store>>> {
            self.0.stores.lock().unwrap().entry(hash).or_default().clone()
        }

        /// A torrent is going away or moving: everything remembered about it (including where its hidden file is).
        pub fn forget(&self, hash: &Id20) {
            self.0.checked.lock().unwrap().remove(hash);
            self.0.stores.lock().unwrap().remove(hash);
            self.0.files.lock().unwrap().remove(hash);
        }

        /// The user changed which files of a torrent are picked. Called before the torrent itself is told, so the files
        /// that are now picked get their stored bytes written into the real file first.
        pub fn set_picks(&self, hash: &Id20, picked: &HashSet<usize>) {
            let files = self.0.files.lock().unwrap().get(hash).and_then(Weak::upgrade);
            if let Some(files) = files {
                files.set_picks(picked);
            }
        }
    }

    #[derive(Clone)]
    pub struct SparseFactory(pub SessionLink);

    impl StorageFactory for SparseFactory {
        type Storage = Box<dyn TorrentStorage>;

        fn create(&self, shared: &ManagedTorrentShared, metadata: &TorrentMetadata) -> anyhow::Result<Box<dyn TorrentStorage>> {
            let files = Arc::new(Files {
                hash: shared.info_hash,
                link: self.0.clone(),
                folder: OnceLock::new(),
                epoch: Instant::now(),
                store: self.0.store_for(shared.info_hash),
                tidied: AtomicBool::new(false),
                picked: Mutex::new(None),
                piece_len: u64::from(metadata.lengths().default_piece_length()),
                total: metadata.lengths().total_length(),
                slots: metadata
                    .file_infos
                    .iter()
                    .map(|f| Slot {
                        rel: f.relative_filename.clone(),
                        padding: f.attrs.padding,
                        start: f.offset_in_torrent,
                        len: f.len,
                        reader: RwLock::new(None),
                        writer: RwLock::new(None),
                        last_write: AtomicU64::new(0),
                    })
                    .collect(),
            });
            self.0.0.files.lock().unwrap().insert(shared.info_hash, Arc::downgrade(&files));
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                let weak = Arc::downgrade(&files);
                rt.spawn(async move {
                    let mut tick = tokio::time::interval(Duration::from_secs(1));
                    loop {
                        tick.tick().await;
                        let Some(files) = weak.upgrade() else { break };
                        files.close_idle_writers();
                    }
                });
            }
            Ok(Box::new(SparseStorage {
                files,
                // Only used to delete files and empty folders, which it knows the torrent's folder for.
                fs: Arc::new(FilesystemStorageFactory::default().create(shared, metadata)?),
                alive: Arc::new(AtomicBool::new(true)),
                // The check that runs when a torrent starts reads parts of files that were never downloaded.
                lenient: true,
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

    /// Where the bytes of the files you did not pick are kept: the torrent's folder, hidden.
    pub fn skipped_store(folder: &Path, hash: &Id20) -> PathBuf {
        folder.join(format!(".drift-skipped-{}", &hash.as_string()[..12]))
    }

    /// Whether a file on disk is the nearly empty stub an older Drift left for a skipped file: sparse, and holding no more
    /// than the two download pieces that are shared with its neighbours. A complete file that merely carries the sparse
    /// flag (from that older Drift, or from another client) is not one.
    fn looks_like_stub(len: u64, allocated: u64, piece_len: u64) -> bool {
        len > 0 && allocated <= 2 * (piece_len + (64 << 10)) && allocated < len / 2
    }

    fn read_exact_at(f: &File, mut buf: &mut [u8], mut at: u64) -> std::io::Result<()> {
        while !buf.is_empty() {
            let n = f.seek_read(buf, at)?;
            if n == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            at += n as u64;
            buf = &mut buf[n..];
        }
        Ok(())
    }

    fn write_all_at(f: &File, mut buf: &[u8], mut at: u64) -> std::io::Result<()> {
        while !buf.is_empty() {
            let n = f.seek_write(buf, at)?;
            if n == 0 {
                return Err(std::io::ErrorKind::WriteZero.into());
            }
            at += n as u64;
            buf = &buf[n..];
        }
        Ok(())
    }

    /// A log of (file, offset, bytes) records plus an in-memory map of which ranges of which file it holds. Every record
    /// starts with a signature, a record with the length `CLEAR` forgets a file, and a tail that doesn't parse (a torn
    /// write, garbage) is cut off when the log is opened. A log that can't be opened is an error, never "empty".
    struct Store {
        path: PathBuf,
        file: Option<File>,
        end: u64,
        /// (file, start) -> (end, position in the log of the byte at `start`); ranges of one file never overlap.
        map: BTreeMap<(usize, u64), (u64, u64)>,
    }

    const MAGIC: u32 = 0x4b53_5244; // "DRSK"
    const HEADER: u64 = 20;
    const CLEAR: u32 = u32::MAX;

    impl Store {
        /// `lens` are the lengths of the torrent's files: a record for a file or a range that doesn't exist is rejected.
        fn open(path: PathBuf, lens: &[u64]) -> anyhow::Result<Store> {
            let mut s = Store { path, file: None, end: 0, map: BTreeMap::new() };
            let f = match OpenOptions::new().read(true).write(true).open(&s.path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(s),
                Err(e) => return Err(anyhow::Error::from(e).context(format!("error opening {:?}", s.path))),
            };
            let len = f.metadata()?.len();
            let (mut pos, mut hdr) = (0u64, [0u8; HEADER as usize]);
            while pos + HEADER <= len {
                read_exact_at(&f, &mut hdr, pos)?;
                if u32::from_le_bytes(hdr[0..4].try_into().unwrap()) != MAGIC {
                    break;
                }
                let id = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
                let off = u64::from_le_bytes(hdr[8..16].try_into().unwrap());
                let n = u32::from_le_bytes(hdr[16..20].try_into().unwrap());
                if id >= lens.len() {
                    break;
                }
                if n == CLEAR {
                    s.map.retain(|(i, _), _| *i != id);
                    pos += HEADER;
                    continue;
                }
                let data = pos + HEADER;
                if n == 0 || off.saturating_add(u64::from(n)) > lens[id] || data + u64::from(n) > len {
                    break;
                }
                s.insert(id, off, off + u64::from(n), data);
                pos = data + u64::from(n);
            }
            if pos < len {
                f.set_len(pos)?;
            }
            s.end = pos;
            s.file = Some(f);
            Ok(s)
        }

        fn file(&mut self) -> anyhow::Result<&File> {
            if self.file.is_none() {
                if let Some(dir) = self.path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                const HIDDEN: u32 = 0x2;
                let f = OpenOptions::new().create(true).read(true).write(true).attributes(HIDDEN).open(&self.path).with_context(|| format!("error opening {:?}", self.path))?;
                self.end = f.metadata()?.len();
                self.file = Some(f);
            }
            Ok(self.file.as_ref().expect("just opened"))
        }

        /// Records [start, end) of `id` as living at log position `pos`, replacing whatever overlapped.
        fn insert(&mut self, id: usize, start: u64, end: u64, pos: u64) {
            let hit: Vec<(u64, u64, u64)> = self.map.range((id, 0)..=(id, u64::MAX)).filter(|(&(_, s), &(e, _))| s < end && e > start).map(|(&(_, s), &(e, p))| (s, e, p)).collect();
            for (s, e, p) in hit {
                self.map.remove(&(id, s));
                if s < start {
                    self.map.insert((id, s), (start, p));
                }
                if e > end {
                    self.map.insert((id, end), (e, p + (end - s)));
                }
            }
            self.map.insert((id, start), (end, pos));
        }

        fn has(&self, id: usize) -> bool {
            self.map.range((id, 0)..=(id, u64::MAX)).next().is_some()
        }

        fn write(&mut self, id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
            self.file()?;
            let at = self.end;
            let mut rec = Vec::with_capacity(HEADER as usize + buf.len());
            rec.extend_from_slice(&MAGIC.to_le_bytes());
            rec.extend_from_slice(&(id as u32).to_le_bytes());
            rec.extend_from_slice(&offset.to_le_bytes());
            rec.extend_from_slice(&(buf.len() as u32).to_le_bytes());
            rec.extend_from_slice(buf);
            write_all_at(self.file()?, &rec, at)?;
            self.end += rec.len() as u64;
            self.insert(id, offset, offset + buf.len() as u64, at + HEADER);
            Ok(())
        }

        /// Reads what is held. What is not held is an error, or with `lenient` zeros: librqbit gives up on a whole file
        /// after one failed read, and a part that was never stored should only fail the piece that needs it (its checksum
        /// won't match), as it would with a sparse file. Only the check that runs when a torrent starts is lenient.
        fn read(&self, id: usize, mut offset: u64, mut buf: &mut [u8], lenient: bool) -> anyhow::Result<()> {
            while !buf.is_empty() {
                let hit = self.map.range((id, 0)..=(id, offset)).next_back().filter(|(_, &(e, _))| e > offset);
                let n = match (hit, &self.file) {
                    (Some((&(_, s), &(e, p))), Some(f)) => {
                        let n = ((e - offset) as usize).min(buf.len());
                        read_exact_at(f, &mut buf[..n], p + (offset - s))?;
                        n
                    }
                    _ if lenient => {
                        let next = self.map.range((id, offset + 1)..=(id, u64::MAX)).next().map_or(u64::MAX, |(&(_, s), _)| s);
                        let n = ((next - offset).min(buf.len() as u64)) as usize;
                        buf[..n].fill(0);
                        n
                    }
                    _ => anyhow::bail!("that part of the file was not stored"),
                };
                offset += n as u64;
                buf = &mut buf[n..];
            }
            Ok(())
        }

        /// Everything held for `id`, as (offset, bytes).
        fn parts(&self, id: usize) -> anyhow::Result<Vec<(u64, Vec<u8>)>> {
            let f = self.file.as_ref().context("nothing is stored")?;
            self.map
                .range((id, 0)..=(id, u64::MAX))
                .map(|(&(_, s), &(e, p))| {
                    let mut data = vec![0u8; (e - s) as usize];
                    read_exact_at(f, &mut data, p)?;
                    Ok((s, data))
                })
                .collect()
        }

        /// Forgets a file (and says so in the log, so it stays forgotten). When nothing is left the log is emptied.
        fn clear(&mut self, id: usize) -> anyhow::Result<()> {
            self.map.retain(|(i, _), _| *i != id);
            if self.map.is_empty() {
                if let Some(f) = &self.file {
                    f.set_len(0)?;
                }
                self.end = 0;
                return Ok(());
            }
            let mut rec = Vec::with_capacity(HEADER as usize);
            rec.extend_from_slice(&MAGIC.to_le_bytes());
            rec.extend_from_slice(&(id as u32).to_le_bytes());
            rec.extend_from_slice(&0u64.to_le_bytes());
            rec.extend_from_slice(&CLEAR.to_le_bytes());
            let at = self.end;
            write_all_at(self.file()?, &rec, at)?;
            self.end += HEADER;
            Ok(())
        }
    }

    struct Slot {
        rel: PathBuf,
        padding: bool,
        /// Where the file starts in the torrent, and how long it is.
        start: u64,
        len: u64,
        /// Opened on first read and kept: a read-only handle does not stop the file from being run.
        reader: RwLock<Option<File>>,
        /// Only while data is being written.
        writer: RwLock<Option<File>>,
        /// Milliseconds after `Files::epoch` of the last write.
        last_write: AtomicU64,
    }

    struct Files {
        hash: Id20,
        link: SessionLink,
        folder: OnceLock<PathBuf>,
        epoch: Instant,
        slots: Vec<Slot>,
        piece_len: u64,
        total: u64,
        /// Opened when first needed; shared with the other storages of this torrent.
        store: Arc<Mutex<Option<Store>>>,
        /// The empty files librqbit's own check created for files that were not picked have been removed.
        tidied: AtomicBool,
        /// Which files are picked: `None` until asked of the torrent; inside, `None` means all of them. After the first
        /// time it is kept up to date by `set_picks`, so reading and writing never go back to the torrent for it (its
        /// own lock may be held by whoever is reading).
        picked: Mutex<Option<Option<HashSet<usize>>>>,
    }

    impl Files {
        fn slot(&self, id: usize) -> anyhow::Result<&Slot> {
            self.slots.get(id).context("no such file")
        }

        fn folder(&self) -> anyhow::Result<&Path> {
            if self.folder.get().is_none() {
                let session = self.link.get()?;
                let handle = session.get(TorrentIdOrHash::Hash(self.hash)).context("the torrent is not in the session")?;
                let _ = self.folder.set(handle.output_folder().to_path_buf());
            }
            Ok(self.folder.get().context("no folder")?)
        }

        fn path(&self, id: usize) -> anyhow::Result<PathBuf> {
            Ok(self.folder()?.join(&self.slot(id)?.rel))
        }

        /// Whether the file is not picked (so it is kept out of the folder). `false` while that is not known yet.
        fn unpicked(&self, id: usize) -> bool {
            let mut g = self.picked.lock().unwrap();
            if g.is_none() {
                let Ok(session) = self.link.get() else { return false };
                let Some(h) = session.get(TorrentIdOrHash::Hash(self.hash)) else { return false };
                *g = Some(h.only_files().map(|v| v.into_iter().collect()));
            }
            g.as_ref().and_then(|p| p.as_ref()).is_some_and(|p| !p.contains(&id))
        }

        /// The user changed the picks. Under the store's lock, so a store write in flight is finished first and the
        /// stored bytes of files that are picked now go into their real files before anything else touches them.
        fn set_picks(&self, picked: &HashSet<usize>) {
            let paths: Vec<(usize, PathBuf)> = (0..self.slots.len()).filter(|id| picked.contains(id)).filter_map(|id| self.path(id).ok().map(|p| (id, p))).collect();
            let r = self.with_store(|s| {
                *self.picked.lock().unwrap() = Some(Some(picked.clone()));
                for (id, path) in &paths {
                    if s.has(*id) {
                        self.materialize_into(s, *id, path)?;
                    }
                }
                Ok(())
            });
            if let Err(e) = r {
                eprintln!("couldn't move the stored bytes into the picked files: {e:#}");
                *self.picked.lock().unwrap() = Some(Some(picked.clone()));
            }
        }

        /// Whether a write of `len` bytes at `offset` of the file lies in a download piece that the file shares with
        /// another file: only those bytes are worth keeping for a file that was not picked.
        fn in_shared_piece(&self, id: usize, offset: u64, len: usize) -> bool {
            let Ok(slot) = self.slot(id) else { return false };
            if self.piece_len == 0 {
                return false;
            }
            let (first, last) = (slot.start + offset, slot.start + offset + len as u64 - 1);
            let piece_start = first / self.piece_len * self.piece_len;
            let piece_end = ((last / self.piece_len + 1) * self.piece_len).min(self.total);
            piece_start < slot.start || piece_end > slot.start + slot.len
        }

        /// librqbit's own check of the folder (run when a torrent is added) creates an empty file for every file of the
        /// torrent. Those for files that were not picked are removed, once, with the folders made for them: such a file
        /// only comes to exist when it is picked. A sparse stub that an older Drift left is converted.
        fn tidy(&self) {
            if self.tidied.load(Ordering::Acquire) {
                return;
            }
            let Ok(folder) = self.folder() else { return };
            self.unpicked(0);
            let picked = match self.picked.lock().unwrap().clone() {
                None => return,
                Some(p) => p,
            };
            self.tidied.store(true, Ordering::Release);
            let Some(picked) = picked else { return };
            for (id, slot) in self.slots.iter().enumerate().filter(|(id, s)| !s.padding && !picked.contains(id)) {
                let path = folder.join(&slot.rel);
                let Ok(meta) = std::fs::metadata(&path) else { continue };
                if meta.file_attributes() & 0x200 != 0 && looks_like_stub(meta.len(), crate::fsmove::sparse::allocated_bytes(&path), self.piece_len) {
                    let _ = self.migrate_stub(id, &path);
                } else if meta.len() == 0 && std::fs::remove_file(&path).is_ok() {
                    let mut dir = path.parent();
                    while let Some(d) = dir.filter(|d| *d != folder && d.starts_with(folder)) {
                        if std::fs::remove_dir(d).is_err() {
                            break;
                        }
                        dir = d.parent();
                    }
                }
            }
        }

        /// Moves what a sparse stub really holds into the store and deletes the stub. On any failure the store forgets
        /// it again and the stub stays, so the one place that has the file is the one that is used.
        fn migrate_stub(&self, id: usize, path: &Path) -> anyhow::Result<()> {
            let f = File::open(path)?;
            let len = f.metadata()?.len();
            let mut buf = vec![0u8; 64 << 10];
            let r = self.with_store(|s| {
                let copied: anyhow::Result<()> = (|| {
                    let mut pos = 0u64;
                    while pos < len {
                        let n = ((len - pos) as usize).min(buf.len());
                        read_exact_at(&f, &mut buf[..n], pos)?;
                        // Only from the first to the last byte that isn't zero: the rest reads back as zeros anyway.
                        if let Some(first) = buf[..n].iter().position(|&b| b != 0) {
                            let last = buf[..n].iter().rposition(|&b| b != 0).expect("there is a first");
                            s.write(id, pos + first as u64, &buf[first..=last])?;
                        }
                        pos += n as u64;
                    }
                    Ok(())
                })();
                if copied.is_err() {
                    let _ = s.clear(id);
                }
                copied
            });
            drop(f);
            r?;
            if std::fs::remove_file(path).is_err() {
                self.with_store(|s| s.clear(id))?;
            }
            Ok(())
        }

        /// The store, opened if it isn't yet. Never called with a slot lock held.
        fn with_store<R>(&self, f: impl FnOnce(&mut Store) -> anyhow::Result<R>) -> anyhow::Result<R> {
            let folder = self.folder()?.to_path_buf();
            let mut g = self.store.lock().unwrap();
            if g.is_none() {
                let lens: Vec<u64> = self.slots.iter().map(|s| s.len).collect();
                *g = Some(Store::open(skipped_store(&folder, &self.hash), &lens)?);
            }
            f(g.as_mut().expect("opened above"))
        }

        fn store_has(&self, id: usize) -> bool {
            self.with_store(|s| Ok(s.has(id))).unwrap_or(false)
        }

        /// A file that was not picked: its bytes go to the store. `false` if it is picked after all.
        fn store_write(&self, id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<bool> {
            self.with_store(|s| {
                // Looked at with the store held, so a pick that happens right now can't lose these bytes: picking moves
                // the stored bytes under the same lock.
                if !self.unpicked(id) {
                    return Ok(false);
                }
                s.write(id, offset, buf)?;
                Ok(true)
            })
        }

        /// A file that has been picked: what the store holds for it goes into the real file, which is made its full
        /// length (as a picked file always is), and the store forgets it.
        fn materialize_into(&self, s: &mut Store, id: usize, path: &Path) -> anyhow::Result<()> {
            let parts = s.parts(id)?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let f = OpenOptions::new().create(true).write(true).truncate(false).open(path).with_context(|| format!("error opening {path:?} for writing"))?;
            for (offset, data) in &parts {
                write_all_at(&f, data, *offset)?;
            }
            let len = self.slot(id)?.len;
            if f.metadata()?.len() < len {
                f.set_len(len)?;
            }
            drop(f);
            s.clear(id)
        }

        fn materialize(&self, id: usize) -> anyhow::Result<()> {
            let path = self.path(id)?;
            self.with_store(|s| if s.has(id) { self.materialize_into(s, id, &path) } else { Ok(()) })
        }

        fn read(&self, id: usize, mut offset: u64, mut buf: &mut [u8], lenient: bool) -> anyhow::Result<()> {
            let slot = self.slot(id)?;
            if slot.padding {
                buf.fill(0);
                return Ok(());
            }
            if buf.is_empty() {
                return Ok(());
            }
            self.tidy();
            // Held for a file that was not picked: answered from the store, all under its lock, so a pick can't slip in
            // between the look and the read.
            let served = self.with_store(|s| {
                if s.has(id) && self.unpicked(id) {
                    s.read(id, offset, &mut *buf, lenient)?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            })?;
            if served {
                return Ok(());
            }
            if self.store_has(id) {
                self.materialize(id)?;
            }
            loop {
                if let Some(f) = slot.reader.read().unwrap().as_ref() {
                    while !buf.is_empty() {
                        let n = f.seek_read(buf, offset)?;
                        if n == 0 {
                            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof").into());
                        }
                        offset += n as u64;
                        buf = &mut buf[n..];
                    }
                    return Ok(());
                }
                let path = self.path(id)?;
                let mut g = slot.reader.write().unwrap();
                if g.is_none() {
                    match OpenOptions::new().read(true).open(&path) {
                        Ok(f) => *g = Some(f),
                        // Not picked, so never created: nothing of it is held, which reads as zeros while the torrent
                        // is being checked (see Store::read).
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound && lenient && {
                            drop(g);
                            self.unpicked(id)
                        } =>
                        {
                            buf.fill(0);
                            return Ok(());
                        }
                        Err(e) => return Err(anyhow::Error::from(e).context(format!("error opening {path:?}"))),
                    }
                }
            }
        }

        fn write(&self, id: usize, mut offset: u64, mut buf: &[u8]) -> anyhow::Result<()> {
            let slot = self.slot(id)?;
            if slot.padding || buf.is_empty() {
                return Ok(());
            }
            self.tidy();
            loop {
                // The read lock is held for the whole write, so the idle closer (which needs the write lock) can't close it under us.
                if let Some(f) = slot.writer.read().unwrap().as_ref() {
                    slot.last_write.store(self.now(), Ordering::Relaxed);
                    while !buf.is_empty() {
                        let n = f.seek_write(buf, offset)?;
                        anyhow::ensure!(n > 0, "the disk accepted no data");
                        offset += n as u64;
                        buf = &buf[n..];
                    }
                    return Ok(());
                }
                // Looked up before the lock is taken: the torrent's own lock must never be waited for while holding ours.
                let path = self.path(id)?;
                // A file that was not picked is not created for the part it shares with its neighbours; and while the
                // store holds something for it, the store is the one place that has the file.
                if (!path.exists() || self.store_has(id)) && self.in_shared_piece(id, offset, buf.len()) && self.store_write(id, offset, buf)? {
                    return Ok(());
                }
                if self.store_has(id) {
                    self.materialize(id)?;
                }
                let mut g = slot.writer.write().unwrap();
                if g.is_none() {
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    let f = OpenOptions::new().create(true).write(true).truncate(false).open(&path).with_context(|| format!("error opening {path:?} for writing"))?;
                    slot.last_write.store(self.now(), Ordering::Relaxed);
                    *g = Some(f);
                }
            }
        }

        /// Milliseconds since the storage was made, plus one so that 0 can mean "nothing open".
        fn now(&self) -> u64 {
            self.epoch.elapsed().as_millis() as u64 + 1
        }

        fn close_idle_writers(&self) {
            let idle = WRITER_IDLE.as_millis() as u64;
            for s in &self.slots {
                let last = s.last_write.load(Ordering::Relaxed);
                if last == 0 || self.now().saturating_sub(last) < idle {
                    continue;
                }
                if let Ok(mut g) = s.writer.try_write() {
                    // Looked at again now that nobody can be writing.
                    if self.now().saturating_sub(s.last_write.load(Ordering::Relaxed)) >= idle {
                        *g = None;
                        s.last_write.store(0, Ordering::Relaxed);
                    }
                }
            }
        }

        fn close_all(&self) {
            for s in &self.slots {
                s.last_write.store(0, Ordering::Relaxed);
                *s.writer.write().unwrap() = None;
                *s.reader.write().unwrap() = None;
            }
        }
    }

    pub struct SparseStorage {
        files: Arc<Files>,
        fs: Arc<dyn TorrentStorage>,
        /// Cleared by `take()`: the old storage of a paused torrent must not write again (it could bring back a file that
        /// was deleted or moved in the meantime).
        alive: Arc<AtomicBool>,
        /// Reads of parts that were never stored give zeros instead of an error. Only for the storage that checks the
        /// torrent when it starts; a running torrent (uploading, streaming) must never answer with made-up bytes.
        lenient: bool,
    }

    impl SparseStorage {
        fn live(&self) -> anyhow::Result<&Files> {
            anyhow::ensure!(self.alive.load(Ordering::Acquire), "the torrent was stopped");
            Ok(&self.files)
        }
    }

    impl TorrentStorage for SparseStorage {
        fn init(&mut self, shared: &ManagedTorrentShared, metadata: &TorrentMetadata) -> anyhow::Result<()> {
            // The first time a torrent is added, librqbit's own storage decides whether files already in the folder are in
            // the way (they are refused unless the add allows overwriting); its handles are closed again at once. Not
            // repeated when the same torrent is started again after an error, which would be refused for its own files.
            let first = self.files.link.0.checked.lock().unwrap().insert(shared.info_hash);
            if first {
                let mut probe = FilesystemStorageFactory::default().create(shared, metadata)?;
                if let Err(e) = probe.init(shared, metadata) {
                    self.files.link.0.checked.lock().unwrap().remove(&shared.info_hash);
                    return Err(e);
                }
            }
            Ok(())
        }

        fn pread_exact(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
            self.live()?.read(file_id, offset, buf, self.lenient)
        }

        fn pwrite_all(&self, file_id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
            self.live()?.write(file_id, offset, buf)
        }

        fn remove_file(&self, file_id: usize, filename: &Path) -> anyhow::Result<()> {
            if let Ok(s) = self.files.slot(file_id) {
                *s.writer.write().unwrap() = None;
                *s.reader.write().unwrap() = None;
            }
            self.fs.remove_file(file_id, filename)
        }

        fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
            self.fs.remove_directory_if_empty(path)
        }

        fn ensure_file_length(&self, file_id: usize, length: u64) -> anyhow::Result<()> {
            let files = self.live()?;
            let path = files.path(file_id)?;
            if files.slot(file_id)?.padding {
                return Ok(());
            }
            files.materialize(file_id)?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // A handle for this alone, gone again at once.
            let f = OpenOptions::new().create(true).write(true).truncate(false).open(&path).with_context(|| format!("error opening {path:?} for writing"))?;
            Ok(f.set_len(length)?)
        }

        /// Pausing: nothing stays open. The files open again by themselves if the torrent is started again.
        fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
            self.alive.store(false, Ordering::Release);
            self.files.close_all();
            Ok(Box::new(SparseStorage { files: self.files.clone(), fs: self.fs.clone(), alive: Arc::new(AtomicBool::new(true)), lenient: false }))
        }
    }

    #[cfg(test)]
    mod store_tests {
        use super::*;

        fn bytes(from: u64, to: u64) -> Vec<u8> {
            (from..to).map(|k| (k * 7 + 3) as u8).collect()
        }

        fn tmp(name: &str) -> PathBuf {
            let d = std::env::temp_dir().join(format!("drift-store-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            d.join(".store")
        }

        const LENS: [u64; 8] = [1000; 8];

        #[test]
        fn what_is_written_reads_back_also_after_reopening_and_overwrites_win() {
            let path = tmp("rt");
            let mut st = Store::open(path.clone(), &LENS).unwrap();
            // File 3: two neighbouring chunks, then a rewrite over part of both. File 5: one piece.
            st.write(3, 100, &bytes(100, 160)).unwrap();
            st.write(3, 160, &bytes(160, 200)).unwrap();
            st.write(3, 140, &[9u8; 40]).unwrap();
            st.write(5, 0, &bytes(0, 30)).unwrap();
            let mut expect3 = bytes(100, 200);
            expect3[40..80].fill(9);
            let check = |st: &Store| {
                let mut b = vec![0u8; 100];
                st.read(3, 100, &mut b, false).unwrap();
                assert_eq!(b, expect3);
                let mut b = vec![0u8; 10];
                st.read(3, 150, &mut b, false).unwrap();
                assert_eq!(b, expect3[50..60]);
                let mut b = vec![0u8; 30];
                st.read(5, 0, &mut b, false).unwrap();
                assert_eq!(b, bytes(0, 30));
                assert!(st.read(3, 99, &mut [0u8; 2], false).is_err(), "a byte that was never stored is an error");
                assert!(st.read(4, 0, &mut [0u8; 1], false).is_err());
                let mut z = vec![1u8; 4];
                st.read(3, 98, &mut z, true).unwrap();
                assert_eq!(z, [0, 0, expect3[0], expect3[1]], "lenient: zeros before, data after");
                let mut z = vec![1u8; 3];
                st.read(4, 0, &mut z, true).unwrap();
                assert_eq!(z, [0, 0, 0]);
            };
            check(&st);
            drop(st);
            check(&Store::open(path.clone(), &LENS).unwrap());

            // Forgetting a file stays forgotten after a reopen; the other file is untouched.
            let mut st = Store::open(path.clone(), &LENS).unwrap();
            assert_eq!(st.parts(3).unwrap().iter().map(|(_, d)| d.len()).sum::<usize>(), 100);
            st.clear(3).unwrap();
            drop(st);
            let st = Store::open(path.clone(), &LENS).unwrap();
            assert!(!st.has(3) && st.has(5));

            // Forgetting the last file empties the log, and it can be written to again.
            let mut st = Store::open(path.clone(), &LENS).unwrap();
            st.clear(5).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
            st.write(7, 0, &bytes(0, 10)).unwrap();
            drop(st);
            let st = Store::open(path, &LENS).unwrap();
            let mut b = vec![0u8; 10];
            st.read(7, 0, &mut b, false).unwrap();
            assert_eq!(b, bytes(0, 10));
        }

        #[test]
        fn overlapping_writes_split_the_old_ranges_correctly() {
            let path = tmp("split");
            let mut st = Store::open(path, &LENS).unwrap();
            st.write(1, 0, &bytes(0, 100)).unwrap();
            st.write(1, 40, &[1u8; 20]).unwrap(); // inside: leaves a left and a right part
            st.write(1, 90, &[2u8; 20]).unwrap(); // across the end
            st.write(1, 0, &[3u8; 10]).unwrap(); // across the start
            let mut expect = bytes(0, 110);
            expect[..10].fill(3);
            expect[40..60].fill(1);
            expect[90..110].fill(2);
            let mut b = vec![0u8; 110];
            st.read(1, 0, &mut b, false).unwrap();
            assert_eq!(b, expect);
        }

        #[test]
        fn a_log_with_garbage_zeros_or_foreign_records_never_leaks_data_into_the_map() {
            let path = tmp("garbage");
            let mut st = Store::open(path.clone(), &LENS).unwrap();
            st.write(2, 10, &bytes(10, 50)).unwrap();
            st.write(6, 0, &bytes(0, 20)).unwrap();
            let good = st.end;
            drop(st);
            // A zero-filled tail (a power cut), then random bytes: both are cut off, the good records stay.
            for junk in [vec![0u8; 100], (0..300u32).map(|k| (k * 31 + 7) as u8).collect::<Vec<u8>>()] {
                use std::io::Write;
                std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(&junk).unwrap();
                let st = Store::open(path.clone(), &LENS).unwrap();
                assert!(st.has(2) && st.has(6) && !st.has(0) && !st.has(1));
                assert_eq!(st.end, good);
                assert_eq!(std::fs::metadata(&path).unwrap().len(), good, "the junk is cut off");
            }
            // The same log opened for a torrent with fewer files: records for files that don't exist are refused.
            let st = Store::open(path.clone(), &[1000, 1000, 1000]).unwrap();
            assert!(st.has(2) && !st.has(6));
        }

        #[test]
        fn a_log_that_cannot_be_opened_is_an_error_not_an_empty_log() {
            let path = tmp("locked");
            let mut st = Store::open(path.clone(), &LENS).unwrap();
            st.write(1, 0, &bytes(0, 10)).unwrap();
            drop(st);
            // Held with no sharing at all, like a scanner that insists on exclusive access.
            let held = OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
            assert!(Store::open(path.clone(), &LENS).is_err());
            drop(held);
            assert!(Store::open(path, &LENS).unwrap().has(1));
        }

        #[test]
        fn only_a_nearly_empty_sparse_file_counts_as_an_old_stub() {
            let piece = 256 << 10;
            assert!(looks_like_stub(641 << 20, 512 << 10, piece), "a 641 MB file holding one piece");
            assert!(!looks_like_stub(10 << 20, 10 << 20, piece), "a complete file that is merely flagged sparse");
            assert!(!looks_like_stub(50 << 20, 40 << 20, piece), "a mostly complete one");
            assert!(!looks_like_stub(0, 0, piece));
        }
    }
}

/// Moves a small file, across drives if need be.
#[cfg(windows)]
fn move_small_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

async fn build_session(cfg: &RqbitConfig) -> Result<(Arc<Session>, Link)> {
    let mut opts = SessionOptions::default();
    #[cfg(windows)]
    let link = sparse_storage::SessionLink::default();
    #[cfg(not(windows))]
    let link: Link = ();
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
    let session = match Session::new_with_opts(cfg.output_dir.clone(), opts).await {
        Ok(s) => s,
        Err(e) => {
            #[cfg(windows)]
            link.fail();
            return Err(backend(e));
        }
    };
    #[cfg(windows)]
    {
        link.set(&session);
    }
    Ok((session, link))
}

impl RqbitEngine {
    pub async fn new(cfg: RqbitConfig) -> Result<Self> {
        let (session, link) = build_session(&cfg).await?;
        Ok(Self {
            session: RwLock::new(session),
            link: Mutex::new(link),
            cfg: Mutex::new(cfg),
            limits: Mutex::new(Limits::default()),
            rebuild: tokio::sync::Mutex::new(()),
            torrent_limits: Mutex::new(HashMap::new()),
            geo: crate::geoip::GeoIp::embedded(),
        })
    }

    /// A list-only add: what the torrent holds, plus the torrent itself and the peers that answered.
    async fn look(&self, source: AddSource) -> Result<(TorrentPreview, Vec<u8>, Vec<SocketAddr>)> {
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
        let preview = TorrentPreview {
            name: l.info.name().map(|n| n.into_owned()).unwrap_or_else(|| id.0.clone()),
            total_bytes: files.iter().map(|f| f.size).sum(),
            id,
            files,
        };
        Ok((preview, l.torrent_bytes.to_vec(), l.seen_peers))
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
            return Err(EngineError::Backend("The torrent's details haven't arrived yet. Try again in a moment.".into()));
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
            ratelimits: self.torrent_limits.lock().unwrap().get(&s.hash.as_string()).copied().unwrap_or_default(),
            ..Default::default()
        };
        self.session().add_torrent(AddTorrent::from_bytes(s.bytes), Some(opts)).await.map_err(backend)?;
        Ok(())
    }

    /// Where librqbit keeps which pieces a torrent already has, when the session is saved to disk.
    fn resume_file(&self, hash: &librqbit::dht::Id20) -> Option<PathBuf> {
        self.cfg.lock().unwrap().persistence_dir.as_ref().map(|d| d.join(format!("{}.bitv", hash.as_string())))
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
        // For a magnet or an address that look has fetched the whole torrent, so add from it rather than fetching it twice.
        let mut source = req.source.clone();
        let mut peers = req.peers.clone();
        // A magnet's own file choice (`so=`) is only read from the magnet, which the code below replaces.
        let only_files = req.only_files.clone().or_else(|| match &req.source {
            AddSource::Magnet(m) => librqbit::Magnet::parse(m).ok().and_then(|m| m.get_select_only()),
            _ => None,
        });
        let output_folder = match &req.output_dir {
            None => None,
            Some(base) => Some(match self.look(req.source.clone()).await {
                Ok((p, bytes, seen)) => {
                    if !matches!(source, AddSource::FileBytes(_)) {
                        source = AddSource::FileBytes(bytes);
                        peers.extend(seen);
                    }
                    torrent_folder(base, &p.name, p.files.len(), req.folder_for_single)
                }
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
                only_files: only_files.clone(),
                output_folder: output_folder.clone().map(|p| p.to_string_lossy().into_owned()),
                initial_peers: (!peers.is_empty()).then(|| peers.clone()),
                ..Default::default()
            };
            match self.session().add_torrent(to_add(source.clone()), Some(opts)).await {
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
        self.look(source).await.map(|(preview, _, _)| preview)
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
        let (hash, folder) = (h.info_hash(), h.output_folder().to_path_buf());
        self.session().delete(TorrentIdOrHash::Hash(hash), delete_files).await.map_err(backend)?;
        #[cfg(windows)]
        self.link.lock().unwrap().forget(&hash);
        // The hidden file that held the bytes of the files you didn't pick goes with the rest, and so does the folder if
        // that leaves it empty (it was what kept the folder from being removed).
        #[cfg(windows)]
        if delete_files && std::fs::remove_file(sparse_storage::skipped_store(&folder, &hash)).is_ok() {
            let _ = std::fs::remove_dir(&folder);
        }
        #[cfg(not(windows))]
        let _ = (hash, folder);
        Ok(())
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
        // The torrent's hidden file is about to move, so the storage must not keep the old place in mind.
        #[cfg(windows)]
        self.link.lock().unwrap().forget(&saved.hash);
        // The hidden file with the bytes of the files you didn't pick goes first, so the old folder can be tidied up.
        #[cfg(windows)]
        let store = (sparse_storage::skipped_store(&saved.output, &saved.hash), sparse_storage::skipped_store(dest, &saved.hash));
        #[cfg(windows)]
        let had_store = store.0.exists() && std::fs::create_dir_all(dest).is_ok() && move_small_file(&store.0, &store.1).is_ok();
        if let Err(e) = crate::fsmove::move_files(&saved.output, dest, &saved.files) {
            // Nothing moved: carry on where it was.
            #[cfg(windows)]
            if had_store {
                let _ = move_small_file(&store.1, &store.0);
            }
            let _ = self.add_saved(saved, None).await;
            return Err(EngineError::Backend(e.to_string()));
        }
        self.add_saved(saved, Some(dest.to_path_buf())).await
    }

    async fn set_files(&self, id: &TorrentId, selected: Vec<usize>) -> Result<()> {
        let h = self.handle(id)?;
        let set: HashSet<usize> = selected.into_iter().collect();
        // The storage hears first: files that are picked now get the bytes it kept for them written into the real file
        // before the torrent starts to want them.
        #[cfg(windows)]
        self.link.lock().unwrap().set_picks(&h.info_hash(), &set);
        let r = self.session().update_only_files(&h, &set).await.map_err(backend);
        #[cfg(windows)]
        if r.is_err() {
            if let Some(now) = h.only_files() {
                self.link.lock().unwrap().set_picks(&h.info_hash(), &now.into_iter().collect());
            }
        }
        r
    }

    async fn set_torrent_limits(&self, id: &TorrentId, limits: Limits) -> Result<()> {
        let want = LimitsConfig { download_bps: limits.down_bps.and_then(NonZeroU32::new), upload_bps: limits.up_bps.and_then(NonZeroU32::new) };
        // One change at a time, and never while the session is being rebuilt: the torrent would be added to a dead one.
        let _one_at_a_time = self.rebuild.lock().await;
        if self.torrent_limits.lock().unwrap().get(&id.0).copied().unwrap_or_default() == want {
            return Ok(());
        }
        let saved = self.saved(id)?;
        {
            let mut map = self.torrent_limits.lock().unwrap();
            if want == LimitsConfig::default() {
                map.remove(&id.0);
            } else {
                map.insert(id.0.clone(), want);
            }
        }
        // Taken when the torrent is added, so add it again: same files, same state. Deleting a torrent also deletes its
        // resume data, and without it the whole torrent would be hashed again, so put that back before adding.
        let resume = self.resume_file(&saved.hash).and_then(|p| std::fs::read(&p).ok().map(|bytes| (p, bytes)));
        self.session().delete(TorrentIdOrHash::Hash(saved.hash), false).await.map_err(backend)?;
        if let Some((path, bytes)) = resume {
            let _ = std::fs::write(path, bytes);
        }
        self.add_saved(saved, None).await
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
        let (session, link) = match build_session(&new_cfg).await {
            Ok(s) => s,
            Err(e) => {
                // Put the previous settings back so the app isn't left without an engine.
                let (restored, link) = build_session(&old_cfg).await?;
                Self::apply_limits(&restored, *self.limits.lock().unwrap());
                *self.session.write().unwrap() = restored;
                *self.link.lock().unwrap() = link;
                self.torrent_limits.lock().unwrap().clear();
                return Err(e);
            }
        };
        Self::apply_limits(&session, *self.limits.lock().unwrap());
        *self.session.write().unwrap() = session;
        *self.link.lock().unwrap() = link;
        *self.cfg.lock().unwrap() = new_cfg;
        self.torrent_limits.lock().unwrap().clear();
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
    #[cfg(windows)]
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
        // The skipped file is not in the folder at all: the few hundred KB it shares with its neighbours are in one small
        // hidden file, and picking the file later makes it, whole and byte for byte.
        let skipped = dl.join("pack").join("b.bin");
        assert!(d.files[1].progress_bytes < 200_000);
        assert!(!skipped.exists(), "the skipped file must not exist");
        let hash = librqbit::dht::Id20::from_str(&id.0).unwrap();
        #[cfg(windows)]
        {
            let store = dl.join("pack").join(format!(".drift-skipped-{}", &hash.as_string()[..12]));
            let len = std::fs::metadata(&store).expect("the edge bytes are kept in a hidden file").len();
            println!("hidden file: {len} bytes for {} bytes of data", d.files[1].progress_bytes);
            assert!(len < 200_000 && len >= d.files[1].progress_bytes, "{len}");
            use std::os::windows::fs::MetadataExt;
            assert!(std::fs::metadata(&store).unwrap().file_attributes() & 0x2 != 0, "and it is hidden");
        }
        let _ = hash;

        // After a check (what a restart does) the picked files still verify with no seeder around: the pieces they share
        // with the skipped file are read back from the hidden file.
        engine.recheck(&id).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(d) = engine.details(&id).await {
                if d.files.len() == 3 && d.files[0].progress_bytes == d.files[0].size && d.files[2].progress_bytes == d.files[2].size {
                    break;
                }
            }
            assert!(Instant::now() < deadline, "the picked files did not verify from the hidden file");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(!skipped.exists(), "still no skipped file after the check");

        // A running torrent answers for the pieces it shares with the skipped file from the hidden file, strictly: another
        // client that wants only a.bin (whose last piece includes the start of b.bin) gets it, byte for byte.
        {
            let leech_dir = root.join("leech");
            std::fs::create_dir_all(&leech_dir).unwrap();
            let our_port = engine.network_status().await.unwrap().listen_port.expect("downloader listens");
            let leech = Session::new_with_opts(leech_dir.clone(), SessionOptions { dht: None, disable_local_service_discovery: true, listen: Some(ListenerOptions { listen_addr: loopback, ..Default::default() }), ..Default::default() }).await.unwrap();
            let opts = AddTorrentOptions { overwrite: true, only_files: Some(vec![0]), initial_peers: Some(vec![SocketAddr::from(([127, 0, 0, 1], our_port))]), ..Default::default() };
            leech.add_torrent(AddTorrent::from_bytes(torrent.clone()), Some(opts)).await.unwrap();
            let a_data: Vec<u8> = (0..sizes[0]).map(|k| ((k * 31) >> 3) as u8 ^ 1).collect();
            let got = leech_dir.join("pack").join("a.bin");
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if std::fs::read(&got).is_ok_and(|b| b == a_data) {
                    break;
                }
                assert!(Instant::now() < deadline, "the other client never got a.bin from us");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            leech.stop().await;
        }

        engine.set_files(&id, vec![0, 1, 2]).await.unwrap();
        // The check forgot the peers; a second seeder dials in so the rest of the file can come.
        let dl_port = engine.network_status().await.unwrap().listen_port.expect("downloader listens");
        let dialer = Session::new_with_opts(seed_dir.clone(), SessionOptions { dht: None, disable_local_service_discovery: true, listen: Some(ListenerOptions { listen_addr: loopback, ..Default::default() }), ..Default::default() }).await.unwrap();
        dialer
            .add_torrent(AddTorrent::from_bytes(torrent.clone()), Some(AddTorrentOptions { overwrite: true, initial_peers: Some(vec![SocketAddr::from(([127, 0, 0, 1], dl_port))]), ..Default::default() }))
            .await
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let d = engine.details(&id).await.unwrap();
            if d.files.iter().all(|f| f.progress_bytes == f.size) {
                break;
            }
            assert!(Instant::now() < deadline, "the picked file never completed");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        for (i, n) in names.iter().enumerate() {
            let expect: Vec<u8> = (0..sizes[i]).map(|k| ((k * 31) >> 3) as u8 ^ (i as u8 + 1)).collect();
            assert_eq!(std::fs::read(dl.join("pack").join(n)).unwrap(), expect, "{n} is byte for byte what was shared");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Downloads a 768 KiB torrent from a seeder over loopback with the given overall and per-torrent limits (bytes
    /// per second) and says how long the transfer took.
    async fn timed_download(name: &str, overall: Option<u32>, own: Option<u32>) -> Duration {
        let root = scratch(name);
        let (seed_dir, dl) = (root.join("seed"), root.join("dl"));
        std::fs::create_dir_all(&seed_dir).unwrap();
        std::fs::create_dir_all(&dl).unwrap();
        let file = seed_dir.join("limited.bin");
        std::fs::write(&file, payload().into_iter().take(768 * 1024).collect::<Vec<u8>>()).unwrap();
        let torrent = create_torrent(&file, CreateTorrentOptions { piece_length: Some(64 * 1024), ..Default::default() }, &BlockingSpawner::new(1)).await.unwrap().as_bytes().unwrap();
        let loopback: SocketAddr = ([127, 0, 0, 1], 0).into();
        let seeder_opts = |l: SocketAddr| SessionOptions { dht: None, disable_local_service_discovery: true, listen: Some(ListenerOptions { listen_addr: l, ..Default::default() }), ..Default::default() };
        let seeder = Session::new_with_opts(seed_dir.clone(), seeder_opts(loopback)).await.unwrap();
        seeder.add_torrent(AddTorrent::from_bytes(torrent.clone()), Some(AddTorrentOptions { overwrite: true, ..Default::default() })).await.unwrap();

        let engine = RqbitEngine::new(engine_cfg(&dl, None)).await.unwrap();
        engine.set_limits(Limits { down_bps: overall, up_bps: None }).await.unwrap();
        let mut req = AddRequest::new(AddSource::FileBytes(torrent.to_vec()));
        req.paused = true;
        let id = engine.add(req).await.unwrap();
        if own.is_some() {
            engine.set_torrent_limits(&id, Limits { down_bps: own, up_bps: None }).await.unwrap();
        }
        // The torrent was added again to take its limit, which also forgets peers; a second seeder dials in instead.
        let port = engine.network_status().await.unwrap().listen_port.expect("downloader listens");
        let dialer = Session::new_with_opts(seed_dir.clone(), seeder_opts(loopback)).await.unwrap();
        dialer
            .add_torrent(AddTorrent::from_bytes(torrent.clone()), Some(AddTorrentOptions { overwrite: true, initial_peers: Some(vec![SocketAddr::from(([127, 0, 0, 1], port))]), ..Default::default() }))
            .await
            .unwrap();
        let started = Instant::now();
        engine.resume(&id).await.unwrap();
        assert_eq!(state_after(&engine, &id, 90).await.0, TorrentState::Seeding, "{name} never finished");
        let took = started.elapsed();
        let _ = std::fs::remove_dir_all(&root);
        took
    }

    /// A torrent's own limit slows that torrent, and the overall limit wins when it is the lower one: 20 MB/s for the
    /// torrent under a 128 KB/s overall limit still goes at 128 KB/s.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_torrent_limit_slows_it_and_the_overall_limit_still_wins_when_lower() {
        let free = timed_download("lim-free", None, None).await;
        let own = timed_download("lim-own", None, Some(128 * 1024)).await;
        let overall_wins = timed_download("lim-overall", Some(128 * 1024), Some(20 * 1024 * 1024)).await;
        println!("unlimited {free:?}, own 128 KB/s {own:?}, overall 128 KB/s with own 20 MB/s {overall_wins:?}");
        assert!(free < Duration::from_secs(3), "unlimited should be quick, took {free:?}");
        assert!(own >= Duration::from_millis(3500), "128 KB/s on the torrent should take about 5 s for 768 KiB, took {own:?}");
        assert!(overall_wins >= Duration::from_millis(3500), "the overall 128 KB/s must hold even though the torrent allows 20 MB/s, took {overall_wins:?}");
    }

    /// Unticking a file that already has data, and choosing to delete it: the file is gone from disk, the neighbours that
    /// shared a download piece with it are repaired by the check that follows, and the rest keeps its data.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn unticking_a_file_with_delete_removes_it_and_repairs_its_neighbours() {
        use crate::library::MemoryMetaStore;
        use crate::manager::Manager;
        let root = scratch("untick");
        let (seed_dir, dl) = (root.join("seed"), root.join("dl"));
        let content = seed_dir.join("pack");
        std::fs::create_dir_all(&content).unwrap();
        std::fs::create_dir_all(&dl).unwrap();
        let sizes = [100_000usize, 3_000_000, 100_000];
        let names = ["a.bin", "b.bin", "c.bin"];
        let data: Vec<Vec<u8>> = sizes.iter().enumerate().map(|(i, &sz)| (0..sz).map(|k| ((k * 31) >> 3) as u8 ^ (i as u8 + 1)).collect()).collect();
        for (n, d) in names.iter().zip(&data) {
            std::fs::write(content.join(n), d).unwrap();
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

        let engine = Arc::new(RqbitEngine::new(engine_cfg(&dl, None)).await.unwrap());
        let settings = Arc::new(Mutex::new(crate::settings::Settings::default()));
        let manager = Manager::new(engine.clone(), Arc::new(MemoryMetaStore::default()), settings);
        let mut req = AddRequest::new(AddSource::FileBytes(torrent.to_vec()));
        req.peers = vec![SocketAddr::from(([127, 0, 0, 1], port))];
        let id = engine.add(req).await.unwrap();
        assert_eq!(state_after(&engine, &id, 60).await.0, TorrentState::Seeding);
        let folder = dl.join("pack");
        assert!(folder.join("b.bin").exists());

        manager.set_files(&id, vec![0, 2], true).await.unwrap();
        // The check adds the torrent again and, with no tracker or DHT on loopback, nothing says where a seeder is
        // (a real swarm finds peers by itself), so a second seeder dials in.
        let dl_port = engine.network_status().await.unwrap().listen_port.expect("downloader listens");
        let dialer = Session::new_with_opts(seed_dir.clone(), SessionOptions { dht: None, disable_local_service_discovery: true, listen: Some(ListenerOptions { listen_addr: loopback, ..Default::default() }), ..Default::default() }).await.unwrap();
        dialer
            .add_torrent(AddTorrent::from_bytes(torrent.clone()), Some(AddTorrentOptions { overwrite: true, initial_peers: Some(vec![SocketAddr::from(([127, 0, 0, 1], dl_port))]), ..Default::default() }))
            .await
            .unwrap();
        // The check runs in the background; wait until the two kept files are whole again.
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Ok(d) = engine.details(&id).await {
                if d.files.len() == 3 && d.files[0].progress_bytes == d.files[0].size && d.files[2].progress_bytes == d.files[2].size && d.files[1].progress_bytes < d.files[1].size {
                    break;
                }
            }
            assert!(Instant::now() < deadline, "the kept files did not come back whole");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(std::fs::read(folder.join("a.bin")).unwrap(), data[0], "a.bin is intact");
        assert_eq!(std::fs::read(folder.join("c.bin")).unwrap(), data[2], "c.bin is intact");
        // The deleted file may be back as a few pieces that it shares with its neighbours, but not as the whole file.
        let left = std::fs::metadata(folder.join("b.bin")).map(|m| allocated(&folder.join("b.bin"), m.len())).unwrap_or(0);
        assert!(left < 600_000, "the deleted file did not come back ({left} bytes on disk)");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Bytes a file really takes on disk (a sparse file takes less than its length).
    #[cfg(windows)]
    fn allocated(p: &Path, _len: u64) -> u64 {
        crate::fsmove::sparse::allocated_bytes(p)
    }
    #[cfg(not(windows))]
    fn allocated(_p: &Path, len: u64) -> u64 {
        len
    }

    /// A sparse stub left by an older Drift for a file that was not picked: the first time the torrent is opened its few real
    /// bytes move to the hidden file and the stub is deleted, and the picked files around it still check out.
    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_old_sparse_stub_of_a_skipped_file_is_replaced_by_the_hidden_file() {
        use std::io::{Seek, SeekFrom, Write};
        let root = scratch("stub");
        let dl = root.join("dl");
        let folder = dl.join("pack");
        std::fs::create_dir_all(&folder).unwrap();
        let sizes = [100_000usize, 3_000_000, 100_000];
        let names = ["a.bin", "b.bin", "c.bin"];
        let data: Vec<Vec<u8>> = sizes.iter().enumerate().map(|(i, &sz)| (0..sz).map(|k| ((k * 31) >> 3) as u8 ^ (i as u8 + 1)).collect()).collect();
        // The torrent is made from a copy of the data elsewhere.
        let src = root.join("src").join("pack");
        std::fs::create_dir_all(&src).unwrap();
        for (n, d) in names.iter().zip(&data) {
            std::fs::write(src.join(n), d).unwrap();
        }
        let torrent = create_torrent(&src, CreateTorrentOptions { piece_length: Some(64 * 1024), ..Default::default() }, &BlockingSpawner::new(1)).await.unwrap().as_bytes().unwrap();
        // What an older Drift left: the picked files whole, and for the skipped one a sparse file at full length that holds
        // only the bytes that share a piece with them (its first 31072 and last 19808 bytes).
        std::fs::write(folder.join("a.bin"), &data[0]).unwrap();
        std::fs::write(folder.join("c.bin"), &data[2]).unwrap();
        {
            let mut f = std::fs::OpenOptions::new().create(true).truncate(true).write(true).open(folder.join("b.bin")).unwrap();
            crate::fsmove::sparse::mark_sparse(&f).unwrap();
            f.set_len(3_000_000).unwrap();
            f.write_all(&data[1][..31072]).unwrap();
            f.seek(SeekFrom::Start(2_980_192)).unwrap();
            f.write_all(&data[1][2_980_192..]).unwrap();
        }
        assert!(crate::fsmove::sparse::allocated_bytes(&folder.join("b.bin")) < 1_000_000);

        let engine = RqbitEngine::new(engine_cfg(&dl, None)).await.unwrap();
        let opts = AddTorrentOptions { overwrite: true, only_files: Some(vec![0, 2]), output_folder: Some(folder.to_string_lossy().into_owned()), ..Default::default() };
        let r = engine.session().add_torrent(AddTorrent::from_bytes(torrent.clone()), Some(opts)).await.unwrap();
        let AddTorrentResponse::Added(_, h) = r else { panic!("not added") };
        let id = id_of(&h);

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(d) = engine.details(&id).await {
                if d.files.len() == 3 && d.files[0].progress_bytes == d.files[0].size && d.files[2].progress_bytes == d.files[2].size {
                    break;
                }
            }
            assert!(Instant::now() < deadline, "the picked files did not check out from the stub's bytes");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(!folder.join("b.bin").exists(), "the stub is gone");
        let store = std::fs::read_dir(&folder).unwrap().filter_map(|e| e.ok()).find(|e| e.file_name().to_string_lossy().starts_with(".drift-skipped-")).expect("the hidden file");
        let store_len = std::fs::metadata(store.path()).unwrap().len();
        assert!(store_len > 40_000 && store_len < 200_000, "the hidden file is {store_len} bytes");

        // And it holds up across a check: nothing is read from the stub any more.
        engine.recheck(&id).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(d) = engine.details(&id).await {
                if d.files.len() == 3 && d.files[0].progress_bytes == d.files[0].size && d.files[2].progress_bytes == d.files[2].size {
                    break;
                }
            }
            assert!(Instant::now() < deadline, "the picked files did not verify after a check");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(!folder.join("b.bin").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Adding a torrent whose file is already in the folder is refused, and the file is left alone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn adding_a_torrent_onto_an_existing_file_is_refused_and_leaves_it_alone() {
        let root = scratch("clash");
        let (seed_dir, dl) = (root.join("seed"), root.join("dl"));
        for d in [&seed_dir, &dl] {
            std::fs::create_dir_all(d).unwrap();
        }
        let sd = seeder(&seed_dir, None).await;
        std::fs::write(dl.join("seedme.bin"), b"my own data").unwrap();
        let engine = RqbitEngine::new(engine_cfg(&dl, None)).await.unwrap();
        let mut req = AddRequest::new(AddSource::FileBytes(sd.torrent.clone()));
        req.peers = vec![sd.addr];
        assert!(engine.add(req).await.is_err());
        assert_eq!(std::fs::read(dl.join("seedme.bin")).unwrap(), b"my own data");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Windows will not start a program that something else has open for writing (its icon is lost too). Opening a file
    /// the way it does, allowing others only to read, fails while any handle with write access is open.
    #[cfg(windows)]
    fn can_be_run(p: &Path) -> bool {
        use std::os::windows::fs::OpenOptionsExt;
        const SHARE_READ_DELETE: u32 = 1 | 4;
        std::fs::OpenOptions::new().read(true).share_mode(SHARE_READ_DELETE).open(p).is_ok()
    }

    /// A downloaded file must not stay open for writing: not once the data is in, and not while the torrent is paused.
    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn downloaded_files_are_not_left_open_for_writing_so_they_can_be_run() {
        let root = scratch("runnable");
        let (seed_dir, dl) = (root.join("seed"), root.join("dl"));
        for d in [&seed_dir, &dl] {
            std::fs::create_dir_all(d).unwrap();
        }
        let sd = seeder(&seed_dir, None).await;
        let engine = RqbitEngine::new(engine_cfg(&dl, None)).await.unwrap();
        let id = download_from(&engine, &sd).await;
        assert_eq!(state_after(&engine, &id, 60).await.0, TorrentState::Seeding);
        let file = dl.join("seedme.bin");
        assert_eq!(std::fs::read(&file).unwrap(), payload(), "the data is all there");

        // While data is arriving the file is open for writing; a moment after the last write it is not.
        tokio::time::sleep(Duration::from_millis(3500)).await;
        assert!(can_be_run(&file), "a finished, seeding file is open only for reading");
        // The check itself works: a handle with write access makes it fail.
        let held = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
        assert!(!can_be_run(&file));
        drop(held);
        assert!(can_be_run(&file));

        // Paused: nothing is held at all, and starting again brings the torrent back without a new write handle.
        engine.pause(&id).await.unwrap();
        assert!(can_be_run(&file), "a paused torrent holds no write handle");
        engine.resume(&id).await.unwrap();
        assert_eq!(state_after(&engine, &id, 60).await.0, TorrentState::Seeding);
        tokio::time::sleep(Duration::from_millis(3500)).await;
        assert!(can_be_run(&file), "and a resumed one doesn't either");
        assert_eq!(std::fs::read(&file).unwrap(), payload());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Changing a torrent's limit adds it to the engine again, and that must not throw away what the engine knows about
    /// which pieces are done (deleting a torrent deletes that record, and without it the whole torrent is hashed again).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn changing_a_torrent_limit_keeps_the_record_of_finished_pieces() {
        let root = scratch("limit-resume");
        let (seed_dir, dl, state) = (root.join("seed"), root.join("dl"), root.join("state"));
        for d in [&seed_dir, &dl, &state] {
            std::fs::create_dir_all(d).unwrap();
        }
        let sd = seeder(&seed_dir, None).await;
        let engine = RqbitEngine::new(RqbitConfig { persistence_dir: Some(state.clone()), ..engine_cfg(&dl, None) }).await.unwrap();
        let id = download_from(&engine, &sd).await;
        assert_eq!(state_after(&engine, &id, 60).await.0, TorrentState::Seeding);
        let record = state.join(format!("{}.bitv", id.0));
        assert!(record.exists(), "the record of finished pieces is there before the change");

        engine.set_torrent_limits(&id, Limits { down_bps: Some(1 << 20), up_bps: Some(1 << 20) }).await.unwrap();
        assert!(record.exists(), "and still there after it");
        assert!(std::fs::metadata(&record).unwrap().len() > 0);
        assert_eq!(state_after(&engine, &id, 30).await.0, TorrentState::Seeding, "the torrent carries on seeding");
        // The same limits again are a no-op; clearing them works too.
        engine.set_torrent_limits(&id, Limits { down_bps: Some(1 << 20), up_bps: Some(1 << 20) }).await.unwrap();
        engine.set_torrent_limits(&id, Limits::default()).await.unwrap();
        assert!(record.exists());
        assert_eq!(state_after(&engine, &id, 30).await.0, TorrentState::Seeding);
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
