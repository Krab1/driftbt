<script lang="ts">
  import { onMount } from "svelte";
  import AddDialog from "./lib/components/AddDialog.svelte";
  import ContextMenu from "./lib/components/ContextMenu.svelte";
  import Icon from "./lib/components/Icon.svelte";
  import Rail from "./lib/components/Rail.svelte";
  import RemoveDialog from "./lib/components/RemoveDialog.svelte";
  import SettingsDialog from "./lib/components/SettingsDialog.svelte";
  import StatusBar from "./lib/components/StatusBar.svelte";
  import Toasts from "./lib/components/Toasts.svelte";
  import ErrorDetails from "./lib/components/ErrorDetails.svelte";
  import TopBar from "./lib/components/TopBar.svelte";
  import TorrentList from "./lib/components/TorrentList.svelte";
  import { api, type OpenPayload } from "./lib/api";
  import GoalDialog from "./lib/components/GoalDialog.svelte";
  import TorrentLimitDialog from "./lib/components/TorrentLimitDialog.svelte";
  import TagsDialog from "./lib/components/TagsDialog.svelte";
  import UpdateBanner from "./lib/components/UpdateBanner.svelte";
  import GuardBanner from "./lib/components/GuardBanner.svelte";
  import Palette from "./lib/components/Palette.svelte";
  import RatioDialog from "./lib/components/RatioDialog.svelte";
  import SearchDialog from "./lib/components/SearchDialog.svelte";
  import DetailPanel from "./lib/components/DetailPanel.svelte";
  import { torrents } from "./lib/stores/torrents.svelte";
  import { ui, errorText } from "./lib/stores/ui.svelte";
  import { applyPack } from "./lib/themepack";

  let topbar = $state<ReturnType<typeof TopBar>>();
  let dragging = $state(false);
  let dragDepth = 0;

  function openSources(p: OpenPayload) {
    if (p.magnets.length || p.files.length) ui.openAdd(p.magnets.join("\n"), p.files);
  }

  onMount(() => {
    const stops: (() => void)[] = [];
    const keep = (p: Promise<() => void>) => p.then((un) => stops.push(un)).catch((e) => ui.fail(e));
    keep(torrents.start());
    const focusSearch = () => topbar?.focusSearch();
    window.addEventListener("drift:focus-search", focusSearch);
    keep(api.onDetails((d) => (ui.details = d)));
    keep(api.onCompleted((c) => ui.toast(`${c.name} finished downloading`)));
    keep(api.onOpenSources(openSources));
    if (api.onUpdateAvailable) keep(api.onUpdateAvailable((u) => (ui.update = u)));
    if (api.onOpenSettings) keep(api.onOpenSettings(() => (ui.settingsOpen = true)));
    keep(api.onShown(() => torrents.refresh().catch(() => {})));
    ui.loadSettings().then(() => ui.refreshLimits()).catch((e) => ui.fail(e));
    api.guardStatus().then((g) => (ui.guard = g)).catch(() => {});
    keep(api.onGuard((g) => (ui.guard = g)));
    keep(api.onWatch((w) => (w.added ? ui.toast(`Added ${w.added} from the watch folder`) : ui.toast(`${w.invalid} isn't a valid torrent file, so it was set aside`, "error"))));
    keep(api.onPluginNotice((n) => ui.toast(n.text, n.error ? "error" : "info")));
    // The scheduled limit can change at any minute without a setting changing.
    const limitsTimer = setInterval(() => ui.refreshLimits(), 30_000);
    stops.push(() => clearInterval(limitsTimer));
    api.takePendingOpen().then(openSources).catch(() => {});

    // Tell the backend when we're hidden so it can poll less; catch up as soon as we're shown again.
    const onVisibility = () => {
      api.setVisible(!document.hidden).catch(() => {});
      if (!document.hidden) torrents.refresh().catch(() => {});
    };
    document.addEventListener("visibilitychange", onVisibility);
    onVisibility();
    return () => {
      stops.forEach((s) => s());
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("drift:focus-search", focusSearch);
    };
  });

  // Theme: "system" removes the attribute so the OS setting applies.
  $effect(() => {
    const theme = ui.settings?.theme ?? "system";
    const pack = ui.settings?.theme_pack ?? null;
    // A pack brings its own light or dark base and its own colours.
    if (pack) document.documentElement.dataset.theme = pack.base;
    else if (theme === "system") document.documentElement.removeAttribute("data-theme");
    else document.documentElement.dataset.theme = theme;
    applyPack(pack);
  });

  function isTyping(t: EventTarget | null) {
    const el = t as HTMLElement | null;
    return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
  }

  function onpaste(e: ClipboardEvent) {
    if (isTyping(e.target) || ui.addOpen) return;
    const text = e.clipboardData?.getData("text") ?? "";
    if (/^\s*magnet:/i.test(text)) {
      e.preventDefault();
      ui.openAdd(text.trim());
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && (e.key.toLowerCase() === "k" || (e.shiftKey && e.key.toLowerCase() === "p"))) {
      e.preventDefault();
      ui.paletteOpen = true;
    } else if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "f") {
      e.preventDefault();
      ui.searchOpen = true;
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      topbar?.focusSearch();
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
      e.preventDefault();
      ui.openAdd();
    }
  }

  const hasFiles = (e: DragEvent) => e.dataTransfer?.types.includes("Files") ?? false;

  async function ondrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    dragDepth = 0;
    const dropped = [...(e.dataTransfer?.files ?? [])].filter((f) => f.name.toLowerCase().endsWith(".torrent"));
    if (!dropped.length) return ui.toast("Only .torrent files can be dropped here.", "error");
    const files = await Promise.all(dropped.map(async (f) => ({ name: f.name, bytes: [...new Uint8Array(await f.arrayBuffer())] })));
    ui.openAdd("", files);
  }
</script>

<svelte:window
  {onpaste}
  {onkeydown}
  ondragenter={(e) => hasFiles(e) && (dragDepth++, (dragging = true))}
  ondragleave={() => --dragDepth <= 0 && (dragging = false)}
  ondragover={(e) => hasFiles(e) && e.preventDefault()}
  {ondrop}
/>

<div class="app">
  <TopBar bind:this={topbar} />
  {#if ui.guard.state === "unprotected"}<GuardBanner />{/if}
  {#if ui.update}<UpdateBanner />{/if}
  <div class="main">
    <Rail />
    <main class="content">
      <h1 class="sr-only">Drift</h1>
      <div class="listwrap"><TorrentList /></div>
      <DetailPanel />
      {#if ui.menu}<ContextMenu />{/if}
    </main>
  </div>
  <StatusBar />
</div>

{#if ui.addOpen}<AddDialog />{/if}
{#if ui.removeTarget}<RemoveDialog />{/if}
{#if ui.settingsOpen}<SettingsDialog />{/if}
{#if ui.goalTarget}<GoalDialog />{/if}
{#if ui.limitTarget}<TorrentLimitDialog />{/if}
{#if ui.tagTarget}<TagsDialog />{/if}
{#if ui.paletteOpen}<Palette />{/if}
{#if ui.ratioOpen}<RatioDialog />{/if}
{#if ui.searchOpen}<SearchDialog />{/if}
{#if ui.detailView}<ErrorDetails />{/if}
<Toasts />

{#if dragging}
  <div class="drop" aria-hidden="true"><Icon name="upload" size={28} />Drop .torrent files to add</div>
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .main {
    flex: 1;
    display: grid;
    grid-template-columns: 168px minmax(0, 1fr);
    min-height: 0;
  }
  @media (max-width: 760px) {
    .main {
      grid-template-columns: 48px minmax(0, 1fr);
    }
  }
  .content {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .listwrap {
    flex: 1;
    min-height: 120px;
    display: grid;
  }
  .drop {
    position: fixed;
    inset: 8px;
    z-index: 200;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 8px;
    border: 2px dashed var(--accent);
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--bg) 85%, transparent);
    color: var(--accent-fg);
    font-size: 15px;
    pointer-events: none;
  }
</style>
