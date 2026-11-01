/** Translation. The English text is the key: `t("Add torrent")`. A language is a JSON file in `src/locales/` that maps English
 *  text to its translation; text without an entry shows in English, so a half-done language never breaks anything. */

export type Vars = Record<string, string | number>;
/** A translation is a string, or one string per plural form (`one`, `few`, `many`, `other`...). */
export type Entry = string | Record<string, string>;

/** Languages Drift ships, each named in its own language. A new language is one file in `src/locales/` and one line here. */
export const LANGUAGES = [
  { code: "en", name: "English" },
  { code: "ru", name: "Русский" },
  { code: "es", name: "Español" },
  { code: "de", name: "Deutsch" },
] as const;

const files = import.meta.glob<Record<string, Entry>>("../locales/*.json", { import: "default" });

// Two plain variables, each replaced as a whole: that is what makes every t() on screen redraw when the language changes.
let lang = $state("en");
let catalog = $state.raw<Record<string, Entry>>({});
// What was asked for (`system` or a code), also a state so formats follow it.
let pref = $state("system");
let seq = 0;
const rules = new Map<string, Intl.PluralRules>();
const plural = (l: string) => rules.get(l) ?? rules.set(l, new Intl.PluralRules(l)).get(l)!;

/** `{name}` placeholders replaced from `vars`; one with no value stays as written. */
export function render(text: string, vars?: Vars): string {
  return vars ? text.replace(/\{(\w+)\}/g, (m, k: string) => (Object.hasOwn(vars, k) ? String(vars[k]) : m)) : text;
}

/** The text to show for one entry: strings as they are, plural entries by the language's rule for `n`. */
export function pick(entry: Entry | undefined, fallback: string, lang: string, n?: number): string {
  if (entry === undefined) return fallback;
  if (typeof entry === "string") return entry;
  const form = plural(lang).select(n ?? 1);
  return entry[form] ?? entry.other ?? fallback;
}

/** Longer, accented text for finding clipped layouts (development builds only, language code `xx`). */
function stretch(text: string): string {
  const from = "aeiouAEIOU", to = "àéîöûÀÉÎÖÛ";
  // placeholders like {n} are left alone
  const body = text.split(/(\{\w+\})/).map((p) => (p.startsWith("{") ? p : p.replace(/[aeiouAEIOU]/g, (c) => to[from.indexOf(c)]))).join("");
  return `[${body} ${"·".repeat(Math.ceil(text.length * 0.4))}]`;
}

/** Translates a fixed English text. */
export function t(text: string, vars?: Vars): string {
  if (import.meta.env.DEV && lang === "xx") return render(stretch(text), vars);
  return render(pick(catalog[text], text, lang), vars);
}

/** A text with a number: `tn("{n} torrent", "{n} torrents", count)`. The second text is the key; `{n}` is the count. */
export function tn(one: string, other: string, n: number, vars?: Vars): string {
  const all = { n, ...vars };
  if (import.meta.env.DEV && lang === "xx") return render(stretch(n === 1 ? one : other), all);
  const entry = catalog[other];
  if (entry === undefined) return render(plural("en").select(n) === "one" ? one : other, all);
  return render(pick(entry, other, lang, n), all);
}

/** Marks a fixed English text that is translated where it is shown, with `td(label)`: tables of labels, for one. Does nothing by itself. */
export const N_ = (text: string): string => text;

/** Translates a text that was marked with `N_`, or one that is not fixed (a message from the engine, say): a lookup that shows the English when there is no entry. */
export const td = (text: string, vars?: Vars): string => t(text, vars);

/** For number and date formats: the chosen language, or the computer's own formats (`undefined`) while the choice is `system`. */
export function locale(): string | undefined {
  return pref === "system" ? undefined : lang === "xx" ? "en" : lang;
}

/** `system` follows the computer; anything else is a code. Falls back from `pt-BR` to `pt`, and to English. */
export function resolveLanguage(pref: string, system: readonly string[] = typeof navigator === "undefined" ? [] : navigator.languages): string {
  const known: readonly string[] = LANGUAGES.map((l) => l.code);
  const want = pref === "system" ? system : [pref];
  for (const w of want) {
    const lower = w.toLowerCase();
    const hit = known.find((k) => k.toLowerCase() === lower) ?? known.find((k) => k === lower.split("-")[0]);
    if (hit) return hit;
  }
  return import.meta.env.DEV && pref === "xx" ? "xx" : "en";
}

const KEY = "drift.language";

/** The choice last asked for (`system` or a code), so the settings can tell when it has changed. */
export const chosenLanguage = () => pref;

/** Switches the interface language: loads that language's file (none for English), then everything redraws. */
export async function setLanguage(want: string): Promise<void> {
  const chosenLang = resolveLanguage(want);
  const mine = ++seq;
  pref = want;
  try {
    localStorage.setItem(KEY, want);
  } catch {
    /* the choice still applies; it is just not remembered for the next start */
  }
  const load = files[`../locales/${chosenLang}.json`];
  const next = load ? await load().catch(() => ({})) : {};
  // Two quick changes: only the last one counts, whichever file finishes loading first.
  if (mine !== seq) return;
  catalog = next;
  lang = chosenLang;
  if (typeof document !== "undefined") document.documentElement.lang = chosenLang === "xx" ? "en" : chosenLang;
}

/** At start: the language chosen last time (remembered in the browser), so the window opens in it without waiting for settings. */
export async function initLanguage(): Promise<void> {
  let saved = "system";
  try {
    saved = localStorage.getItem(KEY) ?? "system";
  } catch {
    /* default */
  }
  await setLanguage(saved);
}
