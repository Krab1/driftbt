<script lang="ts">
  import { ui } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";

  const view = ui.detailView!;
  let copied = $state(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(`${view.title}\n\n${view.text}`);
      copied = true;
    } catch {
      ui.toast("Couldn't copy to the clipboard.", "error");
    }
  }
</script>

<Dialog title="Error details" onclose={() => (ui.detailView = null)}>
  <p class="what">{view.title}</p>
  <pre>{view.text}</pre>
  <small>This is the technical text behind the message. It helps when asking for support, and it may contain file paths from this computer.</small>
  {#snippet footer()}
    <button class="btn" onclick={copy}>{copied ? "Copied" : "Copy"}</button>
    <button class="btn primary" onclick={() => (ui.detailView = null)}>Close</button>
  {/snippet}
</Dialog>

<style>
  .what {
    margin: 0;
    color: var(--text-secondary);
  }
  pre {
    margin: 0;
    max-height: 260px;
    overflow: auto;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg);
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
  small {
    color: var(--text-muted);
  }
</style>
