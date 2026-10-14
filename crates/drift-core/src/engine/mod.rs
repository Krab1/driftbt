//! The engine contract. Owned by `core`; implemented by `rqbit` (real) and `fake` (tests).
//! No third-party engine types may appear here.

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncSeek};

use crate::model::{AddRequest, AddSource, Limits, NetworkConfig, NetworkStatus, TorrentDetails, TorrentId, TorrentPreview, TorrentStats};

pub mod fake;
pub mod rqbit;

/// A seekable byte source for one file of a torrent. Reads wait for the pieces they need, which also
/// makes the engine fetch them next.
pub trait ReadSeek: AsyncRead + AsyncSeek + Unpin + Send {}
impl<T: AsyncRead + AsyncSeek + Unpin + Send> ReadSeek for T {}

pub struct OpenFile {
    pub reader: Box<dyn ReadSeek>,
    pub len: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("torrent not found")]
    NotFound,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// The VPN guard is holding transfers because the chosen adapter is not connected.
    #[error("blocked by the VPN guard")]
    Blocked,
    /// Metadata could not be fetched in time (no peers, bad magnet, offline).
    #[error("timed out fetching torrent info")]
    Timeout,
    #[error("engine error: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, EngineError>;

#[async_trait]
pub trait TorrentEngine: Send + Sync {
    /// Adding a torrent that is already managed returns its existing id.
    async fn add(&self, req: AddRequest) -> Result<TorrentId>;
    /// Read a torrent's name and file list without starting it. Magnets are resolved from peers and
    /// may return `Timeout`.
    async fn inspect(&self, source: AddSource) -> Result<TorrentPreview>;
    /// Opens one file for reading while it is still downloading (used for play-while-downloading).
    /// Fails with `Backend` while the torrent is not running yet; callers may retry briefly.
    async fn open_file(&self, id: &TorrentId, file: usize) -> Result<OpenFile>;
    /// Cheap; called every tick.
    async fn list(&self) -> Result<Vec<TorrentStats>>;
    async fn details(&self, id: &TorrentId) -> Result<TorrentDetails>;
    async fn pause(&self, id: &TorrentId) -> Result<()>;
    async fn resume(&self, id: &TorrentId) -> Result<()>;
    async fn remove(&self, id: &TorrentId, delete_files: bool) -> Result<()>;
    /// Replace the set of files to download.
    async fn set_files(&self, id: &TorrentId, selected: Vec<usize>) -> Result<()>;
    async fn set_limits(&self, limits: Limits) -> Result<()>;
    /// Applies networking settings. Anything that changed restarts the engine's session, which briefly
    /// interrupts transfers; unchanged settings are a no-op.
    async fn set_network(&self, cfg: NetworkConfig) -> Result<()>;
    async fn network_status(&self) -> Result<NetworkStatus>;
}
