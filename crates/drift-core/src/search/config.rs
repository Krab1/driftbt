//! Which search sources are on, and the secrets they need. Keys live in their own file, never in settings,
//! so the settings the UI reads and writes can't leak them.
// ponytail: plain file in the per-user app-data folder; move to Windows Credential Manager if keys need OS-level protection.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::archive_org::ArchiveOrg;
use super::torznab::Torznab;
use super::SearchProvider;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchSettings {
    pub archive_org: bool,
    pub torznab: Vec<TorznabConfig>,
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self { archive_org: true, torznab: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TorznabConfig {
    /// Letters, digits, `-` and `_`; names the secret and the result badge.
    pub id: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
}

impl TorznabConfig {
    pub fn id_is_valid(&self) -> bool {
        !self.id.is_empty() && self.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }
}

/// API keys by source id, in `secrets.json`.
pub struct Secrets {
    path: PathBuf,
    map: Mutex<HashMap<String, String>>,
}

impl Secrets {
    pub fn open(path: PathBuf) -> Self {
        let map = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(crate::jsonfile::strip_bom(&b)).ok()).unwrap_or_default();
        Self { path, map: Mutex::new(map) }
    }

    pub fn get(&self, id: &str) -> String {
        self.map.lock().unwrap().get(id).cloned().unwrap_or_default()
    }

    pub fn has(&self, id: &str) -> bool {
        !self.get(id).is_empty()
    }

    /// An empty key removes the entry.
    pub fn set(&self, id: &str, key: &str) -> std::io::Result<()> {
        let mut m = self.map.lock().unwrap();
        if key.is_empty() {
            m.remove(id);
        } else {
            m.insert(id.to_string(), key.to_string());
        }
        crate::jsonfile::write_atomic(&self.path, &serde_json::to_vec_pretty(&*m).map_err(std::io::Error::other)?)
    }

    pub fn retain(&self, ids: &[&str]) -> std::io::Result<()> {
        let mut m = self.map.lock().unwrap();
        let before = m.len();
        m.retain(|k, _| ids.contains(&k.as_str()));
        if m.len() == before {
            return Ok(());
        }
        crate::jsonfile::write_atomic(&self.path, &serde_json::to_vec_pretty(&*m).map_err(std::io::Error::other)?)
    }
}

/// Enabled, valid sources only; a source with a bad address is left out rather than failing every search.
pub fn build_providers(s: &SearchSettings, secrets: &Secrets) -> Vec<Arc<dyn SearchProvider>> {
    let mut out: Vec<Arc<dyn SearchProvider>> = Vec::new();
    if s.archive_org {
        out.push(Arc::new(ArchiveOrg::new()));
    }
    for t in s.torznab.iter().filter(|t| t.enabled && t.id_is_valid()) {
        if let Ok(p) = Torznab::new(&t.id, &t.name, &t.url, &secrets.get(&t.id)) {
            out.push(Arc::new(p));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tz(id: &str, url: &str, enabled: bool) -> TorznabConfig {
        TorznabConfig { id: id.into(), name: id.to_uppercase(), url: url.into(), enabled }
    }

    #[test]
    fn only_enabled_valid_sources_are_built() {
        let dir = std::env::temp_dir().join(format!("drift-sec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sec = Secrets::open(dir.join("secrets.json"));
        let s = SearchSettings {
            archive_org: true,
            torznab: vec![tz("ok", "http://localhost:9117/api", true), tz("off", "http://localhost:9117/api", false), tz("bad", "nonsense", true), tz("bad id!", "http://x/", true)],
        };
        let ids: Vec<_> = build_providers(&s, &sec).iter().map(|p| p.id().to_string()).collect();
        assert_eq!(ids, ["archive_org", "ok"]);
        assert!(build_providers(&SearchSettings { archive_org: false, torznab: vec![] }, &sec).is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn secrets_persist_clear_and_prune() {
        let dir = std::env::temp_dir().join(format!("drift-sec2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("secrets.json");
        let s = Secrets::open(p.clone());
        s.set("a", "k1").unwrap();
        s.set("b", "k2").unwrap();
        assert_eq!(Secrets::open(p.clone()).get("a"), "k1");
        s.set("a", "").unwrap();
        assert!(!s.has("a") && s.has("b"));
        s.retain(&[]).unwrap();
        assert!(!Secrets::open(p).has("b"));
        std::fs::remove_dir_all(dir).ok();
    }
}
