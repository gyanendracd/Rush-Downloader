// The extension's connection, against the real app: pairing needs the
// user's yes in the app window, and only the paired extension with its key
// may add downloads. Needs a test copy started with RUSHDOWNLOAD_EXT_PORT
// (see README). Usage: node e2e/t23_extension_pair.mjs
import { execFileSync } from "node:child_process";

const PORT = process.env.RUSHDOWNLOAD_EXT_PORT ?? "38999";
const APP = `http://127.0.0.1:${PORT}`;
const ME = "chrome-extension://rushtestextension";

const call = async (path, { method = "GET", origin = ME, key, body } = {}) => {
  const headers = { Origin: origin };
  if (key) headers.Authorization = `Bearer ${key}`;
  if (body) headers["Content-Type"] = "application/json";
  const r = await fetch(APP + path, { method, headers, body: body && JSON.stringify(body) });
  return { status: r.status, json: await r.json().catch(() => ({})) };
};
const add = (key, origin = ME) => call("/add", { method: "POST", origin, key, body: { items: [{ url: "http://127.0.0.1:9/never.bin", kind: "download" }] } });

const result = {};
result.pingBefore = (await call("/ping")).json;
result.addBeforePairing = (await add(null)).status;
result.addFromPage = (await add(null, "https://evil.test")).status;

const asked = await call("/pair", { method: "POST", body: { browser: "TestBrowser 1" } });
const id = asked.json.request;
result.pending = (await call(`/pair/${id}`)).json.state;
result.otherExtensionPolling = (await call(`/pair/${id}`, { origin: "chrome-extension://someoneelse" })).json.state;

// the user's yes, in the real window
result.prompt = JSON.parse(execFileSync("node", ["e2e/cdp.mjs", "e2e/t23_allow_pair.js"], { encoding: "utf8" }).trim());

const answer = (await call(`/pair/${id}`)).json;
result.answered = answer.state;
const key = answer.key;
result.keyGivenAgain = (await call(`/pair/${id}`)).json.state;
result.pingAfter = (await call("/ping", { key })).json;
const added = await add(key);
result.addWithKey = [added.status, added.json.added];
result.addWithWrongKey = (await add("0".repeat(64))).status;
result.addWithKeyFromAnotherExtension = (await add(key, "chrome-extension://someoneelse")).status;

const ok =
  result.addBeforePairing === 401 &&
  result.addFromPage === 403 &&
  result.pending === "pending" &&
  result.otherExtensionPolling === "expired" &&
  result.prompt.mentionsBrowser &&
  result.prompt.mentionsOrigin &&
  result.answered === "approved" &&
  key?.length === 64 &&
  result.keyGivenAgain === "expired" &&
  result.pingBefore.paired === false &&
  result.pingAfter.paired === true &&
  result.addWithKey[0] === 200 &&
  result.addWithKey[1] === 1 &&
  result.addWithWrongKey === 401 &&
  result.addWithKeyFromAnotherExtension === 401;
console.log(JSON.stringify(result, null, 1));
console.log(ok ? "PASSED" : "FAILED: see above");
process.exit(ok ? 0 : 1);
