// The popup: reads the open page for video links, lists them, and sends the
// chosen ones to the app.

const el = (id) => document.getElementById(id);
const list = el("list");
const sendButton = el("send");
const allBox = el("all");

/** What is listed, by address: { url, kind, from, label, size, referer }. */
const items = new Map();
const chosen = new Set();
const sent = new Set();
let tab = null;

const KIND = { video: "Video", audio: "Audio", stream: "Stream", embed: "Player" };
// What plays on the page first, then links, then what is only written in its code.
const ORDER = { player: 0, loaded: 1, embed: 2, link: 3, code: 4 };

function bytes(n) {
  const units = ["B", "KB", "MB", "GB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${i === 0 ? Math.round(n) : n.toFixed(n >= 100 ? 0 : 1)} ${units[i]}`;
}

function nameOf(url) {
  try {
    const parsed = new URL(url);
    const last = parsed.pathname.split("/").filter(Boolean).pop() || "";
    let name = last;
    try {
      name = decodeURIComponent(last);
    } catch (_) {}
    return name || parsed.hostname;
  } catch (_) {
    return url;
  }
}

function hostOf(url) {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch (_) {
    return "";
  }
}

function say(text, tone) {
  el("said").textContent = text;
  el("said").className = tone || "";
}

function take(item) {
  const known = items.get(item.url);
  if (!known) {
    items.set(item.url, { ...item });
    return;
  }
  if (ORDER[item.from] < ORDER[known.from]) known.from = item.from;
  known.size = known.size || item.size || null;
  known.label = known.label || item.label || "";
}

function sorted() {
  return [...items.values()].sort(
    (a, b) =>
      ORDER[a.from] - ORDER[b.from] ||
      (b.size || 0) - (a.size || 0) ||
      (parseInt(b.label, 10) || 0) - (parseInt(a.label, 10) || 0) ||
      a.url.localeCompare(b.url),
  );
}

function refresh() {
  const open = [...items.keys()].filter((url) => !sent.has(url));
  const picked = open.filter((url) => chosen.has(url)).length;
  sendButton.disabled = picked === 0;
  sendButton.textContent = picked > 0 ? `Send ${picked} to app` : "Send to app";
  allBox.disabled = open.length === 0;
  allBox.checked = open.length > 0 && picked === open.length;
  allBox.indeterminate = picked > 0 && picked < open.length;
}

function render(note) {
  const all = sorted();
  list.replaceChildren();
  for (const item of all) {
    const row = document.createElement("li");
    const label = document.createElement("label");
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = chosen.has(item.url);
    box.disabled = sent.has(item.url);
    box.addEventListener("change", () => {
      if (box.checked) chosen.add(item.url);
      else chosen.delete(item.url);
      refresh();
    });
    const what = document.createElement("span");
    what.className = "what";
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = nameOf(item.url);
    name.title = item.url;
    const about = document.createElement("span");
    about.className = "about";
    about.textContent = [
      hostOf(item.url),
      item.label,
      item.size ? bytes(item.size) : "",
      item.from === "code" ? "in the page's code" : item.from === "link" ? "linked" : "",
      sent.has(item.url) ? "sent" : "",
    ]
      .filter(Boolean)
      .join(" · ");
    what.append(name, about);
    const kind = document.createElement("span");
    kind.className = "kind";
    kind.textContent = KIND[item.kind] || "Video";
    label.append(box, what, kind);
    row.append(label);
    if (sent.has(item.url)) row.className = "sent";
    list.append(row);
  }
  el("count").textContent =
    all.length === 0 ? "No video links found" : all.length === 1 ? "1 link found" : `${all.length} links found`;
  list.hidden = all.length === 0;
  el("empty").hidden = all.length > 0;
  if (all.length === 0) el("empty").textContent = note;
  refresh();
}

allBox.addEventListener("change", () => {
  for (const url of items.keys()) {
    if (sent.has(url)) continue;
    if (allBox.checked) chosen.add(url);
    else chosen.delete(url);
  }
  render("");
});

sendButton.addEventListener("click", async () => {
  const going = sorted().filter((item) => chosen.has(item.url) && !sent.has(item.url));
  if (going.length === 0) return;
  sendButton.disabled = true;
  say("Sending");
  const answer = await rushSend(
    going.map((item) => ({
      url: item.url,
      kind: item.kind,
      referer: item.referer || (tab && tab.url) || "",
      title: (tab && tab.title) || "",
    })),
  );
  if (!answer.ok) {
    say(answer.error, "err");
    if (answer.pair) showLink();
    refresh();
    return;
  }
  for (const item of going) {
    sent.add(item.url);
    chosen.delete(item.url);
  }
  const refused = answer.failed.length;
  say(
    refused > 0
      ? `${answer.added} started, ${refused} refused: ${answer.failed[0]}`
      : answer.added === 1
        ? "1 download started in the app"
        : `${answer.added} downloads started in the app`,
    refused > 0 ? "err" : "ok",
  );
  render("");
});

el("send-page").addEventListener("click", async () => {
  if (!tab || !/^https?:/i.test(tab.url || "")) return;
  el("send-page").disabled = true;
  const answer = await rushSend([{ url: tab.url, kind: "page" }]);
  el("send-page").disabled = false;
  if (answer.ok) say("The page is waiting on the app's Home", "ok");
  else {
    say(answer.error, "err");
    if (answer.pair) showLink();
  }
});

/** Shows whether the app is open and this browser connected to it. */
async function showLink() {
  const state = await rushPing();
  el("app").textContent = !state.open ? "App is not open" : state.paired ? "Connected" : "Not connected";
  el("app").className = "app " + (!state.open ? "off" : state.paired ? "on" : "warn");
  el("link").hidden = !state.open || state.paired;
  return state;
}

el("connect").addEventListener("click", async () => {
  el("connect").disabled = true;
  el("link-text").textContent = "Asking the app…";
  const result = await rushPair(() => {
    el("link-text").textContent = "Allow this browser in the RushDownload window.";
  });
  el("connect").disabled = false;
  if (result.ok) {
    say("Connected to RushDownload", "ok");
  } else {
    el("link-text").textContent = "This browser is not connected to RushDownload yet.";
    say(result.error, "err");
  }
  showLink();
});

el("options").addEventListener("click", () => chrome.runtime.openOptionsPage());

el("catch").addEventListener("change", async (e) => {
  const settings = await rushSettings();
  await rushSaveSettings({ ...settings, capture: e.target.checked });
});

async function start() {
  showLink();
  rushSettings().then((settings) => {
    el("catch").checked = settings.capture;
  });

  // "popup.html?tab=12" reads that tab instead of the one in front, so the
  // popup can be tried out in a tab of its own.
  const asked = Number(new URLSearchParams(location.search).get("tab"));
  tab = asked ? await chrome.tabs.get(asked).catch(() => null) : (await chrome.tabs.query({ active: true, currentWindow: true }))[0];
  const web = !!tab && /^https?:/i.test(tab.url || "");
  el("page-title").textContent = (tab && tab.title) || "This page";
  el("send-page").disabled = !web;
  if (!web) {
    el("page-note").textContent = "This is not a web page.";
    render("Open a web page with a video on it, then open this again.");
    return;
  }

  // What the tab has loaded since it opened this page.
  const stored = await chrome.storage.session.get("tab:" + tab.id);
  for (const item of stored["tab:" + tab.id] || []) take({ label: "", ...item });

  // The page itself, and every frame inside it.
  let hidden = false;
  let readable = true;
  try {
    const frames = await chrome.scripting.executeScript({ target: { tabId: tab.id, allFrames: true }, files: ["scan.js"] });
    for (const frame of frames) {
      if (!frame || !frame.result) continue;
      hidden = hidden || frame.result.hidden;
      for (const item of frame.result.items) take(item);
    }
  } catch (_) {
    readable = false;
  }

  // One find is the obvious choice.
  if (items.size === 1) chosen.add([...items.keys()][0]);
  render(
    !readable
      ? "The browser does not let extensions read this page."
      : hidden
        ? "This page's player does not show a file of its own. Use Send page: the app can fetch it from the page link."
        : "Nothing here yet. Start the video, then open this again, or use Send page.",
  );
  if (hidden && items.size > 0) say("The player hides its file. Send page may get the full video.");
}

start();
