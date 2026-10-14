# Features

Scope control: anything not here needs approval. "Engine" = comes from librqbit; "Ours" = we build it in `core`/`platform`/UI.

## MVP (M1–M3): a client you can use daily
| Feature | Source | Notes |
|---|---|---|
| Add by magnet, .torrent file, URL | Engine | Paste anywhere in the window, drag-drop, Add dialog |
| Register as magnet / .torrent handler | Ours | Windows file association, single-instance forwarding |
| Pause / resume / remove (keep or delete data) | Engine | Remove asks once, remembers choice |
| Torrent list with progress, speed, ETA, size | Ours | Virtualised, sortable, tabular numerals |
| Filters: All / Downloading / Seeding / Paused / Error | Ours | Counts in the sidebar |
| Search box filters the list | Ours | Client-side, instant |
| File selection before and after adding | Engine | Tree with checkboxes, size per file |
| Download folder + per-add override | Ours | Free-space check before start |
| Global speed limits | Engine | Toggle + values in the status bar |
| Session persistence across restarts and crashes | Engine | |
| Detail panel: files, peers, trackers, overview | Ours | Live, selected torrent only |
| Completion notification + "open folder" | Ours | Native toast |
| Light/dark theme following system | Ours | Manual override |
| Tray icon, close-to-tray, start minimised | Ours | Optional autostart |

## v1 (M4–M5): the useful extras
| Feature | Source | Why it is useful |
|---|---|---|
| **Play while downloading** (stream to default player) | Engine (HTTP stream) + Ours | Sequential piece priority; button on video files. Differentiator. |
| Download queue (max active, drag to reorder, priority) | Ours | librqbit has no queue; implemented in `core::queue` |
| Seeding goals: stop at ratio or time | Ours | `core::rules` |
| Categories and tags, with default save path per category | Ours | Sidebar group, drag torrent to category |
| Move on complete | Ours | Per category |
| Alt-speed schedule (e.g. night / work hours) | Ours | Weekly grid, simple |
| Bind to network interface / VPN guard | Engine + Ours | Pause all if the bound interface disappears (kill switch) |
| Proxy (SOCKS5) | Engine | |
| Port settings: random/fixed, UPnP toggle, connectivity test | Engine + Ours | |
| **IP filter** (peer blocklists) | Ours + engine hook | Subscribe to list URLs (eMule `ipfilter.dat`, PeerGuardian-style text; `.zip`/`.gz` accepted) or import a file; auto-update on a schedule; applied to inbound and outbound connections; blocked-peer counter. A one-click preset for `http://upd.emule-security.org/ipfilter.zip` is offered but **off until the user enables it**. Needs an engine hook (spike M0-02); if the engine has none, we patch/upstream it before shipping this. |
| Watch folder (auto-add .torrent) | Ours | |
| Keyboard-first: command palette (Ctrl+K), shortcuts | Ours | Add, pause all, search, filter |
| Per-torrent: copy magnet, open folder | Ours | Context menu. Recheck and force announce dropped: librqbit has neither (SPIKES S1). |
| Speed graph (session history) | Ours | Status bar + detail panel |
| Disk-space and error surfacing | Ours | Plain-language messages, not raw errors |
| **Search** across user-enabled providers | Ours | Aggregated, de-duplicated by info-hash, ranked by seeders; per-source status. Providers: Torznab (bring your own Jackett/Prowlarr/etc.) and Internet Archive. One click adds a result. |
| **Plugins** (sidecar processes, independently updatable) | Ours | Search sources first, then post-processing hooks; signed registry, per-plugin settings and logs, rollback. Theme packs as data only. See `docs/PLUGINS.md`. |
| **Headless mode** (`drift-server`) | Ours | Runs without a window on a PC, NAS, or seedbox: HTTP API with token auth, watch folder, same queue/rules/search. Localhost by default. |
| **Remote web UI** for headless mode | Ours | `drift-server` serves the same Svelte UI over HTTP (token login, localhost by default, TLS option for LAN). Same screens as desktop; the front-end `api` layer has two transports (Tauri IPC, HTTP + event stream). |
| **Ratio assistant** (real seeding help) | Ours | Per-tracker upload/download totals and ratio, seed-until goals, minimum seed time (avoids hit-and-run), and a "seed where it helps" mode that gives upload priority to swarms with the most leechers and fewest seeds. Reports only true numbers. |
| **Playlist** (`.m3u8`) for multi-file torrents | Ours | Natural-sorted, relative paths; auto on completion or on demand |

## Later (post-v1, only if asked)
RSS auto-download · torrent creation · mobile companion · sequential-only ordering for non-media · scheduling per torrent · tracker status history · localisation.

## Explicit non-goals
Reporting fake upload or download totals to trackers, or masking real client behaviour from them (ratio spoofing; see D-15) · Bundled scrapers for third-party torrent index sites (search is provider-based; users bring their own sources) · bundled VPN · ads or telemetry · plugins that draw UI or access the engine (sidecar plugins for search and post-processing are in scope, see `docs/PLUGINS.md`) · media library · cloud sync.

## Open questions (resolve in M0)
1. Does librqbit support runtime file re-selection, recheck, force-announce, bind-interface, and blocklist? (Gate for the v1 rows above; checked in M0 spike S1.)
2. Is MSE encryption available or addable? (spike S2)
3. App name and icon — "Drift" is a placeholder.
