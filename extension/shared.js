// Used by the popup, the options page and the background worker: where the
// app listens, how this browser connects to it, the settings for catching
// downloads, and how a link is told to be a video, a sound file or a stream.

// The RushDownload app listens here, on this computer only
// (see src-tauri/src/extension.rs; the two port numbers must match).
const RUSH_APP = "http://127.0.0.1:38951";

const RUSH_MEDIA = /\.(mp4|m4v|webm|mov|mkv|flv|m3u8|mpd|mp3|m4a|aac|ogg|opus|wav|flac)(?:[?#]|$)/i;
const RUSH_AUDIO = ["mp3", "m4a", "aac", "ogg", "opus", "wav", "flac"];
// Pieces of a stream. The stream's playlist is what gets downloaded, not these.
const RUSH_PIECE = /\.(ts|m4s|cmfv|cmfa|m4f|frag)(?:[?#]|$)/i;

/** Settings for catching downloads, as the options page saves them. */
const RUSH_DEFAULTS = {
  // Send downloads the browser starts to the app instead.
  capture: true,
  // Sites whose downloads stay in the browser ("example.com" includes its subdomains).
  excludedSites: [],
  // File types that stay in the browser ("pdf", "jpg").
  excludedTypes: [],
  // Files smaller than this many kilobytes stay in the browser; 0 = none.
  minSizeKb: 0,
  // Send the site's cookies along, which downloads behind a sign-in need.
  sendCookies: true,
};

async function rushSettings() {
  const saved = await chrome.storage.local.get("settings");
  return { ...RUSH_DEFAULTS, ...(saved.settings || {}) };
}

async function rushSaveSettings(settings) {
  await chrome.storage.local.set({ settings: { ...RUSH_DEFAULTS, ...settings } });
}

/** "video", "audio" or "stream" for an address, by its file type alone; null for anything else. */
function rushKindOf(url) {
  const ext = (RUSH_MEDIA.exec(url) || [])[1];
  if (!ext) return null;
  const lower = ext.toLowerCase();
  if (lower === "m3u8" || lower === "mpd") return "stream";
  return RUSH_AUDIO.includes(lower) ? "audio" : "video";
}

/** This browser's name and version, shown in the app when it asks to connect. */
function rushBrowserName() {
  const ua = navigator.userAgent;
  const brands = (navigator.userAgentData && navigator.userAgentData.brands) || [];
  const brand = (name) => brands.find((b) => b.brand === name);
  const firefox = /Firefox\/(\d+)/.exec(ua);
  if (firefox) return `Firefox ${firefox[1]}`;
  const edge = brand("Microsoft Edge") || (/Edg\/(\d+)/.exec(ua) && { brand: "Edge", version: /Edg\/(\d+)/.exec(ua)[1] });
  if (edge) return `Edge ${edge.version}`;
  if (navigator.brave) return `Brave ${(brand("Brave") || brand("Chromium") || {}).version || ""}`.trim();
  const chrome = brand("Google Chrome") || brand("Chromium") || (/Chrome\/(\d+)/.exec(ua) && { version: /Chrome\/(\d+)/.exec(ua)[1] });
  return chrome ? `Chrome ${chrome.version}` : "a browser";
}

/** The key the app gave this browser when it connected, or null. */
async function rushKey() {
  return (await chrome.storage.local.get("key")).key || null;
}

function rushHeaders(key, json) {
  const headers = {};
  if (json) headers["Content-Type"] = "application/json";
  if (key) headers.Authorization = "Bearer " + key;
  return headers;
}

/**
 * Whether the app is open, and whether this browser is connected to it.
 * Resolves with { open, paired }.
 */
async function rushPing() {
  try {
    const key = await rushKey();
    const answer = await fetch(RUSH_APP + "/ping", { cache: "no-store", headers: rushHeaders(key), signal: AbortSignal.timeout(1500) });
    const said = await answer.json();
    if (!answer.ok || said.app !== "RushDownload") return { open: false, paired: false };
    // a key the app no longer knows (disconnected there) is forgotten
    if (key && !said.paired) await chrome.storage.local.remove("key");
    return { open: true, paired: !!said.paired };
  } catch (_) {
    return { open: false, paired: false };
  }
}

/**
 * Connects this browser to the app: the app asks the user, and gives a key
 * when they allow it. `onWaiting` is called once the question is showing.
 * Resolves with { ok, error }.
 */
async function rushPair(onWaiting) {
  let request;
  try {
    const answer = await fetch(RUSH_APP + "/pair", {
      method: "POST",
      headers: rushHeaders(null, true),
      body: JSON.stringify({ browser: rushBrowserName() }),
      signal: AbortSignal.timeout(5000),
    });
    const said = await answer.json().catch(() => ({}));
    if (!answer.ok) return { ok: false, error: said.error || "The app did not take the request" };
    request = said.request;
  } catch (_) {
    return { ok: false, error: "RushDownload is not open. Open the app, then connect again." };
  }
  if (onWaiting) onWaiting();
  const until = Date.now() + 180000;
  while (Date.now() < until) {
    await new Promise((r) => setTimeout(r, 1000));
    try {
      const answer = await fetch(RUSH_APP + "/pair/" + encodeURIComponent(request), { cache: "no-store", signal: AbortSignal.timeout(3000) });
      const said = await answer.json();
      if (said.state === "approved" && said.key) {
        await chrome.storage.local.set({ key: said.key });
        return { ok: true };
      }
      if (said.state === "denied") return { ok: false, error: "The app did not allow this browser to connect." };
      if (said.state === "expired") return { ok: false, error: "The request ran out of time. Connect again." };
    } catch (_) {
      // the app may be busy; keep asking until the time is up
    }
  }
  return { ok: false, error: "No answer came from the app. Connect again." };
}

/**
 * Hands links to the app. Each item is { url, kind, referer?, title?, cookies? }
 * where kind is "video", "audio", "stream", "embed", "page" or "download".
 * Resolves with { ok, added, pages, failed, error, pair } (pair: this
 * browser must connect first).
 */
async function rushSend(items) {
  try {
    const answer = await fetch(RUSH_APP + "/add", {
      method: "POST",
      headers: rushHeaders(await rushKey(), true),
      body: JSON.stringify({ agent: navigator.userAgent, items }),
      signal: AbortSignal.timeout(8000),
    });
    const said = await answer.json().catch(() => ({}));
    if (answer.status === 401) {
      await chrome.storage.local.remove("key");
      return { ok: false, pair: true, error: "Connect this browser to RushDownload first (open this extension and press Connect)." };
    }
    if (!answer.ok) return { ok: false, error: said.error || "The app refused those links" };
    return { ok: true, added: said.added || 0, pages: said.pages || 0, failed: said.failed || [] };
  } catch (_) {
    return { ok: false, error: "RushDownload is not open. Open the app, then send again." };
  }
}

/** Whether `host` is `site` or under it. */
function rushHostIn(host, site) {
  const h = String(host || "").toLowerCase().replace(/\.$/, "");
  const s = String(site || "").trim().toLowerCase().replace(/^\*\./, "").replace(/^\.+/, "").replace(/\.$/, "");
  return !!s && (h === s || h.endsWith("." + s));
}

/** A file's type from its name or address: "pdf", "zip"; "" when it has none. */
function rushTypeOf(name) {
  const clean = String(name || "").split(/[?#]/)[0];
  const last = clean.split(/[\\/]/).pop() || "";
  const dot = last.lastIndexOf(".");
  return dot > 0 ? last.slice(dot + 1).toLowerCase() : "";
}

/**
 * Whether a download the browser started goes to the app. `item` is the
 * browser's DownloadItem; `ownId` is this extension's id (what it starts
 * itself, "Download with browser", stays in the browser).
 */
function rushShouldCatch(item, settings, ownId) {
  if (!settings.capture || !item) return false;
  if (item.byExtensionId && item.byExtensionId === ownId) return false;
  const url = item.finalUrl || item.url || "";
  let host;
  try {
    const parsed = new URL(url);
    // blob:, data: and file: addresses only mean something inside the browser
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return false;
    host = parsed.hostname;
  } catch (_) {
    return false;
  }
  let from = "";
  try {
    from = item.referrer ? new URL(item.referrer).hostname : "";
  } catch (_) {
    from = "";
  }
  if ((settings.excludedSites || []).some((site) => rushHostIn(host, site) || (from && rushHostIn(from, site)))) return false;
  const type = rushTypeOf(item.filename) || rushTypeOf(url);
  const types = (settings.excludedTypes || []).map((t) => String(t).trim().toLowerCase().replace(/^\.+/, "")).filter(Boolean);
  if (type && types.includes(type)) return false;
  const size = item.totalBytes > 0 ? item.totalBytes : item.fileSize > 0 ? item.fileSize : 0;
  if (settings.minSizeKb > 0 && size > 0 && size < settings.minSizeKb * 1024) return false;
  return true;
}

// For the tests (tests/extension.test.mjs), which load this file on its own.
if (typeof module !== "undefined") module.exports = { rushShouldCatch, rushHostIn, rushTypeOf, rushBrowserName, RUSH_DEFAULTS };
