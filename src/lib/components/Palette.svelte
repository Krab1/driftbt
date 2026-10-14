<script lang="ts">
  import { buildCommands } from "../commands";
  import { rank } from "../fuzzy";
  import { ui } from "../stores/ui.svelte";

  const all = buildCommands();
  let el = $state<HTMLDialogElement>();
  let input = $state<HTMLInputElement>();
  let query = $state("");
  let active = $state(0);

  const results = $derived(rank(all, query, (c) => `${c.title} ${c.group}`));
  // Keep the highlighted row valid as the result list shrinks.
  $effect(() => {
    if (active >= results.length) active = Math.max(0, results.length - 1);
  });
  $effect(() => {
    el?.showModal();
    input?.focus();
  });

  async function run(i: number) {
    const cmd = results[i];
    if (!cmd) return;
    el?.close();
    await cmd.run();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") (e.preventDefault(), (active = Math.min(results.length - 1, active + 1)));
    else if (e.key === "ArrowUp") (e.preventDefault(), (active = Math.max(0, active - 1)));
    else if (e.key === "Enter") (e.preventDefault(), run(active));
    // keep the highlighted option in view
    queueMicrotask(() => document.getElementById(`cmd-${active}`)?.scrollIntoView({ block: "nearest" }));
  }
</script>

<dialog bind:this={el} aria-label="Command palette" onclose={() => (ui.paletteOpen = false)} onclick={(e) => e.target === el && el?.close()}>
  <div class="box">
    <input
      bind:this={input}
      bind:value={query}
      oninput={() => (active = 0)}
      {onkeydown}
      role="combobox"
      aria-expanded="true"
      aria-controls="cmd-list"
      aria-activedescendant={results.length ? `cmd-${active}` : undefined}
      aria-label="Type a command"
      placeholder="Type a command…"
      autocomplete="off"
      spellcheck="false"
    />
    <ul id="cmd-list" role="listbox" aria-label="Commands">
      {#each results as c, i (c.id)}
        <li id="cmd-{i}" role="option" aria-selected={i === active} class:on={i === active} onmousemove={() => (active = i)} onclick={() => run(i)} onkeydown={() => {}}>
          <span class="t">{c.title}</span>
          {#if c.shortcut}<kbd>{c.shortcut}</kbd>{:else}<span class="g">{c.group}</span>{/if}
        </li>
      {:else}
        <li class="none" role="presentation">No matching command.</li>
      {/each}
    </ul>
  </div>
</dialog>

<style>
  dialog {
    margin: 12vh auto auto;
    padding: 0;
    width: min(560px, calc(100vw - 32px));
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    background: var(--surface-2);
    color: var(--text-primary);
    box-shadow: var(--shadow-menu);
  }
  dialog::backdrop {
    background: rgba(0, 0, 0, 0.35);
  }
  input {
    display: block;
    width: 100%;
    height: 44px;
    padding: 0 16px;
    border: 0;
    border-bottom: 1px solid var(--border);
    background: transparent;
    font-size: 15px;
    outline: none;
    user-select: text;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 6px;
    max-height: 340px;
    overflow: auto;
  }
  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: 34px;
    padding: 0 10px;
    border-radius: var(--radius);
    cursor: default;
  }
  li.on {
    background: var(--selected);
    color: var(--accent-fg);
  }
  .g,
  kbd {
    font-size: 11px;
    color: var(--text-muted);
    font-family: inherit;
  }
  li.on .g,
  li.on kbd {
    color: inherit;
  }
  .none {
    color: var(--text-muted);
    justify-content: center;
  }
</style>
