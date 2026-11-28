// Notes the video and audio files each tab loads, so the popup can list
// files that never appear in the page itself (a player's stream, say). Only
// addresses are kept, for as long as the tab stays on that page.
// Chromium runs this as a service worker, which loads shared.js itself;
// Firefox loads both files from the manifest.
if (typeof importScripts === "function") importScripts("shared.js");

const PER_TAB = 80;
const key = (tabId) => "tab:" + tabId;

// One change at a time, so two files caught together are both kept.
let turn = Promise.resolve();
function change(tabId, edit) {
  turn = turn
    .then(async () => {
      const stored = (await chrome.storage.session.get(key(tabId)))[key(tabId)] || [];
      const next = edit(stored);
      if (next === stored) return;
      if (next.length === 0) await chrome.storage.session.remove(key(tabId));
      else await chrome.storage.session.set({ [key(tabId)]: next });
      chrome.action.setBadgeText({ tabId, text: next.length ? String(next.length) : "" }).catch(() => {});
    })
    .catch(() => {});
}

function header(details, name) {
  const found = (details.responseHeaders || []).find((h) => h.name.toLowerCase() === name);
  return found ? String(found.value || "") : "";
}

/** What a loaded file is, from what the server says it is and from its address. */
function caughtKind(details) {
  const url = details.url;
  const type = header(details, "content-type").toLowerCase();
  if (type.includes("mpegurl") || type.includes("dash+xml")) return "stream";
  const byName = rushKindOf(url);
  if (byName === "stream") return "stream";
  // A whole file is loaded by the page's video or audio element. Video that
  // a player fetches by script comes in pieces, which are no use alone; the
  // stream's playlist or the page link is what the app can download.
  if (details.type !== "media" || RUSH_PIECE.test(url) || type.includes("mp2t")) return null;
  if (byName) return byName;
  if (type.startsWith("video/")) return "video";
  if (type.startsWith("audio/")) return "audio";
  return null;
}

/** The whole file's size, where the server says. */
function caughtSize(details) {
  const range = /\/(\d+)\s*$/.exec(header(details, "content-range"));
  if (range) return Number(range[1]);
  const length = Number(header(details, "content-length"));
  return details.statusCode === 200 && length > 0 ? length : null;
}

chrome.webRequest.onHeadersReceived.addListener(
  (details) => {
    if (details.tabId < 0 || (details.statusCode !== 200 && details.statusCode !== 206)) return;
    const kind = caughtKind(details);
    if (!kind) return;
    const url = details.url;
    let same;
    try {
      const parsed = new URL(url);
      // The same file asked for again in parts has the same address up to the "?".
      same = kind === "stream" ? url : parsed.origin + parsed.pathname;
    } catch (_) {
      return;
    }
    // A playlist's own size says nothing about the video's.
    const size = kind === "stream" ? null : caughtSize(details);
    const referer = details.initiator && /^https?:/.test(details.initiator) ? details.initiator + "/" : "";
    change(details.tabId, (list) => {
      const known = list.find((item) => item.same === same);
      if (known) {
        if (known.size || !size) return list;
        return list.map((item) => (item === known ? { ...item, size } : item));
      }
      if (list.length >= PER_TAB) return list;
      return [...list, { url, same, kind, size, referer, from: "loaded" }];
    });
  },
  { urls: ["http://*/*", "https://*/*"], types: ["media", "xmlhttprequest", "object", "other"] },
  ["responseHeaders"],
);

// A tab that moves to another page starts its list again.
chrome.tabs.onUpdated.addListener((tabId, changed) => {
  if (changed.url) change(tabId, (list) => (list.length ? [] : list));
});
chrome.tabs.onRemoved.addListener((tabId) => {
  chrome.storage.session.remove(key(tabId)).catch(() => {});
});

/* ---------- right-click menu ---------- */

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "rush-send",
      title: "Send to RushDownload",
      contexts: ["link", "video", "audio", "page"],
    });
    // A file the browser should download itself, whatever the settings say.
    chrome.contextMenus.create({
      id: "rush-browser",
      title: "Download with the browser",
      contexts: ["link"],
    });
  });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === "rush-browser") {
    // Started by this extension, so it is not caught (rushShouldCatch).
    if (info.linkUrl && /^https?:/i.test(info.linkUrl)) chrome.downloads.download({ url: info.linkUrl }).catch(() => {});
    return;
  }
  if (info.menuItemId !== "rush-send") return;
  const page = info.pageUrl || (tab && tab.url) || "";
  let item;
  if (info.srcUrl && /^https?:/i.test(info.srcUrl)) {
    // A video or audio element with an address of its own.
    const kind = rushKindOf(info.srcUrl) || (info.mediaType === "audio" ? "audio" : "video");
    item = { url: info.srcUrl, kind, referer: page, title: (tab && tab.title) || "" };
  } else if (info.linkUrl && /^https?:/i.test(info.linkUrl)) {
    const kind = rushKindOf(info.linkUrl);
    item = kind
      ? { url: info.linkUrl, kind, referer: page, title: (tab && tab.title) || "" }
      : { url: info.linkUrl, kind: "page" };
  } else if (/^https?:/i.test(page)) {
    // The page itself (also a player that shows no address): the app looks it up.
    item = { url: page, kind: "page" };
  } else {
    return;
  }
  const sent = await rushSend([item]);
  // The badge says how it went, then goes back to the tab's count.
  if (!tab || tab.id == null) return;
  const worked = sent.ok && sent.added + sent.pages > 0;
  await chrome.action.setBadgeBackgroundColor({ tabId: tab.id, color: worked ? "#16a34a" : "#e5343b" }).catch(() => {});
  await chrome.action.setBadgeText({ tabId: tab.id, text: worked ? "OK" : "!" }).catch(() => {});
  setTimeout(async () => {
    const list = (await chrome.storage.session.get(key(tab.id)))[key(tab.id)] || [];
    chrome.action.setBadgeBackgroundColor({ tabId: tab.id, color: "#0b63f6" }).catch(() => {});
    chrome.action.setBadgeText({ tabId: tab.id, text: list.length ? String(list.length) : "" }).catch(() => {});
  }, 2500);
});

chrome.action.setBadgeBackgroundColor({ color: "#0b63f6" }).catch(() => {});
if (chrome.action.setBadgeTextColor) chrome.action.setBadgeTextColor({ color: "#ffffff" }).catch(() => {});

/* ---------- downloads the browser starts ---------- */

// The same file asked for twice within moments (a double click) goes once.
const recentlySent = new Map();

/** The site's cookies for an address, as a Cookie header; "" if none or not allowed. */
async function cookieHeader(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    return cookies.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch (_) {
    return "";
  }
}

async function flash(tabId, worked) {
  if (tabId == null || tabId < 0) return;
  await chrome.action.setBadgeBackgroundColor({ tabId, color: worked ? "#16a34a" : "#e5343b" }).catch(() => {});
  await chrome.action.setBadgeText({ tabId, text: worked ? "OK" : "!" }).catch(() => {});
  setTimeout(() => {
    chrome.action.setBadgeBackgroundColor({ tabId, color: "#0b63f6" }).catch(() => {});
    chrome.action.setBadgeText({ tabId, text: "" }).catch(() => {});
  }, 2500);
}

// A download the browser starts goes to the app when the settings say so,
// this browser is connected, and the app takes it. Only then is the
// browser's own download stopped: if anything fails, the browser carries on.
chrome.downloads.onCreated.addListener(async (item) => {
  const settings = await rushSettings();
  if (!rushShouldCatch(item, settings, chrome.runtime.id)) return;
  if (!(await rushKey())) return;
  const url = item.finalUrl || item.url;
  const now = Date.now();
  for (const [seen, at] of recentlySent) if (now - at > 5000) recentlySent.delete(seen);
  if (recentlySent.has(url)) {
    await chrome.downloads.cancel(item.id).catch(() => {});
    await chrome.downloads.erase({ id: item.id }).catch(() => {});
    return;
  }
  const cookies = settings.sendCookies ? await cookieHeader(url) : "";
  const sent = await rushSend([{ url, kind: "download", referer: item.referrer || "", cookies }]);
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true }).catch(() => []);
  if (!sent.ok || sent.added === 0) {
    // Left to the browser, which carries on with it.
    if (sent.ok) flash(tab && tab.id, false);
    return;
  }
  recentlySent.set(url, now);
  await chrome.downloads.cancel(item.id).catch(() => {});
  await chrome.downloads.erase({ id: item.id }).catch(() => {});
  flash(tab && tab.id, true);
});
