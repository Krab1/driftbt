//! Reference search plugin: the smallest thing that speaks protocol v1 (docs/PLUGIN_PROTOCOL.md). It answers every
//! search with two fixed results. The first argument picks a misbehaviour so the host can be tested against it:
//! `crash` (exits on the first search), `hang` (never answers a search), `noise` (junk before each reply).

use std::io::{BufRead, Write};

use serde_json::{json, Value};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let id = req["id"].clone();
        let result = match req["method"].as_str() {
            Some("initialize") => json!({"kinds": ["search"], "name": "Example", "version": "1.0.0"}),
            Some("search") => {
                match mode.as_str() {
                    "crash" => std::process::exit(1),
                    "hang" => std::thread::sleep(std::time::Duration::from_secs(3600)),
                    _ => {}
                }
                let q = req["params"]["query"].as_str().unwrap_or("");
                json!({"results": [
                    {"title": format!("{q} (example one)"), "info_hash": "0123456789abcdef0123456789abcdef01234567", "size": 1048576, "seeders": 5},
                    {"title": format!("{q} (example two)"), "magnet": "magnet:?xt=urn:btih:89abcdef0123456789abcdef0123456789abcdef"}
                ]})
            }
            Some("shutdown") => {
                let _ = writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "result": {}}));
                break;
            }
            _ => continue,
        };
        if mode == "noise" {
            let _ = writeln!(out, "{}", json!({"jsonrpc": "2.0", "method": "log", "params": {"level": "info", "message": "hi"}}));
        }
        let _ = writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "result": result}));
        let _ = out.flush();
    }
}
