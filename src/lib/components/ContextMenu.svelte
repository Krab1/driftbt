<script lang="ts">
  import { api } from "../api";
  import { torrents } from "../stores/torrents.svelte";
  import { ui, errorText } from "../stores/ui.svelte";
  import Icon from "./Icon.svelte";

  const menu = ui.menu!;
  const t = torrents.all.find((x) => x.id === menu.id);
  const canToggle = !!t && t.state !== "checking" && t.state !== "error";

  let el = $state<HTMLDivElement>();
  let pos = $state({ x: menu.x, y: menu.y });

  // Keep the menu inside the window, then focus the first item for keyboard use.
  $effect(() => {
    if (!el) return;
    const r = el.getBoundingClientRect();
    pos = { x: Math.min(menu.x, innerWidth - r.width - 8), y: Math.min(menu.y, innerHeight - r.height - 8) };
    el.querySelector<HTMLButtonElement>("button")?.focus();
  });

  function close() {
    ui.menu = null;
  }

  async function toggle() {
    close();
    if (!t) return;
    try {
      await (t.state === "paused" ? api.resume(t.id) : api.pause(t.id));
      await torrents.refresh();
    } catch (e) {
      ui.toast(errorText(e), "error");
    }
  }

  async function reveal() {
    close();
    if (t) await api.revealTorrent(t.id).catch((e) => ui.toast(errorText(e), "error"));
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
  {#if canToggle}
    <button role="menuitem" onclick={toggle}><Icon name={t!.state === "paused" ? "play" : "pause"} />{t!.state === "paused" ? "Resume" : "Pause"}</button>
  {/if}
  <button role="menuitem" onclick={reveal}><Icon name="folder" />Show in folder</button>
  <button role="menuitem" onclick={copyMagnet}><Icon name="file" />Copy magnet link</button>
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
  button:hover,
  button:focus-visible {
    background: var(--hover);
    outline: none;
  }
</style>
