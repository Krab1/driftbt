<script lang="ts">
  import { onMount } from "svelte";
  import { api, type PluginField, type PluginInfo, type PluginOffer, type PluginRow, type PluginValue } from "../api";
  import { ui } from "../stores/ui.svelte";

  const s = $derived(ui.settings!);
  let list = $state<PluginInfo[]>([]);
  let loaded = $state(false);

  // What is typed into each plugin's form, until it is saved.
  let drafts = $state<Record<string, Record<string, PluginValue | null>>>({});

  async function load() {
    list = await api.listPlugins().catch(() => list);
    for (const p of list) if (!drafts[p.id]) drafts[p.id] = structuredClone($state.snapshot(p.values));
  }

  /** A typed-in value turned into what the plugin stores for a field of this kind. */
  function clean(kind: string, v: unknown): string | number | boolean | undefined {
    if (kind === "bool") return !!v;
    if (kind === "number") return typeof v === "number" ? v : undefined;
    return String(v ?? "").trim();
  }

  const rowsOf = (p: PluginInfo, f: PluginField) => (drafts[p.id]?.[f.key] as PluginRow[] | undefined) ?? [];

  /** Saves the form. Returns whether it worked. */
  async function save(p: PluginInfo, quiet = false): Promise<boolean> {
    const d = drafts[p.id] ?? {};
    const values: Record<string, PluginValue> = {};
    for (const f of p.fields) {
      if (f.type === "secret") continue;
      if (f.type === "list") {
        values[f.key] = rowsOf(p, f).map((r) => {
          const o: PluginRow = { id: String(r.id) };
          for (const i of f.item ?? []) {
            if (i.type === "secret") continue;
            const v = clean(i.type, r[i.key]);
            if (v !== undefined) o[i.key] = v;
          }
          return o;
        });
        continue;
      }
      const v = clean(f.type, d[f.key]);
      if (v !== undefined) values[f.key] = v;
    }
    let ok = true;
    try {
      await api.setPluginSettings(p.id, values);
      if (!quiet) ui.toast(`${p.name} settings saved.`);
    } catch (e) {
      ui.fail(e);
      ok = false;
    }
    await load();
    return ok;
  }

  function addRow(p: PluginInfo, f: PluginField) {
    const row: PluginRow = { id: "r" + Math.random().toString(36).slice(2, 8) };
    for (const i of f.item ?? []) if (i.type === "bool") row[i.key] = true;
    drafts[p.id][f.key] = [...rowsOf(p, f), row];
  }

  function removeRow(p: PluginInfo, f: PluginField, id: string) {
    drafts[p.id][f.key] = rowsOf(p, f).filter((r) => r.id !== id);
  }

  /** A row's secret needs the row to exist, so the form is saved first. */
  async function rowSecret(p: PluginInfo, f: PluginField, row: PluginRow, item: PluginField, clear = false) {
    if (!(await save(p, true))) return;
    await secret(p, `${f.key}.${row.id}.${item.key}`, item.label, clear);
  }

  let tests = $state<Record<string, { busy: boolean; ok?: boolean; text?: string }>>({});

  async function test(id: string) {
    tests[id] = { busy: true };
    try {
      const r = await api.testSearchSource(id);
      tests[id] = r.outcome === "ok" ? { busy: false, ok: true, text: r.count > 0 ? "Working. It answered the sample search." : "Working, though the sample search found nothing." } : { busy: false, ok: false, text: r.message ?? "It didn't answer." };
    } catch (e) {
      tests[id] = { busy: false, ok: false, text: e instanceof Error ? e.message : "The test couldn't run." };
    }
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

  let offers = $state<PluginOffer[] | null>(null);
  let looking = $state(false);
  let busy = $state("");

  async function lookup() {
    looking = true;
    try {
      offers = await api.pluginOffers();
    } catch (e) {
      offers = null;
      ui.fail(e);
    }
    looking = false;
  }

  async function setOnline(on: boolean) {
    await ui.saveSettings({ plugin_registry: on });
    if (!on) offers = null;
  }

  async function install(o: PluginOffer) {
    if (!confirm(`Download ${o.name} ${o.version}?\n\nDrift checks it against the signed plugin list before keeping it. It starts working after you restart Drift, and a new or updated plugin that asks for more access stays off until you turn it on again.`)) return;
    busy = o.id;
    try {
      await api.installPlugin(o.id, o.version);
      ui.toast(`${o.name} ${o.version} is ready. Restart Drift to start using it.`);
    } catch (e) {
      ui.fail(e);
    }
    busy = "";
    await load();
  }

  async function rollback(p: PluginInfo) {
    if (!confirm(`Go back to ${p.name} ${p.previous}? It takes effect when you restart Drift.`)) return;
    try {
      await api.rollbackPlugin(p.id);
      ui.toast(`${p.name} ${p.previous} comes back after you restart Drift.`);
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
          {#if p.kinds.includes("search") && p.enabled && !p.problem}
            <span class="test">
              <button class="btn ghost" type="button" disabled={tests[p.id]?.busy} title="Send one sample search to this source" onclick={() => test(p.id)}>{tests[p.id]?.busy ? "Testing…" : "Test"}</button>
              {#if tests[p.id]?.text}<small class:bad={!tests[p.id].ok} class:good={tests[p.id].ok} role="status">{tests[p.id].text}</small>{/if}
            </span>
          {/if}
          {#if p.pending}<small class="note">Version {p.pending} is ready and takes over when you restart Drift.</small>{/if}
          {#if p.previous && !p.pending}<small>Earlier version {p.previous} is kept. <button class="link" onclick={() => rollback(p)}>Go back to it</button></small>{/if}
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
                      {:else if f.type === "list"}
                        <span class="lbl">{f.label}</span>
                        {#each rowsOf(p, f) as row (row.id)}
                          <div class="rowbox">
                            {#each f.item ?? [] as i (i.key)}
                              {#if i.type === "bool"}
                                <label class="check"><input type="checkbox" checked={!!row[i.key]} onchange={(e) => (row[i.key] = e.currentTarget.checked)} /> {i.label}</label>
                              {:else if i.type === "secret"}
                                {@const saved = p.secrets_set.includes(`${f.key}.${row.id}.${i.key}`)}
                                <span class="sec">
                                  <small>{i.label}: {saved ? "saved" : "not set"}</small>
                                  <button type="button" class="link" onclick={() => rowSecret(p, f, row, i)}>{saved ? "Change" : "Set"}</button>
                                  {#if saved}<button type="button" class="link" onclick={() => rowSecret(p, f, row, i, true)}>Remove</button>{/if}
                                </span>
                              {:else}
                                <input class="txt" type={i.type === "number" ? "number" : i.type === "url" ? "url" : "text"} autocomplete="off" placeholder={i.label} aria-label={i.label} value={row[i.key] ?? ""} oninput={(e) => (row[i.key] = i.type === "number" ? e.currentTarget.valueAsNumber : e.currentTarget.value)} />
                              {/if}
                            {/each}
                            <button type="button" class="btn ghost icon" aria-label="Remove this row" onclick={() => removeRow(p, f, String(row.id))}>✕</button>
                          </div>
                        {/each}
                        <div><button type="button" class="btn" onclick={() => addRow(p, f)}>Add</button></div>
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
    <div class="online">
      <label class="check"><input type="checkbox" checked={s.plugin_registry} onchange={(e) => setOnline(e.currentTarget.checked)} /> Look for plugins and updates online <small>contacts github.com when you ask</small></label>
      {#if s.plugin_registry}<button class="btn" disabled={looking} onclick={lookup}>{looking ? "Looking…" : offers ? "Check again" : "Check now"}</button>{/if}
    </div>
    {#if offers}
      {#each offers as o (o.id)}
        <div class="item">
          <div class="meta">
            <div class="nm">{o.name} <small>{o.version}{#if o.installed}, you have {o.installed}{/if}</small></div>
            {#if o.summary}<small>{o.summary}</small>{/if}
            {#if o.changelog}<small>What's new: {o.changelog}</small>{/if}
          </div>
          {#if o.installed === o.version}
            <small>Up to date</small>
          {:else}
            <button class="btn" disabled={busy === o.id} onclick={() => install(o)}>{busy === o.id ? "Downloading…" : o.installed ? "Update" : "Install"}</button>
          {/if}
        </div>
      {:else}
        <small>The registry has no plugins for this version of Drift yet.</small>
      {/each}
    {/if}
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
  .online {
    display: flex;
    gap: 12px;
    align-items: center;
    flex-wrap: wrap;
  }
  .note {
    color: var(--accent-fg);
  }
  .good {
    color: var(--success);
  }
  .test {
    display: flex;
    gap: 8px;
    align-items: center;
    min-height: 24px;
  }
  .test .btn {
    height: 24px;
    padding: 0 8px;
  }
  .rowbox {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .rowbox .txt {
    flex: 1;
    min-width: 140px;
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
