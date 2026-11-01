<script lang="ts">
  import { t } from "../i18n.svelte";
  import { auth } from "../stores/auth.svelte";

  let token = $state("");
  let remember = $state(false);
  let error = $state("");
  let busy = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = "";
    if (!(await auth.login(token, remember))) error = "That token isn't right, or the server can't be reached.";
    busy = false;
  }
</script>

<main class="wrap">
  <form onsubmit={submit}>
    <h1>{t("Drift")}</h1>
    <p>{t("Enter the access token for this server.")} <code>{t("drift-server --show-token")}</code> {t("prints it.")}</p>
    <label>
      <span>{t("Access token")}</span>
      <input type="password" autocomplete="current-password" bind:value={token} aria-invalid={!!error} aria-describedby={error ? "login-err" : undefined} />
    </label>
    <label class="check"><input type="checkbox" bind:checked={remember} /> {t("Remember on this device")}</label>
    {#if error}<p id="login-err" class="bad" role="alert">{error}</p>{/if}
    <button class="btn primary" type="submit" disabled={busy || !token.trim()}>{busy ? t("Checking…") : t("Sign in")}</button>
  </form>
</main>

<style>
  .wrap {
    height: 100%;
    display: grid;
    place-items: center;
    padding: 16px;
  }
  form {
    width: min(360px, 100%);
    display: grid;
    gap: 12px;
    padding: 20px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    background: var(--surface-2);
  }
  h1 {
    margin: 0;
    font-size: 18px;
  }
  p {
    margin: 0;
    color: var(--text-muted);
  }
  label {
    display: grid;
    gap: 6px;
    color: var(--text-secondary);
    font-size: 12px;
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  input[type="password"] {
    height: 32px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text-primary);
    user-select: text;
  }
  input[type="checkbox"] {
    accent-color: var(--accent);
    margin: 0;
  }
  .bad {
    color: var(--danger-fg);
  }
</style>
