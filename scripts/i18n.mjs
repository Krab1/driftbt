// Keeps the translation files honest.
//
//   node scripts/i18n.mjs               check (part of `npm run check`): fails on stale texts, wrong placeholders, bad plural forms
//   node scripts/i18n.mjs --strict      also fails when a language lacks a text
//   node scripts/i18n.mjs --todo ru     prints a JSON skeleton of the texts `ru` still lacks (key and value both English)
//
// The English text in `t("…")` / `tn("one", "other", n)` is the key. `src/locales/<code>.json` maps it to the translation:
// a string, or for `tn` an object with one string per plural form (the key is the second, plural, English text).
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const args = process.argv.slice(2);
const strict = args.includes("--strict");
const todo = args.includes("--todo") ? args[args.indexOf("--todo") + 1] : null;

const walk = (dir) =>
  readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return n === "locales" ? [] : walk(p);
    return /\.(svelte|ts)$/.test(n) && !/\.test\.ts$/.test(n) ? [p] : [];
  });

// One pass, so `\\n` stays a backslash and an n.
const ESC = { n: "\n", t: "\t", r: "\r" };
const unescape = (s) => s.replace(/\\(u[0-9a-fA-F]{4}|x[0-9a-fA-F]{2}|.)/g, (_, c) => (c.length > 1 ? String.fromCharCode(parseInt(c.slice(1), 16)) : (ESC[c] ?? c)));
const LIT = String.raw`("(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*')`;
const lit = (s) => unescape(s.slice(1, -1));
// Replaces comments with blanks of the same length, so a commented-out t("x") is not a use and line numbers stay right.
const blank = (s) => s.replace(/\/\*[\s\S]*?\*\/|<!--[\s\S]*?-->|(^|[^:"'`])\/\/[^\n]*/g, (m, pre) => (pre ?? "") + m.slice((pre ?? "").length).replace(/[^\n]/g, " "));

const problems = [];
const keys = new Map(); // key -> { plural, where }

function add(key, plural, where) {
  const old = keys.get(key);
  if (old && old.plural !== plural) problems.push(`${where}: "${key}" is used both with t and tn`);
  if (!old) keys.set(key, { plural, where });
}

const T1 = new RegExp(String.raw`^\(\s*${LIT}\s*[,)]`);
const T2 = new RegExp(String.raw`^\(\s*${LIT}\s*,\s*${LIT}\s*,`);
const importsI18n = /from\s+["'][^"']*\/i18n\.svelte(?:\.js)?["']|from\s+["']\.\/i18n\.svelte(?:\.js)?["']/;

for (const file of walk(join(root, "src"))) {
  if (file.endsWith("i18n.svelte.ts")) continue;
  const raw = readFileSync(file, "utf8");
  if (!importsI18n.test(raw)) continue;
  const src = blank(raw);
  const where = (i) => `${relative(root, file)}:${src.slice(0, i).split("\n").length}`;
  for (const m of src.matchAll(/(?<![\w$.])(t|tr|tn|N_)(?=\()/g)) {
    const rest = src.slice(m.index + m[1].length, m.index + m[1].length + 600);
    if (m[1] === "tn") {
      const a = T2.exec(rest);
      if (a) add(lit(a[2]), true, where(m.index));
      else problems.push(`${where(m.index)}: tn needs two "quoted" texts (singular, plural) written out, then the number`);
    } else {
      const a = T1.exec(rest);
      if (a) add(lit(a[1]), false, where(m.index));
      else problems.push(`${where(m.index)}: t needs one "quoted" text on its own (no + or template string); found ${rest.slice(0, 30).trim()}`);
    }
  }
}

const holes = (s) => [...new Set([...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]))].sort();
const dir = join(root, "src", "locales");
const files = readdirSync(dir).filter((f) => f.endsWith(".json")).map((f) => f.slice(0, -5));

// Every language in LANGUAGES has a file (English needs none), and every file is in LANGUAGES.
const listed = [...readFileSync(join(root, "src", "lib", "i18n.svelte.ts"), "utf8").matchAll(/code:\s*"([\w-]+)"/g)].map((m) => m[1]);
for (const c of listed) if (c !== "en" && !files.includes(c)) problems.push(`LANGUAGES lists "${c}" but src/locales/${c}.json does not exist`);
for (const f of files) if (!listed.includes(f)) problems.push(`src/locales/${f}.json is not in LANGUAGES, so it can never be chosen`);

const missing = {};
for (const lang of files) {
  const at = `locales/${lang}.json`;
  if (!/^[a-z]{2,3}(-[A-Za-z0-9]{2,4})*$/.test(lang)) {
    problems.push(`${at}: the file name must be a language code like ru or pt-BR`);
    continue;
  }
  let cat;
  try {
    cat = JSON.parse(readFileSync(join(dir, `${lang}.json`), "utf8"));
  } catch (e) {
    problems.push(`${at}: ${e.message}`);
    continue;
  }
  const rules = new Intl.PluralRules(lang);
  const forms = new Set(rules.resolvedOptions().pluralCategories);
  // "one" covers only 1 in some languages (English, German, Spanish) but 21, 31... in others (Russian).
  const oneIsOnlyOne = [...Array(200).keys()].every((n) => (rules.select(n) === "one") === (n === 1));
  for (const [k, v] of Object.entries(cat)) {
    const use = keys.get(k);
    if (!use) {
      problems.push(`${at}: "${k}" is not used in the code any more`);
      continue;
    }
    const isObj = v !== null && typeof v === "object" && !Array.isArray(v);
    const entries = typeof v === "string" ? [["", v]] : isObj ? Object.entries(v) : null;
    if (!entries || entries.some(([, s]) => typeof s !== "string" || s === "")) {
      problems.push(`${at}: "${k}" must be text, or an object of texts (no empty ones)`);
      continue;
    }
    if (use.plural !== isObj) problems.push(`${at}: "${k}" is ${use.plural ? "a plural (needs an object with one text per form)" : "not a plural (needs text)"}`);
    if (isObj) {
      for (const [f] of entries) if (!forms.has(f)) problems.push(`${at}: "${k}" has a form "${f}" that ${lang} does not use (${[...forms].join(", ")})`);
      for (const f of forms) if (!(f in v)) problems.push(`${at}: "${k}" lacks the "${f}" form (needed for ${lang})`);
    }
    const want = holes(k);
    for (const [f, s] of entries) {
      const got = holes(s);
      const dropN = isObj && f === "one" && oneIsOnlyOne;
      const ok = want.every((x) => got.includes(x) || (dropN && x === "n")) && got.every((x) => want.includes(x));
      if (!ok) problems.push(`${at}: "${k}"${f ? ` [${f}]` : ""} has {${got}} but the English has {${want}}`);
    }
  }
  missing[lang] = [...keys.keys()].filter((k) => !(k in cat));
}

if (todo) {
  console.log(JSON.stringify(Object.fromEntries((missing[todo] ?? [...keys.keys()]).map((k) => [k, k])), null, 2));
  process.exit(0);
}
for (const [lang, list] of Object.entries(missing)) {
  if (list.length) {
    console.log(`${lang}: ${list.length} of ${keys.size} texts not translated yet (they show in English)`);
    if (strict) problems.push(...list.map((k) => `locales/${lang}.json: missing "${k}"`));
  }
}
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log(`i18n: ${keys.size} texts, ${files.length} language file(s) fine`);
