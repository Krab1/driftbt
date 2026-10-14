<script lang="ts">
  import type { TorrentStats } from "../api";
  import { formatBytes, formatEta, formatSpeed, percent } from "../../format";
  import Icon from "./Icon.svelte";

  let {
    t,
    selected,
    index,
    onselect,
    oncontext,
    ontoggle,
    onremove,
  }: {
    t: TorrentStats;
    selected: boolean;
    index: number;
    onselect: () => void;
    oncontext: (e: MouseEvent) => void;
    ontoggle: () => void;
    onremove: () => void;
  } = $props();

  const pct = $derived(percent(t.progress_bytes, t.total_bytes));
  const speed = $derived(t.state === "seeding" ? t.up_bps : t.down_bps);
  const icon = $derived(
    ({ downloading: "down", seeding: "up", paused: "pause", error: "alert", checking: "clock" } as const)[t.state],
  );
  const label = $derived(
    ({ downloading: "Downloading", seeding: "Seeding", paused: "Paused", error: "Error", checking: "Checking" } as const)[t.state],
  );
  const canToggle = $derived(t.state !== "checking" && t.state !== "error");
</script>

<!-- Keyboard input is handled by the parent grid (aria-activedescendant), so rows need no key handler. -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  class="row"
  class:selected
  role="row"
  id="row-{t.id}"
  aria-rowindex={index + 2}
  aria-selected={selected}
  onclick={onselect}
  oncontextmenu={oncontext}
  tabindex="-1"
>
  <div class="name" role="gridcell">
    <span class="state {t.state}" title={t.error ?? label}>
      <Icon name={icon} size={15} />
      <span class="sr-only">{label}</span>
    </span>
    <div class="nm">
      <span class="text" title={t.name}>{t.name}</span>
      <div class="bar" role="progressbar" aria-valuenow={pct} aria-valuemin="0" aria-valuemax="100" aria-label="Progress">
        <i class={t.state} style="transform: scaleX({pct / 100})"></i>
      </div>
    </div>
  </div>
  <div class="num r" role="gridcell">{t.state === "error" ? "" : `${pct}%`}</div>
  <div class="num r" role="gridcell">{t.total_bytes ? formatBytes(t.total_bytes) : "—"}</div>
  <div class="num r" role="gridcell">{formatSpeed(speed)}</div>
  <div class="num r" role="gridcell">{t.state === "downloading" ? formatEta(t.eta_secs) : "—"}</div>
  <div class="actions" role="gridcell">
    {#if canToggle}
      <button class="btn ghost icon" aria-label={t.state === "paused" ? "Resume" : "Pause"} onclick={(e) => { e.stopPropagation(); ontoggle(); }}>
        <Icon name={t.state === "paused" ? "play" : "pause"} />
      </button>
    {/if}
    <button class="btn ghost icon" aria-label="Remove" onclick={(e) => { e.stopPropagation(); onremove(); }}>
      <Icon name="trash" />
    </button>
  </div>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: var(--cols);
    align-items: center;
    gap: 12px;
    height: var(--row-h);
    padding: 0 12px;
    border-bottom: 1px solid var(--border);
    cursor: default;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.selected {
    background: var(--selected);
  }
  .name {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .state {
    flex: none;
    display: grid;
    place-items: center;
    color: var(--neutral);
  }
  .state.downloading,
  .state.checking {
    color: var(--accent);
  }
  .state.seeding {
    color: var(--success);
  }
  .state.error {
    color: var(--danger);
  }
  .nm {
    min-width: 0;
    flex: 1;
  }
  .text {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bar {
    height: 3px;
    margin-top: 6px;
    border-radius: 2px;
    background: var(--track);
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    width: 100%;
    transform-origin: left;
    background: var(--neutral);
    transition: transform var(--dur) linear;
  }
  .bar i.downloading,
  .bar i.checking {
    background: var(--accent);
  }
  .bar i.seeding {
    background: var(--success);
  }
  .bar i.error {
    background: var(--danger);
  }
  .r {
    text-align: right;
    color: var(--text-secondary);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 2px;
    opacity: 0;
  }
  .row:hover .actions,
  .row.selected .actions,
  .row:focus-within .actions {
    opacity: 1;
  }
</style>
