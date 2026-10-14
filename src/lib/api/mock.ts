// In-browser stand-in used when the page runs outside Tauri (dev preview, UI tests).
// Mirrors the backend's behaviour closely enough to build and measure the UI against.
import type {
  Api,
  ApiError,
  Completed,
  FileDetails,
  OpenPayload,
  Settings,
  TorrentDetails,
  TorrentPreview,
  Torrent,
  TorrentState,
  TorrentStats,
} from "./types";

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
const PATHS = ["README.txt", "video/episode-02.mkv", "video/episode-10.mkv", "images/live/filesystem.squashfs", "docs/manual.pdf", "docs/licenses/GPL-3.txt"];

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

function demo(n: number): Torrent[] {
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
      error: state === "error" ? "The download drive is full. Free up space, then resume." : null,
      total_bytes: total,
      progress_bytes: progress,
      uploaded_bytes: Math.floor(r() * total),
      down_bps: down,
      up_bps: state === "seeding" ? Math.floor(r() * 3_000_000) : 0,
      peers_live: state === "downloading" || state === "seeding" ? Math.floor(r() * 60) : 0,
      eta_secs: down > 0 ? Math.floor((total - progress) / down) : null,
      tracker: i % 4 === 3 ? null : i % 2 ? "tracker.example.org" : "open.example.net",
      category: i % 5 === 0 ? "Linux" : i % 7 === 0 ? "Docs" : null,
      tags: [],
      queue_pos: null,
      note: null,
    };
  });
}

const fail = (code: string, message: string): never => {
  throw { code, message } satisfies ApiError;
};

const invalid = () => fail("invalid_input", "That isn't a valid magnet link or torrent file.");

function filesFor(t: TorrentStats, selected: Set<number> | undefined): FileDetails[] {
  const n = 3 + (parseInt(t.id.slice(-2), 16) % 4);
  const weights = Array.from({ length: n }, (_, i) => 1 + ((parseInt(t.id.slice(-6), 16) >> (i * 3)) % 20));
  const sum = weights.reduce((a, b) => a + b, 0);
  return weights.map((w, i) => ({
    index: i,
    path: PATHS[i],
    size: Math.floor((t.total_bytes * w) / sum),
    progress_bytes: Math.floor((t.progress_bytes * w) / sum),
    selected: selected ? selected.has(i) : true,
  }));
}

export function createMockApi(count = 40): Api {
  let torrents = demo(count);
  let settings: Settings = {
    schema: 1,
    download_dir: "C:\\Users\\you\\Downloads",
    theme: "system",
    limits_enabled: false,
    down_limit_kib: 0,
    up_limit_kib: 0,
    close_to_tray: false,
    start_minimized: false,
    start_with_windows: false,
    notify_on_complete: true,
    max_active_downloads: 3,
    default_goal: { ratio: null, minutes: null },
    min_seed_minutes: 0,
    playlist_on_complete: true,
    categories: [
      { name: "Linux", save_path: "D:\Torrents\Linux" },
      { name: "Docs", save_path: null },
    ],
  };
  const selection = new Map<string, Set<number>>();
  let watched: string | null = null;
  const snapSubs = new Set<(l: Torrent[]) => void>();
  const detailSubs = new Set<(d: TorrentDetails) => void>();
  const doneSubs = new Set<(c: Completed) => void>();
  const openSubs = new Set<(p: OpenPayload) => void>();
  const userPaused = new Set<string>();
  const goals = new Map<string, import("./types").Goal | null>();
  const complete = (t: Torrent) => t.total_bytes > 0 && t.progress_bytes >= t.total_bytes;
  // Mirrors the backend manager: only max_active_downloads incomplete torrents run, the rest are queued.
  let order: string[] = torrents.filter((t) => !complete(t)).map((t) => t.id);
  const applyQueue = () => {
    order = order.filter((id) => torrents.some((t) => t.id === id && !complete(t)));
    for (const t of torrents) if (!complete(t) && !order.includes(t.id)) order.push(t.id);
    const max = settings.max_active_downloads;
    let running = 0;
    torrents = torrents.map((t) => {
      const pos = order.indexOf(t.id);
      const base: Torrent = { ...t, queue_pos: pos < 0 ? null : pos + 1 };
      if (complete(t) || userPaused.has(t.id) || t.state === "error") return base;
      running += 1;
      return base;
    });
    running = 0;
    const wants = new Set(order.filter((id) => { const t = torrents.find((x) => x.id === id)!; return !userPaused.has(id) && t.state !== "error"; }));
    torrents = torrents.map((t) => {
      if (!wants.has(t.id)) return t;
      const allowed = max === 0 || ++running <= max;
      if (allowed && (t.state === "queued" || (t.state === "paused" && !userPaused.has(t.id)))) return { ...t, state: "downloading", down_bps: 3_000_000 };
      if (!allowed && (t.state === "downloading" || t.state === "checking")) return { ...t, state: "queued", down_bps: 0, eta_secs: null };
      return t;
    });
  };
  applyQueue();
  const snapshot = () => torrents.map((t) => ({ ...t }));
  const find = (id: string) => torrents.find((t) => t.id === id) ?? fail("not_found", "That torrent no longer exists.");

  const details = (id: string): TorrentDetails => {
    const t = find(id);
    return {
      stats: { ...t },
      files: t.total_bytes ? filesFor(t, selection.get(id)) : [],
      peers: Array.from({ length: Math.min(t.peers_live, 30) }, (_, i) => ({
        addr: `203.0.113.${i + 1}:${51000 + i}`,
        client: ["qBittorrent 5.1", "Transmission 4.0", "rqbit 9.0"][i % 3],
        transport: i % 4 === 0 ? "utp" : "tcp",
        downloaded_bytes: Math.floor(t.progress_bytes / (i + 2)),
        uploaded_bytes: Math.floor(t.uploaded_bytes / (i + 3)),
      })),
      trackers: ["https://tracker.example.org/announce", "udp://open.example.net:1337/announce"],
      save_path: settings.download_dir ?? "",
    };
  };

  setInterval(() => {
    const finished: Torrent[] = [];
    torrents = torrents.map((t) => {
      if (t.state === "checking" && t.total_bytes === 0) {
        // A magnet that "found" its metadata.
        const total = 1_500_000_000;
        return { ...t, name: "metadata-fetched-demo.iso", total_bytes: total, state: "downloading", down_bps: 4_000_000, peers_live: 12 };
      }
      if (t.state !== "downloading") return t;
      const progress = Math.min(t.total_bytes, t.progress_bytes + t.down_bps);
      const done = progress === t.total_bytes;
      const next: Torrent = {
        ...t,
        progress_bytes: progress,
        state: done ? "seeding" : "downloading",
        down_bps: done ? 0 : t.down_bps,
        eta_secs: done || !t.down_bps ? null : Math.floor((t.total_bytes - progress) / t.down_bps),
      };
      if (done) finished.push(next);
      return next;
    });
    applyQueue();
    const s = snapshot();
    snapSubs.forEach((cb) => cb(s));
    finished.forEach((t) => doneSubs.forEach((cb) => cb({ id: t.id, name: t.name })));
  }, 1000);

  setInterval(() => {
    if (!watched || !torrents.some((t) => t.id === watched)) return;
    const d = details(watched);
    detailSubs.forEach((cb) => cb(d));
  }, 500);

  // Test hook: window.__driftOpen({magnets:[...], files:[...]}) behaves like a second launch.
  (window as unknown as { __driftOpen: (p: OpenPayload) => void }).__driftOpen = (p) => openSubs.forEach((cb) => cb(p));

  const pending: OpenPayload = { magnets: [], files: [] };
  const open = new URLSearchParams(location.search).get("open");
  if (open) pending.magnets.push(open);

  const sub = <T>(set: Set<(v: T) => void>) => async (cb: (v: T) => void) => {
    set.add(cb);
    return () => void set.delete(cb);
  };

  return {
    list: async () => snapshot(),
    inspect: async (input) => {
      if (input.kind !== "magnet") return invalid();
      const m = /btih:([0-9a-fA-F]{40})/.exec(input.value);
      if (!m) return invalid();
      if (input.value.includes("slow")) return fail("timeout", "Couldn't fetch the torrent info. Check the link, or try again in a moment.");
      await new Promise((r) => setTimeout(r, 700));
      const files = PATHS.map((path, index) => ({ index, path, size: (index + 1) * 120_000_000 }));
      return { id: m[1].toLowerCase(), name: "demo-distro-1.0-amd64", total_bytes: files.reduce((a, f) => a + f.size, 0), files } satisfies TorrentPreview;
    },
    add: async (input, category = null) => {
      if (input.kind === "magnet") {
        const m = /btih:([0-9a-fA-F]{40})/.exec(input.value);
        if (!m) return invalid();
        const id = m[1].toLowerCase();
        if (!torrents.some((t) => t.id === id)) {
          torrents = [
            { id, name: id, state: "checking", error: null, total_bytes: 0, progress_bytes: 0, uploaded_bytes: 0, down_bps: 0, up_bps: 0, peers_live: 0, eta_secs: null, tracker: null, category, tags: [], queue_pos: null, note: null },
            ...torrents,
          ];
          if (input.only_files) selection.set(id, new Set(input.only_files));
        }
        return id;
      }
      return invalid();
    },
    pause: async (id) => {
      find(id);
      userPaused.add(id);
      torrents = torrents.map((t) => (t.id === id ? { ...t, state: "paused", down_bps: 0, up_bps: 0, eta_secs: null } : t));
    },
    resume: async (id) => {
      find(id);
      userPaused.delete(id);
      torrents = torrents.map((t) =>
        t.id === id
          ? { ...t, error: null, state: t.progress_bytes >= t.total_bytes && t.total_bytes > 0 ? "seeding" : "downloading", down_bps: 3_000_000 }
          : t,
      );
    },
    remove: async (id) => {
      find(id);
      torrents = torrents.filter((t) => t.id !== id);
      if (watched === id) watched = null;
    },
    setCategory: async (id, category) => {
      find(id);
      torrents = torrents.map((t) => (t.id === id ? { ...t, category } : t));
    },
    setTags: async (id, tags) => {
      find(id);
      torrents = torrents.map((t) => (t.id === id ? { ...t, tags } : t));
    },
    setGoal: async (id, goal) => void (find(id), goals.set(id, goal)),
    moveInQueue: async (id, to) => {
      find(id);
      const i = order.indexOf(id);
      if (i < 0) return;
      order.splice(i, 1);
      const j = to === "top" ? 0 : to === "bottom" ? order.length : to === "up" ? Math.max(0, i - 1) : Math.min(order.length, i + 1);
      order.splice(j, 0, id);
      applyQueue();
    },
    setFiles: async (id, selected) => {
      find(id);
      selection.set(id, new Set(selected));
    },
    watchDetails: async (id) => {
      watched = id;
      return id ? details(id) : null;
    },
    onDetails: sub(detailSubs),
    onCompleted: sub(doneSubs),
    onOpenSources: sub(openSubs),
    onShown: sub(new Set<() => void>()),
    takePendingOpen: async () => ({ ...pending }),
    setVisible: async () => {},
    revealTorrent: async (id) => void find(id),
    play: async (id) => {
      find(id);
      (window as unknown as { __played?: string[] }).__played = [...((window as unknown as { __played?: string[] }).__played ?? []), id];
    },
    createPlaylist: async (id) => {
      find(id);
      return `${settings.download_dir}\\playlist.m3u8`;
    },
    getSettings: async () => ({ ...settings }),
    setSettings: async (s) => (settings = { ...s }),
    onSnapshot: sub(snapSubs),
    pickFolder: async () => "D:\\Torrents",
  };
}
