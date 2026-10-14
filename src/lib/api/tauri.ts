import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { Api, ApiError, TorrentStats } from "./types";

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

export const tauriApi: Api = {
  list: () => call("list_torrents"),
  add: (input) => call("add_torrent", { input }),
  pause: (id) => call("pause_torrent", { id }),
  resume: (id) => call("resume_torrent", { id }),
  remove: (id, deleteFiles) => call("remove_torrent", { id, deleteFiles }),
  getSettings: () => call("get_settings"),
  setSettings: (settings) => call("set_settings", { settings }),
  onSnapshot: async (cb) => {
    const un = await listen<TorrentStats[]>("snapshot", (e) => cb(e.payload));
    return un;
  },
  pickFolder: async (current) => {
    const r = await open({ directory: true, multiple: false, defaultPath: current ?? undefined });
    return typeof r === "string" ? r : null;
  },
};
