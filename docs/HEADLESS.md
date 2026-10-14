# Running Drift without a window (`drift-server`)

`drift-server` is the same engine, queue, rules, IP filter, VPN guard, watch folder and search as the desktop app, controlled over HTTP and from a web page it serves itself. Use it on a NAS, a mini PC or a always-on desktop.

## Start it

```
drift-server --data-dir D:\drift-data --ui-dir D:\drift\ui
```

| Option | Default | |
|---|---|---|
| `--data-dir` | `%APPDATA%\drift-server` (Linux: `~/.local/share/drift-server`) | settings, session, library, access token |
| `--bind` | `127.0.0.1:8793` | where to listen |
| `--ui-dir` | a `ui` folder next to the program, if it has an `index.html` | the built web UI (the `dist` folder from `npm run build`) |
| `--show-token` | | print the access token and exit |

First start creates the **access token** (256 random bits, kept in `<data-dir>/server.json`) and prints it once. Open `http://127.0.0.1:8793/`, paste the token, done. Settings are the same file format as the desktop app, so every option (speed limits, schedule, watch folder, proxy, IP lists, VPN guard, search sources) is in **Settings** in the web UI.

Stop it with Ctrl+C: it saves the session and exits.

## One program per data folder

A lock file (`drift.lock`) keeps the desktop app and the server, or two servers, from sharing one data folder. The second one refuses to start and says why. The lock is released by the OS when the first exits, however it exits. Give the server its own `--data-dir`.

## Reaching it from another device

By default it listens on this computer only. To use it from your phone or laptop:

- **Best:** keep `127.0.0.1` and put it behind your VPN (WireGuard, Tailscale) or an HTTPS reverse proxy (Caddy, nginx) that forwards to it.
- **Directly on your LAN:** `--bind 0.0.0.0:8793`. The server warns when you do this. It speaks plain http, so the token and your torrent list can be read by anyone on that network. TLS is not built in yet.

Every `/api/...` call except `/api/health` needs `Authorization: Bearer <token>`. There is no cookie, so a web page on another site can't ride on your session. Anyone who has the token controls the server: treat it like a password, and delete `server.json` (then restart) to make a new one.

## What doesn't work remotely

Playing a file in your player and "Show in folder" act on the server's screen, so the web UI says they aren't available. Folder and file pickers become a typed path on the server's disk. Everything else behaves as in the desktop app.

## API in one minute

```
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8793/api/torrents
curl -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
     -d '{"input":{"kind":"magnet","value":"magnet:?xt=urn:btih:..."},"category":null}' http://127.0.0.1:8793/api/torrents
curl -X POST   -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8793/api/torrents/<id>/pause
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:8793/api/torrents/<id>?delete_files=false"
curl -N        -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8793/api/events      # live snapshots (server-sent events)
```

Routes: `GET/POST /api/torrents`, `POST /api/inspect`, `DELETE /api/torrents/{id}`, `POST .../pause|resume|playlist`, `PUT .../category|tags|goal|move|files`, `GET .../details`, `GET/PUT /api/settings`, `GET /api/limits|guard|adapters|network|filter`, `POST /api/filter/update`, `POST /api/search`, `GET /api/search/keys`, `PUT /api/search/keys/{id}`, `GET /api/events`, `GET /api/health` (no token). Errors are `{ "code": "...", "message": "plain language" }` with a matching status (401 wrong token, 404 unknown torrent, 400 bad input).

## Run it at logon / at boot

These are examples; they have **not** been tested on a clean machine.

**Windows, Task Scheduler** (runs at your logon, hidden, restarts if it stops):

```powershell
$a = New-ScheduledTaskAction -Execute "D:\drift\drift-server.exe" -Argument "--data-dir D:\drift-data --ui-dir D:\drift\ui"
$t = New-ScheduledTaskTrigger -AtLogOn
$s = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit ([TimeSpan]::Zero)
Register-ScheduledTask -TaskName "Drift server" -Action $a -Trigger $t -Settings $s
Start-ScheduledTask -TaskName "Drift server"
# stop / remove:  Stop-ScheduledTask "Drift server" ; Unregister-ScheduledTask "Drift server" -Confirm:$false
```

**Linux, systemd** (`/etc/systemd/system/drift.service`):

```ini
[Unit]
Description=Drift torrent server
After=network-online.target

[Service]
User=drift
ExecStart=/opt/drift/drift-server --data-dir /var/lib/drift --ui-dir /opt/drift/ui
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

`systemctl enable --now drift`. The VPN guard needs Windows (it reads Windows adapter state); on Linux leave it off and use your VPN's own kill switch.
