//! The engine contract. Owned by `core`; implemented by `rqbit` (real) and `fake` (tests).
//! No third-party engine types may appear here.

use async_trait::async_trait;

use crate::model::{AddRequest, Limits, TorrentDetails, TorrentId, TorrentStats};

pub mod fake;
pub mod rqbit;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("torrent not found")]
    NotFound,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("engine error: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, EngineError>;

#[async_trait]
pub trait TorrentEngine: Send + Sync {
    /// Adding a torrent that is already managed returns its existing id.
    async fn add(&self, req: AddRequest) -> Result<TorrentId>;
    /// Cheap; called every tick.
    async fn list(&self) -> Result<Vec<TorrentStats>>;
    async fn details(&self, id: &TorrentId) -> Result<TorrentDetails>;
    async fn pause(&self, id: &TorrentId) -> Result<()>;
    async fn resume(&self, id: &TorrentId) -> Result<()>;
    async fn remove(&self, id: &TorrentId, delete_files: bool) -> Result<()>;
    /// Replace the set of files to download.
    async fn set_files(&self, id: &TorrentId, selected: Vec<usize>) -> Result<()>;
    async fn set_limits(&self, limits: Limits) -> Result<()>;
}
