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

export interface FileDetails {
  index: number;
  path: string;
  size: number;
  progress_bytes: number;
  selected: boolean;
}

export interface PeerInfo {
  addr: string;
  client: string | null;
  transport: string;
  downloaded_bytes: number;
  uploaded_bytes: number;
}

export interface TorrentDetails {
  stats: TorrentStats;
  files: FileDetails[];
  peers: PeerInfo[];
  trackers: string[];
  save_path: string;
}

export interface PreviewFile {
  index: number;
  path: string;
  size: number;
}

/** What a torrent contains, read without starting it. */
export interface TorrentPreview {
  id: string;
  name: string;
  total_bytes: number;
  files: PreviewFile[];
}

/** `only_files` limits the download to those file indices; omit for everything. */
export type AddInput =
  | { kind: "magnet"; value: string; only_files?: number[] }
  | { kind: "file"; bytes: number[]; only_files?: number[] };

export interface OpenPayload {
  magnets: string[];
  files: { name: string; bytes: number[] }[];
}

export interface Completed {
  id: string;
  name: string;
}

export type Theme = "system" | "light" | "dark";

export interface Settings {
  schema: number;
  download_dir: string | null;
  theme: Theme;
  limits_enabled: boolean;
  /** KiB/s; 0 = unlimited. */
  down_limit_kib: number;
  up_limit_kib: number;
  close_to_tray: boolean;
  start_minimized: boolean;
  start_with_windows: boolean;
  notify_on_complete: boolean;
}

/** Already plain-language; show `message` as is. */
export interface ApiError {
  code: string;
  message: string;
}

/** The only door between the UI and the backend. Two transports: Tauri IPC and an in-browser mock. */
export interface Api {
  list(): Promise<TorrentStats[]>;
  /** Reads a torrent's files without starting it. Magnets can take a while and may time out. */
  inspect(input: AddInput): Promise<TorrentPreview>;
  add(input: AddInput): Promise<string>;
  setFiles(id: string, selected: number[]): Promise<void>;
  /** Choose which torrent's details are pushed (or null). Returns its current details at once. */
  watchDetails(id: string | null): Promise<TorrentDetails | null>;
  onDetails(cb: (d: TorrentDetails) => void): Promise<() => void>;
  onCompleted(cb: (c: Completed) => void): Promise<() => void>;
  /** Magnets/files the OS opened the app with (second launch or file association). */
  onOpenSources(cb: (p: OpenPayload) => void): Promise<() => void>;
  /** The window was brought back (e.g. from the tray); refresh because updates were slowed while hidden. */
  onShown(cb: () => void): Promise<() => void>;
  takePendingOpen(): Promise<OpenPayload>;
  /** Tell the backend whether the window is visible so it can slow down when hidden. */
  setVisible(visible: boolean): Promise<void>;
  revealTorrent(id: string): Promise<void>;
  pause(id: string): Promise<void>;
  resume(id: string): Promise<void>;
  remove(id: string, deleteFiles: boolean): Promise<void>;
  getSettings(): Promise<Settings>;
  setSettings(s: Settings): Promise<Settings>;
  /** Returns an unsubscribe function. */
  onSnapshot(cb: (list: TorrentStats[]) => void): Promise<() => void>;
  pickFolder(current: string | null): Promise<string | null>;
}
