import { formatBytes, formatEta, formatSpeed, formatWhen, percent } from "../../format";
import type { Torrent } from "../api/types";
import type { SortKey } from "./view";

export interface Column {
  key: Exclude<SortKey, "name" | "queue">;
  label: string;
  width: string;
  /** Text shown in the row. */
  text: (t: Torrent) => string;
}

const ratio = (t: Torrent) => (t.total_bytes ? t.uploaded_bytes / t.total_bytes : 0);

/** Every column the list can show besides the name, in display order. */
export const COLUMNS: Column[] = [
  { key: "progress", label: "Done", width: "56px", text: (t) => (t.state === "error" ? "" : `${percent(t.progress_bytes, t.total_bytes)}%`) },
  { key: "size", label: "Size", width: "76px", text: (t) => (t.total_bytes ? formatBytes(t.total_bytes) : "—") },
  { key: "speed", label: "Speed", width: "84px", text: (t) => formatSpeed(t.state === "seeding" ? t.up_bps : t.down_bps) },
  { key: "eta", label: "ETA", width: "64px", text: (t) => (t.state === "downloading" ? formatEta(t.eta_secs) : "—") },
  { key: "added", label: "Added", width: "128px", text: (t) => formatWhen(t.added_at) },
  { key: "finished", label: "Finished", width: "128px", text: (t) => formatWhen(t.finished_at) },
  { key: "uploaded", label: "Uploaded", width: "76px", text: (t) => formatBytes(t.uploaded_bytes) },
  { key: "ratio", label: "Ratio", width: "56px", text: (t) => ratio(t).toFixed(2) },
  { key: "peers", label: "Peers", width: "52px", text: (t) => String(t.peers_live) },
  { key: "category", label: "Category", width: "96px", text: (t) => t.category ?? "—" },
  { key: "tags", label: "Tags", width: "120px", text: (t) => (t.tags.length ? t.tags.join(", ") : "—") },
  { key: "tracker", label: "Tracker", width: "132px", text: (t) => t.tracker ?? "—" },
];

const DEFAULT: Column["key"][] = ["progress", "size", "speed", "eta"];
const KEY = "drift.columns";

function load(): Column["key"][] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (Array.isArray(v)) return COLUMNS.filter((c) => v.includes(c.key)).map((c) => c.key);
  } catch {
    /* unreadable or blocked storage: use the defaults */
  }
  return DEFAULT;
}

class ColumnStore {
  on = $state<Column["key"][]>(load());
  /** Visible columns in display order. */
  shown = $derived(COLUMNS.filter((c) => this.on.includes(c.key)));
  /** Grid template: name, the chosen columns, then the action buttons. */
  template = $derived(["minmax(0, 1fr)", ...this.shown.map((c) => c.width), "92px"].join(" "));

  set(key: Column["key"], visible: boolean) {
    this.on = visible ? [...this.on, key] : this.on.filter((k) => k !== key);
    try {
      localStorage.setItem(KEY, JSON.stringify(this.on));
    } catch {
      /* not remembered */
    }
  }

  reset() {
    this.on = DEFAULT;
    try {
      localStorage.removeItem(KEY);
    } catch {
      /* nothing stored anyway */
    }
  }
}

export const columns = new ColumnStore();


