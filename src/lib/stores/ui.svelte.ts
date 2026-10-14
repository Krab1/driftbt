import { api, type Settings } from "../api";

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
  menu = $state<{ x: number; y: number; id: string } | null>(null);

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

  async saveSettings(patch: Partial<Settings>) {
    if (!this.settings) return;
    try {
      this.settings = await api.setSettings({ ...this.settings, ...patch });
    } catch (e) {
      this.toast((e as { message?: string }).message ?? "Couldn't save your settings.", "error");
    }
  }
}

export const ui = new Ui();

export function errorText(e: unknown): string {
  const m = (e as { message?: unknown })?.message;
  return typeof m === "string" ? m : "Something went wrong. Try again.";
}
