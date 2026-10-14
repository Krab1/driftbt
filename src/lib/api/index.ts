import type { Api } from "./types";

export * from "./types";

/** Inside Tauri use IPC; served by drift-server use HTTP; in a plain browser (dev preview, tests) use the mock. `?n=1000` sets mock size. */
async function pick(): Promise<{ api: Api; remote: boolean }> {
  if ("__TAURI_INTERNALS__" in window) return { api: (await import("./tauri")).tauriApi, remote: false };
  const http = await import("./http");
  if (await http.probe()) return { api: http.httpApi, remote: true };
  const n = Number(new URLSearchParams(location.search).get("n") ?? 40);
  return { api: (await import("./mock")).createMockApi(Number.isFinite(n) ? n : 40), remote: false };
}

const picked = await pick();
export const api: Api = picked.api;
/** True when talking to a drift-server, which needs a token. */
export const remote: boolean = picked.remote;
