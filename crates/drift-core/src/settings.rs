//! App settings and the store contract. JSON file, versioned, forward-only migrations.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const CURRENT_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema: u32,
    /// `None` until the app resolves the OS Downloads folder on first run.
    pub download_dir: Option<PathBuf>,
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self { schema: CURRENT_SCHEMA, download_dir: None, theme: Theme::System }
    }
}

/// Persistence contract, consumed by the composition root and `core` services.
pub trait SettingsStore: Send + Sync {
    /// Never fails: a missing or unreadable file yields defaults.
    fn load(&self) -> Settings;
    fn save(&self, settings: &Settings) -> std::io::Result<()>;
}

pub struct JsonFileStore {
    path: PathBuf,
}

impl JsonFileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

/// Upgrades any older shape to `CURRENT_SCHEMA`. Unknown or newer files are read as-is.
fn migrate(mut v: serde_json::Value) -> serde_json::Value {
    let schema = v.get("schema").and_then(|s| s.as_u64()).unwrap_or(0);
    if schema < 1 {
        // v0 (pre-release dev builds) stored the folder as `download_folder`.
        if let Some(obj) = v.as_object_mut() {
            if let Some(old) = obj.remove("download_folder") {
                obj.entry("download_dir").or_insert(old);
            }
            obj.insert("schema".into(), 1.into());
        }
    }
    v
}

fn parse(bytes: &[u8]) -> Option<Settings> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    serde_json::from_value(migrate(v)).ok()
}

impl SettingsStore for JsonFileStore {
    fn load(&self) -> Settings {
        let Ok(bytes) = std::fs::read(&self.path) else {
            return Settings::default();
        };
        parse(&bytes).unwrap_or_else(|| {
            // Keep the unreadable file for inspection instead of silently overwriting it.
            let _ = std::fs::rename(&self.path, bad_path(&self.path));
            Settings::default()
        })
    }

    fn save(&self, settings: &Settings) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write-then-rename so a crash never leaves a half-written file.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, &self.path)
    }
}

fn bad_path(p: &Path) -> PathBuf {
    p.with_extension("json.bad")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("drift-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn round_trips() {
        let dir = tmp("rt");
        let store = JsonFileStore::new(dir.join("settings.json"));
        let s = Settings { download_dir: Some("D:/dl".into()), theme: Theme::Dark, ..Settings::default() };
        store.save(&s).unwrap();
        assert_eq!(store.load(), s);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn migrates_v0_and_ignores_unknown_fields() {
        let dir = tmp("mig");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, br#"{"download_folder":"E:/old","theme":"light","future_field":1}"#).unwrap();
        let s = JsonFileStore::new(&path).load();
        assert_eq!(s.schema, CURRENT_SCHEMA);
        assert_eq!(s.download_dir, Some("E:/old".into()));
        assert_eq!(s.theme, Theme::Light);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_file_gives_defaults_and_is_kept() {
        let dir = tmp("bad");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(JsonFileStore::new(&path).load(), Settings::default());
        assert!(bad_path(&path).exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
