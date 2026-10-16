# tracker-sim: design

A Drift `simulator` plugin (`crates/drift-tracker-sim-plugin`, binary `tracker-sim`). For each torrent the user switches on, it announces to the torrent's trackers as stable, human-looking peers whose counters grow gradually. **Purpose: testing a tracker and its anti-bot checks that you operate or have permission to test.** It is not a way to inflate ratios on someone else's tracker; the manifest description says so. Protocol and the host side are in `docs/PLUGIN_PROTOCOL.md` ("Simulators"); this file is only the plugin.

Status: **Phase 1 is built** (announce-only). Phases 2-4 are designed here and not built.

## Where the brief and the real protocol differ

The brief was written against an assumed API; the repo's differs in these places, and the plugin follows the repo.

| Brief | Repo | Consequence |
|---|---|---|
| `"exec": "tracker-sim.exe"`, `"permissions": ["net","fs"]` | `exec` is an array; `permissions` is `{net:[...], fs:[...]}` | manifest uses `["tracker-sim.exe"]` and `{"net":["*"],"fs":[]}`. Windows-only `exec`, as for the other first-party plugins. |
| Settings type `list` with `options` (a dropdown) | `list` is a repeating-rows field; there is no dropdown | enum settings are `text` with the allowed values in the help text. Unknown values fall back to the default with a warning. |
| `torrents_updated` "notification or request" | always a notification, never answered | handled as a notification; answered only if an `id` is present. |
| `compact_peers=1` | the real parameter is `compact=1` | `compact=1` is sent. |
| Torrent has `torrent_file_path` | not sent today | not used; the plugin needs only `info_hash`, `total_size`, `trackers`. |
| Core change needed | `simulator` kind is already in core | no core change, and no allow-list fallback. |
| Settings for DHT, PEX, proxy, piece strategy | nothing implements them in Phase 1 | left out of the form until their phase (a setting that does nothing misleads). Listed per phase below. |

No core file was changed. Added outside the crate: the workspace member, `scripts/install-tracker-sim.ps1`, this file, and a row in `docs/PLUGINS.md`.

## Phase 1 behaviour

- **Nothing runs by default.** `torrents_updated` only fills a registry. A job starts when `selection_changed(enabled=true)` arrives, or when a torrent that was on before a restart shows up in the list.
- **Per torrent.** Job = N peer tasks (`peer_count`, default 1). Disabling one torrent signals only its job; each peer sends `event=stopped` (`numwant=0`) and saves its counters. A torrent that disappears from Drift's list stops too, but its switch is kept.
- **Quick replies.** The main thread only reads stdin and updates state; announces run on two tokio workers. `selection_changed` returns after spawning. `shutdown` stops all jobs and waits up to 5 s for their `stopped` announces.
- **Waiting for details.** A magnet without metadata has no size or trackers; its peers wait (5 s polls) rather than announce nonsense.
- **Trackers.** HTTP(S) only; every http tracker in the torrent is announced to in turn. `udp://` is skipped (Phase 2).

### Modes

| Mode | Phase 1 | Counters |
|---|---|---|
| `seed_only` | yes | `left=0`, `downloaded` stays 0, `uploaded` grows |
| `partial_seed` | synthetic | starts with `partial_availability_percent` of the size, downloads the rest, uploads in proportion to what it owns |
| `download_and_seed` | synthetic | starts at 0, `left` shrinks at the download rate; upload scales with the owned fraction; `event=completed` once when `left` reaches 0 |
| `loopback_pair` | **not built**; runs as `seed_only` with a warning | Phase 2 |

Selection (mode, peer count) is fixed when the torrent is switched on, from the settings at that moment, and saved; changing the defaults later affects only torrents switched on afterwards.

### Identity (`identity.rs`, `profiles.rs`)

Created once, saved in `state.json`, shared by all torrents (like one client): `identity_id` (`bot-001`...), `peer_id` (20 chars: client prefix + random body in that client's style), `key`, `port`, `client_profile`, `personality_seed`. `auto` picks qBittorrent/Transmission/Deluge by seed (50/30/20 %). The profile is kept per identity; changing the setting later does not change an existing peer. `debug_obvious` uses `-DRSIM0-` so the bots are easy to find in tracker logs; no other profile contains an obvious marker. The profile also sets User-Agent, `numwant` and port style. Peer number *i* of any torrent is identity *i*.

Personality (from the seed, so it survives restarts): rate scale, idle and burst probability, announce-interval factor. `stealth_level` scales jitter and how far personalities spread, and sets the chance of a stretched gap (basic 0, medium 2 %, high 5 %, maximum 8 %).

### Announce schedule (`peer.rs`)

Per peer, independent: a random first delay (5-100 % of `start_delay_max_ms`), then `base × personality factor × (1 ± jitter)`, sometimes ×1.5-3. Minimum 1 s. Optionally the tracker's own `interval` replaces the base (`honor_tracker_interval`; off by default because test trackers answer with 1800 s). Failure: 2 s, 4 s, 8 s ... up to 5 min, ±20 %; a completed event that failed is re-armed. No tracker known: retry in 10 s without counting a failure.

Event sequence: `started` (first success; optional), none for periodic, `completed` once, `stopped`. All values are cumulative.

### Rate engine (`rate.rs`)

Tick 200 ms. Profiles: `constant_jitter` (target near the middle, ±15 %, smoothed), `random_walk` (mean-reverting walk inside min/max), `bursty` (phases normal / burst / idle with random lengths; idle drops to 0, so bursty can go below min). A token bucket turns the rate into bytes: capped at 2 s of max rate, and at 1.5× max per tick. Counters only advance after the first successful announce (a client uploads only once it has announced). Ponytail ceiling: counters pause while a request is in flight; a time step is clamped to 0.5 s so a slow tracker can't cause a catch-up lump.

### State (`state.rs`)

`<data_dir or ./data>/state.json` (the plugin folder is the working directory): `{version, identities[], torrents{hash:{enabled,mode,peer_count,started_at}}, counters{"bot-001:<hash>":{uploaded_bytes,downloaded_bytes,have_bytes?,last_announce_time}}}`. Written to `state.json.tmp` then renamed. A corrupt file is renamed `state.json.corrupt` and the plugin starts empty. Counters are saved after every successful announce and on stop. `persist_counters=false` starts from zero each run (the peer still has the same identity).

### Tracker client (`tracker.rs`)

`GET <announce>?info_hash&peer_id&port&uploaded&downloaded&left&key[&event]&numwant&compact=1&no_peer_id=1&supportcrypto=1[&apikey=...][&extra_query]`. `info_hash` is percent-encoded raw bytes by default; `info_hash_encoding=hex` for trackers that want text. Existing query parts of the announce URL (passkeys) are preserved. API key: secret field `tracker_api_key`, fetched with `get_secret` inside `initialize` (never from a notification), sent as the parameter named by `api_key_param`. The response is parsed as bencode for `failure reason` (a failure) and `interval`. Errors and logs never print the full URL (it may hold a passkey).

Scrape (`scrape_enabled`): `/announce` becomes `/scrape` (BEP 48), GET with the info hash, result ignored. Ponytail: no per-profile scrape pattern yet.

### Errors

Every request gets a valid JSON reply (`-32602` for a bad hash, `-32601` for an unknown method); unparseable lines and replies are skipped. Bad settings fall back to defaults. Tracker failures retry with backoff and never reach the RPC loop. Known limit: the workspace release profile uses `panic = "abort"`, so a panic in a task would end the process (Drift restarts it: 2 s, 4 s, then off on the third). The code avoids panicking paths; there is no `unwrap` on input.

## Stealth criteria, and how each is met

1. Only selected torrents: default-off, tested (`selection_is_per_torrent_and_survives_restart`, integration test).
2. Disabling one keeps others: per-torrent `Job`, tested.
3. Stable peer ids across restarts: identities in `state.json`, tested.
4-5. Non-periodic, desynchronised announces: per-peer RNG, personality factor, first delay; tested.
6. Gradual counters: rate engine + clamp; tested at unit level and over HTTP.
7. Counters persist; 8. no obvious prefix outside `debug_obvious`; 9. started/periodic/stopped events; 10. no host failures in normal use (replies are immediate).

What Phase 1 cannot hide: it sends no peer traffic, so a tracker (or a swarm crawler) that connects to the announced port finds nothing there; all peers of one machine share its IP; and there is no real data behind `uploaded`. Phases 2 and 3 address these.

## Phase 2: real BitTorrent transfer (designed, not built)

New modules in the crate: `btp_peer.rs` (wire protocol), `piece_strategy.rs`, `metainfo.rs` (parse the `.torrent`; for magnets, fetch metadata via BEP 9, or require `torrent_file_path`, which core does not send yet, so a small core change would be listed separately).

- **Listener per identity** on its stored port (TCP). Announced port becomes honest.
- **Wire**: handshake (`\x13BitTorrent protocol`, reserved bits per profile, info hash, peer id), `bitfield`, `have`, `interested`/`not interested`, `choke`/`unchoke`, `request`/`piece`/`cancel`, keep-alive every 2 min; optional fast extension.
- **Data**: for `seed_only`, read real files from `content_dir`; otherwise synthesise deterministic pieces and verify SHA-1 against the metainfo (the test data are legal torrents only, per CLAUDE.md). `uploaded` counts payload bytes written to sockets, `downloaded` counts bytes of verified pieces; failed pieces count for neither; `left` = missing piece bytes.
- **Choking**: regular unchoke slots (4) re-evaluated every 10 s (Transmission) or 15 s (qBittorrent/Deluge), plus one optimistic unchoke rotated every 30 s. Values live in `Profile`.
- **Piece selection**: `sequential`, `random`, `rarest_first` (default; random among ties, random first piece), `endgame_bias` (duplicate requests + `cancel` in the last few pieces). Setting `piece_strategy`.
- **The rate engine stays**: it becomes the socket bandwidth limiter (token bucket per direction) instead of a counter generator, so real transfer has the same shape.
- **`loopback_pair`** first: two or more identities of one torrent in one process connect over 127.0.0.1, one seeding, one downloading; announces report real counters. This validates the wire code without external peers. A mode `partial_seed`/`download_and_seed` against real swarm peers follows.
- Behaviour engine change: `Peer` stops integrating rates itself and instead reads byte counts from the connection layer; the schedule, events and persistence code are unchanged.

New settings: `piece_strategy`, `listen_enabled`.

## Phase 3: network identity diversity (designed, not built)

The strongest bot fingerprint is many peers behind one IP. Module `proxy.rs`:

- `proxy_mode`: `none` / `shared` / `per_peer`. A proxy list (a `list` settings field, rows `id`, `url`, optional `username`, secret `password` via `<list>.<row>.password`) assigns proxy *i* to identity *i* and **persists the assignment** in `state.json` (a bot must not change IP between runs).
- **Tracker requests** go through the identity's proxy (reqwest supports `socks5h://` and `http://` proxies; one `Client` per identity instead of the shared one).
- **Peer connections** (Phase 2): outbound through SOCKS5 (CONNECT); inbound is not possible through a plain proxy, so listening peers need a SOCKS5 BIND/UDP-capable service, a VPN with port forwarding, or a bound interface.
- Optional `outbound_interface` (bind the local address of sockets), and a recipe for running several Drift/tracker-sim instances in separate containers/VMs, each with its own IP, which is the sturdiest option.
- The announce `ip=` parameter is not set (a real client doesn't); behind a proxy the tracker sees the proxy's address.

Done when: two identities announce from two different source addresses in the tracker's log, and an identity keeps its address after a restart.

## Phase 4: swarm realism (designed, not built)

DHT (BEP 5, own routing table per identity, announce_peer for the torrent), PEX (BEP 11, `ut_pex` through the extension handshake, profile-dependent flags), LSD (BEP 14 multicast; only useful on a LAN), client-specific scrape patterns and parameter ordering per profile, long-term personality drift (a human's rate preferences change over days), and daily patterns (idle at night). Settings `enable_dht`, `enable_pex`, `enable_lsd` arrive here.

## Tests

- Unit (in the crate): settings validation, rate bounds and burst limits, state round-trip and corruption, identity stability, peer counters in each mode, announce jitter and backoff, URL building, bencode answers, selection and resume logic, RPC parsing.
- Integration (`tests/protocol.rs`, real binary, stub HTTP tracker, ~20 s): initialize / `torrents_updated` / `get_selection` / `selection_changed` / `shutdown`; nothing announced before selection; `started` first with uploaded 0; cumulative, monotonically growing counters with no single announce carrying most of the total; uneven gaps; `stopped` with `numwant=0` on disable and on shutdown; peer id and counters continue after a restart.

## Manual test

1. Build and install (quit Drift first): `scripts\install-tracker-sim.ps1 -Data "<Drift data folder>"`. Folder name equals the id: `<data>\plugins\tracker-sim\{plugin.json,tracker-sim.exe}`.
2. Start Drift, Settings → Plugins → Tracker Simulator → turn on (check the permissions prompt). Suggested test settings: Announce interval `5000`, Longest first delay `3000`, Upload rate profile `random_walk`, Log level `debug`.
3. Run a tracker you control (any local one). A throwaway check without a real tracker: a local listener that logs requests (PowerShell: `[System.Net.HttpListener]` on `http://127.0.0.1:6969/`, answer body `d8:intervali1800e5:peers0:e`), and a test torrent whose announce URL is `http://127.0.0.1:6969/announce` (legal data only, e.g. a Linux ISO).
4. Add the torrent in Drift, right-click it → Simulator plugins → Tracker Simulator: the tick appears with "Simulating: seed_only, 1 peer".
5. Watch the tracker log for 60 s: first request has `event=started`, `uploaded=0`; later requests have growing `uploaded`, irregular gaps, no `event`. View log (Settings → Plugins) shows the `[DEBUG] announce ...` lines.
6. Untick: a request with `event=stopped&numwant=0`. A second torrent ticked meanwhile is unaffected.
7. Quit and restart Drift: the tick is still on; the first request has the same `peer_id` and `uploaded` ≥ the last value seen.
8. Automated equivalent: `cargo test -p drift-tracker-sim-plugin`.
