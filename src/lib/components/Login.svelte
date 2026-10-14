<script lang="ts">
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
    <h1>Drift</h1>
    <p>Enter the access token for this server. <code>drift-server --show-token</code> prints it.</p>
    <label>
      <span>Access token</span>
      <input type="password" autocomplete="current-password" bind:value={token} aria-invalid={!!error} aria-describedby={error ? "login-err" : undefined} />
    </label>
    <label class="check"><input type="checkbox" bind:checked={remember} /> Remember on this device</label>
    {#if error}<p id="login-err" class="bad" role="alert">{error}</p>{/if}
    <button class="btn primary" type="submit" disabled={busy || !token.trim()}>{busy ? "Checking…" : "Sign in"}</button>
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
