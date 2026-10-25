<script lang="ts">
  import { formatBytes } from "../../format";
  import { ui } from "../stores/ui.svelte";
  import RateField from "./RateField.svelte";

  let { dir, anchor, onclose }: { dir: "down" | "up"; anchor: HTMLElement; onclose: () => void } = $props();

  // What the two lines show is what is in force: with the limit switched off, both read Unlimited.
  const s = $derived(ui.settings);
  const down = $derived(s?.limits_enabled ? s.down_limit_kib : 0);
  const up = $derived(s?.limits_enabled ? s.up_limit_kib : 0);

  async function set(dir: "down" | "up", kib: number) {
    const d = dir === "down" ? kib : down;
    const u = dir === "up" ? kib : up;
    await ui.saveSettings({ limits_enabled: d > 0 || u > 0, down_limit_kib: d, up_limit_kib: u });
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
    el.querySelector<HTMLSelectElement>("select")?.focus();
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
    }
  }
</script>

<svelte:window onclick={onwindowclick} onresize={() => close()} {onkeydown} />

<div class="pop" role="dialog" aria-label="Speed limit" bind:this={el} style="left: {pos.left}px; bottom: {pos.bottom}px">
  {#if ui.limits.scheduled}
    <p class="note">
      A schedule is limiting speed right now{ui.limits.down_bps ? ` (↓ ${formatBytes(ui.limits.down_bps)}/s)` : ""}. What you pick here applies outside its time slots.
      <button class="link" onclick={schedule}>Edit the schedule</button>
    </p>
  {/if}
  <!-- A refused save puts the old values back, so the lines are drawn again from them. -->
  {#key ui.settingsRev}
    <div class="lines">
      {#if dir === "down"}
        <RateField label="↓ Download limit" value={down} onset={(k) => set("down", k)} />
      {:else}
        <RateField label="↑ Upload limit" value={up} onset={(k) => set("up", k)} />
      {/if}
    </div>
  {/key}
  <small>For all torrents together. Click the other speed to limit it too. To limit one torrent, right-click it and choose Speed limit.</small>
</div>

<style>
  .pop {
    position: fixed;
    z-index: 60;
    width: 320px;
    display: grid;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    background: var(--surface-2);
    box-shadow: var(--shadow-menu);
    font-size: 13px;
    color: var(--text-primary);
  }
  .lines {
    display: grid;
    gap: 8px;
  }
  small,
  .note {
    margin: 0;
    color: var(--text-muted);
    font-size: 11px;
    line-height: 1.4;
  }
  .note {
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
