<script lang="ts">
  import { ui } from "../stores/ui.svelte";
</script>

<div class="toasts" aria-live="polite">
  {#each ui.toasts as t (t.id)}
    <div class="toast" class:error={t.kind === "error"} role={t.kind === "error" ? "alert" : "status"}>{t.text}</div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 40px;
    display: grid;
    gap: 8px;
    z-index: 100;
    pointer-events: none;
  }
  .toast {
    padding: 8px 14px;
    border-radius: var(--radius);
    background: var(--text-primary);
    color: var(--bg);
    box-shadow: var(--shadow-menu);
    max-width: 360px;
  }
  .toast.error {
    background: var(--danger);
    color: #fff;
  }
</style>
