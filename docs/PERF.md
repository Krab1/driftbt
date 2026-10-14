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
