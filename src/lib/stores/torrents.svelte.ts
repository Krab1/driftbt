import { api, type TorrentStats } from "../api";
import { matches, visibleTorrents, type Filter, type SortKey } from "./view";

export type { Filter, SortKey };

class TorrentStore {
  all = $state.raw<TorrentStats[]>([]);
  filter = $state<Filter>("all");
  query = $state("");
  sortKey = $state<SortKey>("name");
  sortDir = $state<1 | -1>(1);
  selectedId = $state<string | null>(null);

  visible = $derived(visibleTorrents(this.all, this.filter, this.query, this.sortKey, this.sortDir));
  counts = $derived({
    all: this.all.length,
    downloading: this.all.filter(matches.downloading).length,
    seeding: this.all.filter(matches.seeding).length,
    paused: this.all.filter(matches.paused).length,
    error: this.all.filter(matches.error).length,
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
    return api.onSnapshot((list) => (this.all = list));
  }

  async refresh() {
    this.all = await api.list();
  }
}

export const torrents = new TorrentStore();
