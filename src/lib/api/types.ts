// Mirrors drift-core's model.rs / settings.rs (snake_case, as serialized by serde).

export type TorrentState = "checking" | "downloading" | "seeding" | "paused" | "error";

export interface TorrentStats {
  id: string;
  name: string;
  state: TorrentState;
  error: string | null;
  total_bytes: number;
  progress_bytes: number;
  uploaded_bytes: number;
  down_bps: number;
  up_bps: number;
  peers_live: number;
  eta_secs: number | null;
}

export type AddInput = { kind: "magnet"; value: string } | { kind: "file"; bytes: number[] };

export type Theme = "system" | "light" | "dark";

export interface Settings {
  schema: number;
  download_dir: string | null;
  theme: Theme;
}

/** Already plain-language; show `message` as is. */
export interface ApiError {
  code: string;
  message: string;
}

/** The only door between the UI and the backend. Two transports: Tauri IPC and an in-browser mock. */
export interface Api {
  list(): Promise<TorrentStats[]>;
  add(input: AddInput): Promise<string>;
  pause(id: string): Promise<void>;
  resume(id: string): Promise<void>;
  remove(id: string, deleteFiles: boolean): Promise<void>;
  getSettings(): Promise<Settings>;
  setSettings(s: Settings): Promise<Settings>;
  /** Returns an unsubscribe function. */
  onSnapshot(cb: (list: TorrentStats[]) => void): Promise<() => void>;
  pickFolder(current: string | null): Promise<string | null>;
}
