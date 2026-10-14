import type { Torrent } from "../api/types";

export type Filter = "all" | "downloading" | "seeding" | "paused" | "error";
export type SortKey = "queue" | "name" | "progress" | "size" | "speed" | "eta";

export const matches: Record<Filter, (t: Torrent) => boolean> = {
  all: () => true,
  downloading: (t) => t.state === "downloading" || t.state === "checking" || t.state === "queued",
  seeding: (t) => t.state === "seeding",
  paused: (t) => t.state === "paused",
  error: (t) => t.state === "error",
};

const sortValue: Record<SortKey, (t: Torrent) => string | number> = {
  // Downloads in queue order first, then everything else.
  queue: (t) => t.queue_pos ?? Number.MAX_SAFE_INTEGER,
  name: (t) => t.name.toLowerCase(),
  progress: (t) => (t.total_bytes ? t.progress_bytes / t.total_bytes : 0),
  size: (t) => t.total_bytes,
  speed: (t) => t.down_bps + t.up_bps,
  // No ETA sorts last in ascending order.
  eta: (t) => t.eta_secs ?? Number.MAX_SAFE_INTEGER,
};

/** `category`: null shows everything; a name shows only that category. */
export function visibleTorrents(all: Torrent[], filter: Filter, query: string, key: SortKey, dir: 1 | -1, category: string | null = null): Torrent[] {
  const q = query.trim().toLowerCase();
  const rows = all.filter((t) => matches[filter](t) && (category === null || t.category === category) && (!q || t.name.toLowerCase().includes(q)));
  const val = sortValue[key];
  return rows.sort((a, b) => {
    const x = val(a);
    const y = val(b);
    return (x < y ? -1 : x > y ? 1 : a.id < b.id ? -1 : 1) * dir;
  });
}
