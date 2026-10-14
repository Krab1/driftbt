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

  /** Empty or invalid input means "no limit". Whole KiB/s only. */
  function kib(e: Event): number {
    const n = Math.floor(Number((e.currentTarget as HTMLInputElement).value));
    return Number.isFinite(n) && n > 0 ? Math.min(n, 4_000_000) : 0;
  }
</script>

<Dialog title="Settings" onclose={() => (ui.settingsOpen = false)}>
  {#if ui.settings}
    {@const s = ui.settings}
    <div class="field">
      <span>Download folder</span>
      <div class="folder">
        <code title={s.download_dir ?? ""}>{s.download_dir ?? "…"}</code>
        <button class="btn" onclick={changeFolder}><Icon name="folder" />Change</button>
      </div>
      <small>New torrents are saved here. Existing torrents keep their folder.</small>
    </div>

    <div class="field">
      <span>Speed limits</span>
      <label class="check"><input type="checkbox" checked={s.limits_enabled} onchange={(e) => ui.saveSettings({ limits_enabled: e.currentTarget.checked })} /> Limit transfer speed</label>
      <div class="limits">
        <label>Download <input type="number" min="0" step="50" inputmode="numeric" value={s.down_limit_kib || ""} placeholder="No limit" onchange={(e) => ui.saveSettings({ down_limit_kib: kib(e) })} /> KiB/s</label>
        <label>Upload <input type="number" min="0" step="50" inputmode="numeric" value={s.up_limit_kib || ""} placeholder="No limit" onchange={(e) => ui.saveSettings({ up_limit_kib: kib(e) })} /> KiB/s</label>
      </div>
      <small>Leave a field empty for no limit in that direction. The switch also lives in the status bar.</small>
    </div>

    <div class="field">
      <span>Window and startup</span>
      <label class="check"><input type="checkbox" checked={s.close_to_tray} onchange={(e) => ui.saveSettings({ close_to_tray: e.currentTarget.checked })} /> Keep running in the tray when the window is closed</label>
      <label class="check"><input type="checkbox" checked={s.start_with_windows} onchange={(e) => ui.saveSettings({ start_with_windows: e.currentTarget.checked })} /> Start Drift when Windows starts</label>
      <label class="check sub"><input type="checkbox" checked={s.start_minimized} onchange={(e) => ui.saveSettings({ start_minimized: e.currentTarget.checked })} /> Start minimized to the tray</label>
      <label class="check"><input type="checkbox" checked={s.notify_on_complete} onchange={(e) => ui.saveSettings({ notify_on_complete: e.currentTarget.checked })} /> Notify me when a download finishes</label>
    </div>

    <div class="field" role="radiogroup" aria-label="Theme">
      <span>Theme</span>
      <div class="seg">
        {#each themes as th (th.key)}
          <button role="radio" aria-checked={s.theme === th.key} class:on={s.theme === th.key} onclick={() => ui.saveSettings({ theme: th.key })}>{th.label}</button>
        {/each}
      </div>
    </div>
  {/if}

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
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .check.sub {
    padding-left: 24px;
  }
  input[type="checkbox"] {
    accent-color: var(--accent);
    margin: 0;
  }
  .limits {
    display: flex;
    gap: 16px;
    flex-wrap: wrap;
  }
  .limits label {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  input[type="number"] {
    width: 96px;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    text-align: right;
    user-select: text;
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
    color: var(--accent-fg);
    font-weight: 500;
  }
</style>
