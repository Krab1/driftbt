// Compacts DB-IP "IP to Country Lite" (CSV, optionally .gz) into crates/drift-core/data/country.bin.
//   node scripts/build-geoip.mjs path\to\dbip-country-lite-YYYY-MM.csv.gz
// Output (gzip): u32 n4, n4 x (u32 start, u16 cc), u32 n6, n6 x (u64 start of the top 64 bits, u16 cc), big-endian.
// Each entry runs until the next entry's start; cc 0 means unknown. Data licence: CC BY 4.0 (db-ip.com).
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { gunzipSync, gzipSync } from "node:zlib";
import { dirname } from "node:path";

const input = process.argv[2];
if (!input) throw new Error("usage: node scripts/build-geoip.mjs <dbip csv or csv.gz>");
let raw = readFileSync(input);
if (raw[0] === 0x1f && raw[1] === 0x8b) raw = gunzipSync(raw);

const cc = (s) => (s === "ZZ" || s.length !== 2 ? 0 : (s.charCodeAt(0) << 8) | s.charCodeAt(1));
const v4 = (s) => s.split(".").reduce((a, o) => a * 256 + Number(o), 0);
function v6top(s) {
  // Expand "::" and keep the first four groups (64 bits).
  const [head, tail = ""] = s.split("::");
  const h = head ? head.split(":") : [];
  const t = tail ? tail.split(":") : [];
  const groups = s.includes("::") ? [...h, ...Array(8 - h.length - t.length).fill("0"), ...t] : h;
  return groups.slice(0, 4).reduce((a, g) => (a << 16n) | BigInt(parseInt(g || "0", 16)), 0n);
}

const a4 = [];
const a6 = [];
for (const line of raw.toString("utf8").split("\n")) {
  const [from, , code] = line.trim().split(",");
  if (!code) continue;
  const c = cc(code);
  const list = from.includes(":") ? a6 : a4;
  const start = from.includes(":") ? v6top(from) : v4(from);
  const prev = list[list.length - 1];
  // Merge neighbours with the same country; entries are already sorted and contiguous in the source.
  if (!prev || prev[1] !== c) list.push([start, c]);
}

const out = Buffer.alloc(8 + a4.length * 6 + a6.length * 10);
let o = 0;
out.writeUInt32BE(a4.length, o); o += 4;
for (const [s, c] of a4) (out.writeUInt32BE(s, o), out.writeUInt16BE(c, o + 4), (o += 6));
out.writeUInt32BE(a6.length, o); o += 4;
for (const [s, c] of a6) (out.writeBigUInt64BE(s, o), out.writeUInt16BE(c, o + 8), (o += 10));

const dest = new URL("../crates/drift-core/data/country.bin", import.meta.url);
mkdirSync(dirname(dest.pathname.replace(/^\/(\w:)/, "$1")), { recursive: true });
writeFileSync(dest, gzipSync(out, { level: 9 }));
console.log(`v4 ranges ${a4.length}, v6 ranges ${a6.length}, ${gzipSync(out, { level: 9 }).length} bytes gzipped`);
