import { api, type Torrent } from "../api";
import { matches, visibleTorrents, type Filter, type SortKey } from "./view";

export type { Filter, SortKey };

class TorrentStore {
  all = $state.raw<Torrent[]>([]);
  filter = $state<Filter>("all");
  query = $state("");
  /** null = all categories */
  category = $state<string | null>(null);
  sortKey = $state<SortKey>("name");
  sortDir = $state<1 | -1>(1);
  selectedId = $state<string | null>(null);
  /** Total speeds, one sample per snapshot, newest last (about two minutes). */
  history = $state.raw<{ d: number; u: number }[]>([]);

  visible = $derived(visibleTorrents(this.all, this.filter, this.query, this.sortKey, this.sortDir, this.category));
  counts = $derived({
    all: this.all.length,
    downloading: this.all.filter(matches.downloading).length,
    seeding: this.all.filter(matches.seeding).length,
    paused: this.all.filter(matches.paused).length,
    error: this.all.filter(matches.error).length,
  });
  categoryCounts = $derived.by(() => {
    const m = new Map<string, number>();
    for (const t of this.all) if (t.category) m.set(t.category, (m.get(t.category) ?? 0) + 1);
    return m;
  });
  totals = $derived({
    down: this.all.reduce((s, t) => s + t.down_bps, 0),
    up: this.all.reduce((s, t) => s + t.up_bps, 0),
  });
  selected = $derived(this.all.find((t) => t.id === this.selectedId) ?? null);

  sortBy(key: SortKey) {
    if (this.sortKey === key) this.sortDir = this.sortDir === 1 ? -1 : 1;
    else {
      this.sortKey = key;
      this.sortDir = 1;
    }
  }

  /** Pulls the list once and then follows pushed snapshots. Returns the unsubscribe function. */
  async start(): Promise<() => void> {
    this.all = await api.list();
    return api.onSnapshot((list) => {
      this.all = list;
      const d = list.reduce((s, t) => s + t.down_bps, 0);
      const u = list.reduce((s, t) => s + t.up_bps, 0);
      this.history = [...this.history.slice(-119), { d, u }];
    });
  }

  async refresh() {
    this.all = await api.list();
  }
}

export const torrents = new TorrentStore();
