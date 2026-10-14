<script lang="ts">
  import type { Snippet } from "svelte";

  // Native <dialog>: focus trap, Esc, and inert backdrop come from the browser.
  let { title, onclose, children, footer, wide = false }: { title: string; onclose: () => void; children: Snippet; footer?: Snippet; wide?: boolean } = $props();
  let el = $state<HTMLDialogElement>();

  $effect(() => {
    el?.showModal();
  });
</script>

<dialog bind:this={el} class:wide aria-labelledby="dlg-title" {onclose} onclick={(e) => e.target === el && el?.close()}>
  <div class="box">
    <h2 id="dlg-title">{title}</h2>
    <div class="body">{@render children()}</div>
    {#if footer}<div class="foot">{@render footer()}</div>{/if}
  </div>
</dialog>

<style>
  dialog {
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    background: var(--surface-2);
    color: var(--text-primary);
    box-shadow: var(--shadow-menu);
    width: min(520px, calc(100vw - 32px));
  }
  /* Tall content (settings) scrolls inside the dialog instead of running off the window. */
  dialog.wide {
    width: min(700px, calc(100vw - 32px));
    max-height: calc(100vh - 48px);
  }
  dialog.wide .body {
    overflow: auto;
    max-height: calc(100vh - 180px);
    padding-right: 4px;
  }
  dialog::backdrop {
    background: rgba(0, 0, 0, 0.4);
  }
  .box {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  .body {
    display: grid;
    gap: 16px;
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
