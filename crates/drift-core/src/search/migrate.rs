//! One-time move of the old built-in search settings into the plugins that replaced them: the Internet Archive
//! switch becomes the `archive_org` plugin being on, and each Torznab indexer (with its key) becomes a row of the
//! `torznab` plugin. Nothing is cleared until its plugin has taken the data, so a missing plugin loses nothing.

use serde_json::{json, Map, Value};

use super::config::{Secrets, TorznabConfig};
use crate::manager::SharedSettings;
use crate::plugins::Host;
use crate::settings::SettingsStore;

pub async fn run(settings: &SharedSettings, store: &dyn SettingsStore, host: &Host, secrets: &Secrets) {
    let legacy = settings.lock().unwrap().search.clone();
    if !legacy.archive_org && legacy.torznab.is_empty() {
        return;
    }
    let usable = |id: &str| host.list(&[]).iter().any(|p| p.id == id && p.problem.is_none());

    let mut turn_on: Vec<&str> = Vec::new();
    let mut archive_done = !legacy.archive_org;
    if legacy.archive_org && usable("archive_org") {
        turn_on.push("archive_org");
        archive_done = true;
    }
    let mut torznab_done = legacy.torznab.is_empty();
    if !torznab_done && usable("torznab") {
        torznab_done = move_indexers(&legacy.torznab, host, secrets).await;
        if torznab_done && legacy.torznab.iter().any(|t| t.enabled) {
            turn_on.push("torznab");
        }
    }
    if !archive_done && !torznab_done {
        return;
    }
    {
        let mut s = settings.lock().unwrap();
        for id in turn_on {
            if !s.plugins.iter().any(|p| p == id) {
                s.plugins.push(id.to_string());
            }
        }
        if archive_done {
            s.search.archive_org = false;
        }
        if torznab_done {
            s.search.torznab.clear();
        }
        if let Err(e) = store.save(&s) {
            eprintln!("saving the migrated search settings failed: {e}");
        }
    }
    if torznab_done {
        // The keys now live with the plugin; nothing else reads the old file.
        let _ = secrets.retain(&[]);
    }
}

/// An old indexer the plugin's form will accept. Old settings could hold things the form rejects (an id over 64
/// characters, a duplicate, a name or address that is too long, an address that isn't http or https); those rows
/// never worked in search either, so they are left out instead of stopping the move.
fn usable(t: &TorznabConfig, taken: &[String]) -> Option<Value> {
    let url = t.url.trim();
    let ok = t.id_is_valid() && t.id.len() <= 64 && !taken.contains(&t.id) && crate::plugins::is_http_url(url) && url.len() <= 2048 && !url.contains(char::is_whitespace);
    if !ok {
        eprintln!("search migration: skipped an old Torznab indexer the plugin can't take ({})", t.id.chars().take(40).collect::<String>());
        return None;
    }
    Some(json!({"id": t.id, "name": t.name.chars().take(500).collect::<String>(), "url": url, "enabled": t.enabled}))
}

/// Adds the old indexers to the plugin's list and hands over their keys. Rows already in the plugin, and the keys
/// they hold, are never touched, so running this again after a partial failure can't undo what the user changed
/// in the meantime. Returns whether everything that could move did.
async fn move_indexers(old: &[TorznabConfig], host: &Host, secrets: &Secrets) -> bool {
    let current = host.list(&[]).into_iter().find(|p| p.id == "torznab");
    let mut rows: Vec<Value> = current.as_ref().and_then(|p| p.values.get("indexers")).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut taken: Vec<String> = rows.iter().filter_map(|r| r["id"].as_str().map(String::from)).collect();
    let mut added: Vec<&TorznabConfig> = Vec::new();
    for t in old {
        if rows.len() >= 50 {
            break;
        }
        if let Some(row) = usable(t, &taken) {
            taken.push(t.id.clone());
            rows.push(row);
            added.push(t);
        }
    }
    let mut values = Map::new();
    values.insert("indexers".into(), Value::Array(rows));
    if host.set_values("torznab", values).await.is_err() {
        return false;
    }
    let held = current.map(|p| p.secrets_set).unwrap_or_default();
    for t in added {
        let key = secrets.get(&t.id);
        let name = format!("indexers.{}.apikey", t.id);
        if !key.is_empty() && !held.contains(&name) && host.set_secret("torznab", &name, &key).await.is_err() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::Limits;
    use crate::search::config::SearchSettings;
    use crate::settings::{JsonFileStore, Settings};
    use std::sync::{Arc, Mutex};

    /// A plugins folder holding the real first-party manifests with a stand-in program (nothing is started).
    fn plugins_with(dir: &std::path::Path, ids: &[&str]) -> Host {
        for (id, manifest) in [("archive_org", include_str!("../../../drift-search-plugin/plugins/archive_org/plugin.json")), ("torznab", include_str!("../../../drift-search-plugin/plugins/torznab/plugin.json"))] {
            if ids.contains(&id) {
                std::fs::create_dir_all(dir.join(id)).unwrap();
                std::fs::write(dir.join(id).join("plugin.json"), manifest).unwrap();
                std::fs::write(dir.join(id).join("drift-search.exe"), "x").unwrap();
            }
        }
        Host::load(dir, &dir.join("logs"), Limits::default())
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("drift-mig-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn tz(id: &str, enabled: bool) -> TorznabConfig {
        TorznabConfig { id: id.into(), name: id.to_uppercase(), url: format!("http://localhost:9117/{id}/"), enabled }
    }

    fn with(search: SearchSettings) -> SharedSettings {
        Arc::new(Mutex::new(Settings { search, ..Settings::default() }))
    }

    #[tokio::test]
    async fn indexers_keys_and_the_archive_switch_move_to_their_plugins() {
        let dir = scratch("ok");
        let host = plugins_with(&dir, &["archive_org", "torznab"]);
        let secrets = Secrets::open(dir.join("old-secrets.json"));
        secrets.set("prowlarr", "KEY-P").unwrap();
        let settings = with(SearchSettings { archive_org: true, torznab: vec![tz("prowlarr", true), tz("spare", false)] });
        let store = JsonFileStore::new(dir.join("settings.json"));

        run(&settings, &store, &host, &secrets).await;

        let s = settings.lock().unwrap().clone();
        assert_eq!(s.plugins, ["archive_org", "torznab"]);
        assert!(!s.search.archive_org && s.search.torznab.is_empty(), "the old settings are consumed");
        let info = host.list(&s.plugins);
        let t = info.iter().find(|p| p.id == "torznab").unwrap();
        assert_eq!(t.values["indexers"][0]["name"], "PROWLARR");
        assert_eq!(t.values["indexers"][1]["enabled"], false);
        assert_eq!(t.secrets_set, ["indexers.prowlarr.apikey"], "the key came along, the keyless one has none");
        assert!(!secrets.has("prowlarr"), "and the old copy is deleted");
        assert!(store.load().plugins.contains(&"torznab".to_string()), "saved to disk");

        // Running it again changes nothing.
        let before = settings.lock().unwrap().clone();
        run(&settings, &store, &host, &secrets).await;
        assert_eq!(*settings.lock().unwrap(), before);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn one_row_the_form_would_refuse_does_not_stop_the_others() {
        let dir = scratch("tolerant");
        let host = plugins_with(&dir, &["torznab"]);
        let secrets = Secrets::open(dir.join("old-secrets.json"));
        secrets.set("good", "KEY-G").unwrap();
        let bad_url = TorznabConfig { id: "bad".into(), name: "Bad".into(), url: "nonsense".into(), enabled: true };
        let upper = TorznabConfig { id: "upper".into(), name: "Upper".into(), url: "HTTP://Example.org/api".into(), enabled: true };
        let long_id = TorznabConfig { id: "x".repeat(70), ..tz("x", true) };
        let settings = with(SearchSettings { archive_org: false, torznab: vec![tz("good", true), bad_url, upper, long_id, tz("good", true)] });
        run(&settings, &JsonFileStore::new(dir.join("settings.json")), &host, &secrets).await;
        let t = host.list(&["torznab".to_string()]).into_iter().find(|p| p.id == "torznab").unwrap();
        let ids: Vec<_> = t.values["indexers"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids, ["good", "upper"], "the usable rows moved once each; the rest were left out");
        assert_eq!(t.secrets_set, ["indexers.good.apikey"]);
        assert!(settings.lock().unwrap().search.torznab.is_empty(), "and the move finished instead of retrying forever");
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_rerun_keeps_what_the_user_already_has_in_the_plugin() {
        let dir = scratch("rerun");
        let host = plugins_with(&dir, &["torznab"]);
        let secrets = Secrets::open(dir.join("old-secrets.json"));
        secrets.set("a", "OLD-KEY").unwrap();
        // The plugin already holds a row "a" with a newer key and a row "mine" the user added.
        let mut v = Map::new();
        v.insert("indexers".into(), json!([{"id": "a", "name": "Renamed", "url": "http://localhost:1/", "enabled": true}, {"id": "mine", "name": "Mine", "url": "http://localhost:2/", "enabled": true}]));
        host.set_values("torznab", v).await.unwrap();
        host.set_secret("torznab", "indexers.a.apikey", "NEW-KEY").await.unwrap();
        let settings = with(SearchSettings { archive_org: false, torznab: vec![tz("a", true), tz("b", true)] });
        run(&settings, &JsonFileStore::new(dir.join("settings.json")), &host, &secrets).await;
        let t = host.list(&[]).into_iter().find(|p| p.id == "torznab").unwrap();
        let names: Vec<_> = t.values["indexers"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["Renamed", "Mine", "B"], "nothing the user has was replaced; the missing indexer was added");
        let mut set = t.secrets_set.clone();
        set.sort();
        assert_eq!(set, ["indexers.a.apikey"], "the user's newer key stays (the old one was not copied over it)");
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn a_missing_plugin_loses_nothing() {
        let dir = scratch("missing");
        let host = plugins_with(&dir, &["archive_org"]); // no torznab plugin installed
        let secrets = Secrets::open(dir.join("old-secrets.json"));
        secrets.set("prowlarr", "KEY-P").unwrap();
        let settings = with(SearchSettings { archive_org: true, torznab: vec![tz("prowlarr", true)] });
        run(&settings, &JsonFileStore::new(dir.join("settings.json")), &host, &secrets).await;
        let s = settings.lock().unwrap().clone();
        assert_eq!(s.plugins, ["archive_org"]);
        assert_eq!(s.search.torznab.len(), 1, "the indexers wait until their plugin exists");
        assert!(secrets.has("prowlarr"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn an_archive_switch_that_was_off_stays_off_and_an_empty_setup_is_left_alone() {
        let dir = scratch("off");
        let host = plugins_with(&dir, &["archive_org", "torznab"]);
        let secrets = Secrets::open(dir.join("s.json"));
        let settings = with(SearchSettings { archive_org: false, torznab: vec![] });
        run(&settings, &JsonFileStore::new(dir.join("settings.json")), &host, &secrets).await;
        assert!(settings.lock().unwrap().plugins.is_empty());
        assert!(!dir.join("settings.json").exists(), "nothing to move, nothing written");
        std::fs::remove_dir_all(dir).ok();
    }
}
