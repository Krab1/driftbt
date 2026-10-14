<script lang="ts">
  import { torrents, type Filter } from "../stores/torrents.svelte";
  import { ui } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";

  const items: { key: Filter; label: string; icon: "list" | "down" | "up" | "pause" | "alert" }[] = [
    { key: "all", label: "All", icon: "list" },
    { key: "downloading", label: "Downloading", icon: "down" },
    { key: "seeding", label: "Seeding", icon: "up" },
    { key: "paused", label: "Paused", icon: "pause" },
    { key: "error", label: "Errors", icon: "alert" },
  ];
</script>

<nav aria-label="Filters">
  {#each items as it (it.key)}
    <button class="item" title="{it.label} ({torrents.counts[it.key]})" class:on={torrents.filter === it.key} aria-current={torrents.filter === it.key ? "page" : undefined} onclick={() => (torrents.filter = it.key)}>
      <Icon name={it.icon} />
      <span class="label">{it.label}</span>
      <span class="count num">{torrents.counts[it.key]}</span>
    </button>
  {/each}
  {#if ui.settings?.categories.length}
    <div class="group" id="cat-label">Categories</div>
    <div role="group" aria-labelledby="cat-label" class="cats">
      {#each ui.settings.categories as c (c.name)}
        <button class="item" class:on={torrents.category === c.name} aria-pressed={torrents.category === c.name} title="{c.name} ({torrents.categoryCounts.get(c.name) ?? 0})" onclick={() => (torrents.category = torrents.category === c.name ? null : c.name)}>
          <Icon name="folder" />
          <span class="label">{c.name}</span>
          <span class="count num">{torrents.categoryCounts.get(c.name) ?? 0}</span>
        </button>
      {/each}
    </div>
  {/if}
</nav>

<style>
  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px;
    border-right: 1px solid var(--border);
    background: var(--surface-1);
  }
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-secondary);
    text-align: left;
  }
  .item:hover {
    background: var(--hover);
  }
  .item.on {
    background: var(--selected);
    color: var(--accent-fg);
  }
  .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .group {
    margin: 12px 8px 4px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .cats {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  @media (max-width: 760px) {
    .group {
      display: none;
    }
  }
  .count {
    font-size: 11px;
    color: var(--text-muted);
  }
  .on .count {
    color: inherit;
  }
  /* Icon-only rail when narrow; labels stay available to screen readers. */
  @media (max-width: 760px) {
    .label,
    .count {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip: rect(0 0 0 0);
      white-space: nowrap;
    }
    .item {
      justify-content: center;
      padding: 0;
    }
  }
</style>
