<script lang="ts">
  import { t, td } from "../i18n.svelte";
  import { formatBytes, formatEta, formatSpeed } from "../../format";
  import { ratioTable } from "../ratio";
  import { torrents } from "../stores/torrents.svelte";
  import { ui } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";

  let target = $state(1);
  const rows = $derived(ratioTable(torrents.all, target > 0 ? target : 0));
</script>

<Dialog title={t("Ratios per tracker")} onclose={() => (ui.ratioOpen = false)}>
  <label class="target">{t("Target ratio")} <input type="number" min="0" step="0.1" inputmode="decimal" bind:value={target} /></label>

  {#if rows.length === 0}
    <p class="hint">{t("Add a torrent to see how much you've shared.")}</p>
  {:else}
    <div class="scroll">
      <table>
        <thead>
          <tr><th>{t("Tracker")}</th><th class="r">{t("Torrents")}</th><th class="r">{t("Uploaded")}</th><th class="r">{t("Ratio")}</th><th class="r">{t("Still to upload")}</th><th class="r">{t("At current speed")}</th></tr>
        </thead>
        <tbody>
          {#each rows as r (r.tracker)}
            <tr>
              <td class="tr" title={td(r.tracker)}>{td(r.tracker)}</td>
              <td class="r num">{r.torrents}</td>
              <td class="r num">{formatBytes(r.uploaded)}</td>
              <td class="r num" class:ok={r.gap === 0}>{r.ratio.toFixed(2)}</td>
              <td class="r num">{r.gap === 0 ? t("Reached") : formatBytes(r.gap)}</td>
              <td class="r num">{r.gap === 0 ? "—" : r.etaSecs === null ? t("Not uploading") : `${formatEta(r.etaSecs)} (${formatSpeed(r.upBps)})`}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}

  <small>
    {t("These are true numbers from this app. Ratio here is uploaded bytes over torrent size, because Drift can't see what a tracker has recorded as your lifetime download. \"Still to upload\" is real upload you'd have to do to reach the target; seeding the torrents that have leechers is the fastest way, and an idle torrent with nobody to upload to can't earn ratio.")}
  </small>

  {#snippet footer()}
    <button class="btn" onclick={() => (ui.ratioOpen = false)}>{t("Done")}</button>
  {/snippet}
</Dialog>

<style>
  .target {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  input {
    width: 80px;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    text-align: right;
    user-select: text;
  }
  .scroll {
    overflow: auto;
    max-height: 260px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    font-weight: 400;
    font-size: 11px;
    color: var(--text-muted);
    text-align: left;
    padding: 4px 8px 4px 0;
    white-space: nowrap;
  }
  td {
    padding: 6px 8px 6px 0;
    border-top: 1px solid var(--border);
    white-space: nowrap;
  }
  .r {
    text-align: right;
  }
  .tr {
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ok {
    color: var(--success);
  }
  .hint,
  small {
    margin: 0;
    color: var(--text-muted);
  }
</style>
