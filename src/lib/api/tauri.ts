import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { Api, ApiError, Completed, GuardStatus, OpenPayload, Torrent, TorrentDetails } from "./types";

/** Backend errors arrive already serialized as {code, message}; anything else is a bug worth a generic message. */
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    const err = e as Partial<ApiError>;
    if (err && typeof err.message === "string") throw err as ApiError;
    throw { code: "unknown", message: "Something went wrong. Try again." } satisfies ApiError;
  }
}

const on = <T>(event: string) => (cb: (p: T) => void) => listen<T>(event, (e) => cb(e.payload));

export const tauriApi: Api = {
  list: () => call("list_torrents"),
  inspect: (input) => call("inspect_torrent", { input }),
  add: (input, category = null) => call("add_torrent", { input, category }),
  setCategory: (id, category) => call("set_torrent_category", { id, category }),
  setTags: (id, tags) => call("set_torrent_tags", { id, tags }),
  setGoal: (id, goal) => call("set_torrent_goal", { id, goal }),
  moveInQueue: (id, to) => call("move_in_queue", { id, to }),
  pause: (id) => call("pause_torrent", { id }),
  resume: (id) => call("resume_torrent", { id }),
  remove: (id, deleteFiles) => call("remove_torrent", { id, deleteFiles }),
  setFiles: (id, selected) => call("set_files", { id, selected }),
  watchDetails: (id) => call<TorrentDetails | null>("watch_details", { id }),
  onDetails: on<TorrentDetails>("details"),
  onCompleted: on<Completed>("completed"),
  onOpenSources: on<OpenPayload>("open-sources"),
  onShown: on<null>("window-shown"),
  takePendingOpen: () => call("take_pending_open"),
  setVisible: (visible) => call("set_visible", { visible }),
  revealTorrent: (id) => call("reveal_torrent", { id }),
  listAdapters: () => call("list_adapters"),
  networkStatus: () => call("network_status"),
  pickFile: async (current) => {
    const r = await open({ directory: false, multiple: false, defaultPath: current ?? undefined });
    return typeof r === "string" ? r : null;
  },
  limitsStatus: () => call("limits_status"),
  guardStatus: () => call("guard_status"),
  onGuard: on<GuardStatus>("guard"),
  filterStatus: () => call("filter_status"),
  updateIpLists: () => call("update_ip_lists"),
  onWatch: on<{ added?: string; invalid?: string }>("watch"),
  play: (id, file = null) => call("play_torrent", { id, file }),
  createPlaylist: (id) => call("create_playlist", { id }),
  search: (text) => call("search_torrents", { text }),
  searchKeysSet: () => call("search_keys_set"),
  setSearchKey: (id, key) => call("set_search_key", { id, key }),
  getSettings: () => call("get_settings"),
  setSettings: (settings) => call("set_settings", { settings }),
  onSnapshot: on<Torrent[]>("snapshot"),
  pickFolder: async (current) => {
    const r = await open({ directory: true, multiple: false, defaultPath: current ?? undefined });
    return typeof r === "string" ? r : null;
  },
};
