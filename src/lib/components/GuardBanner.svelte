<script lang="ts">
  import { t } from "../i18n.svelte";
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
    {t("Your VPN isn't connected.")}
    {#if n}{n} {n === 1 ? "torrent is" : "torrents are"} paused{:else}{t("Torrents are on hold")}{/if}
    {t("until it is back.")}
  </span>
  <button class="btn" onclick={openSettings}>{t("VPN settings")}</button>
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
