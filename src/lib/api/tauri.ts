import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { Api, ApiError, Completed, OpenPayload, TorrentDetails, TorrentStats } from "./types";

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
  add: (input) => call("add_torrent", { input }),
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
  getSettings: () => call("get_settings"),
  setSettings: (settings) => call("set_settings", { settings }),
  onSnapshot: on<TorrentStats[]>("snapshot"),
  pickFolder: async (current) => {
    const r = await open({ directory: true, multiple: false, defaultPath: current ?? undefined });
    return typeof r === "string" ? r : null;
  },
};
