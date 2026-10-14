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

  let newName = $state("");
  let newPath = $state<string | null>(null);
  let catError = $state("");

  async function pickCategoryFolder() {
    newPath = (await api.pickFolder(newPath ?? ui.settings?.download_dir ?? null)) ?? newPath;
  }

  async function addCategory() {
    const name = newName.trim();
    const s = ui.settings;
    if (!s) return;
    if (!name) return void (catError = "Enter a category name.");
    if (s.categories.some((c) => c.name.toLowerCase() === name.toLowerCase())) return void (catError = "That category already exists.");
    catError = "";
    await ui.saveSettings({ categories: [...s.categories, { name, save_path: newPath }] });
    newName = "";
    newPath = null;
  }

  /** Empty or invalid input means 0 / unset. */
  function whole(e: Event, max = 1_000_000): number {
    const n = Math.floor(Number((e.currentTarget as HTMLInputElement).value));
    return Number.isFinite(n) && n > 0 ? Math.min(n, max) : 0;
  }

  function ratio(e: Event): number | null {
    const n = Number((e.currentTarget as HTMLInputElement).value);
    return Number.isFinite(n) && n > 0 ? Math.round(n * 100) / 100 : null;
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
      <span>Download queue</span>
      <label class="inline">Download at most <input type="number" min="0" step="1" inputmode="numeric" value={s.max_active_downloads || ""} placeholder="All" onchange={(e) => ui.saveSettings({ max_active_downloads: whole(e, 100) })} /> torrents at a time</label>
      <small>Others wait in the queue and start when a slot frees up. Leave empty to download everything at once.</small>
    </div>

    <div class="field">
      <span>Seeding goals</span>
      <div class="limits">
        <label>Stop at ratio <input type="number" min="0" step="0.1" inputmode="decimal" value={s.default_goal.ratio ?? ""} placeholder="Never" onchange={(e) => ui.saveSettings({ default_goal: { ...s.default_goal, ratio: ratio(e) } })} /></label>
        <label>or after <input type="number" min="0" step="10" inputmode="numeric" value={s.default_goal.minutes ?? ""} placeholder="Never" onchange={(e) => ui.saveSettings({ default_goal: { ...s.default_goal, minutes: whole(e) || null } })} /> minutes</label>
      </div>
      <label class="inline">Always seed for at least <input type="number" min="0" step="10" inputmode="numeric" value={s.min_seed_minutes || ""} placeholder="0" onchange={(e) => ui.saveSettings({ min_seed_minutes: whole(e) })} /> minutes</label>
      <small>Applies to every torrent without its own goal (right-click a torrent to set one). Nothing stops before the minimum, whatever the ratio.</small>
    </div>

    <div class="field">
      <span>Categories</span>
      {#each s.categories as c (c.name)}
        <div class="cat"><Icon name="folder" size={14} /><span class="cn">{c.name}</span><code title={c.save_path ?? "Default download folder"}>{c.save_path ?? "Default folder"}</code>
          <button class="btn ghost icon" aria-label="Remove category {c.name}" onclick={() => ui.saveSettings({ categories: s.categories.filter((x) => x.name !== c.name) })}><Icon name="x" size={14} /></button>
        </div>
      {/each}
      <div class="cat add">
        <input type="text" class="txt" placeholder="New category" aria-label="New category name" bind:value={newName} onkeydown={(e) => e.key === "Enter" && addCategory()} />
        <button class="btn" onclick={pickCategoryFolder} title={newPath ?? "Use the default download folder"}><Icon name="folder" />{newPath ? "Folder set" : "Folder…"}</button>
        <button class="btn" onclick={addCategory}><Icon name="plus" />Add</button>
      </div>
      {#if catError}<small class="bad" role="alert">{catError}</small>{/if}
      <small>A category can send its torrents to their own folder. Assign one in the Add dialog or from a torrent's right-click menu.</small>
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
  .inline {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .cat {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .cn {
    min-width: 90px;
  }
  .cat code {
    flex: 1;
  }
  .txt {
    flex: 1;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    user-select: text;
  }
  .bad {
    color: var(--danger-fg);
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
