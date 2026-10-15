<script lang="ts">
  import { torrents } from "../stores/torrents.svelte";
  import { ui } from "../stores/ui.svelte";
  import { api } from "../api";
  import Icon from "./Icon.svelte";
  import WindowControls from "./WindowControls.svelte";

  let input = $state<HTMLInputElement>();

  export function focusSearch() {
    input?.focus();
    input?.select();
  }
</script>

<header data-tauri-drag-region>
  <button class="btn ghost icon gear" aria-label="Settings" title="Settings" onclick={() => ui.openSettings()}><Icon name="gear" size={18} /></button>
  <label class="search">
    <Icon name="search" />
    <input
      bind:this={input}
      type="search"
      placeholder="Search torrents"
      aria-label="Search torrents"
      value={torrents.query}
      oninput={(e) => (torrents.query = e.currentTarget.value)}
      onkeydown={(e) => e.key === "Escape" && ((torrents.query = ""), e.currentTarget.blur())}
    />
  </label>
  <button class="btn" onclick={() => (ui.searchOpen = true)} title="Search for torrents (Ctrl+Shift+F)"><Icon name="search" />Find online</button>
  <button class="btn primary" onclick={() => ui.openAdd()}><Icon name="plus" />Add torrent</button>
  {#if api.chrome}<WindowControls />{/if}
</header>

<style>
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 48px;
    padding: 0 12px;
    border-bottom: 1px solid var(--border);
  }
  .gear {
    width: 32px;
    height: 32px;
    color: var(--text-secondary);
  }
  .gear:hover {
    color: var(--text-primary);
  }
  .search {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--radius);
    background: var(--surface-1);
    color: var(--text-muted);
    max-width: 560px;
  }
  .search:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: 0;
  }
  input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--text-primary);
    user-select: text;
  }
  input::placeholder {
    color: var(--text-muted);
  }
  /* The gap around the search box is part of the drag region (the header itself). */
  .search {
    margin-right: auto;
  }
</style>
