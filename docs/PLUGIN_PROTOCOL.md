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

`type` is `text`, `url`, `number`, `bool`, `secret` or `list`; `key` follows the same rules as `id` and must be unique. Plain values are sent to the plugin in `initialize` (`settings: {key: value}`). Secrets are never sent that way: the plugin asks for them with `get_secret`. Saving a form replaces all of the plugin's saved plain values, ends the running plugin so it restarts with the new values, and gives a plugin that was switched off after failures another chance. Values for fields the manifest no longer declares, or now declares as `secret`, are never sent. Changing or removing a secret also clears the plugin's log. A form that isn't valid is listed as a problem and the plugin can't be turned on.

A `list` field is a set of rows the user can add and remove (for example several indexers). Its `item` array describes one row with the same kinds except `list`; a list with no `item` fields, or a list inside a list, is an invalid form. Its value in `settings` is an array of objects: each has an `id` (letters, digits, `-`, `_`) plus the row's non-secret fields, at most 50 rows. A secret field of a row is asked for with `get_secret` using the key `<list key>.<row id>.<field key>`, for example `indexers.prowlarr.apikey`; the host answers only for rows that exist in the saved list, and removing a row deletes its secrets.

```json
{ "key": "indexers", "label": "Indexers", "type": "list",
  "item": [ { "key": "name", "label": "Name", "type": "text" }, { "key": "apikey", "label": "API key", "type": "secret" } ] }
```

`api` must be `1`. A plugin with another value, a missing program or a bad folder name is listed in Settings with the reason and can't be turned on. `permissions` is shown to the user before the first enable. It is not enforced.

## Transport

One JSON object per line (UTF-8, `\n`) on the plugin's stdin and stdout, JSON-RPC 2.0. A line over 4 MB ends the call. stderr is written to `<data>/plugin-logs/<id>.log` (cleared at each start). It is not mixed into the normal UI; Settings → Plugins → View log shows the last 32 KB on request, with the plugin's own secret values blanked out. Lines without a matching `id` (for example `{"method":"log",...}` notifications) are ignored.

## Calls (app → plugin)

| Method | Params | Result |
|---|---|---|
| `initialize` | `{api, app_version, settings}` | `{kinds, name, version}` |
| `search` | `{query, limit}` | `{results: [Result]}` |
| `completed` | `{id, name, save_path, files: [{path, size}], category, tags}` | `{actions: [Action]}` (kind `postprocess` only) |
| `shutdown` | `{}` | `{}` (then exit) |

`Result`: `title` (required), and at least one of `info_hash` (40 hex or 32 base32), `magnet` (`magnet:?…`), `torrent_url` (http/https). Optional: `size` (bytes), `seeders`, `leechers`, `published` (unix seconds), `category`. Rows without a title or any way to open them are dropped; titles are cut at 300 characters. A row may also carry `source`, a label (up to 60 characters) for its badge when one plugin searches several places, for example the indexer's name; without it the badge is the plugin's name.

### Post-processing

A plugin with kind `postprocess` that is switched on hears `completed` once for each torrent that finishes downloading while Drift runs (not for torrents that were already complete). `files[].path` is relative to `save_path`. If the built-in "move when finished" folder is set, Drift waits for that move first, so `save_path` is the final folder at that point. Plugins are asked one after another with the same event, so a plugin that comes after one that moved the torrent still sees the old `save_path`. A plugin has up to 5 minutes to answer `completed` (it may be copying or unpacking), and its other calls wait meanwhile.

`Action`: `{"type":"move","dest":"<absolute folder>"}` moves the torrent's files there through the engine (seeding continues from the new place); `{"type":"notify","message":"..."}` shows a message (cut at 200 characters). Anything else, a relative `dest` or an empty message is dropped; at most 20 actions. A plugin that fails (crash, timeout, bad JSON) only produces a notice saying the torrent keeps seeding; the failure rules under Lifecycle apply. Seeding never waits on this. Drift never renames or deletes files for a plugin.

## Calls (plugin → app)

| Method | Params | Result |
|---|---|---|
| `get_secret` | `{key}` | `{value}` (`null` when unset, or when `key` isn't a `secret` field in the manifest) |

This is the only request a plugin can make. Settings values and secrets live beside the plugin folders in `<data>/plugins/.settings.json` and `.secrets.json`, so updating or replacing a plugin folder keeps them.

## Lifecycle

- Started on first use, ended after 5 minutes idle (`shutdown`, then killed).
- Each call has 7 seconds (the search as a whole is cut at 6, so a slow plugin never holds results back longer than that). A timeout, a crash, bad JSON or end of output kills the process and counts as a failure.
- After a failure the plugin isn't started again for 2 s, then 4 s. The third failure in a row switches it off until Drift restarts. Any good answer resets the count.
- An error reply (`"error"` in the response) means the plugin is alive; it fails that search only. The plugin can say why with `error.data.reason`: `network` (couldn't reach the service), `auth` (the service refused the key), `bad_response` (the answer couldn't be read) or `config` (not set up yet). Drift shows its own wording for the reason and never the plugin's `message`, so nothing a remote server said (or an address or key) reaches the screen. No reason, or an unknown one, counts as `bad_response`.
- A failing plugin never fails the search: the other sources still answer and the plugin shows as failed in the results panel.

## Reference

`crates/drift-core/src/bin/drift-example-plugin.rs` is the smallest working plugin (about 40 lines). Its first argument makes it misbehave (`crash`, `hang`, `noise`) for the host tests.
