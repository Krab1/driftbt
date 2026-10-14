import type { Torrent } from "./api/types";

export interface TrackerRow {
  tracker: string;
  torrents: number;
  /** Total size of the torrents; stands in for "downloaded" (see the note in the UI). */
  size: number;
  uploaded: number;
  ratio: number;
  /** Bytes still to upload to reach the target ratio; 0 once there. */
  gap: number;
  /** Combined upload speed right now, bytes/s. */
  upBps: number;
  /** Seconds to close the gap at the current upload speed; null when idle or already there. */
  etaSecs: number | null;
}

export const NO_TRACKER = "No tracker";

/** Groups torrents by tracker host and works out how much real upload each group still needs for `target`. */
export function ratioTable(torrents: Torrent[], target: number): TrackerRow[] {
  const groups = new Map<string, Torrent[]>();
  for (const t of torrents) {
    if (t.total_bytes <= 0) continue; // still fetching metadata
    const key = t.tracker ?? NO_TRACKER;
    groups.set(key, [...(groups.get(key) ?? []), t]);
  }
  const rows = [...groups].map(([tracker, list]) => {
    const size = list.reduce((s, t) => s + t.total_bytes, 0);
    const uploaded = list.reduce((s, t) => s + t.uploaded_bytes, 0);
    const upBps = list.reduce((s, t) => s + t.up_bps, 0);
    const gap = Math.max(0, Math.ceil(size * target - uploaded));
    return {
      tracker,
      torrents: list.length,
      size,
      uploaded,
      ratio: size ? uploaded / size : 0,
      gap,
      upBps,
      etaSecs: gap > 0 && upBps > 0 ? Math.ceil(gap / upBps) : null,
    };
  });
  // Biggest gap first: that's where the next bit of seeding matters most.
  return rows.sort((a, b) => b.gap - a.gap || a.tracker.localeCompare(b.tracker));
}
