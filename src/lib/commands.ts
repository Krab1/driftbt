// The command palette's catalogue. Rebuilt every time the palette opens so it reflects the current selection.
import { api } from "./api";
import { torrents, type Filter, type SortKey } from "./stores/torrents.svelte";
import { errorText, ui } from "./stores/ui.svelte";

export interface Command {
  id: string;
  title: string;
  group: string;
  shortcut?: string;
  run: () => unknown;
}

const filters: [Filter, string][] = [
  ["all", "All"],
  ["downloading", "Downloading"],
  ["seeding", "Seeding"],
  ["paused", "Paused"],
  ["error", "Errors"],
];

const sorts: [SortKey, string][] = [
  ["queue", "Queue order"],
  ["name", "Name"],
  ["progress", "Progress"],
  ["size", "Size"],
  ["speed", "Speed"],
  ["eta", "Time left"],
];

/** Runs an action, refreshes the list, and reports a plain-language error as a toast. */
async function act(fn: () => Promise<unknown>) {
  try {
    await fn();
    await torrents.refresh();
  } catch (e) {
    ui.toast(errorText(e), "error");
  }
}

export function buildCommands(): Command[] {
  const c: Command[] = [];
  const add = (group: string, id: string, title: string, run: Command["run"], shortcut?: string) => c.push({ id, title, group, run, shortcut });

  const sel = torrents.selected;
  if (sel) {
    const g = "Selected torrent";
    add(g, "play", "Play", () => act(() => api.play(sel.id)));
    if (sel.state === "paused") add(g, "resume", "Resume", () => act(() => api.resume(sel.id)), "Space");
    else if (sel.state !== "error" && sel.state !== "checking") add(g, "pause", "Pause", () => act(() => api.pause(sel.id)), "Space");
    if (sel.queue_pos !== null) {
      add(g, "q-top", "Move to top of queue", () => act(() => api.moveInQueue(sel.id, "top")));
      add(g, "q-bottom", "Move to bottom of queue", () => act(() => api.moveInQueue(sel.id, "bottom")));
    }
    add(g, "folder", "Show in folder", () => act(() => api.revealTorrent(sel.id)));
    add(g, "playlist", "Create playlist", () => act(async () => ui.toast(`Playlist saved: ${await api.createPlaylist(sel.id)}`)));
    add(g, "goal", "Set seeding goal…", () => void (ui.goalTarget = { id: sel.id, name: sel.name }));
    add(g, "remove", "Remove torrent…", () => void (ui.removeTarget = { id: sel.id, name: sel.name }), "Del");
  }

  const g = "General";
  add(g, "add", "Add torrent…", () => ui.openAdd(), "Ctrl+N");
  add(g, "search", "Search torrents", () => void window.dispatchEvent(new CustomEvent("drift:focus-search")), "Ctrl+F");
  add(g, "settings", "Open settings", () => void (ui.settingsOpen = true));
  add(g, "find-online", "Find torrents online…", () => void (ui.searchOpen = true), "Ctrl+Shift+F");
  add(g, "ratio", "Show ratios per tracker", () => void (ui.ratioOpen = true));
  add(g, "details", ui.detailOpen ? "Hide details panel" : "Show details panel", () => ui.setDetailOpen(!ui.detailOpen));
  add(g, "pause-all", "Pause all torrents", () =>
    act(async () => {
      for (const t of torrents.all) if (t.state === "downloading" || t.state === "seeding" || t.state === "queued") await api.pause(t.id);
    }),
  );
  add(g, "resume-all", "Resume all torrents", () =>
    act(async () => {
      for (const t of torrents.all) if (t.state === "paused") await api.resume(t.id);
    }),
  );
  const s = ui.settings;
  if (s) {
    add(g, "limit", s.limits_enabled ? "Turn speed limit off" : "Turn speed limit on", () => ui.saveSettings({ limits_enabled: !s.limits_enabled }));
    for (const [theme, label] of [["system", "System"], ["light", "Light"], ["dark", "Dark"]] as const) {
      add("Appearance", `theme-${theme}`, `Theme: ${label}`, () => ui.saveSettings({ theme }));
    }
  }

  for (const [key, label] of filters) add("View", `show-${key}`, `Show: ${label}`, () => void (torrents.filter = key));
  for (const cat of s?.categories ?? []) add("View", `cat-${cat.name}`, `Show category: ${cat.name}`, () => void (torrents.category = cat.name));
  if (torrents.category) add("View", "cat-all", "Show all categories", () => void (torrents.category = null));
  for (const [key, label] of sorts) {
    add("View", `sort-${key}`, `Sort by: ${label}`, () => {
      torrents.sortKey = key;
      torrents.sortDir = 1;
    });
  }
  return c;
}
