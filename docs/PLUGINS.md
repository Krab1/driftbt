# Modules and plugins

Goal: some features ship and update independently of the app, without bloating the core or letting a bad extension take the client down.

## Two tiers

| Tier | What | Updated | Isolation |
|---|---|---|---|
| **Built-in modules** | Compiled into the app (queue, rules, playlist, tray, settings). Registered in the composition root via traits. | With the app | Same process |
| **Plugins** | Separate executables the app discovers, starts, and talks to over a stable protocol. | Independently, from a registry | Separate process |

Rule of thumb: if it needs engine internals or runs every tick, it is a built-in module. If it talks to the outside world or is optional, it can be a plugin.

## Why out-of-process (sidecar), not the alternatives

| Option | Verdict |
|---|---|
| Dynamic libraries (`.dll`) | Rejected. Rust has no stable ABI; one crash kills the app. |
| WASM (wasmtime) | Rejected for now. Adds several MB and a runtime to a "light" app; sandbox is nice but network/file access still needs host plumbing. Revisit if untrusted third-party plugins become common. |
| JS plugins inside the webview | Rejected. They'd run next to the UI with its IPC; security and stability risk. |
| **Sidecar process + JSON-RPC over stdio** | **Chosen.** Language-agnostic, crash-isolated, trivially updatable (swap a folder), no new runtime in the app. |

## Plugin kinds

| Kind | Protocol phase | What it does |
|---|---|---|
| `search` | v1 | Implements `SearchProvider` (see ARCHITECTURE). First kind, because search sources break and change most often. |
| `postprocess` | v1 | Receives a "torrent completed" event (name, save folder, files, category, tags). Returns actions: move the torrent to a folder (through the engine, so seeding carries on) or show a notice. It may do its own work while it runs; renaming files inside a torrent isn't an action because it would break the data being seeded. |
| `simulator` | v1 | Hears the list of torrents (`torrents_updated`) and which of them the user switched on (`selection_changed`, from the torrent's right-click menu: "Simulator plugins"). Drift only provides the list and the switches; everything the plugin does with a selected torrent, and the state it keeps for that, is the plugin's own (in its own folder). Unlike other kinds it is not stopped when idle, because it owns running jobs; Drift ends it when it is switched off or at exit. Kinds can be combined, e.g. `["search", "simulator"]`. Example: `tracker-sim`, in its own repo ([Krab1/driftbt-plugins](https://github.com/Krab1/driftbt-plugins)), installed from the registry. |
| `notifier` | later | Webhooks, Telegram, etc. on events. |
| `blocklist` | later | Supplies and refreshes IP lists. |
| Theme packs | data only | JSON of colour tokens. **No code**, no process, so no permissions needed. See below. |

Not plugin territory: engine, queue, UI components. UI never runs plugin code.

## Theme packs

A pack is one JSON file (see `docs/theme-pack-example.json`), imported in Settings → General → Theme pack:

```json
{ "drift_theme": 1, "name": "Midnight", "base": "dark", "tokens": { "bg": "#0b1020", "accent": "#7c9cff" } }
```

- `base` (`light` or `dark`) is the built-in theme the pack builds on; colours the pack leaves out come from it. An active pack replaces the Light/Dark/System choice until it is removed.
- `tokens` may only name the colour tokens of `tokens.css` (`bg`, `surface-1`, `surface-2`, `hover`, `selected`, `text-primary`, `text-secondary`, `text-muted`, `border`, `border-strong`, `accent`, `accent-fg`, `on-accent`, `success`, `warning`, `danger`, `danger-fg`, `neutral`, `track`), each with a `#rgb`, `#rrggbb`, `#rrggbbaa` or `rgb()/rgba()` value. Fonts, sizes and anything else are refused, as is any unknown key, a file over 20 KB or invalid JSON; the message says why.
- A pack that is accepted but has text under 4.5:1 contrast against its background (main, secondary and faint text, links, error text, text on accent buttons) is used anyway and lists each weak pair. Colours with transparency aren't measured.
- The pack is stored in `settings.json` (`theme_pack`); there's no folder, process or registry entry. `src/lib/themepack.ts` does the checking.

## Layout and manifest

```
<data>/plugins/<id>/
  plugin.json          (id, name, version, a plain-language "description" shown in the list, kinds, permissions, settings form)
  <executable or script entry>
  (optional) README, icon
```

```json
{
  "id": "torznab",
  "name": "Torznab sources",
  "version": "1.2.0",
  "api": 1,
  "kinds": ["search"],
  "exec": ["torznab.exe"],
  "min_app": "1.0.0",
  "permissions": { "net": ["*"], "fs": [] },
  "settings": [{ "key": "endpoint", "label": "Server address", "type": "url" }, { "key": "apikey", "label": "API key", "type": "secret" }]
}
```

`api` is the protocol major version. The host refuses plugins whose `api` it doesn't support. `settings` lets the app render the plugin's settings form generically. Secret fields are kept in a separate file and handed to the plugin only when it asks (`get_secret`); see `PLUGIN_PROTOCOL.md`.

## Protocol v1

Newline-delimited JSON-RPC 2.0 on the plugin's stdin/stdout. stderr is captured to a per-plugin log (never shown as UI text). Host to plugin:

- `initialize {api, app_version, settings}` → `{kinds, name, version}`
- `search {query, category?, limit}` → `{results: SearchResult[]}` (same shape as the in-app `SearchResult`)
- `shutdown {}`

Plugin to host (limited, no engine access): `log {level, message}`, `get_secret {key}` → value for that plugin only. Nothing else. Plugins cannot add torrents or touch the engine; the host takes the results and the user decides.

Lifecycle: started lazily on first use, killed after 5 min idle, per-call timeout (search: 6 s, same as providers), crash → mark failed, restart with exponential backoff, after 3 crashes disable and tell the user. Memory capped via a Windows Job Object. An `ExternalSearchProvider` adapter in `drift-core` wraps a running plugin as a `SearchProvider`, so the aggregator doesn't know the difference.

## Permissions and trust (honest limits)

Manifest permissions (`net` host patterns, `fs` paths) are shown to the user on install and enable. **A plain child process on Windows can't be strictly sandboxed**, so permissions are consent and transparency, not enforcement, unless we later run plugins in AppContainer. Mitigations: registry-only installs by default, signature and hash verification, Job Object resource limits, no engine API, minimal environment (no inherited secrets), unsigned or sideloaded plugins require an explicit warning dialog.

## Registry and updates

- Registry = a static `registry/index.json` in the Drift repo (served from raw.githubusercontent.com), with a detached minisign signature `index.json.sig` made with the same key that signs Drift's own updates (`~/.tauri/drift.key`; the public key is baked into the app). Shape: `{"version":1,"plugins":[{"id","name","summary","versions":[{"version","url","sha256","min_app","api","changelog"}]}]}`. `url` must be https (zip with `plugin.json` at its root, hosted e.g. as a GitHub release asset). The signature covers the hashes, so one signature protects every download.
- The app contacts the registry only when Settings → Plugins → "Look for plugins and updates online" is on (off by default) and the user clicks Check, Install or Update. Nothing runs in the background.
- Install/update: download (https only, size-capped) → check sha256 against the signed index → unpack with path checks into `<plugins>/.staging/<id>` → check the manifest is the promised id and version and loadable. The live folder is never touched while Drift runs. At the next start `apply_pending` moves the live folder to `<plugins>/.previous/<id>` and the staged one into place; if that fails half way the live version is put back and the update waits for the next start.
- Rollback = stage `.previous/<id>` again; the swap at the next start keeps the newer one as the new "previous".
- Installing doesn't turn a plugin on. A plugin that is new, or whose update asks for more `net`/`fs` access than the version it replaces, is switched off when it takes over (`switch_off_unapproved_plugins`) and must be turned on again, which shows the permission prompt. An unpacked folder is checked again before it is swapped in; one that doesn't load is deleted, never swapped in.
- Zip entries may only have plain names: no `..`, drive letters or `:` (alternate streams), reserved device names, trailing dots or spaces, or characters Windows forbids; at most 1000 files and 128 MB unpacked. An id that differs from an installed plugin's folder only in letter case is refused.
- Install and Update need the version the user was shown to still be the newest one, and never move a plugin to an older or equal version (an old signed index stays valid forever, so this is how a replayed index is kept from downgrading anyone). Rolling back is the only way to an older version. Known limit: a replayed old index can still hide a newer release until the real one is fetched again.
- Installing is desktop-only. `drift-server` has no install, update or rollback routes, so a remote token can't bring new code onto the machine.
- Publishing: edit `registry/index.json` (LF line endings; `.gitattributes` stops git changing them), upload the zip, then sign the index again:
  `npx tauri signer sign -f %USERPROFILE%\.tauri\drift.key --password= registry\index.json` and commit both files. A unit test fails if the committed index doesn't verify.
- The app updates separately from plugins. Compatibility is governed by `api` major and `min_app`.

## First-party plugins (dogfooding)

`torznab` and `archive_org` ship **bundled in the installer as plugins** (`crates/drift-search-plugin`; `scripts/bundle-plugins.ps1` lays them out in `src-tauri/bundled-plugins` and the installer carries them). That is the proof the protocol is good enough, and it lets search sources be fixed without an app release.

- **Install and upgrade.** At every start `registry::provision` copies a bundled plugin into `<data>/plugins/<id>` if it is missing, and stages it to take over at the swap if it is older than the bundled copy. A copy that is the same or newer (say, updated from the registry) is left alone, and so is a rollback to an older version as long as the app's bundled copy isn't newer. Bundled folders are the app's own files, so they need no signature check, only to be loadable.
- **Versions.** The bundled plugins take the app's version number (`scripts/bundle-plugins.ps1` writes it into each `plugin.json`), so an app release that changes the program always upgrades installed copies. A newer staged registry update is never replaced by an older bundled copy.
- **Both plugins are the same program** (`drift-search.exe`, about 1 MB each in the installer) with a different argument in `plugin.json`. They don't depend on `drift-core`, so they carry no torrent engine.
- **Updating one without an app release** means publishing a new version of that plugin's zip to the registry (`docs/PLUGINS.md`, Registry and updates); the app's bundled copy only matters on a fresh install or when the app itself carries a newer version.
- **Migration.** Older versions kept search settings in the app's settings (`search.archive_org`, `search.torznab`) and keys in `secrets.json`. `search::migrate` moves them once: each indexer becomes a row of the `torznab` plugin, its key becomes that row's secret, and the plugins are switched on if they were on. The old values are cleared only after their plugin has taken them; a missing plugin loses nothing.
- **Editing an indexer's address clears its saved key** (a key stored for one server is never sent to another). Add the key again after changing the address.
- **Known differences from the built-in sources.** A Torznab indexer's own status (refused key, unreachable) is no longer reported per indexer: the plugin reports one result for all its indexers, and only reports a failure when every indexer failed (the most telling reason: a refused key, then a network error, then unreadable data). The Test button tests the plugin, not one indexer.

## What we deliberately do not build

A plugin store UI with ratings, paid plugins, plugins that draw UI, plugins with engine access, auto-install of unsigned plugins, and multi-language SDKs. A reference plugin plus the protocol doc is the SDK.
