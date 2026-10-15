<script lang="ts">
  import { untrack } from "svelte";
  import { api } from "../api";
  import { torrents } from "../stores/torrents.svelte";
  import { ui } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";
  import RateField from "./RateField.svelte";

  const target = ui.limitTarget!;
  const now = untrack(() => torrents.all.find((x) => x.id === target.id));
  const before = { down: now?.limit_down_kib ?? 0, up: now?.limit_up_kib ?? 0 };
  let down = $state(before.down);
  let up = $state(before.up);
  const changed = $derived(down !== before.down || up !== before.up);
  const overall = $derived(ui.settings?.limits_enabled ? ui.settings : null);

  async function save() {
    try {
      await api.setTorrentLimits(target.id, down, up);
      await torrents.refresh();
      ui.toast(down || up ? "Speed limit saved for this torrent" : "This torrent has no speed limit of its own now");
      ui.limitTarget = null;
    } catch (e) {
      ui.fail(e);
    }
  }
</script>

<Dialog title="Speed limit for this torrent" onclose={() => (ui.limitTarget = null)}>
  <p class="name" title={target.name}>{target.name}</p>
  <div class="lines">
    <RateField label="↓ Download" value={down} live onset={(k) => (down = k)} />
    <RateField label="↑ Upload" value={up} live onset={(k) => (up = k)} />
  </div>
  <small>
    The overall limit{overall ? " (set in the status bar)" : ""} still applies: this torrent never goes faster than it. Saving restarts the torrent briefly.
  </small>
  {#snippet footer()}
    <button class="btn" onclick={() => (ui.limitTarget = null)}>Cancel</button>
    <button class="btn primary" disabled={!changed} onclick={save}>Save</button>
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
  .lines {
    display: grid;
    gap: 8px;
  }
  small {
    color: var(--text-muted);
    line-height: 1.4;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
