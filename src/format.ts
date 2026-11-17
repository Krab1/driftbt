import { locale, t } from "./lib/i18n.svelte";

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
  if (secs < 60) return t("{n}s", { n: Math.floor(secs) });
  const m = Math.floor(secs / 60);
  if (m < 60) return t("{n}m", { n: m });
  const h = Math.floor(m / 60);
  if (h < 24) return t("{h}h {m}m", { h, m: m % 60 });
  return t("{d}d {h}h", { d: Math.floor(h / 24), h: h % 24 });
}

/** Whole percent, never 100 until complete. */
export function percent(progress: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(100, Math.floor((progress / total) * 100));
}

// One formatter per language and kind, kept: `toLocaleDateString` sets one up on every call, and the list asks for
// two dates per visible row on every update.
const dateFormats = new Map<string, Intl.DateTimeFormat>();
function dateFormat(withTime: boolean): Intl.DateTimeFormat {
  const key = `${locale() ?? ""}|${withTime}`;
  let f = dateFormats.get(key);
  if (!f) dateFormats.set(key, (f = new Intl.DateTimeFormat(locale(), withTime ? { dateStyle: "medium", timeStyle: "short" } : { dateStyle: "medium" })));
  return f;
}

/** Unix seconds as a compact local date for list columns; the full time shows on hover. */
export function formatDay(secs: number | null): string {
  if (!secs) return "—";
  return dateFormat(false).format(secs * 1000);
}

/** Unix seconds as a short local date and time; "—" when unknown. */
export function formatWhen(secs: number | null): string {
  if (!secs) return "—";
  return dateFormat(true).format(secs * 1000);
}
