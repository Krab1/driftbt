<script lang="ts">
  import { onMount } from "svelte";
  import { api, type SimStatus } from "../api";
  import { ui, errorText } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";

  const target = ui.simStatsTarget!;
  let status = $state<SimStatus | null>(null);
  let problem = $state<string | null>(null);

  // Asked of the plugin only while this window is open; nothing is kept by Drift.
  async function load() {
    try {
      status = await api.simulatorStatus(target.plugin, target.id);
      problem = null;
    } catch (e) {
      problem = errorText(e);
    }
  }

  // One question at a time: the next is asked two seconds after the last answer, so a slow plugin is never piled up on.
  onMount(() => {
    let closed = false;
    let timer: ReturnType<typeof setTimeout>;
    const tick = async () => {
      await load();
      if (!closed) timer = setTimeout(tick, 2000);
    };
    void tick();
    return () => {
      closed = true;
      clearTimeout(timer);
    };
  });
</script>

<Dialog title="Simulation statistics" wide onclose={() => (ui.simStatsTarget = null)}>
  <p class="name" title={target.name}>{target.name}</p>
  {#if problem}<p class="bad">{problem}</p>{/if}
  {#if !status && !problem}<p class="note">Asking {target.pluginName}…</p>{/if}
  {#if status}
    {#each status.tables as t, ti (ti)}
      <section>
        {#if t.title}<h3>{t.title}</h3>{/if}
        <div class="scroll">
          <table>
            <thead><tr>{#each t.columns as c, ci (ci)}<th>{c}</th>{/each}</tr></thead>
            <tbody>
              {#each t.rows as r, i (i)}<tr>{#each r as cell, j (j)}<td>{cell}</td>{/each}</tr>{/each}
            </tbody>
          </table>
        </div>
      </section>
    {/each}
    {#if status.log.length}
      <section>
        <h3>Recent</h3>
        <pre class="log">{status.log.join("\n")}</pre>
      </section>
    {/if}
  {/if}
  {#snippet footer()}
    <small>Kept by {target.pluginName} in memory only; gone when Drift closes.</small>
    <button class="btn" onclick={() => (ui.simStatsTarget = null)}>Close</button>
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
  .bad {
    margin: 0;
    color: var(--danger-fg);
  }
  h3 {
    margin: 8px 0 4px;
    font-size: 12px;
    color: var(--text-secondary);
  }
  .scroll {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  th,
  td {
    padding: 3px 8px 3px 0;
    text-align: left;
    white-space: nowrap;
  }
  th {
    color: var(--text-muted);
    font-weight: 500;
  }
  .log {
    margin: 0;
    max-height: 180px;
    overflow: auto;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg);
    font-size: 11px;
    line-height: 1.5;
    user-select: text;
  }
  small {
    margin-right: auto;
    color: var(--text-muted);
  }
</style>
