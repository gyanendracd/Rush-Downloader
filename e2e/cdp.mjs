// Runs a snippet inside the real RushDownload window through WebView2's
// debugging port and prints what it returns. Usage: node cdp.mjs <snippet.js>
import { readFileSync } from "node:fs";

const PORT = process.env.CDP_PORT ?? "9333";
const body = readFileSync(process.argv[2], "utf8");

const HELPERS = String.raw`
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const $ = (s, root = document) => root.querySelector(s);
const $$ = (s, root = document) => [...root.querySelectorAll(s)];
const byText = (sel, text) => $$(sel).find((e) => e.textContent.trim().includes(text));
const click = (el, opts = {}) => {
  if (!el) throw new Error("click: element not found");
  el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, ...opts }));
};
const type = (el, value) => {
  if (!el) throw new Error("type: element not found");
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
};
const choose = (el, value) => {
  if (!el) throw new Error("choose: element not found");
  Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(el, value);
  el.dispatchEvent(new Event("change", { bubbles: true }));
};
const key = (k, opts = {}) =>
  window.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...opts }));
const waitFor = async (fn, ms = 10000, what = "condition") => {
  const t = Date.now();
  while (Date.now() - t < ms) {
    const v = fn();
    if (v) return v;
    await sleep(100);
  }
  throw new Error("timed out waiting for " + what);
};
const rows = () =>
  $$(".row").map((r) => ({
    name: $(".row-name", r)?.textContent,
    meta: $(".row-meta", r)?.textContent,
    status: [...r.classList].find((c) => c.startsWith("row-") && c !== "row-main")?.slice(4),
    selected: r.classList.contains("is-selected"),
  }));
const row = (name) => $$(".row").find((r) => $(".row-name", r)?.textContent.includes(name));
const nav = (label) => click($$(".nav-item").find((b) => b.getAttribute("aria-label") === label));
const toasts = () => $$(".toast").map((t) => t.textContent.trim());
const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);
if (!window.__e2eErrors) {
  window.__e2eErrors = [];
  window.addEventListener("error", (e) => window.__e2eErrors.push(String(e.message)));
  window.addEventListener("unhandledrejection", (e) => window.__e2eErrors.push("unhandled: " + String(e.reason)));
  const original = console.error;
  console.error = (...a) => {
    window.__e2eErrors.push(a.map(String).join(" ").slice(0, 300));
    original(...a);
  };
}
const errors = () => window.__e2eErrors.slice();
`;

const targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const page = targets.find((t) => t.type === "page" && t.url.includes("localhost:1420"));
if (!page) {
  console.error("app page not found; targets:", targets.map((t) => t.url));
  process.exit(2);
}

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = () => reject(new Error("could not connect to the app"));
});

const result = await new Promise((resolve) => {
  ws.onclose = () => {
    console.log("FAILED: the app window closed or reloaded while the test was running");
    process.exit(3);
  };
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id === 1) resolve(msg.result);
  };
  ws.send(
    JSON.stringify({
      id: 1,
      method: "Runtime.evaluate",
      params: {
        expression: `(async () => { ${HELPERS}\n${body}\n })()`,
        awaitPromise: true,
        returnByValue: true,
      },
    }),
  );
});
ws.onclose = null;
ws.close();

if (result.exceptionDetails) {
  const ex = result.exceptionDetails;
  console.log("FAILED:", ex.exception?.description ?? ex.text);
  process.exit(1);
}
console.log(JSON.stringify(result.result.value, null, 2));
