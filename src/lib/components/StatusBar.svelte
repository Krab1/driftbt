<script lang="ts">
  import { formatBytes, formatSpeed } from "../../format";
  import { torrents } from "../stores/torrents.svelte";
  import { ui } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";
  import LimitMenu from "./LimitMenu.svelte";
  import Sparkline from "./Sparkline.svelte";

  const queued = $derived(torrents.all.filter((t) => t.state === "queued").length);
  const s = $derived(ui.settings);
  let limitOpen = $state(false);
  let limitBtn = $state<HTMLButtonElement>();
  const label = $derived.by(() => {
    if (ui.limits.scheduled) {
      const part = (bps: number | null, arrow: string) => (bps ? `${arrow} ${formatBytes(bps)}/s` : "");
      return `Scheduled limit ${[part(ui.limits.down_bps, "↓"), part(ui.limits.up_bps, "↑")].filter(Boolean).join(" ") || "none"}`;
    }
    if (!s?.limits_enabled) return "Speed limit off";
    const part = (kib: number, arrow: string) => (kib > 0 ? `${arrow} ${formatBytes(kib * 1024)}/s` : "");
    return `Limit ${[part(s.down_limit_kib, "↓"), part(s.up_limit_kib, "↑")].filter(Boolean).join(" ")}`;
  });

</script>

<footer>
  <span class="num" title="Total download speed"><Icon name="down" size={13} /> {formatSpeed(torrents.totals.down)}</span>
  <span class="num" title="Total upload speed"><Icon name="up" size={13} /> {formatSpeed(torrents.totals.up)}</span>
  <Sparkline down={torrents.history.map((h) => h.d)} up={torrents.history.map((h) => h.u)} label="Transfer speed over the last two minutes: download in blue, upload in green" />
  <button
    bind:this={limitBtn}
    class="lim"
    class:on={s?.limits_enabled || ui.limits.scheduled}
    aria-haspopup="dialog"
    aria-expanded={limitOpen}
    title="Choose a speed limit"
    onclick={() => (limitOpen = !limitOpen)}
  >
    <span class="dot" aria-hidden="true"></span>{label}<Icon name="chevronUp" size={12} />
  </button>
  {#if limitOpen && limitBtn}<LimitMenu anchor={limitBtn} onclose={() => (limitOpen = false)} />{/if}
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
