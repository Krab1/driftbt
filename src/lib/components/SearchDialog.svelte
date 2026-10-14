<script lang="ts">
  import { onMount } from "svelte";
  import { api, type SearchReport, type SearchResult } from "../api";
  import { formatBytes } from "../../format";
  import { ui, errorText } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";
  import Icon from "./Icon.svelte";

  let text = $state("");
  let report = $state<SearchReport | null>(null);
  let busy = $state(false);
  let error = $state("");
  let input = $state<HTMLInputElement>();
  let seq = 0;

  onMount(() => input?.focus());

  async function run() {
    const q = text.trim();
    if (!q) return;
    const mine = ++seq;
    busy = true;
    error = "";
    try {
      const r = await api.search(q);
      if (mine === seq) report = r;
    } catch (e) {
      if (mine === seq) {
        report = null;
        error = errorText(e);
      }
    } finally {
      if (mine === seq) busy = false;
    }
  }

  const age = (secs: number | null) => {
    if (!secs) return "";
    const d = Math.max(0, Date.now() / 1000 - secs) / 86400;
    return d < 1 ? "today" : d < 60 ? `${Math.floor(d)} d` : d < 700 ? `${Math.floor(d / 30)} mo` : `${(d / 365).toFixed(1)} y`;
  };

  function add(r: SearchResult) {
    const link = r.magnet ?? r.torrent_url;
    if (!link) return;
    ui.searchOpen = false;
    ui.openAdd(link);
  }

  const answered = $derived(report ? report.sources.filter((s) => s.outcome === "ok").length : 0);
  const problems = $derived(report?.sources.filter((s) => s.outcome !== "ok") ?? []);
</script>

<Dialog title="Search" wide onclose={() => (ui.searchOpen = false)}>
  <form class="bar" onsubmit={(e) => (e.preventDefault(), run())}>
    <label class="q">
      <Icon name="search" />
      <input bind:this={input} type="search" placeholder="Search your sources" aria-label="Search your sources" bind:value={text} />
    </label>
    <button class="btn primary" type="submit" disabled={busy || !text.trim()}>{busy ? "Searching…" : "Search"}</button>
  </form>

  {#if error}
    <p class="note bad" role="alert">{error}</p>
    <button class="btn" onclick={() => ((ui.searchOpen = false), (ui.settingsOpen = true))}>Open settings</button>
  {:else if report}
    <p class="status" role="status">
      {answered} of {report.sources.length} source{report.sources.length === 1 ? "" : "s"} answered · {report.results.length} result{report.results.length === 1 ? "" : "s"}
      {#each problems as p (p.id)}<span class="bad"> · {p.name}: {p.message ?? "didn't answer"}</span>{/each}
    </p>
    {#if report.results.length}
      <ul class="results">
        {#each report.results as r, i (i)}
          <li>
            <div class="main">
              <span class="title" title={r.title}>{r.title}</span>
              <span class="meta">
                {#each r.sources as s (s)}<span class="badge">{s}</span>{/each}
                {#if r.category}<span>{r.category}</span>{/if}
                {#if r.size}<span class="num">{formatBytes(r.size)}</span>{/if}
                {#if r.published}<span>{age(r.published)}</span>{/if}
              </span>
            </div>
            <span class="peers num" title="Seeders / leechers">{#if r.seeders !== null}<b class="seed">{r.seeders}</b> / {r.leechers ?? "–"}{:else}–{/if}</span>
            <button class="btn" aria-label="Add {r.title}" disabled={!r.magnet && !r.torrent_url} onclick={() => add(r)}><Icon name="plus" />Add</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="note">Nothing found. Try fewer or different words.</p>
    {/if}
  {:else if !busy}
    <p class="note">Searches every source you turned on in Settings at once, and merges the same torrent into one result.</p>
  {/if}
</Dialog>

<style>
  .bar {
    display: flex;
    gap: 8px;
  }
  .q {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--radius);
    background: var(--surface-1);
    color: var(--text-muted);
  }
  .q:focus-within {
    outline: 2px solid var(--accent);
  }
  input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--text-primary);
    user-select: text;
  }
  .status,
  .note {
    margin: 10px 0 6px;
    color: var(--text-muted);
  }
  .bad {
    color: var(--danger-fg);
  }
  .results {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    border-top: 1px solid var(--border);
  }
  li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 2px;
    border-bottom: 1px solid var(--border);
  }
  .main {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: flex;
    gap: 8px;
    align-items: center;
    color: var(--text-muted);
    font-size: 12px;
  }
  .badge {
    padding: 0 6px;
    border-radius: 8px;
    background: var(--surface-1);
    color: var(--text-secondary);
  }
  .peers {
    min-width: 64px;
    text-align: right;
    color: var(--text-muted);
  }
  .seed {
    color: var(--accent-fg);
    font-weight: 500;
  }
</style>
