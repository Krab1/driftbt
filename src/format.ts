/** Human-readable byte size, base 1024. */
export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${i === 0 ? n : n.toFixed(n < 10 ? 1 : 0)} ${units[i]}`;
}

/** Bytes per second; zero shows a dash so idle rows stay quiet. */
export function formatSpeed(bps: number): string {
  return bps > 0 ? `${formatBytes(bps)}/s` : "—";
}

/** Short ETA: "45s", "4m", "1h 12m", "2d 3h". Null or negative shows a dash. */
export function formatEta(secs: number | null): string {
  if (secs === null || secs < 0 || !Number.isFinite(secs)) return "—";
  if (secs < 60) return `${Math.floor(secs)}s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

/** Whole percent, never 100 until complete. */
export function percent(progress: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(100, Math.floor((progress / total) * 100));
}

/** Unix seconds as a short local date and time; "—" when unknown. */
export function formatWhen(secs: number | null): string {
  if (!secs) return "—";
  return new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}
