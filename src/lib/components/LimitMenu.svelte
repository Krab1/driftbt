<script lang="ts">
  import { formatBytes } from "../../format";
  import { ui } from "../stores/ui.svelte";

  let { dir, anchor, onset, oncustom, onclose }: { dir: "down" | "up"; anchor: HTMLElement; onset: (dir: "down" | "up", kib: number) => Promise<boolean>; oncustom: () => void; onclose: () => void } = $props();

  // What the list shows is what is in force: with the limit switched off, Unlimited is ticked.
  const s = $derived(ui.settings);
  const now = $derived(s?.limits_enabled ? (dir === "down" ? s.down_limit_kib : s.up_limit_kib) : 0);
  const CHOICES = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
  const isPreset = $derived(now === 0 || (now % 1024 === 0 && now / 1024 >= 1 && now / 1024 <= 10));
  const word = (k: number) => (k === 0 ? "Unlimited" : `${k} MB/s`);

  async function pick(mb: number) {
    // The save starts before the menu goes away, so nothing it needs is torn down under it.
    const saved = onset(dir, mb * 1024);
    close(true);
    await saved;
  }
  function schedule() {
    onclose();
    ui.openSettings("speed");
  }

  let el = $state<HTMLDivElement>();
  let pos = $state({ left: 0, bottom: 0 });
  $effect(() => {
    if (!el) return;
    const a = anchor.getBoundingClientRect();
    pos = { left: Math.max(8, Math.min(a.left, innerWidth - el.offsetWidth - 8)), bottom: innerHeight - a.top + 6 };
    el.querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
  });

  function close(refocus = false) {
    onclose();
    if (refocus) anchor.focus();
  }
  function onwindowclick(e: MouseEvent) {
    const t = e.target as Node;
    if (!el?.contains(t) && !anchor.contains(t)) close();
  }
  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close(true);
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const items = [...(el?.querySelectorAll<HTMLElement>('[role="menuitemradio"]') ?? [])];
      const i = items.indexOf(document.activeElement as HTMLElement);
      items[(i + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
    }
  }
</script>

<svelte:window onclick={onwindowclick} onresize={() => close()} {onkeydown} />

<div class="pop" bind:this={el} style="left: {pos.left}px; bottom: {pos.bottom}px">
  {#if ui.limits.scheduled}
    <p class="note">
      A schedule is limiting speed right now{ui.limits.down_bps ? ` (↓ ${formatBytes(ui.limits.down_bps)}/s)` : ""}. What you pick here applies outside its time slots.
      <button class="link" onclick={schedule}>Edit the schedule</button>
    </p>
  {/if}
  <p class="head">{dir === "down" ? "↓ Download limit" : "↑ Upload limit"}</p>
  <div class="items" role="menu" aria-label={dir === "down" ? "Download limit" : "Upload limit"}>
  {#each CHOICES as k (k)}
    <button role="menuitemradio" aria-checked={now === k * 1024} class:on={now === k * 1024} onclick={() => pick(k)}><span class="tick">{now === k * 1024 ? "✓" : ""}</span>{word(k)}</button>
  {/each}
  <button role="menuitemradio" aria-checked={!isPreset} class:on={!isPreset} onclick={() => { oncustom(); close(); }}>
    <span class="tick">{isPreset ? "" : "✓"}</span>Custom…{#if !isPreset}<span class="cur">{formatBytes(now * 1024)}/s</span>{/if}
  </button>
  </div>
  <small>For all torrents together. To limit one torrent, right-click it and choose Speed limit.</small>
</div>

<style>
  .pop {
    position: fixed;
    z-index: 60;
    width: 220px;
    display: grid;
    gap: 1px;
    padding: 6px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    background: var(--surface-2);
    box-shadow: var(--shadow-menu);
    font-size: 13px;
    color: var(--text-primary);
  }
  .items {
    display: grid;
    gap: 1px;
  }
  .head {
    margin: 0;
    padding: 4px 8px;
    color: var(--text-muted);
    font-size: 11px;
  }
  [role="menuitemradio"] {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: inherit;
    font-size: inherit;
    text-align: left;
  }
  [role="menuitemradio"]:hover,
  [role="menuitemradio"]:focus-visible {
    background: var(--hover);
  }
  [role="menuitemradio"].on {
    color: var(--accent-fg);
  }
  .tick {
    width: 12px;
    flex: none;
  }
  .cur {
    margin-left: auto;
    color: var(--text-muted);
  }
  small,
  .note {
    margin: 0;
    padding: 6px 8px 2px;
    color: var(--text-muted);
    font-size: 11px;
    line-height: 1.4;
  }
  .note {
    margin-bottom: 4px;
    padding: 6px 8px;
    border-radius: var(--radius);
    background: var(--surface-1);
  }
  .link {
    display: inline;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-fg);
    text-decoration: underline;
    font-size: inherit;
  }
</style>
