# Architecture

## Goals
Fast, light, native-feeling. Reuse a mature engine; spend our effort on UX, queueing, and polish.

## Layers

```
┌───────────────────────── Tauri process ─────────────────────────┐
│  Svelte UI (WebView2)                                           │
│    src/lib/api  ── typed invoke + event wrapper (only IPC door) │
│    src/lib/stores ── view state, derived filters/sorts          │
│    src/components ── presentational Svelte                      │
│            ▲ IPC (commands down, snapshots up)                  │
│  ──────────┼─────────────────────────────────────────────────  │
│  src-tauri/src                                                  │
│    main.rs        composition root: builds & wires everything   │
│    commands.rs    thin IPC handlers → core services             │
│    core/          domain logic, NO I/O libs, NO librqbit        │
│      queue.rs       max-active scheduling, priorities           │
│      rules.rs       seed ratio/time goals, move-on-complete     │
│      model.rs       Torrent, Snapshot, AddRequest, Limits…      │
│    engine/                                                      │
│      mod.rs         trait TorrentEngine  (contract, owned by core)
│      rqbit.rs       adapter over librqbit (ONLY file using it)  │
│      fake.rs        in-memory engine for tests                  │
│    store/         trait SettingsStore + JSON file impl          │
│    platform/      tray, notifications, magnet/.torrent handler, │
│                   single-instance, autostart                    │
└─────────────────────────────────────────────────────────────────┘
```

## Workspace split (headless support)

Headless mode needs the engine and logic to run without a window, so the Rust code is a Cargo workspace. Paths above under `src-tauri/src/{core,engine,store,search}` live in `crates/drift-core/src/`.

```
crates/drift-core    core/, engine/, store/, search/   (no Tauri, no UI)
crates/drift-server  headless binary: HTTP API + watch folder (depends on drift-core)
src-tauri            Tauri app: commands.rs, platform/  (depends on drift-core)
```
Both binaries are composition roots: each builds the engine, store, and search registry and wires `core`. `drift-core` must never depend on `tauri`, `axum`, or `webview`.

## IP filter

`core::ipfilter` owns parsing, the immutable `RangeSet` (sorted, merged, binary-searched), and a `Subscriptions` service that fetches, validates, and stores lists. Each refresh builds a new `IpFilter` from all enabled lists and hands it to the engine through `set_ip_filter`; the engine swaps it atomically, so lookups on the connection path are lock-free reads. Persisted: the subscription list in settings, the last good downloaded copy per list in `<data>/ipfilter/`.

Lists are untrusted input, and some are served over plain HTTP, so:
- strict parsing, bad lines skipped and counted, never executed or interpolated anywhere;
- download and decompressed size caps; zip entries read by name from a whitelist (`ipfilter.dat`, `*.txt`, `*.p2p`), no path extraction to disk;
- sanity limits: empty list or one covering more than 25% of IPv4 space is rejected and the last good list stays (a hostile or broken list must not silently cut you off from the swarm);
- HTTPS preferred and warned about when absent; no redirects to other schemes.

Honest expectation: IP lists reduce contact with some known-bad ranges but are not privacy protection (that is a VPN/bind-interface job) and old eMule-era lists can also block legitimate peers. The UI says so in one line next to the toggle.

## Search (extension point)

Search sources are providers behind a consumer-owned contract, registered in the composition root. Adding a source = adding a file and one registry line.

```rust
#[async_trait]
pub trait SearchProvider: Send + Sync {
    fn id(&self) -> &str;                 // stable key, e.g. "torznab:<name>", "archive-org"
    fn name(&self) -> &str;               // shown in UI
    async fn search(&self, q: &SearchQuery) -> Result<Vec<SearchResult>, ProviderError>;
}
// SearchResult { title, magnet|torrent_url, info_hash?, size?, seeders?, leechers?, published?, category?, provider_id }
```

`core::search::Aggregator` fans out to all enabled providers in parallel with a per-provider timeout (default 6 s), reports per-provider status (ok / failed / timed out) alongside results, de-duplicates by info-hash (merging seeder counts and source badges), and ranks by seeders. One provider failing never fails the search. No result caching beyond the current query.

Shipped providers (no scrapers for third-party indexes are bundled — see D-8):
- `torznab` — any Torznab/Newznab-style endpoint the user configures (Jackett, Prowlarr, and similar). This is how users bring their own sources. API key is stored in the Windows Credential Manager, not in the settings JSON.
- `archive_org` — Internet Archive advanced search, torrents of public items.

Providers are user-toggleable; the user can add several Torznab endpoints.

Plugins (out-of-process, independently updatable) plug into this same registry through an `ExternalSearchProvider` adapter; see `docs/PLUGINS.md`. The trait does not change when a source moves from built-in to plugin.

## Headless server (`drift-server`)

`drift-server --data-dir <folder> [--bind addr] [--ui-dir folder]` runs the same engine, queue, rules, filters, guard, watch folder and search with no UI (full guide: `docs/HEADLESS.md`). axum binds to `127.0.0.1:8793` by default; **every** route except `GET /api/health` needs `Authorization: Bearer <token>` (token created on first start, kept in `server.json`). Routes live under `/api` and mirror the IPC commands; `GET /api/events` is a server-sent-event stream (`snapshot`, `completed`, `guard`, `watch`). Same JSON models as the IPC; errors are `{code, message}`. Shared logic is in `drift-core::{background, service}`, the HTTP layer only translates. `drift.lock` in the data folder keeps the app and the server from sharing one folder. The web UI is the desktop frontend with `src/lib/api/http.ts` as its transport, chosen when `/api/health` answers.

## Playlist

`core::playlist` builds an `.m3u8` for multi-file torrents: keep audio/video files, natural-sort by path (so `E2` precedes `E10`), relative paths, UTF-8. Written next to the data on completion (setting, default on) and on demand from the context menu ("Copy playlist" / "Create playlist"). Pure function over the file list, unit-tested.

Dependencies point one way: `commands → core → (engine trait, store trait)`; `rqbit.rs`, `store/json.rs`, `platform/*` implement or sit beside them. `core` imports nothing from Tauri or librqbit.

## Contract: `TorrentEngine` (owned by `core`)

```rust
#[async_trait]
pub trait TorrentEngine: Send + Sync {
    async fn add(&self, req: AddRequest) -> Result<TorrentId>;     // magnet | file bytes | url
    async fn list(&self) -> Vec<TorrentStats>;                      // cheap, called by the tick
    async fn details(&self, id: TorrentId) -> Result<TorrentDetails>; // files, peers, trackers
    async fn pause(&self, id: TorrentId) -> Result<()>;
    async fn resume(&self, id: TorrentId) -> Result<()>;
    async fn remove(&self, id: TorrentId, delete_files: bool) -> Result<()>;
    async fn set_files(&self, id: TorrentId, selected: Vec<FileIdx>) -> Result<()>;
    async fn set_limits(&self, l: Limits) -> Result<()>;            // down/up bytes/s, 0 = unlimited
    async fn set_network(&self, n: NetworkConfig) -> Result<()>;    // port, bind iface, proxy, DHT on/off
    async fn set_ip_filter(&self, f: Arc<IpFilter>) -> Result<()>;  // swap the active filter atomically; engine rejects blocked addrs in both directions
    async fn stream_url(&self, id: TorrentId, file: FileIdx) -> Result<Option<Url>>;
}
```

**As built (M0):** `crates/drift-core/src/engine/mod.rs` is authoritative. Implemented: `add`, `list`, `details`, `pause`, `resume`, `remove`, `set_files`, `set_limits`; ids are `&TorrentId` (lowercase info-hash hex) and every method returns `Result<_, EngineError>`. `set_network`, `set_ip_filter` and `stream_url` are added with the milestone that needs them (M5-05, M5-02, M4-04). `add` is idempotent (re-adding a managed torrent returns its id, even by bare magnet). ETA is derived in the adapter from speed because librqbit's own is private.

Rules: no librqbit types in signatures. Errors are our `EngineError` enum. Anything librqbit can't do (queueing, scheduling, ratio goals) lives in `core`, driven through this trait (pause/resume).

## Contract: `SettingsStore`
`load() -> Settings`, `save(&Settings)`. JSON in the app config dir, versioned with a `schema` field and forward-only migrations. Per-torrent app metadata (category, tags, priority, goals) is in the same store keyed by info-hash.

## IPC (frontend ↔ backend)

**Commands (invoke):** `add_torrent`, `pause`, `resume`, `remove`, `set_files`, `set_limits`, `get_details`, `get_settings`, `set_settings`, `open_path`, `stream_url`. All return `Result<T, ApiError{code,message}>`; the UI shows `message` verbatim.

**Events (push):**
- `snapshot` — whole list of `TorrentStats`, throttled to 1 Hz when visible, 0.2 Hz when minimised/hidden, stopped when in tray with no window.
- `details` — for the selected torrent only, 2 Hz while the panel is open.
- `toast` — completion, errors, "added".

The backend owns a single tick task (`core::ticker`) that polls the engine, runs `queue`/`rules`, and emits. The UI never polls. The list is virtualised; sorting/filtering is client-side over the snapshot.

## Persistence & restart
librqbit session state persists to `<data>/session`. On start: load settings → build engine → rehydrate → queue re-evaluates. Crash must not lose the torrent list.

## Performance budgets
Cold start < 1 s (measured 303 ms) · memory incl. WebView2 <= 130 MB private (measured 119 MB; the original 80 MB target is not reachable with WebView2) · UI stays 60 fps with 1,000 torrents (measured) · UI cost per snapshot < 4 ms at 1,000 torrents (measured 1.2 ms p50). See docs/PERF.md.

## Testing
- `core/*`: unit tests with `engine::fake`.
- `engine/rqbit.rs`: one integration test that adds a tiny local test torrent over loopback (no internet).
- Frontend: Vitest for stores/formatters; one Playwright smoke per milestone against the built app.

## Security / privacy defaults
No telemetry, no update ping without opt-in. Local HTTP stream URL binds to 127.0.0.1 with a random token. No remote web UI unless the user enables it (post-v1, auth required). Never log magnet contents at info level.

## Known risks (verify early — M0 spikes)
1. **MSE/PE encryption** is not listed in librqbit's README. Some peers refuse plain connections. Spike: measure peer counts on a popular ISO vs a reference client. If poor, options: upstream contribution, or accept it.
2. **uTP** lives in a separate crate (`librqbit-utp`). Check if it is enabled by default and stable.
3. **API churn:** pin an exact `librqbit` version; the adapter is the only blast radius.
4. **Windows file I/O:** long paths, Defender scanning slows hash checks. Spike on a 6 GB ISO.
