# Decisions

Append-only. Format: `D-n · date · decision — why — consequence`.

- **D-1 · 2026-10-13 · Use `librqbit` as the engine, no own protocol implementation** — mature, async, DHT/magnet/PEX/UPnP/IPv6/streaming/limits included; writing a client engine from scratch is weeks of edge-case work. Consequence: we pin the version and isolate it behind `TorrentEngine`.
- **D-2 · 2026-10-13 · Tauri v2, not Electron** — system WebView2, small installer, low RAM. Consequence: Windows first; Linux/macOS later without a rewrite.
- **D-3 · 2026-10-13 · Embed librqbit in the Tauri process; IPC, not its HTTP API** — no extra port, no auth surface, lower latency. The HTTP API stays an option for a future remote UI.
- **D-4 · 2026-10-13 · Svelte 5 + TS, plain CSS tokens** — smallest runtime, fine-grained updates suit a live list. No component kit, to keep the look ours and the bundle small. (Reversible before M1 ends.)
- **D-5 · 2026-10-13 · Queue, ratio goals, schedule built by us in `core`** — not provided by the engine; driven via pause/resume through the trait.
- **D-6 · 2026-10-13 · No Python anywhere** — project owner rule. Tooling is Rust/TS/PowerShell.
- **D-7 · 2026-10-13 · Working name "Drift"** — placeholder until the owner picks one.

- **D-8 · 2026-10-14 · Search is provider-based; no bundled scrapers for third-party index sites** — owner wants search. Scrapers break whenever a site changes and carry legal/maintenance risk; Torznab is the standard way for users to plug in their own sources, and Internet Archive gives a legal built-in. Consequence: `SearchProvider` trait + registry; adding a source never edits existing code.
- **D-9 · 2026-10-14 · Cargo workspace: `drift-core` / `drift-server` / `src-tauri`** — headless mode needs the engine and logic without a window. Consequence: core must stay free of Tauri/HTTP deps; two composition roots.
- **D-10 · 2026-10-14 · Playlists are a pure function in core (`.m3u8`)** — same code for GUI and headless.
- **D-11 · 2026-10-14 · Torznab API keys go in Windows Credential Manager, not the settings JSON** — needs a keyring crate (justified here per CLAUDE.md rule 7).

- **D-12 · 2026-10-14 · Plugins are sidecar processes speaking JSON-RPC over stdio; no DLLs, no WASM, no UI plugins** — language-agnostic, crash-isolated, no extra runtime in a light app. Trade-off accepted: Windows can't strictly sandbox a child process, so permissions are consent plus signatures plus resource limits (PLUGINS.md). Supersedes the earlier "no plugin system" non-goal, which was written before search needed independently updatable sources.
- **D-13 · 2026-10-14 · Build search providers in-tree first (M6), extract to plugins in M7** — the plugin protocol should be shaped by two or three real providers, not guessed. Once extracted, the in-tree copies are deleted and `torznab`/`archive_org` ship bundled as first-party plugins.

- **D-14 · 2026-10-14 · IP filter is a built-in module (`core::ipfilter`), not a plugin, and is applied inside the engine through `set_ip_filter`** — it sits on the connection hot path and must be lock-free; parsing and fetching are pure/simple. A `blocklist` plugin kind can later feed lists into the same service. Third-party lists are untrusted: size caps, sanity limits, last-good fallback. The emule-security list is a user-enabled preset, never on by default.

- **D-15 · 2026-10-14 · No ratio spoofing** — the client always reports true uploaded/downloaded totals to trackers, with no fake-upload mode and no behaviour designed to avoid tracker detection. Reason: it defrauds trackers and other seeders, and it is the fastest way to get an account banned. The need behind it (poor ratio on dead torrents) is served by the ratio assistant instead.
- **D-16 · 2026-10-14 · Remote UI reuses the desktop front-end** — one codebase, `api` layer with two transports; no separate web app.

- **D-17 · 2026-10-14 · `tauri-plugin-dialog` for the folder picker** — official plugin, needed for "change download folder"; permission limited to `dialog:allow-open`. (CLAUDE.md rule 7 justification.)
- **D-18 · 2026-10-14 · The frontend `api` has a mock transport used outside Tauri** — lets the UI be developed and measured in a plain browser with N fake torrents (`?n=1000`), and is the seam for the remote HTTP transport (M6-09). Selected at startup by `__TAURI_INTERNALS__`.
- **D-19 · 2026-10-14 · Native window frame for now** — the custom title bar (M3-05) risks losing Windows 11 snap layouts; do it once the app is otherwise done.

- **D-20 · 2026-10-14 · M2 dependencies (CLAUDE.md rule 7)** — `tauri-plugin-single-instance` (second launch forwards its arguments), `tauri-plugin-notification` (completion toast), Tauri's built-in `tray-icon` feature. All official and small. Notifications and the tray are used from Rust only, so no extra JS packages or capabilities.
- **D-21 · 2026-10-14 · Autostart uses the `auto-launch` crate directly, not `tauri-plugin-autostart`** — the plugin writes the machine-wide Run key whenever the process is elevated and exposes no option; a per-user setting must stay per-user (found by testing from an elevated shell). The crate is already the plugin's own dependency.
- **D-22 · 2026-10-14 · No in-app "register as magnet handler" button** — Windows protects the user's default-app choice and a registry write could overwrite another client's setting. The installer registers `magnet:` and `.torrent` (M3-04).
- **D-23 · 2026-10-14 · The backend decides whether the window is visible** — WebView2 keeps `document.hidden == false` after the window is hidden to the tray, so the loops combine the native window state with the UI's report, and a `window-shown` event makes the UI catch up when it returns.

- **D-24 · 2026-10-14 · `core::manager` sits between the app and the engine** — queue order, categories, tags, goals and the seeding clock are not engine concepts. The manager is the only thing the UI mutates through; it uses pure planners (`queue`, `rules`) that are unit-tested against the fake engine. The engine gains no new methods for any of it; queueing is just pause/resume. Per-torrent bookkeeping lives in `library.json`, separate from `settings.json`.
- **D-25 · 2026-10-14 · A torrent first seen already paused is treated as user-paused** — so adopting torrents from older versions never auto-resumes something the user stopped.

Note: rqbit already ships its own Tauri desktop app (web UI wrapped). We are not forking it; our value is a purpose-built native-feeling UI plus queue/rules/streaming UX.
