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
