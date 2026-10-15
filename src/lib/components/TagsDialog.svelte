<script lang="ts">
  import { api } from "../api";
  import { torrents } from "../stores/torrents.svelte";
  import { ui, errorText } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";

  const MAX = 12;
  const target = ui.tagTarget!;
  const current = torrents.all.find((t) => t.id === target.id)?.tags ?? [];

  let tags = $state<string[]>([...current]);
  let text = $state("");
  const suggestions = $derived([...torrents.tagCounts.keys()].filter((t) => !tags.some((x) => x.toLowerCase() === t.toLowerCase())));

  function add(raw: string) {
    for (const part of raw.split(",")) {
      const t = part.trim().replace(/\s+/g, " ").slice(0, 32);
      if (t && tags.length < MAX && !tags.some((x) => x.toLowerCase() === t.toLowerCase())) tags = [...tags, t];
    }
    text = "";
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault();
      if (text.trim()) add(text);
      else if (e.key === "Enter") void save();
    } else if (e.key === "Backspace" && !text) tags = tags.slice(0, -1);
  }

  async function save() {
    if (text.trim()) add(text);
    try {
      await api.setTags(target.id, tags);
      await torrents.refresh();
      ui.tagTarget = null;
    } catch (e) {
      ui.toast(errorText(e), "error");
    }
  }
</script>

<Dialog title="Tags" onclose={() => (ui.tagTarget = null)}>
  <p class="name" title={target.name}>{target.name}</p>
  <div class="box">
    {#each tags as t (t)}
      <span class="chip">{t}<button aria-label="Remove tag {t}" onclick={() => (tags = tags.filter((x) => x !== t))}>×</button></span>
    {/each}
    <!-- svelte-ignore a11y_autofocus -->
    <input type="text" aria-label="Add a tag" placeholder={tags.length ? "" : "Type a tag and press Enter"} maxlength="32" autofocus bind:value={text} {onkeydown} disabled={tags.length >= MAX} />
  </div>
  {#if suggestions.length}
    <div class="sug" role="group" aria-label="Existing tags">
      {#each suggestions as s (s)}<button class="btn" onclick={() => add(s)}>{s}</button>{/each}
    </div>
  {/if}
  <small>Separate tags with Enter or a comma. Up to {MAX} tags. Click a tag in the left list to show only its torrents.</small>
  {#snippet footer()}
    <button class="btn" onclick={() => (ui.tagTarget = null)}>Cancel</button>
    <button class="btn primary" onclick={save}>Save</button>
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
  .box {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    min-height: 36px;
    padding: 4px 6px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
  }
  .box input {
    flex: 1;
    min-width: 120px;
    height: 26px;
    border: 0;
    background: transparent;
    outline: none;
    user-select: text;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 4px 0 8px;
    border-radius: 12px;
    background: var(--selected);
    color: var(--accent-fg);
    font-size: 12px;
  }
  .chip button {
    width: 18px;
    height: 18px;
    border: 0;
    border-radius: 9px;
    background: transparent;
    color: inherit;
    line-height: 1;
  }
  .chip button:hover {
    background: var(--hover);
  }
  .sug {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .sug .btn {
    height: 24px;
    padding: 0 8px;
    font-size: 12px;
  }
  small {
    color: var(--text-muted);
  }
</style>
