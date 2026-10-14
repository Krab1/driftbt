<script lang="ts">
  import { api } from "../api";
  import { ui, errorText } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";

  let busy = $state(false);

  async function install() {
    busy = true;
    try {
      // The installer closes Drift and starts the new version, so on success this page goes away.
      await api.installUpdate!();
    } catch (e) {
      ui.toast(errorText(e), "error");
      busy = false;
    }
  }
</script>

<div class="banner" role="status">
  <Icon name="upload" />
  <span>Drift {ui.update?.version} is available.</span>
  <button class="btn primary" disabled={busy} onclick={install}>{busy ? "Updating…" : "Install and restart"}</button>
  <button class="btn ghost" disabled={busy} onclick={() => (ui.update = null)}>Later</button>
</div>

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    background: color-mix(in srgb, var(--accent) 14%, var(--bg));
    border-bottom: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .banner :global(svg) {
    color: var(--accent-fg);
    flex: none;
  }
  span {
    flex: 1;
  }
</style>
