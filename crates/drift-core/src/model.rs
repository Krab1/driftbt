//! Plain data shared by the engine contract, core logic and (via serde) the UI.

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Stable torrent identity: the lowercase hex info-hash. Known as soon as a magnet is parsed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TorrentId(pub String);

#[derive(Debug, Clone)]
pub enum AddSource {
    /// `magnet:` link or a bare 40-char info-hash.
    Magnet(String),
    /// `.torrent` file contents.
    FileBytes(Vec<u8>),
    /// http(s) URL of a `.torrent` file.
    Url(String),
}

#[derive(Debug, Clone)]
pub struct AddRequest {
    pub source: AddSource,
    /// Overrides the engine's default download folder.
    pub output_dir: Option<PathBuf>,
    /// File indices to download; `None` means all.
    pub only_files: Option<Vec<usize>>,
    pub paused: bool,
    /// Extra peers to try immediately (also used by loopback tests).
    pub peers: Vec<SocketAddr>,
}

impl AddRequest {
    pub fn new(source: AddSource) -> Self {
        Self { source, output_dir: None, only_files: None, paused: false, peers: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TorrentState {
    /// Fetching metadata or checking existing files.
    Checking,
    Downloading,
    Seeding,
    Paused,
    /// Waiting for a free download slot. Set by the manager; engines never report it.
    Queued,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TorrentStats {
    pub id: TorrentId,
    pub name: String,
    pub state: TorrentState,
    pub error: Option<String>,
    pub total_bytes: u64,
    pub progress_bytes: u64,
    pub uploaded_bytes: u64,
    pub down_bps: u64,
    pub up_bps: u64,
    pub peers_live: u32,
    pub eta_secs: Option<u64>,
    /// Host of the first tracker, for per-tracker totals. `None` for trackerless torrents.
    #[serde(default)]
    pub tracker: Option<String>,
}

/// What the UI shows for one torrent: engine stats plus the manager's own bookkeeping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TorrentView {
    #[serde(flatten)]
    pub stats: TorrentStats,
    pub category: Option<String>,
    pub tags: Vec<String>,
    /// Place in the download queue (1 = first); `None` once complete.
    pub queue_pos: Option<u32>,
    /// Short reason shown next to the name, e.g. "Seeding goal reached".
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDetails {
    pub index: usize,
    pub path: String,
    pub size: u64,
    pub progress_bytes: u64,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub addr: String,
    pub client: Option<String>,
    /// "tcp", "utp" or "socks".
    pub transport: String,
    pub downloaded_bytes: u64,
    pub uploaded_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TorrentDetails {
    pub stats: TorrentStats,
    /// Empty until metadata is known (magnets).
    pub files: Vec<FileDetails>,
    /// Connected peers, busiest first. Empty while paused.
    pub peers: Vec<PeerInfo>,
    pub trackers: Vec<String>,
    /// Folder the torrent is saved in.
    pub save_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewFile {
    pub index: usize,
    pub path: String,
    pub size: u64,
}

/// What a torrent contains, read without starting it (for the Add dialog).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TorrentPreview {
    pub id: TorrentId,
    pub name: String,
    pub total_bytes: u64,
    pub files: Vec<PreviewFile>,
}

/// Bytes per second; `None` means unlimited.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub down_bps: Option<u32>,
    pub up_bps: Option<u32>,
}
