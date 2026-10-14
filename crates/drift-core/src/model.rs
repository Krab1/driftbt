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
pub struct TorrentDetails {
    pub stats: TorrentStats,
    /// Empty until metadata is known (magnets).
    pub files: Vec<FileDetails>,
}

/// Bytes per second; `None` means unlimited.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub down_bps: Option<u32>,
    pub up_bps: Option<u32>,
}
