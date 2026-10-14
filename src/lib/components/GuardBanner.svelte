<script lang="ts">
  import { ui } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";

  function openSettings() {
    try {
      localStorage.setItem("drift.settingsTab", "network");
    } catch {
      /* the dialog just opens on its last tab */
    }
    ui.settingsOpen = true;
  }
  const n = $derived(ui.guard.paused);
</script>

<div class="banner" role="alert">
  <Icon name="alert" />
  <span>
    Your VPN isn't connected.
    {#if n}{n} {n === 1 ? "torrent is" : "torrents are"} paused{:else}Torrents are on hold{/if}
    until it is back.
  </span>
  <button class="btn" onclick={openSettings}>VPN settings</button>
</div>

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    background: color-mix(in srgb, var(--danger) 14%, var(--bg));
    border-bottom: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
  }
  .banner :global(svg) {
    color: var(--danger-fg);
    flex: none;
  }
  span {
    flex: 1;
  }
</style>
