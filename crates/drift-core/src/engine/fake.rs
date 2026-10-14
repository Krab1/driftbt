//! In-memory engine for tests of `core` logic. Progress only moves when a test calls `advance`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::{EngineError, Result, TorrentEngine};
use crate::model::*;

#[derive(Default)]
pub struct FakeEngine {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    torrents: BTreeMap<TorrentId, Entry>,
    limits: Limits,
}

struct Entry {
    stats: TorrentStats,
    files: Vec<FileDetails>,
}

const FAKE_TOTAL: u64 = 1000;

impl FakeEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test helper: add `bytes` of progress to a torrent that is downloading.
    pub fn advance(&self, id: &TorrentId, bytes: u64) {
        let mut g = self.inner.lock().unwrap();
        if let Some(e) = g.torrents.get_mut(id) {
            if e.stats.state == TorrentState::Downloading {
                e.stats.progress_bytes = (e.stats.progress_bytes + bytes).min(e.stats.total_bytes);
                if e.stats.progress_bytes == e.stats.total_bytes {
                    e.stats.state = TorrentState::Seeding;
                }
            }
        }
    }

    pub fn limits(&self) -> Limits {
        self.inner.lock().unwrap().limits
    }

    /// `n` varied fake torrents for UI development (`DRIFT_FAKE=n`). Deterministic.
    pub fn demo(n: usize) -> Self {
        let e = Self::new();
        {
            let mut g = e.inner.lock().unwrap();
            let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
            let mut next = || {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x
            };
            const NAMES: [&str; 8] = ["ubuntu-26.04-desktop-amd64.iso", "debian-13-netinst.iso", "archlinux-2026.10.01-x86_64.iso", "fedora-workstation-43.iso", "mint-22-cinnamon.iso", "opensuse-tumbleweed-dvd.iso", "alpine-standard-3.22.iso", "nixos-minimal-26.05.iso"];
            for i in 0..n {
                let id = TorrentId(format!("{:040x}", next() as u128 * 0x1_0000_0001 + i as u128));
                let total = (50 + next() % 6000) * 1_048_576;
                let roll = next() % 10;
                let (state, progress) = match roll {
                    0..=3 => (TorrentState::Downloading, next() % total),
                    4..=7 => (TorrentState::Seeding, total),
                    8 => (TorrentState::Paused, next() % total),
                    _ => (TorrentState::Error, next() % total),
                };
                let down = if state == TorrentState::Downloading { 200_000 + next() % 15_000_000 } else { 0 };
                let stats = TorrentStats {
                    id: id.clone(),
                    name: format!("{} ({})", NAMES[(next() % 8) as usize], i + 1),
                    state,
                    error: (state == TorrentState::Error).then(|| "No space left on the download drive.".to_string()),
                    total_bytes: total,
                    progress_bytes: progress,
                    uploaded_bytes: next() % total,
                    down_bps: down,
                    up_bps: if state == TorrentState::Seeding { next() % 3_000_000 } else { 0 },
                    peers_live: (next() % 60) as u32,
                    eta_secs: (down > 0).then(|| (total - progress) / down),
                };
                g.torrents.insert(id, Entry { stats, files: Vec::new() });
            }
        }
        e
    }

    /// Advance every downloading demo torrent by one second of its own speed.
    pub fn step(&self) {
        let mut g = self.inner.lock().unwrap();
        for e in g.torrents.values_mut() {
            let s = &mut e.stats;
            if s.state == TorrentState::Downloading {
                s.progress_bytes = (s.progress_bytes + s.down_bps).min(s.total_bytes);
                if s.progress_bytes == s.total_bytes {
                    s.state = TorrentState::Seeding;
                    s.down_bps = 0;
                    s.eta_secs = None;
                } else {
                    s.eta_secs = (s.total_bytes - s.progress_bytes).checked_div(s.down_bps);
                }
            }
        }
    }

    fn set_state(&self, id: &TorrentId, from: impl Fn(TorrentState) -> bool, to: TorrentState) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        let e = g.torrents.get_mut(id).ok_or(EngineError::NotFound)?;
        if from(e.stats.state) {
            e.stats.state = to;
        }
        Ok(())
    }
}

/// Accepts `magnet:?xt=urn:btih:<40 hex>` or a bare 40-char hex hash.
fn parse_hash(s: &str) -> Option<String> {
    let h = s.split("btih:").nth(1).unwrap_or(s);
    let h: String = h.chars().take(40).collect();
    (h.len() == 40 && h.chars().all(|c| c.is_ascii_hexdigit())).then(|| h.to_lowercase())
}

#[async_trait]
impl TorrentEngine for FakeEngine {
    async fn add(&self, req: AddRequest) -> Result<TorrentId> {
        let AddSource::Magnet(m) = &req.source else {
            return Err(EngineError::InvalidInput("fake engine only accepts magnets".into()));
        };
        let hash = parse_hash(m).ok_or_else(|| EngineError::InvalidInput("bad magnet".into()))?;
        let id = TorrentId(hash.clone());
        let mut g = self.inner.lock().unwrap();
        g.torrents.entry(id.clone()).or_insert_with(|| Entry {
            stats: TorrentStats {
                id: id.clone(),
                name: format!("fake-{}", &hash[..8]),
                state: if req.paused { TorrentState::Paused } else { TorrentState::Downloading },
                error: None,
                total_bytes: FAKE_TOTAL,
                progress_bytes: 0,
                uploaded_bytes: 0,
                down_bps: 0,
                up_bps: 0,
                peers_live: 0,
                eta_secs: None,
            },
            files: vec![FileDetails { index: 0, path: "file.bin".into(), size: FAKE_TOTAL, progress_bytes: 0, selected: true }],
        });
        Ok(id)
    }

    async fn list(&self) -> Result<Vec<TorrentStats>> {
        Ok(self.inner.lock().unwrap().torrents.values().map(|e| e.stats.clone()).collect())
    }

    async fn details(&self, id: &TorrentId) -> Result<TorrentDetails> {
        let g = self.inner.lock().unwrap();
        let e = g.torrents.get(id).ok_or(EngineError::NotFound)?;
        Ok(TorrentDetails { stats: e.stats.clone(), files: e.files.clone() })
    }

    async fn pause(&self, id: &TorrentId) -> Result<()> {
        self.set_state(id, |s| matches!(s, TorrentState::Downloading | TorrentState::Seeding), TorrentState::Paused)
    }

    async fn resume(&self, id: &TorrentId) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        let e = g.torrents.get_mut(id).ok_or(EngineError::NotFound)?;
        if e.stats.state == TorrentState::Paused {
            e.stats.state = if e.stats.progress_bytes == e.stats.total_bytes {
                TorrentState::Seeding
            } else {
                TorrentState::Downloading
            };
        }
        Ok(())
    }

    async fn remove(&self, id: &TorrentId, _delete_files: bool) -> Result<()> {
        self.inner.lock().unwrap().torrents.remove(id).map(|_| ()).ok_or(EngineError::NotFound)
    }

    async fn set_files(&self, id: &TorrentId, selected: Vec<usize>) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        let e = g.torrents.get_mut(id).ok_or(EngineError::NotFound)?;
        for f in &mut e.files {
            f.selected = selected.contains(&f.index);
        }
        Ok(())
    }

    async fn set_limits(&self, limits: Limits) -> Result<()> {
        self.inner.lock().unwrap().limits = limits;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    fn magnet() -> AddRequest {
        AddRequest::new(AddSource::Magnet(format!("magnet:?xt=urn:btih:{}", HASH.to_uppercase())))
    }

    #[tokio::test]
    async fn add_is_idempotent_and_id_is_lowercase_hash() {
        let e = FakeEngine::new();
        let a = e.add(magnet()).await.unwrap();
        let b = e.add(magnet()).await.unwrap();
        assert_eq!(a, b);
        assert_eq!(a.0, HASH);
        assert_eq!(e.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn lifecycle_download_pause_resume_seed_remove() {
        let e = FakeEngine::new();
        let id = e.add(magnet()).await.unwrap();
        e.advance(&id, 400);
        e.pause(&id).await.unwrap();
        e.advance(&id, 400); // ignored while paused
        assert_eq!(e.details(&id).await.unwrap().stats.progress_bytes, 400);
        e.resume(&id).await.unwrap();
        e.advance(&id, 10_000);
        assert_eq!(e.details(&id).await.unwrap().stats.state, TorrentState::Seeding);
        e.remove(&id, false).await.unwrap();
        assert!(matches!(e.details(&id).await, Err(EngineError::NotFound)));
    }

    #[tokio::test]
    async fn rejects_garbage_and_applies_limits_and_files() {
        let e = FakeEngine::new();
        assert!(matches!(
            e.add(AddRequest::new(AddSource::Magnet("nope".into()))).await,
            Err(EngineError::InvalidInput(_))
        ));
        let id = e.add(magnet()).await.unwrap();
        e.set_files(&id, vec![]).await.unwrap();
        assert!(!e.details(&id).await.unwrap().files[0].selected);
        let l = Limits { down_bps: Some(1), up_bps: None };
        e.set_limits(l).await.unwrap();
        assert_eq!(e.limits(), l);
    }
}
