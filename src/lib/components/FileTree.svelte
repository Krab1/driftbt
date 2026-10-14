<script lang="ts">
  import { formatBytes, percent } from "../../format";
  import { buildTree, checkState, isMedia, toggleNode, type FileLike, type TreeNode } from "../filetree";
  import Icon from "./Icon.svelte";

  let {
    files,
    selected,
    onchange,
    showProgress = false,
    onplay,
  }: {
    files: FileLike[];
    selected: ReadonlySet<number>;
    /** Called with the new full selection whenever a box is toggled. */
    onchange: (next: Set<number>) => void;
    showProgress?: boolean;
    /** Shows a Play button on video/audio files. */
    onplay?: (fileIndex: number) => void;
  } = $props();

  const tree = $derived(buildTree(files));
  let collapsed = $state(new Set<string>());

  function toggleOpen(key: string) {
    const next = new Set(collapsed);
    next.has(key) ? next.delete(key) : next.add(key);
    collapsed = next;
  }
</script>

{#snippet row(n: TreeNode, depth: number)}
  {@const st = checkState(n, selected)}
  <div class="row" role="treeitem" aria-level={depth + 1} aria-selected={st === "all"} aria-expanded={n.isFile ? undefined : !collapsed.has(n.key)} style="padding-left: {depth * 16 + 4}px">
    {#if n.isFile}
      <span class="twist"></span>
    {:else}
      <button class="twist" aria-label={collapsed.has(n.key) ? `Expand ${n.name}` : `Collapse ${n.name}`} onclick={() => toggleOpen(n.key)}>
        <span class="chev" class:open={!collapsed.has(n.key)}>▸</span>
      </button>
    {/if}
    <input type="checkbox" aria-label={n.name} checked={st === "all"} indeterminate={st === "some"} onchange={() => onchange(toggleNode(n, selected))} />
    <Icon name={n.isFile ? "file" : "folder"} size={14} />
    <span class="nm" title={n.key}>{n.name}</span>
    {#if showProgress}<span class="pc num">{percent(n.progress, n.size)}%</span>{/if}
    <span class="sz num">{formatBytes(n.size)}</span>
    {#if onplay && n.isFile && isMedia(n.key)}
      <button class="play" aria-label="Play {n.name}" title="Play while downloading" onclick={() => onplay(n.indices[0])}><Icon name="play" size={13} /></button>
    {:else if onplay}
      <span class="play-gap"></span>
    {/if}
  </div>
  {#if !n.isFile && !collapsed.has(n.key)}
    {#each n.children as c (c.key)}{@render row(c, depth + 1)}{/each}
  {/if}
{/snippet}

<div class="tree" role="tree" aria-label="Files">
  {#each tree as n (n.key)}{@render row(n, 0)}{/each}
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding-right: 8px;
  }
  .row:hover {
    background: var(--hover);
  }
  .twist {
    flex: none;
    width: 16px;
    height: 16px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-muted);
    display: grid;
    place-items: center;
  }
  .chev {
    display: inline-block;
    font-size: 10px;
    transition: transform var(--dur);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  input[type="checkbox"] {
    margin: 0;
    accent-color: var(--accent);
  }
  .row :global(svg) {
    flex: none;
    color: var(--text-muted);
  }
  .nm {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sz,
  .pc {
    flex: none;
    color: var(--text-secondary);
    text-align: right;
  }
  .sz {
    width: 70px;
  }
  .play {
    flex: none;
    width: 24px;
    height: 22px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--accent-fg);
  }
  .play:hover {
    background: var(--selected);
  }
  .play-gap {
    flex: none;
    width: 24px;
  }
  .pc {
    width: 40px;
  }
</style>
