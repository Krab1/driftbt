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
      note: null, added_at: 1_760_000_000 + i * 3600, finished_at: i % 3 === 0 ? 1_760_100_000 + i * 3600 : null,
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
    check_updates: true,
    max_active_downloads: 3,
    default_goal: { ratio: null, minutes: null },
    min_seed_minutes: 0,
    playlist_on_complete: true,
    listen_port: null,
    upnp: false,
    dht: true,
    proxy: null,
    ip_filter: { enabled: false, lists: [] },
    schedule: { enabled: false, slots: [] },
    watch_dir: null,
    vpn_guard: { enabled: false, adapter: null, auto_resume: true },
    search: { archive_org: true, torznab: [] },
    plugins: [],
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
  const guardSubs = new Set<(s: import("./types").GuardStatus) => void>();
  const watchSubs = new Set<(e: { added?: string; invalid?: string }) => void>();
  const filterStatus = (): import("./types").FilterStatus => ({
    enabled: settings.ip_filter.enabled,
    active_ranges: settings.ip_filter.enabled ? 48213 : 0,
    lists: Object.fromEntries(
      settings.ip_filter.lists.map((l) => [l.id, { ranges: 48213, updated_at: Math.floor(Date.now() / 1000) - 3600, error: l.source.includes("broken") ? "The list would block more than a quarter of the internet, so it was ignored." : null, insecure: l.source.startsWith("http://") }]),
    ),
  });
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
        country: ["US", "NL", "DE", "BR", "JP", null][i % 6],
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
  // Test hook: window.__driftVpn(false) simulates the VPN adapter dropping (and true, coming back).
  (window as unknown as { __driftVpn: (up: boolean) => void }).__driftVpn = (up) => {
    (window as unknown as { __vpnDown?: boolean }).__vpnDown = !up;
    if (!settings.vpn_guard.enabled) return;
    if (!up) {
      const running = torrents.filter((t) => t.state === "downloading" || t.state === "seeding");
      running.forEach((t) => userPaused.add(t.id));
      torrents = torrents.map((t) => (running.includes(t) ? { ...t, state: "paused", down_bps: 0, up_bps: 0, eta_secs: null } : t));
    }
    guardSubs.forEach((cb) => cb({ state: up ? "protected" : "unprotected", paused: up ? 0 : torrents.filter((t) => t.state === "paused").length }));
  };

  const pending: OpenPayload = { magnets: [], files: [] };
  const open = new URLSearchParams(location.search).get("open");
  if (open) pending.magnets.push(open);

  const sub = <T>(set: Set<(v: T) => void>) => async (cb: (v: T) => void) => {
    set.add(cb);
    return () => void set.delete(cb);
  };

  let keys: string[] = [];

  return {
    list: async () => snapshot(),
    inspect: async (input) => {
      if (input.kind === "file") return invalid();
      if (input.kind === "url") return fail("timeout", "Couldn't fetch the torrent info. Check the link, or try again in a moment.");
      const m = /btih:([0-9a-fA-F]{40})/.exec(input.value);
      if (!m) return invalid();
      if (input.value.includes("slow")) return fail("timeout", "Couldn't fetch the torrent info. Check the link, or try again in a moment.");
      await new Promise((r) => setTimeout(r, 700));
      const files = PATHS.map((path, index) => ({ index, path, size: (index + 1) * 120_000_000 }));
      return { id: m[1].toLowerCase(), name: "demo-distro-1.0-amd64", total_bytes: files.reduce((a, f) => a + f.size, 0), files } satisfies TorrentPreview;
    },
    add: async (input, category = null) => {
      if (input.kind === "url") {
        const id = [...input.value].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7).toString(16).padStart(8, "0").repeat(5);
        if (!torrents.some((t) => t.id === id)) {
          torrents = [{ id, name: input.value.split("/").pop() ?? id, state: "checking", error: null, total_bytes: 0, progress_bytes: 0, uploaded_bytes: 0, down_bps: 0, up_bps: 0, peers_live: 0, eta_secs: null, tracker: null, category, tags: [], queue_pos: null, note: null, added_at: null, finished_at: null }, ...torrents];
        }
        return id;
      }
      if (input.kind === "magnet") {
        const m = /btih:([0-9a-fA-F]{40})/.exec(input.value);
        if (!m) return invalid();
        const id = m[1].toLowerCase();
        if (!torrents.some((t) => t.id === id)) {
          torrents = [
            { id, name: id, state: "checking", error: null, total_bytes: 0, progress_bytes: 0, uploaded_bytes: 0, down_bps: 0, up_bps: 0, peers_live: 0, eta_secs: null, tracker: null, category, tags: [], queue_pos: null, note: null, added_at: null, finished_at: null },
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
    listAdapters: async () => [
      { name: "Ethernet", description: "Realtek PCIe GbE Family Controller", up: true, has_ipv4: true },
      { name: "Wi-Fi", description: "Intel Wi-Fi 6E AX211", up: false, has_ipv4: false },
      { name: "ProtonVPN", description: "WireGuard Tunnel", up: !(window as unknown as { __vpnDown?: boolean }).__vpnDown, has_ipv4: true },
    ],
    networkStatus: async () => ({ listen_port: settings.proxy ? null : (settings.listen_port ?? 51413) }),
    pickFile: async () => "D:\\Lists\\my-list.txt",
    limitsStatus: async () => ({ scheduled: false, down_bps: settings.limits_enabled && settings.down_limit_kib ? settings.down_limit_kib * 1024 : null, up_bps: null }),
    guardStatus: async () => ({ state: settings.vpn_guard.enabled ? "protected" : "off", paused: 0 }),
    onGuard: sub(guardSubs),
    filterStatus: async () => filterStatus(),
    updateIpLists: async () => {
      await new Promise((r) => setTimeout(r, 400));
      return filterStatus();
    },
    onWatch: sub(watchSubs),
    play: async (id) => {
      find(id);
      (window as unknown as { __played?: string[] }).__played = [...((window as unknown as { __played?: string[] }).__played ?? []), id];
    },
    createPlaylist: async (id) => {
      find(id);
      return `${settings.download_dir}\\playlist.m3u8`;
    },
    search: async (text) => {
      await new Promise((r) => setTimeout(r, 400));
      const q = text.trim().toLowerCase();
      if (!q) return { results: [], sources: [] };
      const mk = (i: number, src: string, magnet: boolean): import("./types").SearchResult => ({
        title: `${text} (demo result ${i + 1})`,
        sources: [src],
        info_hash: null,
        magnet: magnet ? `magnet:?xt=urn:btih:${String(i + 1).repeat(40).slice(0, 40)}` : null,
        torrent_url: magnet ? null : `https://archive.org/download/demo${i}/demo${i}_archive.torrent`,
        size: (i + 1) * 310_000_000,
        seeders: magnet ? 400 - i * 37 : null,
        leechers: magnet ? 20 + i : null,
        published: 1_760_000_000 - i * 86_400 * 30,
        category: magnet ? "Linux" : "movies",
      });
      const slow = q.includes("slow");
      return {
        results: [...Array.from({ length: 6 }, (_, i) => mk(i, "My indexer", true)), ...Array.from({ length: 4 }, (_, i) => mk(i + 6, "Internet Archive", false))],
        sources: [
          { id: "archive_org", name: "Internet Archive", outcome: "ok", count: 4, message: null },
          { id: "idx", name: "My indexer", outcome: slow ? "timed_out" : "ok", count: 6, message: slow ? "Took too long to answer." : null },
        ],
      };
    },
    searchKeysSet: async () => keys,
    listPlugins: async () => [
      { id: "example", name: "Example search", version: "1.0.0", kinds: ["search"], permissions: { net: ["*"], fs: [] }, enabled: settings.plugins.includes("example"), problem: null },
      { id: "old", name: "Old plugin", version: "0.3.0", kinds: ["search"], permissions: { net: [], fs: [] }, enabled: false, problem: "It was made for plugin protocol 2, and this Drift speaks 1." },
    ],
    setSearchKey: async (id, key) => {
      keys = key ? [...new Set([...keys, id])] : keys.filter((k) => k !== id);
    },
    getSettings: async () => ({ ...settings }),
    setSettings: async (s) => (settings = { ...s }),
    onSnapshot: sub(snapSubs),
    pickFolder: async () => "D:\\Torrents",
  };
}
