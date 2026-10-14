import type { Api } from "./types";

export * from "./types";

/** Inside Tauri use IPC; in a plain browser (dev preview, tests) use the mock. `?n=1000` sets mock size. */
async function pick(): Promise<Api> {
  if ("__TAURI_INTERNALS__" in window) return (await import("./tauri")).tauriApi;
  const n = Number(new URLSearchParams(location.search).get("n") ?? 40);
  return (await import("./mock")).createMockApi(Number.isFinite(n) ? n : 40);
}

export const api: Api = await pick();
