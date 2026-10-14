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
| `postprocess` | v1.1 | Receives a "torrent completed" event (name, path, files, category); may rename, move, notify, run tools. Returns actions or nothing. |
| `notifier` | later | Webhooks, Telegram, etc. on events. |
| `blocklist` | later | Supplies and refreshes IP lists. |
| Theme packs | data only | JSON of design tokens. **No code**, no process, so no permissions needed. |

Not plugin territory: engine, queue, UI components. UI never runs plugin code.

## Layout and manifest

```
<data>/plugins/<id>/
  plugin.json
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
  "settings_schema": { "endpoints": "list<{name:string,url:url,apikey:secret}>" }
}
```

`api` is the protocol major version. The host refuses plugins whose `api` it doesn't support. `settings_schema` lets the app render the plugin's settings form generically, with `secret` fields going to Credential Manager.

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

- Registry = a static signed `index.json` (plugin id → versions → download URL, sha256, min_app, changelog), hosted on GitHub releases/pages. The app only contacts it if the user opts in to plugin updates.
- Install/update: download → verify sha256 and Ed25519 signature → unpack to a staging folder → atomic rename → keep the previous version for one-click rollback.
- The app updates separately from plugins. Compatibility is governed by `api` major and `min_app`.

## First-party plugins (dogfooding)

`torznab` and `archive_org` ship **bundled in the installer as plugins**, not as in-tree providers, once the protocol exists. That is the proof the protocol is good enough, and it lets search sources be fixed without an app release. Until M7 they are built in-tree behind `SearchProvider` (M6), then moved; the in-tree copies are then deleted.

## What we deliberately do not build

A plugin store UI with ratings, paid plugins, plugins that draw UI, plugins with engine access, auto-install of unsigned plugins, and multi-language SDKs. A reference plugin plus the protocol doc is the SDK.
