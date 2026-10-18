//! Per-torrent bookkeeping the engine knows nothing about: category, tags, queue position, goals.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::jsonfile;
use crate::model::TorrentId;
use crate::rules::Goal;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TorrentMeta {
    pub category: Option<String>,
    pub tags: Vec<String>,
    /// Download-queue position; lower runs first.
    pub position: u64,
    /// Paused by the user on purpose: the queue never resumes it.
    pub user_paused: bool,
    /// Overrides the global seeding goal when set.
    pub goal: Option<Goal>,
    /// The goal already fired once; resuming the torrent afterwards is respected.
    pub goal_reached: bool,
    /// Seconds spent seeding while the app was running.
    pub seed_secs: u64,
    /// Unix seconds.
    pub added_at: u64,
    /// Unix seconds when it was seen to complete.
    pub finished_at: Option<u64>,
    /// This torrent's own speed limits in KiB/s; 0 = none.
    pub limit_down_kib: u32,
    pub limit_up_kib: u32,
    /// Bytes the torrent had on disk the last time it was running. A restart that finds much less holds it back.
    pub progress_seen: u64,
    /// The data went missing from disk while Drift was closed: the torrent is held paused until you choose.
    pub files_missing: bool,
    /// Set while the torrent is being added in the background.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingAdd>,
}

/// An add that is still waiting for the torrent's details (a magnet link or address). Saved, so a restart carries on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PendingAdd {
    pub source: String,
    pub only_files: Option<Vec<usize>>,
    pub paused: bool,
}

pub type Library = BTreeMap<TorrentId, TorrentMeta>;

pub trait MetaStore: Send + Sync {
    /// Never fails: a missing or unreadable file gives an empty library.
    fn load(&self) -> Library;
    fn save(&self, library: &Library) -> std::io::Result<()>;
}

pub struct JsonMetaStore {
    path: PathBuf,
}

impl JsonMetaStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl MetaStore for JsonMetaStore {
    fn load(&self) -> Library {
        let Ok(bytes) = std::fs::read(&self.path) else { return Library::new() };
        serde_json::from_slice(jsonfile::strip_bom(&bytes)).unwrap_or_else(|_| {
            // Keep the unreadable file for inspection instead of silently overwriting it.
            let _ = std::fs::rename(&self.path, self.path.with_extension("json.bad"));
            Library::new()
        })
    }

    fn save(&self, library: &Library) -> std::io::Result<()> {
        jsonfile::write_atomic(&self.path, &serde_json::to_vec_pretty(library).map_err(std::io::Error::other)?)
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemoryMetaStore(Mutex<Library>);

impl MetaStore for MemoryMetaStore {
    fn load(&self) -> Library {
        self.0.lock().unwrap().clone()
    }
    fn save(&self, library: &Library) -> std::io::Result<()> {
        *self.0.lock().unwrap() = library.clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("drift-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn round_trips_with_string_keys() {
        let dir = tmp("rt");
        let store = JsonMetaStore::new(dir.join("library.json"));
        let mut lib = Library::new();
        lib.insert(
            TorrentId("abc".into()),
            TorrentMeta { category: Some("Linux".into()), tags: vec!["iso".into()], position: 7, goal: Some(Goal { ratio: Some(2.0), minutes: None }), ..Default::default() },
        );
        store.save(&lib).unwrap();
        assert_eq!(store.load(), lib);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_fields_default_and_garbage_is_kept_aside() {
        let dir = tmp("bad");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.json");
        std::fs::write(&path, br#"{"abc":{"category":"X"}}"#).unwrap();
        let lib = JsonMetaStore::new(&path).load();
        assert_eq!(lib[&TorrentId("abc".into())].category.as_deref(), Some("X"));
        assert!(!lib[&TorrentId("abc".into())].user_paused);

        std::fs::write(&path, b"{ nope").unwrap();
        assert!(JsonMetaStore::new(&path).load().is_empty());
        assert!(path.with_extension("json.bad").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
