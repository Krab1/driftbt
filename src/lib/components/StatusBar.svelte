<script lang="ts">
  import { formatBytes, formatSpeed } from "../../format";
  import { torrents } from "../stores/torrents.svelte";
  import { ui } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";
  import LimitMenu from "./LimitMenu.svelte";
  import Sparkline from "./Sparkline.svelte";

  const queued = $derived(torrents.all.filter((t) => t.state === "queued").length);
  const s = $derived(ui.settings);
  // Which direction's limit popover is open, anchored to its button.
  let open = $state<"down" | "up" | null>(null);
  let anchor = $state<HTMLButtonElement>();
  // The limit in force for a direction in bytes/s (a schedule wins), or 0 for none.
  const cap = (dir: "down" | "up") =>
    ui.limits.scheduled ? ((dir === "down" ? ui.limits.down_bps : ui.limits.up_bps) ?? 0) : s?.limits_enabled ? (dir === "down" ? s.down_limit_kib : s.up_limit_kib) * 1024 : 0;
  function toggle(dir: "down" | "up", e: MouseEvent) {
    anchor = e.currentTarget as HTMLButtonElement;
    open = open === dir ? null : dir;
  }
</script>

<footer>
  <button class="lim num" class:on={cap("down") > 0} aria-haspopup="dialog" aria-expanded={open === "down"} title={cap("down") ? `Download speed, limited to ${formatBytes(cap("down"))}/s. Click to change.` : "Download speed, no limit. Click to set a limit."} onclick={(e) => toggle("down", e)}>
    <Icon name="down" size={13} /> {formatSpeed(torrents.totals.down)}{#if cap("down")}<span class="of">/ {formatBytes(cap("down"))}/s</span>{/if}
  </button>
  <button class="lim num" class:on={cap("up") > 0} aria-haspopup="dialog" aria-expanded={open === "up"} title={cap("up") ? `Upload speed, limited to ${formatBytes(cap("up"))}/s. Click to change.` : "Upload speed, no limit. Click to set a limit."} onclick={(e) => toggle("up", e)}>
    <Icon name="up" size={13} /> {formatSpeed(torrents.totals.up)}{#if cap("up")}<span class="of">/ {formatBytes(cap("up"))}/s</span>{/if}
  </button>
  <Sparkline down={torrents.history.map((h) => h.d)} up={torrents.history.map((h) => h.u)} label="Transfer speed over the last two minutes: download in blue, upload in green" />
  {#if open && anchor}<LimitMenu dir={open} {anchor} onclose={() => (open = null)} />{/if}
  <button class="lim" onclick={() => (ui.ratioOpen = true)}>Ratios</button>
  <span class="end num">{torrents.counts.all} {torrents.counts.all === 1 ? "torrent" : "torrents"} · {torrents.counts.downloading - queued} active{#if queued}{" · "}{queued} queued{/if}</span>
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
  .of {
    color: var(--text-muted);
  }
  .lim.on {
    color: var(--accent-fg);
  }
  .end {
    margin-left: auto;
    color: var(--text-muted);
  }
</style>
