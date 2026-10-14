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
  goalTarget = $state<{ id: string; name: string } | null>(null);
  paletteOpen = $state(false);
  ratioOpen = $state(false);
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

  toast(text: string, kind: Toast["kind"] = "info") {
    const id = nextToast++;
    this.toasts = [...this.toasts, { id, text, kind }];
    setTimeout(() => (this.toasts = this.toasts.filter((t) => t.id !== id)), kind === "error" ? 6000 : 3500);
  }

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

  async refreshLimits() {
    try {
      this.limits = await api.limitsStatus();
    } catch {
      /* the status bar just keeps its last value */
    }
  }
}

export const ui = new Ui();

export function errorText(e: unknown): string {
  const m = (e as { message?: unknown })?.message;
  return typeof m === "string" ? m : "Something went wrong. Try again.";
}
