<script lang="ts">
  import type { Torrent } from "../api";
  import { percent } from "../../format";
  import { columns } from "../stores/columns.svelte";
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
    t: Torrent;
    selected: boolean;
    index: number;
    onselect: () => void;
    oncontext: (e: MouseEvent) => void;
    ontoggle: () => void;
    onremove: () => void;
  } = $props();

  const pct = $derived(percent(t.progress_bytes, t.total_bytes));
  const icon = $derived(
    ({ downloading: "down", seeding: "up", paused: "pause", queued: "list", error: "alert", checking: "clock" } as const)[t.state],
  );
  const label = $derived(
    ({ downloading: "Downloading", seeding: "Seeding", paused: "Paused", queued: "Queued", error: "Error", checking: "Checking" } as const)[t.state],
  );
  const canToggle = $derived(t.state !== "checking" && t.state !== "error");
  // Magnets show their hash until metadata arrives; say what is happening instead.
  const fetching = $derived(t.state === "checking" && t.total_bytes === 0);
  const caption = $derived(
    t.state === "queued"
      ? `Waiting for a free slot · #${t.queue_pos ?? "?"} in queue`
      : t.note
        ? t.note
        : fetching
      ? "Fetching torrent info…"
      : t.state === "checking"
        ? "Checking files…"
        : t.state === "downloading" && t.peers_live === 0 && t.down_bps === 0
          ? "Looking for peers…"
          : t.state === "error"
            ? (t.error ?? "Something went wrong.")
            : "",
  );
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
      <span class="line">
        <span class="text" class:muted={fetching} title={t.name}>{fetching ? "New torrent" : t.name}</span>
        {#if caption}<span class="cap" class:bad={t.state === "error"} title={caption}>{caption}</span>{/if}
      </span>
      <div class="bar" role="progressbar" aria-valuenow={pct} aria-valuemin="0" aria-valuemax="100" aria-label="Progress">
        <i class={t.state} style="transform: scaleX({pct / 100})"></i>
      </div>
    </div>
  </div>
  {#each columns.shown as c (c.key)}
    <div class="num r cell c-{c.key}" role="gridcell" title={c.key === "added" || c.key === "finished" || c.key === "tracker" ? c.text(t) : undefined}>{c.text(t)}</div>
  {/each}
  <div class="actions" role="gridcell">
    {#if t.state === "error"}
      <button class="btn ghost retry" onclick={(e) => { e.stopPropagation(); ontoggle(); }}>Retry</button>
    {/if}
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
  .state.checking,
  .state.queued {
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
  .line {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .text.muted {
    color: var(--text-secondary);
  }
  .cap {
    flex: none;
    max-width: 55%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
    color: var(--text-muted);
  }
  .cap.bad {
    color: var(--danger-fg);
  }
  .retry {
    height: 24px;
    padding: 0 8px;
    color: var(--accent-fg);
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
  /* Finished torrents are done news: keep their bar quiet so active transfers stand out. */
  .bar i.seeding {
    background: var(--success);
    opacity: 0.45;
  }
  .bar i.error {
    background: var(--danger);
  }
  .r {
    text-align: right;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 2px;
  }
  /* Icon actions appear on hover/selection; the Retry button on an errored row is always visible. */
  .actions :global(.btn.icon) {
    opacity: 0;
  }
  .row:hover .actions :global(.btn.icon),
  .row.selected .actions :global(.btn.icon),
  .row:focus-within .actions :global(.btn.icon) {
    opacity: 1;
  }
</style>
