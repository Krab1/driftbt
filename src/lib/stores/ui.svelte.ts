import { api, type GuardStatus, type LimitsStatus, type Settings, type TorrentDetails } from "../api";

function remembered<T extends string | number | boolean>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return fallback;
    if (typeof fallback === "number") return (Number(raw) || fallback) as T;
    if (typeof fallback === "boolean") return (raw === "1") as T;
    return raw as T;
  } catch {
    return fallback;
  }
}

function remember(key: string, value: string | number | boolean) {
  try {
    localStorage.setItem(key, typeof value === "boolean" ? (value ? "1" : "0") : String(value));
  } catch {
    /* private mode: just don't remember */
  }
}

export type DetailTab = "overview" | "files" | "peers" | "trackers";

export interface Toast {
  id: number;
  text: string;
  kind: "info" | "error";
  /** Technical text shown by the Details button. */
  detail?: string;
}

export interface DraftFile {
  name: string;
  bytes: number[];
}

let nextToast = 1;

/** App-level UI state that is not torrent data: dialogs, menus, toasts, settings. */
class Ui {
  toasts = $state<Toast[]>([]);
  settings = $state<Settings | null>(null);

  addOpen = $state(false);
  draftMagnets = $state("");
  draftFiles = $state<DraftFile[]>([]);

  removeTarget = $state<{ id: string; name: string } | null>(null);
  settingsOpen = $state(false);
  /** A tab the settings dialog should open on (consumed when it opens). */
  settingsTab = $state<string | null>(null);
  openSettings(tab?: string) {
    this.settingsTab = tab ?? null;
    this.settingsOpen = true;
  }
  goalTarget = $state<{ id: string; name: string } | null>(null);
  limitTarget = $state<{ id: string; name: string } | null>(null);
  tagTarget = $state<{ id: string; name: string } | null>(null);
  /** The per-torrent settings of a simulator plugin (asked when it is switched on, or from its menu entry). */
  simTarget = $state<{ plugin: string; pluginName: string; id: string; name: string; enabled: boolean; fields: import("../api").PluginField[] } | null>(null);
  simStatsTarget = $state<{ plugin: string; pluginName: string; id: string; name: string } | null>(null);
  /** Torrents a simulator plugin is simulating, by info hash, with the plugin's own short status (for the list badge). */
  simulated = $state<Record<string, string>>({});
  paletteOpen = $state(false);
  ratioOpen = $state(false);
  searchOpen = $state(false);
  /** A newer version waiting for the user's go-ahead. */
  update = $state<import("../api").UpdateInfo | null>(null);
  menu = $state<{ x: number; y: number; id: string } | null>(null);

  // Detail panel (remembered per viewer, not part of app settings).
  detailOpen = $state(remembered<boolean>("drift.detailOpen", true));
  detailHeight = $state(remembered<number>("drift.detailHeight", 260));
  detailTab = $state<DetailTab>(remembered<string>("drift.detailTab", "overview") as DetailTab);
  details = $state.raw<TorrentDetails | null>(null);

  setDetailOpen(open: boolean) {
    this.detailOpen = open;
    remember("drift.detailOpen", open);
  }
  setDetailHeight(px: number) {
    this.detailHeight = px;
    remember("drift.detailHeight", px);
  }
  setDetailTab(tab: DetailTab) {
    this.detailTab = tab;
    remember("drift.detailTab", tab);
  }

  toast(text: string, kind: Toast["kind"] = "info", detail?: string) {
    const id = nextToast++;
    this.toasts = [...this.toasts, { id, text, kind, detail }];
    // An error with details stays long enough to reach for the button.
    setTimeout(() => (this.toasts = this.toasts.filter((t) => t.id !== id)), kind === "error" ? (detail ? 15000 : 6000) : 3500);
  }

  /** Shows a failed call as an error toast, with a Details button when the backend gave technical text. */
  fail(e: unknown) {
    this.toast(errorText(e), "error", errorDetail(e));
  }

  /** The technical text dialog. */
  detailView = $state<{ title: string; text: string } | null>(null);

  openAdd(magnets = "", files: DraftFile[] = []) {
    this.draftMagnets = magnets;
    this.draftFiles = files;
    this.addOpen = true;
  }

  async loadSettings() {
    this.settings = await api.getSettings();
  }

  /** Bumped when the backend refuses a settings change, so inputs remount showing the values actually in use. */
  settingsRev = $state(0);

  /** Resolves to true when the change was accepted. */
  async saveSettings(patch: Partial<Settings>): Promise<boolean> {
    if (!this.settings) return false;
    try {
      this.settings = await api.setSettings({ ...this.settings, ...patch });
      void this.refreshLimits();
      return true;
    } catch (e) {
      this.toast((e as { message?: string }).message ?? "Couldn't save your settings.", "error");
      try {
        this.settings = await api.getSettings();
      } catch {
        /* keep what we have */
      }
      this.settingsRev++;
      return false;
    }
  }

  guard = $state<GuardStatus>({ state: "off", paused: 0 });
  limits = $state<LimitsStatus>({ scheduled: false, down_bps: null, up_bps: null });

  /** Updates the list badges from what the simulator plugins said (also used by the torrent menu, which asks them anyway). */
  applySimulated(states: import("../api").SimulatorState[]) {
    const m: Record<string, string> = {};
    for (const s of states) for (const sel of s.selections) if (sel.enabled) m[sel.info_hash] = sel.label ?? `${s.name} is simulating this torrent`;
    if (JSON.stringify(m) !== JSON.stringify(this.simulated)) this.simulated = m;
  }

  async refreshSimulated() {
    try {
      this.applySimulated(await api.simulatorStates());
    } catch {
      /* the badges keep their last state */
    }
  }

  async refreshLimits() {
    try {
      this.limits = await api.limitsStatus();
    } catch {
      /* the status bar just keeps its last value */
    }
  }
}

export const ui = new Ui();

export function errorDetail(e: unknown): string | undefined {
  const d = (e as { detail?: unknown })?.detail;
  return typeof d === "string" && d.trim() ? d : undefined;
}

export function errorText(e: unknown): string {
  const m = (e as { message?: unknown })?.message;
  return typeof m === "string" ? m : "Something went wrong. Try again.";
}
