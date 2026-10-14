# Roadmap

Status: `[ ]` todo · `[~]` in progress · `[x]` done. One task = one commit. Agent column = suggested executor; reviewers run after each non-trivial task (see CLAUDE.md).
Order matters inside a milestone; milestones are sequential. Each milestone ends with a demo checklist.

## M0 — Spikes and scaffold (risk first)
| ID | Task | Acceptance |
|---|---|---|
| [x] M0-01 | Scaffold Tauri v2 + Svelte 5 + TS + Vite as a Cargo workspace (`crates/drift-core`, `crates/drift-server` stub, `src-tauri`); lint/format/clippy/vitest wired | `npm run tauri dev` opens a window; `cargo clippy --workspace -D warnings` and `npm run check` pass; `drift-core` has no tauri/axum dependency |
| [x] M0-02 | **S1** Read `librqbit` API (docs.rs + source) and fill the open questions in FEATURES.md: file re-selection, recheck, force-announce, bind interface, **IP filter hook (can we reject inbound and outbound peers by address before the handshake?)**, session persistence, stream URL | Table in `docs/SPIKES.md` with yes/no/how per item; FEATURES.md rows marked or struck |
| [ ] M0-03 | **S2** Encryption/uTP spike: download a popular distro ISO, record peer count and speed; compare with a reference client on the same machine | Numbers in SPIKES.md; decision in DECISIONS.md |
| [ ] M0-04 | **S3** Windows I/O spike: 6 GB ISO to a normal drive, with Defender on; note hash-check time and CPU | Numbers in SPIKES.md |
| [ ] M0-05 | Define `TorrentEngine`, models, `FakeEngine` | Compiles; `core` has zero imports of tauri/librqbit (grep check in CI script) |
| [ ] M0-06 | `rqbit.rs` adapter: add (magnet/file), list, pause/resume/remove | Integration test adds a loopback test torrent and completes it |

Demo: terminal-level proof the engine works through our trait.

## M1 — Walking skeleton
| ID | Task | Acceptance |
|---|---|---|
| [ ] M1-01 | Ticker + `snapshot` event + `src/lib/api` typed wrapper | Frontend logs a live list from the fake engine at 1 Hz |
| [ ] M1-02 | Design tokens, themes, custom title bar, layout shell (rail, list, status bar) | Matches UI_SPEC layout at 3 sizes; both themes; measured |
| [ ] M1-03 | Virtualised torrent list, row component, sort, filter rail with counts, search | 1,000 fake torrents scroll at 60 fps; keyboard navigation works |
| [ ] M1-04 | Add flow: paste magnet, drop file, Add dialog (name, size, files, folder) | Real ISO adds and downloads end to end |
| [ ] M1-05 | Pause/resume/remove with confirm; row hover actions; context menu | Works on real torrent; delete-files off by default |
| [ ] M1-06 | Settings store (JSON, versioned), download folder, theme setting | Survives restart; migration test |
| [ ] M1-07 | Session persistence | Kill the process mid-download; list and progress restored |

Demo: add, watch, pause, remove a real torrent in the real UI.

## M2 — Daily-usable
| ID | Task | Acceptance |
|---|---|---|
| [ ] M2-01 | Detail panel: overview, files (tree + checkboxes), peers, trackers | Live at 2 Hz, only for the selected torrent; file select changes the engine |
| [ ] M2-02 | Speed limits + status bar toggles | Measured throttling within ±10% |
| [ ] M2-03 | Single instance, magnet and .torrent handler registration, forward to running app | Click a magnet link in a browser; the app adds it |
| [ ] M2-04 | Tray, close-to-tray, autostart option, snapshot throttling when hidden | Hidden idle CPU ≈ 0; RAM < 80 MB |
| [ ] M2-05 | Completion notification + open folder; error surfacing in plain language | Disk-full and bad-path cases produce the copy in UI_SPEC |
| [ ] M2-06 | All UI_SPEC states implemented (empty, loading metadata, checking, error…) | Screenshot per state attached |

Demo = **MVP**: replaces your current client for a week of normal use.

## M3 — Polish pass
| ID | Task | Acceptance |
|---|---|---|
| [ ] M3-01 | Accessibility pass (`design:accessibility-review`) | Keyboard-only run-through; contrast measured |
| [ ] M3-02 | Design critique pass (`design:design-critique`, `web-design-guidelines`) | Findings fixed or logged |
| [ ] M3-03 | Performance pass against ARCHITECTURE budgets | Numbers recorded in PERF.md |
| [ ] M3-04 | Installer (MSI/NSIS), icon, app name decision | Install, upgrade, and uninstall tested on a clean user profile |

## M4 — Power features
| ID | Task | Acceptance |
|---|---|---|
| [ ] M4-01 | `core::queue`: max active, priority, drag reorder | Unit tests with FakeEngine (ordering, slot release, paused/completed) |
| [ ] M4-02 | Categories + tags + default paths + move on complete | Sidebar groups; file moved after completion; test for path rules |
| [ ] M4-03 | `core::rules`: stop at ratio / seed time | Unit tests; UI shows goal and remaining |
| [ ] M4-04 | Play while downloading (stream URL → default player) | Video starts before 5% downloaded on a test file |
| [ ] M4-05 | Command palette (Ctrl+K) + shortcuts | All main actions reachable without the mouse |
| [ ] M4-06 | Speed graph (status bar + detail), `dataviz` rules | Light and dark checked |
| [ ] M4-07 | `core::playlist` + context menu + auto-on-complete setting | Unit tests: natural sort (`E2` < `E10`), non-media skipped, relative UTF-8 paths; playlist opens in a player |
| [ ] M4-08 | Ratio assistant: per-tracker true totals, min seed time, "seed where it helps" priority (needs peer/swarm counts from the engine) | Unit tests on the priority function with FakeEngine swarm data; totals match engine counters exactly |

## M5 — Network and automation
| ID | Task | Acceptance |
|---|---|---|
| [ ] M5-01 | Bind interface + VPN guard (kill switch) | Disabling the adapter pauses all within 3 s; banner shown |
| [ ] M5-02 | SOCKS5 proxy, port settings, UPnP toggle, connectivity test | Test result shown in Settings |
| [ ] M5-03 | Alt-speed weekly schedule | Unit tests for schedule evaluation incl. DST |
| [ ] M5-04 | Watch folder | Dropping a .torrent adds it once; no duplicates |
| [ ] M5-05 | Engine IP-filter hook: `TorrentEngine::set_ip_filter` implemented in `rqbit.rs`. librqbit has blocklist/allowlist but loads them once at session start (see SPIKES S1): apply via a normalized `file://` list and rebuild the session on update, or upstream an ArcSwap patch (record in DECISIONS.md) | Integration test: a loopback peer inside a blocked range never completes a handshake, in both directions; unblocked peer still works |
| [ ] M5-07 | `core::ipfilter`: parsers (eMule `ipfilter.dat`, PeerGuardian text, plain CIDR/ranges; IPv4, IPv6 ranges ignored with a count), sorted merged `RangeSet`, O(log n) lookup | Unit tests: malformed lines skipped and counted, overlapping ranges merge, boundaries inclusive, 200k-range list loads in < 200 ms and < 10 MB |
| [ ] M5-08 | Subscriptions: HTTP(S) fetch with conditional GET (ETag/Last-Modified), zip/gz extraction with size caps, schedule, last-good list kept on disk | Zip bomb and oversized download rejected; failed or empty update keeps the previous list; a list covering > 25% of IPv4 space is rejected with a warning; works offline |
| [ ] M5-09 | IP filter UI: Settings → Network → IP filter (subscriptions, toggle, add URL, import file, update now, ranges count, last updated) + blocked counter in status bar | All states from UI_SPEC; the emule-security preset appears but is off by default |
| [ ] M5-10 | Peer table marks nothing for blocked peers; detail panel shows "Blocked N peers this session" | Counter increments in the loopback test |
| [ ] M5-06 | Auto-update (opt-in), release pipeline | Update from N to N+1 on a clean machine |

## M6 — Search and headless (can move ahead of M5 if the owner prefers)
| ID | Task | Acceptance |
|---|---|---|
| [ ] M6-01 | `SearchProvider` trait, `Aggregator`, fake provider | Unit tests: parallel fan-out, per-provider timeout, one failing provider doesn't fail the search, info-hash de-dup merges seeders and badges, ranking |
| [ ] M6-02 | `torznab` provider + credential storage (Credential Manager) | Works against a local fake Torznab server in an integration test; API key never appears in the settings JSON or logs |
| [ ] M6-03 | `archive_org` provider | Returns torrent results for a known public-domain query; handles empty and error responses |
| [ ] M6-04 | Search UI (mode toggle, results list, source badges, status strip, add flow) | All search states from UI_SPEC implemented; screenshot per state; keyboard-only flow works |
| [ ] M6-05 | Settings → Search: provider list, add/test/toggle Torznab | Bad URL/key shows a plain-language error; first-run empty state shown |
| [ ] M6-06 | `drift-server`: engine + queue + rules wired, HTTP API with token auth, localhost default | `curl` can add, list, pause, remove, and search; requests without a token get 401; non-localhost bind refuses to start without a token |
| [ ] M6-07 | Headless watch folder + data-dir lock file | Drop a .torrent and it adds once; second instance on the same data dir refuses to start (desktop or server) |
| [ ] M6-08 | Headless docs + example run as a Windows scheduled task / Linux systemd unit | `docs/HEADLESS.md`, tested start/stop on a clean profile |
| [ ] M6-09 | Remote web UI: `src/lib/api` gets an HTTP + event-stream transport; `drift-server` serves the built UI; token login page | Same UI loads at `http://127.0.0.1:<port>` and works from a second device on the LAN with a token; no token = 401; optional TLS cert path in config |

## M7 — Plugins (needs M6; read `docs/PLUGINS.md` first)
| ID | Task | Acceptance |
|---|---|---|
| [ ] M7-01 | Plugin host: manifest parsing, discovery, lifecycle (lazy start, idle kill, timeout, crash backoff, Job Object memory cap) | Tested with a fake plugin binary: handshake, timeout, crash 3× → disabled, idle shutdown, `api` mismatch refused |
| [ ] M7-02 | Protocol v1 doc + `ExternalSearchProvider` adapter + reference plugin | Reference plugin returns results through the aggregator; plugin crash never fails the search; protocol documented in `docs/PLUGIN_PROTOCOL.md` |
| [ ] M7-03 | Settings → Plugins UI: list, enable/disable, version, permissions prompt, generic settings form from `settings_schema`, per-plugin log view | Secret fields stored via Credential Manager; permission prompt shown before first enable |
| [ ] M7-04 | Registry client + install/update/rollback (hash + Ed25519 signature, atomic swap, opt-in) | Tampered archive rejected; failed update leaves previous version working; rollback works; no network call unless opted in |
| [ ] M7-05 | Migrate `torznab` and `archive_org` to bundled first-party plugins; delete the in-tree providers | Search behaves identically in M6-04 UI tests; installer contains the plugins; update of one plugin works without an app release |
| [ ] M7-06 | `postprocess` kind: completion event → plugin → actions (rename/move/notify) | Example plugin renames a completed file; failure shows a toast and never blocks seeding |
| [ ] M7-07 | Theme packs (data-only JSON tokens) | Import a pack; invalid pack rejected with a plain message; contrast check warns below 4.5:1 |

## Working agreements for agents
- Pick the first `[ ]` task whose dependencies are done; mark `[~]` before you start.
- If an acceptance criterion cannot be met, stop and write why in the task row; do not weaken it silently.
- Spikes produce numbers or yes/no answers in `docs/SPIKES.md`, not code to keep.
- Parallelisable: M1-02/M1-03 (UI) with M0-05/M0-06 (engine), once the contract is merged.
- Never ship a task without running the app.
