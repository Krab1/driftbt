/** Theme packs: a JSON file of colours and nothing else. No code runs; unknown keys are refused, so a pack can't
 *  reach anything but the colour tokens in tokens.css. */

export type Base = "light" | "dark";

export interface ThemePack {
  name: string;
  base: Base;
  tokens: Record<string, string>;
}

/** The colour tokens a pack may set (names without the leading `--`). */
export const TOKENS = [
  "bg", "surface-1", "surface-2", "hover", "selected", "text-primary", "text-secondary", "text-muted", "border", "border-strong",
  "accent", "accent-fg", "on-accent", "success", "warning", "danger", "danger-fg", "neutral", "track",
] as const;

/** The built-in values of the tokens whose contrast is checked, used when a pack leaves one out. */
const BASE: Record<Base, Record<string, string>> = {
  light: { bg: "#fafafb", "surface-1": "#f2f3f5", "surface-2": "#ffffff", "text-primary": "#14171f", "text-secondary": "#555b6b", "text-muted": "#666d7d", accent: "#2b6cf0", "accent-fg": "#1d58cf", "on-accent": "#ffffff", "danger-fg": "#b82e2e" },
  dark: { bg: "#14161b", "surface-1": "#1b1e25", "surface-2": "#232730", "text-primary": "#e8eaf0", "text-secondary": "#a6adbd", "text-muted": "#8a91a2", accent: "#5e94ff", "accent-fg": "#7aa6ff", "on-accent": "#0b1020", "danger-fg": "#ff6b6b" },
};

/** [text, background, what it is]: each must reach 4.5:1. */
const PAIRS: [string, string, string][] = [
  ["text-primary", "bg", "Main text on the background"],
  ["text-primary", "surface-1", "Main text on side panels"],
  ["text-primary", "surface-2", "Main text on menus"],
  ["text-secondary", "bg", "Secondary text on the background"],
  ["text-muted", "bg", "Faint text on the background"],
  ["accent-fg", "bg", "Link and highlight text on the background"],
  ["danger-fg", "bg", "Error text on the background"],
  ["on-accent", "accent", "Text on accent buttons"],
];

const MAX_BYTES = 20 * 1024;
const HEX = /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
const FUNC = /^rgba?\(\s*\d{1,3}(?:\.\d+)?\s*,\s*\d{1,3}(?:\.\d+)?\s*,\s*\d{1,3}(?:\.\d+)?\s*(?:,\s*(?:0|1|0?\.\d+|1\.0+)\s*)?\)$/i;

export function validColor(v: unknown): v is string {
  return typeof v === "string" && v.length <= 40 && (HEX.test(v) || FUNC.test(v));
}

/** [r,g,b] when the colour is fully opaque, otherwise null (contrast depends on what is behind it). */
export function opaque(v: string): [number, number, number] | null {
  if (v.startsWith("#")) {
    let h = v.slice(1);
    if (h.length === 4 || h.length === 3) h = [...h].map((c) => c + c).join("");
    if (h.length === 8) {
      if (h.slice(6).toLowerCase() !== "ff") return null;
      h = h.slice(0, 6);
    }
    return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16)) as [number, number, number];
  }
  const n = v.slice(v.indexOf("(") + 1, -1).split(",").map((x) => parseFloat(x));
  if (n.length === 4 && n[3] < 1) return null;
  if (n.slice(0, 3).some((x) => x > 255)) return null;
  return [n[0], n[1], n[2]];
}

function luminance([r, g, b]: [number, number, number]): number {
  const f = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

export function contrast(a: [number, number, number], b: [number, number, number]): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

/** Messages for the colour pairs that fall under 4.5:1. A pair involving a see-through colour is not checked. */
export function warnings(pack: ThemePack): string[] {
  const get = (k: string) => pack.tokens[k] ?? BASE[pack.base][k];
  const out: string[] = [];
  for (const [fg, bg, what] of PAIRS) {
    const [f, b] = [get(fg), get(bg)];
    const fc = f ? opaque(f) : null;
    const bc = b ? opaque(b) : null;
    if (!fc || !bc) continue;
    const r = contrast(fc, bc);
    if (r < 4.5) out.push(`${what} is hard to read (${r.toFixed(1)}:1; 4.5:1 is the minimum).`);
  }
  return out;
}

/** Reads a pack file. Plain-language `error` when it can't be used; otherwise the pack. */
export function parsePack(text: string): { pack?: ThemePack; error?: string } {
  if (text.length > MAX_BYTES) return { error: "That file is too big for a theme pack." };
  let v: unknown;
  try {
    v = JSON.parse(text);
  } catch {
    return { error: "That file isn't a theme pack: it isn't valid JSON." };
  }
  if (!v || typeof v !== "object" || Array.isArray(v)) return { error: "That file isn't a theme pack." };
  const o = v as Record<string, unknown>;
  if (o.drift_theme !== 1) return { error: "That file isn't a Drift theme pack (it needs \"drift_theme\": 1)." };
  const extra = Object.keys(o).filter((k) => !["drift_theme", "name", "base", "tokens"].includes(k));
  if (extra.length) return { error: `The pack has something Drift doesn't know: "${extra[0].slice(0, 40)}".` };
  const name = typeof o.name === "string" ? o.name.trim() : "";
  if (!name || name.length > 40) return { error: "The pack needs a name of up to 40 characters." };
  if (o.base !== "light" && o.base !== "dark") return { error: "The pack's \"base\" must be \"light\" or \"dark\"." };
  if (!o.tokens || typeof o.tokens !== "object" || Array.isArray(o.tokens)) return { error: "The pack needs a \"tokens\" list of colours." };
  const tokens: Record<string, string> = {};
  for (const [k, val] of Object.entries(o.tokens as Record<string, unknown>)) {
    const key = k.startsWith("--") ? k.slice(2) : k;
    if (!(TOKENS as readonly string[]).includes(key)) return { error: `"${k.slice(0, 40)}" isn't a colour Drift lets a pack change.` };
    if (!validColor(val)) return { error: `The colour for "${key}" isn't valid. Use a value like #1a2b3c or rgba(20, 30, 40, 0.5).` };
    tokens[key] = val;
  }
  if (!Object.keys(tokens).length) return { error: "The pack doesn't change any colour." };
  return { pack: { name, base: o.base, tokens } };
}

/** Sets (or, with null, clears) the pack's colours on the page. Anything that isn't a valid known token is skipped. */
export function applyPack(pack: ThemePack | null, root: HTMLElement = document.documentElement) {
  for (const t of TOKENS) root.style.removeProperty(`--${t}`);
  if (!pack) return;
  for (const [k, v] of Object.entries(pack.tokens)) {
    if ((TOKENS as readonly string[]).includes(k) && validColor(v)) root.style.setProperty(`--${k}`, v);
  }
}
