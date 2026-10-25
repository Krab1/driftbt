<script lang="ts">
  import { untrack } from "svelte";
  import { MAX_KIB, parseRate } from "../rate";
  import Dialog from "./Dialog.svelte";

  let { dir, current, onset, onclose }: { dir: "down" | "up"; current: number; onset: (kib: number) => Promise<boolean>; onclose: () => void } = $props();

  // Starts from the limit in force, in the unit that reads best.
  const start = untrack(() => current);
  let text = $state(start <= 0 ? "" : start % 1024 === 0 ? String(start / 1024) : start >= 1024 ? (start / 1024).toFixed(3).replace(/\.?0+$/, "") : String(start));
  let mb = $state(start >= 1024);
  let busy = $state(false);
  const rate = $derived(parseRate(text, mb));
  // Nothing is said about an empty box until something has been typed.
  const shown = $derived(text.trim() === "" || rate.ok ? null : rate.why);

  async function save() {
    if (!rate.ok || busy) return;
    busy = true;
    try {
      // A refused save keeps the dialog open with what was typed; the reason is already on screen.
      if (await onset(rate.kib)) onclose();
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={dir === "down" ? "Custom download limit" : "Custom upload limit"} keep {onclose}>
  <form onsubmit={(e) => { e.preventDefault(); void save(); }}>
    <div class="row">
      <!-- svelte-ignore a11y_autofocus -->
      <input class="txt" type="text" inputmode="decimal" autocomplete="off" spellcheck="false" maxlength="12" autofocus placeholder="Speed" aria-label="Speed" aria-invalid={shown !== null} aria-describedby="rate-why" bind:value={text} />
      <select class="txt" aria-label="Unit" bind:value={mb}><option value={false}>KB/s</option><option value={true}>MB/s</option></select>
    </div>
    <small id="rate-why" class:bad={shown !== null} role="status">{shown ?? `Applies to all torrents together. Between 1 KB/s and ${Math.floor(MAX_KIB / 1024)} MB/s (10 Gbit/s).`}</small>
  </form>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={!rate.ok || busy} onclick={save}>Set limit</button>
  {/snippet}
</Dialog>

<style>
  form {
    display: grid;
    gap: 6px;
  }
  .row {
    display: grid;
    grid-template-columns: 1fr 84px;
    gap: 6px;
  }
  .txt {
    height: 28px;
    min-width: 0;
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
  small.bad {
    color: var(--danger-fg);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
