<script lang="ts">
  import { api } from "../api";
  import { formatBytes, formatEta, formatSpeed, percent } from "../../format";
  import { torrents } from "../stores/torrents.svelte";
  import { ui, errorText, type DetailTab } from "../stores/ui.svelte";
  import FileTree from "./FileTree.svelte";
  import Icon from "./Icon.svelte";

  const tabs: { key: DetailTab; label: string }[] = [
    { key: "overview", label: "Overview" },
    { key: "files", label: "Files" },
    { key: "peers", label: "Peers" },
    { key: "trackers", label: "Trackers" },
  ];

  const sel = $derived(torrents.selected);
  // Details are only trusted when they belong to the torrent that is selected right now.
  const d = $derived(ui.details && sel && ui.details.stats.id === sel.id ? ui.details : null);
  const stats = $derived(d?.stats ?? sel);

  // Ask the backend to push details for the selected torrent only while the panel is open.
  $effect(() => {
    const id = ui.detailOpen ? torrents.selectedId : null;
    let stale = false;
    api
      .watchDetails(id)
      .then((first) => !stale && (ui.details = first))
      .catch(() => !stale && (ui.details = null));
    return () => {
      stale = true;
    };
  });

  // File selection: show the user's change immediately, drop it once the backend reports the same set.
  let override = $state<Set<number> | null>(null);
  const fromBackend = $derived(new Set((d?.files ?? []).filter((f) => f.selected).map((f) => f.index)));
  const selected = $derived(override ?? fromBackend);
  $effect(() => {
    if (override && override.size === fromBackend.size && [...override].every((i) => fromBackend.has(i))) override = null;
  });
  $effect(() => {
    void torrents.selectedId;
    override = null;
  });

  let pending: ReturnType<typeof setTimeout> | undefined;
  function changeFiles(next: Set<number>) {
    override = next;
    const id = sel?.id;
    if (!id) return;
    clearTimeout(pending);
    pending = setTimeout(() => {
      api.setFiles(id, [...next]).catch((e) => {
        override = null;
        ui.toast(errorText(e), "error");
      });
    }, 200);
  }

  async function retry() {
    if (!sel) return;
    try {
      await api.resume(sel.id);
      await torrents.refresh();
    } catch (e) {
      ui.toast(errorText(e), "error");
    }
  }

  async function reveal() {
    if (sel) await api.revealTorrent(sel.id).catch((e) => ui.toast(errorText(e), "error"));
  }

  // Drag the top edge to resize.
  function startResize(e: PointerEvent) {
    const startY = e.clientY;
    const startH = ui.detailHeight;
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      ui.detailHeight = Math.max(140, Math.min(innerHeight * 0.65, startH + (startY - ev.clientY)));
    };
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      ui.setDetailHeight(Math.round(ui.detailHeight));
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
  }

  function onTabKey(e: KeyboardEvent) {
    const i = tabs.findIndex((t) => t.key === ui.detailTab);
    const next = e.key === "ArrowRight" ? i + 1 : e.key === "ArrowLeft" ? i - 1 : null;
    if (next === null) return;
    e.preventDefault();
    ui.setDetailTab(tabs[(next + tabs.length) % tabs.length].key);
  }

  const ratio = $derived(stats && stats.progress_bytes > 0 ? (stats.uploaded_bytes / stats.progress_bytes).toFixed(2) : "0.00");
</script>

<section class="panel" class:closed={!ui.detailOpen} style={ui.detailOpen ? `height: ${ui.detailHeight}px` : undefined} aria-label="Torrent details">
  {#if ui.detailOpen}
    <div class="grip" role="separator" aria-orientation="horizontal" aria-label="Resize details" onpointerdown={startResize}></div>
  {/if}
  <div class="bar">
    <div class="tabs" role="tablist" aria-label="Details" tabindex="-1" onkeydown={onTabKey}>
      {#each tabs as t (t.key)}
        <button role="tab" id="tab-{t.key}" aria-selected={ui.detailTab === t.key} aria-controls="pane" tabindex={ui.detailTab === t.key ? 0 : -1} class:on={ui.detailTab === t.key} onclick={() => { ui.setDetailTab(t.key); ui.setDetailOpen(true); }}>
          {t.label}{#if t.key === "peers" && d}<span class="n num">{d.peers.length}</span>{/if}
        </button>
      {/each}
    </div>
    <span class="who" title={stats?.name ?? ""}>{stats?.name ?? ""}</span>
    <button class="btn ghost icon" aria-label={ui.detailOpen ? "Hide details" : "Show details"} aria-expanded={ui.detailOpen} onclick={() => ui.setDetailOpen(!ui.detailOpen)}>
      <span class="chev" class:down={ui.detailOpen}>▴</span>
    </button>
  </div>

  {#if ui.detailOpen}
    <div class="pane" id="pane" role="tabpanel" aria-labelledby="tab-{ui.detailTab}">
      {#if !stats}
        <p class="hint">Select a torrent to see its details.</p>
      {:else if ui.detailTab === "overview"}
        {#if stats.state === "error"}
          <div class="banner" role="alert">
            <Icon name="alert" /><span>{stats.error ?? "Something went wrong with this torrent."}</span>
            <button class="btn" onclick={retry}>Retry</button>
          </div>
        {/if}
        <dl class="grid">
          <div><dt>Progress</dt><dd class="num">{percent(stats.progress_bytes, stats.total_bytes)}%</dd></div>
          <div><dt>Size</dt><dd class="num">{stats.total_bytes ? formatBytes(stats.total_bytes) : "—"}</dd></div>
          <div><dt>Downloaded</dt><dd class="num">{formatBytes(stats.progress_bytes)}</dd></div>
          <div><dt>Uploaded</dt><dd class="num">{formatBytes(stats.uploaded_bytes)}</dd></div>
          <div><dt>Ratio</dt><dd class="num">{ratio}</dd></div>
          <div><dt>Download speed</dt><dd class="num">{formatSpeed(stats.down_bps)}</dd></div>
          <div><dt>Upload speed</dt><dd class="num">{formatSpeed(stats.up_bps)}</dd></div>
          <div><dt>Peers</dt><dd class="num">{stats.peers_live}</dd></div>
          <div><dt>Time left</dt><dd class="num">{stats.state === "downloading" ? formatEta(stats.eta_secs) : "—"}</dd></div>
        </dl>
        <div class="path">
          <span class="lbl">Saved in</span>
          <code title={d?.save_path ?? ""}>{d?.save_path ?? "…"}</code>
          <button class="btn" onclick={reveal}><Icon name="folder" />Show in folder</button>
        </div>
      {:else if ui.detailTab === "files"}
        {#if !d}
          <p class="hint">Loading…</p>
        {:else if d.files.length === 0}
          <p class="hint">The file list appears once the torrent info has been fetched.</p>
        {:else}
          <FileTree files={d.files} {selected} onchange={changeFiles} showProgress />
        {/if}
      {:else if ui.detailTab === "peers"}
        {#if !d || d.peers.length === 0}
          <p class="hint">{stats.state === "paused" ? "Resume the torrent to connect to peers." : "No peers connected yet."}</p>
        {:else}
          <table>
            <thead><tr><th>Address</th><th>Client</th><th>Via</th><th class="r">Downloaded</th><th class="r">Uploaded</th></tr></thead>
            <tbody>
              {#each d.peers as p (p.addr)}
                <tr><td class="mono">{p.addr}</td><td>{p.client ?? "—"}</td><td>{p.transport.toUpperCase() || "—"}</td><td class="r num">{formatBytes(p.downloaded_bytes)}</td><td class="r num">{formatBytes(p.uploaded_bytes)}</td></tr>
              {/each}
            </tbody>
          </table>
        {/if}
      {:else}
        {#if !d || d.trackers.length === 0}
          <p class="hint">This torrent has no trackers; it finds peers through DHT.</p>
        {:else}
          <ul class="trackers">{#each d.trackers as t (t)}<li class="mono">{t}</li>{/each}</ul>
        {/if}
      {/if}
    </div>
  {/if}
</section>

<style>
  .panel {
    position: relative;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border);
    background: var(--bg);
    min-height: 0;
  }
  .grip {
    position: absolute;
    inset: -3px 0 auto 0;
    height: 6px;
    cursor: ns-resize;
    z-index: 2;
  }
  .grip:hover {
    background: var(--selected);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 34px;
    padding: 0 8px 0 4px;
    flex: none;
  }
  .tabs {
    display: flex;
  }
  .tabs button {
    height: 34px;
    padding: 0 12px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--text-secondary);
  }
  .tabs button:hover {
    color: var(--text-primary);
  }
  .tabs button.on {
    color: var(--text-primary);
    border-bottom-color: var(--accent);
  }
  .n {
    margin-left: 6px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .who {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-muted);
    font-size: 12px;
  }
  .chev {
    display: inline-block;
    transition: transform var(--dur);
  }
  .chev.down {
    transform: rotate(180deg);
  }
  .pane {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 4px 12px 12px;
    border-top: 1px solid var(--border);
  }
  .hint {
    margin: 12px 0;
    color: var(--text-muted);
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 8px 0;
    padding: 8px 12px;
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--danger) 14%, transparent);
    color: var(--text-primary);
  }
  .banner :global(svg) {
    color: var(--danger);
    flex: none;
  }
  .banner span {
    flex: 1;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 10px 16px;
    margin: 8px 0 12px;
  }
  dt {
    font-size: 11px;
    color: var(--text-muted);
  }
  dd {
    margin: 0;
    font-size: 14px;
  }
  .path {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .lbl {
    color: var(--text-muted);
    font-size: 11px;
  }
  code,
  .mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  code {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 4px 8px;
    border-radius: var(--radius);
    background: var(--surface-1);
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--bg);
    text-align: left;
    font-weight: 400;
    font-size: 11px;
    color: var(--text-muted);
    padding: 6px 8px 6px 0;
  }
  td {
    padding: 4px 8px 4px 0;
    border-top: 1px solid var(--border);
    white-space: nowrap;
  }
  .r {
    text-align: right;
  }
  .trackers {
    margin: 8px 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 4px;
  }
  .trackers li {
    padding: 4px 8px;
    border-radius: var(--radius);
    background: var(--surface-1);
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
