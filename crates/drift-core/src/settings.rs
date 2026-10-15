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

/// A colour theme imported from a file. Plain data: the UI checks every value and only ever sets colour tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemePack {
    pub name: String,
    /// `light` or `dark`: which built-in theme the missing colours come from.
    pub base: String,
    pub tokens: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema: u32,
    /// `None` until the app resolves the OS Downloads folder on first run.
    pub download_dir: Option<PathBuf>,
    pub theme: Theme,
    /// Replaces the colours of the chosen theme when set.
    pub theme_pack: Option<ThemePack>,
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
    /// Look for a new version on GitHub once a day. Installing always asks first.
    pub check_updates: bool,
    /// How many incomplete torrents download at once; 0 means no limit. Others wait in the queue.
    pub max_active_downloads: u32,
    /// Stop seeding goal applied to every torrent without its own.
    pub default_goal: crate::rules::Goal,
    /// Nothing stops seeding before this many minutes, whatever the ratio.
    pub min_seed_minutes: u32,
    pub categories: Vec<Category>,
    /// Write an .m3u8 next to a finished multi-file torrent that has several video or audio files.
    pub playlist_on_complete: bool,
    /// Fixed incoming port; `None` picks a random one at each start.
    pub listen_port: Option<u16>,
    pub upnp: bool,
    pub dht: bool,
    /// `socks5://[user:pass@]host:port`; outgoing connections go through it and DHT and incoming are turned off.
    pub proxy: Option<String>,
    pub ip_filter: IpFilterSettings,
    pub schedule: crate::schedule::Schedule,
    /// `.torrent` files dropped here are added automatically.
    pub watch_dir: Option<PathBuf>,
    /// Finished downloads are moved here (and keep seeding from there). `None` leaves them where they were saved.
    pub move_on_complete: Option<PathBuf>,
    /// A single-file torrent is saved in a folder named after it, like a multi-file one.
    pub folder_for_single_files: bool,
    pub vpn_guard: crate::guard::VpnGuard,
    pub search: crate::search::config::SearchSettings,
    /// Ids of the plugins the user has turned on.
    pub plugins: Vec<String>,
    /// Off by default: Drift only contacts the plugin registry (github.com) when this is on.
    pub plugin_registry: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IpFilterSettings {
    pub enabled: bool,
    pub lists: Vec<IpList>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpList {
    /// Letters, digits, `-` and `_` only: it names the cached copy on disk.
    pub id: String,
    pub name: String,
    /// An http(s) address or a path to a file on this computer.
    pub source: String,
    pub enabled: bool,
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
            theme_pack: None,
            limits_enabled: false,
            down_limit_kib: 0,
            up_limit_kib: 0,
            close_to_tray: false,
            start_minimized: false,
            start_with_windows: false,
            notify_on_complete: true,
            check_updates: true,
            max_active_downloads: 3,
            default_goal: crate::rules::Goal::default(),
            min_seed_minutes: 0,
            categories: Vec::new(),
            playlist_on_complete: true,
            listen_port: None,
            upnp: false,
            dht: true,
            proxy: None,
            ip_filter: IpFilterSettings::default(),
            schedule: crate::schedule::Schedule::default(),
            watch_dir: None,
            move_on_complete: None,
            folder_for_single_files: false,
            vpn_guard: crate::guard::VpnGuard::default(),
            search: Default::default(),
            plugins: Vec::new(),
            plugin_registry: false,
        }
    }
}

impl Settings {
    /// The limits in force at this local time: an active schedule slot replaces the global limits.
    /// `weekday`: 0 = Monday ... 6 = Sunday; `minute`: minutes after local midnight.
    pub fn limits_at(&self, weekday: u8, minute: u16) -> (crate::model::Limits, bool) {
        match crate::schedule::active(&self.schedule, weekday, minute) {
            Some(slot) => {
                let bps = |kib: u32| (kib > 0).then(|| kib.saturating_mul(1024));
                (crate::model::Limits { down_bps: bps(slot.down_kib), up_bps: bps(slot.up_kib) }, true)
            }
            None => (self.effective_limits(), false),
        }
    }

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
    /// The file is there but couldn't be read, so what `load` returned are stand-in values: saving them would
    /// replace the user's real settings.
    unreadable: std::sync::atomic::AtomicBool,
}

impl JsonFileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), unreadable: Default::default() }
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
        // A file that is there but can't be read this instant (a virus scanner or the installer has it) is not "no
        // settings yet": look again before giving up, so a busy file never costs the user their settings.
        let mut read = std::fs::read(&self.path);
        for _ in 0..10 {
            if !read.as_ref().is_err_and(|e| e.kind() != std::io::ErrorKind::NotFound) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            read = std::fs::read(&self.path);
        }
        let missing = read.as_ref().is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
        self.unreadable.store(read.is_err() && !missing, std::sync::atomic::Ordering::Relaxed);
        let Ok(bytes) = read else {
            return Settings::default();
        };
        parse(&bytes).unwrap_or_else(|| {
            // Keep the unreadable file for inspection instead of silently overwriting it.
            let _ = std::fs::rename(&self.path, bad_path(&self.path));
            Settings::default()
        })
    }

    fn save(&self, settings: &Settings) -> std::io::Result<()> {
        if self.unreadable.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(std::io::Error::other("the settings file couldn't be read when Drift started, so it is left as it is"));
        }
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

    /// A settings file that something else has open for a moment (a virus scanner, the installer) is still the
    /// user's settings: loading waits for it instead of falling back to the defaults.
    #[cfg(windows)]
    #[test]
    fn a_file_that_is_busy_for_a_moment_is_still_loaded() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tmp("busy");
        let path = dir.join("settings.json");
        let store = JsonFileStore::new(&path);
        let mine = Settings { download_dir: Some("E:/hh".into()), close_to_tray: true, ..Settings::default() };
        store.save(&mine).unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            drop(held);
        });
        assert_eq!(store.load(), mine);
        release.join().unwrap();

        // Held for longer than the wait: the defaults come back, but nothing is allowed to overwrite the real file.
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        assert_eq!(store.load(), Settings::default());
        assert!(store.save(&Settings::default()).is_err());
        drop(held);
        assert_eq!(std::fs::read(&path).map(|b| parse(&b)).unwrap(), Some(mine.clone()));
        assert_eq!(store.load(), mine, "readable again, saving works again");
        store.save(&mine).unwrap();
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
