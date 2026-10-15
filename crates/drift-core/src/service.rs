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
    pub plugins: Arc<crate::plugins::Host>,
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
        Ok(before)
    }

    pub async fn search(&self, text: &str) -> Result<SearchReport, ApiError> {
        let enabled = self.settings.lock().unwrap().plugins.clone();
        let providers = self.plugins.search_providers(&enabled);
        if providers.is_empty() {
            return Err(ApiError::new("no_sources", "No search sources are turned on. Turn one on in Settings, under Plugins."));
        }
        let q = SearchQuery { text: text.trim().chars().take(200).collect(), limit: 50 };
        Ok(search::search_all(&providers, &q, search::DEFAULT_TIMEOUT).await)
    }

    /// Asks one search plugin a sample question so Settings can say whether it answers.
    pub async fn test_search_source(&self, id: &str) -> Result<search::SourceStatus, ApiError> {
        let enabled = self.settings.lock().unwrap().plugins.clone();
        let p = self.plugins.search_providers(&enabled).into_iter().find(|p| p.id() == id).ok_or_else(|| ApiError::new("search", "Turn the plugin on first, then test it."))?;
        let q = SearchQuery { text: "test".into(), limit: 1 };
        let mut report = search::search_all(&[p], &q, search::DEFAULT_TIMEOUT).await;
        Ok(report.sources.remove(0))
    }

    /// Moves the old built-in search settings (Internet Archive switch, Torznab indexers and their keys) into the
    /// plugins that replaced them. Safe to call on every start: once moved, there is nothing left to move.
    pub async fn migrate_search_to_plugins(&self) {
        search::migrate::run(&self.settings, self.store.as_ref(), &self.plugins, &self.secrets).await;
    }

    fn plugins_dir(&self) -> std::path::PathBuf {
        self.data_dir.join("plugins")
    }

    /// Switches off plugins that a registry update just brought in or gave more access, so they run only after
    /// the user turns them on again and sees the permission prompt. Call once, right after building the service.
    pub fn switch_off_unapproved_plugins(&self) {
        let ids = self.plugins.needs_approval();
        if ids.is_empty() {
            return;
        }
        let mut s = self.settings.lock().unwrap();
        let before = s.plugins.len();
        s.plugins.retain(|p| !ids.contains(p));
        if s.plugins.len() != before {
            let _ = self.store.save(&s);
        }
    }

    /// The registry's signed index, fetched only when the user has opted in.
    async fn registry_index(&self) -> Result<crate::plugins::registry::Index, ApiError> {
        use crate::plugins::registry;
        if !self.settings.lock().unwrap().plugin_registry {
            return Err(ApiError::new("registry_off", "Online plugin lookup is switched off. Turn it on first."));
        }
        registry::fetch_index(registry::INDEX_URL, registry::PUBKEY).await.map_err(registry_error)
    }

    /// Plugins the registry offers, each marked with the version installed now.
    pub async fn plugin_offers(&self) -> Result<Vec<crate::plugins::registry::Offer>, ApiError> {
        let index = self.registry_index().await?;
        let installed = self.list_plugins().into_iter().map(|p| (p.id, p.version)).collect();
        Ok(crate::plugins::registry::offers(&index, &installed))
    }

    /// Downloads, verifies and parks a plugin (or its update). It takes over at the next start.
    /// `version` is the one the user was shown; if the list has moved on, nothing is installed.
    pub async fn install_plugin(&self, id: &str, version: &str) -> Result<(), ApiError> {
        use crate::plugins::registry;
        let index = self.registry_index().await?;
        let (_, v) = registry::pick(&index, id).ok_or_else(|| registry_error(registry::RegistryError::NotFound))?;
        if v.version != version {
            return Err(ApiError::new("plugin", "The plugin list changed since you looked. Check again."));
        }
        // Never move a plugin backwards from here (an old signed list stays valid forever); rollback is its own action.
        if self.list_plugins().iter().any(|p| p.id == id && !registry::is_newer(&v.version, &p.version)) {
            return Err(ApiError::new("plugin", "That version is already installed, or an older one."));
        }
        let bytes = registry::download(v).await.map_err(registry_error)?;
        registry::stage(&self.plugins_dir(), id, v, &bytes).map_err(registry_error)
    }

    /// Parks the version that was replaced last time; it comes back at the next start.
    pub fn rollback_plugin(&self, id: &str) -> Result<(), ApiError> {
        crate::plugins::registry::stage_previous(&self.plugins_dir(), id).map_err(registry_error)
    }

    pub async fn set_plugin_settings(&self, id: &str, values: serde_json::Map<String, serde_json::Value>) -> Result<(), ApiError> {
        self.plugins.set_values(id, values).await.map_err(|m| ApiError::new("plugin", &m))
    }

    pub async fn set_plugin_secret(&self, id: &str, key: &str, value: &str) -> Result<(), ApiError> {
        self.plugins.set_secret(id, key, value).await.map_err(|m| ApiError::new("plugin", &m))
    }

    pub fn plugin_log(&self, id: &str) -> Result<String, ApiError> {
        self.plugins.log(id).map_err(|m| ApiError::new("plugin", &m))
    }

    pub fn list_plugins(&self) -> Vec<crate::plugins::PluginInfo> {
        self.plugins.list(&self.settings.lock().unwrap().plugins)
    }

    pub async fn update_ip_lists(&self) -> Result<crate::netctl::FilterStatus, ApiError> {
        let s = self.settings.lock().unwrap().clone();
        self.net.apply(&s, Refresh::Force, unix_now()).await.map_err(|e| {
            eprintln!("ip list update failed: {e}");
            ApiError::new("network", "Couldn't apply the IP filter. Your previous network settings are still in use.")
        })
    }
}

fn registry_error(e: crate::plugins::registry::RegistryError) -> ApiError {
    ApiError::new("plugin", &e.to_string())
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
