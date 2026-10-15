//! The real plugin program, spoken to the way Drift speaks to it: JSON lines on stdin/stdout, with the plugin
//! asking for its secrets in the middle of a search.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;

use serde_json::{json, Value};

const EXE: &str = env!("CARGO_BIN_EXE_drift-search");

struct Plugin {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
    asked: Vec<String>,
}

impl Plugin {
    fn start(source: &str, env: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(EXE);
        cmd.arg(source).env_clear().stdin(Stdio::piped()).stdout(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        // The system root keeps TLS and sockets working on Windows, as Drift passes it too.
        if let Ok(root) = std::env::var("SystemRoot") {
            cmd.env("SystemRoot", root);
        }
        let mut child = cmd.spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Self { child, stdin, out, asked: Vec::new() }
    }

    /// Sends a request and returns the reply, answering `get_secret` requests from `secrets` on the way.
    fn call(&mut self, id: u64, method: &str, params: Value, secrets: &dyn Fn(&str) -> Option<&'static str>) -> Value {
        writeln!(self.stdin, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        self.stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "the plugin ended without answering {method}");
            let v: Value = serde_json::from_str(&line).unwrap();
            if v["method"] == "get_secret" {
                let key = v["params"]["key"].as_str().unwrap().to_string();
                writeln!(self.stdin, "{}", json!({"jsonrpc": "2.0", "id": v["id"], "result": {"value": secrets(&key)}})).unwrap();
                self.stdin.flush().unwrap();
                self.asked.push(key);
                continue;
            }
            if v["id"] == id {
                return v;
            }
        }
    }

    fn finish(mut self) {
        let r = self.call(99, "shutdown", json!({}), &|_| None);
        assert_eq!(r["result"], json!({}));
        assert!(self.child.wait().unwrap().success());
    }
}

/// Serves one request and tells the test what it was.
fn serve_once(status: &str, content_type: &str, body: String) -> (u16, mpsc::Receiver<String>) {
    let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    let status = status.to_string();
    let content_type = content_type.to_string();
    std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut buf = vec![0u8; 8192];
        let n = s.read(&mut buf).unwrap();
        let _ = tx.send(String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string());
        let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    });
    (port, rx)
}

const FEED: &str = r#"<?xml version="1.0"?><rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>
<item><title>Ubuntu 26.04</title><enclosure url="http://idx.local/1.torrent" length="100"/><torznab:attr name="seeders" value="7"/><torznab:attr name="infohash" value="0123456789abcdef0123456789abcdef01234567"/></item>
</channel></rss>"#;

#[test]
fn internet_archive_answers_a_search() {
    let body = r#"{"response":{"docs":[{"identifier":"sintel","title":"Sintel","item_size":"5","mediatype":"movies"}]}}"#;
    let (port, seen) = serve_once("200 OK", "application/json", body.to_string());
    let base = format!("http://127.0.0.1:{port}");
    let mut p = Plugin::start("archive_org", &[("DRIFT_TEST_ARCHIVE_BASE", &base)]);
    let init = p.call(1, "initialize", json!({"api": 1, "app_version": "0", "settings": {}}), &|_| None);
    assert_eq!(init["result"]["name"], "Internet Archive");
    let r = p.call(2, "search", json!({"query": "sintel", "limit": 5}), &|_| None);
    assert_eq!(r["result"]["results"][0]["title"], "Sintel");
    assert_eq!(r["result"]["results"][0]["torrent_url"], format!("{base}/download/sintel/sintel_archive.torrent"));
    assert!(seen.recv().unwrap().starts_with("GET /advancedsearch.php?"));
    assert!(p.asked.is_empty(), "this source has no secrets to ask for");
    p.finish();
}

#[test]
fn torznab_asks_for_each_indexers_key_and_labels_results() {
    let (good, seen) = serve_once("200 OK", "application/xml", FEED.to_string());
    let dead = {
        let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        l.local_addr().unwrap().port()
    };
    let settings = json!({"indexers": [
        {"id": "a1", "name": "Good one", "url": format!("http://127.0.0.1:{good}/api"), "enabled": true},
        {"id": "b2", "name": "Dead one", "url": format!("http://127.0.0.1:{dead}/api"), "enabled": true},
        {"id": "c3", "name": "Switched off", "url": "http://127.0.0.1:9/", "enabled": false},
        {"id": "d4", "name": "Bad address", "url": "not a url", "enabled": true}
    ]});
    let mut p = Plugin::start("torznab", &[]);
    p.call(1, "initialize", json!({"api": 1, "app_version": "0", "settings": settings}), &|_| None);
    let r = p.call(2, "search", json!({"query": "ubuntu", "limit": 10}), &|k| (k == "indexers.a1.apikey").then_some("KEY-A"));
    let rows = r["result"]["results"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{r}");
    assert_eq!((rows[0]["title"].as_str(), rows[0]["source"].as_str()), (Some("Ubuntu 26.04"), Some("Good one")));
    assert_eq!(rows[0]["seeders"], 7);
    assert!(seen.recv().unwrap().contains("apikey=KEY-A"), "the key the host handed over is sent to that indexer");
    assert!(!p.asked.contains(&"indexers.c3.apikey".to_string()), "a switched-off indexer is never asked about");
    assert!(p.asked.contains(&"indexers.a1.apikey".to_string()) && p.asked.contains(&"indexers.b2.apikey".to_string()));
    p.finish();
}

#[test]
fn torznab_reports_a_fixed_reason_when_it_cannot_answer() {
    let mut p = Plugin::start("torznab", &[]);
    p.call(1, "initialize", json!({"api": 1, "app_version": "0", "settings": {"indexers": []}}), &|_| None);
    let r = p.call(2, "search", json!({"query": "x", "limit": 5}), &|_| None);
    assert_eq!(r["error"]["data"]["reason"], "config", "no indexer set up yet");

    let (refused, _) = serve_once("401 Unauthorized", "text/plain", "no".into());
    let settings = json!({"indexers": [{"id": "a1", "name": "A", "url": format!("http://127.0.0.1:{refused}/"), "enabled": true}]});
    p.call(3, "initialize", json!({"api": 1, "app_version": "0", "settings": settings}), &|_| None);
    let r = p.call(4, "search", json!({"query": "x", "limit": 5}), &|_| Some("WRONG"));
    assert_eq!(r["error"]["data"]["reason"], "auth");
    assert!(!r.to_string().contains("WRONG") && !r.to_string().contains("127.0.0.1"), "no address or key in the reply: {r}");
    p.finish();
}

#[test]
fn a_request_that_arrives_while_waiting_for_a_secret_is_answered_afterwards() {
    let (good, _) = serve_once("200 OK", "application/xml", FEED.to_string());
    let settings = json!({"indexers": [{"id": "a1", "name": "A", "url": format!("http://127.0.0.1:{good}/api"), "enabled": true}]});
    let mut p = Plugin::start("torznab", &[]);
    p.call(1, "initialize", json!({"api": 1, "app_version": "0", "settings": settings}), &|_| None);
    writeln!(p.stdin, "{}", json!({"jsonrpc": "2.0", "id": 2, "method": "search", "params": {"query": "x", "limit": 5}})).unwrap();
    let mut line = String::new();
    p.out.read_line(&mut line).unwrap();
    let ask: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ask["method"], "get_secret");
    // The app gives up on the search and sends the next request before it answers the secret.
    writeln!(p.stdin, "{}", json!({"jsonrpc": "2.0", "id": 3, "method": "initialize", "params": {"api": 1, "app_version": "0", "settings": {"indexers": []}}})).unwrap();
    writeln!(p.stdin, "{}", json!({"jsonrpc": "2.0", "id": ask["id"], "result": {"value": "K"}})).unwrap();
    p.stdin.flush().unwrap();
    let mut replies = Vec::new();
    for _ in 0..2 {
        let mut l = String::new();
        p.out.read_line(&mut l).unwrap();
        replies.push(serde_json::from_str::<Value>(&l).unwrap());
    }
    assert!(replies.iter().any(|r| r["id"] == 2 && r["result"]["results"].is_array()), "the search still finished: {replies:?}");
    assert!(replies.iter().any(|r| r["id"] == 3 && r["result"]["name"] == "Torznab indexers"), "and the request that arrived meanwhile was answered, not swallowed: {replies:?}");
    p.finish();
}
