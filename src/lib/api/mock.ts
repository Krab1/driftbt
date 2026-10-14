// In-browser stand-in used when the page runs outside Tauri (dev preview, UI tests).
import type { Api, ApiError, Settings, TorrentState, TorrentStats } from "./types";

const NAMES = [
  "ubuntu-26.04-desktop-amd64.iso",
  "debian-13-netinst.iso",
  "archlinux-2026.10.01-x86_64.iso",
  "fedora-workstation-43.iso",
  "mint-22-cinnamon.iso",
  "opensuse-tumbleweed-dvd.iso",
  "alpine-standard-3.22.iso",
  "nixos-minimal-26.05.iso",
];

function rng(seed: number) {
  let x = seed >>> 0 || 1;
  return () => {
    x ^= x << 13;
    x >>>= 0;
    x ^= x >>> 17;
    x ^= x << 5;
    x >>>= 0;
    return x / 0xffffffff;
  };
}

function demo(n: number): TorrentStats[] {
  const r = rng(42);
  return Array.from({ length: n }, (_, i) => {
    const total = Math.floor((50 + r() * 6000) * 1048576);
    const roll = Math.floor(r() * 10);
    const state: TorrentState = roll <= 3 ? "downloading" : roll <= 7 ? "seeding" : roll === 8 ? "paused" : "error";
    const progress = state === "seeding" ? total : Math.floor(r() * total);
    const down = state === "downloading" ? Math.floor(200_000 + r() * 15_000_000) : 0;
    return {
      id: i.toString(16).padStart(40, "0"),
      name: `${NAMES[Math.floor(r() * 8)]} (${i + 1})`,
      state,
      error: state === "error" ? "No space left on the download drive." : null,
      total_bytes: total,
      progress_bytes: progress,
      uploaded_bytes: Math.floor(r() * total),
      down_bps: down,
      up_bps: state === "seeding" ? Math.floor(r() * 3_000_000) : 0,
      peers_live: Math.floor(r() * 60),
      eta_secs: down > 0 ? Math.floor((total - progress) / down) : null,
    };
  });
}

const fail = (code: string, message: string): never => {
  throw { code, message } satisfies ApiError;
};

export function createMockApi(count = 40): Api {
  let torrents = demo(count);
  let settings: Settings = { schema: 1, download_dir: "C:\\Users\\you\\Downloads", theme: "system" };
  const subs = new Set<(l: TorrentStats[]) => void>();
  const snapshot = () => torrents.map((t) => ({ ...t }));
  const find = (id: string) => torrents.find((t) => t.id === id) ?? fail("not_found", "That torrent no longer exists.");

  setInterval(() => {
    torrents = torrents.map((t) => {
      if (t.state !== "downloading") return t;
      const progress = Math.min(t.total_bytes, t.progress_bytes + t.down_bps);
      const done = progress === t.total_bytes;
      return {
        ...t,
        progress_bytes: progress,
        state: done ? "seeding" : "downloading",
        down_bps: done ? 0 : t.down_bps,
        eta_secs: done || !t.down_bps ? null : Math.floor((t.total_bytes - progress) / t.down_bps),
      };
    });
    const s = snapshot();
    subs.forEach((cb) => cb(s));
  }, 1000);

  return {
    list: async () => snapshot(),
    add: async (input) => {
      if (input.kind === "magnet") {
        const m = /btih:([0-9a-fA-F]{40})/.exec(input.value);
        if (!m) return fail("invalid_input", "That isn't a valid magnet link or torrent file.");
        const id = m[1].toLowerCase();
        if (!torrents.some((t) => t.id === id)) {
          torrents = [
            { id, name: id, state: "checking", error: null, total_bytes: 0, progress_bytes: 0, uploaded_bytes: 0, down_bps: 0, up_bps: 0, peers_live: 0, eta_secs: null },
            ...torrents,
          ];
        }
        return id;
      }
      return fail("invalid_input", "That isn't a valid magnet link or torrent file.");
    },
    pause: async (id) => {
      find(id);
      torrents = torrents.map((t) => (t.id === id ? { ...t, state: "paused", down_bps: 0, up_bps: 0, eta_secs: null } : t));
    },
    resume: async (id) => {
      find(id);
      torrents = torrents.map((t) =>
        t.id === id ? { ...t, state: t.progress_bytes >= t.total_bytes && t.total_bytes > 0 ? "seeding" : "downloading", down_bps: 3_000_000 } : t,
      );
    },
    remove: async (id) => {
      find(id);
      torrents = torrents.filter((t) => t.id !== id);
    },
    getSettings: async () => ({ ...settings }),
    setSettings: async (s) => (settings = { ...s }),
    onSnapshot: async (cb) => {
      subs.add(cb);
      return () => subs.delete(cb);
    },
    pickFolder: async () => "D:\\Torrents",
  };
}
