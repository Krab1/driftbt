//! Drift's first-party search sources as one plugin program (protocol: docs/PLUGIN_PROTOCOL.md). The plugin folders
//! `archive_org` and `torznab` both hold this program; the first argument says which source it is. It keeps a
//! single thread and no state beyond the settings it was given, so it stays small and quick to start.

mod archive_org;
mod row;
mod torznab;

use std::collections::VecDeque;
use std::io::{BufRead, Write};

use serde_json::{json, Value};

use row::{Reason, Row};

fn ok(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failed(id: &Value, reason: Reason) -> Value {
    // The text is for logs only; Drift shows its own wording for the reason.
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": reason.word(), "data": {"reason": reason.word()}}})
}

/// Asks Drift for one of this plugin's secrets and waits for the answer. Anything else that arrives meanwhile
/// (the app may already have moved on to its next request) is kept in `later` for the main loop, not thrown away.
fn ask_secret(out: &mut impl Write, lines: &mut impl Iterator<Item = std::io::Result<String>>, later: &mut VecDeque<String>, key: &str) -> String {
    let _ = writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": 900, "method": "get_secret", "params": {"key": key}}));
    let _ = out.flush();
    while let Some(Ok(line)) = lines.next() {
        match serde_json::from_str::<Value>(&line) {
            Ok(v) if v["id"] == 900 && v.get("method").is_none() => return v["result"]["value"].as_str().unwrap_or("").to_string(),
            _ => later.push_back(line),
        }
    }
    String::new()
}

fn search(source: &str, rt: &tokio::runtime::Runtime, settings: &Value, text: &str, limit: usize, secret: &mut dyn FnMut(&str) -> String) -> Result<Vec<Row>, Reason> {
    match source {
        "archive_org" => {
            // Only tests point this somewhere else; Drift starts plugins with an empty environment.
            let a = match std::env::var("DRIFT_TEST_ARCHIVE_BASE") {
                Ok(base) => archive_org::ArchiveOrg::with_base(&base),
                Err(_) => archive_org::ArchiveOrg::new(),
            };
            rt.block_on(a.search(text, limit))
        }
        _ => {
            let mut indexers = Vec::new();
            for r in settings["indexers"].as_array().into_iter().flatten().filter(|r| r["enabled"].as_bool().unwrap_or(true)) {
                let Some(id) = r["id"].as_str() else { continue };
                let name = r["name"].as_str().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(id);
                let key = secret(&format!("indexers.{id}.apikey"));
                // An indexer with a bad address is left out; the rest still answer.
                if let Ok(i) = torznab::Indexer::new(name, r["url"].as_str().unwrap_or(""), &key) {
                    indexers.push(i);
                }
            }
            if indexers.is_empty() {
                return Err(Reason::Config);
            }
            rt.block_on(torznab::search_all(indexers, text, limit))
        }
    }
}

fn main() {
    let source = std::env::args().nth(1).unwrap_or_default();
    let name = match source.as_str() {
        "archive_org" => "Internet Archive",
        "torznab" => "Torznab indexers",
        _ => {
            eprintln!("usage: drift-search archive_org|torznab");
            std::process::exit(2);
        }
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime always builds");
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let mut out = std::io::stdout();
    let mut settings = Value::Null;
    let mut later: VecDeque<String> = VecDeque::new();
    while let Some(line) = later.pop_front().or_else(|| lines.next().and_then(Result::ok)) {
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let id = req["id"].clone();
        let reply = match req["method"].as_str() {
            Some("initialize") => {
                settings = req["params"]["settings"].clone();
                ok(&id, json!({"kinds": ["search"], "name": name, "version": env!("CARGO_PKG_VERSION")}))
            }
            Some("search") => {
                let text = req["params"]["query"].as_str().unwrap_or("").to_string();
                let limit = req["params"]["limit"].as_u64().unwrap_or(50) as usize;
                let mut secret = |key: &str| ask_secret(&mut out, &mut lines, &mut later, key);
                match search(&source, &rt, &settings, &text, limit, &mut secret) {
                    Ok(rows) => ok(&id, json!({"results": rows})),
                    Err(reason) => failed(&id, reason),
                }
            }
            Some("shutdown") => {
                let _ = writeln!(out, "{}", ok(&id, json!({})));
                break;
            }
            _ => continue,
        };
        let _ = writeln!(out, "{reply}");
        let _ = out.flush();
    }
}
