<script lang="ts">
  import { api, type Theme } from "../api";
  import { ui } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";
  import Icon from "./Icon.svelte";

  const themes: { key: Theme; label: string }[] = [
    { key: "system", label: "System" },
    { key: "light", label: "Light" },
    { key: "dark", label: "Dark" },
  ];

  async function changeFolder() {
    const dir = await api.pickFolder(ui.settings?.download_dir ?? null);
    if (dir) await ui.saveSettings({ download_dir: dir });
  }
</script>

<Dialog title="Settings" onclose={() => (ui.settingsOpen = false)}>
  <div class="field">
    <span>Download folder</span>
    <div class="folder">
      <code title={ui.settings?.download_dir ?? ""}>{ui.settings?.download_dir ?? "…"}</code>
      <button class="btn" onclick={changeFolder}><Icon name="folder" />Change</button>
    </div>
    <small>New torrents are saved here. Existing torrents keep their folder.</small>
  </div>

  <div class="field" role="radiogroup" aria-label="Theme">
    <span>Theme</span>
    <div class="seg">
      {#each themes as th (th.key)}
        <button role="radio" aria-checked={ui.settings?.theme === th.key} class:on={ui.settings?.theme === th.key} onclick={() => ui.saveSettings({ theme: th.key })}>{th.label}</button>
      {/each}
    </div>
  </div>

  {#snippet footer()}
    <button class="btn" onclick={() => (ui.settingsOpen = false)}>Done</button>
  {/snippet}
</Dialog>

<style>
  .field {
    display: grid;
    gap: 6px;
  }
  .field > span {
    color: var(--text-secondary);
    font-size: 12px;
  }
  small {
    color: var(--text-muted);
  }
  .folder {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  code {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 5px 8px;
    border-radius: var(--radius);
    background: var(--surface-1);
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .seg {
    display: inline-flex;
    width: fit-content;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .seg button {
    height: 28px;
    padding: 0 14px;
    border: 0;
    background: transparent;
  }
  .seg button + button {
    border-left: 1px solid var(--border-strong);
  }
  .seg button.on {
    background: var(--selected);
    color: var(--accent);
    font-weight: 500;
  }
</style>
