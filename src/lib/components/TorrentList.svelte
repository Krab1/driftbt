<script lang="ts">
  import { api } from "../api";
  import { torrents, type SortKey } from "../stores/torrents.svelte";
  import { ui, errorText } from "../stores/ui.svelte";
  import TorrentRow from "./TorrentRow.svelte";
  import Icon from "./Icon.svelte";

  const ROW = 44;
  const BUFFER = 6;

  let viewport = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let height = $state(0);

  const items = $derived(torrents.visible);
  const start = $derived(Math.max(0, Math.floor(scrollTop / ROW) - BUFFER));
  const end = $derived(Math.min(items.length, Math.ceil((scrollTop + height) / ROW) + BUFFER));
  const slice = $derived(items.slice(start, end));
  const selIndex = $derived(items.findIndex((t) => t.id === torrents.selectedId));

  const heads: { key: SortKey; label: string; right?: boolean }[] = [
    { key: "name", label: "Name" },
    { key: "progress", label: "Done", right: true },
    { key: "size", label: "Size", right: true },
    { key: "speed", label: "Speed", right: true },
    { key: "eta", label: "ETA", right: true },
  ];

  async function toggle(id: string) {
    const t = torrents.all.find((x) => x.id === id);
    if (!t) return;
    try {
      await (t.state === "paused" ? api.resume(id) : api.pause(id));
      await torrents.refresh();
    } catch (e) {
      ui.toast(errorText(e), "error");
    }
  }

  function askRemove(id: string) {
    const t = torrents.all.find((x) => x.id === id);
    if (t) ui.removeTarget = { id, name: t.name };
  }

  function select(i: number) {
    const t = items[Math.max(0, Math.min(items.length - 1, i))];
    if (!t || !viewport) return;
    torrents.selectedId = t.id;
    const i2 = items.indexOf(t);
    const top = i2 * ROW;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (top + ROW > viewport.scrollTop + height) viewport.scrollTop = top + ROW - height;
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") (e.preventDefault(), select(selIndex < 0 ? 0 : selIndex + 1));
    else if (e.key === "ArrowUp") (e.preventDefault(), select(selIndex < 0 ? 0 : selIndex - 1));
    else if (e.key === "Home") (e.preventDefault(), select(0));
    else if (e.key === "End") (e.preventDefault(), select(items.length - 1));
    else if (e.key === " " && torrents.selectedId) (e.preventDefault(), toggle(torrents.selectedId));
    else if (e.key === "Delete" && torrents.selectedId) askRemove(torrents.selectedId);
  }

  function oncontext(e: MouseEvent, id: string) {
    e.preventDefault();
    torrents.selectedId = id;
    ui.menu = { x: e.clientX, y: e.clientY, id };
  }
</script>

<div class="list" role="grid" aria-label="Torrents" aria-rowcount={items.length + 1} aria-activedescendant={torrents.selectedId ? `row-${torrents.selectedId}` : undefined} tabindex="0" {onkeydown}>
  <div class="head" role="row" aria-rowindex="1">
    {#each heads as h (h.key)}
      <button class="th" class:r={h.right} role="columnheader" aria-sort={torrents.sortKey === h.key ? (torrents.sortDir === 1 ? "ascending" : "descending") : "none"} onclick={() => torrents.sortBy(h.key)}>
        {h.label}{#if torrents.sortKey === h.key}<span class="arrow" aria-hidden="true">{torrents.sortDir === 1 ? "▲" : "▼"}</span>{/if}
      </button>
    {/each}
    <span></span>
  </div>

  {#if torrents.all.length === 0}
    <div class="empty">
      <Icon name="upload" size={28} />
      <h2>Add your first torrent</h2>
      <p>Drop a .torrent file here, or paste a magnet link anywhere in the window.</p>
      <button class="btn primary" onclick={() => ui.openAdd()}><Icon name="plus" />Add torrent</button>
    </div>
  {:else if items.length === 0}
    <div class="empty">
      <h2>No matches</h2>
      <p>Nothing in this view fits the current search or filter.</p>
    </div>
  {:else}
    <div class="viewport" bind:this={viewport} bind:clientHeight={height} onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}>
      <div class="space" style="height: {items.length * ROW}px">
        {#each slice as t, i (t.id)}
          <div class="slot" style="transform: translateY({(start + i) * ROW}px)">
            <TorrentRow
              {t}
              index={start + i}
              selected={t.id === torrents.selectedId}
              onselect={() => (torrents.selectedId = t.id)}
              oncontext={(e) => oncontext(e, t.id)}
              ontoggle={() => toggle(t.id)}
              onremove={() => askRemove(t.id)}
            />
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .list {
    --cols: minmax(0, 1fr) 56px 76px 84px 64px 92px;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    outline-offset: -2px;
  }
  .head {
    display: grid;
    grid-template-columns: var(--cols);
    gap: 12px;
    padding: 0 12px;
    height: 30px;
    align-items: center;
    border-bottom: 1px solid var(--border);
    color: var(--text-muted);
    font-size: 11px;
  }
  .th {
    background: none;
    border: 0;
    padding: 0;
    text-align: left;
    color: inherit;
    font-size: inherit;
  }
  .th.r {
    text-align: right;
  }
  .th:hover {
    color: var(--text-primary);
  }
  .arrow {
    margin-left: 4px;
    font-size: 8px;
  }
  .viewport {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .space {
    position: relative;
  }
  .slot {
    position: absolute;
    inset: 0 0 auto 0;
    height: 44px;
  }
  .empty {
    flex: 1;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 8px;
    color: var(--text-secondary);
    text-align: center;
  }
  .empty h2 {
    margin: 4px 0 0;
    font-size: 15px;
    font-weight: 500;
    color: var(--text-primary);
  }
  .empty p {
    margin: 0 0 8px;
    max-width: 320px;
  }
</style>
