# Performance

Measured 2026-10-14, Windows 11, 2560×1440 display, release build unless noted. Re-measure after big changes.

## Budgets and results

| Budget | Result | Notes |
|---|---|---|
| Cold start to visible window < 1 s | **303 ms** | `drift.exe` start → window titled "Drift" present. Real engine, empty list |
| Memory (app + all WebView2 processes), private | **119 MB** (was 180 MB before the GPU change) | `drift.exe` ≈ 38 MB; WebView2 helpers the rest. Working set shows ~360 MB because WebView2 shares pages with other WebView2 apps |
| Idle CPU | **0.2 %** of one core | `drift.exe`, window visible, no torrents |
| Executable size | **8.6 MB** | Frontend embedded. Installer size not yet measured (M3-04) |
| 60 fps with 1,000 torrents | **60.3 fps, 0 frames over 25 ms** | 4 s scripted scroll in the real window, 1100×700 and maximized 2560×1369, GPU on and off |
| UI cost per snapshot at 1,000 torrents | parse 0.27 ms, apply + filter + sort + render **1.2 ms p50 / 2.0 ms p95** | Payload is 262 KB per tick (268 B per torrent). Cost is negligible, so no delta encoding yet; revisit for the remote UI over a network (M6-09) |
| Scroll through 1,000 rows (browser mock) | p50 5 ms, p95 11 ms per jump | 20–21 DOM rows regardless of list size |

## Decisions from the numbers
- The original "< 80 MB including WebView2" target is not reachable: WebView2's own helper processes cost about 80–100 MB private even for a blank page. Target restated as **≤ 130 MB private, total**.
- `--disable-gpu` is set in `tauri.conf.json` (`additionalBrowserArgs`). It saves about 60 MB private, and the list UI holds 60 fps without it, even maximized at 2560×1369. If a very large or high-DPI display shows jank, make it a setting.
- Tried and rejected: `--enable-low-end-device-mode` (no effect), `--in-process-gpu` (−20 MB but less stable; superseded by disabling the GPU).
- Debug builds are heavier (≈ 215 MB private); only release numbers count.

## How to re-measure
Memory: run the release exe, sum private memory of `drift.exe` and the `msedgewebview2.exe` processes whose command line contains `app.drift.client`. Frames: `DRIFT_FAKE=1000 DRIFT_CDP_PORT=9222` on a debug build, then `node scripts/cdp.mjs evalfile frames.js` (a 4 s `requestAnimationFrame` loop that scrolls the list). Debug-only `DRIFT_WV_ARGS` passes extra Chromium flags for experiments.

## Review of 2026-11-16: large torrents and bulk actions
The numbers above are for the everyday case (1,000 torrents, list view). A read through the code for what grows with file count and torrent count found these; the frontend figures are Node micro-benchmarks of the same functions, not measurements in the app, and the Rust-side ones were not measured.

| What | Before | After |
|---|---|---|
| Files tab, building the file tree (runs on every details update, 2 Hz) | 5,000 files in one folder 46 ms; 20,000 in one folder 620 ms; 20,000 nested 79 ms | 2.5 ms; 8-19 ms; 15 ms (a map for the folder lookup, one kept `Intl.Collator`) |
| "Pause all" / "Resume all" | one IPC call per torrent, each with a full library save, and each resume with a full scheduling round over every torrent | one call, one save, one round (`Manager::pause_many` / `resume_many`); the VPN guard uses the same path |
| Details update for a torrent with 20,000 files | 2.5 MB of JSON twice a second on every tab | the file list only on the Files tab, the peers only on the Peers tab |
| Speed history for the detail graph | 120 samples kept for every torrent: about 12 MB of heap per 1,000 torrents | kept for the selected torrent only (the graph starts again when you select another) |
| Closing idle write handles | one 1 Hz task per torrent, paused and seeding ones included | one task per session |
| Hidden or minimised window | the whole list was still serialised and sent every 5 s | nothing is sent; the page refreshes when shown |
| Date columns (Added, Finished) | 1 ms per update for 20 visible rows (`toLocaleDateString` sets up a formatter per call) | 0.02 ms (formatters kept per language) |
| Copies per scheduling round | every torrent's stats copied twice more after the engine made them, the whole settings copied twice | none of those |

Checked and left alone: list filter + sort (0.4 ms per update at 1,000 torrents, 2.7 ms at 5,000); the size-optimised release profile (SHA-1 goes through the Windows crypto API, so hashing is not affected); sending a `.torrent` as a JSON number array (3.6x its size, but once per add).

Not re-measured: idle CPU and memory with a large real list. The idle figure above is with no torrents.
