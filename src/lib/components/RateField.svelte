<script lang="ts">
  import { untrack } from "svelte";
  import { MAX_KIB } from "../rate";

  /** One speed line: a label and a choice of common speeds, or a custom number. Values are KiB/s; 0 = unlimited. */
  let { label, value, onset, live = false }: { label: string; value: number; onset: (kib: number) => void; live?: boolean } = $props();

  const PRESETS = [0, 128, 256, 512, 1024, 2048, 5120, 10240, 20480, 51200];
  const text = (k: number) => (k === 0 ? "Unlimited" : k >= 1024 && k % 1024 === 0 ? `${k / 1024} MB/s` : `${k} KB/s`);
  const split = (k: number) => (k <= 0 ? { n: "", mb: false } : k % 1024 === 0 ? { n: String(k / 1024), mb: true } : { n: String(k), mb: false });

  // A value that isn't one of the choices (set earlier, or typed) shows as Custom with its number filled in.
  const start = split(untrack(() => value));
  let n = $state(start.n);
  let mb = $state(start.mb);
  let picked = $state(false);
  const custom = $derived(picked || !PRESETS.includes(value));
  const id = $props.id();

  function choose(e: Event & { currentTarget: HTMLSelectElement }) {
    if (e.currentTarget.value === "custom") {
      const s = split(value);
      n = s.n;
      mb = s.mb;
      picked = true;
      return;
    }
    picked = false;
    onset(Number(e.currentTarget.value));
  }

  function commit(text: string = n) {
    const v = Math.floor(Number(text) * (mb ? 1024 : 1));
    onset(Number.isFinite(v) && v > 0 ? Math.min(MAX_KIB, v) : 0);
  }
</script>

<div class="rate">
  <label for="{id}-sel">{label}</label>
  <select id="{id}-sel" value={custom ? "custom" : String(value)} onchange={choose}>
    {#each PRESETS as p (p)}<option value={String(p)}>{text(p)}</option>{/each}
    <option value="custom">Custom…</option>
  </select>
  {#if custom}
    <div class="own">
      <input type="number" min="0" step="any" inputmode="decimal" placeholder="Unlimited" aria-label="{label} custom speed" bind:value={n} onchange={() => commit()} oninput={(e) => live && commit(e.currentTarget.value)} />
      <select aria-label="{label} unit" bind:value={mb} onchange={() => commit()}><option value={false}>KB/s</option><option value={true}>MB/s</option></select>
    </div>
  {/if}
</div>

<style>
  .rate {
    display: grid;
    grid-template-columns: 96px 1fr;
    align-items: center;
    gap: 6px 8px;
  }
  .own {
    grid-column: 2;
    display: grid;
    grid-template-columns: 1fr 76px;
    gap: 6px;
  }
  select,
  input {
    height: 28px;
    min-width: 0;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--surface-1);
  }
</style>
