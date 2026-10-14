//! The operations both front ends (desktop IPC, HTTP) perform on top of the manager. Anything a front end needs
//! that isn't plain torrent control lives here once, so the two can't drift apart.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::background::unix_now;
use crate::engine::{EngineError, TorrentEngine};
use crate::errors::ApiError;
use crate::manager::{Manager, SharedSettings};
use crate::model::{AddRequest, AddSource, TorrentId, TorrentPreview};
use crate::netctl::{NetControl, Refresh};
use crate::playlist;
use crate::search::{self, config::Secrets, SearchQuery, SearchReport};
use crate::settings::{Settings, SettingsStore};

pub struct Service {
    pub manager: Arc<Manager>,
    pub engine: Arc<dyn TorrentEngine>,
    pub net: Arc<NetControl>,
    pub store: Arc<dyn SettingsStore>,
    pub settings: SharedSettings,
    pub secrets: Secrets,
    pub data_dir: PathBuf,
}

/// What the user wants to add. `only_files` limits the download to those file indices.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AddInput {
    Magnet {
        value: String,
        #[serde(default)]
        only_files: Option<Vec<usize>>,
    },
    /// A link to a .torrent file (search results from sources without magnets).
    Url {
        value: String,
        #[serde(default)]
        only_files: Option<Vec<usize>>,
    },
    File {
        bytes: Vec<u8>,
        #[serde(default)]
        only_files: Option<Vec<usize>>,
    },
}

impl AddInput {
    fn into_parts(self) -> (AddSource, Option<Vec<usize>>) {
        match self {
            AddInput::Magnet { value, only_files } => (AddSource::Magnet(value.trim().to_string()), only_files),
            AddInput::Url { value, only_files } => (AddSource::Url(value.trim().to_string()), only_files),
            AddInput::File { bytes, only_files } => (AddSource::FileBytes(bytes), only_files),
        }
    }
}

impl Service {
    /// Reads a torrent's files without starting it.
    pub async fn inspect(&self, input: AddInput) -> Result<TorrentPreview, ApiError> {
        Ok(self.engine.inspect(input.into_parts().0).await?)
    }

    /// The manager picks the save folder (category, else the default) and the queue slot.
    pub async fn add(&self, input: AddInput, category: Option<String>) -> Result<TorrentId, ApiError> {
        let (source, only_files) = input.into_parts();
        let mut req = AddRequest::new(source);
        req.only_files = only_files;
        Ok(self.manager.add(req, category, Vec::new()).await?)
    }

    /// Saves and applies new settings. Returns the settings that were in force before, so the caller can react to
    /// what changed (the desktop app syncs its autostart entry). A failed network change puts everything back.
    pub async fn apply_settings(&self, new: Settings) -> Result<Settings, ApiError> {
        let fail = |what: &str, e: &dyn std::fmt::Display| {
            eprintln!("{what}: {e}");
            ApiError::new("settings", "Couldn't save your settings. Check that the app data folder is writable.")
        };
        self.store.save(&new).map_err(|e| fail("settings save failed", &e))?;
        let before = std::mem::replace(&mut *self.settings.lock().unwrap(), new.clone());

        self.manager.sync_limits().await?;

        let network_changed = before.listen_port != new.listen_port
            || before.upnp != new.upnp
            || before.dht != new.dht
            || before.proxy != new.proxy
            || before.ip_filter != new.ip_filter;
        if network_changed {
            if let Err(e) = self.net.apply(&new, Refresh::IfDue, unix_now()).await {
                eprintln!("network settings failed: {e}");
                // Put everything back so the saved settings always match what is actually running.
                let _ = self.store.save(&before);
                *self.settings.lock().unwrap() = before.clone();
                let _ = self.net.apply(&before, Refresh::No, unix_now()).await;
                let why = match e {
                    EngineError::InvalidInput(_) => "That proxy address isn't valid. Use the form socks5://host:port.",
                    _ => "Couldn't apply the network settings. Is the port already in use? Your previous settings are still running.",
                };
                return Err(ApiError::new("network", why));
            }
        }
        if before.search != new.search {
            let ids: Vec<&str> = new.search.torznab.iter().map(|t| t.id.as_str()).collect();
            let _ = self.secrets.retain(&ids);
        }
        Ok(before)
    }

    pub async fn search(&self, text: &str) -> Result<SearchReport, ApiError> {
        let providers = search::config::build_providers(&self.settings.lock().unwrap().search, &self.secrets);
        if providers.is_empty() {
            return Err(ApiError::new("no_sources", "No search sources are turned on. Add one in Settings."));
        }
        let q = SearchQuery { text: text.trim().chars().take(200).collect(), limit: 50 };
        Ok(search::search_all(&providers, &q, search::DEFAULT_TIMEOUT).await)
    }

    /// Ids of sources that have a key saved. The keys themselves never leave the backend.
    pub fn search_keys_set(&self) -> Vec<String> {
        let s = self.settings.lock().unwrap();
        s.search.torznab.iter().filter(|t| self.secrets.has(&t.id)).map(|t| t.id.clone()).collect()
    }

    pub fn set_search_key(&self, id: &str, key: &str) -> Result<(), ApiError> {
        if !self.settings.lock().unwrap().search.torznab.iter().any(|t| t.id == id) {
            return Err(ApiError::new("search", "Save the source first, then add its key."));
        }
        self.secrets.set(id, key.trim()).map_err(|e| {
            eprintln!("saving a search key failed: {e}");
            ApiError::new("settings", "Couldn't save the key. Check that the app data folder is writable.")
        })
    }

    pub async fn update_ip_lists(&self) -> Result<crate::netctl::FilterStatus, ApiError> {
        let s = self.settings.lock().unwrap().clone();
        self.net.apply(&s, Refresh::Force, unix_now()).await.map_err(|e| {
            eprintln!("ip list update failed: {e}");
            ApiError::new("network", "Couldn't apply the IP filter. Your previous network settings are still in use.")
        })
    }
}

/// Writes `<save folder>/<name>.m3u8` for a multi-file torrent. Returns the path, or `None` when the
/// torrent has fewer than two video/audio files.
pub async fn write_playlist(engine: &dyn TorrentEngine, id: &TorrentId) -> Result<Option<PathBuf>, EngineError> {
    let d = engine.details(id).await?;
    let paths: Vec<String> = d.files.iter().map(|f| f.path.clone()).collect();
    let Some(text) = playlist::m3u8(&paths) else { return Ok(None) };
    let safe: String = d.stats.name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
    let name = if safe.trim().is_empty() { "playlist".to_string() } else { safe };
    let path = Path::new(&d.save_path).join(format!("{name}.m3u8"));
    std::fs::write(&path, text).map_err(|e| EngineError::Backend(format!("playlist write failed: {e}")))?;
    Ok(Some(path))
}
