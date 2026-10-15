// End to end, nothing leaves this computer: headless Drift with OLD-style search settings, the real bundled plugin
// program, and a fake Torznab indexer on localhost.
// Needs: cargo build -p drift-server, and scripts/bundle-plugins.ps1 (so src-tauri/bundled-plugins exists).
// Run: node scripts/e2e-search-plugins.mjs
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import { fileURLToPath } from "node:url";

const repo = fileURLToPath(new URL("..", import.meta.url)).replace(/[\/]$/, "");
const work = `${os.tmpdir()}/drift-e2e`;
fs.rmSync(work, { recursive: true, force: true });
fs.mkdirSync(`${work}/bin`, { recursive: true });
fs.mkdirSync(`${work}/data`, { recursive: true });
fs.copyFileSync(`${repo}/target/debug/drift-server.exe`, `${work}/bin/drift-server.exe`);
fs.cpSync(`${repo}/src-tauri/bundled-plugins`, `${work}/bin/bundled-plugins`, { recursive: true });

const FEED = `<?xml version="1.0"?><rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>
<item><title>Ubuntu 26.04 desktop</title><enclosure url="http://idx.local/1.torrent" length="6000000000"/><torznab:attr name="seeders" value="42"/><torznab:attr name="infohash" value="0123456789abcdef0123456789abcdef01234567"/></item></channel></rss>`;
const seen = [];
const idx = http.createServer((req, res) => {
  seen.push(req.url);
  res.writeHead(200, { "Content-Type": "application/xml" });
  res.end(FEED);
});
await new Promise((r) => idx.listen(0, "127.0.0.1", r));
const idxPort = idx.address().port;

const token = "a".repeat(64);
fs.writeFileSync(`${work}/data/server.json`, JSON.stringify({ token }));
fs.writeFileSync(
  `${work}/data/settings.json`,
  JSON.stringify({ dht: false, search: { archive_org: false, torznab: [{ id: "fake", name: "Fake indexer", url: `http://127.0.0.1:${idxPort}/api`, enabled: true }] } }),
);
fs.writeFileSync(`${work}/data/secrets.json`, JSON.stringify({ fake: "E2EKEY" }));

const port = 18793;
const srv = spawn(`${work}/bin/drift-server.exe`, ["--data-dir", `${work}/data`, "--bind", `127.0.0.1:${port}`], { stdio: ["ignore", "pipe", "pipe"] });
let log = "";
srv.stdout.on("data", (d) => (log += d));
srv.stderr.on("data", (d) => (log += d));

const call = async (method, p, body) => {
  const r = await fetch(`http://127.0.0.1:${port}/api${p}`, { method, headers: { authorization: `Bearer ${token}`, "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
  const text = await r.text();
  return { status: r.status, json: text ? JSON.parse(text) : null };
};
const fail = (m) => {
  console.log("FAIL:", m);
  console.log(log);
  srv.kill();
  idx.close();
  process.exit(1);
};
const check = (cond, m) => (cond ? console.log("ok  ", m) : fail(m));

for (let i = 0; i < 50; i++) {
  try {
    if ((await fetch(`http://127.0.0.1:${port}/api/health`)).status < 500) break;
  } catch {}
  await new Promise((r) => setTimeout(r, 200));
}

const plugins = (await call("GET", "/plugins")).json;
const tz = plugins.find((p) => p.id === "torznab");
const ar = plugins.find((p) => p.id === "archive_org");
check(!!tz && !tz.problem && !!ar && !ar.problem, "both bundled plugins were installed and load");
check(tz.values.indexers?.[0]?.name === "Fake indexer" && tz.values.indexers[0].url.includes(String(idxPort)), "the old indexer became a row of the Torznab plugin");
check(JSON.stringify(tz.secrets_set) === JSON.stringify(["indexers.fake.apikey"]), "its key moved too, and only the key name is visible");
check(!JSON.stringify(plugins).includes("E2EKEY"), "no key value in the plugin list");

const settings = (await call("GET", "/settings")).json;
check(settings.plugins.includes("torznab") && !settings.plugins.includes("archive_org"), "Torznab is on, Internet Archive stays off (it was off)");
check(settings.search.torznab.length === 0 && settings.search.archive_org === false, "the old settings were consumed");
check(!fs.existsSync(`${work}/data/secrets.json`) || fs.readFileSync(`${work}/data/secrets.json`, "utf8").trim() === "{}", "the old keys file is empty");

const s = await call("POST", "/search", { text: "ubuntu" });
check(s.status === 200 && s.json.results.length === 1, "a real search through the real plugin returns the indexer's row");
check(s.json.results[0].sources.join() === "Fake indexer" && s.json.results[0].seeders === 42, "the row carries the indexer's name as its badge");
check(seen.some((u) => u.includes("apikey=E2EKEY") && u.includes("q=ubuntu")), "the plugin sent the migrated key and the query to the indexer");
check(s.json.sources[0].outcome === "ok", "the source status is ok");

const t = await call("POST", "/search/test/torznab");
check(t.status === 200 && t.json.outcome === "ok", "the Test button's call works for a plugin");

// Break the key: the indexer here ignores keys, so stop the indexer instead and expect a plain reason.
idx.close();
await new Promise((r) => setTimeout(r, 300));
const bad = await call("POST", "/search/test/torznab");
check(bad.json.outcome === "failed" && /reach/i.test(bad.json.message) && !/127\.0\.0\.1|E2EKEY/.test(JSON.stringify(bad.json)), "an unreachable indexer gives a plain message without the address or key");

srv.kill();
await new Promise((r) => setTimeout(r, 500));
const onDisk = JSON.parse(fs.readFileSync(`${work}/data/settings.json`, "utf8"));
check(onDisk.plugins.includes("torznab") && onDisk.search.torznab.length === 0, "the migration was saved to settings.json");
console.log("ALL GOOD");
process.exit(0);
