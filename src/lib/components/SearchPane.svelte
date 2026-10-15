<script lang="ts">
  import { onMount } from "svelte";
  import { api, type TorznabConfig } from "../api";
  import { ui } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";

  const s = $derived(ui.settings!);
  let keys = $state<string[]>([]);
  let name = $state("");
  let url = $state("");
  let key = $state("");
  let error = $state("");

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

  onMount(async () => (keys = await api.searchKeysSet().catch(() => [])));

  const slug = (n: string) => n.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 40) || "indexer";
  const setList = (torznab: TorznabConfig[]) => ui.saveSettings({ search: { ...s.search, torznab } });
  const refreshKeys = async () => (keys = await api.searchKeysSet().catch(() => keys));

  async function add() {
    const n = name.trim();
    const u = url.trim();
    if (!/^https?:\/\/\S+$/i.test(u)) return void (error = "Enter the indexer's address, starting with http:// or https://.");
    error = "";
    let host = "indexer";
    try {
      host = new URL(u).hostname;
    } catch {
      /* the check above already passed; keep the default name */
    }
    let id = slug(n || host);
    for (let i = 2; s.search.torznab.some((t) => t.id === id); i++) id = `${slug(n || host)}-${i}`;
    if (!(await setList([...s.search.torznab, { id, name: n || id, url: u, enabled: true }]))) return;
    if (key.trim()) {
      await api.setSearchKey(id, key.trim()).catch((e) => (error = e.message));
      await refreshKeys();
    }
    name = url = key = "";
  }

  async function replaceKey(id: string) {
    const k = prompt("New API key (leave empty to remove it)");
    if (k === null) return;
    await api.setSearchKey(id, k.trim()).catch((e) => ui.toast(e.message, "error"));
    await refreshKeys();
  }
</script>

{#snippet result(id: string, on: boolean)}
  <div class="test">
    <button class="btn ghost" disabled={tests[id]?.busy || !on} title={on ? "Send one sample search to this source" : "Turn this source on to test it"} onclick={() => test(id)}>{tests[id]?.busy ? "Testing…" : "Test"}</button>
    {#if tests[id]?.text}<small class:bad={!tests[id].ok} class:good={tests[id].ok} role="status">{tests[id].text}</small>{/if}
  </div>
{/snippet}

{#key ui.settingsRev}
  <div class="field">
    <span>Sources</span>
    <label class="check"><input type="checkbox" checked={s.search.archive_org} onchange={(e) => ui.saveSettings({ search: { ...s.search, archive_org: e.currentTarget.checked } })} /> Internet Archive <small>public-domain and openly licensed films, music, books and software</small></label>
    {@render result("archive_org", s.search.archive_org)}

    {#each s.search.torznab as t (t.id)}
      <div class="list">
        <input type="checkbox" aria-label="Use {t.name}" checked={t.enabled} onchange={(e) => setList(s.search.torznab.map((x) => (x.id === t.id ? { ...x, enabled: e.currentTarget.checked } : x)))} />
        <div class="meta">
          <div class="nm"><Icon name="search" size={14} />{t.name}</div>
          <code title={t.url}>{t.url}</code>
          <small>{keys.includes(t.id) ? "API key saved" : "No API key"} · <button class="link" onclick={() => replaceKey(t.id)}>{keys.includes(t.id) ? "Change key" : "Add key"}</button></small>
          {@render result(t.id, t.enabled)}
        </div>
        <button class="btn ghost icon" aria-label="Remove {t.name}" onclick={() => setList(s.search.torznab.filter((x) => x.id !== t.id))}><Icon name="x" size={14} /></button>
      </div>
    {/each}

    {#if !s.search.archive_org && !s.search.torznab.some((t) => t.enabled)}
      <small class="bad" role="status">No source is turned on, so searching will have nothing to ask.</small>
    {/if}
  </div>

  <div class="field">
    <span>Add a Torznab indexer</span>
    <div class="row">
      <input class="txt" type="text" placeholder="Name (optional)" aria-label="Indexer name" bind:value={name} />
      <input class="txt wide" type="text" placeholder="http://localhost:9117/api/v2.0/indexers/all/results/torznab/" aria-label="Indexer address" bind:value={url} />
    </div>
    <div class="row">
      <input class="txt" type="password" autocomplete="off" placeholder="API key" aria-label="API key" bind:value={key} onkeydown={(e) => e.key === "Enter" && add()} />
      <button class="btn" onclick={add}><Icon name="plus" />Add</button>
    </div>
    {#if error}<small class="bad" role="alert">{error}</small>{/if}
    <small>
      Torznab is the search API that Jackett and Prowlarr offer. Run one of them yourself and point Drift at it, so what gets searched is your choice. The key is stored only on this
      computer and is never shown again or sent anywhere but that address.
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
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  input[type="checkbox"] {
    accent-color: var(--accent);
    margin: 0;
  }
  .txt {
    flex: 1;
    min-width: 120px;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    user-select: text;
  }
  .txt.wide {
    flex: 2;
    min-width: 200px;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .list {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .list > input {
    margin-top: 4px;
  }
  .meta {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .nm {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  code {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-fg);
    cursor: pointer;
    text-decoration: underline;
  }
</style>
