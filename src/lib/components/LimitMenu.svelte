<script lang="ts">
  import { untrack } from "svelte";
  import { formatBytes } from "../../format";
  import { ui } from "../stores/ui.svelte";

  let { anchor, onclose }: { anchor: HTMLElement; onclose: () => void } = $props();

  // Each preset shows exactly what it sets, so nothing is hidden behind a name.
  const PRESETS = [
    { down: 1024, up: 256 },
    { down: 5120, up: 1024 },
    { down: 20480, up: 5120 },
  ];
  const MAX_KIB = 4_000_000;
  const rate = (kib: number) => (kib >= 1024 && kib % 1024 === 0 ? `${kib / 1024} MB/s` : `${kib} KB/s`);
  const presetLabel = (p: { down: number; up: number }) => `↓ ${rate(p.down)}   ↑ ${rate(p.up)}`;

  const s = $derived(ui.settings);
  const matching = $derived(s?.limits_enabled ? PRESETS.findIndex((p) => p.down === s.down_limit_kib && p.up === s.up_limit_kib) : -1);
  let customPicked = $state(false);
  const choice = $derived(!s?.limits_enabled && !customPicked ? "off" : customPicked || matching < 0 ? "custom" : `p${matching}`);

  // The two custom fields: a number and a unit each. Empty means no limit in that direction.
  function split(kib: number) {
    if (kib <= 0) return { n: "", mb: false };
    return kib % 1024 === 0 ? { n: String(kib / 1024), mb: true } : { n: String(kib), mb: false };
  }
  // Only the starting values: the fields are the user's to edit from here.
  const start = split(untrack(() => s?.down_limit_kib ?? 0));
  const startUp = split(untrack(() => s?.up_limit_kib ?? 0));
  let down = $state(start.n);
  let downMb = $state(start.mb);
  let up = $state(startUp.n);
  let upMb = $state(startUp.mb);

  const toKib = (n: string, mb: boolean) => {
    const v = Math.floor(Number(n));
    return Number.isFinite(v) && v > 0 ? Math.min(MAX_KIB, v * (mb ? 1024 : 1)) : 0;
  };
  const customDown = $derived(toKib(down, downMb));
  const customUp = $derived(toKib(up, upMb));
  const canApply = $derived(customDown > 0 || customUp > 0);

  async function off() {
    customPicked = false;
    if (await ui.saveSettings({ limits_enabled: false })) onclose();
  }
  async function preset(i: number) {
    customPicked = false;
    if (await ui.saveSettings({ limits_enabled: true, down_limit_kib: PRESETS[i].down, up_limit_kib: PRESETS[i].up })) onclose();
  }
  async function apply() {
    if (!canApply) return;
    if (await ui.saveSettings({ limits_enabled: true, down_limit_kib: customDown, up_limit_kib: customUp })) onclose();
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
    el.querySelector<HTMLInputElement>("input[type=radio]:checked")?.focus();
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

<svelte:window onclick={onwindowclick} onblur={() => close()} onresize={() => close()} {onkeydown} />

<div class="pop" role="dialog" aria-label="Speed limit" bind:this={el} style="left: {pos.left}px; bottom: {pos.bottom}px">
  {#if ui.limits.scheduled}
    <p class="note">
      A schedule is limiting speed right now{ui.limits.down_bps ? ` (↓ ${formatBytes(ui.limits.down_bps)}/s)` : ""}. What you pick here applies outside its time slots.
      <button class="link" onclick={schedule}>Edit the schedule</button>
    </p>
  {/if}
  <fieldset>
    <legend class="sr-only">Speed limit</legend>
    <label class="row"><input type="radio" name="limit" checked={choice === "off"} onchange={off} /><span>Off</span><small>No limit</small></label>
    {#each PRESETS as p, i (i)}
      <label class="row"><input type="radio" name="limit" checked={choice === `p${i}`} onchange={() => preset(i)} /><span class="num">{presetLabel(p)}</span></label>
    {/each}
    <label class="row"><input type="radio" name="limit" checked={choice === "custom"} onchange={() => (customPicked = true)} /><span>Custom…</span></label>
  </fieldset>
  {#if choice === "custom"}
    <form class="custom" onsubmit={(e) => (e.preventDefault(), apply())}>
      <div class="line">
        <label for="lim-down">↓ Download</label>
        <input id="lim-down" type="number" min="0" step="1" inputmode="numeric" placeholder="No limit" bind:value={down} />
        <select aria-label="Download unit" bind:value={downMb}><option value={false}>KB/s</option><option value={true}>MB/s</option></select>
      </div>
      <div class="line">
        <label for="lim-up">↑ Upload</label>
        <input id="lim-up" type="number" min="0" step="1" inputmode="numeric" placeholder="No limit" bind:value={up} />
        <select aria-label="Upload unit" bind:value={upMb}><option value={false}>KB/s</option><option value={true}>MB/s</option></select>
      </div>
      <div class="foot">
        <small>Leave one empty for no limit in that direction.</small>
        <button class="btn primary" type="submit" disabled={!canApply}>Apply</button>
      </div>
    </form>
  {/if}
</div>

<style>
  .pop {
    position: fixed;
    z-index: 60;
    width: 290px;
    padding: 4px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    background: var(--surface-2);
    box-shadow: var(--shadow-menu);
    font-size: 13px;
    color: var(--text-primary);
  }
  fieldset {
    margin: 0;
    padding: 0;
    border: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--radius);
    cursor: pointer;
  }
  .row:hover {
    background: var(--hover);
  }
  .row:has(input:checked) {
    color: var(--accent-fg);
  }
  .row input {
    accent-color: var(--accent);
    margin: 0;
  }
  .row small {
    margin-left: auto;
    color: var(--text-muted);
  }
  .custom {
    display: grid;
    gap: 8px;
    margin: 2px 6px 6px;
    padding: 10px;
    border-top: 1px solid var(--border);
  }
  .line {
    display: grid;
    grid-template-columns: 84px 1fr 72px;
    align-items: center;
    gap: 6px;
  }
  .line input,
  .line select {
    height: 28px;
    min-width: 0;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--surface-1);
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .foot small,
  .note {
    color: var(--text-muted);
    font-size: 11px;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .note {
    margin: 4px 8px 6px;
    padding: 6px 8px;
    border-radius: var(--radius);
    background: var(--surface-1);
    line-height: 1.4;
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
