<script lang="ts">
  import { torrents, type Filter } from "../stores/torrents.svelte";
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
    <button class="item" class:on={torrents.filter === it.key} aria-current={torrents.filter === it.key ? "page" : undefined} onclick={() => (torrents.filter = it.key)}>
      <Icon name={it.icon} />
      <span class="label">{it.label}</span>
      <span class="count num">{torrents.counts[it.key]}</span>
    </button>
  {/each}
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
    color: var(--accent);
  }
  .label {
    flex: 1;
  }
  .count {
    font-size: 11px;
    color: var(--text-muted);
  }
  .on .count {
    color: inherit;
  }
</style>
