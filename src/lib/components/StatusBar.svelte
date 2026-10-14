<script lang="ts">
  import { formatBytes, formatSpeed } from "../../format";
  import { torrents } from "../stores/torrents.svelte";
  import { ui } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";

  const s = $derived(ui.settings);
  const hasValues = $derived(!!s && (s.down_limit_kib > 0 || s.up_limit_kib > 0));
  const label = $derived.by(() => {
    if (!s?.limits_enabled) return "Speed limit off";
    const part = (kib: number, arrow: string) => (kib > 0 ? `${arrow} ${formatBytes(kib * 1024)}/s` : "");
    return `Limit ${[part(s.down_limit_kib, "↓"), part(s.up_limit_kib, "↑")].filter(Boolean).join(" ")}`;
  });

  function toggleLimit() {
    if (!s) return;
    // Turning limits on without any value would do nothing; send the user to set one.
    if (!s.limits_enabled && !hasValues) ui.settingsOpen = true;
    else ui.saveSettings({ limits_enabled: !s.limits_enabled });
  }
</script>

<footer>
  <span class="num" title="Total download speed"><Icon name="down" size={13} /> {formatSpeed(torrents.totals.down)}</span>
  <span class="num" title="Total upload speed"><Icon name="up" size={13} /> {formatSpeed(torrents.totals.up)}</span>
  <button class="lim" class:on={s?.limits_enabled} role="switch" aria-checked={s?.limits_enabled ?? false} aria-label="Speed limit" onclick={toggleLimit}>
    <span class="dot" aria-hidden="true"></span>{label}
  </button>
  <span class="end num">{torrents.counts.all} {torrents.counts.all === 1 ? "torrent" : "torrents"} · {torrents.counts.downloading} active</span>
</footer>

<style>
  footer {
    display: flex;
    align-items: center;
    gap: 16px;
    height: 26px;
    padding: 0 12px;
    border-top: 1px solid var(--border);
    font-size: 11px;
    color: var(--text-secondary);
  }
  span {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .lim {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 20px;
    padding: 0 6px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: inherit;
    font-size: inherit;
  }
  .lim:hover {
    background: var(--hover);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    border: 1px solid var(--neutral);
  }
  .lim.on {
    color: var(--accent-fg);
  }
  .lim.on .dot {
    background: var(--accent);
    border-color: var(--accent);
  }
  .end {
    margin-left: auto;
    color: var(--text-muted);
  }
</style>
