<script lang="ts">
  import { onMount } from "svelte";
  import { api, type PluginValue } from "../api";
  import { ui } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";

  const target = ui.simTarget!;
  let loading = $state(true);
  let values = $state<Record<string, PluginValue | null>>({});
  let before = $state("");
  let busy = $state(false);
  let problem = $state<string | null>(null);

  // What the plugin starts from: this torrent's own settings if it is simulated, else the plugin's defaults.
  onMount(async () => {
    try {
      values = { ...(await api.simulatorOptions(target.plugin, target.id)) };
    } catch {
      // The form still opens, so a plugin that can't answer can be started with its own defaults.
      problem = `${target.pluginName} didn't say what it would use, so empty boxes mean its own defaults.`;
    }
    for (const f of target.fields) if (values[f.key] === undefined && f.default !== undefined) values[f.key] = f.default;
    // A torrent not simulated yet starts from what was used last time; one already simulated keeps its own.
    if (!target.enabled) {
      const last = lastUsed();
      for (const f of target.fields) {
        const v = last[f.key];
        if (v !== undefined && (!f.options?.length || f.options.includes(String(v)))) values[f.key] = v;
      }
    }
    before = JSON.stringify(values);
    loading = false;
  });

  const MEMORY = `drift.sim.${target.plugin}`;
  function lastUsed(): Record<string, PluginValue> {
    try {
      return JSON.parse(localStorage.getItem(MEMORY) ?? "{}") ?? {};
    } catch {
      return {};
    }
  }

  /** Empty number boxes are left out, so the plugin keeps its own default for them. */
  function chosen(): Record<string, PluginValue> {
    const out: Record<string, PluginValue> = {};
    for (const f of target.fields) {
      const v = values[f.key];
      if (v === null || v === undefined || v === "" || (typeof v === "number" && !Number.isFinite(v))) continue;
      out[f.key] = v;
    }
    return out;
  }

  async function start() {
    if (busy) return;
    busy = true;
    try {
      const picked = chosen();
      await api.setSimulation(target.plugin, target.id, true, picked);
      try {
        localStorage.setItem(MEMORY, JSON.stringify(picked));
      } catch {
        // Not remembering is fine.
      }
      ui.toast(target.enabled ? `Saved. ${target.pluginName} restarted this torrent's simulation.` : `${target.pluginName} is simulating this torrent.`);
      ui.simTarget = null;
      void ui.refreshSimulated();
    } catch (e) {
      ui.fail(e);
    }
    busy = false;
  }

  async function stop() {
    if (busy) return;
    busy = true;
    try {
      await api.setSimulation(target.plugin, target.id, false);
      ui.toast(`${target.pluginName} stopped simulating this torrent.`);
      ui.simTarget = null;
      void ui.refreshSimulated();
    } catch (e) {
      ui.fail(e);
    }
    busy = false;
  }
</script>

<Dialog title="Simulation settings" keep onclose={() => (ui.simTarget = null)}>
  <p class="name" title={target.name}>{target.name}</p>
  {#if loading}
    <p class="note">Asking {target.pluginName}…</p>
  {:else}
    {#if problem}<p class="note">{problem}</p>{/if}
    <div class="form">
      {#each target.fields as f (f.key)}
        <div class="fld">
          {#if f.type === "bool"}
            <label class="check"><input type="checkbox" checked={!!values[f.key]} onchange={(e) => (values[f.key] = e.currentTarget.checked)} /> {f.label}{#if values[f.key] === undefined}<small> (the plugin's own default)</small>{/if}</label>
          {:else}
            <label for="sim-{f.key}">{f.label}{#if values[f.key] === undefined}<small> (the plugin's own default)</small>{/if}</label>
            {#if f.options?.length}
              <select id="sim-{f.key}" class="txt" value={values[f.key] ?? ""} onchange={(e) => (values[f.key] = e.currentTarget.value)}>
                {#if values[f.key] === undefined}<option value="" selected>(the plugin's own default)</option>{/if}
                {#each f.options as o (o)}<option value={o}>{o}</option>{/each}
              </select>
            {:else}
              <input id="sim-{f.key}" class="txt" type={f.type === "number" ? "number" : "text"} autocomplete="off" value={values[f.key] ?? ""} oninput={(e) => (values[f.key] = f.type === "number" ? (e.currentTarget.value === "" ? null : e.currentTarget.valueAsNumber) : e.currentTarget.value)} />
            {/if}
          {/if}
          {#if f.option_help?.[String(values[f.key] ?? f.default ?? "")]}<small class="tip">{f.option_help[String(values[f.key] ?? f.default ?? "")]}</small>{/if}
          {#if f.help}<small>{f.help}</small>{/if}
        </div>
      {/each}
    </div>
    <small>These apply to this torrent only. {target.pluginName}'s own settings (Settings → Plugins) cover the rest, such as proxies and the custom client.</small>
  {/if}
  {#snippet footer()}
    {#if target.enabled}<button class="btn" disabled={loading || busy} onclick={stop}>Stop simulation</button>{/if}
    <button class="btn" onclick={() => (ui.simTarget = null)}>Cancel</button>
    <button class="btn primary" disabled={loading || busy || (target.enabled && JSON.stringify(values) === before)} onclick={start}>{target.enabled ? "Save" : "Start simulation"}</button>
  {/snippet}
</Dialog>

<style>
  .name {
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-secondary);
  }
  .note {
    margin: 0;
    color: var(--text-muted);
  }
  .form {
    display: grid;
    gap: 10px;
    max-height: min(60vh, 440px);
    overflow: auto;
    padding-right: 4px;
  }
  .fld {
    display: grid;
    gap: 3px;
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .txt {
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text-primary);
    user-select: text;
  }
  .tip {
    color: var(--text-secondary);
  }
  small {
    color: var(--text-muted);
    line-height: 1.4;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
