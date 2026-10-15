//! The plugin host against the real reference plugin binary: handshake, timeouts, crashes, idle shutdown.

use std::path::{Path, PathBuf};
use std::time::Duration;

use drift_core::plugins::{Host, Limits};
use drift_core::search::{ProviderError, SearchQuery};

const EXE: &str = env!("CARGO_BIN_EXE_drift-example-plugin");

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("drift-plug-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Installs the reference plugin as `<root>/<id>/` with the given mode argument and manifest api.
fn install(root: &Path, id: &str, mode: &str, api: u32) {
    let d = root.join(id);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::copy(EXE, d.join("plugin.exe")).unwrap();
    let exec = if mode.is_empty() { r#"["plugin.exe"]"#.to_string() } else { format!(r#"["plugin.exe", "{mode}"]"#) };
    std::fs::write(d.join("plugin.json"), format!(r#"{{"id":"{id}","name":"Example","version":"1.0.0","api":{api},"kinds":["search"],"exec":{exec},"permissions":{{"net":["*"]}}}}"#)).unwrap();
}

fn limits() -> Limits {
    Limits { call_timeout: Duration::from_millis(1500), idle: Duration::from_millis(400), backoff: Duration::from_millis(30) }
}

fn q() -> SearchQuery {
    SearchQuery { text: "ubuntu".into(), limit: 10 }
}

fn host(root: &Path) -> Host {
    Host::load(root, &root.join("logs"), limits())
}

#[tokio::test]
async fn handshake_and_search() {
    let root = scratch("ok");
    install(&root, "ex", "", 1);
    let h = host(&root);
    let on = vec!["ex".to_string()];
    assert!(h.list(&on)[0].enabled && h.list(&on)[0].problem.is_none());
    assert!(!h.is_running("ex").await, "starts lazily");
    let ps = h.search_providers(&on);
    let r = ps[0].search(&q()).await.unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].title, "ubuntu (example one)");
    assert_eq!(r[1].info_hash.as_deref(), Some("89abcdef0123456789abcdef0123456789abcdef"));
    assert!(h.is_running("ex").await);
    assert!(h.search_providers(&[]).is_empty(), "off unless enabled");
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn notifications_between_replies_are_ignored() {
    let root = scratch("noise");
    install(&root, "ex", "noise", 1);
    let h = host(&root);
    let ps = h.search_providers(&["ex".into()]);
    assert_eq!(ps[0].search(&q()).await.unwrap().len(), 2);
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn wrong_api_and_escaping_exec_are_refused() {
    let root = scratch("refuse");
    install(&root, "old", "", 2);
    install(&root, "esc", "", 1);
    std::fs::write(root.join("esc").join("plugin.json"), r#"{"id":"esc","name":"E","version":"1","api":1,"kinds":["search"],"exec":["..\\..\\evil.exe"]}"#).unwrap();
    install(&root, "wrongname", "", 1);
    std::fs::rename(root.join("wrongname"), root.join("other")).unwrap();
    let h = host(&root);
    let on: Vec<String> = ["old", "esc", "wrongname", "other"].map(String::from).to_vec();
    let list = h.list(&on);
    assert_eq!(list.len(), 3);
    assert!(list.iter().all(|p| p.problem.is_some() && !p.enabled));
    assert!(h.search_providers(&on).is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn a_hanging_plugin_times_out_without_blocking_forever() {
    let root = scratch("hang");
    install(&root, "ex", "hang", 1);
    let mut l = limits();
    l.call_timeout = Duration::from_millis(300);
    let h = Host::load(&root, &root.join("logs"), l);
    let t = std::time::Instant::now();
    let e = h.search_providers(&["ex".into()])[0].search(&q()).await.unwrap_err();
    assert_eq!(e, ProviderError::Plugin);
    assert!(t.elapsed() < Duration::from_secs(3));
    assert!(!h.is_running("ex").await, "killed after the timeout");
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn three_crashes_switch_the_plugin_off() {
    let root = scratch("crash");
    install(&root, "ex", "crash", 1);
    let h = host(&root);
    let on = vec!["ex".to_string()];
    let p = &h.search_providers(&on)[0];
    for _ in 0..3 {
        assert_eq!(p.search(&q()).await.unwrap_err(), ProviderError::Plugin);
        tokio::time::sleep(Duration::from_millis(150)).await; // longer than the biggest backoff here
    }
    assert!(h.list(&on)[0].problem.as_deref().is_some_and(|m| m.contains("switched off")));
    assert_eq!(p.search(&q()).await.unwrap_err(), ProviderError::Plugin);
    assert!(!h.is_running("ex").await);
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn idle_plugins_are_shut_down_and_restart_on_demand() {
    let root = scratch("idle");
    install(&root, "ex", "", 1);
    let h = host(&root);
    let p = &h.search_providers(&["ex".into()])[0];
    p.search(&q()).await.unwrap();
    assert!(h.is_running("ex").await);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(!h.is_running("ex").await);
    assert_eq!(p.search(&q()).await.unwrap().len(), 2);
    std::fs::remove_dir_all(root).ok();
}

fn install_with_form(root: &Path, id: &str) {
    install(root, id, "settings", 1);
    let form = r#""settings":[{"key":"greeting","label":"Greeting","type":"text"},{"key":"retries","label":"Retries","type":"number"},{"key":"token","label":"Token","type":"secret"}]"#;
    let m = std::fs::read_to_string(root.join(id).join("plugin.json")).unwrap();
    std::fs::write(root.join(id).join("plugin.json"), m.replacen('{', &format!("{{{form},"), 1)).unwrap();
}

fn obj(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().unwrap().clone()
}

#[tokio::test]
async fn the_settings_form_reaches_the_plugin_and_secrets_only_when_declared() {
    let root = scratch("form");
    install_with_form(&root, "ex");
    let h = host(&root);
    let on = vec!["ex".to_string()];
    let info = &h.list(&on)[0];
    assert_eq!(info.fields.len(), 3);
    assert!(info.secrets_set.is_empty());

    h.set_values("ex", obj(serde_json::json!({"greeting": "hi", "retries": 2}))).await.unwrap();
    h.set_secret("ex", "token", "s3cret").await.unwrap();
    let info = &h.list(&on)[0];
    assert_eq!(info.values["greeting"], "hi");
    assert_eq!(info.secrets_set, ["token"], "the list says a secret is set, never what it is");
    assert!(!serde_json::to_string(info).unwrap().contains("s3cret"));

    let title = h.search_providers(&on)[0].search(&q()).await.unwrap()[0].title.clone();
    assert!(title.contains(r#"greeting="hi""#) && title.contains(r#"token="s3cret""#), "{title}");
    assert!(title.contains("other=null"), "a key the manifest doesn't declare as a secret is never handed out: {title}");

    // Changing a value restarts the plugin so it sees it, and the values survive a restart of the app.
    h.set_values("ex", obj(serde_json::json!({"greeting": "yo"}))).await.unwrap();
    assert!(!h.is_running("ex").await);
    let title = h.search_providers(&on)[0].search(&q()).await.unwrap()[0].title.clone();
    assert!(title.contains(r#"greeting="yo""#), "{title}");
    assert_eq!(host(&root).list(&on)[0].values["greeting"], "yo");

    h.set_secret("ex", "token", "").await.unwrap();
    assert!(h.list(&on)[0].secrets_set.is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn bad_settings_are_refused_with_a_plain_reason() {
    let root = scratch("badform");
    install_with_form(&root, "ex");
    let h = host(&root);
    assert!(h.set_values("ex", obj(serde_json::json!({"retries": "lots"}))).await.unwrap_err().contains("Retries"));
    assert!(h.set_values("ex", obj(serde_json::json!({"nope": "x"}))).await.is_err(), "unknown field");
    assert!(h.set_values("ex", obj(serde_json::json!({"token": "x"}))).await.is_err(), "secrets don't go through plain values");
    assert!(h.set_secret("ex", "greeting", "x").await.is_err(), "only secret fields take secrets");
    assert!(h.set_values("ghost", obj(serde_json::json!({}))).await.is_err());
    assert!(h.log("ghost").is_err());
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn the_log_view_shows_what_the_plugin_wrote_without_its_secrets() {
    let root = scratch("log");
    install_with_form(&root, "ex");
    let h = host(&root);
    assert_eq!(h.log("ex").unwrap(), "", "nothing yet");
    h.set_secret("ex", "token", "s3cret").await.unwrap();
    std::fs::create_dir_all(root.join("logs")).unwrap();
    std::fs::write(root.join("logs").join("ex.log"), "started\nusing token s3cret\n").unwrap();
    let log = h.log("ex").unwrap();
    assert!(log.contains("started") && !log.contains("s3cret") && log.contains("••••"), "{log}");
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn old_values_and_logs_never_outlive_a_field_change() {
    let root = scratch("stale");
    install_with_form(&root, "ex");
    let h = host(&root);
    let on = vec!["ex".to_string()];
    h.set_values("ex", obj(serde_json::json!({"greeting": "hi"}))).await.unwrap();
    h.set_secret("ex", "token", "abc123").await.unwrap();
    std::fs::create_dir_all(root.join("logs")).unwrap();
    std::fs::write(root.join("logs").join("ex.log"), "token=abc123\n").unwrap();
    h.set_secret("ex", "token", "").await.unwrap();
    assert_eq!(h.log("ex").unwrap(), "", "a removed secret can't be read back from the log");

    // The next version of the plugin turns "greeting" into a secret: its saved plain value must not leak.
    let m = std::fs::read_to_string(root.join("ex").join("plugin.json")).unwrap();
    std::fs::write(root.join("ex").join("plugin.json"), m.replace(r#""key":"greeting","label":"Greeting","type":"text""#, r#""key":"greeting","label":"Greeting","type":"secret""#)).unwrap();
    let h2 = host(&root);
    assert!(h2.list(&on)[0].values.is_empty());
    let title = h2.search_providers(&on)[0].search(&q()).await.unwrap()[0].title.clone();
    assert!(title.contains("greeting=null"), "not sent in initialize: {title}");
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn a_plugin_can_say_why_a_search_failed_and_label_its_own_rows() {
    for (mode, expect) in [("auth", ProviderError::Auth), ("network", ProviderError::Network)] {
        let root = scratch(mode);
        install(&root, "ex", mode, 1);
        let err = host(&root).search_providers(&["ex".into()])[0].search(&q()).await.unwrap_err();
        assert_eq!(err, expect, "{mode}");
        assert!(!err.to_string().contains("never shows"), "the plugin's own text is never shown");
        std::fs::remove_dir_all(root).ok();
    }
    let root = scratch("config");
    install(&root, "ex", "config", 1);
    assert!(matches!(host(&root).search_providers(&["ex".into()])[0].search(&q()).await.unwrap_err(), ProviderError::Config(_)));
    std::fs::remove_dir_all(root).ok();

    // Rows that carry a label keep it; the aggregator doesn't add the plugin's name on top.
    let root = scratch("labels");
    install(&root, "ex", "labels", 1);
    let ps = host(&root).search_providers(&["ex".into()]);
    let rep = drift_core::search::search_all(&ps, &q(), Duration::from_secs(3)).await;
    let by_title: Vec<_> = rep.results.iter().map(|r| (r.title.as_str(), r.sources.clone())).collect();
    assert!(by_title.contains(&("ubuntu (example one)", vec!["Indexer A".to_string()])), "{by_title:?}");
    assert!(by_title.contains(&("ubuntu (example two)", vec!["Indexer B".to_string()])), "{by_title:?}");
    std::fs::remove_dir_all(root).ok();
}

fn install_with_list(root: &Path, id: &str) {
    install(root, id, "", 1);
    let form = r#""settings":[{"key":"indexers","label":"Indexers","type":"list","item":[{"key":"name","label":"Name","type":"text"},{"key":"url","label":"Address","type":"url"},{"key":"apikey","label":"API key","type":"secret"},{"key":"on","label":"On","type":"bool"}]}]"#;
    let m = std::fs::read_to_string(root.join(id).join("plugin.json")).unwrap();
    std::fs::write(root.join(id).join("plugin.json"), m.replacen('{', &format!("{{{form},"), 1)).unwrap();
}

#[tokio::test]
async fn list_fields_keep_rows_and_their_secrets_together() {
    let root = scratch("list");
    install_with_list(&root, "ex");
    let h = host(&root);
    let on = vec!["ex".to_string()];
    assert!(h.list(&on)[0].problem.is_none());

    let rows = serde_json::json!({"indexers": [{"id": "a1", "name": "A", "url": "http://localhost:9117/api", "on": true}, {"id": "b2", "name": "B", "url": "", "on": false}]});
    h.set_values("ex", obj(rows)).await.unwrap();
    assert_eq!(h.list(&on)[0].values["indexers"][0]["name"], "A");

    // A secret needs a row that exists, and only a secret field of that list.
    assert!(h.set_secret("ex", "indexers.zz.apikey", "k").await.is_err(), "no such row");
    assert!(h.set_secret("ex", "indexers.a1.name", "k").await.is_err(), "not a secret field");
    assert!(h.set_secret("ex", "indexers.a1.apikey.extra", "k").await.is_err());
    h.set_secret("ex", "indexers.a1.apikey", "KEY-A").await.unwrap();
    h.set_secret("ex", "indexers.b2.apikey", "KEY-B").await.unwrap();
    let mut set = h.list(&on)[0].secrets_set.clone();
    set.sort();
    assert_eq!(set, ["indexers.a1.apikey", "indexers.b2.apikey"]);
    assert!(!serde_json::to_string(&h.list(&on)[0]).unwrap().contains("KEY-"), "values never leave the host");

    // Removing a row removes its secret; a new row with the same id doesn't inherit it.
    let a1 = "http://localhost:9117/api";
    h.set_values("ex", obj(serde_json::json!({"indexers": [{"id": "a1", "name": "A", "url": a1, "on": true}]}))).await.unwrap();
    assert_eq!(h.list(&on)[0].secrets_set, ["indexers.a1.apikey"]);
    h.set_values("ex", obj(serde_json::json!({"indexers": [{"id": "a1", "url": a1}, {"id": "b2"}]}))).await.unwrap();
    assert_eq!(h.list(&on)[0].secrets_set, ["indexers.a1.apikey"], "b2's old key is gone");

    // A key saved for one server is never sent to another: changing a row's address clears it.
    h.set_values("ex", obj(serde_json::json!({"indexers": [{"id": "a1", "url": "http://other.example/api"}]}))).await.unwrap();
    assert!(h.list(&on)[0].secrets_set.is_empty(), "the key stayed with the old address");

    // Bad rows are refused with the field's name.
    for bad in [
        serde_json::json!({"indexers": "not a list"}),
        serde_json::json!({"indexers": [{"name": "no id"}]}),
        serde_json::json!({"indexers": [{"id": "x y"}]}),
        serde_json::json!({"indexers": [{"id": "a"}, {"id": "a"}]}),
        serde_json::json!({"indexers": [{"id": "a", "apikey": "secrets don't go in rows"}]}),
        serde_json::json!({"indexers": [{"id": "a", "url": "ftp://x"}]}),
        serde_json::json!({"indexers": [{"id": "a", "extra": 1}]}),
    ] {
        assert!(h.set_values("ex", obj(bad.clone())).await.unwrap_err().contains("Indexers") || bad["indexers"].is_string(), "{bad}");
    }
    let too_many: Vec<_> = (0..51).map(|i| serde_json::json!({"id": format!("r{i}")})).collect();
    assert!(h.set_values("ex", obj(serde_json::json!({"indexers": too_many}))).await.is_err());
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn a_list_inside_a_list_or_a_list_without_fields_is_not_a_valid_form() {
    let root = scratch("badlist");
    install(&root, "nested", "", 1);
    install(&root, "empty", "", 1);
    let put = |id: &str, form: &str| {
        let m = std::fs::read_to_string(root.join(id).join("plugin.json")).unwrap();
        std::fs::write(root.join(id).join("plugin.json"), m.replacen('{', &format!("{{{form},"), 1)).unwrap();
    };
    put("nested", r#""settings":[{"key":"a","label":"A","type":"list","item":[{"key":"b","label":"B","type":"list","item":[{"key":"c","label":"C","type":"text"}]}]}]"#);
    put("empty", r#""settings":[{"key":"a","label":"A","type":"list","item":[]}]"#);
    let h = host(&root);
    assert!(h.list(&[]).iter().all(|p| p.problem.as_deref() == Some("Its settings form isn't valid.")));
    std::fs::remove_dir_all(root).ok();
}
