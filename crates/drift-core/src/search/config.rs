//! The secrets file, and the shape of the old built-in search settings. Search sources are plugins now; the old
//! settings are only read once, by `search::migrate`, to move them over. Keys live in their own file, never in
//! settings, so the settings the UI reads and writes can't leak them.
// ponytail: plain file in the per-user app-data folder; move to Windows Credential Manager if keys need OS-level protection.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

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

    /// Every stored key (never the values).
    pub fn keys(&self) -> Vec<String> {
        self.map.lock().unwrap().keys().cloned().collect()
    }

    /// The stored values whose key starts with `prefix`; used to blank them out of logs.
    pub fn values_with_prefix(&self, prefix: &str) -> Vec<String> {
        self.map.lock().unwrap().iter().filter(|(k, v)| k.starts_with(prefix) && !v.is_empty()).map(|(_, v)| v.clone()).collect()
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

#[cfg(test)]
mod tests {
    use super::*;

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
