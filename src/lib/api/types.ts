// Mirrors drift-core's model.rs / settings.rs (snake_case, as serialized by serde).

export type TorrentState = "checking" | "downloading" | "seeding" | "paused" | "queued" | "error";

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
  /** Raw engine text behind `error`, for a "More details" button. */
  error_detail?: string | null;
  /** Host of the first tracker. */
  tracker: string | null;
}

/** A torrent as the list shows it: engine stats plus the manager's bookkeeping. */
export interface Torrent extends TorrentStats {
  category: string | null;
  tags: string[];
  /** Place in the download queue (1 = first); null once complete. */
  queue_pos: number | null;
  /** Short reason shown next to the name, e.g. "Seeding goal reached". */
  note: string | null;
  /** Unix seconds; null when unknown. */
  added_at: number | null;
  finished_at: number | null;
}

/** Stop seeding at this share ratio or after this many minutes, whichever comes first. */
export interface Goal {
  ratio: number | null;
  minutes: number | null;
}

export interface Category {
  name: string;
  save_path: string | null;
}

export type QueueMove = "top" | "up" | "down" | "bottom";

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
  /** Two-letter country code, when known. */
  country: string | null;
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
  | { kind: "url"; value: string; only_files?: number[] }
  | { kind: "file"; bytes: number[]; only_files?: number[] };

export interface SearchResult {
  title: string;
  /** Names of the sources that returned it. */
  sources: string[];
  info_hash: string | null;
  magnet: string | null;
  torrent_url: string | null;
  size: number | null;
  seeders: number | null;
  leechers: number | null;
  /** Unix seconds. */
  published: number | null;
  category: string | null;
}

export interface SourceStatus {
  id: string;
  name: string;
  outcome: "ok" | "failed" | "timed_out";
  count: number;
  /** Plain language, only when it didn't work. */
  message: string | null;
}

export interface SearchReport {
  results: SearchResult[];
  sources: SourceStatus[];
}

/** The old built-in search settings. Search sources are plugins now; Drift moves these over once at start. */
export interface SearchSettings {
  archive_org: boolean;
  torznab: { id: string; name: string; url: string; enabled: boolean }[];
}

/** One row of a list field (for example one Torznab indexer): an `id` plus the list's non-secret fields. */
export type PluginRow = Record<string, string | number | boolean>;
export type PluginValue = string | number | boolean | PluginRow[];

export interface PluginInfo {
  id: string;
  name: string;
  version: string;
  kinds: string[];
  permissions: { net: string[]; fs: string[] };
  enabled: boolean;
  /** Why it can't be used; null when it is fine. */
  problem: string | null;
  /** The settings form the plugin asks for, the current plain values, and which secret fields already hold a value. */
  fields: PluginField[];
  values: Record<string, PluginValue>;
  /** Secret fields holding a value. A list row's secret is `<list key>.<row id>.<field key>`. */
  secrets_set: string[];
  /** A version from the registry that takes over at the next start, and the version a rollback would bring back. */
  pending: string | null;
  previous: string | null;
}

/** A plugin the registry offers: its newest version that works with this Drift. */
export interface PluginOffer {
  id: string;
  name: string;
  summary: string;
  version: string;
  changelog: string;
  installed: string | null;
}

export interface PluginField {
  key: string;
  label: string;
  type: "text" | "url" | "number" | "bool" | "secret" | "list";
  help: string | null;
  /** For a list: the fields of one row. */
  item?: PluginField[];
}

export interface OpenPayload {
  magnets: string[];
  files: { name: string; bytes: number[] }[];
}

export interface Completed {
  id: string;
  name: string;
}

export interface ScheduleSlot {
  /** Bit 0 = Monday ... bit 6 = Sunday. */
  days: number;
  /** Minutes after local midnight. */
  from_min: number;
  to_min: number;
  /** KiB/s while active; 0 = unlimited. */
  down_kib: number;
  up_kib: number;
}

export interface Schedule {
  enabled: boolean;
  slots: ScheduleSlot[];
}

export interface IpList {
  /** Letters, digits, - and _ only. */
  id: string;
  name: string;
  /** An http(s) address or a file path. */
  source: string;
  enabled: boolean;
}

export interface IpFilterSettings {
  enabled: boolean;
  lists: IpList[];
}

export interface VpnGuardSettings {
  enabled: boolean;
  adapter: string | null;
  auto_resume: boolean;
}

export interface Adapter {
  name: string;
  description: string;
  up: boolean;
  has_ipv4: boolean;
}

export interface GuardStatus {
  state: "off" | "protected" | "unprotected";
  /** Torrents the guard is holding. */
  paused: number;
}

export interface ListState {
  ranges: number;
  /** Unix seconds of the last successful load. */
  updated_at: number;
  error: string | null;
  /** Fetched over plain http. */
  insecure: boolean;
}

export interface FilterStatus {
  enabled: boolean;
  active_ranges: number;
  lists: Record<string, ListState>;
}

export interface NetworkStatus {
  /** The port the engine is listening on; null when it isn't (for example behind a proxy). */
  listen_port: number | null;
}

export interface LimitsStatus {
  scheduled: boolean;
  down_bps: number | null;
  up_bps: number | null;
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
  /** Look for a new version once a day. Installing always asks first. */
  check_updates: boolean;
  /** Incomplete torrents downloading at once; 0 = no limit. */
  max_active_downloads: number;
  /** Applies to torrents without their own goal. */
  default_goal: Goal;
  /** Nothing stops seeding before this many minutes. */
  min_seed_minutes: number;
  categories: Category[];
  /** Write an .m3u8 next to finished torrents with several video/audio files. */
  playlist_on_complete: boolean;
  /** Fixed incoming port; null = random at each start. */
  listen_port: number | null;
  upnp: boolean;
  dht: boolean;
  /** socks5://[user:pass@]host:port. A proxy turns DHT and incoming connections off. */
  proxy: string | null;
  ip_filter: IpFilterSettings;
  schedule: Schedule;
  /** .torrent files dropped here are added automatically. */
  watch_dir: string | null;
  /** Finished downloads are moved here; null leaves them where they were saved. */
  move_on_complete: string | null;
  vpn_guard: VpnGuardSettings;
  search: SearchSettings;
  /** Ids of the plugins that are turned on. */
  plugins: string[];
  /** Whether Drift may contact the plugin registry (github.com). */
  plugin_registry: boolean;
}

/** Already plain-language; show `message` as is. */
export interface ApiError {
  code: string;
  message: string;
  /** The technical text behind the message, for a "More details" button. */
  detail?: string;
}

/** The frameless desktop window's own title-bar controls. Absent in the browser, where the browser draws its frame. */
export interface WindowChrome {
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  close(): Promise<void>;
  isMaximized(): Promise<boolean>;
  /** Fires when the window is maximized or restored. */
  onMaximizedChange(cb: (maximized: boolean) => void): Promise<() => void>;
  /** The pointer is over the maximize button (Windows reports it to us, the page can't see it). */
  onMaxHover(cb: (over: boolean) => void): Promise<() => void>;
  /** Where the maximize button is, in physical window pixels, so Windows can show snap layouts over it. */
  setMaxButtonRect(rect: [number, number, number, number] | null): Promise<void>;
}

/** The only door between the UI and the backend. Two transports: Tauri IPC and an in-browser mock. */
export interface UpdateInfo {
  version: string;
  notes: string | null;
}

export interface Api {
  /** Looks for a newer version now. null means you're up to date. Desktop app only. */
  checkUpdate?(): Promise<UpdateInfo | null>;
  /** Downloads and installs the version found by the last check; the app restarts. */
  installUpdate?(): Promise<void>;
  onUpdateAvailable?(cb: (u: UpdateInfo) => void): Promise<() => void>;
  /** The tray menu's Settings item (desktop only). */
  onOpenSettings?(cb: () => void): Promise<() => void>;
  /** This app's version. */
  version?(): Promise<string>;
  chrome?: WindowChrome;
  list(): Promise<Torrent[]>;
  /** Reads a torrent's files without starting it. Magnets can take a while and may time out. */
  inspect(input: AddInput): Promise<TorrentPreview>;
  add(input: AddInput, category?: string | null): Promise<string>;
  setCategory(id: string, category: string | null): Promise<void>;
  setTags(id: string, tags: string[]): Promise<void>;
  /** null goes back to the default goal. */
  setGoal(id: string, goal: Goal | null): Promise<void>;
  moveInQueue(id: string, to: QueueMove): Promise<void>;
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
  listAdapters(): Promise<Adapter[]>;
  networkStatus(): Promise<NetworkStatus>;
  /** Opens a file chooser; null when cancelled. */
  pickFile(current: string | null): Promise<string | null>;
  limitsStatus(): Promise<LimitsStatus>;
  guardStatus(): Promise<GuardStatus>;
  onGuard(cb: (s: GuardStatus) => void): Promise<() => void>;
  filterStatus(): Promise<FilterStatus>;
  /** Downloads every enabled IP list again now. */
  updateIpLists(): Promise<FilterStatus>;
  /** A .torrent file was picked up (or rejected) from the watch folder. */
  onWatch(cb: (e: { added?: string; invalid?: string }) => void): Promise<() => void>;
  /** A message from a post-processing plugin, or a note that one failed. */
  onPluginNotice(cb: (e: { text: string; error: boolean }) => void): Promise<() => void>;
  /** Plays a file in the default player while it downloads (the largest media file when `file` is omitted). */
  play(id: string, file?: number | null): Promise<void>;
  /** Writes an .m3u8 next to the files; resolves to its path. */
  createPlaylist(id: string): Promise<string>;
  pause(id: string): Promise<void>;
  resume(id: string): Promise<void>;
  /** Hashes the downloaded files again and re-downloads anything missing or damaged. */
  recheck(id: string): Promise<void>;
  /** Moves the torrent's files into this folder (on the machine running Drift) and keeps seeding from there. */
  moveTorrent(id: string, dest: string): Promise<void>;
  remove(id: string, deleteFiles: boolean): Promise<void>;
  search(text: string): Promise<SearchReport>;
  /** Ids of search sources that have an API key saved. The keys themselves never reach the UI. */
  /** Asks one source a sample question; the result says whether it answered. */
  testSearchSource(id: string): Promise<SourceStatus>;
  listPlugins(): Promise<PluginInfo[]>;
  /** Needs `plugin_registry` on. */
  pluginOffers(): Promise<PluginOffer[]>;
  /** Downloads and verifies a plugin; it takes over at the next start. */
  installPlugin(id: string, version: string): Promise<void>;
  /** Brings back the version that was replaced last time, at the next start. */
  rollbackPlugin(id: string): Promise<void>;
  setPluginSettings(id: string, values: Record<string, PluginValue>): Promise<void>;
  /** An empty value removes the secret. */
  setPluginSecret(id: string, key: string, value: string): Promise<void>;
  /** The end of the plugin's error output, with its secrets blanked. */
  pluginLog(id: string): Promise<string>;
  /** An empty key removes it. The source must already be saved in settings. */
  getSettings(): Promise<Settings>;
  setSettings(s: Settings): Promise<Settings>;
  /** Returns an unsubscribe function. */
  onSnapshot(cb: (list: Torrent[]) => void): Promise<() => void>;
  pickFolder(current: string | null): Promise<string | null>;
}
