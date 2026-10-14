<script lang="ts">
  import { api, type AddInput, type TorrentPreview } from "../api";
  import { formatBytes } from "../../format";
  import { torrents } from "../stores/torrents.svelte";
  import { ui, errorText, type DraftFile } from "../stores/ui.svelte";
  import Dialog from "./Dialog.svelte";
  import FileTree from "./FileTree.svelte";
  import Icon from "./Icon.svelte";

  let magnets = $state(ui.draftMagnets);
  let files = $state<DraftFile[]>([...ui.draftFiles]);
  let error = $state("");
  let busy = $state(false);
  let picker = $state<HTMLInputElement>();

  const magnetLines = $derived(magnets.split(/\r?\n/).map((l) => l.trim()).filter(Boolean));
  const count = $derived(magnetLines.length + files.length);

  // Preview: only when exactly one source is present; stale answers are dropped.
  let preview = $state<TorrentPreview | null>(null);
  let previewing = $state(false);
  let previewError = $state("");
  let selected = $state(new Set<number>());

  const single = $derived<AddInput | null>(
    count !== 1 ? null : magnetLines.length ? { kind: "magnet", value: magnetLines[0] } : { kind: "file", bytes: files[0].bytes },
  );

  $effect(() => {
    const input = single;
    preview = null;
    previewError = "";
    if (!input) {
      previewing = false;
      return;
    }
    // Don't hit the network for text that obviously isn't a magnet yet.
    if (input.kind === "magnet" && !/^magnet:\?|^[0-9a-f]{40}$/i.test(input.value)) {
      previewing = false;
      return;
    }
    let stale = false;
    previewing = true;
    const t = setTimeout(() => {
      api
        .inspect(input)
        .then((p) => {
          if (stale) return;
          preview = p;
          selected = new Set(p.files.map((f) => f.index));
        })
        .catch((e) => !stale && (previewError = errorText(e)))
        .finally(() => !stale && (previewing = false));
    }, 450);
    return () => {
      stale = true;
      clearTimeout(t);
    };
  });

  const chosenBytes = $derived(preview ? preview.files.filter((f) => selected.has(f.index)).reduce((a, f) => a + f.size, 0) : 0);
  const allSelected = $derived(!!preview && selected.size === preview.files.length);

  async function pickFiles(list: FileList | null) {
    if (!list) return;
    const added = await Promise.all([...list].map(async (f) => ({ name: f.name, bytes: [...new Uint8Array(await f.arrayBuffer())] })));
    files = [...files, ...added];
    error = "";
  }

  async function changeFolder() {
    const dir = await api.pickFolder(ui.settings?.download_dir ?? null);
    if (dir) await ui.saveSettings({ download_dir: dir });
  }

  async function submit() {
    if (!count) {
      error = "Paste a magnet link or choose a .torrent file.";
      return;
    }
    if (preview && selected.size === 0) {
      error = "Select at least one file to download.";
      return;
    }
    busy = true;
    error = "";
    const only = preview && !allSelected ? [...selected] : undefined;
    let added = 0;
    const failedMagnets: string[] = [];
    const failedFiles: DraftFile[] = [];
    let firstError = "";
    for (const m of magnetLines) {
      try {
        await api.add({ kind: "magnet", value: m, only_files: only });
        added++;
      } catch (e) {
        failedMagnets.push(m);
        firstError ||= errorText(e);
      }
    }
    for (const f of files) {
      try {
        await api.add({ kind: "file", bytes: f.bytes, only_files: only });
        added++;
      } catch (e) {
        failedFiles.push(f);
        firstError ||= `${f.name}: ${errorText(e)}`;
      }
    }
    busy = false;
    if (added) {
      await torrents.refresh();
      ui.toast(added === 1 ? "Torrent added" : `${added} torrents added`);
    }
    if (firstError) {
      magnets = failedMagnets.join("\n");
      files = failedFiles;
      error = firstError;
    } else {
      ui.addOpen = false;
    }
  }
</script>

<Dialog title="Add torrent" onclose={() => (ui.addOpen = false)}>
  <label class="field">
    <span>Magnet links, one per line</span>
    <textarea rows="3" placeholder="magnet:?xt=urn:btih:…" bind:value={magnets} oninput={() => (error = "")}></textarea>
  </label>

  <div class="field">
    <span>Torrent files</span>
    <div class="files">
      {#each files as f, i (f.name + i)}
        <div class="chip"><Icon name="file" size={14} /><span class="fn">{f.name}</span>
          <button class="btn ghost icon" aria-label="Remove {f.name}" onclick={() => (files = files.filter((_, j) => j !== i))}><Icon name="x" size={14} /></button>
        </div>
      {/each}
      <button class="btn" onclick={() => picker?.click()}><Icon name="plus" />Choose files</button>
      <input bind:this={picker} type="file" accept=".torrent" multiple hidden onchange={(e) => (pickFiles(e.currentTarget.files), (e.currentTarget.value = ""))} />
    </div>
  </div>

  {#if previewing}
    <p class="note" role="status">Fetching torrent info…</p>
  {:else if previewError}
    <p class="note warn" role="status">{previewError} You can still add it.</p>
  {:else if preview}
    <div class="field">
      <div class="phead">
        <span class="pname" title={preview.name}>{preview.name}</span>
        <span class="num psize">{formatBytes(chosenBytes)} of {formatBytes(preview.total_bytes)}</span>
      </div>
      {#if preview.files.length > 1}
        <div class="treebox"><FileTree files={preview.files} {selected} onchange={(n) => (selected = n)} /></div>
      {/if}
    </div>
  {/if}

  <div class="field">
    <span>Save to</span>
    <div class="folder">
      <code title={ui.settings?.download_dir ?? ""}>{ui.settings?.download_dir ?? "…"}</code>
      <button class="btn" onclick={changeFolder}><Icon name="folder" />Change</button>
    </div>
  </div>

  {#if error}<p class="err" role="alert">{error}</p>{/if}

  {#snippet footer()}
    <button class="btn" onclick={() => (ui.addOpen = false)}>Cancel</button>
    <button class="btn primary" disabled={busy} onclick={submit}>{busy ? "Adding…" : "Add torrent"}</button>
  {/snippet}
</Dialog>

<style>
  .field {
    display: grid;
    gap: 6px;
  }
  .field > span {
    color: var(--text-secondary);
    font-size: 12px;
  }
  textarea {
    resize: vertical;
    padding: 8px;
    border-radius: var(--radius);
    border: 1px solid var(--border-strong);
    background: var(--bg);
    user-select: text;
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .files {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 2px 0 8px;
    border-radius: var(--radius);
    background: var(--surface-1);
    max-width: 100%;
  }
  .fn {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
  }
  .note {
    margin: 0;
    color: var(--text-secondary);
  }
  .note.warn {
    color: var(--warning);
  }
  .phead {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    align-items: baseline;
  }
  .pname {
    font-weight: 500;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .psize {
    flex: none;
    color: var(--text-secondary);
  }
  .treebox {
    max-height: 190px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 4px 0;
  }
  .folder {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  code {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 5px 8px;
    border-radius: var(--radius);
    background: var(--surface-1);
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .err {
    margin: 0;
    color: var(--danger-fg);
  }
</style>
