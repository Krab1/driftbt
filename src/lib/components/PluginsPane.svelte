<script lang="ts">
  import { onMount } from "svelte";
  import { api, type PluginInfo } from "../api";
  import { ui } from "../stores/ui.svelte";

  const s = $derived(ui.settings!);
  let list = $state<PluginInfo[]>([]);
  let loaded = $state(false);

  // What is typed into each plugin's form, until it is saved.
  let drafts = $state<Record<string, Record<string, string | number | boolean | null>>>({});

  async function load() {
    list = await api.listPlugins().catch(() => list);
    for (const p of list) if (!drafts[p.id]) drafts[p.id] = { ...p.values };
  }

  async function save(p: PluginInfo) {
    const d = drafts[p.id] ?? {};
    const values: Record<string, string | number | boolean> = {};
    for (const f of p.fields) {
      if (f.type === "secret") continue;
      const v = d[f.key];
      if (f.type === "bool") values[f.key] = !!v;
      else if (f.type === "number") {
        if (typeof v === "number") values[f.key] = v;
      } else values[f.key] = String(v ?? "").trim();
    }
    try {
      await api.setPluginSettings(p.id, values);
      ui.toast(`${p.name} settings saved.`);
    } catch (e) {
      ui.fail(e);
    }
    await load();
  }

  async function secret(p: PluginInfo, key: string, label: string, clear = false) {
    const v = clear ? "" : prompt(`${label} (leave empty to remove it)`);
    if (v === null) return;
    try {
      await api.setPluginSecret(p.id, key, v.trim());
    } catch (e) {
      ui.fail(e);
    }
    await load();
  }

  async function showLog(p: PluginInfo) {
    try {
      const text = await api.pluginLog(p.id);
      ui.detailView = { title: `${p.name} log`, text: text || "Nothing yet. The plugin hasn't written anything." };
    } catch (e) {
      ui.fail(e);
    }
  }
  onMount(() => void load().then(() => (loaded = true)));

  const net = (p: PluginInfo) => (p.permissions.net.includes("*") ? "any website" : p.permissions.net.join(", "));

  async function toggle(p: PluginInfo, on: boolean) {
    // Permissions are shown before the first enable, not after.
    if (on) {
      const parts = [p.permissions.net.length ? `connect to ${net(p)}` : "", p.permissions.fs.length ? `read or write ${p.permissions.fs.join(", ")}` : ""].filter(Boolean);
      const what = parts.length ? parts.join(" and ") : "run on this computer";
      const ok = confirm(`${p.name} ${p.version} says it will ${what}.\n\nPlugins run as their own programs and Drift can't restrict them. Only turn on plugins you trust.\n\nTurn it on?`);
      if (!ok) return void (await load());
    }
    const rest = s.plugins.filter((x) => x !== p.id);
    await ui.saveSettings({ plugins: on ? [...rest, p.id] : rest });
    await load();
  }
</script>

{#key ui.settingsRev}
  <div class="field">
    <span>Plugins</span>
    {#each list as p (p.id)}
      <div class="item">
        <input type="checkbox" aria-label="Use {p.name}" checked={p.enabled} disabled={!!p.problem && !p.enabled} onchange={(e) => toggle(p, e.currentTarget.checked)} />
        <div class="meta">
          <div class="nm">{p.name} <small>{p.version} · {p.kinds.join(", ")}</small></div>
          {#if p.problem}
            <small class="bad">{p.problem}</small>
          {:else}
            <small>Talks to: {p.permissions.net.length ? net(p) : "nothing"}</small>
          {/if}
          {#if p.enabled || !p.problem}
            <details class="more">
              <summary>{p.fields.length ? "Settings and log" : "Log"}</summary>
              {#if p.fields.length}
                <form class="form" onsubmit={(e) => (e.preventDefault(), save(p))}>
                  {#each p.fields as f (f.key)}
                    <div class="fld">
                      {#if f.type === "bool"}
                        <label class="check"><input type="checkbox" checked={!!drafts[p.id]?.[f.key]} onchange={(e) => (drafts[p.id][f.key] = e.currentTarget.checked)} /> {f.label}</label>
                      {:else if f.type === "secret"}
                        <span class="lbl">{f.label}</span>
                        <span class="sec">
                          <small>{p.secrets_set.includes(f.key) ? "Saved" : "Not set"}</small>
                          <button type="button" class="link" onclick={() => secret(p, f.key, f.label)}>{p.secrets_set.includes(f.key) ? "Change" : "Set"}</button>
                          {#if p.secrets_set.includes(f.key)}<button type="button" class="link" onclick={() => secret(p, f.key, f.label, true)}>Remove</button>{/if}
                        </span>
                      {:else}
                        <label class="lbl" for="pf-{p.id}-{f.key}">{f.label}</label>
                        <input id="pf-{p.id}-{f.key}" class="txt" type={f.type === "number" ? "number" : f.type === "url" ? "url" : "text"} autocomplete="off" value={drafts[p.id]?.[f.key] ?? ""} oninput={(e) => (drafts[p.id][f.key] = f.type === "number" ? (e.currentTarget.value === "" ? null : e.currentTarget.valueAsNumber) : e.currentTarget.value)} />
                      {/if}
                      {#if f.help}<small>{f.help}</small>{/if}
                    </div>
                  {/each}
                  <div><button class="btn primary" type="submit">Save</button></div>
                </form>
              {/if}
              <div><button class="btn ghost" type="button" onclick={() => showLog(p)}>View log</button></div>
            </details>
          {/if}
        </div>
      </div>
    {:else}
      {#if loaded}<small>No plugins installed.</small>{/if}
    {/each}
    <small>
      A plugin is a separate program Drift runs for you, for example another search source. Put its folder in <code>plugins</code> inside Drift's data folder and restart Drift. Plugins can only
      answer questions; they can't add or control torrents.
    </small>
  </div>
{/key}

<style>
  .field {
    display: grid;
    gap: 8px;
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
  input[type="checkbox"] {
    accent-color: var(--accent);
    margin: 0;
    margin-top: 4px;
  }
  .item {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .meta {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .more {
    margin-top: 4px;
  }
  .more summary {
    cursor: pointer;
    color: var(--accent-fg);
  }
  .more[open] {
    display: grid;
    gap: 8px;
  }
  .form {
    display: grid;
    gap: 8px;
    margin-top: 6px;
  }
  .fld {
    display: grid;
    gap: 2px;
  }
  .lbl {
    color: var(--text-secondary);
    font-size: 12px;
  }
  .sec {
    display: flex;
    gap: 10px;
    align-items: center;
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .check input {
    margin: 0;
  }
  .txt {
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    user-select: text;
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-fg);
    cursor: pointer;
    text-decoration: underline;
  }
  code {
    font-family: var(--font-mono);
    font-size: 11px;
  }
</style>
