//! Reference simulator plugin (docs/PLUGIN_PROTOCOL.md, "Simulators"). It simulates nothing: it logs every method it
//! hears to stderr, remembers which torrents the user switched on, and reports them back from `get_selection`.
//! The first argument picks a misbehaviour for the host tests: `slow` (never answers `get_selection`), `crash`
//! (exits on `get_selection`).

use std::io::{BufRead, Write};

use serde_json::{json, Value};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut out = std::io::stdout();
    // info hash -> enabled
    let mut selected: Vec<(String, bool)> = Vec::new();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let method = req["method"].as_str().unwrap_or("");
        let params = &req["params"];
        match method {
            "torrents_updated" => eprintln!("torrents_updated: {} torrents", params["torrents"].as_array().map_or(0, Vec::len)),
            "selection_changed" => eprintln!("selection_changed: {} enabled={}", params["info_hash"], params["enabled"]),
            other => eprintln!("{other}"),
        }
        // A notification has no id and gets no reply.
        let id = req["id"].clone();
        if id.is_null() {
            continue;
        }
        let result = match method {
            "initialize" => json!({"kinds": ["simulator"], "name": "Example simulator", "version": "1.0.0"}),
            "get_selection" => {
                match mode.as_str() {
                    "slow" => std::thread::sleep(std::time::Duration::from_secs(3600)),
                    "crash" => std::process::exit(1),
                    _ => {}
                }
                let rows: Vec<Value> = selected.iter().map(|(h, on)| if *on { json!({"info_hash": h, "enabled": true, "label": "Simulating: example"}) } else { json!({"info_hash": h, "enabled": false}) }).collect();
                json!({"selections": rows})
            }
            "selection_changed" => {
                let hash = params["info_hash"].as_str().unwrap_or("").to_string();
                let on = params["enabled"].as_bool().unwrap_or(false);
                match selected.iter_mut().find(|(h, _)| *h == hash) {
                    Some(s) => s.1 = on,
                    None => selected.push((hash, on)),
                }
                json!({})
            }
            "shutdown" => {
                let _ = writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "result": {}}));
                break;
            }
            _ => continue,
        };
        let _ = writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "result": result}));
        let _ = out.flush();
    }
}
