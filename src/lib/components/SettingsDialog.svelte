<script lang="ts">
  import { api, remote, type Theme } from "../api";
  import { auth } from "../stores/auth.svelte";
  import { ui } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";
  import Icon from "./Icon.svelte";
  import NetworkPane from "./NetworkPane.svelte";
  import SearchPane from "./SearchPane.svelte";
  import PluginsPane from "./PluginsPane.svelte";
  import SchedulePane from "./SchedulePane.svelte";

  type Tab = "general" | "speed" | "queue" | "network" | "search" | "plugins";
  const tabs: { key: Tab; label: string }[] = [
    { key: "general", label: "General" },
    { key: "speed", label: "Speed" },
    { key: "queue", label: "Queue and seeding" },
    { key: "network", label: "Network" },
    { key: "search", label: "Search" },
    { key: "plugins", label: "Plugins" },
  ];

  const stored = () => {
    try {
      return (localStorage.getItem("drift.settingsTab") as Tab) || "general";
    } catch {
      return "general" as Tab;
    }
  };
  let tab = $state<Tab>(stored());
  function pick(t: Tab) {
    tab = t;
    try {
      localStorage.setItem("drift.settingsTab", t);
    } catch {
      /* not remembered in private mode */
    }
  }
  function onTabKey(e: KeyboardEvent) {
    const i = tabs.findIndex((t) => t.key === tab);
    const n = e.key === "ArrowRight" ? i + 1 : e.key === "ArrowLeft" ? i - 1 : null;
    if (n === null) return;
    e.preventDefault();
    pick(tabs[(n + tabs.length) % tabs.length].key);
    queueMicrotask(() => document.getElementById(`stab-${tab}`)?.focus());
  }

  const themes: { key: Theme; label: string }[] = [
    { key: "system", label: "System" },
    { key: "light", label: "Light" },
    { key: "dark", label: "Dark" },
  ];

  let version = $state("");
  let checking = $state(false);
  let updateNote = $state("");
  void api.version?.().then((v) => (version = v)).catch(() => {});
  async function checkNow() {
    checking = true;
    updateNote = "";
    try {
      const u = await api.checkUpdate!();
      if (u) ui.update = u;
      updateNote = u ? `Version ${u.version} is available.` : "You're up to date.";
    } catch (e) {
      updateNote = (e as { message?: string }).message ?? "Couldn't check for updates.";
    }
    checking = false;
  }
  let newName = $state("");
  let newPath = $state<string | null>(null);
  let catError = $state("");

  async function changeFolder() {
    const dir = await api.pickFolder(ui.settings?.download_dir ?? null);
    if (dir) await ui.saveSettings({ download_dir: dir });
  }

  async function changeWatch() {
    const dir = await api.pickFolder(ui.settings?.watch_dir ?? ui.settings?.download_dir ?? null);
    if (dir) await ui.saveSettings({ watch_dir: dir });
  }

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
</script>

<Dialog title="Settings" wide onclose={() => (ui.settingsOpen = false)}>
  {#if ui.settings}
    {@const s = ui.settings}
    <div class="tabs" role="tablist" aria-label="Settings sections" tabindex="-1" onkeydown={onTabKey}>
      {#each tabs as t (t.key)}
        <button role="tab" id="stab-{t.key}" aria-selected={tab === t.key} aria-controls="spane" tabindex={tab === t.key ? 0 : -1} class:on={tab === t.key} onclick={() => pick(t.key)}>{t.label}</button>
      {/each}
    </div>

    <div class="pane" id="spane" role="tabpanel" aria-labelledby="stab-{tab}">
      {#if tab === "general"}
        <div class="field">
          <span>Download folder</span>
          <div class="folder">
            <code title={s.download_dir ?? ""}>{s.download_dir ?? "…"}</code>
            <button class="btn" onclick={changeFolder}><Icon name="folder" />Change</button>
          </div>
          <small>New torrents are saved here. Existing torrents keep their folder.</small>
        </div>

        <div class="field">
          <span>Watch folder</span>
          <div class="folder">
            <code title={s.watch_dir ?? ""}>{s.watch_dir ?? "Off"}</code>
            <button class="btn" onclick={changeWatch}><Icon name="folder" />{s.watch_dir ? "Change" : "Choose…"}</button>
            {#if s.watch_dir}<button class="btn" onclick={() => ui.saveSettings({ watch_dir: null })}>Turn off</button>{/if}
          </div>
          <small>A .torrent file dropped in this folder is added automatically, then renamed to <code class="inl">.torrent.added</code> so it isn't added twice.</small>
        </div>

        <div class="field">
          <span>Window and startup</span>
          <label class="check"><input type="checkbox" checked={s.close_to_tray} onchange={(e) => ui.saveSettings({ close_to_tray: e.currentTarget.checked })} /> Keep running in the tray when the window is closed</label>
          <label class="check"><input type="checkbox" checked={s.start_with_windows} onchange={(e) => ui.saveSettings({ start_with_windows: e.currentTarget.checked })} /> Start Drift when Windows starts</label>
          <label class="check sub"><input type="checkbox" checked={s.start_minimized} onchange={(e) => ui.saveSettings({ start_minimized: e.currentTarget.checked })} /> Start minimized to the tray</label>
          <label class="check"><input type="checkbox" checked={s.playlist_on_complete} onchange={(e) => ui.saveSettings({ playlist_on_complete: e.currentTarget.checked })} /> Create a playlist when a multi-file download finishes</label>
          {#if api.checkUpdate}
            <label class="check"><input type="checkbox" checked={s.check_updates} onchange={(e) => ui.saveSettings({ check_updates: e.currentTarget.checked })} /> Check for updates once a day (contacts GitHub)</label>
            <div class="row">
              <button class="btn" disabled={checking} onclick={checkNow}>{checking ? "Checking…" : "Check now"}</button>
              <small role="status">{updateNote || (version ? `Drift ${version}` : "")}</small>
            </div>
          {/if}
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
      {:else if tab === "speed"}
        <div class="field">
          <span>Speed limits</span>
          <label class="check"><input type="checkbox" checked={s.limits_enabled} onchange={(e) => ui.saveSettings({ limits_enabled: e.currentTarget.checked })} /> Limit transfer speed</label>
          <div class="limits">
            <label>Download <input type="number" min="0" step="50" inputmode="numeric" value={s.down_limit_kib || ""} placeholder="No limit" onchange={(e) => ui.saveSettings({ down_limit_kib: whole(e, 4_000_000) })} /> KiB/s</label>
            <label>Upload <input type="number" min="0" step="50" inputmode="numeric" value={s.up_limit_kib || ""} placeholder="No limit" onchange={(e) => ui.saveSettings({ up_limit_kib: whole(e, 4_000_000) })} /> KiB/s</label>
          </div>
          <small>Leave a field empty for no limit in that direction. The switch also lives in the status bar.</small>
        </div>
        {#key ui.settingsRev}<SchedulePane />{/key}
      {:else if tab === "queue"}
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
      {:else if tab === "search"}
        <SearchPane />
      {:else if tab === "plugins"}
        <PluginsPane />
      {:else}
        <NetworkPane />
      {/if}
    </div>
  {/if}

  {#snippet footer()}
    {#if remote}<button class="btn" onclick={() => ((ui.settingsOpen = false), auth.logout())}>Sign out</button>{/if}
    <button class="btn" onclick={() => (ui.settingsOpen = false)}>Done</button>
  {/snippet}
</Dialog>

<style>
  .tabs {
    display: flex;
    border-bottom: 1px solid var(--border);
    margin-bottom: 14px;
  }
  .tabs button {
    height: 32px;
    padding: 0 12px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--text-secondary);
  }
  .tabs button:hover {
    color: var(--text-primary);
  }
  .tabs button.on {
    color: var(--text-primary);
    border-bottom-color: var(--accent);
  }
  .pane {
    display: grid;
    gap: 18px;
    padding-top: 4px;
  }
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
  .bad {
    color: var(--danger-fg);
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
  code.inl {
    display: inline;
    padding: 1px 4px;
    font-size: 11px;
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
  .limits label,
  .inline {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
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
