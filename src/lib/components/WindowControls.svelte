<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../api";

  const chrome = api.chrome!;
  let maximized = $state(false);
  let maxHover = $state(false);
  let maxBtn = $state<HTMLButtonElement>();

  // Windows needs to know where the maximize button is (physical pixels) to offer snap layouts over it.
  function report() {
    if (!maxBtn) return;
    const r = maxBtn.getBoundingClientRect();
    const k = window.devicePixelRatio || 1;
    void chrome.setMaxButtonRect([Math.round(r.left * k), Math.round(r.top * k), Math.round(r.right * k), Math.round(r.bottom * k)]);
  }

  onMount(() => {
    const stops: (() => void)[] = [];
    const keep = (p: Promise<() => void>) => p.then((s) => stops.push(s)).catch(() => {});
    void chrome.isMaximized().then((m) => (maximized = m));
    keep(chrome.onMaximizedChange((m) => ((maximized = m), report())));
    keep(chrome.onMaxHover((o) => (maxHover = o)));
    report();
    const ro = new ResizeObserver(report);
    ro.observe(maxBtn!);
    window.addEventListener("resize", report);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", report);
      stops.forEach((s) => s());
      void chrome.setMaxButtonRect(null);
    };
  });
</script>

<div class="controls">
  <button aria-label="Minimize" onclick={() => chrome.minimize()}>
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" stroke="currentColor" /></svg>
  </button>
  <button bind:this={maxBtn} class:hover={maxHover} aria-label={maximized ? "Restore" : "Maximize"} onclick={() => chrome.toggleMaximize()}>
    {#if maximized}
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M2.5 2.5v-2h7v7h-2M.5 2.5h7v7h-7z" fill="none" stroke="currentColor" /></svg>
    {:else}
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect x=".5" y=".5" width="9" height="9" fill="none" stroke="currentColor" /></svg>
    {/if}
  </button>
  <button class="close" aria-label="Close" onclick={() => chrome.close()}>
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" /></svg>
  </button>
</div>

<style>
  .controls {
    display: flex;
    align-self: stretch;
    margin-right: -12px; /* flush with the window edge: the bar has 12px side padding */
  }
  button {
    width: 46px;
    border: 0;
    background: transparent;
    color: var(--text-primary);
    display: grid;
    place-items: center;
    cursor: default;
  }
  button:hover,
  button.hover {
    background: var(--surface-1);
  }
  .close:hover {
    background: #c42b1c;
    color: #fff;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
</style>
