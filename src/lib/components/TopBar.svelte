<script lang="ts">
  import { torrents } from "../stores/torrents.svelte";
  import { ui } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";

  let input = $state<HTMLInputElement>();

  export function focusSearch() {
    input?.focus();
    input?.select();
  }
</script>

<header>
  <div class="logo" aria-hidden="true">
    <svg width="20" height="20" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8" fill="none" stroke="var(--accent)" stroke-width="3" /><circle cx="12" cy="12" r="2.6" fill="var(--accent)" /></svg>
  </div>
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
  <button class="btn ghost icon" aria-label="Settings" onclick={() => (ui.settingsOpen = true)}><Icon name="sliders" size={18} /></button>
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
  .logo {
    display: grid;
    place-items: center;
    width: 28px;
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
  .search {
    margin-right: auto;
  }
</style>
