# Plugin protocol v1

Everything about the design and trust model is in `PLUGINS.md`. This is the wire format.

## Folder

```
<data>/plugins/<id>/plugin.json
<data>/plugins/<id>/<program>
```

The folder name must equal `id` (letters, digits, `-`, `_`). `exec[0]` is a path **inside** the folder (no `..`, not absolute); the rest of `exec` are arguments. The app starts it with that folder as the working directory, an empty environment (only `SystemRoot`, `TEMP`, `TMP`), no console window and, on Windows, a 256 MB memory cap.

```json
{ "id": "example", "name": "Example", "version": "1.0.0", "api": 1, "kinds": ["search"],
  "exec": ["example.exe"], "permissions": { "net": ["*"], "fs": [] } }
```

Optional `settings` describes a form Drift draws in Settings → Plugins:

```json
"settings": [
  { "key": "endpoint", "label": "Server address", "type": "url", "help": "Where it runs." },
  { "key": "token", "label": "API token", "type": "secret" }
]
```

`type` is `text`, `url`, `number`, `bool`, `secret` or `list`; a field may carry a `default` (a value valid for its kind, not for `secret` or `list`) that the form shows until the user has chosen, so a switch can start ticked; Drift saves what the form shows, so a plugin sees it as a chosen value; a `text` field of `torrent_settings` (below) may also carry `options` (at most 20 different strings of up to 60 characters), which Drift draws as a drop-down and enforces, and an `option_help` map (option → up to 400 characters, only for listed options) whose text for the chosen option is shown under the drop-down; the settings form of Settings → Plugins has no drop-downs, and a plugin whose `settings` use `options` is listed as not valid; `key` follows the same rules as `id` and must be unique. Plain values are sent to the plugin in `initialize` (`settings: {key: value}`). Secrets are never sent that way: the plugin asks for them with `get_secret`. Saving a form replaces all of the plugin's saved plain values, ends the running plugin so it restarts with the new values, and gives a plugin that was switched off after failures another chance. Values for fields the manifest no longer declares, or now declares as `secret`, are never sent. Changing or removing a secret also clears the plugin's log. A form that isn't valid is listed as a problem and the plugin can't be turned on.

A `list` field is a set of rows the user can add and remove (for example several indexers). Its `item` array describes one row with the same kinds except `list`; a list with no `item` fields, or a list inside a list, is an invalid form. Its value in `settings` is an array of objects: each has an `id` (letters, digits, `-`, `_`) plus the row's non-secret fields, at most 50 rows. A secret field of a row is asked for with `get_secret` using the key `<list key>.<row id>.<field key>`, for example `indexers.prowlarr.apikey`; the host answers only for rows that exist in the saved list, and removing a row deletes its secrets.

```json
{ "key": "indexers", "label": "Indexers", "type": "list",
  "item": [ { "key": "name", "label": "Name", "type": "text" }, { "key": "apikey", "label": "API key", "type": "secret" } ] }
```

`kinds` may hold `search`, `postprocess`, `simulator` or any combination; a kind Drift doesn't know is ignored, not an error.

`api` must be `1`. A plugin with another value, a missing program or a bad folder name is listed in Settings with the reason and can't be turned on. `permissions` is shown to the user before the first enable. It is not enforced.

## Transport

One JSON object per line (UTF-8, `\n`) on the plugin's stdin and stdout, JSON-RPC 2.0. A line over 4 MB ends the call. stderr is written to `<data>/plugin-logs/<id>.log` (cleared at each start). It is not mixed into the normal UI; Settings → Plugins → View log shows the last 32 KB on request, with the plugin's own secret values blanked out. Lines without a matching `id` (for example `{"method":"log",...}` notifications) are ignored.

## Calls (app → plugin)

| Method | Params | Result |
|---|---|---|
| `initialize` | `{api, app_version, settings}` | `{kinds, name, version}` |
| `search` | `{query, limit}` | `{results: [Result]}` |
| `completed` | `{id, name, save_path, files: [{path, size}], category, tags}` | `{actions: [Action]}` (kind `postprocess` only) |
| `torrents_updated` | `{torrents: [Torrent]}` | none: a notification (kind `simulator` only) |
| `get_selection` | `{}` | `{selections: [Selection]}` (kind `simulator` only) |
| `selection_changed` | `{info_hash, enabled, torrent, options?}` | `{}` (kind `simulator` only) |
| `get_options` | `{info_hash}` | `{options: {key: value}}` (kind `simulator` with `torrent_settings`) |
| `get_status` | `{info_hash}` | `{status: {tables, log}}` (kind `simulator` with `torrent_status`) |
| `shutdown` | `{}` | `{}` (then exit) |

`Result`: `title` (required), and at least one of `info_hash` (40 hex or 32 base32), `magnet` (`magnet:?…`), `torrent_url` (http/https). Optional: `size` (bytes), `seeders`, `leechers`, `published` (unix seconds), `category`. Rows without a title or any way to open them are dropped; titles are cut at 300 characters. A row may also carry `source`, a label (up to 60 characters) for its badge when one plugin searches several places, for example the indexer's name; without it the badge is the plugin's name.

### Post-processing

A plugin with kind `postprocess` that is switched on hears `completed` once for each torrent that finishes downloading while Drift runs (not for torrents that were already complete). `files[].path` is relative to `save_path`. If the built-in "move when finished" folder is set, Drift waits for that move first, so `save_path` is the final folder at that point. Plugins are asked one after another with the same event, so a plugin that comes after one that moved the torrent still sees the old `save_path`. A plugin has up to 5 minutes to answer `completed` (it may be copying or unpacking), and its other calls wait meanwhile.

`Action`: `{"type":"move","dest":"<absolute folder>"}` moves the torrent's files there through the engine (seeding continues from the new place); `{"type":"notify","message":"..."}` shows a message (cut at 200 characters). Anything else, a relative `dest` or an empty message is dropped; at most 20 actions. A plugin that fails (crash, timeout, bad JSON) only produces a notice saying the torrent keeps seeding; the failure rules under Lifecycle apply. Seeding never waits on this. Drift never renames or deletes files for a plugin.

### Simulators

A plugin with kind `simulator` (alone or with others) is told about the torrents and about the user's per-torrent switch. Drift owns the list and the switches; the plugin owns what it does for a selected torrent and keeps its own state. Unlike other plugins it is **not** ended after 5 minutes idle: it runs from the first message until it is switched off in Settings (Drift sends `shutdown`) or Drift exits.

**`torrents_updated`** is a JSON-RPC notification (no `id`, never answer it). Drift sends the full list of its torrents (every state; use `status`) shortly after the plugin starts, and again whenever the list differs from what the plugin was last sent. Changes are coalesced: at most one message per second, however fast byte counters move. A plugin that is restarted gets the full list again.

```json
{"jsonrpc":"2.0","method":"torrents_updated","params":{"torrents":[
  {"info_hash":"0123456789abcdef0123456789abcdef01234567","name":"ubuntu-24.04.iso","status":"downloading",
   "total_size":5872025600,"have_bytes":1200000000,"uploaded_bytes":345000000,"downloaded_bytes":1200000000,
   "trackers":["http://tracker.example.com/announce"],"content_dir":"D:\Drift\Downloads\ubuntu-24.04.iso"}]}}
```

`Torrent`: `info_hash` (lowercase hex, the same id the UI uses); `name`; `status` (`checking`, `downloading`, `seeding`, `paused`, `queued` or `error`); `total_size`, `have_bytes`, `uploaded_bytes`, `downloaded_bytes` (unsigned byte counts; Drift doesn't count received bytes separately from what it has, so `downloaded_bytes` equals `have_bytes`); `trackers` (may be empty, e.g. while a magnet's details are unknown); optional `content_dir` (save folder joined with the name) and `torrent_file_path`, left out when not known (`torrent_file_path` is not sent today). Because Drift doesn't read replies to a notification, a plugin shouldn't call `get_secret` from inside one; ask for secrets in `initialize` or while answering a request.

**`get_selection`** (2 s) asks which torrents the plugin simulates; Drift calls it when the user opens a torrent's menu and after each `selection_changed`, and caches the answer per plugin and info hash for display.

```json
{"jsonrpc":"2.0","id":10,"method":"get_selection","params":{}}
{"jsonrpc":"2.0","id":10,"result":{"selections":[
  {"info_hash":"0123456789abcdef0123456789abcdef01234567","enabled":true,"label":"Simulating: seed_only, 2 peers"},
  {"info_hash":"89abcdef0123456789abcdef0123456789abcdef","enabled":false}]}}
```

Each row needs `info_hash` (40 hex or 32 base32) and `enabled`; `label` is optional (cut at 80 characters, shown next to the menu item). Rows that don't fit are dropped. A torrent not listed is shown as off.

**`selection_changed`** (3 s) tells the plugin the user switched one torrent on or off. `torrent` is the same object as in `torrents_updated`. Start or stop your own job and answer `{}`; an error reply or no answer is shown to the user as "didn't respond". Drift then calls `get_selection` again.

```json
{"jsonrpc":"2.0","id":11,"method":"selection_changed","params":{"info_hash":"0123456789abcdef0123456789abcdef01234567","enabled":true,"torrent":{"info_hash":"0123456789abcdef0123456789abcdef01234567","name":"ubuntu-24.04.iso","status":"downloading","total_size":5872025600,"have_bytes":1200000000,"uploaded_bytes":345000000,"downloaded_bytes":1200000000,"trackers":[]}}}
{"jsonrpc":"2.0","id":11,"result":{}}
```

**Per-torrent settings.** A simulator can ask the user for settings that belong to one torrent. It declares the form in `plugin.json` with the same field format as `settings`, but only `text`, `number` and `bool` fields:

```json
"torrent_settings": [
  { "key": "mode", "label": "Mode", "type": "text", "options": ["seed_only", "loopback_pair"], "help": "What the peers do." },
  { "key": "peers", "label": "Peers", "type": "number" }
]
```

A plugin whose form is not valid is listed with the problem and can't be turned on. When the user ticks the torrent in its right-click menu, Drift first opens a dialog with this form instead of switching it on at once; a torrent that is already on gets a "Simulation settings…" entry to change them. The dialog starts from **`get_options`** (2 s): the plugin answers with the values it would use for that torrent (its own if it is simulated, else the plugin's defaults). Drift keeps only the declared keys whose values fit the field (right kind, and one of its `options`) and sends them as `options` in `selection_changed` with `enabled: true` (never with `false`). A plugin should keep only what differs from its own defaults, so what the user left alone keeps following the plugin's settings. `get_options` and `get_status` are only sent to a plugin that declares `torrent_settings` or `torrent_status`, for a valid info hash. For a torrent that is already on, `selection_changed` with changed `options` means "change the settings": the plugin applies them (restarting that torrent's job if it must) and answers `{}`. A plugin with no `torrent_settings` is never sent `options`, and the menu item just switches it on or off.

**Statistics.** A simulator with `"torrent_status": true` in `plugin.json` gets a "Simulation statistics…" entry for torrents that are on. While that window is open Drift calls **`get_status`** (2 s) every two seconds and shows the answer; nothing is requested otherwise and Drift keeps nothing. The answer is plain text to draw: `{"status":{"tables":[{"title":"Peers","columns":["Peer","Up"],"rows":[["bot-001","12 MB"]]}],"log":["12:00:03 announce ok"]}}`. At most 6 tables of 12 columns and 50 rows, cells cut at 120 characters, at most 60 log lines of 200 characters. Keep what you report in memory and small: the window is for looking, not for history.

The torrent list shows a "Simulating" badge on torrents a simulator reports as `enabled` in `get_selection` (asked after each change and every 20 s while Drift is open).

Failures: `get_selection`, `get_options` and `get_status` never count against the plugin when they only time out or the plugin doesn't know the method (Drift repeats them on its own, and looking must not switch anything off); a plugin that can't start or dies on one is counted like any other failure. `selection_changed` that times out or fails is counted apart from the lifecycle failures below; five in a row count as one failure, so opening a menu can't switch a plugin off. The menu shows a plugin that didn't answer as "not responding". If the plugin doesn't read a `torrents_updated` line for 3 s, that is a lifecycle failure.

Try it: `crates/drift-core/src/bin/drift-example-simulator-plugin.rs` simulates nothing and logs every method to its log. Manual check: install it as a plugin with `"kinds":["simulator"]`, turn it on, add two torrents, right-click one → Simulator plugins → tick it; View log shows `selection_changed` with `enabled=true` for that hash only. Untick: `enabled=false`. Restart Drift and open the menu: the log shows `get_selection` again.

## Calls (plugin → app)

| Method | Params | Result |
|---|---|---|
| `get_secret` | `{key}` | `{value}` (`null` when unset, or when `key` isn't a `secret` field in the manifest) |

This is the only request a plugin can make. Settings values and secrets live beside the plugin folders in `<data>/plugins/.settings.json` and `.secrets.json`, so updating or replacing a plugin folder keeps them.

## Lifecycle

- Started on first use, ended after 5 minutes idle (simulators excepted, see above) (`shutdown`, then killed).
- Each call has 7 seconds (the search as a whole is cut at 6, so a slow plugin never holds results back longer than that). A timeout, a crash, bad JSON or end of output kills the process and counts as a failure.
- After a failure the plugin isn't started again for 2 s, then 4 s. The third failure in a row switches it off until Drift restarts. Any good answer resets the count.
- An error reply (`"error"` in the response) means the plugin is alive; it fails that search only. The plugin can say why with `error.data.reason`: `network` (couldn't reach the service), `auth` (the service refused the key), `bad_response` (the answer couldn't be read) or `config` (not set up yet). Drift shows its own wording for the reason and never the plugin's `message`, so nothing a remote server said (or an address or key) reaches the screen. No reason, or an unknown one, counts as `bad_response`.
- A failing plugin never fails the search: the other sources still answer and the plugin shows as failed in the results panel.

## Reference

`crates/drift-core/src/bin/drift-example-plugin.rs` is the smallest working plugin (about 40 lines). Its first argument makes it misbehave (`crash`, `hang`, `noise`) for the host tests.
