<script lang="ts">
  import { onMount } from "svelte";
  import { api, type PluginValue } from "../api";
  import { ui } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";

  const target = ui.simTarget!;
  let loading = $state(true);
  let values = $state<Record<string, PluginValue | null>>({});
  let before = $state("");

  // What the plugin starts from: this torrent's own settings if it is simulated, else the plugin's defaults.
  onMount(async () => {
    try {
      values = { ...(await api.simulatorOptions(target.plugin, target.id)) };
    } catch (e) {
      if (ui.simTarget === target) {
        ui.fail(e);
        ui.simTarget = null;
      }
      return;
    }
    before = JSON.stringify(values);
    loading = false;
  });

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
    try {
      await api.setSimulation(target.plugin, target.id, true, chosen());
      ui.toast(target.enabled ? `Saved. ${target.pluginName} restarted this torrent's simulation.` : `${target.pluginName} is simulating this torrent.`);
      ui.simTarget = null;
      void ui.refreshSimulated();
    } catch (e) {
      ui.fail(e);
    }
  }

  async function stop() {
    try {
      await api.setSimulation(target.plugin, target.id, false);
      ui.toast(`${target.pluginName} stopped simulating this torrent.`);
      ui.simTarget = null;
      void ui.refreshSimulated();
    } catch (e) {
      ui.fail(e);
    }
  }
</script>

<Dialog title="Simulation settings" onclose={() => (ui.simTarget = null)}>
  <p class="name" title={target.name}>{target.name}</p>
  {#if loading}
    <p class="note">Asking {target.pluginName}…</p>
  {:else}
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
          {#if f.help}<small>{f.help}</small>{/if}
        </div>
      {/each}
    </div>
    <small>These apply to this torrent only. {target.pluginName}'s own settings (Settings → Plugins) are the defaults.</small>
  {/if}
  {#snippet footer()}
    {#if target.enabled}<button class="btn" disabled={loading} onclick={stop}>Stop simulation</button>{/if}
    <button class="btn" onclick={() => (ui.simTarget = null)}>Cancel</button>
    <button class="btn primary" disabled={loading || (target.enabled && JSON.stringify(values) === before)} onclick={start}>{target.enabled ? "Save" : "Start simulation"}</button>
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
  small {
    color: var(--text-muted);
    line-height: 1.4;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
