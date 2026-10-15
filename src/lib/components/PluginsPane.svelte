<script lang="ts">
  import { onMount } from "svelte";
  import { api, type PluginInfo } from "../api";
  import { ui } from "../stores/ui.svelte";

  const s = $derived(ui.settings!);
  let list = $state<PluginInfo[]>([]);
  let loaded = $state(false);

  const load = async () => (list = await api.listPlugins().catch(() => list));
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
  code {
    font-family: var(--font-mono);
    font-size: 11px;
  }
</style>
