// Tiny CDP driver for the Tauri webview: node cdp.mjs eval "<js>" | shot <file.png> | evalfile <file.js>
import { readFileSync, writeFileSync } from "node:fs";

const port = process.env.CDP_PORT ?? "9222";
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page" && t.url.includes("localhost:1420"));
if (!page) {
  console.error("no Drift page found:", targets.map((t) => t.url));
  process.exit(2);
}

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r, j) => ((ws.onopen = r), (ws.onerror = j)));
let id = 0;
const pending = new Map();
ws.onmessage = (m) => {
  const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) pending.get(d.id)(d);
};
const send = (method, params = {}) =>
  new Promise((res) => {
    const i = ++id;
    pending.set(i, res);
    ws.send(JSON.stringify({ id: i, method, params }));
  });

const [cmd, arg] = process.argv.slice(2);
if (cmd === "eval" || cmd === "evalfile") {
  const expression = cmd === "evalfile" ? readFileSync(arg, "utf8") : arg;
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) console.log("EXCEPTION", JSON.stringify(r.result.exceptionDetails.exception?.description ?? r.result.exceptionDetails));
  else console.log(JSON.stringify(r.result?.result?.value ?? null, null, 1));
} else if (cmd === "shot") {
  const r = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(arg, Buffer.from(r.result.data, "base64"));
  console.log("saved", arg);
}
ws.close();
process.exit(0);
