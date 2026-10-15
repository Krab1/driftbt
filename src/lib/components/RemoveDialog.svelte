<script lang="ts">
  import { api } from "../api";
  import { torrents } from "../stores/torrents.svelte";
  import { ui, errorText } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";

  const KEY = "drift.deleteFiles";
  const target = ui.removeTarget!;

  function remembered(): boolean {
    try {
      return localStorage.getItem(KEY) === "1";
    } catch {
      return false;
    }
  }

  let deleteFiles = $state(remembered());

  async function confirm() {
    try {
      localStorage.setItem(KEY, deleteFiles ? "1" : "0");
    } catch {
      /* private mode: just don't remember */
    }
    try {
      await api.remove(target.id, deleteFiles);
      if (torrents.selectedId === target.id) torrents.selectedId = null;
      await torrents.refresh();
      ui.toast(deleteFiles ? "Torrent and files removed" : "Torrent removed");
    } catch (e) {
      ui.fail(e);
    }
    ui.removeTarget = null;
  }
</script>

<Dialog title="Remove torrent?" onclose={() => (ui.removeTarget = null)}>
  <p class="name" title={target.name}>{target.name}</p>
  <label class="check"><input type="checkbox" bind:checked={deleteFiles} /> Also delete downloaded files</label>
  {#snippet footer()}
    <button class="btn" onclick={() => (ui.removeTarget = null)}>Cancel</button>
    <button class="btn danger" onclick={confirm}>Remove</button>
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
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .danger {
    background: var(--danger);
    border-color: var(--danger);
    color: #fff;
  }
  .danger:hover {
    filter: brightness(1.08);
    background: var(--danger);
  }
</style>
