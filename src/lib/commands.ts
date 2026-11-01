// The command palette's catalogue. Rebuilt every time the palette opens so it reflects the current selection.
import { api } from "./api";
import { N_, t, td } from "./i18n.svelte";
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
  ["all", N_("All")],
  ["downloading", N_("Downloading")],
  ["seeding", N_("Seeding")],
  ["paused", N_("Paused")],
  ["error", N_("Errors")],
];

const sorts: [SortKey, string][] = [
  ["queue", N_("Queue order")],
  ["name", N_("Name")],
  ["progress", N_("Progress")],
  ["size", N_("Size")],
  ["speed", N_("Speed")],
  ["eta", N_("Time left")],
];

/** Runs an action, refreshes the list, and reports a plain-language error as a toast. */
async function act(fn: () => Promise<unknown>) {
  try {
    await fn();
    await torrents.refresh();
  } catch (e) {
    ui.fail(e);
  }
}

export function buildCommands(): Command[] {
  const c: Command[] = [];
  const add = (group: string, id: string, title: string, run: Command["run"], shortcut?: string) => c.push({ id, title, group, run, shortcut });

  const sel = torrents.selected;
  if (sel) {
    const g = t("Selected torrent");
    const player = ui.settings?.media_player !== false;
    if (player) add(g, "play", t("Play"), () => act(() => api.play(sel.id)));
    if (sel.state === "paused") add(g, "resume", t("Resume"), () => act(() => api.resume(sel.id)), "Space");
    else if (sel.state !== "error" && sel.state !== "checking") add(g, "pause", t("Pause"), () => act(() => api.pause(sel.id)), "Space");
    if (sel.queue_pos !== null) {
      add(g, "q-top", t("Move to top of queue"), () => act(() => api.moveInQueue(sel.id, "top")));
      add(g, "q-bottom", t("Move to bottom of queue"), () => act(() => api.moveInQueue(sel.id, "bottom")));
    }
    add(g, "folder", t("Show in folder"), () => act(() => api.revealTorrent(sel.id)));
    if (player) add(g, "playlist", t("Create playlist"), () => act(async () => ui.toast(t("Playlist saved: {path}", { path: await api.createPlaylist(sel.id) }))));
    add(g, "goal", t("Set seeding goal…"), () => void (ui.goalTarget = { id: sel.id, name: sel.name }));
    add(g, "remove", t("Remove torrent…"), () => void (ui.removeTarget = { id: sel.id, name: sel.name }), "Del");
  }

  const g = t("General");
  add(g, "add", t("Add torrent…"), () => ui.openAdd(), "Ctrl+N");
  add(g, "search", t("Search torrents"), () => void window.dispatchEvent(new CustomEvent("drift:focus-search")), "Ctrl+F");
  add(g, "settings", t("Open settings"), () => void (ui.settingsOpen = true));
  add(g, "find-online", t("Find torrents online…"), () => void (ui.searchOpen = true), "Ctrl+Shift+F");
  add(g, "ratio", t("Show ratios per tracker"), () => void (ui.ratioOpen = true));
  add(g, "details", ui.detailOpen ? t("Hide details panel") : t("Show details panel"), () => ui.setDetailOpen(!ui.detailOpen));
  add(g, "pause-all", t("Pause all torrents"), () =>
    act(async () => {
      for (const t of torrents.all) if (t.state === "downloading" || t.state === "seeding" || t.state === "queued") await api.pause(t.id);
    }),
  );
  add(g, "resume-all", t("Resume all torrents"), () =>
    act(async () => {
      for (const t of torrents.all) if (t.state === "paused") await api.resume(t.id);
    }),
  );
  const s = ui.settings;
  if (s) {
    add(g, "limit", s.limits_enabled ? t("Turn speed limit off") : t("Turn speed limit on"), () => ui.saveSettings({ limits_enabled: !s.limits_enabled }));
    for (const [theme, label] of [["system", N_("System")], ["light", N_("Light")], ["dark", N_("Dark")]] as const) {
      add(t("Appearance"), `theme-${theme}`, t("Theme: {label}", { label: td(label) }), () => ui.saveSettings({ theme }));
    }
  }

  for (const [key, label] of filters) add(t("View"), `show-${key}`, t("Show: {label}", { label: td(label) }), () => void (torrents.filter = key));
  for (const cat of s?.categories ?? []) add(t("View"), `cat-${cat.name}`, t("Show category: {name}", { name: cat.name }), () => void (torrents.category = cat.name));
  if (torrents.category) add(t("View"), "cat-all", t("Show all categories"), () => void (torrents.category = null));
  for (const [key, label] of sorts) {
    add(t("View"), `sort-${key}`, t("Sort by: {label}", { label: td(label) }), () => {
      torrents.sortKey = key;
      torrents.sortDir = 1;
    });
  }
  return c;
}
