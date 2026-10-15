//! Reference search plugin: the smallest thing that speaks protocol v1 (docs/PLUGIN_PROTOCOL.md). It answers every
//! search with two fixed results. The first argument picks a misbehaviour so the host can be tested against it:
//! `crash` (exits on the first search), `hang` (never answers a search), `noise` (junk before each reply), `settings` (puts its settings and secrets into the result titles, to test the form), `auth` / `network` / `config` (answer a search with that error reason), `labels` (tags each row with its own `source`). As a post-processing plugin it answers `completed` with a notice, plus a move when the argument is `move=<folder>`; `crash` exits instead, and `junk` asks for things Drift must refuse.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut lines = stdin.lock().lines();
    let mut settings = Value::Null;
    while let Some(Ok(line)) = lines.next() {
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let id = req["id"].clone();
        let result = match req["method"].as_str() {
            Some("initialize") => {
                settings = req["params"]["settings"].clone();
                json!({"kinds": ["search"], "name": "Example", "version": "1.0.0"})
            }
            Some("search") => {
                match mode.as_str() {
                    "crash" => std::process::exit(1),
                    "hang" => std::thread::sleep(std::time::Duration::from_secs(3600)),
                    "auth" | "network" | "config" => {
                        let _ = writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "a message the app never shows", "data": {"reason": mode}}}));
                        let _ = out.flush();
                        continue;
                    }
                    _ => {}
                }
                let mut q = req["params"]["query"].as_str().unwrap_or("").to_string();
                if mode == "settings" {
                    // Asks the host for two secrets, the way a real plugin would, and shows what came back.
                    let mut ask = |key: &str| {
                        let _ = writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": 900, "method": "get_secret", "params": {"key": key}}));
                        let _ = out.flush();
                        let reply: Value = lines.next().and_then(|l| l.ok()).and_then(|l| serde_json::from_str(&l).ok()).unwrap_or(Value::Null);
                        reply["result"]["value"].clone()
                    };
                    let (token, other) = (ask("token"), ask("undeclared"));
                    q = format!("{q} greeting={} token={token} other={other}", settings["greeting"]);
                }
                let tag = |row: Value, label: &str| if mode == "labels" { let mut r = row; r["source"] = json!(label); r } else { row };
                json!({"results": [
                    tag(json!({"title": format!("{q} (example one)"), "info_hash": "0123456789abcdef0123456789abcdef01234567", "size": 1048576, "seeders": 5}), "Indexer A"),
                    tag(json!({"title": format!("{q} (example two)"), "magnet": "magnet:?xt=urn:btih:89abcdef0123456789abcdef0123456789abcdef"}), "Indexer B")
                ]})
            }
            Some("completed") => {
                if mode == "crash" {
                    std::process::exit(1);
                }
                let name = req["params"]["name"].as_str().unwrap_or("");
                let mut actions = vec![json!({"type": "notify", "message": format!("saw {name}")})];
                if let Some(dest) = mode.strip_prefix("move=") {
                    actions.push(json!({"type": "move", "dest": dest}));
                }
                if mode == "junk" {
                    actions = vec![json!({"type": "move", "dest": "relative/dir"}), json!({"type": "delete_everything"}), json!({"type": "notify", "message": "  "})];
                }
                json!({"actions": actions})
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
