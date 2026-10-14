// Enforces CLAUDE.md rule 2: drift-core must not depend on UI/HTTP crates; librqbit only in engine/rqbit.rs.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const banned = ["tauri", "axum", "wry", "tao"];
const toml = readFileSync("crates/drift-core/Cargo.toml", "utf8");
const bad = banned.filter((c) => new RegExp(`^${c}\b`, "m").test(toml));
let failed = false;
if (bad.length) { console.error("drift-core depends on banned crates:", bad.join(", ")); failed = true; }

function walk(dir, out = []) {
  for (const f of readdirSync(dir)) {
    const p = join(dir, f);
    statSync(p).isDirectory() ? walk(p, out) : p.endsWith(".rs") && out.push(p);
  }
  return out;
}
for (const root of ["crates", "src-tauri/src"]) {
  for (const p of walk(root)) {
    if (p.replaceAll("\\", "/").endsWith("engine/rqbit.rs")) continue;
    if (/\blibrqbit\b/.test(readFileSync(p, "utf8"))) { console.error("librqbit used outside engine/rqbit.rs:", p); failed = true; }
  }
}
process.exit(failed ? 1 : 0);
