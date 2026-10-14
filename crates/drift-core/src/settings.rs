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
    /// Global speed limits apply only while this is on, so the values survive being toggled off.
    pub limits_enabled: bool,
    /// KiB/s; 0 means unlimited for that direction.
    pub down_limit_kib: u32,
    pub up_limit_kib: u32,
    /// Closing the window hides it to the tray instead of quitting.
    pub close_to_tray: bool,
    /// Launch hidden in the tray (used with start-with-Windows).
    pub start_minimized: bool,
    /// Mirrors the OS autostart entry; the app keeps the two in sync.
    pub start_with_windows: bool,
    pub notify_on_complete: bool,
    /// How many incomplete torrents download at once; 0 means no limit. Others wait in the queue.
    pub max_active_downloads: u32,
    /// Stop seeding goal applied to every torrent without its own.
    pub default_goal: crate::rules::Goal,
    /// Nothing stops seeding before this many minutes, whatever the ratio.
    pub min_seed_minutes: u32,
    pub categories: Vec<Category>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    /// New torrents in this category download here instead of the default folder.
    pub save_path: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: CURRENT_SCHEMA,
            download_dir: None,
            theme: Theme::System,
            limits_enabled: false,
            down_limit_kib: 0,
            up_limit_kib: 0,
            close_to_tray: false,
            start_minimized: false,
            start_with_windows: false,
            notify_on_complete: true,
            max_active_downloads: 3,
            default_goal: crate::rules::Goal::default(),
            min_seed_minutes: 0,
            categories: Vec::new(),
        }
    }
}

impl Settings {
    /// What the engine should enforce right now, in bytes per second.
    pub fn effective_limits(&self) -> crate::model::Limits {
        let bps = |kib: u32| (self.limits_enabled && kib > 0).then(|| kib.saturating_mul(1024));
        crate::model::Limits { down_bps: bps(self.down_limit_kib), up_bps: bps(self.up_limit_kib) }
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
    let bytes = crate::jsonfile::strip_bom(bytes);
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
        crate::jsonfile::write_atomic(&self.path, &serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?)
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
    fn limits_apply_only_when_enabled_and_nonzero() {
        let mut s = Settings { down_limit_kib: 500, up_limit_kib: 0, ..Settings::default() };
        assert_eq!(s.effective_limits(), crate::model::Limits::default());
        s.limits_enabled = true;
        assert_eq!(s.effective_limits(), crate::model::Limits { down_bps: Some(500 * 1024), up_bps: None });
    }

    #[test]
    fn older_files_without_new_fields_still_load() {
        let dir = tmp("old");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, br#"{"schema":1,"download_dir":"D:/x","theme":"dark"}"#).unwrap();
        let s = JsonFileStore::new(&path).load();
        assert_eq!(s.theme, Theme::Dark);
        assert!(s.notify_on_complete && !s.limits_enabled);
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
    fn a_utf8_bom_from_notepad_is_accepted() {
        let dir = tmp("bom");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"schema":1,"theme":"dark"}"#);
        std::fs::write(&path, bytes).unwrap();
        let s = JsonFileStore::new(&path).load();
        assert_eq!(s.theme, Theme::Dark);
        assert!(!bad_path(&path).exists(), "a BOM must not be treated as corruption");
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
