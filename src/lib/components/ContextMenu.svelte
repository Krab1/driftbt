<script lang="ts">
  import { api, type QueueMove } from "../api";
  import { torrents } from "../stores/torrents.svelte";
  import { ui, errorText } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";

  const menu = ui.menu!;
  const t = torrents.all.find((x) => x.id === menu.id);
  const canToggle = !!t && t.state !== "checking" && t.state !== "error";
  const inQueue = !!t && t.queue_pos !== null;
  const categories = ui.settings?.categories ?? [];
  const queueMoves: { to: QueueMove; label: string }[] = [
    { to: "top", label: "Move to top of queue" },
    { to: "up", label: "Move up in queue" },
    { to: "down", label: "Move down in queue" },
    { to: "bottom", label: "Move to bottom of queue" },
  ];

  let el = $state<HTMLDivElement>();
  let pos = $state({ x: menu.x, y: menu.y });

  // Keep the menu inside the window, then focus the first item for keyboard use.
  $effect(() => {
    if (!el) return;
    const r = el.getBoundingClientRect();
    pos = { x: Math.min(menu.x, innerWidth - r.width - 8), y: Math.min(menu.y, innerHeight - r.height - 8) };
    el.querySelector<HTMLButtonElement>("button")?.focus();
  });

  /** Closing returns focus to the torrent list so keyboard users keep their place. */
  function close() {
    ui.menu = null;
    document.querySelector<HTMLElement>('[role="grid"]')?.focus();
  }

  async function toggle() {
    close();
    if (!t) return;
    try {
      await (t.state === "paused" ? api.resume(t.id) : api.pause(t.id));
      await torrents.refresh();
    } catch (e) {
      ui.fail(e);
    }
  }

  async function recheck() {
    close();
    if (!t) return;
    try {
      await api.recheck(t.id);
      await torrents.refresh();
      ui.toast("Checking the downloaded files. Anything missing or damaged will be downloaded again.");
    } catch (e) {
      ui.fail(e);
    }
  }

  async function moveFiles() {
    close();
    if (!t) return;
    try {
      const dir = await api.pickFolder(ui.settings?.download_dir ?? null);
      if (!dir) return;
      await api.moveTorrent(t.id, dir);
      await torrents.refresh();
      ui.toast(`Moved to ${dir}`);
    } catch (e) {
      ui.fail(e);
    }
  }

  async function play() {
    close();
    if (!t) return;
    try {
      await api.play(t.id);
      await torrents.refresh();
    } catch (e) {
      ui.fail(e);
    }
  }

  async function playlist() {
    close();
    if (!t) return;
    try {
      const path = await api.createPlaylist(t.id);
      ui.toast(`Playlist saved: ${path}`);
    } catch (e) {
      ui.fail(e);
    }
  }

  async function reveal() {
    close();
    if (t) await api.revealTorrent(t.id).catch((e) => ui.fail(e));
  }

  async function copyMagnet() {
    close();
    if (!t) return;
    try {
      await navigator.clipboard.writeText(`magnet:?xt=urn:btih:${t.id}&dn=${encodeURIComponent(t.name)}`);
      ui.toast("Magnet link copied");
    } catch {
      ui.toast("Couldn't copy to the clipboard.", "error");
    }
  }

  async function move(to: QueueMove) {
    close();
    if (!t) return;
    try {
      await api.moveInQueue(t.id, to);
      await torrents.refresh();
    } catch (e) {
      ui.fail(e);
    }
  }

  async function setCategory(name: string | null) {
    close();
    if (!t) return;
    try {
      await api.setCategory(t.id, name);
      await torrents.refresh();
    } catch (e) {
      ui.fail(e);
    }
  }

  function goal() {
    close();
    if (t) ui.goalTarget = { id: t.id, name: t.name };
  }

  function tags() {
    close();
    if (t) ui.tagTarget = { id: t.id, name: t.name };
  }

  function remove() {
    close();
    if (t) ui.removeTarget = { id: t.id, name: t.name };
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") return close();
    const items = [...(el?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "ArrowDown") (e.preventDefault(), items[(i + 1) % items.length]?.focus());
    if (e.key === "ArrowUp") (e.preventDefault(), items[(i - 1 + items.length) % items.length]?.focus());
  }
</script>

<svelte:window onclick={close} onblur={close} onresize={close} {onkeydown} />

<div class="menu" role="menu" tabindex="-1" bind:this={el} style="left: {pos.x}px; top: {pos.y}px" oncontextmenu={(e) => e.preventDefault()}>
  {#if t && t.state !== "error"}
    <button role="menuitem" onclick={play}><Icon name="play" />Play</button>
  {/if}
  {#if canToggle}
    <button role="menuitem" onclick={toggle}><Icon name={t!.state === "paused" ? "play" : "pause"} />{t!.state === "paused" ? "Resume" : "Pause"}</button>
  {/if}
  {#if t && t.total_bytes > 0 && t.state !== "checking"}
    <button role="menuitem" onclick={recheck}><Icon name="check" />Check downloaded files</button>
  {/if}
  {#if t && t.total_bytes > 0 && t.state !== "checking"}
    <button role="menuitem" onclick={moveFiles}><Icon name="folder" />Move files…</button>
  {/if}
  <button role="menuitem" onclick={reveal}><Icon name="folder" />Show in folder</button>
  <button role="menuitem" onclick={playlist}><Icon name="list" />Create playlist</button>
  <button role="menuitem" onclick={copyMagnet}><Icon name="file" />Copy magnet link</button>
  {#if inQueue}
    <hr />
    {#each queueMoves as m (m.to)}
      <button role="menuitem" onclick={() => move(m.to)}><Icon name={m.to === "top" || m.to === "up" ? "up" : "down"} />{m.label}</button>
    {/each}
  {/if}
  {#if categories.length}
    <hr />
    <div class="lbl" id="cm-cat">Category</div>
    <div role="group" aria-labelledby="cm-cat">
      <button role="menuitemradio" aria-checked={!t?.category} onclick={() => setCategory(null)}><span class="tick">{!t?.category ? "✓" : ""}</span>None</button>
      {#each categories as c (c.name)}
        <button role="menuitemradio" aria-checked={t?.category === c.name} onclick={() => setCategory(c.name)}><span class="tick">{t?.category === c.name ? "✓" : ""}</span>{c.name}</button>
      {/each}
    </div>
  {/if}
  <hr />
  <button role="menuitem" onclick={tags}><Icon name="file" />Tags…</button>
  <button role="menuitem" onclick={goal}><Icon name="clock" />Seeding goal…</button>
  <button role="menuitem" onclick={remove}><Icon name="trash" />Remove…</button>
</div>

<style>
  .menu {
    position: fixed;
    z-index: 50;
    min-width: 160px;
    padding: 4px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    background: var(--surface-2);
    box-shadow: var(--shadow-menu);
  }
  button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 30px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    text-align: left;
  }
  hr {
    margin: 4px 0;
    border: 0;
    border-top: 1px solid var(--border);
  }
  .lbl {
    padding: 4px 10px 2px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .tick {
    display: inline-block;
    width: 16px;
    text-align: center;
  }
  button:hover,
  button:focus-visible {
    background: var(--hover);
    outline: none;
  }
</style>
