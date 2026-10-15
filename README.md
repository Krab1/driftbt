# Drift

A fast, light BitTorrent client for Windows. Built with Rust and Svelte on [Tauri](https://tauri.app), using the [librqbit](https://github.com/ikatson/rqbit) engine.

Drift tries to be the client you leave running and forget about: it starts in about 0.3 seconds, sits near 0.2 % CPU when idle, ships as a ~7 MB installer, and keeps the things people actually need (queue, speed limits, categories, seeding goals, search, play-while-downloading) one click away. No ads, no telemetry, no bundled VPN or indexer.

> **Status:** early (0.1.x), Windows 10/11 x64 first. It is used daily by its author, but expect rough edges and file an issue when you find one.

## Install

1. Download the latest `Drift_<version>_x64-setup.exe` from [Releases](https://github.com/Krab1/driftbt/releases/latest) and run it.
2. Open it, paste a magnet link or drop a `.torrent` file anywhere in the window.

Updates are checked once a day (or from **Settings → General → Check now**). A banner appears and nothing is installed until you click **Install and restart**. Updates are signed; Drift refuses anything that doesn't match its built-in key.

Drift needs the Microsoft WebView2 runtime, which is already part of Windows 11 and current Windows 10.

## What it does

**Everyday**
- Add by magnet, `.torrent` file, or URL; paste anywhere, drag and drop, or associate `magnet:` and `.torrent` with Drift.
- Choose which files of a torrent to download, before or after adding. Files you skip are never downloaded and take almost no disk space.
- Torrent list that stays smooth with thousands of entries; filters, search box, categories and tags, sortable columns, keyboard navigation, and a command palette (`Ctrl+K`).
- Detail panel with files, peers, trackers and a live speed graph.
- Tray icon, close to tray, optional start with Windows, completion notifications, and plain-language error messages.
- Light and dark themes, plus colour-only theme packs.

**Control**
- Global speed limits with separate download and upload settings, and a weekly schedule.
- A speed limit for a single torrent (right-click → *Speed limit…*). The overall limit always wins if it is lower.
- Download queue with a maximum number of active downloads and drag-to-reorder priority.
- Seeding goals (stop at a ratio or after some time), a minimum seed time, and per-tracker ratio totals.
- Save folder per category, move finished downloads to another folder (also to another drive), watch folder for `.torrent` files, and `.m3u8` playlists for multi-file torrents.

**Network**
- SOCKS5 proxy, fixed or random port, UPnP toggle, DHT toggle.
- IP filter (blocklists such as eMule `ipfilter.dat` or PeerGuardian text; zip/gz accepted), refreshed on a schedule. Off until you turn it on.
- VPN guard: pauses everything if your chosen network adapter goes away.

**Find and watch**
- **Play while downloading:** start a video in your player before it finishes.
- **Search** across the sources you turn on (your own Torznab indexers such as Prowlarr or Jackett, and the Internet Archive), merged and ranked by seeders.

**Extend**
- **Plugins** are separate programs that talk to Drift over a small JSON protocol. They can add search sources or act when a torrent finishes (move it, show a notice). They run in their own process, so a bad plugin cannot take the app down. See [docs/PLUGINS.md](docs/PLUGINS.md).
- **Headless mode** (`drift-server`) runs the same engine without a window, with a web UI and an HTTP API. See [docs/HEADLESS.md](docs/HEADLESS.md).

## What it does not do

- It does not report fake upload or download numbers to trackers, and never will.
- It ships no torrent index and no scrapers. Search uses sources you add yourself.
- No protocol encryption (MSE) yet, so some peers that require it will not connect.
- The headless server speaks plain HTTP. Keep it on `127.0.0.1` and put it behind a VPN or an HTTPS reverse proxy if you need remote access.
- Windows only for now. The engine and server crates are portable; the desktop app has not been built or tested elsewhere.

Please use Drift only for content you have the right to download and share (Linux distributions, public-domain material, your own files).

## Build from source

You need [Rust](https://rustup.rs) (stable, MSVC toolchain), [Node.js](https://nodejs.org) (a current LTS, 22 or newer), and the Visual Studio C++ build tools.

```powershell
git clone https://github.com/Krab1/driftbt
cd driftbt
npm ci
scripts\bundle-plugins.ps1     # builds the bundled search plugins into src-tauri\bundled-plugins
npm run tauri dev              # development window with hot reload
npm run tauri build            # release installer under target\release\bundle\nsis
```

Checks that must pass before a change is done:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run check                  # svelte-check + the architecture rules
npm test                       # frontend unit tests (Vitest)
```

For UI work without the engine, run `npm run dev` and open `http://localhost:1420/?n=1000` for a mock with 1,000 fake torrents.

The headless server is `cargo build --release -p drift-server`; its web UI is the output of `npm run build`.

## How it is put together

```
crates/drift-core     engine trait, queue, rules, settings, plugins host (no UI or HTTP dependencies)
  engine/rqbit.rs     the only file that touches librqbit
crates/drift-server   headless HTTP server + web UI host
crates/drift-search-plugin   the bundled first-party search plugins
src-tauri             desktop shell and composition root
src                   Svelte 5 + TypeScript front end (talks to the backend only through src/lib/api)
```

The rules that keep it that way (one engine seam, one composition root, a front end that does not know about Tauri or librqbit) are in [CLAUDE.md](CLAUDE.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Why each choice was made is in [docs/DECISIONS.md](docs/DECISIONS.md).

## Documentation

| | |
|---|---|
| [docs/FEATURES.md](docs/FEATURES.md) | what is in scope, and what is not |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | modules, contracts, IPC |
| [docs/UI_SPEC.md](docs/UI_SPEC.md) | layout, tokens, states and copy |
| [docs/PLUGINS.md](docs/PLUGINS.md), [docs/PLUGIN_PROTOCOL.md](docs/PLUGIN_PROTOCOL.md) | writing plugins and theme packs |
| [docs/HEADLESS.md](docs/HEADLESS.md) | running `drift-server` |
| [docs/PERF.md](docs/PERF.md) | measured start-up time, memory and scrolling |
| [docs/RELEASING.md](docs/RELEASING.md) | how releases are built, signed and published |
| [docs/ROADMAP.md](docs/ROADMAP.md) | the task list |

## Performance

Measured on Windows 11 with a release build (details and method in [docs/PERF.md](docs/PERF.md)):

| | |
|---|---|
| Start to visible window | about 0.3 s |
| Memory, app plus WebView2 | about 120 MB |
| Idle CPU | about 0.2 % of one core |
| Scrolling 1,000 torrents | 60 fps |
| Installer | about 7 MB |

## Contributing

Issues and pull requests are welcome. Keep changes small, add a test for new logic, and update the matching document in `docs/` in the same change. A new dependency needs a one-line reason in `docs/DECISIONS.md`.

## License

[MIT](LICENSE)
