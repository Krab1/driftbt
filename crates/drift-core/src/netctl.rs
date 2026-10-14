//! Turns settings into engine networking, and keeps the IP filter lists fresh.
//!
//! Lists are downloaded and validated here (see `ipfilter_source`), merged into one normalised file, and
//! handed to the engine, which can only load a list when its session starts. So a changed list means a
//! session restart, which the engine does while keeping its torrents.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::engine::{Result as EngineResult, TorrentEngine};
use crate::ipfilter::{self, Ranges};
use crate::ipfilter_source::{self, Caps};
use crate::jsonfile;
use crate::model::NetworkConfig;
use crate::settings::{IpFilterSettings, IpList, Settings};

/// Downloaded lists older than this are fetched again.
const REFRESH_AFTER_SECS: u64 = 7 * 24 * 3600;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListState {
    pub ranges: usize,
    /// Unix seconds of the last successful load.
    pub updated_at: u64,
    /// Plain-language reason the last attempt failed; the previous good copy stays in use.
    pub error: Option<String>,
    /// The list came over plain http.
    pub insecure: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct StateFile {
    lists: BTreeMap<String, ListState>,
    active_hash: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FilterStatus {
    pub enabled: bool,
    /// Merged ranges currently handed to the engine.
    pub active_ranges: usize,
    pub lists: BTreeMap<String, ListState>,
}

pub struct IpFilterService {
    dir: PathBuf,
    state: Mutex<StateFile>,
}

/// Ids become file names, so only a safe alphabet is allowed.
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

fn is_url(source: &str) -> bool {
    let l = source.trim_start().to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

impl IpFilterService {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let state = std::fs::read(dir.join("state.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(jsonfile::strip_bom(&b)).ok())
            .unwrap_or_default();
        Self { dir, state: Mutex::new(state) }
    }

    fn list_path(&self, id: &str) -> PathBuf {
        self.dir.join("lists").join(format!("{id}.txt"))
    }

    fn active_path(&self) -> PathBuf {
        self.dir.join("active.txt")
    }

    fn save(&self, st: &StateFile) {
        let _ = serde_json::to_vec_pretty(st).map(|b| jsonfile::write_atomic(&self.dir.join("state.json"), &b));
    }

    /// The merged list the engine should load now, if the filter is on and has anything in it.
    pub fn active_file(&self, cfg: &IpFilterSettings) -> Option<PathBuf> {
        let p = self.active_path();
        (cfg.enabled && p.exists()).then_some(p)
    }

    pub fn status(&self, cfg: &IpFilterSettings) -> FilterStatus {
        let st = self.state.lock().unwrap();
        let active_ranges = if cfg.enabled { std::fs::read_to_string(self.active_path()).map(|t| ipfilter::parse(&t).ranges.len()).unwrap_or(0) } else { 0 };
        FilterStatus { enabled: cfg.enabled, active_ranges, lists: st.lists.clone() }
    }

    async fn load_one(&self, list: &IpList, force: bool, now: u64) {
        let path = self.list_path(&list.id);
        let due = {
            let st = self.state.lock().unwrap();
            let fresh = st.lists.get(&list.id).is_some_and(|s| s.updated_at > 0 && now.saturating_sub(s.updated_at) < REFRESH_AFTER_SECS);
            // Local files are cheap and may have been edited, so they are always re-read.
            force || !is_url(&list.source) || !fresh || !path.exists()
        };
        if !due {
            return;
        }
        let result = if is_url(&list.source) {
            ipfilter_source::fetch(list.source.trim(), Caps::default()).await.map(|f| (f.parsed, f.insecure))
        } else {
            ipfilter_source::from_file(Path::new(list.source.trim()), Caps::default()).map(|p| (p, false))
        };
        let mut st = self.state.lock().unwrap();
        let entry = st.lists.entry(list.id.clone()).or_default();
        match result {
            Ok((parsed, insecure)) => {
                let wrote = jsonfile::write_atomic(&path, parsed.ranges.to_list_text().as_bytes());
                *entry = match wrote {
                    Ok(()) => ListState { ranges: parsed.ranges.len(), updated_at: now, error: None, insecure },
                    Err(_) => ListState { error: Some("Couldn't save the list on this computer.".into()), ..entry.clone() },
                };
            }
            // The last good copy (if any) stays in use; only the status changes.
            Err(e) => entry.error = Some(e.to_string()),
        }
    }

    /// Updates enabled lists that are due (or all of them with `force`), merges the good copies and
    /// returns the engine file plus whether its content changed since the engine last loaded it.
    pub async fn refresh(&self, cfg: &IpFilterSettings, force: bool, now: u64) -> Refreshed {
        for list in cfg.lists.iter().filter(|l| l.enabled && safe_id(&l.id)) {
            self.load_one(list, force, now).await;
        }
        let mut all = Ranges::default();
        for list in cfg.lists.iter().filter(|l| l.enabled && safe_id(&l.id)) {
            if let Ok(text) = std::fs::read_to_string(self.list_path(&list.id)) {
                all = all.union(&ipfilter::parse(&text).ranges);
            }
        }
        let mut st = self.state.lock().unwrap();
        // Lists removed from the settings no longer need their state or cache.
        let keep: Vec<&str> = cfg.lists.iter().map(|l| l.id.as_str()).collect();
        let gone: Vec<String> = st.lists.keys().filter(|k| !keep.contains(&k.as_str())).cloned().collect();
        for id in gone {
            st.lists.remove(&id);
            if safe_id(&id) {
                let _ = std::fs::remove_file(self.list_path(&id));
            }
        }
        let (active_file, hash) = if cfg.enabled && !all.is_empty() {
            let text = all.to_list_text();
            let hash = fnv(&text);
            if st.active_hash != Some(hash) || !self.active_path().exists() {
                let _ = jsonfile::write_atomic(&self.active_path(), text.as_bytes());
            }
            (Some(self.active_path()), Some(hash))
        } else {
            let _ = std::fs::remove_file(self.active_path());
            (None, None)
        };
        let changed = st.active_hash != hash;
        st.active_hash = hash;
        self.save(&st);
        Refreshed { active_file, changed }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Refreshed {
    pub active_file: Option<PathBuf>,
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    /// Use what is on disk.
    No,
    /// Fetch lists that are older than a week.
    IfDue,
    /// Fetch everything now.
    Force,
}

pub fn network_config(s: &Settings, active_filter: Option<PathBuf>) -> NetworkConfig {
    NetworkConfig {
        listen_port: s.listen_port,
        upnp: s.upnp,
        dht: s.dht,
        proxy: s.proxy.clone().filter(|p| !p.trim().is_empty()),
        blocklist_file: active_filter,
    }
}

pub struct NetControl {
    engine: Arc<dyn TorrentEngine>,
    filter: IpFilterService,
}

impl NetControl {
    pub fn new(engine: Arc<dyn TorrentEngine>, filter_dir: impl Into<PathBuf>) -> Self {
        Self { engine, filter: IpFilterService::new(filter_dir) }
    }

    pub fn filter_status(&self, s: &Settings) -> FilterStatus {
        self.filter.status(&s.ip_filter)
    }

    /// Makes the engine match the settings. The engine restarts its session only if something it needs changed.
    pub async fn apply(&self, s: &Settings, refresh: Refresh, now: u64) -> EngineResult<FilterStatus> {
        let active = if s.ip_filter.enabled && refresh != Refresh::No {
            self.filter.refresh(&s.ip_filter, refresh == Refresh::Force, now).await.active_file
        } else {
            self.filter.active_file(&s.ip_filter)
        };
        self.engine.set_network(network_config(s, active)).await?;
        Ok(self.filter.status(&s.ip_filter))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::FakeEngine;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("drift-netctl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn list(id: &str, source: &Path) -> IpList {
        IpList { id: id.into(), name: id.into(), source: source.display().to_string(), enabled: true }
    }

    fn cfg(lists: Vec<IpList>) -> IpFilterSettings {
        IpFilterSettings { enabled: true, lists }
    }

    #[tokio::test]
    async fn merges_enabled_lists_into_one_normalised_file() {
        let d = dir("merge");
        std::fs::write(d.join("a.dat"), "001.002.003.000 - 001.002.003.255 , 000 , A\n").unwrap();
        std::fs::write(d.join("b.txt"), "Bee:5.5.5.0-5.5.5.255\n").unwrap();
        let svc = IpFilterService::new(d.join("f"));
        let c = cfg(vec![list("a", &d.join("a.dat")), list("b", &d.join("b.txt"))]);
        let r = svc.refresh(&c, false, 1000).await;
        let file = r.active_file.expect("active file");
        assert!(r.changed);
        let merged = ipfilter::parse(&std::fs::read_to_string(&file).unwrap()).ranges;
        assert!(merged.contains("1.2.3.9".parse().unwrap()) && merged.contains("5.5.5.5".parse().unwrap()));
        assert_eq!(svc.status(&c).active_ranges, 2);

        // unchanged lists: same file, nothing for the engine to reload
        assert!(!svc.refresh(&c, false, 2000).await.changed);
        let _ = std::fs::remove_dir_all(d);
    }

    #[tokio::test]
    async fn a_failing_list_keeps_its_last_good_copy_and_reports_why() {
        let d = dir("fail");
        let src = d.join("a.txt");
        std::fs::write(&src, "9.9.9.0/24\n").unwrap();
        let svc = IpFilterService::new(d.join("f"));
        let c = cfg(vec![list("a", &src)]);
        assert!(svc.refresh(&c, false, 10).await.active_file.is_some());

        std::fs::write(&src, "garbage only\n").unwrap();
        let r = svc.refresh(&c, true, 20).await;
        assert!(r.active_file.is_some(), "the previous good copy is still in use");
        assert!(!r.changed);
        let st = svc.status(&c);
        assert!(st.lists["a"].error.as_deref().unwrap().contains("no usable entries"), "{:?}", st.lists["a"]);
        assert_eq!(st.lists["a"].ranges, 1, "the good copy's size is still reported");

        std::fs::remove_file(&src).unwrap();
        assert!(svc.refresh(&c, true, 30).await.active_file.is_some(), "a missing file doesn't drop the filter either");
        let _ = std::fs::remove_dir_all(d);
    }

    #[tokio::test]
    async fn disabled_filter_or_nothing_usable_means_no_file_and_unsafe_ids_are_ignored() {
        let d = dir("off");
        let src = d.join("a.txt");
        std::fs::write(&src, "9.9.9.0/24\n").unwrap();
        let svc = IpFilterService::new(d.join("f"));
        let mut c = cfg(vec![list("a", &src), list("../escape", &src)]);
        assert!(svc.refresh(&c, false, 1).await.active_file.is_some());
        assert!(!d.join("escape.txt").exists() && !d.join("f").join("escape.txt").exists());

        c.enabled = false;
        let r = svc.refresh(&c, false, 2).await;
        assert_eq!(r, Refreshed { active_file: None, changed: true });
        assert!(svc.active_file(&c).is_none());

        c.enabled = true;
        c.lists[0].enabled = false;
        assert!(svc.refresh(&c, false, 3).await.active_file.is_none(), "no enabled lists, nothing to filter");
        let _ = std::fs::remove_dir_all(d);
    }

    #[tokio::test]
    async fn removed_lists_lose_their_cache() {
        let d = dir("gc");
        let src = d.join("a.txt");
        std::fs::write(&src, "9.9.9.0/24\n").unwrap();
        let svc = IpFilterService::new(d.join("f"));
        svc.refresh(&cfg(vec![list("a", &src)]), false, 1).await;
        assert!(d.join("f/lists/a.txt").exists());
        svc.refresh(&cfg(vec![]), false, 2).await;
        assert!(!d.join("f/lists/a.txt").exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[tokio::test]
    async fn state_survives_a_restart_so_an_unchanged_list_is_not_reloaded() {
        let d = dir("persist");
        let src = d.join("a.txt");
        std::fs::write(&src, "9.9.9.0/24\n").unwrap();
        let c = cfg(vec![list("a", &src)]);
        let first = IpFilterService::new(d.join("f"));
        assert!(first.refresh(&c, false, 1).await.changed);
        let second = IpFilterService::new(d.join("f"));
        assert!(!second.refresh(&c, false, 2).await.changed);
        let _ = std::fs::remove_dir_all(d);
    }

    #[tokio::test]
    async fn apply_hands_the_engine_the_port_proxy_and_filter_file() {
        let d = dir("apply");
        let src = d.join("a.txt");
        std::fs::write(&src, "9.9.9.0/24\n").unwrap();
        let engine = Arc::new(FakeEngine::new());
        let nc = NetControl::new(engine.clone(), d.join("f"));
        let mut s = Settings { listen_port: Some(6881), upnp: true, dht: false, proxy: Some("socks5://127.0.0.1:1080".into()), ..Settings::default() };
        s.ip_filter = cfg(vec![list("a", &src)]);
        let st = nc.apply(&s, Refresh::Force, 5).await.unwrap();
        let n = engine.network();
        assert_eq!((n.listen_port, n.upnp, n.dht), (Some(6881), true, false));
        assert_eq!(n.proxy.as_deref(), Some("socks5://127.0.0.1:1080"));
        assert_eq!(n.blocklist_file, Some(d.join("f").join("active.txt")));
        assert_eq!(st.active_ranges, 1);

        s.ip_filter.enabled = false;
        s.proxy = Some("   ".into());
        nc.apply(&s, Refresh::No, 6).await.unwrap();
        let n = engine.network();
        assert_eq!((n.blocklist_file, n.proxy), (None, None), "blank proxy and a disabled filter are dropped");
        let _ = std::fs::remove_dir_all(d);
    }
}
