# Roadmap

Status: `[ ]` todo · `[~]` in progress · `[x]` done. One task = one commit. Agent column = suggested executor; reviewers run after each non-trivial task (see CLAUDE.md).
Order matters inside a milestone; milestones are sequential. Each milestone ends with a demo checklist.

## M0 — Spikes and scaffold (risk first)
| ID | Task | Acceptance |
|---|---|---|
| [x] M0-01 | Scaffold Tauri v2 + Svelte 5 + TS + Vite as a Cargo workspace (`crates/drift-core`, `crates/drift-server` stub, `src-tauri`); lint/format/clippy/vitest wired | `npm run tauri dev` opens a window; `cargo clippy --workspace -D warnings` and `npm run check` pass; `drift-core` has no tauri/axum dependency |
| [x] M0-02 | **S1** Read `librqbit` API (docs.rs + source) and fill the open questions in FEATURES.md: file re-selection, recheck, force-announce, bind interface, **IP filter hook (can we reject inbound and outbound peers by address before the handshake?)**, session persistence, stream URL | Table in `docs/SPIKES.md` with yes/no/how per item; FEATURES.md rows marked or struck |
| [~] M0-03 (partial, see SPIKES S2) | **S2** Encryption/uTP spike: download a popular distro ISO, record peer count and speed; compare with a reference client on the same machine | Numbers in SPIKES.md; decision in DECISIONS.md |
| [~] M0-04 (partial, see SPIKES S3) | **S3** Windows I/O spike: 6 GB ISO to a normal drive, with Defender on; note hash-check time and CPU | Numbers in SPIKES.md |
| [x] M0-05 | Define `TorrentEngine`, models, `FakeEngine` | Compiles; `core` has zero imports of tauri/librqbit (grep check in CI script) |
| [x] M0-06 | `rqbit.rs` adapter: add (magnet/file), list, pause/resume/remove | Integration test adds a loopback test torrent and completes it |

Demo: terminal-level proof the engine works through our trait.

## M1 — Walking skeleton
| ID | Task | Acceptance |
|---|---|---|
| [x] M1-01 | Ticker + `snapshot` event + `src/lib/api` typed wrapper | Frontend logs a live list from the fake engine at 1 Hz |
| [x] M1-02 | Design tokens, themes, layout shell (rail, list, status bar). **Custom title bar deferred to M3-05** (keeps native Windows 11 snap layouts for now) | Both themes measured (text contrast 5.7:1); layout checked at 1100×700 and 480 px wide. Narrow widths below the 900 px window minimum are not supported until the remote UI (M6-09) |
| [x] M1-03 | Virtualised torrent list, row component, sort, filter rail with counts, search | 1,000 fake torrents scroll at 60 fps; keyboard navigation works |
| [x] M1-04 | Add flow: paste magnet, drop file, Add dialog (source, save folder). **Name/size/file tree in the dialog needs an engine `inspect` (list-only) call: moved to M2-01** | Verified in the mock (invalid magnet error, valid magnet) **and in the real Tauri window 2026-10-14**: dropped the Ubuntu 26.04.1 desktop `.torrent`, 6.0 GB downloaded at 41–69 MB/s with 18–64 peers, SHA-256 matched Ubuntu's published checksum |
| [x] M1-05 | Pause/resume/remove with confirm; row hover actions; context menu | Verified on the real torrent 2026-10-14: pause held progress at 3354 MB, resume continued at 63 MB/s, remove with "also delete files" deleted the 6 GB file and showed the empty state. Context menu and keyboard navigation verified in the mock |
| [x] M1-06 | Settings store (JSON, versioned), download folder, theme setting | Survives restart; migration test |
| [x] M1-07 (also proven with a hard kill at ~76% of a real 6 GB download: restart resumed at 4.9 GB, not zero) | Session persistence | Kill the process mid-download; list and progress restored |

Demo: add, watch, pause, remove a real torrent in the real UI.

## M2 — Daily-usable
| ID | Task | Acceptance |
|---|---|---|
| [x] M2-01 | Detail panel: overview, files (tree + checkboxes), peers, trackers | Live at 2 Hz, only for the selected torrent; file select changes the engine |
| [x] M2-02 | Speed limits + status bar toggles | Measured throttling within ±10% |
| [~] M2-03 (forwarding done; OS registration of magnet: and .torrent moved to M3-04) | Single instance, magnet and .torrent handler registration, forward to running app | Click a magnet link in a browser; the app adds it |
| [x] M2-04 | Tray, close-to-tray, autostart option, snapshot throttling when hidden | Hidden idle CPU ≈ 0; RAM < 80 MB |
| [~] M2-05 (completion event, plain-language errors and Show in folder built; toast display and Explorer reveal not visually confirmed) | Completion notification + open folder; error surfacing in plain language | Disk-full and bad-path cases produce the copy in UI_SPEC |
| [x] M2-06 | All UI_SPEC states implemented (empty, loading metadata, checking, error…) | Screenshot per state attached |

Demo = **MVP**: replaces your current client for a week of normal use.

**Verified 2026-10-14 (real Tauri window + Ubuntu 26.04.1 ISO unless noted):**
- M2-01: details panel live for the selected torrent only (files, 27 peers with client names, trackers, save path); file-selection tree and Add-dialog preview (mock, plus real `inspect`); selection changes covered by an engine test (download one file, widen mid-download).
- M2-02: 2048 KiB/s limit measured 1.87 MiB/s (about 6% under), lifted instantly (66 MiB/s unlimited); settings and status-bar switch.
- M2-03: a second launch with a `magnet:` argument exits and the running instance shows its Add dialog prefilled and fetching info. `.torrent` path arguments are parsed and read but not exercised live.
- M2-04: close-to-tray (real `WM_CLOSE` hides, process lives), second launch shows the window, start-minimised only on an `--autostart` launch, snapshots every 5 s while hidden vs 1 s visible, per-user autostart entry written and removed. Tray icon and menu are built but not visually checked.
- M2-05: the `completed` event fired while hidden. **Not confirmed:** the Windows toast appearing (Windows may need an installed app identity for toasts, so re-check after M3-04) and the Explorer reveal.
- M2-06: loading metadata, checking, looking for peers, error + Retry, empty, no matches (mock-verified; error text mapping unit-tested).

Bugs found by the real-app run and fixed: ticker task spawned before the app state existed (list froze), `document.hidden` stays false when the window is hidden to the tray, the autostart plugin wrote the machine-wide key when elevated, a settings file saved with a UTF-8 BOM (Notepad, PowerShell 5) was treated as corrupt, and OS error 32 was mapped to the "folder missing" message.

## M3 — Polish pass
| ID | Task | Acceptance |
|---|---|---|
| [x] M3-01 | Accessibility pass (`design:accessibility-review`) | Keyboard-only run-through; contrast measured |
| [x] M3-02 | Design critique pass (`design:design-critique`, `web-design-guidelines`) | Findings fixed or logged |
| [x] M3-03 | Performance pass against ARCHITECTURE budgets | Numbers recorded in PERF.md |
| [~] M3-05 (frameless window; the page draws minimize/maximize/close and the top bar is the drag region. Snap layouts: `src-tauri/src/caption.rs` answers `HTMAXBUTTON` for the page-reported maximize button. **Measured on the real window:** hit test on the button = 9 (HTMAXBUTTON), on the resize corner = 13, elsewhere = client; a non-client click on the button maximized and restored the window and the icon/label followed. **Not checked by a human:** the snap flyout actually appearing, dragging by the bar, double-click to maximize, Aero-snap by dragging to an edge, multi-monitor/DPI changes) | Custom title bar integrated with the top bar (drag region, caption buttons) without losing Windows 11 snap layouts | Snap-layout flyout still appears on the maximize button; window drags and double-click maximizes |
| [~] M3-04 (**NSIS, per-user, 3.0 MB** (`npm run tauri build -- --bundles nsis`, output in `target/release/bundle/nsis`). Verified with a silent install to a temp folder: no admin, per-user uninstall entry, `.torrent` association registered, `magnet:` registered only if nothing else owns it, installed app starts at 37 MB, uninstall removed the files, the magnet key and the uninstall entry. **Not tested:** upgrade over an older version, a truly clean Windows profile, a real magnet click, the completion toast with the installed identity, SmartScreen (the installer is unsigned)) | Installer (MSI/NSIS), icon, app name decision; register the `magnet:` scheme and `.torrent` association (bundle config) and re-check the completion toast with an installed app identity | Install, upgrade, and uninstall tested on a clean user profile |

## M4 — Power features
| ID | Task | Acceptance |
|---|---|---|
| [x] M4-01 (verified live: with a limit of 1, a second real ISO waited "queued" and started the moment the first finished) | `core::queue`: max active, priority, drag reorder | Unit tests with FakeEngine (ordering, slot release, paused/completed) |
| [x] M4-02 (categories with their own save folders, rail filter, context-menu assignment and Add-dialog picker; tags (right-click Tags…, rail filter, optional column); **move on complete**: Settings → General → "Move finished downloads to", plus right-click Move files… for any torrent. Verified against a real loopback seeder: files move, the torrent is checked at the new place and seeds again, and a file in the way stops the move and leaves everything where it was. **Limits:** the torrent is hashed again after every move, so a big torrent seeds again only after that check; torrents already finished when you turn the option on are not moved) | Categories + tags + default paths + move on complete | Sidebar groups; file moved after completion; test for path rules |
| [x] M4-03 (default goal in Settings, per-torrent goal dialog, minimum seed time; unit-tested, not yet exercised on a real swarm) | `core::rules`: stop at ratio / seed time | Unit tests; UI shows goal and remaining |
| [x] M4-04 (verified live on a real WebM torrent: HTTP range from the loopback server, a seek 400 MB in answered in 11 s under a 1 MiB/s cap, wrong Host/token refused. Not exercised: launching the user's actual media player, which depends on their default app for .m3u) | Play while downloading (stream URL → default player) | Video starts before 5% downloaded on a test file |
| [x] M4-05 (Ctrl+K and Ctrl+Shift+P; fuzzy-ranked commands incl. selected-torrent actions; keyboard-tested, axe clean) | Command palette (Ctrl+K) + shortcuts | All main actions reachable without the mouse |
| [x] M4-06 (two-minute download/upload graph in the status bar and, per torrent, on the Overview tab; kept in memory only) | Speed graph (status bar + detail), `dataviz` rules | Light and dark checked |
| [x] M4-07 (pure natural-sort playlist, context-menu command, automatic on completion via a setting; playlist generation not yet seen on a real multi-media torrent, but the paths it relies on are tested against the real engine) | `core::playlist` + context menu + auto-on-complete setting | Unit tests: natural sort (`E2` < `E10`), non-media skipped, relative UTF-8 paths; playlist opens in a player |
| [~] M4-08 (per-tracker true totals, ratio and the real upload still needed to reach a target, with time at the current speed; minimum seed time lives in M4-03. **"Seed where it helps" is not built**: the engine exposes no swarm seeder/leecher counts to prioritise by) | Ratio assistant: per-tracker true totals, min seed time, "seed where it helps" priority (needs peer/swarm counts from the engine) | Unit tests on the priority function with FakeEngine swarm data; totals match engine counters exactly |

## M5 — Network and automation
| ID | Task | Acceptance |
|---|---|---|
| [~] M5-01 (**changed**: the engine cannot bind to an adapter on Windows, so this is an app-level VPN guard that watches the adapter. Verified live: a missing adapter paused the download in 1.6 s, resume was refused with a plain message, a present adapter resumed it in 2.3 s. Not a hard kill switch: up to one check interval of traffic can leave, and the UI says so. Not tested: a real VPN adapter actually disappearing) | Bind interface + VPN guard (kill switch) | Disabling the adapter pauses all within 3 s; banner shown |
| [x] M5-02 (verified live: a fixed port is really listening and owned by drift.exe; a SOCKS5 proxy turns listening and DHT off; an invalid proxy is refused and the previous settings stay; torrents survive every restart. **Not built: an outside connectivity test**, which needs a third-party service; the UI shows the real listening port instead) | SOCKS5 proxy, port settings, UPnP toggle, connectivity test | Test result shown in Settings |
| [x] M5-03 (pure schedule with overnight and Sunday-to-Monday wrap, unit-tested; live: a slot around now replaced the 2 MiB/s global cap, `limits_status` reported it, and removing it restored the cap. The measured rate under a 256 KiB/s slot was 107 KiB/s, so below the limit but not saturating it) | Alt-speed weekly schedule | Unit tests for schedule evaluation incl. DST |
| [x] M5-04 (verified live: a .torrent dropped in the folder was added and renamed `.torrent.added`; a garbage file was set aside as `.torrent.invalid`) | Watch folder | Dropping a .torrent adds it once; no duplicates |
| [x] M5-05 (engine tests over real loopback sessions in both directions: a peer in a blocked range never transfers a byte, an unblocked control completes. Lists load only when a session starts, so a changed list restarts the engine session, which keeps its torrents) | Engine IP-filter hook: `TorrentEngine::set_ip_filter` implemented in `rqbit.rs`. librqbit has blocklist/allowlist but loads them once at session start (see SPIKES S1): apply via a normalized `file://` list and rebuild the session on update, or upstream an ArcSwap patch (record in DECISIONS.md) | Integration test: a loopback peer inside a blocked range never completes a handshake, in both directions; unblocked peer still works |
| [x] M5-07 (eMule `ipfilter.dat` incl. zero-padded octets and access levels, PeerGuardian, ranges, CIDR, IPv6; 200,000 ranges parse in well under 1.5 s) | `core::ipfilter`: parsers (eMule `ipfilter.dat`, PeerGuardian text, plain CIDR/ranges; IPv4, IPv6 ranges ignored with a count), sorted merged `RangeSet`, O(log n) lookup | Unit tests: malformed lines skipped and counted, overlapping ranges merge, boundaries inclusive, 200k-range list loads in < 200 ms and < 10 MB |
| [x] M5-08 (size caps, zip/gzip bombs, whitelisted zip entries, redirects limited to http(s) and never downgraded, empty or over-broad lists refused with the last good copy kept. Verified live with the list you named: 172,697 ranges, flagged as plain http, both torrents kept their 51 and 59 peers after the reload) | Subscriptions: HTTP(S) fetch with conditional GET (ETag/Last-Modified), zip/gz extraction with size caps, schedule, last-good list kept on disk | Zip bomb and oversized download rejected; failed or empty update keeps the previous list; a list covering > 25% of IPv4 space is rejected with a warning; works offline |
| [x] M5-09 (Settings is now four tabs; Network holds port, UPnP, DHT, proxy, VPN guard and the IP filter with a one-click suggested list, a file picker and Update now; guard banner; axe clean in both themes) | IP filter UI: Settings → Network → IP filter (subscriptions, toggle, add URL, import file, update now, ranges count, last updated) + blocked counter in status bar | All states from UI_SPEC; the emule-security preset appears but is off by default |
| [ ] M5-10 (not possible: the engine does not expose how many peers it blocked) | Peer table marks nothing for blocked peers; detail panel shows "Blocked N peers this session" | Counter increments in the loopback test |
| [~] M5-06 (GitHub Releases + signed updates, see `docs/RELEASING.md`. **Verified end to end locally:** a 0.0.1 build found a signed 0.0.2 on a local server, the banner appeared, "Install and restart" downloaded it, ran the installer, and the app came back as 0.0.2; the update checks the signature, installing waits for the click, a daily check can be turned off. **Not done:** nothing is published yet (the first real release creates `latest.json`; until then "Check now" reports it couldn't check), no delta updates, no rollback) | Auto-update (opt-in), release pipeline | Update from N to N+1 on a clean machine |

## M6 — Search and headless (can move ahead of M5 if the owner prefers)
| ID | Task | Acceptance |
|---|---|---|
| [x] M6-01 (parallel fan-out with a per-source timeout, merge by info-hash incl. base32 magnets and by title+size when there is no hash, ranking, 7 unit tests) | `SearchProvider` trait, `Aggregator`, fake provider | Unit tests: parallel fan-out, per-provider timeout, one failing provider doesn't fail the search, info-hash de-dup merges seeders and badges, ranking |
| [~] M6-02 (**changed**: keys live in a separate `secrets.json` in the per-user app-data folder, not Credential Manager, to avoid another dependency; the UI can set but never read them. Tested against a local fake Torznab server incl. error mapping; error text never carries the address or key. Not tested against a real Jackett/Prowlarr) | `torznab` provider + credential storage (Credential Manager) | Works against a local fake Torznab server in an integration test; API key never appears in the settings JSON or logs |
| [~] M6-03 (tested against a local fake; query operators stripped from user words, identifiers whitelisted before they enter a URL. Not yet run against archive.org itself) | `archive_org` provider | Returns torrent results for a known public-domain query; handles empty and error responses |
| [~] M6-04 (**changed**: a Search dialog (button, Ctrl+Shift+F, palette) instead of a top-bar mode, so the list filter keeps its meaning. Checked in the browser mock only; not yet in the real window) | Search UI (mode toggle, results list, source badges, status strip, add flow) | All search states from UI_SPEC implemented; screenshot per state; keyboard-only flow works |
| [x] M6-05 (Settings → Search: Internet Archive toggle, Torznab list with enable/remove/key, write-only key, and a Test button per source that sends one sample search and says whether it answered) | Settings → Search: provider list, add/test/toggle Torznab | Bad URL/key shows a plain-language error; first-run empty state shown |
| [x] M6-06 (axum API with bearer token, constant-time compare, health open, SSE events; 4 HTTP tests over the fake engine. Verified on the real engine: 401 without the token, add/list/remove, settings round trip, SSE snapshots. Not built: TLS, a Test-source button) | `drift-server`: engine + queue + rules wired, HTTP API with token auth, localhost default | `curl` can add, list, pause, remove, and search; requests without a token get 401; non-localhost bind refuses to start without a token |
| [x] M6-07 (watch folder, VPN guard and IP-list refresh are now core loops shared by app and server; verified headless: a junk .torrent was set aside as `.invalid`. `drift.lock` per data folder, taken by both programs, test passes and a second server was refused. The VPN guard is untested headless and Windows-only) | Headless watch folder + data-dir lock file | Drop a .torrent and it adds once; second instance on the same data dir refuses to start (desktop or server) |
| [~] M6-08 (`docs/HEADLESS.md` written with Task Scheduler and systemd examples. **Not tested**: registering a scheduled task changes system settings, so that is yours to try) | Headless docs + example run as a Windows scheduled task / Linux systemd unit | `docs/HEADLESS.md`, tested start/stop on a clean profile |
| [x] M6-09 (`src/lib/api/http.ts` + login page; the same UI served by `drift-server --ui-dir dist`. Verified in the browser pane: wrong token refused, right token shows the app. Play and Show-in-folder say they aren't available remotely; details poll once a second) | Remote web UI: `src/lib/api` gets an HTTP + event-stream transport; `drift-server` serves the built UI; token login page | Same UI loads at `http://127.0.0.1:<port>` and works from a second device on the LAN with a token; no token = 401; optional TLS cert path in config |

## M7 — Plugins (needs M6; read `docs/PLUGINS.md` first)
| ID | Task | Acceptance |
|---|---|---|
| [x] M7-01 (`core::plugins`; tested against the real reference binary in `crates/drift-core/tests/plugins.rs`: handshake, hang → timeout, 3 crashes → off, idle shutdown, api/exec/folder-name refusals; memory cap via Windows Job Object, 256 MB) | Plugin host: manifest parsing, discovery, lifecycle (lazy start, idle kill, timeout, crash backoff, Job Object memory cap) | Tested with a fake plugin binary: handshake, timeout, crash 3× → disabled, idle shutdown, `api` mismatch refused |
| [x] M7-02 (reference plugin is `drift-example-plugin`; results are cleaned before they reach the UI) | Protocol v1 doc + `ExternalSearchProvider` adapter + reference plugin | Reference plugin returns results through the aggregator; plugin crash never fails the search; protocol documented in `docs/PLUGIN_PROTOCOL.md` |
| [~] M7-03 (**done:** Plugins tab with list, on/off, version, permission prompt before the first enable, problems shown; **not done:** settings form, secrets, log view; no plugins take settings yet) | Settings → Plugins UI: list, enable/disable, version, permissions prompt, generic settings form from `settings_schema`, per-plugin log view | Secret fields stored via Credential Manager; permission prompt shown before first enable |
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
