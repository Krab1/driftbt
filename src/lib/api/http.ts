// Transport for the remote web UI: the same Api over HTTP + server-sent events, talking to drift-server.
import type { Api, ApiError, Completed, GuardStatus, Torrent, TorrentDetails } from "./types";

const KEY = "drift.token";

export function storedToken(): string | null {
  try {
    return localStorage.getItem(KEY) ?? sessionStorage.getItem(KEY);
  } catch {
    return null;
  }
}

/** `remember` keeps it across browser restarts; otherwise it lasts for this tab. */
export function saveToken(token: string, remember: boolean) {
  forgetToken();
  try {
    (remember ? localStorage : sessionStorage).setItem(KEY, token);
  } catch {
    /* private mode: the token just lives in memory below */
  }
  memory = token;
}

export function forgetToken() {
  memory = null;
  try {
    localStorage.removeItem(KEY);
    sessionStorage.removeItem(KEY);
  } catch {
    /* nothing stored */
  }
}

let memory: string | null = null;
const token = () => memory ?? storedToken();

let onUnauthorized: () => void = () => {};
export function setUnauthorizedHandler(fn: () => void) {
  onUnauthorized = fn;
}

const unavailable = (what: string): ApiError => ({ code: "unavailable", message: `${what} isn't available from a remote browser.` });

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`/api${path}`, {
      method,
      headers: { ...(body === undefined ? {} : { "Content-Type": "application/json" }), Authorization: `Bearer ${token() ?? ""}` },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw { code: "offline", message: "Can't reach the server. Check that it is running and your connection is up." } satisfies ApiError;
  }
  if (res.status === 401) {
    onUnauthorized();
    throw { code: "unauthorized", message: "Enter the access token for this server." } satisfies ApiError;
  }
  const text = await res.text();
  const data = text ? JSON.parse(text) : undefined;
  if (!res.ok) throw (data && typeof data.message === "string" ? data : { code: "unknown", message: "Something went wrong. Try again." }) as ApiError;
  return data as T;
}

/** True when this origin is a drift-server. */
export async function probe(): Promise<boolean> {
  try {
    const res = await fetch("/api/health", { signal: AbortSignal.timeout(1500) });
    return res.ok && (await res.json()).name === "drift";
  } catch {
    return false;
  }
}

/** Checks the stored token (or a candidate) against the server. */
export async function checkToken(candidate?: string): Promise<boolean> {
  try {
    const res = await fetch("/api/limits", { headers: { Authorization: `Bearer ${candidate ?? token() ?? ""}` } });
    return res.ok;
  } catch {
    return false;
  }
}

// --- events: one shared stream, reconnecting; handlers are refcounted ---------------------------------------------

type Handler = (data: unknown) => void;
const handlers = new Map<string, Set<Handler>>();
let stop: (() => void) | null = null;

function ensureStream() {
  if (stop) return;
  const ctrl = new AbortController();
  stop = () => ctrl.abort();
  void (async () => {
    let wait = 1000;
    while (!ctrl.signal.aborted) {
      try {
        const res = await fetch("/api/events", { headers: { Authorization: `Bearer ${token() ?? ""}` }, signal: ctrl.signal });
        if (res.status === 401) return onUnauthorized();
        if (!res.ok || !res.body) throw new Error("stream");
        wait = 1000;
        const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
        let buf = "";
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buf += value.replace(/\r\n/g, "\n");
          let i: number;
          while ((i = buf.indexOf("\n\n")) >= 0) {
            const block = buf.slice(0, i);
            buf = buf.slice(i + 2);
            let name = "message";
            let data = "";
            for (const line of block.split("\n")) {
              if (line.startsWith("event:")) name = line.slice(6).trim();
              else if (line.startsWith("data:")) data += line.slice(5).trimStart();
            }
            if (!data) continue;
            try {
              const parsed = JSON.parse(data);
              handlers.get(name)?.forEach((h) => h(parsed));
            } catch {
              /* ignore a malformed event */
            }
          }
        }
      } catch {
        if (ctrl.signal.aborted) return;
      }
      await new Promise((r) => setTimeout(r, wait));
      wait = Math.min(wait * 2, 15_000);
    }
  })();
}

function on<T>(event: string) {
  return async (cb: (p: T) => void) => {
    const set = handlers.get(event) ?? new Set<Handler>();
    handlers.set(event, set);
    set.add(cb as Handler);
    ensureStream();
    return () => {
      set.delete(cb as Handler);
      if ([...handlers.values()].every((s) => s.size === 0)) {
        stop?.();
        stop = null;
      }
    };
  };
}

// Details have no push channel over HTTP: poll the one torrent whose panel is open.
let watched: string | null = null;
const detailSubs = new Set<(d: TorrentDetails) => void>();
let detailTimer: ReturnType<typeof setInterval> | null = null;

function syncDetailTimer() {
  if (detailSubs.size > 0 && detailTimer === null) {
    detailTimer = setInterval(async () => {
      const id = watched;
      if (!id || document.hidden) return;
      try {
        const d = await call<TorrentDetails>("GET", `/torrents/${encodeURIComponent(id)}/details`);
        if (watched === id) detailSubs.forEach((cb) => cb(d));
      } catch {
        /* the next tick tries again */
      }
    }, 1000);
  } else if (detailSubs.size === 0 && detailTimer !== null) {
    clearInterval(detailTimer);
    detailTimer = null;
  }
}

const enc = encodeURIComponent;

export const httpApi: Api = {
  list: () => call("GET", "/torrents"),
  inspect: (input) => call("POST", "/inspect", { input }),
  add: (input, category = null) => call("POST", "/torrents", { input, category }),
  setCategory: (id, category) => call("PUT", `/torrents/${enc(id)}/category`, { category }),
  setTags: (id, tags) => call("PUT", `/torrents/${enc(id)}/tags`, { tags }),
  setGoal: (id, goal) => call("PUT", `/torrents/${enc(id)}/goal`, { goal }),
  setTorrentLimits: (id, downKib, upKib) => call("PUT", `/torrents/${enc(id)}/limits`, { down_kib: downKib, up_kib: upKib }),
  moveInQueue: (id, to) => call("PUT", `/torrents/${enc(id)}/move`, { to }),
  pause: (id) => call("POST", `/torrents/${enc(id)}/pause`),
  resume: (id) => call("POST", `/torrents/${enc(id)}/resume`),
  recheck: (id) => call("POST", `/torrents/${enc(id)}/recheck`),
  moveTorrent: (id, dest) => call("POST", `/torrents/${enc(id)}/move`, { dest }),
  remove: (id, deleteFiles) => call("DELETE", `/torrents/${enc(id)}?delete_files=${deleteFiles}`),
  setFiles: (id, selected, deleteUnchecked) => call("PUT", `/torrents/${enc(id)}/files`, { selected, delete_unchecked: deleteUnchecked ?? false }),
  watchDetails: async (id) => {
    watched = id;
    return id ? call<TorrentDetails>("GET", `/torrents/${enc(id)}/details`) : null;
  },
  onDetails: async (cb) => {
    detailSubs.add(cb);
    syncDetailTimer();
    return () => {
      detailSubs.delete(cb);
      syncDetailTimer();
    };
  },
  onCompleted: on<Completed>("completed"),
  onOpenSources: async () => () => {},
  onShown: async () => () => {},
  takePendingOpen: async () => ({ magnets: [], files: [] }),
  setVisible: async () => {},
  revealTorrent: async () => Promise.reject(unavailable("Showing a file in its folder")),
  listAdapters: () => call("GET", "/adapters"),
  networkStatus: () => call("GET", "/network"),
  // The server can't open a dialog on your screen, so paths are typed in. They refer to the server's disk.
  pickFile: async (current) => prompt("Path of the file on the server", current ?? "")?.trim() || null,
  limitsStatus: () => call("GET", "/limits"),
  guardStatus: () => call<GuardStatus>("GET", "/guard"),
  onGuard: on<GuardStatus>("guard"),
  filterStatus: () => call("GET", "/filter"),
  updateIpLists: () => call("POST", "/filter/update"),
  onWatch: on<{ added?: string; invalid?: string }>("watch"),
  onPluginNotice: on<{ text: string; error: boolean }>("plugin_notice"),
  play: () => Promise.reject(unavailable("Playing a file")),
  createPlaylist: (id) => call("POST", `/torrents/${enc(id)}/playlist`),
  getSettings: () => call("GET", "/settings"),
  setSettings: (s) => call("PUT", "/settings", s),
  search: (text) => call("POST", "/search", { text }),
  testSearchSource: (id) => call("POST", `/search/test/${enc(id)}`),
  listPlugins: () => call("GET", "/plugins"),
  // Installing runs code on the host, so it is only offered in the desktop app, never over the remote API.
  pluginOffers: async () => Promise.reject(new Error("Installing plugins isn't available over the remote connection. Use Drift on the computer it runs on.")),
  installPlugin: async () => Promise.reject(new Error("Installing plugins isn't available over the remote connection. Use Drift on the computer it runs on.")),
  rollbackPlugin: async () => Promise.reject(new Error("Installing plugins isn't available over the remote connection. Use Drift on the computer it runs on.")),
  setPluginSettings: (id, values) => call("PUT", `/plugins/${enc(id)}/settings`, values),
  setPluginSecret: (id, key, value) => call("PUT", `/plugins/${enc(id)}/secrets/${enc(key)}`, { key: value }),
  pluginLog: (id) => call("GET", `/plugins/${enc(id)}/log`),
  onSnapshot: on<Torrent[]>("snapshot"),
  pickFolder: async (current) => prompt("Path of the folder on the server", current ?? "")?.trim() || null,
};
