# UI spec

Direction: quiet, dense, flat — an instrument, not a dashboard. Reference sketch from the design session: top bar, filter rail, list with hairline rows and 3px progress bars, detail strip, status footer.
Build with the `ui-ux-pro-max` skill as the base, `taste-skill:minimalist-ui` for the visual direction, `dataviz` for the speed graph; final pass with `web-design-guidelines` and `design:accessibility-review`.

## Layout (desktop, min window 900×560)
```
┌ top bar: settings (gear) · search · [Add] ───────────────┐
├ rail 168px ┬ list (virtualised) ──────────────────────────┤
│ All        │ Name+progress │ Size │ Speed │ ETA │ ⋯      │
│ Downloading│ …                                            │
│ Seeding    ├ detail panel (resizable, collapsible) ───────┤
│ Paused     │ Overview · Files · Peers · Trackers          │
│ Categories │                                              │
├ status bar: ↓ speed · ↑ speed · speed-limit menu · DHT ──────┤
```
Window frame: custom title bar integrated with the top bar (Tauri decorations off), draggable region, native caption buttons feel (Windows 11 snap layouts must still work).

## Tokens (CSS custom properties, one file `src/styles/tokens.css`)
- Surfaces: `--bg` (page), `--surface-1` (rail, panels), `--surface-2` (menus). Dark and light sets, switched by `data-theme`; default follows `prefers-color-scheme`.
- Text: `--text-primary`, `--text-secondary`, `--text-muted`. Never lower opacity for hierarchy.
- One accent (`--accent`, blue) for selection and downloading. State colours: success (seeding/complete), warning, danger, neutral (paused). No other hues.
- Borders: 1px hairline `--border`; no shadows except menus/dialogs.
- Radius: 6px controls, 10px dialogs. Spacing scale 4/8/12/16/24.
- Type: system UI font stack (Segoe UI Variable); numbers use `font-variant-numeric: tabular-nums`. Two weights: 400, 500. Sentence case everywhere.
- Row height 44px (compact option 32px). Touch/click targets ≥ 28px.
- Motion: 120–160 ms, ease-out, only on state changes; respect `prefers-reduced-motion`.

## Components
Top bar · Filter rail item (icon, label, count) · Torrent row · Progress bar (3px; accent=downloading, success=seeding, neutral=paused, danger=error) · Detail tabs · File tree with tri-state checkboxes · Peer table · Status bar · Add dialog · Settings (single page, sections: General, Downloads, Network, Limits and schedule, Appearance) · Context menu · Toast · Command palette · Empty state · Confirm dialog.

## Key flows
1. **Add:** paste a magnet anywhere, drop a .torrent, or click Add → dialog shows name, size, file tree (checkboxes), save folder, category → `Add` (primary, the only filled button). Metadata for magnets loads inline with a skeleton row; the dialog stays usable.
2. **Row actions:** hover shows pause/resume and open-folder icons; right-click shows full menu; double-click opens the folder (or plays, if video and streaming enabled).
3. **Play while downloading:** video files show a Play button in the Files tab and the row menu.
4. **Remove:** confirm dialog with a "Also delete files" checkbox, default off, remembered.

5. **Search:** the top-bar field has two modes, "Filter" (current list, default) and "Search sources", toggled with a chip or `Ctrl+Shift+F`. Search mode replaces the list with results: title, size, seeders, age, source badges (one per provider that returned it), and a quiet `Add` button per row (opens the Add dialog pre-filled; Shift+click adds straight away). A thin strip under the bar shows per-source status ("3 of 4 sources responded", expandable to see which failed and why). Esc returns to the list.
6. **Providers (Settings → Search):** list of sources with toggle, a "Test" action, and "Add Torznab source" (name, URL, API key). Empty first-run state: "No search sources yet. Add a Torznab source to search from here." plus the built-in Internet Archive toggle.
6a. **Simulator plugins:** the context menu has a "Simulator plugins" group. A plugin with a per-torrent form opens its dialog (Simulation settings, drawn from the plugin's form) when the torrent is ticked, and offers "Simulation settings…" and, if it has statistics, "Simulation statistics…" once it is on. A torrent being simulated shows a small "Simulating" badge next to its name, like the "Limited" badge. The statistics window refreshes every 2 s while open and keeps nothing.
7. **Playlist:** context menu on a multi-file torrent: "Create playlist" and "Copy playlist path"; setting "Create playlist when a multi-file download finishes".

8. **IP filter (Settings → Network):** master toggle; list of subscriptions (name, URL, enabled switch, ranges count, last updated, status); actions `Update now`, `Remove`, `Add list` (URL), `Import file`. A "Suggested" row offers the eMule Security list, off by default. One caption under the toggle: "Blocks connections to and from listed addresses. It doesn't hide your IP; use a VPN or bind to an interface for that." Status bar shows a shield icon with the blocked-peer count when the filter is on.

## States (all must be designed and implemented)
IP filter: not enabled · downloading list · updated (count) · update failed, using previous list · list rejected (too broad / empty / too large) with reason · offline.
Search: idle · searching (skeleton rows, per-source spinner) · no results · all sources failed (with reason and Retry) · partial results · no sources configured.
Empty list (invitation: "Add your first torrent", drop zone, paste hint) · No search results · Loading metadata · Checking files (progress) · Queued · Paused · Error (plain message + Retry) · Disk full · Offline / no peers · Seeding · Complete · VPN guard tripped (banner).

## Copy rules
Sentence case, verbs first on buttons ("Add torrent"), no "please", no "successfully", no exclamation marks, no raw error strings. Contractions allowed. Errors say what happened and what to do.

## Accessibility
Full keyboard use (arrow keys in list, Space pause, Del remove with confirm, Ctrl+K palette). Visible focus ring. Contrast ≥ 4.5:1 for text, 3:1 for progress bars. List uses proper roles (`grid`/`row`). Colour is never the only status signal (icon or text too).

## Performance rules for the UI
Virtualise the list; update rows by key, not full re-render; never animate width of 1,000 bars at once (use `transform: scaleX`); detail panel subscribes only for the selected torrent.

## Verification
Check at 900×560, 1280×800, 1920×1080, at 125%/150% DPI, in both themes. Measure row height, truncation, and contrast in the running app; attach screenshots to the task.
