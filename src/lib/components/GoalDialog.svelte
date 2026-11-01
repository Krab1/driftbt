<script lang="ts">
  import { t } from "../i18n.svelte";
  import { api, type Goal } from "../api";
  import { torrents } from "../stores/torrents.svelte";
  import { ui, errorText } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";

  const target = ui.goalTarget!;
  const dflt = ui.settings?.default_goal;

  let custom = $state(false);
  let ratio = $state("");
  let minutes = $state("");

  const describe = (g: Goal | undefined) => {
    if (g?.ratio && g?.minutes) return t("Stop at ratio {ratio} or after {minutes} minutes", { ratio: g.ratio, minutes: g.minutes });
    if (g?.ratio) return t("Stop at ratio {ratio}", { ratio: g.ratio });
    if (g?.minutes) return t("Stop after {minutes} minutes", { minutes: g.minutes });
    return t("Never stop on its own");
  };

  const num = (v: string): number | null => {
    const n = Number(v);
    return Number.isFinite(n) && n > 0 ? n : null;
  };

  async function save() {
    const goal: Goal | null = custom ? { ratio: num(ratio), minutes: num(minutes) } : null;
    try {
      await api.setGoal(target.id, goal);
      await torrents.refresh();
      ui.toast(custom ? t("Seeding goal saved") : t("Using the default seeding goal"));
      ui.goalTarget = null;
    } catch (e) {
      ui.fail(e);
    }
  }
</script>

<Dialog title={t("Seeding goal")} onclose={() => (ui.goalTarget = null)}>
  <p class="name" title={target.name}>{target.name}</p>
  <div class="opts" role="radiogroup" aria-label={t("Goal")}>
    <label class="opt"><input type="radio" name="goal" checked={!custom} onchange={() => (custom = false)} /> {t("Use the default")} <span class="sub">({describe(dflt)})</span></label>
    <label class="opt"><input type="radio" name="goal" checked={custom} onchange={() => (custom = true)} /> {t("Set a goal for this torrent")}</label>
  </div>
  {#if custom}
    <div class="fields">
      <label>{t("Stop at ratio")} <input type="number" min="0" step="0.1" inputmode="decimal" placeholder={t("Never")} bind:value={ratio} /></label>
      <label>{t("or after")} <input type="number" min="0" step="10" inputmode="numeric" placeholder={t("Never")} bind:value={minutes} /> {t("minutes")}</label>
    </div>
    <small>{t("Leave a field empty to ignore it. Whichever is reached first stops seeding.")}</small>
  {/if}
  {#snippet footer()}
    <button class="btn" onclick={() => (ui.goalTarget = null)}>{t("Cancel")}</button>
    <button class="btn primary" onclick={save}>{t("Save")}</button>
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
  .opts {
    display: grid;
    gap: 8px;
  }
  .opt {
    display: flex;
    gap: 8px;
    align-items: baseline;
  }
  .sub {
    color: var(--text-muted);
  }
  input[type="radio"] {
    accent-color: var(--accent);
  }
  .fields {
    display: flex;
    gap: 16px;
    flex-wrap: wrap;
    align-items: center;
  }
  .fields label {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  input[type="number"] {
    width: 88px;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    text-align: right;
    user-select: text;
  }
  small {
    color: var(--text-muted);
  }
</style>
